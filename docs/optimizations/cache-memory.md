# Cache and Memory Optimisation

Modern CPUs can process data far faster than memory can deliver it. Understanding the memory hierarchy is essential for high-performance code.

## Overview

| Level     | Size       | Latency      | Bandwidth     |
|-----------|------------|--------------|---------------|
| L1 Cache  | 32-64 KB   | ~4 cycles    | ~1 TB/s       |
| L2 Cache  | 256-512 KB | ~12 cycles   | ~500 GB/s     |
| L3 Cache  | 8-64 MB    | ~40 cycles   | ~200 GB/s     |
| RAM       | GBs        | ~100 cycles  | ~50-100 GB/s  |

**Key insight**: A cache miss can cost 100x more than a cache hit.

---

## Cache Line Alignment

### The Cache Line

CPUs transfer memory in fixed-size blocks called cache lines (typically 64 bytes).

**Problem**: Data spanning two cache lines causes two memory accesses:

```
Cache line 1:  [.......XXXX]  <- 4 bytes
Cache line 2:  [XXXX.......]  <- 4 bytes

// Reading one 8-byte struct crosses boundary = 2 cache misses
```

### Alignment in Rust

```rust
// Force 64-byte alignment
#[repr(C, align(64))]
struct CacheAligned {
    data: [u64; 8],  // Exactly one cache line
}

// Or use explicit layout
use std::alloc::{Layout, alloc, dealloc};

fn allocate_cache_aligned(count: usize) -> *mut u64 {
    let layout = Layout::from_size_align(
        count * std::mem::size_of::<u64>(),
        64  // Cache line size
    ).unwrap();
    unsafe { alloc(layout) as *mut u64 }
}
```

### Usage in Succinctly

```rust
// src/bits/rank.rs - Cache-aligned rank directory
const CACHE_LINE_SIZE: usize = 64;

struct CacheAlignedL1L2 {
    ptr: *mut u128,
    len: usize,
}

impl CacheAlignedL1L2 {
    fn new(len: usize) -> Self {
        let layout = Layout::from_size_align(
            len * 16,  // 16 bytes per entry
            CACHE_LINE_SIZE
        ).unwrap();
        // ...
    }
}
```

**Result**: 3-4% improvement on rank queries by ensuring L1+L2 entries don't cross cache lines.

---

## Data Layout

### Structure of Arrays (SoA) vs Array of Structures (AoS)

**AoS** (poor cache utilization when accessing single field):
```rust
struct Point { x: f64, y: f64, z: f64 }
let points: Vec<Point> = ...;

// Accessing all x values loads y and z too
for p in &points {
    process(p.x);  // Loads 24 bytes, uses 8
}
```

**SoA** (better for single-field access):
```rust
struct Points {
    x: Vec<f64>,
    y: Vec<f64>,
    z: Vec<f64>,
}

// Sequential access to x values only
for x in &points.x {
    process(*x);  // Loads 64 bytes, uses 64
}
```

### Packed Data Structures

Minimize memory footprint to improve cache utilization:

```rust
// src/bits/rank.rs - Pack 7 × 9-bit values into 128 bits
// Instead of 7 × 16-bit = 112 bits wasted
fn pack_l2_offsets(offsets: &[u16; 7]) -> u128 {
    let mut entry: u128 = 0;
    for (i, &offset) in offsets.iter().enumerate() {
        entry |= (offset as u128) << (32 + i * 9);
    }
    entry
}
```

**Layout**:
```
|  L1 (32 bits)  | L2[0] (9) | L2[1] (9) | ... | L2[6] (9) | padding |
```

---

## Memory Access Patterns

### Sequential Access

Best case - hardware prefetcher predicts and loads ahead:

```rust
// Excellent: Sequential iteration
let sum: u64 = data.iter().sum();

// Hardware prefetcher loads next cache lines automatically
```

**Bandwidth achieved**: Near theoretical maximum (50+ GB/s on modern systems).

### Strided Access

Regular but non-sequential - prefetcher can often handle:

```rust
// Good: Regular stride
for i in (0..len).step_by(8) {
    process(data[i]);
}
```

**Note**: Strides that are powers of 2 can cause cache set conflicts.

### Random Access

Worst case - each access likely misses cache:

```rust
// Poor: Random access
for &idx in random_indices {
    process(data[idx]);  // Cache miss likely
}
```

**Mitigation strategies**:
1. Sort indices before access
2. Batch operations to amortize misses
3. Use smaller data structures that fit in cache

### Usage in Succinctly

Dual select methods optimize for access pattern:

```rust
// src/bits/select.rs

// For random access: Binary search
fn ib_select1(&self, k: u64) -> Option<u64> {
    // O(log n) but each step may miss cache
    binary_search_select(k)
}

// For sequential access: Exponential search from hint
fn ib_select1_from(&self, k: u64, hint: usize) -> Option<u64> {
    // O(log d) where d = distance from hint
    // Sequential locality means cache hits
    exponential_search_from(k, hint)
}
```

**Result**: 3.1x faster sequential iteration, 1.39x faster random access.

---

## Prefetching

### Hardware Prefetching

Modern CPUs automatically prefetch for predictable patterns:

- **Streaming**: Sequential forward/backward access
- **Strided**: Regular power-of-2 strides
- **Next-line**: Always fetches adjacent cache line

**Key insight**: Hardware prefetcher is usually sufficient for sequential workloads.

### Software Prefetching

Hint to CPU to load data before it's needed:

```rust
use core::arch::x86_64::_mm_prefetch;

// Prefetch data that will be needed soon
unsafe {
    _mm_prefetch(ptr.add(64) as *const i8, _MM_HINT_T0);
}
```

**When useful**:
- Irregular access patterns prefetcher can't predict
- Large jumps between accesses
- Latency-critical code with known access order

**When NOT useful** (failed in succinctly):
- NEON prefetching for JSON parsing: 0% improvement
- Hardware prefetcher already handles sequential patterns

---

## Memory Bandwidth

### Bandwidth vs Latency

| Optimization Target | Technique                    |
|---------------------|------------------------------|
| Latency-bound       | Reduce cache misses          |
| Bandwidth-bound     | Process more data per access |

### Bandwidth Saturation

When bandwidth is the bottleneck, wider SIMD doesn't help:

```
AVX2 (32 bytes):  Process 32 bytes, wait for memory
AVX-512 (64 bytes): Process 64 bytes... still waiting for same memory
```

**This is why AVX-512 JSON parsing was 7-17% slower** - the workload was memory-bound.

### Reducing Memory Traffic

1. **Compute over load**: Calculate values instead of storing/loading them
2. **Compression**: Store data in fewer bits
3. **Lazy evaluation**: Only access data that's actually needed

---

## Cache-Oblivious Algorithms

Design algorithms that work well regardless of cache size.

### Principle

Recursively divide problem until subproblems fit in cache:

```rust
fn cache_oblivious_sum(data: &[u64]) -> u64 {
    if data.len() <= THRESHOLD {
        // Base case: fits in L1 cache
        data.iter().sum()
    } else {
        let mid = data.len() / 2;
        cache_oblivious_sum(&data[..mid]) + cache_oblivious_sum(&data[mid..])
    }
}
```

### B-tree vs Binary Tree

B-trees store multiple keys per node, improving cache utilization:

```
Binary tree: 1 key per node = 1 cache line per comparison
B-tree:      B keys per node = B comparisons per cache line
```

---

## Memory Allocation

### Allocation Cost

`malloc`/`free` have non-trivial overhead:
- System call potential
- Lock contention
- Fragmentation management

### Strategies

1. **Preallocate**: Reserve capacity upfront
   ```rust
   let mut vec = Vec::with_capacity(expected_size);
   ```

2. **Reuse buffers**: Don't allocate in hot loops
   ```rust
   let mut buffer = vec![0u8; CHUNK_SIZE];
   for chunk in data.chunks(CHUNK_SIZE) {
       buffer[..chunk.len()].copy_from_slice(chunk);
       process(&buffer);
   }
   ```

3. **Arena allocation**: Bulk allocate, bulk free
   ```rust
   struct Arena {
       storage: Vec<u8>,
       offset: usize,
   }

   impl Arena {
       fn alloc(&mut self, size: usize) -> &mut [u8] {
           let ptr = self.offset;
           self.offset += size;
           &mut self.storage[ptr..self.offset]
       }

       fn reset(&mut self) {
           self.offset = 0;  // "Free" everything at once
       }
   }
   ```

---

## False Sharing

When multiple threads access different data on the same cache line, the cache line bounces between cores.

### Problem

```rust
struct Counters {
    thread0_count: u64,  // Both in same cache line!
    thread1_count: u64,
}

// Thread 0 and Thread 1 constantly invalidate each other's cache
```

### Solution

Pad to cache line boundaries:

```rust
#[repr(C, align(64))]
struct PaddedCounter {
    count: u64,
    _padding: [u8; 56],
}

struct Counters {
    thread0: PaddedCounter,  // Own cache line
    thread1: PaddedCounter,  // Own cache line
}
```

---

## Usage in Succinctly

| Technique              | Location           | Purpose                    | Impact       |
|------------------------|--------------------|----------------------------|--------------|
| Cache-aligned L1L2     | `bits/rank.rs`     | Rank directory             | 3-4% faster  |
| Packed u128 entries    | `bits/rank.rs`     | Reduce memory footprint    | 3% overhead  |
| Dual select methods    | `bits/select.rs`   | Adapt to access pattern    | 3.1x seq     |
| Lightweight index      | `dsv/index_*.rs`   | Simpler = cache-friendly   | 5-9x faster  |
| Zero-copy iteration    | `json/light.rs`    | Avoid allocations          | Varies       |

**Key insight from DSV optimization**: The "heavyweight" BitVec with 3-level RankDirectory was 5-9x slower than a simple cumulative array despite being theoretically optimal. Cache behavior matters more than asymptotic complexity for practical sizes.

---

## Key Lessons

1. **Cache misses dominate**: A single cache miss costs ~100 cycles
2. **Alignment matters**: Cross-boundary access doubles memory traffic
3. **Sequential wins**: Hardware prefetcher handles sequential access well
4. **Simpler is often faster**: Complex data structures have cache overhead
5. **Measure, don't assume**: Theoretical efficiency ≠ practical performance

---

## References

- Drepper, U. "What Every Programmer Should Know About Memory" (2007)
- Fog, A. "Optimizing Software in C++" - Memory access chapter
- Hennessy & Patterson "Computer Architecture: A Quantitative Approach"
- Intel "64 and IA-32 Architectures Optimization Reference Manual"
- Lemire, D. "Memory Bandwidth" blog posts
