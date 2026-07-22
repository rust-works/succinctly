//! Integration tests for the `succinctly dsv generate` CLI command.
//!
//! Regression coverage for issue #180: `--header` was inferred as a bare
//! `SetTrue` flag with `default_value = "true"`, making it a no-op with no way
//! to omit the header row. These drive the pre-built binary so the clap
//! declaration (`--header` / `--no-header` override pair) and the `main`
//! dispatch into `dsv_generators` are exercised end-to-end.
//!
//! Run with: cargo test --test dsv_cli_tests

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::Result;

/// Resolve the path to the pre-built `succinctly` CLI binary, building it once.
///
/// Mirrors the helper in `text_cli_tests.rs`: the integration-test harness
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

/// Run `dsv generate` and capture stdout, stderr, and exit code.
fn run_generate(size: &str, extra_args: &[&str]) -> Result<(String, String, i32)> {
    let output = Command::new(succinctly_bin())
        .args(["dsv", "generate", size])
        .args(extra_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((stdout, stderr, exit_code))
}

/// Header row emitted by the default `tabular` pattern (`dsv_generators.rs`).
const TABULAR_HEADER: &str = "id,name,email,age,score,active,created";

#[test]
fn default_includes_header() -> Result<()> {
    let (stdout, stderr, exit_code) = run_generate("200", &["--seed", "42"])?;
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout.lines().next(),
        Some(TABULAR_HEADER),
        "first line should be the tabular header by default"
    );
    Ok(())
}

#[test]
fn no_header_omits_header() -> Result<()> {
    let (stdout, stderr, exit_code) = run_generate("200", &["--seed", "42", "--no-header"])?;
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(
        stdout.lines().all(|line| line != TABULAR_HEADER),
        "no line should be the header with --no-header:\n{stdout}"
    );
    // The generators emit data rows starting at id 1 regardless of the header.
    assert!(
        stdout.starts_with("1,"),
        "first line should be the first data row, got:\n{stdout}"
    );
    Ok(())
}

/// Regression for issue #180: bare `--header` (previously a no-op that was
/// nevertheless accepted) must keep working and keep the header.
#[test]
fn bare_header_flag_still_accepted() -> Result<()> {
    let (stdout, stderr, exit_code) = run_generate("200", &["--seed", "42", "--header"])?;
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert_eq!(stdout.lines().next(), Some(TABULAR_HEADER));
    Ok(())
}

#[test]
fn header_then_no_header_last_wins() -> Result<()> {
    let (stdout, stderr, exit_code) =
        run_generate("200", &["--seed", "42", "--header", "--no-header"])?;
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(
        stdout.lines().all(|line| line != TABULAR_HEADER),
        "--no-header last should omit the header:\n{stdout}"
    );

    let (stdout, stderr, exit_code) =
        run_generate("200", &["--seed", "42", "--no-header", "--header"])?;
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert_eq!(
        stdout.lines().next(),
        Some(TABULAR_HEADER),
        "--header last should restore the header"
    );
    Ok(())
}
