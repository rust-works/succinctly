# CPU Target Optimizations

This document describes CPU-specific optimization configurations for building high-performance code on various architectures.

## ARM Graviton (Neoverse-V2)

### Platform Details

- **Architecture**: AArch64 (ARM 64-bit)
- **CPU Model**: ARM Neoverse-V2
- **Vector Extensions**: SVE2 (Scalable Vector Extension 2) with 128-bit vectors
- **Advanced Features**:
  - SVE2 Bit Permutation (`svebitperm`) - includes BDEP/BEXT
  - SVE2 AES (`sveaes`)
  - SVE2 SHA3 (`svesha3`)
  - SVE2 Polynomial Multiply (`svepmull`)
  - SVE Int8 Matrix Multiply (`svei8mm`)
  - SVE BFloat16 (`svebf16`)
  - Int8 Matrix Multiply (`i8mm`)
  - BFloat16 operations (`bf16`)

### CPU Features

The Neoverse-V2 includes the following capabilities:

```
fp asimd evtstrm aes pmull sha1 sha2 crc32 atomics fphp asimdhp cpuid
asimdrdm jscvt fcma lrcpc dcpop sha3 asimddp sha512 sve asimdfhm dit uscat
ilrcpc flagm sb paca pacg dcpodp sve2 sveaes svepmull svebitperm svesha3
flagm2 frint svei8mm svebf16 i8mm bf16 dgh rng bti
```

Key features:
- **SVE2**: Scalable Vector Extension 2 (128-bit on Neoverse-V2)
- **SVEBITPERM**: SVE2 Bit Permutation (BDEP/BEXT instructions)
- **ASIMD**: Advanced SIMD (NEON)
- **Crypto**: AES, SHA1, SHA2, SHA3, SHA512
- **Atomics**: Large System Extensions (LSE)
- **Matrix ops**: Int8 and BFloat16 matrix multiply
- **BTI**: Branch Target Identification (security)

### Rust Configuration

#### Cargo Config (~/.cargo/config.toml)

```toml
[build]
# Optimize for Neoverse-V1 with SVE support
rustflags = ["-C", "target-cpu=neoverse-v2"]

[target.aarch64-unknown-linux-gnu]
# Additional Graviton-specific optimizations
rustflags = [
    "-C", "target-cpu=neoverse-v2",
    "-C", "opt-level=3",
    "-C", "codegen-units=1",
]

# Profile for maximum performance
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
strip = true

# Profile for performance testing
[profile.bench]
inherits = "release"
```

#### Compiler Flags Explained

- **`-C target-cpu=neoverse-v2`**: Enables all Neoverse-V1 specific instructions including SVE
- **`-C opt-level=3`**: Maximum optimization level
- **`-C codegen-units=1`**: Single codegen unit for better cross-function optimization
- **`lto = "fat"`**: Full link-time optimization across all crates
- **`strip = true`**: Remove debug symbols from release builds

### Build Commands

```bash
# Build with optimizations
cargo build --release

# Generate assembly to verify SVE usage
cargo rustc --release -- --emit asm

# Run benchmarks
cargo bench

# Check generated assembly for SVE instructions
# Look for: z registers (z0-z31), p registers (p0-p15), SVE instructions
objdump -d target/release/binary_name | grep -E "z[0-9]|ptrue|whilelo"
```

### Verification

#### Check CPU Features at Runtime

```bash
# View all CPU features
cat /proc/cpuinfo | grep Features

# Check for SVE specifically
cat /proc/cpuinfo | grep -i sve

# Get CPU model
lscpu | grep "Model name"
```

#### Verify Rust Toolchain Support

```bash
# List available target CPUs
rustc --print target-cpus | grep neoverse

# List available target features
rustc --print target-features | grep -i sve
```

Output should include:
- `neoverse-v2` in the CPU list
- `sve`, `sve2`, `svei8mm`, `svebf16` in features

### Performance Considerations

#### Auto-Vectorization

The Rust compiler (via LLVM) will automatically vectorize suitable loops using SVE when:
- Target CPU is set to `neoverse-v2` or `native`
- Code patterns are vectorization-friendly (contiguous memory access, simple operations)
- No data dependencies prevent vectorization

Example vectorizable code:
```rust
// This will likely auto-vectorize with SVE
fn vector_add(a: &[f32], b: &[f32], result: &mut [f32]) {
    a.iter()
        .zip(b.iter())
        .zip(result.iter_mut())
        .for_each(|((x, y), r)| *r = x + y);
}
```

#### Manual SIMD

For explicit control, use:

1. **Portable SIMD** (nightly):
```rust
#![feature(portable_simd)]
use std::simd::*;
```

2. **Architecture-specific intrinsics**:
```rust
#[cfg(target_arch = "aarch64")]
use std::arch::aarch64::*;
```

3. **Third-party crates**:
- `packed_simd` - Portable SIMD for stable Rust
- `pulp` - High-level SIMD abstraction
- `simdeez` - Cross-platform SIMD

### SVE vs NEON

| Feature | NEON (ASIMD) | SVE |
|---------|--------------|-----|
| Vector width | Fixed (128-bit) | Scalable (128-2048 bits) |
| On Neoverse-V2 | 128-bit | 128-bit |
| Register count | 32 (v0-v31) | 32 (z0-z31) + predicates |
| Predication | Limited | Full predicate support |
| Portability | All ARMv8+ | ARMv8.2-A+ |

**When to use SVE**:
- Operations on large arrays
- When predicates simplify loop handling
- Matrix operations (with SVE2)
- Machine learning workloads

**When NEON is sufficient**:
- 128-bit vectors are adequate
- Targeting older ARM processors
- Code already optimized for NEON

### Common Pitfalls

1. **Not setting target-cpu**: Without `-C target-cpu=neoverse-v2`, SVE won't be used
2. **Multiple codegen units**: Setting `codegen-units > 1` limits cross-function optimization
3. **Debug builds**: Optimizations are disabled; always benchmark release builds
4. **Memory alignment**: Ensure proper alignment for vectorized loads/stores
5. **Data dependencies**: Loop-carried dependencies prevent auto-vectorization

### Benchmarking Example

See [sve_benchmark](/home/ubuntu/sve_benchmark/) for a complete example showing:
- Vector addition performance comparison
- Matrix multiplication benchmarks
- Verification of SVE instruction generation

Typical speedups with proper vectorization:
- Vector operations: 2-4x
- Matrix operations: 4-8x (with SVE matrix extensions)
- Memory-bound operations: 1.5-2x

### Additional Resources

- [ARM Neoverse V1 Core Software Optimization Guide](https://developer.arm.com/documentation/PJDOC-466751330-590448/latest)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [LLVM AArch64 Backend Documentation](https://llvm.org/docs/CodeGenerator.html)
- [ARM SVE Programming Guide](https://developer.arm.com/documentation/102476/latest/)

### Related Documentation

- [Build Configuration](../build.md) - General build settings
- [Performance Tuning](../performance.md) - Optimization techniques
- [Benchmarking Guide](../benchmarking.md) - How to measure performance

---

**Last Updated**: 2026-01-21
**Target Platform**: AWS Graviton4 / Neoverse-V2
**Rust Version**: 1.92.0
