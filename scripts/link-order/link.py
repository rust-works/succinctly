#!/usr/bin/env python3
"""Link the `succinctly` executable with its startup and hot functions placed first (#7260).

`.cargo/config.toml` names `scripts/link-order/cc` as the linker for the targets the release
workflow ships from Linux and macOS; that script runs this one. It runs the real linker driver
(`cc`, or `$SUCCINCTLY_LINK_ORDER_CC`) with the arguments rustc gave it, plus one option that
orders the executable's functions by `symbols.txt` in this directory:

- Linux: lld's `--symbol-ordering-file`. rustc's own `-fuse-ld=lld` is used when it passes one;
  otherwise the wrapper adds rustup's lld (`gcc-ld/ld.lld`) itself. GNU ld has no such option, so
  a link where neither is available, or where rustc chose another `-fuse-ld`, gets no ordering.
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

It steps aside (plain link) when: the output is not the `succinctly` executable (cargo's
`deps/succinctly-<hash>` or, from cargo 1.100, `build/succinctly/<hash>/out/succinctly`) of a
non-debug profile, the link makes a shared library, `SUCCINCTLY_LINK_ORDER=0`, the platform is
neither Linux nor macOS, or anything in the resolution step fails. For the release binary the
resolved file and a one-line summary (or why it linked plain) are left next to the output as
`<output>.link-order.txt` and `<output>.link-order.log`.

Measurement builds only (#7960; docs/guides/benchmarking.md, "Layout-seeded A/B"). Three variables
let an A/B vary the code layout and nothing else; none is ever set for a shipped build:

- `SUCCINCTLY_LINK_LAYOUT_SEED=N` perturbs the layout by seed `N`, so two binaries linked with the
  same seed are placed alike and N seeds give N layouts. `SUCCINCTLY_LINK_LAYOUT_MODE` picks how:
  `pad` (lld's `--randomize-section-padding=N`, which keeps the order; the Linux default) and/or
  `shuffle` (a seeded permutation of the ordered block, which stays first; the only one ld64 has).
  The link log names the seed and modes; a bad value fails the ordering, and the log says so.
- `SUCCINCTLY_LINK_ORDER_LIST=PATH` orders by another tree's list, so an older commit's objects
  link with that commit's own `symbols.txt` through this wrapper.
- `SUCCINCTLY_LINK_SAVE_DIR=DIR` keeps the link: the objects and temporary files rustc deletes
  afterwards, and the arguments. `relink.py` links them again under other seeds in seconds.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# `SUCCINCTLY_LINK_ORDER_LIST`: another tree's list (measurement builds, see above).
LIST = os.environ.get("SUCCINCTLY_LINK_ORDER_LIST") or os.path.join(HERE, "symbols.txt")


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


def is_release_binary(args, env=os.environ):
    """Whether this link makes the `succinctly` executable of a non-debug profile."""
    if env.get("SUCCINCTLY_LINK_ORDER", "1") == "0":
        return False
    if any(a in ("-shared", "-dynamiclib", "-r") for a in args):
        return False
    out = output_of(args)
    # Cargo up to 1.99 links `target/<profile>/deps/succinctly-<hash>`; 1.100 (its build-dir
    # layout) links `target/<profile>/build/succinctly/<hash>/out/succinctly` and copies it up.
    if not out or not re.fullmatch(r"succinctly(-[0-9a-f]+)?", os.path.basename(out)):
        return False
    parts = os.path.abspath(out).split(os.sep)
    return not any(p == "debug" and q in ("deps", "build") for p, q in zip(parts, parts[1:]))


def rust_lld_dir(args):
    """rustup's `gcc-ld` directory (an `ld.lld` that runs `rust-lld`), found from the `-L
    <sysroot>/lib/rustlib/<target>/lib` rustc passes, or None."""
    for i, a in enumerate(args):
        path = args[i + 1] if a == "-L" and i + 1 < len(args) else a[2:] if a.startswith("-L") else None
        if path and os.path.basename(os.path.dirname(os.path.dirname(path))) == "rustlib":
            d = os.path.join(os.path.dirname(path), "bin", "gcc-ld")
            if os.path.isfile(os.path.join(d, "ld.lld")):
                return d
    return None


def wants_order(args, platform=sys.platform, env=os.environ):
    """(ordering option, other driver arguments to add, why) for this link; option None = plain.

    On Linux the ordering option needs lld. rustc links x86_64 with its own lld (`-fuse-ld=lld`)
    by default since 1.90; a link that reaches the wrapper without any `-fuse-ld` gets rustup's lld
    added here instead, when the toolchain ships it.
    """
    if not is_release_binary(args, env):
        return None, [], "not the release binary"
    if platform == "darwin":
        return "-Wl,-order_file,{}", [], "ld64"
    if not platform.startswith("linux"):
        return None, [], f"no ordering option on {platform}"
    if "-fuse-ld=lld" in args:
        return "-Wl,--symbol-ordering-file={}", [], "rustc links with lld"
    if any(a.startswith("-fuse-ld=") for a in args):
        return None, [], "rustc chose another linker: " + next(a for a in args if a.startswith("-fuse-ld="))
    d = rust_lld_dir(args)
    if d:
        return "-Wl,--symbol-ordering-file={}", ["-B" + d, "-fuse-ld=lld"], "added rustup's lld"
    return None, [], "no lld: GNU ld has no --symbol-ordering-file"


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


# The C runtime's own start-up and exit code, which the `cc` driver links in from `Scrt1.o` and
# `crtbeginS.o`: `_start` runs first in every process, `frame_dummy` from `.init_array` and
# `__do_global_dtors_aux` at exit (#7412). No Rust object defines them, so they cannot be looked up
# like the rest of the list, and lld is given them by name. Left unordered they sit wherever the
# driver's objects fall, 2 MB past the ordered block, and the page they share kept a large folio of
# text resident in every run: 256 KB of `succinctly jq -n 1`'s RSS on a Ryzen 9 7950X (Linux 6.18).
CRT_STARTUP = ["_start", "frame_dummy", "register_tm_clones", "deregister_tm_clones",
               "__do_global_dtors_aux"]


def resolve(names, table, platform=sys.platform):
    out, seen, missing = [], set(), 0
    if platform.startswith("linux"):
        # Before the Rust functions: `_start` is the first code a process runs.
        out, seen = list(CRT_STARTUP), set(CRT_STARTUP)
        names = [n for n in names if n not in seen]
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


def layout_seed(env=os.environ):
    """The layout seed of a measurement build (`SUCCINCTLY_LINK_LAYOUT_SEED`), or None.

    The same seed given to a baseline and a candidate link places both alike, and N seeds give N
    layouts: a difference that holds over the seeds is the change's, one that changes sign
    between them is placement (scripts/layout-ab.py). A seed that is not a non-negative integer
    raises ValueError, which `main` reports as a plain link."""
    s = env.get("SUCCINCTLY_LINK_LAYOUT_SEED", "").strip()
    if not s:
        return None
    if not s.isdigit():
        raise ValueError(f"SUCCINCTLY_LINK_LAYOUT_SEED={s!r} is not a non-negative integer")
    return int(s)


def layout_modes(platform=sys.platform, env=os.environ):
    """Which perturbations a seeded link applies: `pad` (lld's `--randomize-section-padding`,
    which keeps the order) and/or `shuffle` (a permutation of the ordered block, which stays
    first), from `SUCCINCTLY_LINK_LAYOUT_MODE` (`pad`, `shuffle`, or both joined by `,` or `+`).
    The default is `pad` on Linux and `shuffle` on macOS, where ld64 has no padding option, so
    asking for `pad` there raises ValueError rather than linking unperturbed."""
    default = "shuffle" if platform == "darwin" else "pad"
    modes = {m for m in env.get("SUCCINCTLY_LINK_LAYOUT_MODE", default).replace("+", ",").split(",") if m}
    unknown = modes - {"pad", "shuffle"}
    if unknown or not modes:
        raise ValueError(f"SUCCINCTLY_LINK_LAYOUT_MODE: expected pad and/or shuffle, got {sorted(unknown) or 'nothing'}")
    if platform == "darwin" and "pad" in modes:
        raise ValueError("SUCCINCTLY_LINK_LAYOUT_MODE=pad: ld64 has no padding option; use shuffle")
    return modes


def shuffled(order, seed, platform=sys.platform):
    """`order` with everything after the C runtime start-up block permuted by `seed`: the
    ordered functions stay first, so the pages a run touches stay together, and only their
    places within the block change."""
    import random
    k = len(CRT_STARTUP) if platform.startswith("linux") else 0
    head, rest = order[:k], order[k:]
    random.Random(seed).shuffle(rest)
    return head + rest


def save_link(args, save_dir):
    """Copy the link's inputs that rustc deletes afterwards (its objects, and the files in its
    temporary directory, including those named inside a `-Wl,` argument) into `save_dir`, and
    write the arguments, the working directory and the list there as `link.json`, so relink.py
    can link the same objects again under other seeds without rebuilding."""
    import json
    import shutil
    import tempfile
    os.makedirs(save_dir, exist_ok=True)
    tmp = os.path.realpath(tempfile.gettempdir())
    def keep(i, a):
        p = os.path.realpath(a) if os.path.isfile(a) else None
        if p and (a.endswith(".o") or p.startswith(tmp) or "/rustc" in p):
            dst = os.path.join(save_dir, f"{i:04d}-{os.path.basename(a)}")
            shutil.copyfile(a, dst)
            return dst
        return a

    out_args = []
    for i, a in enumerate(args):
        if a.startswith("-Wl,"):  # `-Wl,-exported_symbols_list,<tmp file>` and the like
            out_args.append(",".join(keep(i, part) for part in a.split(",")))
        else:
            out_args.append(keep(i, a))
    with open(os.path.join(save_dir, "link.json"), "w") as f:
        json.dump({"args": out_args, "cwd": os.getcwd(), "list": LIST}, f, indent=0)


def expand(args):
    """`args` with each `@file` (rustc writes one when the command line is long) read in: one
    argument per line, with `\\` before a backslash or a space."""
    out = []
    for a in args:
        if a.startswith("@") and os.path.isfile(a[1:]):
            with open(a[1:]) as f:
                out += [re.sub(r"\\(.)", r"\1", line.rstrip("\n")) for line in f if line.strip()]
        else:
            out.append(a)
    return out


def main(argv):
    args = expand(argv)
    opt, driver, why = wants_order(args)
    extra = []
    trace = os.environ.get("SUCCINCTLY_LINK_ORDER_TRACE")
    if trace:  # every link the wrapper sees, for a build that came out unordered (CI sets it)
        try:
            with open(trace, "a") as f:
                fuse = [a for a in args if a.startswith(("-fuse-ld", "-B", "@"))]
                f.write(f"{output_of(args)}: {why}; {fuse}\n")
        except OSError:
            pass
    if is_release_binary(args):
        out = output_of(args)
        msg = f"plain link: {why}\n"
        if opt and os.path.isfile(LIST):
            try:
                files = objects(args)
                mangled = text_symbols(files)
                table = demangled_to_mangled(mangled, demangle(mangled))
                names = read_list(LIST)
                order, missing = resolve(names, table)
                seed = layout_seed()
                modes = layout_modes() if seed is not None else set()
                if "shuffle" in modes:
                    order = shuffled(order, seed)
                path = out + ".link-order.txt"
                with open(path, "w") as f:
                    f.write("".join(m + "\n" for m in order))
                extra = driver + [opt.format(path)]
                if opt.startswith("-Wl,--symbol"):
                    extra.append("-Wl,--no-warn-symbol-ordering")
                    if "pad" in modes:
                        extra.append(f"-Wl,--randomize-section-padding={seed}")
                msg = (f"{len(names)} listed, {len(names) - missing} found, {len(order)} symbols "
                       f"ordered ({why})\n")
                if seed is not None:
                    msg += f"layout seed {seed}, modes {sorted(modes)}\n"
            except (OSError, ValueError, subprocess.CalledProcessError) as e:
                extra, msg = [], f"plain link: {e}\n"
        try:
            with open(out + ".link-order.log", "w") as f:
                f.write(msg)
        except OSError:
            pass
        save = os.environ.get("SUCCINCTLY_LINK_SAVE_DIR")
        if save:
            save_link(args, save)
    os.execvp(real_cc(), [real_cc()] + argv + extra)


if __name__ == "__main__":
    main(sys.argv[1:])
