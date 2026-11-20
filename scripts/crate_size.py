#!/usr/bin/env python3
"""Package the crate and hold the .crate under a ceiling well below crates.io's upload limit.

    python3 scripts/crate_size.py check [--verify] [--features cli] [--crate PATH]

crates.io refuses a .crate over 10 MiB (10,485,760 bytes). `release.yml` runs `cargo publish`
last, after the GitHub release and the binaries exist, so a crate over the limit leaves a release
half done. v0.8.0's crate was 10,434,790 bytes, 51 KB under the limit, until `tests/` and
`benches/` left `include` in `Cargo.toml`; the library and the CLI alone are about 6.2 MiB, and
`src/jq` keeps growing.

`check` runs `cargo package` into `<target>/crate-size` (`--no-verify` unless `--verify` is given;
`--verify` builds the packaged crate with `--features`, `cli` by default, so a `src/` file that
`include_str!`s something outside `include` outside a `#[cfg(test)]` module fails here, not at
`cargo publish`), or measures `--crate` as given, and compares the .crate's size with CEILING:

- over the ceiling fails, printing the size and both margins. Make the crate smaller (a file that
  does not need to ship leaves `include`), or raise CEILING in this file with the reason, never to
  within 1 MiB of LIMIT;
- otherwise it prints the size and the headroom left to the ceiling and to the limit.

CI runs `check --verify --features cli` (`[[bin]]` needs `cli`, so without it only the library
is built) in the `Crate package` job (`.github/workflows/ci.yml`) and in
`release.yml` before the GitHub release is created; the pre-push `crate-size` gate runs the same.
"""

import argparse
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LIMIT = 10 * 1024 * 1024  # crates.io's maximum upload size
CEILING = 8 * 1024 * 1024  # this check's: 2 MiB of warning before the limit
DEFAULT_FEATURES = "cli"  # the shipped binary's (`release.yml`)


def mib(n):
    return f"{n / (1024 * 1024):.2f} MiB"


def package_id(manifest):
    """`(name, version)` from the `[package]` table of the manifest at path `manifest`."""
    with open(manifest, encoding="utf-8") as f:
        text = f.read()
    table = re.search(r"^\[package\]\n(.*?)(?=^\[|\Z)", text, re.M | re.S)
    if table is None:
        raise RuntimeError(f"{manifest} has no [package] table")
    fields = dict(re.findall(r'^(name|version)\s*=\s*"([^"]+)"', table.group(1), re.M))
    if set(fields) != {"name", "version"}:
        raise RuntimeError(f"{manifest}: [package] needs a literal name and version, found {fields}")
    return fields["name"], fields["version"]


def target_dir(root, environ=None):
    environ = os.environ if environ is None else environ
    return os.path.join(environ.get("CARGO_TARGET_DIR") or os.path.join(root, "target"), "crate-size")


def package_command(target, verify, features, offline):
    cmd = ["cargo", "package", "--allow-dirty", "--target-dir", target]
    if verify:
        cmd += ["--features", features]
    else:
        cmd.append("--no-verify")
    if offline:
        cmd.append("--offline")
    return cmd


def package(root, verify, features, run=subprocess.run):
    """Run `cargo package` and return the path of the .crate it wrote.

    Offline first, so a machine with the registry already cached never waits on the network; again
    online only when cargo says the offline run was missing something (not on a build failure, which
    would fail the same way twice)."""
    target = target_dir(root)
    out = run(package_command(target, verify, features, True), cwd=root, capture_output=True, text=True, check=False)
    if out.returncode != 0 and "offline" in out.stderr:
        out = run(package_command(target, verify, features, False), cwd=root, capture_output=True, text=True, check=False)
    sys.stderr.write(out.stderr)
    if out.returncode != 0:
        raise RuntimeError(f"`cargo package` failed with exit status {out.returncode}")
    name, version = package_id(os.path.join(root, "Cargo.toml"))
    return os.path.join(target, "package", f"{name}-{version}.crate")


def check(measured, ceiling=CEILING, limit=LIMIT):
    """`(exit code, lines to print)` for a .crate of `measured` bytes."""
    line = (
        f".crate is {measured} bytes ({mib(measured)}): {ceiling - measured:+} bytes to this check's ceiling "
        f"({mib(ceiling)}), {limit - measured:+} to crates.io's limit ({mib(limit)})"
    )
    if measured > ceiling:
        return 1, [
            f"::error::the published .crate is {mib(measured)}, over the {mib(ceiling)} ceiling of scripts/crate_size.py "
            f"(crates.io refuses anything over {mib(limit)}, and release.yml publishes last)",
            line,
            "Keep a file that does not need to ship out of `include` in Cargo.toml, or raise CEILING in "
            "scripts/crate_size.py with the reason (never to within 1 MiB of the limit).",
        ]
    return 0, [line]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = parser.add_subparsers(dest="command", required=True)
    c = sub.add_parser("check", help="package the crate and compare the .crate's size with the ceiling")
    c.add_argument("--verify", action="store_true", help="let `cargo package` build the packaged crate")
    c.add_argument("--features", default=DEFAULT_FEATURES, help="features the verify build enables (default: %(default)s)")
    c.add_argument("--crate", help="measure this .crate instead of packaging one")
    args = parser.parse_args(argv)

    try:
        crate = args.crate or package(ROOT, args.verify, args.features)
    except RuntimeError as e:
        print(f"::error::{e}")
        return 1
    code, lines = check(os.path.getsize(crate))
    for line in lines:
        print(line)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write("### Published .crate size\n\n")
            f.write("".join(f"    {line}\n" for line in lines) + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
