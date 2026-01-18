# Succinctly Documentation

Welcome to the succinctly documentation! This page helps you find what you need.

## Quick Links by Audience

### First-Time Users
**Start here**: [Getting Started Guide](getting-started/)

Learn the basics:
- [Installation](getting-started/installation.md)
- [Quickstart Tutorial](getting-started/quickstart.md)
- [Common Examples](getting-started/examples.md)

### Library Users
**Using succinctly in your Rust project**:
- [API Guide](guides/api.md) - Comprehensive API reference with examples
- [CLI Guide](guides/cli.md) - Command-line tool reference

### Contributors
**Want to contribute?**
- [CONTRIBUTING.md](../CONTRIBUTING.md) - Start here
- [Developer Guide](guides/developer.md) - Codebase architecture and workflow
- [Release Guide](guides/release.md) - Release process (for maintainers)

### Performance Engineers
**Optimizing performance**:
- [Optimization Techniques](optimizations/) - 11 comprehensive guides
- [Quick Reference](optimizations/README.md#quick-reference) - One-page technique lookup table
- [Benchmarks](benchmarks/) - Performance comparisons vs other tools

### Researchers & Deep Divers
**Understanding internals**:
- [Architecture](architecture/) - Design decisions and core concepts
- [Parsing Implementation](parsing/) - JSON/YAML/DSV parser internals
- [Implementation Plans](plan/) - Feature planning documents
- [Archive](archive/) - Historical context and failed experiments

### AI-Assisted Development
- [CLAUDE.md](../CLAUDE.md) - Comprehensive guide for AI assistants

---

## Documentation Structure

### [getting-started/](getting-started/)
Quick tutorials for new users. Start here if you've never used succinctly.

### [guides/](guides/)
Practical how-to documentation:
- API usage (api.md)
- CLI tool (cli.md)
- Development (developer.md)
- Releases (release.md)

### [architecture/](architecture/)
Design documentation:
- Core concepts (BitVec, BalancedParens, semi-indexing)
- Module structure
- Implementation decisions

### [parsing/](parsing/)
Parser implementation details:
- JSON semi-indexing
- YAML parser with P0-P10 optimizations
- DSV (CSV/TSV) parsing

### [optimizations/](optimizations/)
Performance optimization techniques:
- 11 comprehensive technique guides
- Decision framework
- Successes AND failures documented

### [benchmarks/](benchmarks/)
Performance comparisons:
- vs jq (JSON queries)
- vs yq (YAML queries)
- vs Rust JSON parsers (serde_json, sonic-rs, simd-json)
- Cross-language parser comparisons
- DSV performance

### [plan/](plan/)
Implementation plans for major features (all implemented).

### [archive/](archive/)
Historical documentation:
- Optimization history (successes and failures)
- Haskell reference implementations
- Migration notes

---

## Finding What You Need

**I want to...**

- **Install and try succinctly** -> [getting-started/](getting-started/)
- **Use BitVec or BalancedParens** -> [guides/api.md](guides/api.md)
- **Query JSON files** -> [guides/cli.md](guides/cli.md#jq-command)
- **Query YAML files** -> [guides/cli.md](guides/cli.md#yq-command)
- **Understand how JSON indexing works** -> [parsing/json.md](parsing/json.md)
- **See YAML optimization journey** -> [parsing/yaml.md](parsing/yaml.md)
- **Learn SIMD techniques** -> [optimizations/simd.md](optimizations/simd.md)
- **Compare performance** -> [benchmarks/](benchmarks/)
- **Contribute code** -> [CONTRIBUTING.md](../CONTRIBUTING.md) + [guides/developer.md](guides/developer.md)
- **Release a new version** -> [guides/release.md](guides/release.md)
- **Understand why AVX-512 was rejected** -> [archive/optimizations/](archive/optimizations/)

---

## Contributing to Documentation

Found a typo or want to improve docs? See [CONTRIBUTING.md](../CONTRIBUTING.md).

Documentation follows these conventions:
- US spelling (optimize, not optimise)
- Breadcrumbs at top of nested docs
- Links use descriptive text (not "click here")
- Code examples are tested and runnable
