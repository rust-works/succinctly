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

| filter                                    | why a frame-blind rule fabricates                                                  |
|--------------------------------------------|--------------------------------------------------------------------------------------|
| `path(.a as $y \| .x \| (.a \| $y))`        | the inner pipe's register path `["a"]` is relative to `.x`; the marker's `["a"]` is relative to the document root |
| `.a as $y \| .x \| path(.a \| $y)`          | the binding happened outside `path()` entirely; `path()`'s own root is `.x`, not the binding's root |
| `path(.a as $y \| .x \| path(.a \| $y))`    | the inner `path(...)` is a *second* resolver invocation with its own root; the outer marker's path means nothing there |

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
its own. An untracked-navigation refusal from the resolver falls back to the ordinary value
evaluator with no origin; any other escape (a genuine `error`/`halt`) keeps the resolver's own
partial prefix, mirroring every other arm's "prefix survives, only the escape is caught" rule.
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
  every path this change doesn't widen.

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
