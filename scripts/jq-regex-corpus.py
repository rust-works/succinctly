#!/usr/bin/env python3
"""Differential corpus for jq-mode regular expressions (#7965, ADR-0031).

The corpus is the gate for moving jq mode onto Oniguruma. A row is one regex builtin (`test`,
`match`, `match` with `g`, a capture summary, `capture`, `scan`, `sub`, `gsub`, `splits` or
`split`) on one pattern and flags string, evaluated on three subject strings. The generators
cover look-behind bodies, references and calls, malformed pieces, classes, intervals, escapes,
option runs, comments and blanks, ordinary patterns, and 133 probe patterns from a reading of the
old text passes. The default corpus (seed 2026, 330,000 patterns requested) has 330,133 distinct
patterns in 459,678 rows; it is regenerated, not committed.

Each binary answers a chunk in one process per 400 rows, each row under `try`/`catch`, so a run is
about 1,150 processes rather than 1.4 million. A row's answer is `["ok", value]` or `["err",
message]` per subject; a row that ends its process is recorded as `CRASH`, `TIMEOUT` or `RSS` and
the batch resumes after it. `--standalone K` instead runs every K-th row as its own process with
the pattern as a literal in the program and no `try`, which `validate` compares with the batch.

Subcommands:

    gen       <corpus-dir> [--patterns N] [--seed S] [--chunk ROWS]
    run       <label> <binary> <corpus-dir> <res-dir> [--workers N] [--budget SECONDS]
              [--standalone K] [--env K=V[,K=V]] [--load-max L] [--heartbeat FILE]
    analyze   <corpus-dir> <res-dir> <candidate> [--base LABEL] [--top N] [--examples N] [--out FILE]
    validate  <res-dir> <label>
    versions  <corpus-dir> <res-dir> <candidate>...

`run` invokes the binary as `jq` (argv[0]), so the same command runs real jq and succinctly's
multi-call binary. It resumes per chunk (`<res-dir>/<label>/chunk-NNNN.out`) and exits 7 when
`--budget` ran out before every chunk was done; run it again to continue. `analyze` compares
`<candidate>` with the labels `jq171` and `jq182` row by row and sorts the differing rows into
families; with `--base` it also says which of the base's rows were resolved, left, or changed, and
which rows regressed. `versions` sorts the rows where jq 1.7.1 and 1.8.2 differ and says whom each
candidate follows.

Typical use (on an M4 Pro with four workers, a release build answers the corpus in about 11 seconds):

    python3 scripts/jq-regex-corpus.py gen corpus
    python3 scripts/jq-regex-corpus.py run jq171 /usr/bin/jq corpus res
    python3 scripts/jq-regex-corpus.py run jq182 path/to/jq-1.8.2 corpus res
    python3 scripts/jq-regex-corpus.py run main target/release/succinctly corpus res
    python3 scripts/jq-regex-corpus.py run main target/release/succinctly corpus res --standalone 40
    python3 scripts/jq-regex-corpus.py analyze corpus res main
    python3 scripts/jq-regex-corpus.py validate res main

The corpus gate of #7965: the rows equal to jq 1.7.1 do not fall, every changed row is listed by
family, and no new family appears (`analyze <candidate> --base <base>`).
"""
import json
import multiprocessing as mp
import os
import random
import re
import resource
import signal
import subprocess
import sys
import time
from collections import Counter, defaultdict

# --------------------------------------------------------------------------------------------
# Generator. The tables, the shares and the order of the random calls are the corpus: changing any
# of them changes the rows, and every stored answer has to be regenerated with it.
# --------------------------------------------------------------------------------------------

ATOMS = ["a", "b", "c", "ab", "bc", "abc", ".", "\\d", "\\w", "\\s", "\\D", "\\W", "\\S", "[ab]", "[^a]", "[a-c]", "é", "\\b", "\\B",
         "^", "$", " ", "  ", "#", "# c\n", "\n", "\t", "\\ ", "\\#", "[ #]", "[a b]", "x", "\\n", "\\t", "(?i)", "(?x)", "(?-x)",
         "(?i-x)", "(?xi)", "(?x:a b)", "(?-x: a)", "(?i:a)", "\\A", "\\z", "\\Z", "\\G", "\\K", "\\X", "\\R", "\\h", "\\H", "\\p{L}",
         "\\P{L}", "\\p{Lu}", "[[:alpha:]]", "[[:digit:]]", "\\x41", "\\u0041", "\\\\", "\\.", "-", "😀", "ß", "K", "(?m)", "(?s)",
         "[é-ë]", "[\\d_]", "[^\\s]", "\\N", "\\O", "\\y", "\\<", "\\>", "|", "1", "0", "_"]
QUANT = ["", "", "", "", "", "*", "+", "?", "*?", "+?", "??", "{2}", "{1,2}", "{0,}", "{,2}", "{0}", "{2,1}", "{", "{1", "{1,", "{,}",
         "{}", "{a}", "{1, 2}", " {2}", "{ 1}", "{1 }", "{2}{3}", "{2}+", "{2}?", "{1,2}?", "**", "++", "+*", "?+", "*+", "{100000}",
         "{100001}", "{1,100001}", "{-1}", "{1,2,3}", "}", "{0,0}", "{01}", "{1}{", " +", " *", "\n?", "{2,}?", "{0,1}+", "+?+", "??+",
         "{1}{1}", "\u000b+", " *", "(?#c)*", "(?#)+", "{2,3}?{2}"]
REFS = ["\\1", "\\2", "\\9", "\\10", "\\k<n>", "\\k<m>", "\\k<1>", "\\k<-1>", "\\k<+1>", "\\k'n'", "\\k<n+0>", "\\k<n-1>", "\\g<n>",
        "\\g<m>", "\\g<1>", "\\g<0>", "\\g<-1>", "\\g<+1>", "\\g'n'", "\\g<2>", "\\g", "\\k", "\\g<", "\\k<", "\\g<>", "\\k<>", "\\g<n",
        "\\k<n", "\\g{1}", "\\k{n}", "\\g1", "\\0", "\\11", "\\k<01>", "\\g<01>", "\\k<n>\\g<n>", "\\k< n >", "\\g<1 >", "\\k<0>",
        "\\k<n$>", "\\g<n$>", "\\k<1+0>", "\\k<5+0>", "(?(1)a|b)", "(?(n)a)", "(?(<n>)a|b)", "(?('n')a)", "(?(2)a)", "(?(0)a)",
        "(?(-1)a)", "(?(1+0)a)", "(?(a)b)", "(?(1)", "(?+1)", "(?-1)", "(?1)", "(?&n)", "(?R)", "\\1234", "\\012", "\\8"]
BAD = ["(", ")", "[", "]", "\\", "(?", "(?<", "(?<n", "(?<>a)", "(?<1>a)", "(?<1a>a)", "(?z)", "(?<n>a)(?<n>b)", "[b-a]", "\\p{Foo}",
       "(?<=", "(?<!", "(?<=a", "*", "+", "?", "a**", "(?#", "(?#c)", "(?#a\\)", "[[:foo:]]", "\\x", "\\u", "\\xZZ", "\\u12", "(?<n>",
       "(?'n'a)", "(?P<n>a)", "(?<n>a)\\k<n", "[a", "(a", "a)", "(?i", "(?:", "(?=", "(?!", "(?>", "(?~a)", "(?~", "(?~|", "\\c",
       "\\M-", "\\C-", "\\p{", "\\p", "\\p{a)", "[]", "[^]", "(?<=a)+", "(?=a)*", "^*", "$+", "\\b+", "(?<!a){2}", "(*FAIL)", "(*F)",
       "(*ab)", "(*", "(*)", "(?{x})", "(?{", "(?ia)", "(?I)", "(?)", "(? a)", "(?-)", "(?--i)a", "(?ii)a", "\\x{110000}",
       "\\x{", "\\x{41", "\\o{101}", "\\o{", "\\o{8}", "\\xC3\\xA9", "\\xC3\\xAz", "\\xC3", "\\xff", "[\\xff]", "[a-\\d]", "[\\d-a]",
       "[a-[:alpha:]]", "[[:alpha:]-z]", "(?<a[>x)", "(?<a{>x)", "(?<n{100001}>a)", "\\N{2}", "\\N{100001}", "a{999999x}"]
LB = ["(?<=%s)", "(?<!%s)", "(?<=%s)", "(?<!%s)", "(?<=%s|%s)", "(?<!%s|%s)", "(?<=(?:%s|%s))", "(?<=(%s)|%s)", "(?<=(?<n>%s))",
      "(?<=%s(?=%s))", "(?<=%s(?!%s))", "(?<=(?=%s)%s)", "(?<=(?<=%s)%s)", "(?<=(?<!%s)%s)", "(?<=(?i:%s)%s)", "(?<=(?x: %s )%s)",
      "(?<=%s\\1)", "(?<=%s\\k<n>)", "(?<=%s\\g<n>)", "(?<=%s\\g<1>)", "(?<= %s | %s )", "(?<=%s # c\n|%s)", "(?<=(?>%s)%s)",
      "(?<=(?:%s){2}|%s)", "(?<=(?:%s|%s){0})", "(?<=(?i)%s|%s)", "(?<=(?x)%s |%s)", "(?<=^%s|%s$)", "(?<=\\b%s)", "(?<=%s\\K)",
      "(?<=[%s]|%s)", "(?=(?<=%s)%s)", "(?!(?<!%s|%s))", "(?<=(%s)\\1)", "(?<=(?<n>%s)\\k<n>)", "(?<=[[%s]]|%s)", "(?<=%s{0}|%s)",
      "(?<=\\Q%s\\E|%s)", "(?<=%s(?#c)|%s)", "(?<=%s\\z)", "(?<=%s\\Z|%s)", "(?<=%s\\X)", "(?<=\\<|%s)", "(?<=\\u0041|%s)",
      "(?<=\\x{e9}%s)", "(?<=(?~%s))"]
INNER = ["a", "b", "ab", "abc", "a*", "a+", "a?", "a{2}", "a{1,2}", "a{0}", "a|bc", "é", "[ab]", ".", "\\d", "", " a", "a ", "a b",
         "(?i)a", "ß", "ǆ", "ﬀ", "K", "ſ", "(?i:ss)", "a{2,}", "a{,2}", "\\w", "a{1}b", "aa|b", "ab|cd", "ab|c|def", "x*", "(a|b)c", "\\1",
         "\\k<n>", "a{", "a{1, 2}", "#", "\\#", "[ ]", "\\n", "$", "^", "\\b", "(?=a)", "(?!a)", "a(?=b)", "(?<=a)", "(?<n>a)", "(a)",
         "(?:a)", "(?x)a b", "a{0 }", "[[:alpha:]]", "\\p{L}", "\\x41", "a?+", "a{2}+", "[^a]"]
CLASS_ITEMS = ["a", "b", "c", "a-c", "c-a", "^", "]", "[", "-", "\\d", "\\w", "\\s", "\\]", "\\[", "\\\\", "\\-", "[:alpha:]", "[:^alpha:]",
               "[:digit:]", "[:foo:]", "[:a", ":]", "&&", "&", "~", "\\x41", "\\x{e9}", "\\xff", "\\n", "\\t", "\\b", "\\v", "\\p{L}",
               "\\P{Lu}", "\\p{^L}", "é", "é-ë", " ", "#", "\\Q", "\\E", "\\Q]\\E", "(", ")", "(?i)", "{", "}", "{2}", "|", ".", "*",
               "\\c", "\\ca", "\\101", "\\0", "\\8", "\\k<n>", "\\1", "\\u0041", "[a]", "[^a]", "[]", "a-\\d", "\\d-a", "a-[:alpha:]",
               "\\N", "\\h", "\\H", "\\R", "\\X", "😀", " ", "\u000b", "a-", "-a", "--", "a--b", "$", "\\z"]
ESC = ["\\c", "\\ca", "\\cA", "\\c$", "\\c[", "\\c(", "\\c\\A", "\\c\\\\", "\\c?", "\\C-a", "\\M-a", "\\M-\\C-a", "\\x", "\\x4", "\\x41",
       "\\x{41}", "\\x{0041}", "\\x{e9}", "\\x{1F600}", "\\x{110000}", "\\x{}", "\\x{g}", "\\x{41", "\\xC3\\xA9", "\\xC3", "\\xC3\\xAz",
       "\\u0041", "\\u00e9", "\\u12", "\\u", "\\U00000041", "\\o{101}", "\\o{}", "\\o{8}", "\\o{101", "\\101", "\\012", "\\0", "\\00",
       "\\08", "\\1", "\\8", "\\9", "\\p{L}", "\\p{^L}", "\\P{L}", "\\P{^L}", "\\pL", "\\p{Foo}", "\\p{ L}", "\\p{In Basic_Latin}",
       "\\p{Alpha}", "\\p{alpha}", "\\p{", "\\p{a)", "\\p^", "\\Qa.b\\E", "\\Q.*\\E", "\\Q", "\\E", "\\Qab", "\\Q(\\E", "\\Q[\\E",
       "\\Q0\\E", "\\Q\\E", "\\k<n>", "\\g<n>", "\\N", "\\N{2}", "\\O", "\\X", "\\R", "\\K", "\\G", "\\h", "\\H", "\\y", "\\Y", "\\e",
       "\\a", "\\f", "\\v", "\\r", "\\n", "\\t", "\\d", "\\w", "\\s", "\\b", "\\B", "\\A", "\\z", "\\Z", "\\<", "\\>", "\\i", "\\j", "\\q",
       "\\T", "\\_", "\\-", "\\ ", "\\#", "\\/", "\\é", "\\😀", "\\b{start}", "\\{", "\\}", "\\(", "\\)", "\\[", "\\]", "\\|", "\\*"]
OPTS = ["(?i)", "(?x)", "(?m)", "(?s)", "(?-i)", "(?-x)", "(?i-x)", "(?x-i)", "(?im)", "(?ix)", "(?imx)", "(?imsx)", "(?-imx)", "(?i-m)",
        "(?i:%s)", "(?x:%s)", "(?-x:%s)", "(?x-i:%s)", "(?i-x:%s)", "(?m:%s)", "(?s:%s)", "(?-:%s)", "(?:%s)", "(?#c)", "(?#%s)",
        "(?#a\\)b)", "(?#(?x))", "(?#[)", "(?# x\n)", "(?a)", "(?u)", "(?L)", "(?W)", "(?I)", "(?U)", "(?y{g})", "(?^)", "(?-)", "(?i-)",
        "(?ii)", "(?--i)", "(?x) ", " # c\n", "#c\n", "# (\n", "# [\n", "# )\n", "#", " ", "\t", "\n", "\r", "\f", "\u000b", "\u0085",
        " ", "　", "\\ ", "[ ]", "[#]", "(?x)#\n", "(?x:#\n)", "(?x: [ a]# ]\n)"]
GROUPS = ["(%s)", "(?:%s)", "(?<n>%s)", "(?<m>%s)", "(?=%s)", "(?!%s)", "(?<=%s)", "(?<!%s)", "(?>%s)", "(%s|b)", "(?i:%s)", "(?x:%s)",
          "(?-x:%s)", "(?x) %s", "( %s )", "(?<n>%s)\\k<n>", "(?<n>%s)\\g<n>?", "(%s)\\1", "(%s)\\g<1>?", "(?'n'%s)", "(?<n>%s)(?<n>b)\\k<n>",
          "(?~%s)", "(?(1)%s|b)", "(%s)(?(1)a|b)", "(?<a-b>%s)", "(?<é>%s)", "(?<n1>%s)", "(?:%s|)", "(|%s)", "(%s)*", "(%s)+?", "(?:%s)*?",
          "((%s)|b)*", "(?:(%s)?)*", "(%s){0}", "(?<n>%s){0}"]
WORDS = ["foo", "bar", "id", "name", "2024", "x", "key", "value", "http", "com", "error", "warn", "v1"]
REAL = ["^%s", "%s$", "^%s$", "%s.*%s", "%s\\d+", "\\b%s\\b", "(%s|%s)", "%s[0-9]+", "^[a-z]+%s", "%s\\s*=\\s*%s", "(?<k>%s)=(?<v>\\w+)",
        "[A-Za-z_][A-Za-z0-9_]*", "\\d{4}-\\d{2}-\\d{2}", "^\\s+|\\s+$", "%s|%s|%s", "(?i)%s", "%s+", "[^,]+", "\\.%s$", "^(%s)://([^/]+)",
        "\\w+@\\w+\\.%s", ",\\s*", "\\s+", "[%s]", "%s?%s", "(\\d+)\\.(\\d+)", "^$", ".", "", "%s{2,}", "\\p{L}+", "[[:space:]]+", "-"]
FLAGS = [None, None, None, None, "x", "x", "x", "g", "i", "gx", "ix", "xi", "gix", "n", "s", "l", "p", "xn", "xs", "xl", "px", "", "xx",
         "gn", "xg", "gi", "ig", "gs", "gl", "m", "in", "gp"]
BADFLAGS = ["q", "X", "z", "gz"]
STRS = ["", "a", "ab", "abc", "aab", "abab", "bca", "AbC", "a b", "a  b c", "a#b", "a\nb", "éa", "ssß", "x{1}", "a{2}", "aa", "aaa", "ba",
        "cab", "abcabc", "{", "a{1, 2}", "KkK", " a", "#", "ǆǅ", "ﬀff", "a1b22", "AbAb", "é😀a", "123abc", "ABC", "a.b", "x-y_z", "\u0000a",
        "  ", "ÉCOLE", "a\n", "\na", "foo=bar", "foo bar 2024", "name@http.com", "a]", "[a]", "a\u000bb", "a b", "A", "u0041",
        "a-b", "]", "^", "a\\b", "\u0001", "aa0", "p", "v", "Q", "<b", "S4444"]
OPS = ["test", "test", "match", "match", "capture", "scan", "sub", "gsub", "splits", "split", "matchg", "caps"]

# inventory probes: every suspected misreading the code reading produced (checked against all three binaries here)
PROBES = [
    "^(?<n>a)(?<n>b)\\k<n>$", "(?x)^(?<n>a)(?<n>b)\\k<n>$", "^(?<n>a)(?<n>b)\\k<n>\\(?x$", "^(?<a[>x)(?<n>a)(?<n>b)\\k<n>$",
    "^(a)\\1\\Q0\\E$", "(?x)((?i)a)\\1", "^(?<a[>x)((?i)y)\\2$", "^\\x{41}(?#)*$", "^a{x}(?#)*$", "^\\c\\A*$", "^(?<n>a)?\\c\\k<n>$",
    "a(?{x})", "a(?#", "[a-\\v]", "[\\v-a]", "[\\v]", "[[:alpha:](]\\1", "a # (\n\\1", "\\c(?(a)", "\\c(?(a)\\1", "\\c(\\1",
    "\\xC3\\xAz", "\\xC3\\xg", "\\p<(?=a)*>", "\\x<(?=a)*>", "\\k{(?=a)*}", "(*FAIL)+", "(?(-1)a)", "(?(1+0)a)", "(a)\u000b{0}b",
    "(a)\u0085{0}b", "(a) {0}b", "(a)　{0}b", "(?<n>a)(?<n>b)\\(?x\\k<n>", "[a[](?=x)", "\\p^", "[[](?~a)", "[a[b](?~c)]",
    "(?~b(?#\\)))", "a* +", "a*\u000b+", "a(?#\\))*+a", "\\N{2,}+a", "(?(1)(?x)a) b", "[^^](?<=a*)", "a # [\n(?<=a|bc)",
    "\\k<5+0>(", "(?<n{1000000}>a)", "(?<a>a)\\g<a{100001}>", "\\k<a{100001}>", "[a[:b]{100001}c:]]", "(?<1a>x)", "(?#a\\)",
    "(?:\\G(?#()(?m))*", "(?x)a\u000b*?b", "a(?#\\)$)(?=b)", "(?=m)a$", "(?'m'a)$", "(?s)(?#\\)[).]", "(?s)(?<a[>.)...]", "[[](\\z)",
    "[[a](\\z)x]", "b(?#[)\\Ka", "a\\Q[\\E?\\Kc", "b[]\\K]a", "(?+1)(a)", "([a[:b](?i)c:])", "(\\Q)\\E(?i)x)X", "(?<=[[:alpha:]{0}])",
    "(?<=\\Q[\\E[^[:alpha:]])", "(?<a{>x)(?<=y{0})", "[^^] (?=a) *", "[\\Q] (?=a) *", "(?=a){1#c\n}", "a{1#c\n}", "(?<=\\x{e9})a",
    "a(?#x\\)y)*", "\\1234{2,3}?{2}", "\\N{2}?+", "[a[b]c]*+", "a{999999x}", "(?<=a(?#\\)))$", "(?<=\\<|a)b", "(?<=\\u0041|abcde)",
    "(?i)(?<ss>a)\\k<ss>", "(?i)s\\u0073", "(?:(?i)|a)+b", "(?:(?#c)|a)+b", "a{2,1}\\k<m>", "\\1\\g<1 >a", "(?x)\\A{2,1}", "\\p{a)",
    "\\p{(?x)?+", "(?~\\g<1 >", "\\g<n\\k<n>\\g<n>", "\\g<++\\k<1>", "\\g<n(?<n>a)\\k<n", "\\g<n?+(?<=abc\\k<n>)", "(?<=(?<n>a)|a{0})b",
    "(?<=(a)|b{0})b", "(?<=(a)|a{0})", "(?x)(?<=[[ab]]|ab)", "(?<=[[ ]]|a b)", "(?<=(?i)a{0}|(a))", "(?<=(?i)a{0}|ab)", "(?x)\\c$+",
    "\\c$+", "(*ab[a-c]+)", "(a)\\g<1>\\k<1+0>", "\\A{2,1}", "${2,1}", "\\b{2,1}", "\\g<1 >", "(?x)(?<![a b]++ab)",
    "(?<=(aa|b)|a{0})(?<=(?i:ß)\\1)", "(?>a){2}?+a", "(?:(a?)+)?", "(a)??++", "b|a{2,}?{2}", "\\d{100000}\\k<01>", "(?P<n>a)",
    "^[a[bc]]$", "^[^^]]$", "[a&&b]", "(?#\\))", "\\C-a", "(?i)a|b", "\\<a", "(?<a.b>a)", "(?<a$>a)\\k<a$>", "[]]", "[]", "[]\\]",
]


def lit(s):
    return "".join(ch for ch in s if ch.isalnum() or ch in " #{},-")[:10]


class Gen:
    def __init__(self, seed):
        self.r = random.Random(seed)

    def q(self):
        return self.r.choice(QUANT)

    def cls(self):
        r = self.r
        n = r.choice([1, 1, 2, 2, 3, 4])
        body = "".join(r.choice(CLASS_ITEMS) for _ in range(n))
        return "[" + r.choice(["", "", "", "^", "^^", "]", "^]"]) + body + r.choice(["]", "]", "]", "]", "]", "", "]]"])

    def part(self, w, d=0):
        r = self.r
        k = r.choices(["lb", "ref", "bad", "group", "atom", "cls", "esc", "opt", "real"], weights=w)[0]
        if k == "lb":
            t = r.choice(LB)
            return (t % tuple(r.choice(INNER) for _ in range(t.count("%s")))) + r.choice(["", "", "", self.q()])
        if k == "ref":
            return r.choice(REFS) + r.choice(["", "", self.q()])
        if k == "bad":
            return r.choice(BAD)
        if k == "group":
            if d >= 2:
                return r.choice(ATOMS) + self.q()
            g = r.choice(GROUPS)
            return g.replace("%s", self.pat(w, d + 1), 1) + self.q()
        if k == "cls":
            return self.cls() + self.q()
        if k == "esc":
            return r.choice(ESC) + self.q()
        if k == "opt":
            o = r.choice(OPTS)
            return o.replace("%s", self.pat(w, d + 1) if d < 2 else "a", 1) + r.choice(["", "", "", self.q()])
        if k == "real":
            t = r.choice(REAL)
            return t % tuple(r.choice(WORDS) for _ in range(t.count("%s")))
        return r.choice(ATOMS) + self.q()

    def pat(self, w, d=0):
        return "".join(self.part(w, d) for _ in range(self.r.choice([1, 1, 2, 2, 3, 4])))


#            lb  ref bad grp atom cls esc opt real
PROFILES = {
    "lookbehind": [40, 8, 4, 14, 20, 5, 4, 5, 0],
    "refs":       [5, 35, 6, 22, 20, 3, 4, 5, 0],
    "errors":     [8, 14, 34, 12, 16, 6, 6, 4, 0],
    "classes":    [3, 3, 3, 10, 14, 55, 6, 6, 0],
    "intervals":  [4, 4, 4, 18, 58, 4, 4, 4, 0],
    "escapes":    [3, 4, 3, 10, 14, 8, 52, 6, 0],
    "options":    [5, 4, 3, 16, 22, 6, 6, 38, 0],
    "general":    [4, 4, 2, 24, 50, 6, 6, 4, 0],
    "plain":      [0, 0, 0, 6, 10, 2, 2, 0, 80],
}
SHARE = [("lookbehind", 14), ("refs", 12), ("errors", 12), ("classes", 12), ("intervals", 12), ("escapes", 12), ("options", 12),
         ("general", 8), ("plain", 6)]


def generate(npat, seed):
    """The corpus rows: [id, generator, op, pattern, flags or None, [s1, s2, s3]]."""
    g = Gen(seed)
    r = g.r
    rows = []
    seen = set()
    rid = 0

    def add(gen, p, variants):
        nonlocal rid
        subj = [r.choice(STRS), r.choice(STRS), lit(p) or r.choice(STRS)]
        for fl in variants:
            rows.append([rid, gen, r.choice(OPS), p, fl, subj])
            rid += 1

    # probes: every flag variant that matters, every op family once
    for p in PROBES:
        seen.add(p)
        for fl in [None, "x", "i", "g", "gx", "ix", "n", "l"]:
            subj = [r.choice(STRS), lit(p) or "a", r.choice(STRS)]
            for op in ["test", "matchg", "capture", "sub"]:
                rows.append([rid, "probe", op, p, fl, subj])
                rid += 1
    total = sum(s for _, s in SHARE)
    for name, share in SHARE:
        want = npat * share // total
        made = 0
        tries = 0
        while made < want and tries < want * 20:
            tries += 1
            p = g.pat(PROFILES[name])
            if r.random() < 0.08:
                p = "(?x)" + p
            if p in seen or len(p) > 200:
                continue
            seen.add(p)
            made += 1
            v = [r.choice(FLAGS)]
            x = r.random()
            if x < 0.12:  # the same pattern with and without x / i / g
                v = [None, "x", "i", "g"]
            elif x < 0.14:
                v.append(r.choice(BADFLAGS))
            add(name, p, v)
    return rows, len(seen)


def cmd_gen(args, opt):
    out = args[0]
    npat, seed, chunk = int(opt.get("--patterns", 330000)), int(opt.get("--seed", 2026)), int(opt.get("--chunk", 5000))
    os.makedirs(out, exist_ok=True)
    rows, npatterns = generate(npat, seed)
    with open(out + "/MANIFEST", "w") as mf:
        n = 0
        for i in range(0, len(rows), chunk):
            fn = "chunk-%04d.jsonl" % n
            with open(out + "/" + fn, "w", encoding="utf-8") as f:
                for row in rows[i:i + chunk]:
                    f.write(json.dumps(row, ensure_ascii=False) + "\n")
            mf.write("%s %d\n" % (fn, len(rows[i:i + chunk])))
            n += 1
    print(json.dumps({"patterns": npatterns, "rows": len(rows), "chunks": n}))
    return 0


# --------------------------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------------------------

ROW_TIMEOUT = 10.0
RSS_CAP_KB = 2 << 20
SUB = 400
ENV = {"PATH": "/usr/bin:/bin", "LC_ALL": "en_US.UTF-8", "TZ": "UTC", "JQ_COLORS": ""}
LOAD_MAX = 10.0
HEARTBEAT = None

PROG = r'''
def run($op; $p; $f):
  if $op == "test" then test($p; $f)
  elif $op == "match" then [match($p; $f)]
  elif $op == "matchg" then [match($p; ($f // "") + "g")]
  elif $op == "caps" then [match($p; $f) | [.offset, .length, (.captures | map([.offset, .length, .string, .name]))]]
  elif $op == "capture" then [capture($p; $f)]
  elif $op == "scan" then [scan($p; $f)]
  elif $op == "sub" then sub($p; "<\(tojson)>"; $f)
  elif $op == "gsub" then gsub($p; "[\(.n // "-")]"; $f)
  elif $op == "splits" then [splits($p; $f)]
  elif $op == "split" then split($p; $f)
  else error("bad op") end;
. as [$id, $gen, $op, $p, $f, $ss]
| [$id] + [$ss[] | . as $s | try ["ok", ($s | run($op; $p; $f))] catch ["err", .]]
'''

# The same expressions with the pattern and flags as literals, for the standalone control.
OPEXPR = {
    "test": "test(%(p)s%(f)s)", "match": "[match(%(p)s%(f)s)]", "matchg": "[match(%(p)s; %(fg)s)]",
    "caps": "[match(%(p)s%(f)s) | [.offset, .length, (.captures | map([.offset, .length, .string, .name]))]]",
    "capture": "[capture(%(p)s%(f)s)]", "scan": "[scan(%(p)s%(f)s)]", "sub": 'sub(%(p)s; "<\\(tojson)>"%(f)s)',
    "gsub": 'gsub(%(p)s; "[\\(.n // "-")]"%(f)s)', "splits": "[splits(%(p)s%(f)s)]", "split": "split(%(p)s; %(f2)s)",
}


def limits():
    """Child limits: its own process group (killed as a group on timeout), no core dump, a file
    size cap, and a 2 GiB address space where the platform enforces one (the RSS poll is the
    backstop where it does not)."""
    os.setsid()
    resource.setrlimit(resource.RLIMIT_CORE, (1, 1))
    resource.setrlimit(resource.RLIMIT_FSIZE, (256 << 20, 256 << 20))
    try:
        resource.setrlimit(resource.RLIMIT_AS, (2 << 30, 2 << 30))
    except (ValueError, OSError):
        pass


def rss_kb(pid):
    try:
        o = subprocess.run(["/bin/ps", "-o", "rss=", "-p", str(pid)], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=5).stdout
        return int(o.strip() or 0)
    except (OSError, ValueError, subprocess.SubprocessError):
        return 0


def kill(p):
    try:
        os.killpg(p.pid, signal.SIGKILL)
    except OSError:
        pass
    try:
        p.kill()
    except OSError:
        pass
    p.wait()


def beat():
    if HEARTBEAT:
        try:
            os.utime(HEARTBEAT)
        except OSError:
            pass


def complete_lines(path):
    with open(path, "rb") as f:
        b = f.read()
    cut = b.rfind(b"\n") + 1
    return b[:cut].count(b"\n"), b[:cut]


def run_batch(binary, rows, wdir):
    """rows: corpus lines (bytes). Returns one output line (bytes) per row: the binary's answer,
    or a CRASH / TIMEOUT / RSS record for the row that ended the process, after which the batch
    resumes at the next row."""
    out = []
    pending = rows
    inp, outp, errp = wdir + "/in.jsonl", wdir + "/out.jsonl", wdir + "/err.txt"
    while pending:
        with open(inp, "wb") as f:
            f.write(b"".join(pending))
        fo = open(outp, "wb")
        fe = open(errp, "wb")
        p = subprocess.Popen(["jq", "-c", "--unbuffered", PROG, inp], executable=binary, stdin=subprocess.DEVNULL, stdout=fo, stderr=fe,
                             env=ENV, cwd=wdir, preexec_fn=limits)
        last_size, last_change, why = 0, time.time(), None
        while True:
            try:
                p.wait(timeout=0.25)
                break
            except subprocess.TimeoutExpired:
                sz = os.path.getsize(outp)
                now = time.time()
                if sz != last_size:
                    last_size, last_change = sz, now
                if rss_kb(p.pid) > RSS_CAP_KB:
                    why = "RSS"
                elif now - last_change > ROW_TIMEOUT:
                    why = "TIMEOUT"
                if why:
                    kill(p)
                    break
        fo.close()
        fe.close()
        n, data = complete_lines(outp)
        lines = data.split(b"\n")[:-1] if n else []
        out.extend(l + b"\n" for l in lines)
        if n >= len(pending):
            break
        rid = json.loads(pending[n])[0]
        if why is None:
            with open(errp, "rb") as f:
                err = f.read(300).decode("utf-8", "replace")
            why, detail = "CRASH", "rc=%s %s" % (p.returncode, err.strip().split("\n")[0][:200])
        else:
            detail = ""
        out.append((json.dumps([rid, why, detail]) + "\n").encode())
        pending = pending[n + 1:]
    return out


def standalone_program(row):
    rid, gen, op, pat, fl, subj = row
    lf = "" if fl is None else "; " + json.dumps(fl)
    expr = OPEXPR[op] % {"p": json.dumps(pat, ensure_ascii=False), "f": lf, "fg": json.dumps((fl or "") + "g"),
                         "f2": "null" if fl is None else json.dumps(fl)}
    return "%s | %s" % (json.dumps(subj[0], ensure_ascii=False), expr)


def standalone(binary, row, wdir):
    rid = row[0]
    try:
        p = subprocess.Popen(["jq", "-nc", standalone_program(row)], executable=binary, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, env=ENV, cwd=wdir, preexec_fn=limits)
    except (OSError, ValueError) as e:
        return [rid, "SPAWNFAIL", str(e)[:100], 0]
    try:
        o, e = p.communicate(timeout=ROW_TIMEOUT)
        rc = p.returncode
    except subprocess.TimeoutExpired:
        kill(p)
        o, e, rc = b"", b"", -999
    return [rid, o.decode("utf-8", "replace")[:20000], e.decode("utf-8", "replace")[:2000], rc]


def work(job):
    label, binary, cdir, odir, chunk, stride, deadline = job
    wid = mp.current_process()._identity[0] if mp.current_process()._identity else 0
    wdir = "%s/work/%s-w%d" % (odir, label, wid)
    os.makedirs(wdir, exist_ok=True)
    final = "%s/%s/%s%s" % (odir, label, "standalone-" if stride else "", chunk.replace(".jsonl", ".out"))
    if os.path.exists(final):
        return "skip"
    if time.time() > deadline:
        return "later"
    while os.getloadavg()[0] >= LOAD_MAX:
        time.sleep(5)
    beat()
    with open(cdir + "/" + chunk, "rb") as f:
        rows = f.readlines()
    tmp = final + ".tmp"
    with open(tmp, "wb") as fo:
        if stride:
            for i in range(0, len(rows), stride):
                fo.write((json.dumps(standalone(binary, json.loads(rows[i]), wdir), ensure_ascii=False) + "\n").encode())
        else:
            for i in range(0, len(rows), SUB):
                fo.write(b"".join(run_batch(binary, rows[i:i + SUB], wdir)))
                beat()
    os.rename(tmp, final)
    return "done"


def cmd_run(args, opt):
    global LOAD_MAX, HEARTBEAT
    label, binary, cdir, odir = args[:4]
    workers, budget, stride = int(opt.get("--workers", 4)), float(opt.get("--budget", 480)), int(opt.get("--standalone", 0))
    LOAD_MAX = float(opt.get("--load-max", LOAD_MAX))
    HEARTBEAT = opt.get("--heartbeat")
    for kv in opt.get("--env", "").split(","):
        if "=" in kv:
            name, value = kv.split("=", 1)
            ENV[name] = value
    # Absolute: each child runs inside its work directory.
    binary, cdir, odir = os.path.abspath(binary), os.path.abspath(cdir), os.path.abspath(odir)
    os.makedirs("%s/%s" % (odir, label), exist_ok=True)
    with open(cdir + "/MANIFEST") as mf:
        chunks = [l.split()[0] for l in mf]
    deadline = time.time() + budget
    with mp.get_context("fork").Pool(workers) as pool:
        res = pool.map(work, [(label, binary, cdir, odir, c, stride, deadline) for c in chunks], chunksize=1)
    c = {x: res.count(x) for x in set(res)}
    print(json.dumps({"label": label, "standalone": stride, "chunks": c}))
    return 7 if c.get("later") else 0


# --------------------------------------------------------------------------------------------
# Analysis
# --------------------------------------------------------------------------------------------

def load(resdir, label, prefix="chunk-"):
    """{row id: raw output line} for one label's batched (or standalone) answers."""
    d = {}
    p = "%s/%s" % (resdir, label)
    for fn in sorted(os.listdir(p)):
        if fn.startswith(prefix) and fn.endswith(".out"):
            with open(p + "/" + fn, "rb") as f:
                for line in f:
                    i = line.find(b",")
                    d[int(line[1:i])] = line
    return d


def load_corpus(cdir):
    corpus = {}
    with open(cdir + "/MANIFEST") as mf:
        names = [l.split()[0] for l in mf]
    for name in names:
        with open(cdir + "/" + name, encoding="utf-8") as f:
            for line in f:
                r = json.loads(line)
                corpus[r[0]] = r
    return corpus


def norm_msg(m):
    if not isinstance(m, str):
        return "<non-string error>"
    m = m.replace("Regex failure: ", "")
    m = re.sub(r"<[^>]*>", "<N>", m, count=1) if ("group name" in m or "reference" in m or "name <" in m) else m
    m = re.sub(r"\{[^}]*\}", "{N}", m) if "property name" in m else m
    if "regex parse error" in m or "error parsing regex" in m.lower():
        return "(regex crate parse error)"
    m = re.sub(r"[0-9]+", "#", m)
    return m[:90]


def feat(p, fl):
    fl = fl or ""
    if "l" in fl:
        return "flag l"
    if "n" in fl:
        return "flag n"
    if "(?<=" in p or "(?<!" in p:
        return "look-behind"
    if "\\g" in p or "(?~" in p or "(*" in p or "(?(" in p or re.search(r"\(\?[-+]?[0-9R&]", p):
        return "call/absent/conditional/callout"
    if re.search(r"\\[1-9]|\\k", p):
        return "backreference"
    if "x" in fl or re.search(r"\(\?[a-z]*x", p):
        return "extended mode"
    if "i" in fl or re.search(r"\(\?[a-z]*i", p):
        return "case folding"
    if "(?=" in p or "(?!" in p or "(?>" in p or "\\K" in p or "\\G" in p:
        return "look-ahead/atomic/\\K/\\G"
    if "$" in p or "\\Z" in p or "\\z" in p or "^" in p:
        return "anchors"
    if re.search(r"[*+?}][*+?{]", p):
        return "stacked/lazy/possessive quantifier"
    return "other"


CLASS_MSG = ("char-class", "char class", "POSIX bracket", "range specifier")
CRATE = "(regex crate parse error)"


def onig_words(m):
    return not (m == CRATE or "size limit" in m or "not supported" in m or "limit over" in m)


def coarse(row, k, f):
    """The coarse family of a differing row (the letters of ADR-0031's family table). The family
    names describe the imitation's failure modes; under an Oniguruma build a row still lands in
    one, by the shape of its answer."""
    p, fl = row[3], row[4] or ""
    xmode = "x" in fl or re.search(r"\(\?[a-z]*x", p) is not None
    call = "\\g<" in p or "\\g'" in p
    if k.startswith("candidate") or k.startswith("jq "):
        return "resource: killed by the driver (RSS / timeout / crash)"
    if k == "refused, jq answers":
        m = f[len("here: "):]
        if m == CRATE or "not supported" in m:
            return "R1 refused in the engines' words, jq answers: pattern with a call \\g" if call else \
                   "R2 refused in the engines' words, jq answers: other constructs"
        if "size limit" in m:
            return "R3 refused for compiled size, jq answers"
        if "limit over" in m:
            return "R4 backtracking budget / stack limit, jq answers"
        return "R5 refused in Oniguruma's words, jq answers" + (" (extended mode)" if xmode else "")
    if k == "answered, jq refuses":
        m = f[len("jq: "):]
        if "look-behind" in m:
            return "A1 answered, jq refuses the look-behind"
        return "A2 answered, jq refuses otherwise (%s)" % m[:45]
    if k == "error text":
        a, b = f[len("jq: "):].split(" | here: ")
        if not onig_words(b):
            return "E1 both refuse, here in the engines' words: pattern with a call \\g" if call else \
                   "E2 both refuse, here in the engines' words: other"
        if a == b:
            return "E3 same refusal, different name text in it"
        if any(w in a or w in b for w in CLASS_MSG):
            return "E4 different refusal, a class error involved (order)"
        if "never ending recursion" in a or "never ending recursion" in b:
            return "E5 different refusal, never-ending recursion involved (order)"
        if "too big number" in b or "too big number" in a:
            return "E6 different refusal, count limit involved (order)"
        return "E7 different refusal, other (order or misreading)"
    if k == "different answer":
        if f in ("flag l", "flag n"):
            return "D1 different answer under the l or n flag"
        if xmode:
            return "D2 different answer in the extended mode"
        return "D3 different answer: " + f
    return "layout only"


def family(row, j, c):
    """(kind, family) of a differing row, from the corpus row, jq's parsed answer `j` and the
    candidate's `c`."""
    p, fl = row[3], row[4]
    if isinstance(c[1], str):
        return ("candidate " + c[1], c[1] + " " + str(c[2])[:60])
    if isinstance(j[1], str):
        return ("jq " + j[1], "jq itself " + j[1])
    for a, b in zip(j[1:], c[1:]):
        if a == b:
            continue
        if a[0] == "err" and b[0] == "err":
            return ("error text", "jq: %s | here: %s" % (norm_msg(a[1]), norm_msg(b[1])))
        if a[0] == "ok" and b[0] == "err":
            return ("refused, jq answers", "here: %s" % norm_msg(b[1]))
        if a[0] == "err" and b[0] == "ok":
            return ("answered, jq refuses", "jq: %s" % norm_msg(a[1]))
        return ("different answer", feat(p, fl))
    return ("layout only", "same values, different text")


def status_of(answer, a1, a2):
    return "same171" if answer == a1 else ("follows182" if answer == a2 else "differs")


def analyze(corpus, j1, j2, c, b=None, nex=3):
    """Totals, per-generator counts, families and (with a base) transitions, as a JSON-able dict."""
    tot = Counter()
    fam = Counter()
    kinds = Counter()
    ex = defaultdict(list)
    bygen = defaultdict(Counter)
    co = Counter()
    cotr = defaultdict(Counter)
    coreg = Counter()
    regress = Counter()
    regressed_rows = []
    for rid, row in corpus.items():
        a1, a2, cc = j1.get(rid), j2.get(rid), c.get(rid)
        if cc is None or a1 is None:
            tot["missing"] += 1
            continue
        tot["rows"] += 1
        gen = row[1]
        bygen[gen]["rows"] += 1
        if a1 != a2:
            tot["jq171 != jq182"] += 1
        status = status_of(cc, a1, a2)
        tot[status] += 1
        bygen[gen][status] += 1
        if status == "differs":
            k, f = family(row, json.loads(a1), json.loads(cc))
            kinds[k] += 1
            fam[(k, f)] += 1
            co[coarse(row, k, f)] += 1
            if len(ex[(k, f)]) < nex:
                ex[(k, f)].append({"id": rid, "gen": gen, "op": row[2], "p": row[3], "f": row[4], "s": row[5],
                                   "jq171": a1.decode("utf-8", "replace")[:400], "here": cc.decode("utf-8", "replace")[:400]})
        if b is not None:
            bb = b.get(rid)
            bstat = status_of(bb, a1, a2)
            if bstat == "differs":
                bk, bf = family(row, json.loads(a1), json.loads(bb))
                t = "resolved" if status != "differs" else ("unchanged" if cc == bb else "still differs, changed")
                cotr[coarse(row, bk, bf)][t] += 1
            elif status == "differs":
                k, f = family(row, json.loads(a1), json.loads(cc))
                regress[(k, f)] += 1
                coreg[coarse(row, k, f)] += 1
                if len(regressed_rows) < 400:
                    regressed_rows.append({"id": rid, "gen": gen, "op": row[2], "p": row[3], "f": row[4], "s": row[5],
                                           "fam": [k, f], "jq171": a1.decode("utf-8", "replace")[:300],
                                           "here": cc.decode("utf-8", "replace")[:300]})
            elif status != bstat:
                tot["moved %s -> %s" % (bstat, status)] += 1
    out = {"totals": dict(tot), "kinds": dict(kinds), "bygen": {g: dict(v) for g, v in sorted(bygen.items())},
           "coarse": dict(sorted(co.items())),
           "families": [{"kind": k, "family": f, "rows": n, "examples": ex[(k, f)]} for (k, f), n in fam.most_common()]}
    if b is not None:
        out["coarse_transitions"] = {k: dict(v) for k, v in sorted(cotr.items())}
        out["coarse_regressed"] = dict(sorted(coreg.items()))
        out["regressed"] = [{"kind": k, "family": f, "rows": n} for (k, f), n in regress.most_common()]
        out["regressed_rows"] = regressed_rows
    return out


def cmd_analyze(args, opt):
    cdir, resdir, cand = args[:3]
    base = opt.get("--base")
    corpus = load_corpus(cdir)
    j1, j2, c = load(resdir, "jq171"), load(resdir, "jq182"), load(resdir, cand)
    b = load(resdir, base) if base else None
    out = analyze(corpus, j1, j2, c, b, int(opt.get("--examples", 3)))
    out["candidate"], out["base"] = cand, base
    dst = opt.get("--out", "%s/analysis-%s.json" % (resdir, cand))
    with open(dst, "w", encoding="utf-8") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
    print(json.dumps(out["totals"]))
    print(json.dumps(out["kinds"]))
    for g, v in out["bygen"].items():
        print(g, json.dumps(v))
    print("COARSE:")
    for name, n in out["coarse"].items():
        print("%7d  %s" % (n, name))
    if b is not None:
        print("COARSE vs %s (resolved / unchanged / still differs, changed):" % base)
        for name, v in out["coarse_transitions"].items():
            print("  %-80s %6d %6d %6d" % (name[:80], v.get("resolved", 0), v.get("unchanged", 0), v.get("still differs, changed", 0)))
        print("COARSE regressed:")
        for name, n in out["coarse_regressed"].items():
            print("  %6d %s" % (n, name))
    print("families:", len(out["families"]))
    for fm in out["families"][:int(opt.get("--top", 60))]:
        print("%7d  [%s] %s" % (fm["rows"], fm["kind"], fm["family"]))
    if b is not None:
        print("regressed:", sum(r["rows"] for r in out["regressed"]))
        for r in out["regressed"][:40]:
            print("  REG %6d [%s] %s" % (r["rows"], r["kind"], r["family"]))
    return 0


def standalone_agrees(batched, out, err, rc):
    """Whether a standalone run (stdout, stderr, exit status) says what the batched row said for
    its first subject: the value on stdout with exit 0, or the error as jq prints it with exit 5."""
    kind, val = batched
    if kind == "ok":
        try:
            return rc == 0 and err == "" and json.loads(out) == val
        except ValueError:
            return False
    return rc == 5 and out == "" and isinstance(val, str) and err == "jq: error (at <unknown>): " + val + "\n"


def cmd_validate(args, opt):
    resdir, label = args[:2]
    p = "%s/%s" % (resdir, label)
    batch = {}
    for fn in os.listdir(p):
        if fn.startswith("chunk-") and fn.endswith(".out"):
            with open(p + "/" + fn, encoding="utf-8") as f:
                for line in f:
                    r = json.loads(line)
                    batch[r[0]] = r
    c = Counter()
    bad = []
    for fn in sorted(os.listdir(p)):
        if fn.startswith("standalone-") and fn.endswith(".out"):
            with open(p + "/" + fn, encoding="utf-8") as f:
                standalone_rows = [json.loads(line) for line in f]
            for rid, out, err, rc in standalone_rows:
                b = batch[rid]
                c["rows"] += 1
                if isinstance(b[1], str):
                    c["batch-" + b[1]] += 1
                    continue
                ok = standalone_agrees(b[1], out, err, rc)
                c["agree" if ok else "DISAGREE"] += 1
                if not ok and len(bad) < 8:
                    bad.append([rid, b[1], out[:200], err[:200], rc])
    print(label, json.dumps(dict(c)))
    for x in bad:
        print("  ", json.dumps(x, ensure_ascii=False)[:500])
    return 1 if c["DISAGREE"] else 0


def cmd_versions(args, opt):
    cdir, resdir, cands = args[0], args[1], args[2:]
    corpus = load_corpus(cdir)
    j1, j2 = load(resdir, "jq171"), load(resdir, "jq182")
    cs = {c: load(resdir, c) for c in cands}
    tab = defaultdict(Counter)
    ex = {}
    for rid, row in corpus.items():
        a, b = j1[rid], j2[rid]
        if a == b:
            continue
        k, f = family(row, json.loads(a), json.loads(b))   # "jq" = 1.7.1, "here" = 1.8.2
        if k == "different answer":
            f = row[2] + ("/empty-capture" if b'"offset":-1' in b or b"null" in b else "")
        key = (k, f.replace("jq:", "1.7.1:").replace("here:", "1.8.2:"))
        tab[key]["rows"] += 1
        for c, d in cs.items():
            x = d.get(rid)
            tab[key][c + ("=171" if x == a else "=182" if x == b else "=neither")] += 1
        ex.setdefault(key, [row[2], row[3], row[4], row[5], a.decode()[:160], b.decode()[:160]])
    for key, v in sorted(tab.items(), key=lambda kv: -kv[1]["rows"])[:int(opt.get("--top", 40))]:
        print(v["rows"], key, {k: n for k, n in v.items() if k != "rows"})
        print("    ", json.dumps(ex[key], ensure_ascii=False)[:520])
    return 0


COMMANDS = {"gen": (cmd_gen, 1), "run": (cmd_run, 4), "analyze": (cmd_analyze, 3), "validate": (cmd_validate, 2),
            "versions": (cmd_versions, 3)}


def main(argv):
    if len(argv) < 2 or argv[1] not in COMMANDS:
        print(__doc__)
        return 2
    fn, npos = COMMANDS[argv[1]]
    rest = argv[2:]
    args = []
    while rest and not rest[0].startswith("--"):
        args.append(rest.pop(0))
    if len(args) < npos or len(rest) % 2:
        print(__doc__)
        return 2
    return fn(args, dict(zip(rest[0::2], rest[1::2])))


if __name__ == "__main__":
    sys.exit(main(sys.argv))
