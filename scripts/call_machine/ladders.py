#!/usr/bin/env python3
"""Measurement ladders for the call machine (#7977, from the #7957 spikes). Run on the bench boxes
only, never a laptop; one invocation per benchlock hold. It stops at a time budget and resumes
from its state file.

  python3 scripts/call_machine/ladders.py \
      --bin off=bin/s@SUCCINCTLY_CALL_MACHINE=0 --bin offA=bin/sA@SUCCINCTLY_CALL_MACHINE=0 \
      --bin on=bin/s@SUCCINCTLY_CALL_MACHINE=1 --jq jq171=PATH --aa off,offA \
      --state results/timing.jsonl --batch b01 --budget 780 [--reps 5] [--cachegrind --cell-cap 1800]

`--bin LABEL=PATH[@VAR=VALUE]...` sets each `VAR` in that binary's environment, so one binary is
measured with the switch set in both arms at equal length (`=1` against `=0`, never present
against absent: the environment's size moves the initial stack, #7961). `--aa` names the A/A pair.

Exit status: 0 every cell done; 3 cells remain (budget reached); 4 the batch failed the A/A gate
(its rows are kept, marked, and re-run by the next invocation, at most twice per cell).

- Interleaved: each repetition runs every binary in a freshly shuffled order.
- Per run: wall time, peak RSS, outcome (ok / refused / oom / crash / timeout / error), and on
  macOS instructions retired from `/usr/bin/time -l`.
- `--cachegrind` (Linux): one valgrind cachegrind run per binary per cell instead (instruction counts).
- Output gate: a succinctly cell is "ok" only when its stdout equals jq's.
- Every child: its own process group (killed as a group on timeout), RLIMIT_CORE (1, 1), and on
  Linux RLIMIT_AS 8 GiB.
"""
import argparse, json, os, platform, random, re, resource, signal, statistics, subprocess, sys, tempfile, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from cases import LADDERS

DARWIN = platform.system() == "Darwin"
AS_CAP = 8 << 30
AA_LIMIT = 0.015


def child_limits():
    os.setpgid(0, 0)
    resource.setrlimit(resource.RLIMIT_CORE, (1, 1))
    if not DARWIN:
        resource.setrlimit(resource.RLIMIT_AS, (AS_CAP, AS_CAP))


def spawn(cmd, timeout, env=None, stdout=subprocess.PIPE, stderr=subprocess.PIPE):
    p = subprocess.Popen(cmd, stdout=stdout, stderr=stderr, stdin=subprocess.DEVNULL, preexec_fn=child_limits, env=env)
    try:
        out, err = p.communicate(timeout=timeout)
        return p.returncode, out, err
    except subprocess.TimeoutExpired:
        try:
            os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        p.communicate()
        return None, b"", b""


def run_once(cmd, timeout, env=None):
    tcmd = ["/usr/bin/time", "-l"] if DARWIN else ["/usr/bin/time", "-v"]
    t0 = time.perf_counter()
    rc, out, err = spawn(tcmd + cmd, timeout, env)
    wall = time.perf_counter() - t0
    if rc is None:
        return dict(outcome="timeout", wall=None, rss=None, instr=None, stdout="", stderr_tail=[])
    e = err.decode("utf-8", "replace")
    if DARWIN:
        m = re.search(r"(\d+)\s+maximum resident set size", e)
        rss = int(m.group(1)) if m else None
        m = re.search(r"(\d+)\s+instructions retired", e)
        instr = int(m.group(1)) if m else None
    else:
        m = re.search(r"Maximum resident set size \(kbytes\): (\d+)", e)
        rss = int(m.group(1)) * 1024 if m else None
        instr = None
    if rc == 0:
        outcome = "ok"
    elif "exceeded maximum recursion depth" in e:
        outcome = "refused"
    elif "memory allocation" in e or "out of memory" in e.lower():
        outcome = "oom"
    elif "signal" in e.lower() or rc > 128:
        outcome = f"crash({rc})"
    else:
        outcome = f"error({rc})"
    return dict(outcome=outcome, wall=wall, rss=rss, instr=instr, stdout=out.decode("utf-8", "replace"),
                stderr_tail=[l for l in e.strip().splitlines() if not l.startswith((" ", "\t"))][:2])


def cachegrind(cmd, cap, env=None):
    with tempfile.NamedTemporaryFile(suffix=".cg") as f:
        rc, out, err = spawn(["valgrind", "--tool=cachegrind", "--cache-sim=no", f"--cachegrind-out-file={f.name}"] + cmd, cap, env)
    if rc is None:
        return None, "timeout"
    m = re.search(rb"I\s+refs:\s+([\d,]+)", err)
    return (int(m.group(1).replace(b",", b"")) if m else None), ("ok" if rc == 0 else f"rc={rc}")


def heartbeat(path, stop):
    while not stop.wait(30):
        try:
            os.utime(path, None) if os.path.exists(path) else open(path, "a").close()
        except OSError:
            pass


ASSIGNMENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=")


def parse_bin(spec):
    """`LABEL=PATH[@VAR=VALUE]...` as `(label, path, env)`, env `None` when no variable is set.

    The assignments are the trailing `@` segments that start `NAME=`, so a path may hold `@`."""
    label, rest = spec.split("=", 1)
    parts = rest.split("@")
    assignments = []
    while len(parts) > 1 and ASSIGNMENT.match(parts[-1]):
        assignments.insert(0, parts.pop())
    path = "@".join(parts)
    if not assignments:
        return label, path, None
    env = dict(os.environ)
    for a in assignments:
        var, value = a.split("=", 1)
        env[var] = value
    return label, path, env


def cells():
    for lad in LADDERS:
        for n in lad["sizes"]:
            yield f"{lad['id']}@{n}", lad, n


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", action="append", default=[])
    ap.add_argument("--jq", action="append", default=[])
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--only", default="")
    ap.add_argument("--cachegrind", action="store_true")
    ap.add_argument("--state", required=True)
    ap.add_argument("--batch", required=True)
    ap.add_argument("--budget", type=float, default=780)
    ap.add_argument("--timeout", type=float, default=120)
    ap.add_argument("--cell-cap", type=float, default=1800)
    ap.add_argument("--aa", default="main,mainA")
    ap.add_argument("--heartbeat", default="")
    a = ap.parse_args()
    stop = threading.Event()
    if a.heartbeat:
        threading.Thread(target=heartbeat, args=(a.heartbeat, stop), daemon=True).start()
    bins = [(l, [p, "jq"], False) for l, p, _ in map(parse_bin, a.bin)]
    bins += [(l, [p], True) for l, p in (s.split("=", 1) for s in a.jq)]
    envs = {l: e for l, _, e in map(parse_bin, a.bin)}
    if not a.cachegrind:
        missing = [l for l in a.aa.split(",") if l not in {b[0] for b in bins}]
        if missing:
            sys.exit(f"--aa names {','.join(missing)}, which no --bin or --jq labels")
    only = set(filter(None, a.only.split(",")))
    done, aa_fails = set(), {}
    if os.path.exists(a.state):
        for line in open(a.state):
            r = json.loads(line)
            if r.get("aa_fail"):
                aa_fails[r["cell"]] = aa_fails.get(r["cell"], 0) + 1
            else:
                done.add(r["cell"])
    start = time.monotonic()
    rows = []
    remaining = False
    for cid, lad, n in cells():
        if only and not any(lad["id"].startswith(o) for o in only):
            continue
        if cid in done or aa_fails.get(cid, 0) >= 3:
            continue
        if time.monotonic() - start > a.budget:
            remaining = True
            break
        filt = lad["filter"].replace("{N}", str(n))
        row = dict(cell=cid, ladder=lad["id"], n=n, batch=a.batch, host=platform.node(), cells={})
        if a.cachegrind:
            # the unit is (cell, binary): each is capped, and the budget is checked before each
            for label, base, _ in bins:
                uid = f"{cid}#{label}"
                if uid in done:
                    continue
                if time.monotonic() - start > a.budget:
                    remaining = True
                    break
                ir, st = cachegrind(base + ["-n", "-c", filt], a.cell_cap, envs.get(label))
                urow = dict(cell=uid, ladder=lad["id"], n=n, label=label, batch=a.batch, host=platform.node(),
                            ir=ir, status=st)
                with open(a.state, "a") as f:
                    f.write(json.dumps(urow) + "\n")
                print(json.dumps(urow), flush=True)
            if remaining:
                break
            continue
        else:
            per = {l: [] for l, _, _ in bins}
            dead = set()
            cell_reps = a.reps
            cut = False
            for rep in range(a.reps):
                order = list(bins)
                random.shuffle(order)
                for label, base, _ in order:
                    if label in dead:
                        continue
                    left = a.budget - (time.monotonic() - start)
                    t = min(a.timeout, max(5.0, left + 90))
                    r = run_once(base + ["-n", "-c", filt], t, envs.get(label))
                    if r["outcome"] == "timeout" and t < a.timeout and rows:
                        cut = True  # cut by the hold, not by the cell's own timeout: redo the cell next batch
                        break
                    per[label].append(r)
                    if r["outcome"] == "timeout" or r["outcome"].startswith("crash") or r["outcome"] == "oom":
                        dead.add(label)
                if cut:
                    break
                if rep + 1 < cell_reps and time.monotonic() - start > a.budget:
                    cut = rep + 1 < 3 and bool(rows)  # keep 3+ repetitions, or the batch's first cell; else redo it
                    cell_reps = rep + 1
                    break
                if rep == 0:
                    # cut the repetitions of a slow cell so one cell cannot overrun the hold
                    first = sum((per[l][0]["wall"] or a.timeout) for l, _, _ in bins if per[l])
                    left = a.budget - (time.monotonic() - start)
                    fit = int(left // first) if first > 0 else a.reps
                    if fit < a.reps - 1:
                        cell_reps = max(2, fit + 1)
                if rep + 1 >= cell_reps:
                    break
            if cut:
                remaining = True
                break
            ref = next((per[l][0]["stdout"] for l, _, isjq in bins if isjq and per[l] and per[l][0]["outcome"] == "ok"), None)
            for label, base, isjq in bins:
                rs = per[label]
                walls = [r["wall"] for r in rs if r["wall"] is not None and r["outcome"] == "ok"]
                out = rs[0]["outcome"] if rs else "skipped"
                if out == "ok" and ref is not None and not isjq and rs[0]["stdout"] != ref:
                    out = "WRONG"
                row["cells"][label] = dict(
                    outcome=out, reps=len(rs),
                    wall_min=min(walls) if walls else None,
                    wall_med=statistics.median(walls) if walls else None,
                    rss_max=max((r["rss"] for r in rs if r["rss"]), default=None),
                    instr_min=min((r["instr"] for r in rs if r["instr"]), default=None),
                    stderr=rs[0]["stderr_tail"] if rs and out != "ok" else None)
        rows.append(row)
        print(json.dumps(row), flush=True)
    status = 0 if not remaining else 3
    if not a.cachegrind and rows:
        x, y = a.aa.split(",")
        ratios = [r["cells"][y]["wall_med"] / r["cells"][x]["wall_med"] for r in rows
                  if r["cells"].get(x, {}).get("wall_med") and r["cells"].get(y, {}).get("wall_med")
                  and r["cells"][x]["wall_med"] > 0.005]
        aa = statistics.median(ratios) - 1 if ratios else 0.0
        print(json.dumps(dict(batch=a.batch, aa=aa, aa_cells=len(ratios))), flush=True)
        if abs(aa) > AA_LIMIT:
            for r in rows:
                r["aa_fail"] = True
            status = 4
    with open(a.state, "a") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    stop.set()
    sys.exit(status)


if __name__ == "__main__":
    main()
