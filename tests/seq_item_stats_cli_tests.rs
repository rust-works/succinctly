//! End-to-end CLI coverage for `succinctly dev bench seq-item-stats` (#106).
//!
//! Spawns the built binary so the `main.rs` dispatch arm and the full
//! `run_all()` path are exercised through the real process (and, under
//! cargo-llvm-cov, counted) — the same approach as
//! `tests/corpus_stats_cli_tests.rs`. Gated on `cli` since the binary only
//! exists with that feature. The working directory for integration tests is the
//! crate root, so the seed corpus paths resolve as-is.
#![cfg(feature = "cli")]

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_succinctly")
}

#[test]
fn seq_item_stats_reports_on_the_seed_corpus() {
    let out = Command::new(bin())
        .args([
            "dev",
            "bench",
            "seq-item-stats",
            "--data-dir",
            "tests/data/bench-corpus/seed",
        ])
        .output()
        .expect("failed to run seq-item-stats");

    assert!(
        out.status.success(),
        "exit {:?}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let report = String::from_utf8_lossy(&out.stdout);
    for heading in [
        "# Seq-Item Density and Position Storage",
        "## Seq-item density",
        "## Predicate vs structure",
        "## Position storage: Compact vs Dense",
        "## Option 3 sizing",
        "## Verdict",
    ] {
        assert!(report.contains(heading), "missing {heading} in:\n{report}");
    }
    // The seed corpus is well-formed, so both correctness invariants are clean.
    assert!(
        report.contains("predicate is exact"),
        "expected a clean invariant verdict:\n{report}"
    );
}

#[test]
fn seq_item_stats_writes_markdown_and_jsonl() {
    let tmp = tempfile::tempdir().unwrap();
    let md = tmp.path().join("report.md");
    let jsonl = tmp.path().join("records.jsonl");

    let out = Command::new(bin())
        .args([
            "dev",
            "bench",
            "seq-item-stats",
            "--data-dir",
            "tests/data/bench-corpus/seed",
            "--markdown",
            md.to_str().unwrap(),
            "--output",
            jsonl.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run seq-item-stats");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        std::fs::read_to_string(&md)
            .unwrap()
            .contains("## Seq-item density"),
        "markdown report not written"
    );

    let records = std::fs::read_to_string(&jsonl).unwrap();
    let n = records.lines().filter(|l| !l.trim().is_empty()).count();
    assert!(n > 0, "expected per-file JSONL records");
    for line in records.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value = serde_json::from_str(line).expect("record is JSON");
        for key in ["workload", "file", "opens", "seq_items", "open_compact"] {
            assert!(v.get(key).is_some(), "record missing {key}: {line}");
        }
    }
}

#[test]
fn seq_item_stats_scans_the_vendored_yaml_test_suite() {
    // The suite is the only corpus in the repo that reaches the
    // `OpenPositions::Dense` fallback, so this exercises that reporting branch.
    let out = Command::new(bin())
        .args([
            "dev",
            "bench",
            "seq-item-stats",
            "--data-dir",
            "tests/data/bench-corpus/seed",
            "--yaml-test-suite",
            "tests/data/yaml-test-suite-2022-01-17.json",
        ])
        .output()
        .expect("failed to run seq-item-stats");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report = String::from_utf8_lossy(&out.stdout);
    assert!(report.contains("YAML Test Suite:"), "suite section missing");
    assert!(
        report.contains("OpenPositions::Dense:"),
        "Dense split not reported"
    );
    assert!(
        report.contains("predicate/structure mismatches: 0 case(s)"),
        "the suite should hold both invariants:\n{report}"
    );
}

#[test]
fn seq_item_stats_explain_mismatch_mode_runs() {
    let out = Command::new(bin())
        .args([
            "dev",
            "bench",
            "seq-item-stats",
            "--data-dir",
            "tests/data/bench-corpus/seed",
            "--explain-mismatch",
        ])
        .output()
        .expect("failed to run seq-item-stats");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // One header per YAML file scanned, and no rows, since the seed corpus is clean.
    let report = String::from_utf8_lossy(&out.stdout);
    assert!(
        report.contains("==="),
        "expected per-file headers:\n{report}"
    );
    assert!(
        !report.contains("text_pred="),
        "seed corpus should have no mismatching nodes:\n{report}"
    );
}
