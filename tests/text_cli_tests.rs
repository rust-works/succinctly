//! Integration tests for the `succinctly text` CLI commands.
//!
//! Covers `text validate utf8` (validation, error reporting, exit codes) and
//! `text generate` / `text generate-suite` (all UTF-8 patterns, verification,
//! file and stdout output). These drive the pre-built binary so the CLI handler
//! modules (`text_validate`, `text_generators`) and their `main` dispatch are
//! exercised end-to-end.
//!
//! Run with: cargo test --features cli --test text_cli_tests

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::Result;
use tempfile::{NamedTempFile, TempDir};

/// Resolve the path to the pre-built `succinctly` CLI binary, building it once.
///
/// Mirrors the helper in `json_validate_tests.rs`: the integration-test harness
/// is compiled without the `cli` feature, and the `succinctly` binary is gated by
/// `required-features = ["cli"]`, so `CARGO_BIN_EXE_succinctly` is unavailable
/// here. We build the binary once with the `cli` feature and derive its path from
/// this test executable's own location. Invoking the built binary directly (not
/// `cargo run`) keeps cargo's output out of each child's captured stderr.
fn succinctly_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let output = Command::new(cargo)
            .args(["build", "--features", "cli", "--bin", "succinctly"])
            .output()
            .expect("failed to spawn `cargo build`");
        assert!(
            output.status.success(),
            "`cargo build --features cli --bin succinctly` failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut path = std::env::current_exe().expect("resolve current_exe");
        path.pop(); // drop the test executable's file name -> `.../deps`
        if path.file_name().and_then(|s| s.to_str()) == Some("deps") {
            path.pop(); // drop `deps` -> `.../<profile>`
        }
        path.push(format!("succinctly{}", std::env::consts::EXE_SUFFIX));
        assert!(
            path.is_file(),
            "built `succinctly` binary not found at {}",
            path.display()
        );
        path
    })
}

/// Run `text validate utf8` with raw bytes piped on stdin.
fn run_validate_stdin(input: &[u8], extra_args: &[&str]) -> Result<(Vec<u8>, String, i32)> {
    let mut cmd = Command::new(succinctly_bin())
        .args(["text", "validate", "utf8"])
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = cmd.stdin.take() {
        stdin.write_all(input)?;
    }

    let output = cmd.wait_with_output()?;
    let exit_code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((output.stdout, stderr, exit_code))
}

/// Run `text validate utf8` against one or more file paths.
fn run_validate_files(paths: &[&str], extra_args: &[&str]) -> Result<(Vec<u8>, String, i32)> {
    let output = Command::new(succinctly_bin())
        .args(["text", "validate", "utf8"])
        .args(extra_args)
        .args(paths)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((output.stdout, stderr, exit_code))
}

/// Run `text generate` and capture raw stdout bytes, stderr, and exit code.
fn run_generate(size: &str, extra_args: &[&str]) -> Result<(Vec<u8>, String, i32)> {
    let output = Command::new(succinctly_bin())
        .args(["text", "generate", size])
        .args(extra_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((output.stdout, stderr, exit_code))
}

/// Every `--pattern` value accepted by `text generate` (clap kebab-cases the
/// `Utf8PatternArg` variants).
const PATTERNS: &[&str] = &[
    "ascii",
    "latin",
    "greek-cyrillic",
    "cjk",
    "emoji",
    "mixed",
    "all-lengths",
    "log-file",
    "source-code",
    "json-like",
    "pathological",
];

// ============================================================================
// `text validate utf8` — exit codes and valid input
// ============================================================================

#[test]
fn validate_valid_ascii_stdin_exit_0() -> Result<()> {
    let (stdout, stderr, code) = run_validate_stdin(b"Hello, world!\n", &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.is_empty(), "stdout should be empty on success");
    Ok(())
}

#[test]
fn validate_valid_multibyte_stdin_exit_0() -> Result<()> {
    let (_, stderr, code) = run_validate_stdin("日本語 café — 🎉".as_bytes(), &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    Ok(())
}

#[test]
fn validate_valid_file_exit_0() -> Result<()> {
    let mut f = NamedTempFile::new()?;
    f.write_all("Grüße, 世界! 🚀".as_bytes())?;
    let (_, stderr, code) = run_validate_files(&[f.path().to_str().unwrap()], &[])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    Ok(())
}

#[test]
fn validate_empty_input_is_valid() -> Result<()> {
    let (_, _, code) = run_validate_stdin(b"", &[])?;
    assert_eq!(code, 0);
    Ok(())
}

// ============================================================================
// `text validate utf8` — invalid input and error kinds
// ============================================================================

#[test]
fn validate_bare_continuation_byte_is_invalid_lead() -> Result<()> {
    // 0x80 cannot start a sequence -> invalid lead byte.
    let (_, stderr, code) = run_validate_stdin(&[0x80], &[])?;
    assert_eq!(code, 1);
    assert!(
        stderr.contains("invalid UTF-8 lead byte"),
        "stderr: {stderr}"
    );
    Ok(())
}

#[test]
fn validate_invalid_continuation_byte() -> Result<()> {
    // 0xE2 begins a 3-byte sequence; 0x28 '(' is not a continuation byte.
    let (_, stderr, code) = run_validate_stdin(&[0xE2, 0x28, 0xA1], &[])?;
    assert_eq!(code, 1);
    assert!(stderr.contains("continuation byte"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn validate_truncated_sequence_at_eof() -> Result<()> {
    // 0xE2 0x82 is the start of a 3-byte sequence, missing its final byte.
    let (_, stderr, code) = run_validate_stdin(&[0xE2, 0x82], &[])?;
    assert_eq!(code, 1);
    assert!(stderr.contains("truncated"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn validate_overlong_encoding() -> Result<()> {
    // 0xC0 0x80 is an overlong (2-byte) encoding of U+0000.
    let (_, stderr, code) = run_validate_stdin(&[0xC0, 0x80], &[])?;
    assert_eq!(code, 1);
    assert!(!stderr.is_empty(), "expected an error message");
    Ok(())
}

#[test]
fn validate_surrogate_codepoint() -> Result<()> {
    // 0xED 0xA0 0x80 encodes U+D800, a reserved UTF-16 surrogate.
    let (_, stderr, code) = run_validate_stdin(&[0xED, 0xA0, 0x80], &[])?;
    assert_eq!(code, 1);
    assert!(stderr.contains("surrogate"), "stderr: {stderr}");
    Ok(())
}

#[test]
fn validate_out_of_range_codepoint() -> Result<()> {
    // 0xF4 0x90 0x80 0x80 would encode U+110000, above the U+10FFFF maximum.
    let (_, stderr, code) = run_validate_stdin(&[0xF4, 0x90, 0x80, 0x80], &[])?;
    assert_eq!(code, 1);
    assert!(!stderr.is_empty(), "expected an error message");
    Ok(())
}

// ============================================================================
// `text validate utf8` — flags and reporting
// ============================================================================

#[test]
fn validate_quiet_mode_suppresses_output() -> Result<()> {
    let (stdout, stderr, code) = run_validate_stdin(&[0x80], &["--quiet"])?;
    assert_eq!(code, 1);
    assert!(stdout.is_empty(), "stdout should be empty");
    assert!(
        stderr.is_empty(),
        "stderr should be empty in quiet mode, got: {stderr}"
    );
    Ok(())
}

#[test]
fn validate_no_color_has_no_ansi() -> Result<()> {
    let (_, stderr, code) = run_validate_stdin(&[0x80], &["--no-color"])?;
    assert_eq!(code, 1);
    assert!(!stderr.is_empty());
    assert!(
        !stderr.contains('\x1b'),
        "no-color output must not contain ANSI escapes: {stderr:?}"
    );
    Ok(())
}

#[test]
fn validate_forced_color_has_ansi() -> Result<()> {
    let (_, stderr, code) = run_validate_stdin(&[0x80], &["--color"])?;
    assert_eq!(code, 1);
    assert!(
        stderr.contains('\x1b'),
        "forced color output should contain ANSI escapes: {stderr:?}"
    );
    Ok(())
}

#[test]
fn validate_file_error_reports_filename_and_position() -> Result<()> {
    let mut f = NamedTempFile::new()?;
    // Valid first line, then an invalid byte on line 2.
    let mut bytes = b"first line ok\nsecond ".to_vec();
    bytes.push(0x80);
    f.write_all(&bytes)?;
    let path = f.path().to_str().unwrap();
    let (_, stderr, code) = run_validate_files(&[path], &["--no-color"])?;
    assert_eq!(code, 1);
    // Location line should mention the file and line 2.
    assert!(
        stderr.contains(path),
        "stderr should name the file: {stderr}"
    );
    assert!(
        stderr.contains(":2:"),
        "stderr should point at line 2: {stderr}"
    );
    // A caret pointer should appear in the snippet.
    assert!(stderr.contains('^'), "expected a caret pointer: {stderr}");
    Ok(())
}

#[test]
fn validate_multiple_files_mixed_validity_exit_1() -> Result<()> {
    let mut good = NamedTempFile::new()?;
    good.write_all("all good ✓".as_bytes())?;
    let mut bad = NamedTempFile::new()?;
    bad.write_all(&[b'x', 0x80, b'y'])?;
    let (_, _, code) = run_validate_files(
        &[good.path().to_str().unwrap(), bad.path().to_str().unwrap()],
        &["--no-color"],
    )?;
    assert_eq!(code, 1, "any invalid file should yield exit 1");
    Ok(())
}

#[test]
fn validate_nonexistent_file_is_io_error() -> Result<()> {
    let (_, stderr, code) =
        run_validate_files(&["/no/such/path/utf8_does_not_exist.txt"], &["--no-color"])?;
    assert_eq!(code, 2, "missing file should yield the IO_ERROR exit code");
    assert!(!stderr.is_empty(), "expected an error message");
    Ok(())
}

#[test]
fn validate_long_line_snippet_is_truncated() -> Result<()> {
    // A line well over the 80-column snippet width, with the error near the end,
    // exercises the truncation branch of get_error_snippet.
    let mut bytes = vec![b'a'; 200];
    bytes.push(0x80);
    let (_, stderr, code) = run_validate_stdin(&bytes, &["--no-color"])?;
    assert_eq!(code, 1);
    assert!(
        stderr.contains("..."),
        "long lines should be elided: {stderr}"
    );
    Ok(())
}

// ============================================================================
// `text generate` — every pattern produces valid UTF-8
// ============================================================================

#[test]
fn generate_all_patterns_to_stdout_are_valid_utf8() -> Result<()> {
    for pattern in PATTERNS {
        let (stdout, stderr, code) =
            run_generate("2kb", &["--pattern", pattern, "--seed", "7", "--verify"])?;
        assert_eq!(code, 0, "pattern {pattern} failed: {stderr}");
        assert!(
            stdout.len() >= 2048,
            "pattern {pattern} produced only {} bytes",
            stdout.len()
        );
        assert!(
            std::str::from_utf8(&stdout).is_ok(),
            "pattern {pattern} produced invalid UTF-8"
        );
        assert!(
            stderr.contains("validated successfully"),
            "pattern {pattern} --verify did not confirm: {stderr}"
        );
    }
    Ok(())
}

#[test]
fn generate_is_deterministic_for_a_fixed_seed() -> Result<()> {
    let (a, _, ca) = run_generate("4kb", &["--pattern", "mixed", "--seed", "123"])?;
    let (b, _, cb) = run_generate("4kb", &["--pattern", "mixed", "--seed", "123"])?;
    assert_eq!(ca, 0);
    assert_eq!(cb, 0);
    assert_eq!(a, b, "same seed should produce identical output");
    Ok(())
}

#[test]
fn generate_unseeded_is_valid_utf8() -> Result<()> {
    // No --seed exercises the deterministic (rng == None) fallback path.
    let (stdout, stderr, code) = run_generate("2kb", &["--pattern", "cjk"])?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(std::str::from_utf8(&stdout).is_ok());
    Ok(())
}

#[test]
fn generate_to_file_writes_valid_utf8() -> Result<()> {
    let dir = TempDir::new()?;
    let out = dir.path().join("emoji.txt");
    let out_str = out.to_str().unwrap();
    let (_, stderr, code) = run_generate(
        "3kb",
        &[
            "--pattern",
            "emoji",
            "--seed",
            "42",
            "-o",
            out_str,
            "--verify",
        ],
    )?;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stderr.contains("Wrote"), "stderr: {stderr}");
    let data = std::fs::read(&out)?;
    assert!(data.len() >= 3072);
    assert!(std::str::from_utf8(&data).is_ok());
    Ok(())
}

// ============================================================================
// `text generate-suite` — writes a directory tree of valid UTF-8 files
// ============================================================================

#[test]
fn generate_suite_writes_all_patterns() -> Result<()> {
    let dir = TempDir::new()?;
    let out_dir = dir.path().join("suite");
    let out_dir_str = out_dir.to_str().unwrap();
    // --max-size 1kb keeps this to one small file per pattern.
    let output = Command::new(succinctly_bin())
        .args(["text", "generate-suite"])
        .args(["--output-dir", out_dir_str])
        .args(["--max-size", "1kb", "--seed", "42", "--clean", "--verify"])
        .output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code().unwrap_or(-1),
        0,
        "generate-suite failed: {stderr}"
    );

    // One subdirectory per pattern, each holding a valid-UTF-8 1kb.txt file.
    let mut found = 0;
    for entry in std::fs::read_dir(&out_dir)? {
        let pattern_dir = entry?.path();
        if pattern_dir.is_dir() {
            let file = pattern_dir.join("1kb.txt");
            if file.is_file() {
                let data = std::fs::read(&file)?;
                assert!(
                    std::str::from_utf8(&data).is_ok(),
                    "{} is not valid UTF-8",
                    file.display()
                );
                found += 1;
            }
        }
    }
    assert_eq!(found, PATTERNS.len(), "expected one file per pattern");
    Ok(())
}
