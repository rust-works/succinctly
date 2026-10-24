#!/usr/bin/env python3
"""Recording shim for the #5223 Phase 0 report: stands in for a succinctly
binary under a differential sweep, logs every row the sweep runs, and answers
with the real binary.

Point a sweep's `--bin` (or `--candidate`) at this file with

    PATH_MODE_RECORD_BIN=<a main build>  PATH_MODE_RECORD_DIR=<empty dir>

and every invocation appends one JSON line, `{"args": [...], "stdin": "..."}`
(the argv after the binary, and the bytes on stdin), to
`$PATH_MODE_RECORD_DIR/rows.jsonl` under an exclusive lock, so parallel sweep jobs
never interleave. The sweep itself sees exactly what the real binary does, so
its own verdicts stay valid. `scripts/jq-path-mode-3way.py --rows` reads the
directory back. Nothing in the sweeps changes.
"""
import fcntl
import json
import os
import subprocess
import sys


def main():
    real = os.environ["PATH_MODE_RECORD_BIN"]
    out_dir = os.environ["PATH_MODE_RECORD_DIR"]
    data = sys.stdin.buffer.read()
    record = {"args": sys.argv[1:], "stdin": data.decode("utf-8", "surrogateescape")}
    with open(os.path.join(out_dir, "rows.jsonl"), "a", encoding="utf-8") as log:
        fcntl.flock(log, fcntl.LOCK_EX)
        log.write(json.dumps(record) + "\n")
    done = subprocess.run([real] + sys.argv[1:], input=data, capture_output=True)
    sys.stdout.buffer.write(done.stdout)
    sys.stderr.buffer.write(done.stderr)
    sys.exit(done.returncode)


if __name__ == "__main__":
    main()
