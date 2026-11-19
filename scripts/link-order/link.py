#!/usr/bin/env python3
"""Link the `succinctly` executable with its startup and hot functions placed first (#7260).

`.cargo/config.toml` names `scripts/link-order/cc` as the linker for the targets the release
workflow ships from Linux and macOS; that script runs this one. It runs the real linker driver
(`cc`, or `$SUCCINCTLY_LINK_ORDER_CC`) with the arguments rustc gave it, plus one option that
orders the executable's functions by `symbols.txt` in this directory:

- Linux, when rustc links with lld (`-fuse-ld=lld`, the x86_64 default since Rust 1.90):
  `--symbol-ordering-file`. GNU ld has no such option, so another linker gets no ordering.
- macOS: ld64's `-order_file`.

Why: the release build is one codegen unit with fat LTO, so the 1-2 MB of functions a query runs
are scattered over 12-15 MB of text, and every page a process touches is resident. On Linux each
file-backed fault maps the 64 KB around it (or the whole large folio), so `succinctly jq -n 1`
had 4.5-5 MB of its own text resident against 0.3 MB for jq 1.7.1. Ordering brings it to
~1.2 MB on a Ryzen 9 7950X and from 1.5 MB to 0.8 MB of `__TEXT` on an Apple M4 Pro.

`symbols.txt` holds demangled names, which carry no crate hash: the mangled names change with the
crate version, the target triple and the toolchain, so the list is resolved here, at link time,
against the symbols of the objects being linked (`nm`, then `c++filt`). A name the objects do not
define is skipped. `generate.py` regenerates the list.

It steps aside (plain link) when: the output is not a `succinctly-*` file under a non-debug
profile directory, the link makes a shared library, `SUCCINCTLY_LINK_ORDER=0`, the platform is
neither Linux nor macOS, or anything in the resolution step fails. The resolved file and a one-line
summary are left next to the output as `<output>.link-order.txt` and `<output>.link-order.log`.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LIST = os.path.join(HERE, "symbols.txt")


def real_cc():
    return os.environ.get("SUCCINCTLY_LINK_ORDER_CC", "cc")


def norm(name):
    """One spelling for a demangled name, whichever demangler printed it.

    GNU c++filt prints crate hashes (`core[a9090a07a7fa7517]::fmt`) and the type of a const
    generic argument (`FatAVX2<1: usize>`); LLVM's prints neither. An LTO suffix on the mangled
    name (`.llvm.123`, `.specialized.4`) is cut before demangling, and a printed one is dropped.
    """
    name = re.sub(r" \(\..*\)$", "", name)
    name = re.sub(r"(?<=[A-Za-z0-9_])\[[0-9a-f]+\]", "", name)
    return re.sub(r": (?:[iu](?:8|16|32|64|128|size)|bool|char)(?=[,>])", "", name)


def demangle(mangled, platform=sys.platform):
    """Normalized demangled names for `mangled`, one each, in order (via `c++filt`)."""
    keys = [m.split(".", 1)[0] for m in mangled]
    if platform == "darwin":
        keys = [k[1:] if k.startswith("_") else k for k in keys]
    out = subprocess.run(["c++filt"], input="".join(k + "\n" for k in keys), capture_output=True,
                         text=True, check=True).stdout.splitlines()
    if len(out) != len(keys):
        raise ValueError("c++filt returned a different number of names")
    return [norm(d) for d in out]


def output_of(args):
    for i, a in enumerate(args):
        if a == "-o" and i + 1 < len(args):
            return args[i + 1]
    return None


def wants_order(args, platform=sys.platform, env=os.environ):
    """The option to add for this link, or None for a plain link."""
    if env.get("SUCCINCTLY_LINK_ORDER", "1") == "0":
        return None
    if any(a in ("-shared", "-dynamiclib", "-r") or a.startswith("@") for a in args):
        return None
    out = output_of(args)
    if not out or not os.path.basename(out).startswith("succinctly-"):
        return None
    if os.path.basename(os.path.dirname(os.path.dirname(os.path.abspath(out)))) == "debug":
        return None
    if platform == "darwin":
        return "-Wl,-order_file,{}"
    if platform.startswith("linux") and "-fuse-ld=lld" in args:
        return "-Wl,--symbol-ordering-file={}"
    return None


def objects(args):
    # With fat LTO every Rust function of the executable is in the one object rustc made.
    return [a for a in args if a.endswith(".o") and os.path.isfile(a)]


def text_symbols(files):
    """Names of the text symbols `files` define (`nm -p`, in file order)."""
    out = subprocess.run(["nm", "-p"] + files, capture_output=True, text=True, check=True).stdout
    names = []
    for line in out.splitlines():
        p = line.split(" ", 2)
        if len(p) == 3 and p[1] in "tT":
            names.append(p[2])
    return names


def demangled_to_mangled(mangled, demangled):
    """Map each normalized demangled name to the mangled names it stands for."""
    table = {}
    for m, d in zip(mangled, demangled):
        table.setdefault(d, []).append(m)
    return table


def read_list(path):
    with open(path) as f:
        return [line.rstrip("\n") for line in f if line.strip() and not line.startswith("#")]


def resolve(names, table, platform=sys.platform):
    out, seen, missing = [], set(), 0
    for n in names:
        ms = table.get(norm(n))
        if not ms:
            missing += 1
            continue
        for m in ms:
            if m not in seen:
                seen.add(m)
                out.append(m)
    return out, missing


def main(args):
    opt = wants_order(args)
    extra = []
    if opt and os.path.isfile(LIST):
        out = output_of(args)
        log = out + ".link-order.log"
        try:
            files = objects(args)
            mangled = text_symbols(files)
            table = demangled_to_mangled(mangled, demangle(mangled))
            names = read_list(LIST)
            order, missing = resolve(names, table)
            path = out + ".link-order.txt"
            with open(path, "w") as f:
                f.write("".join(m + "\n" for m in order))
            extra = [opt.format(path)]
            if opt.startswith("-Wl,--symbol"):
                extra.append("-Wl,--no-warn-symbol-ordering")
            msg = f"{len(names)} listed, {len(names) - missing} found, {len(order)} symbols ordered\n"
        except (OSError, ValueError, subprocess.CalledProcessError) as e:
            extra, msg = [], f"no ordering: {e}\n"
        try:
            with open(log, "w") as f:
                f.write(msg)
        except OSError:
            pass
    os.execvp(real_cc(), [real_cc()] + args + extra)


if __name__ == "__main__":
    main(sys.argv[1:])
