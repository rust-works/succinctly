# YAML SIMD Build Matrix

This document describes the build configurations for YAML SIMD optimizations across different target architectures and feature flags.

## Feature Flags

| Feature          | Description                                                              |
|------------------|--------------------------------------------------------------------------|
| `broadword-yaml` | Use portable broadword (SWAR) instead of NEON on ARM64                   |
| `scalar-yaml`    | Use pure scalar (byte-by-byte) processing. Disables all SIMD/broadword   |

## Build Matrix

### Target: x86_64

| Feature Flags    | Module Used | Functions Used       | Notes                                              |
|------------------|-------------|----------------------|----------------------------------------------------|
| (none)           | `x86`       | SSE2/AVX2 intrinsics | Default. Runtime AVX2 detection with SSE2 fallback |
| `broadword-yaml` | `x86`       | SSE2/AVX2 intrinsics | Flag ignored on x86_64                             |
| `scalar-yaml`    | (none)      | `*_scalar` functions | Pure byte-by-byte, for benchmarking baseline       |

### Target: aarch64 (ARM64)

| Feature Flags    | Module Used | Functions Used       | Notes                                        |
|------------------|-------------|----------------------|----------------------------------------------|
| (none)           | `neon`      | NEON intrinsics      | Default. Uses ARM NEON SIMD                  |
| `broadword-yaml` | `broadword` | Broadword (SWAR)     | Portable u64 arithmetic, no intrinsics       |
| `scalar-yaml`    | (none)      | `*_scalar` functions | Pure byte-by-byte, for benchmarking baseline |

### Target: Other (WebAssembly, RISC-V, etc.)

| Feature Flags    | Module Used | Functions Used       | Notes                                     |
|------------------|-------------|----------------------|-------------------------------------------|
| (none)           | `broadword` | Broadword (SWAR)     | Automatic fallback for non-SIMD platforms |
| `broadword-yaml` | `broadword` | Broadword (SWAR)     | Same as default                           |
| `scalar-yaml`    | (none)      | `*_scalar` functions | Pure byte-by-byte                         |

## Module Compilation Rules

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           scalar-yaml enabled?                               │
│                                                                              │
│  YES → No SIMD modules compiled. All functions use *_scalar implementations  │
│                                                                              │
│  NO  → Continue to architecture check                                        │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Target Architecture                               │
│                                                                              │
│  x86_64:                                                                     │
│    └─ x86 module compiled (SSE2/AVX2)                                        │
│                                                                              │
│  aarch64:                                                                    │
│    └─ broadword-yaml enabled?                                                │
│        YES → broadword module compiled and used                              │
│        NO  → neon module compiled and used                                   │
│                                                                              │
│  Other:                                                                      │
│    └─ broadword module compiled and used (automatic fallback)                │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Module Compilation Details

The broadword module is only compiled when actually needed, eliminating dead code warnings:

```rust
#[cfg(all(
    not(feature = "scalar-yaml"),
    any(
        all(target_arch = "aarch64", feature = "broadword-yaml"),
        not(any(target_arch = "aarch64", target_arch = "x86_64"))
    )
))]
mod broadword;
```

This means:
- **x86_64**: Only the `x86` module is compiled (broadword not needed)
- **aarch64**: Only `neon` is compiled by default; `broadword` requires explicit opt-in via `broadword-yaml`
- **Other platforms**: Only `broadword` is compiled (automatic fallback)

### Historical Context: Why This Changed

Previous versions compiled the broadword module on ARM64 even when not used, which caused dead code warnings:

| Build Target | Feature Flags | Warning Source                     | Reason                                           |
|--------------|---------------|------------------------------------|--------------------------------------------------|
| aarch64      | (none)        | `broadword.rs` functions           | NEON is used; broadword compiled but not called  |
| aarch64      | (none)        | `neon.rs::classify_yaml_chars_16`  | Infrastructure function not yet integrated       |
| aarch64      | (none)        | `neon.rs::find_newline_broadword`  | Infrastructure function not yet integrated       |

The old approach allowed easy testing/comparison via feature flag, but at the cost of dead code warnings. The current approach requires explicitly enabling `broadword-yaml` for comparison testing on ARM64.

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
