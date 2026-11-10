 # yq Memory Optimization Plan

This document analyzes memory usage in the `yq` command and proposes optimizations to reduce memory consumption.

**Status: Archived (snapshot, decided in #5865).** M2 shipped as the cursor-streaming navigation path
([M2 Streaming Navigation Benchmarks](../benchmarks/yq.md#m2-streaming-navigation-benchmarks));
M3 shipped with no measurable effect (see "M3 Results"). The other four phases were re-measured on
2026-11-10 and decided in [Phase decisions](#phase-decisions-5865) below: **M1** is #6813, **M4** is
#6814 (the printer the CLI reaches, not `to_json`), **M5 and M6 are not pursued**, and the largest term
was in none of the phases: the presentation tree, #6815. Everything under "Problem Statement" and after
is the January 2026 analysis, kept as the design record; its memory figures and the `OwnedValue` size
table predate the `Rc` container layout (#2999/#3000).

## Phase decisions (#5865)

Re-measured on 2026-11-10 at `f2535c855` (`main`), a release build with `--features cli`, on both
architectures: an AMD Ryzen 9 7950X (WSL2, 30 GiB) and an Apple M4 Pro (macOS 26.5.1, 24 GiB). No
other build or benchmark was running on either (the 7950X at load average 0.6; the M4 Pro showed only
Spotlight indexing and an idle daemon, with a load average near 3 that macOS inflates). Peak RSS does
not depend on CPU contention, so this does not bias these figures the way it would a timing. Every
`succinctly` run was under `memcap.py`. Inputs are `succinctly yaml generate 20mb|100mb -p <pattern>`
files (20.0 and 100.0 MiB). Each figure is the median of three runs of the peak resident set from
`/usr/bin/time` (`-f %M` on Linux, `-l` on macOS). Output went to `/dev/null`. The attribution is
`valgrind --tool=dhat` on the 7950X at 20 MiB, and `t-gmax` is DHAT's moment of peak live heap.

**Peak RSS, `users` pattern.** The fast-path row is the floor: the file plus the index.

| `users` file, peak RSS (MiB)               | 20 MiB 7950X | 20 MiB M4 Pro | 100 MiB 7950X | 100 MiB M4 Pro |
|--------------------------------------------|-------------:|--------------:|--------------:|---------------:|
| `.` (cursor fast path, the floor)          |           58 |            58 |           259 |            274 |
| `.users[] \| {3 fields}` (M1)              |          156 |           175 |           735 |            844 |
| same, `\| length` (one number per result)  |           83 |           117 |           372 |            553 |
| `[.users[] \| {3 fields}]` (M4)            |          148 |           163 |           660 |            737 |
| same, `\| length`                          |          131 |           139 |           614 |            665 |
| `.zzz = 1`, `-o json -I0` (whole document) |          265 |           309 |          1259 |           1494 |
| `.zzz = 1`, `-o json` (indented)           |          341 |           337 |          1454 |           1722 |
| `.zzz = 1`, `-o yaml`                      |          995 |          1008 |          4901 |           4959 |

**Peak RSS, the whole-document write across the four patterns at 100 MiB.**

| 100 MiB, `.zzz = 1` (peak RSS, MiB) | -I0 7950X | -I0 M4 Pro | indent 7950X | indent M4 Pro | yaml 7950X | yaml M4 Pro |
|-------------------------------------|----------:|-----------:|-------------:|--------------:|-----------:|------------:|
| `users`                             |      1259 |       1494 |         1454 |          1722 |       4901 |        4959 |
| `navigation` (`.[0].zzz = 1`)       |      1343 |       1509 |         1378 |          1613 |       5713 |        5991 |
| `comprehensive`                     |       993 |       1111 |         1160 |          1226 |       3746 |        3855 |
| `nested`                            |       766 |       1178 |         1081 |          1335 |       2816 |        2785 |

**M1 control: the same query in `jq` mode.** `succinctly jq` hands each result to the printer as the
evaluator produces it (`evaluate_bytes_streaming`); `succinctly yq` collects them all first. Both read
the same JSON file.

| `users` as JSON, `Q = .users[] \| {name, age, city}` | 20 MiB 7950X | 20 MiB M4 Pro | 100 MiB 7950X | 100 MiB M4 Pro |
|------------------------------------------------------|-------------:|--------------:|--------------:|---------------:|
| `succinctly yq -o json -I0 Q`                        |          152 |           167 |           716 |            814 |
| `succinctly jq -c Q`                                 |           31 |            27 |           118 |            114 |
| `succinctly jq -c .` (the floor)                     |           29 |            26 |           116 |            113 |

**Where the heap is at the peak.**

| DHAT, 20 MiB `users`, 7950X             | live at peak (MiB) | where                                                |
|-----------------------------------------|-------------------:|------------------------------------------------------|
| `.zzz = 1`, `-o json -I0`               |              217.0 | 42% under `format_json_impl` and its `join`          |
| `.zzz = 1`, `-o json`                   |              260.1 | 51% under `format_json_impl` and its `join`          |
| `.zzz = 1`, `-o yaml`                   |              864.6 | 57.6% in two `CommentTree` bucket arrays (2 x 249.2) |
| `.users[] \| {3 fields}`, `-o json -I0` |              126.7 | 20 MiB source, the index and every result object     |

**M5 and M6 are bounded by counting, not by building them.** The 20 MiB `users` file has 145,830
objects, 1,020,804 entries and 8 distinct keys; the DHAT run shows 1,020,796 key allocations for 4.1
MB of key text, which is 31.1 MiB of 32-byte glibc chunks (15.6 MiB of 16-byte `libmalloc` chunks on
the M4 Pro) plus 7.8 MiB for shrinking a 24-byte `String` to a 16-byte handle. The hash tables measure
12.2 MiB per copy (145,829 x 88 bytes) plus 7.8 MiB of stored hashes. The same arithmetic on each
pattern's structure counts, as a share of the 20 MiB `.zzz = 1` `-o json -I0` peak RSS above:

| Phase                                                  | `users` 7950X | `users` M4 Pro | `navigation` 7950X | `comprehensive` 7950X | `nested` 7950X |
|--------------------------------------------------------|--------------:|---------------:|-------------------:|----------------------:|---------------:|
| M5, lazy `IndexMap` (hash table + stored hashes)       |          8.0% |           6.1% |               8.1% |                  6.7% |           8.8% |
| M6, shared key strings (blocks + 24 to 16 byte handle) |         14.7% |           7.6% |              12.3% |                  2.9% |           5.9% |

`comprehensive` and `nested` have mostly unique keys (490,891 distinct of 703,071 entries and 334,341
of 656,962), so sharing finds little. The percentages use a structure model that rounds each table to
a malloc chunk. On `users` it agrees with the counts above: 21.1 MiB modelled for M5 against 20.0 MiB
counted (12.2 MiB of tables measured by DHAT plus 7.8 MiB of stored hashes), and 38.9 MiB for M6 by
both.

### Decisions

| Phase                   | Verdict                       | Where | Measured headroom                                                                                                                                                                 |
|-------------------------|-------------------------------|-------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| M1, stream results      | **Implement**                 | #6813 | `yq` 716 / 814 MiB against `jq` 118 / 114 MiB on the same query at 100 MiB (7950X / M4 Pro): 6.1x / 7.1x                                                                          |
| M2, cursor navigation   | Shipped                       | -     | -                                                                                                                                                                                 |
| M3, exit-status clone   | Shipped                       | -     | No measurable change (see "M3 Results")                                                                                                                                           |
| M4, streaming JSON      | **Done**, as the printer      | #6814 | `to_json` already writes into one buffer; `format_json_impl` (`output.rs`) still does not: 42-51% of the live heap at a whole-document write, about 11% for a result-shaped print |
| M5, lazy `IndexMap`     | **Not pursued**               | -     | At most 6-9% of peak RSS (table above); touches the `Deref` contract of `ObjectMapOf`                                                                                             |
| M6, key interning       | **Not pursued**               | -     | At most 15% (7950X) / 8% (M4 Pro) of peak RSS on record-shaped input, 3-6% on unique-key shapes; changes the key type of every object                                             |
| (new) presentation tree | **Narrowed**, #6815           | #6815 | 58% of the live heap at a YAML-output write; that write is 3.3-3.9x the JSON-output write (3.7-3.9x on the 7950X, 3.3x on the M4 Pro)                                             |

- **M1** is the largest term that a query-shaped workload hits and the only one with an existing
  implementation in the same binary (`jq` mode). Streaming needs the evaluator to hand results out one
  at a time, not only the runner: `evaluate_yaml_cursor` returns a `Vec` before `all_results` is
  filled. #6813 sets out the `---` look-ahead and the error-ordering rules a change has to keep.
- **M4** as the plan wrote it (`to_json`'s `Vec<String>` and `join`) no longer exists:
  `OwnedValue::write_json_at_depth` appends into one `String`. The same shape is alive in
  `output.rs`, which the CLI reaches for a computed result, and that is #6814.
- **M4 landed as #6814**: `format_json_impl` appends arrays and objects into one buffer. Output is
  byte-identical (827 flag/mode/input runs against the old binary, and a `serde_json`-oracle unit test).
  Interleaved A/B, 7 repetitions, median peak RSS and fastest time, `.zzz = 1` with `-o json`: 100 MiB
  `users` 1260 to 1059 MiB (`-I0`) and 1454 to 1059 MiB (indented) on the 7950X, 1505 to 1174 and 1730 to
  1197 MiB on the M4 Pro; over the four patterns RSS falls 9-47% and time 10-45%, and a result that is
  only part of the document falls 3-12% and 6-24%. Indented output now peaks where compact does.
- **M5** would save at most 6-9% of the peak on the shapes measured, and it would change what
  `ObjectMapOf` derefs to: 135 `as_object()` call sites, 885 `OwnedValue::Object` arms, the `Rc`
  pointer used as node identity (`ptr_eq`, #2889) and the shadow list of a repeated key. A map that
  builds its index on the first lookup needs interior mutability behind a type that is `Rc`-shared,
  for a few percent.
- **M6** is shape-dependent (15% on repeated-key records, 3% on `comprehensive`) and changes the key
  type that `IndexMap<String, V>` exposes at every one of those sites. #3000 is the warning about
  trading allocations for RSS: shrinking one type while its twin kept its width cost +25..+41% RSS
  with 15% less live heap. Revisit only if a measurement taken after #6813 and #6815 shows keys as
  the leading remaining term.
- **The presentation tree (#6815)** was not in the plan. A `CommentTree` is about 224 bytes a node
  (DHAT: a 7-field record's bucket array is 1,792 bytes, 256 per bucket against 64 for the same
  record's `OwnedValue`), comment-free nodes included, and `reconcile_presentation` builds a second
  one beside the pristine tree. That is why `-o yaml` writes cost 27-60 MiB of RSS per MiB of input,
  and why the M1-M6 figures above (all `-o json`) understate the YAML cost.
- **The presentation tree's width landed as #6815.** A `CommentTree` node is 32 bytes, from 224: its
  `NodeMeta` sits behind a one-word `MetaSlot` that boxes it only when the node says something, an
  object's field map behind a one-word `ChildMap`, and its per-key metadata behind a `KeyMap` that
  allocates on its first entry. Output is byte-identical on every row below. Interleaved A/B on `main`
  @ `5c9222a76`, release builds, peak RSS (median of 5 at 20 MiB, of 3 at 100 MiB) and the fastest
  wall time, `.zzz = 1` with `-o yaml` unless noted:

  | peak RSS (MiB) / time (s)         | 7950X before |  7950X after | M4 Pro before | M4 Pro after |
  |-----------------------------------|-------------:|-------------:|--------------:|-------------:|
  | `users` 20 MiB                    |   999 / 2.33 |   594 / 2.06 |   1008 / 1.32 |   589 / 1.25 |
  | `users` 20 MiB, `-P`              |  1324 / 2.61 |   717 / 2.22 |   1290 / 1.46 |   672 / 1.36 |
  | `users` 20 MiB, `del(.users[0])`  |  1000 / 2.32 |   634 / 2.04 |   1098 / 1.35 |   611 / 1.28 |
  | `users` 100 MiB                   | 4905 / 11.74 | 2898 / 10.16 |   4960 / 7.04 |  2852 / 6.63 |
  | `navigation` 100 MiB (`.[0].zzz`) | 5718 / 13.35 | 3316 / 11.88 |   5975 / 7.24 |  3069 / 6.72 |
  | `comprehensive` 100 MiB           |  3750 / 9.37 |  2092 / 8.28 |   3856 / 6.08 |  2014 / 5.65 |
  | `nested` 100 MiB                  |  2821 / 7.88 |  1627 / 7.02 |   2790 / 4.65 |  1629 / 4.46 |

  That is 37-49% less peak RSS and 4-15% less time. The controls do not move: the JSON-output write
  (1060 MiB on the 7950X before and after) and the `.` fast path (261 and 262 MiB). A `users` query
  whose results carry a tree falls 159 to 135 MiB (7950X) and 175 to 139 MiB (M4 Pro). DHAT on the
  20 MiB `users` write (7950X) reads 436.1 MiB live at the peak, from 864.6. What remains is four
  62 MiB blocks (the document materialized as `OwnedValue` twice, once by `to_owned_with_comments` and
  once by the assignment's target resolution, plus a pristine and a reconciled tree), each tree also
  paying 12 MiB for its hash tables and 10 MiB for the object boxes. The pristine tree cannot be
  consumed as the reconciled one is built (several results reconcile against it, and a later pass reads
  it), and collapsing a subtree that says nothing into one leaf changes what the comment setters
  (`head_comment =` and its relatives, which index a node's members by position) find, so neither is
  part of this change.

## Problem Statement

The `yq` command uses more memory than expected given that succinct data structures are designed for space efficiency. While benchmarks show succinctly uses **7-14x less memory** than system `yq` on large files, the absolute memory usage is still higher than the theoretical minimum.

**Current memory usage (100MB YAML file, Apple M1 Max):**

| Pattern       | succinctly | system yq | Ratio |
|---------------|------------|-----------|-------|
| nested        | 250 MB     | 4 GB      | 0.06x |
| comprehensive | 491 MB     | 7 GB      | 0.07x |
| users         | 545 MB     | 9 GB      | 0.06x |

**Expected**: If succinct structures are ~10-15% overhead, a 100MB file should use ~115MB total.

**Actual**: 250-545MB = **2.5-5.5× the input size**.

## Root Cause Analysis

### 1. YamlIndex Memory (Efficient - ~15-25% overhead)

The succinct data structures are working as designed:

| Structure                  | Size Formula     | Purpose                    |
|----------------------------|------------------|----------------------------|
| `ib` (interest bits)       | N/8 bytes        | Structural position markers|
| `ib_rank`                  | N/16 bytes       | Cumulative popcount        |
| `bp` (balanced parens)     | ~2M/8 bytes      | Tree structure             |
| `bp` indices (L0/L1/L2)    | ~5% of BP        | Min-excess navigation      |
| `ty` (type bits)           | M/8 bytes        | Container type markers     |
| **`bp_to_text`**           | **M × 4 bytes**  | BP→text offset mapping     |
| **`bp_to_text_end`**       | **M × 4 bytes**  | Scalar end positions       |
| `containers`               | M/8 bytes        | Marker bits (`seq_items` eliminated in #104/O4) |
| `newlines` (BitVec)        | N/8 + indices    | Line/column lookup         |

Where:
- `N` = input size in bytes
- `M` = number of nodes (BP opens)

For typical YAML, `M ≈ N/10` (average ~10 bytes per node), so:
- `bp_to_text` + `bp_to_text_end` ≈ 0.8× input size
- Total YamlIndex overhead ≈ 15-25% of input size

**Verdict**: YamlIndex is efficient. The overhead comes from elsewhere.

### 2. OwnedValue DOM Materialization (Primary Problem - 3-5× overhead)

The **major memory consumer** is converting the indexed representation to `OwnedValue`:

```rust
pub enum OwnedValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),                         // 24 bytes + heap allocation
    Array(ArrayVec),                        // 8 bytes + heap allocation (#2999)
    Object(ObjectMap),                      // 8 bytes + heap allocation (#3000, #2999)
}
```

**Memory overhead per value type.** Every arm pays the widest one, so the whole
table's "stack size" is really `size_of::<OwnedValue>()`, which #3000 took from
72 bytes to **32** by boxing the object map — `NumberLiteral(NumberRepr,
Box<str>)` is the widest arm now, and `Object` costs one pointer. #2999 then
put both containers behind an `Rc`, so a clone of any value shares every
container under it until one side writes (copy-on-write); the widths below are
unchanged, the heap side gains a 16-byte refcount header per container:

| Type      | Payload    | Heap Overhead           | Notes                        |
|-----------|------------|-------------------------|------------------------------|
| Null/Bool | 0-1 bytes  | 0                       | Discriminant only            |
| Int/Float | 8 bytes    | 0                       | 8-byte value                 |
| String    | 24 bytes   | len + capacity padding  | Copies source text           |
| Array     | 8 bytes    | 40 + 32×len + capacity  | Rc<Vec> (#2999), shared      |
| Object    | 8 bytes    | 88 + ~40×entries + keys | Rc<IndexMap> (#3000, #2999)  |

**Why OwnedValue is 3-5× the source text:**

1. **String duplication**: Every string value copies bytes from source
2. **IndexMap overhead**: ~40 bytes per key-value pair for hash table
3. **Vec capacity**: Vecs often allocate 2× needed capacity
4. **Recursive overhead**: Each nested value has its own allocations
5. **Alignment padding**: Rust aligns to 8 bytes
6. **One allocation per container** (#3000, #2999): the refcounted map or
   `Vec` costs a `malloc` even for `{}`/`[]`, which is the trade for every
   *other* value being 32 bytes instead of 72 -- and, since #2999, for a
   clone of a document costing one refcount bump per container instead of a
   deep copy

### 3. Result Buffering (Additional 1-2×)

Non-fast-path code buffers all results before output:

```rust
// yq_runner.rs:1643
let mut all_results: Vec<Vec<Vec<OwnedValue>>> = Vec::new();
```

This triple-nested structure holds **all query results in memory** before writing any output.

### 4. Cloning for Exit Status (Additional copies)

```rust
// yq_runner.rs:1698
last_output = Some(result.clone());
```

Every result is cloned just to track the last value for `--exit-status`.

### 5. JSON Serialization Intermediates

```rust
// value.rs:194 - to_json()
let parts: Vec<String> = elements.iter().map(|e| e.to_json()).collect();
parts.join(",")
```

Creates intermediate `Vec<String>` during serialization.

## Memory Flow Diagram

```
Input (YAML file)
  │
  ▼
Vec<u8> ─────────────────────────────────────── [1× input size]
  │
  ▼
YamlIndex::build()
  │
  ├─ ib, bp, ty, containers ─────────────────── [~0.2× input size]
  ├─ bp_to_text, bp_to_text_end ─────────────── [~0.8× input size]
  └─ rank indices, newlines ─────────────────── [~0.1× input size]
  │                                              ─────────────────
  │                                              [~1.1× input size]
  ▼
┌─────────────────────────────────────────────────────────────────┐
│ FAST PATH (identity + JSON compact output)                      │
│                                                                 │
│   stream_json() ──────────────────────────── [~0× additional]   │
│   (writes directly to output, no OwnedValue)                    │
│                                                                 │
│   TOTAL: ~2.1× input size                                       │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│ STANDARD PATH (all other queries)                               │
│                                                                 │
│   evaluate_yaml_cursor()                                        │
│     └─ to_owned() ────────────────────────── [3-5× input size]  │
│                                                                 │
│   all_results: Vec<Vec<Vec<OwnedValue>>> ─── [held in memory]   │
│                                                                 │
│   for result in results {                                       │
│       last_output = Some(result.clone()); ── [+1× result size]  │
│       output_value(&result) {                                   │
│           to_json() ────────────────────────  [+1× result size] │
│       }                                                         │
│   }                                                             │
│                                                                 │
│   TOTAL: ~6-8× input size                                       │
└─────────────────────────────────────────────────────────────────┘
```

## Code Locations

| Location                                                                 | Function                | Memory Impact                 |
|--------------------------------------------------------------------------|-------------------------|-------------------------------|
| [yq_runner.rs:130-211](../../src/bin/succinctly/yq_runner.rs#L130-L211)  | `yaml_to_owned_value()` | Full DOM materialization      |
| [eval_generic.rs:29-64](../../src/jq/eval_generic.rs#L29-L64)            | `to_owned()`            | Query result materialization  |
| [yq_runner.rs:1643](../../src/bin/succinctly/yq_runner.rs#L1643)         | `all_results`           | Triple-nested buffer          |
| [yq_runner.rs:1698](../../src/bin/succinctly/yq_runner.rs#L1698)         | `last_output.clone()`   | Redundant cloning             |
| [value.rs:194](../../src/jq/value.rs#L194)                               | `to_json()`             | Intermediate strings          |
| [yq_runner.rs:596-700](../../src/bin/succinctly/yq_runner.rs#L596-L700)  | `emit_yaml_value()`     | Output string building        |

## Proposed Optimizations

### Phase M1: Streaming Output for Non-Identity Queries

**Goal**: Eliminate `all_results` buffering by streaming results as they're generated.

**Current code:**
```rust
// Collect all results first
let mut all_results: Vec<Vec<Vec<OwnedValue>>> = Vec::new();
for (bytes, format) in &input_sources {
    let (doc_results, _) = evaluate_yaml_direct_filtered(...)?;
    all_results.push(doc_results);
}

// Then output
for doc_results in all_results {
    for results in doc_results {
        for result in results {
            output_value(&result)?;
        }
    }
}
```

**Proposed:**
```rust
// Stream results directly
for (bytes, format) in &input_sources {
    evaluate_yaml_direct_streaming(bytes, &expr, |result| {
        output_value(&result)?;
        Ok(())
    })?;
}
```

**Impact**: Eliminates holding all results in memory. Memory becomes O(max_single_result) instead of O(total_results).

**Complexity**: Medium. Requires refactoring output logic to handle streaming.

---

### Phase M2: Reference-Based Results for Navigation Queries

**Goal**: Avoid `to_owned()` for queries that just navigate to subtrees.

For queries like `.users[0]` or `.config.database`, the result is a subtree of the input. Instead of materializing it as OwnedValue, stream directly from the cursor.

#### The Core Problem: Lifetime Constraints

The fundamental challenge is that cursors borrow from `YamlIndex`, which is created inside evaluation functions:

```rust
fn evaluate_yaml_direct(bytes: &[u8], expr: &Expr) -> Result<Vec<OwnedValue>> {
    let index = YamlIndex::build(bytes);    // Index lives here
    let root = index.root(bytes);           // Cursor borrows from index

    // ... evaluate ...

    // index dropped here - cursors become invalid!
}
// Return type must OWN its data - can't return cursors
```

**Key insight**: We can't return cursors from functions that create the index. Instead, we must **stream results before the index is dropped**.

#### Current Materialization Points

In [eval_generic.rs](../../src/jq/eval_generic.rs), `into_owned()` is called at these locations:

| Line    | Context                  | Can Avoid? |
|---------|--------------------------|------------|
| 233     | `into_owned()` One       | Yes        |
| 234     | `into_owned()` OneCursor | Yes        |
| 235     | `into_owned()` Many      | Yes        |
| 247-249 | `collect_owned()`        | Yes        |
| 389-392 | Pipe processing          | Partial    |

In [yq_runner.rs](../../src/bin/succinctly/yq_runner.rs), `to_owned()` is called at these locations:

| Line    | Function                   | Can Avoid?                     |
|---------|----------------------------|--------------------------------|
| 464     | `evaluate_yaml_cursor()`   | Yes - stream instead           |
| 465     | `evaluate_yaml_cursor()`   | Yes - stream instead           |
| 466     | `evaluate_yaml_cursor()`   | Yes - stream instead           |

#### Implementation Sub-Phases

##### M2.1: Streaming Evaluation API

Create a new entry point that streams results directly instead of returning them:

```rust
/// Evaluate and stream results directly to output.
///
/// This keeps the YamlIndex alive during output, avoiding materialization.
pub fn evaluate_yaml_streaming<W: core::fmt::Write>(
    bytes: &[u8],
    expr: &Expr,
    output: &mut W,
    config: &OutputConfig,
) -> Result<OutputStats> {
    let index = YamlIndex::build(bytes)?;
    let root = index.root(bytes);

    // Evaluate - results may be cursors or owned values
    let result = eval_with_cursor(expr, root);

    // Stream each result immediately (index still alive)
    let stats = stream_generic_result(result, output, config)?;

    // Index dropped here, AFTER streaming
    Ok(stats)
}

pub struct OutputStats {
    pub count: usize,
    pub last_was_falsy: bool,  // For --exit-status
}
```

##### M2.2: StreamableValue Trait

Add a trait for values that can stream without allocation:

```rust
// In src/jq/document.rs or new src/jq/stream.rs
pub trait StreamableValue {
    /// Stream this value as JSON to the output.
    fn stream_json<W: core::fmt::Write>(&self, out: &mut W) -> core::fmt::Result;

    /// Stream this value as YAML to the output.
    fn stream_yaml<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: usize
    ) -> core::fmt::Result;

    /// Check if value is falsy (null or false) without full materialization.
    fn is_falsy(&self) -> bool;
}

// Implement for YamlCursor (already has stream_json)
impl<'a, W: AsRef<[u64]>> StreamableValue for YamlCursor<'a, W> {
    fn stream_json<Out: core::fmt::Write>(&self, out: &mut Out) -> core::fmt::Result {
        self.stream_json_value(out)  // Existing method
    }

    fn stream_yaml<Out: core::fmt::Write>(
        &self,
        out: &mut Out,
        indent: usize
    ) -> core::fmt::Result {
        // New: stream as YAML directly
        stream_yaml_from_cursor(self, out, indent)
    }

    fn is_falsy(&self) -> bool {
        matches!(self.value(), YamlValue::Null)
    }
}

// Implement for OwnedValue (needs new streaming method)
impl StreamableValue for OwnedValue {
    fn stream_json<W: core::fmt::Write>(&self, out: &mut W) -> core::fmt::Result {
        // Replace to_json() with direct streaming
        stream_owned_value_json(self, out)
    }

    fn stream_yaml<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: usize
    ) -> core::fmt::Result {
        stream_owned_value_yaml(self, out, indent)
    }

    fn is_falsy(&self) -> bool {
        matches!(self, OwnedValue::Null | OwnedValue::Bool(false))
    }
}
```

##### M2.3: GenericResult Streaming Methods

Add streaming capability to `GenericResult`:

```rust
impl<V: DocumentValue> GenericResult<V>
where
    V: StreamableValue,
    V::Cursor: StreamableValue,
{
    /// Stream all results to output without intermediate allocation.
    /// Returns (count, last_was_falsy) for exit status handling.
    pub fn stream_all<W: core::fmt::Write>(
        self,
        out: &mut W,
        separator: &str,
    ) -> Result<(usize, bool), core::fmt::Error> {
        let mut count = 0;
        let mut last_falsy = false;

        match self {
            GenericResult::One(v) => {
                last_falsy = v.is_falsy();
                v.stream_json(out)?;
                count = 1;
            }
            GenericResult::OneCursor(c) => {
                last_falsy = c.is_falsy();
                c.stream_json(out)?;
                count = 1;
            }
            GenericResult::Many(vs) => {
                for (i, v) in vs.iter().enumerate() {
                    if i > 0 { out.write_str(separator)?; }
                    last_falsy = v.is_falsy();
                    v.stream_json(out)?;
                    count += 1;
                }
            }
            GenericResult::Owned(o) => {
                last_falsy = o.is_falsy();
                o.stream_json(out)?;
                count = 1;
            }
            GenericResult::ManyOwned(os) => {
                for (i, o) in os.iter().enumerate() {
                    if i > 0 { out.write_str(separator)?; }
                    last_falsy = o.is_falsy();
                    o.stream_json(out)?;
                    count += 1;
                }
            }
            GenericResult::None | GenericResult::Error(_) | GenericResult::Break(_) => {}
        }

        Ok((count, last_falsy))
    }
}
```

##### M2.4: Extend Fast Path in yq_runner

Currently the fast path only handles identity queries. Extend to navigation:

```rust
// Queries that can stream without materialization
fn can_stream_query(expr: &Expr) -> bool {
    match expr {
        Expr::Identity => true,
        Expr::Field(_) => true,
        Expr::Index(_) => true,
        Expr::Iterate => true,
        Expr::Optional(inner) => can_stream_query(inner),
        Expr::Pipe(exprs) => exprs.iter().all(can_stream_query),
        // Computed values require materialization
        Expr::Literal(_) => false,
        Expr::ArrayConstruct(_) => false,
        Expr::ObjectConstruct(_) => false,
        // ... etc
        _ => false,
    }
}

// In run_yq():
let can_stream = can_stream_query(&program.expr)
    && output_config.output_format == OutputFormat::Json
    && output_config.compact
    && !args.null_input
    && !args.slurp
    && context.named.is_empty();

if can_stream {
    // Use streaming evaluation
    for (bytes, format) in &input_sources {
        let stats = evaluate_yaml_streaming(
            bytes,
            &program.expr,
            &mut FmtWriter(&mut writer),
            &output_config,
        )?;
        // ... handle stats for exit status
    }
} else {
    // Fall back to OwnedValue path
    // ... existing code
}
```

##### M2.5: YAML Output Streaming

Add streaming support for YAML output format (currently only JSON streams):

```rust
// In src/yaml/light.rs or new file
pub fn stream_yaml_from_cursor<W: core::fmt::Write>(
    cursor: &YamlCursor<'_, impl AsRef<[u64]>>,
    out: &mut W,
    indent: usize,
) -> core::fmt::Result {
    let indent_str = "  ".repeat(indent);

    match cursor.value() {
        YamlValue::Null => out.write_str("null"),
        YamlValue::String(s) => {
            // Smart quoting based on content
            let str_val = s.as_str().map_err(|_| core::fmt::Error)?;
            stream_yaml_string(out, &str_val)
        }
        YamlValue::Mapping(fields) => {
            let mut first = true;
            for field in fields {
                if !first {
                    out.write_char('\n')?;
                    out.write_str(&indent_str)?;
                }
                first = false;

                // Key
                if let YamlValue::String(s) = field.key() {
                    let key = s.as_str().map_err(|_| core::fmt::Error)?;
                    stream_yaml_string(out, &key)?;
                }
                out.write_str(": ")?;

                // Value (recursive)
                stream_yaml_from_cursor(&field.value_cursor(), out, indent + 1)?;
            }
            Ok(())
        }
        YamlValue::Sequence(elements) => {
            for elem in elements {
                out.write_str(&indent_str)?;
                out.write_str("- ")?;
                stream_yaml_value(out, elem, indent + 1)?;
                out.write_char('\n')?;
            }
            Ok(())
        }
        // ... handle other cases
    }
}
```

#### Which Queries Benefit from M2

| Query Pattern              | Can Stream? | Notes                              |
|----------------------------|-------------|------------------------------------|
| `.`                        | ✅ Yes      | Already has fast path              |
| `.field`                   | ✅ Yes      | Returns cursor to subtree          |
| `.[0]`                     | ✅ Yes      | Returns cursor to element          |
| `.[]`                      | ✅ Yes      | Returns Many(cursors)              |
| `.users[].name`            | ✅ Yes      | Chained navigation                 |
| `.users \| .[0]`           | ✅ Yes      | Pipe of navigations                |
| `select(.age > 30)`        | ⚠️ Partial  | Filter can stream, condition can't |
| `.a + .b`                  | ❌ No       | Arithmetic requires values         |
| `[.a, .b]`                 | ❌ No       | Array construction                 |
| `{name: .n}`               | ❌ No       | Object construction                |
| `group_by(.x)`             | ❌ No       | Requires full materialization      |

#### Impact Estimate

| Query Type            | Current Memory | M2 Memory  | Improvement |
|-----------------------|----------------|------------|-------------|
| Identity `.`          | ~2× (fast)     | ~2× (fast) | -           |
| Navigation `.foo`     | 5-8×           | ~2.2×      | 2.5-3.5×    |
| Iteration `.[]`       | 5-8×           | ~2.5×      | 2-3×        |
| Filter `select()`     | 5-8×           | ~3-4×      | 1.5-2×      |
| Computation           | 5-8×           | 5-8×       | None        |

#### Implementation Order

1. **M2.1** - Streaming API (~2-3 hours)
   - New `evaluate_yaml_streaming()` function
   - Wire into yq_runner for identity queries

2. **M2.2** - StreamableValue trait (~2 hours)
   - Define trait in document.rs
   - Implement for YamlCursor (delegate to existing)
   - Implement for OwnedValue (new streaming methods)

3. **M2.3** - GenericResult streaming (~2 hours)
   - Add `stream_all()` method
   - Handle all result variants

4. **M2.4** - Extend fast path (~3-4 hours)
   - `can_stream_query()` analysis
   - Wire streaming path for navigation queries
   - Benchmarking and validation

5. **M2.5** - YAML output streaming (~3-4 hours)
   - `stream_yaml_from_cursor()`
   - Smart quoting logic
   - Multi-document handling

**Total estimate**: 12-15 hours of implementation

---

### Phase M3: Eliminate Exit Status Cloning

**Goal**: Replace `last_output = Some(result.clone())` with a flag or lightweight tracking.

**Current code:**
```rust
for result in results {
    last_output = Some(result.clone());  // Clones entire result!
    output_value(&result)?;
}

// Later:
if let Some(OwnedValue::Null | OwnedValue::Bool(false)) = last_output {
    return Ok(exit_codes::FALSE_OR_NULL);
}
```

**Proposed:**
```rust
enum LastOutputStatus {
    None,
    FalseOrNull,
    Other,
}

for result in results {
    last_status = match &result {
        OwnedValue::Null | OwnedValue::Bool(false) => LastOutputStatus::FalseOrNull,
        _ => LastOutputStatus::Other,
    };
    output_value(&result)?;
}
```

**Impact**: Eliminates O(result_size) cloning per result.

**Complexity**: Low. Simple refactor.

---

### Phase M4: Streaming JSON Serialization

**Goal**: Write JSON directly to output without intermediate strings.

**Current code:**
```rust
fn to_json(&self) -> String {
    match self {
        OwnedValue::Array(elements) => {
            let parts: Vec<String> = elements.iter().map(|e| e.to_json()).collect();
            format!("[{}]", parts.join(","))
        }
        ...
    }
}
```

**Proposed:**
```rust
fn write_json<W: Write>(&self, writer: &mut W) -> io::Result<()> {
    match self {
        OwnedValue::Array(elements) => {
            writer.write_all(b"[")?;
            for (i, elem) in elements.iter().enumerate() {
                if i > 0 { writer.write_all(b",")?; }
                elem.write_json(writer)?;
            }
            writer.write_all(b"]")
        }
        ...
    }
}
```

**Impact**: Eliminates intermediate `Vec<String>` and string concatenation.

**Complexity**: Low-Medium. Straightforward refactor of serialization.

---

### Phase M5: Lazy IndexMap Construction

**Goal**: Defer hash table construction for objects until needed.

For large objects that are immediately serialized, building the full `IndexMap` is wasteful.

**Proposed**: Use a lazy wrapper that only builds the hash table on `.get()`:

```rust
enum LazyObject {
    // Fields stored as Vec, no hash table yet
    Pending(Vec<(String, OwnedValue)>),
    // Hash table built on first lookup
    Indexed(IndexMap<String, OwnedValue>),
}
```

**Impact**: Reduces memory for objects that are serialized without random access.

**Complexity**: Medium. Requires changes to OwnedValue and query evaluation.

---

### Phase M6: String Interning for Repeated Keys

**Goal**: Share memory for common key strings.

YAML configs often have repeated keys (`name`, `value`, `metadata`, `spec`, etc.). String interning would deduplicate these.

**Proposed:**
```rust
struct InternedString {
    data: Arc<str>,
}

// Or use a string interner crate
```

**Impact**: Significant for configs with many repeated keys.

**Complexity**: Medium-High. Requires integration throughout codebase.

---

## Implementation Priority

| Phase | Impact    | Complexity  | Priority                      | Status              |
|-------|-----------|-------------|-------------------------------|---------------------|
| M3    | Low       | Low         | **P1** (quick win)            | ✅ Done              |
| M4    | Medium    | Low-Medium  | **P1** (quick win)            | ✅ Done (#6814)     |
| M1    | High      | Medium      | **P2** (major improvement)    | #6813               |
| M2    | Very High | High        | **P3** (requires design work) | ✅ Done              |
| M5    | Medium    | Medium      | **P4** (nice to have)         | Not pursued (#5865) |
| M6    | Medium    | Medium-High | **P4** (nice to have)         | Not pursued (#5865) |

### M3 Results

**Completed**: 2026-01-24

Replaced `last_output: Option<OwnedValue>` with `last_was_falsy: bool` to eliminate cloning for exit status tracking. The clone only occurred once per result loop (on the final iteration), not for every result, so **no measurable memory or performance improvement** was observed in benchmarks. However, the change simplifies the code and removes unnecessary allocations.

**Benchmark comparison** (100MB users file, `.users[]` query):
- Baseline: 1390 MB, 3.93s
- Optimized: 1390 MB, 3.99s
- Difference: ~0%

The real memory overhead comes from OwnedValue DOM materialization (M2) and result buffering (M1), not from exit status cloning.

## Expected Results

After implementing M1-M4:

| File Size | Current | Expected | Improvement |
|-----------|---------|----------|-------------|
| 100KB     | 8 MB    | 4 MB     | 2×          |
| 1MB       | 12 MB   | 6 MB     | 2×          |
| 10MB      | 58 MB   | 25 MB    | 2.3×        |
| 100MB     | 491 MB  | 150 MB   | 3.3×        |

After implementing M2 (reference-based results for navigation):

| File Size           | Current | Expected | Improvement |
|---------------------|---------|----------|-------------|
| 100MB (`.`)         | 491 MB  | 115 MB   | 4.3×        |
| 100MB (`.users[0]`) | 491 MB  | 115 MB   | 4.3×        |

## Comparison with Fast Path

The fast path (identity + JSON compact) already achieves optimal memory:

| File Size | Fast Path    | Standard Path | Ratio |
|-----------|--------------|---------------|-------|
| 100KB     | 200 KB index | 8 MB          | 40×   |
| 1MB       | 2 MB index   | 12 MB         | 6×    |
| 10MB      | 20 MB index  | 58 MB         | 3×    |
| 100MB     | 200 MB index | 491 MB        | 2.5×  |

The goal is to bring standard path closer to fast path performance.

## Related Documents

- [yq.md](yq.md) - yq command implementation plan
- [../parsing/yaml.md](../parsing/yaml.md) - YAML parser optimizations
- [../benchmarks/yq.md](../benchmarks/yq.md) - Benchmark results

## Changelog

| Date       | Change                                                            |
|------------|-------------------------------------------------------------------|
| 2026-11-10 | **M1, M4, M5, M6 decided (#5865)**: re-measured on both architectures; M1 and M4 filed (#6813, #6814), M5 and M6 not pursued, #6815 added; archived in place |
| 2026-01-24 | **M3 completed**: Eliminated exit status cloning (no perf impact) |
| 2026-01-24 | Added detailed M2 implementation plan with 5 sub-phases           |
| 2026-01-24 | Initial analysis and optimization plan                            |
