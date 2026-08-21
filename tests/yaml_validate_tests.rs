//! Integration tests for the `succinctly yaml validate` CLI command.
//!
//! Run with: cargo test --features cli --test yaml_validate_tests
#![cfg(feature = "cli")]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::NamedTempFile;

/// Path to the pre-built `succinctly` CLI binary. Cargo builds the `succinctly`
/// bin target (gated `required-features = ["cli"]`) before this test binary
/// runs, since this file is itself gated on `cli`, and bakes the resulting
/// path in at compile time — correct under any target-dir layout.
fn succinctly_bin() -> &'static str {
    env!("CARGO_BIN_EXE_succinctly")
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

#[test]
fn rejects_out_dented_sequence_continuation() -> Result<()> {
    // #485. The loader parses these leniently (attaching the out-dented item
    // to the enclosing sequence), but strict rejection stays the validator's
    // job: a dedent that stops strictly between two open levels is invalid
    // (`check_block_indent`'s `popped && matches!(kind, Seq | Map)` case).
    for input in [
        "b:\n    - x\n   - y\nc: 2\n",
        "b:\n    - x\n   - y\n    - z\n",
        "b:\n  - x\n - y\n",
        "a:\n  b:\n    - x\n   - y\n  c: 2\n",
    ] {
        let (_, stderr, code) = run_validate_stdin(input, &["--no-color"])?;
        assert_eq!(code, 1, "expected rejection for {input:?}: {stderr}");
    }

    // Correctly-aligned sequences stay valid.
    for input in ["b:\n  - x\n  - y\n", "b:\n    - x\n    - y\nc: 2\n"] {
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

/// #1242: the strict YAML validator had no encoding check at all, so a
/// document with a stray non-UTF-8 byte validated clean (exit 0) and then
/// produced a scalar nothing could decode. The JSON validator has always
/// checked this; the YAML one only walked the grammar over raw bytes.
///
/// Written as a file rather than through stdin because the helper takes
/// `&str` and this input is deliberately not valid UTF-8.
#[test]
fn test_yaml_validate_rejects_invalid_utf8_1242() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    file.write_all(b"a: 1\nb: \"x\xe4y\"\n")?;
    file.flush()?;

    let (_, stderr, code) = run_validate_file(file.path().to_str().unwrap(), &[])?;
    assert_eq!(code, 1, "stderr: {stderr}");
    assert!(
        stderr.contains("UTF-8"),
        "should name the encoding failure: {stderr}"
    );
    // Same byte offset real yq reports for this document (`offset 11`),
    // rendered as the 1-based line/column this CLI uses everywhere else.
    assert!(
        stderr.contains("2:7"),
        "should locate the offending byte: {stderr}"
    );
    Ok(())
}

/// #1242 guard: a document that is valid UTF-8 but uses multi-byte
/// characters must still validate clean -- the new pass must not reject
/// ordinary non-ASCII content.
#[test]
fn test_yaml_validate_accepts_multibyte_utf8_1242() -> Result<()> {
    let (_, stderr, code) = run_validate_stdin("a: café\nb: 日本語\nc: 😀\n", &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    Ok(())
}
