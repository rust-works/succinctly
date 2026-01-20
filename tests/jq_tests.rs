//! Integration tests for jq query functionality.

use succinctly::jq::{eval, eval_lenient, parse, QueryResult};
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
        match eval(&expr, cursor) {
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
        let results = eval_lenient(&expr, cursor);
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
        let results = eval_lenient(&expr, cursor);
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
    query!(br#"[1, 2, 3]"#, ".", QueryResult::OneCursor(_) => {});
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
fn test_field_missing_error() {
    query!(br#"{"a": 1}"#, ".missing",
        QueryResult::Error(e) => {
            assert!(e.message.contains("not found"), "expected 'not found' error");
        }
    );
}

#[test]
fn test_field_on_non_object_error() {
    query!(br#"[1, 2, 3]"#, ".foo",
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
    query!(br#"[10, 20, 30]"#, ".[0]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 10);
        }
    );
}

#[test]
fn test_index_last() {
    query!(br#"[10, 20, 30]"#, ".[2]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 30);
        }
    );
}

#[test]
fn test_index_negative() {
    query!(br#"[10, 20, 30, 40]"#, ".[-1]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 40);
        }
    );
    query!(br#"[10, 20, 30, 40]"#, ".[-2]",
        QueryResult::One(StandardJson::Number(n)) => {
            assert_eq!(n.as_i64().unwrap(), 30);
        }
    );
}

#[test]
fn test_index_out_of_bounds() {
    query!(br#"[1, 2, 3]"#, ".[10]",
        QueryResult::Error(e) => {
            assert!(e.message.contains("out of bounds"));
        }
    );
}

#[test]
fn test_index_negative_out_of_bounds() {
    query!(br#"[1, 2, 3]"#, ".[-10]",
        QueryResult::Error(e) => {
            assert!(e.message.contains("out of bounds"));
        }
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
    query!(br#"[1, 2, 3, 4, 5]"#, ".[]",
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
    query!(br#"[]"#, ".[]",
        QueryResult::Many(values) => {
            assert!(values.is_empty());
        }
    );
}

#[test]
fn test_iterate_empty_object() {
    query!(br#"{}"#, ".[]",
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
    query!(br#"[0, 1, 2, 3, 4, 5]"#, ".[1:4]",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 3);
        }
    );
}

#[test]
fn test_slice_from_start() {
    query!(br#"[0, 1, 2, 3, 4]"#, ".[:2]",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 2);
        }
    );
}

#[test]
fn test_slice_to_end() {
    query!(br#"[0, 1, 2, 3, 4]"#, ".[3:]",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 2); // indices 3, 4
        }
    );
}

#[test]
fn test_slice_negative_indices() {
    query!(br#"[0, 1, 2, 3, 4, 5]"#, ".[-3:-1]",
        QueryResult::Many(values) => {
            assert_eq!(values.len(), 2); // indices 3, 4 (elements 3, 4)
        }
    );
}

#[test]
fn test_slice_empty_result() {
    query!(br#"[0, 1, 2]"#, ".[5:10]",
        QueryResult::Many(values) => {
            assert!(values.is_empty());
        }
    );
}

// =============================================================================
// Optional tests
// =============================================================================

#[test]
fn test_optional_field_missing() {
    query!(br#"{"a": 1}"#, ".missing?",
        QueryResult::None => {}
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
fn test_optional_index_out_of_bounds() {
    query!(br#"[1, 2, 3]"#, ".[10]?",
        QueryResult::None => {}
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
fn test_lenient_error_returns_empty() {
    query_lenient!(br#"{"name": "test"}"#, ".missing", is_empty);
}

#[test]
fn test_lenient_none_returns_empty() {
    query_lenient!(br#"{"name": "test"}"#, ".missing?", is_empty);
}

#[test]
fn test_lenient_many() {
    query_lenient!(br#"[1, 2, 3]"#, ".[]", len == 3);
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
    query!(br#"[[[[1]]]]"#, ".[0][0][0][0]",
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
            assert!(ts > jan_2024, "timestamp {} should be after 2024-01-01", ts);
            assert!(ts < jan_2100, "timestamp {} should be before 2100-01-01", ts);
        }
    );
}

#[test]
fn test_now_ignores_input() {
    // now ignores its input
    query!(br#"{"foo": "bar"}"#, "now",
        QueryResult::Owned(succinctly::jq::OwnedValue::Float(ts)) => {
            let jan_2024 = 1704067200.0;
            assert!(ts > jan_2024, "timestamp {} should be after 2024-01-01", ts);
        }
    );
}
