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
    CrossParser,
}

impl fmt::Display for BenchmarkCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core => write!(f, "core"),
            Self::Json => write!(f, "json"),
            Self::Yaml => write!(f, "yaml"),
            Self::Dsv => write!(f, "dsv"),
            Self::CrossParser => write!(f, "cross-parser"),
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
            "cross-parser" | "crossparser" | "cross_parser" => Ok(Self::CrossParser),
            _ => Err(format!(
                "unknown category '{}', expected: core, json, yaml, dsv, cross-parser",
                s
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

/// Benchmark metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    },
    BenchmarkInfo {
        name: "balanced_parens",
        description: "Balanced parentheses basic operations",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("balanced_parens"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "bp_select_micro",
        description: "BP select1 micro-benchmarks",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("bp_select_micro"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "elias_fano",
        description: "Elias-Fano encoding benchmarks",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("elias_fano"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "popcount_strategies",
        description: "Popcount implementation comparison",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("popcount_strategies"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "neon_movemask",
        description: "ARM NEON movemask operations",
        category: BenchmarkCategory::Core,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("neon_movemask"),
        cli_subcommand: None,
        working_dir: ".",
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
    },
    BenchmarkInfo {
        name: "json_simd_indexing",
        description: "JSON SIMD indexing (AVX2/SSE/NEON/PFSM, up to 100MB)",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_simd_indexing"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "json_simd_cursor",
        description: "JSON simple cursor (SIMD/Scalar, up to 10MB)",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_simd_cursor"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "json_simd_full",
        description: "JSON full index + pattern comparison",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("json_simd_full"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "pfsm_vs_simd",
        description: "PFSM parser vs SIMD comparison",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("pfsm_vs_simd"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "pfsm_vs_scalar",
        description: "PFSM parser vs scalar comparison",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("pfsm_vs_scalar"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "jq_comparison",
        description: "Criterion: succinctly jq vs system jq",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("jq_comparison"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "jq_bench",
        description: "CLI: succinctly jq vs system jq (with memory)",
        category: BenchmarkCategory::Json,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("jq"),
        working_dir: ".",
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
    },
    BenchmarkInfo {
        name: "yaml_anchor_micro",
        description: "YAML anchor/alias parsing",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yaml_anchor_micro"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "yaml_transcode_micro",
        description: "YAML-to-JSON transcoding",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yaml_transcode_micro"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "yaml_type_stack_micro",
        description: "YAML type stack operations",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yaml_type_stack_micro"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "yq_comparison",
        description: "Criterion: succinctly yq vs system yq",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yq_comparison"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "yq_select",
        description: "YAML select operations",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::Criterion,
        criterion_name: Some("yq_select"),
        cli_subcommand: None,
        working_dir: ".",
    },
    BenchmarkInfo {
        name: "yq_bench",
        description: "CLI: succinctly yq vs system yq (with memory)",
        category: BenchmarkCategory::Yaml,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("yq"),
        working_dir: ".",
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
    },
    BenchmarkInfo {
        name: "dsv_cli",
        description: "CLI: DSV input benchmarks",
        category: BenchmarkCategory::Dsv,
        bench_type: BenchmarkType::CliBench,
        criterion_name: None,
        cli_subcommand: Some("dsv"),
        working_dir: ".",
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
    },
    BenchmarkInfo {
        name: "yaml_parsers",
        description: "Cross-parser YAML comparison (serde_yaml)",
        category: BenchmarkCategory::CrossParser,
        bench_type: BenchmarkType::CrossParser,
        criterion_name: Some("yaml_parsers"),
        cli_subcommand: None,
        working_dir: "bench-compare",
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
#[allow(dead_code)]
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
        assert!(!filter_by_category(BenchmarkCategory::CrossParser).is_empty());
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
