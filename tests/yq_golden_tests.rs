//! Golden tests comparing `succinctly yq` against pinned-`yq` fixtures.
//!
//! Each case under `tests/data/yq-golden/cases/<name>/` holds an input document
//! (`input.yaml`), a jq-style filter (`filter`), CLI arguments (`args`, one per
//! line), and the expected stdout (`expected.out`).
//!
//! A case where yq exits 0 having printed nothing carries an empty
//! `expected.out` alongside a marker file `expected.empty`, so a silent
//! success is distinguishable from a fixture that was never captured. Real yq
//! does this: `key` and `parent` at the document root emit no output at all
//! (#2421).
//!
//! A case where the oracle *rejects* the input additionally carries
//! `expected.status` (its exit code) and `expected.err` (its stderr), both
//! asserted byte-for-byte; a passing case omits both and is asserted to exit
//! 0. Without this, a case where yq is the one that fails has no oracle at
//! all — the exit code and diagnostic are exactly what would diverge, and
//! neither reaches stdout. `expected.out` may still be non-empty on a failing
//! case: yq can stream output before erroring, and that prefix is compared
//! unconditionally either way.
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
use std::path::PathBuf;
use std::process::Command;

#[path = "common/cargo_run_exit.rs"]
mod cargo_run_exit;
use cargo_run_exit::spawn_with_signal_retry;

const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/yq-golden");
const KNOWN_FAILURES: &str = include_str!("data/yq-golden-known-failures.txt");

struct Case {
    name: String,
    input: String,
    filter: String,
    args: Vec<String>,
    expected: String,
    /// yq's exit code, for a case where it rejects the input. `None` means
    /// yq exits 0 and stderr is not asserted.
    expected_status: Option<i32>,
    /// yq's stderr, asserted byte-for-byte. Present exactly when
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
                 — rerun ./scripts/sync-yq-golden.sh"
            );
            assert!(
                !matches!(expected_status, Some(0)),
                "case {name}: expected.status must record a *failing* exit code; \
                 a passing case omits it — rerun ./scripts/sync-yq-golden.sh"
            );
            // A failing case may legitimately produce no stdout (though it can
            // also stream a prefix first); a passing one producing none is
            // usually a fixture that never got captured — but not always, so
            // the exception has to be *declared*. `expected.empty` is written
            // by ./scripts/sync-yq-golden.sh exactly when yq exits 0 having
            // printed nothing, which real yq genuinely does: `key` and
            // `parent` at the document root emit no output at all (#2421,
            // captured live from v4.53.3). Without the marker the corpus could
            // not hold those cases, and "the mechanism cannot express it"
            // would have quietly become "narrow the case until it fits".
            let expected_empty = dir.join("expected.empty").exists();
            assert!(
                !expected_empty || (expected.is_empty() && expected_status.is_none()),
                "case {name} carries expected.empty but is not a silent success \
                 — rerun ./scripts/sync-yq-golden.sh"
            );
            assert!(
                !expected.is_empty() || expected_status.is_some() || expected_empty,
                "case {name} has an empty expected.out — rerun ./scripts/sync-yq-golden.sh"
            );
            Case {
                input: read("input.yaml"),
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
/// stdout byte-equal to the golden, plus yq's exit code — 0, or the recorded
/// failing code and stderr for a case where yq rejects the input.
fn run_case(case: &Case) -> Result<(), String> {
    run_case_with_input(case, &case.input)
}

/// As [`run_case`], but with the document's line breaks possibly rewritten.
///
/// #2016 (code review): routed through `spawn_with_signal_retry` -- see
/// `jq_golden_tests.rs::run_case`'s own #2016 doc comment for why (this is
/// its yq-mode sibling, iterating the same order of fixture-case volume).
fn run_case_with_input(case: &Case, input: &str) -> Result<(), String> {
    let (output, _code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
            command.arg("yq").args(&case.args).arg(&case.filter);
            command
        },
        Some(input.as_bytes()),
    )
    .map_err(|e| e.to_string())?;

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
                    "exit {:?}, yq exits {want}; stderr: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            let want_err = case.expected_err.as_deref().unwrap_or_default();
            if output.stderr != want_err.as_bytes() {
                return Err(format!(
                    "stderr differs from yq\n    expected: {:?}\n    actual:   {:?}",
                    want_err,
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
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

/// The same goldens, with the input's line breaks rewritten to CRLF and to a
/// lone CR (#324).
///
/// No new fixtures are needed: YAML 1.2 §5.4/§5.7 make a processor normalize all
/// three break forms to `\n` on input, so `yq`'s output is identical for all
/// three spellings of the same document — the existing `expected.out` is the
/// oracle for every variant. Rewriting `\n` is sound because a raw LF in YAML is
/// always a line break; the `\n` inside a double-quoted scalar is the two
/// characters `\` and `n`, which `str::replace` leaves alone.
///
/// The assertion is invariance: each variant must fail on exactly the cases the
/// LF run fails on, so this stays honest without a second manifest.
#[test]
fn yq_golden_conformance_under_crlf_and_lone_cr() {
    let cases = cases();
    let baseline: BTreeSet<String> = cases
        .iter()
        .filter(|c| run_case(c).is_err())
        .map(|c| c.name.clone())
        .collect();

    for (variant, rewrite) in [("CRLF", "\r\n"), ("CR", "\r")] {
        let mut failures: BTreeMap<String, String> = BTreeMap::new();
        for case in &cases {
            let input = case.input.replace('\n', rewrite);
            if let Err(reason) = run_case_with_input(case, &input) {
                failures.insert(case.name.clone(), reason);
            }
        }

        let actual: BTreeSet<String> = failures.keys().cloned().collect();
        let regressed: Vec<_> = actual.difference(&baseline).collect();
        let inverted: Vec<_> = baseline.difference(&actual).collect();

        let mut report = String::new();
        if !regressed.is_empty() {
            report.push_str(&format!(
                "\n{} case(s) match pinned yq under LF but not under {variant}:\n",
                regressed.len()
            ));
            for name in &regressed {
                report.push_str(&format!("  {name}: {}\n", failures[name.as_str()]));
            }
        }
        if !inverted.is_empty() {
            report.push_str(&format!(
                "\n{} case(s) diverge from pinned yq under LF but match under {variant} — \
                 line-break form must not change the output at all:\n",
                inverted.len()
            ));
            for name in &inverted {
                report.push_str(&format!("  {name}\n"));
            }
        }
        assert!(report.is_empty(), "{report}");

        println!(
            "yq golden conformance under {variant}: {}/{} cases, same failure set as LF",
            cases.len() - failures.len(),
            cases.len()
        );
    }
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
