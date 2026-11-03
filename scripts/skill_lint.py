#!/usr/bin/env python3
"""Structural lint for the repository's skills in `.claude/skills/` (#5971).

A skill that stops loading, or stops being found, shows no error anywhere: the host skips it.
The only other automated skill check is the `SKILL.md` casing job (#189), so nothing checked a
skill's frontmatter, size or links. This reads files only and checks that:

- `SKILL.md` starts with YAML frontmatter holding a `name` and a `description`;
- `name` is at most 64 characters of lowercase letters, digits and single hyphens, equals the
  skill's directory name and holds neither `claude` nor `anthropic`;
- `description` is at most 1024 characters (Codex's limit as well as Claude's);
- every frontmatter key is one the hosts read (`KNOWN_KEYS`): a misspelt key is silently ignored;
- the body of `SKILL.md` is at most 500 lines (longer detail belongs in a file the skill links to);
- every relative link in a skill's Markdown files resolves to a file or directory that exists.

`.agents/skills` is a symlink to `.claude/skills` (#5956), so Codex reads the same files: the
Claude-only keys (`user-invocable`, `argument-hint`, ...) are accepted there, which is why they
are in `KNOWN_KEYS` and not a warning.

    python3 scripts/skill_lint.py                  # every skill under .claude/skills
    python3 scripts/skill_lint.py DIR...           # the named skill directories
    python3 scripts/skill_lint.py --strict         # warnings fail too

Errors exit 1. Warnings (a description that never says when to use the skill) exit 1 only under
`--strict`. The check is mirrored by `scripts/pre_push.py` and run by the `skill-casing` job of
`.github/workflows/ci.yml`; `scripts/test_skill_lint.py` is its test.
"""

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKILLS_DIR = os.path.join(ROOT, ".claude", "skills")

MAX_NAME = 64
MAX_DESCRIPTION = 1024
MAX_BODY_LINES = 500
RESERVED_WORDS = ("claude", "anthropic")

# Keys a host reads. The first group is the cross-host set; the second is read by Claude Code
# (and accepted by Codex, #5956) only.
KNOWN_KEYS = frozenset(
    {
        "name",
        "description",
        "license",
        "compatibility",
        "metadata",
        "allowed-tools",
        # Claude Code only:
        "when_to_use",
        "argument-hint",
        "arguments",
        "disable-model-invocation",
        "user-invocable",
        "disallowed-tools",
        "paths",
        "context",
        "agent",
        "model",
        "effort",
        "hooks",
        "shell",
    }
)

# A description that says when to use the skill, as the repository's own skills do.
WHEN_TO_USE = re.compile(
    r"\b(use|used) (this skill )?(when|for|to|any time|whenever)\b|\bwhen (the user|asked|you)\b"
    r"|\btriggers? (on|when)\b|\bwhenever\b",
    re.IGNORECASE,
)


def parse_frontmatter(text):
    """`(fields, body, body_start_line)` of a skill file, or `None` without frontmatter.

    A reader of the subset skills use, not a YAML parser (PyYAML is not guaranteed on a runner):
    top-level `key: value` pairs, quoted or plain; an indented line continues the value above
    it; a block scalar (`>`, `|`) drops its indicator and is folded (`>`) or kept per line (`|`).
    A list or map is flattened to a string, enough to know the key is there.
    """
    m = re.match(r"---\r?\n(.*?)\r?\n---\r?\n?", text, re.DOTALL)
    if not m:
        return None
    raw = {}
    key = None
    for line in m.group(1).splitlines():
        kv = re.match(r"([A-Za-z][\w-]*):\s*(.*)$", line)
        if kv:
            key = kv.group(1)
            raw[key] = (kv.group(2).strip(), [])
        elif key and re.match(r"\s+\S", line):
            raw[key][1].append(line.strip())
    fields = {}
    for key, (head, rest) in raw.items():
        block = re.fullmatch(r"([>|])[+-]?", head)
        if block:
            value = (" " if block.group(1) == ">" else "\n").join(rest)
        else:
            value = " ".join(part for part in [head, *rest] if part)
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        fields[key] = value
    body = text[m.end():]
    return fields, body, text[: m.end()].count("\n") + 1


def markdown_links(text):
    """Relative link targets of a Markdown text, outside code fences and code spans."""
    text = re.sub(r"```.*?```", "", text, flags=re.DOTALL)
    text = re.sub(r"~~~.*?~~~", "", text, flags=re.DOTALL)
    text = re.sub(r"`[^`\n]*`", "", text)
    for m in re.finditer(r"\[[^\]\n]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)", text):
        target = m.group(1).split("#", 1)[0]
        # A URL, a mail link or a placeholder such as `(…)` is not a path.
        if not target or not re.search(r"[A-Za-z0-9]", target) or re.match(r"[A-Za-z][A-Za-z0-9+.-]*:", target):
            continue
        yield target


def lint_skill(directory):
    """`[(level, file, message)]` for one skill directory; level is `error` or `warning`."""
    found = []
    rel = lambda p: os.path.relpath(p, ROOT) if p.startswith(ROOT) else p  # noqa: E731

    def add(level, path, message):
        found.append((level, rel(path), message))

    manifest = os.path.join(directory, "SKILL.md")
    if not os.path.isfile(manifest):
        add("error", directory, "SKILL.md not found")
        return found
    with open(manifest, encoding="utf-8") as f:
        text = f.read()
    parsed = parse_frontmatter(text)
    if parsed is None:
        add("error", manifest, "no YAML frontmatter: the file must start with a --- block")
        return found
    fields, body, _ = parsed

    name = fields.get("name", "")
    if not name:
        add("error", manifest, "frontmatter has no name")
    else:
        directory_name = os.path.basename(os.path.realpath(directory))
        if len(name) > MAX_NAME:
            add("error", manifest, f"name is {len(name)} characters (limit {MAX_NAME})")
        if not re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", name):
            add("error", manifest, f'name "{name}" must be lowercase letters, digits and single hyphens')
        if name != directory_name:
            add("error", manifest, f'name "{name}" must equal its directory name "{directory_name}"')
        for word in RESERVED_WORDS:
            if word in name:
                add("error", manifest, f'name holds the reserved word "{word}"')

    description = fields.get("description", "")
    if not description:
        add("error", manifest, "frontmatter has no description")
    else:
        if len(description) > MAX_DESCRIPTION:
            add("error", manifest, f"description is {len(description)} characters (limit {MAX_DESCRIPTION})")
        if not WHEN_TO_USE.search(description):
            add("warning", manifest, 'description does not say when to use the skill ("Use when ...")')

    for key in fields:
        if key not in KNOWN_KEYS:
            add("error", manifest, f'unknown frontmatter key "{key}": no host reads it')

    body_lines = len(body.rstrip("\n").split("\n")) if body.strip() else 0
    if body_lines > MAX_BODY_LINES:
        add("error", manifest, f"SKILL.md body is {body_lines} lines (limit {MAX_BODY_LINES}): move detail into a file it links to")

    for dirpath, dirnames, filenames in os.walk(directory):
        dirnames[:] = sorted(d for d in dirnames if d not in ("node_modules", "results"))
        for filename in sorted(filenames):
            if not filename.lower().endswith(".md"):
                continue
            path = os.path.join(dirpath, filename)
            with open(path, encoding="utf-8") as f:
                content = f.read()
            for target in sorted(set(markdown_links(content))):
                if not os.path.exists(os.path.normpath(os.path.join(dirpath, target))):
                    add("error", path, f"link to a missing file: {target}")
    return found


def skill_directories(root=SKILLS_DIR):
    return [
        os.path.join(root, entry)
        for entry in sorted(os.listdir(root))
        if os.path.isdir(os.path.join(root, entry))
    ]


def main(argv):
    strict = "--strict" in argv
    directories = [a for a in argv if not a.startswith("--")] or skill_directories()
    errors = warnings = 0
    for directory in directories:
        findings = lint_skill(directory)
        for level, path, message in findings:
            print(f"{level.upper() if level == 'error' else level}: {path}: {message}")
            errors += level == "error"
            warnings += level == "warning"
    print(f"{len(directories)} skill(s), {errors} error(s), {warnings} warning(s)")
    return 1 if errors or (strict and warnings) else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
