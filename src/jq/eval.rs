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

use super::expr::{ArithOp, Builtin, CompareOp, Expr, Literal, ObjectKey};
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
            let items: Vec<OwnedValue> = (*elements).map(|e| to_owned(&e)).collect();
            OwnedValue::Array(items)
        }
        StandardJson::Object(fields) => {
            let mut map = BTreeMap::new();
            for field in *fields {
                // Get the key as a string
                if let StandardJson::String(key_str_val) = field.key()
                    && let Ok(cow) = key_str_val.as_str()
                {
                    map.insert(cow.into_owned(), to_owned(&field.value()));
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

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => eval_if(cond, then_branch, else_branch, value, optional),

        Expr::Try { expr, catch } => eval_try(expr, catch.as_deref(), value, optional),

        Expr::Error(msg) => eval_error(msg.as_deref(), value, optional),

        Expr::Builtin(builtin) => eval_builtin(builtin, value, optional),
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
            for elem in *elements {
                collect_recursive(&elem, results);
            }
        }
        StandardJson::Object(fields) => {
            for field in *fields {
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

/// Evaluate if-then-else expression.
fn eval_if<'a, W: Clone + AsRef<[u64]>>(
    cond: &Expr,
    then_branch: &Expr,
    else_branch: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate condition
    let cond_result = eval_single(cond, value.clone(), optional);

    // Check if condition is truthy
    let is_truthy = match &cond_result {
        QueryResult::One(v) => to_owned(v).is_truthy(),
        QueryResult::Owned(v) => v.is_truthy(),
        QueryResult::Many(vs) => vs.first().map(|v| to_owned(v).is_truthy()).unwrap_or(false),
        QueryResult::ManyOwned(vs) => vs.first().map(|v| v.is_truthy()).unwrap_or(false),
        QueryResult::None => false,
        QueryResult::Error(e) => return QueryResult::Error(e.clone()),
    };

    if is_truthy {
        eval_single(then_branch, value, optional)
    } else {
        eval_single(else_branch, value, optional)
    }
}

/// Evaluate try-catch expression.
fn eval_try<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    catch: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the expression
    let result = eval_single(expr, value.clone(), optional);

    match result {
        // If error, use catch handler or return nothing
        QueryResult::Error(_) => match catch {
            Some(catch_expr) => eval_single(catch_expr, value, optional),
            None => QueryResult::None,
        },
        // Non-error results pass through
        other => other,
    }
}

/// Evaluate error expression.
fn eval_error<'a, W: Clone + AsRef<[u64]>>(
    msg: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let message = match msg {
        Some(msg_expr) => {
            let msg_result = eval_single(msg_expr, value, optional);
            match result_to_owned(msg_result) {
                Ok(OwnedValue::String(s)) => s,
                Ok(v) => v.to_json(),
                Err(e) => return QueryResult::Error(e),
            }
        }
        None => "null".into(),
    };

    QueryResult::Error(EvalError::new(message))
}

/// Evaluate a builtin function.
fn eval_builtin<'a, W: Clone + AsRef<[u64]>>(
    builtin: &Builtin,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match builtin {
        // Type functions
        Builtin::Type => {
            let type_name = match &value {
                StandardJson::Null => "null",
                StandardJson::Bool(_) => "boolean",
                StandardJson::Number(_) => "number",
                StandardJson::String(_) => "string",
                StandardJson::Array(_) => "array",
                StandardJson::Object(_) => "object",
                StandardJson::Error(_) => "error",
            };
            QueryResult::Owned(OwnedValue::String(type_name.into()))
        }
        Builtin::IsNull => {
            QueryResult::Owned(OwnedValue::Bool(matches!(value, StandardJson::Null)))
        }
        Builtin::IsBoolean => {
            QueryResult::Owned(OwnedValue::Bool(matches!(value, StandardJson::Bool(_))))
        }
        Builtin::IsNumber => {
            QueryResult::Owned(OwnedValue::Bool(matches!(value, StandardJson::Number(_))))
        }
        Builtin::IsString => {
            QueryResult::Owned(OwnedValue::Bool(matches!(value, StandardJson::String(_))))
        }
        Builtin::IsArray => {
            QueryResult::Owned(OwnedValue::Bool(matches!(value, StandardJson::Array(_))))
        }
        Builtin::IsObject => {
            QueryResult::Owned(OwnedValue::Bool(matches!(value, StandardJson::Object(_))))
        }

        // Length & Keys
        Builtin::Length => builtin_length(value, optional),
        Builtin::Utf8ByteLength => builtin_utf8bytelength(value, optional),
        Builtin::Keys => builtin_keys(value, optional, true),
        Builtin::KeysUnsorted => builtin_keys(value, optional, false),
        Builtin::Has(key_expr) => builtin_has(key_expr, value, optional),
        Builtin::In(obj_expr) => builtin_in(obj_expr, value, optional),

        // Selection & Filtering
        Builtin::Select(cond) => builtin_select(cond, value, optional),
        Builtin::Empty => QueryResult::None,

        // Map & Iteration
        Builtin::Map(f) => builtin_map(f, value, optional),
        Builtin::MapValues(f) => builtin_map_values(f, value, optional),

        // Reduction
        Builtin::Add => builtin_add(value, optional),
        Builtin::Any => builtin_any(value, optional),
        Builtin::All => builtin_all(value, optional),
        Builtin::Min => builtin_min(value, optional),
        Builtin::Max => builtin_max(value, optional),
        Builtin::MinBy(f) => builtin_min_by(f, value, optional),
        Builtin::MaxBy(f) => builtin_max_by(f, value, optional),

        // Phase 5: String Functions
        Builtin::AsciiDowncase => builtin_ascii_downcase(value, optional),
        Builtin::AsciiUpcase => builtin_ascii_upcase(value, optional),
        Builtin::Ltrimstr(s) => builtin_ltrimstr(s, value, optional),
        Builtin::Rtrimstr(s) => builtin_rtrimstr(s, value, optional),
        Builtin::Startswith(s) => builtin_startswith(s, value, optional),
        Builtin::Endswith(s) => builtin_endswith(s, value, optional),
        Builtin::Split(sep) => builtin_split(sep, value, optional),
        Builtin::Join(sep) => builtin_join(sep, value, optional),
        Builtin::Contains(b) => builtin_contains(b, value, optional),
        Builtin::Inside(b) => builtin_inside(b, value, optional),

        // Phase 5: Array Functions
        Builtin::First => builtin_first(value, optional),
        Builtin::Last => builtin_last(value, optional),
        Builtin::Nth(n) => builtin_nth(n, value, optional),
        Builtin::Reverse => builtin_reverse(value, optional),
        Builtin::Flatten => builtin_flatten(value, optional, 1),
        Builtin::FlattenDepth(depth) => builtin_flatten_depth(depth, value, optional),
        Builtin::GroupBy(f) => builtin_group_by(f, value, optional),
        Builtin::Unique => builtin_unique(value, optional),
        Builtin::UniqueBy(f) => builtin_unique_by(f, value, optional),
        Builtin::Sort => builtin_sort(value, optional),
        Builtin::SortBy(f) => builtin_sort_by(f, value, optional),

        // Phase 5: Object Functions
        Builtin::ToEntries => builtin_to_entries(value, optional),
        Builtin::FromEntries => builtin_from_entries(value, optional),
        Builtin::WithEntries(f) => builtin_with_entries(f, value, optional),
    }
}

/// Builtin: length
fn builtin_length<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::Null => QueryResult::Owned(OwnedValue::Int(0)),
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::Int(cow.chars().count() as i64))
            } else {
                QueryResult::Owned(OwnedValue::Int(0))
            }
        }
        StandardJson::Array(elements) => {
            QueryResult::Owned(OwnedValue::Int((*elements).count() as i64))
        }
        StandardJson::Object(fields) => {
            QueryResult::Owned(OwnedValue::Int((*fields).count() as i64))
        }
        StandardJson::Number(n) => {
            // Length of a number is its absolute value
            if let Ok(i) = n.as_i64() {
                QueryResult::Owned(OwnedValue::Int(i.abs()))
            } else if let Ok(f) = n.as_f64() {
                QueryResult::Owned(OwnedValue::Float(f.abs()))
            } else {
                QueryResult::Owned(OwnedValue::Int(0))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new(format!(
            "{} has no length",
            type_name(&value)
        ))),
    }
}

/// Builtin: utf8bytelength
fn builtin_utf8bytelength<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::Int(cow.len() as i64))
            } else {
                QueryResult::Owned(OwnedValue::Int(0))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: keys / keys_unsorted
fn builtin_keys<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
    sorted: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Object(fields) => {
            let mut keys: Vec<String> = Vec::new();
            for field in fields {
                if let StandardJson::String(k) = field.key()
                    && let Ok(cow) = k.as_str()
                {
                    keys.push(cow.into_owned());
                }
            }
            if sorted {
                keys.sort();
            }
            let arr: Vec<OwnedValue> = keys.into_iter().map(OwnedValue::String).collect();
            QueryResult::Owned(OwnedValue::Array(arr))
        }
        StandardJson::Array(elements) => {
            // For arrays, keys returns indices [0, 1, 2, ...]
            let len = elements.count();
            let arr: Vec<OwnedValue> = (0..len).map(|i| OwnedValue::Int(i as i64)).collect();
            QueryResult::Owned(OwnedValue::Array(arr))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("object or array", type_name(&value))),
    }
}

/// Builtin: has(key)
fn builtin_has<'a, W: Clone + AsRef<[u64]>>(
    key_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the key expression
    let key_result = eval_single(key_expr, value.clone(), optional);
    let key_owned = match result_to_owned(key_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    match (&value, &key_owned) {
        // Object has string key
        (StandardJson::Object(fields), OwnedValue::String(key)) => {
            let found = (*fields).clone().any(|f| {
                if let StandardJson::String(k) = f.key()
                    && let Ok(cow) = k.as_str()
                {
                    return cow.as_ref() == key;
                }
                false
            });
            QueryResult::Owned(OwnedValue::Bool(found))
        }
        // Array has index
        (StandardJson::Array(elements), OwnedValue::Int(idx)) => {
            let len = (*elements).count() as i64;
            let in_bounds = if *idx >= 0 {
                *idx < len
            } else {
                idx.abs() <= len
            };
            QueryResult::Owned(OwnedValue::Bool(in_bounds))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new(
            "has() requires object+string or array+number",
        )),
    }
}

/// Builtin: in(obj)
fn builtin_in<'a, W: Clone + AsRef<[u64]>>(
    obj_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // The input should be a key (string or number), and we check if it exists in obj
    let key_owned = to_owned(&value);
    let obj_result = eval_single(obj_expr, value.clone(), optional);

    // Get the object to check against
    let obj = match obj_result {
        QueryResult::One(o) => o,
        QueryResult::Many(os) => {
            if let Some(o) = os.into_iter().next() {
                o
            } else if optional {
                return QueryResult::None;
            } else {
                return QueryResult::Error(EvalError::new(
                    "in() requires an object or array argument",
                ));
            }
        }
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ if optional => return QueryResult::None,
        _ => {
            return QueryResult::Error(EvalError::new("in() requires an object or array argument"));
        }
    };

    match (&key_owned, &obj) {
        (OwnedValue::String(key), StandardJson::Object(fields)) => {
            let found = (*fields).clone().any(|f| {
                if let StandardJson::String(k) = f.key()
                    && let Ok(cow) = k.as_str()
                {
                    return cow.as_ref() == key;
                }
                false
            });
            QueryResult::Owned(OwnedValue::Bool(found))
        }
        (OwnedValue::Int(idx), StandardJson::Array(elements)) => {
            let len = (*elements).count() as i64;
            let in_bounds = if *idx >= 0 {
                *idx < len
            } else {
                idx.abs() <= len
            };
            QueryResult::Owned(OwnedValue::Bool(in_bounds))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new(
            "in() requires string/number key and object/array",
        )),
    }
}

/// Builtin: select(condition)
fn builtin_select<'a, W: Clone + AsRef<[u64]>>(
    cond: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate condition
    let cond_result = eval_single(cond, value.clone(), optional);

    // Check if condition is truthy
    let is_truthy = match &cond_result {
        QueryResult::One(v) => to_owned(v).is_truthy(),
        QueryResult::Owned(v) => v.is_truthy(),
        QueryResult::Many(vs) => vs.first().map(|v| to_owned(v).is_truthy()).unwrap_or(false),
        QueryResult::ManyOwned(vs) => vs.first().map(|v| v.is_truthy()).unwrap_or(false),
        QueryResult::None => false,
        QueryResult::Error(e) => return QueryResult::Error(e.clone()),
    };

    if is_truthy {
        QueryResult::One(value)
    } else {
        QueryResult::None
    }
}

/// Builtin: map(f)
fn builtin_map<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // map(f) is equivalent to [.[] | f]
    match value {
        StandardJson::Array(elements) => {
            let mut results = Vec::new();
            for elem in elements {
                match eval_single(f, elem, optional) {
                    QueryResult::One(v) => results.push(to_owned(&v)),
                    QueryResult::Owned(v) => results.push(v),
                    QueryResult::Many(vs) => results.extend(vs.iter().map(to_owned)),
                    QueryResult::ManyOwned(vs) => results.extend(vs),
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                }
            }
            QueryResult::Owned(OwnedValue::Array(results))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: map_values(f)
fn builtin_map_values<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Object(fields) => {
            let mut result_map = BTreeMap::new();
            for field in fields {
                // Get the key
                let key = if let StandardJson::String(k) = field.key() {
                    if let Ok(cow) = k.as_str() {
                        cow.into_owned()
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };

                // Apply f to the value
                let field_val = field.value();
                match eval_single(f, field_val, optional) {
                    QueryResult::One(v) => {
                        result_map.insert(key, to_owned(&v));
                    }
                    QueryResult::Owned(v) => {
                        result_map.insert(key, v);
                    }
                    QueryResult::Many(vs) => {
                        if let Some(v) = vs.first() {
                            result_map.insert(key, to_owned(v));
                        }
                    }
                    QueryResult::ManyOwned(vs) => {
                        if let Some(v) = vs.into_iter().next() {
                            result_map.insert(key, v);
                        }
                    }
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                }
            }
            QueryResult::Owned(OwnedValue::Object(result_map))
        }
        StandardJson::Array(elements) => {
            // map_values on array applies to each element
            let mut results = Vec::new();
            for elem in elements {
                match eval_single(f, elem, optional) {
                    QueryResult::One(v) => results.push(to_owned(&v)),
                    QueryResult::Owned(v) => results.push(v),
                    QueryResult::Many(vs) => {
                        if let Some(v) = vs.first() {
                            results.push(to_owned(v));
                        }
                    }
                    QueryResult::ManyOwned(vs) => {
                        if let Some(v) = vs.into_iter().next() {
                            results.push(v);
                        }
                    }
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                }
            }
            QueryResult::Owned(OwnedValue::Array(results))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("object or array", type_name(&value))),
    }
}

/// Builtin: add
fn builtin_add<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            if items.is_empty() {
                return QueryResult::Owned(OwnedValue::Null);
            }

            // Fold the items using addition
            let mut acc = items.into_iter();
            let first = acc.next().unwrap();
            let result = acc.try_fold(first, arith_add);

            match result {
                Ok(v) => QueryResult::Owned(v),
                Err(e) => QueryResult::Error(e),
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: any
fn builtin_any<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            for elem in elements {
                let owned = to_owned(&elem);
                if owned.is_truthy() {
                    return QueryResult::Owned(OwnedValue::Bool(true));
                }
            }
            QueryResult::Owned(OwnedValue::Bool(false))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: all
fn builtin_all<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            for elem in elements {
                let owned = to_owned(&elem);
                if !owned.is_truthy() {
                    return QueryResult::Owned(OwnedValue::Bool(false));
                }
            }
            QueryResult::Owned(OwnedValue::Bool(true))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: min
fn builtin_min<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            if items.is_empty() {
                return QueryResult::Owned(OwnedValue::Null);
            }

            let min = items.into_iter().min_by(compare_values).unwrap();
            QueryResult::Owned(min)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: max
fn builtin_max<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            if items.is_empty() {
                return QueryResult::Owned(OwnedValue::Null);
            }

            let max = items.into_iter().max_by(compare_values).unwrap();
            QueryResult::Owned(max)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: min_by(f)
fn builtin_min_by<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<StandardJson<'a, W>> = elements.collect();
            if items.is_empty() {
                return QueryResult::Owned(OwnedValue::Null);
            }

            // Compute keys for each item
            let mut keyed: Vec<(OwnedValue, StandardJson<'a, W>)> = Vec::new();
            for item in items {
                match eval_single(f, item.clone(), optional) {
                    QueryResult::One(v) => keyed.push((to_owned(&v), item)),
                    QueryResult::Owned(v) => keyed.push((v, item)),
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    _ => keyed.push((OwnedValue::Null, item)),
                }
            }

            let min = keyed
                .into_iter()
                .min_by(|(a, _), (b, _)| compare_values(a, b))
                .map(|(_, v)| to_owned(&v))
                .unwrap();
            QueryResult::Owned(min)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: max_by(f)
fn builtin_max_by<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<StandardJson<'a, W>> = elements.collect();
            if items.is_empty() {
                return QueryResult::Owned(OwnedValue::Null);
            }

            // Compute keys for each item
            let mut keyed: Vec<(OwnedValue, StandardJson<'a, W>)> = Vec::new();
            for item in items {
                match eval_single(f, item.clone(), optional) {
                    QueryResult::One(v) => keyed.push((to_owned(&v), item)),
                    QueryResult::Owned(v) => keyed.push((v, item)),
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    _ => keyed.push((OwnedValue::Null, item)),
                }
            }

            let max = keyed
                .into_iter()
                .max_by(|(a, _), (b, _)| compare_values(a, b))
                .map(|(_, v)| to_owned(&v))
                .unwrap();
            QueryResult::Owned(max)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

// =============================================================================
// Phase 5: String Functions
// =============================================================================

/// Builtin: ascii_downcase - lowercase ASCII characters
fn builtin_ascii_downcase<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let lowered: String = cow.chars().map(|c| c.to_ascii_lowercase()).collect();
                QueryResult::Owned(OwnedValue::String(lowered))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: ascii_upcase - uppercase ASCII characters
fn builtin_ascii_upcase<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let uppered: String = cow.chars().map(|c| c.to_ascii_uppercase()).collect();
                QueryResult::Owned(OwnedValue::String(uppered))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: ltrimstr(s) - remove prefix s
fn builtin_ltrimstr<'a, W: Clone + AsRef<[u64]>>(
    prefix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the prefix string
    let prefix_result = eval_single(prefix_expr, value.clone(), optional);
    let prefix = match result_to_owned(prefix_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "non-string")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let result = if cow.starts_with(&prefix) {
                    cow[prefix.len()..].to_string()
                } else {
                    cow.into_owned()
                };
                QueryResult::Owned(OwnedValue::String(result))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: rtrimstr(s) - remove suffix s
fn builtin_rtrimstr<'a, W: Clone + AsRef<[u64]>>(
    suffix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the suffix string
    let suffix_result = eval_single(suffix_expr, value.clone(), optional);
    let suffix = match result_to_owned(suffix_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "non-string")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let result = if cow.ends_with(&suffix) {
                    cow[..cow.len() - suffix.len()].to_string()
                } else {
                    cow.into_owned()
                };
                QueryResult::Owned(OwnedValue::String(result))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: startswith(s) - check if string starts with s
fn builtin_startswith<'a, W: Clone + AsRef<[u64]>>(
    prefix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the prefix string
    let prefix_result = eval_single(prefix_expr, value.clone(), optional);
    let prefix = match result_to_owned(prefix_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "non-string")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::Bool(cow.starts_with(&prefix)))
            } else {
                QueryResult::Owned(OwnedValue::Bool(false))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: endswith(s) - check if string ends with s
fn builtin_endswith<'a, W: Clone + AsRef<[u64]>>(
    suffix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the suffix string
    let suffix_result = eval_single(suffix_expr, value.clone(), optional);
    let suffix = match result_to_owned(suffix_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "non-string")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::Bool(cow.ends_with(&suffix)))
            } else {
                QueryResult::Owned(OwnedValue::Bool(false))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: split(s) - split string by separator
fn builtin_split<'a, W: Clone + AsRef<[u64]>>(
    sep_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the separator string
    let sep_result = eval_single(sep_expr, value.clone(), optional);
    let sep = match result_to_owned(sep_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "non-string")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let parts: Vec<OwnedValue> = cow
                    .split(&sep)
                    .map(|p| OwnedValue::String(p.to_string()))
                    .collect();
                QueryResult::Owned(OwnedValue::Array(parts))
            } else {
                QueryResult::Owned(OwnedValue::Array(vec![]))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: join(s) - join array elements with separator
fn builtin_join<'a, W: Clone + AsRef<[u64]>>(
    sep_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the separator string
    let sep_result = eval_single(sep_expr, value.clone(), optional);
    let sep = match result_to_owned(sep_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "non-string")),
        Err(e) => return QueryResult::Error(e),
    };

    match value {
        StandardJson::Array(elements) => {
            let mut parts: Vec<String> = Vec::new();
            for elem in elements {
                match &elem {
                    StandardJson::String(s) => {
                        if let Ok(cow) = s.as_str() {
                            parts.push(cow.into_owned());
                        }
                    }
                    StandardJson::Null => {
                        // Skip nulls in join
                    }
                    _ => {
                        // For non-strings, convert to string representation
                        parts.push(to_owned(&elem).to_json());
                    }
                }
            }
            QueryResult::Owned(OwnedValue::String(parts.join(&sep)))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: contains(b) - check if input contains b
fn builtin_contains<'a, W: Clone + AsRef<[u64]>>(
    b_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the value to check
    let b_result = eval_single(b_expr, value.clone(), optional);
    let b = match result_to_owned(b_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let input = to_owned(&value);
    QueryResult::Owned(OwnedValue::Bool(owned_contains(&input, &b)))
}

/// Check if `a` contains `b` (recursive containment check)
fn owned_contains(a: &OwnedValue, b: &OwnedValue) -> bool {
    match (a, b) {
        // String contains string
        (OwnedValue::String(a_str), OwnedValue::String(b_str)) => a_str.contains(b_str.as_str()),
        // Array contains: all elements of b must be contained in a
        (OwnedValue::Array(a_arr), OwnedValue::Array(b_arr)) => b_arr
            .iter()
            .all(|b_elem| a_arr.iter().any(|a_elem| owned_contains(a_elem, b_elem))),
        // Object contains: all keys in b must exist in a with matching values
        (OwnedValue::Object(a_obj), OwnedValue::Object(b_obj)) => b_obj.iter().all(|(k, b_val)| {
            a_obj
                .get(k)
                .is_some_and(|a_val| owned_contains(a_val, b_val))
        }),
        // Scalars: equality
        _ => a == b,
    }
}

/// Builtin: inside(b) - check if input is inside b
fn builtin_inside<'a, W: Clone + AsRef<[u64]>>(
    b_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the container value
    let b_result = eval_single(b_expr, value.clone(), optional);
    let b = match result_to_owned(b_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let input = to_owned(&value);
    // inside is the inverse of contains: b contains input
    QueryResult::Owned(OwnedValue::Bool(owned_contains(&b, &input)))
}

// =============================================================================
// Phase 5: Array Functions
// =============================================================================

/// Builtin: first - first element (.[0])
fn builtin_first<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => match elements.get(0) {
            Some(v) => QueryResult::One(v),
            None if optional => QueryResult::None,
            None => QueryResult::Error(EvalError::new("array is empty")),
        },
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: last - last element (.[-1])
fn builtin_last<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<_> = elements.collect();
            if items.is_empty() {
                if optional {
                    QueryResult::None
                } else {
                    QueryResult::Error(EvalError::new("array is empty"))
                }
            } else {
                QueryResult::Owned(to_owned(&items[items.len() - 1]))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: nth(n) - nth element
fn builtin_nth<'a, W: Clone + AsRef<[u64]>>(
    n_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the index
    let n_result = eval_single(n_expr, value.clone(), optional);
    let n = match result_to_owned(n_result) {
        Ok(OwnedValue::Int(i)) => i,
        Ok(_) => return QueryResult::Error(EvalError::type_error("number", "non-number")),
        Err(e) => return QueryResult::Error(e),
    };

    match value {
        StandardJson::Array(elements) => match get_element_at_index(elements, n) {
            Some(v) => QueryResult::One(v),
            None if optional => QueryResult::None,
            None => QueryResult::Error(EvalError::new(format!("index {} out of bounds", n))),
        },
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: reverse - reverse array
fn builtin_reverse<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let mut items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            items.reverse();
            QueryResult::Owned(OwnedValue::Array(items))
        }
        StandardJson::String(s) => {
            // reverse also works on strings
            if let Ok(cow) = s.as_str() {
                let reversed: String = cow.chars().rev().collect();
                QueryResult::Owned(OwnedValue::String(reversed))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array or string", type_name(&value))),
    }
}

/// Builtin: flatten - flatten nested arrays (1 level)
fn builtin_flatten<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
    depth: usize,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            let flattened = flatten_owned(items, depth);
            QueryResult::Owned(OwnedValue::Array(flattened))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Flatten owned values to a specific depth
fn flatten_owned(items: Vec<OwnedValue>, depth: usize) -> Vec<OwnedValue> {
    if depth == 0 {
        return items;
    }

    let mut result = Vec::new();
    for item in items {
        match item {
            OwnedValue::Array(inner) => {
                result.extend(flatten_owned(inner, depth - 1));
            }
            other => result.push(other),
        }
    }
    result
}

/// Builtin: flatten(depth) - flatten to specific depth
fn builtin_flatten_depth<'a, W: Clone + AsRef<[u64]>>(
    depth_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the depth
    let depth_result = eval_single(depth_expr, value.clone(), optional);
    let depth = match result_to_owned(depth_result) {
        Ok(OwnedValue::Int(d)) if d >= 0 => d as usize,
        Ok(OwnedValue::Int(_)) => {
            return QueryResult::Error(EvalError::new("depth must be non-negative"));
        }
        Ok(_) => return QueryResult::Error(EvalError::type_error("number", "non-number")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_flatten(value, optional, depth)
}

/// Builtin: group_by(f) - group by key function
fn builtin_group_by<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<StandardJson<'a, W>> = elements.collect();

            // Compute keys for each item
            let mut keyed: Vec<(OwnedValue, OwnedValue)> = Vec::new();
            for item in items {
                let key = match eval_single(f, item.clone(), optional) {
                    QueryResult::One(v) => to_owned(&v),
                    QueryResult::Owned(v) => v,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    _ => OwnedValue::Null,
                };
                keyed.push((key, to_owned(&item)));
            }

            // Sort by key
            keyed.sort_by(|(a, _), (b, _)| compare_values(a, b));

            // Group consecutive items with same key
            let mut groups: Vec<OwnedValue> = Vec::new();
            let mut current_group: Vec<OwnedValue> = Vec::new();
            let mut current_key: Option<OwnedValue> = None;

            for (key, item) in keyed {
                match &current_key {
                    Some(k) if compare_values(k, &key) == core::cmp::Ordering::Equal => {
                        current_group.push(item);
                    }
                    _ => {
                        if !current_group.is_empty() {
                            groups.push(OwnedValue::Array(current_group));
                        }
                        current_group = vec![item];
                        current_key = Some(key);
                    }
                }
            }
            if !current_group.is_empty() {
                groups.push(OwnedValue::Array(current_group));
            }

            QueryResult::Owned(OwnedValue::Array(groups))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: unique - remove duplicates
fn builtin_unique<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let mut items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();

            // Sort first (jq's unique returns sorted unique values)
            items.sort_by(compare_values);

            // Remove consecutive duplicates
            items.dedup_by(|a, b| compare_values(a, b) == core::cmp::Ordering::Equal);

            QueryResult::Owned(OwnedValue::Array(items))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: unique_by(f) - remove duplicates by key
fn builtin_unique_by<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<StandardJson<'a, W>> = elements.collect();

            // Compute keys for each item
            let mut keyed: Vec<(OwnedValue, OwnedValue)> = Vec::new();
            for item in items {
                let key = match eval_single(f, item.clone(), optional) {
                    QueryResult::One(v) => to_owned(&v),
                    QueryResult::Owned(v) => v,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    _ => OwnedValue::Null,
                };
                keyed.push((key, to_owned(&item)));
            }

            // Sort by key
            keyed.sort_by(|(a, _), (b, _)| compare_values(a, b));

            // Remove consecutive duplicates by key
            keyed.dedup_by(|(a, _), (b, _)| compare_values(a, b) == core::cmp::Ordering::Equal);

            let result: Vec<OwnedValue> = keyed.into_iter().map(|(_, v)| v).collect();
            QueryResult::Owned(OwnedValue::Array(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: sort - sort array
fn builtin_sort<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let mut items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            items.sort_by(compare_values);
            QueryResult::Owned(OwnedValue::Array(items))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: sort_by(f) - sort by key function
fn builtin_sort_by<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<StandardJson<'a, W>> = elements.collect();

            // Compute keys for each item
            let mut keyed: Vec<(OwnedValue, OwnedValue)> = Vec::new();
            for item in items {
                let key = match eval_single(f, item.clone(), optional) {
                    QueryResult::One(v) => to_owned(&v),
                    QueryResult::Owned(v) => v,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    _ => OwnedValue::Null,
                };
                keyed.push((key, to_owned(&item)));
            }

            // Sort by key
            keyed.sort_by(|(a, _), (b, _)| compare_values(a, b));

            let result: Vec<OwnedValue> = keyed.into_iter().map(|(_, v)| v).collect();
            QueryResult::Owned(OwnedValue::Array(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

// =============================================================================
// Phase 5: Object Functions
// =============================================================================

/// Builtin: to_entries - {k:v} → [{key:k, value:v}]
fn builtin_to_entries<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Object(fields) => {
            let mut entries: Vec<OwnedValue> = Vec::new();
            for field in fields {
                let key = if let StandardJson::String(k) = field.key()
                    && let Ok(cow) = k.as_str()
                {
                    cow.into_owned()
                } else {
                    continue;
                };
                let val = to_owned(&field.value());

                let mut entry = BTreeMap::new();
                entry.insert("key".to_string(), OwnedValue::String(key));
                entry.insert("value".to_string(), val);
                entries.push(OwnedValue::Object(entry));
            }
            QueryResult::Owned(OwnedValue::Array(entries))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("object", type_name(&value))),
    }
}

/// Builtin: from_entries - [{key:k, value:v}] → {k:v}
fn builtin_from_entries<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Array(elements) => {
            let mut result = BTreeMap::new();

            for elem in elements {
                // Each element should be an object with "key"/"name" and "value" fields
                if let StandardJson::Object(fields) = elem {
                    let owned = to_owned(&StandardJson::Object(fields));
                    if let OwnedValue::Object(obj) = owned {
                        // Try "key" first, then "name", then "k"
                        let key = obj
                            .get("key")
                            .or_else(|| obj.get("name"))
                            .or_else(|| obj.get("k"));

                        // Try "value" first, then "v"
                        let val = obj
                            .get("value")
                            .or_else(|| obj.get("v"))
                            .cloned()
                            .unwrap_or(OwnedValue::Null);

                        if let Some(OwnedValue::String(k)) = key {
                            result.insert(k.clone(), val);
                        }
                    }
                }
            }

            QueryResult::Owned(OwnedValue::Object(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: with_entries(f) - to_entries | map(f) | from_entries
fn builtin_with_entries<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Object(fields) => {
            // Convert to entries
            let mut entries: Vec<OwnedValue> = Vec::new();
            for field in fields {
                let key = if let StandardJson::String(k) = field.key()
                    && let Ok(cow) = k.as_str()
                {
                    cow.into_owned()
                } else {
                    continue;
                };
                let val = to_owned(&field.value());

                let mut entry = BTreeMap::new();
                entry.insert("key".to_string(), OwnedValue::String(key));
                entry.insert("value".to_string(), val);
                entries.push(OwnedValue::Object(entry));
            }

            // Apply f to each entry
            let mut transformed: Vec<OwnedValue> = Vec::new();
            for entry in entries {
                let entry_json = owned_to_json_bytes(&entry);
                let index = crate::json::JsonIndex::build(&entry_json);
                let cursor = index.root(&entry_json);

                match eval_single(f, cursor.value(), optional) {
                    QueryResult::One(v) => transformed.push(to_owned(&v)),
                    QueryResult::Owned(v) => transformed.push(v),
                    QueryResult::Many(vs) => {
                        for v in vs {
                            transformed.push(to_owned(&v));
                        }
                    }
                    QueryResult::ManyOwned(vs) => transformed.extend(vs),
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                }
            }

            // Convert back from entries
            let mut result = BTreeMap::new();
            for entry in transformed {
                if let OwnedValue::Object(obj) = entry {
                    let key = obj
                        .get("key")
                        .or_else(|| obj.get("name"))
                        .or_else(|| obj.get("k"));

                    let val = obj
                        .get("value")
                        .or_else(|| obj.get("v"))
                        .cloned()
                        .unwrap_or(OwnedValue::Null);

                    if let Some(OwnedValue::String(k)) = key {
                        result.insert(k.clone(), val);
                    }
                }
            }

            QueryResult::Owned(OwnedValue::Object(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("object", type_name(&value))),
    }
}

/// Convert an OwnedValue to JSON bytes for re-parsing
fn owned_to_json_bytes(value: &OwnedValue) -> Vec<u8> {
    value.to_json().into_bytes()
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

    // Phase 3 tests: Conditionals and Control Flow

    #[test]
    fn test_if_then_else() {
        // Basic if-then-else: true condition
        query!(br#"{"a": true}"#, "if .a then 1 else 2 end",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );

        // Basic if-then-else: false condition
        query!(br#"{"a": false}"#, "if .a then 1 else 2 end",
            QueryResult::Owned(OwnedValue::Int(2)) => {}
        );

        // If with comparison condition
        query!(br#"{"x": 10}"#, "if .x > 5 then \"big\" else \"small\" end",
            QueryResult::Owned(OwnedValue::String(s)) if s == "big" => {}
        );

        // If with null condition (falsy)
        query!(br#"{"a": null}"#, "if .a then 1 else 2 end",
            QueryResult::Owned(OwnedValue::Int(2)) => {}
        );

        // If with number condition (truthy, even 0)
        query!(br#"{"a": 0}"#, "if .a then 1 else 2 end",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );
    }

    #[test]
    fn test_if_elif() {
        // if-elif-else with first condition true
        query!(br#"{"x": 1}"#, "if .x == 1 then \"one\" elif .x == 2 then \"two\" else \"other\" end",
            QueryResult::Owned(OwnedValue::String(s)) if s == "one" => {}
        );

        // if-elif-else with second condition true
        query!(br#"{"x": 2}"#, "if .x == 1 then \"one\" elif .x == 2 then \"two\" else \"other\" end",
            QueryResult::Owned(OwnedValue::String(s)) if s == "two" => {}
        );

        // if-elif-else with else branch
        query!(br#"{"x": 3}"#, "if .x == 1 then \"one\" elif .x == 2 then \"two\" else \"other\" end",
            QueryResult::Owned(OwnedValue::String(s)) if s == "other" => {}
        );
    }

    #[test]
    fn test_if_no_else() {
        // if without else (defaults to null)
        query!(br#"{"a": false}"#, "if .a then 1 end",
            QueryResult::Owned(OwnedValue::Null) => {}
        );

        query!(br#"{"a": true}"#, "if .a then 1 end",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );
    }

    #[test]
    fn test_if_with_expressions() {
        // if with arithmetic in branches
        query!(br#"{"x": 5}"#, "if .x > 0 then .x * 2 else .x end",
            QueryResult::Owned(OwnedValue::Int(10)) => {}
        );

        // if with field access
        query!(br#"{"type": "a", "a": 1, "b": 2}"#, "if .type == \"a\" then .a else .b end",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    #[test]
    fn test_try_catch_success() {
        // try with no error - returns result
        query!(br#"{"a": 1}"#, "try .a catch 0",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );

        // try without catch, no error
        query!(br#"{"a": 1}"#, "try .a",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    #[test]
    fn test_try_catch_error() {
        // try with catch - error is caught
        query!(br#"{}"#, "try .missing catch \"default\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "default" => {}
        );

        // try without catch - error is suppressed (returns None)
        query!(br#"{}"#, "try .missing",
            QueryResult::None => {}
        );

        // try with null catch
        query!(br#"{}"#, "try .missing catch null",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_try_catch_optional() {
        // Optional inside try - optional suppresses the error first
        query!(br#"{}"#, "try .missing? catch \"default\"",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_error_basic() {
        // error without message
        query!(br#"{}"#, "error",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "null");
            }
        );

        // error with string message
        query!(br#"{}"#, "error(\"something went wrong\")",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "something went wrong");
            }
        );

        // error with number message (gets serialized)
        query!(br#"{}"#, "error(42)",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "42");
            }
        );
    }

    #[test]
    fn test_error_with_field() {
        // error with field access for message
        query!(br#"{"msg": "custom error"}"#, "error(.msg)",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "custom error");
            }
        );
    }

    #[test]
    fn test_try_catch_raised_error() {
        // try-catch with error - error is caught
        query!(br#"{}"#, "try error(\"oops\") catch \"caught\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "caught" => {}
        );

        // try without catch - error is suppressed
        query!(br#"{}"#, "try error(\"oops\")",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_control_flow_combinations() {
        // if inside try
        query!(br#"{"a": true}"#, "try (if .a then .missing else .a end) catch \"error\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "error" => {}
        );

        // try inside if
        query!(br#"{"a": true}"#, "if .a then try .missing catch 0 else 1 end",
            QueryResult::Owned(OwnedValue::Int(0)) => {}
        );

        // Nested if with arithmetic
        query!(br#"{"x": 15}"#, "if .x < 10 then \"small\" elif .x < 20 then \"medium\" else \"large\" end",
            QueryResult::Owned(OwnedValue::String(s)) if s == "medium" => {}
        );
    }

    // Phase 4 tests: Core Builtin Functions

    #[test]
    fn test_builtin_type() {
        query!(br#"null"#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "null" => {}
        );
        query!(br#"true"#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "boolean" => {}
        );
        query!(br#"42"#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "number" => {}
        );
        query!(br#""hello""#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "string" => {}
        );
        query!(br#"[1, 2]"#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "array" => {}
        );
        query!(br#"{"a": 1}"#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "object" => {}
        );
    }

    #[test]
    fn test_builtin_is_type() {
        // isnull
        query!(br#"null"#, "isnull",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"1"#, "isnull",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isboolean
        query!(br#"true"#, "isboolean",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"1"#, "isboolean",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isnumber
        query!(br#"42"#, "isnumber",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#""42""#, "isnumber",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isstring
        query!(br#""hello""#, "isstring",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"42"#, "isstring",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isarray
        query!(br#"[1, 2]"#, "isarray",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"{}"#, "isarray",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isobject
        query!(br#"{"a": 1}"#, "isobject",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"[]"#, "isobject",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    #[test]
    fn test_builtin_length() {
        // null has length 0
        query!(br#"null"#, "length",
            QueryResult::Owned(OwnedValue::Int(0)) => {}
        );

        // string length (characters)
        query!(br#""hello""#, "length",
            QueryResult::Owned(OwnedValue::Int(5)) => {}
        );

        // unicode string length - use escaped UTF-8 for é (c3 a9)
        query!(b"\"h\\u00e9llo\"", "length",
            QueryResult::Owned(OwnedValue::Int(5)) => {}
        );

        // array length
        query!(br#"[1, 2, 3]"#, "length",
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );

        // object length (key count)
        query!(br#"{"a": 1, "b": 2}"#, "length",
            QueryResult::Owned(OwnedValue::Int(2)) => {}
        );

        // number length is absolute value
        query!(br#"-5"#, "length",
            QueryResult::Owned(OwnedValue::Int(5)) => {}
        );
    }

    #[test]
    fn test_builtin_utf8bytelength() {
        query!(br#""hello""#, "utf8bytelength",
            QueryResult::Owned(OwnedValue::Int(5)) => {}
        );

        // Unicode string - use escaped UTF-8 for é
        query!(b"\"h\\u00e9llo\"", "utf8bytelength",
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );
    }

    #[test]
    fn test_builtin_keys() {
        // Object keys (sorted)
        query!(br#"{"b": 2, "a": 1, "c": 3}"#, "keys",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::String("a".into()));
                assert_eq!(arr[1], OwnedValue::String("b".into()));
                assert_eq!(arr[2], OwnedValue::String("c".into()));
            }
        );

        // Array keys (indices)
        query!(br#"["x", "y", "z"]"#, "keys",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(0));
                assert_eq!(arr[1], OwnedValue::Int(1));
                assert_eq!(arr[2], OwnedValue::Int(2));
            }
        );
    }

    #[test]
    fn test_builtin_keys_unsorted() {
        // keys_unsorted preserves original order
        query!(br#"{"b": 2, "a": 1}"#, "keys_unsorted",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                // Note: Order depends on how JSON was parsed
            }
        );
    }

    #[test]
    fn test_builtin_has() {
        // Object has key
        query!(br#"{"a": 1, "b": 2}"#, "has(\"a\")",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"{"a": 1, "b": 2}"#, "has(\"c\")",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // Array has index
        query!(br#"[1, 2, 3]"#, "has(0)",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"[1, 2, 3]"#, "has(5)",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    #[test]
    fn test_builtin_in() {
        // in() checks if a key/index exists
        // Note: in() with piped owned values requires fixing eval_pipe
        // For now, test has() which works similarly
        query!(br#"{"a": 1, "b": 2}"#, "has(\"a\")",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_builtin_select() {
        // select outputs input only if condition is true
        query!(br#"5"#, "select(. > 3)",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 5);
            }
        );

        // select outputs nothing if condition is false
        query!(br#"2"#, "select(. > 3)",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_builtin_empty() {
        query!(br#"1"#, "empty",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_builtin_map() {
        // map applies function to each element
        query!(br#"[1, 2, 3]"#, "map(. * 2)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(2));
                assert_eq!(arr[1], OwnedValue::Int(4));
                assert_eq!(arr[2], OwnedValue::Int(6));
            }
        );

        // map with type check
        query!(br#"[1, 2, 3]"#, "map(. + 1)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr[0], OwnedValue::Int(2));
            }
        );
    }

    #[test]
    fn test_builtin_map_values() {
        // map_values on object
        query!(br#"{"a": 1, "b": 2}"#, "map_values(. * 10)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(10)));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(20)));
            }
        );

        // map_values on array
        query!(br#"[1, 2, 3]"#, "map_values(. + 1)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr[0], OwnedValue::Int(2));
                assert_eq!(arr[1], OwnedValue::Int(3));
                assert_eq!(arr[2], OwnedValue::Int(4));
            }
        );
    }

    #[test]
    fn test_builtin_add() {
        // Add numbers
        query!(br#"[1, 2, 3]"#, "add",
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );

        // Add strings
        query!(br#"["a", "b", "c"]"#, "add",
            QueryResult::Owned(OwnedValue::String(s)) if s == "abc" => {}
        );

        // Add arrays
        query!(br#"[[1], [2], [3]]"#, "add",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
            }
        );

        // Empty array returns null
        query!(br#"[]"#, "add",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_builtin_any() {
        query!(br#"[true, false]"#, "any",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"[false, false]"#, "any",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(br#"[null, null]"#, "any",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(br#"[1, 0]"#, "any",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}  // numbers are truthy
        );
    }

    #[test]
    fn test_builtin_all() {
        query!(br#"[true, true]"#, "all",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#"[true, false]"#, "all",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(br#"[1, 2, 3]"#, "all",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}  // numbers are truthy
        );
    }

    #[test]
    fn test_builtin_min() {
        query!(br#"[3, 1, 2]"#, "min",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );
        query!(br#"["c", "a", "b"]"#, "min",
            QueryResult::Owned(OwnedValue::String(s)) if s == "a" => {}
        );
        query!(br#"[]"#, "min",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_builtin_max() {
        query!(br#"[3, 1, 2]"#, "max",
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );
        query!(br#"["c", "a", "b"]"#, "max",
            QueryResult::Owned(OwnedValue::String(s)) if s == "c" => {}
        );
        query!(br#"[]"#, "max",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_builtin_min_by() {
        query!(br#"[{"a": 3}, {"a": 1}, {"a": 2}]"#, "min_by(.a)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
            }
        );
    }

    #[test]
    fn test_builtin_max_by() {
        query!(br#"[{"a": 3}, {"a": 1}, {"a": 2}]"#, "max_by(.a)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(3)));
            }
        );
    }

    #[test]
    fn test_builtin_combinations() {
        // map alone works
        query!(br#"[1, 2, 3, 4, 5]"#, "map(. * 2)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 5);
                assert_eq!(arr[0], OwnedValue::Int(2));
            }
        );

        // Use select in map
        query!(br#"[1, 2, 3, 4, 5]"#, "[.[] | select(. > 2)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
            }
        );

        // keys alone works
        query!(br#"{"a": 1, "b": 2}"#, "keys",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
            }
        );

        // Note: Piping owned values (e.g., "map(...) | add" or "keys | map(...)")
        // requires fixing eval_pipe to handle owned values, which is deferred.
    }

    // ==========================================================================
    // Phase 5: String Functions Tests
    // ==========================================================================

    #[test]
    fn test_builtin_ascii_downcase() {
        query!(br#""HELLO World""#, "ascii_downcase",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello world");
            }
        );

        // Non-ASCII characters should be unchanged
        query!(b"\"H\\u00e9LLO\"", "ascii_downcase",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "h\u{00e9}llo");
            }
        );
    }

    #[test]
    fn test_builtin_ascii_upcase() {
        query!(br#""hello World""#, "ascii_upcase",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "HELLO WORLD");
            }
        );
    }

    #[test]
    fn test_builtin_ltrimstr() {
        query!(br#""hello world""#, r#"ltrimstr("hello ")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "world");
            }
        );

        // No match - returns original
        query!(br#""hello world""#, r#"ltrimstr("goodbye")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello world");
            }
        );
    }

    #[test]
    fn test_builtin_rtrimstr() {
        query!(br#""hello world""#, r#"rtrimstr(" world")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello");
            }
        );

        // No match - returns original
        query!(br#""hello world""#, r#"rtrimstr("goodbye")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello world");
            }
        );
    }

    #[test]
    fn test_builtin_startswith() {
        query!(br#""hello world""#, r#"startswith("hello")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        query!(br#""hello world""#, r#"startswith("world")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
    }

    #[test]
    fn test_builtin_endswith() {
        query!(br#""hello world""#, r#"endswith("world")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        query!(br#""hello world""#, r#"endswith("hello")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
    }

    #[test]
    fn test_builtin_split() {
        query!(br#""a,b,c""#, r#"split(",")"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::String("a".into()));
                assert_eq!(arr[1], OwnedValue::String("b".into()));
                assert_eq!(arr[2], OwnedValue::String("c".into()));
            }
        );
    }

    #[test]
    fn test_builtin_join() {
        query!(br#"["a", "b", "c"]"#, r#"join(",")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "a,b,c");
            }
        );

        // Join with null values (should be skipped)
        query!(br#"["a", null, "c"]"#, r#"join("-")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "a-c");
            }
        );
    }

    #[test]
    fn test_builtin_contains() {
        // String contains
        query!(br#""hello world""#, r#"contains("world")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        // Array contains
        query!(br#"[1, 2, 3]"#, r#"contains([2])"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        // Object contains
        query!(br#"{"a": 1, "b": 2}"#, r#"contains({"a": 1})"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );
    }

    #[test]
    fn test_builtin_inside() {
        // inside is the inverse of contains
        query!(br#"[2]"#, r#"inside([1, 2, 3])"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        query!(br#"{"a": 1}"#, r#"inside({"a": 1, "b": 2})"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );
    }

    // ==========================================================================
    // Phase 5: Array Functions Tests
    // ==========================================================================

    #[test]
    fn test_builtin_first() {
        query!(br#"[1, 2, 3]"#, "first",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    #[test]
    fn test_builtin_last() {
        query!(br#"[1, 2, 3]"#, "last",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 3);
            }
        );
    }

    #[test]
    fn test_builtin_nth() {
        query!(br#"[10, 20, 30]"#, "nth(1)",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 20);
            }
        );
    }

    #[test]
    fn test_builtin_reverse() {
        query!(br#"[1, 2, 3]"#, "reverse",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(3), OwnedValue::Int(2), OwnedValue::Int(1)]);
            }
        );

        // Reverse also works on strings
        query!(br#""hello""#, "reverse",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "olleh");
            }
        );
    }

    #[test]
    fn test_builtin_flatten() {
        query!(br#"[[1, 2], [3, [4]]]"#, "flatten",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[3], OwnedValue::Array(vec![OwnedValue::Int(4)]));
            }
        );

        // Flatten with depth
        query!(br#"[[1], [[2]], [[[3]]]]"#, "flatten(2)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(2));
                assert_eq!(arr[2], OwnedValue::Array(vec![OwnedValue::Int(3)]));
            }
        );
    }

    #[test]
    fn test_builtin_group_by() {
        query!(br#"[{"a": 1}, {"a": 2}, {"a": 1}]"#, "group_by(.a)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // Should group by .a value
                assert_eq!(arr.len(), 2);
            }
        );
    }

    #[test]
    fn test_builtin_unique() {
        query!(br#"[1, 2, 1, 3, 2]"#, "unique",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(1), OwnedValue::Int(2), OwnedValue::Int(3)]);
            }
        );
    }

    #[test]
    fn test_builtin_unique_by() {
        query!(br#"[{"a": 1, "b": 1}, {"a": 1, "b": 2}, {"a": 2, "b": 3}]"#, "unique_by(.a)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
            }
        );
    }

    #[test]
    fn test_builtin_sort() {
        query!(br#"[3, 1, 2]"#, "sort",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(1), OwnedValue::Int(2), OwnedValue::Int(3)]);
            }
        );
    }

    #[test]
    fn test_builtin_sort_by() {
        query!(br#"[{"a": 3}, {"a": 1}, {"a": 2}]"#, "sort_by(.a)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                // First element should have a=1
                if let OwnedValue::Object(obj) = &arr[0] {
                    assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
                }
            }
        );
    }

    // ==========================================================================
    // Phase 5: Object Functions Tests
    // ==========================================================================

    #[test]
    fn test_builtin_to_entries() {
        query!(br#"{"a": 1, "b": 2}"#, "to_entries",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                // Each entry should have "key" and "value" fields
                if let OwnedValue::Object(obj) = &arr[0] {
                    assert!(obj.contains_key("key"));
                    assert!(obj.contains_key("value"));
                }
            }
        );
    }

    #[test]
    fn test_builtin_from_entries() {
        query!(br#"[{"key": "a", "value": 1}, {"key": "b", "value": 2}]"#, "from_entries",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(2)));
            }
        );

        // Also supports "name" instead of "key"
        query!(br#"[{"name": "x", "value": 10}]"#, "from_entries",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("x"), Some(&OwnedValue::Int(10)));
            }
        );
    }

    #[test]
    fn test_builtin_with_entries() {
        // Simple transformation - just pass through
        // (Assignment syntax `.value = x` is not supported yet,
        //  so we test a simple transformation using object construction)
        query!(br#"{"a": 1, "b": 2}"#, "with_entries({key: .key, value: .value})",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(2)));
            }
        );
    }
}
