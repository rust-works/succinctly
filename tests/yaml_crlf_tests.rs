//! Line-break differential harness: LF vs CRLF vs lone CR (#324).
//!
//! YAML 1.2 [§5.4] lists `\n`, `\r\n` and `\r` as line breaks, and [§5.7] requires
//! a processor to normalize all of them to `\n` on input. So for any document,
//! rewriting its line breaks from one form to another must not change what the
//! parser produces — that invariance is the property this file tests.
//!
//! [§5.4]: https://yaml.org/spec/1.2.2/#54-line-break-characters
//! [§5.7]: https://yaml.org/spec/1.2.2/#57-escaped-characters
//!
//! # Why a differential rather than fixtures
//!
//! #324 was a *silent* corruption: `a: 1\r\n` parsed as the string `"1 "` instead
//! of the number `1`, because the plain-scalar extent swallowed the `\r`, which
//! the folding decoder then turned into a trailing space. Nothing errored. The
//! whole suite missed it because every fixture and benchmark input in the repo
//! uses LF.
//!
//! Hand-written CRLF fixtures would only ever cover the cases someone thought to
//! write. Driving the entire YAML Test Suite corpus through all three line-break
//! forms and demanding identical output instead *measures* the surface, and keeps
//! measuring it as the parser changes.
//!
//! The property is invariance, not correctness: a case that is wrong under LF is
//! expected to be wrong in exactly the same way under CRLF. Correctness under LF
//! is what `tests/yaml_test_suite.rs` pins, and the two suites compose.
//!
//! # Manifest
//!
//! Cases that are not yet invariant are listed in
//! `tests/data/yaml-crlf-known-failures.txt`, keyed `<case-id>/<variant>`. As in
//! the conformance harness, the assertion is two-sided: a new divergence fails
//! the build, and so does a manifest entry for a case that now passes.
//!
//! Run with: `cargo test --test yaml_crlf_tests -- --nocapture`

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use succinctly::yaml::{locate_offset_detailed, YamlIndex, YamlValue};

const CORPUS: &str = include_str!("data/yaml-test-suite-2022-01-17.json");
const KNOWN_FAILURES: &str = include_str!("data/yaml-crlf-known-failures.txt");

/// The line-break form a document's `\n`s are rewritten to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Breaks {
    /// Windows: `\r\n`.
    Crlf,
    /// Classic Mac: a lone `\r`.
    Cr,
}

impl Breaks {
    fn name(self) -> &'static str {
        match self {
            Self::Crlf => "crlf",
            Self::Cr => "cr",
        }
    }

    /// Rewrite every LF in `yaml` to this form.
    ///
    /// Sound because a raw LF in YAML is *always* a line break — there is no
    /// context in which one is content. (The `\n` inside a double-quoted scalar
    /// is the two characters `\` and `n`, which this leaves alone.)
    fn apply(self, yaml: &str) -> String {
        match self {
            Self::Crlf => yaml.replace('\n', "\r\n"),
            Self::Cr => yaml.replace('\n', "\r"),
        }
    }
}

// ============================================================================
// Parse path — identical to tests/yaml_test_suite.rs, for the same reason:
// this is the path `succinctly yq -o json '.'` actually takes.
// ============================================================================

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

/// The observable outcome of parsing, reduced to what must be invariant.
///
/// Error *messages* carry byte offsets and line numbers that legitimately shift
/// when line breaks change width, so a rejection compares only as "rejected".
/// Accepted documents compare by their full JSON output.
#[derive(PartialEq, Eq)]
enum Outcome {
    Rejected,
    Accepted(Vec<String>),
}

impl Outcome {
    fn of(yaml: &[u8]) -> Self {
        match yaml_to_json_documents(yaml) {
            Ok(docs) => Self::Accepted(docs),
            Err(_) => Self::Rejected,
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Rejected => "<rejected>".to_string(),
            Self::Accepted(docs) => docs.join(" "),
        }
    }
}

// ============================================================================
// Corpus and manifest
// ============================================================================

struct Case {
    id: String,
    name: String,
    yaml: String,
}

/// Every corpus case whose input can be rewritten.
///
/// Cases that already contain a `\r` are skipped: rewriting their `\n`s would
/// produce `\r\r\n` and test a document the corpus never described. Those cases
/// exercise CR handling directly under `tests/yaml_test_suite.rs` instead.
fn corpus() -> Vec<Case> {
    let raw: Vec<Value> = serde_json::from_str(CORPUS).expect("corpus is valid JSON");
    raw.into_iter()
        .map(|c| Case {
            id: c["id"].as_str().expect("case has id").to_string(),
            name: c["name"].as_str().unwrap_or_default().to_string(),
            yaml: c["yaml"].as_str().expect("case has yaml").to_string(),
        })
        .filter(|c| !c.yaml.contains('\r'))
        .collect()
}

/// Parse the known-failures manifest: `<case-id>/<variant>  <category>  <reason>`,
/// with `#` comments and blank lines ignored. Returns key -> category.
fn known_failures() -> BTreeMap<String, String> {
    KNOWN_FAILURES
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            // Columns are padded for readability, so split on whitespace runs.
            let mut parts = line.split_whitespace();
            let key = parts.next().unwrap_or_default();
            let category = parts.next().unwrap_or_default();
            assert!(
                !key.is_empty() && !category.is_empty(),
                "malformed manifest line (want `<case-id>/<variant>  <category>  <reason>`): {line}"
            );
            (key.to_string(), category.to_string())
        })
        .collect()
}

// ============================================================================
// The differential
// ============================================================================

#[test]
fn line_break_form_does_not_change_the_parse() {
    let cases = corpus();
    assert!(
        cases.len() > 300,
        "corpus looks truncated ({} cases) — rerun ./scripts/sync-yaml-test-suite.sh",
        cases.len()
    );

    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    let mut totals: BTreeMap<&str, (usize, usize)> = BTreeMap::new();

    for case in &cases {
        let baseline = Outcome::of(case.yaml.as_bytes());

        for variant in [Breaks::Crlf, Breaks::Cr] {
            let entry = totals.entry(variant.name()).or_insert((0, 0));
            entry.0 += 1;

            let rewritten = variant.apply(&case.yaml);
            let actual = Outcome::of(rewritten.as_bytes());

            if actual == baseline {
                entry.1 += 1;
            } else {
                failures.insert(
                    format!("{}/{}", case.id, variant.name()),
                    format!(
                        "{} — LF gives {}, {} gives {}",
                        case.name,
                        baseline.render(),
                        variant.name().to_uppercase(),
                        actual.render()
                    ),
                );
            }
        }
    }

    println!("\nLine-break invariance ({} cases)\n", cases.len());
    for (variant, (total, pass)) in &totals {
        let pct = if *total == 0 {
            100.0
        } else {
            100.0 * *pass as f64 / *total as f64
        };
        println!("  {variant:<5} matches LF : {pass}/{total} = {pct:.1}%");
    }
    println!("\n  known failures on record: {}\n", known_failures().len());

    let expected: BTreeSet<String> = known_failures().keys().cloned().collect();
    let actual: BTreeSet<String> = failures.keys().cloned().collect();

    let unexpected: Vec<_> = actual.difference(&expected).collect();
    let stale: Vec<_> = expected.difference(&actual).collect();

    let mut report = String::new();
    if !unexpected.is_empty() {
        report.push_str(&format!(
            "\n{} case(s) newly divergent, absent from \
             tests/data/yaml-crlf-known-failures.txt:\n",
            unexpected.len()
        ));
        for key in &unexpected {
            report.push_str(&format!("  {key}: {}\n", failures[key.as_str()]));
        }
        report.push_str(
            "\nIf this is a known gap, add it to the manifest with a reason and issue link.\n",
        );
    }
    if !stale.is_empty() {
        report.push_str(&format!(
            "\n{} case(s) now invariant but still listed as known failures:\n",
            stale.len()
        ));
        for key in &stale {
            report.push_str(&format!("  {key}\n"));
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
        .filter(|key| match key.rsplit_once('/') {
            Some((id, variant)) => !ids.contains(id) || !matches!(variant, "crlf" | "cr"),
            None => true,
        })
        .collect();
    assert!(
        unknown.is_empty(),
        "manifest lists keys that are not `<corpus case id>/<crlf|cr>`: {unknown:?}"
    );
}

/// The opt-in strict validator must reach the same verdict under every
/// line-break form. Without this the validator could drift from the loader —
/// #324 showed the two files handle line breaks in independent code.
#[test]
fn validator_verdict_does_not_change_with_line_breaks() {
    let mut divergent = Vec::new();
    for case in corpus() {
        let baseline = succinctly::yaml::validate::validate(case.yaml.as_bytes()).is_ok();
        for variant in [Breaks::Crlf, Breaks::Cr] {
            let rewritten = variant.apply(&case.yaml);
            let actual = succinctly::yaml::validate::validate(rewritten.as_bytes()).is_ok();
            if actual != baseline {
                let verdict = |ok: bool| if ok { "accepted" } else { "rejected" };
                divergent.push(format!(
                    "  {}/{}: {} — LF {}, {} {}",
                    case.id,
                    variant.name(),
                    case.name,
                    verdict(baseline),
                    variant.name().to_uppercase(),
                    verdict(actual)
                ));
            }
        }
    }
    assert!(
        divergent.is_empty(),
        "the validator changed its verdict on {} case(s) purely from line-break form:\n{}",
        divergent.len(),
        divergent.join("\n")
    );
}

// ============================================================================
// Targeted cases from the issue
// ============================================================================

/// The reproductions in #324, plus the boundaries the issue asked to check.
///
/// Written as byte literals so no git line-ending setting can ever launder the
/// `\r` out of the fixture.
mod issue_324 {
    use super::*;

    #[track_caller]
    fn assert_json(yaml: &[u8], expected: &str) {
        let docs = yaml_to_json_documents(yaml)
            .unwrap_or_else(|e| panic!("parse failed on {:?}: {e}", String::from_utf8_lossy(yaml)));
        assert_eq!(
            docs.join(" "),
            expected,
            "input: {:?}",
            String::from_utf8_lossy(yaml)
        );
    }

    #[test]
    fn mapping_values_keep_their_type() {
        assert_json(b"a: 1\r\nb: 2\r\n", r#"{"a":1,"b":2}"#);
        assert_json(b"a: 1\rb: 2\r", r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn booleans_and_nulls_keep_their_type() {
        assert_json(b"a: true\r\n", r#"{"a":true}"#);
        assert_json(b"a: false\r\n", r#"{"a":false}"#);
        assert_json(b"a: null\r\n", r#"{"a":null}"#);
        assert_json(b"a: ~\r\n", r#"{"a":null}"#);
        assert_json(b"a: 1.5\r\n", r#"{"a":1.5}"#);
        assert_json(b"a: true\r", r#"{"a":true}"#);
    }

    #[test]
    fn sequence_items_have_no_trailing_space() {
        assert_json(b"- a\r\n- b\r\n", r#"["a","b"]"#);
        assert_json(b"- 1\r\n- 2\r\n", "[1,2]");
        assert_json(b"- a\r- b\r", r#"["a","b"]"#);
    }

    #[test]
    fn quoted_scalars_stay_correct() {
        assert_json(b"a: \"x\"\r\n", r#"{"a":"x"}"#);
        assert_json(b"a: 'x'\r\n", r#"{"a":"x"}"#);
        // A quoted number stays a string, as under LF (P10 type preservation).
        assert_json(b"a: \"1.0\"\r\n", r#"{"a":"1.0"}"#);
    }

    #[test]
    fn empty_values_resolve_to_null() {
        assert_json(b"a:\r\nb: 1\r\n", r#"{"a":null,"b":1}"#);
        assert_json(b"a:\rb: 1\r", r#"{"a":null,"b":1}"#);
    }

    #[test]
    fn blank_lines_do_not_become_nodes() {
        assert_json(b"a: 1\r\n\r\nb: 2\r\n", r#"{"a":1,"b":2}"#);
        assert_json(b"a: 1\r\rb: 2\r", r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn comments_terminate_at_the_line_break() {
        assert_json(b"a: 1 # note\r\nb: 2\r\n", r#"{"a":1,"b":2}"#);
        assert_json(b"# lead\r\na: 1\r\n", r#"{"a":1}"#);
        assert_json(b"a: 1 # note\rb: 2\r", r#"{"a":1,"b":2}"#);
    }

    #[test]
    fn document_markers_are_recognised() {
        assert_json(b"---\r\na: 1\r\n", r#"{"a":1}"#);
        assert_json(b"---\r\na: 1\r\n...\r\n", r#"{"a":1}"#);
        assert_json(b"---\r\na: 1\r\n---\r\nb: 2\r\n", r#"{"a":1} {"b":2}"#);
        assert_json(b"---\ra: 1\r", r#"{"a":1}"#);
    }

    #[test]
    fn nested_block_structure_survives() {
        assert_json(
            b"root:\r\n  child:\r\n    - 1\r\n    - two\r\n",
            r#"{"root":{"child":[1,"two"]}}"#,
        );
        assert_json(
            b"root:\r  child:\r    - 1\r    - two\r",
            r#"{"root":{"child":[1,"two"]}}"#,
        );
    }

    #[test]
    fn multiline_plain_scalars_fold_to_a_single_space() {
        assert_json(b"a: one\r\n  two\r\n", r#"{"a":"one two"}"#);
        assert_json(b"a: one\r  two\r", r#"{"a":"one two"}"#);
    }

    #[test]
    fn flow_collections_survive() {
        assert_json(b"a: [1, 2]\r\nb: {c: 3}\r\n", r#"{"a":[1,2],"b":{"c":3}}"#);
        assert_json(b"a: [1, 2]\rb: {c: 3}\r", r#"{"a":[1,2],"b":{"c":3}}"#);
    }

    /// `--- - a` puts a sequence item on the document-marker line, which enters
    /// the parser through `parse_inline_document_value` — a mid-line cursor at
    /// document root — and reaches none of the tests above.
    #[test]
    fn sequence_item_on_the_document_marker_line() {
        assert_json(b"--- - a\n- b\n", r#"["a","b"]"#);
        assert_json(b"--- - a\r\n- b\r\n", r#"["a","b"]"#);
        assert_json(b"--- - a\r- b\r", r#"["a","b"]"#);
    }

    #[test]
    fn literal_block_scalars_drop_the_cr() {
        assert_json(b"a: |\r\n  one\r\n  two\r\n", r#"{"a":"one\ntwo\n"}"#);
        assert_json(b"a: |-\r\n  one\r\n  two\r\n", r#"{"a":"one\ntwo"}"#);
        assert_json(b"a: |+\r\n  one\r\n\r\n", r#"{"a":"one\n\n"}"#);
        assert_json(b"a: |\r  one\r  two\r", r#"{"a":"one\ntwo\n"}"#);
    }

    #[test]
    fn folded_block_scalars_drop_the_cr() {
        assert_json(b"a: >\r\n  one\r\n  two\r\n", r#"{"a":"one two\n"}"#);
        assert_json(b"a: >-\r\n  one\r\n  two\r\n", r#"{"a":"one two"}"#);
        assert_json(b"a: >\r  one\r  two\r", r#"{"a":"one two\n"}"#);
    }

    #[test]
    fn anchors_and_aliases_survive() {
        assert_json(b"a: &x 1\r\nb: *x\r\n", r#"{"a":1,"b":1}"#);
        assert_json(b"a: &x 1\rb: *x\r", r#"{"a":1,"b":1}"#);
    }

    /// A plain scalar long enough to reach the 32-byte SIMD classifier in
    /// `skip_unquoted_simd`. Under a lone CR the classifier must stop at the
    /// break; if it does not, it skips straight past and swallows the rest of
    /// the document into one scalar.
    #[test]
    fn long_scalars_stop_at_the_break() {
        assert_json(
            b"a: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\nb: 2\r\n",
            r#"{"a":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","b":2}"#,
        );
        assert_json(
            b"a: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\rb: 2\r",
            r#"{"a":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","b":2}"#,
        );
        // Same for keys, which use the classifier via `parse_unquoted_key`.
        assert_json(
            b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa: 1\r\nb: 2\r\n",
            r#"{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":1,"b":2}"#,
        );
    }
}

/// Line/column reporting drives `yq-locate` and `at_position`. The newline index
/// already handles all three break forms; assert it, so it stays true.
mod line_column {
    use super::*;

    #[track_caller]
    fn assert_line_column(yaml: &[u8], offset: usize, expected: (usize, usize)) {
        let index = YamlIndex::build(yaml).expect("parses");
        assert_eq!(
            index.to_line_column(offset, yaml),
            expected,
            "offset {offset} in {:?}",
            String::from_utf8_lossy(yaml)
        );
    }

    #[test]
    fn crlf_line_column() {
        // "a: 1\r\nbb: 2\r\n"
        //  0123 4 5 678
        let yaml = b"a: 1\r\nbb: 2\r\n";
        assert_line_column(yaml, 0, (1, 1)); // `a`
        assert_line_column(yaml, 3, (1, 4)); // `1`
        assert_line_column(yaml, 6, (2, 1)); // `b`
        assert_line_column(yaml, 10, (2, 5)); // `2`
    }

    #[test]
    fn lone_cr_line_column() {
        let yaml = b"a: 1\rbb: 2\r";
        assert_line_column(yaml, 0, (1, 1));
        assert_line_column(yaml, 3, (1, 4));
        assert_line_column(yaml, 5, (2, 1));
        assert_line_column(yaml, 9, (2, 5));
    }

    /// Round-trip through `to_offset`, which `yq-locate --line --column` uses.
    #[test]
    fn offset_round_trips_under_every_break_form() {
        for yaml in [
            b"a: 1\nbb: 2\n".to_vec(),
            b"a: 1\r\nbb: 2\r\n".to_vec(),
            b"a: 1\rbb: 2\r".to_vec(),
        ] {
            let index = YamlIndex::build(&yaml).expect("parses");
            for offset in 0..yaml.len() {
                let (line, column) = index.to_line_column(offset, &yaml);
                assert_eq!(
                    index.to_offset(line, column, &yaml),
                    Some(offset),
                    "offset {offset} in {:?}",
                    String::from_utf8_lossy(&yaml)
                );
            }
        }
    }

    /// The byte range `yq-locate` reports comes from `YamlCursor::raw_bytes`,
    /// which re-derives a plain scalar's end from the text rather than reusing
    /// the index. That scan is a second place a `\r` can leak in: the range
    /// would be one byte too long and point at `hello\r` instead of `hello`.
    #[test]
    fn locate_byte_range_excludes_the_line_break() {
        for (yaml, break_form) in [
            (b"a: hello\nb: 2\n".as_slice(), "LF"),
            (b"a: hello\r\nb: 2\r\n".as_slice(), "CRLF"),
            (b"a: hello\rb: 2\r".as_slice(), "CR"),
        ] {
            let index = YamlIndex::build(yaml).expect("parses");
            let found =
                locate_offset_detailed(&index, yaml, 4).expect("offset 4 is inside `hello`");
            let (start, end) = found.byte_range;
            assert_eq!(
                &yaml[start..end],
                b"hello",
                "{break_form}: byte range {:?} covers {:?}",
                found.byte_range,
                String::from_utf8_lossy(&yaml[start..end])
            );
        }
    }

    /// The same scan stops at a comment, so a trailing comment must not extend
    /// the range either — under any break form.
    #[test]
    fn locate_byte_range_stops_before_a_trailing_comment() {
        for yaml in [
            b"a: hi # note\n".as_slice(),
            b"a: hi # note\r\n".as_slice(),
            b"a: hi # note\r".as_slice(),
        ] {
            let index = YamlIndex::build(yaml).expect("parses");
            let found = locate_offset_detailed(&index, yaml, 3).expect("offset 3 is inside `hi`");
            let (start, end) = found.byte_range;
            assert_eq!(
                &yaml[start..end],
                b"hi",
                "byte range {:?} in {:?}",
                found.byte_range,
                String::from_utf8_lossy(yaml)
            );
        }
    }
}
