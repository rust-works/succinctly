//! Error-message parity with jq (#356).
//!
//! Since #158, `catch` binds the raised value, so an evaluator-internal error's
//! *message text* is readable from a filter — `try f catch (if test("Cannot
//! index") then … else … end)` is a real jq idiom. That makes the wording part
//! of the observable surface, not just stderr decoration.
//!
//! Every expectation here is captured from jqlang/jq at the version pinned in
//! `tests/data/jq-golden/JQ_VERSION`, by `./scripts/sync-jq-error-messages.sh`
//! reading `tests/data/jq-error-probes.tsv`. Nothing in this file is written by
//! hand from succinctly's own output, so the suite cannot lock in wording that
//! is merely self-consistent.
//!
//! Each probe runs through BOTH evaluators — the full one (`src/jq/eval.rs`)
//! and the generic one (`src/jq/eval_generic.rs`, which the CLI uses) — because
//! the two have historically drifted from each other as well as from jq, and
//! nothing else in the suite compares their error text.
//!
//! `tests/data/jq-error-known-divergences.txt` records where succinctly still
//! disagrees. The check is two-sided: a probe failing without a line there
//! fails the build, and a line for a probe that now passes fails the build too.

use std::collections::{BTreeMap, BTreeSet};
use succinctly::jq::eval_generic;
use succinctly::jq::{eval, parse, JqSemantics, QueryResult};
use succinctly::json::JsonIndex;

const TABLE: &str = include_str!("data/jq-error-messages.tsv");
const KNOWN_DIVERGENCES: &str = include_str!("data/jq-error-known-divergences.txt");

/// Probes whose jq message depends on an optional feature being compiled in.
/// Without `regex`, `match`/`capture`/`scan`/`splits`/`sub`/`gsub` fail with
/// "regex feature not enabled" before they can reach the type check the probe
/// is about. `test`'s bare/pattern-argument form has a non-regex substring-
/// match fallback that reproduces jq's wording on both build configs, so
/// `test_arg_non_string` needs no entry here — but that fallback doesn't
/// support a flags argument at all, so `test_flags_non_string` still does.
#[cfg(feature = "regex")]
const FEATURE_GATED: &[&str] = &[];
#[cfg(not(feature = "regex"))]
const FEATURE_GATED: &[&str] = &[
    "match_on_number",
    "capture_on_number",
    "scan_on_number",
    "splits_on_number",
    "sub_on_number",
    "gsub_on_number",
    "match_arg_non_string",
    "capture_arg_non_string",
    "scan_arg_non_string",
    "gsub_arg_non_string",
    "sub_arg_non_string",
    "splits_arg_non_string",
    "split_regex_arg_non_string",
    "test_flags_non_string",
    "match_flags_non_string",
    "capture_flags_non_string",
    "sub_flags_non_string",
    "gsub_flags_non_string",
    "scan_flags_non_string",
    "split_flags_non_string",
    "splits_flags_non_string",
];

struct Probe {
    id: String,
    filter: String,
    input: String,
    expected: String,
}

fn probes() -> Vec<Probe> {
    let probes: Vec<Probe> = TABLE
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut cols = line.split('\t');
            let mut next = |what: &str| {
                cols.next()
                    .unwrap_or_else(|| panic!("malformed row (missing {what}): {line}"))
                    .to_string()
            };
            Probe {
                id: next("id"),
                filter: next("filter"),
                input: next("input"),
                expected: next("message"),
            }
        })
        .collect();

    assert!(
        probes.len() >= 90,
        "probe table looks truncated ({} rows) — rerun \
         ./scripts/sync-jq-error-messages.sh",
        probes.len()
    );
    probes
}

/// What an evaluator did with a probe: the error message it raised, or a
/// description of why there was no message to compare.
fn outcome(evaluator: Evaluator, probe: &Probe) -> Result<String, String> {
    let expr = parse(&probe.filter).map_err(|e| format!("filter failed to parse: {e:?}"))?;
    let json = probe.input.as_bytes();
    let index = JsonIndex::build(json);
    let cursor = index.root(json);

    let (error, outputs) = match evaluator {
        Evaluator::Full => {
            let result: QueryResult<Vec<u64>> = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
            match result {
                QueryResult::Error(e) => (Some(e.message), Vec::new()),
                other => (None, render(other.collect_owned())),
            }
        }
        Evaluator::Generic => match eval_generic::eval_with_cursor(&expr, cursor) {
            eval_generic::GenericResult::Error(e) => (Some(e.message), Vec::new()),
            other => (None, render(other.collect_owned())),
        },
    };

    match error {
        Some(message) => Ok(message),
        None => Err(format!("no error raised; produced {outputs:?}")),
    }
}

fn render(values: Vec<succinctly::jq::OwnedValue>) -> Vec<String> {
    values
        .iter()
        .map(succinctly::jq::OwnedValue::to_json)
        .collect()
}

#[derive(Clone, Copy)]
enum Evaluator {
    Full,
    Generic,
}

impl Evaluator {
    fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Generic => "generic",
        }
    }
}

/// Parse the manifest: `<probe>  <category>  <reason>`, `#` and blanks ignored.
fn known_divergences() -> BTreeMap<String, String> {
    KNOWN_DIVERGENCES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let probe = parts.next().unwrap_or_default();
            let category = parts.next().unwrap_or_default();
            assert!(
                !probe.is_empty() && !category.is_empty(),
                "malformed manifest line (want `<probe>  <category>  <reason>`): {line}"
            );
            (probe.to_string(), category.to_string())
        })
        .collect()
}

#[test]
fn jq_error_message_parity() {
    let probes = probes();
    let mut failures: BTreeMap<String, String> = BTreeMap::new();

    for probe in &probes {
        if FEATURE_GATED.contains(&probe.id.as_str()) {
            continue;
        }
        let mut reasons = Vec::new();
        for evaluator in [Evaluator::Full, Evaluator::Generic] {
            match outcome(evaluator, probe) {
                Ok(message) if message == probe.expected => {}
                Ok(message) => reasons.push(format!("{}: {message:?}", evaluator.name())),
                Err(why) => reasons.push(format!("{}: {why}", evaluator.name())),
            }
        }
        if !reasons.is_empty() {
            failures.insert(
                probe.id.clone(),
                format!(
                    "jq: {:?}\n      {}",
                    probe.expected,
                    reasons.join("\n      ")
                ),
            );
        }
    }

    let skipped = probes
        .iter()
        .filter(|p| FEATURE_GATED.contains(&p.id.as_str()))
        .count();
    println!(
        "\njq error-message parity: {}/{} probes match pinned jq in both evaluators \
         ({} known divergences on record, {skipped} feature-gated)\n",
        probes.len() - failures.len() - skipped,
        probes.len() - skipped,
        known_divergences().len(),
    );

    let expected: BTreeSet<String> = known_divergences().keys().cloned().collect();
    let actual: BTreeSet<String> = failures.keys().cloned().collect();

    let unexpected: Vec<_> = actual.difference(&expected).collect();
    let stale: Vec<_> = expected.difference(&actual).collect();

    let mut report = String::new();
    if !unexpected.is_empty() {
        report.push_str(&format!(
            "\n{} probe(s) newly DIVERGING, absent from \
             tests/data/jq-error-known-divergences.txt:\n",
            unexpected.len()
        ));
        for probe in &unexpected {
            report.push_str(&format!("  {probe}\n      {}\n", failures[probe.as_str()]));
        }
        report.push_str(
            "\nIf this is a deliberate gap, add it to the manifest with a reason and \
             issue link, and a section in docs/compliance/jq/limitations.md.\n",
        );
    }
    if !stale.is_empty() {
        report.push_str(&format!(
            "\n{} probe(s) now MATCHING but still listed as known divergences:\n",
            stale.len()
        ));
        for probe in &stale {
            report.push_str(&format!("  {probe}\n"));
        }
        report.push_str("\nNice — remove these lines from the manifest.\n");
    }
    assert!(report.is_empty(), "{report}");
}

/// The compliance page quotes the corpus size, the pass count and the number
/// of gaps in prose, and nothing else checks them — they went stale twice
/// while #356 was being written, once in the same commit that changed the
/// corpus. Parse them back out and compare against the corpus itself.
///
/// Only the summary table carries numbers, so this is the single place to
/// update when the corpus grows; elsewhere the page and its neighbours are
/// worded without counts on purpose.
#[test]
fn limitations_page_quotes_the_real_numbers() {
    const PAGE: &str = include_str!("../docs/compliance/jq/limitations.md");

    let total = probes().len();
    let diverging = known_divergences().len();
    let matching = total - diverging;
    // One decimal place, rounded like `{:.1}` — the page shows e.g. `94.0%`.
    let percent = format!("{:.1}", matching as f64 * 100.0 / total as f64);

    for quote in [
        format!("over the {total} probes"),
        format!("**{matching}/{total} = {percent}%**"),
        format!("| **{diverging}**"),
    ] {
        assert!(
            PAGE.contains(&quote),
            "docs/compliance/jq/limitations.md is stale: expected it to say {quote:?} \
             ({matching} of {total} probes match, {diverging} on the divergence manifest)"
        );
    }
}

/// The manifest is hand-maintained; keep it honest about the corpus it names.
#[test]
fn known_divergences_reference_real_probes() {
    let ids: BTreeSet<String> = probes().into_iter().map(|p| p.id).collect();
    let unknown: Vec<String> = known_divergences()
        .keys()
        .filter(|probe| !ids.contains(*probe))
        .cloned()
        .collect();
    assert!(
        unknown.is_empty(),
        "tests/data/jq-error-known-divergences.txt names probes that are not in \
         tests/data/jq-error-probes.tsv: {unknown:?}"
    );
}
