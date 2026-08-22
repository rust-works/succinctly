# Collapsing duplicate object keys in jq mode (#1385)

[Home](../../) > [Docs](../) > [Plan](README.md) > jq duplicate-key collapse

**Status: design, with implementation following on the same branch.** This document is the
design deliverable for [#1385](https://github.com/rust-works/succinctly/issues/1385). Unlike
`jq-path-trackability-deferral.md`, it is not design-only: the scoping below reduced the
change to a size worth landing in one PR, and the single genuinely open question is a
measurement that requires the code to answer.

**Supersedes two premises from the issue thread.** #1385's second comment concluded "the right
next artifact is a design pass, not a patch", resting on two claims that measurement and
reading disprove — `length` is not O(1), and the four existing tests do not conflict. Both are
corrected below. The thread's *incoherence* finding stands and is the reason to fix rather than
document.

## Problem

In jq mode `succinctly jq` emits duplicate object keys verbatim. Real jq collapses them
**last value wins, first position kept** — exactly `IndexMap::insert` semantics. Captured live
from the pin (`/usr/bin/jq`, `jq-1.7.1-apple`) on `{"b":1,"a":2,"b":3}`:

| filter          | jq 1.7.1        | `sjq` today           |         |
|-----------------|-----------------|-----------------------|---------|
| `.`             | `{"b":3,"a":2}` | `{"b":1,"a":2,"b":3}` | ✗ DIFF  |
| `length`        | `2`             | `3`                   | ✗ DIFF  |
| `keys`          | `["a","b"]`     | `["a","b","b"]`       | ✗ DIFF  |
| `keys_unsorted` | `["b","a"]`     | `["b","a","b"]`       | ✗ DIFF  |
| `[.[]]`         | `[3,2]`         | `[1,2,3]`             | ✗ DIFF  |
| `to_entries`    | 2 entries       | 2 entries             | ✓ agree |
| `.b`            | `3`             | `3`                   | ✓ agree |
| `has("b")`      | `true`          | `true`                | ✓ agree |
| `[paths]`       | `[["b"],["a"]]` | `[["b"],["a"]]`       | ✓ agree |
| `map_values(.)` | `{"b":3,"a":2}` | `{"b":3,"a":2}`       | ✓ agree |

The split is exact: the five agreeing consumers **materialize** through `to_owned`, whose object
arm builds an `IndexMap` and therefore already implements jq's rule by accident. The five
divergent ones are the **cursor-native paths that never materialize**.

The result is not merely a divergence from jq but an internal contradiction, demonstrable
without reference to jq at all: `to_entries | length` is `2` while `keys | length` is `3` on the
same document, and `map_values(.)` — a no-op transform — changes the key count from 3 to 2. A
document cannot both have two keys and three. This is why "record it as a semi-indexing
divergence" is not available: it would mean documenting a contradiction. ADR-0018 rule 4 closes
the door from the other side — "our architecture makes it awkward" is explicitly not one of the
three admissible divergence justifications.

This producer is the root of seven symptom issues patched one consumer at a time: #443, #442,
#478, #1251, #1170 on the jq side; #1342/#1343/#1344 on the yq side.

## Corrections to the issue thread

### 1. `length` is already O(n), not O(1)

`JsonFields` does not override `DocumentFields::len()`, so object `length` uses the trait
default — a plain `uncons` cons-list walk over BP sibling hops. Measured (Apple M-series,
release build, best-of-5, `succinctly jq <filter> obj<N>.json`, minus a `true` filter baseline
that parses the same file without walking fields):

| fields | `true` (parse only) | `length` | marginal |
|--------|---------------------|----------|----------|
| 1K     | 4.1 ms              | 3.8 ms   | ~0       |
| 10K    | 4.3 ms              | 4.7 ms   | 0.4 ms   |
| 100K   | 6.5 ms              | 11.5 ms  | 5.0 ms   |
| 1M     | 44.6 ms             | 84.2 ms  | 39.6 ms  |

Linear, ~54 ns/field. The thread conflated this with *array* `length`, which genuinely is O(1)
via `GenericResult::LazyIndexRange`. The "O(1) → O(n)-with-allocation regression" objection is
void.

### 2. The four "conflicting" tests are `--preserve-input` tests

`test_preserve_input_pretty_preserves_duplicate_keys`,
`test_preserve_input_first_last_expr_preserve_duplicate_keys`,
`test_preserve_input_bare_first_last_preserve_duplicate_keys` and
`test_preserve_input_computed_index_preserves_duplicate_keys` (`tests/jq_cli_tests.rs`) every one
passes `--preserve-input`. That flag is a succinctly extension whose stated purpose is preserving
the input's original formatting; it sets `jq_compat = false`, which is also the *sole* remaining
gate on `OutputConfig::can_use_raw_identity` — the raw-bytes echo path that reproduces the input
verbatim without parsing.

So `--preserve-input` is the natural exemption under ADR-0018 rule 5 (a labelled extension that
perturbs no reference-defined filter), and **all four tests stay green unchanged**. There is no
conflict to resolve, and the raw-bytes identity path needs no work: in default jq mode
(`jq_compat = true`) it is never taken.

### 3. The real constraint the thread did not identify

The dedup helper that already exists, `DocumentFields::effective_fields()`, allocates a `String`
per key and an `IndexMap` of 4-cursor `DocumentField`s. Measured on the 1M-field object via
`to_entries|length`, its only hot caller:

| filter                 | total   | over parse-only |
|------------------------|---------|-----------------|
| `true`                 | 30.5 ms | ~1 ns/field     |
| `length`               | 83.9 ms | 54 ns/field     |
| `keys_unsorted|length` | 83.4 ms | 53 ns/field     |
| `.`                    | 184.1 ms| 154 ns/field    |
| `to_entries|length`    | 925.6 ms| **896 ns/field**|

`effective_fields` costs **16x the bare walk**. Routing the streaming paths through it — the
obvious reading of "just reuse the existing helper" — would be a catastrophic regression. The
fix needs a detector that allocates nothing when no key repeats.

(Note `keys|length` measures the same as `length`: the `LazyKeys` + `Length` fast path answers
from `fields.len()` without ever decoding a key. That fast path must survive the fix.)

## Scope decision

**In scope:** jq mode's five divergent paths, and the `keys_dedup()` format→mode correction that
ADR-0018 rule 2 explicitly assigns to this issue.

The gate move is verified safe. `keys_dedup()` has exactly two callers, `Builtin::ToEntries` and
`collect_paths_generic`. Switching them from format-gating to mode-gating changes yq's behaviour
only for JSON input, and only toward the reference — captured live from Homebrew `yq` v4.53.3:

| filter              | real yq (YAML) | real yq (JSON) | `syq` YAML | `syq` JSON |
|---------------------|----------------|----------------|------------|------------|
| `length`            | `3`            | `3`            | `3`        | **`2`**    |
| `to_entries|length` | `3`            | `3`            | `3`        | **`2`**    |
| `[paths]`           | (no builtin)   | (no builtin)   | preserves  | **collapses** |

Real yq preserves regardless of format; `syq` splits on format. Mode-gating fixes the JSON
column, closing **#1398 divergence 1**.

### Non-goals

- **yq's `.[]` collapse (#1398 divergence 2).** Real yq collapses under iteration and preserves
  everywhere else; `syq` preserves under iteration on both formats. I confirmed `[.[]]` collapses
  through a *different* mechanism than `keys_dedup()` (it is format-split at the array-collection
  step, not at field enumeration), so the gate move neither fixes nor regresses it. It stays with
  #1398.
- **yq's `map_values(.)`**, which collapses in `syq` on both formats where real yq preserves.
  Another #1398-family divergence, out of scope here.
- **`--stream`.** Real jq's `--stream` emits both occurrences even though its value model
  collapses; no change proposed.
- **`--preserve-input`.** Continues to preserve, by design.

## The mechanism

### The gate

A thirteenth const on `EvalSemantics` (`src/jq/eval.rs`), following the established convention of
a doc comment recording the live-verified reference behaviour behind the flag:

```rust
const COLLAPSE_DUPLICATE_KEYS: bool;   // JqSemantics = true, YqSemantics = false
```

`EvalSemantics` is implemented by zero-sized marker types, so **yq monomorphizes the whole check
away**. The yq benchmark story — the 8-10x wins that are this crate's headline — is protected by
construction rather than by measurement.

`DocumentFields::keys_dedup()` and its two impls are then deleted, and `effective_fields()` moves
from a trait method to a free function taking an explicit `collapse: bool`. Passing a bool rather
than an `S` type parameter keeps `document.rs` free of an `eval.rs` import. `collect_paths_generic`
gains an `S` parameter to reach the const.

### The detector

Alongside `effective_fields` in `src/jq/document.rs`:

```rust
/// `None` when no key repeats — the caller then walks `fields` exactly as before,
/// allocating nothing. `Some(v)` only when a duplicate is actually present, with
/// the fields collapsed first-position/last-value.
pub fn collapsed_fields<F: DocumentFields>(fields: &F)
    -> Option<Vec<DocumentField<F::Value, F::Cursor>>>
```

Detection reads borrowed keys via `DocumentField::key_str()`. This is zero-copy in the common
case: it resolves to `JsonStr::as_str`, which returns `Cow::Borrowed` whenever the raw span
contains no backslash, allocating only to decode escapes. Two regimes:

- **n ≤ SMALL** (a threshold to be tuned; nearly every real object): pairwise comparison, length
  first and bytes only on a length match. No allocation, no hashing.
- **larger**: a `HashSet<u64>` of hashed spans, verifying only on collision.

The dirty path reuses `effective_fields`' existing `IndexMap` logic verbatim.

**One specialization.** `print_json` is JSON-concrete and currently avoids decoding keys entirely
— it writes `JsonStr::raw_bytes()` straight through when the span has no backslash. Making it call
`key_str()` would add a UTF-8 validation per key that it does not pay today. So `JsonFields` gets
an inherent `has_duplicate_keys()` that compares raw spans directly, and `print_json` builds a
collapsed order only when that returns true.

### Where it plugs in

The surface is smaller than the thread implies. `.[0]`, `first(.[])`, `last(.[])` and `.[]` over
an *array* of duplicate-key objects all diverge for one reason only: the value reaching the
printer is still a cursor, and the printer preserves. **One output-layer fix covers all of them**,
plus every nested object anywhere in any output.

**Output layer** — jq CLI only, gated on `config.jq_compat` so `--preserve-input` is exempt:

- `print_json`'s `StandardJson::Object` arm, compact and pretty loops (`jq_runner.rs`). *The
  perf-sensitive one — it recurses over every object in the output.*
- The `JqValue::LazyKeysArray` print arm, and its siblings in `src/jq/lazy.rs` (`write_json`,
  `lazy_keys_array_to_owned`, and the two `length` sites). `lazy.rs`'s `JqValue` is jq-only;
  `yq_runner.rs` does not use it.

**Evaluator layer** — gated on `S::COLLAPSE_DUPLICATE_KEYS`:

- `Expr::Iterate`'s object arm in `eval_generic.rs`, and its twin in `eval.rs`.
- `Builtin::Length`'s object arm in `eval_generic.rs`, and `builtin_length` in `eval.rs`.
- `Builtin::Keys` / `Builtin::KeysUnsorted`, and the `GenericResult::LazyKeys` dispatch arms
  (`Length`, `Iterate`, `Index`, `First`, `Last`, `Map`, and the materializing fallback).
- The `LazyKeys` consumers in `jq_runner.rs`'s `evaluate_input`.

Both evaluators must move together. `eval.rs` and `eval_generic.rs` are separate implementations,
delegation runs one way only (generic → owned, via a serialize-and-reindex bridge), and yq's JSON
path reaches `eval.rs`'s native arms directly.

## Staged delivery

| Stage | Content                                                                       |
|-------|-------------------------------------------------------------------------------|
| 1     | This document, plus its row in `docs/plan/README.md`                          |
| 2     | `EvalSemantics` const, `keys_dedup()` removal, `collapsed_fields` detector    |
| 3     | Evaluator layer: `Iterate`, `Length`, `Keys`/`KeysUnsorted`, `LazyKeys` arms  |
| 4     | Output layer: `print_json`, `lazy.rs` / `LazyKeysArray` writers               |
| 5     | A/B measurement, golden cases, compliance-page updates                        |

## Measurement plan

Per `docs/guides/benchmarking.md` § A/B Benchmarking Method:

- **Interleave the two binaries within each repetition.** Running all of A then all of B made an
  improved binary measure up to 2x slower in #106; sequential halves are not fixable with more
  reps.
- **Inputs ≥ 1 MB** for process-spawn A/B — startup is ~4-6 ms, which is the entire runtime at the
  smaller sizes.
- **Report the curve over 2-3 sizes**, not one ratio.
- **Gate on output identity before believing any timing**, and confirm output still matches jq.
- `succinctly bench run jq_bench` on ARM (M4 Pro) and x86_64 (7950X) — memory-bound effects do not
  port across architectures. Run sequentially; benchmarks need exclusive CPU.
- Spot-check `yq_bench` to confirm the monomorphization argument empirically, not just by
  reasoning.

Target ≤2% on `jq_bench`. If the small-n path misses it, tune the threshold — not the correctness.
ADR-0018 rule 4 does not admit a performance exception.

## Open risks for an implementer to sanity-check

1. **`print_json` recursion cost** is the one genuinely open number. The detector runs per object,
   over every object in the output. Stage 5's measurement is the decision point for `SMALL`.
2. **Shared-evaluator hazard.** ADR-0018 rule 2's standing warning, and a repeated failure in this
   repo: a builtin generic over `S` serves *both* modes, so a jq-motivated rewrite can silently
   regress yq. Every touched arm needs a yq-mode check.
3. **The `LazyKeys` + `Length` fast path must survive.** It currently answers from `fields.len()`
   without decoding keys; the deduped answer needs the key text. This arm gets slower by
   construction — quantify it rather than assuming it is lost in the noise.
4. **Non-string keys.** `key_str()` returns `None` for a YAML alias or complex key.
   `effective_fields` currently *drops* such fields on the deduping path (they never reach the
   `IndexMap`). yq is gated off so this is not newly reachable, but the detector must not
   introduce the same drop for JSON, where every key is a string by grammar.

## Critical files

| File                              | Role                                                          |
|-----------------------------------|---------------------------------------------------------------|
| `src/jq/eval.rs`                  | `EvalSemantics` + the two impls; `Expr::Iterate`, `builtin_length` |
| `src/jq/document.rs`              | `DocumentFields`, `keys_dedup`, `effective_fields`, new detector |
| `src/jq/eval_generic.rs`          | `Iterate`, `Length`, `Keys`/`KeysUnsorted`, `LazyKeys`, `collect_paths_generic` |
| `src/json/light.rs`               | `JsonFields` impl, `JsonStr::as_str`, new `has_duplicate_keys` |
| `src/yaml/light.rs`               | `YamlFields`' `keys_dedup` impl (deleted)                      |
| `src/bin/succinctly/jq_runner.rs` | `print_json` object arm, `LazyKeysArray`, `evaluate_input`     |
| `src/jq/lazy.rs`                  | `JqValue` length sites, `write_json`, `lazy_keys_array_to_owned` |

## Related

- [#1385](https://github.com/rust-works/succinctly/issues/1385) — this issue
- [#1398](https://github.com/rust-works/succinctly/issues/1398) — the yq-mode half; divergence 1
  closes here, divergence 2 does not
- [ADR-0018](../adrs/adr-0018.md) — reference-tool fidelity decided by mode; rule 2 assigns the
  `keys_dedup()` correction here, rule 4 forbids a performance-based divergence, rule 5 covers the
  `--preserve-input` exemption
- Symptom issues: #443, #442, #478, #1251, #1170 (jq), #1342/#1343/#1344 (yq)
- [#1377](https://github.com/rust-works/succinctly/issues/1377) — #820's design doc, where this
  was first surfaced as Open Risk 7
