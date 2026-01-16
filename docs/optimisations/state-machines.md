# State Machine Optimisation

Finite state machines (FSMs) are fundamental to parsing and pattern matching. Optimising state machine execution can yield significant performance improvements for text processing.

## Overview

| Technique            | Benefit                      | Overhead        | Use Case              |
|----------------------|------------------------------|-----------------|------------------------|
| Table-driven         | O(1) transitions             | Memory          | General parsing        |
| Packed states        | Cache efficiency             | Bit extraction  | Small state count      |
| Fast-path bypass     | Skip state machine           | Code complexity | Common patterns        |
| SIMD classification  | Parallel character analysis  | Setup cost      | Large inputs           |

---

## Table-Driven State Machines

### Basic Structure

Replace switch statements with table lookups:

```rust
// Switch-based (many branches)
fn transition_switch(state: u8, byte: u8) -> u8 {
    match (state, byte) {
        (0, b'"') => 1,
        (0, b'\\') => 0,
        (1, b'"') => 0,
        (1, b'\\') => 2,
        (2, _) => 1,
        _ => state,
    }
}

// Table-driven (single lookup)
const TRANSITION: [[u8; 256]; 3] = /* precomputed */;

fn transition_table(state: u8, byte: u8) -> u8 {
    TRANSITION[state as usize][byte as usize]
}
```

### Benefits

1. **Predictable performance**: Always one memory access
2. **No branch misprediction**: No conditional branches
3. **Compiler-friendly**: Simple array indexing

---

## Packed State Tables

For small state counts, pack multiple states into single table entry.

### PFSM (Parallel Finite State Machine)

Store transitions from all possible initial states:

```rust
// src/json/pfsm_tables.rs
// Each entry contains transitions for 4 states packed in 32 bits
const TRANSITION_TABLE: [u32; 256] = [
    // For byte 0x00:
    // state 0 → state X, state 1 → state Y, ...
    0x00010203,
    // ...
];

fn get_next_state(packed: u32, current_state: u8) -> u8 {
    ((packed >> (current_state * 8)) & 0xFF) as u8
}
```

### Output Generation

Similarly pack output bits:

```rust
const PHI_TABLE: [u32; 256] = /* precomputed outputs */;

fn get_output(byte: u8, state: u8) -> u8 {
    let packed = PHI_TABLE[byte as usize];
    ((packed >> (state * 8)) & 0xFF) as u8
}
```

**Result**: 33-77% faster than scalar JSON parsing.

---

## Two-Stage Pipeline

Separate character classification from state processing.

### Stage 1: Classification (SIMD)

Process many bytes in parallel to identify character types:

```rust
unsafe fn classify_chunk(data: &[u8; 32]) -> u32 {
    let chunk = _mm256_loadu_si256(data.as_ptr() as *const __m256i);

    // Identify structural characters
    let quotes = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'"' as i8));
    let braces = _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'{' as i8));
    // ... more character types ...

    let structural = _mm256_or_si256(quotes, braces);
    _mm256_movemask_epi8(structural) as u32
}
```

### Stage 2: State Processing (Sequential)

Process classified characters through state machine:

```rust
fn process_structural(
    classification: u32,
    state: &mut u8,
    output: &mut BitWriter,
) {
    for i in 0..32 {
        if classification & (1 << i) != 0 {
            let (new_state, bits) = transition(*state, i);
            *state = new_state;
            output.write_bits(bits);
        }
    }
}
```

**Architecture**:
```
Input bytes → [SIMD Classification] → Structural mask
                                           ↓
                              [Sequential State Machine]
                                           ↓
                                    Output bits
```

---

## Fast-Path String Scanning

When most input is a common pattern (like string contents), bypass the state machine.

### The Insight

In JSON, ~80% of bytes are inside strings. Most need no state processing.

### Implementation

```rust
fn scan_string_fast(data: &[u8], mut pos: usize) -> usize {
    // Fast path: Scan for quote or backslash
    while pos < data.len() {
        match data[pos] {
            b'"' => return pos,      // End of string
            b'\\' => {
                pos += 2;            // Skip escape sequence
            }
            _ => pos += 1,           // Regular character
        }
    }
    pos
}

// In state machine:
if state == IN_STRING {
    let end = scan_string_fast(data, pos);
    // Skip directly to end, no per-byte state updates
    pos = end;
}
```

**Result**: 2.5-8x speedup on string-heavy JSON.

---

## Batch Zero Writing

When state machine produces runs of zeros, write them in bulk.

### The Pattern

```rust
// src/json/bit_writer.rs
impl BitWriter {
    fn write_zeros(&mut self, count: usize) {
        // Handle partial word
        let bits_in_current = 64 - self.bit_offset;
        if count <= bits_in_current {
            self.bit_offset += count;
            return;
        }

        // Finish current word
        self.flush_word();

        // Write full zero words
        let remaining = count - bits_in_current;
        let full_words = remaining / 64;
        for _ in 0..full_words {
            self.words.push(0);
        }

        // Handle leftover bits
        self.bit_offset = remaining % 64;
    }
}
```

---

## State Machine Minimisation

### Reduce State Count

Fewer states = smaller tables = better cache:

```rust
// Before: 8 states for detailed tracking
enum DetailedState {
    Start, InObject, InArray, InString, InNumber, ...
}

// After: 3 states (minimum needed for JSON structure)
enum MinimalState {
    Outside,    // Not in string
    InString,   // Inside string
    Escape,     // After backslash in string
}
```

### Combine Equivalent States

States with identical transitions can be merged:

```rust
// If state A and B have same transitions for all inputs:
// transition[A][*] == transition[B][*]
// They can be merged into single state
```

---

## SIMD State Machine Considerations

### Why Direct SIMD FSM Is Hard

State machines have inherent serial dependency:

```
state[i+1] = f(state[i], input[i])
```

SIMD processes lanes independently - can't easily propagate state.

### Speculative Parallel Execution

Compute results for all possible initial states:

```rust
// For 4-state FSM, compute all 4 possible outcomes
fn speculative_process(byte: u8) -> [u8; 4] {
    [
        transition(0, byte),
        transition(1, byte),
        transition(2, byte),
        transition(3, byte),
    ]
}
```

Then chain results: select appropriate outcome based on previous state.

### Why It Often Fails

**NEON PFSM shuffle composition was 47% slower**:
- Shuffle to compose transitions is expensive
- Still need sequential chain through chunks
- Data dependency can't be fully eliminated

---

## Usage in Succinctly

| Technique            | Location               | Purpose               | Result        |
|----------------------|------------------------|-----------------------|---------------|
| PFSM tables          | `json/pfsm_tables.rs`  | Packed transitions    | 33-77% faster |
| Two-stage pipeline   | `json/pfsm_*.rs`       | SIMD + sequential     | Enables SIMD  |
| Fast-path strings    | `json/simd/*.rs`       | Skip string contents  | 2.5-8x faster |
| Batch zero writing   | `json/bit_writer.rs`   | Bulk bit operations   | Reduces ops   |
| 3-state FSM          | `json/pfsm_*.rs`       | Minimal state count   | Cache-friendly|

### Successful Optimizations

| Technique                    | Result     | Reason                                    |
|------------------------------|------------|-------------------------------------------|
| Lazy line number computation | **+2-6%**  | Avoid per-byte work only needed on errors |
| Direct indexing in hot loops | **+2-5%**  | Eliminate Option overhead in tight loops  |

#### Lazy Line Number Computation (YAML)

Line numbers are only needed for error reporting, but were tracked on every byte:

```rust
// Before: track line on every byte (slow)
fn advance(&mut self) {
    if self.pos < self.input.len() {
        if self.input[self.pos] == b'\n' {
            self.line += 1;  // Paid on EVERY byte
        }
        self.pos += 1;
    }
}

// After: compute on-demand only for errors (fast)
fn advance(&mut self) {
    if self.pos < self.input.len() {
        self.pos += 1;
    }
}

fn current_line(&self) -> usize {
    // Only called on error paths
    self.input[..self.pos].iter().filter(|&&b| b == b'\n').count() + 1
}
```

**Result**: 2-6% faster across YAML workloads.

#### Direct Indexing vs Option Pattern

The peek/advance pattern has hidden overhead:

```rust
// Before: Option wrapping overhead
fn skip_to_eol(&mut self) {
    while let Some(b) = self.peek() {  // Option created each iteration
        if b == b'\n' { break; }
        self.advance();  // Function call + bounds check
    }
}

// After: direct indexing
fn skip_to_eol(&mut self) {
    while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
        self.pos += 1;
    }
}
```

**Result**: Sequences improved 4.7-5.4%, strings improved 2.5-6.2%.

### Failed Optimizations

| Technique              | Result  | Reason                            |
|------------------------|---------|-----------------------------------|
| NEON PFSM shuffle      | -47%    | Shuffle overhead exceeds benefit  |
| AVX-512 FSM            | -10%    | Memory-bound, not compute-bound   |
| BMI1 mask iteration    | -26%    | FSM needs all bytes, not just structural |

---

## Design Guidelines

### When to Use Table-Driven

- Many states (> 4)
- Many input classes (> 8)
- Complex transition logic

### When to Use Switch/Match

- Few states (2-3)
- Simple, predictable patterns
- Hot path where branch prediction works

### Hybrid Approach

Combine techniques for best results:

```rust
fn process_byte(byte: u8, state: u8) -> (u8, u8) {
    // Fast path for common case
    if state == 0 && byte != b'"' && byte != b'\\' {
        return (0, 0);  // Stay in state 0, no output
    }

    // Table lookup for complex cases
    let packed = TRANSITION_TABLE[byte as usize];
    let new_state = ((packed >> (state * 8)) & 0xFF) as u8;
    let output = ((PHI_TABLE[byte as usize] >> (state * 8)) & 0xFF) as u8;
    (new_state, output)
}
```

---

## Key Lessons

1. **Tables beat switches**: Predictable memory access beats branch prediction
2. **Pack small states**: 4 states × 8 bits fits in 32-bit lookup
3. **Fast-path common cases**: Most JSON bytes are string content
4. **SIMD for classification, not FSM**: Use SIMD to find interesting bytes
5. **State dependency is fundamental**: Can't fully parallelize FSM
6. **Defer error-only work**: Track line numbers on-demand, not per-byte
7. **Direct indexing beats Option**: `while pos < len { arr[pos] }` beats `while let Some(b) = peek()`

---

## References

- Aho, A., Lam, M., Sethi, R., Ullman, J. "Compilers: Principles, Techniques, and Tools"
- Mytkowicz, T. et al. "Data-Parallel Finite-State Machines" (2014)
- Langdale, G. & Lemire, D. "Parsing Gigabytes of JSON per Second" (2019)
- Hopcroft, J. "An n log n Algorithm for Minimizing States in a Finite Automaton" (1971)
- Cameron, R. "Parallel Scanning with Bitstream Addition" (2012)
