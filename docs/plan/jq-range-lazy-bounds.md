# Lazy bound resolution for `range()` (#1556)

[Home](../../) > [Docs](../) > [Plan](./) > Range lazy bounds

**Status: implemented.** This document is the
deliverable for [#1556](https://github.com/rust-works/succinctly/issues/1556),
whose own tier review classified it Tier 3 — "changes evaluation shape rather
than fixing a local mistake... wants a design doc first" — and which a prior
agent explicitly declined to implement without one, per that same reasoning.

## Problem

`eval_range` (`src/jq/eval.rs`) resolves `range`'s `from`/`to`/`step` bound
expressions eagerly, via three nested `stream_outputs(eval_single(...))`
loops — one per bound, `from` outer, `to` middle, `step` inner. `eval_each`,
the demand-driven dispatch table [#820](https://github.com/rust-works/succinctly/issues/820)
and [jq-lazy-generator-consumers.md](jq-lazy-generator-consumers.md) built so
a wrapping consumer (`first`, `limit(1;...)`, `isempty`, `path(...)`) can stop
a generator argument mid-stream, has no `Expr::Range` arm, so it falls to the
catch-all `_ => drain_result(eval_single(expr, value, optional), sink)`. That
fallback requires `eval_single` to run to completion — meaning the *entire*
triple-nested eager bound resolution — before the sink ever sees a value.

When a bound is itself a multi-output generator, this means later comma
branches get evaluated even when an earlier one already satisfies the
consumer — including branches with side effects (`input`, `stderr`,
`halt_error`) that real jq's lazy generator semantics would never reach:

```
$ printf '5\n99\n' | jq -c 'first(range(1, input)), input'
0
99
$ printf '5\n99\n' | succinctly jq -c 'first(range(1, input)), input'
jq: error (at <stdin>:2): break
0
```

`first(range(1, input))`'s bound is `(1, input)`. Its first branch (`1`)
already produces `range(0;1) = [0]`, a non-empty result, so real jq's
`label $out | (f, break $out)` desugaring for `first` never asks the
generator for a second value — `input` is never evaluated, and the trailing
`, input` at the top level reads the second document (`99`) cleanly.
succinctly's eager `stream_outputs` call drains `(1, input)` in full before
`eval_range` produces anything, consuming the second document as a side
effect of computing a bound the caller never needed past its first value.
That leaves the top-level `input` queue empty, so the trailing `, input`
raises the same `break` exhaustion error real jq's own `input` raises when
the queue genuinely runs dry (`eval.rs`'s `builtin_input`) — misleadingly,
since here the queue was emptied by an evaluation-order bug, not by the
program's own request.

## Root cause: confirmed single-evaluator scope

This codebase has two evaluators — `src/jq/eval.rs` (the `QueryResult`/
`StandardJson` cursor-based one) and `src/jq/eval_generic.rs` (the generic,
YAML-capable one) — and a fix that only reaches one of them is a documented
recurring trap ([#1054](https://github.com/rust-works/succinctly/issues/1054)
missed a reindex-bridge fix this same way). Traced end-to-end for this issue:

- `eval_generic.rs` has **zero** `Expr::Range` handling anywhere — no native
  arm, no separate range implementation. Every `Range` node it encounters
  bridges into `eval.rs`'s `eval_single`/`eval_range` via `eval_on_owned` or
  the top-level `input`-queue bridge (`takes_input_queue_bridge`,
  `eval_with_cursor_using`).
- `eval_generic.rs`'s own demand-driven mechanism, `eval_each_generic`, is
  explicitly scoped (by its own doc comment) to only `Comma`/`Pipe`/`Paren`
  — it was never meant to mirror `eval.rs`'s wider arm set, and needs no
  change here.
- For the exact CLI repro above, tracing the call chain confirms
  `eval.rs`'s `eval_each` is the evaluator that actually runs:
  `eval_with_cursor` → (`input`-queue bridge, since the query uses `input`)
  → `eval_each_owned_collect` → `eval::eval_each_owned` →
  `eval::eval_each` (`Comma` arm) → `eval_each` on `FirstExpr(Range)` →
  catch-all → `eval_first_expr` → `each_take_first` → `eval_each` on
  `Range` → catch-all → `eval_range` (eager). `eval_generic.rs`'s own
  `each_take_first_generic`/`eval_each_generic` are never entered.

So this is not a repeat of #1054's shape: there is exactly one `range`
implementation to fix, and it lives entirely in `src/jq/eval.rs`.

## The two questions this design settles

The issue asked for a design pass to settle two things before any code is
written.

### 1. Reuse `fanout_two_args_lazy`'s shape, not the function

[`fanout_two_args_lazy`](../../src/jq/eval.rs) (#1531,
[jq-generator-argument-fanout.md](jq-generator-argument-fanout.md)) already
drives two generator arguments through `eval_each`, nested, with a shared
`escape: Option<Control>` variable capturing whichever control should
terminate the whole thing. It is the closest existing precedent for
"multiple nested lazy pulls, one escape". But it cannot be reused directly:
it is hardcoded to exactly two arguments, and it returns a `QueryResult`
rather than a `Flow` — it is itself an ordinary builtin-body helper called
from `eval_single`-reachable code, not a dispatch-table arm. Only inserting
an actual `Expr::Range` arm into `eval_each`'s match closes the gap, and
that arm must be `Flow`/`Demand`-shaped.

`range` needs **three** nested pulls (`from` outer, `to` middle, `step`
inner — jq's own leftmost-outermost `$`-bound order for its `def range($from;
$upto; $by)`, already established correctly by the current eager code's
oracle-verified doc comment, unaffected by this change). The answer: a new
function, `each_range`, built on `fanout_two_args_lazy`'s nesting/escape
pattern one level deeper. It stays a bespoke function rather than a
generalized N-argument helper, for the same reason `eval_range` itself is
bespoke today
([Stage 6](jq-generator-argument-fanout.md#L23), verbatim): *"the three
arities differ enough that a generic 3-argument helper would be all glue."*

### 2. What the new arm owes the dispatch table: the fallback invariant, satisfied by construction

[jq-lazy-generator-consumers.md](jq-lazy-generator-consumers.md#the-fallback-invariant-state-it-then-test-it)
states the obligation every dispatch-table arm has: *for a sink that always
returns `Demand::Continue`, `eval_each` must deliver exactly the values
`push_owned_values(eval_single(...), ...)` would collect, in the same order,
with the same terminal `Control`. A lazy arm can only shrink which
sub-expressions get evaluated — never change which values are delivered or
their order.*

Rather than writing `each_range` and keeping `eval_range`'s existing
triple-nested-loop body as two independently-maintained implementations that
could silently drift (the recurring lesson behind
[#106](https://github.com/rust-works/succinctly/issues/106): "duplicated
predicates diverge silently"), `eval_range` becomes a **thin wrapper** over
`each_range`: it drives `each_range` with a sink that always answers
`Demand::Continue`, collecting into a `Vec`, then converts the terminal
`Flow` back into a `QueryResult`. This is exactly the relationship
`fanout_two_args` already has with `fanout_two_args_lazy`. The invariant
holds by construction — there is one implementation of bound resolution and
value generation, not two.

## Mechanism

```rust
fn each_range<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    from: &Expr,
    to: Option<&Expr>,
    step: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
    sink: &mut dyn FnMut(Item<'a, W>) -> Demand,
) -> Flow
```

Drives `from` through `eval_each` (outer closure); per from-item, drives `to`
through `eval_each` (middle closure) — except the structurally unreachable
`to: None` case (the parser always desugars `range(n)` to
`Range { from: Literal(0), to: Some(n), step: None }`; `to: None` is kept in
lockstep with `eval_range`'s existing dead branch rather than deleted, per
that branch's own comment about the `Option` field rippling through six
rebuild sites for no behavioural gain). Per to-item, if `step` is `Some`,
drives it through `eval_each` (inner closure); if `step` is `None`, uses the
literal default `1` directly with no evaluation — matching current behavior,
since there is nothing to pull.

An `emit` closure turns each fully-resolved `(from_val, to_val, step_val)`
triple into its value sequence via the existing, unmodified
`eval_range_values`/`eval_range_values_f64`, and forwards it to `sink` via
`drain_result(one, sink)` directly rather than a hand-rolled loop.
`owned_vec_to_result`'s three-variant range (`None`/`Owned`/`ManyOwned`)
means `drain_result` can only answer `Exhausted` or `Stopped { pending: None
}` here, so its `Escaped` arm is `unreachable!()` — the same defensive
pattern `drain_result` itself uses for `QueryResult::OneCursor`.

### Two ways to stop, tracked out-of-band

The driving closures can only answer `Demand`, so two `bool`/`Option`
locals in `each_range` carry *why* a stop happened, checked in priority
order once the outer `eval_each(from, ...)` call returns:

- **`sink_stopped: bool`** — set only when the innermost value-emission call
  reports back that the *wrapping* `sink` returned `Demand::Stop`: the
  consumer has what it wants, and every remaining bound value at every
  nesting level is left unevaluated. Collapses to
  `Flow::Stopped { pending: None }`.
- **`escape: Option<Control>`** — set when a bound value fails
  [`range_num`]'s numeric check (Rule 3 below), or when a nested
  `eval_each(to_expr/step_expr, ...)` call itself returns `Flow::Escaped`
  (Rule 4 below — some control, e.g. a `halt`, surfaced from inside a
  bound's own generator). Collapses to `Flow::Escaped(control)`.

These are mutually exclusive by construction: `sink_stopped` is set only
inside `emit`, and once `escape` is set a closure always returns
`Demand::Stop` immediately without reaching `emit` again.

### Dropping `pending`, and why that's the right precedent

Whenever a nested `eval_each(to_expr, ...)` or `eval_each(step_expr, ...)`
call returns `Flow::Stopped { pending }`, the enclosing closure returns
`Demand::Stop` via `Flow::Stopped { .. } => Demand::Stop` — **without
reading `pending`**, at every nesting level. `Flow`'s own doc comment
records that every dispatch-table arm added so far drops `pending` on its
own local stop except `binary_fanout_each` (which has two operands and must
pick one — not applicable here, `each_range` has one linear escape chain).

The precedent to cite for this is **`fanout_two_args_lazy`**, not
`each_limit`. `each_limit` wraps only *one* level of `eval_each` with the
*ultimate* sink passed straight through, so it must stay transparent to its
own caller and keeps `pending` verbatim. `fanout_two_args_lazy` is the one
other function in this file that nests multiple `eval_each` calls under a
shared `escape`, one level shallower than `each_range` needs — and it
already drops `pending` unconditionally at both of its own nesting levels.
(An earlier draft of this design cited `each_limit`; that was corrected
during review.)

### Confirmed no `escape.is_some()` guard is needed

A closure never needs to check `escape.is_some()` before returning — it is
provably redundant. `escape` can only be set either directly inside a
closure immediately before that closure returns `Demand::Stop` (no further
code in it runs), or via a `match nested_flow { Flow::Escaped(c) => ... }`
arm, which by construction only fires when the nested call's own `Flow` is
genuinely `Escaped` — which itself only happens when *no* deeper closure
decided to stop it (a closure-caused stop always yields `Flow::Stopped`,
never `Escaped`). So `escape` being set by a deeper frame always surfaces as
`Flow::Stopped` one level up, which the `Stopped => Demand::Stop` arm
already handles without consulting `escape` again.

## Behaviour, oracle-verified

Rules 3 and 4 are the two escape-priority rules already established (and
tested) for the *eager* `eval_range`, from
[jq-generator-argument-fanout.md](jq-generator-argument-fanout.md)'s rules 3
and 4, restated here because `each_range` must reproduce them exactly:

| Rule | Query | Result |
|---|---|---|
| 3 — failure aborts the loop, keeps the prefix | `jq -cn 'range((1,"x"))'` | `0` on stdout, then `Range bounds must be numeric`, exit 5 |
| 4 — the builtin's own error can outrank a trailing halt | `jq -cn 'range((1, halt_error(4)); 10)'` | `1` through `9` on stdout, exit 4 |

Both hand-traced through `each_range`'s proposed control flow and confirmed
to reproduce identically (see `git log`/PR discussion for the full trace);
both already pass on the current eager binary, so neither regresses.

**A third, previously-undetected divergence, found while validating this
design and fixed as a side effect** — `path()` around a `range` whose `to`
bound contains a `halt_error` behind values `path()` never needs:

```
$ jq -cn 'path(range(1; (2,3,halt_error(9)) + 0))'
jq: error (at <unknown>): Invalid path expression with result 1   (exit 5)
$ succinctly jq -cn 'path(range(1; (2,3,halt_error(9)) + 0))'
(exit 9 — halted, no output)
```

Today, `eval_range`'s eager `stream_outputs` on the `to` bound fully
materializes `(2,3,halt_error(9)) + 0` up front as
`QueryResult::Partial([2,3], Halt(9))`, and that `Partial` reaches
`resolve_leaf`'s stop-after-first sink directly — one nesting level away —
which is exactly the shallow shape `resolve_leaf`'s halt-preserving check
(`eval.rs`, the one path-context consumer that reads `pending` to avoid
downgrading an already-triggered halt into a catchable path error) is
designed for, so it fires the halt. Real jq's `path()` is satisfied by the
very first candidate (`1`) and never asks the `to` generator for a second
value, so `halt_error(9)` is never reached and jq raises its own path error
instead. With `each_range`, the same `Partial` is consumed and its
`pending` dropped *inside* the `to`-level `eval_each` call — three closures
away from `resolve_leaf` — matching real jq's laziness. `resolve_leaf`'s
own doc comment already states the general principle this now correctly
extends to `range`'s bounds (*"`path(range(3))` raises on `0` alone, never
reaching `1`/`2`"*).

This is a genuine, user-visible exit-code change (9 → 5) on an existing
construct, in the direction of correctness — recorded here explicitly
rather than left to be discovered later, and pinned as its own regression
test.

## Verification

- `eval.rs`'s existing `collect_each` differential-invariant corpus gets new
  rows: `range(3)`, `range(1;5)`, `range(1;5;2)`, a multi-output-bound row
  (`range((0,1);(2,3);(1,2))`, matching `eval_range`'s own oracle table),
  and `range` nested inside `limit`/`if`/`try`.
- New `tests/jq_cli_tests.rs` cases, via `run_jq_full` (not the
  `cargo run`-based stdin helper, which builds an uninstrumented binary
  invisible to coverage):
  - The headline repro, modeled on
    `test_compare_operand_consuming_input_documents_1459`: two documents, a
    control run proving both are seen absent a consumer, then
    `first(range(1, input)), input` proving the second is never popped.
  - A negative control: `[range(1, input)]` (no `first`) over two documents
    still consumes both — the fix must not over-lazify.
  - Rules 3 and 4 above, and the `resolve_leaf`/`path()` bonus fix.
  - A `sink_stopped`-via-intermediate-consumer case
    (`isempty(range(1, ("B"|stderr)))`-shaped), confirming demand
    propagates correctly when the stopping consumer is not the immediate
    caller of `range`.
- One `tests/jq_evaluator_parity_tests.rs` row, per that file's own
  convention, so a future native `eval_generic.rs` arm cannot silently
  bypass this fix; plus a yq-mode CLI test confirming `range` still fans
  out with laziness intact under yq's "unopposed extension" gate.

## Blast radius

Single file: `src/jq/eval.rs`. No `eval_generic.rs` change (confirmed
above). No `#[cfg]` matrix concerns — `range` has no regex/`no_std` gating
of its own. yq mode shares the same `<S: EvalSemantics>`-generic evaluator,
so no separate yq-mode implementation is needed, only parity test coverage.
One deliberate, documented exit-code change (`path(range(...))` with an
unreached trailing halt in a bound: 9 → 5), in the direction of jq
fidelity.

## Follow-ups (explicit non-goals)

- **`MAX_RANGE` (100,000) per-combination cap is unaffected.** This fix
  lazifies which `(from, to, step)` *combinations* get tried — not how many
  values one combination can produce. `eval_range_values`/`_f64` are reused
  unmodified, so each combination still eagerly builds one capped `Vec`
  before any of its values reach `sink`, exactly as the eager path already
  does. Out of scope for #1556 (its title and branch name are about bound
  *arguments*), noted here so it doesn't read as an oversight.

## Related

- [#1556](https://github.com/rust-works/succinctly/issues/1556) — this
  document's subject.
- [#1279](https://github.com/rust-works/succinctly/issues/1279),
  [jq-generator-argument-fanout.md](jq-generator-argument-fanout.md) —
  `range`'s existing fan-out (Stage 6) and `fanout_two_args_lazy` (#1531),
  the structural precedent this design extends one level deeper.
- [#820](https://github.com/rust-works/succinctly/issues/820),
  [jq-lazy-generator-consumers.md](jq-lazy-generator-consumers.md) — the
  `Flow`/`Demand`/`eval_each` machinery and the fallback invariant this
  design is bound by.
- [#1054](https://github.com/rust-works/succinctly/issues/1054) — the "two
  evaluators" trap this design confirms it does not repeat.
