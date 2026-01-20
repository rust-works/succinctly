//! Generic expression evaluator for jq-like queries.
//!
//! This module provides a document-agnostic evaluator that works with any type
//! implementing the `DocumentValue` trait, enabling direct evaluation of both
//! JSON and YAML without intermediate conversion.

#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use indexmap::IndexMap;

use super::document::{DocumentCursor, DocumentElements, DocumentFields, DocumentValue};
use super::eval::{eval as full_eval, EvalError, QueryResult};
use super::expr::{Builtin, Expr, Literal};
use super::value::OwnedValue;
use crate::json::JsonIndex;

/// Convert a DocumentValue to an OwnedValue.
///
/// This enables the evaluator to work with both JSON and YAML inputs.
/// Note: The order of checks is important! Check containers first, then scalars,
/// because YAML scalars may have type coercion (e.g., unquoted "true" is a bool).
pub fn to_owned<V: DocumentValue>(value: &V) -> OwnedValue {
    // Check containers first (arrays and objects have no type ambiguity)
    if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut f = fields;
        while let Some((field, rest)) = f.uncons() {
            if let Some(key) = field.key_str() {
                map.insert(key.into_owned(), to_owned(&field.value));
            }
            f = rest;
        }
        OwnedValue::Object(map)
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut elems = elements;
        while let Some((elem, rest)) = elems.uncons() {
            items.push(to_owned(&elem));
            elems = rest;
        }
        OwnedValue::Array(items)
    // Then check scalars in order of specificity
    } else if value.is_null() {
        OwnedValue::Null
    } else if let Some(b) = value.as_bool() {
        OwnedValue::Bool(b)
    } else if let Some(i) = value.as_i64() {
        OwnedValue::Int(i)
    } else if let Some(f) = value.as_f64() {
        OwnedValue::Float(f)
    } else if let Some(s) = value.as_str() {
        OwnedValue::String(s.into_owned())
    } else {
        // Covers error values and any unknown types
        OwnedValue::Null
    }
}

/// Convert a StandardJson value to an OwnedValue.
fn standard_json_to_owned<W: Clone + AsRef<[u64]>>(
    value: &crate::json::light::StandardJson<'_, W>,
) -> OwnedValue {
    use crate::json::light::StandardJson;
    match value {
        StandardJson::Null => OwnedValue::Null,
        StandardJson::Bool(b) => OwnedValue::Bool(*b),
        StandardJson::Number(n) => {
            if let Ok(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Ok(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                OwnedValue::Null
            }
        }
        StandardJson::String(s) => {
            OwnedValue::String(s.as_str().map(|c| c.to_string()).unwrap_or_default())
        }
        StandardJson::Array(elements) => {
            OwnedValue::Array((*elements).map(|e| standard_json_to_owned(&e)).collect())
        }
        StandardJson::Object(fields) => OwnedValue::Object(
            (*fields)
                .filter_map(|field| {
                    let key = match field.key() {
                        StandardJson::String(s) => s.as_str().ok()?.to_string(),
                        _ => return None,
                    };
                    let value = standard_json_to_owned(&field.value());
                    Some((key, value))
                })
                .collect(),
        ),
        StandardJson::Error(_) => OwnedValue::Null,
    }
}

/// Evaluate an expression on an OwnedValue using the full evaluator.
///
/// This converts the OwnedValue to JSON, evaluates using the full evaluator,
/// and converts the result back to GenericResult.
fn eval_on_owned<V: DocumentValue>(expr: &Expr, owned: OwnedValue) -> GenericResult<V> {
    let json_str = owned.to_json();
    let json_bytes = json_str.as_bytes();
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    match full_eval(expr, cursor) {
        QueryResult::One(v) => GenericResult::Owned(standard_json_to_owned(&v)),
        QueryResult::OneCursor(c) => GenericResult::Owned(standard_json_to_owned(&c.value())),
        QueryResult::Many(vs) => {
            GenericResult::ManyOwned(vs.iter().map(standard_json_to_owned).collect())
        }
        QueryResult::None => GenericResult::None,
        QueryResult::Error(e) => GenericResult::Error(e),
        QueryResult::Owned(v) => GenericResult::Owned(v),
        QueryResult::ManyOwned(vs) => GenericResult::ManyOwned(vs),
        QueryResult::Break(label) => GenericResult::Break(label),
    }
}

/// Evaluate an expression on multiple OwnedValues using the full evaluator.
fn eval_on_many_owned<V: DocumentValue>(
    expr: &Expr,
    owned_values: Vec<OwnedValue>,
) -> GenericResult<V> {
    let mut results = Vec::new();
    for owned in owned_values {
        match eval_on_owned::<V>(expr, owned) {
            GenericResult::One(_) => unreachable!("eval_on_owned never returns One"),
            GenericResult::OneCursor(_) => unreachable!("eval_on_owned never returns OneCursor"),
            GenericResult::Many(_) => unreachable!("eval_on_owned never returns Many"),
            GenericResult::None => {}
            GenericResult::Error(e) => return GenericResult::Error(e),
            GenericResult::Owned(o) => results.push(o),
            GenericResult::ManyOwned(os) => results.extend(os),
            GenericResult::Break(label) => return GenericResult::Break(label),
        }
    }
    if results.is_empty() {
        GenericResult::None
    } else {
        GenericResult::ManyOwned(results)
    }
}

/// Result of evaluating a generic jq expression.
#[derive(Debug)]
pub enum GenericResult<V: DocumentValue> {
    /// Single value result (reference to original document).
    One(V),

    /// Single cursor result (for efficient raw output).
    OneCursor(V::Cursor),

    /// Multiple values (from iteration).
    Many(Vec<V>),

    /// No result (optional that was missing).
    None,

    /// Error during evaluation.
    Error(EvalError),

    /// Single owned value (from construction/computation).
    Owned(OwnedValue),

    /// Multiple owned values.
    ManyOwned(Vec<OwnedValue>),

    /// Break from a labeled scope.
    Break(String),
}

impl<V: DocumentValue> GenericResult<V> {
    /// Convert to OwnedValue for output.
    pub fn into_owned(self) -> Option<OwnedValue> {
        match self {
            GenericResult::One(v) => Some(to_owned(&v)),
            GenericResult::OneCursor(c) => Some(to_owned(&c.value())),
            GenericResult::Many(vs) => Some(OwnedValue::Array(vs.iter().map(to_owned).collect())),
            GenericResult::None => None,
            GenericResult::Error(_) => None,
            GenericResult::Owned(o) => Some(o),
            GenericResult::ManyOwned(os) => Some(OwnedValue::Array(os)),
            GenericResult::Break(_) => None,
        }
    }

    /// Collect all results into a Vec of OwnedValues.
    pub fn collect_owned(self) -> Vec<OwnedValue> {
        match self {
            GenericResult::One(v) => vec![to_owned(&v)],
            GenericResult::OneCursor(c) => vec![to_owned(&c.value())],
            GenericResult::Many(vs) => vs.iter().map(to_owned).collect(),
            GenericResult::None => vec![],
            GenericResult::Error(_) => vec![],
            GenericResult::Owned(o) => vec![o],
            GenericResult::ManyOwned(os) => os,
            GenericResult::Break(_) => vec![],
        }
    }

    /// Check if this is an error.
    pub fn is_error(&self) -> bool {
        matches!(self, GenericResult::Error(_))
    }

    /// Get the error if this is an error result.
    pub fn error(&self) -> Option<&EvalError> {
        match self {
            GenericResult::Error(e) => Some(e),
            _ => None,
        }
    }
}

/// Evaluate an expression against a document value.
///
/// This is the main entry point for generic evaluation.
pub fn eval<V: DocumentValue>(expr: &Expr, value: V) -> GenericResult<V> {
    eval_single(expr, value, false, None)
}

/// Evaluate an expression against a cursor.
///
/// This entry point preserves cursor position metadata, enabling
/// `line` and `column` builtins to return actual values.
pub fn eval_with_cursor<C: DocumentCursor>(expr: &Expr, cursor: C) -> GenericResult<C::Value> {
    eval_single(expr, cursor.value(), false, Some(cursor))
}

/// Evaluate a single expression against a value with optional cursor context.
fn eval_single<V: DocumentValue>(
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    match expr {
        Expr::Identity => GenericResult::One(value),

        Expr::Field(name) => {
            if let Some(fields) = value.as_object() {
                match fields.find(name) {
                    Some(v) => GenericResult::One(v),
                    None if optional => GenericResult::None,
                    None => {
                        GenericResult::Error(EvalError::new(format!("field '{}' not found", name)))
                    }
                }
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "expected object, got {}",
                    value.type_name()
                )))
            }
        }

        Expr::Index(idx) => {
            if let Some(elements) = value.as_array() {
                let len = elements.len();
                let actual_idx = if *idx < 0 {
                    (len as i64 + idx) as usize
                } else {
                    *idx as usize
                };
                match elements.get(actual_idx) {
                    Some(v) => GenericResult::One(v),
                    None if optional => GenericResult::None,
                    None => GenericResult::Error(EvalError::new(format!(
                        "index {} out of bounds (length {})",
                        idx, len
                    ))),
                }
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "expected array, got {}",
                    value.type_name()
                )))
            }
        }

        Expr::Iterate => {
            if let Some(elements) = value.as_array() {
                let values = elements.collect_values();
                if values.is_empty() {
                    GenericResult::None
                } else {
                    GenericResult::Many(values)
                }
            } else if let Some(fields) = value.as_object() {
                let mut values = Vec::new();
                let mut f = fields;
                while let Some((field, rest)) = f.uncons() {
                    values.push(field.value);
                    f = rest;
                }
                if values.is_empty() {
                    GenericResult::None
                } else {
                    GenericResult::Many(values)
                }
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "cannot iterate over {}",
                    value.type_name()
                )))
            }
        }

        Expr::Optional(inner) => eval_single(inner, value, true, cursor),

        Expr::Pipe(exprs) => {
            if exprs.is_empty() {
                return GenericResult::One(value);
            }

            let mut current = eval_single(&exprs[0], value, optional, cursor);

            for expr in &exprs[1..] {
                current = match current {
                    GenericResult::One(v) => eval_single(expr, v, optional, None),
                    GenericResult::OneCursor(c) => eval_single(expr, c.value(), optional, Some(c)),
                    GenericResult::Many(vs) => {
                        let mut results = Vec::new();
                        for v in vs {
                            match eval_single(expr, v, optional, None) {
                                GenericResult::One(r) => results.push(to_owned(&r)),
                                GenericResult::OneCursor(c) => results.push(to_owned(&c.value())),
                                GenericResult::Many(rs) => {
                                    results.extend(rs.iter().map(to_owned));
                                }
                                GenericResult::None => {}
                                GenericResult::Error(e) => return GenericResult::Error(e),
                                GenericResult::Owned(o) => results.push(o),
                                GenericResult::ManyOwned(os) => results.extend(os),
                                GenericResult::Break(label) => return GenericResult::Break(label),
                            }
                        }
                        if results.is_empty() {
                            GenericResult::None
                        } else {
                            GenericResult::ManyOwned(results)
                        }
                    }
                    GenericResult::None => GenericResult::None,
                    GenericResult::Error(e) => return GenericResult::Error(e),
                    GenericResult::Owned(o) => {
                        // Continue piping from owned value via JSON round-trip
                        eval_on_owned(expr, o)
                    }
                    GenericResult::ManyOwned(os) => {
                        // Continue piping from owned values via JSON round-trip
                        eval_on_many_owned(expr, os)
                    }
                    GenericResult::Break(label) => return GenericResult::Break(label),
                };
            }

            current
        }

        Expr::Literal(lit) => match lit {
            Literal::Null => GenericResult::Owned(OwnedValue::Null),
            Literal::Bool(b) => GenericResult::Owned(OwnedValue::Bool(*b)),
            Literal::Int(i) => GenericResult::Owned(OwnedValue::Int(*i)),
            Literal::Float(f) => GenericResult::Owned(OwnedValue::Float(*f)),
            Literal::String(s) => GenericResult::Owned(OwnedValue::String(s.clone())),
        },

        Expr::Builtin(builtin) => eval_builtin(builtin, value, optional, cursor),

        // Fall back to the full evaluator for complex expressions
        _ => {
            // Convert to OwnedValue, then to JSON, then evaluate with full evaluator
            let owned = to_owned(&value);
            let json_str = owned.to_json();
            let json_bytes = json_str.as_bytes();
            let index = JsonIndex::build(json_bytes);
            let cursor = index.root(json_bytes);

            // Evaluate using the full evaluator
            match full_eval(expr, cursor) {
                QueryResult::One(v) => {
                    // Convert StandardJson back to OwnedValue
                    GenericResult::Owned(standard_json_to_owned(&v))
                }
                QueryResult::OneCursor(c) => {
                    GenericResult::Owned(standard_json_to_owned(&c.value()))
                }
                QueryResult::Many(vs) => {
                    GenericResult::ManyOwned(vs.iter().map(standard_json_to_owned).collect())
                }
                QueryResult::None => GenericResult::None,
                QueryResult::Error(e) => GenericResult::Error(e),
                QueryResult::Owned(v) => GenericResult::Owned(v),
                QueryResult::ManyOwned(vs) => GenericResult::ManyOwned(vs),
                QueryResult::Break(label) => GenericResult::Break(label),
            }
        }
    }
}

/// Evaluate a builtin function.
fn eval_builtin<V: DocumentValue>(
    builtin: &Builtin,
    value: V,
    _optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    match builtin {
        Builtin::Line => {
            let line = cursor.map(|c| c.line()).unwrap_or(0);
            GenericResult::Owned(OwnedValue::Int(line as i64))
        }

        Builtin::Column => {
            let column = cursor.map(|c| c.column()).unwrap_or(0);
            GenericResult::Owned(OwnedValue::Int(column as i64))
        }
        Builtin::Type => {
            let type_name = value.type_name();
            GenericResult::Owned(OwnedValue::String(type_name.to_string()))
        }

        Builtin::Length => {
            if value.is_null() {
                GenericResult::Owned(OwnedValue::Int(0))
            } else if let Some(s) = value.as_str() {
                GenericResult::Owned(OwnedValue::Int(s.chars().count() as i64))
            } else if let Some(elements) = value.as_array() {
                GenericResult::Owned(OwnedValue::Int(elements.len() as i64))
            } else if let Some(fields) = value.as_object() {
                GenericResult::Owned(OwnedValue::Int(fields.len() as i64))
            } else if let Some(i) = value.as_i64() {
                GenericResult::Owned(OwnedValue::Int(i.abs()))
            } else if let Some(f) = value.as_f64() {
                GenericResult::Owned(OwnedValue::Float(f.abs()))
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "{} has no length",
                    value.type_name()
                )))
            }
        }

        Builtin::Keys | Builtin::KeysUnsorted => {
            if let Some(fields) = value.as_object() {
                let keys = fields.keys();
                let owned_keys: Vec<OwnedValue> =
                    keys.into_iter().map(OwnedValue::String).collect();
                GenericResult::Owned(OwnedValue::Array(owned_keys))
            } else if let Some(elements) = value.as_array() {
                let len = elements.len();
                let indices: Vec<OwnedValue> =
                    (0..len).map(|i| OwnedValue::Int(i as i64)).collect();
                GenericResult::Owned(OwnedValue::Array(indices))
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "keys requires object or array, got {}",
                    value.type_name()
                )))
            }
        }

        Builtin::Values => {
            if let Some(elements) = value.as_array() {
                let values = elements.collect_values();
                GenericResult::ManyOwned(values.iter().map(to_owned).collect())
            } else if let Some(fields) = value.as_object() {
                let mut values = Vec::new();
                let mut f = fields;
                while let Some((field, rest)) = f.uncons() {
                    values.push(to_owned(&field.value));
                    f = rest;
                }
                GenericResult::ManyOwned(values)
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "values requires object or array, got {}",
                    value.type_name()
                )))
            }
        }

        Builtin::IsNull => GenericResult::Owned(OwnedValue::Bool(value.is_null())),

        Builtin::IsBoolean => GenericResult::Owned(OwnedValue::Bool(value.is_bool())),

        Builtin::IsNumber => GenericResult::Owned(OwnedValue::Bool(value.is_number())),

        Builtin::IsString => GenericResult::Owned(OwnedValue::Bool(value.is_string())),

        Builtin::IsArray => GenericResult::Owned(OwnedValue::Bool(value.is_array())),

        Builtin::IsObject => GenericResult::Owned(OwnedValue::Bool(value.is_object())),

        Builtin::Iterables => {
            // Returns input if iterable, empty otherwise
            if value.is_iterable() {
                GenericResult::One(value)
            } else {
                GenericResult::None
            }
        }

        Builtin::Scalars => {
            // Returns input if scalar, empty otherwise
            if !value.is_iterable() {
                GenericResult::One(value)
            } else {
                GenericResult::None
            }
        }

        Builtin::First => {
            if let Some(elements) = value.as_array() {
                match elements.get(0) {
                    Some(v) => GenericResult::One(v),
                    None => GenericResult::Error(EvalError::new("empty array")),
                }
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "first requires array, got {}",
                    value.type_name()
                )))
            }
        }

        Builtin::Last => {
            if let Some(elements) = value.as_array() {
                let len = elements.len();
                if len == 0 {
                    GenericResult::Error(EvalError::new("empty array"))
                } else {
                    match elements.get(len - 1) {
                        Some(v) => GenericResult::One(v),
                        None => GenericResult::Error(EvalError::new("empty array")),
                    }
                }
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "last requires array, got {}",
                    value.type_name()
                )))
            }
        }

        Builtin::Reverse => {
            if let Some(elements) = value.as_array() {
                let values: Vec<OwnedValue> = elements
                    .collect_values()
                    .iter()
                    .rev()
                    .map(to_owned)
                    .collect();
                GenericResult::Owned(OwnedValue::Array(values))
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "reverse requires array, got {}",
                    value.type_name()
                )))
            }
        }

        Builtin::Empty => GenericResult::None,

        Builtin::ToString => {
            let owned = to_owned(&value);
            let s = match &owned {
                OwnedValue::Null => "null".to_string(),
                OwnedValue::Bool(b) => b.to_string(),
                OwnedValue::Int(i) => i.to_string(),
                OwnedValue::Float(f) => f.to_string(),
                OwnedValue::String(s) => s.clone(),
                OwnedValue::Array(_) | OwnedValue::Object(_) => owned.to_json(),
            };
            GenericResult::Owned(OwnedValue::String(s))
        }

        Builtin::ToNumber => {
            if let Some(i) = value.as_i64() {
                GenericResult::Owned(OwnedValue::Int(i))
            } else if let Some(f) = value.as_f64() {
                GenericResult::Owned(OwnedValue::Float(f))
            } else if let Some(s) = value.as_str() {
                if let Ok(i) = s.parse::<i64>() {
                    GenericResult::Owned(OwnedValue::Int(i))
                } else if let Ok(f) = s.parse::<f64>() {
                    GenericResult::Owned(OwnedValue::Float(f))
                } else {
                    GenericResult::Error(EvalError::new(format!(
                        "cannot convert '{}' to number",
                        s
                    )))
                }
            } else {
                GenericResult::Error(EvalError::new(format!(
                    "cannot convert {} to number",
                    value.type_name()
                )))
            }
        }

        // For other builtins, fall back to full evaluator via JSON
        _ => {
            let owned = to_owned(&value);
            eval_on_owned(&Expr::Builtin(builtin.clone()), owned)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::JsonIndex;

    #[test]
    fn test_generic_identity() {
        let json = br#"{"name": "Alice", "age": 30}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Identity, value);
        let owned = result.into_owned().unwrap();

        match owned {
            OwnedValue::Object(map) => {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Alice".to_string()))
                );
                assert_eq!(map.get("age"), Some(&OwnedValue::Int(30)));
            }
            _ => panic!("Expected object"),
        }
    }

    #[test]
    fn test_generic_field_access() {
        let json = br#"{"name": "Alice", "age": 30}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Field("name".to_string()), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_generic_array_index() {
        let json = br#"[1, 2, 3]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Index(1), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::Int(2));
    }

    #[test]
    fn test_generic_iterate() {
        let json = br#"[1, 2, 3]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Iterate, value);
        let owned = result.collect_owned();

        assert_eq!(
            owned,
            vec![OwnedValue::Int(1), OwnedValue::Int(2), OwnedValue::Int(3)]
        );
    }

    #[test]
    fn test_generic_type() {
        let json = br#"{"name": "Alice"}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::Type), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("object".to_string()));
    }

    #[test]
    fn test_generic_length() {
        let json = br#"[1, 2, 3, 4, 5]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::Length), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::Int(5));
    }

    #[test]
    fn test_generic_keys() {
        let json = br#"{"b": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::KeysUnsorted), value);
        let owned = result.into_owned().unwrap();

        match owned {
            OwnedValue::Array(keys) => {
                assert_eq!(keys.len(), 2);
                // Keys are in document order
                assert_eq!(keys[0], OwnedValue::String("b".to_string()));
                assert_eq!(keys[1], OwnedValue::String("a".to_string()));
            }
            _ => panic!("Expected array"),
        }
    }

    #[test]
    fn test_generic_pipe() {
        let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        // .users | .[0] | .name
        let expr = Expr::Pipe(vec![
            Expr::Field("users".to_string()),
            Expr::Index(0),
            Expr::Field("name".to_string()),
        ]);

        let result = eval(&expr, value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("Alice".to_string()));
    }

    // ========== YAML Tests ==========

    #[test]
    fn test_yaml_generic_identity() {
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the first child (the actual mapping) since YAML has a document wrapper
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        let result = eval(&Expr::Identity, value);
        let owned = result.into_owned().unwrap();

        match owned {
            OwnedValue::Object(map) => {
                assert_eq!(
                    map.get("name"),
                    Some(&OwnedValue::String("Alice".to_string()))
                );
                assert_eq!(map.get("age"), Some(&OwnedValue::Int(30)));
            }
            _ => panic!("Expected object, got {:?}", owned),
        }
    }

    #[test]
    fn test_yaml_generic_field_access() {
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the actual mapping
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        let result = eval(&Expr::Field("name".to_string()), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_yaml_generic_array() {
        use crate::yaml::YamlIndex;

        let yaml = b"- 1\n- 2\n- 3";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the actual sequence
        let seq_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = seq_cursor.value();

        let result = eval(&Expr::Index(1), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::Int(2));
    }

    #[test]
    fn test_yaml_generic_line_column() {
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the actual mapping
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");

        // Mapping should be at line 1
        assert_eq!(mapping_cursor.line(), 1);
    }

    #[test]
    fn test_yaml_line_builtin_with_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the actual mapping
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");

        // Use eval_with_cursor to preserve position metadata
        let result = eval_with_cursor(&Expr::Builtin(Builtin::Line), mapping_cursor);
        let owned = result.into_owned().unwrap();

        // Mapping starts at line 1
        assert_eq!(owned, OwnedValue::Int(1));
    }

    #[test]
    fn test_yaml_column_builtin_with_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the actual mapping
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");

        // Use eval_with_cursor to preserve position metadata
        let result = eval_with_cursor(&Expr::Builtin(Builtin::Column), mapping_cursor);
        let owned = result.into_owned().unwrap();

        // Mapping starts at column 1
        assert_eq!(owned, OwnedValue::Int(1));
    }

    #[test]
    fn test_yaml_line_without_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        // Using eval (not eval_with_cursor) loses position metadata
        let result = eval(&Expr::Builtin(Builtin::Line), value);
        let owned = result.into_owned().unwrap();

        // Without cursor, line returns 0
        assert_eq!(owned, OwnedValue::Int(0));
    }

    #[test]
    fn test_yaml_generic_pipe() {
        use crate::yaml::YamlIndex;

        let yaml = b"users:\n  - name: Alice\n  - name: Bob";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the actual mapping
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        // .users | .[0] | .name
        let expr = Expr::Pipe(vec![
            Expr::Field("users".to_string()),
            Expr::Index(0),
            Expr::Field("name".to_string()),
        ]);

        let result = eval(&expr, value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("Alice".to_string()));
    }
}
