# ARM/AArch64: high-performance rank/select (NEON/SVE) implementation plan

## Motivation

`succinctly` aims to provide practical succinct data structures. Rank/select over bitvectors is a core primitive, and ARM/AArch64 performance matters a lot (Apple Silicon, AWS Graviton, Ampere, etc.). We should implement rank/select with layout + primitives that map well to ARM (NEON/SVE) and are cache-friendly.

This document is written to be pasted directly into a GitHub issue for: https://github.com/rust-works/succinctly

---

## Background / references

- Vigna, “Broadword Implementation of Rank/Select Queries” (rank9/select9; broadword/SWAR primitives):  
  https://vigna.di.unimi.it/ftp/papers/Broadword.pdf  
  (also: ACM DOI page https://dl.acm.org/doi/10.5555/1788888.1788900)

- Zhou et al., “Space-Efficient, High-Performance Rank & Select Structures on Uncompressed Bit Sequences” (Poppy/cs-poppy):  
  https://www.cs.cmu.edu/~dongz/papers/poppy.pdf

- Laws et al., “SPIDER: Improved Succinct Rank and Select Performance” (metadata interleaving; select predictions):  
  https://arxiv.org/pdf/2405.05214  
  https://arxiv.org/html/2405.05214v1

- Arm NEON intrinsics reference (CNT/ADDV mappings):  
  https://arm-software.github.io/acle/neon_intrinsics/advsimd.html

- Rust AArch64 intrinsics docs (examples):  
  `vld1q_u8`: https://doc.rust-lang.org/core/arch/aarch64/fn.vld1q_u8.html  
  `vcntq_u8`: https://doc.rust-lang.org/core/arch/aarch64/fn.vcntq_u8.html  
  `vaddvq_u8`: https://doc.rust-lang.org/beta/core/arch/aarch64/fn.vaddvq_u8.html

---

## Proposed scope

### 1) Implement a rank/select bitvector using a proven layout (start with rank9)

Start with a two-level directory approach inspired by rank9:

- **Superblocks** store rank every large chunk (e.g. every 4096 bits).
- **Subblocks** store rank offsets within each superblock (e.g. every 512 bits).
- The underlying bitvector is stored as `u64` words.

Goals:

- Keep directory compact enough that each query typically touches ~1–2 cache lines.
- Choose a block size that maps to cache lines nicely (512-bit blocks = 64 bytes).

### 2) ARM-optimized popcount for within-block rank (NEON)

On AArch64 NEON, use:

- `vcntq_u8` (CNT): per-byte popcount
- `vaddvq_u8` (ADDV): horizontal sum of bytes
- `vld1q_u8` for 16-byte loads

**Primitive:** popcount of a 64-byte (512-bit) block. This is the hot inner op for rank and for select scanning.

Rust shape:

- `#[cfg(target_arch = "aarch64")]`
- `#[target_feature(enable = "neon")] unsafe fn popcount_512_neon(ptr: *const u8) -> u32`
- Dispatch via `std::arch::is_aarch64_feature_detected!("neon")` to NEON, else scalar.

Notes:

- Also support scalar fallback using `u64::count_ones()` on 8 words.
- Keep the NEON version behind `unsafe` with clear invariants (pointer valid for 64 bytes, etc).

### 3) Select implementation: sampled select + fast select-in-u64 (broadword/SWAR)

ARM doesn’t have x86 BMI2 (PDEP/PEXT), so we should:

- Implement **sampled select** (store position of every k-th 1-bit; typical k=256/512).
- Jump to the sampled neighborhood, then scan blocks accumulating popcount until the target.
- Once in the target 64-bit word, use a **fast broadword/SWAR select-in-u64** routine.

Implementation detail:

- For the scan phase, reuse the same 512-bit popcount primitive (NEON if available).
- For select-in-u64:
  - implement the classic SWAR approach (byte popcounts + prefix sums + locate the byte, then locate bit)
  - keep an alternative path that does `ctz` loops for very small `k` when the word is sparse (often wins)

### 4) Optional follow-ups (after rank9 baseline is in place)

If benchmarks show rank/select is dominated by cache misses rather than compute:

- Explore **SPIDER-style interleaving** of metadata with bit blocks to reduce cache misses for rank.
- Consider predictive select (SPIDER) as an experimental structure if we want to push select performance further.

---

## Public API proposal (minimal and ergonomic)

```rust
pub trait RankSelect {
    fn rank1(&self, i: usize) -> usize;   // number of 1s in [0, i)
    fn select1(&self, k: usize) -> usize; // position of k-th 1 (0-based), or panic/Option
    // optionally: rank0/select0
}

pub struct BitVecRankSelect { /* words + directories + optional select samples */ }
```

Construction:

- `BitVecRankSelect::from_bits(&[u64])` (or from bytes / from iter)
- `build_rank_index()`, `build_select_index(sample_rate)`

---

## Benchmark plan (must-have)

Use Criterion benchmarks for:

- Bitvector sizes: 1e6, 1e7, 1e8 bits
- Densities: 1%, 10%, 50%, 90%
- Query mixes: rank-heavy, select-heavy, mixed
- Random queries + adversarial patterns (long runs of zeros/ones)

Report:

- throughput (queries/sec)
- L1/L2 miss sensitivity (optional via `perf` or `dtrace` on supported platforms)

Targets:

- aarch64-apple-darwin (M1/M2/M3)
- aarch64-unknown-linux-gnu (Graviton if possible)
- scalar x86 baseline just to sanity-check competitiveness

---

## Acceptance criteria

- Correctness:
  - property tests vs a reference implementation for rank/select (random bitvectors + random queries)
  - edge cases: empty vector, all-zeros, all-ones, boundaries at word/block/superblock edges

- Performance:
  - NEON path measurably faster than scalar on AArch64 for block popcount and rank queries (esp. large vectors)
  - select performance competitive with straightforward baselines (sampled jump + scan)

- Maintainability:
  - single safe public API, unsafe isolated to small architecture-specific modules
  - clear cfg/feature gates and runtime dispatch

---

## Implementation notes (Rust/ARM specifics)

- Use `core::arch::aarch64::*` intrinsics (NEON).
- Gate NEON code under `#[cfg(all(target_arch="aarch64", target_endian="little"))]` (most practical targets).
- Provide scalar fallback always.
- Prefer runtime detection for binaries; optionally allow `RUSTFLAGS="-C target-cpu=native"` to let users force enable.

---

## Tasks

- [ ] Add rank/select module + baseline scalar rank9-like structure
- [ ] Add NEON popcount512 primitive + dispatch
- [ ] Implement sampled select + SWAR select-in-u64
- [ ] Add criterion benchmarks + correctness/proptests
- [ ] Evaluate layout tweaks (block size, directory packing, metadata interleaving)
