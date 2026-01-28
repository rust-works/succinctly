# End Position Encoding: From Array to Bitmaps

[Home](/) > [Docs](../) > [Optimizations](./) > End Positions

This document traces the evolution of the YAML end-position lookup through three
designs, each reducing memory while keeping query speed competitive. The designs
are:

1. **Direct Array** (`Vec<u32>`) — the baseline on `main`
2. **Three-Bitmap Encoding** — first compact design (~5x memory reduction)
3. **Two-Bitmap Encoding** — simplified design (fewer memory accesses, shorter dependency chain)

---

## Table of Contents

- [Background: What Problem Are We Solving?](#background-what-problem-are-we-solving)
- [Running Example](#running-example)
- [Design 1: Direct Array](#design-1-direct-array-vecu32)
- [Design 2: Three-Bitmap Encoding](#design-2-three-bitmap-encoding)
- [Design 3: Two-Bitmap Encoding](#design-3-two-bitmap-encoding)
- [Performance Results](#performance-results)
- [Key Lessons](#key-lessons)

---

## Background: What Problem Are We Solving?

Succinctly's YAML semi-index parses YAML text into a compact structural index.
Part of this index is a **balanced parentheses** (BP) tree where every node —
scalar values, mapping keys, containers — gets one open parenthesis and one close
parenthesis. Each open parenthesis is numbered sequentially (its **open index**).

To extract a scalar value from the original YAML text, we need two byte offsets:

- **Start position**: where the scalar text begins (e.g. byte 6 for `Alice`)
- **End position**: where the scalar text ends (e.g. byte 11, the `\n` after `Alice`)

The start positions are stored in `OpenPositions`. This document focuses on
**end positions**: one entry per BP open, storing the byte offset where the
node's text content ends.

### Key observations about end positions

1. **Containers have no end position.** A mapping or sequence node is a
   structural bracket — it doesn't have its own text span. These entries store
   `0` as a sentinel.

2. **Non-zero entries are monotonically non-decreasing.** In a depth-first
   traversal of the BP tree, scalar end positions never go backwards. Two
   adjacent scalars can share the same end position (e.g. duplicate values),
   but the sequence never decreases.

3. **End positions are only queried for scalars.** The `value()` function in
   `light.rs` exits early for containers, aliases, and quoted strings — it
   only reaches the end-position lookup for unquoted plain scalars.

These properties are what make compact encoding possible.

---

## Running Example

Throughout this document we use the following YAML:

```yaml
name: Alice
age: 30
active: true
```

This is 33 bytes of text (including trailing newline). The semi-index parser
creates a BP tree with **7 nodes** (1 container + 6 scalars):

```
               root (mapping)                    ← container
              /      |       \
          name    Alice    age    30    active    true
          (key)   (val)   (key)  (val)  (key)    (val)
```

Each node gets an **open index** in depth-first order:

```
open_idx │ Node        │ Kind      │ Text span   │ End pos
─────────┼─────────────┼───────────┼─────────────┼────────
   0     │ root        │ container │ (none)      │   0  ← sentinel
   1     │ key "name"  │ scalar    │ bytes 0–3   │   4
   2     │ val "Alice" │ scalar    │ bytes 6–10  │  11
   3     │ key "age"   │ scalar    │ bytes 12–14 │  15
   4     │ val "30"    │ scalar    │ bytes 17–18 │  19
   5     │ key "active"│ scalar    │ bytes 20–25 │  26
   6     │ val "true"  │ scalar    │ bytes 28–31 │  32
```

The **end positions array** is: `[0, 4, 11, 15, 19, 26, 32]`

The zero at index 0 is the container sentinel. The remaining values (4, 11, 15,
19, 26, 32) are monotonically increasing.

---

## Design 1: Direct Array (`Vec<u32>`)

This is the baseline design on the `main` branch.

### Data structure

```rust
// In YamlIndex:
bp_to_text_end: Vec<u32>   // one u32 per BP open
```

For our example:

```
Index:  0    1    2    3    4    5    6
Value:  0    4   11   15   19   26   32
        ↑
     sentinel
     (container)
```

### Query: `get(open_idx) → Option<usize>`

```rust
fn bp_to_text_end_pos(&self, bp_pos: usize) -> Option<usize> {
    let open_idx = self.bp.rank1(bp_pos);  // BP position → open index
    self.bp_to_text_end
        .get(open_idx)                     // array lookup
        .map(|&pos| pos as usize)
        .filter(|&pos| pos > 0)            // 0 sentinel → None
}
```

**Steps**: 1 rank query + 1 array load + 1 comparison.

### Walkthrough: looking up "30"

```
Input: bp_pos for the "30" value node
Step 1: open_idx = bp.rank1(bp_pos) = 4
Step 2: bp_to_text_end[4] = 19
Step 3: 19 > 0, so return Some(19)

The text from start_pos to end_pos (bytes 17–19) gives "30\n",
which is trimmed to "30".
```

### Memory

```
Storage = N × 4 bytes
       where N = number of BP opens

Example: N = 7  →  28 bytes
1MB YAML (N ≈ 130K opens): 130K × 4 = 520 KB
```

### Strengths and weaknesses

| Aspect          | Rating    | Notes                                             |
|-----------------|-----------|---------------------------------------------------|
| Memory          | Poor      | 4 bytes per node, even for zero-valued containers |
| Query speed     | Excellent | Single array index — one cache-line access        |
| Code simplicity | Excellent | A one-liner                                       |

The direct array is the gold standard for speed. Every query is a single
memory load. But for a 1 MB YAML file with ~130K nodes, it consumes ~520 KB
just for end positions — more than half the input size.

---

## Design 2: Three-Bitmap Encoding

The first compact design replaces the array with three bitmaps. To understand
why three, we need to introduce the concepts one at a time.

### The key insight: most entries are zero

In typical YAML, 30–40% of BP opens are containers (mappings, sequences,
sequence items). These always have end position = 0. We're storing 4 bytes of
zeros for every container — wasted space.

What if we could separate "which nodes have end positions?" from "what are those
positions?"

### Concept 1: The `has_end` bitmap

One bit per BP open. Set to 1 if the node has a non-zero end position (is a
scalar), 0 if it's a container.

```
open_idx:    0   1   2   3   4   5   6
end_pos:     0   4  11  15  19  26  32
has_end:     0   1   1   1   1   1   1
                 ↑                   ↑
              scalar              scalar
```

Using **rank** on this bitmap, we can compute a dense scalar index:

```
scalar_idx = has_end_rank1(open_idx)
           = number of 1-bits in has_end[0..open_idx)
```

```
open_idx:     0   1   2   3   4   5   6
has_end:      0   1   1   1   1   1   1
scalar_idx:   0   0   1   2   3   4   5
                  ↑
         first scalar → scalar_idx = 0
```

Now we only need to store positions for scalars: `[4, 11, 15, 19, 26, 32]` —
6 entries instead of 7.

But we still have 6 × 4 = 24 bytes for the positions. Can we encode those
more compactly?

### Concept 2: The Interest Bits (IB) bitmap

Instead of storing position values as integers, we mark positions in a bitmap
that spans the entire text. One bit per byte of input — set the bit at each
unique end position:

```
Text bytes:   0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 ...
IB:           0  0  0  0  1  0  0  0  0  0  0  1  0  0  0  1 ...

              ... 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32
              ...  0  0  0  1  0  0  0  0  0  0  1  0  0  0  0  0  1
```

Bits are set at positions 4, 11, 15, 19, 26, and 32 — exactly our six
end-position values. The **select** operation on IB recovers the actual
position:

```
IB select1(0) = 4     (the 1st set bit)
IB select1(1) = 11    (the 2nd set bit)
IB select1(2) = 15    (the 3rd set bit)
...
```

But which select index corresponds to which scalar? That's where the advance
bitmap comes in.

### Concept 3: The Advance bitmap

One bit per scalar (not per BP open — only entries where `has_end = 1`).
Set to 1 when the end position changes from the previous scalar.

```
scalar_idx:  0   1   2   3   4   5
position:    4  11  15  19  26  32
advance:     1   1   1   1   1   1     (all different → all advance)
```

When multiple scalars share the same end position (duplicates), the advance bit
is 0 for the repeats. Here all positions are distinct, so all bits are 1.

The **rank** of the advance bitmap tells us how many unique positions have been
seen up to a given scalar:

```
advance_count = advance_rank1(scalar_idx + 1)
```

Then the actual position is:

```
position = IB select1(advance_count - 1)
```

### Putting it together: the full query

```
get(open_idx):
  1. has_end[open_idx]  →  is this a scalar?
     If 0: return None (container)

  2. scalar_idx = has_end_rank1(open_idx)

  3. advance_count = advance_rank1(scalar_idx + 1)
     If 0: return None (impossible for valid data, but defensive)

  4. position = IB_select1(advance_count - 1)
     Return Some(position)
```

### Walkthrough: looking up "30" (open_idx = 4)

```
Step 1: has_end[4] = 1                    ← yes, it's a scalar
Step 2: scalar_idx = has_end_rank1(4) = 3
          (3 ones before index 4: has_end[1]=1, has_end[2]=1, has_end[3]=1)
Step 3: advance_count = advance_rank1(3+1) = advance_rank1(4) = 4
          (4 ones in advance[0..4): all set)
Step 4: IB_select1(4-1) = IB_select1(3) = 19
          (the 4th set bit in IB is at position 19)

Return Some(19)  ✓
```

### Walkthrough: looking up root container (open_idx = 0)

```
Step 1: has_end[0] = 0  ← container
Return None  ✓
```

### Data structures

```rust
struct CompactEndPositions {
    // Bitmap 1: which nodes have end positions
    has_end: Vec<u64>,          // ⌈N/64⌉ words
    has_end_rank: Vec<u32>,     // ⌈N/64⌉ + 1 entries

    // Bitmap 2: text-position markers
    ib_words: Vec<u64>,         // ⌈L/64⌉ words
    ib_select_samples: Vec<u32>,// sampled select index

    // Bitmap 3: position-change markers
    advance_words: Vec<u64>,    // ⌈S/64⌉ words
    advance_rank: Vec<u32>,     // ⌈S/64⌉ + 1 entries

    num_opens: usize,           // N
    num_scalars: usize,         // S
    ib_ones: usize,             // unique positions
    cursor: Cell<SequentialCursor>,
}
```

Where:
- **N** = total BP opens (containers + scalars)
- **S** = number of scalars (nodes with end position)
- **L** = text length in bytes

### Memory formula

```
                    has_end        IB           advance        rank/select
Compact storage = ⌈N/64⌉×8  +  ⌈L/64⌉×8  +  ⌈S/64⌉×8  +  overhead
                ≈   N/8      +    L/8      +    S/8      +  ~5%

vs. Direct Array = N × 4 bytes
```

For our running example (N=7, S=6, L=33):
```
Direct: 7 × 4 = 28 bytes
Compact: 8 + 8 + 8 + overhead ≈ 28 bytes  (too small to see savings)
```

For 1 MB YAML (N≈130K, S≈85K, L=1M):
```
Direct:  130K × 4           = 520 KB
Compact: 130K/8 + 1M/8 + 85K/8 + overhead
       ≈ 16 KB  + 125 KB + 11 KB + ~7 KB  ≈ 159 KB

Compression ratio: 520 / 159 ≈ 3.3×
```

### Sequential cursor optimization

In production, nodes are accessed in depth-first order (open_idx = 0, 1, 2,
3, ...). The random-access query above involves rank and select operations
that scan bitmaps — expensive if repeated thousands of times.

The **sequential cursor** maintains running totals that turn rank into a
single-bit test:

```rust
struct SequentialCursor {
    next_open_idx: usize,    // expected next query
    he_cumulative: usize,    // has_end_rank1(next_open_idx)
    adv_cumulative: usize,   // advance_rank1(he_cumulative)
    ib_word_idx: usize,      // IB scan position
    ib_ones_before: usize,   // ones in IB before scan position
    last_ib_arg: usize,      // duplicate detection
    last_ib_result: usize,   // cached result
}
```

**Size**: 7 fields × 8 bytes = 56 bytes.

On the fast path (`open_idx == cursor.next_open_idx`), the query becomes:

```
Step 1: Read has_end[open_idx / 64], test bit       ← 1 memory access
Step 2: scalar_idx = cursor.he_cumulative            ← register (free)
Step 3: Read advance[scalar_idx / 64], test bit      ← 1 memory access
Step 4: Forward-scan IB from cursor.ib_word_idx      ← 0–1 memory accesses
                                                        (usually same word)
```

**Total**: 2–3 memory accesses per sequential query (vs. ~70 for random access).

### Strengths and weaknesses

| Aspect          | Rating   | Notes                                                    |
|-----------------|----------|----------------------------------------------------------|
| Memory          | Good     | ~3–5× smaller than `Vec<u32>`                            |
| Query speed     | Moderate | 2–3 bitmap accesses on hot path                          |
| Code complexity | Moderate | Three bitmaps, two rank structures, cursor with 7 fields |

The 2–3 bitmap accesses per call introduce a measurable overhead compared to
the single array load of Design 1. On ARM Neoverse V2, this showed as a
**5–15% regression** in yaml_bench for small documents with many nodes.

---

## Design 3: Two-Bitmap Encoding

The second design eliminates the `has_end` bitmap entirely, reducing the hot
path from 3 bitmap accesses to 2 and shrinking the cursor.

### The key insight: `has_end` is always 1 on the hot path

The `value()` function in `light.rs` calls `text_end_position()` only for
**unquoted plain scalars**. By the time execution reaches this point, the code
has already returned for:

- Containers (line 158–164)
- Aliases (line 169–170)
- Quoted strings (line 250–256)
- Block scalars (line 258–276)

Therefore `has_end` is always 1 when checked. We are paying for a bitmap lookup
whose answer is always the same.

### The trick: zero-filling

Instead of storing zeros for containers and then checking `has_end` to skip
them, we **fill** container zeros with the previous non-zero value:

```
Before (original):   [0,  4, 11, 15, 19, 26, 32]
                      ↑
                   container

After (filled):      [0,  4, 11, 15, 19, 26, 32]
                      ↑
                   stays 0 (leading container — no previous value)
```

In this example there's only one leading container, so the filled array looks
the same. Here's an example with containers between scalars:

```
Before (original):   [0, 10,  0,  0, 20,  0, 30]
                      ↑       ↑   ↑       ↑
                   containers ─────────────┘

After (filled):      [0, 10, 10, 10, 20, 20, 30]
                      ↑       ↑   ↑       ↑
                   zero    filled with previous non-zero value
```

Now the entire sequence is monotonically non-decreasing (after any leading
zeros), so the advance bitmap can be indexed directly by **open_idx** instead
of scalar_idx — no `has_end` rank needed.

### What changes

| Aspect                       | 3-Bitmap               | 2-Bitmap                                      |
|------------------------------|------------------------|-----------------------------------------------|
| Bitmaps                      | has_end + IB + advance | IB + advance                                  |
| Advance indexed by           | scalar_idx (S entries) | open_idx (N entries)                          |
| Advance size                 | ⌈S/64⌉ words           | ⌈N/64⌉ words                                  |
| Container result             | `None`                 | `Some(prev_scalar_end)` or `None` for leading |
| Cursor fields                | 7 (56 bytes)           | 6 (48 bytes)                                  |
| Memory accesses (sequential) | 2–3                    | 1–2                                           |

### How zero-filling works

Zero-filling, monotonicity checking, and bitmap construction are performed
in a **single pass** by `CompactEndPositions::try_build()`. This avoids both
a temporary `Vec<u32>` allocation (A1) and a separate monotonicity scan (A2):

```rust
fn try_build(positions: &[u32], text_len: usize) -> Option<Self> {
    // ... bitmap setup ...

    let mut prev_effective: Option<u32> = None;
    let mut prev_nonzero = 0u32;

    for (i, &pos) in positions.iter().enumerate() {
        // Inline zero-fill: containers (pos == 0) inherit previous non-zero value
        let effective = if pos > 0 {
            // Monotonicity check: non-zero positions must be non-decreasing
            if prev_nonzero > 0 && pos < prev_nonzero {
                return None; // fall back to Dense
            }
            prev_nonzero = pos;
            pos
        } else {
            prev_nonzero
        };

        if effective == 0 {
            // Leading container (before any scalar): no advance, no IB bit
            prev_effective = Some(0);
            continue;
        }

        let is_new_position = prev_effective != Some(effective);

        if is_new_position {
            // Set IB bit at this text position
            ib_words[effective as usize / 64] |= 1u64 << (effective as usize % 64);
            // Set advance bit
            advance_words[i / 64] |= 1u64 << (i % 64);
        }

        prev_effective = Some(effective);
    }
    // ... build rank/select indices ...
}
```

This eliminates the O(N) temporary allocation (A1) and the separate O(N)
monotonicity scan (A2) that were previously needed before bitmap construction
(see [issue #72](https://github.com/rust-works/succinctly/issues/72)).

During the bitmap build, leading zeros (value 0, before any scalar) are
skipped — they get no advance bit. This ensures `get()` returns `None` for
leading containers (advance_count stays 0).

### Build walkthrough: `[0, 10, 0, 0, 20, 0, 30]`

**Step 1: Zero-fill**

```
Original:  [0, 10,  0,  0, 20,  0, 30]
Filled:    [0, 10, 10, 10, 20, 20, 30]
```

**Step 2: Build IB (mark unique positions in text)**

```
Position:  0  1  2  ...  9  10  11  ...  19  20  21  ...  29  30
IB:        0  0  0  ...  0   1   0  ...   0   1   0  ...   0   1
                         ↑               ↑               ↑
                     pos=10          pos=20          pos=30
```

3 unique positions → 3 set bits in IB.

**Step 3: Build advance bitmap (one bit per open_idx)**

Scan the filled array. Set bit when position changes from previous:

```
open_idx:   0      1      2      3      4      5      6
filled:     0     10     10     10     20     20     30
prev:      —       0     10     10     10     20     20
changed?   skip    YES    no     no    YES     no    YES
advance:   (skip)  1      0      0      1      0      1
```

Index 0 is skipped (leading zero). Result:

```
advance bits: [_, 1, 0, 0, 1, 0, 1]    (bit 0 unset, bits 1,4,6 set)
```

### Query: `get(open_idx) → Option<usize>`

```
get(open_idx):
  1. advance_bit = advance[open_idx]      ← single word access
  2. advance_count = cursor.adv_cumulative + advance_bit
  3. If advance_count == 0: return None   ← leading container
  4. position = IB_select1(advance_count - 1)
     Return Some(position)
```

Compared to Design 2, there is **no Step 1 `has_end` check** and **no Step 2
`has_end_rank`**. The advance bitmap is accessed directly at `open_idx`.

### Query walkthrough: looking up open_idx=4 (position 20)

```
cursor state: adv_cumulative = 1 (from previous calls: open_idx 1 set bit 1)
              actually after processing 0,1,2,3:
              idx 0: skip (advance_count stays 0 → None)
              idx 1: adv_bit=1, advance_count=0+1=1 → IB_select1(0)=10 ✓
              idx 2: adv_bit=0, advance_count=1+0=1 → IB_select1(0)=10 ✓
              idx 3: adv_bit=0, advance_count=1+0=1 → IB_select1(0)=10 ✓
              → adv_cumulative = 1

Step 1: advance[4] = 1                         ← 1 memory access
Step 2: advance_count = 1 + 1 = 2
Step 3: advance_count > 0, continue
Step 4: IB_select1(2 - 1) = IB_select1(1)      ← 0-1 memory accesses
                           = 20
Return Some(20)  ✓
```

### Query walkthrough: looking up leading container (open_idx=0)

```
Step 1: advance[0] = 0  (leading zero was skipped during build)
Step 2: advance_count = 0 + 0 = 0
Step 3: advance_count == 0 → return None  ✓
```

### Data structures

```rust
struct CompactEndPositions {
    // Bitmap 1: text-position markers
    ib_words: Vec<u64>,         // ⌈L/64⌉ words
    ib_select_samples: Vec<u32>,// sampled select index
    ib_ones: usize,             // unique positions

    // Bitmap 2: position-change markers (indexed by open_idx)
    advance_words: Vec<u64>,    // ⌈N/64⌉ words
    advance_rank: Vec<u32>,     // ⌈N/64⌉ + 1 entries
    num_opens: usize,           // N

    cursor: Cell<SequentialCursor>,
}
```

The cursor is now 6 fields (48 bytes), down from 7 (56 bytes):

```rust
struct SequentialCursor {
    next_open_idx: usize,    // expected next query
    adv_cumulative: usize,   // advance_rank1(next_open_idx)
    ib_word_idx: usize,      // IB scan position
    ib_ones_before: usize,   // ones in IB before scan position
    last_ib_arg: usize,      // duplicate detection
    last_ib_result: usize,   // cached result
}
```

The `he_cumulative` field is gone — there's no `has_end` to track.

### Memory formula

```
                    IB           advance         rank/select
Compact storage = ⌈L/64⌉×8  +  ⌈N/64⌉×8   +   overhead
                ≈   L/8      +    N/8        +   ~5%

vs. Direct Array = N × 4 bytes
```

For 1 MB YAML (N≈130K, L=1M):
```
Direct:   130K × 4              = 520 KB
2-Bitmap: 1M/8 + 130K/8 + overhead
        ≈ 125 KB + 16 KB + ~7 KB ≈ 148 KB

Compression ratio: 520 / 148 ≈ 3.5×
```

Compared to the 3-bitmap design (159 KB), the 2-bitmap design saves ~11 KB
by eliminating `has_end` (N/8 bytes) and its rank array (~N/64 × 4 bytes),
though the advance bitmap grows from S/8 to N/8.

### The dependency chain: why fewer bitmaps means faster queries

On a modern CPU, instructions execute out of order. The CPU can start many
operations simultaneously, but each operation must wait for its **data
dependencies** to resolve. A chain of dependent operations (A feeds B feeds
C) is called a **dependency chain** — the CPU cannot overlap them, so the
total latency is the sum of all links.

**3-bitmap dependency chain** (Design 2):

```
    ┌─────────────────┐
    │ load has_end[w] │ ←── memory load (~4 cycles if L1 hit)
    └────────┬────────┘
             │ bit test
    ┌────────▼────────┐
    │ scalar_idx =    │ ←── depends on has_end result
    │ he_cumulative   │
    └────────┬────────┘
             │
    ┌────────▼────────┐
    │ load advance[w] │ ←── memory load, depends on scalar_idx
    └────────┬────────┘
             │ bit test
    ┌────────▼────────┐
    │ advance_count = │ ←── depends on advance result
    │ adv_cumulative  │
    └────────┬────────┘
             │
    ┌────────▼────────┐
    │ IB select1(k)   │ ←── depends on advance_count
    └─────────────────┘

    Total chain: ~6-8 cycles (3 loads + bit ops)
```

**2-bitmap dependency chain** (Design 3):

```
    ┌─────────────────┐
    │ load advance[w] │ ←── memory load (~4 cycles if L1 hit)
    └────────┬────────┘
             │ bit test
    ┌────────▼────────┐
    │ advance_count = │ ←── depends on advance result
    │ adv_cumulative  │
    └────────┬────────┘
             │
    ┌────────▼────────┐
    │ IB select1(k)   │ ←── depends on advance_count
    └─────────────────┘

    Total chain: ~4-5 cycles (2 loads + bit ops)
```

The 2-bitmap design removes one load and one dependency link. On CPUs
with smaller **reorder buffers** (the hardware structure that tracks
in-flight instructions for out-of-order execution), this shorter chain is
more likely to be hidden by other work. The ARM Neoverse V2, with a ~224-entry
reorder buffer, benefits more from this reduction than the Apple M1 Max
with its ~630-entry buffer.

### Side effect: container behaviour changes

With zero-filling, containers between scalars now return `Some(prev_end)`
instead of `None`:

```
Original positions: [0, 10, 0, 20]
                         ↑     ↑
                     scalar  scalar

Design 2: get(2) = None       (has_end[2] = 0 → container)
Design 3: get(2) = Some(10)   (filled with prev value 10)
```

This is safe because the only production caller (`value()` in `light.rs`)
never calls `get()` for containers. An additional guard in `value()` filters
out stale values:

```rust
let end = self.text_end_position()
    .filter(|&e| e >= effective_text_pos)   // reject stale values
    .unwrap_or(effective_text_pos);
```

---

## Performance Results

All benchmarks on ARM Neoverse V2 (AWS Graviton 3, 64 KB L1D, ~224-entry ROB).

### Within-branch improvement (2-bitmap vs 3-bitmap)

This measures the isolated effect of eliminating `has_end`:

| Benchmark        | 3-Bitmap | 2-Bitmap | Change     |
|------------------|----------|----------|------------|
| simple_kv/10     | 1.86 µs  | 1.64 µs  | **-11.7%** |
| simple_kv/100    | 9.04 µs  | 8.45 µs  | **-6.5%**  |
| simple_kv/1000   | 76.8 µs  | 73.5 µs  | **-4.3%**  |
| sequences/10     | 2.26 µs  | 2.04 µs  | **-9.8%**  |
| sequences/100    | 7.37 µs  | 7.01 µs  | **-4.9%**  |
| quoted/double/10 | 2.80 µs  | 2.43 µs  | **-13.3%** |
| quoted/single/10 | 2.77 µs  | 2.37 µs  | **-14.3%** |
| large/1kb        | 6.75 µs  | 6.28 µs  | **-7.0%**  |
| large/1mb        | 4.39 ms  | 3.88 ms  | **-11.5%** |
| anchors/10       | 4.11 µs  | 3.73 µs  | **-9.1%**  |
| anchors/k8s_10   | 9.06 µs  | 8.50 µs  | **-6.2%**  |

The improvement is largest for small documents with many nodes (high
lookup-to-parse ratio), reaching **14.3%** for quoted/single/10.

### Branch vs main (compact encoding vs `Vec<u32>`)

This shows the total cost of compact encoding (both OpenPositions and
EndPositions bitmaps) compared to the original `Vec<u32>` arrays:

| Benchmark             | main     | 2-Bitmap branch | Change   | Notes                    |
|-----------------------|----------|-----------------|----------|--------------------------|
| simple_kv/10          | 1.41 µs  | 1.65 µs         | +17%     | Many short scalars       |
| simple_kv/1000        | 70.3 µs  | 73.5 µs         | +4.6%    | -                        |
| sequences/10          | 1.67 µs  | 2.04 µs         | +22%     | Highest node density     |
| quoted/double/10      | 1.96 µs  | 2.43 µs         | +24%     | -                        |
| large/1kb             | 5.41 µs  | 6.27 µs         | +16%     | -                        |
| large/1mb             | 3.79 ms  | 4.22 ms         | +11%     | -                        |
| long_strings/1024b    | 113 µs   | 85.2 µs         | **-25%** | Few lookups, long values |
| long_strings/4096b    | 469 µs   | 308 µs          | **-34%** | -                        |
| block_scalars/100x100 | 632 µs   | 530 µs          | **-16%** | -                        |
| block_scalars/10x1000 | 667 µs   | 534 µs          | **-20%** | -                        |

The branch is slower for workloads with many short scalars (high lookup
frequency), but faster for workloads with fewer, longer values (where
other optimizations on the branch dominate).

### Memory usage

| YAML size       | Design   | End pos storage | Ratio        |
|-----------------|----------|-----------------|--------------|
| 1 KB (N≈130)    | Vec<u32> | 520 B           | 1.0×         |
| -               | 3-Bitmap | ~155 B          | 3.4× smaller |
| -               | 2-Bitmap | ~145 B          | 3.6× smaller |
| 10 KB (N≈1.3K)  | Vec<u32> | 5.2 KB          | 1.0×         |
| -               | 3-Bitmap | ~1.5 KB         | 3.5× smaller |
| -               | 2-Bitmap | ~1.4 KB         | 3.7× smaller |
| 1 MB (N≈130K)   | Vec<u32> | 520 KB          | 1.0×         |
| -               | 3-Bitmap | ~159 KB         | 3.3× smaller |
| -               | 2-Bitmap | ~148 KB         | 3.5× smaller |

The 2-bitmap design saves an additional ~7% memory over 3-bitmap by
eliminating the `has_end` bitmap and its rank array.

### Build regression mitigation (A1 + A2 + A4, issue #72)

The compact 2-bitmap encoding introduced an 11-24% `yaml_bench` regression
compared to the `Vec<u32>` baseline, because bitmap construction is more
expensive than simple array allocation. Three optimisations (A1, A2, and A4)
reduced this regression significantly:

**A1: Inline zero-filling** — Eliminated the temporary `Vec<u32>` that was
allocated, filled, and immediately discarded during `EndPositions::build()`.
Zero-filling now happens inline during bitmap construction (see code above).
Saves one O(N) alloc+copy+dealloc per build (~520KB for 1MB YAML).

**A2: Combined monotonicity check** — Merged the monotonicity check into the
bitmap construction loop. Previously, `EndPositions::build()` made a separate
O(N) pass to verify non-zero positions were non-decreasing before calling
`CompactEndPositions::build()` for a second pass. Now `try_build()` checks
monotonicity inline and returns `None` on violation, triggering Dense fallback.

**A4: Lazy newline index** — Changed the `newlines` bitvector in `YamlIndex`
from eager construction to lazy initialisation via `core::cell::OnceCell`.
The newline index is only needed by `to_line_column()` and `to_offset()`
(used by `yq-locate` CLI), not by parsing or query evaluation. This removes
a full O(N) text scan from every `YamlIndex::build()` call.

**yaml_bench results after A1 + A4** (Apple M1 Max):

| Benchmark             | Change          | Notes                           |
|-----------------------|-----------------|---------------------------------|
| simple_kv             | **-11% to -22%**| Core key-value parsing          |
| nested                | **-15% to -24%**| Deeply nested structures        |
| sequences             | **-9% to -15%** | Array-like YAML                 |
| quoted strings        | **-20% to -37%**| Double/single quoted values     |
| long strings          | **-44% to -85%**| Largest gains (newline removed) |
| large files           | **-18% to -21%**| 100KB-1MB files                 |
| block scalars         | **-42% to -57%**| Literal/folded blocks           |
| anchors               | **-12% to -16%**| Anchor/alias workloads          |

The long string and block scalar categories show the largest improvements
because they contain the most newlines, so removing the eager newline scan
has the biggest impact.

---

## Key Lessons

### 1. Bitmap encoding trades CPU for memory

The `Vec<u32>` design uses one array load per query. Bitmap encoding replaces
this with 2–3 bitmap lookups involving rank, select, and bit tests. This is
inherently more work per query, but uses 3–5× less memory. For workloads where
the bottleneck is elsewhere (long strings, I/O), the memory savings dominate.
For workloads that hammer the lookup path (many tiny scalars), the CPU cost
is visible.

### 2. Remove what the hot path doesn't need

The `has_end` bitmap existed to distinguish containers from scalars. But the
only production caller already knows it's dealing with a scalar — it checked
for containers much earlier. Eliminating `has_end` saved one bitmap access
per query and 8 bytes per cursor copy, yielding a consistent 5–14% improvement.

### 3. Shorter dependency chains help narrow CPUs more

The Apple M1 Max (8-wide decode, 630-entry ROB, 3 load ports) barely noticed
the difference between 3-bitmap and 2-bitmap designs — its deep pipeline hid
the extra dependency. The ARM Neoverse V2 (smaller ROB, 2 load ports) showed
clear improvement from the shorter chain. Optimizations that reduce serial
dependencies have more impact on narrower microarchitectures.

### 4. Sequential access patterns unlock amortized O(1)

Both compact designs use a sequential cursor that turns expensive rank/select
operations into single-bit tests when nodes are accessed in order. Without this
cursor, the compact encoding would be impractically slow. The cursor relies on
the natural depth-first traversal order of YAML streaming — a property that
doesn't hold for random-access workloads.

### 5. Zero-filling is a valid encoding trick when the API allows it

By filling container zeros with the previous scalar's end position, we made the
advance bitmap directly indexable by `open_idx`, eliminating the `has_end`
indirection. This changes the return value for containers (from `None` to
`Some(prev_end)`), but the only caller doesn't query containers, so the
semantic change is invisible in production. When an API's actual usage pattern
is narrower than its interface suggests, the implementation can exploit that.

---

## Summary of the three designs

```
┌───────────────────┬────────────────────┬─────────────────────┬─────────────────────┐
│                   │ Design 1           │ Design 2            │ Design 3            │
│                   │ Direct Array       │ 3-Bitmap            │ 2-Bitmap            │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ Storage           │ Vec<u32>           │ has_end + IB +      │ IB + advance        │
│                   │                    │ advance             │                     │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ Memory (1MB YAML) │ 520 KB             │ ~159 KB             │ ~148 KB             │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ Compression ratio │ 1×                 │ 3.3×                │ 3.5×                │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ Accesses per get  │ 1 (array load)     │ 2–3 (bitmap ops)    │ 1–2 (bitmap ops)    │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ Dependency depth  │ ~1 (load)          │ ~6-8 cycles         │ ~4-5 cycles         │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ Cursor size       │ N/A                │ 56 bytes (7 fields) │ 48 bytes (6 fields) │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ Container result  │ None (0 → filter)  │ None (has_end = 0)  │ Some(prev_end)      │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ vs main (kv/10)   │ baseline           │ +28% slower         │ +17% slower         │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ vs main (1MB)     │ baseline           │ +23% slower         │ +11% slower         │
├───────────────────┼────────────────────┼─────────────────────┼─────────────────────┤
│ 3→2 bitmap gain   │ —                  │ baseline            │ 5–14% faster        │
└───────────────────┴────────────────────┴─────────────────────┴─────────────────────┘
```

**Source files**:
- Design 1: `bp_to_text_end: Vec<u32>` in [src/yaml/index.rs](../../src/yaml/index.rs)
- Designs 2 & 3: [src/yaml/end_positions.rs](../../src/yaml/end_positions.rs)
- Query site: `value()` in [src/yaml/light.rs](../../src/yaml/light.rs)
