//! Integration tests for the `succinctly yaml validate` CLI command.
//!
//! Run with: cargo test --features cli --test yaml_validate_tests

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::Result;
use tempfile::NamedTempFile;

/// Resolve the path to the pre-built `succinctly` CLI binary, building it once.
/// Mirrors `tests/json_validate_tests.rs` — the integration harness is compiled
/// without `cli`, so we build the binary and derive its path from this test
/// executable's own location.
fn succinctly_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let output = Command::new(cargo)
            .args(["build", "--features", "cli", "--bin", "succinctly"])
            .output()
            .expect("failed to spawn `cargo build`");
        assert!(
            output.status.success(),
            "`cargo build --features cli --bin succinctly` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut path = std::env::current_exe().expect("resolve current_exe");
        path.pop();
        if path.file_name().and_then(|s| s.to_str()) == Some("deps") {
            path.pop();
        }
        path.push(format!("succinctly{}", std::env::consts::EXE_SUFFIX));
        assert!(
            path.is_file(),
            "built `succinctly` binary not found at {}",
            path.display()
        );
        path
    })
}

fn run_validate_stdin(input: &str, extra_args: &[&str]) -> Result<(String, String, i32)> {
    let mut cmd = Command::new(succinctly_bin())
        .args(["yaml", "validate"])
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }
    let output = cmd.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

fn run_validate_file(path: &str, extra_args: &[&str]) -> Result<(String, String, i32)> {
    let output = Command::new(succinctly_bin())
        .args(["yaml", "validate"])
        .args(extra_args)
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn valid_yaml_exits_zero() -> Result<()> {
    let (stdout, stderr, code) = run_validate_stdin("a: 1\nb: 2\n", &[])?;
    assert_eq!(code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(stdout.is_empty());
    Ok(())
}

#[test]
fn invalid_yaml_exits_one() -> Result<()> {
    // Nested mapping key `a: b: c` — rejected by the validator, accepted by the
    // default loader.
    let (_, stderr, code) = run_validate_stdin("a: b: c: d\n", &["--no-color"])?;
    assert_eq!(code, 1);
    assert!(stderr.contains("nested mapping key"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn quiet_mode_is_silent() -> Result<()> {
    let (stdout, stderr, code) = run_validate_stdin("foo: |0\n", &["--quiet"])?;
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(
        stderr.is_empty(),
        "stderr should be empty in quiet mode: {stderr}"
    );
    Ok(())
}

#[test]
fn error_output_has_rustc_style_caret() -> Result<()> {
    let (_, stderr, code) = run_validate_stdin("---\n\"\\.\"\n", &["--no-color"])?;
    assert_eq!(code, 1);
    assert!(
        stderr.contains("error: invalid escape sequence"),
        "stderr: {stderr}"
    );
    assert!(
        stderr.contains("--> <stdin>:2:"),
        "location line missing: {stderr}"
    );
    assert!(stderr.contains('^'), "caret missing: {stderr}");
    Ok(())
}

#[test]
fn file_input_reports_filename_and_missing_file() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    write!(file, "key: - a\n     - b\n")?; // 5U3A: inline sequence after ':'
    let path = file.path().to_str().unwrap();
    let (_, stderr, code) = run_validate_file(path, &["--no-color"])?;
    assert_eq!(code, 1);
    assert!(
        stderr.contains(path),
        "filename should appear in output: {stderr}"
    );

    // Missing file → I/O error exit code 2.
    let (_, _, code) = run_validate_file("/no/such/file.yaml", &["--no-color"])?;
    assert_eq!(code, 2);
    Ok(())
}

#[test]
fn valid_file_exits_zero() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    write!(file, "users:\n  - name: Alice\n  - name: Bob\n")?;
    let path = file.path().to_str().unwrap();
    let (_, stderr, code) = run_validate_file(path, &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    Ok(())
}

#[test]
fn rejects_inline_sequence_as_mapping_value() -> Result<()> {
    // 5U3A. The loader parses these leniently (#325), so strict rejection is
    // the validator's job. The bare-`-`-at-end-of-input spelling used to slip
    // through because the check required a byte after the `-`.
    // Also rejected one level deeper: a compact mapping entry's own value
    // (`- a: - x`, now leniently parsed by `parse_compact_mapping_entry` too).
    for input in ["a: - x\n", "a: -\n", "a: -", "- a: - x\n"] {
        let (_, stderr, code) = run_validate_stdin(input, &["--no-color"])?;
        assert_eq!(code, 1, "expected rejection for {input:?}: {stderr}");
    }

    // A `-` not followed by whitespace is an ordinary scalar and stays valid.
    for input in ["a: -1\n", "a: -1", "a:\n  - x\n"] {
        let (_, stderr, code) = run_validate_stdin(input, &["--no-color"])?;
        assert_eq!(code, 0, "expected acceptance for {input:?}: {stderr}");
    }
    Ok(())
}

// ============================================================================
// `syq --validate` (the yq runner's opt-in validation flag).
// ============================================================================

fn run_yq_validate_stdin(input: &str, filter: &str) -> Result<(String, String, i32)> {
    let mut cmd = Command::new(succinctly_bin())
        .args(["yq", "--validate", filter])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }
    let output = cmd.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

#[test]
fn yq_validate_passes_valid_yaml() -> Result<()> {
    let (stdout, stderr, code) = run_yq_validate_stdin("a: 1\nb: 2\n", ".a")?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "1");
    Ok(())
}

#[test]
fn yq_validate_rejects_invalid_yaml_before_output() -> Result<()> {
    let (stdout, stderr, code) = run_yq_validate_stdin("a: b: c: d\n", ".")?;
    assert_ne!(code, 0);
    assert!(
        stdout.is_empty(),
        "no query output on validation failure: {stdout}"
    );
    assert!(stderr.contains("validation error"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn yq_without_validate_accepts_the_same_invalid_yaml() -> Result<()> {
    // The default loader is non-validating: the same input succeeds without
    // `--validate`, proving the flag is opt-in.
    let (_, _, code) = {
        let mut cmd = Command::new(succinctly_bin())
            .args(["yq", "."])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = cmd.stdin.take() {
            stdin.write_all(b"a: b: c: d\n")?;
        }
        let output = cmd.wait_with_output()?;
        (
            String::from_utf8(output.stdout)?,
            String::from_utf8(output.stderr)?,
            output.status.code().unwrap_or(-1),
        )
    };
    assert_eq!(code, 0);
    Ok(())
}
