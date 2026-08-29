//! Characterization snapshots for jq/yq/dsv CLI query output (#192).
//!
//! These are behavior-preserving guards for the pure-refactor issues: they
//! snapshot the *current* CLI query output over a small representative
//! JSON/YAML/DSV corpus so each refactor becomes a "snapshots unchanged" review:
//!
//! - #181 jq_runner/yq_runner consolidation  -> jq + yq query output
//! - #183 DSV SSE2 path                       -> `--input-dsv` query output
//! - #184 dead JSON SIMD removal              -> jq query output
//! - #185 YAML SIMD scalar-helper dedup       -> yq query output
//!
//! The existing `cli_golden_tests.rs` only snapshots `json generate` + help
//! output; those don't exercise the query path these refactors touch.
//!
//! Cross-backend structural equivalence (scalar vs SSE2/AVX2/BMI2/NEON/SVE2) is
//! covered separately by `dsv_simd_differential_tests.rs`; here we lock the
//! end-to-end CLI output, which is what the refactors must preserve.
//!
//! Run with: cargo test --features cli --test cli_characterization_tests

// #1847 review: `run` spawns `CARGO_BIN_EXE_succinctly` directly rather
// than building it on demand via an inner `cargo run --features cli` the
// way this file's helper used to -- that on-demand build was what let a
// plain `cargo test` (no `--features cli`) still pass. `CARGO_BIN_EXE_
// succinctly` only exists (and only builds the bin target at all) when
// the *outer* invocation already has `--features cli` active, matching
// every other CLI-invoking test file's own guard (`yq_cli_tests.rs`,
// `json_validate_tests.rs`, ...).
#![cfg(feature = "cli")]

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Result;

#[path = "common/cargo_run_exit.rs"]
mod cargo_run_exit;
use cargo_run_exit::exit_code_or_signal_death;

/// Run `succinctly <args>` with `stdin` piped in, returning captured stdout.
///
/// Goes through the real binary directly (`CARGO_BIN_EXE_succinctly`, a
/// compile-time path to this test binary's own already-built dependency)
/// rather than the library, so the snapshot reflects exactly what a user
/// sees. Used to go through a second `cargo run` subprocess instead: that
/// orphans the real `succinctly` grandchild -- reparented to init, blocked
/// forever on its now-unreadable stdout pipe -- the moment anything kills
/// the outer `cargo test` process group (`cargo-guard.sh` does this by
/// design on a detected stall, #935/#1847). No retry loop needed either:
/// a compile-time path isn't a second cargo invocation that can hit lock
/// contention (exit 101), unlike the removed `classify_cargo_run_exit`
/// path this used to need. Bails with stderr on failure so a broken query
/// is a loud test error, not a silently-empty snapshot.
fn run(args: &[&str], stdin: &str) -> Result<String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut sin) = cmd.stdin.take() {
        sin.write_all(stdin.as_bytes())?;
    }

    let output = cmd.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let exit_code = exit_code_or_signal_death(output.status, &output.stderr)?;

    if exit_code != 0 {
        anyhow::bail!("`succinctly {}` failed: {stderr}", args.join(" "));
    }

    Ok(String::from_utf8(output.stdout)?)
}

// ---------------------------------------------------------------------------
// Corpus (small, inline, stable — matches the convention in jq_cli_tests.rs).
// ---------------------------------------------------------------------------

const JSON_USERS: &str =
    r#"{"users":[{"name":"Alice","age":30},{"name":"Bob","age":25}],"active":true}"#;

const JSON_NESTED: &str = r#"{"a":{"b":{"c":[1,2,3]}}}"#;

const YAML_CONFIG: &str = "\
apiVersion: v1
kind: Pod
metadata:
  name: demo
spec:
  containers:
    - name: web
      image: nginx
    - name: sidecar
      image: envoy
";

/// Exercises type preservation: quoted `\"1.0\"` stays a string while bare
/// scalars become bool/int/float/null.
const YAML_TYPES: &str = "\
version: \"1.0\"
enabled: true
count: 42
ratio: 3.14
missing: null
";

const CSV_PEOPLE: &str = "name,age\nAlice,30\nBob,25\n";

// ---------------------------------------------------------------------------
// JSON / jq  (guards #181, #184)
// ---------------------------------------------------------------------------

#[test]
fn jq_users_identity() -> Result<()> {
    insta::assert_snapshot!("jq_users_identity", run(&["jq", "."], JSON_USERS)?);
    Ok(())
}

#[test]
fn jq_users_compact() -> Result<()> {
    insta::assert_snapshot!("jq_users_compact", run(&["jq", "-c", "."], JSON_USERS)?);
    Ok(())
}

#[test]
fn jq_users_names_raw() -> Result<()> {
    insta::assert_snapshot!(
        "jq_users_names_raw",
        run(&["jq", "-r", ".users[].name"], JSON_USERS)?
    );
    Ok(())
}

#[test]
fn jq_users_length() -> Result<()> {
    insta::assert_snapshot!(
        "jq_users_length",
        run(&["jq", ".users | length"], JSON_USERS)?
    );
    Ok(())
}

#[test]
fn jq_users_csv() -> Result<()> {
    insta::assert_snapshot!(
        "jq_users_csv",
        run(&["jq", "-r", ".users[] | [.name, .age] | @csv"], JSON_USERS)?
    );
    Ok(())
}

#[test]
fn jq_nested_path() -> Result<()> {
    insta::assert_snapshot!("jq_nested_path", run(&["jq", "-c", ".a.b.c"], JSON_NESTED)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// YAML / yq  (guards #181, #185)
// ---------------------------------------------------------------------------

#[test]
fn yq_config_identity() -> Result<()> {
    insta::assert_snapshot!("yq_config_identity", run(&["yq", "."], YAML_CONFIG)?);
    Ok(())
}

#[test]
fn yq_config_to_json() -> Result<()> {
    insta::assert_snapshot!(
        "yq_config_to_json",
        run(&["yq", "-o", "json", "."], YAML_CONFIG)?
    );
    Ok(())
}

#[test]
fn yq_config_container_names() -> Result<()> {
    insta::assert_snapshot!(
        "yq_config_container_names",
        run(&["yq", ".spec.containers[].name"], YAML_CONFIG)?
    );
    Ok(())
}

#[test]
fn yq_types_to_json() -> Result<()> {
    insta::assert_snapshot!(
        "yq_types_to_json",
        run(&["yq", "-o", "json", "."], YAML_TYPES)?
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// DSV  (guards #183). `--input-dsv` yields a stream of row-arrays.
// ---------------------------------------------------------------------------

#[test]
fn dsv_rows() -> Result<()> {
    insta::assert_snapshot!(
        "dsv_rows",
        run(&["jq", "--input-dsv", ",", "-c", "."], CSV_PEOPLE)?
    );
    Ok(())
}

#[test]
fn dsv_col0_raw() -> Result<()> {
    insta::assert_snapshot!(
        "dsv_col0_raw",
        run(&["jq", "--input-dsv", ",", "-r", ".[0]"], CSV_PEOPLE)?
    );
    Ok(())
}

#[test]
fn dsv_select_row() -> Result<()> {
    insta::assert_snapshot!(
        "dsv_select_row",
        run(
            &["jq", "--input-dsv", ",", "-c", "select(.[0] == \"Alice\")"],
            CSV_PEOPLE
        )?
    );
    Ok(())
}
