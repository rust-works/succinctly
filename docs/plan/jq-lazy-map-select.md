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

| Size  | floor `keys_unsorted\|length` (ms/MB) | `keys_unsorted\|map(.)` (ms/MB) | time ratio | mem ratio  |
|-------|--------------------------------------:|--------------------------------:|-----------:|-----------:|
| 100kb | 5.3ms / 8.7MB                         | 7.3ms / 11.2MB                  | 1.39x      | 1.29x      |
| 1mb   | 10.7ms / 9.9MB                        | 34.6ms / 33.2MB                 | 3.23x      | 3.34x      |
| 10mb  | 73.3ms / 21.8MB                       | 308.9ms / 205.3MB               | 4.21x      | 9.42x      |
| 100mb | 681.3ms / 138.9MB                     | 3259.4ms / 1872.6MB             | 4.78x      | **13.48x** |

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

| Slice                                                            | Covers                                                                                                                                | Why this order                                                                                                                                                                                                                             |
|------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| **1**                                                            | `keys_unsorted\|keys \| map/select` — wire the new primitive into the *existing* `LazyKeys`/`LazyIndexRange` arms of the `Pipe` fold. | Smallest diff, touches only already-well-tested machinery, retires the three fallback-pinning tests below, immediately measurable with the `QueryType::KeysUnsortedMap`/`KeysUnsortedSelect` benchmarks #686 already added.                |
| **2**                                                            | Plain `.foo \| map/select`, `arr \| map/select` — `Builtin::Map`'s first-ever native arm in `eval_builtin`.                           | This is where the dominant 75-95% of the measured cost actually lives, so it is **not optional** — scoped alongside Slice 1, sequenced second because it's structurally larger (first real semantics for a previously-absent builtin arm). |
| **3** *(deferred, own issue, not part of #700's implementation)* | CLI streaming-output integration: `yq_runner.rs`'s `can_use_m2_streaming` whitelist, true zero-`Vec` CLI output.                      | Layered on top of an already-general core, same shape as #685 extending `stream_json`/`stream_yaml` *after* #678 made `keys_unsorted` lazy at the evaluator level.                                                                         |

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
- **YAML merge-key mappings** (`YamlFields`'s private `FieldsInner::Merged`, already eager
  today). Stays eager;
  forcing it lazy conflicts with the forward-only constraint below.
- **Recursive laziness** (`map(map(f))` where the *inner* `map` also stays lazy),
  **`map_values`**, and **any backward/lookahead access on YAML**.

## Format-access asymmetry that shapes the design

JSON tolerates real random access cheaply (interest-bit rank/select, O(log n) worst case, no
penalty for jumping around). YAML does not — `AdvancePositions`/`CompactEndPositions`
(`src/yaml/advance_positions.rs`, `src/yaml/end_positions.rs`) keep one document-wide
*sequential* cursor optimized for monotonically-increasing access; any backward/out-of-order
access falls to `get_random`. As of the O1/O2 optimizations (issue #74), `get_random` does
*not* reset the scan to position zero — it resumes the cursor from the jump point via an O(1)
rank-directory lookup plus a bounded sampled-select scan (`end_positions.rs:419-449`,
`advance_positions.rs` equivalent), specifically to avoid the O(n²) pathology a zero-reset
would cause. But each such jump still pays a real, nonzero per-call cost that a pure
sequential walk avoids entirely, and repeated jump/rewind patterns forfeit the amortized O(1)
benefit `get_sequential` is documented to have. More fundamentally, the cons-list traits this
design builds on (`DocumentFields`/`DocumentElements`, below) have no backward-moving method
at all — `uncons()` only ever consumes forward and returns "the rest" as a new `Self`. Any
design that needed backward access would have to route around those traits entirely, not
merely pay a performance tax.

**The design must be forward-only, single-pass, with no buffering of earlier elements.** This
is a hard constraint, not a nice-to-have — both because it matches the underlying traits'
forward-only contract and because a violation is invisible in JSON-only benchmarks and only
shows up as a YAML-specific regression, the same category of trap this repo's benchmarking
discipline (`docs/guides/benchmarking.md`) already warns about for memory-bound effects across
architectures.

## Building blocks already in the codebase

`src/jq/document.rs`'s `DocumentFields`/`DocumentElements` traits expose cons-list `uncons()`
(pop one, return "the rest" as a new `Self`) — genuinely lazy per-step for both JSON
(`JsonFields`/`JsonElements`, thin `Copy` wrappers over a BP cursor, `src/json/light.rs`) and
YAML (`YamlFields`/`YamlElements`, `src/yaml/light.rs` — `Clone` but not `Copy`, since a field
walk may hold an `Rc`-shared, already-eagerly-resolved merge-key entry list rather than a bare
cursor). These traits are **not dyn-compatible** (`Self: Sized`
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

> **Corrected by Slice 3's implementation (#757).** The `JqValue` half held; the
> `stream_lazy_keys_*` half did not. Those two generalize trivially *because keys are plain
> strings* — they write at a fixed top-level indent and never recurse. A `LazySeq` element is
> an arbitrary value that must render at a **nested** indent, and neither
> `DocumentCursor::stream_json` nor `stream_yaml` carries a current-indent parameter to render
> at. So Slice 3 did need new surface after all, though not a new *type family*:
> `DocumentCursor` gained `stream_sequence_json`/`stream_sequence_yaml` plus a
> `supports_sequence_streaming` probe, defaulting to "unsupported" exactly as `stream_json`
> already did. `YamlCursor`'s YAML impl is a straight delegation to the `stream_yaml_sequence`
> that `--slurp` had used since #478, which is why the change stayed small.

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

## Measurement results (2026-08-11, #758)

Run against two binaries built via `git archive` at `588a4c5d` (before, the commit immediately
preceding PR #740's merge) and `6ce12be6` (after, the #740 merge commit itself). Current `HEAD`
was deliberately **not** used as "after": ~10 further `src/jq` commits landed since #740 (e.g.
`047a0afd` threading `indent`/`sort_keys` through `LazySeq`, `5995e4d4` fixing `paths(f)`), and
using HEAD would have attributed their effects to this PR. `scripts/ab-cli.py` was extended to
support `--tool jq` (previously hardcoded `yq`-only `-o`/`-I0` flags, which made it fail outright
for `jq`). Method: 7 reps, interleaved per `docs/guides/benchmarking.md` rule 1; `--control` run
first on each machine to establish the noise floor; identity gated two ways — `ab-cli.py`'s
before-vs-after digest gate, and `dev bench jq`'s always-on gate against system `jq`.

**Control (noise floor):** Apple M4 Pro −2.7%..+1.0% median, AMD Ryzen 9 7950X −1.7%..+0.9%
median (3 files × 2 queries × 7 reps each). Deltas below are only called real signal when they
clear this band.

Both machines showed transient "busy" warnings from `ab-cli.py`'s rule-6 check that were
investigated and are not real contention: on the Apple M4 Pro, `ps -Ao pcpu` summed to 30-44%
across many small processes with no single offender, while macOS's own aggregate reading
(`top -l 1 -n 0`) showed 91.4% idle — the same class of misleading-aggregate-signal issue
`docs/guides/benchmarking.md` documents for load average, just via a different command; on the
x86_64 node, Tailscale SSH exposes the executed remote command as process argv
(`tailscaled be-child ssh --cmd=...`), so the rule-6 `pgrep -f "cargo|rustc|criterion|claude"`
check matched its own invocation and a leftover `cargo --version` probe from setup, not a real
build. Both runs proceeded with `--force` after confirming no genuine competing workload via
`ps aux`/`top`.

### Slice 1 — `keys_unsorted | map(.)` / `keys_unsorted | select(true)` (`wide` pattern)

| Machine           | Query  | Size      | before (med) | after (med) | Δ median   |
|-------------------|--------|-----------|--------------|-------------|------------|
| Apple M4 Pro      | map(.) | **100mb** | 2983.6ms     | 2596.0ms    | **-13.0%** |
| Apple M4 Pro      | map(.) | 10mb      | 290.5ms      | 255.4ms     | **-12.1%** |
| Apple M4 Pro      | map(.) | 1mb       | 30.6ms       | 28.0ms      | **-8.6%**  |
| Apple M4 Pro      | map(.) | 100kb     | 5.7ms        | 5.5ms       | -3.9%      |
| Apple M4 Pro      | select | 100mb     | 2949.0ms     | 2982.2ms    | +1.1%      |
| Apple M4 Pro      | select | 10mb      | 285.3ms      | 281.6ms     | -1.3%      |
| Apple M4 Pro      | select | 1mb       | 29.5ms       | 29.7ms      | +0.7%      |
| Apple M4 Pro      | select | 100kb     | 5.7ms        | 5.4ms       | -4.4%      |
| AMD Ryzen 9 7950X | map(.) | **100mb** | 3957.7ms     | 2887.0ms    | **-27.1%** |
| AMD Ryzen 9 7950X | map(.) | 10mb      | 405.0ms      | 294.2ms     | **-27.3%** |
| AMD Ryzen 9 7950X | map(.) | 1mb       | 42.0ms       | 32.2ms      | **-23.2%** |
| AMD Ryzen 9 7950X | map(.) | 100kb     | 5.3ms        | 4.5ms       | **-16.5%** |
| AMD Ryzen 9 7950X | select | 100mb     | 3878.1ms     | 4111.6ms    | **+6.0%**  |
| AMD Ryzen 9 7950X | select | 10mb      | 395.9ms      | 419.7ms     | **+6.0%**  |
| AMD Ryzen 9 7950X | select | 1mb       | 41.3ms       | 44.1ms      | **+6.8%**  |
| AMD Ryzen 9 7950X | select | 100kb     | 5.2ms        | 5.6ms       | **+6.7%**  |

`map(.)`'s speedup **grows with size** on both machines (the algorithmic-term signature the plan
called for) — 3.9%→13.0% on ARM, 16.5%→27.3% on x86_64. `select(true)` (no dedicated Slice 1 arm
by design — see [`select`: no new code path](#select-no-new-code-path)) is flat within the
control band on Apple M4 Pro, but a small, **flat-not-scaling** ~6-7% regression on AMD Ryzen 9
7950X across all four sizes — see [Regression surfaced](#regression-surfaced-select-is-slower-on-x86_64) below.

### Slice 2 — `map(.)` / `select(true)` (`arrays`, `users` patterns)

| Machine           | Pattern | Query  | Size      | before (med) | after (med) | Δ median   |
|-------------------|---------|--------|-----------|--------------|-------------|------------|
| Apple M4 Pro      | arrays  | map(.) | **100mb** | 11133.7ms    | 6573.7ms    | **-41.0%** |
| Apple M4 Pro      | arrays  | map(.) | 10mb      | 951.9ms      | 575.3ms     | **-39.6%** |
| Apple M4 Pro      | arrays  | map(.) | 1mb       | 93.2ms       | 55.8ms      | **-40.2%** |
| Apple M4 Pro      | arrays  | map(.) | 100kb     | 11.2ms       | 8.1ms       | **-28.0%** |
| Apple M4 Pro      | arrays  | select | 100mb     | 4891.1ms     | 4932.6ms    | +0.8%      |
| Apple M4 Pro      | arrays  | select | 10mb      | 403.5ms      | 404.3ms     | +0.2%      |
| Apple M4 Pro      | arrays  | select | 1mb       | 40.3ms       | 39.6ms      | -1.8%      |
| Apple M4 Pro      | arrays  | select | 100kb     | 6.4ms        | 6.5ms       | +1.3%      |
| Apple M4 Pro      | users   | map(.) | **100mb** | 3218.3ms     | 1620.4ms    | **-49.7%** |
| Apple M4 Pro      | users   | map(.) | 10mb      | 311.2ms      | 158.0ms     | **-49.2%** |
| Apple M4 Pro      | users   | map(.) | 1mb       | 33.9ms       | 17.9ms      | **-47.3%** |
| Apple M4 Pro      | users   | map(.) | 100kb     | 5.9ms        | 4.4ms       | **-26.6%** |
| Apple M4 Pro      | users   | select | 100mb     | 993.1ms      | 1000.6ms    | +0.8%      |
| Apple M4 Pro      | users   | select | 10mb      | 97.7ms       | 95.7ms      | -2.0%      |
| Apple M4 Pro      | users   | select | 1mb       | 11.4ms       | 11.3ms      | -0.4%      |
| Apple M4 Pro      | users   | select | 100kb     | 3.5ms        | 3.6ms       | +0.8%      |
| AMD Ryzen 9 7950X | arrays  | map(.) | **100mb** | 15280.7ms    | 8295.7ms    | **-45.7%** |
| AMD Ryzen 9 7950X | arrays  | map(.) | 10mb      | 1373.4ms     | 768.7ms     | **-44.0%** |
| AMD Ryzen 9 7950X | arrays  | map(.) | 1mb       | 133.4ms      | 76.1ms      | **-43.0%** |
| AMD Ryzen 9 7950X | arrays  | map(.) | 100kb     | 13.1ms       | 8.1ms       | **-38.2%** |
| AMD Ryzen 9 7950X | arrays  | select | 100mb     | 4346.0ms     | 4455.3ms    | **+2.5%**  |
| AMD Ryzen 9 7950X | arrays  | select | 10mb      | 391.4ms      | 401.1ms     | +2.5%      |
| AMD Ryzen 9 7950X | arrays  | select | 1mb       | 38.4ms       | 38.8ms      | +1.0%      |
| AMD Ryzen 9 7950X | arrays  | select | 100kb     | 4.9ms        | 5.0ms       | +1.2%      |
| AMD Ryzen 9 7950X | users   | map(.) | **100mb** | 4693.0ms     | 2283.5ms    | **-51.3%** |
| AMD Ryzen 9 7950X | users   | map(.) | 10mb      | 467.5ms      | 249.9ms     | **-46.5%** |
| AMD Ryzen 9 7950X | users   | map(.) | 1mb       | 42.0ms       | 23.5ms      | **-44.1%** |
| AMD Ryzen 9 7950X | users   | map(.) | 100kb     | 5.1ms        | 3.4ms       | **-34.3%** |
| AMD Ryzen 9 7950X | users   | select | 100mb     | 1025.4ms     | 1045.7ms    | **+2.0%**  |
| AMD Ryzen 9 7950X | users   | select | 10mb      | 104.3ms      | 105.0ms     | +0.7%      |
| AMD Ryzen 9 7950X | users   | select | 1mb       | 11.2ms       | 11.5ms      | **+2.1%**  |
| AMD Ryzen 9 7950X | users   | select | 100kb     | 2.1ms        | 2.2ms       | +1.6%      |

Slice 2's `map(.)` win is larger than Slice 1's (up to -51.3% vs -27.3%) and shows the same
growing-with-size shape, consistent with #686's finding that plain containers carry the dominant
75-95% share of the original eager-fallback cost. `select(true)` again shows the same
platform split: flat on Apple M4 Pro (within the -2.7%..+1.0% control band), a small but
consistently positive ~1-2.5% drift on AMD Ryzen 9 7950X — smaller than Slice 1's ~6-7% but the
same direction, discussed below.

### Regression surfaced: `select` is slower on x86_64

Both slices show `select(true)` getting **consistently slower on AMD Ryzen 9 7950X** after #740
(+6.0-6.8% for `keys_unsorted | select(true)`, +0.7-2.5% for plain `select(true)`) while Apple M4
Pro shows no such effect (all deltas within the control band) — and the effect is **flat across
sizes**, not growing, unlike every `map(.)` result above. That flat shape matches a small
per-call constant-factor cost (plausibly the new `LazySeq` plumbing `select` now flows through
before falling back to its single-pass materialize, per [`select`: no new code
path](#select-no-new-code-path)) rather than an algorithmic regression. Confirmed reproducible:
identical magnitude measured independently against both `HEAD` and the isolated `6ce12be6`
binary. Per this issue's non-goals, no fix is attempted here — filed as
[#789](https://github.com/rust-works/succinctly/issues/789) to investigate.

### Output identity

- `ab-cli.py` before-vs-after digest gate: 48 configurations (Slice 1: 8+8, Slice 2: 16+16
  across both machines), **0 differences**.
- `dev bench jq` vs. system `jq`: 48 configurations, **0 differences** (system `jq` 1.7.1 on the
  Apple M4 Pro node, 1.6 on the AMD Ryzen 9 7950X node — each machine's installed version).

### Memory (supplementary — vs. system `jq`, single run per binary, not interleaved)

Collected via `dev bench jq`'s built-in RSS measurement (`/usr/bin/time`) for context, since
`ab-cli.py` only measures wall time. Not interleaved between the two `succinctly` binaries, so
treated as directional rather than a load-bearing timing claim (memory is far less sensitive to
thermal drift than wall-clock, per the plan's own reasoning for this step). Full per-pattern
tables are in `slice1-{arm,x86}.md` / `slice2-{arm,x86}.md` alongside the raw run data; summary:

- `map(.)`'s memory ratio (succinctly / jq) **improved substantially** from #686's original
  spike numbers (up to 13.48x at 100mb) to roughly parity-to-2x across both slices and machines
  (e.g. Slice 1 `wide`/100mb: 0.50x on ARM, 1.04x on x86_64; Slice 2 `users`/100mb: 1.97x on ARM,
  1.97x on x86_64) — consistent with replacing the old materialize-reserialize-reindex fallback.
- `select(true)` on plain containers (Slice 2, native single-value arm) uses **far less** memory
  than jq at 10-100mb (0.04x-0.29x on both machines, near parity at 1mb) since it never
  materializes the whole container. `keys_unsorted | select(true)` (Slice 1, still the eager
  fallback) uses **more** memory than jq (1.25x-2.34x), unchanged in kind from #686's original
  finding for that path.

### Reproducing

```bash
# two binaries, isolating PR #740's diff specifically (not current HEAD):
git archive --format=tar.gz -o succ-before-src.tar.gz 588a4c5d  # parent of #740's merge
git archive --format=tar.gz -o succ-after-src.tar.gz 6ce12be6   # the #740 merge commit itself
# build each with: cargo build --release --features cli

python3 scripts/ab-cli.py --before ./succ-before --after ./succ-after --tool jq \
  --files wide/100kb.json wide/1mb.json wide/10mb.json wide/100mb.json \
  --queries "keys_unsorted | map(.)" "keys_unsorted | select(true)" --reps 7

./succ-after dev bench jq --data-dir corpus --patterns wide --sizes 100kb,1mb,10mb,100mb \
  --queries keys_unsorted_map,keys_unsorted_select --binary ./succ-after \
  --output slice1.jsonl --markdown slice1.md
```

## Follow-up issues

- Slice 1 (`keys_unsorted`/`keys | map/select` fast path):
  [#724](https://github.com/rust-works/succinctly/issues/724).
- Slice 2 (plain container `map`/`select` — `Builtin::Map`'s first native arm):
  [#725](https://github.com/rust-works/succinctly/issues/725), depends on #724 landing first
  for the shared `LazySeq` machinery.
- Slice 3 (CLI streaming-output integration):
  [#757](https://github.com/rust-works/succinctly/issues/757) — **landed**. Two things the
  scoping did not anticipate:
  - The issue's own "Tier 2" reading (whitelist `LazySeq`, add a `stream_lazy_seq_json` that
    calls `materialize_atomic` then reuses `OwnedValue::stream_*`) turned out to be a no-op:
    `GenericResult::stream_json`/`stream_yaml`'s `LazySeq` arms already did precisely that.
  - The tension with `map`'s atomicity that pushed this out of #724/#725 dissolves once the
    drain is separated from the conversion. `LazySeq::drain_atomic` settles the whole chain
    up front while keeping cursors (`Copy`, pointer-sized), so a failing element can never
    leave a truncated prefix — no element-of-lookahead buffering needed, and the deep
    `OwnedValue` copy is what goes away.
  - The payoff was mostly *correctness*, not throughput: routing `map` through the DOM path
    had been collapsing duplicate mapping keys and dropping comments, anchors, flow style
    and quoted-scalar style, and under `-I0` emitted nested containers at their parent's
    indent (silent data loss). All were verified against the pinned yq v4.53.3 and are now
    covered by `tests/data/yq-golden/cases/map_*`.

- Slice 3, jq side (CLI streaming-output integration for `succinctly jq` itself):
  [#1576](https://github.com/rust-works/succinctly/issues/1576) — **landed**. Deliberately
  deferred out of #757 (yq had no correctness gap to force the jq side; JSON has no
  comments/anchors/style for the DOM path to drop, so the payoff here is throughput only).
  `JsonCursor` gained a pretty/sort-capable recursive writer and `stream_sequence_json`
  support (mirroring what #757 gave `YamlCursor`), plus a new `JsonConvention` enum
  (`src/jq/document.rs`) so the shared streaming machinery can select jq's own number/escape/
  duplicate-key conventions instead of yq's, which it previously hardcoded.
  - **Atomicity is stricter for jq than for yq.** #2066 (postdates this issue's own text) had
    already fixed a jq-specific regression where `map(.)` on `[1, {"bad": xyz123}]` printed a
    garbled `[1,{"bad":` prefix before erroring — real jq's array construction is all-or-
    nothing. `JsonCursor::stream_json` now buffers each top-level value locally before writing
    it to the real output, restoring that guarantee for the new cursor-streaming path; yq's own
    identical cursor path is unaffected and keeps its already-accepted partial-prefix-on-error
    trade (#1641/#1679).
  - **Malformed input is handled by falling back, not by full parity.** Real jq's existing
    `print_json`/`to_owned_cursor`/`DisplayKeyGuard` stack has years of issue-specific,
    sometimes deliberately inconsistent malformed-input behavior. Rather than re-deriving all
    of it in the new writer, `jq_runner.rs`'s fast path detects malformation and falls back to
    the already-correct general path, restricted by a new `m2_json_fallback_safe` predicate to
    AST shapes where that fallback is provably safe (excludes `.[]`/`Iterate`, `select`,
    computed `IndexExpr`, and `keys_unsorted`, which either permit multiple top-level results
    or go through a separate, unverified writer).
  - **Benchmarked** (interleaved A/B, `scripts/ab-cli.py`, 2/10/20 MB corpora, output-identity
    gated against both `succ-base`/`succ-head` and the pinned `/usr/bin/jq` 1.7.1, 2026-09-02):
    `map(.)`/`sort_by`/`reverse` (the queries that render every element) are **1.4-2.0x faster**
    on both Apple M4 Pro and AMD Ryzen 9 7950X, flat across the size ladder (a constant-factor
    win from removing the per-element `OwnedValue` allocation, not an algorithmic one).
    `unique_by`/`min_by`/`max_by` reach the same fast path but show no measurable win, since
    their cost is dominated by scanning/comparing every element, not by rendering the result.
    `keys_unsorted | map(.)` is neutral (excluded from the fast path, see above). **A real,
    reproducible regression surfaced on large-array plain-field access** (`.data`, a
    2238-element array): +2.3-3.0% on M4 Pro, +7.3-11.4% on the 7950X, both clearing the A/B
    noise floor (control range roughly ±2-3%) — the new per-value buffering (needed for the
    atomicity fix above) costs a memory copy on a large single-result value that the old path
    didn't pay, with no rendering-throughput win to offset it since there's only one such
    value, not many. The buffering itself isn't optional -- real jq parses the whole document
    before printing anything, so even a single-result query like `.data` needs the same
    zero-partial-output guarantee if a later structural error exists deeper in that value, the
    same reasoning as the `LazySeq` case, just for one value instead of many. Not fixed in this
    change; a cheaper implementation of that same guarantee (e.g. a size-based fast path, or
    validating before writing instead of buffering while writing) is a reasonable follow-up.

## Critical files

- `src/jq/eval_generic.rs` — `GenericResult` enum, `LazyKeys`/`LazyIndexRange` variants, the
  `Pipe` fold, `eval_builtin`'s missing `Builtin::Map` arm and wildcard fallback, and the
  three fallback-pinning tests to retire once Slice 1 lands
  (`test_generic_keys_unsorted_fallback_map_select`,
  `test_generic_keys_sorted_fallback_map_select`,
  `test_generic_array_keys_unsorted_fallback_map_select`)
- `src/jq/document.rs` — `DocumentCursor`/`DocumentFields`/`DocumentElements` cons-list
  traits this design builds on
- `src/jq/stream.rs` — `stream_lazy_keys_json`/`_yaml`, the intended template for
  `stream_lazy_seq_json`/`_yaml` (see the correction above: Slice 3 used
  `DocumentCursor::stream_sequence_*` and `src/yaml/light.rs`'s
  `stream_yaml_sequence`/`stream_json_sequence` instead, because these two cannot render at a
  nested indent)
- `src/jq/eval.rs` — `EvalSemantics` trait (needs `EvalTag`), `map_over`/`builtin_map`, the
  atomicity precedent
- `src/jq/lazy.rs` — `JqValue::LazyKeysArray`/`LazyIndexRange`, confirmed to need zero new
  variants
- `src/bin/succinctly/jq_bench.rs` — existing `QueryType` variants to reuse for before/after
  measurement
- `src/bin/succinctly/yq_runner.rs` — `can_use_m2_streaming` whitelist, Slice 3 (landed:
  `Builtin::Map(f) => can_use_m2_streaming(f)`, recursing into the body for the same reason
  `FirstExpr`/`LastExpr` do)

## Related

Follow-up from #686 (measurement spike, "go" decision) and #140/#678 (M3.1, `LazyKeys`/
`LazyIndexRange` lazy fast paths). See also #683 (sorted `keys`), #684 (array
`keys`/`keys_unsorted`), #685 (YAML-side lazy `keys_unsorted` output — the closest prior art
for this design's "extend the already-generic layer, don't invent a new type family"
approach, documented in `docs/plan/yq.md`'s "Lazy `keys_unsorted` output (#685)" section).
