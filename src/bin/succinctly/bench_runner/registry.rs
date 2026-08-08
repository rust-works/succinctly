//! Static benchmark registry.
//!
//! Contains metadata for all benchmarks in the project.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Category of benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BenchmarkCategory {
    Core,
    Json,
    Yaml,
    Dsv,
    Text,
    CrossParser,
    Corpus,
}

impl fmt::Display for BenchmarkCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core => write!(f, "core"),
            Self::Json => write!(f, "json"),
            Self::Yaml => write!(f, "yaml"),
            Self::Dsv => write!(f, "dsv"),
            Self::Text => write!(f, "text"),
            Self::CrossParser => write!(f, "cross-parser"),
            Self::Corpus => write!(f, "corpus"),
        }
    }
}

impl std::str::FromStr for BenchmarkCategory {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "core" => Ok(Self::Core),
            "json" => Ok(Self::Json),
            "yaml" => Ok(Self::Yaml),
            "dsv" => Ok(Self::Dsv),
            "text" => Ok(Self::Text),
            "cross-parser" | "crossparser" | "cross_parser" => Ok(Self::CrossParser),
            "corpus" => Ok(Self::Corpus),
            _ => Err(format!(
                "unknown category '{s}', expected: core, json, yaml, dsv, text, cross-parser, corpus"
            )),
        }
    }
}

/// Type of benchmark execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BenchmarkType {
    /// Criterion benchmark (cargo bench --bench <name>)
    Criterion,
    /// CLI benchmark (succinctly bench run <name>)
    CliBench,
    /// Cross-parser comparison (requires bench-compare subproject)
    CrossParser,
}

impl fmt::Display for BenchmarkType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Criterion => write!(f, "criterion"),
            Self::CliBench => write!(f, "cli"),
            Self::CrossParser => write!(f, "cross-parser"),
        }
    }
}

/// Isolation requirement for orchestrated (multi-node) execution (issue #98).
///
/// Every benchmark here is timing/RSS-sensitive, so the registry default is
/// `Exclusive`; `nodes.yaml`'s `benchmarks[].isolation` overrides loosen
/// specific benchmarks to `Concurrent` on beefy nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Isolation {
    /// Needs the whole node — no other benchmark runs alongside it.
    Exclusive,
    /// May share the node with other `Concurrent` benchmarks.
    Concurrent,
}

impl fmt::Display for Isolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `f.pad(..)`, not `write!(f, ..)` — the latter would silently
        // ignore width/alignment specifiers like `{:<10}` (see Architecture's
        // Display impl in orchestrate/config.rs for the same fix, caught by
        // a real `bench nodes --status` misalignment).
        f.pad(match self {
            Self::Exclusive => "exclusive",
            Self::Concurrent => "concurrent",
        })
    }
}

/// Benchmark metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkInfo {
    /// Unique identifier (e.g., "rank_select", "yaml_bench")
    pub name: &'static str,
    /// Human-readable description
    pub description: &'static str,
    /// Category for grouping
    pub category: BenchmarkCategory,
    /// Execution type
    pub bench_type: BenchmarkType,
    /// Criterion benchmark name (if applicable)
    pub criterion_name: Option<&'static str>,
    /// CLI subcommand (if applicable, e.g., "jq", "yq")
    pub cli_subcommand: Option<&'static str>,
    /// Working directory (relative to repo root)
    pub working_dir: &'static str,
    /// Default orchestration isolation (issue #98); overridable per-name via
    /// `nodes.yaml`'s `benchmarks[].isolation`.
    pub default_isolation: Isolation,
}

/// Static registry of all benchmarks.
pub static BENCHMARKS: &[BenchmarkInfo] = &[
    // ========== CORE ==========
    BenchmarkInfo {
        name: "rank_select",
        description: "Rank and select operations on bitvectors",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("rank_select"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "balanced_parens",
        description: "Balanced parentheses basic operations",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("balanced_parens"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "bp_select_micro",
        description: "BP select1 micro-benchmarks",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("bp_select_micro"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "elias_fano",
        description: "Elias-Fano encoding benchmarks",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("elias_fano"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "popcount_strategies",
        description: "Popcount implementation comparison",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("popcount_strategies"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "neon_movemask",
        description: "ARM NEON movemask operations",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("neon_movemask"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    // ========== JSON ==========
    BenchmarkInfo {
        name: "json_pipeline",
        description: "JSON parsing and structural traversal",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_pipeline"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "json_simd_indexing",
        description: "JSON SIMD indexing (AVX2/SSE/NEON/PFSM, up to 100MB)",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_simd_indexing"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "json_simd_cursor",
        description: "JSON simple cursor (SIMD/Scalar, up to 10MB)",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_simd_cursor"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "json_simd_full",
        description: "JSON full index + pattern comparison",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_simd_full"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "pfsm_vs_simd",
        description: "PFSM parser vs SIMD comparison",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("pfsm_vs_simd"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "pfsm_vs_scalar",
        description: "PFSM parser vs scalar comparison",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("pfsm_vs_scalar"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "jq_comparison",
        description: "Criterion: succinctly jq vs system jq",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("jq_comparison"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "jq_string_ops_bench",
        description: "Micro: jq substring search — std vs memchr::memmem (issue #303)",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("jq_string_ops_bench"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "jq_bench",
        description: "CLI: succinctly jq vs system jq (with memory)",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("jq"),
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "json_validate_bench",
        description: "JSON RFC 8259 validation throughput",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_validate_bench"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    // ========== YAML ==========
    BenchmarkInfo {
        name: "yaml_bench",
        description: "YAML parsing micro-benchmarks",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yaml_bench"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "yaml_anchor_micro",
        description: "YAML anchor/alias parsing",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yaml_anchor_micro"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "yaml_transcode_micro",
        description: "YAML-to-JSON transcoding",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yaml_transcode_micro"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "yaml_type_stack_micro",
        description: "YAML type stack operations",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yaml_type_stack_micro"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "yq_comparison",
        description: "Criterion: succinctly yq vs system yq",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yq_comparison"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "yq_select",
        description: "YAML select operations",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yq_select"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "yq_bench",
        description: "CLI: succinctly yq vs system yq (with memory)",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("yq"),
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    // ========== DSV ==========
    BenchmarkInfo {
        name: "dsv_bench",
        description: "DSV parsing benchmarks",
        category: BenchmarkCategory::Dsv,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("dsv_bench"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "dsv_cli",
        description: "CLI: DSV input benchmarks",
        category: BenchmarkCategory::Dsv,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("dsv"),
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    // ========== TEXT ==========
    BenchmarkInfo {
        name: "line_index",
        description: "LineIndex::to_line_column vs the pre-#228 dense BitVec (#543)",
        category: BenchmarkCategory::Text,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("line_index"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "utf8_validate_bench",
        description: "UTF-8 validation throughput (Criterion)",
        category: BenchmarkCategory::Text,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("utf8_validate_bench"),
        cli_subcommand: None,
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "utf8_bench",
        description: "CLI: UTF-8 scalar validation baseline",
        category: BenchmarkCategory::Text,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("utf8"),
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    // ========== CORPUS ==========
    BenchmarkInfo {
        name: "corpus_stats",
        description: "Shape statistics for the real-workload corpus (#301)",
        category: BenchmarkCategory::Corpus,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("corpus-stats"),
        working_dir: ".",
        default_isolation: Isolation::Exclusive,
    },
    // ========== CROSS-PARSER ==========
    BenchmarkInfo {
        name: "json_parsers",
        description: "Cross-parser JSON comparison (serde, simd-json, sonic-rs)",
        category: BenchmarkCategory::CrossParser,
        bench_type: BenchmarkType::CrossParser,
        criterion_name: Some("json_parsers"),
        cli_subcommand: None,
        working_dir: "bench-compare",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "yaml_parsers",
        description: "Cross-parser YAML comparison (serde_yaml)",
        category: BenchmarkCategory::CrossParser,
        bench_type: BenchmarkType::CrossParser,
        criterion_name: Some("yaml_parsers"),
        cli_subcommand: None,
        working_dir: "bench-compare",
        default_isolation: Isolation::Exclusive,
    },
    BenchmarkInfo {
        name: "succinct_libs",
        description: "Cross-library rank/select comparison (vers-vecs, sucds, sux)",
        category: BenchmarkCategory::CrossParser,
        bench_type: BenchmarkType::CrossParser,
        criterion_name: Some("succinct_libs"),
        cli_subcommand: None,
        working_dir: "bench-compare",
        default_isolation: Isolation::Exclusive,
    },
];

/// Filter benchmarks by category.
pub fn filter_by_category(category: BenchmarkCategory) -> Vec<&'static BenchmarkInfo> {
    BENCHMARKS
        .iter()
        .filter(|b| b.category == category)
        .collect()
}

/// Filter benchmarks by name.
pub fn filter_by_names(names: &[String]) -> Vec<&'static BenchmarkInfo> {
    BENCHMARKS
        .iter()
        .filter(|b| names.iter().any(|n| n == b.name))
        .collect()
}

/// Get a benchmark by name.
#[allow(dead_code)] // STYLE-0005: bench-only lookup helper
pub fn get_by_name(name: &str) -> Option<&'static BenchmarkInfo> {
    BENCHMARKS.iter().find(|b| b.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_not_empty() {
        assert!(!BENCHMARKS.is_empty());
    }

    #[test]
    fn test_all_categories_have_benchmarks() {
        assert!(!filter_by_category(BenchmarkCategory::Core).is_empty());
        assert!(!filter_by_category(BenchmarkCategory::Json).is_empty());
        assert!(!filter_by_category(BenchmarkCategory::Yaml).is_empty());
        assert!(!filter_by_category(BenchmarkCategory::Dsv).is_empty());
        assert!(!filter_by_category(BenchmarkCategory::Text).is_empty());
        assert!(!filter_by_category(BenchmarkCategory::CrossParser).is_empty());
        assert!(!filter_by_category(BenchmarkCategory::Corpus).is_empty());
    }

    #[test]
    fn test_category_parsing() {
        assert_eq!(
            "core".parse::<BenchmarkCategory>().unwrap(),
            BenchmarkCategory::Core
        );
        assert_eq!(
            "cross-parser".parse::<BenchmarkCategory>().unwrap(),
            BenchmarkCategory::CrossParser
        );
    }
}
