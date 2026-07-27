//! A tab around a `:` value indicator is separation, never content (#370).
//!
//! YAML's [`ns-plain`] excludes trailing white space, and
//! `ns-s-implicit-yaml-key(c) ::= ns-yaml-key(c) s-separate-in-line?` puts the white
//! space between a key and its `:` outside the key. `s-white` is a space *or a tab*,
//! so `a\t: 1` is the mapping `{"a": 1}` — the tab belongs to neither side.
//!
//! [`ns-plain`]: https://yaml.org/spec/1.2.2/#733-plain-style
//!
//! This is the separation half of the split that `tests/yaml_tab_indentation_tests.rs`
//! draws: there, a tab before *block structure* is illegal indentation; here, a tab
//! before an *indicator on the same line* is legal separation and must be discarded.
//!
//! # Why the byte ranges are asserted too
//!
//! A scalar's extent is derived twice, by two hand-written copies that never consult
//! each other:
//!
//! | derivation | in | reaches the user as |
//! |---|---|---|
//! | the parser's, stored in the index | `parse_unquoted_key` (`src/yaml/parser.rs`) | `syq` output, key lookup |
//! | the cursor's, re-derived from text | `find_scalar_end` (`src/yaml/light.rs`) | `syq-locate`, `at_offset` |
//!
//! Before #370 *both* omitted the tab, and fixing only the first would have left
//! `syq` printing `a` while `syq-locate` reported the byte range `a\t` — a divergence
//! no output-only test can see. The cursor copy also dropped the tab from its
//! *terminator* set, so `a:\t1` — which has no trailing tab at all — located as a
//! range running to end of input. Asserting both halves is what keeps the two copies
//! honest; see the #106 lesson in `CLAUDE.md` on predicates that diverge silently.
//!
//! Inputs are byte literals so that no editor, `.gitattributes` or clippy lint can
//! quietly launder a tab into spaces.
//!
//! Run with: `cargo test --test yaml_tab_separation_tests`

use succinctly::yaml::{locate_offset_detailed, YamlError, YamlIndex, YamlValue};

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

/// One document whose tabs are all separation, with the value it must produce.
struct Row {
    yaml: &'static [u8],
    json: &'static str,
    /// Which part of the fix this row is here to hold down.
    why: &'static str,
}

/// A plain key long enough to reach the 32-byte SIMD classifier in
/// `skip_unquoted_simd`, which only the x86_64 leg of CI exercises (the ARM64
/// broadword path is disabled — see P4 in `docs/parsing/yaml.md`). The trim runs
/// after the scan loop on absolute positions, so it is not path-dependent; this is
/// insurance against a future classifier that stops somewhere else.
const LONG_KEY: &[u8] = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\t: 1\n";

const SEPARATION: &[Row] = &[
    Row {
        yaml: b"a\t: 1\n",
        json: r#"{"a":1}"#,
        why: "the #370 repro: loaded as {\"a\\t\":1} before the fix, so `.a` missed",
    },
    Row {
        yaml: b"a \t : 1\n",
        json: r#"{"a":1}"#,
        why: "a mixed run of separation, not one byte of it",
    },
    Row {
        yaml: b"a\t\t: 1\n",
        json: r#"{"a":1}"#,
        why: "the trim loop runs to exhaustion, not once",
    },
    Row {
        yaml: b"a\t:\t1\n",
        json: r#"{"a":1}"#,
        why: "a tab on both sides: the value indicator may be followed by s-white too",
    },
    Row {
        yaml: b"- a\t: 1\n",
        json: r#"[{"a":1}]"#,
        why: "parse_compact_mapping_entry, the other caller of parse_unquoted_key",
    },
    Row {
        yaml: b"a\t: 1\r\n",
        json: r#"{"a":1}"#,
        why: "tab and CR are trimmed by the same loop — the #324 fix pairs with this one",
    },
    Row {
        yaml: LONG_KEY,
        json: r#"{"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":1}"#,
        why: "a key past the SIMD classifier threshold",
    },
    Row {
        yaml: b"a: 1\t\n",
        json: r#"{"a":1}"#,
        why: "the value side, already correct — pins that the cursor-side edits do not \
               regress it into the string \"1\\t\"",
    },
];

#[track_caller]
fn assert_json(yaml: &[u8], expected: &str, why: &str) {
    let actual =
        to_json(yaml).unwrap_or_else(|e| panic!("{:?} failed to parse: {e} — {why}", Text(yaml)));
    assert_eq!(actual, expected, "input {:?} — {why}", Text(yaml));
}

/// Renders a byte string with its tabs visible in assertion output.
struct Text<'a>(&'a [u8]);

impl core::fmt::Debug for Text<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.0))
    }
}

#[test]
fn a_tab_around_the_value_indicator_is_not_part_of_the_key() {
    for row in SEPARATION {
        assert_json(row.yaml, row.json, row.why);
    }
}

/// The property behind the table: separation is separation whichever `s-white` byte
/// spells it. Stated as an invariance so a future change has to break the *rule* to
/// break the test, not just one of eight pinned strings — and the space form is
/// anchored to the same expectation, so "both spellings are equally wrong" cannot
/// pass for agreement.
#[test]
fn spelling_the_separation_with_a_space_gives_the_same_document() {
    for row in SEPARATION {
        let spaces: Vec<u8> = row
            .yaml
            .iter()
            .map(|&b| if b == b'\t' { b' ' } else { b })
            .collect();
        assert_json(&spaces, row.json, row.why);

        let tabbed = to_json(row.yaml).expect("the tab form parses");
        let spaced = to_json(&spaces).expect("the space form parses");
        assert_eq!(
            tabbed,
            spaced,
            "{:?} and {:?} are the same document — {}",
            Text(row.yaml),
            Text(&spaces),
            row.why
        );
    }
}

/// The other side of the rule, and the one a too-eager trim would break: a tab
/// *inside* a plain scalar is ordinary content. `nb-ns-plain-in-line` admits
/// `s-white*` between plain characters, so these tabs are part of the scalar and must
/// survive verbatim.
#[test]
fn a_tab_inside_a_plain_scalar_is_content() {
    assert_json(b"a\tb: 1\n", r#"{"a\tb":1}"#, "internal tab in a key");
    assert_json(b"k: a\tb\n", r#"{"k":"a\tb"}"#, "internal tab in a value");
    assert_json(
        b"a\tb\t: 1\n",
        r#"{"a\tb":1}"#,
        "internal kept, trailing trimmed",
    );
}

/// The `syq-locate` half. `YamlCursor::raw_bytes` re-derives the extent rather than
/// reusing the index, so these would still have been wrong had #370 changed only the
/// parser — and the `a:\t1` row was wrong regardless of #370's shape.
#[test]
fn the_located_byte_range_excludes_the_separating_tab() {
    // (document, offset to locate, the bytes the reported range must cover)
    for (yaml, offset, expected) in [
        (&b"a\t: 1\n"[..], 0, &b"a"[..]), // key: range was `a\t`
        (b"a\t: 1\n", 4, b"1"),           // its value
        (b"a:\t1\n", 0, b"a"),            // key: range ran to end of input
        (b"a:\t1\n", 3, b"1"),            // its value
        (b"a\t:\t1\n", 0, b"a"),          // both sides tabbed
        (b"a\t:\t1\n", 4, b"1"),
        (b"a\tb: 1\n", 0, b"a\tb"), // content, so the range keeps the tab
    ] {
        let index = YamlIndex::build(yaml).expect("parses");
        let found = locate_offset_detailed(&index, yaml, offset)
            .unwrap_or_else(|| panic!("offset {offset} of {:?} located nothing", Text(yaml)));
        let (start, end) = found.byte_range;
        assert_eq!(
            &yaml[start..end],
            expected,
            "offset {offset} of {:?}: range {:?} covers {:?}",
            Text(yaml),
            found.byte_range,
            Text(&yaml[start..end])
        );
    }
}

/// The opt-in strict validator must agree that these are valid documents. A tab here
/// is separation, so the tab-in-indentation rule (#173) must not fire on it — the two
/// rules meet on the same byte and only the position tells them apart.
#[test]
fn the_strict_validator_accepts_every_separation_form() {
    let mut rejected = Vec::new();
    for row in SEPARATION {
        if let Err(e) = succinctly::yaml::validate::validate(row.yaml) {
            rejected.push(format!("  {:?}: {e} — {}", Text(row.yaml), row.why));
        }
    }
    assert!(
        rejected.is_empty(),
        "the validator rejected {} valid document(s):\n{}",
        rejected.len(),
        rejected.join("\n")
    );
}
