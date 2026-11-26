#!/usr/bin/env python3
"""The call machine's corpus gate (#7977): every program of `cases.py` against jq 1.7.1.

    python3 scripts/call_machine/check.py capture [--jq PATH]   # record jq's answers
    python3 scripts/call_machine/check.py check BIN [--off]     # run `BIN jq ...` on each, compare

`capture` runs each case through jq (`/usr/bin/jq` unless `--jq` names one; its version must
start `jq-1.7.1`) and writes `tests/data/call-machine-corpus-7977.json`: per case its id,
arguments, filter, route, and jq's stdout, stderr and exit status. `tests/jq_cli/i7977.rs` runs
the same golden file through the test binary, so CI checks what this script checks.

`check` runs each case with `SUCCINCTLY_CALL_MACHINE=1` (`=0` with `--off`) and
`SUCCINCTLY_DEBUG_CALL_MACHINE_LOG` naming a file, from which it reads whether the machine took
the program. A case is ok when the three answers equal jq's and, with the switch on, a "take"
case was taken. With `--off` nothing reaches the machine, and every case but `deep_ok_10000`
(which the shipped evaluators refuse at their native-stack floor) should still match.

Run from anywhere: paths are relative to this file.
"""
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
from cases import CORRECT  # noqa: E402

GOLDEN = os.path.join(ROOT, "tests", "data", "call-machine-corpus-7977.json")
SWITCH = "SUCCINCTLY_CALL_MACHINE"
LOG = "SUCCINCTLY_DEBUG_CALL_MACHINE_LOG"
JQ = "/usr/bin/jq"
TIMEOUT = 120


def run(cmd, env=None):
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=TIMEOUT, env=env, stdin=subprocess.DEVNULL)
        return dict(stdout=p.stdout.decode("utf-8", "replace"), stderr=p.stderr.decode("utf-8", "replace"), exit=p.returncode)
    except subprocess.TimeoutExpired:
        return dict(stdout="", stderr="TIMEOUT", exit=None)


def capture(jq):
    version = subprocess.run([jq, "--version"], capture_output=True, text=True).stdout.strip()
    if not version.startswith("jq-1.7.1"):
        sys.exit(f"{jq} is {version!r}, not jq-1.7.1")
    rows = []
    env = dict(os.environ, LC_ALL="C")
    for c in CORRECT:
        got = run([jq] + c["args"] + [c["filter"]], env)
        rows.append(dict(id=c["id"], args=c["args"], filter=c["filter"], route=c["route"], **got))
        print(c["id"], got["exit"])
    with open(GOLDEN, "w") as f:
        json.dump(rows, f, indent=1)
        f.write("\n")


def check(binary, off=False):
    with open(GOLDEN) as f:
        golden = {row["id"]: row for row in json.load(f)}
    bad = 0
    rows = []
    for c in CORRECT:
        log = tempfile.NamedTemporaryFile(delete=False, prefix="call-machine-")
        log.close()
        env = dict(os.environ, LC_ALL="C", **{SWITCH: "0" if off else "1", LOG: log.name})
        got = run([binary, "jq"] + c["args"] + [c["filter"]], env)
        with open(log.name) as f:
            decisions = f.read().split()
        os.unlink(log.name)
        taken = "taken" in decisions
        want = golden[c["id"]]
        if want["filter"] != c["filter"] or want["args"] != c["args"]:
            sys.exit(f"{c['id']}: {GOLDEN} is stale, run `check.py capture`")
        want = dict(stdout=want["stdout"], stderr=want["stderr"], exit=want["exit"])
        same = got == want
        route_ok = off or (taken if c["route"] == "take" else True)
        status = "ok" if same and route_ok else ("WRONG" if not same else "NOT-TAKEN")
        if status != "ok":
            bad += 1
        rows.append((c["id"], c["route"], "taken" if taken else "declined", status))
        if not same:
            print(f"--- {c['id']}: expected {want!r}\n    got      {got!r}")
    for r in rows:
        print("%-28s %-8s %-9s %s" % r)
    print(f"{len(rows) - bad} of {len(rows)} ok")
    return bad


def main(argv):
    if len(argv) >= 2 and argv[1] == "capture":
        jq = argv[argv.index("--jq") + 1] if "--jq" in argv else JQ
        capture(jq)
        return 0
    if len(argv) >= 3 and argv[1] == "check":
        return 1 if check(argv[2], "--off" in argv) else 0
    sys.exit(__doc__)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
