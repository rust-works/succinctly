//! Integration tests for the succinctly yq CLI command
//!
//! These tests verify yq-compatible behavior, especially type preservation
//! for quoted vs unquoted scalars. Byte-for-byte comparison against yq itself
//! lives in tests/yq_golden_tests.rs, driven by fixtures captured from a
//! pinned yq version (see #227).
//!
//! Run with: cargo test --features cli --test yq_cli_tests

#![cfg(feature = "cli")]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;
use tempfile::NamedTempFile;

/// Helper to run yq command with input from stdin
fn run_yq_stdin(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
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

/// Helper to run yq command with input from stdin, capturing stderr too
fn run_yq_stdin_with_stderr(
    filter: &str,
    input: &str,
    extra_args: &[&str],
) -> Result<(String, String, i32)> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
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
    let stderr = String::from_utf8(output.stderr)?;
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, stderr, exit_code))
}

/// Helper to run yq command with file input
fn run_yq_file(filter: &str, file_path: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(extra_args)
        .arg(filter)
        .arg(file_path)
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, exit_code))
}

// ============================================================================
// Type Preservation Tests - Core yq Compatibility
// ============================================================================

#[test]
fn test_quoted_numeric_string_preserved() -> Result<()> {
    let yaml = r#"version: "1.0""#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"version":"1.0"}"#);
    Ok(())
}

#[test]
fn test_quoted_leading_zero_preserved() -> Result<()> {
    let yaml = r#"id: "001""#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"id":"001"}"#);
    Ok(())
}

#[test]
fn test_unquoted_number_as_number() -> Result<()> {
    let yaml = r"count: 123";
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"count":123}"#);
    Ok(())
}

#[test]
fn test_mixed_quoted_unquoted() -> Result<()> {
    let yaml = r#"
version: "1.0"
id: "001"
count: 123
price: 19.99
code: "007"
"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    let expected = r#"{"version":"1.0","id":"001","count":123,"price":19.99,"code":"007"}"#;
    assert_eq!(output.trim(), expected);
    Ok(())
}

#[test]
fn test_single_quoted_string_preserved() -> Result<()> {
    let yaml = r"version: '2.0'";
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"version":"2.0"}"#);
    Ok(())
}

#[test]
fn test_double_quoted_decimal_preserved() -> Result<()> {
    let yaml = r#"value: "3.14159""#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"value":"3.14159"}"#);
    Ok(())
}

#[test]
fn test_field_selection_preserves_type() -> Result<()> {
    let yaml = r#"
metadata:
  version: "1.0"
  build: 42
"#;
    let (output, code) = run_yq_stdin(".metadata.version", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""1.0""#);
    Ok(())
}

#[test]
fn test_array_with_quoted_numbers() -> Result<()> {
    let yaml = r#"
codes:
  - "001"
  - "002"
  - "003"
"#;
    let (output, code) = run_yq_stdin(".codes", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["001","002","003"]"#);
    Ok(())
}

// ============================================================================
// Argument Format Compatibility Tests
// ============================================================================

#[test]
fn test_output_format_equals_syntax() -> Result<()> {
    let yaml = r"test: true";
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert!(output.contains(r#"{"test":true}"#));
    Ok(())
}

#[test]
fn test_output_format_space_syntax() -> Result<()> {
    let yaml = r"test: true";
    let (output, code) = run_yq_stdin(".", yaml, &["-o", "json"])?;

    assert_eq!(code, 0);
    // Default format is pretty-printed, so check for field presence
    assert!(output.contains(r#""test""#));
    assert!(output.contains(r"true"));
    Ok(())
}

#[test]
fn test_indent_equals_syntax() -> Result<()> {
    let yaml = r"a: 1";
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);
    Ok(())
}

#[test]
fn test_indent_space_syntax() -> Result<()> {
    let yaml = r"a: 1";
    let (output, code) = run_yq_stdin(".", yaml, &["-o", "json", "-I", "0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);
    Ok(())
}

// ============================================================================
// -I 0 (compact identity) scalar type preservation — #168/#169/#170/#175
//
// Cases surfaced by code review. Correct cases are asserted directly; where
// succinctly currently diverges from yq the assertion pins the CURRENT output
// and the comment records yq's correct answer plus the tracking issue, so the
// fix is forced to update the assertion (and no silent regression slips in).
// ============================================================================

#[test]
fn test_i0_block_literal_stays_string() -> Result<()> {
    // `|-` (block literal, strip chomp) is always a string.
    let (out, code) = run_yq_stdin(".", "s: |-\n  hello\n  world\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"s":"hello\nworld"}"#);
    Ok(())
}

#[test]
fn test_i0_block_folded_stays_string() -> Result<()> {
    // `>-` (block folded, strip chomp) is always a string.
    let (out, code) = run_yq_stdin(".", "s: >-\n  hello\n  world\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"s":"hello world"}"#);
    Ok(())
}

#[test]
fn test_i0_float_one_point_zero() -> Result<()> {
    // yq preserves `1.0` as a float -> {"x":1.0}. succinctly currently collapses
    // it to the integer {"x":1} -- bug #168/#170.
    let (out, code) = run_yq_stdin(".", "x: 1.0\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"x":1}"#);
    Ok(())
}

#[test]
fn test_i0_leading_dot_float_is_number() -> Result<()> {
    // yq treats `.5` as the number 0.5 -> {"x":0.5}. Matched since the shared
    // core-schema resolver landed (#170, fixed via #226).
    let (out, code) = run_yq_stdin(".", "x: .5\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"x":0.5}"#);
    Ok(())
}

#[test]
fn test_i0_multidoc_json_stream() -> Result<()> {
    // Multi-document input streams one compact JSON value per document. This
    // already matches yq.
    let (out, code) = run_yq_stdin(".", "a: 1\n---\nb: 2\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "{\"a\":1}\n{\"b\":2}");
    Ok(())
}

#[test]
fn test_i0_multidoc_yaml_separator() -> Result<()> {
    // yq emits a `---` separator between YAML documents and preserves numeric
    // types: expected `a: 1\n---\nb: 2`. succinctly currently omits the
    // separator AND stringifies the numbers -- bug #175 (+ type preservation).
    let (out, code) = run_yq_stdin(".", "a: 1\n---\nb: 2\n", &["-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "a: \"1\"\nb: \"2\"");
    Ok(())
}

// ============================================================================
// File Input Tests
// ============================================================================

#[test]
fn test_file_input_type_preservation() -> Result<()> {
    let mut temp_file = NamedTempFile::new()?;
    writeln!(temp_file, r#"version: "1.0""#)?;
    writeln!(temp_file, r#"id: "001""#)?;
    writeln!(temp_file, r"count: 123")?;

    let path = temp_file.path().to_str().unwrap();
    let (output, code) = run_yq_file(".", path, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    let expected = r#"{"version":"1.0","id":"001","count":123}"#;
    assert_eq!(output.trim(), expected);
    Ok(())
}

#[test]
fn test_file_input_field_selection() -> Result<()> {
    let mut temp_file = NamedTempFile::new()?;
    writeln!(temp_file, r#"version: "2.5.1""#)?;
    writeln!(temp_file, r"build: 999")?;

    let path = temp_file.path().to_str().unwrap();
    let (output, code) = run_yq_file(".version", path, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""2.5.1""#);
    Ok(())
}

// ============================================================================
// YAML Special Values Tests
// ============================================================================

#[test]
fn test_null_values() -> Result<()> {
    // Note: Empty values (c:) without explicit null or flow syntax
    // may have parsing edge cases in YAML
    let yaml = r#"
a: null
b: ~
d: "null"
"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    let expected = r#"{"a":null,"b":null,"d":"null"}"#;
    assert_eq!(output.trim(), expected);
    Ok(())
}

#[test]
fn test_boolean_values() -> Result<()> {
    let yaml = r#"
a: true
b: false
c: "true"
d: "false"
"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    let expected = r#"{"a":true,"b":false,"c":"true","d":"false"}"#;
    assert_eq!(output.trim(), expected);
    Ok(())
}

// ============================================================================
// Complex Document Tests
// ============================================================================

#[test]
fn test_nested_structure_type_preservation() -> Result<()> {
    let yaml = r#"
users:
  - name: "Alice"
    id: "001"
    age: 30
  - name: "Bob"
    id: "002"
    age: 25
"#;
    let (output, code) = run_yq_stdin(".users[0]", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    let expected = r#"{"name":"Alice","id":"001","age":30}"#;
    assert_eq!(output.trim(), expected);
    Ok(())
}

#[test]
fn test_deep_nesting_preserves_types() -> Result<()> {
    let yaml = r#"
config:
  database:
    version: "5.7"
    port: 3306
    ssl: "enabled"
"#;
    let (output, code) = run_yq_stdin(".config.database", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    let expected = r#"{"version":"5.7","port":3306,"ssl":"enabled"}"#;
    assert_eq!(output.trim(), expected);
    Ok(())
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_empty_string_quoted() -> Result<()> {
    let yaml = r#"empty: """#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"empty":""}"#);
    Ok(())
}

#[test]
fn test_zero_with_decimal() -> Result<()> {
    let yaml = r#"value: "0.0""#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"value":"0.0"}"#);
    Ok(())
}

#[test]
fn test_negative_number_quoted() -> Result<()> {
    let yaml = r#"value: "-123""#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"value":"-123"}"#);
    Ok(())
}

#[test]
fn test_scientific_notation_quoted() -> Result<()> {
    let yaml = r#"value: "1.5e10""#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"value":"1.5e10"}"#);
    Ok(())
}

// ============================================================================
// Output Format Tests
// ============================================================================

#[test]
fn test_yaml_output_format() -> Result<()> {
    let yaml = r#"version: "1.0""#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=yaml"])?;

    assert_eq!(code, 0);
    assert!(output.contains("version:"));
    Ok(())
}

#[test]
fn test_compact_json_output() -> Result<()> {
    let yaml = r"
a: 1
b: 2
c: 3
";
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    // Compact output should not have newlines between fields
    assert!(!output.trim().contains('\n'));
    Ok(())
}

// ==========================================================================
// Raw input tests (-R / --raw-input)
// ==========================================================================

#[test]
fn test_raw_input_identity() -> Result<()> {
    let input = "line one\nline two\nline three";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "line one\nline two\nline three\n");
    Ok(())
}

#[test]
fn test_raw_input_json_output() -> Result<()> {
    let input = "line one\nline two";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "\"line one\"\n\"line two\"\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp() -> Result<()> {
    let input = "line one\nline two\nline three";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-s"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "- line one\n- line two\n- line three\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_json() -> Result<()> {
    let input = "a\nb\nc";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-s", "-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "[\"a\",\"b\",\"c\"]\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_length() -> Result<()> {
    let input = "one\ntwo\nthree";
    let (output, exit_code) = run_yq_stdin("length", input, &["-R", "-s"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "3");
    Ok(())
}

#[test]
fn test_raw_input_per_line_length() -> Result<()> {
    let input = "hello\nhi\nworld";
    let (output, exit_code) = run_yq_stdin("length", input, &["-R"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "5\n2\n5\n");
    Ok(())
}

#[test]
fn test_raw_input_split() -> Result<()> {
    let input = "hello world\nfoo bar";
    let (output, exit_code) = run_yq_stdin("split(\" \") | .[0]", input, &["-R"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "hello\nfoo\n");
    Ok(())
}

#[test]
fn test_raw_input_select() -> Result<()> {
    let input = "apple\nbanana\navocado\ncherry";
    let (output, exit_code) = run_yq_stdin("select(startswith(\"a\"))", input, &["-R"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "apple\navocado\n");
    Ok(())
}

#[test]
fn test_raw_input_empty_lines() -> Result<()> {
    let input = "line1\n\nline2\n\n\nline3";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R"])?;
    assert_eq!(exit_code, 0);
    // Empty lines become empty strings, which are quoted in YAML output
    assert_eq!(output, "line1\n''\nline2\n''\n''\nline3\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_filter_empty() -> Result<()> {
    let input = "line1\n\nline2\n\nline3";
    let (output, exit_code) = run_yq_stdin(
        "map(select(. != \"\"))",
        input,
        &["-R", "-s", "-o", "json", "-I", "0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "[\"line1\",\"line2\",\"line3\"]\n");
    Ok(())
}

// ============================================================================
// --doc N tests (document selection)
// ============================================================================

#[test]
fn test_doc_select_first() -> Result<()> {
    let input = "---\na: 1\n---\nb: 2\n---\nc: 3";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: 1\n");
    Ok(())
}

#[test]
fn test_doc_select_middle() -> Result<()> {
    let input = "---\na: 1\n---\nb: 2\n---\nc: 3";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "1"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "b: 2\n");
    Ok(())
}

#[test]
fn test_doc_select_last() -> Result<()> {
    let input = "---\na: 1\n---\nb: 2\n---\nc: 3";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "2"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "c: 3\n");
    Ok(())
}

#[test]
fn test_doc_select_out_of_range() -> Result<()> {
    let input = "---\na: 1\n---\nb: 2";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "5"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, ""); // No output for out of range
    Ok(())
}

#[test]
fn test_doc_select_with_query() -> Result<()> {
    let input = "---\nname: Alice\nage: 30\n---\nname: Bob\nage: 25";
    let (output, exit_code) = run_yq_stdin(".name", input, &["--doc", "1"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "Bob\n");
    Ok(())
}

#[test]
fn test_doc_select_json_output() -> Result<()> {
    let input = "---\na: 1\n---\nb: 2";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "0", "-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "{\"a\":1}\n");
    Ok(())
}

#[test]
fn test_doc_select_single_doc() -> Result<()> {
    // Single document (no separators) - --doc 0 should work
    let input = "a: 1\nb: 2";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: 1\nb: 2\n");
    Ok(())
}

#[test]
fn test_doc_select_single_doc_out_of_range() -> Result<()> {
    // Single document - --doc 1 should return nothing
    let input = "a: 1";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "1"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "");
    Ok(())
}

#[test]
fn test_doc_incompatible_with_raw_input() -> Result<()> {
    let input = "line1\nline2";
    let (_, exit_code) = run_yq_stdin(".", input, &["--doc", "0", "-R"])?;
    // Should fail with non-zero exit code
    assert_ne!(exit_code, 0);
    Ok(())
}

#[test]
fn test_doc_with_slurp() -> Result<()> {
    // --doc with --slurp filters before slurping
    let input = "---\na: 1\n---\nb: 2\n---\nc: 3";
    let (output, exit_code) =
        run_yq_stdin(".", input, &["--doc", "1", "-s", "-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    // Should slurp only the selected document into an array
    assert_eq!(output, "[{\"b\":2}]\n");
    Ok(())
}

// ============================================================================
// split_doc tests
// ============================================================================

#[test]
fn test_split_doc_basic_array() -> Result<()> {
    // split_doc should add --- between results
    let input = "[1, 2, 3]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "1\n---\n2\n---\n3\n");
    Ok(())
}

#[test]
fn test_split_doc_with_strings() -> Result<()> {
    let input = "[\"hello\", \"world\"]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "hello\n---\nworld\n");
    Ok(())
}

#[test]
fn test_split_doc_with_objects() -> Result<()> {
    let input = "[{name: alice}, {name: bob}]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "name: alice\n---\nname: bob\n");
    Ok(())
}

#[test]
fn test_split_doc_single_result() -> Result<()> {
    // With only one result, no separator should be added
    let input = "[42]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "42\n");
    Ok(())
}

#[test]
fn test_split_doc_with_no_doc_flag() -> Result<()> {
    // --no-doc should suppress document separators
    let input = "[1, 2, 3]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &["--no-doc"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "1\n2\n3\n");
    Ok(())
}

#[test]
fn test_split_doc_json_output() -> Result<()> {
    // JSON output should not get --- separators
    let input = "[1, 2, 3]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "1\n2\n3\n");
    Ok(())
}

#[test]
fn test_split_doc_with_filter() -> Result<()> {
    // split_doc can be combined with other filters
    let input = "[1, 2, 3, 4, 5]";
    let (output, exit_code) = run_yq_stdin(".[] | select(. > 2) | split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "3\n---\n4\n---\n5\n");
    Ok(())
}

#[test]
fn test_split_doc_empty_array() -> Result<()> {
    // Empty array should produce no output
    let input = "[]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "");
    Ok(())
}

#[test]
fn test_split_doc_nested_arrays() -> Result<()> {
    // split_doc on nested structure
    let input = "[[1, 2], [3, 4]]";
    let (output, exit_code) = run_yq_stdin(".[] | split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    // Each sub-array is output as a YAML sequence
    assert_eq!(output, "- 1\n- 2\n---\n- 3\n- 4\n");
    Ok(())
}

#[test]
fn test_split_doc_identity_passthrough() -> Result<()> {
    // split_doc is semantically identity - just changes output formatting
    let input = "42";
    let (output, exit_code) = run_yq_stdin("split_doc", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "42\n");
    Ok(())
}

// =============================================================================
// Compatibility tests - YAML merge keys (<<)
// =============================================================================

#[test]
#[ignore] // TODO: Fix - merge keys should be expanded
fn test_yaml_merge_key_expansion() -> Result<()> {
    // yq: merge key << should be expanded
    let input = "default: &default\n  a: 1\nitem:\n  <<: *default\n  b: 2";
    let (output, exit_code) = run_yq_stdin(".item", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    // Should have both 'a' from merge and 'b' from item
    assert!(
        output.contains("\"a\"") && output.contains("\"b\""),
        "merge key should expand anchor: got {output}"
    );
    // Should NOT have literal << key
    assert!(
        !output.contains("\"<<\""),
        "merge key << should be expanded, not literal: {output}"
    );
    Ok(())
}

#[test]
#[ignore] // TODO: Fix - merge keys should be expanded
fn test_yaml_merge_key_override() -> Result<()> {
    // When item has same key as anchor, item's value takes precedence
    let input = "default: &default\n  a: 1\n  b: original\nitem:\n  <<: *default\n  b: override";
    let (output, exit_code) = run_yq_stdin(".item.b", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert!(
        output.contains("override"),
        "item's value should override anchor: got {output}"
    );
    Ok(())
}

#[test]
fn test_yaml_anchor_alias_without_merge() -> Result<()> {
    // Regular anchors/aliases (not merge keys) should work
    let input = "anchor: &anchor\n  x: 1\nref: *anchor";
    let (output, exit_code) = run_yq_stdin(".ref.x", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "1");
    Ok(())
}

// =============================================================================
// Alias cycle rejection (#153) - cyclic anchors must be a clean parse error,
// not a stack-overflow abort
// =============================================================================

#[test]
fn test_yaml_alias_cycle_is_parse_error() -> Result<()> {
    // Issue #153 repro: self-referential anchor + a query that follows it.
    // Before the fix this aborted with a stack overflow (exit 134).
    let input = "a: &anchor\n  self: *anchor";
    let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".a.self.self", input, &[])?;
    assert_eq!(exit_code, 1, "expected clean error exit, stderr: {stderr}");
    assert_eq!(stdout, "", "no output should be produced: {stdout}");
    assert!(
        stderr.contains("cyclic alias 'anchor'"),
        "stderr should name the cycle: {stderr}"
    );
    Ok(())
}

#[test]
fn test_yaml_alias_cycle_fails_for_identity_filter() -> Result<()> {
    // Rejection happens at parse time, independent of the filter.
    let input = "a: &anchor\n  self: *anchor";
    let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".", input, &[])?;
    assert_eq!(exit_code, 1, "expected clean error exit, stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("cyclic alias 'anchor'"),
        "stderr should name the cycle: {stderr}"
    );
    Ok(())
}

#[test]
fn test_yaml_direct_self_alias_cycle() -> Result<()> {
    let input = "a: &x *x";
    let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".", input, &[])?;
    assert_eq!(exit_code, 1, "expected clean error exit, stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("cyclic alias 'x'"),
        "stderr should name the cycle: {stderr}"
    );
    Ok(())
}

// =============================================================================
// Compatibility tests - Block scalar edge cases
// =============================================================================

#[test]
fn test_block_scalar_literal_with_clip() -> Result<()> {
    // Literal style with clip chomping (default): single trailing newline
    let input = "text: |\n  line1\n  line2\n";
    let (output, exit_code) = run_yq_stdin(".text", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "\"line1\\nline2\\n\"");
    Ok(())
}

#[test]
fn test_block_scalar_literal_with_strip() -> Result<()> {
    // Literal style with strip chomping (|-): no trailing newline
    let input = "text: |-\n  line1\n  line2\n";
    let (output, exit_code) = run_yq_stdin(".text", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "\"line1\\nline2\"");
    Ok(())
}

#[test]
fn test_block_scalar_literal_with_keep() -> Result<()> {
    // Literal style with keep chomping (|+): preserve trailing newlines
    let input = "text: |+\n  line1\n  line2\n\n";
    let (output, exit_code) = run_yq_stdin(".text", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "\"line1\\nline2\\n\\n\"");
    Ok(())
}

#[test]
fn test_block_scalar_folded() -> Result<()> {
    // Folded style (>): newlines become spaces
    let input = "text: >\n  line1\n  line2\n";
    let (output, exit_code) = run_yq_stdin(".text", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    // Folded converts newlines to spaces (with trailing newline from clip)
    assert_eq!(output.trim(), "\"line1 line2\\n\"");
    Ok(())
}

// =============================================================================
// Compatibility tests - Multi-document handling
// =============================================================================

#[test]
fn test_multi_doc_select_first() -> Result<()> {
    let input = "---\na: 1\n---\nb: 2";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "0"])?;
    assert_eq!(exit_code, 0);
    assert!(
        output.contains("a:"),
        "should output first document: {output}"
    );
    assert!(
        !output.contains("b:"),
        "should not include second document: {output}"
    );
    Ok(())
}

#[test]
fn test_multi_doc_select_second() -> Result<()> {
    let input = "---\na: 1\n---\nb: 2";
    let (output, exit_code) = run_yq_stdin(".", input, &["--doc", "1"])?;
    assert_eq!(exit_code, 0);
    assert!(
        output.contains("b:"),
        "should output second document: {output}"
    );
    assert!(
        !output.contains("a:"),
        "should not include first document: {output}"
    );
    Ok(())
}

// =============================================================================
// Compatibility tests - Type preservation
// =============================================================================

#[test]
fn test_quoted_number_stays_string() -> Result<()> {
    // Quoted "1.0" should stay as string, not become number 1
    let input = "version: \"1.0\"";
    let (output, exit_code) = run_yq_stdin(".version", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    // Should be "1.0" (string), not 1 or 1.0 (number)
    assert_eq!(output.trim(), "\"1.0\"");
    Ok(())
}

#[test]
fn test_unquoted_number_becomes_number() -> Result<()> {
    // Unquoted 1.0 should be a number
    let input = "version: 1.0";
    let (output, exit_code) = run_yq_stdin(".version", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    // Should be 1 (integer, as 1.0 parses to 1 in jq-style)
    let trimmed = output.trim();
    assert!(
        trimmed == "1" || trimmed == "1.0",
        "unquoted number should be numeric: {trimmed}"
    );
    Ok(())
}

#[test]
fn test_quoted_bool_stays_string() -> Result<()> {
    // Quoted "true" should stay as string
    let input = "flag: \"true\"";
    let (output, exit_code) = run_yq_stdin(".flag", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "\"true\"");
    Ok(())
}

#[test]
fn test_unquoted_bool_becomes_bool() -> Result<()> {
    // Unquoted true should be boolean
    let input = "flag: true";
    let (output, exit_code) = run_yq_stdin(".flag", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "true");
    Ok(())
}

#[test]
fn test_multibyte_value_survives_simd_escape_scan() -> Result<()> {
    // Regression test for the x86 signed-compare bug (#150/#230): the AVX2/
    // SSE2 `find_json_escape` kernels misread bytes >= 0x80 as control
    // characters, so a >= 16-byte value with multibyte UTF-8 was cut
    // mid-character. This is the original repro; the CLI path also covers
    // the jq streaming caller (`stream_json_string`).
    let input = "---\nwanted: love \u{2665} and peace \u{262e}\n";

    let (output, exit_code) = run_yq_stdin(".wanted", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "love \u{2665} and peace \u{262e}");

    let (json, exit_code) = run_yq_stdin(".wanted", input, &["-o", "json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(json.trim(), "\"love \u{2665} and peace \u{262e}\"");
    Ok(())
}

// ============================================================================
// Colorized JSON Output
// ============================================================================

#[test]
fn test_colorized_json_output_is_token_aware() -> Result<()> {
    // Pretty (non--I=0) JSON output goes through the shared jq colorizer:
    // keys are colored as keys and keywords as whole tokens (#181).
    let input = "a: true\nn: null\ns: hello\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-C"])?;
    assert_eq!(exit_code, 0);

    // Object key colored with the key color, not the string color.
    assert!(
        output.contains("\u{1b}[1;34m\"a\""),
        "key coloring missing: {output:?}"
    );
    // String values keep the string color.
    assert!(
        output.contains("\u{1b}[0;32m\"hello\""),
        "string coloring missing: {output:?}"
    );
    // Keywords are one colored token...
    assert!(
        output.contains("\u{1b}[0;39mtrue\u{1b}[0m"),
        "whole-token true missing: {output:?}"
    );
    assert!(
        output.contains("\u{1b}[1;30mnull\u{1b}[0m"),
        "whole-token null missing: {output:?}"
    );
    // ...never the old per-letter coloring that painted stray `t`/`r`/`u`/`e`.
    assert!(
        !output.contains("\u{1b}[34mt\u{1b}[0m"),
        "per-letter coloring returned: {output:?}"
    );
    Ok(())
}

#[test]
fn test_build_configuration_flag() -> Result<()> {
    // --build-configuration prints diagnostics and exits successfully.
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("--build-configuration")
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert!(
        stdout.starts_with("succinctly yq build configuration:"),
        "unexpected header: {stdout:?}"
    );
    assert!(stdout.contains("Features:"));
    Ok(())
}
