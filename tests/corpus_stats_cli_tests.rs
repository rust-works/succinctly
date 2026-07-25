//! End-to-end CLI coverage for `succinctly dev bench corpus-stats` (#301).
//!
//! These spawn the built binary so the `main.rs` dispatch arm and the full
//! `run()` / `generate_report()` path are exercised through the real process
//! (and, under cargo-llvm-cov, counted). Gated on `cli` since the binary only
//! exists with that feature. The working directory for integration tests is the
//! crate root, so the seed corpus paths resolve as-is.
#![cfg(feature = "cli")]

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_succinctly")
}

#[test]
fn corpus_stats_check_matches_committed_golden() {
    let status = Command::new(bin())
        .args([
            "dev",
            "bench",
            "corpus-stats",
            "--data-dir",
            "tests/data/bench-corpus/seed",
            "--markdown",
            "tests/data/bench-corpus/expected-shape.md",
            "--check",
        ])
        .status()
        .expect("spawn succinctly");
    assert!(
        status.success(),
        "corpus-stats --check must pass against the committed golden"
    );
}

#[test]
fn corpus_stats_prints_report_to_stdout() {
    let out = Command::new(bin())
        .args([
            "dev",
            "bench",
            "corpus-stats",
            "--data-dir",
            "tests/data/bench-corpus/seed",
        ])
        .output()
        .expect("spawn succinctly");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Real-Workload Corpus"));
    assert!(stdout.contains("## DSV"));
}
