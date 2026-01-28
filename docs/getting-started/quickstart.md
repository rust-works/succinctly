# Quickstart Tutorial

[Home](/) > [Docs](../) > [Getting Started](./) > Quickstart

Get started with succinctly in 5 minutes.

## Using the Library

### BitVec: Efficient Bit Vectors

```rust
use succinctly::bits::BitVec;

fn main() {
    // Create a bit vector
    let bv = BitVec::from_bits(&[true, false, true, true, false, true]);

    // Rank: count 1-bits up to position
    assert_eq!(bv.rank1(3), 2);  // Two 1-bits in positions 0-2
    assert_eq!(bv.rank1(6), 4);  // Four 1-bits total

    // Select: find position of nth 1-bit
    assert_eq!(bv.select1(1), Some(0));  // First 1-bit at position 0
    assert_eq!(bv.select1(3), Some(3));  // Third 1-bit at position 3
}
```

### BalancedParens: Tree Navigation

```rust
use succinctly::trees::BalancedParens;

fn main() {
    // Encode a tree: ( ( ) ( ( ) ) )
    //                 A  B   C  D
    let bp = BalancedParens::from_bits(&[
        true, true, false, true, true, false, false, false
    ]);

    // Navigate the tree
    assert_eq!(bp.find_close(0), Some(7));  // A's closing paren
    assert_eq!(bp.find_close(1), Some(2));  // B's closing paren
    assert_eq!(bp.find_close(3), Some(6));  // C's closing paren
}
```

### JsonIndex: Zero-Copy JSON Parsing

```rust
use succinctly::json::JsonIndex;

fn main() {
    let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;

    // Build the index (one-time cost)
    let index = JsonIndex::build(json);

    // Navigate without copying
    let root = index.root(json);
    if let Some(users) = root.get("users") {
        for user in users.iter() {
            if let Some(name) = user.get("name") {
                println!("User: {}", name.as_str().unwrap());
            }
        }
    }
}
```

## Using the CLI

Install short aliases for interactive use (optional but recommended):

```bash
succinctly install-aliases    # creates sjq, syq, sjq-locate, syq-locate
```

### Query JSON with jq

```bash
# Pretty print
sjq '.' data.json

# Extract field
sjq '.name' data.json

# Filter array
sjq '.users[] | select(.age > 30)' data.json

# Compact output
sjq -c '.' data.json
```

### Query YAML with yq

```bash
# Pretty print
syq '.' config.yaml

# Convert to JSON
syq -o json '.' config.yaml

# Extract field
syq '.metadata.name' deployment.yaml

# Compact JSON output
syq -o json -I 0 '.' config.yaml
```

> **Note:** All commands also work with the full syntax: `succinctly jq`, `succinctly yq`, etc.

### Generate Test Data

```bash
# Generate JSON
succinctly json generate 1mb -o test.json

# Generate YAML test suite
succinctly yaml generate-suite
```

## Performance Tips

1. **Build with release mode**: `cargo build --release`
2. **Use native CPU features**: `RUSTFLAGS="-C target-cpu=native"`
3. **Reuse indexes**: Build once, query many times
4. **Use compact output**: `-c` for jq, `-I 0` for yq

## Next Steps

- [Examples](examples.md) - More usage patterns
- [API Guide](../guides/api.md) - Full library reference
- [CLI Guide](../guides/cli.md) - Complete CLI documentation
