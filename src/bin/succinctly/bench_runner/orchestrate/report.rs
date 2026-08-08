//! Cross-node/cross-architecture reporting (Phase 5, issue #98).
//!
//! Pure, SSH-free, offline: reads the `summary.json` each `bench run`
//! invocation already writes under `<run_dir>/<node>/<benchmark>/`, plus
//! each node's `node_info.json` (for `arch`), and builds a markdown report
//! comparing benchmarks across nodes and architectures. Optionally flags
//! regressions against a prior run of the same shape via `--baseline`.
//! Deliberately separate from `aggregate.rs`: aggregation is a step inside
//! `bench orchestrate`'s SSH-driven run; reporting needs neither `nodes.yaml`
//! nor SSH, so it's its own standalone `bench report` subcommand.

use super::config::Architecture;
use crate::bench_runner::runner::RunSummary;
use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// `succinctly bench report` — compare orchestrated benchmark results
/// across nodes/architectures, optionally against a prior baseline run.
#[derive(Debug, Parser)]
pub struct ReportArgs {
    /// Run directory to report on (e.g. data/bench/distributed/<run_id>)
    #[arg(short = 'c', long = "current")]
    pub current: PathBuf,

    /// A prior run directory of the same shape, to flag regressions against
    #[arg(short, long)]
    pub baseline: Option<PathBuf>,

    /// Regression threshold as a fraction (0.10 = flag anything >10% slower)
    #[arg(long, default_value_t = 0.10)]
    pub threshold: f64,

    /// Where to write the markdown report (default: <current>/summary.md)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Check the report matches the file at the output path instead of
    /// writing it (exits non-zero on mismatch or if the file is missing)
    #[arg(long)]
    pub check: bool,
}

/// Run `bench report`.
pub fn run_report(args: ReportArgs) -> Result<()> {
    let current = collect_runs(&args.current)
        .with_context(|| format!("Failed to collect results from {}", args.current.display()))?;
    if current.is_empty() {
        anyhow::bail!(
            "No summary.json files found under {}",
            args.current.display()
        );
    }

    let baseline = args.baseline.as_deref().map(collect_runs).transpose()?;
    let report = render_report(&current, baseline.as_deref(), args.threshold);

    let output_path = args
        .output
        .clone()
        .unwrap_or_else(|| args.current.join("summary.md"));

    if args.check {
        let existing = fs::read_to_string(&output_path)
            .with_context(|| format!("Failed to read {}", output_path.display()))?;
        if existing != report {
            anyhow::bail!(
                "{} is out of date; run `bench report` (without --check) to regenerate",
                output_path.display()
            );
        }
        eprintln!("{} is up to date", output_path.display());
        return Ok(());
    }

    fs::write(&output_path, &report)
        .with_context(|| format!("Failed to write {}", output_path.display()))?;
    eprintln!("Wrote {}", output_path.display());
    Ok(())
}

/// One benchmark's result on one node, joined with that node's architecture.
#[derive(Debug, Clone)]
struct BenchmarkRun {
    node: String,
    arch: Option<Architecture>,
    benchmark: String,
    duration_seconds: f64,
    success: bool,
}

#[derive(Deserialize)]
struct NodeInfo {
    #[serde(default)]
    arch: Option<Architecture>,
}

/// Walk `run_dir`'s immediate node subdirectories, reading each one's
/// `node_info.json` (for `arch`) and every `summary.json` found underneath
/// (recursively — one per `bench run <name>` invocation).
fn collect_runs(run_dir: &Path) -> Result<Vec<BenchmarkRun>> {
    let mut runs = Vec::new();
    let entries =
        fs::read_dir(run_dir).with_context(|| format!("Failed to read {}", run_dir.display()))?;

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue; // skip results.jsonl / metadata.json / summary.md
        }
        let node_dir = entry.path();
        let node_name = entry.file_name().to_string_lossy().to_string();

        let arch = fs::read_to_string(node_dir.join("node_info.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<NodeInfo>(&text).ok())
            .and_then(|info| info.arch);

        for summary_path in find_files_named(&node_dir, "summary.json") {
            let text = fs::read_to_string(&summary_path)
                .with_context(|| format!("Failed to read {}", summary_path.display()))?;
            let summary: RunSummary = serde_json::from_str(&text)
                .with_context(|| format!("Failed to parse {}", summary_path.display()))?;
            for result in summary.results {
                runs.push(BenchmarkRun {
                    node: node_name.clone(),
                    arch,
                    benchmark: result.name,
                    duration_seconds: result.duration_seconds,
                    success: result.success,
                });
            }
        }
    }

    Ok(runs)
}

/// Recursively collect every file under `dir` named exactly `filename`.
fn find_files_named(dir: &Path, filename: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            found.extend(find_files_named(&path, filename));
        } else if path.file_name().and_then(|n| n.to_str()) == Some(filename) {
            found.push(path);
        }
    }
    found
}

fn render_report(
    current: &[BenchmarkRun],
    baseline: Option<&[BenchmarkRun]>,
    threshold: f64,
) -> String {
    let mut out = String::from("# Orchestrated Benchmark Report\n\n");
    render_per_benchmark_table(&mut out, current);
    render_per_arch_table(&mut out, current);
    if let Some(baseline) = baseline {
        render_regressions(&mut out, current, baseline, threshold);
    }
    out
}

fn sorted_unique_nodes(runs: &[BenchmarkRun]) -> Vec<&str> {
    let mut nodes: Vec<&str> = runs.iter().map(|r| r.node.as_str()).collect();
    nodes.sort_unstable();
    nodes.dedup();
    nodes
}

fn render_per_benchmark_table(out: &mut String, runs: &[BenchmarkRun]) {
    out.push_str("## Per-Benchmark Comparison\n\n");

    let mut by_benchmark: BTreeMap<&str, Vec<&BenchmarkRun>> = BTreeMap::new();
    for run in runs {
        by_benchmark
            .entry(run.benchmark.as_str())
            .or_default()
            .push(run);
    }
    let nodes = sorted_unique_nodes(runs);

    out.push_str("| Benchmark |");
    for node in &nodes {
        out.push_str(&format!(" {node} |"));
    }
    out.push_str("\n|---|");
    for _ in &nodes {
        out.push_str("---|");
    }
    out.push('\n');

    for (benchmark, entries) in &by_benchmark {
        out.push_str(&format!("| {benchmark} |"));
        for node in &nodes {
            match entries.iter().find(|r| r.node == *node) {
                Some(r) if r.success => out.push_str(&format!(" {:.3}s |", r.duration_seconds)),
                Some(_) => out.push_str(" FAILED |"),
                None => out.push_str(" - |"),
            }
        }
        out.push('\n');
    }
    out.push('\n');
}

fn render_per_arch_table(out: &mut String, runs: &[BenchmarkRun]) {
    out.push_str("## Per-Architecture Average Duration\n\n");
    out.push_str("| Architecture | Avg Duration (s) | Benchmarks |\n|---|---|---|\n");

    let mut by_arch: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for run in runs.iter().filter(|r| r.success) {
        let key = run
            .arch
            .map_or_else(|| "unknown".to_string(), |a| a.to_string());
        by_arch.entry(key).or_default().push(run.duration_seconds);
    }

    for (arch, durations) in &by_arch {
        let avg = durations.iter().sum::<f64>() / durations.len() as f64;
        out.push_str(&format!("| {arch} | {avg:.3} | {} |\n", durations.len()));
    }
    out.push('\n');
}

fn render_regressions(
    out: &mut String,
    current: &[BenchmarkRun],
    baseline: &[BenchmarkRun],
    threshold: f64,
) {
    out.push_str(&format!(
        "## Regressions (> {:.0}% slower than baseline)\n\n",
        threshold * 100.0
    ));

    let mut flagged = Vec::new();
    for cur in current.iter().filter(|r| r.success) {
        let Some(base) = baseline
            .iter()
            .find(|b| b.success && b.node == cur.node && b.benchmark == cur.benchmark)
        else {
            continue;
        };
        if base.duration_seconds <= 0.0 {
            continue;
        }
        let change = (cur.duration_seconds - base.duration_seconds) / base.duration_seconds;
        if change > threshold {
            flagged.push((
                &cur.node,
                &cur.benchmark,
                base.duration_seconds,
                cur.duration_seconds,
                change,
            ));
        }
    }

    if flagged.is_empty() {
        out.push_str("No regressions detected.\n\n");
        return;
    }

    out.push_str(
        "| Node | Benchmark | Baseline (s) | Current (s) | Change |\n|---|---|---|---|---|\n",
    );
    for (node, benchmark, base, cur, change) in flagged {
        out.push_str(&format!(
            "| {node} | {benchmark} | {base:.3} | {cur:.3} | +{:.1}% |\n",
            change * 100.0
        ));
    }
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench_runner::runner::BenchmarkResult;
    use tempfile::tempdir;

    fn write_summary(dir: &Path, node: &str, benchmark: &str, duration: f64, success: bool) {
        let out_dir = dir.join(node).join(benchmark);
        fs::create_dir_all(&out_dir).unwrap();
        let summary = RunSummary {
            metadata: None,
            results: vec![BenchmarkResult {
                name: benchmark.to_string(),
                category: "core".to_string(),
                bench_type: "criterion".to_string(),
                success,
                duration_seconds: duration,
                exit_code: Some(i32::from(!success)),
                error_message: None,
            }],
            total_duration_seconds: duration,
            passed: usize::from(success),
            failed: usize::from(!success),
            skipped: 0,
        };
        fs::write(
            out_dir.join("summary.json"),
            serde_json::to_string(&summary).unwrap(),
        )
        .unwrap();
    }

    fn write_node_info(dir: &Path, node: &str, arch: Architecture) {
        fs::write(
            dir.join(node).join("node_info.json"),
            format!(r#"{{"name":"{node}","arch":"{arch}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn collects_nested_summaries_and_joins_arch() {
        let dir = tempdir().unwrap();
        write_summary(dir.path(), "local", "rank_select", 1.5, true);
        write_node_info(dir.path(), "local", Architecture::X86_64);

        let runs = collect_runs(dir.path()).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].node, "local");
        assert_eq!(runs[0].benchmark, "rank_select");
        assert_eq!(runs[0].arch, Some(Architecture::X86_64));
    }

    #[test]
    fn per_benchmark_table_lists_all_nodes_even_when_missing() {
        let dir = tempdir().unwrap();
        write_summary(dir.path(), "local", "rank_select", 1.0, true);
        write_node_info(dir.path(), "local", Architecture::X86_64);
        write_summary(dir.path(), "sydney", "yaml_bench", 2.0, true);
        write_node_info(dir.path(), "sydney", Architecture::Aarch64);

        let runs = collect_runs(dir.path()).unwrap();
        let report = render_report(&runs, None, 0.10);

        assert!(report.contains("| rank_select | 1.000s | - |"));
        assert!(report.contains("| yaml_bench | - | 2.000s |"));
    }

    #[test]
    fn per_arch_table_averages_across_nodes() {
        let dir = tempdir().unwrap();
        write_summary(dir.path(), "a", "x", 1.0, true);
        write_node_info(dir.path(), "a", Architecture::X86_64);
        write_summary(dir.path(), "b", "y", 3.0, true);
        write_node_info(dir.path(), "b", Architecture::X86_64);

        let runs = collect_runs(dir.path()).unwrap();
        let report = render_report(&runs, None, 0.10);

        assert!(report.contains("| x86_64 | 2.000 | 2 |"));
    }

    #[test]
    fn failed_benchmarks_are_excluded_from_arch_average_but_shown_in_table() {
        let dir = tempdir().unwrap();
        write_summary(dir.path(), "a", "x", 1.0, false);
        write_node_info(dir.path(), "a", Architecture::X86_64);

        let runs = collect_runs(dir.path()).unwrap();
        let report = render_report(&runs, None, 0.10);

        assert!(report.contains("| x | FAILED |"));
        assert!(!report.contains("x86_64 | 1.000"));
    }

    #[test]
    fn regression_flagged_when_over_threshold() {
        let baseline_dir = tempdir().unwrap();
        write_summary(baseline_dir.path(), "local", "rank_select", 1.0, true);
        write_node_info(baseline_dir.path(), "local", Architecture::X86_64);
        let baseline = collect_runs(baseline_dir.path()).unwrap();

        let current_dir = tempdir().unwrap();
        write_summary(current_dir.path(), "local", "rank_select", 1.5, true);
        write_node_info(current_dir.path(), "local", Architecture::X86_64);
        let current = collect_runs(current_dir.path()).unwrap();

        let report = render_report(&current, Some(&baseline), 0.10);
        assert!(report.contains("rank_select"));
        assert!(report.contains("+50.0%"));
    }

    #[test]
    fn no_regression_reported_within_threshold() {
        let baseline_dir = tempdir().unwrap();
        write_summary(baseline_dir.path(), "local", "rank_select", 1.0, true);
        write_node_info(baseline_dir.path(), "local", Architecture::X86_64);
        let baseline = collect_runs(baseline_dir.path()).unwrap();

        let current_dir = tempdir().unwrap();
        write_summary(current_dir.path(), "local", "rank_select", 1.02, true);
        write_node_info(current_dir.path(), "local", Architecture::X86_64);
        let current = collect_runs(current_dir.path()).unwrap();

        let report = render_report(&current, Some(&baseline), 0.10);
        assert!(report.contains("No regressions detected."));
    }

    #[test]
    fn check_mode_detects_drift() {
        let dir = tempdir().unwrap();
        write_summary(dir.path(), "local", "rank_select", 1.0, true);
        write_node_info(dir.path(), "local", Architecture::X86_64);

        let output = dir.path().join("summary.md");
        let args = ReportArgs {
            current: dir.path().to_path_buf(),
            baseline: None,
            threshold: 0.10,
            output: Some(output.clone()),
            check: false,
        };
        run_report(args).unwrap();
        let first_write = fs::read_to_string(&output).unwrap();

        // --check against the file we just wrote: matches, succeeds.
        let check_args = ReportArgs {
            current: dir.path().to_path_buf(),
            baseline: None,
            threshold: 0.10,
            output: Some(output.clone()),
            check: true,
        };
        run_report(check_args).unwrap();

        // Now drift the committed file and confirm --check catches it.
        fs::write(&output, format!("{first_write}stale\n")).unwrap();
        let check_args = ReportArgs {
            current: dir.path().to_path_buf(),
            baseline: None,
            threshold: 0.10,
            output: Some(output.clone()),
            check: true,
        };
        assert!(run_report(check_args).is_err());
    }
}
