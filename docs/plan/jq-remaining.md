# Plan: jq Remaining Work

**Status: Archived (done).** Every item this page listed was re-checked against `/usr/bin/jq`
1.7.1 on 2026-11-02 (#5847) and matches, or is not a jq feature at all:

| Item                                                  | Result                                                                                                       |
|-------------------------------------------------------|--------------------------------------------------------------------------------------------------------------|
| `--argjson`, `--slurpfile`, `--rawfile`, `--jsonargs` | Implemented; output identical to jq                                                                          |
| `module {...}` metadata                               | Used: `modulemeta` returns the same `name`/`version`/`deps`/`defs` object as jq                              |
| `$__PROGDIR__`                                        | Not a jq 1.7.1 variable: both report `$__PROGDIR__ is not defined` as a compile error                        |
| Module-relative paths in `import`/`include`           | Same as jq on a nested `import`, a data import and a `./` import from inside a module (both fail to find it) |

Divergences that remain are recorded in
[limitations.md](../compliance/jq/limitations.md), the open module-scope gaps under
[Module-scope gaps that are genuinely open](../compliance/jq/limitations.md#module-scope-gaps-that-are-genuinely-open).
See [jq Language Reference](../reference/jq-language.md) for the implemented features.
