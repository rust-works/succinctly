#!/usr/bin/env python3
"""Regenerate `symbols.txt`, the function order `link.py` gives the release binary (#7260).

    # Linux x86_64 with valgrind: the whole list, from a release binary built from this tree
    python3 scripts/link-order/generate.py linux target/release/succinctly data/compare/inputs > symbols.txt
    # macOS: add the arm64/macOS-only functions the Linux trace cannot see (all of them: not a sample)
    python3 scripts/link-order/generate.py macos target/release/succinctly symbols.txt > new.txt

`linux` runs every row of `comparison/jq/queries/perf.tsv` at 1 MB (and two startup-only runs)
under valgrind. The startup runs (`jq -n 1`, `jq .` on empty input) give the first block, in the
order their functions first execute (lackey superblock trace); then comes every other function
callgrind saw execute in any row, those run by the most rows first, then by instructions. Last come
the functions only the rows at 10 MB run, ranked the same way (#7412): a file of 4 MiB or more takes
the windowed routes (`jq_runner/window.rs`), which no 1 MB row reaches, and left unordered their
two dozen functions kept three or four large folios of text resident in every stream row.

    # Add that 10 MB tail to an existing list without reordering it
    python3 scripts/link-order/generate.py extend target/release/succinctly data/compare/inputs symbols.txt > new.txt

`macos` cannot trace (no valgrind, and lldb needs a debugging grant) and does not sample either: it
adds EVERY function of BINARY whose name looks platform-specific (NEON, aarch64, Apple) and that the
list lacks, at the end of the startup block, whether or not any run calls it (#7826). It used to read
`mincore()` at exit from a fresh clone of the binary, to keep only the functions on touched pages;
on macOS that reads every `__TEXT` page as resident in every run (XProtect scans a fresh clone on its
first exec, and `MINCORE_REFERENCED` on the original reads the same), so it never kept less than
all of them. The hot functions of the arm64 route that are not NEON kernels come from the Linux
trace: their names are the same on both targets. It prints what it added on stderr.

The list is demangled names (see link.py); a name that later stops existing is skipped at link time,
so a stale list degrades gradually: functions new to the startup path land outside the hot block.
Two checks (ci.yml's perf-guard job, x86_64) catch that:

`check BINARY symbols.txt [LIMIT]` fails when a startup run executes more than LIMIT bytes (64 KB) of
functions the list misses, or when BINARY was not linked in its order.

`stale TARGET_DIR_OR_LOG [PERCENT]` reads the wrapper's `*.link-order.log` of a release build (the
newest one under `target/release`) and fails when more than PERCENT of the listed names no longer
resolve (default 5, see STALE_PERCENT). `check` alone missed #7894: over 222 merges the share
went from 3.4% to 8.0% and every small run held 1.3 MB more of the binary resident on Linux, while
the two startup runs `check` traces stayed inside its 64 KB.
"""
import bisect
import collections
import concurrent.futures as cf
import glob
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
PERF = os.path.join(ROOT, "comparison", "jq", "queries", "perf.tsv")
MARK = "# -- end of startup block --"
STARTUP = [(["-n", "1"], None), (["."], "/dev/null")]
sys.path.insert(0, HERE)
from link import CRT_STARTUP, demangle, norm  # noqa: E402


def rows(inputs, size="1mb"):
    """(args after `jq`, stdin path) for every perf.tsv row whose input exists."""
    out = []
    for line in open(PERF):
        if line.startswith("#") or not line.strip():
            continue
        _id, flags, filt, shapes, _tier = line.rstrip("\n").split("\t")
        for shape in shapes.split(","):
            inp = None
            if shape != "none":
                name = f"{shape}.json" if shape.startswith("corpus") else f"{shape}-{size}.json"
                inp = os.path.join(inputs, name)
                if not os.path.exists(inp):
                    continue
            fl = [] if flags == "-" else flags.split(" ")
            args, nfiles, stdin = [], 1, False
            for f in fl:
                if f.startswith("@files="):
                    nfiles = int(f[7:])
                elif f == "@stdin":
                    stdin = True
                elif f == "@INPUT@":
                    args.append(inp)
                else:
                    args.append(f)
            args.append(filt)
            si = None
            if inp and "@INPUT@" not in fl:
                if stdin:
                    si = inp
                else:
                    args += [inp] * nfiles
            out.append((args, si))
    return out


def nm_pairs(binary):
    """[(addr, size, mangled, demangled)] for the text symbols of `binary`, by address."""
    out = subprocess.run(["nm", "-p", "-S", "--defined-only", binary], capture_output=True,
                         text=True, check=True).stdout.splitlines()
    syms = []
    for line in out:
        p = line.split(" ", 3)
        if len(p) == 4 and p[2] in "tTwW":
            syms.append((int(p[0], 16), int(p[1], 16), p[3]))
    dem = demangle([s[2] for s in syms])
    syms = sorted((a, n, m, d) for (a, n, m), d in zip(syms, dem))
    if syms and not any(s[1] for s in syms):  # Mach-O symbols carry no size: run to the next one
        syms = [(a, b[0] - a if b else 1, m, d) for (a, _, m, d), b in zip(syms, syms[1:] + [None])]
    return syms


def lackey_order(binary, args, stdin, syms):
    """Functions of `binary` in the order they first execute, from a lackey superblock trace."""
    entry = int(re.search(r"Entry point address:\s+(0x[0-9a-f]+)",
                          subprocess.run(["readelf", "-h", binary], capture_output=True,
                                         text=True).stdout).group(1), 16)
    pie = "DYN" in subprocess.run(["readelf", "-h", binary], capture_output=True, text=True).stdout
    with tempfile.TemporaryDirectory() as td:
        log = os.path.join(td, "sb")
        with open(stdin or "/dev/null") as si:
            subprocess.run(["valgrind", "--tool=lackey", "--trace-superblocks=yes", f"--log-file={log}",
                            binary, "jq", *args], stdin=si, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL)
        sbs = [int(l[3:], 16) for l in open(log) if l.startswith("SB ")]
    addrs = [s[0] for s in syms]
    hi = syms[-1][0] + syms[-1][1]

    uniq = sorted(set(sbs))

    starts = set(addrs)

    def hits(base):  # superblocks that begin exactly at a function: calls land there
        return sum(1 for a in uniq if a - base in starts)

    # A PIE's load base: some superblock is its entry point, so the base is one of those
    # differences; the right one starts the most superblocks exactly at one of its functions.
    base = 0
    if pie:
        cands = {a - entry for a in sbs if a >= entry and (a - entry) % 4096 == 0}
        cands = [b for b in cands if any(b <= a < b + hi for a in uniq)]
        base = max(cands, key=hits)
    order, seen = [], set()
    for a in sbs:
        i = bisect.bisect_right(addrs, a - base) - 1
        if i >= 0 and syms[i][0] <= a - base < syms[i][0] + max(syms[i][1], 1):
            if syms[i][2] not in seen:
                seen.add(syms[i][2])
                order.append(syms[i][2])
    return order


def callgrind_counts(binary, runs):
    """(rows that ran each function, total self instructions of each function)."""
    nrows, ir = collections.Counter(), collections.Counter()

    def one(run):
        args, stdin = run
        with tempfile.NamedTemporaryFile(suffix=".cg") as f, open(stdin or "/dev/null") as si:
            subprocess.run(["valgrind", "--tool=callgrind", "--demangle=no", "--compress-strings=no",
                            "--compress-pos=no", f"--callgrind-out-file={f.name}", binary, "jq", *args],
                           stdin=si, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            here, fn, skip = collections.Counter(), None, False
            for line in open(f.name, errors="replace"):
                if line.startswith("fn="):
                    fn, skip = line[3:].strip(), False
                elif line.startswith("calls="):
                    skip = True
                elif fn and line[:1].isdigit():
                    if skip:
                        skip = False
                        continue
                    p = line.split()
                    if len(p) >= 2:
                        here[fn] += int(p[1])
            return here

    with cf.ThreadPoolExecutor(os.cpu_count() or 4) as ex:
        for here in ex.map(one, runs):
            for fn, n in here.items():
                nrows[fn] += 1
                ir[fn] += n
    return nrows, ir


def large_rows(inputs):
    """The rows of `rows(inputs, "10mb")` that read a 10 MB input: a row with none is the same run."""
    return [r for r in rows(inputs, "10mb") if any("-10mb." in a for a in r[0] + [r[1] or ""])]


def large_tail(binary, inputs, dem, have):
    """Demangled names of the functions the 10 MB rows run and `have` lacks, most rows first."""
    nrows, ir = callgrind_counts(binary, large_rows(inputs))
    tail = sorted((f for f in nrows if f in dem and norm(dem[f]) not in have), key=lambda f: (-nrows[f], -ir[f]))
    out, seen = [], set()
    for m in tail:
        if norm(dem[m]) not in seen:
            seen.add(norm(dem[m]))
            out.append(dem[m])
    return out


def linux(binary, inputs):
    syms = nm_pairs(binary)
    dem = {s[2]: s[3] for s in syms}
    start = []
    for args, stdin in STARTUP:
        for m in lackey_order(binary, args, stdin, syms):
            if m not in start:
                start.append(m)
    nrows, ir = callgrind_counts(binary, rows(inputs) + STARTUP)
    rest = sorted((f for f in nrows if f in dem and f not in start), key=lambda f: (-nrows[f], -ir[f]))
    out, seen = [], set()
    for block in (start, [None], rest):
        for m in block:
            if m is None:
                out.append(MARK)
            elif dem[m] not in seen:
                seen.add(dem[m])
                out.append(dem[m])
    return out + large_tail(binary, inputs, dem, {norm(n) for n in seen})


def extend(binary, inputs, listfile):
    """`listfile` as it is, then the functions only the 10 MB rows run (see `large_tail`)."""
    old = [l.rstrip("\n") for l in open(listfile)]
    dem = {s[2]: s[3] for s in nm_pairs(binary)}
    return old + large_tail(binary, inputs, dem, {norm(l) for l in old if not l.startswith("#")})


PLATFORM = re.compile(r"neon|aarch64|arm64|apple|darwin|macos", re.I)


def platform_names(demangled, have):
    """The names in `demangled` that look platform-specific and that `have` (normalized) lacks, sorted."""
    return sorted({d for d in demangled if PLATFORM.search(d) and norm(d) not in have})


def macos(binary, listfile):
    """`listfile` with every platform-named function of `binary` it lacks added at the end of the
    startup block. Every one, not the ones a run touches: see the module docstring (#7826)."""
    with open(listfile) as f:
        old = [l.rstrip("\n") for l in f]
    have = {norm(l) for l in old if not l.startswith("#")}
    add = platform_names([s[3] for s in nm_pairs(binary)], have)
    print(f"macos: added {len(add)} platform-named functions: every one {os.path.basename(binary)} defines "
          f"that the list lacks, called or not. This pass does not sample what runs (#7826).", file=sys.stderr)
    cut = old.index(MARK) if MARK in old else len(old)
    return old[:cut] + add + old[cut:]


def check(binary, listfile, limit):
    """Fail when a startup run executes more than `limit` bytes of functions the list does not name,
    or when the binary was not linked in the list's order (the functions it does name are spread
    over more than twice their size). Linux, valgrind."""
    have = {norm(l.rstrip("\n")) for l in open(listfile) if not l.startswith("#")} | set(CRT_STARTUP)
    syms = nm_pairs(binary)
    by = {s[2]: s for s in syms}
    missing, listed = {}, {}
    for args, stdin in STARTUP:
        for m in lackey_order(binary, args, stdin, syms):
            a, size, _, d = by[m]
            (listed if d in have else missing)[m] = (a, size, d)
    for _, size, d in sorted(missing.values(), key=lambda x: -x[1])[:20]:
        print(f"{size:8} {d}")
    lost = sum(v[1] for v in missing.values())
    print(f"startup functions missing from the list: {len(missing)}, {lost} bytes (limit {limit})")
    ok = lost <= limit
    if listed:
        # Ordered, the listed functions open the text (link.py puts the C runtime's `_start` and
        # crtbeginS.o's functions first, #7412); a few can sit elsewhere, so ask for 90% of the bytes.
        lo = min(v[0] for v in listed.values())
        total = sum(v[1] for v in listed.values())
        near = sum(v[1] for v in listed.values() if v[0] + v[1] <= lo + 2 * total + 65536)
        print(f"listed startup functions: {len(listed)}, {total} bytes, {near} of them in the first "
              f"{2 * total + 65536} bytes from the lowest")
        if near < 0.9 * total:
            print("the binary was not linked in the list's order (see <output>.link-order.log)")
            ok = False
    return ok


# The share of listed names that may stop resolving before `stale` fails, in percent. A fresh list is
# not at 0: on x86_64 Linux the names the macOS pass added never resolve, nor do the few names of
# functions that only another feature list has. #7894's history is the other end: 3.4% the day the
# list was last generated, 4.9% fourteen merges later with the first measurable rise in peak RSS (about
# 0.3 MB a small run on a Ryzen 9 7950X), 6.0% at 0.6 MB, 8.0% at 1.3 MB. So 5% is where the cost
# starts to show, and leaves room for about thirty renamed or removed functions after a regeneration.
STALE_PERCENT = 5.0
LOG_COUNTS = re.compile(r"(\d+) listed, (\d+) found, (\d+) symbols ordered")


def stale_share(log_text):
    """(listed, found, percent of listed names that did not resolve) from a link-order log, or None
    when the log says the link was plain."""
    m = LOG_COUNTS.search(log_text)
    if not m or int(m.group(1)) == 0:
        return None
    listed, found = int(m.group(1)), int(m.group(2))
    return listed, found, 100.0 * (listed - found) / listed


def newest_log(path):
    """`path` if it is a file, else the newest link-order log of the `succinctly` binary under the
    target profile directory `path` (cargo up to 1.99 links in `deps/`, 1.100 in `build/`)."""
    if os.path.isfile(path):
        return path
    logs = (glob.glob(os.path.join(path, "deps", "succinctly*.link-order.log"))
            + glob.glob(os.path.join(path, "build", "succinctly", "*", "out", "succinctly*.link-order.log")))
    return max(logs, key=os.path.getmtime) if logs else None


def stale(path, limit):
    """Fail when more than `limit` percent of the listed names did not resolve in the newest link."""
    log = newest_log(path)
    if log is None:
        print(f"no link-order log under {path}: the linker wrapper did not run for a release binary")
        return False
    with open(log) as f:
        text = f.read().strip()
    counts = stale_share(text)
    if counts is None:
        print(f"{log}: {text}\nthe link was not ordered, so the list could not be checked")
        return False
    listed, found, share = counts
    print(f"link-order list: {listed} listed, {found} found, {listed - found} no longer resolve "
          f"({share:.1f}%, limit {limit:g}%)")
    if share > limit:
        print("scripts/link-order/symbols.txt is stale: functions it names were renamed, split or removed, "
              "and their replacements sit outside the ordered block, where each keeps a page resident "
              "(64 KB or a large folio on Linux) in every run that calls it (#7894).\nRegenerate it: "
              "`generate.py linux` on x86_64 Linux, then `generate.py macos` on Apple silicon "
              "(docs/guides/release.md).")
        return False
    return True


def main(argv):
    if len(argv) >= 3 and argv[0] == "linux":
        print("\n".join(linux(argv[1], argv[2])))
    elif len(argv) >= 4 and argv[0] == "extend":
        print("\n".join(extend(argv[1], argv[2], argv[3])))
    elif len(argv) >= 3 and argv[0] == "macos":
        # `macos BINARY LIST`; the inputs directory the sampling pass took in between is ignored
        print("\n".join(macos(argv[1], argv[-1])))
    elif len(argv) >= 3 and argv[0] == "check":
        sys.exit(0 if check(argv[1], argv[2], int(argv[3]) if len(argv) > 3 else 65536) else 1)
    elif len(argv) >= 2 and argv[0] == "stale":
        sys.exit(0 if stale(argv[1], float(argv[2]) if len(argv) > 2 else STALE_PERCENT) else 1)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
