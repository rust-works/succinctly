//! End-to-end CLI coverage for `jq-locate` / `yq-locate` `--line/--column`.
//!
//! The line/column -> byte offset path had no integration coverage at all
//! before #228 — only help-text snapshots mentioned these commands — so the
//! `LineIndex` migration would have been invisible to the test suite at the
//! CLI boundary. These spawn the built binary directly (the
//! `corpus_stats_cli_tests.rs` pattern) rather than going through `cargo run`.
//!
//! The working directory for integration tests is the crate root, so the seed
//! corpus paths resolve as-is.
#![cfg(feature = "cli")]

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

const JSON_FIXTURE: &str = "tests/data/bench-corpus/seed/json/charts/bullet-data.json";
const YAML_FIXTURE: &str = "tests/data/bench-corpus/seed/yaml/k8s/nginx-deployment.yml";

#[test]
fn jq_locate_resolves_line_and_column() {
    let (stdout, stderr, code) = run(&["jq-locate", JSON_FIXTURE, "--line", "3", "--column", "5"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, ".[1].title");
}

#[test]
fn jq_locate_line_column_agrees_with_offset() {
    // Line 3 column 5 of the fixture is byte 122. The two paths share nothing:
    // --offset skips the line index entirely, so agreement is a real check
    // that the line index maps positions the way the file is actually laid out.
    let (by_position, _, _) = run(&["jq-locate", JSON_FIXTURE, "--line", "3", "--column", "5"]);
    let (by_offset, _, _) = run(&["jq-locate", JSON_FIXTURE, "--offset", "122"]);

    assert_eq!(by_position, by_offset);
    assert!(!by_position.is_empty());
}

#[test]
fn jq_locate_rejects_out_of_range_position() {
    let (_, stderr, code) = run(&["jq-locate", JSON_FIXTURE, "--line", "9999", "--column", "1"]);

    assert_ne!(code, 0, "out-of-range position must fail");
    assert!(
        stderr.contains("Invalid position"),
        "expected an invalid-position error, got: {stderr}"
    );
}

#[test]
fn jq_locate_rejects_zero_column() {
    let (_, stderr, code) = run(&["jq-locate", JSON_FIXTURE, "--line", "1", "--column", "0"]);

    assert_ne!(code, 0, "column 0 must fail");
    assert!(
        stderr.contains("Invalid position"),
        "expected an invalid-position error, got: {stderr}"
    );
}

#[test]
fn yq_locate_resolves_line_and_column() {
    // Line 5 of the manifest is "  labels:", so column 3 is the key itself.
    let (stdout, stderr, code) = run(&["yq-locate", YAML_FIXTURE, "--line", "5", "--column", "3"]);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, ".[0].metadata.labels");
}

#[test]
fn yq_locate_rejects_out_of_range_position() {
    let (_, stderr, code) = run(&["yq-locate", YAML_FIXTURE, "--line", "9999", "--column", "1"]);

    assert_ne!(code, 0, "out-of-range position must fail");
    assert!(
        stderr.contains("Invalid position"),
        "expected an invalid-position error, got: {stderr}"
    );
}
