//! Guard tests: deeply-nested but *valid* documents must keep parsing (#192).
//!
//! The DoS fixes #151 (JSON validator), #152 (YAML flow parser) and #153 (YAML
//! alias cycles) add depth caps / cycle guards to stop pathological input from
//! aborting the process. These tests lock in the other side of that fix — that a
//! genuinely-valid, real-world-depth document is **not** over-rejected once the
//! caps land. They are green on `main` today (no cap exists yet) and must stay
//! green after the fixes.
//!
//! `VALID_DEPTH` is deliberately kept well below the ~128 depth cap #151/#152
//! propose, so a correctly-sized cap never trips these guards. If you lower the
//! cap below this, that is a deliberate decision to reject documents this deep —
//! update the constant and say why.
//!
//! Run with: cargo test --features cli --test deep_nesting_valid_tests

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::Result;

const MAX_CARGO_RETRIES: u32 = 3;

/// A real-but-deep nesting depth: deeper than typical documents (~30-50 levels),
/// yet comfortably under the ~128-level DoS cap proposed in #151/#152.
const VALID_DEPTH: usize = 100;

/// Run `succinctly <args>` with `stdin` piped in; return `(stdout, exit_code)`.
fn run(args: &[&str], stdin: &str) -> Result<(String, i32)> {
    for attempt in 0..MAX_CARGO_RETRIES {
        let mut cmd = Command::new("cargo")
            .args(["run", "--features", "cli", "--bin", "succinctly", "--"])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        if let Some(mut sin) = cmd.stdin.take() {
            sin.write_all(stdin.as_bytes())?;
        }

        let output = cmd.wait_with_output()?;
        let exit_code = output.status.code().unwrap_or(-1);

        // Exit code 101 often indicates cargo lock contention; retry.
        if exit_code == 101 && attempt + 1 < MAX_CARGO_RETRIES {
            std::thread::sleep(Duration::from_millis(100 * (attempt as u64 + 1)));
            continue;
        }

        let stdout = String::from_utf8(output.stdout)?;
        return Ok((stdout, exit_code));
    }
    unreachable!()
}

/// `[[[ … ]]]` nested `depth` levels deep with an empty innermost array.
fn nested_json(depth: usize) -> String {
    format!("{}{}", "[".repeat(depth), "]".repeat(depth))
}

/// Guards #151: the JSON validator must accept a valid, deeply-nested document
/// (the DoS fix caps *pathological* depth, not real documents).
#[test]
fn json_validate_accepts_deep_valid_document() -> Result<()> {
    let (_, code) = run(&["json", "validate"], &nested_json(VALID_DEPTH))?;
    assert_eq!(
        code, 0,
        "json validate should accept depth-{VALID_DEPTH} JSON"
    );
    Ok(())
}

/// Guards #152: the YAML flow parser must accept a valid, deeply-nested flow
/// collection (`a: [[[ … ]]]`), the shape whose pathological form aborts today.
#[test]
fn yaml_flow_parser_accepts_deep_valid_document() -> Result<()> {
    let yaml = format!("a: {}", nested_json(VALID_DEPTH));
    let (out, code) = run(&["yq", "-o", "json", ".a"], &yaml)?;
    assert_eq!(code, 0, "yq should accept depth-{VALID_DEPTH} flow YAML");
    assert!(
        out.contains('['),
        "expected nested-array JSON output, got: {out:?}"
    );
    Ok(())
}

/// Guards #153: a valid, *acyclic* anchor/alias must still resolve after the
/// cycle-detection fix (which should reject only self-referential anchors).
#[test]
fn yaml_valid_alias_still_resolves() -> Result<()> {
    let (out, code) = run(&["yq", "-o", "json", ".b"], "a: &x [1, 2]\nb: *x\n")?;
    assert_eq!(code, 0, "yq should resolve a valid alias");
    let compact: String = out.split_whitespace().collect();
    assert_eq!(compact, "[1,2]", "alias .b should resolve to [1, 2]");
    Ok(())
}
