# Access Pattern Optimisation

[Home](../../) > [Docs](../) > [Optimizations](./) > Access Patterns

The pattern of memory access significantly impacts performance. Sequential access can achieve 50+ GB/s while random access may only achieve 1-2 GB/s due to cache misses.

## Overview

| Access Pattern | Cache Behavior      | Typical Bandwidth | Example               |
|----------------|---------------------|-------------------|-----------------------|
| Sequential     | Prefetcher handles  | 50-100 GB/s       | Array iteration       |
| Strided        | Often predictable   | 10-50 GB/s        | Matrix column access  |
| Random         | Cache misses        | 1-5 GB/s          | Hash table lookup     |
| Temporal       | Hot data in cache   | Very high         | Working set access    |

---

## Sequential Access

### Why It's Fast

1. **Hardware prefetcher**: Automatically loads next cache lines
2. **Cache line utilization**: Use all 64 bytes per fetch
3. **Memory pipeline**: Overlapped loads hide latency

```rust
// Optimal: Sequential iteration
let sum: u64 = data.iter().sum();

// Cache line: [a, b, c, d, e, f, g, h]
//              ^  read all 8 values, one cache miss
```

### Maintaining Sequential Access

```rust
// Good: Process in memory order
for i in 0..len {
    process(data[i]);
}

// Bad: Random order destroys prefetching
for &idx in random_indices {
    process(data[idx]);
}
```

---

## Strided Access

Regular but non-unit stride.

### Stride Patterns

```rust
// Stride of 8: Every 8th element
for i in (0..len).step_by(8) {
    process(data[i]);
}

// Matrix column access (row-major storage)
for row in 0..rows {
    process(matrix[row * cols + col]);  // Stride = cols
}
```

### Cache Set Conflicts

Power-of-2 strides can cause set conflicts:

```
Cache with 8 sets, 64-byte lines:
Address 0    → Set 0
Address 512  → Set 0  (same set!)
Address 1024 → Set 0  (conflict!)
```

**Mitigation**: Add small offset or use non-power-of-2 stride.

---

## Random Access

### The Problem

Each access likely misses cache:

```rust
// ~100 cycles per access (memory latency)
for &idx in random_indices {
    process(data[idx]);
}

// vs ~4 cycles per access (L1 cache hit)
for i in 0..len {
    process(data[i]);
}
```

### Mitigation Strategies

#### 1. Sort Indices

Convert random to sequential:

```rust
let mut sorted_indices = indices.to_vec();
sorted_indices.sort_unstable();

// Now access in sorted order
for &idx in &sorted_indices {
    process(data[idx]);
}
```

#### 2. Batch and Sort

Process in batches with sorted access:

```rust
fn process_batch(data: &[u64], indices: &[usize]) {
    let mut batch: Vec<_> = indices.iter()
        .map(|&i| (i, data[i]))
        .collect();
    batch.sort_by_key(|(i, _)| *i);

    for (idx, value) in batch {
        output(idx, value);
    }
}
```

#### 3. Use Smaller Data Structures

Fit working set in cache:

```rust
// Bad: Large sparse structure
struct Sparse {
    data: Vec<u64>,      // 1GB
    indices: Vec<usize>, // Random access into data
}

// Better: Compact representation
struct Compact {
    values: Vec<(usize, u64)>, // Sorted by index
}
```

---

## Search Algorithm Selection

Different algorithms suit different access patterns.

### Binary Search (Random Access)

O(log n) comparisons, but each may miss cache:

```rust
fn binary_search(data: &[u64], target: u64) -> Option<usize> {
    let mut lo = 0;
    let mut hi = data.len();
    while lo < hi {
        let mid = (lo + hi) / 2;  // Random-ish access
        match data[mid].cmp(&target) {
            Less => lo = mid + 1,
            Greater => hi = mid,
            Equal => return Some(mid),
        }
    }
    None
}
```

**Good for**: One-off queries, random access patterns.

### Exponential Search (Sequential Access)

O(log d) where d = distance from hint:

```rust
fn exponential_search(data: &[u64], target: u64, hint: usize) -> Option<usize> {
    // Phase 1: Exponential jumps to find range
    let mut bound = 1;
    while hint + bound < data.len() && data[hint + bound] < target {
        bound *= 2;
    }

    // Phase 2: Binary search within range
    let lo = hint + bound / 2;
    let hi = std::cmp::min(hint + bound, data.len());
    binary_search(&data[lo..hi], target).map(|i| i + lo)
}
```

**Good for**: Sequential iteration where next query is near previous.

### Usage in Succinctly

Dual select methods optimize for access pattern:

```rust
// src/bits/select.rs

// For random access: Binary search from start
pub fn ib_select1(&self, k: u64) -> Option<u64> {
    // O(log n) binary search
    self.binary_search_select(k)
}

// For sequential access: Exponential search from hint
pub fn ib_select1_from(&self, k: u64, hint: usize) -> Option<u64> {
    // O(log d) where d = distance from hint
    self.exponential_search_from(k, hint)
}
```

**Results**:
- Sequential: 3.1x faster (335 µs → 102 µs)
- Random: 1.39x faster (due to better initial guess)

**The hint is only as good as its source** (#2168). `JsonCursor::text_position`
originally seeded `ib_select1_from` with the fixed estimate `rank / 8`, which
assumes eight interest bits per 64-byte word. A node-dense array of short
numbers has ~12, a document of long strings far fewer, so on real input the
estimate drifted by up to hundreds of words and every lookup in a document-order
walk paid the full O(log d) gallop -- [select-scan.md](select-scan.md) had already
measured it at ~17 probes per call on the real-workload corpus, and a profile put
it at over half the cost of the #1755/#1953 validity walk.

`JsonIndex::ib_select1_sequential` now seeds from the previous answer,
extrapolated at the document's own mean density in either direction
(`last_word + (rank - last_rank) * words / ones`, a `Cell` on the index in the
shape of YAML's `AdvancePositions` cursor), and answers an exact repeat of the
last rank from the cache without probing. The seed is exact by construction:
gallop-then-bisect is correct for any seed, so a stale one costs probes, never
an answer. Cost is `1 + ~2 log2(gap)` probes where `gap` is the seed's distance
from the true word -- 3 for any document of short scalars in document order,
~20 for a document of 64 KB strings (consecutive bits ~1000 words apart, against
~33 for the fixed estimate there), and ~16 for random access on a file whose
density drifts along its length (the `users` generator's records grow in digit
count), against ~25 before. Measured probes per lookup on a 4 MB `users`
document: document-order walk 3.00 (mean and max), `.users[] | .name` 3.12 mean
/ 5 max (a draft that only reused the seed going forward left this shape's two
backward asks per element at 23 probes each).

End to end, Apple M4 Pro, interleaved A/B with output identity gated, medians of
7 (PR #2578): `.[][0] | key` on a 14 MB `users` document 131.6 -> 84.6 ms
(-36%), the same on a 20 MB `numbers` array 151.8 -> 84.5 ms (-44%), identity
`.` on the 14 MB file 186.0 -> 144.2 ms (-22%), `[.[][] | key] | length` 167.7
-> 117.3 ms (-30%); the navigation-only control `keys_unsorted[0]` moved -1.3%,
inside the harness's own noise floor.

---

## Temporal Locality

Access same data multiple times while it's still cached.

### Loop Tiling/Blocking

Process data in cache-sized chunks:

```rust
const TILE_SIZE: usize = 4096;  // Fits in L1 cache

fn process_tiled(data: &mut [u64]) {
    for tile_start in (0..data.len()).step_by(TILE_SIZE) {
        let tile_end = std::cmp::min(tile_start + TILE_SIZE, data.len());
        let tile = &mut data[tile_start..tile_end];

        // Multiple passes over same tile (stays in cache)
        pass1(tile);
        pass2(tile);
        pass3(tile);
    }
}
```

### Working Set Management

Keep frequently accessed data together:

```rust
struct HotData {
    // Frequently accessed together - same cache line
    count: u64,
    sum: u64,
    min: u64,
    max: u64,
}

struct ColdData {
    // Rarely accessed - separate allocation
    history: Vec<u64>,
    metadata: String,
}
```

---

## Data Layout for Access Patterns

### Structure of Arrays (SoA)

When accessing single field across many records:

```rust
// Array of Structures (AoS) - bad for single-field access
struct Point { x: f64, y: f64, z: f64 }
let points: Vec<Point> = ...;

// Structure of Arrays (SoA) - good for single-field access
struct Points {
    x: Vec<f64>,
    y: Vec<f64>,
    z: Vec<f64>,
}

// Accessing all x values: sequential!
for x in &points.x {
    process(*x);
}
```

### Hybrid Layouts

Group frequently co-accessed fields:

```rust
// Hot fields together
struct ParticleHot {
    position: [f32; 3],
    velocity: [f32; 3],
}

// Cold fields separate
struct ParticleCold {
    name: String,
    creation_time: u64,
}

struct Particles {
    hot: Vec<ParticleHot>,   // Tight loop accesses these
    cold: Vec<ParticleCold>, // Rarely accessed
}
```

---

## Iteration Patterns

### Forward vs Backward

Both are sequential, but forward works better with prefetcher:

```rust
// Good: Forward iteration
for i in 0..len {
    process(data[i]);
}

// Usually fine: Backward iteration
for i in (0..len).rev() {
    process(data[i]);
}
```

### Chunked Iteration

Process multiple elements per loop iteration:

```rust
// Process 8 elements per iteration
for chunk in data.chunks(8) {
    for &x in chunk {
        process(x);
    }
}

// Even better with SIMD
for chunk in data.chunks_exact(8) {
    process_simd(chunk);
}
```

---

## Usage in Succinctly

| Technique            | Location              | Pattern          | Impact       |
|----------------------|-----------------------|------------------|--------------|
| Exponential search   | `bits/select.rs`      | Sequential       | 3.1x faster  |
| Binary search        | `bits/select.rs`      | Random           | 1.39x faster |
| Chunked processing   | `json/simd/*.rs`      | 32/64 byte chunks| Better cache |
| Cumulative index     | `json/light.rs`       | Sequential scan  | 627x faster  |
| Lightweight index    | `dsv/index_*.rs`      | Sequential iter  | 5-9x faster  |

**Key insight**: DSV lightweight index (simple cumulative array) beat the "optimal" 3-level BitVec structure by 5-9x because simpler data layout has better cache behavior for sequential iteration.

---

## Key Lessons

1. **Sequential is king**: Design for sequential access when possible
2. **Know your pattern**: Choose algorithm based on access pattern
3. **Provide hints**: Let callers specify access pattern (hint parameter)
4. **Profile real workloads**: Synthetic benchmarks may miss cache effects
5. **Simpler can win**: Complex structures have overhead

---

## See Also

- [Cache & Memory](cache-memory.md) — cache line behaviour, alignment, prefetching
- [Zero-Copy Techniques](zero-copy.md) — avoiding unnecessary memory copies

## References

- Drepper, U. "What Every Programmer Should Know About Memory" (2007)
- Fog, A. "Optimizing Software in C++" - Memory chapter
- Hennessy & Patterson "Computer Architecture: A Quantitative Approach"
- Brodal, G. S. & Fagerberg, R. "Cache-Oblivious Algorithms" (2003)
