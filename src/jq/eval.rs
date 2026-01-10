//! Expression evaluator for jq-like queries.
//!
//! Evaluates expressions against JSON using the cursor-based navigation API.

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use indexmap::IndexMap;

use crate::json::light::{JsonCursor, JsonElements, JsonFields, StandardJson};

use super::expr::{
    ArithOp, Builtin, CompareOp, Expr, FormatType, Literal, ObjectEntry, ObjectKey, Pattern,
    StringPart,
};
use super::value::OwnedValue;

/// Result of evaluating a jq expression.
#[derive(Debug)]
pub enum QueryResult<'a, W = Vec<u64>> {
    /// Single value result (reference to original JSON).
    One(StandardJson<'a, W>),

    /// Single cursor result (for unchanged container values).
    ///
    /// This is more efficient than `One` for arrays/objects because
    /// it preserves the cursor, allowing direct output of raw bytes
    /// without decomposing into individual element cursors.
    OneCursor(JsonCursor<'a, W>),

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

impl<'a, W: Clone + AsRef<[u64]>> QueryResult<'a, W> {
    /// Convert OneCursor to One by materializing the cursor value.
    /// Used internally when we need StandardJson from a result.
    #[inline]
    fn materialize_cursor(self) -> Self {
        match self {
            QueryResult::OneCursor(c) => QueryResult::One(c.value()),
            other => other,
        }
    }
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
            let mut map = IndexMap::new();
            for field in *fields {
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

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => eval_if(cond, then_branch, else_branch, value, optional),

        Expr::Try { expr, catch } => eval_try(expr, catch.as_deref(), value, optional),

        Expr::Error(msg) => eval_error(msg.as_deref(), value, optional),

        Expr::Builtin(builtin) => eval_builtin(builtin, value, optional),

        Expr::StringInterpolation(parts) => eval_string_interpolation(parts, value, optional),

        Expr::Format(format_type) => eval_format(*format_type, value, optional),

        // Phase 8: Variables and Advanced Control Flow
        Expr::As { expr, var, body } => eval_as(expr, var, body, value, optional),
        Expr::Var(name) => {
            // Variable references without context should error
            // In practice, variables are resolved by eval_as which substitutes them
            QueryResult::Error(EvalError::new(format!("undefined variable: ${}", name)))
        }
        Expr::Reduce {
            input,
            var,
            init,
            update,
        } => eval_reduce(input, var, init, update, value, optional),
        Expr::Foreach {
            input,
            var,
            init,
            update,
            extract,
        } => eval_foreach(
            input,
            var,
            init,
            update,
            extract.as_deref(),
            value,
            optional,
        ),
        Expr::Limit { n, expr } => eval_limit(n, expr, value, optional),
        Expr::FirstExpr(expr) => eval_first_expr(expr, value, optional),
        Expr::LastExpr(expr) => eval_last_expr(expr, value, optional),
        Expr::NthExpr { n, expr } => eval_nth_expr(n, expr, value, optional),
        Expr::Until { cond, update } => eval_until(cond, update, value, optional),
        Expr::While { cond, update } => eval_while(cond, update, value, optional),
        Expr::Repeat(expr) => eval_repeat(expr, value, optional),
        Expr::Range { from, to, step } => {
            eval_range(from, to.as_deref(), step.as_deref(), value, optional)
        }

        // Phase 9: Variables & Definitions
        Expr::AsPattern {
            expr,
            pattern,
            body,
        } => eval_as_pattern(expr, pattern, body, value, optional),
        Expr::FuncDef {
            name,
            params,
            body,
            then,
        } => eval_func_def(name, params, body, then, value, optional),
        Expr::FuncCall { name, args } => eval_func_call(name, args, value, optional),
        Expr::NamespacedCall {
            namespace,
            name,
            args: _,
        } => {
            // For now, namespaced calls return an error (modules not loaded)
            // This will be properly handled once module loading is implemented
            QueryResult::Error(EvalError::new(format!(
                "module '{}' not loaded (namespaced call {}::{})",
                namespace, namespace, name
            )))
        }
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
        match eval_single(expr, value.clone(), optional).materialize_cursor() {
            QueryResult::One(v) => all_results.push(v),
            QueryResult::OneCursor(_) => {
                unreachable!("materialize_cursor should have converted this")
            }
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

    let items: Vec<OwnedValue> = match result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
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
    let mut map = IndexMap::new();

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
        let owned_val = match val_result.materialize_cursor() {
            QueryResult::One(v) => to_owned(&v),
            QueryResult::OneCursor(_) => unreachable!(),
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
    match result.materialize_cursor() {
        QueryResult::One(v) => Ok(to_owned(&v)),
        QueryResult::OneCursor(_) => unreachable!(),
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
    mut left: IndexMap<String, OwnedValue>,
    right: IndexMap<String, OwnedValue>,
) -> IndexMap<String, OwnedValue> {
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
        QueryResult::OneCursor(_) => unreachable!("eval_single never produces OneCursor"),
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
        QueryResult::OneCursor(_) => unreachable!("eval_single never produces OneCursor"),
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

        // Phase 6: Type Conversions
        Builtin::ToString => builtin_tostring(value, optional),
        Builtin::ToNumber => builtin_tonumber(value, optional),

        // Phase 6: Additional String Functions
        Builtin::Explode => builtin_explode(value, optional),
        Builtin::Implode => builtin_implode(value, optional),
        Builtin::Test(re) => builtin_test(re, value, optional),
        Builtin::Indices(s) => builtin_indices(s, value, optional),
        Builtin::Index(s) => builtin_index(s, value, optional),
        Builtin::Rindex(s) => builtin_rindex(s, value, optional),
        Builtin::ToJsonStream => builtin_tojsonstream(value, optional),
        Builtin::FromJsonStream => builtin_fromjsonstream(value, optional),
        Builtin::GetPath(path) => builtin_getpath(path, value, optional),

        // Phase 7: Regex Functions (requires "regex" feature)
        #[cfg(feature = "regex")]
        Builtin::Match(re, flags) => builtin_match(re, flags.as_deref(), value, optional),
        #[cfg(feature = "regex")]
        Builtin::Capture(re) => builtin_capture(re, value, optional),
        #[cfg(feature = "regex")]
        Builtin::Scan(re) => builtin_scan(re, value, optional),
        #[cfg(feature = "regex")]
        Builtin::Splits(re) => builtin_splits(re, value, optional),
        #[cfg(feature = "regex")]
        Builtin::Sub(re, replacement) => builtin_sub(re, replacement, value, optional),
        #[cfg(feature = "regex")]
        Builtin::Gsub(re, replacement) => builtin_gsub(re, replacement, value, optional),
        #[cfg(feature = "regex")]
        Builtin::TestWithFlags(re, flags) => builtin_test_with_flags(re, flags, value, optional),

        // Phase 8: Advanced Control Flow Builtins
        Builtin::Recurse => builtin_recurse(value, optional),
        Builtin::RecurseF(f) => builtin_recurse_f(f, value, optional),
        Builtin::RecurseCond(f, cond) => builtin_recurse_cond(f, cond, value, optional),
        Builtin::Walk(f) => builtin_walk(f, value, optional),
        Builtin::IsValid(expr) => builtin_isvalid(expr, value, optional),

        // Phase 10: Path Expressions
        Builtin::Path(expr) => builtin_path(expr, value, optional),
        Builtin::Paths => builtin_paths(value, optional),
        Builtin::PathsFilter(filter) => builtin_paths_filter(filter, value, optional),
        Builtin::LeafPaths => builtin_leaf_paths(value, optional),
        Builtin::SetPath(path, val) => builtin_setpath(path, val, value, optional),
        Builtin::DelPaths(paths) => builtin_delpaths(paths, value, optional),

        // Phase 10: Math Functions
        Builtin::Floor => builtin_floor(value, optional),
        Builtin::Ceil => builtin_ceil(value, optional),
        Builtin::Round => builtin_round(value, optional),
        Builtin::Sqrt => builtin_sqrt(value, optional),
        Builtin::Fabs => builtin_fabs(value, optional),
        Builtin::Log => builtin_log(value, optional),
        Builtin::Log10 => builtin_log10(value, optional),
        Builtin::Log2 => builtin_log2(value, optional),
        Builtin::Exp => builtin_exp(value, optional),
        Builtin::Exp10 => builtin_exp10(value, optional),
        Builtin::Exp2 => builtin_exp2(value, optional),
        Builtin::Pow(base, exp) => builtin_pow(base, exp, value, optional),
        Builtin::Sin => builtin_sin(value, optional),
        Builtin::Cos => builtin_cos(value, optional),
        Builtin::Tan => builtin_tan(value, optional),
        Builtin::Asin => builtin_asin(value, optional),
        Builtin::Acos => builtin_acos(value, optional),
        Builtin::Atan => builtin_atan(value, optional),
        Builtin::Atan2(y, x) => builtin_atan2(y, x, value, optional),
        Builtin::Sinh => builtin_sinh(value, optional),
        Builtin::Cosh => builtin_cosh(value, optional),
        Builtin::Tanh => builtin_tanh(value, optional),
        Builtin::Asinh => builtin_asinh(value, optional),
        Builtin::Acosh => builtin_acosh(value, optional),
        Builtin::Atanh => builtin_atanh(value, optional),

        // Phase 10: Number Classification & Constants
        Builtin::Infinite => QueryResult::Owned(OwnedValue::Float(f64::INFINITY)),
        Builtin::Nan => QueryResult::Owned(OwnedValue::Float(f64::NAN)),
        Builtin::IsInfinite => builtin_isinfinite(value, optional),
        Builtin::IsNan => builtin_isnan(value, optional),
        Builtin::IsNormal => builtin_isnormal(value, optional),
        Builtin::IsFinite => builtin_isfinite(value, optional),

        // Phase 10: Debug
        Builtin::Debug => builtin_debug(value, optional),
        Builtin::DebugMsg(msg) => builtin_debug_msg(msg, value, optional),

        // Phase 10: Environment
        Builtin::Env => builtin_env(value, optional),
        Builtin::EnvVar(var) => builtin_envvar(var, value, optional),

        // Phase 10: Null handling
        Builtin::NullLit => QueryResult::Owned(OwnedValue::Null),

        // Phase 10: String functions
        Builtin::Trim => builtin_trim(value, optional),
        Builtin::Ltrim => builtin_ltrim(value, optional),
        Builtin::Rtrim => builtin_rtrim(value, optional),

        // Phase 10: Array functions
        Builtin::Transpose => builtin_transpose(value, optional),
        Builtin::BSearch(x) => builtin_bsearch(x, value, optional),

        // Phase 10: Object functions
        Builtin::ModuleMeta(name) => builtin_modulemeta(name, value, optional),
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
                if let StandardJson::String(k) = field.key() {
                    if let Ok(cow) = k.as_str() {
                        keys.push(cow.into_owned());
                    }
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
                if let StandardJson::String(k) = f.key() {
                    if let Ok(cow) = k.as_str() {
                        return cow.as_ref() == key;
                    }
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
                if let StandardJson::String(k) = f.key() {
                    if let Ok(cow) = k.as_str() {
                        return cow.as_ref() == key;
                    }
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
        QueryResult::OneCursor(_) => unreachable!("eval_single never produces OneCursor"),
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
                match eval_single(f, elem, optional).materialize_cursor() {
                    QueryResult::One(v) => results.push(to_owned(&v)),
                    QueryResult::OneCursor(_) => unreachable!(),
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
            let mut result_map = IndexMap::new();
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
                match eval_single(f, field_val, optional).materialize_cursor() {
                    QueryResult::One(v) => {
                        result_map.insert(key, to_owned(&v));
                    }
                    QueryResult::OneCursor(_) => unreachable!(),
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
                match eval_single(f, elem, optional).materialize_cursor() {
                    QueryResult::One(v) => results.push(to_owned(&v)),
                    QueryResult::OneCursor(_) => unreachable!(),
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
                let key = if let StandardJson::String(k) = field.key() {
                    if let Ok(cow) = k.as_str() {
                        cow.into_owned()
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                let val = to_owned(&field.value());

                let mut entry = IndexMap::new();
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
            let mut result = IndexMap::new();

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
                let key = if let StandardJson::String(k) = field.key() {
                    if let Ok(cow) = k.as_str() {
                        cow.into_owned()
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                let val = to_owned(&field.value());

                let mut entry = IndexMap::new();
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

                match eval_single(f, cursor.value(), optional).materialize_cursor() {
                    QueryResult::One(v) => transformed.push(to_owned(&v)),
                    QueryResult::OneCursor(_) => unreachable!(),
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
            let mut result = IndexMap::new();
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

// =============================================================================
// Phase 6: String Interpolation & Format Strings
// =============================================================================

/// Evaluate string interpolation: `"Hello \(.name)"`
fn eval_string_interpolation<'a, W: Clone + AsRef<[u64]>>(
    parts: &[StringPart],
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut result = String::new();

    for part in parts {
        match part {
            StringPart::Literal(s) => result.push_str(s),
            StringPart::Expr(expr) => {
                let val = eval_single(expr, value.clone(), optional).materialize_cursor();
                let s = match val {
                    QueryResult::One(v) => owned_to_string(&to_owned(&v)),
                    QueryResult::OneCursor(_) => unreachable!(),
                    QueryResult::Owned(v) => owned_to_string(&v),
                    QueryResult::Many(vs) => {
                        if let Some(v) = vs.first() {
                            owned_to_string(&to_owned(v))
                        } else {
                            String::new()
                        }
                    }
                    QueryResult::ManyOwned(vs) => {
                        if let Some(v) = vs.first() {
                            owned_to_string(v)
                        } else {
                            String::new()
                        }
                    }
                    QueryResult::None => String::new(),
                    QueryResult::Error(e) => return QueryResult::Error(e),
                };
                result.push_str(&s);
            }
        }
    }

    QueryResult::Owned(OwnedValue::String(result))
}

/// Convert an owned value to a string representation (for interpolation).
fn owned_to_string(value: &OwnedValue) -> String {
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(true) => "true".to_string(),
        OwnedValue::Bool(false) => "false".to_string(),
        OwnedValue::Int(n) => format!("{}", n),
        OwnedValue::Float(f) => format!("{}", f),
        OwnedValue::String(s) => s.clone(), // Don't quote strings in interpolation
        OwnedValue::Array(_) | OwnedValue::Object(_) => value.to_json(),
    }
}

/// Evaluate a format string: `@json`, `@uri`, etc.
fn eval_format<'a, W: Clone + AsRef<[u64]>>(
    format_type: FormatType,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);

    let result = match format_type {
        FormatType::Text => format_text(&owned),
        FormatType::Json => format_json(&owned),
        FormatType::Uri => format_uri(&owned, optional),
        FormatType::Csv => format_csv(&owned, optional),
        FormatType::Tsv => format_tsv(&owned, optional),
        FormatType::Base64 => format_base64(&owned, optional),
        FormatType::Base64d => format_base64d(&owned, optional),
        FormatType::Html => format_html(&owned, optional),
        FormatType::Sh => format_sh(&owned, optional),
    };

    match result {
        Ok(s) => QueryResult::Owned(OwnedValue::String(s)),
        Err(e) => QueryResult::Error(e),
    }
}

/// @text - Convert to string (same as tostring)
fn format_text(value: &OwnedValue) -> Result<String, EvalError> {
    Ok(owned_to_string(value))
}

/// @json - Format as JSON
fn format_json(value: &OwnedValue) -> Result<String, EvalError> {
    Ok(value.to_json())
}

/// @uri - URI/percent encode
fn format_uri(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::String(s) => {
            let mut result = String::new();
            for c in s.chars() {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                    result.push(c);
                } else {
                    for b in c.to_string().as_bytes() {
                        result.push_str(&format!("%{:02X}", b));
                    }
                }
            }
            Ok(result)
        }
        _ if optional => Ok(String::new()),
        _ => Err(EvalError::type_error("string", value.type_name())),
    }
}

/// @csv - CSV format (for arrays)
fn format_csv(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    OwnedValue::String(s) => {
                        if s.contains('"') || s.contains(',') || s.contains('\n') {
                            format!("\"{}\"", s.replace('"', "\"\""))
                        } else {
                            s.clone()
                        }
                    }
                    OwnedValue::Null => String::new(),
                    other => owned_to_string(other),
                })
                .collect();
            Ok(parts.join(","))
        }
        _ if optional => Ok(String::new()),
        _ => Err(EvalError::type_error("array", value.type_name())),
    }
}

/// @tsv - TSV format (for arrays)
fn format_tsv(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    OwnedValue::String(s) => s
                        .replace('\\', "\\\\")
                        .replace('\t', "\\t")
                        .replace('\n', "\\n")
                        .replace('\r', "\\r"),
                    OwnedValue::Null => String::new(),
                    other => owned_to_string(other),
                })
                .collect();
            Ok(parts.join("\t"))
        }
        _ if optional => Ok(String::new()),
        _ => Err(EvalError::type_error("array", value.type_name())),
    }
}

/// @base64 - Base64 encode (simple implementation without external crate)
fn format_base64(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::String(s) => {
            // Simple base64 encoding
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let bytes = s.as_bytes();
            let mut result = String::new();

            for chunk in bytes.chunks(3) {
                let b0 = chunk[0] as u32;
                let b1 = chunk.get(1).map(|&b| b as u32).unwrap_or(0);
                let b2 = chunk.get(2).map(|&b| b as u32).unwrap_or(0);

                let triple = (b0 << 16) | (b1 << 8) | b2;

                result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
                result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);

                if chunk.len() > 1 {
                    result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
                } else {
                    result.push('=');
                }

                if chunk.len() > 2 {
                    result.push(ALPHABET[(triple & 0x3F) as usize] as char);
                } else {
                    result.push('=');
                }
            }

            Ok(result)
        }
        _ if optional => Ok(String::new()),
        _ => Err(EvalError::type_error("string", value.type_name())),
    }
}

/// @base64d - Base64 decode
fn format_base64d(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::String(s) => {
            // Simple base64 decoding
            fn decode_char(c: u8) -> Option<u8> {
                match c {
                    b'A'..=b'Z' => Some(c - b'A'),
                    b'a'..=b'z' => Some(c - b'a' + 26),
                    b'0'..=b'9' => Some(c - b'0' + 52),
                    b'+' => Some(62),
                    b'/' => Some(63),
                    b'=' => Some(0), // Padding
                    _ => None,
                }
            }

            let s = s.replace(|c: char| c.is_whitespace(), "");
            let bytes: Vec<u8> = s.bytes().collect();
            let mut result = Vec::new();

            for chunk in bytes.chunks(4) {
                if chunk.len() < 4 {
                    break;
                }

                let a = decode_char(chunk[0]).ok_or_else(|| EvalError::new("invalid base64"))?;
                let b = decode_char(chunk[1]).ok_or_else(|| EvalError::new("invalid base64"))?;
                let c_val =
                    decode_char(chunk[2]).ok_or_else(|| EvalError::new("invalid base64"))?;
                let d = decode_char(chunk[3]).ok_or_else(|| EvalError::new("invalid base64"))?;

                let triple =
                    ((a as u32) << 18) | ((b as u32) << 12) | ((c_val as u32) << 6) | (d as u32);

                result.push(((triple >> 16) & 0xFF) as u8);
                if chunk[2] != b'=' {
                    result.push(((triple >> 8) & 0xFF) as u8);
                }
                if chunk[3] != b'=' {
                    result.push((triple & 0xFF) as u8);
                }
            }

            String::from_utf8(result).map_err(|_| EvalError::new("invalid UTF-8 in decoded base64"))
        }
        _ if optional => Ok(String::new()),
        _ => Err(EvalError::type_error("string", value.type_name())),
    }
}

/// @html - HTML entity escape
fn format_html(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::String(s) => {
            let mut result = String::new();
            for c in s.chars() {
                match c {
                    '<' => result.push_str("&lt;"),
                    '>' => result.push_str("&gt;"),
                    '&' => result.push_str("&amp;"),
                    '"' => result.push_str("&quot;"),
                    '\'' => result.push_str("&#39;"),
                    _ => result.push(c),
                }
            }
            Ok(result)
        }
        _ if optional => Ok(String::new()),
        _ => Err(EvalError::type_error("string", value.type_name())),
    }
}

/// @sh - Shell quote
fn format_sh(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::String(s) => {
            // Use single quotes and escape single quotes
            if s.contains('\'') {
                let escaped = s.replace('\'', "'\\''");
                Ok(format!("'{}'", escaped))
            } else {
                Ok(format!("'{}'", s))
            }
        }
        _ if optional => Ok(String::new()),
        _ => Err(EvalError::type_error("string", value.type_name())),
    }
}

// =============================================================================
// Phase 6: Type Conversion Builtins
// =============================================================================

/// Builtin: tostring - convert any value to string
fn builtin_tostring<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    let s = match owned {
        OwnedValue::String(s) => s,
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(true) => "true".to_string(),
        OwnedValue::Bool(false) => "false".to_string(),
        OwnedValue::Int(n) => format!("{}", n),
        OwnedValue::Float(f) => format!("{}", f),
        OwnedValue::Array(_) | OwnedValue::Object(_) => owned.to_json(),
    };
    QueryResult::Owned(OwnedValue::String(s))
}

/// Builtin: tonumber - convert string to number
fn builtin_tonumber<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::Number(n) => {
            // Already a number, return as-is
            if let Ok(i) = n.as_i64() {
                QueryResult::Owned(OwnedValue::Int(i))
            } else if let Ok(f) = n.as_f64() {
                QueryResult::Owned(OwnedValue::Float(f))
            } else {
                QueryResult::Owned(OwnedValue::Int(0))
            }
        }
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let s = cow.as_ref().trim();
                if let Ok(i) = s.parse::<i64>() {
                    QueryResult::Owned(OwnedValue::Int(i))
                } else if let Ok(f) = s.parse::<f64>() {
                    QueryResult::Owned(OwnedValue::Float(f))
                } else if optional {
                    QueryResult::None
                } else {
                    QueryResult::Error(EvalError::new(format!("cannot parse '{}' as number", s)))
                }
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string or number", type_name(&value))),
    }
}

// =============================================================================
// Phase 6: Additional String Builtins
// =============================================================================

/// Builtin: explode - string to array of Unicode codepoints
fn builtin_explode<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let codepoints: Vec<OwnedValue> = cow
                    .chars()
                    .map(|c| OwnedValue::Int(c as u32 as i64))
                    .collect();
                QueryResult::Owned(OwnedValue::Array(codepoints))
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: implode - array of codepoints to string
fn builtin_implode<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::Array(elements) => {
            let mut result = String::new();
            for elem in *elements {
                if let StandardJson::Number(n) = elem {
                    if let Ok(codepoint) = n.as_i64() {
                        if let Some(c) = char::from_u32(codepoint as u32) {
                            result.push(c);
                        } else if optional {
                            continue;
                        } else {
                            return QueryResult::Error(EvalError::new(format!(
                                "invalid codepoint: {}",
                                codepoint
                            )));
                        }
                    }
                }
            }
            QueryResult::Owned(OwnedValue::String(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: test(re) - test if string matches (basic substring matching)
fn builtin_test<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Check if the input string contains the pattern (simple substring match)
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::Bool(cow.contains(&pattern)))
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    }
}

/// Builtin: indices(s) - find all indices of substring s
fn builtin_indices<'a, W: Clone + AsRef<[u64]>>(
    s_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern
    let pattern = match result_to_owned(eval_single(s_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let mut indices = Vec::new();
                let mut start = 0;
                while let Some(pos) = cow[start..].find(&pattern) {
                    indices.push(OwnedValue::Int((start + pos) as i64));
                    start += pos + 1;
                    if start >= cow.len() {
                        break;
                    }
                }
                QueryResult::Owned(OwnedValue::Array(indices))
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        StandardJson::Array(elements) => {
            // For arrays, find indices where element equals the pattern
            let pattern_owned = OwnedValue::String(pattern);
            let mut indices = Vec::new();
            for (i, elem) in (*elements).enumerate() {
                if to_owned(&elem) == pattern_owned {
                    indices.push(OwnedValue::Int(i as i64));
                }
            }
            QueryResult::Owned(OwnedValue::Array(indices))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string or array", type_name(&value))),
    }
}

/// Builtin: index(s) - first index of substring s, or null
fn builtin_index<'a, W: Clone + AsRef<[u64]>>(
    s_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern
    let pattern = match result_to_owned(eval_single(s_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                if let Some(pos) = cow.find(&pattern) {
                    QueryResult::Owned(OwnedValue::Int(pos as i64))
                } else {
                    QueryResult::Owned(OwnedValue::Null)
                }
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        StandardJson::Array(elements) => {
            let pattern_owned = OwnedValue::String(pattern);
            for (i, elem) in (*elements).enumerate() {
                if to_owned(&elem) == pattern_owned {
                    return QueryResult::Owned(OwnedValue::Int(i as i64));
                }
            }
            QueryResult::Owned(OwnedValue::Null)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string or array", type_name(&value))),
    }
}

/// Builtin: rindex(s) - last index of substring s, or null
fn builtin_rindex<'a, W: Clone + AsRef<[u64]>>(
    s_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern
    let pattern = match result_to_owned(eval_single(s_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                if let Some(pos) = cow.rfind(&pattern) {
                    QueryResult::Owned(OwnedValue::Int(pos as i64))
                } else {
                    QueryResult::Owned(OwnedValue::Null)
                }
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        StandardJson::Array(elements) => {
            let pattern_owned = OwnedValue::String(pattern);
            let items: Vec<_> = (*elements).collect();
            for (i, elem) in items.iter().enumerate().rev() {
                if to_owned(elem) == pattern_owned {
                    return QueryResult::Owned(OwnedValue::Int(i as i64));
                }
            }
            QueryResult::Owned(OwnedValue::Null)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("string or array", type_name(&value))),
    }
}

/// Builtin: tojsonstream - convert to JSON text stream format (simplified)
fn builtin_tojsonstream<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // Simplified: just return the value as JSON lines format
    let owned = to_owned(&value);
    fn collect_stream(value: &OwnedValue, path: &[OwnedValue], results: &mut Vec<OwnedValue>) {
        match value {
            OwnedValue::Array(arr) => {
                for (i, v) in arr.iter().enumerate() {
                    let mut new_path = path.to_vec();
                    new_path.push(OwnedValue::Int(i as i64));
                    collect_stream(v, &new_path, results);
                }
            }
            OwnedValue::Object(obj) => {
                for (k, v) in obj.iter() {
                    let mut new_path = path.to_vec();
                    new_path.push(OwnedValue::String(k.clone()));
                    collect_stream(v, &new_path, results);
                }
            }
            _ => {
                let entry =
                    OwnedValue::Array(vec![OwnedValue::Array(path.to_vec()), value.clone()]);
                results.push(entry);
            }
        }
    }

    let mut results = Vec::new();
    collect_stream(&owned, &[], &mut results);
    QueryResult::Owned(OwnedValue::Array(results))
}

/// Builtin: fromjsonstream - convert from JSON text stream format (simplified)
fn builtin_fromjsonstream<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // This is a complex operation - provide a simplified version
    match &value {
        StandardJson::Array(_) => {
            // For now, return the input - full implementation would reconstruct
            QueryResult::Owned(to_owned(&value))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: getpath(path) - get value at path
fn builtin_getpath<'a, W: Clone + AsRef<[u64]>>(
    path_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the path expression
    let path = match result_to_owned(eval_single(path_expr, value.clone(), optional)) {
        Ok(OwnedValue::Array(arr)) => arr,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("array", "path")),
        Err(e) => return QueryResult::Error(e),
    };

    let mut current = to_owned(&value);

    for segment in path {
        match (&current, &segment) {
            (OwnedValue::Object(obj), OwnedValue::String(key)) => {
                current = obj.get(key).cloned().unwrap_or(OwnedValue::Null);
            }
            (OwnedValue::Array(arr), OwnedValue::Int(idx)) => {
                let index = if *idx < 0 {
                    (arr.len() as i64 + *idx) as usize
                } else {
                    *idx as usize
                };
                current = arr.get(index).cloned().unwrap_or(OwnedValue::Null);
            }
            _ if optional => return QueryResult::None,
            _ => {
                return QueryResult::Error(EvalError::new(format!(
                    "cannot index {} with {}",
                    current.type_name(),
                    segment.type_name()
                )));
            }
        }
    }

    QueryResult::Owned(current)
}

// =============================================================================
// Phase 7: Regex Functions (requires "regex" feature)
// =============================================================================

/// Build regex flags from jq flag string
#[cfg(feature = "regex")]
fn build_regex(pattern: &str, flags: Option<&str>) -> Result<regex::Regex, EvalError> {
    let mut pattern = pattern.to_string();

    // Apply flags
    if let Some(flags) = flags {
        let mut prefix = String::from("(?");
        for c in flags.chars() {
            match c {
                'i' => prefix.push('i'), // case insensitive
                'x' => prefix.push('x'), // extended mode (ignore whitespace)
                's' => prefix.push('s'), // single-line mode (. matches newline)
                'm' => prefix.push('m'), // multi-line mode
                'g' => {}                // global - handled at call site
                'p' => {}                // PCRE mode - not fully supported
                _ => {}
            }
        }
        if prefix.len() > 2 {
            prefix.push(')');
            pattern = format!("{}{}", prefix, pattern);
        }
    }

    regex::Regex::new(&pattern).map_err(|e| EvalError::new(format!("invalid regex: {}", e)))
}

/// Builtin: match(re) or match(re; flags) - return match object
#[cfg(feature = "regex")]
fn builtin_match<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    flags: Option<&str>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the input string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, flags) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Check if global flag is set
    let global = flags.map(|f| f.contains('g')).unwrap_or(false);

    if global {
        // Return all matches
        let matches: Vec<OwnedValue> = re
            .find_iter(&input)
            .map(|m| build_match_object(&re, m.as_str(), m.start(), &input))
            .collect();
        QueryResult::Owned(OwnedValue::Array(matches))
    } else {
        // Return first match or null
        match re.find(&input) {
            Some(m) => QueryResult::Owned(build_match_object(&re, m.as_str(), m.start(), &input)),
            None => QueryResult::Owned(OwnedValue::Null),
        }
    }
}

/// Build a jq match object
#[cfg(feature = "regex")]
fn build_match_object(re: &regex::Regex, matched: &str, offset: usize, input: &str) -> OwnedValue {
    let mut obj = IndexMap::new();

    obj.insert("offset".to_string(), OwnedValue::Int(offset as i64));
    obj.insert("length".to_string(), OwnedValue::Int(matched.len() as i64));
    obj.insert(
        "string".to_string(),
        OwnedValue::String(matched.to_string()),
    );

    // Build captures array
    let mut captures = Vec::new();
    if let Some(caps) = re.captures(input) {
        for (i, name) in re.capture_names().enumerate() {
            if i == 0 {
                continue; // Skip the full match
            }
            let cap = caps.get(i);
            let mut cap_obj = IndexMap::new();
            cap_obj.insert(
                "offset".to_string(),
                cap.map(|m| OwnedValue::Int(m.start() as i64))
                    .unwrap_or(OwnedValue::Null),
            );
            cap_obj.insert(
                "length".to_string(),
                cap.map(|m| OwnedValue::Int(m.len() as i64))
                    .unwrap_or(OwnedValue::Int(0)),
            );
            cap_obj.insert(
                "string".to_string(),
                cap.map(|m| OwnedValue::String(m.as_str().to_string()))
                    .unwrap_or(OwnedValue::Null),
            );
            cap_obj.insert(
                "name".to_string(),
                name.map(|n| OwnedValue::String(n.to_string()))
                    .unwrap_or(OwnedValue::Null),
            );
            captures.push(OwnedValue::Object(cap_obj));
        }
    }
    obj.insert("captures".to_string(), OwnedValue::Array(captures));

    OwnedValue::Object(obj)
}

/// Builtin: capture(re) - return named captures as object
#[cfg(feature = "regex")]
fn builtin_capture<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the input string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, None) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Extract named captures
    match re.captures(&input) {
        Some(caps) => {
            let mut result = IndexMap::new();
            for name in re.capture_names().flatten() {
                if let Some(m) = caps.name(name) {
                    result.insert(name.to_string(), OwnedValue::String(m.as_str().to_string()));
                }
            }
            QueryResult::Owned(OwnedValue::Object(result))
        }
        None => QueryResult::Owned(OwnedValue::Null),
    }
}

/// Builtin: scan(re) - find all matches
#[cfg(feature = "regex")]
fn builtin_scan<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the input string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, None) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Find all matches
    let matches: Vec<OwnedValue> = re
        .find_iter(&input)
        .map(|m| OwnedValue::String(m.as_str().to_string()))
        .collect();

    QueryResult::ManyOwned(matches)
}

/// Builtin: splits(re) - split by regex
#[cfg(feature = "regex")]
fn builtin_splits<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the input string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, None) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Split by regex
    let parts: Vec<OwnedValue> = re
        .split(&input)
        .map(|s| OwnedValue::String(s.to_string()))
        .collect();

    QueryResult::Owned(OwnedValue::Array(parts))
}

/// Builtin: sub(re; replacement) - replace first match
#[cfg(feature = "regex")]
fn builtin_sub<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the replacement
    let replacement = match result_to_owned(eval_single(replacement_expr, value.clone(), optional))
    {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "replacement")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the input string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, None) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Replace first match
    let result = re.replace(&input, replacement.as_str());
    QueryResult::Owned(OwnedValue::String(result.into_owned()))
}

/// Builtin: gsub(re; replacement) - replace all matches
#[cfg(feature = "regex")]
fn builtin_gsub<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the replacement
    let replacement = match result_to_owned(eval_single(replacement_expr, value.clone(), optional))
    {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "replacement")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the input string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, None) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Replace all matches
    let result = re.replace_all(&input, replacement.as_str());
    QueryResult::Owned(OwnedValue::String(result.into_owned()))
}

/// Builtin: test(re; flags) - test with flags
#[cfg(feature = "regex")]
fn builtin_test_with_flags<'a, W: Clone + AsRef<[u64]>>(
    re_expr: &Expr,
    flags: &str,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the input string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", type_name(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, Some(flags)) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Test if regex matches
    QueryResult::Owned(OwnedValue::Bool(re.is_match(&input)))
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
    match result.materialize_cursor() {
        QueryResult::One(v) => eval_pipe(rest, v, optional),
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(values) => {
            let mut all_results = Vec::new();
            for v in values {
                match eval_pipe(rest, v, optional).materialize_cursor() {
                    QueryResult::One(r) => all_results.push(r),
                    QueryResult::OneCursor(_) => unreachable!(),
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
        QueryResult::Owned(v) => {
            // Continue piping with owned value using eval_owned_pipe
            eval_owned_pipe::<W>(rest, v, optional)
        }
        QueryResult::ManyOwned(vs) => {
            // Pipe each owned value through the rest
            let mut all_results: Vec<OwnedValue> = Vec::new();
            for v in vs {
                match eval_owned_pipe::<W>(rest, v, optional).materialize_cursor() {
                    QueryResult::Owned(r) => all_results.push(r),
                    QueryResult::OneCursor(_) => unreachable!(),
                    QueryResult::ManyOwned(rs) => all_results.extend(rs),
                    QueryResult::One(r) => all_results.push(to_owned(&r)),
                    QueryResult::Many(rs) => all_results.extend(rs.iter().map(to_owned)),
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                }
            }
            if all_results.is_empty() {
                QueryResult::None
            } else if all_results.len() == 1 {
                QueryResult::Owned(all_results.pop().unwrap())
            } else {
                QueryResult::ManyOwned(all_results)
            }
        }
    }
}

/// Evaluate a pipe with an OwnedValue as input.
fn eval_owned_pipe<'a, W: Clone + AsRef<[u64]>>(
    exprs: &[Expr],
    value: OwnedValue,
    optional: bool,
) -> QueryResult<'a, W> {
    if exprs.is_empty() {
        return QueryResult::Owned(value);
    }

    // For owned values, we need to serialize and re-evaluate
    // This is less efficient but ensures correctness
    let rest_expr = if exprs.len() == 1 {
        exprs[0].clone()
    } else {
        Expr::Pipe(exprs.to_vec())
    };

    match eval_owned_expr(&rest_expr, &value, optional) {
        Ok(v) => QueryResult::Owned(v),
        Err(e) => QueryResult::Error(e),
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
///
/// Uses `get_fast` for O(n) BP operations + O(log n) IB select,
/// instead of `get` which does O(n) IB selects.
fn get_element_at_index<'a, W: Clone + AsRef<[u64]>>(
    elements: JsonElements<'a, W>,
    idx: i64,
) -> Option<StandardJson<'a, W>> {
    if idx >= 0 {
        elements.get_fast(idx as usize)
    } else {
        // Negative index: count from end
        let len = count_elements(elements);
        let positive_idx = len as i64 + idx;
        if positive_idx >= 0 {
            elements.get_fast(positive_idx as usize)
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
            if pos < 0 {
                0
            } else {
                pos as usize
            }
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
    // Special case: Identity returns the cursor directly for efficient output
    // This avoids decomposing arrays/objects into individual cursors
    if matches!(expr, Expr::Identity) {
        return QueryResult::OneCursor(cursor);
    }
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
        QueryResult::OneCursor(c) => vec![c.value()],
        QueryResult::Many(vs) => vs,
        QueryResult::None => Vec::new(),
        QueryResult::Error(_) => Vec::new(),
        QueryResult::Owned(_) => Vec::new(), // Owned values not returned as StandardJson
        QueryResult::ManyOwned(_) => Vec::new(),
    }
}

// =============================================================================
// Phase 8: Variables and Advanced Control Flow Implementation
// =============================================================================

/// Substitute multiple variables in an expression.
///
/// This is useful for CLI tools that pass variables via `--arg`, `--argjson`, etc.
/// The function takes an iterator of (name, value) pairs and substitutes each
/// variable reference `$name` with the corresponding value.
///
/// # Example
///
/// ```ignore
/// use succinctly::jq::{parse, substitute_vars, eval, OwnedValue};
/// use std::collections::BTreeMap;
///
/// let expr = parse("$name + $suffix").unwrap();
/// let mut vars = IndexMap::new();
/// vars.insert("name".to_string(), OwnedValue::String("hello".to_string()));
/// vars.insert("suffix".to_string(), OwnedValue::String("world".to_string()));
///
/// let substituted = substitute_vars(&expr, &vars);
/// // Now evaluate substituted expression...
/// ```
pub fn substitute_vars<'a, I>(expr: &Expr, vars: I) -> Expr
where
    I: IntoIterator<Item = (&'a str, &'a OwnedValue)>,
{
    let mut result = expr.clone();
    for (name, value) in vars {
        result = substitute_var(&result, name, value);
    }
    result
}

/// Substitute a variable in an expression with a value.
/// Returns a new expression with the variable replaced.
fn substitute_var(expr: &Expr, var_name: &str, replacement: &OwnedValue) -> Expr {
    match expr {
        Expr::Var(name) if name == var_name => owned_to_expr(replacement),
        Expr::Var(_) => expr.clone(),
        Expr::Identity => Expr::Identity,
        Expr::Field(name) => Expr::Field(name.clone()),
        Expr::Index(i) => Expr::Index(*i),
        Expr::Slice { start, end } => Expr::Slice {
            start: *start,
            end: *end,
        },
        Expr::Iterate => Expr::Iterate,
        Expr::RecursiveDescent => Expr::RecursiveDescent,
        Expr::Optional(e) => Expr::Optional(Box::new(substitute_var(e, var_name, replacement))),
        Expr::Pipe(exprs) => Expr::Pipe(
            exprs
                .iter()
                .map(|e| substitute_var(e, var_name, replacement))
                .collect(),
        ),
        Expr::Comma(exprs) => Expr::Comma(
            exprs
                .iter()
                .map(|e| substitute_var(e, var_name, replacement))
                .collect(),
        ),
        Expr::Array(e) => Expr::Array(Box::new(substitute_var(e, var_name, replacement))),
        Expr::Object(entries) => Expr::Object(
            entries
                .iter()
                .map(|entry| {
                    let new_key = match &entry.key {
                        ObjectKey::Literal(s) => ObjectKey::Literal(s.clone()),
                        ObjectKey::Expr(e) => {
                            ObjectKey::Expr(Box::new(substitute_var(e, var_name, replacement)))
                        }
                    };
                    ObjectEntry {
                        key: new_key,
                        value: substitute_var(&entry.value, var_name, replacement),
                    }
                })
                .collect(),
        ),
        Expr::Literal(lit) => Expr::Literal(lit.clone()),
        Expr::Paren(e) => Expr::Paren(Box::new(substitute_var(e, var_name, replacement))),
        Expr::Arithmetic { op, left, right } => Expr::Arithmetic {
            op: *op,
            left: Box::new(substitute_var(left, var_name, replacement)),
            right: Box::new(substitute_var(right, var_name, replacement)),
        },
        Expr::Compare { op, left, right } => Expr::Compare {
            op: *op,
            left: Box::new(substitute_var(left, var_name, replacement)),
            right: Box::new(substitute_var(right, var_name, replacement)),
        },
        Expr::And(l, r) => Expr::And(
            Box::new(substitute_var(l, var_name, replacement)),
            Box::new(substitute_var(r, var_name, replacement)),
        ),
        Expr::Or(l, r) => Expr::Or(
            Box::new(substitute_var(l, var_name, replacement)),
            Box::new(substitute_var(r, var_name, replacement)),
        ),
        Expr::Not => Expr::Not,
        Expr::Alternative(l, r) => Expr::Alternative(
            Box::new(substitute_var(l, var_name, replacement)),
            Box::new(substitute_var(r, var_name, replacement)),
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(substitute_var(cond, var_name, replacement)),
            then_branch: Box::new(substitute_var(then_branch, var_name, replacement)),
            else_branch: Box::new(substitute_var(else_branch, var_name, replacement)),
        },
        Expr::Try { expr, catch } => Expr::Try {
            expr: Box::new(substitute_var(expr, var_name, replacement)),
            catch: catch
                .as_ref()
                .map(|e| Box::new(substitute_var(e, var_name, replacement))),
        },
        Expr::Error(msg) => Expr::Error(msg.clone()),
        Expr::Builtin(b) => Expr::Builtin(substitute_var_in_builtin(b, var_name, replacement)),
        Expr::StringInterpolation(parts) => Expr::StringInterpolation(
            parts
                .iter()
                .map(|p| match p {
                    StringPart::Literal(s) => StringPart::Literal(s.clone()),
                    StringPart::Expr(e) => {
                        StringPart::Expr(Box::new(substitute_var(e, var_name, replacement)))
                    }
                })
                .collect(),
        ),
        Expr::Format(f) => Expr::Format(*f),
        // Phase 8 expressions
        Expr::As { expr, var, body } => {
            // Don't substitute if this `as` binds the same variable (shadowing)
            if var == var_name {
                Expr::As {
                    expr: Box::new(substitute_var(expr, var_name, replacement)),
                    var: var.clone(),
                    body: body.clone(), // Don't substitute in body - shadowed
                }
            } else {
                Expr::As {
                    expr: Box::new(substitute_var(expr, var_name, replacement)),
                    var: var.clone(),
                    body: Box::new(substitute_var(body, var_name, replacement)),
                }
            }
        }
        Expr::Reduce {
            input,
            var,
            init,
            update,
        } => {
            if var == var_name {
                Expr::Reduce {
                    input: Box::new(substitute_var(input, var_name, replacement)),
                    var: var.clone(),
                    init: Box::new(substitute_var(init, var_name, replacement)),
                    update: update.clone(), // shadowed
                }
            } else {
                Expr::Reduce {
                    input: Box::new(substitute_var(input, var_name, replacement)),
                    var: var.clone(),
                    init: Box::new(substitute_var(init, var_name, replacement)),
                    update: Box::new(substitute_var(update, var_name, replacement)),
                }
            }
        }
        Expr::Foreach {
            input,
            var,
            init,
            update,
            extract,
        } => {
            if var == var_name {
                Expr::Foreach {
                    input: Box::new(substitute_var(input, var_name, replacement)),
                    var: var.clone(),
                    init: Box::new(substitute_var(init, var_name, replacement)),
                    update: update.clone(),
                    extract: extract.clone(),
                }
            } else {
                Expr::Foreach {
                    input: Box::new(substitute_var(input, var_name, replacement)),
                    var: var.clone(),
                    init: Box::new(substitute_var(init, var_name, replacement)),
                    update: Box::new(substitute_var(update, var_name, replacement)),
                    extract: extract
                        .as_ref()
                        .map(|e| Box::new(substitute_var(e, var_name, replacement))),
                }
            }
        }
        Expr::Limit { n, expr } => Expr::Limit {
            n: Box::new(substitute_var(n, var_name, replacement)),
            expr: Box::new(substitute_var(expr, var_name, replacement)),
        },
        Expr::FirstExpr(e) => Expr::FirstExpr(Box::new(substitute_var(e, var_name, replacement))),
        Expr::LastExpr(e) => Expr::LastExpr(Box::new(substitute_var(e, var_name, replacement))),
        Expr::NthExpr { n, expr } => Expr::NthExpr {
            n: Box::new(substitute_var(n, var_name, replacement)),
            expr: Box::new(substitute_var(expr, var_name, replacement)),
        },
        Expr::Until { cond, update } => Expr::Until {
            cond: Box::new(substitute_var(cond, var_name, replacement)),
            update: Box::new(substitute_var(update, var_name, replacement)),
        },
        Expr::While { cond, update } => Expr::While {
            cond: Box::new(substitute_var(cond, var_name, replacement)),
            update: Box::new(substitute_var(update, var_name, replacement)),
        },
        Expr::Repeat(e) => Expr::Repeat(Box::new(substitute_var(e, var_name, replacement))),
        Expr::Range { from, to, step } => Expr::Range {
            from: Box::new(substitute_var(from, var_name, replacement)),
            to: to
                .as_ref()
                .map(|e| Box::new(substitute_var(e, var_name, replacement))),
            step: step
                .as_ref()
                .map(|e| Box::new(substitute_var(e, var_name, replacement))),
        },
        // Phase 9: Variables & Definitions
        Expr::AsPattern {
            expr,
            pattern,
            body,
        } => {
            // Check if any pattern variable shadows the var_name
            let shadowed = pattern_binds_var(pattern, var_name);
            Expr::AsPattern {
                expr: Box::new(substitute_var(expr, var_name, replacement)),
                pattern: pattern.clone(),
                body: if shadowed {
                    body.clone()
                } else {
                    Box::new(substitute_var(body, var_name, replacement))
                },
            }
        }
        Expr::FuncDef {
            name,
            params,
            body,
            then,
        } => {
            // Check if any parameter shadows the var_name
            let shadowed = params.contains(&var_name.to_string());
            Expr::FuncDef {
                name: name.clone(),
                params: params.clone(),
                body: if shadowed {
                    body.clone()
                } else {
                    Box::new(substitute_var(body, var_name, replacement))
                },
                then: Box::new(substitute_var(then, var_name, replacement)),
            }
        }
        Expr::FuncCall { name, args } => Expr::FuncCall {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_var(a, var_name, replacement))
                .collect(),
        },
        Expr::NamespacedCall {
            namespace,
            name,
            args,
        } => Expr::NamespacedCall {
            namespace: namespace.clone(),
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_var(a, var_name, replacement))
                .collect(),
        },
    }
}

/// Check if a pattern binds a given variable name.
fn pattern_binds_var(pattern: &Pattern, var_name: &str) -> bool {
    match pattern {
        Pattern::Var(name) => name == var_name,
        Pattern::Object(entries) => entries
            .iter()
            .any(|e| pattern_binds_var(&e.pattern, var_name)),
        Pattern::Array(patterns) => patterns.iter().any(|p| pattern_binds_var(p, var_name)),
    }
}

/// Substitute variable in a builtin expression.
fn substitute_var_in_builtin(
    builtin: &Builtin,
    var_name: &str,
    replacement: &OwnedValue,
) -> Builtin {
    match builtin {
        Builtin::Type => Builtin::Type,
        Builtin::IsNull => Builtin::IsNull,
        Builtin::IsBoolean => Builtin::IsBoolean,
        Builtin::IsNumber => Builtin::IsNumber,
        Builtin::IsString => Builtin::IsString,
        Builtin::IsArray => Builtin::IsArray,
        Builtin::IsObject => Builtin::IsObject,
        Builtin::Length => Builtin::Length,
        Builtin::Utf8ByteLength => Builtin::Utf8ByteLength,
        Builtin::Keys => Builtin::Keys,
        Builtin::KeysUnsorted => Builtin::KeysUnsorted,
        Builtin::Has(e) => Builtin::Has(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::In(e) => Builtin::In(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Select(e) => Builtin::Select(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Empty => Builtin::Empty,
        Builtin::Map(e) => Builtin::Map(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::MapValues(e) => {
            Builtin::MapValues(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Add => Builtin::Add,
        Builtin::Any => Builtin::Any,
        Builtin::All => Builtin::All,
        Builtin::Min => Builtin::Min,
        Builtin::Max => Builtin::Max,
        Builtin::MinBy(e) => Builtin::MinBy(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::MaxBy(e) => Builtin::MaxBy(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::AsciiDowncase => Builtin::AsciiDowncase,
        Builtin::AsciiUpcase => Builtin::AsciiUpcase,
        Builtin::Ltrimstr(e) => {
            Builtin::Ltrimstr(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Rtrimstr(e) => {
            Builtin::Rtrimstr(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Startswith(e) => {
            Builtin::Startswith(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Endswith(e) => {
            Builtin::Endswith(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Split(e) => Builtin::Split(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Join(e) => Builtin::Join(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Contains(e) => {
            Builtin::Contains(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Inside(e) => Builtin::Inside(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::First => Builtin::First,
        Builtin::Last => Builtin::Last,
        Builtin::Nth(e) => Builtin::Nth(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Reverse => Builtin::Reverse,
        Builtin::Flatten => Builtin::Flatten,
        Builtin::FlattenDepth(e) => {
            Builtin::FlattenDepth(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::GroupBy(e) => Builtin::GroupBy(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Unique => Builtin::Unique,
        Builtin::UniqueBy(e) => {
            Builtin::UniqueBy(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Sort => Builtin::Sort,
        Builtin::SortBy(e) => Builtin::SortBy(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::ToEntries => Builtin::ToEntries,
        Builtin::FromEntries => Builtin::FromEntries,
        Builtin::WithEntries(e) => {
            Builtin::WithEntries(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::ToString => Builtin::ToString,
        Builtin::ToNumber => Builtin::ToNumber,
        Builtin::Explode => Builtin::Explode,
        Builtin::Implode => Builtin::Implode,
        Builtin::Test(e) => Builtin::Test(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Indices(e) => Builtin::Indices(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Index(e) => Builtin::Index(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Rindex(e) => Builtin::Rindex(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::ToJsonStream => Builtin::ToJsonStream,
        Builtin::FromJsonStream => Builtin::FromJsonStream,
        Builtin::GetPath(e) => Builtin::GetPath(Box::new(substitute_var(e, var_name, replacement))),
        #[cfg(feature = "regex")]
        Builtin::Match(re, flags) => Builtin::Match(
            Box::new(substitute_var(re, var_name, replacement)),
            flags.clone(),
        ),
        #[cfg(feature = "regex")]
        Builtin::Capture(e) => Builtin::Capture(Box::new(substitute_var(e, var_name, replacement))),
        #[cfg(feature = "regex")]
        Builtin::Scan(e) => Builtin::Scan(Box::new(substitute_var(e, var_name, replacement))),
        #[cfg(feature = "regex")]
        Builtin::Splits(e) => Builtin::Splits(Box::new(substitute_var(e, var_name, replacement))),
        #[cfg(feature = "regex")]
        Builtin::Sub(re, repl) => Builtin::Sub(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(repl, var_name, replacement)),
        ),
        #[cfg(feature = "regex")]
        Builtin::Gsub(re, repl) => Builtin::Gsub(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(repl, var_name, replacement)),
        ),
        #[cfg(feature = "regex")]
        Builtin::TestWithFlags(re, flags) => Builtin::TestWithFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            flags.clone(),
        ),
        // Phase 8 builtins
        Builtin::Recurse => Builtin::Recurse,
        Builtin::RecurseF(f) => {
            Builtin::RecurseF(Box::new(substitute_var(f, var_name, replacement)))
        }
        Builtin::RecurseCond(f, c) => Builtin::RecurseCond(
            Box::new(substitute_var(f, var_name, replacement)),
            Box::new(substitute_var(c, var_name, replacement)),
        ),
        Builtin::Walk(f) => Builtin::Walk(Box::new(substitute_var(f, var_name, replacement))),
        Builtin::IsValid(e) => Builtin::IsValid(Box::new(substitute_var(e, var_name, replacement))),
        // Phase 10 builtins
        Builtin::Path(e) => Builtin::Path(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Paths => Builtin::Paths,
        Builtin::PathsFilter(e) => {
            Builtin::PathsFilter(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::LeafPaths => Builtin::LeafPaths,
        Builtin::SetPath(p, v) => Builtin::SetPath(
            Box::new(substitute_var(p, var_name, replacement)),
            Box::new(substitute_var(v, var_name, replacement)),
        ),
        Builtin::DelPaths(e) => {
            Builtin::DelPaths(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Floor => Builtin::Floor,
        Builtin::Ceil => Builtin::Ceil,
        Builtin::Round => Builtin::Round,
        Builtin::Sqrt => Builtin::Sqrt,
        Builtin::Fabs => Builtin::Fabs,
        Builtin::Log => Builtin::Log,
        Builtin::Log10 => Builtin::Log10,
        Builtin::Log2 => Builtin::Log2,
        Builtin::Exp => Builtin::Exp,
        Builtin::Exp10 => Builtin::Exp10,
        Builtin::Exp2 => Builtin::Exp2,
        Builtin::Pow(x, y) => Builtin::Pow(
            Box::new(substitute_var(x, var_name, replacement)),
            Box::new(substitute_var(y, var_name, replacement)),
        ),
        Builtin::Sin => Builtin::Sin,
        Builtin::Cos => Builtin::Cos,
        Builtin::Tan => Builtin::Tan,
        Builtin::Asin => Builtin::Asin,
        Builtin::Acos => Builtin::Acos,
        Builtin::Atan => Builtin::Atan,
        Builtin::Atan2(x, y) => Builtin::Atan2(
            Box::new(substitute_var(x, var_name, replacement)),
            Box::new(substitute_var(y, var_name, replacement)),
        ),
        Builtin::Sinh => Builtin::Sinh,
        Builtin::Cosh => Builtin::Cosh,
        Builtin::Tanh => Builtin::Tanh,
        Builtin::Asinh => Builtin::Asinh,
        Builtin::Acosh => Builtin::Acosh,
        Builtin::Atanh => Builtin::Atanh,
        Builtin::Infinite => Builtin::Infinite,
        Builtin::Nan => Builtin::Nan,
        Builtin::IsInfinite => Builtin::IsInfinite,
        Builtin::IsNan => Builtin::IsNan,
        Builtin::IsNormal => Builtin::IsNormal,
        Builtin::IsFinite => Builtin::IsFinite,
        Builtin::Debug => Builtin::Debug,
        Builtin::DebugMsg(e) => {
            Builtin::DebugMsg(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Env => Builtin::Env,
        Builtin::EnvVar(e) => Builtin::EnvVar(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::NullLit => Builtin::NullLit,
        Builtin::Trim => Builtin::Trim,
        Builtin::Ltrim => Builtin::Ltrim,
        Builtin::Rtrim => Builtin::Rtrim,
        Builtin::Transpose => Builtin::Transpose,
        Builtin::BSearch(e) => Builtin::BSearch(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::ModuleMeta(e) => {
            Builtin::ModuleMeta(Box::new(substitute_var(e, var_name, replacement)))
        }
    }
}

/// Convert an OwnedValue to an Expr, preserving complex types.
fn owned_to_expr(value: &OwnedValue) -> Expr {
    match value {
        OwnedValue::Null => Expr::Literal(Literal::Null),
        OwnedValue::Bool(b) => Expr::Literal(Literal::Bool(*b)),
        OwnedValue::Int(i) => Expr::Literal(Literal::Int(*i)),
        OwnedValue::Float(f) => Expr::Literal(Literal::Float(*f)),
        OwnedValue::String(s) => Expr::Literal(Literal::String(s.clone())),
        OwnedValue::Array(arr) => {
            // Build array construction expression with all elements
            if arr.is_empty() {
                Expr::Array(Box::new(Expr::Builtin(Builtin::Empty)))
            } else {
                let elements: Vec<Expr> = arr.iter().map(owned_to_expr).collect();
                Expr::Array(Box::new(Expr::Comma(elements)))
            }
        }
        OwnedValue::Object(obj) => {
            // Build object construction expression
            let entries: Vec<ObjectEntry> = obj
                .iter()
                .map(|(k, v)| ObjectEntry {
                    key: ObjectKey::Literal(k.clone()),
                    value: owned_to_expr(v),
                })
                .collect();
            Expr::Object(entries)
        }
    }
}

/// Evaluate `as` binding: `expr as $var | body`.
fn eval_as<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    var: &str,
    body: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the expression to get the value to bind
    let bound_result = eval_single(expr, value.clone(), optional);

    // Get all values from the expression
    let bound_values: Vec<OwnedValue> = match bound_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
    };

    // For each bound value, substitute and evaluate the body
    let mut all_results: Vec<OwnedValue> = Vec::new();

    for bound_val in bound_values {
        let substituted_body = substitute_var(body, var, &bound_val);
        match eval_single(&substituted_body, value.clone(), optional).materialize_cursor() {
            QueryResult::One(v) => all_results.push(to_owned(&v)),
            QueryResult::OneCursor(_) => unreachable!(),
            QueryResult::Many(vs) => all_results.extend(vs.iter().map(to_owned)),
            QueryResult::Owned(v) => all_results.push(v),
            QueryResult::ManyOwned(vs) => all_results.extend(vs),
            QueryResult::None => {}
            QueryResult::Error(e) => return QueryResult::Error(e),
        }
    }

    if all_results.is_empty() {
        QueryResult::None
    } else if all_results.len() == 1 {
        QueryResult::Owned(all_results.pop().unwrap())
    } else {
        QueryResult::ManyOwned(all_results)
    }
}

/// Evaluate `reduce`: `reduce EXPR as $var (INIT; UPDATE)`.
fn eval_reduce<'a, W: Clone + AsRef<[u64]>>(
    input: &Expr,
    var: &str,
    init: &Expr,
    update: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate input to get the stream of values
    let input_result = eval_single(input, value.clone(), optional);
    let input_values: Vec<OwnedValue> = match input_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => Vec::new(),
        QueryResult::Error(e) => return QueryResult::Error(e),
    };

    // Evaluate initial accumulator
    let init_result = eval_single(init, value.clone(), optional);
    let mut acc = match result_to_owned(init_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    // For each input value, update the accumulator
    for input_val in input_values {
        // Substitute $var in update, then evaluate with acc as input
        let substituted = substitute_var(update, var, &input_val);
        // We need to evaluate with acc as the input
        let acc_result = eval_owned_expr(&substituted, &acc, optional);
        match acc_result {
            Ok(new_acc) => acc = new_acc,
            Err(e) => return QueryResult::Error(e),
        }
    }

    QueryResult::Owned(acc)
}

/// Evaluate an expression with an OwnedValue as input.
fn eval_owned_expr(
    expr: &Expr,
    input: &OwnedValue,
    optional: bool,
) -> Result<OwnedValue, EvalError> {
    // Create a synthetic JSON from the owned value
    // For simplicity, we'll serialize and reparse
    // This is inefficient but correct
    let json_str = owned_value_to_json_string(input);
    let json_bytes = json_str.as_bytes();

    // We need to create a temporary index and cursor
    use crate::json::JsonIndex;
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    match eval_single(expr, cursor.value(), optional).materialize_cursor() {
        QueryResult::One(v) => Ok(to_owned(&v)),
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Owned(v) => Ok(v),
        QueryResult::Many(vs) => {
            if vs.len() == 1 {
                Ok(to_owned(&vs[0]))
            } else {
                Ok(OwnedValue::Array(vs.iter().map(to_owned).collect()))
            }
        }
        QueryResult::ManyOwned(vs) => {
            if vs.len() == 1 {
                Ok(vs.into_iter().next().unwrap())
            } else {
                Ok(OwnedValue::Array(vs))
            }
        }
        QueryResult::None => Ok(OwnedValue::Null),
        QueryResult::Error(e) => Err(e),
    }
}

/// Convert an OwnedValue to a JSON string.
fn owned_value_to_json_string(value: &OwnedValue) -> String {
    match value {
        OwnedValue::Null => "null".into(),
        OwnedValue::Bool(true) => "true".into(),
        OwnedValue::Bool(false) => "false".into(),
        OwnedValue::Int(i) => format!("{}", i),
        OwnedValue::Float(f) => {
            if f.is_nan() || f.is_infinite() {
                "null".into() // JSON doesn't have NaN or Infinity
            } else {
                format!("{}", f)
            }
        }
        OwnedValue::String(s) => {
            // Escape the string for JSON
            let mut result = String::with_capacity(s.len() + 2);
            result.push('"');
            for c in s.chars() {
                match c {
                    '"' => result.push_str("\\\""),
                    '\\' => result.push_str("\\\\"),
                    '\n' => result.push_str("\\n"),
                    '\r' => result.push_str("\\r"),
                    '\t' => result.push_str("\\t"),
                    c if c.is_control() => {
                        result.push_str(&format!("\\u{:04x}", c as u32));
                    }
                    c => result.push(c),
                }
            }
            result.push('"');
            result
        }
        OwnedValue::Array(arr) => {
            let mut result = String::from("[");
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    result.push(',');
                }
                result.push_str(&owned_value_to_json_string(v));
            }
            result.push(']');
            result
        }
        OwnedValue::Object(obj) => {
            let mut result = String::from("{");
            for (i, (k, v)) in obj.iter().enumerate() {
                if i > 0 {
                    result.push(',');
                }
                result.push_str(&owned_value_to_json_string(&OwnedValue::String(k.clone())));
                result.push(':');
                result.push_str(&owned_value_to_json_string(v));
            }
            result.push('}');
            result
        }
    }
}

/// Evaluate `foreach`: `foreach EXPR as $var (INIT; UPDATE)` or `foreach EXPR as $var (INIT; UPDATE; EXTRACT)`.
fn eval_foreach<'a, W: Clone + AsRef<[u64]>>(
    input: &Expr,
    var: &str,
    init: &Expr,
    update: &Expr,
    extract: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate input to get the stream
    let input_result = eval_single(input, value.clone(), optional);
    let input_values: Vec<OwnedValue> = match input_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => Vec::new(),
        QueryResult::Error(e) => return QueryResult::Error(e),
    };

    // Evaluate initial state
    let init_result = eval_single(init, value.clone(), optional);
    let mut state = match result_to_owned(init_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let mut outputs: Vec<OwnedValue> = Vec::new();

    for input_val in input_values {
        // Substitute $var and evaluate update with state as input
        let substituted_update = substitute_var(update, var, &input_val);
        match eval_owned_expr(&substituted_update, &state, optional) {
            Ok(new_state) => {
                state = new_state;
                // If there's an extract expression, evaluate it
                if let Some(ext) = extract {
                    let substituted_extract = substitute_var(ext, var, &input_val);
                    match eval_owned_expr(&substituted_extract, &state, optional) {
                        Ok(output) => outputs.push(output),
                        Err(e) => return QueryResult::Error(e),
                    }
                } else {
                    // Without extract, output the current state
                    outputs.push(state.clone());
                }
            }
            Err(e) => return QueryResult::Error(e),
        }
    }

    if outputs.is_empty() {
        QueryResult::None
    } else if outputs.len() == 1 {
        QueryResult::Owned(outputs.pop().unwrap())
    } else {
        QueryResult::ManyOwned(outputs)
    }
}

/// Evaluate `limit(n; expr)` - take first n outputs.
fn eval_limit<'a, W: Clone + AsRef<[u64]>>(
    n_expr: &Expr,
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate n
    let n_result = eval_single(n_expr, value.clone(), optional);
    let n = match result_to_owned(n_result) {
        Ok(OwnedValue::Int(i)) if i >= 0 => i as usize,
        Ok(_) => {
            return QueryResult::Error(EvalError::new("limit requires non-negative integer"));
        }
        Err(e) => return QueryResult::Error(e),
    };

    if n == 0 {
        return QueryResult::None;
    }

    // Evaluate expr and take first n
    let result = eval_single(expr, value, optional);
    match result {
        QueryResult::One(v) if n >= 1 => QueryResult::One(v),
        QueryResult::Many(vs) => {
            let taken: Vec<_> = vs.into_iter().take(n).collect();
            if taken.is_empty() {
                QueryResult::None
            } else if taken.len() == 1 {
                QueryResult::One(taken.into_iter().next().unwrap())
            } else {
                QueryResult::Many(taken)
            }
        }
        QueryResult::Owned(v) if n >= 1 => QueryResult::Owned(v),
        QueryResult::ManyOwned(vs) => {
            let taken: Vec<_> = vs.into_iter().take(n).collect();
            if taken.is_empty() {
                QueryResult::None
            } else if taken.len() == 1 {
                QueryResult::Owned(taken.into_iter().next().unwrap())
            } else {
                QueryResult::ManyOwned(taken)
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        _ => QueryResult::None,
    }
}

/// Evaluate `first(expr)` - take first output.
fn eval_first_expr<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single(expr, value, optional);
    match result.materialize_cursor() {
        QueryResult::One(v) => QueryResult::One(v),
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => {
            if let Some(first) = vs.into_iter().next() {
                QueryResult::One(first)
            } else {
                QueryResult::None
            }
        }
        QueryResult::Owned(v) => QueryResult::Owned(v),
        QueryResult::ManyOwned(vs) => {
            if let Some(first) = vs.into_iter().next() {
                QueryResult::Owned(first)
            } else {
                QueryResult::None
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
    }
}

/// Evaluate `last(expr)` - take last output.
fn eval_last_expr<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single(expr, value, optional);
    match result.materialize_cursor() {
        QueryResult::One(v) => QueryResult::One(v),
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => {
            if let Some(last) = vs.into_iter().last() {
                QueryResult::One(last)
            } else {
                QueryResult::None
            }
        }
        QueryResult::Owned(v) => QueryResult::Owned(v),
        QueryResult::ManyOwned(vs) => {
            if let Some(last) = vs.into_iter().last() {
                QueryResult::Owned(last)
            } else {
                QueryResult::None
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
    }
}

/// Evaluate `nth(n; expr)` - take nth output.
fn eval_nth_expr<'a, W: Clone + AsRef<[u64]>>(
    n_expr: &Expr,
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate n
    let n_result = eval_single(n_expr, value.clone(), optional);
    let n = match result_to_owned(n_result) {
        Ok(OwnedValue::Int(i)) if i >= 0 => i as usize,
        Ok(_) => {
            return QueryResult::Error(EvalError::new("nth requires non-negative integer"));
        }
        Err(e) => return QueryResult::Error(e),
    };

    let result = eval_single(expr, value, optional);
    match result.materialize_cursor() {
        QueryResult::One(v) if n == 0 => QueryResult::One(v),
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::One(_) => QueryResult::None,
        QueryResult::Many(vs) => {
            if let Some(item) = vs.into_iter().nth(n) {
                QueryResult::One(item)
            } else {
                QueryResult::None
            }
        }
        QueryResult::Owned(v) if n == 0 => QueryResult::Owned(v),
        QueryResult::Owned(_) => QueryResult::None,
        QueryResult::ManyOwned(vs) => {
            if let Some(item) = vs.into_iter().nth(n) {
                QueryResult::Owned(item)
            } else {
                QueryResult::None
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
    }
}

/// Evaluate `until(cond; update)` - apply update until cond is true.
fn eval_until<'a, W: Clone + AsRef<[u64]>>(
    cond: &Expr,
    update: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut current = to_owned(&value);
    const MAX_ITERATIONS: usize = 10000;

    for _ in 0..MAX_ITERATIONS {
        // Check condition
        match eval_owned_expr(cond, &current, optional) {
            Ok(cond_val) => {
                if cond_val.is_truthy() {
                    return QueryResult::Owned(current);
                }
            }
            Err(e) => return QueryResult::Error(e),
        }

        // Apply update
        match eval_owned_expr(update, &current, optional) {
            Ok(new_val) => current = new_val,
            Err(e) => return QueryResult::Error(e),
        }
    }

    QueryResult::Error(EvalError::new("until: maximum iterations exceeded"))
}

/// Evaluate `while(cond; update)` - output values while cond is true.
fn eval_while<'a, W: Clone + AsRef<[u64]>>(
    cond: &Expr,
    update: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut current = to_owned(&value);
    let mut outputs: Vec<OwnedValue> = Vec::new();
    const MAX_ITERATIONS: usize = 10000;

    for _ in 0..MAX_ITERATIONS {
        // Check condition
        match eval_owned_expr(cond, &current, optional) {
            Ok(cond_val) => {
                if !cond_val.is_truthy() {
                    break;
                }
            }
            Err(e) => return QueryResult::Error(e),
        }

        // Output current value
        outputs.push(current.clone());

        // Apply update
        match eval_owned_expr(update, &current, optional) {
            Ok(new_val) => current = new_val,
            Err(e) => return QueryResult::Error(e),
        }
    }

    if outputs.is_empty() {
        QueryResult::None
    } else if outputs.len() == 1 {
        QueryResult::Owned(outputs.pop().unwrap())
    } else {
        QueryResult::ManyOwned(outputs)
    }
}

/// Evaluate `repeat(expr)` - repeatedly apply expr.
/// Note: This produces an infinite stream, so we limit it.
fn eval_repeat<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut current = to_owned(&value);
    let mut outputs: Vec<OwnedValue> = Vec::new();
    const MAX_ITERATIONS: usize = 1000; // Limit to prevent infinite loops

    for _ in 0..MAX_ITERATIONS {
        outputs.push(current.clone());
        match eval_owned_expr(expr, &current, optional) {
            Ok(new_val) => current = new_val,
            Err(_) => break, // Stop on error
        }
    }

    if outputs.is_empty() {
        QueryResult::None
    } else if outputs.len() == 1 {
        QueryResult::Owned(outputs.pop().unwrap())
    } else {
        QueryResult::ManyOwned(outputs)
    }
}

/// Evaluate `range(n)`, `range(a;b)`, or `range(a;b;step)`.
fn eval_range<'a, W: Clone + AsRef<[u64]>>(
    from: &Expr,
    to: Option<&Expr>,
    step: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let from_val = match eval_single(from, value.clone(), optional) {
        QueryResult::Owned(OwnedValue::Int(i)) => i,
        QueryResult::Owned(OwnedValue::Float(f)) => f as i64,
        QueryResult::One(v) => match to_owned(&v) {
            OwnedValue::Int(i) => i,
            OwnedValue::Float(f) => f as i64,
            _ => return QueryResult::Error(EvalError::new("range requires numbers")),
        },
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::Error(EvalError::new("range requires numbers")),
    };

    let to_val = if let Some(to_expr) = to {
        match eval_single(to_expr, value.clone(), optional) {
            QueryResult::Owned(OwnedValue::Int(i)) => i,
            QueryResult::Owned(OwnedValue::Float(f)) => f as i64,
            QueryResult::One(v) => match to_owned(&v) {
                OwnedValue::Int(i) => i,
                OwnedValue::Float(f) => f as i64,
                _ => return QueryResult::Error(EvalError::new("range requires numbers")),
            },
            QueryResult::Error(e) => return QueryResult::Error(e),
            _ => return QueryResult::Error(EvalError::new("range requires numbers")),
        }
    } else {
        // range(n) means range(0; n)
        let to = from_val;
        return eval_range_values(0, to, 1);
    };

    let step_val = if let Some(step_expr) = step {
        match eval_single(step_expr, value, optional) {
            QueryResult::Owned(OwnedValue::Int(i)) if i != 0 => i,
            QueryResult::Owned(OwnedValue::Float(f)) if f != 0.0 => f as i64,
            QueryResult::One(v) => match to_owned(&v) {
                OwnedValue::Int(i) if i != 0 => i,
                OwnedValue::Float(f) if f != 0.0 => f as i64,
                _ => return QueryResult::Error(EvalError::new("range step cannot be zero")),
            },
            QueryResult::Error(e) => return QueryResult::Error(e),
            _ => return QueryResult::Error(EvalError::new("range step cannot be zero")),
        }
    } else {
        1
    };

    eval_range_values(from_val, to_val, step_val)
}

/// Helper to generate range values.
fn eval_range_values<'a, W: Clone + AsRef<[u64]>>(
    from: i64,
    to: i64,
    step: i64,
) -> QueryResult<'a, W> {
    let mut values: Vec<OwnedValue> = Vec::new();
    const MAX_RANGE: usize = 100000;

    if step > 0 {
        let mut i = from;
        while i < to && values.len() < MAX_RANGE {
            values.push(OwnedValue::Int(i));
            i += step;
        }
    } else if step < 0 {
        let mut i = from;
        while i > to && values.len() < MAX_RANGE {
            values.push(OwnedValue::Int(i));
            i += step;
        }
    }

    if values.is_empty() {
        QueryResult::None
    } else if values.len() == 1 {
        QueryResult::Owned(values.pop().unwrap())
    } else {
        QueryResult::ManyOwned(values)
    }
}

/// Builtin: recurse (recurse(.[]))
fn builtin_recurse<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Default recurse is equivalent to recurse(.[]?)
    let f = Expr::Optional(Box::new(Expr::Iterate));
    builtin_recurse_f(&f, value, optional)
}

/// Builtin: recurse(f)
fn builtin_recurse_f<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    let mut outputs: Vec<OwnedValue> = Vec::new();
    let mut queue: Vec<OwnedValue> = vec![to_owned(&value)];
    const MAX_ITEMS: usize = 10000;

    while !queue.is_empty() && outputs.len() < MAX_ITEMS {
        let current = queue.remove(0);
        outputs.push(current.clone());

        // Apply f to get children
        match eval_owned_expr(f, &current, true) {
            Ok(OwnedValue::Array(arr)) => {
                queue.extend(arr);
            }
            Ok(v) if !matches!(v, OwnedValue::Null) => {
                queue.push(v);
            }
            _ => {}
        }
    }

    if outputs.is_empty() {
        QueryResult::None
    } else if outputs.len() == 1 {
        QueryResult::Owned(outputs.pop().unwrap())
    } else {
        QueryResult::ManyOwned(outputs)
    }
}

/// Builtin: recurse(f; cond)
fn builtin_recurse_cond<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    cond: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut outputs: Vec<OwnedValue> = Vec::new();
    let mut queue: Vec<OwnedValue> = vec![to_owned(&value)];
    const MAX_ITEMS: usize = 10000;

    while !queue.is_empty() && outputs.len() < MAX_ITEMS {
        let current = queue.remove(0);

        // Check condition
        let should_continue = match eval_owned_expr(cond, &current, optional) {
            Ok(v) => v.is_truthy(),
            Err(_) => false,
        };

        if !should_continue {
            continue;
        }

        outputs.push(current.clone());

        // Apply f to get children
        match eval_owned_expr(f, &current, true) {
            Ok(OwnedValue::Array(arr)) => {
                queue.extend(arr);
            }
            Ok(v) if !matches!(v, OwnedValue::Null) => {
                queue.push(v);
            }
            _ => {}
        }
    }

    if outputs.is_empty() {
        QueryResult::None
    } else if outputs.len() == 1 {
        QueryResult::Owned(outputs.pop().unwrap())
    } else {
        QueryResult::ManyOwned(outputs)
    }
}

/// Builtin: walk(f) - recursively transform all values.
fn builtin_walk<'a, W: Clone + AsRef<[u64]>>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    match walk_impl(f, owned, optional) {
        Ok(result) => QueryResult::Owned(result),
        Err(e) => QueryResult::Error(e),
    }
}

/// Implementation of walk - processes children first, then applies f.
fn walk_impl(f: &Expr, value: OwnedValue, optional: bool) -> Result<OwnedValue, EvalError> {
    // First, recursively process children
    let processed = match value {
        OwnedValue::Array(arr) => {
            let new_arr: Result<Vec<_>, _> =
                arr.into_iter().map(|v| walk_impl(f, v, optional)).collect();
            OwnedValue::Array(new_arr?)
        }
        OwnedValue::Object(obj) => {
            let new_obj: Result<IndexMap<_, _>, _> = obj
                .into_iter()
                .map(|(k, v)| walk_impl(f, v, optional).map(|nv| (k, nv)))
                .collect();
            OwnedValue::Object(new_obj?)
        }
        other => other,
    };

    // Then apply f to the processed value
    eval_owned_expr(f, &processed, optional)
}

/// Builtin: isvalid(expr) - check if expr succeeds without errors.
fn builtin_isvalid<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    match eval_single(expr, value, true) {
        QueryResult::Error(_) => QueryResult::Owned(OwnedValue::Bool(false)),
        QueryResult::None => QueryResult::Owned(OwnedValue::Bool(false)),
        _ => QueryResult::Owned(OwnedValue::Bool(true)),
    }
}

// ============================================================================
// Phase 10: Path Expressions, Math, Environment, etc.
// ============================================================================

/// Builtin: path(expr) - return the path to values selected by expr
/// Since we don't track paths during evaluation, we return an empty array (stub)
fn builtin_path<'a, W: Clone + AsRef<[u64]>>(
    _expr: &Expr,
    _value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // In a full implementation, path() would track the path to each value.
    // For now, return an empty array as a stub.
    QueryResult::Owned(OwnedValue::Array(vec![]))
}

/// Helper to collect all paths recursively
fn collect_paths(value: &OwnedValue, current_path: &[OwnedValue], paths: &mut Vec<OwnedValue>) {
    match value {
        OwnedValue::Object(entries) => {
            for (key, val) in entries {
                let mut new_path = current_path.to_vec();
                new_path.push(OwnedValue::String(key.clone()));
                paths.push(OwnedValue::Array(new_path.clone()));
                collect_paths(val, &new_path, paths);
            }
        }
        OwnedValue::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                let mut new_path = current_path.to_vec();
                new_path.push(OwnedValue::Int(i as i64));
                paths.push(OwnedValue::Array(new_path.clone()));
                collect_paths(val, &new_path, paths);
            }
        }
        _ => {}
    }
}

/// Builtin: paths - all paths to values (excluding empty paths)
fn builtin_paths<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    let mut paths = Vec::new();
    collect_paths(&owned, &[], &mut paths);
    QueryResult::Owned(OwnedValue::Array(paths))
}

/// Builtin: paths(filter) - paths to values matching filter
fn builtin_paths_filter<'a, W: Clone + AsRef<[u64]>>(
    filter: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    let mut all_paths = Vec::new();
    collect_paths(&owned, &[], &mut all_paths);

    let mut filtered_paths = Vec::new();
    for path in all_paths {
        if let OwnedValue::Array(path_arr) = &path {
            // Get the value at this path
            if let Some(val_at_path) = get_value_at_path(&owned, path_arr) {
                // Check if filter matches
                if let Ok(OwnedValue::Bool(true)) = eval_owned_expr(filter, &val_at_path, optional)
                {
                    filtered_paths.push(path);
                }
            }
        }
    }
    QueryResult::Owned(OwnedValue::Array(filtered_paths))
}

/// Helper to get value at a path
fn get_value_at_path(value: &OwnedValue, path: &[OwnedValue]) -> Option<OwnedValue> {
    if path.is_empty() {
        return Some(value.clone());
    }
    match (&path[0], value) {
        (OwnedValue::String(key), OwnedValue::Object(entries)) => {
            for (k, v) in entries {
                if k == key {
                    return get_value_at_path(v, &path[1..]);
                }
            }
            None
        }
        (OwnedValue::Int(idx), OwnedValue::Array(arr)) => {
            let index = if *idx < 0 {
                (arr.len() as i64 + *idx) as usize
            } else {
                *idx as usize
            };
            arr.get(index)
                .and_then(|v| get_value_at_path(v, &path[1..]))
        }
        _ => None,
    }
}

/// Helper to collect leaf paths (paths to scalars)
fn collect_leaf_paths(
    value: &OwnedValue,
    current_path: &[OwnedValue],
    paths: &mut Vec<OwnedValue>,
) {
    match value {
        OwnedValue::Object(entries) => {
            if entries.is_empty() {
                // Empty object is a leaf
                paths.push(OwnedValue::Array(current_path.to_vec()));
            } else {
                for (key, val) in entries {
                    let mut new_path = current_path.to_vec();
                    new_path.push(OwnedValue::String(key.clone()));
                    collect_leaf_paths(val, &new_path, paths);
                }
            }
        }
        OwnedValue::Array(arr) => {
            if arr.is_empty() {
                // Empty array is a leaf
                paths.push(OwnedValue::Array(current_path.to_vec()));
            } else {
                for (i, val) in arr.iter().enumerate() {
                    let mut new_path = current_path.to_vec();
                    new_path.push(OwnedValue::Int(i as i64));
                    collect_leaf_paths(val, &new_path, paths);
                }
            }
        }
        _ => {
            // Scalar value is a leaf
            paths.push(OwnedValue::Array(current_path.to_vec()));
        }
    }
}

/// Builtin: leaf_paths - paths to scalar (non-container) values
fn builtin_leaf_paths<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    let mut paths = Vec::new();
    collect_leaf_paths(&owned, &[], &mut paths);
    QueryResult::Owned(OwnedValue::Array(paths))
}

/// Helper to set a value at a path
fn set_value_at_path(value: OwnedValue, path: &[OwnedValue], new_val: OwnedValue) -> OwnedValue {
    if path.is_empty() {
        return new_val;
    }
    match &path[0] {
        OwnedValue::String(key) => match value {
            OwnedValue::Object(mut entries) => {
                if entries.contains_key(key) {
                    let old = entries.shift_remove(key).unwrap_or(OwnedValue::Null);
                    let new = set_value_at_path(old, &path[1..], new_val);
                    entries.insert(key.clone(), new);
                } else {
                    let val = set_value_at_path(OwnedValue::Null, &path[1..], new_val);
                    entries.insert(key.clone(), val);
                }
                OwnedValue::Object(entries)
            }
            _ => {
                // Create new object
                let val = set_value_at_path(OwnedValue::Null, &path[1..], new_val);
                let mut entries = IndexMap::new();
                entries.insert(key.clone(), val);
                OwnedValue::Object(entries)
            }
        },
        OwnedValue::Int(idx) => match value {
            OwnedValue::Array(mut arr) => {
                let index = if *idx < 0 {
                    (arr.len() as i64 + *idx) as usize
                } else {
                    *idx as usize
                };
                // Extend array if needed
                while arr.len() <= index {
                    arr.push(OwnedValue::Null);
                }
                let old = arr[index].clone();
                arr[index] = set_value_at_path(old, &path[1..], new_val);
                OwnedValue::Array(arr)
            }
            _ => {
                // Create new array
                let index = if *idx < 0 { 0 } else { *idx as usize };
                let mut arr = vec![OwnedValue::Null; index + 1];
                arr[index] = set_value_at_path(OwnedValue::Null, &path[1..], new_val);
                OwnedValue::Array(arr)
            }
        },
        _ => value,
    }
}

/// Builtin: setpath(path; value) - set value at path
fn builtin_setpath<'a, W: Clone + AsRef<[u64]>>(
    path_expr: &Expr,
    val_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate path expression
    let path_result = eval_single(path_expr, value.clone(), optional);
    let path_owned = match path_result {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::Error(EvalError::new("setpath requires array path")),
    };

    let path = match path_owned {
        OwnedValue::Array(p) => p,
        _ => return QueryResult::Error(EvalError::new("setpath requires array path")),
    };

    // Evaluate value expression
    let new_val = match eval_single(val_expr, value.clone(), optional) {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => OwnedValue::Null,
    };

    let owned = to_owned(&value);
    let result = set_value_at_path(owned, &path, new_val);
    QueryResult::Owned(result)
}

/// Helper to delete a path from a value
fn delete_path(value: OwnedValue, path: &[OwnedValue]) -> OwnedValue {
    if path.is_empty() {
        return OwnedValue::Null;
    }
    if path.len() == 1 {
        match &path[0] {
            OwnedValue::String(key) => match value {
                OwnedValue::Object(mut entries) => {
                    entries.shift_remove(key);
                    return OwnedValue::Object(entries);
                }
                other => return other,
            },
            OwnedValue::Int(idx) => match value {
                OwnedValue::Array(mut arr) => {
                    let index = if *idx < 0 {
                        (arr.len() as i64 + *idx) as usize
                    } else {
                        *idx as usize
                    };
                    if index < arr.len() {
                        arr.remove(index);
                    }
                    return OwnedValue::Array(arr);
                }
                other => return other,
            },
            _ => return value,
        }
    }
    match &path[0] {
        OwnedValue::String(key) => match value {
            OwnedValue::Object(mut entries) => {
                if let Some(v) = entries.shift_remove(key) {
                    let updated = delete_path(v, &path[1..]);
                    entries.insert(key.clone(), updated);
                }
                OwnedValue::Object(entries)
            }
            other => other,
        },
        OwnedValue::Int(idx) => match value {
            OwnedValue::Array(mut arr) => {
                let index = if *idx < 0 {
                    (arr.len() as i64 + *idx) as usize
                } else {
                    *idx as usize
                };
                if index < arr.len() {
                    let old = arr[index].clone();
                    arr[index] = delete_path(old, &path[1..]);
                }
                OwnedValue::Array(arr)
            }
            other => other,
        },
        _ => value,
    }
}

/// Builtin: delpaths(paths) - delete multiple paths
fn builtin_delpaths<'a, W: Clone + AsRef<[u64]>>(
    paths_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate paths expression
    let paths_result = eval_single(paths_expr, value.clone(), optional);
    let paths_owned = match paths_result {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::Error(EvalError::new("delpaths requires array of paths")),
    };

    let paths = match paths_owned {
        OwnedValue::Array(p) => p,
        _ => return QueryResult::Error(EvalError::new("delpaths requires array of paths")),
    };

    let mut result = to_owned(&value);
    // Sort paths by length descending to delete deeper paths first
    let mut sorted_paths: Vec<Vec<OwnedValue>> = paths
        .into_iter()
        .filter_map(|p| match p {
            OwnedValue::Array(arr) => Some(arr),
            _ => None,
        })
        .collect();
    sorted_paths.sort_by_key(|b| core::cmp::Reverse(b.len()));

    for path in sorted_paths {
        result = delete_path(result, &path);
    }
    QueryResult::Owned(result)
}

// Phase 10: Math Functions

/// Helper to get float value from input
fn get_float_value<'a, W: Clone + AsRef<[u64]>>(
    value: &StandardJson<'a, W>,
    optional: bool,
) -> Result<f64, QueryResult<'a, W>> {
    match value {
        StandardJson::Number(n) => {
            if let Ok(f) = n.as_f64() {
                Ok(f)
            } else if optional {
                Err(QueryResult::None)
            } else {
                Err(QueryResult::Error(EvalError::new("invalid number")))
            }
        }
        _ if optional => Err(QueryResult::None),
        _ => Err(QueryResult::Error(EvalError::new(
            "math function requires number",
        ))),
    }
}

// no_std compatible floor: truncate towards negative infinity
fn floor_f64(x: f64) -> f64 {
    let t = x as i64 as f64;
    if x < t {
        t - 1.0
    } else {
        t
    }
}

// no_std compatible ceil: truncate towards positive infinity
fn ceil_f64(x: f64) -> f64 {
    let t = x as i64 as f64;
    if x > t {
        t + 1.0
    } else {
        t
    }
}

// no_std compatible round: round to nearest integer, half away from zero
fn round_f64(x: f64) -> f64 {
    if x >= 0.0 {
        floor_f64(x + 0.5)
    } else {
        ceil_f64(x - 0.5)
    }
}

// no_std compatible sqrt using Newton-Raphson
fn sqrt_f64(x: f64) -> f64 {
    if x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return 0.0;
    }
    let mut guess = x / 2.0;
    for _ in 0..50 {
        let next = (guess + x / guess) / 2.0;
        if (next - guess).abs() < 1e-15 * guess.abs() {
            break;
        }
        guess = next;
    }
    guess
}

/// Builtin: floor
fn builtin_floor<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Int(floor_f64(n) as i64)),
        Err(r) => r,
    }
}

/// Builtin: ceil
fn builtin_ceil<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Int(ceil_f64(n) as i64)),
        Err(r) => r,
    }
}

/// Builtin: round
fn builtin_round<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Int(round_f64(n) as i64)),
        Err(r) => r,
    }
}

/// Builtin: sqrt
fn builtin_sqrt<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(sqrt_f64(n))),
        Err(r) => r,
    }
}

/// Builtin: fabs (absolute value)
fn builtin_fabs<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::fabs(n))),
        Err(r) => r,
    }
}

/// Builtin: log (natural logarithm)
fn builtin_log<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::log(n))),
        Err(r) => r,
    }
}

/// Builtin: log10
fn builtin_log10<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::log10(n))),
        Err(r) => r,
    }
}

/// Builtin: log2
fn builtin_log2<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::log2(n))),
        Err(r) => r,
    }
}

/// Builtin: exp (e^x)
fn builtin_exp<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::exp(n))),
        Err(r) => r,
    }
}

/// Builtin: exp10 (10^x)
fn builtin_exp10<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::pow(10.0, n))),
        Err(r) => r,
    }
}

/// Builtin: exp2 (2^x)
fn builtin_exp2<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::exp2(n))),
        Err(r) => r,
    }
}

/// Error type for number extraction
enum NumberError {
    None,
    Error(EvalError),
}

/// Helper to get number from eval result
fn get_number_from_result<W: Clone + AsRef<[u64]>>(
    result: QueryResult<'_, W>,
    optional: bool,
) -> Result<f64, NumberError> {
    match result {
        QueryResult::Owned(OwnedValue::Int(n)) => Ok(n as f64),
        QueryResult::Owned(OwnedValue::Float(n)) => Ok(n),
        QueryResult::One(StandardJson::Number(n)) => n
            .as_f64()
            .map_err(|_| NumberError::Error(EvalError::new("invalid number"))),
        QueryResult::Error(e) => Err(NumberError::Error(e)),
        _ if optional => Err(NumberError::None),
        _ => Err(NumberError::Error(EvalError::new("expected number"))),
    }
}

/// Builtin: pow(base; exp) - power function
fn builtin_pow<'a, W: Clone + AsRef<[u64]>>(
    base_expr: &Expr,
    exp_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let base =
        match get_number_from_result(eval_single(base_expr, value.clone(), optional), optional) {
            Ok(n) => n,
            Err(NumberError::None) => return QueryResult::None,
            Err(NumberError::Error(e)) => return QueryResult::Error(e),
        };

    let exp = match get_number_from_result(eval_single(exp_expr, value, optional), optional) {
        Ok(n) => n,
        Err(NumberError::None) => return QueryResult::None,
        Err(NumberError::Error(e)) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Float(libm::pow(base, exp)))
}

// Trigonometric functions

/// Builtin: sin
fn builtin_sin<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::sin(n))),
        Err(r) => r,
    }
}

/// Builtin: cos
fn builtin_cos<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::cos(n))),
        Err(r) => r,
    }
}

/// Builtin: tan
fn builtin_tan<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::tan(n))),
        Err(r) => r,
    }
}

/// Builtin: asin
fn builtin_asin<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::asin(n))),
        Err(r) => r,
    }
}

/// Builtin: acos
fn builtin_acos<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::acos(n))),
        Err(r) => r,
    }
}

/// Builtin: atan
fn builtin_atan<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::atan(n))),
        Err(r) => r,
    }
}

/// Builtin: atan2(y; x)
fn builtin_atan2<'a, W: Clone + AsRef<[u64]>>(
    y_expr: &Expr,
    x_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let y = match get_number_from_result(eval_single(y_expr, value.clone(), optional), optional) {
        Ok(n) => n,
        Err(NumberError::None) => return QueryResult::None,
        Err(NumberError::Error(e)) => return QueryResult::Error(e),
    };

    let x = match get_number_from_result(eval_single(x_expr, value, optional), optional) {
        Ok(n) => n,
        Err(NumberError::None) => return QueryResult::None,
        Err(NumberError::Error(e)) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Float(libm::atan2(y, x)))
}

// Hyperbolic functions

/// Builtin: sinh
fn builtin_sinh<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::sinh(n))),
        Err(r) => r,
    }
}

/// Builtin: cosh
fn builtin_cosh<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::cosh(n))),
        Err(r) => r,
    }
}

/// Builtin: tanh
fn builtin_tanh<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::tanh(n))),
        Err(r) => r,
    }
}

/// Builtin: asinh
fn builtin_asinh<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::asinh(n))),
        Err(r) => r,
    }
}

/// Builtin: acosh
fn builtin_acosh<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::acosh(n))),
        Err(r) => r,
    }
}

/// Builtin: atanh
fn builtin_atanh<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::atanh(n))),
        Err(r) => r,
    }
}

// Number Classification
// Note: is_infinite(), is_nan(), is_normal(), is_finite() are available on f64 in no_std

/// Builtin: isinfinite
fn builtin_isinfinite<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::Number(n) => {
            if let Ok(f) = n.as_f64() {
                QueryResult::Owned(OwnedValue::Bool(f.is_infinite()))
            } else {
                QueryResult::Owned(OwnedValue::Bool(false))
            }
        }
        _ => QueryResult::Owned(OwnedValue::Bool(false)),
    }
}

/// Builtin: isnan
fn builtin_isnan<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::Number(n) => {
            if let Ok(f) = n.as_f64() {
                QueryResult::Owned(OwnedValue::Bool(f.is_nan()))
            } else {
                QueryResult::Owned(OwnedValue::Bool(false))
            }
        }
        _ => QueryResult::Owned(OwnedValue::Bool(false)),
    }
}

/// Builtin: isnormal
fn builtin_isnormal<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::Number(n) => {
            if let Ok(f) = n.as_f64() {
                QueryResult::Owned(OwnedValue::Bool(f.is_normal()))
            } else {
                QueryResult::Owned(OwnedValue::Bool(false))
            }
        }
        _ => QueryResult::Owned(OwnedValue::Bool(false)),
    }
}

/// Builtin: isfinite
fn builtin_isfinite<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match get_float_value(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Bool(n.is_finite())),
        Err(_) => QueryResult::Owned(OwnedValue::Bool(false)),
    }
}

// Debug functions

/// Builtin: debug - output value to stderr, pass through unchanged
fn builtin_debug<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // In a library context we don't actually print to stderr
    // We just pass through the value unchanged
    QueryResult::Owned(to_owned(&value))
}

/// Builtin: debug(msg) - output message and value to stderr
fn builtin_debug_msg<'a, W: Clone + AsRef<[u64]>>(
    _msg: &Expr,
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // In a library context we don't actually print to stderr
    // We just pass through the value unchanged
    QueryResult::Owned(to_owned(&value))
}

// Environment functions

/// Builtin: env - object of all environment variables
fn builtin_env<'a, W: Clone + AsRef<[u64]>>(
    _value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // Return empty object in no_std context
    // In std context, we could use std::env::vars()
    QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
}

/// Builtin: env.VAR or $ENV.VAR - get environment variable
fn builtin_envvar<'a, W: Clone + AsRef<[u64]>>(
    _var: &Expr,
    _value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // Return null in no_std context
    QueryResult::Owned(OwnedValue::Null)
}

// String functions

/// Builtin: trim - remove leading/trailing whitespace
fn builtin_trim<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::String(cow.trim().into()))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new("trim requires string")),
    }
}

/// Builtin: ltrim - remove leading whitespace
fn builtin_ltrim<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::String(cow.trim_start().into()))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new("ltrim requires string")),
    }
}

/// Builtin: rtrim - remove trailing whitespace
fn builtin_rtrim<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::String(cow.trim_end().into()))
            } else {
                QueryResult::Owned(OwnedValue::String(String::new()))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new("rtrim requires string")),
    }
}

// Array functions

/// Builtin: transpose - transpose array of arrays
fn builtin_transpose<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let elements = match value {
        StandardJson::Array(a) => a,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::new("transpose requires array")),
    };

    // Collect all inner arrays
    let mut inner_arrays: Vec<Vec<OwnedValue>> = Vec::new();
    for item in elements {
        match item {
            StandardJson::Array(inner) => {
                inner_arrays.push(inner.map(|v| to_owned(&v)).collect());
            }
            _ => {
                // Non-array elements are treated as single-element arrays
                inner_arrays.push(vec![to_owned(&item)]);
            }
        }
    }

    if inner_arrays.is_empty() {
        return QueryResult::Owned(OwnedValue::Array(vec![]));
    }

    // Find max length
    let max_len = inner_arrays.iter().map(|a| a.len()).max().unwrap_or(0);

    // Build transposed result
    let mut result = Vec::with_capacity(max_len);
    for i in 0..max_len {
        let mut row = Vec::new();
        for inner in &inner_arrays {
            if let Some(val) = inner.get(i) {
                row.push(val.clone());
            }
        }
        result.push(OwnedValue::Array(row));
    }

    QueryResult::Owned(OwnedValue::Array(result))
}

/// Helper to compare OwnedValue for binary search
fn compare_owned_values(a: &OwnedValue, b: &OwnedValue) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    match (a, b) {
        (OwnedValue::Null, OwnedValue::Null) => Ordering::Equal,
        (OwnedValue::Bool(x), OwnedValue::Bool(y)) => x.cmp(y),
        (OwnedValue::Int(x), OwnedValue::Int(y)) => x.cmp(y),
        (OwnedValue::Float(x), OwnedValue::Float(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (OwnedValue::Int(x), OwnedValue::Float(y)) => {
            (*x as f64).partial_cmp(y).unwrap_or(Ordering::Equal)
        }
        (OwnedValue::Float(x), OwnedValue::Int(y)) => {
            x.partial_cmp(&(*y as f64)).unwrap_or(Ordering::Equal)
        }
        (OwnedValue::String(x), OwnedValue::String(y)) => x.cmp(y),
        // Different types: use a consistent ordering
        _ => {
            let type_order = |v: &OwnedValue| match v {
                OwnedValue::Null => 0,
                OwnedValue::Bool(_) => 1,
                OwnedValue::Int(_) | OwnedValue::Float(_) => 2,
                OwnedValue::String(_) => 3,
                OwnedValue::Array(_) => 4,
                OwnedValue::Object(_) => 5,
            };
            type_order(a).cmp(&type_order(b))
        }
    }
}

/// Builtin: bsearch(x) - binary search for x in sorted array
fn builtin_bsearch<'a, W: Clone + AsRef<[u64]>>(
    x_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let elements_iter = match value.clone() {
        StandardJson::Array(a) => a,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::new("bsearch requires array")),
    };

    // Evaluate search value
    let x = match eval_single(x_expr, value, optional) {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::None,
    };

    // Collect array elements
    let elements: Vec<OwnedValue> = elements_iter.map(|v| to_owned(&v)).collect();

    // Binary search
    match elements.binary_search_by(|probe| compare_owned_values(probe, &x)) {
        Ok(idx) => QueryResult::Owned(OwnedValue::Int(idx as i64)),
        Err(idx) => {
            // jq returns an object with "index" field when not found
            let mut obj = IndexMap::new();
            obj.insert("index".to_string(), OwnedValue::Int(idx as i64));
            QueryResult::Owned(OwnedValue::Object(obj))
        }
    }
}

// Object functions

/// Builtin: modulemeta(name) - get module metadata (stub for compatibility)
fn builtin_modulemeta<'a, W: Clone + AsRef<[u64]>>(
    _name: &Expr,
    _value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // Return null as we don't support modules
    QueryResult::Owned(OwnedValue::Null)
}

// ============================================================================
// Phase 9: Variables & Definitions
// ============================================================================

/// Evaluate destructuring pattern binding: `expr as {key: $var, ...} | body`.
fn eval_as_pattern<'a, W: Clone + AsRef<[u64]>>(
    expr: &Expr,
    pattern: &Pattern,
    body: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the expression to get the value to destructure
    let bound_result = eval_single(expr, value.clone(), optional);

    let bound_values: Vec<OwnedValue> = match bound_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
    };

    let mut all_results: Vec<OwnedValue> = Vec::new();

    for bound_val in bound_values {
        // Extract bindings from the pattern
        let bindings = match extract_pattern_bindings(pattern, &bound_val) {
            Ok(b) => b,
            Err(e) => return QueryResult::Error(e),
        };

        // Substitute all bindings in the body
        let mut substituted_body = body.clone();
        for (var_name, var_value) in &bindings {
            substituted_body = substitute_var(&substituted_body, var_name, var_value);
        }

        match eval_single(&substituted_body, value.clone(), optional).materialize_cursor() {
            QueryResult::One(v) => all_results.push(to_owned(&v)),
            QueryResult::OneCursor(_) => unreachable!(),
            QueryResult::Many(vs) => all_results.extend(vs.iter().map(to_owned)),
            QueryResult::Owned(v) => all_results.push(v),
            QueryResult::ManyOwned(vs) => all_results.extend(vs),
            QueryResult::None => {}
            QueryResult::Error(e) => return QueryResult::Error(e),
        }
    }

    if all_results.is_empty() {
        QueryResult::None
    } else if all_results.len() == 1 {
        QueryResult::Owned(all_results.pop().unwrap())
    } else {
        QueryResult::ManyOwned(all_results)
    }
}

/// Extract variable bindings from a pattern matching an OwnedValue.
fn extract_pattern_bindings(
    pattern: &Pattern,
    value: &OwnedValue,
) -> Result<Vec<(String, OwnedValue)>, EvalError> {
    match pattern {
        Pattern::Var(name) => Ok(vec![(name.clone(), value.clone())]),
        Pattern::Object(entries) => {
            let obj = match value {
                OwnedValue::Object(o) => o,
                _ => return Err(EvalError::type_error("object", owned_type_name(value))),
            };
            let mut bindings = Vec::new();
            for entry in entries {
                let field_value = obj.get(&entry.key).cloned().unwrap_or(OwnedValue::Null);
                let sub_bindings = extract_pattern_bindings(&entry.pattern, &field_value)?;
                bindings.extend(sub_bindings);
            }
            Ok(bindings)
        }
        Pattern::Array(patterns) => {
            let arr = match value {
                OwnedValue::Array(a) => a,
                _ => return Err(EvalError::type_error("array", owned_type_name(value))),
            };
            let mut bindings = Vec::new();
            for (i, pat) in patterns.iter().enumerate() {
                let elem_value = arr.get(i).cloned().unwrap_or(OwnedValue::Null);
                let sub_bindings = extract_pattern_bindings(pat, &elem_value)?;
                bindings.extend(sub_bindings);
            }
            Ok(bindings)
        }
    }
}

/// Get a type name for an OwnedValue.
fn owned_type_name(value: &OwnedValue) -> &'static str {
    match value {
        OwnedValue::Null => "null",
        OwnedValue::Bool(_) => "boolean",
        OwnedValue::Int(_) | OwnedValue::Float(_) => "number",
        OwnedValue::String(_) => "string",
        OwnedValue::Array(_) => "array",
        OwnedValue::Object(_) => "object",
    }
}

/// Evaluate function definition: `def name(params): body; then`.
///
/// In jq, function definitions are scoped - the function is available in `then`.
/// We implement this by substituting function calls with the body (with args substituted).
fn eval_func_def<'a, W: Clone + AsRef<[u64]>>(
    name: &str,
    params: &[String],
    body: &Expr,
    then: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Substitute all calls to this function in `then` with the body
    let expanded_then = expand_func_calls(then, name, params, body);
    eval_single(&expanded_then, value, optional)
}

/// Expand function calls to a defined function by inlining the body.
fn expand_func_calls(expr: &Expr, func_name: &str, params: &[String], body: &Expr) -> Expr {
    match expr {
        Expr::FuncCall { name, args } if name == func_name => {
            // Check arity
            if args.len() != params.len() {
                // Return an error expression - wrong number of arguments
                // Use a string literal as the error message
                return Expr::Error(Some(Box::new(Expr::Literal(Literal::String(format!(
                    "function {} takes {} arguments, got {}",
                    func_name,
                    params.len(),
                    args.len()
                ))))));
            }
            // Substitute parameters with arguments in the body
            let mut result = body.clone();
            // First, expand any nested function calls in the arguments
            let expanded_args: Vec<Expr> = args
                .iter()
                .map(|a| expand_func_calls(a, func_name, params, body))
                .collect();
            // Then substitute each parameter
            for (param, arg) in params.iter().zip(expanded_args.iter()) {
                result = substitute_func_param(&result, param, arg);
            }
            // Also expand any recursive calls in the result
            expand_func_calls(&result, func_name, params, body)
        }
        // Recursively expand in all subexpressions
        Expr::Identity => Expr::Identity,
        Expr::Field(s) => Expr::Field(s.clone()),
        Expr::Index(i) => Expr::Index(*i),
        Expr::Slice { start, end } => Expr::Slice {
            start: *start,
            end: *end,
        },
        Expr::Iterate => Expr::Iterate,
        Expr::RecursiveDescent => Expr::RecursiveDescent,
        Expr::Optional(e) => {
            Expr::Optional(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Expr::Pipe(exprs) => Expr::Pipe(
            exprs
                .iter()
                .map(|e| expand_func_calls(e, func_name, params, body))
                .collect(),
        ),
        Expr::Comma(exprs) => Expr::Comma(
            exprs
                .iter()
                .map(|e| expand_func_calls(e, func_name, params, body))
                .collect(),
        ),
        Expr::Array(e) => Expr::Array(Box::new(expand_func_calls(e, func_name, params, body))),
        Expr::Object(entries) => Expr::Object(
            entries
                .iter()
                .map(|entry| {
                    let new_key = match &entry.key {
                        ObjectKey::Literal(s) => ObjectKey::Literal(s.clone()),
                        ObjectKey::Expr(e) => {
                            ObjectKey::Expr(Box::new(expand_func_calls(e, func_name, params, body)))
                        }
                    };
                    ObjectEntry {
                        key: new_key,
                        value: expand_func_calls(&entry.value, func_name, params, body),
                    }
                })
                .collect(),
        ),
        Expr::Literal(lit) => Expr::Literal(lit.clone()),
        Expr::Paren(e) => Expr::Paren(Box::new(expand_func_calls(e, func_name, params, body))),
        Expr::Arithmetic { op, left, right } => Expr::Arithmetic {
            op: *op,
            left: Box::new(expand_func_calls(left, func_name, params, body)),
            right: Box::new(expand_func_calls(right, func_name, params, body)),
        },
        Expr::Compare { op, left, right } => Expr::Compare {
            op: *op,
            left: Box::new(expand_func_calls(left, func_name, params, body)),
            right: Box::new(expand_func_calls(right, func_name, params, body)),
        },
        Expr::And(l, r) => Expr::And(
            Box::new(expand_func_calls(l, func_name, params, body)),
            Box::new(expand_func_calls(r, func_name, params, body)),
        ),
        Expr::Or(l, r) => Expr::Or(
            Box::new(expand_func_calls(l, func_name, params, body)),
            Box::new(expand_func_calls(r, func_name, params, body)),
        ),
        Expr::Not => Expr::Not,
        Expr::Alternative(l, r) => Expr::Alternative(
            Box::new(expand_func_calls(l, func_name, params, body)),
            Box::new(expand_func_calls(r, func_name, params, body)),
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(expand_func_calls(cond, func_name, params, body)),
            then_branch: Box::new(expand_func_calls(then_branch, func_name, params, body)),
            else_branch: Box::new(expand_func_calls(else_branch, func_name, params, body)),
        },
        Expr::Try { expr, catch } => Expr::Try {
            expr: Box::new(expand_func_calls(expr, func_name, params, body)),
            catch: catch
                .as_ref()
                .map(|c| Box::new(expand_func_calls(c, func_name, params, body))),
        },
        Expr::Error(msg) => Expr::Error(msg.clone()),
        Expr::Builtin(b) => Expr::Builtin(expand_func_calls_in_builtin(b, func_name, params, body)),
        Expr::StringInterpolation(parts) => Expr::StringInterpolation(
            parts
                .iter()
                .map(|p| match p {
                    StringPart::Literal(s) => StringPart::Literal(s.clone()),
                    StringPart::Expr(e) => {
                        StringPart::Expr(Box::new(expand_func_calls(e, func_name, params, body)))
                    }
                })
                .collect(),
        ),
        Expr::Format(f) => Expr::Format(*f),
        Expr::Var(v) => Expr::Var(v.clone()),
        Expr::As {
            expr,
            var,
            body: as_body,
        } => Expr::As {
            expr: Box::new(expand_func_calls(expr, func_name, params, body)),
            var: var.clone(),
            body: Box::new(expand_func_calls(as_body, func_name, params, body)),
        },
        Expr::Reduce {
            input,
            var,
            init,
            update,
        } => Expr::Reduce {
            input: Box::new(expand_func_calls(input, func_name, params, body)),
            var: var.clone(),
            init: Box::new(expand_func_calls(init, func_name, params, body)),
            update: Box::new(expand_func_calls(update, func_name, params, body)),
        },
        Expr::Foreach {
            input,
            var,
            init,
            update,
            extract,
        } => Expr::Foreach {
            input: Box::new(expand_func_calls(input, func_name, params, body)),
            var: var.clone(),
            init: Box::new(expand_func_calls(init, func_name, params, body)),
            update: Box::new(expand_func_calls(update, func_name, params, body)),
            extract: extract
                .as_ref()
                .map(|e| Box::new(expand_func_calls(e, func_name, params, body))),
        },
        Expr::Limit { n, expr } => Expr::Limit {
            n: Box::new(expand_func_calls(n, func_name, params, body)),
            expr: Box::new(expand_func_calls(expr, func_name, params, body)),
        },
        Expr::FirstExpr(e) => {
            Expr::FirstExpr(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Expr::LastExpr(e) => {
            Expr::LastExpr(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Expr::NthExpr { n, expr } => Expr::NthExpr {
            n: Box::new(expand_func_calls(n, func_name, params, body)),
            expr: Box::new(expand_func_calls(expr, func_name, params, body)),
        },
        Expr::Until { cond, update } => Expr::Until {
            cond: Box::new(expand_func_calls(cond, func_name, params, body)),
            update: Box::new(expand_func_calls(update, func_name, params, body)),
        },
        Expr::While { cond, update } => Expr::While {
            cond: Box::new(expand_func_calls(cond, func_name, params, body)),
            update: Box::new(expand_func_calls(update, func_name, params, body)),
        },
        Expr::Repeat(e) => Expr::Repeat(Box::new(expand_func_calls(e, func_name, params, body))),
        Expr::Range { from, to, step } => Expr::Range {
            from: Box::new(expand_func_calls(from, func_name, params, body)),
            to: to
                .as_ref()
                .map(|e| Box::new(expand_func_calls(e, func_name, params, body))),
            step: step
                .as_ref()
                .map(|e| Box::new(expand_func_calls(e, func_name, params, body))),
        },
        Expr::AsPattern {
            expr,
            pattern,
            body: pattern_body,
        } => Expr::AsPattern {
            expr: Box::new(expand_func_calls(expr, func_name, params, body)),
            pattern: pattern.clone(),
            body: Box::new(expand_func_calls(pattern_body, func_name, params, body)),
        },
        Expr::FuncDef {
            name: inner_name,
            params: inner_params,
            body: inner_body,
            then,
        } => {
            // If this defines the same function name, it shadows our function
            if inner_name == func_name {
                // Don't expand in the body or then - this is a new definition
                expr.clone()
            } else {
                Expr::FuncDef {
                    name: inner_name.clone(),
                    params: inner_params.clone(),
                    body: Box::new(expand_func_calls(inner_body, func_name, params, body)),
                    then: Box::new(expand_func_calls(then, func_name, params, body)),
                }
            }
        }
        Expr::FuncCall { name, args } => {
            // Different function name, just expand in arguments
            Expr::FuncCall {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|a| expand_func_calls(a, func_name, params, body))
                    .collect(),
            }
        }
        Expr::NamespacedCall {
            namespace,
            name,
            args,
        } => Expr::NamespacedCall {
            namespace: namespace.clone(),
            name: name.clone(),
            args: args
                .iter()
                .map(|a| expand_func_calls(a, func_name, params, body))
                .collect(),
        },
    }
}

/// Substitute a function parameter with an argument expression.
fn substitute_func_param(expr: &Expr, param: &str, arg: &Expr) -> Expr {
    match expr {
        // A variable reference to the parameter becomes the argument expression
        Expr::Var(name) if name == param => arg.clone(),
        Expr::Var(_) => expr.clone(),
        Expr::Identity => Expr::Identity,
        Expr::Field(name) => Expr::Field(name.clone()),
        Expr::Index(i) => Expr::Index(*i),
        Expr::Slice { start, end } => Expr::Slice {
            start: *start,
            end: *end,
        },
        Expr::Iterate => Expr::Iterate,
        Expr::RecursiveDescent => Expr::RecursiveDescent,
        Expr::Optional(e) => Expr::Optional(Box::new(substitute_func_param(e, param, arg))),
        Expr::Pipe(exprs) => Expr::Pipe(
            exprs
                .iter()
                .map(|e| substitute_func_param(e, param, arg))
                .collect(),
        ),
        Expr::Comma(exprs) => Expr::Comma(
            exprs
                .iter()
                .map(|e| substitute_func_param(e, param, arg))
                .collect(),
        ),
        Expr::Array(e) => Expr::Array(Box::new(substitute_func_param(e, param, arg))),
        Expr::Object(entries) => Expr::Object(
            entries
                .iter()
                .map(|entry| {
                    let new_key = match &entry.key {
                        ObjectKey::Literal(s) => ObjectKey::Literal(s.clone()),
                        ObjectKey::Expr(e) => {
                            ObjectKey::Expr(Box::new(substitute_func_param(e, param, arg)))
                        }
                    };
                    ObjectEntry {
                        key: new_key,
                        value: substitute_func_param(&entry.value, param, arg),
                    }
                })
                .collect(),
        ),
        Expr::Literal(lit) => Expr::Literal(lit.clone()),
        Expr::Paren(e) => Expr::Paren(Box::new(substitute_func_param(e, param, arg))),
        Expr::Arithmetic { op, left, right } => Expr::Arithmetic {
            op: *op,
            left: Box::new(substitute_func_param(left, param, arg)),
            right: Box::new(substitute_func_param(right, param, arg)),
        },
        Expr::Compare { op, left, right } => Expr::Compare {
            op: *op,
            left: Box::new(substitute_func_param(left, param, arg)),
            right: Box::new(substitute_func_param(right, param, arg)),
        },
        Expr::And(l, r) => Expr::And(
            Box::new(substitute_func_param(l, param, arg)),
            Box::new(substitute_func_param(r, param, arg)),
        ),
        Expr::Or(l, r) => Expr::Or(
            Box::new(substitute_func_param(l, param, arg)),
            Box::new(substitute_func_param(r, param, arg)),
        ),
        Expr::Not => Expr::Not,
        Expr::Alternative(l, r) => Expr::Alternative(
            Box::new(substitute_func_param(l, param, arg)),
            Box::new(substitute_func_param(r, param, arg)),
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(substitute_func_param(cond, param, arg)),
            then_branch: Box::new(substitute_func_param(then_branch, param, arg)),
            else_branch: Box::new(substitute_func_param(else_branch, param, arg)),
        },
        Expr::Try { expr, catch } => Expr::Try {
            expr: Box::new(substitute_func_param(expr, param, arg)),
            catch: catch
                .as_ref()
                .map(|c| Box::new(substitute_func_param(c, param, arg))),
        },
        Expr::Error(msg) => Expr::Error(msg.clone()),
        Expr::Builtin(b) => Expr::Builtin(substitute_func_param_in_builtin(b, param, arg)),
        Expr::StringInterpolation(parts) => Expr::StringInterpolation(
            parts
                .iter()
                .map(|p| match p {
                    StringPart::Literal(s) => StringPart::Literal(s.clone()),
                    StringPart::Expr(e) => {
                        StringPart::Expr(Box::new(substitute_func_param(e, param, arg)))
                    }
                })
                .collect(),
        ),
        Expr::Format(f) => Expr::Format(*f),
        Expr::As { expr, var, body } => {
            // If var shadows param, don't substitute in body
            if var == param {
                Expr::As {
                    expr: Box::new(substitute_func_param(expr, param, arg)),
                    var: var.clone(),
                    body: body.clone(),
                }
            } else {
                Expr::As {
                    expr: Box::new(substitute_func_param(expr, param, arg)),
                    var: var.clone(),
                    body: Box::new(substitute_func_param(body, param, arg)),
                }
            }
        }
        Expr::Reduce {
            input,
            var,
            init,
            update,
        } => {
            if var == param {
                Expr::Reduce {
                    input: Box::new(substitute_func_param(input, param, arg)),
                    var: var.clone(),
                    init: Box::new(substitute_func_param(init, param, arg)),
                    update: update.clone(),
                }
            } else {
                Expr::Reduce {
                    input: Box::new(substitute_func_param(input, param, arg)),
                    var: var.clone(),
                    init: Box::new(substitute_func_param(init, param, arg)),
                    update: Box::new(substitute_func_param(update, param, arg)),
                }
            }
        }
        Expr::Foreach {
            input,
            var,
            init,
            update,
            extract,
        } => {
            if var == param {
                Expr::Foreach {
                    input: Box::new(substitute_func_param(input, param, arg)),
                    var: var.clone(),
                    init: Box::new(substitute_func_param(init, param, arg)),
                    update: update.clone(),
                    extract: extract.clone(),
                }
            } else {
                Expr::Foreach {
                    input: Box::new(substitute_func_param(input, param, arg)),
                    var: var.clone(),
                    init: Box::new(substitute_func_param(init, param, arg)),
                    update: Box::new(substitute_func_param(update, param, arg)),
                    extract: extract
                        .as_ref()
                        .map(|e| Box::new(substitute_func_param(e, param, arg))),
                }
            }
        }
        Expr::Limit { n, expr } => Expr::Limit {
            n: Box::new(substitute_func_param(n, param, arg)),
            expr: Box::new(substitute_func_param(expr, param, arg)),
        },
        Expr::FirstExpr(e) => Expr::FirstExpr(Box::new(substitute_func_param(e, param, arg))),
        Expr::LastExpr(e) => Expr::LastExpr(Box::new(substitute_func_param(e, param, arg))),
        Expr::NthExpr { n, expr } => Expr::NthExpr {
            n: Box::new(substitute_func_param(n, param, arg)),
            expr: Box::new(substitute_func_param(expr, param, arg)),
        },
        Expr::Until { cond, update } => Expr::Until {
            cond: Box::new(substitute_func_param(cond, param, arg)),
            update: Box::new(substitute_func_param(update, param, arg)),
        },
        Expr::While { cond, update } => Expr::While {
            cond: Box::new(substitute_func_param(cond, param, arg)),
            update: Box::new(substitute_func_param(update, param, arg)),
        },
        Expr::Repeat(e) => Expr::Repeat(Box::new(substitute_func_param(e, param, arg))),
        Expr::Range { from, to, step } => Expr::Range {
            from: Box::new(substitute_func_param(from, param, arg)),
            to: to
                .as_ref()
                .map(|e| Box::new(substitute_func_param(e, param, arg))),
            step: step
                .as_ref()
                .map(|e| Box::new(substitute_func_param(e, param, arg))),
        },
        Expr::AsPattern {
            expr,
            pattern,
            body,
        } => {
            let shadowed = pattern_binds_var(pattern, param);
            Expr::AsPattern {
                expr: Box::new(substitute_func_param(expr, param, arg)),
                pattern: pattern.clone(),
                body: if shadowed {
                    body.clone()
                } else {
                    Box::new(substitute_func_param(body, param, arg))
                },
            }
        }
        Expr::FuncDef {
            name,
            params,
            body,
            then,
        } => {
            let shadowed = params.contains(&param.to_string());
            Expr::FuncDef {
                name: name.clone(),
                params: params.clone(),
                body: if shadowed {
                    body.clone()
                } else {
                    Box::new(substitute_func_param(body, param, arg))
                },
                then: Box::new(substitute_func_param(then, param, arg)),
            }
        }
        Expr::FuncCall { name, args } => {
            // In jq, function parameters are bare identifiers that parse as zero-arg FuncCalls
            // Check if this is a reference to the parameter
            if name == param && args.is_empty() {
                arg.clone()
            } else {
                Expr::FuncCall {
                    name: name.clone(),
                    args: args
                        .iter()
                        .map(|a| substitute_func_param(a, param, arg))
                        .collect(),
                }
            }
        }
        Expr::NamespacedCall {
            namespace,
            name,
            args,
        } => Expr::NamespacedCall {
            namespace: namespace.clone(),
            name: name.clone(),
            args: args
                .iter()
                .map(|a| substitute_func_param(a, param, arg))
                .collect(),
        },
    }
}

/// Expand function calls in a builtin expression.
fn expand_func_calls_in_builtin(
    builtin: &Builtin,
    func_name: &str,
    params: &[String],
    body: &Expr,
) -> Builtin {
    match builtin {
        Builtin::Type => Builtin::Type,
        Builtin::IsNull => Builtin::IsNull,
        Builtin::IsBoolean => Builtin::IsBoolean,
        Builtin::IsNumber => Builtin::IsNumber,
        Builtin::IsString => Builtin::IsString,
        Builtin::IsArray => Builtin::IsArray,
        Builtin::IsObject => Builtin::IsObject,
        Builtin::Length => Builtin::Length,
        Builtin::Utf8ByteLength => Builtin::Utf8ByteLength,
        Builtin::Keys => Builtin::Keys,
        Builtin::KeysUnsorted => Builtin::KeysUnsorted,
        Builtin::Has(e) => Builtin::Has(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::In(e) => Builtin::In(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::Select(e) => {
            Builtin::Select(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Empty => Builtin::Empty,
        Builtin::Map(e) => Builtin::Map(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::MapValues(e) => {
            Builtin::MapValues(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Add => Builtin::Add,
        Builtin::Any => Builtin::Any,
        Builtin::All => Builtin::All,
        Builtin::Min => Builtin::Min,
        Builtin::Max => Builtin::Max,
        Builtin::MinBy(e) => {
            Builtin::MinBy(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::MaxBy(e) => {
            Builtin::MaxBy(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::AsciiDowncase => Builtin::AsciiDowncase,
        Builtin::AsciiUpcase => Builtin::AsciiUpcase,
        Builtin::Ltrimstr(e) => {
            Builtin::Ltrimstr(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Rtrimstr(e) => {
            Builtin::Rtrimstr(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Startswith(e) => {
            Builtin::Startswith(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Endswith(e) => {
            Builtin::Endswith(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Split(e) => {
            Builtin::Split(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Join(e) => Builtin::Join(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::Contains(e) => {
            Builtin::Contains(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Inside(e) => {
            Builtin::Inside(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::First => Builtin::First,
        Builtin::Last => Builtin::Last,
        Builtin::Nth(e) => Builtin::Nth(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::Reverse => Builtin::Reverse,
        Builtin::Flatten => Builtin::Flatten,
        Builtin::FlattenDepth(e) => {
            Builtin::FlattenDepth(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::GroupBy(e) => {
            Builtin::GroupBy(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Unique => Builtin::Unique,
        Builtin::UniqueBy(e) => {
            Builtin::UniqueBy(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Sort => Builtin::Sort,
        Builtin::SortBy(e) => {
            Builtin::SortBy(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::ToEntries => Builtin::ToEntries,
        Builtin::FromEntries => Builtin::FromEntries,
        Builtin::WithEntries(e) => {
            Builtin::WithEntries(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::ToString => Builtin::ToString,
        Builtin::ToNumber => Builtin::ToNumber,
        Builtin::Explode => Builtin::Explode,
        Builtin::Implode => Builtin::Implode,
        Builtin::Test(e) => Builtin::Test(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::Indices(e) => {
            Builtin::Indices(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Index(e) => {
            Builtin::Index(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Rindex(e) => {
            Builtin::Rindex(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::ToJsonStream => Builtin::ToJsonStream,
        Builtin::FromJsonStream => Builtin::FromJsonStream,
        Builtin::GetPath(e) => {
            Builtin::GetPath(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        #[cfg(feature = "regex")]
        Builtin::Match(re, flags) => Builtin::Match(
            Box::new(expand_func_calls(re, func_name, params, body)),
            flags.clone(),
        ),
        #[cfg(feature = "regex")]
        Builtin::Capture(e) => {
            Builtin::Capture(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        #[cfg(feature = "regex")]
        Builtin::Scan(e) => Builtin::Scan(Box::new(expand_func_calls(e, func_name, params, body))),
        #[cfg(feature = "regex")]
        Builtin::Splits(e) => {
            Builtin::Splits(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        #[cfg(feature = "regex")]
        Builtin::Sub(re, repl) => Builtin::Sub(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(repl, func_name, params, body)),
        ),
        #[cfg(feature = "regex")]
        Builtin::Gsub(re, repl) => Builtin::Gsub(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(repl, func_name, params, body)),
        ),
        #[cfg(feature = "regex")]
        Builtin::TestWithFlags(re, flags) => Builtin::TestWithFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            flags.clone(),
        ),
        Builtin::Recurse => Builtin::Recurse,
        Builtin::RecurseF(f) => {
            Builtin::RecurseF(Box::new(expand_func_calls(f, func_name, params, body)))
        }
        Builtin::RecurseCond(f, c) => Builtin::RecurseCond(
            Box::new(expand_func_calls(f, func_name, params, body)),
            Box::new(expand_func_calls(c, func_name, params, body)),
        ),
        Builtin::Walk(f) => Builtin::Walk(Box::new(expand_func_calls(f, func_name, params, body))),
        Builtin::IsValid(e) => {
            Builtin::IsValid(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        // Phase 10 builtins
        Builtin::Path(e) => Builtin::Path(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::Paths => Builtin::Paths,
        Builtin::PathsFilter(e) => {
            Builtin::PathsFilter(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::LeafPaths => Builtin::LeafPaths,
        Builtin::SetPath(p, v) => Builtin::SetPath(
            Box::new(expand_func_calls(p, func_name, params, body)),
            Box::new(expand_func_calls(v, func_name, params, body)),
        ),
        Builtin::DelPaths(e) => {
            Builtin::DelPaths(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Floor => Builtin::Floor,
        Builtin::Ceil => Builtin::Ceil,
        Builtin::Round => Builtin::Round,
        Builtin::Sqrt => Builtin::Sqrt,
        Builtin::Fabs => Builtin::Fabs,
        Builtin::Log => Builtin::Log,
        Builtin::Log10 => Builtin::Log10,
        Builtin::Log2 => Builtin::Log2,
        Builtin::Exp => Builtin::Exp,
        Builtin::Exp10 => Builtin::Exp10,
        Builtin::Exp2 => Builtin::Exp2,
        Builtin::Pow(x, y) => Builtin::Pow(
            Box::new(expand_func_calls(x, func_name, params, body)),
            Box::new(expand_func_calls(y, func_name, params, body)),
        ),
        Builtin::Sin => Builtin::Sin,
        Builtin::Cos => Builtin::Cos,
        Builtin::Tan => Builtin::Tan,
        Builtin::Asin => Builtin::Asin,
        Builtin::Acos => Builtin::Acos,
        Builtin::Atan => Builtin::Atan,
        Builtin::Atan2(x, y) => Builtin::Atan2(
            Box::new(expand_func_calls(x, func_name, params, body)),
            Box::new(expand_func_calls(y, func_name, params, body)),
        ),
        Builtin::Sinh => Builtin::Sinh,
        Builtin::Cosh => Builtin::Cosh,
        Builtin::Tanh => Builtin::Tanh,
        Builtin::Asinh => Builtin::Asinh,
        Builtin::Acosh => Builtin::Acosh,
        Builtin::Atanh => Builtin::Atanh,
        Builtin::Infinite => Builtin::Infinite,
        Builtin::Nan => Builtin::Nan,
        Builtin::IsInfinite => Builtin::IsInfinite,
        Builtin::IsNan => Builtin::IsNan,
        Builtin::IsNormal => Builtin::IsNormal,
        Builtin::IsFinite => Builtin::IsFinite,
        Builtin::Debug => Builtin::Debug,
        Builtin::DebugMsg(e) => {
            Builtin::DebugMsg(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Env => Builtin::Env,
        Builtin::EnvVar(e) => {
            Builtin::EnvVar(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::NullLit => Builtin::NullLit,
        Builtin::Trim => Builtin::Trim,
        Builtin::Ltrim => Builtin::Ltrim,
        Builtin::Rtrim => Builtin::Rtrim,
        Builtin::Transpose => Builtin::Transpose,
        Builtin::BSearch(e) => {
            Builtin::BSearch(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::ModuleMeta(e) => {
            Builtin::ModuleMeta(Box::new(expand_func_calls(e, func_name, params, body)))
        }
    }
}

/// Substitute function parameter in a builtin expression.
fn substitute_func_param_in_builtin(builtin: &Builtin, param: &str, arg: &Expr) -> Builtin {
    match builtin {
        Builtin::Type => Builtin::Type,
        Builtin::IsNull => Builtin::IsNull,
        Builtin::IsBoolean => Builtin::IsBoolean,
        Builtin::IsNumber => Builtin::IsNumber,
        Builtin::IsString => Builtin::IsString,
        Builtin::IsArray => Builtin::IsArray,
        Builtin::IsObject => Builtin::IsObject,
        Builtin::Length => Builtin::Length,
        Builtin::Utf8ByteLength => Builtin::Utf8ByteLength,
        Builtin::Keys => Builtin::Keys,
        Builtin::KeysUnsorted => Builtin::KeysUnsorted,
        Builtin::Has(e) => Builtin::Has(Box::new(substitute_func_param(e, param, arg))),
        Builtin::In(e) => Builtin::In(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Select(e) => Builtin::Select(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Empty => Builtin::Empty,
        Builtin::Map(e) => Builtin::Map(Box::new(substitute_func_param(e, param, arg))),
        Builtin::MapValues(e) => Builtin::MapValues(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Add => Builtin::Add,
        Builtin::Any => Builtin::Any,
        Builtin::All => Builtin::All,
        Builtin::Min => Builtin::Min,
        Builtin::Max => Builtin::Max,
        Builtin::MinBy(e) => Builtin::MinBy(Box::new(substitute_func_param(e, param, arg))),
        Builtin::MaxBy(e) => Builtin::MaxBy(Box::new(substitute_func_param(e, param, arg))),
        Builtin::AsciiDowncase => Builtin::AsciiDowncase,
        Builtin::AsciiUpcase => Builtin::AsciiUpcase,
        Builtin::Ltrimstr(e) => Builtin::Ltrimstr(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Rtrimstr(e) => Builtin::Rtrimstr(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Startswith(e) => {
            Builtin::Startswith(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::Endswith(e) => Builtin::Endswith(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Split(e) => Builtin::Split(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Join(e) => Builtin::Join(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Contains(e) => Builtin::Contains(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Inside(e) => Builtin::Inside(Box::new(substitute_func_param(e, param, arg))),
        Builtin::First => Builtin::First,
        Builtin::Last => Builtin::Last,
        Builtin::Nth(e) => Builtin::Nth(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Reverse => Builtin::Reverse,
        Builtin::Flatten => Builtin::Flatten,
        Builtin::FlattenDepth(e) => {
            Builtin::FlattenDepth(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::GroupBy(e) => Builtin::GroupBy(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Unique => Builtin::Unique,
        Builtin::UniqueBy(e) => Builtin::UniqueBy(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Sort => Builtin::Sort,
        Builtin::SortBy(e) => Builtin::SortBy(Box::new(substitute_func_param(e, param, arg))),
        Builtin::ToEntries => Builtin::ToEntries,
        Builtin::FromEntries => Builtin::FromEntries,
        Builtin::WithEntries(e) => {
            Builtin::WithEntries(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::ToString => Builtin::ToString,
        Builtin::ToNumber => Builtin::ToNumber,
        Builtin::Explode => Builtin::Explode,
        Builtin::Implode => Builtin::Implode,
        Builtin::Test(e) => Builtin::Test(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Indices(e) => Builtin::Indices(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Index(e) => Builtin::Index(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Rindex(e) => Builtin::Rindex(Box::new(substitute_func_param(e, param, arg))),
        Builtin::ToJsonStream => Builtin::ToJsonStream,
        Builtin::FromJsonStream => Builtin::FromJsonStream,
        Builtin::GetPath(e) => Builtin::GetPath(Box::new(substitute_func_param(e, param, arg))),
        #[cfg(feature = "regex")]
        Builtin::Match(re, flags) => Builtin::Match(
            Box::new(substitute_func_param(re, param, arg)),
            flags.clone(),
        ),
        #[cfg(feature = "regex")]
        Builtin::Capture(e) => Builtin::Capture(Box::new(substitute_func_param(e, param, arg))),
        #[cfg(feature = "regex")]
        Builtin::Scan(e) => Builtin::Scan(Box::new(substitute_func_param(e, param, arg))),
        #[cfg(feature = "regex")]
        Builtin::Splits(e) => Builtin::Splits(Box::new(substitute_func_param(e, param, arg))),
        #[cfg(feature = "regex")]
        Builtin::Sub(re, repl) => Builtin::Sub(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(repl, param, arg)),
        ),
        #[cfg(feature = "regex")]
        Builtin::Gsub(re, repl) => Builtin::Gsub(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(repl, param, arg)),
        ),
        #[cfg(feature = "regex")]
        Builtin::TestWithFlags(re, flags) => Builtin::TestWithFlags(
            Box::new(substitute_func_param(re, param, arg)),
            flags.clone(),
        ),
        Builtin::Recurse => Builtin::Recurse,
        Builtin::RecurseF(f) => Builtin::RecurseF(Box::new(substitute_func_param(f, param, arg))),
        Builtin::RecurseCond(f, c) => Builtin::RecurseCond(
            Box::new(substitute_func_param(f, param, arg)),
            Box::new(substitute_func_param(c, param, arg)),
        ),
        Builtin::Walk(f) => Builtin::Walk(Box::new(substitute_func_param(f, param, arg))),
        Builtin::IsValid(e) => Builtin::IsValid(Box::new(substitute_func_param(e, param, arg))),
        // Phase 10 builtins
        Builtin::Path(e) => Builtin::Path(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Paths => Builtin::Paths,
        Builtin::PathsFilter(e) => {
            Builtin::PathsFilter(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::LeafPaths => Builtin::LeafPaths,
        Builtin::SetPath(p, v) => Builtin::SetPath(
            Box::new(substitute_func_param(p, param, arg)),
            Box::new(substitute_func_param(v, param, arg)),
        ),
        Builtin::DelPaths(e) => Builtin::DelPaths(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Floor => Builtin::Floor,
        Builtin::Ceil => Builtin::Ceil,
        Builtin::Round => Builtin::Round,
        Builtin::Sqrt => Builtin::Sqrt,
        Builtin::Fabs => Builtin::Fabs,
        Builtin::Log => Builtin::Log,
        Builtin::Log10 => Builtin::Log10,
        Builtin::Log2 => Builtin::Log2,
        Builtin::Exp => Builtin::Exp,
        Builtin::Exp10 => Builtin::Exp10,
        Builtin::Exp2 => Builtin::Exp2,
        Builtin::Pow(x, y) => Builtin::Pow(
            Box::new(substitute_func_param(x, param, arg)),
            Box::new(substitute_func_param(y, param, arg)),
        ),
        Builtin::Sin => Builtin::Sin,
        Builtin::Cos => Builtin::Cos,
        Builtin::Tan => Builtin::Tan,
        Builtin::Asin => Builtin::Asin,
        Builtin::Acos => Builtin::Acos,
        Builtin::Atan => Builtin::Atan,
        Builtin::Atan2(x, y) => Builtin::Atan2(
            Box::new(substitute_func_param(x, param, arg)),
            Box::new(substitute_func_param(y, param, arg)),
        ),
        Builtin::Sinh => Builtin::Sinh,
        Builtin::Cosh => Builtin::Cosh,
        Builtin::Tanh => Builtin::Tanh,
        Builtin::Asinh => Builtin::Asinh,
        Builtin::Acosh => Builtin::Acosh,
        Builtin::Atanh => Builtin::Atanh,
        Builtin::Infinite => Builtin::Infinite,
        Builtin::Nan => Builtin::Nan,
        Builtin::IsInfinite => Builtin::IsInfinite,
        Builtin::IsNan => Builtin::IsNan,
        Builtin::IsNormal => Builtin::IsNormal,
        Builtin::IsFinite => Builtin::IsFinite,
        Builtin::Debug => Builtin::Debug,
        Builtin::DebugMsg(e) => Builtin::DebugMsg(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Env => Builtin::Env,
        Builtin::EnvVar(e) => Builtin::EnvVar(Box::new(substitute_func_param(e, param, arg))),
        Builtin::NullLit => Builtin::NullLit,
        Builtin::Trim => Builtin::Trim,
        Builtin::Ltrim => Builtin::Ltrim,
        Builtin::Rtrim => Builtin::Rtrim,
        Builtin::Transpose => Builtin::Transpose,
        Builtin::BSearch(e) => Builtin::BSearch(Box::new(substitute_func_param(e, param, arg))),
        Builtin::ModuleMeta(e) => {
            Builtin::ModuleMeta(Box::new(substitute_func_param(e, param, arg)))
        }
    }
}

/// Evaluate a function call to an undefined function.
/// This is an error case - the function was not defined.
fn eval_func_call<'a, W: Clone + AsRef<[u64]>>(
    name: &str,
    _args: &[Expr],
    _value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    QueryResult::Error(EvalError::new(format!("undefined function: {}", name)))
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
        // Identity returns OneCursor for efficient passthrough of unchanged containers
        query!(br#"{"foo": 1}"#, ".", QueryResult::OneCursor(_) => {});
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

    // =========================================================================
    // Phase 6: String Interpolation & Format Strings
    // =========================================================================

    #[test]
    fn test_string_interpolation() {
        // Simple interpolation
        query!(br#"{"name": "Alice"}"#, r#""Hello \(.name)""#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "Hello Alice");
            }
        );

        // Multiple interpolations
        query!(br#"{"first": "John", "last": "Doe"}"#, r#""\(.first) \(.last)""#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "John Doe");
            }
        );

        // Interpolation with number
        query!(br#"{"count": 42}"#, r#""Count: \(.count)""#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "Count: 42");
            }
        );
    }

    #[test]
    fn test_format_json() {
        query!(br#"{"a": 1}"#, "@json",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#"{"a":1}"#);
            }
        );
    }

    #[test]
    fn test_format_text() {
        query!(br#""hello""#, "@text",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello");
            }
        );

        query!(br#"42"#, "@text",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "42");
            }
        );
    }

    #[test]
    fn test_format_uri() {
        query!(br#""hello world""#, "@uri",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello%20world");
            }
        );
    }

    #[test]
    fn test_format_csv() {
        query!(br#"["a", "b", "c"]"#, "@csv",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "a,b,c");
            }
        );

        // CSV with quotes
        query!(br#"["hello, world", "test"]"#, "@csv",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "\"hello, world\",test");
            }
        );
    }

    #[test]
    fn test_format_tsv() {
        query!(br#"["a", "b", "c"]"#, "@tsv",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "a\tb\tc");
            }
        );
    }

    #[test]
    fn test_format_base64() {
        query!(br#""hello""#, "@base64",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "aGVsbG8=");
            }
        );
    }

    #[test]
    fn test_format_base64d() {
        query!(br#""aGVsbG8=""#, "@base64d",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello");
            }
        );
    }

    #[test]
    fn test_format_html() {
        query!(br#""<script>alert('xss')</script>""#, "@html",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;");
            }
        );
    }

    #[test]
    fn test_format_sh() {
        query!(br#""hello world""#, "@sh",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "'hello world'");
            }
        );

        // Shell quoting with embedded single quote
        query!(br#""it's a test""#, "@sh",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "'it'\\''s a test'");
            }
        );
    }

    // =========================================================================
    // Phase 6: Type Conversion Builtins
    // =========================================================================

    #[test]
    fn test_builtin_tostring() {
        query!(br#"42"#, "tostring",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "42");
            }
        );

        query!(br#"true"#, "tostring",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "true");
            }
        );

        query!(br#"null"#, "tostring",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "null");
            }
        );
    }

    #[test]
    fn test_builtin_tonumber() {
        query!(br#""42""#, "tonumber",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 42);
            }
        );

        query!(br#""2.75""#, "tonumber",
            QueryResult::Owned(OwnedValue::Float(f)) => {
                assert!((f - 2.75).abs() < 0.001);
            }
        );

        // Already a number
        query!(br#"42"#, "tonumber",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 42);
            }
        );
    }

    // =========================================================================
    // Phase 6: Additional String Builtins
    // =========================================================================

    #[test]
    fn test_builtin_explode() {
        query!(br#""abc""#, "explode",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(97));  // 'a'
                assert_eq!(arr[1], OwnedValue::Int(98));  // 'b'
                assert_eq!(arr[2], OwnedValue::Int(99));  // 'c'
            }
        );
    }

    #[test]
    fn test_builtin_implode() {
        query!(br#"[97, 98, 99]"#, "implode",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "abc");
            }
        );
    }

    #[test]
    fn test_builtin_test() {
        query!(br#""hello world""#, r#"test("world")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        query!(br#""hello world""#, r#"test("xyz")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
    }

    #[test]
    fn test_builtin_indices() {
        query!(br#""abcabc""#, r#"indices("bc")"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(4));
            }
        );
    }

    #[test]
    fn test_builtin_index() {
        query!(br#""hello world""#, r#"index("world")"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 6);
            }
        );

        query!(br#""hello world""#, r#"index("xyz")"#,
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_builtin_rindex() {
        query!(br#""abcabc""#, r#"rindex("bc")"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 4);
            }
        );
    }

    #[test]
    fn test_builtin_getpath() {
        query!(br#"{"a": {"b": 42}}"#, r#"getpath(["a", "b"])"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 42);
            }
        );

        query!(br#"[1, 2, 3]"#, r#"getpath([1])"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 2);
            }
        );
    }

    // =========================================================================
    // Phase 7: Regex Functions (requires "regex" feature)
    // =========================================================================

    #[cfg(feature = "regex")]
    #[test]
    fn test_regex_scan() {
        query!(br#""test abc test""#, r#"scan("test")"#,
            QueryResult::ManyOwned(matches) => {
                assert_eq!(matches.len(), 2);
                assert_eq!(matches[0], OwnedValue::String("test".to_string()));
                assert_eq!(matches[1], OwnedValue::String("test".to_string()));
            }
        );
    }

    #[cfg(feature = "regex")]
    #[test]
    fn test_regex_splits() {
        query!(br#""a1b2c3d""#, r#"splits("[0-9]")"#,
            QueryResult::Owned(OwnedValue::Array(parts)) => {
                assert_eq!(parts.len(), 4);
                assert_eq!(parts[0], OwnedValue::String("a".to_string()));
                assert_eq!(parts[1], OwnedValue::String("b".to_string()));
                assert_eq!(parts[2], OwnedValue::String("c".to_string()));
                assert_eq!(parts[3], OwnedValue::String("d".to_string()));
            }
        );
    }

    #[cfg(feature = "regex")]
    #[test]
    fn test_regex_sub() {
        query!(br#""hello world world""#, r#"sub("world"; "there")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello there world");
            }
        );
    }

    #[cfg(feature = "regex")]
    #[test]
    fn test_regex_gsub() {
        query!(br#""hello world world""#, r#"gsub("world"; "there")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello there there");
            }
        );

        // Replace all digits with X
        query!(br#""a1b2c3""#, r#"gsub("[0-9]"; "X")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "aXbXcX");
            }
        );
    }

    #[cfg(feature = "regex")]
    #[test]
    fn test_regex_match() {
        query!(br#""test123test""#, r#"match("[0-9]+")"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("string"), Some(&OwnedValue::String("123".to_string())));
                assert_eq!(obj.get("offset"), Some(&OwnedValue::Int(4)));
                assert_eq!(obj.get("length"), Some(&OwnedValue::Int(3)));
            }
        );

        // No match returns null
        query!(br#""hello""#, r#"match("[0-9]+")"#,
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[cfg(feature = "regex")]
    #[test]
    fn test_regex_capture() {
        query!(br#""foo bar""#, r#"capture("(?P<first>\\w+) (?P<second>\\w+)")"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("first"), Some(&OwnedValue::String("foo".to_string())));
                assert_eq!(obj.get("second"), Some(&OwnedValue::String("bar".to_string())));
            }
        );
    }

    // =========================================================================
    // Phase 8 Tests: Variables and Advanced Control Flow
    // =========================================================================

    #[test]
    fn test_variable_binding_as() {
        // Simple variable binding: .foo as $x | .bar + $x
        query!(br#"{"foo": 10, "bar": 5}"#, r#".foo as $x | .bar + $x"#,
            QueryResult::Owned(OwnedValue::Int(15)) => {}
        );

        // Variable with object construction
        query!(br#"{"name": "Alice", "age": 30}"#, r#".name as $n | {name: $n, greeting: "Hello"}"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("name"), Some(&OwnedValue::String("Alice".to_string())));
                assert_eq!(obj.get("greeting"), Some(&OwnedValue::String("Hello".to_string())));
            }
        );
    }

    #[test]
    fn test_reduce() {
        // Sum array elements
        query!(br#"[1, 2, 3, 4, 5]"#, r#"reduce .[] as $x (0; . + $x)"#,
            QueryResult::Owned(OwnedValue::Int(15)) => {}
        );

        // Count elements
        query!(br#"["a", "b", "c"]"#, r#"reduce .[] as $x (0; . + 1)"#,
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );
    }

    #[test]
    fn test_foreach() {
        // Running sum
        query!(br#"[1, 2, 3]"#, r#"[foreach .[] as $x (0; . + $x)]"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(3));
                assert_eq!(arr[2], OwnedValue::Int(6));
            }
        );
    }

    #[test]
    fn test_range() {
        // range(n) - generates 0 to n-1
        query!(br#"null"#, r#"[range(5)]"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(0),
                    OwnedValue::Int(1),
                    OwnedValue::Int(2),
                    OwnedValue::Int(3),
                    OwnedValue::Int(4),
                ]);
            }
        );

        // range(a;b) - generates a to b-1
        query!(br#"null"#, r#"[range(2;5)]"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(2),
                    OwnedValue::Int(3),
                    OwnedValue::Int(4),
                ]);
            }
        );

        // range(a;b;step)
        query!(br#"null"#, r#"[range(0;10;2)]"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(0),
                    OwnedValue::Int(2),
                    OwnedValue::Int(4),
                    OwnedValue::Int(6),
                    OwnedValue::Int(8),
                ]);
            }
        );
    }

    #[test]
    fn test_limit() {
        // limit(n; expr) - take first n outputs
        query!(br#"null"#, r#"[limit(3; range(10))]"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(0),
                    OwnedValue::Int(1),
                    OwnedValue::Int(2),
                ]);
            }
        );
    }

    #[test]
    fn test_first_last_expr() {
        // first(expr) - returns a reference to first element
        query!(br#"[1, 2, 3]"#, r#"first(.[])"#,
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );

        // last(expr) - returns a reference to last element
        query!(br#"[1, 2, 3]"#, r#"last(.[])"#,
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 3);
            }
        );
    }

    #[test]
    fn test_until() {
        // until(cond; update) - iterate until condition is true
        query!(br#"1"#, r#"until(. >= 10; . * 2)"#,
            QueryResult::Owned(OwnedValue::Int(16)) => {}
        );
    }

    #[test]
    fn test_while() {
        // while(cond; update) - output while condition is true
        query!(br#"1"#, r#"[while(. < 10; . * 2)]"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(1),
                    OwnedValue::Int(2),
                    OwnedValue::Int(4),
                    OwnedValue::Int(8),
                ]);
            }
        );
    }

    #[test]
    fn test_recurse() {
        // Basic recurse with filter - collect all values recursively
        query!(br#"{"a": 1, "b": {"c": 2}}"#, r#"[recurse | .a? // .c? // empty]"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // Should contain the values 1 and 2
                assert!(arr.len() >= 2);
            }
        );
    }

    #[test]
    fn test_isvalid() {
        // isvalid returns true for valid expressions
        query!(br#"{"a": 1}"#, r#"isvalid(.a)"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        // isvalid returns false for error-producing expressions
        query!(br#"{"a": 1}"#, r#"isvalid(.b)"#,
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    // =========================================================================
    // Phase 9 Tests: Destructuring and Function Definitions
    // =========================================================================

    #[test]
    fn test_destructuring_object_pattern() {
        // Object destructuring: . as {name: $n, age: $a} | ...
        query!(br#"{"name": "Alice", "age": 30}"#, r#". as {name: $n, age: $a} | "\($n) is \($a)""#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "Alice is 30");
            }
        );
    }

    #[test]
    fn test_destructuring_array_pattern() {
        // Array destructuring: . as [$first, $second] | ...
        query!(br#"[1, 2, 3]"#, r#". as [$a, $b] | $a + $b"#,
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );
    }

    #[test]
    fn test_destructuring_nested_pattern() {
        // Nested destructuring
        query!(br#"{"user": {"name": "Bob", "id": 42}}"#, r#". as {user: {name: $n}} | $n"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "Bob");
            }
        );
    }

    #[test]
    fn test_function_def_simple() {
        // Simple function definition without parameters
        query!(br#"5"#, r#"def double: . * 2; double"#,
            QueryResult::Owned(OwnedValue::Int(10)) => {}
        );
    }

    #[test]
    fn test_function_def_with_params() {
        // Function with parameters (using 'addtwo' to avoid conflict with builtin 'add')
        query!(br#"null"#, r#"def addtwo(a; b): a + b; addtwo(3; 4)"#,
            QueryResult::Owned(OwnedValue::Int(7)) => {}
        );
    }

    #[test]
    fn test_function_def_chained() {
        // Single def, then use in pipe
        query!(br#"5"#, r#"def inc: . + 1; . | inc"#,
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );

        // Two defs, use only second - this exercises nested func def
        query!(br#"5"#, r#"def double: . * 2; def inc: . + 1; inc"#,
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );

        // Two defs, use only first
        query!(br#"5"#, r#"def double: . * 2; def inc: . + 1; double"#,
            QueryResult::Owned(OwnedValue::Int(10)) => {}
        );

        // This should work: use both in a pipe, but inc is defined first
        // (so inc isn't nested inside double's scope)
        query!(br#"5"#, r#"def inc: . + 1; def double: . * 2; double | inc"#,
            QueryResult::Owned(OwnedValue::Int(11)) => {}
        );
    }

    #[test]
    fn test_function_uses_input() {
        // Function that uses the input value
        query!(br#"{"x": 10}"#, r#"def getx: .x; getx"#,
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 10);
            }
        );
    }

    #[test]
    fn test_function_with_filter_param() {
        // Function with filter parameter (jq-style)
        query!(br#"[1, 2, 3]"#, r#"def apply(f): map(f); apply(. * 2)"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(2),
                    OwnedValue::Int(4),
                    OwnedValue::Int(6),
                ]);
            }
        );
    }

    // Phase 10 tests

    #[test]
    fn test_floor() {
        query!(b"3.7", "floor", QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 3);
        });
        query!(b"-3.2", "floor", QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, -4);
        });
    }

    #[test]
    fn test_ceil() {
        query!(b"3.2", "ceil", QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 4);
        });
        query!(b"-3.7", "ceil", QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, -3);
        });
    }

    #[test]
    fn test_round() {
        query!(b"3.4", "round", QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 3);
        });
        query!(b"3.5", "round", QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 4);
        });
    }

    #[test]
    fn test_sqrt() {
        query!(b"9", "sqrt", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 3.0).abs() < f64::EPSILON);
        });
        query!(b"2", "sqrt", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - core::f64::consts::SQRT_2).abs() < 1e-10);
        });
    }

    #[test]
    fn test_fabs() {
        query!(b"-5", "fabs", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 5.0).abs() < f64::EPSILON);
        });
        query!(b"5", "fabs", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 5.0).abs() < f64::EPSILON);
        });
    }

    #[test]
    fn test_log() {
        query!(b"2.718281828459045", "log", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 1.0).abs() < 1e-10);
        });
    }

    #[test]
    fn test_exp() {
        query!(b"1", "exp", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - core::f64::consts::E).abs() < 1e-10);
        });
    }

    #[test]
    fn test_pow() {
        query!(b"2", "pow(.; 3)", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 8.0).abs() < f64::EPSILON);
        });
    }

    #[test]
    fn test_sin_cos_tan() {
        query!(b"0", "sin", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < f64::EPSILON);
        });
        query!(b"0", "cos", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 1.0).abs() < f64::EPSILON);
        });
        query!(b"0", "tan", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < f64::EPSILON);
        });
    }

    #[test]
    fn test_infinite_nan() {
        query!(b"null", "infinite", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.is_infinite() && n > 0.0);
        });
        query!(b"null", "nan", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.is_nan());
        });
    }

    #[test]
    fn test_isinfinite_isnan_isnormal() {
        query!(b"1.0", "isinfinite", QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b);
        });
        query!(b"1.0", "isnan", QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(!b);
        });
        query!(b"1.0", "isnormal", QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b);
        });
        query!(b"1.0", "isfinite", QueryResult::Owned(OwnedValue::Bool(b)) => {
            assert!(b);
        });
    }

    #[test]
    fn test_trim() {
        query!(br#""  hello world  ""#, "trim", QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "hello world");
        });
    }

    #[test]
    fn test_ltrim() {
        query!(br#""  hello""#, "ltrim", QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "hello");
        });
    }

    #[test]
    fn test_rtrim() {
        query!(br#""hello  ""#, "rtrim", QueryResult::Owned(OwnedValue::String(s)) => {
            assert_eq!(s, "hello");
        });
    }

    #[test]
    fn test_transpose() {
        query!(br#"[[1, 2], [3, 4], [5, 6]]"#, "transpose",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                match &arr[0] {
                    OwnedValue::Array(inner) => {
                        assert_eq!(inner.len(), 3);
                        assert_eq!(inner[0], OwnedValue::Int(1));
                        assert_eq!(inner[1], OwnedValue::Int(3));
                        assert_eq!(inner[2], OwnedValue::Int(5));
                    }
                    _ => panic!("expected array"),
                }
            }
        );
    }

    #[test]
    fn test_bsearch() {
        // Found case - returns index
        query!(br#"[1, 2, 3, 4, 5]"#, "bsearch(3)",
            QueryResult::Owned(OwnedValue::Int(idx)) => {
                assert_eq!(idx, 2);
            }
        );
        // Not found case - returns object with index
        query!(br#"[1, 2, 4, 5]"#, "bsearch(3)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("index"), Some(&OwnedValue::Int(2)));
            }
        );
    }

    #[test]
    fn test_paths() {
        query!(br#"{"a": 1, "b": {"c": 2}}"#, "paths",
            QueryResult::Owned(OwnedValue::Array(paths)) => {
                // Should have paths: ["a"], ["b"], ["b", "c"]
                assert_eq!(paths.len(), 3);
            }
        );
    }

    #[test]
    fn test_leaf_paths() {
        query!(br#"{"a": 1, "b": {"c": 2}}"#, "leaf_paths",
            QueryResult::Owned(OwnedValue::Array(paths)) => {
                // Should have paths: ["a"], ["b", "c"]
                assert_eq!(paths.len(), 2);
            }
        );
    }

    #[test]
    fn test_setpath() {
        query!(br#"{"a": 1}"#, r#"setpath(["b"]; 2)"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(2)));
            }
        );
    }

    #[test]
    fn test_delpaths() {
        query!(br#"{"a": 1, "b": 2}"#, r#"delpaths([["b"]])"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
                assert_eq!(obj.get("b"), None);
            }
        );
    }

    #[test]
    fn test_debug() {
        // debug just passes through the value
        query!(b"42", "debug", QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 42);
        });
    }

    #[test]
    fn test_log10_log2() {
        query!(b"100", "log10", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 2.0).abs() < 1e-10);
        });
        query!(b"8", "log2", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 3.0).abs() < 1e-10);
        });
    }

    #[test]
    fn test_exp10_exp2() {
        query!(b"2", "exp10", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 100.0).abs() < 1e-10);
        });
        query!(b"3", "exp2", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 8.0).abs() < 1e-10);
        });
    }

    #[test]
    fn test_asin_acos_atan() {
        query!(b"0", "asin", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
        query!(b"1", "acos", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
        query!(b"0", "atan", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
    }

    #[test]
    fn test_atan2() {
        query!(b"1", "atan2(1; 1)", QueryResult::Owned(OwnedValue::Float(n)) => {
            // atan2(1, 1) = pi/4
            assert!((n - core::f64::consts::FRAC_PI_4).abs() < 1e-10);
        });
    }

    #[test]
    fn test_sinh_cosh_tanh() {
        query!(b"0", "sinh", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
        query!(b"0", "cosh", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 1.0).abs() < 1e-10);
        });
        query!(b"0", "tanh", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
    }

    #[test]
    fn test_asinh_acosh_atanh() {
        query!(b"0", "asinh", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
        query!(b"1", "acosh", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
        query!(b"0", "atanh", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!(n.abs() < 1e-10);
        });
    }

    #[test]
    fn test_env() {
        // env returns empty object in no_std context
        query!(b"null", "env", QueryResult::Owned(OwnedValue::Object(obj)) => {
            assert!(obj.is_empty());
        });
    }

    #[test]
    fn test_null_literal() {
        query!(b"42", "null", QueryResult::Owned(OwnedValue::Null) => {});
    }

    #[test]
    fn test_modulemeta() {
        // modulemeta returns null (stub)
        query!(b"null", r#"modulemeta("test")"#, QueryResult::Owned(OwnedValue::Null) => {});
    }

    #[test]
    fn test_path_expr() {
        // path(expr) is a stub that returns empty array
        // Full implementation would track paths during expression evaluation
        query!(br#"{"a": 1, "b": 2}"#, "path(.a)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty()); // Stub returns empty
            }
        );
    }

    #[test]
    fn test_paths_filter() {
        // paths(filter) returns paths where values match filter
        query!(br#"{"a": 1, "b": "hello", "c": 2}"#, "paths(type == \"number\")",
            QueryResult::Owned(OwnedValue::Array(paths)) => {
                // Should have paths to "a" and "c" (both numbers)
                assert_eq!(paths.len(), 2);
            }
        );
    }

    // Missing test coverage additions

    #[test]
    fn test_walk() {
        // walk applies a function to all values bottom-up
        query!(br#"{"a": 1, "b": [2, 3]}"#, "walk(if type == \"number\" then . + 10 else . end)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(11)));
                match obj.get("b") {
                    Some(OwnedValue::Array(arr)) => {
                        assert_eq!(arr[0], OwnedValue::Int(12));
                        assert_eq!(arr[1], OwnedValue::Int(13));
                    }
                    _ => panic!("expected array for b"),
                }
            }
        );
    }

    #[test]
    fn test_tojsonstream() {
        // tojsonstream converts to path/value pairs
        query!(br#"{"a": 1}"#, "tojsonstream",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // Returns array of [path, value] pairs
                assert!(!arr.is_empty());
            }
        );
    }

    #[test]
    fn test_fromjsonstream() {
        // fromjsonstream currently returns the input unchanged (stub behavior)
        // In full jq, it would reconstruct the object from path/value pairs
        query!(br#"[[["a"], 1]]"#, "fromjsonstream",
            QueryResult::Owned(OwnedValue::Array(_)) => {
                // Stub returns input as-is
            }
        );
    }

    #[test]
    fn test_getpath() {
        // getpath returns Owned Int, not One StandardJson
        query!(br#"{"a": {"b": 42}}"#, r#"getpath(["a", "b"])"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 42);
            }
        );
        // Non-existent path returns null
        query!(br#"{"a": 1}"#, r#"getpath(["missing"])"#,
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_debug_msg() {
        // debug(msg) passes through value unchanged
        query!(b"42", r#"debug("test message")"#, QueryResult::Owned(OwnedValue::Int(n)) => {
            assert_eq!(n, 42);
        });
    }
}
