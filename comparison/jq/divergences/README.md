# Known differences from the reference jq

The ledger of `succinctly dev compare jq` (#5409), one entry per file (#5769). The run fails on a
difference that is not listed here and on a listed one that no longer differs.

An entry is `<suite>/<probe>.toml`. Its id is the path: the directory is the suite, the file name
is the probe, and the `/` before an arity is written `.`:

| Id                    | File                               |
|-----------------------|------------------------------------|
| `cli:version`         | `cli/version.toml`                 |
| `features:strftime/1` | `features/strftime.1.toml`         |

A suite and a probe use only letters, digits, `_` and `-`. A file that no id maps to, a file that
is not `.toml`, and a `divergences.toml` beside this directory fail the loader
(`src/bin/succinctly/compare/ledger.rs`).

The file holds the other fields, with no `[[divergence]]` header and no `id`:

| Field      | Meaning                                                                                       |
|------------|-----------------------------------------------------------------------------------------------|
| `category` | `by-design`, `host-dependent`, `newer-than-primary`, `reference-crash`, `reference-slow` or `to-triage` (a real difference nobody has yet classified against ADR-0018 or filed) |
| `reason`   | what differs and why                                                                          |
| `issue`    | optional tracking issue. A branch that references a `to-triage` entry's issue deletes the entry in the same change (`scripts/ledger_refs.py`, #5585); one that only mentions it says `[ledger keeps #N]` on its own line in a commit message |
| `platform` | optional `std::env::consts::OS` the entry was observed on; other OSes neither expect nor call it stale |
| `volatile` | optional `true`: whether the difference shows depends on the host's speed (a probe the reference finishes inside the 10 s deadline on a fast machine only); expected where it shows, not stale where it does not |

Write each field as a basic `"..."` string (or `true`), one per line: `scripts/ledger_refs.py` reads
`category` and `issue` by pattern, not with a TOML parser.

Adding an entry adds a file and deleting one deletes its file, so two PRs that touch different
entries never touch the same lines. Two PRs that ledger the same probe produce the same path and
still conflict, which is the right result.

## Notes carried over from the single file

- G: colour and locale probes (#5552) were found with the per-probe `env` column; none was
  tracked yet, so they are `to-triage`.
- `strptime`/`strftime`: libc in jq (locale-dependent, `%B` strict, weekday from the unset date
  fields); a fixed English implementation here.
