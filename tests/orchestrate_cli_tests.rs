//! End-to-end CLI coverage for `succinctly bench orchestrate` (issue #98).
//!
//! These spawn the built binary directly (the `locate_cli_tests.rs` pattern).
//! Real multi-node runs over Tailscale SSH are manual-only — see
//! `docs/guides/benchmarking.md`'s "Distributed Benchmark Orchestration"
//! section for that runbook. What's covered here is everything that doesn't
//! need real network access: config validation, and the localhost path
//! (`SystemSsh`'s `is_local()` bypass runs commands directly with no `ssh`
//! involved), which is real orchestration plumbing end-to-end, not a mock.
#![cfg(feature = "bench-runner")]

use std::fs;
use std::path::Path;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_succinctly")
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn succinctly");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn write_config(dir: &Path, yaml: &str) -> String {
    let path = dir.join("nodes.yaml");
    fs::write(&path, yaml).unwrap();
    path.to_str().unwrap().to_string()
}

fn local_only_yaml(results_dir: &Path) -> String {
    format!(
        "coordinator:\n  results_dir: {}\nnodes:\n  - name: local\n    host: localhost\n    arch: x86_64\n",
        results_dir.display()
    )
}

#[test]
fn orchestrate_rejects_missing_config() {
    let (_, stderr, code) = run(&[
        "bench",
        "orchestrate",
        "--config",
        "/nonexistent/nodes.yaml",
        "--all",
    ]);

    assert_ne!(code, 0);
    assert!(stderr.contains("Failed to read config file"), "{stderr}");
}

#[test]
fn orchestrate_rejects_invalid_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let config = write_config(dir.path(), "not: [valid, yaml");

    let (_, stderr, code) = run(&["bench", "orchestrate", "--config", &config, "--all"]);

    assert_ne!(code, 0);
    assert!(stderr.contains("Failed to parse YAML config"), "{stderr}");
}

#[test]
fn orchestrate_rejects_unknown_node_selection() {
    let dir = tempfile::tempdir().unwrap();
    let results_dir = dir.path().join("results");
    let config = write_config(dir.path(), &local_only_yaml(&results_dir));

    let (_, stderr, code) = run(&[
        "bench",
        "orchestrate",
        "--config",
        &config,
        "--node",
        "nonexistent",
        "--all",
    ]);

    assert_ne!(code, 0);
    assert!(stderr.contains("No nodes selected"), "{stderr}");
}

#[test]
fn orchestrate_dry_run_prints_plan_without_side_effects() {
    let dir = tempfile::tempdir().unwrap();
    let results_dir = dir.path().join("results");
    let config = write_config(dir.path(), &local_only_yaml(&results_dir));

    let (stdout, stderr, code) = run(&[
        "bench",
        "orchestrate",
        "--config",
        &config,
        "--dry-run",
        "corpus_stats",
    ]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("Dry run"), "{stdout}");
    assert!(stdout.contains("corpus_stats"), "{stdout}");
    assert!(
        !results_dir.exists(),
        "dry-run must not create the results dir"
    );
}

/// Runs the real `SystemSsh` localhost path end-to-end: connectivity check,
/// benchmark exec, result download, aggregation, and metadata — all real
/// code, no `RemoteExec` fake. Whether the nested `bench run corpus_stats`
/// subprocess itself succeeds depends on whether a release binary happens to
/// be built already; either way the orchestration plumbing must complete
/// with exit 0 and produce its result files.
#[test]
fn orchestrate_local_only_node_runs_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let results_dir = dir.path().join("results");
    let config = write_config(dir.path(), &local_only_yaml(&results_dir));

    let (_, stderr, code) = run(&["bench", "orchestrate", "--config", &config, "corpus_stats"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stderr.contains("Orchestration run complete"), "{stderr}");

    let run_dirs: Vec<_> = fs::read_dir(&results_dir)
        .expect("results dir should exist")
        .filter_map(std::result::Result::ok)
        .collect();
    assert_eq!(run_dirs.len(), 1, "expected exactly one run directory");
    let run_dir = run_dirs[0].path();

    assert!(run_dir.join("metadata.json").exists());
    assert!(run_dir.join("results.jsonl").exists());
    assert!(run_dir.join("local/node_info.json").exists());
}
