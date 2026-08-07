//! Naive result aggregation for Phase 1: concatenate each node's
//! per-benchmark `*.jsonl` files into one combined `results.jsonl`, plus a
//! run-level `metadata.json`. Per-node directories (and their `node_info.json`)
//! are left in place so Phase 5's `report.rs` can build per-architecture
//! comparisons directly from them without a schema change here.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Run-level metadata written alongside the aggregated results.
#[derive(Debug, Clone, Serialize)]
pub struct RunMetadata {
    pub run_id: String,
    pub git_commit: Option<String>,
    pub config_path: String,
    pub nodes: Vec<String>,
    pub benchmarks: Vec<String>,
    pub started_at: String,
    pub completed_at: String,
}

/// Concatenate every node's `*.jsonl` result file under `run_dir` into a
/// single `results.jsonl`. A node directory that's missing or has no JSONL
/// files is warned about, not treated as fatal — partial results are still
/// useful. Each `bench run <name>` invocation gets its own
/// `<node>/<benchmark>/` subdirectory, so this searches recursively rather
/// than assuming `*.jsonl` sits directly under the node directory.
pub fn aggregate_results(run_dir: &Path, node_names: &[String]) -> Result<PathBuf> {
    let combined_path = run_dir.join("results.jsonl");
    let mut combined = fs::File::create(&combined_path)
        .with_context(|| format!("Failed to create {}", combined_path.display()))?;

    for node_name in node_names {
        let node_dir = run_dir.join(node_name);
        if !node_dir.is_dir() {
            eprintln!("Warning: no results directory for node '{node_name}', skipping");
            continue;
        }

        let mut jsonl_files = find_files_with_extension(&node_dir, "jsonl");
        jsonl_files.sort();

        if jsonl_files.is_empty() {
            eprintln!("Warning: node '{node_name}' produced no .jsonl result files");
            continue;
        }

        for path in jsonl_files {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            for line in content.lines() {
                if !line.trim().is_empty() {
                    writeln!(combined, "{line}")?;
                }
            }
        }
    }

    Ok(combined_path)
}

/// Recursively collect every file under `dir` whose extension is `ext`.
/// An unreadable `dir` yields an empty result rather than an error — callers
/// treat "no files found" as a normal, warn-worthy condition already.
pub(crate) fn find_files_with_extension(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(find_files_with_extension(&path, ext));
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            found.push(path);
        }
    }
    found
}

/// Write the run-level `metadata.json`.
pub fn write_run_metadata(run_dir: &Path, meta: &RunMetadata) -> Result<PathBuf> {
    let path = run_dir.join("metadata.json");
    let json = serde_json::to_string_pretty(meta)?;
    fs::write(&path, json).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn merges_jsonl_nested_under_per_benchmark_subdirectories() {
        // Each `bench run <name> --output-dir <node>/<name>/` invocation
        // writes its .jsonl one level under the node dir, not directly in
        // it — this is the real shape `collect_node_results` downloads.
        let dir = tempdir().unwrap();
        let run_dir = dir.path();

        fs::create_dir_all(run_dir.join("local/rank_select")).unwrap();
        fs::write(
            run_dir.join("local/rank_select/rank_select.jsonl"),
            "{\"a\":1}\n",
        )
        .unwrap();
        fs::create_dir_all(run_dir.join("sydney/rank_select")).unwrap();
        fs::write(
            run_dir.join("sydney/rank_select/rank_select.jsonl"),
            "{\"a\":2}\n\n",
        )
        .unwrap();

        let combined =
            aggregate_results(run_dir, &["local".to_string(), "sydney".to_string()]).unwrap();

        let content = fs::read_to_string(combined).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines, vec!["{\"a\":1}", "{\"a\":2}"]);
    }

    #[test]
    fn missing_node_dir_is_skipped_not_fatal() {
        let dir = tempdir().unwrap();
        let run_dir = dir.path();
        fs::create_dir_all(run_dir.join("local")).unwrap();
        fs::write(run_dir.join("local/a.jsonl"), "{}\n").unwrap();

        let combined =
            aggregate_results(run_dir, &["local".to_string(), "missing".to_string()]).unwrap();

        let content = fs::read_to_string(combined).unwrap();
        assert_eq!(content.lines().count(), 1);
    }

    #[test]
    fn writes_run_metadata() {
        let dir = tempdir().unwrap();
        let meta = RunMetadata {
            run_id: "2026-01-01T00-00-00".to_string(),
            git_commit: Some("abc123".to_string()),
            config_path: "nodes.yaml".to_string(),
            nodes: vec!["local".to_string()],
            benchmarks: vec!["rank_select".to_string()],
            started_at: "2026-01-01T00:00:00Z".to_string(),
            completed_at: "2026-01-01T00:01:00Z".to_string(),
        };
        let path = write_run_metadata(dir.path(), &meta).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("abc123"));
        assert!(content.contains("rank_select"));
    }
}
