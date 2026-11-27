#!/usr/bin/env python3
"""Per-record cost of a routed program over NDJSON against one document (#7977 step 5).

  python3 scripts/call_machine/per_record.py \
      --bin base=bin/s0@SUCCINCTLY_CALL_MACHINE=1 --bin new=bin/s1@SUCCINCTLY_CALL_MACHINE=1 \
      [--records 100000] [--reps 7] [--program depth8] [--out results/per-record.jsonl]

Each program is one `def` and a body. For every binary it times two runs that do the same work
and print the same lines:

- `ndjson`: `def ...; BODY` over a file of N records, one small number per line, so the program
  runs N times, once per document;
- `single`: `def ...; .[] | BODY` over one document holding the same N numbers in an array, so
  the program runs once and the body N times.

The per-record cost is each run's wall time over N, and the step 5 criterion is that `ndjson`'s is
within 10% of `single`'s: what a document costs beyond its record is the route, the input and the
output, not the program's compilation. The script reports the ratio and says PASS or FAIL against
1.10; on a laptop that is a sanity check, not a timing claim (run it on the bench boxes, under the
benchlock, for that).

Give the same binary with `=0` as a control: the shipped route's own ratio is what a document
costs outside the machine (reading it, routing it, printing it), which no change to the machine
removes.

`--bin LABEL=PATH[@VAR=VALUE]...` sets each `VAR` in that binary's environment, as `ladders.py`
does. Repetitions are interleaved: each one runs every (binary, workload) pair in a freshly
shuffled order. A binary's two outputs must be equal to each other and to the first binary's, or
the script stops: a timing of a wrong answer means nothing.
"""
import argparse
import json
import os
import random
import statistics
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from ladders import parse_bin  # noqa: E402

PROGRAMS = {
    # Linear recursion a few levels deep per record: the call machine's frames, a value
    # parameter, no leaf past the comparison.
    "depth8": ("def f($n): if $n == 0 then . else f($n - 1) end;", "f(. % 8)"),
    # A caught `break` per record: jq numbers labels across documents, so the label counter
    # carries from one document to the next.
    "label": (
        "def f($n): if $n == 0 then . else f($n - 1) end;",
        "f(. % 3) as $x | [label $l | try break $l catch .] | [$x, .[0].__jq]",
    ),
    # A generator per record: fork points, backtracking, a collected array.
    "nat": ("def nat: ., (. + 1 | nat);", "[limit(. % 5 + 1; 0 | nat)]"),
}
LIMIT = 1.10
LOG = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG"


def run(path, env, args, log=None):
    env = dict(env if env is not None else os.environ)
    if log:
        env[LOG] = log
    t0 = time.perf_counter()
    p = subprocess.run([path, "jq", "-c"] + args, env=env, capture_output=True, stdin=subprocess.DEVNULL)
    wall = time.perf_counter() - t0
    if p.returncode != 0:
        sys.exit(f"{path} {args[0]!r}: exit {p.returncode}: {p.stderr.decode()[-400:]}")
    return wall, p.stdout


def events(path, env, args, tmp):
    """The decision log of one untimed run: (decisions, compilations)."""
    log = os.path.join(tmp, "decisions.log")
    if os.path.exists(log):
        os.unlink(log)
    run(path, env, args, log)
    lines = open(log).read().split() if os.path.exists(log) else []
    return [e for e in lines if e != "compiled"], sum(e == "compiled" for e in lines)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--bin", action="append", required=True, help="LABEL=PATH[@VAR=VALUE]...")
    ap.add_argument("--records", type=int, default=100_000)
    ap.add_argument("--reps", type=int, default=7)
    ap.add_argument("--program", action="append", choices=sorted(PROGRAMS), help="default: all")
    ap.add_argument("--out", help="append one JSON row per (program, binary) here")
    a = ap.parse_args(argv[1:])
    bins = [parse_bin(b) for b in a.bin]
    n = a.records
    with tempfile.TemporaryDirectory(prefix="per-record-") as tmp:
        ndjson = os.path.join(tmp, "records.ndjson")
        single = os.path.join(tmp, "records.json")
        values = [str(i % 7) for i in range(n)]
        with open(ndjson, "w") as f:
            f.write("\n".join(values) + "\n")
        with open(single, "w") as f:
            f.write("[" + ",".join(values) + "]\n")
        failed = 0
        for name in a.program or sorted(PROGRAMS):
            prelude, body = PROGRAMS[name]
            work = {
                "ndjson": [f"{prelude} {body}", ndjson],
                "single": [f"{prelude} .[] | {body}", single],
            }
            want = None
            times = {(b[0], w): [] for b in bins for w in work}
            for _ in range(a.reps):
                pairs = [(b, w) for b in bins for w in work]
                random.shuffle(pairs)
                outs = {}
                for (label, path, env), w in pairs:
                    wall, out = run(path, env, work[w])
                    times[(label, w)].append(wall)
                    outs[(label, w)] = out
                for (label, w), out in outs.items():
                    want = want if want is not None else out
                    if out != want:
                        sys.exit(f"{name}: {label} {w} printed other lines than {bins[0][0]}")
            print(f"{name}: {n} records, {a.reps} interleaved reps, median wall")
            for label, path, env in bins:
                # One untimed run each, with the decision log: is it the machine's, and how often
                # did it decide and compile? (A binary without the log line counts none.)
                for w in work:
                    decided, compiled = events(path, env, work[w], tmp)
                    print(f"  {label:<10} {w:<7} decisions {decided} compilations {compiled}")
            for label, path, _ in bins:
                nd = statistics.median(times[(label, "ndjson")])
                sd = statistics.median(times[(label, "single")])
                ratio = nd / sd
                verdict = "PASS" if ratio <= LIMIT else "FAIL"
                failed += verdict == "FAIL"
                print(
                    f"  {label:<10} ndjson {nd * 1e3:9.1f} ms ({nd / n * 1e6:6.2f} us/record)"
                    f"  single {sd * 1e3:9.1f} ms ({sd / n * 1e6:6.2f} us/record)"
                    f"  ratio {ratio:5.3f} {verdict}"
                )
                if a.out:
                    with open(a.out, "a") as f:
                        row = dict(program=name, bin=label, path=path, records=n, reps=a.reps,
                                   ndjson=times[(label, "ndjson")], single=times[(label, "single")])
                        f.write(json.dumps(row) + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
