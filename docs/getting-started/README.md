# Getting Started with Succinctly

[Home](/) > [Docs](../) > Getting Started

Welcome! This guide will get you up and running with succinctly.

## Prerequisites

- Rust 1.70 or later
- Cargo (comes with Rust)

## Quick Install

Add to your `Cargo.toml`:

```toml
[dependencies]
succinctly = "0.1"
```

Or use cargo:

```bash
cargo add succinctly
```

## CLI Tool

For command-line usage (jq/yq replacement):

```bash
# Build the CLI
cargo install succinctly --features cli

# Or build from source
cargo build --release --features cli
```

## Next Steps

- [Installation Details](installation.md) - Feature flags, platform-specific notes
- [Quickstart Tutorial](quickstart.md) - 5-minute hands-on introduction
- [Common Examples](examples.md) - Real-world usage patterns

## What's Next?

Once you're comfortable with the basics:

- [API Guide](../guides/api.md) - Full library reference
- [CLI Guide](../guides/cli.md) - Command-line tool reference
- [Architecture](../architecture/) - How it works under the hood
