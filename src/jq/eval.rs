//! Expression evaluator for jq-like queries.
//!
//! Evaluates expressions against JSON using the cursor-based navigation API.

#[cfg(not(test))]
use alloc::collections::BTreeMap;
#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::collections::BTreeMap;

use crate::json::light::{JsonCursor, JsonElements, JsonFields, StandardJson};

use super::expr::{ArithOp, CompareOp, Expr, Literal, ObjectKey};
use super::value::OwnedValue;

/// Result of evaluating a jq expression.
#[derive(Debug)]
pub enum QueryResult<'a, W = Vec<u64>> {
    /// Single value result (reference to original JSON).
    One(StandardJson<'a, W>),

    /// Multiple values (from iteration).
    Many(Vec<StandardJson<'a, W>>),

    /// No result (optional that was missing).
    None,

    /// Error during evaluation.
    Error(EvalError),

    /// Single owned value (from construction/computation).
    Owned(OwnedValue),

    /// Multiple owned values.
    ManyOwned(Vec<OwnedValue>),
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

/// Convert a StandardJson value to an OwnedValue.
fn to_owned<W: Clone + AsRef<[u64]>>(value: &StandardJson<'_, W>) -> OwnedValue {
    match value {
        StandardJson::Null => OwnedValue::Null,
        StandardJson::Bool(b) => OwnedValue::Bool(*b),
        StandardJson::Number(n) => {
            if let Ok(i) = n.as_i64() {
                OwnedValue::Int(i)
            } else if let Ok(f) = n.as_f64() {
                OwnedValue::Float(f)
            } else {
                // Fallback - shouldn't happen for valid JSON
                OwnedValue::Float(0.0)
            }
        }
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                OwnedValue::String(cow.into_owned())
            } else {
                OwnedValue::String(String::new())
            }
        }
        StandardJson::Array(elements) => {
            let items: Vec<OwnedValue> = elements.clone().map(|e| to_owned(&e)).collect();
            OwnedValue::Array(items)
        }
        StandardJson::Object(fields) => {
            let mut map = BTreeMap::new();
            for field in fields.clone() {
                // Get the key as a string
                if let StandardJson::String(key_str_val) = field.key() {
                    if let Ok(cow) = key_str_val.as_str() {
                        map.insert(cow.into_owned(), to_owned(&field.value()));
                    }
                }
            }
            OwnedValue::Object(map)
        }
        StandardJson::Error(_) => OwnedValue::Null,
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
            StandardJson::Array(elements) => match get_element_at_index(elements, *idx) {
                Some(v) => QueryResult::One(v),
                None if optional => QueryResult::None,
                None => {
                    // Count elements to give accurate error
                    let len = count_elements(elements);
                    QueryResult::Error(EvalError::index_out_of_bounds(*idx, len))
                }
            },
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

        Expr::Comma(exprs) => eval_comma(exprs, value, optional),

        Expr::Array(inner) => eval_array_construction(inner, value, optional),

        Expr::Object(entries) => eval_object_construction(entries, value, optional),

        Expr::Literal(lit) => QueryResult::Owned(literal_to_owned(lit)),

        Expr::RecursiveDescent => eval_recursive_descent(value),

        Expr::Paren(inner) => eval_single(inner, value, optional),

        Expr::Arithmetic { op, left, right } => eval_arithmetic(*op, left, right, value, optional),

        Expr::Compare { op, left, right } => eval_compare(*op, left, right, value, optional),

        Expr::And(left, right) => eval_and(left, right, value, optional),

        Expr::Or(left, right) => eval_or(left, right, value, optional),

        Expr::Not => eval_not(value),

        Expr::Alternative(left, right) => eval_alternative(left, right, value, optional),
    }
}

/// Convert a literal to an owned value.
fn literal_to_owned(lit: &Literal) -> OwnedValue {
    match lit {
        Literal::Null => OwnedValue::Null,
        Literal::Bool(b) => OwnedValue::Bool(*b),
        Literal::Int(n) => OwnedValue::Int(*n),
        Literal::Float(f) => OwnedValue::Float(*f),
        Literal::String(s) => OwnedValue::String(s.clone()),
    }
}

/// Evaluate a comma expression (multiple outputs).
fn eval_comma<'a, W: Clone + AsRef<[u64]>>(
    exprs: &[Expr],
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    if exprs.is_empty() {
        return QueryResult::None;
    }

    let mut all_results = Vec::new();
    let mut all_owned = Vec::new();
    let mut has_owned = false;

    for expr in exprs {
        match eval_single(expr, value.clone(), optional) {
            QueryResult::One(v) => all_results.push(v),
            QueryResult::Many(vs) => all_results.extend(vs),
            QueryResult::Owned(v) => {
                has_owned = true;
                all_owned.push(v);
            }
            QueryResult::ManyOwned(vs) => {
                has_owned = true;
                all_owned.extend(vs);
            }
            QueryResult::None => {}
            QueryResult::Error(e) => return QueryResult::Error(e),
        }
    }

    // If we have any owned values, we need to convert all results to owned
    if has_owned {
        let mut converted: Vec<OwnedValue> = all_results.iter().map(to_owned).collect();
        converted.extend(all_owned);
        if converted.len() == 1 {
            QueryResult::Owned(converted.pop().unwrap())
        } else {
            QueryResult::ManyOwned(converted)
        }
    } else if all_results.len() == 1 {
        QueryResult::One(all_results.pop().unwrap())
    } else {
        QueryResult::Many(all_results)
    }
}

/// Evaluate array construction.
fn eval_array_construction<'a, W: Clone + AsRef<[u64]>>(
    inner: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Collect all outputs from the inner expression into an array
    let result = eval_single(inner, value, optional);

    let items: Vec<OwnedValue> = match result {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => vec![],
        QueryResult::Error(e) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Array(items))
}

/// Evaluate object construction.
fn eval_object_construction<'a, W: Clone + AsRef<[u64]>>(
    entries: &[super::expr::ObjectEntry],
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut map = BTreeMap::new();

    for entry in entries {
        // Evaluate the key
        let key_str = match &entry.key {
            ObjectKey::Literal(s) => s.clone(),
            ObjectKey::Expr(key_expr) => {
                let key_result = eval_single(key_expr, value.clone(), optional);
                match key_result {
                    QueryResult::One(StandardJson::String(s)) => {
                        if let Ok(cow) = s.as_str() {
                            cow.into_owned()
                        } else {
                            return QueryResult::Error(EvalError::new("key must be a string"));
                        }
                    }
                    QueryResult::Owned(OwnedValue::String(s)) => s,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    _ => {
                        return QueryResult::Error(EvalError::new("key must be a string"));
                    }
                }
            }
        };

        // Evaluate the value
        let val_result = eval_single(&entry.value, value.clone(), optional);
        let owned_val = match val_result {
            QueryResult::One(v) => to_owned(&v),
            QueryResult::Owned(v) => v,
            QueryResult::Many(vs) => {
                // Multiple values - take the first one (jq behavior)
                if let Some(v) = vs.first() {
                    to_owned(v)
                } else {
                    OwnedValue::Null
                }
            }
            QueryResult::ManyOwned(vs) => {
                if let Some(v) = vs.into_iter().next() {
                    v
                } else {
                    OwnedValue::Null
                }
            }
            QueryResult::None => OwnedValue::Null,
            QueryResult::Error(e) => return QueryResult::Error(e),
        };

        map.insert(key_str, owned_val);
    }

    QueryResult::Owned(OwnedValue::Object(map))
}

/// Evaluate recursive descent.
fn eval_recursive_descent<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
) -> QueryResult<'a, W> {
    let mut results = Vec::new();
    collect_recursive(&value, &mut results);
    QueryResult::Many(results)
}

/// Collect all values recursively.
fn collect_recursive<'a, W: Clone + AsRef<[u64]>>(
    value: &StandardJson<'a, W>,
    results: &mut Vec<StandardJson<'a, W>>,
) {
    results.push(value.clone());

    match value {
        StandardJson::Array(elements) => {
            for elem in elements.clone() {
                collect_recursive(&elem, results);
            }
        }
        StandardJson::Object(fields) => {
            for field in fields.clone() {
                collect_recursive(&field.value(), results);
            }
        }
        _ => {}
    }
}

/// Convert a QueryResult to an OwnedValue for use in computations.
fn result_to_owned<W: Clone + AsRef<[u64]>>(
    result: QueryResult<'_, W>,
) -> Result<OwnedValue, EvalError> {
    match result {
        QueryResult::One(v) => Ok(to_owned(&v)),
        QueryResult::Owned(v) => Ok(v),
        QueryResult::Many(vs) => {
            if let Some(v) = vs.first() {
                Ok(to_owned(v))
            } else {
                Err(EvalError::new("empty result"))
            }
        }
        QueryResult::ManyOwned(vs) => {
            if let Some(v) = vs.into_iter().next() {
                Ok(v)
            } else {
                Err(EvalError::new("empty result"))
            }
        }
        QueryResult::None => Err(EvalError::new("no value")),
        QueryResult::Error(e) => Err(e),
    }
}

/// Evaluate arithmetic operations.
fn eval_arithmetic<'a, W: Clone + AsRef<[u64]>>(
    op: ArithOp,
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let left_val = match result_to_owned(eval_single(left, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };
    let right_val = match result_to_owned(eval_single(right, value, optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let result = match op {
        ArithOp::Add => arith_add(left_val, right_val),
        ArithOp::Sub => arith_sub(left_val, right_val),
        ArithOp::Mul => arith_mul(left_val, right_val),
        ArithOp::Div => arith_div(left_val, right_val),
        ArithOp::Mod => arith_mod(left_val, right_val),
    };

    match result {
        Ok(v) => QueryResult::Owned(v),
        Err(e) => QueryResult::Error(e),
    }
}

/// Add two values (numbers, strings, arrays, objects).
fn arith_add(left: OwnedValue, right: OwnedValue) -> Result<OwnedValue, EvalError> {
    match (left, right) {
        // Number addition
        (OwnedValue::Int(a), OwnedValue::Int(b)) => Ok(OwnedValue::Int(a.wrapping_add(b))),
        (OwnedValue::Int(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a as f64 + b)),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => Ok(OwnedValue::Float(a + b as f64)),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a + b)),
        // String concatenation
        (OwnedValue::String(mut a), OwnedValue::String(b)) => {
            a.push_str(&b);
            Ok(OwnedValue::String(a))
        }
        // Array concatenation
        (OwnedValue::Array(mut a), OwnedValue::Array(b)) => {
            a.extend(b);
            Ok(OwnedValue::Array(a))
        }
        // Object merge (right overwrites left)
        (OwnedValue::Object(mut a), OwnedValue::Object(b)) => {
            a.extend(b);
            Ok(OwnedValue::Object(a))
        }
        // null + x = x, x + null = x
        (OwnedValue::Null, other) | (other, OwnedValue::Null) => Ok(other),
        (a, b) => Err(EvalError::new(format!(
            "cannot add {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

/// Subtract two values.
fn arith_sub(left: OwnedValue, right: OwnedValue) -> Result<OwnedValue, EvalError> {
    match (left, right) {
        (OwnedValue::Int(a), OwnedValue::Int(b)) => Ok(OwnedValue::Int(a.wrapping_sub(b))),
        (OwnedValue::Int(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a as f64 - b)),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => Ok(OwnedValue::Float(a - b as f64)),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a - b)),
        // Array subtraction (remove elements)
        (OwnedValue::Array(a), OwnedValue::Array(b)) => {
            let result: Vec<_> = a.into_iter().filter(|x| !b.contains(x)).collect();
            Ok(OwnedValue::Array(result))
        }
        (a, b) => Err(EvalError::new(format!(
            "cannot subtract {} from {}",
            b.type_name(),
            a.type_name()
        ))),
    }
}

/// Multiply two values.
fn arith_mul(left: OwnedValue, right: OwnedValue) -> Result<OwnedValue, EvalError> {
    match (left, right) {
        (OwnedValue::Int(a), OwnedValue::Int(b)) => Ok(OwnedValue::Int(a.wrapping_mul(b))),
        (OwnedValue::Int(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a as f64 * b)),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => Ok(OwnedValue::Float(a * b as f64)),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a * b)),
        // String repetition: "ab" * 3 = "ababab"
        (OwnedValue::String(s), OwnedValue::Int(n))
        | (OwnedValue::Int(n), OwnedValue::String(s)) => {
            if n < 0 {
                Ok(OwnedValue::Null)
            } else {
                Ok(OwnedValue::String(s.repeat(n as usize)))
            }
        }
        // Object recursive merge
        (OwnedValue::Object(a), OwnedValue::Object(b)) => {
            Ok(OwnedValue::Object(merge_objects(a, b)))
        }
        // null * x = null
        (OwnedValue::Null, _) | (_, OwnedValue::Null) => Ok(OwnedValue::Null),
        (a, b) => Err(EvalError::new(format!(
            "cannot multiply {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

/// Recursively merge two objects.
fn merge_objects(
    mut left: BTreeMap<String, OwnedValue>,
    right: BTreeMap<String, OwnedValue>,
) -> BTreeMap<String, OwnedValue> {
    for (k, v) in right {
        match (left.get(&k).cloned(), v) {
            (Some(OwnedValue::Object(a)), OwnedValue::Object(b)) => {
                left.insert(k, OwnedValue::Object(merge_objects(a, b)));
            }
            (_, v) => {
                left.insert(k, v);
            }
        }
    }
    left
}

/// Divide two values.
fn arith_div(left: OwnedValue, right: OwnedValue) -> Result<OwnedValue, EvalError> {
    match (left, right) {
        (OwnedValue::Int(a), OwnedValue::Int(b)) => {
            if b == 0 {
                Err(EvalError::new("division by zero"))
            } else {
                Ok(OwnedValue::Float(a as f64 / b as f64))
            }
        }
        (OwnedValue::Int(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a as f64 / b)),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => Ok(OwnedValue::Float(a / b as f64)),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a / b)),
        // String split: "a,b,c" / "," = ["a", "b", "c"]
        (OwnedValue::String(s), OwnedValue::String(sep)) => {
            let parts: Vec<OwnedValue> = s
                .split(&sep)
                .map(|p| OwnedValue::String(p.to_string()))
                .collect();
            Ok(OwnedValue::Array(parts))
        }
        (a, b) => Err(EvalError::new(format!(
            "cannot divide {} by {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

/// Modulo two values.
fn arith_mod(left: OwnedValue, right: OwnedValue) -> Result<OwnedValue, EvalError> {
    match (left, right) {
        (OwnedValue::Int(a), OwnedValue::Int(b)) => {
            if b == 0 {
                Err(EvalError::new("modulo by zero"))
            } else {
                Ok(OwnedValue::Int(a % b))
            }
        }
        (OwnedValue::Float(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a % b)),
        (OwnedValue::Int(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a as f64 % b)),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => Ok(OwnedValue::Float(a % b as f64)),
        (a, b) => Err(EvalError::new(format!(
            "cannot compute modulo of {} and {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

/// Evaluate comparison operations.
fn eval_compare<'a, W: Clone + AsRef<[u64]>>(
    op: CompareOp,
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let left_val = match result_to_owned(eval_single(left, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };
    let right_val = match result_to_owned(eval_single(right, value, optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let result = match op {
        CompareOp::Eq => left_val == right_val,
        CompareOp::Ne => left_val != right_val,
        CompareOp::Lt => compare_values(&left_val, &right_val) == core::cmp::Ordering::Less,
        CompareOp::Le => compare_values(&left_val, &right_val) != core::cmp::Ordering::Greater,
        CompareOp::Gt => compare_values(&left_val, &right_val) == core::cmp::Ordering::Greater,
        CompareOp::Ge => compare_values(&left_val, &right_val) != core::cmp::Ordering::Less,
    };

    QueryResult::Owned(OwnedValue::Bool(result))
}

/// Compare two values using jq ordering: null < bool < number < string < array < object.
fn compare_values(left: &OwnedValue, right: &OwnedValue) -> core::cmp::Ordering {
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
        return left_type.cmp(&right_type);
    }

    match (left, right) {
        (OwnedValue::Null, OwnedValue::Null) => Ordering::Equal,
        (OwnedValue::Bool(a), OwnedValue::Bool(b)) => a.cmp(b),
        (OwnedValue::Int(a), OwnedValue::Int(b)) => a.cmp(b),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (OwnedValue::Int(a), OwnedValue::Float(b)) => {
            (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
        }
        (OwnedValue::Float(a), OwnedValue::Int(b)) => {
            a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
        }
        (OwnedValue::String(a), OwnedValue::String(b)) => a.cmp(b),
        (OwnedValue::Array(a), OwnedValue::Array(b)) => {
            for (av, bv) in a.iter().zip(b.iter()) {
                match compare_values(av, bv) {
                    Ordering::Equal => continue,
                    other => return other,
                }
            }
            a.len().cmp(&b.len())
        }
        (OwnedValue::Object(a), OwnedValue::Object(b)) => {
            // Compare objects by sorted keys, then values
            let mut a_keys: Vec<_> = a.keys().collect();
            let mut b_keys: Vec<_> = b.keys().collect();
            a_keys.sort();
            b_keys.sort();

            for (ak, bk) in a_keys.iter().zip(b_keys.iter()) {
                match ak.cmp(bk) {
                    Ordering::Equal => match compare_values(&a[*ak], &b[*bk]) {
                        Ordering::Equal => continue,
                        other => return other,
                    },
                    other => return other,
                }
            }
            a.len().cmp(&b.len())
        }
        _ => Ordering::Equal,
    }
}

/// Evaluate boolean AND (short-circuiting).
fn eval_and<'a, W: Clone + AsRef<[u64]>>(
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate left first
    let left_val = match result_to_owned(eval_single(left, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    // Short-circuit: if left is falsy, return false
    if !left_val.is_truthy() {
        return QueryResult::Owned(OwnedValue::Bool(false));
    }

    // Evaluate right
    let right_val = match result_to_owned(eval_single(right, value, optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Bool(right_val.is_truthy()))
}

/// Evaluate boolean OR (short-circuiting).
fn eval_or<'a, W: Clone + AsRef<[u64]>>(
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate left first
    let left_val = match result_to_owned(eval_single(left, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    // Short-circuit: if left is truthy, return true
    if left_val.is_truthy() {
        return QueryResult::Owned(OwnedValue::Bool(true));
    }

    // Evaluate right
    let right_val = match result_to_owned(eval_single(right, value, optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Bool(right_val.is_truthy()))
}

/// Evaluate boolean NOT.
fn eval_not<'a, W: Clone + AsRef<[u64]>>(value: StandardJson<'a, W>) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    QueryResult::Owned(OwnedValue::Bool(!owned.is_truthy()))
}

/// Evaluate alternative operator (//): returns left if truthy, otherwise right.
fn eval_alternative<'a, W: Clone + AsRef<[u64]>>(
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate left
    let left_result = eval_single(left, value.clone(), optional);

    // Check if left produced a truthy result
    let is_truthy = match &left_result {
        QueryResult::One(v) => to_owned(v).is_truthy(),
        QueryResult::Owned(v) => v.is_truthy(),
        QueryResult::Many(vs) => vs.first().map(|v| to_owned(v).is_truthy()).unwrap_or(false),
        QueryResult::ManyOwned(vs) => vs.first().map(|v| v.is_truthy()).unwrap_or(false),
        QueryResult::None => false,
        QueryResult::Error(_) => false,
    };

    if is_truthy {
        left_result
    } else {
        eval_single(right, value, optional)
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
                    QueryResult::Owned(_) | QueryResult::ManyOwned(_) => {
                        // TODO: Handle owned values in pipe properly
                        // For now, skip (this would need refactoring)
                    }
                }
            }
            QueryResult::Many(all_results)
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Owned(_) | QueryResult::ManyOwned(_) => {
            // Cannot continue piping with owned values without more complex handling
            // For Phase 1, we return the owned value as-is
            result
        }
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
        QueryResult::Owned(_) => Vec::new(), // Owned values not returned as StandardJson
        QueryResult::ManyOwned(_) => Vec::new(),
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

    #[test]
    fn test_comma() {
        query!(br#"{"a": 1, "b": 2}"#, ".a, .b",
            QueryResult::Many(values) => {
                assert_eq!(values.len(), 2);
            }
        );
    }

    #[test]
    fn test_literals() {
        query!(br#"{}"#, "null",
            QueryResult::Owned(OwnedValue::Null) => {}
        );

        query!(br#"{}"#, "true",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br#"{}"#, "42",
            QueryResult::Owned(OwnedValue::Int(42)) => {}
        );

        query!(br#"{}"#, "\"hello\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "hello" => {}
        );
    }

    #[test]
    fn test_array_construction() {
        query!(br#"{"a": 1, "b": 2}"#, "[.a, .b]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(2));
            }
        );

        // Empty array
        query!(br#"{}"#, "[]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 0);
            }
        );
    }

    #[test]
    fn test_object_construction() {
        query!(br#"{"name": "Alice", "age": 30}"#, "{name: .name, years: .age}",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 2);
                assert!(obj.contains_key("name"));
                assert!(obj.contains_key("years"));
            }
        );

        // Empty object
        query!(br#"{}"#, "{}",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 0);
            }
        );
    }

    #[test]
    fn test_recursive_descent() {
        query!(br#"{"a": {"b": 1}}"#, "..",
            QueryResult::Many(values) => {
                // Should include: root object, "a" object, 1
                assert_eq!(values.len(), 3);
            }
        );
    }

    #[test]
    fn test_parentheses() {
        query!(br#"{"foo": {"bar": 1}}"#, "(.foo).bar",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    // Phase 2 tests: Arithmetic, Comparison, Boolean operators

    #[test]
    fn test_arithmetic_add() {
        // Number addition
        query!(br#"{"a": 10, "b": 5}"#, ".a + .b",
            QueryResult::Owned(OwnedValue::Int(15)) => {}
        );

        // Float addition
        query!(br#"{"a": 1.5, "b": 2.5}"#, ".a + .b",
            QueryResult::Owned(OwnedValue::Float(f)) if (f - 4.0).abs() < 0.001 => {}
        );

        // String concatenation
        query!(br#"{"a": "hello", "b": " world"}"#, ".a + .b",
            QueryResult::Owned(OwnedValue::String(s)) if s == "hello world" => {}
        );

        // Array concatenation
        query!(br#"{"a": [1, 2], "b": [3, 4]}"#, ".a + .b",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
            }
        );
    }

    #[test]
    fn test_arithmetic_sub() {
        query!(br#"{"a": 10, "b": 3}"#, ".a - .b",
            QueryResult::Owned(OwnedValue::Int(7)) => {}
        );
    }

    #[test]
    fn test_arithmetic_mul() {
        query!(br#"{"a": 6, "b": 7}"#, ".a * .b",
            QueryResult::Owned(OwnedValue::Int(42)) => {}
        );

        // String repetition
        query!(br#"{"s": "ab", "n": 3}"#, ".s * .n",
            QueryResult::Owned(OwnedValue::String(s)) if s == "ababab" => {}
        );
    }

    #[test]
    fn test_arithmetic_div() {
        query!(br#"{"a": 10, "b": 4}"#, ".a / .b",
            QueryResult::Owned(OwnedValue::Float(f)) if (f - 2.5).abs() < 0.001 => {}
        );

        // String split
        query!(br#"{"s": "a,b,c", "sep": ","}"#, ".s / .sep",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
            }
        );
    }

    #[test]
    fn test_arithmetic_mod() {
        query!(br#"{"a": 10, "b": 3}"#, ".a % .b",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );
    }

    #[test]
    fn test_arithmetic_precedence() {
        // 2 + 3 * 4 = 2 + 12 = 14
        query!(br#"{}"#, "2 + 3 * 4",
            QueryResult::Owned(OwnedValue::Int(14)) => {}
        );

        // (2 + 3) * 4 = 5 * 4 = 20
        query!(br#"{}"#, "(2 + 3) * 4",
            QueryResult::Owned(OwnedValue::Int(20)) => {}
        );
    }

    #[test]
    fn test_comparison_eq() {
        query!(br#"{"a": 1, "b": 1}"#, ".a == .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br#"{"a": 1, "b": 2}"#, ".a == .b",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        query!(br#"{"a": "foo", "b": "foo"}"#, ".a == .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_comparison_ne() {
        query!(br#"{"a": 1, "b": 2}"#, ".a != .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_comparison_lt() {
        query!(br#"{"a": 1, "b": 2}"#, ".a < .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br#"{"a": 2, "b": 1}"#, ".a < .b",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    #[test]
    fn test_comparison_le() {
        query!(br#"{"a": 1, "b": 1}"#, ".a <= .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_comparison_gt() {
        query!(br#"{"a": 2, "b": 1}"#, ".a > .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_comparison_ge() {
        query!(br#"{"a": 2, "b": 2}"#, ".a >= .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_boolean_and() {
        query!(br#"{"a": true, "b": true}"#, ".a and .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br#"{"a": true, "b": false}"#, ".a and .b",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // Short-circuit: if first is falsy, second is not evaluated
        query!(br#"{"a": false}"#, ".a and .nonexistent",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    #[test]
    fn test_boolean_or() {
        query!(br#"{"a": false, "b": true}"#, ".a or .b",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br#"{"a": false, "b": false}"#, ".a or .b",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // Short-circuit: if first is truthy, second is not evaluated
        query!(br#"{"a": true}"#, ".a or .nonexistent",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_boolean_not() {
        query!(br#"true"#, ". | not",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        query!(br#"false"#, ". | not",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br#"null"#, ". | not",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        // Numbers are truthy
        query!(br#"0"#, ". | not",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    #[test]
    fn test_alternative() {
        // Truthy value is returned
        query!(br#"{"a": 1}"#, ".a // 0",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );

        // Falsy value (null) uses alternative
        query!(br#"{"a": null}"#, ".a // 0",
            QueryResult::Owned(OwnedValue::Int(0)) => {}
        );

        // Missing value uses alternative
        query!(br#"{}"#, ".missing? // \"default\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "default" => {}
        );

        // Chain alternatives
        query!(br#"{"a": null, "b": null}"#, ".a // .b // 42",
            QueryResult::Owned(OwnedValue::Int(42)) => {}
        );
    }

    #[test]
    fn test_complex_expressions() {
        // Comparison with arithmetic
        query!(br#"{"x": 10}"#, ".x > 5 and .x < 20",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        // Alternative with comparison
        query!(br#"{"val": 3}"#, ".val > 0 // false",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }
}
