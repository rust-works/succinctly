# YAML SIMD Build Matrix

This document describes the build configurations for YAML SIMD optimizations across different target architectures and feature flags.

## Feature Flags

| Feature          | Description                                                              |
|------------------|--------------------------------------------------------------------------|
| `broadword-yaml` | Use portable broadword (SWAR) instead of NEON on ARM64                   |
| `scalar-yaml`    | Use pure scalar (byte-by-byte) processing. Disables all SIMD/broadword   |

## Build Matrix

`scalar` is compiled unconditionally on every target and flag combination below: the
vector kernels call it to finish the remainder their vector loop cannot cover, so it is
never merely a fallback. The "Module Used" column names what is compiled *in addition*.

### Target: x86_64

| Feature Flags    | Module Used | Functions Used       | Notes                                              |
|------------------|-------------|----------------------|----------------------------------------------------|
| (none)           | `x86`       | SSE2/AVX2 intrinsics | Default. Runtime AVX2 detection with SSE2 fallback |
| `broadword-yaml` | `x86`       | SSE2/AVX2 intrinsics | Flag ignored on x86_64                             |
| `scalar-yaml`    | (none)      | `*_scalar` functions | Pure byte-by-byte, for benchmarking baseline       |

### Target: aarch64 (ARM64)

| Feature Flags    | Module Used          | Functions Used                           | Notes                                                       |
|------------------|----------------------|------------------------------------------|-------------------------------------------------------------|
| (none)           | `neon` + `broadword` | NEON intrinsics; SWAR for `find_newline` | Default. `broadword` also owns the classifier (#185)        |
| `broadword-yaml` | `broadword`          | Broadword (SWAR)                         | Portable u64 arithmetic, no intrinsics; `neon` not compiled |
| `scalar-yaml`    | (none)               | `*_scalar` functions                     | Pure byte-by-byte, for benchmarking baseline                |

### Target: Other (WebAssembly, RISC-V, etc.)

| Feature Flags    | Module Used | Functions Used       | Notes                                     |
|------------------|-------------|----------------------|-------------------------------------------|
| (none)           | `broadword` | Broadword (SWAR)     | Automatic fallback for non-SIMD platforms |
| `broadword-yaml` | `broadword` | Broadword (SWAR)     | Same as default                           |
| `scalar-yaml`    | (none)      | `*_scalar` functions | Pure byte-by-byte                         |

## Module Compilation Rules

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  scalar module: always compiled (vector kernels call it for the remainder)   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           scalar-yaml enabled?                               │
│                                                                              │
│  YES → No vector modules compiled. All functions use *_scalar implementations│
│                                                                              │
│  NO  → Continue to architecture check                                        │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Target Architecture                               │
│                                                                              │
│  x86_64:                                                                     │
│    └─ x86 module compiled (SSE2/AVX2). broadword NOT compiled                │
│                                                                              │
│  Everything else:                                                            │
│    └─ broadword module compiled (owns the classifier and find_newline)       │
│        └─ aarch64 without broadword-yaml?                                    │
│            YES → neon module compiled too; dispatch prefers NEON where it    │
│                  has a kernel, broadword elsewhere                           │
│            NO  → broadword is the whole implementation                       │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Module Compilation Details

The broadword module is compiled on every non-x86_64 target, because it owns the only
copy of the SWAR classifier and of `find_newline_broadword`:

```rust
#[cfg(all(not(feature = "scalar-yaml"), not(target_arch = "x86_64")))]
mod broadword;
```

This means:
- **x86_64**: Only the `x86` module is compiled (native SIMD covers all of it)
- **aarch64**: Both `neon` and `broadword` are compiled. NEON supplies quote scanning,
  indentation counting, anchors and block scalars; broadword supplies the classifier
  and `find_newline`, which have no NEON kernel. `broadword-yaml` switches the
  *dispatch* of the NEON-backed operations over to broadword for comparison — it no
  longer controls whether the module is compiled.
- **Other platforms**: Only `broadword` is compiled (automatic fallback)

### Historical Context: Why This Changed

Until #185 the gate above was `aarch64 + broadword-yaml`, or a non-SIMD arch. That kept
the module out of the default ARM64 build — but only because `neon.rs` carried its own
copy of the whole broadword layer to serve that build. The two copies were gated to
complementary cfgs, so no build ever compiled both and the compiler never compared them;
they drifted until #185 deleted the `neon.rs` copy and widened the gate. The
`#[allow(dead_code)]` markers that gate was avoiding are cheaper than the drift, and are
now attached per function with a STYLE-0005 justification.

Before that, an earlier revision compiled broadword on ARM64 unconditionally and hit
these dead-code warnings, which is what motivated the narrow gate in the first place:

| Build Target | Feature Flags | Warning Source              | Reason                                          |
|--------------|---------------|-----------------------------|-------------------------------------------------|
| aarch64      | (none)        | `broadword.rs` scan kernels | NEON is dispatched to instead                   |
| aarch64      | (none)        | `classify_yaml_chars_16`    | Only the disabled `skip_unquoted_simd` calls it |

Use `--features broadword-yaml` for comparison testing on ARM64.

## Performance Characteristics

| Implementation   | Throughput           | Best For                        |
|------------------|----------------------|---------------------------------|
| AVX2 (x86_64)    | 32 bytes/iteration   | Long strings, bulk processing   |
| SSE2 (x86_64)    | 16 bytes/iteration   | Fallback when AVX2 unavailable  |
| NEON (aarch64)   | 16 bytes/iteration   | ARM64 default                   |
| Broadword        | 8 bytes/iteration    | Portable fallback, no intrinsics|
| Scalar           | 1 byte/iteration     | Baseline, debugging             |

## Usage Examples

```bash
# Default build (uses platform-native SIMD)
cargo build --release

# ARM64: Use broadword instead of NEON (for comparison)
cargo build --release --features broadword-yaml

# Benchmark scalar baseline
cargo bench --features scalar-yaml --bench yaml_bench

# Compare broadword vs NEON on ARM64
cargo bench --bench yaml_bench                    # NEON
cargo bench --features broadword-yaml --bench yaml_bench  # Broadword
```
