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
    assert_eq!(output, "-\n  key: a\n  value: 1\n-\n  key: a\n  value: 2\n");
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
    assert_eq!(pretty, "-\n  a: 1\n  a: 2\n");

    let (compact, code) = run_yq_stdin(".", yaml, &["--slurp", "-I0"])?;
    assert_eq!(code, 0);
    assert_eq!(compact, "-\n  a: 1\n  a: 2\n");

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
    assert_eq!(stdout, "-\n  a: 1\n  a: 2\n-\n  b: 3\n");
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
    assert_eq!(output, "-\n  a: 1\n");
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
/// #707 (flow-style preservation) has since landed on the M2 cursor-streaming
/// path only — the DOM fallback path `-P` forces was never touched by it and
/// still renders unconditionally block-style. So for flow-style input, `-P`
/// now diverges from the default (which preserves the input's flow style)
/// and produces real block-style pretty-printing, matching real yq's `-P`
/// semantics.
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
