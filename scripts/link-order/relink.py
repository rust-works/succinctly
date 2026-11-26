#!/usr/bin/env python3
"""Link a saved `succinctly` link again, optionally under a layout seed (spike).

A release build run with `SUCCINCTLY_LINK_SAVE_DIR=<dir>` leaves the link's objects and arguments
in `<dir>` (link.py's `save_link`). This links them again into `OUT`, through link.py, so the
ordering list is resolved exactly as in the original link and only the layout seed differs:

    scripts/link-order/relink.py SAVE_DIR OUT [--seed N] [--mode pad|shuffle|pad,shuffle]

With no `--seed` the result should match the cargo-built binary byte for byte (the method's own
control). Each seeded link takes seconds where a rebuild takes minutes (one codegen unit, fat LTO).
`OUT`'s basename must be `succinctly` so link.py treats it as the release binary.
"""
import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("save_dir")
    p.add_argument("out")
    p.add_argument("--seed", type=int)
    p.add_argument("--mode")
    a = p.parse_args(argv)
    if os.path.basename(a.out) != "succinctly":
        sys.exit("OUT must be named `succinctly` (link.py orders only the release binary)")
    with open(os.path.join(a.save_dir, "link.json")) as f:
        saved = json.load(f)
    args = list(saved["args"])
    i = args.index("-o")
    args[i + 1] = os.path.abspath(a.out)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    env = dict(os.environ)
    env.pop("SUCCINCTLY_LINK_SAVE_DIR", None)
    env.pop("SUCCINCTLY_LINK_LAYOUT_SEED", None)
    env.pop("SUCCINCTLY_LINK_LAYOUT_MODE", None)
    env["SUCCINCTLY_LINK_ORDER_LIST"] = saved["list"]
    if a.seed is not None:
        env["SUCCINCTLY_LINK_LAYOUT_SEED"] = str(a.seed)
    if a.mode:
        env["SUCCINCTLY_LINK_LAYOUT_MODE"] = a.mode
    r = subprocess.run([sys.executable, os.path.join(HERE, "link.py")] + args, cwd=saved["cwd"], env=env)
    if r.returncode:
        sys.exit(r.returncode)
    with open(a.out + ".link-order.log") as f:
        sys.stdout.write(f.read())


if __name__ == "__main__":
    main()
