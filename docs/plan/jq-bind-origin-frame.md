# The frame witness for a navigated `$var` in path position (#2042)

**Status: implemented.** This note records the design behind
[#2042](https://github.com/rust-works/succinctly/issues/2042), for anyone reading
`Origin::At`/`Frame` in `src/jq/expr.rs`/`src/jq/eval.rs` later and wanting to know why the
witness is shaped the way it is, not just what it does. The doc comments on `Tracked`,
`Origin`, `Frame`, `resolve_bind_source` and `register_identical` are the primary source; this
note is the connective tissue between them, plus the alternatives that were tried and
rejected.

## The problem: a snapshot's value tells you nothing about its node

Before #2042, `Expr::TrackedVar` carried a bare `Rc<OwnedValue>` — a frozen snapshot of what
a `$var` was bound to, recognised at a `path()` use site by comparing values
(`register_identical`). That works for `. as $x`: a value-equal ambient really is the same
node, because a finite tree cannot contain itself as a proper descendant (`#844`/`#1466`'s
argument). It does not work once the snapshot comes from a *navigated* position, because a
document can hold the same value at two different places:

```jq
path(.a as $y | .c | $y)   # on {"a":{"b":1},"c":{"b":1}}
```

jq refuses this — `$y` was bound to the node at `.a`, and `.c` is a *different* node that
happens to hold an equal value. Real jq's own rule is `jv_identical`: pointer equality for
the three heap kinds (string/array/object), exact bit representation for a number, and "same
kind" for `null`/`true`/`false` (jq never allocates those, so equal literally means identical
there). `OwnedValue` has no pointers to compare, so widening the pre-#2042 gate
(`substitute_var_tracked`'s `is_identity_passthrough` check) to also mark navigated bindings,
without changing what a marker is *checked against*, reopens exactly the #1466 class: two
distinct nodes, equal values, one gets the other's path.

A rebuilt copy is the same failure by a different route (`tojson | fromjson`, `{k: .}`,
`[.]`) — the value survives, the node does not — which is why `register_identical` cannot be
fixed by adding "was this value ever mutated" bookkeeping; it needs to know *where the value
came from*, not just what it looks like now.

## The fix: a witness with a place in it

A `$var` bound from a navigated position now carries `Origin::At { invocation, path }`
alongside the frozen value (`Tracked { value, origin }`). In a tree, two nodes are the same
node iff they sit at the same path from the same root, so the witness records exactly that:
the resolver invocation the binding happened in, and the absolute path the source resolved to
within it. `Origin::Snapshot` is the unchanged pre-#2042 case (`. as $x`), still certified by
value alone — a marker's origin is a disjunction, not a replacement rule, and the
`null`/`true`/`false` value-identity arm survives underneath both as a third disjunct
(`register_identical`).

### Why the path alone isn't enough — the frame

A path by itself is still ambiguous, because `path()` can nest and a pipe can navigate to a
different root partway through. Three shapes pin this, all refused by jq and all *accepted*
by a rule that compares bind paths without knowing whose root they're relative to:

| filter                                   | why a frame-blind rule fabricates                                                                                      |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `path(.a as $y \| .x \| (.a \| $y))`     | the inner pipe's register path `["a"]` is relative to `.x`; the marker's `["a"]` is relative to the document root      |
| `.a as $y \| .x \| path(.a \| $y)`       | the binding happened outside `path()` entirely; `path()`'s own root is `.x`, not the binding's root                    |
| `path(.a as $y \| .x \| path(.a \| $y))` | the inner `path(...)` is a *second* resolver invocation with its own root; the outer marker's path means nothing there |

So the witness is `(resolver invocation, absolute path within it)`, not a bare path. `Frame`
(`src/jq/eval.rs`) carries both halves: `invocation` is a fresh `u64` per resolver entry
(`Frame::enter`, from a plain `AtomicU64` counter — a `thread_local!` would not survive a
`no_std` embedding), and `at` is the absolute position, from that invocation's root, of
wherever the path register currently sits. `Frame::certifies` admits an `Origin::At` marker
only when both halves match the current frame exactly; a marker from a different invocation,
or the same invocation at a different position, is refused.

### `None` is the safe default

`at` is `Option<Rc<PathPrefix>>`, and `None` means "not provably known" rather than "at the
root." Every recursion site that resolves against a *different* value passes `None` unless it
can extend `at` by a path it built itself — a site that gets this wrong in the `Some`
direction fabricates a path jq refuses, which is the direction `del()`/`=` then write
through, so the type is shaped so that a wrong omission can only ever cost an acceptance,
never invent one. The one syntactic gate, `may_bind_navigated`, decides whether an expression
can bind a variable from a navigated position *at all* (any `Expr::As`/`AsPattern`, or
anything that can hide one behind a call, `def`, or `Expr::Shared`); when it can't,
`Frame::enter` seeds `at` as `None` from the start, so the per-stage `Frame::extend` calls
along the way are free — an `Rc` clone, not a new walk — for the ordinary write workloads
(`.[] |= f`, `del(.[] | select(..))`) that never bind a `$var` from navigation.

## The three comparison sites

One `Frame::certifies` call, reached from three places, so a fix to the rule can't drift
between copies:

- **`resolve_node_eager`'s `Expr::TrackedVar` arm** — a `$var` reference in path position is
  certified against the frame the resolver is *currently* at.
- **`resolve_seq_stage`'s carried register** (`reestablishes_register`/`carry_register`) — the
  plain-pipe register `#1573` already carries across navigation-free stages gains the frame
  its own position sits in, so `register_identical` can check a carried register the same way
  it checks a fresh `TrackedVar` reference.
- **`FoldRegister`** — a fold's own register (seeded once per INIT fork, `#1466`/`#2046`)
  gains the same `frame` field, taken at `FoldRegister::enter`.

## Where the bind path comes from

`resolve_bind_source` resolves an `as` source in path position — reusing the same
value-vs-path fork `resolve_fold_source` already established for fold sources
(`#1467`/`#2031`) — as **a witness only**: jq evaluates an `as` source with path tracking
suspended (`subexp_nest > 0` in the real interpreter), so the source itself never moves the
register (`path(.a as $y \| .b)` is `["b"]`, not an error) and never raises a path error of
its own. The witness therefore runs only where it is provably the same computation as value
evaluation, on a **closed pure-navigation grammar** (`is_pure_navigation`: `.`, `.a`, `.[n]`,
a slice, `.[]`, `..`/`recurse`, `getpath` of a literal, pipes and commas of those, a literal,
`error`), against a tracked value at a known position, and only when `$var` can reach a
position the resolver dispatches on in the body (`var_reaches_path_position`). Anything else
— `try`, `?`, `//`, `select`, `if`, `first`, a construction, a builtin — binds by value with
no origin, exactly as before. On that grammar the resolver's own refusal (a slice of a
non-array, or navigation of a literal) always escapes, and the fallback re-runs the source by
value with nothing to repeat; any other escape (a genuine `error`/`halt`) keeps the
resolver's own partial prefix, mirroring every other arm's "prefix survives, only the escape
is caught" rule.

The grammar is the review's finding, not the first design. The first cut witnessed every
source and relied on the refusal *escaping*; three reviewers independently showed that a
`try`/`?`/`//` inside the source catches it (jq never raises it) and binds the handler's
value — `del(([1] | try .[0] catch "c") as $y | .[$y|tostring])` deleted key `"c"` where jq
deletes `"1"` — that `getpath`/`..` on a construction raised the other refusal kind as a real
error, and that the fallback repeated side effects (`input` consumed twice) and computed
stages (+25% on `(.tags | map(.) | .[0]) as $y`). A fallback keyed on an error escaping can
never be sound while an error-catching arm can sit between the raise and the fallback; a
grammar with no such arm makes the question moot.

Two more rules came out of the same review. A **slice** witnesses a node only for a non-empty
array (`slice_witnesses_node`): `jv_identical` is allocation identity, a non-empty array
slice shares its parent's buffer, an empty one and a string slice are fresh values — the
first cut certified and wrote through both. And a **marker-headed source** is re-rooted
through nested pipe heads too (`marker_headed`): `$y.b | .c` parses as
`Pipe([Pipe([$y, .b]), .c])`, which a one-level head check missed. A marker anywhere but the
head (`(.c | $y | .b) as $w`) stays refuse-only: honouring it needs an absolute position on
the source-mode branch, the deferred refactor below.
`cannot_move_register`'s `Expr::As` arm follows the same evidence and now consults only the
*body* — the source can no longer block carrying a register through an `as` stage, since it
was never the register moving in the first place.

## What was rejected

- **Pointer identity on `Cow::Borrowed` values** — literally `jv_identical`. Rejected because
  a fold accumulator is owned and reallocated on every step (ABA: two different logical
  values can share an address at different times), and a marker can outlive the frame it was
  built in through `FuncDef`'s call-site cache. This file deliberately models identity
  without pointers.
- **A frame identified by its root *value*** (skip the invocation counter, key frames by what
  `.` looked like at binding time) — [#2642](https://github.com/rust-works/succinctly/issues/2642)
  is the counterexample: `. as $x | {a:1} | ($x.a) = 9` on `{"a":1}` shows the *root*
  marker's own value-only rule already fabricating across a rebuilt-copy boundary, so keying
  frames by value would inherit the identical bug one level up rather than avoid it.
- **Making every branch path absolute up front** (fewer allocations than the current
  parent-chain `PathPrefix`, since nothing would need composing on return) — deferred rather
  than rejected outright; a candidate follow-up if `Frame`/`PathPrefix` traffic ever shows up
  in perf-guard, not attempted here because the allocation is already free (`Rc` clone) on
  every path this change doesn't widen — and, since the review, the follow-up that would let
  a marker anywhere in a source resolve to its own absolute node.

## Measured cost

Interleaved A/B on the pinned boxes (Apple M4 Pro, AMD Ryzen 9 7950X; 11 reps + warm-up,
min and median; `users` corpora at 1 MB and 10 MB; base `main @ baa26d72e`), with cachegrind
instruction counts on the 7950X at 1 MB. Output was byte-identical on all 29 workload×size
pairs per machine, both rounds.

| workload                                                    | 7950X Δ med    | M4 Pro Δ med   | Ir drift, 1 MB |
| ----------------------------------------------------------- | -------------- | -------------- | -------------- |
| `.users[] \|= (.score = 1)`                                 | +0.2 … +1.2%   | −1.6 … −2.1%   | −0.00%         |
| `del(.users[] \| select(.score < 100))`                     | +1.2 … +2.0%   | +0.8 … +1.5%   | +0.17%         |
| `[paths] \| length`                                         | +0.6%          | +0.2%          | −0.00%         |
| `[path(.users[] \| .age)] \| length`                       | +0.5 … +3.0%   | **−25 … −31%** | −0.00%         |
| `(.users[] \| . as $r \| .score) \|= 1`                   | +2.3 … +4.0%   | −0.5 … +0.3%   | +1.62%         |
| `(.users[] \| . as $r \| $r.score) = 1`                    | +2.7 … +4.9%   | +1.3 … +1.8%   | +1.61%         |
| `del(.users[] \| .score as $y \| select($y < 100))`        | +5.1 … +6.7%   | +7.0 … +8.8%   | +5.51%         |
| `[path(.users[] \| .score as $y \| .age)]` (100 KB/300 KB) | −2.3 … −6.2%   | −0.0 … −0.5%   | —              |

Three findings, in order of what they cost:

- **No `as` in the target: neutral.** The write and `path()` workloads execute a bit-identical
  amount of work (−0.00% instructions); wall clock sits inside the ±2.5% base-vs-base control
  band. The x86 run carried a +1–2% whole-run offset (rows cachegrind proves identical read
  +0.9 … +3.5%), so read the 7950X column net of that. The M4 Pro's −25 … −31% on
  `[path(.users[] | .age)]` is real but not less work — identical instruction counts — so it
  is ARM code layout, not portable, and not claimed.
- **A navigating binding in a `del`/write target** first measured **+21% (7950X) / +31%
  (M4 Pro), +30.9% instructions**. The suspect was the witness resolve; sampling charged it
  elsewhere: `select`'s condition `$y < 100` now held an `Expr::TrackedVar` where the
  pre-#2042 substitution put an `Expr::Literal`, and #2048's owned fast path admitted only the
  literal, so every branch went through `to_json_for_reindex` + `JsonIndex::build`. Admitting
  the marker there (`da36d45bb`) cut it to **+5.5% instructions, +5 … +9% wall clock**; the
  demand gate (`var_reaches_path_position`) then removes the witness itself for that shape,
  since `$y` never leaves `select(..)`. The `. as $r` shapes' +1.6% instructions is the
  per-stage `Frame::extend` on a gate-true target, recorded, not chased.
- **perf-guard passed at −0.0% on all six queries, both rounds** — and could not have seen
  either the regression or its fix, because its matrix has no `path()`/`del()`/assignment
  target and no `as`. [#2655](https://github.com/rust-works/succinctly/issues/2655) adds the
  rows. Two pre-existing quadratic `path()` shapes surfaced by the 10 MB scaling point are
  [#2656](https://github.com/rust-works/succinctly/issues/2656).

## What remains

- [#2642](https://github.com/rust-works/succinctly/issues/2642) — the pre-existing
  root-marker (`Origin::Snapshot`) fabrication across a rebuilt copy, unrelated to the frame
  witness and not touched by it.
- **Value-mode bindings.** `eval_as` (the non-path-tracked evaluator) still binds with no
  path at all, so `.a as $y | path(.a | $y)` — jq `["a"]` — stays refuse-only. Closing it
  needs document-absolute identity reachable from a value-mode cursor plus
  `needs_path_context` routing to decide when to pay for it; it is the accepting-direction
  twin of #2642 and belongs in the same follow-up, not here.
- [#2646](https://github.com/rust-works/succinctly/issues/2646) — `first`/`last`/`add`
  navigating inside their own jq-level definitions against a *constructed* value inside
  `path()` never raise, found by `scripts/jq-bind-origin-fuzz.py`'s differential fuzz and
  confirmed pre-existing (not caused by this change).

See [docs/compliance/jq/limitations.md](../compliance/jq/limitations.md) (shape #1, under
"Where succinctly errors and jq does not") for the user-facing record of what's closed and
what's still refuse-only, and `scripts/jq-bind-origin-oracle-sweep.sh` /
`scripts/jq-bind-origin-fuzz.py` for the verification matrices this design was checked
against.
