//! Integration tests for XML semi-indexing (issue #667, xq milestone 1).
//!
//! Unlike `src/xml/scan.rs`'s and `src/xml/light.rs`'s own unit tests (which
//! exercise scanner internals and `XmlCursor`'s inherent methods directly),
//! these tests go through the public API only, and specifically through the
//! same generic jq evaluator (`succinctly::jq::eval_generic`) that JSON/YAML
//! use — proving `XmlIndex`/`XmlCursor`/`XmlValue`'s `DocumentValue`/
//! `DocumentCursor` trait implementations actually work end-to-end, not just
//! their own inherent methods.

use succinctly::jq::document::DocumentFields;
use succinctly::jq::eval_generic::{eval_with_cursor, to_owned, GenericResult};
use succinctly::jq::{parse, OwnedValue};
use succinctly::xml::{locate_offset, locate_offset_detailed, XmlIndex};

/// Evaluate `filter` against `xml` through the generic evaluator and collect
/// every output value as an `OwnedValue`, panicking on an uncaught error —
/// the shape most of these tests want.
fn eval_xml(xml: &[u8], filter: &str) -> Vec<OwnedValue> {
    let index = XmlIndex::build(xml).expect("valid XML fixture");
    let cursor = index.root(xml);
    let expr = parse(filter).expect("valid filter");
    match eval_with_cursor(&expr, cursor) {
        GenericResult::One(v) => vec![to_owned(&v)],
        GenericResult::OneCursor(c) => vec![to_owned(&c.value())],
        GenericResult::Many(vs) => vs.iter().map(to_owned).collect(),
        GenericResult::ManyCursor(cs) => cs.iter().map(|c| to_owned(&c.value())).collect(),
        GenericResult::LazyKeys { fields, sorted } => {
            let mut keys = fields.keys();
            if sorted {
                keys.sort();
            }
            vec![OwnedValue::Array(
                keys.into_iter().map(OwnedValue::String).collect(),
            )]
        }
        GenericResult::LazyIndexRange(len) => vec![OwnedValue::Array(
            (0..len).map(|i| OwnedValue::Int(i as i64)).collect(),
        )],
        GenericResult::None => vec![],
        GenericResult::Owned(v) => vec![v],
        GenericResult::ManyOwned(vs) => vs,
        GenericResult::Error(e) => panic!("unexpected eval error: {}", e.message),
        GenericResult::Break(label) => panic!("unexpected break: {label}"),
        GenericResult::Partial(_, ctrl) => panic!("unexpected partial result: {ctrl:?}"),
    }
}

const USERS_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<users>
  <user id="1">
    <name>Alice</name>
    <email>alice@example.com</email>
  </user>
  <user id="2">
    <name>Bob</name>
  </user>
</users>
"#;

/// The acceptance criterion from issue #667, verbatim: `.foo.bar`-style
/// navigation must work against a basic XML document via the format-agnostic
/// jq evaluator.
#[test]
fn foo_bar_style_navigation_works() {
    let xml = b"<root><foo><bar>hello</bar></foo></root>";
    let values = eval_xml(xml, ".foo.bar");
    assert_eq!(values.len(), 1);
    let content = match &values[0] {
        OwnedValue::Object(map) => map.get("+content").cloned(),
        other => panic!("expected object, got {other:?}"),
    };
    assert_eq!(content, Some(OwnedValue::String("hello".to_string())));
}

#[test]
fn attribute_navigation() {
    let values = eval_xml(USERS_XML, r#".user."+@id""#);
    // First `<user>` wins for singular `.user` access (decision #2).
    assert_eq!(values, vec![OwnedValue::String("1".to_string())]);
}

#[test]
fn attribute_value_stays_a_string_not_a_number() {
    // Regression test: `id="1"` must materialize as OwnedValue::String, not
    // Int — DocumentValue::as_i64/as_f64/as_bool must never auto-coerce XML
    // text, since to_owned() (src/jq/eval_generic.rs) checks them before
    // as_str(). A numeric attribute silently turning into a JSON number on
    // every full-value dump was a real bug caught by manual CLI testing.
    let values = eval_xml(USERS_XML, r#".user."+@id" | type"#);
    assert_eq!(values, vec![OwnedValue::String("string".to_string())]);
}

#[test]
fn tonumber_still_coerces_explicitly() {
    let values = eval_xml(USERS_XML, r#".user."+@id" | tonumber"#);
    assert_eq!(values, vec![OwnedValue::Int(1)]);
}

#[test]
fn element_projects_to_object_never_array() {
    let values = eval_xml(USERS_XML, ".user | (type, (. | tostring | length > 0))");
    assert_eq!(values[0], OwnedValue::String("object".to_string()));
}

#[test]
fn keys_lists_attributes_and_children_with_the_documented_prefix_convention() {
    let values = eval_xml(USERS_XML, ".user | keys");
    let OwnedValue::Array(keys) = &values[0] else {
        panic!("expected array");
    };
    let keys: Vec<&str> = keys
        .iter()
        .map(|v| match v {
            OwnedValue::String(s) => s.as_str(),
            _ => panic!("expected string key"),
        })
        .collect();
    assert!(keys.contains(&"+@id"));
    assert!(keys.contains(&"name"));
}

/// Regression test: `keys_unsorted`'s `LazyKeysUnsorted` fast path
/// (`eval_generic.rs`, #140) forwards `DocumentField::key_cursor` and later
/// calls `.value()` on it for `.[]`/`.[n]`/`first`/`last`. JSON/YAML's
/// `key_cursor` points at a real key node, so that's correct there; XML has
/// no separate key node, so `XmlFields::uncons` must hand back a cursor
/// whose `.value()` yields the synthesized key (`XmlValue::Key`), not the
/// field's real value — otherwise these all silently return field values
/// instead of key names (caught by manual CLI testing, not by this suite
/// before this test existed).
#[test]
fn keys_unsorted_iteration_yields_key_names_not_field_values() {
    let xml = br#"<root id="1" name="x"><child>hi</child></root>"#;

    let via_iterate = eval_xml(xml, "keys_unsorted | .[]");
    assert_eq!(
        via_iterate,
        vec![
            OwnedValue::String("+@id".to_string()),
            OwnedValue::String("+@name".to_string()),
            OwnedValue::String("child".to_string()),
        ]
    );

    assert_eq!(
        eval_xml(xml, "keys_unsorted | first"),
        vec![OwnedValue::String("+@id".to_string())]
    );
    assert_eq!(
        eval_xml(xml, "keys_unsorted | last"),
        vec![OwnedValue::String("child".to_string())]
    );
    assert_eq!(
        eval_xml(xml, "keys_unsorted | .[1]"),
        vec![OwnedValue::String("+@name".to_string())]
    );
}

#[test]
fn identity_round_trips_through_owned_value() {
    let xml = br#"<root a="1"><child>text &amp; more</child></root>"#;
    let values = eval_xml(xml, ".");
    assert_eq!(values.len(), 1);
    let OwnedValue::Object(root) = &values[0] else {
        panic!("expected object");
    };
    assert_eq!(root.get("+@a"), Some(&OwnedValue::String("1".to_string())));
    let OwnedValue::Object(child) = root.get("child").expect("child field") else {
        panic!("expected object");
    };
    assert_eq!(
        child.get("+content"),
        Some(&OwnedValue::String("text & more".to_string()))
    );
}

#[test]
fn at_offset_and_at_position_navigate_to_the_same_node_as_the_locate_cli_would() {
    // "alice@example.com" appears once in USERS_XML — unlike "Bob" (whose
    // enclosing `.user.name` path is ambiguous between the two `<user>`
    // siblings, see the test below), a position inside it round-trips
    // through a path string unambiguously.
    let offset = USERS_XML.windows(5).position(|w| w == b"alice").unwrap();

    let by_offset = eval_xml(USERS_XML, &format!("at_offset({offset})"));
    assert_eq!(
        by_offset,
        vec![OwnedValue::String("alice@example.com".to_string())]
    );

    let index = XmlIndex::build(USERS_XML).unwrap();
    let (line, col) = index.to_line_column(offset, USERS_XML);
    let by_position = eval_xml(USERS_XML, &format!("at_position({line}; {col})"));
    assert_eq!(by_position, by_offset);

    // Cross-check against the reverse direction (`xq-locate`'s core): the
    // path found for that offset, evaluated fresh, reaches the same value.
    let expr = locate_offset(&index, USERS_XML, offset).expect("locatable offset");
    assert_eq!(eval_xml(USERS_XML, &expr), by_offset);
}

#[test]
fn locate_path_string_is_ambiguous_for_repeated_sibling_keys() {
    // Documents the known, shared limitation (not new to XML — JSON/YAML's
    // own locate implementations have the same property): a path string
    // like `.user.name."+content"` can't distinguish which `<user>` it came
    // from, since `find`/`find_cursor` always resolve a bareword key to the
    // *first* match. `at_offset`/`at_position` themselves stay precise
    // (position-addressed, not key-addressed) — only the round-trip through
    // a *path string* loses that precision.
    let offset = USERS_XML.windows(3).position(|w| w == b"Bob").unwrap();
    let index = XmlIndex::build(USERS_XML).unwrap();

    let by_offset = eval_xml(USERS_XML, &format!("at_offset({offset})"));
    assert_eq!(by_offset, vec![OwnedValue::String("Bob".to_string())]);

    let expr = locate_offset(&index, USERS_XML, offset).expect("locatable offset");
    assert_eq!(expr, ".user.name[\"+content\"]");
    // Re-evaluating that same expression resolves to the *first* <user>'s
    // name instead — the documented ambiguity, not a round-trip bug.
    assert_eq!(
        eval_xml(USERS_XML, &expr),
        vec![OwnedValue::String("Alice".to_string())]
    );
}

#[test]
fn locate_offset_detailed_reports_string_type_and_exact_byte_range() {
    let offset = USERS_XML.windows(3).position(|w| w == b"Bob").unwrap();
    let index = XmlIndex::build(USERS_XML).unwrap();
    let result = locate_offset_detailed(&index, USERS_XML, offset).expect("locatable offset");
    assert_eq!(result.value_type, "string");
    assert_eq!(&USERS_XML[result.byte_range.0..result.byte_range.1], b"Bob");
}

#[test]
fn malformed_xml_is_rejected_not_silently_misindexed() {
    assert!(XmlIndex::build(b"<root><unclosed></root>").is_err());
    assert!(XmlIndex::build(b"not xml at all").is_err());
    assert!(XmlIndex::build(b"").is_err());
}

#[test]
fn self_closing_and_nested_repeated_siblings() {
    let xml = b"<root><item n=\"1\"/><item n=\"2\"/><item n=\"3\"/></root>";
    // Singular access sees the first (decision #2's documented limitation:
    // no automatic array grouping over repeated same-name children).
    let values = eval_xml(xml, r#".item."+@n""#);
    assert_eq!(values, vec![OwnedValue::String("1".to_string())]);
}
