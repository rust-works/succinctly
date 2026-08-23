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

## What the detector cost, and how it was made cheap (#1514)

Open risk 1 above asked for the 4-6% to be confirmed on the pinned hosts. It was not 4-6%.
Measured against `40c5ff93` on both boxes — interleaved, identity-gated, `codegen-units=1` (see
"The measurement had a hole in it" below) — the detector cost up to **+141%** on a document
holding one wide object, and *no object in that document carries a repeated key*.

Two corrections to what this document says above, both of which it had the information to avoid:

- **"The residue is the printer's alone."** It was not. The printer's share (`.`) was the smaller
  half; `keys_unsorted` — the path that had no probe at all before this series and gained a whole
  `census` walk — more than doubled, and `keys_unsorted | length` with it.
- **The numbers came from `succinctly json generate`'s default shape.** A detector's cost shows
  most on the workload where the guarded code is *already fastest*, which here is a single wide
  object (82K keys at 1 MB, 7.1M at 100 MB). On the record-shaped `users` pattern the same
  detector reads +14%, which is why it shipped.

### Finding the cost: disable the detector, then measure

Timings alone cannot separate "what the detector costs" from "what the code around it costs".
Building each binary a second time with the detector *forcibly disabled* can, and the whole
investigation turned on it:

| `wide` `keys_unsorted`, Apple M4 Pro | 10 MB | 100 MB |
|---|---|---|
| `main`, probe disabled | **−1.1%** | **−0.7%** |
| an intermediate fix, dedup disabled | +89.4% | +100.0% |
| the same intermediate, dedup on | +112.3% | +154.2% |

Row 1 proves #1514's premise: with the probe gone, `main` sits exactly on the pre-#1385
baseline, so on that path the detector was the entire cost. Row 2 caught a regression introduced
*by the fix* — see "The mistake the fix made".

### Where the cost sat

Three separate passes, which is why one bisect signal only ever revealed one of them:

1. **The printer** (`spans_repeat`) sorted the key *spans*, so every comparison chased a pointer
   into a random offset of the document text.
2. **The evaluator** ran a second full cons-list walk (`census`), `key_str()`-decoding every field
   — `as_str` scans for the closing quote, scans again for a backslash, then validates UTF-8 —
   before sorting anything.
3. **The probe ran in guard position**, ahead of the arm match, so `select(true)` and `map(.)`
   paid a full census and discarded the answer.

### The fix

- Hash with eight bytes per multiply and a splitmix64 finalizer, not byte-at-a-time FNV-1a.
- Hash the key's **raw span** when it is escape-free and ASCII, instead of decoding it. ASCII is
  what makes the substitution sound, not just fast: it is valid UTF-8, so `key_string` would have
  answered `Some` with those exact bytes, which keeps the keyed/unkeyed split `census` counts on.
- Run the probe **only where the answer is read positionally** (`.[n]`, `last`). `length` asks
  `effective_len`; `.[]` and `map` dedup during the walk they were already making; `first` needs
  nothing, because collapsing keeps a key at its first position and the first field is nobody's
  repeat; the materializing fallback applies the rule itself.
- **Walk keys without materializing values.** `DocumentFields::uncons_key` exists because `uncons`
  builds a whole `DocumentField`, constructing the value a key walk never looks at.
- Sort hashes; do not probe a table — except in `DistinctKeyCursors`, which answers per key as it
  streams and has nothing to sort yet.

### The mistake the fix made

Removing the probe walk from `keys_unsorted` did not make it faster, and the dedup-disabled build
said why: the replacement walk cost **+89% at 10 MB and +100% at 100 MB on its own**. Routing the
writer through the generic `DocumentFields::uncons` materialized each field's value as well as its
key, and the consumer then called `cursor.value()` to rebuild the key the walk had just dropped —
two wasted materializations per key, worth about what the probe had cost.

Local sanity checks did not catch it: the same binary measured 21% apart between two runs on a
laptop. Only a pinned box, with the detector disabled, separated the two effects.

### The table was the wrong structure, and only one architecture said so

An open-addressed hash set replaced every sort first. On an M4 Pro that was right. On a 7950X it
cost **24% on the identity path**, where at 7.1M keys the table is 134 MB against 32 MB of L3 per
CCD: a sort streams, a table does not. Sorting `u64` hashes instead is better than both on both
boxes — it keeps the printer's real win (registers, not pointer-chasing memcmp) without the
random access.

Two further findings, recorded so they are not re-derived:

- **Size the table; never let it grow into a wide object.** Shipped growing first and measured:
  `keys_unsorted` on `wide/10mb` went from +110% to +146% — the sort it replaced was *faster*.
  Seventeen doublings each rehash everything held.
- **Rejected: quadrupling instead of doubling.** Cuts rehashed entries to about a third and moved
  `wide/100mb` from +108.5% to +108.2%, with 10 MB flat. Growth was not where the money was: a
  rehash reads the old table sequentially and writes into one it is filling, so those inserts
  pipeline; live inserts do not.

### The measurement had a hole in it

`succinctly jq type` — which reports the root's type and touches no keys — measured **+12% on an
M4 Pro and +27% on a 7950X** between binaries in this series, scaling with input size (~0.19
ms/MB, so it is index build). It did not bisect to the same commit on the two machines. At
`codegen-units=1` it is **+0.2%**.

The crate has no `[profile.release]`, so it builds at `codegen-units = 16` with no LTO, and adding
code to one module changes how unrelated modules are optimized. Every CLI A/B this project has
recorded carries that component, this document's own figures included. All numbers below were
re-measured with `codegen-units=1` on both sides.

### What it costs now

`codegen-units=1`, median of 7, against `40c5ff93`. Control bands: M4 Pro −0.7%..+0.4%, 7950X
−2.8%..+0.8%. Identity: 25 configurations, 0 differences on each box.

| `wide` | M4 Pro `main` | M4 Pro tip | 7950X `main` | 7950X tip |
|---|---|---|---|---|
| `.` 1/10/100 MB | +24 / +30 / +34% | **+16 / +19 / +24%** | +39 / +46 / +49% | **+24 / +28 / +30%** |
| `keys_unsorted` | +88 / +123 / +136% | **+50 / +57 / +96%** | +129 / +137 / +141% | **+78 / +97 / +142%** |
| `\| select(true)` | +27 / +32 / +37% | **+4 / +4 / +5%** | +32 / +39 / +52% | **+2 / +4 / +3%** |
| `\| map(.)` | +20 / +25 / +30% | **+4 / +6 / +12%** | +38 / +39 / +38% | **+16 / +23 / +36%** |
| `\| length` | +77 / +105 / +119% | **+22 / +29 / +25%** | +98 / +106 / +107% | **+23 / +22 / +21%** |

Median across all 25 configurations: M4 Pro **+25.1% → +4.8%**, 7950X **+38.1% → +12.2%**. Head to
head against `main` on the 7950X: **median −12.0%, 24 of 25 rows faster**; the exception is
`keys_unsorted` at 100 MB (+3.8%). yq is flat on both (median −0.3%, every row inside ±1.6%).

### The floor

At 7.1M keys in one object the streaming table is 134 MB and every live insert is a DRAM round
trip — ~87 ns per key against a 670 ms baseline query. That is why `keys_unsorted` at 100 MB is
the one row that does not improve on a 7950X. Identity `.` pays the same absolute cost and reads
as +30% only because it has an order of magnitude more work to hide it behind: the
"measure a precheck where the guarded code is already fastest" rule, demonstrated rather than
argued.

### Memory

Peak RSS, Apple M4 Pro, MiB:

| workload | `40c5ff93` | `main` | tip |
|---|---|---|---|
| `wide/10mb` `.` | 20.8 | 63.0 | **57.1** |
| `wide/10mb` `keys_unsorted` | 20.8 | 29.7 | 52.9 |
| `wide/10mb` `\| length` | 20.8 | 29.8 | 29.8 |
| `wide/100mb` `.` | 132.5 | 514.0 | **459.7** |
| `wide/100mb` `keys_unsorted` | 132.5 | 188.9 | 388.7 |
| `wide/100mb` `\| length` | 132.5 | 189.0 | 189.0 |
| `users/10mb`, every filter | 16.4 | 16.5 | 16.5 |

`.` improves on `main` (the printer's `Vec<u64>` is half the width of the `Vec<&[u8]>` it
replaced) and `length` is identical. Only the streaming `keys_unsorted` path doubles: an
open-addressed table keeps 16 bytes per key at its half-load point where a sorted `Vec<u64>` keeps
8. That is the trade for the time above, and defect 4 in this document is the precedent for
reporting both rather than one.

## Open risks for an implementer to sanity-check

1. ~~**The 4-6% wants confirming on the pinned hosts.**~~ **Closed by
   [#1514](https://github.com/rust-works/succinctly/issues/1514), and the answer was not 4-6%.**
   On both pinned hosts the detector cost up to **+134%** on a document holding one wide
   object — none of whose objects carries a repeated key. The laptop figure was low for two
   reasons this risk named only half of: the machine, and the *workload*. See "What the
   detector cost, and how it was made cheap" below.

   The prediction that the cost is memory-traffic-shaped and would not port between
   architectures was right, and mattered more than expected: the fix's first structure won on
   an M4 Pro and lost 24% on a 7950X.
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
- [#1514](https://github.com/rust-works/succinctly/issues/1514) — what the detector cost once it
  was measured on the pinned hosts, and the work that made it cheap
