# Changelog fragments

`CHANGELOG.md` used to be the one file every PR edited, so any two PRs in flight
conflicted on the same hunk (#3920). Instead, **each PR adds its own file here**;
`scripts/changelog.py collect` assembles them into `CHANGELOG.md` at release time.
Two PRs that each add a fragment never touch the same file.

## Adding a fragment

```
changelog.d/<id>.<type>[.<n>].md
```

| Part     | Meaning                                                                                    |
|----------|--------------------------------------------------------------------------------------------|
| `<id>`   | the issue number (`3767`), or `+<slug>` (`+fix-typo`) for a PR with no issue               |
| `<type>` | `added`, `changed`, `deprecated`, `removed`, `performance`, `fixed` or `security`          |
| `<n>`    | optional positive integer for a second entry of the same id and type (`3767.changed.2.md`) |

The file body is the entry exactly as it is written in `CHANGELOG.md`: a bullet
(`- `) with the bold lead-in, the prose and the measured numbers, continuation
lines indented two spaces. No section heading; the type is the section.

```markdown
- **jq: `indices` of a string no longer panics on a multi-byte needle** (#3903).
  `"éé" | indices("é")` panicked with `byte index 1 is not a char boundary`; ...
```

A follow-up that corrects or refreshes an entry edits **that PR's own fragment**
(if it has not been released yet), so it conflicts with nothing.

Anything in this directory other than `README.md` that is not a valid fragment name
fails the check, so a typo (`3767.chnaged.md`) cannot silently drop an entry at release.

## What CI checks

The `Changelog fragments` workflow runs `python3 scripts/changelog.py check` (every
fragment's name, type and body) and `check-pr` (the PR adds, edits or renames a
fragment). A PR with no user-visible effect (docs-only, CI-only, a refactor) waives
the second check with **either** the `no-changelog` label **or** a `[no changelog]`
line in the PR body (the marker is matched anywhere in the body outside HTML comments, so
do not quote it when merely describing it). Release PRs (`chore(release): ...`) and bot PRs are waived
automatically.

## Releasing

```bash
python3 scripts/changelog.py collect --version X.Y.Z --dry-run   # preview the fragments
python3 scripts/changelog.py collect --version X.Y.Z             # rewrite CHANGELOG.md
```

`collect` renders `## [X.Y.Z] - DATE` with sections in Keep a Changelog order (Added,
Changed, Deprecated, Removed, Performance, Fixed, Security), entries by ascending issue
number (`+slug` entries last), updates the `[Unreleased]` / `[X.Y.Z]` compare links, and
deletes the consumed fragments. See `docs/guides/release.md`.

## The legacy `[Unreleased]` text

`CHANGELOG.md`'s `[Unreleased]` section predates fragments and is left untouched. At the
first release, `collect` carries whatever is under `## [Unreleased]` into the new section
verbatim, **after** the fragment sections, and leaves an empty `[Unreleased]` behind. From
then on the changelog is purely fragments. That first section therefore repeats `###`
headings (the fragments' sections, then the legacy text's own); the legacy text already did. Nobody should add new entries under
`[Unreleased]` by hand.
