//! Expression evaluator for jq-like queries.
//!
//! Evaluates expressions against JSON using the cursor-based navigation API.

#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::String;
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use crate::json::light::{JsonCursor, JsonElements, JsonFields, StandardJson};

use super::expr::Expr;

/// Result of evaluating a jq expression.
#[derive(Debug)]
pub enum QueryResult<'a, W = Vec<u64>> {
    /// Single value result.
    One(StandardJson<'a, W>),

    /// Multiple values (from iteration).
    Many(Vec<StandardJson<'a, W>>),

    /// No result (optional that was missing).
    None,

    /// Error during evaluation.
    Error(EvalError),
}

/// Error that occurs during evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalError {
    pub message: String,
}

impl EvalError {
    fn new(message: impl Into<String>) -> Self {
        EvalError {
            message: message.into(),
        }
    }

    fn type_error(expected: &str, got: &str) -> Self {
        EvalError::new(format!("expected {}, got {}", expected, got))
    }

    fn field_not_found(name: &str) -> Self {
        EvalError::new(format!("field '{}' not found", name))
    }

    fn index_out_of_bounds(index: i64, len: usize) -> Self {
        EvalError::new(format!("index {} out of bounds (length {})", index, len))
    }
}

impl core::fmt::Display for EvalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Get the type name of a JSON value for error messages.
fn type_name<W>(value: &StandardJson<'_, W>) -> &'static str {
    match value {
        StandardJson::Object(_) => "object",
        StandardJson::Array(_) => "array",
        StandardJson::String(_) => "string",
        StandardJson::Number(_) => "number",
        StandardJson::Bool(_) => "boolean",
        StandardJson::Null => "null",
        StandardJson::Error(_) => "error",
    }
}

/// Evaluate a single expression against a JSON value.
fn eval_single<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match expr {
        Expr::Identity => QueryResult::One(value),

        Expr::Field(name) => match value {
            StandardJson::Object(fields) => match find_field(fields, name) {
                Some(v) => QueryResult::One(v),
                None if optional => QueryResult::None,
                None => QueryResult::Error(EvalError::field_not_found(name)),
            },
            _ if optional => QueryResult::None,
            _ => QueryResult::Error(EvalError::type_error("object", type_name(&value))),
        },

        Expr::Index(idx) => match value {
            StandardJson::Array(elements) => {
                match get_element_at_index(elements, *idx) {
                    Some(v) => QueryResult::One(v),
                    None if optional => QueryResult::None,
                    None => {
                        // Count elements to give accurate error
                        let len = count_elements(elements);
                        QueryResult::Error(EvalError::index_out_of_bounds(*idx, len))
                    }
                }
            }
            _ if optional => QueryResult::None,
            _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
        },

        Expr::Slice { start, end } => match value {
            StandardJson::Array(elements) => {
                let results = slice_elements(elements, *start, *end);
                QueryResult::Many(results)
            }
            _ if optional => QueryResult::None,
            _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
        },

        Expr::Iterate => match value {
            StandardJson::Array(elements) => {
                let results: Vec<_> = elements.collect();
                QueryResult::Many(results)
            }
            StandardJson::Object(fields) => {
                let results: Vec<_> = fields.map(|f| f.value()).collect();
                QueryResult::Many(results)
            }
            _ if optional => QueryResult::None,
            _ => QueryResult::Error(EvalError::type_error("array or object", type_name(&value))),
        },

        Expr::Optional(inner) => eval_single(inner, value, true),

        Expr::Pipe(exprs) => eval_pipe(exprs, value, optional),
    }
}

/// Evaluate a pipe (chain) of expressions.
fn eval_pipe<'a, W: Clone + AsRef<[u64]>>(
    exprs: &[Expr],
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    if exprs.is_empty() {
        return QueryResult::One(value);
    }

    let (first, rest) = exprs.split_first().unwrap();

    // Evaluate first expression
    let result = eval_single(first, value, optional);

    if rest.is_empty() {
        return result;
    }

    // Apply remaining expressions to the result
    match result {
        QueryResult::One(v) => eval_pipe(rest, v, optional),
        QueryResult::Many(values) => {
            let mut all_results = Vec::new();
            for v in values {
                match eval_pipe(rest, v, optional) {
                    QueryResult::One(r) => all_results.push(r),
                    QueryResult::Many(rs) => all_results.extend(rs),
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                }
            }
            QueryResult::Many(all_results)
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
    }
}

/// Find a field in an object by name.
fn find_field<'a, W: Clone + AsRef<[u64]>>(
    fields: JsonFields<'a, W>,
    name: &str,
) -> Option<StandardJson<'a, W>> {
    fields.find(name)
}

/// Get element at index (supports negative indexing).
fn get_element_at_index<'a, W: Clone + AsRef<[u64]>>(
    elements: JsonElements<'a, W>,
    idx: i64,
) -> Option<StandardJson<'a, W>> {
    if idx >= 0 {
        elements.get(idx as usize)
    } else {
        // Negative index: count from end
        let len = count_elements(elements);
        let positive_idx = len as i64 + idx;
        if positive_idx >= 0 {
            elements.get(positive_idx as usize)
        } else {
            None
        }
    }
}

/// Count elements in an array (consumes the iterator).
fn count_elements<W: Clone + AsRef<[u64]>>(elements: JsonElements<'_, W>) -> usize {
    elements.count()
}

/// Slice elements from an array.
fn slice_elements<'a, W: Clone + AsRef<[u64]>>(
    elements: JsonElements<'a, W>,
    start: Option<i64>,
    end: Option<i64>,
) -> Vec<StandardJson<'a, W>> {
    let all: Vec<_> = elements.collect();
    let len = all.len();

    // Resolve negative indices
    let resolve_idx = |idx: i64, _default: usize| -> usize {
        if idx >= 0 {
            (idx as usize).min(len)
        } else {
            let pos = len as i64 + idx;
            if pos < 0 { 0 } else { pos as usize }
        }
    };

    let start_idx = start.map(|i| resolve_idx(i, 0)).unwrap_or(0);
    let end_idx = end.map(|i| resolve_idx(i, len)).unwrap_or(len);

    if start_idx >= end_idx || start_idx >= len {
        return Vec::new();
    }

    all.into_iter()
        .skip(start_idx)
        .take(end_idx - start_idx)
        .collect()
}

/// Evaluate a jq expression against a JSON cursor.
///
/// # Examples
///
/// ```ignore
/// use succinctly::jq::{parse, eval};
/// use succinctly::json::JsonIndex;
///
/// let json = br#"{"name": "Alice", "age": 30}"#;
/// let index = JsonIndex::build(json);
/// let cursor = index.root(json);
///
/// let expr = parse(".name").unwrap();
/// let result = eval(&expr, cursor);
/// ```
pub fn eval<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    cursor: JsonCursor<'a, W>,
) -> QueryResult<'a, W> {
    eval_single(expr, cursor.value(), false)
}

/// Evaluate a jq expression, returning only successfully matched values.
/// Errors and None results are filtered out.
pub fn eval_lenient<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    cursor: JsonCursor<'a, W>,
) -> Vec<StandardJson<'a, W>> {
    match eval(expr, cursor) {
        QueryResult::One(v) => vec![v],
        QueryResult::Many(vs) => vs,
        QueryResult::None => Vec::new(),
        QueryResult::Error(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::parse;
    use crate::json::JsonIndex;

    /// Helper macro to run a query and match the result.
    macro_rules! query {
        ($json:expr, $expr:expr, $pattern:pat $(if $guard:expr)? => $body:expr) => {{
            let json_bytes: &[u8] = $json;
            let index = JsonIndex::build(json_bytes);
            let cursor = index.root(json_bytes);
            let expr = parse($expr).unwrap();
            match eval(&expr, cursor) {
                $pattern $(if $guard)? => $body,
                other => panic!("unexpected result: {:?}", other),
            }
        }};
    }

    #[test]
    fn test_identity() {
        query!(br#"{"foo": 1}"#, ".", QueryResult::One(StandardJson::Object(_)) => {});
    }

    #[test]
    fn test_field_access() {
        query!(br#"{"name": "Alice", "age": 30}"#, ".name",
            QueryResult::One(StandardJson::String(s)) => {
                assert_eq!(s.as_str().unwrap().as_ref(), "Alice");
            }
        );

        query!(br#"{"name": "Alice", "age": 30}"#, ".age",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 30);
            }
        );
    }

    #[test]
    fn test_missing_field() {
        query!(br#"{"name": "Alice"}"#, ".missing",
            QueryResult::Error(e) => {
                assert!(e.message.contains("not found"));
            }
        );

        // Optional should return None
        query!(br#"{"name": "Alice"}"#, ".missing?",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_array_index() {
        query!(br#"[10, 20, 30]"#, ".[0]",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 10);
            }
        );

        query!(br#"[10, 20, 30]"#, ".[2]",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 30);
            }
        );

        // Negative index
        query!(br#"[10, 20, 30]"#, ".[-1]",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 30);
            }
        );
    }

    #[test]
    fn test_iterate() {
        query!(br#"[1, 2, 3]"#, ".[]",
            QueryResult::Many(values) => {
                assert_eq!(values.len(), 3);
            }
        );
    }

    #[test]
    fn test_chained() {
        query!(br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#, ".users[0].name",
            QueryResult::One(StandardJson::String(s)) => {
                assert_eq!(s.as_str().unwrap().as_ref(), "Alice");
            }
        );

        // Iterate then access field
        query!(br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#, ".users[].name",
            QueryResult::Many(values) => {
                assert_eq!(values.len(), 2);
                match &values[0] {
                    StandardJson::String(s) => {
                        assert_eq!(s.as_str().unwrap().as_ref(), "Alice");
                    }
                    other => panic!("unexpected: {:?}", other),
                }
            }
        );
    }

    #[test]
    fn test_slice() {
        query!(br#"[0, 1, 2, 3, 4, 5]"#, ".[1:4]",
            QueryResult::Many(values) => {
                assert_eq!(values.len(), 3);
            }
        );

        query!(br#"[0, 1, 2, 3, 4, 5]"#, ".[2:]",
            QueryResult::Many(values) => {
                assert_eq!(values.len(), 4); // 2, 3, 4, 5
            }
        );

        query!(br#"[0, 1, 2, 3, 4, 5]"#, ".[:2]",
            QueryResult::Many(values) => {
                assert_eq!(values.len(), 2); // 0, 1
            }
        );
    }
}
