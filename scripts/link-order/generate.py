#!/usr/bin/env python3
"""Regenerate `symbols.txt`, the function order `link.py` gives the release binary (#7260).

    # Linux x86_64 with valgrind: the whole list, from a release binary built from this tree
    python3 scripts/link-order/generate.py linux target/release/succinctly data/compare/inputs > symbols.txt
    # macOS: add the arm64/macOS-only functions the Linux trace cannot see
    python3 scripts/link-order/generate.py macos target/release/succinctly data/compare/inputs symbols.txt > new.txt

`linux` runs every row of `comparison/jq/queries/perf.tsv` at 1 MB (and two startup-only runs)
under valgrind. The startup runs (`jq -n 1`, `jq .` on empty input) give the first block, in the
order their functions first execute (lackey superblock trace); then comes every other function
callgrind saw execute in any row, those run by the most rows first, then by instructions. Last come
the functions only the rows at 10 MB run, ranked the same way (#7412): a file of 4 MiB or more takes
the windowed routes (`jq_runner/window.rs`), which no 1 MB row reaches, and left unordered their
two dozen functions kept three or four large folios of text resident in every stream row.

    # Add that 10 MB tail to an existing list without reordering it
    python3 scripts/link-order/generate.py extend target/release/succinctly data/compare/inputs symbols.txt > new.txt

`macos` cannot trace (no valgrind, and lldb needs a debugging grant), so it samples instead: each
run gets a fresh copy of the binary, whose pages are then in no cache, and `mincore()` at exit
says which 16 KB pages of `__TEXT` it touched. Functions on touched pages whose names look
platform-specific (NEON, aarch64, Apple) and are missing from the list are added: those after the
startup marker if only the full rows touched them, before it if a startup run did.

The list is demangled names (see link.py); a name that later stops existing is skipped at link time,
so a stale list degrades gradually: functions new to the startup path land outside the hot block.
`check BINARY symbols.txt [LIMIT]` (ci.yml's perf-guard job, x86_64) fails when a startup run executes
more than LIMIT bytes (64 KB) of functions the list misses, or when BINARY was not linked in its order.
"""
import bisect
import collections
import concurrent.futures as cf
import json
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


MINCORE_C = r"""
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <mach-o/dyld.h>
#include <mach-o/loader.h>
#include <unistd.h>
__attribute__((destructor)) static void fin(void) {
  const char *out = getenv("LINK_ORDER_MINCORE"); if (!out) return;
  const struct mach_header_64 *h = 0; intptr_t slide = 0;
  for (uint32_t k = 0; k < _dyld_image_count(); k++) {  /* image 0 can be this inserted library */
    const struct mach_header_64 *c = (const void *)_dyld_get_image_header(k);
    if (c->filetype == MH_EXECUTE) { h = c; slide = _dyld_get_image_vmaddr_slide(k); break; }
  }
  if (!h) return;
  const struct load_command *lc = (const void *)(h + 1);
  for (unsigned i = 0; i < h->ncmds; i++, lc = (const void *)((const char *)lc + lc->cmdsize)) {
    const struct segment_command_64 *s = (const void *)lc;
    if (lc->cmd != LC_SEGMENT_64 || __builtin_strcmp(s->segname, "__TEXT")) continue;
    long ps = getpagesize(); size_t np = s->vmsize / ps; char *v = malloc(np);
    if (mincore((void *)(s->vmaddr + slide), s->vmsize, v)) return;
    FILE *o = fopen(out, "w"); fprintf(o, "%lx %ld ", (unsigned long)s->vmaddr, ps);
    for (size_t j = 0; j < np; j++) fputc(v[j] & 1 ? '1' : '0', o);
    fclose(o);
  }
}
"""

PLATFORM = re.compile(r"neon|aarch64|arm64|apple|darwin|macos", re.I)


def macos(binary, inputs, listfile):
    old = [l.rstrip("\n") for l in open(listfile)]
    have = {norm(l) for l in old if not l.startswith("#")}
    syms = nm_pairs(binary)
    with tempfile.TemporaryDirectory() as td:
        lib = os.path.join(td, "mincore.dylib")
        src = os.path.join(td, "mincore.c")
        open(src, "w").write(MINCORE_C)
        subprocess.run(["cc", "-O2", "-dynamiclib", "-o", lib, src], check=True)

        def touched(run):
            args, stdin = run
            copy, rep = os.path.join(td, f"b{os.getpid()}"), os.path.join(td, "pages")
            # A clone (APFS `cp -c`) shares the blocks but none of the cached pages, so what is
            # resident at exit is what this run touched. A byte copy would leave every page cached.
            subprocess.run(["cp", "-c", binary, copy], check=True)
            env = dict(os.environ, DYLD_INSERT_LIBRARIES=lib, LINK_ORDER_MINCORE=rep)
            with open(stdin or "/dev/null") as si:
                subprocess.run([copy, "jq", *args], stdin=si, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, env=env)
            vm, ps, bits = open(rep).read().split()
            os.unlink(copy)
            vm, ps = int(vm, 16), int(ps)
            return {vm + i * ps for i, b in enumerate(bits) if b == "1"}, ps

        def names(pages, ps):
            return {s[3] for s in syms if PLATFORM.search(s[3]) and s[3] not in have
                    and any((p <= s[0] < p + ps) or (s[0] <= p < s[0] + max(s[1], 1)) for p in pages)}

        start = set()
        for run in STARTUP:
            start |= names(*touched(run))
        rest = set()
        for run in rows(inputs):
            rest |= names(*touched(run))
    rest -= start
    cut = old.index(MARK) if MARK in old else len(old)
    return old[:cut] + sorted(start) + old[cut:cut + 1] + old[cut + 1:] + sorted(rest)


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


def main(argv):
    if len(argv) >= 3 and argv[0] == "linux":
        print("\n".join(linux(argv[1], argv[2])))
    elif len(argv) >= 4 and argv[0] == "extend":
        print("\n".join(extend(argv[1], argv[2], argv[3])))
    elif len(argv) >= 4 and argv[0] == "macos":
        print("\n".join(macos(argv[1], argv[2], argv[3])))
    elif len(argv) >= 3 and argv[0] == "check":
        sys.exit(0 if check(argv[1], argv[2], int(argv[3]) if len(argv) > 3 else 65536) else 1)
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
