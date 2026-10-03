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

Run a coverage pass first, then this script from the same checkout with no
arguments (it reads the objects, profile and ignore regex back from
`cargo llvm-cov report`, so it follows whatever the pass was run with):

    cargo llvm-cov --no-report --features cli,simd,regex,serde --workspace
    scripts/coverage-mismatched-functions.py

Needs `cargo-llvm-cov`, the `llvm-tools` rustup component, and `rustc`.
`LLVM_COV` and `LLVM_PROFDATA` are honoured, as cargo-llvm-cov honours them.
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


def llvm_tool(name):
    """llvm-cov / llvm-profdata: the environment override cargo-llvm-cov honours,
    else the rustup `llvm-tools` copy it falls back to."""
    override = os.environ.get(name.upper().replace("-", "_"))
    if override:
        return override
    sysroot = run(["rustc", "--print", "sysroot"]).stdout.strip()
    host = next(
        line.split(": ", 1)[1]
        for line in run(["rustc", "-vV"]).stdout.splitlines()
        if line.startswith("host: ")
    )
    return os.path.join(sysroot, "lib", "rustlib", host, "bin", name)


def report_invocation():
    """The exact `llvm-cov report` command cargo-llvm-cov runs: objects, profile, ignore regex.

    Read back from its verbose output rather than rebuilt here, so the object list
    cannot drift from what the real report uses."""
    out = run(["cargo", "llvm-cov", "report", "--summary-only", "-v"])
    for line in out.stderr.splitlines():
        m = re.search(r"Running `(.*llvm-cov report .*-object.*)`", line)
        if m:
            return shlex.split(m.group(1))
    sys.exit("could not find cargo-llvm-cov's llvm-cov invocation in its -v output; "
             "run `cargo llvm-cov --no-report ...` first.\n" + out.stderr[-500:])


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


def empty_profile(tmp):
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
    subprocess.run([llvm_tool("llvm-profdata"), "merge", "-o", out, raw], check=True)
    return out


def export(obj, profile, ignore, *flags):
    cmd = [llvm_tool("llvm-cov"), "export", "-format=text", *flags, f"-instr-profile={profile}"]
    if ignore:
        cmd.append(f"-ignore-filename-regex={ignore}")
    out = run(cmd + [obj])
    # A binary whose every function the ignore regex filters out (an integration test
    # with no library code of its own) has nothing to export; that is zero functions,
    # not a failure.
    if out.returncode != 0 and "no coverage data found" in out.stderr:
        out.stdout = '{"data": [{"functions": []}]}'
    elif out.returncode != 0:
        sys.exit(f"llvm-cov export failed on {obj} (exit {out.returncode}):\n{out.stderr[-500:]}")
    return out


def functions(obj, profile, ignore):
    data = json.loads(export(obj, profile, ignore, "-skip-expansions").stdout)["data"][0]["functions"]
    return collections.Counter((f["name"], (f["filenames"] or [""])[0]) for f in data)


def mismatch_count(obj, profile, ignore):
    m = re.search(r"(\d+) functions have mismatched data",
                  export(obj, profile, ignore, "-summary-only").stderr)
    return int(m.group(1)) if m else 0


def demangle(name):
    """Best-effort path from a v0 Rust symbol: its length-prefixed identifiers, in order.

    Walks the symbol left to right, consuming each identifier whole (so nothing
    inside one is mistaken for syntax) and each crate disambiguator `s<base62>_`."""
    parts, i = [], 0
    while i < len(name):
        if name[i] == "C" and name[i + 1:i + 2] == "s":
            end = name.find("_", i + 2)
            i = end + 1 if end != -1 else len(name)
            continue
        m = re.match(r"\d+", name[i:])
        if m:
            n = int(m.group())
            start = i + len(m.group())
            ident = name[start:start + n]
            if len(ident) == n and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", ident):
                parts.append(ident)
                i = start + n
                continue
        i += 1
    return "::".join(parts) if parts else name


def main():
    argparse.ArgumentParser(description=__doc__.split("\n\n")[0]).parse_args()
    objects, profile, ignore = parse_invocation(report_invocation())
    if not objects or not profile:
        sys.exit("no objects or profile found; run `cargo llvm-cov --no-report ...` first")
    with tempfile.TemporaryDirectory() as tmp:
        empty = empty_profile(tmp)
        # The premise: against a profile that names none of the functions, nothing mismatches.
        if mismatch_count(objects[0], empty, ignore):
            sys.exit("the stand-in empty profile reports mismatches; the diff would be unsound")
        per_binary = {}
        for n, obj in enumerate(objects, 1):
            print(f"\rcounting {n}/{len(objects)}", end="", file=sys.stderr, flush=True)
            count = mismatch_count(obj, profile, ignore)
            if count:
                per_binary[obj] = count
        print(file=sys.stderr)
        total = sum(per_binary.values())
        print(f"{total} mismatched in {len(per_binary)} of {len(objects)} binaries")
        names = collections.Counter()
        for n, obj in enumerate(per_binary, 1):
            print(f"\rnaming {n}/{len(per_binary)}", end="", file=sys.stderr, flush=True)
            names.update(functions(obj, empty, ignore) - functions(obj, profile, ignore))
        print(file=sys.stderr)
        for (name, filename), count in names.most_common():
            rel = os.path.relpath(filename) if filename else "?"
            print(f"{count:4d} binaries  {rel}  {demangle(name)}")
        if sum(names.values()) != total:
            print(f"note: named {sum(names.values())} of {total}", file=sys.stderr)


if __name__ == "__main__":
    main()
