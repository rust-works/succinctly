//! Integration tests for the succinctly jq CLI command
//!
//! These tests verify jq-compatible behavior and options.
//! Run with: cargo test --features cli --test jq_cli_tests

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;
use tempfile::NamedTempFile;

/// Maximum retries for cargo run commands that fail with exit code 101.
/// This handles flaky failures from cargo lock contention when tests run in parallel.
const MAX_CARGO_RETRIES: u32 = 3;

/// Maximum retries for spawning the pre-built binary directly.
///
/// `run_jq_full` execs a fixed path (`CARGO_BIN_EXE_succinctly`) that other
/// tests in this file concurrently rewrite via `cargo run` (see
/// `run_jq_stdin_streams`). `spawn()` can transiently observe that path
/// mid-replacement and fail with `ENOENT` (#550).
const MAX_SPAWN_RETRIES: u32 = 3;

/// Helper to run jq command with input from stdin
fn run_jq_stdin(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let (stdout, _, code) = run_jq_stdin_streams(filter, input, extra_args)?;
    Ok((stdout, code))
}

/// Helper to run jq command with input from stdin, keeping stderr.
///
/// Most tests only care about stdout and the exit code; use this when the
/// absence of a diagnostic is itself the thing under test. `--quiet` keeps
/// cargo's own progress lines ("Compiling", "Blocking waiting for file lock")
/// off stderr, so what remains is the binary's.
fn run_jq_stdin_streams(
    filter: &str,
    input: &str,
    extra_args: &[&str],
) -> Result<(String, String, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let mut cmd = Command::new("cargo")
            .args([
                "run",
                "--quiet",
                "--features",
                "cli",
                "--bin",
                "succinctly",
                "--",
                "jq",
            ])
            .args(extra_args)
            .arg(filter)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = cmd.stdin.take() {
            stdin.write_all(input.as_bytes())?;
        }

        let output = cmd.wait_with_output()?;
        let exit_code = output.status.code().unwrap_or(-1);

        // Exit code 101 often indicates cargo lock contention; retry
        if exit_code == 101 && attempt + 1 < MAX_CARGO_RETRIES {
            std::thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
            continue;
        }

        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;
        return Ok((stdout, stderr, exit_code));
    }
    unreachable!()
}

/// Run jq with an arbitrary argument list, returning stdout, stderr and the
/// exit code.
///
/// `run_jq_stdin` discards stderr and cannot take file arguments, which makes
/// it unusable for the diagnostics in #355 — those live entirely on stderr and
/// in the exit code, and their `(at <file>:<line>)` marker needs a real file.
///
/// Invokes the built binary directly rather than through `cargo run`: cargo
/// writes its own `Finished`/`Running` progress lines to stderr, which would be
/// indistinguishable from the diagnostics under test. That also makes the
/// lock-contention retry unnecessary — but the direct spawn can still race
/// concurrent rebuilds of the same path, so it gets its own retry below (#550).
fn run_jq_full(args: &[&str], input: Option<&str>) -> Result<(String, String, i32)> {
    let mut cmd = spawn_jq_full(args)?;

    if let Some(mut stdin) = cmd.stdin.take() {
        if let Some(input) = input {
            stdin.write_all(input.as_bytes())?;
        }
    }

    let output = cmd.wait_with_output()?;
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
        output.status.code().unwrap_or(-1),
    ))
}

/// Spawns the pre-built `succinctly` binary, retrying on `ENOENT`.
///
/// See `MAX_SPAWN_RETRIES` for why this retry exists.
fn spawn_jq_full(args: &[&str]) -> std::io::Result<std::process::Child> {
    for attempt in 0..MAX_SPAWN_RETRIES {
        match Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("jq")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => return Ok(child),
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound && attempt + 1 < MAX_SPAWN_RETRIES =>
            {
                std::thread::sleep(Duration::from_millis(50 * (attempt as u64 + 1)));
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// Helper to run jq command with file input
#[allow(dead_code)]
fn run_jq_file(filter: &str, file_path: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let output = Command::new("cargo")
            .args([
                "run",
                "--features",
                "cli",
                "--bin",
                "succinctly",
                "--",
                "jq",
            ])
            .args(extra_args)
            .arg(filter)
            .arg(file_path)
            .output()?;

        let exit_code = output.status.code().unwrap_or(-1);

        // Exit code 101 often indicates cargo lock contention; retry
        if exit_code == 101 && attempt + 1 < MAX_CARGO_RETRIES {
            std::thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
            continue;
        }

        let stdout = String::from_utf8(output.stdout)?;
        return Ok((stdout, exit_code));
    }
    unreachable!()
}

/// Helper to run jq with null input (-n)
fn run_jq_null(filter: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let output = Command::new("cargo")
            .args([
                "run",
                "--features",
                "cli",
                "--bin",
                "succinctly",
                "--",
                "jq",
            ])
            .arg("-n")
            .args(extra_args)
            .arg(filter)
            .output()?;

        let exit_code = output.status.code().unwrap_or(-1);

        // Exit code 101 often indicates cargo lock contention; retry
        if exit_code == 101 && attempt + 1 < MAX_CARGO_RETRIES {
            std::thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
            continue;
        }

        let stdout = String::from_utf8(output.stdout)?;
        return Ok((stdout, exit_code));
    }
    unreachable!()
}

// =============================================================================
// Basic Functionality Tests
// =============================================================================

#[test]
fn test_identity_filter() -> Result<()> {
    let (output, code) = run_jq_stdin(".", r#"{"a":1,"b":2}"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1,"b":2}"#);
    Ok(())
}

#[test]
fn test_field_access() -> Result<()> {
    let (output, code) = run_jq_stdin(".name", r#"{"name":"Alice","age":30}"#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""Alice""#);
    Ok(())
}

#[test]
fn test_nested_field_access() -> Result<()> {
    let (output, code) = run_jq_stdin(".user.name", r#"{"user":{"name":"Bob"}}"#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""Bob""#);
    Ok(())
}

#[test]
fn test_array_index() -> Result<()> {
    let (output, code) = run_jq_stdin(".[1]", r"[10,20,30]", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "20");
    Ok(())
}

#[test]
fn test_array_iteration() -> Result<()> {
    let (output, code) = run_jq_stdin(".[]", r"[1,2,3]", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "1\n2\n3\n");
    Ok(())
}

// Array slicing yields a single sub-array, not a stream of elements (issue #154).

#[test]
fn test_array_slice_returns_single_array() -> Result<()> {
    let (output, code) = run_jq_stdin(".[2:4]", r"[0,1,2,3,4]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[2,3]\n");
    Ok(())
}

#[test]
fn test_array_slice_construction_nests() -> Result<()> {
    let (output, code) = run_jq_stdin("[.[2:4]]", r"[0,1,2,3,4]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[[2,3]]\n");
    Ok(())
}

#[test]
fn test_array_slice_then_iterate_streams() -> Result<()> {
    let (output, code) = run_jq_stdin(".[2:4][]", r"[0,1,2,3,4]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "2\n3\n");
    Ok(())
}

#[test]
fn test_array_slice_piped_to_length() -> Result<()> {
    let (output, code) = run_jq_stdin(".[2:4] | length", r"[0,1,2,3,4]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "2\n");
    Ok(())
}

#[test]
fn test_array_slice_out_of_range_is_empty_array() -> Result<()> {
    let (output, code) = run_jq_stdin(".[5:10]", r"[0,1,2,3,4]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[]\n");
    Ok(())
}

#[test]
fn test_full_array_slice_returns_whole_array() -> Result<()> {
    let (output, code) = run_jq_stdin(".[:]", r"[0,1,2,3,4]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[0,1,2,3,4]\n");
    Ok(())
}

#[test]
fn test_string_slice_unchanged() -> Result<()> {
    let (output, code) = run_jq_stdin(".[1:3]", r#""hello""#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "\"el\"\n");
    Ok(())
}

#[test]
fn test_arithmetic() -> Result<()> {
    let (output, code) = run_jq_null("1 + 2 * 3", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "7");
    Ok(())
}

#[test]
fn test_unary_minus() -> Result<()> {
    // Negate input value - use -- to prevent option parsing of -. filter
    let mut cmd = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "--",
            "-.",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(b"5")?;
    }
    let output = cmd.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout.trim(), "-5");
    Ok(())
}

#[test]
fn test_unary_minus_expression() -> Result<()> {
    // Negate a complex expression
    let mut cmd = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "--",
            "-(.a + .b)",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(br#"{"a":3,"b":2}"#)?;
    }
    let output = cmd.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout.trim(), "-5");
    Ok(())
}

#[test]
fn test_double_negation() -> Result<()> {
    // Double negation should return original value
    let mut cmd = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "--",
            "--.",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(b"5")?;
    }
    let output = cmd.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout.trim(), "5");
    Ok(())
}

// =============================================================================
// Input Options Tests
// =============================================================================

#[test]
fn test_null_input() -> Result<()> {
    let (output, code) = run_jq_null("42", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "42");
    Ok(())
}

#[test]
fn test_raw_input() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "line1\nline2\nline3", &["-R"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "\"line1\"\n\"line2\"\n\"line3\"\n");
    Ok(())
}

#[test]
fn test_slurp() -> Result<()> {
    let (output, code) = run_jq_stdin("add", "1\n2\n3", &["-s"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "6");
    Ok(())
}

#[test]
fn test_slurp_with_raw_input() -> Result<()> {
    // jq -R -s: the entire input is one string, not an array of lines
    let (output, code) = run_jq_stdin(".", "a\nb\nc", &["-R", "-s", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""a\nb\nc""#);
    Ok(())
}

#[test]
fn test_slurp_with_raw_input_preserves_trailing_newline() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "a\nb\n", &["-R", "-s", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""a\nb\n""#);
    Ok(())
}

#[test]
fn test_slurp_with_raw_input_raw_output() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "x\ny", &["-R", "-s", "-r"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "x\ny\n");
    Ok(())
}

#[test]
fn test_slurp_with_raw_input_empty_input() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "", &["-R", "-s", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""""#);
    Ok(())
}

#[test]
fn test_slurp_with_raw_input_multiple_files() -> Result<()> {
    let mut file1 = NamedTempFile::new()?;
    writeln!(file1, "a")?;

    let mut file2 = NamedTempFile::new()?;
    writeln!(file2, "b")?;

    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
        ])
        .arg("-R")
        .arg("-s")
        .arg("-c")
        .arg(".")
        .arg(file1.path())
        .arg(file2.path())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), r#""a\nb\n""#);
    Ok(())
}

// =============================================================================
// Output Options Tests
// =============================================================================

#[test]
fn test_compact_output() -> Result<()> {
    let (output, code) = run_jq_stdin(".", r#"{"a": 1, "b": 2}"#, &["-c"])?;
    assert_eq!(code, 0);
    // Compact output should be on one line
    assert!(!output.contains('\n') || output.trim().lines().count() == 1);
    Ok(())
}

#[test]
fn test_raw_output() -> Result<()> {
    let (output, code) = run_jq_stdin(".name", r#"{"name":"Alice"}"#, &["-r"])?;
    assert_eq!(code, 0);
    // Raw output should not have quotes
    assert_eq!(output.trim(), "Alice");
    Ok(())
}

#[test]
fn test_join_output() -> Result<()> {
    let (output, code) = run_jq_stdin(".[]", r#"["a","b","c"]"#, &["-j"])?;
    assert_eq!(code, 0);
    // Join output should have no newlines between outputs
    assert_eq!(output, "abc");
    Ok(())
}

#[test]
fn test_sort_keys() -> Result<()> {
    let (output, code) = run_jq_stdin(".", r#"{"z":1,"a":2,"m":3}"#, &["-S", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":2,"m":3,"z":1}"#);
    Ok(())
}

#[test]
fn test_tab_indent() -> Result<()> {
    let (output, code) = run_jq_stdin(".", r#"{"a":1}"#, &["--tab"])?;
    assert_eq!(code, 0);
    assert!(output.contains('\t'));
    Ok(())
}

#[test]
fn test_custom_indent() -> Result<()> {
    let (output, code) = run_jq_stdin(".", r#"{"a":1}"#, &["--indent", "4"])?;
    assert_eq!(code, 0);
    // Should have 4-space indentation
    assert!(output.contains("    "));
    Ok(())
}

// =============================================================================
// Variable Tests
// =============================================================================

#[test]
fn test_arg_string_variable() -> Result<()> {
    let (output, code) = run_jq_null("$name", &["--arg", "name", "Alice"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""Alice""#);
    Ok(())
}

#[test]
fn test_argjson_variable() -> Result<()> {
    let (output, code) = run_jq_null("$count + 10", &["--argjson", "count", "42"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "52");
    Ok(())
}

#[test]
fn test_multiple_variables() -> Result<()> {
    let (output, code) = run_jq_null("$a + $b", &["--argjson", "a", "10", "--argjson", "b", "20"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "30");
    Ok(())
}

#[test]
fn test_arg_string_concatenation() -> Result<()> {
    let (output, code) = run_jq_null(
        r#"$first + " " + $last"#,
        &["--arg", "first", "Hello", "--arg", "last", "World", "-r"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "Hello World");
    Ok(())
}

#[test]
fn test_slurpfile() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, r#"{{"x":1}}"#)?;
    writeln!(file, r#"{{"x":2}}"#)?;

    let (output, code) = run_jq_null(
        "$data | length",
        &["--slurpfile", "data", file.path().to_str().unwrap()],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "2");
    Ok(())
}

#[test]
fn test_rawfile() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    write!(file, "Hello, World!")?;

    let (output, code) = run_jq_null(
        "$content",
        &["--rawfile", "content", file.path().to_str().unwrap(), "-r"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "Hello, World!");
    Ok(())
}

// =============================================================================
// Filter File Tests
// =============================================================================

#[test]
fn test_from_file() -> Result<()> {
    let mut filter_file = NamedTempFile::new()?;
    writeln!(filter_file, ".name")?;

    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, r#"{{"name":"Alice"}}"#)?;

    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
        ])
        .arg("-f")
        .arg(filter_file.path())
        .arg(input_file.path())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), r#""Alice""#);
    Ok(())
}

// =============================================================================
// Exit Status Tests
// =============================================================================

#[test]
fn test_exit_status_true() -> Result<()> {
    let (_, code) = run_jq_stdin(".", "true", &["-e"])?;
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_false() -> Result<()> {
    let (_, code) = run_jq_stdin(".", "false", &["-e"])?;
    assert_eq!(code, 1);
    Ok(())
}

#[test]
fn test_exit_status_null() -> Result<()> {
    let (_, code) = run_jq_stdin(".", "null", &["-e"])?;
    assert_eq!(code, 1);
    Ok(())
}

#[test]
fn test_exit_status_number() -> Result<()> {
    let (_, code) = run_jq_stdin(".", "0", &["-e"])?;
    // 0 is truthy in jq
    assert_eq!(code, 0);
    Ok(())
}

// -----------------------------------------------------------------------------
// Exit status on the identity fast path (-c enables can_use_raw_identity()).
//
// The plain `-e` tests above never reach the identity fast path because
// `can_use_raw_identity()` requires compact mode. With `-c -e` the fast path
// emits raw bytes without materializing the value, so exit status must be
// derived from the raw JSON token. Regression coverage for #175 / #178.
// -----------------------------------------------------------------------------

#[test]
fn test_exit_status_fast_path_false() -> Result<()> {
    let (out, code) = run_jq_stdin(".", "false", &["-c", "-e"])?;
    assert_eq!(out, "false\n");
    assert_eq!(code, 1, "false on the identity fast path must exit 1");
    Ok(())
}

#[test]
fn test_exit_status_fast_path_null() -> Result<()> {
    let (out, code) = run_jq_stdin(".", "null", &["-c", "-e"])?;
    assert_eq!(out, "null\n");
    assert_eq!(code, 1, "null on the identity fast path must exit 1");
    Ok(())
}

#[test]
fn test_exit_status_fast_path_true() -> Result<()> {
    let (out, code) = run_jq_stdin(".", "true", &["-c", "-e"])?;
    assert_eq!(out, "true\n");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_zero_is_truthy() -> Result<()> {
    let (out, code) = run_jq_stdin(".", "0", &["-c", "-e"])?;
    assert_eq!(out, "0\n");
    // 0 is truthy in jq even though it is "falsy" in many other languages.
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_false_string_is_truthy() -> Result<()> {
    // A quoted "false" is a non-empty string, which is truthy.
    let (out, code) = run_jq_stdin(".", "\"false\"", &["-c", "-e"])?;
    assert_eq!(out, "\"false\"\n");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_last_value_wins() -> Result<()> {
    // With multiple inputs, exit status reflects the LAST output value.
    // Last value is `false` -> exit 1.
    let (out, code) = run_jq_stdin(".", "true false", &["-c", "-e"])?;
    assert_eq!(out, "true\nfalse\n");
    assert_eq!(code, 1);

    // Last value is `true` -> exit 0.
    let (out, code) = run_jq_stdin(".", "false true", &["-c", "-e"])?;
    assert_eq!(out, "false\ntrue\n");
    assert_eq!(code, 0);
    Ok(())
}

// -----------------------------------------------------------------------------
// Exit status on the NON-fast path (#244).
//
// Non-identity filters never hit the identity fast path (the is_identity()
// gate), regardless of -c. These lock in the currently-correct exit codes so
// the #178 fast-path fix can't regress the path that already works.
// -----------------------------------------------------------------------------

#[test]
fn test_exit_status_nonfast_true() -> Result<()> {
    let (out, code) = run_jq_stdin(".a", r#"{"a":true}"#, &["-e"])?;
    assert_eq!(out, "true\n");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_false() -> Result<()> {
    let (out, code) = run_jq_stdin(".a", r#"{"a":false}"#, &["-e"])?;
    assert_eq!(out, "false\n");
    assert_eq!(code, 1, "false on the non-fast path must exit 1");
    Ok(())
}

#[test]
fn test_exit_status_nonfast_null() -> Result<()> {
    let (out, code) = run_jq_stdin(".a", r#"{"a":null}"#, &["-e"])?;
    assert_eq!(out, "null\n");
    assert_eq!(code, 1, "null on the non-fast path must exit 1");
    Ok(())
}

#[test]
fn test_exit_status_nonfast_number() -> Result<()> {
    let (out, code) = run_jq_stdin(".a", r#"{"a":1}"#, &["-e"])?;
    assert_eq!(out, "1\n");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_empty_string_is_truthy() -> Result<()> {
    let (out, code) = run_jq_stdin(".a", r#"{"a":""}"#, &["-e"])?;
    assert_eq!(out, "\"\"\n");
    // Only false and null are falsy in jq; the empty string is truthy.
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_missing_key_is_null() -> Result<()> {
    // A missing key produces null output (exit 1), not empty output (exit 4).
    let (out, code) = run_jq_stdin(".a", "{}", &["-e"])?;
    assert_eq!(out, "null\n");
    assert_eq!(code, 1);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_no_output() -> Result<()> {
    let (out, code) = run_jq_stdin(".[] | select(. > 5)", "[1,2,3]", &["-e"])?;
    assert_eq!(out, "");
    assert_eq!(code, 4, "no output with -e must exit 4");
    Ok(())
}

#[test]
fn test_exit_status_nonfast_last_value_wins() -> Result<()> {
    // With multiple outputs, exit status reflects the LAST output value.
    let (out, code) = run_jq_stdin(".[]", "[true,false]", &["-e"])?;
    assert_eq!(out, "true\nfalse\n");
    assert_eq!(code, 1);

    let (out, code) = run_jq_stdin(".[]", "[false,true]", &["-e"])?;
    assert_eq!(out, "false\ntrue\n");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_long_flag() -> Result<()> {
    let (out, code) = run_jq_stdin(".a", r#"{"a":false}"#, &["--exit-status"])?;
    assert_eq!(out, "false\n");
    assert_eq!(code, 1, "--exit-status must behave like -e");
    Ok(())
}

#[test]
fn test_exit_status_nonfast_compact() -> Result<()> {
    // Compact mode enables the identity fast path, but only for the identity
    // filter; a non-identity filter with -c must still exit 1 on false.
    let (out, code) = run_jq_stdin(".a", r#"{"a":false}"#, &["-c", "-e"])?;
    assert_eq!(out, "false\n");
    assert_eq!(code, 1);
    Ok(())
}

// =============================================================================
// Multiple Input Tests
// =============================================================================

#[test]
fn test_multiple_json_inputs() -> Result<()> {
    let (output, code) = run_jq_stdin(".x", r#"{"x":1}{"x":2}{"x":3}"#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "1\n2\n3\n");
    Ok(())
}

#[test]
fn test_multiple_file_inputs() -> Result<()> {
    let mut file1 = NamedTempFile::new()?;
    writeln!(file1, r#"{{"name":"Alice"}}"#)?;

    let mut file2 = NamedTempFile::new()?;
    writeln!(file2, r#"{{"name":"Bob"}}"#)?;

    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
        ])
        .arg("-r")
        .arg(".name")
        .arg(file1.path())
        .arg(file2.path())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout, "Alice\nBob\n");
    Ok(())
}

// =============================================================================
// Builtin Function Tests
// =============================================================================

#[test]
fn test_builtin_length() -> Result<()> {
    let (output, code) = run_jq_stdin("length", r"[1,2,3,4,5]", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "5");
    Ok(())
}

#[test]
fn test_builtin_values_is_identity_on_non_null() -> Result<()> {
    // #161: jq's `values` is `select(. != null)`, not object/array iteration
    let (output, code) = run_jq_stdin("values", r#"{"a":1}"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);

    let (output, code) = run_jq_stdin("values", "1", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1");
    Ok(())
}

#[test]
fn test_builtin_values_on_null_outputs_nothing() -> Result<()> {
    // #161: jq: null | values => (no output)
    let (output, code) = run_jq_stdin("values", "null", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "");
    Ok(())
}

#[test]
fn test_builtin_first_last_on_empty_array_output_null() -> Result<()> {
    // #161: jq's `first` is `.[0]`, so `[] | first` => null, not an error
    let (output, code) = run_jq_stdin("first", "[]", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    let (output, code) = run_jq_stdin("last", "[]", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");
    Ok(())
}

#[test]
fn test_builtin_first_on_non_array_errors() -> Result<()> {
    // jq: 5 | first => error. The error goes to stderr and nothing to stdout,
    // and the process exits 5 so the failure is visible to a shell (#355).
    let mut cmd = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "first",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(b"5")?;
    }
    let output = cmd.wait_with_output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stdout.trim(), "");
    // jq defines `first` as `.[0]`, so it reports an indexing error (#356);
    // the exact wording is pinned against jq-1.7.1 in
    // tests/data/jq-error-messages.tsv.
    assert!(
        stderr.contains("Cannot index number with number"),
        "Should report jq's indexing error for `5 | first`: {stderr}"
    );
    assert_eq!(
        output.status.code(),
        Some(5),
        "An uncaught eval error must exit 5 like jq: {stderr}"
    );
    Ok(())
}

/// Run a filter and return its stderr, for cases whose observable behaviour is
/// the error message rather than stdout.
///
/// The jq golden corpus cannot host these: it compares stdout only and requires
/// exit 0.
fn jq_stderr(filter: &str, input: &str, extra_args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
        ])
        .args(extra_args)
        .arg(filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(input.as_bytes())?;
    }
    let output = cmd.wait_with_output()?;
    Ok(String::from_utf8(output.stderr)?)
}

/// Indexing by a key whose *kind* cannot index the container reports jq's
/// wording verbatim (#360).
///
/// The messages are transcribed from jq 1.7.1. Note the string form inserts the
/// key raw rather than JSON-escaped — `--arg k 'a"b'` really does produce
/// `Cannot index array with string "a"b"` in jq.
#[test]
fn test_cannot_index_error_wording() -> Result<()> {
    let cases: &[(&str, &str, &[&str], &str)] = &[
        (".[null]", "{}", &[], "Cannot index object with null"),
        (".[null]", "[]", &[], "Cannot index array with null"),
        (".[null]", "null", &[], "Cannot index null with null"),
        (".[true]", "{}", &[], "Cannot index object with boolean"),
        (".[{}]", "{}", &[], "Cannot index object with object"),
        (".[[1]]", "{}", &[], "Cannot index object with array"),
        (
            ".[$k]",
            "[1,2]",
            &["--arg", "k", "a"],
            r#"Cannot index array with string "a""#,
        ),
        (
            ".[$k]",
            r#""s""#,
            &["--arg", "k", "a"],
            r#"Cannot index string with string "a""#,
        ),
        // The same wording must come out of the path/assignment pre-pass, not
        // just the value path.
        (".[null] = 1", "{}", &[], "Cannot index object with null"),
        ("path(.[null])", "{}", &[], "Cannot index object with null"),
        ("del(.[null])", "{}", &[], "Cannot index object with null"),
    ];

    for (filter, input, args, expected) in cases {
        let stderr = jq_stderr(filter, input, args)?;
        assert!(
            stderr.contains(expected),
            "`{filter}` on `{input}` should report {expected:?}, got: {stderr}"
        );
    }
    Ok(())
}

/// A *literal* key reports the same thing a computed one does.
///
/// The pair in each row is the same query written two ways. They travel
/// different routes to get there — a constant key folds to
/// `Expr::Field`/`Expr::Index` at parse time (#360), a computed one stays an
/// `Expr::IndexExpr` and dispatches on the key's kind at run time — and each
/// route raises its error from its own site, so nothing but a test holds them to
/// one wording. A message that changes with the spelling of the key is worse
/// than either message alone.
///
/// Nothing else covers the pairing: the #356 error corpus probes each spelling
/// against jq independently, which catches a drift from jq but not a drift
/// between the two spellings if both were to move together.
#[test]
fn test_cannot_index_wording_is_spelling_independent() -> Result<()> {
    // (literal filter, computed filter, extra args, input, expected message)
    let cases: &[(&str, &str, &[&str], &str, &str)] = &[
        (
            ".[0]",
            ".[$n]",
            &["--argjson", "n", "0"],
            r#"{"a":1}"#,
            "Cannot index object with number",
        ),
        (
            r#".["x"]"#,
            ".[$k]",
            &["--arg", "k", "x"],
            "[1,2]",
            r#"Cannot index array with string "x""#,
        ),
        (
            ".x",
            ".[$k]",
            &["--arg", "k", "x"],
            "[1,2]",
            r#"Cannot index array with string "x""#,
        ),
        (
            ".a",
            ".[$k]",
            &["--arg", "k", "a"],
            "123",
            r#"Cannot index number with string "a""#,
        ),
        (
            ".[0]",
            ".[$n]",
            &["--argjson", "n", "0"],
            r#""s""#,
            "Cannot index string with number",
        ),
    ];

    for (literal, computed, args, input, expected) in cases {
        for filter in [literal, computed] {
            let stderr = jq_stderr(filter, input, args)?;
            assert!(
                stderr.contains(expected),
                "`{filter}` on `{input}` should report {expected:?}, got: {stderr}"
            );
        }
    }
    Ok(())
}

/// A NaN key reads as null but must never *write*.
///
/// `f64 as i64` maps NaN to `0`, so the natural implementation silently reads —
/// and, in an assignment, overwrites — element zero. jq yields null on the read
/// and errors on the write; both halves are checked here because only the read
/// half is observable in the golden corpus (`index_nan_key`).
#[test]
fn test_nan_key_reads_null_and_rejects_writes() -> Result<()> {
    let (output, _) = run_jq_stdin("[.[nan]]", "[10,20,30]", &["-c"])?;
    assert_eq!(output.trim(), "[null]");

    for filter in [".[nan] = 5", ".[nan] |= 5", "path(.[nan])", "del(.[nan])"] {
        let stderr = jq_stderr(filter, "[10,20,30]", &[])?;
        assert!(
            stderr.contains("Cannot set array element at NaN index"),
            "`{filter}` should refuse a NaN index, got: {stderr}"
        );
        let (output, _) = run_jq_stdin(filter, "[10,20,30]", &["-c"])?;
        assert_eq!(
            output.trim(),
            "",
            "`{filter}` must not emit a mutated document"
        );
    }
    Ok(())
}

/// `?` suppresses a bad-key error rather than propagating it.
///
/// Regression guard for the `eval_generic` fallback: routing an unhandled
/// expression through `full_eval` restarts with `optional = false`, which
/// silently dropped the `?` here.
#[test]
fn test_optional_suppresses_cannot_index() -> Result<()> {
    let (output, _) = run_jq_stdin(".[null]?", "{}", &["-c"])?;
    assert_eq!(output.trim(), "");
    let stderr = jq_stderr(".[null]?", "{}", &[])?;
    assert!(
        !stderr.contains("Cannot index"),
        "`?` should suppress the error, got: {stderr}"
    );
    Ok(())
}

/// `?` covers the indexing, not the key expression.
///
/// jq's `.[K]?` is `try (E[K])` only over the index step: `.[error("boom")]?`
/// still raises `boom`, and walking `..` onto a string with `.[.k]?` still fails
/// on the key lookup. Passing the enclosing `optional` down into the key's
/// evaluation made succinctly strictly more forgiving than jq — `[.. | .[.k]?]`
/// returned `[1]` where jq errors.
#[test]
fn test_optional_does_not_suppress_key_errors() -> Result<()> {
    let cases: &[(&str, &str, &str)] = &[
        (
            "[.. | .[.k]?]",
            r#"{"k":"a","a":1}"#,
            r#"Cannot index string with string "k""#,
        ),
        (".[error(\"boom\")]?", r#"{"a":1}"#, "boom"),
    ];

    for (filter, input, expected) in cases {
        let stderr = jq_stderr(filter, input, &[])?;
        assert!(
            stderr.contains(expected),
            "`{filter}` should propagate the key error {expected:?}, got: {stderr}"
        );
    }

    // `try` still catches what `?` declines to swallow, so the error is
    // recoverable — it is raised, not fatal by construction.
    let (output, _) = run_jq_stdin(
        r#"[.. | try .[.k] catch "E"]"#,
        r#"{"k":"a","a":1}"#,
        &["-c"],
    )?;
    assert_eq!(output.trim(), r#"[1,"E","E"]"#);
    Ok(())
}

#[test]
fn test_contains_type_mismatch_errors() -> Result<()> {
    // jq: `1 | contains("a")` is an error, not `false` (#358). This exercises the
    // CLI's generic evaluator, which reaches the check by delegating to the full
    // one. Like `first` above, the exit status is not asserted: runtime eval
    // errors still exit 0 rather than jq's 5 (#355).
    let mut cmd = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-c",
            r#"contains("a")"#,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(b"1")?;
    }
    let output = cmd.wait_with_output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stdout.trim(), "", "no `false` on stdout: {stdout}");
    assert!(
        stderr.contains(r#"number (1) and string ("a") cannot have their containment checked"#),
        "Should report jq's containment error: {stderr}"
    );
    Ok(())
}

#[test]
fn test_builtin_keys() -> Result<()> {
    let (output, code) = run_jq_stdin("keys", r#"{"z":1,"a":2}"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["a","z"]"#);
    Ok(())
}

#[test]
fn test_builtin_map() -> Result<()> {
    let (output, code) = run_jq_stdin("map(. * 2)", r"[1,2,3]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[2,4,6]");
    Ok(())
}

#[test]
fn test_builtin_select() -> Result<()> {
    let (output, code) = run_jq_stdin(".[] | select(. > 2)", r"[1,2,3,4,5]", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "3\n4\n5\n");
    Ok(())
}

#[test]
fn test_builtin_type() -> Result<()> {
    let (output, code) = run_jq_stdin("type", r#"{"a":1}"#, &["-r"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "object");
    Ok(())
}

// =============================================================================
// Complex Expression Tests
// =============================================================================

#[test]
fn test_object_construction() -> Result<()> {
    let (output, code) = run_jq_stdin(
        r"{name: .user, id: .id}",
        r#"{"user":"Alice","id":42}"#,
        &["-c"],
    )?;
    assert_eq!(code, 0);
    // jq preserves expression order: name comes first, then id
    assert_eq!(output.trim(), r#"{"name":"Alice","id":42}"#);
    Ok(())
}

#[test]
fn test_object_construction_cartesian_issue_354() -> Result<()> {
    // `{...}` is a generator: an entry whose key or value yields n outputs
    // multiplies the objects emitted. Object construction used to keep a single
    // value per entry and reject multi-output keys outright (#354).
    //
    // Both reproducers from the issue. Same-source operands throughout, so these
    // stay independent of the comma-ordering bug (#353).
    let (output, code) = run_jq_stdin("{a: (.x,.y)}", r#"{"x":9,"y":8}"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\"a\":9}\n{\"a\":8}\n");

    let (output, code) = run_jq_stdin("{a: .[]}", r#"{"x":9,"y":8}"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\"a\":9}\n{\"a\":8}\n");

    // Multi-output keys used to error with "key must be a string" -- both keys
    // ARE strings, there were simply two of them.
    let (output, code) = run_jq_stdin(r#"{("a","b"): 1}"#, "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\"a\":1}\n{\"b\":1}\n");

    // The last entry varies fastest; within an entry the key varies slower than
    // the value. A transposed product would reorder these.
    let (output, code) = run_jq_stdin("{a: (1,2), b: (3,4)}", "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "{\"a\":1,\"b\":3}\n{\"a\":1,\"b\":4}\n{\"a\":2,\"b\":3}\n{\"a\":2,\"b\":4}\n"
    );

    // An entry with zero outputs empties the product, and short-circuits the
    // entries to its right -- the `error("boom")` below is never evaluated.
    let (output, code) = run_jq_stdin("{a: empty, b: 1}", "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "");

    let (output, code) = run_jq_stdin(r#"{a: empty, b: error("boom")}"#, "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "");

    // A non-string key still errors, but only once the value stream is non-empty
    // -- `{(.n): empty}` raises nothing even though `.n` is a number.
    let (output, code) = run_jq_stdin("{(.n): empty}", r#"{"n":1}"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "");

    // Once the value stream is non-empty the numeric key does raise, producing
    // no output. Only stdout is asserted: the CLI exits 0 on every evaluation
    // error (jq exits 5), which is a pre-existing gap unrelated to #354.
    let (output, _) = run_jq_stdin("{(.n): 2}", r#"{"n":1}"#, &["-c"])?;
    assert_eq!(output, "");

    Ok(())
}

#[test]
fn test_object_construction_multi_output_after_pipe_issue_354() -> Result<()> {
    // A multi-output object on the RHS of a pipe stays a stream of objects
    // rather than being folded into one array.
    let (output, code) = run_jq_stdin(r#"{"p":1} | {a: (2,3)}"#, "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\"a\":2}\n{\"a\":3}\n");

    // ... and collapses back into a single array only when asked to.
    let (output, code) = run_jq_stdin("[{a: (1,2)}]", "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"[{"a":1},{"a":2}]"#);

    Ok(())
}

#[test]
fn test_arithmetic_compare_cartesian_fanout_issue_768() -> Result<()> {
    // Every arithmetic/comparison operator used to collapse a multi-output
    // operand to its first value via `result_to_owned`, instead of jq's
    // cartesian-product fanout (#768). All 11 operators, from the issue's
    // own reproduction table.
    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) + 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[3,4,5]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) - 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[-1,0,1]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) * 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[2,4,6]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) / 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[0.5,1,1.5]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) % 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[1,0,1]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) == 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[false,true,false]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) != 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[true,false,true]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) < 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[true,false,false]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) <= 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[true,true,false]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) > 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[false,false,true]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2,3) >= 2]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[false,true,true]");

    // A generator operand streams directly (not just under an array
    // constructor): `.[] + 1` on `[1,2,3]` is `2`, `3`, `4`.
    let (stdout, _, code) = run_jq_full(&["-c", ".[] + 1"], Some("[1,2,3]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "2\n3\n4\n");

    let (stdout, _, code) = run_jq_full(&["-c", "[.[] > 1]"], Some("[1,2,3]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[false,true,true]");

    // A bare (non-array-wrapped) top-level comparison exercises a distinct
    // code path from the array-wrapped assertions above: the CLI's default
    // fast path evaluates a top-level `Expr::Compare` through
    // `eval_generic.rs`'s own native arm (for cursor-context/perf reasons),
    // not `eval.rs`'s `eval_compare` -- wrapping in `[...]` routes through
    // array construction instead, which always hits the full evaluator. Both
    // arms had the identical collapse-to-first bug and both needed fixing
    // (#768); arithmetic has no such native arm in eval_generic.rs, so bare
    // arithmetic worked correctly even before this fix.
    let (stdout, _, code) = run_jq_full(&["-c", "(1,2,3) > 2"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "false\nfalse\ntrue\n");

    let (stdout, _, code) = run_jq_full(&["-c", ".[] > 1"], Some("[1,2,3]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "false\ntrue\ntrue\n");

    // Cartesian ordering when both sides are generators: right operand
    // outer, left operand inner (jq's actual order, verified against the
    // pinned oracle -- NOT the reverse `eval_boolean`/`and`/`or` uses).
    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2) + (10,20)]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[11,12,21,22]");

    let (stdout, _, code) = run_jq_full(&["-c", "[(1,2) == (1,2)]"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[true,false,false,true]");

    Ok(())
}

#[test]
fn test_arithmetic_compare_cartesian_fanout_error_and_break_issue_768() -> Result<()> {
    // A raised error anywhere in the fanout aborts the whole computation
    // rather than skipping just that pairing -- jq doesn't retry a
    // generator past a fatal error. `[...]` only prints once its full
    // stream is known, so an error mid-collection leaves stdout empty
    // (verified against the pinned oracle: exit 5, no stdout).
    let (stdout, _, code) = run_jq_full(&["-c", r#"[(1,2,error("x")) + 1]"#], Some("null"))?;
    assert_eq!(code, 5);
    assert_eq!(stdout, "");

    let (stdout, _, code) = run_jq_full(&["-c", r#"[(1,"a",3) + 1]"#], Some("null"))?;
    assert_eq!(code, 5);
    assert_eq!(stdout, "");

    // `?` preserves the prefix already produced before the abort, rather
    // than discarding it -- it doesn't skip-and-continue past the failing
    // pairing either.
    let (stdout, _, code) = run_jq_full(&["-c", r#"[((1,2,error("x")) + 1)?]"#], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[2,3]");

    // Same abort-with-prefix behavior for an op-application error (not an
    // operand-evaluation error): `"a" + 1` fails, but `1 + 1` already ran.
    let (stdout, _, code) = run_jq_full(&["-c", r#"[((1,"a",3) + 1)?]"#], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[2]");

    // A `break` from an operand also aborts after the pairings already
    // emitted, without erroring.
    let (stdout, _, code) =
        run_jq_full(&["-c", "label $out | (1,2,break $out,4) + 1"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "2\n3\n");

    Ok(())
}

#[test]
fn test_array_construction() -> Result<()> {
    let (output, code) = run_jq_stdin("[.a, .b, .c]", r#"{"a":1,"b":2,"c":3}"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[1,2,3]");
    Ok(())
}

#[test]
fn test_conditional() -> Result<()> {
    let (output, code) = run_jq_stdin(r#"if . > 5 then "big" else "small" end"#, "10", &["-r"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "big");
    Ok(())
}

#[test]
fn test_object_comparison_operators() -> Result<()> {
    // Issue #162: object </> comparisons were always false in the CLI path.
    // jq compares objects by sorted key arrays first, then values in key order.
    let (output, code) = run_jq_stdin(r#"{"a":1} < {"a":2}"#, "null", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "true\n");

    // Key arrays decide before any values: ["a","b"] < ["a","c"].
    let (output, code) = run_jq_stdin(r#"{"a":2,"b":1} < {"a":1,"c":9}"#, "null", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "true\n");
    Ok(())
}

#[test]
fn test_try_catch() -> Result<()> {
    // jq returns null for .foo.bar when .foo is null (not an error)
    let (output, code) = run_jq_stdin(r#"try .foo.bar catch "error""#, r#"{"foo":null}"#, &["-r"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    // Actual error case: .foo.bar when .foo is a number triggers catch
    let (output, code) = run_jq_stdin(r#"try .foo.bar catch "error""#, r#"{"foo":123}"#, &["-r"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "error");

    Ok(())
}

#[test]
fn test_reduce() -> Result<()> {
    let (output, code) = run_jq_stdin("reduce .[] as $x (0; . + $x)", r"[1,2,3,4,5]", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "15");
    Ok(())
}

// =============================================================================
// Default Filter Test
// =============================================================================

#[test]
fn test_default_identity_filter() -> Result<()> {
    // When no filter is provided, should default to "."
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
        ])
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // This test would need special handling for empty filter
    // For now, just verify the command runs
    drop(output);
    Ok(())
}

// =============================================================================
// Help Output Test
// =============================================================================

#[test]
fn test_jq_help() -> Result<()> {
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "--help",
        ])
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("jq filter expression"));
    assert!(stdout.contains("--null-input"));
    assert!(stdout.contains("--raw-output"));
    assert!(stdout.contains("--compact-output"));
    assert!(stdout.contains("--slurp"));
    assert!(stdout.contains("--arg"));
    assert!(stdout.contains("--argjson"));
    assert!(stdout.contains("--raw-output0"));
    assert!(stdout.contains("--unbuffered"));
    assert!(stdout.contains("--ascii-output"));
    assert!(stdout.contains("--color-output"));
    assert!(stdout.contains("--monochrome-output"));
    Ok(())
}

#[test]
fn test_ascii_output() -> Result<()> {
    // Test that --ascii-output escapes non-ASCII characters
    let (output, _) = run_jq_stdin(".", r#"{"name":"世界"}"#, &["-c", "-a"])?;
    // Chinese characters should be escaped as \uXXXX
    assert!(output.contains(r"\u4e16\u754c"));
    assert!(!output.contains("世界"));
    Ok(())
}

#[test]
fn test_ascii_output_emoji() -> Result<()> {
    // Test that emoji (outside BMP) are escaped as surrogate pairs
    let (output, _) = run_jq_stdin(".", r#"{"emoji":"🌍"}"#, &["-c", "-a"])?;
    // Earth emoji U+1F30D should be encoded as surrogate pair
    assert!(output.contains(r"\ud83c\udf0d"));
    assert!(!output.contains("🌍"));
    Ok(())
}

#[test]
fn test_color_output() -> Result<()> {
    // Test that -C adds ANSI color codes
    let (output, _) = run_jq_stdin(".", r#"{"name":"test"}"#, &["-c", "-C"])?;
    // Should contain ANSI escape sequences
    assert!(output.contains("\x1b["));
    Ok(())
}

#[test]
fn test_monochrome_output() -> Result<()> {
    // Test that -M disables color even when -C might be implied
    let (output, _) = run_jq_stdin(".", r#"{"name":"test"}"#, &["-c", "-M"])?;
    // Should NOT contain ANSI escape sequences
    assert!(!output.contains("\x1b["));
    assert_eq!(output.trim(), r#"{"name":"test"}"#);
    Ok(())
}

// =============================================================================
// New Compatibility Features Tests
// =============================================================================

#[test]
fn test_raw_output0() -> Result<()> {
    // Test that --raw-output0 outputs strings with NUL terminator
    let (output, _) = run_jq_stdin(".name", r#"{"name":"Alice"}"#, &["--raw-output0"])?;
    // Output should be "Alice\0" (NUL terminated)
    assert_eq!(output.as_bytes(), b"Alice\0");
    Ok(())
}

#[test]
fn test_raw_output0_multiple() -> Result<()> {
    // Test multiple outputs with NUL terminators
    let (output, _) = run_jq_stdin(".[]", r#"["a","b","c"]"#, &["--raw-output0"])?;
    // Each string should be NUL terminated
    assert_eq!(output.as_bytes(), b"a\0b\0c\0");
    Ok(())
}

#[test]
fn test_unbuffered_flag() -> Result<()> {
    // Test that --unbuffered flag works (just verify it parses correctly)
    let (output, code) = run_jq_stdin(".", r#"{"a":1}"#, &["-c", "--unbuffered"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);
    Ok(())
}

#[test]
fn test_number_formatting_preserved() -> Result<()> {
    // Test that exponential notation is preserved with --preserve-input
    // By default, jq-compat mode normalizes numbers like jq does
    let (output, code) = run_jq_stdin(".", r#"{"val":4e4}"#, &["-c", "--preserve-input"])?;
    assert_eq!(code, 0);
    // With --preserve-input, should preserve "4e4" not convert to "40000"
    assert_eq!(output.trim(), r#"{"val":4e4}"#);
    Ok(())
}

#[test]
fn test_number_formatting_various() -> Result<()> {
    // Test various number formats are preserved with --preserve-input
    let (output, code) = run_jq_stdin(
        ".",
        r#"{"a":1.0e10,"b":2e-5,"c":3.14159}"#,
        &["-c", "--preserve-input"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1.0e10,"b":2e-5,"c":3.14159}"#);
    Ok(())
}

#[test]
fn test_number_formatting_field_access() -> Result<()> {
    // Test number formatting preserved through field access with --preserve-input
    let (output, code) = run_jq_stdin(".val", r#"{"val":4e4}"#, &["-c", "--preserve-input"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "4e4");
    Ok(())
}

#[test]
fn test_number_formatting_array_iteration() -> Result<()> {
    // Test number formatting preserved through array iteration with --preserve-input
    let (output, code) = run_jq_stdin(".[]", r"[1e100, 2e-100]", &["-c", "--preserve-input"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1e100\n2e-100");
    Ok(())
}

/// #478 item 3: `jq --preserve-input '.'` in pretty mode (no `-c`) used to
/// collapse duplicate object keys via `standard_json_to_jq_value`'s
/// `IndexMap`, unlike its own `-c` output. Fixed incidentally by #532's
/// `Expr::Identity => GenericResult::OneCursor` change, which routes
/// identity through the cursor-based `print_json` path in both modes —
/// pinned here as a regression guard rather than a code change.
#[test]
fn test_preserve_input_pretty_preserves_duplicate_keys() -> Result<()> {
    let input = r#"{"a":1,"a":2}"#;

    let (compact, code) = run_jq_stdin(".", input, &["-c", "--preserve-input"])?;
    assert_eq!(code, 0);
    assert_eq!(compact.trim(), r#"{"a":1,"a":2}"#);

    let (pretty, code) = run_jq_stdin(".", input, &["--preserve-input"])?;
    assert_eq!(code, 0);
    assert_eq!(pretty, "{\n  \"a\": 1,\n  \"a\": 2\n}\n");

    Ok(())
}

/// #607: `first(.[])`/`last(.[])` (the `Expr::FirstExpr`/`LastExpr` one-arg
/// stream form `first(f)`/`last(f)` compiles to — distinct from the bare
/// zero-arg `first`/`last` keyword, which is `Builtin::First`/`Last`) had no
/// native arm in `eval_generic::eval_single`, so it fell through the
/// catch-all `to_owned()` bridge and collapsed duplicate keys inside the
/// selected element before the extraction even ran — unlike `.[0]`, which
/// #532 already routed through the cursor-preserving path.
#[test]
fn test_preserve_input_first_last_expr_preserve_duplicate_keys() -> Result<()> {
    let input = r#"[{"a":1,"a":2},{"b":3,"b":4}]"#;

    let (first_compact, _, code) =
        run_jq_full(&["-c", "--preserve-input", "first(.[])"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(first_compact.trim(), r#"{"a":1,"a":2}"#);

    let (first_pretty, _, code) = run_jq_full(&["--preserve-input", "first(.[])"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(first_pretty, "{\n  \"a\": 1,\n  \"a\": 2\n}\n");

    let (last_compact, _, code) =
        run_jq_full(&["-c", "--preserve-input", "last(.[])"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(last_compact.trim(), r#"{"b":3,"b":4}"#);

    let (last_pretty, _, code) = run_jq_full(&["--preserve-input", "last(.[])"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(last_pretty, "{\n  \"b\": 3,\n  \"b\": 4\n}\n");

    Ok(())
}

/// #607: the bare zero-arg `first`/`last` keyword (`Builtin::First`/`Last`,
/// equivalent to `.[0]`/`.[-1]`) called `elements.get(...)` instead of the
/// `get_cursor(...)` sibling `.[0]`/`.[-1]` (`Expr::Index`) already used.
#[test]
fn test_preserve_input_bare_first_last_preserve_duplicate_keys() -> Result<()> {
    let input = r#"[{"a":1,"a":2},{"b":3,"b":4}]"#;

    let (first, _, code) = run_jq_full(&["-c", "--preserve-input", "first"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(first.trim(), r#"{"a":1,"a":2}"#);

    let (last, _, code) = run_jq_full(&["-c", "--preserve-input", "last"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(last.trim(), r#"{"b":3,"b":4}"#);

    Ok(())
}

/// #607: computed-key indexing (`.[$k]`/`.[(expr)]`, `Expr::IndexExpr`) went
/// through `index_one_generic`'s old `fields.find`/`elements.get` calls —
/// the same bug class as `first`/`last` above but reached via a different
/// code path (`eval_index_expr`). `.[(1-1)]` forces a genuinely computed
/// index rather than one the parser folds into a literal `Expr::Index`.
#[test]
fn test_preserve_input_computed_index_preserves_duplicate_keys() -> Result<()> {
    let input = r#"[{"a":1,"a":2},{"b":3,"b":4}]"#;

    let (compact, _, code) = run_jq_full(&["-c", "--preserve-input", ".[(1-1)]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(compact.trim(), r#"{"a":1,"a":2}"#);

    Ok(())
}

#[test]
fn test_jq_compat_default() -> Result<()> {
    // Test that jq-compat is now the default behavior
    // Numbers should be formatted like jq does (normalized scientific notation)
    let (output, code) = run_jq_stdin(".", r#"{"val":4e4}"#, &["-c"])?;
    assert_eq!(code, 0);
    // Default jq-compat normalizes 4e4 to 4E+4 (like jq)
    assert_eq!(output.trim(), r#"{"val":4E+4}"#);
    Ok(())
}

#[test]
fn test_large_integer_literal_prints_like_jq() -> Result<()> {
    // Integer literals beyond i64 degrade to floats like jq (issue #166):
    // jq -n '9999999999999999999' => 10000000000000000000
    let (output, code) = run_jq_stdin("9999999999999999999", "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "10000000000000000000");
    Ok(())
}

#[test]
fn test_args_positional() -> Result<()> {
    // Test --args: positional args become $ARGS.positional
    // Note: Use pipe syntax since parser doesn't support $VAR.field directly
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "-c",
            "$ARGS | .positional",
            "--args",
            "hello",
            "world",
        ])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), r#"["hello","world"]"#);
    Ok(())
}

#[test]
fn test_jsonargs_positional() -> Result<()> {
    // Test --jsonargs: positional args are parsed as JSON
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "-c",
            "$ARGS | .positional",
            "--jsonargs",
            "123",
            "true",
            r#"{"x":1}"#,
        ])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), r#"[123,true,{"x":1}]"#);
    Ok(())
}

#[test]
fn test_args_named() -> Result<()> {
    // Test $ARGS.named contains all named args
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "--arg",
            "name",
            "Alice",
            "--arg",
            "age",
            "30",
            "$ARGS | .named",
        ])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let parsed: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(parsed["name"], "Alice");
    assert_eq!(parsed["age"], "30");
    Ok(())
}

#[test]
fn test_args_combined() -> Result<()> {
    // Test $ARGS with both named and positional args
    // Named args first, then filter, then --args with values
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "--arg",
            "x",
            "1",
            "$ARGS",
            "--args",
            "a",
            "b",
        ])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let parsed: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(parsed["named"]["x"], "1");
    assert_eq!(parsed["positional"][0], "a");
    assert_eq!(parsed["positional"][1], "b");
    Ok(())
}

// =============================================================================
// Environment Variable Tests
// =============================================================================

#[test]
fn test_no_color_env_var() -> Result<()> {
    // Test that NO_COLOR environment variable disables color output
    // When NO_COLOR is set and no explicit -C/-M flag is given, colors should be disabled.
    let mut child = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            ".", // No -C or -M flag
        ])
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(b"{\"a\":1}")?;
    }
    let result = child.wait_with_output()?;
    let stdout = String::from_utf8(result.stdout)?;

    // Without -C flag and with NO_COLOR set, output should not contain ANSI codes
    assert!(
        !stdout.contains("\x1b["),
        "Output should not contain ANSI escape codes when NO_COLOR is set"
    );
    Ok(())
}

#[test]
fn test_jq_colors_env_var() -> Result<()> {
    // Test that JQ_COLORS environment variable customizes colors
    // Format: "null:false:true:numbers:strings:arrays:objects:objectkeys"
    // Use a distinctive color for null (red = 31) to verify it works
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-C", // Force color output
            "-n",
            "null",
        ])
        .env("JQ_COLORS", "0;31:::::::") // Red null, defaults for rest
        .stdout(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    // Check that the red color code (31) is present for null
    assert!(
        stdout.contains("\x1b[0;31m"),
        "Output should contain custom red color for null"
    );
    Ok(())
}

#[test]
fn test_color_output_overrides_no_color() -> Result<()> {
    // Test that -C flag overrides NO_COLOR env var
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-C", // Force color
            "-n",
            r#"{"a":1}"#,
        ])
        .env("NO_COLOR", "1") // This should be overridden by -C
        .stdout(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    // -C should force colors even with NO_COLOR set
    assert!(
        stdout.contains("\x1b["),
        "Output should contain ANSI codes when -C is used"
    );
    Ok(())
}

#[test]
fn test_monochrome_overrides_jq_colors() -> Result<()> {
    // Test that -M flag disables colors even if JQ_COLORS is set
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-M", // Monochrome output
            "-n",
            r#"{"a":1}"#,
        ])
        .env("JQ_COLORS", "0;31:0;32:0;33:0;34:0;35:0;36:0;37:0;38")
        .stdout(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    // -M should disable all colors
    assert!(
        !stdout.contains("\x1b["),
        "Output should not contain ANSI codes when -M is used"
    );
    Ok(())
}

#[test]
fn test_jq_colors_invalid_spec_warns_and_uses_defaults() -> Result<()> {
    // A malformed JQ_COLORS spec is rejected as a whole: jq warns on stderr,
    // falls back to the default scheme, and still exits successfully.
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-C",
            "-n",
            "null",
        ])
        .env("JQ_COLORS", "bogus")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("Failed to set $JQ_COLORS"),
        "expected warning on stderr: {stderr:?}"
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.contains("\x1b[1;30mnull\x1b[0m"),
        "null should use the default color: {stdout:?}"
    );
    Ok(())
}

#[test]
fn test_color_output_materializes_cursor_values() -> Result<()> {
    // A non-identity filter yields a cursor result; with -C it must take the
    // materialize-and-colorize path rather than the streaming printer.
    let (output, code) = run_jq_stdin(".a", r#"{"a":[1,false]}"#, &["-c", "-C"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output.trim(),
        "\x1b[1;39m[\x1b[0m\x1b[0;39m1\x1b[0m,\x1b[0;39mfalse\x1b[0m\x1b[1;39m]\x1b[0m"
    );
    Ok(())
}

#[test]
fn test_build_configuration_flag() -> Result<()> {
    // --build-configuration prints diagnostics and exits successfully.
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "--build-configuration",
        ])
        .stdout(Stdio::piped())
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.starts_with("succinctly jq build configuration:"),
        "unexpected header: {stdout:?}"
    );
    assert!(stdout.contains("Features:"));
    Ok(())
}

// =============================================================================
// Module System Tests
// =============================================================================

#[test]
fn test_include_directive() -> Result<()> {
    // Create a temporary module file
    let temp_dir = tempfile::tempdir()?;
    let module_path = temp_dir.path().join("utils.jq");
    std::fs::write(&module_path, "def double: . * 2;")?;

    // Test include directive
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "-L",
        ])
        .arg(temp_dir.path())
        .arg(r#"include "utils"; 21 | double"#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error"),
        "Should parse include directive without error: {stderr}"
    );
    assert_eq!(stdout.trim(), "42", "21 | double should equal 42");
    Ok(())
}

#[test]
fn test_import_directive() -> Result<()> {
    // Create a temporary module file
    let temp_dir = tempfile::tempdir()?;
    let module_path = temp_dir.path().join("mymod.jq");
    std::fs::write(&module_path, "def triple: . * 3;")?;

    // Test import directive with namespaced function call
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "-L",
        ])
        .arg(temp_dir.path())
        .arg(r#"import "mymod" as m; 10 | m::triple"#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error"),
        "Should parse import directive without error: {stderr}"
    );
    assert_eq!(stdout.trim(), "30", "10 | m::triple should equal 30");
    Ok(())
}

#[test]
fn test_library_path_option() -> Result<()> {
    // Test -L option with a non-existent path (should still parse)
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "-L",
            "/nonexistent/path",
            ".",
        ])
        .stdout(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), "null");
    Ok(())
}

#[test]
fn test_jq_library_path_env() -> Result<()> {
    // Create a temporary module directory
    let temp_dir = tempfile::tempdir()?;
    let module_path = temp_dir.path().join("envmod.jq");
    std::fs::write(&module_path, "def quadruple: . * 4;")?;

    // Test JQ_LIBRARY_PATH environment variable
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
        ])
        .arg(r#"include "envmod"; 5 | quadruple"#)
        .env("JQ_LIBRARY_PATH", temp_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error"),
        "Should parse include directive without error: {stderr}"
    );
    assert_eq!(stdout.trim(), "20", "5 | quadruple should equal 20");
    Ok(())
}

#[test]
fn test_module_not_found_error() -> Result<()> {
    // Test that a missing module produces an appropriate error
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
        ])
        .arg(r#"include "nonexistent_module_xyz"; ."#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stderr = String::from_utf8(output.stderr)?;

    // Should produce a module error
    assert!(
        stderr.contains("module") && stderr.contains("not found"),
        "Should report module not found error: {stderr}"
    );
    Ok(())
}

#[test]
fn test_namespaced_call_parse() -> Result<()> {
    // Test that namespaced calls parse correctly
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
        ])
        .arg("mymod::func")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stderr = String::from_utf8(output.stderr)?;

    // Should parse but fail at runtime (module not loaded)
    // Not a compile error, but an eval error
    assert!(
        !stderr.contains("compile error"),
        "Should parse namespaced call without compile error: {stderr}"
    );
    Ok(())
}

#[test]
fn test_home_jq_file_autoload() -> Result<()> {
    // Create a temporary home directory with a .jq file
    let temp_home = tempfile::tempdir()?;
    let jq_file = temp_home.path().join(".jq");
    std::fs::write(&jq_file, "def my_custom_func: . * 100;")?;

    // Test that function from ~/.jq is available
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "5 | my_custom_func",
        ])
        .env("HOME", temp_home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    // Should successfully execute the function
    assert!(
        !stderr.contains("compile error"),
        "Should not have compile error: {stderr}"
    );

    // The function should multiply by 100
    assert_eq!(
        stdout.trim(),
        "500",
        "my_custom_func should multiply by 100"
    );
    Ok(())
}

#[test]
fn test_home_jq_dir_search_path() -> Result<()> {
    // Create a temporary home directory with a .jq directory containing modules
    let temp_home = tempfile::tempdir()?;
    let jq_dir = temp_home.path().join(".jq");
    std::fs::create_dir(&jq_dir)?;
    std::fs::write(jq_dir.join("homemod.jq"), "def home_func: . + 1000;")?;

    // Test that module from ~/.jq directory can be included
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
        ])
        .arg(r#"include "homemod"; 7 | home_func"#)
        .env("HOME", temp_home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error") && !stderr.contains("module error"),
        "Should find module in ~/.jq directory: {stderr}"
    );
    assert_eq!(stdout.trim(), "1007", "7 | home_func should equal 1007");
    Ok(())
}

#[test]
fn test_import_with_namespace() -> Result<()> {
    // Create a temporary module directory
    let temp_dir = tempfile::tempdir()?;
    let module_path = temp_dir.path().join("mymath.jq");
    std::fs::write(&module_path, "def double: . * 2; def triple: . * 3;")?;

    // Test import with namespace - should be able to call mymath::double
    let output = Command::new("cargo")
        .args([
            "run",
            "--features",
            "cli",
            "--bin",
            "succinctly",
            "--",
            "jq",
            "-n",
            "-L",
        ])
        .arg(temp_dir.path())
        .arg(r#"import "mymath" as mymath; 5 | mymath::double"#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    // Should not have errors
    assert!(
        !stderr.contains("compile error") && !stderr.contains("module error"),
        "Should import module and use namespaced function: {stderr}"
    );

    // Should output 10 (5 * 2)
    assert_eq!(stdout.trim(), "10", "mymath::double should multiply by 2");
    Ok(())
}

// =============================================================================
// Stream operators: `//`, `and`, `or` (#160)
// =============================================================================

#[test]
fn test_stream_operators_emit_every_output() -> Result<()> {
    // Exact stdout including trailing newlines — the output *count* is the
    // point, so `.trim()` would hide the bug (#160).
    for (filter, expected) in [
        (r#"(false,1,null,2) // "backup""#, "1\n2\n"),
        ("false // (null,7)", "null\n7\n"),
        ("(true,false) and (true,false)", "true\nfalse\nfalse\n"),
        ("(true,false) or (true,false)", "true\ntrue\nfalse\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "null", &["-c"])?;
        assert_eq!(code, 0, "`{filter}` should succeed");
        assert_eq!(stdout, expected, "wrong output stream for `{filter}`");
    }
    Ok(())
}

#[test]
fn test_boolean_with_empty_operand_is_silent() -> Result<()> {
    // An empty operand used to reach `result_to_owned`, which reported it as
    // `Error("no value")` and printed a diagnostic. jq emits nothing at all,
    // quietly. The goldens compare stdout only, so stderr is asserted here.
    //
    // Matching on the absence of a diagnostic rather than on a byte-empty
    // stderr keeps the test independent of whatever else may reach that stream:
    // `--quiet` silences cargo's own progress lines, but not a rustc warning
    // from the build it triggers. Requiring stderr to be empty would couple this
    // assertion to the whole workspace compiling warning-free, which is not what
    // it is testing.
    let (stdout, stderr, code) = run_jq_stdin_streams("empty and true", "null", &["-c"])?;
    assert_eq!(stdout, "", "expected no output");
    assert!(
        !stderr.contains("jq: error"),
        "expected no diagnostic on stderr, got: {stderr}"
    );
    assert_eq!(code, 0, "expected success");
    Ok(())
}

// =============================================================================
// JSON Sequence Format (RFC 7464) Tests
// =============================================================================

/// Helper to run jq command with binary input (for testing --seq with RS characters)
fn run_jq_binary_stdin(filter: &str, input: &[u8], extra_args: &[&str]) -> Result<(Vec<u8>, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let mut cmd = Command::new("cargo")
            .args([
                "run",
                "--features",
                "cli",
                "--bin",
                "succinctly",
                "--",
                "jq",
            ])
            .args(extra_args)
            .arg(filter)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = cmd.stdin.take() {
            stdin.write_all(input)?;
        }

        let output = cmd.wait_with_output()?;
        let exit_code = output.status.code().unwrap_or(-1);

        // Exit code 101 often indicates cargo lock contention; retry
        if exit_code == 101 && attempt + 1 < MAX_CARGO_RETRIES {
            std::thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
            continue;
        }

        return Ok((output.stdout, exit_code));
    }
    unreachable!()
}

#[test]
fn test_seq_output_format() -> Result<()> {
    // --seq should prepend RS (0x1E) before each output value
    let (output, code) = run_jq_binary_stdin(".", br#"{"a":1}"#, &["--seq", "-c"])?;
    assert_eq!(code, 0);

    // Output should start with RS (0x1E)
    assert!(
        output.starts_with(&[0x1E]),
        "Output should start with RS (0x1E), got: {:?}",
        &output[..output.len().min(10)]
    );

    // Rest should be the JSON value
    let rest = String::from_utf8(output[1..].to_vec())?;
    assert_eq!(rest.trim(), r#"{"a":1}"#);
    Ok(())
}

#[test]
fn test_seq_input_parsing() -> Result<()> {
    // --seq should parse RS-separated input values
    let mut input = Vec::new();
    input.push(0x1E); // RS
    input.extend_from_slice(br#"{"x":1}"#);
    input.push(b'\n');
    input.push(0x1E); // RS
    input.extend_from_slice(br#"{"x":2}"#);
    input.push(b'\n');

    let (output, code) = run_jq_binary_stdin(".x", &input, &["--seq"])?;
    assert_eq!(code, 0);

    // Should have two RS-prefixed outputs
    let output_str = String::from_utf8(output)?;
    let lines: Vec<_> = output_str.lines().collect();

    // Each line should start with RS followed by the value
    assert!(lines.len() >= 2, "Should have at least 2 output lines");
    Ok(())
}

#[test]
fn test_seq_ignores_parse_errors() -> Result<()> {
    // RFC 7464 recommends silently ignoring parse errors
    let mut input = Vec::new();
    input.push(0x1E);
    input.extend_from_slice(br#"{"valid":1}"#);
    input.push(b'\n');
    input.push(0x1E);
    input.extend_from_slice(b"not valid json");
    input.push(b'\n');
    input.push(0x1E);
    input.extend_from_slice(br#"{"valid":2}"#);
    input.push(b'\n');

    let (output, code) = run_jq_binary_stdin(".valid", &input, &["--seq"])?;
    assert_eq!(code, 0);

    // Should only see outputs from valid JSON (1 and 2), not the invalid segment
    let output_str = String::from_utf8(output)?;
    assert!(
        output_str.contains('1'),
        "Should have output from first valid value"
    );
    assert!(
        output_str.contains('2'),
        "Should have output from second valid value"
    );
    Ok(())
}

#[test]
fn test_seq_multiple_outputs() -> Result<()> {
    // Each output from iterator should get RS prefix
    let (output, code) = run_jq_binary_stdin(".[]", br"[1,2,3]", &["--seq"])?;
    assert_eq!(code, 0);

    // Count RS characters - should be 3 (one per output)
    let rs_count = output.iter().filter(|&&b| b == 0x1E).count();
    assert_eq!(rs_count, 3, "Should have 3 RS characters for 3 outputs");
    Ok(())
}

#[test]
fn test_seq_with_slurp() -> Result<()> {
    // --seq with -s should slurp all seq inputs into an array
    let mut input = Vec::new();
    input.push(0x1E);
    input.extend_from_slice(b"1");
    input.push(b'\n');
    input.push(0x1E);
    input.extend_from_slice(b"2");
    input.push(b'\n');
    input.push(0x1E);
    input.extend_from_slice(b"3");
    input.push(b'\n');

    let (output, code) = run_jq_binary_stdin("add", &input, &["--seq", "-s"])?;
    assert_eq!(code, 0);

    // Output should be RS + "6" (1+2+3)
    assert!(output.starts_with(&[0x1E]), "Output should start with RS");
    let rest = String::from_utf8(output[1..].to_vec())?;
    assert_eq!(rest.trim(), "6");
    Ok(())
}

// =============================================================================
// Regex builtins (regression tests for #167 - cli feature must bundle regex)
// =============================================================================

#[test]
fn test_regex_test_available_in_cli() -> Result<()> {
    // The #167 repro: must use regex semantics, not substring matching
    let (output, code) = run_jq_stdin(r#"test("[0-9]+")"#, r#""abc123""#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "true");

    let (output, code) = run_jq_stdin(r#"test("[0-9]+")"#, r#""abc""#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "false");
    Ok(())
}

#[test]
fn test_regex_gsub_available_in_cli() -> Result<()> {
    let (output, code) = run_jq_stdin(r#"gsub("[0-9]"; "X")"#, r#""a1b2""#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""aXbX""#);
    Ok(())
}

#[test]
fn test_regex_capture_available_in_cli() -> Result<()> {
    let (output, code) = run_jq_stdin(
        r#"capture("(?<word>[a-z]+)") | .word"#,
        r#""hello123""#,
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""hello""#);
    Ok(())
}

#[test]
fn test_regex_capture_bare_form_no_match_produces_no_output() -> Result<()> {
    // #805: bare capture(re) printed `null` on no match; real jq prints nothing.
    let (output, code) = run_jq_stdin(r#"capture("[0-9]+")"#, r#""abc""#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "");
    Ok(())
}

#[test]
fn test_regex_capture_flags_form_no_match_produces_no_output() -> Result<()> {
    // #805: capture(re; flags)'s non-optional no-match branch printed `{}`;
    // real jq prints nothing there either.
    let (output, code) = run_jq_stdin(r#"capture("[0-9]+"; "")"#, r#""abc""#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "");
    Ok(())
}

#[test]
fn test_regex_match_bare_form_no_match_produces_no_output() -> Result<()> {
    // #810: bare match(re) printed `null` on no match; real jq prints nothing.
    let (output, code) = run_jq_stdin(r#"match("[0-9]+")"#, r#""abc""#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "");
    Ok(())
}

#[test]
fn test_regex_match_flags_form_no_match_produces_no_output() -> Result<()> {
    // #810: match(re; flags) printed `null` on no match; real jq prints nothing.
    let (output, code) = run_jq_stdin(r#"match("[0-9]+"; "")"#, r#""abc""#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "");
    Ok(())
}

// =============================================================================
// range() float support (issue #165)
// =============================================================================

#[test]
fn test_range_float_step() -> Result<()> {
    // Issue #165 repro: fractional step was truncated to 0, yielding [].
    // jq 1.7.1 accumulates doubles: [0,0.3,0.6,0.8999999999999999]
    let (output, code) = run_jq_null("[range(0;1;0.3)]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,0.3,0.6,0.8999999999999999]");
    Ok(())
}

#[test]
fn test_range_float_from() -> Result<()> {
    // Issue #165 repro: fractional lower bound was truncated to 2.
    let (output, code) = run_jq_null("[range(2.5;5)]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[2.5,3.5,4.5]");
    Ok(())
}

#[test]
fn test_range_int_unchanged() -> Result<()> {
    // All-integer ranges keep exact integer output.
    let (output, code) = run_jq_null("[range(0;10;2)]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,2,4,6,8]");
    Ok(())
}

#[test]
fn test_range_zero_step_empty() -> Result<()> {
    // jq 1.7.1 emits no values for a zero step rather than erroring.
    let (output, code) = run_jq_null("[range(0;1;0)]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[]");
    Ok(())
}

// Collecting an iterator pipe `[.[] | f]` must gather one output per element,
// matching `map(f)` and jq — not yield `[]` (issue #295). Expected outputs
// ground-truthed against jq-1.7.1.
#[test]
fn test_collect_iterator_pipe_issue_295() -> Result<()> {
    // Array construction over a computed inner filter, and equality with map.
    let (out, code) = run_jq_stdin("[.[] | . + 1]", r"[1,2,3]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[2,3,4]");
    let (out_map, _) = run_jq_stdin("map(. + 1)", r"[1,2,3]", &["-c"])?;
    assert_eq!(out.trim(), out_map.trim());

    // Object value context.
    let (out, code) = run_jq_stdin("{a: [.[] | . + 1]}", r"[1,2,3]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"a":[2,3,4]}"#);

    // Reduction over the collected pipe.
    let (out, code) = run_jq_stdin("[.[] | . + 1] | length", r"[1,2,3]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "3");

    // Owned array-construction inner filter.
    let (out, code) = run_jq_stdin("[.[] | [.]]", r"[1,2,3]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[[1],[2],[3]]");

    // first/1 over the array construction returns the single collected array.
    let (out, code) = run_jq_stdin("first([.[] | . + 1])", r"[1,2,3]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[2,3,4]");

    Ok(())
}

// ---------------------------------------------------------------------------
// #355: an uncaught evaluation error must fail the process
//
// Before this, a raised error printed a diagnostic and exited 0, so `set -e`,
// `&&` and `$?` all read a failed filter as success. Every expectation below
// was captured from jq 1.7.1 unless a comment says otherwise.
// ---------------------------------------------------------------------------

/// Write `contents` to a temp file and return it with its path.
fn temp_json(contents: &str) -> Result<(NamedTempFile, String)> {
    let mut file = NamedTempFile::new()?;
    file.write_all(contents.as_bytes())?;
    file.flush()?;
    let path = file.path().to_string_lossy().to_string();
    Ok((file, path))
}

#[test]
fn test_uncaught_error_exits_5() -> Result<()> {
    // jq: `jq: error (at <stdin>:0): boom`, exit 5. Line 0: the input has no
    // trailing newline, so jq's zero-based counter never advances (#524).
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"error("boom")"#], Some(r#"{"x":1}"#))?;
    assert_eq!(code, 5, "uncaught error must exit 5: {stderr}");
    assert_eq!(stdout, "", "a failed filter produces no output");
    assert_eq!(stderr.trim_end(), "jq: error (at <stdin>:0): boom");
    Ok(())
}

// `halt`, `halt_error`/`halt_error(n)`, and `stderr` (#791). Every byte-exact
// expectation below was captured directly from real `jq` (1.7.1 and 1.8.2,
// via `xxd` on separately-redirected stdout/stderr -- `2>&1` interleaves the
// two streams misleadingly, since stdout is buffered when piped but stderr
// is not) rather than assumed from the manual page wording.

#[test]
fn test_halt_exits_0_with_no_output() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "halt"], None)?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_halt_prints_outputs_produced_before_it() -> Result<()> {
    // Verified against jq 1.7.1/1.8.2: `jq -n '1,2,halt,3'` prints `1` and
    // `2` then exits 0 -- nothing after `halt` runs.
    let (stdout, stderr, code) = run_jq_full(&["-n", "1,2,halt,3"], None)?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "1\n2\n");
    Ok(())
}

#[test]
fn test_halt_error_string_prints_raw_with_no_trailing_newline() -> Result<()> {
    // Verified against jq 1.7.1/1.8.2: stderr is exactly `foo` (no quotes,
    // no newline); stdout is empty (halt_error never passes its value
    // through, unlike `stderr`); exit code defaults to 5.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""foo" | halt_error"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "foo");
    Ok(())
}

#[test]
fn test_halt_error_non_string_prints_compact_json_with_trailing_newline() -> Result<()> {
    // Verified against jq 1.7.1/1.8.2: non-string, non-null values print as
    // compact JSON *with* a trailing newline -- unlike the string case above.
    for (filter, want_stderr) in [
        (r#"[1,2,"a b"] | halt_error"#, "[1,2,\"a b\"]\n"),
        (r#"{"a":1} | halt_error"#, "{\"a\":1}\n"),
        ("false | halt_error", "false\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!(code, 5, "{filter}: stdout: {stdout:?} stderr: {stderr:?}");
        assert_eq!(stdout, "", "{filter}");
        assert_eq!(stderr, want_stderr, "{filter}");
    }
    Ok(())
}

#[test]
fn test_halt_error_null_prints_nothing() -> Result<()> {
    // Verified against jq 1.7.1/1.8.2: `null` is the one value `halt_error`
    // special-cases to print *nothing* at all (not even "null").
    let (stdout, stderr, code) = run_jq_full(&["-n", "null | halt_error(9)"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_halt_error_custom_exit_code() -> Result<()> {
    // Verified against jq 1.7.1/1.8.2: `halt_error(n)` exits with `n`, not
    // the default 5.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""foo" | halt_error(7)"#], None)?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "foo");
    Ok(())
}

#[test]
fn test_stderr_passes_through_and_prints_raw_compact_with_no_newline() -> Result<()> {
    // Verified against jq 1.7.1/1.8.2: `stderr` always writes with no
    // trailing newline (unlike halt_error's non-string case), raw for
    // strings and compact JSON for everything else including `null` (unlike
    // halt_error's null-skips-entirely rule) -- and passes its input through
    // unchanged to stdout via the normal output path.
    for (filter, want_stdout, want_stderr) in [
        (r#""hello" | stderr"#, "\"hello\"\n", "hello"),
        ("[1,2] | stderr", "[1,2]\n", "[1,2]"),
        ("null | stderr", "null\n", "null"),
        ("false | stderr", "false\n", "false"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", "-c", filter], None)?;
        assert_eq!(code, 0, "{filter}: stderr: {stderr:?}");
        assert_eq!(stdout, want_stdout, "{filter}");
        assert_eq!(stderr, want_stderr, "{filter}");
    }
    Ok(())
}

#[test]
fn test_halt_not_caught_by_try_catch_or_label() -> Result<()> {
    // Verified live against jq 1.7.1/1.8.2: none of these ever print
    // "caught", and the exit code is `halt`/`halt_error`'s own, not
    // whatever `try`/`catch`/`label` would otherwise produce.
    for (filter, want_code) in [
        (r#"try (halt) catch "caught""#, 0),
        (r#"try ("x"|halt_error) catch "caught""#, 5),
        (r"label $out | (halt, break $out)", 0),
        (r#"("x"|halt_error)? // "fallback""#, 5),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!(code, want_code, "{filter}: stderr: {stderr:?}");
        assert!(!stdout.contains("caught"), "{filter}: stdout: {stdout:?}");
        assert!(!stdout.contains("fallback"), "{filter}: stdout: {stdout:?}");
    }
    Ok(())
}

// Follow-up fixes to #791's halt-propagation sweep: a code-review pass found
// several `eval_single`-consuming sites that read `QueryResult` directly via
// a wildcard match instead of the new `result_to_owned`/`query_result_from_error`
// helpers, so a halt reaching them was misreported as an ordinary error, a
// bogus boolean, or silently swallowed. Every expectation below was verified
// live against jq 1.7.1.

#[test]
fn test_halt_not_caught_by_try_catch_in_path_expression() -> Result<()> {
    // `resolve_node`'s `Expr::Try` arm (the separate path-expression resolver
    // behind `path()`, `=`, `|=`, `del()`) used to treat a halt smuggled back
    // through `EvalError::halt_escape` as an ordinary catchable error.
    // Verified against jq 1.7.1: `jq -n '1, del(try halt_error(9) catch empty), 2'`
    // prints `1` and exits 9 -- the halt is never caught, so `2` never runs.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "1, del(try halt_error(9) catch empty), 2"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_halt_not_caught_by_bare_optional_in_path_expression() -> Result<()> {
    // `resolve_node`'s `Expr::Optional` blanket arm used to discard the
    // error object entirely -- including any halt marker inside it -- as if
    // it were a suppressed `?` failure. Verified against jq 1.7.1:
    // `jq -n 'del((halt_error(4))?)'` exits 4, not 0.
    let (stdout, stderr, code) = run_jq_full(&["-n", "del((halt_error(4))?)"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_halt_not_caught_by_try_catch_in_update_assignment() -> Result<()> {
    // Same root cause as the `path()`/`del()` cases above, reached via `|=`:
    // `eval_update`'s own top-level halt check never fires because the halt
    // is already gone by the time `resolve_node` returns. Verified against
    // jq 1.7.1: exits 9 after printing `1`.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "{\"a\":1} | 1, ((try halt_error(9) catch empty) |= .+1), 2",
        ],
        None,
    )?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_isempty_propagates_halt_instead_of_answering_false() -> Result<()> {
    // `builtin_isempty`'s wildcard arm used to answer `false` for a halt
    // with zero prior outputs, exiting 0. Verified against jq 1.7.1:
    // `jq -n 'isempty(halt_error(14))'` exits 14 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "isempty(halt_error(14))"], None)?;
    assert_eq!(code, 14, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_setpath_propagates_halt_in_path_argument() -> Result<()> {
    // `builtin_setpath`'s path-array argument used to misreport a halt as
    // "Path must be specified as an array". Verified against jq 1.7.1:
    // `jq -n 'setpath([(halt_error(6))]; 1)'` exits 6.
    let (stdout, stderr, code) = run_jq_full(&["-n", "setpath([(halt_error(6))]; 1)"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_setpath_propagates_halt_in_value_argument() -> Result<()> {
    // `builtin_setpath`'s value argument used to fall through to `null`,
    // silently writing `null` instead of halting. Verified against jq 1.7.1:
    // `jq -n '{} | setpath(["a"]; halt_error(13))'` exits 13 with no output.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"{} | setpath(["a"]; halt_error(13))"#], None)?;
    assert_eq!(code, 13, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_delpaths_propagates_halt_in_paths_argument() -> Result<()> {
    // `builtin_delpaths` had the identical gap as `builtin_setpath` above.
    // Verified against jq 1.7.1: `jq -n '{"a":1} | delpaths([(halt_error(7))])'`
    // exits 7.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "{\"a\":1} | delpaths([(halt_error(7))])"], None)?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_nth_stream_propagates_halt_in_n_argument() -> Result<()> {
    // The two-argument `nth(n; expr)` form matched its `n` argument's result
    // directly; the wildcard arm turned a halt into a generic type error.
    // Verified against jq 1.7.1: `jq -n 'nth(halt_error(12); 1,2,3)'` exits 12.
    let (stdout, stderr, code) = run_jq_full(&["-n", "nth(halt_error(12); 1,2,3)"], None)?;
    assert_eq!(code, 12, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_bsearch_propagates_halt_in_target_argument() -> Result<()> {
    // `builtin_bsearch`'s target-expression wildcard swallowed a halt as if
    // the target produced no value. Verified against jq 1.7.1:
    // `jq -n '[1,2,3] | bsearch(halt_error(15))'` exits 15.
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2,3] | bsearch(halt_error(15))"], None)?;
    assert_eq!(code, 15, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_parent_propagates_halt_in_n_argument() -> Result<()> {
    // `Builtin::ParentN`'s `n`-argument wildcard swallowed a halt smuggled
    // back through the `EvalEscape` a `Result<_, EvalEscape>`-typed helper
    // returns, treating it as an ordinary `optional` failure and answering
    // `QueryResult::None` instead. `parent` is a succinctly extension (real
    // jq has no such builtin), so this is checked against succinctly's own
    // halt-propagation contract rather than jq: `parent(halt_error(9))?`
    // must still exit 9, not 0, matching every other `?`-suppressible
    // builtin argument in this file.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", ".a.b | parent(halt_error(9))?"],
        Some(r#"{"a":{"b":1}}"#),
    )?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_path_context_optional_does_not_swallow_halt_in_builtin_arm() -> Result<()> {
    // `eval_pipe_with_path_context_internal`'s `Expr::Builtin` arm (reached
    // whenever a pipe needs `key`/`parent`/`file_index`/`path` tracking) used
    // to check `optional` before checking for a smuggled-back halt, the same
    // shape of bug `ParentN` had one arm over. Verified live against jq
    // 1.7.1's contract that `?` never suppresses `halt`/`halt_error`:
    // `.a.b | (has(halt_error(9)))?, key` must exit 9 with no output --
    // `.a.b`'s value feeds the pipe's right side as input, it is never
    // itself printed, and the halt inside `has(...)`'s argument aborts
    // before the comma's `key` branch ever runs.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", ".a.b | (has(halt_error(9)))?, key"],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_path_context_optional_does_not_swallow_halt_in_object_literal_arm() -> Result<()> {
    // Same bug shape as the `Expr::Builtin` arm above, one arm over: object
    // construction inside a path-context pipe.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", ".a.b | ({x: halt_error(9)})?, key"],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_compound_assign_propagates_halt_after_partial_rhs_output() -> Result<()> {
    // `eval_rhs_once`'s `Partial(vs, _control)` arm took the RHS stream's
    // first output and silently dropped a trailing halt, so a compound
    // assignment whose RHS produced a value and *then* halted finished the
    // assignment and exited 0. Verified against jq 1.7.1: `{"a":0} | .a +=
    // (1, halt_error(3))` exits 3 with no output at all -- the halt fires
    // while still computing the RHS, before `+=` ever produces the modified
    // document that would otherwise be this input's one output.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", ".a += (1, halt_error(3))"], Some(r#"{"a":0}"#))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_isvalid_propagates_halt_instead_of_answering_true() -> Result<()> {
    // `builtin_isvalid`'s wildcard `_ => Bool(true)` arm swallowed
    // `QueryResult::Halt`, answering `true` and letting evaluation continue
    // -- the same bug class already fixed for `isempty` above.
    let (stdout, stderr, code) = run_jq_full(&["-n", "isvalid(halt_error(3)), \"after\""], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_isvalid_propagates_halt_from_error_message_expression() -> Result<()> {
    // A distinct site from the one above: `isvalid` forces `optional=true`
    // down its whole subtree, and `eval_error`'s own `Err(_) if optional`
    // arm used to swallow a halt reached while evaluating `error(msg)`'s
    // message expression before `isvalid` ever saw the result. Fixing
    // `isvalid`'s own wildcard alone does not fix this one.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "isvalid(error(halt_error(3))), \"after\""], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_range_halt_not_caught_by_try_catch() -> Result<()> {
    // `range_arg`'s wildcard arm downgraded a halt in a range bound to a
    // fresh "Range bounds must be numeric" `EvalError`, making it an
    // ordinary catchable error -- the clearest possible violation of the
    // "halt is never caught" contract, since `try`/`catch` is the mechanism
    // that must never see it.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "try (range(halt_error(3))) catch \"caught\""], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_pow_propagates_halt_in_argument() -> Result<()> {
    // `get_number_from_result`'s wildcard arm -- shared by every two-arg
    // math builtin (`pow`, `atan2`, ...) -- consumed a halt and reported
    // "expected number" instead.
    let (stdout, stderr, code) = run_jq_full(&["-n", "pow(halt_error(3); 2)"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_repeat_propagates_halt_instead_of_running_to_iteration_cap() -> Result<()> {
    // `eval_owned_expr_ctrl`'s `Partial(vs, _control)` arm collapsed a
    // multi-output result to first-value-or-array and dropped a trailing
    // halt, so a `repeat` body that output a value and then halted on the
    // *same* iteration kept looping instead of stopping -- silently
    // discarding the halt every round until hitting the internal iteration
    // cap. `repeat` is a succinctly extension (no upstream jq builtin), so
    // this is checked against succinctly's own contract: the halt must win
    // on the very first iteration, giving exit 3 and no output, not a
    // 1000-element array and exit 0.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "[repeat(if . > 2 then error(\"done\") else (. + 1, halt_error(3)) end)]?",
        ],
        None,
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_halt_error_document_sourced_float_exit_code() -> Result<()> {
    // `builtin_halt_error`'s numeric-code match covered `Float` and
    // `NumberLiteral(Int)` but not `NumberLiteral(Float)` -- what `to_owned`
    // produces for every non-integer number read from a document -- so a
    // float exit code from `.` raised "requires a number exit code" instead
    // of halting. Verified against jq 1.7.1: `echo 2.5 | jq 'halt_error(.)'`
    // writes `2.5` to stderr and exits 2 (the float truncated toward zero).
    let (stdout, stderr, code) = run_jq_full(&["halt_error(.)"], Some("2.5"))?;
    assert_eq!(code, 2, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "2.5\n");
    Ok(())
}

#[test]
fn test_stderr_nan_prints_json_null_not_text_format() -> Result<()> {
    // `builtin_stderr` used `owned_to_string` -- the string-interpolation
    // renderer, which spells non-finite floats `NaN`/`inf` -- instead of
    // `to_json`'s compact-JSON convention. Verified against jq 1.7.1:
    // `jq -n 'nan | stderr'` writes literal `null` to stderr, matching how
    // JSON represents (the un-representable) NaN.
    let (stdout, stderr, code) = run_jq_full(&["-n", "-c", "nan | stderr"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stderr, "null");
    assert_eq!(stdout, "null\n");
    Ok(())
}

#[test]
fn test_debug_msg_argument_halt_propagates() -> Result<()> {
    // `builtin_debug_msg` never evaluated its `msg` argument at all (a
    // pre-existing library-context no-print stub), which after halt/
    // halt_error's introduction made `debug(msg)` the one builtin argument
    // position where a halt had zero effect -- no stderr write, no exit
    // code. Verified against jq 1.7.1: `jq -n 'debug(halt_error(3))'` exits
    // 3 (the pre-existing no-print policy for `msg`'s *text* is unchanged;
    // only its control effects must reach the process).
    let (stdout, stderr, code) = run_jq_full(&["-n", "debug(halt_error(3)), \"after\""], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_debug_msg_argument_error_propagates() -> Result<()> {
    // Code-review follow-up (#791): the fix above only forwarded a `Halt`
    // out of `msg`'s evaluation, matching only the `Err(EvalEscape::Halt)`
    // arm and silently discarding a plain `Err(EvalEscape::Error(_))` the
    // same way it discarded a successful `Ok(_)` -- so `debug(error(...))`
    // printed the original input and exited 0 instead of erroring. Real
    // jq's `builtin.jq` defines `debug(msg)` as a plain pipe
    // (`(msg|debug|empty), .`, no `try`/`?`), so an error while evaluating
    // `msg` aborts the whole expression. Verified against jq 1.7.1:
    // `echo 1 | jq 'debug(error("boom"))'` exits 5 with the error on
    // stderr, never reaching stdout.
    let (stdout, stderr, code) = run_jq_full(&["debug(error(\"boom\"))"], Some("1"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("boom"), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_paths_filter_halt_on_scalar_root() -> Result<()> {
    // `builtin_paths_filter` only evaluated `filter` against nodes reached
    // by non-root paths, never the root itself, so for an input with no
    // non-root paths (`null`, a scalar, an empty container) `filter` never
    // ran at all -- `paths(halt_error(3))` was silently never invoked.
    // Verified against jq 1.7.1: `jq -n 'null | [paths(halt_error(3))]'`
    // exits 3, since real jq's `paths(node_filter)` evaluates `node_filter`
    // against every node `recurse` visits, root included, even though the
    // root's own path never appears in the output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "null | [paths(halt_error(3))]"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// Companion to the halt test above (#850): an ordinary error on the root
/// itself must also abort the whole builtin, matching real jq. Before this
/// fix, `builtin_paths_filter`'s root-value precheck only special-cased
/// `Halt` -- an `Error` or `Break` from evaluating `filter` against the
/// root fell through silently, so the root's own escape was discarded
/// entirely rather than just missing (unlike the halt case, which at least
/// never ran `filter` on the root at all for a scalar/empty-container
/// input). Verified against jq 1.7.1: `1 | [paths(error("x"))]` raises
/// immediately with no output.
#[test]
fn test_paths_filter_aborts_on_scalar_root_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "1 | [paths(error(\"x\"))]"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "jq: error (at <unknown>): x\n");
    Ok(())
}

/// Sharper than the scalar-root case above: here the root filter's error is
/// reached only via a `node_filter` that also matches a legitimate child
/// path (`"a"`), so a swallowed root error wouldn't just be a missing
/// error -- it would produce *wrong* output (`["a"]`) as if the root's own
/// escape had never happened. Verified against jq 1.7.1: `{"a":1} |
/// paths(if type=="object" then error("root-err") else true end)` raises
/// immediately, never reaching the child path at all.
#[test]
fn test_paths_filter_root_error_aborts_before_child_paths() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"paths(if type=="object" then error("root-err") else true end)"#,
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "jq: error (at <stdin>:0): root-err\n");
    Ok(())
}

/// Same root-level fix, but for `break`: unlike an ordinary error, a `break
/// $label` from the root's own `node_filter` evaluation must unwind to an
/// enclosing `label`, not surface as any kind of error. This specifically
/// exercises why the root precheck uses `eval_owned_expr_ctrl` rather than
/// `eval_owned_expr`: the latter collapses `Control::Break` into a
/// synthetic "break $label not in label" `EvalError` (#833), which would
/// make this case report a bogus error instead of unwinding cleanly.
/// Verified against jq 1.7.1: `label $out | 1 | [paths(break $out)]`
/// produces no output and exits 0.
#[test]
fn test_paths_filter_root_break_unwinds_to_outer_label() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "label $out | 1 | [paths(break $out)]"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_halt_error_negative_exit_code_floors_to_zero() -> Result<()> {
    // Real jq clamps any negative `halt_error(n)` argument to exit code 0,
    // rather than letting the OS's usual two's-complement byte-truncation of
    // a negative process-exit status produce a nonzero code. Verified live
    // against jq 1.7.1 for every value below.
    for filter in [
        r#""x" | halt_error(-1)"#,
        r#""x" | halt_error(-100)"#,
        r#""x" | halt_error(-2147483648)"#,
        r#""x" | halt_error(-infinite)"#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!(code, 0, "{filter}: stdout: {stdout:?} stderr: {stderr:?}");
    }
    Ok(())
}

#[test]
fn test_halt_error_positive_and_special_exit_codes_unaffected() -> Result<()> {
    // Sanity check that the negative-floor fix above didn't disturb the
    // already-correct positive/NaN/+infinity handling. Verified against jq
    // 1.7.1. Platform-independent rows only; see the `#[cfg(unix)]` block
    // below for the two that depend on Unix's exit-status byte truncation.
    for (filter, want_code) in [
        (r#""x" | halt_error(nan)"#, 0),
        (r#""x" | halt_error(7)"#, 7),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!(
            code, want_code,
            "{filter}: stdout: {stdout:?} stderr: {stderr:?}"
        );
    }

    // `halt_error`'s own saturating cast to i32 (see its doc comment) is
    // platform-independent and matches real jq's untruncated i32::MAX exit
    // code on every OS. What's Unix-specific is only the *observed* exit
    // code here: `WEXITSTATUS` truncates `process::exit(i32::MAX)` to its
    // low byte (255), while Windows' `ExitStatus::code()` returns the full
    // i32 untouched. There is no Windows CI leg exercising this today, so
    // this gate documents a latent portability difference rather than
    // guarding an active failure.
    #[cfg(unix)]
    for (filter, want_code) in [
        (r#""x" | halt_error(4294967296)"#, 255),
        (r#""x" | halt_error(infinite)"#, 255),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!(
            code, want_code,
            "{filter}: stdout: {stdout:?} stderr: {stderr:?}"
        );
    }

    Ok(())
}

#[test]
fn test_computed_index_streams_keys_produced_before_halt() -> Result<()> {
    // `eval_index_expr`'s key stream used to discard already-produced keys
    // when the key generator itself later halted, printing nothing instead
    // of the indexed values for the keys already yielded. Verified against
    // jq 1.7.1: `echo '[10,20,30]' | jq '.[(1,2,halt)]'` prints `20` then
    // `30` before halting. Piped stdin input exercises the cursor-based
    // evaluator (`eval_generic.rs`), distinct from the `-n` literal-array
    // path (`eval.rs`) -- both must agree.
    let (stdout, stderr, code) = run_jq_full(&[".[(1,2,halt)]"], Some("[10,20,30]"))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout, "20\n30\n");

    let (stdout, stderr, code) = run_jq_full(&["-n", "[10,20,30] | .[(1,2,halt)]"], None)?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout, "20\n30\n");
    Ok(())
}

#[test]
fn test_computed_index_still_conservative_on_error_and_break() -> Result<()> {
    // The fix above is deliberately halt-specific: `Error`/`Break` keep the
    // pre-existing, documented conservative behavior (discard the prefix
    // rather than stream it), matching real jq's own gap here -- see
    // `eval_index_expr`'s doc comment. `Error` still aborts the whole
    // process (exit 5), it just doesn't print `20`/`30` first.
    let (stdout, stderr, code) = run_jq_full(&[r#".[(1,2,error("boom"))]"#], Some("[10,20,30]"))?;
    assert_eq!(code, 5);
    assert_eq!(stdout, "");
    assert!(stderr.contains("boom"));
    Ok(())
}

#[test]
fn test_computed_index_target_error_after_pending_halt_still_streams_prefix() -> Result<()> {
    // #791 follow-up: unlike the key-stream's own error/break above, a later
    // key's *index* error (not the key stream itself) had no test coverage
    // and dropped its already-indexed prefix -- and, since the key stream
    // here already recorded `pending_halt` before any indexing happened, it
    // discarded that too. Verified against jq 1.7.1/1.8.2: `{"a":1} |
    // .[("a", 5, halt)]` prints `1`, then errors "Cannot index object with
    // number" (exit 5) -- jq's interleaved generator means the type error on
    // key `5` fires before the key stream ever reaches `halt`. Piped stdin
    // exercises `eval_generic.rs`; `-n` exercises `eval.rs` -- both must
    // agree.
    let (stdout, stderr, code) = run_jq_full(&[r#".[("a", 5, halt)]"#], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5);
    assert_eq!(stdout, "1\n");
    assert!(
        stderr.contains("Cannot index object with number"),
        "{stderr}"
    );

    let (stdout, stderr, code) = run_jq_full(&["-n", r#"{"a":1} | .[("a", 5, halt)]"#], None)?;
    assert_eq!(code, 5);
    assert_eq!(stdout, "1\n");
    assert!(
        stderr.contains("Cannot index object with number"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_693_optional_around_stream_stops_at_the_first_error() -> Result<()> {
    // The `jq`/`yq` CLIs' default path evaluates through `eval_generic`'s
    // native cursor-based evaluator (`jq_runner.rs`'s `evaluate_input`/
    // `evaluate_bytes_lazy`), not the `eval.rs`-level `eval()` API directly
    // — so a fix that only touched `eval.rs` would leave this reachable
    // through the shipped binary. Verified against jq 1.7.1: `jq -n '[1,2,3]
    // | (.[] | if .==2 then error("boom") else . end)?'` prints only `1`.
    // Pre-#693 this codebase's binary printed `1` and `3` (the masked error
    // at the second element self-suppressed instead of stopping the
    // fan-out).
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"(.[] | if .==2 then error("boom") else . end)?"#],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_uncaught_type_error_exits_5() -> Result<()> {
    // Internal errors carry no payload, so no `(not a string)` marker. The
    // wording still differs from jq's ("Cannot index number with string") --
    // message parity is a separate concern from the exit code.
    let (stdout, stderr, code) = run_jq_full(&["-c", "1|.foo"], Some(r#"{"x":1}"#))?;
    assert_eq!(code, 5, "uncaught type error must exit 5: {stderr}");
    assert_eq!(stdout, "");
    // Line 0: the input has no trailing newline (#524).
    assert!(stderr.starts_with("jq: error (at <stdin>:0): "), "{stderr}");
    assert!(!stderr.contains("(not a string)"), "{stderr}");
    Ok(())
}

#[test]
fn test_uncaught_error_marks_non_string_payload() -> Result<()> {
    // jq flags a raised payload that is not a string. Needs the raw value at
    // the print site -- the rendered message alone cannot tell the two apart.
    // Line 0 throughout: the input has no trailing newline (#524).
    for (filter, want) in [
        (
            r#"error({"a":1})"#,
            r#"jq: error (at <stdin>:0) (not a string): {"a":1}"#,
        ),
        (
            "error(null)",
            "jq: error (at <stdin>:0) (not a string): null",
        ),
        ("error(42)", "jq: error (at <stdin>:0) (not a string): 42"),
        // A string payload is *not* flagged.
        (r#"error("boom")"#, "jq: error (at <stdin>:0): boom"),
    ] {
        let (_, stderr, code) = run_jq_full(&["-c", filter], Some(r#"{"x":1}"#))?;
        assert_eq!(code, 5, "{filter}");
        assert_eq!(stderr.trim_end(), want, "{filter}");
    }
    Ok(())
}

#[test]
fn test_uncaught_error_names_the_file_and_line() -> Result<()> {
    // jq's marker names the line the input value *ends* on, counted in the
    // whole file rather than within the value.
    let (_keep, path) = temp_json("{\"n\":1}\n{\"n\":2}\n")?;
    let (_, stderr, code) = run_jq_full(
        &["-c", r#"if .n==2 then error("bad") else . end"#, &path],
        None,
    )?;
    assert_eq!(code, 5);
    assert_eq!(stderr.trim_end(), format!("jq: error (at {path}:2): bad"));

    // A value spanning several lines is reported at its last line, not first.
    let (_keep, path) = temp_json("{\n  \"n\": 1\n}\n{\n  \"n\": 2\n}\n")?;
    let (_, stderr, code) = run_jq_full(
        &["-c", r#"if .n==2 then error("bad") else . end"#, &path],
        None,
    )?;
    assert_eq!(code, 5);
    assert_eq!(stderr.trim_end(), format!("jq: error (at {path}:6): bad"));
    Ok(())
}

#[test]
fn test_uncaught_error_line_number_without_trailing_newline() -> Result<()> {
    // #524: `line_at` reported one line too many whenever the erroring value
    // is the last one and the input lacks a trailing newline. Every
    // expectation here was captured against pinned jq 1.7.1-apple.

    // Multi-value input with no trailing newline after the last value: real
    // jq reports line 1 for *both* values, not line 2 for the second.
    let (_keep, path) = temp_json("1\n2")?;
    let (_, stderr, code) = run_jq_full(&["-c", r#"error("boom")"#, &path], None)?;
    assert_eq!(code, 5);
    assert_eq!(
        stderr.trim_end(),
        format!("jq: error (at {path}:1): boom\njq: error (at {path}:1): boom")
    );

    // A container value spanning multiple lines with no trailing newline:
    // real jq names its closing brace's line, not one past it.
    let (_keep, path) = temp_json("{\n\"a\":1\n}")?;
    let (_, stderr, code) = run_jq_full(&["-c", r#"error("boom")"#, &path], None)?;
    assert_eq!(code, 5);
    assert_eq!(stderr.trim_end(), format!("jq: error (at {path}:2): boom"));
    Ok(())
}

#[test]
fn test_uncaught_error_on_any_input_fails_the_run() -> Result<()> {
    // DELIBERATE DIVERGENCE FROM jq. jq's exit code reflects only the *last*
    // input, so an error on any earlier one exits 0 -- the exact
    // indistinguishability #355 exists to remove. We fail if any input raised.
    let (_keep, path) = temp_json("{\"n\":1}\n{\"n\":2}\n")?;
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"if .n==1 then error("bad") else . end"#, &path],
        None,
    )?;
    assert_eq!(code, 5, "real jq exits 0 here; we deliberately do not");
    assert_eq!(stderr.trim_end(), format!("jq: error (at {path}:1): bad"));
    // Evaluation still continues to the remaining inputs, as jq does.
    assert_eq!(stdout.trim(), r#"{"n":2}"#);
    Ok(())
}

#[test]
fn test_caught_error_still_exits_0() -> Result<()> {
    // Only *uncaught* errors fail. `try`/`catch` handles it, so the run
    // succeeded and nothing goes to stderr.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"try error("boom") catch "caught""#], Some("{}"))?;
    assert_eq!(code, 0, "a caught error is not a failure: {stderr}");
    assert_eq!(stdout.trim(), r#""caught""#);
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_null_input_error_reports_unknown_location() -> Result<()> {
    // Under -n there is no input to point at; jq prints `<unknown>`, with no
    // line number at all.
    let (_, stderr, code) = run_jq_full(&["-n", "-c", r#"error("boom")"#], None)?;
    assert_eq!(code, 5);
    assert_eq!(stderr.trim_end(), "jq: error (at <unknown>): boom");
    Ok(())
}

#[test]
fn test_error_exit_code_outranks_exit_status_flag() -> Result<()> {
    // #178 gave -e its own codes for a falsy *result*; #355's 5 means the
    // filter *failed*. The two must not be conflated, and the error wins.
    let (_, stderr, code) = run_jq_full(&["-e", "-c", r#"error("boom")"#], Some("{}"))?;
    assert_eq!(code, 5, "error outranks -e's 1/4: {stderr}");

    // -e's own semantics are untouched when nothing raised.
    let (_, _, code) = run_jq_full(&["-e", "-c", "false"], Some("{}"))?;
    assert_eq!(code, 1, "-e still reports a falsy last result as 1");
    let (_, _, code) = run_jq_full(&["-e", "-c", "empty"], Some("{}"))?;
    assert_eq!(code, 4, "-e still reports no output as 4");
    Ok(())
}

#[test]
fn test_uncaught_error_locations_across_input_modes() -> Result<()> {
    // The non-default input paths reach the evaluator through a different
    // route than plain JSON, and each has to keep its own line mapping.
    let (_keep, path) = temp_json("{\"n\":1}\n{\"n\":2}\n")?;

    // --slurp collapses every input into one array; jq names the line the
    // last of them ended on.
    let (_, stderr, code) = run_jq_full(&["-s", "-c", r#"error("x")"#, &path], None)?;
    assert_eq!(code, 5);
    assert_eq!(stderr.trim_end(), format!("jq: error (at {path}:2): x"));

    // --sort-keys routes through the materializing path, but per-value lines
    // must survive it.
    let (_, stderr, code) = run_jq_full(&["-S", "-c", r#"error("x")"#, &path], None)?;
    assert_eq!(code, 5);
    assert_eq!(
        stderr.trim_end(),
        format!("jq: error (at {path}:1): x\njq: error (at {path}:2): x")
    );

    // -R makes each line a string; the line number is that line.
    let (_keep, path) = temp_json("aaa\nbbb\nccc\n")?;
    let (_, stderr, code) = run_jq_full(&["-R", "-c", r#"error("x")"#, &path], None)?;
    assert_eq!(code, 5);
    assert_eq!(
        stderr.trim_end(),
        format!(
            "jq: error (at {path}:1): x\njq: error (at {path}:2): x\njq: error (at {path}:3): x"
        )
    );

    // -R -s makes the whole input one string, reported at its last content
    // line -- a trailing newline does not open a new one.
    let (_, stderr, code) = run_jq_full(&["-R", "-s", "-c", r#"error("x")"#, &path], None)?;
    assert_eq!(code, 5);
    assert_eq!(stderr.trim_end(), format!("jq: error (at {path}:3): x"));
    Ok(())
}

#[test]
fn test_uncaught_break_after_output_keeps_the_prefix() -> Result<()> {
    // #400/#494 for the `break` terminator: the outputs a stream produced
    // before an uncaught `break` still reach stdout, and the break still
    // drives the diagnostic and the exit code.
    //
    // The error terminator is pinned against real jq in
    // `tests/data/jq-golden/cases/*_error_after_output`. `break` cannot be:
    // jq rejects an unlabelled `break $out` at *compile* time ("$*label-out
    // is not defined", exit 3), so there is no oracle for the shape that
    // reaches this arm. These pin succinctly's own accept-and-report
    // behavior; the labelled (caught) forms, which jq does accept, are
    // covered by the `and_break_after_output`, `or_break_after_output` and
    // `label_break_after_comma` golden cases.

    // `run_jq_full` spawns the built binary rather than shelling out to
    // `cargo run`, which would build (and measure) a second, separate
    // binary — under `cargo llvm-cov` only the former is instrumented.

    // Lazy raw-bytes path (the default for JSON on stdin).
    let (stdout, stderr, code) = run_jq_full(&["1,2,break $out"], Some("null"))?;
    assert_eq!(stdout, "1\n2\n");
    assert!(
        stderr.contains("break $out not in label"),
        "expected the break diagnostic, got: {stderr}"
    );
    assert_eq!(code, 5);

    // A prefix built one pipe element at a time, rather than by a comma.
    let (stdout, stderr, code) = run_jq_full(
        &[".[] | if . == 3 then break $out else . end"],
        Some("[1,2,3,4]"),
    )?;
    assert_eq!(stdout, "1\n2\n");
    assert!(stderr.contains("break $out not in label"), "{stderr}");
    assert_eq!(code, 5);

    // `--null-input` takes the serde-parsed path instead, which has its own
    // copy of the result conversion.
    let (stdout, stderr, code) = run_jq_full(&["-n", "1,2,break $out"], None)?;
    assert_eq!(stdout, "1\n2\n");
    assert!(stderr.contains("break $out not in label"), "{stderr}");
    assert_eq!(code, 5);
    Ok(())
}

#[test]
fn regression_issue_575_break_in_loop_constructs_reaches_label() -> Result<()> {
    // A `break $label` raised from inside `while`/`foreach`/`repeat`'s
    // per-iteration expression used to degrade into a bogus
    // "break $out not in label" error (exit 5) instead of unwinding to the
    // enclosing `label` (real jq: clean output, exit 0) — the label never
    // got a chance to catch it. All three transcripts here are verified
    // against jq 1.7.1.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "label $out | while(true; if . >= 1 then break $out else .+1 end)",
        ],
        Some("1"),
    )?;
    assert_eq!(stdout, "1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, 0);

    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "label $out | repeat(if . >= 1 then break $out else .+1 end)",
        ],
        Some("1"),
    )?;
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert_eq!(code, 0);

    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "label $out | foreach (1,2,3) as $x (0; if $x == 2 then break $out else . + $x end)",
        ],
        Some("null"),
    )?;
    assert_eq!(stdout, "1\n");
    assert_eq!(stderr, "");
    assert_eq!(code, 0);
    Ok(())
}

/// A `NumberLiteral` that overflows to infinity (e.g. `1e400`) used to render
/// as garbage like `"NaNE+2147483647"` in every non-JSON text format instead
/// of `"inf"`/`"-inf"` (#561). `tostring` reaches the fix directly, but
/// `@uri`/`@html`/`@sh`/string interpolation/`@csv` are dispatched through
/// `eval_generic`'s cursor-reindexing bridge, which round-trips the value
/// through JSON text -- and used to substitute `"null"` for the overflowed
/// value before the fix ever saw it. This test exercises the real CLI (not
/// `eval.rs::eval()` directly) so it actually covers that bridge.
#[test]
fn test_number_literal_overflow_text_formats_via_cli() -> Result<()> {
    let (output, code) = run_jq_stdin("tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin("tostring", "-1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""-inf""#);

    let (output, code) = run_jq_stdin("@uri", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin("@html", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin("@sh", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin(r#""\(.)""#, "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin("@csv", "[1e400]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    Ok(())
}

/// The CLI's identity/raw-print path is a separate code path from the jq
/// evaluator (it prints source number bytes straight through
/// `format_number_jq_compat`), and had the same overflow-renders-as-garbage
/// bug (#561): unlike JSON output's established "NaN/Infinity -> null"
/// convention (`OwnedValue::to_json`), it printed
/// `"NaNE+2147483647"` for `1e400 | .` instead of `null`.
#[test]
fn test_number_literal_overflow_identity_prints_null_via_cli() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "1e400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    let (output, code) = run_jq_stdin(".", "-1e400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    Ok(())
}

/// `eval_owned_expr`/`eval_owned_input` (backing `reduce`/`foreach`/`as $x`
/// variable binding) and `with_entries`'s `owned_to_json_bytes` each have
/// their own serialize-and-reparse bridge, separate from `eval_generic`'s
/// (already covered by `test_number_literal_overflow_text_formats_via_cli`).
/// Before switching these to `to_json_for_reindex`, they too silently turned
/// an overflowed `NumberLiteral` into JSON `null`, so `. as $x | $x |
/// tostring` printed `"null"` instead of `"inf"` even though a direct
/// `tostring` was already fixed (#561).
#[test]
fn test_number_literal_overflow_owned_reindex_bridges_via_cli() -> Result<()> {
    let (output, code) = run_jq_stdin(". as $x | $x | tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin(". as $x | $x | tostring", "-1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""-inf""#);

    let (output, code) = run_jq_stdin("reduce (1) as $x (.; .) | tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin("foreach (1) as $x (.; .) | tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""inf""#);

    let (output, code) = run_jq_stdin(
        "with_entries(.value |= (. | tostring))",
        r#"{"a":1e400}"#,
        &["-c"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":"inf"}"#);

    Ok(())
}

/// `path`/`parent`/`parent(n)`/`key` used to silently answer `[]`/`{}`/`null`
/// (the root-level defaults) whenever they weren't the very first pipe stage:
/// the CLI's streaming evaluator (`eval_generic.rs`) bridged only the bare
/// trailing builtin to the full evaluator, discarding the pipe structure
/// `eval.rs`'s `needs_path_context` routing needs to see (#554). Uses
/// `run_jq_full` (the pre-built binary), not the `cargo run`-based
/// `run_jq_stdin`, so this is actually covered by `cargo llvm-cov`.
#[test]
fn test_path_context_builtins_across_pipe_stages_554() -> Result<()> {
    let (output, _, code) = run_jq_full(&["-c", ".a | path"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["a"]"#);

    let (output, _, code) = run_jq_full(&["-c", ".a | parent"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);

    let (output, _, code) = run_jq_full(&["-c", ".a | key"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""a""#);

    let (output, _, code) = run_jq_full(&["-c", ".a.b | parent"], Some(r#"{"a":{"b":1}}"#))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"b":1}"#);

    let (output, _, code) = run_jq_full(
        &["-c", ".a.b.c | parent(2)"],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"b":{"c":1}}"#);

    Ok(())
}

/// Regression tests (#715 follow-up) for the path-context evaluator's
/// fan-out loops (`Iterate`, `If`'s multi-valued `cond`, `Comma`, `Select`):
/// each independently collapsed an escaping `break` or `error` to a bare
/// synthetic error / `None`, discarding output already produced by earlier
/// iterations. Fixed by routing every loop through a shared
/// accumulate-or-stop helper that matches the plain evaluator's `eval_comma`
/// (#400/#494) semantics instead of re-deriving (and mis-deriving) the same
/// logic per arm.
#[test]
fn test_path_context_fanout_preserves_output_before_break_or_error_715() -> Result<()> {
    // `break` inside `if`/`then`/`else` whose `cond` needs path context
    // (`key`) used to wrongly report "break $out not in label" even though
    // the label genuinely enclosed it -- `Expr::Break` had no arm in the
    // path-context evaluator and fell into a fallback that unconditionally
    // converts `Control::Break` into a synthetic error.
    let (stdout, _stderr, code) = run_jq_full(
        &["label $out | .[] | if key == 1 then break $out else key end"],
        Some("[10,20,30]"),
    )?;
    assert_eq!(stdout, "0\n");
    assert_eq!(code, 0);

    // `Iterate` (`.[]`) used to discard output from earlier elements when a
    // later element's path-context-dependent branch errored.
    let (stdout, stderr, code) = run_jq_full(
        &[".[] | if key == 2 then error(\"boom\") else key end"],
        Some("[10,20,30]"),
    )?;
    assert_eq!(stdout, "0\n1\n");
    assert!(stderr.contains("boom"), "expected the error, got: {stderr}");
    assert_eq!(code, 5);

    // `Comma` used to discard an earlier branch's output on a later
    // branch's error (and also collapsed independent branch outputs into a
    // single array, unlike plain jq's `(a, b)`).
    let (stdout, stderr, code) = run_jq_full(&[".[0] | (key, error(\"boom\"))"], Some("[10,20]"))?;
    assert_eq!(stdout, "0\n");
    assert!(stderr.contains("boom"), "expected the error, got: {stderr}");
    assert_eq!(code, 5);

    // `(key, key)` now fans out to independent top-level outputs, matching
    // plain jq's comma semantics instead of collapsing per-element into a
    // `[key, key]` array.
    let (stdout, _stderr, code) = run_jq_full(&[".[] | (key, key)"], Some("[10,20]"))?;
    assert_eq!(stdout, "0\n0\n1\n1\n");
    assert_eq!(code, 0);

    // `Select`'s continuation used to collapse a `Partial`/`Break` result to
    // a bare `None`; it now delegates to the same shared helper.
    let (stdout, stderr, code) = run_jq_full(
        &[".[] | select(key == 0, error(\"boom\"))"],
        Some("[10,20]"),
    )?;
    assert_eq!(stdout, "10\n");
    assert!(stderr.contains("boom"), "expected the error, got: {stderr}");
    assert_eq!(code, 5);

    Ok(())
}

/// `keys_unsorted` stays lazy through `length`/`.[]`/`.[n]`/`first`/`last`
/// (#140), backed by a new `JqValue::LazyKeysArray` output writer in
/// `print_json`. Uses `run_jq_full` (the pre-built binary) to exercise that
/// writer directly, both compact and pretty, rather than just the evaluator
/// (already covered by `eval_generic.rs`'s unit tests).
#[test]
fn test_keys_unsorted_lazy_output_140() -> Result<()> {
    let input = r#"{"b":1,"a":2,"c":3}"#;

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["b","a","c"]"#);

    let (output, _, code) = run_jq_full(&["--indent", "2", "keys_unsorted"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\n  \"b\",\n  \"a\",\n  \"c\"\n]");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | length"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "3");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | (length)"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(
        output.trim(),
        "3",
        "parenthesized length must still hit the fast path"
    );

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "\"b\"\n\"a\"\n\"c\"");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[0]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""b""#);

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[10]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null", "out of bounds is null, not an error");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | first"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""b""#);

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | last"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""c""#);

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | first"], Some("{}"))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    // `map`/`select` have no native lazy path and must still materialize
    // correctly through the fallback.
    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | map(ascii_upcase)"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["B","A","C"]"#);

    // Escaped/non-ASCII keys go through the zero-copy raw-bytes path only
    // when safe; otherwise decode-and-reescape, same as a regular object's
    // keys.
    let escaped_input = "{\"a\\\"b\":1,\"caf\u{e9}\":2}";
    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted"], Some(escaped_input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\"a\\\"b\",\"caf\u{e9}\"]");

    let (output, _, code) = run_jq_full(
        &["-c", "--ascii-output", "keys_unsorted"],
        Some(escaped_input),
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\"a\\\"b\",\"caf\\u00e9\"]");

    Ok(())
}
/// Sorted `keys | length` mirror of `test_keys_unsorted_lazy_output_140`
/// above (#683): `length` now answers from the field iterator without
/// decoding or sorting, while bare `keys`/`.[]`/`.[n]`/`first`/`last` stay
/// byte-identical to today's eager-sorted output (`JqValue::LazyKeysArray`
/// is document-order-only, so a sorted result never reaches it -- see the
/// `generic_result_to_jq_values` fix in `jq_runner.rs`). Uses `run_jq_full`
/// (the pre-built binary), not `cargo run` (invisible to coverage).
#[test]
fn test_keys_lazy_length_output_683() -> Result<()> {
    let input = r#"{"b":1,"a":2,"c":3}"#;

    let (output, _, code) = run_jq_full(&["-c", "keys"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(
        output.trim(),
        r#"["a","b","c"]"#,
        "no regression: still sorted"
    );

    let (output, _, code) = run_jq_full(&["--indent", "2", "keys"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\n  \"a\",\n  \"b\",\n  \"c\"\n]");

    let (output, _, code) = run_jq_full(&["-c", "keys | length"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "3");

    let (output, _, code) = run_jq_full(&["-c", "keys | (length)"], Some(input))?;
    assert_eq!(
        output.trim(),
        "3",
        "parenthesized length must still hit the fast path"
    );
    assert_eq!(code, 0);

    let (output, _, code) = run_jq_full(&["-c", "keys | .[]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(
        output.trim(),
        "\"a\"\n\"b\"\n\"c\"",
        "sorted order, not document order"
    );

    let (output, _, code) = run_jq_full(&["-c", "keys | .[0]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""a""#);

    let (output, _, code) = run_jq_full(&["-c", "keys | .[-1]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""c""#);

    let (output, _, code) = run_jq_full(&["-c", "keys | .[10]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null", "out of bounds is null, not an error");

    let (output, _, code) = run_jq_full(&["-c", "keys | first"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""a""#, "sorted first, not document first");

    let (output, _, code) = run_jq_full(&["-c", "keys | last"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""c""#);

    let (output, _, code) = run_jq_full(&["-c", "keys | first"], Some("{}"))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    // `map`/`select` have no native lazy path and must still materialize
    // correctly, sorted, through the fallback.
    let (output, _, code) = run_jq_full(&["-c", "keys | map(ascii_upcase)"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["A","B","C"]"#);

    // Escaped/non-ASCII keys still decode-and-reescape correctly when
    // sorted (this pair happens to sort in the same order as document
    // order -- `a"b` < `café` either way -- so it exercises the escape path
    // without duplicating the sort-order regression guard, which lives in
    // `eval_generic.rs`'s `test_generic_keys_sorted_still_fully_sorted`).
    let escaped_input = "{\"a\\\"b\":1,\"caf\u{e9}\":2}";
    let (output, _, code) = run_jq_full(&["-c", "keys"], Some(escaped_input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\"a\\\"b\",\"caf\u{e9}\"]");

    let (output, _, code) = run_jq_full(&["-c", "--ascii-output", "keys"], Some(escaped_input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\"a\\\"b\",\"caf\\u00e9\"]");

    Ok(())
}

/// Array `keys`/`keys_unsorted` stays lazy through `length`/`.[]`/`.[n]`/
/// `first`/`last` too (#684), backed by `JqValue::LazyIndexRange` in
/// `print_json` -- the array counterpart of `test_keys_unsorted_lazy_output_140`
/// above. Uses `run_jq_full` to exercise the CLI output writer directly.
#[test]
fn test_array_keys_unsorted_lazy_output_684() -> Result<()> {
    let input = r#"["x","y","z"]"#;

    // `keys` and `keys_unsorted` are identical on an array (the index range
    // is already sorted).
    let (output, _, code) = run_jq_full(&["-c", "keys"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1,2]");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1,2]");

    let (output, _, code) = run_jq_full(&["--indent", "2", "keys_unsorted"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\n  0,\n  1,\n  2\n]");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | length"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "3");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | (length)"], Some(input))?;
    assert_eq!(
        output.trim(),
        "3",
        "parenthesized length must still hit the fast path"
    );
    assert_eq!(code, 0);

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0\n1\n2");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[0]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[-1]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "2");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[10]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null", "out of bounds is null, not an error");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | first"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | last"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "2");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | first"], Some("[]"))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted"], Some("[]"))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[]");

    // `map`/`select` have no native lazy path and must still materialize
    // correctly through the fallback.
    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | map(. * 10)"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,10,20]");

    // Far out-of-bounds negative index (normalizes below zero, not just
    // negative-in-range like `.[-1]` above) is still `null`, not an error.
    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted | .[-100]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "null");

    Ok(())
}

/// `GenericResult::LazyIndexRange` fallback paths #684 that the `Pipe`/M2
/// fast paths above never reach: `select`'s truthiness check, the
/// `.[] | keys_unsorted`-shaped per-element degrade from `ManyCursor`,
/// `==` comparison against a lazy operand, and `first(...)`/`last(...)`
/// function-call syntax (as opposed to the bare `first`/`last` builtins
/// already covered above).
#[test]
fn test_array_keys_unsorted_lazy_fallback_paths_684() -> Result<()> {
    let input = r#"["x","y","z"]"#;

    // `select`'s condition only needs truthiness -- an array is always
    // truthy in jq -- so this never materializes the index range.
    let (output, _, code) = run_jq_full(&["-c", "select(keys_unsorted)"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["x","y","z"]"#);

    // Each element of the outer iteration degrades from `ManyCursor` to a
    // materialized `LazyIndexRange` per element, since `keys_unsorted` on an
    // array element isn't itself a single cursor.
    let (output, _, code) = run_jq_full(
        &["-c", ".[] | keys_unsorted"],
        Some(r#"[["a","b"],["c","d","e"]]"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1]\n[0,1,2]");

    // Both operands of `==` are lazy index ranges here.
    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted == keys_unsorted"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "true");

    let (output, _, code) = run_jq_full(&["-c", "keys_unsorted == [0,1]"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "false");

    // `first(...)`/`last(...)` function-call syntax, distinct from the bare
    // `first`/`last` builtins the `Pipe` dispatch fast-paths above. Unlike
    // `keys_unsorted | first` (which iterates the array), `keys_unsorted`
    // itself is a generator with exactly one output -- the whole array --
    // so `first(keys_unsorted)`/`last(keys_unsorted)` both forward that one
    // `LazyIndexRange` output unchanged.
    let (output, _, code) = run_jq_full(&["-c", "first(keys_unsorted)"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1,2]");

    let (output, _, code) = run_jq_full(&["-c", "last(keys_unsorted)"], Some(input))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1,2]");

    Ok(())
}

/// The "original"/serde_json path (`evaluate_input` in `jq_runner.rs`,
/// forced whenever `--sort-keys` disables the lazy-bytes path) has its own
/// `GenericResult::LazyIndexRange` arm distinct from the lazy path's --
/// exercise it directly rather than relying on incidental coverage from
/// another flag combination.
#[test]
fn test_array_keys_unsorted_sort_keys_path_684() -> Result<()> {
    let (output, _, code) = run_jq_full(
        &["--sort-keys", "-c", "keys_unsorted"],
        Some(r#"["x","y","z"]"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1,2]");

    Ok(())
}

/// The top-level materializing boundary on the *lazy-bytes* path
/// (`generic_result_to_jq_values` in `jq_runner.rs`) has its own
/// `GenericResult::LazySeq` arm, reached only when the whole parsed query is
/// itself `map(f)`/`keys_unsorted | map(f)` -- with no further pipe stage to
/// resolve it into a narrower shape first (#725). Exercise all three
/// outcomes (success, error, break) directly against the real binary rather
/// than relying on incidental coverage from another test's query shape.
#[test]
fn test_top_level_map_lazy_seq_materializes_at_cli_boundary_725() -> Result<()> {
    let (output, _, code) = run_jq_full(&["-c", "map(. + 1)"], Some("[1,2,3]"))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[2,3,4]");

    let (output, stderr, code) = run_jq_full(&["-c", "map(. + 1)"], Some(r#"[1,2,"x"]"#))?;
    assert_eq!(code, 5);
    assert_eq!(output, "");
    assert!(stderr.contains("cannot be added"), "{stderr}");

    let (output, stderr, code) = run_jq_full(&["-c", "map(break $out)"], Some("[1,2,3]"))?;
    assert_eq!(code, 5);
    assert_eq!(output, "");
    assert!(stderr.contains("break $out not in label"), "{stderr}");

    Ok(())
}

/// Same `GenericResult::LazySeq` fast path, but reached through the
/// "original"/serde_json evaluator (`evaluate_input`'s
/// `query_result_to_owned_values`) instead -- forced by `--sort-keys`,
/// mirroring `test_array_keys_unsorted_sort_keys_path_684` above. This is a
/// distinct match arm from the lazy-bytes path's, not just a different flag
/// combination reaching the same code.
#[test]
fn test_top_level_map_lazy_seq_sort_keys_path_725() -> Result<()> {
    let (output, _, code) = run_jq_full(&["--sort-keys", "-c", "map(. + 1)"], Some("[1,2,3]"))?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[2,3,4]");

    let (output, stderr, code) =
        run_jq_full(&["--sort-keys", "-c", "map(. + 1)"], Some(r#"[1,2,"x"]"#))?;
    assert_eq!(code, 5);
    assert_eq!(output, "");
    assert!(stderr.contains("cannot be added"), "{stderr}");

    let (output, stderr, code) =
        run_jq_full(&["--sort-keys", "-c", "map(break $out)"], Some("[1,2,3]"))?;
    assert_eq!(code, 5);
    assert_eq!(output, "");
    assert!(stderr.contains("break $out not in label"), "{stderr}");

    Ok(())
}

#[test]
fn test_path_halt_after_partial_resolution_converts_via_evalescape_into_control() -> Result<()> {
    // `error.rs`'s `impl From<EvalEscape> for Control` -- specifically its
    // `EvalEscape::Halt(code) => Self::Halt(code)` arm (line 110) -- is
    // reached from exactly one call site: `builtin_path`'s
    // `partial(paths, e.into())`, where `e: EvalEscape` comes from
    // `resolve_dynamic_indexes`/`resolve_node`'s `Expr::Comma` arm.
    // `path(.a, halt_error(3))` resolves `.a` into a path successfully
    // first (populating `paths`), then hits the halt while resolving the
    // second comma branch -- `paths` is non-empty at that point, so
    // `builtin_path` takes the `partial(paths, e.into())` branch, not its
    // `paths.is_empty()` sibling (which instead goes through `eval.rs`'s
    // own separate `impl From<EvalEscape> for QueryResult` and never
    // touches this line). Verified against jq 1.7.1: `jq -c 'path(.a,
    // halt_error(3))'` on `{"a":1}` prints `["a"]` to stdout (the path
    // resolved before the halt), `{"a":1}` to stderr (halt_error's
    // non-string value -- `.` at the point of the halt is still the whole
    // input, since `,` evaluates each branch against the same value), and
    // exits 3.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path(.a, halt_error(3))"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "{\"a\":1}\n");
    Ok(())
}

#[test]
fn test_input_dsv_streaming_halt_aborts_remaining_rows() -> Result<()> {
    // The `--input-dsv` streaming loop's `if let Some(code) = sink.halted()
    // { out.flush()?; return Ok(code); }` check (jq_runner.rs, added by
    // #791) used to be entirely absent: a halt raised while evaluating one
    // row was recorded in `sink` but nothing inspected it until the whole
    // rows/files loop finished naturally, letting every later row's filter
    // keep running (and its output print) after the halt fired.
    // `--input-dsv` is a succinctly-only extension (real jq has no DSV
    // input mode), so this is checked against succinctly's own contract,
    // not jq: row 1 prints normally, row 2's filter halts, and row 3 --
    // which would print `["c","d"]` if this check were missing -- must
    // never be reached.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "--input-dsv",
            ",",
            "-c",
            r#"if .[0] == "HALT" then halt_error(9) else . end"#,
        ],
        Some("a,b\nHALT,x\nc,d\n"),
    )?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\",\"b\"]\n");
    assert_eq!(stderr, "[\"HALT\",\"x\"]\n");
    Ok(())
}

#[test]
fn test_lazyseq_map_halt_propagates_through_evaluate_input_sort_keys_path() -> Result<()> {
    // `evaluate_input`'s `GenericResult::LazySeq(seq) => match
    // seq.materialize_atomic() { ... Err(jq::Control::Halt(code)) =>
    // sink.request_halt(code) ... }` arm (jq_runner.rs) is the non-lazy CLI
    // path used whenever `-S`/`-s`/`-R`/`--color-output`/`--ascii-output`/
    // `--input-dsv` is set -- a separate copy of this match from the
    // default lazy-bytes path's own (see the next test). `keys_unsorted |
    // map(...)` on an object builds a `LazySeq` (#724/#725's composability
    // engine: `LazyKeys | map(f)` stays lazy instead of materializing
    // eagerly), and `-S` (sort_keys) forces `can_use_lazy_path` false,
    // routing evaluation through `evaluate_input` instead of
    // `evaluate_bytes_lazy`. Verified against jq 1.7.1: `jq -S
    // 'keys_unsorted | map(if . == "a" then halt_error(3) else . end)'` on
    // `{"a":1,"b":2}` exits 3 with no stdout and stderr `a` (the halted
    // value, a string, printed raw).
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-S",
            r#"keys_unsorted | map(if . == "a" then halt_error(3) else . end)"#,
        ],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "a");
    Ok(())
}

#[test]
fn test_lazyseq_map_halt_propagates_through_default_lazy_path() -> Result<()> {
    // `generic_result_to_jq_values`'s `GenericResult::LazySeq(seq) => match
    // seq.materialize_atomic() { ... Err(jq::Control::Halt(code)) =>
    // sink.request_halt(code) ... }` arm (jq_runner.rs) is reached from the
    // *default* lazy-bytes CLI path (`evaluate_bytes_lazy`, used for plain
    // stdin/file input with none of -S/-s/-R/--color-output/--ascii-output/
    // --input-dsv set) -- a distinct site from `evaluate_input`'s own copy
    // of this match (tested above), since the two functions build the
    // `JqValue`/`OwnedValue` output representations independently.
    // Verified against jq 1.7.1: `jq 'keys_unsorted | map(if . == "a" then
    // halt_error(3) else . end)'` on `{"a":1,"b":2}` exits 3 with no
    // stdout and stderr `a`.
    let (stdout, stderr, code) = run_jq_full(
        &[r#"keys_unsorted | map(if . == "a" then halt_error(3) else . end)"#],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "a");
    Ok(())
}

#[test]
fn test_array_construction_drops_partial_prefix_on_halt() -> Result<()> {
    // `eval_array_construction`'s `Partial(_, Control::Halt(code))` arm
    // (#791): array construction is atomic in jq, so a halt reached partway
    // through building `[1, halt_error(3)]` must drop the in-progress array
    // entirely and propagate the bare halt, not surface a truncated array --
    // the same atomicity `Error`/`Break` already get one arm above this one.
    // Verified against jq 1.7.1: `jq -n '[1, halt_error(3)]'` exits 3 with no
    // output on either stream (`.` at the point `halt_error` runs is still
    // `null`, which prints nothing).
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1, halt_error(3)]"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_object_construction_keeps_partial_prefix_on_halt() -> Result<()> {
    // `eval_object_construction`'s `Err(ObjectEscape::Halt(code))` arm when
    // `objects` is already non-empty (#791): earlier key/value combinations
    // already pushed to `objects` before a later combination halts must
    // survive as `QueryResult::Partial`'s prefix, not vanish -- the same
    // "outputs already produced don't vanish" contract #400/#494 gave
    // `Error`/`Break` on the two arms right above this one. Verified against
    // jq 1.7.1: `jq -c -n '{a: (1,2), b: (3, halt_error(9))}'` prints
    // `{"a":1,"b":3}` -- the a=1,b=3 combination, completed before a=1's
    // second b-value halts -- then exits 9; the a=2 branch is never reached.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "-c", "{a: (1,2), b: (3, halt_error(9))}"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\":1,\"b\":3}\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_reduce_update_halt_produces_no_output() -> Result<()> {
    // `eval_owned_expr_fork`'s `Halt` arm (#791), reached through `reduce`'s
    // UPDATE expression: a bare halt on a fold step has no prior UPDATE
    // output on *that* step to fall back to, unlike a `Partial`'s trailing
    // control (the arm right below this one) -- so `eval_reduce`'s `aborted`
    // stays set and `outputs` (the accumulator across earlier fold steps)
    // is returned as-is via `finish_fork`, never gaining this step's value.
    // Verified against jq 1.7.1: `jq -n 'reduce (1) as $x (0; halt_error(3))'`
    // prints nothing to stdout, prints `0` (the accumulator, `.` at the
    // point `halt_error` runs) to stderr, and exits 3.
    let (stdout, stderr, code) = run_jq_full(&["-n", "reduce (1) as $x (0; halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "0\n");
    Ok(())
}

#[test]
fn test_try_catch_prepend_propagates_halt_from_handler() -> Result<()> {
    // `prepend`'s `Halt` arm (#791): `eval_try`'s `Partial(prefix,
    // Control::Error)` arm splices the catch handler's own result in after
    // the body's prefix via `prepend(prefix, handled)`. If the handler
    // itself halts, `prepend` must keep the body's earlier outputs *and*
    // still propagate the halt via `partial(prefix, Control::Halt(code))`,
    // not just return the handler's bare `Halt` and drop the prefix.
    // Verified against jq 1.7.1: `jq -n 'try (1, error("x")) catch
    // halt_error(9)'` prints `1` to stdout, `x` (the caught error's payload,
    // which `.` is bound to inside the catch handler) to stderr with no
    // trailing newline, and exits 9.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"try (1, error("x")) catch halt_error(9)"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    assert_eq!(stderr, "x");
    Ok(())
}

#[test]
fn test_result_to_owned_many_empty_arm_via_ltrimstr_argument() -> Result<()> {
    // `result_to_owned`'s `Many(vs)` arm when `vs` is empty. `(empty,empty)`
    // is two `None`-valued comma operands, so `eval_comma` never touches its
    // `owned` promotion path and falls out through its own `None =>
    // QueryResult::Many(borrowed)` arm with `borrowed` still `[]` --
    // `ltrimstr`'s argument slot then feeds that `Many(vec![])` into
    // `result_to_owned`, hitting the "empty result" error arm. Not
    // halt-specific (this arm predates #791; only its `.into()` wrapper is
    // new) and a pre-existing, unrelated divergence from real jq worth
    // flagging: jq treats `f(g)` as backtracking over every output of `g`,
    // so a zero-output argument means zero outputs overall -- `jq -n '"abc"
    // | ltrimstr((empty,empty))'` exits 0 with no output -- while
    // succinctly's `ltrimstr` resolves its argument to a single value via
    // `result_to_owned`, which treats "the argument stream produced
    // nothing" as an error instead.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | ltrimstr((empty,empty))"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "jq: error (at <unknown>): empty result\n");
    Ok(())
}

#[test]
fn test_result_to_owned_manyowned_empty_arm_via_ltrimstr_argument() -> Result<()> {
    // `result_to_owned`'s `ManyOwned(vs)` arm when `vs` is empty -- the
    // `Owned`-target sibling of the `Many`-empty case above, reached through
    // a different producer: `eval_index_expr`'s `Targets::Owned` branch ends
    // its match on `out.len()` with only `1 => Owned(...)`, so the `_ =>
    // ManyOwned(out)` wildcard also covers `out.len() == 0`. `(2+3)` is
    // computed (arithmetic is always `Owned`/`ManyOwned`, never
    // document-borrowed, so its target is `Targets::Owned`), and both
    // `"x"`/`"y"` keys against the number `5` -- with the trailing `?`
    // making `optional` true -- each resolve via `index_one_owned`'s `_ if
    // optional => Ok(None)` refusal-suppression arm, leaving `out` empty:
    // `QueryResult::ManyOwned(vec![])`. Same pre-existing, halt-unrelated
    // divergence from real jq as the `Many`-empty case above: `jq -n '"abc"
    // | ltrimstr((2+3) | .[("x","y")]?)'` exits 0 with no output, while
    // succinctly's `result_to_owned` reports it as an error.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#""abc" | ltrimstr((2+3) | .[("x","y")]?)"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "jq: error (at <unknown>): empty result\n");
    Ok(())
}

#[test]
fn test_result_to_owned_partial_halt_outranks_its_prefix() -> Result<()> {
    // `result_to_owned`'s `Partial(_, Control::Halt(code))` arm (#791): a
    // single-value builtin argument that produced an output and *then*
    // halted must still halt, not quietly compute with the value already
    // produced. This dedicated arm is checked *before* the generic
    // `Partial(vs, _control) => Ok(vs.into_iter().next().unwrap())`
    // fallback right below it, which would otherwise silently take `1` and
    // let `ltrimstr` continue. Note `ltrimstr`'s argument is a single-value
    // site here, not a backtracking generator: real jq's own `ltrimstr((1,
    // halt_error(6)))` actually calls `ltrimstr` once per argument output,
    // so `jq -n '"abc" | ltrimstr((1, halt_error(6)))'` prints `abc` (from
    // the `1` branch, where `ltrimstr` is a no-op on a non-string argument)
    // *before* the second branch halts -- succinctly's `ltrimstr` instead
    // resolves its argument to one value via `result_to_owned`, so it never
    // gets that first output at all; this test pins succinctly's own
    // contract, which #791 documents explicitly at this arm.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#""abc" | ltrimstr((1, halt_error(6)))"#], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "abc");
    Ok(())
}

#[test]
fn test_result_to_owned_none_error_break_arms_via_ltrimstr_argument() -> Result<()> {
    // `result_to_owned`'s `None`/`Error`/`Break` arms -- all three now
    // wrapped in `.into()` to fit the `Result<OwnedValue, EvalEscape>`
    // return type this PR introduced (previously `Result<OwnedValue,
    // EvalError>`), exercised together since they are adjacent,
    // mechanically-identical conversions and none of them are
    // halt-specific. Uses `ltrimstr`'s argument slot as the call site, same
    // as the `Many`/`ManyOwned`-empty and `Partial`-halt tests above.
    // `None`/`Break` here are a pre-existing, halt-unrelated divergence from
    // real jq worth flagging: jq's `empty` argument yields zero output (not
    // an error), and a `break` whose label encloses the whole expression is
    // caught there, not reported as "not in label" -- both verified: `jq -n
    // '"abc" | ltrimstr(empty)'` and `jq -n 'label $out | ("abc" |
    // ltrimstr(break $out))'` both exit 0 with no output. succinctly's
    // `ltrimstr` resolves its argument through `result_to_owned`, which
    // turns both into a generic catchable error instead. The `Error` arm,
    // by contrast, matches real jq exactly.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | ltrimstr(empty)"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "jq: error (at <unknown>): no value\n");

    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | ltrimstr(error("boom"))"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "jq: error (at <unknown>): boom\n");

    let (stdout, stderr, code) = run_jq_full(
        &["-n", r#"label $out | ("abc" | ltrimstr(break $out))"#],
        None,
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <unknown>): break $out not in label\n"
    );

    Ok(())
}

#[test]
fn test_boolean_and_propagates_halt_from_right_operand() -> Result<()> {
    // `push_truthiness`'s `Halt` arm (#791), reached through `eval_boolean`'s
    // right-operand fork -- `and`/`or` are generators over both operands
    // here, not scalar short-circuit operators, so the right operand's
    // stream is pushed through `push_truthiness` once per non-short-circuiting
    // left output. A halt while evaluating the right operand must escape
    // immediately rather than contribute a truthiness bit. Verified against
    // jq 1.7.1: `jq -n 'true and halt_error(4)'` exits 4 with no output on
    // either stream (`.` at the point `halt_error` runs is still `null`,
    // which prints nothing).
    let (stdout, stderr, code) = run_jq_full(&["-n", "true and halt_error(4)"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_arithmetic_propagates_halt_from_right_operand() -> Result<()> {
    // `push_owned_values`'s `Halt` arm (#791), reached through
    // `binary_fanout_core`'s right-operand fork -- shared by every
    // arithmetic and comparison operator (#768). A halt while evaluating
    // the right operand must escape immediately, the `OwnedValue`-collecting
    // analog of `push_truthiness`'s arm `and`/`or` use above. Verified
    // against jq 1.7.1: `jq -n '1 + halt_error(6)'` exits 6 with no output
    // on either stream.
    let (stdout, stderr, code) = run_jq_full(&["-n", "1 + halt_error(6)"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_isvalid_suppresses_genuine_error_in_error_message_expression() -> Result<()> {
    // `eval_error`'s message-expression arm, `Err(EvalEscape::Error(_)) if
    // optional => return QueryResult::None` (#791): this PR narrowed the
    // match from `Err(_) if optional` to `Err(EvalEscape::Error(_)) if
    // optional` specifically, so a halt in the message expression escapes
    // instead of being swallowed (covered by
    // `test_isvalid_propagates_halt_from_error_message_expression` above).
    // This test pins the other half: the narrowing must not have also
    // stopped swallowing a *genuine* (non-halt) error in the message
    // expression, which `isvalid`'s forced `optional=true` still has to
    // suppress into `false`. `isvalid` is a succinctly extension (real jq
    // has no such builtin -- `jq -n 'isvalid(error("boom"))'` reports
    // `isvalid/1 is not defined`), so this is checked against succinctly's
    // own documented contract instead of jq parity.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#"isvalid(error(error("boom")))"#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "false\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_map_propagates_bare_halt_from_element() -> Result<()> {
    // `map_over`'s bare `Halt` arm (#791): `map(f)` is `[.[] | f]` --
    // array-construction atomic, so a halt applying `f` to any element
    // drops the whole in-progress result array, mirroring
    // `eval_array_construction`'s own bare-`Halt` arm. Verified against jq
    // 1.7.1: `jq -n '[1] | map(halt_error(4))'` exits 4 with no stdout
    // (stderr `1`, the element `f` ran against).
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1] | map(halt_error(4))"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_map_propagates_partial_halt_from_element() -> Result<()> {
    // `map_over`'s `Partial(_, Control::Halt(code))` arm, one arm below the
    // bare-`Halt` one above: `f` producing an output and *then* halting on
    // the same element must still drop the whole result array, matching
    // the `Error`/`Break` `Partial` arms right beside it. Verified against
    // jq 1.7.1: `jq -n '[1] | map(2, halt_error(4))'` exits 4 with no
    // output (the `2` never surfaces -- array construction is atomic, same
    // as `test_array_construction_drops_partial_prefix_on_halt` above).
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1] | map(2, halt_error(4))"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_map_values_object_propagates_bare_halt_from_field() -> Result<()> {
    // `builtin_map_values`'s object-branch bare `Halt` arm (#791): object
    // construction is atomic (see `eval_object_construction`), so a halt
    // applying `f` to any field's value drops the whole in-progress result
    // object. Verified against jq 1.7.1: `jq -n '{"a":1} |
    // map_values(halt_error(5))'` exits 5 with no stdout (stderr `1`, the
    // field value `f` ran against).
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"{"a":1} | map_values(halt_error(5))"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_map_values_object_propagates_partial_halt_from_field() -> Result<()> {
    // `builtin_map_values`'s object-branch `Partial(_, Control::Halt(code))`
    // arm: `f` producing an output and then halting on the same field must
    // still drop the whole result object. Real jq's `map_values` is
    // defined as `.[] |= f` (`_modify`), which only ever observes the
    // *first* output of `f` and never reaches a second one -- `jq -n
    // '{"a":1} | map_values(2, halt_error(5))'` prints `{"a":2}` and exits
    // 0, never reaching `halt_error` at all -- while succinctly's
    // `map_values` instead evaluates `f` eagerly via `eval_single` and
    // walks every `QueryResult` variant it can return, so it does reach the
    // halt. This test pins succinctly's own contract for this pre-existing
    // divergence, not jq parity.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"{"a":1} | map_values(2, halt_error(5))"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_map_values_array_propagates_bare_halt_from_element() -> Result<()> {
    // `builtin_map_values`'s array-branch bare `Halt` arm (#791): the array
    // sibling of the object-branch test above. Verified against jq 1.7.1:
    // `jq -n '[1] | map_values(halt_error(6))'` exits 6 with no stdout
    // (stderr `1`).
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1] | map_values(halt_error(6))"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_map_values_array_propagates_partial_halt_from_element() -> Result<()> {
    // `builtin_map_values`'s array-branch `Partial(_, Control::Halt(code))`
    // arm. Same pre-existing multi-output-`f` divergence from real jq as
    // the object-branch `Partial` test above: real jq's `.[] |= f` only
    // observes `f`'s first output, so `jq -n '[1] | map_values(2,
    // halt_error(6))'` prints `[2]` and exits 0 without ever reaching
    // `halt_error` -- this test pins succinctly's own contract, which does
    // reach it.
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1] | map_values(2, halt_error(6))"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_add_reports_type_error_via_into_conversion() -> Result<()> {
    // `builtin_add`'s `Err(e) => e.into()` arm -- a pure `.into()` wrapper
    // this PR's `Result<_, EvalError> -> QueryResult` conversions added
    // (not halt-specific: `arith_add` only ever returns a plain
    // `EvalError`, never an `EvalEscape`, so this always resolves to
    // `QueryResult::Error`), exercised via a genuine type error since
    // that's the only way to reach it. Verified against jq 1.7.1: `jq -n
    // '[1, "a"] | add'` reports `number (1) and string ("a") cannot be
    // added` and exits 5.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#"[1, "a"] | add"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <unknown>): number (1) and string (\"a\") cannot be added\n"
    );
    Ok(())
}

#[test]
fn test_any_propagates_halt_from_condition() -> Result<()> {
    // `control_to_result`'s `Halt` arm (#791), reached via `any(cond)`:
    // `any_all_probe_element` forks `cond` through `eval_owned_expr_fork`
    // and, when it halts with no truthy output for this element, returns
    // `Err(Control::Halt(code))`; `any_all_f`'s `Err(control) =>
    // control_to_result(control)` converts that back into a `QueryResult`
    // through this arm. Verified against jq 1.7.1: `jq -n '[1,2] |
    // any(halt_error(4))'` exits 4 with no stdout (stderr `1`, the first
    // element `cond` ran against -- the second element is never probed).
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2] | any(halt_error(4))"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_min_by_propagates_halt_from_key_expr() -> Result<()> {
    // `builtin_min_by`'s `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm (#791): each item's key is computed via
    // `eval_array_construction`, and a halt while computing any item's key
    // must abort the whole `min_by` rather than being folded into the
    // `_ => unreachable!(...)` wildcard right below (which `Owned`
    // is not, being explicitly listed as `eval_array_construction`'s only
    // other possible non-escaping return). Verified against jq 1.7.1: `jq
    // -n '[1,2] | min_by(halt_error(4))'` exits 4 with no stdout (stderr
    // `1`, the first item the key filter ran against).
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2] | min_by(halt_error(4))"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_max_by_propagates_halt_from_key_expr() -> Result<()> {
    // `builtin_max_by`'s `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm (#791) -- the `max_by` sibling of the
    // `min_by` test above, identical shape. Verified against jq 1.7.1: `jq
    // -n '[1,2] | max_by(halt_error(4))'` exits 4 with no stdout (stderr
    // `1`, the first item the key filter ran against).
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2] | max_by(halt_error(4))"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "1\n");
    Ok(())
}

#[test]
fn test_ltrimstr_propagates_halt_in_prefix_argument() -> Result<()> {
    // `builtin_ltrimstr`'s `Err(e) => return e.into()` arm (#791): the
    // prefix argument's `result_to_owned` failure -- here a bare halt --
    // must escape via `.into()` (which preserves `EvalEscape::Halt` through
    // the `From<EvalEscape> for QueryResult` impl) rather than being folded
    // into a plain `QueryResult::Error`. Verified against jq 1.7.1: `jq -n
    // '"abc" | ltrimstr(halt_error(7))'` exits 7 with no stdout (stderr
    // `abc`, the string `.` halt_error printed raw with no trailing
    // newline, per its string-payload contract).
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | ltrimstr(halt_error(7))"#], None)?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "abc");
    Ok(())
}

#[test]
fn test_rtrimstr_propagates_halt_in_suffix_argument() -> Result<()> {
    // `builtin_rtrimstr`'s `Err(e) => return e.into()` arm (#791) -- the
    // `rtrimstr` sibling of the `ltrimstr` test above, same shape. Verified
    // against jq 1.7.1: `jq -n '"abc" | rtrimstr(halt_error(8))'` exits 8
    // with no stdout (stderr `abc`, no trailing newline).
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | rtrimstr(halt_error(8))"#], None)?;
    assert_eq!(code, 8, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "abc");
    Ok(())
}

#[test]
fn test_startswith_propagates_halt_in_prefix_argument() -> Result<()> {
    // `builtin_startswith`'s `Err(e) => return e.into()` arm (#791) --
    // same shape as `ltrimstr`/`rtrimstr` above, one call site further down
    // the file. Verified against jq 1.7.1: `jq -n '"abc" |
    // startswith(halt_error(9))'` exits 9 with no stdout (stderr `abc`, no
    // trailing newline).
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#""abc" | startswith(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "abc");
    Ok(())
}

#[test]
fn test_endswith_propagates_halt_in_suffix_argument() -> Result<()> {
    // `builtin_endswith`'s suffix-argument arm used `query_result_from_error`,
    // which read a halt marker smuggled inside `EvalError`; `result_to_owned`
    // now returns `Result<OwnedValue, EvalEscape>` directly and this arm just
    // forwards the escape via `From<EvalEscape> for QueryResult`. Verified
    // against jq 1.7.1: `jq -n '"abc" | endswith(halt_error(9))'` exits 9
    // with nothing on stdout.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | endswith(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_split_propagates_halt_in_separator_argument() -> Result<()> {
    // `builtin_split`'s separator-argument arm: same `query_result_from_error`
    // -> `e.into()` refactor as `endswith` above, now exercised at this
    // distinct call site. Verified against jq 1.7.1:
    // `jq -n '"a,b" | split(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""a,b" | split(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_join_propagates_halt_in_separator_argument() -> Result<()> {
    // `builtin_join`'s separator-argument arm. Verified against jq 1.7.1:
    // `jq -n '["a","b"] | join(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#"["a","b"] | join(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_contains_propagates_halt_in_argument() -> Result<()> {
    // `builtin_contains`'s `b`-argument arm. Verified against jq 1.7.1:
    // `jq -n '"abc" | contains(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | contains(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_inside_propagates_halt_in_argument() -> Result<()> {
    // `builtin_inside`'s `b`-argument arm -- `inside`'s own dedicated call
    // site, distinct from `contains`'s even though the two share
    // `owned_contains`. Verified against jq 1.7.1:
    // `jq -n '"abc" | inside(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | inside(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_nth_single_arg_propagates_halt_in_index_argument() -> Result<()> {
    // `builtin_nth`'s index-argument arm -- the single-argument `nth(n)`
    // form (`.[n]`), a distinct function from `builtin_nth_stream`, which
    // backs the two-argument `nth(n; expr)` form already covered by
    // `test_nth_stream_propagates_halt_in_n_argument` above. Verified
    // against jq 1.7.1: `jq -n '[1,2,3] | nth(halt_error(9))'` exits 9 with
    // no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2,3] | nth(halt_error(9))"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_flatten_depth_propagates_halt_in_depth_argument() -> Result<()> {
    // `builtin_flatten_depth`'s depth-argument arm (the `flatten(depth)`
    // two-arg-equivalent form, not the depth-less `flatten` builtin).
    // Verified against jq 1.7.1: `jq -n '[[1,2]] | flatten(halt_error(9))'`
    // exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "[[1,2]] | flatten(halt_error(9))"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_group_by_propagates_halt_in_key_function() -> Result<()> {
    // `builtin_group_by`'s per-item match on `eval_array_construction`'s
    // result now has its own `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm (previously the wildcard below it would
    // have hit `unreachable!()` on a halt, since that arm was written
    // assuming only Owned/Error/Break could reach it pre-#791). Verified
    // against jq 1.7.1: `jq -n '[1,2,3] | group_by(halt_error(9))'` exits 9
    // with no output -- the halt fires while keying the very first element.
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2,3] | group_by(halt_error(9))"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_unique_by_propagates_halt_in_key_function() -> Result<()> {
    // `builtin_unique_by`'s own copy of the `group_by` key-computation match,
    // now carrying the same `QueryResult::Halt` arm. Verified against jq
    // 1.7.1: `jq -n '[1,2,3] | unique_by(halt_error(9))'` exits 9 with no
    // output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2,3] | unique_by(halt_error(9))"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_sort_by_propagates_halt_in_key_function() -> Result<()> {
    // `builtin_sort_by`'s own copy of the same key-computation match, third
    // (and last) of the three sites sharing this shape. Verified against jq
    // 1.7.1: `jq -n '[1,2,3] | sort_by(halt_error(9))'` exits 9 with no
    // output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1,2,3] | sort_by(halt_error(9))"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// The four tests below are siblings of `test_group_by_propagates_halt_in_key_function`
/// above, but exercise `eval.rs`'s generic `eval_pipe`/`eval_index_expr` machinery
/// itself rather than `builtin_group_by`'s own match arm: that test's key
/// function (`halt_error(9)`, a bare builtin call) halts directly inside
/// `eval_single`'s `Expr::Builtin` dispatch without ever entering `eval_pipe`.
/// A key function that is itself a multi-stage `Pipe`/`IndexExpr` is needed to
/// reach `eval_pipe`'s own internal arms -- and it must be routed through
/// `group_by`/`sort_by`/`min_by`/`max_by`/`unique_by` (or another builtin whose
/// argument falls through `eval_generic.rs`'s `eval_on_owned` fallback) rather
/// than used as the top-level CLI filter, since ordinary pipes/index
/// expressions at the top level are handled natively by `eval_generic.rs`'s own
/// (separate) implementation and never reach `eval.rs` at all.
#[test]
fn test_eval_pipe_many_loop_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_pipe`'s `QueryResult::Many(values)` loop, `QueryResult::Halt`
    // arm: `group_by`'s single item is `[1,2,3]`, and the key function's
    // first stage (`.[]`) fans it out into three *borrowed* values before
    // the second stage halts on the second one. Verified against jq 1.7.1:
    // `jq -c 'group_by(.[] | if . == 2 then halt else . end)'` on `[[1,2,3]]`
    // exits 0 with no output -- array construction for the key is atomic, so
    // the `1` already produced before the halt never reaches stdout (#400
    // does not apply inside `[f]`).
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "group_by(.[] | if . == 2 then halt else . end)"],
        Some("[[1,2,3]]"),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_pipe_top_level_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_pipe`'s top-level `QueryResult::Halt(code) => QueryResult::Halt(code)`
    // arm: the key function's *first* stage (bare `halt`) halts outright,
    // reached via the `match result.materialize_cursor()` dispatch rather
    // than the `Many` loop above (there is a second pipe stage, `.`, so
    // `eval_pipe` cannot take its own `rest.is_empty()` early return either).
    // Verified against jq 1.7.1: `jq -c 'group_by(halt | .)'` on `[[1,2,3]]`
    // exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(halt | .)"], Some("[[1,2,3]]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_pipe_owned_prefix_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `pipe_owned_prefix`'s own `QueryResult::Halt` arm: the key function's
    // first stage (`1,2,3`, a comma of literals) fans out into *owned*
    // values, taking `eval_pipe`'s `ManyOwned` arm into `pipe_owned_prefix`
    // rather than the borrowed `Many` loop above. Verified against jq 1.7.1:
    // `jq -c 'group_by((1,2,3) | halt)'` on `[1]` exits 0 with no output --
    // the halt fires while piping the very first generated value (`1`)
    // through the second stage.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by((1,2,3) | halt)"], Some("[1]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_direct_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_index_expr`'s direct `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm: the key function's computed-index key
    // expression (`halt_error(3)`) halts outright with zero keys ever
    // produced, distinct from the `Partial(vs, Control::Halt(code))` arm
    // right below it (which handles a key stream that halts *after*
    // already yielding some keys). `halt_error`'s non-string argument is the
    // group_by item itself (`1`), printed as JSON to stderr per halt_error's
    // own contract -- not stdout. Verified against jq 1.7.1: `jq -c
    // 'group_by(.[(halt_error(3))])'` on `[1]` exits 3 with no stdout.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(.[(halt_error(3))])"], Some("[1]"))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains('1'), "stderr: {stderr:?}");
    Ok(())
}

/// The seven tests below cover `eval_index_expr`/`eval_slice_expr`/
/// `eval_slice_bound`'s remaining halt-propagation arms, all reached the same
/// way as the test above -- through a `group_by` key function, since a
/// top-level `E[K]`/`E[S:T]` is handled natively by `eval_generic.rs` and
/// never reaches `eval.rs` at all. One parser subtlety applies throughout:
/// `parse_postfix` only builds an `Expr::IndexExpr`/`Expr::SliceExpr` (with
/// its own explicit `target`) when the bracket contains at least one
/// *computed* key/bound (`push_bracket`'s `Bracket::Dynamic`/
/// `Bracket::DynamicSlice`); a bracket with only *literal* contents (`[0]`,
/// `[0:1]`) is a flat chain element instead, folding `PRECEDING[LITERAL]`
/// into a plain `Expr::Pipe` that never constructs either node. So every
/// filter below keeps at least one bracket position non-literal (`.`, or a
/// `halt`-containing sub-expression) purely to force the right AST shape --
/// independent of which arm is under test.
#[test]
fn test_eval_index_expr_target_none_with_pending_key_halt_reached_through_group_by_key_fn(
) -> Result<()> {
    // `eval_index_expr`'s `QueryResult::None => { return match pending_halt
    // { Some(code) => QueryResult::Halt(code), ... } }` arm: the key stream
    // (`1, halt`) yields one key before halting (setting `pending_halt`),
    // but the target (`empty`) produces zero outputs -- so there is no
    // key/target pair left to index, yet the key side's pending halt must
    // still fire rather than silently degrading to `None`. Verified against
    // jq 1.7.1: `jq -c 'group_by(empty[(1,halt)])'` on `[1]` exits 0 with no
    // output (`empty[...]` is legitimately empty regardless).
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(empty[(1,halt)])"], Some("[1]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_target_direct_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_index_expr`'s target-materialization match has its own direct
    // `QueryResult::Halt(code) => return QueryResult::Halt(code)` arm,
    // distinct from the key-stream's own (already-covered) direct-halt arm:
    // here the *key* (`.`) succeeds trivially, but the *target* (`halt`)
    // halts outright. Verified against jq 1.7.1: `jq -c 'group_by(halt[.])'`
    // on `[5]` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(halt[.])"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_target_partial_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_index_expr`'s target-materialization match, `Partial(_,
    // Control::Halt(code))` arm: the target (`1,2,halt`) produces two real
    // outputs before halting -- conservatively treated the same as a direct
    // halt (the already-produced prefix is discarded, matching the key
    // stream's own conservative `Error`/`Break` treatment right above it in
    // the source). Verified against jq 1.7.1: `jq -c 'group_by((1,2,halt)[.])'`
    // on `[5]` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by((1,2,halt)[.])"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_owned_targets_pending_key_halt_reached_through_group_by_key_fn(
) -> Result<()> {
    // `eval_index_expr`'s `Targets::Owned` branch, `pending_halt` arm --
    // the *owned* twin of the already-covered `Targets::Borrowed` arm right
    // above it. The target (`[9,8,7],[6,5,4]`, two array literals) is
    // *computed* rather than borrowed from the input, taking the `Owned`
    // branch; the key (`0, halt`) yields one real key (`0`) before halting.
    // Both targets get indexed by the one produced key before the pending
    // halt fires, so this also proves the already-indexed values (`9`, `6`)
    // are computed before being discarded, not skipped outright. Verified
    // against jq 1.7.1: `jq -c 'group_by(([9,8,7],[6,5,4])[(0, halt)])'` on
    // `[5]` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "group_by(([9,8,7],[6,5,4])[(0, halt)])"],
        Some("[5]"),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_slice_expr_start_bound_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_slice_expr`'s start-bound evaluation: `eval_slice_bound`
    // returning `Err(Control::Halt(code))` (because the start bound itself,
    // `halt`, halts) must abort before the end bound or target are ever
    // touched. Verified against jq 1.7.1: `jq -c 'group_by(.[(halt):2])'`
    // on `[5]` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(.[(halt):2])"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_slice_expr_end_bound_halt_reached_through_group_by_key_fn() -> Result<()> {
    // Sibling of the test above for the *end* bound: the start bound (`0`)
    // succeeds first, then the end bound (`halt`) halts. Distinct call site
    // from the start-bound test (`eval_slice_expr` evaluates the two bounds
    // with two separate `eval_slice_bound` calls). Verified against jq
    // 1.7.1: `jq -c 'group_by(.[0:(halt)])'` on `[5]` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(.[0:(halt)])"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_slice_expr_target_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_slice_expr`'s own target-materialization match, direct-halt arm
    // (mirrors `eval_index_expr`'s equivalent arm, tested above, but this is
    // a separate match in a separate function). Both bounds (`0`, `.`)
    // succeed trivially -- the end bound is `.` rather than a literal
    // *purely* to keep the bracket "dynamic" so the parser builds an
    // `Expr::SliceExpr` with an explicit `target` at all (see this group's
    // doc comment); it plays no role in the halt itself, which comes from
    // the target (`halt`). Verified against jq 1.7.1: `jq -c
    // 'group_by(halt[0:.])'` on `[5]` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(halt[0:.])"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_slice_expr_target_partial_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_slice_expr`'s target-materialization match, `Partial(_,
    // Control::Halt(code))` arm -- the `eval_slice_expr` sibling of
    // `test_eval_index_expr_target_partial_halt_reached_through_group_by_key_fn`.
    // The target (`1,2,halt`) produces two real outputs before halting;
    // like the index-expr case, the already-produced prefix is discarded
    // rather than sliced. Verified against jq 1.7.1: `jq -c
    // 'group_by((1,2,halt)[0:.])'` on `[5]` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by((1,2,halt)[0:.])"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_with_entries_propagates_halt_from_map_function() -> Result<()> {
    // `builtin_with_entries`'s `map(f)` loop: a bare (non-`Partial`) halt
    // from evaluating `f` against one entry must abort the whole builtin
    // immediately via its `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm, rather than being folded into the
    // `transformed` vector. Verified against jq 1.7.1:
    // `jq -n '{"a":1} | with_entries(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"{"a":1} | with_entries(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_with_entries_propagates_halt_after_partial_map_output() -> Result<()> {
    // Distinct arm from the bare-halt one above: when `f` yields one or more
    // values and *then* halts, `eval_single(...).materialize_cursor()`
    // returns `QueryResult::Partial(_, Control::Halt(code))`, which must
    // also abort immediately rather than silently folding the partial
    // prefix into `transformed`. Verified against jq 1.7.1:
    // `jq -n '{"a":1} | with_entries(., halt_error(9))'` exits 9 with no
    // output (real jq discards the same partial output here too, since
    // `with_entries` is `from_entries(map(f))` and array construction is
    // atomic).
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"{"a":1} | with_entries(., halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_string_interpolation_propagates_bare_halt() -> Result<()> {
    // `eval_string_interpolation`'s `\(...)` slot: a bare (non-`Partial`)
    // halt from the embedded expression must abort the whole interpolated
    // string via its `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm. Verified against jq 1.7.1:
    // `jq -n '"\(halt_error(9))"'` exits 9 with no stdout and (since the
    // filter's overall input, `null`, is what `halt_error` receives here)
    // no stderr either.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""\(halt_error(9))""#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_string_interpolation_propagates_halt_after_partial_output() -> Result<()> {
    // Distinct arm from the bare-halt one above: when the `\(...)` slot's
    // expression yields a value and *then* halts, `materialize_cursor()`
    // returns `QueryResult::Partial(_, Control::Halt(code))`, handled here by
    // its own two-line arm. Per this function's own doc comment ("string
    // interpolation is atomic ... a `Partial` just surfaces its control,
    // same as a bare one"), succinctly deliberately does NOT match real
    // jq's behavior here: real jq forks the whole string per output of the
    // slot's generator (`jq -n '"\(1, halt_error(9))"'` prints `"1"` to
    // stdout *before* halting 9), but succinctly's `\(...)` embeds only the
    // slot's single embedded value and discards the rest of a multi-output
    // stream, so here the halt wins outright with no partial "1" on stdout.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""\(1, halt_error(9))""#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_skip_propagates_bare_halt_in_expr_argument() -> Result<()> {
    // `builtin_skip`'s `result` match: a bare halt from evaluating `expr`
    // (no prior output to skip past) surfaces via its own
    // `QueryResult::Halt(code) => QueryResult::Halt(code)` arm. `skip` is a
    // succinctly extension (real jq has no `skip/2` builtin -- confirmed via
    // `jq -n 'skip(1; 1,2,3)'` => "skip/2 is not defined"), so this is
    // checked against succinctly's own halt-propagation contract: the halt
    // must exit the process, not be swallowed as an ordinary `QueryResult`.
    let (stdout, stderr, code) = run_jq_full(&["-n", "skip(0; halt_error(9))"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_indices_propagates_halt_in_pattern_argument() -> Result<()> {
    // `builtin_indices`'s pattern-argument arm. Verified against jq 1.7.1:
    // `jq -n '"abc" | indices(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | indices(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_index_propagates_halt_in_pattern_argument() -> Result<()> {
    // `builtin_index`'s pattern-argument arm, distinct from `indices`'s own
    // copy above. Verified against jq 1.7.1:
    // `jq -n '"abc" | index(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | index(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_index_reports_invalid_slice_descriptor_error() -> Result<()> {
    // `builtin_index`'s `sliced` match: when the pattern is an object (jq's
    // slice-descriptor form, e.g. `{start:...}`) but `SliceBounds::
    // from_descriptor` rejects it, the `Err(e) => e.into()` arm here forwards
    // a plain (non-halt) `EvalError` -- distinct from the two `Err(e) =>
    // e.into()` arms above that forward an `EvalEscape`. Verified against jq
    // 1.7.1: `jq -n '"abcdef" | index({start:"x"})'` errors with "Array/
    // string slice indices must be integers" and exits 5 (confirmed this is
    // reached rather than the sibling `Ok(_) => cannot_index_with_type(...)`
    // arm by first checking `jq -n '"abcdef" | index({start:1,end:3})'`,
    // which *does* hit that other arm with "Cannot index string with
    // number").
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abcdef" | index({start:"x"})"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Array/string slice indices must be integers"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_rindex_propagates_halt_in_pattern_argument() -> Result<()> {
    // `builtin_rindex`'s pattern-argument arm, `rindex`'s own copy of the
    // `index`/`indices` shape. Verified against jq 1.7.1:
    // `jq -n '"abc" | rindex(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | rindex(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_rindex_reports_invalid_slice_descriptor_error() -> Result<()> {
    // `builtin_rindex`'s `sliced` match, same shape as `index`'s above but at
    // its own call site. Verified against jq 1.7.1:
    // `jq -n '"abcdef" | rindex({start:"x"})'` errors with "Array/string
    // slice indices must be integers" and exits 5.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abcdef" | rindex({start:"x"})"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Array/string slice indices must be integers"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_getpath_propagates_halt_in_path_argument() -> Result<()> {
    // `builtin_getpath`'s path-argument arm (distinct from `builtin_setpath`'s
    // already-covered one). Verified against jq 1.7.1:
    // `jq -n '{"a":1} | getpath([(halt_error(9))])'` exits 9 with no stdout
    // (the input `{"a":1}` goes to stderr instead, from `halt_error`).
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"{"a":1} | getpath([(halt_error(9))])"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_getpath_reports_invalid_string_slice_descriptor_error() -> Result<()> {
    // `builtin_getpath`'s `(String(s), Object(desc))` segment arm: a
    // malformed slice descriptor against a *string* current value hits its
    // own `Err(e) => e.into()`, a separate call site from the sibling array
    // segment arm just above it (which is not in this cluster -- it isn't a
    // newly-uncovered line, unlike this one). Verified against jq 1.7.1:
    // `jq -n '"abcdef" | getpath([{"start":"x"}])'` errors with "Array/
    // string slice indices must be integers" and exits 5.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#""abcdef" | getpath([{"start":"x"}])"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Array/string slice indices must be integers"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_test_builtin_propagates_halt_in_pattern_argument() -> Result<()> {
    // `builtin_test_regex`'s pattern-argument arm (the `test(re)` builtin,
    // gated on the `regex` feature, which the `cli` feature enables).
    // Verified against jq 1.7.1: `jq -n '"abc" | test(halt_error(9))'`
    // exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | test(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_match_propagates_halt_in_pattern_argument() -> Result<()> {
    // `builtin_match`'s pattern-argument arm, `match`'s own copy of the
    // `test`-shaped pattern evaluation. Verified against jq 1.7.1:
    // `jq -n '"abc" | match(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | match(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_match_reports_invalid_regex_error() -> Result<()> {
    // `builtin_match`'s `build_regex` arm: `build_regex` returns a plain
    // `Result<JqRegex, EvalError>` (it only compiles a pattern string, never
    // evaluates a jq expression), so its `Err(e) => e.into()` here can only
    // ever forward an ordinary `EvalError`, never a halt -- a distinct case
    // from the pattern-argument arm above. Verified against jq 1.7.1:
    // `jq -n '"abc" | match("(")'` errors with "Regex failure: end pattern
    // with unmatched parenthesis" and exits 5; succinctly reports its own
    // "invalid regex: ..." wording for the same malformed pattern (message
    // parity is a separate concern from the exit code, per this file's other
    // uncaught-error tests).
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | match("(")"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("invalid regex"), "{stderr}");
    Ok(())
}

#[test]
fn test_scan_propagates_halt_in_pattern_argument() -> Result<()> {
    // `builtin_scan`'s pattern-argument arm. Verified against jq 1.7.1:
    // `jq -n '"abc" | scan(halt_error(9))'` exits 9 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | scan(halt_error(9))"#], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_scan_reports_invalid_regex_error() -> Result<()> {
    // `builtin_scan`'s `build_regex` arm, same shape as `match`'s above but
    // at `scan`'s own call site. Verified against jq 1.7.1:
    // `jq -n '"abc" | scan("(")'` errors with "Regex failure: end pattern
    // with unmatched parenthesis" and exits 5.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | scan("(")"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("invalid regex"), "{stderr}");
    Ok(())
}

/// `builtin_splits`'s pattern-argument arm (`splits(re)`, the 1-arg stream
/// form) now forwards `result_to_owned`'s `Err(EvalEscape)` via `.into()`
/// instead of the old bare `QueryResult::Error(e)` wrap that could only ever
/// hold an `EvalError` -- the conversion this PR adds so a halt smuggled back
/// through the pattern argument's evaluation keeps being a halt instead of an
/// ordinary catchable error. Verified against jq 1.7.1: `jq 'splits(halt_error(21))'`
/// on `"abc"` exits 21 with empty stdout.
#[test]
fn test_splits_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "splits(halt_error(21))"], Some(r#""abc""#))?;
    assert_eq!(code, 21, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_splits`'s `build_regex` arm, reached once the pattern argument
/// itself evaluates cleanly to a string that fails to compile as a regex.
/// This arm's `Err(e)` is a plain `EvalError` (not an `EvalEscape`), so
/// `.into()` here is the same conversion as before the refactor -- this test
/// confirms the ordinary "bad regex" path still works post-refactor.
/// Verified against jq 1.7.1: `jq 'splits("[")'` on `"abc"` exits 5
/// (oniguruma reports "premature end of char-class"; succinctly reports its
/// own message via the Rust `regex` crate, so only the exit code and empty
/// stdout are asserted here).
#[test]
fn test_splits_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"splits("[")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_sub`'s pattern-argument arm (`sub(re; replacement)`, the 2-arg
/// no-flags form) forwards a smuggled-back halt via `.into()` instead of
/// downgrading it to an ordinary error. Verified against jq 1.7.1:
/// `jq 'sub(halt_error(22); "x")'` on `"abc"` exits 22 with empty stdout.
#[test]
fn test_sub_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"sub(halt_error(22); "x")"#], Some(r#""abc""#))?;
    assert_eq!(code, 22, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_sub`'s replacement-argument arm, a distinct evaluation site from
/// the pattern arm one match block above -- the pattern must find a match
/// first, so this exercises `eval_sub_replacement`'s per-match evaluation of
/// the replacement expression (#826: `.` is bound to that match's captures,
/// not the original input). Verified against jq 1.7.1: `jq 'sub("a";
/// halt_error(23))'` on `"abc"` exits 23 with empty stdout.
#[test]
fn test_sub_propagates_halt_in_replacement_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"sub("a"; halt_error(23))"#], Some(r#""abc""#))?;
    assert_eq!(code, 23, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_sub`'s `build_regex` arm (2-arg, no-flags form), reached once
/// both pattern and replacement evaluate cleanly but the pattern string
/// itself fails to compile as a regex. Verified against jq 1.7.1:
/// `jq 'sub("["; "x")'` on `"abc"` exits 5 with empty stdout.
#[test]
fn test_sub_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("["; "x")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_gsub`'s pattern-argument arm (`gsub(re; replacement)`, the 2-arg
/// no-flags form) -- same shape as `builtin_sub`'s pattern arm, but this is
/// a distinct function/site in the source. Verified against jq 1.7.1:
/// `jq 'gsub(halt_error(24); "x")'` on `"abc"` exits 24 with empty stdout.
#[test]
fn test_gsub_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"gsub(halt_error(24); "x")"#], Some(r#""abc""#))?;
    assert_eq!(code, 24, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_gsub`'s replacement-argument arm, evaluated per match via
/// `stitch_replacements_evaluated`/`eval_sub_replacement` (#826). Verified
/// against jq 1.7.1: `jq 'gsub("a"; halt_error(25))'` on `"abc"` exits 25
/// with empty stdout.
#[test]
fn test_gsub_propagates_halt_in_replacement_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"gsub("a"; halt_error(25))"#], Some(r#""abc""#))?;
    assert_eq!(code, 25, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_test_flags`'s flags-argument arm (`test(re; flags)`) -- flags is
/// evaluated *before* the pattern in this function, so this is the first
/// `result_to_owned(eval_single(...))` call site hit. Verified against jq
/// 1.7.1: `jq 'test("a"; halt_error(26))'` on `"abc"` exits 26 with empty
/// stdout.
#[test]
fn test_test_flags_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"test("a"; halt_error(26))"#], Some(r#""abc""#))?;
    assert_eq!(code, 26, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_test_flags`'s pattern-argument arm, reached only after the flags
/// argument evaluates cleanly -- a distinct site from the flags arm above.
/// Verified against jq 1.7.1: `jq 'test(halt_error(27); "i")'` on `"abc"`
/// exits 27 with empty stdout.
#[test]
fn test_test_flags_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"test(halt_error(27); "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 27, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_test_flags`'s `build_regex` arm, reached once both flags and
/// pattern evaluate cleanly but the pattern fails to compile as a regex.
/// Verified against jq 1.7.1: `jq 'test("["; "i")'` on `"abc"` exits 5 with
/// empty stdout.
#[test]
fn test_test_flags_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"test("["; "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_match_flags`'s flags-argument arm (`match(re; flags)`) --
/// evaluated before it ever delegates to `builtin_match`, so a halt here
/// must escape without `builtin_match` getting a chance to run at all.
/// Verified against jq 1.7.1: `jq 'match("a"; halt_error(28))'` on `"abc"`
/// exits 28 with empty stdout.
#[test]
fn test_match_flags_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"match("a"; halt_error(28))"#], Some(r#""abc""#))?;
    assert_eq!(code, 28, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_capture_flags`'s flags-argument arm (`capture(re; flags)`) --
/// evaluated before it ever delegates to `builtin_capture_with_flags`, so a
/// halt here must escape before capture's own pattern/build_regex arms are
/// ever reached. Verified against jq 1.7.1: `jq 'capture("a"; halt_error(29))'`
/// on `"abc"` exits 29 with empty stdout.
#[test]
fn test_capture_flags_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"capture("a"; halt_error(29))"#], Some(r#""abc""#))?;
    assert_eq!(code, 29, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_capture_with_flags`'s pattern-argument arm, reached directly from
/// the bare `capture(re)` (1-arg) form -- `Builtin::Capture` calls
/// `builtin_capture_with_flags` with `flags: None`, skipping the flags-arm
/// site entirely, so this is a distinct code path from `capture(re; flags)`
/// above. Verified against jq 1.7.1: `jq 'capture(halt_error(30))'` on
/// `"abc"` exits 30 with empty stdout.
#[test]
fn test_capture_bare_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "capture(halt_error(30))"], Some(r#""abc""#))?;
    assert_eq!(code, 30, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_capture_with_flags`'s `build_regex` arm, reached via the bare
/// `capture(re)` (1-arg) form once the pattern evaluates cleanly to a string
/// that fails to compile as a regex. Verified against jq 1.7.1:
/// `jq 'capture("[")'` on `"abc"` exits 5 with empty stdout.
#[test]
fn test_capture_bare_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"capture("[")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_sub_flags`'s flags-argument arm (`sub(re; replacement; flags)`,
/// the 3-arg form) -- flags is evaluated before delegating to
/// `builtin_sub_with_flags`, so a halt here must escape before that
/// function's own pattern/replacement/build_regex arms are ever reached.
/// Verified against jq 1.7.1: `jq 'sub("a"; "b"; halt_error(31))'` on
/// `"abc"` exits 31 with empty stdout.
#[test]
fn test_sub_flags_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"sub("a"; "b"; halt_error(31))"#],
        Some(r#""abc""#),
    )?;
    assert_eq!(code, 31, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_sub_with_flags`'s pattern-argument arm, reached from the 3-arg
/// `sub(re; replacement; flags)` form once the flags argument (a literal
/// here) has already resolved cleanly. Verified against jq 1.7.1:
/// `jq 'sub(halt_error(32); "b"; "i")'` on `"abc"` exits 32 with empty
/// stdout.
#[test]
fn test_sub_with_flags_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"sub(halt_error(32); "b"; "i")"#],
        Some(r#""abc""#),
    )?;
    assert_eq!(code, 32, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_sub_with_flags`'s replacement-argument arm, reached once flags
/// and pattern resolve cleanly and a match is found -- the replacement is
/// evaluated per match via `eval_sub_replacement` (#826), not once up
/// front. Verified against jq 1.7.1:
/// `jq 'sub("a"; halt_error(33); "i")'` on `"abc"` exits 33 with empty
/// stdout.
#[test]
fn test_sub_with_flags_propagates_halt_in_replacement_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"sub("a"; halt_error(33); "i")"#],
        Some(r#""abc""#),
    )?;
    assert_eq!(code, 33, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_sub_with_flags`'s `build_regex` arm (3-arg `sub` form), reached
/// once flags, pattern and replacement all evaluate cleanly but the pattern
/// fails to compile as a regex. Verified against jq 1.7.1:
/// `jq 'sub("["; "b"; "i")'` on `"abc"` exits 5 with empty stdout.
#[test]
fn test_sub_with_flags_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("["; "b"; "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `eval_sub_replacement`'s non-optional type-mismatch arm (#826): once a
/// match is found, the replacement expression is evaluated per match and
/// must produce a string. Real jq also errors here (`string ("") and number
/// (5) cannot be added`, from its own `+=`-based definition) -- succinctly's
/// wording differs (a direct type check) but the exit code matches. Verified
/// against jq 1.7.1: `jq 'sub("a"; 5)'` on `"abc"` exits 5.
#[test]
fn test_sub_replacement_wrong_type_errors() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("a"; 5)"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// Black-box counterpart to the test above: `sub("a"; 5)?` swallows the same
/// non-string replacement instead of raising it. Note this exercises
/// `eval_sub_replacement` with `optional: false` (`E?`'s `Expr::Optional`
/// dispatch evaluates `E` with the *ambient* optional and lets its own
/// `eval_try` catch the aggregate error -- see that arm's doc comment,
/// `Expr::Optional(inner) => eval_try::<W, S>(inner, None, value,
/// optional)`), so it hits the same non-optional error arm as the test above
/// internally; `test_sub_replacement_wrong_type_optional_via_isvalid` below
/// is what actually drives `eval_sub_replacement`'s own `optional: true`
/// arm. Verified against jq 1.7.1: `jq 'sub("a"; 5)?'` on `"abc"` exits 0
/// with empty stdout.
#[test]
fn test_sub_replacement_wrong_type_optional_is_silent() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("a"; 5)?"#], Some(r#""abc""#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `eval_sub_replacement`'s `optional: true` type-mismatch arm (#826),
/// reached via `isvalid`, which -- unlike a bare `?` postfix (see the test
/// above) -- forces `optional: true` all the way down into the expression it
/// validates (already exploited by `test_isvalid_propagates_halt_from_error_message_expression`
/// for the same reason). `isvalid` is a succinctly/jq-1.8+ builtin absent
/// from the pinned jq-1.7.1 oracle, so this is a self-contained assertion
/// rather than a golden fixture; the type mismatch makes `sub` invalid
/// either way, so `isvalid` reports `false` regardless of which arm
/// internally handles it -- this test's value is in exercising the
/// `optional: true` code path itself, not in a distinguishable outward
/// symptom.
#[test]
fn test_sub_replacement_wrong_type_optional_via_isvalid() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", r#"isvalid(sub("a"; 5))"#], Some(r#""abc""#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "false\n");
    Ok(())
}

/// `builtin_sub_with_flags`'s `global` arm (3-arg `sub(re; replacement;
/// "g")`, i.e. `sub` used as `gsub` via an explicit flag) propagating a
/// replacement error through `stitch_replacements_evaluated` (#826) -- a
/// distinct call site from plain `gsub`'s own error propagation tested
/// above. Verified against jq 1.7.1: `jq 'sub("a"; 5; "g")'` on `"abc"`
/// exits 5.
#[test]
fn test_sub_with_flags_global_replacement_wrong_type_errors() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("a"; 5; "g")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `eval_sub_replacement`'s multi-output stopgap (#826): a replacement
/// filter that yields more than one value for a single match (a real jq
/// feature -- jq 1.7.1 forks the whole `sub`/`gsub` call, producing one
/// whole-string output per replacement value, verified live: `jq -c
/// 'sub("a"; "x","y")'` on `"abc"` prints `"xbc"` then `"ybc"`) is not fully
/// implemented here; `eval_sub_replacement` instead takes the first value,
/// via `result_to_owned`'s policy, matching what the pre-#826 code already
/// did when it pre-evaluated the whole replacement once. This is a
/// deliberate, documented divergence (see that function's doc comment and
/// follow-up #840), not a golden-fixture case (there is no single jq output
/// to pin against). What this test guards against is a *regression* off
/// that stopgap: earlier in #826's own review cycle, routing the
/// per-match evaluation through `eval_owned_expr` (which array-collapses a
/// multi-output filter) turned this into a hard type-mismatch error instead.
#[test]
fn test_sub_replacement_multi_value_takes_first_value() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("a"; "x","y")"#], Some(r#""abc""#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"xbc\"\n");
    Ok(())
}

/// `gsub` counterpart to the test above, routed through
/// `stitch_replacements_evaluated` rather than `builtin_sub_with_flags`'s
/// single-match arm -- a distinct call site for the same stopgap (#826).
#[test]
fn test_gsub_replacement_multi_value_takes_first_value() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"gsub("a"; "x","y")"#], Some(r#""aa""#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"xx\"\n");
    Ok(())
}

/// `builtin_gsub_flags`'s flags-argument arm (`gsub(re; replacement; flags)`,
/// the 3-arg form) -- same shape as `builtin_sub_flags`'s flags arm, but a
/// distinct function/site in the source. Verified against jq 1.7.1:
/// `jq 'gsub("a"; "b"; halt_error(34))'` on `"abc"` exits 34 with empty
/// stdout.
#[test]
fn test_gsub_flags_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"gsub("a"; "b"; halt_error(34))"#],
        Some(r#""abc""#),
    )?;
    assert_eq!(code, 34, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_gsub_with_flags`'s pattern-argument arm, reached from the 3-arg
/// `gsub(re; replacement; flags)` form once the flags argument has already
/// resolved cleanly. Verified against jq 1.7.1:
/// `jq 'gsub(halt_error(35); "b"; "i")'` on `"abc"` exits 35 with empty
/// stdout.
#[test]
fn test_gsub_with_flags_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"gsub(halt_error(35); "b"; "i")"#],
        Some(r#""abc""#),
    )?;
    assert_eq!(code, 35, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_gsub_with_flags`'s replacement-argument arm, reached once both
/// flags and pattern resolve cleanly -- the replacement is evaluated per
/// match via `stitch_replacements_evaluated`/`eval_sub_replacement` (#826).
/// Verified against jq 1.7.1:
/// `jq 'gsub("a"; halt_error(36); "i")'` on `"abc"` exits 36 with empty
/// stdout.
#[test]
fn test_gsub_with_flags_propagates_halt_in_replacement_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"gsub("a"; halt_error(36); "i")"#],
        Some(r#""abc""#),
    )?;
    assert_eq!(code, 36, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_gsub_with_flags`'s `build_regex` arm (3-arg `gsub` form),
/// reached once flags, pattern and replacement all evaluate cleanly but the
/// pattern fails to compile as a regex. Verified against jq 1.7.1:
/// `jq 'gsub("["; "b"; "i")'` on `"abc"` exits 5 with empty stdout.
#[test]
fn test_gsub_with_flags_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"gsub("["; "b"; "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_scan_flags`'s flags-argument arm (`scan(re; flags)`) -- evaluated
/// before it ever delegates to `builtin_scan_with_flags`. Verified against
/// jq 1.7.1: `jq '[scan("a"; halt_error(37))]'` on `"abc"` exits 37 with
/// empty stdout (the enclosing array collector never gets a chance to close).
#[test]
fn test_scan_flags_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"scan("a"; halt_error(37))"#], Some(r#""abc""#))?;
    assert_eq!(code, 37, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_scan_with_flags`'s pattern-argument arm, reached from
/// `scan(re; flags)` once the flags argument has already resolved cleanly.
/// Verified against jq 1.7.1: `jq 'scan(halt_error(38); "i")'` on `"abc"`
/// exits 38 with empty stdout.
#[test]
fn test_scan_with_flags_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"scan(halt_error(38); "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 38, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_scan_with_flags`'s `build_regex` arm, reached once flags and
/// pattern both evaluate cleanly but the pattern fails to compile as a
/// regex. Verified against jq 1.7.1: `jq 'scan("["; "i")'` on `"abc"` exits
/// 5 with empty stdout.
#[test]
fn test_scan_with_flags_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"scan("["; "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_split_regex`'s flags-argument arm (`split(re; flags)`, the
/// regex-based 2-arg form distinct from the literal-separator `split(sep)`)
/// -- flags is evaluated before the pattern in this function. Verified
/// against jq 1.7.1: `jq 'split("a"; halt_error(39))'` on `"abc"` exits 39
/// with empty stdout.
#[test]
fn test_split_regex_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"split("a"; halt_error(39))"#], Some(r#""abc""#))?;
    assert_eq!(code, 39, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_split_regex`'s pattern-argument arm, reached once the flags
/// argument (`null`, a valid literal) has already resolved cleanly.
/// Verified against jq 1.7.1: `jq 'split(halt_error(40); null)'` on `"abc"`
/// exits 40 with empty stdout.
#[test]
fn test_split_regex_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "split(halt_error(40); null)"], Some(r#""abc""#))?;
    assert_eq!(code, 40, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_split_regex`'s `build_regex` arm, reached once flags and pattern
/// both evaluate cleanly but the pattern fails to compile as a regex.
/// Verified against jq 1.7.1: `jq 'split("["; null)'` on `"abc"` exits 5
/// with empty stdout.
#[test]
fn test_split_regex_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"split("["; null)"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_splits_flags`'s flags-argument arm (`splits(re; flags)`, the
/// 2-arg stream form) -- evaluated before it ever delegates to
/// `builtin_splits_with_flags`. Verified against jq 1.7.1:
/// `jq 'splits("a"; halt_error(41))'` on `"abc"` exits 41 with empty stdout.
#[test]
fn test_splits_flags_propagates_halt_in_flags_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"splits("a"; halt_error(41))"#], Some(r#""abc""#))?;
    assert_eq!(code, 41, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_splits_with_flags`'s pattern-argument arm, reached from
/// `splits(re; flags)` once the flags argument has already resolved
/// cleanly. Verified against jq 1.7.1: `jq 'splits(halt_error(42); "i")'`
/// on `"abc"` exits 42 with empty stdout.
#[test]
fn test_splits_with_flags_propagates_halt_in_pattern_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"splits(halt_error(42); "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 42, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_splits_with_flags`'s `build_regex` arm, reached once flags and
/// pattern both evaluate cleanly but the pattern fails to compile as a
/// regex. Verified against jq 1.7.1: `jq 'splits("["; "i")'` on `"abc"`
/// exits 5 with empty stdout.
#[test]
fn test_splits_with_flags_reports_invalid_regex_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"splits("["; "i")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `eval_pipe`'s `Many(values)` loop, `QueryResult::Halt(code)` arm: when the
/// pipe's first stage yields multiple borrowed values (here, `.[]` over an
/// array) and piping one of them through the rest of the pipe halts, any
/// values already piped through for earlier elements must still be flushed
/// as output before the halt takes effect -- the same "prefix survives, the
/// terminator wins" contract `Partial` already gave `Error`/`Break` (#400,
/// #494), extended to `Halt` by #791. Verified against jq 1.7.1:
/// `jq -c '.[] | if . == 2 then halt_error(43) else . end'` on `[1,2,3]`
/// prints `1` then exits 43 -- `2` and `3` never appear, and the halt is
/// reached mid-loop rather than at the very first or last element.
#[test]
fn test_eval_pipe_many_branch_propagates_halt_mid_stream() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", ".[] | if . == 2 then halt_error(43) else . end"],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 43, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_eval_pipe_many_loop_propagates_halt_from_borrowed_element() -> Result<()> {
    // `eval_pipe`'s `QueryResult::Many(values)` loop (reached when the first
    // stage of a pipe fans out into several *borrowed* results, e.g. `.[]`)
    // has its own `QueryResult::Halt(code)` arm, distinct from the top-level
    // `Halt` arm a few lines up in the same function: this one fires when
    // piping one of those borrowed elements through the rest of the pipe
    // halts partway through the loop, not when the very first stage halts
    // outright. Verified against jq 1.7.1: `[1,2,3] | .[] | halt_error(3)`
    // halts on the very first element, exit 3, no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[] | halt_error(3)"], Some("[1,2,3]"))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_pipe_top_level_propagates_direct_halt() -> Result<()> {
    // `eval_pipe`'s top-level `match result.materialize_cursor()` has its own
    // `QueryResult::Halt(code) => QueryResult::Halt(code)` arm for when the
    // *first* stage of the pipe halts outright -- distinct from the sibling
    // arm inside the `Many` loop, which only fires for one element of a
    // fanned-out borrowed stream (see the test above). Verified against jq
    // 1.7.1: `halt_error(3) | .` exits 3 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "halt_error(3) | ."], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_pipe_owned_prefix_propagates_halt() -> Result<()> {
    // `pipe_owned_prefix` -- the owned-value twin of `eval_pipe`'s `Many`
    // loop, reached when the first stage of a pipe fans out into several
    // *owned* results (here, a comma of literals) rather than values
    // borrowed straight from the input document -- has the same
    // per-element `Halt` arm the borrowed loop does. Verified against jq
    // 1.7.1: `(1,2,3) | halt_error(5)` halts while piping the first
    // generated value (`1`) through `halt_error`, exit 5, no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "(1,2,3) | halt_error(5)"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_key_stream_propagates_direct_halt() -> Result<()> {
    // `eval_index_expr`'s key-stream match has a direct `QueryResult::Halt`
    // arm for when the *whole* key expression halts with zero keys already
    // produced -- distinct from the `Partial(vs, Control::Halt(code))` arm
    // right below it, which handles a key stream that halts *after*
    // already yielding some keys (see the `pending_halt` tests further
    // down). Verified against jq 1.7.1: `.[(halt_error(3))]` exits 3 with
    // no output -- the key generator never gets past its first attempt, so
    // the target is never even touched.
    let (stdout, stderr, code) = run_jq_full(&["-n", ".[(halt_error(3))]"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_pending_halt_survives_an_empty_target() -> Result<()> {
    // `eval_index_expr`'s `QueryResult::None` target arm: a target with zero
    // outputs (here, `empty`) indexes to zero results for every key -- it
    // is not itself an error/break/halt that could "happen first" -- so a
    // `pending_halt` already captured from the key stream (`(0,1,
    // halt_error(7))`, which yields two keys before halting) must still
    // win, rather than being discarded just because the target produced
    // nothing (#791). Verified live against jq 1.7.1's matching contract:
    // `empty[(0,1,halt_error(7))]` exits 7 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "empty[(0,1,halt_error(7))]"], None)?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_target_propagates_direct_halt() -> Result<()> {
    // `eval_index_expr`'s *target* stream has the same direct
    // `QueryResult::Halt` arm the key stream does (see the key-side test
    // above), just on the other operand: the key stream here (`(0,0)`)
    // resolves cleanly to two ordinary keys first, and it is evaluating
    // the target (`halt_error(5)`) that halts outright. Verified against jq
    // 1.7.1: `(halt_error(5))[(0,0)]` exits 5 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "(halt_error(5))[(0,0)]"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_target_partial_halt_discards_prefix() -> Result<()> {
    // `eval_index_expr`'s target-side `QueryResult::Partial(_, Control::
    // Halt(code))` arm: unlike the *key*-side `pending_halt` handling
    // (#791, see `test_eval_index_expr_owned_target_flushes_prior_keys_
    // before_halting`), the *target* side has no equivalent bookkeeping --
    // it is the same conservative treatment this function's own doc
    // comment gives the target's `Error`/`Break` arms right above it
    // ("conservatively matching the existing Error/Break arms rather than
    // inventing new partial-key behavior"), just extended uniformly to
    // `Halt`. The already-produced target values (`[1,2]` and `[3,4]`, the
    // first two comma operands, evaluated before the third one halts) are
    // discarded rather than flushed -- a real, pre-existing divergence from
    // jq 1.7.1, which streams `1` then `3` (`[1,2][0]` then `[3,4][0]`)
    // before halting: `([1,2],[3,4],halt_error(6))[(0,0)]` on real jq exits
    // 6 with stdout "1\n3\n"; succinctly exits 6 with no output at all,
    // since it evaluates the whole target stream up front rather than
    // interleaving it with indexing.
    let (stdout, stderr, code) = run_jq_full(&["-n", "([1,2],[3,4],halt_error(6))[(0,0)]"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_index_expr_owned_target_flushes_prior_keys_before_halting() -> Result<()> {
    // `eval_index_expr`'s `Targets::Owned` arm (an owned/computed target --
    // here, an inline array literal, rather than a value borrowed straight
    // from the input document) carries its own copy of the `pending_halt`
    // flush the `Targets::Borrowed` arm has: a key stream that yields some
    // keys before halting (`(0,1,halt_error(9))`) must still index the
    // target with every key already produced before the halt propagates
    // (#791). Verified against jq 1.7.1:
    // `[10,20,30][(0,1,halt_error(9))]` prints `10`, `20`, then exits 9
    // with nothing on stderr (halt_error's own argument is `null`, jq's -n
    // root). The array literal is wrapped in parens here because
    // succinctly's parser (unlike jq's) doesn't accept an index bracket
    // directly after an unparenthesized array-literal target.
    let (stdout, stderr, code) = run_jq_full(&["-n", "([10,20,30])[(0,1,halt_error(9))]"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "10\n20\n");
    Ok(())
}

#[test]
fn test_eval_slice_expr_start_bound_propagates_halt() -> Result<()> {
    // `eval_slice_expr`'s `starts` match converts `eval_slice_bound`'s
    // `Err(Control::Halt(code))` -- itself reached through
    // `eval_slice_bound`'s own `QueryResult::Halt` arm, since the start
    // bound here is a bare `halt_error` call with nothing produced before
    // it -- into a halt for the whole slice, before the end bound or the
    // target are ever touched. Verified against jq 1.7.1:
    // `.[(halt_error(3)):2]` exits 3 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", ".[(halt_error(3)):2]"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_slice_expr_end_bound_propagates_halt() -> Result<()> {
    // Same arm family as the start-bound test above, but the *end* bound's
    // own call into the shared `eval_slice_bound` function is what halts
    // here, after the start bound (`0`) already resolved cleanly. Verified
    // against jq 1.7.1: `.[0:(halt_error(4))]` exits 4 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", ".[0:(halt_error(4))]"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_slice_expr_target_propagates_direct_halt() -> Result<()> {
    // `eval_slice_expr`'s own `targets` match (mirroring `eval_index_expr`'s)
    // has a direct `QueryResult::Halt` arm for when the *target* -- not
    // either bound -- halts outright. Both bounds here (`1-1` and `2`) are
    // deliberately non-literal so the whole expression compiles to
    // `Expr::SliceExpr` (a literal `0:2` would fold to the static
    // `Expr::Slice` fast path instead and never reach this function).
    // Verified against jq 1.7.1: `(halt_error(5))[(1-1):2]` exits 5 with no
    // output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "(halt_error(5))[(1-1):2]"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_slice_expr_target_partial_halt_discards_prefix() -> Result<()> {
    // `eval_slice_expr`'s target-side `QueryResult::Partial(_, Control::
    // Halt(code))` arm: the same conservative "discard the already-
    // produced prefix" treatment `eval_index_expr`'s analogous target-side
    // arm has (see `test_eval_index_expr_target_partial_halt_discards_
    // prefix`), and the same real divergence from jq 1.7.1 follows: real
    // jq streams `[1,2]` then `[4,5]` (slicing each of the two arrays
    // before the third comma operand halts) before exiting 6, while
    // succinctly evaluates the whole target stream up front, sees it end
    // in `Partial(_, Halt(6))`, and discards the buffered `[1,2,3]`/
    // `[4,5,6]` entirely -- exit 6, no output at all.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "([1,2,3],[4,5,6],halt_error(6))[(1-1):2]"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_rhs_once_propagates_direct_halt() -> Result<()> {
    // `eval_rhs_once` (the RHS evaluator shared by `+=`/`-=`/`*=`/`/=`/`%=`)
    // has a direct `QueryResult::Halt` arm for when the RHS halts outright,
    // distinct from the `Partial(_, Control::Halt(code))` arm right below
    // it (a RHS that outputs a value *before* halting -- see
    // `test_compound_assign_propagates_halt_after_partial_rhs_output`).
    // Verified against jq 1.7.1: `{"a":0} | .a += halt_error(3)` exits 3
    // after printing the original document (`{"a":0}`, since `+=`'s RHS is
    // evaluated against the pristine input) to stderr, no stdout.
    let (stdout, stderr, code) = run_jq_full(&["-c", ".a += halt_error(3)"], Some(r#"{"a":0}"#))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_assign_rhs_propagates_direct_halt() -> Result<()> {
    // `eval_assign`'s own RHS-collecting match (distinct from
    // `eval_rhs_once`, which `+=`/`|=`/etc. use -- plain `=` forks over
    // every RHS output instead, #392) has its own `QueryResult::Halt` arm:
    // a RHS that halts outright collects zero `rhs_values` with
    // `terminal = Some(Control::Halt(code))`, so `eval_assign` returns a
    // bare halt without ever resolving or writing to any path. Verified
    // against jq 1.7.1: `{"a":0} | .a = halt_error(3)` exits 3 after
    // printing the original document to stderr, no stdout.
    let (stdout, stderr, code) = run_jq_full(&["-c", ".a = halt_error(3)"], Some(r#"{"a":0}"#))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_assign_optional_still_swallows_ordinary_path_error() -> Result<()> {
    // `eval_assign`'s `resolve_dynamic_indexes` match used to be a single
    // `Err((_, _)) if optional => return QueryResult::None` guard preceded
    // by its own `e.halt.is_some()` check; the #791 refactor splits it into
    // `Err((_, EvalEscape::Error(_))) if optional => ...` plus a
    // fall-through `Err((_, escape)) => escape.into()`, so an ordinary
    // (non-halt) error inside a computed index must still be swallowed by
    // `?` exactly as before -- this is the "still works" half of that
    // split, proven against the same `.[(EXPR)] = 1` shape the halt-side
    // contract (`(.[(halt_error(3))] = 1)?` still halts) documents.
    // Verified against jq 1.7.1: `(.[(error("x"))] = 1)?` exits 0 with no
    // output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#"(.[(error("x"))] = 1)?"#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_assign_terminal_halt_keeps_already_built_documents() -> Result<()> {
    // `eval_assign`'s final `terminal` match: a RHS that produces at least
    // one output before halting (`(1, halt_error(3))`) still builds and
    // keeps the document for every output produced before the halt
    // (#400/#494's `Partial` machinery, now extended to `Halt`) instead of
    // discarding it the way the target-side arms in `eval_index_expr`/
    // `eval_slice_expr` above do -- `=` forks per RHS output, and each fork
    // is a fully independent, already-completed document, unlike those
    // functions' single shared target stream. Verified against jq 1.7.1:
    // `{"a":0} | .a = (1, halt_error(3))` prints `{"a":1}`, then exits 3.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", ".a = (1, halt_error(3))"], Some(r#"{"a":0}"#))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\":1}\n");
    Ok(())
}

#[test]
fn test_eval_update_optional_still_swallows_ordinary_path_error() -> Result<()> {
    // Same split as `eval_assign`'s `resolve_dynamic_indexes` match (see
    // `test_eval_assign_optional_still_swallows_ordinary_path_error`), one
    // function over in `eval_update` (`|=`'s own path resolution): an
    // ordinary error inside a computed index must still be swallowed by an
    // outer `?`, even though a halt in the same position never is.
    // Verified against jq 1.7.1: `(.[(error("x"))] |= .+1)?` exits 0 with
    // no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#"(.[(error("x"))] |= .+1)?"#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// Sibling of `test_eval_assign_optional_still_swallows_ordinary_path_error`
/// using a distinct error mechanism: an object used as a computed index
/// (`{}`) is a type error raised while *applying* the resolved key, not
/// while *evaluating* it (`error("x")`'s mechanism) -- both are ordinary,
/// non-halt errors from `resolve_dynamic_indexes`, but the difference
/// matters for patch-coverage: this shape reaches
/// `eval_assign`'s own `Err((_, EvalEscape::Error(_))) if optional`
/// guard, distinct from whatever internal path the `error("x")` shape takes.
/// Verified against jq 1.7.1: `(.[({})] = 1)?` exits 0 with no output.
#[test]
fn test_eval_assign_optional_swallows_type_error_from_object_key() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "(.[({})] = 1)?"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `eval_update` sibling of the test above -- see its doc comment. Verified
/// against jq 1.7.1: `(.[({})] |= .+1)?` exits 0 with no output.
#[test]
fn test_eval_update_optional_swallows_type_error_from_object_key() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "(.[({})] |= .+1)?"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_owned_multi_propagates_partial_halt_from_computed_key() -> Result<()> {
    // `eval_owned_multi`'s `QueryResult::Partial(_, Control::Halt(code))`
    // arm: the computed key (`1, halt`) produces one real output before
    // halting, distinct from a *bare* halt with zero prefix (`.[(halt)]`,
    // already covered) which takes the sibling `QueryResult::Halt(code)` arm
    // right above this one. Verified against jq 1.7.1: `.[(1, halt)] = 1`
    // on `null` exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[(1, halt)] = 1"], Some("null"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_resolve_node_try_catch_runs_handler_as_path_after_ordinary_error() -> Result<()> {
    // `resolve_node`'s `Expr::Try` arm, `Some(catch_expr)` branch: when the
    // `try` body fails with a genuine (non-halt, non-invalid-path-expression)
    // error partway through producing its own path outputs, the `catch`
    // handler runs as a path expression too, against the error's payload,
    // and its resolved paths are appended after whatever the `try` body
    // already resolved before failing. `.a` succeeds first (contributing
    // `["a"]`), then `.x[0]` fails indexing the number `5` with a number;
    // `catch empty` contributes nothing further. Verified against jq 1.7.1:
    // `path(try (.a, .x[0]) catch empty)` on `{"a":1,"x":5}` is `["a"]`.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "path(try (.a, .x[0]) catch empty)"],
        Some(r#"{"a":1,"x":5}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    Ok(())
}

#[test]
fn test_update_path_index_arm_reports_type_error_on_non_array() -> Result<()> {
    // `update_path`'s bare `Expr::Index(idx)` arm (reached when the whole
    // resolved path is a single index component, e.g. `.[0]`): indexing a
    // non-array, non-null root with `optional: false` reports jq's
    // ordinary "cannot index" error. This is plain pre-existing type
    // checking, unrelated to halt propagation -- the source line only
    // moved because `update_path`'s return type changed from
    // `Result<(), EvalError>` to `Result<(), EvalEscape>` for #791, so
    // every ordinary error return needed `.into()` added. Verified against
    // jq 1.7.1: `"abc" | .[0] |= .+1` exits 5 with "Cannot index string
    // with number".
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[0] |= .+1"], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot index string with number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_update_path_iterate_arm_reports_type_error_on_non_iterable() -> Result<()> {
    // `update_path`'s bare `Expr::Iterate` arm (the whole resolved path is
    // `.[]`): iterating a scalar root with `optional: false` reports jq's
    // ordinary "cannot iterate" error -- same "line moved for the
    // `.into()` conversion, not new behavior" story as the `Index` arm
    // test above. Verified against jq 1.7.1: `5 | .[] |= .+1` exits 5 with
    // "Cannot iterate over number (5)".
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[] |= .+1"], Some("5"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot iterate over number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_update_path_pipe_field_arm_reports_type_error_on_non_object() -> Result<()> {
    // `update_path`'s `Expr::Pipe(exprs)` branch has its own copy of the
    // `Expr::Field` type check for a *non-last* path component (the whole
    // resolved path here is `.a.b`, two components, so `first =
    // Field("a")` is checked against the root *before* recursing into
    // `rest`) -- distinct from the top-level bare `Expr::Field` arm, which
    // only ever sees a single-component path. Verified against jq 1.7.1:
    // `5 | .a.b |= .+1` exits 5 with `Cannot index number with string
    // "a"`.
    let (stdout, stderr, code) = run_jq_full(&["-c", ".a.b |= .+1"], Some("5"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains(r#"Cannot index number with string "a""#),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_update_path_pipe_index_arm_reports_type_error_on_non_array() -> Result<()> {
    // Same shape as the `Pipe`-branch `Field` arm test above, one arm over:
    // the whole resolved path here is `.[0][1]`, two components, so
    // `first = Index(0)` is checked against the (non-array) root before
    // recursing into `rest`. Verified against jq 1.7.1:
    // `"abc" | .[0][1] |= .+1` exits 5 with "Cannot index string with
    // number".
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[0][1] |= .+1"], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot index string with number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_update_path_pipe_iterate_arm_reports_type_error_on_non_iterable() -> Result<()> {
    // Same shape again, for `Expr::Iterate` inside the `Pipe` branch: the
    // whole resolved path here is `.[][0]`, two components, so
    // `first = Iterate` is checked against the (non-iterable) root before
    // recursing into `rest`. Verified against jq 1.7.1:
    // `5 | .[][0] |= .+1` exits 5 with "Cannot iterate over number (5)".
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[][0] |= .+1"], Some("5"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot iterate over number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_path_as_binding_propagates_halt_from_partial_bind_stream() -> Result<()> {
    // `resolve_node`'s `Expr::As` arm (`path()`'s own `E as $x | body`
    // handling), its `Some(Control::Halt(code))` trailing-control arm: the
    // bind expression produces one output and then halts. This used to be
    // impossible to see at all -- the arm's predecessor eagerly collected
    // the *whole* bind stream via `eval_owned_multi` before running `body`
    // even once, so a bind source that halts partway through discarded any
    // prefix and never bound `$x` for the earlier, successful output (#791).
    // Verified against jq 1.7.1: `path((1, halt_error(3)) as $x | .a)` on
    // `{"a":1}` prints `["a"]` (the $x=1 iteration's own body completing)
    // to stdout, dumps the still-unmodified input `{"a":1}` to stderr (bare
    // `halt_error`'s current-value dump), and exits 3 -- it never attempts a
    // second iteration for the failed bind.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "path((1, halt_error(3)) as $x | .a)"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    Ok(())
}

#[test]
fn test_path_as_binding_propagates_error_from_partial_bind_stream() -> Result<()> {
    // Sibling of the halt test above, `Expr::As`'s `Some(Control::Error(e))`
    // trailing-control arm instead of its `Halt` arm: the bind source
    // produces one output, `body` resolves it, and only the *second*
    // attempt to advance the bind source raises an ordinary error --
    // the already-resolved prefix from the first binding must still reach
    // the caller. Verified against jq 1.7.1: `path((1, error("boom")) as $x
    // | .a)` on `{"a":1}` prints `["a"]`, then errors "boom", exit 5.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path((1, error("boom")) as $x | .a)"#],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains("boom"), "{stderr}");
    Ok(())
}

#[test]
fn test_path_as_binding_break_in_bind_stream_reaches_an_outer_label() -> Result<()> {
    // `Expr::As`'s `Some(Control::Break(label))` trailing-control arm,
    // reached when the bind source itself raises a labeled `break` after
    // already producing an output. Before #824, `resolve_node`'s error type
    // (`EvalEscape`, threaded through `PathResolveResult`) had no way to
    // carry a `Control::Break`, so every site that received one -- this arm
    // and `eval_owned_multi` alike -- had no choice but to fold it into a
    // synthetic "break $label not in label" `EvalError`, even when a
    // `label` with that exact name was sitting right outside the
    // `path(...)` call, fully able to catch it. `EvalEscape::Break` now
    // carries the label through instead, so `label $out`, sitting outside
    // this `path(...)` call, catches it cleanly: verified against jq 1.7.1,
    // `label $out | path((1, break $out) as $x | .a)` on `{"a":1}` exits 0
    // with `["a"]` (the `$x=1` iteration's own `body` resolution completing
    // before the break), not an error.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "label $out | path((1, break $out) as $x | .a)"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_break_across_a_path_call_boundary_reaches_an_outer_label_824() -> Result<()> {
    // Issue #824's own first repro: a `break $label` raised inside a
    // `path(...)` call's argument couldn't reach a `label $label` sitting
    // *outside* that call -- it degraded into a bogus "break $out not in
    // label" error (exit 5) instead of unwinding cleanly, even though the
    // output already resolved before the break (`["a"]`) still reached
    // stdout first. Verified against jq 1.7.1. The issue's second repro,
    // through an `as`-binding's bind source, is covered by
    // `test_path_as_binding_break_in_bind_stream_reaches_an_outer_label`
    // above rather than duplicated here.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "label $out | path((.a, break $out))"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_break_inside_a_path_call_is_caught_by_a_label_inside_it_824() -> Result<()> {
    // The mirror image of the boundary-crossing case above: a `label`
    // sitting *inside* `path(...)` catches its own matching `break` right
    // there -- `path(...)` still resolves normally, and (unlike a break
    // that escapes `path(...)` entirely) evaluation continues afterward
    // rather than unwinding any further. Verified against jq 1.7.1:
    // `path(label $out | (.a, break $out))` is `["a"]`, and following it
    // with another expression in the same enclosing label still reaches
    // that expression.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "path(label $out | (.a, break $out))"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (path(label $out | (.a, break $out)), "after")"#,
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n\"after\"\n");
    assert_eq!(stderr, "");

    // A *non-matching* break (still meant for something further out) is not
    // this `label`'s to catch, so it propagates past it unchanged -- here as
    // a plain error unrelated to any break at all, exercising the same
    // catch-all fallthrough. Verified against jq 1.7.1: `path(label $out |
    // error("boom"))` raises "boom", exit 5 (the `label` plays no role when
    // there's no break to catch).
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(label $out | error("boom"))"#],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("boom"), "{stderr}");
    Ok(())
}

#[test]
fn test_try_catch_inside_a_path_call_catches_a_break_regardless_of_label_824() -> Result<()> {
    // `try`/`catch` (and bare `?`, sugar for a catch-less `try`) intercept a
    // `break` passing through them unconditionally, regardless of which
    // label it targets -- the same rule the value-position evaluator's
    // `eval_try` already applies (#562), now mirrored in path context
    // (#824). `catch empty` here (rather than a handler that is itself not
    // a valid path expression, e.g. `catch "x"`) is deliberate regardless:
    // it isolates *this* test's break-interception claim from the catch
    // handler's own success/failure, which is covered separately below and
    // by the dedicated #832 regression tests
    // (`test_path_try_catch_handler_error_keeps_prefix_832` and
    // `test_path_try_catch_handler_halt_keeps_prefix_832`) -- #832 fixed
    // `resolve_catch` (shared by this arm's `Error` and `Break` cases) so
    // the resolved prefix survives even when the handler itself then fails,
    // for all three escape kinds (error, break, halt).
    // Verified against jq 1.7.1: `path(try (.a, break $out) catch empty)`
    // on `{"a":1}` is `["a"]`, exit 0 -- the break never reaches `$out` at
    // all, catch runs before the label ever gets a chance.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "label $out | path(try (.a, break $out) catch empty)"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "");

    // Bare `?` (no catch) prunes the break the same way, keeping just the
    // prefix: `path((.a, break $out)?)` on `{"a":1}` is `["a"]`, exit 0.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "label $out | path((.a, break $out)?)"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "");

    // `try EXPR` with *no* `catch` clause at all (distinct from bare `?`
    // above at the AST level, `Expr::Try { catch: None, .. }` rather than
    // `Expr::Optional`) catches the break the same unconditional way.
    // Verified against jq 1.7.1: `path(try (.a, break $out))` on `{"a":1}`
    // is `["a"]`, exit 0.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "label $out | path(try (.a, break $out))"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "");

    // The catch handler itself raising a *different* error, rather than
    // failing as a path expression (#530) — the same "catch runs, and its
    // own failure propagates" shape, through the code path #832 fixed
    // (`resolve_catch`, shared by this arm's `Error` and `Break` cases):
    // confirmed live, real jq's `path(try (.a, break $out) catch
    // error("y"))` on `{"a":1}` prints the prefix `["a"]` then raises "y",
    // exit 5. Before #832, succinctly lost the prefix here (empty stdout)
    // while still raising "y" and exiting 5 -- the handler's own failure was
    // reported correctly, only the earlier prefix was dropped. `resolve_catch`
    // now threads `prefix` into the `Err` side the same way every other arm
    // in this resolver already does, so the prefix survives regardless of
    // which escape kind (error, break, or halt) the handler itself raises --
    // see #832's dedicated regression tests below for the ordinary-error and
    // halt cases.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | path(try (.a, break $out) catch error("y"))"#,
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains("error (at <stdin>:"), "{stderr}");
    assert!(stderr.trim_end().ends_with(": y"), "{stderr}");
    Ok(())
}

#[test]
fn test_path_try_catch_handler_error_keeps_prefix_832() -> Result<()> {
    // #832: `resolve_catch` (the helper `resolve_node`'s `Expr::Try` arm
    // shares between its `Error` and `Break` cases) used to return the catch
    // handler's own failure via a bare `?`, discarding whatever `prefix` the
    // failed `try` body had already resolved. Ordinary-error variant:
    // `catch_expr` itself is not a valid path expression (#530). Verified
    // against jq 1.7.1: `path(try (.a, .x[0]) catch "x")` on
    // `{"a":1,"x":5}` prints the prefix `["a"]` (the `.a` branch, resolved
    // before `.x[0]` failed) then raises the #530 "Invalid path expression"
    // complaint about `"x"`, exit 5.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(try (.a, .x[0]) catch "x")"#],
        Some(r#"{"a":1,"x":5}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains("Invalid path expression"), "{stderr}");
    Ok(())
}

#[test]
fn test_path_try_catch_handler_halt_keeps_prefix_832() -> Result<()> {
    // #832, halt variant: the catch handler itself calls `halt_error`, which
    // is never caught by `try`/`catch` (#791) and exits the whole process.
    // `resolve_catch`'s fix is escape-kind-agnostic (it matches on the
    // handler's `Err((prefix, escape))` generically), so this needs its own
    // regression case distinct from the ordinary-error and break variants
    // above/below -- confirmed independently rather than assumed from the
    // other two. The prefix `["a"]` (resolved before `.x[0]` failed) must
    // still reach stdout before the process halts.
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", "path(try (.a, .x[0]) catch halt_error)"],
        Some(r#"{"a":1,"x":5}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    Ok(())
}

#[test]
fn test_limit_n_bound_break_inside_a_path_call_reaches_an_outer_label_824() -> Result<()> {
    // `resolve_limit`'s own `n_expr` evaluation (`limit(n; f)`'s count) sits
    // directly in `resolve_node`'s domain, adjacent to every other arm this
    // PR fixed, so it was switched from `eval_owned_expr` (which still
    // collapses a break into a synthetic "not in label" error, same as
    // #833's broader, out-of-scope call sites) to `eval_owned_expr_ctrl`
    // (which preserves `Control` losslessly), rather than leaving this one
    // adjacent gap unfixed. Verified against jq 1.7.1: `path(limit(break
    // $out; .a))` on `{"a":1}` produces no output, exit 0.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "label $out | path(limit(break $out; .a))"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_path_as_binding_keeps_earlier_binding_prefix_when_a_later_body_errors() -> Result<()> {
    // `Expr::As`'s per-binding loop body, the `Err((body_prefix, escape))`
    // arm: the bind source itself succeeds fully (no error/break/halt), but
    // a *later* binding's `body` resolution fails as a path expression.
    // Whatever `out` accumulated from earlier, successful bindings must
    // still reach the caller ahead of the failure, not just `body_prefix`
    // from the failing iteration alone. Verified against jq 1.7.1:
    // `path((1,2) as $x | if $x==1 then .a else ($x|.a) end)` on `{"a":1}`
    // prints `["a"]` (the `$x=1` iteration), then raises an "invalid path
    // expression" error for the `$x=2` iteration (indexing a number as a
    // path) and exits 5 -- the exact wording differs from jq's own (a
    // pre-existing, accepted divergence documented on `Expr::Try` above),
    // but the exit code and the kept prefix both match.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "path((1,2) as $x | if $x==1 then .a else ($x|.a) end)",
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains("Invalid path expression"), "{stderr}");
    Ok(())
}

#[test]
fn test_update_assign_propagates_halt_from_bare_filter() -> Result<()> {
    // `eval_owned_multi_first`'s bare `QueryResult::Halt(code)` arm --
    // `update_path`'s `Expr::Identity` arm (`|=`'s own update-filter
    // evaluation) uses this function specifically because it must keep only
    // the update filter's *first* output. Unlike a `Partial`'s trailing
    // control (deliberately kept as "just the prefix" per this function's
    // own doc comment), a *bare* halt has no prior output to fall back to --
    // `collect_owned()` would turn it into `Ok(vec![])`, read back as an
    // empty filter and silently assigning `null` instead of halting (#791).
    // This exercises the direct filter path (no `try`/`catch` wrapper),
    // distinct from the existing `try halt_error(9) catch empty` regression
    // test. Verified against jq 1.7.1: `{"a":1} | .a |= halt_error(7)`
    // prints nothing to stdout, dumps `1` (the pre-update `.a`) to stderr,
    // and exits 7.
    let (stdout, stderr, code) = run_jq_full(&["-c", ".a |= halt_error(7)"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_resolve_node_try_without_catch_keeps_prefix_on_ordinary_error() -> Result<()> {
    // `resolve_node`'s `Expr::Try` arm, the genuine-error (non-halt) branch
    // guarded by `!e.is_invalid_path_expression()`: with no `catch`, the
    // path components already resolved before the failure are kept, the
    // same policy `?`'s matching arm uses. This is the positive-path sibling
    // of `test_halt_not_caught_by_try_catch_in_path_expression`, which only
    // exercises the case where the escape *is* a halt and this whole `match
    // catch` block is never entered. Verified against jq 1.7.1:
    // `jq -c 'path(try (.a, .x[0]))'` on `{"a":1,"x":5}` (`.x[0]` errors --
    // 5 is a number, not indexable) prints `["a"]`, exit 0.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path(try (.a, .x[0]))"], Some(r#"{"a":1,"x":5}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    Ok(())
}

#[test]
fn test_resolve_node_try_catch_handles_ordinary_error() -> Result<()> {
    // `resolve_node`'s `Expr::Try` arm, the `Some(catch_expr)` branch: a
    // genuine (non-halt) error resolved by `try` runs the `catch` handler as
    // a path expression too, via `resolve_against_cow` (the error payload is
    // a fresh, function-local `OwnedValue`, so it can't lend a reference with
    // this function's own `'a`). Verified against jq 1.7.1:
    // `jq -c 'path(try (.a, .x[0]) catch empty)'` on `{"a":1,"x":5}` prints
    // `["a"]`, exit 0 -- the caught error's `catch empty` handler resolves
    // to zero path components, contributing nothing after `.a`'s own.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "path(try (.a, .x[0]) catch empty)"],
        Some(r#"{"a":1,"x":5}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    Ok(())
}

#[test]
fn test_resolve_limit_path_context_rejects_non_numeric_n() -> Result<()> {
    // `resolve_limit`'s wildcard arm (`path(limit(...))`'s own `n`-argument
    // check, the path-tracking sibling of `eval_limit`'s equivalent check):
    // any `n` that isn't a non-negative int reports "limit requires
    // non-negative integer" rather than silently coercing. Real jq's
    // `limit/2` is defined via `foreach`/arithmetic, so a non-numeric `n`
    // fails there with its own "cannot be subtracted" wording instead --
    // different text, but the same "reject, don't silently misbehave" shape:
    // both exit non-zero. Verified against jq 1.7.1:
    // `jq -c 'path(limit("x"; .a))'` on `{"a":1}` exits 5.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path(limit(\"x\"; .a))"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("limit requires non-negative integer"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_as_binding_propagates_halt_from_bind_expression() -> Result<()> {
    // `eval_as`'s `bound_result` match: a halt while evaluating the bind
    // expression itself (`EXPR as $x | body`'s `EXPR`), with zero prior
    // output, must return `QueryResult::Halt` directly rather than being
    // read as "no bound values" and silently running `body` zero times with
    // exit 0. Verified against jq 1.7.1: `jq -n 'halt_error(3) as $x | $x'`
    // exits 3 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "halt_error(3) as $x | $x"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_as_binding_propagates_halt_from_body_after_partial_output() -> Result<()> {
    // `eval_as`'s per-bound-value loop match: once bound values are being
    // iterated, a halt raised while evaluating `body` for a later bound
    // value must still surface, carrying forward whatever `all_results` the
    // earlier bound values already produced (#400/#494's "outputs already
    // produced no longer vanish" policy applied to `as`). Verified against
    // jq 1.7.1: `jq -n '(1,2) as $x | if $x==2 then halt_error(4) else $x
    // end'` prints `1` then exits 4 -- the `$x=2` iteration halts before
    // producing its own output.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "(1,2) as $x | if $x==2 then halt_error(4) else $x end",
        ],
        None,
    )?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_reduce_propagates_halt_from_input_stream() -> Result<()> {
    // `eval_reduce`'s `input_result` match, bare `QueryResult::Halt` arm: a
    // halt while evaluating `reduce EXPR as $var (...)`'s source stream,
    // with zero prior input values, must escape as a halt rather than being
    // read as "empty input", which would still run the accumulator's INIT
    // and answer normally. Verified against jq 1.7.1:
    // `jq -n 'reduce halt_error(5) as $x (0; .+$x)'` exits 5 with no output.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "reduce halt_error(5) as $x (0; .+$x)"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_reduce_propagates_halt_from_partial_input_stream() -> Result<()> {
    // `eval_reduce`'s `input_result` match, `QueryResult::Partial(_,
    // Control::Halt(code))` arm: `reduce`'s source stream produced a value
    // and then halted. `reduce`'s own output is always single-shot (only the
    // final accumulator, never intermediates), so the partial prefix is
    // dropped and only the halt escapes -- verified against jq 1.7.1:
    // `jq -n 'reduce (1, halt_error(6)) as $x (0; .+$x)'` exits 6 with no
    // output at all, not the accumulator computed from the single `1`.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "reduce (1, halt_error(6)) as $x (0; .+$x)"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_reduce_propagates_halt_from_init_expression() -> Result<()> {
    // `eval_reduce`'s `init_result` match, bare `QueryResult::Halt` arm: a
    // halt while evaluating INIT (with zero prior INIT outputs) must escape
    // directly -- INIT forks the whole reduce over each of its own outputs
    // (#534), and a bare halt here means there is no fork to run at all.
    // Verified against jq 1.7.1: `jq -n 'reduce 1 as $x (halt_error(7);
    // .+$x)'` exits 7 with no output.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "reduce 1 as $x (halt_error(7); .+$x)"], None)?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_parentn_n_expr_break_reported_as_break_not_in_label() -> Result<()> {
    // `eval_owned_expr`'s `Control::Break` arm (split out from `Error`
    // alongside the new `Control::Halt` arm one line below it, forced by
    // `Control` gaining the `Halt` variant): an unmatched `break` raised
    // while evaluating an owned-value-based argument -- here `parent(n)`'s
    // `n` argument, via `ParentN`'s call to `eval_owned_expr` -- converts
    // into the ordinary "break $label not in label" `EvalError`, the same
    // diagnostic every other uncaught break in this file produces. `parent`
    // is a succinctly extension (no real-jq equivalent), so this is checked
    // against succinctly's own established "break $out not in label" wording
    // (see `test_uncaught_break_after_output_keeps_the_prefix`), not jq.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", ".a.b | parent(break $out)"],
        Some(r#"{"a":{"b":1}}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("break $out not in label"), "{stderr}");
    Ok(())
}

#[test]
fn test_foreach_propagates_halt_from_input_stream() -> Result<()> {
    // `eval_foreach`'s `input_result` match, bare `QueryResult::Halt` arm:
    // same shape as `eval_reduce`'s equivalent arm -- a halt while evaluating
    // `foreach`'s source stream, with zero prior input values, must escape
    // directly. Verified against jq 1.7.1:
    // `jq -n 'foreach halt_error(8) as $x (0; .+$x)'` exits 8 with no output.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "foreach halt_error(8) as $x (0; .+$x)"], None)?;
    assert_eq!(code, 8, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_foreach_propagates_halt_from_init_expression() -> Result<()> {
    // `eval_foreach`'s `init_result` match, bare `QueryResult::Halt` arm:
    // same shape as `eval_reduce`'s equivalent arm -- INIT forks the whole
    // foreach over each of its own outputs (#534), and a bare halt here
    // means there is no fork to run. Verified against jq 1.7.1:
    // `jq -n 'foreach 1 as $x (halt_error(9); .+$x)'` exits 9 with no output.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "foreach 1 as $x (halt_error(9); .+$x)"], None)?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_limit_propagates_halt_in_n_argument() -> Result<()> {
    // `eval_limit`'s `n_result` match, `Err(e) => return e.into()` arm: a
    // halt while evaluating `limit(n; expr)`'s `n` argument must escape
    // rather than being reported as "limit requires non-negative integer".
    // Verified against jq 1.7.1: `jq -n 'limit(halt_error(11); 1,2,3)'`
    // exits 11 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "limit(halt_error(11); 1,2,3)"], None)?;
    assert_eq!(code, 11, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_limit_propagates_halt_in_expr_argument() -> Result<()> {
    // `eval_limit`'s main result match, `QueryResult::Halt(code)` arm: a
    // halt used to be reachable only via the old wildcard alongside `Break`
    // (fixed alongside #494's "limit drops what it shouldn't" family) --
    // a bare halt from `expr` (no prior outputs) must escape as `Halt`, not
    // be dropped the way excess values past `n` are. Verified against jq
    // 1.7.1: `jq -n 'limit(3; halt_error(12))'` exits 12 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "limit(3; halt_error(12))"], None)?;
    assert_eq!(code, 12, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_first_expr_propagates_halt() -> Result<()> {
    // `eval_first_expr`'s result match, `QueryResult::Halt(code)` arm: a
    // bare halt from `expr` (no prior output to take as "first") must
    // escape as `Halt`, not be read as `None`. Verified against jq 1.7.1:
    // `jq -n 'first(halt_error(13))'` exits 13 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "first(halt_error(13))"], None)?;
    assert_eq!(code, 13, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_last_expr_propagates_bare_halt() -> Result<()> {
    // `eval_last_expr`'s result match, bare `QueryResult::Halt(code)` arm: a
    // halt from `expr` with zero prior outputs must escape directly.
    // Verified against jq 1.7.1: `jq -n 'last(halt_error(14))'` exits 14
    // with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "last(halt_error(14))"], None)?;
    assert_eq!(code, 14, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_last_expr_propagates_halt_past_partial_prefix() -> Result<()> {
    // `eval_last_expr`'s result match, `QueryResult::Partial(_,
    // Control::Halt(code))` arm: unlike `first`, `last` cannot short-circuit
    // -- it doesn't know a value is the last until the stream is exhausted
    // -- so a `Partial` prefix (here `[1]`, produced before the halt) is
    // dropped and only the halt surfaces. Verified against jq 1.7.1:
    // `jq -n 'last(1, halt_error(15))'` exits 15 with no output -- it does
    // not answer `1`.
    let (stdout, stderr, code) = run_jq_full(&["-n", "last(1, halt_error(15))"], None)?;
    assert_eq!(code, 15, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// The three tests below are `eval.rs`-specific siblings of
/// `test_first_expr_propagates_halt`/`test_last_expr_propagates_bare_halt`/
/// `test_last_expr_propagates_halt_past_partial_prefix` above: those three
/// use `first`/`last` as the *top-level* CLI filter, which
/// `eval_generic.rs`'s own native `Expr::FirstExpr`/`Expr::LastExpr` handling
/// (`eval_first_or_last_generic`, added for #607) intercepts before
/// `eval.rs`'s `eval_first_expr`/`eval_last_expr` are ever reached. Routing
/// through a `group_by` key function (as with the `eval_pipe`/`eval_index_expr`
/// tests earlier in this file) forces evaluation through `eval.rs`'s own
/// implementations instead.
#[test]
fn test_eval_first_expr_bare_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_first_expr`'s own `QueryResult::Halt(code)` arm. Verified against
    // jq 1.7.1: `jq -c 'group_by(first(halt))'` on `[5]` exits 0 with no
    // output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(first(halt))"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_last_expr_bare_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_last_expr`'s own bare `QueryResult::Halt(code)` arm. Verified
    // against jq 1.7.1: `jq -c 'group_by(last(halt))'` on `[5]` exits 0 with
    // no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(last(halt))"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_eval_last_expr_partial_halt_reached_through_group_by_key_fn() -> Result<()> {
    // `eval_last_expr`'s own `QueryResult::Partial(_, Control::Halt(code))`
    // arm: `last` cannot short-circuit, so a prefix (`1`, `2`) produced
    // before the halt is dropped, and only the halt surfaces. Verified
    // against jq 1.7.1: `jq -c 'group_by(last((1, 2, halt)))'` on `[5]`
    // exits 0 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-c", "group_by(last((1, 2, halt)))"], Some("[5]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_range_bound_error_is_caught_by_try_catch() -> Result<()> {
    // `range_arg`'s `QueryResult::Error(e) => Err(e.into())` arm -- the
    // ordinary-error sibling of the `Halt` arm covered by
    // `test_range_halt_not_caught_by_try_catch`. A genuine (non-halt) error
    // in a range bound is fully catchable, unlike a halt. Verified against
    // jq 1.7.1: `jq -n 'try (range(error("boom"))) catch .'` is `"boom"`,
    // exit 0.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "try (range(error(\"boom\"))) catch ."], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"boom\"\n");
    Ok(())
}

#[test]
fn test_range_from_bound_partial_halt_aborts_immediately() -> Result<()> {
    // `range_arg`'s `QueryResult::Partial(_, Control::Halt(code))` arm,
    // reached via `eval_range`'s `from_val` computation (`Err(e) => return
    // e.into()`): the `from` bound produces one output and then halts. This
    // is a succinctly-only shape to pin: real jq's `range/2` fans out over
    // every output of a multi-valued bound (confirmed live --
    // `jq -n 'range((1, halt_error(4)); 10)'` prints `1` through `9` from
    // the first `from` value's whole `range(1;10)` before halting on the
    // second), whereas this evaluator's `from`/`to`/`step` each take exactly
    // one resolved value and never fork. So here the halt is caught before
    // `eval_range` ever calls `eval_range_values`: no range values are
    // generated at all, unlike jq's partial run. Succinctly's own contract
    // (halt is never downgraded, never partially computed around) still
    // holds: no output, exit 4.
    let (stdout, stderr, code) = run_jq_full(&["-n", "range((1, halt_error(4)); 10)"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_range_bound_non_numeric_value_rejected() -> Result<()> {
    // `range_arg`'s trailing wildcard arm ("Range bounds must be numeric"):
    // a single-argument `range(N)` whose bound evaluates to a non-numeric
    // `OwnedValue` (here a plain string literal, which reaches this
    // function as `QueryResult::Owned(OwnedValue::String(..))` -- none of
    // the numeric `Owned`/`One` arms above it) is rejected outright rather
    // than silently coerced. Verified against jq 1.7.1: `jq -n 'range("x")'`
    // exits 5 with the identical "Range bounds must be numeric" message.
    let (stdout, stderr, code) = run_jq_full(&["-n", "range(\"x\")"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Range bounds must be numeric"), "{stderr}");
    Ok(())
}

#[test]
fn test_range_step_bound_propagates_halt() -> Result<()> {
    // `eval_range`'s `step_val` computation, `Err(e) => return e.into()`
    // arm: a halt while evaluating `range(a;b;step)`'s `step` argument must
    // escape before any range values are generated. Verified against jq
    // 1.7.1: `jq -n 'range(1; 10; halt_error(6))'` exits 6 with no output --
    // the step is resolved once, up front, so (unlike `from`'s fan-out
    // divergence) this one matches jq exactly.
    let (stdout, stderr, code) = run_jq_full(&["-n", "range(1; 10; halt_error(6))"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_recurse_cond_propagates_halt_from_f() -> Result<()> {
    // `builtin_recurse_cond`'s `f`-evaluation match, `Err(e) => return
    // partial(outputs, e.into())` arm: an error/halt from `f` aborts
    // immediately rather than pruning just the current node's children
    // (#636's "nothing catches an error from `f`" rule extended to halt).
    // The root is still emitted first (`recurse`'s own `.,` output happens
    // before `f` is evaluated), so this exercises the halt carrying that
    // one-element `outputs` prefix forward as a `Partial`. Verified against
    // jq 1.7.1: `jq -n '1 | recurse(halt_error(7); true)'` prints `1` to
    // stdout, dumps `1` (the current value at the point of the halt) to
    // stderr, and exits 7.
    let (stdout, stderr, code) = run_jq_full(&["-n", "1 | recurse(halt_error(7); true)"], None)?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_recurse_f_keeps_own_partial_fanout_before_error_842() -> Result<()> {
    // Issue #842's primary repro (value position). `f`'s own fan-out
    // (`.a, .b[0]`) is a generator like any other comma expression: a
    // later output of that same call erroring must not retroactively
    // un-emit an earlier one it already produced. Before this fix,
    // `resolve_recurse`/`builtin_recurse_f` dropped `f`'s own partial
    // fan-out on error, matching neither jq's semantics nor the codebase's
    // existing "never un-emit an already-produced output" rule (#530,
    // #636, #694, #824). Verified against jq 1.7.1:
    // `echo '{"a":1,"b":2}' | jq -c 'recurse(if . == {"a":1,"b":2} then
    // (.a, .b[0]) else empty end)'` prints `{"a":1,"b":2}` and `1` to
    // stdout before erroring, and exits 5.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"recurse(if . == {"a":1,"b":2} then (.a, .b[0]) else empty end)"#,
        ],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\":1,\"b\":2}\n1\n");
    assert!(
        stderr.contains("Cannot index number with number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_resolve_recurse_keeps_f_partial_fanout_before_error_842() -> Result<()> {
    // Issue #842's secondary repro (path position, `resolve_recurse`).
    // Same underlying bug as the value-position test above, in the
    // path-tracking evaluator `path(...)` uses. Verified against jq 1.7.1:
    // `echo '{"a":1,"b":2}' | jq -c 'path(recurse(if . == {"a":1,"b":2}
    // then (.a, .b[0]) else empty end))'` prints `[]` and `["a"]` to
    // stdout before erroring, and exits 5.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(recurse(if . == {"a":1,"b":2} then (.a, .b[0]) else empty end))"#,
        ],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n[\"a\"]\n");
    assert!(
        stderr.contains("Cannot index number with number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// #856: `resolve_recurse`'s null-child guard (`if matches!(child_value...,
/// OwnedValue::Null) { continue; }` in its main loop) stopped recursion
/// *into* a null child, as documented -- but the bare `continue` also
/// discarded the null child's own path entirely, rather than still
/// emitting it as a leaf the way `builtin_recurse_f`/`builtin_recurse_cond`
/// (value position) correctly do. Verified against jq 1.7.1:
/// `{"a":null} | path(recurse(if . == {"a":null} then .a else empty
/// end))` prints `[]` (root) *and* `["a"]` (the null child, emitted as a
/// leaf since its own recursion into `f` produces nothing further).
#[test]
fn test_resolve_recurse_emits_null_childs_own_path() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(recurse(if . == {"a":null} then .a else empty end))"#,
        ],
        Some(r#"{"a":null}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n[\"a\"]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Companion to the test above: a null child emitted as a leaf must not
/// jump ahead of an *earlier* sibling's own subtree just because it has no
/// descendants of its own. `resolve_recurse` gates the null-recursion bound
/// at the point a node is popped from its DFS stack (the same point every
/// other node is emitted), not at child-collection time, specifically so a
/// null child queued alongside a non-null sibling still waits its correct
/// turn. Verified against jq 1.7.1: `{"a":null,"b":{"x":1,"y":2}} |
/// path(recurse(if (.==null) then empty elif (type=="object") then .[]
/// else empty end))` streams `[]`, `["a"]`, `["b"]`, `["b","x"]`,
/// `["b","y"]` in that order -- the null leaf right after the root, before
/// `b`'s own subtree, not interleaved with or after it.
#[test]
fn test_resolve_recurse_null_child_does_not_disturb_sibling_order() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(recurse(if (.==null) then empty elif (type=="object") then .[] else empty end))"#,
        ],
        Some(r#"{"a":null,"b":{"x":1,"y":2}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(
        stdout,
        "[]\n[\"a\"]\n[\"b\"]\n[\"b\",\"x\"]\n[\"b\",\"y\"]\n"
    );
    assert_eq!(stderr, "");
    Ok(())
}

/// Companion to the two tests above, for `recurse(f; cond)`: `cond` still
/// gates whether a null child is emitted *at all* -- the null-recursion
/// bound only decides whether a null child that already passed `cond`
/// gets recursed into further, not whether `cond` itself applies to it.
/// Verified against jq 1.7.1: `{"a":null} | path(recurse(.a;
/// type=="object"))` prints only `[]` (root) and exits 0 -- the null
/// child never appears, since `type=="object"` rejects it before it would
/// ever be emitted (real jq's `paths(node_filter)` = `path(recurse|select(
/// node_filter))`, and a `select`-rejected child is never even reached).
#[test]
fn test_resolve_recurse_cond_still_gates_null_child_emission() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(recurse(.a; type=="object"))"#],
        Some(r#"{"a":null}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Review-driven regression guard for the fix above: bounding recursion
/// *into* a null node must not also skip *evaluating* `f` on it -- real
/// jq's `recurse` always applies `f` to every node it visits, root
/// included, so an `f` that errors on a null node (e.g. `.[]`, "Cannot
/// iterate over null") must still abort the whole call, exactly like an
/// `f` error on any other node. An earlier version of this fix skipped
/// `resolve_against_cow` entirely whenever the popped node was null,
/// silently swallowing that error instead of propagating it -- for the
/// DFS-seed (root) value specifically, since a null root is seeded
/// directly onto the stack without going through the per-child collection
/// loop at all. Verified against jq 1.7.1: `null | path(recurse(.[]))`
/// raises "Cannot iterate over null (null)" and exits 5, printing `[]`
/// (the root's own path) first.
#[test]
fn test_resolve_recurse_null_root_still_evaluates_f_and_propagates_its_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "null | path(recurse(.[]))"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert_eq!(
        stderr,
        "jq: error (at <unknown>): Cannot iterate over null (null)\n"
    );
    Ok(())
}

/// Same regression guard as above, but for a null *child* rather than the
/// root: `f` must still be evaluated on a null child that was correctly
/// queued and emitted (per the fix's primary #856 repro), not just on a
/// null root. Verified against jq 1.7.1: `{"a":null} |
/// path(recurse(.[]))` streams `[]` then `["a"]` before raising "Cannot
/// iterate over null (null)" and exiting 5 -- the error surfaces only
/// once the traversal actually reaches the null child, not before.
#[test]
fn test_resolve_recurse_null_child_still_evaluates_f_and_propagates_its_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "path(recurse(.[]))"], Some(r#"{"a":null}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n[\"a\"]\n");
    assert_eq!(
        stderr,
        "jq: error (at <stdin>:0): Cannot iterate over null (null)\n"
    );
    Ok(())
}

/// Second review-driven regression guard: an earlier version of the fix
/// above evaluated `f` unconditionally on a null `current` (correctly
/// propagating its error) but left `cond`'s evaluation nested inside the
/// same `is_null_current` gate as the "queue for further recursion"
/// decision -- so `cond`'s own error on a candidate child produced from an
/// already-null `current` was still silently swallowed. This is a
/// pre-existing gap (confirmed identical on `main` before #856 for the
/// null-root case specifically, since a non-root value could never reach
/// `current` while null before this fix), but #856's own restructuring
/// widened its reach from "root only" to "any null node encountered
/// during traversal" -- so it's fixed here rather than deferred. Verified
/// against jq 1.7.1: `null | path(recurse(.a; error("boom")))` streams
/// `[]` (the root's own path) then raises "boom" and exits 5.
#[test]
fn test_resolve_recurse_null_root_still_evaluates_cond_and_propagates_its_error() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "null | path(recurse(.a; error(\"boom\")))"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "jq: error (at <unknown>): boom\n");
    Ok(())
}

/// Companion to the test above, for the *widened* reach specifically: a
/// null value reached partway through the traversal (not the root) must
/// also still evaluate `cond` on its own candidate children. Verified
/// against jq 1.7.1: `{"a":null} | path(recurse(.a; error("boom")))`
/// streams `[]` (the root's own path) then raises "boom" and exits 5 --
/// the null child `"a"` is reached, `f`(`.a`) is evaluated on it, and
/// `cond`'s error on that result aborts the call, all before any further
/// descent.
#[test]
fn test_resolve_recurse_null_child_still_evaluates_cond_and_propagates_its_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "path(recurse(.a; error(\"boom\")))"],
        Some(r#"{"a":null}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "jq: error (at <stdin>:0): boom\n");
    Ok(())
}

#[test]
fn test_recurse_f_keeps_own_partial_fanout_subtree_before_error_842() -> Result<()> {
    // A deeper #842 repro: the successfully-produced child (`.child`)
    // itself has further children under recursion, so the *entire*
    // subtree reached from `f`'s own partial fan-out must be visited
    // before the error — not just the bare partial value. Verified
    // against jq 1.7.1: `echo
    // '{"child":{"child":"leaf","leafflag":true},"bad":3}' | jq -c
    // 'recurse(if (.|type)=="object" then (.child, .bad[0]) else empty
    // end)'` prints the root, `{"child":"leaf","leafflag":true}`,
    // `"leaf"`, and `null` (in that order) before erroring, and exits 5.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"recurse(if (.|type)=="object" then (.child, .bad[0]) else empty end)"#,
        ],
        Some(r#"{"child":{"child":"leaf","leafflag":true},"bad":3}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(
        stdout,
        "{\"child\":{\"child\":\"leaf\",\"leafflag\":true},\"bad\":3}\n{\"child\":\"leaf\",\"leafflag\":true}\n\"leaf\"\nnull\n"
    );
    assert!(
        stderr.contains("Cannot index number with number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_walk_propagates_halt_from_f() -> Result<()> {
    // `builtin_walk`'s `Err(e) => e.into()` arm: `walk_impl` applies `f` to
    // the (already child-processed) value via `eval_owned_expr`, and a halt
    // from `f` propagates back up through every recursive `walk_impl` call's
    // `?` (through `Vec<_>`/`IndexMap<_>`'s `collect()` for container
    // values) to this top-level match, converting back into a `QueryResult`.
    // Verified against jq 1.7.1: `jq -n '1 | walk(halt_error(8))'` prints
    // nothing to stdout, dumps `1` (the value `f` was applied to) to stderr,
    // and exits 8.
    let (stdout, stderr, code) = run_jq_full(&["-n", "1 | walk(halt_error(8))"], None)?;
    assert_eq!(code, 8, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_isvalid`'s `QueryResult::Partial(_, Control::Halt(code))` arm
/// (distinct from the bare `QueryResult::Halt` arm already covered by
/// `test_isvalid_propagates_halt_instead_of_answering_true`): `isvalid`
/// forces `optional = true` and evaluates its argument via a single
/// `eval_single` call, so an argument that produces an output *before*
/// halting (`1, halt_error(3)`) comes back as `QueryResult::Partial`, not a
/// bare `Halt`. Must still halt, not report `true` just because a value was
/// already produced before the halt.
#[test]
fn test_isvalid_propagates_halt_from_partial_argument_result() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "isvalid(1, halt_error(3)), \"after\""], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_isvalid`'s `QueryResult::Break`/`Partial(_, Control::Break(_))`
/// arms (#867): mirrors the `Halt` arms directly above -- an unresolved
/// `break $label` in `isvalid`'s argument must keep unwinding toward its
/// enclosing `label`, not be swallowed by the old `_ => Bool(true)`
/// catch-all. `isvalid` is a succinctly-only extension (no real jq to diff
/// against), but the `break`/`label` semantics it must not interfere with
/// are real jq's own: this input would produce no output and exit 0 in real
/// jq if `isvalid` didn't exist to swallow the break at all.
#[test]
fn test_isvalid_propagates_bare_break_to_outer_label() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "label $out | isvalid(break $out), \"after\""], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// Companion to the bare-break test above, for the `Partial(_,
/// Control::Break(_))` arm specifically: an argument that produces an
/// output *before* breaking (`1, break $out`) comes back as
/// `QueryResult::Partial`, not a bare `Break` -- same distinction
/// `test_isvalid_propagates_halt_from_partial_argument_result` draws for
/// `Halt`.
#[test]
fn test_isvalid_propagates_break_from_partial_argument_result() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "label $out | isvalid(1, break $out), \"after\""],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// The issue's own repro shape: a `break` surfacing through
/// `paths(node_filter)` (only reachable as a bare `Break` since #850 fixed
/// `builtin_paths_filter`'s own control-signal handling) wrapped in
/// `isvalid`. Confirms the fix is reachable through a realistic nested
/// builtin call, not just a directly-written `break $out`.
#[test]
fn test_isvalid_propagates_break_through_paths_filter() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | isvalid(paths(if type=="number" then break $out else true end))"#,
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// `continue_rest_with_context`'s bare `QueryResult::Halt` arm: reached
/// whenever an `If`/`Comma`/`Try`/`Label` branch inside a path-context pipe
/// (here, `if`'s `then` branch) halts with zero prior output of its own and
/// there is still more pipe left to run (`| "tail"`) after it. Distinct
/// from `accumulate_path_context_step`'s own halt handling, which stops the
/// `key` comma branch's accumulation one level up, and from the `If` arm's
/// own `cond`-halts-first case (see the `test_path_context_if_arm_...`
/// test below). Verified against jq 1.7.1 -- bare if/pipe/halt has no
/// succinctly-specific behavior here: `jq -n '1, (if true then
/// halt_error(3) else 1 end | "tail")'` prints `1` then exits 3.
#[test]
fn test_path_context_continue_rest_propagates_bare_halt_from_if_branch() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#".a.b | key, (if true then halt_error(3) else 1 end | "tail")"#,
        ],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"b\"\n");
    Ok(())
}

/// Companion to `test_parent_propagates_halt_in_n_argument`: that test
/// proves a halt in `parent`'s `n` argument escapes even under `?`; this
/// proves the `Err(EvalEscape::Error(_)) if optional` guard right above it
/// still swallows a genuine error -- the split from the old catch-all
/// `Err(_) if optional` must not have started leaking ordinary errors too.
/// `has(error("x"))` is used rather than bare `error("x")` because
/// `error`'s own `optional` handling would otherwise self-swallow to
/// `Ok(Null)` before `parent`'s `n`-argument evaluator ever observes an
/// `Err`; `has` turns that into its own unconditional "no value" error,
/// which is what actually reaches this arm. `parent` is a succinctly
/// extension (no real-jq equivalent), so this is checked against
/// succinctly's own contract.
#[test]
fn test_parent_n_argument_still_swallows_ordinary_error_under_optional() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#".a.b | parent(has(error("x")))?"#],
        Some(r#"{"a":{"b":1}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// Companion to `test_path_context_optional_does_not_swallow_halt_in_builtin_arm`:
/// that test proves a halt escapes `eval_pipe_with_path_context_internal`'s
/// `Expr::Builtin` arm even under `?`; this proves the
/// `Err(EvalEscape::Error(_)) if optional` guard right above it still
/// swallows a genuine error. Same `has(error(...))` trick as the `parent`
/// test above, for the same reason (`error`'s own optional self-swallow
/// would otherwise never let an `Err` reach this arm at all).
#[test]
fn test_path_context_builtin_arm_still_swallows_ordinary_error_under_optional() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#".a.b | (has(error("boom")))?, key"#],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"b\"\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Companion to `test_path_context_optional_does_not_swallow_halt_in_object_literal_arm`:
/// proves the `Err(EvalEscape::Error(_)) if optional` guard in
/// `eval_pipe_with_path_context_internal`'s `Expr::Object | Expr::Array |
/// Expr::Literal` arm still swallows a genuine error, the same
/// Error-vs-Halt split as the `Expr::Builtin` arm two arms up. Same
/// `has(error(...))` trick, for the same reason.
#[test]
fn test_path_context_object_literal_arm_still_swallows_ordinary_error_under_optional() -> Result<()>
{
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#".a.b | ({x: has(error("boom"))})?, key"#],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"b\"\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// `eval_pipe_with_path_context_internal`'s `Expr::If` arm unpacks
/// `cond_result` into `(cond_values, cond_control)` before running any
/// branch; this targets its `QueryResult::Halt(code) => (Vec::new(),
/// Some(Control::Halt(code)))` case -- `cond` itself halting with zero
/// prior output -- distinct from `continue_rest_with_context`'s halt
/// handling (which only runs *after* a branch has already been picked and
/// evaluated, see the test above). Verified against jq 1.7.1: `jq -n '1,
/// (if halt_error(3) then "t" else "f" end)'` prints `1` then exits 3.
#[test]
fn test_path_context_if_arm_converts_bare_halt_from_cond() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#".a.b | key, (if halt_error(3) then "t" else "f" end)"#,
        ],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"b\"\n");
    Ok(())
}

/// Companion to the `Expr::Builtin`/object-literal arms above: targets
/// `eval_pipe_with_path_context_internal`'s final catch-all `_` arm
/// (reached for expression kinds with no dedicated handling here, e.g. `X
/// as $v | BODY`). Proves its `Err(EvalEscape::Error(_)) if optional`
/// guard still swallows a genuine error reached this way. Same
/// `has(error(...))` trick, for the same reason.
#[test]
fn test_path_context_generic_fallback_still_swallows_ordinary_error_under_optional() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#".a.b | key, (1 as $x | has(error("boom")))?"#],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"b\"\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// `builtin_fromstream`'s bare `QueryResult::Halt` arm: a filter argument
/// that halts before producing any tostream-style event has no prior
/// prefix to fall back to (unlike a `Partial`'s trailing control, handled
/// separately by the `Partial` arm just below it). Falling through to
/// `result.collect_owned()` would silently treat the halt as "zero events"
/// instead of halting. `fromstream` is a real jq builtin; verified against
/// jq 1.7.1: `jq -n 'fromstream(halt_error(3))'` exits 3 with no output.
#[test]
fn test_fromstream_propagates_bare_halt_from_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "fromstream(halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_truncate_stream`'s bare `QueryResult::Halt` arm -- same shape
/// as the matching arm in `builtin_fromstream` just above it (the doc
/// comment on this arm points back at that one). `truncate_stream` is a
/// real jq builtin, taking its depth from `.` and its stream filter as the
/// argument; verified against jq 1.7.1: `jq -n 'null |
/// truncate_stream(halt_error(3))'` exits 3 with no output.
#[test]
fn test_truncate_stream_propagates_bare_halt_from_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "null | truncate_stream(halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_paths_filter`'s per-path loop (distinct from its root-level
/// halt pre-check just above, which only fires if the *root* value itself
/// halts the filter) used the old `EvalError::halt` field
/// (`Err(e) if e.halt.is_some() => return query_result_from_error(e)`)
/// before this refactor to `EvalEscape`; this exercises the same halt path
/// through its new `Err(EvalEscape::Halt(code))` arm. `paths` is a real jq
/// builtin. The root here is an array (`type == "array"`), so the root
/// pre-check does not fire; the filter only halts once the walk reaches
/// the string child at index 1 -- the match already found at index 0 must
/// still be reported (#400/#494), matching real jq's own streaming `paths`,
/// which prints `[0]` before halting.
#[test]
fn test_paths_filter_propagates_halt_from_non_root_path() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"paths(if type=="string" then halt_error(3) else true end)"#,
        ],
        Some(r#"[1,"x"]"#),
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[0]\n");
    Ok(())
}

/// Companion to the halt test above, now pinning #850's fix: an ordinary
/// per-node error/break aborts the whole `paths` call instead of being
/// swallowed and moving on to the next node (see `builtin_paths_filter`'s
/// own comment for why this matches real jq). Verified against real jq
/// 1.7.1: `[1,"x",2] | paths(if type=="string" then error("bad") else true
/// end)` streams `[0]`, then raises without ever reaching index 2.
#[test]
fn test_paths_filter_aborts_on_ordinary_per_path_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"paths(if type=="string" then error("bad") else true end)"#,
        ],
        Some(r#"[1,"x",2]"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[0]\n");
    assert_eq!(stderr, "jq: error (at <stdin>:0): bad\n");
    Ok(())
}

/// #773: `builtin_paths_filter` used to evaluate `node_filter` per path via
/// `eval_owned_expr`, which collapses a multi-output result into a single
/// `OwnedValue::Array` when it produces 2+ outputs. A non-empty array is
/// always truthy in jq, so a `node_filter` with two falsy outputs (`false,
/// false`) was wrapped into the truthy array `[false,false]` *before* the
/// truthiness check ever ran, silently keeping every path. Verified against
/// real jq 1.7.1: `{"a":1,"b":{"c":2}} | [paths(false,false)]` is `[]`.
#[test]
fn test_paths_filter_all_falsy_multi_output_keeps_nothing() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "[paths(false,false)]"],
        Some(r#"{"a":1,"b":{"c":2}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Companion to the all-falsy test above, pinning the other half of #773's
/// fix: the fan-out direction. jq's `paths(node_filter)` is
/// `path(recurse|select(node_filter))`, and `select(f)` is literally
/// `if f then . else empty end` in jq's own builtin.jq — `if` forks over
/// every output its condition produces. So a multi-output `node_filter`
/// with 2+ *truthy* outputs must duplicate the path once per truthy
/// output, not keep it once regardless of how many outputs were truthy.
/// Verified against real jq 1.7.1:
/// `{"a":1,"b":{"c":2}} | [paths(true,true)]` is
/// `[["a"],["a"],["b"],["b"],["b","c"],["b","c"]]` — every path duplicated,
/// not deduplicated to one occurrence each.
#[test]
fn test_paths_filter_multi_truthy_output_duplicates_each_path() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "[paths(true,true)]"],
        Some(r#"{"a":1,"b":{"c":2}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(
        stdout,
        "[[\"a\"],[\"a\"],[\"b\"],[\"b\"],[\"b\",\"c\"],[\"b\",\"c\"]]\n"
    );
    assert_eq!(stderr, "");
    Ok(())
}

/// A single-truthy-output `node_filter` (the common case) must not
/// duplicate — this is the control proving the fan-out fix above didn't
/// overcorrect into always duplicating. Verified against real jq 1.7.1:
/// `{"a":1,"b":{"c":2}} | [paths(true)]` is `[["a"],["b"],["b","c"]]`.
#[test]
fn test_paths_filter_single_truthy_output_does_not_duplicate() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "[paths(true)]"], Some(r#"{"a":1,"b":{"c":2}}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[[\"a\"],[\"b\"],[\"b\",\"c\"]]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Mixed truthy/falsy outputs (`true,false`) keep the path exactly once
/// (one truthy output out of two), distinguishing this from both the
/// all-falsy (#773's original repro) and all-truthy (duplicate) cases
/// above. Verified against real jq 1.7.1:
/// `{"a":1,"b":{"c":2}} | [paths(true,false)]` is
/// `[["a"],["b"],["b","c"]]`.
#[test]
fn test_paths_filter_mixed_truthy_falsy_keeps_path_once() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "[paths(true,false)]"],
        Some(r#"{"a":1,"b":{"c":2}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[[\"a\"],[\"b\"],[\"b\",\"c\"]]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #773 follow-up (review of the original fix, commit 047133e2): once
/// `builtin_paths_filter` switched to `eval_owned_multi` to preserve
/// fan-out cardinality, a *different* regression appeared -- a node whose
/// `node_filter` fans out into a truthy output followed by an ordinary
/// `error` had its truthy output silently dropped too, where the pre-fix
/// code (via `eval_owned_expr`'s single-value collapse) happened to keep
/// it. Confirmed against real jq 1.7.1: `[1,"x",2] | paths(if
/// type=="string" then (true, error("bad")) else true end)` streams `[0]`
/// then `[1]` before raising -- the truthy output preceding the error is
/// not discarded. Since #850, succinctly also aborts the whole `paths`
/// call on that same error rather than continuing to the next node (see
/// `builtin_paths_filter`'s own comment), so index 2 (`[2]`) is never
/// reached, matching real jq exactly.
#[test]
fn test_paths_filter_keeps_truthy_prefix_before_ordinary_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"paths(if type=="string" then (true, error("bad")) else true end)"#,
        ],
        Some(r#"[1,"x",2]"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[0]\n[1]\n");
    assert_eq!(stderr, "jq: error (at <stdin>:0): bad\n");
    Ok(())
}

/// Same regression as the ordinary-error test above, but for `halt_error`:
/// a truthy output preceding a `halt` within one node's fan-out must still
/// be reported before the whole builtin halts (halt itself must still
/// propagate out unconditionally, #791 -- only the *dropped prefix* was
/// the bug). Confirmed against real jq 1.7.1: `[1,"x",2] | paths(if
/// type=="string" then (true, halt_error(3)) else true end)` streams
/// `[0]` then `[1]` to stdout before exiting 3.
#[test]
fn test_paths_filter_keeps_truthy_prefix_before_halt() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"paths(if type=="string" then (true, halt_error(3)) else true end)"#,
        ],
        Some(r#"[1,"x",2]"#),
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[0]\n[1]\n");
    Ok(())
}

/// Same regression again, but for `break`: a truthy output preceding a
/// `break $label` within one node's fan-out must still be reported, and
/// (since #850) the `break` itself now correctly unwinds to a `label`
/// lexically enclosing the whole `paths(...)` call instead of being
/// swallowed and treated as an ordinary per-node error (see
/// `builtin_paths_filter`'s own comment for why this works without needing
/// #833's broader fix). Confirmed against real jq 1.7.1: `label $out |
/// paths(if type=="string" then (true, break $out) else true end)` on
/// `[1,"x",2]` streams `[0]` then `[1]` before the break unwinds, exiting 0
/// -- index 2 is never reached.
#[test]
fn test_paths_filter_keeps_truthy_prefix_before_break() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | paths(if type=="string" then (true, break $out) else true end)"#,
        ],
        Some(r#"[1,"x",2]"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[0]\n[1]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Distinct from `test_setpath_propagates_halt_in_path_argument` above,
/// which hits `builtin_setpath`'s bare `QueryResult::Halt` arm (the path
/// argument halts with zero prior output): this hits the
/// `QueryResult::Partial(_, Control::Halt(code))` arm just below it, where
/// the path argument's stream produces an output *before* halting.
/// `builtin_setpath` reads a single (not fanned-out) path result via
/// `eval_single`, so this diverges from real jq's own generator-based
/// `setpath`, which evaluates the path argument lazily and errors on the
/// first (non-array) output before ever reaching the halt: `jq -n
/// 'setpath((1, halt_error(6)); 1)'` raises "Path must be specified as an
/// array" instead of halting. This checks succinctly's own contract
/// instead -- the halt still wins over any partially-produced path output,
/// matching the pre-#791 bug this arm fixes (silently writing `null`/an
/// unrelated error) rather than propagating the halt.
#[test]
fn test_setpath_propagates_halt_from_partial_path_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "setpath((1, halt_error(6)); 1)"], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// Distinct from `test_setpath_propagates_halt_in_value_argument` above,
/// which hits the bare `QueryResult::Halt` arm (value argument halts with
/// zero prior output): this hits the `QueryResult::Partial(_,
/// Control::Halt(code))` arm just below it, where the value argument's
/// stream produces an output *before* halting. Diverges from real jq here
/// too -- `jq -n 'setpath(["a"]; (1, halt_error(6)))'` fans out and prints
/// `{"a":1}` before exiting 6 -- because `builtin_setpath` takes a single
/// (not fanned-out) value result, the same architectural simplification as
/// the path-argument test above; this checks succinctly's own contract
/// that the halt still wins and no output escapes.
#[test]
fn test_setpath_propagates_halt_from_partial_value_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"setpath(["a"]; (1, halt_error(6)))"#], None)?;
    assert_eq!(code, 6, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_del`'s `resolve_dynamic_indexes` error handling: `?` swallows
/// only a genuine error resolving a computed key (`Err((_,
/// EvalEscape::Error(_))) if optional`), matching the same split
/// `eval_assign` already applies -- this is the ordinary-error half; a
/// halt in the same position is checked by
/// `test_halt_not_caught_by_bare_optional_in_path_expression` (a different
/// call site, `resolve_node`'s own `Expr::Optional` arm, not this one).
/// `del` is a real jq builtin; verified against jq 1.7.1: `jq -n
/// '[1,2,3]|del(.[error("x")])?'` exits 0 with no output, same as
/// succinctly here.
#[test]
fn test_del_computed_index_still_swallows_ordinary_error_under_optional() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"del(.[error("x")])?"#], Some("[1,2,3]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_mktime`'s six `get_int(IDX)` call sites were mechanically
/// migrated off the old `query_result_from_error(e)` helper (which read an
/// `EvalError::halt` field that no longer exists) to `e.into()`. This is
/// not itself a halt-propagation fix -- `get_int` only inspects an
/// already-materialized array element and never evaluates an expression,
/// so it can never carry a halt marker -- but exercises all six sites the
/// mechanical migration touched, one per broken-down-time field. `mktime`
/// is a real jq builtin; verified against jq 1.7.1 that a non-numeric
/// element anywhere in the first six positions is a hard error (jq's own
/// message differs -- it validates the whole array at once rather than
/// per-field -- but the error-not-swallowed shape matches).
#[test]
fn test_mktime_get_int_error_sites_report_plain_type_error_per_field() -> Result<()> {
    for (idx, input) in [
        (0, r#"["bad",1,1,0,0,0]"#),
        (1, r#"[2020,"bad",1,0,0,0]"#),
        (2, r#"[2020,0,"bad",0,0,0]"#),
        (3, r#"[2020,0,1,"bad",0,0]"#),
        (4, r#"[2020,0,1,0,"bad",0]"#),
        (5, r#"[2020,0,1,0,0,"bad"]"#),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", "mktime"], Some(input))?;
        assert_eq!(
            code, 5,
            "index {idx}: stdout: {stdout:?} stderr: {stderr:?}"
        );
        assert_eq!(stdout, "", "index {idx}");
    }
    Ok(())
}

/// `builtin_strftime`'s format-string argument went through the same
/// mechanical `query_result_from_error` -> `e.into()` migration as
/// `mktime`'s fields, but unlike those, `fmt_expr` is a full expression
/// that can legitimately halt (`result_to_owned` on its `eval_single`
/// result). `strftime` is a real jq builtin; verified against jq 1.7.1:
/// `jq -n 'strftime(halt_error(3))'` exits 3 with no output.
#[test]
fn test_strftime_propagates_halt_in_format_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "strftime(halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// Same shape as `builtin_strftime`'s format argument test above, one
/// function over (`builtin_strptime`'s `fmt_expr`). `strptime` is a real
/// jq builtin; verified against jq 1.7.1: `jq -n
/// 'strptime(halt_error(3))'` exits 3 with no output.
#[test]
fn test_strptime_propagates_halt_in_format_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "strptime(halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_combinations_n`'s `n`-argument wildcard swallowed a halt the
/// same way `nth`'s `n` argument used to. `combinations(n)` is a real jq
/// builtin, but real jq's own `n` is bound via `as $n` and interacts with
/// a halt in a way `builtin_combinations_n`'s single `result_to_owned` +
/// `eval_single` read does not attempt to reproduce -- confirmed live:
/// `jq -n '[1] | combinations(halt_error(3))'` unexpectedly prints `[1]`
/// before halting. This checks succinctly's own single-shot contract
/// instead, the same kind of pre-existing, unrelated-to-#791 divergence
/// already documented for `nth`/`setpath` elsewhere in this file.
#[test]
fn test_combinations_n_propagates_halt_in_n_argument() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "[1] | combinations(halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_limit`'s body-stream match gained an explicit
/// `QueryResult::Halt` arm: a bare halt (zero prior output) from `expr`
/// must exit, not fall into the `Partial` handling below it (which drops
/// its trailing control once the prefix already reaches `n`). `limit` is a
/// real jq builtin; verified against jq 1.7.1: `jq -n 'limit(1;
/// halt_error(3))'` exits 3 with no output.
#[test]
fn test_limit_stream_propagates_bare_halt_from_body() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "limit(1; halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// Same shape as `builtin_limit`'s bare-halt arm above, one function over
/// in `builtin_first_stream`. `first(expr)` is a real jq builtin; verified
/// against jq 1.7.1: `jq -n 'first(halt_error(3))'` exits 3 with no
/// output.
#[test]
fn test_first_stream_propagates_bare_halt_from_body() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "first(halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_last_stream` has two distinct halt arms: a bare
/// `QueryResult::Halt` (zero prior output) and a `QueryResult::Partial(_,
/// Control::Halt(code))` (some output already produced) -- unlike `first`,
/// `last` can never short-circuit, so it must consume the whole stream and
/// both shapes are reachable. `last(expr)` is a real jq builtin; verified
/// against jq 1.7.1: both `jq -n 'last(halt_error(3))'` and `jq -n
/// 'last(1, halt_error(3))'` exit 3 with no output.
#[test]
fn test_last_stream_propagates_halt_bare_and_after_partial_output() -> Result<()> {
    for filter in ["last(halt_error(3))", "last(1, halt_error(3))"] {
        let (stdout, stderr, code) = run_jq_full(&["-n", filter], None)?;
        assert_eq!(code, 3, "{filter}: stdout: {stdout:?} stderr: {stderr:?}");
        assert_eq!(stdout, "", "{filter}");
    }
    Ok(())
}

/// `builtin_nth_stream` has two halt sites this batch targets: `n`'s own
/// evaluation ending in a `QueryResult::Partial(_, Control::Halt(code))`
/// (some `n` candidate produced before halting), and a bare
/// `QueryResult::Halt` from `expr`'s body stream once `n` itself resolved
/// cleanly. `nth(n; expr)` is a real jq builtin, but real jq's own `n` is
/// bound via `as $n` and fans out -- confirmed live: `jq -n
/// 'nth((1, halt_error(3)); 1,2,3)'` actually *prints* `2` (it fully
/// evaluates the body for the first `n` candidate before the second one
/// halts) -- while `builtin_nth_stream` reads `n_expr` with a single
/// `eval_single` call, the same pre-existing, unrelated-to-#791
/// simplification already documented for `combinations`/`setpath`
/// elsewhere in this file; this checks succinctly's own single-shot
/// contract for that first case. The second case (`expr` itself halting,
/// `n` already resolved) has no such divergence: `jq -n 'nth(0;
/// halt_error(3))'` exits 3 with no output, matching succinctly here too.
#[test]
fn test_nth_stream_propagates_halt_from_n_partial_and_body_bare() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "nth((1, halt_error(3)); 1,2,3)"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");

    let (stdout, stderr, code) = run_jq_full(&["-n", "nth(0; halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_nth_stream_partial_halt_in_stream_argument() -> Result<()> {
    // `builtin_nth_stream`'s second halt arm: unlike
    // `test_nth_stream_propagates_halt_in_n_argument` above (bare
    // `QueryResult::Halt` reached when the `n` argument itself halts), this
    // exercises the `Control::Halt` arm nested inside the trailing
    // `QueryResult::Partial` match on the *stream* argument -- reached when
    // `n` indexes past every value the stream produced before it halted.
    // Verified against jq 1.7.1: `jq -n 'nth(5; 1,2,3,halt_error(7))'` exits
    // 7 with no output (the stream never reaches index 5, so the trailing
    // halt wins over the type-error fallback).
    let (stdout, stderr, code) = run_jq_full(&["-n", "nth(5; 1,2,3,halt_error(7))"], None)?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_delpaths_propagates_halt_via_partial_prefix_in_paths_argument() -> Result<()> {
    // `builtin_delpaths`'s `QueryResult::Partial(_, Control::Halt(code))`
    // arm -- a second, distinct arm from the bare `Halt` case
    // `test_delpaths_propagates_halt_in_paths_argument` above covers --
    // fires when the `paths` argument produces at least one successful
    // output (via a comma expression) before halting.
    //
    // Note: `builtin_delpaths` evaluates `paths_expr` with a single
    // `eval_single` call, unlike real jq's per-output "run once per value
    // the argument filter generates" semantics for builtin filter arguments
    // (a pre-existing, separate implementation gap unrelated to this halt
    // fix): `jq -n '{"a":1,"b":2,"c":3} | delpaths((1, halt_error(9)))'`
    // errors out immediately on the first output `1` not being an array
    // (exit 5), never reaching `halt_error` at all. Checked here against
    // succinctly's own contract instead: the halt must still win, discarding
    // the `1` prefix entirely rather than emitting a result for it first.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "{\"a\":1,\"b\":2,\"c\":3} | delpaths((1, halt_error(9)))",
        ],
        None,
    )?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "{\"a\":1,\"b\":2,\"c\":3}\n");
    Ok(())
}

#[test]
fn test_pow_propagates_halt_via_partial_prefix_in_exponent_argument() -> Result<()> {
    // `get_number_from_result`'s `QueryResult::Partial(_, Control::Halt(code))`
    // arm -- reached when an argument expression produces at least one
    // output (e.g. a comma expression) before halting, rather than halting
    // immediately -- feeding `builtin_pow`'s own `exp`-branch
    // `Err(NumberError::Halt(code)) => return QueryResult::Halt(code)` arm
    // (a distinct site from the `base`-branch arm
    // `test_pow_propagates_halt_in_argument` above already covers, one
    // match block over).
    //
    // Note: `builtin_pow` evaluates each argument with a single
    // `eval_single` call, the same pre-existing generator-vs-single-eval gap
    // noted on `delpaths` above: `jq -n 'pow(2; (1, halt_error(3)))'` prints
    // `2` (from the first exponent output) before halting with exit 3.
    // Checked here against succinctly's own contract instead: the halt
    // discards the whole call, including the already-computed
    // `1`-exponent partial output, producing no stdout at all.
    let (stdout, stderr, code) = run_jq_full(&["-n", "pow(2; (1, halt_error(3)))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_atan2_propagates_halt_in_y_argument() -> Result<()> {
    // `builtin_atan2`'s `y`-branch `Err(NumberError::Halt(code)) => return
    // QueryResult::Halt(code)` arm -- shares `get_number_from_result` with
    // `pow`/`atan2`'s other argument but is its own distinct return site.
    // Verified against jq 1.7.1: `jq -n 'atan2(halt_error(4); 2)'` exits 4
    // with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "atan2(halt_error(4); 2)"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_atan2_propagates_halt_in_x_argument() -> Result<()> {
    // `builtin_atan2`'s `x`-branch `Err(NumberError::Halt(code)) => return
    // QueryResult::Halt(code)` arm -- a distinct site from the `y`-branch
    // arm above, one match block over. Verified against jq 1.7.1:
    // `jq -n 'atan2(2; halt_error(4))'` exits 4 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "atan2(2; halt_error(4))"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_halt_error_with_non_number_argument_is_ordinary_catchable_error() -> Result<()> {
    // `builtin_halt_error`'s `Ok(_) => return QueryResult::Error(...)` arm
    // fires when the exit-code argument evaluates to a non-number (string,
    // bool, array, object, ...) -- distinct from every other arm in this
    // match, which all handle a genuine halt. Unlike `halt`/`halt_error(n)`
    // itself, a malformed exit-code argument must NOT halt at all: it's an
    // ordinary, `try`/`catch`-catchable type error. Verified against jq
    // 1.7.1: `jq -n 'try halt_error("x") catch "caught"'` prints `"caught"`
    // and exits 0 (real jq's own uncaught "number required" error exits 5 --
    // the ordinary uncaught-error code, not a halt -- confirming this is a
    // regular error in real jq too, not a halt).
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#"try halt_error("x") catch "caught""#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"caught\"\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_halt_error_propagates_halt_from_exit_code_argument() -> Result<()> {
    // `builtin_halt_error`'s `Err(e) => return e.into()` arm, reached when
    // `result_to_owned` on the exit-code argument returns
    // `Err(EvalEscape::Halt(code))` -- i.e. the exit-code expression itself
    // halts. `EvalEscape`'s `From` impl for `QueryResult` preserves `Halt`
    // by construction (#791), so the *inner* halt code wins and the outer
    // `halt_error` call never runs at all. Verified against jq 1.7.1:
    // `jq -n 'halt_error(halt_error(3))'` exits 3 with no stderr output
    // (`.` is null in `-n` mode at the inner call).
    let (stdout, stderr, code) = run_jq_full(&["-n", "halt_error(halt_error(3))"], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_bsearch_propagates_halt_via_partial_prefix_in_target_argument() -> Result<()> {
    // `builtin_bsearch`'s `QueryResult::Partial(_, Control::Halt(code))` arm
    // -- a second, distinct arm from the bare `Halt` case
    // `test_bsearch_propagates_halt_in_target_argument` above covers --
    // fires when the target expression produces an output before halting.
    //
    // Note: `builtin_bsearch` evaluates `x_expr` with a single
    // `eval_single` call, the same pre-existing generator-vs-single-eval gap
    // noted on `delpaths`/`pow` above: `jq -n '[1,2,3] | bsearch((1,
    // halt_error(15)))'` prints `0` (bsearch(1) succeeds on the first
    // output) before halting with exit 15. Checked here against
    // succinctly's own contract instead: the halt discards the whole call,
    // producing no stdout. stderr still matches real jq byte-for-byte
    // though, since `.` at the `halt_error` call is the same either way --
    // the original `[1,2,3]` input to `bsearch`, untouched by which
    // semantics evaluate the target.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "[1,2,3] | bsearch((1, halt_error(15)))"], None)?;
    assert_eq!(code, 15, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "[1,2,3]\n");
    Ok(())
}

#[test]
fn test_as_pattern_propagates_halt_in_bind_expression() -> Result<()> {
    // `eval_as_pattern`'s `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm, reached when the bind expression (`expr`
    // in `expr as $pattern | body`) itself halts before producing any value
    // to destructure. Verified against jq 1.7.1:
    // `jq -n '(halt_error(4)) as {a: $x} | $x'` exits 4 with no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "(halt_error(4)) as {a: $x} | $x"], None)?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_as_pattern_propagates_halt_after_partial_body_output() -> Result<()> {
    // `eval_as_pattern`'s `QueryResult::Halt(code) => return
    // partial(all_results, Control::Halt(code))` arm, reached inside the
    // per-bound-value loop when an earlier bound value's `body` already
    // produced an output and a *later* bound value's `body` halts. The
    // prefix accumulated from the earlier iteration must still reach the
    // caller (as a `Partial`) instead of vanishing. Verified against jq
    // 1.7.1: `jq -cn '(({a: 1}, {a: 2})) as {a: $x} | if $x == 1 then $x else
    // halt_error(7) end'` prints `1` then exits 7.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "-c",
            "(({a: 1}, {a: 2})) as {a: $x} | if $x == 1 then $x else halt_error(7) end",
        ],
        None,
    )?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_func_def_expand_recurses_through_halt_stderr_and_halt_error_builtins() -> Result<()> {
    // `expand_func_calls_in_builtin`'s `Halt`/`Stderr`/`HaltError`/
    // `HaltErrorCode` arms, added alongside the four new `Control`/
    // `QueryResult` variants this PR introduces, so `eval_func_def`'s
    // AST-rewrite pass (`expand_func_calls`, run on the entire `then` tree
    // of every `def` regardless of whether the defined function is ever
    // called) can walk through these builtins instead of leaving an
    // unmatched `Builtin` variant. A single `def`'s `then` mentioning all
    // four covers every arm in one pass, since the static tree is rewritten
    // once regardless of which comma branch actually runs at evaluation
    // time -- here, only the first (`halt_error(3)`) does. Verified against
    // jq 1.7.1: `jq -n 'def noop: .; (halt_error(3), halt, ("x" | stderr),
    // halt_error)'` exits 3 with no output.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            r#"def noop: .; (halt_error(3), halt, ("x" | stderr), halt_error)"#,
        ],
        None,
    )?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_func_call_param_substitution_recurses_through_halt_stderr_and_halt_error_builtins(
) -> Result<()> {
    // `substitute_func_param`'s `Halt`/`Stderr`/`HaltError`/`HaltErrorCode`
    // arms -- the parameter-substitution twin of the `expand_func_calls`
    // test above, invoked once per parameter when a *parameterized*
    // function is actually called (`expand_func_calls`'s `FuncCall` arm
    // walks the body once per param via `substitute_func_param`). The
    // `HaltErrorCode(x)` arm's own recursive substitution is exercised too:
    // `x` is the function's parameter and must be replaced with the call's
    // actual argument (`4`) before evaluation, or the code wouldn't be `4`.
    // Verified against jq 1.7.1: `jq -n 'def f(x): (halt_error(x), halt,
    // ("y" | stderr), halt_error); f(4)'` exits 4 with no output.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            r#"def f(x): (halt_error(x), halt, ("y" | stderr), halt_error); f(4)"#,
        ],
        None,
    )?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// `eval_generic.rs`'s `LazySeq` machinery for #791: `map(f)` on the
/// element produced by `.[]` builds a `GenericResult::LazySeq` that stays
/// unforced (`Builtin::Map`'s native arm) until something pulls it. Here
/// that force happens inside `flatten_generic_results`, called once `.[]`'s
/// `ManyCursor` loop finishes (`eval_single`'s `Expr::Pipe` handling) --
/// `f`'s own `halt` reaches `LazySeq::fold_one`'s `into_lazy_items(...)`
/// call (`GenericResult::Halt(code) => Err(Control::Halt(code))`), which
/// `materialize_atomic()` propagates as `Err`, which `GenericResult::
/// materialize_lazy()`'s own `LazySeq` arm turns into a bare `Self::Halt`,
/// which `flatten_generic_results` re-raises as `Err(Control::Halt(code))`
/// instead of quietly treating it as a normal value. Piped stdin input is
/// required (not `-n` with an inline array literal): an inline literal
/// would force the whole pipe through the owned-value/`eval.rs` bridge,
/// bypassing this file's `Builtin::Map` arm entirely. Verified against jq
/// 1.7.1: `echo '[[1,2,3]]' | jq '.[] | map(if . == 2 then halt else .
/// end)'` prints nothing and exits 0.
#[test]
fn test_iterate_then_map_lazy_seq_halt_discards_array_no_stray_output() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[".[] | map(if . == 2 then halt else . end)"],
        Some("[[1,2,3]]"),
    )?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `push_generic_owned_values`'s `GenericResult::Halt(code) => return
/// Some(Control::Halt(code))` arm (#791, #768's fork helper): `Expr::
/// Compare`'s native handling forks its `right` operand through this
/// function before ever looking at `left`. Bare `halt` isn't itself
/// natively dispatched by `eval_single`, so it falls through to the `_`
/// wildcard's `full_eval` bridge, which hands back a bare `GenericResult::
/// Halt` -- exactly the shape this arm exists to catch instead of quietly
/// forking zero comparisons. Verified against jq 1.7.1: `jq -n '1 ==
/// halt'` prints nothing and exits 0 (not 5 -- bare `halt`, not
/// `halt_error`).
#[test]
fn test_compare_right_operand_halt_short_circuits_fork() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "1 == halt"], None)?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `eval_on_many_owned`'s `GenericResult::Halt(code) => return
/// partial_generic(results, Control::Halt(code))` arm (#791): reached when
/// an earlier pipe stage collapses to `GenericResult::ManyOwned` (here,
/// `(1,2,3)` -- an `Expr::Comma` bridged through the `_` wildcard since
/// comma isn't natively dispatched by `eval_single`) and a *later* stage
/// halts partway through piping each owned value in turn: the value
/// already produced (`1`) survives as a `Partial` prefix instead of
/// vanishing, matching #400/#494's contract for `Error`/`Break`. Verified
/// against jq 1.7.1: `jq -n '(1,2,3) | if . == 2 then halt else . end'`
/// prints `1` and exits 0 -- `3` never runs.
#[test]
fn test_comma_then_conditional_halt_keeps_prefix_from_many_owned_pipe() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "(1,2,3) | if . == 2 then halt else . end"], None)?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

/// `Expr::Optional`'s `GenericResult::LazySeq(seq) => match seq.
/// materialize_atomic() { ... Err(Control::Halt(code)) => GenericResult::
/// Halt(code) ... }` arm (#791): `map(f)?` parses to `Expr::Optional(
/// Builtin::Map(f))`, and plain `arr | map(f)` builds an unforced
/// `GenericResult::LazySeq` (#724, #725). `?` has to force it to know
/// whether to catch anything, and this arm deliberately does NOT fold a
/// resulting `Halt` into the `Error`/`Break` arm right above it -- `?`
/// catches those but must let a halt escape uncaught, mirroring the
/// non-lazy case `test_halt_not_caught_by_try_catch_or_label` already pins
/// for `("x"|halt_error)? // "fallback"`. Piped input (not an inline `-n`
/// array literal) keeps `map` on the cursor-native `Builtin::Map` arm
/// instead of `eval.rs`'s own eager implementation. Verified against jq
/// 1.7.1: `echo '[1,2,3]' | jq 'map(if . == 2 then halt_error(7) else .
/// end)?'` prints nothing to stdout, `2` to stderr, and exits 7 -- not 0,
/// which is what a caught halt would produce.
#[test]
fn test_map_optional_lazy_seq_does_not_catch_halt() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["map(if . == 2 then halt_error(7) else . end)?"],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "2\n");
    Ok(())
}

/// The `ManyCursor` loop's `GenericResult::Error(e) => { return match
/// flatten_generic_results(per_element) { ... Err(Control::Halt(code)) =>
/// GenericResult::Halt(code), ... } }` arm (#791, inside `eval_single`'s
/// `Expr::Pipe` handling): the first array element (`["a","b"]`) buffers an
/// unforced `map(...)`-produced `LazySeq` into `per_element` without
/// forcing it (`other => per_element.push(other)`); the second element
/// (`"boom"`, a plain string) can't be mapped at all, so its own `rest`
/// evaluation immediately yields a bare `GenericResult::Error`, triggering
/// this early-return branch before the loop ever reaches a third element.
/// Forcing `per_element` to compute the prefix then discovers the *first*
/// element's own buffered map halts -- and since it's chronologically
/// first, that halt outranks the second element's error entirely. Piped
/// input keeps `.[]`/`map` on the cursor-native path. Verified against jq
/// 1.7.1: `echo '[["a","b"], "boom"]' | jq '.[] | map(if . == "b" then
/// halt else . end)'` prints nothing and exits 0 (not an error about
/// `"boom"`).
#[test]
fn test_iterate_earlier_buffered_map_halt_outranks_later_element_error() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[".[] | map(if . == \"b\" then halt else . end)"],
        Some(r#"[["a","b"], "boom"]"#),
    )?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// The `ManyCursor` loop's own `GenericResult::Halt(code) => { return
/// match flatten_generic_results(per_element) { Ok(prefix) =>
/// partial_generic(prefix, Control::Halt(code)), ... } }` arm (#791, inside
/// `eval_single`'s `Expr::Pipe` handling): unlike the `Error`/`Break`
/// siblings just above it (pre-existing before #791), this whole branch is
/// new -- a halt reaching one cursor element directly (not via a buffered
/// `LazySeq`) must stop the loop immediately rather than falling through to
/// the `other => per_element.push(other)` wildcard, which would keep
/// evaluating later elements. The first element (`1`) succeeds normally
/// (buffered as `Owned(1)`, nothing to force), so `flatten_generic_results`
/// returns `Ok([1])` and this hits the `Ok(prefix)` sub-arm specifically.
/// Piped input keeps `.[]` on the cursor-native path. Verified against jq
/// 1.7.1: `echo '[1,2,3]' | jq '.[] | if . == 2 then halt else . end'`
/// prints `1` and exits 0 -- `3` is never evaluated.
#[test]
fn test_iterate_direct_halt_stops_loop_keeping_earlier_prefix() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&[".[] | if . == 2 then halt else . end"], Some("[1,2,3]"))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_manycursor_post_loop_flatten_propagates_halt_from_buffered_lazyseq() -> Result<()> {
    // `Expr::Pipe`'s `GenericResult::ManyCursor` arm buffers each cursor's
    // own result into `per_element` when it isn't itself an immediate
    // `Error`/`Break`/`Halt`/`Partial` (the `other => per_element.push(other)`
    // wildcard) -- a bare `map(f)` stays exactly that: an unmaterialized
    // `GenericResult::LazySeq`, since `Builtin::Map` only *builds* the lazy
    // chain, never evaluates `f`. Once the per-cursor loop exhausts every
    // cursor without an early return, `flatten_generic_results(per_element)`
    // finally pulls that buffered `LazySeq` -- and if `f` halts on its
    // first item, this is the one place a halt discovered *after* the
    // per-cursor loop has already finished must still make it out as
    // `GenericResult::Halt`, not get silently absorbed into `Ok`. Verified
    // against jq 1.7.1: `[[1]] | .[] | map(halt)` halts (real jq's `map`
    // is eagerly atomic) with no output and exit 0.
    let (stdout, stderr, code) = run_jq_full(&[".[] | map(halt)"], Some("[[1]]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_lazyseq_length_composability_propagates_halt_mid_fold() -> Result<()> {
    // `Expr::Pipe`'s `GenericResult::LazySeq` composability arm dispatches
    // `Builtin::Length` to a count-and-discard loop over the still-lazy
    // `map(f)` chain -- every element still runs (so `length` of a `map`
    // that halts partway isn't a partial count), and its own
    // `Err(Control::Halt(code)) => return GenericResult::Halt(code)` arm is
    // what turns that mid-fold halt into the pipe's overall result instead
    // of falling through to a bogus count. Verified against jq 1.7.1:
    // `[1,2,3] | map(halt) | length` halts while still building the mapped
    // array (real jq's `map` is atomic), never reaching `length` -- no
    // output, exit 0.
    let (stdout, stderr, code) = run_jq_full(&["map(halt) | length"], Some("[1,2,3]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_lazyseq_iterate_composability_propagates_halt_mid_fold() -> Result<()> {
    // Same `GenericResult::LazySeq` composability dispatch as `length`
    // above, but for `Expr::Iterate` (`.[]`): it must pull the *entire*
    // lazy `map(f)` chain before yielding anything (real jq's array
    // construction is atomic, so a failure discards every already-yielded
    // element too), and its own `Err(Control::Halt(code)) => return
    // GenericResult::Halt(code)` arm is the site under test. Verified
    // against jq 1.7.1: `[1,2,3] | map(halt) | .[]` halts while building
    // the array -- no output, exit 0.
    let (stdout, stderr, code) = run_jq_full(&["map(halt) | .[]"], Some("[1,2,3]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_lazyseq_first_index0_composability_propagates_halt_on_pulled_element() -> Result<()> {
    // The `Builtin::First | Expr::Index(0)` composability arm is the
    // "pull-one-and-stop" fast path -- at most one element of the lazy
    // `map(f)` chain is ever evaluated. Its `Some(Err(Control::Halt(code)))
    // => GenericResult::Halt(code)` arm is what happens when *that one*
    // pulled element halts. Verified against jq 1.7.1: `[1,2,3] |
    // map(halt) | .[0]` halts while building the array (real jq's `map`
    // is atomic, so `.[0]` never gets to see a partial result) -- no
    // output, exit 0.
    let (stdout, stderr, code) = run_jq_full(&["map(halt) | .[0]"], Some("[1,2,3]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_lazyseq_fallback_materialize_propagates_halt() -> Result<()> {
    // Every other consumer of a `GenericResult::LazySeq` (anything besides
    // `Map`/`Length`/`Iterate`/`First`/`Index(0)`, e.g. `last`) falls to the
    // `_` arm: one atomic `materialize_atomic()` pass, then hand off to the
    // full evaluator. Its `Err(Control::Halt(code)) => GenericResult::Halt(code)`
    // arm is the site under test -- `last` never even gets a materialized
    // value to inspect. Verified against jq 1.7.1: `[1,2,3] | map(halt) |
    // last` halts while building the array -- no output, exit 0.
    let (stdout, stderr, code) = run_jq_full(&["map(halt) | last"], Some("[1,2,3]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_pipe_fold_forwards_halt_from_current_into_next_stage() -> Result<()> {
    // `Expr::Pipe`'s per-stage fold matches `current` before running the
    // *next* stage; when an earlier stage already resolved to
    // `GenericResult::Halt(code)`, its own `GenericResult::Halt(code) =>
    // return GenericResult::Halt(code)` arm is what stops the fold from
    // ever invoking the next stage at all -- the site under test.
    // Verified against jq 1.7.1: `halt | 1` halts on the first stage and
    // never evaluates `1` -- no output, exit 0.
    let (stdout, stderr, code) = run_jq_full(&["halt | 1"], Some("null"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_last_of_bare_halt_forwards_halt_directly() -> Result<()> {
    // `eval_first_or_last_generic`'s `want_last = true` branch (reached via
    // `Expr::LastExpr`, i.e. `last(...)`) has its own `GenericResult::Halt(code)
    // => GenericResult::Halt(code)` forwarding arm, distinct from the
    // `want_last = false` one exercised below -- both need independent
    // coverage since they're separate match arms in separate branches of
    // the same function. Verified against jq 1.7.1: `last(halt)` halts
    // immediately (real jq's `last(g)` def still has to run `g` to find
    // its last output) -- no output, exit 0.
    let (stdout, stderr, code) = run_jq_full(&["last(halt)"], Some("null"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_last_of_comma_then_halt_drops_prefix_and_forwards_halt() -> Result<()> {
    // `eval_first_or_last_generic`'s `want_last = true` branch also has a
    // dedicated `GenericResult::Partial(_, Control::Halt(code)) =>
    // GenericResult::Halt(code)` arm, distinct from the bare-`Halt` arm
    // above: `last` can't short-circuit on a first output (it doesn't know
    // a value is the last one until the stream ends), so a `Partial`'s
    // trailing control is what determines the outcome and its prefix is
    // dropped, matching `eval::eval_last_expr`. Verified against jq 1.7.1:
    // `last(1, halt)` still halts with no output despite `1` having been
    // produced first -- `last` has no "final" value to report once the
    // stream is cut short.
    let (stdout, stderr, code) = run_jq_full(&["last(1, halt)"], Some("null"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_first_of_bare_halt_forwards_halt_directly() -> Result<()> {
    // The `want_last = false` mirror of the `last(halt)` test above --
    // `Expr::FirstExpr`'s own `GenericResult::Halt(code) =>
    // GenericResult::Halt(code)` arm. Verified against jq 1.7.1:
    // `first(halt)` halts immediately -- no output, exit 0.
    let (stdout, stderr, code) = run_jq_full(&["first(halt)"], Some("null"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_computed_index_key_stream_bare_halt_with_no_prior_keys() -> Result<()> {
    // `eval_index_expr`'s key-stream match has its own bare
    // `GenericResult::Halt(code) => return GenericResult::Halt(code)` arm,
    // distinct from the `Partial(vs, Control::Halt(code))` arm exercised
    // by `test_computed_index_streams_keys_produced_before_halt` -- this
    // one fires when the key expression halts on its *very first*
    // attempt, before any key has ever been produced (so there is no
    // prefix to thread through `pending_halt` at all). Verified against jq
    // 1.7.1: `.[(halt_error(4))]` on `{"a":1}` writes the input document
    // (halt_error's current value, compact JSON with a trailing newline)
    // to stderr and exits 4 with no stdout.
    let (stdout, stderr, code) = run_jq_full(&[".[(halt_error(4))]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "{\"a\":1}\n");
    Ok(())
}

#[test]
fn test_computed_index_target_bare_halt_before_any_key_applied() -> Result<()> {
    // `eval_index_expr`'s *target* match (evaluated after the key stream
    // has already produced at least one key) has the identical bare
    // `GenericResult::Halt(code) => return GenericResult::Halt(code)` arm,
    // one match over from the key-stream's own copy above -- kept out of
    // the `owned @ (...)` group below so `collect_owned()` can't quietly
    // turn a halted target into an empty `Vec`. Verified against jq 1.7.1:
    // `(halt_error(4))[("a")]` writes the current value ("ignored", raw,
    // no trailing newline since it's a string) to stderr and exits 4 with
    // no stdout -- the target halts before any indexing ever happens.
    let (stdout, stderr, code) = run_jq_full(&[r#"(halt_error(4))[("a")]"#], Some("\"ignored\""))?;
    assert_eq!(code, 4, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "ignored");
    Ok(())
}

#[test]
fn test_computed_index_empty_target_still_honors_pending_halt() -> Result<()> {
    // `eval_index_expr`'s target match's `GenericResult::None` arm handles
    // a target with zero outputs -- not itself an error/break/halt, so it
    // has nothing that "happens first" to preempt a `pending_halt` already
    // recorded from the key stream. Its `match pending_halt { Some(code)
    // => GenericResult::Halt(code), None => GenericResult::None }` is the
    // site under test: a zero-output target must still let an
    // already-pending halt through rather than reporting `None` (#791).
    // Verified against jq 1.7.1: `(.a[])[(1, 2, halt)]` on `{"a": []}`
    // halts with no output -- `.a[]` produces nothing to index against for
    // either key, and the key generator's own halt still fires.
    let (stdout, stderr, code) = run_jq_full(&["(.a[])[(1, 2, halt)]"], Some(r#"{"a": []}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_computed_index_target_indexes_first_output_before_reaching_a_later_halt() -> Result<()> {
    // Unlike the target-`Error`/`Break` precedent this was originally
    // modeled on (`test_computed_index_still_conservative_on_error_and_break`),
    // a computed target is NOT collected as a whole standalone stream before
    // indexing starts: indexing the target's first output (`1`) is
    // attempted immediately, errors ("Cannot index number with string"),
    // and that error surfaces without ever reaching the target stream's
    // second output (`halt_error(6)`) at all -- matching real jq exactly.
    // Verified against jq 1.7.1: `(1, halt_error(6))[("a")]` errors the
    // same way, exit 5.
    let (stdout, stderr, code) = run_jq_full(&[r#"(1, halt_error(6))[("a")]"#], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot index number with string"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_computed_index_owned_target_streams_prefix_before_pending_halt() -> Result<()> {
    // The `owned @ (...)` branch handles a *computed* (non-navigational)
    // target -- here `({"a":1},{"a":2})`, a comma expression, which
    // resolves to `GenericResult::ManyOwned` rather than a cursor. Its
    // `match pending_halt { Some(code) => partial_generic(out,
    // Control::Halt(code)), ... }` arm is the site under test: every
    // already-indexed `out` value must still stream before the key
    // stream's recorded halt takes over. Verified against jq 1.7.1:
    // `({"a":1},{"a":2})[("a", halt)]` prints `1` then `2` (real jq's
    // key-outer/target-inner generator indexes both targets for key "a"
    // before the key generator's next attempt reaches `halt`), then halts
    // -- exit 0.
    let (stdout, stderr, code) = run_jq_full(&[r#"({"a":1},{"a":2})[("a", halt)]"#], Some("null"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_computed_index_array_key_error_streams_prefix_without_pending_halt() -> Result<()> {
    // `eval_index_expr`'s "key outer, target inner" cursor loop's `Error`
    // arm assembles the already-indexed prefix (`out`) before returning it
    // as a `Partial` -- the same fix as
    // `test_computed_index_target_error_after_pending_halt_still_streams_prefix`,
    // but isolated from any `pending_halt` interaction: no halt is
    // involved here at all, just a later key's ordinary type error on an
    // array target, confirming the prefix-preservation logic itself works
    // independently of the #791 halt-tracking it was added alongside.
    // Verified against jq 1.7.1: `.[(0, "x")]` on `[10,20]` prints `10`,
    // then errors "Cannot index array with string \"x\"" and exits 5.
    let (stdout, stderr, code) = run_jq_full(&[r#".[(0, "x")]"#], Some("[10,20]"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "10\n");
    assert!(
        stderr.contains("Cannot index array with string"),
        "{stderr}"
    );
    Ok(())
}

#[test]
fn test_computed_index_pending_halt_streams_owned_prefix_after_missing_key() -> Result<()> {
    // The cursor loop's tail `if let Some(code) = pending_halt { let out =
    // if any_owned { owned } else { cursors... }; return
    // partial_generic(out, Control::Halt(code)); }` has two branches
    // depending on whether the loop ever needed to fall back from cursors
    // to owned values -- `any_owned` becomes true the moment any key maps
    // to a *missing* field (`index_one_generic`'s `None => Owned(Null)`
    // arm), which forces every already-collected cursor to be converted to
    // owned too. This test drives that conversion so the `owned` branch
    // (not the `cursors` branch, already covered by
    // `test_computed_index_streams_keys_produced_before_halt`) is what's
    // returned. Verified against jq 1.7.1: `.[("a", "missing", halt)]` on
    // `{"a":1}` prints `1` then `null` (the missing key), then halts --
    // exit 0.
    let (stdout, stderr, code) =
        run_jq_full(&[r#".[("a", "missing", halt)]"#], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\nnull\n");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_slice_start_bound_halt_propagates() -> Result<()> {
    // `eval_slice_expr` evaluates its `start` bound via `eval_slice_bound`
    // before ever looking at `end` or the target; its own
    // `Err(Control::Halt(code)) => return GenericResult::Halt(code)` arm
    // converts a halted start-bound stream into the slice's overall
    // result. This also exercises `eval_slice_bound`'s own
    // `GenericResult::Halt(code) => return Err(Control::Halt(code))` arm,
    // which is what makes the halt visible to `eval_slice_expr` in the
    // first place instead of being silently swallowed by the
    // `other => other.collect_owned()` fallback (#791). Verified against
    // jq 1.7.1: `[1,2,3][halt_error(3):]` halts (the current value at that
    // point is the ambient input, `null`, so nothing prints to stderr) and
    // exits 3 with no stdout. The array literal is wrapped in parens here
    // because succinctly's parser (unlike jq's) doesn't accept a slice
    // bracket directly after an unparenthesized array-literal target.
    let (stdout, stderr, code) = run_jq_full(&["([1,2,3])[halt_error(3):]"], Some("null"))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_slice_end_bound_halt_propagates() -> Result<()> {
    // The `end`-bound counterpart of the `start`-bound test above --
    // `eval_slice_expr`'s second `eval_slice_bound` call has its own,
    // separate `Err(Control::Halt(code)) => return GenericResult::Halt(code)`
    // match arm one text block down, and reaches the same
    // `eval_slice_bound` Halt-detection arm as a dynamic slice bound like
    // `.[:halt_error(3)]` documents (#791). Verified against jq 1.7.1:
    // `[1,2,3][:halt_error(3)]` halts with no stdout and exit 3. The array
    // literal is wrapped in parens here because succinctly's parser (unlike
    // jq's) doesn't accept a slice bracket directly after an
    // unparenthesized array-literal target.
    let (stdout, stderr, code) = run_jq_full(&["([1,2,3])[:halt_error(3)]"], Some("null"))?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_slice_target_bare_halt_propagates() -> Result<()> {
    // `eval_slice_expr`'s own target match (evaluated once both bounds
    // have resolved) has the same bare `GenericResult::Halt(code) =>
    // return GenericResult::Halt(code)` arm `eval_index_expr`'s target
    // match has -- kept separate from the `owned @ (...)` group so a
    // halted target isn't silently swallowed by `collect_owned()`.
    // Verified against jq 1.7.1: `(halt_error(7))[0:1]` halts with no
    // stdout, and since the current value at that point (the ambient
    // input) is `null`, nothing prints to stderr either -- exit 7.
    let (stdout, stderr, code) = run_jq_full(&["(halt_error(7))[0:1]"], Some("null"))?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_slice_target_slices_first_output_before_reaching_a_later_halt() -> Result<()> {
    // Same shape as
    // `test_computed_index_target_indexes_first_output_before_reaching_a_later_halt`,
    // one construct over: a computed slice target is not collected as a
    // whole standalone stream before slicing starts either -- slicing the
    // target's first output (`1`) is attempted immediately, errors
    // ("Cannot index number with object"), and that error surfaces without
    // ever reaching the target stream's second output (`halt_error(7)`) at
    // all, matching real jq exactly. Verified against jq 1.7.1: `(1,
    // halt_error(7))[0:1]` errors the same way, exit 5.
    let (stdout, stderr, code) = run_jq_full(&["(1, halt_error(7))[0:1]"], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot index number with object"),
        "{stderr}"
    );
    Ok(())
}
