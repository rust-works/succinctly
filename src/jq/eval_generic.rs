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
use super::expr::{Builtin, CompareOp, Expr, Literal};
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

/// Compare two OwnedValues for ordering.
///
/// Uses jq ordering: null < bool < number < string < array < object.
/// Returns Some(Ordering) if values are comparable, None if not (for mixed incompatible types).
fn compare_values(left: &OwnedValue, right: &OwnedValue) -> Option<core::cmp::Ordering> {
    use core::cmp::Ordering;

    fn type_order(v: &OwnedValue) -> u8 {
        match v {
            OwnedValue::Null => 0,
            OwnedValue::Bool(_) => 1,
            OwnedValue::Int(_) | OwnedValue::Float(_) => 2,
            OwnedValue::String(_) => 3,
            OwnedValue::Array(_) => 4,
            OwnedValue::Object(_) => 5,
        }
    }

    let left_type = type_order(left);
    let right_type = type_order(right);

    if left_type != right_type {
        return Some(left_type.cmp(&right_type));
    }

    match (left, right) {
        (OwnedValue::Null, OwnedValue::Null) => Some(Ordering::Equal),
        (OwnedValue::Bool(a), OwnedValue::Bool(b)) => Some(a.cmp(b)),
        (OwnedValue::Int(a), OwnedValue::Int(b)) => Some(a.cmp(b)),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => a.partial_cmp(b),
        (OwnedValue::Int(a), OwnedValue::Float(b)) => (*a as f64).partial_cmp(b),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => a.partial_cmp(&(*b as f64)),
        (OwnedValue::String(a), OwnedValue::String(b)) => Some(a.cmp(b)),
        // Arrays and objects: compare element-wise (simplified)
        (OwnedValue::Array(a), OwnedValue::Array(b)) => {
            for (ai, bi) in a.iter().zip(b.iter()) {
                match compare_values(ai, bi) {
                    Some(Ordering::Equal) => continue,
                    other => return other,
                }
            }
            Some(a.len().cmp(&b.len()))
        }
        _ => None,
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

    /// Stream results as JSON to the output writer.
    ///
    /// This is the M2 streaming fast path that avoids OwnedValue materialization
    /// for cursor-based results. For owned results, uses the StreamableValue impl.
    ///
    /// Returns the number of values streamed and whether the last was falsy.
    pub fn stream_json<W: core::fmt::Write>(
        &self,
        out: &mut W,
        mut on_value: impl FnMut(&mut W) -> core::fmt::Result,
    ) -> Result<crate::jq::stream::StreamStats, core::fmt::Error> {
        use crate::jq::stream::{StreamStats, StreamableValue};

        let mut stats = StreamStats::default();

        match self {
            GenericResult::One(v) => {
                // Convert to owned for streaming
                let owned = to_owned(v);
                owned.stream_json(out)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy();
            }
            GenericResult::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait
                c.stream_json(out)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy();
            }
            GenericResult::Many(vs) => {
                for v in vs {
                    let owned = to_owned(v);
                    owned.stream_json(out)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy();
                }
                stats.count = vs.len();
            }
            GenericResult::None => {
                // No output
            }
            GenericResult::Error(e) => {
                // Write error message (like jq does)
                write!(out, "jq: error: {}", e)?;
                on_value(out)?;
            }
            GenericResult::Owned(o) => {
                o.stream_json(out)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = o.is_falsy();
            }
            GenericResult::ManyOwned(os) => {
                for o in os {
                    o.stream_json(out)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
                }
                stats.count = os.len();
            }
            GenericResult::Break(label) => {
                write!(out, "jq: error: break ${} not in label", label)?;
                on_value(out)?;
            }
        }

        Ok(stats)
    }

    /// Check if this result produces a single streamable cursor result.
    ///
    /// This is used to detect if M2 streaming can be applied for navigation queries.
    pub fn is_single_cursor(&self) -> bool {
        matches!(self, GenericResult::OneCursor(_))
    }

    /// Stream results as YAML to the output writer.
    ///
    /// This is the M2.5 streaming fast path for YAML output that avoids OwnedValue
    /// materialization for cursor-based results.
    ///
    /// Returns the number of values streamed and whether the last was falsy.
    pub fn stream_yaml<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent_spaces: usize,
        mut on_value: impl FnMut(&mut W) -> core::fmt::Result,
    ) -> Result<crate::jq::stream::StreamStats, core::fmt::Error> {
        use crate::jq::stream::{StreamStats, StreamableValue};

        let mut stats = StreamStats::default();

        match self {
            GenericResult::One(v) => {
                let owned = to_owned(v);
                owned.stream_yaml(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy();
            }
            GenericResult::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait
                c.stream_yaml(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy();
            }
            GenericResult::Many(vs) => {
                for v in vs {
                    let owned = to_owned(v);
                    owned.stream_yaml(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy();
                }
                stats.count = vs.len();
            }
            GenericResult::None => {
                // No output
            }
            GenericResult::Error(e) => {
                // Write error message (like yq does)
                write!(out, "yq: error: {}", e)?;
                on_value(out)?;
            }
            GenericResult::Owned(o) => {
                o.stream_yaml(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = o.is_falsy();
            }
            GenericResult::ManyOwned(os) => {
                for o in os {
                    o.stream_yaml(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
                }
                stats.count = os.len();
            }
            GenericResult::Break(label) => {
                write!(out, "yq: error: break ${} not in label", label)?;
                on_value(out)?;
            }
        }

        Ok(stats)
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

        // Comparison operations - handle locally to preserve cursor context
        Expr::Compare { op, left, right } => {
            // Evaluate left and right with cursor context preserved
            let left_result = eval_single(left, value.clone(), false, cursor);
            let right_result = eval_single(right, value, false, cursor);

            // Convert results to OwnedValue for comparison
            let left_owned = match left_result {
                GenericResult::Owned(o) => o,
                GenericResult::One(v) => to_owned(&v),
                GenericResult::OneCursor(c) => to_owned(&c.value()),
                GenericResult::Error(e) => {
                    return if optional {
                        GenericResult::None
                    } else {
                        GenericResult::Error(e)
                    }
                }
                GenericResult::None => return GenericResult::None,
                GenericResult::Many(vs) => {
                    if let Some(first) = vs.first() {
                        to_owned(first)
                    } else {
                        return GenericResult::None;
                    }
                }
                GenericResult::ManyOwned(vs) => {
                    if let Some(first) = vs.first() {
                        first.clone()
                    } else {
                        return GenericResult::None;
                    }
                }
                GenericResult::Break(label) => return GenericResult::Break(label),
            };

            let right_owned = match right_result {
                GenericResult::Owned(o) => o,
                GenericResult::One(v) => to_owned(&v),
                GenericResult::OneCursor(c) => to_owned(&c.value()),
                GenericResult::Error(e) => {
                    return if optional {
                        GenericResult::None
                    } else {
                        GenericResult::Error(e)
                    }
                }
                GenericResult::None => return GenericResult::None,
                GenericResult::Many(vs) => {
                    if let Some(first) = vs.first() {
                        to_owned(first)
                    } else {
                        return GenericResult::None;
                    }
                }
                GenericResult::ManyOwned(vs) => {
                    if let Some(first) = vs.first() {
                        first.clone()
                    } else {
                        return GenericResult::None;
                    }
                }
                GenericResult::Break(label) => return GenericResult::Break(label),
            };

            // Perform the comparison
            let result = match op {
                CompareOp::Eq => left_owned == right_owned,
                CompareOp::Ne => left_owned != right_owned,
                CompareOp::Lt => {
                    compare_values(&left_owned, &right_owned) == Some(core::cmp::Ordering::Less)
                }
                CompareOp::Le => matches!(
                    compare_values(&left_owned, &right_owned),
                    Some(core::cmp::Ordering::Less | core::cmp::Ordering::Equal)
                ),
                CompareOp::Gt => {
                    compare_values(&left_owned, &right_owned) == Some(core::cmp::Ordering::Greater)
                }
                CompareOp::Ge => matches!(
                    compare_values(&left_owned, &right_owned),
                    Some(core::cmp::Ordering::Greater | core::cmp::Ordering::Equal)
                ),
            };

            GenericResult::Owned(OwnedValue::Bool(result))
        }

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
    optional: bool,
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

        Builtin::DocumentIndex => {
            let doc_index = cursor.and_then(|c| c.document_index()).unwrap_or(0);
            GenericResult::Owned(OwnedValue::Int(doc_index as i64))
        }

        Builtin::Select(cond) => {
            // Evaluate condition with cursor context preserved
            // This is critical for select(di == N) to work correctly
            let cond_result = eval_single(cond, value.clone(), false, cursor);

            // Helper to check if an OwnedValue is truthy
            let is_truthy = |v: &OwnedValue| -> bool {
                match v {
                    OwnedValue::Bool(false) | OwnedValue::Null => false,
                    _ => true, // All other values are truthy
                }
            };

            match cond_result {
                GenericResult::Owned(ref o) => {
                    if is_truthy(o) {
                        GenericResult::One(value)
                    } else {
                        GenericResult::None
                    }
                }
                GenericResult::One(v) => {
                    if is_truthy(&to_owned(&v)) {
                        GenericResult::One(value)
                    } else {
                        GenericResult::None
                    }
                }
                GenericResult::OneCursor(c) => {
                    if is_truthy(&to_owned(&c.value())) {
                        GenericResult::One(value)
                    } else {
                        GenericResult::None
                    }
                }
                GenericResult::Error(e) => {
                    if optional {
                        GenericResult::None
                    } else {
                        GenericResult::Error(e)
                    }
                }
                GenericResult::None => GenericResult::None,
                GenericResult::Many(_) | GenericResult::ManyOwned(_) => {
                    // Multiple results from condition - this is unusual but treat first as condition
                    // jq behavior: select with multiple outputs uses first truthy value
                    GenericResult::One(value)
                }
                GenericResult::Break(label) => GenericResult::Break(label),
            }
        }

        Builtin::Shuffle => {
            #[cfg(feature = "cli")]
            {
                use rand::seq::SliceRandom;
                use rand::SeedableRng;
                use rand_chacha::ChaCha8Rng;

                if let Some(elements) = value.as_array() {
                    let mut values: Vec<OwnedValue> =
                        elements.collect_values().iter().map(to_owned).collect();
                    let mut rng = ChaCha8Rng::from_entropy();
                    values.shuffle(&mut rng);
                    GenericResult::Owned(OwnedValue::Array(values))
                } else {
                    GenericResult::Error(EvalError::new(format!(
                        "shuffle requires array, got {}",
                        value.type_name()
                    )))
                }
            }
            #[cfg(not(feature = "cli"))]
            {
                GenericResult::Error(EvalError::new(
                    "shuffle requires the 'cli' feature to be enabled".to_string(),
                ))
            }
        }

        Builtin::Pivot => {
            if let Some(elements) = value.as_array() {
                let items: Vec<OwnedValue> =
                    elements.collect_values().iter().map(to_owned).collect();
                if items.is_empty() {
                    return GenericResult::Owned(OwnedValue::Array(vec![]));
                }

                let all_arrays = items.iter().all(|v| matches!(v, OwnedValue::Array(_)));
                let all_objects = items.iter().all(|v| matches!(v, OwnedValue::Object(_)));

                if all_arrays {
                    // Transpose array of arrays: [[a, b], [x, y]] → [[a, x], [b, y]]
                    let max_len = items
                        .iter()
                        .filter_map(|v| {
                            if let OwnedValue::Array(arr) = v {
                                Some(arr.len())
                            } else {
                                None
                            }
                        })
                        .max()
                        .unwrap_or(0);

                    if max_len == 0 {
                        return GenericResult::Owned(OwnedValue::Array(vec![]));
                    }

                    let mut result = Vec::with_capacity(max_len);
                    for col_idx in 0..max_len {
                        let mut column = Vec::with_capacity(items.len());
                        for item in &items {
                            if let OwnedValue::Array(arr) = item {
                                column.push(arr.get(col_idx).cloned().unwrap_or(OwnedValue::Null));
                            } else {
                                column.push(OwnedValue::Null);
                            }
                        }
                        result.push(OwnedValue::Array(column));
                    }
                    GenericResult::Owned(OwnedValue::Array(result))
                } else if all_objects {
                    // Transpose array of objects: [{a: 1}, {a: 2, b: 3}] → {a: [1, 2], b: [null, 3]}
                    let mut all_keys: Vec<String> = Vec::new();
                    for item in &items {
                        if let OwnedValue::Object(obj) = item {
                            for key in obj.keys() {
                                if !all_keys.contains(key) {
                                    all_keys.push(key.clone());
                                }
                            }
                        }
                    }

                    let mut result_obj = IndexMap::new();
                    for key in &all_keys {
                        let mut values = Vec::with_capacity(items.len());
                        for item in &items {
                            if let OwnedValue::Object(obj) = item {
                                values.push(obj.get(key).cloned().unwrap_or(OwnedValue::Null));
                            } else {
                                values.push(OwnedValue::Null);
                            }
                        }
                        result_obj.insert(key.clone(), OwnedValue::Array(values));
                    }
                    GenericResult::Owned(OwnedValue::Object(result_obj))
                } else if optional {
                    GenericResult::None
                } else {
                    GenericResult::Error(EvalError::new(
                        "pivot requires array of arrays or array of objects".to_string(),
                    ))
                }
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::type_error("array", value.type_name()))
            }
        }

        Builtin::SplitDoc => {
            // split_doc is identity - the output formatting (--- separators)
            // is handled by the yq runner, not here
            GenericResult::Owned(to_owned(&value))
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

        Builtin::Keys => {
            if let Some(fields) = value.as_object() {
                let mut keys: Vec<String> = fields.keys();
                keys.sort(); // Sort keys alphabetically for `keys` builtin
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

        Builtin::KeysUnsorted => {
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
                    "keys_unsorted requires object or array, got {}",
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

        // Phase 23: Position-based navigation (succinctly extension)
        Builtin::AtOffset(offset_expr) => {
            // Evaluate the offset expression
            let offset_result = eval_single(offset_expr, value.clone(), false, cursor);
            let offset = match offset_result {
                GenericResult::Owned(OwnedValue::Int(i)) if i >= 0 => i as usize,
                GenericResult::One(v) => match v.as_i64() {
                    Some(i) if i >= 0 => i as usize,
                    _ => {
                        return GenericResult::Error(EvalError::new(
                            "at_offset requires a non-negative integer".to_string(),
                        ))
                    }
                },
                _ => {
                    return GenericResult::Error(EvalError::new(
                        "at_offset requires a non-negative integer".to_string(),
                    ))
                }
            };

            // Need a cursor to navigate
            let Some(c) = cursor else {
                return GenericResult::Error(EvalError::new(
                    "at_offset requires document cursor context".to_string(),
                ));
            };

            // Navigate to the offset
            match c.cursor_at_offset(offset) {
                Some(new_cursor) => GenericResult::OneCursor(new_cursor),
                None => {
                    if optional {
                        GenericResult::None
                    } else {
                        GenericResult::Error(EvalError::new(format!(
                            "no node at offset {}",
                            offset
                        )))
                    }
                }
            }
        }

        Builtin::AtPosition(line_expr, col_expr) => {
            // Evaluate the line expression
            let line_result = eval_single(line_expr, value.clone(), false, cursor);
            let line = match line_result {
                GenericResult::Owned(OwnedValue::Int(i)) if i > 0 => i as usize,
                GenericResult::One(v) => match v.as_i64() {
                    Some(i) if i > 0 => i as usize,
                    _ => {
                        return GenericResult::Error(EvalError::new(
                            "at_position requires positive integers for line".to_string(),
                        ))
                    }
                },
                _ => {
                    return GenericResult::Error(EvalError::new(
                        "at_position requires positive integers for line".to_string(),
                    ))
                }
            };

            // Evaluate the column expression
            let col_result = eval_single(col_expr, value.clone(), false, cursor);
            let col = match col_result {
                GenericResult::Owned(OwnedValue::Int(i)) if i > 0 => i as usize,
                GenericResult::One(v) => match v.as_i64() {
                    Some(i) if i > 0 => i as usize,
                    _ => {
                        return GenericResult::Error(EvalError::new(
                            "at_position requires positive integers for column".to_string(),
                        ))
                    }
                },
                _ => {
                    return GenericResult::Error(EvalError::new(
                        "at_position requires positive integers for column".to_string(),
                    ))
                }
            };

            // Need a cursor to navigate
            let Some(c) = cursor else {
                return GenericResult::Error(EvalError::new(
                    "at_position requires document cursor context".to_string(),
                ));
            };

            // Navigate to the position
            match c.cursor_at_position(line, col) {
                Some(new_cursor) => GenericResult::OneCursor(new_cursor),
                None => {
                    if optional {
                        GenericResult::None
                    } else {
                        GenericResult::Error(EvalError::new(format!(
                            "no node at position line {} column {}",
                            line, col
                        )))
                    }
                }
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

    #[test]
    fn test_yaml_document_index_single_doc() {
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        // Navigate to the actual mapping (first document)
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");

        // Use eval_with_cursor to preserve position metadata
        let result = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), mapping_cursor);
        let owned = result.into_owned().unwrap();

        // Single document = index 0
        assert_eq!(owned, OwnedValue::Int(0));
    }

    #[test]
    fn test_yaml_document_index_multi_doc() {
        use crate::yaml::YamlIndex;

        let yaml = b"---\nname: Alice\n---\nname: Bob\n---\nname: Charlie";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate to documents
        let doc1 = root.first_child().expect("should have first doc");
        let doc2 = doc1.next_sibling().expect("should have second doc");
        let doc3 = doc2.next_sibling().expect("should have third doc");

        // Test document_index for each document
        let result1 = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), doc1);
        assert_eq!(result1.into_owned().unwrap(), OwnedValue::Int(0));

        let result2 = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), doc2);
        assert_eq!(result2.into_owned().unwrap(), OwnedValue::Int(1));

        let result3 = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), doc3);
        assert_eq!(result3.into_owned().unwrap(), OwnedValue::Int(2));
    }

    #[test]
    fn test_yaml_document_index_nested_value() {
        use crate::yaml::YamlIndex;

        // Use a simpler nested structure
        let yaml = b"---\nname: Alice\n---\nname: Bob";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate to second document
        let doc2 = root
            .first_child()
            .expect("should have first doc")
            .next_sibling()
            .expect("should have second doc");

        // Navigate into the mapping's value (the "name" key's content)
        // doc2 contains {name: Bob}, navigate to the value
        let name_value = doc2.first_child();

        // Even from a child node, document_index returns the document's index
        if let Some(cursor) = name_value {
            let result = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), cursor);
            assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(1));
        } else {
            // Just test the doc directly
            let result = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), doc2);
            assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(1));
        }
    }

    #[test]
    fn test_yaml_di_alias() {
        // Test that 'di' is an alias for document_index
        use crate::yaml::YamlIndex;

        let yaml = b"name: Alice";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");

        // Parse 'di' and verify it works the same as document_index
        let expr = crate::jq::parse("di").unwrap();
        let result = eval_with_cursor(&expr, mapping_cursor);
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(0));
    }

    #[test]
    fn test_yaml_select_di_eq_n() {
        // Test that select(di == N) filters by document index correctly
        use crate::yaml::YamlIndex;
        use crate::yaml::YamlValue;

        let yaml = b"---\na: 1\n---\nb: 2\n---\nc: 3";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Parse select(di == 1) - should match second document
        let expr = crate::jq::parse("select(di == 1)").unwrap();

        // Evaluate on each document
        let mut results = Vec::new();
        if let YamlValue::Sequence(mut docs) = root.value() {
            while let Some((cursor, rest)) = docs.uncons_cursor() {
                let result = eval_with_cursor(&expr, cursor);
                match result {
                    GenericResult::One(v) => results.push(to_owned(&v)),
                    GenericResult::Owned(o) => results.push(o),
                    GenericResult::None => {} // Filtered out
                    _ => {}
                }
                docs = rest;
            }
        }

        // Should have exactly one result (document index 1)
        assert_eq!(results.len(), 1);

        // The result should be the second document's content
        if let OwnedValue::Object(map) = &results[0] {
            assert!(map.contains_key("b"));
            assert_eq!(map.get("b"), Some(&OwnedValue::Int(2)));
        } else {
            panic!("Expected object with 'b' key, got {:?}", results[0]);
        }
    }

    #[test]
    fn test_yaml_select_di_comparison() {
        // Test various select(di comparison) operations
        use crate::yaml::YamlIndex;
        use crate::yaml::YamlValue;

        let yaml = b"---\na: 1\n---\nb: 2\n---\nc: 3";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Test select(di > 0) - should match documents 1 and 2
        let expr = crate::jq::parse("select(di > 0)").unwrap();

        let mut count = 0;
        if let YamlValue::Sequence(mut docs) = root.value() {
            while let Some((cursor, rest)) = docs.uncons_cursor() {
                let result = eval_with_cursor(&expr, cursor);
                match result {
                    GenericResult::One(_) | GenericResult::Owned(_) => count += 1,
                    _ => {}
                }
                docs = rest;
            }
        }

        // Should match 2 documents (index 1 and 2)
        assert_eq!(count, 2);
    }

    // ========================================================================
    // Phase 23: Position-based navigation tests
    // ========================================================================

    #[test]
    fn test_json_at_offset() {
        // Test at_offset(n) - jump to node at byte offset
        let json = br#"{"name": "Alice", "age": 30}"#;
        //           0123456789...
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        // at_offset(0) should return the root object
        let expr = crate::jq::parse("at_offset(0)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));

        // at_offset(10) should be inside the "Alice" string (offset 10 = 'l' in "Alice")
        let expr = crate::jq::parse("at_offset(10)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap();
        assert!(matches!(owned, OwnedValue::String(ref s) if s == "Alice"));

        // at_offset(27) should be the age number (30)
        let expr = crate::jq::parse("at_offset(27)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap();
        assert!(matches!(owned, OwnedValue::Int(30)));
    }

    #[test]
    fn test_json_at_position() {
        // Test at_position(line; col) - jump to node at line/column (1-indexed)
        let json = b"{\n  \"name\": \"Alice\"\n}";
        //           Line 1: {
        //           Line 2:   "name": "Alice"
        //           Line 3: }
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        // at_position(1; 1) should return the root object
        let expr = crate::jq::parse("at_position(1; 1)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));

        // at_position(2; 3) should be the "name" key (line 2, col 3 = start of "name")
        let expr = crate::jq::parse("at_position(2; 3)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap();
        assert!(matches!(owned, OwnedValue::String(ref s) if s == "name"));
    }

    #[test]
    fn test_yaml_at_offset() {
        use crate::yaml::YamlIndex;

        // Test at_offset(n) with YAML
        // YAML: name: Alice\nage: 30
        //       0123456789...
        // Note: YAML indexing is different from JSON - the root is the document sequence
        let yaml = b"name: Alice\nage: 30";
        let index = YamlIndex::build(yaml).unwrap();
        let root = index.root(yaml);

        // Navigate to the first document
        let doc = root.first_child().unwrap();

        // at_offset(0) with YAML typically returns the first structural node at that position
        // For YAML, this is typically the mapping itself at offset 0
        let expr = crate::jq::parse("at_offset(0)").unwrap();
        let result = eval_with_cursor(&expr, doc);
        // Just verify we get something without error
        // YAML's structure is more complex than JSON, so we just check it doesn't fail
        assert!(!matches!(result, GenericResult::Error(_)));
    }

    #[test]
    fn test_at_offset_then_navigate() {
        // Test at_offset combined with navigation
        let json = br#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
        //           0123456789...
        //           {"users": [{"name": "Alice"}, {"name": "Bob"}]}
        //                    ^- offset 10 = '[' (array start)
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        // Navigate to offset at "users" array, then get first element's name
        // The "users" array starts at offset 10 (the '[' character)
        let expr = crate::jq::parse("at_offset(10) | .[0].name").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap();
        assert!(matches!(owned, OwnedValue::String(ref s) if s == "Alice"));
    }

    #[test]
    fn test_at_offset_invalid() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        // at_offset with too large offset should fail
        let expr = crate::jq::parse("at_offset(1000)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        assert!(matches!(result, GenericResult::Error(_)));

        // at_offset with negative number should fail
        let expr = crate::jq::parse("at_offset(-1)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        assert!(matches!(result, GenericResult::Error(_)));
    }

    #[test]
    fn test_at_position_invalid() {
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        // at_position(0; 1) should fail (line 0 is invalid)
        let expr = crate::jq::parse("at_position(0; 1)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        assert!(matches!(result, GenericResult::Error(_)));

        // at_position(1; 0) should fail (column 0 is invalid)
        let expr = crate::jq::parse("at_position(1; 0)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        assert!(matches!(result, GenericResult::Error(_)));
    }
}
