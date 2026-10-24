# Phase 0 of #5223: a path-mode interpreter that carries jq's register

**Status: implemented.** This note is the go/no-go report the
[#5223](https://github.com/rust-works/succinctly/issues/5223) design asked for before any
replacement work started; its verdict was GO. Phases 1 to 3 (identity preconditions, shadow
mode, switch) have since landed: the spike is now `src/jq/path_mode.rs`, and a jq-mode program
with a dynamic path target runs on it ([ADR-0026](../adrs/adr-0026.md)). The sections below
describe Phase 0 as it was built and measured; later changes (native C builtins, writes, the
routing rule) are on the issue and in the ADR.

## What was built

`src/jq/path_spike.rs` is an interpreter over the parsed `Expr`. It **carries** jq 1.7.1's
register (`path`, `value_at_path`, `subexp_nest`) with every output and applies `jv_identical`
at jq's three check sites: `INDEX`/`EACH`, `getpath`'s `_jq_path_append`, and `PATH_END`. It
infers nothing.

- **Evaluation** is push style. `eval(expr, env, input, register, sink)` calls the sink once per
  output with that output's own register. A generator resumed for its next output therefore
  continues from its own register, which is what jq's fork-point restore amounts to.
- **Values** are a spike-local `SVal` with `jv` identity:
  - arrays, objects, strings and literal numbers compare by pointer;
  - `null`/`true`/`false` compare by kind;
  - computed numbers compare by their `f64` bits;
  - each constant site is one value, as `LOADK` hands out (including the arrays and objects
    `gen_const_array`/`gen_const_object` fold, and `constant_fold`'s results);
  - array slices are views, as `jvp_array_slice` makes them: a whole-range slice is the array
    itself, and equal ranges of one array are one value.

  This makes identity exact inside the spike without touching `OwnedValue`'s promotion rules,
  which are D5 and Phase 1.
- **Construct arms** follow the bytecode `compile.c` emits for each one. The doc comment on each
  arm cites the layout. The rules the sweeps leaned on hardest:
  - subexps for `if` conditions, bind sources, index keys (evaluated before the target, on the
    target's input), object entries and every C-builtin argument (the right operand outermost);
  - `INDEX_OPT`/`EACH_OPT` still raising a `path_intact` failure, where `(E)?` is `try`;
  - `TRY_BEGIN`/`TRY_END`: a `try` handles only errors raised in its body, and an error raised
    downstream passes through wrapped;
  - a `break` is an error a `try` catches;
  - `?//` (`DESTRUCTURE_ALT`) retries on any error raised after it, downstream included;
  - the fold layouts:
    - `INIT` is tracked;
    - `SOURCE` starts at the register `INIT` left;
    - an empty `UPDATE` leaves the state `null` (`LOADVN`);
    - `EXTRACT` runs straight after `UPDATE`;
    - `reduce`'s result carries the register after `INIT`;
  - `DUPN`: every `INIT` output after the first runs `SOURCE` on `null`, a jq quirk that value
    mode already reproduces;
  - array patterns `INDEX` their last element first.
- **jq-defined builtins** run their 1.7.1 `builtin.jq` bodies from a prelude. Their private C
  helpers (`_sort_by_impl`, `_group_by_impl`, `_min_by_impl`, `_max_by_impl`, `_strindices`)
  are implemented natively, and the helpers that return input elements keep those elements'
  identity.
- **C builtins** are delegated to the shipped evaluator and answer a fresh value, as the issue
  specifies for Phase 0. A run records which stubbed builtins jq could have answered
  differently on identity, and the dev entry prints them as `path-spike: stubbed: ...`. They
  are:
  - C builtins that can hand back their input: `ltrimstr`/`rtrimstr` with no match,
    `tostring`, `tonumber`, `@text`, `delpaths([])`, `setpath([]; v)`, `min`/`max` and `sort`;
  - jq-defined builtins whose bodies navigate and which the prelude does not run yet (`match`,
    `splits`, `sub`, `transpose`, `tostream`, ...).
- **Compile errors**: `resolve_all_jq`, the check `succinctly jq` itself runs, rejects what jq
  rejects at compile time.

Errors are jq's three path messages, raised where jq raises them and catchable as jq's are. A
construct the spike does not implement raises `Unsupported`, which nothing catches. Such a row
is counted apart and is never read as a verdict.

The dev entry is a hidden `succinctly dev jq-path-spike`. `succinctly jq` does not change.

## How it was measured

`scripts/jq-path-spike-3way.py` replays rows through `/usr/bin/jq` 1.7.1 (the oracle), a release
build of `main` (see the results for the commit), and a release build of the spike at the commit this PR lands. It
compares stdout and exit code per row, as the sweeps do. The classes:

| class        | meaning                                                                  |
|--------------|--------------------------------------------------------------------------|
| both         | `main` and the spike match jq                                            |
| gain         | only the spike matches jq                                                |
| LOSS         | only `main` matches jq, on a run that touched no stub: fails the gate    |
| LOSS-stub    | only `main` matches jq, on a run that answered a builtin through the stub |
| neither      | neither matches jq                                                       |
| neither-stub | neither matches jq, on a stubbed run                                     |
| unsupported  | the spike declined the row                                               |

The rows come from two sources:

1. **`scripts/jq-path-spike-rows/repros.jsonl`**: every live probe in #5223 and every repro in
   the issues it supersedes (#4800, #5013, #5030, #5042, #5044, #5101, #5113, #5156, #5175,
   #5181, #5194, #5203, #5205, #5207 and #5213), 73 rows. Every row of every superseded issue
   matches jq. The one `LOSS-stub` is the issue's own `ltrimstr("z")` probe.
2. **Every sweep the issue lists**, captured unchanged. Each sweep ran once with its `--bin`
   pointed at `scripts/jq-path-spike-record.py`, a shim that logs each `(argv, stdin)` and
   answers with `main`. Two grids are too large for a gate, so only part of each was taken:
   - `jq-path-register-sweep.py`'s full grid is about 10M rows; it ran with
     `--stage-only --sample 30000 --seed 5223`;
   - two grids are prefixes cut off at a time cap: `jq-fold-body-register-grid-sweep.py`
     (1,215,396 rows) at its first 55,091 distinct rows, and `jq-fold-update-alt-bind-sweep.py`
     at its first 16,762.

   Every other sweep ran in full, at its default size and seed.

## Results

Final pass: `main` at `0bfdb0d56` and the spike at this branch's head, both release builds,
on an Apple M4 Pro running macOS against `/usr/bin/jq` 1.7.1. Rows are distinct
`(argv, stdin)` pairs.

| source                                      |    rows |    both |   gain | LOSS | LOSS-stub | neither | neither-stub | unsupported |
|---------------------------------------------|--------:|--------:|-------:|-----:|----------:|--------:|-------------:|------------:|
| issue repros (`repros.jsonl`)               |      73 |      51 |     21 |    0 |         1 |       0 |            0 |           0 |
| `jq-path-register-sweep` (sample)           |  30,024 |  28,406 |  1,557 |    0 |        14 |       0 |            1 |          46 |
| `jq-fold-source-register-sweep`             | 112,320 | 112,224 |      0 |    0 |        96 |       0 |            0 |           0 |
| `jq-fold-body-register-grid-sweep` (prefix) |  55,091 |  52,524 |  2,567 |    0 |         0 |       0 |            0 |           0 |
| `jq-nested-fold-source-sweep`               |   5,999 |   5,691 |    308 |    0 |         0 |       0 |            0 |           0 |
| `jq-nested-fold-frozen-var-sweep`           |   2,673 |   2,451 |    222 |    0 |         0 |       0 |            0 |           0 |
| `jq-nested-fold-outer-var-sweep`            |  19,080 |  17,996 |  1,084 |    0 |         0 |       0 |            0 |           0 |
| `jq-foreach-update-state-register-sweep`    |   6,000 |   5,163 |    837 |    0 |         0 |       0 |            0 |           0 |
| `jq-foreach-update-if-arm-sweep`            |  40,000 |  36,185 |  3,815 |    0 |         0 |       0 |            0 |           0 |
| `jq-fold-update-alt-bind-sweep` (prefix)    |  16,762 |  16,533 |    229 |    0 |         0 |       0 |            0 |           0 |
| `jq-reduce-update-bind-sweep`               | 108,864 | 108,474 |    390 |    0 |         0 |       0 |            0 |           0 |
| `jq-fold-source-alt-chain-sweep`            |   4,320 |   4,320 |      0 |    0 |         0 |       0 |            0 |           0 |
| `jq-alt-passthrough-sweep`                  |  76,230 |  57,607 | 18,623 |    0 |         0 |       0 |            0 |           0 |
| **total**                                   | 477,436 | 447,625 | 29,653 |    0 |       111 |       0 |            1 |          46 |

The stubbed builtins named on the 112 `-stub` rows:

- `tostring` (96 rows): of a string, which `f_tostring` returns unchanged;
- `delpaths([])` (12 rows), alone or with `setpath`, `min` or `ltrimstr`;
- `setpath([]; v)` (3 rows);
- `ltrimstr` with no match (1 row).

All 46 `unsupported` rows read input (`input`, `input_line_number`). The spike has no input
queue.

## What the losses were, and what changed because of them

The first runs showed four classes of `LOSS`. Each one traced to the spike's machinery, not to
a construct rule, and each is now fixed or attributed:

1. **A 1.8 rule read as 1.7.1.** The first draft renormalized a negative index into the path,
   as jq 1.8 does. In 1.7.1 `[1,2,3] | path(.[-1])` is `[-1]`. Fixed from `execute.c`'s
   `INDEX`.
2. **Slice identity.** A whole-range slice is the array itself in jq (`jvp_array_slice` keeps
   the pointer and sets `offset`/`size`). The spike's fresh slices lost 9 fold-source rows, for
   example `[1,2] | path(reduce .[0:2] as $k (.; $k))`, which is `[]` in jq. This is an identity
   rule (D5's class), now modelled as views.
3. **Compile errors.** An unbound `$a` is a compile error in jq (exit 3); the spike met it only
   if evaluation reached it (262 rows of the reduce-UPDATE grid). It now runs `resolve_all_jq`
   first.
4. **The C-builtin stub.** These rows go through a builtin jq lets hand back its input
   (`delpaths([])`, `setpath([]; .)`, `ltrimstr("z")`, ...) or through a jq-defined builtin
   the prelude did not run yet (`index`, `rindex`, `indices`, `unique_by`, `min_by`). The
   jq-defined ones now run from `builtin.jq`. The C ones are the `LOSS-stub` rows above, which
   are Phase 1's table (D4).

The code review of this branch found more of the same kind, in the dev entry and the
spike's own builtins: input validation and multi-document input, `range` with a NaN bound, an
`INDEX_OPT` that swallowed `Unsupported`, and builtins that read input state. Each is fixed
or now declined as unsupported.

One issue repro also disagreed with both builds at first:
`path(. as $x | foreach .a as $i ((.,.); U; $i))`. That row is `DUPN`. A fold's later `INIT`
outputs run `SOURCE` on `null`, so jq raises at `.a` on the second `INIT`. Once the spike
modelled `DUPN`, the row matched jq.

## Verdict: GO

The gate in #5223:

- **"Go if the spike loses no row the current resolver matches"**: no row lost outside the
  stub, across every source above (the `LOSS` column).
- **"... and matches the residual repros"**: every repro of every superseded issue matches jq
  1.7.1 on stdout and exit code.
- **"Rows it gets wrong must trace to D5 identity gaps or to unstubbed builtins"**: every
  remaining `LOSS-stub`/`neither-stub` row is on a stubbed run.
- **"No-go if any construct rule from the table disagrees with jq"**: none did. The issue's
  layout table needed two refinements that `compile.c` states and the issue's prose
  simplifies:
  - `reduce`'s result carries the register after `INIT`, not at `reduce` entry. The two
    differ when `INIT` navigates: `path(reduce 1 as $x (.a; .))` is `["a"]`.
  - `DUPN` gives every later `INIT` output a `null` source input.

## What Phase 1 and 2 inherit

- **The C-builtin table (D4)** is what the `LOSS-stub` rows measure. Each row there names its
  builtins on stderr. One oracle probe per builtin closes them.
- **The prelude** already runs 74 definitions: `builtin.jq`'s, plus the six `_sp_upd_*`
  helpers for the update-assignment operators. The jq-defined builtins still
  stubbed are listed in `stub_risk` (`src/jq/path_spike.rs`).
- **Identity on `OwnedValue` (D5)**: the spike's `SVal` is the reference Phase 1's promotion
  has to reproduce. It covers constants per site, literal versus computed numbers, and slices
  as views.
- **Not measured here**: performance, stack depth, and the `eval_generic.rs` route's doors. The
  spike is a separate evaluator with its own value type, so none of them applies to it yet.
