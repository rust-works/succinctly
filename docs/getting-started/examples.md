# Examples

[Home](/) > [Docs](../) > [Getting Started](./) > Examples

Common usage patterns and real-world examples.

## Library Examples

### Counting Set Bits in Ranges

```rust
use succinctly::bits::BitVec;

fn count_active_in_range(flags: &BitVec, start: usize, end: usize) -> u64 {
    if start == 0 {
        flags.rank1(end)
    } else {
        flags.rank1(end) - flags.rank1(start)
    }
}

fn main() {
    // User activity flags (1 = active)
    let activity = BitVec::from_bits(&[
        true, false, true, true, false,
        true, true, false, true, true
    ]);

    // Count active users in range [2, 7)
    let active = count_active_in_range(&activity, 2, 7);
    println!("Active users in range: {}", active);  // 4
}
```

### Finding nth Occurrence

```rust
use succinctly::bits::BitVec;

fn find_nth_error(error_log: &BitVec, n: u64) -> Option<usize> {
    error_log.select1(n)
}

fn main() {
    // Error log (1 = error at that position)
    let errors = BitVec::from_bits(&[
        false, false, true, false, true,
        false, false, true, false, false
    ]);

    // Find 2nd error
    if let Some(pos) = find_nth_error(&errors, 2) {
        println!("2nd error at position: {}", pos);  // 4
    }
}
```

### JSON Path Extraction

```rust
use succinctly::json::JsonIndex;

fn extract_all_names(json: &[u8]) -> Vec<String> {
    let index = JsonIndex::build(json);
    let root = index.root(json);

    let mut names = Vec::new();
    if let Some(users) = root.get("users") {
        for user in users.iter() {
            if let Some(name) = user.get("name") {
                if let Some(s) = name.as_str() {
                    names.push(s.to_string());
                }
            }
        }
    }
    names
}
```

## CLI Examples

### Kubernetes Configuration

```bash
# Extract all container images from a deployment
succinctly yq '.spec.template.spec.containers[].image' deployment.yaml

# Get all service ports
succinctly yq '.spec.ports[] | .port' service.yaml

# Convert deployment to JSON for processing
succinctly yq -o json '.' deployment.yaml | jq '.metadata'
```

### API Response Processing

```bash
# Extract user IDs from API response
curl -s https://api.example.com/users | succinctly jq '.[].id'

# Filter active users
curl -s https://api.example.com/users | succinctly jq '.[] | select(.active)'

# Format as CSV
curl -s https://api.example.com/users | \
  succinctly jq -r '.[] | [.name, .email] | @csv'
```

### Log Analysis

```bash
# Parse JSON logs
cat app.log | succinctly jq 'select(.level == "ERROR") | .message'

# Count errors by type
cat app.log | succinctly jq -s 'group_by(.error_type) | map({type: .[0].error_type, count: length})'

# Extract timestamps
cat app.log | succinctly jq -r '.timestamp'
```

### Configuration Merging

```bash
# Merge default and override configs
succinctly yq -s '.[0] * .[1]' defaults.yaml overrides.yaml

# Update a specific field
succinctly yq '.version = "2.0"' -i config.yaml
```

### Data Transformation

```bash
# JSON to YAML
succinctly jq '.' input.json | succinctly yq -p json '.'

# YAML to compact JSON
succinctly yq -o json -I 0 '.' config.yaml

# Filter and transform
succinctly jq '.items[] | {id, name: .title}' data.json
```

## Performance Patterns

### Batch Processing

```rust
use succinctly::json::JsonIndex;

fn process_batch(json_bytes: &[u8]) {
    // Build index once
    let index = JsonIndex::build(json_bytes);

    // Query multiple times without rebuilding
    let root = index.root(json_bytes);

    // Process items
    for item in root.get("items").unwrap().iter() {
        // ... process each item
    }
}
```

### Streaming Large Files

```bash
# Process large JSON line by line
while IFS= read -r line; do
    echo "$line" | succinctly jq -c '.id'
done < large_file.jsonl

# Or use the CLI directly
succinctly jq '.id' large_file.jsonl
```

## See Also

- [API Guide](../guides/api.md) - Complete API reference
- [CLI Guide](../guides/cli.md) - All CLI options
- [Benchmarks](../benchmarks/) - Performance data
