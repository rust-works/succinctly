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

/// `--argjson`'s number literal preserves its exact source spelling when
/// echoed back untouched, matching a filter-embedded literal (#1035) and a
/// document-sourced one, rather than round-tripping through
/// `serde_json::Value`'s own `f64`/`i64` `Display` (#1058). Verified
/// against the pinned real `jq` binary, which also preserves these exactly.
#[test]
fn test_argjson_preserves_number_literal_fidelity_1058() -> Result<()> {
    let (output, code) = run_jq_null("$n", &["--argjson", "n", "1.500"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1.500");

    let (output, code) = run_jq_null("$n", &["--argjson", "n", "1e100"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1E+100");
    Ok(())
}

// =============================================================================
// #1094: `--argjson` tolerates a leading-zero number the way real jq's own
// number parser does, instead of rejecting it outright via strict RFC 8259
// validation. All cases live-verified against jq 1.7.1.
// =============================================================================

/// The issue's own repro: a leading-zero integer/float, accepted and
/// normalized exactly like real jq.
#[test]
fn test_argjson_tolerates_leading_zero_number_1094() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", "007", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "7");

    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", "00", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "0");

    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", "007.5", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "7.5");
    Ok(())
}

/// A leading-zero number combined with a *trailing* zero (`007.500`) is
/// accepted (not rejected outright, this issue's own scope) and, since
/// #1149, also keeps the trailing zero's spelling (`7.500`, matching real
/// jq) rather than losing it (`7.5`) the way this crate's own
/// `NumberLiteral`-preservation fast path used to for any leading-zero
/// literal -- same fix (`OwnedValue::from_number_bytes` /
/// `DocumentValue::number_literal`) as the sibling scientific-notation
/// case below, since it's one root cause, broader than #1149's own title
/// suggested.
#[test]
fn test_argjson_leading_zero_accepted_and_trailing_zero_spelling_preserved_1149() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", "007.500", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "7.500");
    Ok(())
}

/// Trailing garbage after a leading-zero number is still rejected --
/// leading-zero tolerance must not accidentally widen the #284 guard it
/// sits next to.
#[test]
fn test_argjson_leading_zero_still_rejects_trailing_garbage_1094() -> Result<()> {
    let (_, _, code) = run_jq_full(&["-n", "--argjson", "n", "007 garbage", "$n"], None)?;
    assert_ne!(code, 0);
    Ok(())
}

/// Plain trailing garbage (no leading zero involved at all) is still
/// rejected -- confirms the fast, common-case path (no normalization
/// needed) is unaffected by this fix.
#[test]
fn test_argjson_plain_trailing_garbage_still_rejected_1094() -> Result<()> {
    let (_, _, code) = run_jq_full(&["-n", "--argjson", "n", "42 garbage", "$n"], None)?;
    assert_ne!(code, 0);
    Ok(())
}

/// A leading-zero number nested inside an array or object is tolerated
/// too, not just a bare top-level scalar.
#[test]
fn test_argjson_leading_zero_tolerated_when_nested_1094() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-cn", "--argjson", "n", "[007, 1]", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[7,1]");

    let (stdout, _, code) = run_jq_full(&["-cn", "--argjson", "n", r#"{"a": 007}"#, "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#"{"a":7}"#);
    Ok(())
}

/// Digits that merely *look* like a leading-zero number, but are actually
/// inside a string (a value or an object key), are left completely
/// untouched by the normalization pass.
#[test]
fn test_argjson_leading_zero_inside_string_or_key_untouched_1094() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", r#""007""#, "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#""007""#);

    let (stdout, _, code) = run_jq_full(&["-cn", "--argjson", "n", r#"{"007": 1}"#, "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#"{"007":1}"#);
    Ok(())
}

/// A normal number with no leading zero is completely unaffected --
/// confirms the fast path (strict validation succeeds on the first try,
/// no normalization attempted) isn't disturbed by this fix.
#[test]
fn test_argjson_normal_number_unaffected_1094() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", "42", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "42");
    Ok(())
}

/// Genuinely malformed JSON (not just a leading zero) is still rejected --
/// normalization doesn't accidentally make garbage input parse.
#[test]
fn test_argjson_malformed_json_still_rejected_1094() -> Result<()> {
    let (_, _, code) = run_jq_full(&["-n", "--argjson", "n", "not json", "$n"], None)?;
    assert_ne!(code, 0);
    Ok(())
}

/// Regression guard (found by review before merge): a bare `-` with no
/// digit following it isn't a number token at all, but an earlier draft
/// of `normalize_leading_zero_numbers` fabricated a `0` digit for it
/// (turning the genuinely invalid `-` into the valid number `-0`), which
/// made the retried validation wrongly pass and let this malformed input
/// silently reach materialization as `null` instead of erroring. Real jq
/// rejects all three of these outright.
#[test]
fn test_argjson_bare_hyphen_rejected_not_fabricated_into_negative_zero_1094() -> Result<()> {
    let (_, _, code) = run_jq_full(&["-n", "--argjson", "n", "-", "$n"], None)?;
    assert_ne!(code, 0);

    let (_, _, code) = run_jq_full(&["-cn", "--argjson", "n", "[-,1]", "$n"], None)?;
    assert_ne!(code, 0);

    let (_, _, code) = run_jq_full(&["-cn", "--argjson", "n", r#"{"a":-}"#, "$n"], None)?;
    assert_ne!(code, 0);
    Ok(())
}

/// A negative leading-zero number, nested inside an array -- exercises
/// `normalize_leading_zero_numbers`'s own leading-`-` handling. Nesting
/// was originally required to route around #1150's separate CLI
/// arg-parsing bug (a bare negative `--argjson` value was rejected before
/// `normalize_leading_zero_numbers` ever ran); #1150's own fix means the
/// bare form is now directly testable too, see
/// `test_argjson_bare_negative_leading_zero_1150` below.
#[test]
fn test_argjson_negative_leading_zero_when_nested_1094() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-cn", "--argjson", "n", "[-007, 1]", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[-7,1]");
    Ok(())
}

// --- #1150: clap rejected any negative-number (or other hyphen-prefixed)
// --argjson/--arg/--slurpfile/--rawfile VALUE before it ever reached this
// crate's own JSON-content validation -- fixed via `allow_hyphen_values`
// on all six affected clap::Arg sites (JqCommand's arg/argjson/slurpfile/
// rawfile, YqCommand's arg/argjson). Verified live against real jq 1.7.1.

#[test]
fn test_argjson_bare_hyphen_prefixed_value_1150() -> Result<()> {
    for (value, expected) in [("-7", "-7"), ("-007", "-7")] {
        let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", value, "$n"], None)?;
        assert_eq!(code, 0, "value {value:?}");
        assert_eq!(stdout.trim_end(), expected, "value {value:?}");
    }
    Ok(())
}

#[test]
fn test_arg_hyphen_prefixed_string_value_1150() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "--arg", "n", "-hello", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#""-hello""#);
    Ok(())
}

#[test]
fn test_multiple_argjson_one_negative_one_positive_1150() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &[
            "-cn",
            "--argjson",
            "a",
            "1",
            "--argjson",
            "b",
            "-2",
            "[$a,$b]",
        ],
        None,
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1,-2]");
    Ok(())
}

/// `--args`/`--jsonargs` (`num_args = 0..`) consume the *remaining*
/// command line as positional values -- the same clap hyphen-rejection
/// defect as the two-value `--arg`/`--argjson`/etc. flags above, on a
/// different clap shape (found during this issue's own code review, not
/// in the original repro). Verified live against real jq 1.7.1.
#[test]
fn test_args_positional_hyphen_prefixed_values_1150() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "$ARGS.positional", "--args", "-7", "abc"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[\n  \"-7\",\n  \"abc\"\n]");
    Ok(())
}

#[test]
fn test_jsonargs_positional_hyphen_prefixed_values_1150() -> Result<()> {
    let (stdout, _, code) =
        run_jq_full(&["-n", "$ARGS.positional", "--jsonargs", "-7", "-8"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[\n  -7,\n  -8\n]");
    Ok(())
}

/// `-L` (`ArgAction::Append`, exactly one value per occurrence -- a
/// different clap shape from every arg above, which is why #1150's
/// `allow_hyphen_values` fix didn't cover it -- and short-only, unlike
/// every other flag in this file: `succinctly jq --help` confirms there is
/// no `--library-path` long form to invoke) had the identical bug: a
/// hyphen-prefixed module directory was rejected by clap's
/// negative-number/unknown-flag heuristic before ever reaching this
/// crate's module-search-path logic. Filed as #1203 during #1150's own
/// review; fixed separately since it needed its own `allow_hyphen_values`
/// site. Verified live against real jq 1.7.1 (`jq -L -mymodules '.'`
/// succeeds there too -- the directory need not exist for a filter that
/// never imports a module).
#[test]
fn test_library_path_hyphen_prefixed_directory_1203() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-L", "-mymodules", "-n", "null"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "null");
    Ok(())
}

/// A string containing a backslash escape sequence, alongside a
/// leading-zero number that triggers normalization -- confirms the escape
/// handling inside `normalize_leading_zero_numbers`'s string-tracking
/// correctly copies the escaped character without misreading it as ending
/// the string early.
#[test]
fn test_argjson_escaped_string_alongside_leading_zero_1094() -> Result<()> {
    let (stdout, _, code) =
        run_jq_full(&["-cn", "--argjson", "n", r#"["a\nb", 007]"#, "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#"["a\nb",7]"#);
    Ok(())
}

/// A leading-zero number combined with scientific notation is accepted
/// (not rejected outright), exercising `normalize_leading_zero_numbers`'s
/// exponent-handling branch, and -- since #1149 -- keeps its scientific
/// notation on display (`7E+5`, matching real jq) instead of being
/// expanded to a plain decimal (`700000`).
#[test]
fn test_argjson_leading_zero_with_exponent_spelling_preserved_1149() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", "007e5", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "7E+5");
    Ok(())
}

/// Same as above, but with an explicit `+` sign on the exponent --
/// exercises `normalize_leading_zero_numbers`'s explicit-exponent-sign
/// branch specifically.
#[test]
fn test_argjson_leading_zero_with_signed_exponent_spelling_preserved_1149() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-n", "--argjson", "n", "007e+5", "$n"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "7E+5");
    Ok(())
}

// --- #1149: the same leading-zero spelling loss, via the crate's primary
// document-input path (not just --argjson) -- the issue's own repro shape.

/// A leading-zero number combined with scientific notation, via plain
/// document input, keeps its scientific notation on display (matching
/// real jq: `007e5 | .` -> `7E+5`) instead of being expanded to a plain
/// decimal.
#[test]
fn test_document_input_leading_zero_exponent_spelling_preserved_1149() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", "."], Some("007e5"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "7E+5");

    let (stdout, _, code) = run_jq_full(&["-c", "."], Some("-007e5"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "-7E+5");
    Ok(())
}

/// A leading-zero number combined with a trailing zero, via plain
/// document input, keeps the trailing zero's spelling (matching real jq:
/// `007.500 | .` -> `7.500`) instead of collapsing it to `7.5`.
#[test]
fn test_document_input_leading_zero_trailing_zero_spelling_preserved_1149() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", "."], Some("007.500"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "7.500");
    Ok(())
}

/// An all-zero leading-zero run (`00`) and a plain leading-zero integer
/// (`007`) still strip down to their correct, minimal spelling -- not
/// regressed by the new exponent/trailing-zero handling above.
#[test]
fn test_document_input_leading_zero_plain_and_all_zero_unaffected_1149() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", "."], Some("007"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "7");

    let (stdout, _, code) = run_jq_full(&["-c", "."], Some("00"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "0");
    Ok(())
}

/// A leading-zero number nested inside a container, via plain document
/// input, gets the same spelling fix as a bare top-level scalar.
#[test]
fn test_document_input_nested_leading_zero_exponent_spelling_preserved_1149() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", "."], Some(r#"{"a":007e5,"b":007.500}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), r#"{"a":7E+5,"b":7.500}"#);
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

// =============================================================================
// #1093: `--slurpfile`/`--seq` preserve number-literal source fidelity the
// same way `--argjson` does (#1058), instead of round-tripping through
// `serde_json::Value`'s own `f64`/`i64` `Display`. All cases live-verified
// against jq 1.7.1.
// =============================================================================

#[test]
fn test_slurpfile_preserves_number_literal_fidelity_1093() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, "1.500")?;

    let (stdout, _, code) = run_jq_full(
        &[
            "-cn",
            "--slurpfile",
            "n",
            file.path().to_str().unwrap(),
            "$n",
        ],
        None,
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1.500]");
    Ok(())
}

/// Multiple values in one slurped file, one of which needs fidelity
/// preservation -- confirms each value's own byte span is computed
/// correctly, not just the first/only one.
#[test]
fn test_slurpfile_multiple_values_preserve_fidelity_1093() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, r#"{{"a":1}}"#)?;
    writeln!(file, r#"{{"b":1.500}}"#)?;

    let (stdout, _, code) = run_jq_full(
        &[
            "-cn",
            "--slurpfile",
            "n",
            file.path().to_str().unwrap(),
            "$n",
        ],
        None,
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#"[{"a":1},{"b":1.500}]"#);
    Ok(())
}

/// Values separated by extra whitespace/blank lines -- confirms the
/// leading-whitespace-skip when computing each value's start offset is
/// correct, not just the common single-newline-separated case.
#[test]
fn test_slurpfile_whitespace_between_values_1093() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, "  {{\"a\":1}}   ")?;
    writeln!(file)?;
    writeln!(file, "  {{\"b\":1.500}}  ")?;

    let (stdout, _, code) = run_jq_full(
        &[
            "-cn",
            "--slurpfile",
            "n",
            file.path().to_str().unwrap(),
            "$n",
        ],
        None,
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#"[{"a":1},{"b":1.500}]"#);
    Ok(())
}

/// Genuinely malformed content is still rejected -- fidelity preservation
/// doesn't widen what `--slurpfile` accepts.
#[test]
fn test_slurpfile_malformed_content_still_rejected_1093() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, "not json")?;

    let (_, _, code) = run_jq_full(
        &[
            "-cn",
            "--slurpfile",
            "n",
            file.path().to_str().unwrap(),
            "$n",
        ],
        None,
    )?;
    assert_ne!(code, 0);
    Ok(())
}

/// `--seq`'s own output format prefixes each value with the same RS
/// (`\x1e`) byte its input uses (RFC 7464), unrelated to this fix -- kept
/// in the expected string, not stripped, to pin the byte-exact output.
#[test]
fn test_seq_preserves_number_literal_fidelity_1093() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["--seq", "-c", "."], Some("\x1e1.500\n"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "\x1e1.500");
    Ok(())
}

/// Multiple RS-separated values, each independently isolated and
/// validated -- confirms fidelity preservation applies per-segment, not
/// just to a single value.
#[test]
fn test_seq_multiple_values_preserve_fidelity_1093() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["--seq", "-c", "."], Some("\x1e1.500\n\x1e2.000\n"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "\x1e1.500\n\x1e2.000");
    Ok(())
}

/// A malformed segment is silently skipped (RFC 7464's own recommendation,
/// unchanged by this fix) while the surrounding valid segments still
/// preserve their fidelity correctly.
#[test]
fn test_seq_malformed_segment_silently_skipped_1093() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["--seq", "-c", "."],
        Some("\x1e1.500\n\x1egarbage\n\x1e2.000\n"),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "\x1e1.500\n\x1e2.000");
    Ok(())
}

// =============================================================================
// #1243: `--slurpfile`/`--slurp`/`--seq` tolerate a leading-zero number the
// same way document input and `--argjson` already do (#1094/#1149), instead
// of erroring outright (`--slurpfile`/`--slurp`) or silently dropping the
// record (`--seq`). All cases live-verified against jq 1.7.1.
// =============================================================================

#[test]
fn test_slurpfile_tolerates_leading_zero_number_1243() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, "007e5")?;

    let (stdout, _, code) = run_jq_full(
        &[
            "-cn",
            "--slurpfile",
            "n",
            file.path().to_str().unwrap(),
            "$n",
        ],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "[7E+5]");
    Ok(())
}

/// A leading-zero number document with plain `--slurp` (not `--slurpfile`)
/// hit the same over-strict `serde_json::Deserializer` validation, found
/// while scoping this fix -- the primary document-input path falls onto
/// this function's slower fallback whenever `--slurp` is set (bypassing the
/// fast lazy path), not just for `--slurpfile`'s own argument file.
#[test]
fn test_slurp_flag_tolerates_leading_zero_number_1243() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", "--slurp", "."], Some("007e5"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "[7E+5]");
    Ok(())
}

#[test]
fn test_seq_tolerates_leading_zero_number_1243() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["--seq", "-c", "."], Some("\x1e007e5\n"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "\x1e7E+5");
    Ok(())
}

/// #1267: swapping `--seq`'s per-record validation to the crate's own
/// zero-allocation grammar validator (performance-motivated) also fixed a
/// real correctness divergence discovered while verifying the swap's
/// grammar equivalence against `serde_json`: a magnitude-overflowing float
/// literal (`1e400`) real jq itself accepts on ordinary document input
/// (`1E+400`, live-verified) used to silently drop as an "unparseable"
/// record here instead, because the old `serde_json::Value`-based
/// validator rejects anything that doesn't fit in a finite `f64`. The new
/// validator is a pure grammar check with no such range rejection, so this
/// now materializes correctly, matching real jq.
#[test]
fn test_seq_accepts_magnitude_overflowing_number_1267() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["--seq", "-c", "."], Some("\x1e1e400\n"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "\x1e1E+400");
    Ok(())
}

/// Control: a genuinely malformed record is still silently dropped after
/// the validator swap -- the new validator isn't accidentally more lenient
/// on syntax, only on numeric magnitude.
#[test]
fn test_seq_still_drops_genuinely_malformed_record_1267() -> Result<()> {
    let (stdout, _, code) =
        run_jq_full(&["--seq", "-c", "."], Some("\x1enot valid json\n\x1e5\n"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim_end(), "\x1e5");
    Ok(())
}

/// A hyphen-prefixed `--slurpfile`/`--rawfile` FILE value must reach this
/// crate's own file-open logic, not get rejected by clap as an unknown
/// flag first (#1150, same `allow_hyphen_values` fix as `--arg`/
/// `--argjson`). Uses a nonexistent filename -- the point is only to
/// distinguish clap's "unexpected argument" rejection (the pre-fix
/// failure mode) from this crate's own "no such file" error (proof the
/// value was accepted and passed through), not to exercise real file I/O.
#[test]
fn test_slurpfile_rawfile_hyphen_prefixed_filename_reaches_file_open_1150() -> Result<()> {
    let (_, stderr, code) =
        run_jq_full(&["-n", "--slurpfile", "n", "-nonexistent.json", "$n"], None)?;
    assert_ne!(code, 0);
    assert!(
        !stderr.contains("unexpected argument"),
        "clap should not reject the hyphen-prefixed value: {stderr}"
    );

    let (_, stderr, code) = run_jq_full(&["-n", "--rawfile", "n", "-nonexistent.txt", "$n"], None)?;
    assert_ne!(code, 0);
    assert!(
        !stderr.contains("unexpected argument"),
        "clap should not reject the hyphen-prefixed value: {stderr}"
    );
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
    // Integer literals beyond i64 degrade to floats *numerically* (issue
    // #166), but #1035 keeps the literal's own source spelling through
    // evaluation, matching real jq: `jq -n '9999999999999999999'` prints
    // the digit string back verbatim, not a rounded
    // `10000000000000000000` (verified against jq 1.7.1 -- the comment
    // this test previously carried claiming otherwise was never actually
    // checked against a live oracle).
    let (output, code) = run_jq_stdin("9999999999999999999", "null", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "9999999999999999999");
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
fn test_stream_operator_truthy_retain_collapses_borrowed_many_to_every_arity_1038() -> Result<()> {
    // `//`'s truthy-retain over a *borrowed* multi-output stream (`.[]`, not
    // a constructed/owned one) collapses through `borrowed_vec_to_result`
    // (#1038) -- pin all three arities its match covers: zero truthy values
    // left, exactly one, and more than one.
    for (filter, input, expected) in [
        (r#".[] // "backup""#, "[false,null]", "\"backup\"\n"),
        (r#".[] // "backup""#, "[false,1,null]", "1\n"),
        (r#".[] // "backup""#, "[false,1,null,2]", "1\n2\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(code, 0, "`{filter}` on {input} should succeed");
        assert_eq!(stdout, expected, "wrong output for `{filter}` on {input}");
    }
    Ok(())
}

#[test]
fn test_limit_collapses_borrowed_many_to_every_arity_1038() -> Result<()> {
    // `limit`'s `Many` arm collapses its taken prefix through
    // `borrowed_vec_to_result` (#1038) -- pin both reachable arities (one
    // taken, more than one) from a *borrowed* multi-output stream (`.[]`).
    // `limit(0; ...)` takes its own dedicated early return before `expr` is
    // even evaluated, so it never reaches the `Many` arm at all; included
    // anyway as ordinary black-box coverage of `limit`'s zero-output case.
    for (filter, expected) in [
        ("[limit(0; .[])]", "[]\n"),
        ("[limit(1; .[])]", "[1]\n"),
        ("[limit(2; .[])]", "[1,2]\n"),
    ] {
        let (stdout, code) = run_jq_stdin(filter, "[1,2,3]", &["-c"])?;
        assert_eq!(code, 0, "`{filter}` should succeed");
        assert_eq!(stdout, expected, "wrong output for `{filter}`");
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

/// #1213: `--seq`'s error-location reporting stays correct once its
/// per-value line lookup is incremental (`LineCounter`) instead of a
/// from-scratch rescan per value -- the erroring record here is deep enough
/// into the stream (record 50 of 100) that an off-by-one in the incremental
/// bookkeeping would show up as a wrong line number, not just a wrong
/// answer.
#[test]
fn test_seq_error_location_correct_with_many_preceding_records_1213() -> Result<()> {
    let mut input = String::new();
    for i in 0..100 {
        input.push('\u{1e}');
        input.push_str(&format!("{{\"n\":{i}}}\n"));
    }
    let (_, stderr, code) = run_jq_full(
        &["--seq", r#"if .n==50 then error("boom") else . end"#],
        Some(&input),
    )?;
    assert_eq!(code, 5, "stderr: {stderr}");
    // Record 50 (0-indexed) is the 51st RS-delimited record, ending on the
    // 51st line of the input.
    assert!(stderr.contains("(at <stdin>:51): boom"), "stderr: {stderr}");
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

/// Companion to `test_isempty_propagates_halt_instead_of_answering_false`,
/// for `break` (#867 follow-up): the same "zero prior outputs must
/// propagate" reasoning applies to a bare `Break`, not just `Halt`.
/// Verified against jq 1.7.1: `label $out | isempty(break $out), "after"`
/// produces no output and exits 0.
#[test]
fn test_isempty_propagates_bare_break_to_outer_label() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "label $out | isempty(break $out), \"after\""], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// The asymmetric counterpart: unlike the bare-break case above, a `break`
/// that only surfaces *after* `g` already produced an output must NOT
/// propagate — real jq's `isempty` is defined as `label $out | (g|false,
/// break $out), true`, so `g`'s second output (the `break`) is never even
/// requested once its first output already answered `isempty`'s own
/// internal `break $out`. Verified against jq 1.7.1: `isempty(1, break
/// $out)` answers `false` then prints `"after"`, exiting 0 — the exact
/// same shape as `isempty(1, halt_error(3))` staying `false` above it in
/// this file. This must keep passing after the bare-break fix; a
/// mechanical "propagate every Break/Partial(_, Break)" fix (mirroring
/// `isvalid`'s) would have broken this case, which is why `isempty`'s fix
/// only touches the bare `Break` arm, not `Partial(_, Control::Break(_))`.
#[test]
fn test_isempty_break_after_partial_output_does_not_propagate() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "label $out | isempty(1, break $out), \"after\""],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "false\n\"after\"\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// `builtin_isempty`'s bare `QueryResult::Error(_)` arm used to answer
/// `true` unconditionally ("errors count as empty"), swallowing a genuine
/// uncaught error instead of propagating it (#882). Real jq's `isempty` has
/// no `try`/`catch` around its argument, so an uncaught error must fail the
/// whole evaluation. Verified against jq 1.7.1: `jq -n
/// 'isempty(error("x"))'` exits 5 with no output.
#[test]
fn test_isempty_propagates_bare_error_instead_of_answering_true() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "isempty(error(\"x\")), \"after\""], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("jq: error"), "{stderr}");
    assert!(stderr.contains('x'), "{stderr}");
    Ok(())
}

/// The asymmetric counterpart, mirroring `test_isempty_break_after_partial_output_does_not_propagate`:
/// an error surfacing *after* `g` already produced an output must NOT
/// propagate, for the same laziness reason - real jq's `isempty` never
/// requests `g`'s second output once the first already answered its own
/// internal `break $out`. Verified against jq 1.7.1: `isempty(1,
/// error("x"))` answers `false` then prints `"after"`, exiting 0. This must
/// keep passing after the bare-error fix; a mechanical "propagate every
/// Error/Partial(_, Error)" fix would have broken this case.
#[test]
fn test_isempty_error_after_partial_output_does_not_propagate() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "isempty(1, error(\"x\")), \"after\""], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "false\n\"after\"\n");
    assert_eq!(stderr, "");
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

/// Companion to `test_delpaths_propagates_halt_in_paths_argument`, for a
/// bare `break` (#867 follow-up): must keep unwinding, not be misreported
/// as "Paths must be specified as an array". Verified against jq 1.7.1:
/// `label $out | delpaths(break $out), "after"` produces no output and
/// exits 0.
#[test]
fn test_delpaths_propagates_bare_break_to_outer_label() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "label $out | delpaths(break $out), \"after\""],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// The asymmetric counterpart, mirroring `isempty`'s equivalent pair of
/// tests: unlike the bare-break case above, a `break` surfacing *after*
/// `paths_expr` already produced a value must NOT propagate. Real jq's
/// `delpaths` only ever demands `paths_expr`'s first output, so a
/// non-array first value raises immediately without ever reaching a later
/// comma branch. Verified against jq 1.7.1: `delpaths(1, break $out)`
/// raises "Paths must be specified as an array", the same error the bare
/// non-array case already produces — it never reaches the break.
#[test]
fn test_delpaths_break_after_partial_output_does_not_propagate() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "label $out | delpaths(1, break $out), \"after\""],
        None,
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Paths must be specified as an array"),
        "{stderr}"
    );
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
    // A distinct site from the one above: `eval_error`'s own guard for its
    // message expression specifically matches `Err(EvalEscape::Error(_))`,
    // not a bare `Err(_)`, so a halt reached while evaluating `error(msg)`'s
    // message expression always falls through to `Err(escape) =>
    // escape.into()` and propagates -- regardless of whether `optional` is
    // `true` (isvalid's old forced broadcast) or `false` (isvalid's current
    // ambient evaluation, #881). Fixing `isvalid`'s own wildcard alone does
    // not fix this one; both fixes are independent.
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
fn test_range_bare_break_reaches_outer_label_833() -> Result<()> {
    // Sibling fix to #833: `range_arg` had the same missing-`Break`-arm
    // shape as `result_to_owned`/`eval_owned_expr` (found auditing for
    // #833's own sibling helpers), falling into the generic "Range bounds
    // must be numeric" wildcard instead of propagating. Matches real jq
    // 1.7.1: `jq -n 'label $out | [range(break $out; 5)]'` exits 0 with no
    // output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "label $out | [range(break $out; 5)]"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_pow_base_bare_break_reaches_outer_label_833() -> Result<()> {
    // Sibling fix to #833: `get_number_from_result`/`NumberError` had the
    // same missing-`Break`-variant shape, shared by `pow`'s base and
    // exponent arguments and `atan2`'s y/x arguments. Matches real jq
    // 1.7.1: `jq -n 'label $out | pow(break $out; 2)'` exits 0, no output.
    let (stdout, stderr, code) = run_jq_full(&["-n", "label $out | pow(break $out; 2)"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_pow_exp_bare_break_reaches_outer_label_833() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "label $out | pow(2; break $out)"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_atan2_y_bare_break_reaches_outer_label_833() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "label $out | atan2(break $out; 1)"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_atan2_x_bare_break_reaches_outer_label_833() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "label $out | atan2(1; break $out)"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_repeat_propagates_halt_instead_of_running_to_iteration_cap() -> Result<()> {
    // A halt was already threaded correctly even before #855 (`eval_repeat`
    // now uses `eval_owned_expr_fork`, not `eval_owned_expr_ctrl`, but both
    // always propagated a trailing `Control::Halt` rather than silencing
    // it) -- this pins that a `repeat` body producing a value and then
    // halting on the *same* round stops immediately rather than looping.
    // `repeat` is a succinctly extension (no upstream jq builtin), so
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
/// of jq's own `DBL_MAX`-text substitution (#561), then as Rust's own
/// `f64::Display` (`"inf"`/`"-inf"`, still wrong -- #1075). `tostring`
/// reaches the fix directly, but `@uri`/`@html`/`@sh`/string
/// interpolation/`@csv` are dispatched through `eval_generic`'s
/// cursor-reindexing bridge, which round-trips the value through JSON text --
/// and used to substitute `"null"` for the overflowed value before the fix
/// ever saw it. This test exercises the real CLI (not `eval.rs::eval()`
/// directly) so it actually covers that bridge.
#[test]
fn test_number_literal_overflow_text_formats_via_cli() -> Result<()> {
    let (output, code) = run_jq_stdin("tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin("tostring", "-1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""-1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin("@uri", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e%2B308""#);

    let (output, code) = run_jq_stdin("@html", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin("@sh", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin(r#""\(.)""#, "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin("@csv", "[1e400]", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    Ok(())
}

/// The CLI's identity/raw-print path is a separate code path from the jq
/// evaluator (it prints source number bytes straight through
/// `format_number_jq_compat`), and had the same overflow-renders-as-garbage
/// bug (#561): unlike JSON output's established "NaN/Infinity -> null"
/// convention (`OwnedValue::to_json`), it printed
/// `"NaNE+2147483647"` for `1e400 | .` instead of `null`. #561's "null"
/// fallback was itself superseded by #1087: `format_number_jq_compat`
/// already reformats a non-finite input's mantissa correctly
/// (`format_overflow_literal_mantissa`, added by #930, after #561 landed),
/// so a *literal* overflow now echoes it (`1E+400`) rather than "null" --
/// confirmed live against jq 1.7.1, `1e400 | .` (identity, no computation)
/// echoes `1E+400` too. "null" remains correct only for a genuinely
/// *computed* Infinity with no source text of its own (see
/// `test_jq_infinite_direct_json_output_matches_real_jq_1087`, where it's
/// `DBL_MAX` text instead), and for NaN in both cases (no fallback text
/// exists for NaN in real jq either).
#[test]
fn test_number_literal_overflow_identity_echoes_mantissa_via_cli_1087() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "1e400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1E+400");

    let (output, code) = run_jq_stdin(".", "-1e400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "-1E+400");

    Ok(())
}

/// #1099: the symmetric *underflow* case (a literal whose magnitude
/// underflows `f64` to exactly `0.0`, e.g. `1e-400`) used to lose the
/// mantissa entirely (`0E-400` instead of `1E-400`) -- `value == 0.0` can't
/// tell a genuinely-zero-mantissa literal apart from a nonzero one that
/// simply underflowed, so `format_number_jq_compat` used to always spell
/// the mantissa as `"0"`. Verified live against jq 1.7.1.
#[test]
fn test_number_literal_underflow_preserves_mantissa_via_cli_1099() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "1e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1E-400");

    let (output, code) = run_jq_stdin(".", "-1e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "-1E-400");

    let (output, code) = run_jq_stdin(".", "12.34e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1.234E-399");

    let (output, code) = run_jq_stdin(".", "0.5e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "5E-401");

    let (output, code) = run_jq_stdin(".", "100.5e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1.005E-398");

    Ok(())
}

/// A genuinely-zero-mantissa literal at an extreme negative exponent is
/// unaffected by #1099's fix -- regression guard alongside the nonzero
/// case above.
#[test]
fn test_number_literal_zero_mantissa_extreme_exponent_still_zero_1099() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "0e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0E-400");

    let (output, code) = run_jq_stdin(".", "-0e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "-0E-400");

    Ok(())
}

/// #1178: a genuinely-zero-mantissa literal's printed exponent must still
/// shift by the fractional-zero-digit count, the same normalization real
/// jq applies to a nonzero mantissa -- #1099's own zero-mantissa test above
/// only covers a no-fraction spelling (`0e-400`, shift 0), which is why
/// this gap wasn't caught by that PR's suite. Live-verified against jq
/// 1.7.1.
#[test]
fn test_number_literal_zero_mantissa_exponent_shifts_by_fraction_length_1178() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "0.000e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0E-403");

    let (output, code) = run_jq_stdin(".", "0.0e400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0E+399");

    let (output, code) = run_jq_stdin(".", "0.00e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0E-402");

    Ok(())
}

/// The sign is preserved through the shift, and `-0.00e-400` shifts
/// identically to the unsigned case above -- confirmed live against jq
/// 1.7.1 (`-0e5` also still unaffected: shift is 0 when there's no
/// fraction, matching #1099's existing `-0e-400` test above).
#[test]
fn test_number_literal_negative_zero_mantissa_exponent_shifts_1178() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "-0.00e-400", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "-0E-402");

    Ok(())
}

/// Unlike overflow (which falls back to `DBL_MAX` text past a documented
/// exponent-magnitude ceiling), this crate imposes no *deliberate* ceiling
/// on underflow -- verified live against jq 1.7.1: `1e-1000000000`
/// (exponent magnitude *at* the overflow ceiling) still prints the literal
/// mantissa unchanged. (Real jq itself breaks down for magnitudes beyond
/// ~1,147,483,647 -- an apparent internal bug in its own decNumber, not
/// replicated here; see `format_underflow_literal_mantissa`'s doc comment.)
#[test]
fn test_number_literal_underflow_has_no_ceiling_unlike_overflow_1099() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "1e-1000000000", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1E-1000000000");

    Ok(())
}

/// Code review finding on #1099's own PR: the exponent digit string was
/// parsed at `i32` width for dispatch (`exp == 0` / `(-5..0)` fast-path
/// checks), and an out-of-`i32`-range exponent silently became `0` via
/// `.unwrap_or(0)` -- misrouting into the "eliminate exponent" fast path
/// *before* the mantissa-preserving logic above ever ran, reintroducing
/// #1099's exact original symptom one exponent-digit past `i32::MIN`
/// (`-2147483648`). Verified live: real jq already falls into its own
/// ~1.147B breakdown by this magnitude (see the no-ceiling test above), so
/// this is a succinctly-only boundary, not oracle-comparable.
#[test]
fn test_number_literal_underflow_beyond_i32_exponent_range_1099() -> Result<()> {
    // One exponent digit past i32::MIN (-2147483648) -- used to silently
    // dispatch through `exp == 0` and print bare `0`, losing sign,
    // mantissa, and exponent entirely.
    let (output, code) = run_jq_stdin(".", "1e-2147483649", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1E-2147483649");

    let (output, code) = run_jq_stdin(".", "-1e-2147483649", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "-1E-2147483649");

    // Right at the (now-irrelevant) old i32 boundary -- must stay correct
    // too, not just the one-past case.
    let (output, code) = run_jq_stdin(".", "1e-2147483648", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1E-2147483648");

    // Same bug, zero-mantissa side: a genuinely-zero mantissa at an
    // out-of-i32-range exponent used to also mis-dispatch through
    // `exp == 0`, wrongly eliminating (rather than preserving) the huge
    // exponent.
    let (output, code) = run_jq_stdin(".", "0e-2147483649", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0E-2147483649");

    Ok(())
}

/// #1177: a literal that parses to a nonzero but *subnormal* `f64` (below
/// `f64::MIN_POSITIVE`, but still representable) used to render its
/// mantissa as the literal text `"inf"` -- not valid JSON.
/// `libm::pow(10.0, log10(abs_value).floor())` itself underflows to `0.0`
/// at the extreme low end of the subnormal range, making
/// `abs_value / 0.0 = +inf`. Verified live against jq 1.7.1.
///
/// CLI-level counterpart to `value.rs`'s
/// `test_format_number_jq_compat_subnormal_preserves_mantissa_1177` (same
/// #1099-established pattern: `run_jq_stdin` spawns a subprocess, invisible
/// to `cargo llvm-cov`, so both an in-process and a CLI-level test exist
/// for the same fix rather than one being redundant with the other).
#[test]
fn test_number_literal_subnormal_preserves_mantissa_1177() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "5e-324", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "5E-324");

    let (output, code) = run_jq_stdin(".", "-5e-324", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "-5E-324");

    let (output, code) = run_jq_stdin(".", "4.9e-324", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "4.9E-324");

    let (output, code) = run_jq_stdin(".", "1e-315", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1E-315");

    Ok(())
}

/// CLI-level counterpart to
/// `test_format_number_jq_compat_scientific_notation_preserves_trailing_zeros_1206`
/// (`run_jq_stdin` spawns `cargo run`, invisible to `cargo llvm-cov`, so both
/// an in-process and a CLI-level test exist for the same fix).
#[test]
fn test_number_literal_scientific_notation_preserves_trailing_zeros_via_cli_1206() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "1.50e10", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1.50E+10");

    let (output, code) = run_jq_stdin(".", "3.000e100", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "3.000E+100");

    Ok(())
}

/// CLI-level counterpart to
/// `test_format_number_jq_compat_scientific_notation_mantissa_stays_below_ten_1206`.
#[test]
fn test_number_literal_scientific_notation_mantissa_stays_below_ten_via_cli_1206() -> Result<()> {
    let (output, code) = run_jq_stdin(".", "9.9999999999999e-64", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "9.9999999999999E-64");

    let (output, code) = run_jq_stdin(".", "9.999999999999999e300", &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "9.999999999999999E+300");

    Ok(())
}

/// `eval_owned_expr`/`eval_owned_input` (backing `reduce`/`foreach`/`as $x`
/// variable binding) and `with_entries`'s `owned_to_json_bytes` each have
/// their own serialize-and-reparse bridge, separate from `eval_generic`'s
/// (already covered by `test_number_literal_overflow_text_formats_via_cli`).
/// Before switching these to `to_json_for_reindex`, they too silently turned
/// an overflowed `NumberLiteral` into JSON `null`, so `. as $x | $x |
/// tostring` printed `"null"` instead of jq's `DBL_MAX`-text substitution
/// even though a direct `tostring` was already fixed (#561, then #1075 for
/// the exact substitution text).
#[test]
fn test_number_literal_overflow_owned_reindex_bridges_via_cli() -> Result<()> {
    let (output, code) = run_jq_stdin(". as $x | $x | tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin(". as $x | $x | tostring", "-1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""-1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin("reduce (1) as $x (.; .) | tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin("foreach (1) as $x (.; .) | tostring", "1e400", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.7976931348623157e+308""#);

    let (output, code) = run_jq_stdin(
        "with_entries(.value |= (. | tostring))",
        r#"{"a":1e400}"#,
        &["-c"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":"1.7976931348623157e+308"}"#);

    Ok(())
}

/// #939: the same reindex bridges as the test above, but previewing the
/// overflowed value itself (via `keys`) rather than converting it with
/// `tostring`. `to_json_for_reindex` substituted a generic `1e999`/`-1e999`
/// sentinel for *every* infinite `NumberLiteral`, discarding a document-
/// sourced overflow literal's own text before #930's `describe()` fix ever
/// got a chance to reformat it - so every such literal previewed
/// identically regardless of its actual magnitude, e.g. `123e400` and
/// `9e400` both showed `1E+999`. Oracle-verified: `reduce`/`with_entries`
/// now reuse the literal's own text (matching real jq exactly), since it's
/// already valid JSON number syntax guaranteed to reparse to this value.
#[test]
fn test_document_overflow_literal_keys_preview_via_reindex_bridges_cli_939() -> Result<()> {
    let (output, code) = run_jq_stdin(
        "try (reduce (1) as $x (.; .) | keys) catch .",
        "123e400",
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""number (1.23E+402) has no keys""#);

    let (output, code) = run_jq_stdin(
        "try (reduce (1) as $x (.; .) | keys) catch .",
        "-1e400",
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""number (-1E+400) has no keys""#);

    let (output, code) = run_jq_stdin(
        "try with_entries(.value |= (. | keys)) catch .",
        r#"{"a":12.34e400}"#,
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""number (1.234E+401) has no keys""#);

    Ok(())
}

// ============================================================================
// #945: `. as $x | ...`/`literal as $x | ...` used to be suspected of
// dropping a document literal's own source-text spelling (e.g. `1.50`
// rendering back as `1.5`) once bound to `$x`. Re-verified post-merge
// (2026-08-21): every shape below already matches jq 1.7.1 exactly on
// `main` -- the fix was an incidental side effect of #1035/#1062 adding
// `Literal::NumberLiteral(NumberRepr, String)` (mirroring
// `OwnedValue::NumberLiteral`), not a deliberate fix for this issue, so
// nothing previously guarded it. Pinned here per that comment's own
// recommendation, before closing #945.
// ============================================================================

/// #945: a document-sourced float literal keeps its own spelling (`1.50`,
/// not the collapsed `1.5`) once bound via `as $x`, across several
/// consuming contexts -- plain re-emission, `tostring`, array/object
/// construction, and `tojson` -- all oracle-verified.
#[test]
fn test_as_binding_preserves_document_float_literal_spelling_945() -> Result<()> {
    // plain re-emission
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | $x"], Some("1.50"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1.50");

    // tostring
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | $x | tostring"], Some("1.50"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "\"1.50\"");

    // array construction
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | [$x]"], Some("1.50"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1.50]");

    // object construction
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | {v:$x}"], Some("1.50"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "{\"v\":1.50}");

    // tojson
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | ($x|tojson)"], Some("1.50"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "\"1.50\"");

    Ok(())
}

/// #945: arithmetic on a bound literal correctly *collapses* its spelling
/// (`1.50 + 0` becomes plain `1.5`, matching jq) -- the fix preserves
/// source spelling only through pass-through contexts, not through actual
/// computation, so this must not regress into over-preserving.
#[test]
fn test_as_binding_arithmetic_collapses_literal_spelling_945() -> Result<()> {
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | $x + 0"], Some("1.50"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1.5");
    Ok(())
}

/// #945: an exponent-spelled literal (`1e2`, not `1.50`) is preserved the
/// same way, both bare and inside an array -- the fix isn't specific to
/// decimal-point spellings.
#[test]
fn test_as_binding_preserves_exponent_literal_spelling_945() -> Result<()> {
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | $x"], Some("1e2"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1E+2");

    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as $x | [$x]"], Some("1e2"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1E+2]");
    Ok(())
}

/// #945: a *query*-literal binding (`1.50 as $x | $x`, the value coming
/// from the jq program text itself, not the input document) preserves
/// spelling the same way as a document-sourced one above -- the
/// "query-literal spelling loss" gap the 2026-08-21 comment flagged as
/// likely-should-be-bundled turned out to already be closed too.
#[test]
fn test_as_binding_preserves_query_literal_spelling_945() -> Result<()> {
    let (stdout, _stderr, code) = run_jq_full(&["-c", "1.50 as $x | $x"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1.50");
    Ok(())
}

/// #945: binding a non-iterable literal and then trying to iterate it
/// still raises jq's own "Cannot iterate over number" error, unaffected
/// by the spelling-preservation fix -- confirms `as $x` still type-checks
/// normally, this isn't a blanket "numbers become opaque" change.
#[test]
fn test_as_binding_non_iterable_literal_still_raises_945() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", "try (. as $x | $x[]) catch ."], Some("1.50"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "\"Cannot iterate over number (1.50)\"");
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

/// `needs_path_context` had no `Expr::Array` arm (#1302), so `[key]` --
/// semantically equivalent to `(key,key)`'s single-branch case, just
/// array-wrapped instead of comma-joined -- silently lost path context and
/// stubbed to `null`, even though `needs_path_context` already recursed
/// into the structurally-identical `Comma` wrapper. Fixing just the
/// recursion wasn't enough on its own: `eval_pipe_with_path_context_internal`'s
/// array-construction arm built the array via the plain, context-less
/// evaluator, so `key`/`parent`/`file_index` nested inside `[...]` needed a
/// second, dedicated fix to route through the path-context evaluator during
/// construction itself. Verified live against real yq v4.53.3 for `[key]`
/// and `[parent]` (`key`/`parent` have no real-jq equivalent -- succinctly
/// extensions -- so jq mode here pins internal consistency, not an oracle).
#[test]
fn test_array_wrapped_path_context_builtins_1302() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ".a | [key]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"["a"]"#);

    let (stdout, _, code) = run_jq_full(&["-c", ".a | [parent]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"[{"a":1}]"#);

    // Multiple path-context builtins in the same array literal, plus a
    // plain value alongside them -- all outputs collected into one array,
    // matching `[...]`'s ordinary multi-output-collection semantics.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | [key, key, parent]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"["a","a",{"a":1}]"#);

    // Nested inside a longer pipe: `rest` after the array must still see
    // the newly constructed array as its fresh root (path reset), matching
    // the plain `Expr::Array`/`Expr::Object`/`Expr::Literal` arm's own
    // "reset path and root to the new value" behavior.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | [key] | .[0]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""a""#);

    // A plain array (no path-context builtin inside) takes the original,
    // unmodified code path and must be entirely unaffected.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | [1,2,3]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[1,2,3]");

    Ok(())
}

/// Exercises the non-`Owned`/`ManyOwned` arms of #1302's new inner-result
/// match (`None`/`Error`/`Break`/`Halt`/`Partial`) -- array construction is
/// atomic, so each of these must abandon the whole array rather than
/// producing a partial one, mirroring `eval_array_construction`'s identical
/// reasoning for the plain (non-path-context) case.
#[test]
fn test_array_wrapped_path_context_builtin_control_flow_1302() -> Result<()> {
    // Zero output from the inner expression (`select` with a false
    // condition on `key`) -> an empty array, not an error.
    let (stdout, _, code) =
        run_jq_full(&["-c", ".a | [select(key == \"z\")]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "[]");

    // An error from the inner expression, as the pipe's very first output
    // (no successful output preceding it), aborts the whole array
    // construction as a bare error.
    let (_stdout, stderr, code) =
        run_jq_full(&["-c", ".a | [key | error(\"boom\")]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5);
    assert!(stderr.contains("boom"), "stderr: {stderr}");

    // Same, but with real output already produced by an earlier comma
    // branch (`key` succeeds with "a" before `error` fires) -- exercises
    // the `Partial(_, Control::Error(_))` arm rather than the bare one
    // above. Still discards the whole array, matching jq's atomic array
    // construction.
    let (stdout, _, code) =
        run_jq_full(&["-c", ".a | [key, error(\"boom\")]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5);
    assert_eq!(stdout, "");

    // A bare `break` out of the inner expression (pipe form, no prior
    // output) aborts the whole array construction and unwinds past it to
    // the enclosing label -- the comma's second branch ("reached") must
    // never run.
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "label $out | ((.a | [key | break $out]), \"reached\")",
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "",
        "break must discard the whole array, not emit it"
    );

    // Same, but as a comma branch with real output already produced first
    // (`key` succeeds with "a" before `break` fires) -- exercises the
    // `Partial(_, Control::Break(_))` arm rather than the bare one above.
    let (stdout, _, code) = run_jq_full(
        &["-c", "label $out | ((.a | [key, break $out]), \"reached\")"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "");

    // `halt_error(n)` from the inner expression, as the pipe's very first
    // output (no successful output preceding it), propagates its own exit
    // code as a bare halt.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | [key | halt_error(9)]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 9);
    assert_eq!(stdout, "");

    // Same, but with real output already produced by an earlier comma
    // branch -- exercises the `Partial(_, Control::Halt(_))` arm rather
    // than the bare one above.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | [key, halt_error(9)]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 9);
    assert_eq!(stdout, "");

    Ok(())
}

/// Code review on #1302's own PR (#1333): the new array arm originally
/// passed the ambient `optional` straight into evaluating the array's
/// inner expression, letting a leaf error deep inside self-swallow via its
/// own local `if optional` check before ever reaching the array's own
/// atomicity match -- corrupting `[key, error("boom")]?` into a partial
/// array (`["a"]`) instead of the whole construction being caught, unlike
/// real jq's structurally identical `[1, error("x")]?` (empty output).
/// Verified live against real jq for the baseline comparison.
#[test]
fn test_array_wrapped_path_context_builtin_optional_is_atomic_1302() -> Result<()> {
    // A comma branch that succeeds (`key`) before a later branch errors:
    // `?` must discard the whole array, not just the erroring branch.
    let (stdout, _, code) =
        run_jq_full(&["-c", ".a | [key, error(\"boom\")]?"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "");

    // Same shape via a genuine type error instead of an explicit `error()`.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | [key, 1 + \"x\"]?"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "");

    // An error as the array's *only* content: must produce no output at
    // all, not a spurious `[]` (which would conflate a caught error with
    // #1280's genuine-zero-output-generator case).
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a | [(key | error(\"boom\"))]?"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "");

    // Without `?`, the same query still hard-errors -- the fix must not
    // accidentally make every error inside such an array silently vanish.
    let (_stdout, stderr, code) =
        run_jq_full(&["-c", ".a | [key, error(\"boom\")]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5);
    assert!(stderr.contains("boom"), "stderr: {stderr}");

    // A genuinely nested `?` *inside* the array (not on the array itself)
    // must still work on its own terms, independent of the array's own
    // (here absent) `?`.
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a | [key, (error(\"boom\"))?]"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"["a"]"#);

    Ok(())
}

/// `needs_path_context` had no `Expr::StringInterpolation` arm (#1334), the
/// same gap #1302 fixed for `Expr::Array` -- so `"k=\(key)"` silently
/// stubbed `key` to `null` inside a `\(...)` slot, even once
/// `needs_path_context` recursed into the slot correctly. #1302's own
/// two-part fix (recurse in `needs_path_context`, *and* route the slot's own
/// evaluation through the path-context evaluator during construction) is
/// the exact template followed here. Verified live for internal consistency
/// (`key`/`parent` are succinctly extensions, no oracle).
#[test]
fn test_string_interpolation_path_context_builtins_1334() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ".a | \"k=\\(key)\""], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""k=a""#);

    // Multiple slots, plus literal text woven between them.
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a | \"[\\(key)] parent=\\(parent)\""],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""[a] parent={\"a\":1}""#);

    // Nested inside a longer pipe: `rest` after the string must see the
    // newly constructed string as its fresh root (path reset), mirroring
    // #1302's identical array-literal assertion.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | \"\\(key)\" | key"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "null");

    // A plain string (no path-context builtin inside) takes the original,
    // unmodified code path and must be entirely unaffected.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | \"x\\(1)y\""], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""x1y""#);

    Ok(())
}

/// Exercises the non-`Owned` arms of the new `StringInterpolation` slot
/// match -- mirrors #1302's identical `Array`-arm coverage
/// (`test_array_wrapped_path_context_builtin_control_flow_1302`), one slot
/// at a time. A slot construction is atomic *per slot*, not across the
/// whole string: unlike `Array`'s single collection point, each `\(...)`
/// slot embeds independently, so `break`/`halt`/an uncaught error from any
/// one slot still aborts the entire surrounding string (nothing partial is
/// ever embedded), matching `eval_string_interpolation`'s own existing
/// early-return-on-control-signal behavior for the plain (non-path-context)
/// case.
#[test]
fn test_string_interpolation_path_context_builtin_control_flow_1334() -> Result<()> {
    // A multi-valued slot (`key, key`, a `Comma` of two path-context
    // builtins) -- embeds just the *first* output, matching
    // `eval_string_interpolation`'s own existing convention (the
    // `QueryResult::ManyOwned` arm).
    let (stdout, _, code) = run_jq_full(&["-c", ".a | \"[\\(key, key)]\""], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""[a]""#);

    // A bare `break` out of a slot (no prior output within that slot)
    // aborts the whole string construction and unwinds past it to the
    // enclosing label -- the comma's second branch ("reached") must never
    // run.
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "label $out | ((.a | \"\\(key | break $out)\"), \"reached\")",
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(
        code, 0,
        "break must unwind to the label, not surface as an error"
    );
    assert_eq!(
        stdout, "",
        "break must discard the whole string, not emit it"
    );

    // Same, but with real output already produced within that one slot's
    // own generator first (`key` succeeds with "a" before `break` fires) --
    // exercises the `Partial(_, Control::Break(_))` arm rather than the
    // bare one above.
    let (stdout, _, code) = run_jq_full(
        &[
            "-c",
            "label $out | ((.a | \"\\(key, break $out)\"), \"reached\")",
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "");

    // `halt_error(n)` from a slot, as that slot's very first output (no
    // successful output preceding it within the slot), propagates its own
    // exit code as a bare halt.
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a | \"\\(key | halt_error(9))\""],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 9);
    assert_eq!(stdout, "");

    // Same, but with real output already produced within that slot first --
    // exercises the `Partial(_, Control::Halt(_))` arm rather than the bare
    // one above.
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a | \"\\(key, halt_error(9))\""],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 9);
    assert_eq!(stdout, "");

    // A comma-grouped slot where real output is produced before an
    // uncaught error -- exercises the `Partial(_, Control::Error(_))` arm
    // (the un-guarded one, `optional == false`) rather than the bare
    // `Error` arm the atomicity test below already covers.
    let (_stdout, stderr, code) = run_jq_full(
        &["-c", ".a | \"\\(key, error(\"boom\"))\""],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5);
    assert!(stderr.contains("boom"), "stderr: {stderr}");

    Ok(())
}

/// `?`-atomicity for the new `StringInterpolation` arm, built in from the
/// start per #1302's own review lesson (its first cut needed a follow-up,
/// #1333, to force the inner slot's `optional` to `false` so a genuine
/// error surfaces for this arm's own catch instead of self-swallowing at
/// the leaf) rather than left to be independently rediscovered here.
#[test]
fn test_string_interpolation_path_context_builtin_optional_is_atomic_1334() -> Result<()> {
    // A slot that succeeds (`key`) before a later slot errors: `?` must
    // discard the whole string, not embed a partial one.
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a | \"k=\\(key)-\\(error(\"boom\"))\"?"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "");

    // Without `?`, the same query still hard-errors.
    let (_stdout, stderr, code) = run_jq_full(
        &["-c", ".a | \"k=\\(key)-\\(error(\"boom\"))\""],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5);
    assert!(stderr.contains("boom"), "stderr: {stderr}");

    // A genuinely nested `?` *inside* a slot must still work on its own
    // terms, independent of the interpolation's own (here absent) `?`.
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a | \"k=\\(key)-\\((error(\"boom\"))?)\""],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""k=a-""#);

    Ok(())
}

/// `needs_path_context` had no `Expr::FuncDef` arm (#1306), so a `def`
/// scope's own `then` continuation -- even one as simple as `def f: 5; f,
/// key` -- never routed into path-context evaluation at all: the whole
/// pipe fell to the plain evaluator, which has no path tracking, and `key`
/// stubbed to `null` regardless of what preceded the `def`. Recursing into
/// `body`/`then` fixes the *routing* decision; a second, dedicated
/// `eval_pipe_with_path_context_internal` arm was needed too (mirroring
/// #1302's two-part shape again) because the plain evaluator's own
/// `eval_func_def` unconditionally evaluates the expanded `then` via
/// `eval_single`, which would otherwise still drop path context even once
/// routing correctly entered path-context evaluation.
#[test]
fn test_func_def_path_context_builtins_1306() -> Result<()> {
    // The issue's own repro: `key` as a comma sibling to a call, after a
    // `def`.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | def f: 5; f, key"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "5\n\"a\"\n");

    // `key` reached directly through the def's own body, not just a
    // comma sibling in `then` -- the `needs_path_context(body)` half of
    // the fix, not just `needs_path_context(then)`.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | def f: key; f"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""a""#);

    // Nested `def`s, each calling the next -- confirms the fix's recursive
    // expansion (`expand_func_calls`, run again on the freshly expanded
    // tree by this arm re-matching) doesn't stop after one level.
    let (stdout, _, code) = run_jq_full(
        &["-c", ".a.b | def f: key; def g: f; g"],
        Some(r#"{"a":{"b":2}}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""b""#);

    // A plain `def` (no path-context builtin anywhere in body or then)
    // takes the original, unmodified code path and must be unaffected.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | def f: . + 1; f"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "2");

    // A parenthesized `def` with a further pipe stage *after* the closing
    // paren -- `Expr::Paren`'s own arm splices its inner expression (the
    // `FuncDef`) in front of whatever already followed, so this is the one
    // real-syntax shape where the new `FuncDef` arm's own `rest` is
    // non-empty rather than always drained by `then`, exercising its
    // `continue_rest_with_context` branch.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | (def f: 5; f) | key"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#""a""#);

    Ok(())
}

/// Documents a deliberate, narrow gap left open by #1306 rather than
/// silently missed: `needs_path_context`'s new `FuncDef` arm is a cheap
/// syntactic scan of `body`/`then` (no macro expansion), so a path-context
/// builtin reaching the body *only* through a call argument
/// (`def f(x): x; f(key)`) isn't detected -- the argument `key` never
/// appears literally in `body` or `then`'s own text, only after
/// substitution. Not covered by #1306's own repros; pinned here so a
/// regression (this starting to silently resolve, without the routing
/// decision being deliberately revisited) is visible rather than an
/// unnoticed behavior change either way.
#[test]
fn test_func_def_path_context_argument_passing_is_a_known_gap_1306() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ".a | def f(x): x; f(key)"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "null");

    Ok(())
}

/// Documents the other half of #1306's own repro as *not* a bug, contrary
/// to the issue's initial expectation: a comma's branches are independent,
/// each evaluated against the same starting `current_path` the comma
/// itself received -- not threaded from one sibling's own navigation into
/// the next, matching real jq's own comma semantics (confirmed live:
/// `.a.b?, path(.)` on `{"a":{"b":null}}` is `null` then `[]`, not `null`
/// then `["a","b"]` -- `path(.)` in the second branch reflects *its own*
/// lack of navigation, unaffected by what the first branch did). succinctly's
/// own `key` already established and tested this independence
/// (`test_key_survives_comma_branches`, #715: `.[] | (key, key)` on a
/// 2-element array is `["0","0","1","1"]`, not `["0","1","0","1"]` or any
/// other cross-branch-aware shape) before #1306 was filed. `key`/`.a.b?` as
/// top-level comma siblings with nothing preceding them share the same
/// root `current_path` (empty) `key` alone at the root already returns
/// `null` for -- so `null` here is that same, already-correct answer, not
/// a regression of a real bug. See #1306's own comment thread for the
/// full oracle comparison.
#[test]
fn test_func_def_key_comma_sibling_independence_is_not_a_bug_1306() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ".a.b?, key"], Some(r#"{"a":{"b":null}}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "null\nnull\n");

    let (stdout, _, code) = run_jq_full(&["-c", "key, .a.b?"], Some(r#"{"a":{"b":null}}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "null\nnull\n");

    Ok(())
}

/// `Expr::Optional`'s own dispatch in `eval_pipe_with_path_context_internal`
/// broadcast `optional=true` into whatever it wrapped, with no catch of its
/// own -- unlike the plain evaluator's `Expr::Optional` (`eval_try`) and the
/// Array arm's own fix above (#1302). Two distinct symptoms, both from the
/// same code:
///
/// 1. When the `?` had nothing after it in the pipe, a leaf-level `if
///    optional` self-check deep inside the wrapped expression could produce
///    a plausible-but-wrong value instead of this node genuinely catching an
///    escaping error atomically.
/// 2. When the `?` had a `rest` of the pipe after it, `rest` was combined
///    into the *same* list evaluated under the forced `optional=true` --
///    meaning an error *after* the `?`, which the `?` has nothing to do
///    with, was also silently swallowed. `.a | (key)? | error("boom")`
///    produced no output and no error at all, instead of real jq's error
///    (confirmed against real jq's structurally identical
///    `.a | (1)? | error("boom")`).
///
/// #1335's own posted repro (`(key == "a" and error("boom"))?`) turned out
/// not to exercise this arm at all -- `needs_path_context` doesn't recurse
/// into `Expr::And`/`Expr::Or` (filed separately as #1405), so that whole
/// expression routes through the plain evaluator instead, where `key`
/// silently stubs. These tests instead use constructs `needs_path_context`
/// already recurses into (`Comma`, `Compare`, `Label`/`break`) to actually
/// exercise the fixed code path.
#[test]
fn test_optional_dispatch_catches_atomically_not_broadcast_1335() -> Result<()> {
    // The rest-leak: an error *after* the `?` must not be swallowed by it.
    let (_stdout, stderr, code) =
        run_jq_full(&["-c", ".a | (key)? | error(\"boom\")"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5);
    assert!(stderr.contains("boom"), "stderr: {stderr}");

    // Sibling positive case: `rest` still runs normally when nothing errors.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | (key)? | . + \"!\""], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "\"a!\"");

    // A comma branch that succeeds (`key`) before a later branch errors:
    // real jq's `?` keeps the already-produced prefix rather than discarding
    // it (unlike the Array arm's atomicity above -- comma isn't atomic).
    let (stdout, _, code) =
        run_jq_full(&["-c", ".a | (key, error(\"boom\"))?"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "\"a\"");

    // Without `?`, the same query still hard-errors.
    let (_stdout, stderr, code) =
        run_jq_full(&["-c", ".a | key, error(\"boom\")"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5);
    assert!(stderr.contains("boom"), "stderr: {stderr}");

    // `?` also catches a `break` for an enclosing label -- verified against
    // jq 1.7.1: `label $out | (1, break $out)?` prints `1`, exit 0 (both
    // implemented as bare `try`, which catches break the same way it
    // catches a raised error).
    let (stdout, _, code) = run_jq_full(
        &["-c", "label $out | (.a | (key, break $out))?"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "\"a\"");

    // Doubly-nested `?` still resolves correctly (ambient optional=true
    // from the outer `?` reaching the inner one is harmless, not a forced
    // broadcast introduced by this node itself).
    let (stdout, _, code) = run_jq_full(&["-c", ".a | ((key)?)?"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "\"a\"");

    Ok(())
}

/// A `?`-wrapped navigational `inner` (`.[]`/`.foo`) followed by a
/// path-context builtin in `rest` must still thread each output's *own*
/// `current_path`, not the stale pre-`?` one. Isolating `inner`'s evaluation
/// with an empty `rest` (needed to scope the catch above to `inner` alone)
/// discards path updates -- `Field`/`Index`/`Iterate` only compute and
/// thread a new path when they have a non-empty `rest` to recurse into, so
/// evaluated in isolation they just return bare values. Caught during
/// review: naively isolating in every case turned `.a | (.[])? | key` from
/// `0`, `1`, `2` into three wrongly-identical `"a"`s. The fix only takes the
/// isolated path when `rest` doesn't consult path context in the first
/// place (this arm's `if rest.is_empty() || !rest.iter().any(...)` guard);
/// otherwise it falls back to evaluating `[inner, ...rest]` combined, which
/// still threads `current_path` correctly (#1406).
#[test]
fn test_optional_wrapped_navigation_still_threads_path_into_rest_1335() -> Result<()> {
    // `.[]` under `?`, `key` in `rest`: must report each element's own
    // index, not a stale outer one.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | (.[])? | key"], Some(r#"{"a":[10,20,30]}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "0\n1\n2");

    // Same shape via a field step instead of iteration.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | (.b)? | key"], Some(r#"{"a":{"b":1}}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "\"b\"");

    // Without `?`, the same query is the baseline this must match.
    let (stdout, _, code) = run_jq_full(&["-c", ".a | .[] | key"], Some(r#"{"a":[10,20,30]}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "0\n1\n2");

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
    // #1043 fixed the bug this test originally probed: `eval_comma`'s tail
    // match was missing a `0 => None` arm, so `(empty,empty)` (two
    // `None`-valued comma operands, `owned` never promoted) fell out through
    // `None => QueryResult::Many(borrowed)` with `borrowed` still `[]` --
    // producing `Many(vec![])` instead of `None`. That's now routed through
    // `borrowed_vec_to_result`, which correctly collapses an empty
    // accumulator to `None`.
    //
    // `ltrimstr`'s argument slot now feeds that `None` into
    // `result_to_owned_full`'s `QueryResult::None => Ok(None)` arm (#1045),
    // which `ltrimstr` propagates as its own zero output -- matching real
    // jq's `f(g)` semantics (backtracking over every output of `g`, zero
    // times if `g` produces none): `jq -n '"abc" | ltrimstr((empty,empty))'`
    // exits 0 with no output, same as succinctly now does.
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | ltrimstr((empty,empty))"#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_result_to_owned_manyowned_empty_arm_via_ltrimstr_argument() -> Result<()> {
    // #1043 fixed the bug this test originally probed: the `Owned`-target
    // sibling of the `Many`-empty case above, reached through a different
    // producer -- `eval_index_expr`'s `Targets::Owned` branch used to end
    // its match on `out.len()` with only `1 => Owned(...)`, so the
    // `_ => ManyOwned(out)` wildcard also covered `out.len() == 0`. `(2+3)`
    // is computed (arithmetic is always `Owned`/`ManyOwned`, never
    // document-borrowed, so its target is `Targets::Owned`), and both
    // `"x"`/`"y"` keys against the number `5` -- with the trailing `?`
    // making `optional` true -- each resolve via `index_one_owned`'s `_ if
    // optional => Ok(None)` refusal-suppression arm, leaving `out` empty.
    // That's now routed through `owned_vec_to_result`, which correctly
    // collapses an empty accumulator to `None`.
    //
    // Same fix as the `Many`-empty case above (#1045): `result_to_owned_full`'s
    // `QueryResult::None => Ok(None)` arm lets `ltrimstr` propagate zero
    // output instead of erroring: `jq -n '"abc" | ltrimstr((2+3) |
    // .[("x","y")]?)'` exits 0 with no output in real jq, matching
    // succinctly now.
    let (stdout, stderr, code) =
        run_jq_full(&["-n", r#""abc" | ltrimstr((2+3) | .[("x","y")]?)"#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
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
fn test_result_to_owned_none_error_arms_via_ltrimstr_argument() -> Result<()> {
    // `result_to_owned_full`'s `None`/`Error` arms. Uses `ltrimstr`'s
    // argument slot as the call site, same as the `Many`/`ManyOwned`-empty
    // and `Partial`-halt tests above. `None` used to be a generic catchable
    // "no value" error (`result_to_owned`'s old contract); #1045 fixed
    // `ltrimstr` to propagate a zero-output argument as its own zero
    // output instead, matching real jq: `jq -n '"abc" | ltrimstr(empty)'`
    // exits 0 with no output. The `Error` arm is unaffected by #1045 and
    // still matches real jq exactly. (The `Break` arm this test used to
    // cover alongside these two is fixed separately -- see
    // `test_break_via_ltrimstr_argument_reaches_outer_label_833` below.)
    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | ltrimstr(empty)"#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");

    let (stdout, stderr, code) = run_jq_full(&["-n", r#""abc" | ltrimstr(error("boom"))"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "jq: error (at <unknown>): boom\n");

    Ok(())
}

#[test]
fn test_break_via_ltrimstr_argument_reaches_outer_label_833() -> Result<()> {
    // #833: `result_to_owned`'s `Break` arm now propagates a real
    // `EvalEscape::Break` instead of collapsing it into a synthetic "not in
    // label" error, so a `break` in a builtin's argument expression can
    // reach a `label` enclosing the whole call. Matches real jq exactly:
    // `jq -n 'label $out | ("abc" | ltrimstr(break $out))'` exits 0 with no
    // output (this test previously pinned the pre-fix "not in label" error
    // as expected/current behavior).
    let (stdout, stderr, code) = run_jq_full(
        &["-n", r#"label $out | ("abc" | ltrimstr(break $out))"#],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");

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
    // stopped correctly turning a *genuine* (non-halt) error in the
    // message expression into `isvalid` reporting `false`. #881 changed
    // *how* that happens: `isvalid` no longer forces `optional=true`, so
    // this arm's `Err(EvalEscape::Error(_)) if optional` guard no longer
    // fires here at all (ambient `optional` is `false`) -- the message
    // expression's error instead propagates as a genuine
    // `QueryResult::Error` all the way up through the outer `error(...)`
    // call, caught directly by `isvalid`'s `QueryResult::is_error` check.
    // Same observable outcome (`false`), different mechanism than when
    // this test was first written. `isvalid` is a succinctly extension
    // (real jq has no such builtin -- `jq -n 'isvalid(error("boom"))'`
    // reports `isvalid/1 is not defined`), so this is checked against
    // succinctly's own documented contract instead of jq parity.
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

/// `stitch_replacements_evaluated`'s (and `sub_with_resolved_pattern`'s
/// single-match arm's) non-optional type-mismatch case (#826, wording fixed
/// by #1034): once a match is found, the replacement expression is
/// evaluated per match and combined with the preceding gap text via
/// `arith_add` (jq's real `sub`/`gsub` builds `$gap + $inserts[$ix]`,
/// `src/builtin.jq`), so a non-string replacement now surfaces jq's own
/// binary-op wording byte-for-byte instead of a bespoke message. Verified
/// against jq 1.7.1: `jq 'sub("a"; 5)'` on `"abc"` exits 5 with exactly this
/// stderr text.
#[test]
fn test_sub_replacement_wrong_type_errors() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("a"; 5)"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <stdin>:0): string (\"\") and number (5) cannot be added\n"
    );
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
/// replacement error through `stitch_replacements_evaluated` (#826, wording
/// fixed by #1034) -- a distinct call site from plain `gsub`'s own error
/// propagation tested above. Verified against jq 1.7.1: `jq 'sub("a"; 5;
/// "g")'` on `"abc"` exits 5 with exactly this stderr text.
#[test]
fn test_sub_with_flags_global_replacement_wrong_type_errors() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("a"; 5; "g")"#], Some(r#""abc""#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(
        stderr,
        "jq: error (at <stdin>:0): string (\"\") and number (5) cannot be added\n"
    );
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

/// `eval_sub_replacement`'s zero-output shape (#840), the sibling gap to the
/// multi-value stopgap above -- untested until now, and (unlike that
/// stopgap) now fully implemented rather than left as a divergence.
/// `stitch_replacements_evaluated`/`builtin_sub_with_flags` re-derive jq's
/// own rule for a replacement filter that produces zero outputs: if *every*
/// match's replacement is empty, the whole input comes back unchanged; a
/// single-match `sub` is the trivial case of that rule (one match is
/// trivially "every match"). Verified against jq 1.7.1: `jq -c 'sub("a";
/// empty)'` on `"cab"` prints `"cab"` unchanged, exit 0.
#[test]
fn test_sub_replacement_all_matches_empty_leaves_input_unchanged() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"sub("a"; empty)"#], Some(r#""cab""#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"cab\"\n");
    Ok(())
}

/// `gsub` counterpart to the test above, with 3 matches (not 1) all empty --
/// the case that actually distinguishes "every match is empty" from
/// "delete every empty match", since a naive per-match deletion would give
/// `""`, not `"banana"`. Verified against jq 1.7.1: `jq -c 'gsub("a";
/// empty)'` on `"banana"` prints `"banana"` unchanged, exit 0.
#[test]
fn test_gsub_replacement_all_matches_empty_leaves_input_unchanged() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"gsub("a"; empty)"#], Some(r#""banana""#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"banana\"\n");
    Ok(())
}

/// `gsub` with a *mix* of empty and non-empty replacements, the shape
/// #840's own repro used -- distinct from the two "every match empty" tests
/// above, whose all-input-unchanged rule only applies when there is no
/// non-empty replacement anywhere in the call. Verified against jq 1.7.1:
/// `jq -c 'gsub("(?<x>[aeiou])"; if .x=="e" then empty else "["+.x+"]"
/// end)'` on `"hello world"` prints `"ll[o] w[o]rld"` -- the empty match's
/// own preceding gap (`"h"`) is dropped along with the match itself, while
/// the two non-empty matches (both `"o"`) are processed normally.
#[test]
fn test_gsub_replacement_mixed_empty_and_nonempty_drops_only_empty_matches_own_gap() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"gsub("(?<x>[aeiou])"; if .x=="e" then empty else "["+.x+"]" end)"#,
        ],
        Some(r#""hello world""#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"ll[o] w[o]rld\"\n");
    Ok(())
}

/// Regression guard for the exact gap-tracking rule: with *non-adjacent*
/// empty matches (real text sitting between two empty matches, not just
/// between an empty and a non-empty one), each empty match still only drops
/// its own immediately-preceding gap -- the gap belongs to whichever match
/// follows it, whether or not that match happens to be a survivor.
/// Verified against jq 1.7.1: `jq -c 'gsub("(?<x>[ac])"; if .x=="c" then
/// "["+.x+"]" else empty end)'` on `"xaYaZc"` prints `"Z[c]"` -- `"x"`
/// (before the first empty `a`) and `"Y"` (before the second empty `a`) are
/// both dropped, but `"Z"` (before the surviving `c`) is kept.
#[test]
fn test_gsub_replacement_non_adjacent_empty_matches_each_drop_only_their_own_gap() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"gsub("(?<x>[ac])"; if .x=="c" then "["+.x+"]" else empty end)"#,
        ],
        Some(r#""xaYaZc""#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "\"Z[c]\"\n");
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
fn test_parentn_n_expr_genuinely_uncaught_break_still_errors() -> Result<()> {
    // No `label $out` anywhere in this filter, so `break $out` raised while
    // evaluating `parent(n)`'s `n` argument (via `ParentN`'s call to
    // `eval_owned_expr`) has nowhere to land regardless of #833's fix to
    // that function -- `eval_owned_expr` now propagates a real
    // `EvalEscape::Break` instead of collapsing it at this arm, but with no
    // enclosing label to catch it, it still surfaces as the ordinary
    // "break $label not in label" top-level diagnostic once fully unwound,
    // the same as any other genuinely-uncaught break in this file. `parent`
    // is a succinctly extension (no real-jq equivalent), so this is checked
    // against succinctly's own established "break $out not in label" wording
    // (see `test_uncaught_break_after_output_keeps_the_prefix`), not jq. See
    // `test_break_via_parentn_argument_reaches_outer_label_833` below for
    // the case where a `label $out` genuinely encloses the call.
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
fn test_break_via_parentn_argument_reaches_outer_label_833() -> Result<()> {
    // #833: same fix as `test_break_via_ltrimstr_argument_reaches_outer_label_833`,
    // but through `eval_owned_expr` (ParentN's call path) rather than
    // `result_to_owned`. With a `label $out` genuinely enclosing the call,
    // the break now unwinds cleanly instead of misreporting "not in label".
    // `parent` is a succinctly extension; there's no real-jq oracle for this
    // exact shape, but the underlying "an unmatched builtin-argument break
    // reaches its enclosing label" contract is the same one #833 fixes
    // uniformly for both `result_to_owned` and `eval_owned_expr` callers.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", ".a.b | label $out | parent(break $out)"],
        Some(r#"{"a":{"b":1}}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
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

/// Review-driven regression guard on #854's own rewrite of `resolve_recurse`'s
/// `Some(cond)` arm: switching `cond`'s own evaluation to
/// `eval_owned_multi_keep_partial` must not drop the pre-existing
/// `is_null_current` gate that bounds recursion *past* a null node (#856) --
/// a truthy `cond` must not re-open that bound. NOT a real-jq-parity case:
/// `recurse(.a?; true)` on `{"a":null}` never terminates in real jq 1.7.1
/// either (confirmed live: `null.a?` is `null` again, `true` keeps accepting
/// it, `path()` prints ever-longer paths until killed -- real jq's actual
/// unbounded semantics here, not a succinctly gap). This pins succinctly's
/// own documented, deliberate divergence instead: `cond` present must
/// terminate in exactly the same 2 outputs the no-`cond` case does.
#[test]
fn test_resolve_recurse_cond_still_bounds_null_current_growth_854() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r"path(recurse(.a?; true))"], Some(r#"{"a":null}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n[\"a\"]\n");
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
fn test_recurse_cond_keeps_already_approved_siblings_before_error_854() -> Result<()> {
    // Issue #854's primary repro (value position). Distinct from #842
    // (which was `f`'s own fan-out): here `f` succeeds fully and it's
    // `cond`'s own per-child evaluation loop that errors partway through a
    // node's children, dropping the siblings that already passed `cond`
    // earlier in that same loop. Verified against jq 1.7.1:
    // `echo '[1,2,3]' | jq -c 'recurse(if type=="array" then .[] else
    // empty end; if . == 2 then error("boom") else true end)'` prints
    // `[1,2,3]` and `1` to stdout before erroring, and exits 5.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"recurse(if type=="array" then .[] else empty end; if . == 2 then error("boom") else true end)"#,
        ],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[1,2,3]\n1\n");
    assert!(stderr.contains("boom"), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_resolve_recurse_cond_keeps_already_approved_siblings_before_error_854() -> Result<()> {
    // Issue #854's secondary repro (path position, `resolve_recurse`).
    // Same underlying bug as the value-position test above, in the
    // path-tracking evaluator `path(...)` uses. Verified against jq 1.7.1:
    // `echo '[1,2,3]' | jq -c 'path(recurse(if type=="array" then .[] else
    // empty end; if . == 2 then error("boom") else true end))'` prints
    // `[]` and `[0]` to stdout before erroring, and exits 5.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(recurse(if type=="array" then .[] else empty end; if . == 2 then error("boom") else true end))"#,
        ],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n[0]\n");
    assert!(stderr.contains("boom"), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_recurse_cond_own_multi_output_fanout_kept_before_error_854() -> Result<()> {
    // The deepest layer of #854: `cond` itself can be a multi-output
    // generator (independent of the child), so a truthy output it already
    // produced for a given child before erroring on a later output of that
    // same call must still queue the child for its own full recursive
    // descent. Real jq's lazy interleaving then recurses fully into that
    // approved child *before* ever asking `cond` for its second output, so
    // the error that actually surfaces is whatever that recursion itself
    // hits — not `cond`'s own deferred one. Verified against jq 1.7.1:
    // `echo '{"a":1,"b":2,"c":3}' | jq -c 'recurse(.[]; (true,
    // error("cond-err")))'` prints the root and `1` to stdout, then exits
    // 5 with "Cannot iterate over number (1)" — not "cond-err".
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"recurse(.[]; (true, error("cond-err")))"#],
        Some(r#"{"a":1,"b":2,"c":3}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\":1,\"b\":2,\"c\":3}\n1\n");
    assert!(
        stderr.contains("Cannot iterate over number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// #896: the same "drops an already-produced generator prefix on a later
/// error" bug class #842/#854 fixed for `recurse`'s own `f`/`cond` loops was
/// independently live in `resolve_node`'s `Select` arm — `cond` runs
/// through the all-or-nothing `eval_owned_multi`, so an already-produced
/// truthy branch was silently dropped instead of printed before the error.
/// Verified against jq 1.7.1: `echo '1' | jq -c
/// 'path(select((true, error("x"))))'` prints `[]` to stdout before
/// erroring, and exits 5.
#[test]
fn test_resolve_node_select_keeps_cond_partial_fanout_before_error_896() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"path(select((true, error("x"))))"#], Some("1"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    Ok(())
}

/// #896: same bug class as the `Select` test above, in `resolve_node`'s
/// `If` arm's `cond` evaluation. Verified against jq 1.7.1: `echo '1' | jq
/// -c 'path(if (true, error("x")) then . else empty end)'` prints `[]` to
/// stdout before erroring, and exits 5.
#[test]
fn test_resolve_node_if_keeps_cond_partial_fanout_before_error_896() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(if (true, error("x")) then . else empty end)"#],
        Some("1"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    Ok(())
}

/// #896: same bug class, in `resolve_index_expr`'s computed-key evaluation
/// (`.[EXPR]` where `EXPR` is a multi-output generator). Verified against
/// jq 1.7.1: `echo '{"a":1,"b":2}' | jq -c 'path(.[("a", error("x"))])'`
/// prints `["a"]` to stdout before erroring, and exits 5.
#[test]
fn test_resolve_index_expr_keeps_key_partial_fanout_before_error_896() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(.[("a", error("x"))])"#],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    Ok(())
}

/// #896: same bug class, in `resolve_node`'s `Builtin::GetPath` arm's
/// argument evaluation. Verified against jq 1.7.1: `echo '{"a":1}' | jq -c
/// 'path(getpath((["a"], error("x"))))'` prints `["a"]` to stdout before
/// erroring, and exits 5.
#[test]
fn test_resolve_node_getpath_keeps_arg_partial_fanout_before_error_896() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(getpath((["a"], error("x"))))"#],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    Ok(())
}

/// #896: a bare escape (zero prior outputs of the generator) must still
/// propagate normally at each of the 4 sites above — the partial-keeping
/// fix must not change this case. Verified against jq 1.7.1: both queries
/// below print nothing to stdout and exit 5.
#[test]
fn test_resolve_node_896_sites_bare_error_still_propagates() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"path(select(error("x")))"#], Some("1"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");

    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"path(.[(error("x"))])"#], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    Ok(())
}

/// #896 review round: `resolve_index_expr`'s `target_branches =
/// resolve_node::<S>(target, value, trackable)?` used to discard the whole
/// prefix on any `target` error via `?` — harmless before #896, since none
/// of the 4 fixed sites could ever produce a non-empty prefix on `Err`. Now
/// that they can, that prefix has to be indexed by `keys` like any other
/// target branch, not returned unindexed. Verified against jq 1.7.1:
/// `echo '{"a":1,"x":9}' | jq -c 'path(select((true, error("t")))[("x",
/// error("k"))])'` prints `["x"]` (the already-produced `select` branch,
/// indexed by `"x"`) then raises `t` — `key`'s own deferred escape (`k`)
/// is never reached, since jq's `K as $k | E | .[$k]` compilation (this
/// function's own doc comment) exhausts `E`'s whole generator, escape
/// included, before ever asking `K` for its next value.
#[test]
fn test_resolve_index_expr_indexes_targets_partial_fanout_before_its_own_error_896() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(select((true, error("t")))[("x", error("k"))])"#,
        ],
        Some(r#"{"a":1,"x":9}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"x\"]\n");
    assert!(stderr.contains('t'), "stderr: {stderr:?}");
    assert!(!stderr.contains('k'), "stderr: {stderr:?}");
    Ok(())
}

/// #977: `resolve_seq`'s fan-out loop used to return an escaping element's
/// partial prefix without ever applying a purely-static tail after it (a
/// literal `.foo`/`[N]`/`[N:M]`, desugared to a flat `Pipe` at parse time —
/// distinct from #896's own 4 sites, which only cover a *dynamic*
/// subscript). Verified against jq 1.7.1 for all three tail shapes.
#[test]
fn test_resolve_seq_applies_static_tail_after_fanout_element_escape_977() -> Result<()> {
    // Literal index tail.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path((select(true, error("t")))[0])"#],
        Some("[10,20,30]"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[0]\n");
    assert!(stderr.contains('t'), "stderr: {stderr:?}");

    // Literal slice tail.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path((select(true, error("t")))[0:1])"#],
        Some("[10,20,30]"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[{\"start\":0,\"end\":1}]\n");
    assert!(stderr.contains('t'), "stderr: {stderr:?}");

    // Literal field tail.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path((select(true, error("t"))).a)"#],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains('t'), "stderr: {stderr:?}");
    Ok(())
}

/// #977: if the static tail itself also fails while being applied to the
/// fan-out element's partial prefix, that later failure takes priority over
/// the earlier deferred one — the same "later step's own failure outranks
/// an earlier deferred one" rule this codebase already applies elsewhere
/// (`resolve_slice_expr`'s `target_escape`). Verified against jq 1.7.1:
/// indexing a number with `[0]` raises jq's own type error, not `t`.
#[test]
fn test_resolve_seq_static_tail_failure_outranks_earlier_fanout_escape_977() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"path((select(true, error("t")))[0])"#], Some("5"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("Cannot index number with number"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// #1013 (split from #977's own review): when a pipe has 2+ dynamic
/// (fan-out) elements and an *earlier* one escapes — not the last, i.e.
/// `flat`'s dynamic element at some index `i < last_dynamic` — the
/// escaping branch's own partial output (the successful alternative(s) it
/// produced before its escape) still has to thread through every remaining
/// dynamic stage (`flat[i+1..=last_dynamic]`) and the static tail after
/// `last_dynamic`, exactly like a branch that never escaped. Before #1013,
/// `resolve_seq` returned that partial prefix immediately instead —
/// pre-#977's silent-tail-drop shape, just for this narrower,
/// 2+-dynamic-element pipe shape. Verified against jq 1.7.1: `.c[(0,1)]`
/// (the second dynamic element) and `.foo` (the tail) both still have to
/// run for `.a[0]`, the successful alternative, before jq ever raises `t`.
#[test]
fn test_resolve_seq_earlier_fanout_escape_threads_through_later_dynamic_stage_1013() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(.a[(0,error("t"))] | .c[(0,1)] | .foo)"#],
        Some(r#"{"a":[{"c":[{"foo":1},{"foo":2}]}]}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(
        stdout,
        "[\"a\",0,\"c\",0,\"foo\"]\n[\"a\",0,\"c\",1,\"foo\"]\n"
    );
    assert!(stderr.contains('t'), "stderr: {stderr:?}");

    // A second shape, with a dynamic (not literal) second stage (`.k[.c]`)
    // and a static tail (`.d`) after it.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(.a[(0,error("t"))] | .k[.c] | .d)"#],
        Some(r#"{"a":[{"c":"k","k":{"d":99}}]}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\",0,\"k\",\"k\",\"d\"]\n");
    assert!(stderr.contains('t'), "stderr: {stderr:?}");
    Ok(())
}

/// #1013: the fix generalizes past exactly 2 dynamic elements — a 3-dynamic
/// pipe with the escape in the *middle* (`.b[(0,error("t"))]`, neither
/// first nor last) still has to thread its `0` alternative through the
/// remaining dynamic stage (`.c[(0,1)]`) and the tail (`.foo`). Verified
/// against jq 1.7.1.
#[test]
fn test_resolve_seq_earlier_fanout_escape_threads_through_middle_of_three_dynamic_stages_1013(
) -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(.a[0] | .b[(0,error("t"))] | .c[(0,1)] | .foo)"#,
        ],
        Some(r#"{"a":[{"b":[{"c":[{"foo":1},{"foo":2}]}]}]}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(
        stdout,
        "[\"a\",0,\"b\",0,\"c\",0,\"foo\"]\n[\"a\",0,\"b\",0,\"c\",1,\"foo\"]\n"
    );
    assert!(stderr.contains('t'), "stderr: {stderr:?}");
    Ok(())
}

/// #1013: when *two* dynamic stages each escape (not just one), the later
/// stage's escape outranks the earlier deferred one — the same "later
/// step's own failure outranks an earlier deferred one" rule
/// `test_resolve_seq_static_tail_failure_outranks_earlier_fanout_escape_977`
/// already pins between a fan-out escape and a tail failure, extended here
/// across two fan-out stages. This matches jq's real generator order: jq
/// fully threads `.a[0]` through everything downstream — including
/// `.c[...]`'s own escape — before it would ever backtrack to try
/// `.a[...]`'s second alternative (`error("t1")`), so `t2` is the error jq
/// actually raises, not `t1`. Verified against jq 1.7.1.
#[test]
fn test_resolve_seq_later_fanout_escape_outranks_earlier_one_1013() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(.a[(0,error("t1"))] | .c[(0,error("t2"))] | .foo)"#,
        ],
        Some(r#"{"a":[{"c":[{"foo":1}]}]}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\",0,\"c\",0,\"foo\"]\n");
    assert!(stderr.contains("t2"), "stderr: {stderr:?}");
    assert!(!stderr.contains("t1"), "stderr: {stderr:?}");
    Ok(())
}

/// #896 review round: `GetPath`'s arm only builds its own partial-prefix
/// branch for the exact single-array-output shape; any other shape (here,
/// 2+ valid array outputs before the error) falls through to `resolve_leaf`,
/// which re-evaluates `arg` from scratch via a different, single-shot code
/// path that discards the escape it rediscovers — before this fix, that
/// fabricated an unrelated "Invalid path expression" message instead of the
/// real error. Now it at least surfaces the correct error (still without the
/// fan-out, which needs a separate, larger fix to `resolve_leaf`'s own
/// multi-output handling — out of scope here, filed separately). Verified
/// against jq 1.7.1: `echo '{"a":1,"b":2}' | jq -c 'path(getpath((["a"],
/// ["b"], error("x"))))'` prints `["a"]` then `["b"]` before raising `x`;
/// this codebase intentionally only matches the error, not the fan-out.
#[test]
fn test_resolve_node_getpath_multi_output_surfaces_real_error_not_a_fabricated_one_896(
) -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path(getpath((["a"], ["b"], error("x"))))"#],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    assert!(
        !stderr.contains("Invalid path expression"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// #973 review round: `resolve_slice_expr`'s `target_branches =
/// resolve_node::<S>(target, value, trackable)?` had the same bare-`?`
/// bug #896's review already found and fixed in `resolve_index_expr`'s
/// sibling target-resolution step — reachable only with a *dynamic* slice
/// bound (`(0+1)`, not a literal), since a literal bound desugars into a
/// static `Pipe` at parse time and never reaches this function at all
/// (that literal-bound gap is a separate, unrelated bug in `resolve_seq`,
/// filed as #977). Verified against jq 1.7.1: `echo '[10,20,30]' | jq -c
/// 'path((select(true, error("t")))[(0+1):2])'` prints
/// `[{"start":1,"end":2}]` (the already-produced `select` branch, sliced)
/// before raising `t`.
#[test]
fn test_resolve_slice_expr_keeps_target_partial_fanout_before_its_own_error_973() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"path((select(true, error("t")))[(0+1):2])"#],
        Some("[10,20,30]"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[{\"start\":1,\"end\":2}]\n");
    assert!(stderr.contains('t'), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_walk_propagates_halt_from_f() -> Result<()> {
    // `walk_impl` applies `f` via `eval_owned_expr_fork` at every level
    // (#855, #960); a scalar input like `1` has no children, so this
    // exercises that application directly, and a halt from `f` propagates
    // through `finish_fork`'s `Control::Halt` handling into a `QueryResult`.
    // Verified against jq 1.7.1: `jq -n '1 | walk(halt_error(8))'` prints
    // nothing to stdout, dumps `1` (the value `f` was applied to) to stderr,
    // and exits 8.
    let (stdout, stderr, code) = run_jq_full(&["-n", "1 | walk(halt_error(8))"], None)?;
    assert_eq!(code, 8, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

// ============================================================================
// walk(f)/repeat(f) drop a trailing error/break after a multi-output f
// (#855), and don't fork on a multi-output f at all, independent of errors
// (#960)
// ============================================================================
// eval_owned_expr_ctrl's Partial(vs, control) arm keeps the values `f`
// already produced but silently drops the trailing Error/Break. Fixed for
// `repeat` by switching its loop to eval_owned_expr_fork (which also fixes
// the same collapse-to-array bug for a plain, non-erroring multi-output f).
// `walk_impl` implements jq's actual recursive fork semantics directly
// (verified against jq 1.7.1 for every combination rule -- see its own doc
// comment): array children flatten-concatenate their sub-streams (`map`),
// object children take the first output or delete the key (`map_values`/
// `|=`), and both are atomic on an interior error/break, discarding every
// sibling already processed. This closes both #855 and #960 for walk; #960
// remains open only in spirit for `repeat`, which cannot meaningfully
// "recurse into children" (it's a flat loop over the unchanged original
// input, not a tree), and already forks fully as of #855's own fix.

#[test]
fn test_walk_propagates_error_after_partial_output() -> Result<()> {
    // Verified against jq 1.7.1: `echo 5 | jq -c 'walk(1, error("bad"))'`
    // prints `1` then raises, exit 5.
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"walk(1, error("bad"))"#], Some("5"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    assert!(stderr.contains("bad"), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_walk_propagates_break_after_partial_output() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", "label $out | walk(1, break $out)"], Some("5"))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_repeat_propagates_error_after_partial_output() -> Result<()> {
    // Verified against jq 1.7.1: `echo null | jq -c 'limit(3;
    // repeat((1, error("bad"))))'` prints `1` once then raises, exit 5 --
    // not looping to `MAX_ITERATIONS` with the error silently dropped every
    // round (the pre-fix behavior).
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"limit(3; repeat((1, error("bad"))))"#],
        Some("null"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "1\n");
    assert!(stderr.contains("bad"), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_repeat_propagates_break_after_partial_output() -> Result<()> {
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", "label $out | limit(3; repeat((1, break $out)))"],
        Some("null"),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout, "1\n");
    Ok(())
}

#[test]
fn test_repeat_multi_output_extends_rather_than_collapses() -> Result<()> {
    // The non-error sibling of the two tests above: `repeat`'s own loop
    // previously collapsed each round's multi-output `expr` into one array
    // per round instead of extending its output stream with every value.
    // Verified against jq 1.7.1: `echo 0 | jq -c '[limit(6; repeat((.+1,
    // .+100)))]'` is `[1,100,1,100,1,100]`, not
    // `[[1,100],[1,100],[1,100]]`.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "[limit(6; repeat((.+1, .+100)))]"], Some("0"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[1,100,1,100,1,100]\n");
    Ok(())
}

#[test]
fn test_repeat_empty_expr_yields_nothing_instead_of_looping_forever_on_nulls() -> Result<()> {
    // `eval_owned_expr_ctrl` mapped a zero-output round to `Ok(Null)`, so
    // `repeat(empty)` used to emit `MAX_ITERATIONS` `null`s; `eval_owned_expr_fork`
    // reports zero-output rounds as genuinely zero values, so `repeat(empty)`
    // now emits nothing at all -- jq itself has no oracle answer here (a
    // bare `repeat(empty)` spins forever with no output either way, so this
    // pins succinctly's own bounded behavior rather than a jq comparison).
    let (stdout, stderr, code) = run_jq_full(&["-c", "[limit(3; repeat(empty))]"], Some("null"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    Ok(())
}

#[test]
fn test_walk_array_multi_output_flattens_children_streams() -> Result<()> {
    // `map(w) = [.[] | w]`: every child's own output stream flattens
    // straight into the rebuilt array, not one sub-array per child.
    // Verified against jq 1.7.1: `echo '[5,6]' | jq -c 'walk(if
    // type=="array" then . else (1,2) end)'` is `[1,2,1,2]`.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"walk(if type=="array" then . else (1,2) end)"#],
        Some("[5,6]"),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[1,2,1,2]\n");
    Ok(())
}

#[test]
fn test_walk_array_zero_output_child_drops_out_of_the_array() -> Result<()> {
    // Verified against jq 1.7.1: `echo '[5,6]' | jq -c 'walk(if
    // type=="array" then . else empty end)'` is `[]`, not `[null,null]`.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"walk(if type=="array" then . else empty end)"#],
        Some("[5,6]"),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[]\n");
    Ok(())
}

#[test]
fn test_walk_object_multi_output_child_takes_the_first_value() -> Result<()> {
    // `map_values(w) = .[] |= w`: jq's `|=` commits the *first* output of
    // the update stream (via its `label`/`break` desugaring), not the
    // last. Verified against jq 1.7.1: `echo '{"a":5}' | jq -c 'walk(if
    // type=="object" then . else (1,2) end)'` is `{"a":1}`, not `{"a":2}`.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"walk(if type=="object" then . else (1,2) end)"#],
        Some(r#"{"a":5}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\":1}\n");
    Ok(())
}

#[test]
fn test_walk_object_zero_output_child_deletes_the_key() -> Result<()> {
    // Verified against jq 1.7.1: `echo '{"a":5}' | jq -c 'walk(if
    // type=="object" then . else empty end)'` is `{}`.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"walk(if type=="object" then . else empty end)"#],
        Some(r#"{"a":5}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{}\n");
    Ok(())
}

#[test]
fn test_walk_array_construction_is_atomic_on_an_interior_error() -> Result<()> {
    // A child's error must discard the *whole* array being built, not just
    // that one element -- matching how `[...]` construction works in jq
    // generally. Verified against jq 1.7.1: `echo '[5,6]' | jq -c 'walk(if
    // .==5 then error("x") else . end)'` raises with no output at all, not
    // a partial array (e.g. `[6]` or `[null,6]`).
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"walk(if .==5 then error("x") else . end)"#],
        Some("[5,6]"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_walk_object_construction_is_atomic_on_an_interior_error() -> Result<()> {
    // The object counterpart to the array test above -- a distinct branch
    // in `walk_impl` (objects use `map_values`/`|=` semantics, not `map`'s),
    // so it needs its own coverage. Verified against jq 1.7.1: `echo
    // '{"a":5,"b":6}' | jq -c 'walk(if .==5 then error("x") else . end)'`
    // raises with no output at all.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"walk(if .==5 then error("x") else . end)"#],
        Some(r#"{"a":5,"b":6}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains('x'), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_repeat_value_budget_bounds_total_output_independent_of_fan_out() -> Result<()> {
    // Regression guard for the memory-bound fix: without a per-value
    // budget, a single highly-fanning-out round (`range(20001)` produces
    // 20001 values in one round) could balloon `outputs` far past any
    // reasonable size before the per-round `MAX_ITERATIONS` cap ever had a
    // chance to matter. `repeat` is a succinctly extension (no upstream jq
    // builtin to compare against), so this pins succinctly's own contract:
    // the budget (`REDUCE_FOREACH_MAX_STEPS`, shared with `reduce`/
    // `foreach`) cuts the stream at exactly 10000 values with a clear
    // error, not an unbounded allocation.
    let (stdout, stderr, code) = run_jq_full(&["-c", "repeat(range(20001))"], Some("null"))?;
    assert_eq!(code, 5, "stderr: {stderr:?}");
    assert_eq!(stdout.lines().count(), 10000, "stdout: {stdout:?}");
    assert_eq!(stdout.lines().last(), Some("9999"));
    assert!(
        stderr.contains("maximum iterations exceeded"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_walk_nested_break_reaches_its_enclosing_label() -> Result<()> {
    // Before this fix, a nested `f` application collapsed a `Control::Break`
    // into a synthetic "break $label not in label" `EvalError` (the same
    // #575 shape already fixed for `repeat`'s own single-level loop, now
    // fixed for every level of `walk`'s tree recursion too). Verified
    // against jq 1.7.1: `echo '[5]' | jq -c 'label $out | walk(if
    // type=="number" then break $out else . end)'` prints nothing and
    // exits 0 -- not a "break $out not in label" error.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | walk(if type=="number" then break $out else . end)"#,
        ],
        Some("[5]"),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_walk_nested_error_reports_the_same_message_jq_does() -> Result<()> {
    // Before this fix, a nested trailing error was silently absorbed and
    // the *outermost* `f` was re-applied to the corrupted, partially-built
    // tree instead -- reporting a synthetic error message and a spurious
    // stdout value neither of which jq ever produces. Verified against jq
    // 1.7.1: `echo '[5]' | jq -c 'walk(1, error(tostring))'` prints nothing
    // to stdout and reports "5" (the value `f` was applied to, via
    // `tostring`), not "[1]" (a rebuilt array `f` was never really applied
    // to in the real evaluation).
    let (stdout, stderr, code) = run_jq_full(&["-c", "walk(1, error(tostring))"], Some("[5]"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains('5'), "stderr: {stderr:?}");
    assert!(!stderr.contains('1'), "stderr: {stderr:?}");
    Ok(())
}

#[test]
fn test_walk_deep_nesting_does_not_overflow_the_stack() -> Result<()> {
    // Regression guard: an earlier version of this fix split `walk_impl`
    // into two mutually-recursive functions, adding a non-inlined stack
    // frame per nesting level. Measured on a release build, that dropped
    // the depth `walk(.)` could handle before overflowing from ~7000-8000
    // down to ~6000-7000; on the debug build this test binary actually
    // runs, frames are far larger and the safe/unsafe boundary sits
    // between depth 500 and 700 post-fix (measured). Depth 200 here is
    // comfortably under that boundary (leaving margin for a CI runner with
    // a smaller default stack than the dev machine this was measured on)
    // while still deep enough that reintroducing a meaningfully-sized
    // per-level frame has a real chance of tripping it.
    let depth = 200;
    let input = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    // Compares stdout directly against the input text (both already
    // whitespace-free) rather than round-tripping through jq's own `==`,
    // whose comparison recursion could mask a `walk`-specific regression
    // behind a stack limit of its own.
    let (stdout, stderr, code) = run_jq_full(&["-c", "walk(.)"], Some(&input))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), input);
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

/// #833: a `key`/`parent`/`path` (or any other path-context-triggering)
/// pipe stage ahead of `isvalid`/`isempty` routes their argument through
/// `eval_pipe_with_path_context_internal`'s generic `Expr::Builtin`
/// fallback -> `eval_builtin_owned` -> `eval_owned_expr`, not through
/// `eval_single`'s ordinary dispatch -- a genuinely different call path
/// from every other test in this file, which all evaluate `isvalid`/
/// `isempty` directly. Before #833 this still misreported "not in label"
/// even after #867/#879 fixed the direct-evaluation path, since the bug
/// lived in `eval_owned_expr` itself, not in `isvalid`/`isempty`'s own
/// match arms. Confirmed live against jq 1.7.1 (both real jq's own `key`
/// only exists in `--eval-all`/path-context contexts here via succinctly's
/// own routing, but the surrounding break/label semantics are jq's own):
/// both exit 0 with no output once `break $out` reaches its label.
#[test]
fn test_isvalid_propagates_break_through_path_context_routing_833() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "label $out | key | isvalid(break $out), \"after\""],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_isempty_propagates_break_through_path_context_routing_833() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "label $out | key | isempty(break $out), \"after\""],
        None,
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
/// proves `parent(...)?` still swallows an ordinary error in its `n`
/// argument to zero output. `has(error("x"))` is used as the argument
/// because `error`'s own `optional` handling would otherwise self-swallow
/// to `Ok(Null)` directly, without exercising `has` at all.
///
/// Before #1045, `has`'s own argument-`None` case turned into an
/// unconditional "no value" `Err`, so this reached `parent`'s `Err(...) if
/// optional` arm specifically. #1045 correctly changed `has` to propagate a
/// zero-output argument as `QueryResult::None` instead (matching real jq's
/// `has(empty)` -- zero output, not an error) -- so `has(error("x"))?` now
/// produces `Ok(Null)` (via `eval_owned_expr_ctrl_full`'s pre-existing
/// `QueryResult::None => Ok(Null)` collapse) and reaches `parent`'s `Ok(_)
/// if optional` arm instead. Either way `parent(...)?` still swallows to
/// zero output, which is what this test actually pins -- it doesn't
/// distinguish *which* internal arm produced that. `parent` is a
/// succinctly extension (no real-jq equivalent), so this is checked
/// against succinctly's own contract.
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
/// `Expr::Builtin` arm even under `?`.
///
/// This test used to pin the sibling `Err(EvalEscape::Error(_)) if optional`
/// guard via the same `has(error(...))` trick as the `parent` test above.
/// #1045 broke that trick here specifically: `has(error("boom"))?` (as
/// `first` itself, not nested inside another builtin's argument) no longer
/// reaches this arm as an `Err` at all -- it now resolves to
/// `QueryResult::None` directly (correct, matching real jq's own `has(...)?`
/// swallowing to zero output, live-verified). #1280 fixed the arm's own
/// `Ok`-side handling to preserve that `None` as zero output instead of
/// collapsing it to `Ok(Null)` via `eval_builtin_owned` -- this test now
/// pins the corrected output (matching real jq's own `has(...)?` empty-not-
/// null contract) instead of the characterized bug it originally caught.
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

/// Companion to `test_path_context_optional_does_not_swallow_halt_in_object_literal_arm`,
/// same #1045/#1280 history as
/// `test_path_context_builtin_arm_still_swallows_ordinary_error_under_optional`
/// just above -- see that test's doc comment for the full explanation.
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
/// as $v | BODY`). Same #1045/#1280 history as
/// `test_path_context_builtin_arm_still_swallows_ordinary_error_under_optional`
/// just above -- see that test's doc comment for the full explanation.
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

/// #968: an out-of-range `%m` month (`00`, or `13`+) reached
/// `month_days[(month - 1) as usize]`'s fixed 12-element array with no
/// bounds check, panicking the process (month=0 wraps `(0 - 1) as usize`
/// to `usize::MAX`; month=13+ is a plain out-of-bounds index) instead of
/// reporting an ordinary parse error the way real jq does. Also covers an
/// out-of-range `%d` day (`32`+), which can't panic (day is only used in
/// arithmetic, never as an array index) but real jq rejects it too.
/// Pinned against the pinned jq oracle for every case, including the
/// (surprising, but confirmed live) fact that day `00` is accepted.
#[test]
fn test_strptime_out_of_range_month_or_day_errors_instead_of_panicking_968() -> Result<()> {
    for input in ["\"2024-00-15\"", "\"2024-13-15\"", "\"2024-01-32\""] {
        let (stdout, _, code) = run_jq_full(&["-c", r#"strptime("%Y-%m-%d")"#], Some(input))?;
        assert_eq!(code, 5, "input: {input}, stdout: {stdout:?}");
    }

    let (stdout, _, code) =
        run_jq_full(&["-c", r#"strptime("%Y-%m-%d")"#], Some("\"2024-01-00\""))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim(), "[2024,0,0,0,0,0,0,-1]");

    let (stdout, _, code) =
        run_jq_full(&["-c", r#"strptime("%Y-%m-%d")"#], Some("\"2024-06-15\""))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim(), "[2024,5,15,0,0,0,6,166]");

    // %D (mm/dd/yy) is a separate parsing arm reaching the same panic site.
    let (stdout, _, code) = run_jq_full(&["-c", r#"strptime("%D")"#], Some("\"13/15/24\""))?;
    assert_eq!(code, 5, "stdout: {stdout:?}");
    Ok(())
}

/// #968 follow-on: the first fix (validating `month`/`day` before
/// `month_days[(month - 1) as usize]`'s array index) left a second,
/// distinct panic reachable through the *weekday* computation two lines
/// above that array index -- a separate Howard Hinnant `days_from_civil`
/// formula, `u32`-typed, that assumes `day >= 1`. `day == 0` is real,
/// jq-valid input the fix above deliberately keeps accepting (see that
/// test), and for March specifically (the one month where the formula's
/// leading term truncates to exactly `0`) `day == 0` made `0u32 - 1`
/// underflow: confirmed live, `"2024-03-00"` panicked with "attempt to
/// subtract with overflow" even after the array-index fix alone. Every
/// other month's day-0 case was already safe (verified by sweeping all
/// 12), which is exactly why a test that only covered January (like the
/// one above) didn't catch it. Fixed by widening that computation's
/// intermediate types from `u32` to `i64`, matching the pinned jq oracle
/// for every month.
#[test]
fn test_strptime_day_zero_all_months_matches_march_underflow_fixed_968() -> Result<()> {
    let expected = [
        "[2024,0,0,0,0,0,0,-1]",
        "[2024,1,0,0,0,0,3,30]",
        "[2024,2,0,0,0,0,4,59]",
        "[2024,3,0,0,0,0,0,90]",
        "[2024,4,0,0,0,0,2,120]",
        "[2024,5,0,0,0,0,5,151]",
        "[2024,6,0,0,0,0,0,181]",
        "[2024,7,0,0,0,0,3,212]",
        "[2024,8,0,0,0,0,6,243]",
        "[2024,9,0,0,0,0,1,273]",
        "[2024,10,0,0,0,0,4,304]",
        "[2024,11,0,0,0,0,6,334]",
    ];
    for (i, exp) in expected.iter().enumerate() {
        let month = i + 1;
        let input = format!("\"2024-{month:02}-00\"");
        let (stdout, _, code) = run_jq_full(&["-c", r#"strptime("%Y-%m-%d")"#], Some(&input))?;
        assert_eq!(code, 0, "month: {month}, stdout: {stdout:?}");
        assert_eq!(stdout.trim(), *exp, "month: {month}");
    }
    Ok(())
}

/// #971 (found reviewing #968): `%H`/`%M`/`%I` parsed up to 2 digits with
/// no range check, unlike the now-fixed `%m`/`%d` -- confirmed live
/// against the pinned jq oracle, out-of-range values for each error
/// ("does not match format", exit 5) in real jq but were silently
/// accepted here. `%S` deliberately stays permissive: `60` is a valid
/// leap second and real jq accepts it too. `%I`'s 1-12 range wasn't
/// named in #971's body (only `%H`/`%M` were), but the issue's own title
/// says "hour/minute range" and `%I` is an hour specifier with the exact
/// same gap -- confirmed live (`jq -n '"13" | strptime("%I")'` errors)
/// and fixed alongside `%H` rather than filing a separate near-duplicate
/// issue for it.
#[test]
fn test_strptime_hour_minute_out_of_range_errors_971() -> Result<()> {
    for (fmt, input) in [
        (r#"strptime("%H:%M:%S")"#, "\"99:99:99\""),
        (r#"strptime("%H:%M:%S")"#, "\"24:00:00\""),
        (r#"strptime("%H:%M:%S")"#, "\"00:60:00\""),
        // %S: 61+ errors, even though 60 (leap second) is valid.
        (r#"strptime("%H:%M:%S")"#, "\"23:59:61\""),
        (r#"strptime("%H:%M:%S")"#, "\"23:59:99\""),
        (r#"strptime("%I")"#, "\"13\""),
        // %R/%T duplicate %H/%M/%S's own parsing, so they need the same
        // range check independently -- confirmed live these leaked too.
        (r#"strptime("%R")"#, "\"99:99\""),
        (r#"strptime("%T")"#, "\"25:00:00\""),
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", fmt], Some(input))?;
        assert_eq!(code, 5, "fmt: {fmt}, input: {input}, stdout: {stdout:?}");
    }

    // Leap second: %S stays permissive up to (and including) 60, matching
    // real jq.
    let (stdout, _, code) = run_jq_full(&["-c", r#"strptime("%H:%M:%S")"#], Some("\"23:59:60\""))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim(), "[1970,0,1,23,59,60,4,0]");

    // Boundary values stay valid, including %I's jq-accepted "00" (a real
    // jq oracle divergence from the naive 1-12 range a first pass assumed).
    for input in ["\"12\"", "\"01\"", "\"00\""] {
        let (stdout, _, code) = run_jq_full(&["-c", r#"strptime("%I")"#], Some(input))?;
        assert_eq!(code, 0, "input: {input}, stdout: {stdout:?}");
    }
    Ok(())
}

/// #971: a non-matching `%b`/`%B`/`%h` month name (e.g. `"xyz"`) left
/// `month` at its prior/default value instead of erroring, silently
/// masking malformed input as a valid (defaulted) date -- confirmed live
/// against the pinned jq oracle, which rejects it ("does not match
/// format", exit 5).
#[test]
fn test_strptime_unrecognized_month_name_errors_971() -> Result<()> {
    for fmt in [
        r#"strptime("%b")"#,
        r#"strptime("%B")"#,
        r#"strptime("%h")"#,
    ] {
        let (stdout, _, code) = run_jq_full(&["-c", fmt], Some("\"xyz\""))?;
        assert_eq!(code, 5, "fmt: {fmt}, stdout: {stdout:?}");
    }

    let (stdout, _, code) = run_jq_full(&["-c", r#"strptime("%b")"#], Some("\"Jan\""))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout.trim(), "[1970,0,1,0,0,0,4,0]");
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

// =============================================================================
// #720: `?//` alternative destructuring patterns. All cases live-verified
// against jq 1.7.1.
// =============================================================================

/// The issue's own repro: a pattern-match failure (array pattern against a
/// non-array input) falls through to the next alternative.
#[test]
fn test_as_pattern_alt_falls_through_on_pattern_mismatch_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as [$a] ?// {a: $a} | $a"], Some("[1]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");
    Ok(())
}

/// A body that references `.` (the original, cursor-backed document) rather
/// than only substituted variables -- exercises `try_pattern_alternatives`'s
/// `QueryResult::One`/`Many` arms specifically, which a body built entirely
/// from substituted-literal variable references (like the test above) never
/// reaches.
#[test]
fn test_as_pattern_alt_body_references_original_document_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as [$a] ?// {a: $a} | ."], Some("[1]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1]");
    Ok(())
}

/// Same idea, but the body also fans out (`.[]` on the original,
/// cursor-backed document) -- exercises the `QueryResult::Many` arm
/// specifically, distinct from the single-value `QueryResult::One` case
/// above.
#[test]
fn test_as_pattern_alt_body_fans_out_over_original_document_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as [$a] ?// {a: $a} | .[]"], Some("[1,2]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n2\n");
    Ok(())
}

/// A 3-way alternation, the last (bare-var, always-matches) alternative
/// used as a catch-all fallback.
#[test]
fn test_as_pattern_alt_three_way_chain_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-c", ". as [$a] ?// {a: $a} ?// $a | $a"],
        Some(r#"{"a":5}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "5");
    Ok(())
}

/// An error raised in the matched branch's *body* (not the pattern match
/// itself) also falls through, retrying under the next alternative's
/// bindings -- confirmed live: `jq -c '. as {a:$a} ?// $a | $a + "x"'` on
/// `{"a":1}` gives `object ({"a":1}) and string ("x") cannot be added`,
/// meaning the second alternative (bare `$a`, binding the whole input) is
/// what actually produced the surfaced error, not the first.
#[test]
fn test_as_pattern_alt_falls_through_on_body_error_720() -> Result<()> {
    let (_, stderr, code) = run_jq_full(
        &["-c", r#". as {a: $a} ?// $a | $a + "x""#],
        Some(r#"{"a":1}"#),
    )?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains(r#"object ({"a":1}) and string ("x") cannot be added"#),
        "stderr: {stderr}"
    );
    Ok(())
}

/// The *last* alternative's own error propagates normally once matched --
/// no further fallback exists.
#[test]
fn test_as_pattern_alt_last_alternative_error_propagates_720() -> Result<()> {
    let (_, stderr, code) = run_jq_full(
        &["-c", r#". as $a ?// {a: $a} | $a + "x""#],
        Some(r#"{"a":1}"#),
    )?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains(r#"number (1) and string ("x") cannot be added"#),
        "stderr: {stderr}"
    );
    Ok(())
}

/// Every alternative's pattern fails to match -> the last alternative's
/// own error is the final one.
#[test]
fn test_as_pattern_alt_all_mismatch_errors_720() -> Result<()> {
    let (_, stderr, code) = run_jq_full(&["-c", ". as [$a] ?// {a: $a} | $a"], Some("5"))?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains(r#"Cannot index number with string "a""#),
        "stderr: {stderr}"
    );
    Ok(())
}

/// A variable bound only by a *non-matching* alternative still resolves to
/// `null` in the body, rather than an "undefined variable" error --
/// confirmed live against jq 1.7.1.
#[test]
fn test_as_pattern_alt_unbound_var_from_other_alt_is_null_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as [$a] ?// {b: $b} | [$a, $b]"], Some("[1]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1,null]");

    let (stdout, _, code) = run_jq_full(
        &["-c", ". as [$a] ?// {b: $b} | [$a, $b]"],
        Some(r#"{"b":9}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[null,9]");
    Ok(())
}

/// `?//`'s bind expression can still be a generator -- each output is
/// independently destructured and fanned out, matching `as`'s existing
/// (non-`?//`) generator behavior.
#[test]
fn test_as_pattern_alt_bind_expr_generator_fans_out_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-cn", "(1,2) as [$a] ?// $a | $a"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "1\n2\n");
    Ok(())
}

/// `break`/`halt`/empty output inside the body do *not* trigger
/// fallthrough -- only a genuine `error(...)`/type error does. Confirmed
/// live: `break` propagates cleanly with no output (not caught as a
/// pattern/body failure), and `empty` is genuinely empty output, not an
/// implicit retry signal.
#[test]
fn test_as_pattern_alt_break_and_empty_are_not_fallthrough_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-c", "label $out | (. as {a: $a} ?// $a | (break $out))"],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "");

    let (stdout, _, code) =
        run_jq_full(&["-c", "[. as {a: $a} ?// $a | empty]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[]");
    Ok(())
}

/// A body error's own partial output before it errors is *not* discarded
/// on fallthrough -- each alternative actually tried contributes whatever
/// it managed to produce before failing, not just the last one. Confirmed
/// live: two failing alternatives (a generator body producing `1` then
/// erroring, tried under two different bindings) each contribute their own
/// `1` to the final output stream before the last alternative's error
/// terminates it.
#[test]
fn test_as_pattern_alt_partial_output_survives_fallthrough_720() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#". as {a: $a} ?// $a | (1, error("boom"))"#],
        Some(r#"{"a":1}"#),
    )?;
    assert_ne!(code, 0);
    assert_eq!(stdout, "1\n1\n");
    assert!(stderr.contains("boom"), "stderr: {stderr}");
    Ok(())
}

/// Without `?//`, a bare `$var`/single-pattern `as` binding is completely
/// unaffected by this feature's addition -- same AST shape (`Expr::As`),
/// same behavior as before.
#[test]
fn test_as_pattern_no_alt_unaffected_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as $a | $a"], Some("5"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "5");

    let (stdout, _, code) = run_jq_full(&["-c", ". as [$a,$b] | $a + $b"], Some("[1,2]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "3");
    Ok(())
}

// ============================================================================
// #1366: a pattern binding the same variable name more than once used to
// always keep the *first* occurrence -- an artifact of `substitute_bindings`
// folding `substitute_var` (which replaces every remaining occurrence of a
// name) over the raw, undeduped binding list in pattern order, so only the
// first bind ever found anything left to replace. Real jq's actual rule is
// asymmetric between the two container kinds, confirmed live against jq
// 1.7.1 (not inferred, per CLAUDE.md's #1120 lesson that a fix matching
// only one repro shape can encode the wrong general rule): an **array**
// pattern keeps the *last* position's value; an **object** pattern keeps
// the *first* field's value, tracking the pattern's own field order rather
// than the underlying object's key order.
// ============================================================================

/// #1366: array pattern, own repro plus a 3-position variant with a
/// distinct variable in between (confirms this isn't specific to two
/// *adjacent* duplicate positions).
#[test]
fn test_array_pattern_duplicate_var_keeps_last_position_1366() -> Result<()> {
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as [$a,$a] | $a"], Some("[1,2]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");

    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as [$a,$b,$a] | $a"], Some("[1,2,3]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "3");
    Ok(())
}

/// #1366: object pattern keeps the *first* field's value, and -- the
/// non-obvious half -- this tracks the pattern's own field order, not the
/// object's key order. Both `{"x":1,"y":2}` and the differently-ordered
/// `{"y":2,"x":1}` (confirmed via `keys_unsorted` to actually differ in
/// jq's own eyes) give the same two answers for the two pattern
/// orderings, so the object's key order is not what decides this.
#[test]
fn test_object_pattern_duplicate_var_keeps_first_field_by_pattern_order_1366() -> Result<()> {
    for input in ["{\"x\":1,\"y\":2}", "{\"y\":2,\"x\":1}"] {
        let (stdout, _stderr, code) = run_jq_full(&["-c", ". as {x:$a,y:$a} | $a"], Some(input))?;
        assert_eq!(code, 0, "input: {input}");
        assert_eq!(stdout.trim_end(), "1", "input: {input}");

        let (stdout, _stderr, code) = run_jq_full(&["-c", ". as {y:$a,x:$a} | $a"], Some(input))?;
        assert_eq!(code, 0, "input: {input}");
        assert_eq!(stdout.trim_end(), "2", "input: {input}");
    }
    Ok(())
}

/// #1366: a nested pattern combining both container kinds -- the array
/// sub-pattern's own last-wins resolution (`[$a,$a]` on `[1,2]` -> `2`)
/// becomes field `x`'s single contribution to the outer object pattern,
/// which then keeps *that* (the first field, `x`) over the second field
/// `y`'s own `$a` binding -- confirming the two rules compose per
/// container level rather than being flattened into one global rule.
#[test]
fn test_nested_pattern_duplicate_var_composes_per_container_1366() -> Result<()> {
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as {x:[$a,$a],y:$a} | $a"],
        Some(r#"{"x":[1,2],"y":3}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");
    Ok(())
}

/// #1366: the issue's own `reduce` repro -- `#1201`'s `substitute_bindings`
/// call site shares the same `extract_pattern_bindings` fix, so `reduce`
/// (and by the same code path, `foreach`) must resolve this identically
/// to plain `. as PATTERN`.
#[test]
fn test_reduce_array_pattern_duplicate_var_keeps_last_position_1366() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", "reduce .[] as [$a,$a] (0; $a)"], Some("[[1,2]]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");
    Ok(())
}

/// #1366 code review: the previous test's own doc comment claims `foreach`
/// shares the same code path "by the same code path" as `reduce` -- this
/// pins that claim directly instead of leaving it asserted-but-untested,
/// for both container kinds (`reduce`'s own sibling test above only
/// covers the array case).
#[test]
fn test_foreach_and_reduce_object_pattern_duplicate_var_keeps_first_field_1366() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", "foreach .[] as [$a,$a] (0; $a)"], Some("[[1,2]]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");

    let (stdout, _stderr, code) = run_jq_full(
        &["-c", "foreach .[] as {x:$a,y:$a} (0; $a)"],
        Some(r#"[{"x":1,"y":2}]"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");

    let (stdout, _stderr, code) = run_jq_full(
        &["-c", "reduce .[] as {x:$a,y:$a} (0; $a)"],
        Some(r#"[{"x":1,"y":2}]"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");
    Ok(())
}

/// #1366 code review: `{$a: Pattern}` (#1204's key-shorthand pattern)
/// desugars into two `PatternEntry`s sharing the key `"a"` -- an implicit
/// whole-field bind (`{key:"a", pattern:Var("a")}`) followed by whatever
/// the user's own nested `Pattern` does. When that nested pattern also
/// binds `$a`, this is a same-name collision baked into one syntactic
/// construct rather than two explicit fields, and it happens to already
/// resolve correctly (the auto-bind is the object pattern's *first* field
/// textually, so it wins under the plain, non-`?//` rule) -- pinned here
/// since none of the tests above exercise this desugaring path.
#[test]
fn test_key_shorthand_pattern_duplicate_var_1366() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", ". as {$a: {x:$a}} | $a"], Some(r#"{"a":{"x":99}}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "{\"x\":99}");

    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as {$a: [$b,$a]} | [$a,$b]"],
        Some(r#"{"a":[10,20]}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[[10,20],10]");
    Ok(())
}

/// #1366 regression check: an ordinary pattern with no repeated variable
/// name must still bind each name to its own, independent value --
/// confirms the dedup step doesn't accidentally touch distinct names.
#[test]
fn test_object_pattern_distinct_vars_unaffected_1366() -> Result<()> {
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as {x:$p,y:$q} | [$p,$q]"],
        Some(r#"{"x":100,"y":200}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[100,200]");
    Ok(())
}

/// #1366 code review: a genuine `?//`-chain (2+ alternatives) *inverts*
/// real jq's duplicate-binding dedup rule relative to a bare, non-`?//`
/// pattern of the identical shape -- confirmed live against jq 1.7.1 and
/// 1.8.2. This is not "which code path handles it": `eval_as_pattern`
/// routes a bare pattern through the same `try_pattern_alternatives` as a
/// one-element list, so `patterns.len() > 1` (a real `?//`) is the only
/// thing that actually distinguishes the two cases, and it flips the
/// answer for *both* container kinds.
#[test]
fn test_array_pattern_duplicate_var_inverts_under_alternation_1366() -> Result<()> {
    // Bare: last position wins (`2`).
    let (stdout, _stderr, code) = run_jq_full(&["-c", ". as [$a,$a] | $a"], Some("[1,2]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");

    // Same pattern, trivially `?//`-chained with itself: first position
    // wins instead (`1`).
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", ". as [$a,$a] ?// [$a,$a] | $a"], Some("[1,2]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");

    // Holds for a 3-position pattern too, not just the 2-position case.
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as [$a,$b,$a] ?// [$a,$b,$a] | $a"],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");
    Ok(())
}

/// #1366 code review: the object-pattern counterpart of the array
/// inversion above -- bare keeps the *first* field, `?//` keeps the
/// *last*.
#[test]
fn test_object_pattern_duplicate_var_inverts_under_alternation_1366() -> Result<()> {
    // Bare: first field wins (`1`, x's value).
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", ". as {x:$a,y:$a} | $a"], Some(r#"{"x":1,"y":2}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");

    // Same pattern via `?//` (second alternative never taken, just needs
    // to exist): last field wins instead (`2`, y's value).
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as {x:$a,y:$a} ?// $z | $a"],
        Some(r#"{"x":1,"y":2}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");

    // Holds for a 3-field pattern too.
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as {z:$a,x:$a,y:$a} ?// $w | $a"],
        Some(r#"{"x":1,"y":2,"z":3}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");
    Ok(())
}

/// #1366 code review: the `?//` inversion isn't limited to the outermost
/// container -- a nested sub-pattern's own duplicate-binding rule inverts
/// too, at any depth, since `invert` threads unchanged through every
/// recursive call rather than being computed once at the top.
#[test]
fn test_nested_pattern_duplicate_var_inverts_under_alternation_1366() -> Result<()> {
    // Array nested inside an object, bare vs. `?//`.
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", ". as {x:[$a,$a]} | $a"], Some(r#"{"x":[1,2]}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");

    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as {x:[$a,$a]} ?// $z | $a"],
        Some(r#"{"x":[1,2]}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");

    // Object nested inside an array, bare vs. `?//`.
    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as [{x:$a,y:$a}] | $a"],
        Some(r#"[{"x":1,"y":2}]"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");

    let (stdout, _stderr, code) = run_jq_full(
        &["-c", ". as [{x:$a,y:$a}] ?// $z | $a"],
        Some(r#"[{"x":1,"y":2}]"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");
    Ok(())
}

/// #1366 code review: the inversion holds even when the duplicate-bound
/// alternative is reached via a genuine fallback (the first alternative's
/// own pattern fails to match), not just a trivial self-`?//`.
#[test]
fn test_pattern_duplicate_var_inverts_via_genuine_fallback_1366() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", ". as {x:$a} ?// [$a,$a] | $a"], Some("[1,2]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");
    Ok(())
}

/// yq mode does not support `?//` at all -- real yq's own parser rejects
/// it ("lexer: invalid input text", confirmed live against yq v4.53.3) --
/// so succinctly's shared jq/yq parser must keep erroring on it in yq mode
/// too, rather than silently accepting broader syntax than the oracle.
#[test]
fn test_as_pattern_alt_rejected_in_yq_mode_720() -> Result<()> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    cmd.arg("yq")
        .arg(". as [$a] ?// {a: $a} | $a")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    child.stdin.take().unwrap().write_all(b"a: 5\n")?;
    let output = child.wait_with_output()?;
    assert_ne!(
        output.status.code().unwrap_or(-1),
        0,
        "yq mode must reject ?// as a parse error, matching real yq"
    );
    Ok(())
}

/// `substitute_func_param`'s `Expr::AsPattern` arm: a `$`-parameter used
/// directly as a `?//` binding's own bind expression, inside a
/// parameterized function body.
#[test]
fn test_as_pattern_alt_substituted_as_func_param_bind_expr_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-c", "def f($x): $x as [$a] ?// $a | $a; f(5)"],
        Some("1"),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "5");
    Ok(())
}

/// `expand_func_calls`'s `Expr::AsPattern` arm: a call to a zero-arg
/// function reached only by recursing into a `?//` binding's body.
#[test]
fn test_as_pattern_alt_func_call_inside_body_expanded_720() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", "def f: 1; (. as [$a] ?// $a | f)"], Some("null"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");
    Ok(())
}

// =============================================================================
// #1139: object destructuring pattern shorthand `{$a}` (implicit key from
// var name). Real jq desugars a bare `$var` entry inside an object pattern
// to `key: $var`, where `key` is the variable's own name -- e.g. `{$a}` is
// sugar for `{a: $a}`. All cases live-verified against jq 1.7.1.
// =============================================================================

#[test]
fn test_object_pattern_var_shorthand_bare_1139() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as {$a} | $a"], Some(r#"{"a":1,"b":2}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");
    Ok(())
}

/// A quoted string key (needed for a key that isn't a valid bare
/// identifier, e.g. one containing a space) still works in the non-
/// shorthand branch, unaffected by threading it through the new
/// `(key, pattern)`-tuple restructuring.
#[test]
fn test_object_pattern_string_literal_key_unaffected_1139() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", r#". as {"a b": $x} | $x"#], Some(r#"{"a b":5}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "5");
    Ok(())
}

/// Mixed with an ordinary explicit `key: $var` entry in the same pattern.
#[test]
fn test_object_pattern_var_shorthand_mixed_with_explicit_1139() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-c", ". as {$a, b: $c} | [$a,$c]"],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1,2]");
    Ok(())
}

/// Two shorthand entries in the same pattern.
#[test]
fn test_object_pattern_var_shorthand_multiple_1139() -> Result<()> {
    let (stdout, _, code) =
        run_jq_full(&["-c", ". as {$a, $b} | [$a,$b]"], Some(r#"{"a":1,"b":2}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1,2]");
    Ok(())
}

/// A missing field binds `null`, same as an explicit `key: $var` would.
#[test]
fn test_object_pattern_var_shorthand_missing_key_is_null_1139() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as {$a} | $a"], Some(r#"{"b":2}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "null");
    Ok(())
}

/// Nested inside an array pattern, and nested inside another object
/// pattern -- confirms the shorthand works at any recursion depth for
/// free, since it's handled by the same recursive `parse_pattern` call.
#[test]
fn test_object_pattern_var_shorthand_nested_1139() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as [{$a}] | $a"], Some(r#"[{"a":1}]"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "1");

    let (stdout, _, code) = run_jq_full(&["-c", ". as {a: {$b}} | $b"], Some(r#"{"a":{"b":2}}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "2");
    Ok(())
}

/// The shorthand works as either side of a `?//` alternative (#720):
/// falls through to a shorthand-using second alternative when the first
/// genuinely fails (an array pattern against a non-array input), and is
/// also reachable directly when the first alternative succeeds.
#[test]
fn test_object_pattern_var_shorthand_with_alt_patterns_720_1139() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as [$x] ?// {$a} | $a"], Some(r#"{"a":9}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "9");

    let (stdout, _, code) = run_jq_full(&["-c", ". as [$x] ?// {$a} | $x"], Some("[9]"))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "9");
    Ok(())
}

// =============================================================================
// #1204: object destructuring pattern entry `{$x: Pattern}` (bind and
// further destructure). Real jq's `$IDENT: Pattern` entry binds the
// matched value to `$IDENT` -- same as the `{$x}` shorthand (#1139) --
// *and*, independently, destructures that same (unindexed) value again
// against `Pattern`. Distinct from both `{$x}` (no further destructuring)
// and `key: Pattern` (key is a literal, never a binding). All cases
// live-verified against jq 1.7.1.
// =============================================================================

/// The issue's own repro: `$y` binds by re-destructuring the same value
/// `$x` already bound whole.
#[test]
fn test_object_pattern_var_and_pattern_bare_1204() -> Result<()> {
    let (stdout, _, code) =
        run_jq_full(&["-c", ". as {$x: $y} | [$x,$y]"], Some(r#"{"x":5,"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[5,5]");
    Ok(())
}

/// The nested pattern can itself be an object pattern, reaching into the
/// same value `$x` was bound to whole.
#[test]
fn test_object_pattern_var_and_pattern_nested_object_1204() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-c", ". as {$x: {a: $y}} | [$x,$y]"],
        Some(r#"{"x":{"a":10}}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), r#"[{"a":10},10]"#);
    Ok(())
}

/// The nested pattern can also be an array pattern.
#[test]
fn test_object_pattern_var_and_pattern_nested_array_1204() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-c", ". as {$x: [$a,$b]} | [$x,$a,$b]"],
        Some(r#"{"x":[1,2]}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[[1,2],1,2]");
    Ok(())
}

/// A missing field binds `null` to both `$x` and whatever the nested
/// pattern names, same as the plain shorthand does.
#[test]
fn test_object_pattern_var_and_pattern_missing_key_is_null_1204() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-c", ". as {$x: $y} | [$x,$y]"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[null,null]");
    Ok(())
}

/// A nested pattern that can't match the value's shape still raises the
/// ordinary indexing error, same as any other failing nested pattern.
#[test]
fn test_object_pattern_var_and_pattern_nested_mismatch_errors_1204() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", ". as {$x: {a: $y}} | [$x,$y]"], Some(r#"{"x":5}"#))?;
    assert_ne!(code, 0, "stdout: {stdout:?}");
    assert!(
        stderr.contains("Cannot index number with string"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// The `{$x}` bare shorthand (#1139) and `key: $var` forms remain
/// unaffected in the same pattern as a `{$x: Pattern}` entry.
#[test]
fn test_object_pattern_var_and_pattern_mixed_with_other_forms_1204() -> Result<()> {
    let (stdout, _, code) = run_jq_full(
        &["-c", ". as {$a, $x: $y, b: $c} | [$a,$x,$y,$c]"],
        Some(r#"{"a":1,"x":5,"b":2}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[1,5,5,2]");
    Ok(())
}

/// Works nested inside an array pattern, and at any recursion depth, the
/// same as the plain shorthand does (#1139's own nested test) -- both
/// handled by the same recursive `parse_pattern` call.
#[test]
fn test_object_pattern_var_and_pattern_nested_inside_array_1204() -> Result<()> {
    let (stdout, _, code) =
        run_jq_full(&["-c", ". as [{$x: $y}] | [$x,$y]"], Some(r#"[{"x":7}]"#))?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim_end(), "[7,7]");
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

/// #880: `builtin_in`'s (`in(xs)`) match on its argument's evaluation result
/// had no arm for `Halt`, `Break`, or their `Partial` variants -- all fell
/// into either the `optional` guard or a bogus "in() requires an object or
/// array argument" type error, instead of propagating the escape. Verified
/// against jq 1.7.1 (with stdout/stderr captured separately): `jq -n -c '"a"
/// | in(halt_error(3))'` writes `a` (the raw value `halt_error` writes as
/// its own message) to *stderr*, produces no stdout, and exits 3.
#[test]
fn test_builtin_in_propagates_a_bare_halt_880() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "-c", r#""a" | in(halt_error(3))"#], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "a");
    Ok(())
}

/// Companion to the test above, for `break`: unlike `Halt`, a bare `break`
/// with no enclosing `label` in scope produces no output and a clean exit
/// (real jq's actual behavior once the break has nowhere left to unwind
/// to -- confirmed live). Verified against jq 1.7.1: `jq -n -c 'label $out
/// | ("a" | in(break $out)), "after"'` produces no output, exit 0 -- the
/// break unwinds past the whole comma expression, including `"after"`.
#[test]
fn test_builtin_in_propagates_a_bare_break_880() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-n",
            "-c",
            r#"label $out | ("a" | in(break $out)), "after""#,
        ],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// A `break`/`halt` on a *later* `xs` candidate must not erase the boolean
/// already produced by an *earlier* one (#842's "keep partial output before
/// the escape" precedent, applied here since `in(xs)` now fans out over
/// every `xs` output via `eval_owned_multi_keep_partial`). Verified against
/// jq 1.7.1: `jq -n -c 'label $out | "a" | in({a:1}, break $out)'` prints
/// `true` (from the first candidate) then exits 0 -- the break silently
/// stops the rest of the stream without erroring.
#[test]
// `"in({a:1}, break $out)"` is a jq filter literal, not a formatting
// string; clippy cannot tell the two apart from the brace shape alone.
#[allow(clippy::literal_string_with_formatting_args)]
fn test_builtin_in_keeps_earlier_candidates_before_a_later_break_880() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-n", "-c", r#"label $out | "a" | in({a:1}, break $out)"#],
        None,
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "true\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Same shape as the previous test, for `halt_error` instead of `break`.
/// Verified against jq 1.7.1 (streams captured separately): `jq -n -c '"a" |
/// in({a:1}, halt_error(3))'` prints `true` (the first candidate's result)
/// to stdout, writes `a` (halt_error's own message) to *stderr*, exits 3.
#[test]
// `"in({a:1}, halt_error(3))"` is a jq filter literal, not a formatting
// string; clippy cannot tell the two apart from the brace shape alone.
#[allow(clippy::literal_string_with_formatting_args)]
fn test_builtin_in_keeps_earlier_candidates_before_a_later_halt_880() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-n", "-c", r#""a" | in({a:1}, halt_error(3))"#], None)?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "true\n");
    assert_eq!(stderr, "a");
    Ok(())
}

/// A genuine type-mismatch error on a *later* `xs` candidate behaves the
/// same way as `xs` itself erroring: the stream truncates at that point
/// (real jq's `try`/`?` semantics never resume a generator past a caught
/// error), so an outer `?` keeps only the already-produced candidates and
/// silently drops the rest, rather than either erroring or somehow still
/// reaching a later, otherwise-valid candidate. Verified against jq 1.7.1:
/// `jq -n -c '"a" | in({b:1}, 5, {a:1})?'` prints only `false` (`{b:1}`'s
/// result) -- `{a:1}`'s own `true` is never reached, exit 0.
#[test]
// `"in({b:1}, 5, {a:1})?"` is a jq filter literal, not a formatting
// string; clippy cannot tell the two apart from the brace shape alone.
#[allow(clippy::literal_string_with_formatting_args)]
fn test_builtin_in_optional_truncates_stream_at_first_type_mismatch_880() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "-c", r#""a" | in({b:1}, 5, {a:1})?"#], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "false\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Companion to the test above without `?`: the same type mismatch, but
/// unsuppressed, keeps the already-produced `false` and then raises.
/// Verified against jq 1.7.1: `jq -n -c '"a" | in({b:1}, 5, {a:1})'` prints
/// `false` to stdout, then errors ("Cannot check whether number has a
/// string key"), exit 5.
#[test]
// `"in({b:1}, 5, {a:1})"` is a jq filter literal, not a formatting string;
// clippy cannot tell the two apart from the brace shape alone.
#[allow(clippy::literal_string_with_formatting_args)]
fn test_builtin_in_keeps_earlier_candidates_before_a_later_type_mismatch_error_880() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-n", "-c", r#""a" | in({b:1}, 5, {a:1})"#], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "false\n");
    assert!(
        stderr.contains("Cannot check whether number has a string key"),
        "{stderr}"
    );
    Ok(())
}

/// #966: a leniently-accepted-but-RFC-8259-invalid number (leading zero)
/// no longer echoes its raw source text once materialized through
/// `to_owned`/`OwnedValue` (array construction forces materialization) --
/// it numerically sanitizes instead, matching what real jq would compute
/// for the same digits.
#[test]
fn test_leading_zero_number_sanitizes_on_materialization_966() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "[.a]"], Some(r#"{"a": 007}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[7]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #966: a number with two decimal points was silently materializing as
/// `0`; it now correctly falls through to `null`, since neither `i64` nor
/// `f64` parses it (matching real jq's behavior of rejecting `1.2.3`
/// outright -- succinctly's semi-indexing architecture is deliberately
/// lenient about accepting it as a token in the first place, but the
/// materialized *value* should never silently be a wrong number).
#[test]
fn test_malformed_two_dot_number_becomes_null_not_zero_966() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "[.a]"], Some(r#"{"a": 1.2.3}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[null]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Sanity check that ordinary valid numbers (including ones that share a
/// prefix shape with the invalid cases above -- trailing zero, exponent,
/// bare zero) are completely unaffected by #966's new validity gate.
#[test]
fn test_valid_numbers_unaffected_by_966_validity_gate() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "."],
        Some(r#"{"a": 42, "b": -3.14, "c": 1e10, "d": 0, "e": 1.50}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(
        stdout,
        "{\"a\":42,\"b\":-3.14,\"c\":1E+10,\"d\":0,\"e\":1.50}\n"
    );
    assert_eq!(stderr, "");
    Ok(())
}

/// #966 code review found the fix above didn't reach the CLI's actual
/// *default* output path: `print_json`'s `JqCompatFormatter::format_raw_number`
/// (`src/bin/succinctly/jq_runner.rs`) is what plain field access, `.[]`,
/// and `select(...)` print through, completely bypassing `[.a]`-style
/// materialization -- so `.a` alone was still echoing invalid JSON after
/// the original fix. Now fixed at that call site directly (same
/// `OwnedValue::from_number_bytes` gate, consolidated across every "raw
/// bytes -> number" conversion in the crate).
#[test]
fn test_plain_field_access_sanitizes_leading_zero_966() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ".a"], Some(r#"{"a": 007}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "7\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Same as above for `.[]` iteration over malformed and valid numbers
/// mixed together -- confirms the fix applies per-element, not just to a
/// whole-document shortcut.
#[test]
fn test_array_iteration_sanitizes_malformed_numbers_966() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ".[]"], Some("[007, 1.2.3, 42]"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "7\nnull\n42\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Bare identity (`.`, no computation at all) also sanitizes now --
/// confirms the M2 raw-copy fast path (`StandardJson::stream_json`) isn't
/// actually reachable in default (jq-compat) mode:
/// `OutputConfig::can_use_raw_identity` requires `!jq_compat`, and
/// `jq_compat` defaults to `true`. What plain `.` was actually printing
/// through was `JqCompatFormatter::format_raw_number`, the same call site
/// `test_plain_field_access_sanitizes_leading_zero_966` covers.
#[test]
fn test_bare_identity_sanitizes_leading_zero_966() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "."], Some(r#"{"a": 007}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\":7}\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// `--preserve-input` is a deliberate, documented "keep the exact source
/// formatting" mode (`PreserveFormatter`) -- it's expected to keep echoing
/// a leniently-scanned-but-invalid number verbatim, since that's the
/// whole point of the flag. This isn't the #966 bug; it's confirming the
/// fix didn't overreach into a mode where raw echo is intentional.
#[test]
fn test_preserve_input_still_echoes_raw_number_text() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "--preserve-input", "."], Some(r#"{"a": 007}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\": 007}\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #966 code review found that once a malformed number's cursor
/// materializes to `Null` (`src/jq/lazy.rs::cursor_to_owned`, reached by
/// `jq -e`'s exit-status check), `jq -e '.a'` on a document with a
/// malformed `.a` now correctly reports failure (exit 1) instead of
/// silently succeeding on a value that only *looked* like a number.
#[test]
fn test_exit_status_false_for_malformed_number_966() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-e", "-c", ".a"], Some(r#"{"a": 1.2.3}"#))?;
    assert_eq!(code, 1, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "null\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Arithmetic on a malformed number now fails loudly instead of silently
/// computing against a wrong `0` (the original #966 bug's most dangerous
/// shape -- a query that looks like it succeeded but used fabricated
/// data). Matches real jq's own hard-reject of `1.2.3` as input, just
/// surfaced at the point the value is actually used rather than at parse
/// time (succinctly's semi-indexing architecture defers validation --
/// see docs/architecture/semi-indexing.md).
#[test]
fn test_arithmetic_on_malformed_number_errors_not_silent_zero_966() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ".a - 1"], Some(r#"{"a": 1.2.3}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("cannot be subtracted"),
        "unexpected stderr: {stderr}"
    );
    Ok(())
}

/// #983: `limit`'s own jq definition (`if $n > 0 then ... elif $n == 0
/// then empty else exp end`) treats a negative count as "no limit at
/// all", not an error -- verified against real jq 1.7.1. succinctly used
/// to error instead.
#[test]
fn test_limit_negative_count_is_unlimited_not_error_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-cn", "[limit(-1; 1,2,3)]"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[1,2,3]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Same as above for a `null` count, which jq's total ordering places
/// below every number -- same branch a negative count takes.
#[test]
fn test_limit_null_count_is_unlimited_not_error_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-cn", "[limit(null; 1,2,3)]"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[1,2,3]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// Sanity check that ordinary positive/zero counts are unaffected by
/// #983's fix.
#[test]
fn test_limit_positive_and_zero_count_unaffected_983() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-cn", "[limit(2; 1,2,3)]"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[1,2]\n");

    let (stdout, _, code) = run_jq_full(&["-cn", "[limit(0; 1,2,3)]"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[]\n");
    Ok(())
}

/// #983: two-arg `nth`'s own jq definition errors on a negative index
/// (jq's total ordering makes `null < 0` true too, so both take the same
/// branch) -- verified against real jq 1.7.1 (`nth/2` predates 1.8).
/// succinctly used to silently substitute index 0 instead.
#[test]
fn test_nth_negative_index_errors_not_silent_zero_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-cn", "[1,2,3] | nth(-1; .[])"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("nth doesn't support negative indices"),
        "unexpected stderr: {stderr}"
    );
    Ok(())
}

/// Same as above but with the index sourced from the input document
/// rather than a literal, exercising `builtin_nth_stream`'s other
/// number-extraction branch (`StandardJson::Number` vs `OwnedValue`).
#[test]
fn test_nth_negative_index_from_document_errors_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "nth(.n; .arr[])"],
        Some(r#"{"n": -1, "arr": [1,2,3]}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("nth doesn't support negative indices"),
        "unexpected stderr: {stderr}"
    );
    Ok(())
}

/// Sanity check that an ordinary non-negative index is unaffected.
#[test]
fn test_nth_non_negative_index_unaffected_983() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-cn", "[1,2,3] | nth(1; .[])"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "2\n");
    Ok(())
}

/// #983: jq 1.8's own `skip` definition errors on a negative count
/// (`skip` doesn't exist in the pinned 1.7.1 oracle, but its definition
/// is unambiguous: `else error("skip doesn't support negative count")`).
/// succinctly used to silently return the unskipped stream instead.
#[test]
fn test_skip_negative_count_errors_not_silent_pass_through_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-cn", "[skip(-1; 1,2,3)]"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("skip doesn't support negative count"),
        "unexpected stderr: {stderr}"
    );
    Ok(())
}

/// Sanity check that ordinary positive/zero counts are unaffected.
#[test]
fn test_skip_positive_and_zero_count_unaffected_983() -> Result<()> {
    let (stdout, _, code) = run_jq_full(&["-cn", "[skip(1; 1,2,3)]"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[2,3]\n");

    let (stdout, _, code) = run_jq_full(&["-cn", "[skip(0; 1,2,3)]"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[1,2,3]\n");
    Ok(())
}

/// #983 review: `limit`'s path-context resolver (`resolve_limit`, reached
/// when `limit` appears inside a path/update expression like `|=`) shares
/// `eval_limit`'s n-conversion rule but wasn't updated by the initial fix
/// -- confirmed live against real jq 1.7.1, which gives unlimited
/// passthrough here too.
#[test]
fn test_resolve_limit_negative_count_is_unlimited_not_error_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "(limit(-1; .a, .b) |= 99)"],
        Some(r#"{"a": 1, "b": 2}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "{\"a\":99,\"b\":99}\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #983 review: `limit`'s fix only matched `Int`/`NumberLiteral(Int)`;
/// a negative *float* count fell through to the same error a negative
/// int used to hit. jq's own comparison is type-generic, so a negative
/// float takes the identical "no limit" branch -- verified against real
/// jq 1.7.1.
#[test]
fn test_limit_negative_float_count_is_unlimited_not_error_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-cn", "[limit(-1.5; 1,2,3)]"], None)?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[1,2,3]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #983 review: a bool count argument (`true`/`false`) orders below every
/// number in jq's total ordering, same as `null` -- verified against real
/// jq 1.7.1, both give unlimited passthrough.
#[test]
fn test_limit_bool_count_is_unlimited_not_error_983() -> Result<()> {
    for lit in ["true", "false"] {
        let (stdout, stderr, code) =
            run_jq_full(&["-cn", &format!("[limit({lit}; 1,2,3)]")], None)?;
        assert_eq!(code, 0, "{lit}: stdout: {stdout:?} stderr: {stderr:?}");
        assert_eq!(stdout, "[1,2,3]\n", "{lit}");
        assert_eq!(stderr, "");
    }
    Ok(())
}

/// A *positive* non-integer float count is a separate, pre-existing gap
/// (real jq bounds `limit(1.9; ...)` to 2 outputs via its fractional
/// foreach-decrement loop; succinctly errors) deliberately left untouched
/// by #983, which is scoped to the negative/null/bool "no limit" branch
/// specifically. Pinned here so a future change doesn't accidentally
/// widen #983's fix to also swallow this case silently.
#[test]
fn test_limit_positive_float_count_still_errors_983() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-cn", "[limit(1.9; 1,2,3)]"], None)?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("limit requires non-negative integer"),
        "unexpected stderr: {stderr}"
    );
    Ok(())
}

// ============================================================================
// #845: path()'s Alternative (//) arm required the left operand to be
// path-shaped even when it's simply falsy and gets filtered out by `//`
// itself before path-shape would ever matter.
// ============================================================================

/// #845: a falsy, non-path-shaped left operand falls through to the right
/// side instead of raising #530. Verified against jq 1.7.1: `echo
/// '{"a":10}' | jq -c 'path(false // .b)'` prints `["b"]`, exit 0.
#[test]
fn test_resolve_node_alternative_falsy_non_path_left_falls_through_845() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r"path(false // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"b\"]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #845 companion: `null` (also falsy, also not path-shaped) behaves the
/// same way. Verified against jq 1.7.1: `path(null // .b)` on `{"a":10}`
/// prints `["b"]`, exit 0.
#[test]
fn test_resolve_node_alternative_null_left_falls_through_845() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r"path(null // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"b\"]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #845: a *truthy* non-path-shaped left operand must still raise -- the
/// fix must not turn `//`'s left side into a blanket path-shape exemption.
/// Verified against jq 1.7.1: `path(1 // .b)` on `{"a":10}` raises "Invalid
/// path expression with result 1", exit 5.
#[test]
fn test_resolve_node_alternative_truthy_non_path_left_still_raises_845() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r"path(1 // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Invalid path expression with result 1"));
    Ok(())
}

/// #845: a genuine `error(...)` raised while resolving the left side must
/// still propagate -- `//` only substitutes for falsy/absent output, never
/// for a raised error. Verified against jq 1.7.1: `path(error("x") // .b)`
/// raises `x`, exit 5.
#[test]
fn test_resolve_node_alternative_left_error_still_propagates_845() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"path(error("x") // .b)"#], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains('x'));
    Ok(())
}

/// #845: a truthy, path-shaped left operand is untouched by the fix --
/// `//`'s right side is never even resolved. Verified against jq 1.7.1:
/// `path(.a // .b)` on `{"a":10}` prints `["a"]`, exit 0.
#[test]
fn test_resolve_node_alternative_truthy_path_left_unaffected_845() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r"path(.a // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #845: bare `path(false)` (no `//` at all) still raises -- the fix is
/// scoped to `//`'s own truthy-filtering context, not a general "falsy
/// values are always path-exempt" rule. Verified against jq 1.7.1:
/// `path(false)` on `{"a":10}` raises "Invalid path expression with result
/// false", exit 5.
#[test]
fn test_resolve_node_bare_path_false_still_raises_845() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r"path(false)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Invalid path expression with result false"));
    Ok(())
}

/// #845: the fix only special-cases a bare literal `left` -- a `Comma`
/// fanning out to a truthy, non-path-shaped later sibling takes the
/// original, untouched code path, whose pre-existing prefix-carrying
/// behavior already matches jq exactly on its own. Verified against jq
/// 1.7.1: `path((.a, 1) // .b)` on `{"a":10}` prints `["a"]` before raising
/// on `1`, exit 5.
#[test]
fn test_resolve_node_alternative_keeps_prefix_when_later_sibling_truthy_and_non_path_845(
) -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r"path((.a, 1) // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"a\"]\n");
    assert!(stderr.contains("Invalid path expression with result 1"));
    Ok(())
}

/// #845: `try`/`catch` interaction -- `//`'s own filtering, not `try`'s
/// error-catching, is what suppresses the would-be #530 here, so this
/// still succeeds even without a `catch` clause. Verified against jq
/// 1.7.1: `try (path(false // .b)) catch "caught"` on `{"a":10}` prints
/// `["b"]`, exit 0 (the `try` never even has anything to catch).
#[test]
fn test_resolve_node_alternative_falsy_left_no_catch_needed_845() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"try (path(false // .b)) catch "caught""#],
        Some(r#"{"a":10}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "[\"b\"]\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #845 review round: the fix must not evaluate a non-literal `left` a
/// second time to check its truthiness -- an earlier draft did, via a
/// generic re-evaluation instead of a literal-only static check, and that
/// duplicated any observable side effect `left` had (confirmed live:
/// `path(stderr // .b)` wrote its input to stderr twice under that draft).
/// The shipped fix only special-cases a bare literal (whose value needs no
/// evaluation at all), so a non-literal falsy-*valued* left like `stderr`
/// still takes the original, single-evaluation code path -- this pins
/// "exactly one write", not jq-matching output (that query still diverges
/// from jq for an unrelated, pre-existing reason, tracked as #986).
#[test]
fn test_resolve_node_alternative_does_not_double_evaluate_non_literal_left_845() -> Result<()> {
    let (_stdout, stderr, _code) = run_jq_full(&["-c", "path(stderr // .b)"], Some(r#"{"a":10}"#))?;
    // Count only the portion before the error message, whose own dumped
    // payload also happens to contain the same text -- that's not a second
    // `stderr` write, just the error describing what it saw.
    let before_error = stderr.split("jq: error").next().unwrap_or(&stderr);
    let write_count = before_error.matches(r#"{"a":10}"#).count();
    assert_eq!(
        write_count, 1,
        "stderr should be written exactly once: {stderr:?}"
    );
    Ok(())
}

/// #845 review round: the fix's benefit reaches every other top-level
/// caller of `resolve_node`'s `Alternative` arm, not just `path()` --
/// `del()`, `=`, and `|=` all share the same code. Verified against jq
/// 1.7.1: all three queries below match jq exactly.
#[test]
fn test_alternative_falsy_left_fix_reaches_del_assign_and_update_845() -> Result<()> {
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", "del(false // .b)"], Some(r#"{"a":10,"b":20}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout, "{\"a\":10}\n");

    let (stdout, _stderr, code) =
        run_jq_full(&["-c", "(false // .b) = 99"], Some(r#"{"a":10,"b":20}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout, "{\"a\":10,\"b\":99}\n");

    let (stdout, _stderr, code) = run_jq_full(
        &["-c", "(false // .b) |= . + 1"],
        Some(r#"{"a":10,"b":20}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?}");
    assert_eq!(stdout, "{\"a\":10,\"b\":21}\n");
    Ok(())
}

/// #845 review round: interaction with `catch`'s untracked payload (#843).
/// A falsy literal `left` falls through unconditionally, regardless of
/// trackability (it never attempts navigation into the payload at all);
/// the untracked check still applies normally to whatever `right` does.
/// Verified against jq 1.7.1: `path(try error(false) catch (false // .b))`
/// on `{"a":10}` raises "Invalid path expression near attempt to access
/// element \"b\" of false" (from `.b`'s own untracked-navigation check, not
/// from `false` itself), and `path(try error(false) catch (false // .))`
/// raises the plain `#530` "with result false" (no navigation attempted at
/// all).
#[test]
fn test_resolve_node_alternative_falsy_literal_interacts_correctly_with_untracked_catch_845(
) -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "path(try error(false) catch (false // .b))"],
        Some(r#"{"a":10}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(stderr.contains("near attempt to access element \"b\" of false"));

    let (stdout, stderr, code) = run_jq_full(
        &["-c", "path(try error(false) catch (false // .))"],
        Some(r#"{"a":10}"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(stderr.contains("Invalid path expression with result false"));
    Ok(())
}

// ============================================================================
// #1023: resolve_node's Select/If arms both hand-rolled the identical
// "evaluate cond via eval_owned_multi_keep_partial, fork per output's
// truthiness, collect branches, propagate a branch's own error immediately,
// defer to the cond-level escape once the loop drains" shape. Unified
// behind a shared resolve_cond_fork helper -- these tests pin that the
// unification is behavior-preserving for both arms' pre-existing edge
// cases (multi-output fork, partial-prefix-then-error), not just the
// common case.
// ============================================================================

/// #1023: `Select`'s multi-output cond fork -- confirmed against jq 1.7.1,
/// `path(select(false,false))` (three-arg `select` filters each output
/// independently) is empty, and `path(select(true,true))` resolves twice
/// (`[[],[]]`), matching `If`'s own #628 multi-output behavior next to it.
#[test]
fn test_resolve_node_select_multi_output_fork_1023() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "[path(select(false,false))]"], Some("null"))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[]");

    let (stdout, stderr, code) = run_jq_full(&["-c", "[path(select(true,true))]"], Some("null"))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[[],[]]");
    Ok(())
}

/// #1023: `Select`'s own partial-prefix-then-error case (the counterpart
/// to `test_resolve_node_if_keeps_cond_partial_fanout_before_error_896`
/// just above, for the sibling arm this issue unifies with it) -- a truthy
/// output already resolved before a later `cond` output errors must still
/// surface that resolved branch, not discard it. Confirmed against jq
/// 1.7.1: `path(select(true, error("x")))` prints `[]` before raising `x`.
#[test]
fn test_resolve_node_select_keeps_cond_partial_fanout_before_error_1023() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"path(select(true, error("x")))"#], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[]");
    assert!(stderr.contains('x'));
    Ok(())
}

/// #1023: `Select`/`If` now share `resolve_cond_fork`'s implementation --
/// CLAUDE.md's own "duplicated predicates diverge silently" rule asks for
/// "one definition, plus a test that the call sites agree", not just that
/// each independently matches jq in isolation (code review). `select(cond)`
/// and `if cond then . else empty end` are the same operation expressed
/// two ways, so running the identical partial-fanout-then-error cond
/// through both and asserting they produce byte-identical output directly
/// demonstrates the two arms can no longer silently diverge, since they
/// now run through the same fork loop underneath.
#[test]
fn test_select_and_if_cond_fork_agree_with_each_other_1023() -> Result<()> {
    let (select_stdout, select_stderr, select_code) =
        run_jq_full(&["-c", r#"path(select(true, error("x")))"#], Some("null"))?;
    let (if_stdout, if_stderr, if_code) = run_jq_full(
        &["-c", r#"path(if (true, error("x")) then . else empty end)"#],
        Some("null"),
    )?;
    assert_eq!(
        select_code, if_code,
        "select: {select_stderr:?} if: {if_stderr:?}"
    );
    assert_eq!(select_stdout, if_stdout);
    assert!(select_stderr.contains('x'), "stderr: {select_stderr:?}");
    assert!(if_stderr.contains('x'), "stderr: {if_stderr:?}");
    Ok(())
}

// ============================================================================
// #980: a `Comma`-fanned `//` left operand mixing a falsy/non-path sibling
// with a path-shaped or truthy one used to raise a spurious error --
// `Comma` committed to a sibling's own failure eagerly, before `//`'s
// truthy filter ever got a chance to discard it. Fixed incidentally by
// #1288's `Expr::Comma` fix, not by the `Alternative` arm itself; pinned
// here (as its own closing comment asked) since nothing previously guarded
// it, and #1023's neighboring refactor touches the same machinery cluster.
// All eight shapes confirmed against jq 1.7.1.
// ============================================================================

/// #980: earlier falsy sibling, later path-shaped one.
#[test]
fn test_resolve_node_alternative_comma_falsy_then_path_sibling_980() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path((.a, false) // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[\"a\"]");
    Ok(())
}

/// #980: earlier path-shaped sibling, later falsy one -- order doesn't
/// matter.
#[test]
fn test_resolve_node_alternative_comma_path_then_falsy_sibling_980() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path((false, .a) // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[\"a\"]");
    Ok(())
}

/// #980: a `null` sibling (falsy, distinct from `false`) is treated the
/// same way.
#[test]
fn test_resolve_node_alternative_comma_path_then_null_sibling_980() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path((.a, null) // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[\"a\"]");
    Ok(())
}

/// #980: every sibling falsy (mixed `null`/`false`) -- `//` falls through
/// to the right side entirely.
#[test]
fn test_resolve_node_alternative_comma_all_falsy_siblings_falls_through_980() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path((null, false) // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[\"b\"]");

    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path((false, false) // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[\"b\"]");
    Ok(())
}

/// #980: two path-shaped siblings, no falsy one involved at all -- both
/// survive.
#[test]
fn test_resolve_node_alternative_comma_two_path_shaped_siblings_980() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "[path((.a, .c) // .b)]"],
        Some(r#"{"a":10,"c":20}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[[\"a\"],[\"c\"]]");
    Ok(())
}

/// #980: a *missing* field (`.x`, absent from the input) is falsy (`null`)
/// exactly like a literal `false` sibling -- discarded the same way, not a
/// distinct "two path-shaped siblings" case despite `.x` itself being a
/// navigation step. Confirmed against jq 1.7.1: `path((.x, .a) // .b)`
/// prints only `["a"]`, the missing-field output never surviving `//`'s
/// truthy filter.
#[test]
fn test_resolve_node_alternative_comma_missing_field_sibling_is_falsy_980() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "[path((.x, .a) // .b)]"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[[\"a\"]]");
    Ok(())
}

/// #980: three siblings, falsy/path/falsy -- the fix must hold across more
/// than two outputs, not just a pair.
#[test]
fn test_resolve_node_alternative_comma_three_siblings_mixed_980() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "[path((.a, false, .a) // .b)]"],
        Some(r#"{"a":10}"#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[[\"a\"],[\"a\"]]");
    Ok(())
}

/// #980 boundary: a later sibling that is truthy but *not* path-shaped
/// still raises -- the fix only lets *falsy* siblings through `//`'s
/// filter uncontested, it doesn't exempt every non-path-shaped value.
/// This is #845's own pre-existing behavior, unaffected by #980's fix;
/// pinned here alongside the other seven shapes so the boundary between
/// "silently filtered" and "still raises" is guarded in one place.
#[test]
fn test_resolve_node_alternative_comma_truthy_non_path_sibling_still_raises_980() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "path((.a, 1) // .b)"], Some(r#"{"a":10}"#))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[\"a\"]");
    assert!(stderr.contains("Invalid path expression with result 1"));
    Ok(())
}

// ============================================================================
// #891: resolve_leaf's non-primitive fallback used a bespoke message for a
// multi-output result (`range(3)` used bare inside `path(...)`) instead of
// the same "#530" wording its single-output sibling already uses, naming
// the first output -- matching real jq's own per-output-checked laziness,
// which raises on the first non-path-shaped value and never even reaches
// the rest.
// ============================================================================

/// #891: `path(range(3))` now names the first output (`0`), matching real
/// jq's own wording exactly, instead of the old bespoke "Cannot use a
/// computed index after a multi-output path component" message. Verified
/// against jq 1.7.1: `echo null | jq -c 'path(range(3))'` raises "Invalid
/// path expression with result 0", exit 5.
#[test]
fn test_resolve_leaf_multi_output_names_first_value_891() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "path(range(3))"], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Invalid path expression with result 0"));
    Ok(())
}

/// #891 companion: the same fix for a multi-output non-primitive whose
/// first offending value is itself a container (`paths(...)`'s array
/// output), not a scalar -- confirms the message uses the *rendered* first
/// value, not just a bare number. Verified against jq 1.7.1: raises
/// "Invalid path expression with result [0]", exit 5.
#[test]
fn test_resolve_leaf_multi_output_names_first_container_value_891() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"path(paths(if type=="string" then error("my custom message") else true end))"#,
        ],
        Some(r#"[1,2,"trigger"]"#),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Invalid path expression with result [0]"));
    Ok(())
}

/// #891 review round: routing the multi-output case through
/// `EvalError::invalid_path_expression` (the same `#530` constructor the
/// single-output arm already used) has a second, previously-untested effect
/// beyond the message text -- `EvalError::is_invalid_path_expression()` is a
/// string-prefix check, so `?`/`try`/`catch` now correctly stop suppressing
/// or catching this error, matching real jq. Before this fix, the bespoke
/// message didn't match that prefix, so `?` wrongly swallowed the error
/// entirely and `try`/`catch` wrongly ran the catch handler. Verified
/// against jq 1.7.1: both queries below raise "Invalid path expression with
/// result 0", exit 5 -- `?` does not suppress it, and `catch`'s handler
/// never runs.
#[test]
fn test_resolve_leaf_multi_output_error_survives_optional_and_try_catch_891() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "path((range(3))?)"], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Invalid path expression with result 0"));

    let (stdout, stderr, code) =
        run_jq_full(&["-c", r#"path(try range(3) catch "x")"#], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Invalid path expression with result 0"));
    // If `catch`'s handler wrongly ran (the pre-fix bug), the message would
    // instead name its own output: `Invalid path expression with result "x"`.
    assert!(
        !stderr.contains("result \"x\""),
        "catch handler should never run: {stderr:?}"
    );
    Ok(())
}

/// #891 review round companion: the same fix reached through `del(...)`,
/// not just `path(...)`. Verified against jq 1.7.1: `del((range(3))?)` on
/// `null` raises "Invalid path expression with result 0", exit 5 -- `?`
/// does not turn this into a silent no-op.
#[test]
fn test_del_multi_output_error_survives_optional_891() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "del((range(3))?)"], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert!(stderr.contains("Invalid path expression with result 0"));
    Ok(())
}

/// `[[[...1...]]]`, `depth` levels of array nesting wrapping a single `1`.
fn nested_arrays(depth: usize) -> String {
    format!("{}1{}", "[".repeat(depth), "]".repeat(depth))
}

/// #1016: a self-recursive `def` has no base case at expansion time --
/// `expand_func_calls` statically substitutes `deep`'s body in place of
/// each call *before* any evaluation happens, so it can't observe that
/// `n == 0` will eventually hold and unrolls unconditionally. Confirmed
/// live before this fix: `deep(1)` crashed with SIGABRT (stack overflow)
/// even at the shallowest possible depth, since expansion never terminates
/// on its own regardless of `n`'s actual value.
///
/// `n = 60` safely exceeds `MAX_FUNC_EXPANSION_DEPTH` (50), so this must
/// still fail -- but cleanly, as a catchable `EvalError`, not a SIGABRT
/// that takes the whole process down.
#[test]
fn test_self_recursive_def_rejects_past_expansion_depth_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "def deep(n): if n == 0 then . else [deep(n-1)] end; deep(60)",
        ],
        Some("null"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(
        stderr.contains("recursion depth exceeds limit of 50"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// Companion to the above: a self-recursive `def` well within
/// `MAX_FUNC_EXPANSION_DEPTH` must still evaluate correctly, byte-for-byte
/// matching real jq 1.7.1's own output for the same query (confirmed live)
/// -- the guard must not reject ordinary, ceiling-respecting recursion.
#[test]
fn test_self_recursive_def_accepts_depth_under_limit_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "def deep(n): if n == 0 then . else [deep(n-1)] end; deep(40)",
        ],
        Some("null"),
    )?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(
        stdout.trim_end(),
        "[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[[null]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]]"
    );
    Ok(())
}

/// Pins the exact boundary rather than just "well under"/"well over": for
/// this single-argument shape, `deep(49)` is the largest `n` that succeeds
/// and `deep(50)` is the first that fails -- confirmed live. A future
/// change to the budget check (e.g. `>` instead of `>=`, or a shifted
/// increment) would silently move this boundary by one with nothing to
/// catch it if only the "well within"/"well past" tests above existed.
#[test]
fn test_self_recursive_def_boundary_is_exactly_49_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "def deep(n): if n == 0 then . else [deep(n-1)] end; deep(49)",
        ],
        Some("null"),
    )?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(
        stdout.trim_end(),
        format!("{}null{}", "[".repeat(49), "]".repeat(49))
    );

    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "def deep(n): if n == 0 then . else [deep(n-1)] end; deep(50)",
        ],
        Some("null"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(
        stderr.contains("recursion depth exceeds limit of 50"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// A zero-argument self-recursive `def` (no growing argument expression to
/// substitute) is the *cheapest possible* case for `expand_func_calls`'s own
/// recursion, yet still crashed at only ~383 levels in a debug build before
/// this fix (measured while calibrating `MAX_FUNC_EXPANSION_DEPTH`) -- it
/// has no base case at all, so it must hit the depth guard on every build,
/// not just adversarially deep ones. Confirms the guard covers this shape
/// too, not just the parameterized one above.
#[test]
fn test_unconditional_self_recursive_def_rejects_cleanly_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "def deep: [deep]; deep"], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(
        stderr.contains("recursion depth exceeds limit of 50"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// Non-recursive `def`s, and defs that recurse via a hand-written `Builtin`
/// (`recurse`, not a user-defined self-call), must stay completely
/// unaffected by `MAX_FUNC_EXPANSION_DEPTH` -- confirms the guard is scoped
/// to `expand_func_calls`'s own self-reference arm, not a blanket limit on
/// every recursive-looking query.
#[test]
fn test_non_self_recursive_constructs_unaffected_by_expansion_guard_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "def inc: . + 1; 5 | inc"], Some("null"))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "6");

    let (stdout, stderr, code) = run_jq_full(&["-c", "[recurse]"], Some("[1,[2,[3]]]"))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[[1,[2,[3]]],1,[2,[3]],2,[3],3]");
    Ok(())
}

/// #1016 code review: a `def` body with *more than one* syntactic self-call
/// (branching self-recursion, e.g. naive `fib`) defeats a plain per-chain
/// depth counter -- every structural arm visiting multiple children (`+`'s
/// two operands here) passes the same depth to each, so `k` self-calls per
/// level compound to `O(k^depth)` total substitutions even though no single
/// chain exceeds the cap. Confirmed live before this fix: `def f: [f, f];
/// f` consumed tens of GB and never terminated. `MAX_FUNC_EXPANSION_DEPTH`
/// must bound *total* expansion work (a shared budget, not a per-chain
/// value) to stay safe for this shape too -- this must return quickly with
/// a clean error, not hang or exhaust memory. Real jq resolves `fib(3)`
/// instantly (`2`); this guard's shared-budget design means even this
/// shallow, everyday case can't succeed under static expansion (see
/// `MAX_FUNC_EXPANSION_DEPTH`'s doc comment) -- but it must fail cleanly.
#[test]
fn test_branching_self_recursive_def_bounded_not_exponential_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "def fib(n): if n < 2 then n else fib(n-1) + fib(n-2) end; fib(3)",
        ],
        Some("null"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(
        stderr.contains("recursion depth exceeds limit of 50"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// #1016 code review: sharing *one* global substitution counter for the
/// entire `then` expression (an intermediate design between the per-chain
/// counter and the final per-occurrence one) starves every later,
/// syntactically-independent call to the same function once an earlier one
/// has consumed the budget. Confirmed live before this fix: three
/// completely independent, shallow calls to the same recursive `fact` --
/// none of which come close to the depth limit on their own -- failed
/// outright, while real jq 1.7.1 returns `[6,24,120]` for the same query.
#[test]
fn test_independent_sibling_calls_do_not_share_budget_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "def fact(n): if n <= 1 then 1 else n * fact(n-1) end; [fact(3), fact(4), fact(5)]",
        ],
        Some("null"),
    )?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), "[6,24,120]");
    Ok(())
}

/// #1016 code review: `MAX_FUNC_EXPANSION_DEPTH` only bounds how many
/// times the `FuncCall` self-reference arm fires, not the native stack
/// depth needed to walk from one self-reference to the next -- a `def`
/// body that wraps its recursive call in substantial structure (here, 20
/// levels of array nesting) overflows the native stack (SIGABRT) well
/// before that budget is anywhere near exhausted, confirmed live before
/// this fix. `MAX_FUNC_EXPANSION_CHAIN_DEPTH` bounds raw recursion depth
/// directly and must catch this case too, cleanly, not just the thin-body
/// shape the other tests in this group use.
#[test]
fn test_thickly_wrapped_self_recursive_def_bounded_by_chain_depth_1016() -> Result<()> {
    let wrap_open = "[".repeat(20);
    let wrap_close = "]".repeat(20);
    let query = format!(
        "def deep(m): if m == 0 then . else {wrap_open}deep(m-1){wrap_close} end; deep(60)"
    );
    let (stdout, stderr, code) = run_jq_full(&["-c", &query], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(
        stderr.contains("nesting exceeds depth limit of 300"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// #1016 patch-coverage: `expand_func_calls_in_builtin`'s ~150-arm match
/// forwards `budget`/`chain_depth` identically at every arm, verified
/// mechanically correct by /code-review's exhaustive line-by-line trace of
/// every call site -- but a query only ever *reaches* the specific arms it
/// happens to use, so most of that mechanical diff went untouched by any
/// existing test. This wraps a self-recursive `def` around a broad, varied
/// sample of `Builtin` variants (path/object, array, math, string, regex,
/// streaming) spanning categories the existing #1016 tests never touch, to
/// exercise a meaningfully wider slice of those arms in one pass rather
/// than adding a near-duplicate test per builtin.
#[test]
fn test_self_recursive_def_wrapping_varied_builtins_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            r#"def cover(x): if x == 0 then null else
  [
    cover(x-1),
    ({"a":1} | has("a")),
    ("a" | in({"a":1})),
    ({"a":1} | path(.a)),
    ({"a":1} | getpath(["a"])),
    ({} | setpath(["a"]; 1)),
    ({"a":1} | delpaths([["a"]])),
    ([[1,2],[3]] | flatten),
    ([3,1,2] | sort_by(.)),
    ([1,1,2] | unique_by(.)),
    ([1,2,3] | group_by(. % 2)),
    ([1,2,3] | min_by(.)),
    ([1,2,3] | max_by(.)),
    (3.7 | floor),
    (3.2 | ceil),
    (3.5 | round),
    (4 | sqrt),
    (2 | exp),
    (2 | log),
    ("ABC" | ascii_downcase),
    ("abc" | ascii_upcase),
    ("abcdef" | ltrimstr("abc")),
    ("abcdef" | rtrimstr("def")),
    ("a,b,c" | split(",")),
    ("abc" | test("b")),
    ("abc" | match("b") | .string),
    ("abc" | capture("(?<x>b)")),
    ("aba" | sub("a";"X")),
    ("aba" | gsub("a";"X")),
    ([1,2,3] | limit(2; .[])),
    ([1,[2,[3]]] | [recurse])
  ]
end;
cover(1)"#,
        ],
        Some("null"),
    )?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(
        stdout.trim_end(),
        r#"[null,true,true,["a"],1,{"a":1},{},[1,2,3],[1,2,3],[1,2],[[2],[1,3]],1,3,3,4,4,2,7.38905609893065,0.6931471805599453,"abc","ABC","def","abc",["a","b","c"],true,"b",{"x":"b"},"Xba","XbX",1,2,[[1,[2,[3]]],1,[2,[3]],2,[3],3]]"#
    );
    Ok(())
}

/// #1016 patch-coverage: `expand_func_calls`'s arity-mismatch check (right
/// above the depth guard this PR adds, in the same `FuncCall` match arm)
/// had zero direct test coverage anywhere in this crate before this PR --
/// confirmed by grepping for its exact message. Calling a `def` with the
/// wrong number of arguments must still surface that pre-existing, clean
/// `EvalError` rather than reaching (or being masked by) the new depth
/// guard added just below it in the same arm.
#[test]
fn test_func_call_arity_mismatch_reports_clean_error_1016() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "def f(x): x; f(1;2)"], Some("null"))?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert!(
        stderr.contains("function f takes 1 arguments, got 2"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// #998: the bare identity `.` never materializes an `OwnedValue` tree (it
/// stays lazy, streaming straight from the cursor via `print_json`), so
/// `eval_generic::to_owned`'s own depth guard never gets a chance to fire
/// for it -- confirmed live before this fix, `succinctly jq '.'` on a
/// 200,000-level-deep document aborted with a raw stack overflow (SIGABRT,
/// exit 134), not a clean error. `print_json` needs its own guard.
#[test]
fn test_identity_query_rejects_adversarial_nesting_998() -> Result<()> {
    let input = nested_arrays(500);
    let (_stdout, stderr, code) = run_jq_full(&["-c", "."], Some(&input))?;
    assert_eq!(code, 1, "stderr: {stderr:?}");
    assert!(
        stderr.contains("nesting depth exceeds limit of 256"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// Companion to the above: legitimately-nested input well under the limit
/// must still round-trip exactly, unaffected by the new guard.
#[test]
fn test_identity_query_accepts_nesting_under_limit_998() -> Result<()> {
    let input = nested_arrays(100);
    let (stdout, stderr, code) = run_jq_full(&["-c", "."], Some(&input))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), input);
    Ok(())
}

/// #998 review: `--exit-status`/`-e` forces `JqValue::materialize()` on
/// every result before `print_json`'s own guarded output path ever runs,
/// reaching `lazy.rs`'s independent `cursor_to_owned` materializer
/// directly -- a second, unguarded recursive tree-walker with the exact
/// same shape as `eval_generic::to_owned_cursor`, missed by this PR's own
/// first pass. Confirmed live before this follow-up fix: `succinctly jq -e
/// '.[0]'` on a 200,000-level-deep document raw-stack-overflowed (SIGABRT,
/// exit 134) even with `print_json`'s guard already in place.
#[test]
fn test_exit_status_query_rejects_adversarial_nesting_998() -> Result<()> {
    let input = nested_arrays(500);
    let (_stdout, stderr, code) = run_jq_full(&["-e", "-c", ".[0]"], Some(&input))?;
    assert_eq!(code, 101, "stderr: {stderr:?}");
    assert!(
        stderr.contains("nesting depth exceeds limit of 256"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// Companion to the above: `-e` on ordinary (non-adversarial) object input
/// still round-trips correctly through `cursor_to_owned`'s `Object` arm,
/// not just its `Array` arm.
#[test]
fn test_exit_status_query_materializes_object_998() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-e", "-c", "."], Some(r#"{"a":{"b":1}}"#))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(stdout.trim_end(), r#"{"a":{"b":1}}"#);
    Ok(())
}

/// #1008 (yq PR) code review: `format_number_jq_compat`'s `value == 0.0`/
/// `value as i64` checks don't distinguish -0.0 from 0.0 (IEEE 754), so a
/// negative-zero exponent literal silently lost its sign across all three
/// of the function's branches (`e0`/`e-0`, small negative exponents, and
/// general scientific notation). Pre-existing and reachable via plain JSON
/// input (nothing YAML-specific about it), just newly exposed by that PR's
/// widening of a YAML-side predicate -- fixed at the source so `jq` mode's
/// own output is also correct, not just yq's.
#[test]
fn test_negative_zero_exponent_literal_preserves_sign_1008() -> Result<()> {
    for (input, want) in [
        (r#"{"a": -0e0}"#, "-0"),
        (r#"{"a": -0e10}"#, "-0E+10"),
        (r#"{"a": -0e5}"#, "-0E+5"),
        (r#"{"a": 0e10}"#, "0E+10"),
    ] {
        let (out, code) = run_jq_stdin(".a", input, &[])?;
        assert_eq!(code, 0, "for {input:?}: {out:?}");
        assert_eq!(out.trim(), want, "for {input:?}");
    }
    Ok(())
}

/// #1030 (`#1008` follow-up): `numeric_display_string`/`to_json_yq` gained
/// yq-mode-only verbatim echo -- confirm jq mode's own `tostring`/`@json`/
/// string interpolation keep `format_number_jq_compat`'s reformatting
/// unchanged (uppercase `E`, forced sign), not accidentally picking up
/// yq's verbatim-echo branch.
#[test]
fn test_jq_tostring_and_json_and_interpolation_unaffected_by_yq_verbatim_echo_1030() -> Result<()> {
    for (filter, want) in [
        (".a | tostring", r#""1E+2""#),
        (".a | @json", r#""1E+2""#),
        (r#""\(.a)""#, r#""1E+2""#),
    ] {
        let (out, code) = run_jq_stdin(filter, r#"{"a": 1e2}"#, &[])?;
        assert_eq!(code, 0, "for {filter:?}: {out:?}");
        assert_eq!(out.trim(), want, "for {filter:?}");
    }
    Ok(())
}

/// #953: top-level `[...]` array construction (`eval.rs`'s native
/// `Expr::Array` cursor arm) never reaches `to_json_for_reindex` at all, so
/// this doesn't exercise the reindex-bridge fix directly -- kept as a
/// baseline sanity check that jq's own array construction is unaffected.
/// See `test_jq_reduce_accumulator_unaffected_by_yq_float_fraction_fix_953`
/// below for a case that actually reaches the bridge.
#[test]
fn test_jq_array_construction_unaffected_by_yq_float_fraction_fix_953() -> Result<()> {
    for (filter, input, want) in [
        ("[.a * 1.0]", r#"{"a": 5}"#, "[5]"),
        ("[.a / 5]", r#"{"a": 5}"#, "[1]"),
    ] {
        let (out, code) = run_jq_stdin(filter, input, &["-c"])?;
        assert_eq!(code, 0, "for {filter:?}: {out:?}");
        assert_eq!(out.trim(), want, "for {filter:?}");
    }
    Ok(())
}

/// #953 code review: `to_json_for_reindex`'s `Float` fallback must format
/// through the yq-only `format_float_with_fraction` behind an
/// `S::TAG == EvalTag::Yq` gate, not unconditionally -- an earlier draft
/// applied it unconditionally, which silently regressed this exact case in
/// **jq** mode. `reduce`'s owned accumulator (`1.0 + 4.0` computes a plain,
/// non-`NumberLiteral` `Float` with no fast path in
/// `eval_owned_fast_path`) genuinely reaches `to_json_for_reindex` each
/// iteration, unlike the top-level `[...]` case above. Real jq gives
/// `[[[5]]]` (a computed value loses its literal formatting entirely, per
/// jq's own normalize-computed-floats convention) -- confirmed live against
/// the pinned oracle.
#[test]
fn test_jq_reduce_accumulator_unaffected_by_yq_float_fraction_fix_953() -> Result<()> {
    let (out, code) = run_jq_stdin("[reduce (1,2) as $i (1.0 + 4.0; [.])]", "{}", &["-c"])?;
    assert_eq!(code, 0, "{out:?}");
    assert_eq!(out.trim(), "[[[5]]]");
    Ok(())
}

/// #1051: `builtin_stderr` gained an `S: EvalSemantics` parameter so yq mode
/// can echo a `NumberLiteral` verbatim; confirm jq mode's own container
/// formatting (`format_number_jq_compat`'s uppercase-`E` reformatting) is
/// unaffected, matching real jq.
#[test]
fn test_jq_stderr_and_halt_error_unaffected_by_yq_mode_fix_1051() -> Result<()> {
    let (_stdout, stderr, code) =
        run_jq_stdin_streams(".a | stderr | empty", r#"{"a": [1e2, "x"]}"#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(stderr.trim_end(), r#"[1E+2,"x"]"#);

    let (_stdout, stderr, _code) =
        run_jq_stdin_streams(".a | halt_error", r#"{"a": [1e2, "x"]}"#, &[])?;
    assert_eq!(stderr.trim_end(), r#"[1E+2,"x"]"#);
    Ok(())
}

/// #1060: `numeric_display_string`'s NaN/Infinity fast path gained an
/// `S`-gated yq-only branch (`.nan`/`.inf`/`-.inf`); confirm jq mode's own
/// spelling is unaffected by that yq-only addition. jq mode's own spelling
/// was, at the time #1060 landed, still Rust's bare `f64::Display`
/// (`NaN`/`inf`/`-inf`) -- a separate, pre-existing divergence from real
/// jq's actual `DBL_MAX`/`null` substitution, since fixed by #1075. This test
/// now pins that corrected, oracle-matching jq-mode output instead.
#[test]
fn test_jq_tostring_special_floats_unaffected_by_yq_mode_fix_1060() -> Result<()> {
    for (filter, want) in [
        ("infinite | tostring", r#""1.7976931348623157e+308""#),
        (
            "(-1 * infinite) | tostring",
            r#""-1.7976931348623157e+308""#,
        ),
        ("nan | tostring", r#""null""#),
    ] {
        let (out, code) = run_jq_stdin(filter, "null", &["-c"])?;
        assert_eq!(code, 0, "for {filter:?}: {out:?}");
        assert_eq!(out.trim(), want, "for {filter:?}");
    }
    Ok(())
}

/// #1087: a computed Infinity's direct `-c`/pretty JSON output (not just
/// `tostring`'s *text*-format path, already correct since #1075) was
/// `null` regardless of sign -- `JqCompatFormatter` (the default) and
/// `PreserveFormatter` (`--preserve-input`) in `jq_runner.rs` both
/// hand-rolled the same `is_nan() || is_infinite() => "null"` collapse.
/// Confirmed live against jq 1.7.1: `null | infinite` is
/// `1.7976931348623157e+308`, not `null`; only NaN has no such
/// fallback text. Covers both formatters, since #1087 found the identical
/// bug independently duplicated in each.
#[test]
fn test_jq_infinite_direct_json_output_matches_real_jq_1087() -> Result<()> {
    for extra_args in [vec!["-c"], vec!["-c", "--preserve-input"]] {
        for (filter, want) in [
            ("infinite", "1.7976931348623157e+308"),
            ("(-1) * infinite", "-1.7976931348623157e+308"),
            ("[infinite]", "[1.7976931348623157e+308]"),
            (r#"{"a":infinite}"#, r#"{"a":1.7976931348623157e+308}"#),
            (
                "[1, infinite] | join(\",\")",
                "\"1,1.7976931348623157e+308\"",
            ),
            ("nan", "null"),
            ("[nan]", "[null]"),
        ] {
            let (out, code) = run_jq_stdin(filter, "null", &extra_args)?;
            assert_eq!(code, 0, "for {filter:?} {extra_args:?}: {out:?}");
            assert_eq!(out.trim(), want, "for {filter:?} {extra_args:?}");
        }
    }
    Ok(())
}

/// #950's yq-only strict numeric equality must not leak into jq mode: jq
/// has no strict int/float distinction, so `2.0 == 2` stays `true` (real
/// jq 1.7.1, verified). Pins that `apply_compare_op`'s `S::
/// STRICT_NUMERIC_EQUALITY` gate is correctly `false` under `JqSemantics`.
#[test]
fn test_jq_equality_still_widens_int_and_float_950() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ". == 2"], Some("2.0"))?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "true\n");
    assert_eq!(stderr, "");
    Ok(())
}

/// #1065's yq-only "scalar slices to an empty array" rule must not leak
/// into jq mode: real jq gives no output for `.[0:1]?` on a number (matches
/// succinctly's pre-existing, unaffected behavior).
#[test]
fn test_jq_slice_number_scalar_still_gives_no_output_1065() -> Result<()> {
    let (output, code) = run_jq_stdin(".[0:1]?", "5", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "");
    Ok(())
}

/// #1101's yq-only "assigning through a scalar slice target is a no-op"
/// rule must not leak into jq mode: real jq errors on `.[0:1] = 99` for a
/// number target, matching succinctly's pre-existing, unaffected behavior.
#[test]
fn test_jq_slice_assign_scalar_still_errors_1101() -> Result<()> {
    let (_out, code) = run_jq_stdin(".[0:1] = 99", "5", &[])?;
    assert_eq!(code, 5);
    Ok(())
}

/// `@urid`'s percent-decode loop must round-trip non-ASCII UTF-8 correctly
/// -- both literal pass-through bytes (no `%` escapes present at all) and
/// genuinely percent-decoded multi-byte sequences. Before #1123, pushing
/// each raw byte individually via `bytes[i] as char` mis-encoded any byte
/// at or above 0x80 as its own Latin-1 codepoint, corrupting the string
/// even though decoding a plain string with no escapes should be a no-op.
#[test]
fn test_urid_nonascii_passthrough_and_decode_1123() -> Result<()> {
    let (out, code) = run_jq_stdin("@urid", r#""café""#, &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "\"café\"");

    let (out, code) = run_jq_stdin("@urid", r#""caf%C3%A9""#, &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "\"café\"");
    Ok(())
}

/// #1119's yq-only "array + non-array appends" rule must not leak into jq
/// mode: real jq has no array-append concept and errors on `[] + 99`,
/// matching succinctly's pre-existing, unaffected behavior.
#[test]
fn test_jq_array_plus_scalar_still_errors_1119() -> Result<()> {
    let (_out, code) = run_jq_stdin("[] + 99", "null", &[])?;
    assert_eq!(code, 5);

    let (_out, code) = run_jq_stdin("[1,2] + 3", "null", &[])?;
    assert_eq!(code, 5);
    Ok(())
}

/// `arith_add`'s `null + x` / `x + null` passthrough arm must not collapse
/// a `NumberLiteral` operand's own source spelling before the arm even
/// runs (#1143). Not directly observable as the raw literal text in jq
/// mode -- jq's own number formatting (`format_number_jq_compat`) always
/// normalizes a `NumberLiteral`'s exponent/trailing-zero spelling on
/// output -- but that normalization only fires for a value that actually
/// stayed a `NumberLiteral`; a `Float`/`Int` that lost its literal
/// upstream renders via plain `f64`/`i64::to_string()` instead, which
/// diverges on exactly these inputs (confirmed against the pre-fix
/// binary: `null + 1e10` printed `10000000000`, `null + 3.00` printed
/// `3` -- both silently losing the literal).
#[test]
fn test_jq_null_plus_number_literal_preserves_literal_path_1143() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-cn", "null + 1e10"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "1E+10");

    let (out, _, code) = run_jq_full(&["-cn", "1e10 + null"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "1E+10");

    let (out, _, code) = run_jq_full(&["-cn", "null + 3.00"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "3.00");
    Ok(())
}

/// Control: a genuinely *computed* sum must still get canonical formatting
/// -- #1143's fix only defers `into_plain_number()` for the passthrough/
/// append arms, not for the arm that actually adds two numbers.
#[test]
fn test_jq_genuine_arithmetic_still_reformats_1143() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-cn", "3.00 + 1"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "4");

    let (out, _, code) = run_jq_full(&["-cn", "1 + 1.500"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "2.5");
    Ok(())
}

/// `add` is documented as `[.[] | .]` folded with `+` (see `builtin_add`'s
/// own doc comment), so it inherits `arith_add`'s fix automatically -- a
/// `null` element folded against a `NumberLiteral` must preserve the
/// literal's spelling the same way a bare `null + <literal>` does (#1143).
#[test]
fn test_jq_add_builtin_preserves_number_literal_through_null_fold_1143() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-cn", "[null, 1.500] | add"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "1.500");
    Ok(())
}

/// `.a += x` desugars to `.a |= . + x`, so a `null` field's compound-assign
/// also goes through `arith_add`'s null-passthrough arm and must preserve
/// the RHS literal's spelling (#1143).
#[test]
fn test_jq_compound_assign_plus_preserves_number_literal_on_null_1143() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", ".a += 1.500"], Some(r#"{"a":null}"#))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), r#"{"a":1.500}"#);
    Ok(())
}

/// Real jq 1.7.1 errors on every `null`-involving multiplication, with no
/// exceptions (`NULL_MERGES_AS_EMPTY` is yq-only, so jq mode never takes
/// any of `arith_mul`'s no-op/merge arms) -- #1175. Supersedes a prior
/// version of this test that locked in succinctly's then-current (wrong)
/// `null`-returning behavior. Covers both operand orders, a number
/// literal (`1e10`, to guard against a literal-spelling special case),
/// scalars of every remaining type, `null * null`, and containers
/// (`{}`/`[]`) on both sides -- the last two of which yq mode treats as a
/// no-op/empty-container-merge instead (see the yq-side tests), so jq's
/// unconditional error here is exactly what distinguishes the two modes.
#[test]
fn test_jq_null_times_anything_errors_1175() -> Result<()> {
    for expr in [
        "1e10 * null",
        "null * 1e10",
        "5 * null",
        "null * 5",
        r#"null * "ab""#,
        "null * true",
        "null * null",
        "null * {}",
        "null * []",
        "{} * null",
        "[] * null",
    ] {
        let (out, err, code) = run_jq_full(&["-cn", expr], None)?;
        assert_eq!(code, 5, "expr {expr:?}: out={out:?} err={err:?}");
        assert!(err.contains("cannot be multiplied"), "expr {expr:?}: {err}");
    }
    Ok(())
}

// --- #1199: a `NumberLiteral` operand's source spelling must survive into
// a binary-op type-mismatch error message, not just the *value* returned
// on success -- found broader than #1175's own repro (which only exposed
// this on a `null`-involving `*` pairing): every arith_* function's
// catch-all error arm previously called `into_plain_number()` on both
// operands before ever building the error, discarding a `NumberLiteral`'s
// own text (e.g. `1E+10`) in favor of its canonically-reformatted value
// (`10000000000`). Verified live against real jq 1.7.1 for every operator.

/// Every operator's type-mismatch error preserves a `NumberLiteral`
/// operand's own spelling, not the reformatted value.
#[test]
fn test_jq_binary_op_error_preserves_number_literal_spelling_1199() -> Result<()> {
    for (expr, op_wording) in [
        ("1e10 + {}", "cannot be added"),
        ("1e10 - {}", "cannot be subtracted"),
        ("1e10 * {}", "cannot be multiplied"),
        ("1e10 / {}", "cannot be divided"),
        ("1e10 % {}", "cannot be divided (remainder)"),
    ] {
        let (out, err, code) = run_jq_full(&["-cn", expr], None)?;
        assert_eq!(code, 5, "expr {expr:?}: out={out:?} err={err:?}");
        assert!(
            err.contains("number (1E+10)"),
            "expr {expr:?}: expected spelling preserved, got: {err}"
        );
        assert!(err.contains(op_wording), "expr {expr:?}: {err}");
    }
    Ok(())
}

/// The right operand's spelling is preserved too, not just the left's --
/// checked for every operator, not just `+`, since each `arith_*`
/// function's catch-all arm was restructured independently and a mistake
/// in any one of them (e.g. a `&right`/`&left` swap) wouldn't be caught by
/// checking only one operator.
#[test]
fn test_jq_binary_op_error_preserves_right_operand_spelling_1199() -> Result<()> {
    for (expr, op_wording) in [
        ("{} + 1e10", "cannot be added"),
        ("{} - 1e10", "cannot be subtracted"),
        ("{} * 1e10", "cannot be multiplied"),
        ("{} / 1e10", "cannot be divided"),
        ("{} % 1e10", "cannot be divided (remainder)"),
    ] {
        let (out, err, code) = run_jq_full(&["-cn", expr], None)?;
        assert_eq!(code, 5, "expr {expr:?}: out={out:?} err={err:?}");
        assert!(
            err.contains("number (1E+10)"),
            "expr {expr:?}: expected spelling preserved, got: {err}"
        );
        assert!(err.contains(op_wording), "expr {expr:?}: {err}");
    }
    Ok(())
}

/// `divisor_is_zero` (`/`'s and `%`'s own dedicated error, distinct from
/// the generic type-mismatch `binary_op`) has the identical bug, reached
/// from *inside* an otherwise-successful numeric arm rather than a
/// trailing catch-all -- confirms the fix threads through that path too,
/// not just the type-mismatch one. Both `/` and `%` check for their own
/// distinguishing wording, not just the shared "number (X)" spelling
/// preservation both `binary_op` and `divisor_is_zero` would show
/// identically -- a regression that misrouted either operator's
/// zero-divisor case to the generic type-mismatch arm instead (losing the
/// "divisor is zero" phrasing) would otherwise still pass.
#[test]
fn test_jq_divisor_is_zero_error_preserves_number_literal_spelling_1199() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "1e10 / 0"], None)?;
    assert_eq!(code, 5, "out: {out:?}");
    assert!(err.contains("number (1E+10)"), "{err}");
    assert!(
        err.contains("cannot be divided because the divisor is zero"),
        "{err}"
    );

    let (out, err, code) = run_jq_full(&["-cn", "1e10 % 0"], None)?;
    assert_eq!(code, 5, "out: {out:?}");
    assert!(err.contains("number (1E+10)"), "{err}");
    assert!(
        err.contains("cannot be divided (remainder) because the divisor is zero"),
        "{err}"
    );
    Ok(())
}

/// `divisor_is_zero`'s `(Int, Float)` and `(Float, Float)` arms specifically
/// -- the sibling test above only exercises `(Int, Int)`/`1e10 % 0`
/// (`NumberLiteral`-repr `Int`). Direct coverage for the two other
/// zero-divisor shapes `arith_div`'s `number_repr()`-based match handles.
#[test]
fn test_jq_divisor_is_zero_mixed_int_float_shapes_1199() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "5 / 0.0"], None)?;
    assert_eq!(code, 5, "out: {out:?}");
    assert!(
        err.contains("cannot be divided because the divisor is zero"),
        "{err}"
    );

    let (out, err, code) = run_jq_full(&["-cn", "5.5 / 0.0"], None)?;
    assert_eq!(code, 5, "out: {out:?}");
    assert!(
        err.contains("cannot be divided because the divisor is zero"),
        "{err}"
    );
    Ok(())
}

/// Control: genuine successful computation (every operator) is unaffected
/// by the restructuring -- the numeric fast path still reaches the same
/// arms it always did, just guarded on a non-consuming peek first.
#[test]
fn test_jq_binary_op_success_paths_unaffected_1199() -> Result<()> {
    for (expr, expected) in [
        ("5 + 3", "8"),
        ("5 - 3", "2"),
        ("5 * 3", "15"),
        ("10 / 4", "2.5"),
        ("10 % 3", "1"),
        (r#""ab" * 3"#, r#""ababab""#),
        (r#""a,b,c" / ",""#, r#"["a","b","c"]"#),
        ("[1,2,3] - [2]", "[1,3]"),
    ] {
        let (out, err, code) = run_jq_full(&["-cn", expr], None)?;
        assert_eq!(code, 0, "expr {expr:?}: {err}");
        assert_eq!(out.trim(), expected, "expr {expr:?}");
    }
    Ok(())
}

/// A `Float` (or a `NumberLiteral` carrying a float repr) paired with a
/// `String` for `*` must never panic -- regression guard for a bug an
/// earlier draft of this fix introduced and code review caught before
/// merge. `arith_mul`'s restructuring initially gated its numeric/
/// string-repetition arm behind `is_number()` (true for `Float` too, not
/// just `Int`), but the inner match's string-repetition pattern only ever
/// handled `Int`, so a `Float`+`String` pair fell through every named arm
/// into an `unreachable!()` that was, in fact, reachable -- `"ab" * 2.5`
/// crashed the process (exit 101) instead of erroring (exit 5).
///
/// At the time this test was written, succinctly had no float-repeat-count
/// support at all, so "doesn't panic" meant "errors cleanly" (exit 5,
/// "cannot be multiplied"). #1230 added that support (truncating toward
/// zero, matching real jq's own `intmax_t` cast), so a moderate float count
/// now succeeds instead -- that success path is covered by
/// `test_jq_string_repetition_accepts_float_count_1230` above. This test
/// keeps only the still-relevant "doesn't panic" cases: `nan` and negative
/// `infinite` repeat counts, both of which now succeed cleanly (Rust's `as
/// i64` cast saturates `NaN` to `0` and `-inf` to `i64::MIN`, so neither
/// reaches `String::repeat` with an unreasonable count -- `i64::MIN` is
/// negative, so it takes the existing negative-count-returns-null branch
/// instead).
///
/// Positive `infinite` (and a very large finite magnitude like `1e10`) are
/// deliberately not covered here: truncating either yields a huge repeat
/// count fed to `String::repeat`, an unbounded-allocation hang/OOM hazard,
/// not a panic -- and that exact hazard already exists, unguarded, for a
/// same-magnitude `Int` operand (`"ab" * 10000000000`) on `main` today, so
/// it's a pre-existing characteristic of this arm rather than something
/// #1230 introduced. Live-verified against real jq: `timeout 3 jq -cn
/// '(infinite) * "ab"'` hangs (no output, no error) rather than rejecting
/// the input, so there's no clean "must error" behavior to pin here either.
///
/// Note real jq's own `nan * "ab"` returns `null` (verified: jq-1.7.1),
/// not `""` -- but that's `(int)NaN` C-cast undefined behavior (platform-
/// dependent; jq has no explicit NaN check here), not a deliberate
/// contract, so succinctly's well-defined saturating-cast result (`""`) is
/// intentionally not matched to it.
#[test]
fn test_jq_string_times_float_errors_cleanly_not_panics_1199() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-cn", "(nan) * \"ab\""], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "\"\"");

    let (out, _, code) = run_jq_full(&["-cn", "\"ab\" * (0 - infinite)"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "null");
    Ok(())
}

// --- #1171: document-input parsing must not silently produce wrong
// output for content real jq either accepts leniently (a leading-dot
// number) or rejects outright (truncated/malformed JSON) -- both
// previously exited 0 with silently wrong output on the default (lazy)
// input path.

/// A leading-dot number literal (`.5`), top-level and nested, must parse
/// as the number it spells -- real jq's own reader is lenient beyond
/// strict JSON here (confirmed live: `jq -c '.'` on `.5` outputs `0.5`).
#[test]
fn test_jq_leading_dot_number_parses_correctly_1171() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "."], Some(".5"))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "0.5");

    let (out, _, code) = run_jq_full(&["-c", "."], Some("[.5]"))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "[0.5]");

    let (out, _, code) = run_jq_full(&["-c", "."], Some(r#"{"a":.5}"#))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), r#"{"a":0.5}"#);

    Ok(())
}

/// Truncated/malformed JSON must error (exit 5), not silently produce
/// empty output with exit 0 -- `find_json_values` previously skipped
/// unparseable content and kept going with no error recorded at all.
#[test]
fn test_jq_truncated_json_errors_not_silent_empty_output_1171() -> Result<()> {
    let (out, stderr, code) = run_jq_full(&["-c", "."], Some("[1,2,"))?;
    assert_eq!(code, 5, "out: {out:?}, stderr: {stderr:?}");
    assert!(out.trim().is_empty(), "out: {out:?}");
    assert!(!stderr.trim().is_empty(), "expected a diagnostic on stderr");

    Ok(())
}

/// A bare `.` (no digit at all) must still be rejected -- the leading-dot
/// widening above requires at least one digit to follow, matching real
/// jq's own boundary (confirmed live: `jq -c '.'` on a bare `.` errors
/// with "Invalid numeric literal", not accepted as some default number).
#[test]
fn test_jq_bare_dot_still_errors_1171() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "."], Some("."))?;
    assert_eq!(code, 5, "out: {out:?}");
    assert!(out.trim().is_empty(), "out: {out:?}");

    Ok(())
}

/// Regression guard: ordinary, well-formed multi-value document input is
/// unaffected by the stricter error handling above.
#[test]
fn test_jq_valid_multi_value_stream_unaffected_1171() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "."], Some("1\n2\n3\n"))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "1\n2\n3");

    Ok(())
}

/// A top-level token that only *looks* number-shaped but has no digit
/// anywhere in its mantissa (`-e5`: `-` then `e5`, no digit before the
/// exponent marker) must error, not silently materialize as `null` --
/// `number_literal_end`'s digit-position validation, not just "does it
/// contain a digit somewhere" (confirmed live: real jq errors with
/// "Invalid numeric literal", exit 5). Found by code review before merge.
#[test]
fn test_jq_dash_e_digit_top_level_errors_not_silent_null_1171() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "."], Some("-e5"))?;
    assert_eq!(code, 5, "out: {out:?}");
    assert!(out.trim().is_empty(), "out: {out:?}");

    Ok(())
}

/// A top-level token with a valid mantissa but an exponent marker with
/// no digit after it (`1e`) must error too -- rejected outright, not
/// truncated to just the valid `1` prefix, matching real jq (confirmed
/// live: `jq -c '.'` on `1e` errors with "Invalid numeric literal", not
/// `1`). Distinct code path from `-e5` above (that one has no mantissa
/// digit at all; this one's mantissa is fine, only the exponent is
/// incomplete).
#[test]
fn test_jq_incomplete_exponent_top_level_errors_1171() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "."], Some("1e"))?;
    assert_eq!(code, 5, "out: {out:?}");
    assert!(out.trim().is_empty(), "out: {out:?}");

    Ok(())
}

/// Trailing zeros on a leading-dot literal's fractional part must be
/// preserved, not collapsed -- `.500` -> `0.500`, not `0.5` (confirmed
/// live: real jq's own reader adds the leading `0` but keeps trailing
/// zeros verbatim, same as it does for a strictly-valid decimal). Covers
/// both the positive and negative-sign spellings. Found by code review
/// before merge (`OwnedValue::from_number_bytes` was collapsing to a
/// plain lossy `Float` instead of a spelling-preserving `NumberLiteral`).
#[test]
fn test_jq_leading_dot_number_preserves_trailing_zeros_1171() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "."], Some(".500"))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "0.500");

    let (out, _, code) = run_jq_full(&["-c", "."], Some("-.500"))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "-0.500");

    Ok(())
}

/// A malformed number *nested* inside an otherwise well-formed container
/// keeps this crate's own established #966 precedent (materialize as
/// `null`, don't error the whole document) even after #1171's stricter
/// top-level handling -- the two code paths (`find_json_values`'s
/// top-level document splitter vs. `light.rs`'s per-value materializer)
/// deliberately have different error-vs-null conventions. Regression
/// guard: an earlier draft of #1171's fix used one shared, strict
/// number-span function for both paths, which truncated `1.2.3` after
/// `1.2` and silently fabricated the wrong value `1.2` instead of `null`
/// (caught by review before merge -- see `nested_number_span`'s own doc
/// comment in `src/json/light.rs`).
#[test]
fn test_jq_nested_malformed_number_still_becomes_null_not_fabricated_1171() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "[.a]"], Some(r#"{"a": 1.2.3}"#))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "[null]");

    Ok(())
}

/// #1116's yq-only "chained scalar-slice-assign no-ops" / "del() deletes
/// the parent key" rules must not leak into jq mode: real jq errors on
/// both `.a[0:1] = 99` and `del(.a[0:1])` for a scalar `.a`, matching
/// succinctly's pre-existing, unaffected behavior.
#[test]
fn test_jq_chained_scalar_slice_assign_and_del_still_error_1116() -> Result<()> {
    let input = r#"{"a":5,"b":6}"#;

    let (_out, code) = run_jq_stdin(".a[0:1] = 99", input, &[])?;
    assert_eq!(code, 5);

    let (_out, code) = run_jq_stdin("del(.a[0:1])", input, &[])?;
    assert_eq!(code, 5);
    Ok(())
}

/// #1153's `delete_at_path` `Expr::Paren` arm is general (not gated by
/// `EvalSemantics`), so a plain parenthesized delete target now works in
/// jq mode too, matching real jq (`del((.a))` succeeds there). But the
/// *other* half of #1153 -- `yq_del_slice_outcome`'s
/// `unwrap_paren` fix -- is itself gated to yq mode by its caller
/// (`S::TAG == EvalTag::Yq`), so a parenthesized chained scalar-slice
/// target must still error in jq mode exactly like its unparenthesized
/// form does (#1116's jq-mode guard above), not silently start no-oping.
#[test]
fn test_jq_parenthesized_del_target_works_but_chained_slice_still_errors_1153() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "del((.a))"], Some(r#"{"a":5,"b":6}"#))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), r#"{"b":6}"#);

    let (_out, _, code) = run_jq_full(&["-c", "del((.a[0:1]))"], Some(r#"{"a":5,"b":6}"#))?;
    assert_eq!(code, 5);
    Ok(())
}

/// #1170: `to_entries` on a JSON object with a repeated key must collapse
/// it to one entry -- keeping the first occurrence's position but the
/// last occurrence's value -- matching real jq (oracle-verified against
/// jq 1.7.1: `{"a":1,"b":2,"a":3}|to_entries` is
/// `[{"key":"a","value":3},{"key":"b","value":2}]`), not the raw,
/// undeduplicated per-token walk this crate previously used (which showed
/// three entries, one per JSON token occurrence).
#[test]
fn test_jq_to_entries_deduplicates_repeated_json_key_1170() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "to_entries"], Some(r#"{"a":1,"b":2,"a":3}"#))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(
        out.trim(),
        r#"[{"key":"a","value":3},{"key":"b","value":2}]"#
    );
    Ok(())
}

/// #1170 regression guard: a plain object with no repeated keys is
/// unaffected by the dedup logic.
#[test]
fn test_jq_to_entries_no_duplicate_keys_unaffected_1170() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", "to_entries"], Some(r#"{"x":1,"y":2}"#))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(
        out.trim(),
        r#"[{"key":"x","value":1},{"key":"y","value":2}]"#
    );
    Ok(())
}

/// #1251: `.foo` field access on a duplicate JSON key must resolve to the
/// *last* value, matching real jq / RFC 8259 convention -- this used to
/// return the first (oracle-verified against jq 1.7.1:
/// `{"a":1,"b":2,"a":3}|.a` is `3`).
#[test]
fn test_jq_field_access_duplicate_key_last_wins_1251() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-c", ".a"], Some(r#"{"a":1,"b":2,"a":3}"#))?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "3");
    Ok(())
}

/// #1230: `"x" * n` accepts a float repeat count, truncating toward zero
/// like real jq's own `intmax_t` cast, instead of erroring as a type
/// mismatch.
#[test]
fn test_jq_string_repetition_accepts_float_count_1230() -> Result<()> {
    let (out, _, code) = run_jq_full(&["-cn", "2.5 * \"ab\""], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "\"abab\"");

    let (out, _, code) = run_jq_full(&["-cn", "2.9 * \"ab\""], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "\"abab\"");

    let (out, _, code) = run_jq_full(&["-cn", "\"ab\" * 0.5"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "\"\"");

    let (out, _, code) = run_jq_full(&["-cn", "\"ab\" * (0.0 - 1.5)"], None)?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "null");
    Ok(())
}

/// A destructuring pattern nested past the parser's depth limit exits
/// cleanly with a compile error instead of overflowing the process stack --
/// regression test for #1240, reachable from query text alone (no large
/// input document needed). Mirrors the issue's own live repro, which used a
/// 100k-deep pattern -- reproduced directly against the CLI binary and
/// confirmed there, but scaled down to just past the 256-deep limit here:
/// passing a 100k-deep string as a literal argv entry exceeds Linux's
/// ARG_MAX in CI ("Argument list too long", os error 7) well before the
/// query is ever parsed, which is an unrelated process-spawn limit, not
/// this fix's own behavior.
#[test]
fn test_as_pattern_deep_nesting_exits_cleanly_not_stack_overflow_1240() -> Result<()> {
    let n = 300;
    let pattern = format!("{}$x{}", "{a: ".repeat(n), "}".repeat(n));
    let query = format!(". as {pattern} | $x");
    let (out, err, code) = run_jq_full(&["-cn", &query], None)?;
    assert_ne!(code, 0, "out={out:?} err={err:?}");
    assert!(err.contains("depth limit"), "err={err}");
    Ok(())
}

/// A destructuring pattern nested under an absent/null field resolves every
/// bound variable to `null`, matching real jq, instead of hard-erroring --
/// regression test for #1239. Mirrors the issue's own live repro exactly.
#[test]
fn test_destructuring_pattern_null_propagates_through_nested_object_1239() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-c", ". as {x: {y: $y}} | $y"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "null");
    Ok(())
}

#[test]
fn test_destructuring_pattern_null_propagates_through_nested_array_1239() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-c", ". as {x: [$y]} | $y"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "null");
    Ok(())
}

/// #1201's own two repros, end-to-end through the CLI: `reduce`/`foreach`'s
/// `as` clause takes a full destructuring pattern, not just a bare `$var`.
/// The happy paths are pinned byte-for-byte against real jq by the
/// `reduce_as_*`/`foreach_as_*` golden cases; these exist so the issue's
/// literal reproduction commands stay covered from the binary's own entry
/// point, and so a bare `$var` binding is shown not to have regressed.
#[test]
fn test_reduce_as_full_pattern_1201() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", "reduce .[] as {a: $a} (0; . + $a)"],
        Some(r#"[{"a":1},{"a":2}]"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "3");

    let (out, err, code) = run_jq_full(
        &["-c", "reduce .[] as [$a,$b] (0; . + $a + $b)"],
        Some("[[1,2],[3,4]]"),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "10");

    // A bare `$var` is just `Pattern::Var` now -- unaffected.
    let (out, err, code) = run_jq_full(&["-c", "reduce .[] as $x (0; . + $x)"], Some("[1,2,3]"))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "6");
    Ok(())
}

/// #1201: `foreach` emits one output per step, so a pattern that fails to
/// match partway through the input must still leave every earlier step's
/// output in place -- the same "keep the prefix" contract #494 pinned for an
/// ordinary per-step UPDATE error, now holding for this *new* per-element
/// error source. Checked here at the CLI boundary (not just as an in-process
/// `QueryResult::Partial`) because what matters is that the bytes are
/// actually written to stdout before the process exits non-zero.
#[test]
fn test_foreach_as_pattern_error_keeps_prefix_on_stdout_1201() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", "foreach .[] as {a:$a} (0; . + $a; .)"],
        Some(r#"[{"a":1},{"a":2},"bad",{"a":4}]"#),
    )?;
    assert_eq!(code, 5, "out={out:?} err={err:?}");
    assert_eq!(out.lines().collect::<Vec<_>>(), ["1", "3"]);
    assert!(err.contains("Cannot index string with string"), "err={err}");
    Ok(())
}

/// #1201 routes `reduce`/`foreach`'s binding clause through the *same*
/// `parse_pattern` that `. as PATTERN` uses, so #1240's `MAX_PATTERN_DEPTH`
/// guard covers the two new construction sites for free. This pins that:
/// without the shared entry point, a deeply nested pattern here would
/// recurse to a stack overflow instead of a clean compile error. Cannot be a
/// golden case -- real jq has no equivalent limit, so its output would
/// legitimately differ. Scaled to just past the 256-deep limit for the same
/// ARG_MAX reason as `..._1240` above.
#[test]
fn test_reduce_as_pattern_respects_pattern_depth_limit_1201() -> Result<()> {
    let n = 300;
    let pattern = format!("{}$x{}", "{a: ".repeat(n), "}".repeat(n));
    let query = format!("reduce .[] as {pattern} (0; $x)");
    let (out, err, code) = run_jq_full(&["-cn", &query], None)?;
    assert_ne!(code, 0, "out={out:?} err={err:?}");
    assert!(err.contains("depth limit"), "err={err}");
    Ok(())
}

/// #1201 deliberately scoped out `?//` alternatives in the `reduce`/`foreach`
/// clause: real jq accepts them, but retrying an alternative after the body
/// errors requires rolling the accumulator back to the element's pre-UPDATE
/// value, which the fold can't express. Tracked by #1365.
///
/// This pins the *current* divergence so it can't drift silently -- it is
/// expected to fail, and should be replaced by a behaviour test, when #1365
/// lands.
#[test]
fn test_reduce_foreach_reject_pattern_alternatives_1201() -> Result<()> {
    for query in [
        "reduce .[] as [$a] ?// {a:$a} (0; . + $a)",
        "foreach .[] as [$a] ?// {a:$a} (0; . + $a; .)",
    ] {
        let (out, err, code) = run_jq_full(&["-c", query], Some(r#"[[1],{"a":2}]"#))?;
        assert_ne!(code, 0, "`{query}` should not compile\nout={out:?}");
        assert!(err.contains("compile error"), "`{query}`\nerr={err}");
    }
    Ok(())
}

// =============================================================================
// #1164: a builtin argument generator that produces a value and *then*
// breaks/errors no longer silently continues past that escape once the
// builtin has finished using the value -- `result_to_owned`/
// `eval_owned_expr_ctrl`'s own `Partial` arm used to drop the trailing
// control entirely. All cases below live-verified against real jq (or, for
// `tz` -- a succinctly-only extension with no jq equivalent -- checked for
// internal consistency against the same builtin without the trailing
// escape).
//
// The pattern throughout: `label $out | (builtin((v, break $out)), "after")`
// -- real jq computes the builtin's result using `v`, produces it, and only
// then unwinds to `$out`, so `"after"` never prints. Before this fix,
// succinctly silently dropped the escape and printed both the builtin's
// result *and* `"after"`.
// =============================================================================

#[test]
fn test_ltrimstr_uses_first_arg_value_then_propagates_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (ltrimstr(("a", break $out)), "after")"#,
        ],
        Some(r#""abcabc""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), r#""bcabc""#);
    Ok(())
}

#[test]
fn test_rtrimstr_uses_first_arg_value_then_propagates_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (rtrimstr(("c", break $out)), "after")"#,
        ],
        Some(r#""abcabc""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), r#""abcab""#);
    Ok(())
}

#[test]
fn test_startswith_endswith_split_propagate_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (startswith(("a", break $out)), "after")"#,
        ],
        Some(r#""abcabc""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "true");

    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (endswith(("c", break $out)), "after")"#,
        ],
        Some(r#""abcabc""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "true");

    let (out, err, code) = run_jq_full(
        &["-c", r#"label $out | (split((",", break $out)), "after")"#],
        Some(r#""abcabc""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), r#"["abcabc"]"#);
    Ok(())
}

#[test]
fn test_join_uses_first_separator_value_then_propagates_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#"label $out | (join((",", break $out)), "after")"#],
        Some(r#"["a","b"]"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), r#""a,b""#);
    Ok(())
}

#[test]
fn test_contains_inside_propagate_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (contains(("a", break $out)), "after")"#,
        ],
        Some(r#""abc""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "true");

    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (inside(("abc", break $out)), "after")"#,
        ],
        Some(r#""a""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "true");
    Ok(())
}

/// The issue's own repro for the deeper (not just bare-Break) case: real
/// jq's own downstream error still wins over a trailing break the argument
/// generator's second output would have raised -- `has`'s own type-mismatch
/// error fires first, since the break's own generator output is never
/// reached (control test, unaffected by this fix but confirming the "own
/// error wins" rule this fix relies on).
#[test]
fn test_has_propagates_trailing_break_after_success_but_not_after_own_error_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#"label $out | (has(("a", break $out)), "after")"#],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "true");

    let (_out, err, code) = run_jq_full(
        &["-c", r#"label $out | (has(("a", break $out)), "after")"#],
        Some("5"),
    )?;
    assert_ne!(code, 0);
    assert!(err.contains("Cannot check"), "err={err}");
    Ok(())
}

#[test]
fn test_nth_uses_first_arg_value_then_propagates_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#"label $out | (nth((1, break $out)), "after")"#],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "2");
    Ok(())
}

#[test]
fn test_flatten_depth_uses_first_arg_value_then_propagates_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#"label $out | (flatten((1, break $out)), "after")"#],
        Some("[[1,[2]]]"),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "[1,[2]]");
    Ok(())
}

#[test]
fn test_getpath_uses_first_arg_value_then_propagates_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (getpath((["a"], break $out)), "after")"#,
        ],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "1");
    Ok(())
}

#[test]
fn test_strftime_strptime_propagate_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (gmtime | strftime(("%Y", break $out)), "after")"#,
        ],
        Some("0"),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), r#""1970""#);

    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (strptime(("%Y-%m-%d", break $out)), "after")"#,
        ],
        Some(r#""2020-01-01""#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "[2020,0,1,0,0,0,3,0]");
    Ok(())
}

/// `tz` is a succinctly-only extension (no real jq equivalent to verify
/// against) -- checked instead for internal consistency: the same query
/// without the trailing break prints `"after"` normally, confirming the
/// break (not some unrelated bug) is what suppresses it below.
#[test]
fn test_tz_uses_first_arg_value_then_propagates_trailing_break_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", r#"label $out | (0 | tz("UTC")), "after""#], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"1970-01-01T00:00:00Z\"\n\"after\"");

    let (out, err, code) = run_jq_full(
        &[
            "-cn",
            r#"label $out | (0 | tz(("UTC", break $out))), "after""#,
        ],
        None,
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"1970-01-01T00:00:00Z\"");
    Ok(())
}

#[test]
fn test_load_uses_first_arg_value_then_propagates_trailing_break_1164() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    writeln!(file, r#"{{"x":1}}"#)?;
    let path = file.path().to_str().unwrap();

    let query = format!(r#"label $out | (load(("{path}", break $out))), "after""#);
    let (out, err, code) = run_jq_full(&["-cn", &query], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), r#"{"x":1}"#);
    Ok(())
}

// `builtin_envvar`'s own fix (below) has no CLI-level test: `Builtin::EnvVar`
// has no parser construction site anywhere in this codebase (confirmed by
// that function's own pre-existing Halt/Break regression tests in
// src/jq/eval.rs) -- `env.VAR`/`$ENV.VAR`/`$ENV["VAR"]` all resolve through
// ordinary field/index access on a materialized object instead, never
// through this function. Its #1164 fix is covered by a direct unit test in
// src/jq/eval.rs instead, mirroring those same pre-existing tests' own
// "exercise it directly, since the CLI can't reach it" convention.

/// Control: `error(msg)`'s own "success" path already collapses into
/// producing an error, matching what the trailing break would have done
/// anyway -- deliberately left unfixed by #1164 (verified live: real jq's
/// `try error(("a", break $out)) catch (.)` still prints `"after"` too, the
/// break is never re-observed once the error is caught, so there is no
/// separate escape to propagate here). Confirms this fix didn't
/// accidentally change `error`'s own behavior.
#[test]
fn test_error_message_arg_break_semantics_unaffected_by_1164() -> Result<()> {
    let (_out, err, code) =
        run_jq_full(&["-cn", r#"label $out | error(("a", break $out))"#], None)?;
    assert_ne!(code, 0);
    assert!(err.contains('a'), "err={err}");
    Ok(())
}

/// #1164 coverage: the `optional` (`?`) arm of a wrong-typed argument is a
/// separate branch from the non-optional error arm every other test above
/// already exercises -- `?` suppresses the type mismatch to no output
/// (`QueryResult::None`) for each of these builtins' argument-evaluation
/// gate, independent of whether the trailing-control fix applies at all.
#[test]
fn test_argument_type_mismatch_optional_arms_produce_no_output_1164() -> Result<()> {
    for (expr, input) in [
        ("getpath(\"notarray\")?", r#"{"a":1}"#),
        ("gmtime | strftime(5)?", "0"),
        ("strptime(5)?", r#""x""#),
        ("tz(5)?", "0"),
    ] {
        let (out, err, code) = run_jq_full(&["-c", expr], Some(input))?;
        assert_eq!(code, 0, "expr={expr}: err={err}");
        assert_eq!(out, "", "expr={expr}: out={out:?}");
    }

    let (out, err, code) = run_jq_full(&["-cn", "load(5)?"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

/// #1164 coverage: a negative `flatten` depth still errors even when the
/// argument generator that produced it also has a trailing break -- the
/// error arm wins outright (own-error-supersedes-argument's-escape, same
/// rule as `has`'s control test above), independent of the guard on the
/// success arm every other `flatten` test above already exercises.
#[test]
fn test_flatten_negative_depth_errors_even_with_trailing_break_1164() -> Result<()> {
    let (_out, err, code) = run_jq_full(
        &["-c", r"label $out | flatten((-1, break $out))"],
        Some("[[1]]"),
    )?;
    assert_ne!(code, 0);
    assert!(err.contains("non-negative"), "err={err}");
    Ok(())
}

/// Control: `combinations(n)` uses `n` inside a nested `range(n)`
/// generator (not as a simple scalar), so real jq's own escape semantics
/// there are different from every other case above -- a trailing break in
/// `n`'s own generator aborts the whole array-construction context real
/// jq's `def combinations(n): ...` body uses internally, producing *no*
/// output at all, not "the first n's worth of combinations, then stop".
/// Deliberately left unfixed by #1164; this pins the current (pre-existing,
/// still-divergent-from-jq) behavior so a future attempt doesn't assume
/// the same simple wrap other builtins got would be correct here too.
#[test]
fn test_combinations_n_break_semantics_documented_not_fixed_by_1164() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &[
            "-c",
            r#"label $out | (combinations((2, break $out)), "after")"#,
        ],
        Some("[1,2]"),
    )?;
    assert_eq!(code, 0, "err={err}");
    // Pre-existing divergence from real jq (which produces no output at all
    // here, verified live: `jq -c 'label $out | (combinations((2, break
    // $out)), "after")'` on `[1,2]` prints nothing) -- not attempted by
    // #1164, see this test's own doc comment.
    assert_eq!(out.trim(), "[1,1]\n[1,2]\n[2,1]\n[2,2]\n\"after\"");
    Ok(())
}

// =============================================================================
// #1045: a generator argument that produces zero outputs (e.g. `empty`, or a
// `select`/`if` that filters everything out) now makes the whole builtin
// call produce zero outputs too, instead of erroring "no value" -- matching
// real jq's `x as $b | ...` desugaring, whose `as` binding runs its body
// zero times when `x` produces nothing. Verified live against real jq for
// every builtin below except `tz`/`load` (succinctly-only extensions with no
// jq equivalent), which get the same fix on the strength of the same
// language-level `as`-desugaring argument, not a per-builtin oracle check.
// =============================================================================

#[test]
fn test_has_empty_argument_produces_no_output_not_error_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "{\"a\":1} | has(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_ltrimstr_rtrimstr_empty_argument_produce_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "\"abc\" | ltrimstr(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "\"abc\" | rtrimstr(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_startswith_endswith_empty_argument_produce_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "\"abc\" | startswith(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "\"abc\" | endswith(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_split_join_empty_argument_produce_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "\"a,b\" | split(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "[\"a\",\"b\"] | join(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_contains_inside_empty_argument_produce_no_output_1045() -> Result<()> {
    // The issue's own repro.
    let (out, err, code) = run_jq_full(&["-cn", "[1,2,3] | contains(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "[1,2] | inside(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_nth_empty_argument_produces_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "[1,2,3] | nth(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_flatten_depth_empty_argument_produces_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "[[1,[2]]] | flatten(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_getpath_empty_argument_produces_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "{\"a\":{\"b\":1}} | getpath(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

#[test]
fn test_strftime_strptime_empty_argument_produce_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "0 | strftime(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "\"x\" | strptime(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

/// `tz`/`load` are succinctly-only extensions with no real jq equivalent to
/// check live -- fixed on the strength of the same `x as $b | ...`
/// desugaring argument as every other builtin here, not a per-builtin oracle
/// check (see this block's own header comment).
#[test]
fn test_tz_load_empty_argument_produce_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "0 | tz(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "1 | load(empty)"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

/// A `Some`-shaped (value + trailing control) result must still work exactly
/// as #1164 left it -- #1045's new `Ok(None)` arm must not have disturbed
/// the existing `Ok(Some(...))` path.
#[test]
fn test_has_still_propagates_trailing_break_after_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#"label $out | (has(("a", break $out)), "after")"#],
        Some(r#"{"a":1}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "true");
    Ok(())
}

/// `optional` (`?`) already suppressed a wrong-typed argument to
/// `QueryResult::None`; confirm a zero-output argument composes correctly
/// with `optional` too, rather than the new `Ok(None)` arm accidentally
/// bypassing or double-handling that existing guard.
#[test]
fn test_has_empty_argument_under_optional_produces_no_output_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "\"not an object\" | has(empty)?"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

/// A genuine type-mismatch error from `contains` must still be a real error,
/// not silently swallowed by the new zero-output handling.
#[test]
fn test_contains_type_mismatch_still_errors_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "[1,2] | contains(\"x\")"], None)?;
    assert_ne!(code, 0);
    assert!(
        err.contains("cannot have their containment checked"),
        "err={err}"
    );
    let _ = out;
    Ok(())
}

/// #1045 coverage: `result_to_owned_full`'s widened `Ok(Some(_))` wrapping
/// (vs. the old bare `Ok(_)`) means each of these builtins' pre-existing
/// "wrong-typed argument, but `optional`" guard is now a distinct pattern
/// (`Ok(Some(_)) if optional`) the diff attributes as a new line.
///
/// A *bare* top-level `builtin(bad_arg)?` does NOT exercise this guard: `?`
/// desugars to `try E` (`Expr::Optional(inner) => eval_try(inner, None,
/// value, optional)` in `eval_single`'s dispatch), which evaluates `inner`
/// with the *ambient* (ordinarily `false`) optional and only catches the
/// resulting error afterward -- #693's fix, deliberately not forcing
/// `optional = true` down the whole subtree (that used to let a masked
/// error inside a combinator look like ordinary `empty`). So each builtin's
/// OWN `optional` parameter is `false` here, and the swallow happens in
/// `eval_try`'s catch instead of this guard. This still confirms the
/// end-to-end swallow-to-empty behavior (worth having), it just doesn't
/// reach this specific line.
#[test]
fn test_getpath_strftime_strptime_tz_load_optional_type_mismatch_suppresses_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", "{\"a\":1} | getpath(1)?"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "0 | strftime(1)?"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "\"x\" | strptime(1)?"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "0 | tz(1)?"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    let (out, err, code) = run_jq_full(&["-cn", "1 | load(1)?"], None)?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");
    Ok(())
}

/// Companion to the test above: actually reaches each builtin's own
/// `Ok(Some(_)) if optional` guard directly, via `eval_pipe_with_path_context_internal`'s
/// `Expr::Optional` arm, which -- unlike `eval_single`'s (see the test
/// above) -- forces `optional = true` directly onto the wrapped node
/// instead of going through `eval_try`'s catch-afterward semantics. `key`
/// (a succinctly-only path-tracking extension) is what forces path-context
/// evaluation to engage at all here.
///
/// Each case used to print `null` before `"b"` rather than just `"b"` --
/// `eval_owned_expr_ctrl_full`'s `QueryResult::None => Ok(Null)` collapse
/// (confirmed to predate #1045), fixed by #1280. Now pins the correct
/// empty-not-null output.
#[test]
fn test_getpath_strftime_strptime_tz_load_optional_guard_via_path_context_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", ".a.b | (getpath(1))?, key"],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");

    let (out, err, code) = run_jq_full(
        &["-c", ".a.b | (strftime(1))?, key"],
        Some(r#"{"a":{"b":0}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");

    let (out, err, code) = run_jq_full(
        &["-c", ".a.b | (strptime(1))?, key"],
        Some(r#"{"a":{"b":"x"}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");

    let (out, err, code) = run_jq_full(&["-c", ".a.b | (tz(1))?, key"], Some(r#"{"a":{"b":0}}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");

    let (out, err, code) =
        run_jq_full(&["-c", ".a.b | (load(1))?, key"], Some(r#"{"a":{"b":1}}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");
    Ok(())
}

/// #1280's own issue repro: `has(...)` on a non-object/array primitive under
/// `?` (not an error -- `eval_owned_fast_path`'s type-mismatch branch), the
/// simplest live-reachable trigger for `eval_owned_expr_ctrl_full`'s
/// `QueryResult::None => Ok(Null)` collapse. `key` forces path-context mode.
#[test]
fn test_path_context_builtin_arm_type_mismatch_swallows_to_empty_1280() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#".a.b | (has("x"))?, key"#],
        Some(r#"{"a":{"b":"str"}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");
    Ok(())
}

/// A non-swallowed builtin result alongside `key` still shows its own real
/// value (`false`, not empty) -- confirms #1280's fix only changed the
/// `None`-collapse case, not the ordinary success path. Verified against
/// real jq 1.7.1 (`"b"`, the succinctly-only `key` swapped for a literal, is
/// unaffected by which builtin runs first).
#[test]
fn test_path_context_builtin_arm_non_swallowed_result_unaffected_1280() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#".a.b | has("x"), key"#],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "false\n\"b\"\n");
    Ok(())
}

/// #1280's `Expr::Object`/`Array`/`Literal` arm: an object-construction key
/// generator that produces zero outputs (`{(empty): 1}`) means the whole
/// construction contributes zero outputs, matching real jq
/// (`{(empty): 1}` -> no output at all) -- confirmed live to have printed a
/// spurious `null` before this fix, via `eval_owned_expr::<S>(first, ...)`'s
/// same collapse (a distinct call site from the builtin arm above, since
/// `Expr::Object` never reaches `eval_owned_fast_path`).
#[test]
fn test_path_context_object_arm_empty_key_generator_swallows_to_empty_1280() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-c", ".a | {(empty):1}, key"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"a\"");
    Ok(())
}

/// #1280's generic-fallback `_` arm: `X as $v | BODY` where the source `X`
/// produces zero outputs (`empty as $x | $x`) means the whole binding
/// contributes zero outputs -- confirmed live to have printed a spurious
/// `null` before this fix.
#[test]
fn test_path_context_generic_fallback_empty_source_swallows_to_empty_1280() -> Result<()> {
    let (out, err, code) =
        run_jq_full(&["-c", ".a | (empty as $x | $x), key"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"a\"");
    Ok(())
}

/// #1280's `ParentN` arm: a `parent(...)` call whose `n` argument produces
/// zero outputs (`parent(empty)`) must contribute zero outputs itself, not
/// fall through to the "expected number" type-error arm via a fabricated
/// `Null`. This is the fourth call site the same review round found still
/// routed through the old `eval_owned_expr` after the other three (Builtin,
/// Object/Array/Literal, generic-fallback, covered above) were fixed --
/// before the fix, `parent(empty), key` errored `expected number, got
/// other` instead of printing just `key`'s own value. `parent` is a
/// succinctly extension (no real-jq equivalent), so this is checked against
/// succinctly's own contract, like the sibling test above it.
#[test]
fn test_path_context_parent_n_argument_empty_swallows_to_empty_1280() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", ".a.b | parent(empty), key"],
        Some(r#"{"a":{"b":1}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");
    Ok(())
}

/// `ParentN`'s float-`n` arm (`f as usize`), reached only when the `n`
/// argument evaluates to a float rather than an int -- unchanged logic from
/// before this PR, just newly wrapped in `Ok(Some(_))`; otherwise dead for
/// coverage purposes since #1280 rewrote every arm's own pattern.
#[test]
fn test_path_context_parent_n_argument_float_1280() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", ".a.b.c | parent(1.0), key"],
        Some(r#"{"a":{"b":{"c":1}}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "{\"c\":1}\n\"c\"\n");
    Ok(())
}

/// `ParentN`'s `Ok(Some(_)) if optional` arm: a non-number `n` argument
/// under `?` swallows to zero output, same as any other type mismatch.
#[test]
fn test_path_context_parent_n_argument_wrong_type_optional_1280() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", r#".a.b | (parent("x"))?, key"#],
        Some(r#"{"a":{"b":1}}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out.trim(), "\"b\"");
    Ok(())
}

/// `ParentN`'s `Ok(Some(_))` unconditional arm: a non-number `n` argument
/// without `?` is a hard type error, matching every other numeric-argument
/// builtin's own unconditional type-check.
#[test]
fn test_path_context_parent_n_argument_wrong_type_errors_1280() -> Result<()> {
    let (_out, err, code) =
        run_jq_full(&["-c", r#".a.b | parent("x")"#], Some(r#"{"a":{"b":1}}"#))?;
    assert_ne!(code, 0);
    assert!(err.contains("expected number"), "err={err}");
    Ok(())
}

/// #1313: `resolve_limit`'s own `n`-bound evaluation used `eval_owned_expr_ctrl`,
/// which by design collapses a genuinely zero-output bound to `Null` --
/// falling into the `Null`/`Bool` "unlimited passthrough" branch instead of
/// jq's own `n as $n | ...` desugaring, where `n` producing zero outputs
/// makes the whole call produce zero output. Both `path(...)` mode (this
/// function) and plain value mode (`eval_limit`, the sibling test below)
/// had the identical bug via two different collapsing helpers.
#[test]
fn test_limit_path_mode_zero_output_bound_produces_no_output_1313() -> Result<()> {
    let (out, _err, code) = run_jq_full(
        &["-c", "path(limit(empty; .a,.b))"],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 0);
    assert_eq!(out, "");

    // Sanity: a normal bound is unaffected.
    let (out, _err, code) =
        run_jq_full(&["-c", "path(limit(1; .a,.b))"], Some(r#"{"a":1,"b":2}"#))?;
    assert_eq!(code, 0);
    assert_eq!(out, "[\"a\"]\n");

    Ok(())
}

/// #1313 (code review): `resolve_limit`'s n-classification was missing the
/// negative-float "unlimited passthrough" arm `eval_limit` already had
/// (#983) -- `path(limit(-1.5; ...))` raised "limit requires non-negative
/// integer" instead of agreeing with plain-value-mode `limit(-1.5; ...)`,
/// which already correctly passed everything through. A pre-existing
/// divergence between the two functions, unrelated to and not introduced by
/// this issue's own zero-output-bound fix, but caught while both functions
/// were already being edited for it.
#[test]
fn test_limit_path_mode_negative_float_bound_is_unlimited_passthrough_1313() -> Result<()> {
    let (out, err, code) = run_jq_full(
        &["-c", "path(limit(-1.5; .a,.b))"],
        Some(r#"{"a":1,"b":2}"#),
    )?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "[\"a\"]\n[\"b\"]\n");

    Ok(())
}

/// #1313: the plain value-mode sibling of the fix above -- `eval_limit`
/// used `result_to_owned`, which collapses a zero-output bound into
/// `result_to_owned_ctrl`'s own `Err("no value")` instead of the zero-output
/// case `result_to_owned_full` (#1045) already exists to distinguish (and
/// every other #1045-migrated caller, e.g. `builtin_ltrimstr`, already
/// uses).
#[test]
fn test_limit_value_mode_zero_output_bound_produces_no_output_1313() -> Result<()> {
    let (out, _err, code) = run_jq_full(&["-c", "limit(empty; .a,.b)"], Some(r#"{"a":1,"b":2}"#))?;
    assert_eq!(code, 0);
    assert_eq!(out, "");

    // Sanity: negative/null/bool bounds still take the pre-existing
    // "unlimited passthrough" branch (#983), unaffected by this fix.
    let (out, _err, code) = run_jq_full(&["-c", "limit(-1; .a,.b)"], Some(r#"{"a":1,"b":2}"#))?;
    assert_eq!(code, 0);
    assert_eq!(out, "1\n2\n");

    Ok(())
}

/// #1313: `eval_rhs_once` (backing `+=`/`-=`/`*=`/`/=`/`%=`/`//=`) collapsed
/// a genuinely zero-output RHS to `Null` and spliced it into the update
/// filter (`. op null`), instead of real jq's `value as $value | ...`
/// desugaring, where `value` producing zero outputs makes the whole
/// assignment produce zero output. Covers every compound operator, plus
/// `//=` (whose RHS is unconditional -- it's evaluated regardless of
/// whether the current value is already truthy, since `//=`'s own
/// short-circuiting is about which *filter result* wins per resolved path,
/// not whether the RHS expression itself runs).
#[test]
fn test_compound_assign_empty_rhs_produces_no_output_1313() -> Result<()> {
    for op in ["+=", "-=", "*=", "/=", "%="] {
        let (out, err, code) = run_jq_full(&["-c", &format!(".a {op} empty")], Some(r#"{"a":2}"#))?;
        assert_eq!(code, 0, "op={op} err={err}");
        assert_eq!(out, "", "op={op}");
    }

    let (out, err, code) = run_jq_full(&["-c", ".a //= empty"], Some(r#"{"a":null}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    // Sanity: a normal RHS is unaffected.
    let (out, _err, code) = run_jq_full(&["-c", ".a += 5"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"a":6}"#);

    Ok(())
}

/// #1313 (code review): an `optional`-swallowed RHS error is a distinct
/// shape from a genuinely zero-output generator (`empty`, covered above),
/// but reaches `eval_rhs_once` the same way -- `.a += (1/0)?` swallows the
/// division error to zero output at `eval_single`'s own `?` handling, then
/// `eval_rhs_once` sees `QueryResult::None` exactly as it would for `empty`,
/// and the whole assignment correctly produces zero output rather than
/// splicing `null` in and evaluating `. + null` (which used to be a type
/// error, not a silent no-op -- this is a case where the old bug produced a
/// hard error instead of a wrong value, but still diverged from jq).
#[test]
fn test_compound_assign_optional_swallowed_rhs_error_produces_no_output_1313() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-c", ".a += (1/0)?"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 0, "err={err}");
    assert_eq!(out, "");

    // Without `?`, the same division-by-zero still hard-errors.
    let (_out, err, code) = run_jq_full(&["-c", ".a += (1/0)"], Some(r#"{"a":1}"#))?;
    assert_eq!(code, 5);
    assert!(err.contains("divided"), "err={err}");

    Ok(())
}

/// #1045 coverage: `flatten(depth)`'s `Ok(Some(_)) => ...type_error("number",
/// "non-number")` arm -- a non-number depth argument, unconditional (no
/// `optional` gate), unlike the negative-depth arm covered by
/// `test_flatten_negative_depth_errors_even_with_trailing_break_1164` above.
#[test]
fn test_flatten_depth_non_number_argument_errors_1045() -> Result<()> {
    let (out, err, code) = run_jq_full(&["-cn", r#"[[1]] | flatten("x")"#], None)?;
    assert_ne!(code, 0);
    assert!(err.contains("non-number"), "err={err}");
    let _ = out;
    Ok(())
}

/// Pins the seven **non-`Pipe`** path-expression shapes that already match jq
/// 1.7.1 today, so that #986/#989's deferred-trackability rework cannot break
/// them without a test failing first.
///
/// These exist because of a specific, concretely-identified hazard found while
/// reviewing that rework's design doc
/// (`docs/plan/jq-path-trackability-deferral.md`, "Where the terminal check
/// belongs"). An earlier draft placed the deferred trackability check inside
/// `resolve_seq`. Every shape below is non-`Pipe`, so `resolve_seq` never runs
/// for any of them — the check would simply never fire, the `trackable: false`
/// branch would flow into `resolve_dynamic_indexes`' `assemble()`, and that
/// helper maps a zero-component branch to `Expr::Identity`. `path(1)` would
/// silently become `[]`, and **`del(1)` would become `del(.)` -> `null`,
/// destroying the whole document with no error at all** — the same write-path
/// corruption class that got PR #985 reverted on #972.
///
/// The design's own verification sweep could not have caught this: it
/// generated only `E1 | E2` and `(E1, E2) | E3` shapes, so every case it
/// produced had a pipe in it and a bare `del(1)` never appeared. Hence pinning
/// the non-pipe set explicitly, ahead of the implementation rather than after.
///
/// All seven were captured live against pinned jq 1.7.1: each writes nothing
/// to stdout, exits 5, and reports `Invalid path expression with result <v>`
/// naming the offending value.
#[test]
fn test_non_pipe_path_expressions_still_raise_986() -> Result<()> {
    // (query, the value jq names in "with result <v>")
    let cases = [
        ("path(1)", "1"),
        (r#"path("x")"#, r#""x""#),
        ("path([1])", "[1]"),
        // Folds to `2` before the resolver sees it -- jq names the folded
        // value, not the source text.
        ("path(1+1)", "2"),
        ("del(1)", "1"),
        // Multi-output: jq names the *first* output, `0`, not the last.
        ("del(range(2))", "0"),
        ("(1) = 9", "1"),
    ];

    for (query, named_value) in cases {
        let (stdout, stderr, code) = run_jq_full(&["-c", query], Some(r#"{"a":1}"#))?;
        assert_eq!(
            code, 5,
            "`{query}` should exit 5, got {code}\nstdout: {stdout}\nstderr: {stderr}"
        );
        assert!(
            stdout.is_empty(),
            "`{query}` should write nothing to stdout, got: {stdout}"
        );
        // The whole point: it must still *raise*, naming the offending value.
        // A regression here shows up as an empty stdout with exit 0 (for
        // `path`), or as a silently rewritten document (for `del`/`=`).
        assert!(
            stderr.contains(&format!(
                "Invalid path expression with result {named_value}"
            )),
            "`{query}` should name `{named_value}`, got: {stderr}"
        );
    }
    Ok(())
}

/// Pins the shapes #986/#989's deferred-trackability rework fixed, so a later
/// change to `resolve_leaf`/`resolve_seq`/`Expr::Comma` cannot quietly undo
/// them.
///
/// Companion to `test_non_pipe_path_expressions_still_raise_986`, which pins
/// the shapes that had to *keep* working. This pins the ones that had to
/// start. All expectations captured live from pinned jq 1.7.1.
///
/// The grouping matters: Stage 1 moved the trackability decision to the
/// genuinely terminal position, which is what lets a non-path-shaped value
/// reach `resolve_index_expr`'s pre-existing #843 checks and pick up jq's
/// "near attempt to access element K of V" wording. Stage 2 then stopped
/// `Expr::Comma` from answering the same question a position too early.
#[test]
fn test_deferred_trackability_matches_jq_986_989() {
    // (query, expected stderr fragment) -- all exit 5, no stdout.
    let raising = [
        // Stage 1: the error names the *navigation* that failed, not the
        // value that merely wasn't a path.
        (
            r"path(1|.foo)",
            r#"near attempt to access element "foo" of 1"#,
        ),
        (
            r#"(1 | .[("x","y")]) = 9"#,
            r#"near attempt to access element "x" of 1"#,
        ),
        (
            r#"(range(3) | .[("x","y")]) = 9"#,
            r#"near attempt to access element "x" of 0"#,
        ),
        (
            r#"(1 | .a)[("x","y")] = 9"#,
            r#"near attempt to access element "a" of 1"#,
        ),
        (
            r#"path((1|.)[("x","y")])"#,
            r#"near attempt to access element "x" of 1"#,
        ),
        // A pipe's *last* output is the one named, not its first.
        (r"path(1|2)", "Invalid path expression with result 2"),
        // The downstream error is reported, not masked by #530's wording.
        (r#"path(1|error("boom"))"#, "boom"),
    ];
    for (query, fragment) in raising {
        let (stdout, stderr, code) = run_jq_full(&["-c", query], Some(r#"{"a":1,"foo":10}"#))
            .unwrap_or_else(|e| panic!("`{query}` failed to run: {e}"));
        assert_eq!(code, 5, "`{query}`\nstdout: {stdout}\nstderr: {stderr}");
        assert!(
            stderr.contains(fragment),
            "`{query}` should mention `{fragment}`, got: {stderr}"
        );
    }

    // Stage 2: a `Comma` mid-pipe is not the terminal position. The `.a`
    // branch resolves and is emitted first; only then does `.c` raise
    // against the literal `1` -- naming `.c`, not the literal.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", r"path((.a, 1)|.c)"], Some(r#"{"a":{"c":1},"x":2}"#))
            .expect("comma-mid-pipe repro runs");
    assert_eq!(code, 5, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "[\"a\",\"c\"]\n", "stderr: {stderr}");
    assert!(
        stderr.contains(r#"near attempt to access element "c" of 1"#),
        "{stderr}"
    );

    // `halt_error` must still halt rather than being downgraded into a
    // catchable "Invalid path expression" -- the one repro here that is a
    // genuine correctness violation rather than a wording fix. Exit code is
    // part of the contract.
    let (stdout, _stderr, code) =
        run_jq_full(&["-c", r"path(1|halt_error(3))"], Some("{}")).expect("halt_error repro runs");
    assert_eq!(code, 3, "halt_error must halt with its own code");
    assert!(stdout.is_empty(), "stdout: {stdout}");

    // `break` must still unwind to its label rather than raising.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r"label $out | path(1|break $out)"],
        Some(r#"{"a":10}"#),
    )
    .expect("break repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
}

// #723: input/inputs/input_line_number. Every expected value below was
// verified live against pinned jq 1.7.1 during implementation.

/// `., input` on 3 documents: doc 1 is `.`'s own current input, `input`
/// reads doc 2 (both output); the outer loop's *next* iteration is then doc
/// 3 (already in sync with what `input` consumed), whose own `input` call
/// finds nothing left and errors -- so this exits 5 despite all three
/// values reaching stdout. Confirmed this exact shape live against jq
/// 1.7.1: stdout `1`/`2`/`3` plus a `break` error on stderr, not a clean
/// exit -- jq's own `input`/outer-loop interaction has the identical
/// "last iteration's own `input` call exhausts" behavior this mirrors.
#[test]
fn test_jq_input_reads_next_document_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "., input"], Some("1 2 3")).expect("input repro runs");
    assert_eq!(code, 5, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\n2\n3\n");
    assert!(stderr.contains("break"), "{stderr}");
    Ok(())
}

/// jq's own exhaustion error is oddly spelled `break`, not a more
/// descriptive "No more inputs" -- confirmed live against jq 1.7.1. The
/// `:0` (not `:1`) is a separate, pre-existing quirk unrelated to #723:
/// `succinctly jq`'s own location tracking reports line 0 for a document
/// ending on the input's first line even without `input` involved at all
/// (confirmed identical on `main` before this change, e.g. `echo '1 2' |
/// jq '.,error'` reports `:0` for the first value too) -- not something
/// this issue introduces or should silently paper over here.
#[test]
fn test_jq_input_exhausted_errors_with_break_723() {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "input"], Some("1")).expect("input-exhaustion repro runs");
    assert_eq!(code, 5, "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.is_empty(), "stdout: {stdout}");
    assert!(
        stderr.contains("jq: error (at <stdin>:0): break"),
        "{stderr}"
    );
}

#[test]
fn test_jq_input_optional_catches_exhaustion_silently_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "input?"], Some("1")).expect("input? repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_jq_try_input_catch_catches_exhaustion_723() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", r#"try input catch "caught""#], Some("1"))
        .expect("try/catch input repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "\"caught\"\n");
    Ok(())
}

/// jq's own canonical `-n` streaming-aggregation idiom -- the primary
/// real-world use case for `inputs`.
#[test]
fn test_jq_null_input_reduce_over_inputs_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-cn", "reduce inputs as $x (0; .+$x)"], Some("1 2 3"))
            .expect("-n reduce inputs repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "6\n");
    Ok(())
}

/// Unlike bare `input`, `inputs` never errors on exhaustion -- it's a
/// generator that just stops.
#[test]
fn test_jq_inputs_stream_remaining_without_error_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "inputs"], Some("1 2 3")).expect("inputs repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    // The bare top-level loop already consumed document 1 as `.`'s own
    // input before `inputs` ever ran, so only 2 and 3 remain.
    assert_eq!(stdout, "2\n3\n");
    Ok(())
}

#[test]
fn test_jq_null_input_inputs_sees_every_document_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-cn", "inputs"], Some("1 2 3")).expect("-n inputs repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\n2\n3\n");
    Ok(())
}

#[test]
fn test_jq_input_line_number_tracks_reads_723() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", "., input_line_number"], Some("1\n2\n3\n"))
        .expect("input_line_number repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\n1\n2\n2\n3\n3\n");
    Ok(())
}

#[test]
fn test_jq_input_line_number_zero_before_any_read_723() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-cn", "input_line_number"], Some("1"))
        .expect("-n input_line_number repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "0\n");
    Ok(())
}

/// A filter's own `input` call and the outer per-document loop share one
/// queue (#723): a document `input` consumes mid-evaluation must never also
/// be re-processed by the loop as a fresh top-level invocation.
#[test]
fn test_jq_input_and_outer_loop_share_one_queue_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "(., input)"], Some("1 2 3 4")).expect("shared-queue repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    // Doc 1 -> `.` = 1, `input` reads doc 2. Doc 3 -> `.` = 3, `input`
    // reads doc 4. All four documents seen exactly once, in order.
    assert_eq!(stdout, "1\n2\n3\n4\n");
    Ok(())
}

#[test]
fn test_jq_null_input_reduce_inputs_over_multiple_files_723() -> Result<()> {
    let mut f1 = NamedTempFile::new()?;
    write!(f1, "1")?;
    let mut f2 = NamedTempFile::new()?;
    write!(f2, "2")?;
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-cn",
            "reduce inputs as $x (0; .+$x)",
            f1.path().to_str().unwrap(),
            f2.path().to_str().unwrap(),
        ],
        None,
    )
    .expect("multi-file inputs repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "3\n");
    Ok(())
}

#[test]
fn test_jq_halt_after_input_still_halts_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "(input, halt)"], Some("1 2 3")).expect("input+halt repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    // `input` reads doc 2 and outputs it; `halt` then exits immediately,
    // before the outer loop would otherwise move on to doc 3.
    assert_eq!(stdout, "2\n");
    Ok(())
}

/// `-n` without any of these builtins must keep working exactly as before
/// (#723's own `uses_input_builtins` gate must not force a real read, or
/// route through a different code path, for a filter that never
/// mentions them).
#[test]
fn test_jq_null_input_unaffected_when_not_using_input_builtins_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-cn", "1 + 1"], Some("")).expect("-n unaffected repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "2\n");
    Ok(())
}

/// `halt` reached inside `-n`'s own single-invocation branch (as opposed to
/// the non-`-n` per-document loop `test_jq_halt_after_input_still_halts_723`
/// already covers) must still exit cleanly.
#[test]
fn test_jq_null_input_halt_after_inputs_723() -> Result<()> {
    let (stdout, stderr, code) =
        run_jq_full(&["-cn", "(inputs, halt)"], Some("1 2 3")).expect("-n halt repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\n2\n3\n");
    Ok(())
}

/// `input` called from inside a user-defined function: forces
/// `Builtin::Input`/`Inputs`/`InputLineNumber` through the AST-rewriting
/// passes (`expand_func_calls_in_builtin`/`substitute_func_param_in_builtin`)
/// that inline a function's own body at its call site -- exercising their
/// mechanical pass-through arms for these three new builtins, not just the
/// dispatch arm a bare top-level call already covers.
#[test]
fn test_jq_input_inside_user_defined_function_723() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(&["-c", ".,(def f: input; f)"], Some("1 2 3"))
        .expect("input-inside-function repro runs");
    assert_eq!(code, 5, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\n2\n3\n");
    assert!(stderr.contains("break"), "{stderr}");
    Ok(())
}

/// `inputs`/`input_line_number` (the other two of #723's three new
/// builtins) called from inside a user-defined function -- same
/// AST-rewriting-pass coverage reasoning as the `input`-specific test
/// above, for the two builtins it didn't happen to exercise.
#[test]
fn test_jq_inputs_and_input_line_number_inside_user_defined_function_723() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", ".,(def f: input_line_number; f)"],
        Some("1\n2\n3\n"),
    )
    .expect("input_line_number-inside-function repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\n1\n2\n2\n3\n3\n");

    // `substitute_func_param_in_builtin`'s own Inputs/InputLineNumber arms
    // (as opposed to `expand_func_calls_in_builtin`'s, exercised above) only
    // run when a filter-style (non-`$`) function *parameter* is substituted
    // through a body that itself contains these builtins -- unlike passing
    // `inputs` as an *argument*, which just swaps the whole argument
    // expression in wholesale without walking its own contents.
    //
    // Input ends with a trailing newline deliberately: without one, the
    // last document's own reported line number is one lower than with it, a
    // separate pre-existing `LineCounter`/`extend_from_ends` quirk
    // unrelated to #723 (confirmed reproducing identically for `input`'s
    // own document-location tracking, not just `input_line_number`) --
    // sidestepped here rather than chased, since this test's only job is
    // patch-coverage for the AST-rewriting pass, not pinning that quirk.
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "(def f(x): x, input, inputs, input_line_number; f(1))",
        ],
        Some("1 2 3\n"),
    )
    .expect("function-parameter-substitution repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(stdout, "1\n2\n3\n1\n");
    Ok(())
}

/// #1088: `path()` reports a numeric component as the key *was*, not as the
/// index it resolves to.
///
/// jq appends the resolved key verbatim, so a key still carrying a float
/// spelling keeps it — succinctly used to route every numeric key through
/// `numeric_key_to_index` and emit a bare integer. Every expectation here is
/// pinned jq 1.7.1's own output.
///
/// The `path(.[-1.0])` → `[-1]` case is not an index-specific asymmetry, and
/// is here to keep anyone from "fixing" it into `[-1.0]`: jq's unary minus
/// destroys number-literal preservation (`jq -n '-1.0'` is already `-1`), so
/// the negated key is a plain double before indexing happens at all. The
/// `from-data` case is the control that proves it — a negative float that
/// never passes through unary minus *does* keep its spelling.
#[test]
fn test_1088_path_reports_float_index_as_written() {
    let arr = Some("[1,2,3,4,5]");
    for (query, expected) in [
        // Literal keys keep their own spelling.
        ("path(.[2.0])", "[2.0]\n"),
        ("path(.[2.00])", "[2.00]\n"),
        ("path(.[1.7])", "[1.7]\n"),
        ("path(.[0.0])", "[0.0]\n"),
        ("path(.[1e10])", "[1E+10]\n"),
        // An integer-spelled key is unchanged — nothing to preserve.
        ("path(.[2])", "[2]\n"),
        ("path(.[-1])", "[-1]\n"),
        // Negated literals: jq's own unary minus already collapsed them.
        ("path(.[-1.0])", "[-1]\n"),
        ("path(.[-2.5])", "[-2.5]\n"),
        ("path(.[-1e10])", "[-10000000000]\n"),
        ("path(.[-0.0])", "[-0]\n"),
        // A dynamic key resolves through `key_to_path_component`, not the
        // parser's constant fold — both had to be fixed.
        ("(2.0) as $x | path(.[$x])", "[2.0]\n"),
        ("(-2.5) as $x | path(.[$x])", "[-2.5]\n"),
        // Arithmetic produces a *new* number, which drops the spelling in
        // succinctly and jq alike.
        ("(1.0+1.0) as $x | path(.[$x])", "[2]\n"),
        // Beyond `i64`, where the old code saturated the component to
        // `i64::MAX` and reported a value that was never written.
        ("path(.[9223372036854775808])", "[9223372036854775808]\n"),
        ("path(.[infinite])", "[1.7976931348623157e+308]\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", query], arr)
            .unwrap_or_else(|e| panic!("`{query}` failed to run: {e}"));
        assert_eq!(code, 0, "`{query}`\nstdout: {stdout}\nstderr: {stderr}");
        assert_eq!(stdout, expected, "`{query}`\nstderr: {stderr}");
    }

    // The control for the negation rule above: from data, no unary minus is
    // involved, and jq keeps every spelling — including the negative ones.
    let (stdout, stderr, code) = run_jq_full(
        &["-c", "[.i[] as $x | path(.a[$x])]"],
        Some(r#"{"i":[2.0,2.50,-1.0,-1.00,-1e10],"a":[1,2,3,4,5]}"#),
    )
    .expect("from-data repro runs");
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert_eq!(
        stdout, "[[\"a\",2.0],[\"a\",2.50],[\"a\",-1.0],[\"a\",-1.00],[\"a\",-1E+10]]\n",
        "stderr: {stderr}"
    );

    // Reading, writing and deleting through a float key are all unchanged —
    // only the *reported* component moved. A component fed back into
    // `setpath`/`getpath`/`delpaths` still lands on the truncated index.
    for (query, expected) in [
        (".[2.5]", "3\n"),
        (".[-1.5]", "5\n"),
        (".[2.5] = 99", "[1,2,99,4,5]\n"),
        (".[1.5] |= .+100", "[1,102,3,4,5]\n"),
        ("del(.[1.5])", "[1,3,4,5]\n"),
        ("setpath(path(.[2.5]); 99)", "[1,2,99,4,5]\n"),
        ("getpath(path(.[2.0]))", "3\n"),
        ("delpaths([path(.[-1.0])])", "[1,2,3,4]\n"),
        // Slices are deliberately untouched by #1088 and must not drift.
        ("path(.[1:3])", "[{\"start\":1,\"end\":3}]\n"),
        (".[1.5:3.5]", "[2,3,4]\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", query], arr)
            .unwrap_or_else(|e| panic!("`{query}` failed to run: {e}"));
        assert_eq!(code, 0, "`{query}`\nstdout: {stdout}\nstderr: {stderr}");
        assert_eq!(stdout, expected, "`{query}`\nstderr: {stderr}");
    }

    // A float index inside a user-defined function body goes through
    // `expand_func_calls`/`substitute_func_param`, which rebuild the AST
    // node by node — a variant they forgot would be dropped there rather
    // than at the point of use.
    for (query, expected) in [
        ("def f: path(.[2.0]); f", "[2.0]\n"),
        ("def g($n): path(.[2.0]); g(1)", "[2.0]\n"),
        ("def h($n): path(.[$n]); h(2.0)", "[2.0]\n"),
        ("def k: .[2.0]; k", "3\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["-c", query], arr)
            .unwrap_or_else(|e| panic!("`{query}` failed to run: {e}"));
        assert_eq!(code, 0, "`{query}`\nstdout: {stdout}\nstderr: {stderr}");
        assert_eq!(stdout, expected, "`{query}`\nstderr: {stderr}");
    }

    // The float component also reaches the `Invalid path expression` message,
    // which names the element that failed to navigate.
    let (stdout, stderr, code) =
        run_jq_full(&["-c", "path(reverse | .[1.5])"], arr).expect("untracked repro runs");
    assert_eq!(code, 5, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stderr.contains("near attempt to access element 1.5 of"),
        "{stderr}"
    );
}

/// #1090: `tonumber` preserves the string's own spelling instead of
/// renormalizing it through `f64`. Every expectation below was read off
/// real jq 1.7.1 -- succinctly previously collapsed `"2.50"` to `2.5` and
/// `"1e3"` to `1000`.
///
/// This lives in the jq suite as well as the yq one because
/// `tonumber_from_str` is shared by both evaluators, and the two apply
/// *different* final formatters to the preserved literal: jq renormalizes
/// through `format_number_jq_compat` (hence `1E+3`, not `1e3`), while yq
/// echoes it verbatim. A change that satisfies only one oracle would look
/// correct in half the test suite.
#[test]
fn test_tonumber_preserves_source_spelling_1090() {
    for (input, expected) in [
        (r#""2.0""#, "2.0\n"),
        (r#""2.50""#, "2.50\n"),
        (r#""1e3""#, "1E+3\n"),
        (r#""1E5""#, "1E+5\n"),
        (r#""2e0""#, "2\n"),
        (r#""42""#, "42\n"),
        // Spellings JSON rejects fall back to a plain number, matching jq.
        (r#""007""#, "7\n"),
        (r#"".5""#, "0.5\n"),
        // A leading `+` has an exact JSON-safe equivalent, so the literal
        // still survives -- a bare `Float` here would print `2`, not `2.0`.
        (r#""+2.0""#, "2.0\n"),
    ] {
        let (stdout, stderr, code) = run_jq_full(&["tonumber"], Some(input))
            .unwrap_or_else(|e| panic!("`{input} | tonumber` failed to run: {e}"));
        assert_eq!(code, 0, "`{input}`\nstdout: {stdout}\nstderr: {stderr}");
        assert_eq!(stdout, expected, "`{input}`\nstderr: {stderr}");
    }
}

/// #1090 follow-on: the leading-`+` retry must not turn a *doubled* sign
/// into an accepted number. `is_valid_number` accepts a leading `-` of its
/// own, so stripping the `+` off `"+-1"` leaves a perfectly valid `-1` --
/// which real jq 1.7.1 rejects outright, and which this crate rejected
/// before the retry existed. Error wording confirmed against jq 1.7.1.
#[test]
fn test_tonumber_rejects_doubled_sign_1090() {
    for input in [
        r#""+-1""#,
        r#""+-1.5""#,
        r#""+-0""#,
        r#""+-1e3""#,
        r#""-+1""#,
    ] {
        let (stdout, stderr, code) = run_jq_full(&["tonumber"], Some(input))
            .unwrap_or_else(|e| panic!("`{input} | tonumber` failed to run: {e}"));
        assert_ne!(code, 0, "`{input}` should error\nstdout: {stdout}");
        assert!(
            stderr.contains("Invalid numeric literal"),
            "`{input}`\nstderr: {stderr}"
        );
    }
}

/// #1090 follow-on: preserving `tonumber`'s literal must not start
/// accepting text real jq rejects. The internal overflow sentinels
/// (`9e999e999` -> NaN, `8e999e999` -> Infinity) are ordinary user input
/// here, and routing this builtin through `OwnedValue::from_number_bytes`
/// -- which decodes them -- would silently turn jq's documented error into
/// a NaN. Error wording confirmed against jq 1.7.1.
#[test]
fn test_tonumber_rejects_internal_overflow_sentinels_1090() {
    for input in [r#""9e999e999""#, r#""8e999e999""#, r#""-8e999e999""#] {
        let (stdout, stderr, code) = run_jq_full(&["tonumber"], Some(input))
            .unwrap_or_else(|e| panic!("`{input} | tonumber` failed to run: {e}"));
        assert_ne!(code, 0, "`{input}` should error\nstdout: {stdout}");
        assert!(
            stderr.contains("Invalid numeric literal"),
            "`{input}`\nstderr: {stderr}"
        );
    }
}

// Short-circuiting generator consumers and evaluation-time side effects
// (#820, #932, #987; Stage 1 of `docs/plan/jq-lazy-generator-consumers.md`).
//
// Every expectation below was captured from the pinned oracle `/usr/bin/jq`
// (jq-1.7.1-apple) and this crate's own `--release --features cli` binary at
// `bd73d2436`, with stdout and stderr redirected *separately* -- `2>&1`
// interleaves the two misleadingly, since stdout is buffered when piped but
// stderr is not (same convention as the `halt`/`stderr` block above).
//
// `stderr` and `halt_error` are the only two builtins whose Rust
// implementation performs I/O *at evaluation time* (`write_stderr`, via
// `builtin_stderr`/`builtin_halt_error` in `src/jq/eval.rs`); `debug` and
// `debug(msg)` are deliberate library-context no-ops, and `error(...)` carries
// its payload in the result. So `stderr` is the only usable probe for "was
// this sub-expression evaluated at all?" -- which is why #820's own original
// `first(1, debug)` repro was a false negative.
//
// `input`/`inputs` are a second, *destructive* probe: they pop from the same
// process-global queue the CLI's own per-document driver loop drains, so an
// eagerly-evaluated discarded branch containing one silently eats a document.

/// One CLI expectation: (args, stdin, stdout, stderr, exit code).
///
/// Named rather than inlined because the tuple trips clippy's
/// `type_complexity` lint at the two `&[...]` tables below.
type SideEffectCase = (
    &'static [&'static str],
    Option<&'static str>,
    &'static str,
    &'static str,
    i32,
);

/// The shapes that already match jq, pinned *before* any laziness work.
///
/// This is the high-value half of Stage 1. The `eval_each` design's
/// characteristic failure mode is stopping **too early** and suppressing a
/// side effect real jq genuinely performs -- so this test, not the
/// divergence test below, is what a too-eager `Demand::Stop` would break.
/// Pinning it first follows #1284's precedent (guard rails before the change,
/// not after).
#[test]
fn test_short_circuit_side_effect_shapes_already_match_jq_820() -> Result<()> {
    let cases: &[SideEffectCase] = &[
        // `limit(n)` pulls exactly `n` values, so a side effect sitting at
        // position `n` IS reached. Stopping at the first would suppress it.
        (
            &["-cn", "limit(2; 1, stderr, 3)"],
            None,
            "1\nnull\n",
            "null",
            0,
        ),
        // `empty` contributes no output, so `isempty` is not yet satisfied
        // when it reaches `stderr` -- the write must still happen.
        (
            &["-cn", "isempty(empty, stderr)"],
            None,
            "false\n",
            "null",
            0,
        ),
        (
            &["-cn", r#"first(empty, ("B"|stderr))"#],
            None,
            "\"B\"\n",
            "B",
            0,
        ),
        // One output still means the producer ran to produce it.
        (
            &["-cn", r#"isempty(("B"|stderr))"#],
            None,
            "false\n",
            "B",
            0,
        ),
        // The side effect is in the *first* branch, which is always needed.
        (&["-cn", "first(stderr, 1)"], None, "null\n", "null", 0),
        // `n == 0` must not evaluate the operand at all.
        (&["-cn", r#"[limit(0; ("B"|stderr))]"#], None, "[]\n", "", 0),
        // Array construction is atomic in jq; laziness must not leak into it.
        (
            &["-cn", r#"isempty([1, ("B"|stderr)])"#],
            None,
            "false\n",
            "B",
            0,
        ),
        // `nth(2)` needs index 2, which IS the side-effecting branch.
        (
            &["-cn", r#"nth(2; 1,2,("B"|stderr),4)"#],
            None,
            "\"B\"\n",
            "B",
            0,
        ),
        // `all` short-circuits on a FALSY element, so a truthy element 1
        // means element 2 is still reached. #932's text guesses `all` leaks
        // like `any`; it does not -- real jq writes `5` here too. Only the
        // `any` direction of `any_all_gen_cond` may ever stop early.
        (
            &["-c", "all(2, (5|stderr); .==2)"],
            Some("2"),
            "false\n",
            "5",
            0,
        ),
        // No match, so the generator must be exhausted.
        (
            &["-c", "any(9, (5|stderr); .==2)"],
            Some("2"),
            "false\n",
            "5",
            0,
        ),
        (&["-c", "IN(2, (5|stderr))"], Some("[2]"), "false\n", "5", 0),
        // Already lazy today, and must stay lazy.
        (
            &["-cn", r#"false and ("B"|stderr)"#],
            None,
            "false\n",
            "",
            0,
        ),
        (
            &["-cn", r#"if false then ("B"|stderr) else 1 end"#],
            None,
            "1\n",
            "",
            0,
        ),
        // A *bare* escape (zero prior output) must still propagate rather
        // than be answered by the consumer (#882, #791, #867).
        (&["-cn", r#"isempty("m"|halt_error(3))"#], None, "", "m", 3),
        (&["-cn", "label $o|isempty(break $o)"], None, "", "", 0),
        // ...but an escape AFTER an output must not, once `isempty` is
        // satisfied: real jq never asks the generator for that second value.
        (
            &["-cn", "label $o | [isempty(1, break $o)]"],
            None,
            "[false]\n",
            "",
            0,
        ),
        // Under-satisfied consumers must still surface the trailing control.
        (
            &["-cn", "last(1,2,error(\"x\"))"],
            None,
            "",
            "jq: error (at <unknown>): x",
            5,
        ),
        (
            &["-cn", "nth(5; 1,2,error(\"x\"))"],
            None,
            "",
            "jq: error (at <unknown>): x",
            5,
        ),
        (
            &["-cn", "[limit(3;1,2,error(\"x\"),4)]"],
            None,
            "",
            "jq: error (at <unknown>): x",
            5,
        ),
        // Legitimate `input` use must stay unaffected by any laziness work:
        // each run consumes exactly one extra document, pairing them up.
        (
            &["-c", "[., input] | map(.id)"],
            Some(r#"{"id":1} {"id":2} {"id":3} {"id":4}"#),
            "[1,2]\n[3,4]\n",
            "",
            0,
        ),
    ];

    for (args, stdin, want_out, want_err, want_code) in cases {
        let (stdout, stderr, code) = run_jq_full(args, *stdin)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end_matches('\n'), code),
            (*want_out, *want_err, *want_code),
            "`{}` diverged from pinned jq 1.7.1",
            args.join(" ")
        );
    }
    Ok(())
}

/// jq's generator-argument backtracking genuinely RESUMES after the body
/// runs, so an ordinary builtin's argument must never get a stopping sink.
///
/// `result_to_owned_ctrl`'s own doc comment (`src/jq/eval.rs`) records the
/// desugaring -- `f(x)` is roughly `x as $b | body` -- and #833's
/// `ltrimstr(("a", break $out))` repro. This test pins the *side-effect*
/// half: real jq evaluates the second argument output, so `B` reaches stderr.
/// Exactly one of `result_to_owned`'s 20 call sites (`builtin_halt_error`)
/// may ever stop, and only because its body terminates the process.
///
/// The stdout halves deliberately differ: jq emits one result per argument
/// output (`"bcabc"` then `"abcabc"`), succinctly only the first. That gap is
/// #1279 -- the OPPOSITE bug to #820, and pinned here so laziness work does
/// not silently deepen it.
#[test]
fn test_generator_argument_backtracking_still_evaluates_the_tail_1279() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &["-c", r#"ltrimstr(("a", ("B"|stderr)))"#],
        Some(r#""abcabc""#),
    )?;
    assert_eq!(code, 0, "stdout: {stdout:?} stderr: {stderr:?}");
    // The property that matters: jq DID evaluate the second argument output.
    assert_eq!(stderr, "B", "the argument's tail must still be evaluated");
    // Today's (wrong, #1279) stdout. jq gives "bcabc"\n"abcabc"\n.
    assert_eq!(stdout, "\"bcabc\"\n");
    Ok(())
}

/// The leaks themselves, asserting today's WRONG behaviour so the eventual
/// fix's diff shows exactly which ones closed.
///
/// Each row records what real jq does in a comment; the assertion is
/// succinctly's current output. When a stage of the #820 design lands, the
/// corresponding rows flip to jq's column and move into the test above.
#[test]
fn test_short_circuit_side_effect_leaks_820_932_987() -> Result<()> {
    let cases: &[SideEffectCase] = &[
        // #820's own repro. jq: stderr empty.
        (
            &["-cn", r#"isempty(1, ("B"|stderr))"#],
            None,
            "false\n",
            "B",
            0,
        ),
        // The paren spelling, mentioned in no issue: `isempty(...)` eats only
        // its own parens, so this is `IsEmpty(Paren(Comma(..)))`. jq: empty.
        (
            &["-cn", r#"isempty((1, ("B"|stderr)))"#],
            None,
            "false\n",
            "B",
            0,
        ),
        // jq: stderr empty for all three.
        (&["-cn", r#"first(1, ("B"|stderr))"#], None, "1\n", "B", 0),
        (
            &["-cn", r#"limit(1; 1, ("B"|stderr))"#],
            None,
            "1\n",
            "B",
            0,
        ),
        (&["-cn", r#"nth(0; 1, ("B"|stderr))"#], None, "1\n", "B", 0),
        // jq: stderr exactly one write (`1`), not two.
        (
            &["-c", "first(.[] | stderr)"],
            Some("[1,2]"),
            "1\n",
            "12",
            0,
        ),
        // Reached through a subtree that has already bounced to eval.rs.
        (
            &["-cn", r#"isempty(first(1, ("B"|stderr)))"#],
            None,
            "false\n",
            "B",
            0,
        ),
        // #932. jq: stderr empty for both.
        (
            &["-c", "any(2, (5|stderr); .==2)"],
            Some("2"),
            "true\n",
            "5",
            0,
        ),
        (&["-c", "IN(2, (5|stderr))"], Some("2"), "true\n", "5", 0),
        (
            &["-cn", "[IN((2,3); 2, (5|stderr))]"],
            None,
            "[true]\n",
            "5",
            0,
        ),
        // #820's halt_error repro. jq: stderr `o`, exit 1 -- the leaked `B`
        // is written before the outer message.
        (
            &["-cn", r#""o" | halt_error(1, ("B"|stderr))"#],
            None,
            "",
            "Bo",
            1,
        ),
        // The same repro's nested form. #820's text reports `innerouter` /
        // exit 1, which is STALE: #791's `Partial(_, Control::Halt)` arm in
        // `result_to_owned_full` now fires before `builtin_halt_error` writes,
        // so the inner halt wins outright. jq: `outer`, exit 1.
        (
            &["-cn", r#""outer" | halt_error(1, ("inner"|halt_error(2)))"#],
            None,
            "",
            "inner",
            2,
        ),
    ];

    for (args, stdin, want_out, want_err, want_code) in cases {
        let (stdout, stderr, code) = run_jq_full(args, *stdin)?;
        assert_eq!(
            (stdout.as_str(), stderr.trim_end_matches('\n'), code),
            (*want_out, *want_err, *want_code),
            "`{}` changed -- if a #820 stage landed, move this row into \
             `test_short_circuit_side_effect_shapes_already_match_jq_820`",
            args.join(" ")
        );
    }
    Ok(())
}

/// #987: `path(paths(f))` runs `f` against a node real jq never visits.
///
/// jq's `path()` demands only `paths(f)`'s FIRST output and aborts on it, so
/// node `3` is never reached and nothing is written. Note this is not comma
/// laziness -- jq evaluates both branches of the `(stderr, true)` comma too.
#[test]
fn test_path_paths_filter_leaks_a_never_visited_node_987() -> Result<()> {
    let (stdout, stderr, code) = run_jq_full(
        &[
            "-c",
            "path(paths(if . == 3 then (stderr, true) else true end))",
        ],
        Some("[1,2,3]"),
    )?;
    assert_eq!(code, 5, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    // jq writes only the diagnostic; succinctly prefixes the leaked `3`.
    assert!(
        stderr.starts_with('3'),
        "expected the leaked `3` before the diagnostic; stderr: {stderr:?}"
    );
    assert!(
        stderr.contains("Invalid path expression with result [0]"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

/// The same eagerness applied to `input`/`inputs` destroys data.
///
/// `input`/`inputs` pop from the process-global queue that the CLI's own
/// per-document driver loop also drains, so a discarded branch containing one
/// consumes documents that are then never processed -- exit 0, empty stderr,
/// correct-looking output. This is why #820 is a `bug`, not an `enhancement`:
/// it was filed when #723 had not yet implemented these builtins.
///
/// The `.id`-only control run is essential: without it, a future failure here
/// could be mistaken for a parsing difference rather than lost documents.
#[test]
fn test_discarded_generator_branch_consumes_input_documents_820() -> Result<()> {
    const DOCS: &str = r#"{"id":1} {"id":2} {"id":3} {"id":4}"#;

    // Control: every document is processed when nothing consumes the queue.
    let (stdout, stderr, code) = run_jq_full(&["-c", ".id"], Some(DOCS))?;
    assert_eq!(code, 0, "stderr: {stderr:?}");
    assert_eq!(
        stdout, "1\n2\n3\n4\n",
        "control run must see all 4 documents"
    );

    // `input` in a discarded branch eats every other document.
    // jq: [false,1] [false,2] [false,3] [false,4].
    let (stdout, _, code) = run_jq_full(&["-c", "[isempty(1, input), .id]"], Some(DOCS))?;
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "[false,1]\n[false,3]\n",
        "documents 2 and 4 were consumed"
    );

    // Same for `first`, which reaches it via `eval_generic`'s own arm.
    // jq: [1,1] [1,2] [1,3] [1,4].
    let (stdout, _, code) = run_jq_full(&["-c", "[first(1, input), .id]"], Some(DOCS))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[1,1]\n[1,3]\n", "documents 2 and 4 were consumed");

    // `inputs` (plural) drains the WHOLE queue from one discarded branch, so
    // the loss scales with input size: N documents in, exactly 1 processed.
    // jq: [false,1] [false,2] [false,3] [false,4].
    let (stdout, _, code) = run_jq_full(&["-c", "[isempty(1, inputs), .id]"], Some(DOCS))?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "[false,1]\n", "documents 2-4 were all consumed");

    Ok(())
}

/// A long *pipe* driven through the evaluator, not just the parser.
///
/// `MAX_EXPR_DEPTH` deliberately does not charge flat pipe/comma chains
/// (`src/jq/parser.rs`), and `test_flat_chains_are_not_charged_against_expr_depth_1156`
/// pins that a 1024-stage pipe must parse. Nothing anywhere pinned that such
/// a pipe also *evaluates* -- `tests/deep_nesting_valid_tests.rs` is entirely
/// document depth. #820's design makes `eval_pipe` recursion deeper (one
/// closure frame per stage on top of today's), so this is the guard rail for
/// design Open Risk 6.
#[test]
fn test_long_pipe_and_comma_chains_evaluate_without_overflowing() -> Result<()> {
    // Matches the parser test's bound (MAX_EXPR_DEPTH * 4).
    let filter = vec![".a"; 1024].join(" | ");
    let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some(r#"{"a":null}"#))?;
    assert_eq!(code, 0, "1024-stage pipe: stderr: {stderr:?}");
    assert_eq!(stdout, "null\n");

    let filter = format!("[{}]", vec!["1"; 1024].join(", "));
    let (stdout, stderr, code) = run_jq_full(&["-c", &filter], Some("null"))?;
    assert_eq!(code, 0, "1024-element comma list: stderr: {stderr:?}");
    assert_eq!(stdout.matches('1').count(), 1024);

    Ok(())
}
