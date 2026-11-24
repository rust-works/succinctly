# Known differences from the reference jq

The ledger of `succinctly dev compare jq` (#5409), one entry per file (#5769). The run fails on a
difference that is not listed here and on a listed one that no longer differs.

An entry is `<suite>/<probe>.toml`. Its id is the path: the directory is the suite, the file name
is the probe, and the `/` before an arity is written `.`:

| Id                      | File                               |
|-------------------------|------------------------------------|
| `cli:version`           | `cli/version.toml`                 |
| `features:strftime/1`   | `features/strftime.1.toml`         |
| `upstream-jq.test:1942` | `upstream-jq.test/1942.toml`       |

A suite and a probe use only letters, digits, `_` and `-`, and a suite also a `.` after its first
character: the differential suites are named after their test file (`upstream-jq.test`), and the
id of a case is its line there. A file that no id maps to, a file that
is not `.toml`, and a `divergences.toml` beside this directory fail the loader
(`src/bin/succinctly/compare/ledger.rs`).

The `fuzz` suite is the exception in what the probe name is: a fuzzer finding is a program, which a
file name cannot hold, so its probe is `p-` and the first 12 hex digits of the SHA-256 of the
minimized program (`fuzz:p-cb983ce3375e` is `fuzz/p-cb983ce3375e.toml`). The report prints the id
of every finding, and the entry's `reason` names the program.

The file holds the other fields, with no `[[divergence]]` header and no `id`:

| Field      | Meaning                                                                                       |
|------------|-----------------------------------------------------------------------------------------------|
| `category` | `by-design`, `host-dependent`, `newer-than-primary`, `reference-crash`, `reference-slow` or `to-triage` (a real difference nobody has yet classified against ADR-0018 or filed) |
| `reason`   | what differs and why                                                                          |
| `issue`    | optional tracking issue. A branch that references a `to-triage` entry's issue deletes the entry in the same change (`scripts/ledger_refs.py`, #5585); one that only mentions it says `[ledger keeps #N]` on its own line in a commit message |
| `platform` | optional `std::env::consts::OS` the entry was observed on; other OSes neither expect nor call it stale |
| `volatile` | optional `true`: whether the difference shows depends on the host, on its speed (a probe the reference finishes inside the 10 s deadline on a fast machine only), on its address-space footprint (a probe just under a `ulimit -v` cap) or on which locales it has installed (`locale-strptime-ampm-de-de`: a host with `de_DE` matches, one without falls back to `C` and differs); expected where it shows, not stale where it does not |
| `subject_class` | optional, `robust` entries only: the outcome class the entry accepts for succinctly, see below |
| `reference_class` | optional, `robust` entries only: the same for the reference jq                             |

Write each field as a basic `"..."` string (or `true`), one per line: `scripts/ledger_refs.py` reads
`category` and `issue` by pattern, not with a TOML parser. The two class fields may also be a list
or an inline table, still on one line.

## Outcome classes of a robustness entry

An id says that a row differs, not how each tool ends, so a row can change under an unchanged
entry: `mem-limit-64m-string` went from an exit status to death by a signal between two runs and
nothing reported it (#7891). A `robust` entry can therefore state the class it accepts for each
tool, in the words the robustness dimension uses (`ok`, `error` for a non-zero exit, `crash` for
a signal or a panic, `timeout` for the 10 s deadline):

```toml
subject_class = "error"                                # one class
subject_class = ["error", "crash"]                     # either: it depends on the build or the host
subject_class = { linux = "timeout", macos = "error" } # per `std::env::consts::OS`
reference_class = "crash"
```

- The fields are optional and independent. Without one, that tool's class is not checked, as
  before the fields existed.
- A row is judged when the run observes it as diverging. A tool that ends in a class the entry
  does not accept fails the run like an unlisted divergence, naming the row, the class observed
  and the classes accepted, and the report says so in the row's ledger column:

  ```text
  listed with another outcome class: robust:mem-limit-64m-string: subject class `crash`, expected `error`
  ```
- A list is for an outcome that legitimately varies, and the `reason` says why. A table states a
  class per OS where the platforms differ; an OS it does not name is not checked there, which is
  how a platform nobody has observed is left out. Each value of a table is a class or a list.
- The loader refuses a name that is not a class, an empty list or table, a class listed twice, a
  key that is not an OS, an OS other than the entry's own `platform`, and either field on an
  entry outside `robust`.
- State `reference_class` when the entry's meaning depends on how jq ends (`reference-crash`,
  `reference-slow`, or jq answering where succinctly does not).
- A `volatile` entry is judged only on a run where its difference shows.

Adding an entry adds a file and deleting one deletes its file, so two PRs that touch different
entries never touch the same lines. Two PRs that ledger the same probe produce the same path and
still conflict, which is the right result.

## Notes carried over from the single file

- G: colour and locale probes (#5552) were found with the per-probe `env` column; none was
  tracked yet, so they are `to-triage`.
- `strptime`/`strftime`: libc in jq (locale-dependent, `%B` strict, weekday from the unset date
  fields); a fixed English implementation here.
