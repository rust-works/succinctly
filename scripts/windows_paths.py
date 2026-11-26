#!/usr/bin/env python3
"""Fail on any tracked path that a Windows checkout cannot create (#7953).

`src/util/simd/nul.rs` reached a release tag: `NUL` is a reserved device name on Windows, with or
without an extension, so git refuses to check the file out there, and the release workflow's
Windows build failed at checkout. CI's Windows leg is a type check cross-compiled on Linux, which
never checks the tree out on Windows, so nothing caught it.

A path component is refused when:

- its name before the first dot is a reserved device name (`CON`, `PRN`, `AUX`, `NUL`,
  `COM0`-`COM9`, `LPT0`-`LPT9`, and the superscript-digit forms), in any case;
- it holds a character Windows forbids in a name (`<>:"|?*`, a backslash, or a control
  character);
- it ends in a dot or a space.

Usage:
    python3 scripts/windows_paths.py check     # every path `git ls-files` lists
"""

import re
import subprocess
import sys

RESERVED = re.compile(r"^(con|prn|aux|nul|com[0-9¹²³]|lpt[0-9¹²³])$", re.IGNORECASE)
FORBIDDEN = re.compile(r'[<>:"|?*\\\x00-\x1f]')


def problem(component: str):
    """Why Windows cannot create a path component, or None when it can."""
    if RESERVED.match(component.split(".", 1)[0]):
        return "a reserved device name"
    if FORBIDDEN.search(component):
        return "a character Windows forbids in a name"
    if component.endswith((".", " ")):
        return "a trailing dot or space"
    return None


def bad_paths(paths):
    """(path, reason) for every path with a component Windows cannot create."""
    out = []
    for path in paths:
        for component in path.split("/"):
            reason = problem(component)
            if reason:
                out.append((path, f"`{component}` is {reason}"))
                break
    return out


def tracked_paths():
    listing = subprocess.run(["git", "ls-files", "-z"], check=True, capture_output=True).stdout
    return [p for p in listing.decode("utf-8", "surrogateescape").split("\0") if p]


def main(argv):
    if argv[1:] != ["check"]:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    bad = bad_paths(tracked_paths())
    for path, reason in bad:
        print(f"{path}: {reason}; a Windows checkout cannot create it (#7953)")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
