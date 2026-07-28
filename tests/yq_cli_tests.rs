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
    // #169 (fixed): a scalar the core-schema resolver typed as `!!float` is
    // emitted with its decimal point, so `1.0` no longer collapses to `1`.
    let (out, code) = run_yq_stdin(".", "x: 1.0\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"x":1.0}"#);
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
fn test_duplicate_mapping_key_is_last_wins() -> Result<()> {
    // YAML 1.2 / yq: a mapping with a duplicate key resolves `.key` to the
    // *last* occurrence, not the first (issue #174).
    let yaml = "a: 1\na: 2\n";
    let (output, code) = run_yq_stdin(".a", yaml, &[])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), "2");
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
// Anchored sequence items (#328) - an anchor on `- ` binds to the item's value
// whatever its kind. Expectations are mikefarah/yq v4.53.3 output.
// =============================================================================

#[test]
fn test_yaml_anchored_seq_item_with_block_collection() -> Result<()> {
    // The headline #328 repro: the mapping used to be read as the plain scalar
    // "k", with `v` leaking out as a top-level key.
    let input = "list:\n  - &m\n    k: v\n  - *m\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"list":[{"k":"v"},{"k":"v"}]}"#);
    Ok(())
}

#[test]
fn test_yaml_anchored_seq_item_with_flow_collection() -> Result<()> {
    // The anchor used to be swallowed into the key text, so the alias resolved
    // to nothing and the mapping came out as {"": "1}"}.
    let input = "items:\n  - &first {id: 1}\n  - *first\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"items":[{"id":1},{"id":1}]}"#);
    Ok(())
}

#[test]
fn test_yaml_anchored_seq_item_alias_is_navigable() -> Result<()> {
    // Not just identity: the alias must resolve to a real mapping you can index.
    let input = "list:\n  - &m\n    k: v\n  - *m\n";
    let (output, exit_code) = run_yq_stdin(".list[1].k", input, &["-o=json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#""v""#);
    Ok(())
}

#[test]
fn test_yaml_anchor_on_compact_mapping_key_binds_to_key() -> Result<()> {
    // `- &a k: v` anchors the *key*, matching yq. Before the fix `&a k` was
    // swallowed into the key text and the alias resolved to nothing.
    let input = "items:\n  - &a k: v\n  - *a\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"items":[{"k":"v"},"k"]}"#);
    Ok(())
}

#[test]
fn test_yaml_anchored_tag_in_seq_item_is_rejected() -> Result<()> {
    // Consuming the anchor before dispatching means the tag is now seen rather
    // than absorbed into a plain scalar, so `- &a !!str x` errors instead of
    // silently yielding the string "!!str x". Tags are documented non-support
    // (#224); `a: !!str 1` already errored the same way.
    let input = "items:\n  - &a !!str x\n";
    let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".", input, &[])?;
    assert_eq!(exit_code, 1, "expected clean error exit, stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("tags (!) not supported"),
        "stderr should name the tag: {stderr}"
    );
    Ok(())
}

#[test]
fn test_yaml_flow_context_rejects_tags_like_block_context() -> Result<()> {
    // #369. Block context has always errored on `!` via `check_unsupported`;
    // flow context fell through to the plain-scalar readers and absorbed the
    // tag as text, so `a: [!!str x]` yielded the *string* `"!!str x"`. Silently
    // wrong data is worse than a refusal, which is why this is a bug in its own
    // right rather than something that waits for tag support (#224).
    //
    // Every flow position that reaches a plain-scalar reader. The last two are
    // the ones no other case covers: an implicit `k: v` entry inside a flow
    // *sequence* enters through `parse_flow_key_scalar`, and the explicit
    // `? k : v` form through `parse_explicit_flow_unquoted_key`. Without them,
    // deleting either gate leaves this test green.
    for (name, input) in [
        ("seq item", "a: [!!str x]\n"),
        ("mapping value", "a: {k: !custom v}\n"),
        ("mapping key", "a: {!!str k: v}\n"),
        ("seq item with a plain sibling", "a: [!custom x, plain]\n"),
        ("implicit entry key in a seq", "a: [!!str k: v]\n"),
        ("explicit key", "a: [? !!str k : v]\n"),
    ] {
        let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".", input, &[])?;
        assert_eq!(exit_code, 1, "{name}: expected clean error exit: {stderr}");
        assert_eq!(stdout, "", "{name}: nothing should reach stdout");
        assert!(
            stderr.contains("tags (!) not supported"),
            "{name}: stderr should name the tag: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_yaml_bang_inside_a_plain_scalar_is_still_content() -> Result<()> {
    // The boundary the #369 fix must not cross. `!` is an indicator only at the
    // *start* of a node; inside plain scalar content it is ordinary text, in
    // both flow and block context. These passed before that fix and must keep
    // passing after it — if one of them starts erroring, the check moved from
    // "node begins with a tag" to "node contains a bang".
    for (name, input, expected) in [
        ("flow seq", "a: [x!y, a!b]\n", r#"{"a":["x!y","a!b"]}"#),
        ("block value", "a: hello!world\n", r#"{"a":"hello!world"}"#),
        ("flow value", "a: {k: v!w}\n", r#"{"a":{"k":"v!w"}}"#),
    ] {
        let (stdout, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I", "0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(stdout.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_quoted_flow_key_that_looks_like_a_tag_stays_a_string() -> Result<()> {
    // The other half of the #369 boundary, and the premise the gate placement
    // rests on: a quoted node cannot *begin* with `!`, so the quoted arms of the
    // flow key readers need no tag check and a `!` behind quotes is content.
    // Those arms had no test of their own — `parse_flow_key`'s single-quoted arm
    // and both of `parse_explicit_flow_key_scalar`'s were unexecuted lines.
    for (name, input, expected) in [
        ("double-quoted key", "a: {\"k\": v}\n", r#"{"a":{"k":"v"}}"#),
        ("single-quoted key", "a: {'k': v}\n", r#"{"a":{"k":"v"}}"#),
        (
            "double-quoted explicit key",
            "a: [? \"k\" : v]\n",
            r#"{"a":[{"k":"v"}]}"#,
        ),
        (
            "single-quoted explicit key",
            "a: [? 'k' : v]\n",
            r#"{"a":[{"k":"v"}]}"#,
        ),
        (
            "tag text behind quotes",
            "a: {\"!k\": v}\n",
            r#"{"a":{"!k":"v"}}"#,
        ),
        (
            "tag text behind quotes, explicit",
            "a: [? \"!!str k\" : v]\n",
            r#"{"a":[{"!!str k":"v"}]}"#,
        ),
    ] {
        let (stdout, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I", "0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(stdout.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_explicit_flow_key_scalar_shapes() -> Result<()> {
    // `parse_explicit_flow_unquoted_key` is where the #369 gate for the
    // `? k : v` form sits, and the gate was the only line in it this change
    // exercised: its terminator, internal-space, embedded-colon and
    // continuation-line branches had no test at all. Each expectation below was
    // taken from yq v4.53.3, so this pins agreement, not just current output.
    for (name, input, expected) in [
        (
            "comma ends the key",
            "a: [? k, v]\n",
            r#"{"a":[{"k":null},"v"]}"#,
        ),
        (
            "bracket ends the key",
            "a: [? k]\n",
            r#"{"a":[{"k":null}]}"#,
        ),
        (
            "space then comma",
            "a: [? k , v]\n",
            r#"{"a":[{"k":null},"v"]}"#,
        ),
        (
            "internal space",
            "a: [? a b : v]\n",
            r#"{"a":[{"a b":"v"}]}"#,
        ),
        (
            "internal double space",
            "a: [? a  b : v]\n",
            r#"{"a":[{"a  b":"v"}]}"#,
        ),
        (
            "embedded colon",
            "a: [? a:b : v]\n",
            r#"{"a":[{"a:b":"v"}]}"#,
        ),
        (
            "space then embedded colon",
            "a: [? a :b : v]\n",
            r#"{"a":[{"a :b":"v"}]}"#,
        ),
        (
            "trailing space",
            "a: [? a b  ]\n",
            r#"{"a":[{"a b":null}]}"#,
        ),
        (
            "trailing space before a break",
            "a: [? k \n  ]\n",
            r#"{"a":[{"k":null}]}"#,
        ),
        (
            "continued on the next line",
            "a: [? a b\n  c : v]\n",
            r#"{"a":[{"a b c":"v"}]}"#,
        ),
        (
            "value indicator on the next line",
            "a: [? k\n  : v]\n",
            r#"{"a":[{"k":"v"}]}"#,
        ),
        (
            "delimiter on the next line",
            "a: [? k\n  , x]\n",
            r#"{"a":[{"k":null},"x"]}"#,
        ),
    ] {
        let (stdout, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I", "0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(stdout.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_explicit_flow_key_running_off_the_end_is_rejected() -> Result<()> {
    // The same reader's EOF exits: the key scan reaches the end of input with
    // nothing closing the sequence. Both must fail rather than quietly return
    // the partial key, and `yq` rejects both too.
    for (name, input) in [
        ("no closing bracket", "a: [? k "),
        ("colon then end of input", "a: [? k :"),
    ] {
        let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".", input, &[])?;
        assert_eq!(exit_code, 1, "{name}: expected clean error exit: {stderr}");
        assert_eq!(stdout, "", "{name}: nothing should reach stdout");
        assert!(
            stderr.contains("unexpected end of input"),
            "{name}: stderr should name the truncation: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn test_yaml_anchored_seq_item_is_line_break_agnostic() -> Result<()> {
    // #328 and #324 have to compose: the anchor fix reads the item's value
    // through `at_line_end` and `looks_like_mapping_entry`, and the line-break
    // fix is what taught those two to stop at `\r`. Neither alone is enough —
    // this landed as a characterization test pinning the CRLF corruption while
    // #324 was open, and #324 turned it green.
    //
    // All three break forms must give the one answer yq gives, per YAML 1.2 §5.4.
    for (name, input) in [
        ("LF", "list:\n  - &m\n    k: v\n  - *m\n"),
        ("CRLF", "list:\r\n  - &m\r\n    k: v\r\n  - *m\r\n"),
        ("CR", "list:\r  - &m\r    k: v\r  - *m\r"),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name} input should parse");
        assert_eq!(
            output.trim(),
            r#"{"list":[{"k":"v"},{"k":"v"}]}"#,
            "{name} line breaks should give the same document"
        );
    }
    Ok(())
}

#[test]
fn test_yaml_anchor_on_flow_mapping_key_binds_to_key() -> Result<()> {
    // Found by the anchor-targets-an-open-bit invariant (corpus case CN3R): the
    // key's BP node is opened before the anchor is read, so recording the
    // *next* position bound the anchor to the value instead of the key.
    let input = "a: { &e e: f }\nb: *e\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"e":"f"},"b":"e"}"#);
    Ok(())
}

#[test]
fn test_yaml_anchor_on_null_explicit_value_resolves_to_null() -> Result<()> {
    // Also found by the invariant (corpus case PW8X): an anchor on an explicit
    // value that turns out to be null had no node to point at, so the alias
    // resolved to the following key.
    let input = "? e\n: &a\nz: *a\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"e":null,"z":null}"#);
    Ok(())
}

// =============================================================================
// Explicit keys as sequence items (#339) - `- ? k` / `  : v` is a mapping, not
// a plain scalar. Expectations are mikefarah/yq v4.53.3 output.
// =============================================================================

#[test]
fn test_yaml_explicit_key_seq_item_is_a_mapping() -> Result<()> {
    // The headline #339 repro: the item used to read as the plain scalar "? e",
    // and the `: v` line became a phantom second element — ["? e","v"].
    let input = "- ? e\n  : v\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"e":"v"}]"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_key_seq_item_without_value_is_null() -> Result<()> {
    // The second repro: `- ? e` alone. The pending key is closed with a null
    // when the item's mapping is popped, so the item is {"e":null}, not "? e".
    let input = "- ? e\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"e":null}]"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_key_seq_item_is_navigable() -> Result<()> {
    // Not just identity: the element must be a real mapping you can index, and
    // the sequence must have exactly one element rather than the old two.
    let input = "list:\n  - ? e\n    : v\n";
    let (output, exit_code) = run_yq_stdin(".list[0].e", input, &["-o=json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#""v""#);

    let (output, exit_code) = run_yq_stdin(".list | length", input, &["-o=json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "1", "the `: v` line must not add an element");
    Ok(())
}

#[test]
fn test_yaml_explicit_key_seq_item_shares_the_items_mapping() -> Result<()> {
    // Later entries at the mapping's indent join it rather than starting a new
    // element — the same reuse `parse_explicit_key` gives at mapping level.
    let input = "- ? e\n  : v\n  ? f\n  : w\n  g: h\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"e":"v","f":"w","g":"h"}]"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_key_seq_item_agrees_across_positions() -> Result<()> {
    // The fix is "route the item through the path that was already right", so
    // the three positions must agree. They diverged before: only the last one
    // produced a mapping.
    for (name, input, expected) in [
        ("top level", "? e\n: v\n", r#"{"e":"v"}"#),
        ("map value", "m:\n  ? e\n  : v\n", r#"{"m":{"e":"v"}}"#),
        ("seq item", "m:\n  - ? e\n    : v\n", r#"{"m":[{"e":"v"}]}"#),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name} should parse");
        assert_eq!(output.trim(), expected, "{name} mismatch");
    }
    Ok(())
}

#[test]
fn test_yaml_explicit_key_seq_item_is_line_break_agnostic() -> Result<()> {
    // Composes with #324: the new arm's guard admits `\n`, `\r` and EOI after
    // the `?`, so all three YAML 1.2 §5.4 break forms give the one document.
    for (name, input) in [
        ("LF", "- ? e\n  : v\n"),
        ("CRLF", "- ? e\r\n  : v\r\n"),
        ("CR", "- ? e\r  : v\r"),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name} input should parse");
        assert_eq!(
            output.trim(),
            r#"[{"e":"v"}]"#,
            "{name} line breaks should give the same document"
        );
    }
    Ok(())
}

#[test]
fn test_yaml_seq_item_question_mark_without_space_is_a_scalar() -> Result<()> {
    // `?` is only an indicator when followed by whitespace or EOI, so a plain
    // scalar that merely starts with `?` must not be dragged into the new arm.
    let input = "- ?foo\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"["?foo"]"#);
    Ok(())
}

// =============================================================================
// A non-scalar explicit key at ordinary mapping level (#172) - `? - a\n  - b\n:
// value` used to lose the whole entry (`{}`). Fixed as a side effect of #325
// (key-side parsing, via the same route as the #339 sequence-item fix) and
// #429/#346 (the mid-line-return fix that let the value survive in nested
// positions). Expectations are mikefarah/yq v4.53.3 output.
// =============================================================================

#[test]
fn test_yaml_explicit_non_scalar_key_headline_repro() -> Result<()> {
    // The original #172 repro: `{}` before the fix, both key and value lost.
    let input = "? - a\n  - b\n: value\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"":"value"}"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_non_scalar_key_keeps_its_siblings() -> Result<()> {
    // Entries before and after the explicit entry are unaffected.
    let input = "x: 1\n? - a\n  - b\n: value\ny: 2\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"x":1,"":"value","y":2}"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_non_scalar_key_two_entries_in_one_mapping() -> Result<()> {
    // Two non-scalar-keyed entries in one mapping - yq keeps both `""` entries,
    // same as the #346 same-line case's two-complex-keys pin.
    let input = "? - a\n: v1\n? - b\n: v2\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"":"v1","":"v2"}"#);
    Ok(())
}

// =============================================================================
// An explicit key and its `: ` on one line (#346) - `? k: v` makes the whole
// `k: v` a mapping used as the key, so the entry has a complex key (rendered
// `""`) and no value. Expectations are mikefarah/yq v4.53.3 output.
// =============================================================================

#[test]
fn test_yaml_explicit_key_same_line_is_a_complex_key() -> Result<()> {
    // The headline #346 repro. `parse_explicit_key` used to stop the key scalar
    // at the `: ` and return mid-line, reading this as the simple entry
    // {"k":"v"} - a well-formed, wrong document with no error raised.
    let input = "? k: v\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"":null}"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_key_same_line_agrees_across_positions() -> Result<()> {
    // The three rows from the issue. They diverged three different ways: the
    // mid-line return left the main loop re-deriving the line's indent as 0, so
    // top level kept the value while both nested spellings nulled it.
    for (name, input, expected) in [
        ("top level", "? k: v\n", r#"{"":null}"#),
        ("map value", "m:\n  ? k: v\n", r#"{"m":{"":null}}"#),
        ("seq item", "- ? k: v\n", r#"[{"":null}]"#),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name} should parse");
        assert_eq!(output.trim(), expected, "{name} mismatch");
    }
    Ok(())
}

#[test]
fn test_yaml_explicit_key_same_line_takes_its_value_from_the_next_line() -> Result<()> {
    // The complex key is still an ordinary explicit key: a following `: ` line
    // at the `?`'s column supplies its value.
    let input = "? k: v\n: w\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"":"w"}"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_key_same_line_ends_at_the_indicators_column() -> Result<()> {
    // The key mapping sits at the key content's column, so a line there joins the
    // key, while one back at the `?`'s column ends the key and is a sibling entry.
    let (output, exit_code) = run_yq_stdin(".", "? k: v\n  j: u\n", &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"":null}"#, "`  j: u` joins the key");

    let (output, exit_code) = run_yq_stdin(".", "? k: v\nj: u\n", &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output.trim(),
        r#"{"":null,"j":"u"}"#,
        "`j: u` is a sibling entry"
    );
    Ok(())
}

#[test]
fn test_yaml_explicit_value_same_line_is_a_mapping() -> Result<()> {
    // The value indicator had the identical mid-line defect, so the fix is
    // mirrored there. Corpus case V9D5 needs both arms at once.
    let (output, exit_code) = run_yq_stdin(".", "? a\n: b: c\n", &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"b":"c"}}"#);

    let input = "- sun: yellow\n- ? earth: blue\n  : moon: white\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output.trim(),
        r#"[{"sun":"yellow"},{"":{"moon":"white"}}]"#,
        "V9D5"
    );
    Ok(())
}

#[test]
fn test_yaml_explicit_key_same_line_is_navigable() -> Result<()> {
    // Not just identity: the entry must be a real one-entry mapping, and the
    // complex key must not leave a `k` field behind for queries to find.
    let input = "? k: v\n: w\n";
    let (output, exit_code) = run_yq_stdin("length", input, &["-o=json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "1", "one entry, not two");

    let (output, exit_code) = run_yq_stdin(".k", input, &["-o=json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "null", "`k` is inside the key, not a field");
    Ok(())
}

#[test]
fn test_yaml_explicit_key_same_line_is_line_break_agnostic() -> Result<()> {
    // Composes with #324: all three YAML 1.2 §5.4 break forms give one document.
    for (name, input) in [
        ("LF", "? k: v\n: w\n"),
        ("CRLF", "? k: v\r\n: w\r\n"),
        ("CR", "? k: v\r: w\r"),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name} input should parse");
        assert_eq!(
            output.trim(),
            r#"{"":"w"}"#,
            "{name} line breaks should give the same document"
        );
    }
    Ok(())
}

#[test]
fn test_yaml_explicit_key_colon_without_space_stays_key_text() -> Result<()> {
    // `:` is only a value indicator when followed by whitespace or EOI, so the
    // new arm must not claim a key that merely contains a colon.
    let input = "? k:v\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"k:v":null}"#);

    // ...nor the multi-line spelling, which was always correct.
    let (output, exit_code) = run_yq_stdin(".", "? k\n: v\n", &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"k":"v"}"#);
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

// ============================================================================
// Variable Tests - --arg / --argjson / $ARGS (jq-inherited extensions, #284)
// ============================================================================

#[test]
fn test_arg_string_variable() -> Result<()> {
    // --arg NAME VALUE binds $NAME to the string VALUE.
    let (output, code) = run_yq_stdin("$g", "a: 1", &["--arg", "g", "hello"])?;
    assert_eq!(output.trim(), "hello");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_argjson_variable() -> Result<()> {
    // --argjson NAME VALUE binds $NAME to the parsed JSON VALUE.
    let (output, code) = run_yq_stdin(
        ".n = $n",
        "{}",
        &["--argjson", "n", "42", "-o=json", "-I=0"],
    )?;
    assert_eq!(output.trim(), r#"{"n":42}"#);
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_multiple_variables() -> Result<()> {
    // Multiple --arg pairs all resolve.
    let (output, code) = run_yq_stdin("$a + $b", "a: 1", &["--arg", "a", "x", "--arg", "b", "y"])?;
    assert_eq!(output.trim(), "xy");
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_args_named_object() -> Result<()> {
    // $ARGS.named exposes all --arg/--argjson values (jq special variable).
    let (output, code) = run_yq_stdin(
        "$ARGS.named",
        "a: 1",
        &["--arg", "g", "hello", "-o=json", "-I=0"],
    )?;
    assert_eq!(output.trim(), r#"{"g":"hello"}"#);
    assert_eq!(code, 0);
    Ok(())
}

#[test]
fn test_argjson_invalid_value_errors() -> Result<()> {
    // Malformed --argjson is rejected (RFC 8259 strict), matching jq, with the
    // "invalid JSON for --argjson" context surfaced on stderr.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("$n", "a: 1", &["--argjson", "n", "not json"])?;
    assert!(stdout.is_empty(), "expected no stdout, got: {stdout:?}");
    assert_ne!(code, 0, "expected non-zero exit for invalid --argjson");
    assert!(
        stderr.contains("invalid JSON for --argjson"),
        "stderr missing context, got: {stderr:?}",
    );
    Ok(())
}

// ============================================================================
// Sequence-entry indicator in flow context (#332)
//
// These inputs are invalid YAML — `-` followed by whitespace is always the
// sequence-entry indicator, and no block sequence can start inside a flow
// collection — and real `yq` rejects every one of them. So they cannot be yq
// goldens: `scripts/sync-yq-golden.sh` has no `expected.out` to capture. The
// loader is deliberately lenient here (`succinctly yaml validate` is the layer
// that rejects them); what it must not do is silently discard the content,
// which is what it did before #332.
// ============================================================================

#[test]
fn test_flow_dash_space_scalar_content_is_not_dropped() -> Result<()> {
    for (yaml, expected) in [
        ("[- x]\n", r#"["- x"]"#),
        ("{a: - x}\n", r#"{"a":"- x"}"#),
        ("[- x, 1]\n", r#"["- x",1]"#),
        ("[a, - b]\n", r#"["a","- b"]"#),
        ("{a: [- x]}\n", r#"{"a":["- x"]}"#),
    ] {
        let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "for {yaml:?}");
        assert_eq!(output.trim(), expected, "for {yaml:?}");
    }
    Ok(())
}

#[test]
fn test_flow_dash_without_whitespace_is_still_a_scalar() -> Result<()> {
    // A `-` not followed by whitespace is a legitimate plain scalar in flow
    // context and was never affected; pinned so #332's fix cannot regress it.
    for (yaml, expected) in [
        ("[-]\n", r#"["-"]"#),
        ("{a: -}\n", r#"{"a":"-"}"#),
        ("[-1, -2]\n", "[-1,-2]"),
    ] {
        let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "for {yaml:?}");
        assert_eq!(output.trim(), expected, "for {yaml:?}");
    }
    Ok(())
}

#[test]
fn test_empty_block_sequence_items_remain_null() -> Result<()> {
    // The other shape that reaches the same branch. These are valid YAML and
    // must keep reading as null — `yq` agrees.
    for (yaml, expected) in [
        ("-\n", "[null]"),
        ("- # comment\n", "[null]"),
        ("a:\n  -\n  - y\n", r#"{"a":[null,"y"]}"#),
    ] {
        let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "for {yaml:?}");
        assert_eq!(output.trim(), expected, "for {yaml:?}");
    }
    Ok(())
}

#[test]
fn test_flow_dash_space_scalar_round_trips_through_yaml_output() -> Result<()> {
    // Default `-o yaml` must quote the scalar: emitted bare under a `- ` marker
    // it would read back as a nested sequence.
    let (yaml_out, code) = run_yq_stdin(".", "[- x]\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(yaml_out, "- \"- x\"\n");

    let (json_out, code) = run_yq_stdin(".", &yaml_out, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(json_out.trim(), r#"["- x"]"#);
    Ok(())
}

#[test]
fn test_flow_container_key_does_not_leak_its_closing_bracket() -> Result<()> {
    // `set_bp_text_end` writes the last slot *pushed*, so recording an end after
    // parsing a nested `[`/`{` landed it on the innermost node inside that
    // container and stretched its extent past the closing bracket (#332). One
    // case per site that used to do it; `at_offset` names the inner node, whose
    // decoded value carried the stray delimiter before the fix.
    //
    // Each `offset` below points at the last element inside the nested container:
    // the assertion fails with a trailing `]` or `}` if the clobber returns.
    for (yaml, offset, expected) in [
        // flow-mapping key via parse_flow_key, sequence — `e` of `[d, e]`, was "e]"
        ("{[d, e]: f}\n", 5, r#""e""#),
        // flow-mapping key via parse_flow_key, mapping — the `1` of `{a: 1}`
        ("{{a: 1}: 2}\n", 5, "1"),
        // explicit flow key, sequence — `b` of `[a, b]`
        ("{? [a, b]: 1}\n", 7, r#""b""#),
        // explicit flow key, mapping — the `1` of `{a: 1}`
        ("{? {a: 1}: 2}\n", 7, "1"),
        // explicit flow entry inside a sequence — `b` of `[a, b]`
        ("[? [a, b] : 1]\n", 7, r#""b""#),
        // implicit flow-mapping-entry key — `b` of `[a, b]`
        ("[[a, b]: 1]\n", 5, r#""b""#),
        // implicit flow-mapping-entry value — `c` of `[b, c]`
        ("[a: [b, c]]\n", 8, r#""c""#),
    ] {
        let (output, code) =
            run_yq_stdin(&format!("at_offset({offset})"), yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "for {yaml:?}");
        assert_eq!(output.trim(), expected, "for {yaml:?} at offset {offset}");
    }
    Ok(())
}

#[test]
fn test_alias_as_a_flow_mapping_key_keeps_its_own_extent() -> Result<()> {
    // The flow-mapping key site records the key's end itself, so the alias node
    // *is* the key node and its extent must cover exactly `*x` for the alias to
    // resolve. It used to reach this through `parse_alias`, which opened a second
    // node inside the already-open key and carried the end on that one — which is
    // why the site had to skip recording an end at all (#332), and why the key
    // itself had none and rendered as `""` (#405).
    let yaml = "x: &x 1\ny: {*x: 2}\n";
    let (output, code) = run_yq_stdin("at_offset(13)", yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "1");

    // And the document as a whole still reads back correctly. `yq` v4.53.3 agrees.
    let (whole, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(whole.trim(), r#"{"x":1,"y":{"1":2}}"#);
    Ok(())
}

#[test]
fn test_alias_as_a_flow_mapping_key_resolves() -> Result<()> {
    // #405. An alias used as a flow-mapping key resolved internally but rendered
    // as the empty string: the caller had already opened the key's BP node, and
    // `parse_alias` opened another one inside it and bound the alias edge to
    // *that*, so the key node itself had neither an extent nor an edge. The three
    // other key positions already went through `record_key_alias`; this one now
    // does too.
    //
    // Every expectation is `yq` v4.53.3's output for the same input, on the
    // streaming path (`-I 0`). The pretty path is deliberately not asserted here:
    // its DOM collapses duplicate mapping keys (#174), which resolution makes
    // reachable from more inputs but does not cause.
    for (yaml, expected) in [
        // Alias to an anchored key, and to an anchored value — different targets.
        ("{&x k: 1, *x: 2}\n", r#"{"k":1,"k":2}"#),
        ("{k: &x 1, *x: 2}\n", r#"{"k":1,"1":2}"#),
        // Flow mapping nested in a block mapping, and two sibling flow mappings
        // inside a flow sequence: the anchor and the alias in separate entries.
        ("a: {&x k: 1, *x: 2}\n", r#"{"a":{"k":1,"k":2}}"#),
        ("[{&x k: 1}, {*x: 2}]\n", r#"[{"k":1},{"k":2}]"#),
        // A space before the `:` must not widen the key's extent past the name.
        ("{&x k: 1, *x : 2}\n", r#"{"k":1,"k":2}"#),
        // Alias key whose value is an implicit null.
        ("{&x k: 1, *x}\n", r#"{"k":1,"k":null}"#),
        // An alias key resolving to a *complex* (sequence or mapping) node still
        // stringifies as `""`, which is what it did before this fix and what `yq`
        // does — the empty key here is the complex-key rule, not the bug.
        ("{&x [1,2]: 3, *x: 4}\n", r#"{"":3,"":4}"#),
        ("{a: &x {p: 1}, *x: 2}\n", r#"{"a":{"p":1},"":2}"#),
    ] {
        let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "for {yaml:?}");
        assert_eq!(output.trim(), expected, "for {yaml:?}");
    }

    // An anchor and an alias to it on the *same* node: the alias edge now lands on
    // the node the anchor names, so this is a cycle by the `target == alias` arm of
    // `validate_alias_acyclicity` rather than by its ancestor arm. It must stay an
    // error — resolving it would be unbounded materialization.
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", "{&x *x: 1}\n", &["-o=json"])?;
    assert_eq!(code, 1, "expected clean error exit, stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("cyclic alias 'x'"),
        "stderr should name the cycle: {stderr}"
    );
    Ok(())
}

#[test]
fn test_computed_key_in_index_brackets() -> Result<()> {
    // The yq runner walks the parsed program to decide whether it can stream,
    // and that walk has to descend into both halves of a computed index (#360).
    // A key it fails to look inside would be scanned as an opaque leaf, so a
    // `split_doc` hiding there would pick the wrong output path.
    let yaml = "a: 1\nb: 2\nk: a\n";

    // Key outer, target inner — one output per key, in key order.
    let (output, code) = run_yq_stdin(r#".[("a","b")]"#, yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "1\n2\n");

    // A key read out of the document itself.
    let (output, code) = run_yq_stdin(".[.k]", yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "1\n");

    // A missing key is null, not an error, exactly as `.missing` is.
    let (output, code) = run_yq_stdin(r#".[("nope")]"#, yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "null\n");
    Ok(())
}

// =============================================================================
// Whole-float representation — #169
// =============================================================================

/// A scalar resolved as `!!float` must never print as an integer, whatever
/// spelling it had in the source. Each expectation was measured against
/// `yq` v4.53.3 with `yq -o=json -I=0`.
#[test]
fn test_whole_floats_keep_their_decimal_point() -> Result<()> {
    for (scalar, want) in [
        ("1.0", "1.0"),
        ("2.0", "2.0"),
        ("0.0", "0.0"),
        ("-0.0", "-0.0"),
        ("1.", "1.0"),
        ("-5.0", "-5.0"),
        // Above i64::MAX, where the old guard gave up and dropped the `.0`.
        ("12345678901234567890123", "12345678901234568000000.0"),
        // Genuine integers stay integers.
        ("42", "42"),
        ("-5", "-5"),
        ("0x2A", "42"),
    ] {
        let yaml = format!("x: {scalar}\n");
        let (out, code) = run_yq_stdin(".", &yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "exit code for {scalar:?}");
        assert_eq!(out.trim(), format!("{{\"x\":{want}}}"), "for {scalar:?}");
    }
    Ok(())
}

/// The compact and pretty printers must agree on float representation; before
/// the fix the compact path collapsed `1.0` while the pretty path did not.
#[test]
fn test_compact_and_pretty_agree_on_whole_floats() -> Result<()> {
    let yaml = "a: 1.0\nb: 0.0\nc: -0.0\nd: 2.5\ne: 42\n";

    let (compact, compact_code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;
    let (pretty, pretty_code) = run_yq_stdin(".", yaml, &["-o=json"])?;

    assert_eq!(compact_code, 0);
    assert_eq!(pretty_code, 0);
    assert_eq!(
        compact.trim(),
        r#"{"a":1.0,"b":0.0,"c":-0.0,"d":2.5,"e":42}"#
    );
    assert_eq!(compact.trim(), pretty.replace([' ', '\n'], ""));
    Ok(())
}

/// Non-identity navigation (`.field`, `.[0]`, `.[]`, chained) takes the M2
/// streaming fast path in compact mode, a different writer
/// (`stream_owned_value_json_with` in `src/jq/stream.rs`) than the `.`
/// identity path fixed first. It must agree with `.` and with real `yq`
/// v4.53.3 rather than collapsing whole floats back to integers.
#[test]
fn test_navigation_queries_keep_whole_float_decimal_point() -> Result<()> {
    for (filter, yaml, want) in [
        (".x", "x: 1.0\n", "1.0"),
        (".x[0]", "x: [1.0, 2.0]\n", "1.0"),
        (".x[]", "x: [1.0, 2.0]\n", "1.0\n2.0"),
        (".x.y[1]", "x:\n  y: [1.0, 2.0]\n", "2.0"),
    ] {
        let (out, code) = run_yq_stdin(filter, yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "exit code for {filter:?}");
        assert_eq!(out.trim(), want, "for {filter:?} over {yaml:?}");
    }
    Ok(())
}

/// The same navigation queries must also agree in compact YAML output
/// (`-o=yaml -I=0`), which streams through a sibling writer
/// (`stream_owned_value_yaml`) that had the identical bug.
#[test]
fn test_navigation_queries_keep_whole_float_decimal_point_yaml() -> Result<()> {
    let (out, code) = run_yq_stdin(".x", "x: 1.0\n", &["-o=yaml", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "1.0");
    Ok(())
}
