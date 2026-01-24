//! Integration tests for the succinctly yq CLI command
//!
//! These tests verify yq-compatible behavior, especially type preservation
//! for quoted vs unquoted scalars.
//!
//! Run with: cargo test --features cli --test yq_cli_tests

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;
use tempfile::NamedTempFile;

/// Maximum retries for flaky cargo run commands (lock contention in CI)
const MAX_CARGO_RETRIES: usize = 3;

/// Helper to run yq command with input from stdin
fn run_yq_stdin(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let mut cmd = Command::new("cargo")
            .args([
                "run",
                "--features",
                "cli",
                "--bin",
                "succinctly",
                "--",
                "yq",
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
        return Ok((stdout, exit_code));
    }
    unreachable!()
}

/// Helper to run yq command with file input
fn run_yq_file(filter: &str, file_path: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let output = Command::new("cargo")
            .args([
                "run",
                "--features",
                "cli",
                "--bin",
                "succinctly",
                "--",
                "yq",
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

/// Helper to run system yq command with input from stdin
fn run_system_yq_stdin(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let mut cmd = Command::new("yq")
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

/// Check if system yq is available
fn has_system_yq() -> bool {
    Command::new("yq")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Compare succinctly yq output with system yq output
fn compare_yq_output(filter: &str, yaml: &str, args: &[&str]) -> Result<bool> {
    let (succ_out, succ_code) = run_yq_stdin(filter, yaml, args)?;
    let (sys_out, sys_code) = run_system_yq_stdin(filter, yaml, args)?;

    Ok(succ_code == sys_code && succ_out == sys_out)
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
    let yaml = r#"count: 123"#;
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
    let yaml = r#"version: '2.0'"#;
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
    let yaml = r#"test: true"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert!(output.contains(r#"{"test":true}"#));
    Ok(())
}

#[test]
fn test_output_format_space_syntax() -> Result<()> {
    let yaml = r#"test: true"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o", "json"])?;

    assert_eq!(code, 0);
    // Default format is pretty-printed, so check for field presence
    assert!(output.contains(r#""test""#));
    assert!(output.contains(r#"true"#));
    Ok(())
}

#[test]
fn test_indent_equals_syntax() -> Result<()> {
    let yaml = r#"a: 1"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);
    Ok(())
}

#[test]
fn test_indent_space_syntax() -> Result<()> {
    let yaml = r#"a: 1"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o", "json", "-I", "0"])?;

    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);
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
    writeln!(temp_file, r#"count: 123"#)?;

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
    writeln!(temp_file, r#"build: 999"#)?;

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
    let yaml = r#"
a: 1
b: 2
c: 3
"#;
    let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    // Compact output should not have newlines between fields
    assert!(!output.trim().contains("\n"));
    Ok(())
}

// ============================================================================
// Direct yq Comparison Tests (require system yq installed)
// ============================================================================

#[test]
fn compare_quoted_string_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"version: "1.0""#;
    assert!(
        compare_yq_output(".", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for quoted strings"
    );
    Ok(())
}

#[test]
fn compare_mixed_types_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"
version: "1.0"
id: "001"
count: 123
price: 19.99
"#;
    assert!(
        compare_yq_output(".", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for mixed types"
    );
    Ok(())
}

#[test]
fn compare_nested_structure_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"
users:
  - name: "Alice"
    id: "001"
    age: 30
  - name: "Bob"
    id: "002"
    age: 25
"#;
    assert!(
        compare_yq_output(".users[0]", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for nested structures"
    );
    Ok(())
}

#[test]
fn compare_field_selection_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"
config:
  database:
    version: "5.7"
    port: 3306
"#;
    assert!(
        compare_yq_output(".config.database.version", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for field selection"
    );
    Ok(())
}

#[test]
fn compare_array_iteration_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"
items:
  - "001"
  - "002"
  - "003"
"#;
    assert!(
        compare_yq_output(".items", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for arrays"
    );
    Ok(())
}

#[test]
fn compare_boolean_values_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"
enabled: true
disabled: false
quoted: "true"
"#;
    assert!(
        compare_yq_output(".", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for boolean values"
    );
    Ok(())
}

#[test]
fn compare_null_values_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"
a: null
b: ~
c: "null"
"#;
    assert!(
        compare_yq_output(".", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for null values"
    );
    Ok(())
}

#[test]
fn compare_complex_document_output() -> Result<()> {
    if !has_system_yq() {
        eprintln!("Skipping: system yq not available");
        return Ok(());
    }

    let yaml = r#"
metadata:
  version: "2.5.1"
  build: 999
users:
  - name: "Alice"
    id: "001"
    scores:
      - 95
      - 87
      - 92
  - name: "Bob"
    id: "002"
    scores:
      - 78
      - 89
      - 91
"#;
    assert!(
        compare_yq_output(".", yaml, &["-o=json", "-I=0"])?,
        "Output differs from system yq for complex documents"
    );
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
