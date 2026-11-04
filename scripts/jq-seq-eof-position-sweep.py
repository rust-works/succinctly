#!/usr/bin/env python3
"""Sweep of `--seq` streams against jq: where `input` past the end names its source (#6146).

    scripts/jq-seq-eof-position-sweep.py SUBJECT [ORACLE]

Every stream of up to four pieces drawn from `PIECES` (RS, newline, blank, `1`, `[1]`, `2`) is
read by `[.,input]`, `-n [.,input]` and `-n [.,input,input]` with `--seq -c` in both tools, and the
exit status, stdout and the `(at ...)` marker of stderr are compared. Exit 0 when they agree, 1
with the differences listed by kind. Streams that end in a fault after their last value differ in
the wording jq gives the fault (#6137) and are listed, not hidden.
"""
import collections
import itertools
import os
import re
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

RS = "\x1e"
PIECES = ["1", " ", "\n", RS, "[1]", "2"]
PROGRAMS = [("[.,input]", []), ("[.,input]", ["-n"]), ("[.,input,input]", ["-n"])]


def marker(stderr):
    found = re.search(r"\(at ([^)]*)\)", stderr)
    return found.group(1) if found else None


def run(job):
    subject, oracle, stream, program, flags = job
    out = []
    for argv in ([oracle], [subject, "jq"]):
        done = subprocess.run(argv + ["-c", "--seq"] + flags + [program],
                              input=stream.encode(), capture_output=True, timeout=10)
        out.append((done.returncode, done.stdout, marker(done.stderr.decode())))
    return job, out


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    subject = sys.argv[1]
    oracle = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("JQ_ORACLE", "/usr/bin/jq")
    streams = sorted({"".join(c) for n in range(1, 5) for c in itertools.product(PIECES, repeat=n)})
    jobs = [(subject, oracle, s, p, f) for s in streams for p, f in PROGRAMS]
    kinds = collections.Counter()
    shown = {}
    with ThreadPoolExecutor(8) as pool:
        for job, (want, got) in pool.map(run, jobs):
            if want != got:
                kinds[(want[2], got[2])] += 1
                shown.setdefault((want[2], got[2]), job)
    print(f"{len(jobs)} runs, {sum(kinds.values())} differ")
    for (want, got), count in kinds.most_common():
        _, _, stream, program, flags = shown[(want, got)]
        print(f"  {count:4d}  jq {want}  subject {got}  e.g. {stream!r} {flags} {program}")
    sys.exit(1 if kinds else 0)


main()
