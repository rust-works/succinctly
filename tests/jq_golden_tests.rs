//! Golden tests comparing `succinctly jq` against pinned-`jq` fixtures.
//!
//! Each case under `tests/data/jq-golden/cases/<name>/` holds an input document
//! (`input.json`), a jq filter (`filter`), CLI arguments (`args`, one per line),
//! and the expected stdout (`expected.out`).
//!
//! A case where jq *fails* additionally holds `expected.status` (its exit code)
//! and `expected.err` (its stderr), which are then asserted byte-for-byte. Both
//! are absent for a passing case, which is asserted to exit 0. Without them a
//! failure has no oracle at all: exit code and diagnostic text are exactly what
//! diverged in #355, and neither appears on stdout.
//!
//! A failing case may still carry a non-empty `expected.out`: jq streams the
//! outputs it produced before the error, then exits non-zero (the
//! `*_error_after_output` cases). Stdout is compared in *both* branches, so
//! that pair is what gets pinned — a fix that emits the prefix but loses the
//! failure, or exits correctly but drops the prefix, fails either way. That
//! pairing is the point of the shape for #400 and #494, where the bug is
//! precisely that the outputs preceding the error are discarded.
//!
//! # Golden provenance
//!
//! `expected.out` is captured from jqlang/jq — the oracle — at the version
//! pinned in `tests/data/jq-golden/JQ_VERSION`, via `./scripts/sync-jq-golden.sh`.
//! Never regenerate a golden from succinctly's own output: that would enshrine
//! succinctly's bugs as "correct" and reduce this suite to a regression test
//! with no oracle value. The `jq-drift` CI job re-verifies the goldens against
//! the pinned jq, so the fixtures cannot silently go stale.
//!
//! This is the external-oracle counterpart to the internal-only checks:
//! `tests/cli_golden_tests.rs` is self-snapshot (it locks in succinctly's own
//! past output, preserving divergences rather than catching them) and
//! `tests/jq_evaluator_parity_tests.rs` compares the two evaluators to each
//! other (it cannot catch a bug they share — e.g. #295). Only an external
//! oracle catches that class. The goldens are committed, so these tests need no
//! `jq` binary and run on every CI leg.
//!
//! Succinctly-only extensions (`at_offset`, `at_position`, `@dsv`, `@urid`,
//! `@props`, `@yaml`) have no jq oracle and are deliberately absent here — they
//! stay in the self-snapshot `tests/cli_golden_tests.rs` suite.
//!
//! Every case runs on every invocation. Cases that diverge from jq are listed
//! in `tests/data/jq-golden-known-failures.txt`, and the harness asserts that
//! the set of failures matches that manifest **exactly** — a new divergence
//! fails the build, and so does a manifest entry for a case that now passes.
//!
//! Run with: cargo test --features cli --test jq_golden_tests

#![cfg(feature = "cli")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/jq-golden");
const KNOWN_FAILURES: &str = include_str!("data/jq-golden-known-failures.txt");

struct Case {
    name: String,
    input: String,
    filter: String,
    args: Vec<String>,
    expected: String,
    /// jq's exit code, for cases where it fails. `None` means jq exits 0 and
    /// stderr is not asserted.
    expected_status: Option<i32>,
    /// jq's stderr, asserted byte-for-byte. Present exactly when
    /// `expected_status` is.
    expected_err: Option<String>,
}

/// Load every case directory, failing loudly on an incomplete or empty corpus
/// so a fixture mishap cannot silently shrink coverage.
fn cases() -> Vec<Case> {
    let cases_dir = PathBuf::from(GOLDEN_DIR).join("cases");
    let mut cases: Vec<Case> = fs::read_dir(&cases_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", cases_dir.display()))
        .map(|entry| entry.expect("read case dir entry").path())
        .filter(|p| p.is_dir())
        .map(|dir| {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            let read = |file: &str| {
                fs::read_to_string(dir.join(file))
                    .unwrap_or_else(|e| panic!("case {name} is missing {file}: {e}"))
            };
            let opt_read = |file: &str| fs::read_to_string(dir.join(file)).ok();
            let expected = read("expected.out");
            let expected_status = opt_read("expected.status").map(|s| {
                s.trim()
                    .parse::<i32>()
                    .unwrap_or_else(|e| panic!("case {name} has a bad expected.status: {e}"))
            });
            let expected_err = opt_read("expected.err");
            assert_eq!(
                expected_status.is_some(),
                expected_err.is_some(),
                "case {name}: expected.status and expected.err must be present together \
                 — rerun ./scripts/sync-jq-golden.sh"
            );
            assert!(
                !matches!(expected_status, Some(0)),
                "case {name}: expected.status must record a *failing* exit code; \
                 a passing case omits it — rerun ./scripts/sync-jq-golden.sh"
            );
            // A failing case may legitimately produce no stdout (though it can
            // also stream a prefix first); a passing one producing none means
            // the fixture never got captured.
            assert!(
                !expected.is_empty() || expected_status.is_some(),
                "case {name} has an empty expected.out — rerun ./scripts/sync-jq-golden.sh"
            );
            Case {
                input: read("input.json"),
                filter: read("filter").trim_end_matches('\n').to_string(),
                args: read("args").lines().map(str::to_string).collect(),
                expected,
                expected_status,
                expected_err,
                name,
            }
        })
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        cases.len() >= 20,
        "golden corpus looks truncated ({} cases) — expected at least the seed \
         corpus from #300 (format functions, assignment operators, iterator/collect)",
        cases.len()
    );
    cases
}

/// Parse the known-failures manifest: `<case>  <category>  <reason>`, with `#`
/// comments and blank lines ignored. Returns case -> category.
fn known_failures() -> BTreeMap<String, String> {
    KNOWN_FAILURES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let case = parts.next().unwrap_or_default();
            let category = parts.next().unwrap_or_default();
            assert!(
                !case.is_empty() && !category.is_empty(),
                "malformed manifest line (want `<case>  <category>  <reason>`): {line}"
            );
            (case.to_string(), category.to_string())
        })
        .collect()
}

/// Run `succinctly jq <args> <filter>` with the case input on stdin and demand
/// stdout byte-equal to the golden, plus jq's exit code — 0, or the recorded
/// failing code and stderr for a case that jq fails.
fn run_case(case: &Case) -> Result<(), String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("jq")
        .args(&case.args)
        .arg(&case.filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn succinctly: {e}"))?;
    // #2016: write, but don't propagate a failure yet -- matching
    // `spawn_with_signal_retry`'s own #1891 fix. A `?` here, before
    // `wait_with_output()` below, would drop `child` without reaping it on
    // a write failure (e.g. the child exits before ever reading stdin),
    // leaking a zombie for the rest of this test binary's run -- driven by
    // potentially thousands of fixture cases here, so a regression that
    // hits this path could leak many at once.
    let write_result = child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(case.input.as_bytes())
        .map_err(|e| format!("write stdin: {e}"));
    // Prefer the write error's own diagnostic over `wait_with_output`'s, on
    // the rare double failure where the child is also reaped or killed by
    // something else between the write failing and this wait running
    // (matching `spawn_with_signal_retry`'s own #1891-review priority).
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(wait_err) => {
            return Err(write_result
                .err()
                .unwrap_or_else(|| format!("wait: {wait_err}")))
        }
    };
    write_result?;

    match case.expected_status {
        None => {
            if !output.status.success() {
                return Err(format!(
                    "exit {:?}, stderr: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }
        Some(want) => {
            if output.status.code() != Some(want) {
                return Err(format!(
                    "exit {:?}, jq exits {want}; stderr: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            let want_err = case.expected_err.as_deref().unwrap_or_default();
            if output.stderr != want_err.as_bytes() {
                return Err(format!(
                    "stderr differs from jq\n    expected: {:?}\n    actual:   {:?}",
                    want_err,
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
    }
    if output.stdout != case.expected.as_bytes() {
        return Err(format!(
            "output differs from jq\n    expected: {:?}\n    actual:   {:?}",
            case.expected,
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    Ok(())
}

#[test]
fn jq_golden_conformance() {
    let cases = cases();
    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    for case in &cases {
        if let Err(reason) = run_case(case) {
            failures.insert(case.name.clone(), reason);
        }
    }

    println!(
        "\njq golden conformance: {}/{} cases match pinned jq \
         ({} known failures on record)\n",
        cases.len() - failures.len(),
        cases.len(),
        known_failures().len()
    );

    let expected: BTreeSet<String> = known_failures().keys().cloned().collect();
    let actual: BTreeSet<String> = failures.keys().cloned().collect();

    let unexpected: Vec<_> = actual.difference(&expected).collect();
    let stale: Vec<_> = expected.difference(&actual).collect();

    let mut report = String::new();
    if !unexpected.is_empty() {
        report.push_str(&format!(
            "\n{} case(s) newly FAILING, absent from \
             tests/data/jq-golden-known-failures.txt:\n",
            unexpected.len()
        ));
        for case in &unexpected {
            report.push_str(&format!("  {case}: {}\n", failures[case.as_str()]));
        }
        report.push_str(
            "\nIf this is a known gap, add it to the manifest with a reason and issue link.\n",
        );
    }
    if !stale.is_empty() {
        report.push_str(&format!(
            "\n{} case(s) now PASSING but still listed as known failures:\n",
            stale.len()
        ));
        for case in &stale {
            report.push_str(&format!("  {case}\n"));
        }
        report.push_str("\nNice — remove these lines from the manifest.\n");
    }
    assert!(report.is_empty(), "{report}");
}

/// The manifest is hand-maintained; keep it honest about the corpus it describes.
#[test]
fn known_failures_manifest_is_wellformed() {
    let names: BTreeSet<String> = cases().into_iter().map(|c| c.name).collect();
    let unknown: Vec<_> = known_failures()
        .into_keys()
        .filter(|name| !names.contains(name))
        .collect();
    assert!(
        unknown.is_empty(),
        "manifest lists cases that are not in tests/data/jq-golden/cases: {unknown:?}"
    );
}
