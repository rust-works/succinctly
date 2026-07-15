//! Conformance harness for the official YAML Test Suite.
//!
//! <https://github.com/yaml/yaml-test-suite>
//!
//! The corpus is vendored at a pinned upstream tag in
//! `tests/data/yaml-test-suite-<tag>.json`; regenerate it with
//! `./scripts/sync-yaml-test-suite.sh`. Vendoring keeps `cargo test` offline and
//! makes the exact conformance input reviewable in-tree.
//!
//! # How this harness is meant to work
//!
//! Every case in the corpus runs on every invocation. Cases that do not pass are
//! listed in `tests/data/yaml-test-suite-known-failures.txt`, and the test asserts
//! that the set of failures matches that manifest **exactly**:
//!
//! * a case that starts failing but is not in the manifest fails the build;
//! * a case that starts passing but is still in the manifest also fails the build.
//!
//! That two-sided check is the point. This file previously held 5040 lines of
//! generated tests covering a hand-picked 253 of the suite's 402 cases, with all 64
//! error cases `#[ignore]`d — so the parser's rejection behavior was entirely
//! unverified, and 54 of the then-failing cases were simply absent. See
//! `docs/compliance/yaml/limitations.md` for current conformance numbers, and
//! `.claude/skills/testing/SKILL.md` ("Critical Anti-Pattern: Success-Only Tests")
//! for why that shape of test is worse than no test at all.
//!
//! Run `cargo test --test yaml_test_suite -- --nocapture` to print the scoreboard.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use succinctly::yaml::{YamlIndex, YamlValue};

const CORPUS: &str = include_str!("data/yaml-test-suite-2022-01-17.json");
const KNOWN_FAILURES: &str = include_str!("data/yaml-test-suite-known-failures.txt");

/// What the suite says a case should do.
enum Expectation<'a> {
    /// Input is invalid; `YamlIndex::build` must reject it.
    MustFail,
    /// Input is valid and JSON-representable; output must equal this JSON.
    Loads(&'a str),
    /// Input is valid but has no JSON representation; it need only parse.
    Parses,
}

/// Convert YAML to one JSON string per document, mirroring the path that
/// `succinctly yq -o json '.'` actually takes (see `yq_runner.rs`): iterate
/// documents via `uncons_cursor()` and stream each one.
///
/// Deliberately does *not* hand-roll a converter. The previous harness did, and so
/// tested a code path no user ever runs.
fn yaml_to_json_documents(yaml: &[u8]) -> Result<Vec<String>, String> {
    let index = YamlIndex::build(yaml).map_err(|e| e.to_string())?;
    let root = index.root(yaml);

    let mut docs = Vec::new();
    match root.value() {
        YamlValue::Sequence(mut elements) => {
            while let Some((cursor, rest)) = elements.uncons_cursor() {
                docs.push(cursor.to_json());
                elements = rest;
            }
        }
        _ => docs.push(root.to_json_document()),
    }
    Ok(docs)
}

/// Rebuild every object with its keys in sorted order.
///
/// `serde_json`'s `preserve_order` feature is off under a plain `cargo test` but on
/// under the coverage job's `--features cli` (Cargo unifies features across the
/// graph). Without this, map comparison would be order-sensitive in one
/// configuration and not the other. Canonicalizing makes it order-insensitive and
/// identical in every configuration.
fn canonicalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<String, Value> =
                map.into_iter().map(|(k, v)| (k, canonicalize(v))).collect();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

/// Parse a stream of concatenated JSON values — the form the suite uses for
/// multi-document cases, where `in.json` holds one value per document, back to back.
fn parse_json_stream(text: &str) -> Result<Vec<Value>, String> {
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .map(|r| r.map(canonicalize).map_err(|e| e.to_string()))
        .collect()
}

fn render(docs: &[Value]) -> String {
    docs.iter()
        .map(|d| serde_json::to_string(d).unwrap_or_else(|_| "<unrenderable>".into()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Run one case. `Ok(())` means it behaved as the suite requires.
fn run_case(yaml: &str, expectation: &Expectation) -> Result<(), String> {
    let produced = yaml_to_json_documents(yaml.as_bytes());

    match expectation {
        Expectation::MustFail => match produced {
            Err(_) => Ok(()),
            Ok(docs) => Err(format!(
                "accepted invalid input, produced {}",
                docs.join(" ")
            )),
        },
        Expectation::Parses => produced
            .map(|_| ())
            .map_err(|e| format!("parse error: {e}")),
        Expectation::Loads(expected_json) => {
            let docs = produced.map_err(|e| format!("parse error: {e}"))?;

            let actual = docs
                .iter()
                .map(|d| {
                    serde_json::from_str(d)
                        .map(canonicalize)
                        .map_err(|e| format!("emitted invalid JSON ({e}): {d}"))
                })
                .collect::<Result<Vec<_>, _>>()?;

            let expected = parse_json_stream(expected_json)
                .map_err(|e| format!("corpus has unparseable in.json ({e})"))?;

            if actual == expected {
                Ok(())
            } else {
                Err(format!(
                    "want {}, got {}",
                    render(&expected),
                    render(&actual)
                ))
            }
        }
    }
}

/// Parse the known-failures manifest: `<case-id>  <category>  <reason>`, with `#`
/// comments and blank lines ignored. Returns id -> category.
fn known_failures() -> BTreeMap<String, String> {
    KNOWN_FAILURES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            // Columns are padded for readability, so split on whitespace runs.
            let mut parts = line.split_whitespace();
            let id = parts.next().unwrap_or_default();
            let category = parts.next().unwrap_or_default();
            assert!(
                !id.is_empty() && !category.is_empty(),
                "malformed manifest line (want `<case-id>  <category>  <reason>`): {line}"
            );
            (id.to_string(), category.to_string())
        })
        .collect()
}

struct Case {
    id: String,
    name: String,
    yaml: String,
    fail: bool,
    json: Option<String>,
}

fn corpus() -> Vec<Case> {
    let raw: Vec<Value> = serde_json::from_str(CORPUS).expect("corpus is valid JSON");
    raw.into_iter()
        .map(|c| Case {
            id: c["id"].as_str().expect("case has id").to_string(),
            name: c["name"].as_str().unwrap_or_default().to_string(),
            yaml: c["yaml"].as_str().expect("case has yaml").to_string(),
            fail: c["fail"].as_bool().unwrap_or(false),
            json: c.get("json").and_then(Value::as_str).map(str::to_string),
        })
        .collect()
}

#[test]
fn yaml_test_suite_conformance() {
    let cases = corpus();
    assert!(
        cases.len() > 300,
        "corpus looks truncated ({} cases) — rerun ./scripts/sync-yaml-test-suite.sh",
        cases.len()
    );

    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    let (mut load_total, mut load_pass) = (0usize, 0usize);
    let (mut fail_total, mut fail_pass) = (0usize, 0usize);
    let (mut parse_total, mut parse_pass) = (0usize, 0usize);

    for case in &cases {
        // `fail` is authoritative: a few must-fail cases also ship an in.json.
        let expectation = if case.fail {
            Expectation::MustFail
        } else if let Some(json) = &case.json {
            Expectation::Loads(json)
        } else {
            Expectation::Parses
        };

        let (total, pass) = match expectation {
            Expectation::MustFail => (&mut fail_total, &mut fail_pass),
            Expectation::Loads(_) => (&mut load_total, &mut load_pass),
            Expectation::Parses => (&mut parse_total, &mut parse_pass),
        };
        *total += 1;

        match run_case(&case.yaml, &expectation) {
            Ok(()) => *pass += 1,
            Err(reason) => {
                failures.insert(case.id.clone(), format!("{} — {reason}", case.name));
            }
        }
    }

    let pct = |pass: usize, total: usize| {
        if total == 0 {
            100.0
        } else {
            100.0 * pass as f64 / total as f64
        }
    };
    println!("\nYAML Test Suite conformance ({} cases)\n", cases.len());
    println!(
        "  load    (valid YAML, output compared) : {load_pass}/{load_total} = {:.1}%",
        pct(load_pass, load_total)
    );
    println!(
        "  reject  (invalid YAML, must fail)     : {fail_pass}/{fail_total} = {:.1}%",
        pct(fail_pass, fail_total)
    );
    println!(
        "  parse   (valid YAML, no JSON form)    : {parse_pass}/{parse_total} = {:.1}%",
        pct(parse_pass, parse_total)
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
             tests/data/yaml-test-suite-known-failures.txt:\n",
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

/// The manifest is hand-maintained; keep it honest about the corpus it describes.
#[test]
fn known_failures_manifest_is_wellformed() {
    let ids: BTreeSet<String> = corpus().into_iter().map(|c| c.id).collect();
    let unknown: Vec<_> = known_failures()
        .into_keys()
        .filter(|id| !ids.contains(id))
        .collect();
    assert!(
        unknown.is_empty(),
        "manifest lists case IDs that are not in the corpus: {unknown:?}"
    );
}
