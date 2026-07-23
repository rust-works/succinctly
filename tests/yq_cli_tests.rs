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
    // #175 (fixed): yq emits a `---` separator between YAML documents (never
    // before the first) and preserves numeric types.
    let (out, code) = run_yq_stdin(".", "a: 1\n---\nb: 2\n", &["-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "a: 1\n---\nb: 2");
    Ok(())
}

#[test]
fn test_i0_identity_preserves_scalar_representation() -> Result<()> {
    // #175 (fixed): the compact-YAML identity fast path re-emits source plain
    // scalars verbatim, preserving both type and representation exactly as yq
    // does (`1.0` stays `1.0`, `.5` stays `.5`, `yes` stays unquoted), and
    // quoted source scalars keep their quotes.
    let input = "a: 1\nb: true\nc: hello\nd: \"1\"\ne: 1.0\nf: .5\ng: yes\n";
    let (out, code) = run_yq_stdin(".", input, &["-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(
        out.trim(),
        "a: 1\nb: true\nc: hello\nd: \"1\"\ne: 1.0\nf: .5\ng: yes"
    );
    Ok(())
}

#[test]
fn test_i0_multidoc_navigation_separator() -> Result<()> {
    // #175 (fixed): yq also separates per-document results of a navigation
    // query with `---` in YAML output mode.
    let (out, code) = run_yq_stdin(".a", "a: 1\n---\na: 2\n", &["-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "1\n---\n2");
    Ok(())
}

#[test]
fn test_i0_multidoc_separator_skips_empty_results() -> Result<()> {
    // #175: a document whose query yields no values gets no separator either
    // side (yq prints just `1` here).
    let (out, code) = run_yq_stdin(".[]", "- 1\n---\n[]\n", &["-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "1");
    Ok(())
}

#[test]
fn test_i0_multidoc_doc_filter_no_separator() -> Result<()> {
    // #175: selecting a single document with --doc emits no stray separator.
    let (out, code) = run_yq_stdin(".", "a: 1\n---\nb: 2\n", &["-I=0", "--doc", "1"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "b: 2");
    Ok(())
}

#[test]
fn test_i0_multifile_yaml_separator() -> Result<()> {
    // #175: documents from separate input files are also `---`-separated,
    // matching yq's concatenated document stream.
    let mut f1 = NamedTempFile::new()?;
    f1.write_all(b"a: 1\n")?;
    let mut f2 = NamedTempFile::new()?;
    f2.write_all(b"b: 2\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-I=0", "."])
        .arg(f1.path())
        .arg(f2.path())
        .output()?;

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout)?.trim(), "a: 1\n---\nb: 2");
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
    // yq -R -s (jq semantics): the entire input is one string, not an
    // array of lines
    let input = "line one\nline two\nline three";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-s"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "\"line one\\nline two\\nline three\"\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_json() -> Result<()> {
    let input = "a\nb\nc";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-s", "-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "\"a\\nb\\nc\"\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_length() -> Result<()> {
    // length of the whole input string (13 chars), not the line count
    let input = "one\ntwo\nthree";
    let (output, exit_code) = run_yq_stdin("length", input, &["-R", "-s"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "13");
    Ok(())
}

#[test]
fn test_raw_input_slurp_preserves_trailing_newline() -> Result<()> {
    let input = "a\nb\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-s", "-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "\"a\\nb\\n\"\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_raw_output() -> Result<()> {
    let input = "x\ny";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-s", "-r"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "x\ny\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_empty_input() -> Result<()> {
    let (output, exit_code) = run_yq_stdin(".", "", &["-R", "-s"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "''\n");
    Ok(())
}

#[test]
fn test_raw_input_slurp_multiple_files() -> Result<()> {
    let mut file1 = NamedTempFile::new()?;
    writeln!(file1, "a")?;

    let mut file2 = NamedTempFile::new()?;
    writeln!(file2, "b")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-R", "-s", "-o", "json", "-I", "0"])
        .arg(".")
        .arg(file1.path())
        .arg(file2.path())
        .output()?;

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout)?, "\"a\\nb\\n\"\n");
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
    // Under jq -R -s semantics the input is one string, so line handling
    // uses the split("\n") idiom
    let input = "line1\n\nline2\n\nline3";
    let (output, exit_code) = run_yq_stdin(
        "split(\"\\n\") | map(select(. != \"\"))",
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
// Colorized Output (JSON and YAML)
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
fn test_float_modulo_uses_float_semantics() -> Result<()> {
    // yq (unlike jq) performs float modulo: 10.5 % 3 => 1.5.
    // This guards against the jq integer-truncation fix (issue #164)
    // leaking into yq's semantics.
    let (output, exit_code) = run_yq_stdin("10.5 % 3", "null", &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "1.5");
    Ok(())
}

#[test]
fn test_colorized_yaml_output_mapping() -> Result<()> {
    // Default (YAML) output with -C goes through the YAML colorizer, which is
    // a separate path from the JSON colorizer above: mapping keys are cyan.
    let input = "name: Alice\nage: 30\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-C"])?;
    assert_eq!(exit_code, 0);

    // Keys are wrapped in cyan (\x1b[36m ... \x1b[0m); the plain text survives.
    assert!(
        output.contains("\u{1b}[36mname\u{1b}[0m"),
        "cyan key coloring missing: {output:?}"
    );
    assert!(
        output.contains("\u{1b}[36mage\u{1b}[0m"),
        "cyan key coloring missing: {output:?}"
    );
    Ok(())
}

#[test]
fn test_colorized_yaml_output_sequence_dash() -> Result<()> {
    // Block-sequence dashes are colored yellow by the YAML colorizer.
    let input = "items:\n  - a\n  - b\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-C"])?;
    assert_eq!(exit_code, 0);
    assert!(
        output.contains("\u{1b}[33m-\u{1b}[0m"),
        "yellow sequence dash missing: {output:?}"
    );
    Ok(())
}

// ============================================================================
// Special float values (NaN / Infinity)
// ============================================================================

#[test]
fn test_yaml_special_floats_passthrough() -> Result<()> {
    // .nan / .inf / -.inf round-trip through YAML output unchanged.
    let input = "x: .nan\ny: .inf\nz: -.inf\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "x: .nan\ny: .inf\nz: -.inf\n");
    Ok(())
}

#[test]
fn test_yaml_special_floats_to_json_are_null() -> Result<()> {
    // JSON has no NaN/Infinity literals, so non-finite floats serialize as null.
    let input = "x: .nan\ny: .inf\nz: -.inf\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "{\"x\":null,\"y\":null,\"z\":null}\n");
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

// ============================================================================
// From-File Filter Tests (#177)
// ============================================================================

/// Helper to run yq with --from-file and positional input files.
///
/// stdin is explicitly null so that a regression to reading stdin produces
/// an empty-input error instead of hanging.
fn run_yq_from_file(
    filter_path: &str,
    files: &[&str],
    extra_args: &[&str],
) -> Result<(String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(extra_args)
        .arg("--from-file")
        .arg(filter_path)
        .args(files)
        .stdin(Stdio::null())
        .output()?;

    let stdout = String::from_utf8(output.stdout)?;
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, exit_code))
}

#[test]
fn test_from_file_with_input_file() -> Result<()> {
    // Regression test for #177: clap binds the input file positional to
    // `filter`, and yq dropped it and read stdin instead.
    let mut filter_file = NamedTempFile::new()?;
    writeln!(filter_file, ".name")?;
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "name: Alice")?;
    writeln!(input_file, "age: 30")?;

    let (output, code) = run_yq_from_file(
        filter_file.path().to_str().unwrap(),
        &[input_file.path().to_str().unwrap()],
        &[],
    )?;

    assert_eq!(code, 0);
    assert_eq!(output, "Alice\n");
    Ok(())
}

#[test]
fn test_from_file_with_input_file_fast_path() -> Result<()> {
    // Same as test_from_file_with_input_file, but -o=json -I=0 routes the
    // query through the M2 streaming fast path's file loop.
    let mut filter_file = NamedTempFile::new()?;
    writeln!(filter_file, ".name")?;
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "name: Alice")?;

    let (output, code) = run_yq_from_file(
        filter_file.path().to_str().unwrap(),
        &[input_file.path().to_str().unwrap()],
        &["-o=json", "-I=0"],
    )?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""Alice""#);
    Ok(())
}

#[test]
fn test_from_file_with_stdin() -> Result<()> {
    // --from-file with no positional input file must still read stdin.
    let mut filter_file = NamedTempFile::new()?;
    writeln!(filter_file, ".name")?;

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("--from-file")
        .arg(filter_file.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(b"name: Bob\n")?;
    }

    let output = cmd.wait_with_output()?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout)?, "Bob\n");
    Ok(())
}

#[test]
fn test_from_file_with_multiple_input_files() -> Result<()> {
    let mut filter_file = NamedTempFile::new()?;
    writeln!(filter_file, ".a")?;
    let mut input_one = NamedTempFile::new()?;
    writeln!(input_one, "a: 1")?;
    let mut input_two = NamedTempFile::new()?;
    writeln!(input_two, "a: 2")?;

    let (output, code) = run_yq_from_file(
        filter_file.path().to_str().unwrap(),
        &[
            input_one.path().to_str().unwrap(),
            input_two.path().to_str().unwrap(),
        ],
        &[],
    )?;

    assert_eq!(code, 0);
    assert_eq!(output, "---\n1\n---\n2\n");
    Ok(())
}

#[test]
fn test_inplace_from_file() -> Result<()> {
    // Before #177, --inplace --from-file bailed with "requires at least one
    // file argument" because the input file was swallowed by `filter`.
    let mut filter_file = NamedTempFile::new()?;
    writeln!(filter_file, ".name")?;
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "name: Alice")?;

    let (output, code) = run_yq_from_file(
        filter_file.path().to_str().unwrap(),
        &[input_file.path().to_str().unwrap()],
        &["-i"],
    )?;

    assert_eq!(code, 0);
    assert_eq!(output, "");
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "Alice\n");
    Ok(())
}

// ============================================================================
// Exit Status Tests (-e / --exit-status)
//
// Regression coverage for #178: the M2 identity fast paths streamed output
// without tracking falsiness, so `-e` wrongly exited 0 on false/null. The
// compact flags (-I 0) are what route these through the fast path; the
// default-indent tests cover the non-fast path for contrast.
//
// Semantics match mikefarah yq (verified against v4.53.3), not jq: exit 1
// unless SOME result is truthy, with empty output and all-falsy output both
// reported as "Error: no matches found" on stderr. jq's last-value-wins rule
// and its distinct no-output exit code 4 do not apply to yq.
// ============================================================================

#[test]
fn test_exit_status_fast_path_false() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "false", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_null() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "null", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_tilde_null() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "~", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_true() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "true", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_zero_is_truthy() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "0", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_quoted_false_is_truthy() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "\"false\"", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_mapping_is_truthy() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "a: 1", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

// Multi-doc inputs below start with a mapping doc: the current indexer folds
// scalar-only multi-docs (e.g. "true\n---\nfalse") into one plain scalar, so
// a mapping first doc is needed to actually exercise the per-document loop.

#[test]
fn test_exit_status_fast_path_multidoc_any_truthy_wins() -> Result<()> {
    // yq exits 0 if any document is truthy, even when the last one is falsy
    // (unlike jq, where only the last output value counts).
    let (_, exit_code) = run_yq_stdin(".", "a: 1\n---\nfalse\n", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    let (_, exit_code) = run_yq_stdin(".", "a: 1\n---\nnull\n", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_multidoc_all_truthy() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "a: 1\n---\nb: 2\n", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_doc_filter_selects_falsy() -> Result<()> {
    let input = "a: 1\n---\nfalse\n";
    let (_, exit_code) = run_yq_stdin(".", input, &["-e", "-I", "0", "--doc", "1"])?;
    assert_eq!(exit_code, 1);
    // Selecting the truthy mapping doc instead exits 0.
    let (_, exit_code) = run_yq_stdin(".", input, &["-e", "-I", "0", "--doc", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_json_false() -> Result<()> {
    let (output, exit_code) = run_yq_stdin(".", "false", &["-e", "-o", "json", "-I", "0"])?;
    assert_eq!(output.trim(), "false");
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_json_null() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "null", &["-e", "-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_json_true() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "true", &["-e", "-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_file_input_false() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    file.write_all(b"false\n")?;
    let (_, exit_code) = run_yq_file(".", file.path().to_str().unwrap(), &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_false() -> Result<()> {
    // Default indent disables the fast path; this path was already correct.
    let (_, exit_code) = run_yq_stdin(".", "false", &["-e"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_true() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "true", &["-e"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_nonfast_multidoc_any_truthy_wins() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "a: 1\n---\nfalse\n", &["-e"])?;
    assert_eq!(exit_code, 0);
    Ok(())
}

#[test]
fn test_exit_status_comma_any_truthy_wins() -> Result<()> {
    // Multiple results from one document: any truthy result exits 0,
    // regardless of order.
    let (_, exit_code) = run_yq_stdin(".a, .b", "a: true\nb: false\n", &["-e"])?;
    assert_eq!(exit_code, 0);
    let (_, exit_code) = run_yq_stdin(".a, .b", "a: false\nb: true\n", &["-e"])?;
    assert_eq!(exit_code, 0);
    let (_, exit_code) = run_yq_stdin(".a, .b", "a: false\nb: false\n", &["-e"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_no_output_exits_one() -> Result<()> {
    // yq folds "no output" into the same exit code 1 (jq would use 4).
    let (_, exit_code) = run_yq_stdin(".a | select(. == 2)", "a: 1", &["-e"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_fast_path_doc_filter_out_of_range_exits_one() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "a: 1", &["-e", "-I", "0", "--doc", "9"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_empty_input_exits_one() -> Result<()> {
    let (_, exit_code) = run_yq_stdin(".", "", &["-e", "-I", "0"])?;
    assert_eq!(exit_code, 1);
    Ok(())
}

#[test]
fn test_exit_status_prints_no_matches_found_to_stderr() -> Result<()> {
    // Match yq's exact stderr message; stdout still carries the falsy value.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-e", "-I", "0", "."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(b"false")?;
    }
    let output = cmd.wait_with_output()?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)?.trim(),
        "Error: no matches found"
    );
    Ok(())
}

#[test]
fn test_exit_status_no_stderr_message_when_truthy() -> Result<()> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(["yq", "-e", "-I", "0", "."])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(b"true")?;
    }
    let output = cmd.wait_with_output()?;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stderr)?.trim(), "");
    Ok(())
}

#[test]
fn test_arithmetic_semantics_match_between_stdin_and_null_input() -> Result<()> {
    // The stdin/file path (generic evaluator) and the -n path (full evaluator)
    // must agree on yq numeric semantics. Before threading EvalSemantics through
    // the generic evaluator, the stdin path silently used jq semantics.
    for expr in ["10.5 % 3", "1 / 0", "7.5 % 2.5"] {
        let (stdin_out, stdin_code) = run_yq_stdin(expr, "null", &["-o=json", "-I=0"])?;
        let (null_out, null_code) = run_yq_stdin(expr, "", &["-n", "-o=json", "-I=0"])?;
        assert_eq!(
            (stdin_out.trim(), stdin_code),
            (null_out.trim(), null_code),
            "stdin vs -n disagree for `{expr}`",
        );
    }
    Ok(())
}

/// #262: yq JSON control-char escaping must be identical across all three yq
/// output paths — pretty (`-o json`), the compact M2 streaming fast path, and
/// the compact DOM formatter (`OwnedValue`) — and must match `mikefarah/yq`:
/// backspace/form-feed as `\u0008`/`\u000c` (NOT jq's `\b`/`\f`), C1 controls
/// left raw, other C0 controls as `\u00xx`.
#[test]
fn test_yq_json_control_char_escaping_consistent_across_paths() -> Result<()> {
    // s = "a<BS>b<FF>c<U+0085>d<NUL>e" via YAML double-quoted escapes
    // (\b, \f, \x85 = C1 NEL, \x00 = NUL).
    let yaml = "s: \"a\\bb\\fc\\x85d\\x00e\"\n";
    // yq re-emits BS/FF as \u0008/\u000c, leaves the C1 (U+0085) byte raw,
    // and escapes NUL as \u0000.
    let expected = "\"a\\u0008b\\u000cc\u{85}d\\u0000e\"";

    // Compact streaming fast path: `.s` is M2-streamable.
    let (stream, code) = run_yq_stdin(".s", yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(stream.trim(), expected, "compact streaming path");

    // Compact DOM path: `.s + ""` is not streamable, so it routes through the
    // OwnedValue formatter.
    let (dom, code) = run_yq_stdin(".s + \"\"", yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(dom.trim(), expected, "compact DOM path");

    // Pretty path: a bare scalar has no indentation, so its bytes equal compact.
    let (pretty, code) = run_yq_stdin(".s", yaml, &["-o=json"])?;
    assert_eq!(code, 0);
    assert_eq!(pretty.trim(), expected, "pretty path");

    // Mutual consistency is the core guarantee (#262).
    assert_eq!(stream.trim(), dom.trim(), "streaming vs DOM");
    assert_eq!(stream.trim(), pretty.trim(), "streaming vs pretty");
    Ok(())
}
