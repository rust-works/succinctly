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
