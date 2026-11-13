#!/usr/bin/env python3
"""Differential sweep of `-p xml` against the pinned `yq` (#6925).

Random XML documents (elements, attributes, namespaces, text, entities, CDATA, comments in every
position, processing instructions, directives, whitespace) and mutations of them (a deleted, a
duplicated or a replaced byte, a truncation) are read by `yq -p xml` and by `succinctly yq -p xml`
under several flag sets, and the outputs (and exit statuses) compared:

    scripts/yq-xml-input-sweep.py TARGET_BIN [--n N] [--seed S] [--show K] [--jobs J]

* `json`: `-o=json -I0 .`, the data model with no comment or style in the way;
* `yaml`: `-o=yaml .`, which adds the head, line and foot comments of every node;
* `props`: `-o=props .`, which prints the comments a node has on its own lines.

A run is `ok` when both tools print the same bytes and exit with the same status. A mutated input
that makes yq die of a Go panic (it dereferences a nil element after the root's end tag) is `panic`
and is not counted: `succinctly yq` reads it as an error instead (limitations.md). Exit 1 when any
run differs. Standard library only. Every input is a few hundred bytes, but run it under
`.ai/scratch/memcap.py --cap-mb 4000 --scope tree --` like any other run of succinctly.
"""
import argparse
import concurrent.futures
import random
import subprocess
import sys

NAMES = ["a", "b", "c", "item", "x", "a.b", "a-b", "_u", "é", "n:b", "n:c", "xml:lang"]
ATTRS = ["id", "x", "n:y", "xmlns:n", "xmlns", "é", "a-b", "xml:lang"]
TEXTS = [
    "1", "true", "null", "~", "0x1f", "  hi  ", "a&amp;b", "&lt;&gt;", "&#65;&#x42;", "&bogus;",
    "&amp", "&#;", "&#xZZ;", "multi\nline", "x y", "é", "\t", "tab\there", "a\r\nb", "-", "# not",
    "[x]", "{a}", "yes", "'q'", '"q"', "a: b", " nbsp ", "​zw​", "",
]
COMMENTS = ["c", " c ", "a\nb", "", "# x", "- y", "  lead", "trail  ", "x\n\ny", "\nstart", "end\n"]
PIS = ['<?xml version="1.0"?>', '<?xml version="1.0" encoding="UTF-8"?>',
       '<?xml version="1.0" encoding="ISO-8859-1"?>', '<?xml version="1.1"?>',
       '<?php echo 1 ?>', '<?a?>', '<?a b="c"?>']
DIRECTIVES = ["<!DOCTYPE a>", '<!DOCTYPE a [<!ENTITY e "v">]>', "<!ELEMENT a (b)>", "<!-- d -->",
              "<!DOCTYPE a [<!-- in --><!ENTITY e 'x>y'>]>"]
VALUES = ["1", "", "a b", "&amp;", "x<y", "it's", 'say "hi"', "é", "a\nb", "0x1f", "true"]


def attrs(r):
    out = []
    for _ in range(r.choice([0, 0, 0, 1, 1, 2, 3])):
        name = r.choice(ATTRS)
        value = r.choice(VALUES)
        style = r.random()
        if style < 0.55:
            out.append(' %s="%s"' % (name, value.replace('"', "&quot;")))
        elif style < 0.8:
            out.append(" %s='%s'" % (name, value.replace("'", "&apos;")))
        elif style < 0.9:
            out.append(" %s=%s" % (name, r.choice(["v", "1", "a-b", "x_y"])))
        else:
            out.append(" %s" % name)
    return "".join(out)


def content(r, depth):
    parts = []
    for _ in range(r.choice([0, 1, 1, 2, 2, 3, 4])):
        k = r.random()
        if k < 0.30:
            parts.append(r.choice(TEXTS))
        elif k < 0.60 and depth < 4:
            parts.append(element(r, depth + 1))
        elif k < 0.78:
            parts.append("<!--%s-->" % r.choice(COMMENTS))
        elif k < 0.84:
            parts.append("<![CDATA[%s]]>" % r.choice(["x<y", " sp ", "", "a]b", "&amp;"]))
        elif k < 0.88:
            parts.append(r.choice(PIS[4:]))
        elif k < 0.92:
            parts.append(r.choice([" ", "\n", "\n  ", "\t"]))
        elif k < 0.95:
            parts.append(r.choice(DIRECTIVES))
        else:
            parts.append(element(r, depth + 1))
    return "".join(parts)


def element(r, depth):
    name = r.choice(NAMES)
    if r.random() < 0.15:
        return "<%s%s/>" % (name, attrs(r))
    close = name if r.random() > 0.04 else r.choice(NAMES)
    return "<%s%s>%s</%s>" % (name, attrs(r), content(r, depth), close)


def document(r):
    head = ""
    for _ in range(r.choice([0, 0, 1, 1, 2])):
        k = r.random()
        if k < 0.4:
            head += r.choice(PIS)
        elif k < 0.7:
            head += "<!--%s-->" % r.choice(COMMENTS)
        elif k < 0.85:
            head += r.choice(DIRECTIVES)
        else:
            head += r.choice(["\n", " ", "﻿"])
    body = element(r, 0)
    if r.random() < 0.08:
        body += element(r, 0)
    tail = ""
    for _ in range(r.choice([0, 0, 0, 1, 2])):
        k = r.random()
        if k < 0.6:
            tail += "<!--%s-->" % r.choice(COMMENTS)
        elif k < 0.8:
            tail += r.choice(["\n", " ", "text"])
        else:
            tail += r.choice(PIS[4:])
    return head + body + tail


def mutate(r, text):
    data = bytearray(text.encode("utf-8"))
    for _ in range(r.choice([1, 1, 2])):
        if not data:
            break
        i = r.randrange(len(data))
        k = r.random()
        if k < 0.3:
            del data[i]
        elif k < 0.5:
            data.insert(i, data[i])
        elif k < 0.8:
            data[i] = r.choice(b"<>&/\"'=?!-[]; \n\x00\x01:")
        else:
            del data[i:]
    return bytes(data)


FLAG_SETS = [
    [], [], [], ["--xml-strict-mode"], ["--xml-raw-token=false"],
    ["--xml-raw-token=false", "--xml-strict-mode"], ["--xml-keep-namespace=false"],
    ["--xml-skip-proc-inst"], ["--xml-skip-directives"],
    ["--xml-attribute-prefix=@", "--xml-content-name=#text"],
    ["--xml-proc-inst-prefix=?", "--xml-directive-name=!"],
]
OUTPUTS = [["-o=json", "-I0"], ["-o=yaml"], ["-o=props"]]


def run(argv, data):
    try:
        p = subprocess.run(argv, input=data, capture_output=True, timeout=30)
    except subprocess.TimeoutExpired:
        return (-1, b"timeout")
    # An error message holds the raw bytes of an invalid name in Go and U+FFFD here (limitations.md),
    # so the messages are compared as text.
    err = p.stderr.replace(b"\r", b"").decode("utf-8", "replace").encode("utf-8")
    return (p.returncode, p.stdout + b"\n--stderr--\n" + err)


def one(args):
    target, case, data, flags, output = args
    base = ["-p", "xml"] + flags + output + ["."]
    want = run(["yq"] + base, data)
    if b"panic: runtime error" in want[1]:
        return (case, "panic", None, None, data, flags, output)
    got = run([target, "yq"] + base, data)
    if want == got:
        return (case, "ok", None, None, data, flags, output)
    return (case, "DIFF", want, got, data, flags, output)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("target")
    ap.add_argument("--n", type=int, default=400)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--show", type=int, default=12)
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--mutate", type=float, default=0.35)
    ap.add_argument("--only", choices=["json", "yaml", "props"])
    ap.add_argument("--diff", action="store_true", help="show a unified diff, not both outputs")
    opts = ap.parse_args()
    r = random.Random(opts.seed)
    work = []
    for case in range(opts.n):
        text = document(r)
        data = text.encode("utf-8")
        if r.random() < opts.mutate:
            data = mutate(r, text)
        flags = r.choice(FLAG_SETS)
        for output in OUTPUTS:
            if opts.only and ("-o=" + opts.only) not in output:
                continue
            work.append((opts.target, case, data, flags, output))
    counts = {"ok": 0, "DIFF": 0, "panic": 0}
    shown = 0
    with concurrent.futures.ThreadPoolExecutor(opts.jobs) as pool:
        for case, status, want, got, data, flags, output in pool.map(one, work):
            counts[status] += 1
            if status == "DIFF" and shown < opts.show:
                shown += 1
                print("=== case %d %s %s" % (case, flags, output))
                print("input: %r" % data)
                if opts.diff:
                    import difflib
                    a = want[1].decode("utf-8", "replace").splitlines()
                    b = got[1].decode("utf-8", "replace").splitlines()
                    print("exit yq %s, succinctly %s" % (want[0], got[0]))
                    print("\n".join(difflib.unified_diff(a, b, "yq", "succinctly", lineterm="", n=2)))
                else:
                    print("--- yq (exit %s)\n%s" % (want[0], want[1].decode("utf-8", "replace")))
                    print("--- succinctly (exit %s)\n%s" % (got[0], got[1].decode("utf-8", "replace")))
    print("runs: %d ok, %d differ, %d yq panics (skipped)" % (counts["ok"], counts["DIFF"], counts["panic"]))
    sys.exit(1 if counts["DIFF"] else 0)


main()
