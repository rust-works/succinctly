//! Integration tests for jq query functionality.

use succinctly::jq::{eval, eval_lenient, parse, JqSemantics, OwnedValue, QueryResult};
use succinctly::json::light::StandardJson;
use succinctly::json::JsonIndex;

/// Helper macro to run a query and match the result.
/// Keeps the JsonIndex alive for the duration of the match.
macro_rules! query {
    ($json:expr, $expr:expr, $pattern:pat $(if $guard:expr)? => $body:expr) => {{
        let json_bytes: &[u8] = $json;
        let index = JsonIndex::build(json_bytes);
        let cursor = index.root(json_bytes);
        let expr = parse($expr).expect("parse failed");
        match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
            $pattern $(if $guard)? => $body,
            other => panic!("unexpected result: {:?}", other),
        }
    }};
}

/// Helper macro to run eval_lenient and check results.
macro_rules! query_lenient {
    ($json:expr, $expr:expr, len == $expected:expr) => {{
        let json_bytes: &[u8] = $json;
        let index = JsonIndex::build(json_bytes);
        let cursor = index.root(json_bytes);
        let expr = parse($expr).expect("parse failed");
        let results = eval_lenient::<Vec<u64>, JqSemantics>(&expr, cursor);
        assert_eq!(
            results.len(),
            $expected,
            "expected {} results, got {}",
            $expected,
            results.len()
        );
    }};
    ($json:expr, $expr:expr, is_empty) => {{
        let json_bytes: &[u8] = $json;
        let index = JsonIndex::build(json_bytes);
        let cursor = index.root(json_bytes);
        let expr = parse($expr).expect("parse failed");
        let results = eval_lenient::<Vec<u64>, JqSemantics>(&expr, cursor);
        assert!(
            results.is_empty(),
            "expected empty results, got {}",
            results.len()
        );
    }};
}

// =============================================================================
// Identity tests
// =============================================================================

// Identity (.) now returns OneCursor for efficient passthrough of unchanged values

#[test]
fn test_identity_object() {
    query!(br#"{"a": 1}"#, ".", QueryResult::OneCursor(_) => {});
}

#[test]
fn test_identity_array() {
    query!(br"[1, 2, 3]", ".", QueryResult::OneCursor(_) => {});
}

#[test]
fn test_identity_string() {
    query!(br#""hello""#, ".", QueryResult::OneCursor(_) => {});
}

#[test]
fn test_identity_number() {
    query!(b"42", ".", QueryResult::OneCursor(_) => {});
}

#[test]
fn test_identity_bool() {
    query!(b"true", ".", QueryResult::OneCursor(_) => {});
}

#[test]
fn test_identity_null() {
    query!(b"null", ".", QueryResult::OneCursor(_) => {});
}

// =============================================================================
// Field access tests
// =============================================================================

#[test]
fn test_field_string_value() {
    query!(br#"{"name": "Alice", "city": "NYC"}"#, ".name",
        QueryResult::One(StandardJson::String(s)) => {
            assert_eq!(s.as_str().unwrap().as_ref(), "Alice");
        }
    );
}

#[test]
fn test_field_number_value() {
    query!(br#"{"age": 30, "score": 95.5}"#, ".age",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 30);
        }
    );
}

#[test]
fn test_field_nested_object() {
    query!(br#"{"user": {"name": "Bob"}}"#, ".user",
        QueryResult::One(StandardJson::Object(_)) => {}
    );
}

#[test]
fn test_field_missing_returns_null() {
    // jq returns null for missing fields on objects (not an error)
    query!(br#"{"a": 1}"#, ".missing",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_field_on_non_object_error() {
    query!(br"[1, 2, 3]", ".foo",
        QueryResult::Error(e) => {
            assert!(e.message.contains("object"), "expected type error");
        }
    );
}

#[test]
fn test_field_with_underscore() {
    query!(br#"{"user_name": "test", "_private": true}"#, ".user_name",
        QueryResult::One(StandardJson::String(s)) => {
            assert_eq!(s.as_str().unwrap().as_ref(), "test");
        }
    );
    query!(br#"{"user_name": "test", "_private": true}"#, "._private",
        QueryResult::One(StandardJson::Bool(b)) => {
            assert!(b);
        }
    );
}

// =============================================================================
// Array index tests
// =============================================================================

#[test]
fn test_index_first() {
    query!(br"[10, 20, 30]", ".[0]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 10);
        }
    );
}

#[test]
fn test_index_last() {
    query!(br"[10, 20, 30]", ".[2]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 30);
        }
    );
}

#[test]
fn test_index_negative() {
    query!(br"[10, 20, 30, 40]", ".[-1]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 40);
        }
    );
    query!(br"[10, 20, 30, 40]", ".[-2]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 30);
        }
    );
}

#[test]
fn test_index_out_of_bounds_returns_null() {
    // jq returns null for out-of-bounds array access (not an error)
    query!(br"[1, 2, 3]", ".[10]",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_index_negative_out_of_bounds_returns_null() {
    // jq returns null for negative out-of-bounds array access (not an error)
    query!(br"[1, 2, 3]", ".[-10]",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_index_on_null_returns_null() {
    // jq returns null when indexing null
    query!(b"null", ".[0]",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_index_on_null_negative_returns_null() {
    // jq returns null when indexing null with negative index
    query!(b"null", ".[-1]",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_index_on_non_array_error() {
    query!(br#"{"a": 1}"#, ".[0]",
        QueryResult::Error(e) => {
            assert!(e.message.contains("array"));
        }
    );
}

// =============================================================================
// Iteration tests
// =============================================================================

#[test]
fn test_iterate_array() {
    query!(br"[1, 2, 3, 4, 5]", ".[]",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 5);
        }
    );
}

#[test]
fn test_iterate_object_values() {
    query!(br#"{"a": 1, "b": 2, "c": 3}"#, ".[]",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 3);
            // All should be numbers
            for v in &values {
                assert!(matches!(v, StandardJson::Number(_)));
            }
        }
    );
}

#[test]
fn test_iterate_empty_array() {
    query!(br"[]", ".[]",
        QueryResult::Many(values) => {
            assert!(values.is_empty());
        }
    );
}

#[test]
fn test_iterate_empty_object() {
    query!(br"{}", ".[]",
        QueryResult::Many(values) => {
            assert!(values.is_empty());
        }
    );
}

#[test]
fn test_iterate_on_scalar_error() {
    query!(b"42", ".[]",
        QueryResult::Error(e) => {
            assert!(e.message.contains("array or object"));
        }
    );
}

// =============================================================================
// Slice tests
// =============================================================================

#[test]
fn test_slice_range() {
    // jq array slicing yields a single sub-array, not a stream (issue #154).
    query!(br"[0, 1, 2, 3, 4, 5]", ".[1:4]",
        QueryResult::Owned(OwnedValue::Array(values)) => {
            assert_eq!(
                values,
                vec![OwnedValue::Int(1), OwnedValue::Int(2), OwnedValue::Int(3)]
            );
        }
    );
}

#[test]
fn test_slice_from_start() {
    query!(br"[0, 1, 2, 3, 4]", ".[:2]",
        QueryResult::Owned(OwnedValue::Array(values)) => {
            assert_eq!(values, vec![OwnedValue::Int(0), OwnedValue::Int(1)]);
        }
    );
}

#[test]
fn test_slice_to_end() {
    query!(br"[0, 1, 2, 3, 4]", ".[3:]",
        QueryResult::Owned(OwnedValue::Array(values)) => {
            assert_eq!(values, vec![OwnedValue::Int(3), OwnedValue::Int(4)]);
        }
    );
}

#[test]
fn test_slice_negative_indices() {
    query!(br"[0, 1, 2, 3, 4, 5]", ".[-3:-1]",
        QueryResult::Owned(OwnedValue::Array(values)) => {
            assert_eq!(values, vec![OwnedValue::Int(3), OwnedValue::Int(4)]);
        }
    );
}

#[test]
fn test_slice_empty_result() {
    // Out-of-range slice yields an empty array, not an empty stream.
    query!(br"[0, 1, 2]", ".[5:10]",
        QueryResult::Owned(OwnedValue::Array(values)) => {
            assert!(values.is_empty());
        }
    );
}

#[test]
fn test_slice_on_null_returns_null() {
    // jq returns null when slicing null
    query!(b"null", ".[0:2]",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_slice_on_string() {
    // jq supports string slicing
    query!(br#""hello""#, ".[1:3]",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "el");
        }
    );
}

#[test]
fn test_slice_on_string_from_start() {
    query!(br#""hello""#, ".[:2]",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "he");
        }
    );
}

#[test]
fn test_slice_on_string_to_end() {
    query!(br#""hello""#, ".[3:]",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "lo");
        }
    );
}

#[test]
fn test_slice_piped_to_length() {
    // A slice is a single array, so `length` sees the array (issue #154).
    query!(br"[0, 1, 2, 3, 4]", ".[1:4] | length",
        QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 3);
        }
    );
}

#[test]
fn test_slice_piped_to_index() {
    query!(br"[0, 1, 2, 3, 4]", ".[1:4] | .[0]",
        QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 1);
        }
    );
}

#[test]
fn test_slice_array_construction_nests() {
    // `[.[1:4]]` wraps the sub-array, producing a nested array.
    query!(br"[0, 1, 2, 3, 4]", "[.[1:4]]",
        QueryResult::Owned(OwnedValue::Array(outer)) => {
            assert_eq!(
                outer,
                vec![OwnedValue::Array(vec![
                    OwnedValue::Int(1),
                    OwnedValue::Int(2),
                    OwnedValue::Int(3),
                ])]
            );
        }
    );
}

#[test]
fn test_slice_single_element_stays_array() {
    // A one-element slice must remain an array, not collapse to the element.
    query!(br"[0, 1, 2, 3, 4]", "[.[]] | .[1:2]",
        QueryResult::Owned(OwnedValue::Array(values)) => {
            assert_eq!(values, vec![OwnedValue::Int(1)]);
        }
    );
}

#[test]
fn test_full_slice_returns_whole_array() {
    // `.[:]` is a full slice returning the whole array as a single value,
    // taking the fast path that returns the original borrowed value.
    query!(br"[0, 1, 2]", ".[:]",
        QueryResult::One(StandardJson::Array(_)) => {}
    );
}

// =============================================================================
// Optional tests
// =============================================================================

#[test]
fn test_optional_field_missing() {
    // jq returns null for missing fields on objects (even with optional syntax)
    query!(br#"{"a": 1}"#, ".missing?",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_optional_field_present() {
    query!(br#"{"a": 1}"#, ".a?",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 1);
        }
    );
}

#[test]
fn test_optional_index_out_of_bounds_returns_null() {
    // jq returns null for optional out-of-bounds (not empty)
    query!(br"[1, 2, 3]", ".[10]?",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_optional_index_on_null_returns_null() {
    // jq returns null for optional index on null
    query!(b"null", ".[0]?",
        QueryResult::One(StandardJson::Null) => {}
    );
}

#[test]
fn test_optional_on_wrong_type() {
    query!(b"42", ".foo?",
        QueryResult::None => {}
    );
}

#[test]
fn test_optional_iterate_on_scalar() {
    query!(b"42", ".[]?",
        QueryResult::None => {}
    );
}

// =============================================================================
// Chained expression tests
// =============================================================================

#[test]
fn test_chain_field_field() {
    query!(br#"{"user": {"name": "Alice"}}"#, ".user.name",
        QueryResult::One(StandardJson::String(s)) => {
            assert_eq!(s.as_str().unwrap().as_ref(), "Alice");
        }
    );
}

#[test]
fn test_chain_field_index() {
    query!(br#"{"items": [10, 20, 30]}"#, ".items[1]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 20);
        }
    );
}

#[test]
fn test_chain_index_field() {
    query!(br#"[{"name": "Alice"}, {"name": "Bob"}]"#, ".[0].name",
        QueryResult::One(StandardJson::String(s)) => {
            assert_eq!(s.as_str().unwrap().as_ref(), "Alice");
        }
    );
}

#[test]
fn test_chain_iterate_field() {
    query!(br#"{"users": [{"name": "A"}, {"name": "B"}, {"name": "C"}]}"#, ".users[].name",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 3);
        }
    );
}

#[test]
fn test_chain_deep_nesting() {
    query!(br#"{"a": {"b": {"c": {"d": {"e": 42}}}}}"#, ".a.b.c.d.e",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 42);
        }
    );
}

#[test]
fn test_chain_multiple_iterations() {
    query!(br#"{"matrix": [[1, 2], [3, 4], [5, 6]]}"#, ".matrix[][]",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 6);
        }
    );
}

// =============================================================================
// eval_lenient tests
// =============================================================================

#[test]
fn test_lenient_success() {
    query_lenient!(br#"{"name": "test"}"#, ".name", len == 1);
}

#[test]
fn test_lenient_missing_field_returns_null() {
    // jq returns null for missing fields - eval_lenient collects this as one result
    query_lenient!(br#"{"name": "test"}"#, ".missing", len == 1);
}

#[test]
fn test_lenient_missing_field_optional_returns_null() {
    // jq returns null for missing fields even with optional syntax
    query_lenient!(br#"{"name": "test"}"#, ".missing?", len == 1);
}

#[test]
fn test_lenient_many() {
    query_lenient!(br"[1, 2, 3]", ".[]", len == 3);
}

// =============================================================================
// Parser error tests
// =============================================================================

#[test]
fn test_parse_empty_error() {
    assert!(parse("").is_err());
}

#[test]
fn test_parse_func_call_syntax() {
    // "foo" now parses as FuncCall{name:"foo", args:[]} for user-defined functions
    // It will fail at evaluation time if the function doesn't exist
    use succinctly::jq::Expr;
    let expr = parse("foo").unwrap();
    assert!(matches!(expr, Expr::FuncCall { name, args } if name == "foo" && args.is_empty()));
}

#[test]
fn test_parse_unclosed_bracket_error() {
    assert!(parse(".[").is_err());
}

#[test]
fn test_parse_invalid_index_error() {
    assert!(parse(".[abc]").is_err());
}

#[test]
fn test_parse_field_starting_with_number_error() {
    assert!(parse(".123abc").is_err());
}

#[test]
fn test_parse_trailing_garbage_error() {
    assert!(parse(".foo bar").is_err());
}

// =============================================================================
// Edge cases and complex scenarios
// =============================================================================

#[test]
fn test_whitespace_in_expression() {
    query!(br#"{"a": 1}"#, " . a ",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 1);
        }
    );
}

#[test]
fn test_deeply_nested_arrays() {
    query!(br"[[[[1]]]]", ".[0][0][0][0]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 1);
        }
    );
}

#[test]
fn test_mixed_nesting() {
    query!(br#"{"data": [{"items": [{"value": 42}]}]}"#, ".data[0].items[0].value",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 42);
        }
    );
}

#[test]
fn test_unicode_field_values() {
    query!(br#"{"greeting": "Hello, \u4e16\u754c!"}"#, ".greeting",
        QueryResult::One(StandardJson::String(_)) => {}
    );
}

#[test]
fn test_escaped_strings() {
    query!(br#"{"text": "line1\nline2\ttab"}"#, ".text",
        QueryResult::One(StandardJson::String(_)) => {}
    );
}

#[test]
fn test_special_number_formats() {
    query!(br#"{"int": -42, "float": 3.14159, "exp": 1.5e10}"#, ".int",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), -42);
        }
    );

    query!(br#"{"int": -42, "float": 3.14159, "exp": 1.5e10}"#, ".float",
        QueryResult::One(StandardJson::Number(n)) => {
            let f = n.as_f64().unwrap();
            assert!((f - std::f64::consts::PI).abs() < 0.001);
        }
    );
}

// =============================================================================
// Real-world-like scenarios
// =============================================================================

#[test]
fn test_api_response_pattern() {
    let json = br#"{
        "status": "ok",
        "data": {
            "users": [
                {"id": 1, "name": "Alice", "active": true},
                {"id": 2, "name": "Bob", "active": false},
                {"id": 3, "name": "Charlie", "active": true}
            ],
            "total": 3
        }
    }"#;

    // Get all user names
    query!(json, ".data.users[].name",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 3);
        }
    );

    // Get second user's id
    query!(json, ".data.users[1].id",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 2);
        }
    );

    // Get total
    query!(json, ".data.total",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 3);
        }
    );
}

#[test]
fn test_config_file_pattern() {
    let json = br#"{
        "database": {
            "host": "localhost",
            "port": 5432,
            "credentials": {
                "user": "admin",
                "password": "secret"
            }
        },
        "features": ["auth", "logging", "metrics"]
    }"#;

    query!(json, ".database.host",
        QueryResult::One(StandardJson::String(s)) => {
            assert_eq!(s.as_str().unwrap().as_ref(), "localhost");
        }
    );

    query!(json, ".database.credentials.user",
        QueryResult::One(StandardJson::String(s)) => {
            assert_eq!(s.as_str().unwrap().as_ref(), "admin");
        }
    );

    query!(json, ".features[0]",
        QueryResult::One(StandardJson::String(s)) => {
            assert_eq!(s.as_str().unwrap().as_ref(), "auth");
        }
    );
}

// =============================================================================
// Date/Time builtin tests (Phase 7)
// =============================================================================

#[test]
fn test_now_returns_timestamp() {
    // now returns the current Unix timestamp as a float
    query!(b"null", "now",
        QueryResult::Owned(succinctly::jq::OwnedValue::Float(ts)) => {
            // Verify it's a reasonable timestamp (after 2024-01-01 and not too far in the future)
            let jan_2024 = 1704067200.0; // 2024-01-01 00:00:00 UTC
            let jan_2100 = 4102444800.0; // 2100-01-01 00:00:00 UTC
            assert!(ts > jan_2024, "timestamp {ts} should be after 2024-01-01");
            assert!(ts < jan_2100, "timestamp {ts} should be before 2100-01-01");
        }
    );
}

#[test]
fn test_now_ignores_input() {
    // now ignores its input
    query!(br#"{"foo": "bar"}"#, "now",
        QueryResult::Owned(succinctly::jq::OwnedValue::Float(ts)) => {
            let jan_2024 = 1704067200.0;
            assert!(ts > jan_2024, "timestamp {ts} should be after 2024-01-01");
        }
    );
}

// =============================================================================
// jq-compatibility: null handling tests
// These tests verify jq-compatible behavior where null is returned instead of
// errors for various edge cases. See issue #61 and related PRs.
// =============================================================================

#[test]
fn test_has_on_null_returns_false() {
    // jq: null | has("foo") => false
    query!(b"null", r#"has("foo")"#,
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b);
        }
    );
}

#[test]
fn test_has_on_object() {
    query!(br#"{"a": 1}"#, r#"has("a")"#,
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b);
        }
    );
}

#[test]
fn test_has_on_object_missing() {
    query!(br#"{"a": 1}"#, r#"has("b")"#,
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b);
        }
    );
}

#[test]
fn test_has_on_array() {
    query!(br"[1, 2, 3]", "has(1)",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b);
        }
    );
}

#[test]
fn test_has_on_array_out_of_bounds() {
    query!(br"[1, 2, 3]", "has(10)",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b);
        }
    );
}

#[test]
fn test_in_on_object() {
    // jq: "a" | in({"a": 1}) => true
    query!(br#""a""#, r#"in({"a": 1})"#,
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b);
        }
    );
}

#[test]
fn test_in_on_object_missing() {
    query!(br#""b""#, r#"in({"a": 1})"#,
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b);
        }
    );
}

#[test]
fn test_in_on_array() {
    // jq: 1 | in([10, 20, 30]) => true (index 1 exists)
    query!(b"1", "in([10, 20, 30])",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b);
        }
    );
}

#[test]
fn test_first_on_null_returns_null() {
    // jq: null | first => null
    query!(b"null", "first",
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

#[test]
fn test_first_on_empty_returns_null() {
    // jq: [] | first => null
    query!(b"[]", "first",
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

#[test]
fn test_last_on_null_returns_null() {
    // jq: null | last => null
    query!(b"null", "last",
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

#[test]
fn test_last_on_empty_returns_null() {
    // jq: [] | last => null
    query!(b"[]", "last",
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

#[test]
fn test_nth_on_null_returns_null() {
    // jq: null | nth(0) => null
    query!(b"null", "nth(0)",
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

#[test]
fn test_reverse_on_null_returns_empty_array() {
    // jq: null | reverse => []
    query!(b"null", "reverse",
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert!(arr.is_empty());
        }
    );
}

#[test]
fn test_values_on_null_returns_empty() {
    // jq: null | values => (no output)
    query!(b"null", "values",
        QueryResult::None => {}
    );
}

#[test]
fn test_getpath_on_null_returns_null() {
    // jq: null | getpath(["a"]) => null
    query!(b"null", r#"getpath(["a"])"#,
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

#[test]
fn test_getpath_on_null_nested() {
    // jq: null | getpath(["a", "b", "c"]) => null
    query!(b"null", r#"getpath(["a", "b", "c"])"#,
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

// =============================================================================
// jq-compatibility: format function tests
// =============================================================================

#[test]
fn test_uri_on_number() {
    // jq: 42 | @uri => "42" (converts to string first)
    query!(b"42", "@uri",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "42");
        }
    );
}

#[test]
fn test_uri_on_bool() {
    // jq: true | @uri => "true"
    query!(b"true", "@uri",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "true");
        }
    );
}

#[test]
fn test_html_on_number() {
    // jq: 42 | @html => "42"
    query!(b"42", "@html",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "42");
        }
    );
}

#[test]
fn test_sh_on_array() {
    // jq: [1, 2, 3] | @sh => "1 2 3"
    query!(b"[1, 2, 3]", "@sh",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "1 2 3");
        }
    );
}

#[test]
fn test_sh_on_array_with_strings() {
    // jq: ["a", "b c", "d"] | @sh => "'a' 'b c' 'd'"
    query!(br#"["a", "b c", "d"]"#, "@sh",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "'a' 'b c' 'd'");
        }
    );
}

// =============================================================================
// Compatibility tests - Arithmetic edge cases
// =============================================================================

#[test]
fn test_float_division_by_zero_returns_error() {
    // jq: 1.5 / 0 => error "cannot be divided because the divisor is zero"
    query!(b"null", "1.5 / 0",
        QueryResult::Error(_) => {}
    );
}

#[test]
fn test_float_modulo_by_zero_returns_error() {
    // jq: 1.5 % 0 => error "cannot be divided (remainder) because the divisor is zero"
    query!(b"null", "1.5 % 0",
        QueryResult::Error(_) => {}
    );
}

#[test]
fn test_integer_division_by_zero_returns_error() {
    // jq: 1 / 0 => error (integer division by zero)
    query!(b"null", "1 / 0",
        QueryResult::Error(_) => {}
    );
}

#[test]
fn test_integer_modulo_by_zero_returns_error() {
    // jq: 1 % 0 => error (integer modulo by zero)
    query!(b"null", "1 % 0",
        QueryResult::Error(_) => {}
    );
}

#[test]
fn test_integer_addition_overflow_converts_to_float() {
    // jq: 9223372036854775807 + 1 => 9223372036854776000 (float)
    query!(b"null", "9223372036854775807 + 1",
        QueryResult::Owned(OwnedValue::Float(f)) => {
            // jq converts to float, so result is approximately 9.22e18
            assert!(f > 9e18, "expected large positive float, got {f}");
        }
    );
}

#[test]
fn test_integer_multiplication_overflow_converts_to_float() {
    // jq: 9223372036854775807 * 2 => 18446744073709552000 (float)
    query!(b"null", "9223372036854775807 * 2",
        QueryResult::Owned(OwnedValue::Float(f)) => {
            // jq converts to float, so result is approximately 1.84e19
            assert!(f > 1e19, "expected large positive float, got {f}");
        }
    );
}

#[test]
fn test_integer_subtraction_overflow_converts_to_float() {
    // jq: -9223372036854775808 - 1 => -9223372036854776000 (float)
    query!(b"null", "-9223372036854775808 - 1",
        QueryResult::Owned(OwnedValue::Float(f)) => {
            assert!(f < -9e18, "expected large negative float, got {f}");
        }
    );
}

#[test]
fn test_string_multiply_positive_repeats() {
    // jq: "ab" * 2 => "abab"
    query!(b"null", r#""ab" * 2"#,
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "abab");
        }
    );
}

#[test]
fn test_string_multiply_zero_returns_empty_string() {
    // jq >= 1.7: "ab" * 0 => "" (changed from null in 1.6; jqlang/jq#1593).
    // #161 suggested null here, but that claim predates jq 1.7 -- verified
    // against jq-1.7.1.
    query!(b"null", r#""ab" * 0"#,
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "");
        }
    );
}

#[test]
fn test_string_multiply_negative_returns_null() {
    // jq: "ab" * -1 => null
    query!(b"null", r#""ab" * -1"#,
        QueryResult::Owned(OwnedValue::Null) => {}
    );
}

#[test]
fn test_length_of_negative_int_is_absolute_value() {
    // jq: -5 | length => 5
    query!(b"-5", "length",
        QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 5);
        }
    );
}

#[test]
fn test_length_of_i64_min_falls_back_to_float() {
    // jq: -9223372036854775808 | length => 9223372036854775808; -2^63 has no
    // i64 absolute value, so the result is the exact f64 2^63 (#161)
    query!(b"-9223372036854775808", "length",
        QueryResult::Owned(OwnedValue::Float(f)) => {
            assert_eq!(f, 9_223_372_036_854_775_808.0);
        }
    );
}

// =============================================================================
// Compatibility tests - has()/in() with negative indices
// =============================================================================

#[test]
fn test_has_negative_index_returns_false() {
    // jq: [1,2,3] | has(-1) => false
    query!(b"[1, 2, 3]", "has(-1)",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "has(-1) should return false for arrays");
        }
    );
}

#[test]
fn test_has_negative_two_returns_false() {
    // jq: [1,2,3] | has(-2) => false
    query!(b"[1, 2, 3]", "has(-2)",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "has(-2) should return false for arrays");
        }
    );
}

#[test]
fn test_has_valid_positive_index_returns_true() {
    // jq: [1,2,3] | has(0) => true
    query!(b"[1, 2, 3]", "has(0)",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b, "has(0) should return true");
        }
    );
}

#[test]
fn test_has_out_of_bounds_positive_index_returns_false() {
    // jq: [1,2,3] | has(5) => false
    query!(b"[1, 2, 3]", "has(5)",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "has(5) should return false for 3-element array");
        }
    );
}

#[test]
fn test_in_negative_index_returns_false() {
    // jq: -1 | in([1,2,3]) => false
    query!(b"null", "-1 | in([1, 2, 3])",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "in() with -1 should return false for arrays");
        }
    );
}

#[test]
fn test_in_valid_positive_index_returns_true() {
    // jq: 0 | in([1,2,3]) => true
    query!(b"null", "0 | in([1, 2, 3])",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b, "in() with 0 should return true");
        }
    );
}

#[test]
fn test_in_out_of_bounds_positive_index_returns_false() {
    // jq: 5 | in([1,2,3]) => false
    query!(b"null", "5 | in([1, 2, 3])",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "in() with 5 should return false for 3-element array");
        }
    );
}

// =============================================================================
// Compatibility tests - split("") edge cases
// =============================================================================

#[test]
fn test_split_empty_delimiter() {
    // jq: "hello" | split("") => ["h","e","l","l","o"]
    query!(br#""hello""#, r#"split("")"#,
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert_eq!(arr.len(), 5, "split(\"\") should produce 5 elements, got {}", arr.len());
            let expected = ["h", "e", "l", "l", "o"];
            for (i, (actual, exp)) in arr.iter().zip(expected.iter()).enumerate() {
                match actual {
                    OwnedValue::String(s) => assert_eq!(s, *exp, "element {i} mismatch"),
                    _ => panic!("expected string at index {i}"),
                }
            }
        }
    );
}

#[test]
fn test_split_empty_delimiter_on_empty_string() {
    // jq: "" | split("") => []
    query!(br#""""#, r#"split("")"#,
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert!(arr.is_empty(), "split(\"\") on empty string should return [], got {arr:?}");
        }
    );
}

#[test]
fn test_split_normal_delimiter() {
    // jq: "a,b,c" | split(",") => ["a","b","c"]
    query!(br#""a,b,c""#, r#"split(",")"#,
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert_eq!(arr.len(), 3);
        }
    );
}

#[test]
fn test_split_consecutive_delimiters() {
    // jq: "a,,b" | split(",") => ["a","","b"]
    query!(br#""a,,b""#, r#"split(",")"#,
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert_eq!(arr.len(), 3);
            match &arr[1] {
                OwnedValue::String(s) => assert_eq!(s, ""),
                _ => panic!("expected empty string at index 1"),
            }
        }
    );
}

// =============================================================================
// Compatibility tests - @csv quoting behavior
// =============================================================================

#[test]
fn test_csv_quotes_fields_with_comma() {
    // jq: ["a","b,c"] | @csv => "\"a\",\"b,c\""
    query!(br#"["a", "b,c"]"#, "@csv",
        QueryResult::Owned(OwnedValue::String(s)) => {
            // Must contain quoted "b,c"
            assert!(s.contains("\"b,c\""), "comma in field should be quoted: {s}");
        }
    );
}

#[test]
fn test_csv_escapes_quotes() {
    // jq: ["a\"b"] | @csv => "\"a\"\"b\""
    query!(br#"["a\"b"]"#, "@csv",
        QueryResult::Owned(OwnedValue::String(s)) => {
            // Quote in field should be doubled
            assert!(s.contains("\"\""), "quote in field should be doubled: {s}");
        }
    );
}

#[test]
fn test_csv_numbers_not_quoted() {
    // jq: [1,2,3] | @csv => "1,2,3"
    query!(b"[1, 2, 3]", "@csv",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "1,2,3");
        }
    );
}

// =============================================================================
// Compatibility tests - @base64d edge cases
// =============================================================================

#[test]
fn test_base64_roundtrip() {
    // jq: "hello" | @base64 | @base64d => "hello"
    query!(br#""hello""#, "@base64 | @base64d",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "hello");
        }
    );
}

#[test]
fn test_base64_encode() {
    // jq: "hello" | @base64 => "aGVsbG8="
    query!(br#""hello""#, "@base64",
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "aGVsbG8=");
        }
    );
}

// =============================================================================
// Compatibility tests - Comparison edge cases
// =============================================================================

#[test]
fn test_null_not_equal_false() {
    // jq: null == false => false
    query!(b"null", "null == false",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "null should not equal false");
        }
    );
}

#[test]
fn test_null_not_equal_zero() {
    // jq: null == 0 => false
    query!(b"null", "null == 0",
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "null should not equal 0");
        }
    );
}

#[test]
fn test_null_not_equal_empty_string() {
    // jq: null == "" => false
    query!(b"null", r#"null == """#,
        QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b, "null should not equal empty string");
        }
    );
}

// =============================================================================
// Compatibility tests - Alternative operator edge cases
// =============================================================================

#[test]
fn test_alternative_with_null() {
    // jq: null // "default" => "default"
    query!(b"null", r#"null // "default""#,
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "default");
        }
    );
}

#[test]
fn test_alternative_with_false() {
    // jq: false // "default" => "default"
    query!(b"null", r#"false // "default""#,
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "default");
        }
    );
}

#[test]
fn test_alternative_with_zero() {
    // jq: 0 // "default" => 0 (0 is truthy in jq)
    query!(b"null", r#"0 // "default""#,
        QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 0);
        }
    );
}

#[test]
fn test_alternative_with_empty_string() {
    // jq: "" // "default" => "" (empty string is truthy in jq)
    query!(b"null", r#""" // "default""#,
        QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "");
        }
    );
}
