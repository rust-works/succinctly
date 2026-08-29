//! Golden tests for the succinctly CLI tool
//!
//! These tests use snapshot testing to ensure CLI outputs remain stable.
//! Run with: cargo test --features cli --test cli_golden_tests

use anyhow::Result;
use std::process::{Command, Stdio};

#[path = "common/cargo_run_exit.rs"]
mod cargo_run_exit;
use cargo_run_exit::signal_death_error;

/// Helper to run a CLI command and capture its output. A thin wrapper over
/// `run_cli_bin`'s own already-correct approach (#1847): this used to spawn
/// a *second* `cargo run` subprocess, which orphans the real `succinctly`
/// grandchild -- reparented to init, blocked forever on its now-unreadable
/// stdout pipe -- the moment anything kills the outer `cargo test` process
/// group (`cargo-guard.sh` does this by design on a detected stall, #935).
/// `CARGO_BIN_EXE_succinctly` needs no retry loop either: it's a
/// compile-time path to the binary this very test binary's own build
/// already produced, not a second cargo invocation that can hit lock
/// contention (`MAX_CARGO_RETRIES`/`classify_cargo_run_exit` accordingly
/// dropped, not just unused).
fn run_cli(args: &[&str]) -> Result<String> {
    let (stdout, stderr, exit_code) = run_cli_bin(args)?;
    if exit_code != 0 {
        anyhow::bail!("Command failed: {stderr}");
    }
    Ok(stdout)
}

#[test]
fn test_help_main() -> Result<()> {
    let output = run_cli(&["--help"])?;
    insta::assert_snapshot!("help_main", output);
    Ok(())
}

#[test]
fn test_help_json() -> Result<()> {
    let output = run_cli(&["json", "--help"])?;
    insta::assert_snapshot!("help_json", output);
    Ok(())
}

#[test]
fn test_help_json_generate() -> Result<()> {
    let output = run_cli(&["json", "generate", "--help"])?;
    insta::assert_snapshot!("help_json_generate", output);
    Ok(())
}

#[test]
fn test_help_install_aliases() -> Result<()> {
    let output = run_cli(&["install-aliases", "--help"])?;
    insta::assert_snapshot!("help_install_aliases", output);
    Ok(())
}

#[test]
fn test_version() -> Result<()> {
    let output = run_cli(&["--version"])?;
    insta::assert_snapshot!("version", output);
    Ok(())
}

#[test]
fn test_json_generate_small() -> Result<()> {
    // Generate a small JSON (100 bytes) for deterministic testing
    let output = run_cli(&["json", "generate", "100", "--seed", "42"])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    // Snapshot the output
    insta::assert_snapshot!("json_generate_100b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_comprehensive_1kb() -> Result<()> {
    // Generate 1KB comprehensive pattern with seed for reproducibility
    let output = run_cli(&[
        "json",
        "generate",
        "1kb",
        "--pattern",
        "comprehensive",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    // Snapshot the output
    insta::assert_snapshot!("json_generate_comprehensive_1kb_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_users_1kb() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "1kb",
        "--pattern",
        "users",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_users_1kb_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_nested() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "nested",
        "--depth",
        "3",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_nested_500b_depth3_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_arrays() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "arrays",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_arrays_500b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_mixed() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "mixed",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_mixed_500b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_strings() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "strings",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_strings_500b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_numbers() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "numbers",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_numbers_500b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_literals() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "literals",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_literals_500b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_unicode() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "1kb",
        "--pattern",
        "unicode",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_unicode_1kb_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_pathological() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "pathological",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_pathological_500b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_wide() -> Result<()> {
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "wide",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON, and a flat object with many top-level keys -
    // the whole point of this pattern (see generators.rs).
    let value: serde_json::Value = serde_json::from_str(&output)?;
    let obj = value.as_object().expect("wide pattern must be an object");
    assert!(
        obj.len() > 1,
        "wide pattern should have multiple top-level keys, got {}",
        obj.len()
    );

    insta::assert_snapshot!("json_generate_wide_500b_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_escape_density() -> Result<()> {
    // Test with higher escape density
    let output = run_cli(&[
        "json",
        "generate",
        "500",
        "--pattern",
        "strings",
        "--escape-density",
        "0.5",
        "--seed",
        "42",
    ])?;

    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&output)?;

    insta::assert_snapshot!("json_generate_strings_escape_density_0_5_seed42", output);
    Ok(())
}

#[test]
fn test_json_generate_reproducible() -> Result<()> {
    // Verify same seed produces identical output
    let output1 = run_cli(&["json", "generate", "1kb", "--seed", "12345"])?;
    let output2 = run_cli(&["json", "generate", "1kb", "--seed", "12345"])?;

    assert_eq!(
        output1, output2,
        "Same seed should produce identical output"
    );

    // Different seed should produce different output
    let output3 = run_cli(&["json", "generate", "1kb", "--seed", "54321"])?;
    assert_ne!(
        output1, output3,
        "Different seed should produce different output"
    );

    Ok(())
}

/// Runs the pre-built `succinctly` binary directly (unlike `run_cli` above, which
/// spawns a second, uninstrumented binary via `cargo run` and so is invisible to
/// `cargo llvm-cov`) -- needed for #1212's own coverage, since its
/// `validate_generated_json` consolidation and both call sites are otherwise
/// unexercised by any existing test in this file.
fn run_cli_bin(args: &[&str]) -> Result<(String, String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // #1546: `.code().unwrap_or(-1)` would coerce a signal-killed child to a
    // fake exit code -1 rather than reporting the death.
    let Some(exit_code) = output.status.code() else {
        return Err(signal_death_error(output.status, &stderr));
    };
    Ok((stdout, stderr, exit_code))
}

/// #1212: `--verify` alone validates and discards the parsed tree (the
/// `keep_parsed = false` path through `validate_generated_json`).
#[test]
fn test_json_generate_verify_only_1212() -> Result<()> {
    let (stdout, stderr, code) = run_cli_bin(&["json", "generate", "1kb", "--verify"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stderr.contains("JSON validated successfully"), "{stderr}");
    // No --pretty, no -o: output goes to stdout unmodified.
    serde_json::from_str::<serde_json::Value>(stdout.trim())?;
    Ok(())
}

/// #1212: `--pretty` alone (no `--verify`) takes the `None` arm of
/// `validate_generated_json`'s reuse match -- its own independent parse, not the
/// validation pass's (there is none).
#[test]
fn test_json_generate_pretty_only_1212() -> Result<()> {
    let (stdout, stderr, code) = run_cli_bin(&["json", "generate", "1kb", "--pretty"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(!stderr.contains("validated"), "{stderr}");
    assert!(
        stdout.starts_with("{\n"),
        "expected pretty-printed JSON, got: {stdout:?}"
    );
    serde_json::from_str::<serde_json::Value>(&stdout)?;
    Ok(())
}

/// #1212: `--verify --pretty` together is the one combination that reuses
/// `validate_generated_json`'s own parsed tree (`keep_parsed = true`, the `Some`
/// arm) instead of parsing the generated JSON a second time.
#[test]
fn test_json_generate_verify_and_pretty_1212() -> Result<()> {
    let (stdout, stderr, code) = run_cli_bin(&["json", "generate", "1kb", "--verify", "--pretty"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stderr.contains("JSON validated successfully"), "{stderr}");
    assert!(
        stdout.starts_with("{\n"),
        "expected pretty-printed JSON, got: {stdout:?}"
    );
    serde_json::from_str::<serde_json::Value>(&stdout)?;
    Ok(())
}

/// #1212: `generate-suite --verify`'s own, separately-worded call site
/// (`validate_generated_json(&json, false)` with a per-file error context) --
/// keeps everything within `--max-size` tiny so the suite finishes quickly.
#[test]
fn test_json_generate_suite_verify_1212() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (_, stderr, code) = run_cli_bin(&[
        "json",
        "generate-suite",
        "--verify",
        "--max-size",
        "2kb",
        "--output-dir",
        dir.path().to_str().expect("tempdir path is valid UTF-8"),
    ])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("All files validated successfully"),
        "{stderr}"
    );
    Ok(())
}
