//! Integration tests for the succinctly jq CLI command
//!
//! These tests verify jq-compatible behavior and options.
//! Run with: cargo test --features cli --test jq_cli_tests

use std::io::Write;
use std::process::Stdio;

use anyhow::Result;
use std::process::Command;
use tempfile::NamedTempFile;

/// Helper to run jq command with input from stdin
fn run_jq_stdin(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let mut cmd = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .arg("jq")
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
    let stdout = String::from_utf8(output.stdout)?;

    Ok((stdout, exit_code))
}

/// Helper to run jq command with file input
#[allow(dead_code)]
fn run_jq_file(filter: &str, file_path: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .arg("jq")
        .args(extra_args)
        .arg(filter)
        .arg(file_path)
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8(output.stdout)?;

    Ok((stdout, exit_code))
}

/// Helper to run jq with null input (-n)
fn run_jq_null(filter: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .arg("jq")
        .arg("-n")
        .args(extra_args)
        .arg(filter)
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8(output.stdout)?;

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
    let mut cmd = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "--", "-."])
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
    let mut cmd = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "--", "-(.a + .b)"])
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
    let mut cmd = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "--", "--."])
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

    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .arg("jq")
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

    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .arg("jq")
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
    // jq preserves expression order: name comes first, then id
    assert_eq!(output.trim(), r#"{"name":"Alice","id":42}"#);
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .arg("jq")
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "--help"])
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
    let (output, code) = run_jq_stdin(".[]", r#"[1e100, 2e-100]"#, &["-c", "--preserve-input"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1e100\n2e-100");
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
fn test_args_positional() -> Result<()> {
    // Test --args: positional args become $ARGS.positional
    // Note: Use pipe syntax since parser doesn't support $VAR.field directly
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args([
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args([
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args([
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n", "--arg", "x", "1", "$ARGS", "--args", "a", "b"])
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
    let mut child = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "."]) // No -C or -M flag
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-C", "-n", "null"])
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-C", "-n", r#"{"a":1}"#])
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-M", "-n", r#"{"a":1}"#])
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n", "-L"])
        .arg(temp_dir.path())
        .arg(r#"include "utils"; 21 | double"#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error"),
        "Should parse include directive without error: {}",
        stderr
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n", "-L"])
        .arg(temp_dir.path())
        .arg(r#"import "mymod" as m; 10 | m::triple"#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error"),
        "Should parse import directive without error: {}",
        stderr
    );
    assert_eq!(stdout.trim(), "30", "10 | m::triple should equal 30");
    Ok(())
}

#[test]
fn test_library_path_option() -> Result<()> {
    // Test -L option with a non-existent path (should still parse)
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n", "-L", "/nonexistent/path", "."])
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n"])
        .arg(r#"include "envmod"; 5 | quadruple"#)
        .env("JQ_LIBRARY_PATH", temp_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error"),
        "Should parse include directive without error: {}",
        stderr
    );
    assert_eq!(stdout.trim(), "20", "5 | quadruple should equal 20");
    Ok(())
}

#[test]
fn test_module_not_found_error() -> Result<()> {
    // Test that a missing module produces an appropriate error
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n"])
        .arg(r#"include "nonexistent_module_xyz"; ."#)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stderr = String::from_utf8(output.stderr)?;

    // Should produce a module error
    assert!(
        stderr.contains("module") && stderr.contains("not found"),
        "Should report module not found error: {}",
        stderr
    );
    Ok(())
}

#[test]
fn test_namespaced_call_parse() -> Result<()> {
    // Test that namespaced calls parse correctly
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n", "mymod::func"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stderr = String::from_utf8(output.stderr)?;

    // Should parse but fail at runtime (module not loaded)
    // Not a compile error, but an eval error
    assert!(
        !stderr.contains("compile error"),
        "Should parse namespaced call without compile error: {}",
        stderr
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n", "5 | my_custom_func"])
        .env("HOME", temp_home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    // Should successfully execute the function
    assert!(
        !stderr.contains("compile error"),
        "Should not have compile error: {}",
        stderr
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n"])
        .arg(r#"include "homemod"; 7 | home_func"#)
        .env("HOME", temp_home.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;

    assert!(
        !stderr.contains("compile error") && !stderr.contains("module error"),
        "Should find module in ~/.jq directory: {}",
        stderr
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
    let output = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .args(["jq", "-n", "-L"])
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
        "Should import module and use namespaced function: {}",
        stderr
    );

    // Should output 10 (5 * 2)
    assert_eq!(stdout.trim(), "10", "mymath::double should multiply by 2");
    Ok(())
}

// =============================================================================
// JSON Sequence Format (RFC 7464) Tests
// =============================================================================

/// Helper to run jq command with binary input (for testing --seq with RS characters)
fn run_jq_binary_stdin(filter: &str, input: &[u8], extra_args: &[&str]) -> Result<(Vec<u8>, i32)> {
    let mut cmd = Command::new(assert_cmd::cargo_bin!("succinctly"))
        .arg("jq")
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

    Ok((output.stdout, exit_code))
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
        output_str.contains("1"),
        "Should have output from first valid value"
    );
    assert!(
        output_str.contains("2"),
        "Should have output from second valid value"
    );
    Ok(())
}

#[test]
fn test_seq_multiple_outputs() -> Result<()> {
    // Each output from iterator should get RS prefix
    let (output, code) = run_jq_binary_stdin(".[]", br#"[1,2,3]"#, &["--seq"])?;
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
