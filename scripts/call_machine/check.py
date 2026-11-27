#!/usr/bin/env python3
"""The call machine's corpus gate (#7977): every program of `cases.py` against jq 1.7.1.

    python3 scripts/call_machine/check.py capture [--jq PATH]   # record jq's answers
    python3 scripts/call_machine/check.py check BIN [--off]     # run `BIN jq ...` on each, compare

`capture` runs each case through jq (`/usr/bin/jq` unless `--jq` names one; its version must
start `jq-1.7.1`) and writes `tests/data/call-machine-corpus-7977.json`: per case its id,
arguments, filter, route, and jq's stdout, stderr and exit status. `tests/jq_cli/i7977.rs` runs
the same golden file through the test binary, so CI checks what this script checks.

`check` runs each case, with its standard input if it has one, under `SUCCINCTLY_CALL_MACHINE=1`
(`=0` with `--off`) and `SUCCINCTLY_DEBUG_CALL_MACHINE_LOG` naming a file, from which it reads the
machine's routing decisions. With the switch on, a case is ok when the three answers equal jq's
and every decision is `taken` for a "take" case, `declined` for a "decline" one, with none for a
"compile-error" one. With `--off` no
decision may be made, and a case whose `off_known` says why the shipped route answers otherwise
(`deep_ok_10000`, refused at the native-stack floor) is reported `known`, not a failure.

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


def run(cmd, env=None, stdin=None):
    """Run `cmd` with `stdin` (text) on its standard input, or none (`/dev/null`)."""
    feed = dict(input=stdin.encode()) if stdin is not None else dict(stdin=subprocess.DEVNULL)
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=TIMEOUT, env=env, **feed)
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
        got = run([jq] + c["args"] + [c["filter"]], env, c["stdin"])
        rows.append(dict(id=c["id"], args=c["args"], filter=c["filter"], route=c["route"], stdin=c["stdin"], **got))
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
        got = run([binary, "jq"] + c["args"] + [c["filter"]], env, c["stdin"])
        with open(log.name) as f:
            decisions = f.read().split()
        os.unlink(log.name)
        taken = "taken" in decisions
        want = golden.get(c["id"])
        if want is None or any(want[k] != c[k] for k in ("filter", "args", "route", "stdin")):
            sys.exit(f"{c['id']}: {GOLDEN} is stale, run `check.py capture`")
        want = dict(stdout=want["stdout"], stderr=want["stderr"], exit=want["exit"])
        same = got == want
        if off:
            route_ok = not decisions
        elif c["route"] == "compile-error":
            route_ok = not decisions
        else:
            want_decision = "taken" if c["route"] == "take" else "declined"
            route_ok = bool(decisions) and all(d == want_decision for d in decisions)
        if not same:
            status = "known" if off and c.get("off_known") else "WRONG"
        else:
            status = "ok" if route_ok else "MISROUTED"
        if status not in ("ok", "known"):
            bad += 1
        rows.append((c["id"], c["route"], "taken" if taken else "declined", status))
        if not same:
            print(f"--- {c['id']}: expected {want!r}\n    got      {got!r}")
    for r in rows:
        print("%-28s %-8s %-9s %s" % r)
    known = sum(1 for r in rows if r[3] == "known")
    print(f"{len(rows) - bad} of {len(rows)} ok" + (f" ({known} known on the shipped route)" if known else ""))
    return bad


def main(argv):
    import argparse

    ap = argparse.ArgumentParser(description="The call machine's corpus gate against jq 1.7.1.")
    sub = ap.add_subparsers(dest="command", required=True)
    cap = sub.add_parser("capture", help="record jq's answers into the golden file")
    cap.add_argument("--jq", default=JQ)
    chk = sub.add_parser("check", help="run `BIN jq ...` on every case and compare")
    chk.add_argument("binary")
    chk.add_argument("--off", action="store_true", help="with SUCCINCTLY_CALL_MACHINE=0")
    a = ap.parse_args(argv[1:])
    if a.command == "capture":
        capture(a.jq)
        return 0
    return 1 if check(a.binary, a.off) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
