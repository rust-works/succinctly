#!/usr/bin/env python3
"""Link a saved `succinctly` link again, optionally under a layout seed (#7960).

A release build run with `SUCCINCTLY_LINK_SAVE_DIR=<dir>` leaves the link's objects and arguments
in `<dir>` (link.py's `save_link`). This links them again into `OUT`, through link.py, so the
ordering list is resolved exactly as in the original link and only the layout seed differs:

    scripts/link-order/relink.py SAVE_DIR OUT [--seed N] [--mode pad|shuffle|pad,shuffle]

With no `--seed` the result is cargo's own binary again: byte for byte on Linux; on macOS the
code (`__text`, `__const`, `__eh_frame`) is identical and `__LINKEDIT` is larger, from the saved
objects' longer paths. That is the method's control. Each seeded link takes seconds where a
rebuild takes minutes (one codegen unit, fat LTO). `OUT`'s basename must be `succinctly` so
link.py treats it as the release binary. It fails when the link log shows that the ordering, or
the seed asked for, was not applied: a bad seed or mode would otherwise give an unperturbed binary.

docs/guides/benchmarking.md ("Layout-seeded A/B") has the whole recipe.
"""
import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# The measurement variables link.py reads; a relink sets them afresh from its own arguments.
LAYOUT_VARS = ("SUCCINCTLY_LINK_SAVE_DIR", "SUCCINCTLY_LINK_LAYOUT_SEED", "SUCCINCTLY_LINK_LAYOUT_MODE",
               "SUCCINCTLY_LINK_ORDER_LIST")


def relink_args(saved_args, out):
    """The saved link's arguments with its output replaced by `out` (made absolute)."""
    args = list(saved_args)
    i = args.index("-o")
    args[i + 1] = os.path.abspath(out)
    return args


def relink_env(base, order_list, seed=None, mode=None):
    """`base` without any measurement variable it carried, then with the saved list and the seed
    and mode asked for: a relink never inherits a seed or a save directory from its caller."""
    env = {k: v for k, v in base.items() if k not in LAYOUT_VARS}
    env["SUCCINCTLY_LINK_ORDER_LIST"] = order_list
    if seed is not None:
        env["SUCCINCTLY_LINK_LAYOUT_SEED"] = str(seed)
    if mode:
        env["SUCCINCTLY_LINK_LAYOUT_MODE"] = mode
    return env


def log_problem(log, seed=None):
    """Why the link log says the relink did not do what was asked, or None."""
    if log.startswith("plain link"):
        return f"linked without the function order: {log.strip()}"
    if seed is not None and f"layout seed {seed}," not in log:
        return f"the link log does not show layout seed {seed}: {log.strip()}"
    return None


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("save_dir", help="the directory a SUCCINCTLY_LINK_SAVE_DIR build saved its link in")
    p.add_argument("out", help="the binary to write; its basename must be `succinctly`")
    p.add_argument("--seed", type=int, help="the layout seed (SUCCINCTLY_LINK_LAYOUT_SEED)")
    p.add_argument("--mode", help="pad, shuffle or pad,shuffle (SUCCINCTLY_LINK_LAYOUT_MODE)")
    a = p.parse_args(argv)
    if os.path.basename(a.out) != "succinctly":
        sys.exit("OUT must be named `succinctly` (link.py orders only the release binary)")
    with open(os.path.join(a.save_dir, "link.json")) as f:
        saved = json.load(f)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)), exist_ok=True)
    env = relink_env(os.environ, saved["list"], a.seed, a.mode)
    r = subprocess.run([sys.executable, os.path.join(HERE, "link.py")] + relink_args(saved["args"], a.out),
                       cwd=saved["cwd"], env=env)
    if r.returncode:
        sys.exit(r.returncode)
    with open(a.out + ".link-order.log") as f:
        log = f.read()
    sys.stdout.write(log)
    problem = log_problem(log, a.seed)
    if problem:
        sys.exit(f"relink: {a.out}: {problem}")


if __name__ == "__main__":
    main()
