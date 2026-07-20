//! Integration tests for the succinctly json validate CLI command
//!
//! These tests verify RFC 8259 compliance and CLI behavior.
//! Run with: cargo test --features cli --test json_validate_tests

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::Result;
use tempfile::NamedTempFile;

/// Resolve the path to the pre-built `succinctly` CLI binary, building it once.
///
/// The integration-test harness is compiled without the `cli` feature (CI runs
/// plain `cargo test`), and the `succinctly` binary is gated by
/// `required-features = ["cli"]`, so `CARGO_BIN_EXE_succinctly` is not available
/// here. We therefore build the binary once with the `cli` feature and derive its
/// path from this test executable's own location.
///
/// Invoking the built binary directly (rather than `cargo run`) keeps cargo's own
/// output — compile progress and, on nightly, the future-incompatibility `note:` —
/// out of each child's captured stderr, so stderr assertions observe only the
/// application's output. The one-time `cargo build` blocking-waits on the build
/// lock, so no retry loop for lock contention is needed.
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

        // The test executable lives at `<target>/<profile>/deps/<test>-<hash>`;
        // the CLI binary is its sibling at `<target>/<profile>/succinctly`.
        let mut path = std::env::current_exe().expect("resolve current_exe");
        path.pop(); // drop the test executable's file name -> `.../deps`
        if path.file_name().and_then(|s| s.to_str()) == Some("deps") {
            path.pop(); // drop `deps` -> `.../<profile>`
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

/// Helper to run `json validate` with input from stdin.
fn run_validate_stdin(input: &str, extra_args: &[&str]) -> Result<(String, String, i32)> {
    let mut cmd = Command::new(succinctly_bin())
        .args(["json", "validate"])
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }

    let output = cmd.wait_with_output()?;
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    Ok((stdout, stderr, exit_code))
}

/// Helper to run `json validate` with file input.
fn run_validate_file(file_path: &str, extra_args: &[&str]) -> Result<(String, String, i32)> {
    let output = Command::new(succinctly_bin())
        .args(["json", "validate"])
        .args(extra_args)
        .arg(file_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    Ok((stdout, stderr, exit_code))
}

// ============================================================================
// Exit code tests
// ============================================================================

#[test]
fn test_valid_json_exit_code_0() -> Result<()> {
    let (stdout, stderr, exit_code) = run_validate_stdin(r#"{"key": "value"}"#, &[])?;
    assert_eq!(exit_code, 0, "stdout: {stdout}, stderr: {stderr}");
    assert!(stdout.is_empty(), "stdout should be empty for valid JSON");
    Ok(())
}

#[test]
fn test_invalid_json_exit_code_1() -> Result<()> {
    let (_, _, exit_code) = run_validate_stdin(r#"{"key": }"#, &[])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_quiet_mode_no_output() -> Result<()> {
    let (stdout, stderr, exit_code) = run_validate_stdin(r#"{"invalid": }"#, &["--quiet"])?;
    assert_eq!(exit_code, 1);
    assert!(stdout.is_empty(), "stdout should be empty in quiet mode");
    // The binary is invoked directly (not via `cargo run`), so its stderr carries
    // no cargo output to filter — quiet mode must produce a truly empty stderr.
    assert!(
        stderr.is_empty(),
        "stderr should be empty in quiet mode, got: {stderr}"
    );
    Ok(())
}

/// Issue #151: deeply nested input must fail validation cleanly instead of
/// aborting the process with a stack overflow (SIGABRT).
#[test]
fn test_deeply_nested_input_errors_instead_of_aborting() -> Result<()> {
    let input = "[".repeat(20_000);
    let (_, stderr, exit_code) = run_validate_stdin(&input, &[])?;
    assert_eq!(exit_code, 1, "stderr: {stderr}");
    assert!(
        stderr.contains("nesting depth exceeds limit"),
        "stderr should report the nesting cap, got: {stderr}"
    );
    Ok(())
}

// ============================================================================
// Valid JSON acceptance tests
// ============================================================================

#[test]
fn test_valid_empty_object() -> Result<()> {
    let (_, _, exit_code) = run_validate_stdin("{}", &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_empty_array() -> Result<()> {
    let (_, _, exit_code) = run_validate_stdin("[]", &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_null() -> Result<()> {
    let (_, _, exit_code) = run_validate_stdin("null", &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_true() -> Result<()> {
    let (_, _, exit_code) = run_validate_stdin("true", &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_false() -> Result<()> {
    let (_, _, exit_code) = run_validate_stdin("false", &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_nested_structure() -> Result<()> {
    let json =
        r#"{"users": [{"name": "Alice", "active": true}, {"name": "Bob", "active": false}]}"#;
    let (_, _, exit_code) = run_validate_stdin(json, &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_string_escapes() -> Result<()> {
    let json = r#"{"escapes": "quote\" backslash\\ slash\/ newline\n tab\t"}"#;
    let (_, _, exit_code) = run_validate_stdin(json, &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_unicode_escape() -> Result<()> {
    let json = r#"{"unicode": "\u0041\u0042\u0043"}"#;
    let (_, _, exit_code) = run_validate_stdin(json, &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_surrogate_pair() -> Result<()> {
    // U+1F600 (grinning face emoji) as surrogate pair
    let json = r#"{"emoji": "\uD83D\uDE00"}"#;
    let (_, _, exit_code) = run_validate_stdin(json, &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_valid_numbers() -> Result<()> {
    let json = r#"{"int": 42, "neg": -17, "float": 3.14, "exp": 1e10, "neg_exp": 2.5e-3}"#;
    let (_, _, exit_code) = run_validate_stdin(json, &[])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

// ============================================================================
// Invalid JSON rejection tests
// ============================================================================

#[test]
fn test_invalid_trailing_comma_object() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin(r#"{"key": "value",}"#, &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("error:"));
    Ok(())
}

#[test]
fn test_invalid_trailing_comma_array() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin("[1, 2, 3,]", &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("error:"));
    Ok(())
}

#[test]
fn test_invalid_leading_zero() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin(r#"{"count": 007}"#, &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("leading zero"));
    Ok(())
}

#[test]
fn test_invalid_leading_plus() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin("+42", &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("leading plus"));
    Ok(())
}

#[test]
fn test_invalid_escape_sequence() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin(r#"{"msg": "hello\qworld"}"#, &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("escape"));
    Ok(())
}

#[test]
fn test_invalid_control_character() -> Result<()> {
    // Tab character directly in string (should be escaped as \t)
    let (_, stderr, exit_code) =
        run_validate_stdin("{\"msg\": \"hello\tworld\"}", &["--no-color"])?;
    // Note: tab is valid whitespace in JSON strings per spec, so this should pass
    // Let me check - actually raw tab in a string requires escaping per RFC 8259
    // "All Unicode characters may be placed within the quotation marks, except for
    // the characters that MUST be escaped: quotation mark, reverse solidus, and
    // the control characters (U+0000 through U+001F)."
    // Tab is U+0009, so it must be escaped.
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("control character"));
    Ok(())
}

#[test]
fn test_invalid_lone_surrogate() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin(r#"{"bad": "\uD83D"}"#, &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("surrogate"));
    Ok(())
}

#[test]
fn test_invalid_unclosed_string() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin(r#"{"key": "unclosed"#, &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("unclosed") || stderr.contains("end of input"));
    Ok(())
}

#[test]
fn test_invalid_trailing_content() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin("null extra", &["--no-color"])?;
    assert_eq!(exit_code, 1);
    assert!(stderr.contains("trailing"));
    Ok(())
}

// ============================================================================
// Error position accuracy tests
// ============================================================================

#[test]
fn test_error_position_line_column() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_stdin(r#"{"key": "value",}"#, &["--no-color"])?;
    assert_eq!(exit_code, 1);
    // Error should be at column 17 (the closing brace after the comma)
    assert!(stderr.contains(":1:17") || stderr.contains("column 17"));
    Ok(())
}

#[test]
fn test_error_position_multiline() -> Result<()> {
    let json = "{\n  \"key\": \"value\",\n}";
    let (_, stderr, exit_code) = run_validate_stdin(json, &["--no-color"])?;
    assert_eq!(exit_code, 1);
    // Error should be on line 3
    assert!(stderr.contains(":3:") || stderr.contains("line 3"));
    Ok(())
}

// ============================================================================
// File input tests
// ============================================================================

#[test]
fn test_file_input_valid() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, r#"{{"name": "Alice"}}"#)?;
    file.flush()?;

    let (stdout, stderr, exit_code) = run_validate_file(file.path().to_str().unwrap(), &[])?;
    assert_eq!(exit_code, 0, "stdout: {stdout}, stderr: {stderr}");
    Ok(())
}

#[test]
fn test_file_input_invalid() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, r#"{{"name": "Alice",}}"#)?;
    file.flush()?;

    let (_, stderr, exit_code) = run_validate_file(file.path().to_str().unwrap(), &["--no-color"])?;
    assert_eq!(exit_code, 1);
    // Error should include the filename
    assert!(stderr.contains(file.path().file_name().unwrap().to_str().unwrap()));
    Ok(())
}

#[test]
fn test_file_not_found() -> Result<()> {
    let (_, stderr, exit_code) = run_validate_file("/nonexistent/path.json", &["--no-color"])?;
    assert_eq!(exit_code, 2); // I/O error
    assert!(stderr.contains("error:"));
    Ok(())
}

// ============================================================================
// Multiple files tests
// ============================================================================

#[test]
fn test_multiple_files_all_valid() -> Result<()> {
    let mut file1 = NamedTempFile::new()?;
    writeln!(file1, r#"{{"a": 1}}"#)?;
    file1.flush()?;

    let mut file2 = NamedTempFile::new()?;
    writeln!(file2, r#"{{"b": 2}}"#)?;
    file2.flush()?;

    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "json",
            "validate",
        ])
        .arg(file1.path())
        .arg(file2.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert_eq!(output.status.code().unwrap_or(-1), 0);
    Ok(())
}

#[test]
fn test_multiple_files_one_invalid() -> Result<()> {
    let mut file1 = NamedTempFile::new()?;
    writeln!(file1, r#"{{"a": 1}}"#)?;
    file1.flush()?;

    let mut file2 = NamedTempFile::new()?;
    writeln!(file2, r#"{{"b": }}"#)?; // invalid
    file2.flush()?;

    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "json",
            "validate",
        ])
        .arg(file1.path())
        .arg(file2.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert_eq!(output.status.code().unwrap_or(-1), 1);
    Ok(())
}

// ============================================================================
// Line number alignment tests
// ============================================================================

/// Helper to verify pipe alignment in error output.
/// Returns the column position of the '|' character on each line, or None if not found.
fn find_pipe_columns(stderr: &str) -> Vec<usize> {
    stderr.lines().filter_map(|line| line.find('|')).collect()
}

/// Creates a JSON object with an error at a specific line number.
/// The JSON structure is an array with one element per line (each on its own line).
fn create_json_with_error_at_line(target_line: usize) -> String {
    let mut json = String::new();
    json.push_str("[\n"); // Line 1

    // Add valid elements up to target_line - 1
    // Line 2 through target_line-1 have valid elements
    for i in 2..target_line {
        if i == target_line - 1 {
            // Last valid line before error - no trailing comma needed since error line follows
            json.push_str(&format!("  {}\n", i - 1));
        } else {
            json.push_str(&format!("  {},\n", i - 1));
        }
    }

    // Add invalid element at target_line (missing value after colon)
    json.push_str("  {\"bad\": }\n"); // This is the error line
    json.push_str("]\n");

    json
}

#[test]
fn test_alignment_single_digit_line() -> Result<()> {
    // Create JSON with error on line 9 (single digit)
    let json = create_json_with_error_at_line(9);

    let (_, stderr, exit_code) = run_validate_stdin(&json, &["--no-color"])?;
    assert_eq!(exit_code, 1);

    // Verify the error is on line 9
    assert!(
        stderr.contains(":9:"),
        "Error should be on line 9, got:\n{stderr}"
    );

    // Verify pipe alignment: all '|' should be at the same column
    let pipe_cols = find_pipe_columns(&stderr);
    assert!(
        pipe_cols.len() >= 2,
        "Should have at least 2 lines with pipes"
    );
    let first_col = pipe_cols[0];
    for col in &pipe_cols {
        assert_eq!(
            *col, first_col,
            "All pipes should be at the same column, got {pipe_cols:?}"
        );
    }
    Ok(())
}

#[test]
fn test_alignment_double_digit_line() -> Result<()> {
    // Create JSON with error on line 10 (double digit - transition point)
    let json = create_json_with_error_at_line(10);

    let (_, stderr, exit_code) = run_validate_stdin(&json, &["--no-color"])?;
    assert_eq!(exit_code, 1);

    // Verify the error is on line 10
    assert!(
        stderr.contains(":10:"),
        "Error should be on line 10, got:\n{stderr}"
    );

    // Verify pipe alignment
    let pipe_cols = find_pipe_columns(&stderr);
    assert!(
        pipe_cols.len() >= 2,
        "Should have at least 2 lines with pipes"
    );
    let first_col = pipe_cols[0];
    for col in &pipe_cols {
        assert_eq!(
            *col, first_col,
            "All pipes should be at the same column, got {pipe_cols:?}"
        );
    }
    Ok(())
}

#[test]
fn test_alignment_triple_digit_line() -> Result<()> {
    // Create JSON with error on line 999 (triple digit)
    let json = create_json_with_error_at_line(999);

    let (_, stderr, exit_code) = run_validate_stdin(&json, &["--no-color"])?;
    assert_eq!(exit_code, 1);

    // Verify the error is on line 999
    assert!(
        stderr.contains(":999:"),
        "Error should be on line 999, got:\n{stderr}"
    );

    // Verify pipe alignment
    let pipe_cols = find_pipe_columns(&stderr);
    assert!(
        pipe_cols.len() >= 2,
        "Should have at least 2 lines with pipes"
    );
    let first_col = pipe_cols[0];
    for col in &pipe_cols {
        assert_eq!(
            *col, first_col,
            "All pipes should be at the same column, got {pipe_cols:?}"
        );
    }
    Ok(())
}

#[test]
fn test_alignment_four_digit_line() -> Result<()> {
    // Create JSON with error on line 1000 (four digit - transition point)
    let json = create_json_with_error_at_line(1000);

    let (_, stderr, exit_code) = run_validate_stdin(&json, &["--no-color"])?;
    assert_eq!(exit_code, 1);

    // Verify the error is on line 1000
    assert!(
        stderr.contains(":1000:"),
        "Error should be on line 1000, got:\n{stderr}"
    );

    // Verify pipe alignment
    let pipe_cols = find_pipe_columns(&stderr);
    assert!(
        pipe_cols.len() >= 2,
        "Should have at least 2 lines with pipes"
    );
    let first_col = pipe_cols[0];
    for col in &pipe_cols {
        assert_eq!(
            *col, first_col,
            "All pipes should be at the same column, got {pipe_cols:?}"
        );
    }
    Ok(())
}
