#!/usr/bin/env python3
"""Changelog fragments: validate them, check a PR carries one, assemble a release.

`CHANGELOG.md` used to be the one file every PR edited, so any two PRs in
flight conflicted on the same hunk (#3920). Each PR now adds its own file under
`changelog.d/` instead, and this script renders them into a release section.

    changelog.d/<id>.<type>[.<n>].md

  <id>    an issue number (`3767`), or `+<slug>` (`+fix-typo`) for a PR with none
  <type>  added | changed | deprecated | removed | performance | fixed | security
  <n>     optional positive integer, for a second entry of the same id and type

The body is the entry exactly as it is written in CHANGELOG.md: a bullet (`- `)
and any continuation lines.

Subcommands:

  check      validate every fragment's name, type and body (exit 1 on any error)
  check-pr   additionally require that a PR adds/edits a fragment or is waived
  collect    render the fragments into `## [X.Y.Z] - DATE`, carry whatever is
             under `## [Unreleased]` into it verbatim, update the compare links
             and delete the consumed fragments (`--dry-run` previews)

Standard library only. See changelog.d/README.md for the contributor view and
docs/guides/release.md for the release flow.
"""

import argparse
import datetime
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_CHANGELOG = ROOT / "CHANGELOG.md"
DEFAULT_FRAGMENTS = ROOT / "changelog.d"

# (type, section heading), in the order a rendered release lists them: Keep a
# Changelog's order, with `Performance` where the 0.7.0 section put it.
TYPES = [
    ("added", "Added"),
    ("changed", "Changed"),
    ("deprecated", "Deprecated"),
    ("removed", "Removed"),
    ("performance", "Performance"),
    ("fixed", "Fixed"),
    ("security", "Security"),
]
TYPE_NAMES = dict(TYPES)

# Files in the fragments directory that are not fragments.
NON_FRAGMENTS = {"README.md"}

FRAGMENT_RE = re.compile(
    r"^(?P<id>[1-9][0-9]*|\+[a-z0-9][a-z0-9-]*)"
    r"\.(?P<type>[a-z]+)"
    r"(?:\.(?P<n>[1-9][0-9]*))?"
    r"\.md$"
)
VERSION_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")
DATE_RE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$")

WAIVER_LABEL = "no-changelog"
WAIVER_MARKER_RE = re.compile(r"\[(?:no|skip) changelog\]", re.IGNORECASE)
HTML_COMMENT_RE = re.compile(r"<!--.*?-->", re.DOTALL)
RELEASE_TITLE_PREFIX = "chore(release):"


class ChangelogError(Exception):
    """A problem the user must fix; printed without a traceback."""


class Fragment:
    def __init__(self, path, frag_id, frag_type, n, body):
        self.path = path
        self.id = frag_id
        self.type = frag_type
        self.n = n
        self.body = body

    @property
    def sort_key(self):
        # Issue numbers ascending, `+slug` orphans after them, then the suffix.
        if self.id.startswith("+"):
            return (1, 0, self.id, self.n)
        return (0, int(self.id), "", self.n)


def validate_name(name):
    """Return (id, type, n) for a fragment filename, or raise ChangelogError."""
    m = FRAGMENT_RE.match(name)
    if not m:
        raise ChangelogError(
            f"{name}: not a valid fragment name; expected "
            "<issue>.<type>[.<n>].md or +<slug>.<type>[.<n>].md"
        )
    if m["type"] not in TYPE_NAMES:
        raise ChangelogError(
            f"{name}: unknown type '{m['type']}'; expected one of "
            + ", ".join(t for t, _ in TYPES)
        )
    return m["id"], m["type"], int(m["n"] or 0)


def read_body(path, name):
    try:
        text = path.read_bytes().decode("utf-8")
    except UnicodeDecodeError:
        raise ChangelogError(f"{name}: not valid UTF-8")
    if "\r" in text:
        raise ChangelogError(f"{name}: contains a carriage return; use LF line endings")
    body = text.rstrip()
    if not body:
        raise ChangelogError(f"{name}: empty")
    if not body.startswith("- "):
        raise ChangelogError(
            f"{name}: an entry must start with a bullet ('- '), as in CHANGELOG.md"
        )
    return body


def load_fragments(directory):
    """Parse every fragment in `directory`; return (fragments, errors)."""
    fragments, errors = [], []
    if not directory.is_dir():
        return fragments, errors
    for path in sorted(directory.iterdir()):
        name = path.name
        if name in NON_FRAGMENTS:
            continue
        if not path.is_file():
            errors.append(f"{name}: not a regular file")
            continue
        try:
            frag_id, frag_type, n = validate_name(name)
            body = read_body(path, name)
        except ChangelogError as e:
            errors.append(str(e))
            continue
        fragments.append(Fragment(path, frag_id, frag_type, n, body))
    fragments.sort(key=lambda f: f.sort_key)
    return fragments, errors


def render_sections(fragments):
    """Render fragments as `### Heading` blocks, one string per non-empty type."""
    sections = []
    for frag_type, heading in TYPES:
        bodies = [f.body for f in fragments if f.type == frag_type]
        if bodies:
            sections.append(f"### {heading}\n\n" + "\n".join(bodies))
    return sections


def _heading_line(line):
    return line.rstrip("\r\n")


def build_release(changelog_text, fragments, version, date):
    """Return the new CHANGELOG.md text; raises ChangelogError if it cannot."""
    if not VERSION_RE.match(version):
        raise ChangelogError(f"'{version}' is not a semantic version (X.Y.Z)")
    if not DATE_RE.match(date):
        raise ChangelogError(f"'{date}' is not a YYYY-MM-DD date")

    lines = changelog_text.splitlines(keepends=True)
    heads = [_heading_line(line) for line in lines]
    if "## [Unreleased]" not in heads:
        raise ChangelogError("CHANGELOG.md has no '## [Unreleased]' heading")
    if any(h.startswith(f"## [{version}]") for h in heads):
        raise ChangelogError(f"CHANGELOG.md already has a section for {version}")
    start = heads.index("## [Unreleased]")

    # The Unreleased body ends at the next release heading, or at the footer
    # link block when no release has been made yet.
    end = len(lines)
    for i in range(start + 1, len(lines)):
        if heads[i].startswith("## [") or re.match(r"^\[[^\]]+\]: ", heads[i]):
            end = i
            break

    body = lines[start + 1 : end]
    while body and not body[0].strip():
        body.pop(0)
    while body and not body[-1].strip():
        body.pop()
    legacy = "".join(body)
    if legacy and not legacy.endswith("\n"):
        legacy += "\n"

    sections = render_sections(fragments)
    if not sections and not legacy:
        raise ChangelogError("nothing to release: no fragments and an empty [Unreleased]")

    content = "\n\n".join(sections) + "\n" if sections else ""
    if legacy:
        content += ("\n" if content else "") + legacy

    previous = None
    for h in heads[end:]:
        m = re.match(r"^## \[([0-9][^\]]*)\]", h)
        if m:
            previous = m[1]
            break

    tail = "".join(lines[end:])
    link_re = re.compile(
        r"^\[Unreleased\]: (?P<base>\S+?)/compare/v[^\s]+\.\.\.HEAD[ \t]*$", re.MULTILINE
    )
    m = link_re.search(tail)
    if not m:
        raise ChangelogError(
            "CHANGELOG.md has no '[Unreleased]: .../compare/vX...HEAD' link to update"
        )
    base = m["base"]
    new_links = f"[Unreleased]: {base}/compare/v{version}...HEAD\n"
    if previous:
        new_links += f"[{version}]: {base}/compare/v{previous}...v{version}"
    else:
        new_links += f"[{version}]: {base}/releases/tag/v{version}"
    tail = tail[: m.start()] + new_links + tail[m.end() :]

    head = "".join(lines[: start + 1])
    return f"{head}\n## [{version}] - {date}\n\n{content}\n{tail}"


def write_atomically(path, text):
    """Replace `path` with `text`, keeping its mode (a temp file is 0600)."""
    mode = path.stat().st_mode & 0o7777
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=path.name + ".")
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="") as f:
            f.write(text)
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        if os.path.exists(tmp):
            os.unlink(tmp)
        raise


def waiver_reason(title, body, labels, author):
    """Return why this PR needs no fragment, or None if it does."""
    if title.strip().startswith(RELEASE_TITLE_PREFIX):
        return f"release PR (title starts with '{RELEASE_TITLE_PREFIX}'; it consumes fragments)"
    if author.endswith("[bot]"):
        return f"opened by a bot ({author})"
    if WAIVER_LABEL in labels:
        return f"label '{WAIVER_LABEL}'"
    # A marker inside an HTML comment is the template's own instructions.
    if WAIVER_MARKER_RE.search(HTML_COMMENT_RE.sub("", body)):
        return "'[no changelog]' marker in the PR body"
    return None


def changed_fragment_names(base, head, repo=ROOT, directory="changelog.d"):
    """Names of fragments the PR adds, modifies or renames into place."""
    out = subprocess.run(
        ["git", "-C", str(repo), "diff", "--name-only", "--diff-filter=AMR",
         f"{base}...{head}", "--", directory],
        check=True, capture_output=True, text=True,
    ).stdout
    names = [Path(p).name for p in out.splitlines()]
    return [n for n in names if n not in NON_FRAGMENTS and FRAGMENT_RE.match(n)]


def cmd_check(args):
    fragments, errors = load_fragments(Path(args.fragments_dir))
    for e in errors:
        print(f"error: {e}", file=sys.stderr)
    if errors:
        print(f"{len(errors)} invalid fragment(s) in {args.fragments_dir}", file=sys.stderr)
        return 1
    print(f"{len(fragments)} fragment(s) valid")
    return 0


def cmd_check_pr(args):
    status = cmd_check(args)
    if status:
        return status
    labels = {l.strip() for l in args.labels.split(",") if l.strip()}
    body = Path(args.body_file).read_text(encoding="utf-8") if args.body_file else ""
    reason = waiver_reason(args.title, body, labels, args.author)
    if reason:
        print(f"changelog fragment waived: {reason}")
        return 0
    names = changed_fragment_names(args.base, args.head)
    if names:
        print("changelog fragment present: " + ", ".join(names))
        return 0
    print(
        "error: this PR neither adds a changelog fragment nor is waived.\n"
        "  Add changelog.d/<issue>.<type>.md (see changelog.d/README.md), or waive it\n"
        f"  with the '{WAIVER_LABEL}' label or a '[no changelog]' line in the PR body\n"
        "  (docs-only / CI-only / refactor changes with no user-visible effect).",
        file=sys.stderr,
    )
    return 1


def cmd_collect(args):
    changelog = Path(args.changelog)
    fragments, errors = load_fragments(Path(args.fragments_dir))
    if errors:
        for e in errors:
            print(f"error: {e}", file=sys.stderr)
        raise ChangelogError(f"{len(errors)} invalid fragment(s); fix them before releasing")
    date = args.date or datetime.date.today().isoformat()
    text = changelog.read_text(encoding="utf-8")
    new_text = build_release(text, fragments, args.version, date)

    sections = render_sections(fragments)
    names = ", ".join(f.path.name for f in fragments) or "none"
    if args.dry_run:
        print(f"## [{args.version}] - {date}\n")
        print("\n\n".join(sections))
        print(
            f"\n-- dry run: would consume {len(fragments)} fragment(s) ({names}); "
            "nothing written or deleted --",
            file=sys.stderr,
        )
        return 0

    write_atomically(changelog, new_text)
    for f in fragments:
        f.path.unlink()
    print(f"wrote {changelog.name}: [{args.version}] - {date}; consumed {len(fragments)} fragment(s)")
    return 0


def build_parser():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = p.add_subparsers(dest="command", required=True)

    def common(sp, changelog=False):
        sp.add_argument("--fragments-dir", default=str(DEFAULT_FRAGMENTS))
        if changelog:
            sp.add_argument("--changelog", default=str(DEFAULT_CHANGELOG))

    sp = sub.add_parser("check", help="validate every fragment")
    common(sp)
    sp.set_defaults(func=cmd_check)

    sp = sub.add_parser("check-pr", help="require a fragment or a waiver for a PR")
    common(sp)
    sp.add_argument("--base", default="origin/main")
    sp.add_argument("--head", default="HEAD")
    sp.add_argument("--title", default="")
    sp.add_argument("--body-file", default="")
    sp.add_argument("--labels", default="", help="comma-separated label names")
    sp.add_argument("--author", default="")
    sp.set_defaults(func=cmd_check_pr)

    sp = sub.add_parser("collect", help="render fragments into a release section")
    common(sp, changelog=True)
    sp.add_argument("--version", required=True)
    sp.add_argument("--date", default="")
    sp.add_argument("--dry-run", action="store_true")
    sp.set_defaults(func=cmd_collect)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        return args.func(args)
    except ChangelogError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
