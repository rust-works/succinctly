# A general lazy `map`/`select` primitive for `eval_generic.rs` (#700)

**Status: design only, not implemented.** This document is the deliverable for issue
[#700](https://github.com/rust-works/succinctly/issues/700) — a follow-up from
[#686](https://github.com/rust-works/succinctly/issues/686) (measurement spike) and
[#140](https://github.com/rust-works/succinctly/issues/140)/[#678](https://github.com/rust-works/succinctly/pull/678)
(M3.1, which shipped `GenericResult::LazyKeys`/`LazyIndexRange`). No code in this repo has
been changed to implement it; see "Follow-up issues" at the bottom for where implementation
is tracked.

## Problem

`keys_unsorted | map/select` falls back to full eager materialization in
`src/jq/eval_generic.rs`, costing **4.6-4.8x time and up to 13.5x memory** at 100MB scale on
a flat-object (`wide`) pattern (#686's measurement, Apple M5 Max, byte-identical output vs
system `jq` verified). Both ratios grow monotonically with input size — an algorithmic cost,
not a fixed constant.

| Size  | floor `keys_unsorted\|length` (ms/MB) | `keys_unsorted\|map(.)` (ms/MB) | time ratio | mem ratio |
|-------|---:|---:|---:|---:|
| 100kb | 5.3ms / 8.7MB      | 7.3ms / 11.2MB     | 1.39x | 1.29x |
| 1mb   | 10.7ms / 9.9MB     | 34.6ms / 33.2MB    | 3.23x | 3.34x |
| 10mb  | 73.3ms / 21.8MB    | 308.9ms / 205.3MB  | 4.21x | 9.42x |
| 100mb | 681.3ms / 138.9MB  | 3259.4ms / 1872.6MB| 4.78x | **13.48x** |

Critically, #686 also measured that **75-95% of this same cost hits plain `map(.)`** on the
same document at the same size, with no `keys_unsorted` involved at all — `Builtin::Map` has
*zero* arms anywhere in `eval_builtin`; every `map(f)` call falls through an unconditional
wildcard fallback (`src/jq/eval_generic.rs`, near the end of `eval_builtin`):

```rust
// For other builtins, fall back to full evaluator via JSON
_ => {
    let owned = to_owned(&value);
    eval_on_owned::<S, _>(&Expr::Builtin(builtin.clone()), owned, optional)
}
```

`to_owned` recursively materializes the whole subtree (one `IndexMap`/`Vec` allocation per
container node, one `String` per key/scalar). `eval_on_owned` then re-serializes that tree
back to a JSON string, rebuilds a brand-new succinct index over it (`JsonIndex::build` — SIMD
interest-bit scan, balanced-parens bit counting, rank-directory construction), and
re-evaluates through the *separate* `eval.rs` evaluator. Four expensive full-tree passes,
regardless of how little of the container `map`/`select` actually needs to touch.

The only existing laziness mechanism, the `Expr::Pipe` fold in `eval_generic.rs`, works by
pattern-matching the **literal next AST node** against `GenericResult::LazyKeys`/
`LazyIndexRange` (the #678 fast path), special-casing exactly `length`/`.[]`/`.[n]`/`first`/
`last`. This is why it can't compose: nothing `map`/`select` produces today is itself a
recognized lazy shape, so `keys_unsorted | map(f) | select(g)` can't stay lazy past the first
stage even if `map` alone grew a fast path.

## Scope decision

**General primitive, staged delivery.**

#686's own attribution table (75-95% of the cost is *not* `keys_unsorted`-specific) rules out
a `keys_unsorted`-only patch as the *final* state — that would cap the achievable win at the
~5-25% that's actually keys-specific and leave the dominant cost untouched. This exact
deferral has already happened twice: #678 deferred the general fix once ("would mean
building it generally, a bigger unrelated change"), #686 deferred it again pending
measurement. #700 is the issue that was supposed to resolve that deferral; picking "narrow"
as the final scope here would be a third deferral in different clothes.

Delivery is still staged narrow-first, because the mechanism below splits into two
independently-mergeable, independently-measurable slices without being redesigned between
them:

| Slice | Covers | Why this order |
|---|---|---|
| **1** | `keys_unsorted\|keys \| map/select` — wire the new primitive into the *existing* `LazyKeys`/`LazyIndexRange` arms of the `Pipe` fold. | Smallest diff, touches only already-well-tested machinery, retires the three fallback-pinning tests below, immediately measurable with the `QueryType::KeysUnsortedMap`/`KeysUnsortedSelect` benchmarks #686 already added. |
| **2** | Plain `.foo \| map/select`, `arr \| map/select` — `Builtin::Map`'s first-ever native arm in `eval_builtin`. | This is where the dominant 75-95% of the measured cost actually lives, so it is **not optional** — scoped alongside Slice 1, sequenced second because it's structurally larger (first real semantics for a previously-absent builtin arm). |
| **3** *(deferred, own issue, not part of #700's implementation)* | CLI streaming-output integration: `yq_runner.rs`'s `can_use_m2_streaming` whitelist, true zero-`Vec` CLI output. | Layered on top of an already-general core, same shape as #685 extending `stream_json`/`stream_yaml` *after* #678 made `keys_unsorted` lazy at the evaluator level. |

### Non-goals (explicit, to prevent scope creep)

- **Unifying the three parallel lazy-representation families** —
  `GenericResult::LazyKeys`/`LazyIndexRange`, `JqValue::LazyKeysArray`/`LazyIndexRange` in
  `src/jq/lazy.rs`, and `yq_runner.rs`'s `can_use_m2_streaming` whitelist. Each serves a
  different layer with different constraints (evaluator-internal laziness vs. JSON
  CLI-output laziness vs. YAML forward-only-cursor streaming eligibility). Tempting, but its
  own project — matches #685's own precedent of declining to generalize `JqValue`'s `Copy`
  bound even though it was "more correct" in the abstract.
- **Sorted `keys | map/select` staying as lazy as `keys_unsorted`.** Sorting requires
  observing every key before emitting the first one — a different complexity class, not a
  narrower version of the same problem. Falls back to the eager path indefinitely; this does
  not contradict the "general" scope decision above.
- **YAML merge-key mappings** (`YamlFields::Merged`, already eager today). Stays eager;
  forcing it lazy conflicts with the forward-only constraint below.
- **Recursive laziness** (`map(map(f))` where the *inner* `map` also stays lazy),
  **`map_values`**, and **any backward/lookahead access on YAML**.

## Format-access asymmetry that shapes the design

JSON tolerates real random access cheaply (interest-bit rank/select, O(log n) worst case, no
penalty for jumping around). YAML does not — `AdvancePositions`/`CompactEndPositions`
(`src/yaml/advance_positions.rs`, `src/yaml/end_positions.rs`) keep one document-wide
*sequential* cursor optimized for monotonically-increasing access; any backward/out-of-order
access falls to `get_random`, which resets the incremental scan to position zero, penalizing
whatever sequential access follows.

**The design must be forward-only, single-pass, with no buffering of earlier elements.** This
is a hard constraint, not a nice-to-have — a violation is invisible in JSON-only benchmarks
and only shows up as a YAML-specific regression, the same category of trap this repo's
benchmarking discipline (`docs/guides/benchmarking.md`) already warns about for
memory-bound effects across architectures.

## Building blocks already in the codebase

`src/jq/document.rs`'s `DocumentFields`/`DocumentElements` traits expose cons-list `uncons()`
(pop one, return "the rest" as a new `Self`) — genuinely lazy per-step for both JSON
(`JsonFields`/`JsonElements`, thin `Copy` wrappers over a BP cursor, `src/json/light.rs`) and
YAML (`YamlFields`/`YamlElements`, same, except merge-key mappings which are already-eager
and `Rc`-shared, `src/yaml/light.rs`). These traits are **not dyn-compatible** (`Self: Sized`
bounds throughout, several methods return `Self` by value) — the design below stays in fully
static generics, no `Box<dyn Iterator>`.

`src/jq/stream.rs`'s `stream_lazy_keys_json`/`stream_lazy_keys_yaml` (added by #685) are the
closest existing precedent: generic over `F: DocumentFields`, pull one field at a time via
`uncons()`, write straight to a `core::fmt::Write` sink, no intermediate `Vec`/`OwnedValue`.
They're a terminal *output* sink, not a composable intermediate value — but the design below
reuses exactly this idiom.

## The mechanism

### Types (new, in `eval_generic.rs`)

```rust
/// One pending element: still a live pointer into the source document,
/// or a value an earlier `map` stage computed that no longer corresponds
/// to one node in the source.
enum LazyElem<V: DocumentValue> {
    Cursor(V::Cursor),
    Owned(OwnedValue),
}

/// The four possible starting points for a lazy chain.
enum LazySource<V: DocumentValue> {
    Elements(V::Elements),                  // bare `arr | map(f)`      -- Slice 2
    Values(V::Fields),                      // bare `obj | map(f)`      -- Slice 2
    Keys(V::Fields),                        // `keys_unsorted | map(f)` -- Slice 1
    IndexRange { next: usize, len: usize }, // array `keys_unsorted | map(f)` -- Slice 1
}

/// One deferred stage. Only ever `Map` -- `select(g)` composes as
/// `Instruction::Map(Builtin::Select(g))`, reusing the builtin's
/// existing whole-value pass/drop logic verbatim (see "select" below).
enum Instruction {
    Map(Rc<Expr>, EvalTag),   // EvalTag: Jq | Yq, a new tag on EvalSemantics
}

/// A composed, not-yet-materialized `map`/`select` chain. `source` never
/// rewinds (forward-only by construction -- consumed cons cells are
/// simply gone). `instructions` grows by one Rc-shared entry per
/// composed stage, so an arbitrary-length chain is ONE value, not one
/// type per depth. `pending` buffers only the current source element's
/// own fan-out (0..N outputs from `,`/`empty` inside one stage), never
/// an earlier or later element.
struct LazySeq<V: DocumentValue> {
    source: LazySource<V>,
    instructions: Rc<Vec<Instruction>>,
    pending: Vec<LazyElem<V>>,
}
```

Add one variant to the existing enum:

```rust
pub enum GenericResult<V: DocumentValue> {
    // ...existing variants unchanged...
    LazySeq(LazySeq<V>),
}
```

**No new lifetime parameter on `GenericResult<V>`** — every field of `LazySeq<V>` is either
`V`'s own associated type or `Rc`/`Vec`-owned data (`Rc<Expr>`, plain owned AST, already
`Clone`). A `Box<dyn Iterator<Item=...> + 'a>` alternative was considered and rejected for
this reason: it would force naming the document's borrow lifetime on `GenericResult` itself,
rippling through roughly 15 functions in `eval_generic.rs` plus `jq_runner.rs`/
`yq_runner.rs`, for a representation that also isn't `Clone` (forgoing replay for `last`/
negative-index/sorted-`keys`-after-`map`) and pays a vtable hop per element per stage.
`LazySeq` stays plain owned/`Copy`/`Clone` data throughout, so it's trivially cloneable for
exactly those replay cases — materializing on demand costs no more than a YAML `get_random`
rescan already costs today.

`Instruction` deliberately holds only `Map` — not one variant per AST shape (`Length`,
`Iterate`, `Index(n)`, ...) mirroring today's ad hoc matching. Everything else (`length`,
`.[]`, `first`, whole-value `select`) is handled by the *consumer* of a `LazySeq` (the `Pipe`
fold), not baked into the chain. This is what actually buys composability: the `Pipe` fold's
`GenericResult::LazySeq(seq) => ... Expr::Builtin(Builtin::Map(h)) =>
GenericResult::LazySeq(seq.push_map(...))` arm is self-recursive by construction — it doesn't
need a twin per chain depth, because pushing one more instruction returns the same variant,
not a new one.

### Iteration

`LazySource::advance(&mut self)` calls `uncons()`/`uncons_cursor()` and stores "the rest"
back into itself — no backward-moving method exists anywhere in the type. `LazySeq`
implements `Iterator<Item = Result<LazyElem<V>, Control>>`: pull one source element, fold it
through `instructions` in order (each stage's output can fan out via `,`, drop via
`empty`/falsy, or itself be any `GenericResult` shape — normalized through a shared
`into_lazy_items` helper), buffer the resulting 0..N items in `pending`, and yield them one
at a time before pulling the next source element. Array construction is atomic (mirrors
`eval.rs`'s existing `map_over`): a mid-chain error discards the whole in-progress element,
not just downstream output.

### `select`: no new code path

`Builtin::Select`'s existing native arm (tests `cond` once against the whole current value,
republishes it unchanged if truthy — `def select(f): if f then . else empty end;`) is
**untouched**. This design adds no lazy fast path for whole-value `select`.
`keys_unsorted | map(f) | select(g)`'s `select(g)` still ends up testing a materialized
`OwnedValue`, but reaches it via `LazySeq`'s own single-forward-pass fallback (below), not the
four-pass round trip. Elementwise `.[] | select(g)` is a structurally distinct case, out of
this design's required scope (see open risk #3) — it would fold in as
`Instruction::Map(Builtin::Select(g))`, reusing existing per-element logic, but only where
`Expr::Iterate` is the immediately preceding AST node, never inferred from "the previous
stage happened to be lazy."

### Where this plugs in

Two new production sites, one composability arm:

1. **`eval_builtin`, next to the existing `Builtin::Select` arm** (Slice 2) — `Builtin::Map(f)`
   on a plain array/object builds `LazySeq::new(LazySource::Elements(..)|Values(..))`.
2. **The existing `LazyKeys`/`LazyIndexRange` arms in the `Pipe` fold** (Slice 1) — one more
   `unwrap_paren(expr)` case each:
   `Expr::Builtin(Builtin::Map(f)) if !sorted => GenericResult::LazySeq(LazySeq::new(LazySource::Keys(fields)).push_map(...))`
   (same `!sorted` guard the existing `.[]`/`first`/`last` arms already use — sorted `keys`
   still needs a full decode+sort first, per the non-goals above).
3. **One new `GenericResult::LazySeq(seq) => match unwrap_paren(expr) { ... }` arm in the
   `Pipe` fold** — the composability engine: `Builtin::Map(h)` pushes one more instruction
   onto the *same* `LazySeq`; `Builtin::Length`/`Expr::Iterate`/`first`/`.[0]` get
   single-forward-pass native handling (count-and-discard, stream-with-`Partial`-on-error
   mirroring the existing `ManyCursor` arm, pull-one-and-stop); everything else (whole-value
   `select`, `last`, nonzero `.[n]`, comparisons) materializes via one forward pass
   (`materialize_atomic`) and delegates to `eval_on_owned` — still one pass, not the
   four-pass round trip.

### Blast radius: which existing `LazyKeys`/`LazyIndexRange` match sites need new logic

Of the roughly a dozen match sites on `LazyKeys`/`LazyIndexRange` in `eval_generic.rs`: two
need genuinely new logic (`stream_json`/`stream_yaml`, the one place actual laziness must be
preserved end-to-end); one needs the composability arm (the `Pipe` fold, above); the rest
(`into_owned`, `collect_owned`, `push_generic_truthiness`, `flatten_generic_results`,
`Expr::Compare`, `eval_first_or_last_generic`, `eval_index_expr`/`eval_slice_expr`) collapse
into one shared `materialize_lazy(self) -> Self` helper that every "was always going to
materialize anyway" consumer calls once, instead of a third near-identical match arm.

**Output streaming needs no new type family.** A `stream_lazy_seq_json`/`_yaml` generalizes
`stream_lazy_keys_json`/`_yaml`'s existing idiom to `LazySeq`, and `GenericResult::stream_json`/
`stream_yaml` gain one arm each calling it. `src/jq/lazy.rs`'s `JqValue` needs **zero** new
variants — `JqValue::Array` already stores per-element cursors (its own "Phase 1 Lazy
Optimization"), so a `LazySeq` converts into it by pulling forward and wrapping each element,
reusing the existing `write_json` code path entirely.

## Open risks for an implementer to sanity-check

1. `Rc::make_mut` in `push_map` clones `instructions` if a `LazySeq` was previously cloned
   while shared — confirm no code path clones mid-chain before pushing another stage (should
   stay O(1) in the normal single-owner `Pipe`-fold walk).
2. `materialize_atomic`'s atomicity (mid-stream error discards the whole prefix, mirroring
   `eval.rs`'s `map_over`) should be spot-checked against real jq's behavior for
   `keys_unsorted | map(f)` where `f` errors partway — inferred from existing
   array-construction semantics, not verified against a live jq run.
3. `eval_first_or_last_generic`/`eval_index_expr`/`eval_slice_expr` collapsing to
   `materialize_lazy()` means `keys_unsorted | map(f) | .[2]` (nonzero index after a map) is
   *not* lazy in this initial design (only `.[0]`/`first` get a fast path) — confirm that's
   acceptable initial scope; a bounded-forward-walk fast path for small positive indices is a
   cheap, separable follow-up. Elementwise `.[] | select(g)`/`.[] | map(f)` composability is
   similarly out of this design's required scope (see Slice 2's own description) even though
   `Instruction` could accommodate it later.
4. `EvalTag`/`EvalSemantics::tag()` is a new method on the existing `EvalSemantics` trait
   (`src/jq/eval.rs`) — confirm no implementors beyond `JqSemantics`/`YqSemantics` exist that
   would need updating.
5. Streaming error UX (`stream_lazy_seq_json` writing a truncated-but-valid `[...]` then
   reporting the error) should be checked against how `GenericResult::stream_json`'s existing
   `Partial`-adjacent arms currently signal "prefix streamed, then failed," so error UX
   doesn't regress specifically for `map`/`select`.

## Measurement plan for the follow-up implementation PR(s)

Reuse existing infrastructure — `src/bin/succinctly/jq_bench.rs`'s
`QueryType::{KeysUnsortedMap, KeysUnsortedSelect, Map, Select}` were added by #686
specifically to measure this gap and map directly onto the two slices. Apply this repo's
documented A/B discipline (`docs/guides/benchmarking.md`) per slice, not just once at the end:

- Interleaved A/B (never sequential-halves), sizes ≥1MB for CLI process-spawn comparisons,
  reusing #686's 100kb/1mb/10mb/100mb points.
- Report the scaling curve, not one ratio — a real fix should show the ratio *shrinking or
  flattening* with size (the algorithmic term removed), the mirror image of #686's finding
  that it currently *grows* with size.
- Gate on output identity against system `jq`/`yq` at every size × query combination.
- Measure both ARM and x86_64 and name the chip — #686's numbers are Apple-only; a fix claim
  needs at least one non-Apple data point too.
- Slice 1's PR ships its own before/after table restricted to `KeysUnsortedMap`/
  `KeysUnsortedSelect`; Slice 2's PR ships its own table on plain `Map`/`Select` — don't
  combine them into one end-of-project table, or it becomes impossible to attribute which
  slice earned which part of the win.
- Confirm the benchmark actually exercises the shape being fixed: Slice 1 needs the `wide`
  pattern (already exists), Slice 2 needs `Pattern::Arrays`/`Pattern::Users`-shaped inputs
  (already exist) run through `Map`/`Select` — not just `Wide` again, per this repo's #106
  lesson about a benchmark that can't see the shape it's meant to improve.

## Follow-up issues

- Slice 1 (`keys_unsorted`/`keys | map/select` fast path):
  [#724](https://github.com/rust-works/succinctly/issues/724).
- Slice 2 (plain container `map`/`select` — `Builtin::Map`'s first native arm):
  [#725](https://github.com/rust-works/succinctly/issues/725), depends on #724 landing first
  for the shared `LazySeq` machinery.
- Slice 3 (CLI streaming-output integration): deferred, not yet filed.

## Critical files

- `src/jq/eval_generic.rs` — `GenericResult` enum, `LazyKeys`/`LazyIndexRange` variants, the
  `Pipe` fold, `eval_builtin`'s missing `Builtin::Map` arm and wildcard fallback, and the
  three fallback-pinning tests to retire once Slice 1 lands
  (`test_generic_keys_unsorted_fallback_map_select`,
  `test_generic_keys_sorted_fallback_map_select`,
  `test_generic_array_keys_unsorted_fallback_map_select`)
- `src/jq/document.rs` — `DocumentCursor`/`DocumentFields`/`DocumentElements` cons-list
  traits this design builds on
- `src/jq/stream.rs` — `stream_lazy_keys_json`/`_yaml`, the template for
  `stream_lazy_seq_json`/`_yaml`
- `src/jq/eval.rs` — `EvalSemantics` trait (needs `EvalTag`), `map_over`/`builtin_map`, the
  atomicity precedent
- `src/jq/lazy.rs` — `JqValue::LazyKeysArray`/`LazyIndexRange`, confirmed to need zero new
  variants
- `src/bin/succinctly/jq_bench.rs` — existing `QueryType` variants to reuse for before/after
  measurement
- `src/bin/succinctly/yq_runner.rs` — `can_use_m2_streaming` whitelist, Slice 3 (deferred)

## Related

Follow-up from #686 (measurement spike, "go" decision) and #140/#678 (M3.1, `LazyKeys`/
`LazyIndexRange` lazy fast paths). See also #683 (sorted `keys`), #684 (array
`keys`/`keys_unsorted`), #685 (YAML-side lazy `keys_unsorted` output — the closest prior art
for this design's "extend the already-generic layer, don't invent a new type family"
approach, documented in `docs/plan/yq.md`'s "Lazy `keys_unsorted` output (#685)" section).
