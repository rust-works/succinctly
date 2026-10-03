#!/usr/bin/env python3
"""Name the functions behind llvm-cov's `warning: N functions have mismatched data`.

`llvm-cov` prints only the count. This finds the functions by reading each
instrumented binary's coverage mapping twice: once against the merged profile
of a coverage run, once against a profile that has no record of any of its
functions. A function with no profile record is kept (with zero counts); one
whose record has a different hash or counter count is dropped and counted as
mismatched. So per binary, the functions present with the empty profile and
absent with the merged one are exactly the mismatched ones (#3672).

The comparison has to be per binary: llvm-cov merges identical functions across
the objects of one invocation, so a function that mismatches in one binary but
matches in another disappears from the combined diff.

Run a coverage pass first, with the arguments CI uses, then this script with
the same feature arguments:

    cargo llvm-cov --no-report --features cli,simd,regex,serde --workspace
    scripts/coverage-mismatched-functions.py --features cli,simd,regex,serde

Needs `cargo-llvm-cov`, the `llvm-tools` rustup component, and `rustc`.
Standard library only.
"""

import argparse
import collections
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile


def run(cmd, **kwargs):
    return subprocess.run(cmd, capture_output=True, text=True, **kwargs)


def llvm_tool_dir():
    """The `bin` directory `cargo llvm-cov` itself uses for llvm-cov/llvm-profdata."""
    sysroot = run(["rustc", "--print", "sysroot"]).stdout.strip()
    host = next(
        line.split(": ", 1)[1]
        for line in run(["rustc", "-vV"]).stdout.splitlines()
        if line.startswith("host: ")
    )
    return os.path.join(sysroot, "lib", "rustlib", host, "bin")


def report_invocation(args):
    """The exact `llvm-cov report` command cargo-llvm-cov runs: objects, profile, ignore regex."""
    cmd = ["cargo", "llvm-cov", "report", "--summary-only", "-v"]
    out = run(cmd)
    for line in out.stderr.splitlines():
        if "llvm-cov report " in line and "-object" in line:
            return shlex.split(line.split("`")[1])
    sys.exit("could not find cargo-llvm-cov's llvm-cov invocation; run a coverage pass first")


def parse_invocation(argv):
    objects, profile, ignore = [], None, None
    i = 1
    while i < len(argv):
        a = argv[i]
        if a == "-object":
            objects.append(argv[i + 1])
            i += 2
        elif a.startswith("-instr-profile="):
            profile = a.split("=", 1)[1]
            i += 1
        elif a == "-ignore-filename-regex":
            ignore = argv[i + 1]
            i += 2
        else:
            i += 1
    return objects, profile, ignore


def empty_profile(tools, tmp):
    """A valid profile that names none of the functions under test: a different program's."""
    src = os.path.join(tmp, "d.rs")
    with open(src, "w") as f:
        f.write('fn main() { println!("{}", std::env::args().count()); }\n')
    exe = os.path.join(tmp, "d")
    subprocess.run(["rustc", "-C", "instrument-coverage", src, "-o", exe], check=True,
                   capture_output=True)
    raw = os.path.join(tmp, "d.profraw")
    subprocess.run([exe], check=True, capture_output=True, env={**os.environ, "LLVM_PROFILE_FILE": raw})
    out = os.path.join(tmp, "empty.profdata")
    subprocess.run([os.path.join(tools, "llvm-profdata"), "merge", "-o", out, raw], check=True)
    return out


def functions(tools, obj, profile, ignore):
    out = run([os.path.join(tools, "llvm-cov"), "export", "-format=text", "-skip-expansions",
               f"-ignore-filename-regex={ignore}", f"-instr-profile={profile}", obj])
    data = json.loads(out.stdout)["data"][0]["functions"]
    return collections.Counter((f["name"], (f["filenames"] or [""])[0]) for f in data)


def mismatch_count(tools, obj, profile, ignore):
    out = run([os.path.join(tools, "llvm-cov"), "export", "-format=text", "-summary-only",
               f"-ignore-filename-regex={ignore}", f"-instr-profile={profile}", obj])
    m = re.search(r"(\d+) functions have mismatched data", out.stderr)
    return int(m.group(1)) if m else 0


def demangle(name):
    """Best-effort path from a v0 Rust symbol: the length-prefixed identifiers, in order."""
    # Drop the crate disambiguators (`Cs2mXM1HRIz6G_`) so their base-62 digits
    # are not read as a length prefix.
    name = re.sub(r"Cs[0-9A-Za-z]*_", "C", name)
    parts, i = [], 0
    while i < len(name):
        m = re.match(r"(\d+)", name[i:])
        if not m:
            i += 1
            continue
        n = int(m.group(1))
        start = i + len(m.group(1))
        ident = name[start:start + n]
        if len(ident) == n and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", ident):
            parts.append(ident)
            i = start + n
        else:
            i += 1
    return "::".join(parts) if parts else name


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--features", help="accepted for symmetry with the coverage run; unused")
    args = ap.parse_args()
    tools = llvm_tool_dir()
    objects, profile, ignore = parse_invocation(report_invocation(args))
    if not objects or not profile:
        sys.exit("no objects or profile found; run `cargo llvm-cov --no-report ...` first")
    ignore = ignore or ""
    with tempfile.TemporaryDirectory() as tmp:
        empty = empty_profile(tools, tmp)
        per_binary = {}
        total = 0
        for obj in objects:
            n = mismatch_count(tools, obj, profile, ignore)
            total += n
            if n:
                per_binary[obj] = n
        print(f"{total} mismatched in {len(per_binary)} of {len(objects)} binaries")
        names = collections.Counter()
        for obj in per_binary:
            for key, count in (functions(tools, obj, empty, ignore)
                               - functions(tools, obj, profile, ignore)).items():
                names[key] += count
        for (name, filename), count in names.most_common():
            rel = os.path.relpath(filename) if filename else "?"
            print(f"{count:4d} binaries  {rel}  {demangle(name)}")
        if sum(names.values()) != total:
            print(f"note: named {sum(names.values())} of {total}", file=sys.stderr)


if __name__ == "__main__":
    main()
