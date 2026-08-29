//! Conformance harness for JSONTestSuite.
//!
//! <https://github.com/nst/JSONTestSuite>
//!
//! The corpus is vendored at a pinned upstream commit in
//! `tests/data/json-test-suite-<short-sha>.json`; regenerate it with
//! `./scripts/sync-json-test-suite.sh`. Vendoring keeps `cargo test` offline and
//! makes the exact conformance input reviewable in-tree.
//!
//! # How this harness is meant to work
//!
//! Upstream encodes the verdict in each filename's prefix, and this harness
//! scores the three classes differently:
//!
//! * `y_*` MUST be accepted and `n_*` MUST be rejected. Cases that do not pass
//!   are listed in `tests/data/json-test-suite-known-failures.txt`, and the test
//!   asserts the failure set matches that manifest **exactly** — a case that
//!   starts failing without a manifest line fails the build, and a line for a
//!   case that now passes also fails the build. Every fix is forced to shrink
//!   the file, and no divergence can hide.
//!
//! * `i_*` is implementation-defined: either verdict conforms to RFC 8259, so
//!   pass/fail is not meaningful. Instead every `i_` case has a **recorded
//!   decision** in `tests/data/json-test-suite-i-decisions.txt`, and the test
//!   asserts actual behaviour equals the record. That turns "undefined" into
//!   "pinned": a change to an implementation-defined behaviour fails the build
//!   naming the case. It doubles as the only written description of which JSON
//!   dialect this crate actually accepts.
//!
//! Run `cargo test --test json_test_suite -- --nocapture` to print the scoreboard.

// #1670: `clippy.toml`'s `disallowed-methods` bans a bare `Vec`/`String`
// `with_capacity` crate-wide (re-enabled only in `succinctly::jq::eval`/
// `eval_generic`) -- every call site here sizes from a single collection's
// own length (or a length-times-constant) and was never part of that bug
// shape.
#![allow(clippy::disallowed_methods)]

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use succinctly::json::validate;

const CORPUS: &str = include_str!("data/json-test-suite-1ef36fa.json");
const KNOWN_FAILURES: &str = include_str!("data/json-test-suite-known-failures.txt");
const I_DECISIONS: &str = include_str!("data/json-test-suite-i-decisions.txt");

/// Decode standard base64 into raw bytes.
///
/// Hand-rolled rather than reusing `jq`'s `@base64d`: that one is private, takes
/// an `OwnedValue`, and yields a `String`, so it cannot represent the corpus
/// cases that are deliberately invalid UTF-8 — which are exactly the cases this
/// harness most needs to feed through unchanged.
fn base64_decode(s: &str) -> Vec<u8> {
    fn sextet(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let input: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(input.len() / 4 * 3);

    for chunk in input.chunks(4) {
        let pad = chunk.iter().filter(|&&b| b == b'=').count();
        let mut acc = 0u32;
        for &b in chunk {
            let v = if b == b'=' {
                0
            } else {
                sextet(b).unwrap_or_else(|| panic!("corpus contains non-base64 byte {b:#04x}"))
            };
            acc = (acc << 6) | v;
        }
        let bytes = acc.to_be_bytes();
        // 4 sextets = 3 bytes; each '=' drops one output byte.
        out.extend_from_slice(&bytes[1..4 - pad]);
    }
    out
}

/// What upstream says about a case.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// `y_` — must be accepted.
    Accept,
    /// `n_` — must be rejected.
    Reject,
    /// `i_` — implementation-defined; scored against our recorded decision.
    Defined,
}

struct Case {
    id: String,
    expect: Expect,
    bytes: Vec<u8>,
}

fn corpus() -> Vec<Case> {
    let raw: Vec<Value> = serde_json::from_str(CORPUS).expect("corpus is valid JSON");
    raw.into_iter()
        .map(|c| Case {
            id: c["id"].as_str().expect("case has id").to_string(),
            expect: match c["expect"].as_str().expect("case has expect") {
                "y" => Expect::Accept,
                "n" => Expect::Reject,
                "i" => Expect::Defined,
                other => panic!("unknown expect {other:?}"),
            },
            bytes: base64_decode(c["bytes_b64"].as_str().expect("case has bytes_b64")),
        })
        .collect()
}

/// Parse a two-column manifest (`<case-id>  <rest...>`), skipping comments.
fn parse_manifest(text: &str, want: &str) -> BTreeMap<String, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let id = parts.next().unwrap_or_default();
            let second = parts.next().unwrap_or_default();
            assert!(
                !id.is_empty() && !second.is_empty(),
                "malformed manifest line (want `{want}`): {line}"
            );
            (id.to_string(), second.to_string())
        })
        .collect()
}

fn known_failures() -> BTreeMap<String, String> {
    parse_manifest(KNOWN_FAILURES, "<case-id>  <category>  <reason>")
}

/// Recorded verdicts for the implementation-defined cases.
fn i_decisions() -> BTreeMap<String, String> {
    let map = parse_manifest(I_DECISIONS, "<case-id>  <accept|reject>  <rationale>");
    for (id, verdict) in &map {
        assert!(
            verdict == "accept" || verdict == "reject",
            "{id}: verdict must be `accept` or `reject`, got {verdict:?}"
        );
    }
    map
}

/// Render a case's bytes for an assertion message.
///
/// Half the corpus is not valid UTF-8, so a lossy-only rendering is unreadable
/// exactly where debugging matters most; the hex dump is the load-bearing half.
fn render(bytes: &[u8]) -> String {
    let hex: Vec<String> = bytes.iter().take(48).map(|b| format!("{b:02x}")).collect();
    format!(
        "{:?}{} [{}{}]",
        String::from_utf8_lossy(&bytes[..bytes.len().min(48)]),
        if bytes.len() > 48 { "..." } else { "" },
        hex.join(" "),
        if bytes.len() > 48 { " ..." } else { "" },
    )
}

#[test]
fn json_test_suite_conformance() {
    let cases = corpus();
    assert!(
        cases.len() > 300,
        "corpus looks truncated ({} cases) — rerun ./scripts/sync-json-test-suite.sh",
        cases.len()
    );

    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    let (mut accept_total, mut accept_pass) = (0usize, 0usize);
    let (mut reject_total, mut reject_pass) = (0usize, 0usize);

    for case in &cases {
        let (total, pass) = match case.expect {
            Expect::Accept => (&mut accept_total, &mut accept_pass),
            Expect::Reject => (&mut reject_total, &mut reject_pass),
            Expect::Defined => continue, // scored by the i_-decisions test
        };
        *total += 1;

        let got = validate::validate(&case.bytes);
        match (case.expect, &got) {
            (Expect::Accept, Ok(())) | (Expect::Reject, Err(_)) => *pass += 1,
            (Expect::Accept, Err(e)) => {
                failures.insert(
                    case.id.clone(),
                    format!("should ACCEPT but rejected: {e} — {}", render(&case.bytes)),
                );
            }
            (Expect::Reject, Ok(())) => {
                failures.insert(
                    case.id.clone(),
                    format!("should REJECT but accepted — {}", render(&case.bytes)),
                );
            }
            (Expect::Defined, _) => unreachable!("filtered above"),
        }
    }

    let pct = |pass: usize, total: usize| {
        if total == 0 {
            100.0
        } else {
            100.0 * pass as f64 / total as f64
        }
    };
    println!("\nJSONTestSuite conformance ({} cases)\n", cases.len());
    println!(
        "  accept (y_, valid JSON)   : {accept_pass}/{accept_total} = {:.1}%",
        pct(accept_pass, accept_total)
    );
    println!(
        "  reject (n_, invalid JSON) : {reject_pass}/{reject_total} = {:.1}%",
        pct(reject_pass, reject_total)
    );
    println!("\n  known failures on record: {}\n", known_failures().len());

    let expected: BTreeSet<String> = known_failures().keys().cloned().collect();
    let actual: BTreeSet<String> = failures.keys().cloned().collect();

    let unexpected: Vec<_> = actual.difference(&expected).collect();
    let stale: Vec<_> = expected.difference(&actual).collect();

    let mut report = String::new();
    if !unexpected.is_empty() {
        report.push_str(&format!(
            "\n{} case(s) newly FAILING, absent from \
             tests/data/json-test-suite-known-failures.txt:\n",
            unexpected.len()
        ));
        for id in &unexpected {
            report.push_str(&format!("  {id}: {}\n", failures[id.as_str()]));
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
        for id in &stale {
            report.push_str(&format!("  {id}\n"));
        }
        report.push_str("\nNice — remove these lines from the manifest.\n");
    }
    assert!(report.is_empty(), "{report}");
}

/// Every `i_` case's verdict must equal the one on record.
///
/// These are the cases RFC 8259 leaves open, so there is no "right" answer to
/// assert — but there is a *current* answer, and silently changing it (which a
/// validator rewrite would very likely do around depth limits, huge numbers and
/// lone surrogates) should never pass unnoticed.
#[test]
fn implementation_defined_verdicts_match_the_record() {
    let decisions = i_decisions();
    let mut drift = String::new();

    for case in corpus().iter().filter(|c| c.expect == Expect::Defined) {
        let recorded = decisions.get(&case.id).unwrap_or_else(|| {
            panic!(
                "{}: no recorded decision — add one to \
                 tests/data/json-test-suite-i-decisions.txt",
                case.id
            )
        });
        let actual = if validate::validate(&case.bytes).is_ok() {
            "accept"
        } else {
            "reject"
        };
        if actual != recorded {
            drift.push_str(&format!(
                "  {}: recorded {recorded}, now {actual} — {}\n",
                case.id,
                render(&case.bytes)
            ));
        }
    }

    assert!(
        drift.is_empty(),
        "\nimplementation-defined behaviour changed:\n{drift}\n\
         If the new behaviour is intended, update \
         tests/data/json-test-suite-i-decisions.txt with the rationale.\n"
    );
}

/// The manifests are hand-maintained; keep them honest about the corpus.
#[test]
fn manifests_are_wellformed() {
    let cases = corpus();
    let all_ids: BTreeSet<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    let defined_ids: BTreeSet<&str> = cases
        .iter()
        .filter(|c| c.expect == Expect::Defined)
        .map(|c| c.id.as_str())
        .collect();

    let unknown: Vec<_> = known_failures()
        .into_keys()
        .filter(|id| !all_ids.contains(id.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "known-failures manifest lists case IDs that are not in the corpus: {unknown:?}"
    );

    // The i_-decisions manifest must cover the implementation-defined cases
    // exactly: a missing line means an unpinned behaviour, an extra line means a
    // record for a case that no longer exists.
    let recorded: BTreeSet<String> = i_decisions().into_keys().collect();
    let recorded_refs: BTreeSet<&str> = recorded.iter().map(String::as_str).collect();

    let missing: Vec<_> = defined_ids.difference(&recorded_refs).collect();
    let extra: Vec<_> = recorded_refs.difference(&defined_ids).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "i_-decisions manifest does not match the corpus's implementation-defined cases\n  \
         missing (behaviour unpinned): {missing:?}\n  \
         extra (no such i_ case): {extra:?}"
    );

    // A known-failure line for an i_ case is a category error: those are scored
    // against the decisions manifest, never as pass/fail.
    let miscategorised: Vec<_> = known_failures()
        .into_keys()
        .filter(|id| defined_ids.contains(id.as_str()))
        .collect();
    assert!(
        miscategorised.is_empty(),
        "known-failures manifest lists implementation-defined cases; \
         record them in json-test-suite-i-decisions.txt instead: {miscategorised:?}"
    );
}
