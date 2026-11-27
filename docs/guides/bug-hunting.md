# Differential bug hunting

[Home](../../) > [Docs](../) > [Guides](./) > Differential bug hunting

A **bug hunt** is a time-boxed search for unknown bugs in `succinctly jq` (or `succinctly yq`). It compares the tool
with its reference implementation, and with itself, on generated input, then turns each confirmed difference into an
issue, a fix and a pinning test. It is not the regression suite: the suite and `succinctly dev compare jq` re-check what
is already known, and a hunt looks for what nobody has written a test for yet.

This guide describes the method of the hunt run before a release from 2026-11-21 to 2026-11-25, in three rounds. As of
2026-11-23 00:45 UTC, with fixes still being written, timed and merged, its filing log lists 198 issues (#7425 to
#7815): 133 from the first round and 65 from the second, where the 40 findings of the second round's five hunters
became 38 issues and fixers, timing agents and verifiers filed the rest as follow-ups. Of the 198, 168 were closed:
149 by a merged change to code or tooling, 15 by a recorded divergence with a pinning test, 2 as duplicates and 2 by
other changes. Of the 30 still open, 23 had a fix on a branch or in an open pull request (several of them held for
timing) and 7 had none: design work, or a fix in progress. Another 19 issues opened in the same period are not in the
log: fixers filed some directly (#7575, #7576, #7775), and others concern CI, tooling or yq (#7615 to #7618). The
third round, from 2026-11-23 to 2026-11-25, started no new hunters: it ran the verification work as a hunt
(differentials of the build from before the hunt against `main`, the AVX2 machine, a full `dev compare`, corpora built
for three design studies) and filed 47 more issues (#7826 to #7935); see [The third round](#the-third-round). The
guide is written for whoever runs the next hunt, alone or as a coordinator of several agents. Read it once top to
bottom; the later sections are reference.

- [When to run a hunt](#when-to-run-a-hunt)
- [What counts as a finding](#what-counts-as-a-finding)
- [The known-set check](#the-known-set-check)
- [Strategies](#strategies)
- [Coverage map](#coverage-map)
- [The third round](#the-third-round)
- [Territories and the hunter's report](#territories-and-the-hunters-report)
- [Harness rules](#harness-rules)
- [From finding to merged fix](#from-finding-to-merged-fix)
- [Coordinating several agents](#coordinating-several-agents)
- [Running the next hunt: checklist](#running-the-next-hunt-checklist)

## When to run a hunt

Run one when the board has run dry of reported bugs but you have no evidence the tool is clean, and above all before a
release. The 2026-11-21 hunt started after sweeps that had all reported zero differences: 96,000 grammar-fuzz
programs, 4,999 real JSON files under 10 filters and 20 option sets, the output formats over random arrays, the JSON
test suite and jq's golden cases. In two days it found 130 jq issues, nearly all in dimensions those sweeps did not
generate, and a second round of five hunters found 40 more. A clean sweep says only that the shapes it generates are
clean.

Run another after a large batch of fixes has merged. More than 100 PRs merged during the first round, many of them in
the same input routes, and fixes interact: #7570 was a new bug in the fix for #7455, #7720 came from #7602 and #7795
from #7744, each found by a later hunter or fixer, not by the fix's own tests (see
[Regressions from the hunt's own fixes](#regressions-from-the-hunts-own-fixes)). In the third round at least 16 of
the 47 issues filed were regressions of the hunt's own fixes, and the
[regression differentials](#regression-differentials) found 12 of them.

Do not hunt in the last hours before a release. A finding then has no time to be fixed and verified; see
[Stopping rules](#stopping-rules).

## What counts as a finding

### The four classes

A finding is any one of:

1. **A difference from the reference** in stdout, in the text of stderr, or in the exit status, on the same command line,
   standard input, environment and working directory. "jq rejects it and we accept it" is a difference, and so is the
   reverse: a reference error is behaviour too.
2. **A crash, hang or runaway memory** in `succinctly`: a panic (exit 101), an abort (134), a native stack overflow, a
   signal, no answer in many times the reference's time, or peak memory growing with the input on a route documented
   as flat. What the reference does is irrelevant to this class.
3. **Two routes or two builds disagreeing.** The same data read from a file, a pipe, `/dev/fd/N`, a terminal, split
   across files, under `-n inputs` or through the windowed route; or the same case on the `cli` build, the `cli,simd`
   build and a debug build. A disagreement is a bug even before you know which side is wrong: reduce the case and run
   the reference on the small one to say which.
4. **Wrong at scale.** A correct answer on 1 MB that becomes wrong, refused or quadratic at 2^31 or 2^32 bytes, lines
   or elements, or memory far beyond what [limitations.md](../compliance/jq/limitations.md) documents.

### The reference

`succinctly jq` follows jq 1.7.1 bug for bug ([ADR-0018](../adrs/adr-0018.md)); on macOS that is `/usr/bin/jq`
(`jq-1.7.1-apple`). The repository owner added a rule during this hunt: on every difference from 1.7.1, also run jq
1.8.2, the latest release, and record all three outputs.

- **1.8.2 agrees with 1.7.1:** 1.7.1 is the reference, as before.
- **1.8.2 differs because upstream fixed a 1.7.1 bug** (a crash, a wrong answer, an inconsistency their NEWS, commit or
  PR calls a fix): adopt 1.8.2. Keep the 1.7.1 golden and list the row as `newer_than_primary` in
  `tests/data/jq-golden-known-failures.txt`, record the difference from the pin in limitations.md, and give every
  oracle sweep that judges against 1.7.1 a ledger entry for it in the same change (#7815). Examples: negated literals
  (#7453), `last(empty)` (#7518), NaN with a payload (#7522, a CVE fix), `--indent 0` (#7605), a `--seq` record after
  the last newline (#7718).
- **1.8.2 differs by a deliberate feature or language change:** 1.7.1 stays the reference. The reworded value
  previews in type errors (jqlang/jq#3478) are one.
- **1.8.2's hard limits that protect its C implementation** (the 10,000-level caps on paths, equality, comparison,
  containment and merge; `String too long` at 2^31 bytes; the 1 GiB token and 999,999,999-digit limits; the 4,095
  parameter limit; `Repeat string result too long`): not adopted, by the owner's decision. `succinctly jq` answers
  those inputs correctly and each is a recorded divergence (#7526, #7527, #7540, #7541, #7500).
- **The evidence is unclear:** say so, quote what upstream says, and leave the decision to the owner rather than guess.

Decide from upstream evidence, never from which answer looks better: jq's `NEWS.md` at tag `jq-1.8.2`, the source at
both tags, and the commit or PR that changed the behaviour (`gh api repos/jqlang/jq/...`). The 1.8.2 test files are
vendored in `comparison/jq/vendor/jq-1.8.2/`.

For an edge of jq's input reader (a NUL, a byte-order mark, a fault near a 4,095-byte read, a file boundary), read
jq's own source, `src/util.c` and `src/jv_parse.c`, at each tag. No test file and no NEWS entry states the rules: jq
reads with `fgets` into a 4,096-byte buffer, keeps one parser for the whole run and drops the rest of a read on a
fault, and the answers fall out of that. The third round's fix for #7921 came from reading them. Two cautions from that
work:

- **Check which tag a source file comes from before reasoning from it.** Copies of the reader taken as jq 1.8.2's were
  1.8.0's and 1.8.1's, which read input differently; fetch the file at the tag (`gh api
  'repos/jqlang/jq/contents/src/util.c?ref=jq-1.8.2'`).
- **Read each 1.8 change's intent from jq's own commits, change by change, before adopting it.** Of the ways 1.8.2's
  reader differs from 1.7.1's, the end of the cut at a NUL is a security fix (commit 6374ae0, "Fix NUL truncation in
  the JSON parser", CVE-2026-33948), while the 4,091-byte read is a side effect, on JSON input, of a fix for `-R -s`
  splitting multibyte characters (jqlang/jq#3421), and the closing empty read is the deliberate mechanism of a
  `--slurp --stream` fix (jqlang/jq#3279). Only the first is a fix of the behaviour it changes; for the other two,
  what they do to other routes is a separate decision.

The latest-jq rule above does not reach the reader's facts any more: ADR-0018's #7963 amendment pins them to jq 1.7.1
(the read size, the position after the last read, the closing refill, the depth limit), keeps the 1.8.2 adoptions
already recorded (#7718, #7523, #7807, #5476) as listed exceptions, and revisits the pin when jq 1.9 ships. The same
amendment owes jq only the first fault exactly, with a floor after it, and makes a NUL outside a string a parse error
(#7966, #7967), so a difference after a stream's first fault that keeps the floor is not a bug to file.

`succinctly yq` follows yq v4.53.3 the same way; see [For a yq hunt](#for-a-yq-hunt).

### Capture, never recall

Never state what jq does from memory: run the pinned binary and quote its output. That applies to "jq has no such
feature" as much as to any output. Some behaviour is a property of the platform, not of the version: the last digit
of a math builtin comes from the platform libm ([limitations.md](../compliance/jq/limitations.md#the-math-builtins-print-the-platform-libms-last-bit-as-jq-does-3045)),
jq 1.7.1's casts from double to int are undefined behaviour that x86_64 and aarch64 answer differently
(`halt_error(2147483648)` exits 0 on x86_64 Linux and 255 on aarch64, #7561), and its dates come from the platform
libc, which reads short time-zone names differently on glibc and macOS (#7783). Capture such a behaviour on each
platform it can differ on, and check that a Linux box's `jq` really is 1.7.1: a distribution `jq` may be 1.6.

## The known-set check

Before a candidate is written up, check it against everything already known. Every source, every time:

- [docs/compliance/jq/limitations.md](../compliance/jq/limitations.md): the deliberate divergences and the documented
  gaps. It is long (15,834 lines at `47c652241`), so search it: `git grep -n -i <keyword> -- docs/compliance/jq/limitations.md`.
  Read the lazy-validation sections in particular ("Malformed text a filter never reads is never diagnosed", #3035, and
  the #2103 and #2261 entries): a semi-index validates only what a filter reads, and much of what a malformed-input
  fuzzer reports is that rule.
- The ledger, `comparison/jq/divergences/**/*.toml` (one file per entry; 65 entries at `47c652241`), and its `reason`
  fields.
- The known-failure lists: `tests/data/jq-error-known-divergences.txt`, `tests/data/jq-golden-known-failures.txt`,
  `tests/data/json-test-suite-known-failures.txt` and `tests/data/json-test-suite-i-decisions.txt`.
- Issues, **open and closed**: `gh issue list -R rust-works/succinctly --state all --search '<keywords> in:title,body'`.
- The hunt's own record: the findings other hunters already wrote, the issues filed so far in this hunt, and the open
  pull requests, whose fixes may already cover the case.
- Standing facts: the semi-index validates lazily; jq 1.8-only builtins and documented extensions (`leaf_paths`,
  `isvalid`, `at_offset`, `@dsv`, `@yaml`, `@props`) are not differences; compile-error *wording* is a recorded
  divergence (#2703), so for exit 3 compare the exit status and the number of errors, not the text.

The check is the cheapest step and the one that saves the most. A duplicate costs a filer's verification, then a
fixer's reproduction and design time, then a closed-as-duplicate PR. Of the first 115 findings, 5 were rejected at
filing (a documented extension, a case that already matched 1.8.2, three variants of filed issues), and of 29
follow-ups that fixers reported, 8 were rejected as documented or already in progress. A complete check by
whoever found them would have stopped each one earlier. One duplicate issue was still filed (#7700, a duplicate of
#7699).

## Strategies

Each strategy below says what it generates, what it compares, and what it found. The
[coverage map](#coverage-map) has the volumes. A strategy that finds nothing still produces a result: record the case
count, so the next hunt does not repeat it.

| Strategy                         | Generates                                                                 | Compares                                      | Example issue | Yield                                                                                               |
|----------------------------------|---------------------------------------------------------------------------|-----------------------------------------------|---------------|-----------------------------------------------------------------------------------------------------|
| Grammar and mutation fuzzing     | programs from a grammar, or token edits and splices of seeds              | against jq                                    | #7557         | 5 parser bugs from 146k one-edit mutants; 11 issues from 4.2M two-edit mutants                      |
| Builtin families                 | generated programs per builtin family, each over 2 to 30 inputs           | against jq                                    | #7720         | 7 findings from about 1.4M programs (second round)                                                  |
| Hostile values                   | ordinary programs over extreme values, as input and as literals           | against jq; input vs literal                  | #7453         | 6 issues                                                                                            |
| Route differentials              | the same data through every input route                                   | route against route, and jq                   | #7446         | 13 issues (first round); 12 findings (second round), 6 in the next row                              |
| Split files and unopenable files | a fault in one file and an unopenable path in another, at every cut       | route against route, and jq                   | #7750         | 6 issues; 1,259 of 54,216 lists differed before the fixes, 0 after                                  |
| Fault shapes and positions       | one fault per container shape, or one fault moved across offsets          | reader against reader, and jq                 | #7554         | 3 issues; window-seam sweep 0 of 3,242                                                              |
| Terminals and character devices  | a pseudo-terminal or an endless device as standard input or a named file  | against jq, under a deadline and a memory cap | #7737         | 4 issues, then #7738 and #7812 from their fixes                                                     |
| Build differentials              | the same cases on `cli`, `cli,simd`, debug builds and every SIMD level    | build against build                           | #7725         | 3 issues; `cli` and `cli,simd` never differed                                                       |
| Platform differentials           | the same corpus on Linux x86_64 and macOS arm64                           | platform against platform                     | #7486         | 8 issues (first round); 10 findings (second round)                                                  |
| Scale tests                      | inputs past 2^31 and 2^32 bytes, lines and elements                       | against jq, or route vs route                 | #7425         | 7 issues                                                                                            |
| Resource limits                  | runs under `ulimit -v`, `-n`, `-u`, a full disk, a native-stack floor     | against jq                                    | #7477         | 5 issues (#7476, #7477, #7502, #7576, #7733)                                                        |
| Upstream fixes and test suites   | one probe per entry of jq's NEWS; jq's own test files, three-way          | 1.7.1 vs 1.8.2 vs succinctly, stderr included | #7710         | 21 issues from about 140 upstream entries; 6 findings from the suites, modules and arguments        |
| Real-world programs              | programs people wrote (Rosetta Code, jq wiki, public `.jq` files)         | against jq, time included                     | #7460         | 8 issues from about 33,000 runs                                                                     |
| Call-heavy programs              | recursion grids by leaf and combiner; chains and DAGs of defs and modules | against jq, time included                     | #7786         | #7713, #7786, #7787                                                                                 |
| Complexity probes                | one program at three or more sizes; one quantity varied, the others held  | growth against jq's growth                    | #7563         | 12 quadratic-or-worse issues in the first round; causes found for #7451, #7483, #7697, #7699, #7721 |
| Verification passes              | every filed reproduction, at full size and on both platforms              | against the target reference                  | #7448         | 0 regressions; 3 incomplete fixes completed by follow-up PRs                                        |
| Regression differentials         | the earlier generators, plus new ones weighted to what the fixes changed  | the pre-hunt build against `main`, then jq    | #7887         | 5 clusters in 2.73M cases (#7842 to #7845); 5 more and a slow-down in 1.94M (#7887 to #7889, #7893) |

### Grammar and mutation fuzzing

A grammar fuzzer generates well-formed programs from a model of the language and runs them against jq.
`scripts/jq-grammar-fuzz.py` is the repo's (the `fuzz` dimension of `dev compare jq`); with
`scripts/jq-pattern-fanout-fuzz.py` and `scripts/jq-bind-origin-fuzz.py` it had found nothing in 96,000 programs before
the hunt, and 85,500 more on a debug build during it found nothing again. A grammar only generates what its author
modelled, so the yield came from mutation instead:

- **Byte insertion:** every byte 0x01-0x7f, and 23 multi-byte and control sequences (NBSP, U+2028, BOM, CRLF, ...),
  inserted at every position of 37 seed programs, passed as an argument and through `-f` (74,504 and 27,186 programs).
  It found `.@base64 "x"` refused where jq treats it as a field access (#7456).
- **One token edit** (delete, duplicate, swap, replace or insert one of 78 vocabulary items at each token of 86 seeds,
  146,423 mutants): five parser bugs, among them `def f(a,b)` accepted (#7457), `$__loc__` accepted as a label (#7458)
  and `3end` refused (#7496).
- **Two token edits and splices** (a sub-expression of one seed inserted into another), seeded with the programs of
  jq's own test files for 1.7.1 and 1.8.2 and the real-world corpus (9,128 seeds), each mutant run with `-n`, its seed's
  input and three generic inputs: 4.2 million mutants and 301,000 option-and-route mutants on an x86_64 Linux box gave
  11 issues, among them `break $f.a` refused (#7557), `.[-2][]? = 0` raising early (#7559) and `map(f) | map(g)` running
  `g` before `f` has finished (#7565).

Bucket the differences by a signature (the pair of exit statuses, the first differing token class, the normalised
error text) and investigate the largest buckets first. Run mutants with a short timeout on the reference (2 s) and a
longer one on `succinctly` (10 s), and re-run a one-sided timeout alone before believing it.

For builtins, generate per **family** and spend the volume where nothing has been found yet. The second round did
this: about 1.26 million generated programs over 14 families (math, strings, collections, control flow, dates, number
formatting, sorting, paths, path expressions, flow, idioms, literals, lexing, and a grid shadowing every builtin), each
over 2 to 30 inputs, gave 6 issues. Most families were clean at 20,000 to 240,000 programs each. The findings came
from idioms that combine features, not from single builtins: a `?//` inside `map` in an object value refused at
compile time (#7720), `try` handing a caught `break` to its handler as `null` (#7719), a `def` named like an operator
that jq compiles to a call (#7739), `mktime` answering `-2` where jq raises (#7722), and `combinations` built whole
before its first row (#7726). Regex builtins are a separate territory with their own long tail (#7689, #7804, #7805).

### Hostile values

Keep the programs simple and make the values hostile: every builtin of `tests/data/jq-builtin-names.txt` at each arity,
and every operator, over a pool of about 190 values. The pool covered number spellings at every limit (`-0`, `1e1000`,
`5e-324`, 17+ significant digits, integers around 2^31, 2^53, 2^63, 2^64), strings with NUL, lone surrogates, invalid
UTF-8 and non-BMP characters, empty, deep and duplicate-keyed containers. Feed each value three ways: as input (the
document route), as a program literal and through `--jsonargs` (the owned route). A value that behaves differently as
input and as a literal is a strong lead: that is how `-1 == -1.00000000000000000001` was found to compare the literal
exactly (#7453), and how a NUL in a key was found printed whole in an error message (#7450). Sweep whole classes of
doubles, not only samples: a sweep of every power of two from 2^-1074 to 2^1023, times 3 and times 1.5, showed that
only ±2^-24 printed as a decimal that does not read back (#7730).

### Route differentials

`succinctly jq` reads input through several routes: the whole-file semi-index, the windowed route for large files,
lists of files and pipes, the lazy `input`/`inputs` reader, `-R` raw lines, `--seq`, `--stream`, `--slurp`, the
streaming fast path for a plain filter, and path-mode evaluation for writes. The same bytes and program must give the
same stdout, stderr (file names normalised) and exit status on every route, so a route differential needs no reference
to find a bug. This was the hunt's richest vein in both rounds.

Cross four dimensions:

- **Delivery:** a named file, `< file`, a pipe, `/dev/fd/N`, `/dev/stdin`, a FIFO, a terminal, `-` among named files,
  the same file twice, standard input at a non-zero offset, the same bytes split across 2 to 40 files at every kind of
  boundary (inside a token, between documents, inside a BOM), with an unopenable path among them.
- **Program:** about 40 that read different amounts (`.`, `.a`, `.[0]`, `first`, `length`, `-n input`, `[inputs]`,
  `first(inputs)`, `limit(1; inputs)`, `input_line_number`, `input_filename`, `halt`, `error` at document K, `tostream`).
- **Flags:** `-c -r -j -a -S -s -n -R -e --seq --stream --stream-errors --tab --indent N`, `--arg` and its family, in
  combinations of two and three.
- **Size and fault position:** sizes just below, at and above each route threshold, with any fault placed at a window
  or piece boundary and at plus or minus one byte. Find the thresholds in the source, not from memory: the window
  (`WINDOW_BYTES` in `src/bin/succinctly/jq_runner/window.rs`, 256 KiB since #7412), the size at which a route switches
  to windows (four windows, 1 MiB, for a file; two, 512 KiB, for a pipe; a sixteenth of a window, 16 KiB, for a lone
  `--seq` file: the `min_windowed_*_bytes` functions there), the lazy reader's prefix (`LAZY_PREFIX_BYTES` in
  `jq_runner.rs`, 256 KiB), the 32-file hand-off, the `--seq` record cap, and jq's own 4,095-byte read chunk, which
  decides what `input_line_number` and `(at FILE:N)` report.

A debug build reads a smaller window from `SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES` (8192 to 262144), so a debug hunt can
cross window boundaries with inputs of tens of KiB ([developer.md](developer.md#testing)). The release window is a
constant: re-check a debug-window finding on a release build at full size.

Also test what happens to the descriptors around a run, which a single-run differential cannot see: whether a
redirected standard input is left where jq leaves it after an early exit (`{ jq -n input; jq -n input; } < f`, #7455),
and whether a file that grows while it is read is read to its new end (#7446).

The first round's routes pass found 6 issues, among them lines appended during a run dropped (#7446), the stdin offset
after an early exit (#7455), and about 23 builtins running a generator argument to its end before their first output
(#7474). Its `--stream` pass (inputs split across 2 to 40 files, 5 to 8 MiB streams, 36 fault classes, 27 flag sets)
found 7 more, among them a BOM split across the first files (#7566) and the new #7570. The second round crossed the
same dimensions with the first round's fixes in place (14,246 route-differential cases; 23,324 self-differential
cases, the same program under `-e`, `--unbuffered`, `--arg`, `--indent 2`, `-C`, wrapper programs and other deliveries;
a 13,629-case fault-offset sweep) and found 12, half of them in one family (next section). The others: a NUL right
after a number refused on the lazy stdin route (#7748, #7749, residue of #7437), `-e` going on with a document after a
fault the program reached (#7757), and a lone surrogate raising an error `try` cannot catch through `input` (#7758).

A hang that only one tool shows under stress is not always that tool's bug. A named-pipe stress found 2 hangs in
73,092 runs of succinctly and none in 65,497 of jq; a fixer then showed that the macOS kernel loses the FIFO's end of
file, and that jq, `cat` and a plain `read()` hang on it the same way (#7304). Run a rare hang's harness with the
reference and with `cat` at the same volume before filing it.

### Split files and unopenable files

Give a file list a fault in one file and an unopenable path (missing, a directory, mode 000) in another, and cut the
same bytes at every kind of boundary. jq reads the files as one stream and reports an open failure when it reaches the
file; succinctly's stitcher, its early checks and its `-s` route each decided on their own which error comes first and
whether the run goes on. About 24,000 cases found six issues: a run that stops for good after a caught fault (#7750),
`input_filename` naming the last file (#7751), a scalar printed and its parse error lost (#7752), an open failure
reported before an earlier fault (#7753), and two `-s` shapes (#7754, #7755). The fix's own sweep of 54,216 lists
went from 1,259 differences in the family to 0, with 19,612 control lists unchanged; one shape (validation that depends
on where a file cut falls) was recorded instead. The fix's timing pass found the stitcher already quadratic in the
number of files for one document spread across them (#7792): time a multi-file fix over several file counts.

### Fault shapes and fault positions

A semi-index validates what a filter reads, so its fast paths can answer from text a slower reader would reject. Map
this systematically rather than by example: every one-fault container shape (`[1,]`, `[,1]`, `[1 2]`, `{"a":1,}`,
`{"a" 1}`, `{"a":1 "b":2}`, at depth 1 to 3, as an NDJSON line, as an element of a 5 MiB array) against every reader
that could take a short cut (`first`, `last`, `.[0]`, `nth(0)`, `any`, `all`, `has`, `length`, `keys`, `getpath`,
`to_entries`, `.a`, ...). The documented lazy rule explains most cells. What matters is an inconsistency it does not
explain: two readers of the same element that disagree, or a value taken from across the fault. About 37,000 small and
19,000 large runs found three: `[,1] | first` printing `1` where `.[0]` raises (#7554), `all(f)` and `any(f)` answering
across a gap where `all(.[]; f)` raises (#7555), and an absent-key lookup ignoring a missing separator that `has`
reports (#7556).

A **fault-position sweep** moves one malformed byte, or one window boundary, across every offset of a document. The
window-seam sweep placed 37 valid values so a 256 KiB window boundary fell at every byte inside them, and 30 faulty
tokens at 5 offsets each: 0 of 3,242 cases differed. Invalid UTF-8 at jq's 4095, 8190 and 12285 read boundaries gave 0
of 5,184. A stdin-offset fuzz (where a run that stops early leaves a redirected standard input, 1.77 million cases) is
the net for #7570, a stop on an event completed by the last byte of jq's 4,095-byte chunk: on a build without that
fix it finds 16 differences in 12,000 cases, and with it 0 in 246,011.

### Terminals and character devices

A terminal and an endless device are inputs no file-based differential reaches: the first round fixed pipes, FIFOs
and sockets that never end (#7478), and nothing covered a character device. Drive them through a pseudo-terminal
opened in the test with `openpty(3)` (`tests/common/pty.rs`; Linux names the slave side `/dev/pts/N`, macOS
`/dev/ttysNNN`), as standard input and named on the command line, with lines typed one at a time and `^D` at the start
of a line. Give every run a deadline, since a run that waits for input it is never given otherwise hangs the suite, and
give a run over an endless device a memory cap that kills it (`Running::wait_within`).

This found an endless device named as input read to its end before the program ran (`-R -n 'input|length'
/dev/urandom`, #7737), `--stream` over a terminal named by path losing its first three bytes (#7740), a second `^D`
needed on a terminal (#7741), and `-n` with a terminal on standard input seeing an empty stream (#7742: a deliberate
exception that had never been recorded). The fixes found two more: a stream whose front is not UTF-8 was read whole too
(#7738), and a fault inside a container that never closes was read on forever looking for the close (#7812).

### Build differentials

The shipped binary is `--features cli`. Nearly all earlier fuzzing and the whole jq comparison had run on
`cli,simd`, and a debug build turns on overflow checks and `debug_assert!`s that a release build skips. Build all three
from one commit and require identical stdout, stderr and exit status:

- **Scanner boundaries:** strings whose closing quote, backslash run (`\\\"`), escape or non-ASCII byte falls at every
  offset relative to 16-, 32- and 64-byte blocks; every document length 0 to 300 (600 on x86_64); long whitespace runs.
- **Tree navigation:** documents of 3 to 500,000 nodes in chain, wide, balanced and random shapes, sized at word and
  block boundaries, under navigation programs.
- **Mutated documents** of the JSON test suite and the golden cases, checked for panics on the debug build.
- **Every SIMD level and CPU:** AVX-512 (an AMD Ryzen 9 7950X), AVX2 without AVX-512 (an Intel i7-6700), NEON (Apple
  M-series), and the levels `SUCCINCTLY_SIMD` can force on x86_64 (`scalar`, `sse2`, `sse4.2`, `avx2`;
  [environment-variables.md](../reference/environment-variables.md#succinctly_simd)). Any difference between levels is
  a bug whatever jq says.

`cli` and `cli,simd` never disagreed in either round: in the first, 18,474 scanner cases on Apple silicon, 551,200
block-boundary documents and 3.84 million mutants on the 7950X, and about 880,000 cases on the AVX2 machine; in the
second, about 550,000 cases on the 7950X (AVX2 and `sse2` forced, a debug build, a debug build with an 8 KiB window)
and 1,200,937 on the AVX2 machine, which also passed the full test suites on three feature sets. The yield came from
mutation and program size, run on all three builds: a malformed document under 44 open arrays panicked in
`src/trees/bp.rs` on every build (#7482, one hit in 45,000 mutants), and a program of 100,000 `as` bindings overflowed
the native stack (#7475; the threshold lies between 45,000 and 60,000). A fixer found that the NEON and portable index
builders disagree on one malformed token (#7539), which no CLI run had shown: when a library-level differential is
cheap, run it too.

They did not disagree in the third round either. The AVX2 machine ran 580,218 cases at `77bff4b58` and 185,620 more
at `e43c3f9c4`, from a generator weighted toward the merged fixes (files cut inside escapes and multibyte
characters, byte-order marks, `--seq`, `-R`), on `cli`, `cli,simd` and `cli,portable-popcount`, and on `cli,simd`
with `sse2` forced: 1,488,692 and 734,975 variant runs with no mismatch between them, and the test suites green on
that CPU each time. Its findings were #7866, which is not CPU-specific, and two more shapes of #7888's error order.

A third run on the same machine, at `a9da4e814` (the code of the release candidate), added a mode that puts faults at
jq's 4,095-byte read boundaries (#7921, #7897) and ran `cli,simd` with `sse2` forced as a fourth build. The four
builds never disagreed: 602,250 cases and 1,742,616 variant runs with no mismatch, and on that CPU 6,341 library tests,
5,206 `jq_cli_tests`, 3,698 `yq_cli_tests` and all 27 deep-recursion tests passed. Against `e43c3f9c4`, 24 rows
changed. The 22 in the read-boundary mode are all improvements. Of the 2 regex rows, one is #7878's open shape, wrong
before and wrong in other words now. The other is an error-order regression under the `x` flag: an option group with a
blank before its `)` is reported as a later repeat error (`(?i ) ?`, `(?ix )(?<= ?)`, `(?i )(?= ?)` are `target of
repeat operator is not specified`, where jq says `undefined group option`). `(?i ) ?` and `(?i )(?= ?)` already
answered so at `e43c3f9c4`, and `(?ix )(?<= ?)` is new since; it is recorded as a comment on #7929. Like #7866, it is
not CPU-specific.

A **debug-versus-release differential** finds what only an internal assertion sees. Run every generated case on both
builds and keep the cases where only the debug build differs. In about 135,000 generated programs the 562 debug-only
differences had one cause: negating the input 2^63 left a number whose stored form disagreed with its own text, which
the release build printed correctly and a `debug_assert!` caught (#7725). After the fix, 117,000 cases were clean.

### Platform differentials

Run the same corpus on Linux x86_64 and on macOS arm64, against jq 1.7.1 on each. One pass on an x86_64 Linux box found
8 issues, 5 of which the Macs could not show:

- **libc:** time zones `posix/...`, `right/...` and `TZDIR` read differently from glibc (#7486); `now` with nanosecond
  resolution where jq has microseconds (#7484).
- **Double-to-int conversion:** `"x" * n` for n of 2^31 or more, where jq 1.7.1 on x86_64 answers `""` (#7500);
  `halt_error(n)` exit codes (#7561). `9223372036854775808 % 5` also differs on x86_64, but jq 1.8.2 fixed it and
  succinctly already answers as 1.8.2, so it was rejected, not filed.
- **Descriptor semantics:** Linux has `/dev/full` (a stderr write failure panicked, #7476) and enforces `ulimit -v`
  (#7502); Linux opens `/dev/stdin` and `/dev/fd/N` again from offset 0, where macOS shares the open file, so a test
  of descriptor offsets has two sets of expectations (#7687).
- **Locale:** a test that passed under `LC_ALL=C` failed under `LANG=en_AU.UTF-8` (#7612). Hunt under more than one
  locale.

The second round's x86_64 and Linux hunter found 10 more, most of which reproduce on every platform once seen: SIGPIPE
inherited as ignored reset to the default, so a closed stdout killed the run where jq reports `writing output failed`
(#7732; services, Python's `subprocess` and CI runners often start a process with SIGPIPE ignored); every run failing
at the user's process limit because the evaluation thread could not be spawned (#7733, Linux only); environment
entries without `=` dropped (#7734); `nan | gmtime` answering as arm64 jq does where x86_64 jq raises (#7735, recorded:
undefined behaviour in jq); 2^-24 printed as a decimal that does not read back (#7730); `range` past 2^53 stepping in
exact integers where jq steps in doubles (#7731); and the four terminal and device issues above.

Dates depend on the platform's libc as much as on jq. A three-way date pass (macOS jq 1.7.1, jq 1.8.2 and succinctly,
with glibc jq 1.7.1 for time-zone questions) of 225,000 evaluations over 82 `TZ` values (51 IANA zones with 30- and
45-minute offsets and historical changes, POSIX strings, `:path`, empty, unset and invalid values), at DST edges,
before 1970 and 1900, past 9999 and at year 0, plus grids of 627 `TZ` spellings and of 25 locales by 49 formats, found
two: POSIX `TZ` strings with a one-digit minute or a three-digit hour read as UTC (#7783), and `%+` neither read back by
`strptime` nor localized (#7784). Forms that glibc and macOS read differently from each other (zone names shorter than
three characters) were recorded as platform-dependent.

Math digits from the platform libm are known and recorded; do not report them.

### Scale tests

Nothing in the routine tests went past 1 GB. Generate, with a fast tool rather than a Python loop, inputs past each
32-bit limit: one document over 2^32 bytes, a string over 2^31 bytes, more than 2^32 lines, a stream past 4 GiB.
Search for the counters first (`git grep -n 'u32' -- src/json src/bin/succinctly/jq_runner.rs`) and aim an input at
each. Use cheap queries (`length`, `.[N]` past the limit, `input_line_number`, `-c .` hashed on both sides rather than
diffed in memory), and compare with succinctly's other routes where jq is too slow.

This found a 4 GiB document panicking (#7425), most programs refused on one (#7537), `input_line_number` wrapping at
2^32 (#7472), parse-error positions past 2^31 (recorded as a divergence, #7461), and memory growing with the input on
routes documented as flat (#7447, #7480). Verifying these fixes needs the same sizes: the fourth verification pass ran
the 4.3 GB rows on the x86_64 Linux box at `47c652241` (#7425, #7472 and #7537 fixed; #7461, #7540 and #7541 recorded
accurately), where jq 1.7.1 took 221 s to count 2^32 newlines (AMD Ryzen 9 7950X).

### Resource limits

Run under limits jq copes with: `ulimit -n 4` (one free descriptor; #7477), `ulimit -v` below the file size (Linux
only; #7502 and #7576), `ulimit -u` at the process count (Linux only; #7733: jq is single-threaded, succinctly spawns an
evaluation thread), a small `RLIMIT_STACK`, a full disk or `/dev/full` as stdout or stderr (#7476), stdout closed or
`| head -1` (SIGPIPE, with SIGPIPE both default and ignored, #7732), stdin closed. Programs of tens of thousands of
bindings or operators test the native-stack floor (#7435, #7475, #7676). macOS does not enforce `RLIMIT_AS`, so
memory-limit rows need Linux. Stack limits from 64 KiB, `RLIMIT_NOFILE` and `RLIMIT_DATA` were clean in the second
round.

### Upstream fixes and the reference's own test suites

List every upstream entry between the pinned and the latest reference (jq's `NEWS.md` for 1.8.0 to 1.8.2 and the
commit log between the tags), write one probe per entry, and run it on 1.7.1, 1.8.2 and succinctly. Classify each
entry as a bug fix, a security fix, a deliberate change or not applicable. A bug fix that succinctly does not follow is
a finding. Of about 140 entries, 27 were findings (filed as 21 issues between #7518 and #7542), about 18 were already
adopted, 8 were already filed, about 60 did not apply and 21 were deliberate changes, kept in a separate
list for a future decision to move the pin.

jq's own test files are an oracle-independent corpus. The repo runs 1.7.1's (the golden cases in
`tests/data/jq-golden/` and the vendored `comparison/jq/vendor/jq-1.7.1/`); 1.8.2's are vendored but informational. Of
the vendored 1.8.2 cases, 119 differ between the two jq versions: succinctly matched 1.8.2 on 29, 1.7.1 on 68 and
neither on 22, and each of those was accounted for. jq's shell tests (`tests/shtest`) are scripts: never run a
downloaded one; read it and re-express its cases as your own commands.

Run the test files **three-way, stderr included**. The repo's suite runner compared stdout and exit status only, so
upstream's own `%%FAIL` case for an interpolated import path (`include "\(1)"; 0`, `Import path must be constant`) had
never been checked; the second round's three-way run of both tags' files found it (#7710, whose fix also made
`dev compare jq` compare the `%%FAIL` stderr). That pass, with the shell tests re-expressed as 570 scenarios, 213,104
generated module programs, `JQ_COLORS`, `-f` and argument-order matrices, found 6 issues: an unopenable module
imported twice reported once (#7709), the syntax error at the end of a program that ends in whitespace placed on the
wrong line (#7711), option errors not reported in argument order (#7712), `--run-tests` refused (#7715, recorded as
unsupported), and the interpolated import path. The sixth, tree recursion refused at about 8,192 leaves, is in
[Call-heavy programs](#call-heavy-programs).

### Real-world programs

Generators do not write the programs people write. A corpus of 4,951 programs (jq's test files, the jq wiki, 2,283
Rosetta Code solutions, 1,376 public `.jq` files) and 4,845 `jq '...'` commands taken from public shell scripts and CI
files, run over a small set of generic inputs, gave about 33,000 paired runs. jq's own test cases all agreed apart
from the recorded compile-error wording, but long programs found what nothing else did: equal-length slices of one
array that jq compares as equal (#7459), recursion refused after about 8,000 sequential calls (#7460), a
shebang-polyglot comment continuation (#7481), a lost compile error (#7443), and four quadratic or worse
accumulators (#7451, #7452, #7473, #7483). Treat downloads as untrusted data: keep them in their own directory and
pass programs only as arguments.

### Call-heavy programs

A program that calls a def many times tests what the evaluator does per call, and a program with many defs tests its
analysis. Generate both:

- **Recursion grids:** a tree recursion `def t($n): if $n <= 0 then LEAF else COMBINE end; 0 | t(13)` crossed over leaf
  shapes and combiners (79 leaves by 41 combiners, plus closures, module hops and mutual recursion; 4,800 programs).
  jq answers every one. After the first round's fix for linear recursion (#7460), succinctly still refused trees of
  2,048 to 8,192 leaves for many shapes (#7713), and after that fix for 19 leaves and 9 combiners (#7787). A refusal is
  a wrong answer here: jq prints a number.
- **Def DAGs:** chains in which each def calls the one below twice, Fibonacci-shaped chains, and layers of modules
  each importing the whole layer below. jq's time stays flat; succinctly's analysis grew 8 times every three levels,
  even for a call in code that never runs (`reduce empty as $i (0; g26)`, #7786). Its fix has a 30,200-program DAG
  corpus as the regression net, and the same shape in yq mode is #7788.
- **The constant per call against jq,** on the benchmark machines: what remains after the fixes above is design work
  (#7699, #7803).

### Complexity probes

Time one program at three or more sizes, doubling each time, on both tools. A factor of about 4 per doubling where jq's
time stays flat is quadratic; 8 is cubic. The same binary pair at three sizes on any machine is evidence of an
algorithmic bug, but label it indicative: it is not a benchmark number (see
[The performance hold](#the-performance-hold)). On a loaded machine use user CPU time or instruction counts
(`valgrind --tool=cachegrind`) instead of wall time. The probe also confirms a fix: the slope should change, not just
the constant.

A cheaper variant is **time-ratio fuzzing**: keep the mutants where succinctly takes 10 or more times jq's time. That is
how a negative-index assignment in `reduce` (#7563) and a dead `1 |= 1` branch slowing `add` 15 to 80 times (#7564)
surfaced. The quadratic-or-worse issues of the first round were #7451, #7452, #7470, #7471, #7473, #7483, #7495, #7563,
#7564, #7567, #7610 and #7611.

Once a probe shows growth, **attribute it before fixing it**: vary one quantity while holding the others constant,
and see which one the time follows ([benchmarking.md](benchmarking.md#ab-benchmarking-method)). Each of these tables
named a different term from the one first suspected:

- **#7483:** with the number of calls fixed, time grew with the size of a member the program never touches, so the
  cost was a re-index of the whole value at every def call, not copy-on-write.
- **#7451:** with the step count fixed at 500, growing the starting accumulator grew the time for `. + [range(1)]` and
  left `. + [1]` flat: a step the in-place walk had no stage for went through a route that copied the accumulator.
- **#7721:** with the steps fixed the time followed the size of a bound array, and with the array fixed it followed the
  steps, while jq stayed flat on both: the term was container size times reads (a re-index per `getpath`), not the
  accumulator.
- **#7699:** a never-taken branch of `k` field reads added to the body, with calls and evaluated work constant, made the
  cost per call follow `k`: the body is copied per call. The model predicted a 20 to 25% instruction saving from
  removing one copy, and the fix measured 23.5 to 25.7%.
- **#7697:** a cost found while attributing an earlier fix was glibc's `malloc_consolidate` sweeping a document that
  path mode had freed mid-run; a malloc tunable that removes the sweep (`GLIBC_TUNABLES=glibc.malloc.mxfast=160`)
  confirmed the mechanism before any code changed.

### Verification passes

After fixes merge, re-run every filed reproduction against a fresh build of `main`, on all three tools, and classify
each issue as FIXED (byte-identical to the target reference), RECORDED (limitations.md names it), STILL-DIFFERS or
CANNOT-RUN (needs a size or platform the pass does not have). Keep one case script per issue so each pass re-runs the
same rows, and compare each pass with the previous one so a row that was FIXED and now differs stands out.

The hunt ran four passes: on the combined tree of unmerged fixes (107 issues, 0 regressions), on `main` at
`41642530` (86 FIXED), at `b7e58eb2a` (of 129 jq issues, 99 FIXED, 9 RECORDED, 10 STILL-DIFFERS, 11 CANNOT-RUN, 0
regressions), and on the x86_64 Linux box at `47c652241` for the rows that need Linux or 4 GiB inputs. The second pass
found three fixes incomplete on shapes their tests did not cover (a nested one-member object, #7448; `/dev/fd/7`,
#7446; `/dev/stdin`, #7568), and each got a follow-up PR on the closed issue: two fixes and one recorded divergence.
A fifth pass, at `77bff4b58` in the third round, covered 83 issues (64 FIXED, 12 RECORDED, 7 still differing, all
of them open, partial or a duplicate) and re-checked 30 from earlier passes, with no row regressed. Its own
differentials around the newest fixes found three more: a recursion over a parenthesised `foreach` leaf under string
interpolation refused at depth 12 since #7828 (#7846), a write to a closed stderr pipe on macOS that sometimes exits
normally, dating from #5547 (#7847), and what was left of #7483 and #7789 (#7848). No separate pass ran at the
release candidate, `6362c4bc4`: the last [regression differential](#regression-differentials) ran at `a9da4e814`,
whose code the candidate shares (it changes no source file, only the link-order list, its tooling and docs), and
stands in for one.

Verify on the other platform too. Two fixes verified on macOS (the descriptor-offset tests of #7686 and #7687) carry
separate Linux expectations; the fourth pass ran those expectations against jq 1.7.1 on Linux, 48 runs, and all
agreed.

### Regression differentials

A verification pass re-runs what was filed; a **regression differential** looks for what the fixes broke. Build
`succinctly` at the commit the hunt started from (here `73f049b2c`, the commit of the last committed `dev compare`
runs) and at current `main`, run both on every generated case, and run jq 1.7.1 only where the two differ. Classify
each differing case on stdout, stderr and exit status:

- **REG:** the old build matched jq on stdout and exit status and the new one does not; **REGERR:** the same on
  stderr alone. These are the leads.
- **FIX:** the new build matches jq on all three and the old one did not.
- **NEWCRASH:** the new build panicked, was signalled, timed out or was killed for memory where the old one was not.
- **OTHER:** neither build matches jq. Not a regression by this definition; scan it for new crashes and leave it.

It finds what a fix's own tests cannot. A fix's test pins the shapes its author had in mind, over a matrix built for
that issue; a regression differential runs every shape the generators make against the build before any fix, so a
fix that changes a neighbouring shape shows up as old-agreed and new-differs whether or not anyone wrote a test for
that shape, and jq decides which side is right, so no expectation has to be written. It is also cheap: two
`succinctly` runs per case, and jq only on the few percent that differ. Expect most REG rows to be deliberate
changes, above all adoptions of jq 1.8.2 (`[last(empty)]` is `[]` now, so every row that reaches it differs), so
classify each family against [limitations.md](../compliance/jq/limitations.md) by one example before reading rows, and
re-run each real lead by hand on both builds and both jq releases.

Weight the generators toward what changed. List the merged fixes by area, add generators for those areas, and keep
the earlier hunters' generators unchanged beside them as the control.

The third round ran three, each finding five clusters:

- **x86_64 Linux** (an AVX2-only Intel i7-6700), about 2.73 million cases against `main` up to `77bff4b58`. Four
  live regressions, each traced to a merged fix: a stream's BOM read as text after a file that does not open
  (#7842, from #7646), `path(inside(.b |= 1))` reporting the wrong path error (#7843, from #7543), a regex error
  reported in the wrong order (#7844, from #7745) and a native builtin with `inputs` as its argument draining the
  stream before it raises (#7845, from #7543). The fifth, a cut document read in quadratic time, was already fixed
  (#7792).
- **macOS arm64** (an Apple M5), 1,943,904 cases at `e43c3f9c4`, the commit first meant as the release candidate,
  weighted toward regex, named arguments, label and `break`, builtin arguments that read `input`, deep recursion,
  byte-order marks, truncated UTF-8 under `-R`, closed pipes and dates. Five clusters: three regex regressions under
  the `x` flag from #7865 (#7887; one of its rows only looked new, and its cause predates the hunt), regex error
  order from #7852 (#7888), and a `--seq` position from #7775 (#7889). A flag for a recursive generator whose time
  grew with the square of its depth, with the same output, became #7893: quadratic since #7702, and since #7856
  for the same recursion over a `$n` parameter. No crash, abort or hang was the new build's alone.
- **macOS arm64 again** (an Apple M5 laptop), 1,806,339 cases in 31 steps, `73f049b2c` against `a9da4e814`, the code
  of the release candidate, with a build of `e43c3f9c4` to tell what changed since the second run from what was
  already there. New generators covered 4,095-byte reads, closure arguments that grow at each level, `--argjson` texts,
  the regex shapes of #7887 and #7888, and `--seq` with `halt`. It counted FIX 231,370, REG 27,141, REGERR 5,378,
  NEWCRASH 4 and OTHER 23,318, and 1,501 rows that differ only by a recorded normalisation (#7453, #7605, #7534).
  Five clusters, all filed: a doubly recursive def with a `try` in a closure argument that takes about 16 s where jq
  answers at once, bisected to PR #7828 for #7787 (#7928); regex error order and answers changed by the fixes for
  #7887 and #7888 (#7929); regex rows that expose older gaps (#7930); a wrong position and a fault reported twice at a
  BOM-led 4,095-byte read boundary (#7931); and exit 0 with `[]` where jq exits 5, after a BOM, a fault and a number
  ending in a NUL at the read boundaries, very likely from #7923 (#7934). #7928 and #7931 were already in
  `e43c3f9c4`; #7929 and #7934 are newer. Both builds passed 2 GiB on a recursive collect-and-reyield that jq answers
  in 2.6 MB, a lead outside the regression scope, filed as #7935.

The regex compiler is where fixes broke fixes fastest: in one day #7865 led to #7887, and #7852 with #7858 led to
#7888, each fix's own sweep clean, and the fixes for #7887 and #7888 led in turn to #7929. A rule decided by several
hand-written passes over the same text fails this way (see the regex proposal in
[Three design proposals](#three-design-proposals)).

## Coverage map

Volumes are approximate and come from the hunters' reports; "zero" means no new difference after the known-set check.
Re-fuzzing a zero row at the same volume is unlikely to pay. A row with findings may hold more: most hunters stopped on
a time box, not on exhaustion.

### First round (2026-11-21 and 2026-11-22)

| Area                                                         | Approximate cases                                                    | Outcome                                                                     |
|--------------------------------------------------------------|----------------------------------------------------------------------|-----------------------------------------------------------------------------|
| Expression grammar fuzz (repo fuzzers)                       | 96,000 before; 85,500 on debug; 10,000 on the combined fix tree      | zero                                                                        |
| Program lexer: byte and sequence insertion                   | 101,690 programs                                                     | #7456; the rest known (non-ASCII identifiers #7433, NUL in `-f` #7434)      |
| One-token program mutation                                   | 146,423                                                              | #7456, #7457, #7458, #7496, #7497                                           |
| Two-token mutation and splices                               | 4.2 million, plus 301,000 option and route mutants                   | 11 issues (#7557 to #7565, #7572, #7574)                                    |
| Builtin by argument-variant grid                             | about a third of the builtins                                        | the split-flags family (#7560); not finished                                |
| Hostile values through every builtin and operator            | about 3,500 batched runs; 20,000 doubles; 6,000 number pairs         | #7426, #7435, #7436, #7445, #7450, #7453                                    |
| Number spellings in input                                    | 18,048 runs                                                          | #7468, #7469                                                                |
| Malformed input mutation (22 mutation kinds)                 | about 900 documents by 32 programs, file and pipe                    | #7427, #7437, #7448, #7449, #7454                                           |
| Invalid UTF-8 in input, including at jq's read boundaries    | 13,984 runs                                                          | zero                                                                        |
| One-fault container shapes by 100 readers                    | about 37,000 small and 19,000 at 5 MiB                               | #7554, #7555, #7556                                                         |
| Window-seam sweep                                            | 3,242                                                                | zero                                                                        |
| `scripts/jq-windowed-file-fuzz.py`, all modes, new seeds     | about 43,000 over all runs                                           | zero                                                                        |
| Route differential, plain and `-n` programs                  | about 10,000 (routes, metamorphs, special stdin shapes)              | #7444, #7446, #7455, #7463, #7474, #7478                                    |
| `--stream`, `--stream-errors` and input programs over splits | about 27,500, plus 1.77 million offset and 8,100 trickled-pipe cases | #7566 to #7571, #7573                                                       |
| CLI argument matrices                                        | 3,299 rows when re-run on the fixes (first-run volume not recorded)  | #7438, #7439, #7462, #7494                                                  |
| Modules, `-f` files, colours, time, environment              | about 33,000                                                         | #7428 to #7434, #7440 to #7442                                              |
| Scale past 2^31 and 2^32                                     | a few dozen multi-GB runs                                            | #7425, #7447, #7461, #7470, #7472, #7480, #7537                             |
| cli vs simd vs debug on Apple silicon (NEON)                 | 18,474 scanner, 2,358 file, 850 large-stream, 720 tree, 1,100 DSV    | #7475, #7482; builds never differed                                         |
| Debug-build panic hunt (hostile corpus, mutants, sweeps)     | 14,616 hostile jobs; 45,000 mutants; 22,000 `--seq` sweep cases      | #7482                                                                       |
| cli vs simd on x86_64 with AVX-512                           | 551,200 block-boundary, 246,800 string, 3.84 million mutants         | zero                                                                        |
| cli vs simd on x86_64 with AVX2 only                         | about 880,000, plus the three test suites                            | zero                                                                        |
| DSV input against Python's `csv`                             | 282,000 on x86_64, 76,200 on AVX2                                    | zero                                                                        |
| glibc and Linux I/O against macOS                            | 9,408 time and locale, 6,000 number, 84 I/O scenarios                | #7476, #7477, #7484, #7486, #7487, #7500, #7502                             |
| jq 1.8 upstream fixes                                        | about 140 upstream entries; the vendored 1.8.2 test files            | 21 issues                                                                   |
| Real-world programs                                          | about 33,000 paired runs                                             | 8 issues                                                                    |
| Regex builtins                                               | 632 cases in the values pass; 37,067 rows in a dedicated fuzz        | #7520, #7528, #7654, #7669, #7670, #7682; 1,401 rows still differed (#7689) |

### Second round (2026-11-22 and 2026-11-23)

Five hunters, one territory each, on `main` with the first round's fixes merged (`47c652241`; the last hunter on a
later `main`). Their findings by territory:

| Territory                                                                      | Machine                                           | Findings                                                                         |
|--------------------------------------------------------------------------------|---------------------------------------------------|----------------------------------------------------------------------------------|
| Language and builtins, generated per family                                    | an Apple M4 Pro Mac mini                          | 7                                                                                |
| Routes, input shapes and flags                                                 | an Apple M4 Pro Mac mini and a development laptop | 12, mostly file lists with a fault in one file and an unopenable file in another |
| jq's test suites, modules, the argument family                                 | an Apple M5 laptop used for builds                | 6                                                                                |
| x86_64 and Linux                                                               | an x86_64 Linux box (AMD Ryzen 9 7950X)           | 10                                                                               |
| Dates outside UTC, call-heavy programs, module compile errors, large documents | an Apple M5 laptop used for builds                | 5                                                                                |

| Area                                                                               | Approximate cases                                                                                       | Outcome                                                                                     |
|------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------|
| Builtin families (14), generated, each program over 2 to 30 inputs                 | about 1.26 million programs                                                                             | #7719, #7720, #7722, #7723, #7726, #7739                                                    |
| Debug against release, generated programs                                          | about 135,000                                                                                           | #7725                                                                                       |
| Directed builtin grids (generator laziness, number texts, lexing, `$ENV`, `halt`)  | not counted                                                                                             | zero                                                                                        |
| Route differential with the first round's fixes                                    | 14,246 route, 23,324 self-differential, 13,629 fault-offset cases                                       | #7748, #7749, #7757, #7758                                                                  |
| Split files by unopenable files                                                    | about 24,000; 54,216 lists in the fix's sweep                                                           | #7750 to #7755                                                                              |
| Route grids (BOM, `-` in lists, `-e`, raw input, early exit, nesting, path mode)   | about 20,000                                                                                            | zero                                                                                        |
| Named FIFO stress                                                                  | 73,092 and 279,575 runs                                                                                 | the macOS kernel (#7304), not succinctly                                                    |
| jq's test files at 1.7.1 and 1.8.2, three-way with stderr                          | every case of the upstream files                                                                        | #7710                                                                                       |
| jq's shell tests, re-expressed                                                     | 570 scenarios, 906 prefix cases by 6 modes, 160 random-byte `--seq` cases                               | zero                                                                                        |
| Modules, generated and by hand                                                     | 213,104 generated, 166 by hand                                                                          | #7709, #7713                                                                                |
| CLI arguments, `JQ_COLORS`, `-f` wrappers                                          | 7,500; 1,499; 8,460                                                                                     | #7711, #7712, #7715                                                                         |
| SIMD levels and builds on the AVX-512 box                                          | about 550,000                                                                                           | zero                                                                                        |
| AVX2-only hardware: three test suites and cli vs simd                              | 1,200,937 cases; 15,956, 15,943 and 5,828 tests                                                         | zero                                                                                        |
| One corpus across macOS arm64, Linux `cli`, `simd`, `sse2` and glibc malloc checks | 24,000 rows by 2 routes                                                                                 | zero                                                                                        |
| Computed strings and numeric sweeps on x86_64                                      | 2.72 million strings; over 3.6 million numeric evaluations                                              | #7730, #7731                                                                                |
| Linux I/O, signals, limits and environment                                         | about 40,000 (stdin, FIFOs, argv and environment, files shrinking mid-read)                             | #7732, #7733, #7734, #7735                                                                  |
| Terminals and character devices                                                    | not counted                                                                                             | #7737, #7740, #7741, #7742                                                                  |
| Dates outside UTC and in other locales, three-way                                  | 225,000 evaluations over 82 `TZ` values; 6,081 DST arrays; 627 `TZ` spellings; 25 locales by 49 formats | #7783, #7784                                                                                |
| Call-heavy recursion grids, closures, module hops                                  | 4,800 programs; 10,000 random def chains                                                                | #7787                                                                                       |
| Def chains and module DAGs, 2 to 12 wide and 2 to 64 deep                          | not counted                                                                                             | #7786                                                                                       |
| Compile errors across several modules                                              | 10,000 programs, 700 of them three-way                                                                  | zero (documented classes only)                                                              |
| Large documents                                                                    | about 6,000 runs (50 programs, 5 shapes, 2 KB to 93 MB, 7 flag sets)                                    | zero; a 55-program memory sweep found #7789                                                 |
| Regex tail, dedicated fuzz                                                         | 34,308 quantifier rows; 19,500 wide rows                                                                | #7689 closed; #7804, #7805; 629 and about 1,243 rows still differ, recorded or engine-level |

### Third round (2026-11-23 to 2026-11-25)

No hunters: verification differentials, timing passes and three design studies' corpora, on `main` as the second
round's fixes merged (`77bff4b58`, then `e43c3f9c4`, then `a9da4e814`), and at the release candidate, `6362c4bc4`.

| Area                                                                        | Approximate cases                                  | Outcome                                                                      |
|-----------------------------------------------------------------------------|----------------------------------------------------|------------------------------------------------------------------------------|
| Regression differential, x86_64 Linux (AVX2), pre-hunt build against `main` | about 2.73 million                                 | #7842 to #7845; a fifth cluster already fixed (#7792)                        |
| Regression differential, macOS arm64 (Apple M5), at `e43c3f9c4`             | 1,943,904                                          | #7887, #7888, #7889, #7893                                                   |
| Regression differential, macOS arm64 (Apple M5), at `a9da4e814`             | 1,806,339                                          | #7928 to #7931, #7934; #7935 outside its scope                               |
| Verification pass 5 at `77bff4b58`                                          | 83 issues, 30 re-checked                           | 0 rows regressed; #7846, #7847, #7848                                        |
| AVX2-only x86_64: builds, SIMD levels and test suites                       | 580,218 and 185,620 cases                          | zero build differences; #7866                                                |
| AVX2-only x86_64 at `a9da4e814`: four builds and test suites                | 602,250 cases, 1,742,616 variant runs              | zero build differences; one regex error order (#7929)                        |
| Placement, Apple M4 Pro, interleaved against `73f049b2c`                    | 1 MB pass-through and startup rows                 | no regression; the link-order lead refuted; #7826 (tooling)                  |
| Placement, AMD Ryzen 9 7950X, interleaved against `73f049b2c`               | 1 MB pass-through and startup rows, 173 more cells | #7880 (a per-byte scan from #7719); #7881 found measuring it                 |
| `dev compare jq --full --huge` on both chips at `e43c3f9c4`                 | 251 timed rows, 32 sweeps, the probes              | every timed row gated `match`; exit 1 on a stale sweep (#7890); #7891        |
| `dev compare jq --full --huge` on both chips at `6362c4bc4`                 | 253 timed rows, 32 sweeps, the probes              | every timed row gated `match`; exit 0; #7933 (placement)                     |
| Peak memory against the #7412 paragraph, AMD Ryzen 9 7950X                  | 231 rows, with an A/A' control                     | #7894                                                                        |
| Peak memory with the link-order list regenerated (#7927), both chips        | 231 rows, with an A/A' control                     | rows above jq 92 to 86 (7950X) and 79 to 76 (M4 Pro); #7826, #7894 closed    |
| Input reader (design study), against jq 1.7.1 and 1.8.2                     | 320,000                                            | #7921, and #7922 from its fix; #7904's family is wider than the issue        |
| Regex patterns (design study)                                               | 330,133 patterns, 459,678 rows                     | 49,641 rows differ from both releases; #7864, #7877, #7878, #7887 still open |
| The fixes' own differentials (old, `main` and fix against jq)               | about 1.6 million over 9 changes                   | 0 rows regressed against `main`; #7897 and #7904 found along the way         |

### Never covered

- **`succinctly yq`.** No hunt of this kind has run on yq mode; four yq issues were filed only in passing (#7615 to
  #7618), and one shape found in jq mode was confirmed there (#7788).
- **Linux aarch64** (Graviton, Neoverse) and the SVE2 path: every arm64 run was on Apple silicon.
- **Windows** and 32-bit targets.
- **jq 1.8.2 on Linux:** no 1.8.2 binary was on the Linux box, so every 1.8.2 output came from macOS arm64, and the
  4 GiB rows were never run on 1.8.2.
- **Malformed `--slurpfile`, `--argjson` and `--rawfile` data**, and `--seq` and `-R` under heavy mutation: lightly
  covered (#7769 and #7814 were found by fixers and a whole-tree comparison, not by a hunter).
- **The lazy `--seq` file route** (16 KiB and up) with RS-framed input, shape by shape; three of the second round's four
  fault-offset sweep shards; `O_NONBLOCK` standard input (a 353-case smoke test only); a fuzz of glibc's `strptime`.
- **The index budget at 2^21 keys or elements and above** (sizes 2^21 and 2^21+1 did not finish), and the memory
  findings repeated at 8 GB or on pipes with several files.
- **CPU-bound real programs against jq** on the benchmark machines: several Rosetta Code programs and the call-heavy
  corpus ran slower than jq in uncontrolled runs, so no ratio is quoted (#7699's measured gap is in
  [limitations.md](../compliance/jq/limitations.md); #7803 is another).
- **The library API** apart from the CLI, except the builder differential behind #7539.
- **Compile-error locations in the middle of a program:** 33 bison-versus-parser differences were left unchased.

## The third round

The third round began at the second round's snapshot (2026-11-23 00:45 UTC), with that round's hunters finished and
30 of its issues still open. It started no hunters, and the owner extended the time box by a day during it. Each
verification step was run as a hunt aimed at what the batch of fixes had changed: regression differentials against
the build from before the hunt, a fifth verification pass, the AVX2 machine, timing and placement passes on both
benchmark machines, a full `dev compare` on both, a memory check against the documented bound, and the corpora of
three design studies. Small items the owner picked from the backlog ran beside it (#7275, #7385, #7388, #7467).

### What it found and fixed

- **Filed:** 47 issues, #7826 to #7935, all in the filing log. At least 16 were regressions of the hunt's own fixes:
  #7842 to #7846, #7880, #7881, #7887 to #7889, #7893, #7897, #7928, #7929, #7931 and #7934. #7933 is a slowdown
  since `e43c3f9c4` at equal instruction counts (code placement), one row family on each chip.
- **Closed, as of 2026-11-24 19:45 UTC:** 30, all by merged changes: 28 by a change to code, tests or tooling (#7826
  and #7894 by #7927), and #7848 and #7866 by a recorded divergence with a pinning test. 17 are open.
- **Merged:** 72 pull requests between the snapshot and `6362c4bc4`: 26 landed the second round's work still in flight
  (fixes, docs and the new perf-guard rows of #7841), and 46 were opened in the third round, two of them dependency
  updates (#7884; #7900, `jiff` 1.0, whose migration was checked by a 682,074-case differential with no difference).
  The last two recorded the deferred divergences (#7925) and regenerated the link-order list (#7927).
- **Open, 17:**
  - **Regressions present in the candidate:** #7928, #7929, #7931 and #7934 since the hunt began, and #7933, the
    placement slowdowns against `e43c3f9c4`.
  - **Older, found in this round:** #7924 (a literal past decNumber's smallest exponent compared as written), #7926
    (Unicode blanks skipped under the regex `x` flag), #7930 (a `\cX` or a backreference before a look-behind turns
    off its refusal) and #7935 (a recursive collect-and-reyield whose memory grows exponentially).
  - **Deferred to the design proposals:** the regex tail, #7864, #7877, #7878 and #7887 (its remaining item predates
    the hunt); #7870, whose point fix (#7915) made the issue's chain linear where it doubled with each level, with the
    per-call constant and two yq-mode shapes left; and the input reader's #7904, #7921 (its filed shapes fixed by
    #7923, a residual recorded) and #7922.

#7925 recorded the deferred jq divergences in [limitations.md](../compliance/jq/limitations.md). #7904's family is
there as an error where both jq releases read the number, wider than the issue's case, left to the proposed single
model of jq's input reader. The regex design study's corpus is there as five families of generated patterns that the
text passes read otherwise than Oniguruma (refused in other words, about 11,700 rows; a different refusal around the
repeat-count limit, about 2,988; an answer where jq refuses, about 623; refused where jq answers, about 2,439; a
different answer under `x`, about 1,620), left to the regex front-end proposal. No test pins either. The same change
recorded #7924 and #7926, also with no test. #7931, #7934 and #7935 have since been fixed (#7941, #7942). A
later change recorded #7928, #7929 and #7930 there, with three found after this snapshot (#7946, #7947 and #7948), all
open and with no test.

### Three design proposals

Three areas kept producing findings that one fix per issue cannot close, as [Stopping rules](#stopping-rules)
describes. The owner commissioned a design study of each; each wrote an ADR-shaped proposal with a prototype on an
unpushed branch. They are proposals, not decisions:

- **`def` calls** (#7870, #7699, #7803, #7787): run them on a call machine with environments and a heap fork stack,
  as jq does, instead of substituting the body per call on the native stack (it would amend
  [ADR-0020](../adrs/adr-0020.md) and [ADR-0025](../adrs/adr-0025.md)).
- **Regex** (#7864, #7877, #7878, #7887): compile and match jq-mode patterns with Oniguruma itself, at the commit jq
  1.7.1 pins, and read the pattern text once. Accepted as [ADR-0031](../adrs/adr-0031.md), which supersedes
  [ADR-0019](../adrs/adr-0019.md); the migration is tracked in #7965.
- **Input reader** (#7904, #7921, #7922): read jq-mode input through one model of jq's reader, entered at sync points,
  with the semi-index running between them (the first ADR for the input layer).

### Verification and its results

- **Regression differentials:** three runs, five clusters each, 13 of the round's issues; see
  [Regression differentials](#regression-differentials).
- **AVX2 machine:** no build or SIMD-level difference in 3,966,283 variant runs over three runs; see
  [Build differentials](#build-differentials).
- **`dev compare jq --full --huge` before and after,** at `e43c3f9c4` on the AMD Ryzen 9 7950X and the Apple M4 Pro,
  against each machine's committed run at `73f049b2c`. Every one of the 251 timed rows gated `match` on both chips.
  No row flipped from succinctly-faster to jq-faster on either chip, and no row was slower than the committed run
  beyond its noise: the three M4 Pro rows that read slower by median were faster by min-of-reps under an interleaved
  A/A'/B hold, and retire 3.5 to 10.7% fewer instructions (cachegrind, on the 7950X). No row's peak memory rose by
  more than 10% against the committed run, and most rows fell by more. Both runs exited 1 for one reason, a sweep
  that had not learned an adopted jq 1.8.2 behaviour (#7890). The two runs also wrote 9 crash dumps from untimed
  probes on the Linux box (#7891), and they measure the comparison harness's build, not the shipped one
  ([docs/compare/jq.md](../compare/jq.md), #7385).
- **Placement passes** on both chips, interleaved against `73f049b2c` with A/A' controls: no 1 MB or startup row is
  slower, and the one regression in 173 more cells was real work, not placement: a per-byte scan of every string
  literal for `__jq` since #7719, which took `users_rawfile` from 8.84 to 91.41 million instructions at 10 MB on the
  7950X (#7880). Measuring its fix found `--argjson` parsed twice since #7743 (#7881).
- **Memory against the documented #7412 bound:** no longer true at `a9da4e814`, from a stale link-order list (#7894),
  and restored at the candidate. #7927 regenerated the list from `a9da4e814` (on Linux, then on macOS for the
  platform-named functions), closed #7894 and #7826, and made CI fail when more than 5% of the listed names no longer
  resolve. On the 231 rows of the #7412 run, with A/A' controls, the rows above jq 1.7.1 went from 92 to 86 on the
  AMD Ryzen 9 7950X and from 79 to 76 on the Apple M4 Pro, the counts of the #7412 run, and no row moved above jq.
  The geomean of the memory ratio went from 0.720 to 0.626 and from 0.578 to 0.553 (0.616 and 0.547 in the #7412
  run), and `startup` on the 7950X from 5.10 to 3.63 MB. Instruction counts did not change. #7927 also refreshed the
  paragraph in [docs/benchmarks/jq.md](../benchmarks/jq.md) with these figures.
- **Input-reader corpus:** 320,000 cases built from a seed, each compared with jq 1.7.1 and 1.8.2 on all three
  channels: 310,249 equal jq 1.7.1, 7,686 follow jq 1.8.2 (recorded adoptions), and 2,065 match neither. Read by
  hand, the wrong values among those reduce to two recorded mixtures of the releases, #7866, lazy validation, and
  #7921, which predates the hunt; 1,305 of the 2,065 are inputs on which the two jq releases disagree with each
  other.
- **Final re-verification at the release candidate,** `6362c4bc4`, `main` after #7927. It changes no source file
  of `a9da4e814`, only the link-order list, its tooling, CI and docs, so the differentials run there cover its code:
  - **Regression differential:** 1,806,339 cases on an Apple M5 laptop, five clusters, all filed (#7928 to #7931,
    #7934); see [Regression differentials](#regression-differentials).
  - **AVX2 differential:** four builds on an AVX2-only Intel i7-6700 that never disagreed in 602,250 cases and
    1,742,616 variant runs, every test gate green, and one regex error-order regression, recorded on #7929; see
    [Build differentials](#build-differentials).
  - **`dev compare jq --full --huge` on both chips,** at `6362c4bc4` itself. Both runs exited 0, the ledger had 0
    unlisted and 0 stale entries, and all 223 file rows, 24 stdin rows and 6 huge rows gated `match`. The all-rows
    geomean speed-up over jq 1.7.1 is 3.20x on the AMD Ryzen 9 7950X (2.74x in the committed run, 3.19x at
    `e43c3f9c4`), with jq faster on 0 of 221 rows, and 2.85x on the Apple M4 Pro (2.56x and 2.86x), with jq faster on
    26 of 221, unchanged from `e43c3f9c4`. No row flipped to jq-faster. Peak RSS geomean fell 44.2% on the 7950X and
    18.7% on the M4 Pro against the committed run, no row's peak RSS rose by more than 10% and 1 MiB, and the Linux
    box wrote no crash dumps (#7891's 1-byte core limit works). Against the committed run, the only rows more than 5%
    slower by median are two known ones on the M4 Pro: `strings_length` at 100 MB (+6.7%, -0.6% by min, a bimodal
    row) and `wide_map_values` at 1 MB (+5.3%, the accepted #7392 and #7416 trade of time for memory).
  - **Placement against `e43c3f9c4`:** two row families are slower under an interleaved A/A'/B with the same
    instruction counts: `ndjson_slurp` and `seq_slurp` by 3 to 6% on the M4 Pro, and `strings_upcase` by 9.7 to 11.6%
    on the 7950X, still 43 to 53 times faster than jq and faster than in the committed run at 1 MB (7.41 against
    8.33 ms). Both are filed as #7933 and not bisected; the suspect is #7927's list.
  - **The committed runs in `docs/compare/`** were not refreshed; that is left to release preparation.
  - **Verification pass:** not run as a separate step; the regression differential at `a9da4e814` covers the
    candidate's code (see [Verification passes](#verification-passes)).

### Lessons

Each is folded into the section it belongs to:

- Perf-guard counts move from start-up heap state and from link layout alone (#7907, #7915, #7923): see
  [What the CI perf guard does not see](#what-the-ci-perf-guard-does-not-see).
- A regression differential against the build from before the hunt catches what fixes break (#7887, #7888): see
  [Regression differentials](#regression-differentials) and
  [Regressions from the hunt's own fixes](#regressions-from-the-hunts-own-fixes).
- Sweep expectations and one-off perf-guard thresholds go stale (#7890, #7913): see [Gates](#gates) and
  [What the CI perf guard does not see](#what-the-ci-perf-guard-does-not-see).
- A stale link-order list costs memory silently (#7894): see
  [What went wrong when a step was skipped](#what-went-wrong-when-a-step-was-skipped).
- Core-dump limits for children on shared machines (#7869, #7891), and stopping only processes by recorded PID: see
  [Harness rules](#harness-rules) and [Sharing machines](#sharing-machines).
- Read jq's own source for reader edges, check which tag a file comes from, and read a 1.8 change's intent from jq's
  commits (CVE-2026-33948): see [The reference](#the-reference).

## Territories and the hunter's report

Give each hunter a **territory** that no other hunter shares: a set of dimensions (builtins, routes, platform), not a
list of issues. Disjoint territories keep two hunters from minimising the same bug in parallel and filing it twice,
and they make the coverage map possible: each zero result belongs to one territory. Split along the strategies above,
and along machines where a strategy needs one (x86_64 SIMD and glibc need an x86_64 Linux box; scale needs disk and
memory). Name in each hunter's brief what neighbouring hunters own and tell it to stay out, list what is already known
and swept at zero in its area, and give it the earlier hunters' drivers to extend rather than rewrite.

The first round used 90 to 100 minute time boxes and a cap of six findings per hunter. Four hunters stopped at the cap
(one 33 minutes into its 100), leaving time and leads unused; the later rounds dropped the cap, ran for several hours,
and kept one finding per root cause.

A hunter does not file issues, edit the repository or push. It writes one file per finding, in this shape:

```text
TITLE: jq: <what succinctly does>, where jq 1.7.1 <does what>
SEVERITY: wrong-output | crash | hang | memory | error-text | exit-status
CONFIDENCE: confirmed | probable
## Reproduce
<exact commands, runnable from any directory, building their inputs with printf or python3>
## jq 1.7.1
<stdout, stderr, exit status>
## jq 1.8.2
<stdout, stderr, exit status, and the verdict when the two versions differ>
## succinctly jq
<stdout, stderr, exit status; the commit and the builds that reproduce; the platforms>
## Scope
<variants that share the cause; what does not reproduce; the size threshold if any>
## Suspected cause
<file and function, marked as a guess>
## Not a known divergence because
<the limitations.md keywords, ledger entries and issue searches checked, with the nearest matches>
```

Its final report lists the findings (one line each), **the areas it covered with zero differences, with case counts and
the builds used**, and the leads it did not finish. Write findings as they are confirmed, not at the end, so a hunter
cut off by a time box or a dropped connection loses nothing; when an agent's tools do not let it write the file, it
puts the findings in its report and the coordinator writes them. Re-check every candidate on a build of current `main`
(the hunters' binaries were hours old while fixes merged), and on the other platform when the finding could be
platform-dependent. Collect the unfinished leads in one list and have a filer verify them in a batch near the end: the
second round's leads became seven issues (#7803 to #7809).

## Harness rules

Each rule comes from an incident in this hunt.

- **Write drivers in Python, and run every case as its own subprocess with a timeout that kills the whole process
  group and waits for it.** Shell loops report wrong exit statuses, and a timeout that kills only the direct child
  leaves its children running. On the overloaded machine of the next rule, one case
  (`[limit(50; combinations(4294967297))]`) was found running at 98% CPU.
- **Cap concurrent children and reap each one.** A route hunter's driver spawned processes faster than it waited for
  them and leaked pipe descriptors: an M4 Pro Mac mini went to a load average of 31 to 33 on 12 cores and the driver
  died with `OSError: [Errno 24] Too many open files`. Four concurrent children per hunter was the working limit;
  check that load and open descriptors stay flat before scaling a driver up.
- **Cap input sizes and memory.** jq builds a full DOM: it held 4.1 GB for a 4.3 GB file, and the reproductions of the
  unbounded-read bugs (#7463, #7478) took several GB each. Give each run a size limit (500 MB to 1 GB on shared remote
  machines), watch peak RSS, and delete multi-GB inputs as soon as their runs are done.
- **Bound what a child writes, and where it goes.** A driver that kept each child's stdout in memory was OOM-killed at
  about 15 GB: `--stream -c .` over a 100,000-deep document writes about 10 GB in both tools. Spool stdout to a file in
  the hunter's own directory (set `TMPDIR` there as well), keep only its first few MB and its size, cap the file with
  `RLIMIT_FSIZE`, and run drivers and children under `RLIMIT_AS` (4 GB and 2 GB worked). Output spooled to the
  system temporary directory without a cap fills a disk every agent on the machine shares.
- **Never wrap a test harness in `RLIMIT_AS`.** Thread-stack reservations count against the address-space limit, so
  the test harness's thread spawns fail with `EAGAIN` and it aborts. Keep the limit for fuzz drivers, their children
  and single peak-RSS probes; for a test run, watch available memory and the kernel log for OOM kills.
- **Give every child that may abort a core limit of exactly 1 byte, set with `setrlimit`.** On a Linux host whose
  `core_pattern` is a pipe (WSL2's crash capture, systemd-coredump, apport), an aborting child writes a dump of up to
  hundreds of MB whatever a limit of 0 says: the kernel skips a piped handler only for a limit of 1. A deliberate-abort
  test aborted the CLI about 21 times per run and each left a dump (#7869), and one pre-release `dev compare --full`
  wrote 9 dumps, 800 MB, from untimed probes on which jq 1.7.1 or succinctly is expected to abort (#7891). Since then
  the comparison sets that limit for itself and every child it starts (`src/bin/succinctly/compare/core_limit.rs`), as
  `tests/common/no_core_dump.rs` does for test children. Set it with `setrlimit(RLIMIT_CORE, (1, 1))` (in Python,
  `resource.setrlimit` in a `preexec_fn`, or a wrapper that sets it and execs, for `cargo test` too), never with the
  shell's `ulimit -c`: `ulimit -c 1` sets 1,024 bytes, which does not stop the dump, and `ulimit -c 0` sets the hard
  limit to 0 as well, after which no child can raise it to 1 and the repository's own core-limit tests fail. On macOS
  the limit stops the core file but not the crash reporter.
- **Never put real random input in a test.** A test reading `/dev/urandom` failed now and then in the merge queue and
  looked like a flake; it was a real unbounded read (#7812) that only some random prefixes reach (2 of 200 local runs).
  Feed a test fixed bytes (#7813 types fixed lines into a pseudo-terminal) and hunt the random shape separately.
- **Read how a shared tool parses its arguments before running it.** The machine lock read `--help` as the name of a
  hold and took the machine exclusively until the call was killed. On a machine where timing runs, an exploratory
  command is not free.
- **Never pipe a differential run through `tail` or `head`.** Write the full output to a file and search the file. A
  truncated tail once read as "only a documented extension differs" and hid a real regression (#2658).
- **Check that the side under test took the route under test.** `scripts/jq-windowed-file-fuzz.py` used `/dev/fd/N`
  and pipes as its oracle, and those became windowed routes themselves, so for some modes it compared a route with
  itself and could not fail (#7688). Assert the input size against the route's threshold, and compare with a route that
  reads the input whole or with jq.
- **Generate the matrix; do not hand-pick it.** A hand-picked matrix holds the cases its author expected. The grammar
  fuzzer's zero over 181,500 programs and the one-token mutation's five parser bugs over 146,423 mutants come from the
  same language; only the second generated near-miss syntax.
- **Cluster by cause before minimising.** Bucket by signature, check each bucket against the known set, then minimise
  one representative per bucket. One root cause is one finding: #7474 is one issue covering about 23 builtins.
- **Minimise, then re-run the minimal case alone.** Confirm the exact stdout, stderr and exit status of both tools
  outside the loop before writing it up.
- **Expect the reference to misbehave.** jq 1.7.1 reads an uninitialised byte on a bare `n` token, so a ledger row
  diverged intermittently on Linux CI (#7391): a flaky row may be the reference. jq also aborts on some inputs (module
  cycles, `%j` in `strftime`) and hangs on others; 1.8.2 has its own bugs (it aborts on an assertion over wide module
  DAGs). A crash of the reference is not a target.
- **Exclude what cannot be compared.** `now`, random values and the clock; compile-error wording; math last digits.
  Normalise only the program name and temporary paths.
- **Never conclude anything from timing on a loaded machine.** The laptop that ran the agents (an Apple M5 Max) reached
  load averages of 191 and 277 during the hunt. Use instruction counts or leave timing to the benchmark machines.

## From finding to merged fix

### Filing

A filer reproduces each finding (all three tools, on current `main`), runs the known-set check again, and files one
issue per root cause. The issue holds:

- a title in the repo's style: `jq: <what succinctly does>, where jq 1.7.1 <does what>`;
- the exact commands, self-contained (inputs built with `printf` or `python3`);
- the three outputs (succinctly, jq 1.7.1, jq 1.8.2), the versions and the commit, and which platforms reproduce;
- the scope (variants, what does not reproduce, the size threshold);
- the jq 1.8.2 verdict and its upstream evidence when the two versions differ, or a "Decision needed" paragraph;
- why it is not already known (the searches and their nearest matches).

A variant of an existing issue goes on that issue as a comment, not as a new issue.

### One worktree and branch per issue

Each fix gets its own branch, `issue-N-<slug>`, in its own worktree outside the repository, branched from a freshly
fetched `origin/main`. Before creating it, check that nobody else has one (`git branch -a --list "*issue-N-*"`,
`gh pr list --search N`). Branch from `origin/main`, not from another unmerged fix, unless the fix depends on it; then
say "must merge after" in the hand-off. Reproduce the issue on a build of the worktree before changing code, and stop
if it does not reproduce. The test goes in `tests/jq_cli/iN.rs`, pins the reference's captured output over a generated
matrix, and must fail before the fix (prove it once). The changelog entry is a fragment in `changelog.d/`.

### Deciding what the fix is

When the reference's behaviour looks wrong, decide in [ADR-0018](../adrs/adr-0018.md)'s order: first whether a condition
lets succinctly refuse the reference's behaviour, then which option matches the pinned reference more closely, and only
then performance. In practice the outcomes were three:

- **Fix to match 1.7.1.** Lines appended to a file during a run are read, as jq reads them (#7446).
- **Adopt the newer reference.** Negated number literals compute as in jq 1.8.2, which fixed them upstream, `-0`
  spelling included; 27 pins moved to 1.8.2 and the 1.7.1 rows were listed as `newer_than_primary` (#7453).
- **Record a divergence with a pinning test.** jq 1.7.1 and 1.8.2 both say `.[:3] == .[3:]` is true for
  `[1,2,3,4,5,6]`, because their equality compares buffer pointers and ignores the slice offset; succinctly keeps
  comparing contents, and limitations.md records it with a test that pins succinctly's answer (#7459). A record follows
  [STYLE-0014](../STYLE_GUIDE.md#style-0014-one-sentence-per-line-in-the-compliance-limitation-pages): one sentence per
  source line.

A decision the rules do not settle goes to the owner with a recommendation; the fixer skips that part rather than
guess. When the coordinator decides one under time pressure (the second round adopted jq 1.8.2 for every `-s` route's
end-of-text faults, #7807, and reversed an earlier recorded divergence for `range` past 2^53, #7731), it records the
decision with the owner as one to confirm or overrule.

### Gates

Run every gate after the last edit, with cargo output redirected to a file and its exit status read directly (a pipe
hides it):

- `cargo check --all-targets --features cli,simd,regex,serde` and `cargo check --no-default-features`;
- the clippy invocations of `scripts/lib/clippy-variants.sh` for each architecture the change compiles on, and rustdoc
  with `-D warnings`: a branch whose clippy ran only on macOS failed five lints when its final gates ran on Linux;
- `scripts/fmt-per-issue.sh` and `cargo fmt --check`;
- the lib tests and the full `--test jq_cli_tests`; `yq_cli_tests` as well when shared evaluator or builtin code
  changed, because a jq fix can change yq mode;
- `python3 scripts/ledger_refs.py check`, `python3 scripts/changelog.py check`, `python3 scripts/doc_lines.py check`;
- `scripts/pre-push.sh` (`--full` for the lints CI runs).

A change that adopts jq 1.8.2 also needs CI's full ledger comparison, on Linux. The local `--ledger-only` gate finds
only *stale* entries; a new divergence that is not in the ledger fails CI's `Ledger drift` job instead (#7583 and #7584
failed that way, #7613). Before handing the branch over, run CI's Compare step on a Linux box, against source builds of
both jq versions (`scripts/compare-jq-build.sh`) and a `cli,simd,regex,serde,bench-runner` release build:

```bash
./target/release/succinctly dev compare jq --quick --dimension smoke --dimension features --dimension cli \
  --dimension differential --dimension fuzz --dimension input --dimension robustness --out data/compare/ledger
```

The fuzz dimension uses the manifest's seeds (1 to 3 in CI). An entry that only other seeds find would read as stale in
CI, so ledger only what CI's seeds find. The `--full` comparison runs more oracle sweeps than CI does, and an adoption
that leaves one of them without an entry makes the release's `--full` run exit 1 (#7815).

It happened again in the third round. #7807 adopted jq 1.8.2 for a `-s` fault at the end of the text, and the
`stream-errors` sweep, which judges against jq 1.7.1, met that shape on 3 of its 2,600 runs; nothing showed it until
the pre-release `--full` run exited 1 on both chips (#7890). The bisect found the deliberate adoption, not a bug; the
fix taught the sweep the adopted shapes rather than ledgering the whole sweep, and its run over seeds 1 to 20 found
one real bug on seed 17 (#7897). When a change adopts jq 1.8.2 behaviour, search every sweep under `scripts/` that
judges against jq 1.7.1 for the shape, not only the ledger, and run the sweeps' extra seeds once.

### The performance hold

A fix that touches code ordinary queries run (a hot path) is held until it has been timed. The fixer marks the hand-off
`HOLD-PERF` and names the queries and inputs to time and why; a timing agent clears it, or sends it back with the cost
and its cause. The method is the [A/B Benchmarking Method](benchmarking.md#ab-benchmarking-method). A timing pass
measures:

- **Three builds from one source:** A (the base), A' (the same source built a second time from a clean target, the
  A/A control) and B (A with the fix), each built with the tree as the working directory so `.cargo/config.toml` and the
  link order apply. A and A' were byte-identical every time, so A'/A is run-to-run noise (about ±2% at the median, more
  on startup-bound 1 MB rows).
- **Identity first:** every cell runs once per binary, and stdout hashes, stderr and exit statuses must match each
  other and jq 1.7.1 before any timing counts.
- **Instruction counts on x86_64:** cachegrind (`--cache-sim=no`) on the 7950X over every row, and a per-function diff
  (`cg_annotate`, `cg_diff`) for a row that moves. Counts are deterministic, so a cost of a fraction of a percent shows,
  and the diff names the function that pays it.
- **Wall clock on both chips:** A, A', B and jq 1.7.1 interleaved in a rotating order within each repetition (15 to 21
  repetitions at 1 and 10 MB, 7 to 9 at 100 MB), each block under one exclusive machine hold with the machine's idleness
  recorded before it, on an AMD Ryzen 9 7950X (x86_64 Linux) and an Apple M4 Pro Mac mini only. Never the machine that
  runs the agents, and never on battery.
- **Peak RSS, against the same control:** Linux RSS is unimodal; macOS RSS is bimodal (A and A' land in two modes), so
  an M4 Pro row has moved only when B lies outside every A and A' sample.
- **The CI perf guard,** `scripts/perf-guard.py --binary B --baseline-binary A --arch x86_64 --check` on the 7950X. It
  must pass, but passing proves less than it seems: see
  [What the CI perf guard does not see](#what-the-ci-perf-guard-does-not-see).

The acceptance policy that settled during the second round:

- **Instructions:** every row the hold names within a small bound of the base (+0.4%; a fixer may name a tighter one).
  A row above it is work the fix added, and goes back to the fixer with the per-function diff.
- **Wall clock at flat instructions:** a shift outside the A/A' spread on one chip, with instructions flat and the
  other chip unmoved or moving the other way, is code placement, not work. It is accepted per branch up to about 3% on
  one chip. Placement does not add up across twenty merging branches, so it is tuned once, on the integrated tree,
  through the link order (`scripts/link-order/symbols.txt`), not chased per branch. One branch read +2.1% on the 7950X
  and nothing on the M4 Pro (#7451); another read 3.5 to 7.2% faster on the 7950X at flat instructions (#7718). Neither
  is a property of the change, and neither may be quoted as one.
- **Allocator effects** are attributed like any other cost. Two rows of #7757 rose by 0.4% and 1.4% in instructions,
  all inside glibc's `_int_malloc` and `_int_free`, because results were now freed earlier, while the fix's own code got
  cheaper; it was accepted with that attribution.
- **A trade-off** (a correct fix that is necessarily slower on some rows, like #7554's O(n) `first`) goes to the owner,
  not through the hold.

Eighteen branches were timed in the second round. Nine cleared at their first timing, two of them only under the
placement clause: #7554 with #7555, #7451, #7699, #7702, #7713, #7718, #7721 with #7785, #7752 and #7757. Nine came
back with a real cost:

| Issue | Cost the timing found                                                                                                                      | Rework                                                                                                |
|-------|--------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------|
| #7483 | a marker walk at every def-call entry: +5.4% instructions on tree recursion; +2 to 3.5% on the 7950X at flat instructions                  | walk skipped where no reader can see a marker, door made cold; cleared                                |
| #7719 | a routing check per document: +0.6 to 3.2% instructions on every NDJSON record; one newly routed program went quadratic                    | route decided once per program; accepted                                                              |
| #7750 | the open tail re-checked at every file boundary: quadratic in files (a 100 MB array in 40 files, 2.8 s to 8.3 s on the 7950X)              | parser state carried across files, which also fixed the base's own quadratic stitch (#7792); accepted |
| #7758 | an escape check per backslash: +143.5 instructions each, +14% on escape-heavy input, now slower than jq there                              | the check moved off paths a reader already rules out; escape-heavy rows 19% below the base; cleared   |
| #7787 | the settle question asked at every evaluation of every operator: up to +4.2% instructions, +24.7% on one M4 Pro row                        | an inline shape test, the answer remembered per definition; accepted                                  |
| #7786 | a seen-set created and dropped at every walk: +0.8 to 2.0% instructions per NDJSON record                                                  | in rework at the time of writing                                                                      |
| #7789 | path chains grafted where nothing can share them: +6.3 to 6.6% instructions and +26 to 31% peak RSS on two ordinary delete and update rows | in rework at the time of writing                                                                      |
| #7775 | an early-exit reader that held the whole pipe in memory and opened every named file up front                                               | memory bounded, files opened when reached; accepted                                                   |
| #7697 | the document kept to the end of the run: 6 to 15% more peak RSS on the `..` path controls on both chips                                    | held at the time of writing                                                                           |

The lesson: a correct fix on a hot path costs something until a timing pass shows it does not. Each of these branches
had passed its test suites and the CI perf guard, and five of the costs (#7719, #7750, #7758, #7786, #7787) sat on
shapes the guard has no row for.

### What the CI perf guard does not see

The guard counts instructions over a fixed list of rows on every PR and fails on drift past each row's threshold. The
second round's timing passes found three ways it misleads, and the third round two more:

- **Shapes with no row.** Many small records with one evaluation each (#7719, #7786); one document across several files
  (#7750); escape-heavy text (#7758); per-element `and`/`or` and object construction with a def (#7787); fold steps
  (#7451, #7702). #7798 adds 21 rows for these shapes, with fixtures the script generates. Replayed against the first
  versions of the reworked branches, it fails #7719 on 3 rows, #7787 on 5 and #7750 on 4; it gives identical counts on
  identical builds, and the guard takes 14% longer.
- **Loose per-row overrides.** A `QUERY_THRESHOLDS` entry of 30%, kept from an older change, let a +5.9% row of #7789
  (`del(.users[] | select(.score < 100))`) pass. Remove an override once `main` carries the change it was for (#7695),
  and read the drift column, not only the verdict. A comment saying so was not enough: in the third round nine more
  overrides, of 8% to 90%, had outlived their changes by up to four weeks (#7913). Every override above the default
  is now named with the pull request it covers (`ONE_OFF_UNTIL` in `scripts/perf-guard.py`), and the script's
  self-tests fail once `main` is 50 merges past that pull request.
- **It fails on improvements too.** A row more than 5% faster fails like one that is slower (#7672 failed on -7.2%), so
  a fix that speeds up a guarded row needs an override until `main` has it, and new rows merge after the fixes that
  move them: #7798 was held to merge last, or every fix still in flight would have tripped it.
- **It moves when the counted code does not: start-up heap state.** #7907, the fix for #7881, moved per-record rows
  by up to 1.5% on CI, and no function of the crate changed its count on them. The branch kept argv until later in
  start-up instead of freeing it where it always had been, which changed the heap every run starts from and so which
  of glibc's allocator paths each later allocation takes: the whole difference was in `_int_malloc`, `unlink_chunk`,
  `_int_free`, `memcpy` and `memcmp`. Keeping argv only when a value was kept put every row back within a start-up
  constant.
- **It moves with link layout.** A yq row read +0.073% on #7923's branch, all of it in `__memcmp_avx2_movbe`: the
  same calls, each costing more, from operand alignment in a binary 3,912 bytes larger, for a change that touches no
  yq code. A term of the same kind on #7915 showed on one machine's guard run and not on CI's. Another row of #7923 read
  +0.108% from allocator state, and -0.26% when the same two binaries were started from a path one character
  shorter: its sign follows the length of the binary's path.

So attribute a moved row by function before believing it: `cg_annotate` of base and branch, with a second build of
the base as the A/A control, as in [The performance hold](#the-performance-hold). A difference that sits entirely in
the allocator or in `memcmp`, with every function of the crate flat, is state or layout, not work the change added;
say so in the PR, and let CI's run of the same rows confirm it.

### Landing

- **Docs-page conflicts are rarer than they look.** Most fixes add a section to limitations.md. The first round
  serialised every PR that touched it, after its first integration pass found 7 of 44 candidates conflicting only
  there. In the second round an integration pass merged 21 branches with 2 conflicts, both in limitations.md and both
  mechanical, so the rule was relaxed: branches that edit the same page may be open together when each pair merges
  cleanly (`git merge-tree`), and the merge agent merges `main` into the next one after each lands (rerere replays a
  keep-both resolution). Place a new section beside the most closely related one rather than at the top of a list every
  branch edits.
- **Integrate before releasing a batch.** Merge every unmerged fix into one throwaway branch and run the full suite
  there. The first round's three passes found a test interaction between #7492 and #7501, and a branch that failed
  against newer `main` (#7489 against the stack-floor change of #7635); the second round's pass merged 21 branches with
  no code conflict and no failing test on Linux.
- **Compare the whole tree mid-way.** A `succinctly dev compare jq --full` run on the integrated `main` against the
  last committed run finds what no single branch's timing can; see
  [Regressions from the hunt's own fixes](#regressions-from-the-hunts-own-fixes).
- **Respect CI capacity.** macOS runners were the scarce resource: a merge queue of 20 entries thrashed on 60-minute
  timeouts and nothing merged for four hours. What worked: at most 5 PRs in the merge queue, each after a clean
  `git merge-tree` against the queue tip; no new PR while jobs it needs are queued; and cancelling runs of merge groups
  that no longer exist (#7552). Only one agent (the merge agent, below) opens, enqueues and repairs PRs. A test that
  fails intermittently ejects unrelated PRs from the queue: fix it first, as a queue-blocking change of its own
  (#7813), rather than re-enqueueing the ejected ones.

### Regressions from the hunt's own fixes

Fixes written quickly in the same code break each other, and their own tests do not show it. In the second round they
were found four ways, and in the third round mostly a fifth:

- **By a later hunter or fixer in the same area.** #7720 (a `?//` inside `map` in an object value refused at compile
  time) came from #7602's parser change and was found by the next round's builtin-family fuzzer. #7795 (a def whose body
  is `first(... ?// ...)` no longer retrying the `?//`) came from #7744 and was found by the fixer reworking the same
  rule for #7787. Both were release-blocking and got a small fix branch of their own, released ahead of the larger
  work. Point later hunters at the areas with the most merged fixes, and have a fixer that changes a recent fix's code
  re-run that fix's matrix.
- **By a test that looked flaky.** A memory guard in a terminal test (512 MiB for `-c .` over `/dev/urandom`) tripped
  in some merge-group runs. The cause was real: a stream that opens a container and meets a fault inside it was read on
  forever looking for the close (#7812), and only some random prefixes reach that state. Until the test was made
  deterministic (#7813) it ejected unrelated PRs from the merge queue, one of them twice. A memory guard that trips is a
  finding until shown otherwise.
- **By a whole-tree comparison mid-way.** `dev compare jq --full` on the M4 Pro, on the integrated `main` against the
  committed run, showed the batch as a whole faster (the geomean speed-up over jq across 221 rows went from 2.56x to
  2.81x, and no row flipped to jq-faster) and found three things no branch's timing could. `--rawfile` had held two
  copies of the file since #7436 (+34% time and +86% peak RSS at 100 MB on the Apple M4 Pro, #7814). 1 MB pass-through
  rows were 5 to 9% slower on the M4 Pro while x86_64 instruction counts on the same rows had fallen, which pointed at
  the link order: its symbol list had been generated on x86_64 and lacks the NEON kernels. A placement pass in the
  third round refuted that lead: relinked without the order file the build was not faster, and under interleaved
  A/A' control `main` was faster than the committed build on those rows, so the midway reading was load on the
  machine, not code. Confirm a cross-run difference under interleaving before bisecting it. And the comparison itself
  exited 1, because six oracle sweeps still judged against jq 1.7.1 after deliberate adoptions of 1.8.2 behaviour
  (#7518, #7525) and had no ledger entries (#7815).
- **By a verification pass,** as in the first round: a row FIXED in one pass that differs in the next.
- **By a regression differential against the build from before the hunt** (the third round): three runs gave 13 of
  its issues, 12 of them regressions of merged fixes, among them two regex misreadings from one fix (#7887 from
  #7865) and an error order from another (#7888 from #7852), with each fix's own sweep clean; see
  [Regression differentials](#regression-differentials). The fixes of those regressions were checked the same way,
  three builds against jq: the regex repair's 420,000-case differential moved 47,040 rows to jq's answers and
  regressed none against `main`. The third regression differential still found #7929 in the fixes for #7887 and
  #7888.

### Verification after merge

Close the loop with a [verification pass](#verification-passes) on `main`, including the rows that need Linux or large
inputs. Merged is not the same as fixed: the first round's second pass found three fixes that missed shapes outside
their tests.

### What went wrong when a step was skipped

- **Linux-sensitive tests verified on macOS only.** #7687 and #7686 passed every local gate on macOS and failed or hung
  on Linux CI: Linux opens `/dev/stdin` again where macOS shares the descriptor, and a test blocked reading a pipe that
  only Linux hands it. One of the two hid a real Linux product bug. Run descriptor, terminal and I/O tests on Linux
  before the PR.
- **A fix gated on an old base.** #7679 passed its gates and did not compile once merged with `main`, where newer PRs
  had added code using the old `u32` line counter. Gate a long-lived branch again after merging current `main`.
- **A local ledger gate taken for CI's.** See [Gates](#gates).
- **A coverage-only failure.** A test passed under `cargo test` and failed only in the coverage shards, because the
  profiler's runtime sets an environment variable in the child (#7586). Read which job failed before assuming a flake.
- **A fixer's own timing taken for a timing pass.** #7775's fixer measured instruction counts of the rows it changed;
  the coordinator's review found that the new early-exit reader held a whole pipe in memory and opened every named file
  up front. Review what a fix does to memory and to files it does not read, not only to the rows it targets.
- **A generated list left to go stale.** `scripts/link-order/symbols.txt` orders the binary's hot functions at link
  time, and was edited twice in the 222 merges between the #7412 memory run and the third round, while the code moved
  under it: 1,339 of its 1,455 names still resolved. Nothing failed, not the tests, not the perf guard (it counts
  instructions, not pages), not `generate.py check`. Peak RSS on rows under 60 MB rose by a median 1.29 MB on the AMD
  Ryzen 9 7950X, in file-backed pages of the binary, gradually over the merges and not in any one PR, and the
  documented #7412 bound stopped being true (#7894). Rebuilding with only the list regenerated recovered nearly all
  of it. Check how many listed names still resolve (the link log prints it) before a release, and regenerate the
  list after a large batch of merges. Since #7927, CI fails when more than 5% of them no longer resolve.

## Coordinating several agents

### Roles

- **Coordinator:** writes the briefs, assigns territories and issues, files or delegates filing, relays owner
  decisions, keeps the running log, and decides when to stop. It does not hunt or fix.
- **Hunters:** one territory each; find, minimise and write up. Read-only on the repository.
- **Filer:** verifies hunters' findings on current `main`, de-duplicates them, files issues and records each in the
  filing log. A **triager** can run the latest-reference check for a batch of issues at once.
- **Fixers:** one issue (or a queue of small ones) each, from worktree to a gated local commit; they hand off rather
  than push.
- **Timing agents:** run the performance holds on the benchmark machines and clear or reject them.
- **Integrators, verifiers and a whole-tree comparison:** the integration passes, the verification passes, and a
  mid-way `dev compare jq --full`.
- **Auditor:** sweeps shipped text (PR and issue bodies, comments, commit messages, changelog fragments, docs) for
  figures from the wrong machines and for private details; see [Shipped text](#shipped-text-privacy-and-figures).
- **One merge agent:** the only agent that pushes, opens PRs, enqueues, re-enqueues after an ejection, and parks a
  failing PR as "needs fixer". Exactly one runs at a time; a successor starts from the previous one's hand-off file.
  It checks every PR title and body against the shipped-text rules before opening it.
- **Machine coordinator:** grants per-machine budgets, approves exclusive holds, and is the route for every request on
  a machine other people use.

### Shared state files

Agents share state through files in a git-ignored scratch directory, one writer per file where possible, and each
line self-contained:

- **The briefs:** the hunter brief (what counts, what is known, the report format), the fixer brief (worktree, gates,
  hand-off), the latest-reference rule, and per-machine rules. Every agent prompt names the briefs to read; a rule
  change is appended to the brief and sent to running agents.
- **Findings directory:** one file per finding, named `<territory>-<nn>-<slug>.md`.
- **Filing log:** finding file, issue number, severity, size estimate, title. **Rejection log:** finding file, reason.
- **Latest-reference triage table:** issue, verdict (same, differs and 1.7.1 is a bug, differs by design, unclear), note.
- **Hand-off list ("ready"):** branch, worktree, issues, full head sha, priority, note. The note carries gates run, what
  is unverified, merge-order dependencies, `Closes #N` or `Refs #N`, and a hold marker (`HOLD-PERF`, `HOLD-GATES`,
  `HOLD-LEDGER`) only when a hold is meant: a marker word inside an ordinary note held one branch back by mistake, so
  a branch with no hold says "no perf hold". A hold lifted on evidence is lifted on every line that carries it: in the
  third round a superseded line still marked `HOLD-PERF` kept the merge agent from opening #7870's released branch.
- **Merge-agent lists:** PRs waiting on CI (PR, issue, head sha, note); released branches (branch, PR, sha, time);
  parked PRs (PR, sha, reason); and a status table (PR, issue, state, last action, time).
- **Verification tables:** issue, state, class, target reference, note; one per pass.
- **Timing reports:** one per branch timed, with the verdict first, then the rows outside the line, the cause, the
  method and the raw data's location.
- **Follow-ups and leads lists:** loose ends fixers and hunters reported, to verify and file in batches.
- **The coordinator's log:** decisions, owner rulings, agent assignments and failures, with times, and a list of the
  decisions pending with the owner. Re-read after any context loss.

### Shipped text: privacy and figures

The repository is public, and every PR title and body, issue, comment, commit message, changelog fragment, code
comment and doc is shipped text. Two rules apply to all of it, and each is enforced twice: in every brief, and at the
point of publication (the merge agent's check before it opens a PR, and an auditor's sweep of what was published). A
brief alone did not hold: a PR body published after the rule was written still named a host for about an hour and a
half.

- **No host names, account names, home-directory paths, or session and agent identifiers.** Name machines by role and
  chip ("the AMD Ryzen 9 7950X box", "the Apple M4 Pro Mac mini", "an AVX2-only x86_64 VM", "a development laptop"),
  write paths relative to the repository or as `~/...`, and do not cite scratch files (the hand-off list, a report's
  file name). An audit of the first two days found host names or home paths in 22 PR bodies, 3 issue bodies and 18
  issue comments, among them claim comments that carried a host in an agent's name. Editing fixes the current text
  only: GitHub keeps each edit's earlier revision, and purging those is the owner's decision.
- **Timing and memory figures only from the benchmark machines, with the chip named.** A figure from a development
  laptop, a build machine or a shared lock is a signal for the agent and the coordinator, never shipped text. Shipped
  text may carry a complexity statement ("linear where it was quadratic", with the shape and what jq does), a
  cachegrind instruction count on the x86_64 box, said to be one, or a wall-clock or memory figure from an exclusive
  timing hold on the 7950X or the M4 Pro, with the chip named. The audit found 40 figures from a development laptop and
  5 jq times from the same uncontrolled runs, in 14 PR bodies and 11 files, and replaced each with a figure from a
  timing report or with a complexity statement.

### Permission boundaries

An agent's tool permissions and safety checks are the owner's boundary, not an obstacle to route around. When one
agent's check refuses an action (a cleanup `rm` on a remote machine, a `gh issue create`), no other agent performs it
on that agent's behalf: not the coordinator and not the machine coordinator. Doing so launders the refusal. The
refused action goes to the owner, saying what it would do and why; a refused issue goes into a draft in the agent's
worktree for the coordinator to file. Leftovers on a machine wait for the owner, on the list of pending decisions.
In the third round four refused issue drafts waited in worktrees until the owner allowed the coordinator to file
them, and a fixer's edit to a sweep's expectations, refused as weakening a check, went to the owner as three options
before the sweep was changed (#7890).

### Hand-over when an agent's context runs out

Long-running agents run out of context in the middle of a task: three merge agents and a fixer did in the second round.
Write the state down before that happens, as a file a fresh agent can act on without the conversation: the branch and
its base; the timing report and the cause it names; the exact change to make, by file and function; the target the
rework must meet (for example, every per-record row within +0.1% instructions of the base); the tests that must still
pass; the gate recipe used before; and what not to change. The fixer of #7786 and #7789 left such a note when its
context ran out before either rework began, and two fresh fixers took the reworks on from it. The #7750 branch was
finished (lint fixes, Linux gates, a 68,320-list re-sweep) by an agent that had not written it. Merge agents keep a
status table and a notes file, and each successor starts from them.

A context limit ends one agent; a usage limit ends every agent on the same model at once. In the third round a usage
limit ended a fixer, the merge agent and the agent preparing the link-order regeneration within minutes of each
other, with one fix's last sweep and pull request still to do. Agents cannot change model mid-run, so continuations
on another model started from the three agents' state files and took the work on. Keep every long-running agent's
state file current, not only when its context runs low.

### Sharing machines

- **A lock on each shared machine** with two kinds of hold: *shared* for builds, tests and fuzzing, *exclusive* for
  timing. A timing run queues for an exclusive hold, and new shared holds wait behind it. Keep holds to 30 minutes or
  less (10 minutes on a timing machine, so an exclusive hold can get in) and give each driver a time budget so it exits
  by itself. Do setup (copying, extracting, deleting trees) and post-processing (`cg_annotate`, log parsing) inside a
  shared hold too: anything outside the lock disturbs a timing run.
- **Budgets are per machine, for all agents together:** jobs, RAM and disk. The grants in this hunt were 24 jobs, 24 GB
  and 30 GB on the 7950X box; 8 jobs and 16 GB on the M4 Pro (2 cores left free for its owner); 8 jobs and 10 GB on an
  Apple M5 laptop (builds and tests only); 6 jobs and 5 GB on an AVX2-only x86_64 VM. Test threads count as jobs: set
  `RUST_TEST_THREADS` and `cargo test -j`, and run the deep-recursion tests (2 GB stacks each) at 2 threads. On the
  coordinating laptop: `CARGO_BUILD_JOBS=4`, one cargo invocation per agent, never a build and a test run together.
  Before building, read the machine's status file and split the limit with any other agent building there.
- **Disk:** a cap per agent as well as per machine (14 GB per agent on the 7950X box, with no new files once it has
  less than 40 GB free; 14 GB on the M4 Pro; 20 GB on a build laptop; 30 GB on the VM): room for one live branch's
  gates, not for history. `CARGO_INCREMENTAL=0`, one source tree and one target directory per agent, the gates run in
  phases (debug tests, then lints and rustdoc, then a release build) with a clean between them, and `cargo clean` once
  a branch's gates are done.
- **Keep the main checkout clean.** Every stray file in its root is visible to the owner. `git -C <main> archive ... >
  file` run from the main checkout's directory drops the archive there (it happened twice): give `git archive` an
  explicit `-o` path in the agent's own scratch directory, and check `git status --porcelain` of the main checkout
  before finishing.
- **Use every granted machine.** A machine granted and idle is capacity the hunt is not using: keep a queue of work each
  machine can take (builds and gates on a build laptop, sweeps and Linux gates on a Linux box, timing only on the
  benchmark machines), and assign from it when an agent leaves one.
- **Resumable jobs.** The network dropped several times and every remote box went offline once for hours. Run each long
  job in a detached terminal, as a script that skips finished steps and logs to files on the machine; never as one long
  foreground call. When a machine is unreachable, probe it at most every 10 minutes and work locally meanwhile. In
  the third round every remote machine went offline at once for about an hour, two of them rebooting; one step that was
  running there was lost, a sweep moved to the coordinating laptop at two jobs, and the work that needed the
  machines waited for them.
- **Liveness:** every live job touches a heartbeat file each minute and appends `started`/`finished` lines to a status
  file, so the machine owner can tell finished from crashed. A runner refuses to start a second copy of itself.
- **Cleanup:** delete sources, targets and inputs, keep small logs, append the agent's tag to a done file, and tell the
  machine owner. Never kill a process you did not start, and never `pkill` by pattern.
- **Stop your own jobs by recorded PID, not by pattern.** Record the PID or process group of every job you start (a
  `.pid` file) and stop it with `kill` on that PID or `killpg` on that group. `pkill -f`, `killall` and `pgrep | xargs
  kill` match on a command line, and on a shared machine another session's process can carry the same words: in the
  third round an agent stopped its own steps with `pkill -f`, which happened to harm nothing, and the rule became one
  of the machine rules.

### Stopping rules

- **Freeze hunting when findings outpace fixes.** Ten hours in, 110 issues had been filed and 23 PRs merged; the
  coordinator froze hunting and spent the next day landing fixes. Hunting reopened once only owner decisions and
  performance work were left open.
- **Close hunting on time, not on exhaustion.** No round ran dry: the second round's five hunters found 40 more after
  the first round's 130, and each still had leads. Hunting closed after them with about a day and a half of the time
  box left, because that time was needed for their fixes, the timing passes (half of which sent a branch back), the
  placement pass, a whole-tree comparison and a final verification pass.
- **Stop assigning new fixes before the release** with enough time left for one integration pass and one verification
  pass of the release commit. The plan used here: no new fixers in the last 1.5 hours of a 12-hour time box, and a
  final verification pass in the last half hour.
- **Record a long tail instead of chasing it.** The regex translation is the example: one fix (#7682) cut the differing
  rows of a 37,067-row regex fuzz from 2,567 to 1,401, but also made 11 formerly matching rows differ (4 of them toward
  jq 1.8.2; the others were recorded). The remaining shapes were filed together as #7689, worked once the scope was
  widened, and closed with the rest recorded and the fuzz kept as the regression net.
- **Defer design work, and say so.** Three areas kept producing findings that one fix per issue cannot close, and were
  left as design work: the def-call evaluation model (a recursive def's body is copied and analysed again at each call,
  which is the constant of #7699 and #7803 and the per-call settle question of #7713 and #7787); validation of input a
  filter never reads (the semi-index validates lazily, so faults beside what a filter reads keep surfacing, and #7556
  and a file-cut case of the route hunter were recorded rather than fixed); and the two evaluators
  (`src/jq/eval.rs` and `src/jq/eval_generic.rs`), each of which needs every fix (#7788 is #7786's shape in yq mode).
  Each needs an owner decision and more time than the tail of a hunt. In the third round the owner commissioned a
  design study for three such areas (the def-call model, the regex front end and the input reader), each delivered as
  a proposal with a prototype and nothing merged; see [Three design proposals](#three-design-proposals).
- **Leave design questions to the owner.** A performance trade-off (#7554's O(n) `first`), an ownership-model change
  (#7483) or a cap that jq added for its own safety is a decision, not a fix.

## Running the next hunt: checklist

1. Read this guide's coverage map. Choose territories from the [never covered](#never-covered) list and the rows with
   findings, not the zero rows, and point at least one hunter at the areas with the most fixes merged since the last
   hunt.
2. Fetch `main` and build `cli`, `cli,simd` and a debug build from one commit, on every platform you will hunt on.
   Confirm the reference versions with `--version` on each machine.
3. Write the briefs: finding classes, the reference rule, the known-set sources, the report format, the harness rules,
   the shipped-text rules, and the machine budgets. List what is already filed and swept at zero.
4. Give each hunter one territory, a time box of several hours, the earlier drivers to extend, and no findings cap.
5. Run each candidate through the known-set check, then file one issue per root cause with three outputs. Collect
   unfinished leads and file them in one verified batch.
6. One worktree and branch per issue; reproduce first; test fails before the fix; all gates after the last edit, on
   each architecture the change compiles differently on; Linux for descriptor, terminal and I/O tests; CI's full ledger
   comparison, and ledger entries for every sweep, for adopted behaviour.
7. `HOLD-PERF` for every hot-path fix, and a timing pass on both benchmark machines before it lands: instructions within
   the bound, placement accepted per branch and tuned once on the integrated tree.
8. One merge agent lands the hand-offs, a few at a time, within CI's capacity, and checks each PR's text before opening
   it; an auditor sweeps the published text.
9. Integrate the unmerged fixes and run the full suite; run a whole-tree `dev compare jq --full` mid-way; merge new
   perf-guard rows after the fixes that move them.
10. Verify every filed reproduction on `main` (on Linux at size, and each platform-sensitive fix on the other
    platform) after the fixes land, and compare with the last pass.
11. Run a regression differential of the build from before the hunt against `main`, weighted toward what the fixes
    changed, after each large batch of merges and again at the release candidate, on more than one platform.
12. Before the release: check how many link-order names still resolve and regenerate the list if they have drifted;
    drop perf-guard overrides `main` no longer needs; make sure every sweep that judges against jq 1.7.1 knows each
    adopted jq 1.8.2 behaviour; run `dev compare jq --full --huge` on both benchmark machines and read its exit status.
13. Update this page's coverage map with the new areas, volumes and outcomes, and the index.

### For a yq hunt

The method carries over; the inputs change:

- The reference is yq v4.53.3 (`yq` on `PATH`; capture its output, never recall it). Real yq features look like
  extensions (`--front-matter`, `--split-exp`); check before calling anything an extension.
- The known set is [docs/compliance/yq/limitations.md](../compliance/yq/limitations.md),
  `tests/data/yq-golden-known-failures.txt`, `tests/data/yq-path-consistency-known-failures.txt`, and issues titled
  `yq:`. There is no yq ledger.
- jq-only builtins are gated behind `--jq-extensions` in yq mode: a run without the flag must reject them as yq does.
- The YAML test suite (`tests/data/yaml-test-suite-*`) is the oracle-independent corpus, and `scripts/yq-*` hold the
  existing sweeps: read their headers before writing new ones.
- Shared evaluator code means a yq finding's fix can change jq mode and the reverse: gate both suites.

## Related

- [ADR-0018](../adrs/adr-0018.md): reference fidelity and the decision order.
- [docs/compliance/jq/limitations.md](../compliance/jq/limitations.md): the known set for jq.
- [comparison/jq/README.md](../../comparison/jq/README.md): `succinctly dev compare jq`, the ledger and the probes.
- [Reproducing the jq comparison](reproducing-the-jq-comparison.md): valid comparison runs on any machine.
- [Developer Guide](developer.md#differential-sweeps-against-jq): the repo's committed differential sweeps.
- [Benchmarking Guide](benchmarking.md#ab-benchmarking-method): the A/B method behind the performance hold.
- [Release Guide](release.md): the release checklist a hunt feeds.
- [changelog.d/README.md](../../changelog.d/README.md): changelog fragments.
