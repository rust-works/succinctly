#!/usr/bin/env python3
"""Fit patchcov's rendered coverage comment under GitHub's comment limit.

    fit-comment.py IN OUT ARTIFACT

GitHub refuses an issue comment longer than 65,536 characters, and patchcov lists
every tolerated region and every line of unchanged code whose coverage changed. A
change that moves much unchanged code in or out of coverage (#5223: 5,467 lines)
renders a comment several times that size, and the sticky-comment step then fails
the job. The headline figures and the patch section stay as rendered. The longest
`<details>` lists are cut, longest first, to their first entries and a line saying
how many were left out and that the full report is in the ARTIFACT artifact, until
the comment fits. A comment that already fits is copied unchanged.
"""
import re
import sys

LIMIT = 65_536
# Room for the cut notes and for a sticky-comment header the posting step adds.
BUDGET = LIMIT - 2_048
KEEP = 40

DETAILS = re.compile(r"(<details><summary>.*?</summary>\n)(.*?)(\n</details>)", re.S)


def fit(text, artifact):
    if len(text) <= BUDGET:
        return text
    blocks = sorted(DETAILS.finditer(text), key=lambda m: len(m.group(2)), reverse=True)
    cuts = {}
    excess = len(text) - BUDGET
    for m in blocks:
        if excess <= 0:
            break
        entries = [line for line in m.group(2).split("\n") if line.startswith(("- ", "| `"))]
        if len(entries) <= KEEP:
            continue
        head, n = [], 0
        for line in m.group(2).split("\n"):
            if line.startswith(("- ", "| `")):
                n += 1
                if n > KEEP:
                    continue
            head.append(line)
        left = len(entries) - KEEP
        note = f"\n_{left} more not shown: the full report is in the `{artifact}` artifact._"
        body = "\n".join(head) + note
        cuts[m.start(2)] = (m.end(2), body)
        excess -= len(m.group(2)) - len(body)
    out, at = [], 0
    for start in sorted(cuts):
        end, body = cuts[start]
        out.append(text[at:start])
        out.append(body)
        at = end
    out.append(text[at:])
    fitted = "".join(out)
    if len(fitted) > BUDGET:
        # Still too long (no list was long enough to cut): keep the headline only.
        fitted = fitted[:BUDGET] + f"\n\n_Truncated: the full report is in the `{artifact}` artifact._\n"
    return fitted


def main():
    src, dst, artifact = sys.argv[1:4]
    with open(src, encoding="utf-8") as f:
        text = f.read()
    fitted = fit(text, artifact)
    with open(dst, "w", encoding="utf-8") as f:
        f.write(fitted)
    print(f"coverage comment: {len(text)} -> {len(fitted)} characters")


if __name__ == "__main__":
    main()
