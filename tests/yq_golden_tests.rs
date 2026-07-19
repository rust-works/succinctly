//! Golden tests comparing `succinctly yq` against pinned-`yq` fixtures.
//!
//! Each case under `tests/data/yq-golden/cases/<name>/` holds an input document
//! (`input.yaml`), a jq-style filter (`filter`), CLI arguments (`args`, one per
//! line), and the expected stdout (`expected.out`).
//!
//! # Golden provenance
//!
//! `expected.out` is captured from mikefarah/yq — the oracle — at the version
//! pinned in `tests/data/yq-golden/YQ_VERSION`, via `./scripts/sync-yq-golden.sh`.
//! Never regenerate a golden from succinctly's own output: that would enshrine
//! succinctly's bugs as "correct" and reduce this suite to a regression test
//! with no oracle value. The `yq-drift` CI job re-verifies the goldens against
//! the pinned yq, so the fixtures cannot silently go stale.
//!
//! Unlike the skip-when-yq-is-absent comparisons this suite replaced (#227),
//! these tests need no external binary and run on every CI leg.
//!
//! Every case runs on every invocation. Cases that diverge from yq are listed
//! in `tests/data/yq-golden-known-failures.txt`, and the harness asserts that
//! the set of failures matches that manifest **exactly** — a new divergence
//! fails the build, and so does a manifest entry for a case that now passes.
//!
//! Run with: cargo test --features cli --test yq_golden_tests

#![cfg(feature = "cli")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/yq-golden");
const KNOWN_FAILURES: &str = include_str!("data/yq-golden-known-failures.txt");

struct Case {
    name: String,
    input: String,
    filter: String,
    args: Vec<String>,
    expected: String,
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
            let expected = read("expected.out");
            assert!(
                !expected.is_empty(),
                "case {name} has an empty expected.out — rerun ./scripts/sync-yq-golden.sh"
            );
            Case {
                input: read("input.yaml"),
                filter: read("filter").trim_end_matches('\n').to_string(),
                args: read("args").lines().map(str::to_string).collect(),
                expected,
                name,
            }
        })
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(
        cases.len() >= 8,
        "golden corpus looks truncated ({} cases) — expected at least the 8 \
         migrated from the #227 comparison tests",
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

/// Run `succinctly yq <args> <filter>` with the case input on stdin and demand
/// exit code 0 plus stdout byte-equal to the golden.
fn run_case(case: &Case) -> Result<(), String> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_succinctly"))
        .arg("yq")
        .args(&case.args)
        .arg(&case.filter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn succinctly: {e}"))?;
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(case.input.as_bytes())
        .map_err(|e| format!("write stdin: {e}"))?;
    let output = child.wait_with_output().map_err(|e| format!("wait: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "exit {:?}, stderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    if output.stdout != case.expected.as_bytes() {
        return Err(format!(
            "output differs from yq\n    expected: {:?}\n    actual:   {:?}",
            case.expected,
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    Ok(())
}

#[test]
fn yq_golden_conformance() {
    let cases = cases();
    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    for case in &cases {
        if let Err(reason) = run_case(case) {
            failures.insert(case.name.clone(), reason);
        }
    }

    println!(
        "\nyq golden conformance: {}/{} cases match pinned yq \
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
             tests/data/yq-golden-known-failures.txt:\n",
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
        "manifest lists cases that are not in tests/data/yq-golden/cases: {unknown:?}"
    );
}
