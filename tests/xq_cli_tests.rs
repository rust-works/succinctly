//! Integration tests for the succinctly `xq`/`xq-locate` CLI commands
//! (issue #667, xq milestone 1).
//!
//! Run with: cargo test --features cli --test xq_cli_tests
//!
//! Mirrors `jq_cli_tests.rs`'s `run_jq_full`/`spawn_jq_full`: spawns the
//! pre-built `succinctly` binary directly via `CARGO_BIN_EXE_succinctly`
//! rather than `cargo run`, so these tests are visible to `cargo llvm-cov`
//! coverage (a `cargo run` subprocess builds a second, uninstrumented
//! binary and silently produces zero coverage signal for a brand-new
//! module — the worst time to have that blind spot).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;
use tempfile::NamedTempFile;

const MAX_SPAWN_RETRIES: u32 = 3;

/// Spawns the pre-built `succinctly` binary with `xq` as the first
/// argument, retrying on a transient `ENOENT` (see `jq_cli_tests.rs`'s
/// `spawn_jq_full` for why this retry exists — #550).
fn spawn_xq(args: &[&str]) -> std::io::Result<std::process::Child> {
    for attempt in 0..MAX_SPAWN_RETRIES {
        match Command::new(env!("CARGO_BIN_EXE_succinctly"))
            .arg("xq")
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

fn run_xq_full(args: &[&str], input: Option<&str>) -> Result<(String, String, i32)> {
    let mut cmd = spawn_xq(args)?;
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

fn run_xq_stdin(filter: &str, input: &str, extra_args: &[&str]) -> Result<(String, i32)> {
    let mut args: Vec<&str> = extra_args.to_vec();
    args.push(filter);
    let (stdout, _, code) = run_xq_full(&args, Some(input))?;
    Ok((stdout, code))
}

const USERS_XML: &str = r#"<?xml version="1.0"?>
<users>
  <user id="1"><name>Alice</name></user>
  <user id="2"><name>Bob</name></user>
</users>
"#;

#[test]
fn identity_navigation() -> Result<()> {
    let (stdout, code) = run_xq_stdin(".user.name", USERS_XML, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"{"+content":"Alice"}"#);
    Ok(())
}

#[test]
fn foo_bar_style_navigation_matches_acceptance_criteria() -> Result<()> {
    let xml = "<root><foo><bar>hello</bar></foo></root>";
    let (stdout, code) = run_xq_stdin(".foo.bar", xml, &["-c"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), r#"{"+content":"hello"}"#);
    Ok(())
}

#[test]
fn attribute_navigation_with_raw_output() -> Result<()> {
    let (stdout, code) = run_xq_stdin(r#".user."+@id""#, USERS_XML, &["-r"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "1");
    Ok(())
}

#[test]
fn attribute_stays_a_string_across_the_cli() -> Result<()> {
    let (stdout, code) = run_xq_stdin(r#".user."+@id" | type"#, USERS_XML, &["-r"])?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "string");
    Ok(())
}

#[test]
fn null_input_and_arithmetic() -> Result<()> {
    let (stdout, _, code) = run_xq_full(&["-n", "1 + 1"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "2");
    Ok(())
}

#[test]
fn arg_binding() -> Result<()> {
    let (stdout, _, code) = run_xq_full(&["-n", "--arg", "greeting", "hello", "$greeting"], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "\"hello\"");
    Ok(())
}

#[test]
fn invalid_xml_reports_error_and_nonzero_exit() -> Result<()> {
    let (_, stderr, code) = run_xq_full(&["."], Some("<root><unclosed>"))?;
    assert_ne!(code, 0);
    assert!(
        stderr.contains("xq:"),
        "stderr should carry the xq: prefix, got: {stderr}"
    );
    Ok(())
}

#[test]
fn file_argument() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    file.write_all(USERS_XML.as_bytes())?;
    let path = file.path().to_str().unwrap();
    let (stdout, _, code) = run_xq_full(&["-c", "-r", ".user.name.\"+content\"", path], None)?;
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "Alice");
    Ok(())
}

#[test]
fn version_flag() -> Result<()> {
    let (stdout, _, code) = run_xq_full(&["--version"], None)?;
    assert_eq!(code, 0);
    assert!(stdout.contains("succinctly-xq"));
    Ok(())
}

fn spawn_xq_locate(args: &[&str]) -> std::io::Result<std::process::Child> {
    Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("xq-locate")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

#[test]
fn xq_locate_expression_format() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    file.write_all(USERS_XML.as_bytes())?;
    let path = file.path().to_str().unwrap();
    let offset = USERS_XML.find("Alice").unwrap();

    let child = spawn_xq_locate(&[path, "--offset", &offset.to_string()])?;
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), ".user.name[\"+content\"]");
    Ok(())
}

#[test]
fn xq_locate_json_format() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    file.write_all(USERS_XML.as_bytes())?;
    let path = file.path().to_str().unwrap();
    let offset = USERS_XML.find("Alice").unwrap();

    let child = spawn_xq_locate(&[path, "--offset", &offset.to_string(), "--format", "json"])?;
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8(output.stdout)?;
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(json["expression"], ".user.name[\"+content\"]");
    assert_eq!(json["type"], "string");
    Ok(())
}

#[test]
fn xq_locate_line_column_matches_offset() -> Result<()> {
    let mut file = NamedTempFile::new()?;
    file.write_all(USERS_XML.as_bytes())?;
    let path = file.path().to_str().unwrap();

    // "Alice" is on line 3 of USERS_XML; column is its 1-indexed byte offset
    // within that line.
    let line3 = USERS_XML.lines().nth(2).unwrap();
    let col = line3.find("Alice").unwrap() + 1;

    let child = spawn_xq_locate(&[path, "--line", "3", "--column", &col.to_string()])?;
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout.trim(), ".user.name[\"+content\"]");
    Ok(())
}
