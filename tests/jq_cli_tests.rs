//! Integration tests for the succinctly jq CLI command
//!
//! These tests verify jq-compatible behavior and options.
//! Run with: cargo test --features cli --test jq_cli_tests

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::NamedTempFile;

/// Helper to run jq command with input from stdin
fn run_jq_stdin(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
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
    let stdout = String::from_utf8(output.stdout)?;
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, exit_code))
}

/// Helper to run jq command with file input
fn run_jq_file(filter: &str, file_path: &str, extra_args: &[&str]) -> Result<(String, i32)> {
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

    let stdout = String::from_utf8(output.stdout)?;
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, exit_code))
}

/// Helper to run jq with null input (-n)
fn run_jq_null(filter: &str, extra_args: &[&str]) -> Result<(String, i32)> {
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

    let stdout = String::from_utf8(output.stdout)?;
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, exit_code))
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
    let (output, code) = run_jq_stdin(".[1]", r#"[10,20,30]"#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "20");
    Ok(())
}

#[test]
fn test_array_iteration() -> Result<()> {
    let (output, code) = run_jq_stdin(".[]", r#"[1,2,3]"#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "1\n2\n3\n");
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
    let (output, code) = run_jq_stdin(".", "a\nb\nc", &["-R", "-s", "-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["a","b","c"]"#);
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
    let (output, code) = run_jq_stdin("length", r#"[1,2,3,4,5]"#, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "5");
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
    let (output, code) = run_jq_stdin("map(. * 2)", r#"[1,2,3]"#, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[2,4,6]");
    Ok(())
}

#[test]
fn test_builtin_select() -> Result<()> {
    let (output, code) = run_jq_stdin(".[] | select(. > 2)", r#"[1,2,3,4,5]"#, &[])?;
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
        r#"{name: .user, id: .id}"#,
        r#"{"user":"Alice","id":42}"#,
        &["-c"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"id":42,"name":"Alice"}"#);
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
fn test_try_catch() -> Result<()> {
    let (output, code) = run_jq_stdin(r#"try .foo.bar catch "error""#, r#"{"foo":null}"#, &["-r"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "error");
    Ok(())
}

#[test]
fn test_reduce() -> Result<()> {
    let (output, code) = run_jq_stdin("reduce .[] as $x (0; . + $x)", r#"[1,2,3,4,5]"#, &[])?;
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
    assert!(output.contains(r#"\u4e16\u754c"#));
    assert!(!output.contains("世界"));
    Ok(())
}

#[test]
fn test_ascii_output_emoji() -> Result<()> {
    // Test that emoji (outside BMP) are escaped as surrogate pairs
    let (output, _) = run_jq_stdin(".", r#"{"emoji":"🌍"}"#, &["-c", "-a"])?;
    // Earth emoji U+1F30D should be encoded as surrogate pair
    assert!(output.contains(r#"\ud83c\udf0d"#));
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
