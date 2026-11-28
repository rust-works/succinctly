# The jq regex corpus

[Home](../../) > [Docs](../) > [Guides](./) > The jq regex corpus

The jq regex corpus is a differential corpus for jq-mode regular expressions.
It is the gate of [ADR-0031](../adrs/adr-0031.md)'s migration onto Oniguruma (#7965): a change to the regex engine or its adapter is judged by how its answers move against jq 1.7.1 and jq 1.8.2, row by row.
The tool is [`scripts/jq-regex-corpus.py`](../../scripts/jq-regex-corpus.py); its docstring is the reference for every subcommand and option.

## What a row is

A row is one regex builtin on one pattern and flags string, evaluated on three subject strings.
The builtins are `test`, `match`, `match` with `g`, a capture summary, `capture`, `scan`, `sub`, `gsub`, `splits` and `split`.
The generators cover look-behind bodies, references and calls (defined and undefined), malformed pieces, classes, intervals, escapes, option runs, comments and blanks, a general mix, ordinary patterns (`plain`), and 133 probe patterns taken from a reading of the old text passes.
12% of the generated patterns are run four ways (no flags, `x`, `i`, `g`).

Those 459,678 rows (seed 2026, 330,133 distinct patterns, 45 MB in chunks 0 to 91) are the corpus ADR-0031's measurements used.
After them, in chunks 92 to 99 of their own, come 40,000 literal rows: literals and alternations of literals, which jq mode searches without Oniguruma (ADR-0031 step 5), and near-literals one metacharacter, empty alternative or flag away from one, which must still reach Oniguruma.
Only 155 of the 459,678 rows reached that path, so a change to it was not otherwise checked.
The literal rows have their own random stream, so the chunks before them, and every answer stored for those chunks, stay valid; `gen --literal 0` leaves them out.
The corpus is regenerated in about five seconds, not committed.
`scripts/test_jq_regex_corpus.py` pins both digests: stored answers are comparable only while the rows stay the same.

## Running it

```bash
python3 scripts/jq-regex-corpus.py gen corpus
python3 scripts/jq-regex-corpus.py run jq171 /usr/bin/jq corpus res
python3 scripts/jq-regex-corpus.py run jq182 path/to/jq-1.8.2 corpus res
python3 scripts/jq-regex-corpus.py run main target/release/succinctly corpus res
python3 scripts/jq-regex-corpus.py run main target/release/succinctly corpus res --standalone 40
python3 scripts/jq-regex-corpus.py analyze corpus res main
python3 scripts/jq-regex-corpus.py validate res main
```

- `/usr/bin/jq` is the pinned 1.7.1 on the Mac; a Linux distribution's `jq` may be 1.6, so check `--version`.
  The `dev compare jq` harness builds both pinned releases under `target/compare/jq-1.7.1/bin/jq` and `target/compare/jq-1.8.2/bin/jq`.
- Each binary answers a chunk in one process per 400 rows, each row under `try`/`catch`, so a run is about 1,150 processes.
  A row that ends its process (a crash, 10 s without output, or 2 GiB of RSS) is recorded as `CRASH`, `TIMEOUT` or `RSS`, and the batch resumes after it.
- `run` is resumable per chunk, and exits 7 when `--budget` (seconds, default 480) ran out first: run it again to continue.
  This keeps each hold of a shared box short.
- `--standalone K` runs every K-th row as its own process, with the pattern as a literal in the program and no `try`.
  `validate` then checks that the batched answer equals that run's stdout, stderr and exit status.
- `--env K=V` passes a variable to the binary, for a build that chooses its engine at run time.
  A `cli` build since ADR-0031's step 6 has one engine, the vendored Oniguruma; the `SUCCINCTLY_REGEX_ONIG` switch of steps 3 and 4 is gone.

On an M4 Pro with four workers, a release `succinctly` build took 11 seconds for the batched run and 10 seconds for `--standalone 40` (2026-11-27).

## Reading the result

`analyze <candidate>` compares the candidate with the labels `jq171` and `jq182`.
A row is `same171` when the candidate's answer equals jq 1.7.1's, `follows182` when it equals jq 1.8.2's instead, and `differs` otherwise.
Differing rows are sorted into families, from the kind of disagreement (who refused, which error, which feature of the pattern).
With `--base <label>`, it also says which of the base's differing rows were resolved, left unchanged or changed, and which rows regressed.

**The corpus gate (#7965):** the rows equal to jq 1.7.1 do not fall, every changed row is listed by family in the PR, and no new family appears.

`versions` sorts the 7,361 rows where jq 1.7.1 and 1.8.2 differ, and says whom each candidate follows on them.
There are three causes:

- Oniguruma 6.9.10 accepts `[\d-a]` (upstream #298), about 4,800 rows.
- jq 1.8 reports a group that took no part in an empty match as `offset: -1` (jqlang/jq#3238; adopted, #7520), about 1,420 rows.
- jq 1.8 orders the letters of `split`'s modifier error differently (1.7.1 kept, #7560), about 1,100 rows.

## Baseline

Measured on macOS arm64 with release `--features cli` builds, on 2026-11-24 (main and the C prototypes) and 2026-11-27 (ferroni, and the vendored pin under `SUCCINCTLY_REGEX_ONIG=1`):

| Build                                                           | equal to jq 1.7.1 | follow jq 1.8.2 | differ from both |
|-----------------------------------------------------------------|-------------------|-----------------|------------------|
| main `cd718f9a6`                                                | 408,312           | 1,725           | 49,641           |
| C Oniguruma 6.9.9 (prototype through the `onig` crate)          | 458,156           | 1,411           | 111              |
| C Oniguruma 6.9.10                                              | 453,292           | 6,273           | 113              |
| ferroni 2.9.0, the two post-pin syntax flags of Perl_NG cleared | 451,449           | 1,394           | 6,835            |
| C Oniguruma at the pin `d2f1a14ced`, with the `f_match` port    | 458,259           | 1,419           | 0                |

The standalone control agrees with the batch on all 11,492 sampled rows for both jq builds and each candidate.
The pin's 1,419 rows that follow jq 1.8.2 are all the adopted empty-match captures (#7520); the prototype's 111 were jq's byte step after an empty match, which the port reproduces (#7965).
On main it agrees on 11,487: four are error texts longer than the driver keeps, and one batch was stopped by the RSS cap.
