//! Registry of YAML generator patterns: names, dispatch enum, and size-ladder
//! scale.
//!
//! Split out of `yaml_generators.rs` (#517) so `benches/yq_comparison.rs` can
//! share `ALL_PATTERNS` via `#[path]` without also compiling every
//! `generate_*` function (and `rand`) that only the generators themselves
//! need.

#[derive(Debug, Clone, Copy)]
pub enum YamlPattern {
    /// Comprehensive pattern testing various YAML features
    Comprehensive,
    /// Array of user/person records (realistic structure)
    Users,
    /// Deeply nested mappings and sequences
    Nested,
    /// Sequences at various levels
    Sequences,
    /// Mixed mappings and sequences
    Mixed,
    /// String-heavy with quoted strings
    Strings,
    /// Numeric values (integers and floats)
    Numbers,
    /// Configuration file style (realistic config)
    Config,
    /// Unicode strings in various scripts
    Unicode,
    /// Worst case for parsing (maximum depth and density)
    Pathological,
    /// Top-level array designed for M2 navigation benchmarks
    /// (supports .[0], .[], .[0].name queries)
    Navigation,
    /// Flow mappings `{...}` and sequences `[...]`
    Flow,
    /// Anchors `&name` and aliases `*name`
    Anchors,
    /// Literal `|` and folded `>` block scalars, with chomping modifiers
    BlockScalars,
    /// Explicit keys (`? ` / `: `), including the valueless form that makes
    /// open positions non-monotonic
    ExplicitKeys,
    /// Multi-document streams (`---` / `...`)
    MultiDoc,
    /// Top-level sequence of childless bare-dash items (`-\n` repeated) — the
    /// shape #337 found quadratic and no other pattern generates
    EmptyItems,
    /// Childless bare-dash items interleaved with items carrying inline
    /// scalar content (`- x`)
    HalfEmptyItems,
    /// Records whose `-` item indicator sits alone on its line, with the item
    /// body indented beneath it. Every other sequence pattern here emits
    /// dash-space (`- `), so this shape had no benchmark coverage at all — which
    /// is how the seq-item detection divergence in #106 stayed hidden.
    SeqWrap,
}

/// How the suite generator sizes a pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternScale {
    /// Generated once per size in the suite's size ladder.
    Scalable,
    /// A realistic fixed-size template; generated at one size only.
    Fixed,
}

/// Every generator pattern: suite directory name, pattern, and how it scales.
///
/// Single source of truth. `yaml generate-suite`, `dev bench yq`, and
/// `benches/yq_comparison.rs` (#517) all derive their pattern lists from this,
/// and `tests/yaml_bench_suite_coverage.rs` checks it against the
/// `yaml generate --pattern` value list, so a pattern cannot be registered in
/// one place and forgotten in another.
///
/// **Order is load-bearing.** `generate-suite` seeds each file with
/// `base_seed + file_index`, counting in this order, so appending a pattern
/// leaves every earlier pattern's bytes — and the benchmark numbers recorded
/// against them — unchanged. That is why `config` sits before the
/// feature-focused patterns added in #327 rather than at the end.
pub const ALL_PATTERNS: &[(&str, YamlPattern, PatternScale)] = &[
    (
        "comprehensive",
        YamlPattern::Comprehensive,
        PatternScale::Scalable,
    ),
    ("users", YamlPattern::Users, PatternScale::Scalable),
    ("nested", YamlPattern::Nested, PatternScale::Scalable),
    ("sequences", YamlPattern::Sequences, PatternScale::Scalable),
    ("mixed", YamlPattern::Mixed, PatternScale::Scalable),
    ("strings", YamlPattern::Strings, PatternScale::Scalable),
    ("numbers", YamlPattern::Numbers, PatternScale::Scalable),
    ("unicode", YamlPattern::Unicode, PatternScale::Scalable),
    (
        "pathological",
        YamlPattern::Pathological,
        PatternScale::Scalable,
    ),
    (
        "navigation",
        YamlPattern::Navigation,
        PatternScale::Scalable,
    ),
    ("config", YamlPattern::Config, PatternScale::Fixed),
    ("flow", YamlPattern::Flow, PatternScale::Scalable),
    ("anchors", YamlPattern::Anchors, PatternScale::Scalable),
    (
        "block-scalars",
        YamlPattern::BlockScalars,
        PatternScale::Scalable,
    ),
    (
        "explicit-keys",
        YamlPattern::ExplicitKeys,
        PatternScale::Scalable,
    ),
    ("multi-doc", YamlPattern::MultiDoc, PatternScale::Scalable),
    (
        "empty-items",
        YamlPattern::EmptyItems,
        PatternScale::Scalable,
    ),
    (
        "half-empty-items",
        YamlPattern::HalfEmptyItems,
        PatternScale::Scalable,
    ),
    ("seqwrap", YamlPattern::SeqWrap, PatternScale::Scalable),
];
