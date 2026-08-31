//! Cross-path consistency: `yq -o=json` must equal `yq -o=json -I=0` in value.
//!
//! `succinctly yq` has two YAML→JSON conversions, both taking the streaming
//! M2 fast path (`YamlCursor::stream_json`) directly from the cursor for the
//! identity query this harness runs — compact (`-I 0`) and pretty differ only
//! in whitespace, not in whether duplicate keys survive (issue #442). Older
//! query shapes that fall back to materializing an `OwnedValue` still collapse
//! duplicate keys via its `IndexMap`, but identity does not. An indent flag
//! must not change data, so for every input the two paths must agree on the
//! JSON *values* they produce (issue #222).
//!
//! This harness runs every case in the vendored YAML Test Suite corpus through
//! both paths and compares. Comparison is on parsed, key-sorted JSON values,
//! not bytes: the paths legitimately differ in whitespace. Objects with
//! duplicate `""` keys (multiple complex keys in one mapping) would collapse
//! last-wins identically on both sides once parsed even if the comparator
//! were byte-exact, since the comparator's own JSON parser dedupes on parse —
//! this harness's parsed-value comparison can't distinguish "both paths kept
//! both keys" from "both paths dropped one," so it isn't a substitute for the
//! byte-level assertions in `tests/yq_cli_tests.rs` and
//! `tests/yq_golden_tests.rs`. If either path errors, they must both error; if
//! either emits output that is not valid JSON, that counts as a divergence.
//!
//! Cases that still diverge — for reasons tracked by *other* issues, not the
//! key/fold/block-scalar drift #222 fixed — are listed in
//! `tests/data/yq-path-consistency-known-failures.txt`, and the set of actual
//! divergences must match that manifest exactly (a new divergence and a
//! newly-agreeing case both fail the build).
//!
//! Run with: cargo test --features cli --test yq_path_consistency_tests

#![cfg(feature = "cli")]

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde_json::Value;

#[path = "common/cargo_run_exit.rs"]
mod cargo_run_exit;
use cargo_run_exit::spawn_with_signal_retry;

const CORPUS: &str = include_str!("data/yaml-test-suite-2022-01-17.json");
const KNOWN_FAILURES: &str = include_str!("data/yq-path-consistency-known-failures.txt");

/// Outcome of running one path (compact or pretty) on an input.
enum PathResult {
    /// Exited 0 and stdout parsed as a stream of JSON values (key-sorted).
    Values(Vec<Value>),
    /// Exited 0 but stdout was not valid JSON.
    Unparseable(String),
    /// Exited non-zero.
    Errored,
}

/// Rebuild every object with keys in sorted order so comparison is
/// order-insensitive regardless of the `preserve_order` feature (mirrors
/// tests/yaml_test_suite.rs).
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

/// Run `succinctly yq <args> '.'` with `yaml` on stdin and classify the result.
/// #2016 (code review): routed through `spawn_with_signal_retry` (was a
/// hand-rolled `spawn()` + `write_all(...).expect(...)` +
/// `wait_with_output()`) -- an `.expect(...)` on the write, before the
/// wait, unwinds past it the same way a `?` early-return would, dropping
/// `child` without reaping it on a write failure and leaking a zombie for
/// the rest of this test binary's run (matching `spawn_with_signal_retry`'s
/// own #1891 fix). Also picks up the ENOENT/signal-death retry that
/// function provides, and its write-error-vs-wait-error diagnostic
/// priority on a double failure, neither of which this hand-rolled version
/// had.
fn run_path(args: &[&str], yaml: &str) -> PathResult {
    let (output, _code) = spawn_with_signal_retry(
        || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_succinctly"));
            command.arg("yq").args(args).arg(".");
            command
        },
        Some(yaml.as_bytes()),
    )
    .expect("spawn succinctly");

    if !output.status.success() {
        return PathResult::Errored;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    match serde_json::Deserializer::from_str(&text)
        .into_iter::<Value>()
        .map(|r| r.map(canonicalize))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(values) => PathResult::Values(values),
        Err(e) => PathResult::Unparseable(e.to_string()),
    }
}

/// Compare the two paths for one input; `Ok(())` if they agree, `Err(reason)`
/// if they diverge.
fn compare(yaml: &str) -> Result<(), String> {
    let compact = run_path(&["-o=json", "-I=0"], yaml);
    let pretty = run_path(&["-o=json"], yaml);
    match (compact, pretty) {
        // Both refuse the input — consistent.
        (PathResult::Errored, PathResult::Errored) => Ok(()),
        (PathResult::Errored, _) => Err("compact errored, pretty did not".into()),
        (_, PathResult::Errored) => Err("pretty errored, compact did not".into()),
        (PathResult::Unparseable(e), _) => Err(format!("compact emitted invalid JSON: {e}")),
        (_, PathResult::Unparseable(e)) => Err(format!("pretty emitted invalid JSON: {e}")),
        (PathResult::Values(c), PathResult::Values(p)) => {
            if c == p {
                Ok(())
            } else {
                let render = |vs: &[Value]| {
                    vs.iter()
                        .map(|v| serde_json::to_string(v).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                Err(format!(
                    "values differ\n    compact: {}\n    pretty:  {}",
                    render(&c),
                    render(&p)
                ))
            }
        }
    }
}

/// Parse the known-divergence manifest: `<case>  <category>  <reason>`.
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

/// The corpus is a JSON array of `{ id, yaml, ... }` objects.
fn corpus() -> Vec<(String, String)> {
    let cases: Vec<Value> = serde_json::from_str(CORPUS).expect("parse corpus");
    cases
        .into_iter()
        .map(|c| {
            let id = c["id"].as_str().expect("case id").to_string();
            let yaml = c["yaml"].as_str().expect("case yaml").to_string();
            (id, yaml)
        })
        .collect()
}

#[test]
fn yq_paths_agree_on_corpus() {
    let cases = corpus();
    assert!(
        cases.len() >= 400,
        "corpus looks truncated ({} cases)",
        cases.len()
    );

    let mut divergences: BTreeMap<String, String> = BTreeMap::new();
    for (id, yaml) in &cases {
        if let Err(reason) = compare(yaml) {
            divergences.insert(id.clone(), reason);
        }
    }

    println!(
        "\nyq path consistency: {}/{} cases agree across -I 0 and pretty \
         ({} known divergences on record)\n",
        cases.len() - divergences.len(),
        cases.len(),
        known_failures().len()
    );

    let expected: BTreeSet<String> = known_failures().keys().cloned().collect();
    let actual: BTreeSet<String> = divergences.keys().cloned().collect();

    let unexpected: Vec<_> = actual.difference(&expected).collect();
    let stale: Vec<_> = expected.difference(&actual).collect();

    let mut report = String::new();
    if !unexpected.is_empty() {
        report.push_str(&format!(
            "\n{} case(s) newly DIVERGING, absent from \
             tests/data/yq-path-consistency-known-failures.txt:\n",
            unexpected.len()
        ));
        for case in &unexpected {
            report.push_str(&format!("  {case}: {}\n", divergences[case.as_str()]));
        }
        report.push_str(
            "\n-I 0 must not change values. Fix the divergence, or (if it is a \
             separately-tracked bug) add it to the manifest with a reason and issue link.\n",
        );
    }
    if !stale.is_empty() {
        report.push_str(&format!(
            "\n{} case(s) now AGREEING but still listed as known divergences:\n",
            stale.len()
        ));
        for case in &stale {
            report.push_str(&format!("  {case}\n"));
        }
        report.push_str("\nNice — remove these lines from the manifest.\n");
    }
    assert!(report.is_empty(), "{report}");
}

/// The manifest is hand-maintained; keep it honest about the corpus.
#[test]
fn manifest_is_wellformed() {
    let ids: BTreeSet<String> = corpus().into_iter().map(|(id, _)| id).collect();
    let unknown: Vec<_> = known_failures()
        .into_keys()
        .filter(|id| !ids.contains(id))
        .collect();
    assert!(
        unknown.is_empty(),
        "manifest lists cases not in the corpus: {unknown:?}"
    );
}

/// Focused regressions for the constructs #222 fixed, so a failure names the
/// construct rather than an opaque suite id. Each input must produce identical
/// JSON from both output paths.
#[test]
fn paths_agree_on_key_and_scalar_constructs() {
    let inputs: &[&str] = &[
        // alias used as a mapping key resolves to the anchored scalar
        "top1:\n  key1: &a scalar1\ntop3:\n  *a : scalar3\n",
        // alias keys both directions
        "&a a: &b b\n*b : *a\n",
        // complex (sequence) key is kept with key ""
        "{a: [b, c], [d, e]: f}\n",
        // explicit empty/null key kept with key ""
        "? []\n: x\n",
        // block scalar is a string, never a number
        "n: |-\n  123\n",
        // empty block scalars are "" not null
        "strip: >-\n\nclip: >\n\nkeep: |+\n\n",
        // content-less keep block scalars gain no line break — #344. `explicit` goes
        // last: a content-less block with an explicit indicator swallows the line after
        // it, so with `sibling` behind it both paths would agree by losing the same key
        // and the case could not fail for the reason it exists.
        "literal: |+\nfolded: >+\nsibling: 1\nexplicit: |2+\n",
        // quoted-string folding: literal trailing whitespace dropped, escaped kept
        "\"folded \nto a space,\t\n \nto a line feed, or \t\\\n \\ \tnon-content\"\n",
        "\"1 trailing\\t\n    tab\"\n",
        // #332: a plain scalar beginning `- ` in flow context is preserved, not
        // dropped — and both emitters must preserve the same thing
        "[- x]\n",
        "[- x, 1]\n",
        "{a: - x}\n",
        "{a: [- x]}\n",
        // and an empty block sequence item stays null on both paths
        "a:\n  -\n  - y\n",
    ];
    for yaml in inputs {
        compare(yaml).unwrap_or_else(|e| panic!("paths diverge on {yaml:?}:\n{e}"));
    }
}
