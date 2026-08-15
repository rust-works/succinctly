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
use tempfile::{NamedTempFile, TempDir};

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
fn test_select_after_iterate_stays_many_cursor() -> Result<()> {
    // `select` isn't a "navigation-only" expression, so `.[] | select(...)`
    // takes the DOM/cursor evaluation path (`evaluate_yaml_cursor`) instead
    // of the M2 streaming fast path. When every filtered element keeps its
    // position, the pipe's result stays a top-level `ManyCursor`, exercising
    // that arm of `evaluate_yaml_cursor` directly (as opposed to a plain
    // `.[]`, which the M2 fast path intercepts before it ever reaches here).
    let (out, code) = run_yq_stdin(".[] | select(. > 0)", "- 2\n- 3\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "2\n3");
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

/// #707: `yq '.'` on flow-styled input must preserve flow style, matching
/// real yq. Before the fix, every container was forced to block style on
/// every query, even a pure identity pass-through.
#[test]
fn test_identity_preserves_top_level_flow_style() -> Result<()> {
    let yaml = "a: [1, 2, 3]\nb: {c: 1, d: 2}\n";
    let (output, code) = run_yq_stdin(".", yaml, &[])?;

    assert_eq!(code, 0);
    assert_eq!(output, "a: [1, 2, 3]\nb: {c: 1, d: 2}\n");
    Ok(())
}

/// #707: flow style nested under a block sequence must stay flow and inline
/// (`- [1, 2]`), not get exploded onto its own indented block lines.
#[test]
fn test_identity_preserves_flow_nested_under_block_sequence() -> Result<()> {
    let yaml = "a:\n  - [1, 2]\n  - 3\n";
    let (output, code) = run_yq_stdin(".", yaml, &[])?;

    assert_eq!(code, 0);
    assert_eq!(output, "a:\n  - [1, 2]\n  - 3\n");
    Ok(())
}

/// #707: flow style nested under a block mapping key must stay flow and
/// inline (`b: {x: 1, y: 2}`), not get exploded onto its own block lines.
#[test]
fn test_identity_preserves_flow_nested_under_block_mapping() -> Result<()> {
    let yaml = "a:\n  b: {x: 1, y: 2}\n  c: 3\n";
    let (output, code) = run_yq_stdin(".", yaml, &[])?;

    assert_eq!(code, 0);
    assert_eq!(output, "a:\n  b: {x: 1, y: 2}\n  c: 3\n");
    Ok(())
}

/// #707's second repro: writing to one element of a flow-style array must
/// not reformat the array itself to block style. The `_739` tests above
/// cover an untouched *sibling* keeping its style, and a directly-written
/// *scalar* keeping its own; this is the container analog of the latter --
/// the array's own node is what's mutated (`reconcile_presentation`'s
/// `Array` arm), not just a value inside it. Verified against real yq:
/// `a: [1, 2, 3]` + `.a[0] = 9` -> `a: [9, 2, 3]`.
#[test]
fn test_assign_to_flow_array_element_preserves_the_arrays_own_flow_style_707() -> Result<()> {
    let (output, code) = run_yq_stdin(".a[0] = 9", "a: [1, 2, 3]\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: [9, 2, 3]\n");
    Ok(())
}

/// #707: the fix applies to any pure-navigation query, not just the bare
/// identity `.` P9 fast path — `.top` still routes through the same
/// cursor-streaming `stream_yaml_value` and must preserve style too.
#[test]
fn test_field_navigation_preserves_flow_style() -> Result<()> {
    let yaml = "top:\n  a: [1, 2, 3]\n";
    let (output, code) = run_yq_stdin(".top", yaml, &[])?;

    assert_eq!(code, 0);
    assert_eq!(output, "a: [1, 2, 3]\n");
    Ok(())
}

/// #739 (ADR-0017): a write to one field must not reformat an unrelated,
/// untouched sibling — the issue's own repro. Every case verified against
/// the pinned real `yq` binary.
#[test]
fn test_assign_preserves_untouched_sibling_single_quote_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b = 2", "a: 'single'\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 'single'\nb: 2\n");
    Ok(())
}

#[test]
fn test_assign_preserves_untouched_sibling_double_quote_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b = 2", "a: \"double\"\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: \"double\"\nb: 2\n");
    Ok(())
}

#[test]
fn test_assign_preserves_untouched_sibling_flow_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b = 2", "a: {x: 1, y: 2}\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: {x: 1, y: 2}\nb: 2\n");
    Ok(())
}

#[test]
fn test_del_preserves_untouched_sibling_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin("del(.c)", "a: 'single'\nb: 1\nc: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 'single'\nb: 1\n");
    Ok(())
}

#[test]
fn test_compound_assign_preserves_untouched_sibling_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b += 1", "a: 'single'\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 'single'\nb: 2\n");
    Ok(())
}

/// #739: a *written* scalar field keeps its own quote style too, as long
/// as the new value is still a scalar (real `yq`'s in-place node-mutation
/// model updates the value but never touches the node's own style) — not
/// just untouched siblings. A kind change (scalar to container) does drop
/// it, since there's no such node to keep.
#[test]
fn test_assign_new_string_value_keeps_written_fields_own_quote_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".a = \"new\"", "a: 'single'\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 'new'\n");
    Ok(())
}

#[test]
fn test_assign_kind_change_drops_the_written_fields_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".a = {\"x\": 1}", "a: 'single'\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a:\n  x: 1\n");
    Ok(())
}

/// #739/#705: `-P` still forces block/plain style unconditionally, even
/// though the DOM path now tracks real style data for writes too.
#[test]
fn test_pretty_print_still_forces_block_style_on_a_write_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b = 2", "a: {x: 1, y: 2}\nb: 1\n", &["-P"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a:\n  x: 1\n  y: 2\nb: 2\n");
    Ok(())
}

/// #739 review finding: `reconcile_presentation` used to copy a key's
/// stale "deferred value materialized as nothing" flag (#765) forward
/// from the pristine document verbatim, even once a write gave that key a
/// real value — `key_comment_if_value_absent`'s consumer in
/// `emit_yaml_value` then rendered only the key and its comment, silently
/// dropping the written value entirely.
#[test]
fn test_assign_to_a_key_with_a_deferred_value_comment_keeps_the_written_value_739() -> Result<()> {
    let (output, code) = run_yq_stdin(
        ".version = \"1.0.0\"",
        "version: # TODO fill in\nauthor: me\n",
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output, "version: 1.0.0 # TODO fill in\nauthor: me\n");
    Ok(())
}

/// #739 review finding: an untouched double-quoted *empty* string sibling
/// used to always flip to single-quote style (`yaml_quote_string_with_style`'s
/// empty-string short-circuit ran before consulting `style` at all).
#[test]
fn test_assign_preserves_untouched_empty_string_double_quote_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b = 2", "a: \"\"\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: \"\"\nb: 2\n");
    Ok(())
}

/// #739: a brand-new field (no pristine counterpart at that key) gets
/// fresh, empty metadata in `reconcile_presentation`'s `Object` arm,
/// while its untouched sibling keeps its own style.
#[test]
fn test_assign_adds_a_new_field_with_no_pristine_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".c = 3", "a: 'single'\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 'single'\nb: 1\nc: 3\n");
    Ok(())
}

/// #739: an untouched double-quoted sibling containing every character
/// `yaml_double_quote_escaped` escapes must round-trip through the
/// style-forced "double" path, not just the default heuristic path.
#[test]
fn test_assign_preserves_double_quote_escaping_on_untouched_sibling_739() -> Result<()> {
    let (output, code) = run_yq_stdin(
        ".b = 2",
        "a: \"line1\\nline2\\ttab\\\\back\\\"quote\"\nb: 1\n",
        &[],
    )?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "a: \"line1\\nline2\\ttab\\\\back\\\"quote\"\nb: 2\n"
    );
    Ok(())
}

/// #739: `yaml_double_quote_escaped`'s carriage-return and generic
/// ascii-control-character arms specifically (`\n`/`\t` are covered by
/// the sibling test above).
#[test]
fn test_assign_preserves_double_quote_escaping_of_carriage_return_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b = 2", "a: \"cr\\rhere\"\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: \"cr\\rhere\"\nb: 2\n");
    Ok(())
}

/// #739: `yaml_single_quote_escaped` doubles an embedded single quote per
/// YAML's own escaping rule.
#[test]
fn test_assign_preserves_single_quote_escaping_of_embedded_quote_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".b = 2", "a: 'it''s'\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 'it''s'\nb: 2\n");
    Ok(())
}

/// #852, exercised through a write this time: a write followed by
/// navigating to a scalar result must still drop that scalar's own
/// styling at the top level, even though the DOM path now tracks style
/// data for writes (#739).
#[test]
fn test_write_then_navigate_to_scalar_still_drops_root_style_739() -> Result<()> {
    let (output, code) = run_yq_stdin(".a = 5 | .a", "a: 'single'\nb: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "5\n");
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

/// Field-lookup semantics (`.a` above, last-wins) are distinct from
/// output/serialization semantics: on identity pass-through, real yq keeps
/// *both* duplicate keys, in every output mode. Before #442, succinctly's
/// pretty (indented) output silently dropped the earlier occurrence via an
/// `IndexMap`-backed DOM, while compact (`-I0`) streamed straight from the
/// cursor and kept both. The M2 fast path now covers pretty output too, so
/// all four combinations agree with real yq.
#[test]
fn test_duplicate_mapping_key_survives_json_output() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (pretty, code) = run_yq_stdin(".", yaml, &["-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(pretty, "{\n  \"a\": 1,\n  \"a\": 2\n}\n");

    let (compact, code) = run_yq_stdin(".", yaml, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(compact, "{\"a\":1,\"a\":2}\n");

    Ok(())
}

/// Same as [`test_duplicate_mapping_key_survives_json_output`], but for the
/// default YAML output format — plain `yq '.'` with no flags shares the
/// same materialize-into-`OwnedValue::Object` code path as `-o json`, so it
/// had the identical pre-#442 bug (undocumented by the issue's own repro,
/// which only showed `-o json`).
#[test]
fn test_duplicate_mapping_key_survives_yaml_output() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (pretty, code) = run_yq_stdin(".", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(pretty, "a: 1\na: 2\n");

    let (compact, code) = run_yq_stdin(".", yaml, &["-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(compact, "a: 1\na: 2\n");

    Ok(())
}

/// `(.)` is semantically identical to `.`, but before the fix for #614 it
/// took a different code path: `can_use_m2_streaming` treated
/// `Expr::Paren(Expr::Identity)` as streamable, yet the stricter
/// `is_identity` check that unlocks direct cursor streaming required a bare
/// `Expr::Identity`, so `(.)` fell through to `eval_generic::eval_single`'s
/// `to_owned()` bridge and silently collapsed duplicate keys instead.
#[test]
fn test_duplicate_mapping_key_survives_parenthesized_identity() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (pretty, code) = run_yq_stdin("(.)", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(pretty, "a: 1\na: 2\n");

    let (compact, code) = run_yq_stdin("(.)", yaml, &["-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(compact, "a: 1\na: 2\n");

    let (json_pretty, code) = run_yq_stdin("(.)", yaml, &["-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(json_pretty, "{\n  \"a\": 1,\n  \"a\": 2\n}\n");

    let (json_compact, code) = run_yq_stdin("(.)", yaml, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(json_compact, "{\"a\":1,\"a\":2}\n");

    Ok(())
}

#[test]
fn test_duplicate_mapping_key_to_entries_preserves_both() -> Result<()> {
    // Unlike `.a` field access (last-wins, #174), `to_entries` must pass
    // every occurrence of a duplicate key through unmerged, matching real
    // `yq` (issue #443).
    let yaml = "a: 1\na: 2\n";
    let (output, code) = run_yq_stdin("to_entries", yaml, &[])?;

    assert_eq!(code, 0);
    // Each element renders in real yq's "compact" form (#785): `- ` shares
    // its line with the mapping's own first field.
    assert_eq!(output, "- key: a\n  value: 1\n- key: a\n  value: 2\n");
    Ok(())
}

#[test]
fn test_duplicate_mapping_key_to_entries_json_compact() -> Result<()> {
    // Same as above, exercising the exact `-o=json` repro from issue #443.
    let yaml = "a: 1\na: 2\n";
    let (output, code) = run_yq_stdin("to_entries", yaml, &["-o=json", "-I=0"])?;

    assert_eq!(code, 0);
    assert_eq!(
        output.trim(),
        r#"[{"key":"a","value":1},{"key":"a","value":2}]"#
    );
    Ok(())
}

/// #478: `--slurp '.'` shares the same `IndexMap`-backed conversion
/// (`yaml_to_owned_value`) #442 didn't touch, so it kept collapsing
/// duplicate keys within each slurped element even after plain `yq '.'`
/// was fixed. Must now match [`test_duplicate_mapping_key_survives_yaml_output`]
/// on the same input, just wrapped in the slurped array.
#[test]
fn test_duplicate_mapping_key_survives_slurp() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (pretty, code) = run_yq_stdin(".", yaml, &["--slurp"])?;
    assert_eq!(code, 0);
    // Renders in real yq's "compact" form (#785): `- ` shares its line
    // with the mapping's own first field.
    assert_eq!(pretty, "- a: 1\n  a: 2\n");

    let (compact, code) = run_yq_stdin(".", yaml, &["--slurp", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(compact, "- a: 1\n  a: 2\n");

    Ok(())
}

/// #478: like [`test_duplicate_mapping_key_survives_slurp`], `--slurp`
/// combining documents from multiple sources into one array must preserve
/// duplicate keys within each source, not just across sources.
#[test]
fn test_duplicate_mapping_key_survives_slurp_multiple_sources() -> Result<()> {
    let mut file_a = NamedTempFile::new()?;
    writeln!(file_a, "a: 1\na: 2")?;
    let mut file_b = NamedTempFile::new()?;
    writeln!(file_b, "b: 3")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("--slurp")
        .arg(".")
        .arg(file_a.path())
        .arg(file_b.path())
        .stdin(Stdio::null())
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;

    assert!(output.status.success());
    // Renders in real yq's "compact" form (#785): `- ` shares its line
    // with the mapping's own first field.
    assert_eq!(stdout, "- a: 1\n  a: 2\n- b: 3\n");
    Ok(())
}

/// #478: `can_slurp_fast_path` tracks `-e`/`--exit-status` by inspecting the
/// built cursor list directly (`any_truthy = true` whenever `--slurp`
/// produces its one array result), a separate code path from the non-slurp
/// M2 fast path's per-cursor `is_falsy()` check. Exercise it directly so
/// that branch runs at least once.
#[test]
fn test_slurp_exit_status_fast_path() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "a: 1\n", &["--slurp", "-e"])?;
    assert_eq!(code, 0);
    // Renders in real yq's "compact" form (#785): `- ` shares its line
    // with the mapping's own first field.
    assert_eq!(output, "- a: 1\n");
    Ok(())
}

/// #478: when `--doc N` filters out every input document, `can_slurp_fast_path`
/// builds an empty cursor list and streams it via `stream_yaml_sequence`,
/// whose empty-iterator early return (`"[]"`) is otherwise never exercised
/// by the duplicate-key-preservation tests above (they always match at
/// least one document).
#[test]
fn test_slurp_doc_filter_no_match_yields_empty_array() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "a: 1\n", &["--slurp", "--doc", "5"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[]\n");
    Ok(())
}

/// #835: a block-sequence item whose value is a non-empty mapping, written
/// in the source as a totally bare `-` on its own line followed by the
/// indented mapping (rather than the compact `- key: value` form), used to
/// re-serialize as a lone `-` with the whole mapping silently dropped.
///
/// Root cause: `YamlElements::uncons_cursor` deliberately leaves a bare `-`
/// item pointed at its sequence-item *wrapper* node rather than unwrapping
/// it to the deferred value (`corpus_stats` needs the wrapper positionally).
/// `is_yaml_cursor_container` and `stream_yaml_value`'s `Mapping` arm both
/// then read `is_container()`/`first_child()` off the wrapper itself, which
/// never carries a TY bit and has exactly one child - the mapping - so
/// `first_child()` returned the mapping node in place of its first field,
/// producing a mapping with a "field" whose key has no sibling and
/// collapsing to zero rendered fields. Matches real `yq` v4.53.3, which
/// re-serializes this into the same "compact" form #785 uses for every
/// other non-empty container element, regardless of the source's own style.
#[test]
fn test_bare_dash_alone_mapping_value_not_truncated_835() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "-\n  a: 1\n  b: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- a: 1\n  b: 2\n");
    Ok(())
}

/// #835: same bug, but for a sequence-valued (rather than mapping-valued)
/// bare-dash item - `stream_yaml_value`'s `Sequence` arm doesn't share the
/// `Mapping` arm's raw-field-walk optimization, so this shape wasn't
/// actually truncated pre-fix, but it exercises the same
/// `is_yaml_cursor_container` misclassification that picked the wrong
/// (deferred, non-"compact") render branch. Pinned here alongside the
/// mapping case so both container kinds stay covered together.
#[test]
fn test_bare_dash_alone_sequence_value_renders_compact_835() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "-\n  - x\n  - y\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- - x\n  - y\n");
    Ok(())
}

/// #835: the fix must not regress `-o json`, which was already correct
/// pre-fix (`yaml_to_owned_value` resolves through `YamlCursor::value`'s own
/// delegation rather than re-deriving `first_child()` off the wrapper).
#[test]
fn test_bare_dash_alone_mapping_value_json_output_835() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "-\n  a: 1\n  b: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[{\"a\":1,\"b\":2}]\n");
    Ok(())
}

/// #835: a sequence mixing all three item styles - compact (`- key: value`),
/// bare-dash-deferred, and a plain scalar - in one document, matching real
/// `yq` v4.53.3. Guards against a fix that only special-cases a
/// single-item sequence.
#[test]
fn test_bare_dash_alone_mapping_value_mixed_with_compact_and_scalar_835() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "- x: 1\n-\n  y: 2\n  z: 3\n- 5\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- x: 1\n- y: 2\n  z: 3\n- 5\n");
    Ok(())
}

/// #835: the `--slurp` fast path (`stream_yaml_sequence`) shares
/// `is_yaml_cursor_container` with `stream_yaml_value`'s `Sequence` arm, so
/// covers it independently of `stream_yaml_document`'s own entry point.
#[test]
fn test_bare_dash_alone_mapping_value_survives_slurp_835() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "-\n  a: 1\n  b: 2\n", &["--slurp"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- - a: 1\n    b: 2\n");
    Ok(())
}

/// #847: a block-sequence item whose value is a *flow*-style sequence,
/// written as a bare `-` on its own line followed by the indented flow
/// value on the next line, must preserve the source's flow style rather
/// than re-serializing in block style. Distinct from the #835 tests above,
/// which all use block-style sources for the deferred value. Matches real
/// `yq` v4.53.3.
#[test]
fn test_bare_dash_alone_flow_sequence_value_preserves_flow_style_847() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "-\n  [1, 2]\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- [1, 2]\n");
    Ok(())
}

/// #847: same as above, for a flow-style *mapping* value. Matches real `yq`
/// v4.53.3.
#[test]
fn test_bare_dash_alone_flow_mapping_value_preserves_flow_style_847() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "-\n  {a: 1, b: 2}\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- {a: 1, b: 2}\n");
    Ok(())
}

/// #847: the `--slurp` fast path (`stream_yaml_sequence`) is a separate code
/// path from `stream_yaml_document`'s `Sequence` arm exercised by the tests
/// above - mirrors the #835 slurp test's rationale for the same reason.
/// Matches real `yq` v4.53.3 (`eval-all '[.]' -`).
#[test]
fn test_bare_dash_alone_flow_sequence_value_preserves_flow_style_survives_slurp_847() -> Result<()>
{
    let (output, code) = run_yq_stdin(".", "-\n  [1, 2]\n", &["--slurp"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- - [1, 2]\n");
    Ok(())
}

/// #847: same as above, for a flow-style *mapping* value under `--slurp`.
/// Matches real `yq` v4.53.3.
#[test]
fn test_bare_dash_alone_flow_mapping_value_preserves_flow_style_survives_slurp_847() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "-\n  {a: 1, b: 2}\n", &["--slurp"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- - {a: 1, b: 2}\n");
    Ok(())
}

/// #478: `stream_yaml_sequence`'s block-style rendering has a container vs.
/// scalar branch per slurped item (mirroring `stream_yaml_value`'s `Sequence`
/// arm); the tests above only slurp mapping documents, which always take the
/// container branch. Bare scalar documents take the `"- "` scalar branch
/// instead.
#[test]
fn test_slurp_scalar_documents_use_block_style_dash_items() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "a\n---\nb\n", &["--slurp"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "- a\n- b\n");
    Ok(())
}

/// #478: `--inplace '.'` went through the same lossy `yaml_to_owned_value`
/// path as `--slurp`, unlike plain `yq '.'` which #442 already fixed.
/// Real `yq --inplace` v4.53.3 keeps both `a:` entries on this input.
#[test]
fn test_duplicate_mapping_key_survives_inplace() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\na: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg(".")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "a: 1\na: 2\n");
    Ok(())
}

/// #631: `first(.[])`/`last(.[])` (the `Expr::FirstExpr`/`LastExpr` one-arg
/// stream form `first(f)`/`last(f)` compiles to) fell through
/// `evaluate_yaml_cursor`'s unconditional `to_owned()` DOM path, unlike
/// `.[0]` which #442 already routed through the M2 cursor-streaming fast
/// path. `eval_generic.rs` itself threads `GenericResult::OneCursor`/
/// `ManyCursor` through these shapes correctly since #607 — but that alone
/// wasn't enough for `yq`: unlike `jq_runner.rs`'s
/// `generic_result_to_jq_values`, `evaluate_yaml_cursor` never special-cased
/// cursor results, so `yq` kept collapsing duplicate keys. Fixed by widening
/// `can_use_m2_streaming` so these shapes stream through the same fast path
/// `.[0]` uses, instead of expanding `evaluate_yaml_cursor` itself.
#[test]
fn test_duplicate_mapping_key_survives_first_last_stream() -> Result<()> {
    let yaml = "- a: 1\n  a: 2\n- b: 3\n  b: 4\n";

    let (first, code) = run_yq_stdin("first(.[])", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(first, "a: 1\na: 2\n");

    let (last, code) = run_yq_stdin("last(.[])", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(last, "b: 3\nb: 4\n");

    let (first_json, code) = run_yq_stdin("first(.[])", yaml, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(first_json, "{\"a\":1,\"a\":2}\n");

    Ok(())
}

/// #631: computed indexing (`.[(expr)]`, `Expr::IndexExpr`) is the same bug
/// class via a different AST node than `first`/`last` above. `.[0]`
/// (`Expr::Index`, a literal the parser folds at parse time) already
/// preserved duplicate keys, but `.[(1-1)]` — forced past that folding —
/// did not, until `Expr::IndexExpr` was added to `can_use_m2_streaming` too.
#[test]
fn test_duplicate_mapping_key_survives_computed_index() -> Result<()> {
    let yaml = "- a: 1\n  a: 2\n- b: 3\n  b: 4\n";

    let (pretty, code) = run_yq_stdin(".[(1-1)]", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(pretty, "a: 1\na: 2\n");

    let (compact, code) = run_yq_stdin(".[(1-1)]", yaml, &["-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(compact, "a: 1\na: 2\n");

    Ok(())
}

/// #796: `select(...)` had the same latent bug as #631's `first`/`last`/
/// computed-indexing - `eval_generic.rs`'s own `Builtin::Select` arm already
/// forwarded the incoming cursor unchanged, but `can_use_m2_streaming` never
/// had an arm for it, so `yq` always fell through to `evaluate_yaml_cursor`'s
/// `to_owned()` DOM path and silently collapsed duplicate keys (and, since
/// #710, the earlier key's own comment right along with it).
#[test]
fn test_duplicate_mapping_key_survives_select_796() -> Result<()> {
    let yaml = "a: 1 # first\na: 2 # second\n";

    let (out, code) = run_yq_stdin("select(true)", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 1 # first\na: 2 # second\n");

    // A real (non-`true`) predicate must still evaluate correctly through
    // the new M2 path, not just pass everything through unconditionally.
    let (filtered_out, code) = run_yq_stdin("select(.a == 2) | .a", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(filtered_out, "2\n");

    let (json_out, code) = run_yq_stdin("select(true)", yaml, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(json_out, "{\"a\":1,\"a\":2}\n");

    Ok(())
}

/// #733: `-S`/`--sort-keys` excluded identity/navigation queries from the M2
/// cursor-streaming fast path, forcing them through the `OwnedValue` DOM
/// (`IndexMap`-backed), which silently collapsed duplicate keys — the same
/// bug class #442 fixed for the unadorned fast path. Fixed by teaching the
/// YAML mapping cursor streamer to sort fields itself (materializing into a
/// `Vec<YamlField>`, which stays duplicate-key-safe since sorting is stable
/// and doesn't merge same-key entries) rather than excluding `-S` from the
/// fast path. Uses an extra out-of-order, non-duplicate key (`b`) alongside
/// the duplicate `a` pair so the test also confirms sorting still happens.
#[test]
fn test_duplicate_mapping_key_survives_sort_keys() -> Result<()> {
    let yaml = "b: 1\na: 2\na: 3\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-S"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 2\na: 3\nb: 1\n");

    Ok(())
}

/// Same as [`test_duplicate_mapping_key_survives_sort_keys`], for `-o json`.
#[test]
fn test_duplicate_mapping_key_survives_sort_keys_json_output() -> Result<()> {
    let yaml = "b: 1\na: 2\na: 3\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-S", "-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\n  \"a\": 2,\n  \"a\": 3,\n  \"b\": 1\n}\n");

    Ok(())
}

/// #733: `--tab` excluded identity/navigation queries from the M2
/// cursor-streaming fast path for the same reason as `-S` above (the
/// streamers only accepted a numeric space count, not a string/char indent
/// unit), forcing the same `IndexMap`-backed DOM collapse. Fixed by
/// threading an indent unit character through the streamers instead of a
/// bare space count.
#[test]
fn test_duplicate_mapping_key_survives_tab() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["--tab"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 1\na: 2\n");

    Ok(())
}

/// Same as [`test_duplicate_mapping_key_survives_tab`], for `-o json` —
/// also confirms nested indentation actually uses a literal tab character.
#[test]
fn test_duplicate_mapping_key_survives_tab_json_output() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["--tab", "-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\n\t\"a\": 1,\n\t\"a\": 2\n}\n");

    Ok(())
}

/// `-S` and `--tab` combined: both flags widen the same `can_stream_pretty`
/// gate (#733), so confirm they compose correctly rather than one silently
/// overriding the other.
#[test]
fn test_duplicate_mapping_key_survives_sort_keys_and_tab() -> Result<()> {
    let yaml = "b: 1\na: 2\na: 3\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-S", "--tab"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "a: 2\na: 3\nb: 1\n");

    let (json, code) = run_yq_stdin(".", yaml, &["-S", "--tab", "-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(json, "{\n\t\"a\": 2,\n\t\"a\": 3,\n\t\"b\": 1\n}\n");

    Ok(())
}

/// #785's compact-rendering fix has to align a block-sequence item's
/// mapping/sequence value under `- `'s own literal 2-byte width, which
/// can't be expressed as a repetition count of `unit` once `unit != ' '`
/// (`--tab`) - real yq itself has no `--tab` flag at all to serve as an
/// oracle here (`Error: unknown flag: --tab` against v4.53.3), so the bar
/// this pins is self-consistency: the identity/M2 streaming path
/// (`YamlCursor::stream_yaml_value`) and the DOM path (`emit_yaml_value`,
/// forced via `map(.)`) must byte-match each other under `--tab`, not just
/// each look individually plausible. A first version of this fix fed the
/// alignment offset through the same `unit`-repetition helper as ordinary
/// indentation, which under `--tab` wrote literal tab characters instead
/// of the fixed-width literal-space offset `- ` itself always needs,
/// producing two different (and differently wrong) outputs from the two
/// paths for the same input.
#[test]
fn test_compact_seq_item_tab_self_consistent_785() -> Result<()> {
    let yaml = "- a: 1\n  b: 2\n";

    let (streamed, code) = run_yq_stdin(".", yaml, &["--tab"])?;
    assert_eq!(code, 0);

    let (dom, code) = run_yq_stdin("map(.)", yaml, &["--tab"])?;
    assert_eq!(code, 0);

    assert_eq!(streamed, dom);
    // No outer indent level applies at the top level, so the only
    // alignment in play here is the fixed 2-literal-ASCII-space compact
    // offset itself - never tab characters, matching `- `'s own always-
    // ASCII width.
    assert_eq!(streamed, "- a: 1\n  b: 2\n");

    Ok(())
}

/// Same self-consistency bar, but with a genuine outer `--tab` indent
/// level *and* the compact offset both in play, and in the order that
/// actually exposed the bug this pins: the compact transition happens
/// *first* (the top-level array's own single element renders compactly,
/// since it's a non-empty mapping), and only *then* does an ordinary
/// `unit`-based nesting step follow (that mapping's `items` field defers
/// its own array value to a new, further-indented line). A `(current_indent:
/// usize, extra_spaces: usize)` representation collapses to a fixed
/// unit-then-extra order regardless of which happened first, so it got
/// this specific compact-then-normal ordering backwards even after the
/// first `--tab` fix (`extra_spaces` always written after `current_indent`'s
/// `unit` repeats, when here the literal-space compact offset actually
/// came *before* the later tab step, chronologically) - only visible once
/// a normal indent step follows a compact one under a non-space `unit`,
/// which the top-level, no-further-nesting cases above don't exercise.
/// The wrapper array element (rather than a bare top-level mapping) is
/// required for the `map(.)` DOM-forcing query below to preserve
/// structure: real jq's `map(f)` is `[.[] | f]`, and `.[]` on a bare
/// object iterates its *values* (restructuring `{items: [...]}` into
/// `[[...]]`), not its entries - wrapping in an array first makes `map(.)`
/// a true per-element identity instead.
#[test]
fn test_compact_seq_item_tab_self_consistent_compact_then_normal_785() -> Result<()> {
    let yaml = "- items:\n    - a: 1\n      b: 2\n";

    let (streamed, code) = run_yq_stdin(".", yaml, &["--tab"])?;
    assert_eq!(code, 0);

    let (dom, code) = run_yq_stdin("map(.)", yaml, &["--tab"])?;
    assert_eq!(code, 0);

    assert_eq!(streamed, dom);
    assert_eq!(streamed, "- items:\n  \t- a: 1\n  \t  b: 2\n");

    Ok(())
}

/// Same self-consistency bar as [`test_compact_seq_item_tab_self_consistent_785`],
/// for nested compact items (`- - 1\n  - 2\n`) - confirms the `--tab`
/// alignment offset accumulates correctly across more than one compact
/// transition, not just the first. No outer indent level applies at the
/// top level, so - same as the single-level case - the only alignment in
/// play is two lots of the fixed 2-literal-ASCII-space compact offset,
/// never tab characters.
#[test]
fn test_compact_seq_item_tab_self_consistent_nested_785() -> Result<()> {
    let yaml = "- - 1\n  - 2\n";

    let (streamed, code) = run_yq_stdin(".", yaml, &["--tab"])?;
    assert_eq!(code, 0);

    let (dom, code) = run_yq_stdin("map(.)", yaml, &["--tab"])?;
    assert_eq!(code, 0);

    assert_eq!(streamed, dom);
    assert_eq!(streamed, "- - 1\n  - 2\n");

    Ok(())
}

/// `--tab`'s fix (#733) widened `can_stream_pretty`, which also covers
/// `keys_unsorted` (part of `can_use_m2_streaming`) — not a duplicate-key
/// case (keys_unsorted returns an array of key names, so nothing to
/// collapse), but its lazy streamer (`stream_lazy_keys_json`/`_yaml` in
/// `src/jq/stream.rs`) has its own indentation helper, independent of the
/// mapping cursor streamer's. Guards against that helper silently emitting
/// spaces instead of tabs now that this path is reachable with `--tab`.
#[test]
fn test_tab_indent_keys_unsorted() -> Result<()> {
    let yaml = "b: 1\na: 2\n";

    let (output, code) = run_yq_stdin("keys_unsorted", yaml, &["--tab", "-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "[\n\t\"b\",\n\t\"a\"\n]\n");

    Ok(())
}

/// #478: the `--inplace` fast path is scoped to M2-streamable expressions
/// (identity/field/index/iterate); confirm field navigation still rewrites
/// the file correctly, not just plain identity.
#[test]
fn test_inplace_field_navigation_still_works() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a:\n  b: 1\n  c: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg(".a")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "b: 1\nc: 2\n");
    Ok(())
}

/// #478: the `--inplace` fast path's per-file loop iterates the root's
/// virtual document sequence directly (`docs.uncons_cursor()`), rather than
/// the `first_child()` single-document fallback the other `--inplace` tests
/// above exercise implicitly. A multi-document file with a non-identity
/// filter drives that loop through more than one `stream_cursor!` call.
#[test]
fn test_inplace_multi_doc_field_navigation() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a:\n  b: 1\n---\na:\n  b: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg(".a")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "b: 1\n---\nb: 2\n");
    Ok(())
}

/// #478: filters outside `can_use_m2_streaming` must still fall back to the
/// pre-existing DOM path for `--inplace`. `keys` was the original example
/// here, but #685 admitted `Builtin::KeysUnsorted` (what `keys` parses to in
/// `ParserMode::Yq`) into the M2 whitelist, so it no longer exercises this
/// fallback -- `length` still requires `OwnedValue` construction (see
/// `can_use_m2_streaming`'s doc comment) and stands in instead. A two-document
/// file also drives the DOM loop's multi-doc `---` separator logic, which a
/// single document can't reach.
#[test]
fn test_inplace_non_m2_filter_still_works() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\nb: 2\n---\nc: 3")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("length")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "---\n2\n---\n1\n");
    Ok(())
}

/// `--inplace`'s DOM fallback loop applies `--doc` filtering with its own
/// `continue`-past-non-matching-documents branch, separate from the
/// multi-doc `---` separator logic `test_inplace_non_m2_filter_still_works`
/// covers -- exercise it too so both branches of that loop are reached.
#[test]
fn test_inplace_non_m2_filter_doc_filter_still_works() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\nb: 2\n---\nc: 3")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("--doc")
        .arg("1")
        .arg("length")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "1\n");
    Ok(())
}

/// #685: `keys`/`keys_unsorted` becoming M2-eligible means `--inplace 'keys'`
/// now takes the fast path above (`can_inplace_yaml_fast_path`) instead of
/// the DOM fallback `test_inplace_non_m2_filter_still_works` covers --
/// confirm that path still rewrites the file correctly.
#[test]
fn test_inplace_keys_unsorted_m2_fast_path_685() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\nb: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("keys")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "- a\n- b\n");
    Ok(())
}

/// #478: `--inplace` has its own JSON-output M2 fast path gate
/// (`can_inplace_json_fast_path`), separate from the YAML-output one
/// (`can_inplace_yaml_fast_path`) exercised by the tests above, which all
/// default to YAML output. Cover the JSON-output gate directly.
#[test]
fn test_inplace_json_output_fast_path() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\nb: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("-o")
        .arg("json")
        .arg("-I")
        .arg("0")
        .arg(".")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "{\"a\":1,\"b\":2}\n");
    Ok(())
}

/// #224: `--slurp` with `-o json` is the one combination `can_slurp_fast_path`
/// always excludes (it requires YAML output), so it routes through the slow
/// `parse_input` -> `yaml_to_owned_value` -> `resolved_scalar_to_owned` DOM
/// path instead of the M2 cursor streamer. The extensive tag/alias tests
/// added by #224 elsewhere in this file all exercise the direct/M2 path
/// (`-o=json` without `--slurp`), leaving this DOM path's
/// `ResolvedScalar::Float` arm uncovered.
#[test]
fn test_slurp_json_float_scalar_through_dom_path() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "pi: 3.14\n", &["--slurp", "-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"[{"pi":3.14}]"#);
    Ok(())
}

/// #224: like [`test_slurp_json_float_scalar_through_dom_path`], forces the
/// DOM path via `--slurp -o json`, but exercises `yaml_to_owned_value`'s
/// explicit-tag check (`cursor.explicit_tag()` + `resolve_tagged`) instead —
/// the DOM path's copy of the core fix under test in this PR. Mirrors the
/// already-tested direct-path case (`test_yaml_anchored_tag_in_seq_item_resolves`,
/// `test_yaml_default_output_preserves_the_literal_tag`'s "value, quoted"
/// case) where a core-schema tag forces resolution regardless of quoting:
/// `!!int "5"` becomes the number `5`, not the string `"5"`.
#[test]
fn test_slurp_json_explicit_tag_through_dom_path() -> Result<()> {
    let (output, code) = run_yq_stdin(".", "a: !!int \"5\"\n", &["--slurp", "-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"[{"a":5}]"#);
    Ok(())
}

/// #224: sibling of the test above, but the explicit tag (`!custom`) isn't
/// one of the 5 core-schema tags, so `resolve_tagged` returns `None` and
/// `yaml_to_owned_value` must fall through past the tag check to the
/// quoted-string-preservation check below it, rather than resolving.
#[test]
fn test_slurp_json_custom_tag_falls_through_on_dom_path() -> Result<()> {
    let (output, code) =
        run_yq_stdin(".", "a: !custom \"5\"\n", &["--slurp", "-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"[{"a":"5"}]"#);
    Ok(())
}

/// #224: like the two tests above, forces `--slurp -o json`'s DOM path, this
/// time through `yaml_to_owned_value`'s `YamlValue::Alias` arm. That arm now
/// recurses on the target *cursor* rather than a bare `YamlValue`, since a
/// tag on the aliased node lives on the cursor's `bp_pos` and a bare value
/// has already lost it. Checks a plain aliased scalar first, then — mirroring
/// the direct-path `test_yaml_anchored_tag_in_seq_item_resolves` — an aliased
/// node whose *source* carries an explicit tag, which is the only case that
/// can tell "cursor passed through" apart from "bare value passed through".
#[test]
fn test_slurp_json_alias_through_dom_path() -> Result<()> {
    let (plain, code) = run_yq_stdin(".", "a: &x 1\nb: *x\n", &["--slurp", "-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(plain.trim(), r#"[{"a":1,"b":1}]"#);

    let (tagged, code) = run_yq_stdin(
        ".",
        "items:\n  - &a !!str x\n  - *a\n",
        &["--slurp", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(tagged.trim(), r#"[{"items":["x","x"]}]"#);

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
    // array of lines. A bare top-level scalar result drops all of its
    // own styling (#852), so this prints the raw content - including its
    // embedded newlines written literally, not quoted/escaped - matching
    // real yq's own root-scalar behavior (verified elsewhere in this file
    // for the analogous block-scalar-root case).
    let input = "line one\nline two\nline three";
    let (output, exit_code) = run_yq_stdin(".", input, &["-R", "-s"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "line one\nline two\nline three\n");
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
    assert_eq!(output, "\n");
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
    // Each line is its own bare top-level result; an empty line's result
    // drops its own styling (#852) and prints as a blank line, not `''`.
    assert_eq!(output, "line1\n\nline2\n\n\nline3\n");
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
    // Each object keeps its flow style (#739) - verified against the
    // pinned real `yq` binary.
    assert_eq!(output, "{name: alice}\n---\n{name: bob}\n");
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
    // Each sub-array is output as a YAML sequence, keeping its flow style
    // (#739) - verified against the pinned real `yq` binary.
    assert_eq!(output, "[1, 2]\n---\n[3, 4]\n");
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

/// #835: `merge_sources`'s `<<: [...]` sequence-of-sources arm pushed the
/// unresolved sequence-item wrapper `uncons_cursor` yields for a totally
/// bare `-` source (rather than the mapping it defers to), so
/// `merge_field_into`'s `source.first_child()` read the wrapper's one
/// child - the mapping node itself - in place of its first key, silently
/// dropping every field the bare-dash source would have contributed.
#[test]
fn test_yaml_merge_key_bare_dash_deferred_source_expands_712_835() -> Result<()> {
    let input = "item:\n  <<:\n    -\n      a: 1\n      b: 2\n  c: 3\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":1,"b":2,"c":3}"#);
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
// Assignment through an anchor propagates to aliases (#711) - real yq treats
// an anchor/alias pair as one shared node in the representation graph, so a
// write through the anchor must be visible through every alias. succinctly
// deliberately does not reproduce the `&x`/`*x` syntax on output (a separate,
// already-tracked gap, #709) -- only value propagation is in scope here, so
// these assert on values (via `-o=json`) rather than YAML anchor syntax.
// =============================================================================

#[test]
fn test_yaml_assign_through_anchor_updates_alias() -> Result<()> {
    // The issue's own repro: `.a = 99` must be visible through `.b`'s alias.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":99,"b":99}"#);
    Ok(())
}

#[test]
fn test_yaml_assign_through_alias_does_not_clobber_anchor() -> Result<()> {
    // Writing directly to the alias detaches it -- the anchor's own value
    // must be left alone, matching real yq.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".b = 5", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":1,"b":5}"#);
    Ok(())
}

#[test]
fn test_yaml_assign_unrelated_key_does_not_sync_alias() -> Result<()> {
    // Writing an unrelated key must not perturb an anchor/alias pair it
    // never touched.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".c = 5", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":1,"b":1,"c":5}"#);
    Ok(())
}

#[test]
fn test_yaml_assign_nested_field_through_anchor_updates_alias() -> Result<()> {
    // A write nested inside the anchored subtree must propagate the whole
    // updated subtree to every alias, not just a top-level scalar.
    let input = "a: &x\n  p: 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a.p = 9", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"p":9},"b":{"p":9}}"#);
    Ok(())
}

#[test]
fn test_yaml_assign_through_anchor_updates_multiple_aliases() -> Result<()> {
    let input = "a: &x 1\nb: *x\nc: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":99,"b":99,"c":99}"#);
    Ok(())
}

/// #835: `walk_alias_groups` (the sync bookkeeping this whole family relies
/// on) read `cursor.anchor()` on the unresolved sequence-item wrapper
/// `uncons_cursor` yields for a totally bare `-` item, so an anchor written
/// on its own deferred line was never registered - `.items[0].x = 99` wrote
/// only the anchor's own copy and left the alias stale. Same root cause as
/// the mapping-truncation bug this issue was originally filed for, just at
/// a different call site.
#[test]
fn test_yaml_assign_through_bare_dash_deferred_anchor_updates_alias_835() -> Result<()> {
    let input = "items:\n-\n  &base\n  x: 1\n- *base\n";
    let (output, exit_code) = run_yq_stdin(".items[0].x = 99", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"items":[{"x":99},{"x":99}]}"#);
    Ok(())
}

#[test]
fn test_yaml_update_assign_through_anchor_updates_alias() -> Result<()> {
    // `|=` goes through the same fallback path as `=` and must sync too.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a |= . + 100", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":101,"b":101}"#);
    Ok(())
}

#[test]
fn test_yaml_compound_assign_through_anchor_updates_alias() -> Result<()> {
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a += 100", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":101,"b":101}"#);
    Ok(())
}

#[test]
fn test_yaml_del_nested_field_through_anchor_updates_alias() -> Result<()> {
    // Deleting a field inside the anchored subtree shrinks every alias's
    // copy too -- the shared node lost a field, not just the `.a` view.
    let input = "a: &x\n  p: 1\n  q: 2\nb: *x\n";
    let (output, exit_code) = run_yq_stdin("del(.a.q)", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"p":1},"b":{"p":1}}"#);
    Ok(())
}

#[test]
fn test_yaml_del_anchor_key_leaves_alias_at_last_value() -> Result<()> {
    // Deleting the anchor's own key removes it from the document; there's
    // nothing left to propagate, so the alias keeps its last-resolved value
    // -- matching how a detached graph node behaves in real yq.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin("del(.a)", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"b":1}"#);
    Ok(())
}

#[test]
fn test_yaml_plain_alias_read_unaffected_by_assign_gating() -> Result<()> {
    // Regression check: a plain read of an alias (no assignment involved)
    // must remain unaffected by the new alias-sync gating check.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".b", input, &["-o=json"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "1");
    Ok(())
}

#[test]
fn test_yaml_pipe_chained_assigns_through_anchor_updates_alias() -> Result<()> {
    // A chain of assignments joined by `|` -- the common `yq -i '.a = 1 |
    // .b = 2' file` idiom -- must sync aliases too, not just a single
    // top-level assignment. Each `|` stage here rewrites `.a` in place, so
    // the whole chain still qualifies as alias-sensitive.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 5 | .a = 10", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":10,"b":10}"#);
    Ok(())
}

#[test]
fn test_yaml_pipe_chained_assigns_to_different_paths_through_anchor_updates_alias() -> Result<()> {
    // The stages don't need to touch the same path -- as long as every
    // stage is itself alias-sensitive, later stages can read the
    // already-synced value.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99 | .c = .a + 1", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":99,"b":99,"c":100}"#);
    Ok(())
}

// =============================================================================
// Issue #712 - merge keys and anchors/aliases must survive verbatim in YAML
// (not JSON) output on a query that doesn't touch the affected mapping.
// Expected strings are all pinned against mikefarah/yq v4.53.3 output,
// verified directly (not copied from the issue text, which paraphrased away
// yq's `!!merge` tag).
// =============================================================================

#[test]
fn test_yaml_merge_key_preserved_on_identity_output_712() -> Result<()> {
    let input = "default: &d\n  a: 1\nitem:\n  <<: *d\n  b: 2\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output,
        "default: &d\n  a: 1\nitem:\n  !!merge <<: *d\n  b: 2\n"
    );
    Ok(())
}

#[test]
fn test_yaml_scalar_anchor_alias_round_trip_on_identity_output_712() -> Result<()> {
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, input);
    Ok(())
}

#[test]
fn test_yaml_anchored_sequence_item_round_trip_on_identity_output_712() -> Result<()> {
    let input = "items:\n  - &x\n    a: 1\n  - *x\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, input);
    Ok(())
}

#[test]
fn test_yaml_flow_style_anchor_alias_round_trip_on_identity_output_712() -> Result<()> {
    let input = "a: {x: &y 1, z: *y}\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, input);
    Ok(())
}

#[test]
fn test_yaml_merge_key_inline_mapping_source_preserved_712() -> Result<()> {
    let input = "item:\n  <<: {a: 1}\n  b: 2\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "item:\n  !!merge <<: {a: 1}\n  b: 2\n");
    Ok(())
}

#[test]
fn test_yaml_merge_key_multiple_sources_preserved_712() -> Result<()> {
    let input = "a: &a\n  x: 1\nb: &b\n  y: 2\nitem:\n  <<: [*a, *b]\n  c: 3\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output,
        "a: &a\n  x: 1\nb: &b\n  y: 2\nitem:\n  !!merge <<: [*a, *b]\n  c: 3\n"
    );
    Ok(())
}

#[test]
fn test_yaml_duplicate_merge_keys_each_tagged_712() -> Result<()> {
    let input = "a: &a\n  x: 1\nb: &b\n  y: 2\nitem:\n  <<: *a\n  <<: *b\n  c: 3\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output,
        "a: &a\n  x: 1\nb: &b\n  y: 2\nitem:\n  !!merge <<: *a\n  !!merge <<: *b\n  c: 3\n"
    );
    Ok(())
}

#[test]
fn test_yaml_quoted_merge_key_not_tagged_712() -> Result<()> {
    // A quoted "<<" is an ordinary string key, not a merge key - no `!!merge` tag.
    let input = "item:\n  \"<<\": 5\n  b: 2\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, input);
    Ok(())
}

#[test]
fn test_yaml_merge_key_untouched_across_partial_selection_712() -> Result<()> {
    // Selecting a sub-path that doesn't include the merge key's own anchor
    // definition still preserves the merge key literally (a "dangling" alias
    // reference in the printed subtree) - matches real yq's own behavior.
    let input = "default: &d\n  a: 1\nitem:\n  <<: *d\n  b: 2\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "!!merge <<: *d\nb: 2\n");
    Ok(())
}

#[test]
fn test_yaml_merge_key_field_access_still_resolves_712() -> Result<()> {
    // Output preservation must not break actual field lookup through a merge.
    let input = "default: &d\n  a: 1\nitem:\n  <<: *d\n  b: 2\n";
    let (output, exit_code) = run_yq_stdin(".item.a", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "1");
    Ok(())
}

#[test]
fn test_yaml_mapping_anchor_preserved_on_query_result_712() -> Result<()> {
    // A query result whose OWN cursor carries the anchor (not a nested
    // mapping/sequence field's value) must still print it. Verified against
    // real yq v4.53.3: `printf 'item: &x\n  a: 1\n' | yq '.item'` -> `&x\na: 1`.
    let input = "item: &x\n  a: 1\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "&x\na: 1\n");
    Ok(())
}

#[test]
fn test_yaml_whole_document_anchor_preserved_on_identity_712() -> Result<()> {
    // Same as above, but the anchor is on the document root itself (`.`
    // identity, not a sub-query). Verified against real yq v4.53.3.
    let input = "&root\na: 1\nb: 2\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, input);
    Ok(())
}

#[test]
fn test_yaml_sequence_anchor_preserved_on_query_result_712() -> Result<()> {
    let input = "item: &s\n  - 1\n  - 2\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "&s\n- 1\n- 2\n");
    Ok(())
}

#[test]
fn test_yaml_bare_scalar_anchor_dropped_on_query_result_712() -> Result<()> {
    // Real yq (v4.53.3) drops a *scalar's* own anchor when the scalar itself
    // is the top-level output (unlike a mapping/sequence in the same
    // position, which keeps its anchor - see the tests above). Verified:
    // `printf 'item: &y 1\n' | yq '.item'` -> `1`, no `&y`.
    let input = "item: &y 1\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "1");
    Ok(())
}

#[test]
fn test_yaml_empty_mapping_anchor_preserved_on_query_result_712() -> Result<()> {
    // An *empty* container still keeps its own anchor at the top level, same
    // as a non-empty one (see test_yaml_mapping_anchor_preserved_on_query_result_712
    // above) - only a bare scalar's anchor is dropped in this position.
    // Verified: `printf 'item: &x {}\n' | yq '.item'` -> `&x {}`.
    let input = "item: &x {}\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "&x {}\n");
    Ok(())
}

#[test]
fn test_yaml_empty_sequence_anchor_preserved_on_query_result_712() -> Result<()> {
    // Verified: `printf 'item: &x []\n' | yq '.item'` -> `&x []`.
    let input = "item: &x []\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "&x []\n");
    Ok(())
}

#[test]
fn test_yaml_empty_mapping_anchor_preserved_on_whole_document_root_712() -> Result<()> {
    // Same as above, but the anchor is on the document root itself (`.`
    // identity). Verified: `printf '&root {}\n' | yq '.'` -> `&root {}`.
    let input = "&root {}\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, input);
    Ok(())
}

/// #852: a bare top-level scalar document root drops all of its own
/// styling (quote style here), the same way it already drops its own
/// anchor (#712, tests above) and trailing comment (#710). Verified:
/// `printf '"hello world"\n' | yq '.'` -> `hello world`.
#[test]
fn test_yaml_double_quoted_scalar_style_dropped_on_whole_document_root_852() -> Result<()> {
    let input = "\"hello world\"\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "hello world\n");
    Ok(())
}

#[test]
fn test_yaml_single_quoted_scalar_style_dropped_on_query_result_852() -> Result<()> {
    let input = "item: 'hello world'\n";
    let (output, exit_code) = run_yq_stdin(".item", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "hello world\n");
    Ok(())
}

/// A nested scalar under the same key keeps its quote style when the
/// *whole document* (not just that field) is the result - contrasts with
/// the two tests above, where the scalar itself is the entire output.
#[test]
fn test_yaml_quoted_scalar_style_kept_when_nested_under_the_document_root_852() -> Result<()> {
    let input = "item: 'hello world'\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, input);
    Ok(())
}

/// Real `yq` drops root-scalar styling unconditionally, even for content
/// that would be ambiguous (parsed as a different type) if left unquoted
/// in a normal nested position - there's no sibling content at the
/// document root for it to be confused with. Verified:
/// `printf '"true"\n' | yq '.'` -> bare `true`, not re-quoted for safety.
#[test]
fn test_yaml_ambiguous_scalar_style_dropped_unconditionally_on_document_root_852() -> Result<()> {
    let input = "\"true\"\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "true\n");
    Ok(())
}

/// #852: an empty double-quoted string root prints as literally nothing
/// (not `""`), matching real `yq` exactly.
#[test]
fn test_yaml_empty_string_style_dropped_on_document_root_852() -> Result<()> {
    let (output, exit_code) = run_yq_stdin(".", "\"\"\n", &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "\n");
    Ok(())
}

/// #852: content that would need quoting in a normal nested position
/// (starts with `- `, ambiguous with a block-sequence item) still drops
/// its quotes unconditionally at the document root - there's no sibling
/// content there for it to be confused with.
#[test]
fn test_yaml_dash_prefixed_scalar_style_dropped_on_document_root_852() -> Result<()> {
    let input = "'- foo'\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "- foo\n");
    Ok(())
}

/// #852 (found in code review): the M2/cursor path fix alone left the
/// *computed*-value path (`GenericResult::One`/`Many`, e.g. `-n`
/// construction or any query whose root goes through the general
/// evaluator rather than staying a pure cursor passthrough) still quoting
/// an ambiguous root string - `OwnedValue::stream_yaml`
/// (`src/jq/stream.rs`) needed the identical root-only special case.
#[test]
fn test_null_input_computed_scalar_style_dropped_on_document_root_852() -> Result<()> {
    let (output, exit_code) = run_yq_stdin("\"true\"", "", &["-n"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "true\n");
    Ok(())
}

/// Same computed-value gap as above, reached via arithmetic (`+`, not
/// M2-streamable) on real input instead of `-n` construction.
#[test]
fn test_computed_string_concat_style_dropped_on_query_result_852() -> Result<()> {
    let input = "a: \"tr\"\nb: \"ue\"\n";
    let (output, exit_code) = run_yq_stdin(".a + .b", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "true\n");
    Ok(())
}

// =============================================================================
// Alias-sync survives a pass-through stage mixed into the pipe (#764) - the
// #711 gate originally required *every* pipe stage to be assignment-family,
// so `.a = 99 | select(true)` fell outside it and `.b` went stale again. `.`,
// `select(...)`, `debug`, and `empty` never rewrite or reshape the document
// they pass through, so mixing one into an assignment pipe is now allowed.
// =============================================================================

#[test]
fn test_yaml_assign_then_select_true_through_anchor_updates_alias() -> Result<()> {
    // The issue's own repro.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99 | select(true)", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":99,"b":99}"#);
    Ok(())
}

#[test]
fn test_yaml_assign_then_select_false_produces_no_output() -> Result<()> {
    // `select(false)` drops the document entirely -- there's nothing to
    // sync, and nothing should be printed.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99 | select(false)", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "");
    Ok(())
}

#[test]
fn test_yaml_select_guard_then_assign_through_anchor_updates_alias() -> Result<()> {
    // The guard-style idiom named in #764: a leading `select` filters, then
    // the assignment writes.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin("select(.a > 0) | .a = 5", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":5,"b":5}"#);
    Ok(())
}

#[test]
fn test_yaml_assign_then_debug_through_anchor_updates_alias() -> Result<()> {
    // `debug` passes its input through unchanged (aside from the stderr
    // side effect), so it must not block the sync either.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99 | debug", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":99,"b":99}"#);
    Ok(())
}

#[test]
fn test_yaml_assign_then_empty_produces_no_output() -> Result<()> {
    // `empty` drops the document -- same as `select(false)`, nothing to
    // sync and nothing to print.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99 | empty", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "");
    Ok(())
}

#[test]
fn test_yaml_assign_then_map_through_anchor_still_excluded() -> Result<()> {
    // Regression guard: `map` is not on the pass-through allow-list (it can
    // reshape the document), so a pipe mixing it with an assignment must
    // stay excluded from alias-sync, exactly as before #764.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin(".a = 99 | map(.)", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r"[99,1]");
    Ok(())
}

#[test]
fn test_yaml_paren_wrapped_optional_assign_through_anchor_updates_alias() -> Result<()> {
    // `(.a = 99)?` -- `Optional` wrapping `Paren` wrapping `Assign` -- must
    // still count as alias-sensitive: `is_alias_sensitive_assign` unwraps
    // both `Paren` and `Optional` before checking for a write underneath.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin("(.a = 99)?", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":99,"b":99}"#);
    Ok(())
}

#[test]
fn test_yaml_bare_select_without_assign_is_unaffected() -> Result<()> {
    // Regression guard: a pass-through stage with *no* assignment anywhere
    // in the pipe must not trigger alias-sync snapshotting at all -- there's
    // nothing to diff, so the plain read behavior from before #764 holds.
    let input = "a: &x 1\nb: *x\n";
    let (output, exit_code) = run_yq_stdin("select(true)", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":1,"b":1}"#);
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
fn test_yaml_anchored_tag_in_seq_item_resolves() -> Result<()> {
    // Consuming the anchor before dispatching means the tag is seen rather
    // than absorbed into a plain scalar, so `- &a !!str x` resolves to the
    // string "x" (matching yq: the tag forces the type, then is dropped from
    // output) rather than erroring or yielding the literal text "!!str x"
    // (#224). The anchor still resolves too.
    let input = "items:\n  - &a !!str x\n  - *a\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"items":["x","x"]}"#);
    Ok(())
}

#[test]
fn test_yaml_flow_context_resolves_tags_like_block_context() -> Result<()> {
    // #369 made flow context *reject* a tag the way block context did, rather
    // than absorbing it as scalar text (`a: [!!str x]` used to yield the
    // string `"!!str x"`). #224 replaced that rejection with real resolution
    // everywhere, in both contexts uniformly — so every position below now
    // drops the tag from output (and lets a core-schema tag force the type)
    // instead of erroring.
    //
    // Every flow position that reaches a plain-scalar reader. The last two are
    // the ones no other case covers: an implicit `k: v` entry inside a flow
    // *sequence* enters through `parse_flow_key` (shared with the flow-mapping
    // key site since #409), and the explicit `? k : v` form through
    // `parse_explicit_flow_unquoted_key`. Without them, breaking either path
    // leaves this test green.
    for (name, input, expected) in [
        ("seq item", "a: [!!str x]\n", r#"{"a":["x"]}"#),
        ("mapping value", "a: {k: !custom v}\n", r#"{"a":{"k":"v"}}"#),
        ("mapping key", "a: {!!str k: v}\n", r#"{"a":{"k":"v"}}"#),
        (
            "seq item with a plain sibling",
            "a: [!custom x, plain]\n",
            r#"{"a":["x","plain"]}"#,
        ),
        (
            "implicit entry key in a seq",
            "a: [!!str k: v]\n",
            r#"{"a":[{"k":"v"}]}"#,
        ),
        (
            "explicit key",
            "a: [? !!str k : v]\n",
            r#"{"a":[{"k":"v"}]}"#,
        ),
        // #402 routed the flow *mapping*'s explicit key through the same reader.
        // Without this the tag could be dropped from that path unnoticed.
        (
            "explicit key in a mapping",
            "a: {? !!str k : v}\n",
            r#"{"a":{"k":"v"}}"#,
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name}: expected clean success");
        assert_eq!(output.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_default_output_preserves_the_literal_tag() -> Result<()> {
    // Default (YAML) output has tag syntax, unlike JSON, so a tag is
    // re-emitted verbatim rather than dropped — matching real `yq`
    // v4.53.3, checked directly against each case below. JSON output for
    // the same inputs still drops the tag (#224); this is that dropped
    // information being representable again in the format that can hold it.
    for (name, input, expected) in [
        ("value, unquoted", "a: !!str 1\n", "a: !!str 1"),
        ("value, quoted", "a: !!int \"5\"\n", "a: !!int \"5\""),
        ("value, custom tag", "a: !custom v\n", "a: !custom v"),
        ("key, same line", "!!str key: value\n", "!!str key: value"),
        (
            "key, nested",
            "outer:\n  !!str key: value\n",
            "outer:\n  !!str key: value",
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &[])?;
        assert_eq!(exit_code, 0, "{name}: expected clean success");
        assert_eq!(output.trim(), expected, "{name}");
    }
    Ok(())
}

/// #747: `type`/`==`/arithmetic/`select` on the cursor-evaluator path
/// (`eval_generic.rs`) previously ignored an explicit YAML tag, because
/// their shared materializer (`to_owned`) only ever saw a bare
/// `DocumentValue`, which has no `bp_pos` to look the tag up with — only a
/// `YamlCursor` does. `succinctly yq '.'`'s JSON/YAML output was already
/// correct (it streams straight from a cursor, never through `to_owned`),
/// so this is the divergence the issue reported: `.` said `"1"` but
/// `.a | type` still said `"number"`. `type` doesn't call `to_owned` at all
/// (it reads `DocumentValue::type_name()` directly), so it needed its own
/// fix (`tagged_type_name`) rather than inheriting `to_owned_cursor`'s.
#[test]
fn test_yaml_explicit_tag_type_resolves_747() -> Result<()> {
    for (name, input, expected) in [
        ("str tag forces string", "a: !!str 1\n", "string"),
        ("int tag forces number", "a: !!int \"5\"\n", "number"),
        ("float tag forces number", "a: !!float \"5\"\n", "number"),
        ("bool tag forces boolean", "a: !!bool \"yes\"\n", "boolean"),
        ("null tag forces null", "a: !!null anything\n", "null"),
        // A non-core-schema tag doesn't force anything — falls through to
        // ordinary plain-scalar resolution, same as `resolve_tagged`'s
        // `None` contract elsewhere in this file (#224).
        ("custom tag is untouched", "a: !custom 1\n", "number"),
    ] {
        let (output, exit_code) = run_yq_stdin(".a | type", input, &[])?;
        assert_eq!(exit_code, 0, "{name}: expected clean success");
        assert_eq!(output.trim(), expected, "{name}");
    }
    Ok(())
}

/// #747: sibling of [`test_yaml_explicit_tag_type_resolves_747`] for `==`
/// and arithmetic, which go through `to_owned_cursor` (via `eval_single`'s
/// `OneCursor`/`ManyCursor` `GenericResult` arms) rather than `type`'s own
/// direct fix.
#[test]
fn test_yaml_explicit_tag_eq_and_arithmetic_resolve_747() -> Result<()> {
    let (eq_str, code) = run_yq_stdin(r#".a == "1""#, "a: !!str 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(eq_str.trim(), "true");

    let (eq_num, code) = run_yq_stdin(".a == 1", "a: !!str 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(eq_num.trim(), "false");

    let (add, code) = run_yq_stdin(".a + 1", "a: !!int \"5\"\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(add.trim(), "6");

    // `tagged_scalar_to_owned`'s Null/Bool/Float arms: `type`'s own tests
    // (`test_yaml_explicit_tag_type_resolves_747`) exercise these tags only
    // through `tagged_type_name`, which resolves straight to
    // `ResolvedScalar::type_name()` without ever materializing an
    // `OwnedValue` — so an `==` comparison (which does materialize, via
    // `to_owned_cursor`) is needed to reach these three arms at all.
    let (eq_null, code) = run_yq_stdin(".a == null", "a: !!null anything\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(eq_null.trim(), "true");

    let (eq_bool, code) = run_yq_stdin(".a == true", "a: !!bool \"yes\"\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(eq_bool.trim(), "true");

    let (eq_float, code) = run_yq_stdin(".a == 5.0", "a: !!float \"5\"\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(eq_float.trim(), "true");

    Ok(())
}

/// #747: `select`'s cursor-forwarding arm (`eval_builtin`'s `Builtin::
/// Select`, #378) republishes the incoming cursor on a truthy condition
/// rather than a plain value — the condition itself is evaluated through the
/// same `eval_single` recursion as the tests above, so a tagged condition
/// resolves correctly, and (since select is a passthrough) the untouched
/// original value keeps its own tag on output.
#[test]
fn test_yaml_explicit_tag_select_condition_resolves_747() -> Result<()> {
    let (output, exit_code) = run_yq_stdin(
        r#"select(.a | type == "string")"#,
        "a: !!str 1\nb: 2\n",
        &[],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), "a: !!str 1\nb: 2");

    let (empty, exit_code) = run_yq_stdin(
        r#"select(.a | type == "number")"#,
        "a: !!str 1\nb: 2\n",
        &[],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(empty.trim(), "");

    Ok(())
}

/// #747: an explicit tag nested inside a sequence/mapping must resolve too —
/// `to_owned_cursor` has to recurse via `field.value_cursor`/
/// `elems.uncons_cursor` (mirroring `to_owned_with_comments`'s existing
/// cursor-threading pattern) rather than plain `to_owned`'s cursor-less
/// `field.value`/`elems.uncons`, or only the top-level node's tag would ever
/// be seen.
#[test]
fn test_yaml_explicit_tag_resolves_when_nested_747() -> Result<()> {
    let input = "a:\n  - !!str 1\n  - !!int \"2\"\nb:\n  c: !!str 3\n";

    let (seq_types, code) = run_yq_stdin("[.a[] | type]", input, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(seq_types.trim(), r#"["string","number"]"#);

    let (obj_type, code) = run_yq_stdin(".b.c | type", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(obj_type.trim(), "string");

    Ok(())
}

/// #903 review round: a YAML alias occurrence has no `bp_pos` tag of its
/// own — a valid alias node (`*anchor`) can't carry a tag in the source —
/// so `YamlCursor::explicit_tag()` must dereference through
/// `YamlValue::Alias`'s `target` to the anchor definition's tag, the same
/// way every other alias-transparent accessor on `YamlValue` already does
/// (`as_bool`/`as_i64`/`as_f64`/`as_object`/`as_array`/`type_name`).
/// Without that dereference, `.y`'s tag silently vanished even though the
/// direct `.x` access (and `-o=json`'s cursor-streaming output) already
/// resolved it correctly.
#[test]
fn test_yaml_explicit_tag_resolves_through_alias_903() -> Result<()> {
    let input = "x: &a !!str 1\ny: *a\n";

    let (types, code) = run_yq_stdin("[.x, .y] | map(type)", input, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(types.trim(), r#"["string","string"]"#);

    let (eq, code) = run_yq_stdin(r#".y == "1""#, input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(eq.trim(), "true");

    let (add, code) = run_yq_stdin(".y + 1", "x: &a !!int \"5\"\ny: *a\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(add.trim(), "6");

    Ok(())
}

/// #903 review round: `Expr::Field`/`Expr::Index`'s type-mismatch error
/// messages, and `Builtin::First`/`Last`/`Reverse`/`Pivot`/`Shuffle`'s
/// non-array-input error messages, all had `cursor` in scope but still read
/// the raw, untagged `value.type_name()` — so the error text could name a
/// different type than `.a | type` (fixed earlier in #747) reports for the
/// exact same node. Now routed through `tagged_type_name` like `type`
/// itself.
#[test]
fn test_yaml_explicit_tag_error_messages_agree_with_type_903() -> Result<()> {
    let input = "a: !!str 1\n";

    let (_, err, code) = run_yq_stdin_with_stderr(".a.foo", input, &[])?;
    assert_eq!(code, 1);
    assert!(err.contains("Cannot index string with"), "{err}");

    let (_, err, code) = run_yq_stdin_with_stderr(".a | last", input, &[])?;
    assert_eq!(code, 1);
    assert!(err.contains("Cannot index string with"), "{err}");

    let (_, err, code) = run_yq_stdin_with_stderr(".a | reverse", input, &[])?;
    assert_eq!(code, 1);
    assert!(err.contains("Cannot index string with"), "{err}");

    let (_, err, code) = run_yq_stdin_with_stderr(".a | pivot", input, &[])?;
    assert_eq!(code, 1);
    assert!(err.contains("expected array, got string"), "{err}");

    let (_, err, code) = run_yq_stdin_with_stderr(".a | shuffle", input, &[])?;
    assert_eq!(code, 1);
    assert!(err.contains("shuffle requires array, got string"), "{err}");

    Ok(())
}

/// #903 review round: `to_entries`, `reverse`, `pivot`, and `shuffle` each
/// materialize their elements via a bare `to_owned`/`collect_values` instead
/// of the cursor-carrying `to_owned_cursor`/`collect_cursors`, silently
/// dropping an explicit tag on any element they touch — the exact #747
/// defect, left unfixed in these four builtins specifically. `shuffle`'s
/// element order is random, so it's checked via `type` over every element
/// rather than positionally.
#[test]
fn test_yaml_explicit_tag_resolves_in_to_entries_reverse_pivot_shuffle_903() -> Result<()> {
    let (entries, code) =
        run_yq_stdin(". | to_entries", "a: !!str 1\nb: 2\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(
        entries.trim(),
        r#"[{"key":"a","value":"1"},{"key":"b","value":2}]"#
    );

    let (reversed, code) = run_yq_stdin(
        ".a | reverse",
        "a:\n  - !!str 1\n  - 2\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(reversed.trim(), r#"[2,"1"]"#);

    let (pivoted, code) = run_yq_stdin(
        ".a | pivot",
        "a:\n  - [!!str 1, 2]\n  - [3, 4]\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(pivoted.trim(), r#"[["1",3],[2,4]]"#);

    let (shuffled_types, code) = run_yq_stdin(
        "[.a | shuffle | .[] | type]",
        "a:\n  - !!str 1\n  - !!str 2\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(shuffled_types.trim(), r#"["string","string"]"#);

    Ok(())
}

/// #903 review round: the `is*` family (`isnull`/`isboolean`/`isnumber`/
/// `isstring`/`isarray`/`isobject`) called `DocumentValue::is_null`/
/// `is_bool`/etc. directly — the exact same tag-blind gap `type` had before
/// #747 — plus a second, tag-independent bug the fix incidentally closes for
/// these call sites: `is_number`/`is_string`'s *default* implementations can
/// both answer `true` for the same untagged plain YAML scalar (`as_str()`
/// always succeeds on a `YamlValue::String` node regardless of its resolved
/// type), which `type_name()`'s single-answer match doesn't have. Routing
/// through `tagged_type_name` fixes both at once.
#[test]
fn test_yaml_is_builtins_resolve_tags_and_agree_with_type_903() -> Result<()> {
    for (query, input, expected) in [
        ("isboolean", "a: !!bool \"yes\"\n", "true"),
        ("isnull", "a: !!null anything\n", "true"),
        ("isstring", "a: !!str 1\n", "true"),
        ("isnumber", "a: !!str 1\n", "false"),
        // Untagged plain number: isnumber true, isstring false (not both).
        ("isnumber", "a: 1\n", "true"),
        ("isstring", "a: 1\n", "false"),
        ("isarray", "a: [1, 2]\n", "true"),
        ("isobject", "a: {x: 1}\n", "true"),
    ] {
        let (out, code) = run_yq_stdin(&format!(".a | {query}"), input, &[])?;
        assert_eq!(code, 0, "{query} on {input:?}: expected clean success");
        assert_eq!(out.trim(), expected, "{query} on {input:?}");
    }
    Ok(())
}

/// #903 review round: `to_owned_key_shape` (backing computed-index keys and
/// slice bounds) took a bare `&V` with no cursor, so a tagged key/bound
/// expression resolved as its untagged plain-scalar type instead —
/// `to_owned_key_shape_cursor` is the cursor-carrying sibling that closes
/// this the same way `to_owned_cursor` closes it for `to_owned`.
#[test]
fn test_yaml_explicit_tag_resolves_in_computed_key_and_slice_bound_903() -> Result<()> {
    let (matched, code) = run_yq_stdin(".a[.k]", "k: !!str 1\na:\n  \"1\": matched\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(matched.trim(), "matched");

    let (sliced, code) = run_yq_stdin(
        ".arr[.k:3]",
        "k: !!int \"1\"\narr: [10, 20, 30, 40]\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(sliced.trim(), "[20,30]");

    Ok(())
}

// The following `test_yaml_tag_gate_gap_*` tests were the #664 audit's output:
// they used to pin the *wrong* behavior (`exit_code == 0`, tag text silently
// absorbed into the surrounding scalar/key) for every block-context path
// found ungated against `check_unsupported` (`src/yaml/parser.rs`, since
// deleted — #224 replaced it with real property consumption throughout, see
// `parse_node_properties`/`record_key_properties`). Kept under their
// original `_gate_gap_` names since they're still the audit's per-root-cause
// map, now asserting each is *closed*: tag dropped from output, and
// resolution/anchor/alias behavior all correct. See
// `docs/compliance/yaml/limitations.md` for the historical audit (root
// causes, file:line references, the complete gated/ungated table).

#[test]
fn test_yaml_tag_gate_gap_document_root_indented_scalar() -> Result<()> {
    // Root cause 1 (`parse_document_line`'s gate ran before indentation was
    // skipped, so it only ever fired at column 0) is moot now that a tag
    // never errors — column 0 and indented must resolve identically.
    for (name, input) in [("column 0", "!!str x\n"), ("indented", "  !!str x\n")] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{name}: expected clean success");
        assert_eq!(out.trim(), "\"x\"", "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_tag_gate_gap_document_start_two_documents() -> Result<()> {
    // The `J7PZ` corpus shape (`--- !!omap`). `parse_inline_document_value`
    // (root cause 2) used to absorb the tag as a complete document-root
    // scalar, so the sequence content on the following lines started a
    // *second* document — one YAML input streamed as two JSON documents.
    // `parse_block_node`'s combined `&`/`!` arm now consumes the tag first,
    // so `!!omap`'s deferred sequence stays this document's single value.
    let (out, code) = run_yq_stdin(".", "--- !!omap\n- a: 1\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[{\"a\":1}]");
    Ok(())
}

#[test]
fn test_yaml_tag_gate_gap_document_start_inline_value() -> Result<()> {
    // Root cause 2: `parse_inline_document_value` (`parser.rs`) dispatches
    // content after `---` through `parse_block_node` directly. Its doc
    // comment used to name this as #224's to settle; `parse_block_node`'s
    // combined property arm now covers it the same as every other position.
    for (name, input) in [
        ("plain tag", "--- !!str x\n"),
        ("anchor then tag", "--- &a !!str x\n"),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{name}: expected clean success");
        assert_eq!(out.trim(), "\"x\"", "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_tag_gate_gap_value_deferred_to_next_line() -> Result<()> {
    // Root causes 2/3: a value deferred to the next line eventually lands in
    // `parse_block_node`'s scalar-dispatch arms, which used to never call
    // `check_unsupported`. `parse_mapping_entry`'s next-line branch now defers
    // to the main loop for a `!`-prefixed line the same way it already did
    // for `&`/`*` (its own gate only ever guarded the *same-line* branch).
    for (name, input, expected) in [
        (
            "mapping value",
            "key:\n  !!str value\n",
            "{\"key\":\"value\"}",
        ),
        ("sequence item", "- \n  !!str x\n", "[\"x\"]"),
        (
            "compact mapping value",
            "- k:\n    !!str v\n",
            "[{\"k\":\"v\"}]",
        ),
        (
            "anchor-prefixed block value",
            "a: &b\n  !!str x\n",
            "{\"a\":\"x\"}",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{name}: expected clean success");
        assert_eq!(out.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_tag_gate_gap_anchor_then_tag_same_line() -> Result<()> {
    // Root cause 4: `parse_mapping_entry` and `parse_compact_mapping_entry`
    // each used to check `check_unsupported` once, *before* checking for a
    // `&anchor`, then dispatch the post-anchor value inline with no second
    // check. `parse_node_properties` now consumes both, in either order, in
    // one call — an anchor no longer leaves a stale gate behind it.
    for (name, input, expected) in [
        ("mapping entry", "key: &a !!str x\n", "{\"key\":\"x\"}"),
        (
            "compact mapping entry",
            "- k: &a !!str v\n",
            "[{\"k\":\"v\"}]",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{name}: expected clean success");
        assert_eq!(out.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_tag_gate_gap_explicit_value() -> Result<()> {
    // Root cause 5: `parse_explicit_value` (the `: value` half of `? key` /
    // `: value`) used to have zero `check_unsupported` calls in the entire
    // function — its scalar arm, its anchor-then-value arm, and its
    // compact-mapping-value arm. `parse_node_properties` now runs once at
    // entry, ahead of all three.
    for (name, input, expected) in [
        ("plain value", "? k\n: !!str v\n", "{\"k\":\"v\"}"),
        ("anchor then tag", "? k\n: &a !!str v\n", "{\"k\":\"v\"}"),
        (
            "compact mapping value, tag on its key",
            "? k\n: !!str b: c\n",
            "{\"k\":{\"b\":\"c\"}}",
        ),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{name}: expected clean success");
        assert_eq!(out.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_tag_gate_gap_key_position() -> Result<()> {
    // Root cause 6: no block-context key parser used to gate the key itself —
    // `parse_mapping_entry` and `parse_explicit_key` (whose inline-key
    // dispatch had no gate at all) never called `check_unsupported` on key
    // text. `record_key_properties` now runs before each key is parsed. The
    // mapping-entry case is indented so root cause 1's column-0 accident
    // isn't what makes it pass.
    //
    // A compact-mapping key reached via a sequence item (`- !!str k: v`) is
    // *not* included here — it was already gated, because
    // `parse_sequence_item`'s own gate ran unconditionally right after `- `,
    // before the compact-mapping dispatch was even decided.
    for (name, input, expected) in [
        (
            "nested mapping key",
            "outer:\n  !!str key: value\n",
            "{\"outer\":{\"key\":\"value\"}}",
        ),
        ("explicit key", "? !!str k\n: v\n", "{\"k\":\"v\"}"),
    ] {
        let (out, code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "{name}: expected clean success");
        assert_eq!(out.trim(), expected, "{name}");
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
        // #402. A `:` ends the key only before a blank, a break or end of input.
        // Before a flow indicator it is content, and the scan stops at the
        // indicator one byte later — so the colon stays in the key.
        (
            "colon then comma",
            "a: [? k :, x]\n",
            r#"{"a":[{"k :":null},"x"]}"#,
        ),
        (
            "colon then bracket",
            "a: [? k :]\n",
            r#"{"a":[{"k :":null}]}"#,
        ),
        (
            "unspaced colon then bracket",
            "a: [? k:]\n",
            r#"{"a":[{"k:":null}]}"#,
        ),
        (
            "colon then comma on the next line",
            "a: [? k\n  :, x]\n",
            r#"{"a":[{"k :":null},"x"]}"#,
        ),
        (
            "colon then bracket on the next line",
            "a: [? k\n  :]\n",
            r#"{"a":[{"k :":null}]}"#,
        ),
        // #402. A space before the break used to abort the parse outright
        // ("unexpected character 'x'") while the same input without it parsed.
        // Folding drops the trailing space, so both give the one key.
        (
            "space before a continued line",
            "a: [? k \n  x : v]\n",
            r#"{"a":[{"k x":"v"}]}"#,
        ),
        (
            "tab before a continued line",
            "a: [? k\t\n  x : v]\n",
            r#"{"a":[{"k x":"v"}]}"#,
        ),
        (
            "space before a continued line, no value",
            "a: [? k \n  x]\n",
            r#"{"a":[{"k x":null}]}"#,
        ),
        (
            "space before a continued line, in a mapping",
            "a: {? k \n  x : v}\n",
            r#"{"a":{"k x":"v"}}"#,
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
fn test_yaml_anchor_on_flow_sequence_key_binds_to_key() -> Result<()> {
    // The flow-*sequence* counterpart of the test above (#409): an implicit
    // single-pair-mapping entry inside `[...]` also opens the key's BP node
    // before the anchor is read, so a bare `parse_anchor` here bound the
    // anchor to the mapping *wrapper* `parse_implicit_flow_mapping_entry` was
    // about to open, not the key `k` inside it — invisible on `&flowseq [...,
    // &c c: d, ...]` (corpus case CN3R) since that case never aliases `&c`.
    let input = "[&x k: 1, *x: 2]\n";
    let (output, exit_code) = run_yq_stdin("at_offset(11)", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#""k""#);
    Ok(())
}

#[test]
fn test_yaml_plain_scalar_dash_continuation_at_indent_two_folds() -> Result<()> {
    // #484, corpus-latent the same way #409 was: the YAML Test Suite's only
    // relevant case, AB8U, uses a `- `-led continuation line at indent 1 - the
    // one indent the parser happened to get right - so it gave no signal that
    // indent 2 and deeper wrongly cut the scalar short and reparsed the `- `
    // line as a nested sequence instead of folding it into the scalar, as `yq`
    // does.
    let input = "- x\n  - y\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"["x - y"]"#);
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
// Explicit keys in flow mappings (#402) - `{? k : v}` keys on `k`, not on `? k`.
// Expectations are mikefarah/yq v4.53.3 output.
// =============================================================================

#[test]
fn test_yaml_explicit_flow_mapping_key_drops_the_indicator() -> Result<()> {
    // `? ` is a node marker, not key text. The flow *sequence* path consumed it
    // before marking the key's start; the flow *mapping* path planted the
    // interest bit on the `?` first, so the indicator and the space after it
    // ended up inside the key string — `{"? k":"v"}`. Silently wrong data, the
    // same failure mode as #339 (block context) and #369 (flow tags).
    for (name, input, expected) in [
        ("plain key", "a: {? k : v}\n", r#"{"a":{"k":"v"}}"#),
        ("no value", "a: {? k}\n", r#"{"a":{"k":null}}"#),
        // The quoted arms: every input that reached them exhibited the bug, so
        // they were the last uncovered lines of the explicit branch.
        (
            "double-quoted key",
            "a: {? \"k\" : v}\n",
            r#"{"a":{"k":"v"}}"#,
        ),
        (
            "single-quoted key",
            "a: {? 'k' : v}\n",
            r#"{"a":{"k":"v"}}"#,
        ),
        ("embedded colon", "a: {? a:b : v}\n", r#"{"a":{"a:b":"v"}}"#),
        ("empty key", "a: {? : v}\n", r#"{"a":{"":"v"}}"#),
        (
            "empty key, no value",
            "a: {? , b: 1}\n",
            r#"{"a":{"":null,"b":1}}"#,
        ),
        ("empty key at the brace", "a: {? }\n", r#"{"a":{"":null}}"#),
        (
            "two explicit entries",
            "a: {? k : v, ? j : w}\n",
            r#"{"a":{"k":"v","j":"w"}}"#,
        ),
        (
            "after an implicit entry",
            "a: {x: 1, ? k : v}\n",
            r#"{"a":{"x":1,"k":"v"}}"#,
        ),
        // The anchor is stripped when the key is read, as in the sequence form.
        ("anchored key", "a: {? &n k : v}\n", r#"{"a":{"k":"v"}}"#),
        // Complex keys open their own BP nodes; yq renders them as "" in JSON.
        ("sequence key", "a: {? [1,2] : v}\n", r#"{"a":{"":"v"}}"#),
        ("mapping key", "a: {? {x: 1} : v}\n", r#"{"a":{"":"v"}}"#),
    ] {
        let (stdout, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I", "0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(stdout.trim(), expected, "{name}");
    }
    Ok(())
}

// =============================================================================
// An anchor at the end of a compact mapping entry's line (#406) - `- k: &a`
// puts the value on the *next* line, not this one. Expectations are
// mikefarah/yq v4.53.3 output.
// =============================================================================

#[test]
fn test_yaml_compact_entry_trailing_anchor_keeps_the_nested_value() -> Result<()> {
    // The headline #406 repros. `parse_compact_mapping_entry` asked
    // `at_line_end` before anything consumed the `&a`, so the entry took the
    // inline path and `parse_inline_value`'s multi-line plain-scalar rule ate
    // the block below: the mapping form came out as {"k":"b"} and the sequence
    // form as {"k":null}. Adding the anchor is the only difference from the
    // shapes above them, which were always right.
    for (name, input, expected) in [
        (
            "mapping, no anchor",
            "- k:\n    b: 1\n",
            r#"[{"k":{"b":1}}]"#,
        ),
        ("mapping", "- k: &a\n    b: 1\n", r#"[{"k":{"b":1}}]"#),
        ("sequence, no anchor", "- k:\n    - 1\n", r#"[{"k":[1]}]"#),
        ("sequence", "- k: &a\n    - 1\n", r#"[{"k":[1]}]"#),
        // A block sequence may sit at its parent key's own indent.
        (
            "sequence at entry indent",
            "- k: &a\n  - 1\n",
            r#"[{"k":[1]}]"#,
        ),
        // The entry keeps its siblings: the nested mapping must close so `j`
        // lands beside `k`, not inside it.
        (
            "sibling entry after the nested block",
            "- k: &a\n    b: 1\n  j: 2\n",
            r#"[{"k":{"b":1},"j":2}]"#,
        ),
        // `at_line_end` is also true at a comment, so the anchor is still last.
        (
            "anchor then comment",
            "- k: &a # note\n    b: 1\n",
            r#"[{"k":{"b":1}}]"#,
        ),
        // A flow collection on the next line went the same way — `"[1, 2]"`.
        (
            "flow collection below",
            "- k: &a\n    [1, 2]\n",
            r#"[{"k":[1,2]}]"#,
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(output.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_explicit_flow_key_agrees_across_positions() -> Result<()> {
    // The fix is "one definition of `? key`, shared by both flow containers",
    // so the two call sites must agree. They diverged before precisely because
    // there were two copies of the dispatch — only the sequence one was right.
    //
    // Comparing the entries themselves rather than whole documents: `.a` in a
    // mapping and `.a[0]` in a sequence are the same single-pair mapping.
    for (name, shape, expected) in [
        ("plain", "k : v", r#"{"k":"v"}"#),
        ("no value", "k", r#"{"k":null}"#),
        ("double-quoted", "\"k\" : v", r#"{"k":"v"}"#),
        ("single-quoted", "'k' : v", r#"{"k":"v"}"#),
        ("embedded colon", "a:b : v", r#"{"a:b":"v"}"#),
        ("trailing colon", "k :", r#"{"k :":null}"#),
        ("continued line", "k \n  x : v", r#"{"k x":"v"}"#),
    ] {
        let (in_mapping, code) = run_yq_stdin(
            ".a",
            &format!("a: {{? {shape}}}\n"),
            &["-o", "json", "-I", "0"],
        )?;
        assert_eq!(code, 0, "{name}: mapping form should parse");
        let (in_sequence, code) = run_yq_stdin(
            ".a[0]",
            &format!("a: [? {shape}]\n"),
            &["-o", "json", "-I", "0"],
        )?;
        assert_eq!(code, 0, "{name}: sequence form should parse");

        assert_eq!(
            in_mapping.trim(),
            in_sequence.trim(),
            "{name}: the two call sites disagree"
        );
        assert_eq!(in_mapping.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_compact_entry_trailing_anchor_null_values_are_unchanged() -> Result<()> {
    // The other side of the same decision: when no value follows, the entry is
    // null and the anchor names an explicit empty node rather than dangling on
    // whatever BP bit comes next. These already passed and pin the boundary —
    // if one of them starts returning a collection, the fix has widened from
    // "the value is on the next line" to "the next line is the value".
    for (name, input, expected) in [
        ("EOF", "- k: &a\n", r#"[{"k":null}]"#),
        ("lower indent", "- k: &a\n- b\n", r#"[{"k":null},"b"]"#),
        (
            "same indent, not a sequence",
            "- k: &a\n  j: 2\n",
            r#"[{"k":null,"j":2}]"#,
        ),
        (
            "alias to the null anchor",
            "- k: &a\n  j: 2\n- *a\n",
            r#"[{"k":null,"j":2},null]"#,
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(output.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_compact_entry_anchor_names_the_nested_collection() -> Result<()> {
    // Identity output alone cannot tell "the anchor names the nested mapping"
    // from "the anchor names a scalar that happens to render the same", so
    // resolve it. Before the fix the alias propagated the collapsed scalar —
    // `[{"k":"b"},{"c":"b"}]` for the third case, self-consistent and wrong.
    for (name, input, expected) in [
        (
            "alias as a sequence item",
            "- k: &a\n    b: 1\n- *a\n",
            r#"[{"k":{"b":1}},{"b":1}]"#,
        ),
        (
            "alias as a block mapping value",
            "seq:\n  - k: &a\n      b: 1\nref: *a\n",
            r#"{"seq":[{"k":{"b":1}}],"ref":{"b":1}}"#,
        ),
        (
            "alias as another compact entry's value",
            "- k: &a\n    b: 1\n- c: *a\n",
            r#"[{"k":{"b":1}},{"c":{"b":1}}]"#,
        ),
        (
            "anchored sequence",
            "- k: &a\n    - 1\n- *a\n",
            r#"[{"k":[1]},[1]]"#,
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(output.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_every_block_value_site_consumes_a_trailing_anchor() -> Result<()> {
    // The bug was that one of the four block-context value sites decided where
    // the value was *before* consuming the anchor, and the other three did not.
    // One input shape — `&a` last on its line, the value indented below, an
    // alias to it — through all four. Pinning them together is what stops the
    // outlier coming back: a copy that regresses fails here even if its own
    // section's tests are deleted.
    for (name, input, expected) in [
        (
            "block mapping value (parse_mapping_entry)",
            "k: &a\n  b: 1\nc: *a\n",
            r#"{"k":{"b":1},"c":{"b":1}}"#,
        ),
        (
            "compact mapping value (parse_compact_mapping_entry)",
            "- k: &a\n    b: 1\n- *a\n",
            r#"[{"k":{"b":1}},{"b":1}]"#,
        ),
        (
            "sequence item (parse_sequence_item_inner)",
            "- &a\n  b: 1\n- *a\n",
            r#"[{"b":1},{"b":1}]"#,
        ),
        (
            "explicit value (parse_explicit_value)",
            "? k\n: &a\n  b: 1\nz: *a\n",
            r#"{"k":{"b":1},"z":{"b":1}}"#,
        ),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(output.trim(), expected, "{name}");
    }
    Ok(())
}

#[test]
fn test_yaml_compact_entry_trailing_anchor_is_line_break_agnostic() -> Result<()> {
    // `at_line_end` is now consulted twice per entry rather than once, and it
    // is one of the predicates #324 taught to stop at `\r`. All three break
    // forms are the same document per YAML 1.2 §5.4.
    for (name, input) in [
        ("LF", "- k: &a\n    b: 1\n- *a\n"),
        ("CRLF", "- k: &a\r\n    b: 1\r\n- *a\r\n"),
        ("CR", "- k: &a\r    b: 1\r- *a\r"),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
        assert_eq!(exit_code, 0, "{name} input should parse");
        assert_eq!(
            output.trim(),
            r#"[{"k":{"b":1}},{"b":1}]"#,
            "{name} line breaks should give the same document"
        );
    }
    Ok(())
}

#[test]
fn test_yaml_flow_question_mark_without_a_space_is_content() -> Result<()> {
    // The boundary the fix must not cross. `?` is an indicator only when
    // whitespace or end-of-input follows it; otherwise it starts no node and is
    // ordinary scalar text. If one of these starts losing the `?`, the check
    // moved from "node begins with an explicit-key indicator" to "node contains
    // a `?`".
    //
    // The first two are yq v4.53.3 output. The third is not: yq rejects a `?`
    // inside a flow plain scalar, while the YAML Test Suite says it is content
    // (JR7V, "Question marks in scalars"), and that corpus case passes. It is
    // pinned here against the suite, not against yq.
    for (name, input, expected) in [
        ("unspaced key", "a: {?k : v}\n", r#"{"a":{"?k":"v"}}"#),
        ("bare question mark", "a: {?}\n", r#"{"a":{"?":null}}"#),
        (
            "inside a plain scalar",
            "a: [b ? c]\n",
            r#"{"a":["b ? c"]}"#,
        ),
    ] {
        let (stdout, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I", "0"])?;
        assert_eq!(exit_code, 0, "{name}: should parse cleanly");
        assert_eq!(stdout.trim(), expected, "{name}");
    }
    Ok(())
}

// =============================================================================
// `#` as a comment start in flow-mapping keys (#437). `parse_flow_unquoted_key`
// had no `#` arm at all, so a comment folded into the key text instead of
// erroring — unlike the block-key path (#410) and the flow-*value* path, both
// of which already treat a whitespace-preceded `#` as a comment.
// =============================================================================

#[test]
fn test_yaml_flow_key_comment_requires_preceding_whitespace() -> Result<()> {
    for (name, input) in [
        ("space before hash, implicit key", "{a # b: c}\n"),
        ("tab before hash, implicit key", "{a\t# b: c}\n"),
    ] {
        let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".", input, &[])?;
        assert_eq!(exit_code, 1, "{name}: expected clean error exit: {stderr}");
        assert_eq!(stdout, "", "{name}: nothing should reach stdout");
        assert!(
            stderr.contains("key without value"),
            "{name}: stderr should name the missing value: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn hash_without_preceding_space_is_flow_key_content() -> Result<()> {
    // The boundary the #437 fix must not cross: `#` not preceded by a space or
    // tab is ordinary key content, exactly as it already is in block context
    // (`a#b: value`).
    let (stdout, exit_code) = run_yq_stdin(".", "{a#b: c}\n", &["-o", "json", "-I", "0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), r#"{"a#b":"c"}"#);
    Ok(())
}

#[test]
fn test_yaml_flow_explicit_key_colon_before_a_flow_indicator_is_content() -> Result<()> {
    // A deliberate divergence from YAML 1.2, pinned here because nothing else
    // can catch it moving.
    //
    // In an explicit flow key, `:` ends the key only before a blank, a break or
    // end of input; before a flow indicator it is content. That is yq's rule
    // (see `test_yaml_explicit_flow_key_scalar_shapes`), and it contradicts
    // §7.3.3, under which `:` before `,`/`}`/`]` is the value indicator.
    //
    // The visible cost is spec example 7.3, YAML Test Suite case FRK4, whose
    // first key the spec reads as `foo` and we now read as `foo :`. yq rejects
    // that document outright, so there is no yq answer to agree with. FRK4 is a
    // parses-only corpus case (`json: null`), so `yaml_test_suite`'s manifest
    // will not notice the change — this assertion is the only guard. #402.
    let (stdout, exit_code) = run_yq_stdin(
        ".",
        "{\n  ? foo :,\n  : bar,\n}\n",
        &["-o", "json", "-I", "0"],
    )?;
    assert_eq!(exit_code, 0, "FRK4 must still parse");
    assert_eq!(stdout.trim(), r#"{"foo :":null,"":"bar"}"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_flow_key_comment_requires_preceding_whitespace() -> Result<()> {
    // #437: `parse_explicit_flow_unquoted_key` has the same shape as
    // `parse_flow_unquoted_key` and was missing the same `#` arm — a comment
    // inside a `? key : value` flow key folded into the key text instead of
    // erroring.
    let (stdout, stderr, exit_code) = run_yq_stdin_with_stderr(".", "{? a # b : c}\n", &[])?;
    assert_eq!(exit_code, 1, "expected clean error exit: {stderr}");
    assert_eq!(stdout, "", "nothing should reach stdout");
    assert!(
        stderr.contains("key without value"),
        "stderr should name the missing value: {stderr}"
    );
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
fn test_yaml_explicit_key_sequence_item_compact_mapping_second_field_877() -> Result<()> {
    // A different shape from the headline repro above: the key sequence's
    // first item is itself a compact mapping (`? - a: 1`, not `? - a`). This
    // arm hardcoded `indent + 3` for the compact mapping's own indent - wrong
    // even for ordinary single-space spacing (`?` + ` ` + `-` + ` ` is 4
    // columns, not 3) - so the second field silently landed at the wrong
    // indent and the `: value` line was swallowed into it instead of closing
    // the entry. A single-field compact mapping here never exercised the bug
    // (nothing to land at the wrong indent), which is why it takes a second
    // field to distinguish this from #877's own primary repro.
    let input = "? - a: 1\n    b: 2\n: value\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"":"value"}"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_key_sequence_item_compact_mapping_extra_spaces_877() -> Result<()> {
    // Combines the two #877 shapes: extra spaces after the nested `-`, on
    // top of the explicit-key wrapper.
    let input = "? -   a: 1\n      b: 2\n: value\n";
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

#[test]
fn test_yaml_anchored_first_item_of_explicit_key_sequence_binds() -> Result<()> {
    // `? - &a 1` anchors the sequence's first item inline, not the `?` key as a
    // whole. This is the one call site that reaches `parse_value` with the
    // anchor still at the cursor (every other caller strips it first), so it's
    // the sole remaining coverage for that branch.
    let input = "? - &a 1\n: b\nc: *a\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"":"b","c":1}"#);
    Ok(())
}

#[test]
fn test_yaml_explicit_non_scalar_key_dash_tab_separated() -> Result<()> {
    // Same construct as `test_yaml_explicit_non_scalar_key_headline_repro`
    // (`? - a\n  - b\n: value`), but with a tab instead of a space after each
    // `-`. `parse_explicit_key`'s inline dispatch on the key's first byte
    // matched `Some(b'-') if matches!(self.peek_at(1), Some(b' ' | b'\n' |
    // b'\r') | None)` - missing the tab that every sibling `-` check in this
    // file, and the canonical `is_seq_indicator_next` (#332), already
    // include. Before the fix, `-\ta` fell through to being parsed as a
    // plain scalar key instead of a sequence key, and the second item and
    // the value were lost entirely: `{"-\ta":["b"]}` instead of
    // `{"":"value"}` (#434).
    let input = "? -\ta\n  -\tb\n: value\n";
    let (output, exit_code) = run_yq_stdin(".", input, &["-o=json", "-I=0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output.trim(),
        r#"{"":"value"}"#,
        "the tab form must parse the same document as the space form"
    );
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
// Compatibility tests - Block scalar style preservation on re-serialization
// (#836: the M2 streaming/cursor path used to lose `|`/`>` entirely and
// re-emit a double-quoted string with `\n` escapes instead)
// =============================================================================

#[test]
fn test_block_scalar_literal_style_preserved_as_sequence_item() -> Result<()> {
    // Exact repro from #836.
    let input = "- |\n  line1\n  line2\n- next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "- |\n  line1\n  line2\n- next\n");
    Ok(())
}

#[test]
fn test_block_scalar_folded_style_preserved_as_sequence_item() -> Result<()> {
    // Exact repro from #836. Real yq's own re-encoding always adds an extra
    // blank line after a folded scalar with clip/keep chomping (verified
    // against the pinned yq v4.53.3 oracle) - not a succinctly gap, so
    // replicated rather than "fixed" into a new divergence.
    let input = "- >\n  line1\n  line2\n- next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "- >\n  line1 line2\n\n- next\n");
    Ok(())
}

#[test]
fn test_block_scalar_literal_style_preserved_as_mapping_field() -> Result<()> {
    let input = "a: |\n  line1\n  line2\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: |\n  line1\n  line2\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_folded_style_preserved_as_mapping_field() -> Result<()> {
    let input = "a: >\n  line1\n  line2\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: >\n  line1 line2\n\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_literal_strip_style_preserved() -> Result<()> {
    let input = "a: |-\n  line1\n  line2\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: |-\n  line1\n  line2\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_literal_keep_style_preserved() -> Result<()> {
    let input = "a: |+\n  line1\n  line2\n\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: |+\n  line1\n  line2\n\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_folded_keep_style_preserved() -> Result<()> {
    let input = "a: >+\n  line1\n  line2\n\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    // Chomping suffix (`+`) is chosen from the decoded value's own trailing
    // newline count, not copied from the source's chomping indicator - see
    // `chomping_indicator` in src/yaml/light.rs.
    assert_eq!(output, "a: >+\n  line1 line2\n\n\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_folded_embedded_blank_line_widened() -> Result<()> {
    // A single `\n` between two equally-indented lines folds back to a
    // space on re-parse (YAML 1.2 §8.1.3) - an *embedded* blank line (not
    // the scalar's own trailing one) has to be widened to a real blank
    // line (two breaks) for its fold-preserving `\n` to survive, exercising
    // `widen_folded_breaks`'s embedded-run branch specifically (its
    // trailing-run branch is already covered by the clip/keep tests
    // above).
    let input = "a: >\n  line1\n\n  line2\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: >\n  line1\n\n  line2\n\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_empty_falls_back_to_quoted_string() -> Result<()> {
    // An empty block scalar has no lines to re-indent under `|`/`>` - real
    // yq drops block style here too (verified against the pinned oracle),
    // so falling through to normal quoting rather than emitting `|-`/`>-`
    // with nothing after it matches, not diverges.
    let input = "a: |\n  \nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: \"\"\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_style_preserved_through_slurp() -> Result<()> {
    // The `--slurp` fast path (`stream_yaml_sequence`) delegates each
    // element to the same `stream_yaml_value` this fix changes, rather than
    // duplicating its own String-rendering arm - so it inherits the fix
    // without any changes of its own.
    let dir = TempDir::new()?;
    let file1 = dir.path().join("a.yaml");
    std::fs::write(&file1, "- |\n  line1\n  line2\n")?;
    let cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("--slurp")
        .arg(".")
        .arg(&file1)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let output = cmd.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    assert_eq!(stdout, "- - |\n    line1\n    line2\n");
    Ok(())
}

#[test]
fn test_block_scalar_explicit_indent_indicator_preserved() -> Result<()> {
    // A source explicit indent indicator (`|2`) means the content's first
    // line has 2 literal leading spaces of its own, beyond the 2-space
    // structural indent - decoded value is "  foo\nbar\n". Dropping the
    // indicator on re-emission and relying on auto-detection would make a
    // re-parse see 4 leading spaces on "foo" as the (wrongly) detected
    // content indent, then dedent-terminate the scalar at "bar" (2 spaces
    // < 4) - silently truncating the value and misparsing the rest of the
    // document as a new mapping entry. This was a real, confirmed
    // corruption bug found in review, not just a style/byte mismatch.
    let input = "a: |2\n    foo\n  bar\nc: after\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: |2\n    foo\n  bar\nc: after\n");
    // Round-trip through succinctly's own JSON decoder to confirm no
    // corruption, not just a byte match against this one fixture.
    let (decoded, code2) = run_yq_stdin(".", &output, &["-o", "json", "-I0"])?;
    assert_eq!(code2, 0);
    assert_eq!(decoded.trim(), "{\"a\":\"  foo\\nbar\\n\",\"c\":\"after\"}");
    Ok(())
}

#[test]
fn test_block_scalar_explicit_indent_folded_more_indented_lines_not_widened() -> Result<()> {
    // Every content line under an explicit indent indicator commonly
    // carries its own extra leading whitespace (that's usually *why* the
    // source needed the indicator at all) - making every line
    // "more-indented" per YAML 1.2 §8.1.3, which already blocks folding
    // between them without any widening. `widen_folded_breaks` must not
    // treat this array's embedded `\n` the same as a plain (non-indented)
    // one - real yq keeps the single break as-is (found in review: an
    // earlier version of this fix over-widened here, producing an extra
    // blank line real yq does not).
    let input = "a: >2\n    foo\n    bar\nc: after\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: >2\n    foo\n    bar\nc: after\n");
    Ok(())
}

#[test]
fn test_block_scalar_explicit_indent_trailing_run_not_widened() -> Result<()> {
    // Auto-detected folded style writes its trailing run one `\n` wider
    // than the decoded value's own count (see
    // `test_block_scalar_folded_style_preserved_as_mapping_field`) -
    // that "+1" quirk is specific to auto-detection; an explicit-indent
    // folded scalar's trailing run must NOT get the same treatment (found
    // in review: an earlier version of this fix always widened it,
    // producing a spurious blank line before the next sibling that real
    // yq does not).
    let input = "a: >2+\n    foo\n    bar\n\nc: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: >2+\n    foo\n    bar\n\nc: next\n");
    Ok(())
}

/// #852: `.a` navigates to a single scalar and makes it the *whole*
/// output - a bare top-level/navigated scalar root drops ALL of its own
/// styling (quotes, `|`/`>` block-scalar indicators), matching real `yq`
/// exactly, verified against the pinned binary: `foo\nbar\n` prints as
/// bare `foo` / `bar` lines, no quoting, no `|` indicator.
///
/// This superseded an earlier "fall back to quoting" workaround (dropped
/// here) that avoided a then-real data-loss bug: re-emitting block-style
/// content at the root's empty indent has no structural indentation,
/// which isn't valid block-scalar syntax and used to silently decode back
/// to `""` on re-parse. Bypassing `stream_yaml_value`'s styling logic
/// entirely at the root (rather than trying to re-emit block style there)
/// sidesteps that indentation problem by construction - there's no block
/// scalar being written at all. The round-trip is still not lossless, but
/// only in the way real `yq`'s own output is: two plain-scalar lines fold
/// into one space-joined line on YAML re-parse (`foo\nbar` -> `foo bar`),
/// which is YAML's own plain-scalar folding rule, not a succinctly bug -
/// confirmed identical against the pinned real `yq` binary's own
/// round-trip.
#[test]
fn test_block_scalar_bare_top_level_projection_drops_all_styling_852() -> Result<()> {
    let input = "a: |\n  foo\n  bar\nc: 1\n";
    let (output, exit_code) = run_yq_stdin(".a", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "foo\nbar\n\n");
    // Not lossless, but matches real yq's own round-trip exactly (YAML's
    // plain-scalar line-folding rule, not a succinctly-specific bug).
    let (decoded, code2) = run_yq_stdin(".", &output, &["-o", "json"])?;
    assert_eq!(code2, 0);
    assert_eq!(decoded.trim(), "\"foo bar\"");
    Ok(())
}

#[test]
fn test_block_scalar_astral_character_falls_back_to_quoted() -> Result<()> {
    // Real yq always quotes a block scalar containing a supplementary-
    // plane character (U+10000+, e.g. most emoji) rather than keep it
    // block-styled (found in review, verified against the pinned oracle).
    // Our own quoting doesn't escape it the way real yq's `\U` form does
    // (a separate, pre-existing gap in `stream_yaml_double_quoted`
    // predating this fix, unrelated to block scalars specifically) - this
    // test only pins the *style* choice (quoted, not block), not the
    // escape bytes.
    let input = "a: |\n  emoji \u{1F389} here\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: \"emoji \u{1F389} here\\n\"\nb: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_bmp_character_keeps_block_style() -> Result<()> {
    // Contrast with the astral test above: a BMP character (here CJK,
    // U+65E5, well under U+10000) does not disqualify block style.
    let input = "a: |\n  cjk \u{65E5}\u{672C}\u{8A9E} here\nb: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output,
        "a: |\n  cjk \u{65E5}\u{672C}\u{8A9E} here\nb: next\n"
    );
    Ok(())
}

#[test]
fn test_block_scalar_explicit_indent_and_trailing_space_both_disqualify() -> Result<()> {
    // A content line can need an explicit indent indicator (leading
    // whitespace of its own) *and* independently be trailing-space
    // disqualified at the same time - the two checks are independent
    // conditions on the same decoded value, and this drives both true at
    // once to make sure the (already-computed) explicit-indent digit
    // doesn't leak into the quoted fallback path.
    let input = "a: |2\n    foo \n  bar\nc: after\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: \"  foo \\nbar\\n\"\nc: after\n");
    Ok(())
}

#[test]
fn test_block_scalar_bare_top_level_projection_of_plain_safe_content_stays_unquoted() -> Result<()>
{
    // Contrast with `..._falls_back_to_quoted` above: a bare top-level
    // projection whose decoded value doesn't need quoting for any other
    // reason (single line, strip chomping, no leading/trailing
    // whitespace, not keyword/number-looking) comes out completely bare,
    // not "-safe-but-still-quoted" - `stream_yaml_block_scalar_quoted`
    // makes its own independent `needs_yaml_quoting` decision once block
    // style is ruled out, it doesn't force quoting just because block
    // style was.
    let input = "a: |-\n  hello\nb: 2\n";
    let (output, exit_code) = run_yq_stdin(".a", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "hello\n");
    Ok(())
}

#[test]
fn test_block_scalar_folded_explicit_indent_single_line_no_embedded_runs() -> Result<()> {
    // A single physical content line under an explicit indent indicator
    // has nothing to fold against, so `widen_folded_breaks` takes its
    // no-embedded-runs fast path (`Cow::Borrowed`, narrowing rather than
    // reallocating) while still needing its trailing `\n` reduced by one
    // (`reduce_trailing`, since an indicator was written) - distinct from
    // the multi-line explicit-indent tests above, which all go through
    // the full scanning path instead.
    let input = "a: >2\n    foo\nc: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: >2\n    foo\nc: next\n");
    Ok(())
}

#[test]
fn test_block_scalar_folded_strip_chomping_embedded_blank_line_still_widened() -> Result<()> {
    // Found by a systematic style x chomping x context sweep against the
    // pinned oracle: an earlier version of `widen_folded_breaks` computed
    // "is there an embedded run to widen" as `trailing_run_start !=
    // decoded.len() && ...`, which was meant as a quick "is there a
    // trailing run at all" shortcut but instead skipped the whole
    // embedded-run scan whenever there was NO trailing run - exactly the
    // strip-chomping case (`decoded` doesn't end in `\n`, so
    // `trim_end_matches('\n')` is a no-op and `trailing_run_start ==
    // decoded.len()`). A folded scalar with strip chomping AND an
    // embedded blank line silently lost that blank line's widening,
    // collapsing "foo\n\nbar" back down to "foo\nbar" - which decodes to
    // a different value than the source ("foo\nbar" folds its break away
    // entirely, versus the two breaks needed to preserve it, #836 review).
    let input = "a: >-\n  foo\n\n  bar\nc: after\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: >-\n  foo\n\n  bar\nc: after\n");
    Ok(())
}

#[test]
fn test_block_scalar_folded_auto_detect_trailing_run_reduced_when_last_line_more_indented(
) -> Result<()> {
    // The auto-detect folded "+1" trailing quirk (see
    // `test_block_scalar_folded_style_preserved_as_mapping_field`) turned
    // out to have a second trigger condition, found by a sweep: it fires
    // only when *neither* an explicit indent digit was written *nor* the
    // last content line before the trailing run is itself more-indented.
    // This case isolates that second condition alone (no explicit
    // indicator at all - "foo" auto-detects the indent - but "bar" is
    // more-indented relative to it), which an earlier version of this fix
    // missed (it kept the trailing quirk regardless of the last line's
    // own indentation, adding a spurious extra blank line, #836 review).
    let input = "a: >+\n  foo\n    bar\n\nc: next\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "a: >+\n  foo\n    bar\n\nc: next\n");
    // The critical property: round-tripping preserves the decoded value.
    let (decoded, code2) = run_yq_stdin(".", &output, &["-o", "json", "-I0"])?;
    assert_eq!(code2, 0);
    assert_eq!(
        decoded.trim(),
        "{\"a\":\"foo\\n  bar\\n\\n\",\"c\":\"next\"}"
    );
    Ok(())
}

#[test]
fn test_block_scalar_folded_mixed_indentation_transition_stays_lossless() -> Result<()> {
    // A deliberate, documented divergence from the pinned oracle (see
    // `widen_folded_breaks`'s own doc comment): real yq's encoder widens
    // an embedded run between a plain line and a more-indented one when
    // the *plain* line comes first, but not when it comes second - a
    // genuine, confirmed-non-idempotent bug (round-tripping through real
    // yq `.` twice injects a blank line the first pass's own decoded
    // value never had), not a style choice worth replicating. This test
    // pins that this crate's own output stays lossless across that exact
    // transition shape instead of reproducing the asymmetric bug.
    let input = "a: >+\n  qux\n    foo\n\nc: after\n";
    let (output, exit_code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(exit_code, 0);
    let (decoded, code2) = run_yq_stdin(".", &output, &["-o", "json", "-I0"])?;
    assert_eq!(code2, 0);
    assert_eq!(
        decoded.trim(),
        "{\"a\":\"qux\\n  foo\\n\\n\",\"c\":\"after\"}"
    );
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

/// #748: `-C`/`--color` excluded identity/navigation queries from the M2
/// cursor-streaming fast path (`can_stream_pretty` in `yq_runner.rs`),
/// forcing them through the `OwnedValue` DOM (`IndexMap`-backed), which
/// silently collapsed duplicate keys — the same bug class #442 fixed for the
/// unadorned fast path and #733 fixed for `-S`/`--tab`. Unlike #733, color
/// doesn't need the streamers taught anything new: `colorize_yaml`/
/// `colorize_json` are pure text-level re-lexers, so the fix buffers the
/// still-duplicate-key-safe cursor-streamed output and colorizes the buffer,
/// reusing the existing colorizers unmodified.
#[test]
fn test_duplicate_mapping_key_survives_color_yaml_output() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-C"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[36ma\u{1b}[0m: 1\n\u{1b}[36ma\u{1b}[0m: 2\u{1b}[0m\n"
    );

    Ok(())
}

/// Same as [`test_duplicate_mapping_key_survives_color_yaml_output`], for
/// `-o json`.
#[test]
fn test_duplicate_mapping_key_survives_color_json_output() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-C", "-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[1;39m{\u{1b}[0m\n  \u{1b}[1;34m\"a\"\u{1b}[0m: \u{1b}[0;39m1\u{1b}[0m,\n  \u{1b}[1;34m\"a\"\u{1b}[0m: \u{1b}[0;39m2\u{1b}[0m\n\u{1b}[1;39m}\u{1b}[0m\n"
    );

    Ok(())
}

/// Compact output (`-I0`) already took the fast path regardless of color
/// (`can_json_fast_path`/`can_yaml_fast_path`'s `output_config.compact ||`
/// short-circuit predates #748), which meant `-C -I0` silently produced
/// uncolored output — a separate quirk from the duplicate-key collapse this
/// issue is about, but resolved as a side effect since #748's fix keys the
/// buffer-and-colorize decision only on `use_color`, not on which condition
/// let the fast path through. Guards both the color and duplicate-key fixes
/// together in compact mode, YAML and JSON.
#[test]
fn test_duplicate_mapping_key_survives_color_compact() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-C", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[36ma\u{1b}[0m: 1\n\u{1b}[36ma\u{1b}[0m: 2\u{1b}[0m\n"
    );

    let (json, code) = run_yq_stdin(".", yaml, &["-C", "-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(
        json,
        "\u{1b}[1;39m{\u{1b}[0m\u{1b}[1;34m\"a\"\u{1b}[0m:\u{1b}[0;39m1\u{1b}[0m,\u{1b}[1;34m\"a\"\u{1b}[0m:\u{1b}[0;39m2\u{1b}[0m\u{1b}[1;39m}\u{1b}[0m\n"
    );

    Ok(())
}

/// `stream_maybe_colored` (#748) buffers a whole `stream_yaml`/`stream_json`
/// call's output — which, for an iterating query, can be several
/// concatenated top-level documents, not just one — before handing it to
/// `colorize_yaml`/`colorize_json`. Neither colorizer is exercised on
/// multi-document buffers anywhere else (the DOM path colorizes one value
/// per call), so this guards that the re-lexers still track state correctly
/// across a document boundary instead of only ever seeing a single value.
#[test]
fn test_color_output_survives_iteration_with_duplicate_keys() -> Result<()> {
    let yaml = "- a: 1\n  a: 2\n- b: 3\n";

    let (output, code) = run_yq_stdin(".[]", yaml, &["-C"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[36ma\u{1b}[0m: 1\n\u{1b}[36ma\u{1b}[0m: 2\n\u{1b}[36mb\u{1b}[0m: 3\n\u{1b}[0m"
    );

    Ok(())
}

/// Same as [`test_color_output_survives_iteration_with_duplicate_keys`], for
/// `-o json`: the `stream_maybe_colored` call in the non-identity JSON
/// branch of `stream_cursor!` (`result.stream_json`, used for anything other
/// than plain `.`) is otherwise only exercised by identity queries.
#[test]
fn test_color_output_survives_iteration_with_duplicate_keys_json() -> Result<()> {
    let yaml = "- a: 1\n  a: 2\n- b: 3\n";

    let (output, code) = run_yq_stdin(".[]", yaml, &["-C", "-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[1;39m{\u{1b}[0m\n  \u{1b}[1;34m\"a\"\u{1b}[0m: \u{1b}[0;39m1\u{1b}[0m,\n  \u{1b}[1;34m\"a\"\u{1b}[0m: \u{1b}[0;39m2\u{1b}[0m\n\u{1b}[1;39m}\u{1b}[0m\n\u{1b}[1;39m{\u{1b}[0m\n  \u{1b}[1;34m\"b\"\u{1b}[0m: \u{1b}[0;39m3\u{1b}[0m\n\u{1b}[1;39m}\u{1b}[0m\n"
    );

    Ok(())
}

/// #809: `--slurp -C` used to intentionally route through the `OwnedValue`/
/// `IndexMap` DOM path — `can_slurp_fast_path` only checked
/// `can_stream_pretty`, not `can_stream_pretty_or_colored`, since
/// `stream_yaml_sequence` never got `stream_maybe_colored` support (a
/// documented scope limit, not a silent gap — #748). That collapsed
/// duplicate mapping keys within each slurped document, unlike plain
/// `--slurp` (see [`test_duplicate_mapping_key_survives_slurp`]). Fixed by
/// wrapping the `stream_yaml_sequence` call in `stream_maybe_colored`,
/// mirroring the stdout path's own fix — `stream_yaml_sequence` needed no
/// changes itself, since it's already generic over `core::fmt::Write`.
#[test]
fn test_slurp_color_output_yaml() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["--slurp", "-C"])?;
    assert_eq!(code, 0);
    // Renders in real yq's "compact" form (#785): `- ` shares its line
    // with the mapping's own first field. `colorize_yaml` is a simple
    // single-pass text colorizer (its own scheme, not oracle-matched
    // against real yq's `-C`, which uses entirely different codes/colors)
    // whose `at_key_start` flag was never previously reachable directly
    // after a list marker - compact form is the first shape that puts a
    // key there instead of a value or a newline. It now peeks ahead
    // (`compact_item_opens_with_key`) to tell a compact key from a bare
    // scalar value in that position before deciding whether to color it,
    // so this first "a" gets its cyan key coloring too, matching every
    // other key in the document.
    assert_eq!(
        output,
        "\u{1b}[33m-\u{1b}[0m \u{1b}[36ma\u{1b}[0m: 1\n  \u{1b}[36ma\u{1b}[0m: 2\u{1b}[0m\n"
    );

    Ok(())
}

/// `compact_item_opens_with_key`'s quote-aware lookahead must skip a `:`
/// that's inside a quoted compact-form key, not treat it as the
/// key-indicating colon - and must still resolve `at_key_start` correctly
/// once the quote closes and the *real* key-ending `:` follows. The
/// quoted key itself renders green either way (`colorize_yaml`'s `"`/`'`
/// arm colors it unconditionally, independent of `at_key_start`), so this
/// doesn't change what's visible - it pins the lookahead's own quote
/// entry/exit and in-quote-skip behavior directly, which no other test
/// exercises.
#[test]
fn test_color_compact_quoted_key_with_colon_785() -> Result<()> {
    let yaml = "- \"x: y\": 1\n  b: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-C"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[33m-\u{1b}[0m \u{1b}[32m\"x: y\"\u{1b}[0m: 1\n  \u{1b}[36mb\u{1b}[0m: 2\u{1b}[0m\n"
    );

    Ok(())
}

/// Same as [`test_slurp_color_compact_quoted_key_with_colon_785`], for an
/// escaped quote inside the compact-form quoted key - pins the lookahead's
/// escape-skip branch (`if c == '\\' { chars.next(); }`), which the
/// unescaped case above doesn't reach.
#[test]
fn test_color_compact_quoted_key_with_escaped_quote_785() -> Result<()> {
    let yaml = "- \"a\\\"b: c\": 1\n  d: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["-C"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[33m-\u{1b}[0m \u{1b}[32m\"a\\\"b: c\"\u{1b}[0m: 1\n  \u{1b}[36md\u{1b}[0m: 2\u{1b}[0m\n"
    );

    Ok(())
}

/// Unlike [`test_slurp_color_output_yaml`], `-o json --slurp` stays on the
/// `OwnedValue`/`IndexMap` DOM path regardless of `-C` — `can_slurp_fast_path`
/// requires YAML output, so `-o json --slurp` is unaffected by #809's fix and
/// still collapses duplicate keys, exactly as it did (with or without color)
/// before #809. This is the same pre-existing, separately-scoped `-o json
/// --slurp` limitation the `can_slurp_fast_path` code comment documents, not
/// a `-C`-specific gap. Exercises `output_value`'s `config.use_color` JSON
/// branch, which #748's M2-fast-path color fix made unreachable from every
/// other angle.
#[test]
fn test_slurp_color_output_json() -> Result<()> {
    let yaml = "a: 1\na: 2\n";

    let (output, code) = run_yq_stdin(".", yaml, &["--slurp", "-C", "-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(
        output,
        "\u{1b}[1;39m[\u{1b}[0m\u{1b}[1;39m{\u{1b}[0m\u{1b}[1;34m\"a\"\u{1b}[0m:\u{1b}[0;39m2\u{1b}[0m\u{1b}[1;39m}\u{1b}[0m\u{1b}[1;39m]\u{1b}[0m\n"
    );

    Ok(())
}

/// #809 follow-up: before this fix, `--slurp -C`'s identity query was the
/// only thing exercising `output_value`'s YAML `config.use_color` branch.
/// Once `can_slurp_fast_path` started accepting color (this PR), that query
/// moved onto the new fast path and stopped covering it. `--null-input` has
/// no cursor to stream from, so it bypasses the M2 fast path entirely
/// regardless of query shape and keeps hitting `output_value` — pinning it
/// here so the DOM-path YAML color branch stays covered.
#[test]
fn test_null_input_color_output_yaml() -> Result<()> {
    let (output, code) = run_yq_stdin("{a: 1}", "", &["-n", "-C"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "\u{1b}[36ma\u{1b}[0m: 1\u{1b}[0m\n");
    Ok(())
}

/// #809: `-C --inplace` fell through to the `OwnedValue`/`IndexMap` DOM
/// path for any non-compact indent (`can_inplace_yaml_fast_path` excluded
/// color via `can_stream_pretty`), collapsing duplicate keys — mirrors
/// [`test_duplicate_mapping_key_survives_inplace`], plus `-C`. `--inplace`
/// still never writes ANSI to the file even once the fast path is taken:
/// the fast-path branch passes `false` as `stream_cursor!`'s `$use_color`
/// argument explicitly, since a bare `output_config.use_color` reference
/// inside that macro resolves against the *original* binding from where the
/// macro was defined, not a later same-named shadow at the call site — see
/// the code comment above `macro_rules! stream_cursor` in `yq_runner.rs`.
#[test]
fn test_duplicate_mapping_key_survives_color_inplace() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\na: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("-C")
        .arg(".")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "a: 1\na: 2\n");
    assert!(
        !rewritten.contains('\u{1b}'),
        "inplace must never write ANSI color codes to disk"
    );
    Ok(())
}

/// Same as [`test_duplicate_mapping_key_survives_color_inplace`], for
/// `-o json`.
#[test]
fn test_duplicate_mapping_key_survives_color_inplace_json_output() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\na: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("-C")
        .arg("-o")
        .arg("json")
        .arg(".")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "{\n  \"a\": 1,\n  \"a\": 2\n}\n");
    assert!(
        !rewritten.contains('\u{1b}'),
        "inplace must never write ANSI color codes to disk"
    );
    Ok(())
}

/// #809 bonus finding: compact output (`-I0`) already took `--inplace`'s
/// fast path unconditionally, since the gate is `compact || (color-aware
/// condition)` and `compact ||` short-circuits before color is ever
/// checked. Before this fix, that meant `-C -I0 --inplace` wrote raw ANSI
/// escape bytes straight into the file — worse than the non-compact case,
/// which never colored its (collapsed) DOM-path output. Regression test:
/// compact + color + inplace must never leak ANSI into the file.
#[test]
fn test_inplace_color_compact_does_not_write_ansi_to_file() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1\na: 2")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("-C")
        .arg("-I0")
        .arg(".")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(rewritten, "a: 1\na: 2\n");
    assert!(
        !rewritten.contains('\u{1b}'),
        "inplace must never write ANSI color codes to disk"
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

/// #939: fixed `keys`/`.[]`-style previews of a document-sourced overflow
/// *number* literal (`123e400`) to reuse the literal's own text instead of
/// `OwnedValue::to_json_for_reindex`'s generic `1e999` sentinel. YAML's
/// `.inf`/`-.inf` special tokens never construct `OwnedValue::NumberLiteral`
/// at all (only JSON's own parsing path does - see `to_json_for_reindex`'s
/// doc comment), so they take the unrelated, unmodified `OwnedValue::Float`
/// arm regardless of this fix; pinning here that the sentinel text they've
/// always used stays unchanged.
#[test]
fn test_yaml_special_float_keys_preview_is_unaffected_by_939() -> Result<()> {
    let (output, exit_code) = run_yq_stdin("try keys catch .", ".inf\n", &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "number (1E+999) has no keys\n");

    let (output, exit_code) = run_yq_stdin("try keys catch .", "-.inf\n", &[])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output, "number (-1E+999) has no keys\n");
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
    // No leading `---` before the first file's result, matching real yq —
    // the separator only appears *between* documents (#442 routed this
    // non-compact multi-file case through the M2 fast path's
    // `emit_yaml_doc_separator`, which corrects a pre-existing divergence
    // from the DOM/slow path that had emitted a spurious leading `---`).
    assert_eq!(output, "1\n---\n2\n");
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
// line / column builtins (#532) — position-based navigation on the default
// (cursor-preserving) YAML CLI path. Before this fix, `line`/`column`
// resolved correctly only when called with zero preceding navigation
// (`line` alone); anything downstream of `.foo`/`.[]`/`select(...)` silently
// returned 0. No prior CLI-level test exercised these builtins through a
// real pipeline — see the issue for the full root-cause writeup.
// ============================================================================

#[test]
fn test_line_iterate_over_sequence() -> Result<()> {
    // The issue's exact repro.
    let yaml = "a: 1\nb: 2\nc: 3\n";
    let (output, code) = run_yq_stdin(".[] | line", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "1\n2\n3\n");
    Ok(())
}

#[test]
fn test_line_field_access() -> Result<()> {
    let yaml = "other: 1\nfoo: bar\n";
    let (output, code) = run_yq_stdin(".foo | line", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "2");
    Ok(())
}

#[test]
fn test_column_field_access() -> Result<()> {
    let yaml = "foo: bar\n";
    let (output, code) = run_yq_stdin(".foo | column", yaml, &[])?;
    assert_eq!(code, 0);
    // "foo: bar" -> "bar" starts at column 6.
    assert_eq!(output.trim(), "6");
    Ok(())
}

#[test]
fn test_line_select_filters_and_keeps_position() -> Result<()> {
    let yaml = "- 1\n- 2\n- 3\n";
    let (output, code) = run_yq_stdin(".[] | select(. > 1) | line", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "2\n3\n");
    Ok(())
}

#[test]
fn test_line_column_object_construction_is_a_known_limitation() -> Result<()> {
    // Object/array construction (`{...}`/`[...]`) isn't natively cursor-aware
    // in the generic evaluator — it round-trips through OwnedValue, which
    // has nowhere to carry a position. Documented limitation (#532), pinned
    // here so a future fix is visible as an intentional test change rather
    // than a silent behavior shift.
    let yaml = "foo: bar\nbaz: qux\n";
    let (output, code) = run_yq_stdin(".baz | {l: line, c: column}", yaml, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"l":0,"c":0}"#);
    Ok(())
}

#[test]
fn test_line_dot_chain_through_nested_iteration() -> Result<()> {
    // `.containers[].image` parses as its own nested Pipe distinct from the
    // outer `| line` — exercises `ManyCursor` surviving a return out of an
    // inner pipe evaluation, not just a single flat pipe.
    let yaml = "containers:\n  - image: a\n  - image: b\n";
    let (output, code) = run_yq_stdin(".containers[].image | line", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, "2\n3\n");
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

// ============================================================================
// Inline block sequence as a mapping value (#325)
// ============================================================================
// `a: - x` is invalid YAML (test-suite case 5U3A) and `yq` rejects it. The
// loader does minimal validation by design, so rather than silently dropping
// the item -- which is what it used to do, yielding `{"a":null}` -- it parses
// the obvious extension. Strict rejection stays available via `yaml validate`.

#[test]
fn test_inline_sequence_as_mapping_value_is_not_dropped() -> Result<()> {
    let (output, exit_code) = run_yq_stdin(".", "a: - x\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":["x"]}"#);
    Ok(())
}

#[test]
fn test_inline_sequence_continuation_line_joins_same_sequence() -> Result<()> {
    // 5U3A itself: both items belong to one sequence, keyed off the column of
    // the `-` rather than a fixed indent.
    let (output, exit_code) = run_yq_stdin(".", "key: - a\n     - b\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"key":["a","b"]}"#);
    Ok(())
}

#[test]
fn test_bare_dash_as_mapping_value_is_an_empty_item() -> Result<()> {
    // Per YAML 1.2 `ns-plain-first`, `-` before whitespace or end-of-input is
    // always the sequence-entry indicator, so this is `[null]`, not `"-"`.
    for input in ["a: -\n", "a: -"] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I0"])?;
        assert_eq!(exit_code, 0);
        assert_eq!(output.trim(), r#"{"a":[null]}"#, "input: {input:?}");
    }
    Ok(())
}

#[test]
fn test_inline_sequence_as_compact_mapping_value_is_not_dropped() -> Result<()> {
    // `- a: - x` is the same shape #325 fixed for `a: - x`, one level deeper: a
    // compact mapping entry (inside a sequence item) whose own value is an
    // inline dash sequence. `parse_compact_mapping_entry` didn't get the dash
    // dispatch when #325 landed, so this fell through to the scalar `"- x"`.
    let (output, exit_code) = run_yq_stdin(".", "- a: - x\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":["x"]}]"#);
    Ok(())
}

// ============================================================================
// Flow collection as a compact mapping's *first* field (#864)
// ============================================================================
// The dash-dispatch fix above (#325/inline-sequence) covers one of
// `parse_compact_mapping_entry`'s inline-value arms; `[`/`{`/`|`/`>` were
// still missing entirely, so any of them as the value of a block-sequence
// item's *first* field fell through to the scalar fallback
// (`parse_inline_value`), which treats `{`/`[`/`,`/`}`/`]` as ordinary plain
// scalar characters. A flow array lost its real content (read back as `[]`);
// a flow mapping was worse — the scalar scanner stopped at its first inner
// `key:`+space and stranded `self.pos` mid-line, corrupting every sibling
// field parsed after it too. Confirmed via oracle diff against pinned `yq`
// v4.53.3 and via before/after `git stash` bisection against this fix.

#[test]
fn test_compact_mapping_first_field_flow_mapping_value_864() -> Result<()> {
    // Before the fix: `[{"a":{}}]` - `b` vanished entirely, not just `a`'s
    // content, matching the issue title's "corrupting sibling parsing".
    let (output, exit_code) = run_yq_stdin(".", "- a: {x: 1}\n  b: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":{"x":1},"b":2}]"#);
    Ok(())
}

#[test]
fn test_compact_mapping_first_field_flow_mapping_value_multi_key_multi_sibling_864() -> Result<()> {
    // The worst-case shape: a multi-key flow mapping followed by multiple
    // sibling fields. Before the fix this read back as just `[{"a":{}}]`,
    // silently dropping `b` and `c` along with `a`'s own contents.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "- a: {x: 1, y: 2}\n  b: 3\n  c: 4\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":{"x":1,"y":2},"b":3,"c":4}]"#);
    Ok(())
}

#[test]
fn test_compact_mapping_first_field_flow_sequence_value_864() -> Result<()> {
    // Before the fix: `[{"a":[],"b":3}]` - `a`'s real content (`[1,2]`) was
    // lost, though `b` happened to survive this particular shape.
    let (output, exit_code) = run_yq_stdin(".", "- a: [1, 2]\n  b: 3\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":[1,2],"b":3}]"#);
    Ok(())
}

#[test]
fn test_compact_mapping_first_field_empty_flow_collections_864() -> Result<()> {
    for (input, expected) in [
        ("- a: {}\n  b: 2\n", r#"[{"a":{},"b":2}]"#),
        ("- a: []\n  b: 2\n", r#"[{"a":[],"b":2}]"#),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I0"])?;
        assert_eq!(exit_code, 0, "input: {input:?}");
        assert_eq!(output.trim(), expected, "input: {input:?}");
    }
    Ok(())
}

#[test]
fn test_compact_mapping_first_field_flow_mapping_value_with_anchor_864() -> Result<()> {
    // An anchor prefixing the flow-mapping value must still resolve through
    // the same dispatch, not just the bare (unanchored) form.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "- a: &anchor {x: 1}\n  b: 2\n- a: *anchor\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":{"x":1},"b":2},{"a":{"x":1}}]"#);
    Ok(())
}

#[test]
fn test_compact_mapping_multiple_items_with_flow_first_field_864() -> Result<()> {
    // Two compact-mapping sequence items in a row, each with a flow-mapping
    // first field, guards against the fix only working for a lone item.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "- a: {x: 1}\n  b: 2\n- a: {x: 3}\n  b: 4\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output.trim(),
        r#"[{"a":{"x":1},"b":2},{"a":{"x":3},"b":4}]"#
    );
    Ok(())
}

#[test]
fn test_compact_mapping_first_field_block_scalar_value_864() -> Result<()> {
    // Colon-free/hash-free body content happens to resolve correctly even
    // through the pre-fix scalar fallback, so this shape alone doesn't
    // prove the dispatch arm does anything - see the sibling test below for
    // body content that actually distinguishes the two.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "- a: |\n    hello\n    world\n  b: 2\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":"hello\nworld\n","b":2}]"#);
    Ok(())
}

#[test]
fn test_compact_mapping_first_field_block_scalar_with_colon_and_hash_lines_864() -> Result<()> {
    // Unlike the plain-content case above, this shape *does* distinguish
    // the fix from the fallback: the pre-fix scalar scanner doesn't know
    // it's inside literal block-scalar content, so a body line shaped like
    // `key: value` or starting with `#` hits the same `:`/`#` terminator
    // rules real keys/comments use. Pre-fix this silently dropped `b`
    // entirely for the colon case, and corrupted the hash case into a
    // spurious `"world":"b"` field - confirmed via before/after bisection
    // during review. This is the only new-arm test in this file that
    // actually fails without the `Some(b'|' | b'>')` dispatch arm.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "- a: |\n    key: value\n    world\n  b: 2\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":"key: value\nworld\n","b":2}]"#);

    let (output, exit_code) = run_yq_stdin(
        ".",
        "- a: |\n    # not a comment\n    world\n  b: 2\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output.trim(),
        r##"[{"a":"# not a comment\nworld\n","b":2}]"##
    );
    Ok(())
}

#[test]
fn test_compact_mapping_first_field_dispatch_agrees_with_ordinary_mapping_entry_864() -> Result<()>
{
    // Invariant guard against this exact bug class recurring again.
    // `parse_compact_mapping_entry` and `parse_mapping_entry` each carry
    // their own hand-written copy of the `[`/`{`/`|`/`>` inline-value
    // dispatch arms, and this file has fixed the same "one copy is missing
    // an arm the other has" defect six times now (#325, #372, #406, #224,
    // #785, #864) - each individually-correct example test above only
    // proves *this* dispatch table currently handles *this* shape; none
    // proves the two tables stay in agreement with each other going
    // forward. A future edit to one dispatch table without the other -
    // exactly how #864 happened - passes every existing example test right
    // up until it ships. This test instead asserts the two tables produce
    // identical parsed values for the same shape in both positions, so a
    // future divergence fails here rather than shipping silently (the #106
    // lesson: "duplicated predicates diverge silently - one definition,
    // plus a test that the call sites agree").
    let cases: &[(&str, &str, &str, &str)] = &[
        (
            "a: {x: 1, y: 2}\n",
            ".a",
            "- a: {x: 1, y: 2}\n  b: 9\n",
            ".[0].a",
        ),
        ("a: [1, 2, 3]\n", ".a", "- a: [1, 2, 3]\n  b: 9\n", ".[0].a"),
        ("a: {}\n", ".a", "- a: {}\n  b: 9\n", ".[0].a"),
        ("a: []\n", ".a", "- a: []\n  b: 9\n", ".[0].a"),
        (
            "a: |\n  hello\n  world\n",
            ".a",
            "- a: |\n    hello\n    world\n  b: 9\n",
            ".[0].a",
        ),
        (
            "a: >\n  hello\n  world\n",
            ".a",
            "- a: >\n    hello\n    world\n  b: 9\n",
            ".[0].a",
        ),
    ];
    for (ordinary_doc, ordinary_filter, compact_doc, compact_filter) in cases {
        let (ordinary_out, ordinary_code) =
            run_yq_stdin(ordinary_filter, ordinary_doc, &["-o", "json", "-I0"])?;
        let (compact_out, compact_code) =
            run_yq_stdin(compact_filter, compact_doc, &["-o", "json", "-I0"])?;
        assert_eq!(ordinary_code, 0, "ordinary doc: {ordinary_doc:?}");
        assert_eq!(compact_code, 0, "compact doc: {compact_doc:?}");
        assert_eq!(
            ordinary_out.trim(),
            compact_out.trim(),
            "ordinary mapping entry and compact-item first field disagree for {ordinary_doc:?} vs {compact_doc:?}"
        );
    }
    Ok(())
}

#[test]
fn test_flow_or_block_value_dispatch_agrees_across_all_four_sites_876() -> Result<()> {
    // #876: `parse_compact_mapping_entry`, `parse_mapping_entry`,
    // `parse_explicit_value`, and `parse_value` used to each hand-roll their
    // own copy of the `[`/`{`/`|`/`>` inline-value dispatch table - the same
    // "one copy is missing an arm the other has" bug class recurring six
    // times (#325, #372, #406, #224, #785, #864) before being unified into
    // one shared `try_dispatch_flow_or_block_value` helper. The test above
    // guards two of those four sites (compact-mapping vs. ordinary mapping);
    // this one extends the same "the tables must agree" invariant to the
    // other two - `parse_explicit_value` (`? key` / `: value`) and
    // `parse_value` (a sequence item that's directly a flow collection or
    // block scalar, not wrapped in a compact mapping) - against the same
    // ordinary-mapping-entry baseline, so a future edit to any one site
    // without the shared helper fails here rather than shipping silently.
    let baseline: &[(&str, &str)] = &[
        ("a: {x: 1, y: 2}\n", ".a"),
        ("a: [1, 2, 3]\n", ".a"),
        ("a: {}\n", ".a"),
        ("a: []\n", ".a"),
        ("a: |\n  hello\n  world\n", ".a"),
        ("a: >\n  hello\n  world\n", ".a"),
    ];
    let explicit_value: &[(&str, &str)] = &[
        ("? a\n: {x: 1, y: 2}\n", ".a"),
        ("? a\n: [1, 2, 3]\n", ".a"),
        ("? a\n: {}\n", ".a"),
        ("? a\n: []\n", ".a"),
        ("? a\n: |\n  hello\n  world\n", ".a"),
        ("? a\n: >\n  hello\n  world\n", ".a"),
    ];
    let bare_sequence_item: &[(&str, &str)] = &[
        ("- {x: 1, y: 2}\n", ".[0]"),
        ("- [1, 2, 3]\n", ".[0]"),
        ("- {}\n", ".[0]"),
        ("- []\n", ".[0]"),
        ("- |\n  hello\n  world\n", ".[0]"),
        ("- >\n  hello\n  world\n", ".[0]"),
    ];
    for ((baseline_doc, baseline_filter), (ev_doc, ev_filter), (bsi_doc, bsi_filter)) in baseline
        .iter()
        .zip(explicit_value.iter())
        .zip(bare_sequence_item.iter())
        .map(|((b, e), s)| (b, e, s))
    {
        let (baseline_out, baseline_code) =
            run_yq_stdin(baseline_filter, baseline_doc, &["-o", "json", "-I0"])?;
        assert_eq!(baseline_code, 0, "baseline doc: {baseline_doc:?}");

        let (ev_out, ev_code) = run_yq_stdin(ev_filter, ev_doc, &["-o", "json", "-I0"])?;
        assert_eq!(ev_code, 0, "explicit-value doc: {ev_doc:?}");
        assert_eq!(
            baseline_out.trim(),
            ev_out.trim(),
            "ordinary mapping entry and explicit key/value disagree for {baseline_doc:?} vs {ev_doc:?}"
        );

        let (bsi_out, bsi_code) = run_yq_stdin(bsi_filter, bsi_doc, &["-o", "json", "-I0"])?;
        assert_eq!(bsi_code, 0, "bare-sequence-item doc: {bsi_doc:?}");
        assert_eq!(
            baseline_out.trim(),
            bsi_out.trim(),
            "ordinary mapping entry and bare sequence item disagree for {baseline_doc:?} vs {bsi_doc:?}"
        );
    }
    Ok(())
}

// ============================================================================
// Trailing content after a flow collection's closing delimiter (#878)
// ============================================================================
// Real YAML (and real `yq`) treats content after a flow collection's closing
// `]`/`}` on the same line - other than whitespace or a comment - as a hard
// parse error. This loader used to silently continue instead, corrupting the
// rest of the document (dropping every sibling field that followed) with no
// error and exit code 0.

#[test]
fn test_trailing_content_after_flow_collection_errors_878() -> Result<()> {
    // Document root.
    let (_, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        "a: [note] see below\nb: 2\nc: 3\nd: 4\n",
        &["-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains("after a flow collection's closing delimiter"),
        "stderr: {stderr}"
    );

    // Ordinary block-mapping field, non-first (used to silently drop b/c/d).
    let (_, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        "- x: 1\n  a: [note] see below\n  b: 2\n  c: 3\n  d: 4\n",
        &["-o", "json", "-I0"],
    )?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains("after a flow collection's closing delimiter"),
        "stderr: {stderr}"
    );

    // A trailing comment is not an error - only non-whitespace/non-comment
    // content is.
    let (out, code) = run_yq_stdin(
        ".a",
        "a: [1, 2] # trailing comment\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "[1,2]");

    Ok(())
}

#[test]
fn test_flow_collection_as_implicit_mapping_key_still_permitted_878() -> Result<()> {
    // #878's validation must not reject a flow collection followed by `:` -
    // that's a legitimate implicit-mapping-key shape (confirmed against the
    // YAML test suite's own "Implicit Flow Mapping Key" and "6BFJ"/"Q9WF"
    // cases), not trailing garbage after a value. Reachable through
    // `parse_value` (document root / bare sequence item) and
    // `parse_explicit_value` (`? key` / `: ...`) - both pass
    // `check_trailing: false` for exactly this reason.
    //
    // Pins *actual* output, not just exit code (this project's testing
    // conventions treat an exit-code-only assertion here as a false-
    // confidence anti-pattern) - and deliberately documents a known,
    // pre-existing gap rather than papering over it: succinctly doesn't
    // actually implement flow-collection-keyed implicit mappings yet. Real
    // `yq` folds `[a, b]: value` into a single-entry mapping `{"":
    // "value"}`; succinctly currently only avoids hard-erroring on it,
    // emitting the flow collection and the trailing value as two disjoint
    // results instead (confirmed identical on `main` before this PR - not
    // introduced or worsened here). This test's job is to confirm the
    // document doesn't error and to catch a regression in the *current*
    // output if one occurs; implementing the real feature is tracked as a
    // follow-up.
    let cases: &[(&str, &str)] = &[
        ("[a, b]: value\n", "[\"a\",\"b\"]\n\"value\""),
        ("{a: 1}: value\n", "{\"a\":1}\n\"value\""),
        ("? k\n: [a, b]: value\n", r#"{"k":["a","b"]}"#),
        ("? k\n: {a: 1}: value\n", r#"{"k":{"a":1}}"#),
    ];
    for (doc, expected) in cases {
        let (out, code) = run_yq_stdin(".", doc, &["-o", "json", "-I0"])?;
        assert_eq!(code, 0, "doc: {doc:?}");
        assert_eq!(out.trim(), *expected, "doc: {doc:?}");
    }
    Ok(())
}

// ============================================================================
// Extra spaces after a block-sequence dash (#877)
// ============================================================================
// `parse_sequence_item`'s compact-mapping dispatch used to hardcode
// `compact_indent = indent + 2`, assuming exactly one space between `-` and
// a compact item's first key. With more than one space, every field after
// the first folded into the first field's own scalar value instead of being
// recognized as its own entry - `-   a: hello` / `    b: 2` (three spaces
// after the dash) read back as `{"a":"hello b"}` (`b` concatenated as text
// onto `a`'s value, `2` lost entirely).

#[test]
fn test_compact_mapping_dash_spacing_877() -> Result<()> {
    let cases: &[(&str, &str)] = &[
        // Extra spaces, the headline repro.
        ("-   a: hello\n    b: 2\n", r#"[{"a":"hello","b":2}]"#),
        // Extra spaces, three fields.
        (
            "-     a: 1\n      b: 2\n      c: 3\n",
            r#"[{"a":1,"b":2,"c":3}]"#,
        ),
        // Extra spaces + a flow-collection first field: interacts with
        // #864's fix, since the compact-mapping indent this dispatches
        // through is the same one #877 fixes.
        ("-   a: {x: 1}\n    b: 2\n", r#"[{"a":{"x":1},"b":2}]"#),
        // Extra spaces, multiple compact items in one sequence.
        (
            "-   a: 1\n    b: 2\n-   a: 3\n    b: 4\n",
            r#"[{"a":1,"b":2},{"a":3,"b":4}]"#,
        ),
        // Control: the ordinary single-space case must remain unaffected.
        ("- a: hello\n  b: 2\n", r#"[{"a":"hello","b":2}]"#),
    ];
    for (input, expected) in cases {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I0"])?;
        assert_eq!(exit_code, 0, "input: {input:?}");
        assert_eq!(output.trim(), *expected, "input: {input:?}");
    }
    Ok(())
}

// ============================================================================
// Out-dented block sequence continuation (#485)
// ============================================================================
// A continuation `-` indented strictly between a sequence's own indent and
// whatever encloses it is invalid YAML (`yq` rejects it, and so does the
// strict validator), but the loader parses the obvious extension rather than
// silently dropping the item, the same policy #325 chose for `a: - x`.
// Before this fix, closing the sequence for the out-of-range indent reopened
// a second, untagged sequence as a sibling child of the mapping instead of a
// value under a key, which not only dropped the misaligned item but corrupted
// the *next* mapping entry into a phantom `"":<value>` pair.

#[test]
fn test_out_dented_sequence_continuation_joins_the_sequence() -> Result<()> {
    // The #485 repro: `y` used to vanish and `c: 2` corrupted into `"":"c"`.
    let (output, exit_code) =
        run_yq_stdin(".", "b:\n    - x\n   - y\nc: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"b":["x","y"],"c":2}"#);
    Ok(())
}

#[test]
fn test_out_dented_sequence_continuation_does_not_lose_later_items() -> Result<()> {
    // Everything after the misaligned item must survive too, not just resume
    // being lost one item later.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "b:\n    - x\n   - y\n    - z\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"b":["x","y","z"]}"#);
    Ok(())
}

#[test]
fn test_out_dented_sequence_continuation_minimal_form() -> Result<()> {
    // No trailing entry, no outer nesting.
    let (output, exit_code) = run_yq_stdin(".", "b:\n  - x\n - y\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"b":["x","y"]}"#);
    Ok(())
}

#[test]
fn test_out_dented_sequence_continuation_nested_in_a_mapping() -> Result<()> {
    // The same shape one level deeper: `b`'s sequence sits inside `a`, and the
    // enclosing frame the out-dented `- y` must reach past is `b`, not the
    // document root.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "a:\n  b:\n    - x\n   - y\n  c: 2\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"b":["x","y"],"c":2}}"#);
    Ok(())
}

#[test]
fn test_correctly_aligned_sequence_is_unaffected_by_out_dent_handling() -> Result<()> {
    // Regression guard: an ordinary, correctly-indented sequence must parse
    // exactly as before.
    let (output, exit_code) = run_yq_stdin(".", "b:\n  - x\n  - y\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"b":["x","y"]}"#);
    Ok(())
}

// ============================================================================
// Inconsistent compact-mapping continuation indent (#885)
// ============================================================================
// The mapping-shaped analog of #485 above: a continuation line indented
// strictly between a compact mapping's own indent and its enclosing sequence
// item's virtual indent is invalid YAML (`yq` v4.53.3 rejects it outright),
// but before this fix, the plain indent comparison `close_deeper_indents`
// applies closed the compact mapping and reopened a second, orphaned mapping
// as a sibling directly under the same sequence item — which structurally
// expects only one value child, so the JSON serializer silently dropped
// every field after the first inconsistent line. Same "parse the obvious
// extension" policy #485 (and #325 before it) already established for this
// class of problem.

#[test]
fn test_inconsistent_compact_mapping_indent_headline_repro() -> Result<()> {
    // yq v4.53.3: `Error: bad file '-': yaml: while parsing a block
    // collection ...: did not find expected '-' indicator`.
    let (output, exit_code) = run_yq_stdin(".", "-   a: hello\n  b: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":"hello","b":2}]"#);
    Ok(())
}

#[test]
fn test_inconsistent_compact_mapping_indent_does_not_lose_later_fields() -> Result<()> {
    // Every field after the first inconsistent line must survive, not just
    // the first one.
    let (output, exit_code) =
        run_yq_stdin(".", "-   a: 1\n  b: 2\n  c: 3\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":1,"b":2,"c":3}]"#);
    Ok(())
}

#[test]
fn test_inconsistent_compact_mapping_indent_does_not_leak_into_next_item() -> Result<()> {
    // A properly-dashed sibling item after the inconsistent one must still
    // be recognized as its own item, not folded into the first.
    let (output, exit_code) =
        run_yq_stdin(".", "-   a: 1\n  b: 2\n-   x: 9\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":1,"b":2},{"x":9}]"#);
    Ok(())
}

#[test]
fn test_indent_genuinely_below_the_item_stays_a_separate_top_level_entry() -> Result<()> {
    // Regression guard: an indent at or below the sequence item's own dash
    // column (strictly below the compact mapping's *and* the item's own
    // virtual indent) must not be swallowed into the mapping by
    // `compact_mapping_gap_reaches`. Real `yq` v4.53.3 rejects this input
    // too (`did not find expected '-' indicator`) — same as every other
    // case in this file — succinctly's own two-item output here comes from
    // a separate, pre-existing "parse the obvious extension" heuristic
    // (an indent-0 mapping line becoming a new top-level sequence item),
    // not evidence this shape is any more spec-legitimate than the others.
    let (output, exit_code) = run_yq_stdin(".", "-   a: hello\nb: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":"hello"},{"b":2}]"#);
    Ok(())
}

#[test]
fn test_inconsistent_compact_mapping_indent_single_space_continuation() -> Result<()> {
    // The most common real-world shape: single space after the dash (not
    // this file's extra-spaced headline repro), with the continuation
    // indented exactly at the sequence item's own virtual indent — the
    // inclusive lower bound `compact_mapping_gap_reaches` needs (`indent >=`,
    // not `indent >`) for this to be recognized as still belonging to the
    // mapping rather than silently dropped.
    let (output, exit_code) = run_yq_stdin(".", "- a: 1\n b: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":1,"b":2}]"#);
    Ok(())
}

#[test]
fn test_inconsistent_compact_mapping_indent_with_explicit_key() -> Result<()> {
    // `parse_explicit_key`/`parse_explicit_value` (the `?`/`:` dispatch
    // arms) carry the identical close_deeper_indents/need_new_mapping
    // shape `parse_mapping_entry` does, and needed the same fix.
    let (output, exit_code) =
        run_yq_stdin(".", "-   a: hello\n  ? b\n  : 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":"hello","b":2}]"#);
    Ok(())
}

#[test]
fn test_inconsistent_compact_mapping_indent_with_anchor_prefixed_key() -> Result<()> {
    // The `&`/`!` dispatch arm has its own `close_deeper_indents` call and
    // needed the same gap-tolerant indent as the plain-key arm.
    let (output, exit_code) = run_yq_stdin(".", "-   &anc k: v\n  b: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"k":"v","b":2}]"#);
    Ok(())
}

#[test]
fn test_inconsistent_compact_mapping_indent_with_tag_prefixed_key() -> Result<()> {
    let (output, exit_code) =
        run_yq_stdin(".", "-   !!str k: v\n  b: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"k":"v","b":2}]"#);
    Ok(())
}

#[test]
fn test_inconsistent_compact_mapping_indent_with_alias_key() -> Result<()> {
    // The `*` (alias-as-key) dispatch arm.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "x: &k1 hello\nlist:\n-   *k1: v\n  b: 2\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(
        output.trim(),
        r#"{"x":"hello","list":[{"hello":"v","b":2}]}"#
    );
    Ok(())
}

// ============================================================================
// A `-` continuation in a compact mapping's gap (#900)
// ============================================================================
// The sibling shape #885's own doc comment flagged as deliberately
// out-of-scope: a bare `-` line (not a `key: value` line) landing in the
// same gap. Unlike a mapping entry, a `-` can't "just be added" to the open
// compact mapping -- the obvious extension instead closes through both the
// mapping and its enclosing sequence item and treats this as the next item
// of the *outer* sequence, same "parse the obvious extension" policy as
// #325/#485/#885. Real `yq` v4.53.3 rejects every one of these inputs
// outright (`did not find expected '-' indicator`), same as #885's own
// cases.

#[test]
fn test_sequence_item_gap_headline_repro() -> Result<()> {
    let (output, exit_code) = run_yq_stdin(".", "-   a: hello\n  - b\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":"hello"},"b"]"#);
    Ok(())
}

#[test]
fn test_sequence_item_gap_at_the_mappings_own_indent_with_resolved_key() -> Result<()> {
    // Lower bound aside, the mapping's own exact indent is *not* included in
    // this fix's trigger range (see `sequence_item_gap_reaches`'s own doc
    // comment for why: that exact indent is ambiguous with the legitimate
    // "block sequence value at the key's own indent" shape when the key is
    // deferred, so this fix deliberately leaves it alone rather than risk
    // misreading that case). This regression guard pins the current,
    // unchanged (pre-existing, still-buggy) behavior for the one sub-case
    // this fix does *not* reach, so a future change to the boundary doesn't
    // silently start (or stop) altering it without a test noticing either
    // way.
    let (output, exit_code) = run_yq_stdin(".", "-   a: hello\n    - b\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":"hello"}]"#);
    Ok(())
}

#[test]
fn test_sequence_item_gap_does_not_regress_deferred_value_at_own_indent() -> Result<()> {
    // The legitimate case this fix must not disturb: a compact mapping's
    // last key with its value deferred to the next line, and that next line
    // is a `-` at exactly the mapping's own indent -- valid YAML (a block
    // sequence may sit at its key's own indent), already correct before
    // #900, and exercised end-to-end here (not just at the
    // `YamlIndex`-internal level `test_compact_entry_trailing_anchor_targets_its_collection`
    // already covers).
    let (output, exit_code) = run_yq_stdin(".", "- k: &a\n  - 1\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"k":[1]}]"#);
    Ok(())
}

#[test]
fn test_sequence_item_gap_does_not_lose_later_items() -> Result<()> {
    // The second `-   x: 9\n` sibling (a fresh compact-mapping item, not a
    // plain scalar) confirms the outer sequence is genuinely reused, not
    // just tolerated for a single extra item. `- c` is deliberately at
    // indent 0 rather than the same 2-space indent as `- b`: indent 2 is
    // *greater* than item `b`'s own virtual indent (1), so it would fold as
    // a continuation of `b`'s own plain-scalar text under ordinary YAML
    // rules (verified against real yq: `- b\n  - c\n` reads as `["b - c"]`)
    // -- an unrelated, pre-existing folding rule, not this fix's concern.
    let (output, exit_code) = run_yq_stdin(
        ".",
        "-   a: 1\n  - b\n- c\n-   x: 9\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"[{"a":1},"b","c",{"x":9}]"#);
    Ok(())
}

// ============================================================================
// A mapping-under-mapping sibling gap (#901)
// ============================================================================
// #885's own doc comment flagged this as needing its own correctness
// argument before extending its tolerance: unlike a `SequenceItem` frame,
// an ordinary `Mapping` frame doesn't guarantee it's the *only* possible
// enclosing scope for an out-of-range sibling -- both the inner and outer
// mapping are structurally plausible owners. Real `yq` v4.53.3 rejects
// every one of these inputs outright too (`did not find expected key`), and
// unlike #900, no "obvious extension" resolves the ambiguity here, so this
// stays a genuine parse error rather than a tolerated shape.

#[test]
fn test_mapping_under_mapping_gap_headline_repro() -> Result<()> {
    let (_output, stderr, exit_code) =
        run_yq_stdin_with_stderr(".", "a:\n    b: 1\n  c: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 1);
    assert!(
        stderr.contains("inconsistent indentation"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_mapping_under_mapping_gap_does_not_reject_sibling_at_outer_indent() -> Result<()> {
    // Regression guard: a sibling that lands exactly at the *outer*
    // mapping's own indent is ordinary, unambiguous YAML (close the inner
    // mapping, add a sibling key to the outer one), not this gap shape.
    let (output, exit_code) = run_yq_stdin(".", "a:\n    b: 1\nc: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"b":1},"c":2}"#);
    Ok(())
}

#[test]
fn test_mapping_under_mapping_gap_does_not_reject_sibling_at_inner_indent() -> Result<()> {
    // Regression guard: a sibling at exactly the *inner* mapping's own
    // indent is just another ordinary key of that mapping.
    let (output, exit_code) =
        run_yq_stdin(".", "a:\n    b: 1\n    c: 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"b":1,"c":2}}"#);
    Ok(())
}

#[test]
fn test_mapping_under_mapping_gap_does_not_regress_deferred_value_at_inner_indent() -> Result<()> {
    // The mapping-under-mapping analog of #900's own deferred-value guard:
    // a key with its value deferred to the next line, and that next line is
    // a nested sequence item at exactly the inner mapping's own indent.
    let (output, exit_code) =
        run_yq_stdin(".", "a:\n    k: &x\n    - 1\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 0);
    assert_eq!(output.trim(), r#"{"a":{"k":[1]}}"#);
    Ok(())
}

#[test]
fn test_mapping_under_mapping_gap_via_explicit_key() -> Result<()> {
    // `parse_explicit_key`'s own copy of the same check.
    let (_output, stderr, exit_code) =
        run_yq_stdin_with_stderr(".", "a:\n    b: 1\n  ? c\n  : 2\n", &["-o", "json", "-I0"])?;
    assert_eq!(exit_code, 1);
    assert!(
        stderr.contains("inconsistent indentation"),
        "stderr: {stderr:?}"
    );
    Ok(())
}

#[test]
fn test_mapping_under_mapping_gap_via_explicit_value() -> Result<()> {
    // `parse_explicit_value`'s own copy of the same check, genuinely
    // exercised: the key (`? c`) is at the inner mapping's own valid indent
    // (4), so it triggers no check itself -- only the *value* line (`: 2`)
    // lands in the gap (indent 2). A key at the gap indent instead (as an
    // earlier version of this test used) triggers `parse_explicit_key`'s
    // copy before this one is ever reached, leaving this branch untested.
    let (_output, stderr, exit_code) = run_yq_stdin_with_stderr(
        ".",
        "a:\n    b: 1\n    ? c\n  : 2\n",
        &["-o", "json", "-I0"],
    )?;
    assert_eq!(exit_code, 1);
    assert!(
        stderr.contains("inconsistent indentation"),
        "stderr: {stderr:?}"
    );
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
    // Default `-o yaml` must quote the scalar regardless of container style:
    // emitted bare under a `- ` marker it would read back as a nested
    // sequence. Since #707 fixed flow-style preservation, this flow-style
    // source now correctly stays flow on output (previously forced to
    // block) — the quoting obligation itself is unconditional and still
    // applies.
    let (yaml_out, code) = run_yq_stdin(".", "[- x]\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(yaml_out, "[\"- x\"]\n");

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
    // streaming path (`-I 0`). Before #442, the pretty path wasn't asserted
    // here: its DOM collapsed duplicate mapping keys, which resolution makes
    // reachable from more inputs but does not cause. #442 fixed pretty output
    // for identity/navigation queries, so a couple of representative cases
    // are now spot-checked in pretty mode too, below.
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

    // Pretty-mode spot check (#442): a duplicate-`k`-key case and a
    // duplicate-empty-string-key case, matching real yq's pretty output.
    let (output, code) = run_yq_stdin(".", "{&x k: 1, *x: 2}\n", &["-o=json"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\n  \"k\": 1,\n  \"k\": 2\n}\n");

    let (output, code) = run_yq_stdin(".", "{&x [1,2]: 3, *x: 4}\n", &["-o=json"])?;
    assert_eq!(code, 0);
    assert_eq!(output, "{\n  \"\": 3,\n  \"\": 4\n}\n");

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
fn test_alias_as_a_flow_sequence_key_resolves() -> Result<()> {
    // #409, the flow-*sequence* counterpart of #405 above. An implicit
    // single-pair-mapping entry inside `[...]` never reached the pair check
    // at all for an alias key: the sequence loop consumed a leading `*` as a
    // *standalone* item and `continue`d, so `*x: 2` left the cursor on `:`
    // and errored `expected ',' or ']'`. An anchor on such a key parsed
    // without erroring but bound to the mapping wrapper instead of the key,
    // the same wrong-node shape #405 fixed for the flow-mapping key.
    //
    // Every expectation is `yq` v4.53.3's output for the same input, on the
    // streaming path (`-I 0`), taken directly from the issue's repro table.
    for (yaml, expected) in [
        // Alias to an anchored key.
        ("[&x k: 1, *x: 2]\n", r#"[{"k":1},{"k":2}]"#),
        // Alias to an anchored value, nested inside a block mapping.
        ("{a: &x 1, b: [*x: 2]}\n", r#"{"a":1,"b":[{"1":2}]}"#),
        // Anchor on the key, aliased by a later *plain* (non-pair) item.
        ("[&x k: 1, *x]\n", r#"[{"k":1},"k"]"#),
        // Unaffected baselines the issue calls out: no anchor/alias at all,
        // and a plain aliased scalar item (not a key) - both must still work.
        ("[a: 1, b: 2]\n", r#"[{"a":1},{"b":2}]"#),
        ("[&x a, *x]\n", r#"["a","a"]"#),
    ] {
        let (output, code) = run_yq_stdin(".", yaml, &["-o=json", "-I=0"])?;
        assert_eq!(code, 0, "for {yaml:?}");
        assert_eq!(output.trim(), expected, "for {yaml:?}");
    }

    // An anchor and an alias to it on the same key: must still be rejected as
    // a cycle, the same as the flow-mapping key position above - this call
    // path must not have quietly dropped `record_key_alias`'s cycle check.
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".", "[&x *x: 1]\n", &["-o=json"])?;
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

#[test]
fn test_split_doc_hides_in_computed_slice_bounds() -> Result<()> {
    // Same reasoning as `test_computed_key_in_index_brackets`, for a computed
    // slice's target and bounds (#499). `contains_split_doc` has to descend
    // into all three, or a `split_doc` hiding in one is scanned as an opaque
    // leaf and the stream never gets its `---` separators.
    let yaml = "[[1,2,3,4],[5,6,7,8]]\n";
    let expected = "- 2\n- 3\n---\n- 6\n- 7\n";

    // Hidden in the start bound.
    let (output, code) = run_yq_stdin(".[] | .[(1|split_doc):(3)]", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, expected);

    // Hidden in the end bound.
    let (output, code) = run_yq_stdin(".[] | .[(1):(3|split_doc)]", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, expected);

    // Hidden in the target.
    let (output, code) = run_yq_stdin(".[] | (split_doc)[(1):(3)]", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output, expected);
    Ok(())
}

#[test]
fn test_split_doc_hides_in_halt_error_argument() -> Result<()> {
    // #791 follow-up: `contains_split_doc` never recursed into
    // `Builtin::HaltErrorCode`'s argument, so `has_split_doc` came back
    // `false` for a filter with `split_doc` reachable only through a
    // never-taken `halt_error(...)` branch. That misses more than the
    // separators `split_doc` itself would add: `has_split_doc == false`
    // also *re-enables* the DOM path's own regular multi-doc `---`
    // injection (gated on `!has_split_doc`), which -- unlike
    // `SplitDocState` -- writes a separator before the very first document
    // too, not just between documents.
    let yaml = "x: 1\n---\nx: 2\n";
    let (output, code) = run_yq_stdin("if false then halt_error(split_doc) else . end", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(
        output, "x: 1\n---\nx: 2\n",
        "no leading separator before doc 0"
    );
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

#[test]
fn test_dash_not_followed_by_space_is_still_a_scalar() -> Result<()> {
    // Guard against over-matching: negative numbers and `-`-prefixed plain
    // scalars must not be reinterpreted as sequences.
    for (input, want) in [
        ("a: -1\n", r#"{"a":-1}"#),
        ("a: -x\n", r#"{"a":"-x"}"#),
        ("{a: -}\n", r#"{"a":"-"}"#),
    ] {
        let (output, exit_code) = run_yq_stdin(".", input, &["-o", "json", "-I0"])?;
        assert_eq!(exit_code, 0);
        assert_eq!(output.trim(), want, "input: {input:?}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// #355: an uncaught evaluation error must fail the process
//
// yq's conventions differ from jq's and both are drop-in targets, so the yq
// path keeps yq's: `Error: <msg>` on stderr, exit 1, no position marker.
// Captured from mikefarah/yq v4.53.3.
// ---------------------------------------------------------------------------

#[test]
fn test_uncaught_error_exits_1_yq_style() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(r#"error("boom")"#, "x: 1\n", &[])?;
    assert_eq!(code, 1, "uncaught error must exit 1 like yq: {stderr}");
    assert_eq!(stderr.trim_end(), "Error: boom");
    assert_eq!(stdout, "", "a failed filter produces no output");
    // yq carries neither of jq's markers.
    assert!(!stderr.contains("(at "), "{stderr}");
    assert!(!stderr.contains("(not a string)"), "{stderr}");
    Ok(())
}

// ---------------------------------------------------------------------------
// `halt`, `halt_error`/`halt_error(n)`, and `stderr` (#791). Byte-exact
// stderr/exit-code expectations are inherited from `tests/jq_cli_tests.rs`'s
// (verified live against real jq) since these are jq-language builtins, not
// yq-specific diagnostics -- the one yq-specific piece is bare `halt_error`'s
// *default* exit code (1, matching yq's uniform failure code, not jq's 5);
// there is no real `yq` to check that default against (mikefarah/yq has no
// `halt_error` at all -- this is this codebase's own documented extension).
// ---------------------------------------------------------------------------

#[test]
fn test_yq_halt_exits_0_with_no_output() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr("halt", "x: 1\n", &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_yq_halt_error_default_exit_code_is_1() -> Result<()> {
    // yq's uniform failure code (`DiagStyle::error_exit_code()`'s yq arm),
    // not jq's 5 -- see this section's header comment.
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(r#""foo" | halt_error"#, "x: 1\n", &[])?;
    assert_eq!(code, 1, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "foo");
    Ok(())
}

#[test]
fn test_yq_halt_error_custom_exit_code_overrides_the_yq_default() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#""foo" | halt_error(7)"#, "x: 1\n", &[])?;
    assert_eq!(code, 7, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "foo");
    Ok(())
}

#[test]
fn test_yq_halt_error_null_prints_nothing() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr("null | halt_error(9)", "x: 1\n", &[])?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_yq_stderr_passes_through_and_prints_raw_compact_with_no_newline() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#""hello" | stderr"#, "x: 1\n", &["-o", "json"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "\"hello\"\n");
    assert_eq!(stderr, "hello");
    Ok(())
}

#[test]
fn test_yq_halt_not_caught_by_try_catch_or_label() -> Result<()> {
    for (filter, want_code) in [
        (r#"try (halt) catch "caught""#, 0),
        (r#"try ("x"|halt_error) catch "caught""#, 1),
        (r"label $out | (halt, break $out)", 0),
    ] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(filter, "x: 1\n", &[])?;
        assert_eq!(code, want_code, "{filter}: stderr: {stderr:?}");
        assert!(!stdout.contains("caught"), "{filter}: stdout: {stdout:?}");
    }
    Ok(())
}

/// #791 follow-up: `map(f)`'s path-context evaluator (only reachable via
/// `--eval-all` when `f` references `file_index`/`key`/`path`/`parent`, which
/// routes through `eval_pipe_with_path_context_internal` instead of the
/// ordinary `map_over`) discarded the partial array on a mid-map `Error`/
/// `Break` but let a `Halt` leak the elements already mapped before it as if
/// they were legitimate output. `map(f)` is array construction, atomic in jq
/// (`[1,error("x"),3]` produces no output at all) -- a halt partway through
/// must discard the whole array, same as error/break.
#[test]
fn test_map_with_path_context_discards_partial_array_on_halt() -> Result<()> {
    let mut f1 = NamedTempFile::new()?;
    writeln!(f1, "1")?;
    let mut f2 = NamedTempFile::new()?;
    writeln!(f2, "2")?;
    let mut f3 = NamedTempFile::new()?;
    writeln!(f3, "3")?;

    let (stdout, stderr, code) = run_yq_files(
        "map(if . == 2 then halt else . + 10 + file_index end)",
        &[f1.path(), f2.path(), f3.path()],
        &["--eval-all"],
    )?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "", "partial map output must not leak: {stdout:?}");
    Ok(())
}

/// #791 follow-up: in the M2 YAML streaming fast path, `will_output`'s
/// exclusions covered `None`/`Break`/empty `Many*` but not the zero-output
/// `GenericResult::Halt`, so a document that halts is misclassified as
/// "about to output," and `emit_yaml_doc_separator` writes a stray `---`
/// with nothing behind it. `select(...)` stays on the M2 fast path
/// regardless of its predicate's shape (#796), so wrapping a per-document
/// conditional halt in `select` reaches this exact branch.
#[test]
fn test_m2_select_halt_does_not_emit_stray_separator() -> Result<()> {
    let yaml = "a: 1\n---\na: 2\n";
    let (output, code) = run_yq_stdin("select(if .a == 2 then halt else true end)", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(
        output, "a: 1\n",
        "no stray separator after the halted document"
    );
    Ok(())
}

/// #791 follow-up: `std::fs::write` in the `--inplace` write-back ran
/// unconditionally, so a filter that produced no output for a file (a
/// `halt`/`halt_error` before that file's first document, `empty`, ...)
/// truncated it to zero bytes -- destroying the original content. `halt`
/// used as an early-exit guard clause is a natural way to trigger this, more
/// so than the pre-existing `empty`/error triggers.
#[test]
fn test_inplace_halt_before_any_output_does_not_truncate_file() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    write!(input_file, "a: 1\nb: hello\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("halt")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let content = std::fs::read_to_string(input_file.path())?;
    assert_eq!(content, "a: 1\nb: hello\n", "original content must survive");
    Ok(())
}

/// #791 follow-up: the multi-doc `---` separator was written into the
/// in-place output buffer *before* evaluating the document it precedes, so
/// a `halt` on the first document of a multi-document file left the buffer
/// non-empty (just the separator) even though no real output was produced.
/// The write-back guard checked `output_buffer.is_empty()`, saw a non-empty
/// buffer, and wrote it back -- truncating the original two-document file
/// down to a lone `---`.
#[test]
fn test_inplace_halt_before_any_output_in_multi_doc_file_does_not_truncate_file() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    write!(input_file, "a: 1\n---\na: 2\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("if .a == 1 then halt else . end")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let content = std::fs::read_to_string(input_file.path())?;
    assert_eq!(
        content, "a: 1\n---\na: 2\n",
        "original content must survive"
    );
    Ok(())
}

/// Regression test (#791 follow-up), a distinct bug from the one above: the
/// DOM `--inplace` branch wrote the multi-doc `---` separator into the
/// buffer *before* evaluating the document it precedes, so a halt on a
/// *later* document (not the first) left a dangling separator with nothing
/// after it committed to disk -- a spurious trailing null document. Not
/// halt-specific either: the same eager write also dangled a `---` for an
/// ordinary empty-output document with no halt involved at all (`empty`
/// case below), since the separator was written speculatively regardless of
/// whether the document that followed it ever produced anything.
#[test]
fn test_inplace_multi_doc_no_dangling_separator_before_halting_or_empty_document() -> Result<()> {
    for (filter, want) in [
        ("if .a == 2 then halt else . end", "---\na: 1\n"),
        ("if .a == 2 then empty else . end", "---\na: 1\n---\na: 3\n"),
    ] {
        let mut input_file = NamedTempFile::new()?;
        write!(input_file, "a: 1\n---\na: 2\n---\na: 3\n")?;

        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("yq")
            .arg("-i")
            .arg(filter)
            .arg(input_file.path())
            .stdin(Stdio::null())
            .output()?;

        assert!(output.status.success(), "{filter}");
        let content = std::fs::read_to_string(input_file.path())?;
        assert_eq!(content, want, "{filter}: no dangling separator");
    }
    Ok(())
}

/// The fix above is deliberately halt-specific, not "any empty output
/// preserves the file": real yq (v4.53.3, verified live) truncates a file
/// to reflect a filter that legitimately produces no output for it --
/// `-i 'select(false)'` empties the file rather than leaving it untouched.
/// Only a `halt`-caused emptiness gets the preserve-original-content
/// protection.
#[test]
fn test_inplace_legitimately_empty_output_still_truncates_file() -> Result<()> {
    for filter in ["select(false)", "empty"] {
        let mut input_file = NamedTempFile::new()?;
        write!(input_file, "a: 1\nb: hello\n")?;

        let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("yq")
            .arg("-i")
            .arg(filter)
            .arg(input_file.path())
            .stdin(Stdio::null())
            .output()?;

        assert!(output.status.success(), "{filter}");
        let content = std::fs::read_to_string(input_file.path())?;
        assert_eq!(content, "", "{filter}: must truncate, matching real yq");
    }
    Ok(())
}

/// A multi-file `--inplace` halt partway through must still: edit files
/// before the halt normally, leave the halting file's original content
/// intact (the halt fired before that file produced any output), and leave
/// every later file completely untouched.
#[test]
fn test_inplace_multi_file_halt_leaves_untouched_files_alone() -> Result<()> {
    let mut f1 = NamedTempFile::new()?;
    writeln!(f1, "a: 1")?;
    let mut f2 = NamedTempFile::new()?;
    writeln!(f2, "a: 2")?;
    let mut f3 = NamedTempFile::new()?;
    writeln!(f3, "a: 3")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("if .a == 2 then halt else .a += 100 end")
        .arg(f1.path())
        .arg(f2.path())
        .arg(f3.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    assert_eq!(std::fs::read_to_string(f1.path())?, "a: 101\n");
    assert_eq!(std::fs::read_to_string(f2.path())?, "a: 2\n");
    assert_eq!(std::fs::read_to_string(f3.path())?, "a: 3\n");
    Ok(())
}

/// The diagnostic must never reach stdout.
///
/// The YAML and JSON streaming fast paths used to `write!` it into the output
/// writer, so a failed filter emitted its error inline with data -- invisible
/// to `2>/dev/null` and indistinguishable from a result to any consumer.
#[test]
fn test_uncaught_error_never_reaches_stdout() -> Result<()> {
    for args in [&[][..], &["-o", "json"][..], &["-o", "yaml"][..]] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(r#"error("boom")"#, "x: 1\n", args)?;
        assert_eq!(stdout, "", "diagnostic leaked to stdout with {args:?}");
        assert_eq!(code, 1, "{args:?}");
        assert!(stderr.contains("boom"), "{args:?}: {stderr}");
    }
    // Multi-document input goes through the same streaming path.
    let (stdout, _, code) = run_yq_stdin_with_stderr(r#"error("boom")"#, "a: 1\n---\nb: 2\n", &[])?;
    assert_eq!(stdout, "");
    assert_eq!(code, 1);
    Ok(())
}

/// yq reaches the evaluator by several routes; a half-fix would leave some
/// exiting 0. Every route must agree.
#[test]
fn test_uncaught_error_fails_on_every_evaluation_path() -> Result<()> {
    // YAML cursor streaming (the default), and the YAML DOM path via --slurp.
    for args in [&[][..], &["-s"][..]] {
        let (stdout, stderr, code) = run_yq_stdin_with_stderr(r#"error("boom")"#, "x: 1\n", args)?;
        assert_eq!(code, 1, "{args:?}: {stderr}");
        assert_eq!(stdout, "", "{args:?}");
        assert_eq!(stderr.trim_end(), "Error: boom", "{args:?}");
    }

    // JSON input, and null input, which skip the YAML paths entirely.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#"error("boom")"#, r#"{"a":1}"#, &["-p", "json"])?;
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr.trim_end(), "Error: boom");

    let (stdout, stderr, code) = run_yq_stdin_with_stderr(r#"error("boom")"#, "", &["-n"])?;
    assert_eq!(code, 1, "{stderr}");
    assert_eq!(stdout, "");
    assert_eq!(stderr.trim_end(), "Error: boom");
    Ok(())
}

#[test]
fn test_yq_caught_error_and_clean_run_still_exit_0() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#"try error("boom") catch "caught""#, "x: 1\n", &[])?;
    assert_eq!(code, 0, "a caught error is not a failure: {stderr}");
    assert_eq!(stderr, "");
    assert_eq!(stdout.trim(), "caught");

    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".x", "x: 1\n", &[])?;
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout.trim(), "1");
    Ok(())
}

#[test]
fn test_yq_error_outranks_exit_status_flag() -> Result<()> {
    // Both are exit 1 for yq, but the message must be the error, not the
    // "no matches found" that -e reports for an empty/falsy result (#178).
    let (_, stderr, code) = run_yq_stdin_with_stderr(r#"error("boom")"#, "x: 1\n", &["-e"])?;
    assert_eq!(code, 1);
    assert_eq!(stderr.trim_end(), "Error: boom");
    assert!(
        !stderr.contains("no matches found"),
        "an error is not a falsy result: {stderr}"
    );

    // -e's own semantics are untouched when nothing raised.
    let (_, stderr, code) = run_yq_stdin_with_stderr("false", "x: 1\n", &["-e"])?;
    assert_eq!(code, 1);
    assert!(stderr.contains("no matches found"), "{stderr}");
    Ok(())
}

#[test]
fn test_yq_outputs_before_an_error_or_break_survive() -> Result<()> {
    // The yq side of #400/#494: a stream that produces outputs and *then*
    // fails keeps those outputs, with the failure reported on stderr and in
    // the exit code. yq's two evaluation routes each convert the result
    // separately, so both are exercised here.
    //
    // Real yq (mikefarah v4.53.3) buffers the whole result before printing,
    // so it emits nothing at all on stdout for these filters and exits 1 --
    // succinctly streams instead. Only the diagnostic and exit code match.
    // These therefore pin succinctly's own behavior; the byte-for-byte yq
    // oracle lives in tests/yq_golden_tests.rs.

    // Default YAML input goes through the direct-cursor (generic) route.
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(r#"1,2,error("x")"#, "a: 1\n", &[])?;
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(stderr.trim_end(), "Error: x");
    assert_eq!(code, 1);

    let (stdout, stderr, code) = run_yq_stdin_with_stderr("1,2,break $out", "a: 1\n", &[])?;
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(stderr.trim_end(), "Error: break $out not in label");
    assert_eq!(code, 1);

    // The same under `-o json`: the output format is orthogonal to the
    // prefix-then-failure contract.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#"1,2,error("x")"#, "a: 1\n", &["-o", "json"])?;
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(stderr.trim_end(), "Error: x");
    assert_eq!(code, 1);

    // `--null-input` and `--slurp` take the OwnedValue route, which converts
    // the full evaluator's result rather than the generic one's.
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(r#"1,2,error("x")"#, "", &["-n"])?;
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(stderr.trim_end(), "Error: x");
    assert_eq!(code, 1);

    let (stdout, stderr, code) = run_yq_stdin_with_stderr("1,2,break $out", "", &["-n"])?;
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(stderr.trim_end(), "Error: break $out not in label");
    assert_eq!(code, 1);

    let (stdout, stderr, code) = run_yq_stdin_with_stderr("1,2,break $out", "a: 1\n", &["-s"])?;
    assert_eq!(stdout, "1\n2\n");
    assert_eq!(stderr.trim_end(), "Error: break $out not in label");
    assert_eq!(code, 1);
    Ok(())
}

#[test]
fn test_693_optional_around_stream_stops_at_the_first_error() -> Result<()> {
    // Mirrors `test_yq_outputs_before_an_error_or_break_survive`'s two-route
    // pattern: yq's default (non-null-input) route evaluates through the
    // same native `eval_generic` cursor-based evaluator jq's default route
    // does (`yq_runner.rs`'s `evaluate_yaml_cursor`), while `--null-input`
    // goes through the `eval.rs`-level `eval()` route instead. Both were
    // independently affected by #693 (a masked error inside a `?`-wrapped
    // stream self-suppressed instead of stopping it) and both are exercised
    // here. Verified against jq 1.7.1 (`yq` itself has no `if`/`then`/`else`):
    // `jq -n '[1,2,3] | (.[] | if .==2 then error("boom") else . end)?'`
    // prints only `1`.

    // Default YAML input goes through the direct-cursor (generic) route.
    let (stdout, code) = run_yq_stdin(
        r#"(.[] | if .==2 then error("boom") else . end)?"#,
        "- 1\n- 2\n- 3\n",
        &[],
    )?;
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);

    // `--null-input` takes the OwnedValue/full-evaluator route.
    let (stdout, code) = run_yq_stdin(
        r#"[1,2,3] | (.[] | if .==2 then error("boom") else . end)?"#,
        "",
        &["-n"],
    )?;
    assert_eq!(stdout, "1\n");
    assert_eq!(code, 0);

    Ok(())
}

/// `path`/`parent`/`parent(n)`/`key` used to silently answer `[]`/`{}`/`null`
/// (the root-level defaults) whenever they weren't the very first pipe stage
/// (#554), because the CLI's streaming evaluator (`eval_generic.rs`, driving
/// both `jq` and `yq`) bridged only the bare trailing builtin to the full
/// evaluator, discarding the pipe structure `eval.rs`'s `needs_path_context`
/// routing needs to see. This is the only automated coverage of the fix on
/// the YAML/`YamlCursor` side of `eval_generic.rs` -- `jq_evaluator_parity_tests.rs`
/// only exercises the JSON side.
#[test]
fn test_path_context_builtins_across_pipe_stages_554() -> Result<()> {
    let (output, code) = run_yq_stdin(".a | path", "a: 1\n", &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["a"]"#);

    let (output, code) = run_yq_stdin(".a | parent", "a: 1\n", &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":1}"#);

    let (output, code) = run_yq_stdin(".a | key", "a: 1\n", &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""a""#);

    let (output, code) = run_yq_stdin(".a.b | parent", "a:\n  b: 1\n", &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"b":1}"#);

    Ok(())
}

/// `keys_unsorted` gained a lazy `GenericResult`/evaluator path shared with
/// `jq` (#140); `yq_runner.rs`'s CLI output boundary now streams it lazily
/// too, via `can_use_m2_streaming` admitting `Builtin::KeysUnsorted` and
/// `GenericResult::stream_json`/`stream_yaml`'s `LazyKeys { sorted: false, .. }` arms
/// writing each key straight from `fields` (#685) — no `Vec<String>` or
/// `OwnedValue::Array` is built. Covers every M2-reachable output shape
/// (compact/pretty JSON, compact/pretty YAML) plus `length`/`.[0]`, which
/// were already lazy before this issue.
#[test]
fn test_keys_unsorted_yaml_lazy_output_685() -> Result<()> {
    let input = "b: 1\na: 2\nc: 3\n";

    let (output, code) = run_yq_stdin("keys_unsorted", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["b","a","c"]"#);

    let (output, code) = run_yq_stdin("keys_unsorted", input, &["-o", "json", "-I2"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[\n  \"b\",\n  \"a\",\n  \"c\"\n]");

    let (output, code) = run_yq_stdin("keys_unsorted", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "- b\n- a\n- c");

    let (output, code) = run_yq_stdin("keys_unsorted | length", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "3");

    let (output, code) = run_yq_stdin("keys_unsorted | .[0]", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#""b""#);

    Ok(())
}

/// `stream_lazy_keys_json`/`stream_lazy_keys_yaml`'s empty-`fields` early
/// return (`"[]"`, skipping the `uncons()` loop entirely) is only reachable
/// with zero mapping entries -- `test_keys_unsorted_yaml_lazy_output_685`'s
/// fixture always has three, so it never exercises this arm (#685).
#[test]
fn test_keys_unsorted_yaml_lazy_output_empty_685() -> Result<()> {
    let input = "{}\n";

    let (output, code) = run_yq_stdin("keys_unsorted", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[]");

    let (output, code) = run_yq_stdin("keys_unsorted", input, &["-o", "json", "-I2"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[]");

    let (output, code) = run_yq_stdin("keys_unsorted", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[]");

    let (output, code) = run_yq_stdin("keys_unsorted", input, &["-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[]");

    Ok(())
}

// Note (#683): there is no sorted-`keys` CLI test mirroring
// `test_keys_unsorted_yaml_lazy_output_685` here, because `keys` is
// unreachable via the `yq` CLI's own dialect -- `run_yq` always parses in
// `ParserMode::Yq`, where the `keys` keyword itself resolves to
// `Builtin::KeysUnsorted` (matching real yq's document-order semantics; see
// `parser.rs`), so `yq 'keys'` and `yq 'keys_unsorted'` are already the same
// query and both exercised by the test above. The generic evaluator's
// `sorted: true` path is still real and reachable via the `jq` CLI/JSON
// (`test_keys_lazy_length_output_683`, `jq_cli_tests.rs`), and is exercised
// against a YAML value directly (bypassing the `yq` CLI's parser dialect) by
// `test_yaml_keys_sorted_lazy_length` in `eval_generic.rs`'s unit tests, to
// prove the `Pipe` dispatch fast path is generic over `V: DocumentValue`.

/// Array `keys`/`keys_unsorted` gained the same lazy `GenericResult` fast
/// paths as the object case (#684), and hits the same YAML-side materialize
/// fallback as `test_keys_unsorted_yaml_materialize_fallback_140` above.
#[test]
fn test_array_keys_unsorted_yaml_materialize_fallback_684() -> Result<()> {
    let input = "- x\n- y\n- z\n";

    let (output, code) = run_yq_stdin("keys", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1,2]");

    let (output, code) = run_yq_stdin("keys_unsorted", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[0,1,2]");

    let (output, code) = run_yq_stdin("keys_unsorted | length", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "3");

    let (output, code) = run_yq_stdin("keys_unsorted | .[0]", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "0");

    let (output, code) = run_yq_stdin("keys_unsorted | last", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "2");

    Ok(())
}

/// Flags that force the DOM path must still agree byte-for-byte with the M2
/// lazy path above — `evaluate_yaml_cursor`'s `LazyKeys { sorted: false, .. }`
/// arm ([yq_runner.rs]) stays a materializing fallback for those flag
/// combinations rather than a `keys_unsorted`-specific gap (#685).
///
/// `-I0` (compact) satisfies `can_json_fast_path`/`can_yaml_fast_path` on its
/// own (`output_config.compact || ...`), so `--sort-keys` alone does *not*
/// force DOM in compact mode — only in pretty mode, where it's excluded via
/// `can_stream_pretty`. `--arg` (an unused named variable) forces DOM
/// unconditionally via `context.named.is_empty()`, so it's used for the
/// compact case instead.
#[test]
fn test_keys_unsorted_yaml_dom_fallback_matches_lazy_685() -> Result<()> {
    let input = "b: 1\na: 2\nc: 3\n";

    let (lazy, _) = run_yq_stdin("keys_unsorted", input, &["-o", "json", "-I0"])?;
    let (dom, code) = run_yq_stdin(
        "keys_unsorted",
        input,
        &["--arg", "_unused", "x", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(lazy, dom);

    let (lazy, _) = run_yq_stdin("keys_unsorted", input, &[])?;
    let (dom, code) = run_yq_stdin("keys_unsorted", input, &["--sort-keys"])?;
    assert_eq!(code, 0);
    assert_eq!(lazy, dom);

    Ok(())
}

/// `-P`/`--prettyPrint` (#705) was previously read nowhere at all — parsed
/// into `YqCommand::pretty_print` and then silently ignored on every path.
/// It's now wired into `can_stream_pretty` (`yq_runner.rs`), forcing the DOM
/// fallback exactly like `--sort-keys` above (`test_keys_unsorted_yaml_dom_fallback_matches_lazy_685`).
///
/// #707 (flow-style preservation) landed on the M2 cursor-streaming path
/// only; the DOM fallback path `-P` forces had no style tracking at all
/// back then, so it always rendered block-style regardless of `-P`. #739
/// later gave the DOM path real style tracking too (`CommentTree`'s style
/// field, `evaluate_yaml_cursor`'s `reconcile_presentation`) — real `yq`'s
/// own doc says `-P` is "shorthand for `... style = \"\"`", i.e. a genuine
/// clear, not just "there's nothing to clear" — so `evaluate_yaml_cursor`'s
/// `strip_style` parameter now does that explicitly
/// (`strip_presentation_style`) whenever `args.pretty_print` is set. The
/// observable result is unchanged (block-style output either way); only
/// the reason changed, from "no style data exists here" to "style data
/// exists and `-P` clears it."
#[test]
fn test_pretty_print_flag_forces_block_style_705() -> Result<()> {
    let input = "a: [1, 2, 3]\nb: {c: 1, d: 2}\n";

    // YAML output: default preserves the input's flow style (#707); -P
    // forces the DOM path, which unconditionally renders block-style.
    let (default_out, code) = run_yq_stdin(".", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(default_out, "a: [1, 2, 3]\nb: {c: 1, d: 2}\n");
    let (pretty_out, code) = run_yq_stdin(".", input, &["-P"])?;
    assert_eq!(code, 0);
    assert_eq!(pretty_out, "a:\n  - 1\n  - 2\n  - 3\nb:\n  c: 1\n  d: 2\n");
    assert_ne!(default_out, pretty_out);

    // Long-flag form parses identically to -P.
    let (pretty_long_out, code) = run_yq_stdin(".", input, &["--prettyPrint"])?;
    assert_eq!(code, 0);
    assert_eq!(pretty_out, pretty_long_out);

    // JSON output: the DOM detour -P forces doesn't affect the JSON path.
    let (default_json, code) = run_yq_stdin(".", input, &["-o", "json"])?;
    assert_eq!(code, 0);
    let (pretty_json, code) = run_yq_stdin(".", input, &["-o", "json", "-P"])?;
    assert_eq!(code, 0);
    assert_eq!(default_json, pretty_json);

    // -I0 (compact) satisfies the fast-path gate on its own
    // (`output_config.compact || ...`), so -P alone does *not* force DOM in
    // compact mode — mirroring --sort-keys' documented compact-mode
    // exemption above. Still must produce identical output either way.
    let (default_compact, code) = run_yq_stdin(".", input, &["-I0"])?;
    assert_eq!(code, 0);
    let (pretty_compact, code) = run_yq_stdin(".", input, &["-I0", "-P"])?;
    assert_eq!(code, 0);
    assert_eq!(default_compact, pretty_compact);

    Ok(())
}

/// `keys_unsorted` on a mapping resolved through a `<<: *anchor` merge key
/// exercises `YamlFields`'s `Merged` variant (an `Rc`-shared entry list, the
/// reason `YamlFields` can't be `Copy` the way `JsonFields` is) through the
/// new lazy path — must still stream in merge-then-local order.
#[test]
fn test_keys_unsorted_yaml_merge_key_lazy_685() -> Result<()> {
    let input = "defaults: &defaults\n  b: 1\n  a: 2\nitem:\n  <<: *defaults\n  c: 3\n";

    let (output, code) = run_yq_stdin(".item | keys_unsorted", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"["b","a","c"]"#);

    let (output, code) = run_yq_stdin(".item | keys_unsorted", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "- b\n- a\n- c");

    Ok(())
}

// ============================================================================
// anchor/style builtins (#709) - previously hardcoded to always return ""
// ============================================================================

#[test]
fn test_anchor_builtin_returns_real_anchor_name() -> Result<()> {
    let input = "a: &x 1\nb: *x\n";
    let (output, code) = run_yq_stdin(".a | anchor", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "x");

    Ok(())
}

#[test]
fn test_anchor_builtin_empty_when_no_anchor() -> Result<()> {
    let input = "a: 1\n";
    let (output, code) = run_yq_stdin(".a | anchor", input, &["-o=json"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "\"\"");

    Ok(())
}

/// #835: the generic jq/yq evaluator navigates `.[n]`/`.[]` via
/// `DocumentElements::uncons_cursor` (the `DocumentElements` trait impl,
/// distinct from `YamlElements`' own inherent method of the same name,
/// which several internal callers need to stay raw/unresolved). Before this
/// fix that trait method hadn't been overridden to resolve a totally bare
/// `-` sequence-item wrapper, so `anchor` on a bare-dash-deferred anchored
/// value returned empty instead of the real name.
#[test]
fn test_anchor_builtin_bare_dash_deferred_anchor_835() -> Result<()> {
    let input = "-\n  &x\n  a: 1\n";
    let (output, code) = run_yq_stdin(".[0] | anchor", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "x");
    Ok(())
}

#[test]
fn test_style_builtin_flow_collection() -> Result<()> {
    let input = "a: [1, 2, 3]\n";
    let (output, code) = run_yq_stdin(".a | style", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "flow");

    Ok(())
}

#[test]
fn test_style_builtin_scalar_quote_styles() -> Result<()> {
    let cases = [
        ("a: \"hi\"\n", "\"double\""),
        ("a: 'hi'\n", "\"single\""),
        ("a: |\n  hi\n", "\"literal\""),
        ("a: >\n  hi\n", "\"folded\""),
        ("a: hi\n", "\"\""),
        ("a: {b: 1}\n", "\"flow\""),
    ];

    for (input, expected) in cases {
        let (output, code) = run_yq_stdin(".a | style", input, &["-o=json"])?;
        assert_eq!(code, 0, "input: {input:?}");
        assert_eq!(output.trim(), expected, "input: {input:?}");
    }

    Ok(())
}

/// An anchored scalar's `style` must still reflect its own style, not the
/// anchor indicator preceding it in the source text.
#[test]
fn test_style_builtin_anchor_prefix_does_not_mask_style() -> Result<()> {
    let input = "a: &x \"hi\"\n";
    let (output, code) = run_yq_stdin(".a | style", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "double");

    Ok(())
}

/// The DOM path's `evaluate_yaml_cursor` (`yq_runner.rs`) has its own
/// `GenericResult::LazySeq` arm. `can_use_m2_streaming` rejects
/// `Builtin::Map` outright, so any top-level `map(f)` query takes this DOM
/// fallback rather than the M2 streaming path -- no special flag needed
/// (unlike `keys_unsorted`, which the M2 path *does* accept) (#725).
/// Exercise all three outcomes (success, error, break) against the real
/// binary rather than relying on incidental coverage elsewhere.
#[test]
fn test_top_level_map_lazy_seq_dom_fallback_725() -> Result<()> {
    let input = "a: 1\nb: 2\nc: 3\n";

    let (output, code) = run_yq_stdin("map(. + 1)", input, &["-o", "json", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[2,3,4]");

    let (output, stderr, code) = run_yq_stdin_with_stderr("map(. + 1)", "a: 1\nb: two\n", &[])?;
    assert_eq!(code, 1);
    assert_eq!(output, "");
    assert!(stderr.contains("cannot be added"), "{stderr}");

    let (output, stderr, code) = run_yq_stdin_with_stderr("map(break $out)", input, &[])?;
    assert_eq!(code, 1);
    assert_eq!(output, "");
    assert!(stderr.contains("break $out not in label"), "{stderr}");

    Ok(())
}

// Trailing line comment preservation (#710). Every expected string here was
// verified byte-for-byte against the pinned real `yq` binary
// (tests/data/yq-golden/YQ_VERSION) before being pinned.

/// Identity on a document with a trailing comment must keep it verbatim,
/// with the gap before `#` normalized to one space (matching real `yq`,
/// which does the same regardless of the source's original spacing).
#[test]
fn test_identity_preserves_line_comment_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: 1 # keep this\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 1 # keep this\nb: 2\n");
    Ok(())
}

/// A scalar extracted alone (not as part of its parent mapping) does not
/// carry its former sibling comment - it belongs to the mapping entry, not
/// the bare value. Matches real `yq`.
#[test]
fn test_field_navigation_drops_the_comment_like_real_yq_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".a", "a: 1 # keep this\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "1");
    Ok(())
}

#[test]
fn test_sequence_item_comments_preserved_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "items:\n  - 1 # first\n  - 2 # second\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "items:\n  - 1 # first\n  - 2 # second\n");
    Ok(())
}

#[test]
fn test_nested_mapping_comment_preserved_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a:\n  b: 1 # nested\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a:\n  b: 1 # nested\n");
    Ok(())
}

/// A comment trailing a whole flow collection (not between its elements)
/// attaches to the field that owns it, same as a scalar value would.
#[test]
fn test_flow_collection_trailing_comment_preserved_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: [1, 2, 3] # flow comment\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: [1, 2, 3] # flow comment\n");
    Ok(())
}

/// #794: unlike the comment-after-the-closing-bracket case just above, a
/// comment between the *last element* and the closing bracket *on a
/// following line* used to be silently dropped from identity output
/// entirely (not merely reformatted differently). The parser already
/// attributes it to the last element (verified via the DOM path, which
/// showed it correctly before this fix - see #793's own repro); the bug was
/// that flow-style rendering never emitted an item's own trailing comment
/// at all. A newline before the closing bracket is required for validity -
/// `#` would otherwise consume the bracket into the comment text - so exact
/// formatting isn't expected to byte-match real `yq`'s own reformatting.
#[test]
fn test_flow_sequence_comment_before_closing_bracket_on_next_line_794() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: [1, 2, 3 # trailing\n]\n", &[])?;
    assert_eq!(code, 0);
    assert!(out.contains("# trailing"), "comment missing: {out:?}");
    // Must still be valid YAML that round-trips without losing the comment.
    let (out2, code2) = run_yq_stdin(".", &out, &[])?;
    assert_eq!(code2, 0);
    assert_eq!(out2, out);
    Ok(())
}

/// Same shape, but for a flow mapping rather than a flow sequence.
#[test]
fn test_flow_mapping_comment_before_closing_brace_on_next_line_794() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: {b: 1, c: 2 # trailing\n}\n", &[])?;
    assert_eq!(code, 0);
    assert!(out.contains("# trailing"), "comment missing: {out:?}");
    let (out2, code2) = run_yq_stdin(".", &out, &[])?;
    assert_eq!(code2, 0);
    assert_eq!(out2, out);
    Ok(())
}

/// Regression guard: a *single*-element flow collection with the comment
/// before the closing bracket on the next line hits the same "last item"
/// code path as the multi-element cases above.
#[test]
fn test_flow_sequence_single_element_comment_before_closing_bracket_794() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: [1 # trailing\n]\n", &[])?;
    assert_eq!(code, 0);
    assert!(out.contains("# trailing"), "comment missing: {out:?}");
    Ok(())
}

#[test]
fn test_quoted_scalar_comment_preserved_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: \"hello\" # quoted\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: \"hello\" # quoted\n");
    Ok(())
}

/// A cursor-preserving filter (stays on the P9/DOM path via a live cursor,
/// not a JSON round-trip) keeps comments too, not just bare identity.
#[test]
fn test_select_true_preserves_comments_710() -> Result<()> {
    let (out, code) = run_yq_stdin("select(true)", "a: 1 # keep this\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 1 # keep this\nb: 2\n");
    Ok(())
}

/// `-S`/`--sort-keys` forces the DOM output path but doesn't rebuild the
/// underlying `IndexMap`, just its display order - comments still resolve
/// by field name.
#[test]
fn test_sort_keys_preserves_comments_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "b: 2\na: 1 # keep this\n", &["-S"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 1 # keep this\nb: 2\n");
    Ok(())
}

/// `line_comment` getter: strips `# ` (hash + one space) when present.
#[test]
fn test_line_comment_builtin_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".a | line_comment", "a: 1 # keep this\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "keep this");
    Ok(())
}

/// No space after `#` - nothing to strip, matching real `yq`'s value. A
/// bare top-level scalar result drops its own styling unconditionally
/// (#852), so this prints raw `#keep this` even though an unquoted string
/// starting with `#` would look like a comment if re-parsed - real `yq`
/// does the identical thing (verified against the pinned binary).
#[test]
fn test_line_comment_builtin_no_space_after_hash_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".a | line_comment", "a: 1 #keep this\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "#keep this");
    Ok(())
}

/// No comment at all - the getter returns `""`, not `null` (matches real
/// `yq`'s value, verified empirically - this is not the same default as
/// `line`/`column`, which return `0`). Output is a blank line, not `''`:
/// this was the exact gap #852 fixed (a bare top-level empty-string result
/// used to be quoted, unlike real `yq`).
#[test]
fn test_line_comment_builtin_empty_when_absent_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".a | line_comment", "a: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\n");
    Ok(())
}

/// #739 (was a documented gap, #710): assignment (`=`, `|=`, ...) used to
/// fall through a JSON-round-trip fallback (`eval_generic.rs`'s catch-all)
/// that discarded cursor/comment data for the *entire* document - not just
/// the assigned field. `evaluate_yaml_cursor`'s `reconcile_presentation`
/// now recovers it: an untouched sibling keeps its comment (`.b = 5` here
/// never touches `a`), and - verified against the pinned real `yq` binary -
/// even the *written* field's own comment survives a same-kind value
/// change (`.a = 5` still keeps `a`'s comment; only a kind change, e.g.
/// scalar to object, would drop it).
#[test]
fn test_assignment_preserves_comments_739() -> Result<()> {
    let (out, code) = run_yq_stdin(".a = 5", "a: 1 # keep this\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 5 # keep this\nb: 2\n");

    let (out, code) = run_yq_stdin(".b = 5", "a: 1 # keep this\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 1 # keep this\nb: 5\n");
    Ok(())
}

/// A standalone comment on the line right after a block scalar's content
/// belongs to whatever follows it, not to the block scalar - it must not be
/// misattributed to `a`. `set_bp_text_end`'s generic same-line capture used
/// `self.pos`, which by the time block-scalar content parsing finishes has
/// already advanced past the block region (see
/// `set_bp_text_end_position`'s doc comment); this stole `b`'s comment and
/// attached it to `a`. Real `yq` drops this comment entirely (it's not a
/// same-line trailing comment for anything in this document, and
/// `head_comment` isn't implemented), so this pins the correct "drop, don't
/// steal" behavior rather than replicating misattribution.
#[test]
fn test_block_scalar_does_not_steal_following_comment_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: |\n  line one\n# comment for b\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    // `a`'s own scalar re-emits as `|` block style, not a quoted string
    // with `\n` escapes, since #836 - unrelated to what this test itself
    // pins (that the comment isn't misattributed to `a`'s value).
    assert_eq!(out, "a: |\n  line one\nb: 2\n");
    Ok(())
}

/// Same misattribution risk for an empty block scalar (no content lines at
/// all): `detect_block_content_indent` also leaves `self.pos` at the start
/// of the following line before returning `None`.
#[test]
fn test_empty_block_scalar_does_not_steal_following_comment_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: |\n# comment for b\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: \"\"\nb: 2\n");
    Ok(())
}

/// The block scalar's own trailing comment on the header line itself
/// (`| # text`, captured explicitly before content parsing) must still work
/// after splitting `set_bp_text_end` into capture/no-capture variants.
#[test]
fn test_block_scalar_header_comment_still_preserved_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".a | line_comment", "a: | # keep this\n  line one\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "keep this");
    Ok(())
}

// Root-node trailing comments (#710 follow-up). Every expected string here
// was verified byte-for-byte against the pinned real `yq` binary. Real `yq`
// has a quirk worth pinning explicitly: a comment trailing a *scalar*
// document root is dropped from output (though still readable via
// `line_comment`), but a comment trailing an *array/object* document root is
// kept - this is replicated exactly, not "improved into" a new divergence.

/// A comment trailing the whole document's own array/object root (not a
/// child field) was previously dropped everywhere - `emit_yaml_value`'s
/// scalar/root arms only ever append a *child's* comment, appended by the
/// child's parent during recursion; nothing appended the outermost value's
/// own comment. Matches real `yq`, which keeps this for container roots.
#[test]
fn test_root_array_comment_preserved_on_identity_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "[1, 2, 3] # trailing\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "[1, 2, 3] # trailing\n");
    Ok(())
}

#[test]
fn test_root_object_comment_preserved_on_identity_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "{a: 1} # trailing\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "{a: 1} # trailing\n");
    Ok(())
}

/// Same fix, but for `select(true)` rather than plain identity above.
///
/// Before #796, `select(...)` always fell through to the DOM path
/// (`output_value`'s `root_comment_suffix` in `yq_runner.rs`), which
/// reserializes every container to block style regardless of the source's
/// own style - so this used to assert the reformatted `"a: 1 # trailing"`.
/// #796 routes `select(...)` through the same cursor-native M2 path plain
/// identity already used, which preserves the source's original flow/block
/// style instead of always reformatting to block - so the expected output
/// here changed to match, and now agrees with real `yq` byte-for-byte
/// (verified against the pinned v4.53.3 binary), where the old block-style
/// expectation did not.
#[test]
fn test_root_array_comment_preserved_on_select_710() -> Result<()> {
    let (out, code) = run_yq_stdin("select(true)", "{a: 1} # trailing\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim_end(), "{a: 1} # trailing");
    Ok(())
}

/// A comment trailing a *scalar* document root is a known real-`yq` quirk
/// (verified empirically): it's dropped from output on both identity and
/// `select`, unlike an array/object root. `line_comment` still returns it
/// (the data isn't lost internally, just not re-emitted) - pinning this
/// exact behavior rather than "fixing" it into a new divergence from real
/// `yq`.
#[test]
fn test_root_scalar_comment_dropped_on_identity_matches_real_yq_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "42 # trailing\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "42");

    let (out, code) = run_yq_stdin("select(true)", "42 # trailing\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "42");

    let (out, code) = run_yq_stdin(". | line_comment", "42 # trailing\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "trailing");
    Ok(())
}

/// Field/index navigation extracts a bare value and must NOT gain the
/// parent-context comment, even where the M2 path shares its streaming
/// entry point with plain identity - `stream_yaml_as_document` (identity
/// only) vs. `stream_yaml` (bare navigated results) must stay distinct.
/// Matches real `yq`: `.a` on `a: 1 # keep this` outputs bare `1`.
#[test]
fn test_field_navigation_still_drops_comment_after_root_fix_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".a", "a: 1 # keep this\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "1");
    Ok(())
}

/// #793a: unlike a navigated *scalar* (dropped, see above - matches real
/// `yq`), a navigated *container* keeps its own trailing comment, the same
/// as when that container is the whole document's root. Before the fix,
/// `GenericResult::stream_yaml`'s `OneCursor`/`ManyCursor` arms always used
/// the bare (comment-less) `stream_yaml`, so `.a` alone silently dropped it
/// even though plain `.` on the same document already kept it.
#[test]
fn test_navigated_container_keeps_own_comment_793a() -> Result<()> {
    let (out, code) = run_yq_stdin(".a", "a: [1, 2, 3] # trailing\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "[1, 2, 3] # trailing\n");
    Ok(())
}

/// Same fix, but for a multi-result stream (`.[]`) rather than a single
/// navigated result - each streamed container result keeps its own comment
/// independently.
#[test]
fn test_iterated_containers_keep_own_comments_793a() -> Result<()> {
    let (out, code) = run_yq_stdin(".[]", "- [1, 2] # x\n- [3, 4] # y\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "[1, 2] # x\n[3, 4] # y\n");
    Ok(())
}

// #793b: on the DOM path (`OwnedValue` + `emit_yaml_value`, still reachable
// via flags like `--arg` that can't use the M2 fast path even after #796
// widened which queries can), a container's own trailing comment used to be
// concatenated directly onto its last child's rendered line with no
// separator - indistinguishable from that child's own comment. Fixed by
// giving the container's own comment a standalone comment line instead,
// for block-rendered output. #739 later taught this same DOM path to
// preserve a container's own flow-vs-block style (previously always
// forced to block, unconditionally, regardless of source): both of these
// inputs use flow syntax (`[1, 2, 3]`), so the container now renders on
// one line and its own comment glues onto that line instead - unambiguous
// since there's no separate "last child's line" to collide with. Both
// outputs verified against the pinned real `yq` binary.
// `--arg x y` is used as the M2-blocking flag throughout (rather than the
// original issue's `select(...)`, which #796 now routes through M2 and so
// no longer reaches this code at all for these shapes).

#[test]
fn test_dom_path_container_comment_gets_own_line_not_glued_to_last_child_793b() -> Result<()> {
    let (out, code) = run_yq_stdin(
        ".a",
        "a: [1, 2, 3] # trailing\nb: 2\n",
        &["--arg", "x", "y"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out, "[1, 2, 3] # trailing\n");
    Ok(())
}

#[test]
fn test_dom_path_root_container_comment_gets_own_line_793b() -> Result<()> {
    let (out, code) = run_yq_stdin(
        ".",
        "items: [1, 2, 3] # container comment\n",
        &["--arg", "x", "y"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out, "items: [1, 2, 3] # container comment\n");
    Ok(())
}

/// A child's own comment and the container's own comment must both survive,
/// as two distinct comments - not silently concatenated onto one line.
///
/// The source is flow syntax (`[1, 2 ...]`), but a `#` comment runs to end
/// of line and this one trails a non-final element, so there's nowhere for
/// it to go on one line without breaking flow's grammar; `is_flow_safe`
/// (#739) falls back to block rendering rather than lose the comment
/// (real `yq` instead keeps flow with a synthetic trailing comma before
/// the line break - a narrower fidelity gap this PR accepts, see
/// `is_flow_safe`'s own doc comment).
#[test]
fn test_dom_path_container_and_child_comments_stay_distinct_793b() -> Result<()> {
    let (out, code) = run_yq_stdin(
        ".",
        "items: [1, 2 # child\n] # container\n",
        &["--arg", "x", "y"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out, "items:\n  - 1\n  - 2 # child\n  # container\n");
    Ok(())
}

/// A comment trailing the first document's scalar root in a multi-document
/// stream is dropped by real `yq` too (verified empirically) - not a
/// regression to "fix" here.
#[test]
fn test_multi_doc_root_scalar_comment_dropped_matches_real_yq_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "42 # trailing\n---\nfoo: bar\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "42\n---\nfoo: bar\n");
    Ok(())
}

/// `-o json` never reads `CommentTree` (JSON has no comment syntax), so
/// `evaluate_yaml_cursor` skips building one for JSON output (#710
/// follow-up efficiency fix) - this just pins that the skip doesn't change
/// JSON output correctness for a document that does have comments.
#[test]
fn test_json_output_unaffected_by_comment_tree_skip_710() -> Result<()> {
    let (out, code) = run_yq_stdin("select(true)", "a: 1 # keep this\nb: 2\n", &["-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "{\n  \"a\": 1,\n  \"b\": 2\n}\n");
    Ok(())
}

/// A named variable (`--arg`) forces every document through
/// `evaluate_yaml_cursor`'s DOM-ish fallback (`can_yaml_fast_path` requires
/// `context.named.is_empty()`), same as a non-M2-streamable expression.
/// `keys_unsorted` on a top-level array still resolves to
/// `GenericResult::LazyIndexRange` there (#684) even though nothing forced
/// materialization for its own sake - this pins that fallback arm produces
/// the same `[0, 1, ..., len-1]` a plain `syq keys` would.
#[test]
fn test_keys_unsorted_lazy_index_range_via_dom_fallback_710() -> Result<()> {
    let (out, code) = run_yq_stdin("keys_unsorted", "- a\n- b\n- c\n", &["--arg", "x", "y"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "- 0\n- 1\n- 2\n");
    Ok(())
}

/// `--input-format json` (`-p json`) routes evaluation through
/// `evaluate_input`/`jq::eval` - the plain, cursor-generic-free evaluator in
/// `eval.rs`, distinct from `eval_generic.rs`'s `Builtin::LineComment` arm
/// the earlier `line_comment` tests in this file exercise via the normal
/// YAML M2 path. `builtin_line_comment` there is a permanent `""` regardless
/// of cursor, matching `builtin_line`/`builtin_column`'s same contract.
#[test]
fn test_line_comment_builtin_via_json_input_dom_path_710() -> Result<()> {
    let (out, code) = run_yq_stdin(".a | line_comment", "{\"a\": 1}", &["-p", "json"])?;
    assert_eq!(code, 0);
    // Default output stays YAML (`-p` only sets the *input* format); a
    // bare top-level empty-string result renders as a blank line, not
    // JSON's `""` or YAML's `''` (#852).
    assert_eq!(out, "\n");
    Ok(())
}

/// Same DOM path as above, but with `line_comment` reached only after
/// `def`-expansion substitutes a zero-arg function's body into the call
/// site - exercises `expand_func_calls_in_builtin`'s `Builtin::LineComment`
/// passthrough arm, which `eval.rs`'s plain evaluator (unlike
/// `eval_generic.rs`, which has no `def` AST-rewriting of its own) uses to
/// inline every `def` before evaluation.
#[test]
fn test_line_comment_builtin_through_def_expansion_710() -> Result<()> {
    let (out, code) = run_yq_stdin("def f: line_comment; .a | f", "{\"a\": 1}", &["-p", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\n");
    Ok(())
}

/// A parameterized `def` forces the call site's argument to be substituted
/// into the function body via `substitute_func_param_in_builtin` - which,
/// like `expand_func_calls_in_builtin` above, walks every `Builtin` node in
/// the body (including a `line_comment` that doesn't reference the param at
/// all) and must pass `Builtin::LineComment` through unchanged.
#[test]
fn test_line_comment_builtin_through_param_substitution_710() -> Result<()> {
    let (out, code) = run_yq_stdin(
        "def f($x): $x, line_comment; f(.a)",
        "{\"a\": 1}",
        &["-p", "json", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out, "1\n\"\"\n");
    Ok(())
}

// Key-scoped trailing line comment preservation (#765), a follow-up to
// #710: a comment trailing a mapping *key*'s own line, when the value is
// deferred to a following line (nested mapping/sequence), belongs to the
// key, not the value - #710's `line_comments`/`CommentTree` machinery was
// entirely value-node-scoped and dropped it silently. Every expected string
// here was verified byte-for-byte against the pinned real `yq` binary
// before being pinned, same as the #710 block above.

/// The issue's own repro: a key-trailing comment, value deferred to a
/// nested mapping.
#[test]
fn test_key_comment_preserved_with_nested_mapping_value_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: # comment on key\n  b: 1\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\n  b: 1\n");
    Ok(())
}

/// Same shape, but the deferred value is a nested sequence rather than a
/// nested mapping.
#[test]
fn test_key_comment_preserved_with_nested_sequence_value_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: # comment on key\n  - 1\n  - 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\n  - 1\n  - 2\n");
    Ok(())
}

/// Real `yq` doesn't expose this comment through any getter - it only
/// survives via full-tree re-serialization (see the issue's own
/// investigation). `line_comment` on the value (`.a`) or a descendant
/// (`.a.b`) must stay blank; this is the non-goal the issue explicitly
/// pins down. (`head_comment`/`foot_comment` aren't implemented by
/// succinctly at all - `Error: undefined function` - so there's nothing
/// to pin for them here.)
#[test]
fn test_key_comment_not_exposed_via_line_comment_getter_765() -> Result<()> {
    let input = "a: # comment on key\n  b: 1\n";

    let (out, code) = run_yq_stdin(".a | line_comment", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\n");

    let (out, code) = run_yq_stdin(".a.b | line_comment", input, &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "\n");

    Ok(())
}

/// The DOM/`CommentTree` path must also place the key's comment right
/// after `key:`, not just plain `.` on the M2 streaming path (`--arg`
/// forces the DOM path, since `select(...)` now routes through M2 after
/// #796 and would no longer exercise this code for a query shape this
/// simple - same reasoning as `test_explicit_key_comment_preserved_via_dom_path_795`).
#[test]
fn test_key_comment_preserved_via_dom_path_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: # comment on key\n  b: 1\n", &["--arg", "x", "y"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\n  b: 1\n");
    Ok(())
}

/// `-S`/`--sort-keys` reorders siblings but must still resolve the key
/// comment by field name, same as #710's value-comment equivalent
/// (`test_sort_keys_preserves_comments_710`).
#[test]
fn test_key_comment_preserved_with_sort_keys_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "z: 9\na: # comment on key\n  b: 1\n", &["-S"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\n  b: 1\nz: 9\n");
    Ok(())
}

/// Regression guard: a comment trailing a same-line (not deferred) value
/// keeps belonging to the value (#710), not the key - the new #765 capture
/// point is only reached when the value is deferred to a following line.
#[test]
fn test_key_comment_not_captured_for_same_line_value_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: 1 # keep this\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: 1 # keep this\nb: 2\n");
    Ok(())
}

// A key's deferred value can also resolve to nothing at all - the "next
// line" turns out to be a sibling key (at the same or a lower indent) or
// EOF, rather than the nested mapping/sequence the cases above cover. Real
// yq keeps the key's comment with no value token in every one of these
// shapes too; succinctly's first #765 pass only wired up the non-empty
// container case, silently dropping the comment here just like before the
// fix (verified byte-for-byte against the pinned real `yq` binary, same as
// every other block in this file).

/// A sibling key immediately follows at the same indent - `a`'s deferred
/// value is null.
#[test]
fn test_key_comment_preserved_with_null_value_sibling_same_indent_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: # comment on key\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\nb: 2\n");
    Ok(())
}

/// Same shape, but nested: the sibling that ends `a`'s deferred value sits
/// at a lower indent than `a` itself.
#[test]
fn test_key_comment_preserved_with_null_value_sibling_lower_indent_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "x:\n  a: # comment on key\ny: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "x:\n  a: # comment on key\ny: 2\n");
    Ok(())
}

/// The deferred key is the last thing in the document - EOF ends it,
/// leaving a null value.
#[test]
fn test_key_comment_preserved_with_null_value_at_eof_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: # comment on key\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\n");
    Ok(())
}

/// The DOM/`CommentTree` path (`--arg` forces it, since `select(...)` now
/// routes through M2 after #796 - see `test_key_comment_preserved_via_dom_path_765`
/// above) must also keep the comment for a null deferred value.
#[test]
fn test_key_comment_preserved_with_null_value_via_dom_path_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: # comment on key\nb: 2\n", &["--arg", "x", "y"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\nb: 2\n");
    Ok(())
}

/// `-S`/`--sort-keys` must resolve a null-valued key's comment by field
/// name too, same as `test_key_comment_preserved_with_sort_keys_765` above
/// for the non-empty-container case.
#[test]
fn test_key_comment_preserved_with_null_value_and_sort_keys_765() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "z: 9\na: # comment on key\nb: 2\n", &["-S"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: # comment on key\nb: 2\nz: 9\n");
    Ok(())
}

// ============================================================================
// Explicit-key (`? k ... : v`) trailing comment (#795)
// ============================================================================
//
// Distinct from #765 above: #765 covers the *implicit*-key form (`a: #
// comment\n  b: 1`, key/value on separate lines via indentation); this is
// the *explicit*-key form (`? k ... : v`), a different grammar production.
// The parser already captures the key's own comment generically (any
// scalar node close, including an explicit key) via the same side-table
// #710 added, but no write site read it back for a same-line scalar value
// until this fix - it was captured but never re-emitted anywhere.

/// The issue's own repro: an explicit key's trailing comment, with a
/// same-line scalar value - re-serialized to implicit single-line form,
/// same as real `yq`.
#[test]
fn test_explicit_key_comment_preserved_on_identity_795() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "? k # key comment\n: v\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "k: v # key comment\n");
    Ok(())
}

/// Same fix, but through the DOM/`CommentTree` path (`--arg` forces it,
/// since `select(...)` now routes through the M2 path after #796 and would
/// no longer exercise this code for a query shape this simple).
#[test]
fn test_explicit_key_comment_preserved_via_dom_path_795() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "? k # key comment\n: v\n", &["--arg", "x", "y"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "k: v # key comment\n");
    Ok(())
}

/// Regression guard: an ordinary implicit key with its own same-line value
/// comment is unaffected by the new key-comment fallback (the value's own
/// comment always takes priority; the key's own comment is only ever
/// present at all for the explicit-key shape above).
#[test]
fn test_implicit_key_same_line_value_comment_unaffected_by_795_fallback() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: v # normal\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: v # normal\n");
    Ok(())
}

/// Same regression guard, but through the DOM/`CommentTree` path (`--arg`
/// forces it, same as the other `_765`/`_795` DOM-path variants above) -
/// the value's own comment must win outright on this path too, rather than
/// falling back to a (non-existent, for this shape) key comment.
#[test]
fn test_implicit_key_same_line_value_comment_unaffected_by_795_fallback_dom_path() -> Result<()> {
    let (out, code) = run_yq_stdin(".", "a: v # normal\n", &["--arg", "x", "y"])?;
    assert_eq!(code, 0);
    assert_eq!(out, "a: v # normal\n");
    Ok(())
}

// ============================================================================
// Merge-flag suffixes on `*`/`*=` (#713)
// ============================================================================

/// Issue #713's own repro: unflagged array `*=` used to error outright
/// ("array (...) and array (...) cannot be multiplied") instead of doing
/// yq's default "rhs replaces lhs wholesale".
#[test]
fn test_713_array_merge_assign_no_longer_errors() -> Result<()> {
    let input = "a: [1, 2]\nb: [3, 4]\n";
    let (output, code) = run_yq_stdin(".a *= .b", input, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":[3,4],"b":[3,4]}"#);

    let (output, code) = run_yq_stdin(".a * .b", input, &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "[3,4]");

    Ok(())
}

/// Issue #713's own repro of the *malformed* flags-before-`=` spelling
/// (`*+=`, `*?=`). Real yq doesn't accept these either — flags belong after
/// `=`, not before — so succinctly should still fail to parse them, just no
/// longer with "unexpected character" (it now recognizes `*+`/`*?` as
/// tokens, same as real yq, and fails downstream instead).
#[test]
fn test_713_malformed_flags_before_equals_still_rejected() -> Result<()> {
    let (_, stderr, code) = run_yq_stdin_with_stderr(".a *+= .b", "a: [1, 2]\nb: [3, 4]\n", &[])?;
    assert_ne!(code, 0);
    assert!(
        !stderr.contains("unexpected character '+'"),
        "stderr: {stderr}"
    );

    let (_, stderr, code) =
        run_yq_stdin_with_stderr(".a *?= .b", "a:\n  x: 1\nb:\n  x: 2\n  y: 3\n", &[])?;
    assert_ne!(code, 0);
    assert!(
        !stderr.contains("unexpected character '?'"),
        "stderr: {stderr}"
    );

    Ok(())
}

/// The correct in-place spellings put flags after `=`: `*=+`, `*=n`, `*=nd`, ...
#[test]
fn test_713_merge_flags_after_equals() -> Result<()> {
    let (output, code) = run_yq_stdin(".a *=+ .b", "a: [1, 2]\nb: [3, 4]\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":[1,2,3,4],"b":[3,4]}"#);

    let (output, code) = run_yq_stdin(
        ".a *=n .b",
        "a:\n  thing: one\nb:\n  thing: two\n  missing: two\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(
        output.trim(),
        r#"{"a":{"thing":"one","missing":"two"},"b":{"thing":"two","missing":"two"}}"#
    );

    Ok(())
}

/// jq mode has no merge-flag syntax at all (real jq doesn't either) — this
/// must not regress just because yq mode gained it.
#[test]
fn test_713_jq_mode_unaffected() -> Result<()> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    cmd.arg("jq")
        .arg(".a * .b")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"a":[1,2],"b":[3,4]}"#)?;
    let output = child.wait_with_output()?;
    assert_ne!(
        output.status.code().unwrap_or(-1),
        0,
        "array * array must still error in jq mode"
    );

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"));
    cmd.arg("jq")
        .arg(".a *+ .b")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"a":[1,2],"b":[3,4]}"#)?;
    let output = child.wait_with_output()?;
    assert_ne!(
        output.status.code().unwrap_or(-1),
        0,
        "flag suffixes must still be unrecognized in jq mode"
    );

    Ok(())
}

/// A null (or absent, which evaluates to null) merge target is the most
/// common way `*=n`/`*=?` get invoked in practice. `arith_mul` must route
/// null operands through the same flag-gated merge machinery as a real
/// container pair instead of short-circuiting to `null` before flags apply
/// — otherwise `n`/`?` silently do nothing on a fresh/absent field, which is
/// exactly the case #713's own examples lead with. Expected values
/// cross-checked against real yq v4.53.3.
#[test]
fn test_713_merge_flags_on_null_or_absent_target() -> Result<()> {
    // `n` on an explicit null target: writes the full rhs, same as a null
    // nested field would.
    let (output, code) = run_yq_stdin(
        ".a *=n .b",
        "a: null\nb: {x: 1, y: 2}\n",
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":{"x":1,"y":2},"b":{"x":1,"y":2}}"#);

    // `?` on an absent target: never creates new fields, so it merges into
    // an empty object and every field is blocked, leaving `a: {}`.
    let (output, code) = run_yq_stdin(".a *=? .b", "b: {x: 1, y: 2}\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"b":{"x":1,"y":2},"a":{}}"#);

    // A null right operand is a no-op regardless of flags.
    let (output, code) = run_yq_stdin(".a *=+ null", "a: [1, 2]\n", &["-o=json", "-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), r#"{"a":[1,2]}"#);

    Ok(())
}

// ============================================================================
// --front-matter Tests (#715)
// ============================================================================

const FRONT_MATTER_FIXTURE: &str = "---\ntitle: My Post\ntags: [a, b]\n---\n# Body\n\nSome text.\n";

#[test]
fn test_front_matter_extract_evaluates_only_yaml() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    write!(input_file, "{FRONT_MATTER_FIXTURE}")?;

    let (output, code) = run_yq_file(
        ".title",
        input_file.path().to_str().unwrap(),
        &["--front-matter", "extract"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(output.trim(), "My Post");
    assert!(!output.contains("Body"));
    Ok(())
}

#[test]
fn test_front_matter_process_reattaches_body_verbatim() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    write!(input_file, "{FRONT_MATTER_FIXTURE}")?;

    let (output, code) = run_yq_file(
        ".title = \"New\"",
        input_file.path().to_str().unwrap(),
        &["--front-matter", "process"],
    )?;
    assert_eq!(code, 0);
    // `tags: [a, b]` keeps its flow style (#739) even though `.title` is
    // the field actually written - verified against the pinned real `yq`
    // binary.
    assert_eq!(
        output,
        "---\ntitle: New\ntags: [a, b]\n---\n# Body\n\nSome text.\n"
    );
    Ok(())
}

#[test]
fn test_front_matter_process_inplace_rewrites_file() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    write!(input_file, "{FRONT_MATTER_FIXTURE}")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["--front-matter", "process", "-i"])
        .arg(".title = \"New\"")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());

    let rewritten = std::fs::read_to_string(input_file.path())?;
    assert_eq!(
        rewritten,
        "---\ntitle: New\ntags:\n  - a\n  - b\n---\n# Body\n\nSome text.\n"
    );
    Ok(())
}

/// Regression test: `extract` mode captures no body to reattach (only
/// `process` does), so `--front-matter=extract -i` used to overwrite the
/// file with just the transformed front matter, silently discarding
/// everything after the closing fence (#715 follow-up).
#[test]
fn test_front_matter_extract_rejects_inplace() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    write!(input_file, "{FRONT_MATTER_FIXTURE}")?;
    let original = std::fs::read_to_string(input_file.path())?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["--front-matter", "extract", "-i"])
        .arg(".title = \"New\"")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--inplace"), "stderr: {stderr}");

    // The file must be left untouched, not partially overwritten.
    let unchanged = std::fs::read_to_string(input_file.path())?;
    assert_eq!(unchanged, original);
    Ok(())
}

#[test]
fn test_front_matter_no_fence_errors() -> Result<()> {
    let (_output, _stderr, code) =
        run_yq_stdin_with_stderr(".", "just: yaml\n", &["--front-matter", "extract"])?;
    assert_ne!(code, 0);
    Ok(())
}

#[test]
fn test_front_matter_unterminated_errors() -> Result<()> {
    let (_output, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        "---\ntitle: foo\nno closing fence\n",
        &["--front-matter", "extract"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("unterminated"), "stderr: {stderr}");
    Ok(())
}

/// Regression test: `apply_front_matter` always forces `InputFormat::Yaml`
/// once a mode is set (front matter is YAML by definition), but it used to
/// do so silently even when the caller explicitly asked for
/// `--input-format json` -- reject the contradictory combination instead
/// (#715 follow-up).
#[test]
fn test_front_matter_rejects_json_input_format() -> Result<()> {
    let (_output, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        FRONT_MATTER_FIXTURE,
        &["--front-matter", "extract", "--input-format", "json"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--input-format"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_front_matter_rejects_doc_flag() -> Result<()> {
    let (_output, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        FRONT_MATTER_FIXTURE,
        &["--front-matter", "extract", "--doc", "0"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--doc"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_front_matter_rejects_null_input() -> Result<()> {
    let (_output, stderr, code) =
        run_yq_stdin_with_stderr(".", "", &["--front-matter", "extract", "-n"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--null-input"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_front_matter_rejects_raw_input() -> Result<()> {
    let (_output, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        FRONT_MATTER_FIXTURE,
        &["--front-matter", "extract", "-R"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--raw-input"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_front_matter_process_rejects_slurp() -> Result<()> {
    let (_output, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        FRONT_MATTER_FIXTURE,
        &["--front-matter", "process", "-s"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--slurp"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_front_matter_process_rejects_json_output() -> Result<()> {
    let (_output, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        FRONT_MATTER_FIXTURE,
        &["--front-matter", "process", "-o", "json"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("json"), "stderr: {stderr}");
    Ok(())
}

/// Regression test: `output_value` treats anything other than `Yaml` as
/// JSON output (including `Auto`), but the compat guard only checked for
/// the explicit `Json` variant -- `-o auto` slipped through and wrapped a
/// JSON body in `---` fences (#715 follow-up).
#[test]
fn test_front_matter_process_rejects_auto_output() -> Result<()> {
    let (_output, stderr, code) = run_yq_stdin_with_stderr(
        ".",
        FRONT_MATTER_FIXTURE,
        &["--front-matter", "process", "-o", "auto"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("auto"), "stderr: {stderr}");
    Ok(())
}

/// Regression test: reattaching a body that doesn't end in a newline used
/// to run straight into the next file's opening fence with no separator,
/// e.g. `Body A---` -- corrupting the stream (#715 follow-up).
#[test]
fn test_front_matter_process_multi_file_separates_body_from_next_fence() -> Result<()> {
    let mut file1 = NamedTempFile::new()?;
    write!(file1, "---\ntitle: A\n---\nBody A")?; // no trailing newline
    let mut file2 = NamedTempFile::new()?;
    write!(file2, "---\ntitle: B\n---\nBody B\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["--front-matter", "process"])
        .arg(".title = \"X\"")
        .arg(file1.path())
        .arg(file2.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(
        stdout,
        "---\ntitle: X\n---\nBody A\n---\ntitle: X\n---\nBody B\n"
    );
    Ok(())
}

/// Regression test: the closing `---` fence injected right before a
/// reattached body was always LF-only, even when the body itself was CRLF
/// -- producing a file with mixed line endings (#715 follow-up).
#[test]
fn test_front_matter_process_preserves_crlf_line_endings() -> Result<()> {
    let (output, stderr, code) = run_yq_stdin_with_stderr(
        ".title = \"X\"",
        "---\r\ntitle: A\r\n---\r\nBody\r\ntext\r\n",
        &["--front-matter", "process"],
    )?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(output, "---\ntitle: X\n---\r\nBody\r\ntext\r\n");
    Ok(())
}

#[test]
fn test_front_matter_extract_allows_slurp_across_files() -> Result<()> {
    let mut file1 = NamedTempFile::new()?;
    write!(file1, "---\ntitle: One\n---\nBody one\n")?;
    let mut file2 = NamedTempFile::new()?;
    write!(file2, "---\ntitle: Two\n---\nBody two\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["--front-matter", "extract", "-s", "-o", "json", "-I0"])
        .arg(".")
        .arg(file1.path())
        .arg(file2.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), r#"[{"title":"One"},{"title":"Two"}]"#);
    Ok(())
}

/// A top-level `break` with no enclosing `label` reaches
/// `query_result_to_owned_values` as a bare `QueryResult::Break`, distinct
/// from the `Partial(_, Control::Break(_))` case an escaping break-after-
/// some-output takes -- exercised elsewhere via `--eval-all`'s
/// `write_split_result`, but not through the plain evaluation path.
#[test]
fn test_bare_break_outside_label_reports_error() -> Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-n", "break $foo"])
        .stdin(Stdio::null())
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("break $foo not in label"),
        "stderr: {stderr}"
    );
    Ok(())
}

// ============================================================================
// --split-exp Tests (#715)
// ============================================================================

fn run_yq_split(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, String, i32)> {
    run_yq_stdin_with_stderr(filter, input, extra_args)
}

#[test]
fn test_split_exp_writes_one_file_per_result_by_index() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/out_\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let (stdout, _stderr, code) = run_yq_split(
        ".[]",
        r#"[{"name":"a"},{"name":"b"},{"name":"c"}]"#,
        &["--split-exp", &pattern, "-p", "json"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "", "stdout must be suppressed on success");

    for (i, expected) in ["a", "b", "c"].into_iter().enumerate() {
        let content = std::fs::read_to_string(dir.path().join(format!("out_{i}.yml")))?;
        assert_eq!(content, format!("name: {expected}\n"));
    }
    Ok(())
}

#[test]
fn test_split_exp_non_string_result_errors() -> Result<()> {
    let (_stdout, stderr, code) =
        run_yq_split(".[]", "[1, 2]", &["--split-exp", "$index", "-p", "json"])?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains("must evaluate to a string"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn test_split_exp_dot_is_bound_to_result() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!("\"{}/\" + .name + \".yml\"", dir.path().display());
    let (_stdout, _stderr, code) = run_yq_split(
        ".[]",
        r#"[{"name":"a"},{"name":"b"}]"#,
        &["--split-exp", &pattern, "-p", "json"],
    )?;
    assert_eq!(code, 0);

    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.yml"))?,
        "name: a\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.yml"))?,
        "name: b\n"
    );
    Ok(())
}

/// Regression test: `--split-exp`'s expression was parsed independently of
/// the main filter and never received `--arg`/`--argjson`/`$ARGS`
/// substitution (only `$index` was bound), so a filename expression
/// referencing an `--arg` value failed as an undefined variable even though
/// the same `--arg` works fine for the main filter (#715 follow-up).
#[test]
fn test_split_exp_uses_arg_variable() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!("\"{}/\" + $prefix + .name + \".yml\"", dir.path().display());
    let (_stdout, stderr, code) = run_yq_split(
        ".[]",
        r#"[{"name":"a"}]"#,
        &[
            "--arg",
            "prefix",
            "pre_",
            "--split-exp",
            &pattern,
            "-p",
            "json",
        ],
    )?;
    assert_eq!(code, 0, "stderr: {stderr}");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("pre_a.yml"))?,
        "name: a\n"
    );
    Ok(())
}

#[test]
fn test_split_exp_and_slurp_incompatible() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(".", "{}", &["--split-exp", "\"f.yml\"", "-s"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--slurp"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_split_exp_and_inplace_incompatible() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: 1")?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["--split-exp", "\"f.yml\"", "-i"])
        .arg(".")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("--inplace"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_split_exp_and_raw_input_not_yet_supported() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(".", "hello", &["--split-exp", "\"f.yml\"", "-R"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("not yet supported"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_split_exp_and_front_matter_incompatible() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(
        ".",
        "---\na: 1\n---\nbody\n",
        &["--split-exp", "\"f.yml\"", "--front-matter", "extract"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--front-matter"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_split_exp_with_null_input() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-n", "--split-exp", &pattern])
        .arg("range(3)")
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());

    for (i, expected) in ["0", "1", "2"].into_iter().enumerate() {
        let content = std::fs::read_to_string(dir.path().join(format!("f{i}.yml")))?;
        assert_eq!(content.trim(), expected);
    }
    Ok(())
}

/// Regression test (#791 follow-up): `write_split_result`'s halt guard was
/// meant to catch a halt inside the *split-filename* expression, but
/// `sink.halted()` is sticky for the whole run, so once the *main*
/// expression halted with an output-bearing `Partial` prefix, every
/// subsequent `write_split_result` call misread the already-set flag as its
/// own and silently skipped writing a result that must still be split out.
/// `1, halt` produces one real output (`1`) before halting, so `f0.yml` must
/// still be written with that value -- not left missing.
#[test]
fn test_split_exp_writes_prefix_produced_before_main_expression_halts() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-n", "--split-exp", &pattern])
        .arg("1, halt")
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());

    let content = std::fs::read_to_string(dir.path().join("f0.yml"))?;
    assert_eq!(content.trim(), "1");
    Ok(())
}

#[test]
fn test_split_exp_duplicate_filename_warns_and_overwrites() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!("\"{}/const.yml\"", dir.path().display());
    let (_stdout, stderr, code) =
        run_yq_split(".[]", "[1, 2]", &["--split-exp", &pattern, "-p", "json"])?;
    assert_eq!(code, 0);
    assert!(
        stderr.contains("written more than once"),
        "stderr: {stderr}"
    );

    let content = std::fs::read_to_string(dir.path().join("const.yml"))?;
    assert_eq!(content.trim(), "2");
    Ok(())
}

/// Regression test: `ErrorSink::hit()` is sticky for the whole run, so
/// comparing it before/after each call only correctly detects "this call
/// just reported" for the very first error -- every later real error was
/// double-reported with a spurious extra "produced no output" message
/// (#715 follow-up). Fixed via `report_count()`, which can be compared
/// per-call regardless of earlier hits.
#[test]
fn test_split_exp_reports_each_error_exactly_once() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "if . == 1 then error(\"boom1\") elif . == 2 then error(\"boom2\") else \"{}/f3.yml\" end",
        dir.path().display()
    );
    let (_stdout, stderr, code) =
        run_yq_split(".[]", "[1, 2, 3]", &["--split-exp", &pattern, "-p", "json"])?;
    assert_ne!(code, 0);
    let error_lines: Vec<&str> = stderr.lines().filter(|l| l.starts_with("Error:")).collect();
    assert_eq!(
        error_lines,
        vec!["Error: boom1", "Error: boom2"],
        "stderr: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f3.yml"))?.trim(),
        "3"
    );
    Ok(())
}

#[test]
fn test_split_exp_output_format_respected() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/j\" + ($index|tostring) + \".json\"",
        dir.path().display()
    );
    let (_stdout, _stderr, code) = run_yq_split(
        ".[]",
        r#"[{"a":1},{"a":2}]"#,
        &["--split-exp", &pattern, "-p", "json", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);

    assert_eq!(
        std::fs::read_to_string(dir.path().join("j0.json"))?.trim(),
        r#"{"a":1}"#
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("j1.json"))?.trim(),
        r#"{"a":2}"#
    );
    Ok(())
}

/// Regression test: `.` alone is `is_m2_streamable`, so without the
/// `split_expr.is_none()` guard on the M2 fast-path gates, `--split-exp`
/// combined with an identity filter would silently stream straight to
/// stdout instead of writing the split file (#715).
#[test]
fn test_split_exp_with_identity_filter_not_bypassed_by_m2() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!("\"{}/identity_out.yml\"", dir.path().display());
    let (stdout, _stderr, code) = run_yq_split(".", "a: 1\n", &["--split-exp", &pattern])?;
    assert_eq!(code, 0);
    assert_eq!(
        stdout, "",
        "stdout must stay empty; M2 fast path must not bypass split-exp"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("identity_out.yml"))?,
        "a: 1\n"
    );
    Ok(())
}

#[test]
fn test_split_exp_empty_result_errors() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(".", "a: 1\n", &["--split-exp", "empty"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("produced no output"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_split_exp_many_results_errors() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(".", "a: 1\n", &["--split-exp", "$index, $index"])?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains("exactly one string") && stderr.contains("2 results"),
        "stderr: {stderr}"
    );
    Ok(())
}

/// Regression coverage for `write_split_result`'s own `std::fs::write`
/// failure path (distinct from the filename-evaluation error paths above):
/// a directory that doesn't exist makes the actual file write fail, which
/// must surface as a normal CLI error rather than a panic, across all three
/// input-gathering branches (`-n`/null-input, YAML, JSON).
#[test]
fn test_split_exp_write_failure_reports_error_null_input() -> Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-n", "--split-exp", "\"/nonexistent-dir-715/out.yml\""])
        .arg("1")
        .stdin(Stdio::null())
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(
        stderr.contains("failed to write --split-exp output file"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn test_split_exp_write_failure_reports_error_yaml_input() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(
        ".",
        "a: 1\n",
        &["--split-exp", "\"/nonexistent-dir-715/out.yml\""],
    )?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains("failed to write --split-exp output file"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn test_split_exp_write_failure_reports_error_json_input() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(
        ".",
        "{\"a\":1}",
        &[
            "--split-exp",
            "\"/nonexistent-dir-715/out.yml\"",
            "-p",
            "json",
        ],
    )?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains("failed to write --split-exp output file"),
        "stderr: {stderr}"
    );
    Ok(())
}

/// `--split-exp` combined with `--doc`/`document` filtering for JSON input:
/// with two JSON files and `--doc 1`, the first file's document must be
/// skipped (not written) and only the second file's document processed.
#[test]
fn test_split_exp_document_filter_skips_earlier_json_files() -> Result<()> {
    let dir = TempDir::new()?;
    let mut file1 = NamedTempFile::new()?;
    write!(file1, "{{\"which\":\"one\"}}")?;
    let mut file2 = NamedTempFile::new()?;
    write!(file2, "{{\"which\":\"two\"}}")?;
    let pattern = format!("\"{}/out.json\"", dir.path().display());

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args([
            "--split-exp",
            &pattern,
            "-p",
            "json",
            "-o",
            "json",
            "-I0",
            "--doc",
            "1",
        ])
        .arg(".")
        .arg(file1.path())
        .arg(file2.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());

    let content = std::fs::read_to_string(dir.path().join("out.json"))?;
    assert_eq!(content.trim(), r#"{"which":"two"}"#);
    Ok(())
}

/// `--split-exp` reading from stdin with `--validate` set must reject
/// invalid YAML before ever reaching the split-write loop.
#[test]
fn test_split_exp_validate_rejects_invalid_yaml_from_stdin() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(
        ".",
        "a: b: c: d\n",
        &["--split-exp", "\"f.yml\"", "--validate"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("validation error"), "stderr: {stderr}");
    Ok(())
}

/// Same as above, but reading from a file (exercises the file-gathering
/// branch of `--split-exp`'s input collection rather than the stdin one).
#[test]
fn test_split_exp_validate_rejects_invalid_yaml_from_file() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    writeln!(input_file, "a: b: c: d")?;
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["--split-exp", "\"f.yml\"", "--validate"])
        .arg(".")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.contains("validation error"), "stderr: {stderr}");
    Ok(())
}

/// A hard parse failure (as opposed to `--validate`'s opt-in strict check)
/// from the loose YAML loader itself, e.g. tab-indentation, must still
/// surface as a normal CLI error under `--split-exp`'s YAML branch, not a
/// panic -- exercises `evaluate_yaml_direct_filtered`'s own `Err` path
/// rather than `write_split_result`'s.
#[test]
fn test_split_exp_hard_yaml_parse_error_propagates() -> Result<()> {
    let (_stdout, stderr, code) = run_yq_split(".", "a:\n\t- 1\n", &["--split-exp", "\"f.yml\""])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("YAML parse error"), "stderr: {stderr}");
    Ok(())
}

// ============================================================================
// --eval-all / file_index Tests (#715)
// ============================================================================

fn run_yq_files(
    filter: &str,
    files: &[&std::path::Path],
    extra_args: &[&str],
) -> Result<(String, String, i32)> {
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(extra_args)
        .arg(filter)
        .args(files)
        .stdin(Stdio::null())
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    let stderr = String::from_utf8(output.stderr)?;
    let code = output.status.code().unwrap_or(-1);
    Ok((stdout, stderr, code))
}

fn two_doc_fixtures() -> Result<(NamedTempFile, NamedTempFile)> {
    let mut f1 = NamedTempFile::new()?;
    write!(f1, "a: 1\nname: first\n")?;
    let mut f2 = NamedTempFile::new()?;
    write!(f2, "b: 2\nname: second\n")?;
    Ok((f1, f2))
}

#[test]
fn test_eval_all_combines_documents_across_files() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files("length", &[f1.path(), f2.path()], &["--eval-all"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "2");
    Ok(())
}

/// `--eval-all` reading a single document from stdin (no input files at
/// all) is a distinct input-gathering branch from the file-list one every
/// other `--eval-all` test above exercises.
#[test]
fn test_eval_all_works_from_stdin() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(".", "a: 1\n", &["--eval-all", "-o", "json", "-I0"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), r#"[{"a":1}]"#);
    Ok(())
}

#[test]
fn test_eval_all_ea_alias() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files("length", &[f1.path(), f2.path()], &["--ea"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "2");
    Ok(())
}

#[test]
fn test_eval_all_file_index_bare() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        ".[] | file_index",
        &[f1.path(), f2.path()],
        &["--eval-all", "-o", "json"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "0\n1");
    Ok(())
}

/// Regression test for #822: `Expr::Arithmetic` inside
/// `eval_pipe_with_path_context_internal` used to collapse a multi-output
/// operand to its first value whenever the pipe also needed path context
/// (e.g. shared a comma with `file_index`) -- the same #768 bug class, in a
/// call site #768 didn't touch.
#[test]
fn test_eval_all_arithmetic_fanout_survives_file_index_in_pipe_issue_822() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        ".[] | ((1,2,3) + 1), file_index",
        &[f1.path(), f2.path()],
        &["--eval-all", "-o", "json"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "2\n3\n4\n0\n2\n3\n4\n1");
    Ok(())
}

/// Same #822 gap, for `Expr::Compare`.
#[test]
fn test_eval_all_compare_fanout_survives_file_index_in_pipe_issue_822() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        ".[] | ((1,2,3) > 1), file_index",
        &[f1.path(), f2.path()],
        &["--eval-all", "-o", "json"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "false\ntrue\ntrue\n0\nfalse\ntrue\ntrue\n1");
    Ok(())
}

/// The headline `--eval-all` idiom -- regression test for the
/// `needs_path_context`/`Select` runtime-arm fix (#715).
#[test]
fn test_eval_all_file_index_select() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        ".[] | select(file_index == 0)",
        &[f1.path(), f2.path()],
        &["--eval-all"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a: 1\nname: first\n");
    Ok(())
}

#[test]
fn test_eval_all_file_index_select_then_field() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        ".[] | select(file_index == 0) | .name",
        &[f1.path(), f2.path()],
        &["--eval-all"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "first");
    Ok(())
}

/// Regression test: `needs_path_context` recurses into `Expr::If`, but
/// `eval_pipe_with_path_context_internal` had no matching arm, so it fell
/// into the generic (non-path-context) fallback and silently lost
/// `file_index` for anything nested in `then`/`else` (#715 follow-up).
#[test]
fn test_eval_all_file_index_in_if_then_else() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        r#".[] | if file_index == 0 then "from-f1" else "from-f2" end"#,
        &[f1.path(), f2.path()],
        &["--eval-all"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "from-f1\n---\nfrom-f2\n");
    Ok(())
}

/// Regression test: same gap as `test_eval_all_file_index_in_if_then_else`,
/// for `Expr::Try` (#715 follow-up).
#[test]
fn test_eval_all_file_index_in_try_catch() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        r#".[] | try select(file_index == 1) catch "err""#,
        &[f1.path(), f2.path()],
        &["--eval-all"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "b: 2\nname: second\n");
    Ok(())
}

/// Regression test: `label $x | ...` is otherwise inert, but wrapping a
/// `file_index`-using pipe in one silently dropped path context on both the
/// `needs_path_context` predicate and the interpreter side (#715 follow-up).
#[test]
fn test_eval_all_file_index_survives_label_wrapper() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        "label $out | .[] | file_index",
        &[f1.path(), f2.path()],
        &["--eval-all", "-o", "json"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "0\n1");
    Ok(())
}

/// Regression test: same gap as `test_eval_all_file_index_in_if_then_else`,
/// for `map(...)` -- `file_index` silently stubbed to 0 for every element
/// instead of resolving, so `map(select(file_index == 0))` returned every
/// document from every file instead of filtering to file 0's (#715
/// follow-up).
#[test]
fn test_eval_all_file_index_in_map() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        "map(select(file_index == 0))",
        &[f1.path(), f2.path()],
        &["--eval-all"],
    )?;
    assert_eq!(code, 0);
    // Renders in real yq's "compact" form (#785): `- ` shares its line
    // with the mapping's own first field.
    assert_eq!(stdout, "- a: 1\n  name: first\n");
    Ok(())
}

/// `map(f)` is `[.[] | f]`, so it must stay atomic on error like real array
/// construction (`[1,error("x"),3]` produces no output at all) rather than
/// leaking an in-progress array (#715 follow-up).
#[test]
fn test_eval_all_file_index_in_map_is_atomic_on_error() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, stderr, code) = run_yq_files(
        r#"map(if file_index == 0 then error("boom") else . end)"#,
        &[f1.path(), f2.path()],
        &["--eval-all"],
    )?;
    assert_eq!(stdout, "");
    assert!(stderr.contains("boom"), "expected the error, got: {stderr}");
    assert_eq!(code, 1);
    Ok(())
}

#[test]
fn test_eval_all_file_index_outside_eval_all_is_zero() -> Result<()> {
    let (f1, _f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files("file_index", &[f1.path()], &[])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "0");
    Ok(())
}

#[test]
fn test_eval_all_file_index_camelcase_and_short_alias() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    for keyword in ["fileIndex", "fi"] {
        let (stdout, _stderr, code) = run_yq_files(
            &format!(".[] | {keyword}"),
            &[f1.path(), f2.path()],
            &["--eval-all", "-o", "json"],
        )?;
        assert_eq!(code, 0, "keyword: {keyword}");
        assert_eq!(stdout.trim(), "0\n1", "keyword: {keyword}");
    }
    Ok(())
}

#[test]
fn test_eval_all_reduce_merge() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        "reduce .[] as $item ({}; . * $item)",
        &[f1.path(), f2.path()],
        &["--eval-all", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"{"a":1,"name":"second","b":2}"#);
    Ok(())
}

/// The approximated real-yq merge idiom -- correct only when each file
/// contributes exactly one matching top-level document (#715). Also the
/// regression test for the `Expr::Arithmetic` path-context runtime-arm fix:
/// without it, both `select`s evaluate against the 0-stub and the
/// `file_index==1` side comes back empty ("no value" error).
#[test]
fn test_eval_all_select_star_merge_single_doc_per_file() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(
        "(.[] | select(file_index == 0)) * (.[] | select(file_index == 1))",
        &[f1.path(), f2.path()],
        &["--eval-all", "-o", "json", "-I0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"{"a":1,"name":"second","b":2}"#);
    Ok(())
}

#[test]
fn test_eval_all_doc_separator_between_results() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) = run_yq_files(".[]", &[f1.path(), f2.path()], &["--eval-all"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a: 1\nname: first\n---\nb: 2\nname: second\n");
    Ok(())
}

/// Regression test: `--eval-all` never routed through `SplitDocState` (the
/// state machine every other output path uses to honor an explicit
/// `split_doc` marker), so `--eval-all '... | split_doc'` silently merged
/// every result with zero `---` separators instead of one per result
/// (#715 follow-up).
#[test]
fn test_eval_all_split_doc_emits_separators() -> Result<()> {
    let (f1, f2) = two_doc_fixtures()?;
    let (stdout, _stderr, code) =
        run_yq_files(".[] | split_doc", &[f1.path(), f2.path()], &["--eval-all"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "a: 1\nname: first\n---\nb: 2\nname: second\n");
    Ok(())
}

#[test]
fn test_eval_all_doc_flag_interaction() -> Result<()> {
    let mut multi = NamedTempFile::new()?;
    write!(multi, "x: 1\n---\nx: 2\n")?;
    let (stdout, _stderr, code) =
        run_yq_files(".[] | .x", &[multi.path()], &["--eval-all", "--doc", "1"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "2");
    Ok(())
}

#[test]
fn test_eval_all_rejects_slurp() -> Result<()> {
    let (f1, _f2) = two_doc_fixtures()?;
    let (_stdout, stderr, code) = run_yq_files(".", &[f1.path()], &["--eval-all", "-s"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--slurp"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_eval_all_rejects_inplace() -> Result<()> {
    let (f1, _f2) = two_doc_fixtures()?;
    let (_stdout, stderr, code) = run_yq_files(".", &[f1.path()], &["--eval-all", "-i"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--inplace"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_eval_all_rejects_raw_input() -> Result<()> {
    let (f1, _f2) = two_doc_fixtures()?;
    let (_stdout, stderr, code) = run_yq_files(".", &[f1.path()], &["--eval-all", "-R"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--raw-input"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_eval_all_rejects_split_exp() -> Result<()> {
    let (f1, _f2) = two_doc_fixtures()?;
    let (_stdout, stderr, code) = run_yq_files(
        ".",
        &[f1.path()],
        &["--eval-all", "--split-exp", "\"f.yml\""],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--split-exp"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn test_eval_all_rejects_front_matter() -> Result<()> {
    let (f1, _f2) = two_doc_fixtures()?;
    let (_stdout, stderr, code) = run_yq_files(
        ".",
        &[f1.path()],
        &["--eval-all", "--front-matter", "extract"],
    )?;
    assert_ne!(code, 0);
    assert!(stderr.contains("--front-matter"), "stderr: {stderr}");
    Ok(())
}

/// `--eval-all` reading from stdin with `--validate` set must reject invalid
/// YAML before combining it into the evaluation array.
#[test]
fn test_eval_all_validate_rejects_invalid_yaml_from_stdin() -> Result<()> {
    let (_stdout, stderr, code) =
        run_yq_stdin_with_stderr(".", "a: b: c: d\n", &["--eval-all", "--validate"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("validation error"), "stderr: {stderr}");
    Ok(())
}

/// Same as above, but reading from files (exercises the file-gathering
/// branch of `--eval-all`'s input collection rather than the stdin one).
#[test]
fn test_eval_all_validate_rejects_invalid_yaml_from_file() -> Result<()> {
    let mut bad_file = NamedTempFile::new()?;
    writeln!(bad_file, "a: b: c: d")?;
    let (_stdout, stderr, code) =
        run_yq_files(".", &[bad_file.path()], &["--eval-all", "--validate"])?;
    assert_ne!(code, 0);
    assert!(stderr.contains("validation error"), "stderr: {stderr}");
    Ok(())
}

/// Regression test for the `needs_path_context`/`Compare` runtime-arm fix
/// (#715): `key` used inside `select(...)`'s condition previously produced
/// no output at all, independent of `--eval-all`.
#[test]
fn test_key_inside_select_regression() -> Result<()> {
    let (stdout, code) = run_yq_stdin(".[] | select(key >= 1)", "[10, 20, 30]", &["-o", "json"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "20\n30");
    Ok(())
}

/// Regression test for the same fix, via `document_index` (yq's existing
/// builtin) inside a comparison instead of `key` inside `select`.
#[test]
fn test_document_index_inside_comparison_regression() -> Result<()> {
    let (stdout, code) = run_yq_stdin("select(document_index == 1)", "a: 1\n---\nb: 2\n", &[])?;
    assert_eq!(code, 0);
    assert_eq!(stdout, "b: 2\n");
    Ok(())
}

/// Targets `absorb_stream_stats`'s `else if let Some(err) = &stats.error`
/// arm (yq_runner.rs) -- the #791 refactor that pulled the M2 YAML/JSON
/// streaming macro's halt-vs-error precedence check out of two duplicated
/// inline copies into this one shared helper, called from both the YAML and
/// JSON branches of `stream_cursor!`. `.foo` is M2-streamable (`Expr::
/// Field`), so applying it to a YAML sequence reaches the cursor streamer
/// directly instead of the DOM path; the resulting type error must still
/// surface through `report_stream` to stderr now that the check lives in a
/// shared function instead of inline in the macro, not vanish or leak to
/// stdout.
#[test]
fn test_m2_field_type_error_reported_via_absorb_stream_stats() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr(".foo", "- 1\n- 2\n", &[])?;
    assert_eq!(code, 1, "stderr: {stderr}");
    assert_eq!(stdout, "", "M2 stream error must not leak to stdout");
    assert_eq!(
        stderr.trim_end(),
        "Error: Cannot index array with string \"foo\""
    );
    Ok(())
}

/// #791 follow-up, two adjacent sites in one scenario. `write_split_result`'s
/// own halt guard (`if !halted_before && sink.halted().is_some() { return
/// Ok(()) }`) is meant to catch a halt inside *this call's own*
/// split-filename expression and return quietly (no "produced no output"
/// complaint); the outer per-result loop's own check right after the call
/// (`if sink.halted().is_some() { break 'files; }`, the YAML branch's copy)
/// then stops the loop before any later result is reached. `.[]` over a
/// 3-element array yields three results from one document, so making the
/// split expression itself halt on `$index == 1` fires mid-loop with a real
/// result (index 2) still pending -- proving both the early return and the
/// break are immediate, not merely "happened to be last anyway".
#[test]
fn test_split_exp_own_halt_returns_early_and_stops_further_results() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "if $index == 1 then halt else \"{}/f\" + ($index|tostring) + \".yml\" end",
        dir.path().display()
    );
    let (stdout, stderr, code) = run_yq_split(".[]", "[1, 2, 3]\n", &["--split-exp", &pattern])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f0.yml"))?.trim(),
        "1"
    );
    assert!(
        !dir.path().join("f1.yml").exists(),
        "index 1's own split-exp halt must return early without writing a file or complaining"
    );
    assert!(
        !dir.path().join("f2.yml").exists(),
        "index 2 must never be reached once index 1's split expression halted"
    );
    Ok(())
}

/// Sibling of the test above, covering the case its own doc comment didn't:
/// the split-filename expression's own halt can carry a *produced* value
/// with it (a comma expression like `filename, halt`), not just an empty
/// one. `write_split_result`'s halt guard must only skip writing when the
/// halt left nothing behind -- a legitimately-produced filename must still
/// reach the match below and get written, or the result is silently lost
/// with exit code 0 and no diagnostic at all.
#[test]
fn test_split_exp_own_halt_with_produced_value_still_writes_file() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\", halt",
        dir.path().display()
    );
    let (stdout, stderr, code) = run_yq_split(".[]", "[1, 2, 3]\n", &["--split-exp", &pattern])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f0.yml"))?.trim(),
        "1",
        "the filename produced before the halt must still be written to"
    );
    assert!(
        !dir.path().join("f1.yml").exists(),
        "index 1 must never be reached once index 0's split expression halted"
    );
    Ok(())
}

/// Targets `evaluate_yaml_cursor`'s `GenericResult::LazySeq` arm (#791
/// follow-up): `seq.materialize_atomic()`'s `Err(jq::Control::Halt(code))`
/// must reach `sink.request_halt`, not be swallowed as an ordinary error.
/// `keys_unsorted | map(f)` takes the composability `LazySeq` fast path
/// (#724/#725, native to `eval_single`'s `Pipe` folding) rather than falling
/// into `evaluate_yaml_cursor`'s generic `Partial`/`Error` handling, so a
/// halt inside `f` is the only way to reach this specific arm instead of the
/// ordinary `Error`/`Break` ones right next to it. `map(f)` is atomic array
/// construction, so the halt must also discard whatever partial array was
/// being built, matching real jq's `[1,error("x"),3]` semantics.
#[test]
fn test_keys_unsorted_map_halt_propagates_through_lazyseq() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("keys_unsorted | map(halt)", "a: 1\nb: 2\n", &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout, "",
        "a halt inside map must discard the whole in-progress array, not leak a partial one"
    );
    Ok(())
}

/// Targets `evaluate_yaml_cursor`'s `GenericResult::Partial(vs,
/// jq::Control::Halt(code))` arm (#791 follow-up), distinct from its
/// `Error`/`Break` siblings right above it. `,` isn't handled natively by
/// `eval_single`, so it falls back to `full_eval`'s DOM path and comes back
/// wrapped as a `GenericResult::Partial`, carrying whatever output was
/// produced before the halt fired. `1, halt` produces one real output
/// before halting, so it must still print `1` -- the outputs-already-
/// produced-don't-vanish rule (#400, #494) applied to a halt specifically,
/// not the ordinary-error case the neighboring arms handle.
#[test]
fn test_yaml_cursor_partial_result_keeps_prefix_output_before_halt() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr("1, halt", "x: 1\n", &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout, "1\n",
        "prefix output produced before the halt must survive"
    );
    Ok(())
}

/// #791 sibling of `test_split_doc_hides_in_halt_error_argument`:
/// `contains_split_doc`'s `Expr::Builtin(b) => match b { ... }` fan-in
/// gained a new `Builtin::HaltErrorCode(e)` leg spliced in ahead of the
/// pre-existing `Builtin::Has(e)` leg, turning `Has`'s own line into a fresh
/// OR-pattern alternative in the diff even though its recursion logic
/// (`contains_split_doc(e)`) is unchanged. `has(split_doc)` inside a dead
/// `if false` branch (never actually evaluated, exactly like the sibling
/// `halt_error` test) proves the static scan still walks into `has(...)`'s
/// own argument and correctly reports `has_split_doc == true`, which
/// disables the DOM path's own regular multi-doc `---` injection so no
/// leading separator appears before doc 0.
#[test]
fn test_split_doc_hides_in_has_argument() -> Result<()> {
    let yaml = "x: 1\n---\nx: 2\n";
    let (output, code) = run_yq_stdin("if false then has(split_doc) else . end", yaml, &[])?;
    assert_eq!(code, 0);
    assert_eq!(
        output, "x: 1\n---\nx: 2\n",
        "no leading separator before doc 0"
    );
    Ok(())
}

/// #791 follow-up: the M2 fast path's *real-files* loop (`'m2_files: for
/// file_path in &input_files`, a separate copy of the loop body from the
/// stdin case right above it) needed its own halt check after
/// `stream_cursor!` for the same reason the stdin copy does -- without it, a
/// halt inside `select(...)`'s predicate (the only shape that reaches the M2
/// path with a halt at all; `can_use_m2_streaming` special-cases
/// `Builtin::Select` regardless of its predicate's shape, #796) would keep
/// streaming the rest of this file's documents and then move on to open the
/// next file entirely. Two real files (not stdin) exercises this loop
/// specifically, distinct from the stdin copy.
#[test]
fn test_m2_multi_file_select_halt_stops_further_files() -> Result<()> {
    let mut f1 = NamedTempFile::new()?;
    write!(f1, "a: 1\n---\na: 2\n---\na: 3\n")?;
    let mut f2 = NamedTempFile::new()?;
    writeln!(f2, "a: 9")?;

    let (stdout, stderr, code) = run_yq_files(
        "select(if .a == 2 then halt else true end)",
        &[f1.path(), f2.path()],
        &[],
    )?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout, "a: 1\n",
        "halt inside f1's second document must stop streaming before f2 is ever opened"
    );
    Ok(())
}

/// #791 follow-up: `--split-exp`'s YAML per-file loop checks `sink.halted()`
/// twice: once inside the per-result loop (right after each
/// `write_split_result` call, covered by
/// `test_split_exp_own_halt_returns_early_and_stops_further_results`), and
/// once again after that loop, for the case where a *document's own*
/// evaluation halts before producing any output at all -- so the per-result
/// loop for that document runs zero times and the first check never fires.
/// Unconditional `halt` on doc0 of a single-doc file is exactly that case:
/// `evaluate_yaml_direct_filtered` already returns an empty `doc_results`
/// (nothing to loop over, since it only pushes non-empty result sets), yet
/// `sink` is already halted from evaluating that document internally -- only
/// this second, outer check catches it.
#[test]
fn test_split_exp_halt_with_no_document_output_breaks_via_outer_check() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let (stdout, stderr, code) = run_yq_split("halt", "x: 1\n", &["--split-exp", &pattern])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(
        std::fs::read_dir(dir.path())?.next().is_none(),
        "no split file is ever written when halt fires before doc0 produces anything"
    );
    Ok(())
}

/// The `InputFormat::Json` sibling of
/// `test_split_exp_own_halt_returns_early_and_stops_further_results`: the
/// JSON arm of `--split-exp`'s file loop (a separate `match format` branch
/// from the YAML one) keeps its own copy of the per-result halt check
/// (`if sink.halted().is_some() { break 'files; }` right after
/// `write_split_result`), on its own source line since the two branches
/// don't share code. `.[]` over a JSON array yields three results from one
/// `evaluate_input` call, so `$index == 1`'s halt in the split expression
/// fires mid-loop with a real result (index 2) still pending.
#[test]
fn test_split_exp_own_halt_breaks_json_branch_immediately() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "if $index == 1 then halt else \"{}/f\" + ($index|tostring) + \".yml\" end",
        dir.path().display()
    );
    let (stdout, stderr, code) =
        run_yq_split(".[]", "[1, 2, 3]", &["--split-exp", &pattern, "-p", "json"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f0.yml"))?.trim(),
        "1"
    );
    assert!(!dir.path().join("f1.yml").exists());
    assert!(!dir.path().join("f2.yml").exists());
    Ok(())
}

/// The `InputFormat::Json` sibling of
/// `test_split_exp_halt_with_no_document_output_breaks_via_outer_check`:
/// same "halt before this input's own per-result loop ever runs" shape
/// (unconditional `halt` on the only JSON value in this source, so
/// `results` is empty and the per-result loop's own break check never
/// fires), but through the JSON branch's separate
/// `if sink.halted().is_some() { break 'files; }` after its
/// `for input in inputs` loop -- a distinct source line from the YAML
/// branch's equivalent outer check.
#[test]
fn test_split_exp_json_halt_with_no_output_breaks_via_outer_check() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let (stdout, stderr, code) =
        run_yq_split("halt", "5", &["--split-exp", &pattern, "-p", "json"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
    assert!(std::fs::read_dir(dir.path())?.next().is_none());
    Ok(())
}

/// Null-input sibling of `test_split_exp_own_halt_returns_early_and_stops_further_results`
/// (which drives the equivalent file/YAML branch via `.[]`): with `-n`, the
/// *main* filter (`1, 2, 3`) never halts on its own, so `halted_before_batch`
/// is `false` for the whole batch -- the halt introduced by index 1's own
/// split-filename expression must still be recognized as *new* and break the
/// per-result loop immediately, leaving index 2 unprocessed. Distinguishes
/// this guarded `if !halted_before_batch && sink.halted().is_some() { break;
/// }` from the unconditional form it replaced, which broke here too but for
/// the wrong reason.
#[test]
fn test_split_exp_own_halt_in_null_input_stops_further_results() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "if $index == 1 then halt else \"{}/f\" + ($index|tostring) + \".yml\" end",
        dir.path().display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-n", "--split-exp", &pattern])
        .arg("1, 2, 3")
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());

    assert_eq!(
        std::fs::read_to_string(dir.path().join("f0.yml"))?.trim(),
        "1"
    );
    assert!(
        !dir.path().join("f1.yml").exists(),
        "index 1's own split-exp halt must return early without writing a file"
    );
    assert!(
        !dir.path().join("f2.yml").exists(),
        "index 2 must never be reached once index 1's split expression halted"
    );
    Ok(())
}

/// Code-review follow-up (#791): distinct from
/// `test_split_exp_writes_prefix_produced_before_main_expression_halts`,
/// which uses a *single*-element pre-halt prefix (`1, halt`) and so cannot
/// tell "wrote the whole batch" apart from "broke after the first element".
/// `sink.halted()` is already set by the time `results` comes back from
/// `evaluate_input` (the *main* filter itself halted after producing three
/// legitimate outputs), so the per-result loop's own halt check must not
/// mistake that pre-existing flag for something this iteration caused --
/// every element of the prefix still owes its file, not just the first one.
#[test]
fn test_split_exp_writes_every_result_in_a_multi_value_halt_prefix_null_input() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(["-n", "--split-exp", &pattern])
        .arg("1, 2, 3, halt")
        .stdin(Stdio::null())
        .output()?;
    assert!(output.status.success());

    for (i, expected) in ["1", "2", "3"].into_iter().enumerate() {
        let content = std::fs::read_to_string(dir.path().join(format!("f{i}.yml")))?;
        assert_eq!(content.trim(), expected, "f{i}.yml");
    }
    Ok(())
}

/// YAML-branch sibling of the null-input test above: `range(3), halt`
/// against a single document produces a `doc_results` entry with three
/// legitimate prefix values before the halt, all from one
/// `evaluate_yaml_direct_filtered` call -- so `sink.halted()` is already
/// `Some` before the per-result loop even starts.
#[test]
fn test_split_exp_writes_every_result_in_a_multi_value_halt_prefix_yaml() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let (stdout, stderr, code) =
        run_yq_split("range(3), halt", "x: 1\n", &["--split-exp", &pattern])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");

    for (i, expected) in ["0", "1", "2"].into_iter().enumerate() {
        let content = std::fs::read_to_string(dir.path().join(format!("f{i}.yml")))?;
        assert_eq!(content.trim(), expected, "f{i}.yml");
    }
    Ok(())
}

/// JSON-branch sibling of the two tests above, through `--split-exp`'s
/// separate `InputFormat::Json` per-document loop.
#[test]
fn test_split_exp_writes_every_result_in_a_multi_value_halt_prefix_json() -> Result<()> {
    let dir = TempDir::new()?;
    let pattern = format!(
        "\"{}/f\" + ($index|tostring) + \".yml\"",
        dir.path().display()
    );
    let (stdout, stderr, code) = run_yq_split(
        "range(3), halt",
        "5",
        &["--split-exp", &pattern, "-p", "json"],
    )?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");

    for (i, expected) in ["0", "1", "2"].into_iter().enumerate() {
        let content = std::fs::read_to_string(dir.path().join(format!("f{i}.yml")))?;
        assert_eq!(content.trim(), expected, "f{i}.yml");
    }
    Ok(())
}

/// #791 follow-up: `-R` (raw-input) without `--slurp` evaluates each line as
/// its own string input in a loop over `input_content.lines()`; that loop
/// needed its own `if sink.halted().is_some() { break; }` check after each
/// line's results are written, matching the pattern used by every other
/// per-input loop in this file. Three lines, halting on the second, proves
/// the third is never evaluated at all (not just "produces no output").
#[test]
fn test_raw_input_non_slurp_halt_stops_further_lines() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr(r#"if . == "b" then halt else . end"#, "a\nb\nc\n", &["-R"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout, "a\n",
        "line c must never be evaluated after line b halts"
    );
    Ok(())
}

/// #791 follow-up: the M2 `--inplace` fast path (`can_inplace_fast_path`,
/// reachable for M2-streamable filters like `select(...)`) needed its own
/// halt-stops-streaming check inside its `Sequence` arm, matching the DOM
/// `--inplace` branch's equivalent guard the comment right above this line
/// points at. `select(if .a == 2 then halt else true end)` is M2-streamable
/// (`can_use_m2_streaming` special-cases `Builtin::Select` regardless of its
/// predicate's shape, #796), so a halt inside its predicate reaches this
/// exact M2 inplace loop rather than the DOM one already covered by
/// `test_inplace_halt_before_any_output_in_multi_doc_file_does_not_truncate_file`.
#[test]
fn test_inplace_m2_select_halt_stops_streaming_further_documents() -> Result<()> {
    let mut input_file = NamedTempFile::new()?;
    write!(input_file, "a: 1\n---\na: 2\n---\na: 3\n")?;

    let output = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .arg("-i")
        .arg("select(if .a == 2 then halt else true end)")
        .arg(input_file.path())
        .stdin(Stdio::null())
        .output()?;

    assert!(output.status.success());
    let content = std::fs::read_to_string(input_file.path())?;
    assert_eq!(
        content, "a: 1\n",
        "doc0's already-streamed output survives; doc2 is never reached"
    );
    Ok(())
}

/// Targets the `InputFormat::Json` arm of the default "Standard path"'s
/// input-collection loop (the plain-stdout, non-`--eval-all`/`--split-exp`/
/// `--slurp`/`--inplace`/`--null-input`/`--raw-input` fallback): its inner
/// `for input in inputs` loop's `if sink.halted().is_some() { break; }`
/// (matching the sibling YAML arm's own `break 'collect` right above it,
/// already covered by the plain `run_yq_stdin_with_stderr("halt", ...)`
/// test), and the outer `if sink.halted().is_some() { break 'collect; }`
/// right after it that stops evaluating any further *files*. `., halt`
/// produces one real output before halting, so the second file must never
/// even be read.
#[test]
fn test_default_path_json_halt_stops_further_files() -> Result<()> {
    let mut f1 = NamedTempFile::new()?;
    write!(f1, "1")?;
    let mut f2 = NamedTempFile::new()?;
    write!(f2, "2")?;

    let (stdout, stderr, code) = run_yq_files("., halt", &[f1.path(), f2.path()], &["-p", "json"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout, "1\n",
        "second file must never be evaluated after the first file's halt"
    );
    Ok(())
}

/// `builtin_tz`'s zone-name argument (`Err(e) => return e.into()`) used
/// the same pre-#791 style of error propagation as every other
/// string-argument builtin in this file. `tz` is a real yq keyword (`.
/// | tz("UTC")`), though succinctly's version reads a numeric Unix
/// timestamp off `.` (via `get_float_value`, matching jq's own
/// gmtime/mktime family) rather than mikefarah/yq's already-formatted
/// date string -- confirmed live: `echo 'x: 1' | yq '1700000000 |
/// tz("UTC")'` errors trying to parse `"1700000000"` as a date layout, a
/// pre-existing, unrelated-to-#791 semantic difference. This is therefore
/// checked against succinctly's own halt contract, not real yq's `tz`
/// output shape: `.`'s numeric value is valid enough to reach the zone
/// argument, which is where the halt fires.
#[test]
fn test_yq_tz_propagates_halt_in_zone_argument() -> Result<()> {
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("1700000000 | tz(halt_error(3))", "x: 1\n", &[])?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

/// `builtin_load`'s filename argument (`Err(e) => return e.into()`) had
/// the same pre-#791 gap. `load` is a real yq keyword
/// (`load("file.yaml")`); confirmed live that a missing file is an
/// ordinary error (`echo 'x: 1' | yq 'load("nonexistent.yaml")'` ->
/// "Error: failed to load nonexistent.yaml: ..."), not relevant here since
/// the halt fires while evaluating the filename argument itself, before
/// any file is ever opened.
#[test]
fn test_yq_load_propagates_halt_in_filename_argument() -> Result<()> {
    let (stdout, stderr, code) = run_yq_stdin_with_stderr("load(halt_error(3))", "x: 1\n", &[])?;
    assert_eq!(code, 3, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    Ok(())
}

#[test]
fn test_yq_pick_propagates_halt_in_keys_argument() -> Result<()> {
    // `builtin_pick`'s `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm, reached when the `keys` argument itself
    // halts. `pick`'s ordinary array-of-keys shape matches real yq, verified
    // live: `printf 'a: 1\nb: 2\nc: 3\n' | yq '. | pick(["a","c"])'` prints
    // `a: 1\nc: 3` (real jq's own `pick` instead takes path expressions like
    // `.a,.c` -- a different, pre-existing shape divergence, not this
    // fix). `halt_error` itself has no real-yq contract to check (mikefarah/
    // yq has no `halt_error` at all), so this is checked against
    // succinctly's own documented halt contract (#791), same as the other
    // `halt`/`halt_error` tests in this file's #791 section.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("null | pick(halt_error(9))", "x: 1\n", &[])?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

#[test]
fn test_yq_omit_propagates_halt_in_keys_argument() -> Result<()> {
    // `builtin_omit`'s `QueryResult::Halt(code) => return
    // QueryResult::Halt(code)` arm -- the same shape as `pick`'s above, one
    // builtin over. `omit`'s ordinary array-of-keys shape matches real yq,
    // verified live: `printf 'a: 1\nb: 2\nc: 3\n' | yq '. | omit(["b"])'`
    // prints `a: 1\nc: 3`. `omit` is not a real jq builtin at all (`jq:
    // error: omit/1 is not defined`), and mikefarah/yq has no `halt_error`,
    // so -- as with `pick` above -- this is checked against succinctly's own
    // documented halt contract.
    let (stdout, stderr, code) =
        run_yq_stdin_with_stderr("null | omit(halt_error(9))", "x: 1\n", &[])?;
    assert_eq!(code, 9, "stdout: {stdout:?} stderr: {stderr:?}");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    Ok(())
}

/// `GenericResult::produces_output`'s exhaustive match (#791: this replaced
/// a hand-maintained exclusion list that `d259fba4` had to separately patch
/// for a missed `Halt` case) folds `One`/`OneCursor`/`LazyKeys`/
/// `LazyIndexRange`/`LazySeq`/`Error`/`Owned`/`Partial` into one shared
/// `true` arm. `.a` on a mapping lands in its `OneCursor` case at the M2
/// CLI entry point (`eval_with_cursor_using`'s result, streamed by
/// `stream_cursor!` in `yq_runner.rs`) -- pins that this arm's `true`
/// answer still correctly triggers the `---` separator between two
/// navigation results, the same contract `d259fba4`'s fix depends on for
/// every non-empty variant.
#[test]
fn test_m2_field_access_one_cursor_triggers_doc_separator() -> Result<()> {
    let (out, code) = run_yq_stdin(".a", "a: 1\n---\na: 2\n", &["-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "1\n---\n2");
    Ok(())
}

/// `GenericResult::produces_output`'s `Self::ManyCursor(cs) =>
/// !cs.is_empty()` and `Self::ManyOwned(vs) => !vs.is_empty()` arms (#791),
/// each compiled separately from the surrounding `true`/`false` blocks.
/// `.[("a","b")]`'s computed-key fan-out (#360) yields `ManyCursor` when
/// every key resolves through a live cursor (first document, both `a` and
/// `b` present) and `ManyOwned` when at least one key is missing
/// (`eval_index_expr`'s `any_owned` fallback to `null`; second document has
/// no `b`) -- both non-empty, so both must trigger their own `---`
/// separator, complementing `test_i0_multidoc_separator_skips_empty_results`,
/// which only pins the *false* (empty) side of this same match.
#[test]
fn test_m2_computed_index_many_cursor_and_many_owned_trigger_separator() -> Result<()> {
    let yaml = "a: 1\nb: 2\n---\na: 3\n";
    let (out, code) = run_yq_stdin(r#".[("a","b")]"#, yaml, &["-I=0"])?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "1\n2\n---\n3\nnull");
    Ok(())
}

/// JSON-output sibling of `test_m2_select_halt_does_not_emit_stray_separator`:
/// `GenericResult::stream_json`'s own `Self::Halt(code)` arm (#791) is a
/// *separate* match from `stream_yaml`'s -- `stream_json`/`stream_yaml` are
/// two independent methods -- and is reached only via `-o json` M2 output
/// (`stream_cursor!`'s JSON branch in `yq_runner.rs`), not the default YAML
/// output the sibling test exercises. Confirms a halting document
/// contributes zero JSON output (`control_to_stream_outcome` routes the
/// halt into `stats.halt`, not stdout) while the document before it still
/// streams normally, and the process still exits with the halt's own code.
#[test]
fn test_m2_json_output_select_halt_writes_nothing_for_halted_doc() -> Result<()> {
    let yaml = "a: 1\n---\na: 2\n";
    let (out, code) = run_yq_stdin(
        "select(if .a == 2 then halt else true end)",
        yaml,
        &["-o=json", "-I=0"],
    )?;
    assert_eq!(code, 0);
    assert_eq!(out.trim(), r#"{"a":1}"#);
    Ok(())
}

/// #880/#917: `builtin_in`'s array-index arm has a yq-specific
/// negative-index branch (`S::NEGATIVE_INDEX_IN_HAS`, unreachable via
/// `succinctly jq`'s `JqSemantics`). #917's review corrected the rule this
/// pins: real yq accepts *any* negative index unconditionally, regardless
/// of magnitude vs. the array's own length -- #880's own version of this
/// test asserted the bounded `abs(idx) <= len` rule this repo's docs used
/// to (incorrectly) claim, written without checking the real binary. Real
/// yq itself has no `in(...)` call syntax at all (`builtin_in`'s own doc
/// comment has the real-yq-verified detail), so the values below are
/// checked via `succinctly yq`'s own binary, not real yq directly -- the
/// oracle-verified claim is the *value* semantics (via `has($x)`'s
/// desugared form, confirmed live against the pinned v4.53.3), not this
/// `in(...)` invocation syntax. `-1 | in([1,2,3])` and `-4 | in([1,2,3])`
/// (magnitude 4 > len 3) are both `true`; `2 | in([1,2,3])` is `true`;
/// `5 | in([1,2,3])` is `false` (jq-shared, non-negative-only bound still
/// applies on the positive side).
#[test]
fn test_builtin_in_yq_negative_index_880() -> Result<()> {
    let (out, code) = run_yq_stdin("in([1,2,3])", "-1", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");

    let (out, code) = run_yq_stdin("in([1,2,3])", "-4", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");

    let (out, code) = run_yq_stdin("in([1,2,3])", "2", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");

    let (out, code) = run_yq_stdin("in([1,2,3])", "5", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");
    Ok(())
}

/// Review finding on #908: `idx.abs()` in the negative-index branch above
/// overflowed for `i64::MIN` -- a debug build panicked ("attempt to negate
/// with overflow", exit 101), a release build silently wrapped back to a
/// still-negative `i64::MIN`. #917 replaced the whole bounded-magnitude
/// check with a plain `idx < len` comparison (any negative index is simply
/// always in range in real yq, since `len` is never negative), which
/// sidesteps the overflow concern entirely rather than needing
/// `unsigned_abs()` to guard it -- pinning that `i64::MIN` still doesn't
/// panic and now correctly answers `true` (not `false`), matching every
/// other negative magnitude.
#[test]
fn test_builtin_in_and_has_yq_negative_index_i64_min_no_overflow_908() -> Result<()> {
    let (out, code) = run_yq_stdin("in([1,2,3])", "-9223372036854775808", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");

    let (out, code) = run_yq_stdin("has(-9223372036854775808)", "[1,2,3]", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");
    Ok(())
}

/// #909 review: unlike jq, real yq (mikefarah/yq, confirmed live against
/// the pinned v4.53.3) never accepts a Float key for array indexing at
/// all -- not even a negative one that would truncate to an in-bounds
/// index. `printf 'a: [1,2,3]\n' | yq '.a | has(-1.5)'` and `has(-4.5)`
/// are both `false`, unlike jq's `.[1.5]`-style truncation. An earlier
/// version of this test asserted the opposite (that yq truncates just
/// like jq) before this was checked against the real binary; ports
/// `numeric_key_to_array_index`'s corrected behavior.
#[test]
fn test_builtin_in_yq_negative_float_index_909() -> Result<()> {
    let (out, code) = run_yq_stdin("in([1,2,3])", "-1.5", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin("in([1,2,3])", "-4.5", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");
    Ok(())
}

/// Companion test: yq mode also rejects a *positive* Float key, even one
/// whose truncated value would be an in-bounds index (confirmed live:
/// `has(2.0)` and `has(1.5)` are both `false` on a 3-element array, unlike
/// jq's truncate-then-bounds-check).
///
/// Uses `2.5`/`1.5`, not `2.0`, as the YAML *input* value -- an
/// integer-valued float scalar like `2.0` gets normalized to a genuine
/// `OwnedValue::Int` during YAML parsing itself (confirmed via direct
/// inspection; a separate, pre-existing characteristic of this codebase's
/// YAML-to-OwnedValue conversion, unrelated to this fix -- filed
/// separately). `2.5`/`1.5` have a real fractional part and are preserved
/// as `Float` all the way through, which is what this test needs to
/// exercise the array-key arm's Float-rejection path.
#[test]
fn test_builtin_in_yq_positive_float_index_never_matches_909() -> Result<()> {
    let (out, code) = run_yq_stdin("in([1,2,3])", "2.5", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin("in([1,2,3])", "1.5", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin("has(2.0)", "[1,2,3]", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");
    Ok(())
}

/// #917: real yq (confirmed live against the pinned v4.53.3) never raises a
/// type-mismatch error for `has()` -- a string key on an array, a numeric
/// key on an object, and any key at all on a bare scalar are all `false`,
/// never `Cannot check whether <container> has a <key> key`. jq mode is
/// unaffected by this fix and keeps erroring on the identical inputs --
/// see `test_builtin_in_keeps_earlier_candidates_before_a_later_type_mismatch_error_880`
/// in tests/jq_cli_tests.rs for that (pre-existing) coverage.
#[test]
fn test_builtin_has_yq_type_mismatch_never_errors_917() -> Result<()> {
    let (out, code) = run_yq_stdin(r#"has("x")"#, "[1,2,3]", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin("has(0)", "{a: 1, b: 2}", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin(r#"has("x")"#, "42", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin("has(0)", "42", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");
    Ok(())
}

/// Companion to the above for `in(xs)`, using the same real-yq-verified
/// type-mismatch cases (`in()`'s own definition routes through the same
/// per-candidate match arm as `has()`, #917).
#[test]
fn test_builtin_in_yq_type_mismatch_never_errors_917() -> Result<()> {
    let (out, code) = run_yq_stdin("in([1,2,3])", r#""x""#, &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin("in({a: 1, b: 2})", "0", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");

    let (out, code) = run_yq_stdin("in(42)", r#""x""#, &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "false");
    Ok(())
}

/// Review finding on #917: `builtin_has`'s "yq never errors on a type
/// mismatch" arm originally sat *after* the pre-existing `_ if optional`
/// arm, so it was unreachable whenever `optional` was true -- at the time,
/// `isvalid(EXPR)` forced `optional=true` unconditionally for its inner
/// expression (`builtin_isvalid`), so `isvalid(has("x"))` on `[1,2,3]`
/// returned `false` (as if `has("x")` *would* error without the forced
/// `?`) instead of `true` (yq's `has()` never errors here at all, so the
/// expression is valid). Fixed by moving the permissive arm before the
/// `optional` check -- an ordering fix, not one that depends on `optional`
/// being forced, so it's unaffected by #881 later removing that forcing
/// from `isvalid` entirely. `in()`'s equivalent per-candidate arm never had
/// this bug (it doesn't have a separate `optional`-gated arm in its loop),
/// pinned here too so the two stay symmetric.
#[test]
fn test_isvalid_has_yq_type_mismatch_is_valid_917() -> Result<()> {
    let (out, code) = run_yq_stdin(r#"isvalid(has("x"))"#, "[1,2,3]", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");

    let (out, code) = run_yq_stdin("isvalid(has(0))", "{a: 1}", &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");

    let (out, code) = run_yq_stdin(r"isvalid(in(5))", r#""x""#, &[])?;
    assert_eq!(code, 0, "out: {out:?}");
    assert_eq!(out.trim(), "true");
    Ok(())
}
