//! A `#` not preceded by white space is plain scalar content, not a comment (#411).
//!
//! YAML's [`ns-plain-char`] excludes `#` only when it follows `nb-char` that is
//! itself white space — `c-comment` requires `#` to be preceded by `s-white` or to
//! start the line. So `a#b` is one scalar, `{"a#b": 1}`, not `a` followed by a
//! comment.
//!
//! [`ns-plain-char`]: https://yaml.org/spec/1.2.2/#733-plain-style
//!
//! # Why the byte ranges are asserted too
//!
//! A scalar's extent is derived twice, by two hand-written copies that never
//! consult each other:
//!
//! | derivation | in | reaches the user as |
//! |---|---|---|
//! | the parser's, stored in the index | `parse_unquoted_key` / `parse_unquoted_value_*` (`src/yaml/parser.rs`) | `syq` output, key lookup |
//! | the cursor's, re-derived from text | `find_scalar_end` (`src/yaml/light.rs`) | `syq-locate`, `at_offset` |
//!
//! Before #411, `find_scalar_end` broke on every `#` unconditionally, so `syq`
//! printed the key as `a#b` while `syq-locate` reported a byte range covering only
//! `a` — a divergence no output-only (expression-only) test can see, because the
//! expression is built from the index and was already correct. This is the same
//! class of bug as #370 (tab separation), just triggered by `#` instead of a tab;
//! see the #106 lesson in `CLAUDE.md` on predicates that diverge silently.
//!
//! Run with: `cargo test --test yaml_hash_scalar_tests`

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

/// Renders a byte string with any non-ASCII-printable bytes visible in assertion
/// output.
struct Text<'a>(&'a [u8]);

impl core::fmt::Debug for Text<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.0))
    }
}

#[track_caller]
fn assert_json(yaml: &[u8], expected: &str, why: &str) {
    let actual =
        to_json(yaml).unwrap_or_else(|e| panic!("{:?} failed to parse: {e} — {why}", Text(yaml)));
    assert_eq!(actual, expected, "input {:?} — {why}", Text(yaml));
}

/// A `#` with nothing but non-white-space before it is content, on both sides of
/// the `:`. This is the parser half — already correct before #411, but pinned here
/// so the byte-range test below has a known-good baseline to diverge from.
#[test]
fn a_hash_not_preceded_by_white_space_is_scalar_content() {
    assert_json(
        b"a#b: 1\n",
        r#"{"a#b":1}"#,
        "the #411 repro: a hash inside a key",
    );
    assert_json(b"k: a#b\n", r#"{"k":"a#b"}"#, "a hash inside a value");
    assert_json(
        b"a#b#c: 1\n",
        r#"{"a#b#c":1}"#,
        "more than one hash, none preceded by white space",
    );
}

/// The other side of the rule: a `#` preceded by a space or tab starts a comment
/// and ends the scalar, whether or not it is also preceded by more scalar content.
#[test]
fn a_hash_preceded_by_white_space_starts_a_comment() {
    assert_json(b"a: 1 # comment\n", r#"{"a":1}"#, "comment after a value");
    assert_json(
        b"a: 1 # comment: b\n",
        r#"{"a":1}"#,
        "a colon inside the comment does not confuse the value's extent",
    );
    assert_json(
        b"a\t# comment\n",
        r#""a""#,
        "a tab also counts as separation before `#`",
    );
}

/// The `syq-locate` half — the bug in #411. `YamlCursor::raw_bytes` re-derives the
/// extent from text rather than reusing the index's already-correct extent, and
/// before the fix it broke on every `#` regardless of what preceded it.
#[test]
fn the_located_byte_range_includes_a_hash_that_is_not_a_comment() {
    // (document, offset to locate, the bytes the reported range must cover)
    for (yaml, offset, expected) in [
        (&b"a#b: 1\n"[..], 0, &b"a#b"[..]), // key: range was `a`
        (b"a#b: 1\n", 5, b"1"),             // its value, unaffected
        (b"k: a#b\n", 3, b"a#b"),           // value: range was `a`
        (b"a#b#c: 1\n", 0, b"a#b#c"),       // more than one hash in the key
        (b"a: 1 # comment\n", 3, b"1"),     // comment excluded, not swallowed into the value
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
