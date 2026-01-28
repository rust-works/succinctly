//! Golden tests for the succinctly CLI tool
//!
//! These tests use snapshot testing to ensure CLI outputs remain stable.
//! Run with: cargo test --features cli --test cli_golden_tests

use anyhow::Result;
use std::process::Command;
use std::time::Duration;

/// Maximum retries for cargo run commands that fail with exit code 101.
/// This handles flaky failures from cargo lock contention when tests run in parallel.
const MAX_CARGO_RETRIES: u32 = 3;

/// Helper to run a CLI command and capture its output
fn run_cli(args: &[&str]) -> Result<String> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let output = Command::new("cargo")
            .args(["run", "--features", "cli", "--bin", "succinctly", "--"])
            .args(args)
            .output()?;

        let exit_code = output.status.code().unwrap_or(-1);

        // Exit code 101 often indicates cargo lock contention; retry
        if exit_code == 101 && attempt + 1 < MAX_CARGO_RETRIES {
            std::thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
            continue;
        }

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Command failed: {}", stderr);
        }

        return Ok(String::from_utf8(output.stdout)?);
    }
    unreachable!()
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
