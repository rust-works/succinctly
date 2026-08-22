# Collapsing duplicate object keys in jq mode (#1385)

[Home](../../) > [Docs](../) > [Plan](README.md) > jq duplicate-key collapse

**Status: implemented.** This document is the design deliverable for
[#1385](https://github.com/rust-works/succinctly/issues/1385). Unlike
`jq-path-trackability-deferral.md`, it is not design-only: the scoping below reduced the
change to a size worth landing in one PR, and the single genuinely open question was a
measurement that required the code to answer. "What the measurement actually cost" at the
bottom records the answer, including two places this design's own predictions were wrong.

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

The gate move is safe: `keys_dedup()` has exactly two callers, `Builtin::ToEntries` and
`collect_paths_generic`, and yq's observable behaviour is byte-identical before and after on
both input formats.

**It does not, however, close #1398 divergence 1, as this document first claimed.** Real yq
preserves a repeated key regardless of format while `syq` splits on it — captured live from
Homebrew `yq` v4.53.3:

| filter              | real yq (YAML) | real yq (JSON) | `syq` YAML | `syq` JSON |
|---------------------|----------------|----------------|------------|------------|
| `length`            | `3`            | `3`            | `3`        | **`2`**    |
| `to_entries|length` | `3`            | `3`            | `3`        | **`2`**    |
| `[paths]`           | (no builtin)   | (no builtin)   | preserves  | **collapses** |

Removing `keys_dedup()` left that JSON column exactly as it was. The cause is upstream of the
evaluator: `parse_input`'s `InputFormat::Json` arm (`yq_runner.rs`) materializes through
`to_owned_canonicalizing_numbers` — an `IndexMap` — before any filter runs, so the collapse has
already happened by the time a duplicate-key rule could apply. ADR-0018 named `keys_dedup()` as
the mechanism behind that symptom; it was not. The rule-2 correction still stands on its own
terms (the gate genuinely was on the wrong axis), but #1398 divergence 1 needs `parse_input`
fixed, not this.

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

## What the measurement actually cost

Interleaved A/B against the merge-base, Apple M-series laptop, release builds,
`succinctly json generate` inputs at 1 MB and 10 MB. Output identity was gated first: A and B
produce byte-identical bytes for `.`, `.[]`, `length` and `keys` at both sizes, compact and
pretty.

**Caveat on all numbers below: this was not measured on an idle machine, and not on the
project's benchmark hosts.** The laptop carried someone else's interactive load throughout
(a browser renderer at 30-65% of a core, load average drifting between 7 and 14), and the
same baseline measured 142 ms at the low end and 193 ms at the high end for `10mb .`. Per
`docs/guides/benchmarking.md` a real result wants the pinned hosts — ARM (M4 Pro) *and*
x86_64 (7950X), memory-bound effects not porting between them — and neither was used.
**A confirming run on both remains outstanding.**

Four successive versions of the probe, worst case across five measured points, each measured
against the same baseline binary in the same session:

| version                                                        | worst regression |
|----------------------------------------------------------------|------------------|
| separate probe walk, `raw_bytes()` + `contains(&b'\\')`         | 10.4%            |
| one scan per key (`raw_and_escaped`)                            |  8.8%            |
| single field walk, collect spans, per-object `Vec`              |  6.5%            |
| shared scratch stack + span fingerprints                        |  **~4%**         |

The final figure is a range, not a point. Min-of-30 and median-of-30 at load ~7 gave 3.9% and
4.7% worst-case; a later paired-ratio run at load ~11 (A and B alternating within each
repetition, so drift cancels, median of the per-repetition ratios) gave:

| size  | query    | median delta | p25   | p75   |
|-------|----------|--------------|-------|-------|
| 1 MB  | `.`      | +3.4%        | -0.4% | +7.4% |
| 1 MB  | `.[]`    | +5.0%        | -0.0% | +10.4%|
| 10 MB | `.`      | +6.9%        | +2.8% | +9.6% |
| 10 MB | `.[]`    | +3.3%        | +1.9% | +5.7% |
| 10 MB | `length` | +0.6%        | -1.4% | +2.2% |

Call it **4-6% on jq-mode streaming output over object-heavy JSON, and free on `length`**.

**This misses the ≤2% target.** It is shipped anyway: ADR-0018 rule 4 admits three grounds for
diverging from the reference and performance is not among them, so the choice was never
"collapse or stay fast" but "how cheaply can the collapse be made". `length` is free because
the evaluator already walked the field list; the residue is entirely the printer's, which is
the one path that previously touched no field twice.

Two predictions in this design were wrong, both about where the cost sits:

- **The key text scan was not the bottleneck.** Fusing the closing-quote scan with the escape
  check — the fix this document proposed — recovered only 1.6 points. The dominant cost is the
  *field walk*: `uncons` is two BP sibling hops, and probing meant doing the whole walk twice.
  Collecting the fields on the single walk instead was worth 2.3 points more than the scan fix.
- **The `n <= SMALL` pairwise branch is not a tuning knob.** This document called the threshold
  "a correctness-of-scale question, not a tuning one" only after shipping an unbounded pairwise
  scan and measuring **1240%** on a 10 MB document with a wide root object. Left in, it would
  have been a quadratic blowup on exactly the input the crate exists to handle well.

The remaining lever, unexplored: `PreparedField` stores two whole `JsonCursor`s per field, and
a cursor's `text` and `index` members are invariant across the entire document — roughly 48 of
its 88 bytes are redundant. Hoisting them would cut the buffer's memory traffic by half.

## What code review found afterwards

Five defects survived the staged delivery above. All are fixed; each is recorded because the
design predicted the *shape* of three of them and still shipped them.

**1. The collapse itself was quadratic.** `PAIRWISE_SPAN_SCAN_LIMIT` bounded *detection* and
nothing bounded the rebuild, which picked each surviving slot by scanning the keys accepted so
far. One duplicate in a wide root object:

| fields | before this issue | as first shipped | after the fix | real jq |
|--------|-------------------|------------------|---------------|---------|
| 10K    | 0.00 s            | 0.09 s           | 0.00 s        | 0.00 s  |
| 50K    | 0.01 s            | 1.92 s           | 0.01 s        | 0.02 s  |
| 100K   | 0.02 s            | **8.54 s**       | 0.03 s        | 0.04 s  |
| 300K   | —                 | —                | 0.09 s        | 0.14 s  |

This is the same failure the section above congratulates itself on catching ("*not a tuning
knob* … 1240% on a 10 MB document"). The guard just landed on the wrong half, and
`document.rs`'s `IndexMap` counterpart — which was already right — sat next to it as the
model. Both now key the surviving slot off a map. `test_duplicate_keys_collapse_is_linear_1385`
would not finish in the quadratic form.

**2. A key that would not decode was silently dropped from output.** Open risk 4 named exactly
this hazard, checked `document.rs` for it, and missed the runner's own copy of the same loop.
`{"a\q":1,"b":2}` printed as `{"b":2}` — data loss on the one path that previously echoed such
a key verbatim — while `length` and `[.[]]` went on counting it, re-creating the incoherence
this issue exists to remove. Both copies now keep an unnamed key where it stands: it has no
name to collapse *on*, so it can neither absorb a field nor be absorbed.

**3. `--preserve-input` stopped being self-consistent.** The non-goal above ("continues to
preserve, by design") is true of the printer, which is gated on `jq_compat`, and false of the
evaluator, which is gated on a const the flag cannot reach — so `.` preserved three keys while
`length` answered 2. Making the flag reach the evaluator needs a third `EvalSemantics`
implementor and a third monomorphization of the generic evaluator, which is a worse trade than
the split itself: the flag governs *spelling*, exactly as it does for `4e4` vs `.n + 0`. The
split is now written down in `docs/compliance/jq/limitations.md` and pinned by
`test_preserve_input_duplicate_keys_are_output_only_1385`.

**4. Memory, which the measurement plan never asked for.** Every figure above is a time. Peak
RSS on a 16 MB, 1M-field object (no duplicates anywhere):

| filter   | before this issue | as first shipped | after the fix |
|----------|-------------------|------------------|---------------|
| `length` | 28.2 MB           | 198.2 MB         | 38.0 MB       |
| `.`      | 28.1 MB           | 128.8 MB         | 82.9 MB       |

`length` and the `LazyKeys` probe both materialized a four-cursor `DocumentField` per field
(152 bytes) to answer a question about *keys*. They now take a census that keeps one 64-bit
fingerprint per field (8 bytes) and owns a `String` only for keys whose fingerprints actually
collide — so a collision costs work, never a wrong answer. The printer's buffer took the
"remaining lever" this document identified and left unexplored: a `JsonCursor`'s `text` and
`index` are invariant across the document, so the buffer keeps two `bp_pos` and rebuilds
cursors against one hoisted pair, 88 bytes per field down to 40. On a realistic 10 MB document
`.` went 24.5 MB → 21.9 MB against an 18.5 MB baseline.

This document predicted that hoisting would also *speed things up* ("cut the buffer's memory
traffic by half"). It did not. Interleaved A/B of all five fixes against the tip that
preceded them, same laptop and same caveats as every number above — 21 paired repetitions,
median of the per-repetition ratio:

| input               | filter   | median | p25   | p75   |
|---------------------|----------|--------|-------|-------|
| 1 MB `json generate`| `.`      | +0.8%  | -1.5% | +1.9% |
| 1 MB `json generate`| `.[]`    | +1.2%  | -0.3% | +2.2% |
| 10 MB               | `.`      | +1.0%  | -0.0% | +2.2% |
| 10 MB               | `.[]`    | +1.1%  | +0.5% | +1.4% |
| 10 MB               | `length` | -0.2%  | -1.9% | +0.8% |

About +1% on the printer paths, free on `length` — three of the five straddle zero at p25, so
the honest reading is "at most a point, possibly nothing". The memory win is real and the
speed win predicted alongside it was not; rebuilding a cursor is cheap but so was copying one.

**5. `JsonFields::has_duplicate_keys`/`collapsed` were dead on arrival.** The design specified
them for `print_json`, which then grew its own span-based copies; nothing ever called the
originals, and being `pub` they drew no `dead_code` lint. 96 lines deleted — they were also
why patch coverage on `src/json/light.rs` read 15%. `raw_and_escaped`, from the same stage, is
genuinely used and stays. Deleting them also left exactly one pairwise-scan threshold in the
tree rather than three.

## Open risks for an implementer to sanity-check

1. **The 4-6% wants confirming on the pinned hosts.** Everything above was measured on a
   laptop under someone else's load. Re-run `succinctly bench run jq_bench` interleaved on the
   M4 Pro and the 7950X before treating the figure as settled — and note the cost is
   memory-traffic-shaped (a per-field buffer), which is exactly the kind that does not port
   between architectures.
2. **Shared-evaluator hazard.** ADR-0018 rule 2's standing warning, and a repeated failure in this
   repo: a builtin generic over `S` serves *both* modes, so a jq-motivated rewrite can silently
   regress yq. Every touched arm needs a yq-mode check.
3. **The `LazyKeys` + `Length` fast path must survive.** It currently answers from `fields.len()`
   without decoding keys; the deduped answer needs the key text. This arm gets slower by
   construction — quantify it rather than assuming it is lost in the noise.
4. **Non-string keys.** `key_str()` returns `None` for a YAML alias or complex key.
   `effective_fields` used to *drop* such fields on the deduping path (they never reached the
   `IndexMap`). Answered by defect 2 above: both collapse paths now keep them. Reachable for
   JSON after all, through a key whose escape will not decode — semi-indexing admits input a
   validating parser rejects.
5. **Memory has no line in the measurement plan above, and needed one.** Defect 4 was found by
   measuring peak RSS, which nothing in "Measurement plan" asks for. A change that buys time
   with a per-field buffer should report both.

## Critical files

| File                              | Role                                                          |
|-----------------------------------|---------------------------------------------------------------|
| `src/jq/eval.rs`                  | `EvalSemantics` + the two impls; `Expr::Iterate`, `builtin_length` |
| `src/jq/document.rs`              | `DocumentFields`, `keys_dedup`, `effective_fields`, the key census |
| `src/jq/eval_generic.rs`          | `Iterate`, `Length`, `Keys`/`KeysUnsorted`, `LazyKeys`, `collect_paths_generic` |
| `src/json/light.rs`               | `JsonFields` impl, `JsonStr::as_str`/`raw_and_escaped`, `JsonCursor::text`/`index` |
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
