//! Tabs in YAML indentation: the loader and the validator must agree (#173).
//!
//! YAML forbids a tab in indentation, but a tab is only *indentation* when block
//! structure follows it. Before a plain scalar the same byte is separation and
//! perfectly legal — `foo:\n \tbar` (DK95/00) and `x:\n - x\n  \tx` (UV7Q, "Legal
//! tab after indentation") are both valid documents. That distinction is what
//! the crate-private `yaml::line_is_structural` encodes (`src/yaml/mod.rs`); from
//! out here it is reachable only through its two call sites, which is what this
//! file exercises.
//!
//! Before #173 only the strict validator applied the rule; the default loader
//! rejected a tab at column 0 and treated a tab after one or more spaces as
//! start-of-content, so `a:\n \tb: 1` loaded as `{"a":{"\tb":1}}` — the tab folded
//! into the key. Both now consult one definition.
//!
//! # Why an agreement *table* and not an invariant
//!
//! The obvious assertion — "the loader rejects a tab shape only if the validator
//! does" — is false, in both directions, for reasons that have nothing to do with
//! tabs (see the two `Verdict` rows that disagree). Writing it as an invariant would
//! mean either a red build or quietly weakening it until it asserted nothing.
//! Stating both verdicts per row instead makes each divergence a reviewed fact with
//! a reason attached, which is what CLAUDE.md's #106 lesson asks for: one definition
//! of the predicate, plus a test that the call sites agree.
//!
//! Run with: `cargo test --test yaml_tab_indentation_tests`

use succinctly::yaml::validate::YamlValidationErrorKind;
use succinctly::yaml::{YamlError, YamlIndex, YamlValue};

const CORPUS: &str = include_str!("data/yaml-test-suite-2022-01-17.json");

/// Render as `succinctly yq -o json '.'` does, one JSON value per document.
/// Mirrors `yaml_to_json_documents` in `tests/yaml_test_suite.rs`.
fn to_json(yaml: &[u8]) -> Result<String, YamlError> {
    let index = YamlIndex::build(yaml)?;
    let root = index.root(yaml);
    Ok(match root.value() {
        YamlValue::Sequence(mut elements) => {
            let mut docs = Vec::new();
            while let Some((cursor, rest)) = elements.uncons_cursor() {
                docs.push(cursor.to_json());
                elements = rest;
            }
            docs.join(" ")
        }
        _ => root.to_json_document(),
    })
}

fn loader_reports_tab_indentation(yaml: &[u8]) -> bool {
    matches!(
        YamlIndex::build(yaml),
        Err(YamlError::TabIndentation { .. })
    )
}

/// What the opt-in strict validator makes of a document.
///
/// Deliberately finer than `is_err()`: a row that rejects for an unrelated reason
/// would otherwise read as agreement with the loader's tab verdict, and would keep
/// passing if the tab rule stopped firing entirely.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Validator {
    Accepts,
    /// Rejected as `TabInIndentation` — the same judgement the loader makes.
    RejectsTheTab,
    /// Rejected, but for something else; the tab verdict is not what is under test.
    RejectsOtherwise,
}

fn validator_verdict(yaml: &[u8]) -> Validator {
    match succinctly::yaml::validate::validate(yaml) {
        Ok(()) => Validator::Accepts,
        Err(e) if e.kind == YamlValidationErrorKind::TabInIndentation => Validator::RejectsTheTab,
        Err(_) => Validator::RejectsOtherwise,
    }
}

/// One row of the agreement table.
struct Verdict {
    yaml: &'static [u8],
    /// Does `YamlIndex::build` reject it as `TabIndentation`?
    loader: bool,
    /// What the opt-in strict validator makes of it.
    validator: Validator,
    /// Why — and, where the two disagree, why that is not a tab bug.
    why: &'static str,
}

const VERDICTS: &[Verdict] = &[
    // ---- A tab that is indentation. Both must reject. --------------------------
    Verdict {
        yaml: b"a:\n \tb: 1\n",
        loader: true,
        validator: Validator::RejectsTheTab,
        why: "#173 repro: loaded as {\"a\":{\"\\tb\":1}} before the fix",
    },
    Verdict {
        yaml: b"foo:\n  a: 1\n  \tb: 2\n",
        loader: true,
        validator: Validator::RejectsTheTab,
        why: "DK95/06, the suite's must-fail case for this shape",
    },
    Verdict {
        yaml: b"  \t- a\n",
        loader: true,
        validator: Validator::RejectsTheTab,
        why: "a tab before a sequence entry is indentation too",
    },
    // ---- A tab that is separation. Both must accept. ---------------------------
    Verdict {
        yaml: b"foo:\n \tbar\n",
        loader: false,
        validator: Validator::Accepts,
        why: "DK95/00: a tab before a plain scalar is separation, not indentation",
    },
    Verdict {
        yaml: b"x:\n - x\n  \tx\n",
        loader: false,
        validator: Validator::Accepts,
        why: "UV7Q, named upstream 'Legal tab after indentation'",
    },
    Verdict {
        yaml: b"\t{}\n",
        loader: false,
        validator: Validator::Accepts,
        why: "Q5MG: a root flow node's leading separation may contain tabs",
    },
    Verdict {
        yaml: b"\t[\n\t]\n",
        loader: false,
        validator: Validator::Accepts,
        why: "6CA3, the sequence form of Q5MG",
    },
    Verdict {
        yaml: b"a: |\n  x\n  \tb: c\n",
        loader: false,
        validator: Validator::Accepts,
        why: "inside a block scalar the tab is content; the body never reaches \
              either line dispatcher",
    },
    Verdict {
        yaml: b"a:\n \t\"x: y\"\n",
        loader: false,
        validator: Validator::Accepts,
        why: "the `:` is inside a quoted scalar, so the line is a node and the tab \
              is separation — same production as DK95/00",
    },
    Verdict {
        yaml: b"a:\n \t'x: y'\n",
        loader: false,
        validator: Validator::Accepts,
        why: "the single-quoted form of the row above",
    },
    Verdict {
        yaml: b"a:\n \t\"b\": 1\n",
        loader: true,
        validator: Validator::RejectsTheTab,
        why: "a quoted *key* is still a mapping entry, so the tab is indentation — \
              the pair to the two rows above, and why the scan skips the quoted \
              span rather than bailing out at the quote",
    },
    Verdict {
        yaml: b"a: 1\n \t# c: d\nb: 2\n",
        loader: false,
        validator: Validator::Accepts,
        why: "a tab before a comment is separation; the `: ` in the comment text is \
              not a value indicator",
    },
    // ---- Rows where the two legitimately disagree. -----------------------------
    Verdict {
        yaml: b"a: 1\n \tb: 2\n",
        loader: false,
        validator: Validator::RejectsTheTab,
        why: "the loader folds the line into the preceding plain scalar before the \
              dispatcher sees it (parser.rs, the `start_indent == 0` continuation \
              arm), yielding {\"a\":\"1 b\"} — that is #371, which #173 does not reach",
    },
    Verdict {
        yaml: b"a: |\n    x\n  \tb: c\n",
        loader: true,
        validator: Validator::Accepts,
        why: "indent 2 < the block's content indent 4, so the scalar ends and the \
              tab really is indentation; the validator's block-scalar body check \
              measures against the parent indent and skips the line",
    },
];

/// The #106 artefact: one definition of `line_is_structural`, and a test that its
/// two call sites — the loader and the validator — classify the same bytes the same
/// way, with every deliberate divergence named.
#[test]
fn loader_and_validator_agree_on_tab_indentation() {
    let mut wrong = Vec::new();
    for v in VERDICTS {
        let text = String::from_utf8_lossy(v.yaml);
        let loader = loader_reports_tab_indentation(v.yaml);
        let validator = validator_verdict(v.yaml);
        if loader != v.loader {
            wrong.push(format!(
                "  {text:?}: loader should {} — {}",
                if v.loader {
                    "report TabIndentation but accepted it"
                } else {
                    "accept but reported TabIndentation"
                },
                v.why
            ));
        }
        if validator != v.validator {
            wrong.push(format!(
                "  {text:?}: validator {validator:?}, expected {:?} — {}",
                v.validator, v.why
            ));
        }
    }
    assert!(wrong.is_empty(), "verdicts changed:\n{}", wrong.join("\n"));
}

/// The acceptance side, pinned by **output** rather than by `is_ok()`: a document
/// where the tab is separation must keep producing the same value, so "tab is
/// separation" cannot regress into "tab is an error" — nor into silently different
/// scalar content.
#[test]
fn a_tab_used_as_separation_still_produces_its_value() {
    // UV7Q. This is the spec-correct output; the suite agrees.
    assert_eq!(to_json(b"x:\n - x\n  \tx\n").unwrap(), r#"{"x":["x x"]}"#);

    // DK95/00. The leading tab is separation and is stripped, matching the suite
    // (#381). Previously a known scalar-folding gap, on record in
    // tests/data/yaml-test-suite-known-failures.txt as `DK95/00 scalars`.
    assert_eq!(to_json(b"foo:\n \tbar\n").unwrap(), r#"{"foo":"bar"}"#);

    // Q5MG and 6CA3, both spec-correct.
    assert_eq!(to_json(b"\t{}\n").unwrap(), "{}");
    assert_eq!(to_json(b"\t[\n\t]\n").unwrap(), "[]");

    // A tab inside a block scalar body is content, and survives verbatim.
    assert_eq!(
        to_json(b"a: |\n  x\n  \tb: c\n").unwrap(),
        r#"{"a":"x\n\tb: c\n"}"#
    );

    // A tab before a comment is separation, and the comment is dropped.
    assert_eq!(
        to_json(b"a: 1\n \t# c: d\nb: 2\n").unwrap(),
        r#"{"a":1,"b":2}"#
    );

    // A quoted scalar after the tab. The tab is stripped as separation, so the
    // opening quote is the node's first byte and it reads as the quoted scalar
    // it is, rather than as a mapping (#381).
    assert_eq!(to_json(b"a:\n \t\"x: y\"\n").unwrap(), r#"{"a":"x: y"}"#);

    // The single-quoted form of the row above — a separate dispatch arm
    // (`parse_single_quoted`), so it needs its own pinned value rather than
    // relying on the double-quoted case to stand in for it.
    assert_eq!(to_json(b"a:\n \t'x: y'\n").unwrap(), r#"{"a":"x: y"}"#);
}

/// `parse_document_line` is not always entered at a line start — `parse_explicit_key`
/// returns mid-line, and the flow and quoted scanners stop just past their closing
/// delimiter, after which the main loop re-derives an "indent" from a mid-line
/// cursor. A tab reached that way is separation, never indentation, so it must not
/// be blamed on indentation. These inputs are malformed for other reasons; what is
/// asserted is *which* complaint they draw.
#[test]
fn a_mid_line_tab_is_never_blamed_on_indentation() {
    for yaml in [
        &b"[1] \tfoo: bar\n"[..],
        b"\"a\" \tb: c\n",
        b"? k: v\n \tx\n",
    ] {
        assert!(
            !loader_reports_tab_indentation(yaml),
            "{:?} was reported as tab indentation",
            String::from_utf8_lossy(yaml)
        );
    }
}

/// Corpus guardrail, the loader-side counterpart to `validator_accepts_all_valid_cases`
/// in `tests/yaml_test_suite.rs`: tightening the tab rule must never make the loader
/// reject a **valid** suite case as tab indentation.
///
/// The conformance harness would catch a valid case that stops loading, but only as
/// "newly failing" among 402 cases. This says what went wrong and why.
#[test]
fn the_loader_never_reports_tab_indentation_for_a_valid_corpus_case() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(CORPUS).expect("corpus is valid JSON");
    let mut rejected = Vec::new();
    let mut checked = 0usize;

    for case in &cases {
        if case["fail"].as_bool().unwrap_or(false) {
            continue;
        }
        let yaml = case["yaml"].as_str().expect("case has yaml");
        checked += 1;
        if loader_reports_tab_indentation(yaml.as_bytes()) {
            rejected.push(format!(
                "  {}: {}",
                case["id"].as_str().unwrap_or("?"),
                case["name"].as_str().unwrap_or_default()
            ));
        }
    }

    assert!(
        checked > 250,
        "corpus looks truncated ({checked} valid cases)"
    );
    assert!(
        rejected.is_empty(),
        "the loader reported tab indentation for {} valid case(s):\n{}",
        rejected.len(),
        rejected.join("\n")
    );
}
