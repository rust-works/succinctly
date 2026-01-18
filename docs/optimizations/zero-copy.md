# Zero-Copy and Allocation Optimisation

Memory allocation is expensive. Each allocation involves system calls, lock contention, and fragmentation management. Zero-copy techniques eliminate unnecessary data movement and allocation.

## Overview

| Technique             | Benefit                       | Use Case                   |
|-----------------------|-------------------------------|----------------------------|
| Borrowed references   | No allocation                 | Read-only access           |
| Type punning          | O(1) conversion               | Binary data formats        |
| Memory mapping        | OS-managed I/O                | Large files                |
| Buffer reuse          | Amortized allocation          | Repeated operations        |
| Streaming output      | Constant memory               | Large result sets          |

---

## Borrowed References

### The Problem

```rust
// Allocates new String for each call
fn get_name(data: &Data) -> String {
    data.name.clone()
}

// Called in loop = O(n) allocations
for item in items {
    let name = get_name(&item);  // Allocation!
    process(&name);
}
```

### The Solution

Return references when possible:

```rust
// Returns reference - no allocation
fn get_name(data: &Data) -> &str {
    &data.name
}

// Zero allocations in loop
for item in items {
    let name = get_name(&item);  // Just a pointer
    process(name);
}
```

### Generic Over Ownership

Use `Cow` or generics for flexibility:

```rust
use std::borrow::Cow;

// Works with both owned and borrowed data
fn process_data(data: Cow<str>) {
    // Can borrow if already a reference
    // Only allocates if modification needed
}

// Generic approach
fn process<S: AsRef<str>>(data: S) {
    let s: &str = data.as_ref();
}
```

---

## Type Punning (Cast Slices)

### Safe Reinterpretation

Convert between types with compatible memory layouts:

```rust
use bytemuck::{cast_slice, cast_slice_mut};

// Zero-copy conversion: &[u64] → &[u8]
fn words_to_bytes(words: &[u64]) -> &[u8] {
    cast_slice(words)
}

// Zero-copy conversion: &[u8] → &[u64]
fn bytes_to_words(bytes: &[u8]) -> &[u64] {
    cast_slice(bytes)
}
```

### Requirements

- Types must have compatible alignments
- No padding bytes
- Use `bytemuck` crate for safe transmutation

### Usage in Succinctly

```rust
// src/binary.rs
pub fn to_bytes(words: &[u64]) -> &[u8] {
    bytemuck::cast_slice(words)
}

pub fn from_bytes(bytes: &[u8]) -> &[u64] {
    bytemuck::cast_slice(bytes)
}
```

**Application**: Serialize/deserialize index structures without copying.

---

## Memory Mapping

### Principle

Let the OS manage file I/O through virtual memory:

```rust
use memmap2::MmapOptions;

fn mmap_file(path: &Path) -> io::Result<Mmap> {
    let file = File::open(path)?;
    unsafe { MmapOptions::new().map(&file) }
}

// Access file contents like a slice
let mmap = mmap_file("data.bin")?;
let data: &[u8] = &mmap[..];
```

### Benefits

1. **No explicit I/O**: OS pages in data on demand
2. **Shared memory**: Multiple processes can share same mapping
3. **Large files**: Don't need to fit entire file in RAM
4. **Lazy loading**: Only accessed pages are loaded

### Usage in Succinctly

```rust
// Generic over storage type
struct JsonIndex<W: AsRef<[u64]>> {
    words: W,  // Can be Vec<u64> or &[u64] from mmap
}

// Works with owned data
let index: JsonIndex<Vec<u64>> = JsonIndex::from_json(json)?;

// Or memory-mapped data
let mmap = mmap_file("index.bin")?;
let words: &[u64] = cast_slice(&mmap);
let index: JsonIndex<&[u64]> = JsonIndex::from_words(words);
```

---

## Buffer Reuse

### The Problem

```rust
// Allocates per iteration
for line in lines {
    let mut buffer = Vec::new();  // Allocation!
    process(line, &mut buffer);
    output(&buffer);
}
```

### The Solution

Reuse buffers across iterations:

```rust
let mut buffer = Vec::new();
for line in lines {
    buffer.clear();  // Keeps capacity
    process(line, &mut buffer);
    output(&buffer);
}
```

### With Capacity Hints

```rust
// Preallocate expected size
let mut buffer = Vec::with_capacity(expected_size);

// After clear(), capacity remains
buffer.clear();
assert!(buffer.capacity() >= expected_size);
```

---

## Streaming Output

### The Problem

Building large results in memory:

```rust
// Collects all results before returning
fn transform(data: &[Item]) -> Vec<Output> {
    data.iter()
        .map(process)
        .collect()  // Allocates Vec with all results
}
```

### The Solution

Return iterators or use callbacks:

```rust
// Returns iterator - lazy evaluation
fn transform(data: &[Item]) -> impl Iterator<Item = Output> + '_ {
    data.iter().map(process)
}

// Or use writer
fn transform_to<W: Write>(data: &[Item], mut writer: W) -> io::Result<()> {
    for item in data {
        let output = process(item);
        write!(writer, "{}", output)?;  // Stream directly
    }
    Ok(())
}
```

### Usage in Succinctly

```rust
// src/bin/main.rs - Streaming output
fn output_results<W: Write>(results: impl Iterator<Item = Value>, writer: &mut W) {
    for value in results {
        writeln!(writer, "{}", value).unwrap();
    }
}

// Uses BufWriter to reduce syscalls
let stdout = io::stdout();
let mut writer = BufWriter::new(stdout.lock());
output_results(results, &mut writer);
```

---

## String Handling

### Raw Bytes vs Strings

When UTF-8 validation isn't needed:

```rust
// Validates UTF-8 - may be unnecessary
fn get_field(data: &[u8]) -> &str {
    std::str::from_utf8(data).unwrap()
}

// Skip validation for known-valid data
fn get_field_raw(data: &[u8]) -> &[u8] {
    data  // No validation overhead
}

// Or use unchecked (when safety is guaranteed)
fn get_field_unchecked(data: &[u8]) -> &str {
    unsafe { std::str::from_utf8_unchecked(data) }
}
```

### Usage in Succinctly

```rust
// Access raw JSON bytes without string conversion
fn get_raw_value(&self) -> &[u8] {
    &self.input[self.start..self.end]
}

// Only convert to string when needed
fn get_string(&self) -> Result<&str, Utf8Error> {
    std::str::from_utf8(self.get_raw_value())
}
```

---

## Arena Allocation

### Principle

Allocate many objects from a single buffer, free all at once:

```rust
struct Arena {
    storage: Vec<u8>,
    offset: usize,
}

impl Arena {
    fn alloc<T>(&mut self) -> &mut T {
        let align = std::mem::align_of::<T>();
        let size = std::mem::size_of::<T>();

        // Align offset
        self.offset = (self.offset + align - 1) & !(align - 1);

        let ptr = &mut self.storage[self.offset] as *mut u8 as *mut T;
        self.offset += size;

        unsafe { &mut *ptr }
    }

    fn reset(&mut self) {
        self.offset = 0;  // "Free" everything
    }
}
```

### Benefits

1. **Fast allocation**: Just bump a pointer
2. **No fragmentation**: Contiguous memory
3. **Fast deallocation**: Reset offset to zero
4. **Cache-friendly**: Allocations are adjacent

---

## Lazy Evaluation

### Defer Work Until Needed

```rust
// Eager: Does all work upfront
struct EagerIndex {
    data: Vec<ComputedResult>,
}

// Lazy: Computes on demand
struct LazyIndex<'a> {
    source: &'a [u8],
    positions: Vec<usize>,  // Just positions, not results
}

impl<'a> LazyIndex<'a> {
    fn get(&self, idx: usize) -> ComputedResult {
        let pos = self.positions[idx];
        compute_result(&self.source[pos..])  // Compute on access
    }
}
```

### Cursor-Based Navigation

```rust
// src/json/light.rs - Lazy JSON cursor
struct JsonCursor<'a> {
    index: &'a JsonIndex,
    position: usize,  // Just an offset
}

impl<'a> JsonCursor<'a> {
    // Navigation doesn't decode values
    fn next_sibling(&self) -> JsonCursor<'a> { ... }
    fn first_child(&self) -> JsonCursor<'a> { ... }

    // Only decode when explicitly requested
    fn value(&self) -> JsonValue { ... }
}
```

---

## Usage in Succinctly

| Technique         | Location               | Purpose                    |
|-------------------|------------------------|----------------------------|
| Type punning      | `binary.rs`            | Index serialization        |
| Memory mapping    | `binary.rs`            | Large file handling        |
| Generic storage   | `json/light.rs`        | `Vec<u64>` or `&[u64]`     |
| Streaming output  | `bin/main.rs`          | CLI output                 |
| Raw bytes         | `json/light.rs`        | Skip UTF-8 validation      |
| Lazy cursors      | `json/light.rs`        | Navigate without decode    |
| Buffer reuse      | `json/bit_writer.rs`   | Avoid per-chunk allocation |

---

## Allocation Cost Reference

| Operation                  | Approximate Cost       |
|----------------------------|------------------------|
| Stack allocation           | ~0 cycles (compile-time) |
| Bump allocation (arena)    | ~1-5 cycles            |
| Small heap alloc (< 256B)  | ~50-100 cycles         |
| Large heap alloc (> 4KB)   | ~200-500 cycles        |
| System call (mmap)         | ~1000+ cycles          |
| Memory copy (per byte)     | ~0.1 cycles (cached)   |

---

## Key Lessons

1. **Question every allocation**: Can you borrow instead?
2. **Reuse buffers**: `clear()` keeps capacity
3. **Stream large results**: Don't materialize entire output
4. **Lazy is often better**: Defer work until actually needed
5. **Profile allocations**: Use `DHAT` or `heaptrack`

---

## References

- Rust Performance Book - Memory allocation chapter
- Google "Abseil Tips" on memory management
- Chandler Carruth "Efficiency with Algorithms, Performance with Data Structures" (CppCon)
- `bytemuck` crate documentation
- `memmap2` crate documentation
