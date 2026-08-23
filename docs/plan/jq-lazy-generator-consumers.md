# A demand-driven sink for jq's short-circuiting generator consumers (#820, #932, #987)

[Home](../../) > [Docs](../) > [Plan](./) > Lazy generator consumers

**Status: Stages 1, 2, 2b, 3, 4 and 5 implemented.** This document is the
deliverable for [#820](https://github.com/rust-works/succinctly/issues/820), which its
own tier review (2026-08-20) classified Tier 3 — "evaluator-architecture change, design
doc first, in the shape of #1282". It also scopes the two issues that name #820 as their
real fix, [#932](https://github.com/rust-works/succinctly/issues/932) (closed — Stage 2
took `any`/`all`/`IN(s)`, Stage 4 the remaining `IN(src; s)`) and
[#987](https://github.com/rust-works/succinctly/issues/987) (closed
by Stage 3: `each_paths_filter` + `resolve_leaf`'s stop-after-first sink). See
[Follow-up issues](#follow-up-issues).

| Stage | What                                                | Status                     |
|-------|-----------------------------------------------------|----------------------------|
| 1     | Characterization tests (the over-stopping trap set) | ✅ merged — #1386, PR #1389 |
| 2     | `eval_each` + ten `eval.rs` consumers               | ✅ merged — #1421, PR #1424 |
| 2b    | `eval_generic.rs`'s `first` comma walk              | ✅ merged — #1434, PR #1435 |
| —     | `eval_each_pipe`'s owned-value arm (Stage 2 gap)    | ✅ merged — PR #1450        |
| 3     | `paths(f)` producer + `resolve_leaf` sink           | ✅ merged — closes #987     |
| 4     | `Expr::Compare`'s outer loop                        | ✅ merged — closes #1459    |
| 5     | Widen the lazy arm set                              | ✅ merged — closes #1462    |
| (c)   | Mirror the sink into `eval_generic` for `Pipe`      | 🟡 partial — #1451, rest #1461 |

**What actually shipped, against what this document predicted.** #820's silent data loss —
a discarded branch consuming `input`/`inputs` documents the CLI's driver loop then never
processed — is closed, as is `halt_error`'s wrong exit code and the stderr leak in
`isempty`/`first`/`limit`/`nth`/`any`/`IN(s)`, and — since Stage 4 — `IN(src; s)` and every
other compare reached through a lazy consumer. Since Stage 5, `eval_each` also carries native
`If`, `Try`/`Optional`, `Label`, `As`, `AsPattern` and `Limit` arms, closing the
demand-forwarding gap for a generator nested inside any of them (`isempty(limit(3; 1,
("B"|stderr)))`, `isempty(if true then (1,("B"|stderr)) else 9 end)`, and the rest of the
family the same shape). **Three** shapes remain divergent, all pinned as such in
`test_short_circuit_side_effect_leaks_820_932_987`. Two are the bare-`first(...)` family:
`first(.[] | stderr)` (see "Option (c), scoped" below) and
`first((1,2) == (10, ("B"|stderr)))`, both a `first(...)` over a
non-`Comma`, which Stage 2b's sibling walk does not reach — option (c), #1461. This is a
*separate* obstacle from Stage 5's own arms: a bare `first(...)`/`last(...)` never even
reaches `eval.rs` for a non-`Comma` argument, because `eval_generic.rs`'s own native
`first`/`last` routing intercepts it first (confirmed live while implementing Stage 5, below);
`isempty`, `limit`, `nth`, `path`, `any`, `all` and `IN` have no such native `eval_generic` arm
and so reach every one of Stage 5's new arms correctly. The third divergent shape is
`("A"|stderr) == (("B"|stderr), ("C"|stderr))`, a *top-level* compare, which never reaches
`eval_each` at all — #1481.

**One divergence Stage 5's own oracle sweep found and did not attempt.** `?//`-alternatives
wrapped in a short-circuiting consumer: real jq's builtins are macro-expanded `label $out |
... break $out` definitions, and its `?//` operator retries the next alternative on *any*
escaping break, including one addressed to a label entirely outside the `?//` — so
`isempty(1 as $x ?// $y | 5)` is `false` **twice** in real jq (a three-alternative chain,
three times), where succinctly's native, non-macro-expanded `?//` resolution always answers
once. Confirmed pre-existing (unaffected by Stage 5's changes) and orthogonal to this
document's own scope — filed as [#1519](https://github.com/rust-works/succinctly/issues/1519)
rather than folded in.

**Option (c), scoped.** `first`/`last` were the *only* `eval_generic.rs` consumers with a
native, cursor-preserving fast-path arm shadowing `eval.rs`'s already-lazy implementation
(`isempty`/`limit`/`nth`/`any`/`all`/`IN` have no native arm in `eval_generic.rs` at all —
see `first_over_comma_generic`'s own doc comment — so they already bridge into `eval.rs`'s
`eval_each`-based lazy path and never needed this). #1451 widened `first_over_comma_generic`
to recurse through a pipe's own *2-stage* prefix: the leading stage is evaluated exactly
once, then [`first_over_pipe_prefix_result`](../../src/jq/eval_generic.rs) dispatches on
every possible shape of that one result (a mutually-recursive `OwnedValue`-flavored twin,
`first_over_comma_owned_generic`, handles the *computed*-value shapes —
`GenericResult::Owned`/`ManyOwned`/`Partial`, the common case for a literal/arithmetic
leading stage — since a bare `V`/cursor doesn't exist for those) without ever re-evaluating
it. This closes the two narrower `Pipe`-composition shapes #1451 itself named
(`first(1 | (1, stderr))`, `first((1, stderr), 2)`).

An earlier version of this fix additionally tried real per-output backtracking over a
`Many`/`ManyCursor` (document-derived, e.g. `.[]`) prefix's already-materialized values —
which would have also closed `first(.[] | stderr)` — but review (#1503) found this
regressed two properties only a *whole, unsplit* pipe evaluation preserves:
`eval_single`'s own multi-stage `ManyCursor` handling, which threads a cursor through the
*entire* remaining pipe together rather than stage-by-stage (a `.[] | select(...) | line`
query lost its `line` cursor under the per-value split), and `LazySeq`'s own #724/#725
composability (`first(map(f) | .[0])` started forcing full materialization instead of
short-circuiting). Both regressions trace to the same root cause: splitting a pipe into an
eagerly-evaluated prefix plus a separately-dispatched tail is only safe when the prefix is a
*single* value (nothing to lose by handling it apart from the rest of the pipe) — a
`Many`/`ManyCursor`/`LazyKeys`/`LazyIndexRange`/`LazySeq` prefix is deferred back to the
existing whole-pipe fallback instead (safe to redo, since none of those shapes have run any
side effect yet), leaving `first(.[] | stderr)` a known, still-pinned divergence rather than
a second, deeper thing option (c) closes. The full generalized `Demand`/`Item`/`Flow` mirror
this row originally called for remains the only way to close it soundly — filed as **#1461**.

**One correction this document earned the hard way.** Stage 2 shipped an
`eval_each_pipe` whose `Item::Owned` arm fell back to the eager `eval_owned_pipe`,
justified in review by this document's own fallback invariant. That was a misreading: the
invariant says an un-lazified arm cannot change *which values are delivered*, and says
nothing about side effects — which are the entire subject here. So `isempty(1 | (1, input))`
still destroyed a document until PR #1450 routed that arm through `eval_each_owned`. The
behaviour tables below did not catch it; the ~1300-case systematic sweep in
[Verification approach](#verification-approach-for-the-follow-up-implementation-prs), which
crosses consumers against generator shapes including `(1|X)`, would have. **Run the sweep,
not just the tables.**

**Mechanism decision, already made:** an **additive sink/callback path alongside** the
existing eager `eval_single`, not a rewrite of it. `eval_comma`, `eval_pipe` and
`QueryResult` keep their current bodies and contracts verbatim. This document's job is to
make that concrete and correct.

**Everything below marked "verified" was captured at `cedee4cbb`, before any stage
landed** — so the "succinctly today" columns record the pre-fix behaviour this design set
out to change, not current `main`. They are kept as the historical record; for what `main`
does now, see the tests named above. Each was run against the pinned oracle
`/usr/bin/jq` (jq-1.7.1-apple) and a `--release --features cli` build of this worktree,
with stdout and stderr captured *separately*.** `2>&1` interleaves the two misleadingly,
since stdout is buffered when piped and stderr is not — the convention stated at
`tests/jq_cli_tests.rs:3469`.

## Problem

Every consumer that only needs a *prefix* of a generator's outputs evaluates the **whole**
generator first, then truncates the already-materialized `QueryResult`. For `error(...)`
and `debug` that is externally indistinguishable from real laziness — their "output" is
data carried in the result, with no I/O at evaluation time. For `stderr` and `halt_error`
it is not: both call `write_stderr` (`src/jq/eval.rs:24928`) *during* evaluation, so the
bytes are gone by the time the consumer decides it never needed that output.

```
$ jq -n 'isempty(1, stderr)'              false         (nothing on stderr)
$ succinctly jq -n 'isempty(1, stderr)'   false         (leaks `null` to stderr)
```

`builtin_debug`/`builtin_debug_msg` (`:24882`/`:24892`) are deliberate library-context
no-ops, which is why #820's original `first(1, debug)` repro was a false negative that
made `first`/`limit` look already-lazy. Re-tested with `stderr`, they leak identically to
`isempty`.

### This is no longer a cosmetic leak — it causes silent data loss

**#820's own severity rationale is out of date**, because a dependency changed after it
was filed. When #820 was written, `input`/`inputs` were `undefined function` — that was
#723's whole point. #723 is now closed and they are implemented, parsed at
`src/jq/parser.rs:3444-3454` and backed by a process-global `thread_local!` queue
(`mod remaining_inputs`, `src/jq/eval.rs:21428`) that the CLI's own per-document driver
loop also drains (`src/bin/succinctly/jq_runner.rs:1023`).

`input` is therefore a **consuming** side effect, not merely a visible one. An eagerly
evaluated, never-needed comma branch containing it eats a document the driver loop then
never processes (verified):

```
$ printf '{"id":1} {"id":2} {"id":3} {"id":4}' | jq -c '.id'
1 2 3 4
$ printf '{"id":1} {"id":2} {"id":3} {"id":4}' | succinctly jq -c '.id'
1 2 3 4                                    <- control: the two agree

$ printf '{"id":1} {"id":2} {"id":3} {"id":4}' | jq -c '[isempty(1, input), .id]'
[false,1] [false,2] [false,3] [false,4]
$ printf '{"id":1} {"id":2} {"id":3} {"id":4}' | succinctly jq -c '[isempty(1, input), .id]'
[false,1] [false,3]                        <- documents 2 and 4 silently consumed
```

Half the input disappears, exit code 0, nothing on stderr. Identical shape for
`[first(1, input), .id]`. The mechanism is exactly the one #820 already describes; only
the consequence has changed, from a stray stderr write to silent data loss.

This is what justifies the work. The cluster-B framing — "three Low-severity issues, fix
them together" — is no longer what carries it; #820 carries it alone.

### `halt_error`'s exit code is also wrong

Not recorded in #820, whose text for this repro predates #791 (verified):

```
$ jq -n '"outer" | halt_error(1, ("inner"|halt_error(2)))'             stderr: outer   exit 1
$ succinctly jq -n '"outer" | halt_error(1, ("inner"|halt_error(2)))'  stderr: inner   exit 2
```

`eval_comma` runs the second branch, which halts; `result_to_owned_full`'s
`Partial(_, Control::Halt(code)) => Err(EvalEscape::Halt(code))` arm (`:1511`, correct in
isolation per #791) then lets the inner halt win over the outer. #820's body reports
`innerouter` / exit 1, which is stale — anyone re-running it may wrongly conclude the
issue is partly fixed.

## Root cause (verified by reading, not guessed)

`eval_comma` (`src/jq/eval.rs:853`) is a plain `for expr in exprs` loop calling
`eval_single` per branch into a `borrowed: Vec<StandardJson>` / `owned:
Option<Vec<OwnedValue>>` pair — one ordered accumulator, promoted on the first owned
operand (#353). It returns on the first `Error`/`Break`/`Halt`/`Partial` as a `Partial`.
**There is no channel by which a consumer can tell it to stop.**

`eval_pipe` (`:9953`) materializes stage 1 fully via `eval_single`, then loops
`for v in values { eval_pipe(rest, v) }`. Downstream *already* streams per value; the two
gaps are that stage 1 is fully materialized first, and that nothing downstream can stop
the loop.

Every short-circuiting consumer sits on top of that: `eval_single` (or
`eval_owned_expr_fork` / `eval_owned_multi_keep_partial`) on the whole sub-expression,
then `.take(n)` / `.next()` / a membership scan on the *result*. They correctly drop the
trailing `Control` once satisfied — that half is right and oracle-verified — but the side
effect has already fired.

`resolve_leaf`'s catch-all (`:13813`) says so in its own comment: *"unlike jq,
`eval_owned_multi_keep_partial` already ran the candidate that halted, side effects
included, by the time control reaches here."*

### Re-evaluation is a known-bad fix shape here

`Expr::Alternative`'s resolver comment (`:13420-13427`) records a live-verified
double-fire: checking an operand's truthiness by evaluating it a *second* time made
`path(stderr // .b)` "write its input to stderr twice", and was rejected in review. The
same comment (`:13435`) names #820 as the general case. This rules out the cheaper
"demand budget, re-evaluate with a bigger `n`" alternative: a fix must **not evaluate**,
rather than evaluate-and-discard or evaluate-again.

## Two evaluators, and which one the CLI actually runs

Load-bearing, and mis-scoped in #820's thread, so it is stated before the design.

`src/bin/succinctly/jq_runner.rs:2199` and `:2296` both call
`eval_generic::eval_with_cursor` (`eval_generic.rs:2138`). **Every `succinctly jq`
invocation enters `eval_generic.rs` first**, recurses through its native arms, and only
hands a subtree to `eval.rs`'s `full_eval` at a node with no native generic arm — at which
point the *whole* subtree crosses over.

| Consumer                                     | Native `eval_generic` arm?           | Evaluator that runs it |
|----------------------------------------------|--------------------------------------|------------------------|
| `isempty(g)`                                 | no                                   | `eval.rs`              |
| `limit(n; f)`, `nth(n; f)`                   | no                                   | `eval.rs`              |
| `any(g;c)` / `all(g;c)` / `IN` / `IN(src;s)` | no                                   | `eval.rs`              |
| `halt_error(n)`                              | no                                   | `eval.rs`              |
| `path(...)` / `paths(f)`                     | no                                   | `eval.rs`              |
| **`first(f)` / `last(f)`**                   | **yes** (`:2878`, `:4274` → `:3121`) | **`eval_generic.rs`**  |

**Consequence: an `eval.rs`-only fix does not fix `succinctly jq -n 'first(1, stderr)'`.**
The generic `FirstExpr` arm calls `eval_generic::eval_single` on the inner `Comma`, hits
the native eager Comma arm at `eval_generic.rs:3018`, and never reaches `eval.rs`'s
`eval_first_expr`. Meanwhile `limit` — which has no generic arm — *would* be fixed. Two
adjacent short-circuiting consumers, opposite outcomes, for reasons that have nothing to
do with the fix. Stage 2b addresses this.

`eval.rs`'s `eval_first_expr` is still worth lazifying: it is reached whenever `first(...)`
sits *inside* a subtree that has already bounced (e.g. `isempty(first(1, stderr))`).

## Scope decision

**Add one demand channel; do not touch the eager path.**

Making `eval_comma`/`eval_pipe` themselves lazy means changing `QueryResult`'s contract,
which 113 `eval_single` call sites and both evaluators depend on. Adding a `QueryResult`
variant costs ~110 exhaustive-match arms in `eval.rs` alone plus cross-crate test and
`yq_runner.rs` sites. This design instead adds a second, parallel entry point (`eval_each`)
reached *only* from consumers that genuinely need a prefix.

### Non-goals (explicit, to prevent scope creep)

- **Replacing `eval_comma`/`eval_pipe` with `eval_each` + a collecting adapter.**
  Attractive as an endgame — it retires the duplicated Comma/Pipe semantics this design
  creates — but it puts the #295/#353 borrowed/owned promotion and the whole `Partial`
  contract back in play. Deferred past Stage 5, gated on the `collect_each` differential
  test below being green across a release cycle.
- **`GenericResult::LazySeq`** ([jq-lazy-map-select.md](jq-lazy-map-select.md),
  #700/#724/#725). A real pull-based chain, but it models `map`/`select` over *document
  containers* on the `DocumentValue` side; it has no representation for a comma/pipe
  generator and no demand channel back to a producer. Not reusable. Its *reasoning* is
  reused — see [Why `dyn` is right here](#why-a-dyn-trait-object-is-right-here-and-was-wrong-there).
- **`last(f)`, `reduce`, `foreach`, array construction, `collect_paths`' walk.** None can
  short-circuit by definition (`last` cannot know a value is last until exhaustion; `[…]`
  is atomic in jq). They stay eager.
- **jq's multi-output generator-argument fan-out for ordinary builtins** — `"abcabc" |
  ltrimstr(("a","b"))` yields two outputs in jq and one here. That is **#1279**, an
  independent and opposite-direction bug (succinctly too *lazy* there, not too eager). The
  design below is careful not to make it worse; see [The `ltrimstr` trap](#the-ltrimstr-trap).
- **`eval_generic.rs`'s `Expr::Comma`/`Expr::Pipe` arms.** Only its `first`/`last` arm is
  in scope (Stage 2b).

## The mechanism

### Types (new, in `src/jq/eval.rs`, next to `QueryResult`)

```rust
/// What a sink wants after receiving one output.
///
/// `Stop` is this design's model of the `break $out` in jq's own definitions
/// (`def first(f): label $out | (f, break $out);`,
/// `def isempty(g): label $out | (g|false, break $out), true;`): not "an error
/// happened", but "the consumer has what it came for, and jq's generator would
/// never have been asked for another value".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Demand { Continue, Stop }

/// One output on its way to a sink.
///
/// Two variants, not one: `eval_first_expr`/`eval_limit` return
/// `QueryResult::One`/`Many` when their input was borrowed, and collapsing that
/// to `OwnedValue` would lose the zero-copy path #295/#353 exist to protect and
/// change observable duplicate-key and number-spelling behaviour. No `Cursor`
/// variant: every producer calls `.materialize_cursor()` first, exactly as
/// `eval_comma`/`eval_pipe` already do.
enum Item<'a, W = Vec<u64>> {
    Borrowed(StandardJson<'a, W>),
    Owned(OwnedValue),
}

/// How a generator stopped producing.
///
/// This is `Partial`'s information re-factored for a pushed rather than
/// collected world: the pushes *are* the prefix, so only the terminator needs a
/// representation.
enum Flow {
    /// Ran to exhaustion; every output was delivered.
    Exhausted,
    /// A sink returned `Stop`. `pending` is a control an *eager fallback* had
    /// already raised before the stop — only ever `Some` on a `Partial` drained
    /// past a `Stop`, never on a lazified arm. The consumer applies its own
    /// existing keep-or-drop policy.
    Stopped { pending: Option<Control> },
    /// Terminated in a control. Everything produced first was already delivered.
    Escaped(Control),
}
```

`Flow` is isomorphic to `struct { stopped: bool, control: Option<Control> }`. It is an
enum for the reason `EvalEscape`'s own doc comment (`src/jq/error.rs:66-113`) gives for
splitting `Error`/`Break`/`Halt`: it makes "escaped without a control" and "stopped with a
control the consumer must decide about" structurally distinct, so the mistake cannot be
made by writing the natural-looking arm.

### Signatures

```rust
/// Push every output of `expr` into `sink`, stopping as soon as `sink` says to.
fn eval_each<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr, value: StandardJson<'a, W>, optional: bool,
    sink: &mut dyn FnMut(Item<'a, W>) -> Demand,
) -> Flow;

/// Slice twin, so the `Pipe` arm recurses on `&rest` without rebuilding an
/// `Expr::Pipe` per value (mirrors `eval_pipe`'s own slice recursion).
fn eval_each_pipe<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    exprs: &[Expr], value: StandardJson<'a, W>, optional: bool,
    sink: &mut dyn FnMut(Item<'a, W>) -> Demand,
) -> Flow;

/// Owned-input twin, mirroring `eval_owned_input` (`:16650`) exactly:
/// `eval_owned_fast_path` first, otherwise serialize via `to_json_for_reindex`,
/// rebuild a `JsonIndex`, run `eval_each` against the fresh cursor.
///
/// Its sink takes `OwnedValue`, not `Item`: items produced against a locally
/// built index cannot outlive this call — the same reason `eval_owned_input`
/// normalizes `One`/`Many` to `Owned`/`ManyOwned` today. **This is what lets the
/// borrowed-surface primitive serve the owned-surface consumers** (`any`/`all`,
/// `resolve_leaf`), which is otherwise the design's hardest constraint.
fn eval_each_owned<S: EvalSemantics>(
    expr: &Expr, input: &OwnedValue, optional: bool,
    sink: &mut dyn FnMut(OwnedValue) -> Demand,
) -> Flow;

/// Drain an already-materialized `QueryResult` into a sink, checking demand
/// between values. The fallback for every un-lazified arm.
fn drain_result<'a, W: Clone + AsRef<[u64]>>(
    result: QueryResult<'a, W>, sink: &mut dyn FnMut(Item<'a, W>) -> Demand,
) -> Flow;

/// Test-only inverse: collect a sink stream back into a `QueryResult`,
/// reproducing `eval_comma`'s borrowed/owned promotion. Makes the fallback
/// invariant differentially testable.
#[cfg(test)]
fn collect_each<'a, W: Clone + AsRef<[u64]>>(
    run: impl FnOnce(&mut dyn FnMut(Item<'a, W>) -> Demand) -> Flow,
) -> QueryResult<'a, W>;
```

### `&mut dyn FnMut`, not a generic `F`

A generic `F: FnMut(Item<'a, W>) -> Demand` does not merely bloat — **it does not
compile.** The `Pipe` arm builds a *new* closure capturing the *outer* sink and calls
`eval_each` again:

```
eval_each::<F>(Pipe([a, b]), …, sink: &mut F)
    builds driver_1: Driver<F>
    calls eval_each::<Driver<F>>(a, …)
        if `a` is itself a Pipe, builds Driver<Driver<F>>
        …
```

The recursion is on **runtime AST data**, not a type-level counter, so the compiler has no
bound on the instantiation set and fails with *"reached the recursion limit while
instantiating"*. `&mut dyn FnMut` collapses every depth to one instantiation. (Structural
argument, not a compiled experiment — an implementer wanting the evidence should try the
generic form first, and should not be surprised.)

### Why a `dyn` trait object is right here and was wrong there

[jq-lazy-map-select.md](jq-lazy-map-select.md) rejected `Box<dyn Iterator<Item=…> + 'a>`
for `LazySeq` because it would have been **stored in the returned enum**, forcing a
lifetime parameter onto `GenericResult<V>` itself and "rippling through roughly 15
functions in `eval_generic.rs` plus `jq_runner.rs`/`yq_runner.rs`", while forgoing `Clone`
(needed for replay) and paying a vtable hop per element per stage.

None of that applies here. The trait object is a **borrowed function parameter**; it lives
only for the call, never enters a return type; `'a` and `W` are already on `eval_single`'s
signature so nothing new is introduced; `Flow` is plain owned data. The vtable hop is paid
only on the short-circuiting path, which by construction stops early. **The prior
document's reasoning argues *for* this shape** — what it objected to was putting an
un-`Clone`able borrow into a long-lived result type.

### The fallback drain, in full

```rust
fn drain_result<'a, W: Clone + AsRef<[u64]>>(
    result: QueryResult<'a, W>, sink: &mut dyn FnMut(Item<'a, W>) -> Demand,
) -> Flow {
    match result.materialize_cursor() {
        QueryResult::None => Flow::Exhausted,
        QueryResult::OneCursor(_) => unreachable!("materialize_cursor removes OneCursor"),

        QueryResult::One(v) => match sink(Item::Borrowed(v)) {
            Demand::Continue => Flow::Exhausted,
            Demand::Stop => Flow::Stopped { pending: None },
        },
        QueryResult::Owned(v) => match sink(Item::Owned(v)) {
            Demand::Continue => Flow::Exhausted,
            Demand::Stop => Flow::Stopped { pending: None },
        },
        QueryResult::Many(vs) => {
            for v in vs {
                if sink(Item::Borrowed(v)) == Demand::Stop {
                    return Flow::Stopped { pending: None };
                }
            }
            Flow::Exhausted
        }
        QueryResult::ManyOwned(vs) => {
            for v in vs {
                if sink(Item::Owned(v)) == Demand::Stop {
                    return Flow::Stopped { pending: None };
                }
            }
            Flow::Exhausted
        }

        QueryResult::Error(e)     => Flow::Escaped(Control::Error(e)),
        QueryResult::Break(label) => Flow::Escaped(Control::Break(label)),
        QueryResult::Halt(code)   => Flow::Escaped(Control::Halt(code)),

        // The prefix was produced *before* the control (`partial` (`:1328`)
        // guarantees it is non-empty), so it is delivered first, exactly as
        // `push_owned_values` (`:1630`) already delivers it. If a sink stops
        // mid-prefix, the control it never reached rides out as `pending` — not
        // dropped here, because consumers already have different, individually
        // oracle-verified policies for it.
        QueryResult::Partial(vs, control) => {
            for v in vs {
                if sink(Item::Owned(v)) == Demand::Stop {
                    return Flow::Stopped { pending: Some(control) };
                }
            }
            Flow::Escaped(control)
        }
    }
}
```

### The fallback invariant (state it, then test it)

> **For a sink that always returns `Demand::Continue`, `eval_each(e, v, opt, sink)`
> delivers exactly the values `push_owned_values(eval_single(e, v, opt), …)` would
> collect, in the same order, and returns the same terminal `Control`.**

`drain_result` *is* `push_owned_values` plus a demand check between values, minus the
`to_owned` on borrowed items. So an expression form with no lazy arm is observationally
identical to today: **an un-lazified arm is a missed optimization, never a regression.** A
lazy arm can only *shrink* the set of sub-expressions evaluated; it cannot change which
values are delivered or their order.

Make it checkable rather than asserted: a test asserting
`collect_each(|s| eval_each(e, v, false, s)) == eval_single(e, v, false)` over the existing
`jq_tests.rs` expression corpus with an always-`Continue` sink. That is the guard against
the duplicated Comma/Pipe semantics drifting from the originals.

### Which `Expr` arms get a native lazy arm

**Stage 2's minimal set is three arms.** Everything else falls back.

```rust
match expr {
    // Mirrors eval_comma (:853) exactly, minus the accumulator.
    Expr::Comma(exprs) => {
        for e in exprs {
            match eval_each::<W, S>(e, value.clone(), optional, sink) {
                Flow::Exhausted => {}
                other => return other,     // Stopped or Escaped
            }
        }
        Flow::Exhausted
    }

    Expr::Pipe(exprs) => eval_each_pipe::<W, S>(exprs, value, optional, sink),

    // Mandatory: `isempty((1, stderr))` leaks today and `isempty(...)` eats only
    // its own parens, so without this the fix is a coin flip on spelling.
    Expr::Paren(inner) => eval_each::<W, S>(inner, value, optional, sink),

    _ => drain_result(eval_single::<W, S>(expr, value, optional), sink),
}
```

`eval_each_pipe` **must** open with `eval_pipe`'s own precheck, or `path`/`parent`/
`file_index` semantics silently break:

```rust
// eval_pipe (:9953) diverts to eval_pipe_with_path_context (:17869) whenever any
// stage needs path context. `needs_path_context` (:466) recurses through
// Pipe/Paren/Comma/Array/If/Try/Label, so this is a WHOLE-PIPE property and
// cannot be decided per stage. Delegate to the eager fallback, which performs
// the same check itself, rather than re-deriving it.
if exprs.iter().any(needs_path_context) {
    return drain_result(eval_pipe::<W, S>(exprs, value, optional), sink);
}
if exprs.is_empty() { return match sink(Item::Borrowed(value)) { … }; }
let (first, rest) = exprs.split_first().unwrap();
if rest.is_empty()  { return eval_each::<W, S>(first, value, optional, sink); }

let mut inner_escape: Option<Control> = None;
let mut driver = |item: Item<'a, W>| -> Demand {
    let flow = match item {
        Item::Borrowed(v) => eval_each_pipe::<W, S>(rest, v, optional, sink),
        // Mirrors eval_pipe's Owned/ManyOwned arms, which route through
        // eval_owned_pipe (:10111) -> eval_owned_input. Same bridge, same cost.
        Item::Owned(v) => eval_each_owned::<S>(
            &pipe_of(rest), &v, optional, &mut |o| sink(Item::Owned(o)),
        ),
    };
    match flow {
        Flow::Exhausted           => Demand::Continue,
        Flow::Stopped { pending } => { inner_escape = pending; Demand::Stop }
        Flow::Escaped(c)          => { inner_escape = Some(c);  Demand::Stop }
    }
};
match eval_each::<W, S>(first, value, optional, &mut driver) {
    _ if inner_escape.is_some() => Flow::Escaped(inner_escape.unwrap()),
    flow => flow,
}
```

*Borrowck note:* `driver` captures `sink` by unique borrow and reborrows it (`&mut **sink`)
per call. This works, but is the fiddliest part of the diff — expect to hoist
`let sink = &mut *sink;` and to split `inner_escape` out of the closure's capture set.

#### How a sink raises

`Demand` has no `Fail(Control)` variant. A consumer that wants to raise (`resolve_leaf`'s
"Invalid path expression", `any_all_probe_element`'s `cond` escape) records the control in
its own captured state and returns `Demand::Stop`, then inspects that state after
`eval_each` returns. This keeps one escape channel per direction — producer→consumer via
`Flow`, consumer→itself via its own closure — and avoids inventing a precedence rule for
"producer and consumer both raised in the same step".

#### Arms deliberately *not* lazified, and what that costs

Each checked against the oracle. "already correct" means succinctly matches jq today.

| Arm                                        | Needed for #820/#932/#987? | Note                                                                                                    |
|--------------------------------------------|----------------------------|---------------------------------------------------------------------------------------------------------|
| `Iterate`, `Range`, literals, field/index  | no, ever                   | side-effect-free producers; the drain already stops per value                                           |
| `Break`                                    | no, ever                   | a leaf returning `QueryResult::Break`; drain maps it to `Escaped`                                       |
| `And`/`Or`/`Alternative`                   | no — already correct       | `false and ("B"\|stderr)` etc. agree today                                                              |
| `If` (branch selection)                    | no — already correct       | `eval_fanout` (`:1662`) evaluates only the taken branch                                                 |
| `If` (generator inside a branch)           | no                         | but `first(if true then (1,("B"\|stderr)) else 9 end)` leaks. Stage 5                                   |
| `Try`/`Optional`                           | no                         | but `first(try (1,("B"\|stderr)) catch 9)` leaks. Stage 5                                               |
| `Label`                                    | no                         | but `first(label $o \| (1,("B"\|stderr)))` leaks. Stage 5                                               |
| `AsPattern`                                | no                         | but `first(1 as $x \| (1,("B"\|stderr)))` leaks. Stage 5                                                |
| `FuncCall`/`FuncDef`                       | no                         | but `def f: (1,("B"\|stderr)); first(f)` leaks. Stage 5                                                 |
| `FirstExpr`/`Limit`/`NthExpr` as producers | no                         | but `isempty(limit(3; 1, ("B"\|stderr)))` leaks — demand does not forward *through* a consumer. Stage 5 |
| `Reduce`/`Foreach`                         | no                         | `reduce` has one output; `foreach`'s #534 fork machinery is a non-goal                                  |
| **`Compare`**                              | **yes — for #932**         | Stage 4; see below                                                                                      |
| **`Builtin::PathsFilter`**                 | **yes — for #987**         | Stage 3                                                                                                 |
| **`Builtin::Inputs`**                      | **yes — for #1309**        | Stage 3.5. The one arm that is a *correctness* fix, not an optimization — see below                     |

**`Builtin::Inputs` breaks the "un-lazified is only a missed optimization" rule.**
`drain_result`'s invariant is stated over *the values delivered to this sink*; it says
nothing about side effects on state the sink does not own. Every other producer here
over-*computes* at worst, and the extra values are discarded. `inputs` over-*consumes* a
shared, destructive, non-replayable queue that the CLI's own per-document driver loop and
every later `input` call also draw from, so a document produced for a consumer that never
asked for it is gone from the whole program. Before #1309, `., first(inputs)` on five
documents printed `1 2` against jq's `1 2 3 4 5`. Anything added to this table in future
should be checked against that distinction: a producer that reads external, non-replayable
state cannot wait for Stage 5.

**A caveat Stage 5 needs to carry, not just `eval.rs`.** Every `first(...)`-based example
in the table above is a bare CLI call — and, per "Option (c), scoped" above, the CLI's
`first`/`last` never reach `eval.rs`'s `eval_each` at all (`eval_generic.rs`'s own native
`Expr::FirstExpr`/`LastExpr` arm intercepts first, kept cursor-preserving for #607).
Widening `eval_each`'s native arm set to cover `If`/`Try`/`Label`/`AsPattern`/`FuncCall`
will *not*, by itself, close any of the `first(...)`-shaped rows above through the CLI —
`first_over_comma_generic`/`first_over_comma_owned_generic` (`eval_generic.rs`) fall back to
one eager `eval_single`/`eval_on_owned` call for exactly this fallback-shape residual
(see [`try_lazy_first_generic`](../../src/jq/eval_generic.rs)'s own doc comment), same as
before Stage 5. Closing these for the CLI needs the identical widening applied a second
time there, or `first`/`last` no longer shadowing `eval.rs` for these shapes specifically --
worth folding into **#1462**'s own scope rather than rediscovering as a surprise gap once
that issue's `eval.rs` half ships.

**Honest scoping of #932.** `builtin_upper_in` (`:3759`) synthesizes
`gen = Expr::Compare { Eq, src, s }` for the `IN(src; s)` form and hands it to
`any_all_gen_cond`. `binary_fanout_core` (`:1772`) evaluates the **right** operand to
completion first, then re-evaluates the left per right value — jq's real nested-loop
order, established and oracle-verified by #910. So `IN(src; s)` needs `Expr::Compare`
demand-aware in its outer loop, or it keeps leaking. **Stage 2 closed `any`/`all`/`IN(s)`;
Stage 4 closed `IN(src; s)`.**

Stage 4 did that without moving the loop order into `builtin_upper_in`, which #910's own
review had already rejected as a hand-rolled cross-product. `binary_fanout_core`'s body
became `binary_fanout_each` — the same right-outer/left-inner loop, parameterized over how
an operand is *enumerated* rather than merely evaluated. `binary_fanout_core` is now the
collecting wrapper that supplies the eager strategy (evaluate the whole operand, replay it
through `drain_result`), so `eval_arithmetic`, `eval_compare` and the path-context sibling
are behaviourally untouched; `eval_each`'s new `Expr::Compare` arm supplies `eval_each`
itself. Keeping one loop was the point: #768 fixed a collapse-to-first-value bug there that
the then-duplicated path-context copy independently re-acquired and had to be fixed again
for #822.

## Behaviour tables (oracle-verified at `cedee4cbb`)

Columns are `stdout | exit | stderr`. Input is `null` (`-cn`) unless noted.

### Divergent today — the fix's targets

| Query                                                              | jq 1.7.1       | succinctly today | Closed by |
|--------------------------------------------------------------------|----------------|------------------|-----------|
| `isempty(1, ("B"\|stderr))`                                        | `false\|0\|`   | `false\|0\|B`    | Stage 2   |
| `isempty((1, ("B"\|stderr)))`                                      | `false\|0\|`   | `false\|0\|B`    | Stage 2   |
| `first(1, ("B"\|stderr))`                                          | `1\|0\|`       | `1\|0\|B`        | Stage 2b  |
| `limit(1; 1, ("B"\|stderr))`                                       | `1\|0\|`       | `1\|0\|B`        | Stage 2   |
| `nth(0; 1, ("B"\|stderr))`                                         | `1\|0\|`       | `1\|0\|B`        | Stage 2   |
| `"o" \| halt_error(1, ("B"\|stderr))`                              | `\|1\|o`       | `\|1\|Bo`        | Stage 2   |
| `"outer" \| halt_error(1, ("inner"\|halt_error(2)))`               | `\|1\|outer`   | `\|2\|inner`     | Stage 2   |
| `2 \| any(2, (5\|stderr); .==2)`                                   | `true\|0\|`    | `true\|0\|5`     | Stage 2   |
| `2 \| IN(2, (5\|stderr))`                                          | `true\|0\|`    | `true\|0\|5`     | Stage 2   |
| `[1,2] \| first(.[] \| stderr)`                                    | `1\|0\|1`      | `1\|0\|12`       | Stage 2b  |
| `isempty(first(1, ("B"\|stderr)))`                                 | `false\|0\|`   | `false\|0\|B`    | Stage 2   |
| `[1,2,3] \| path(paths(if .==3 then (stderr,true) else true end))` | `\|5\|<#530>`  | `\|5\|3<#530>`   | Stage 3   |
| `[IN((2,3); 2, (5\|stderr))]`                                      | `[true]\|0\|`  | `[true]\|0\|5`   | Stage 4   |
| `[isempty((1,2) == (10, ("B"\|stderr)))]`                          | `[false]\|0\|` | `[false]\|0\|B`  | Stage 4   |
| `[isempty(.id == (1, input)), .id]` over 4 docs                    | `ids 1-4\|0\|` | `ids 1,3\|0\|`   | Stage 4   |
| `[all((1,2) == (10, ("B"\|stderr)); .)]`                           | `[false]\|0\|` | `[false]\|0\|B`  | Stage 4   |
| `isempty(limit(3; 1, ("B"\|stderr)))`                              | `false\|0\|`   | `false\|0\|B`    | Stage 5   |
| `first(if true then (1,("B"\|stderr)) else 9 end)`                 | `1\|0\|`       | `1\|0\|B`        | Stage 5   |
| `first(try (1,("B"\|stderr)) catch 9)`                             | `1\|0\|`       | `1\|0\|B`        | Stage 5   |
| `first(label $o \| (1,("B"\|stderr)))`                             | `1\|0\|`       | `1\|0\|B`        | Stage 5   |
| `first(1 as $x \| (1,("B"\|stderr)))`                              | `1\|0\|`       | `1\|0\|B`        | Stage 5   |
| `def f: (1,("B"\|stderr)); first(f)`                               | `1\|0\|`       | `1\|0\|B`        | Stage 5   |

Plus the data-loss shapes in [Problem](#this-is-no-longer-a-cosmetic-leak--it-causes-silent-data-loss),
closed by Stage 2 (`isempty`) and Stage 2b (`first`).

### Must not regress — the over-stopping trap set

**This design's characteristic failure mode is stopping too early** and suppressing a side
effect jq genuinely performs. Every row below was verified to agree *today* and would
break under a too-eager `Stop`.

| Query                                                                                 | jq and succinctly agree on | Why it is a trap                                                            |
|---------------------------------------------------------------------------------------|----------------------------|-----------------------------------------------------------------------------|
| `limit(2; 1, stderr, 3)`                                                              | `1 null\|0\|null`          | `limit(2)` **does** pull the second value, which is `stderr`                |
| `isempty(empty, stderr)`                                                              | `false\|0\|null`           | `empty` yields nothing, so `stderr` **is** reached                          |
| `isempty(("B"\|stderr))`                                                              | `false\|0\|B`              | one output still means the producer ran                                     |
| `first(empty, ("B"\|stderr))`                                                         | `"B"\|0\|B`                | same                                                                        |
| `first(stderr, 1)`                                                                    | `null\|0\|null`            | the *first* branch is the side-effecting one                                |
| `[limit(0; ("B"\|stderr))]`                                                           | `[]\|0\|`                  | `n == 0` must not evaluate the operand at all                               |
| `isempty([1, ("B"\|stderr)])`                                                         | `false\|0\|B`              | `[…]` is atomic; laziness must not leak into array construction             |
| `nth(2; 1,2,("B"\|stderr),4)`                                                         | `"B"\|0\|B`                | index 2 *is* the side-effecting branch                                      |
| `2 \| all(2, (5\|stderr); .==2)`                                                      | `false\|0\|5`              | **`all` short-circuits on falsy, so element 2 is reached**                  |
| `2 \| any(9, (5\|stderr); .==2)`                                                      | `false\|0\|5`              | no match ⇒ must exhaust                                                     |
| `[2] \| IN(2, (5\|stderr))`                                                           | `false\|0\|5`              | same                                                                        |
| `false and ("B"\|stderr)` / `if false then … else 1 end`                              | `\|0\|` / `1\|0\|`         | already lazy; must stay lazy                                                |
| `isempty(error("x"))` / `isempty("m"\|halt_error(3))` / `label $o\|isempty(break $o)` | raises / exit 3 / exit 0   | bare escapes must still propagate (#882, #791, #867)                        |
| `label $o \| [isempty(1, break $o)]`                                                  | `[false]\|0\|`             | a `Break` to an **outer** label must not escape once `isempty` is satisfied |
| `nth(5; 1,2,error("x"))` / `last(1,2,error("x"))` / `[limit(3;1,2,error("x"),4)]`     | all raise                  | under-satisfied consumers must still surface the trailing control           |

**`all` is the one most likely to be got wrong.** #932's text guesses that `all(gen;cond)`
"presumably shares the identical code path" and leaks too. It does not: real jq writes `5`
here as well, because `all` must inspect every output before it can answer. Only the `any`
direction (`target_truthy: true` in `any_all_gen_cond`, `:4162`) may stop early. Wiring
early exit into both directions would be a new divergence in the opposite direction. This
correction has been posted to #932.

### The `ltrimstr` trap

`result_to_owned_ctrl`'s doc comment (`:1450-1463`) documents that jq desugars `f(x)`
roughly as `x as $b | body`, runs `body` on the **first** output, then **resumes
backtracking into `x`**. Verified:

```
$ jq -cn '["abcabc" | ltrimstr(("a", ("B"|stderr)))]'
["bcabc","abcabc"]                                   stderr: B    <- jq DID evaluate the tail
$ succinctly jq -cn '["abcabc" | ltrimstr(("a", ("B"|stderr)))]'
["bcabc"]                                            stderr: B
```

**Therefore `result_to_owned` must not be blanket-converted to a stopping sink.** A
consumer may install a `Stop` sink only when it can show that, in jq's `as`-desugaring,
control never returns to the generator. Exactly two justifications qualify:

1. **The body escapes unconditionally** — `halt_error`, whose body terminates the process,
   so backtracking never resumes.
2. **jq's own definition contains the short-circuit** — `def first(f): label $out | (f,
   break $out);`, `def isempty(g): label $out | (g|false, break $out), true;`, `limit`'s
   `foreach … break $out`, `nth` via `last(limit($n+1; f))`, `any`/`all` via
   `isempty(first(…))`, `IN` via `any`.

`builtin_halt_error` is the **only** one of `result_to_owned`'s 20 production call sites
that may change. (The missing-second-output half of that example is #1279, a separate
issue this design must not make worse.)

## How each consumer is rewritten

| Consumer                           | Line                   | Currently                                                                      | Sink it installs                                      | Stage |
|------------------------------------|------------------------|--------------------------------------------------------------------------------|-------------------------------------------------------|-------|
| `builtin_isempty`                  | `:24109`               | `eval_single` + 8-arm match                                                    | first item ⇒ `Stop`                                   | 2     |
| `eval_first_expr`                  | `:16984`               | `eval_single` + `.next()`                                                      | first item ⇒ record + `Stop`                          | 2     |
| `builtin_first_stream`             | `:23952`               | same                                                                           | same                                                  | 2     |
| `eval_limit`                       | `:16890`               | `eval_single` + `.take(n)`                                                     | count ⇒ `Stop` at `n` (after the `n==0` return)       | 2     |
| `builtin_limit`                    | `:23884`               | same                                                                           | same                                                  | 2     |
| `eval_nth_expr`                    | `:17062`               | `eval_single` + `.nth(n)`                                                      | keep index `n`, `Stop` there                          | 2     |
| `builtin_nth_stream`               | `:24024`               | same                                                                           | same                                                  | 2     |
| `any_all_gen_cond`                 | `:4162`                | `eval_owned_expr_fork` + probe loop                                            | probe per element ⇒ `Stop` on match / escape          | 2     |
| `builtin_upper_in`                 | `:3759`                | `eval_owned_expr_fork` + `.any(eq)`                                            | equality per candidate ⇒ `Stop` on match              | 2     |
| `builtin_halt_error`               | `:24983`               | `eval_single` + `result_to_owned`                                              | first item ⇒ `Stop` (justification 1)                 | 2     |
| `eval_first_or_last_generic`       | `eval_generic.rs:3121` | `eval_single` + `.next()`                                                      | see Stage 2b                                          | 2b    |
| `resolve_leaf` catch-all           | `:13813`               | `eval_owned_multi_keep_partial`                                                | first item ⇒ shape check, record, `Stop`              | 3     |
| `builtin_paths_filter`             | `:19590`               | `for path in all_paths { … push }`                                             | *producer*: push each surviving path, honour `Demand` | 3     |
| `binary_fanout_core` right operand | `:1772`                | `push_owned_values` + nested loop                                              | outer loop becomes the sink; #910's order preserved   | 4     |
| **`eval_last_expr`**               | `:17022`               | **unchanged** — cannot short-circuit; its `Partial` drop-the-prefix rule stays | —                                                     | —     |
| **`builtin_last_stream`**          | `:23987`               | **unchanged**, same reason                                                     | —                                                     | —     |

### Worked example — `builtin_isempty`

```rust
/// jq: `def isempty(g): label $out | (g|false, break $out), true;`
///
/// The `break $out` fires on `g`'s *first* output, so `g` is never asked for a
/// second one — that `break` is exactly `Demand::Stop`. Expressing it as a sink
/// replaces the eight-arm case analysis this function grew (#882, #791, #867)
/// with three structural arms: every oracle fact those comments record now falls
/// out of `Flow`'s shape instead of being re-derived per variant.
fn builtin_isempty<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr, value: StandardJson<'a, W>, optional: bool,
) -> QueryResult<'a, W> {
    match eval_each::<W, S>(expr, value, optional, &mut |_item| Demand::Stop) {
        // `g` produced an output and we stopped it there. `pending` (only ever
        // `Some` from an eager fallback's already-raised trailing control) is
        // deliberately dropped: real jq never reaches it, and this reproduces
        // today's answers exactly — `isempty(1, error("x"))`,
        // `isempty(1, ("m"|halt_error(3)))` and `isempty(1, break $out)` all
        // answer `false`, exit 0, oracle-confirmed.
        Flow::Stopped { pending: _ } => QueryResult::Owned(OwnedValue::Bool(false)),

        Flow::Exhausted => QueryResult::Owned(OwnedValue::Bool(true)),

        // A *bare* escape — `g`'s very first "output" was itself an
        // error/break/halt, so there is nothing to answer with. All three must
        // propagate rather than be answered `true` (#882/#791/#867).
        // `control_to_result` (`:4106`) preserves Halt/Break by construction.
        Flow::Escaped(control) => control_to_result(control),
    }
}
```

Every currently-documented case maps: `None`/empty `Many` → `Exhausted` → `true`; bare
`Error`/`Break`/`Halt` → `Escaped` → propagate; `Partial(_, _)` → the prefix's first value
hits the sink → `Stopped` → `false`. The asymmetry the existing comment block spends 40
lines justifying becomes the difference between `Escaped` and `Stopped`.

`any_all_gen_cond`'s rewrite follows the same shape, using `eval_each_owned` (so its
`to_owned` + owned bridge is preserved byte-for-byte) and carrying `cond`'s own escape
out-of-band per [How a sink raises](#how-a-sink-raises). `any_all_probe_element` (`:4091`)
and `any_all_f` (`:4118`, the container form) are **unchanged** — the latter iterates
already-materialized container elements, which is pure navigation and leaks nothing.

## Where `Stop` meets `Partial`, `Halt`, `Break` and `optional`

### `Partial` disappears from the lazy path

`Partial(prefix, control)` exists (#400, #494) because the eager API has one return slot
and must carry "these outputs, then this terminator". A sink has two channels: the pushes
*are* the prefix, `Flow` *is* the terminator. `partial()` (`:1328`) is untouched and only
reached at the eager boundary — inside `eval_single`, and inside `collect_each`.

### `Halt` before the stop is structurally impossible on the lazy path

A raised `Halt` means the producer aborted, so it cannot subsequently have delivered a
value that triggered a `Stop`. It always arrives as `Flow::Escaped(Control::Halt(_))`,
after every value it did produce, and every consumer's existing bare-`Halt` arm handles it
unchanged.

The case *does* arise on the **fallback** path, where `eval_single` returned
`Partial([v₁ …], Halt(c))` — the halt was already raised before any push.
`Flow::Stopped { pending: Some(Halt(c)) }` carries it back so **each consumer keeps its
own existing, individually verified policy**:

- `first`/`limit`/`nth`/`isempty`/`any`/`all` **drop** it once satisfied. Verified:
  `first(1, ("BOOM"|halt_error(3)))` is `1`, exit 0 in jq.
- `resolve_leaf` **keeps** it. Its own comment (`:13836`) argues this at length: a script
  relying on `halt_error` as a hard, uncatchable abort must not have it downgraded into a
  catchable "Invalid path expression".

**This is why `Flow::Stopped` carries `Option<Control>` rather than deciding centrally.** A
central rule would have to pick one of two policies that are both currently correct and
both currently oracle-verified, violating the never-a-regression invariant for whichever it
lost. Once an arm is lazified, `pending` becomes `None` for that shape and the question
evaporates.

### A `Break` addressed to an outer label

- **Bare** (`isempty(break $out)`): nothing pushed, `Flow::Escaped(Break("out"))`, the
  consumer's existing arm propagates, `eval_label` (`:2926`) catches it. Verified exit 0,
  no output.
- **After an output** (`label $o | [isempty(1, break $o)]`): the sink stops on `1`, so
  under the lazy `Comma` arm **`break $o` is never evaluated at all** — exactly jq's
  semantics, and strictly *more* faithful than today's evaluate-then-discard. Verified
  `[false]`, exit 0, both.

`Control::Halt`'s guarantee (`error.rs:56-63`) — that `try`/`catch` and `label`/`break`
pass it through via their existing `other => other` fallthrough — is unaffected, because
`eval_try` (`:2870`) and `eval_label` are **not** lazified in any stage here.

### `optional` (`?`)

`eval_each` **never inspects `optional`**; it only threads it, byte-identically to
`eval_comma`/`eval_pipe`/`eval_single`. Two consequences:

1. **`?`/`try` semantics cannot change in Stages 2–3**, because `Expr::Optional` and
   `Expr::Try` have no lazy arm. Every `?` boundary goes through
   `drain_result(eval_single(…))`, i.e. through `eval_try` verbatim, preserving #693's
   "don't force `optional` down the subtree" fix and #1069's `.[EXPR]?` carve-out.
2. `finish_fork` (`:1383`) is **outside the blast radius** — its callers are
   `reduce`/`foreach`/`while`/`until`/`binary_fanout_core`, none a prefix consumer. Its
   doc comment is still the governing precedent: `optional` never silences a `Break` or
   `Halt`, only a trailing `Error`, and only after the `?`/`try` boundary has had its
   chance. `eval_each` obeys it by making no `optional` decision at all.

## Blast radius

### What changes

| Stage | New code                                                                                                                                      | Function bodies changed |
|-------|-----------------------------------------------------------------------------------------------------------------------------------------------|-------------------------|
| 1     | none (tests only)                                                                                                                             | none                    |
| 2     | `Demand`, `Item`, `Flow`, `eval_each`, `eval_each_pipe`, `eval_each_owned`, `drain_result`, `collect_each` — ~250 lines in a 46,898-line file | 10                      |
| 2b    | one narrowing arm in `eval_generic.rs`                                                                                                        | 1                       |
| 3     | `Builtin::PathsFilter` lazy arm                                                                                                               | 2                       |
| 4     | demand-aware outer loop                                                                                                                       | 3                       |
| 5     | six new arms (`each_if`, `each_try`, `each_label`, `each_as`, `each_as_pattern`, `each_pattern_alternatives`, `each_limit`) + one inline arm  | `eval_each` only        |

### What does **not** change

- **`QueryResult`, `Control`, `EvalEscape`** — no new variants, no changed contract.
- **`eval_single` (`:546`), `eval_comma` (`:853`), `eval_pipe` (`:9953`),
  `eval_owned_input` (`:16650`), `eval_owned_pipe` (`:10111`)** — bodies untouched.
- **113 `eval_single` call sites** — 0 change in Stage 2; consumers' own calls are
  *replaced*, not modified in place.
- **13 of 15 `eval_owned_expr_fork` sites**; **8 of 9 `eval_owned_multi_keep_partial`
  sites**; **19 of 20 `result_to_owned` sites**.
- **`partial`, `finish_fork`, `prepend`, `push_owned_values`, `push_truthiness`,
  `eval_fanout`, `result_to_owned{,_ctrl,_full}`** — unchanged; `drain_result` is a
  *sibling* of `push_owned_values`, not a replacement.
- **`eval_try`, `eval_label`, `eval_if`, `eval_alternative`, `eval_array_construction`,
  `eval_reduce`, `eval_foreach`, `map_over`** — unchanged.
- **`eval_generic.rs`'s `Expr::Comma` (`:3018`), `Expr::Pipe` fold, `LazySeq`/`LazyKeys`** —
  unchanged.
- **`src/jq/lazy.rs`, `jq_runner.rs`, `yq_runner.rs`, `src/jq/stream.rs`** — unchanged.
- **`no_std`** — `dyn FnMut` needs no allocator; `Item`/`Flow` are plain data. (Note
  `write_stderr` is `#[cfg(feature = "std")]` with a no-op twin at `:24934`, so none of
  these bugs is observable under `no_std` — this is a std-only fix.)

### The one real cost this creates

Comma and Pipe semantics exist **twice** in `eval.rs` (eager `eval_comma`/`eval_pipe`,
lazy `eval_each`). That is the price of additive-not-rewrite. Mitigations, in order:

1. The `collect_each` differential test over the `jq_tests.rs` corpus mechanically catches
   drift.
2. Each lazy arm's doc comment names its eager twin's line and states "mirrors it exactly,
   minus the accumulator".
3. The post-Stage-5 slice (explicit non-goal above) collapses the two by reimplementing
   `eval_comma`/`eval_pipe` as `collect_each(eval_each(…))`, only once the differential has
   been green across a release cycle.

## Staged delivery

Each stage is one issue, one PR, independently shippable and independently verifiable.

**Stage 1 — characterization, no behaviour change.** Pin both tables above in
`tests/jq_cli_tests.rs` via `run_jq_full` (`:93`, which spawns `CARGO_BIN_EXE_succinctly`
directly, so stderr is the binary's alone — `run_jq_stdin_streams` goes through
`cargo run` and is unusable here). Two test functions, mirroring #1284's split:
`test_short_circuit_side_effect_shapes_already_match_jq_820` (the trap set — **the
high-value one**, and what a too-eager `Stop` would break) and
`test_short_circuit_side_effect_leaks_820_932_987` (the divergence set, asserting today's
*leaked* stderr so the fix's diff shows exactly which leaks closed). Template:
`test_non_pipe_path_expressions_still_raise_986` (`:13822`).

There are currently **zero** tests anywhere mentioning #820, #932 or #987, and the jq
golden corpus cannot pin these: `Case::expected_status` is `Option<i32>` with "`None`
means jq exits 0 and **stderr is not asserted**" (`tests/jq_golden_tests.rs:66`), and the
loader requires a non-zero status wherever `expected.err` is present. These cases exit 0
with empty stderr, so `jq_cli_tests.rs` is the only home unless that invariant is relaxed.

**Stage 2 — `eval.rs`: the mechanism plus 10 consumers.** Closes #820 (both repros
including the exit-code bug, the paren spelling, and the `isempty` data-loss shape) and
#932's `any`/`IN(s)` half.

**Stage 2b — `eval_generic.rs`'s `first`/`last` arm.** Without this,
`succinctly jq -n 'first(1, stderr)'` and the `first(1, input)` data-loss shape still leak
after Stage 2, because the CLI never reaches `eval.rs`'s `eval_first_expr` for them. Three
options:

- **(a) Narrow the native arm** by a syntactic "contains `Builtin::Stderr`/`HaltError`"
  predicate. **Rejected** — a syntactic I/O predicate rots, and it would not catch `input`.
- **(b) One special case:** `first(Comma(exprs))` ⇒ return the first sibling that yields an
  output, paren-unwrapping as it goes. ~25 lines, closes `first(1, stderr)` and
  `first(1, input)` exactly. Does **not** close `first(.[] | stderr)`, which needs real
  backtracking.
- **(c) Mirror `Demand`/`Item`/`Flow` into `eval_generic.rs`** for `Comma`/`Pipe`/`Paren`.
  ~150 lines in a 10,124-line file. The principled endgame, and the only option that closes
  `first(.[] | stderr)`.

  **Recommendation: (b) now, (c) filed by (b)'s own PR.** #607's regression tests
  (`eval_generic.rs:8034`, `:8053`) pin the shapes any option must keep native.

**Stage 3 — `paths(f)` as a lazy producer + `resolve_leaf` as a sink consumer.** Closes
#987. `builtin_paths_filter`'s per-path loop becomes a sink push (mechanical);
`collect_paths` stays eager (pure structural walk, no user filter, leaks nothing).
`resolve_leaf`'s catch-all switches to `eval_each_owned` with a take-first-and-`Stop` sink.
**Its `trackable`-primitive branch must keep an always-`Continue` sink**, because that
branch's `match values.len()` genuinely needs `0`/`1`/`many`; the branch condition is
purely syntactic, so the sink can be chosen before evaluation. Sequenced after Stage 2
because it touches the path resolver, which #1283 Part 2 C says must not be worked
concurrently with cluster A's family.

**Stage 4 — `Expr::Compare`'s outer loop.** Closes #932's remaining `IN(src; s)` half.
Must preserve #910's live-verified nested-loop order. Own oracle sweep. **Implemented**
as `binary_fanout_each` (the loop, sink-driven) plus `binary_fanout_core` (the eager
collecting wrapper, behaviourally unchanged) and an `Expr::Compare` arm in `eval_each`.

**Stage 5 — widen the arm set.** Closes #1462. **Implemented** as six new native arms —
`each_if`, `each_try` (also reached from `Expr::Optional`), `each_label`, `each_as` (the
bare-`$var` bind node the parser actually builds for `EXPR as $x | body`), `each_as_pattern`
(its destructuring/`?//` sibling, `Expr::AsPattern`) and `each_limit` (`Expr::Limit`'s own
demand-*forwarding* gap) — plus one inline arm for `Expr::FuncDef` (pure AST substitution via
the existing `expand_func_calls`, then `eval_each` on the expanded tree — no new function
needed). `Expr::FuncCall` and `Expr::FirstExpr`/`NthExpr` turned out to need no arm at all; see
[Follow-up issues](#follow-up-issues) for why, and for two more findings its own oracle sweep
made along the way.

**Option (c) — mirror the sink into `eval_generic`.** Originally scoped for
`first(.[] | stderr)`. #1309 gave it two more owners, both now pinned in
[`docs/compliance/jq/limitations.md`](../compliance/jq/limitations.md): `inputs | f` does
not interleave (`inputs | input_line_number` is `3 3 3` against jq's `1 2 3`), and
`(., input) | error(...)` raises once where jq raises twice. Both are the eager `Pipe`/
`Comma` in that module, not the input builtins — #1309's own `first(...)` fix sidesteps it
by delegating the whole node to `eval.rs` rather than by making this module lazy.

## Open risks for an implementer to sanity-check

1. **Over-stopping is the failure mode, not under-stopping.** Pin the trap set in Stage 1
   before a line of `eval_each` is written. `limit(2; 1, stderr, 3)`,
   `isempty(empty, stderr)` and `all(2, (5|stderr); .==2)` are the likeliest to break.
2. **The `ltrimstr` trap.** Resist converting `result_to_owned`'s other 19 call sites. Any
   new stopping sink must be justified by one of the two rules above, in a comment, citing
   the oracle run.
3. **Borrowck friction in `eval_each_pipe`.** The driver closure captures `sink` by unique
   borrow and reborrows per call while also capturing `inner_escape`. Expect to
   restructure; do not "solve" it by cloning the sink or boxing per value.
4. **`needs_path_context` must gate `eval_each_pipe` before anything else.** It is a
   *whole-pipe* property (`:466`), so a per-stage check is wrong. Getting it wrong silently
   stubs `file_index`/`key`/`parent` to zero defaults — the #715/#1302 failure class, which
   produces wrong *output*, not an error.
5. **`eval_generic.rs` is the real CLI entry point.** Any Stage-2 test written against
   `succinctly jq` rather than the library passes or fails based on which evaluator owns the
   *root* node. Write both a library-level unit test on `full_eval` and a CLI test via
   `run_jq_full`, and expect them to disagree for `first`/`last` until Stage 2b.
6. **Stack depth — the parser deliberately leaves pipe/comma chains uncapped.**
   `src/jq/parser.rs:154-156` records that chained pipes and commas are *not* charged
   against `MAX_EXPR_DEPTH` because they parse iteratively, and
   `test_flat_chains_are_not_charged_against_expr_depth_1156` (`parser.rs:6675`) asserts
   `MAX_EXPR_DEPTH * 4` = 1024-stage pipes must parse, calling a cap "a real regression".
   Today's `eval_pipe` already recurses per stage, so the lazy version is not a new class
   of risk — but it adds a closure frame per stage on top, and every stage's frame stays
   live across the stream. `MAX_EXPR_DEPTH`'s own doc warns it "is not a stack-size
   guarantee" and that some constructs abort at ~96 levels on cargo's 2 MiB test threads.
   **Now guarded** by `test_long_pipe_and_comma_chains_evaluate_without_overflowing`
   (PR #1389), which drives a 1024-stage pipe and a 1024-element comma list through the
   evaluator — `tests/deep_nesting_valid_tests.rs` is entirely *document* depth and covers
   neither. It passes today; re-run it after the lazy `Pipe` arm lands.
7. **`first(.[])`'s pre-existing divergence.** `[{"a":1,"a":2}] | first(.[])` gives
   `{"a":1,"a":2}` here and `{"a":2}` in jq — #607's native generic arm producing a
   jq-divergence in jq mode. Out of scope, but Stage 2b touches that exact arm, so it will
   surface in review. File separately rather than fixing in passing; the arm presumably
   exists for yq's duplicate-key semantics and removing it needs its own yq sweep.
   **Filed as #1385** — where it also turns out to be broader than this one arm (`.`,
   `.[]` and `.[0]` all preserve duplicate keys in jq mode), and the undocumented root of
   the #1251/#1170/#443 symptom family. **Unverified** which yq behaviours depend on the
   arm staying native.
8. **`eval_each_owned`'s `pipe_of(rest)` allocation.** `eval_owned_pipe` already clones
   `exprs.to_vec()` per owned intermediate value; the lazy twin inherits that. A
   borrowed-slice variant would avoid it but is out of scope — do not "improve" it in the
   same PR.

## Verification approach for the follow-up implementation PRs

A correctness/fidelity fix, not a performance one — no benchmark gate, but an oracle
differential gate at least as rigorous as #1282's.

- Compare **stdout, exit code, *and* stderr** against pinned jq 1.7.1 for every table row,
  captured separately. A sweep that only diffs stdout is vacuous for this issue.
- **Include the `input`-consumption shapes.** They need a multi-document stdin and a
  control run (`.id` alone) to distinguish consumption from a parsing difference.
- Systematic sweep, not just the tables: cross
  `C ∈ {isempty, first, last, limit(1;·), limit(2;·), limit(0;·), nth(0;·), nth(2;·),
  any(·;true), all(·;false), IN(·), halt_error(1,·), path(·)}` against
  `G ∈ {(1,X), (X,1), (empty,X), (1,2,X), (1|X), ((1,2)|X), ((1,X)),
  if true then (1,X) else 9 end, try (1,X) catch 9, label $o|(1,X), 1 as $v|(1,X),
  first(1,X), limit(3;1,X), [1,X]}` with
  `X ∈ {("B"|stderr), ("m"|halt_error(3)), error("e"), break $o, input, debug, 7}`.
  ~1300 cases; run them all, diff all three streams.
- Run the sweep **twice**: through `succinctly jq` (CLI, `eval_generic` first) and through a
  library harness calling `full_eval` directly. Divergence between the two runs *is* the
  Stage-2b gap, and should shrink to zero by the end of Stage 2b.
- Gate each stage on zero regressions in `jq_cli_tests`, `jq_golden_tests`, `jq_tests`,
  `jq_evaluator_parity_tests`, `jq_error_message_tests`, plus a new deep-pipe test for
  Open Risk 6.
- Add the `collect_each` differential (always-`Continue` sink ≡ `eval_single`) as a
  permanent test.
- **Run the yq suites too.** `eval_generic`'s wildcard fallback means the `eval.rs`
  consumers are shared, and `S::TAG`/`DEFAULT_HALT_ERROR_CODE` differ between modes.

## Critical files

- **`src/jq/eval.rs`** — `eval_single` (`:546`), `eval_comma` (`:853`), `eval_pipe`
  (`:9953`), `needs_path_context` (`:466`), `eval_pipe_with_path_context` (`:17869`);
  `partial` (`:1328`), `finish_fork` (`:1383`), `push_owned_values` (`:1630`),
  `control_to_result` (`:4106`); `result_to_owned` (`:1442`) / `_ctrl` (`:1465`) / `_full`
  (`:1490`, Halt arm `:1511`); the ten consumers (`:24109`, `:16984`, `:23952`, `:16890`,
  `:23884`, `:17062`, `:24024`, `:4162`, `:3759`, `:24983`); the two deliberately
  unchanged (`:17022`, `:23987`); `resolve_leaf` (`:13813`) and `builtin_paths_filter`
  (`:19590`) for Stage 3; `binary_fanout_core`/`binary_fanout_each` for Stage 4; `write_stderr`
  (`:24928`), `builtin_stderr` (`:24961`), `builtin_halt_error` (`:24983`) — the only
  sites with evaluation-time I/O; `mod remaining_inputs` (`:21428`) for the `input` queue.
- **`src/jq/error.rs`** — `Control` (`:54`), `EvalEscape` (`:120`) and their doc comments;
  `Flow` is designed to obey both. No change required.
- **`src/jq/eval_generic.rs`** — `eval_with_cursor` (`:2138`) / `eval_single` (`:2154`), the
  real CLI entry point; `Expr::FirstExpr`/`LastExpr` (`:2878`) and
  `Builtin::FirstStream`/`LastStream` (`:4274`) routing to `eval_first_or_last_generic`
  (`:3121`) — Stage 2b's target; native `Expr::Comma` (`:3018`).
- **`src/bin/succinctly/jq_runner.rs`** — `eval_with_cursor` call sites (`:2199`, `:2296`),
  which make `eval_generic` the CLI's front door; the input driver loop (`:1023`). No
  change required, but read them before believing any "the fix reaches the CLI" claim.
- **`src/jq/parser.rs`** — `MAX_EXPR_DEPTH` and its flat-chain carve-out (`:154`), and
  `test_flat_chains_are_not_charged_against_expr_depth_1156` (`:6675`), for Open Risk 6.
- **`tests/jq_cli_tests.rs`** — `run_jq_full` (`:93`) / `spawn_jq_full` (`:113`), the only
  helper with clean stderr; the `halt`/`stderr` convention header (`:3469`);
  `test_non_pipe_path_expressions_still_raise_986` (`:13822`) as the template.

## Related

- [#820](https://github.com/rust-works/succinctly/issues/820) — this document's subject.
  Its 2026-08-21 comment records the `input` data-loss finding and the severity
  re-assessment above.
- [#932](https://github.com/rust-works/succinctly/issues/932) — `any`/`all`/`IN`; closes in
  Stage 2 except `IN(src; s)` (Stage 4). Its `all` claim is retracted; see the trap set.
- [#987](https://github.com/rust-works/succinctly/issues/987) — `path()`'s non-primitive
  resolver; closes in Stage 3.
- [#980](https://github.com/rust-works/succinctly/issues/980) — closed incidentally by
  #1283 cluster A (#1288); no longer a dependant.
- [#1279](https://github.com/rust-works/succinctly/issues/1279) — generator-argument
  builtins not fanning out over a multi-output argument. The *opposite* bug, and the reason
  the `ltrimstr` trap must be respected rather than "fixed" here.
- [#723](https://github.com/rust-works/succinctly/issues/723) — implemented `input`/
  `inputs`, which is what turned this issue from a cosmetic leak into data loss.
- [jq-path-trackability-deferral.md](jq-path-trackability-deferral.md) (#1282) — the
  structural template for this document, and the source of the "pin the guard rails before
  you touch anything" discipline (#1284) Stage 1 follows. Its Non-goals section explicitly
  hands #820 off to here.
- [jq-lazy-map-select.md](jq-lazy-map-select.md) (#700/#724/#725) — `LazySeq`, the repo's
  only genuinely pull-based lazy chain, and the source of the `Box<dyn Iterator>` rejection
  this document cites *in support of* its `&mut dyn FnMut` choice.
- [#910](https://github.com/rust-works/succinctly/issues/910) — established
  `binary_fanout_core`'s nested-loop order, which Stage 4 must preserve.
- [#607](https://github.com/rust-works/succinctly/issues/607) — why `eval_generic`'s
  `first`/`last` arm is native at all; Stage 2b's constraint.
- [#1283](https://github.com/rust-works/succinctly/issues/1283) — the cluster plan that
  scheduled this design as Track 2.

## Follow-up issues

The status table at the top of this document is the authoritative list. Recorded here with
the reasoning behind each placement:

1. **Stage 1** — characterization tests. Filed #1386, merged PR #1389. Shipped without
   waiting for this document's review, since it blocked nothing and was worth having even
   if the rest were never built. It earned that: on Stage 2's first build the two tests
   asserting the old wrong behaviour failed and the over-stopping trap set stayed green,
   which is exactly the signal it was written to give.
2. **Stage 2** — `eval_each` + ten `eval.rs` consumers. Filed #1421, merged PR #1424.
3. **Stage 2b** — `eval_generic.rs`'s `first` comma walk (option (b)). Filed #1434, merged
   PR #1435.
4. **Stage 2 gap** — `eval_each_pipe`'s owned-value arm. Merged PR #1450. Not a planned
   stage; see the correction note at the top.
5. **Stage 3** — `paths(f)` producer (`each_paths_filter`) + `resolve_leaf`'s
   stop-after-first sink. Closes #987. Depends on 2. **Implemented.** `Flow::Stopped`
   gained back its `pending: Option<Control>` payload as part of this stage, exactly as
   scoped below — `resolve_leaf` is its one reader, preserving an already-triggered
   `Halt` from an eager-fallback arm rather than downgrading it into a catchable path
   error. One residual, documented rather than closed: an un-lazified `eval_each` arm
   (`If`/`Try`/`Label`/`AsPattern`/`FuncCall`, ...) still leaks a side effect for a
   node's own filter when that filter's shape isn't one of `Comma`/`Pipe`/`Paren`/
   `Builtin::PathsFilter` — pinned as a known-remaining row in
   `test_short_circuit_side_effect_leaks_820_932_987`, left to Stage 5.
6. **Stage 4** — `Expr::Compare`'s outer loop. Filed **#1459**, implemented. #932 was
   closed when Stage 2 landed, which was right for its `any`/`all`/`IN(s)` thirds but left
   `IN(src; s)` — still divergent, and pinned as such in the test suite — with no open
   issue. #1459 carried it. `binary_fanout_core`'s body became `binary_fanout_each` (one
   loop, two operand strategies) and `eval_each` gained an `Expr::Compare` arm. Its own
   oracle sweep — 105 shapes covering all six `CompareOp`s × operand arities, every lazy
   consumer, terminators in either operand, `?`/`try`, and `input` — took the divergence
   count from 18 to 5.

   **Two findings it did not close, both left as separate work rather than folded in.**
   (a) A bare `first(<non-comma>)` never reaches `eval.rs` at all, so
   `first((1,2) == (10, ("B"|stderr)))` stays eager — the same class as
   `first(.[] | stderr)`, i.e. option (c) below. (b) The **eager** copies of the fanout
   loop still finish the right operand before starting the left, where jq interleaves them.
   That is broader than #820: it affects `Expr::Arithmetic` as much as `Expr::Compare`, and
   it lives in three places — `binary_fanout_core` (via `eval_binary_fanout`),
   `eval_binary_fanout_with_path_context`, and `eval_generic.rs`'s own third copy in its
   `Expr::Compare` arm, which is what a top-level `==` actually reaches. With
   `input`/`inputs` operands it changes *values*, not just stderr ordering:
   `(input) + (input, input)` over `"a" "b" "c" "d"` is `["ba","dc"]` in jq and
   `["ca","db"]` here, so it is silent data loss rather than a cosmetic leak. **Filed as
   #1481.**

   **Two things review added afterwards.** (i) #1459's own severity line — "Low (stray
   stderr write; no wrong output, no data loss)" — is true of its `IN(src; s)` repro and
   false of the arm the fix adds. Once a compare's operands can pop `input`, the eager
   fanout's discarded candidates eat documents the CLI's per-document driver loop then
   never processes: `[isempty(.id == (1, input)), .id]` over four documents printed two
   before Stage 4 and prints four after, matching jq. That is the same silent data loss
   #820 was filed for, so Stage 4 closes a `Comma`-arm-equivalent hole in the `Compare`
   arm, not just a stderr leak — pinned separately from the stderr tables in
   `test_compare_operand_consuming_input_documents_1459`, because every stderr row would
   stay green if the arm regressed to eager only for document-popping operands.
   (ii) `binary_fanout_each` has two operands and so can be handed two `Flow::Stopped`
   `pending`s; `abort.unwrap_or(outer)` keeps the *inner* one and drops the *outer* one.
   `resolve_leaf` is the only consumer that reads `pending`, so the effect is confined to
   a `Halt` under `path(...)`, and the drop is the half that lands on jq's verdict (jq's
   `path()` is lazy end to end and never evaluates the trailing `halt_error` at all).
   Left and right therefore answer differently, deliberately: both rows are pinned in
   `test_short_circuit_side_effect_leaks_820_932_987`, and `Flow::Stopped`'s own doc
   comment now records this as its one stated exception.
7. **Stage 5** — widen the lazy arm set. Filed **#1462**, which re-rated it Medium rather
   than the low-priority tail this document predicted: the wrappers destroy `input`
   documents, they do not merely leak stderr. **Implemented** as `each_if`, `each_try`
   (shared by `Expr::Try` and `Expr::Optional`), `each_label`, `each_as` (`Expr::As`, the
   bare-`$var` bind node), `each_as_pattern` (`Expr::AsPattern`, its destructuring/`?//`
   sibling) and `each_limit` (`Expr::Limit`, the one arm needed purely for
   demand-*forwarding* — `FirstExpr`/`NthExpr` needed none, since both already emit at most
   one downstream output and Stage 2's `each_take_first`/`each_take_nth` already drive that
   through their own `eval_each` call). `Expr::FuncDef` needed no new function at all: it is
   pure AST substitution (`expand_func_calls`), so routing the expanded tree through
   `eval_each` instead of `eval_single` is the whole fix. `Expr::FuncCall` needed no arm
   either — `eval_func_call` is unconditionally `"undefined function"`, since every real call
   is resolved by `FuncDef`'s substitution before evaluation ever reaches it.

   **Two things this stage's own oracle sweep found that the original plan didn't
   anticipate.** (i) The parser builds `Expr::As`, not `Expr::AsPattern`, for the bare-`$var`
   form — the plan's own `first(1 as $x | (1,("B"|stderr)))` repro is therefore an `Expr::As`
   case, and needed `each_as` specifically; `AsPattern`/`each_as_pattern` covers only
   destructuring and `?//`-chains. (ii) Every repro in this section's own divergence table
   wraps its generator in a bare `first(...)`, but `eval_generic.rs`'s native `first`/`last`
   routing intercepts a bare `first(...)`/`last(...)` before it ever reaches `eval.rs`'s
   `eval_each` — so **none of Stage 5's new arms is reachable through a bare
   `first(...)`/`last(...)` at the CLI**; that gap is option (c)/#1461, not this stage. Every
   other short-circuiting consumer (`isempty`, `limit`, `nth`, `path`, `any`, `all`, `IN`) has
   no such native `eval_generic` arm and reaches Stage 5's new arms correctly — confirmed via
   `isempty(...)`-wrapped CLI tests instead.

   **One divergence found and deliberately not attempted**: `?//`-alternatives wrapped in a
   short-circuiting consumer re-run once per alternative in real jq (a macro-expansion/`?//`
   retry-on-break interaction, unrelated to demand-forwarding) but not in succinctly. Confirmed
   pre-existing, unaffected by this stage, and a materially larger design question — filed
   separately as **#1519** rather than folded in.
8. **Option (c)** — mirror `Demand`/`Item`/`Flow` into `eval_generic.rs` for
   `Comma`/`Pipe`/`Paren`. Filed **#1461**, also Medium for the same reason (`first(1 | (1,
   input))` consumes documents), with the narrower `first_over_comma_generic` gap it sits
   next to filed separately as **#1451**. This is the only remaining route to
   `first(.[] | stderr)`, which is a `Pipe` and so out of reach of Stage 2b's comma walk.
