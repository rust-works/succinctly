# Installation

[Home](/) > [Docs](../) > [Getting Started](./) > Installation

## Library Installation

Add succinctly to your `Cargo.toml`:

```toml
[dependencies]
succinctly = "0.1"
```

Or use cargo:

```bash
cargo add succinctly
```

## Feature Flags

| Feature | Description | Default |
|---------|-------------|---------|
| `simd` | Explicit SIMD intrinsics | Off |
| `portable-popcount` | Portable bitwise popcount | Off |
| `cli` | Command-line tool | Off |

### SIMD Feature

For maximum performance on supported platforms:

```toml
[dependencies]
succinctly = { version = "0.1", features = ["simd"] }
```

This enables:
- AVX2 on x86_64
- NEON on ARM64

### CLI Tool

To build the command-line tool:

```bash
cargo build --release --features cli
```

Or install globally:

```bash
cargo install succinctly --features cli
```

## Platform-Specific Notes

### macOS (Apple Silicon)

Best performance with native CPU features:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release --features cli
```

### Linux (x86_64)

For AVX2 support:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release --features cli
```

### Windows

Standard build works, but for best performance:

```powershell
$env:RUSTFLAGS="-C target-cpu=native"
cargo build --release --features cli
```

## Verifying Installation

### Library

```rust
use succinctly::bits::BitVec;

fn main() {
    let bv = BitVec::from_bits(&[true, false, true, true]);
    println!("Rank at position 3: {}", bv.rank1(3));
}
```

### CLI

```bash
# Check version
./target/release/succinctly --version

# Test jq
echo '{"name": "test"}' | ./target/release/succinctly jq '.name'

# Test yq
echo 'name: test' | ./target/release/succinctly yq '.name'
```

## Troubleshooting

### Missing SIMD Support

If you see performance warnings about SIMD:

1. Ensure you're building with `--release`
2. Try adding `-C target-cpu=native` to RUSTFLAGS
3. Check your CPU supports AVX2 (x86_64) or NEON (ARM64)

### Build Errors

If cargo build fails:

1. Update Rust: `rustup update`
2. Ensure Rust 1.70 or later: `rustc --version`
3. Clear cache: `cargo clean`

## See Also

- [Quickstart Tutorial](quickstart.md) - Hands-on introduction
- [Examples](examples.md) - Common usage patterns
