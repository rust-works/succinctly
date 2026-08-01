//! Expression evaluator for jq-like queries.
//!
//! Evaluates expressions against JSON using the cursor-based navigation API.

#[cfg(not(test))]
use alloc::boxed::Box;
// `BTreeSet`, not `HashSet`: this crate is `no_std`, and `alloc` has no hasher.
use alloc::collections::BTreeSet;
#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

use indexmap::IndexMap;

use super::slice::{self, SliceBounds};

/// Trait for evaluation semantics - determines behavior for edge cases.
///
/// jq and yq (mikefarah/yq) have different behaviors for some operations.
/// This trait is implemented by zero-sized marker types, allowing the compiler
/// to monomorphize and optimize away all branches at compile time.
pub trait EvalSemantics: Copy + Default {
    /// If true, integer overflow wraps (yq). If false, converts to float (jq).
    const OVERFLOW_WRAPS: bool;
    /// If true, division by zero returns infinity (yq). If false, returns error (jq).
    const DIV_BY_ZERO_IS_INFINITY: bool;
    /// If true, has(-1) on arrays checks if abs(idx) <= len (yq). If false, only non-negative (jq).
    const NEGATIVE_INDEX_IN_HAS: bool;
    /// If true, `%` truncates float operands to integers (jq). If false, float modulo (yq).
    const MOD_TRUNCATES_FLOATS: bool;
}

/// jq-compatible evaluation semantics (default).
///
/// - Integer overflow converts to float
/// - Division by zero returns error
/// - has(-1) on arrays returns false
#[derive(Debug, Clone, Copy, Default)]
pub struct JqSemantics;

impl EvalSemantics for JqSemantics {
    const OVERFLOW_WRAPS: bool = false;
    const DIV_BY_ZERO_IS_INFINITY: bool = false;
    const NEGATIVE_INDEX_IN_HAS: bool = false;
    const MOD_TRUNCATES_FLOATS: bool = true;
}

/// yq-compatible evaluation semantics.
///
/// - Integer overflow wraps
/// - Division by zero returns infinity
/// - has(-1) on arrays returns true (if abs(idx) <= len)
#[derive(Debug, Clone, Copy, Default)]
pub struct YqSemantics;

impl EvalSemantics for YqSemantics {
    const OVERFLOW_WRAPS: bool = true;
    const DIV_BY_ZERO_IS_INFINITY: bool = true;
    const NEGATIVE_INDEX_IN_HAS: bool = true;
    const MOD_TRUNCATES_FLOATS: bool = false;
}

use crate::json::light::{JsonCursor, JsonElements, JsonFields, StandardJson};

use super::expr::{
    ArithOp, AssignOp, Builtin, CompareOp, Expr, FormatType, Literal, ObjectEntry, ObjectKey,
    Pattern, StringPart,
};
use super::value::{cmp_f64, format_number_jq_compat, numeric_repr_cmp, NumberRepr, OwnedValue};

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

    /// Break from a labeled scope.
    /// Contains the label name to match against enclosing Label expressions.
    Break(String),
}

impl<W: Clone + AsRef<[u64]>> QueryResult<'_, W> {
    /// Convert OneCursor to One by materializing the cursor value.
    /// Used internally when we need StandardJson from a result.
    #[inline]
    fn materialize_cursor(self) -> Self {
        match self {
            QueryResult::OneCursor(c) => QueryResult::One(c.value()),
            other => other,
        }
    }

    /// Returns true if this result is an evaluation error.
    ///
    /// Mirrors [`crate::jq::eval_generic::GenericResult::is_error`] so the two
    /// evaluators can be compared directly in tests.
    pub fn is_error(&self) -> bool {
        matches!(self, QueryResult::Error(_))
    }

    /// Collect all output values into a `Vec<OwnedValue>`.
    ///
    /// Mirrors [`crate::jq::eval_generic::GenericResult::collect_owned`]: `None`,
    /// `Error`, and `Break` collect to an empty `Vec`. This gives the full
    /// evaluator the same materialization surface as the generic (CLI) path,
    /// which is what evaluator-parity tests rely on.
    pub fn collect_owned(self) -> Vec<OwnedValue> {
        match self {
            QueryResult::One(v) => vec![to_owned(&v)],
            QueryResult::OneCursor(c) => vec![to_owned(&c.value())],
            QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
            QueryResult::None => Vec::new(),
            QueryResult::Error(_) => Vec::new(),
            QueryResult::Owned(o) => vec![o],
            QueryResult::ManyOwned(os) => os,
            QueryResult::Break(_) => Vec::new(),
        }
    }
}

// `EvalError` and the jq-compatible wording of its messages live in
// `super::error` (#356), so both evaluators construct errors from one
// vocabulary instead of inlining format strings at each raise site.
//
// The kind helpers below stay here: they are about values, not messages,
// and `super::error` renders whatever it is handed.
pub use super::error::{BinOp, EvalError};

// jq's `jv_kind` discriminants, verbatim from `jv.h`, so the two enums can be
// read side by side. `JV_KIND_INVALID` (0) has no `OwnedValue` counterpart — an
// `OwnedValue` is always a valid value — so the numbering starts at 1.
const JQ_KIND_NULL: u8 = 1;
const JQ_KIND_FALSE: u8 = 2;
const JQ_KIND_TRUE: u8 = 3;
const JQ_KIND_NUMBER: u8 = 4;
const JQ_KIND_STRING: u8 = 5;
const JQ_KIND_ARRAY: u8 = 6;
const JQ_KIND_OBJECT: u8 = 7;

/// A value's kind as jq's `jv_get_kind` reports it, for the operand screens that
/// compare kinds rather than type names.
///
/// This is a *finer* partition than [`OwnedValue::type_name`]: jq's `jv_kind`
/// enum has separate `JV_KIND_FALSE` and `JV_KIND_TRUE`, and only `jv_kind_name`
/// collapses the pair back to the one word `boolean`. `f_contains` screens on
/// `jv_get_kind`, so jq errors on `true | contains(false)` — with a message that
/// calls *both* operands `boolean` — while `true | contains(true)` is a plain
/// `true`. Screening on `type_name` instead answers `false` for that pair, which
/// is exactly the divergence #358 is about (#358 review).
///
/// This is the one definition of "what kind is this value" in the jq module;
/// [`sort_rank`] is derived from it rather than matched independently. Only
/// equality is ever asked of the value here — orderings go through `sort_rank`.
fn jq_kind(value: &OwnedValue) -> u8 {
    match value {
        OwnedValue::Null => JQ_KIND_NULL,
        OwnedValue::Bool(false) => JQ_KIND_FALSE,
        OwnedValue::Bool(true) => JQ_KIND_TRUE,
        // jq has one number kind; `1 | contains(1.0)` is a comparison, not an error.
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => JQ_KIND_NUMBER,
        OwnedValue::String(_) => JQ_KIND_STRING,
        OwnedValue::Array(_) => JQ_KIND_ARRAY,
        OwnedValue::Object(_) => JQ_KIND_OBJECT,
    }
}

/// A value's rank in jq's sort order:
/// `null < false < true < number < string < array < object`.
///
/// This is [`jq_kind`]'s partition with the two boolean kinds merged into the
/// single `boolean` slot `jv_kind_name` reports — which is what every *ordering*
/// caller wants (`sort`, `min`, `max`, `unique`, `group_by`, the comparison
/// operators, `bsearch`). Only `contains`/`inside` need the finer `jq_kind`.
///
/// Derived rather than matched so the two cannot drift: before #358 there were
/// three hand-written copies of this table in the jq module, and a fourth would
/// have arrived with the containment screen. See the #106 lesson in `CLAUDE.md`
/// — one definition, plus a test that the call sites agree
/// (`sort_rank_agrees_with_jq_kind`).
pub(crate) fn sort_rank(value: &OwnedValue) -> u8 {
    let kind = jq_kind(value);
    // Shift off the unused `JV_KIND_INVALID` slot, then close the gap that
    // merging `false` and `true` into one rank leaves behind.
    kind - 1 - u8::from(kind >= JQ_KIND_TRUE)
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
        StandardJson::Number(n) => match core::str::from_utf8(n.raw_bytes()) {
            Ok(s) => OwnedValue::from_number_literal(s),
            // Fallback - shouldn't happen for valid JSON
            Err(_) => OwnedValue::Float(0.0),
        },
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

/// jq truthiness of a borrowed value: everything except `null` and `false`.
///
/// Equivalent to `to_owned(value).is_truthy()` — [`to_owned`] maps
/// `StandardJson::Error` to `OwnedValue::Null`, which is falsy — but O(1)
/// where `to_owned` deep-copies whole arrays and objects to answer a yes/no
/// question. That matters for the stream operators, which test *every* output
/// rather than just the first.
fn json_is_truthy<W>(value: &StandardJson<'_, W>) -> bool {
    !matches!(
        value,
        StandardJson::Null | StandardJson::Bool(false) | StandardJson::Error(_)
    )
}

/// Check if an expression contains PathNoArg, Parent, or Key builtins that need path context.
fn needs_path_context(expr: &Expr) -> bool {
    match expr {
        Expr::Builtin(Builtin::PathNoArg) => true,
        Expr::Builtin(Builtin::Parent) => true,
        Expr::Builtin(Builtin::ParentN(_)) => true,
        Expr::Builtin(Builtin::Key) => true,
        Expr::Pipe(exprs) => exprs.iter().any(needs_path_context),
        Expr::Paren(inner) => needs_path_context(inner),
        Expr::Optional(inner) => needs_path_context(inner),
        Expr::Comma(exprs) => exprs.iter().any(needs_path_context),
        Expr::IndexExpr { target, key } => needs_path_context(target) || needs_path_context(key),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            needs_path_context(cond)
                || needs_path_context(then_branch)
                || needs_path_context(else_branch)
        }
        Expr::Try { expr, catch } => {
            needs_path_context(expr) || catch.as_ref().is_some_and(|c| needs_path_context(c))
        }
        _ => false,
    }
}

/// Evaluate a single expression against a JSON value.
fn eval_single<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match expr {
        Expr::Identity => QueryResult::One(value),

        Expr::Field(name) => index_object_by_name::<W>(value, name, optional),

        Expr::Index(idx) => index_array_by_position::<W>(value, *idx, optional),

        Expr::IndexExpr { target, key } => eval_index_expr::<W, S>(target, key, value, optional),

        Expr::Slice { start, end } => match value {
            StandardJson::Array(elements) => {
                // Fast path: full slice [:] / [0:] returns the original array unchanged
                if matches!(start, None | Some(0)) && end.is_none() {
                    return QueryResult::One(value);
                }
                // jq array slicing yields a single sub-array, not a stream of elements
                let items: Vec<OwnedValue> = slice_elements::<W>(elements, *start, *end)
                    .iter()
                    .map(to_owned)
                    .collect();
                QueryResult::Owned(OwnedValue::Array(items))
            }
            // jq returns null for slice on null
            StandardJson::Null => QueryResult::One(StandardJson::Null),
            // jq supports string slicing
            StandardJson::String(s) => {
                let s_str = match s.as_str() {
                    Ok(s) => s,
                    Err(_) => return QueryResult::Error(EvalError::new("invalid UTF-8 in string")),
                };

                // Fast path: identity slice [:] returns original string without character counting
                if start.is_none() && end.is_none() {
                    return QueryResult::One(value);
                }

                // Fast path: [0:] on non-empty string returns original
                if let Some(0) = start {
                    if end.is_none() && !s_str.is_empty() {
                        return QueryResult::One(value);
                    }
                }

                // Only count characters when actually slicing. jq indexes a
                // string slice by character, so the length handed to `resolve`
                // is a character count — the bound arithmetic is the same
                // either way.
                let len = s_str.chars().count();
                let range = SliceBounds::from_literals(*start, *end).resolve(len);

                // Fast path: if the resolved slice is the whole string, return
                // the original rather than rebuilding it.
                if range == (0..len) {
                    return QueryResult::One(value);
                }

                QueryResult::Owned(OwnedValue::String(slice::slice_str(&s_str, range)))
            }
            _ if optional => QueryResult::None,
            // jq models `.[a:b]` as indexing with `{"start":a,"end":b}`, so a
            // slice of a non-sliceable value reports an *object* key.
            _ => QueryResult::Error(EvalError::cannot_index_with_type(
                type_name(&value),
                "object",
            )),
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
            _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
        },

        Expr::Optional(inner) => eval_single::<W, S>(inner, value, true),

        Expr::Pipe(exprs) => eval_pipe::<W, S>(exprs, value, optional),

        Expr::Comma(exprs) => eval_comma::<W, S>(exprs, value, optional),

        Expr::Array(inner) => eval_array_construction::<W, S>(inner, value, optional),

        Expr::Object(entries) => eval_object_construction::<W, S>(entries, value, optional),

        Expr::Literal(lit) => QueryResult::Owned(literal_to_owned(lit)),

        Expr::RecursiveDescent => eval_recursive_descent::<W, S>(value),

        Expr::Paren(inner) => eval_single::<W, S>(inner, value, optional),

        Expr::Arithmetic { op, left, right } => {
            eval_arithmetic::<W, S>(*op, left, right, value, optional)
        }

        Expr::Compare { op, left, right } => {
            eval_compare::<W, S>(*op, left, right, value, optional)
        }

        Expr::And(left, right) => eval_and::<W, S>(left, right, value, optional),

        Expr::Or(left, right) => eval_or::<W, S>(left, right, value, optional),

        Expr::Not => eval_not::<W>(value),

        Expr::Alternative(left, right) => eval_alternative::<W, S>(left, right, value, optional),

        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => eval_if::<W, S>(cond, then_branch, else_branch, value, optional),

        Expr::Try { expr, catch } => eval_try::<W, S>(expr, catch.as_deref(), value, optional),

        Expr::Error(msg) => eval_error::<W, S>(msg.as_deref(), value, optional),

        Expr::Builtin(builtin) => eval_builtin::<W, S>(builtin, value, optional),

        Expr::StringInterpolation(parts) => {
            eval_string_interpolation::<W, S>(parts, value, optional)
        }

        Expr::Format(format_type) => eval_format::<W>(format_type.clone(), value, optional),

        // Phase 8: Variables and Advanced Control Flow
        Expr::As { expr, var, body } => eval_as::<W, S>(expr, var, body, value, optional),
        Expr::Var(name) => {
            // Variable references without context should error
            // In practice, variables are resolved by eval_as which substitutes them
            QueryResult::Error(EvalError::new(format!("undefined variable: ${name}")))
        }
        Expr::Loc { line } => {
            // $__loc__ returns {"file": "<stdin>", "line": N}
            // where N is the 1-based line number in the jq filter source
            let mut obj = IndexMap::new();
            obj.insert("file".into(), OwnedValue::String("<stdin>".into()));
            obj.insert("line".into(), OwnedValue::Int(*line as i64));
            QueryResult::Owned(OwnedValue::Object(obj))
        }
        Expr::Env => {
            // $ENV returns an object containing all environment variables
            eval_env::<W>(optional)
        }
        Expr::Reduce {
            input,
            var,
            init,
            update,
        } => eval_reduce::<W, S>(input, var, init, update, value, optional),
        Expr::Foreach {
            input,
            var,
            init,
            update,
            extract,
        } => eval_foreach::<W, S>(
            input,
            var,
            init,
            update,
            extract.as_deref(),
            value,
            optional,
        ),
        Expr::Limit { n, expr } => eval_limit::<W, S>(n, expr, value, optional),
        Expr::FirstExpr(expr) => eval_first_expr::<W, S>(expr, value, optional),
        Expr::LastExpr(expr) => eval_last_expr::<W, S>(expr, value, optional),
        Expr::NthExpr { n, expr } => eval_nth_expr::<W, S>(n, expr, value, optional),
        Expr::Until { cond, update } => eval_until::<W, S>(cond, update, value, optional),
        Expr::While { cond, update } => eval_while::<W, S>(cond, update, value, optional),
        Expr::Repeat(expr) => eval_repeat::<W, S>(expr, value, optional),
        Expr::Range { from, to, step } => {
            eval_range::<W, S>(from, to.as_deref(), step.as_deref(), value, optional)
        }

        // Phase 9: Variables & Definitions
        Expr::AsPattern {
            expr,
            pattern,
            body,
        } => eval_as_pattern::<W, S>(expr, pattern, body, value, optional),
        Expr::FuncDef {
            name,
            params,
            body,
            then,
        } => eval_func_def::<W, S>(name, params, body, then, value, optional),
        Expr::FuncCall { name, args } => eval_func_call::<W>(name, args, value, optional),
        Expr::NamespacedCall {
            namespace,
            name,
            args: _,
        } => {
            // For now, namespaced calls return an error (modules not loaded)
            // This will be properly handled once module loading is implemented
            QueryResult::Error(EvalError::new(format!(
                "module '{namespace}' not loaded (namespaced call {namespace}::{name})"
            )))
        }

        // Assignment operators
        Expr::Assign { path, value: val } => eval_assign::<W, S>(path, val, value, optional),
        Expr::Update { path, filter } => eval_update::<W, S>(path, filter, value, optional),
        Expr::CompoundAssign {
            op,
            path,
            value: val,
        } => eval_compound_assign::<W, S>(*op, path, val, value, optional),
        Expr::AlternativeAssign { path, value: val } => {
            eval_alternative_assign::<W, S>(path, val, value, optional)
        }

        // Label-break for non-local control flow
        Expr::Label { name, body } => eval_label::<W, S>(name, body, value, optional),
        Expr::Break(name) => QueryResult::Break(name.clone()),
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
fn eval_comma<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
        match eval_single::<W, S>(expr, value.clone(), optional).materialize_cursor() {
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
            QueryResult::Break(label) => return QueryResult::Break(label),
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
fn eval_array_construction<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    inner: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Collect all outputs from the inner expression into an array
    let result = eval_single::<W, S>(inner, value, optional);

    let items: Vec<OwnedValue> = match result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => vec![],
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
    };

    QueryResult::Owned(OwnedValue::Array(items))
}

/// jq's refusal of a non-string object key — or nothing at all under the
/// `optional` flag, which is what jq's `?` suffix sets: `0 | {(.):1}?` prints
/// nothing there.
///
/// Succinctly's parser does not yet accept `?` on anything but a path
/// expression, so today the flag arrives only from within; the branch is
/// covered directly by `test_optional_suppresses_the_object_key_refusal`.
fn refuse_object_key<'a, W: Clone + AsRef<[u64]>>(
    key: &OwnedValue,
    optional: bool,
) -> QueryResult<'a, W> {
    if optional {
        QueryResult::None
    } else {
        QueryResult::Error(EvalError::cannot_use_as_object_key(key))
    }
}

/// Evaluate object construction.
fn eval_object_construction<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
                let key_result =
                    eval_single::<W, S>(key_expr, value.clone(), optional).materialize_cursor();
                match key_result {
                    QueryResult::One(StandardJson::String(s)) => match s.as_str() {
                        Ok(cow) => cow.into_owned(),
                        // Not a condition jq has — its strings are always
                        // valid UTF-8 — so this keeps succinctly's wording.
                        Err(_) => {
                            return QueryResult::Error(EvalError::new("key must be a string"))
                        }
                    },
                    QueryResult::Owned(OwnedValue::String(s)) => s,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
                    // A single non-string key is jq's `Cannot use <t> (<v>) as
                    // object key` — the same sentence `from_entries` raises,
                    // because jq *defines* `from_entries` as object
                    // construction over the entries (#391).
                    QueryResult::One(v) => return refuse_object_key(&to_owned(&v), optional),
                    QueryResult::Owned(v) => return refuse_object_key(&v, optional),
                    // A key expression yielding no value, or more than one, is
                    // a separate behavioural gap — jq gives `empty` and a
                    // cartesian product respectively. See
                    // docs/compliance/jq/limitations.md.
                    _ => {
                        return QueryResult::Error(EvalError::new("key must be a string"));
                    }
                }
            }
        };

        // Evaluate the value
        let val_result = eval_single::<W, S>(&entry.value, value.clone(), optional);
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
            QueryResult::Break(label) => return QueryResult::Break(label),
        };

        map.insert(key_str, owned_val);
    }

    QueryResult::Owned(OwnedValue::Object(map))
}

/// Evaluate recursive descent.
fn eval_recursive_descent<W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    value: StandardJson<'_, W>,
) -> QueryResult<'_, W> {
    let mut results = Vec::new();
    collect_recursive::<W, S>(&value, &mut results);
    QueryResult::Many(results)
}

/// Collect all values recursively.
fn collect_recursive<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    value: &StandardJson<'a, W>,
    results: &mut Vec<StandardJson<'a, W>>,
) {
    results.push(value.clone());

    match value {
        StandardJson::Array(elements) => {
            for elem in *elements {
                collect_recursive::<W, S>(&elem, results);
            }
        }
        StandardJson::Object(fields) => {
            for field in *fields {
                collect_recursive::<W, S>(&field.value(), results);
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
        QueryResult::Break(label) => Err(EvalError::new(format!("break ${label} not in label"))),
    }
}

/// Keep only the truthy outputs of a stream, normalizing the result.
///
/// A stream with no surviving output becomes [`QueryResult::None`], and a
/// single survivor is normalized to `One`/`Owned` so the caller cannot tell a
/// filtered stream from a value that was single all along. `None`, `Error` and
/// `Break` pass through untouched — filtering says nothing about them.
///
/// Borrowed values stay borrowed: this never calls [`to_owned`], so `//` over a
/// document-derived stream keeps the zero-copy path.
fn retain_truthy<W: Clone + AsRef<[u64]>>(result: QueryResult<'_, W>) -> QueryResult<'_, W> {
    match result.materialize_cursor() {
        QueryResult::One(v) => {
            if json_is_truthy(&v) {
                QueryResult::One(v)
            } else {
                QueryResult::None
            }
        }
        QueryResult::OneCursor(_) => unreachable!("materialize_cursor removes OneCursor"),
        QueryResult::Owned(v) => {
            if v.is_truthy() {
                QueryResult::Owned(v)
            } else {
                QueryResult::None
            }
        }
        QueryResult::Many(mut vs) => {
            vs.retain(json_is_truthy);
            match vs.len() {
                0 => QueryResult::None,
                1 => QueryResult::One(vs.pop().unwrap()),
                _ => QueryResult::Many(vs),
            }
        }
        QueryResult::ManyOwned(mut vs) => {
            vs.retain(OwnedValue::is_truthy);
            match vs.len() {
                0 => QueryResult::None,
                1 => QueryResult::Owned(vs.pop().unwrap()),
                _ => QueryResult::ManyOwned(vs),
            }
        }
        other @ (QueryResult::None | QueryResult::Error(_) | QueryResult::Break(_)) => other,
    }
}

/// Append one truthiness bit per output of a stream to `out`.
///
/// Returns `Some(control)` when the stream was an `Error` or a `Break`, which
/// the caller must propagate as its own result; `None` when every output was
/// consumed. An empty stream contributes no bits, which is how `empty and true`
/// ends up yielding nothing.
///
/// Collecting `bool` rather than `OwnedValue` is deliberate: `and`/`or` only
/// ever need whether an output was truthy, never the output itself.
fn push_truthiness<'a, W: Clone + AsRef<[u64]>>(
    result: QueryResult<'a, W>,
    out: &mut Vec<bool>,
) -> Option<QueryResult<'a, W>> {
    match result.materialize_cursor() {
        QueryResult::One(v) => out.push(json_is_truthy(&v)),
        QueryResult::OneCursor(_) => unreachable!("materialize_cursor removes OneCursor"),
        QueryResult::Owned(v) => out.push(v.is_truthy()),
        QueryResult::Many(vs) => out.extend(vs.iter().map(json_is_truthy)),
        QueryResult::ManyOwned(vs) => out.extend(vs.iter().map(OwnedValue::is_truthy)),
        QueryResult::None => {}
        QueryResult::Error(e) => return Some(QueryResult::Error(e)),
        QueryResult::Break(label) => return Some(QueryResult::Break(label)),
    }
    None
}

/// Turn a stream of booleans into the result of a boolean operator.
fn bools_to_result<'a, W: Clone + AsRef<[u64]>>(mut bools: Vec<bool>) -> QueryResult<'a, W> {
    match bools.len() {
        0 => QueryResult::None,
        1 => QueryResult::Owned(OwnedValue::Bool(bools.pop().unwrap())),
        _ => QueryResult::ManyOwned(bools.into_iter().map(OwnedValue::Bool).collect()),
    }
}

/// Evaluate arithmetic operations.
fn eval_arithmetic<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    op: ArithOp,
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let left_val = match result_to_owned(eval_single::<W, S>(left, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };
    let right_val = match result_to_owned(eval_single::<W, S>(right, value, optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let result = match op {
        ArithOp::Add => arith_add::<S>(left_val, right_val),
        ArithOp::Sub => arith_sub::<S>(left_val, right_val),
        ArithOp::Mul => arith_mul::<S>(left_val, right_val),
        ArithOp::Div => arith_div::<S>(left_val, right_val),
        ArithOp::Mod => arith_mod::<S>(left_val, right_val),
    };

    match result {
        Ok(v) => QueryResult::Owned(v),
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(e),
    }
}

/// Add two values (numbers, strings, arrays, objects).
fn arith_add<S: EvalSemantics>(
    left: OwnedValue,
    right: OwnedValue,
) -> Result<OwnedValue, EvalError> {
    // A `NumberLiteral` operand degrades to plain `Int`/`Float` the moment it
    // computes with something -- jq gives computed numbers canonical
    // formatting, only values that reach output untouched keep their literal.
    let (left, right) = (left.into_plain_number(), right.into_plain_number());
    match (left, right) {
        // Number addition - jq converts to float on overflow, yq wraps
        (OwnedValue::Int(a), OwnedValue::Int(b)) => {
            if S::OVERFLOW_WRAPS {
                // yq behavior: wrapping add
                Ok(OwnedValue::Int(a.wrapping_add(b)))
            } else {
                // jq behavior: convert to float on overflow
                match a.checked_add(b) {
                    Some(result) => Ok(OwnedValue::Int(result)),
                    None => Ok(OwnedValue::Float(a as f64 + b as f64)),
                }
            }
        }
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
        (a, b) => Err(EvalError::binary_op(&a, &b, BinOp::Add)),
    }
}

/// Subtract two values.
fn arith_sub<S: EvalSemantics>(
    left: OwnedValue,
    right: OwnedValue,
) -> Result<OwnedValue, EvalError> {
    let (left, right) = (left.into_plain_number(), right.into_plain_number());
    match (left, right) {
        // jq converts to float on overflow, yq wraps
        (OwnedValue::Int(a), OwnedValue::Int(b)) => {
            if S::OVERFLOW_WRAPS {
                // yq behavior: wrapping sub
                Ok(OwnedValue::Int(a.wrapping_sub(b)))
            } else {
                // jq behavior: convert to float on overflow
                match a.checked_sub(b) {
                    Some(result) => Ok(OwnedValue::Int(result)),
                    None => Ok(OwnedValue::Float(a as f64 - b as f64)),
                }
            }
        }
        (OwnedValue::Int(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a as f64 - b)),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => Ok(OwnedValue::Float(a - b as f64)),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a - b)),
        // Array subtraction (remove elements)
        (OwnedValue::Array(a), OwnedValue::Array(b)) => {
            let result: Vec<_> = a.into_iter().filter(|x| !b.contains(x)).collect();
            Ok(OwnedValue::Array(result))
        }
        (a, b) => Err(EvalError::binary_op(&a, &b, BinOp::Subtract)),
    }
}

/// Multiply two values.
fn arith_mul<S: EvalSemantics>(
    left: OwnedValue,
    right: OwnedValue,
) -> Result<OwnedValue, EvalError> {
    let (left, right) = (left.into_plain_number(), right.into_plain_number());
    match (left, right) {
        // jq converts to float on overflow, yq wraps
        (OwnedValue::Int(a), OwnedValue::Int(b)) => {
            if S::OVERFLOW_WRAPS {
                // yq behavior: wrapping mul
                Ok(OwnedValue::Int(a.wrapping_mul(b)))
            } else {
                // jq behavior: convert to float on overflow
                match a.checked_mul(b) {
                    Some(result) => Ok(OwnedValue::Int(result)),
                    None => Ok(OwnedValue::Float(a as f64 * b as f64)),
                }
            }
        }
        (OwnedValue::Int(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a as f64 * b)),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => Ok(OwnedValue::Float(a * b as f64)),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => Ok(OwnedValue::Float(a * b)),
        // String repetition: "ab" * 3 = "ababab". jq >= 1.7 yields "" for
        // n == 0 and null only for n < 0 (jqlang/jq#1593)
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
        (a, b) => Err(EvalError::binary_op(&a, &b, BinOp::Multiply)),
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
fn arith_div<S: EvalSemantics>(
    left: OwnedValue,
    right: OwnedValue,
) -> Result<OwnedValue, EvalError> {
    let (left, right) = (left.into_plain_number(), right.into_plain_number());
    match (left, right) {
        (OwnedValue::Int(a), OwnedValue::Int(b)) => {
            if b == 0 {
                if S::DIV_BY_ZERO_IS_INFINITY {
                    // yq behavior: return infinity
                    Ok(OwnedValue::Float(a as f64 / b as f64))
                } else {
                    // jq behavior: error
                    Err(EvalError::divisor_is_zero(
                        &OwnedValue::Int(a),
                        &OwnedValue::Int(b),
                        BinOp::Divide,
                    ))
                }
            } else {
                Ok(OwnedValue::Float(a as f64 / b as f64))
            }
        }
        (OwnedValue::Int(a), OwnedValue::Float(b)) => {
            if b == 0.0 && !S::DIV_BY_ZERO_IS_INFINITY {
                Err(EvalError::divisor_is_zero(
                    &OwnedValue::Int(a),
                    &OwnedValue::Float(b),
                    BinOp::Divide,
                ))
            } else {
                Ok(OwnedValue::Float(a as f64 / b))
            }
        }
        (OwnedValue::Float(a), OwnedValue::Int(b)) => {
            if b == 0 && !S::DIV_BY_ZERO_IS_INFINITY {
                Err(EvalError::divisor_is_zero(
                    &OwnedValue::Float(a),
                    &OwnedValue::Int(b),
                    BinOp::Divide,
                ))
            } else {
                Ok(OwnedValue::Float(a / b as f64))
            }
        }
        (OwnedValue::Float(a), OwnedValue::Float(b)) => {
            if b == 0.0 && !S::DIV_BY_ZERO_IS_INFINITY {
                Err(EvalError::divisor_is_zero(
                    &OwnedValue::Float(a),
                    &OwnedValue::Float(b),
                    BinOp::Divide,
                ))
            } else {
                Ok(OwnedValue::Float(a / b))
            }
        }
        // String split: "a,b,c" / "," = ["a", "b", "c"]
        (OwnedValue::String(s), OwnedValue::String(sep)) => {
            let parts: Vec<OwnedValue> = s
                .split(&sep)
                .map(|p| OwnedValue::String(p.to_string()))
                .collect();
            Ok(OwnedValue::Array(parts))
        }
        (a, b) => Err(EvalError::binary_op(&a, &b, BinOp::Divide)),
    }
}

/// Modulo two values.
fn arith_mod<S: EvalSemantics>(
    left: OwnedValue,
    right: OwnedValue,
) -> Result<OwnedValue, EvalError> {
    let (left, right) = (left.into_plain_number(), right.into_plain_number());
    match (left, right) {
        (OwnedValue::Int(a), OwnedValue::Int(b)) => {
            if b == 0 {
                if S::DIV_BY_ZERO_IS_INFINITY {
                    // yq behavior: return NaN (will be serialized as null)
                    Ok(OwnedValue::Float(f64::NAN))
                } else {
                    // jq behavior: error
                    Err(EvalError::divisor_is_zero(
                        &OwnedValue::Int(a),
                        &OwnedValue::Int(b),
                        BinOp::Modulo,
                    ))
                }
            } else {
                Ok(OwnedValue::Int(a.wrapping_rem(b)))
            }
        }
        (OwnedValue::Float(a), OwnedValue::Float(b)) => {
            mod_floats::<S>(a, b, &OwnedValue::Float(a), &OwnedValue::Float(b))
        }
        (OwnedValue::Int(a), OwnedValue::Float(b)) => {
            mod_floats::<S>(a as f64, b, &OwnedValue::Int(a), &OwnedValue::Float(b))
        }
        (OwnedValue::Float(a), OwnedValue::Int(b)) => {
            mod_floats::<S>(a, b as f64, &OwnedValue::Float(a), &OwnedValue::Int(b))
        }
        (a, b) => Err(EvalError::binary_op(&a, &b, BinOp::Modulo)),
    }
}

/// Modulo where at least one operand is a float.
///
/// jq truncates both operands to integers (`intmax_t`), so `10.5 % 3 == 1` and
/// `5 % 0.5` errors because the divisor truncates to zero. yq performs float
/// modulo, returning NaN for a zero divisor.
fn mod_floats<S: EvalSemantics>(
    a: f64,
    b: f64,
    left: &OwnedValue,
    right: &OwnedValue,
) -> Result<OwnedValue, EvalError> {
    if S::MOD_TRUNCATES_FLOATS {
        if a.is_nan() || b.is_nan() {
            return Ok(OwnedValue::Float(f64::NAN));
        }
        // `as` truncates toward zero and saturates, matching jq's intmax_t cast.
        let (ai, bi) = (a as i64, b as i64);
        if bi == 0 {
            Err(EvalError::divisor_is_zero(left, right, BinOp::Modulo))
        } else {
            // wrapping_rem: i64::MIN % -1 must not panic.
            Ok(OwnedValue::Int(ai.wrapping_rem(bi)))
        }
    } else if b == 0.0 && !S::DIV_BY_ZERO_IS_INFINITY {
        Err(EvalError::divisor_is_zero(left, right, BinOp::Modulo))
    } else {
        Ok(OwnedValue::Float(a % b))
    }
}

/// Evaluate comparison operations.
fn eval_compare<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    op: CompareOp,
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let left_val = match result_to_owned(eval_single::<W, S>(left, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };
    let right_val = match result_to_owned(eval_single::<W, S>(right, value, optional)) {
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
///
/// The one definition of this ordering in the jq module -- shared with the
/// generic (CLI) evaluator, which imports this rather than keeping its own
/// copy (#421, following the #358/#384 precedent: one definition, plus a test
/// that call sites agree).
///
/// NaN orders as strictly less than every other number, including another
/// NaN -- see [`cmp_f64`](super::value::cmp_f64)'s doc comment for the full
/// truth table. That makes this comparator *not* a strict weak ordering
/// whenever two NaNs are compared against each other, which
/// `sort`/`sort_by`/`unique`/`unique_by`/`group_by` feed straight into
/// `[T]::sort_by`. In practice the risk is narrow: `[T]::sort_by` only
/// reaches the internal check that can panic on such a comparator
/// (`core::slice::sort::shared::smallsort`'s bidirectional-merge consistency
/// check) for slices longer than 20 elements --
/// `MAX_LEN_ALWAYS_INSERTION_SORT` in the current standard library, an
/// implementation detail, not a stable contract. Anything at or below that
/// length uses plain insertion sort, which cannot panic regardless of
/// comparator validity, and stress-testing (1,510 randomized/adversarial
/// trials up to length 5000 and 90% NaN density) found no panics at any size.
/// `min`/`max`/`min_by`/`max_by` never sort (`Iterator::min_by`/`max_by` fold
/// left-to-right) and `unique`'s/`unique_by`'s `dedup_by` only compares
/// adjacent elements, so neither has any panic surface at all. See
/// `test_sort_many_nans_does_not_panic_421`.
pub(crate) fn compare_values(left: &OwnedValue, right: &OwnedValue) -> core::cmp::Ordering {
    use core::cmp::Ordering;

    let left_type = sort_rank(left);
    let right_type = sort_rank(right);

    if left_type != right_type {
        return left_type.cmp(&right_type);
    }

    match (left, right) {
        (OwnedValue::Null, OwnedValue::Null) => Ordering::Equal,
        (OwnedValue::Bool(a), OwnedValue::Bool(b)) => a.cmp(b),
        (OwnedValue::Int(a), OwnedValue::Int(b)) => a.cmp(b),
        (OwnedValue::Float(a), OwnedValue::Float(b)) => cmp_f64(*a, *b),
        (OwnedValue::Int(a), OwnedValue::Float(b)) => cmp_f64(*a as f64, *b),
        (OwnedValue::Float(a), OwnedValue::Int(b)) => cmp_f64(*a, *b as f64),
        // A `NumberLiteral` operand compares by its parsed value, exactly
        // like `Int`/`Float` -- ordering never looks at the source text.
        // `numeric_repr_cmp` dispatches on the same `(Int,Int)`/`(Float,Float)`/
        // mixed pairing `==` uses (`numeric_repr_eq`), so ordering can't
        // disagree with equality about the same pair (see its doc comment).
        (OwnedValue::NumberLiteral(..), _) | (_, OwnedValue::NumberLiteral(..)) => {
            match (left.number_repr(), right.number_repr()) {
                (Some(a), Some(b)) => numeric_repr_cmp(a, b),
                _ => Ordering::Equal,
            }
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
            // jq compares the sorted key arrays first, then values in
            // sorted-key order.
            let mut a_keys: Vec<&String> = a.keys().collect();
            let mut b_keys: Vec<&String> = b.keys().collect();
            a_keys.sort();
            b_keys.sort();
            match a_keys.cmp(&b_keys) {
                Ordering::Equal => {}
                other => return other,
            }
            for k in a_keys {
                match compare_values(&a[k], &b[k]) {
                    Ordering::Equal => continue,
                    other => return other,
                }
            }
            Ordering::Equal
        }
        _ => Ordering::Equal,
    }
}

/// Evaluate boolean AND (short-circuiting).
fn eval_and<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // `false and _` is false without consulting the right operand.
    eval_boolean::<W, S>(left, right, value, optional, false)
}

/// Evaluate boolean OR (short-circuiting).
fn eval_or<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // `true or _` is true without consulting the right operand.
    eval_boolean::<W, S>(left, right, value, optional, true)
}

/// Shared body of `and` and `or`, which differ only in which truth value
/// short-circuits: `false` for `and`, `true` for `or`.
///
/// Both are generators, not scalar operators (#160). jq emits one boolean per
/// (left output, right output) pair, with the left operand as the outer loop:
/// `(true,false) and (true,false)` yields `true false false` because the second
/// left output short-circuits. A left output that short-circuits contributes
/// its boolean without evaluating the right operand at all, which is what makes
/// `false and error("x")` yield `false` rather than raising.
///
/// The right operand is re-evaluated per left output rather than evaluated once
/// and reused. That matches jq's backtracking, and it costs nothing in the
/// common case where the left operand is single-output.
fn eval_boolean<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
    short_circuit: bool,
) -> QueryResult<'a, W> {
    let mut left_bools = Vec::new();
    if let Some(control) = push_truthiness(
        eval_single::<W, S>(left, value.clone(), optional),
        &mut left_bools,
    ) {
        return control;
    }

    let mut out = Vec::with_capacity(left_bools.len());
    for left_bool in left_bools {
        if left_bool == short_circuit {
            out.push(short_circuit);
            continue;
        }
        // An error or a break in the right operand is the whole result. Outputs
        // already accumulated are lost, because `QueryResult` models both as a
        // property of the stream rather than of an element — the same limit
        // `eval_comma` has.
        if let Some(control) = push_truthiness(
            eval_single::<W, S>(right, value.clone(), optional),
            &mut out,
        ) {
            return control;
        }
    }

    bools_to_result(out)
}

/// Evaluate boolean NOT.
fn eval_not<W: Clone + AsRef<[u64]>>(value: StandardJson<'_, W>) -> QueryResult<'_, W> {
    let owned = to_owned(&value);
    QueryResult::Owned(OwnedValue::Bool(!owned.is_truthy()))
}

/// Evaluate alternative operator (`//`).
///
/// `a // b` emits **every** output of `a` that is neither `null` nor `false`,
/// and only when `a` yields no such output does it evaluate `b` (#160). `b`'s
/// outputs are emitted unfiltered — `false // (null,7)` is `null, 7` — which is
/// what makes the left-associative chain `a // b // c` filter `b`'s stream.
fn eval_alternative<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    left: &Expr,
    right: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    match retain_truthy(eval_single::<W, S>(left, value.clone(), optional)) {
        // A `break` escapes the operator rather than selecting a branch.
        QueryResult::Break(label) => QueryResult::Break(label),
        // No truthy output on the left, so the right side answers.
        QueryResult::None => eval_single::<W, S>(right, value, optional),
        // An error on the left propagates, matching jq 1.7.1: `//` only
        // substitutes for falsy/absent output, not for a raised error. Use
        // `?` on the left (e.g. `.a? // 3`) to suppress the error instead.
        error @ QueryResult::Error(_) => error,
        kept => kept,
    }
}

/// Evaluate if-then-else expression.
fn eval_if<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    cond: &Expr,
    then_branch: &Expr,
    else_branch: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate condition
    let cond_result = eval_single::<W, S>(cond, value.clone(), optional);

    // Check if condition is truthy
    let is_truthy = match &cond_result {
        QueryResult::One(v) => to_owned(v).is_truthy(),
        QueryResult::OneCursor(_) => unreachable!("eval_single never produces OneCursor"),
        QueryResult::Owned(v) => v.is_truthy(),
        QueryResult::Many(vs) => vs.first().is_some_and(|v| to_owned(v).is_truthy()),
        QueryResult::ManyOwned(vs) => vs.first().is_some_and(super::value::OwnedValue::is_truthy),
        QueryResult::None => false,
        QueryResult::Error(e) => return QueryResult::Error(e.clone()),
        QueryResult::Break(label) => return QueryResult::Break(label.clone()),
    };

    if is_truthy {
        eval_single::<W, S>(then_branch, value, optional)
    } else {
        eval_single::<W, S>(else_branch, value, optional)
    }
}

/// Evaluate try-catch expression.
fn eval_try<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    catch: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the expression
    let result = eval_single::<W, S>(expr, value, optional);

    match result {
        // If error, use catch handler or return nothing. jq runs the handler
        // with the *raised value* as its input, not the original input, so
        // `try error("boom") catch .` yields "boom".
        QueryResult::Error(e) => match catch {
            Some(catch_expr) => eval_owned_input::<W, S>(catch_expr, &e.payload(), optional),
            None => QueryResult::None,
        },
        // Non-error results pass through
        other => other,
    }
}

/// Evaluate label expression.
/// `label $name | expr` establishes a scope that can be exited with `break $name`.
fn eval_label<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    name: &str,
    body: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single::<W, S>(body, value, optional);
    match result {
        // If we get a Break with matching label, convert to empty output
        QueryResult::Break(label) if label == name => QueryResult::None,
        // Non-matching breaks propagate up
        other => other,
    }
}

/// Evaluate error expression.
fn eval_error<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    msg: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // The payload is raised verbatim so `catch` can inspect it, and so the CLI
    // can flag a non-string one; `EvalError` renders the message from it. Bare
    // `error` raises the input value, as jq does — `{"x":1} | error` reports
    // `{"x":1}`, not `null`.
    let payload = match msg {
        Some(msg_expr) => {
            let msg_result = eval_single::<W, S>(msg_expr, value, optional);
            match result_to_owned(msg_result) {
                Ok(v) => v,
                Err(_) if optional => return QueryResult::None,
                Err(e) => return QueryResult::Error(e),
            }
        }
        None => to_owned(&value),
    };

    if optional {
        QueryResult::None
    } else {
        QueryResult::Error(EvalError::from_value(payload))
    }
}

/// Evaluate a builtin function.
fn eval_builtin<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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

        // Type filter functions (select by type)
        // These return the input unchanged if the type matches, or nothing otherwise
        Builtin::Values => {
            // values - select non-null values
            if matches!(value, StandardJson::Null) {
                QueryResult::None
            } else {
                QueryResult::One(value)
            }
        }
        Builtin::Nulls => {
            // nulls - select only null values
            if matches!(value, StandardJson::Null) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }
        Builtin::Booleans => {
            // booleans - select only boolean values
            if matches!(value, StandardJson::Bool(_)) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }
        Builtin::Numbers => {
            // numbers - select only number values
            if matches!(value, StandardJson::Number(_)) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }
        Builtin::Strings => {
            // strings - select only string values
            if matches!(value, StandardJson::String(_)) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }
        Builtin::Arrays => {
            // arrays - select only array values
            if matches!(value, StandardJson::Array(_)) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }
        Builtin::Objects => {
            // objects - select only object values
            if matches!(value, StandardJson::Object(_)) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }
        Builtin::Iterables => {
            // iterables - select arrays and objects
            if matches!(value, StandardJson::Array(_) | StandardJson::Object(_)) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }
        Builtin::Scalars => {
            // scalars - select non-iterables (null, bool, number, string)
            if matches!(
                value,
                StandardJson::Null
                    | StandardJson::Bool(_)
                    | StandardJson::Number(_)
                    | StandardJson::String(_)
            ) {
                QueryResult::One(value)
            } else {
                QueryResult::None
            }
        }

        // Length & Keys
        Builtin::Length => builtin_length::<W>(value, optional),
        Builtin::Utf8ByteLength => builtin_utf8bytelength(value, optional),
        Builtin::Keys => builtin_keys::<W>(value, optional, true),
        Builtin::KeysUnsorted => builtin_keys::<W>(value, optional, false),
        Builtin::Has(key_expr) => builtin_has::<W, S>(key_expr, value, optional),
        Builtin::In(obj_expr) => builtin_in::<W, S>(obj_expr, value, optional),

        // Selection & Filtering
        Builtin::Select(cond) => builtin_select::<W, S>(cond, value, optional),
        Builtin::Empty => QueryResult::None,

        // Map & Iteration
        Builtin::Map(f) => builtin_map::<W, S>(f, value, optional),
        Builtin::MapValues(f) => builtin_map_values::<W, S>(f, value, optional),

        // Reduction
        Builtin::Add => builtin_add::<W, S>(value, optional),
        Builtin::Any => builtin_any::<W>(value, optional),
        Builtin::All => builtin_all::<W>(value, optional),
        Builtin::Min => builtin_min::<W>(value, optional),
        Builtin::Max => builtin_max::<W>(value, optional),
        Builtin::MinBy(f) => builtin_min_by::<W, S>(f, value, optional),
        Builtin::MaxBy(f) => builtin_max_by::<W, S>(f, value, optional),

        // Phase 5: String Functions
        Builtin::AsciiDowncase => builtin_ascii_downcase::<W>(value, optional),
        Builtin::AsciiUpcase => builtin_ascii_upcase::<W>(value, optional),
        Builtin::Ltrimstr(s) => builtin_ltrimstr::<W, S>(s, value, optional),
        Builtin::Rtrimstr(s) => builtin_rtrimstr::<W, S>(s, value, optional),
        Builtin::Startswith(s) => builtin_startswith::<W, S>(s, value, optional),
        Builtin::Endswith(s) => builtin_endswith::<W, S>(s, value, optional),
        Builtin::Split(sep) => builtin_split::<W, S>(sep, value, optional),
        Builtin::Join(sep) => builtin_join::<W, S>(sep, value, optional),
        Builtin::Contains(b) => builtin_contains::<W, S>(b, value, optional),
        Builtin::Inside(b) => builtin_inside::<W, S>(b, value, optional),

        // Phase 5: Array Functions
        Builtin::First => builtin_first::<W>(value, optional),
        Builtin::Last => builtin_last::<W>(value, optional),
        Builtin::Nth(n) => builtin_nth::<W, S>(n, value, optional),
        Builtin::Reverse => builtin_reverse::<W>(value, optional),
        Builtin::Flatten => builtin_flatten::<W>(value, optional, 1),
        Builtin::FlattenDepth(depth) => builtin_flatten_depth::<W, S>(depth, value, optional),
        Builtin::GroupBy(f) => builtin_group_by::<W, S>(f, value, optional),
        Builtin::Unique => builtin_unique::<W>(value, optional),
        Builtin::UniqueBy(f) => builtin_unique_by::<W, S>(f, value, optional),
        Builtin::Sort => builtin_sort::<W>(value, optional),
        Builtin::SortBy(f) => builtin_sort_by::<W, S>(f, value, optional),

        // Phase 5: Object Functions
        Builtin::ToEntries => builtin_to_entries::<W>(value, optional),
        Builtin::FromEntries => builtin_from_entries::<W>(value, optional),
        Builtin::WithEntries(f) => builtin_with_entries::<W, S>(f, value, optional),

        // Phase 6: Type Conversions
        Builtin::ToString => builtin_tostring::<W>(value, optional),
        Builtin::ToNumber => builtin_tonumber::<W>(value, optional),
        Builtin::ToJson => builtin_tojson::<W>(value, optional),
        Builtin::FromJson => builtin_fromjson::<W>(value, optional),

        // Phase 6: Additional String Functions
        Builtin::Explode => builtin_explode::<W>(value, optional),
        Builtin::Implode => builtin_implode::<W>(value, optional),
        #[cfg(feature = "regex")]
        Builtin::Test(re) => builtin_test_regex::<W, S>(re, value, optional),
        #[cfg(not(feature = "regex"))]
        Builtin::Test(re) => builtin_test::<W, S>(re, value, optional),
        Builtin::Indices(s) => builtin_indices::<W, S>(s, value, optional),
        Builtin::Index(s) => builtin_index::<W, S>(s, value, optional),
        Builtin::Rindex(s) => builtin_rindex::<W, S>(s, value, optional),
        Builtin::ToJsonStream => builtin_tojsonstream::<W>(value, optional),
        Builtin::FromJsonStream => builtin_fromjsonstream::<W>(value, optional),
        Builtin::ToStream => builtin_tostream::<W>(value, optional),
        Builtin::FromStream(f) => builtin_fromstream::<W, S>(f, value, optional),
        Builtin::TruncateStream(f) => builtin_truncate_stream::<W, S>(f, value, optional),
        Builtin::GetPath(path) => builtin_getpath::<W, S>(path, value, optional),

        // Phase 16: Regex Functions
        #[cfg(feature = "regex")]
        Builtin::TestFlags(re, flags) => builtin_test_flags::<W, S>(re, flags, value, optional),
        #[cfg(feature = "regex")]
        Builtin::Match(re) => builtin_match::<W, S>(re, None, value, optional),
        #[cfg(feature = "regex")]
        Builtin::MatchFlags(re, flags) => builtin_match_flags::<W, S>(re, flags, value, optional),
        #[cfg(feature = "regex")]
        Builtin::Capture(re) => builtin_capture::<W, S>(re, value, optional),
        #[cfg(feature = "regex")]
        Builtin::CaptureFlags(re, flags) => {
            builtin_capture_flags::<W, S>(re, flags, value, optional)
        }
        #[cfg(feature = "regex")]
        Builtin::Sub(re, replacement) => builtin_sub::<W, S>(re, replacement, value, optional),
        #[cfg(feature = "regex")]
        Builtin::SubFlags(re, replacement, flags) => {
            builtin_sub_flags::<W, S>(re, replacement, flags, value, optional)
        }
        #[cfg(feature = "regex")]
        Builtin::Gsub(re, replacement) => builtin_gsub::<W, S>(re, replacement, value, optional),
        #[cfg(feature = "regex")]
        Builtin::GsubFlags(re, replacement, flags) => {
            builtin_gsub_flags::<W, S>(re, replacement, flags, value, optional)
        }
        #[cfg(feature = "regex")]
        Builtin::Scan(re) => builtin_scan::<W, S>(re, value, optional),
        #[cfg(feature = "regex")]
        Builtin::ScanFlags(re, flags) => builtin_scan_flags::<W, S>(re, flags, value, optional),
        #[cfg(feature = "regex")]
        Builtin::SplitRegex(re, flags) => builtin_split_regex::<W, S>(re, flags, value, optional),
        #[cfg(feature = "regex")]
        Builtin::Splits(re) => builtin_splits::<W, S>(re, value, optional),
        #[cfg(feature = "regex")]
        Builtin::SplitsFlags(re, flags) => builtin_splits_flags::<W, S>(re, flags, value, optional),
        // Non-regex fallbacks for when regex feature is not enabled
        #[cfg(not(feature = "regex"))]
        Builtin::TestFlags(_, _)
        | Builtin::Match(_)
        | Builtin::MatchFlags(_, _)
        | Builtin::Capture(_)
        | Builtin::CaptureFlags(_, _)
        | Builtin::Sub(_, _)
        | Builtin::SubFlags(_, _, _)
        | Builtin::Gsub(_, _)
        | Builtin::GsubFlags(_, _, _)
        | Builtin::Scan(_)
        | Builtin::ScanFlags(_, _)
        | Builtin::SplitRegex(_, _)
        | Builtin::Splits(_)
        | Builtin::SplitsFlags(_, _) => {
            QueryResult::Error(EvalError::new("regex feature not enabled"))
        }

        // Phase 8: Advanced Control Flow Builtins
        Builtin::Recurse => builtin_recurse::<W, S>(value, optional),
        Builtin::RecurseF(f) => builtin_recurse_f::<W, S>(f, value, optional),
        Builtin::RecurseCond(f, cond) => builtin_recurse_cond::<W, S>(f, cond, value, optional),
        Builtin::Walk(f) => builtin_walk::<W, S>(f, value, optional),
        Builtin::IsValid(expr) => builtin_isvalid::<W, S>(expr, value, optional),

        // Phase 10: Path Expressions
        Builtin::Path(expr) => builtin_path::<W, S>(expr, value, optional),
        Builtin::PathNoArg => {
            // PathNoArg requires path context which is handled in eval_pipe_with_context
            // When called without context, return empty path (root position)
            QueryResult::Owned(OwnedValue::Array(vec![]))
        }
        Builtin::Parent => {
            // Parent requires path context which is handled in eval_pipe_with_context
            // When called without context, return empty object (no parent at root)
            QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
        }
        Builtin::ParentN(n_expr) => {
            // ParentN requires path context which is handled in eval_pipe_with_context
            // When called without context, return empty object
            let _ = n_expr; // Unused here, but evaluated in context version
            QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
        }
        Builtin::Paths => builtin_paths::<W>(value, optional),
        Builtin::PathsFilter(filter) => builtin_paths_filter::<W, S>(filter, value, optional),
        Builtin::LeafPaths => builtin_leaf_paths::<W>(value, optional),
        Builtin::SetPath(path, val) => builtin_setpath::<W, S>(path, val, value, optional),
        Builtin::DelPaths(paths) => builtin_delpaths::<W, S>(paths, value, optional),

        // Phase 10: Math Functions
        Builtin::Floor => builtin_floor::<W>(value, optional),
        Builtin::Ceil => builtin_ceil::<W>(value, optional),
        Builtin::Round => builtin_round::<W>(value, optional),
        Builtin::Sqrt => builtin_sqrt::<W>(value, optional),
        Builtin::Fabs => builtin_fabs::<W>(value, optional),
        Builtin::Log => builtin_log::<W>(value, optional),
        Builtin::Log10 => builtin_log10::<W>(value, optional),
        Builtin::Log2 => builtin_log2::<W>(value, optional),
        Builtin::Exp => builtin_exp::<W>(value, optional),
        Builtin::Exp10 => builtin_exp10::<W>(value, optional),
        Builtin::Exp2 => builtin_exp2::<W>(value, optional),
        Builtin::Pow(base, exp) => builtin_pow::<W, S>(base, exp, value, optional),
        Builtin::Sin => builtin_sin::<W>(value, optional),
        Builtin::Cos => builtin_cos::<W>(value, optional),
        Builtin::Tan => builtin_tan::<W>(value, optional),
        Builtin::Asin => builtin_asin::<W>(value, optional),
        Builtin::Acos => builtin_acos::<W>(value, optional),
        Builtin::Atan => builtin_atan::<W>(value, optional),
        Builtin::Atan2(y, x) => builtin_atan2::<W, S>(y, x, value, optional),
        Builtin::Sinh => builtin_sinh::<W>(value, optional),
        Builtin::Cosh => builtin_cosh::<W>(value, optional),
        Builtin::Tanh => builtin_tanh::<W>(value, optional),
        Builtin::Asinh => builtin_asinh::<W>(value, optional),
        Builtin::Acosh => builtin_acosh::<W>(value, optional),
        Builtin::Atanh => builtin_atanh::<W>(value, optional),

        // Phase 10: Number Classification & Constants
        Builtin::Infinite => QueryResult::Owned(OwnedValue::Float(f64::INFINITY)),
        Builtin::Nan => QueryResult::Owned(OwnedValue::Float(f64::NAN)),
        Builtin::IsInfinite => builtin_isinfinite::<W>(value, optional),
        Builtin::IsNan => builtin_isnan::<W>(value, optional),
        Builtin::IsNormal => builtin_isnormal::<W>(value, optional),
        Builtin::IsFinite => builtin_isfinite::<W>(value, optional),

        // Phase 10: Debug
        Builtin::Debug => builtin_debug::<W>(value, optional),
        Builtin::DebugMsg(msg) => builtin_debug_msg::<W>(msg, value, optional),

        // Phase 10: Environment
        Builtin::Env => builtin_env::<W>(value, optional),
        Builtin::EnvVar(var) => builtin_envvar::<W, S>(var, value, optional),
        Builtin::EnvObject(name) => builtin_env_object::<W>(name, optional),
        Builtin::StrEnv(name) => builtin_strenv::<W>(name, optional),

        // Phase 10: Null handling
        Builtin::NullLit => QueryResult::Owned(OwnedValue::Null),

        // Phase 10: String functions
        Builtin::Trim => builtin_trim::<W>(value, optional),
        Builtin::Ltrim => builtin_ltrim::<W>(value, optional),
        Builtin::Rtrim => builtin_rtrim::<W>(value, optional),

        // Phase 10: Array functions
        Builtin::Transpose => builtin_transpose::<W>(value, optional),
        Builtin::BSearch(x) => builtin_bsearch::<W, S>(x, value, optional),

        // Phase 10: Object functions
        Builtin::ModuleMeta(name) => builtin_modulemeta::<W>(name, value, optional),
        Builtin::Pick(keys) => builtin_pick::<W, S>(keys, value, optional),
        Builtin::Omit(keys) => builtin_omit::<W, S>(keys, value, optional),

        // YAML metadata functions (yq)
        Builtin::Tag => builtin_tag::<W>(value),
        Builtin::Anchor => builtin_anchor::<W>(),
        Builtin::Style => builtin_style::<W>(value),
        Builtin::Kind => builtin_kind::<W>(value),
        Builtin::Line => builtin_line::<W>(),
        Builtin::Column => builtin_column::<W>(),
        Builtin::DocumentIndex => builtin_document_index::<W>(),
        Builtin::Shuffle => builtin_shuffle::<W>(value, optional),
        Builtin::Pivot => builtin_pivot::<W>(value, optional),
        Builtin::SplitDoc => {
            // split_doc is identity - the output formatting (--- separators)
            // is handled by the yq runner, not here
            QueryResult::One(value.clone())
        }
        Builtin::Key => {
            // Key requires path context which is handled in eval_pipe_with_context
            // If we reach here without context, return null (at root level)
            QueryResult::Owned(OwnedValue::Null)
        }

        // Phase 11: Path manipulation
        Builtin::Del(path) => builtin_del::<W, S>(path, value, optional),

        // Phase 12: Additional builtins
        Builtin::Now => builtin_now::<W>(),
        Builtin::Abs => builtin_fabs::<W>(value, optional), // abs is an alias for fabs
        Builtin::Builtins => builtin_builtins::<W>(),
        Builtin::Normals => builtin_normals::<W>(value),
        Builtin::Finites => builtin_finites::<W>(value),

        // Phase 13: Iteration control
        Builtin::Limit(n_expr, expr) => builtin_limit::<W, S>(n_expr, expr, value, optional),
        Builtin::FirstStream(expr) => builtin_first_stream::<W, S>(expr, value, optional),
        Builtin::LastStream(expr) => builtin_last_stream::<W, S>(expr, value, optional),
        Builtin::NthStream(n_expr, expr) => {
            builtin_nth_stream::<W, S>(n_expr, expr, value, optional)
        }
        Builtin::IsEmpty(expr) => builtin_isempty::<W, S>(expr, value, optional),

        // Phase 14: Recursive traversal (extends Phase 8)
        Builtin::RecurseDown => builtin_recurse::<W, S>(value, optional), // alias for recurse

        // Phase 15: Date/Time functions
        Builtin::Gmtime => builtin_gmtime::<W>(value, optional),
        Builtin::Localtime => builtin_localtime::<W>(value, optional),
        Builtin::Mktime => builtin_mktime::<W>(value, optional),
        Builtin::Strftime(fmt) => builtin_strftime::<W, S>(fmt, value, optional),
        Builtin::Strptime(fmt) => builtin_strptime::<W, S>(fmt, value, optional),
        Builtin::Todate => builtin_todate::<W>(value, optional),
        Builtin::Fromdate => builtin_fromdate::<W>(value, optional),
        Builtin::Todateiso8601 => builtin_todate::<W>(value, optional), // alias for todate
        Builtin::Fromdateiso8601 => builtin_fromdate::<W>(value, optional), // alias for fromdate

        // Phase 17: Combinations
        Builtin::Combinations => builtin_combinations::<W>(value, optional),
        Builtin::CombinationsN(n) => builtin_combinations_n::<W, S>(n, value, optional),

        // Phase 18: Additional math functions
        Builtin::Trunc => builtin_trunc::<W>(value, optional),

        // Phase 19: Type conversion
        Builtin::ToBoolean => builtin_toboolean::<W>(value, optional),

        // Phase 20: Iteration control extension
        Builtin::Skip(n_expr, expr) => builtin_skip::<W, S>(n_expr, expr, value, optional),

        // Phase 21: Extended Date/Time functions (yq)
        Builtin::FromUnix => builtin_from_unix::<W>(value, optional),
        Builtin::ToUnix => builtin_to_unix::<W>(value, optional),
        Builtin::Tz(zone) => builtin_tz::<W, S>(zone, value, optional),

        // Phase 22: File operations (yq)
        Builtin::Load(file_expr) => builtin_load::<W, S>(file_expr, value, optional),

        // Phase 23: Position-based navigation (succinctly extension)
        // These require cursor context - handled in eval_generic.rs
        Builtin::AtOffset(_) => QueryResult::Error(EvalError::new(
            "at_offset requires document cursor context".to_string(),
        )),
        Builtin::AtPosition(_, _) => QueryResult::Error(EvalError::new(
            "at_position requires document cursor context".to_string(),
        )),
    }
}

/// Builtin: length
fn builtin_length<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
            // Length of a number is its absolute value.
            // checked_abs: i64::MIN has no i64 absolute value; use f64
            if let Ok(i) = n.as_i64() {
                QueryResult::Owned(match i.checked_abs() {
                    Some(a) => OwnedValue::Int(a),
                    None => OwnedValue::Float(-(i as f64)),
                })
            } else if let Ok(f) = n.as_f64() {
                QueryResult::Owned(OwnedValue::Float(f.abs()))
            } else {
                QueryResult::Owned(OwnedValue::Int(0))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::has_no_length(&to_owned(&value))),
    }
}

/// Builtin: utf8bytelength
fn builtin_utf8bytelength<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                QueryResult::Owned(OwnedValue::Int(cow.len() as i64))
            } else {
                QueryResult::Owned(OwnedValue::Int(0))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::no_utf8_byte_length(&to_owned(&value))),
    }
}

/// Builtin: keys / keys_unsorted
fn builtin_keys<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
    sorted: bool,
) -> QueryResult<'_, W> {
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
        _ => QueryResult::Error(EvalError::has_no_keys(&to_owned(&value))),
    }
}

/// Builtin: has(key)
fn builtin_has<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    key_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the key expression
    let key_result = eval_single::<W, S>(key_expr, value.clone(), optional);
    let key_owned = match result_to_owned(key_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    match (&value, &key_owned) {
        // jq: null | has("key") => false
        (StandardJson::Null, _) => QueryResult::Owned(OwnedValue::Bool(false)),
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
        // Array has index - jq returns false for negative, yq returns true if in range
        (
            StandardJson::Array(elements),
            OwnedValue::Int(idx) | OwnedValue::NumberLiteral(NumberRepr::Int(idx), _),
        ) => {
            let len = (*elements).count() as i64;
            let in_bounds = if S::NEGATIVE_INDEX_IN_HAS {
                // yq behavior: negative indices are valid if abs(idx) <= len
                if *idx >= 0 {
                    *idx < len
                } else {
                    idx.abs() <= len
                }
            } else {
                // jq behavior: only non-negative indices
                *idx >= 0 && *idx < len
            };
            QueryResult::Owned(OwnedValue::Bool(in_bounds))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_check_has(
            type_name(&value),
            key_owned.type_name(),
        )),
    }
}

/// Builtin: in(obj)
fn builtin_in<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    obj_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // The input should be a key (string or number), and we check if it exists in obj
    let key_owned = to_owned(&value);
    let obj_result = eval_single::<W, S>(obj_expr, value.clone(), optional);

    // Get the object/array to check against (need to handle Owned case for object literals)
    let obj_owned = match obj_result {
        QueryResult::One(o) => to_owned(&o),
        QueryResult::Many(os) => {
            if let Some(o) = os.into_iter().next() {
                to_owned(&o)
            } else if optional {
                return QueryResult::None;
            } else {
                return QueryResult::Error(EvalError::new(
                    "in() requires an object or array argument",
                ));
            }
        }
        QueryResult::Owned(o) => o,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ if optional => return QueryResult::None,
        _ => {
            return QueryResult::Error(EvalError::new("in() requires an object or array argument"));
        }
    };

    match (&key_owned, &obj_owned) {
        (OwnedValue::String(key), OwnedValue::Object(fields)) => {
            let found = fields.keys().any(|k| k == key);
            QueryResult::Owned(OwnedValue::Bool(found))
        }
        // jq returns false for negative indices, yq returns true if in range
        (
            OwnedValue::Int(idx) | OwnedValue::NumberLiteral(NumberRepr::Int(idx), _),
            OwnedValue::Array(elements),
        ) => {
            let len = elements.len() as i64;
            let in_bounds = if S::NEGATIVE_INDEX_IN_HAS {
                // yq behavior: negative indices are valid if abs(idx) <= len
                if *idx >= 0 {
                    *idx < len
                } else {
                    idx.abs() <= len
                }
            } else {
                // jq behavior: only non-negative indices
                *idx >= 0 && *idx < len
            };
            QueryResult::Owned(OwnedValue::Bool(in_bounds))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_check_has(
            obj_owned.type_name(),
            key_owned.type_name(),
        )),
    }
}

/// Builtin: select(condition)
fn builtin_select<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    cond: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate condition
    let cond_result = eval_single::<W, S>(cond, value.clone(), optional);

    // Check if condition is truthy
    let is_truthy = match &cond_result {
        QueryResult::One(v) => to_owned(v).is_truthy(),
        QueryResult::OneCursor(_) => unreachable!("eval_single never produces OneCursor"),
        QueryResult::Owned(v) => v.is_truthy(),
        QueryResult::Many(vs) => vs.first().is_some_and(|v| to_owned(v).is_truthy()),
        QueryResult::ManyOwned(vs) => vs.first().is_some_and(super::value::OwnedValue::is_truthy),
        QueryResult::None => false,
        QueryResult::Error(e) => return QueryResult::Error(e.clone()),
        QueryResult::Break(label) => return QueryResult::Break(label.clone()),
    };

    if is_truthy {
        QueryResult::One(value)
    } else {
        QueryResult::None
    }
}

/// Builtin: map(f)
/// Applies `f` to each element and materializes the results into a flat
/// array — the shared body of `map(f)`, over either an array's elements or
/// an object's values.
fn map_over<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    f: &Expr,
    elements: impl Iterator<Item = StandardJson<'a, W>>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut results = Vec::new();
    for elem in elements {
        match eval_single::<W, S>(f, elem, optional).materialize_cursor() {
            QueryResult::One(v) => results.push(to_owned(&v)),
            QueryResult::OneCursor(_) => unreachable!(),
            QueryResult::Owned(v) => results.push(v),
            QueryResult::Many(vs) => results.extend(vs.iter().map(to_owned)),
            QueryResult::ManyOwned(vs) => results.extend(vs),
            QueryResult::None => {}
            QueryResult::Error(e) => return QueryResult::Error(e),
            QueryResult::Break(label) => return QueryResult::Break(label),
        }
    }
    QueryResult::Owned(OwnedValue::Array(results))
}

fn builtin_map<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // map(f) is [.[] | f], and .[] over an object iterates its values, so jq
    // accepts an object here as readily as an array (#422).
    match value {
        StandardJson::Array(elements) => map_over::<W, S>(f, elements, optional),
        StandardJson::Object(fields) => {
            map_over::<W, S>(f, fields.map(|fld| fld.value()), optional)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    }
}

/// Builtin: map_values(f)
fn builtin_map_values<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
                match eval_single::<W, S>(f, field_val, optional).materialize_cursor() {
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
                    QueryResult::Break(label) => return QueryResult::Break(label),
                }
            }
            QueryResult::Owned(OwnedValue::Object(result_map))
        }
        StandardJson::Array(elements) => {
            // map_values on array applies to each element
            let mut results = Vec::new();
            for elem in elements {
                match eval_single::<W, S>(f, elem, optional).materialize_cursor() {
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
                    QueryResult::Break(label) => return QueryResult::Break(label),
                }
            }
            QueryResult::Owned(OwnedValue::Array(results))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("object or array", type_name(&value))),
    }
}

/// Builtin: add
fn builtin_add<W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    // add is [.[] | .] folded with +, and .[] over an object iterates its
    // values, so jq accepts an object here as readily as an array (#422).
    let items: Vec<OwnedValue> = match value {
        StandardJson::Array(elements) => elements.map(|e| to_owned(&e)).collect(),
        StandardJson::Object(fields) => fields.map(|f| to_owned(&f.value())).collect(),
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    };
    if items.is_empty() {
        return QueryResult::Owned(OwnedValue::Null);
    }

    // Fold the items using addition
    let mut acc = items.into_iter();
    let first = acc.next().unwrap();
    let result = acc.try_fold(first, arith_add::<S>);

    match result {
        Ok(v) => QueryResult::Owned(v),
        Err(e) => QueryResult::Error(e),
    }
}

/// Short-circuiting fold shared by `any`/`all`: returns `target_truthy` as
/// soon as an element's truthiness matches it, instead of walking the rest
/// of the container. `any` looks for a truthy element (`target_truthy =
/// true`); `all` looks for a falsy one (`target_truthy = false`).
fn any_all_over<'a, W: Clone + AsRef<[u64]> + 'a>(
    elements: impl Iterator<Item = StandardJson<'a, W>>,
    target_truthy: bool,
) -> bool {
    for elem in elements {
        if to_owned(&elem).is_truthy() == target_truthy {
            return target_truthy;
        }
    }
    !target_truthy
}

/// Builtin: any
///
/// any is [.[] | .] with an early-exit truthiness check, and .[] over an
/// object iterates its values, so jq accepts an object here as readily as
/// an array (#422).
fn builtin_any<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => {
            QueryResult::Owned(OwnedValue::Bool(any_all_over(elements, true)))
        }
        StandardJson::Object(fields) => QueryResult::Owned(OwnedValue::Bool(any_all_over(
            fields.map(|f| f.value()),
            true,
        ))),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    }
}

/// Builtin: all
///
/// Same shape as `any` — see #422.
fn builtin_all<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => {
            QueryResult::Owned(OwnedValue::Bool(any_all_over(elements, false)))
        }
        StandardJson::Object(fields) => QueryResult::Owned(OwnedValue::Bool(any_all_over(
            fields.map(|f| f.value()),
            false,
        ))),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    }
}

/// Builtin: min
fn builtin_min<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
        _ => QueryResult::Error(EvalError::pair_cannot_be_iterated(
            &to_owned(&value),
            &to_owned(&value),
        )),
    }
}

/// Builtin: max
fn builtin_max<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
        _ => QueryResult::Error(EvalError::pair_cannot_be_iterated(
            &to_owned(&value),
            &to_owned(&value),
        )),
    }
}

/// Builtin: min_by(f)
fn builtin_min_by<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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

            // Compute keys for each item. jq keys by `[f]` — the array of
            // *all* outputs of the key filter, not just its first output
            // (#155).
            let mut keyed: Vec<(OwnedValue, StandardJson<'a, W>)> = Vec::new();
            for item in items {
                match eval_array_construction::<W, S>(f, item.clone(), optional) {
                    QueryResult::Owned(v) => keyed.push((v, item)),
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
                    _ => unreachable!("eval_array_construction only returns Owned/Error/Break"),
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
fn builtin_max_by<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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

            // Compute keys for each item. jq keys by `[f]` — the array of
            // *all* outputs of the key filter, not just its first output
            // (#155).
            let mut keyed: Vec<(OwnedValue, StandardJson<'a, W>)> = Vec::new();
            for item in items {
                match eval_array_construction::<W, S>(f, item.clone(), optional) {
                    QueryResult::Owned(v) => keyed.push((v, item)),
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
                    _ => unreachable!("eval_array_construction only returns Owned/Error/Break"),
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
fn builtin_ascii_downcase<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
        _ => QueryResult::Error(EvalError::new("explode input must be a string")),
    }
}

/// Builtin: ascii_upcase - uppercase ASCII characters
fn builtin_ascii_upcase<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
        // As `ascii_downcase`: jq defines both as `explode | map(…) | implode`,
        // so both report `explode`'s refusal.
        _ => QueryResult::Error(EvalError::new("explode input must be a string")),
    }
}

/// Builtin: ltrimstr(s) - remove prefix s
fn builtin_ltrimstr<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    prefix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the prefix string
    let prefix_result = eval_single::<W, S>(prefix_expr, value.clone(), optional);
    let prefix = match result_to_owned(prefix_result) {
        Ok(OwnedValue::String(s)) => s,
        // jq's ltrimstr is total: a non-string argument leaves input unchanged.
        Ok(_) => return QueryResult::Owned(to_owned(&value)),
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
        // jq's ltrimstr is total: a non-string input passes through unchanged.
        _ => QueryResult::Owned(to_owned(&value)),
    }
}

/// Builtin: rtrimstr(s) - remove suffix s
fn builtin_rtrimstr<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    suffix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the suffix string
    let suffix_result = eval_single::<W, S>(suffix_expr, value.clone(), optional);
    let suffix = match result_to_owned(suffix_result) {
        Ok(OwnedValue::String(s)) => s,
        // jq's rtrimstr is total: a non-string argument leaves input unchanged.
        Ok(_) => return QueryResult::Owned(to_owned(&value)),
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
        // jq's rtrimstr is total: a non-string input passes through unchanged.
        _ => QueryResult::Owned(to_owned(&value)),
    }
}

/// Builtin: startswith(s) - check if string starts with s
fn builtin_startswith<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    prefix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the prefix string
    let prefix_result = eval_single::<W, S>(prefix_expr, value.clone(), optional);
    let prefix = match result_to_owned(prefix_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::new("startswith() requires string inputs")),
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
        _ => QueryResult::Error(EvalError::new("startswith() requires string inputs")),
    }
}

/// Builtin: endswith(s) - check if string ends with s
fn builtin_endswith<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    suffix_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the suffix string
    let suffix_result = eval_single::<W, S>(suffix_expr, value.clone(), optional);
    let suffix = match result_to_owned(suffix_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::new("endswith() requires string inputs")),
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
        _ => QueryResult::Error(EvalError::new("endswith() requires string inputs")),
    }
}

/// Builtin: split(s) - split string by separator
fn builtin_split<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    sep_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the separator string
    let sep_result = eval_single::<W, S>(sep_expr, value.clone(), optional);
    let sep = match result_to_owned(sep_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => {
            return QueryResult::Error(EvalError::new("split input and separator must be strings"))
        }
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                // jq: split("") returns each character as a separate element
                // Rust's split("") includes empty strings at boundaries, so special-case it
                let parts: Vec<OwnedValue> = if sep.is_empty() {
                    cow.chars()
                        .map(|c| OwnedValue::String(c.to_string()))
                        .collect()
                } else {
                    cow.split(&sep)
                        .map(|p| OwnedValue::String(p.to_string()))
                        .collect()
                };
                QueryResult::Owned(OwnedValue::Array(parts))
            } else {
                QueryResult::Owned(OwnedValue::Array(vec![]))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new("split input and separator must be strings")),
    }
}

/// Stringifies each element for `join`: strings pass through, nulls are
/// skipped, everything else is rendered as JSON.
fn join_parts<'a, W: Clone + AsRef<[u64]> + 'a>(
    elements: impl Iterator<Item = StandardJson<'a, W>>,
) -> Vec<String> {
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
    parts
}

/// Builtin: join(s) - join array elements with separator
///
/// join(s) is [.[] | tostring] joined by s, and .[] over an object iterates
/// its values, so jq accepts an object here as readily as an array (#422).
fn builtin_join<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    sep_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the separator string
    let sep_result = eval_single::<W, S>(sep_expr, value.clone(), optional);
    let sep = match result_to_owned(sep_result) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "non-string")),
        Err(e) => return QueryResult::Error(e),
    };

    match value {
        StandardJson::Array(elements) => {
            QueryResult::Owned(OwnedValue::String(join_parts(elements).join(&sep)))
        }
        StandardJson::Object(fields) => QueryResult::Owned(OwnedValue::String(
            join_parts(fields.map(|f| f.value())).join(&sep),
        )),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    }
}

/// Builtin: contains(b) - check if input contains b
fn builtin_contains<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    b_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the value to check
    let b_result = eval_single::<W, S>(b_expr, value.clone(), optional);
    let b = match result_to_owned(b_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let input = to_owned(&value);
    if jq_kind(&input) != jq_kind(&b) {
        if optional {
            return QueryResult::None;
        }
        return QueryResult::Error(EvalError::containment_check(&input, &b));
    }
    QueryResult::Owned(OwnedValue::Bool(owned_contains(&input, &b)))
}

/// Check if `a` contains `b` (recursive containment check)
///
/// Only the *top-level* kinds have to match; a mismatch nested inside a
/// container is plain `false`, as in jq — `[1,"a"] | contains(["a",2])` is
/// `false`, not the error [`EvalError::containment_check`] raises. The callers
/// screen the top level with [`jq_kind`], so this stays a total function.
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
fn builtin_inside<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    b_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the container value
    let b_result = eval_single::<W, S>(b_expr, value.clone(), optional);
    let b = match result_to_owned(b_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let input = to_owned(&value);
    // inside is the inverse of contains: b contains input
    if jq_kind(&b) != jq_kind(&input) {
        if optional {
            return QueryResult::None;
        }
        return QueryResult::Error(EvalError::containment_check(&b, &input));
    }
    QueryResult::Owned(OwnedValue::Bool(owned_contains(&b, &input)))
}

// =============================================================================
// Phase 5: Array Functions
// =============================================================================

/// Builtin: first - first element (.[0])
fn builtin_first<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => match elements.get(0) {
            Some(v) => QueryResult::One(v),
            // jq: [] | first => null
            None => QueryResult::Owned(OwnedValue::Null),
        },
        // jq: null | first => null
        StandardJson::Null => QueryResult::Owned(OwnedValue::Null),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_index_with_type(
            type_name(&value),
            "number",
        )),
    }
}

/// Builtin: last - last element (.[-1])
fn builtin_last<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<_> = elements.collect();
            if items.is_empty() {
                // jq: [] | last => null
                QueryResult::Owned(OwnedValue::Null)
            } else {
                QueryResult::Owned(to_owned(&items[items.len() - 1]))
            }
        }
        // jq: null | last => null
        StandardJson::Null => QueryResult::Owned(OwnedValue::Null),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_index_with_type(
            type_name(&value),
            "number",
        )),
    }
}

/// Builtin: nth(n) - nth element
fn builtin_nth<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    n_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // jq: null | nth(0) => null
    if matches!(value, StandardJson::Null) {
        return QueryResult::Owned(OwnedValue::Null);
    }

    // Get the index
    let n_result = eval_single::<W, S>(n_expr, value.clone(), optional);
    let n = match result_to_owned(n_result) {
        Ok(OwnedValue::Int(i) | OwnedValue::NumberLiteral(NumberRepr::Int(i), _)) => i,
        Ok(_) => return QueryResult::Error(EvalError::type_error("number", "non-number")),
        Err(e) => return QueryResult::Error(e),
    };

    match value {
        StandardJson::Array(elements) => match get_element_at_index::<W>(elements, n) {
            Some(v) => QueryResult::One(v),
            // jq: [1,2] | nth(10) => null
            None => QueryResult::Owned(OwnedValue::Null),
        },
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Builtin: reverse - reverse array
fn builtin_reverse<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
        // jq: null | reverse => []
        StandardJson::Null => QueryResult::Owned(OwnedValue::Array(Vec::new())),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_index_with_type(
            type_name(&value),
            "number",
        )),
    }
}

/// Builtin: flatten - flatten nested arrays (1 level)
///
/// flatten is defined over [.[]], and .[] over an object iterates its
/// values, so jq accepts an object here as readily as an array (#422).
fn builtin_flatten<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
    depth: usize,
) -> QueryResult<'_, W> {
    let items: Vec<OwnedValue> = match value {
        StandardJson::Array(elements) => elements.map(|e| to_owned(&e)).collect(),
        StandardJson::Object(fields) => fields.map(|f| to_owned(&f.value())).collect(),
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    };
    let flattened = flatten_owned(items, depth);
    QueryResult::Owned(OwnedValue::Array(flattened))
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
fn builtin_flatten_depth<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    depth_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the depth
    let depth_result = eval_single::<W, S>(depth_expr, value.clone(), optional);
    let depth = match result_to_owned(depth_result) {
        Ok(OwnedValue::Int(d) | OwnedValue::NumberLiteral(NumberRepr::Int(d), _)) if d >= 0 => {
            d as usize
        }
        Ok(OwnedValue::Int(_) | OwnedValue::NumberLiteral(NumberRepr::Int(_), _)) => {
            return QueryResult::Error(EvalError::new("depth must be non-negative"));
        }
        Ok(_) => return QueryResult::Error(EvalError::type_error("number", "non-number")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_flatten::<W>(value, optional, depth)
}

/// Builtin: group_by(f) - group by key function
fn builtin_group_by<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
                // jq keys by `[f]` — the array of *all* outputs of the key
                // filter — not just its first output, so `sort_by(.a,.b)`
                // is a genuine multi-key sort (#155).
                let key = match eval_array_construction::<W, S>(f, item.clone(), optional) {
                    QueryResult::Owned(v) => v,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
                    _ => unreachable!("eval_array_construction only returns Owned/Error/Break"),
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
        _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    }
}

/// Builtin: unique - remove duplicates
fn builtin_unique<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
        _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    }
}

/// Builtin: unique_by(f) - remove duplicates by key
fn builtin_unique_by<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
                // jq keys by `[f]` — the array of *all* outputs of the key
                // filter — not just its first output, so `sort_by(.a,.b)`
                // is a genuine multi-key sort (#155).
                let key = match eval_array_construction::<W, S>(f, item.clone(), optional) {
                    QueryResult::Owned(v) => v,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
                    _ => unreachable!("eval_array_construction only returns Owned/Error/Break"),
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
fn builtin_sort<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => {
            let mut items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            items.sort_by(compare_values);
            QueryResult::Owned(OwnedValue::Array(items))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_be_sorted(&to_owned(&value))),
    }
}

/// Builtin: sort_by(f) - sort by key function
fn builtin_sort_by<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
                // jq keys by `[f]` — the array of *all* outputs of the key
                // filter — not just its first output, so `sort_by(.a,.b)`
                // is a genuine multi-key sort (#155).
                let key = match eval_array_construction::<W, S>(f, item.clone(), optional) {
                    QueryResult::Owned(v) => v,
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
                    _ => unreachable!("eval_array_construction only returns Owned/Error/Break"),
                };
                keyed.push((key, to_owned(&item)));
            }

            // Sort by key
            keyed.sort_by(|(a, _), (b, _)| compare_values(a, b));

            let result: Vec<OwnedValue> = keyed.into_iter().map(|(_, v)| v).collect();
            QueryResult::Owned(OwnedValue::Array(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    }
}

// =============================================================================
// Phase 5: Object Functions
// =============================================================================

/// Builtin: to_entries - {k:v} → [{key:k, value:v}]
///
/// jq defines this over `keys_unsorted`, so it accepts everything that has
/// keys: an array's keys are its indices, and `[1,2] | to_entries` is
/// `[{"key":0,"value":1},{"key":1,"value":2}]`. Anything with no keys at all
/// gets `keys`' own refusal.
fn builtin_to_entries<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => {
            let entries: Vec<OwnedValue> = elements
                .enumerate()
                .map(|(i, elem)| {
                    let mut entry = IndexMap::new();
                    entry.insert("key".to_string(), OwnedValue::Int(i as i64));
                    entry.insert("value".to_string(), to_owned(&elem));
                    OwnedValue::Object(entry)
                })
                .collect();
            QueryResult::Owned(OwnedValue::Array(entries))
        }
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
        _ => QueryResult::Error(EvalError::has_no_keys(&to_owned(&value))),
    }
}

/// The key aliases `from_entries` reads, in the order jq's `//` chain tries
/// them. `k`/`K` are *not* among them — jq 1.7.1 does not accept those.
const ENTRY_KEY_ALIASES: [&str; 4] = ["key", "Key", "name", "Name"];

/// The key and value jq takes from one `from_entries` entry:
/// `{(.key // .Key // .name // .Name): (if has("value") then .value else .Value end)}`.
///
/// The key chain is the alternative operator, not a presence test, so an alias
/// holding `null` or `false` is passed over in favour of a later one — but a
/// `0` is not, which is how a number key reaches the refusal in
/// [`entries_to_object`] rather than being quietly skipped.
///
/// The *last* alias does not fall through, because there is nothing left to
/// fall through to: `a // b` yields `b` whatever `b` is once `a` has nothing
/// truthy, so a `false` in `.Name` is the chain's value and is refused as a
/// boolean. Reading it as null instead is what made
/// `[{"Name":false}] | from_entries` report the wrong kind.
///
/// The value half is `if has("value") then .value else .Value end` — presence,
/// not truthiness, and so deliberately unlike the key half. An explicit
/// `"value": null` beats a `"Value"` beside it, which a `//` chain would get
/// wrong. Reading either half the other way changes the answer silently, which
/// is why they are one function: the asymmetry is the thing to keep.
///
/// A non-object entry is indexed all the same: jq's `.key` on a scalar is the
/// indexing error, and on `null` it is `null` — refused as a key by
/// [`entries_to_object`] before the value it returns beside it is ever used,
/// which is how `[null] | from_entries` reports the key and not a failure to
/// look up `value`.
fn entry_key_and_value(entry: &OwnedValue) -> Result<(OwnedValue, OwnedValue), EvalError> {
    let obj = match entry {
        OwnedValue::Object(obj) => obj,
        OwnedValue::Null => return Ok((OwnedValue::Null, OwnedValue::Null)),
        other => return Err(EvalError::cannot_index_with_field(other.type_name(), "key")),
    };

    // Irrefutable on a fixed-size array, so the tail is derived from the chain
    // rather than named twice.
    let [earlier @ .., tail] = &ENTRY_KEY_ALIASES;

    let key = earlier
        .iter()
        .find_map(|alias| obj.get(*alias).filter(|v| v.is_truthy()))
        .or_else(|| obj.get(*tail))
        .cloned()
        .unwrap_or(OwnedValue::Null);

    let value = obj
        .get("value")
        .or_else(|| obj.get("Value"))
        .cloned()
        .unwrap_or(OwnedValue::Null);

    Ok((key, value))
}

/// jq's `from_entries` over entries already materialised as [`OwnedValue`]s.
///
/// One definition, because `from_entries` and `with_entries` are one
/// definition in jq (`def with_entries(f): to_entries | map(f) |
/// from_entries;`) and had drifted here as two hand-written copies of the same
/// filter — the second of which is what silently dropped every entry it could
/// not use (#391).
///
/// The key is refused before the value beside it is used, as jq's `{(K):(V)}`
/// evaluates it: `[null] | from_entries` reports the key refusal, not a failure
/// to look up `value`. Re-inserting a key keeps its original position and
/// replaces its value, which is what jq's `add` over the mapped singletons
/// does.
fn entries_to_object<I: IntoIterator<Item = OwnedValue>>(
    entries: I,
) -> Result<IndexMap<String, OwnedValue>, EvalError> {
    let mut result = IndexMap::new();
    for entry in entries {
        let (key, value) = entry_key_and_value(&entry)?;
        let OwnedValue::String(k) = key else {
            return Err(EvalError::cannot_use_as_object_key(&key));
        };
        result.insert(k, value);
    }
    Ok(result)
}

/// Builtin: from_entries - [{key:k, value:v}] → {k:v}
fn builtin_from_entries<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    // map(f) is [.[] | f], and .[] over an object iterates its values, so jq
    // accepts an object of entries as readily as an array of them (#422).
    let entries: Vec<OwnedValue> = match value {
        StandardJson::Array(elements) => elements.map(|elem| to_owned(&elem)).collect(),
        StandardJson::Object(fields) => fields.map(|f| to_owned(&f.value())).collect(),
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::cannot_iterate(&to_owned(&value))),
    };
    match entries_to_object(entries) {
        Ok(result) => QueryResult::Owned(OwnedValue::Object(result)),
        // The `?` suffix swallows the refusal outright in jq, as the arm
        // above already does for a non-array/non-object input. See
        // `refuse_object_key` on why the flag is not yet reachable from
        // succinctly's own syntax.
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(e),
    }
}

/// Builtin: with_entries(f) - to_entries | map(f) | from_entries
///
/// Composed out of the two builtins jq composes it from, rather than
/// reimplementing either. That is what lets an *array* input through: jq's
/// `to_entries` accepts one (its keys are the indices), so `[1,2] |
/// with_entries(.)` reaches `from_entries` with number keys and inherits its
/// refusal — where the previous object-only shape reported `array ([1,2]) has
/// no keys` from a type check jq does not have (#391).
fn builtin_with_entries<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // `to_entries` owns the input-shape check, so a value with no keys at all
    // reports its sentence and nothing else needs to know the shape.
    let entries = match builtin_to_entries::<W>(value, optional) {
        QueryResult::Owned(OwnedValue::Array(entries)) => entries,
        QueryResult::None => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
        // `to_entries` yields an owned array on every success path today, so
        // this is dead — but it is *its* invariant, not one visible here, so a
        // change over there should degrade rather than abort a caller.
        _ => {
            return QueryResult::Error(EvalError::new(
                "to_entries did not yield an array of entries",
            ))
        }
    };

    // map(f)
    let mut transformed: Vec<OwnedValue> = Vec::new();
    for entry in entries {
        let entry_json = owned_to_json_bytes(&entry);
        let index = crate::json::JsonIndex::build(&entry_json);
        let cursor = index.root(&entry_json);

        match eval_single::<Vec<u64>, S>(f, cursor.value(), optional).materialize_cursor() {
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
            QueryResult::Break(label) => return QueryResult::Break(label),
        }
    }

    // from_entries
    match entries_to_object(transformed) {
        Ok(result) => QueryResult::Owned(OwnedValue::Object(result)),
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(e),
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
fn eval_string_interpolation<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    parts: &[StringPart],
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut result = String::new();

    for part in parts {
        match part {
            StringPart::Literal(s) => result.push_str(s),
            StringPart::Expr(expr) => {
                let val = eval_single::<W, S>(expr, value.clone(), optional).materialize_cursor();
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
                    QueryResult::Break(label) => return QueryResult::Break(label),
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
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            value.number_str().expect("numeric variant").into_owned()
        }
        OwnedValue::String(s) => s.clone(), // Don't quote strings in interpolation
        OwnedValue::Array(_) | OwnedValue::Object(_) => value.to_json(),
    }
}

/// Evaluate a format string: `@json`, `@uri`, etc.
fn eval_format<W: Clone + AsRef<[u64]>>(
    format_type: FormatType,
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    let owned = to_owned(&value);

    let result = match format_type {
        FormatType::Text => format_text(&owned),
        FormatType::Json => format_json(&owned),
        FormatType::Uri => format_uri(&owned, optional),
        FormatType::Csv => format_csv(&owned, optional),
        FormatType::Tsv => format_tsv(&owned, optional),
        FormatType::Dsv(delimiter) => format_dsv(&owned, &delimiter, optional),
        FormatType::Base64 => format_base64(&owned, optional),
        FormatType::Base64d => format_base64d(&owned, optional),
        FormatType::Html => format_html(&owned, optional),
        FormatType::Sh => format_sh(&owned, optional),
        FormatType::Urid => format_urid(&owned, optional),
        FormatType::Yaml => format_yaml(&owned),
        FormatType::Props => format_props(&owned),
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
fn format_uri(value: &OwnedValue, _optional: bool) -> Result<String, EvalError> {
    // jq converts non-strings to strings first (e.g., 42 | @uri => "42")
    let s = match value {
        OwnedValue::String(s) => s.clone(),
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            value.number_str().expect("numeric variant").into_owned()
        }
        OwnedValue::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        OwnedValue::Null => "null".to_string(),
        _ => return Err(EvalError::type_error("string", value.type_name())),
    };

    let mut result = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
            result.push(c);
        } else {
            for b in c.to_string().as_bytes() {
                result.push_str(&format!("%{b:02X}"));
            }
        }
    }
    Ok(result)
}

/// @urid - URI/percent decode
fn format_urid(value: &OwnedValue, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::String(s) => {
            let mut result = String::new();
            let bytes = s.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' && i + 2 < bytes.len() {
                    // Try to parse the next two characters as hex
                    if let (Some(h1), Some(h2)) = (
                        (bytes[i + 1] as char).to_digit(16),
                        (bytes[i + 2] as char).to_digit(16),
                    ) {
                        let decoded = (h1 * 16 + h2) as u8;
                        result.push(decoded as char);
                        i += 3;
                        continue;
                    }
                }
                // Not a valid percent-encoded sequence, just copy the character
                result.push(bytes[i] as char);
                i += 1;
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
                    // jq unconditionally double-quotes every string field
                    // (inner `"` doubled), regardless of whether it contains a
                    // delimiter — see #306.
                    OwnedValue::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
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

/// @dsv(delimiter) - Generic DSV format with custom delimiter (for arrays)
fn format_dsv(value: &OwnedValue, delimiter: &str, optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .map(|v| match v {
                    // Match @csv: always double-quote string fields (inner `"`
                    // doubled) so @dsv(",") stays byte-identical to @csv — #306.
                    OwnedValue::String(s) => format!("\"{}\"", s.replace('"', "\"\"")),
                    OwnedValue::Null => String::new(),
                    other => owned_to_string(other),
                })
                .collect();
            Ok(parts.join(delimiter))
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
                let b1 = chunk.get(1).map_or(0, |&b| b as u32);
                let b2 = chunk.get(2).map_or(0, |&b| b as u32);

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
fn format_html(value: &OwnedValue, _optional: bool) -> Result<String, EvalError> {
    // jq converts non-strings to strings first (e.g., 42 | @html => "42")
    let s = match value {
        OwnedValue::String(s) => s.clone(),
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            value.number_str().expect("numeric variant").into_owned()
        }
        OwnedValue::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        OwnedValue::Null => "null".to_string(),
        _ => return Err(EvalError::type_error("string", value.type_name())),
    };

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

/// Shell-quote a single value for @sh
fn shell_quote_value(value: &OwnedValue) -> String {
    match value {
        // jq always quotes strings in @sh array output
        OwnedValue::String(s) => {
            if s.contains('\'') {
                let escaped = s.replace('\'', "'\\''");
                format!("'{escaped}'")
            } else {
                format!("'{s}'")
            }
        }
        // Numbers, bools, null are NOT quoted in jq
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            value.number_str().expect("numeric variant").into_owned()
        }
        OwnedValue::Bool(b) => {
            if *b {
                "true".to_string()
            } else {
                "false".to_string()
            }
        }
        OwnedValue::Null => "null".to_string(),
        _ => String::new(),
    }
}

/// @sh - Shell quote
fn format_sh(value: &OwnedValue, _optional: bool) -> Result<String, EvalError> {
    match value {
        OwnedValue::String(s) => {
            // Use single quotes and escape single quotes
            if s.contains('\'') {
                let escaped = s.replace('\'', "'\\''");
                Ok(format!("'{escaped}'"))
            } else {
                Ok(format!("'{s}'"))
            }
        }
        // jq: [1, 2, 3] | @sh => "1 2 3"
        OwnedValue::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(shell_quote_value).collect();
            Ok(parts.join(" "))
        }
        // Numbers, bools, null are converted to strings
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            Ok(value.number_str().expect("numeric variant").into_owned())
        }
        OwnedValue::Bool(b) => Ok(if *b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        OwnedValue::Null => Ok("null".to_string()),
        _ => Err(EvalError::type_error("string", value.type_name())),
    }
}

/// @yaml - Format value as YAML string (yq)
fn format_yaml(value: &OwnedValue) -> Result<String, EvalError> {
    Ok(owned_to_yaml(value))
}

/// @props - Format value as Java properties format (yq)
///
/// Objects are flattened with dot-notation keys:
/// `{database: "postgres", nested: {a: 1}}` → `database = postgres\nnested.a = 1`
///
/// Arrays use numeric indices:
/// `{arr: [1, 2, 3]}` → `arr.0 = 1\narr.1 = 2\narr.2 = 3`
///
/// Non-objects are converted to strings.
fn format_props(value: &OwnedValue) -> Result<String, EvalError> {
    let mut lines = Vec::new();
    format_props_recursive(value, String::new(), &mut lines);
    Ok(lines.join("\n"))
}

/// Recursively format a value into Java properties lines
fn format_props_recursive(value: &OwnedValue, prefix: String, lines: &mut Vec<String>) {
    match value {
        OwnedValue::Object(obj) => {
            for (key, val) in obj {
                let new_prefix = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                format_props_recursive(val, new_prefix, lines);
            }
        }
        OwnedValue::Array(arr) => {
            for (idx, val) in arr.iter().enumerate() {
                let new_prefix = if prefix.is_empty() {
                    format!("{idx}")
                } else {
                    format!("{prefix}.{idx}")
                };
                format_props_recursive(val, new_prefix, lines);
            }
        }
        _ => {
            // Scalar value - output as "key = value"
            let value_str = props_value_to_string(value);
            if prefix.is_empty() {
                // Top-level scalar without key
                lines.push(value_str);
            } else {
                lines.push(format!("{prefix} = {value_str}"));
            }
        }
    }
}

/// Convert a scalar value to its properties string representation
fn props_value_to_string(value: &OwnedValue) -> String {
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(true) => "true".to_string(),
        OwnedValue::Bool(false) => "false".to_string(),
        OwnedValue::Int(n) => format!("{n}"),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                ".nan".to_string()
            } else if f.is_infinite() {
                if *f > 0.0 {
                    ".inf".to_string()
                } else {
                    "-.inf".to_string()
                }
            } else {
                format!("{f}")
            }
        }
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => ".nan".to_string(),
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_infinite() => {
            if *f > 0.0 {
                ".inf".to_string()
            } else {
                "-.inf".to_string()
            }
        }
        OwnedValue::NumberLiteral(_, literal) => format_number_jq_compat(literal.as_bytes()),
        OwnedValue::String(s) => {
            // Replace newlines with spaces, as yq does
            s.replace(['\n', '\r'], " ")
        }
        // Objects and arrays should not reach here, but handle gracefully
        OwnedValue::Object(_) | OwnedValue::Array(_) => value.to_json(),
    }
}

/// Convert OwnedValue to YAML flow-style (compact, single-line like JSON).
/// This matches yq's @yaml behavior which outputs flow-style YAML.
fn owned_to_yaml(value: &OwnedValue) -> String {
    match value {
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(true) => "true".to_string(),
        OwnedValue::Bool(false) => "false".to_string(),
        OwnedValue::Int(n) => format!("{n}"),
        OwnedValue::Float(f) => {
            if f.is_nan() {
                ".nan".to_string()
            } else if f.is_infinite() {
                if *f > 0.0 {
                    ".inf".to_string()
                } else {
                    "-.inf".to_string()
                }
            } else {
                format!("{f}")
            }
        }
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => ".nan".to_string(),
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_infinite() => {
            if *f > 0.0 {
                ".inf".to_string()
            } else {
                "-.inf".to_string()
            }
        }
        OwnedValue::NumberLiteral(_, literal) => format_number_jq_compat(literal.as_bytes()),
        OwnedValue::String(s) => yaml_quote_string(s),
        OwnedValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(owned_to_yaml).collect();
            format!("[{}]", items.join(", "))
        }
        OwnedValue::Object(obj) => {
            let pairs: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("{}: {}", yaml_quote_string(k), owned_to_yaml(v)))
                .collect();
            format!("{{{}}}", pairs.join(", "))
        }
    }
}

/// Quote a string for YAML output if necessary
fn yaml_quote_string(s: &str) -> String {
    // Check if string needs quoting
    let needs_quoting = s.is_empty()
        || s.starts_with(' ')
        || s.ends_with(' ')
        || s.contains(':')
        || s.contains('#')
        || s.contains('\n')
        || s.contains('\r')
        || s.contains('\t')
        || s.contains('"')
        || s.contains('\'')
        || s.contains('\\')
        || s.starts_with('-')
        || s.starts_with('?')
        || s.starts_with('*')
        || s.starts_with('&')
        || s.starts_with('!')
        || s.starts_with('|')
        || s.starts_with('>')
        || s.starts_with('%')
        || s.starts_with('@')
        || s.starts_with('`')
        || s.starts_with('{')
        || s.starts_with('}')
        || s.starts_with('[')
        || s.starts_with(']')
        || s.starts_with(',')
        || s == "true"
        || s == "false"
        || s == "null"
        || s == "~"
        || s == "yes"
        || s == "no"
        || s == "on"
        || s == "off"
        || s.parse::<i64>().is_ok()
        || s.parse::<f64>().is_ok();

    if needs_quoting {
        // Use double quotes and escape special characters
        let mut result = String::with_capacity(s.len() + 2);
        result.push('"');
        for c in s.chars() {
            match c {
                '"' => result.push_str("\\\""),
                '\\' => result.push_str("\\\\"),
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                // YAML's `\xNN`, not JSON's `\u00xx` — so this is deliberately
                // not one of the writers #385 unified in `super::escape`, and
                // `is_control()` is the right predicate here: YAML has an 8-bit
                // escape and `Cc` (C0, DEL, C1) is exactly what it covers.
                c if c.is_control() => {
                    result.push_str(&format!("\\x{:02x}", c as u32));
                }
                c => result.push(c),
            }
        }
        result.push('"');
        result
    } else {
        s.to_string()
    }
}

// =============================================================================
// Phase 6: Type Conversion Builtins
// =============================================================================

/// Builtin: tostring - convert any value to string
fn builtin_tostring<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
    let owned = to_owned(&value);
    let s = match owned {
        OwnedValue::String(s) => s,
        OwnedValue::Null => "null".to_string(),
        OwnedValue::Bool(true) => "true".to_string(),
        OwnedValue::Bool(false) => "false".to_string(),
        OwnedValue::Int(n) => format!("{n}"),
        OwnedValue::Float(f) => format!("{f}"),
        OwnedValue::NumberLiteral(_, literal) => format_number_jq_compat(literal.as_bytes()),
        OwnedValue::Array(_) | OwnedValue::Object(_) => owned.to_json(),
    };
    QueryResult::Owned(OwnedValue::String(s))
}

/// Builtin: tonumber - convert string to number
fn builtin_tonumber<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match &value {
        StandardJson::Number(n) => {
            // Already a number, return as-is -- this is a passthrough, not a
            // computation, so (like `.`) it keeps the source literal.
            QueryResult::Owned(match core::str::from_utf8(n.raw_bytes()) {
                Ok(s) => OwnedValue::from_number_literal(s),
                Err(_) => OwnedValue::Int(0),
            })
        }
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                match tonumber_from_str(cow.as_ref()) {
                    Ok(n) => QueryResult::Owned(n),
                    Err(_) if optional => QueryResult::None,
                    Err(e) => QueryResult::Error(e),
                }
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_parse_as_number(&to_owned(&value))),
    }
}

/// `tonumber` on a string, shared by both evaluators.
///
/// jq implements `tonumber` by handing the string to its JSON parser, which
/// splits the failures in two: a string that parses as some *other* JSON value
/// (`"null"`, `"true"`, `"[1]"`) is reported as the string it is, while one
/// that does not parse at all gets the parser's own diagnostic. We reuse
/// [`parse_complete_json`] to tell the two apart.
///
/// Kept in one place because the two evaluators previously had *different*
/// wording here — "cannot parse 'a' as number" against "cannot convert 'a' to
/// number" — which is exactly the drift #356 is about.
pub(super) fn tonumber_from_str(s: &str) -> Result<OwnedValue, EvalError> {
    // jq's JSON parser skips surrounding whitespace, so `" 1 "` is 1.
    let trimmed = s.trim();
    if let Ok(i) = trimmed.parse::<i64>() {
        Ok(OwnedValue::Int(i))
    } else if let Ok(f) = trimmed.parse::<f64>() {
        Ok(OwnedValue::Float(f))
    } else if parse_complete_json(trimmed).is_ok() {
        Err(EvalError::cannot_parse_as_number(&OwnedValue::String(
            s.to_string(),
        )))
    } else {
        Err(EvalError::invalid_numeric_literal(s))
    }
}

/// Builtin: toboolean - convert to boolean
/// Accepts: true, false, "true", "false"
fn builtin_toboolean<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match &value {
        StandardJson::Bool(b) => QueryResult::Owned(OwnedValue::Bool(*b)),
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                match cow.as_ref() {
                    "true" => QueryResult::Owned(OwnedValue::Bool(true)),
                    "false" => QueryResult::Owned(OwnedValue::Bool(false)),
                    _ if optional => QueryResult::None,
                    other => QueryResult::Error(EvalError::new(format!(
                        "string ({other:?}) cannot be parsed as a boolean"
                    ))),
                }
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new(format!(
            "{} cannot be parsed as a boolean",
            type_name(&value)
        ))),
    }
}

/// Builtin: skip(n; expr) - skip first n outputs from expr
fn builtin_skip<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    n_expr: &Expr,
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate n
    let n_result = eval_single::<W, S>(n_expr, value.clone(), optional);
    let n = match n_result {
        QueryResult::One(v) => {
            if let StandardJson::Number(num) = v {
                num.as_i64().unwrap_or(0) as usize
            } else {
                return QueryResult::Error(EvalError::type_error("number", type_name(&v)));
            }
        }
        QueryResult::Owned(
            ref owned @ (OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..)),
        ) => owned.as_f64().unwrap_or(0.0) as usize,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::Error(EvalError::type_error("number", "null")),
    };

    // Evaluate expr and skip first n results
    let result = eval_single::<W, S>(expr, value, optional);
    match result {
        QueryResult::One(v) => {
            if n == 0 {
                QueryResult::Owned(to_owned(&v))
            } else {
                QueryResult::None
            }
        }
        QueryResult::OneCursor(c) => {
            if n == 0 {
                QueryResult::Owned(to_owned(&c.value()))
            } else {
                QueryResult::None
            }
        }
        QueryResult::Owned(v) => {
            if n == 0 {
                QueryResult::Owned(v)
            } else {
                QueryResult::None
            }
        }
        QueryResult::Many(results) => {
            let skipped: Vec<OwnedValue> =
                results.into_iter().skip(n).map(|v| to_owned(&v)).collect();
            if skipped.is_empty() {
                QueryResult::None
            } else if skipped.len() == 1 {
                QueryResult::Owned(skipped.into_iter().next().unwrap())
            } else {
                QueryResult::ManyOwned(skipped)
            }
        }
        QueryResult::ManyOwned(results) => {
            let skipped: Vec<OwnedValue> = results.into_iter().skip(n).collect();
            if skipped.is_empty() {
                QueryResult::None
            } else if skipped.len() == 1 {
                QueryResult::Owned(skipped.into_iter().next().unwrap())
            } else {
                QueryResult::ManyOwned(skipped)
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Builtin: tojson - convert any value to JSON string
fn builtin_tojson<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
    let owned = to_owned(&value);
    let json_string = owned.to_json();
    QueryResult::Owned(OwnedValue::String(json_string))
}

/// Builtin: fromjson - parse JSON string to value
fn builtin_fromjson<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match &value {
        StandardJson::String(s) => {
            if let Ok(cow) = s.as_str() {
                let json_str = cow.as_ref();
                // The whole string must be one JSON value: jq rejects `"0x10"`
                // and `"1 2"` rather than reading the first value and dropping
                // the rest, which is what the old prefix parse did here.
                match parse_complete_json(json_str) {
                    Ok(owned) => QueryResult::Owned(owned),
                    Err(_) if optional => QueryResult::None,
                    Err(_) => QueryResult::Error(EvalError::invalid_numeric_literal(json_str)),
                }
            } else if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::new("invalid string"))
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::only_strings_can_be_parsed(&to_owned(&value))),
    }
}

/// Parse a JSON string that must consume all of its input.
///
/// This is what both `tonumber` and `fromjson` need, because jq parses their
/// argument with a parser that reports leftovers: the older prefix parse here
/// stopped at the end of the first value, so `"0x10"` came back as `0` with
/// `x10` silently dropped and `"1 2"` as `1`. `tonumber` needs it to tell
/// "this is valid JSON but not a number" (`"null"`) from "this is not valid
/// JSON at all" (`"0x10"`), which jq words differently.
fn parse_complete_json(s: &str) -> Result<OwnedValue, String> {
    let bytes = s.as_bytes();
    let mut pos = 0;
    let value = parse_json_value(bytes, &mut pos)?;
    while pos < bytes.len() && matches!(bytes[pos], b' ' | b'\t' | b'\n' | b'\r') {
        pos += 1;
    }
    if pos != bytes.len() {
        return Err("trailing content after JSON value".to_string());
    }
    Ok(value)
}

/// Parse a JSON value starting at the given position
fn parse_json_value(bytes: &[u8], pos: &mut usize) -> Result<OwnedValue, String> {
    // Skip whitespace
    while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }

    if *pos >= bytes.len() {
        return Err("unexpected end of input".to_string());
    }

    match bytes[*pos] {
        b'n' => {
            // null
            if bytes[*pos..].starts_with(b"null") {
                *pos += 4;
                Ok(OwnedValue::Null)
            } else {
                Err("expected 'null'".to_string())
            }
        }
        b't' => {
            // true
            if bytes[*pos..].starts_with(b"true") {
                *pos += 4;
                Ok(OwnedValue::Bool(true))
            } else {
                Err("expected 'true'".to_string())
            }
        }
        b'f' => {
            // false
            if bytes[*pos..].starts_with(b"false") {
                *pos += 5;
                Ok(OwnedValue::Bool(false))
            } else {
                Err("expected 'false'".to_string())
            }
        }
        b'"' => {
            // string
            parse_json_string_value(bytes, pos)
        }
        b'[' => {
            // array
            parse_json_array(bytes, pos)
        }
        b'{' => {
            // object
            parse_json_object(bytes, pos)
        }
        b'-' | b'0'..=b'9' => {
            // number
            parse_json_number(bytes, pos)
        }
        c => Err(format!("unexpected character: '{}'", c as char)),
    }
}

/// Parse a JSON string value
///
/// The bounds check is not redundant with [`parse_json_value`]'s: an object's
/// key position is reached from [`parse_json_object`] directly, so `"{"` and
/// `{"a":1,` arrive here at end of input.
fn parse_json_string_value(bytes: &[u8], pos: &mut usize) -> Result<OwnedValue, String> {
    if *pos >= bytes.len() {
        return Err("unexpected end of input".to_string());
    }
    if bytes[*pos] != b'"' {
        return Err("expected '\"'".to_string());
    }
    *pos += 1;

    let mut result = String::new();
    while *pos < bytes.len() {
        match bytes[*pos] {
            b'"' => {
                *pos += 1;
                return Ok(OwnedValue::String(result));
            }
            b'\\' => {
                *pos += 1;
                if *pos >= bytes.len() {
                    return Err("unexpected end of string".to_string());
                }
                match bytes[*pos] {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'/' => result.push('/'),
                    b'b' => result.push('\x08'),
                    b'f' => result.push('\x0C'),
                    b'n' => result.push('\n'),
                    b'r' => result.push('\r'),
                    b't' => result.push('\t'),
                    b'u' => {
                        // Unicode escape
                        *pos += 1;
                        if *pos + 4 > bytes.len() {
                            return Err("invalid unicode escape".to_string());
                        }
                        let hex = core::str::from_utf8(&bytes[*pos..*pos + 4])
                            .map_err(|_| "invalid unicode escape")?;
                        let codepoint =
                            u32::from_str_radix(hex, 16).map_err(|_| "invalid unicode escape")?;

                        // Handle surrogate pairs
                        if (0xD800..=0xDBFF).contains(&codepoint) {
                            // High surrogate - look for low surrogate
                            *pos += 4;
                            if *pos + 6 <= bytes.len()
                                && bytes[*pos] == b'\\'
                                && bytes[*pos + 1] == b'u'
                            {
                                let hex2 = core::str::from_utf8(&bytes[*pos + 2..*pos + 6])
                                    .map_err(|_| "invalid unicode escape")?;
                                let low = u32::from_str_radix(hex2, 16)
                                    .map_err(|_| "invalid unicode escape")?;
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    // Valid surrogate pair
                                    let combined =
                                        0x10000 + ((codepoint - 0xD800) << 10) + (low - 0xDC00);
                                    if let Some(c) = char::from_u32(combined) {
                                        result.push(c);
                                    }
                                    *pos += 5; // Move past the low surrogate (will be incremented again below)
                                } else {
                                    // Lone high surrogate - use replacement character
                                    result.push('\u{FFFD}');
                                    *pos -= 1; // Back up so we don't skip the next escape
                                }
                            } else {
                                // Lone high surrogate
                                result.push('\u{FFFD}');
                                *pos -= 1;
                            }
                        } else if let Some(c) = char::from_u32(codepoint) {
                            result.push(c);
                            *pos += 3; // Move past the hex digits (will be incremented again below)
                        } else {
                            return Err("invalid unicode codepoint".to_string());
                        }
                    }
                    c => return Err(format!("invalid escape sequence: \\{}", c as char)),
                }
                *pos += 1;
            }
            c => {
                // Regular character - handle UTF-8
                let remaining = &bytes[*pos..];
                if let Ok(s) = core::str::from_utf8(remaining) {
                    if let Some(c) = s.chars().next() {
                        result.push(c);
                        *pos += c.len_utf8();
                    } else {
                        return Err("unexpected end of string".to_string());
                    }
                } else {
                    // Try to get just the next character
                    result.push(c as char);
                    *pos += 1;
                }
            }
        }
    }
    Err("unterminated string".to_string())
}

/// Parse a JSON array
fn parse_json_array(bytes: &[u8], pos: &mut usize) -> Result<OwnedValue, String> {
    if *pos >= bytes.len() || bytes[*pos] != b'[' {
        return Err("expected '['".to_string());
    }
    *pos += 1;

    let mut elements = Vec::new();

    // Skip whitespace
    while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }

    // Check for empty array
    if *pos < bytes.len() && bytes[*pos] == b']' {
        *pos += 1;
        return Ok(OwnedValue::Array(elements));
    }

    loop {
        let value = parse_json_value(bytes, pos)?;
        elements.push(value);

        // Skip whitespace
        while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
            *pos += 1;
        }

        if *pos >= bytes.len() {
            return Err("unterminated array".to_string());
        }

        match bytes[*pos] {
            b']' => {
                *pos += 1;
                return Ok(OwnedValue::Array(elements));
            }
            b',' => {
                *pos += 1;
            }
            c => return Err(format!("expected ',' or ']', got '{}'", c as char)),
        }
    }
}

/// Parse a JSON object
fn parse_json_object(bytes: &[u8], pos: &mut usize) -> Result<OwnedValue, String> {
    if *pos >= bytes.len() || bytes[*pos] != b'{' {
        return Err("expected '{{'".to_string());
    }
    *pos += 1;

    let mut entries = IndexMap::new();

    // Skip whitespace
    while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }

    // Check for empty object
    if *pos < bytes.len() && bytes[*pos] == b'}' {
        *pos += 1;
        return Ok(OwnedValue::Object(entries));
    }

    loop {
        // Skip whitespace
        while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
            *pos += 1;
        }

        // Parse key (must be a string)
        let key = match parse_json_string_value(bytes, pos)? {
            OwnedValue::String(s) => s,
            _ => return Err("object key must be a string".to_string()),
        };

        // Skip whitespace
        while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
            *pos += 1;
        }

        // Expect colon
        if *pos >= bytes.len() || bytes[*pos] != b':' {
            return Err("expected ':'".to_string());
        }
        *pos += 1;

        // Parse value
        let value = parse_json_value(bytes, pos)?;
        entries.insert(key, value);

        // Skip whitespace
        while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\n' | b'\r') {
            *pos += 1;
        }

        if *pos >= bytes.len() {
            return Err("unterminated object".to_string());
        }

        match bytes[*pos] {
            b'}' => {
                *pos += 1;
                return Ok(OwnedValue::Object(entries));
            }
            b',' => {
                *pos += 1;
            }
            c => return Err(format!("expected ',' or '}}', got '{}'", c as char)),
        }
    }
}

/// Parse a JSON number
fn parse_json_number(bytes: &[u8], pos: &mut usize) -> Result<OwnedValue, String> {
    let start = *pos;

    // Optional minus sign
    if *pos < bytes.len() && bytes[*pos] == b'-' {
        *pos += 1;
    }

    // Integer part
    if *pos >= bytes.len() {
        return Err("expected number".to_string());
    }

    if bytes[*pos] == b'0' {
        *pos += 1;
    } else if bytes[*pos].is_ascii_digit() {
        while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
            *pos += 1;
        }
    } else {
        return Err("expected digit".to_string());
    }

    let mut is_float = false;

    // Fractional part
    if *pos < bytes.len() && bytes[*pos] == b'.' {
        is_float = true;
        *pos += 1;
        if *pos >= bytes.len() || !bytes[*pos].is_ascii_digit() {
            return Err("expected digit after decimal point".to_string());
        }
        while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }

    // Exponent part
    if *pos < bytes.len() && matches!(bytes[*pos], b'e' | b'E') {
        is_float = true;
        *pos += 1;
        if *pos < bytes.len() && matches!(bytes[*pos], b'+' | b'-') {
            *pos += 1;
        }
        if *pos >= bytes.len() || !bytes[*pos].is_ascii_digit() {
            return Err("expected digit in exponent".to_string());
        }
        while *pos < bytes.len() && bytes[*pos].is_ascii_digit() {
            *pos += 1;
        }
    }

    let num_str =
        core::str::from_utf8(&bytes[start..*pos]).map_err(|_| "invalid number encoding")?;

    if is_float {
        let f: f64 = num_str.parse().map_err(|_| "invalid float")?;
        Ok(OwnedValue::Float(f))
    } else {
        // Try integer first
        if let Ok(i) = num_str.parse::<i64>() {
            Ok(OwnedValue::Int(i))
        } else {
            // Fall back to float for large numbers
            let f: f64 = num_str.parse().map_err(|_| "invalid number")?;
            Ok(OwnedValue::Float(f))
        }
    }
}

// =============================================================================
// Phase 6: Additional String Builtins
// =============================================================================

/// Builtin: explode - string to array of Unicode codepoints
fn builtin_explode<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
        _ => QueryResult::Error(EvalError::new("explode input must be a string")),
    }
}

/// Builtin: implode - array of codepoints to string
fn builtin_implode<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
                                "invalid codepoint: {codepoint}"
                            )));
                        }
                    }
                }
            }
            QueryResult::Owned(OwnedValue::String(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new("implode input must be an array")),
    }
}

/// Builtin: test(re) - test if string matches (substring fallback without regex feature)
#[cfg(not(feature = "regex"))]
fn builtin_test<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(v) => return QueryResult::Error(EvalError::not_string_or_array(v.type_name())),
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
        _ => QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    }
}

/// The refusal for a non-string pattern handed to `indices`, `index` or
/// `rindex` on a string input.
///
/// jq implements all three by indexing the string with the pattern, so the
/// message is the indexing one — `"abc" | index(1)` is `Cannot index string
/// with number` — rather than a complaint that names the argument. One
/// definition, because the three call sites are identical and had already
/// drifted into wording (`expected string, got pattern`) that names no type.
///
/// An *object* pattern never reaches here: it is jq's slice, handled by
/// [`string_slice_pattern`].
fn non_string_pattern<W>(value: &StandardJson<'_, W>, pattern: &OwnedValue) -> EvalError {
    EvalError::cannot_index(type_name(value), pattern)
}

/// What `indices`/`index`/`rindex` answer for an *object* pattern against a
/// string, which in jq is the slice rather than a search.
///
/// jq defines all three over `.[$i]`, so an object pattern indexes the string
/// with a slice descriptor: `"abcabc" | indices({"start":1,"end":2})` is
/// `"b"`, not a list of positions. `index`/`rindex` then take `.[0]` of that
/// string and report jq's `Cannot index string with number` — which falls out
/// of their own definitions rather than needing a case here.
///
/// `None` when this is not a string searched by an object pattern, leaving
/// the caller's own [`non_string_pattern`] refusal in place.
fn string_slice_pattern<W>(
    value: &StandardJson<'_, W>,
    pattern: &OwnedValue,
) -> Option<Result<OwnedValue, EvalError>> {
    let (StandardJson::String(s), OwnedValue::Object(desc)) = (value, pattern) else {
        return None;
    };
    let Ok(text) = s.as_str() else {
        return Some(Err(EvalError::new("invalid string")));
    };
    Some(SliceBounds::from_descriptor(desc).map(|bounds| {
        OwnedValue::String(slice::slice_str(
            &text,
            bounds.resolve(text.chars().count()),
        ))
    }))
}

/// What `indices`, `index` and `rindex` answer for an input they cannot search.
///
/// jq routes a *string* pattern through `_strindices`, which answers `null`
/// for an input holding no characters to search rather than raising: `null |
/// index("a")` and `{} | index("a")` are both `null`. Anything else indexes the
/// input with the pattern, so a scalar — or an object handed a non-string
/// pattern, `{} | indices(1)` — reports that indexing error.
///
/// One definition, because this is the outer half of the same refusal
/// [`non_string_pattern`] covers, and the three searches had drifted here too.
fn unsearchable_input<'a, W: Clone + AsRef<[u64]>>(
    value: &StandardJson<'a, W>,
    pattern: &OwnedValue,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Null => QueryResult::Owned(OwnedValue::Null),
        StandardJson::Object(_) if matches!(pattern, OwnedValue::String(_)) => {
            QueryResult::Owned(OwnedValue::Null)
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_index(type_name(value), pattern)),
    }
}

/// Builtin: indices(s) - find all indices of substring/element s
fn builtin_indices<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    s_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern (can be any type for arrays, must be string for strings)
    let pattern = match result_to_owned(eval_single::<W, S>(s_expr, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            // An object pattern is jq's slice, which answers a *substring*
            // rather than a list of positions — see `string_slice_pattern`.
            if let Some(sliced) = string_slice_pattern(&value, &pattern) {
                return match sliced {
                    Ok(v) => QueryResult::Owned(v),
                    Err(_) if optional => QueryResult::None,
                    Err(e) => QueryResult::Error(e),
                };
            }
            // For strings, pattern must be a string
            let pattern_str = match &pattern {
                OwnedValue::String(p) => p,
                _ if optional => return QueryResult::None,
                _ => return QueryResult::Error(non_string_pattern(&value, &pattern)),
            };
            if let Ok(cow) = s.as_str() {
                let mut indices = Vec::new();
                let mut start = 0;
                while let Some(pos) = cow[start..].find(pattern_str.as_str()) {
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
            // For arrays, find indices where element equals the pattern (any type)
            let mut indices = Vec::new();
            for (i, elem) in (*elements).enumerate() {
                if to_owned(&elem) == pattern {
                    indices.push(OwnedValue::Int(i as i64));
                }
            }
            QueryResult::Owned(OwnedValue::Array(indices))
        }
        _ => unsearchable_input(&value, &pattern, optional),
    }
}

/// Builtin: index(s) - first index of substring/element s, or null
fn builtin_index<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    s_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern (can be any type for arrays, must be string for strings)
    let pattern = match result_to_owned(eval_single::<W, S>(s_expr, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            // jq defines `index`/`rindex` as `.[0]`/`.[-1:][0]` of what
            // `indices` answers, so an object pattern — jq's slice — leaves a
            // *substring* to be indexed with a number, and reports that.
            if let Some(sliced) = string_slice_pattern(&value, &pattern) {
                return match sliced {
                    _ if optional => QueryResult::None,
                    Err(e) => QueryResult::Error(e),
                    Ok(_) => {
                        QueryResult::Error(EvalError::cannot_index_with_type("string", "number"))
                    }
                };
            }
            // For strings, pattern must be a string
            let pattern_str = match &pattern {
                OwnedValue::String(p) => p,
                _ if optional => return QueryResult::None,
                _ => return QueryResult::Error(non_string_pattern(&value, &pattern)),
            };
            if let Ok(cow) = s.as_str() {
                if let Some(pos) = cow.find(pattern_str.as_str()) {
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
            // For arrays, pattern can be any type
            for (i, elem) in (*elements).enumerate() {
                if to_owned(&elem) == pattern {
                    return QueryResult::Owned(OwnedValue::Int(i as i64));
                }
            }
            QueryResult::Owned(OwnedValue::Null)
        }
        _ => unsearchable_input(&value, &pattern, optional),
    }
}

/// Builtin: rindex(s) - last index of substring/element s, or null
fn builtin_rindex<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    s_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern (can be any type for arrays, must be string for strings)
    let pattern = match result_to_owned(eval_single::<W, S>(s_expr, value.clone(), optional)) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    match &value {
        StandardJson::String(s) => {
            // jq defines `index`/`rindex` as `.[0]`/`.[-1:][0]` of what
            // `indices` answers, so an object pattern — jq's slice — leaves a
            // *substring* to be indexed with a number, and reports that.
            if let Some(sliced) = string_slice_pattern(&value, &pattern) {
                return match sliced {
                    _ if optional => QueryResult::None,
                    Err(e) => QueryResult::Error(e),
                    Ok(_) => {
                        QueryResult::Error(EvalError::cannot_index_with_type("string", "number"))
                    }
                };
            }
            // For strings, pattern must be a string
            let pattern_str = match &pattern {
                OwnedValue::String(p) => p,
                _ if optional => return QueryResult::None,
                _ => return QueryResult::Error(non_string_pattern(&value, &pattern)),
            };
            if let Ok(cow) = s.as_str() {
                if let Some(pos) = cow.rfind(pattern_str.as_str()) {
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
            // For arrays, pattern can be any type
            let items: Vec<_> = (*elements).collect();
            for (i, elem) in items.iter().enumerate().rev() {
                if to_owned(elem) == pattern {
                    return QueryResult::Owned(OwnedValue::Int(i as i64));
                }
            }
            QueryResult::Owned(OwnedValue::Null)
        }
        _ => unsearchable_input(&value, &pattern, optional),
    }
}

/// Builtin: tojsonstream - convert to JSON text stream format (simplified)
fn builtin_tojsonstream<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
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
                for (k, v) in obj {
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
fn builtin_fromjsonstream<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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

/// Resolve an array index for a *read*, the way jq resolves one.
///
/// Same truncation and negative-index rules as [`resolve_setpath_index`] — a
/// float truncates toward zero, a negative index counts back from the end —
/// but a read that lands nowhere is not an error: jq answers `null` for an
/// index past either end, and for NaN, which reaches no element at all.
/// `None` is that "no such element".
fn resolve_read_index(key: &OwnedValue, len: usize) -> Option<usize> {
    // `as` saturates, so ±inf lands past the end and reads as `null`; NaN
    // returns `None` here same as it does from `numeric_key_to_index`.
    let index = numeric_key_to_index(key)?;
    let resolved = if index < 0 { len as i64 + index } else { index };
    usize::try_from(resolved).ok().filter(|i| *i < len)
}

/// Builtin: getpath(path) - get value at path
fn builtin_getpath<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    path_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the path expression
    let path = match result_to_owned(eval_single::<W, S>(path_expr, value.clone(), optional)) {
        Ok(OwnedValue::Array(arr)) => arr,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::path_must_be_array()),
        Err(e) => return QueryResult::Error(e),
    };

    let mut current = to_owned(&value);

    for segment in path {
        match (&current, &segment) {
            // jq: null | getpath(["a"]) => null
            (OwnedValue::Null, _) => {
                return QueryResult::Owned(OwnedValue::Null);
            }
            (OwnedValue::Object(obj), OwnedValue::String(key)) => {
                current = obj.get(key).cloned().unwrap_or(OwnedValue::Null);
            }
            (
                OwnedValue::Array(arr),
                OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..),
            ) => {
                current = resolve_read_index(&segment, arr.len())
                    .map_or(OwnedValue::Null, |i| arr[i].clone());
            }
            // An object segment is jq's slice, `{"start":s,"end":e}`. jq
            // checks the *container* first — an object or a scalar reports
            // `Cannot index <type> with object` below without ever looking at
            // the descriptor — and only then validates the bounds.
            (OwnedValue::Array(arr), OwnedValue::Object(desc)) => {
                let range = match SliceBounds::from_descriptor(desc) {
                    Ok(bounds) => bounds.resolve(arr.len()),
                    Err(_) if optional => return QueryResult::None,
                    Err(e) => return QueryResult::Error(e),
                };
                current = OwnedValue::Array(arr[range].to_vec());
            }
            (OwnedValue::String(s), OwnedValue::Object(desc)) => {
                let range = match SliceBounds::from_descriptor(desc) {
                    Ok(bounds) => bounds.resolve(s.chars().count()),
                    Err(_) if optional => return QueryResult::None,
                    Err(e) => return QueryResult::Error(e),
                };
                current = OwnedValue::String(slice::slice_str(s, range));
            }
            _ if optional => return QueryResult::None,
            _ => {
                return QueryResult::Error(EvalError::cannot_index(current.type_name(), &segment));
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
            pattern = format!("{prefix}{pattern}");
        }
    }

    regex::Regex::new(&pattern).map_err(|e| EvalError::new(format!("invalid regex: {e}")))
}

/// Builtin: test(re) - test if string matches the regex
#[cfg(feature = "regex")]
fn builtin_test_regex<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(v) => return QueryResult::Error(EvalError::not_string_or_array(v.type_name())),
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex and test for a match
    let re = match build_regex(&pattern, None) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Bool(re.is_match(&input)))
}

/// Builtin: match(re) or match(re; flags) - return match object
#[cfg(feature = "regex")]
fn builtin_match<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags: Option<&str>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(v) => return QueryResult::Error(EvalError::not_string_or_array(v.type_name())),
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, flags) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Check if global flag is set
    let global = flags.is_some_and(|f| f.contains('g'));

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
                cap.map_or(OwnedValue::Null, |m| OwnedValue::Int(m.start() as i64)),
            );
            cap_obj.insert(
                "length".to_string(),
                cap.map_or(OwnedValue::Int(0), |m| OwnedValue::Int(m.len() as i64)),
            );
            cap_obj.insert(
                "string".to_string(),
                cap.map_or(OwnedValue::Null, |m| {
                    OwnedValue::String(m.as_str().to_string())
                }),
            );
            cap_obj.insert(
                "name".to_string(),
                name.map_or(OwnedValue::Null, |n| OwnedValue::String(n.to_string())),
            );
            captures.push(OwnedValue::Object(cap_obj));
        }
    }
    obj.insert("captures".to_string(), OwnedValue::Array(captures));

    OwnedValue::Object(obj)
}

/// Builtin: capture(re) - return named captures as object
#[cfg(feature = "regex")]
fn builtin_capture<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(v) => return QueryResult::Error(EvalError::not_string_or_array(v.type_name())),
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
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
fn builtin_scan<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
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
fn builtin_splits<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
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
fn builtin_sub<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the replacement
    let replacement = match result_to_owned(eval_single::<W, S>(
        replacement_expr,
        value.clone(),
        optional,
    )) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
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
fn builtin_gsub<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the replacement
    let replacement = match result_to_owned(eval_single::<W, S>(
        replacement_expr,
        value.clone(),
        optional,
    )) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
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

/// Builtin: test(re; flags) - test with flags expression
#[cfg(feature = "regex")]
fn builtin_test_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(v) => return QueryResult::Error(EvalError::not_string_or_array(v.type_name())),
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex with flags
    let re = match build_regex(&pattern, Some(&flags)) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Test if regex matches
    QueryResult::Owned(OwnedValue::Bool(re.is_match(&input)))
}

/// Builtin: match(re; flags) - match with flags expression
#[cfg(feature = "regex")]
fn builtin_match_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_match::<W, S>(re_expr, Some(&flags), value, optional)
}

/// Builtin: capture(re; flags) - capture with flags expression
#[cfg(feature = "regex")]
fn builtin_capture_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_capture_with_flags::<W, S>(re_expr, Some(&flags), value, optional)
}

/// Builtin: capture(re) or capture(re; flags) - capture named groups
#[cfg(feature = "regex")]
fn builtin_capture_with_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags: Option<&str>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(v) => return QueryResult::Error(EvalError::not_string_or_array(v.type_name())),
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, flags) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Find first match and extract named captures
    if let Some(caps) = re.captures(&input) {
        let mut entries = IndexMap::new();
        for name in re.capture_names().flatten() {
            if let Some(m) = caps.name(name) {
                entries.insert(name.to_string(), OwnedValue::String(m.as_str().to_string()));
            }
        }
        QueryResult::Owned(OwnedValue::Object(entries))
    } else if optional {
        QueryResult::None
    } else {
        // jq returns empty object when no match
        QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
    }
}

/// Builtin: sub(re; replacement; flags) - replace first match with flags
#[cfg(feature = "regex")]
fn builtin_sub_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_sub_with_flags::<W, S>(re_expr, replacement_expr, Some(&flags), value, optional)
}

/// Builtin: sub(re; replacement) or sub(re; replacement; flags) - replace first match
#[cfg(feature = "regex")]
fn builtin_sub_with_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    flags: Option<&str>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the replacement
    let replacement = match result_to_owned(eval_single::<W, S>(
        replacement_expr,
        value.clone(),
        optional,
    )) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, flags) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Convert jq replacement syntax (\(.name)) to regex replacement syntax ($name)
    let replacement = convert_jq_replacement(&replacement);

    // Replace first match
    let result = re.replace(&input, replacement.as_str());
    QueryResult::Owned(OwnedValue::String(result.into_owned()))
}

/// Builtin: gsub(re; replacement; flags) - replace all matches with flags
#[cfg(feature = "regex")]
fn builtin_gsub_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_gsub_with_flags::<W, S>(re_expr, replacement_expr, Some(&flags), value, optional)
}

/// Builtin: gsub(re; replacement) or gsub(re; replacement; flags) - replace all matches
#[cfg(feature = "regex")]
fn builtin_gsub_with_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    replacement_expr: &Expr,
    flags: Option<&str>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "pattern")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the replacement
    let replacement = match result_to_owned(eval_single::<W, S>(
        replacement_expr,
        value.clone(),
        optional,
    )) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, flags) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Convert jq replacement syntax (\(.name)) to regex replacement syntax ($name)
    let replacement = convert_jq_replacement(&replacement);

    // Replace all matches
    let result = re.replace_all(&input, replacement.as_str());
    QueryResult::Owned(OwnedValue::String(result.into_owned()))
}

/// Builtin: scan(re; flags) - find all matches with flags
#[cfg(feature = "regex")]
fn builtin_scan_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_scan_with_flags::<W, S>(re_expr, Some(&flags), value, optional)
}

/// Builtin: scan(re) or scan(re; flags) - find all matches
#[cfg(feature = "regex")]
fn builtin_scan_with_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags: Option<&str>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, flags) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Find all matches
    let mut results = Vec::new();
    let capture_count = re.captures_len();

    for caps in re.captures_iter(&input) {
        if capture_count > 1 {
            // Has capture groups - return array of captured strings
            let mut captured = Vec::new();
            for i in 1..capture_count {
                if let Some(m) = caps.get(i) {
                    captured.push(OwnedValue::String(m.as_str().to_string()));
                }
            }
            results.push(OwnedValue::Array(captured));
        } else {
            // No capture groups - return the matched string
            if let Some(m) = caps.get(0) {
                results.push(OwnedValue::String(m.as_str().to_string()));
            }
        }
    }

    if results.is_empty() {
        QueryResult::None
    } else {
        QueryResult::ManyOwned(results)
    }
}

/// Builtin: split(re; flags) - split by regex with flags
#[cfg(feature = "regex")]
fn builtin_split_regex<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, Some(&flags)) {
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

/// Builtin: splits(re; flags) - split by regex with flags as stream
#[cfg(feature = "regex")]
fn builtin_splits_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the flags expression
    let flags = match result_to_owned(eval_single::<W, S>(flags_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "flags")),
        Err(e) => return QueryResult::Error(e),
    };

    builtin_splits_with_flags::<W, S>(re_expr, Some(&flags), value, optional)
}

/// Builtin: splits(re) or splits(re; flags) - split by regex as stream
#[cfg(feature = "regex")]
fn builtin_splits_with_flags<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    re_expr: &Expr,
    flags: Option<&str>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the pattern
    let pattern = match result_to_owned(eval_single::<W, S>(re_expr, value.clone(), optional)) {
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
        _ => return QueryResult::Error(EvalError::cannot_be_matched(&to_owned(&value))),
    };

    // Build regex
    let re = match build_regex(&pattern, flags) {
        Ok(r) => r,
        Err(_e) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Split by regex and return as stream
    let parts: Vec<OwnedValue> = re
        .split(&input)
        .map(|s| OwnedValue::String(s.to_string()))
        .collect();

    if parts.is_empty() {
        QueryResult::None
    } else {
        QueryResult::ManyOwned(parts)
    }
}

/// Convert jq replacement syntax to regex replacement syntax
/// jq uses \(.name) for backreferences, regex crate uses $name or ${name}
#[cfg(feature = "regex")]
fn convert_jq_replacement(replacement: &str) -> String {
    // Simple conversion: \(.name) -> ${name}
    // This is a simplified version; full jq supports arbitrary expressions
    let mut result = String::new();
    let mut chars = replacement.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if chars.peek() == Some(&'(') {
                chars.next(); // consume '('
                let mut name = String::new();
                while let Some(&nc) = chars.peek() {
                    if nc == ')' {
                        chars.next(); // consume ')'
                        break;
                    }
                    name.push(nc);
                    chars.next();
                }
                // Check if it's a simple variable reference like .name
                if let Some(stripped) = name.strip_prefix('.') {
                    result.push_str("${");
                    result.push_str(stripped);
                    result.push('}');
                } else {
                    // Not a simple reference, just output literally
                    result.push_str("\\(");
                    result.push_str(&name);
                    result.push(')');
                }
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Evaluate a pipe (chain) of expressions.
fn eval_pipe<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    exprs: &[Expr],
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Check if any expression in the pipe needs path context (PathNoArg, Parent)
    if exprs.iter().any(needs_path_context) {
        let owned = to_owned(&value);
        return eval_pipe_with_path_context::<W, S>(exprs, &owned, &[], optional);
    }

    if exprs.is_empty() {
        return QueryResult::One(value);
    }

    let (first, rest) = exprs.split_first().unwrap();

    // Evaluate first expression
    let result = eval_single::<W, S>(first, value, optional);

    if rest.is_empty() {
        return result;
    }

    // Apply remaining expressions to the result
    match result.materialize_cursor() {
        QueryResult::One(v) => eval_pipe::<W, S>(rest, v, optional),
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(values) => {
            // Rest-of-pipe applied per element may yield borrowed (One/Many) OR
            // computed owned (Owned/ManyOwned) results — e.g. `.+1`, `[.]`,
            // `tostring`. Keep the borrowed fast-path (return `Many`), but the
            // moment any owned result appears, promote the whole batch to owned
            // so nothing is dropped (#295). Order is preserved across promotion.
            let mut borrowed: Vec<StandardJson<'a, W>> = Vec::new();
            let mut owned: Option<Vec<OwnedValue>> = None;
            for v in values {
                match eval_pipe::<W, S>(rest, v, optional).materialize_cursor() {
                    QueryResult::One(r) => match owned.as_mut() {
                        Some(acc) => acc.push(to_owned(&r)),
                        None => borrowed.push(r),
                    },
                    QueryResult::OneCursor(_) => unreachable!(),
                    QueryResult::Many(rs) => match owned.as_mut() {
                        Some(acc) => acc.extend(rs.iter().map(to_owned)),
                        None => borrowed.extend(rs),
                    },
                    QueryResult::Owned(r) => owned
                        .get_or_insert_with(|| {
                            core::mem::take(&mut borrowed)
                                .iter()
                                .map(to_owned)
                                .collect()
                        })
                        .push(r),
                    QueryResult::ManyOwned(rs) => owned
                        .get_or_insert_with(|| {
                            core::mem::take(&mut borrowed)
                                .iter()
                                .map(to_owned)
                                .collect()
                        })
                        .extend(rs),
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
                }
            }
            match owned {
                Some(acc) => QueryResult::ManyOwned(acc),
                None => QueryResult::Many(borrowed),
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Break(label) => QueryResult::Break(label),
        QueryResult::Owned(v) => {
            // Continue piping with owned value using eval_owned_pipe
            // Convert the result type since eval_owned_pipe uses Vec<u64> internally
            match eval_owned_pipe::<Vec<u64>, S>(rest, v, optional) {
                QueryResult::Owned(o) => QueryResult::Owned(o),
                QueryResult::Error(e) => QueryResult::Error(e),
                QueryResult::None => QueryResult::None,
                QueryResult::ManyOwned(vs) => QueryResult::ManyOwned(vs),
                QueryResult::Break(label) => QueryResult::Break(label),
                _ => unreachable!("eval_owned_pipe only returns Owned variants"),
            }
        }
        QueryResult::ManyOwned(vs) => {
            // Pipe each owned value through the rest
            let mut all_results: Vec<OwnedValue> = Vec::new();
            for v in vs {
                match eval_owned_pipe::<Vec<u64>, S>(rest, v, optional).materialize_cursor() {
                    QueryResult::Owned(r) => all_results.push(r),
                    QueryResult::OneCursor(_) => unreachable!(),
                    QueryResult::ManyOwned(rs) => all_results.extend(rs),
                    QueryResult::One(r) => all_results.push(to_owned(&r)),
                    QueryResult::Many(rs) => all_results.extend(rs.iter().map(to_owned)),
                    QueryResult::None => {}
                    QueryResult::Error(e) => return QueryResult::Error(e),
                    QueryResult::Break(label) => return QueryResult::Break(label),
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
fn eval_owned_pipe<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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

    eval_owned_input::<W, S>(&rest_expr, &value, optional)
}

/// Find a field in an object by name.
fn find_field<'a, W: Clone + AsRef<[u64]>>(
    fields: JsonFields<'a, W>,
    name: &str,
) -> Option<StandardJson<'a, W>> {
    fields.find(name)
}

/// Look a string key up in an object — the shared body of `.foo`, `.["foo"]`
/// and the string-key case of `.[$k]`.
///
/// Shared so a computed key cannot drift from a literal one; in particular the
/// null-input passthrough below is reached *only* for a valid key kind, which is
/// what makes `null | .["a"]` yield `null` while `null | .[null]` errors.
#[inline]
fn index_object_by_name<'a, W: Clone + AsRef<[u64]>>(
    value: StandardJson<'a, W>,
    name: &str,
    optional: bool,
) -> QueryResult<'a, W> {
    match value {
        StandardJson::Object(fields) => match find_field::<W>(fields, name) {
            Some(v) => QueryResult::One(v),
            // jq returns null for missing fields on objects (not an error)
            None => QueryResult::One(StandardJson::Null),
        },
        // jq returns null for field access on null
        StandardJson::Null => QueryResult::One(StandardJson::Null),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_index_with_field(type_name(&value), name)),
    }
}

/// Index an array by position — the shared body of `.[0]` and the numeric-key
/// case of `.[$k]`. See [`index_object_by_name`] for why this is shared.
#[inline]
fn index_array_by_position<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    idx: i64,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => match get_element_at_index::<W>(elements, idx) {
            Some(v) => QueryResult::One(v),
            // jq returns null for out-of-bounds array access (not an error)
            None => QueryResult::One(StandardJson::Null),
        },
        // jq returns null for index on null
        StandardJson::Null => QueryResult::One(StandardJson::Null),
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_index_with_type(
            type_name(&value),
            "number",
        )),
    }
}

/// Truncate a numeric key to an array index the way jq does.
///
/// Truncation is toward zero, not floor: measured against jq 1.7.1, `.[-1.5]`
/// on `[10,20,30]` is `30` (index -1), not `20`. Out-of-range floats saturate
/// via `as`, so `.[1e100]` reads as a huge index and yields `null` rather than
/// wrapping or panicking.
///
/// `None` means "numeric, but no index" — NaN only. `f64 as i64` maps NaN to
/// `0`, which would silently read (and, in an assignment, *write*) element zero;
/// jq's `.[nan]` is `null`. Callers must treat `None` as out of bounds on a
/// read, and reject it on a write — see [`key_to_path_component`].
pub(crate) fn numeric_key_to_index(key: &OwnedValue) -> Option<i64> {
    match key {
        OwnedValue::Int(i) => Some(*i),
        OwnedValue::Float(f) if !f.is_nan() => Some(f.trunc() as i64),
        OwnedValue::NumberLiteral(NumberRepr::Int(i), _) => Some(*i),
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if !f.is_nan() => Some(f.trunc() as i64),
        _ => None,
    }
}

/// Apply one resolved key to one target value.
///
/// The key kind is dispatched *before* the container is inspected, so the error
/// is jq's `Cannot index <container> with <key>` rather than the generic
/// `expected object, got …` that the static arms produce.
fn index_one<'a, W: Clone + AsRef<[u64]>>(
    target: StandardJson<'a, W>,
    key: &OwnedValue,
    optional: bool,
) -> QueryResult<'a, W> {
    let indexable_by_string = matches!(target, StandardJson::Object(_) | StandardJson::Null);
    let indexable_by_number = matches!(target, StandardJson::Array(_) | StandardJson::Null);

    match key {
        OwnedValue::String(s) if indexable_by_string => {
            index_object_by_name::<W>(target, s, optional)
        }
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..)
            if indexable_by_number =>
        {
            match numeric_key_to_index(key) {
                Some(idx) => index_array_by_position::<W>(target, idx, optional),
                // NaN: a number, so the container check above still applies, but
                // no element. jq yields null rather than erroring.
                None => QueryResult::One(StandardJson::Null),
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::cannot_index(type_name(&target), key)),
    }
}

/// [`index_one`] for a target that is already an owned value, as when the
/// indexed expression computed rather than navigated (`(.a|tostring)[$k]`).
///
/// Mirrors the borrowed path's rules exactly: missing key and out-of-bounds
/// index both yield null, null input passes through for a valid key kind, and
/// an invalid key kind errors even on null.
pub(crate) fn index_one_owned(
    target: &OwnedValue,
    key: &OwnedValue,
    optional: bool,
) -> Result<Option<OwnedValue>, EvalError> {
    match (key, target) {
        (OwnedValue::String(s), OwnedValue::Object(map)) => {
            Ok(Some(map.get(s).cloned().unwrap_or(OwnedValue::Null)))
        }
        (OwnedValue::String(_), OwnedValue::Null) => Ok(Some(OwnedValue::Null)),
        (
            OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..),
            OwnedValue::Array(items),
        ) => {
            // A NaN key has no index, so it reads as out of bounds — null.
            let Some(idx) = numeric_key_to_index(key) else {
                return Ok(Some(OwnedValue::Null));
            };
            let resolved = if idx < 0 {
                items.len() as i64 + idx
            } else {
                idx
            };
            let element = usize::try_from(resolved)
                .ok()
                .and_then(|i| items.get(i))
                .cloned()
                .unwrap_or(OwnedValue::Null);
            Ok(Some(element))
        }
        (
            OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..),
            OwnedValue::Null,
        ) => Ok(Some(OwnedValue::Null)),
        _ if optional => Ok(None),
        _ => Err(EvalError::cannot_index(owned_type_name(target), key)),
    }
}

/// Evaluate `E[K]` — indexing by a computed key.
///
/// jq compiles this as `K as $k | E | .[$k]`, and three consequences of that
/// desugaring are load-bearing (each measured against jq 1.7.1):
///
/// 1. `K` is evaluated against *this node's* input, not against `E`'s output —
///    which is why [`Expr::IndexExpr`] carries its own target instead of being
///    a flat chain element.
/// 2. The key stream is outer and the target stream inner:
///    `[({"a":1,"b":2},{"a":3,"b":4})[("a","b")]]` is `[1,3,2,4]`.
/// 3. An empty key stream short-circuits *before* `E` runs:
///    `[(error("boom"))[empty]]` is `[]`, not an error.
///
/// A trailing `?` covers the indexing, and *only* the indexing — jq's
/// `gen_index_opt(obj, key)` puts one opcode in its opt form and compiles both
/// halves normally. So neither the key nor the target is evaluated optionally:
/// `.[error("boom")]?` still raises `boom`, `{"k":"a","a":1} | [.. | .[.k]?]`
/// still fails on `Cannot index string with string "k"` when `..` reaches the
/// string `"a"`, and `"str" | .a[length]?` still fails on `Cannot index string
/// with string "a"` — which the folded spelling `"str" | .a[0]?` has always
/// done, so passing `optional` down here would make `?` mean two different
/// things depending on whether the key happened to be a constant. `optional`
/// reaches [`index_one`] and nothing else.
fn eval_index_expr<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    target: &Expr,
    key: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // (3) Keys first: an empty key stream must not evaluate the target.
    let keys = match eval_single::<W, S>(key, value.clone(), false).materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
        QueryResult::OneCursor(_) => unreachable!("materialize_cursor should have converted this"),
    };
    if keys.is_empty() {
        return QueryResult::None;
    }

    let targets = eval_single::<W, S>(target, value, false).materialize_cursor();

    // Borrowed and owned targets are kept apart so the common (borrowed) case
    // never materializes the document.
    enum Targets<'a, W> {
        Borrowed(Vec<StandardJson<'a, W>>),
        Owned(Vec<OwnedValue>),
    }
    let targets = match targets {
        QueryResult::One(v) => Targets::Borrowed(vec![v]),
        QueryResult::Many(vs) => Targets::Borrowed(vs),
        QueryResult::Owned(v) => Targets::Owned(vec![v]),
        QueryResult::ManyOwned(vs) => Targets::Owned(vs),
        QueryResult::None => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
        QueryResult::OneCursor(_) => unreachable!("materialize_cursor should have converted this"),
    };

    // (2) Key outer, target inner.
    match targets {
        Targets::Borrowed(ts) => {
            let mut out: Vec<StandardJson<'a, W>> = Vec::with_capacity(keys.len() * ts.len());
            for k in &keys {
                for t in &ts {
                    match index_one::<W>(t.clone(), k, optional) {
                        QueryResult::One(v) => out.push(v),
                        QueryResult::None => {}
                        QueryResult::Error(e) => return QueryResult::Error(e),
                        _ => unreachable!("index_one yields only One/None/Error"),
                    }
                }
            }
            match out.len() {
                1 => QueryResult::One(out.pop().expect("len checked")),
                _ => QueryResult::Many(out),
            }
        }
        Targets::Owned(ts) => {
            let mut out: Vec<OwnedValue> = Vec::with_capacity(keys.len() * ts.len());
            for k in &keys {
                for t in &ts {
                    match index_one_owned(t, k, optional) {
                        Ok(Some(v)) => out.push(v),
                        Ok(None) => {}
                        Err(e) => return QueryResult::Error(e),
                    }
                }
            }
            match out.len() {
                1 => QueryResult::Owned(out.pop().expect("len checked")),
                _ => QueryResult::ManyOwned(out),
            }
        }
    }
}

/// Get element at index (supports negative indexing).
///
/// Uses `get_fast` for O(n) BP operations + O(log n) IB select,
/// instead of `get` which does O(n) IB selects.
fn get_element_at_index<W: Clone + AsRef<[u64]>>(
    elements: JsonElements<'_, W>,
    idx: i64,
) -> Option<StandardJson<'_, W>> {
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
fn slice_elements<W: Clone + AsRef<[u64]>>(
    elements: JsonElements<'_, W>,
    start: Option<i64>,
    end: Option<i64>,
) -> Vec<StandardJson<'_, W>> {
    let all: Vec<_> = elements.collect();
    let range = SliceBounds::from_literals(start, end).resolve(all.len());
    all.into_iter()
        .skip(range.start)
        .take(range.len())
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
/// let result = eval::<Vec<u64>, JqSemantics>(&expr, cursor);
/// ```
pub fn eval<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    cursor: JsonCursor<'a, W>,
) -> QueryResult<'a, W> {
    // Special case: Identity returns the cursor directly for efficient output
    // This avoids decomposing arrays/objects into individual cursors
    if matches!(expr, Expr::Identity) {
        return QueryResult::OneCursor(cursor);
    }
    eval_single::<W, S>(expr, cursor.value(), false)
}

/// Evaluate a jq expression, returning only successfully matched values.
/// Errors and None results are filtered out.
pub fn eval_lenient<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    cursor: JsonCursor<'a, W>,
) -> Vec<StandardJson<'a, W>> {
    match eval::<W, S>(expr, cursor) {
        QueryResult::One(v) => vec![v],
        QueryResult::OneCursor(c) => vec![c.value()],
        QueryResult::Many(vs) => vs,
        QueryResult::None => Vec::new(),
        QueryResult::Error(_) => Vec::new(),
        QueryResult::Owned(_) => Vec::new(), // Owned values not returned as StandardJson
        QueryResult::ManyOwned(_) => Vec::new(),
        QueryResult::Break(_) => Vec::new(), // Break without matching label
    }
}

// =============================================================================
// Assignment Operators Implementation
// =============================================================================

/// Evaluate an expression once against `value`, reducing its output stream to
/// a single owned value: `One`/`Owned` pass through, `None` becomes `Null`,
/// and a multi-valued stream keeps only its first element (also `Null` if
/// empty). Used for the right-hand side of assignment operators, all of which
/// jq evaluates as a single value rather than once per updated path.
/// `Err` carries a `QueryResult` the caller should return immediately
/// (an error or an in-flight `break`).
fn eval_rhs_once<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> Result<OwnedValue, QueryResult<'a, W>> {
    match eval_single::<W, S>(expr, value, optional).materialize_cursor() {
        QueryResult::One(v) => Ok(to_owned(&v)),
        QueryResult::Owned(v) => Ok(v),
        QueryResult::None => Ok(OwnedValue::Null),
        QueryResult::Error(e) => Err(QueryResult::Error(e)),
        QueryResult::Many(vs) => Ok(vs.first().map_or(OwnedValue::Null, to_owned)),
        QueryResult::ManyOwned(vs) => Ok(vs.into_iter().next().unwrap_or(OwnedValue::Null)),
        QueryResult::OneCursor(_) => unreachable!("materialize_cursor should have converted this"),
        QueryResult::Break(label) => Err(QueryResult::Break(label)),
    }
}

/// Evaluate simple assignment: `.path = value`
/// Sets the value at path and returns the modified input.
fn eval_assign<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    path_expr: &Expr,
    value_expr: &Expr,
    input: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // First evaluate the value expression
    let mut new_value = match eval_rhs_once::<W, S>(value_expr, input.clone(), optional) {
        Ok(v) => v,
        Err(early_return) => return early_return,
    };

    // Convert input to owned for modification
    let mut result = to_owned(&input);

    // Resolve computed keys against the *original* document, before any write,
    // then apply every resolved path: `.[("a","b")] = 1` assigns both.
    //
    // `optional` here is `=`'s *own* `?` (`(.a = 1)?`), i.e. jq's ordinary
    // `try/catch` around the whole expression -- not a per-step tolerance. It
    // must never reach `set_path` as a starting flag: `set_path`'s own
    // `Expr::Optional` arm already gives per-component `?` its narrower
    // meaning (path production only, never the write-time bounds check), and
    // conflating the two would either over-suppress an inline `.[-5]? = 9` or
    // under-suppress an outer `(.[-5] = 9)?` depending on which flag won.
    // Instead every fallible step here is caught at this boundary and turned
    // into empty output when the whole call is optional -- mirroring how
    // `builtin_del` (#537) already separates `del(...)?` from a `?` written
    // inside the path.
    let paths = match resolve_dynamic_indexes::<S>(path_expr, &result) {
        Ok(paths) => paths,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    let last = paths.len().saturating_sub(1);
    for (i, path) in paths.iter().enumerate() {
        // Only the final application needs to own `new_value`.
        let value = if i == last {
            core::mem::replace(&mut new_value, OwnedValue::Null)
        } else {
            new_value.clone()
        };
        if let Err(e) = set_path(&mut result, path, value) {
            return if optional {
                QueryResult::None
            } else {
                QueryResult::Error(e)
            };
        }
    }

    QueryResult::Owned(result)
}

/// Evaluate update assignment: `.path |= filter`
/// Applies filter to the value at path and updates it.
fn eval_update<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    path_expr: &Expr,
    filter_expr: &Expr,
    input: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Convert input to owned for modification
    let mut result = to_owned(&input);

    // Computed keys resolve against the original document, before any update.
    //
    // `optional` is `|=`'s *own* `?` (`(.a |= f)?`) -- see `eval_assign`'s
    // matching comment for why it is caught here, at the call boundary,
    // rather than threaded into `update_path` as a starting flag: doing that
    // would let an outer `?` swallow a genuinely-raised `.[-5]? |= 9` (an
    // inline path `?` never covers the write-time bounds check, #498) while
    // an outer `(.[-5] |= 9)?` needs exactly that swallowed. `update_path` is
    // always entered with `false` below; any `?` it still sees came from an
    // `Expr::Optional` node inside `path_expr` itself.
    let paths = match resolve_dynamic_indexes::<S>(path_expr, &result) {
        Ok(paths) => paths,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Get current value at path, apply filter, and set back
    for path in &paths {
        if let Err(e) = update_path::<S>(&mut result, path, filter_expr, false) {
            return if optional {
                QueryResult::None
            } else {
                QueryResult::Error(e)
            };
        }
    }

    QueryResult::Owned(result)
}

/// Evaluate compound assignment: `.path += value`, `.path -= value`, etc.
fn eval_compound_assign<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    op: AssignOp,
    path_expr: &Expr,
    value_expr: &Expr,
    input: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Convert to update: .path op= value  becomes  .path |= . op value
    let arith_op = match op {
        AssignOp::Add => ArithOp::Add,
        AssignOp::Sub => ArithOp::Sub,
        AssignOp::Mul => ArithOp::Mul,
        AssignOp::Div => ArithOp::Div,
        AssignOp::Mod => ArithOp::Mod,
    };

    // jq evaluates the RHS of `a op= b` once against the original input `.`,
    // not against the sub-value at `a` (confirmed against real jq: `(.a,.b)
    // += .a` on `{"a":1,"b":2}` yields `{"a":2,"b":3}`, so `.b`'s `+=` sees
    // the pristine `.a`, not the value `.a` was just updated to). Evaluate it
    // up front and splice in the resulting value rather than the raw
    // expression, so `update_path`'s per-path `Identity` no longer resolves
    // `.` inside it to the sub-value being replaced.
    let rhs_value = match eval_rhs_once::<W, S>(value_expr, input.clone(), optional) {
        Ok(v) => v,
        Err(early_return) => return early_return,
    };

    let filter = Expr::Arithmetic {
        op: arith_op,
        left: Box::new(Expr::Identity),
        right: Box::new(owned_to_expr(&rhs_value)),
    };

    eval_update::<W, S>(path_expr, &filter, input, optional)
}

/// Evaluate alternative assignment: `.path //= value`
/// Sets path to value only if current value is null or false.
fn eval_alternative_assign<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    path_expr: &Expr,
    value_expr: &Expr,
    input: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Same root-vs-sub-value fix as eval_compound_assign: evaluate `value_expr`
    // once against the original input before splicing it into the filter.
    let rhs_value = match eval_rhs_once::<W, S>(value_expr, input.clone(), optional) {
        Ok(v) => v,
        Err(early_return) => return early_return,
    };

    // Convert to update: .path //= value  becomes  .path |= . // value
    let filter = Expr::Alternative(
        Box::new(Expr::Identity),
        Box::new(owned_to_expr(&rhs_value)),
    );

    eval_update::<W, S>(path_expr, &filter, input, optional)
}

/// Turn `root` into a fresh empty object if it is `Null`, otherwise leave it
/// untouched.
///
/// The one auto-vivification jq performs when a write walks through a missing
/// or explicitly-`null` intermediate — mirrors `set_value_at_path`'s
/// `OwnedValue::Null` arm, adapted to mutate a `&mut OwnedValue` in place
/// instead of returning a fresh owned value. Any other non-`null` value is
/// left alone, so indexing it still refuses exactly as before: `null` is the
/// only value auto-vivified, matching `set_value_at_path`'s own rule.
fn autovivify_object(root: &mut OwnedValue) {
    if matches!(root, OwnedValue::Null) {
        *root = OwnedValue::Object(IndexMap::new());
    }
}

/// Sibling of [`autovivify_object`] for an `Index` step.
fn autovivify_array(root: &mut OwnedValue) {
    if matches!(root, OwnedValue::Null) {
        *root = OwnedValue::Array(Vec::new());
    }
}

/// Resolve `idx` against `arr`, padding with `null`s if it names a position
/// past the end, and return that slot.
///
/// Mirrors `set_value_at_path`'s numeric-key arm, and reuses
/// `resolve_setpath_index` (already shared with `setpath()`) for the index
/// math, so there is exactly one place that decides what a numeric write
/// index resolves to. A still-negative index (after counting back from the
/// end) is jq's `Out of bounds negative array index` — the one write-time
/// check `?` does not suppress — so this is never gated by an
/// `optional`/`here` flag at any call site; only `set_path`'s
/// `Expr::Optional` arm ever needs to tell it apart from a suppressible
/// error, via [`EvalError::is_negative_index_out_of_bounds`] (#498).
fn write_index(arr: &mut Vec<OwnedValue>, idx: i64) -> Result<&mut OwnedValue, EvalError> {
    let actual_idx = resolve_setpath_index(&OwnedValue::Int(idx), arr.len())?;
    if actual_idx >= arr.len() {
        pad_with_nulls(arr, actual_idx)?;
    }
    Ok(&mut arr[actual_idx])
}

/// Set a value at a path in an owned value.
fn set_path(
    root: &mut OwnedValue,
    path_expr: &Expr,
    new_value: OwnedValue,
) -> Result<(), EvalError> {
    match path_expr {
        Expr::Identity => {
            *root = new_value;
            Ok(())
        }
        Expr::Field(name) => {
            autovivify_object(root);
            if let OwnedValue::Object(map) = root {
                map.insert(name.clone(), new_value);
                Ok(())
            } else {
                Err(EvalError::cannot_index_with_field(
                    owned_type_name(root),
                    name,
                ))
            }
        }
        Expr::Index(idx) => {
            autovivify_array(root);
            if let OwnedValue::Array(arr) = root {
                *write_index(arr, *idx)? = new_value;
                Ok(())
            } else {
                Err(EvalError::cannot_index_with_type(
                    owned_type_name(root),
                    "number",
                ))
            }
        }
        Expr::Pipe(exprs) if !exprs.is_empty() => {
            // For chained paths like .a.b.c, navigate to parent and set at last element
            if exprs.len() == 1 {
                set_path(root, &exprs[0], new_value)
            } else if let Some(split) = split_at_slice(exprs) {
                match get_path_mut(root, split.before)? {
                    None => Ok(()),
                    Some(parent) => through_slice(parent, split.start, split.end, false, |sub| {
                        set_path(sub, &split.tail, new_value)
                    }),
                }
            } else {
                // Navigate to parent
                let parent_path = &exprs[..exprs.len() - 1];
                let last_path = &exprs[exprs.len() - 1];

                match get_path_mut(root, parent_path)? {
                    None => Ok(()),
                    Some(parent) => set_path(parent, last_path, new_value),
                }
            }
        }
        Expr::Optional(inner) => match set_path(root, inner, new_value) {
            Ok(()) => Ok(()),
            // jq's `?` suppresses errors raised while *collecting* a path,
            // but not the write-time bounds check on a still-negative array
            // index — the one error left that `Index`'s arm above can still
            // raise now that a positive overrun pads instead of erroring
            // (#498).
            Err(e) if e.is_negative_index_out_of_bounds() => Err(e),
            Err(_) => Ok(()), // Silently succeed for optional
        },
        Expr::Iterate => match root {
            OwnedValue::Array(arr) => {
                for elem in arr.iter_mut() {
                    *elem = new_value.clone();
                }
                Ok(())
            }
            OwnedValue::Object(map) => {
                for (_, elem) in map.iter_mut() {
                    *elem = new_value.clone();
                }
                Ok(())
            }
            _ => Err(EvalError::cannot_iterate(root)),
        },
        // `.[a:b] = v` splices `v`'s elements over the range, so `v` has to be
        // an array — that is the whole reason jq has a separate sentence for
        // it. An out-of-range range clamps rather than erroring, unlike the
        // `Expr::Index` arm above: `[1,2,3] | .[5:9] = ["x"]` appends.
        Expr::Slice { start, end } => through_slice(root, *start, *end, false, |sub| {
            *sub = new_value;
            Ok(())
        }),
        // Unreachable: `resolve_dynamic_indexes` rewrites every computed key
        // into a static component before this runs. Explicit rather than left
        // to the catch-all so a missed install point fails loudly here instead
        // of being reported as a user error.
        Expr::IndexExpr { .. } => Err(EvalError::new(
            "internal error: unresolved computed index in assignment path",
        )),
        _ => Err(EvalError::new(
            "cannot use expression as assignment target".to_string(),
        )),
    }
}

/// Strip the wrappers a *resolved* path component can still carry, and say
/// whether a `?` was among them.
///
/// `resolve_node` rewrites a computed key into the static component it denotes,
/// but a branch resolved under `?` keeps an `Expr::Optional` around it —
/// `path(.a[]? | .[.k]?)` resolves to `Optional(Field("x")) |
/// Optional(Field("v"))`, not `Field("x") | Field("v")`. The wrapper carries
/// one bit ("a failure here prunes rather than propagates"); the component
/// underneath is what the walk actually follows.
///
/// [`walk_path`] looks through these already, which is why
/// `path(…)` read correctly while `=`, `|=` and `del(…)` did not: their walkers
/// matched the wrapper against `Field`/`Index`/`Iterate`, missed, and fell to a
/// catch-all — `get_path_mut` to "invalid path component", and the two `Pipe`
/// arms to one that silently *drops the rest of the path* and acts at the
/// wrapper's own position, deleting or overwriting the parent of the intended
/// target. [`flatten_delete_path`] got this right; the rest now agree with it.
fn unwrap_path_component(expr: &Expr) -> (&Expr, bool) {
    let mut current = expr;
    let mut optional = false;
    loop {
        match current {
            Expr::Optional(inner) => {
                optional = true;
                current = inner;
            }
            Expr::Paren(inner) => current = inner,
            other => return (other, optional),
        }
    }
}

/// Run `edit` over the sub-array a slice names, splicing the answer back over
/// the original range.
///
/// The shared shape behind every write through `.[a:b]`, whether it is the
/// last component (`.[1:2] = v`, `.[1:2] |= f`) or one the path continues
/// through (`.[1:3][] = 9`). The edit sees the slice as an array of its own,
/// and whatever it leaves has to still be an array — splicing back is element
/// by element, which is exactly what jq's `A slice of an array can only be
/// assigned another array` is about.
fn through_slice(
    root: &mut OwnedValue,
    start: Option<i64>,
    end: Option<i64>,
    optional: bool,
    edit: impl FnOnce(&mut OwnedValue) -> Result<(), EvalError>,
) -> Result<(), EvalError> {
    match root {
        OwnedValue::Array(arr) => {
            let range = SliceBounds::from_literals(start, end).resolve(arr.len());
            let mut sub = OwnedValue::Array(arr[range.clone()].to_vec());
            edit(&mut sub)?;
            let OwnedValue::Array(items) = sub else {
                return Err(EvalError::slice_assign_non_array());
            };
            arr.splice(range, items);
            Ok(())
        }
        // jq reads a string slice but will not write one back, whatever the
        // replacement — the refusal beats the non-array one above.
        OwnedValue::String(_) => Err(EvalError::cannot_update_string_slices()),
        _ if optional => Ok(()),
        other => Err(EvalError::cannot_index_with_type(
            owned_type_name(other),
            "object",
        )),
    }
}

/// Where a chained path crosses a slice, as [`split_at_slice`] found it.
struct SliceSplit<'a> {
    /// The components to navigate before reaching the slice.
    before: &'a [Expr],
    start: Option<i64>,
    end: Option<i64>,
    /// Everything left to apply *inside* the slice, as one expression.
    tail: Expr,
}

/// Split a chained path at its first slice, if it has one.
///
/// A slice names a *range*, not a slot, so `get_path_mut` cannot navigate
/// through one — `.a[1:2][0] = 9` would fail on the middle component — and the
/// walker has to hand off to [`through_slice`] there instead.
fn split_at_slice(exprs: &[Expr]) -> Option<SliceSplit<'_>> {
    let at = exprs.iter().position(|e| matches!(e, Expr::Slice { .. }))?;
    let Expr::Slice { start, end } = &exprs[at] else {
        unreachable!("position matched a Slice")
    };
    let rest = &exprs[at + 1..];
    Some(SliceSplit {
        before: &exprs[..at],
        start: *start,
        end: *end,
        // An empty tail means the slice itself is the target, which `Identity`
        // against the extracted sub-array says exactly.
        tail: if rest.is_empty() {
            Expr::Identity
        } else {
            Expr::Pipe(rest.to_vec())
        },
    })
}

/// Get a mutable reference to the parent named by `path_parts`, or `None` if
/// a component along the way could not be walked and was itself marked
/// optional (`?`).
///
/// Autovivification means a *wrong-type* mismatch — the only failure this can
/// still raise — can only be reached through data the document already held:
/// `Null` always vivifies into whatever the next step needs, so nothing this
/// function creates can itself go on to fail. That is what makes per-step `?`
/// safe to honour here without any risk of leaving a half-built container
/// behind.
///
/// Each component's own `?` is scoped to that component only, mirroring
/// `update_path`'s `Pipe`-chain arm: `{"a":5} | .a?.b.c = 1` still raises on
/// `.c` (unprotected) even though `.a?` (protected, but never triggered here
/// since `.a` reads fine) precedes it. This used to drop the bit entirely —
/// "every path reaching a walker has already been resolved" is only true of
/// the *computed-key* pre-pass (`resolve_dynamic_indexes`/`resolve_node`); a
/// plain static chain like `.a?.b = 1` never goes through it at all
/// (`needs_path_prepass` says no), so `get_path_mut` was the last word on
/// whether `?` applied and was ignoring it: `"str" | .a?.b = 1` raised
/// `Cannot index string with string "a"` instead of leaving `"str"`
/// untouched, matching jq.
fn get_path_mut<'a>(
    root: &'a mut OwnedValue,
    path_parts: &[Expr],
) -> Result<Option<&'a mut OwnedValue>, EvalError> {
    let mut current = root;

    for part in path_parts {
        let (part, optional) = unwrap_path_component(part);
        current = match part {
            Expr::Identity => current,
            Expr::Field(name) => {
                autovivify_object(current);
                if let OwnedValue::Object(map) = current {
                    map.entry(name.clone()).or_insert(OwnedValue::Null)
                } else if optional {
                    return Ok(None);
                } else {
                    return Err(EvalError::cannot_index_with_field(
                        owned_type_name(current),
                        name,
                    ));
                }
            }
            Expr::Index(idx) => {
                autovivify_array(current);
                if let OwnedValue::Array(arr) = current {
                    // A still-negative index is the write-time bounds check,
                    // not a walking failure, so `?` never covers it here
                    // either — same reasoning as `set_path`'s `Expr::Optional`
                    // arm, just with nothing to catch: `write_index` never
                    // returns that error for a component this function can
                    // itself elect to suppress.
                    write_index(arr, *idx)?
                } else if optional {
                    return Ok(None);
                } else {
                    return Err(EvalError::cannot_index_with_type(
                        owned_type_name(current),
                        "number",
                    ));
                }
            }
            Expr::IndexExpr { .. } => {
                return Err(EvalError::new(
                    "internal error: unresolved computed index in path component",
                ))
            }
            _ => return Err(EvalError::new("invalid path component")),
        };
    }

    Ok(Some(current))
}

/// Update a value at a path by applying a filter.
fn update_path<S: EvalSemantics>(
    root: &mut OwnedValue,
    path_expr: &Expr,
    filter_expr: &Expr,
    optional: bool,
) -> Result<(), EvalError> {
    match path_expr {
        Expr::Identity => {
            // The filter runs as an ordinary sub-expression, not one
            // implicitly wrapped in `try/catch` by whatever `?` got us here:
            // `optional` at this point only ever came from an inline path
            // component (`.a?`), which prunes path *production*, never
            // covers path *application* (#498) -- `{"a":1} | .a? |=
            // error("boom")` raises `boom` in jq, it does not fall back to
            // leaving `.a` untouched. A `?` wrapping the *whole* `.path |=
            // filter` expression is handled by `eval_update`'s caller
            // instead, which is why `update_path` is always entered with
            // `optional: false` at the top.
            let v = eval_owned_expr::<S>(filter_expr, root, false)?;
            *root = v;
            Ok(())
        }
        Expr::Field(name) => {
            autovivify_object(root);
            if let OwnedValue::Object(map) = root {
                let current = map.entry(name.clone()).or_insert(OwnedValue::Null);
                update_path::<S>(current, &Expr::Identity, filter_expr, optional)
            } else if optional {
                Ok(())
            } else {
                Err(EvalError::cannot_index_with_field(
                    owned_type_name(root),
                    name,
                ))
            }
        }
        Expr::Index(idx) => {
            autovivify_array(root);
            if let OwnedValue::Array(arr) = root {
                update_path::<S>(
                    write_index(arr, *idx)?,
                    &Expr::Identity,
                    filter_expr,
                    optional,
                )
            } else if optional {
                Ok(())
            } else {
                Err(EvalError::cannot_index_with_type(
                    owned_type_name(root),
                    "number",
                ))
            }
        }
        Expr::Iterate => {
            // Update all elements
            match root {
                OwnedValue::Array(arr) => {
                    for elem in arr.iter_mut() {
                        update_path::<S>(elem, &Expr::Identity, filter_expr, optional)?;
                    }
                    Ok(())
                }
                OwnedValue::Object(map) => {
                    for value in map.values_mut() {
                        update_path::<S>(value, &Expr::Identity, filter_expr, optional)?;
                    }
                    Ok(())
                }
                _ if optional => Ok(()),
                _ => Err(EvalError::cannot_iterate(root)),
            }
        }
        Expr::Pipe(exprs) if !exprs.is_empty() => {
            // Chain: navigate and update
            if exprs.len() == 1 {
                update_path::<S>(root, &exprs[0], filter_expr, optional)
            } else {
                // Navigate to the penultimate path, then update the last.
                // `here` is whether *this step* may fail quietly; `optional`
                // keeps travelling to `rest`, which carries its own wrappers.
                let (first, first_optional) = unwrap_path_component(&exprs[0]);
                let here = optional || first_optional;
                let rest = Expr::Pipe(exprs[1..].to_vec());

                match first {
                    Expr::Field(name) => {
                        autovivify_object(root);
                        if let OwnedValue::Object(map) = root {
                            let current = map.entry(name.clone()).or_insert(OwnedValue::Null);
                            update_path::<S>(current, &rest, filter_expr, optional)
                        } else if here {
                            Ok(())
                        } else {
                            Err(EvalError::cannot_index_with_field(
                                owned_type_name(root),
                                name,
                            ))
                        }
                    }
                    Expr::Index(idx) => {
                        autovivify_array(root);
                        if let OwnedValue::Array(arr) = root {
                            update_path::<S>(write_index(arr, *idx)?, &rest, filter_expr, optional)
                        } else if here {
                            Ok(())
                        } else {
                            Err(EvalError::cannot_index_with_type(
                                owned_type_name(root),
                                "number",
                            ))
                        }
                    }
                    Expr::Iterate => match root {
                        OwnedValue::Array(arr) => {
                            for elem in arr.iter_mut() {
                                update_path::<S>(elem, &rest, filter_expr, optional)?;
                            }
                            Ok(())
                        }
                        OwnedValue::Object(map) => {
                            for value in map.values_mut() {
                                update_path::<S>(value, &rest, filter_expr, optional)?;
                            }
                            Ok(())
                        }
                        _ if here => Ok(()),
                        _ => Err(EvalError::cannot_iterate(root)),
                    },
                    // `resolve_node`'s `?` arm emits `Optional(Pipe([…]))`
                    // when a branch resolved to more than one component, so
                    // unwrapping can expose a nested pipe. Splice it in rather
                    // than recursing on it alone, which would strand `rest`.
                    Expr::Pipe(inner) => {
                        let mut spliced = inner.clone();
                        spliced.extend_from_slice(&exprs[1..]);
                        update_path::<S>(root, &Expr::Pipe(spliced), filter_expr, here)
                    }
                    // The chain continues *inside* the slice, so the update
                    // runs against the sub-array and is spliced back:
                    // `[1,2,3,4] | .[1:3][] |= .*10` is `[1,20,30,4]`.
                    Expr::Slice { start, end } => through_slice(root, *start, *end, here, |sub| {
                        update_path::<S>(sub, &rest, filter_expr, optional)
                    }),
                    _ => update_path::<S>(root, first, filter_expr, here),
                }
            }
        }
        Expr::Optional(inner) => update_path::<S>(root, inner, filter_expr, true),
        // `.[a:b] |= f` runs `f` on the sub-array — not on each element — and
        // splices the answer back, so `[1,2,3] | .[1:2] |= . + ["q"]` is
        // `[1,2,"q",3]`. `f` may return an array of any length, but it has to
        // be an array.
        Expr::Slice { start, end } => through_slice(root, *start, *end, optional, |sub| {
            update_path::<S>(sub, &Expr::Identity, filter_expr, optional)
        }),
        // Unreachable: `resolve_dynamic_indexes` rewrites every computed key
        // into a static component before this runs. Explicit rather than left
        // to the catch-all so a missed install point fails loudly here instead
        // of being reported as a user error.
        Expr::IndexExpr { .. } => Err(EvalError::new(
            "internal error: unresolved computed index in update path",
        )),
        _ => Err(EvalError::new("cannot use expression as update target")),
    }
}

/// Get the type name for an owned value.
fn owned_type_name(value: &OwnedValue) -> &'static str {
    match value {
        OwnedValue::Null => "null",
        OwnedValue::Bool(_) => "boolean",
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => "number",
        OwnedValue::String(_) => "string",
        OwnedValue::Array(_) => "array",
        OwnedValue::Object(_) => "object",
    }
}

// =============================================================================
// Computed keys in path expressions (#360)
// =============================================================================
//
// `set_path`, `get_path_mut`, `update_path`, `delete_at_path` and `walk_path`
// all understand only *static* path components: `Identity`, `Field`, `Index`,
// `Iterate`, `Slice`. Rather than teach every walker about computed keys, the
// key is resolved to the concrete component it denotes *before* they run, so
// they keep seeing exactly the shapes they always have.
//
// A pre-pass (rather than recursion inside `set_path`) is also what makes
// multi-key assignment match jq: keys are computed against the *original*
// document, so `{"a":"x","x":"y"} | .[.a, .x] = 1` yields
// `{"a":"x","x":1,"y":1}` — the second key is `"y"`, read from the original
// `.x`, not the `1` the first assignment just wrote.

/// Does this expression need the multi-path pre-pass — a computed-key index
/// anywhere in its *path* structure, or a `Comma` naming more than one path
/// outright?
///
/// A `Comma` needs the pre-pass even when every branch is static (`.a, .b`):
/// none of the single-path walkers (`get_path_mut`, `set_path`, `update_path`,
/// `delete_at_path`) have a `Comma` arm, so one handed through verbatim is
/// rejected as an invalid path component. Only the shapes that can appear as a
/// path expression are traversed; a computed key nested inside, say, an `if`
/// is not a path component and cannot reach the walkers. Used as a cheap guard
/// so programs with neither shape skip the pre-pass entirely.
fn needs_path_prepass(expr: &Expr) -> bool {
    match expr {
        Expr::IndexExpr { .. } | Expr::Comma(_) => true,
        Expr::Pipe(exprs) => exprs.iter().any(needs_path_prepass),
        Expr::Optional(inner) | Expr::Paren(inner) => needs_path_prepass(inner),
        _ => false,
    }
}

/// Append `expr` to a path component list, splicing nested pipes and dropping
/// `Identity`.
///
/// `get_path_mut` matches a `&[Expr]` element-wise against
/// `Identity | Field | Index` and rejects anything else as "invalid path
/// component", so a nested `Pipe` emitted by the pre-pass would break
/// assignment with a message that reads like user error.
fn push_path_components(out: &mut Vec<Expr>, expr: &Expr) {
    match expr {
        Expr::Identity => {}
        Expr::Pipe(exprs) => {
            for e in exprs {
                push_path_components(out, e);
            }
        }
        Expr::Paren(inner) => push_path_components(out, inner),
        other => out.push(other.clone()),
    }
}

/// Evaluate an expression against an owned value, preserving the whole output
/// stream.
///
/// [`eval_owned_expr`] cannot be used for keys: it collapses a multi-output
/// result into a single `OwnedValue::Array`, which would turn `.[("a","b")]`
/// into one array-valued key. `QueryResult::collect_owned` is likewise unsafe
/// on its own because it silently maps `Error` to an empty vec, so both `Error`
/// and `Break` are intercepted first.
fn eval_owned_multi<S: EvalSemantics>(
    expr: &Expr,
    input: &OwnedValue,
) -> Result<Vec<OwnedValue>, EvalError> {
    match eval_owned_input::<Vec<u64>, S>(expr, input, false) {
        QueryResult::Error(e) => Err(e),
        QueryResult::Break(label) => Err(EvalError::new(format!("break ${label} not in label"))),
        other => Ok(other.collect_owned()),
    }
}

/// Turn a resolved key value into the static path component it denotes.
///
/// NaN is the one numeric key with no component: reading `.[nan]` is null, but
/// there is no element for a write to land on, so an assignment must fail rather
/// than pick one. jq's own wording, and its own choice —
/// `[10,20,30] | .[nan] = 5` is an error there too. (jq's `path(.[nan])` instead
/// yields `[null]`, a path its own `setpath` then rejects; erroring at the
/// source is the coherent half of that.)
///
/// That is only the right complaint where a number addresses an element at all.
/// The key kind is otherwise dispatched before the container is inspected — which
/// is what produces jq's `Cannot index <container> with <key>` rather than a
/// generic type error — but NaN has to consult the container first, or a document
/// that a number cannot index is reported as an array that has no such element:
/// `{"a":1} | .[nan] = 5` is `Cannot index object with number` in jq, the same
/// message `.[0] = 5` gets there, and says nothing about NaN.
fn key_to_path_component(key: &OwnedValue, container: &OwnedValue) -> Result<Expr, EvalError> {
    match key {
        OwnedValue::String(s) => Ok(Expr::Field(s.clone())),
        // Truncation toward zero, as in the value path.
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            // Null belongs with array: a write builds the array the index names,
            // so `null | .[nan] = 5` is the NaN complaint in jq too.
            if !matches!(container, OwnedValue::Array(_) | OwnedValue::Null) {
                return Err(EvalError::cannot_index(owned_type_name(container), key));
            }
            numeric_key_to_index(key)
                .map(Expr::Index)
                .ok_or_else(|| EvalError::new("Cannot set array element at NaN index"))
        }
        _ => Err(EvalError::cannot_index(owned_type_name(container), key)),
    }
}

/// One resolved branch: the static path components reaching it, and the value
/// found there (needed to resolve any computed key further along the chain).
type PathBranch = (Vec<Expr>, OwnedValue);

/// Resolve one path node against one value, yielding a branch per output.
fn resolve_node<S: EvalSemantics>(
    expr: &Expr,
    value: &OwnedValue,
) -> Result<Vec<PathBranch>, EvalError> {
    match expr {
        Expr::Pipe(exprs) => resolve_seq::<S>(exprs, value),
        Expr::Paren(inner) => resolve_node::<S>(inner, value),

        Expr::Comma(exprs) => {
            let mut out = Vec::new();
            for e in exprs {
                out.extend(resolve_node::<S>(e, value)?);
            }
            Ok(out)
        }

        Expr::Optional(inner) => match inner.as_ref() {
            // `E[K]?` only covers a failure to *index* — evaluating `E` or `K`
            // is not covered (see `eval_index_expr`'s doc comment, which the
            // value-position evaluator already honors). The blanket catch below
            // is right for every other `?`-wrapped node, where evaluation and
            // indexing are the same step, but it would also swallow an error
            // raised while computing `K` itself, e.g. `"str" | .[.k]? = 5`
            // (#413). Only the bare shape needs intercepting, and that is jq's
            // own distinction rather than a limit of what parses: `(.[.k])?` is
            // `try .[.k]`, which catches everything inside it including the key,
            // so `"str" | (.[.k])? = 5` is `"str"` there while
            // `"str" | .[.k]? = 5` raises. A parenthesised key therefore *should*
            // reach the blanket arm below. It cannot yet — the postfix `?` takes
            // a path expression and nothing else until #367 — but when it can,
            // this arm still wants to see only the bare shape.
            Expr::IndexExpr { target, key } => resolve_index_expr::<S>(target, key, value, true),

            // Every other `?`-wrapped node (`.foo?`, `.[0]?`, ...): evaluation
            // and indexing are the same step, so any failure anywhere
            // underneath is the failure to index, and `?` covers all of it.
            //
            // The `Expr::Optional` wrapper below marks the branch as one `?`
            // reached, for whatever downstream still wants to know that —
            // but it is *not* an instruction to keep suppressing failures at
            // write time. `resolve_dynamic_indexes` strips every such
            // wrapper from the final component list before handing it to
            // `set_path`/`update_path`/`delete_at_path`, because those
            // functions apply an already-resolved path unconditionally, the
            // same way jq's `setpath` never re-consults the `?` that
            // `path()` already spent (#498's multi-branch case: a sibling
            // write earlier in the same fan-out batch can still clobber the
            // container this branch needs, and that failure must propagate
            // regardless of this branch's own `?`).
            _ => {
                // A failure under `?` prunes the branch instead of propagating.
                let Ok(branches) = resolve_node::<S>(inner, value) else {
                    return Ok(Vec::new());
                };
                Ok(branches
                    .into_iter()
                    .map(|(components, v)| {
                        let inner_path = if components.len() == 1 {
                            components.into_iter().next().expect("len checked")
                        } else {
                            // Unreached *today*, and deliberately not a panic: the
                            // postfix `?` attaches to a single path element until
                            // #367, so `(.a.b)?`, `(..)?` and `recurse?` are parse
                            // errors, and `E[K]?` — which used to arrive here with
                            // its target's components attached — now goes to
                            // `resolve_index_expr`. #367 reopens it on purpose:
                            // `(.a[.k])?` resolves through the `Paren` arm to
                            // `["a","b"]`, two components, and jq writes
                            // `{"a":{"b":5},"k":"b"}` for it. `eval_generic` can
                            // synthesize `Expr::Optional` around any expression
                            // too, so this was never an invariant of the type.
                            Expr::Pipe(components)
                        };
                        (vec![Expr::Optional(Box::new(inner_path))], v)
                    })
                    .collect())
            }
        },

        // `.[]` before a computed key has to be expanded to concrete
        // components, because each element continues with its own key.
        // `[path(.xs[] | .[.k])]` is `[["xs",0,"p"],["xs",1,"q"]]` in jq.
        Expr::Iterate => match value {
            OwnedValue::Array(items) => Ok(items
                .iter()
                .enumerate()
                .map(|(i, v)| (vec![Expr::Index(i as i64)], v.clone()))
                .collect()),
            OwnedValue::Object(map) => Ok(map
                .iter()
                .map(|(k, v)| (vec![Expr::Field(k.clone())], v.clone()))
                .collect()),
            other => Err(EvalError::cannot_iterate(other)),
        },

        Expr::IndexExpr { target, key } => resolve_index_expr::<S>(target, key, value, false),

        // `..` fans out to every node in the tree (pre-order, self before
        // children), so each needs its own Field/Index chain rather than the
        // verbatim `RecursiveDescent` the static-leaf arm below would
        // otherwise store — the very case #412 was filed for.
        Expr::RecursiveDescent => Ok(resolve_recursive_descent(value)),

        // Bare `recurse` *is* `..` — jq defines it as `recurse(.[]?)` and
        // `[recurse]` and `[..]` agree output for output. Sharing `..`'s
        // resolver rather than routing `.[]?` through `resolve_recurse` is
        // both simpler and what keeps the components bare: resolving under a
        // `?` wraps each one in `Expr::Optional`, which the walkers that
        // *write* (`get_path_mut`, `update_path`, `delete_at_path`) then have
        // to unwrap. They do, but there is no reason to make them.
        Expr::Builtin(Builtin::Recurse | Builtin::RecurseDown) => {
            Ok(resolve_recursive_descent(value))
        }
        // The parameterised spellings have no such shortcut: `f` is arbitrary,
        // so the queue has to run.
        Expr::Builtin(Builtin::RecurseF(f)) => resolve_recurse::<S>(f, None, value),
        Expr::Builtin(Builtin::RecurseCond(f, cond)) => resolve_recurse::<S>(f, Some(cond), value),

        // `select(f)` and the typeof filters (`objects`, `arrays`, ...) add no
        // path component of their own — they either pass a branch through
        // unchanged or prune it, exactly like `Optional` above but driven by a
        // predicate instead of an error.
        Expr::Builtin(Builtin::Select(cond)) => {
            if eval_owned_expr::<S>(cond, value, false)?.is_truthy() {
                Ok(vec![(Vec::new(), value.clone())])
            } else {
                Ok(Vec::new())
            }
        }
        Expr::Builtin(
            builtin @ (Builtin::Values
            | Builtin::Nulls
            | Builtin::Booleans
            | Builtin::Numbers
            | Builtin::Strings
            | Builtin::Arrays
            | Builtin::Objects
            | Builtin::Iterables
            | Builtin::Scalars),
        ) => {
            if type_filter_matches(builtin, value) {
                Ok(vec![(Vec::new(), value.clone())])
            } else {
                Ok(Vec::new())
            }
        }

        // A static leaf: keep it verbatim and thread its value through.
        other => {
            let mut components = Vec::new();
            push_path_components(&mut components, other);
            let mut values = eval_owned_multi::<S>(other, value)?;
            match values.len() {
                // No output prunes the branch.
                0 => Ok(Vec::new()),
                1 => Ok(vec![(components, values.pop().expect("len checked"))]),
                // A multi-output component with no path-tracking arm above
                // (an arbitrary function call, `getpath` with a computed
                // argument, ...). jq resolves these via general bytecode path
                // tracking; we do not (#412), so say so in the user's terms
                // rather than the resolver's.
                _ => Err(EvalError::new(
                    "Cannot use a computed index after a multi-output path component",
                )),
            }
        }
    }
}

/// Fan `..` out into one branch per node in `value`'s tree, self before
/// children in the same pre-order `collect_recursive` uses for the value-only
/// `..`, so `path(..)`-derived branches visit values in the same order
/// `[.. ]` would output them.
fn resolve_recursive_descent(value: &OwnedValue) -> Vec<PathBranch> {
    let mut out = Vec::new();
    push_recursive_branches(&[], value, &mut out);
    out
}

fn push_recursive_branches(prefix: &[Expr], value: &OwnedValue, out: &mut Vec<PathBranch>) {
    out.push((prefix.to_vec(), value.clone()));
    match value {
        OwnedValue::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let mut path = prefix.to_vec();
                path.push(Expr::Index(i as i64));
                push_recursive_branches(&path, item, out);
            }
        }
        OwnedValue::Object(map) => {
            for (k, v) in map {
                let mut path = prefix.to_vec();
                path.push(Expr::Field(k.clone()));
                push_recursive_branches(&path, v, out);
            }
        }
        _ => {}
    }
}

/// Fan `recurse(f)` / `recurse(f; cond)` out into one branch per visited
/// node. Follows `builtin_recurse_f`/`builtin_recurse_cond`'s breadth-first
/// queue — including swallowing `f`'s errors, a falsy `cond` pruning rather
/// than propagating, not queueing a null child, and the `MAX_ITEMS` cutoff —
/// but resolves `f` through [`resolve_node`] at each step instead of
/// [`eval_owned_expr`], so every queued value carries the path components that
/// reach it.
///
/// Not queueing a null child is the load-bearing one. `f` is arbitrary and
/// nothing says it makes progress: `recurse(.a?)` over `{"a":null}` reads
/// `null` from `null` forever, and the queue would run to `MAX_ITEMS` with a
/// prefix one component longer each round — quadratic, and measured at 9 GB
/// for an 18-byte document before this guard existed.
///
/// One difference from those two is deliberate. When `f` yields an array they
/// queue its *elements* (`queue.extend(arr)`), an artefact of
/// [`eval_owned_expr`] collapsing a stream into one array — so `[recurse(.a?)]`
/// descends into an array-valued `.a` where jq stops at the array itself.
/// Resolving through [`resolve_node`] keeps the array, which is jq's answer;
/// mirroring the value path here would mean mirroring its bug.
fn resolve_recurse<S: EvalSemantics>(
    f: &Expr,
    cond: Option<&Expr>,
    value: &OwnedValue,
) -> Result<Vec<PathBranch>, EvalError> {
    let mut outputs: Vec<PathBranch> = Vec::new();
    let mut queue: Vec<PathBranch> = vec![(Vec::new(), value.clone())];
    const MAX_ITEMS: usize = 10000;

    while !queue.is_empty() && outputs.len() < MAX_ITEMS {
        let (prefix, current) = queue.remove(0);

        if let Some(cond) = cond {
            let should_continue =
                eval_owned_expr::<S>(cond, &current, false).is_ok_and(|v| v.is_truthy());
            if !should_continue {
                continue;
            }
        }

        outputs.push((prefix.clone(), current.clone()));

        for (child_components, child_value) in resolve_node::<S>(f, &current).unwrap_or_default() {
            // `builtin_recurse_f`'s `Ok(v) if !matches!(v, OwnedValue::Null)`:
            // a null child ends that line of descent. See the note above on
            // why this is what bounds the queue at all.
            if matches!(child_value, OwnedValue::Null) {
                continue;
            }
            let mut path = prefix.clone();
            path.extend(child_components);
            queue.push((path, child_value));
        }
    }

    Ok(outputs)
}

/// Does `builtin` (one of `values`/`nulls`/`booleans`/.../`scalars`) keep
/// `value`? Mirrors the `matches!` checks the value-only evaluator uses for
/// these same builtins.
fn type_filter_matches(builtin: &Builtin, value: &OwnedValue) -> bool {
    match builtin {
        Builtin::Values => !matches!(value, OwnedValue::Null),
        Builtin::Nulls => matches!(value, OwnedValue::Null),
        Builtin::Booleans => matches!(value, OwnedValue::Bool(_)),
        Builtin::Numbers => matches!(
            value,
            OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..)
        ),
        Builtin::Strings => matches!(value, OwnedValue::String(_)),
        Builtin::Arrays => matches!(value, OwnedValue::Array(_)),
        Builtin::Objects => matches!(value, OwnedValue::Object(_)),
        Builtin::Iterables => matches!(value, OwnedValue::Array(_) | OwnedValue::Object(_)),
        Builtin::Scalars => !matches!(value, OwnedValue::Array(_) | OwnedValue::Object(_)),
        _ => false,
    }
}

/// Resolve `E[K]` in path context, with or without a trailing `?`.
///
/// The two spellings differ in one thing only: what happens when the resolved
/// key cannot be applied to the container it reached. `E[K]` propagates that
/// failure; `E[K]?` prunes just that branch, because a failure to *index* is
/// exactly what `?` covers. Everything else is shared, and worth sharing — the
/// two were separate copies until #413, and the copy is where the NaN rule
/// drifted into rejecting a case jq accepts.
///
/// Two rules the shared body carries, both from [`eval_index_expr`]:
///
/// - `K` is evaluated before `E`, and an empty key stream short-circuits before
///   `E` runs at all. jq compiles `E[K]` as `K as $k | E | .[$k]`, so
///   `5 | .a[.k] = 9` blames the `.k` that failed rather than the `.a` it never
///   reached, and `5 | .a[empty] = 9` is `5` rather than an error.
/// - The key stream is outer and the target stream inner, so
///   `path(.[("a","b")])` emits `["a"]` then `["b"]`.
fn resolve_index_expr<S: EvalSemantics>(
    target: &Expr,
    key: &Expr,
    value: &OwnedValue,
    optional: bool,
) -> Result<Vec<PathBranch>, EvalError> {
    let keys = eval_owned_multi::<S>(key, value)?;
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let target_branches = resolve_node::<S>(target, value)?;

    let mut out = Vec::with_capacity(keys.len() * target_branches.len());
    for k in &keys {
        for (components, target_value) in &target_branches {
            // A NaN key names no element for a write to land on, so `?` does not
            // save it — but only where a number addresses an element at all. On
            // an object the failure is the ordinary `Cannot index object with
            // number`, which is precisely the failure to index that `?` covers,
            // so it falls through to `key_to_path_component` below and is pruned
            // or propagated with every other mismatch: `{"a":1} | .[nan]? = 5` is
            // `{"a":1}` in jq while `[1,2,3] | .[nan]? = 5` raises.
            if matches!(
                k,
                OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..)
            ) && numeric_key_to_index(k).is_none()
                && matches!(target_value, OwnedValue::Array(_) | OwnedValue::Null)
            {
                return Err(EvalError::new("Cannot set array element at NaN index"));
            }
            let component = match key_to_path_component(k, target_value) {
                Ok(component) => component,
                Err(_) if optional => continue,
                Err(e) => return Err(e),
            };
            // `false`, not `optional`: the failure has to arrive as an error for
            // the two spellings to be told apart here, rather than as the `None`
            // that the optional form of this call reports it as.
            let next_value = match index_one_owned(target_value, k, false) {
                Ok(v) => v.expect("non-optional index yields a value or errors"),
                Err(_) if optional => continue,
                Err(e) => return Err(e),
            };
            // `resolve_dynamic_indexes` strips the `Expr::Optional` wrapper
            // below from every component before it ever reaches
            // `set_path`/`update_path`/`delete_at_path` (#498) — see the
            // note on `resolve_node`'s matching arm for why a wrapper must
            // not survive into the write.
            let mut path = components.clone();
            path.push(if optional {
                Expr::Optional(Box::new(component))
            } else {
                component
            });
            out.push((path, next_value));
        }
    }
    Ok(out)
}

/// Thread a value through a run of static path components, without expanding
/// them.
///
/// The components stay verbatim in the resolved path; only the value carried
/// alongside them is computed. That value is what a computed key further along
/// the chain is checked against, so skipping the walk does not produce a wrong
/// *path* — components come from the key, never from the container — but it
/// does produce a spurious `Cannot index <wrong type> with <key>`. `.a.b[.k]`
/// with a numeric `.k` looked the key up in the document root, an object, and
/// failed where jq assigns.
///
/// A component with no single output stops the walk at null: null accepts every
/// key kind that can index anything at all, so a later key still resolves
/// instead of erroring against a container it never actually saw.
fn value_after_components<S: EvalSemantics>(
    components: &[Expr],
    value: &OwnedValue,
) -> Result<OwnedValue, EvalError> {
    let mut current = value.clone();
    for component in components {
        let mut values = eval_owned_multi::<S>(component, &current)?;
        match values.len() {
            1 => current = values.pop().expect("len checked"),
            _ => return Ok(OwnedValue::Null),
        }
    }
    Ok(current)
}

/// Resolve a pipe of path nodes, threading the value left to right.
///
/// The threading is the whole point: a computed key sees the value reaching
/// *its* position, which is the document root only when it sits at the top of
/// the path. `path(.x | .a[.k])` resolves `.k` against `.x`, giving
/// `["x","a","a"]`.
fn resolve_seq<S: EvalSemantics>(
    exprs: &[Expr],
    value: &OwnedValue,
) -> Result<Vec<PathBranch>, EvalError> {
    let mut flat = Vec::new();
    for e in exprs {
        push_path_components(&mut flat, e);
    }

    // Everything after the last computed key or Comma keeps its components
    // verbatim — resolving them would expand `.[]` into one path per element
    // for no gain. The *value* still has to be threaded through them, because
    // an enclosing `IndexExpr` indexes it: in `.a[.k].b[.j]`, `.j` applies to
    // `.a[.k].b`.
    let Some(last_dynamic) = flat.iter().rposition(needs_path_prepass) else {
        let end = value_after_components::<S>(&flat, value)?;
        return Ok(vec![(flat, end)]);
    };

    let mut branches: Vec<PathBranch> = vec![(Vec::new(), value.clone())];
    for element in &flat[..=last_dynamic] {
        let mut next = Vec::new();
        for (prefix, current) in &branches {
            for (components, resulting) in resolve_node::<S>(element, current)? {
                let mut path = prefix.clone();
                path.extend(components);
                next.push((path, resulting));
            }
        }
        branches = next;
    }

    let tail = &flat[last_dynamic + 1..];
    for (prefix, current) in &mut branches {
        let end = value_after_components::<S>(tail, current)?;
        *current = end;
        prefix.extend_from_slice(tail);
    }
    Ok(branches)
}

/// Rewrite every computed key in a path expression into the static component it
/// denotes for `input`, and fan a top-level `Comma` out into one path per
/// branch, yielding one fully-static path expression per resolved path.
///
/// jq applies *all* of them: `{"a":0,"b":0} | .[("a","b")] = 1` is
/// `{"a":1,"b":1}`, and `path()` emits one output per resolved path. Likewise
/// for a purely static `Comma` — `{"a":1,"b":2} | del(.a, .b)` is `{}`.
fn resolve_dynamic_indexes<S: EvalSemantics>(
    expr: &Expr,
    input: &OwnedValue,
) -> Result<Vec<Expr>, EvalError> {
    if !needs_path_prepass(expr) {
        return Ok(vec![expr.clone()]);
    }

    let branches = resolve_node::<S>(expr, input)?;
    Ok(branches
        .into_iter()
        .map(|(components, _)| {
            let components: Vec<Expr> = components
                .into_iter()
                .map(strip_resolved_optional)
                .collect();
            match components.len() {
                0 => Expr::Identity,
                1 => components.into_iter().next().expect("len checked"),
                _ => Expr::Pipe(components),
            }
        })
        .collect())
}

/// Drop any `Expr::Optional` wrapper a resolved path component still
/// carries — from `resolve_node`'s bare-`?` arm, `resolve_index_expr`, or
/// verbatim from `resolve_seq`'s no-computed-key fast path, whose static
/// suffix is spliced in unresolved and can still hold a source-level `?`.
///
/// `?` finishes its job during path *production* — this function's caller
/// is the one place every resolved branch passes through on its way to
/// `set_path`/`update_path`/`delete_at_path`, so it is the one place that
/// can guarantee none of them still carry a marker telling those functions
/// to keep suppressing failures at *write* time. jq's own model never asks
/// `setpath` the question at all: `path()` computes the fully static path
/// array first (pruning under `?` there, once), and applies that array
/// unconditionally after — even when an earlier sibling in the same
/// fan-out batch has since clobbered the container this branch needs
/// (#498's multi-branch case, and its purely-static variant reached through
/// `resolve_seq` rather than a computed key).
fn strip_resolved_optional(component: Expr) -> Expr {
    match component {
        Expr::Optional(inner) => strip_resolved_optional(*inner),
        Expr::Pipe(exprs) => Expr::Pipe(exprs.into_iter().map(strip_resolved_optional).collect()),
        other => other,
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
        Expr::Loc { line } => Expr::Loc { line: *line },
        Expr::Env => Expr::Env,
        Expr::Identity => Expr::Identity,
        Expr::Field(name) => Expr::Field(name.clone()),
        Expr::Index(i) => Expr::Index(*i),
        Expr::Slice { start, end } => Expr::Slice {
            start: *start,
            end: *end,
        },
        Expr::Iterate => Expr::Iterate,
        // Must recurse into `key`: variables are resolved by substitution, so
        // skipping it would leave `$k` in `.[$k]` unbound at eval time.
        Expr::IndexExpr { target, key } => Expr::IndexExpr {
            target: Box::new(substitute_var(target, var_name, replacement)),
            key: Box::new(substitute_var(key, var_name, replacement)),
        },
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
        Expr::Format(f) => Expr::Format(f.clone()),
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
        // Assignment operators
        Expr::Assign { path, value } => Expr::Assign {
            path: Box::new(substitute_var(path, var_name, replacement)),
            value: Box::new(substitute_var(value, var_name, replacement)),
        },
        Expr::Update { path, filter } => Expr::Update {
            path: Box::new(substitute_var(path, var_name, replacement)),
            filter: Box::new(substitute_var(filter, var_name, replacement)),
        },
        Expr::CompoundAssign { op, path, value } => Expr::CompoundAssign {
            op: *op,
            path: Box::new(substitute_var(path, var_name, replacement)),
            value: Box::new(substitute_var(value, var_name, replacement)),
        },
        Expr::AlternativeAssign { path, value } => Expr::AlternativeAssign {
            path: Box::new(substitute_var(path, var_name, replacement)),
            value: Box::new(substitute_var(value, var_name, replacement)),
        },

        // Label-break
        Expr::Label { name, body } => {
            // Don't substitute if the label shadows our variable
            if name == var_name {
                expr.clone()
            } else {
                Expr::Label {
                    name: name.clone(),
                    body: Box::new(substitute_var(body, var_name, replacement)),
                }
            }
        }
        Expr::Break(name) => Expr::Break(name.clone()),
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
        Builtin::Values => Builtin::Values,
        Builtin::Nulls => Builtin::Nulls,
        Builtin::Booleans => Builtin::Booleans,
        Builtin::Numbers => Builtin::Numbers,
        Builtin::Strings => Builtin::Strings,
        Builtin::Arrays => Builtin::Arrays,
        Builtin::Objects => Builtin::Objects,
        Builtin::Iterables => Builtin::Iterables,
        Builtin::Scalars => Builtin::Scalars,
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
        Builtin::ToJson => Builtin::ToJson,
        Builtin::FromJson => Builtin::FromJson,
        Builtin::Explode => Builtin::Explode,
        Builtin::Implode => Builtin::Implode,
        Builtin::Test(e) => Builtin::Test(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Indices(e) => Builtin::Indices(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Index(e) => Builtin::Index(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Rindex(e) => Builtin::Rindex(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::ToJsonStream => Builtin::ToJsonStream,
        Builtin::FromJsonStream => Builtin::FromJsonStream,
        Builtin::ToStream => Builtin::ToStream,
        Builtin::FromStream(e) => {
            Builtin::FromStream(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::TruncateStream(e) => {
            Builtin::TruncateStream(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::GetPath(e) => Builtin::GetPath(Box::new(substitute_var(e, var_name, replacement))),
        // Phase 16: Regex Functions
        Builtin::TestFlags(re, flags) => Builtin::TestFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
        ),
        Builtin::Match(re) => Builtin::Match(Box::new(substitute_var(re, var_name, replacement))),
        Builtin::MatchFlags(re, flags) => Builtin::MatchFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
        ),
        Builtin::Capture(re) => {
            Builtin::Capture(Box::new(substitute_var(re, var_name, replacement)))
        }
        Builtin::CaptureFlags(re, flags) => Builtin::CaptureFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
        ),
        Builtin::Sub(re, repl) => Builtin::Sub(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(repl, var_name, replacement)),
        ),
        Builtin::SubFlags(re, repl, flags) => Builtin::SubFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(repl, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
        ),
        Builtin::Gsub(re, repl) => Builtin::Gsub(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(repl, var_name, replacement)),
        ),
        Builtin::GsubFlags(re, repl, flags) => Builtin::GsubFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(repl, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
        ),
        Builtin::Scan(re) => Builtin::Scan(Box::new(substitute_var(re, var_name, replacement))),
        Builtin::ScanFlags(re, flags) => Builtin::ScanFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
        ),
        Builtin::SplitRegex(re, flags) => Builtin::SplitRegex(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
        ),
        Builtin::Splits(re) => Builtin::Splits(Box::new(substitute_var(re, var_name, replacement))),
        Builtin::SplitsFlags(re, flags) => Builtin::SplitsFlags(
            Box::new(substitute_var(re, var_name, replacement)),
            Box::new(substitute_var(flags, var_name, replacement)),
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
        Builtin::PathNoArg => Builtin::PathNoArg,
        Builtin::Parent => Builtin::Parent,
        Builtin::ParentN(e) => Builtin::ParentN(Box::new(substitute_var(e, var_name, replacement))),
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
        Builtin::EnvObject(name) => Builtin::EnvObject(name.clone()),
        Builtin::StrEnv(name) => Builtin::StrEnv(name.clone()),
        Builtin::NullLit => Builtin::NullLit,
        Builtin::Trim => Builtin::Trim,
        Builtin::Ltrim => Builtin::Ltrim,
        Builtin::Rtrim => Builtin::Rtrim,
        Builtin::Transpose => Builtin::Transpose,
        Builtin::BSearch(e) => Builtin::BSearch(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::ModuleMeta(e) => {
            Builtin::ModuleMeta(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Pick(e) => Builtin::Pick(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Omit(e) => Builtin::Omit(Box::new(substitute_var(e, var_name, replacement))),
        Builtin::Tag => Builtin::Tag,
        Builtin::Anchor => Builtin::Anchor,
        Builtin::Style => Builtin::Style,
        Builtin::Kind => Builtin::Kind,
        Builtin::Key => Builtin::Key,
        Builtin::Line => Builtin::Line,
        Builtin::Column => Builtin::Column,
        Builtin::DocumentIndex => Builtin::DocumentIndex,
        Builtin::Shuffle => Builtin::Shuffle,
        Builtin::Pivot => Builtin::Pivot,
        Builtin::SplitDoc => Builtin::SplitDoc,
        Builtin::Del(e) => Builtin::Del(Box::new(substitute_var(e, var_name, replacement))),
        // Phase 12 builtins (no args to substitute)
        Builtin::Now => Builtin::Now,
        Builtin::Abs => Builtin::Abs,
        Builtin::Builtins => Builtin::Builtins,
        Builtin::Normals => Builtin::Normals,
        Builtin::Finites => Builtin::Finites,
        // Phase 13: Iteration control
        Builtin::Limit(n, e) => Builtin::Limit(
            Box::new(substitute_var(n, var_name, replacement)),
            Box::new(substitute_var(e, var_name, replacement)),
        ),
        Builtin::FirstStream(e) => {
            Builtin::FirstStream(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::LastStream(e) => {
            Builtin::LastStream(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::NthStream(n, e) => Builtin::NthStream(
            Box::new(substitute_var(n, var_name, replacement)),
            Box::new(substitute_var(e, var_name, replacement)),
        ),
        Builtin::IsEmpty(e) => Builtin::IsEmpty(Box::new(substitute_var(e, var_name, replacement))),
        // Phase 14: Recursive traversal (extends Phase 8)
        Builtin::RecurseDown => Builtin::RecurseDown,
        // Phase 15: Date/Time functions
        Builtin::Gmtime => Builtin::Gmtime,
        Builtin::Localtime => Builtin::Localtime,
        Builtin::Mktime => Builtin::Mktime,
        Builtin::Strftime(e) => {
            Builtin::Strftime(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Strptime(e) => {
            Builtin::Strptime(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::Todate => Builtin::Todate,
        Builtin::Fromdate => Builtin::Fromdate,
        Builtin::Todateiso8601 => Builtin::Todateiso8601,
        Builtin::Fromdateiso8601 => Builtin::Fromdateiso8601,

        // Phase 17: Combinations
        Builtin::Combinations => Builtin::Combinations,
        Builtin::CombinationsN(e) => {
            Builtin::CombinationsN(Box::new(substitute_var(e, var_name, replacement)))
        }

        // Phase 18: Additional math functions
        Builtin::Trunc => Builtin::Trunc,

        // Phase 19: Type conversion
        Builtin::ToBoolean => Builtin::ToBoolean,

        // Phase 20: Iteration control extension
        Builtin::Skip(n, e) => Builtin::Skip(
            Box::new(substitute_var(n, var_name, replacement)),
            Box::new(substitute_var(e, var_name, replacement)),
        ),

        // Phase 21: Extended Date/Time functions (yq)
        Builtin::FromUnix => Builtin::FromUnix,
        Builtin::ToUnix => Builtin::ToUnix,
        Builtin::Tz(e) => Builtin::Tz(Box::new(substitute_var(e, var_name, replacement))),

        // Phase 22: File operations (yq)
        Builtin::Load(e) => Builtin::Load(Box::new(substitute_var(e, var_name, replacement))),

        // Phase 23: Position-based navigation (succinctly extension)
        Builtin::AtOffset(e) => {
            Builtin::AtOffset(Box::new(substitute_var(e, var_name, replacement)))
        }
        Builtin::AtPosition(line, col) => Builtin::AtPosition(
            Box::new(substitute_var(line, var_name, replacement)),
            Box::new(substitute_var(col, var_name, replacement)),
        ),
    }
}

/// Convert an OwnedValue to an Expr, preserving complex types.
fn owned_to_expr(value: &OwnedValue) -> Expr {
    match value {
        OwnedValue::Null => Expr::Literal(Literal::Null),
        OwnedValue::Bool(b) => Expr::Literal(Literal::Bool(*b)),
        OwnedValue::Int(i) => Expr::Literal(Literal::Int(*i)),
        OwnedValue::Float(f) => Expr::Literal(Literal::Float(*f)),
        // A filter `Literal` has no source-text slot (out of scope -- see
        // #387's plan), so a document-sourced literal degrades to its plain
        // parsed form here, same as it does after arithmetic.
        OwnedValue::NumberLiteral(NumberRepr::Int(i), _) => Expr::Literal(Literal::Int(*i)),
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) => Expr::Literal(Literal::Float(*f)),
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
fn eval_as<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    var: &str,
    body: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the expression to get the value to bind
    let bound_result = eval_single::<W, S>(expr, value.clone(), optional);

    // Get all values from the expression
    let bound_values: Vec<OwnedValue> = match bound_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
    };

    // For each bound value, substitute and evaluate the body
    let mut all_results: Vec<OwnedValue> = Vec::new();

    for bound_val in bound_values {
        let substituted_body = substitute_var(body, var, &bound_val);
        match eval_single::<W, S>(&substituted_body, value.clone(), optional).materialize_cursor() {
            QueryResult::One(v) => all_results.push(to_owned(&v)),
            QueryResult::OneCursor(_) => unreachable!(),
            QueryResult::Many(vs) => all_results.extend(vs.iter().map(to_owned)),
            QueryResult::Owned(v) => all_results.push(v),
            QueryResult::ManyOwned(vs) => all_results.extend(vs),
            QueryResult::None => {}
            QueryResult::Error(e) => return QueryResult::Error(e),
            QueryResult::Break(label) => return QueryResult::Break(label),
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
fn eval_reduce<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    input: &Expr,
    var: &str,
    init: &Expr,
    update: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate input to get the stream of values
    let input_result = eval_single::<W, S>(input, value.clone(), optional);
    let input_values: Vec<OwnedValue> = match input_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => Vec::new(),
        QueryResult::Error(_) if optional => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
    };

    // Evaluate initial accumulator
    let init_result = eval_single::<W, S>(init, value.clone(), optional);
    let mut acc = match result_to_owned(init_result) {
        Ok(v) => v,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // For each input value, update the accumulator
    for input_val in input_values {
        // Substitute $var in update, then evaluate with acc as input
        let substituted = substitute_var(update, var, &input_val);
        // We need to evaluate with acc as the input
        let acc_result = eval_owned_expr::<S>(&substituted, &acc, optional);
        match acc_result {
            Ok(new_acc) => acc = new_acc,
            Err(_) if optional => return QueryResult::None,
            Err(e) => return QueryResult::Error(e),
        }
    }

    QueryResult::Owned(acc)
}

/// Evaluate an expression with an OwnedValue as input.
fn eval_owned_expr<S: EvalSemantics>(
    expr: &Expr,
    input: &OwnedValue,
    optional: bool,
) -> Result<OwnedValue, EvalError> {
    // Create a synthetic JSON from the owned value
    // For simplicity, we'll serialize and reparse
    // This is inefficient but correct
    let json_str = input.to_json();
    let json_bytes = json_str.as_bytes();

    // We need to create a temporary index and cursor
    use crate::json::JsonIndex;
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    match eval_single::<Vec<u64>, S>(expr, cursor.value(), optional).materialize_cursor() {
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
        QueryResult::Break(label) => Err(EvalError::new(format!("break ${label} not in label"))),
    }
}

/// Evaluate an expression with an OwnedValue as input, preserving the full
/// output stream.
///
/// [`eval_owned_expr`] collapses a multi-output result into a single array,
/// which is what `reduce`/`foreach` want but wrong for a filter that is allowed
/// to fan out: `try error("x") catch (., .)` must emit two values, not one
/// two-element array. This variant keeps `Many`/`ManyOwned` intact.
///
/// The returned result borrows nothing from the temporary document — every
/// variant it produces is owned — so it is free to satisfy any caller's `'a`
/// and `W`, the same way [`eval_owned_pipe`] does.
fn eval_owned_input<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    input: &OwnedValue,
    optional: bool,
) -> QueryResult<'a, W> {
    // Serialize and reparse to obtain a document the evaluator can index into.
    // Only reached on the error path, so the round-trip is off the hot path.
    let json_str = input.to_json();
    let json_bytes = json_str.as_bytes();

    use crate::json::JsonIndex;
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    match eval_single::<Vec<u64>, S>(expr, cursor.value(), optional).materialize_cursor() {
        QueryResult::One(v) => QueryResult::Owned(to_owned(&v)),
        QueryResult::OneCursor(_) => unreachable!("materialize_cursor removes OneCursor"),
        QueryResult::Many(vs) => QueryResult::ManyOwned(vs.iter().map(to_owned).collect()),
        QueryResult::Owned(v) => QueryResult::Owned(v),
        QueryResult::ManyOwned(vs) => QueryResult::ManyOwned(vs),
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Evaluate `foreach`: `foreach EXPR as $var (INIT; UPDATE)` or `foreach EXPR as $var (INIT; UPDATE; EXTRACT)`.
fn eval_foreach<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    input: &Expr,
    var: &str,
    init: &Expr,
    update: &Expr,
    extract: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate input to get the stream
    let input_result = eval_single::<W, S>(input, value.clone(), optional);
    let input_values: Vec<OwnedValue> = match input_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => Vec::new(),
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
    };

    // Evaluate initial state
    let init_result = eval_single::<W, S>(init, value.clone(), optional);
    let mut state = match result_to_owned(init_result) {
        Ok(v) => v,
        Err(e) => return QueryResult::Error(e),
    };

    let mut outputs: Vec<OwnedValue> = Vec::new();

    for input_val in input_values {
        // Substitute $var and evaluate update with state as input
        let substituted_update = substitute_var(update, var, &input_val);
        match eval_owned_expr::<S>(&substituted_update, &state, optional) {
            Ok(new_state) => {
                state = new_state;
                // If there's an extract expression, evaluate it
                if let Some(ext) = extract {
                    let substituted_extract = substitute_var(ext, var, &input_val);
                    match eval_owned_expr::<S>(&substituted_extract, &state, optional) {
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
fn eval_limit<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    n_expr: &Expr,
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate n
    let n_result = eval_single::<W, S>(n_expr, value.clone(), optional);
    let n = match result_to_owned(n_result) {
        Ok(OwnedValue::Int(i) | OwnedValue::NumberLiteral(NumberRepr::Int(i), _)) if i >= 0 => {
            i as usize
        }
        Ok(_) => {
            return QueryResult::Error(EvalError::new("limit requires non-negative integer"));
        }
        Err(e) => return QueryResult::Error(e),
    };

    if n == 0 {
        return QueryResult::None;
    }

    // Evaluate expr and take first n
    let result = eval_single::<W, S>(expr, value, optional);
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
fn eval_first_expr<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single::<W, S>(expr, value, optional);
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
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Evaluate `last(expr)` - take last output.
fn eval_last_expr<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single::<W, S>(expr, value, optional);
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
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Evaluate `nth(n; expr)` - take nth output.
fn eval_nth_expr<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    n_expr: &Expr,
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate n
    let n_result = eval_single::<W, S>(n_expr, value.clone(), optional);
    let n = match result_to_owned(n_result) {
        Ok(OwnedValue::Int(i)) if i >= 0 => i as usize,
        Ok(_) => {
            return QueryResult::Error(EvalError::new("nth requires non-negative integer"));
        }
        Err(e) => return QueryResult::Error(e),
    };

    let result = eval_single::<W, S>(expr, value, optional);
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
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Evaluate `until(cond; update)` - apply update until cond is true.
fn eval_until<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    cond: &Expr,
    update: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let mut current = to_owned(&value);
    const MAX_ITERATIONS: usize = 10000;

    for _ in 0..MAX_ITERATIONS {
        // Check condition
        match eval_owned_expr::<S>(cond, &current, optional) {
            Ok(cond_val) => {
                if cond_val.is_truthy() {
                    return QueryResult::Owned(current);
                }
            }
            Err(e) => return QueryResult::Error(e),
        }

        // Apply update
        match eval_owned_expr::<S>(update, &current, optional) {
            Ok(new_val) => current = new_val,
            Err(e) => return QueryResult::Error(e),
        }
    }

    QueryResult::Error(EvalError::new("until: maximum iterations exceeded"))
}

/// Evaluate `while(cond; update)` - output values while cond is true.
fn eval_while<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
        match eval_owned_expr::<S>(cond, &current, optional) {
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
        match eval_owned_expr::<S>(update, &current, optional) {
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

/// Evaluate `repeat(expr)` - repeatedly evaluate expr with the original input.
/// In jq, `repeat(expr)` evaluates `expr` with the original input each time,
/// producing an infinite stream of outputs. This is different from feeding
/// the output back as input.
/// Note: This produces an infinite stream, so it should be used with `limit`.
fn eval_repeat<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    let mut outputs: Vec<OwnedValue> = Vec::new();
    const MAX_ITERATIONS: usize = 1000; // Limit to prevent infinite loops

    for _ in 0..MAX_ITERATIONS {
        // Evaluate expr with the original input each time
        match eval_owned_expr::<S>(expr, &owned, optional) {
            Ok(new_val) => outputs.push(new_val),
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

/// A numeric argument to `range()`: kept as `i64` when exact so all-integer
/// ranges emit `Int` values, promoted to `f64` when any argument is a float.
#[derive(Clone, Copy)]
enum RangeNum {
    Int(i64),
    Float(f64),
}

impl RangeNum {
    fn as_f64(self) -> f64 {
        match self {
            Self::Int(i) => i as f64,
            Self::Float(f) => f,
        }
    }
}

/// Extract a numeric `range()` argument from an evaluated expression result.
fn range_arg<W: Clone + AsRef<[u64]>>(result: QueryResult<'_, W>) -> Result<RangeNum, EvalError> {
    match result {
        QueryResult::Owned(OwnedValue::Int(i)) => Ok(RangeNum::Int(i)),
        QueryResult::Owned(OwnedValue::Float(f)) => Ok(RangeNum::Float(f)),
        QueryResult::Owned(OwnedValue::NumberLiteral(NumberRepr::Int(i), _)) => {
            Ok(RangeNum::Int(i))
        }
        QueryResult::Owned(OwnedValue::NumberLiteral(NumberRepr::Float(f), _)) => {
            Ok(RangeNum::Float(f))
        }
        QueryResult::One(v) => match to_owned(&v) {
            OwnedValue::Int(i) => Ok(RangeNum::Int(i)),
            OwnedValue::Float(f) => Ok(RangeNum::Float(f)),
            OwnedValue::NumberLiteral(NumberRepr::Int(i), _) => Ok(RangeNum::Int(i)),
            OwnedValue::NumberLiteral(NumberRepr::Float(f), _) => Ok(RangeNum::Float(f)),
            _ => Err(EvalError::new("Range bounds must be numeric")),
        },
        QueryResult::Error(e) => Err(e),
        _ => Err(EvalError::new("Range bounds must be numeric")),
    }
}

/// Evaluate `range(n)`, `range(a;b)`, or `range(a;b;step)`.
fn eval_range<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    from: &Expr,
    to: Option<&Expr>,
    step: Option<&Expr>,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let from_val = match range_arg(eval_single::<W, S>(from, value.clone(), optional)) {
        Ok(n) => n,
        Err(e) => return QueryResult::Error(e),
    };

    let to_val = if let Some(to_expr) = to {
        match range_arg(eval_single::<W, S>(to_expr, value.clone(), optional)) {
            Ok(n) => n,
            Err(e) => return QueryResult::Error(e),
        }
    } else {
        // range(n) means range(0; n)
        return match from_val {
            RangeNum::Int(to) => eval_range_values::<W>(0, to, 1),
            RangeNum::Float(to) => eval_range_values_f64::<W>(0.0, to, 1.0),
        };
    };

    let step_val = if let Some(step_expr) = step {
        match range_arg(eval_single::<W, S>(step_expr, value, optional)) {
            Ok(n) => n,
            Err(e) => return QueryResult::Error(e),
        }
    } else {
        RangeNum::Int(1)
    };

    match (from_val, to_val, step_val) {
        (RangeNum::Int(from), RangeNum::Int(to), RangeNum::Int(step)) => {
            eval_range_values::<W>(from, to, step)
        }
        (from, to, step) => eval_range_values_f64::<W>(from.as_f64(), to.as_f64(), step.as_f64()),
    }
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

/// Helper to generate range values over floats.
///
/// Accumulates by repeated addition of `step` (jq semantics), so results carry
/// the same floating-point drift as jq (e.g. `range(0;1;0.3)` ends at
/// 0.8999999999999999). A zero or NaN step yields no values, matching jq.
fn eval_range_values_f64<'a, W: Clone + AsRef<[u64]>>(
    from: f64,
    to: f64,
    step: f64,
) -> QueryResult<'a, W> {
    let mut values: Vec<OwnedValue> = Vec::new();
    const MAX_RANGE: usize = 100000;

    if step > 0.0 {
        let mut i = from;
        while i < to && values.len() < MAX_RANGE {
            values.push(OwnedValue::Float(i));
            i += step;
        }
    } else if step < 0.0 {
        let mut i = from;
        while i > to && values.len() < MAX_RANGE {
            values.push(OwnedValue::Float(i));
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
fn builtin_recurse<W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    // Default recurse is equivalent to recurse(.[]?)
    let f = Expr::Optional(Box::new(Expr::Iterate));
    builtin_recurse_f::<W, S>(&f, value, optional)
}

/// Builtin: recurse(f)
fn builtin_recurse_f<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
        match eval_owned_expr::<S>(f, &current, true) {
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
fn builtin_recurse_cond<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
        let should_continue = match eval_owned_expr::<S>(cond, &current, optional) {
            Ok(v) => v.is_truthy(),
            Err(_) => false,
        };

        if !should_continue {
            continue;
        }

        outputs.push(current.clone());

        // Apply f to get children
        match eval_owned_expr::<S>(f, &current, true) {
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
fn builtin_walk<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);
    match walk_impl::<S>(f, owned, optional) {
        Ok(result) => QueryResult::Owned(result),
        Err(e) => QueryResult::Error(e),
    }
}

/// Implementation of walk - processes children first, then applies f.
fn walk_impl<S: EvalSemantics>(
    f: &Expr,
    value: OwnedValue,
    optional: bool,
) -> Result<OwnedValue, EvalError> {
    // First, recursively process children
    let processed = match value {
        OwnedValue::Array(arr) => {
            let new_arr: Result<Vec<_>, _> = arr
                .into_iter()
                .map(|v| walk_impl::<S>(f, v, optional))
                .collect();
            OwnedValue::Array(new_arr?)
        }
        OwnedValue::Object(obj) => {
            let new_obj: Result<IndexMap<_, _>, _> = obj
                .into_iter()
                .map(|(k, v)| walk_impl::<S>(f, v, optional).map(|nv| (k, nv)))
                .collect();
            OwnedValue::Object(new_obj?)
        }
        other => other,
    };

    // Then apply f to the processed value
    eval_owned_expr::<S>(f, &processed, optional)
}

/// Builtin: isvalid(expr) - check if expr succeeds without errors.
fn builtin_isvalid<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    match eval_single::<W, S>(expr, value, true) {
        QueryResult::Error(_) => QueryResult::Owned(OwnedValue::Bool(false)),
        QueryResult::None => QueryResult::Owned(OwnedValue::Bool(false)),
        _ => QueryResult::Owned(OwnedValue::Bool(true)),
    }
}

// ============================================================================
// Phase 10: Path Expressions, Math, Environment, etc.
// ============================================================================

/// Evaluate a pipe while tracking the traversal path.
/// This enables PathNoArg and Parent to access the path context.
fn eval_pipe_with_path_context<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    exprs: &[Expr],
    value: &OwnedValue,
    current_path: &[OwnedValue],
    optional: bool,
) -> QueryResult<'a, W> {
    // Call the internal version with root value
    eval_pipe_with_path_context_internal::<W, S>(exprs, value, value, current_path, optional)
}

/// Internal helper that also tracks the root value for parent navigation.
fn eval_pipe_with_path_context_internal<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    exprs: &[Expr],
    value: &OwnedValue,
    root: &OwnedValue,
    current_path: &[OwnedValue],
    optional: bool,
) -> QueryResult<'a, W> {
    if exprs.is_empty() {
        return QueryResult::Owned(value.clone());
    }

    let (first, rest) = exprs.split_first().unwrap();

    // Handle PathNoArg - return the current path
    if matches!(first, Expr::Builtin(Builtin::PathNoArg)) {
        let path_result = QueryResult::Owned(OwnedValue::Array(current_path.to_vec()));
        if rest.is_empty() {
            return path_result;
        }
        // Continue with remaining expressions
        if let QueryResult::Owned(v) = path_result {
            return eval_pipe_with_path_context_internal::<W, S>(
                rest,
                &v,
                root,
                current_path,
                optional,
            );
        }
    }

    // Handle Key - return the last element of current path (current key)
    if matches!(first, Expr::Builtin(Builtin::Key)) {
        let key_result = if current_path.is_empty() {
            // At root - return null (yq behavior)
            QueryResult::Owned(OwnedValue::Null)
        } else {
            // Get the last element of the path (the current key)
            QueryResult::Owned(current_path.last().unwrap().clone())
        };
        if rest.is_empty() {
            return key_result;
        }
        // Continue with remaining expressions
        if let QueryResult::Owned(v) = key_result {
            return eval_pipe_with_path_context_internal::<W, S>(
                rest,
                &v,
                root,
                current_path,
                optional,
            );
        }
    }

    // Handle Parent - return the parent value
    if matches!(first, Expr::Builtin(Builtin::Parent)) {
        let parent_result = if current_path.is_empty() {
            // At root - return empty object (yq behavior)
            QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
        } else {
            // Get the parent path (all but last element)
            let parent_path = &current_path[..current_path.len() - 1];
            // Navigate from root to parent
            match get_value_at_owned_path(root, parent_path) {
                Some(parent_value) => QueryResult::Owned(parent_value),
                None => QueryResult::Owned(OwnedValue::Object(IndexMap::new())),
            }
        };
        if rest.is_empty() {
            return parent_result;
        }
        if let QueryResult::Owned(v) = parent_result {
            // Parent path is one level up
            let parent_path = if current_path.is_empty() {
                vec![]
            } else {
                current_path[..current_path.len() - 1].to_vec()
            };
            return eval_pipe_with_path_context_internal::<W, S>(
                rest,
                &v,
                root,
                &parent_path,
                optional,
            );
        }
    }

    // Handle ParentN - return the nth parent value
    if let Expr::Builtin(Builtin::ParentN(n_expr)) = first {
        // Evaluate n
        let n = match eval_owned_expr::<S>(n_expr, value, optional) {
            Ok(OwnedValue::Int(i) | OwnedValue::NumberLiteral(NumberRepr::Int(i), _)) => i as usize,
            Ok(OwnedValue::Float(f) | OwnedValue::NumberLiteral(NumberRepr::Float(f), _)) => {
                f as usize
            }
            Ok(_) if optional => return QueryResult::None,
            Ok(_) => return QueryResult::Error(EvalError::type_error("number", "other")),
            Err(_) if optional => return QueryResult::None,
            Err(e) => return QueryResult::Error(e),
        };

        // Calculate parent path (n levels up)
        let parent_path = if n >= current_path.len() {
            vec![]
        } else {
            current_path[..current_path.len() - n].to_vec()
        };

        let parent_result = if parent_path.is_empty() && n > 0 {
            // Gone past root - return empty object
            QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
        } else {
            match get_value_at_owned_path(root, &parent_path) {
                Some(parent_value) => QueryResult::Owned(parent_value),
                None => QueryResult::Owned(OwnedValue::Object(IndexMap::new())),
            }
        };

        if rest.is_empty() {
            return parent_result;
        }
        if let QueryResult::Owned(v) = parent_result {
            return eval_pipe_with_path_context_internal::<W, S>(
                rest,
                &v,
                root,
                &parent_path,
                optional,
            );
        }
    }

    // Evaluate first expression and update path
    match first {
        Expr::Identity => {
            // Identity doesn't change the path
            eval_pipe_with_path_context_internal::<W, S>(rest, value, root, current_path, optional)
        }
        Expr::Field(name) => {
            // Extend path with field name
            let mut new_path = current_path.to_vec();
            new_path.push(OwnedValue::String(name.clone()));

            // Get the field value
            if let OwnedValue::Object(entries) = value {
                if let Some(v) = entries.get(name) {
                    if rest.is_empty() {
                        return QueryResult::Owned(v.clone());
                    }
                    return eval_pipe_with_path_context_internal::<W, S>(
                        rest, v, root, &new_path, optional,
                    );
                }
                // jq returns null for missing fields on objects (not an error)
                return QueryResult::Owned(OwnedValue::Null);
            }
            // jq returns null for field access on null
            if matches!(value, OwnedValue::Null) {
                return QueryResult::Owned(OwnedValue::Null);
            }
            // Non-object/null: error (or None if optional)
            if optional {
                QueryResult::None
            } else {
                QueryResult::Error(EvalError::cannot_index_with_field(
                    owned_type_name(value),
                    name,
                ))
            }
        }
        Expr::Index(idx) => {
            // Extend path with index
            let mut new_path = current_path.to_vec();
            new_path.push(OwnedValue::Int(*idx));

            // Get the element value
            if let OwnedValue::Array(arr) = value {
                let len = arr.len() as i64;
                let actual_idx = if *idx < 0 { len + *idx } else { *idx };
                if actual_idx >= 0 && (actual_idx as usize) < arr.len() {
                    let v = &arr[actual_idx as usize];
                    if rest.is_empty() {
                        return QueryResult::Owned(v.clone());
                    }
                    return eval_pipe_with_path_context_internal::<W, S>(
                        rest, v, root, &new_path, optional,
                    );
                }
            }
            if optional {
                QueryResult::None
            } else if let OwnedValue::Array(arr) = value {
                QueryResult::Error(EvalError::index_out_of_bounds(*idx, arr.len()))
            } else {
                // Indexing a non-array is not an out-of-bounds access; jq
                // reports it as an indexing type error like `.[0]` does.
                QueryResult::Error(EvalError::cannot_index_with_type(
                    owned_type_name(value),
                    "number",
                ))
            }
        }
        Expr::Iterate => {
            // Iterate produces multiple paths
            let mut results = Vec::new();
            match value {
                OwnedValue::Array(arr) => {
                    for (i, v) in arr.iter().enumerate() {
                        let mut new_path = current_path.to_vec();
                        new_path.push(OwnedValue::Int(i as i64));
                        if rest.is_empty() {
                            results.push(v.clone());
                        } else {
                            match eval_pipe_with_path_context_internal::<W, S>(
                                rest, v, root, &new_path, optional,
                            ) {
                                QueryResult::Owned(r) => results.push(r),
                                QueryResult::ManyOwned(rs) => results.extend(rs),
                                QueryResult::None => {}
                                QueryResult::Error(e) => return QueryResult::Error(e),
                                _ => {}
                            }
                        }
                    }
                }
                OwnedValue::Object(entries) => {
                    for (key, v) in entries {
                        let mut new_path = current_path.to_vec();
                        new_path.push(OwnedValue::String(key.clone()));
                        if rest.is_empty() {
                            results.push(v.clone());
                        } else {
                            match eval_pipe_with_path_context_internal::<W, S>(
                                rest, v, root, &new_path, optional,
                            ) {
                                QueryResult::Owned(r) => results.push(r),
                                QueryResult::ManyOwned(rs) => results.extend(rs),
                                QueryResult::None => {}
                                QueryResult::Error(e) => return QueryResult::Error(e),
                                _ => {}
                            }
                        }
                    }
                }
                _ if optional => return QueryResult::None,
                _ => return QueryResult::Error(EvalError::cannot_iterate(value)),
            }
            if results.is_empty() {
                QueryResult::None
            } else if results.len() == 1 {
                QueryResult::Owned(results.pop().unwrap())
            } else {
                QueryResult::ManyOwned(results)
            }
        }
        Expr::Paren(inner) => {
            // Parentheses don't change path, just evaluate inner
            if rest.is_empty() {
                eval_pipe_with_path_context_internal::<W, S>(
                    &[(**inner).clone()],
                    value,
                    root,
                    current_path,
                    optional,
                )
            } else {
                let mut combined = vec![(**inner).clone()];
                combined.extend(rest.iter().cloned());
                eval_pipe_with_path_context_internal::<W, S>(
                    &combined,
                    value,
                    root,
                    current_path,
                    optional,
                )
            }
        }
        Expr::Optional(inner) => {
            // Optional - evaluate with optional=true
            if rest.is_empty() {
                eval_pipe_with_path_context_internal::<W, S>(
                    &[(**inner).clone()],
                    value,
                    root,
                    current_path,
                    true,
                )
            } else {
                let mut combined = vec![(**inner).clone()];
                combined.extend(rest.iter().cloned());
                eval_pipe_with_path_context_internal::<W, S>(
                    &combined,
                    value,
                    root,
                    current_path,
                    true,
                )
            }
        }
        Expr::Pipe(inner_exprs) => {
            // Flatten nested pipe - combine inner pipe with rest
            let mut combined = inner_exprs.clone();
            combined.extend(rest.iter().cloned());
            eval_pipe_with_path_context_internal::<W, S>(
                &combined,
                value,
                root,
                current_path,
                optional,
            )
        }
        Expr::Builtin(builtin) => {
            // Handle other builtins that don't need special path handling
            match eval_builtin_owned::<S>(builtin, value, optional) {
                Ok(result) => {
                    if rest.is_empty() {
                        QueryResult::Owned(result)
                    } else {
                        eval_pipe_with_path_context_internal::<W, S>(
                            rest,
                            &result,
                            root,
                            current_path,
                            optional,
                        )
                    }
                }
                Err(_) if optional => QueryResult::None,
                Err(e) => QueryResult::Error(e),
            }
        }
        Expr::Object(_) | Expr::Array(_) | Expr::Literal(_) => {
            // Value-constructing expressions reset the path context
            // because we're now at the "root" of a newly constructed value
            match eval_owned_expr::<S>(first, value, optional) {
                Ok(result) => {
                    if rest.is_empty() {
                        QueryResult::Owned(result)
                    } else {
                        // Reset path and root to the new value
                        eval_pipe_with_path_context_internal::<W, S>(
                            rest,
                            &result,
                            &result,
                            &[],
                            optional,
                        )
                    }
                }
                Err(_) if optional => QueryResult::None,
                Err(e) => QueryResult::Error(e),
            }
        }
        _ => {
            // For other expressions, evaluate normally and continue
            // Note: This loses path context for complex expressions
            match eval_owned_expr::<S>(first, value, optional) {
                Ok(result) => {
                    if rest.is_empty() {
                        QueryResult::Owned(result)
                    } else {
                        eval_pipe_with_path_context_internal::<W, S>(
                            rest,
                            &result,
                            root,
                            current_path,
                            optional,
                        )
                    }
                }
                Err(_) if optional => QueryResult::None,
                Err(e) => QueryResult::Error(e),
            }
        }
    }
}

/// Helper to evaluate a builtin with an OwnedValue
fn eval_builtin_owned<S: EvalSemantics>(
    builtin: &Builtin,
    value: &OwnedValue,
    optional: bool,
) -> Result<OwnedValue, EvalError> {
    // For most builtins, we can just delegate to eval_owned_expr
    eval_owned_expr::<S>(&Expr::Builtin(builtin.clone()), value, optional)
}

/// Builtin: path(expr) - return the path to values selected by expr
/// This evaluates the expression while tracking the path taken to reach each value.
fn builtin_path<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let owned = to_owned(&value);

    // Computed keys must become static components before the tracker runs — it
    // ignores anything it does not recognise, so an unresolved one would
    // silently yield no paths at all.
    let exprs = match resolve_dynamic_indexes::<S>(expr, &owned) {
        Ok(exprs) => exprs,
        Err(e) => return QueryResult::Error(e),
    };

    let mut reached = Vec::new();
    for expr in &exprs {
        if let Err(e) = walk_path::<S>(expr, &owned, &[], &mut reached, optional) {
            return QueryResult::Error(e);
        }
    }

    let mut paths: Vec<OwnedValue> = reached
        .into_iter()
        .map(|(path, _)| OwnedValue::Array(path))
        .collect();

    match paths.len() {
        // "No paths at all" is *no output*, never `Array(vec![])`. The empty
        // path is a real answer — it is what `path(.)` returns — and the one
        // path that always resolves, so rendering emptiness as it aims a
        // caller's `getpath`/`setpath`/`delpaths` at the document root (#489).
        0 => QueryResult::None,
        1 => QueryResult::Owned(paths.pop().expect("len checked")),
        _ => QueryResult::ManyOwned(paths),
    }
}

/// Walk `expr` as a path expression, pushing `(path, value-at-path)` for every
/// path it resolves to.
///
/// Every step's verdict — whether it resolves at all, and to what — comes from
/// the value evaluator via [`eval_owned_multi`], never from a second copy of
/// jq's indexing rules here. That is what keeps `path(f)` agreeing with `f`:
/// a step that *reads* as `null` (a missing key, an out-of-range index, any
/// step through `null`) keeps its component, exactly as jq's does, and a step
/// that cannot index its value at all raises the sentence `src/jq/error.rs`
/// already spells the way jq does rather than inventing a component for a
/// place that does not exist.
///
/// `optional` is that error's off switch — `?`, which is precisely "turn this
/// step's error into no output". It arrives either from an `Expr::Optional`
/// wrapper (`.a?`) or from an enclosing `path(...)?`, and both mean the same
/// thing here.
///
/// One walker, not one per position: the last step of a path and the steps
/// before it obey the same rules, and keeping two copies of them is what let
/// `path(.b.c)` lose the path that `path(.b)` kept (#489).
fn walk_path<S: EvalSemantics>(
    expr: &Expr,
    value: &OwnedValue,
    current_path: &[OwnedValue],
    out: &mut Vec<(Vec<OwnedValue>, OwnedValue)>,
    optional: bool,
) -> Result<(), EvalError> {
    match expr {
        // The path already reaching here, named as it stands.
        Expr::Identity => out.push((current_path.to_vec(), value.clone())),

        // One component each, written as the step was written rather than as
        // it resolves: jq keeps `path(.[-1])` as `[-1]` and `path(.[1:2])` as
        // `[{"start":1,"end":2}]`, because a path is resolved against a
        // container only when it is applied.
        Expr::Field(name) => {
            step_into::<S>(
                expr,
                OwnedValue::String(name.clone()),
                value,
                current_path,
                out,
                optional,
            )?;
        }
        Expr::Index(idx) => {
            step_into::<S>(
                expr,
                OwnedValue::Int(*idx),
                value,
                current_path,
                out,
                optional,
            )?;
        }
        Expr::Slice { start, end } => {
            step_into::<S>(
                expr,
                slice::literal_component(*start, *end),
                value,
                current_path,
                out,
                optional,
            )?;
        }

        // The one step whose components come from the *value* rather than the
        // expression, so it reads them off the container directly. Anything
        // that is not a container still takes the evaluator's verdict, which
        // is jq's `Cannot iterate over <t> (<v>)`.
        Expr::Iterate => match value {
            OwnedValue::Array(arr) => {
                for (i, val) in arr.iter().enumerate() {
                    out.push((extend(current_path, OwnedValue::Int(i as i64)), val.clone()));
                }
            }
            OwnedValue::Object(entries) => {
                for (key, val) in entries {
                    out.push((
                        extend(current_path, OwnedValue::String(key.clone())),
                        val.clone(),
                    ));
                }
            }
            other => {
                if let Err(e) = eval_owned_multi::<S>(expr, other) {
                    if !optional {
                        return Err(e);
                    }
                }
            }
        },

        Expr::Pipe(exprs) => return walk_pipe::<S>(exprs, value, current_path, out, optional),
        Expr::Optional(inner) => return walk_path::<S>(inner, value, current_path, out, true),
        Expr::Paren(inner) => return walk_path::<S>(inner, value, current_path, out, optional),

        // Unreachable: computed keys are resolved to static components before
        // the walker runs. The catch-all below emits *no* path rather than an
        // error, so an unresolved one would silently produce an empty result.
        Expr::IndexExpr { .. } => {
            debug_assert!(false, "unresolved computed index reached path tracking");
        }

        // Expressions with no path-tracking arm — `..`, `recurse`, `select`,
        // arithmetic — name no path (#483). `builtin_path` renders that as no
        // output, which is still the wrong answer but no longer a path that
        // resolves.
        _ => {}
    }
    Ok(())
}

/// Walk a pipe of path steps, threading each value reached into the next step.
fn walk_pipe<S: EvalSemantics>(
    exprs: &[Expr],
    value: &OwnedValue,
    current_path: &[OwnedValue],
    out: &mut Vec<(Vec<OwnedValue>, OwnedValue)>,
    optional: bool,
) -> Result<(), EvalError> {
    let Some((first, rest)) = exprs.split_first() else {
        // An empty pipe reaches nothing new, so it names the path handed to
        // it — `path(())` is `[]`, as `path(.)` is.
        out.push((current_path.to_vec(), value.clone()));
        return Ok(());
    };
    if rest.is_empty() {
        return walk_path::<S>(first, value, current_path, out, optional);
    }

    let mut reached = Vec::new();
    walk_path::<S>(first, value, current_path, &mut reached, optional)?;
    for (path, val) in reached {
        walk_pipe::<S>(rest, &val, &path, out, optional)?;
    }
    Ok(())
}

/// Take one path step: `component` names it, and the value evaluator decides
/// both what it reaches and whether it may be taken at all.
///
/// Asking the evaluator rather than re-deciding here is the point — see
/// [`walk_path`]. `Err` is suppressed into no output under `?`, which is all
/// `?` means on a path step.
fn step_into<S: EvalSemantics>(
    step: &Expr,
    component: OwnedValue,
    value: &OwnedValue,
    current_path: &[OwnedValue],
    out: &mut Vec<(Vec<OwnedValue>, OwnedValue)>,
    optional: bool,
) -> Result<(), EvalError> {
    let values = match eval_owned_multi::<S>(step, value) {
        Ok(values) => values,
        Err(_) if optional => return Ok(()),
        Err(e) => return Err(e),
    };
    debug_assert!(values.len() <= 1, "one path step reaches at most one value");
    let Some(reached) = values.into_iter().next() else {
        return Ok(());
    };
    out.push((extend(current_path, component), reached));
    Ok(())
}

/// `current_path` with one more component on the end.
fn extend(current_path: &[OwnedValue], component: OwnedValue) -> Vec<OwnedValue> {
    let mut path = current_path.to_vec();
    path.push(component);
    path
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

/// Helper to collect `tostream`-style events: `[path, value]` for every leaf
/// (scalar, or an empty array/object), plus a closing `[path]` marker after
/// every *non-empty* container, whose path is the container's own path with
/// its last key/index appended (jq's own convention — verified against
/// jq-1.7.1, see `test_tostream_*` below).
fn collect_tostream_events(value: &OwnedValue, path: &[OwnedValue], events: &mut Vec<OwnedValue>) {
    match value {
        OwnedValue::Object(entries) if !entries.is_empty() => {
            let mut last_path = path.to_vec();
            for (key, val) in entries {
                let mut child_path = path.to_vec();
                child_path.push(OwnedValue::String(key.clone()));
                collect_tostream_events(val, &child_path, events);
                last_path = child_path;
            }
            events.push(OwnedValue::Array(vec![OwnedValue::Array(last_path)]));
        }
        OwnedValue::Array(arr) if !arr.is_empty() => {
            let mut last_path = path.to_vec();
            for (i, val) in arr.iter().enumerate() {
                let mut child_path = path.to_vec();
                child_path.push(OwnedValue::Int(i as i64));
                collect_tostream_events(val, &child_path, events);
                last_path = child_path;
            }
            events.push(OwnedValue::Array(vec![OwnedValue::Array(last_path)]));
        }
        leaf => {
            events.push(OwnedValue::Array(vec![
                OwnedValue::Array(path.to_vec()),
                leaf.clone(),
            ]));
        }
    }
}

/// Builtin: tostream - jq-compatible stream of `[path,value]` / `[path]` events
fn builtin_tostream<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
    let owned = to_owned(&value);
    let mut events = Vec::new();
    collect_tostream_events(&owned, &[], &mut events);
    // Always non-empty: every value (including an empty container or a
    // top-level scalar) produces at least one leaf event.
    if events.len() == 1 {
        QueryResult::Owned(events.pop().unwrap())
    } else {
        QueryResult::ManyOwned(events)
    }
}

/// Builtin: fromstream(f) - reconstruct values from a stream of tostream-style events
///
/// Mirrors jq's own `builtin.jq` definition: an accumulator `x` built up via
/// `setpath`, and a completion flag `e` that is recomputed from each event's
/// path length, reset to the initial state whenever the previous event
/// completed a value.
fn builtin_fromstream<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    f: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let events = match eval_single::<W, S>(f, value, optional) {
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
        result => result.collect_owned(),
    };

    let mut outputs = Vec::new();
    let mut x = OwnedValue::Null;
    let mut e = false;

    for event in events {
        if e {
            x = OwnedValue::Null;
        }
        let OwnedValue::Array(parts) = &event else {
            return QueryResult::Error(EvalError::new("Invalid streaming format"));
        };
        match parts.as_slice() {
            [OwnedValue::Array(path), leaf_value] => {
                e = path.is_empty();
                x = match set_value_at_path(x, path, leaf_value.clone()) {
                    Ok(v) => v,
                    Err(err) => return QueryResult::Error(err),
                };
            }
            [OwnedValue::Array(path)] => {
                e = path.len() == 1;
            }
            _ => return QueryResult::Error(EvalError::new("Invalid streaming format")),
        }
        if e {
            outputs.push(x.clone());
        }
    }

    match outputs.len() {
        0 => QueryResult::None,
        1 => QueryResult::Owned(outputs.pop().unwrap()),
        _ => QueryResult::ManyOwned(outputs),
    }
}

/// Builtin: truncate_stream(stream) - drop the leading `.` path components
/// from `stream`'s events.
///
/// jq's real signature takes a single filter argument: the depth comes from
/// `.` (the value piped into `truncate_stream` itself), not a second
/// argument, and `stream` is evaluated against that same `.` — matching
/// jq-1.7.1's `def truncate_stream(stream): . as $n | stream | ...`.
///
/// jq's own definition compares the path length against `$n` with the
/// generic `>` operator, not a numeric type check, so a non-numeric `.` at
/// the call site (e.g. `null | truncate_stream(...)`) does not error — every
/// event's length sorts above `null` in jq's ordering, so every event is
/// kept unmodified. [`compare_values`] reuses that same ordering here.
fn builtin_truncate_stream<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    stream_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let depth = to_owned(&value);

    let events = match eval_single::<W, S>(stream_expr, value, optional) {
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
        result => result.collect_owned(),
    };

    let mut outputs = Vec::new();
    for event in events {
        let OwnedValue::Array(mut parts) = event else {
            return QueryResult::Error(EvalError::new("Invalid streaming format"));
        };
        let Some(OwnedValue::Array(path)) = parts.first() else {
            return QueryResult::Error(EvalError::new("Invalid path in streaming format"));
        };
        let path_len = OwnedValue::Int(path.len() as i64);
        if compare_values(&path_len, &depth) == core::cmp::Ordering::Greater {
            // Reachable non-number depths here are only null/bool (jq order:
            // null < bool < number < ...), never string/array/object, since
            // a number never sorts above those. Both null and bool slice as
            // offset 0 — null matches jq's own `.[null:]` semantics; a
            // boolean depth is what real jq refuses with "Array/string slice
            // indices must be integers", which succinctly does not reproduce
            // here (out of scope: `truncate_stream` is always called with an
            // integer depth in practice).
            let offset = depth.as_f64().map_or(0, |f| f.max(0.0) as usize);
            let truncated = path[offset.min(path.len())..].to_vec();
            parts[0] = OwnedValue::Array(truncated);
            outputs.push(OwnedValue::Array(parts));
        }
    }

    match outputs.len() {
        0 => QueryResult::None,
        1 => QueryResult::Owned(outputs.pop().unwrap()),
        _ => QueryResult::ManyOwned(outputs),
    }
}

/// Builtin: paths - all paths to values (excluding empty paths)
/// Returns each path as a separate output (streaming), matching jq behavior
fn builtin_paths<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
    let owned = to_owned(&value);
    let mut paths = Vec::new();
    collect_paths(&owned, &[], &mut paths);
    // Stream individual paths instead of wrapping in array
    if paths.is_empty() {
        QueryResult::None
    } else if paths.len() == 1 {
        QueryResult::Owned(paths.pop().unwrap())
    } else {
        QueryResult::ManyOwned(paths)
    }
}

/// Builtin: paths(filter) - paths to values matching filter
/// Returns each path as a separate output (streaming), matching jq behavior
fn builtin_paths_filter<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
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
                if let Ok(OwnedValue::Bool(true)) =
                    eval_owned_expr::<S>(filter, &val_at_path, optional)
                {
                    filtered_paths.push(path);
                }
            }
        }
    }
    // Stream individual paths instead of wrapping in array
    if filtered_paths.is_empty() {
        QueryResult::None
    } else if filtered_paths.len() == 1 {
        QueryResult::Owned(filtered_paths.pop().unwrap())
    } else {
        QueryResult::ManyOwned(filtered_paths)
    }
}

/// Helper to get value at a path (alias for convenience)
#[inline]
fn get_value_at_owned_path(value: &OwnedValue, path: &[OwnedValue]) -> Option<OwnedValue> {
    get_value_at_path(value, path)
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
/// Returns each path as a separate output (streaming), matching jq's paths(scalars) behavior
fn builtin_leaf_paths<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
    let owned = to_owned(&value);
    let mut paths = Vec::new();
    collect_leaf_paths(&owned, &[], &mut paths);
    // Stream individual paths instead of wrapping in array
    if paths.is_empty() {
        QueryResult::None
    } else if paths.len() == 1 {
        QueryResult::Owned(paths.pop().unwrap())
    } else {
        QueryResult::ManyOwned(paths)
    }
}

/// Resolve a `setpath` array index against the array's current length.
///
/// jq truncates a float index toward zero — `setpath([1.7]; v)` writes element
/// 1 and `setpath([-0.5]; v)` writes element 0 — and refuses NaN outright. A
/// negative index counts back from the end; one that is *still* negative after
/// that is out of bounds rather than clamped to zero, which is also what keeps
/// the caller's null padding bounded: `(len + idx) as usize` for
/// `[1,2] | setpath([-5]; 9)` wraps to ~1.8e19 and pads until memory runs out.
fn resolve_setpath_index(key: &OwnedValue, len: usize) -> Result<usize, EvalError> {
    let index = match key {
        OwnedValue::Int(i) => *i,
        OwnedValue::Float(f) if f.is_nan() => {
            return Err(EvalError::new("Cannot set array element at NaN index"))
        }
        OwnedValue::Float(f) => f.trunc() as i64,
        OwnedValue::NumberLiteral(NumberRepr::Int(i), _) => *i,
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => {
            return Err(EvalError::new("Cannot set array element at NaN index"))
        }
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) => f.trunc() as i64,
        // Not reachable from `set_value_at_path`, which matches the numeric
        // path elements before calling; stated so the helper stands alone.
        other => {
            return Err(EvalError::cannot_index_with_type(
                "array",
                other.type_name(),
            ))
        }
    };

    let resolved = if index < 0 { len as i64 + index } else { index };
    if resolved < 0 {
        return Err(EvalError::out_of_bounds_negative_index());
    }
    // Still fallible on a 32-bit target, where a large positive index does not
    // fit a `usize`. That is the same refusal as `pad_with_nulls`', not a
    // negative index, so it must not borrow the sentence above.
    usize::try_from(resolved).map_err(|_| cannot_grow_array(resolved as u64 + 1))
}

/// Refusal for an array length that cannot be allocated.
///
/// Not a jq sentence: jq does not survive the filters that reach this (it dies
/// on the allocation), so there is no wording to reproduce — see
/// `docs/compliance/jq/limitations.md`.
fn cannot_grow_array(len: u64) -> EvalError {
    EvalError::new(format!("Cannot grow array to {len} elements"))
}

/// Pad `arr` with nulls until `index` is a valid element, reporting a failure
/// the caller can catch.
///
/// The length is taken from the document — `setpath([n]; v)` writes element `n`
/// of an array it grows to fit — so an absurd `n` asks for an absurd
/// allocation. `Vec::resize` answers that with a `capacity overflow` *panic*,
/// which for a library means taking the embedder down with it: `null |
/// setpath([1e30]; 9)` wants 9.2e18 elements. jq has no sentence to reproduce
/// here because jq does not survive the same filter either (it dies on the
/// allocation), so this is succinctly's own wording.
///
/// Only the impossible is refused: every length that fits in memory still
/// grows, which is what keeps `[1,2] | setpath([5]; 9)` agreeing with jq.
fn pad_with_nulls(arr: &mut Vec<OwnedValue>, index: usize) -> Result<(), EvalError> {
    // `index + 1` is only an overflow on a 32-bit target, where `usize::MAX`
    // is a reachable index; the refusal is the same either way.
    let len = index
        .checked_add(1)
        .ok_or_else(|| cannot_grow_array(index as u64 + 1))?;
    arr.try_reserve(len - arr.len())
        .map_err(|_| cannot_grow_array(len as u64))?;
    arr.resize(len, OwnedValue::Null);
    Ok(())
}

/// Helper to set a value at a path
///
/// Follows jq: `null` is the only value auto-vivified into whatever container
/// the next path element needs. Every other non-container refuses to be
/// indexed, so `1 | setpath(["a"]; 1)` is an error rather than `{"a":1}`
/// (#359), and so does a container indexed with the wrong kind of key —
/// `{} | setpath([0]; 1)`, `[] | setpath(["a"]; 1)`.
fn set_value_at_path(
    value: OwnedValue,
    path: &[OwnedValue],
    new_val: OwnedValue,
) -> Result<OwnedValue, EvalError> {
    let Some(key) = path.first() else {
        return Ok(new_val);
    };
    let rest = &path[1..];

    match key {
        OwnedValue::String(name) => match value {
            OwnedValue::Object(mut entries) => {
                if let Some(slot) = entries.get_mut(name) {
                    // Replace through the slot rather than remove-and-reinsert:
                    // jq leaves an existing key where it was, and `IndexMap`
                    // would move it to the end after a `shift_remove`.
                    let old = core::mem::replace(slot, OwnedValue::Null);
                    *slot = set_value_at_path(old, rest, new_val)?;
                } else {
                    let val = set_value_at_path(OwnedValue::Null, rest, new_val)?;
                    entries.insert(name.clone(), val);
                }
                Ok(OwnedValue::Object(entries))
            }
            OwnedValue::Null => {
                let mut entries = IndexMap::new();
                entries.insert(
                    name.clone(),
                    set_value_at_path(OwnedValue::Null, rest, new_val)?,
                );
                Ok(OwnedValue::Object(entries))
            }
            other => Err(EvalError::cannot_index(other.type_name(), key)),
        },
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            let mut arr = match value {
                OwnedValue::Array(arr) => arr,
                OwnedValue::Null => Vec::new(),
                other => return Err(EvalError::cannot_index(other.type_name(), key)),
            };
            let index = resolve_setpath_index(key, arr.len())?;
            if index >= arr.len() {
                pad_with_nulls(&mut arr, index)?;
            }
            let old = core::mem::replace(&mut arr[index], OwnedValue::Null);
            arr[index] = set_value_at_path(old, rest, new_val)?;
            Ok(OwnedValue::Array(arr))
        }
        // An object path element is jq's slice, `{"start":s,"end":e}` — what
        // `path(.[1:2])` yields. Writing through one reads the sub-array,
        // walks `rest` inside it, and splices the answer back over the
        // original range, so the replacement has to be an array however deep
        // the path went: `null | setpath([{"start":0,"end":1},"a"]; 9)` builds
        // `{"a":9}` and is refused here, as it is in jq.
        OwnedValue::Object(desc) => {
            // Only a container jq would slice gets as far as the descriptor;
            // on anything else the refusal names the container, which is why
            // `{"a":1} | setpath([{"foo":1}]; 9)` is `Cannot index object with
            // object` while `"abc"` and `null` report the malformed descriptor.
            if !matches!(
                value,
                OwnedValue::Array(_) | OwnedValue::Null | OwnedValue::String(_)
            ) {
                return Err(EvalError::cannot_index(value.type_name(), key));
            }
            let bounds = SliceBounds::from_descriptor(desc)?;
            // jq reads a string slice but will not write one back.
            if matches!(value, OwnedValue::String(_)) {
                return Err(EvalError::cannot_update_string_slices());
            }
            // jq reads the child with `jv_get` before recursing, and a `null`
            // root reads as `null` rather than as an empty array — which is
            // what makes `null | setpath([{"start":0,"end":1},"a"]; 9)` build
            // `{"a":9}` and then fail the array check below, as jq does.
            let (mut arr, sub, range) = match value {
                OwnedValue::Array(arr) => {
                    let range = bounds.resolve(arr.len());
                    let sub = OwnedValue::Array(arr[range.clone()].to_vec());
                    (arr, sub, range)
                }
                _ => (Vec::new(), OwnedValue::Null, 0..0),
            };
            let OwnedValue::Array(items) = set_value_at_path(sub, rest, new_val)? else {
                return Err(EvalError::slice_assign_non_array());
            };
            arr.splice(range, items);
            Ok(OwnedValue::Array(arr))
        }
        // null, booleans and arrays index nothing, in any container.
        _ => Err(EvalError::cannot_index(value.type_name(), key)),
    }
}

/// Builtin: setpath(path; value) - set value at path
fn builtin_setpath<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    path_expr: &Expr,
    val_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate path expression
    let path_result = eval_single::<W, S>(path_expr, value.clone(), optional);
    let path_owned = match path_result {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        // A suppressed sub-result (e.g. `setpath((.a)?; 1)` on a value `.a`
        // can't index) propagates as a suppressed whole, not `null`/an error.
        QueryResult::None => return QueryResult::None,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::path_must_be_array()),
    };

    let path = match path_owned {
        OwnedValue::Array(p) => p,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::path_must_be_array()),
    };

    // Evaluate value expression
    let new_val = match eval_single::<W, S>(val_expr, value.clone(), optional) {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        // Same reasoning as the path result above: `setpath(["a"]; error)?`
        // suppresses the whole call rather than setting `"a"` to `null` (#367).
        QueryResult::None => return QueryResult::None,
        _ => OwnedValue::Null,
    };

    let owned = to_owned(&value);
    match set_value_at_path(owned, &path, new_val) {
        Ok(result) => QueryResult::Owned(result),
        // An optional context swallows the refusal, as it does for every other
        // builtin here.
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(e),
    }
}

/// Delete every path in `paths` from `value`, the way jq's `delpaths_sorted`
/// does (jq's `src/jv_aux.c`).
///
/// `paths` is sorted ascending in jq's total value order, and every path is
/// longer than `start`. Paths sharing the component at `start` form one run.
/// If the run's first — hence, the list being sorted, its shortest — path ends
/// here, the whole subtree goes and the longer paths under it are never
/// walked: `[10,20,30,40] | delpaths([[0],[0,1]])` is `[20,30,40]`, not an
/// attempt to index into the number it just deleted. `delpaths([[0,1]])` on
/// its own *is* an error in jq for exactly that reason (#415).
///
/// Every key that ends at one level is collected and removed in a single pass,
/// which is the part a per-path loop cannot reproduce: the deletions there are
/// simultaneous, so each index resolves against the length the array had
/// *before* any sibling was removed. `delpaths([[-1],[-2]])` is `[10,20]`, not
/// the `[10,30]` that deleting one at a time gives (#398).
fn delete_paths_sorted(
    mut value: OwnedValue,
    paths: &[&[OwnedValue]],
    start: usize,
) -> Result<OwnedValue, EvalError> {
    debug_assert!(
        paths.iter().all(|p| p.len() > start),
        "every path in a run is longer than the depth it is grouped at"
    );
    let mut del_keys: Vec<&OwnedValue> = Vec::new();
    let mut i = 0;
    while i < paths.len() {
        let key = &paths[i][start];
        let mut j = i + 1;
        while j < paths.len() && compare_values(&paths[j][start], key) == core::cmp::Ordering::Equal
        {
            j += 1;
        }
        // The run's first path is its shortest — a prefix sorts before its own
        // extensions — so this decides the whole run, and a repeated path
        // contributes one key rather than one deletion per mention.
        if paths[i].len() == start + 1 {
            del_keys.push(key);
        } else {
            value = delete_paths_under(value, key, &paths[i..j], start + 1)?;
        }
        i = j;
    }
    delete_keys(value, &del_keys)
}

/// Recurse into the child of `value` under `key`. A key that names nothing is
/// a no-op, as it is in jq — `{"a":1} | delpaths([["b","c"]])` is `{"a":1}`.
fn delete_paths_under(
    value: OwnedValue,
    key: &OwnedValue,
    paths: &[&[OwnedValue]],
    start: usize,
) -> Result<OwnedValue, EvalError> {
    match value {
        OwnedValue::Object(mut entries) => match key {
            OwnedValue::String(name) => {
                if let Some(slot) = entries.get_mut(name) {
                    // Replace through the slot rather than remove-and-reinsert:
                    // jq leaves an existing key where it was, and `IndexMap`
                    // would move it to the end after a `shift_remove`.
                    let old = core::mem::replace(slot, OwnedValue::Null);
                    *slot = delete_paths_sorted(old, paths, start)?;
                }
                Ok(OwnedValue::Object(entries))
            }
            // A non-string key names no child at all: jq's `Cannot index
            // object with <kind>`.
            other => Err(EvalError::cannot_index("object", other)),
        },
        OwnedValue::Array(mut arr) => match key {
            // An object-shaped key is jq's slice descriptor (`path(.[a:b])`
            // produces `{"start":a,"end":b}`). Recursing *through* one means
            // deleting inside the sub-array and splicing it back:
            // `[1,[2],[3]] | delpaths([[{"start":1,"end":3},0]])` is `[1,[3]]`.
            OwnedValue::Object(desc) => {
                let range = SliceBounds::from_descriptor(desc)?.resolve(arr.len());
                let sub = OwnedValue::Array(arr[range.clone()].to_vec());
                let OwnedValue::Array(items) = delete_paths_sorted(sub, paths, start)? else {
                    unreachable!("deleting from an array yields an array")
                };
                arr.splice(range, items);
                Ok(OwnedValue::Array(arr))
            }
            OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
                if let Some(index) = resolve_read_index(key, arr.len()) {
                    let old = core::mem::replace(&mut arr[index], OwnedValue::Null);
                    arr[index] = delete_paths_sorted(old, paths, start)?;
                }
                Ok(OwnedValue::Array(arr))
            }
            // A non-number key names no element at all: jq's `Cannot index
            // array with <kind>`.
            other => Err(EvalError::cannot_index("array", other)),
        },
        // jq reads the child with `jv_get`: a `null` is skipped, and any other
        // scalar is `Cannot index <type> with <key>`.
        OwnedValue::Null => Ok(OwnedValue::Null),
        other => Err(EvalError::cannot_index(other.type_name(), key)),
    }
}

/// jq's `jv_dels`: remove every key in `keys` from one container in a single
/// pass. Keys naming nothing are ignored, and every array index resolves
/// against the length the array had on entry, so one deletion cannot shift the
/// array under its siblings.
fn delete_keys(value: OwnedValue, keys: &[&OwnedValue]) -> Result<OwnedValue, EvalError> {
    if keys.is_empty() {
        return Ok(value);
    }
    match value {
        OwnedValue::Object(mut entries) => {
            // A non-string key is jq's `Cannot delete <kind> field of object`;
            // checked over every key in the batch, since a run can group
            // several distinct key types together.
            let mut doomed: BTreeSet<&str> = BTreeSet::new();
            for key in keys {
                match key {
                    OwnedValue::String(name) => {
                        doomed.insert(name.as_str());
                    }
                    other => {
                        return Err(EvalError::cannot_delete_field_of_object(other.type_name()))
                    }
                }
            }
            // One order-preserving `retain`, for the reason the array arm below
            // takes the same shape: `shift_remove` per key shifts the tail every
            // time, so deleting half of a 60k-key object cost 4.4s against jq's
            // 0.02s. `retain` is `shift_remove`'s equal for a single key and
            // linear for any number of them.
            entries.retain(|name, _| !doomed.contains(name.as_str()));
            Ok(OwnedValue::Object(entries))
        }
        OwnedValue::Array(mut arr) => {
            // A key that is not a number is jq's `Cannot delete <kind> element
            // of array`. An object-shaped key is jq's slice descriptor
            // (`.[a:b]`), which contributes its whole element range.
            // `resolve_read_index` is `getpath`'s resolver: a float truncates
            // toward zero, a negative counts back from the end, and anything
            // that reaches no element (out of range, or NaN) is dropped rather
            // than raised.
            //
            // Every key resolves against the length the array had on entry and
            // they are deleted in one pass, which is what makes overlapping
            // ranges union rather than compound: `[1,2,3,4] | del(.[0:2],
            // .[1:3])` is `[4]`, and a slice naming the same element as a bare
            // index deletes it once — `delpaths([[1],[{"start":1,"end":2}]])`
            // is `[1,3,4]`.
            let mut indices: Vec<usize> = Vec::with_capacity(keys.len());
            for key in keys {
                match key {
                    OwnedValue::Object(desc) => {
                        indices.extend(SliceBounds::from_descriptor(desc)?.resolve(arr.len()));
                    }
                    OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
                        if let Some(idx) = resolve_read_index(key, arr.len()) {
                            indices.push(idx);
                        }
                    }
                    other => {
                        return Err(EvalError::cannot_delete_element_of_array(other.type_name()))
                    }
                }
            }
            indices.sort_unstable();
            indices.dedup();
            // One `retain` pass with a cursor into the ascending index list. A
            // `Vec::remove` per key would be quadratic, and `delpaths` is
            // precisely how a filter deletes many elements at once.
            let (mut index, mut cursor) = (0usize, 0usize);
            arr.retain(|_| {
                let doomed = cursor < indices.len() && indices[cursor] == index;
                cursor += usize::from(doomed);
                index += 1;
                !doomed
            });
            Ok(OwnedValue::Array(arr))
        }
        // `null` has no fields to begin with, so deleting one is a no-op —
        // jq agrees. Every other scalar is `Cannot delete fields from <type>`.
        OwnedValue::Null => Ok(OwnedValue::Null),
        other => Err(EvalError::cannot_delete_fields_from(other.type_name())),
    }
}

/// One atomic step of a resolved `del` path — a `Field`, `Index`, or
/// `Iterate` component — paired with whether a type or bounds mismatch there
/// should be swallowed rather than raised. That flag comes from a `?` at or
/// before this step *within the path expression itself* (`del(.a?.b)`) —
/// never from `del(...)`'s own `optional` argument, which is `?` wrapping the
/// whole call (`del(.a)?`) and is handled by `builtin_del` catching the
/// walk's error instead (#537); see [`flatten_delete_path`].
struct DeleteStep {
    component: Expr,
    optional: bool,
}

/// Flatten a resolved (fully static) `del` path into atomic steps.
///
/// Mirrors `push_path_components`, but fully resolves `Optional` instead of
/// leaving it as an opaque wrapper: [`delete_expr_paths_at`] compares steps
/// across sibling paths position by position, so it needs one flat sequence
/// of `Field`/`Index`/`Iterate` rather than a tree that hides some of them
/// behind `Optional`/`Pipe` nodes.
fn flatten_delete_path(expr: &Expr, optional: bool, out: &mut Vec<DeleteStep>) {
    match expr {
        Expr::Identity => {}
        Expr::Pipe(exprs) => {
            for e in exprs {
                flatten_delete_path(e, optional, out);
            }
        }
        Expr::Paren(inner) => flatten_delete_path(inner, optional, out),
        Expr::Optional(inner) => flatten_delete_path(inner, true, out),
        other => out.push(DeleteStep {
            component: other.clone(),
            optional,
        }),
    }
}

/// Delete every resolved `del` path in `paths` from `value`, the way
/// [`delete_paths_sorted`] deletes `delpaths`' runtime path arrays: paths
/// sharing a container at this depth are grouped, and their keys are removed
/// from it together, so a negative index resolves against the length the
/// container had before any sibling here was removed (#424) — one at a time,
/// `[10,20,30,40] | del(.[(-1,-2)])` took the last element, shortened the
/// array, and then took what was *now* second-to-last, giving `[10,30]` where
/// jq gives `[10,20]`.
///
/// Unlike `delete_paths_sorted`, paths here are still `Expr` steps rather
/// than resolved `OwnedValue` keys, so navigating a `Field`/`Index` still
/// raises the type and bounds errors `del` has always raised (`delete_keys`
/// cannot yet, #415) — `delete_keys` itself is reused for the part that does
/// not need that: removing a container's own keys simultaneously once they
/// are known.
///
/// Every path here comes from one `del` argument's single expression tree, so
/// whichever branch a computed key took, the shape that follows is *usually*
/// identical — only the concrete `Field`/`Index` values differ — which is
/// what lets every path be exactly as long as its siblings. It is not always
/// identical in *kind*, though: `null` accepts a string key, a numeric key,
/// or `.[]` without erroring (`null | .a`, `null | .[0]`, and `null | .[]`
/// are all `null`), so `.[("a",0)]` resolved against a null target yields one
/// `Field("a")` path and one `Index(0)` path at the same position. Partition
/// by actual shape below rather than trusting `paths[0]` to speak for the
/// rest, which used to panic on exactly that input. Nor always identical in
/// *length*: a bare `.` sibling flattens to zero steps while the rest of the
/// comma keeps going, so the leaf check below scans every sibling rather than
/// trusting `paths[0]` to be exhausted (or not) for all of them (#505).
fn delete_expr_paths_at(
    mut value: OwnedValue,
    paths: &[&[DeleteStep]],
    start: usize,
) -> Result<OwnedValue, EvalError> {
    if paths.is_empty() {
        return Ok(value);
    }
    if paths.iter().any(|path| path.len() == start) {
        // `del(.)`, a path wrapped in enough `?`/`()` to flatten to nothing,
        // or `.` as one branch of a comma whose other branches still have
        // components left (#505): replace the whole value reached here, as
        // `delete_at_path`'s `Expr::Identity` arm does for the single-path
        // case. An exhausted sibling deletes this entire subtree, which
        // subsumes whatever any other sibling here would have deleted from
        // within it — the same short-circuit `delpaths` gets from sorting
        // the empty path first (`Some([]) => Ok(OwnedValue::Null)`), just
        // without needing a sort, since `paths[0]` isn't assumed
        // representative of every sibling's length either.
        return Ok(OwnedValue::Null);
    }

    let mut fields: Vec<&[DeleteStep]> = Vec::new();
    let mut indices: Vec<&[DeleteStep]> = Vec::new();
    let mut iterates: Vec<&[DeleteStep]> = Vec::new();
    for &path in paths {
        match &path[start].component {
            Expr::Field(_) => fields.push(path),
            // A `Slice` joins the `Index` bucket rather than getting one of
            // its own: `delete_expr_array_paths` funnels every terminal key
            // into a single `delete_keys` call, and that one batch is what
            // makes overlapping ranges union instead of compound. Split into
            // two calls, `del(.[0:2], .[1:3])` on `[1,2,3,4]` would resolve
            // the second range against the already-shortened array and give
            // `[3]` where jq gives `[4]`.
            Expr::Index(_) | Expr::Slice { .. } => indices.push(path),
            Expr::Iterate => iterates.push(path),
            // `flatten_delete_path` only ever leaves Field/Index/Slice/Iterate
            // components behind; anything else is a path shape `del` has
            // never supported, matching `delete_at_path`'s catch-all.
            _ => return Err(EvalError::new("cannot use expression as delete target")),
        }
    }

    // `value` can only be one concrete type at a time, so at most one of
    // these three actually mutates it — the others see a container of the
    // wrong shape and take that branch's optional-vs-error path (see
    // `delete_expr_object_paths`/`delete_expr_array_paths`). All three can
    // be non-empty only when `value` is `null`, per the doc comment above.
    if !fields.is_empty() {
        value = delete_expr_object_paths(value, &fields, start)?;
    }
    if !indices.is_empty() {
        value = delete_expr_array_paths(value, &indices, start)?;
    }
    if !iterates.is_empty() {
        value = delete_expr_iterate_paths(value, &iterates, start)?;
    }
    Ok(value)
}

/// Walk what is left of `paths` against the `null` that a step naming nothing
/// reads as, for its errors alone.
///
/// `del(f)` is `delpaths([path(f)])`, and the two halves treat a dead end
/// differently: `path()` reads a missing object key, an out-of-range index, or
/// any key of `null` as `null` and keeps walking, and only then does
/// `delpaths` skip what named nothing. So a dead end is not the end of the
/// walk — the *tail* decides. Every step `null` tolerates is a no-op
/// (`{"a":{}} | del(.a.b.c)`, `null | del(.a.b)`), but `.[]` refuses `null`
/// even there, so `{"a":{}} | del(.a.b.c[])` is jq's `Cannot iterate over null
/// (null)` rather than a no-op (#527).
///
/// Returning early instead — which every one of these sites used to do —
/// exempts the whole rest of the path on the strength of one step, which is
/// how the `.[]` case got lost. Nothing can be written back through a `null`
/// root, so the rebuilt values are discarded and the container the dead end
/// named is left exactly as it was: jq does not vivify it either.
///
/// A path that ends at `start` has no tail to walk and is skipped; the caller
/// passes the position *after* the step that named nothing.
fn delete_expr_paths_through_absent(
    paths: &[&[DeleteStep]],
    start: usize,
) -> Result<(), EvalError> {
    for path in paths {
        if path.len() > start {
            let deleted = delete_expr_paths_at(OwnedValue::Null, &[*path], start)?;
            debug_assert!(
                matches!(deleted, OwnedValue::Null),
                "deleting through a synthetic null produced a value to write back"
            );
        }
    }
    Ok(())
}

/// [`delete_expr_paths_through_absent`]'s single-path counterpart, for
/// [`delete_at_path`]'s chain-walk. Same contract: errors propagate, the
/// rebuilt `null` is discarded, the container is untouched.
fn delete_at_path_through_absent(rest: &Expr, optional: bool) -> Result<(), EvalError> {
    let mut absent = OwnedValue::Null;
    delete_at_path(&mut absent, rest, optional)
}

/// [`delete_expr_paths_at`]'s `Field` case: group by field name, delete the
/// terminal ones from `value` together via [`delete_keys`], and recurse into
/// the rest.
fn delete_expr_object_paths(
    mut value: OwnedValue,
    paths: &[&[DeleteStep]],
    start: usize,
) -> Result<OwnedValue, EvalError> {
    // `null` tolerates any field key unconditionally — `null | del(.a)` is
    // `null` — so every sibling path here is a no-op regardless of
    // `optional`, the same exemption `delete_keys`/`delete_paths_under`
    // already give a runtime key (#476). The exemption is per *step*, though:
    // returning outright would hand it to every later step too, including the
    // `.[]` #476 deliberately withheld it from, so the tails still get walked
    // (#527).
    if matches!(value, OwnedValue::Null) {
        delete_expr_paths_through_absent(paths, start + 1)?;
        return Ok(value);
    }
    let mut terminal: Vec<&str> = Vec::new();
    let mut groups: Vec<(&str, Vec<&[DeleteStep]>)> = Vec::new();
    for path in paths {
        let Expr::Field(name) = &path[start].component else {
            unreachable!("delete_expr_paths_at only dispatches Field paths here")
        };
        let name = name.as_str();
        if path.len() == start + 1 {
            if !terminal.contains(&name) {
                terminal.push(name);
            }
            continue;
        }
        let mut appended = false;
        for group in &mut groups {
            if group.0 == name {
                group.1.push(*path);
                appended = true;
                break;
            }
        }
        if !appended {
            groups.push((name, vec![*path]));
        }
    }

    // Every path here is Field-kind, so `resolve_node` (invoked by
    // `resolve_dynamic_indexes` before `delete_expr_paths_at` ever runs)
    // already validated `value` as field-indexable for each path — raising
    // the same `Cannot index … with …` error itself — before grouping ever
    // sees a non-object root. `null` is excluded by the check above, so this
    // can only fire if that earlier validation regresses.
    let OwnedValue::Object(entries) = &mut value else {
        unreachable!("delete_expr_object_paths reached a non-object, non-null root")
    };
    for (name, group) in &groups {
        match entries.get_mut(*name) {
            Some(slot) => {
                let old = core::mem::replace(slot, OwnedValue::Null);
                *slot = delete_expr_paths_at(old, group, start + 1)?;
            }
            // A field the object doesn't have is a dead end, not an error:
            // `del(.a.b.c, .a.b.d)` is a no-op in jq where this used to raise
            // succinctly's own `field 'b' not found` (#527). The tail decides
            // whether it stays one, so it is walked rather than skipped — see
            // `delete_expr_paths_through_absent`. `optional` gets no say: a
            // `?` marks a *failure to index* as tolerable, and a key that is
            // merely absent never failed, which is why jq raises for
            // `del(.a.b?[])` just as it does for `del(.a.b[])`.
            None => delete_expr_paths_through_absent(group, start + 1)?,
        }
    }

    if !terminal.is_empty() {
        let owned_keys: Vec<OwnedValue> = terminal
            .iter()
            .map(|name| OwnedValue::String((*name).to_string()))
            .collect();
        let key_refs: Vec<&OwnedValue> = owned_keys.iter().collect();
        value = delete_keys(value, &key_refs)?;
    }

    Ok(value)
}

/// [`delete_expr_paths_at`]'s `Index` case: group by index, delete the
/// terminal ones from `value` together via [`delete_keys`] — the one call
/// that resolves every negative index against `value`'s length once, rather
/// than once per deletion — and recurse into the rest.
fn delete_expr_array_paths(
    mut value: OwnedValue,
    paths: &[&[DeleteStep]],
    start: usize,
) -> Result<OwnedValue, EvalError> {
    // Same per-step `null` exemption as `delete_expr_object_paths` above —
    // applies to both a bare index and a slice component (#476), and likewise
    // covers only this step, so the tails are still walked (#527).
    if matches!(value, OwnedValue::Null) {
        delete_expr_paths_through_absent(paths, start + 1)?;
        return Ok(value);
    }
    let mut terminal: Vec<(ArrayStep, bool)> = Vec::new();
    let mut groups: Vec<(ArrayStep, Vec<&[DeleteStep]>)> = Vec::new();
    for path in paths {
        let step = match &path[start].component {
            Expr::Index(idx) => ArrayStep::Index(*idx),
            Expr::Slice { start, end } => ArrayStep::Slice(*start, *end),
            _ => unreachable!("delete_expr_paths_at only dispatches Index/Slice paths here"),
        };
        if path.len() == start + 1 {
            // One occurrence of `step` marking it optional is enough to cover
            // every other occurrence — merge rather than keep only whichever
            // one was pushed first, which used to make the outcome depend on
            // argument order (`del(.[(0,5)], .[5]?)` errored or not
            // depending on which side `.[5]?` was written on).
            match terminal.iter_mut().find(|(s, _)| *s == step) {
                Some(entry) => entry.1 |= path[start].optional,
                None => terminal.push((step, path[start].optional)),
            }
            continue;
        }
        let mut appended = false;
        for group in &mut groups {
            if group.0 == step {
                group.1.push(*path);
                appended = true;
                break;
            }
        }
        if !appended {
            groups.push((step, vec![*path]));
        }
    }

    // Unlike the object case above, this is NOT dead code: `resolve_node`
    // validates a `Slice` component by evaluating it as a *read*, and slicing
    // a string is a legal read (`"hi" | .[0:1]` is `"h"`) even though `del`
    // through that same slice is not. `"hi" | del(.[0:1], .[1:2])` reaches
    // here with `value` still a `String`, so this gate is load-bearing —
    // dd2df4d1 removed it as believed-unreachable and #504's CI caught the
    // regression via `test_del_static_comma_type_error_reports_the_first_sibling`.
    if !matches!(value, OwnedValue::Array(_)) {
        // A non-array container fails every path here identically, so a
        // single non-optional path among the siblings has to raise even when
        // others are optional.
        //
        // *Which* sentence comes from the first sibling, because jq walks the
        // paths in source order and dies on the first: `5 | del(.[0], .[1:2])`
        // is `Cannot index number with number`, and the same two written the
        // other way round is `… with object`.
        return if paths.iter().all(|p| p[start].optional) {
            Ok(value)
        } else {
            Err(match &paths[0][start].component {
                Expr::Slice { .. } if matches!(value, OwnedValue::String(_)) => {
                    EvalError::cannot_delete_fields_from("string")
                }
                Expr::Slice { .. } => {
                    EvalError::cannot_index_with_type(owned_type_name(&value), "object")
                }
                _ => EvalError::cannot_index_with_type(owned_type_name(&value), "number"),
            })
        };
    }

    if let OwnedValue::Array(arr) = &mut value {
        for (step, group) in &groups {
            match step {
                ArrayStep::Index(idx) => {
                    let len = arr.len() as i64;
                    let actual = if *idx < 0 { len + idx } else { *idx };
                    if actual >= 0 && (actual as usize) < arr.len() {
                        let slot = &mut arr[actual as usize];
                        let old = core::mem::replace(slot, OwnedValue::Null);
                        *slot = delete_expr_paths_at(old, group, start + 1)?;
                    }
                    // An out-of-range index names nothing to delete through —
                    // jq's delpaths silently skips it, `?` or not (#477).
                }
                // Deleting *through* a slice deletes inside the sub-array and
                // splices it back: `[1,[2],[3]] | del(.[1:3][0])` is `[1,[3]]`.
                ArrayStep::Slice(s, e) => {
                    let range = SliceBounds::from_literals(*s, *e).resolve(arr.len());
                    let sub = OwnedValue::Array(arr[range.clone()].to_vec());
                    let OwnedValue::Array(items) = delete_expr_paths_at(sub, group, start + 1)?
                    else {
                        unreachable!("deleting from an array yields an array")
                    };
                    arr.splice(range, items);
                }
            }
        }
    }

    // Terminal indices are deleted below via `delete_keys`, which already
    // silently drops an index that names nothing (`delpaths` has always done
    // that, #415) — including an out-of-range one, `?` or not (#477).
    if !terminal.is_empty() {
        let owned_keys: Vec<OwnedValue> = terminal
            .iter()
            .map(|(step, _)| match step {
                ArrayStep::Index(idx) => OwnedValue::Int(*idx),
                ArrayStep::Slice(s, e) => slice::literal_component(*s, *e),
            })
            .collect();
        let key_refs: Vec<&OwnedValue> = owned_keys.iter().collect();
        value = delete_keys(value, &key_refs)?;
    }

    Ok(value)
}

/// One array-level step of a `del` path: a bare index, or a slice.
///
/// The two share a bucket in [`delete_expr_paths_at`] because they name
/// elements of the same array and have to reach [`delete_keys`] in one batch —
/// see the comment there on why splitting them would compound overlapping
/// ranges instead of unioning them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ArrayStep {
    Index(i64),
    Slice(Option<i64>, Option<i64>),
}

/// [`delete_expr_paths_at`]'s `Iterate` case: every path here shares the same
/// tail — an `Iterate` names no component of its own to differ by — so there
/// is nothing to group. Either every path ends here, clearing the whole
/// container, or every path continues, and each element/value recurses with
/// the same sibling list.
fn delete_expr_iterate_paths(
    value: OwnedValue,
    paths: &[&[DeleteStep]],
    start: usize,
) -> Result<OwnedValue, EvalError> {
    let optional = paths[0][start].optional;
    if paths[0].len() == start + 1 {
        return match value {
            OwnedValue::Array(mut arr) => {
                arr.clear();
                Ok(OwnedValue::Array(arr))
            }
            OwnedValue::Object(mut map) => {
                map.clear();
                Ok(OwnedValue::Object(map))
            }
            other if optional => Ok(other),
            other => Err(EvalError::cannot_iterate(&other)),
        };
    }
    match value {
        OwnedValue::Array(mut arr) => {
            for elem in &mut arr {
                let old = core::mem::replace(elem, OwnedValue::Null);
                *elem = delete_expr_paths_at(old, paths, start + 1)?;
            }
            Ok(OwnedValue::Array(arr))
        }
        OwnedValue::Object(mut map) => {
            for v in map.values_mut() {
                let old = core::mem::replace(v, OwnedValue::Null);
                *v = delete_expr_paths_at(old, paths, start + 1)?;
            }
            Ok(OwnedValue::Object(map))
        }
        other if optional => Ok(other),
        other => Err(EvalError::cannot_iterate(&other)),
    }
}

/// Builtin: del(path) - delete a single path
fn builtin_del<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    path_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Convert to owned and delete the path
    let result = to_owned(&value);

    // `optional` here is `del(...)`'s *own* `?` (`del(.a)?`), i.e. jq's
    // `try del(.a) catch empty` around the whole call — not a per-step
    // tolerance. It must never reach the walkers below as their starting
    // flag: threading it in there converts a step's type/bounds error into a
    // silent no-op that still emits the unchanged input, where jq emits
    // nothing at all (#537). Instead every fallible step here passes `false`
    // and any resulting error is caught right here, turning the *whole call's
    // output* into empty when `optional` is set. A `?` written *inside* the
    // path (`del(.a?)`) is unaffected — it's already a distinct
    // `Expr::Optional` node baked into `path_expr`, which the walkers still
    // honor on their own.

    // Computed keys resolve against the original document; each becomes one
    // fully static path expression (Field/Index/Iterate components — see
    // `resolve_dynamic_indexes`).
    let paths = match resolve_dynamic_indexes::<S>(path_expr, &result) {
        Ok(paths) => paths,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // The overwhelmingly common case — no computed key at all, or one that
    // resolved to a single value — has no sibling that could shift under it,
    // so it keeps the simple per-path walk.
    if paths.len() <= 1 {
        let mut result = result;
        for path in &paths {
            if let Err(e) = delete_at_path(&mut result, path, false) {
                return if optional {
                    QueryResult::None
                } else {
                    QueryResult::Error(e)
                };
            }
        }
        return QueryResult::Owned(result);
    }

    // More than one resolved path happens when a computed key produced
    // several values (`.[(0,2)]`, `.[.a,.b]`, …), or when a top-level `Comma`
    // names more than one static path outright (`.a, .b`, #475) — either way
    // the same shape #424 got wrong by deleting them one at a time.
    // `flatten_delete_path` reduces each resolved `Expr` to the same
    // atomic-steps shape so `delete_expr_paths_at` can delete every resolved
    // path together.
    let flattened: Vec<Vec<DeleteStep>> = paths
        .iter()
        .map(|path| {
            let mut steps = Vec::new();
            flatten_delete_path(path, false, &mut steps);
            steps
        })
        .collect();
    let refs: Vec<&[DeleteStep]> = flattened.iter().map(Vec::as_slice).collect();

    match delete_expr_paths_at(result, &refs, 0) {
        Ok(result) => QueryResult::Owned(result),
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(e),
    }
}

/// Delete a value at a path expression.
fn delete_at_path(
    root: &mut OwnedValue,
    path_expr: &Expr,
    optional: bool,
) -> Result<(), EvalError> {
    match path_expr {
        Expr::Identity => {
            // del(.) replaces with null
            *root = OwnedValue::Null;
            Ok(())
        }
        Expr::Field(name) => match root {
            OwnedValue::Object(map) => {
                map.shift_remove(name);
                Ok(())
            }
            // jq indexes `null` with any key and gets `null` back, so
            // deleting through one is always a no-op — `null | del(.a)` is
            // `null` (#476).
            OwnedValue::Null => Ok(()),
            _ if optional => Ok(()),
            _ => Err(EvalError::cannot_index_with_field(
                owned_type_name(root),
                name,
            )),
        },
        Expr::Index(idx) => match root {
            OwnedValue::Array(arr) => {
                let len = arr.len() as i64;
                let actual_idx = if *idx < 0 { len + idx } else { *idx };
                if actual_idx >= 0 && (actual_idx as usize) < arr.len() {
                    arr.remove(actual_idx as usize);
                }
                // An out-of-range index names nothing to delete — jq's
                // delpaths silently skips it, `?` or not (#477).
                Ok(())
            }
            // `null` has no elements at any index, so this is always a
            // no-op — `null | del(.[0])` is `null` (#476).
            OwnedValue::Null => Ok(()),
            _ if optional => Ok(()),
            _ => Err(EvalError::cannot_index_with_type(
                owned_type_name(root),
                "number",
            )),
        },
        Expr::Iterate => {
            // del(.[]) removes all elements
            match root {
                OwnedValue::Array(arr) => {
                    arr.clear();
                    Ok(())
                }
                OwnedValue::Object(map) => {
                    map.clear();
                    Ok(())
                }
                _ if optional => Ok(()),
                _ => Err(EvalError::cannot_iterate(root)),
            }
        }
        // `del(.[a:b])` drops the whole range. A range that reaches nothing is
        // silently empty rather than an error — unlike the `Expr::Index` arm
        // above, jq does not refuse an out-of-range slice, so `[1,2,3] |
        // del(.[5:9])` is `[1,2,3]`.
        Expr::Slice { start, end } => match root {
            OwnedValue::Array(arr) => {
                let range = SliceBounds::from_literals(*start, *end).resolve(arr.len());
                arr.drain(range);
                Ok(())
            }
            // `null` has no elements to drop, so jq leaves it alone.
            OwnedValue::Null => Ok(()),
            _ if optional => Ok(()),
            OwnedValue::String(_) => Err(EvalError::cannot_delete_fields_from("string")),
            other => Err(EvalError::cannot_index_with_type(
                owned_type_name(other),
                "object",
            )),
        },
        Expr::Pipe(exprs) if !exprs.is_empty() => {
            // Chain: navigate and delete at the last path
            if exprs.len() == 1 {
                delete_at_path(root, &exprs[0], optional)
            } else {
                // Same unwrap as `update_path`'s chain arm: a resolved
                // component can still be wrapped in `?`, and matching the
                // wrapper as an unknown component used to fall through to the
                // catch-all below, which deletes at *this* position and
                // strands the rest of the path — `del(recurse | objects |
                // .[.k]?)` removing the whole parent instead of one key.
                let (first, first_optional) = unwrap_path_component(&exprs[0]);
                let here = optional || first_optional;
                let rest = Expr::Pipe(exprs[1..].to_vec());

                match first {
                    Expr::Field(name) => match root {
                        OwnedValue::Object(map) => match map.get_mut(name) {
                            Some(current) => delete_at_path(current, &rest, optional),
                            // Same "reads as `null`, keep walking" rule as
                            // `delete_expr_object_paths`' missing-field arm
                            // (#527): `del(.a.b.c)` is a no-op, `del(.a.b[])`
                            // still raises. `here` no longer gates this — a
                            // `?` on the missing step does not suppress what
                            // the tail itself raises, and the walk into a
                            // throwaway `null` cannot create the key.
                            None => delete_at_path_through_absent(&rest, optional),
                        },
                        // `null` tolerates any key — `null | del(.a.b)` and
                        // `{"x":null} | del(.x.a)` are both no-ops (#476) —
                        // but only for *this* step. Returning here handed the
                        // exemption to the whole rest of the chain, `.[]`
                        // included, so `{"x":null} | del(.x.a[])` no-op'd
                        // where jq raises `Cannot iterate over null (null)`
                        // (#527).
                        OwnedValue::Null => delete_at_path_through_absent(&rest, optional),
                        _ if here => Ok(()),
                        _ => Err(EvalError::cannot_index_with_field(
                            owned_type_name(root),
                            name,
                        )),
                    },
                    Expr::Index(idx) => match root {
                        OwnedValue::Array(arr) => {
                            let len = arr.len() as i64;
                            let actual_idx = if *idx < 0 { len + idx } else { *idx };
                            if actual_idx >= 0 && (actual_idx as usize) < arr.len() {
                                delete_at_path(&mut arr[actual_idx as usize], &rest, optional)
                            } else {
                                // An out-of-range index resolves to null;
                                // deleting further into null is always a
                                // no-op, `?` or not (#477).
                                Ok(())
                            }
                        }
                        // Same per-step `null` exemption as the `Field` case
                        // above — `null | del(.[0].a)` is a no-op (#476),
                        // `null | del(.[0][])` still raises (#527).
                        OwnedValue::Null => delete_at_path_through_absent(&rest, optional),
                        _ if here => Ok(()),
                        _ => Err(EvalError::cannot_index_with_type(
                            owned_type_name(root),
                            "number",
                        )),
                    },
                    Expr::Iterate => match root {
                        OwnedValue::Array(arr) => {
                            for elem in arr.iter_mut() {
                                delete_at_path(elem, &rest, optional)?;
                            }
                            Ok(())
                        }
                        OwnedValue::Object(map) => {
                            for value in map.values_mut() {
                                delete_at_path(value, &rest, optional)?;
                            }
                            Ok(())
                        }
                        _ if here => Ok(()),
                        _ => Err(EvalError::cannot_iterate(root)),
                    },
                    // A nested pipe from `Optional(Pipe([…]))` — splice, do
                    // not recurse on it alone, or `rest` is stranded.
                    Expr::Pipe(inner) => {
                        let mut spliced = inner.clone();
                        spliced.extend_from_slice(&exprs[1..]);
                        delete_at_path(root, &Expr::Pipe(spliced), here)
                    }
                    // The chain continues *inside* the slice, so the delete
                    // happens in the sub-array and the remainder is spliced
                    // back: `[1,[2],[3]] | del(.[1:3][0])` is `[1,[3]]`, not
                    // the whole range dropped.
                    // `null` has no elements to descend into, matching the
                    // top-level `Expr::Slice` arm's `Null` case above
                    // (#476) — `null | del(.[0:2].a)` is a no-op, while
                    // `null | del(.[0:2][])` still raises from the tail
                    // (#527). This is deliberately not pushed into
                    // `through_slice` itself: that helper is shared with
                    // `=`/`|=` assignment, where a slice write auto-vivifies
                    // `null` instead of no-op'ing (a separate, documented
                    // divergence — see docs/compliance/jq/limitations.md).
                    Expr::Slice { .. } if matches!(root, OwnedValue::Null) => {
                        delete_at_path_through_absent(&rest, optional)
                    }
                    Expr::Slice { start, end } => through_slice(root, *start, *end, here, |sub| {
                        delete_at_path(sub, &rest, optional)
                    }),
                    _ => delete_at_path(root, first, here),
                }
            }
        }
        Expr::Optional(inner) => delete_at_path(root, inner, true),
        // Unreachable: `resolve_dynamic_indexes` rewrites every computed key
        // into a static component before this runs. Explicit rather than left
        // to the catch-all so a missed install point fails loudly here instead
        // of being reported as a user error.
        Expr::IndexExpr { .. } => Err(EvalError::new(
            "internal error: unresolved computed index in delete path",
        )),
        _ => Err(EvalError::new("cannot use expression as delete target")),
    }
}

// Phase 12: Additional builtins

/// Builtin: now - current Unix timestamp
fn builtin_now<'a, W: Clone + AsRef<[u64]>>() -> QueryResult<'a, W> {
    #[cfg(feature = "std")]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => QueryResult::Owned(OwnedValue::Float(duration.as_secs_f64())),
            Err(_) => QueryResult::Error(EvalError::new("failed to get current time")),
        }
    }
    #[cfg(not(feature = "std"))]
    {
        // no_std environment - return 0.0 as a fallback
        QueryResult::Owned(OwnedValue::Float(0.0))
    }
}

/// Builtin: gmtime - convert Unix timestamp to broken-down UTC time
/// Returns [year, month(0-11), day(1-31), hour, minute, second, weekday(0-6, Sunday=0), yearday(0-365)]
fn builtin_gmtime<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    let timestamp = match get_float_value::<W>(&value, optional) {
        Ok(f) => f,
        Err(r) => return r,
    };

    // Convert Unix timestamp to broken-down time (UTC)
    let secs = timestamp.trunc() as i64;

    // Days since Unix epoch (Jan 1, 1970)
    let days = if secs >= 0 {
        secs / 86400
    } else {
        (secs - 86399) / 86400
    };
    let time_of_day = ((secs % 86400) + 86400) % 86400;
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    // Calculate year, month, day from days since epoch
    // Using algorithm from Howard Hinnant's date library
    let z = days + 719468; // days since Mar 1, 0000
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32; // day of era [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // year of era [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month [0, 11] starting from March
    let day = (doy - (153 * mp + 2) / 5 + 1) as i64;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);

    // Calculate weekday (0 = Sunday, 6 = Saturday)
    // Jan 1, 1970 was a Thursday (4)
    let weekday = (days % 7 + 4 + 7) % 7;

    // Calculate day of year (0-365, 0 = Jan 1)
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days: [i64; 12] = if is_leap {
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335]
    } else {
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
    };
    let yearday = month_days[(month - 1) as usize] + day - 1;

    let result = vec![
        OwnedValue::Int(year),
        OwnedValue::Int((month - 1) as i64), // 0-indexed month
        OwnedValue::Int(day),
        OwnedValue::Int(hour),
        OwnedValue::Int(minute),
        OwnedValue::Int(second),
        OwnedValue::Int(weekday),
        OwnedValue::Int(yearday),
    ];

    QueryResult::Owned(OwnedValue::Array(result))
}

/// Builtin: localtime - convert Unix timestamp to broken-down local time
/// Returns [year, month(0-11), day(1-31), hour, minute, second, weekday(0-6, Sunday=0), yearday(0-365)]
fn builtin_localtime<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    #[cfg(feature = "std")]
    {
        let timestamp = match get_float_value::<W>(&value, optional) {
            Ok(f) => f,
            Err(r) => return r,
        };

        // Get local timezone offset using chrono if available, otherwise fall back to gmtime
        // For now, we'll compute it manually using the libc-style approach
        // This is a simplified implementation that uses a heuristic for timezone offset

        // Get the current local time offset by comparing system time with UTC
        use std::time::{SystemTime, UNIX_EPOCH};
        let now_utc = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);

        // We need to compute the local offset. A simple approach is to use environment TZ,
        // but that's complex. For simplicity, we'll compute gmtime and apply a local offset.
        // This implementation uses the system's local time approximation.

        // For a proper implementation, we'd need platform-specific code or a crate like chrono.
        // For now, we'll approximate by using a simple offset calculation.

        // Actually, let's just use gmtime logic with an offset.
        // Try to get the timezone offset from the system.

        // Simplified: compute based on the offset from UTC
        // In practice, this should use libc::localtime_r or similar
        // For now, we'll attempt to detect offset using the current time

        // Get offset: (local_now - utc_now) rounded to nearest minute
        // This is a hack - proper implementation needs platform time APIs

        // Fallback: Use UTC for now (same as gmtime)
        // A proper implementation would use platform-specific APIs or chrono crate
        let secs = timestamp.trunc() as i64;

        // Try to estimate local offset by looking at current system time
        // This gives us the offset at the current moment (may differ from timestamp's offset due to DST)
        let local_offset = estimate_local_offset(now_utc);
        let local_secs = secs + local_offset;

        // Days since Unix epoch
        let days = if local_secs >= 0 {
            local_secs / 86400
        } else {
            (local_secs - 86399) / 86400
        };
        let time_of_day = ((local_secs % 86400) + 86400) % 86400;
        let hour = time_of_day / 3600;
        let minute = (time_of_day % 3600) / 60;
        let second = time_of_day % 60;

        // Calculate year, month, day from days since epoch
        let z = days + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = (z - era * 146097) as u32;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as i64;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = y + i64::from(month <= 2);

        let weekday = (days % 7 + 4 + 7) % 7;

        let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let month_days: [i64; 12] = if is_leap {
            [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335]
        } else {
            [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
        };
        let yearday = month_days[(month - 1) as usize] + day - 1;

        let result = vec![
            OwnedValue::Int(year),
            OwnedValue::Int((month - 1) as i64),
            OwnedValue::Int(day),
            OwnedValue::Int(hour),
            OwnedValue::Int(minute),
            OwnedValue::Int(second),
            OwnedValue::Int(weekday),
            OwnedValue::Int(yearday),
        ];

        QueryResult::Owned(OwnedValue::Array(result))
    }
    #[cfg(not(feature = "std"))]
    {
        // In no_std, fall back to gmtime (UTC)
        builtin_gmtime::<W>(value, optional)
    }
}

/// Estimate local timezone offset in seconds from UTC
#[cfg(feature = "std")]
fn estimate_local_offset(utc_secs: i64) -> i64 {
    // This is a simplified estimation that works for most common cases
    // A proper implementation would use platform-specific APIs

    // Try to get TZ from environment
    if let Ok(tz) = std::env::var("TZ") {
        // Parse simple TZ formats like "EST5EDT" or "PST8PDT"
        // Format: STDoffset[DST[offset][,rule]]
        if let Some(offset) = parse_simple_tz_offset(&tz) {
            return offset;
        }
    }

    // Fallback: try to detect from system
    // On many systems, we can compute the offset by comparing local and UTC representations
    // For a portable solution without external crates, we'll return 0 (UTC)
    // Users needing accurate local time should ensure TZ is set correctly
    let _ = utc_secs; // silence unused warning
    0
}

/// Parse a simple TZ offset like "EST5" or "PST8" and return offset in seconds
#[cfg(feature = "std")]
fn parse_simple_tz_offset(tz: &str) -> Option<i64> {
    // Skip the timezone name (letters)
    let offset_start = tz.find(|c: char| c.is_ascii_digit() || c == '-' || c == '+')?;
    let offset_part = &tz[offset_start..];

    // Find where the offset ends (at DST name or end of string)
    let offset_end = offset_part
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(offset_part.len());
    let offset_str = &offset_part[..offset_end];

    // Parse the offset (hours, optionally minutes)
    let negative = offset_str.starts_with('-');
    let offset_str = offset_str.trim_start_matches(['+', '-']);

    let parts: Vec<&str> = offset_str.split(':').collect();
    let hours: i64 = parts.first()?.parse().ok()?;
    let minutes: i64 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);

    // TZ offset is positive for west of UTC, but we want seconds to add
    let offset_secs = (hours * 3600 + minutes * 60) * if negative { 1 } else { -1 };
    Some(offset_secs)
}

/// Builtin: mktime - convert broken-down time to Unix timestamp
fn builtin_mktime<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    let arr = match to_owned(&value) {
        OwnedValue::Array(a) => a,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("array", "mktime")),
    };

    // Need at least 6 elements: [year, month, day, hour, minute, second]
    if arr.len() < 6 {
        if optional {
            return QueryResult::None;
        }
        return QueryResult::Error(EvalError::new(
            "mktime requires array with at least 6 elements",
        ));
    }

    let get_int = |idx: usize| -> Result<i64, EvalError> {
        match arr.get(idx) {
            Some(OwnedValue::Int(n)) => Ok(*n),
            Some(OwnedValue::Float(f)) => Ok(*f as i64),
            Some(OwnedValue::NumberLiteral(NumberRepr::Int(n), _)) => Ok(*n),
            Some(OwnedValue::NumberLiteral(NumberRepr::Float(f), _)) => Ok(*f as i64),
            _ => Err(EvalError::new(format!(
                "mktime: element {idx} must be a number"
            ))),
        }
    };

    let year = match get_int(0) {
        Ok(y) => y,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };
    let month = match get_int(1) {
        Ok(m) => m + 1, // jq uses 0-indexed months
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };
    let day = match get_int(2) {
        Ok(d) => d,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };
    let hour = match get_int(3) {
        Ok(h) => h,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };
    let minute = match get_int(4) {
        Ok(m) => m,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };
    let second = match get_int(5) {
        Ok(s) => s,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(e),
    };

    // Convert to Unix timestamp using inverse of the gmtime algorithm
    // Algorithm from Howard Hinnant's date library (civil_from_days inverse)
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32; // year of era [0, 399]
    let m = month as u32;
    let d = day as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // day of year [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // day of era [0, 146096]
    let days = era * 146097 + doe as i64 - 719468; // days since Unix epoch

    let timestamp = days * 86400 + hour * 3600 + minute * 60 + second;

    QueryResult::Owned(OwnedValue::Float(timestamp as f64))
}

/// Builtin: strftime(fmt) - format broken-down time as string
fn builtin_strftime<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    fmt_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // First get the format string
    let fmt = match result_to_owned(eval_single::<W, S>(fmt_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "strftime format")),
        Err(e) => return QueryResult::Error(e),
    };

    // Value should be a broken-down time array
    let arr = match to_owned(&value) {
        OwnedValue::Array(a) => a,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("array", "strftime")),
    };

    if arr.len() < 6 {
        if optional {
            return QueryResult::None;
        }
        return QueryResult::Error(EvalError::new(
            "strftime requires array with at least 6 elements",
        ));
    }

    let get_int = |idx: usize| -> i64 {
        match arr.get(idx) {
            Some(OwnedValue::Int(n)) => *n,
            Some(OwnedValue::Float(f)) => *f as i64,
            Some(OwnedValue::NumberLiteral(NumberRepr::Int(n), _)) => *n,
            Some(OwnedValue::NumberLiteral(NumberRepr::Float(f), _)) => *f as i64,
            _ => 0,
        }
    };

    let year = get_int(0);
    let month = get_int(1) + 1; // jq uses 0-indexed
    let day = get_int(2);
    let hour = get_int(3);
    let minute = get_int(4);
    let second = get_int(5);
    let weekday = if arr.len() > 6 { get_int(6) } else { 0 };
    let yearday = if arr.len() > 7 { get_int(7) } else { 0 };

    let result = format_strftime(
        &fmt, year, month, day, hour, minute, second, weekday, yearday,
    );
    QueryResult::Owned(OwnedValue::String(result))
}

/// Format a time according to strftime format specifiers
#[allow(clippy::too_many_arguments)] // STYLE-0004: mirrors strftime's broken-down-time field set
fn format_strftime(
    fmt: &str,
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    weekday: i64,
    yearday: i64,
) -> String {
    let mut result = String::new();
    let mut chars = fmt.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => result.push('%'),
                Some('Y') => result.push_str(&format!("{year:04}")),
                Some('y') => result.push_str(&format!("{:02}", year % 100)),
                Some('m') => result.push_str(&format!("{month:02}")),
                Some('d') => result.push_str(&format!("{day:02}")),
                Some('e') => result.push_str(&format!("{day:2}")),
                Some('H') => result.push_str(&format!("{hour:02}")),
                Some('I') => result.push_str(&format!(
                    "{:02}",
                    if hour == 0 {
                        12
                    } else if hour > 12 {
                        hour - 12
                    } else {
                        hour
                    }
                )),
                Some('M') => result.push_str(&format!("{minute:02}")),
                Some('S') => result.push_str(&format!("{second:02}")),
                Some('p') => result.push_str(if hour < 12 { "AM" } else { "PM" }),
                Some('P') => result.push_str(if hour < 12 { "am" } else { "pm" }),
                Some('j') => result.push_str(&format!("{:03}", yearday + 1)), // 1-indexed
                Some('w') => result.push_str(&format!("{weekday}")),
                Some('u') => {
                    result.push_str(&format!("{}", if weekday == 0 { 7 } else { weekday }));
                } // Monday=1
                Some('a') => {
                    let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                    result.push_str(names[weekday as usize % 7]);
                }
                Some('A') => {
                    let names = [
                        "Sunday",
                        "Monday",
                        "Tuesday",
                        "Wednesday",
                        "Thursday",
                        "Friday",
                        "Saturday",
                    ];
                    result.push_str(names[weekday as usize % 7]);
                }
                Some('b' | 'h') => {
                    let names = [
                        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                        "Nov", "Dec",
                    ];
                    result.push_str(names[(month - 1) as usize % 12]);
                }
                Some('B') => {
                    let names = [
                        "January",
                        "February",
                        "March",
                        "April",
                        "May",
                        "June",
                        "July",
                        "August",
                        "September",
                        "October",
                        "November",
                        "December",
                    ];
                    result.push_str(names[(month - 1) as usize % 12]);
                }
                Some('C') => result.push_str(&format!("{:02}", year / 100)),
                Some('D') => result.push_str(&format!("{:02}/{:02}/{:02}", month, day, year % 100)),
                Some('F') => result.push_str(&format!("{year:04}-{month:02}-{day:02}")),
                Some('R') => result.push_str(&format!("{hour:02}:{minute:02}")),
                Some('T') => result.push_str(&format!("{hour:02}:{minute:02}:{second:02}")),
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('z') => result.push_str("+0000"), // UTC offset (we're always UTC for gmtime)
                Some('Z') => result.push_str("UTC"),
                Some(other) => {
                    result.push('%');
                    result.push(other);
                }
                None => result.push('%'),
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Builtin: strptime(fmt) - parse string to broken-down time
fn builtin_strptime<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    fmt_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // First get the format string
    let fmt = match result_to_owned(eval_single::<W, S>(fmt_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "strptime format")),
        Err(e) => return QueryResult::Error(e),
    };

    // Value should be a string
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", "strptime")),
    };

    match parse_strptime(&input, &fmt) {
        Ok(t) => {
            let result = vec![
                OwnedValue::Int(t.year),
                OwnedValue::Int(t.month - 1), // 0-indexed
                OwnedValue::Int(t.day),
                OwnedValue::Int(t.hour),
                OwnedValue::Int(t.minute),
                OwnedValue::Int(t.second),
                OwnedValue::Int(t.weekday),
                OwnedValue::Int(t.yearday),
            ];
            QueryResult::Owned(OwnedValue::Array(result))
        }
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(EvalError::new(e)),
    }
}

/// Broken-down time representation (matches jq's format)
struct BrokenDownTime {
    year: i64,
    month: i64, // 1-indexed (1-12)
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    weekday: i64, // 0=Sunday
    yearday: i64, // 0-indexed (0-365)
}

/// Parse a time string according to strptime format specifiers
#[allow(clippy::type_complexity)] // STYLE-0004: return mirrors strptime's broken-down-time field set
fn parse_strptime(input: &str, fmt: &str) -> Result<BrokenDownTime, String> {
    let mut year: i64 = 1970;
    let mut month: i64 = 1;
    let mut day: i64 = 1;
    let mut hour: i64 = 0;
    let mut minute: i64 = 0;
    let mut second: i64 = 0;
    // weekday and yearday are parsed from format specifiers like %w, %j, %u,
    // but then recalculated at the end for consistency with the parsed date.
    // This matches jq's behavior where weekday/yearday in output are always
    // computed from the date, not taken from the parsed input.
    #[allow(unused_variables, unused_assignments)]
    // STYLE-0004: weekday parsed from %w/%u then recomputed from the date (jq parity)
    let mut weekday: i64 = 4; // Thursday (Jan 1, 1970)
    #[allow(unused_variables, unused_assignments)]
    // STYLE-0004: yearday parsed from %j then recomputed from the date (jq parity)
    let mut yearday: i64 = 0;

    let mut input_iter = input.chars().peekable();
    let mut fmt_iter = fmt.chars().peekable();

    while let Some(fc) = fmt_iter.next() {
        if fc == '%' {
            match fmt_iter.next() {
                Some('%') => {
                    if input_iter.next() != Some('%') {
                        return Err("expected '%'".to_string());
                    }
                }
                Some('Y') => {
                    year = parse_digits(&mut input_iter, 4)?;
                }
                Some('y') => {
                    let y = parse_digits(&mut input_iter, 2)?;
                    year = if y >= 69 { 1900 + y } else { 2000 + y };
                }
                Some('m') => {
                    month = parse_digits(&mut input_iter, 2)?;
                }
                Some('d') => {
                    day = parse_digits(&mut input_iter, 2)?;
                }
                Some('e') => {
                    // Skip leading space if present
                    if input_iter.peek() == Some(&' ') {
                        input_iter.next();
                    }
                    day = parse_digits(&mut input_iter, 2)?;
                }
                Some('H') => {
                    hour = parse_digits(&mut input_iter, 2)?;
                }
                Some('I') => {
                    hour = parse_digits(&mut input_iter, 2)?;
                    // Will be adjusted by %p if present
                }
                Some('M') => {
                    minute = parse_digits(&mut input_iter, 2)?;
                }
                Some('S') => {
                    second = parse_digits(&mut input_iter, 2)?;
                }
                Some('p' | 'P') => {
                    let mut ampm = String::new();
                    while let Some(&c) = input_iter.peek() {
                        if c.is_ascii_alphabetic() {
                            ampm.push(c);
                            input_iter.next();
                        } else {
                            break;
                        }
                    }
                    let ampm_lower = ampm.to_lowercase();
                    if ampm_lower == "pm" && hour < 12 {
                        hour += 12;
                    } else if ampm_lower == "am" && hour == 12 {
                        hour = 0;
                    }
                }
                #[allow(unused_assignments)]
                // STYLE-0004: assignment superseded once the date is recomputed (jq parity)
                Some('j') => {
                    // Parse day-of-year, but we recalculate it from the date for consistency
                    yearday = parse_digits(&mut input_iter, 3)? - 1; // Convert to 0-indexed
                }
                #[allow(unused_assignments)]
                // STYLE-0004: assignment superseded once the date is recomputed (jq parity)
                Some('w') => {
                    // Parse weekday, but we recalculate it from the date for consistency
                    weekday = parse_digits(&mut input_iter, 1)?;
                }
                #[allow(unused_assignments)]
                // STYLE-0004: assignment superseded once the date is recomputed (jq parity)
                Some('u') => {
                    // Parse ISO weekday (1=Monday, 7=Sunday), convert to 0=Sunday
                    let w = parse_digits(&mut input_iter, 1)?;
                    weekday = if w == 7 { 0 } else { w };
                }
                Some('a' | 'A') => {
                    // Skip day name
                    while let Some(&c) = input_iter.peek() {
                        if c.is_ascii_alphabetic() {
                            input_iter.next();
                        } else {
                            break;
                        }
                    }
                }
                Some('b' | 'B' | 'h') => {
                    let mut name = String::new();
                    while let Some(&c) = input_iter.peek() {
                        if c.is_ascii_alphabetic() {
                            name.push(c);
                            input_iter.next();
                        } else {
                            break;
                        }
                    }
                    let name_lower = name.to_lowercase();
                    let months = [
                        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct",
                        "nov", "dec",
                    ];
                    for (i, m) in months.iter().enumerate() {
                        if name_lower.starts_with(m) {
                            month = (i + 1) as i64;
                            break;
                        }
                    }
                }
                Some('C') => {
                    let century = parse_digits(&mut input_iter, 2)?;
                    year = century * 100 + (year % 100);
                }
                Some('D') => {
                    // mm/dd/yy
                    month = parse_digits(&mut input_iter, 2)?;
                    if input_iter.next() != Some('/') {
                        return Err("expected '/'".to_string());
                    }
                    day = parse_digits(&mut input_iter, 2)?;
                    if input_iter.next() != Some('/') {
                        return Err("expected '/'".to_string());
                    }
                    let y = parse_digits(&mut input_iter, 2)?;
                    year = if y >= 69 { 1900 + y } else { 2000 + y };
                }
                Some('F') => {
                    // yyyy-mm-dd
                    year = parse_digits(&mut input_iter, 4)?;
                    if input_iter.next() != Some('-') {
                        return Err("expected '-'".to_string());
                    }
                    month = parse_digits(&mut input_iter, 2)?;
                    if input_iter.next() != Some('-') {
                        return Err("expected '-'".to_string());
                    }
                    day = parse_digits(&mut input_iter, 2)?;
                }
                Some('R') => {
                    // HH:MM
                    hour = parse_digits(&mut input_iter, 2)?;
                    if input_iter.next() != Some(':') {
                        return Err("expected ':'".to_string());
                    }
                    minute = parse_digits(&mut input_iter, 2)?;
                }
                Some('T') => {
                    // HH:MM:SS
                    hour = parse_digits(&mut input_iter, 2)?;
                    if input_iter.next() != Some(':') {
                        return Err("expected ':'".to_string());
                    }
                    minute = parse_digits(&mut input_iter, 2)?;
                    if input_iter.next() != Some(':') {
                        return Err("expected ':'".to_string());
                    }
                    second = parse_digits(&mut input_iter, 2)?;
                }
                Some('n' | 't') => {
                    // Skip whitespace
                    while let Some(&c) = input_iter.peek() {
                        if c.is_whitespace() {
                            input_iter.next();
                        } else {
                            break;
                        }
                    }
                }
                Some('z') => {
                    // Skip timezone offset like +0000 or -0500
                    if let Some(&c) = input_iter.peek() {
                        if c == '+' || c == '-' {
                            input_iter.next();
                            for _ in 0..4 {
                                if input_iter.peek().is_some_and(char::is_ascii_digit) {
                                    input_iter.next();
                                }
                            }
                        }
                    }
                }
                Some('Z') => {
                    // Skip timezone name
                    while let Some(&c) = input_iter.peek() {
                        if c.is_ascii_alphabetic() {
                            input_iter.next();
                        } else {
                            break;
                        }
                    }
                }
                Some(other) => {
                    // Unknown specifier - skip % and match literal
                    if input_iter.next() != Some(other) {
                        return Err(format!("expected '{other}'"));
                    }
                }
                None => {
                    // Trailing % - match literal
                    if input_iter.next() != Some('%') {
                        return Err("expected '%'".to_string());
                    }
                }
            }
        } else if fc.is_whitespace() {
            // Skip any whitespace in input
            while let Some(&c) = input_iter.peek() {
                if c.is_whitespace() {
                    input_iter.next();
                } else {
                    break;
                }
            }
        } else {
            // Match literal character
            match input_iter.next() {
                Some(c) if c == fc => {}
                Some(c) => return Err(format!("expected '{fc}', got '{c}'")),
                None => return Err(format!("expected '{fc}', got end of input")),
            }
        }
    }

    // Calculate weekday if not explicitly set
    // Using Zeller's congruence or similar
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = month as u32;
    let d = day as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe as i64 - 719468;

    // Calculate weekday from days
    weekday = (days % 7 + 4 + 7) % 7;

    // Calculate yearday
    let is_leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let month_days: [i64; 12] = if is_leap {
        [0, 31, 60, 91, 121, 152, 182, 213, 244, 274, 305, 335]
    } else {
        [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334]
    };
    let yearday = month_days[(month - 1) as usize] + day - 1;

    Ok(BrokenDownTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday,
        yearday,
    })
}

/// Parse up to n digits from input
fn parse_digits(
    input: &mut core::iter::Peekable<core::str::Chars>,
    max_digits: usize,
) -> Result<i64, String> {
    let mut s = String::new();
    for _ in 0..max_digits {
        if let Some(&c) = input.peek() {
            if c.is_ascii_digit() {
                s.push(c);
                input.next();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    if s.is_empty() {
        return Err("expected digits".to_string());
    }
    s.parse().map_err(|_| "invalid number".to_string())
}

/// Builtin: todate - convert Unix timestamp to ISO 8601 date string
fn builtin_todate<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    let timestamp = match get_float_value::<W>(&value, optional) {
        Ok(f) => f,
        Err(r) => return r,
    };

    // Convert to broken-down time first
    let secs = timestamp.trunc() as i64;
    let days = if secs >= 0 {
        secs / 86400
    } else {
        (secs - 86399) / 86400
    };
    let time_of_day = ((secs % 86400) + 86400) % 86400;
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);

    let result = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");

    QueryResult::Owned(OwnedValue::String(result))
}

/// Builtin: fromdate - parse ISO 8601 date string to Unix timestamp
fn builtin_fromdate<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    let input = match &value {
        StandardJson::String(s) => match s.as_str() {
            Ok(cow) => cow.into_owned(),
            Err(_) if optional => return QueryResult::None,
            Err(_) => return QueryResult::Error(EvalError::new("invalid string")),
        },
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("string", "fromdate")),
    };

    // Parse ISO 8601 format: YYYY-MM-DDTHH:MM:SSZ or YYYY-MM-DDTHH:MM:SS+HH:MM
    match parse_iso8601(&input) {
        Ok(timestamp) => QueryResult::Owned(OwnedValue::Float(timestamp)),
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(EvalError::new(e)),
    }
}

/// Parse ISO 8601 date string to Unix timestamp
fn parse_iso8601(input: &str) -> Result<f64, String> {
    // Handle common ISO 8601 formats:
    // YYYY-MM-DDTHH:MM:SSZ
    // YYYY-MM-DDTHH:MM:SS.sssZ
    // YYYY-MM-DDTHH:MM:SS+HH:MM
    // YYYY-MM-DD

    let input = input.trim();

    // Try to parse with strptime-like logic
    let mut chars = input.chars().peekable();

    // Year
    let year: i64 = parse_digits(&mut chars, 4)?;

    if chars.next() != Some('-') {
        return Err("expected '-' after year".to_string());
    }

    // Month
    let month: i64 = parse_digits(&mut chars, 2)?;

    if chars.next() != Some('-') {
        return Err("expected '-' after month".to_string());
    }

    // Day
    let day: i64 = parse_digits(&mut chars, 2)?;

    // Check for time component
    let (hour, minute, second, tz_offset) =
        if chars.peek() == Some(&'T') || chars.peek() == Some(&'t') || chars.peek() == Some(&' ') {
            chars.next(); // Skip T or space

            let hour: i64 = parse_digits(&mut chars, 2)?;

            if chars.next() != Some(':') {
                return Err("expected ':' after hour".to_string());
            }

            let minute: i64 = parse_digits(&mut chars, 2)?;

            let second = if chars.peek() == Some(&':') {
                chars.next();
                let s = parse_digits(&mut chars, 2)?;
                // Skip fractional seconds
                if chars.peek() == Some(&'.') {
                    chars.next();
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        chars.next();
                    }
                }
                s
            } else {
                0
            };

            // Parse timezone
            let tz_offset = match chars.peek() {
                Some('Z' | 'z') => {
                    chars.next();
                    0
                }
                Some('+') => {
                    chars.next();
                    let h = parse_digits(&mut chars, 2)?;
                    let m = if chars.peek() == Some(&':') {
                        chars.next();
                        parse_digits(&mut chars, 2)?
                    } else if chars.peek().is_some_and(char::is_ascii_digit) {
                        parse_digits(&mut chars, 2)?
                    } else {
                        0
                    };
                    -(h * 3600 + m * 60) // Positive offset means behind UTC
                }
                Some('-') => {
                    chars.next();
                    let h = parse_digits(&mut chars, 2)?;
                    let m = if chars.peek() == Some(&':') {
                        chars.next();
                        parse_digits(&mut chars, 2)?
                    } else if chars.peek().is_some_and(char::is_ascii_digit) {
                        parse_digits(&mut chars, 2)?
                    } else {
                        0
                    };
                    h * 3600 + m * 60 // Negative offset means ahead of UTC
                }
                _ => 0, // Assume UTC if no timezone
            };

            (hour, minute, second, tz_offset)
        } else {
            (0, 0, 0, 0)
        };

    // Convert to Unix timestamp
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let m = month as u32;
    let d = day as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe as i64 - 719468;

    let timestamp = days * 86400 + hour * 3600 + minute * 60 + second + tz_offset;

    Ok(timestamp as f64)
}

// Phase 21: Extended Date/Time functions (yq)

/// Builtin: from_unix - convert Unix epoch to ISO 8601 date string
/// This is semantically identical to todate/todateiso8601 but with yq naming convention
fn builtin_from_unix<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    // from_unix is the same as todate - converts Unix timestamp to ISO 8601 string
    builtin_todate::<W>(value, optional)
}

/// Builtin: to_unix - convert ISO 8601 date string to Unix epoch
/// This is semantically identical to fromdate/fromdateiso8601 but with yq naming convention
fn builtin_to_unix<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    // to_unix is the same as fromdate - parses ISO 8601 string to Unix timestamp
    builtin_fromdate::<W>(value, optional)
}

/// Format Unix timestamp to ISO 8601 string with timezone offset
fn format_datetime_with_offset(timestamp: f64, offset_seconds: i64) -> String {
    // Apply the timezone offset to the timestamp
    let adjusted_secs = timestamp.trunc() as i64 + offset_seconds;

    let days = if adjusted_secs >= 0 {
        adjusted_secs / 86400
    } else {
        (adjusted_secs - 86399) / 86400
    };
    let time_of_day = ((adjusted_secs % 86400) + 86400) % 86400;
    let hour = time_of_day / 3600;
    let minute = (time_of_day % 3600) / 60;
    let second = time_of_day % 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);

    if offset_seconds == 0 {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    } else {
        let offset_hours = offset_seconds.abs() / 3600;
        let offset_mins = (offset_seconds.abs() % 3600) / 60;
        let sign = if offset_seconds >= 0 { '+' } else { '-' };
        format!(
            "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}{sign}{offset_hours:02}:{offset_mins:02}"
        )
    }
}

/// Get timezone offset in seconds for a given IANA timezone name
/// Returns Ok(offset_seconds) or Err(error_message)
fn get_timezone_offset(zone: &str, timestamp: f64) -> Result<i64, String> {
    match zone.to_lowercase().as_str() {
        "utc" | "z" | "gmt" => Ok(0),
        "local" => {
            // Get local timezone offset
            // In a no_std-compatible library, we can't reliably get local timezone
            // without platform-specific code or external dependencies.
            // Return UTC offset (0) as a fallback - users should use explicit timezone names
            let _ = timestamp; // Suppress unused warning
            Ok(0)
        }
        _ => {
            // Try to parse common IANA timezone abbreviations
            // This is a simplified subset - full IANA support would require a timezone database
            let offset = match zone.to_uppercase().as_str() {
                // US timezones
                "EST" => -5 * 3600,
                "EDT" => -4 * 3600,
                "CST" => -6 * 3600,
                "CDT" => -5 * 3600,
                "MST" => -7 * 3600,
                "MDT" => -6 * 3600,
                "PST" => -8 * 3600,
                "PDT" => -7 * 3600,
                "AKST" => -9 * 3600,
                "AKDT" => -8 * 3600,
                "HST" => -10 * 3600,
                // European timezones
                "WET" => 0,
                "WEST" => 3600,
                "CET" => 3600,
                "CEST" => 2 * 3600,
                "EET" => 2 * 3600,
                "EEST" => 3 * 3600,
                // Asian timezones
                "JST" => 9 * 3600,
                "KST" => 9 * 3600,
                "CST_CHINA" => 8 * 3600,
                "IST" => 5 * 3600 + 30 * 60, // India: +5:30
                // Australian timezones
                "AEST" => 10 * 3600,
                "AEDT" => 13600,
                "ACST" => 9 * 3600 + 30 * 60,
                "ACDT" => 10 * 3600 + 30 * 60,
                "AWST" => 8 * 3600,
                _ => {
                    // Try to parse IANA-style timezone names
                    // Common patterns: America/New_York, Europe/London, Asia/Tokyo
                    let offset = match zone {
                        // Americas
                        "America/New_York" | "US/Eastern" => {
                            // EDT/EST - simplified, always using standard time
                            if is_dst_us_eastern(timestamp) {
                                -4 * 3600
                            } else {
                                -5 * 3600
                            }
                        }
                        "America/Chicago" | "US/Central" => {
                            if is_dst_us_eastern(timestamp) {
                                -5 * 3600
                            } else {
                                -6 * 3600
                            }
                        }
                        "America/Denver" | "US/Mountain" => {
                            if is_dst_us_eastern(timestamp) {
                                -6 * 3600
                            } else {
                                -7 * 3600
                            }
                        }
                        "America/Los_Angeles" | "US/Pacific" => {
                            if is_dst_us_eastern(timestamp) {
                                -7 * 3600
                            } else {
                                -8 * 3600
                            }
                        }
                        "America/Anchorage" | "US/Alaska" => {
                            if is_dst_us_eastern(timestamp) {
                                -8 * 3600
                            } else {
                                -9 * 3600
                            }
                        }
                        "Pacific/Honolulu" | "US/Hawaii" => -10 * 3600,
                        // Europe
                        "Europe/London" | "GB" => {
                            if is_dst_europe(timestamp) {
                                3600
                            } else {
                                0
                            }
                        }
                        "Europe/Paris" | "Europe/Berlin" | "Europe/Rome" => {
                            if is_dst_europe(timestamp) {
                                2 * 3600
                            } else {
                                3600
                            }
                        }
                        "Europe/Moscow" => 3 * 3600,
                        // Asia
                        "Asia/Tokyo" | "Japan" => 9 * 3600,
                        "Asia/Seoul" | "ROK" => 9 * 3600,
                        "Asia/Shanghai" | "Asia/Hong_Kong" | "PRC" => 8 * 3600,
                        "Asia/Kolkata" | "Asia/Calcutta" => 5 * 3600 + 30 * 60,
                        "Asia/Dubai" => 4 * 3600,
                        "Asia/Singapore" => 8 * 3600,
                        // Australia
                        "Australia/Sydney" => {
                            if is_dst_australia(timestamp) {
                                13600
                            } else {
                                10 * 3600
                            }
                        }
                        "Australia/Melbourne" => {
                            if is_dst_australia(timestamp) {
                                13600
                            } else {
                                10 * 3600
                            }
                        }
                        "Australia/Perth" => 8 * 3600,
                        // UTC aliases
                        "Etc/UTC" | "Etc/GMT" | "UTC" | "GMT" => 0,
                        _ => {
                            // Try parsing numeric offset like "+05:30" or "-08:00"
                            if let Some(offset) = parse_numeric_offset(zone) {
                                return Ok(offset);
                            }
                            return Err(format!("unknown timezone: {zone}"));
                        }
                    };
                    return Ok(offset);
                }
            };
            Ok(offset)
        }
    }
}

/// Parse numeric timezone offset like "+05:30" or "-0800"
fn parse_numeric_offset(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (sign, rest) = match s.chars().next() {
        Some('+') => (1i64, &s[1..]),
        Some('-') => (-1i64, &s[1..]),
        _ => return None,
    };

    // Try HH:MM format
    if rest.len() == 5 && rest.chars().nth(2) == Some(':') {
        let hours: i64 = rest[..2].parse().ok()?;
        let mins: i64 = rest[3..].parse().ok()?;
        return Some(sign * (hours * 3600 + mins * 60));
    }

    // Try HHMM format
    if rest.len() == 4 && rest.chars().all(|c| c.is_ascii_digit()) {
        let hours: i64 = rest[..2].parse().ok()?;
        let mins: i64 = rest[2..].parse().ok()?;
        return Some(sign * (hours * 3600 + mins * 60));
    }

    // Try HH format
    if rest.len() == 2 && rest.chars().all(|c| c.is_ascii_digit()) {
        let hours: i64 = rest.parse().ok()?;
        return Some(sign * hours * 3600);
    }

    None
}

/// Simplified DST check for US Eastern timezone
/// DST starts 2nd Sunday of March, ends 1st Sunday of November (since 2007)
fn is_dst_us_eastern(timestamp: f64) -> bool {
    // Convert to days since epoch and calculate approximate month/day
    let secs = timestamp.trunc() as i64;
    let days = secs / 86400;

    // Get year, month, day from days since epoch
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as i32;

    // Approximate DST: March 8-14 to November 1-7 (simplified)
    // More accurate would check actual Sunday dates
    match month {
        3 => day >= 8,  // After ~2nd week of March
        4..=10 => true, // April through October
        11 => day < 7,  // First week of November
        _ => false,     // December through February
    }
}

/// Simplified DST check for European timezones
/// DST starts last Sunday of March, ends last Sunday of October
fn is_dst_europe(timestamp: f64) -> bool {
    let secs = timestamp.trunc() as i64;
    let days = secs / 86400;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as i32;

    match month {
        3 => day >= 25, // Last week of March
        4..=9 => true,  // April through September
        10 => day < 25, // Before last week of October
        _ => false,
    }
}

/// Simplified DST check for Australian Eastern timezones
/// DST starts first Sunday of October, ends first Sunday of April
fn is_dst_australia(timestamp: f64) -> bool {
    let secs = timestamp.trunc() as i64;
    let days = secs / 86400;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as i32;

    // Australian DST is opposite to Northern Hemisphere
    match month {
        10 => day >= 7,              // After first Sunday of October
        11 | 12 | 1 | 2 | 3 => true, // November through March
        4 => day < 7,                // Before first Sunday of April
        _ => false,
    }
}

/// Builtin: tz(zone) - convert Unix timestamp to datetime in specified timezone
fn builtin_tz<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    zone_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get the timestamp from input
    let timestamp = match get_float_value::<W>(&value, optional) {
        Ok(f) => f,
        Err(r) => return r,
    };

    // Evaluate the timezone expression to get the zone name
    let zone_str = match result_to_owned(eval_single::<W, S>(zone_expr, value.clone(), optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "tz zone argument")),
        Err(e) => return QueryResult::Error(e),
    };

    // Get the timezone offset
    let offset = match get_timezone_offset(&zone_str, timestamp) {
        Ok(o) => o,
        Err(_) if optional => return QueryResult::None,
        Err(e) => return QueryResult::Error(EvalError::new(e)),
    };

    // Format the datetime with the timezone offset
    let result = format_datetime_with_offset(timestamp, offset);
    QueryResult::Owned(OwnedValue::String(result))
}

// Phase 22: File operations (yq)

/// Builtin: load(file) - load external YAML/JSON file and return its parsed content
/// This function is only available with the "std" feature enabled.
#[cfg(feature = "std")]
fn builtin_load<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    file_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    use std::path::Path;

    // Evaluate the file expression to get the filename
    let filename = match result_to_owned(eval_single::<W, S>(file_expr, value, optional)) {
        Ok(OwnedValue::String(s)) => s,
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("string", "load filename")),
        Err(e) => return QueryResult::Error(e),
    };

    // Read the file contents
    let file_bytes = match std::fs::read(&filename) {
        Ok(bytes) => bytes,
        Err(_) if optional => {
            // In optional mode, return null for file errors
            return QueryResult::None;
        }
        Err(e) => {
            return QueryResult::Error(EvalError::new(format!(
                "load: failed to read file '{filename}': {e}"
            )));
        }
    };

    // Detect format from file extension
    let path = Path::new(&filename);
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));

    if is_json {
        // Parse as JSON
        let index = crate::json::JsonIndex::build(&file_bytes);
        let cursor = index.root(&file_bytes);
        QueryResult::Owned(to_owned(&cursor.value()))
    } else {
        // Parse as YAML (default)
        match crate::yaml::YamlIndex::build(&file_bytes) {
            Ok(index) => {
                let root = index.root(&file_bytes);
                // YAML documents are wrapped in a sequence at the root
                match root.value() {
                    crate::yaml::YamlValue::Sequence(docs) => {
                        // If single document, return it directly; otherwise return array
                        let doc_values: Vec<OwnedValue> =
                            docs.into_iter().map(|v| yaml_value_to_owned(v)).collect();
                        if doc_values.len() == 1 {
                            QueryResult::Owned(doc_values.into_iter().next().unwrap())
                        } else {
                            QueryResult::Owned(OwnedValue::Array(doc_values))
                        }
                    }
                    other => {
                        // Single value at root
                        QueryResult::Owned(yaml_value_to_owned(other))
                    }
                }
            }
            Err(_) if optional => QueryResult::None,
            Err(e) => QueryResult::Error(EvalError::new(format!(
                "load: failed to parse YAML file '{filename}': {e}"
            ))),
        }
    }
}

/// Convert a YAML value to OwnedValue (helper for load)
#[cfg(feature = "std")]
fn yaml_value_to_owned<W: Clone + AsRef<[u64]>>(
    value: crate::yaml::YamlValue<'_, W>,
) -> OwnedValue {
    use crate::yaml::{resolve_plain, ResolvedScalar, YamlValue};

    match value {
        YamlValue::Null => OwnedValue::Null,
        YamlValue::String(s) => {
            // Get the string value
            let str_value = match s.as_str() {
                Ok(cow) => cow.into_owned(),
                Err(_) => return OwnedValue::Null,
            };

            // Quoted strings are kept as strings
            if !s.is_unquoted() {
                return OwnedValue::String(str_value);
            }

            // Resolve plain scalars per the YAML 1.2 core schema
            match resolve_plain(&str_value) {
                ResolvedScalar::Null => OwnedValue::Null,
                ResolvedScalar::Bool(b) => OwnedValue::Bool(b),
                ResolvedScalar::Int(n) => OwnedValue::Int(n),
                ResolvedScalar::Float(f) => OwnedValue::Float(f),
                ResolvedScalar::Str => OwnedValue::String(str_value),
            }
        }
        YamlValue::Sequence(elements) => {
            let items: Vec<OwnedValue> = elements.into_iter().map(yaml_value_to_owned).collect();
            OwnedValue::Array(items)
        }
        YamlValue::Mapping(fields) => {
            let mut map = indexmap::IndexMap::new();
            for field in fields {
                let key = match field.key() {
                    YamlValue::String(s) => match s.as_str() {
                        Ok(cow) => cow.into_owned(),
                        Err(_) => continue,
                    },
                    other => {
                        // Non-string keys - convert to string representation
                        let v = yaml_value_to_owned(other);
                        v.to_json()
                    }
                };
                let value = yaml_value_to_owned(field.value());
                map.insert(key, value);
            }
            OwnedValue::Object(map)
        }
        YamlValue::Alias { target, .. } => {
            // Resolve alias by following the target cursor
            if let Some(target_cursor) = target {
                yaml_value_to_owned(target_cursor.value())
            } else {
                OwnedValue::Null
            }
        }
        YamlValue::Error(_) => OwnedValue::Null,
    }
}

/// Builtin: load(file) - stub for no_std builds (returns error)
#[cfg(not(feature = "std"))]
fn builtin_load<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    _file_expr: &Expr,
    _value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    if optional {
        QueryResult::None
    } else {
        QueryResult::Error(EvalError::new(
            "load() requires the 'std' feature to be enabled".to_string(),
        ))
    }
}

// Phase 17: Combinations

/// Builtin: combinations - generate all combinations from array of arrays
/// Input: [[1,2], [3,4]] -> outputs [1,3], [1,4], [2,3], [2,4]
fn builtin_combinations<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    // Input must be an array of arrays
    let arrays = match &value {
        StandardJson::Array(elements) => {
            let mut arrays: Vec<Vec<OwnedValue>> = Vec::new();
            for elem in *elements {
                match elem {
                    StandardJson::Array(inner) => {
                        let inner_values: Vec<OwnedValue> = inner.map(|v| to_owned(&v)).collect();
                        arrays.push(inner_values);
                    }
                    _ if optional => return QueryResult::None,
                    _ => {
                        return QueryResult::Error(EvalError::type_error("array", type_name(&elem)))
                    }
                }
            }
            arrays
        }
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    };

    // If any array is empty, return no results
    if arrays.iter().any(alloc::vec::Vec::is_empty) {
        return QueryResult::None;
    }

    // If no arrays, return empty array
    if arrays.is_empty() {
        return QueryResult::Owned(OwnedValue::Array(Vec::new()));
    }

    // Generate Cartesian product
    let results = cartesian_product(&arrays);
    QueryResult::ManyOwned(results)
}

/// Builtin: combinations(n) - generate n-way combinations (Cartesian product with itself n times)
/// Input with n=2: [1,2] -> outputs [1,1], [1,2], [2,1], [2,2]
fn builtin_combinations_n<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    n_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Get n
    let n = match result_to_owned(eval_single::<W, S>(n_expr, value.clone(), optional)) {
        Ok(OwnedValue::Int(i) | OwnedValue::NumberLiteral(NumberRepr::Int(i), _)) if i >= 0 => {
            i as usize
        }
        Ok(OwnedValue::Int(_) | OwnedValue::NumberLiteral(NumberRepr::Int(_), _)) if optional => {
            return QueryResult::None
        }
        Ok(OwnedValue::Int(_) | OwnedValue::NumberLiteral(NumberRepr::Int(_), _)) => {
            return QueryResult::Error(EvalError::new("combinations(n): n must be non-negative"))
        }
        Ok(_) if optional => return QueryResult::None,
        Ok(_) => return QueryResult::Error(EvalError::type_error("number", "n")),
        Err(e) => return QueryResult::Error(e),
    };

    // Input must be an array
    let base_array = match &value {
        StandardJson::Array(elements) => (*elements).map(|v| to_owned(&v)).collect::<Vec<_>>(),
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    };

    // n=0 returns single empty array
    if n == 0 {
        return QueryResult::Owned(OwnedValue::Array(Vec::new()));
    }

    // If base array is empty and n > 0, return no results
    if base_array.is_empty() {
        return QueryResult::None;
    }

    // Create n copies of the base array and compute Cartesian product
    let arrays: Vec<Vec<OwnedValue>> = (0..n).map(|_| base_array.clone()).collect();
    let results = cartesian_product(&arrays);
    QueryResult::ManyOwned(results)
}

/// Compute the Cartesian product of a list of arrays
fn cartesian_product(arrays: &[Vec<OwnedValue>]) -> Vec<OwnedValue> {
    if arrays.is_empty() {
        return vec![OwnedValue::Array(Vec::new())];
    }

    let mut results = Vec::new();
    let mut indices = vec![0usize; arrays.len()];

    loop {
        // Build current combination
        let combination: Vec<OwnedValue> = indices
            .iter()
            .enumerate()
            .map(|(i, &idx)| arrays[i][idx].clone())
            .collect();
        results.push(OwnedValue::Array(combination));

        // Increment indices (like counting in mixed radix)
        let mut carry = true;
        for i in (0..arrays.len()).rev() {
            if carry {
                indices[i] += 1;
                if indices[i] >= arrays[i].len() {
                    indices[i] = 0;
                } else {
                    carry = false;
                }
            }
        }

        // If we carried all the way through, we're done
        if carry {
            break;
        }
    }

    results
}

/// Builtin: builtins - list all builtin function names
fn builtin_builtins<'a, W: Clone + AsRef<[u64]>>() -> QueryResult<'a, W> {
    // Return a sorted array of all builtin function names with their arity
    let builtins = vec![
        // Type functions (arity 0)
        "type/0",
        "isnull/0",
        "isboolean/0",
        "isnumber/0",
        "isstring/0",
        "isarray/0",
        "isobject/0",
        // Type filters (arity 0)
        "values/0",
        "nulls/0",
        "booleans/0",
        "numbers/0",
        "strings/0",
        "arrays/0",
        "objects/0",
        "iterables/0",
        "scalars/0",
        "normals/0",
        "finites/0",
        // Length & keys (arity 0)
        "length/0",
        "utf8bytelength/0",
        "keys/0",
        "keys_unsorted/0",
        // has/in (arity 1)
        "has/1",
        "in/1",
        // Selection (arity 0-1)
        "select/1",
        "empty/0",
        // Map/Iteration (arity 1)
        "map/1",
        "map_values/1",
        // Reduction (arity 0-1)
        "add/0",
        "any/0",
        "all/0",
        "min/0",
        "max/0",
        "min_by/1",
        "max_by/1",
        // String functions (arity 0-1)
        "ascii_downcase/0",
        "ascii_upcase/0",
        "ltrimstr/1",
        "rtrimstr/1",
        "startswith/1",
        "endswith/1",
        "split/1",
        "join/1",
        "contains/1",
        "inside/1",
        "trim/0",
        "ltrim/0",
        "rtrim/0",
        // Array functions (arity 0-1)
        "first/0",
        "last/0",
        "nth/1",
        "reverse/0",
        "flatten/0",
        "flatten/1",
        "group_by/1",
        "unique/0",
        "unique_by/1",
        "sort/0",
        "sort_by/1",
        "transpose/0",
        "bsearch/1",
        // Object functions (arity 0-1)
        "to_entries/0",
        "from_entries/0",
        "with_entries/1",
        "pick/1",
        // Type conversions (arity 0)
        "tostring/0",
        "tonumber/0",
        "tojson/0",
        "fromjson/0",
        // String functions (arity 0-1)
        "explode/0",
        "implode/0",
        "test/1",
        "indices/1",
        "index/1",
        "rindex/1",
        "tojsonstream/0",
        "fromjsonstream/0",
        "tostream/0",
        "fromstream/1",
        "truncate_stream/1",
        // Path operations (arity 0-2)
        "path/1",
        "paths/0",
        "paths/1",
        "leaf_paths/0",
        "getpath/1",
        "setpath/2",
        "delpaths/1",
        "del/1",
        // Math functions (arity 0-2)
        "floor/0",
        "ceil/0",
        "round/0",
        "sqrt/0",
        "fabs/0",
        "abs/0",
        "log/0",
        "log10/0",
        "log2/0",
        "exp/0",
        "exp10/0",
        "exp2/0",
        "pow/2",
        "sin/0",
        "cos/0",
        "tan/0",
        "asin/0",
        "acos/0",
        "atan/0",
        "atan2/2",
        "sinh/0",
        "cosh/0",
        "tanh/0",
        "asinh/0",
        "acosh/0",
        "atanh/0",
        // Number classification (arity 0)
        "infinite/0",
        "nan/0",
        "isinfinite/0",
        "isnan/0",
        "isnormal/0",
        "isfinite/0",
        // Control flow (arity 1-2)
        "recurse/0",
        "recurse/1",
        "recurse/2",
        "walk/1",
        "isvalid/1",
        "limit/2",
        "skip/2",
        "first/1",
        "last/1",
        "nth/2",
        "until/2",
        "while/2",
        "repeat/1",
        "range/1",
        "range/2",
        "range/3",
        "reduce/3",
        "foreach/3",
        "foreach/4",
        // Debug (arity 0-1)
        "debug/0",
        "debug/1",
        // Environment (arity 0-1)
        "env/0",
        "env/1",
        "strenv/1",
        // Time (arity 0-1)
        "now/0",
        "gmtime/0",
        "localtime/0",
        "mktime/0",
        "strftime/1",
        "strptime/1",
        "todate/0",
        "fromdate/0",
        "todateiso8601/0",
        "fromdateiso8601/0",
        // Combinations (arity 0-1)
        "combinations/0",
        "combinations/1",
        // Additional math (arity 0)
        "trunc/0",
        // Type conversion (arity 0)
        "toboolean/0",
        // Meta (arity 0-1)
        "builtins/0",
        "modulemeta/1",
        // Error handling (arity 0-1)
        "error/0",
        "error/1",
        // YAML metadata (yq, arity 0)
        "tag/0",
        "anchor/0",
        "style/0",
        "kind/0",
        "key/0",
        "line/0",
        "column/0",
        "parent/0",
        "parent/1",
    ];

    let arr: Vec<OwnedValue> = builtins
        .iter()
        .map(|s| OwnedValue::String((*s).to_string()))
        .collect();

    QueryResult::Owned(OwnedValue::Array(arr))
}

/// Builtin: normals - select only normal numbers (not zero, infinite, NaN, or subnormal)
fn builtin_normals<W: Clone + AsRef<[u64]>>(value: StandardJson<'_, W>) -> QueryResult<'_, W> {
    if let StandardJson::Number(n) = &value {
        if let Ok(f) = n.as_f64() {
            if f.is_normal() {
                return QueryResult::One(value);
            }
        }
    }
    QueryResult::None
}

/// Builtin: finites - select only finite numbers (not infinite or NaN)
fn builtin_finites<W: Clone + AsRef<[u64]>>(value: StandardJson<'_, W>) -> QueryResult<'_, W> {
    if let StandardJson::Number(n) = &value {
        if let Ok(f) = n.as_f64() {
            if f.is_finite() {
                return QueryResult::One(value);
            }
        }
    }
    QueryResult::None
}

// Phase 13: Iteration control

/// Builtin: limit(n; expr) - output at most n values from expr
fn builtin_limit<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    n_expr: &Expr,
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate n
    let n_result = eval_single::<W, S>(n_expr, value.clone(), optional);
    let n = match n_result {
        QueryResult::One(v) => {
            if let StandardJson::Number(num) = v {
                num.as_i64().unwrap_or(0) as usize
            } else {
                return QueryResult::Error(EvalError::type_error("number", type_name(&v)));
            }
        }
        QueryResult::Owned(OwnedValue::Int(i)) => i as usize,
        QueryResult::Owned(OwnedValue::Float(f)) => f as usize,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::Error(EvalError::type_error("number", "null")),
    };

    if n == 0 {
        return QueryResult::None;
    }

    // Evaluate expr and take at most n results
    let result = eval_single::<W, S>(expr, value, optional);
    match result {
        QueryResult::One(v) => QueryResult::Owned(to_owned(&v)),
        QueryResult::OneCursor(c) => QueryResult::Owned(to_owned(&c.value())),
        QueryResult::Owned(v) => QueryResult::Owned(v),
        QueryResult::Many(results) => {
            let limited: Vec<OwnedValue> =
                results.into_iter().take(n).map(|v| to_owned(&v)).collect();
            if limited.is_empty() {
                QueryResult::None
            } else if limited.len() == 1 {
                QueryResult::Owned(limited.into_iter().next().unwrap())
            } else {
                QueryResult::ManyOwned(limited)
            }
        }
        QueryResult::ManyOwned(results) => {
            let limited: Vec<OwnedValue> = results.into_iter().take(n).collect();
            if limited.is_empty() {
                QueryResult::None
            } else if limited.len() == 1 {
                QueryResult::Owned(limited.into_iter().next().unwrap())
            } else {
                QueryResult::ManyOwned(limited)
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Builtin: first(expr) - output only the first value from expr (stream version)
fn builtin_first_stream<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single::<W, S>(expr, value, optional);
    match result {
        QueryResult::One(v) => QueryResult::Owned(to_owned(&v)),
        QueryResult::OneCursor(c) => QueryResult::Owned(to_owned(&c.value())),
        QueryResult::Owned(v) => QueryResult::Owned(v),
        QueryResult::Many(results) => {
            if let Some(first) = results.into_iter().next() {
                QueryResult::Owned(to_owned(&first))
            } else {
                QueryResult::None
            }
        }
        QueryResult::ManyOwned(results) => {
            if let Some(first) = results.into_iter().next() {
                QueryResult::Owned(first)
            } else {
                QueryResult::None
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Builtin: last(expr) - output only the last value from expr (stream version)
fn builtin_last_stream<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single::<W, S>(expr, value, optional);
    match result {
        QueryResult::One(v) => QueryResult::Owned(to_owned(&v)),
        QueryResult::OneCursor(c) => QueryResult::Owned(to_owned(&c.value())),
        QueryResult::Owned(v) => QueryResult::Owned(v),
        QueryResult::Many(results) => {
            if let Some(last) = results.into_iter().last() {
                QueryResult::Owned(to_owned(&last))
            } else {
                QueryResult::None
            }
        }
        QueryResult::ManyOwned(results) => {
            if let Some(last) = results.into_iter().last() {
                QueryResult::Owned(last)
            } else {
                QueryResult::None
            }
        }
        QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Builtin: nth(n; expr) - output only the nth value from expr (0-indexed, stream version)
fn builtin_nth_stream<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    n_expr: &Expr,
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate n
    let n_result = eval_single::<W, S>(n_expr, value.clone(), optional);
    let n = match n_result {
        QueryResult::One(v) => {
            if let StandardJson::Number(num) = v {
                num.as_i64().unwrap_or(0) as usize
            } else {
                return QueryResult::Error(EvalError::type_error("number", type_name(&v)));
            }
        }
        QueryResult::Owned(
            ref owned @ (OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..)),
        ) => owned.as_f64().unwrap_or(0.0) as usize,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::Error(EvalError::type_error("number", "null")),
    };

    // Evaluate expr and get the nth result
    let result = eval_single::<W, S>(expr, value, optional);
    match result {
        QueryResult::One(v) if n == 0 => QueryResult::Owned(to_owned(&v)),
        QueryResult::OneCursor(c) if n == 0 => QueryResult::Owned(to_owned(&c.value())),
        QueryResult::Owned(v) if n == 0 => QueryResult::Owned(v),
        QueryResult::Many(results) => {
            if let Some(nth) = results.into_iter().nth(n) {
                QueryResult::Owned(to_owned(&nth))
            } else {
                QueryResult::None
            }
        }
        QueryResult::ManyOwned(results) => {
            if let Some(nth) = results.into_iter().nth(n) {
                QueryResult::Owned(nth)
            } else {
                QueryResult::None
            }
        }
        QueryResult::One(_)
        | QueryResult::OneCursor(_)
        | QueryResult::Owned(_)
        | QueryResult::None => QueryResult::None,
        QueryResult::Error(e) => QueryResult::Error(e),
        QueryResult::Break(label) => QueryResult::Break(label),
    }
}

/// Builtin: isempty(expr) - returns true if expr produces no outputs
fn builtin_isempty<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let result = eval_single::<W, S>(expr, value, optional);
    let is_empty = match result {
        QueryResult::None => true,
        QueryResult::Many(ref v) if v.is_empty() => true,
        QueryResult::ManyOwned(ref v) if v.is_empty() => true,
        QueryResult::Error(_) => true, // Errors count as empty
        _ => false,
    };
    QueryResult::Owned(OwnedValue::Bool(is_empty))
}

/// Builtin: delpaths(paths) - delete multiple paths
fn builtin_delpaths<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    paths_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate paths expression
    let paths_result = eval_single::<W, S>(paths_expr, value.clone(), optional);
    let paths_owned = match paths_result {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::Error(EvalError::paths_must_be_array()),
    };

    let mut paths = match paths_owned {
        OwnedValue::Array(p) => p,
        _ => return QueryResult::Error(EvalError::paths_must_be_array()),
    };

    // Every entry must itself be an array before any deletion runs — jq
    // validates the whole list up front, so a bad entry anywhere refuses the
    // call rather than deleting the entries that sort ahead of it:
    // `delpaths([[0],"a"])` and `delpaths(["a",[0]])` both raise, neither
    // deletes `[0]` first.
    for path in &paths {
        if !matches!(path, OwnedValue::Array(_)) {
            return QueryResult::Error(EvalError::path_must_be_array_not(path.type_name()));
        }
    }

    // A NaN component is dropped with the path around it, because it names no
    // element at any depth — `resolve_read_index` refuses it, as jq's `jv_get`
    // does. Leaving it in would do more than waste a walk: since #421,
    // `compare_values` orders NaN as strictly less than every number,
    // including another NaN, so two NaN-headed paths would compare `Less`
    // than *each other* simultaneously — a comparator property `sort_by` is
    // entitled to panic on (`core::slice::sort`'s bidirectional-merge
    // consistency check). Filtering NaN out here keeps this particular
    // `sort_by` call entirely NaN-free, so it never risks that regardless.
    // jq cannot arbitrate either — 1.7.1 loops forever on `delpaths([[nan]])`
    // — so this is pinned by unit test rather than by a golden.
    paths.retain(|path| match path {
        OwnedValue::Array(components) => !components
            .iter()
            .any(|key| matches!(key, OwnedValue::Float(f) if f.is_nan())),
        _ => false,
    });

    // jq sorts the whole path list in its total value order — arrays compare
    // lexicographically — before deleting anything, so the order the caller
    // wrote them in is immaterial: `delpaths([[0],[2]])` and
    // `delpaths([[2],[0]])` are both `[20,40]` on `[10,20,30,40]`. Deleting
    // left to right instead let an earlier deletion shift the array under a
    // later path (#398).
    //
    // `compare_values` *is* that ordering, already shared with `sort` and
    // `unique`, so the paths stay wrapped as `OwnedValue::Array` through the
    // sort and it applies unchanged, rather than a second lexicographic
    // comparator growing here to drift from it. The sort has to be stable, as
    // jq's is: two paths can compare equal and still name different keys,
    // `[[1],[1.0]]` being the pair.
    paths.sort_by(compare_values);

    // Every survivor of the `retain` is an array, so this only re-borrows.
    let paths: Vec<&[OwnedValue]> = paths
        .iter()
        .filter_map(|path| match path {
            OwnedValue::Array(components) => Some(components.as_slice()),
            _ => None,
        })
        .collect();

    // The empty path names the document itself, and sorts before every other
    // array, so only the first entry needs checking: `delpaths([[],[0]])` and
    // `delpaths([[0],[]])` are both `null`.
    let result = match paths.first() {
        None => Ok(to_owned(&value)),
        Some([]) => Ok(OwnedValue::Null),
        Some(_) => delete_paths_sorted(to_owned(&value), &paths, 0),
    };
    match result {
        Ok(v) => QueryResult::Owned(v),
        Err(_) if optional => QueryResult::None,
        Err(e) => QueryResult::Error(e),
    }
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
fn builtin_floor<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Int(floor_f64(n) as i64)),
        Err(r) => r,
    }
}

/// Builtin: ceil
fn builtin_ceil<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Int(ceil_f64(n) as i64)),
        Err(r) => r,
    }
}

/// Builtin: round
fn builtin_round<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Int(round_f64(n) as i64)),
        Err(r) => r,
    }
}

/// Builtin: trunc - truncate toward zero
fn builtin_trunc<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Int(libm::trunc(n) as i64)),
        Err(r) => r,
    }
}

/// Builtin: sqrt
fn builtin_sqrt<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(sqrt_f64(n))),
        Err(r) => r,
    }
}

/// Builtin: fabs (absolute value)
fn builtin_fabs<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::fabs(n))),
        Err(r) => r,
    }
}

/// Builtin: log (natural logarithm)
fn builtin_log<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::log(n))),
        Err(r) => r,
    }
}

/// Builtin: log10
fn builtin_log10<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::log10(n))),
        Err(r) => r,
    }
}

/// Builtin: log2
fn builtin_log2<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::log2(n))),
        Err(r) => r,
    }
}

/// Builtin: exp (e^x)
fn builtin_exp<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::exp(n))),
        Err(r) => r,
    }
}

/// Builtin: exp10 (10^x)
fn builtin_exp10<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::pow(10.0, n))),
        Err(r) => r,
    }
}

/// Builtin: exp2 (2^x)
fn builtin_exp2<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
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
        QueryResult::Owned(v @ OwnedValue::NumberLiteral(..)) => v
            .as_f64()
            .ok_or_else(|| NumberError::Error(EvalError::new("invalid number"))),
        QueryResult::One(StandardJson::Number(n)) => n
            .as_f64()
            .map_err(|_| NumberError::Error(EvalError::new("invalid number"))),
        QueryResult::Error(e) => Err(NumberError::Error(e)),
        _ if optional => Err(NumberError::None),
        _ => Err(NumberError::Error(EvalError::new("expected number"))),
    }
}

/// Builtin: pow(base; exp) - power function
fn builtin_pow<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    base_expr: &Expr,
    exp_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let base = match get_number_from_result(
        eval_single::<W, S>(base_expr, value.clone(), optional),
        optional,
    ) {
        Ok(n) => n,
        Err(NumberError::None) => return QueryResult::None,
        Err(NumberError::Error(e)) => return QueryResult::Error(e),
    };

    let exp = match get_number_from_result(eval_single::<W, S>(exp_expr, value, optional), optional)
    {
        Ok(n) => n,
        Err(NumberError::None) => return QueryResult::None,
        Err(NumberError::Error(e)) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Float(libm::pow(base, exp)))
}

// Trigonometric functions

/// Builtin: sin
fn builtin_sin<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::sin(n))),
        Err(r) => r,
    }
}

/// Builtin: cos
fn builtin_cos<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::cos(n))),
        Err(r) => r,
    }
}

/// Builtin: tan
fn builtin_tan<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::tan(n))),
        Err(r) => r,
    }
}

/// Builtin: asin
fn builtin_asin<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::asin(n))),
        Err(r) => r,
    }
}

/// Builtin: acos
fn builtin_acos<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::acos(n))),
        Err(r) => r,
    }
}

/// Builtin: atan
fn builtin_atan<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::atan(n))),
        Err(r) => r,
    }
}

/// Builtin: atan2(y; x)
fn builtin_atan2<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    y_expr: &Expr,
    x_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    let y = match get_number_from_result(
        eval_single::<W, S>(y_expr, value.clone(), optional),
        optional,
    ) {
        Ok(n) => n,
        Err(NumberError::None) => return QueryResult::None,
        Err(NumberError::Error(e)) => return QueryResult::Error(e),
    };

    let x = match get_number_from_result(eval_single::<W, S>(x_expr, value, optional), optional) {
        Ok(n) => n,
        Err(NumberError::None) => return QueryResult::None,
        Err(NumberError::Error(e)) => return QueryResult::Error(e),
    };

    QueryResult::Owned(OwnedValue::Float(libm::atan2(y, x)))
}

// Hyperbolic functions

/// Builtin: sinh
fn builtin_sinh<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::sinh(n))),
        Err(r) => r,
    }
}

/// Builtin: cosh
fn builtin_cosh<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::cosh(n))),
        Err(r) => r,
    }
}

/// Builtin: tanh
fn builtin_tanh<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::tanh(n))),
        Err(r) => r,
    }
}

/// Builtin: asinh
fn builtin_asinh<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::asinh(n))),
        Err(r) => r,
    }
}

/// Builtin: acosh
fn builtin_acosh<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::acosh(n))),
        Err(r) => r,
    }
}

/// Builtin: atanh
fn builtin_atanh<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Float(libm::atanh(n))),
        Err(r) => r,
    }
}

// Number Classification
// Note: is_infinite(), is_nan(), is_normal(), is_finite() are available on f64 in no_std

/// Builtin: isinfinite
fn builtin_isinfinite<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
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
fn builtin_isnan<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
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
fn builtin_isnormal<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
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
fn builtin_isfinite<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match get_float_value::<W>(&value, optional) {
        Ok(n) => QueryResult::Owned(OwnedValue::Bool(n.is_finite())),
        Err(_) => QueryResult::Owned(OwnedValue::Bool(false)),
    }
}

// Debug functions

/// Builtin: debug - output value to stderr, pass through unchanged
fn builtin_debug<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
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

/// $ENV expression - returns object of all environment variables
#[cfg(feature = "std")]
fn eval_env<'a, W: Clone + AsRef<[u64]>>(_optional: bool) -> QueryResult<'a, W> {
    let mut env_obj = IndexMap::new();
    for (key, value) in std::env::vars() {
        env_obj.insert(key, OwnedValue::String(value));
    }
    QueryResult::Owned(OwnedValue::Object(env_obj))
}

#[cfg(not(feature = "std"))]
fn eval_env<'a, W: Clone + AsRef<[u64]>>(_optional: bool) -> QueryResult<'a, W> {
    // Return empty object in no_std context
    QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
}

/// Builtin: env - object of all environment variables
#[cfg(feature = "std")]
fn builtin_env<W: Clone + AsRef<[u64]>>(
    _value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
    let mut env_obj = IndexMap::new();
    for (key, value) in std::env::vars() {
        env_obj.insert(key, OwnedValue::String(value));
    }
    QueryResult::Owned(OwnedValue::Object(env_obj))
}

#[cfg(not(feature = "std"))]
fn builtin_env<'a, W: Clone + AsRef<[u64]>>(
    _value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // Return empty object in no_std context
    QueryResult::Owned(OwnedValue::Object(IndexMap::new()))
}

/// Builtin: env.VAR or $ENV.VAR - get environment variable (expression-based)
#[cfg(feature = "std")]
fn builtin_envvar<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    var: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the expression to get the variable name
    let owned_value = to_owned(&value);
    let var_result = eval_owned_expr::<S>(var, &owned_value, optional);
    let var_name = match var_result {
        Ok(OwnedValue::String(s)) => s,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::new("env variable name must be a string")),
    };

    match std::env::var(&var_name) {
        Ok(val) => QueryResult::Owned(OwnedValue::String(val)),
        Err(_) if optional => QueryResult::None,
        Err(_) => QueryResult::Owned(OwnedValue::Null),
    }
}

#[cfg(not(feature = "std"))]
fn builtin_envvar<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    _var: &Expr,
    _value: StandardJson<'a, W>,
    _optional: bool,
) -> QueryResult<'a, W> {
    // Return null in no_std context
    QueryResult::Owned(OwnedValue::Null)
}

/// Builtin: env(VAR_NAME) - get environment variable by literal name (yq syntax)
#[cfg(feature = "std")]
fn builtin_env_object<'a, W: Clone + AsRef<[u64]>>(
    name: &str,
    optional: bool,
) -> QueryResult<'a, W> {
    match std::env::var(name) {
        Ok(val) => QueryResult::Owned(OwnedValue::String(val)),
        Err(_) if optional => QueryResult::None,
        Err(_) => QueryResult::Error(EvalError::new(format!(
            "value for env variable '{name}' not provided in env()"
        ))),
    }
}

#[cfg(not(feature = "std"))]
fn builtin_env_object<'a, W: Clone + AsRef<[u64]>>(
    name: &str,
    optional: bool,
) -> QueryResult<'a, W> {
    if optional {
        QueryResult::None
    } else {
        QueryResult::Error(EvalError::new(format!(
            "value for env variable '{}' not provided in env() (no_std)",
            name
        )))
    }
}

/// Builtin: strenv(VAR_NAME) - get environment variable as string (yq syntax)
/// This is the same as env() but explicitly for strings
#[cfg(feature = "std")]
fn builtin_strenv<'a, W: Clone + AsRef<[u64]>>(name: &str, optional: bool) -> QueryResult<'a, W> {
    match std::env::var(name) {
        Ok(val) => QueryResult::Owned(OwnedValue::String(val)),
        Err(_) if optional => QueryResult::None,
        Err(_) => QueryResult::Error(EvalError::new(format!(
            "value for env variable '{name}' not provided in strenv()"
        ))),
    }
}

#[cfg(not(feature = "std"))]
fn builtin_strenv<'a, W: Clone + AsRef<[u64]>>(name: &str, optional: bool) -> QueryResult<'a, W> {
    if optional {
        QueryResult::None
    } else {
        QueryResult::Error(EvalError::new(format!(
            "value for env variable '{}' not provided in strenv() (no_std)",
            name
        )))
    }
}

// String functions

/// Builtin: trim - remove leading/trailing whitespace
fn builtin_trim<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
fn builtin_ltrim<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
fn builtin_rtrim<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
fn builtin_transpose<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
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
    let max_len = inner_arrays
        .iter()
        .map(alloc::vec::Vec::len)
        .max()
        .unwrap_or(0);

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

/// Builtin: bsearch(x) - binary search for x in sorted array
fn builtin_bsearch<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    x_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // jq's `bsearch($target)` desugars to `target as $target | if length == 0
    // then -1 ...`, which evaluates `target` before ever looking at the input's
    // shape — a `target`-side error (or `empty`) wins over the checks below.
    let x = match eval_single::<W, S>(x_expr, value.clone(), optional) {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::Owned(v) => v,
        QueryResult::Error(e) => return QueryResult::Error(e),
        _ => return QueryResult::None,
    };

    let elements_iter = match value {
        StandardJson::Array(a) => a,
        // `null | length` is `0` in jq, so `null` takes the same `length == 0`
        // branch as `[]` and answers "not found" rather than erroring (#420).
        // It is the only non-array for which this applies: every other
        // non-array's `length` itself errors in jq, matching the guard below.
        StandardJson::Null => return QueryResult::Owned(OwnedValue::Int(-1)),
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::new("bsearch requires array")),
    };

    // Collect array elements
    let elements: Vec<OwnedValue> = elements_iter.map(|v| to_owned(&v)).collect();

    // jq's search from `builtin.jq`: an inclusive `hi` and a
    // `floor((lo + hi) / 2)` midpoint, so the probe sequence — and with it which
    // of several equal elements gets reported — is jq's.
    //
    // The absent case reaches jq's answer by a shorter route. `builtin.jq`
    // re-probes after its loop and returns `-2 - start` or `-1 - start`
    // depending on that probe; each of its three exit paths reduces to
    // `-1 - (count of elements below the target)`, and `hi` has settled one
    // below that count, so `-2 - hi` is the same number without the re-probe.
    //
    // `Vec::binary_search_by` is not a substitute. It documents that "any one of
    // the matches could be returned", and it does in fact land elsewhere than jq
    // among equal elements: on rustc 1.97.0 `[1,1] | bsearch(1)` is 0 in jq and
    // 1 there, `[1,1,1]` is 1 in jq and 2 there, `[1,1,1,1]` is 1 in jq and 3
    // there. Depending on an explicitly unspecified choice would let a std
    // change silently break oracle parity.
    //
    // The two arms jq special-cases before its loop need no special case here:
    // an empty array leaves `hi` at -1 and yields -1, and a one-element array
    // reduces to a single probe.
    let mut lo: i64 = 0;
    let mut hi: i64 = elements.len() as i64 - 1;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        // `compare_values` is the one comparator for jq's total order — the same
        // one `sort` uses, so the two cannot disagree about a pair. The private
        // copy that used to live here lacked `(Array, Array)` and
        // `(Object, Object)` arms, so every pair of containers compared Equal
        // and `bsearch` reported absent values as found (#384).
        match compare_values(&elements[mid as usize], &x) {
            core::cmp::Ordering::Equal => return QueryResult::Owned(OwnedValue::Int(mid)),
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Greater => hi = mid - 1,
        }
    }

    // Absent: jq returns the negated insertion point, not an object. `hi` has
    // settled one below the insertion point, so `-2 - hi` is `-1 - insertion`.
    QueryResult::Owned(OwnedValue::Int(-2 - hi))
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

/// Builtin: pick(keys) - select only specified keys from object/array (yq)
fn builtin_pick<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    keys_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the keys expression to get the array of keys
    let keys_owned = match eval_single::<W, S>(keys_expr, value.clone(), optional) {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::OneCursor(c) => to_owned(&c.value()),
        QueryResult::Owned(v) => v,
        QueryResult::ManyOwned(v) if !v.is_empty() => v.into_iter().next().unwrap(),
        QueryResult::ManyOwned(_) => {
            return QueryResult::Error(EvalError::new("pick: keys expression produced no output"))
        }
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
        QueryResult::None if optional => return QueryResult::None,
        QueryResult::None => {
            return QueryResult::Error(EvalError::new("pick: keys expression produced no output"))
        }
        QueryResult::Many(v) if !v.is_empty() => to_owned(&v.into_iter().next().unwrap()),
        QueryResult::Many(_) => {
            return QueryResult::Error(EvalError::new("pick: keys expression produced no output"))
        }
    };

    // Keys must be an array
    let keys = match &keys_owned {
        OwnedValue::Array(arr) => arr,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::new("pick: argument must be an array of keys")),
    };

    match &value {
        StandardJson::Object(fields) => {
            // For objects, pick specified string keys
            let mut result = IndexMap::new();
            for key in keys {
                if let OwnedValue::String(k) = key {
                    // Find the field in the object
                    for field in *fields {
                        if let StandardJson::String(key_str) = field.key() {
                            if let Ok(cow) = key_str.as_str() {
                                if cow.as_ref() == k.as_str() {
                                    result.insert(k.clone(), to_owned(&field.value()));
                                    break;
                                }
                            }
                        }
                    }
                    // If key not found, yq silently skips it
                }
            }
            QueryResult::Owned(OwnedValue::Object(result))
        }
        StandardJson::Array(elements) => {
            // For arrays, pick specified indices
            let arr: Vec<_> = (*elements).collect();
            let len = arr.len() as i64;
            let mut result = Vec::new();

            for key in keys {
                let idx = match key {
                    OwnedValue::Int(i) => *i,
                    OwnedValue::Float(f) => *f as i64,
                    OwnedValue::NumberLiteral(NumberRepr::Int(i), _) => *i,
                    OwnedValue::NumberLiteral(NumberRepr::Float(f), _) => *f as i64,
                    _ => continue, // Skip non-numeric indices
                };

                // Handle negative indices
                let actual_idx = if idx < 0 { len + idx } else { idx };

                if actual_idx >= 0 && actual_idx < len {
                    result.push(to_owned(&arr[actual_idx as usize]));
                }
                // If index out of bounds, yq silently skips it
            }
            QueryResult::Owned(OwnedValue::Array(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new("pick: input must be an object or array")),
    }
}

/// Builtin: omit(keys) - remove specified keys from object/indices from array
/// Inverse of `pick`: keeps all keys/indices except those specified.
fn builtin_omit<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    keys_expr: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the keys expression to get the array of keys to omit
    let keys_owned = match eval_single::<W, S>(keys_expr, value.clone(), optional) {
        QueryResult::One(v) => to_owned(&v),
        QueryResult::OneCursor(c) => to_owned(&c.value()),
        QueryResult::Owned(v) => v,
        QueryResult::ManyOwned(v) if !v.is_empty() => v.into_iter().next().unwrap(),
        QueryResult::ManyOwned(_) => {
            return QueryResult::Error(EvalError::new("omit: keys expression produced no output"))
        }
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
        QueryResult::None if optional => return QueryResult::None,
        QueryResult::None => {
            return QueryResult::Error(EvalError::new("omit: keys expression produced no output"))
        }
        QueryResult::Many(v) if !v.is_empty() => to_owned(&v.into_iter().next().unwrap()),
        QueryResult::Many(_) => {
            return QueryResult::Error(EvalError::new("omit: keys expression produced no output"))
        }
    };

    // Keys must be an array
    let keys = match &keys_owned {
        OwnedValue::Array(arr) => arr,
        _ if optional => return QueryResult::None,
        _ => return QueryResult::Error(EvalError::new("omit: argument must be an array of keys")),
    };

    match &value {
        StandardJson::Object(fields) => {
            // For objects, keep all keys except those in the omit list
            // Use a Vec for no_std compatibility (typical omit lists are small)
            let omit_keys: Vec<&str> = keys
                .iter()
                .filter_map(|k| {
                    if let OwnedValue::String(s) = k {
                        Some(s.as_str())
                    } else {
                        None
                    }
                })
                .collect();

            let mut result = IndexMap::new();
            for field in *fields {
                if let StandardJson::String(key_str) = field.key() {
                    if let Ok(cow) = key_str.as_str() {
                        let key = cow.as_ref();
                        if !omit_keys.contains(&key) {
                            result.insert(key.to_string(), to_owned(&field.value()));
                        }
                    }
                }
            }
            QueryResult::Owned(OwnedValue::Object(result))
        }
        StandardJson::Array(elements) => {
            // For arrays, keep all indices except those in the omit list
            let arr: Vec<_> = (*elements).collect();
            let len = arr.len() as i64;

            // Use a Vec for no_std compatibility (typical omit lists are small)
            let omit_indices: Vec<usize> = keys
                .iter()
                .filter_map(|k| {
                    let idx = match k {
                        OwnedValue::Int(i) => *i,
                        OwnedValue::Float(f) => *f as i64,
                        OwnedValue::NumberLiteral(NumberRepr::Int(i), _) => *i,
                        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) => *f as i64,
                        _ => return None,
                    };
                    // Handle negative indices
                    let actual_idx = if idx < 0 { len + idx } else { idx };
                    if actual_idx >= 0 && actual_idx < len {
                        Some(actual_idx as usize)
                    } else {
                        None // Out of bounds indices are ignored
                    }
                })
                .collect();

            let result: Vec<OwnedValue> = arr
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    if omit_indices.contains(&i) {
                        None
                    } else {
                        Some(to_owned(v))
                    }
                })
                .collect();

            QueryResult::Owned(OwnedValue::Array(result))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::new("omit: input must be an object or array")),
    }
}

// Builtin: tag - return YAML type tag (!!str, !!int, !!map, etc.)
// Since we evaluate on JSON/OwnedValue (not raw YAML), we derive the tag from the JSON type.
fn builtin_tag<W: Clone + AsRef<[u64]>>(value: StandardJson<'_, W>) -> QueryResult<'_, W> {
    let tag = match &value {
        StandardJson::Null => "!!null",
        StandardJson::Bool(_) => "!!bool",
        StandardJson::Number(n) => {
            // Distinguish int from float
            if n.as_i64().is_ok() {
                "!!int"
            } else {
                "!!float"
            }
        }
        StandardJson::String(_) => "!!str",
        StandardJson::Array(_) => "!!seq",
        StandardJson::Object(_) => "!!map",
        StandardJson::Error(_) => "!!null",
    };
    QueryResult::Owned(OwnedValue::String(tag.to_string()))
}

// Builtin: anchor - return anchor name if present
// Since YAML metadata is lost during conversion to OwnedValue, this always returns empty string.
// In a full yq implementation, this would require tracking anchor metadata through the pipeline.
fn builtin_anchor<'a, W: Clone + AsRef<[u64]>>() -> QueryResult<'a, W> {
    // Currently YAML anchors are not preserved through the OwnedValue conversion.
    // Return empty string to match yq behavior for values without anchors.
    QueryResult::Owned(OwnedValue::String(String::new()))
}

// Builtin: style - return scalar/collection style
// Since YAML style metadata is lost during conversion to OwnedValue, this returns
// reasonable defaults based on the JSON structure.
fn builtin_style<W: Clone + AsRef<[u64]>>(value: StandardJson<'_, W>) -> QueryResult<'_, W> {
    let style = match &value {
        // Collections: yq returns "flow" for flow-style, empty for block-style
        // Since we lose this info, we return empty string (block-style is more common)
        StandardJson::Array(_) | StandardJson::Object(_) => "",
        // Scalars: yq returns "double", "single", "literal", "folded", or empty for plain
        // Since we lose quote info, we return empty string (plain scalar)
        _ => "",
    };
    QueryResult::Owned(OwnedValue::String(style.to_string()))
}

/// `kind` - returns the node kind: "scalar", "seq", or "map"
/// This is a yq function that returns the YAML node kind.
fn builtin_kind<W: Clone + AsRef<[u64]>>(value: StandardJson<'_, W>) -> QueryResult<'_, W> {
    let kind = match &value {
        StandardJson::Array(_) => "seq",
        StandardJson::Object(_) => "map",
        // All other types are scalars: null, bool, number, string
        _ => "scalar",
    };
    QueryResult::Owned(OwnedValue::String(kind.to_string()))
}

/// `line` - returns the 1-based line number of the current node (yq)
/// Since YAML position metadata is lost during conversion to OwnedValue, this returns 0.
/// In a full yq implementation, this would require tracking source positions through the pipeline.
fn builtin_line<'a, W: Clone + AsRef<[u64]>>() -> QueryResult<'a, W> {
    // Currently source positions are not preserved through the OwnedValue conversion.
    // Return 0 to indicate position is unknown (yq returns 1 for actual positions).
    QueryResult::Owned(OwnedValue::Int(0))
}

/// `column` - returns the 1-based column number of the current node (yq)
/// Since YAML position metadata is lost during conversion to OwnedValue, this returns 0.
/// In a full yq implementation, this would require tracking source positions through the pipeline.
fn builtin_column<'a, W: Clone + AsRef<[u64]>>() -> QueryResult<'a, W> {
    // Currently source positions are not preserved through the OwnedValue conversion.
    // Return 0 to indicate position is unknown (yq returns 1 for actual positions).
    QueryResult::Owned(OwnedValue::Int(0))
}

/// `document_index` / `di` - returns the 0-indexed document position in multi-doc stream (yq)
/// Since document context is lost during conversion to OwnedValue for JSON processing,
/// this returns 0. The actual document index is preserved through the generic evaluator
/// when processing YAML directly.
fn builtin_document_index<'a, W: Clone + AsRef<[u64]>>() -> QueryResult<'a, W> {
    // For JSON input or when document context is lost, return 0 (single document assumed).
    // The generic evaluator handles this properly for YAML with cursor metadata.
    QueryResult::Owned(OwnedValue::Int(0))
}

/// `shuffle` - randomly shuffle array elements (yq)
/// Uses non-cryptographic RNG for performance.
#[cfg(feature = "cli")]
fn builtin_shuffle<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    match value {
        StandardJson::Array(elements) => {
            let mut items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();
            // Use a seeded RNG for reproducibility in tests if needed,
            // but seed from system entropy for actual randomness
            let mut rng = ChaCha8Rng::from_os_rng();
            items.shuffle(&mut rng);
            QueryResult::Owned(OwnedValue::Array(items))
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// `shuffle` - fallback when cli feature is not enabled
#[cfg(not(feature = "cli"))]
fn builtin_shuffle<W: Clone + AsRef<[u64]>>(
    _value: StandardJson<'_, W>,
    _optional: bool,
) -> QueryResult<'_, W> {
    QueryResult::Error(EvalError::new(
        "shuffle requires the 'cli' feature to be enabled",
    ))
}

/// `pivot` - transpose arrays/objects (yq)
///
/// For array of arrays: transposes rows/columns
///   [[a, b], [x, y]] | pivot  → [[a, x], [b, y]]
///
/// For array of objects: collects values by key
///   [{name: "Alice", age: 30}, {name: "Bob", age: 25}] | pivot
///   → {name: ["Alice", "Bob"], age: [30, 25]}
///
/// Handles missing keys with null padding.
fn builtin_pivot<W: Clone + AsRef<[u64]>>(
    value: StandardJson<'_, W>,
    optional: bool,
) -> QueryResult<'_, W> {
    match value {
        StandardJson::Array(elements) => {
            let items: Vec<OwnedValue> = elements.map(|e| to_owned(&e)).collect();

            if items.is_empty() {
                // Empty array pivots to empty array
                return QueryResult::Owned(OwnedValue::Array(vec![]));
            }

            // Check if all elements are arrays (array-of-arrays case)
            let all_arrays = items.iter().all(|v| matches!(v, OwnedValue::Array(_)));
            // Check if all elements are objects (array-of-objects case)
            let all_objects = items.iter().all(|v| matches!(v, OwnedValue::Object(_)));

            if all_arrays {
                // Transpose array of arrays
                pivot_arrays::<W>(&items)
            } else if all_objects {
                // Transpose array of objects
                pivot_objects::<W>(&items)
            } else {
                // Mixed or unsupported types
                if optional {
                    QueryResult::None
                } else {
                    QueryResult::Error(EvalError::new(
                        "pivot requires array of arrays or array of objects",
                    ))
                }
            }
        }
        _ if optional => QueryResult::None,
        _ => QueryResult::Error(EvalError::type_error("array", type_name(&value))),
    }
}

/// Transpose array of arrays: [[a, b], [x, y]] → [[a, x], [b, y]]
fn pivot_arrays<'a, W: Clone + AsRef<[u64]>>(items: &[OwnedValue]) -> QueryResult<'a, W> {
    // Get the maximum row length
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
        return QueryResult::Owned(OwnedValue::Array(vec![]));
    }

    // Build transposed array
    let mut result = Vec::with_capacity(max_len);
    for col_idx in 0..max_len {
        let mut column = Vec::with_capacity(items.len());
        for item in items {
            if let OwnedValue::Array(arr) = item {
                // Get element at col_idx, or null if missing
                column.push(arr.get(col_idx).cloned().unwrap_or(OwnedValue::Null));
            } else {
                column.push(OwnedValue::Null);
            }
        }
        result.push(OwnedValue::Array(column));
    }

    QueryResult::Owned(OwnedValue::Array(result))
}

/// Transpose array of objects: [{a: 1}, {a: 2, b: 3}] → {a: [1, 2], b: [null, 3]}
fn pivot_objects<'a, W: Clone + AsRef<[u64]>>(items: &[OwnedValue]) -> QueryResult<'a, W> {
    // Collect all unique keys in order of first appearance
    let mut all_keys: Vec<String> = Vec::new();
    for item in items {
        if let OwnedValue::Object(obj) = item {
            for key in obj.keys() {
                if !all_keys.contains(key) {
                    all_keys.push(key.clone());
                }
            }
        }
    }

    // Build result object with arrays for each key
    let mut result = IndexMap::new();
    for key in &all_keys {
        let mut values = Vec::with_capacity(items.len());
        for item in items {
            if let OwnedValue::Object(obj) = item {
                values.push(obj.get(key).cloned().unwrap_or(OwnedValue::Null));
            } else {
                values.push(OwnedValue::Null);
            }
        }
        result.insert(key.clone(), OwnedValue::Array(values));
    }

    QueryResult::Owned(OwnedValue::Object(result))
}

// ============================================================================
// Phase 9: Variables & Definitions
// ============================================================================

/// Evaluate destructuring pattern binding: `expr as {key: $var, ...} | body`.
fn eval_as_pattern<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    expr: &Expr,
    pattern: &Pattern,
    body: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Evaluate the expression to get the value to destructure
    let bound_result = eval_single::<W, S>(expr, value.clone(), optional);

    let bound_values: Vec<OwnedValue> = match bound_result.materialize_cursor() {
        QueryResult::One(v) => vec![to_owned(&v)],
        QueryResult::OneCursor(_) => unreachable!(),
        QueryResult::Many(vs) => vs.iter().map(to_owned).collect(),
        QueryResult::Owned(v) => vec![v],
        QueryResult::ManyOwned(vs) => vs,
        QueryResult::None => return QueryResult::None,
        QueryResult::Error(e) => return QueryResult::Error(e),
        QueryResult::Break(label) => return QueryResult::Break(label),
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

        match eval_single::<W, S>(&substituted_body, value.clone(), optional).materialize_cursor() {
            QueryResult::One(v) => all_results.push(to_owned(&v)),
            QueryResult::OneCursor(_) => unreachable!(),
            QueryResult::Many(vs) => all_results.extend(vs.iter().map(to_owned)),
            QueryResult::Owned(v) => all_results.push(v),
            QueryResult::ManyOwned(vs) => all_results.extend(vs),
            QueryResult::None => {}
            QueryResult::Error(e) => return QueryResult::Error(e),
            QueryResult::Break(label) => return QueryResult::Break(label),
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
            let mut bindings = Vec::new();
            for entry in entries {
                // jq destructures by indexing once per key, so a non-object
                // reports exactly what `.<key>` would — and a pattern with no
                // keys never indexes, so it cannot fail.
                let obj = match value {
                    OwnedValue::Object(o) => o,
                    _ => {
                        return Err(EvalError::cannot_index_with_field(
                            owned_type_name(value),
                            &entry.key,
                        ))
                    }
                };
                let field_value = obj.get(&entry.key).cloned().unwrap_or(OwnedValue::Null);
                let sub_bindings = extract_pattern_bindings(&entry.pattern, &field_value)?;
                bindings.extend(sub_bindings);
            }
            Ok(bindings)
        }
        Pattern::Array(patterns) => {
            let mut bindings = Vec::new();
            for (i, pat) in patterns.iter().enumerate() {
                // As above: one index per element position, so the error is
                // the one `.[i]` would raise.
                let arr = match value {
                    OwnedValue::Array(a) => a,
                    _ => {
                        return Err(EvalError::cannot_index_with_type(
                            owned_type_name(value),
                            "number",
                        ))
                    }
                };
                let elem_value = arr.get(i).cloned().unwrap_or(OwnedValue::Null);
                let sub_bindings = extract_pattern_bindings(pat, &elem_value)?;
                bindings.extend(sub_bindings);
            }
            Ok(bindings)
        }
    }
}

/// Evaluate function definition: `def name(params): body; then`.
///
/// In jq, function definitions are scoped - the function is available in `then`.
/// We implement this by substituting function calls with the body (with args substituted).
fn eval_func_def<'a, W: Clone + AsRef<[u64]>, S: EvalSemantics>(
    name: &str,
    params: &[String],
    body: &Expr,
    then: &Expr,
    value: StandardJson<'a, W>,
    optional: bool,
) -> QueryResult<'a, W> {
    // Substitute all calls to this function in `then` with the body
    let expanded_then = expand_func_calls(then, name, params, body);
    eval_single::<W, S>(&expanded_then, value, optional)
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
        Expr::IndexExpr { target, key } => Expr::IndexExpr {
            target: Box::new(expand_func_calls(target, func_name, params, body)),
            key: Box::new(expand_func_calls(key, func_name, params, body)),
        },
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
        Expr::Format(f) => Expr::Format(f.clone()),
        Expr::Var(v) => Expr::Var(v.clone()),
        Expr::Loc { line } => Expr::Loc { line: *line },
        Expr::Env => Expr::Env,
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
        Expr::Assign { path, value } => Expr::Assign {
            path: Box::new(expand_func_calls(path, func_name, params, body)),
            value: Box::new(expand_func_calls(value, func_name, params, body)),
        },
        Expr::Update { path, filter } => Expr::Update {
            path: Box::new(expand_func_calls(path, func_name, params, body)),
            filter: Box::new(expand_func_calls(filter, func_name, params, body)),
        },
        Expr::CompoundAssign { op, path, value } => Expr::CompoundAssign {
            op: *op,
            path: Box::new(expand_func_calls(path, func_name, params, body)),
            value: Box::new(expand_func_calls(value, func_name, params, body)),
        },
        Expr::AlternativeAssign { path, value } => Expr::AlternativeAssign {
            path: Box::new(expand_func_calls(path, func_name, params, body)),
            value: Box::new(expand_func_calls(value, func_name, params, body)),
        },

        // Label-break
        Expr::Label { name, body: lbody } => Expr::Label {
            name: name.clone(),
            body: Box::new(expand_func_calls(lbody, func_name, params, body)),
        },
        Expr::Break(name) => Expr::Break(name.clone()),
    }
}

/// Substitute a function parameter with an argument expression.
fn substitute_func_param(expr: &Expr, param: &str, arg: &Expr) -> Expr {
    match expr {
        // A variable reference to the parameter becomes the argument expression
        Expr::Var(name) if name == param => arg.clone(),
        Expr::Var(_) => expr.clone(),
        Expr::Loc { line } => Expr::Loc { line: *line },
        Expr::Env => Expr::Env,
        Expr::Identity => Expr::Identity,
        Expr::Field(name) => Expr::Field(name.clone()),
        Expr::Index(i) => Expr::Index(*i),
        Expr::Slice { start, end } => Expr::Slice {
            start: *start,
            end: *end,
        },
        Expr::Iterate => Expr::Iterate,
        Expr::IndexExpr { target, key } => Expr::IndexExpr {
            target: Box::new(substitute_func_param(target, param, arg)),
            key: Box::new(substitute_func_param(key, param, arg)),
        },
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
        Expr::Format(f) => Expr::Format(f.clone()),
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
        Expr::Assign { path, value } => Expr::Assign {
            path: Box::new(substitute_func_param(path, param, arg)),
            value: Box::new(substitute_func_param(value, param, arg)),
        },
        Expr::Update { path, filter } => Expr::Update {
            path: Box::new(substitute_func_param(path, param, arg)),
            filter: Box::new(substitute_func_param(filter, param, arg)),
        },
        Expr::CompoundAssign { op, path, value } => Expr::CompoundAssign {
            op: *op,
            path: Box::new(substitute_func_param(path, param, arg)),
            value: Box::new(substitute_func_param(value, param, arg)),
        },
        Expr::AlternativeAssign { path, value } => Expr::AlternativeAssign {
            path: Box::new(substitute_func_param(path, param, arg)),
            value: Box::new(substitute_func_param(value, param, arg)),
        },

        // Label-break
        Expr::Label { name, body } => Expr::Label {
            name: name.clone(),
            body: Box::new(substitute_func_param(body, param, arg)),
        },
        Expr::Break(name) => Expr::Break(name.clone()),
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
        Builtin::Values => Builtin::Values,
        Builtin::Nulls => Builtin::Nulls,
        Builtin::Booleans => Builtin::Booleans,
        Builtin::Numbers => Builtin::Numbers,
        Builtin::Strings => Builtin::Strings,
        Builtin::Arrays => Builtin::Arrays,
        Builtin::Objects => Builtin::Objects,
        Builtin::Iterables => Builtin::Iterables,
        Builtin::Scalars => Builtin::Scalars,
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
        Builtin::ToJson => Builtin::ToJson,
        Builtin::FromJson => Builtin::FromJson,
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
        Builtin::ToStream => Builtin::ToStream,
        Builtin::FromStream(e) => {
            Builtin::FromStream(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::TruncateStream(e) => {
            Builtin::TruncateStream(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::GetPath(e) => {
            Builtin::GetPath(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        // Phase 16: Regex Functions
        Builtin::TestFlags(re, flags) => Builtin::TestFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
        ),
        Builtin::Match(re) => {
            Builtin::Match(Box::new(expand_func_calls(re, func_name, params, body)))
        }
        Builtin::MatchFlags(re, flags) => Builtin::MatchFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
        ),
        Builtin::Capture(e) => {
            Builtin::Capture(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::CaptureFlags(re, flags) => Builtin::CaptureFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
        ),
        Builtin::Sub(re, repl) => Builtin::Sub(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(repl, func_name, params, body)),
        ),
        Builtin::SubFlags(re, repl, flags) => Builtin::SubFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(repl, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
        ),
        Builtin::Gsub(re, repl) => Builtin::Gsub(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(repl, func_name, params, body)),
        ),
        Builtin::GsubFlags(re, repl, flags) => Builtin::GsubFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(repl, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
        ),
        Builtin::Scan(re) => {
            Builtin::Scan(Box::new(expand_func_calls(re, func_name, params, body)))
        }
        Builtin::ScanFlags(re, flags) => Builtin::ScanFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
        ),
        Builtin::SplitRegex(re, flags) => Builtin::SplitRegex(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
        ),
        Builtin::Splits(re) => {
            Builtin::Splits(Box::new(expand_func_calls(re, func_name, params, body)))
        }
        Builtin::SplitsFlags(re, flags) => Builtin::SplitsFlags(
            Box::new(expand_func_calls(re, func_name, params, body)),
            Box::new(expand_func_calls(flags, func_name, params, body)),
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
        Builtin::PathNoArg => Builtin::PathNoArg,
        Builtin::Parent => Builtin::Parent,
        Builtin::ParentN(e) => {
            Builtin::ParentN(Box::new(expand_func_calls(e, func_name, params, body)))
        }
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
        Builtin::EnvObject(name) => Builtin::EnvObject(name.clone()),
        Builtin::StrEnv(name) => Builtin::StrEnv(name.clone()),
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
        Builtin::Pick(e) => Builtin::Pick(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::Omit(e) => Builtin::Omit(Box::new(expand_func_calls(e, func_name, params, body))),
        Builtin::Tag => Builtin::Tag,
        Builtin::Anchor => Builtin::Anchor,
        Builtin::Style => Builtin::Style,
        Builtin::Kind => Builtin::Kind,
        Builtin::Key => Builtin::Key,
        Builtin::Line => Builtin::Line,
        Builtin::Column => Builtin::Column,
        Builtin::DocumentIndex => Builtin::DocumentIndex,
        Builtin::Shuffle => Builtin::Shuffle,
        Builtin::Pivot => Builtin::Pivot,
        Builtin::SplitDoc => Builtin::SplitDoc,
        Builtin::Del(e) => Builtin::Del(Box::new(expand_func_calls(e, func_name, params, body))),
        // Phase 12 builtins (no args to expand)
        Builtin::Now => Builtin::Now,
        Builtin::Abs => Builtin::Abs,
        Builtin::Builtins => Builtin::Builtins,
        Builtin::Normals => Builtin::Normals,
        Builtin::Finites => Builtin::Finites,
        // Phase 13: Iteration control
        Builtin::Limit(n, e) => Builtin::Limit(
            Box::new(expand_func_calls(n, func_name, params, body)),
            Box::new(expand_func_calls(e, func_name, params, body)),
        ),
        Builtin::FirstStream(e) => {
            Builtin::FirstStream(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::LastStream(e) => {
            Builtin::LastStream(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::NthStream(n, e) => Builtin::NthStream(
            Box::new(expand_func_calls(n, func_name, params, body)),
            Box::new(expand_func_calls(e, func_name, params, body)),
        ),
        Builtin::IsEmpty(e) => {
            Builtin::IsEmpty(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        // Phase 14: Recursive traversal (extends Phase 8)
        Builtin::RecurseDown => Builtin::RecurseDown,
        // Phase 15: Date/Time functions
        Builtin::Gmtime => Builtin::Gmtime,
        Builtin::Localtime => Builtin::Localtime,
        Builtin::Mktime => Builtin::Mktime,
        Builtin::Strftime(e) => {
            Builtin::Strftime(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Strptime(e) => {
            Builtin::Strptime(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::Todate => Builtin::Todate,
        Builtin::Fromdate => Builtin::Fromdate,
        Builtin::Todateiso8601 => Builtin::Todateiso8601,
        Builtin::Fromdateiso8601 => Builtin::Fromdateiso8601,

        // Phase 17: Combinations
        Builtin::Combinations => Builtin::Combinations,
        Builtin::CombinationsN(e) => {
            Builtin::CombinationsN(Box::new(expand_func_calls(e, func_name, params, body)))
        }

        // Phase 18: Additional math functions
        Builtin::Trunc => Builtin::Trunc,

        // Phase 19: Type conversion
        Builtin::ToBoolean => Builtin::ToBoolean,

        // Phase 20: Iteration control extension
        Builtin::Skip(n, e) => Builtin::Skip(
            Box::new(expand_func_calls(n, func_name, params, body)),
            Box::new(expand_func_calls(e, func_name, params, body)),
        ),

        // Phase 21: Extended Date/Time functions (yq)
        Builtin::FromUnix => Builtin::FromUnix,
        Builtin::ToUnix => Builtin::ToUnix,
        Builtin::Tz(e) => Builtin::Tz(Box::new(expand_func_calls(e, func_name, params, body))),

        // Phase 22: File operations (yq)
        Builtin::Load(e) => Builtin::Load(Box::new(expand_func_calls(e, func_name, params, body))),

        // Phase 23: Position-based navigation (succinctly extension)
        Builtin::AtOffset(e) => {
            Builtin::AtOffset(Box::new(expand_func_calls(e, func_name, params, body)))
        }
        Builtin::AtPosition(line, col) => Builtin::AtPosition(
            Box::new(expand_func_calls(line, func_name, params, body)),
            Box::new(expand_func_calls(col, func_name, params, body)),
        ),
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
        Builtin::Values => Builtin::Values,
        Builtin::Nulls => Builtin::Nulls,
        Builtin::Booleans => Builtin::Booleans,
        Builtin::Numbers => Builtin::Numbers,
        Builtin::Strings => Builtin::Strings,
        Builtin::Arrays => Builtin::Arrays,
        Builtin::Objects => Builtin::Objects,
        Builtin::Iterables => Builtin::Iterables,
        Builtin::Scalars => Builtin::Scalars,
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
        Builtin::ToJson => Builtin::ToJson,
        Builtin::FromJson => Builtin::FromJson,
        Builtin::Explode => Builtin::Explode,
        Builtin::Implode => Builtin::Implode,
        Builtin::Test(e) => Builtin::Test(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Indices(e) => Builtin::Indices(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Index(e) => Builtin::Index(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Rindex(e) => Builtin::Rindex(Box::new(substitute_func_param(e, param, arg))),
        Builtin::ToJsonStream => Builtin::ToJsonStream,
        Builtin::FromJsonStream => Builtin::FromJsonStream,
        Builtin::ToStream => Builtin::ToStream,
        Builtin::FromStream(e) => {
            Builtin::FromStream(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::TruncateStream(e) => {
            Builtin::TruncateStream(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::GetPath(e) => Builtin::GetPath(Box::new(substitute_func_param(e, param, arg))),
        // Phase 16: Regex Functions
        Builtin::TestFlags(re, flags) => Builtin::TestFlags(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
        ),
        Builtin::Match(re) => Builtin::Match(Box::new(substitute_func_param(re, param, arg))),
        Builtin::MatchFlags(re, flags) => Builtin::MatchFlags(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
        ),
        Builtin::Capture(e) => Builtin::Capture(Box::new(substitute_func_param(e, param, arg))),
        Builtin::CaptureFlags(re, flags) => Builtin::CaptureFlags(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
        ),
        Builtin::Sub(re, repl) => Builtin::Sub(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(repl, param, arg)),
        ),
        Builtin::SubFlags(re, repl, flags) => Builtin::SubFlags(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(repl, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
        ),
        Builtin::Gsub(re, repl) => Builtin::Gsub(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(repl, param, arg)),
        ),
        Builtin::GsubFlags(re, repl, flags) => Builtin::GsubFlags(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(repl, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
        ),
        Builtin::Scan(re) => Builtin::Scan(Box::new(substitute_func_param(re, param, arg))),
        Builtin::ScanFlags(re, flags) => Builtin::ScanFlags(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
        ),
        Builtin::SplitRegex(re, flags) => Builtin::SplitRegex(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
        ),
        Builtin::Splits(re) => Builtin::Splits(Box::new(substitute_func_param(re, param, arg))),
        Builtin::SplitsFlags(re, flags) => Builtin::SplitsFlags(
            Box::new(substitute_func_param(re, param, arg)),
            Box::new(substitute_func_param(flags, param, arg)),
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
        Builtin::PathNoArg => Builtin::PathNoArg,
        Builtin::Parent => Builtin::Parent,
        Builtin::ParentN(e) => Builtin::ParentN(Box::new(substitute_func_param(e, param, arg))),
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
        Builtin::EnvObject(name) => Builtin::EnvObject(name.clone()),
        Builtin::StrEnv(name) => Builtin::StrEnv(name.clone()),
        Builtin::NullLit => Builtin::NullLit,
        Builtin::Trim => Builtin::Trim,
        Builtin::Ltrim => Builtin::Ltrim,
        Builtin::Rtrim => Builtin::Rtrim,
        Builtin::Transpose => Builtin::Transpose,
        Builtin::BSearch(e) => Builtin::BSearch(Box::new(substitute_func_param(e, param, arg))),
        Builtin::ModuleMeta(e) => {
            Builtin::ModuleMeta(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::Pick(e) => Builtin::Pick(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Omit(e) => Builtin::Omit(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Tag => Builtin::Tag,
        Builtin::Anchor => Builtin::Anchor,
        Builtin::Style => Builtin::Style,
        Builtin::Kind => Builtin::Kind,
        Builtin::Key => Builtin::Key,
        Builtin::Line => Builtin::Line,
        Builtin::Column => Builtin::Column,
        Builtin::DocumentIndex => Builtin::DocumentIndex,
        Builtin::Shuffle => Builtin::Shuffle,
        Builtin::Pivot => Builtin::Pivot,
        Builtin::SplitDoc => Builtin::SplitDoc,
        Builtin::Del(e) => Builtin::Del(Box::new(substitute_func_param(e, param, arg))),
        // Phase 12 builtins (no args to substitute)
        Builtin::Now => Builtin::Now,
        Builtin::Abs => Builtin::Abs,
        Builtin::Builtins => Builtin::Builtins,
        Builtin::Normals => Builtin::Normals,
        Builtin::Finites => Builtin::Finites,
        // Phase 13: Iteration control
        Builtin::Limit(n, e) => Builtin::Limit(
            Box::new(substitute_func_param(n, param, arg)),
            Box::new(substitute_func_param(e, param, arg)),
        ),
        Builtin::FirstStream(e) => {
            Builtin::FirstStream(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::LastStream(e) => {
            Builtin::LastStream(Box::new(substitute_func_param(e, param, arg)))
        }
        Builtin::NthStream(n, e) => Builtin::NthStream(
            Box::new(substitute_func_param(n, param, arg)),
            Box::new(substitute_func_param(e, param, arg)),
        ),
        Builtin::IsEmpty(e) => Builtin::IsEmpty(Box::new(substitute_func_param(e, param, arg))),
        // Phase 14: Recursive traversal (extends Phase 8)
        Builtin::RecurseDown => Builtin::RecurseDown,
        // Phase 15: Date/Time functions
        Builtin::Gmtime => Builtin::Gmtime,
        Builtin::Localtime => Builtin::Localtime,
        Builtin::Mktime => Builtin::Mktime,
        Builtin::Strftime(e) => Builtin::Strftime(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Strptime(e) => Builtin::Strptime(Box::new(substitute_func_param(e, param, arg))),
        Builtin::Todate => Builtin::Todate,
        Builtin::Fromdate => Builtin::Fromdate,
        Builtin::Todateiso8601 => Builtin::Todateiso8601,
        Builtin::Fromdateiso8601 => Builtin::Fromdateiso8601,

        // Phase 17: Combinations
        Builtin::Combinations => Builtin::Combinations,
        Builtin::CombinationsN(e) => {
            Builtin::CombinationsN(Box::new(substitute_func_param(e, param, arg)))
        }

        // Phase 18: Additional math functions
        Builtin::Trunc => Builtin::Trunc,

        // Phase 19: Type conversion
        Builtin::ToBoolean => Builtin::ToBoolean,

        // Phase 20: Iteration control extension
        Builtin::Skip(n, e) => Builtin::Skip(
            Box::new(substitute_func_param(n, param, arg)),
            Box::new(substitute_func_param(e, param, arg)),
        ),

        // Phase 21: Extended Date/Time functions (yq)
        Builtin::FromUnix => Builtin::FromUnix,
        Builtin::ToUnix => Builtin::ToUnix,
        Builtin::Tz(e) => Builtin::Tz(Box::new(substitute_func_param(e, param, arg))),

        // Phase 22: File operations (yq)
        Builtin::Load(e) => Builtin::Load(Box::new(substitute_func_param(e, param, arg))),

        // Phase 23: Position-based navigation (succinctly extension)
        Builtin::AtOffset(e) => Builtin::AtOffset(Box::new(substitute_func_param(e, param, arg))),
        Builtin::AtPosition(line, col) => Builtin::AtPosition(
            Box::new(substitute_func_param(line, param, arg)),
            Box::new(substitute_func_param(col, param, arg)),
        ),
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
    QueryResult::Error(EvalError::new(format!("undefined function: {name}")))
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
            match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                $pattern $(if $guard)? => $body,
                other => panic!("unexpected result: {:?}", other),
            }
        }};
    }

    /// Every output of `filter` on `json`, rendered as compact JSON.
    ///
    /// Multi-output expectations read better as a `Vec` comparison than as a
    /// `QueryResult` variant match, and this deliberately does not care whether
    /// the stream came back borrowed (`Many`) or owned (`ManyOwned`). Use
    /// `query!` when the variant itself is the thing under test.
    fn outputs(json: &[u8], filter: &str) -> Vec<String> {
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let expr = parse(filter).unwrap();
        eval::<Vec<u64>, JqSemantics>(&expr, cursor)
            .collect_owned()
            .iter()
            .map(OwnedValue::to_json)
            .collect()
    }

    #[test]
    fn test_numeric_key_to_index_number_literal_387() {
        assert_eq!(
            numeric_key_to_index(&OwnedValue::from_number_literal("2")),
            Some(2)
        );
        // Truncates toward zero, same as plain Float.
        assert_eq!(
            numeric_key_to_index(&OwnedValue::from_number_literal("1.7")),
            Some(1)
        );
    }

    #[test]
    fn test_substitute_vars_number_literal_degrades_to_plain_literal_387() {
        // `substitute_vars` (the `--arg`/`--argjson`-style public API) inlines
        // each variable's value into the AST via `owned_to_expr`. Unlike a
        // document-sourced value flowing through a builtin argument,
        // `Expr::Literal` has no source-text slot (#387's documented scope
        // boundary -- see `owned_to_expr`'s doc comment), so a `NumberLiteral`
        // substituted in degrades to its plain parsed value: the substituted
        // filter's own re-evaluation loses jq's canonical spelling and falls
        // back to Rust's `f64`/`i64` Display, same as any other computed
        // (non-passthrough) number.
        let expr = parse("$n").unwrap();

        let int_lit = OwnedValue::from_number_literal("42");
        let substituted = substitute_vars(&expr, [("n", &int_lit)]);
        assert_eq!(substituted, Expr::Literal(Literal::Int(42)));

        let float_lit = OwnedValue::from_number_literal("1e100");
        let substituted = substitute_vars(&expr, [("n", &float_lit)]);
        assert_eq!(substituted, Expr::Literal(Literal::Float(1e100)));

        let index = JsonIndex::build(b"null");
        let cursor = index.root(b"null");
        assert_eq!(
            eval::<Vec<u64>, JqSemantics>(&substituted, cursor)
                .collect_owned()
                .iter()
                .map(OwnedValue::to_json)
                .collect::<Vec<_>>(),
            // Not jq's "1E+100" -- see the doc comment above.
            ["1e100".parse::<f64>().unwrap().to_string()]
        );
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
        // jq returns null for missing fields on objects (not an error)
        query!(br#"{"name": "Alice"}"#, ".missing",
            QueryResult::One(StandardJson::Null) => {}
        );

        // Optional also returns null for missing fields on objects
        query!(br#"{"name": "Alice"}"#, ".missing?",
            QueryResult::One(StandardJson::Null) => {}
        );
    }

    #[test]
    fn test_missing_field_iteration() {
        // Issue #61: When iterating over objects, missing fields should return null
        // This matches jq behavior: [.[].status_code] returns [404, null, null] not [404]
        query!(
            br#"[{"status_code": 404}, {"msg": "no status"}, {"msg": "also none"}]"#,
            "[.[].status_code]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(404));
                assert_eq!(arr[1], OwnedValue::Null);
                assert_eq!(arr[2], OwnedValue::Null);
            }
        );
    }

    #[test]
    fn test_field_on_non_object() {
        // Accessing .field on non-object is an error (unlike missing field on object)
        query!(br"123", ".field",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot index number with string \"field\"");
            }
        );

        // But with optional, it returns nothing (not null)
        query!(br"123", ".field?",
            QueryResult::None => {}
        );

        // For null input, jq returns null (not error)
        query!(br"null", ".field",
            QueryResult::One(StandardJson::Null) => {}
        );

        query!(br"null", ".field?",
            QueryResult::One(StandardJson::Null) => {}
        );
    }

    #[test]
    fn test_nested_missing_field() {
        // Nested field access where intermediate field exists but final doesn't
        query!(br#"{"a": {"b": 1}}"#, ".a.missing",
            QueryResult::One(StandardJson::Null) => {}
        );

        // Nested field where intermediate exists but is null - jq returns null
        query!(br#"{"a": null}"#, ".a.missing",
            QueryResult::One(StandardJson::Null) => {}
        );

        // With optional on null, also returns null
        query!(br#"{"a": null}"#, ".a.missing?",
            QueryResult::One(StandardJson::Null) => {}
        );
    }

    #[test]
    fn test_array_index() {
        query!(br"[10, 20, 30]", ".[0]",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 10);
            }
        );

        query!(br"[10, 20, 30]", ".[2]",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 30);
            }
        );

        // Negative index
        query!(br"[10, 20, 30]", ".[-1]",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 30);
            }
        );
    }

    #[test]
    fn test_iterate() {
        query!(br"[1, 2, 3]", ".[]",
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
                    other => panic!("unexpected: {other:?}"),
                }
            }
        );
    }

    #[test]
    fn test_slice() {
        // jq array slicing yields a single sub-array, not a stream of elements.
        query!(br"[0, 1, 2, 3, 4, 5]", ".[1:4]",
            QueryResult::Owned(OwnedValue::Array(values)) => {
                assert_eq!(
                    values,
                    vec![OwnedValue::Int(1), OwnedValue::Int(2), OwnedValue::Int(3)]
                );
            }
        );

        query!(br"[0, 1, 2, 3, 4, 5]", ".[2:]",
            QueryResult::Owned(OwnedValue::Array(values)) => {
                assert_eq!(
                    values,
                    vec![
                        OwnedValue::Int(2),
                        OwnedValue::Int(3),
                        OwnedValue::Int(4),
                        OwnedValue::Int(5)
                    ]
                );
            }
        );

        query!(br"[0, 1, 2, 3, 4, 5]", ".[:2]",
            QueryResult::Owned(OwnedValue::Array(values)) => {
                assert_eq!(values, vec![OwnedValue::Int(0), OwnedValue::Int(1)]);
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
        query!(br"{}", "null",
            QueryResult::Owned(OwnedValue::Null) => {}
        );

        query!(br"{}", "true",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br"{}", "42",
            QueryResult::Owned(OwnedValue::Int(42)) => {}
        );

        query!(br"{}", "\"hello\"",
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
        query!(br"{}", "[]",
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
        query!(br"{}", "{}",
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
    fn test_arithmetic_mod_float_truncates() {
        // jq truncates both operands to integers: 10.5 % 3 == 1
        query!(br#"{"a": 10.5, "b": 3}"#, ".a % .b",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );

        // Float % Float: 10.9 % 3.9 == 10 % 3 == 1
        query!(br#"{"a": 10.9, "b": 3.9}"#, ".a % .b",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );

        // Truncation is toward zero, not floor: -7.5 % 2 == -7 % 2 == -1
        query!(br#"{"a": -7.5, "b": 2}"#, ".a % .b",
            QueryResult::Owned(OwnedValue::Int(-1)) => {}
        );
    }

    #[test]
    fn test_arithmetic_mod_float_divisor_truncates_to_zero() {
        // jq: 5 % 0.5 errors because the divisor truncates to 0
        query!(br#"{"a": 5, "b": 0.5}"#, ".a % .b",
            QueryResult::Error(_) => {}
        );
    }

    #[test]
    fn test_arithmetic_mod_nan_and_infinite() {
        // jq: a NaN operand yields NaN (serialized as null), not an error
        query!(b"null", "nan % 2",
            QueryResult::Owned(OwnedValue::Float(n)) if n.is_nan() => {}
        );
        query!(b"null", "2 % nan",
            QueryResult::Owned(OwnedValue::Float(n)) if n.is_nan() => {}
        );

        // jq: infinite saturates to i64::MAX, so infinite % 3 == 1
        query!(b"null", "infinite % 3",
            QueryResult::Owned(OwnedValue::Int(1)) => {}
        );
    }

    #[test]
    fn test_arithmetic_precedence() {
        // 2 + 3 * 4 = 2 + 12 = 14
        query!(br"{}", "2 + 3 * 4",
            QueryResult::Owned(OwnedValue::Int(14)) => {}
        );

        // (2 + 3) * 4 = 5 * 4 = 20
        query!(br"{}", "(2 + 3) * 4",
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
    fn test_comparison_objects() {
        // jq compares objects by [sorted keys] first, then values in
        // sorted-key order. All expected values verified against real jq.
        query!(b"null", r#"{"a":1} < {"a":2}"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(b"null", r#"{"a":1} < {"b":1}"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        // Key arrays decide before any values: ["a","b"] < ["a","c"] even
        // though the value at the shared key "a" compares Greater.
        query!(b"null", r#"{"a":2,"b":1} < {"a":1,"c":9}"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        // Equal key sets fall through to values in sorted-key order.
        query!(b"null", r#"{"a":1,"b":2} < {"a":1,"b":3}"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        // A key array that is a strict prefix compares Less.
        query!(b"null", r#"{"a":1} < {"a":1,"b":2}"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        // Insertion order is irrelevant; these objects are equal.
        query!(b"null", r#"{"b":1,"a":2} <= {"a":2,"b":1}"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(b"null", r#"{"a":1,"b":2} < {"a":1}"#,
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
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
        query!(br"true", ". | not",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        query!(br"false", ". | not",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        query!(br"null", ". | not",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        // Numbers are truthy
        query!(br"0", ". | not",
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
        query!(br"{}", ".missing? // \"default\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "default" => {}
        );

        // Chain alternatives
        query!(br#"{"a": null, "b": null}"#, ".a // .b // 42",
            QueryResult::Owned(OwnedValue::Int(42)) => {}
        );
    }

    #[test]
    fn regression_issue_377_alternative_propagates_left_hand_errors() {
        // jq 1.7.1 raises a left-hand error rather than treating it as falsy;
        // `//` only substitutes for false/null/absent output.
        query!(br"null", r#"error("x") // 3"#,
            QueryResult::Error(e) => {
                assert_eq!(e.message, "x");
            }
        );
        query!(br"1", ".a // 3",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot index number with string \"a\"");
            }
        );

        // A `?` on the erroring operand still suppresses the error and falls
        // through to the right side, since `.a?` never produces `Error` in
        // the first place.
        query!(br"1", ".a? // 3",
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );
    }

    #[test]
    fn regression_issue_160_alternative_emits_every_truthy_output() {
        // `//` is a filter over the whole left stream, not a test of its first
        // output. Before #160 the truthiness check read `vs.first()` and then
        // returned the left stream *unfiltered*, so these gave "backup", 3 and
        // `1 false 2` respectively.
        assert_eq!(
            outputs(b"null", r#"(false,1,null,2) // "backup""#),
            ["1", "2"]
        );
        assert_eq!(outputs(b"null", "(null,1) // 3"), ["1"]);
        assert_eq!(outputs(b"null", "(1,false,2) // 3"), ["1", "2"]);
    }

    #[test]
    fn regression_issue_160_alternative_does_not_filter_its_right_side() {
        // Only the left operand is filtered.
        assert_eq!(outputs(b"null", "false // (null,7)"), ["null", "7"]);

        // Which is exactly what makes the left-associative chain filter the
        // middle operand: `((null,false) // (null,5)) // 6` emits `null, 5`
        // from the inner `//`, and the outer one keeps only the 5.
        assert_eq!(outputs(b"null", "(null,false) // (null,5) // 6"), ["5"]);
    }

    #[test]
    fn regression_issue_160_alternative_keeps_a_borrowed_stream_borrowed() {
        // Document-derived values must survive the filter without being
        // promoted to owned, so `//` keeps the zero-copy path.
        query!(br#"{"a": [1, false, 2]}"#, ".a[] // 9",
            QueryResult::Many(vs) => {
                assert_eq!(vs.len(), 2, "expected both truthy elements to survive");
            }
        );

        // A single survivor normalizes back to `One`, so callers cannot tell a
        // filtered stream from a value that was single all along.
        query!(br#"{"a": [1, false]}"#, ".a[] // 9",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    #[test]
    fn regression_issue_160_alternative_with_empty_operands() {
        assert_eq!(outputs(b"null", "empty // 9"), ["9"]);

        // Nothing truthy on the left and nothing at all on the right: no output.
        query!(b"null", "false // empty", QueryResult::None => {});
    }

    #[test]
    fn regression_issue_160_boolean_operators_are_cartesian() {
        // jq loops the left operand outermost and short-circuits per output, so
        // the trailing `false` of `(true,false) and _` contributes a single
        // `false` while the leading `true` fans out over the right operand.
        assert_eq!(
            outputs(b"null", "(true,false) and (true,false)"),
            ["true", "false", "false"]
        );
        assert_eq!(
            outputs(b"null", "(true,false) or (true,false)"),
            ["true", "true", "false"]
        );
        assert_eq!(
            outputs(b"null", "(false,true) and (1,2)"),
            ["false", "true", "true"]
        );
    }

    #[test]
    fn regression_issue_160_boolean_short_circuit_skips_the_right_operand() {
        // A short-circuiting left output must not evaluate the right operand at
        // all, so the error never surfaces.
        query!(b"null", r#"false and error("x")"#,
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(b"null", r#"true or error("x")"#,
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn regression_issue_160_boolean_with_empty_operand_yields_nothing() {
        // These used to be `Error("no value")` / `Error("empty result")`,
        // because each operand was funnelled through `result_to_owned`.
        query!(b"null", "empty and true", QueryResult::None => {});
        query!(b"null", "true and empty", QueryResult::None => {});
        query!(b"null", "false or empty", QueryResult::None => {});
    }

    #[test]
    fn regression_issue_160_boolean_propagates_break() {
        // `result_to_owned` turned a `Break` into
        // `Error("break $out not in label")`; it now reaches its label.
        query!(b"null", "label $out | ((break $out) and true)",
            QueryResult::None => {}
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
        // try with catch on missing field - no error to catch, returns null
        // (jq returns null for missing fields on objects, not an error)
        query!(br"{}", "try .missing catch \"default\"",
            QueryResult::One(StandardJson::Null) => {}
        );

        // try without catch on missing field - returns null
        query!(br"{}", "try .missing",
            QueryResult::One(StandardJson::Null) => {}
        );

        // try with catch on actual error (field access on number) - catch is triggered
        query!(br"123", "try .foo catch \"default\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "default" => {}
        );

        // try without catch on actual error - error is suppressed (returns None)
        query!(br"123", "try .foo",
            QueryResult::None => {}
        );

        // try with null catch on actual error
        query!(br"123", "try .foo catch null",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_try_catch_optional() {
        // Optional on missing field returns null, not None
        query!(br"{}", "try .missing? catch \"default\"",
            QueryResult::One(StandardJson::Null) => {}
        );

        // Optional on actual error (field on number) - optional suppresses error
        query!(br"123", "try .foo? catch \"default\"",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_error_basic() {
        // Bare `error` raises the input value, as jq does: `{} | error`
        // reports `{}`, not `null`.
        query!(br"{}", "error",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "{}");
                assert_eq!(e.value, Some(OwnedValue::Object(IndexMap::new())));
            }
        );

        // error with string message
        query!(br"{}", "error(\"something went wrong\")",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "something went wrong");
                assert_eq!(
                    e.value,
                    Some(OwnedValue::String("something went wrong".into()))
                );
            }
        );

        // error with number message (message is serialized, payload is not)
        query!(br"{}", "error(42)",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "42");
                assert_eq!(e.value, Some(OwnedValue::Int(42)));
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
        query!(br"{}", "try error(\"oops\") catch \"caught\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "caught" => {}
        );

        // try without catch - error is suppressed
        query!(br"{}", "try error(\"oops\")",
            QueryResult::None => {}
        );
    }

    /// The catch handler's input is the *raised error value*, not the input the
    /// `try` was evaluated against — regression test for #158, where the error
    /// was discarded and `catch` ran on the original input.
    #[test]
    fn test_catch_binds_the_error_value() {
        // String payload: the handler sees "boom", not the input {"x":1}.
        query!(br#"{"x":1}"#, "try error(\"boom\") catch .",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "boom");
            }
        );

        // ...and it is a real input, not just a value that prints right.
        query!(br#"{"x":1}"#, r#"try error("boom") catch "c:\(.)""#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "c:boom");
            }
        );

        // Object payload survives as an object, so the handler can index it.
        query!(br#"{"x":1}"#, r#"try error({"a":1}) catch .a"#,
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(1));
            }
        );

        // A null payload is preserved rather than collapsing to the input.
        query!(br#"{"x":1}"#, "try error(null) catch .",
            QueryResult::Owned(OwnedValue::Null) => {}
        );

        // Bare `error` raises the input, so `catch` sees it round-tripped.
        query!(br#"{"x":1}"#, "try error catch .x",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(1));
            }
        );

        // Internal (non-`error`) failures raise their message as a string.
        query!(br"1", "try .foo catch type",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "string");
            }
        );

        // A handler that fans out keeps every output instead of collapsing
        // into a single array.
        query!(br"{}", "try error(\"x\") catch (., .)",
            QueryResult::ManyOwned(vs) => {
                assert_eq!(
                    vs,
                    vec![
                        OwnedValue::String("x".into()),
                        OwnedValue::String("x".into()),
                    ]
                );
            }
        );
    }

    #[test]
    fn test_control_flow_combinations() {
        // if inside try - missing field returns null (no error to catch)
        query!(br#"{"a": true}"#, "try (if .a then .missing else .a end) catch \"error\"",
            QueryResult::One(StandardJson::Null) => {}
        );

        // if inside try with actual error (field access on number)
        query!(br#"{"a": 123}"#, "try (if true then .a.foo else 0 end) catch \"error\"",
            QueryResult::Owned(OwnedValue::String(s)) if s == "error" => {}
        );

        // try inside if - missing field returns null (no error to catch)
        query!(br#"{"a": true}"#, "if .a then try .missing catch 0 else 1 end",
            QueryResult::One(StandardJson::Null) => {}
        );

        // try inside if with actual error
        query!(br#"{"a": 123}"#, "if true then try .a.foo catch 0 else 1 end",
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
        query!(br"null", "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "null" => {}
        );
        query!(br"true", "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "boolean" => {}
        );
        query!(br"42", "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "number" => {}
        );
        query!(br#""hello""#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "string" => {}
        );
        query!(br"[1, 2]", "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "array" => {}
        );
        query!(br#"{"a": 1}"#, "type",
            QueryResult::Owned(OwnedValue::String(s)) if s == "object" => {}
        );
    }

    #[test]
    fn test_builtin_is_type() {
        // isnull
        query!(br"null", "isnull",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"1", "isnull",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isboolean
        query!(br"true", "isboolean",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"1", "isboolean",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isnumber
        query!(br"42", "isnumber",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br#""42""#, "isnumber",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isstring
        query!(br#""hello""#, "isstring",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"42", "isstring",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isarray
        query!(br"[1, 2]", "isarray",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"{}", "isarray",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );

        // isobject
        query!(br#"{"a": 1}"#, "isobject",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"[]", "isobject",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    #[test]
    fn test_builtin_length() {
        // null has length 0
        query!(br"null", "length",
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
        query!(br"[1, 2, 3]", "length",
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );

        // object length (key count)
        query!(br#"{"a": 1, "b": 2}"#, "length",
            QueryResult::Owned(OwnedValue::Int(2)) => {}
        );

        // number length is absolute value
        query!(br"-5", "length",
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
        query!(br"[1, 2, 3]", "has(0)",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"[1, 2, 3]", "has(5)",
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
        query!(br"5", "select(. > 3)",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 5);
            }
        );

        // select outputs nothing if condition is false
        query!(br"2", "select(. > 3)",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_builtin_empty() {
        query!(br"1", "empty",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_builtin_map() {
        // map applies function to each element
        query!(br"[1, 2, 3]", "map(. * 2)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(2));
                assert_eq!(arr[1], OwnedValue::Int(4));
                assert_eq!(arr[2], OwnedValue::Int(6));
            }
        );

        // map with type check
        query!(br"[1, 2, 3]", "map(. + 1)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr[0], OwnedValue::Int(2));
            }
        );

        // map(f) is [.[] | f], and .[] over an object iterates its values,
        // so jq accepts an object of entries as readily as an array (#422).
        query!(br#"{"a": 1, "b": 2}"#, "map(. + 1)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(2), OwnedValue::Int(3)]);
            }
        );
        query!(br"{}", "map(. + 1)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty());
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
        query!(br"[1, 2, 3]", "map_values(. + 1)",
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
        query!(br"[1, 2, 3]", "add",
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );

        // Add strings
        query!(br#"["a", "b", "c"]"#, "add",
            QueryResult::Owned(OwnedValue::String(s)) if s == "abc" => {}
        );

        // Add arrays
        query!(br"[[1], [2], [3]]", "add",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
            }
        );

        // Empty array returns null
        query!(br"[]", "add",
            QueryResult::Owned(OwnedValue::Null) => {}
        );

        // add is [.[] | .] folded with +, and .[] over an object iterates
        // its values, so jq accepts an object here as readily as an array
        // (#422).
        query!(br#"{"a": 1, "b": 2, "c": 3}"#, "add",
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );
        query!(br"{}", "add",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_builtin_any() {
        query!(br"[true, false]", "any",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"[false, false]", "any",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(br"[null, null]", "any",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(br"[1, 0]", "any",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}  // numbers are truthy
        );

        // any is [.[] | .] with an early-exit truthiness check, and .[] over
        // an object iterates its values, so jq accepts an object here as
        // readily as an array (#422).
        query!(br#"{"a": false, "b": true}"#, "any",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"{}", "any",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
    }

    #[test]
    fn test_builtin_all() {
        query!(br"[true, true]", "all",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
        query!(br"[true, false]", "all",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(br"[1, 2, 3]", "all",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}  // numbers are truthy
        );

        // Same shape as `any` — see #422.
        query!(br#"{"a": true, "b": false}"#, "all",
            QueryResult::Owned(OwnedValue::Bool(false)) => {}
        );
        query!(br"{}", "all",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );
    }

    #[test]
    fn test_builtin_min() {
        query!(br"[3, 1, 2]", "min",
            QueryResult::Owned(v) => assert_eq!(v, OwnedValue::Int(1))
        );
        query!(br#"["c", "a", "b"]"#, "min",
            QueryResult::Owned(OwnedValue::String(s)) if s == "a" => {}
        );
        query!(br"[]", "min",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_builtin_max() {
        query!(br"[3, 1, 2]", "max",
            QueryResult::Owned(v) => assert_eq!(v, OwnedValue::Int(3))
        );
        query!(br#"["c", "a", "b"]"#, "max",
            QueryResult::Owned(OwnedValue::String(s)) if s == "c" => {}
        );
        query!(br"[]", "max",
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
        query!(br"[1, 2, 3, 4, 5]", "map(. * 2)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 5);
                assert_eq!(arr[0], OwnedValue::Int(2));
            }
        );

        // Use select in map
        query!(br"[1, 2, 3, 4, 5]", "[.[] | select(. > 2)]",
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

        // Piping owned values through the rest of a pipe now works (#295):
        // each `.+1` yields an owned value that eval_pipe's Many branch collects.
        query!(br"[1, 2, 3]", "[.[] | . + 1] | add",
            QueryResult::Owned(OwnedValue::Int(sum)) => {
                assert_eq!(sum, 9);
            }
        );
    }

    /// Regression tests for #295: collecting an iterator pipe `[.[] | f]` must
    /// yield one output per element (matching `map(f)` and jq), not `[]`. The
    /// bug dropped every *computed/owned* inner result in `eval_pipe`'s `Many`
    /// branch. Expected outputs ground-truthed against `jq-1.7.1`.
    #[test]
    fn test_collect_iterator_pipe() {
        let int_arr =
            |xs: &[i64]| OwnedValue::Array(xs.iter().map(|&i| OwnedValue::Int(i)).collect());

        // [.[] | .+1] == [2,3,4] and must equal map(.+1).
        query!(br"[1, 2, 3]", "[.[] | . + 1]",
            QueryResult::Owned(v) => assert_eq!(v, int_arr(&[2, 3, 4])));
        query!(br"[1, 2, 3]", "map(. + 1)",
            QueryResult::Owned(v) => assert_eq!(v, int_arr(&[2, 3, 4])));

        // [.[] | [.]] == [[1],[2],[3]] (owned array-construction inner filter).
        query!(br"[1, 2, 3]", "[.[] | [.]]",
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert_eq!(arr, vec![int_arr(&[1]), int_arr(&[2]), int_arr(&[3])]);
        });

        // [.[] | tostring] == ["1","2","3"].
        query!(br"[1, 2, 3]", "[.[] | tostring]",
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert_eq!(
                arr,
                vec![
                    OwnedValue::String("1".to_string()),
                    OwnedValue::String("2".to_string()),
                    OwnedValue::String("3".to_string()),
                ]
            );
        });

        // {a: [.[] | .+1]} == {"a":[2,3,4]} (object value context).
        query!(br"[1, 2, 3]", "{a: [.[] | . + 1]}",
        QueryResult::Owned(OwnedValue::Object(obj)) => {
            assert_eq!(obj.get("a"), Some(&int_arr(&[2, 3, 4])));
        });

        // [.[] | .+1] | length == 3 (reduction over the collected pipe).
        query!(br"[1, 2, 3]", "[.[] | . + 1] | length",
            QueryResult::Owned(OwnedValue::Int(len)) => assert_eq!(len, 3));

        // first([.[] | .+1]) == [2,3,4] (jq: first of the single array output).
        query!(br"[1, 2, 3]", "first([.[] | . + 1])",
            QueryResult::Owned(v) => assert_eq!(v, int_arr(&[2, 3, 4])));

        // [.[] | .[1:2]] == [[2],[5]] (array-slice inner filter, ref #154).
        query!(br"[[1, 2, 3], [4, 5, 6]]", "[.[] | .[1:2]]",
        QueryResult::Owned(OwnedValue::Array(arr)) => {
            assert_eq!(arr, vec![int_arr(&[2]), int_arr(&[5])]);
        });
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

        // jq's ltrimstr is total: non-string argument leaves input unchanged (#394)
        query!(br#""abc""#, "ltrimstr(1)",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "abc");
            }
        );

        // jq's ltrimstr is total: non-string input passes through unchanged (#394).
        // A document-sourced number materializes as NumberLiteral, not Int (#387).
        query!(b"1", r#"ltrimstr("a")"#,
            QueryResult::Owned(OwnedValue::NumberLiteral(NumberRepr::Int(n), _)) => {
                assert_eq!(n, 1);
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

        // jq's rtrimstr is total: non-string argument leaves input unchanged (#394)
        query!(br#""abc""#, "rtrimstr(null)",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "abc");
            }
        );

        // jq's rtrimstr is total: non-string input passes through unchanged (#394).
        // A document-sourced number materializes as NumberLiteral, not Int (#387).
        query!(b"1", r#"rtrimstr("a")"#,
            QueryResult::Owned(OwnedValue::NumberLiteral(NumberRepr::Int(n), _)) => {
                assert_eq!(n, 1);
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

        // join(s) is [.[] | tostring] joined by s, and .[] over an object
        // iterates its values, so jq accepts an object here as readily as
        // an array (#422).
        query!(br#"{"a": "x", "b": "y"}"#, r#"join(",")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "x,y");
            }
        );
        query!(br"{}", r#"join(",")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "");
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
        query!(br"[1, 2, 3]", r"contains([2])",
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

        // Same type, no containment: false, not an error.
        query!(br"[1]", r"contains([2])",
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
        query!(br"1", r"contains(2)",
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
    }

    /// One representative value per `OwnedValue` variant, in jq's sort order.
    fn one_of_each_kind() -> Vec<OwnedValue> {
        vec![
            OwnedValue::Null,
            OwnedValue::Bool(false),
            OwnedValue::Bool(true),
            OwnedValue::Int(1),
            OwnedValue::Float(1.5),
            OwnedValue::String("s".to_string()),
            OwnedValue::Array(vec![]),
            OwnedValue::Object(IndexMap::new()),
        ]
    }

    /// [`sort_rank`] must stay a faithful coarsening of [`jq_kind`]: the same
    /// order, with `false`/`true` — and only those — collapsed into one rank.
    ///
    /// Deriving `sort_rank` from `jq_kind` makes this true by construction; the
    /// test exists so that re-expanding either into a hand-written match (there
    /// were three such copies before #358) fails loudly instead of drifting. See
    /// the #106 lesson in `CLAUDE.md`: one definition, plus a test that the call
    /// sites agree.
    #[test]
    fn sort_rank_agrees_with_jq_kind() {
        let values = one_of_each_kind();

        for a in &values {
            for b in &values {
                let same_kind = jq_kind(a) == jq_kind(b);
                let same_rank = sort_rank(a) == sort_rank(b);
                // Ranks may merge kinds, never split them.
                if same_kind {
                    assert!(same_rank, "{a:?} vs {b:?}: same kind, different rank");
                }
                // The bool pair is the *only* place a rank merges two kinds.
                if same_rank && !same_kind {
                    assert!(
                        matches!(a, OwnedValue::Bool(_)) && matches!(b, OwnedValue::Bool(_)),
                        "{a:?} vs {b:?}: ranks merged a pair that is not false/true"
                    );
                }
                // Coarsening preserves order: a merge may turn Less/Greater into
                // Equal, but wherever the ranks still differ they must differ
                // the same way the kinds do.
                if !same_rank {
                    assert_eq!(
                        sort_rank(a).cmp(&sort_rank(b)),
                        jq_kind(a).cmp(&jq_kind(b)),
                        "{a:?} vs {b:?}: rank order disagrees with kind order",
                    );
                }
            }
        }

        // jq's documented order: null < false < true < number < string < array
        // < object, with the two booleans sharing the `boolean` slot.
        let ranks: Vec<u8> = values.iter().map(sort_rank).collect();
        assert_eq!(ranks, vec![0, 1, 1, 2, 2, 3, 4, 5]);
    }

    /// jq errors when the two operands' kinds cannot be compared, rather than
    /// answering `false` (#358).
    ///
    /// This covers the *shape* of the result — which `QueryResult` variant each
    /// outcome lands on — for one case of each. The exhaustive oracle matrix
    /// (nesting, `Int`/`Float`, both truncation boundaries, `?` suppression, and
    /// every case run through the generic evaluator too) lives in
    /// `tests/jq_containment_tests.rs`; add new cases there rather than here, so
    /// there is one place to re-check when jq's behaviour is re-probed.
    #[test]
    fn test_builtin_contains_type_mismatch() {
        query!(br"1", r#"contains("a")"#,
            QueryResult::Error(e) => {
                assert_eq!(
                    e.message,
                    r#"number (1) and string ("a") cannot have their containment checked"#
                );
            }
        );

        // `true` and `false` are distinct jq kinds that share the *name*
        // `boolean`, so a mixed pair errors — with both operands called
        // `boolean` — while a matched pair is a plain comparison. Screening on
        // `type_name` would answer `false` for the mixed pair; see `jq_kind`.
        query!(br"true", r"contains(false)",
            QueryResult::Error(e) => {
                assert_eq!(
                    e.message,
                    "boolean (true) and boolean (false) cannot have their containment checked"
                );
            }
        );
        query!(br"true", r"contains(true)",
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        // A mismatch *inside* a container is still plain false: the screen is
        // top-level only, so `owned_contains` stays total.
        query!(br#"[1,"a"]"#, r#"contains(["a",2])"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
    }

    #[test]
    fn test_builtin_inside() {
        // inside is the inverse of contains
        query!(br"[2]", r"inside([1, 2, 3])",
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

    /// `inside` reports the container first — it is `contains` with the operands
    /// swapped, so the *argument* leads the message (#358). Wider coverage is in
    /// `tests/jq_containment_tests.rs`; see `test_builtin_contains_type_mismatch`.
    #[test]
    fn test_builtin_inside_type_mismatch() {
        query!(br"1", r"inside([1])",
            QueryResult::Error(e) => {
                assert_eq!(
                    e.message,
                    "array ([1]) and number (1) cannot have their containment checked"
                );
            }
        );

        // The boolean-kind split reaches `inside` too, argument still first.
        query!(br"true", r"inside(false)",
            QueryResult::Error(e) => {
                assert_eq!(
                    e.message,
                    "boolean (false) and boolean (true) cannot have their containment checked"
                );
            }
        );
    }

    // ==========================================================================
    // Phase 5: Array Functions Tests
    // ==========================================================================

    #[test]
    fn test_builtin_first() {
        query!(br"[1, 2, 3]", "first",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    #[test]
    fn test_builtin_last() {
        query!(br"[1, 2, 3]", "last",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(3));
            }
        );
    }

    #[test]
    fn test_builtin_nth() {
        query!(br"[10, 20, 30]", "nth(1)",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 20);
            }
        );
    }

    #[test]
    fn test_builtin_reverse() {
        query!(br"[1, 2, 3]", "reverse",
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
        query!(br"[[1, 2], [3, [4]]]", "flatten",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[3], OwnedValue::Array(vec![OwnedValue::Int(4)]));
            }
        );

        // Flatten with depth
        query!(br"[[1], [[2]], [[[3]]]]", "flatten(2)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(2));
                assert_eq!(arr[2], OwnedValue::Array(vec![OwnedValue::Int(3)]));
            }
        );

        // flatten is defined over [.[]], and .[] over an object iterates
        // its values, so jq accepts an object here as readily as an array
        // (#422). Same one-level depth as the array case above.
        query!(br#"{"a": [1, 2], "b": [3, [4]]}"#, "flatten",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(
                    arr,
                    vec![
                        OwnedValue::Int(1),
                        OwnedValue::Int(2),
                        OwnedValue::Int(3),
                        OwnedValue::Array(vec![OwnedValue::Int(4)]),
                    ]
                );
            }
        );
        query!(br"{}", "flatten",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty());
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
        query!(br"[1, 2, 1, 3, 2]", "unique",
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
        query!(br"[3, 1, 2]", "sort",
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

    /// `sort_by`/`group_by`/`unique_by`/`min_by`/`max_by` key on `[f]` — the
    /// array of *all* outputs of the key filter, not just its first output
    /// (#155). A comma-generator key filter (`sort_by(.a,.b)`) now reaches
    /// this code because the parser accepts a top-level comma in call
    /// arguments; verify the eval side actually does a multi-key sort
    /// instead of silently keying everything on `null`.
    #[test]
    fn test_by_builtins_multi_key_comma_generator() {
        let data = br#"[{"a":2,"b":1},{"a":1,"b":2},{"a":1,"b":1}]"#;

        assert_eq!(
            outputs(data, "[sort_by(.a,.b)[] | [.a,.b]]"),
            vec![r"[[1,1],[1,2],[2,1]]".to_string()]
        );

        query!(data, "min_by(.a,.b)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(1)));
            }
        );

        query!(data, "max_by(.a,.b)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(2)));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(1)));
            }
        );

        // group_by(.tags[]) — a plain (non-comma) generator key filter also
        // exercises the same "collect all outputs" fix.
        let tagged = br#"[{"a":2,"tags":["x"]},{"a":1,"tags":["x","y"]}]"#;
        query!(tagged, "group_by(.tags[])",
            QueryResult::Owned(OwnedValue::Array(groups)) => {
                assert_eq!(groups.len(), 2);
            }
        );

        // unique_by(.a,.b) — distinct (a,b) pairs are not deduplicated
        // against each other even when .a alone repeats.
        let pairs = br#"[{"a":1,"b":2},{"a":1,"b":3},{"a":1,"b":2}]"#;
        query!(pairs, "unique_by(.a,.b)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
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

        // from_entries is map({...}) | add | .//={}, and .[] over an object
        // iterates its values, so jq accepts an object of entries as
        // readily as an array of them (#422).
        query!(br#"{"x": {"key": "a", "value": 1}}"#, "from_entries",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
            }
        );
        query!(br"{}", "from_entries",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert!(obj.is_empty());
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

        query!(br"42", "@text",
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
        // jq always double-quotes every string field (#306).
        query!(br#"["a", "b", "c"]"#, "@csv",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#""a","b","c""#);
            }
        );

        // CSV with an embedded delimiter — still one pair of quotes.
        query!(br#"["hello, world", "test"]"#, "@csv",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#""hello, world","test""#);
            }
        );

        // Non-strings stay bare; null is empty (matches jq).
        query!(br#"["a", "b,c", 1, true, null]"#, "@csv",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#""a","b,c",1,true,"#);
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
    fn test_format_dsv_pipe() {
        query!(br#"["a", "b", "c"]"#, r#"@dsv("|")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#""a"|"b"|"c""#);
            }
        );
    }

    #[test]
    fn test_format_dsv_semicolon() {
        query!(br#"["a", "b", "c"]"#, r#"@dsv(";")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#""a";"b";"c""#);
            }
        );
    }

    #[test]
    fn test_format_dsv_with_quoting() {
        query!(br#"["a", "b|c", "d"]"#, r#"@dsv("|")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#""a"|"b|c"|"d""#);
            }
        );
    }

    #[test]
    fn test_format_dsv_with_quotes_in_data() {
        query!(br#"["a", "b\"c", "d"]"#, r#"@dsv(",")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, r#""a","b""c","d""#);
            }
        );
    }

    #[test]
    fn test_format_dsv_with_newline() {
        query!(br#"["a", "b\nc", "d"]"#, r#"@dsv(",")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "\"a\",\"b\nc\",\"d\"");
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

    #[test]
    fn test_format_yaml() {
        // Simple object
        query!(br#"{"a": 1, "b": 2}"#, "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "{a: 1, b: 2}");
            }
        );

        // Array
        query!(br"[1, 2, 3]", "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "[1, 2, 3]");
            }
        );

        // Nested structure
        query!(br#"{"name": "test", "items": [1, 2]}"#, "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "{name: test, items: [1, 2]}");
            }
        );

        // String that needs quoting (reserved word)
        query!(br#"{"value": "true"}"#, "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "{value: \"true\"}");
            }
        );

        // Null and boolean
        query!(br#"{"flag": true, "nothing": null}"#, "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "{flag: true, nothing: null}");
            }
        );

        // Empty containers
        query!(br"[]", "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "[]");
            }
        );

        query!(br"{}", "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "{}");
            }
        );

        // Float values
        query!(br"3.14", "@yaml",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "3.14");
            }
        );
    }

    #[test]
    fn test_format_props() {
        // Simple object
        query!(br#"{"database": "postgres", "port": 5432}"#, "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "database = postgres\nport = 5432");
            }
        );

        // Nested object
        query!(br#"{"nested": {"a": 1, "b": 2}, "top": "value"}"#, "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "nested.a = 1\nnested.b = 2\ntop = value");
            }
        );

        // Array
        query!(br#"{"arr": [1, 2, 3]}"#, "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "arr.0 = 1\narr.1 = 2\narr.2 = 3");
            }
        );

        // Deeply nested
        query!(br#"{"a": {"b": {"c": 42}}}"#, "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "a.b.c = 42");
            }
        );

        // Top-level scalar
        query!(br#""just a string""#, "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "just a string");
            }
        );

        // Null
        query!(br"null", "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "null");
            }
        );

        // Boolean values
        query!(br#"{"enabled": true, "disabled": false}"#, "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "enabled = true\ndisabled = false");
            }
        );

        // Special characters in values (preserved)
        query!(br#"{"key": "value=with=equals"}"#, "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "key = value=with=equals");
            }
        );

        // Empty object
        query!(br"{}", "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "");
            }
        );

        // Top-level array
        query!(br"[1, 2, 3]", "@props",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "0 = 1\n1 = 2\n2 = 3");
            }
        );
    }

    // =========================================================================
    // Phase 6: Type Conversion Builtins
    // =========================================================================

    #[test]
    fn test_builtin_tostring() {
        query!(br"42", "tostring",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "42");
            }
        );

        query!(br"true", "tostring",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "true");
            }
        );

        query!(br"null", "tostring",
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
        query!(br"42", "tonumber",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(42));
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
        query!(br"[97, 98, 99]", "implode",
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
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(42));
            }
        );

        query!(br"[1, 2, 3]", r"getpath([1])",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(2));
            }
        );

        // Negative index support
        query!(br"[1, 2, 3]", r"getpath([-1])",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(3));
            }
        );

        query!(br"[1, 2, 3]", r"getpath([-2])",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(2));
            }
        );

        // Nested with negative index
        query!(br#"{"a": [10, 20, 30]}"#, r#"getpath(["a", -1])"#,
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(30));
            }
        );
    }

    // =========================================================================
    // Phase 7: Regex Functions (requires "regex" feature)
    // =========================================================================

    #[cfg(feature = "regex")]
    #[test]
    fn test_regex_test() {
        // The #167 repro: character classes must match with regex semantics
        query!(br#""abc123""#, r#"test("[0-9]+")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        // Metacharacter case a substring match would get wrong
        query!(br#""abc""#, r#"test("a.c")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );

        query!(br#""abc""#, r#"test("[0-9]+")"#,
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
    }

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
        query!(br#"{"foo": 10, "bar": 5}"#, r".foo as $x | .bar + $x",
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
    fn test_variable_binding_preserves_streaming_through_a_multi_output_filter() {
        // A pipe stage after `as` binds a value is still a pipe: piping a
        // bound variable into a multi-output filter must keep streaming N
        // separate top-level outputs, the same as piping the document
        // itself would. `$doc | paths` used to collapse into one array
        // (`eval_owned_pipe` collapsed through `eval_owned_expr`, which is
        // correct for `reduce`/`foreach` but not for a plain pipe
        // continuation) where `paths` alone streams correctly.
        assert_eq!(
            outputs(br#"{"a":{"b":1}}"#, ". as $doc | $doc | paths"),
            outputs(br#"{"a":{"b":1}}"#, "paths")
        );
        assert_eq!(
            outputs(br#"{"a":{"b":1}}"#, ". as $doc | $doc | paths"),
            [r#"["a"]"#, r#"["a","b"]"#]
        );
    }

    #[test]
    fn test_reduce() {
        // Sum array elements
        query!(br"[1, 2, 3, 4, 5]", r"reduce .[] as $x (0; . + $x)",
            QueryResult::Owned(OwnedValue::Int(15)) => {}
        );

        // Count elements
        query!(br#"["a", "b", "c"]"#, r"reduce .[] as $x (0; . + 1)",
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );
    }

    #[test]
    fn test_foreach() {
        // Running sum
        query!(br"[1, 2, 3]", r"[foreach .[] as $x (0; . + $x)]",
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
        query!(br"null", r"[range(5)]",
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
        query!(br"null", r"[range(2;5)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(2),
                    OwnedValue::Int(3),
                    OwnedValue::Int(4),
                ]);
            }
        );

        // range(a;b;step)
        query!(br"null", r"[range(0;10;2)]",
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
        query!(br"null", r"[limit(3; range(10))]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(0),
                    OwnedValue::Int(1),
                    OwnedValue::Int(2),
                ]);
            }
        );
    }

    /// `limit`'s `expr` argument now accepts a top-level comma-generator
    /// (#155): `[limit(2;1,2,3,4)]` == `[1,2]`, matching real jq. The
    /// generator here is finite, so the existing eager
    /// evaluate-then-`take(n)` implementation already produces the correct
    /// answer without needing true short-circuiting.
    #[test]
    fn test_limit_comma_generator_argument() {
        query!(br"null", r"[limit(2;1,2,3,4)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(1), OwnedValue::Int(2)]);
            }
        );
    }

    #[test]
    fn test_first_last_expr() {
        // first(expr) - returns a reference to first element
        query!(br"[1, 2, 3]", r"first(.[])",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );

        // last(expr) - returns a reference to last element
        query!(br"[1, 2, 3]", r"last(.[])",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 3);
            }
        );
    }

    /// `first`/`last`'s argument now accepts a top-level comma-generator
    /// (#155): `first(1,2,3)` == `1`, `last(1,2,3)` == `3`.
    #[test]
    fn test_first_last_expr_comma_generator_argument() {
        query!(br"null", r"first(1,2,3)",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 1);
            }
        );

        query!(br"null", r"last(1,2,3)",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 3);
            }
        );
    }

    /// A single-parameter user-defined function called with a
    /// comma-generator argument (#155): `def f(x): x; f(1,2)` fans out to
    /// two outputs, matching real jq's call-by-name substitution semantics.
    #[test]
    fn test_user_function_call_with_comma_generator_argument() {
        assert_eq!(
            outputs(b"null", "def f(x): x; f(1,2)"),
            vec!["1".to_string(), "2".to_string()]
        );
    }

    #[test]
    fn test_until() {
        // until(cond; update) - iterate until condition is true
        query!(br"1", r"until(. >= 10; . * 2)",
            QueryResult::Owned(OwnedValue::Int(16)) => {}
        );
    }

    #[test]
    fn test_while() {
        // while(cond; update) - output while condition is true
        query!(br"1", r"[while(. < 10; . * 2)]",
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
    fn test_repeat() {
        // repeat(expr) - repeatedly evaluate expr with original input
        // jq behavior: repeat(. * 2) on input 1 produces 2, 2, 2, ...
        query!(br"1", r"[limit(5; repeat(. * 2))]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::Int(2),
                    OwnedValue::Int(2),
                    OwnedValue::Int(2),
                    OwnedValue::Int(2),
                    OwnedValue::Int(2),
                ]);
            }
        );
    }

    #[test]
    fn test_repeat_identity() {
        // repeat(.) produces the same value infinitely
        query!(br#""hello""#, r"[limit(3; repeat(.))]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![
                    OwnedValue::String("hello".into()),
                    OwnedValue::String("hello".into()),
                    OwnedValue::String("hello".into()),
                ]);
            }
        );
    }

    #[test]
    fn test_recurse() {
        // Basic recurse with filter - collect all values recursively
        query!(br#"{"a": 1, "b": {"c": 2}}"#, r"[recurse | .a? // .c? // empty]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // Should contain the values 1 and 2
                assert!(arr.len() >= 2);
            }
        );
    }

    #[test]
    fn test_isvalid() {
        // isvalid returns true for valid expressions
        query!(br#"{"a": 1}"#, r"isvalid(.a)",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        // isvalid returns true for missing field (returns null, not error)
        query!(br#"{"a": 1}"#, r"isvalid(.b)",
            QueryResult::Owned(OwnedValue::Bool(true)) => {}
        );

        // isvalid returns false for actual error-producing expressions
        query!(br"123", r"isvalid(.foo)",
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
        query!(br"[1, 2, 3]", r". as [$a, $b] | $a + $b",
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );
    }

    #[test]
    fn test_destructuring_nested_pattern() {
        // Nested destructuring
        query!(br#"{"user": {"name": "Bob", "id": 42}}"#, r". as {user: {name: $n}} | $n",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "Bob");
            }
        );
    }

    #[test]
    fn test_function_def_simple() {
        // Simple function definition without parameters
        query!(br"5", r"def double: . * 2; double",
            QueryResult::Owned(OwnedValue::Int(10)) => {}
        );
    }

    #[test]
    fn test_function_def_with_params() {
        // Function with parameters (using 'addtwo' to avoid conflict with builtin 'add')
        query!(br"null", r"def addtwo(a; b): a + b; addtwo(3; 4)",
            QueryResult::Owned(OwnedValue::Int(7)) => {}
        );
    }

    #[test]
    fn test_function_def_chained() {
        // Single def, then use in pipe
        query!(br"5", r"def inc: . + 1; . | inc",
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );

        // Two defs, use only second - this exercises nested func def
        query!(br"5", r"def double: . * 2; def inc: . + 1; inc",
            QueryResult::Owned(OwnedValue::Int(6)) => {}
        );

        // Two defs, use only first
        query!(br"5", r"def double: . * 2; def inc: . + 1; double",
            QueryResult::Owned(OwnedValue::Int(10)) => {}
        );

        // This should work: use both in a pipe, but inc is defined first
        // (so inc isn't nested inside double's scope)
        query!(br"5", r"def inc: . + 1; def double: . * 2; double | inc",
            QueryResult::Owned(OwnedValue::Int(11)) => {}
        );
    }

    #[test]
    fn test_function_uses_input() {
        // Function that uses the input value
        query!(br#"{"x": 10}"#, r"def getx: .x; getx",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 10);
            }
        );
    }

    #[test]
    fn test_function_with_filter_param() {
        // Function with filter parameter (jq-style)
        query!(br"[1, 2, 3]", r"def apply(f): map(f); apply(. * 2)",
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
        query!(br"[[1, 2], [3, 4], [5, 6]]", "transpose",
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

    /// Every expectation here is jq-1.7.1's own output for the same input.
    ///
    /// The absent cases carry the weight: before #384 they returned
    /// `{"index": n}` instead of jq's negative insertion point, and the
    /// container cases returned a *found* index for a value that is not
    /// present, because `bsearch` used a comparator with no `(Array, Array)`
    /// or `(Object, Object)` arm.
    #[test]
    fn test_bsearch() {
        macro_rules! bsearch_is {
            ($input:expr, $filter:expr, $expected:expr) => {
                query!($input, $filter,
                    QueryResult::Owned(OwnedValue::Int(idx)) => {
                        assert_eq!(idx, $expected, "{} | {}", stringify!($input), $filter);
                    }
                );
            };
        }

        // Scalars, present and absent.
        bsearch_is!(br"[1, 2, 3, 4, 5]", "bsearch(3)", 2);
        bsearch_is!(br"[1, 2, 4, 5]", "bsearch(3)", -3);
        bsearch_is!(br"[1, 2, 3]", "bsearch(5)", -4);
        bsearch_is!(br"[1, 3, 5]", "bsearch(2)", -2);
        bsearch_is!(br#"["a", "b"]"#, r#"bsearch("c")"#, -3);

        // Containers: the comparator must recurse into contents, not stop at
        // the type rank (which is equal for any two arrays).
        bsearch_is!(br"[[1], [2], [3]]", "bsearch([2])", 1);
        bsearch_is!(br"[[1], [2], [3]]", "bsearch([9])", -4);
        bsearch_is!(br#"[{"a": 1}, {"a": 2}]"#, r#"bsearch({"a": 2})"#, 1);
        bsearch_is!(br#"[{"a": 1}, {"a": 3}]"#, r#"bsearch({"a": 2})"#, -2);

        // Degenerate lengths jq special-cases before its loop; the uniform
        // loop must reach the same answers.
        bsearch_is!(br"[]", "bsearch(1)", -1);
        bsearch_is!(br"[5]", "bsearch(5)", 0);
        bsearch_is!(br"[5]", "bsearch(9)", -2);
        bsearch_is!(br"[5]", "bsearch(1)", -1);

        // Which of several equal elements is chosen is jq's midpoint
        // convention, not an arbitrary one — see the note in `builtin_bsearch`.
        bsearch_is!(br"[1, 1]", "bsearch(1)", 0);
        bsearch_is!(br"[1, 1, 1]", "bsearch(1)", 1);
        bsearch_is!(br"[1, 1, 1, 1]", "bsearch(1)", 1);
        bsearch_is!(br"[0, 1, 1, 1, 2]", "bsearch(1)", 2);

        // Cross-type ordering still applies within a mixed sorted array.
        bsearch_is!(
            br#"[null, true, 1, "a", [1], {"a": 1}]"#,
            r#"bsearch("a")"#,
            3
        );
        bsearch_is!(br#"[null, true, 1, "a", [1], {"a": 1}]"#, "bsearch(2)", -4);

        // NaN never compares Equal to anything, including itself (#421), so
        // it is reported absent rather than falsely "found" by the old
        // fold-to-Equal bug.
        bsearch_is!(br"[1, 2, 3]", "bsearch(nan)", -1);

        // `null | length == 0` in jq, so `null` takes the empty-array branch
        // and answers "not found" instead of erroring (#420).
        bsearch_is!(br"null", "bsearch(1)", -1);
    }

    /// Every other non-array still errors, because jq's own `length` errors
    /// on them too — `null` (#420) is the only exception.
    #[test]
    fn test_bsearch_non_array_errors() {
        query!(br"5", "bsearch(1)",
            QueryResult::Error(e) => {
                assert!(e.to_string().contains("bsearch requires array"));
            }
        );
        query!(br#""abc""#, "bsearch(1)",
            QueryResult::Error(e) => {
                assert!(e.to_string().contains("bsearch requires array"));
            }
        );
        query!(br#"{"a": 1}"#, "bsearch(1)",
            QueryResult::Error(e) => {
                assert!(e.to_string().contains("bsearch requires array"));
            }
        );
        query!(br"false", "bsearch(1)",
            QueryResult::Error(e) => {
                assert!(e.to_string().contains("bsearch requires array"));
            }
        );
    }

    /// `compare_values` orders NaN as `Less` than every number, including
    /// another NaN (#421) -- a genuine violation of the strict weak ordering
    /// `[T]::sort_by` assumes. Rust's stable sort only reaches the internal
    /// consistency check that can panic on such a violation
    /// (`core::slice::sort::shared::smallsort`'s bidirectional merge) for
    /// slices longer than 20 elements; shorter slices use a plain insertion
    /// sort that cannot panic regardless of comparator validity.
    ///
    /// This is a canary for that threshold, not a jq-parity check: it only
    /// pins that `sort`/`sort_by`/`unique`/`unique_by`/`group_by` complete
    /// without panicking on an array past that threshold holding several
    /// NaNs -- not what order they land in (genuinely unspecified once two or
    /// more NaNs share a slice this large; jq's own qsort-based sort makes no
    /// promise here either).
    ///
    /// `sort`/`sort_by` don't dedup, so their length is pinned exactly.
    /// `unique`/`unique_by`/`group_by` do dedup/group by `compare_values`
    /// equality, and a separate, pre-existing defect (a freshly-constructed
    /// array materializes through JSON text, which has no NaN literal, so
    /// two or more NaN elements collapse to real, mutually-`Equal` `Null`s
    /// before `compare_values` ever runs -- see #421's "Separate defect"
    /// section) makes their post-dedup length unpredictable here. That
    /// defect is out of scope for this fix; this test only needs "did not
    /// panic", so it doesn't assert a specific count for those three.
    #[test]
    fn test_sort_many_nans_does_not_panic_421() {
        for filter in [
            "[range(30), nan, nan, nan] | sort | length",
            "[range(30), nan, nan, nan] | sort_by(.) | length",
        ] {
            query!(b"null", filter,
                QueryResult::Owned(OwnedValue::Int(n)) => {
                    assert_eq!(n, 33, "{filter}");
                }
            );
        }
        for filter in [
            "[range(30), nan, nan, nan] | unique | length",
            "[range(30), nan, nan, nan] | unique_by(.) | length",
            "[range(30), nan, nan, nan] | group_by(.) | length",
        ] {
            query!(b"null", filter,
                QueryResult::Owned(OwnedValue::Int(_)) => {}
            );
        }
    }

    #[test]
    fn test_paths() {
        // paths streams individual paths (matching jq behavior)
        query!(br#"{"a": 1, "b": {"c": 2}}"#, "paths",
            QueryResult::ManyOwned(paths) => {
                // Should have paths: ["a"], ["b"], ["b", "c"]
                assert_eq!(paths.len(), 3);
                assert_eq!(paths[0], OwnedValue::Array(vec![OwnedValue::String("a".into())]));
                assert_eq!(paths[1], OwnedValue::Array(vec![OwnedValue::String("b".into())]));
                assert_eq!(paths[2], OwnedValue::Array(vec![
                    OwnedValue::String("b".into()),
                    OwnedValue::String("c".into())
                ]));
            }
        );
    }

    #[test]
    fn test_paths_single() {
        // Single path returns single Owned result
        query!(br#"{"a": 1}"#, "paths",
            QueryResult::Owned(OwnedValue::Array(path)) => {
                assert_eq!(path, vec![OwnedValue::String("a".into())]);
            }
        );
    }

    #[test]
    fn test_paths_collected() {
        // Collected with [...] matches jq's [paths]
        query!(br#"{"a": 1, "b": 2}"#, "[paths]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
            }
        );
    }

    #[test]
    fn test_leaf_paths() {
        // leaf_paths streams individual paths (like paths(scalars) in jq)
        query!(br#"{"a": 1, "b": {"c": 2}}"#, "leaf_paths",
            QueryResult::ManyOwned(paths) => {
                // Should have paths: ["a"], ["b", "c"]
                assert_eq!(paths.len(), 2);
                assert_eq!(paths[0], OwnedValue::Array(vec![OwnedValue::String("a".into())]));
                assert_eq!(paths[1], OwnedValue::Array(vec![
                    OwnedValue::String("b".into()),
                    OwnedValue::String("c".into())
                ]));
            }
        );
    }

    #[test]
    fn test_leaf_paths_single() {
        // Single leaf returns single Owned result
        query!(br#"{"a": 1}"#, "leaf_paths",
            QueryResult::Owned(OwnedValue::Array(path)) => {
                assert_eq!(path, vec![OwnedValue::String("a".into())]);
            }
        );
    }

    #[test]
    fn test_leaf_paths_array() {
        // Arrays also work
        query!(br"[1, [2, 3]]", "[leaf_paths]",
            QueryResult::Owned(OwnedValue::Array(paths)) => {
                // Paths: [0], [1, 0], [1, 1]
                assert_eq!(paths.len(), 3);
            }
        );
    }

    #[test]
    fn test_leaf_paths_with_null() {
        // null is included as a leaf (unlike jq's paths(scalars) which excludes it)
        query!(br#"{"a": null, "b": 1}"#, "[leaf_paths]",
            QueryResult::Owned(OwnedValue::Array(paths)) => {
                assert_eq!(paths.len(), 2);
            }
        );
    }

    #[test]
    fn test_leaf_paths_empty_containers() {
        // Empty containers are considered leaves
        query!(br#"{"a": [], "b": {}}"#, "[leaf_paths]",
            QueryResult::Owned(OwnedValue::Array(paths)) => {
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

    /// Run `filter` over `json`, rendering the outcome the way the CLI would:
    /// `Ok` with the outputs as JSON, `Err` with the raised message.
    ///
    /// Every expectation below was read off jq-1.7.1 (the version pinned in
    /// `tests/data/jq-golden/JQ_VERSION`), not off succinctly.
    fn outcome(json: &[u8], filter: &str) -> Result<String, String> {
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let expr = parse(filter).unwrap();
        match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
            QueryResult::Error(e) => Err(e.message),
            other => Ok(other
                .collect_owned()
                .iter()
                .map(OwnedValue::to_json)
                .collect::<Vec<_>>()
                .join(" ")),
        }
    }

    /// Asserts `filter` over each `input` produces the paired outcome.
    fn assert_outcomes(cases: &[(&[u8], &str, Result<&str, &str>)]) {
        for (input, filter, expected) in cases {
            let want = expected.map(str::to_string).map_err(str::to_string);
            assert_eq!(
                outcome(input, filter),
                want,
                "{} | {filter}",
                core::str::from_utf8(input).unwrap()
            );
        }
    }

    /// jq refuses to index a scalar; it does not replace it with a freshly
    /// built container. `1 | setpath(["a"]; 1)` is an error, not `{"a":1}`
    /// (#359).
    #[test]
    fn test_setpath_refuses_to_index_a_scalar() {
        assert_outcomes(&[
            (
                b"1",
                r#"setpath(["a"]; 1)"#,
                Err(r#"Cannot index number with string "a""#),
            ),
            (
                br#""str""#,
                r#"setpath(["a"]; 1)"#,
                Err(r#"Cannot index string with string "a""#),
            ),
            (
                b"true",
                r#"setpath(["a"]; 1)"#,
                Err(r#"Cannot index boolean with string "a""#),
            ),
            (
                b"1",
                "setpath([0]; 1)",
                Err("Cannot index number with number"),
            ),
            (
                br#""s""#,
                "setpath([0]; 1)",
                Err("Cannot index string with number"),
            ),
        ]);
    }

    /// The refusal follows the path down, not just at the root: the scalar
    /// `{"a":1}` reaches at `.a` is as unindexable as a scalar input.
    #[test]
    fn test_setpath_refuses_a_scalar_reached_through_the_path() {
        assert_outcomes(&[
            (
                br#"{"a": 1}"#,
                r#"setpath(["a", "b"]; 2)"#,
                Err(r#"Cannot index number with string "b""#),
            ),
            (
                br#"{"a": null}"#,
                r#"setpath(["a", "b"]; 2)"#,
                Ok(r#"{"a":{"b":2}}"#),
            ),
        ]);
    }

    /// `null` is the one value jq auto-vivifies, and it becomes whichever
    /// container the path element calls for — at any depth.
    #[test]
    fn test_setpath_auto_vivifies_only_null() {
        assert_outcomes(&[
            (b"null", r#"setpath(["a"]; 1)"#, Ok(r#"{"a":1}"#)),
            (b"null", "setpath([0]; 1)", Ok("[1]")),
            (b"null", r#"setpath(["a", 0]; 1)"#, Ok(r#"{"a":[1]}"#)),
            (b"null", r#"setpath([0, "a"]; 1)"#, Ok(r#"[{"a":1}]"#)),
            (b"1", "setpath([]; 2)", Ok("2")),
        ]);
    }

    /// `=`/`|=` agree with `setpath()` on every auto-vivification/padding shape
    /// above (#486) — the shared case table the issue asked for so `set_path`/
    /// `update_path` and `set_value_at_path` cannot drift apart again.
    #[test]
    fn test_assign_and_update_agree_with_setpath_on_autovivification() {
        assert_outcomes(&[
            (b"null", ".a = 1", Ok(r#"{"a":1}"#)),
            (b"null", ".a |= 1", Ok(r#"{"a":1}"#)),
            (b"null", ".[0] = 1", Ok("[1]")),
            (b"null", ".[0] |= 1", Ok("[1]")),
            (br#"{"a":[]}"#, ".a[0] = 1", Ok(r#"{"a":[1]}"#)),
            (br#"{"a":[]}"#, ".a[0] |= 1", Ok(r#"{"a":[1]}"#)),
        ]);
    }

    /// A real container indexed with the wrong kind of key is refused too —
    /// it is not silently replaced by the container the key does fit.
    #[test]
    fn test_setpath_refuses_a_container_indexed_with_the_wrong_key() {
        assert_outcomes(&[
            (
                b"{}",
                "setpath([0]; 1)",
                Err("Cannot index object with number"),
            ),
            (
                b"[]",
                r#"setpath(["a"]; 1)"#,
                Err(r#"Cannot index array with string "a""#),
            ),
            (
                b"null",
                "setpath([null]; 2)",
                Err("Cannot index null with null"),
            ),
            (
                b"null",
                "setpath([true]; 2)",
                Err("Cannot index null with boolean"),
            ),
            (
                br#"{"a": 1}"#,
                "setpath([[1]]; 2)",
                Err("Cannot index object with array"),
            ),
            (
                b"1",
                r#"setpath([{"start": 1, "end": 2}]; ["x"])"#,
                Err("Cannot index number with object"),
            ),
        ]);
    }

    /// A negative index counts from the end; one that stays negative is out of
    /// bounds. Before #359 the out-of-range case computed `(2 + -5) as usize`
    /// and padded with nulls until memory ran out, so this test also asserts
    /// termination.
    #[test]
    fn test_setpath_negative_index() {
        assert_outcomes(&[
            (b"[1,2]", "setpath([-1]; 9)", Ok("[1,9]")),
            (
                b"[1,2]",
                "setpath([-5]; 9)",
                Err("Out of bounds negative array index"),
            ),
            (
                b"null",
                "setpath([-1]; 9)",
                Err("Out of bounds negative array index"),
            ),
        ]);
    }

    /// jq accepts a float index and truncates it toward zero, and refuses NaN.
    #[test]
    fn test_setpath_float_index_truncates_toward_zero() {
        assert_outcomes(&[
            (b"null", "setpath([1.7]; 9)", Ok("[null,9]")),
            (b"[1,2,3]", "setpath([1.9]; 9)", Ok("[1,9,3]")),
            (b"[1,2,3]", "setpath([-0.5]; 9)", Ok("[9,2,3]")),
            (b"[1,2,3]", "setpath([-1.5]; 9)", Ok("[1,2,9]")),
            (
                b"null",
                "setpath([nan]; 9)",
                Err("Cannot set array element at NaN index"),
            ),
        ]);
    }

    /// Writing to an existing key leaves it where it was, and an index past
    /// the end pads with nulls.
    #[test]
    fn test_setpath_preserves_key_order_and_pads_arrays() {
        assert_outcomes(&[
            (
                br#"{"a": 1, "b": 2}"#,
                r#"setpath(["a"]; 9)"#,
                Ok(r#"{"a":9,"b":2}"#),
            ),
            (b"[1,2]", "setpath([5]; 9)", Ok("[1,2,null,null,null,9]")),
        ]);
    }

    /// The padding length comes from the document, so an absurd index asks for
    /// an absurd allocation: `1e30` truncates to `i64::MAX` and `Vec::resize`
    /// answers that with a `capacity overflow` *panic*, which for a library
    /// means taking the embedder's process with it. It is a catchable error.
    ///
    /// The message is matched by prefix because it names the requested length,
    /// which differs by word size.
    #[test]
    fn test_setpath_refuses_a_length_it_cannot_allocate() {
        let err = outcome(b"null", "setpath([1e30]; 9)").unwrap_err();
        assert!(err.starts_with("Cannot grow array to"), "{err}");
        // Lengths that do fit still pad, so jq's behaviour is untouched.
        assert_outcomes(&[(b"null", "setpath([3]; 9)", Ok("[null,null,null,9]"))]);
    }

    /// `fromjson` reads one JSON value and requires it to be the whole string,
    /// as jq's parser does. The prefix parse it used before returned `0` for
    /// `"0x10"` and `1` for `"1 2"`, silently dropping the rest.
    #[test]
    fn test_fromjson_requires_the_whole_string() {
        assert_outcomes(&[
            (
                br#""0x10""#,
                "fromjson",
                Err("Invalid numeric literal at EOF at line 1, column 4 (while parsing '0x10')"),
            ),
            (
                br#""1 2""#,
                "fromjson",
                Err("Invalid numeric literal at EOF at line 1, column 3 (while parsing '1 2')"),
            ),
            (br#"" 42 ""#, "fromjson", Ok("42")),
            (br#""[1,2]""#, "fromjson", Ok("[1,2]")),
        ]);
    }

    /// A string that opens a container and stops used to index one byte past
    /// the input while looking for an object key, panicking inside the JSON
    /// parser. `fromjson` reached it directly; `tonumber` reaches it too, since
    /// it asks the same parser whether a non-numeric string is valid JSON.
    #[test]
    fn test_conversions_do_not_panic_at_end_of_input() {
        for input in [br#""{""#.as_slice(), br#""{\"a\":1,""#, br#""[""#] {
            for filter in ["fromjson", "tonumber"] {
                let result = outcome(input, filter);
                assert!(
                    result.is_err(),
                    "{} | {filter} should error, got {result:?}",
                    core::str::from_utf8(input).unwrap()
                );
            }
        }
    }

    /// A path argument that is not an array at all gets jq's own sentence, and
    /// the refusal is catchable like any other raised error.
    ///
    /// The `setpath(…)?` form jq also accepts is not covered here: the parser
    /// does not yet take `?` after a call, which is a separate gap.
    #[test]
    fn test_setpath_path_argument_and_catch() {
        assert_outcomes(&[
            (
                b"1",
                r#"setpath("a"; 1)"#,
                Err("Path must be specified as an array"),
            ),
            (
                b"1",
                r#"try setpath(["a"]; 1) catch ."#,
                Ok(r#""Cannot index number with string \"a\"""#),
            ),
        ]);
    }

    /// `getpath` resolves an array index exactly as `setpath` does — a float
    /// truncates toward zero, a negative counts back from the end — but a read
    /// that lands nowhere is `null` rather than an error, so NaN and either
    /// end's overrun all answer `null`.
    ///
    /// The probe corpus cannot pin any of this: jq answers with a value, and a
    /// probe is only admitted if jq errors.
    #[test]
    fn test_getpath_resolves_numeric_indices_like_setpath() {
        assert_outcomes(&[
            (b"[1,2,3]", "getpath([1.5])", Ok("2")),
            (b"[1,2,3]", "getpath([-0.5])", Ok("1")),
            (b"[1,2,3]", "getpath([-1.5])", Ok("3")),
            (b"[1,2,3]", "getpath([-1])", Ok("3")),
            // Out of range at either end, NaN and ±infinity are all `null`.
            (b"[1,2,3]", "getpath([5])", Ok("null")),
            (b"[1,2,3]", "getpath([-5])", Ok("null")),
            (b"[1,2,3]", "getpath([nan])", Ok("null")),
            (b"[1,2,3]", "getpath([infinite])", Ok("null")),
            (b"[1,2,3]", "getpath([-infinite])", Ok("null")),
            // A miss keeps walking, so the rest of the path reads through null.
            (br#"[{"a":9}]"#, r#"getpath([1.5,"a"])"#, Ok("null")),
            (br#"[{"a":9}]"#, r#"getpath([0.5,"a"])"#, Ok("9")),
            // A non-numeric key is still refused.
            (
                b"[1,2,3]",
                r#"getpath(["a"])"#,
                Err(r#"Cannot index array with string "a""#),
            ),
        ]);
    }

    /// jq defines `to_entries` over `keys_unsorted`, so an array's keys are its
    /// indices and only a value with no keys at all is refused — with `keys`'
    /// own sentence, which `with_entries` beside it already used.
    #[test]
    fn test_to_entries_follows_keys() {
        assert_outcomes(&[
            (
                b"[1,2]",
                "to_entries",
                Ok(r#"[{"key":0,"value":1},{"key":1,"value":2}]"#),
            ),
            (
                br#"{"a":1}"#,
                "to_entries",
                Ok(r#"[{"key":"a","value":1}]"#),
            ),
            (b"[]", "to_entries", Ok("[]")),
            (b"1", "to_entries", Err("number (1) has no keys")),
            (b"null", "to_entries", Err("null (null) has no keys")),
        ]);
    }

    /// `from_entries` refuses an entry it cannot use; it does not drop it.
    ///
    /// Dropping is what #391 was: `[{"key":0,"value":1}] | from_entries` came
    /// back `{}`, so the caller got a smaller object with nothing to say it had
    /// lost an entry. Every case here answered `{}` before the fix.
    #[test]
    fn test_from_entries_refuses_a_key_it_cannot_use() {
        assert_outcomes(&[
            (
                br#"[{"key":0,"value":1}]"#,
                "from_entries",
                Err("Cannot use number (0) as object key"),
            ),
            (
                br#"[{"key":true,"value":1}]"#,
                "from_entries",
                Err("Cannot use boolean (true) as object key"),
            ),
            (
                br#"[{"key":[1],"value":1}]"#,
                "from_entries",
                Err("Cannot use array ([1]) as object key"),
            ),
            // No key alias at all leaves the key `null`, which is refused too —
            // jq names the value it got, not the absence.
            (
                br#"[{"value":1}]"#,
                "from_entries",
                Err("Cannot use null (null) as object key"),
            ),
            (
                b"[null]",
                "from_entries",
                Err("Cannot use null (null) as object key"),
            ),
            // A non-object entry is indexed with "key" all the same, so it
            // reports the indexing error rather than the key refusal.
            (
                br#"["a"]"#,
                "from_entries",
                Err(r#"Cannot index string with string "key""#),
            ),
            (
                b"[[0,1]]",
                "from_entries",
                Err(r#"Cannot index array with string "key""#),
            ),
            // The refusal fires at the first bad entry, not after collecting
            // the good ones.
            (
                br#"[{"key":"a","value":1},{"key":0,"value":2}]"#,
                "from_entries",
                Err("Cannot use number (0) as object key"),
            ),
            // `try` swallows it outright, as jq does.
            (br#"[{"key":0,"value":1}]"#, "try from_entries", Ok("")),
            (br#"["a"]"#, "try from_entries", Ok("")),
            (
                br#"[{"key":0,"value":1}]"#,
                r#"try from_entries catch "E""#,
                Ok(r#""E""#),
            ),
        ]);
    }

    /// The `optional` flag suppresses the refusal, which is what jq's `?`
    /// suffix does — `[{"key":0}] | from_entries?` prints nothing.
    ///
    /// Driven through the builtins directly because succinctly's parser does
    /// not yet accept `?` after anything but a path expression: `from_entries?`
    /// and `{(.):1}?` are both compile errors today, a gap of their own. The
    /// flag still has to mean the right thing when it arrives.
    #[test]
    fn test_optional_suppresses_the_object_key_refusal() {
        let json = br#"[{"key":0,"value":1}]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        assert!(matches!(
            builtin_from_entries::<Vec<u64>>(cursor.value(), true),
            QueryResult::None
        ));

        let f = parse(".").unwrap();
        assert!(matches!(
            builtin_with_entries::<Vec<u64>, JqSemantics>(&f, cursor.value(), true),
            QueryResult::None
        ));

        let json = b"0";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let expr = parse("{(.):1}").unwrap();
        assert!(matches!(
            eval_single::<Vec<u64>, JqSemantics>(&expr, cursor.value(), true),
            QueryResult::None
        ));
    }

    /// jq's key aliases are `.key // .Key // .name // .Name` and its value
    /// aliases are `if has("value") then .value else .Value end`.
    ///
    /// The two halves disagree on purpose, and both halves matter: the key
    /// chain is the *alternative* operator, so an alias holding `null` or
    /// `false` falls through — while the value lookup is *presence*, so an
    /// explicit `"value": null` beats a `"Value"` beside it. Reading either
    /// half the other way changes the answer, silently.
    #[test]
    fn test_from_entries_alias_chain_follows_jq() {
        assert_outcomes(&[
            (
                br#"[{"key":"a","value":1},{"Key":"b","value":2},{"name":"c","value":3},{"Name":"d","value":4}]"#,
                "from_entries",
                Ok(r#"{"a":1,"b":2,"c":3,"d":4}"#),
            ),
            // `//`, not presence: a null or false alias is skipped.
            (
                br#"[{"key":null,"Key":false,"name":"n","value":1}]"#,
                "from_entries",
                Ok(r#"{"n":1}"#),
            ),
            // The *last* alias has nothing to be skipped in favour of, so its
            // falsy value is the chain's: `a // b` yields `b` whatever `b` is.
            // Letting the tail fall through too reported `null (null)` here.
            (
                br#"[{"Name":false,"value":1}]"#,
                "from_entries",
                Err("Cannot use boolean (false) as object key"),
            ),
            (
                br#"[{"key":false,"Key":false,"name":false,"Name":false}]"#,
                "from_entries",
                Err("Cannot use boolean (false) as object key"),
            ),
            // …and an absent tail really is null, which is the case that made
            // the wrong reading look right.
            (
                br#"[{"key":false,"value":1}]"#,
                "from_entries",
                Err("Cannot use null (null) as object key"),
            ),
            // Presence, not `//`: an explicit null value wins over `Value`.
            (
                br#"[{"key":"a","value":null,"Value":2},{"key":"b","Value":3}]"#,
                "from_entries",
                Ok(r#"{"a":null,"b":3}"#),
            ),
            // Earlier alias wins when several are present.
            (
                br#"[{"key":"a","Key":"b","name":"c","Name":"d","value":1}]"#,
                "from_entries",
                Ok(r#"{"a":1}"#),
            ),
            // A repeated key keeps its position and takes the later value,
            // which is what jq's `add` over the mapped singletons does.
            (
                br#"[{"key":"a","value":1},{"key":"b","value":2},{"key":"a","value":3}]"#,
                "from_entries",
                Ok(r#"{"a":3,"b":2}"#),
            ),
            (b"[]", "from_entries", Ok("{}")),
        ]);
    }

    /// `with_entries(f)` is `to_entries | map(f) | from_entries`, so it inherits
    /// both ends: `to_entries` accepts an array (its keys are the indices) and
    /// `from_entries` then refuses those number keys. Before #391 the object-only
    /// shape reported `array ([1,2]) has no keys` from a type check jq has not
    /// got, and a filter that wrote a non-string key dropped the entry.
    #[test]
    fn test_with_entries_composes_to_entries_and_from_entries() {
        assert_outcomes(&[
            (
                b"[1,2]",
                "with_entries(.)",
                Err("Cannot use number (0) as object key"),
            ),
            (
                br#"{"a":1}"#,
                "with_entries(.key = 0)",
                Err("Cannot use number (0) as object key"),
            ),
            // A value with no keys at all is still `to_entries`' refusal.
            (b"1", "with_entries(.)", Err("number (1) has no keys")),
            (
                br#"{"a":1,"b":2}"#,
                "with_entries(.value += 10)",
                Ok(r#"{"a":11,"b":12}"#),
            ),
        ]);
    }

    /// jq *defines* `from_entries` as object construction over the entries, so
    /// `{(0):1}` owes the same sentence — the family rule in
    /// `docs/compliance/jq/limitations.md` applied to a family jq's own source
    /// spells out. It used to say `key must be a string`.
    #[test]
    fn test_object_construction_refuses_a_non_string_key() {
        assert_outcomes(&[
            (b"0", "{(.):1}", Err("Cannot use number (0) as object key")),
            (
                b"null",
                "{(.):1}",
                Err("Cannot use null (null) as object key"),
            ),
            (
                b"[1]",
                "{(.):1}",
                Err("Cannot use array ([1]) as object key"),
            ),
            (br#""s""#, "{(.):1}", Ok(r#"{"s":1}"#)),
            (b"0", r#"try {(.):1} catch "E""#, Ok(r#""E""#)),
        ]);
    }

    /// `indices`, `index` and `rindex` all index their string input with the
    /// pattern, so a non-string pattern reports jq's indexing error rather than
    /// naming the argument. One helper, three call sites.
    #[test]
    fn test_string_search_reports_the_indexing_error() {
        assert_outcomes(&[
            (
                br#""abc""#,
                "indices(1)",
                Err("Cannot index string with number"),
            ),
            (
                br#""abc""#,
                "index(null)",
                Err("Cannot index string with null"),
            ),
            (
                br#""abc""#,
                "rindex([1])",
                Err("Cannot index string with array"),
            ),
            // The string cases still work.
            (br#""abcabc""#, r#"indices("b")"#, Ok("[1,4]")),
            (br#""abcabc""#, r#"index("b")"#, Ok("1")),
            (br#""abcabc""#, r#"rindex("b")"#, Ok("4")),
        ]);
    }

    /// The other half of the same refusal: what the three searches do with an
    /// input they cannot search. jq's `_strindices` answers `null` where there
    /// are no characters to search rather than raising, so `null` and an object
    /// handed a string pattern are values, not errors — and everything else
    /// reports the indexing error, for all three alike.
    #[test]
    fn test_string_search_unsearchable_inputs() {
        assert_outcomes(&[
            // jq answers with a value here, so no probe can pin these.
            (b"null", r#"indices("a")"#, Ok("null")),
            (b"null", r#"index("a")"#, Ok("null")),
            (b"null", r#"rindex("a")"#, Ok("null")),
            (b"{}", r#"indices("a")"#, Ok("null")),
            (b"{}", r#"index("a")"#, Ok("null")),
            (b"{}", r#"rindex("a")"#, Ok("null")),
            // A non-string pattern never reaches `_strindices`, so the object
            // is indexed with it and refused.
            (b"{}", "indices(1)", Err("Cannot index object with number")),
            // Scalars report the indexing error, which `index`/`rindex` used
            // to word as `expected string or array, got <t>`.
            (
                b"1",
                r#"index("a")"#,
                Err(r#"Cannot index number with string "a""#),
            ),
            (
                b"true",
                r#"rindex("a")"#,
                Err(r#"Cannot index boolean with string "a""#),
            ),
        ]);
    }

    /// `ascii_upcase` and `ascii_downcase` are the same jq definition
    /// (`explode | map(…) | implode`), so they refuse a non-string alike.
    #[test]
    fn test_ascii_case_refusals_agree() {
        assert_outcomes(&[
            (b"1", "ascii_upcase", Err("explode input must be a string")),
            (
                b"1",
                "ascii_downcase",
                Err("explode input must be a string"),
            ),
        ]);
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

    /// jq sorts the path list before deleting anything, so the order the caller
    /// wrote the paths in cannot change the answer, and a repeated path deletes
    /// once. Deleting left to right instead let `[0]` shift the array under
    /// `[2]`, taking `30` rather than the `40` that path named (#398).
    #[test]
    fn test_delpaths_is_order_independent() {
        assert_outcomes(&[
            (b"[10,20,30,40]", "delpaths([[0],[2]])", Ok("[20,40]")),
            (b"[10,20,30,40]", "delpaths([[2],[0]])", Ok("[20,40]")),
            (b"[10,20,30,40]", "delpaths([[0],[0]])", Ok("[20,30,40]")),
            (
                b"[10,20,30,40]",
                "delpaths([[0],[0],[0]])",
                Ok("[20,30,40]"),
            ),
            (
                br#"{"a":1,"b":2,"c":3}"#,
                r#"delpaths([["a"],["c"]])"#,
                Ok(r#"{"b":2}"#),
            ),
            (
                br#"{"a":1,"b":2,"c":3}"#,
                r#"delpaths([["c"],["a"]])"#,
                Ok(r#"{"b":2}"#),
            ),
        ]);
    }

    /// The empty path names the document itself. It sorts before every other
    /// array, so it wins wherever the caller wrote it.
    #[test]
    fn test_delpaths_empty_path_deletes_the_document() {
        assert_outcomes(&[
            (b"[10,20,30,40]", "delpaths([])", Ok("[10,20,30,40]")),
            (b"[10,20,30,40]", "delpaths([[]])", Ok("null")),
            (b"[10,20,30,40]", "delpaths([[],[0]])", Ok("null")),
            (b"[10,20,30,40]", "delpaths([[0],[]])", Ok("null")),
        ]);
    }

    /// A path that ends at one level takes its whole subtree, and the longer
    /// paths under it are never walked. This is why `delpaths([[0,1]])` on
    /// `[10,20,30,40]` is `Cannot delete fields from number` in jq while
    /// `delpaths([[0],[0,1]])` is a value — the number is gone before anything
    /// tries to index it. A per-path loop would refuse both.
    #[test]
    fn test_delpaths_shorter_path_shadows_its_extensions() {
        assert_outcomes(&[
            (b"[10,20,30,40]", "delpaths([[0],[0,1]])", Ok("[20,30,40]")),
            (b"[[1,2],[3,4]]", "delpaths([[0],[0,1]])", Ok("[[3,4]]")),
            (b"[[1,2],[3,4]]", "delpaths([[0,1],[0]])", Ok("[[3,4]]")),
            (b"[[1,2],[3,4]]", "delpaths([[0,1],[1,0]])", Ok("[[1],[4]]")),
            (
                br#"{"a":{"x":1,"y":2},"b":3}"#,
                r#"delpaths([["a"],["a","x"]])"#,
                Ok(r#"{"b":3}"#),
            ),
            // The extension alone, with no shorter path to shadow it, does
            // reach the walk and raise.
            (
                b"[10,20,30,40]",
                "delpaths([[0,1]])",
                Err("Cannot delete fields from number"),
            ),
        ]);
    }

    /// Deleting inside an object leaves the enclosing key where it was. The
    /// walk replaces through the slot; a `shift_remove` and reinsert would move
    /// the key to the end of the `IndexMap`, which is only visible when another
    /// key follows the one whose child was edited.
    #[test]
    fn test_delpaths_preserves_nested_key_order() {
        assert_outcomes(&[
            (
                br#"{"a":{"x":1},"b":2}"#,
                r#"delpaths([["a","x"]])"#,
                Ok(r#"{"a":{},"b":2}"#),
            ),
            (
                br#"{"a":{"x":1,"y":2},"b":3,"c":4}"#,
                r#"delpaths([["a","x"],["b"]])"#,
                Ok(r#"{"a":{"y":2},"c":4}"#),
            ),
            (
                br#"{"a":{"x":1,"y":2},"b":3,"c":4}"#,
                r#"delpaths([["b"],["a","x"]])"#,
                Ok(r#"{"a":{"y":2},"c":4}"#),
            ),
            (
                br#"{"a":[{"x":1,"y":2},{"z":3}],"b":9}"#,
                r#"delpaths([["a",0,"x"],["a",1],["b"]])"#,
                Ok(r#"{"a":[{"y":2}]}"#),
            ),
        ]);
    }

    /// Every key that ends at one level is removed in a single pass, so each
    /// index resolves against the length the array had before any sibling went.
    /// Deleting one at a time makes `[[-1],[-2]]` yield `[10,30]`, because the
    /// second index counts back from an array that has already lost an element
    /// (#398). Indices go through `resolve_read_index`, so a float truncates
    /// toward zero and `1` and `1.0` name the same element.
    #[test]
    fn test_delpaths_resolves_indices_against_the_undeleted_array() {
        assert_outcomes(&[
            (b"[10,20,30,40]", "delpaths([[-1],[-2]])", Ok("[10,20]")),
            (b"[10,20,30,40]", "delpaths([[-1],[-3]])", Ok("[10,30]")),
            (b"[10,20,30,40]", "delpaths([[-1],[-4]])", Ok("[20,30]")),
            (b"[10,20,30,40]", "delpaths([[-2],[-4]])", Ok("[20,40]")),
            // `[3]` and `[-1]` are distinct values, so they are two groups —
            // but they resolve to one index, and deleting it twice is a no-op.
            (b"[10,20,30,40]", "delpaths([[3],[-1]])", Ok("[10,20,30]")),
            (b"[10,20,30,40]", "delpaths([[0],[-4]])", Ok("[20,30,40]")),
            (b"[10,20,30,40]", "delpaths([[-9]])", Ok("[10,20,30,40]")),
            (b"[10,20,30,40]", "delpaths([[1.7]])", Ok("[10,30,40]")),
            (b"[10,20,30,40]", "delpaths([[1],[1.0]])", Ok("[10,30,40]")),
            (b"[10,20,30,40]", "delpaths([[1.0],[1]])", Ok("[10,30,40]")),
            (
                br"[[1,2,3],[4,5,6]]",
                "delpaths([[0,-1],[0,-2]])",
                Ok("[[1],[4,5,6]]"),
            ),
        ]);
    }

    /// A path that reaches no element deletes nothing, and does not disturb the
    /// paths beside it.
    #[test]
    fn test_delpaths_ignores_keys_that_name_nothing() {
        assert_outcomes(&[
            (br#"{"a":1}"#, r#"delpaths([["b","c"]])"#, Ok(r#"{"a":1}"#)),
            (b"null", r#"delpaths([["a"]])"#, Ok("null")),
            (b"[1,2]", r#"delpaths([[5,"a"]])"#, Ok("[1,2]")),
            (
                br#"{"a":null}"#,
                r#"delpaths([["a","b"]])"#,
                Ok(r#"{"a":null}"#),
            ),
            (
                br#"{"a":{"x":1},"b":2}"#,
                r#"delpaths([["a","zz"],["a","x"]])"#,
                Ok(r#"{"a":{},"b":2}"#),
            ),
        ]);
    }

    /// A key of the wrong *kind* for its container names nothing valid at all
    /// — jq refuses it rather than treating it as absent.
    #[test]
    fn test_delpaths_rejects_wrong_type_keys() {
        assert_outcomes(&[
            // Terminal: object field named by a non-string key.
            (
                br#"{"a":1}"#,
                "delpaths([[0]])",
                Err("Cannot delete number field of object"),
            ),
            (
                br#"{"a":1}"#,
                "delpaths([[null]])",
                Err("Cannot delete null field of object"),
            ),
            (
                br#"{"a":1}"#,
                "delpaths([[true]])",
                Err("Cannot delete boolean field of object"),
            ),
            (
                br#"{"a":1}"#,
                "delpaths([[[1]]])",
                Err("Cannot delete array field of object"),
            ),
            (
                br#"{"a":1}"#,
                "delpaths([[{}]])",
                Err("Cannot delete object field of object"),
            ),
            // Terminal: array element named by a non-number key.
            (
                b"[1,2]",
                r#"delpaths([["a"]])"#,
                Err("Cannot delete string element of array"),
            ),
            (
                b"[1,2]",
                "delpaths([[true]])",
                Err("Cannot delete boolean element of array"),
            ),
            (
                b"[1,2]",
                "delpaths([[null]])",
                Err("Cannot delete null element of array"),
            ),
            (
                b"[1,2]",
                "delpaths([[[1]]])",
                Err("Cannot delete array element of array"),
            ),
            // Mid-path: non-string key navigating into an object.
            (
                br#"{"a":{"x":1}}"#,
                r#"delpaths([[0,"x"]])"#,
                Err("Cannot index object with number"),
            ),
            // Mid-path: wrong-type key navigating into an array.
            (
                b"[[1,2,3]]",
                r#"delpaths([[0,"x",1]])"#,
                Err(r#"Cannot index array with string "x""#),
            ),
            // Mid-path: a scalar reached with path left over.
            (
                br#"{"a":1}"#,
                r#"delpaths([["a","b","c"]])"#,
                Err(r#"Cannot index number with string "b""#),
            ),
            (
                br#"{"a":"hi"}"#,
                r#"delpaths([["a","b","c"]])"#,
                Err(r#"Cannot index string with string "b""#),
            ),
            // Terminal: a scalar reached with no key left to apply.
            (
                b"1",
                "delpaths([[0]])",
                Err("Cannot delete fields from number"),
            ),
            (
                b"1",
                r#"delpaths([["a"]])"#,
                Err("Cannot delete fields from number"),
            ),
            // Contrast: `null` stays a no-op at both mid-path and terminal
            // position — it has no fields to begin with, so there is nothing
            // to refuse.
            (
                br#"{"a":null}"#,
                r#"delpaths([["a","b"]])"#,
                Ok(r#"{"a":null}"#),
            ),
            (b"null", r#"delpaths([["a"]])"#, Ok("null")),
            // An object-shaped key against an array is jq's slice descriptor.
            // A malformed one — no `start`/`end`, or a non-number bound — is
            // the only place `delpaths` refuses an *object* key rather than
            // deleting a range (#366).
            (
                b"[1,2,3]",
                "delpaths([[{}]])",
                Err("Array/string slice indices must be integers"),
            ),
            (
                b"[1,2,3]",
                r#"delpaths([[{"start":1}]])"#,
                Err("Array/string slice indices must be integers"),
            ),
            (
                b"[1,2,3]",
                r#"delpaths([[{"start":"a","end":2}]])"#,
                Err("Array/string slice indices must be integers"),
            ),
            // Contrast: a well-formed one deletes the range it names.
            (
                b"[1,2,3]",
                r#"delpaths([[{"start":1,"end":2}]])"#,
                Ok("[1,3]"),
            ),
        ]);
    }

    /// A NaN component names no element, so its path is dropped — and dropped
    /// *whole*, before the sort, because (since #421) `compare_values` orders
    /// NaN as strictly less than every number, including another NaN. Left
    /// in, two NaN-headed paths would each compare `Less` than the other — a
    /// property `sort_by` is entitled to panic on — instead of the pre-#421
    /// failure mode this test used to describe (NaN comparing `Equal` to
    /// every sibling and cancelling their deletions).
    ///
    /// jq cannot arbitrate either: 1.7.1 loops forever on `delpaths([[nan]])`,
    /// which is why this is a unit test and not a golden case. What it pins is
    /// that a path succinctly cannot use costs only itself.
    #[test]
    fn test_delpaths_nan_path_does_not_cancel_its_siblings() {
        assert_outcomes(&[
            (b"[10,20,30,40]", "delpaths([[nan]])", Ok("[10,20,30,40]")),
            (b"[10,20,30,40]", "delpaths([[nan],[0]])", Ok("[20,30,40]")),
            (b"[10,20,30,40]", "delpaths([[0],[nan]])", Ok("[20,30,40]")),
            (b"[10,20,30,40]", "delpaths([[0],[nan],[2]])", Ok("[20,40]")),
            (
                b"[[1,2],[3,4]]",
                "delpaths([[0,nan],[0,1]])",
                Ok("[[1],[3,4]]"),
            ),
            // A bare NaN is not a path at all — the shape pre-pass rejects it
            // the same way any other non-array entry is rejected, before the
            // NaN-inside-a-path handling above ever runs.
            (
                b"[10,20,30,40]",
                "delpaths([nan,[0]])",
                Err("Path must be specified as array, not number"),
            ),
        ]);
    }

    /// A path list entry that is not itself an array is refused before any
    /// deletion runs — jq validates the whole list up front, so a bad entry
    /// anywhere refuses the call rather than deleting the entries that sort
    /// ahead of it.
    ///
    /// What must hold either way is that the entry is never mistaken for the
    /// empty path, which would delete the whole document.
    #[test]
    fn test_delpaths_rejects_entries_that_are_not_paths() {
        assert_outcomes(&[
            (
                b"[1,2]",
                "delpaths([0])",
                Err("Path must be specified as array, not number"),
            ),
            (
                b"[1,2]",
                r#"delpaths(["a"])"#,
                Err("Path must be specified as array, not string"),
            ),
            (
                b"[1,2]",
                "delpaths([null])",
                Err("Path must be specified as array, not null"),
            ),
            (
                b"[1,2]",
                "delpaths(null)",
                Err("Paths must be specified as an array"),
            ),
            // A bad entry sorting after a good one still refuses outright —
            // no partial deletion of the good entry happens first.
            (
                b"[10,20,30,40]",
                r#"delpaths([[0],"a"])"#,
                Err("Path must be specified as array, not string"),
            ),
            (
                b"[10,20,30,40]",
                r#"delpaths(["a",[0]])"#,
                Err("Path must be specified as array, not string"),
            ),
        ]);
    }

    #[test]
    fn test_debug() {
        // debug just passes through the value
        query!(b"42", "debug", QueryResult::Owned(v) => {
            assert_eq!(v, OwnedValue::Int(42));
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
        // env returns object of environment variables (non-empty when std feature is enabled)
        query!(b"null", "env", QueryResult::Owned(OwnedValue::Object(obj)) => {
            // In std context, env should have at least PATH
            #[cfg(feature = "std")]
            assert!(!obj.is_empty(), "env should return non-empty object in std context");
            #[cfg(not(feature = "std"))]
            assert!(obj.is_empty(), "env should return empty object in no_std context");
        });
    }

    #[test]
    fn test_dollar_env() {
        // $ENV returns object of environment variables (same as env builtin)
        query!(b"null", "$ENV", QueryResult::Owned(OwnedValue::Object(obj)) => {
            // In std context, $ENV should have at least PATH
            #[cfg(feature = "std")]
            assert!(!obj.is_empty(), "$ENV should return non-empty object in std context");
            #[cfg(not(feature = "std"))]
            assert!(obj.is_empty(), "$ENV should return empty object in no_std context");
        });
    }

    #[test]
    fn test_dollar_env_field_access() {
        // $ENV.VAR should return the environment variable value
        query!(b"null", "$ENV.PATH", QueryResult::Owned(OwnedValue::String(s)) => {
            #[cfg(feature = "std")]
            assert!(!s.is_empty(), "PATH should be non-empty");
        });
    }

    #[test]
    fn test_dollar_env_missing_var() {
        // $ENV.NONEXISTENT_VAR_12345 returns null (jq-compatible behavior)
        query!(b"null", "$ENV.NONEXISTENT_VAR_12345",
            QueryResult::Owned(OwnedValue::Null) => {}
        );

        // Optional syntax also returns null
        query!(b"null", "$ENV.NONEXISTENT_VAR_12345?",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_dollar_env_bracket_access() {
        // $ENV["PATH"] should also work
        query!(b"null", r#"$ENV["PATH"]"#, QueryResult::Owned(OwnedValue::String(s)) => {
            #[cfg(feature = "std")]
            assert!(!s.is_empty(), "PATH should be non-empty");
        });
    }

    #[test]
    fn test_env_var() {
        // env(VAR) returns the environment variable value
        // This test uses PATH which should always exist
        query!(b"null", "env(PATH)", QueryResult::Owned(OwnedValue::String(s)) => {
            #[cfg(feature = "std")]
            assert!(!s.is_empty(), "PATH should be non-empty");
        });
    }

    #[test]
    fn test_strenv() {
        // strenv(VAR) returns the environment variable value as string
        query!(b"null", "strenv(PATH)", QueryResult::Owned(OwnedValue::String(s)) => {
            #[cfg(feature = "std")]
            assert!(!s.is_empty(), "PATH should be non-empty");
        });
    }

    #[test]
    fn test_env_field_access() {
        // env.VAR should return the environment variable value (like $ENV.VAR)
        query!(b"null", "env.PATH", QueryResult::Owned(OwnedValue::String(s)) => {
            #[cfg(feature = "std")]
            assert!(!s.is_empty(), "PATH should be non-empty");
        });
    }

    #[test]
    fn test_env_bracket_access() {
        // env["PATH"] should also work (like $ENV["PATH"])
        query!(b"null", r#"env["PATH"]"#, QueryResult::Owned(OwnedValue::String(s)) => {
            #[cfg(feature = "std")]
            assert!(!s.is_empty(), "PATH should be non-empty");
        });
    }

    #[test]
    fn test_env_missing_var() {
        // env.NONEXISTENT_VAR_12345? returns null with optional syntax
        query!(b"null", "env.NONEXISTENT_VAR_12345?",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
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
        // path(expr) returns the path components to the value selected by expr
        query!(br#"{"a": 1, "b": 2}"#, "path(.a)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0], OwnedValue::String("a".into()));
            }
        );

        // Test nested path
        query!(br#"{"a": {"b": {"c": 1}}}"#, "path(.a.b.c)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::String("a".into()));
                assert_eq!(arr[1], OwnedValue::String("b".into()));
                assert_eq!(arr[2], OwnedValue::String("c".into()));
            }
        );

        // Test array index
        query!(br"[10, 20, 30]", "path(.[1])",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0], OwnedValue::Int(1));
            }
        );

        // Test negative index (preserved as-is, matching jq)
        query!(br"[10, 20, 30]", "path(.[-1])",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0], OwnedValue::Int(-1));
            }
        );

        // Test identity path
        query!(br#"{"a": 1}"#, "path(.)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty()); // Identity has no path components
            }
        );
    }

    /// "No paths at all" is no output — never the *root* path (#489).
    ///
    /// `[]` is a real answer: it is what `path(.)` returns, and the one path
    /// that always resolves. Rendering emptiness as it aimed a caller's
    /// `getpath`/`setpath`/`delpaths` at the document root, which is why this
    /// pins the variant (`None`) and not just the rendered output.
    #[test]
    fn test_path_of_nothing_is_no_output_not_the_root_path() {
        query!(br#"{"a": 1}"#, "path(empty)", QueryResult::None => {});
        // Both branches prune, so the comma composes to nothing at all.
        query!(br#""s""#, "path(.a?, .b?)", QueryResult::None => {});
        // The other direction, so a fix that returned `None` for everything
        // would not pass: an empty path is still an answer when it is meant.
        query!(br#"{"a": 1}"#, "path(.)",
            QueryResult::Owned(OwnedValue::Array(arr)) => assert!(arr.is_empty())
        );
    }

    /// A `?`-suppressed step that cannot resolve contributes *no component*.
    ///
    /// It used to leave its component behind — `"s" | path(.a?)` answered
    /// `["a"]`, a path into a string (#489). `?` is only an off switch for the
    /// step's error; a step that never happened names nothing.
    #[test]
    fn test_optional_step_that_cannot_resolve_names_no_path() {
        for filter in [
            "path(.a?)",
            "path(.[0]?)",
            "path(.[]?)",
            r#"path(.["a"]?)"#,
            "path((.a)?)",
            // `?` outside `path(...)` reaches the same walk, so it prunes too.
            "path(.a)?",
        ] {
            assert_eq!(outputs(br#""s""#, filter), Vec::<String>::new(), "{filter}");
        }
        // The suppression is per *value*, not per spelling: the same step on a
        // container it can index keeps its component.
        assert_eq!(outputs(br#"{"a": 1}"#, "path(.a?)"), [r#"["a"]"#]);
        assert_eq!(outputs(br"[1, 2]", "path(.[0]?)"), ["[0]"]);
    }

    /// A step *through* a missing key, a null, or an out-of-range index keeps
    /// the whole path — jq's `{"a":1} | path(.b.c)` is `["b","c"]` (#489).
    ///
    /// Reading a path is not walking one: `.b` on `{"a":1}` reads `null`, and
    /// `null` accepts a further step, so the path exists even though nothing
    /// is stored along it. This is what `setpath`'s auto-vivification consumes.
    #[test]
    fn test_path_survives_a_step_that_reads_null() {
        assert_eq!(outputs(br#"{"a": 1}"#, "path(.b.c)"), [r#"["b","c"]"#]);
        assert_eq!(outputs(br"{}", "path(.a.b)"), [r#"["a","b"]"#]);
        assert_eq!(outputs(br"null", "path(.a.b)"), [r#"["a","b"]"#]);
        assert_eq!(outputs(br"[1]", "path(.[3].x)"), [r#"[3,"x"]"#]);
        // Including under `?`, which suppresses errors and nothing else — a
        // step that reads `null` never errored in the first place.
        assert_eq!(outputs(br#"{"a": 1}"#, "path(.b.c?)"), [r#"["b","c"]"#]);
        assert_eq!(outputs(br"[1, 2]", "path(.[5]?)"), ["[5]"]);
    }

    /// Without `?`, a step that cannot index its value raises jq's own
    /// sentence rather than inventing a component for it (#489).
    ///
    /// The wording is not spelled here: it comes from the value evaluator, so
    /// `path(f)` reports exactly what `f` reports. `tests/jq_error_message_tests.rs`
    /// is what pins the text against real jq.
    #[test]
    fn test_unindexable_step_errors_instead_of_inventing_a_path() {
        for (json, filter, message) in [
            (
                &br#""s""#[..],
                "path(.a)",
                r#"Cannot index string with string "a""#,
            ),
            (
                &br#""s""#[..],
                "path(.[0])",
                "Cannot index string with number",
            ),
            (
                &br#"{"a": 1}"#[..],
                "path(.a.b)",
                r#"Cannot index number with string "b""#,
            ),
            (
                &br#"{"a": 1}"#[..],
                "path(.[0])",
                "Cannot index object with number",
            ),
            (
                &br#"{"a": 1}"#[..],
                "path(.[1:2])",
                "Cannot index object with object",
            ),
            (
                &br"[1, 2]"[..],
                "path(.a)",
                r#"Cannot index array with string "a""#,
            ),
            (
                &br#""s""#[..],
                "path(.[])",
                r#"Cannot iterate over string ("s")"#,
            ),
            (
                &br"null"[..],
                "path(.[])",
                "Cannot iterate over null (null)",
            ),
        ] {
            query!(json, filter,
                QueryResult::Error(e) => assert_eq!(e.message, message, "{filter}")
            );
        }
    }

    #[test]
    fn test_paths_filter() {
        // paths(filter) streams paths where values match filter
        query!(br#"{"a": 1, "b": "hello", "c": 2}"#, "paths(type == \"number\")",
            QueryResult::ManyOwned(paths) => {
                // Should have paths to "a" and "c" (both numbers)
                assert_eq!(paths.len(), 2);
                assert_eq!(paths[0], OwnedValue::Array(vec![OwnedValue::String("a".into())]));
                assert_eq!(paths[1], OwnedValue::Array(vec![OwnedValue::String("c".into())]));
            }
        );
    }

    #[test]
    fn test_paths_filter_single() {
        // Single match returns single Owned result
        query!(br#"{"a": 1, "b": "hello"}"#, "paths(type == \"number\")",
            QueryResult::Owned(OwnedValue::Array(path)) => {
                assert_eq!(path, vec![OwnedValue::String("a".into())]);
            }
        );
    }

    #[test]
    fn test_paths_filter_none() {
        // No matches returns None
        query!(br#"{"a": "x", "b": "y"}"#, "paths(type == \"number\")",
            QueryResult::None => {}
        );
    }

    #[test]
    fn test_paths_filter_collected() {
        // Collected with [...] matches jq
        query!(br#"{"a": 1, "b": "hello", "c": 2}"#, "[paths(type == \"number\")]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
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

    // The following tostream/fromstream/truncate_stream expectations are all
    // verified against jq-1.7.1 (#396) — not derived from this codebase's own
    // output.

    #[test]
    fn test_tostream_object() {
        assert_eq!(
            outputs(br#"{"a":1,"b":[2,3]}"#, "tostream"),
            [
                r#"[["a"],1]"#,
                r#"[["b",0],2]"#,
                r#"[["b",1],3]"#,
                r#"[["b",1]]"#,
                r#"[["b"]]"#,
            ]
        );
    }

    #[test]
    fn test_tostream_nested() {
        assert_eq!(
            outputs(br#"{"a":{"b":1,"c":2}}"#, "tostream"),
            [
                r#"[["a","b"],1]"#,
                r#"[["a","c"],2]"#,
                r#"[["a","c"]]"#,
                r#"[["a"]]"#,
            ]
        );
    }

    #[test]
    fn test_tostream_empty_containers_are_leaves() {
        assert_eq!(
            outputs(br#"{"a":[],"b":{},"c":1}"#, "tostream"),
            [
                r#"[["a"],[]]"#,
                r#"[["b"],{}]"#,
                r#"[["c"],1]"#,
                r#"[["c"]]"#,
            ]
        );
    }

    #[test]
    fn test_tostream_top_level_scalar() {
        assert_eq!(outputs(b"1", "tostream"), [r"[[],1]"]);
    }

    #[test]
    fn test_tostream_top_level_empty_array() {
        assert_eq!(outputs(b"[]", "tostream"), [r"[[],[]]"]);
    }

    #[test]
    fn test_tostream_top_level_empty_object() {
        assert_eq!(outputs(b"{}", "tostream"), [r"[[],{}]"]);
    }

    #[test]
    fn test_fromstream_roundtrip_object() {
        assert_eq!(
            outputs(br#"{"a":1,"b":[2,3]}"#, "fromstream(tostream)"),
            [r#"{"a":1,"b":[2,3]}"#]
        );
    }

    #[test]
    fn test_fromstream_roundtrip_array() {
        assert_eq!(outputs(b"[10,20]", "fromstream(tostream)"), ["[10,20]"]);
    }

    #[test]
    fn test_fromstream_roundtrip_scalar() {
        assert_eq!(outputs(b"1", "fromstream(tostream)"), ["1"]);
    }

    #[test]
    fn test_fromstream_multiple_values() {
        // Exercises the state-reset-on-completion path with two independent
        // top-level values in one event stream, not just a single round trip.
        assert_eq!(
            outputs(
                br#"[[["a"],1],[["a"]],[["b"],2],[["b"]]]"#,
                "fromstream(.[])"
            ),
            [r#"{"a":1}"#, r#"{"b":2}"#]
        );
    }

    #[test]
    fn test_truncate_stream_depth1() {
        assert_eq!(
            outputs(
                br#"{"a":{"b":1,"c":2}}"#,
                ". as $doc | 1 | truncate_stream($doc|tostream)"
            ),
            [r#"[["b"],1]"#, r#"[["c"],2]"#, r#"[["c"]]"#]
        );
    }

    #[test]
    fn test_truncate_stream_depth0_is_identity() {
        assert_eq!(
            outputs(
                br#"{"a":{"b":1,"c":2}}"#,
                ". as $doc | 0 | truncate_stream($doc|tostream)"
            ),
            outputs(br#"{"a":{"b":1,"c":2}}"#, "tostream")
        );
    }

    #[test]
    fn test_truncate_stream_then_fromstream() {
        // The documented idiom for pulling one sub-object out of a stream.
        assert_eq!(
            outputs(
                br#"{"a":{"b":1,"c":2},"d":9}"#,
                ". as $doc | 1 | fromstream(truncate_stream($doc|tostream))"
            ),
            [r#"{"b":1,"c":2}"#]
        );
    }

    #[test]
    fn test_truncate_stream_null_depth_keeps_everything() {
        // jq's own definition compares path length against `$n` with the
        // generic `>` operator, not a type check: every length sorts above
        // `null` in jq's ordering, so a null depth keeps every event
        // unmodified rather than erroring on a non-numeric depth.
        assert_eq!(
            outputs(
                br#"{"a":{"b":1,"c":2}}"#,
                "null | truncate_stream(([[[\"a\",\"b\"],1],[[\"a\",\"c\"],2]])|.[])"
            ),
            [r#"[["a","b"],1]"#, r#"[["a","c"],2]"#]
        );
    }

    #[test]
    fn test_getpath() {
        // getpath returns Owned Int, not One StandardJson
        query!(br#"{"a": {"b": 42}}"#, r#"getpath(["a", "b"])"#,
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(42));
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
        query!(b"42", r#"debug("test message")"#, QueryResult::Owned(v) => {
            assert_eq!(v, OwnedValue::Int(42));
        });
    }

    #[test]
    fn test_query_result_collect_owned_and_is_error() {
        // Covers the None / Error / Break / Owned / ManyOwned arms and is_error,
        // which the integration parity tests do not exercise.
        let none: QueryResult<Vec<u64>> = QueryResult::None;
        assert!(none.collect_owned().is_empty());
        assert!(!QueryResult::<Vec<u64>>::None.is_error());

        let err: QueryResult<Vec<u64>> = QueryResult::Error(EvalError::new("boom"));
        assert!(err.is_error());
        assert!(err.collect_owned().is_empty());

        let brk: QueryResult<Vec<u64>> = QueryResult::Break("lbl".into());
        assert!(brk.collect_owned().is_empty());

        let owned: QueryResult<Vec<u64>> = QueryResult::Owned(OwnedValue::Int(7));
        assert_eq!(owned.collect_owned(), vec![OwnedValue::Int(7)]);

        let many: QueryResult<Vec<u64>> =
            QueryResult::ManyOwned(vec![OwnedValue::Int(1), OwnedValue::Int(2)]);
        assert_eq!(many.collect_owned().len(), 2);
    }

    #[test]
    fn test_query_result_collect_owned_cursor_arms() {
        // Covers the One / OneCursor / Many arms via real evaluation.
        fn owned(json: &[u8], filter: &str) -> Vec<OwnedValue> {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let expr = parse(filter).expect("parse failed");
            eval::<Vec<u64>, JqSemantics>(&expr, cursor).collect_owned()
        }
        // OneCursor: identity passes a container through unchanged.
        assert_eq!(owned(br#"{"a":1}"#, ".").len(), 1);
        // Many: iterating an array yields several values.
        assert_eq!(owned(br"[1,2,3]", ".[]").len(), 3);
        // One: field access returns a scalar reference.
        assert_eq!(owned(br#"{"a":1}"#, ".a"), vec![OwnedValue::Int(1)]);
    }

    // =============================================
    // Date/Time function tests (Phase 15)
    // =============================================

    #[test]
    fn test_localtime_structure() {
        // localtime is timezone-dependent, so assert only the 8-field structure.
        // This still exercises the hour/minute/second/weekday computation
        // regardless of the host's $TZ.
        query!(b"0", r"localtime",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 8);
            }
        );
    }

    #[test]
    fn test_gmtime() {
        // Unix epoch (Jan 1, 1970 00:00:00 UTC)
        query!(b"0", r"gmtime",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 8);
                assert_eq!(arr[0], OwnedValue::Int(1970)); // year
                assert_eq!(arr[1], OwnedValue::Int(0));    // month (0-indexed)
                assert_eq!(arr[2], OwnedValue::Int(1));    // day
                assert_eq!(arr[3], OwnedValue::Int(0));    // hour
                assert_eq!(arr[4], OwnedValue::Int(0));    // minute
                assert_eq!(arr[5], OwnedValue::Int(0));    // second
                assert_eq!(arr[6], OwnedValue::Int(4));    // weekday (Thursday)
                assert_eq!(arr[7], OwnedValue::Int(0));    // yearday
            }
        );
    }

    #[test]
    fn test_mktime() {
        // Round-trip: gmtime | mktime should return original timestamp
        query!(b"[1970,0,1,0,0,0,4,0]", r"mktime",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 0.0);
            }
        );

        // Jan 15, 2024 10:30:00 UTC
        query!(b"[2024,0,15,10,30,0,1,14]", r"mktime",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 1705314600.0);
            }
        );
    }

    #[test]
    fn test_strftime() {
        query!(b"[2024,0,15,10,30,0,1,14]", r#"strftime("%Y-%m-%d")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15");
            }
        );

        query!(b"[2024,0,15,10,30,0,1,14]", r#"strftime("%Y-%m-%dT%H:%M:%SZ")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T10:30:00Z");
            }
        );
    }

    #[test]
    fn test_strptime() {
        query!(br#""2024-01-15T10:30:00Z""#, r#"strptime("%Y-%m-%dT%H:%M:%SZ")"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 8);
                assert_eq!(arr[0], OwnedValue::Int(2024)); // year
                assert_eq!(arr[1], OwnedValue::Int(0));    // month (0-indexed)
                assert_eq!(arr[2], OwnedValue::Int(15));   // day
                assert_eq!(arr[3], OwnedValue::Int(10));   // hour
                assert_eq!(arr[4], OwnedValue::Int(30));   // minute
                assert_eq!(arr[5], OwnedValue::Int(0));    // second
            }
        );
    }

    #[test]
    fn test_todate() {
        // todate converts timestamp to ISO 8601 string
        query!(b"0", r"todate",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "1970-01-01T00:00:00Z");
            }
        );
    }

    #[test]
    fn test_fromdate() {
        // fromdate parses ISO 8601 string to timestamp
        query!(br#""1970-01-01T00:00:00Z""#, r"fromdate",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 0.0);
            }
        );

        query!(br#""2024-01-15T10:30:00Z""#, r"fromdate",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 1705314600.0);
            }
        );
    }

    #[test]
    fn test_date_roundtrip() {
        // gmtime | mktime should return original value
        query!(b"1705314600", r"gmtime | mktime",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 1705314600.0);
            }
        );

        // todate | fromdate should return original value
        query!(b"1705314600", r"todate | fromdate",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 1705314600.0);
            }
        );
    }

    // =============================================
    // Phase 21: Extended Date/Time functions (yq)
    // =============================================

    #[test]
    fn test_from_unix() {
        // from_unix converts timestamp to ISO 8601 string (same as todate)
        query!(b"0", r"from_unix",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "1970-01-01T00:00:00Z");
            }
        );

        query!(b"1705314600", r"from_unix",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T10:30:00Z");
            }
        );
    }

    #[test]
    fn test_to_unix() {
        // to_unix parses ISO 8601 string to timestamp (same as fromdate)
        query!(br#""1970-01-01T00:00:00Z""#, r"to_unix",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 0.0);
            }
        );

        query!(br#""2024-01-15T10:30:00Z""#, r"to_unix",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 1705314600.0);
            }
        );
    }

    #[test]
    fn test_from_unix_to_unix_roundtrip() {
        // from_unix | to_unix should return original value
        query!(b"1705314600", r"from_unix | to_unix",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                assert_eq!(n, 1705314600.0);
            }
        );
    }

    #[test]
    fn test_tz_utc() {
        // tz("UTC") should work like todate
        query!(b"1705314600", r#"tz("UTC")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T10:30:00Z");
            }
        );

        // tz("GMT") is an alias for UTC
        query!(b"1705314600", r#"tz("GMT")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T10:30:00Z");
            }
        );
    }

    #[test]
    fn test_tz_abbreviations() {
        // Test common timezone abbreviations
        // 1705314600 = 2024-01-15T10:30:00Z

        // EST is UTC-5
        query!(b"1705314600", r#"tz("EST")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T05:30:00-05:00");
            }
        );

        // PST is UTC-8
        query!(b"1705314600", r#"tz("PST")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T02:30:00-08:00");
            }
        );

        // JST is UTC+9
        query!(b"1705314600", r#"tz("JST")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T19:30:00+09:00");
            }
        );

        // CET is UTC+1
        query!(b"1705314600", r#"tz("CET")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T11:30:00+01:00");
            }
        );
    }

    #[test]
    fn test_tz_iana_names() {
        // Test IANA-style timezone names
        // 1705314600 = 2024-01-15T10:30:00Z (January, so standard time in most places)

        query!(b"1705314600", r#"tz("Asia/Tokyo")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T19:30:00+09:00");
            }
        );

        query!(b"1705314600", r#"tz("Europe/Moscow")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T13:30:00+03:00");
            }
        );

        query!(b"1705314600", r#"tz("Pacific/Honolulu")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T00:30:00-10:00");
            }
        );
    }

    #[test]
    fn test_tz_numeric_offset() {
        // Test numeric offset format
        query!(b"1705314600", r#"tz("+05:30")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T16:00:00+05:30");
            }
        );

        query!(b"1705314600", r#"tz("-0800")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T02:30:00-08:00");
            }
        );

        query!(b"1705314600", r#"tz("+09")"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T19:30:00+09:00");
            }
        );
    }

    #[test]
    fn test_tz_unknown_error() {
        // Unknown timezone should error
        query!(b"1705314600", r#"tz("Unknown/Timezone")"#,
            QueryResult::Error(e) => {
                assert!(e.to_string().contains("unknown timezone"));
            }
        );
    }

    #[test]
    fn test_tz_with_variable() {
        // tz should work with a variable expression
        query!(b"1705314600", r#""JST" as $tz | tz($tz)"#,
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "2024-01-15T19:30:00+09:00");
            }
        );
    }

    // =============================================
    // Assignment operator tests
    // =============================================

    #[test]
    fn test_simple_assign() {
        // Simple assignment: .a = value
        query!(br#"{"a": 1, "b": 2}"#, r".a = 42",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(42));
                let b = obj.get("b").unwrap();
                assert_eq!(*b, OwnedValue::Int(2));
            }
        );
    }

    #[test]
    fn test_nested_assign() {
        // Nested assignment: .a.b = value
        query!(br#"{"a": {"b": 1}}"#, r".a.b = 99",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                if let OwnedValue::Object(inner) = a {
                    let b = inner.get("b").unwrap();
                    assert_eq!(*b, OwnedValue::Int(99));
                } else {
                    panic!("Expected nested object");
                }
            }
        );
    }

    #[test]
    fn test_array_index_assign() {
        // Array index assignment: .[0] = value
        query!(br"[1, 2, 3]", r".[1] = 99",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(99));
                assert_eq!(arr[2], OwnedValue::Int(3));
            }
        );
    }

    #[test]
    fn test_update_assign() {
        // Update assignment: .a |= . + 1
        query!(br#"{"a": 5}"#, r".a |= . + 1",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(6));
            }
        );
    }

    #[test]
    fn test_update_assign_array() {
        // Update assignment on array elements: .[] |= . * 2
        query!(br"[1, 2, 3]", r".[] |= . * 2",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr[0], OwnedValue::Int(2));
                assert_eq!(arr[1], OwnedValue::Int(4));
                assert_eq!(arr[2], OwnedValue::Int(6));
            }
        );
    }

    #[test]
    fn test_compound_assign_add() {
        // Compound assignment: .a += 10
        query!(br#"{"a": 5}"#, r".a += 10",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(15));
            }
        );
    }

    #[test]
    fn test_compound_assign_sub() {
        // Compound subtraction: .a -= 3
        query!(br#"{"a": 10}"#, r".a -= 3",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(7));
            }
        );
    }

    #[test]
    fn test_compound_assign_mul() {
        // Compound multiplication: .a *= 4
        query!(br#"{"a": 5}"#, r".a *= 4",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(20));
            }
        );
    }

    #[test]
    fn test_compound_assign_div() {
        // Compound division: .a /= 2
        // Division returns float even for integer inputs
        query!(br#"{"a": 10}"#, r".a /= 2",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                match a {
                    OwnedValue::Float(f) => assert!((f - 5.0).abs() < 0.001),
                    OwnedValue::Int(i) => assert_eq!(*i, 5),
                    _ => panic!("Expected number, got {a:?}"),
                }
            }
        );
    }

    #[test]
    fn test_compound_assign_mod() {
        // Compound modulo: .a %= 3
        query!(br#"{"a": 10}"#, r".a %= 3",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(1));
            }
        );
    }

    #[test]
    fn test_compound_assign_mod_float_truncates() {
        // jq truncates float operands: 10.5 %= 3 leaves 10 % 3 == 1
        query!(br#"{"a": 10.5}"#, r".a %= 3",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(1));
            }
        );
    }

    #[test]
    fn test_alternative_assign() {
        // Alternative assignment: .a //= "default" (when .a is null)
        query!(br#"{"a": null}"#, r#".a //= "default""#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::String("default".to_string()));
            }
        );
    }

    #[test]
    fn test_alternative_assign_existing() {
        // Alternative assignment should not change non-null values
        query!(br#"{"a": "existing"}"#, r#".a //= "default""#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::String("existing".to_string()));
            }
        );
    }

    #[test]
    fn test_compound_assign_references_root() {
        // Regression test for #159: the RHS of `.a += .b` must be evaluated
        // against the root, not against the sub-value at `.a`.
        query!(br#"{"a": 1, "b": 2}"#, r".a += .b",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(*obj.get("a").unwrap(), OwnedValue::Int(3));
                assert_eq!(*obj.get("b").unwrap(), OwnedValue::Int(2));
            }
        );
    }

    #[test]
    fn test_alternative_assign_references_root() {
        // Regression test for #159: the RHS of `.a //= .b` must be evaluated
        // against the root, not against the sub-value at `.a`.
        query!(br#"{"a": null, "b": 5}"#, r".a //= .b",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(*obj.get("a").unwrap(), OwnedValue::Int(5));
                assert_eq!(*obj.get("b").unwrap(), OwnedValue::Int(5));
            }
        );
    }

    #[test]
    fn test_compound_assign_multi_path_freezes_rhs() {
        // Regression test for #159: the RHS is evaluated once against the
        // pristine root before any path is updated, then reused for every
        // element `.a[]` resolves to -- not re-evaluated per element against
        // the already-updated root. Confirmed against real jq (jq-1.7.1):
        // `{"a":[1,2,3]} | .a[] += .a[0]` yields `{"a":[2,3,4]}` (every
        // element gets the original `.a[0]` == 1 added), not `[2,4,6]` (which
        // is what re-reading a progressively-updated `.a[0]` would produce).
        query!(br#"{"a": [1, 2, 3]}"#, r".a[] += .a[0]",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let arr = obj.get("a").unwrap();
                assert_eq!(*arr, OwnedValue::Array(vec![
                    OwnedValue::Int(2),
                    OwnedValue::Int(3),
                    OwnedValue::Int(4),
                ]));
            }
        );
    }

    #[test]
    fn test_assign_autovivifies_through_null_and_missing_fields() {
        // #486: a write through a missing or `null` intermediate builds the
        // container the path implies, exactly as `setpath()` already does,
        // instead of erroring `Cannot index null with string "..."`.
        assert_outcomes(&[
            (b"null", ".a = 1", Ok(r#"{"a":1}"#)),
            (b"{}", ".a.b = 9", Ok(r#"{"a":{"b":9}}"#)),
            (br#"{"a":null}"#, ".a.b.c = 9", Ok(r#"{"a":{"b":{"c":9}}}"#)),
            (b"{}", ".a[0] = 9", Ok(r#"{"a":[9]}"#)),
        ]);
    }

    #[test]
    fn test_assign_autovivifies_and_pads_through_null_index() {
        // Sibling of the field case above, for a numeric index: `null`
        // becomes an array, and an index past the end pads with `null`s
        // rather than erroring — matching `setpath([5]; 9)`.
        assert_outcomes(&[
            (b"null", ".[0] = 9", Ok("[9]")),
            (b"[1,2]", ".[5] = 9", Ok("[1,2,null,null,null,9]")),
            (br#"{"a":[]}"#, ".a[0] = 9", Ok(r#"{"a":[9]}"#)),
        ]);
    }

    #[test]
    fn test_update_compound_and_alternative_assign_autovivify_too() {
        // The same auto-vivification applies to every write operator that
        // funnels through `update_path`, not just plain `=`.
        assert_outcomes(&[
            (b"{}", ".a.b |= 9", Ok(r#"{"a":{"b":9}}"#)),
            (b"{}", ".a.b += 1", Ok(r#"{"a":{"b":1}}"#)),
            (b"{}", ".a.b //= 9", Ok(r#"{"a":{"b":9}}"#)),
            (b"[1,2]", ".[5] |= 9", Ok("[1,2,null,null,null,9]")),
            // `|=`/`+=`/etc. walk a path recursively rather than looping like
            // `get_path_mut` does for plain `=`, so a mid-path (not the last
            // segment) `Index` step is a separate call site with its own
            // autovivify_array + write_index calls — exercised only by an
            // Index step that still has more path left after it.
            (b"{}", ".a[0].b |= 9", Ok(r#"{"a":[{"b":9}]}"#)),
            (b"[{},{}]", ".[0].x += 1", Ok(r#"[{"x":1},{}]"#)),
        ]);
    }

    #[test]
    fn test_optional_assign_through_autovivified_null_no_longer_leaves_a_remnant() {
        // Before #486/#498, `?` swallowed the write failure but not before
        // `get_path_mut`'s `or_insert(Null)` had already created the key, so
        // `.a[.k]? = 5` and `.a.b? = 5` left `"a":null` behind instead of
        // either the full write or an untouched input. Now that the write
        // itself succeeds, there is nothing left for `?` to swallow.
        assert_outcomes(&[
            (
                br#"{"k":"z"}"#,
                ".a[.k]? = 5",
                Ok(r#"{"k":"z","a":{"z":5}}"#),
            ),
            (b"{}", ".a.b? = 5", Ok(r#"{"a":{"b":5}}"#)),
        ]);
    }

    #[test]
    fn test_optional_write_still_raises_on_negative_index_out_of_bounds() {
        // #498: `?` suppresses errors raised while *collecting* a path, but
        // not the write-time bounds check on a still-negative array index —
        // jq raises `.[-5]? = 9` even though the trailing `?` would swallow
        // any other indexing failure at that position.
        assert_outcomes(&[
            (
                b"[1,2]",
                ".[-5]? = 9",
                Err("Out of bounds negative array index"),
            ),
            (
                br#"{"a":[1,2]}"#,
                ".a[-5]? = 9",
                Err("Out of bounds negative array index"),
            ),
            // Unaffected: `?` on a genuine type mismatch (not an OOB index)
            // still suppresses, same as before.
            (b"5", ".a? = 1", Ok("5")),
        ]);
    }

    #[test]
    fn test_optional_write_does_not_protect_the_filter_or_the_whole_expression() {
        // #498's rule applies just as much to `|=`'s filter as to the write
        // itself: `?` on a path component prunes path *production*, and
        // running the filter at the resolved location is path *application*
        // — jq raises `boom` here rather than leaving `.a` untouched or
        // swallowing the error. (Before this fix, `update_path` threaded the
        // component's own `optional` into `eval_owned_expr` too, so the
        // filter's `error("boom")` was itself evaluated as if `?`-guarded and
        // silently produced `null`, corrupting `.a` to `{"a":null}` instead
        // of raising.)
        assert_outcomes(&[(br#"{"a":1}"#, ".a? |= error(\"boom\")", Err("boom"))]);
    }

    #[test]
    fn test_outer_optional_around_a_failing_write_produces_no_output() {
        // A `?` wrapping the *whole* `path op value` expression is ordinary
        // `try/catch`, unrelated to a `?` written on a path component —
        // jq's answer is no output at all, not the unchanged input and not a
        // raised error. `eval_assign`/`eval_update` catch every failure at
        // this boundary (mirroring `builtin_del`'s #537 fix) rather than
        // threading their own `optional` into `set_path`/`update_path` as a
        // starting flag, which is what let this either raise unconditionally
        // (`=`, never fixed before this change) or leak a corrupted partial
        // write (`|=`, once `?` also had to stop protecting the filter).
        assert_outcomes(&[
            (b"[1,2]", "(.[-5] = 9)?", Ok("")),
            (b"[1,2]", "(.[-5] |= 9)?", Ok("")),
            (b"[1,2]", "(.[-5] += 1)?", Ok("")),
            (br#"{"a":1}"#, "(.a |= error(\"boom\"))?", Ok("")),
        ]);
    }

    #[test]
    fn test_optional_midchain_walking_failure_stays_scoped_to_its_own_component() {
        // `get_path_mut` used to drop every inline `?` on a non-final path
        // component ("every path reaching a walker has already been
        // resolved" was only true of the computed-key pre-pass, which a
        // plain static chain like `.a?.b` never goes through), so
        // `"str" | .a?.b = 1` raised `Cannot index string with string "a"`
        // instead of leaving `"str"` untouched like jq.
        assert_outcomes(&[(br#""str""#, ".a?.b = 1", Ok(r#""str""#))]);

        // A component's own `?` protects only *that* component: `.a?`
        // resolves fine here (`.a` already holds `5`, reading an existing
        // object field never fails), so it is never exercised, and the
        // unprotected `.c` still raises on the number it lands on.
        assert_outcomes(&[(
            br#"{"a":{"b":5}}"#,
            ".a?.b.c |= 1",
            Err(r#"Cannot index number with string "c""#),
        )]);
    }

    #[test]
    fn test_optional_write_does_not_swallow_a_sibling_clobber_in_a_fan_out() {
        // #498's multi-branch case: `?` prunes path *production*, never
        // *application* — and that holds even for a path that resolved
        // cleanly, once it is one of several a fan-out applies in sequence.
        // `.. | objects` here visits the root and `.a`, both objects whose
        // own `.k` names an existing field, so `.[.k]?` on each resolves
        // fully against the *original* document (`path()` sees `["a"]` then
        // `["a","a"]`, no pruning). The first write (`.a = 7`) turns `.a`
        // from an object into a number; the second (`.a.a`) then needs `.a`
        // to still be an object and raises, matching jq. Before this fix,
        // the resolved path's leftover `Expr::Optional` marker (from
        // `resolve_index_expr`/`resolve_node`, meant only to record that a
        // `?` was involved in producing the branch) was still being
        // consulted by `set_path`/`update_path` at write time, so the raise
        // was swallowed and `.a` was left at `7` instead.
        let doc = br#"{"k":"a","a":{"k":"a","a":1}}"#;
        assert_outcomes(&[
            (
                doc,
                "(.. | objects | .[.k]?) = 7",
                Err(r#"Cannot index number with string "a""#),
            ),
            (
                doc,
                "(.. | objects | .[.k]?) |= 7",
                Err(r#"Cannot index number with string "a""#),
            ),
        ]);

        // The purely static flavour, reached without any computed key at
        // all: `resolve_seq`'s no-dynamic-component fast path splices a
        // chain like `.a.a?` straight through unresolved rather than routing
        // it through `resolve_node`, so it needed its own fix at
        // `resolve_dynamic_indexes`'s assembly point rather than only at the
        // two sites that build a resolved component from a computed key.
        assert_outcomes(&[(
            br#"{"a":{"a":1}}"#,
            "(.a, .a.a?) = 7",
            Err(r#"Cannot index number with string "a""#),
        )]);
    }

    #[test]
    fn test_set_path_and_get_path_mut_autovivify_null_directly() {
        // Direct-function coverage for the three walkers `autovivify_object`/
        // `autovivify_array`/`write_index` touch (#486), bypassing the parser.
        let mut root = OwnedValue::Null;
        set_path(&mut root, &Expr::Field("a".to_string()), OwnedValue::Int(1)).unwrap();
        assert_eq!(
            root,
            OwnedValue::Object(IndexMap::from([("a".to_string(), OwnedValue::Int(1),)]))
        );

        let mut root = OwnedValue::Null;
        set_path(&mut root, &Expr::Index(0), OwnedValue::Int(1)).unwrap();
        assert_eq!(root, OwnedValue::Array(vec![OwnedValue::Int(1)]));

        let mut root = OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]);
        set_path(&mut root, &Expr::Index(4), OwnedValue::Int(9)).unwrap();
        assert_eq!(
            root,
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Null,
                OwnedValue::Null,
                OwnedValue::Int(9),
            ])
        );

        // `get_path_mut` vivifies each intermediate as it walks a multi-part
        // parent path, not just the first step.
        let mut root = OwnedValue::Object(IndexMap::new());
        let slot = get_path_mut(
            &mut root,
            &[Expr::Field("a".to_string()), Expr::Field("b".to_string())],
        )
        .unwrap()
        .unwrap();
        assert_eq!(*slot, OwnedValue::Null);
        assert_eq!(
            root,
            OwnedValue::Object(IndexMap::from([(
                "a".to_string(),
                OwnedValue::Object(IndexMap::from([("b".to_string(), OwnedValue::Null)])),
            )]))
        );
    }

    #[test]
    fn test_del_field() {
        // del(.a) removes a field
        query!(br#"{"a": 1, "b": 2}"#, r"del(.a)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert!(!obj.contains_key("a"));
                assert!(obj.contains_key("b"));
            }
        );
    }

    #[test]
    fn test_del_array_element() {
        // del(.[1]) removes an array element
        query!(br"[1, 2, 3]", r"del(.[1])",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(3));
            }
        );
    }

    #[test]
    fn test_del_nested() {
        // del(.a.b) removes nested field
        query!(br#"{"a": {"b": 1, "c": 2}}"#, r"del(.a.b)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                if let OwnedValue::Object(inner) = a {
                    assert!(!inner.contains_key("b"));
                    assert!(inner.contains_key("c"));
                } else {
                    panic!("Expected nested object");
                }
            }
        );
    }

    #[test]
    fn test_del_through_a_missing_field_walks_the_rest_against_null() {
        // #527: a field the object doesn't have reads as `null`, and jq keeps
        // walking the rest of the path against that `null` rather than
        // stopping at it — so the *tail* decides whether this is a no-op.
        // These all reach `delete_at_path`'s `Expr::Pipe` chain-walk, which
        // used to raise succinctly's own `field 'b' not found`.
        query!(br#"{"a": {"x": 1}}"#, r"del(.a.b.c)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let OwnedValue::Object(inner) = obj.get("a").unwrap() else {
                    panic!("expected nested object");
                };
                assert!(inner.contains_key("x"));
                // Walking through the absent key must not materialise it.
                assert!(!inner.contains_key("b"), "del() created the missing key");
            }
        );
        // The missing key can be the first component too.
        query!(br#"{"a": 1}"#, r"del(.b.c)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert!(obj.contains_key("a"));
                assert!(!obj.contains_key("b"), "del() created the missing key");
            }
        );
        // Any length of `Field` tail, and an `Index`/`Slice` tail, stay
        // no-ops: `null` tolerates all three (#476).
        for filter in [r"del(.a.b.c.d)", r"del(.a.b[0])", r"del(.a.b[1:2])"] {
            query!(br#"{"a": {"x": 1}}"#, filter,
                QueryResult::Owned(OwnedValue::Object(obj)) => {
                    let OwnedValue::Object(inner) = obj.get("a").unwrap() else {
                        panic!("expected nested object");
                    };
                    assert_eq!(inner.len(), 1, "`{filter}` changed the object");
                }
            );
        }
        // An `[]` tail is the exception — iterating `null` raises in jq, and
        // a `?` on the *missing* step does not suppress what the tail itself
        // raises.
        query!(br#"{"a": {"x": 1}}"#, r"del(.a.b[])",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot iterate over null (null)");
            }
        );
        query!(br#"{"a": {"x": 1}}"#, r"del(.a.b?[])",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot iterate over null (null)");
            }
        );
        // `?` on the iterate step itself does suppress it.
        query!(br#"{"a": {"x": 1}}"#, r"del(.a.b[]?)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let OwnedValue::Object(inner) = obj.get("a").unwrap() else {
                    panic!("expected nested object");
                };
                assert_eq!(inner.len(), 1);
            }
        );
        // The `[]` can be any distance past the absent key — the walk has to
        // survive more than one step, which is what makes this a walk rather
        // than a one-off check of the very next component.
        for filter in [r"del(.a.b.c[])", r"del(.a.b[0][])", r"del(.a.b[1:2][])"] {
            query!(br#"{"a": {"x": 1}}"#, filter,
                QueryResult::Error(e) => {
                    assert_eq!(e.message, "Cannot iterate over null (null)", "`{filter}`");
                }
            );
        }
    }

    #[test]
    fn test_del_through_null_still_raises_on_an_iterate_tail() {
        // #476 exempted `null` from `Field`/`Index`/`Slice` steps but
        // explicitly not from `.[]`, which jq refuses on `null` — and the
        // exemption is per *step*, so it cannot be granted by returning early
        // and skipping whatever the path still had to say. #527: every one of
        // these used to be a silent no-op.
        for (json, filter) in [
            (&br"null"[..], r"del(.a[])"),
            (&br"null"[..], r"del(.a.b[])"),
            (&br"null"[..], r"del(.[0][])"),
            (&br"null"[..], r"del(.[0:2][])"),
            (&br#"{"x": null}"#[..], r"del(.x.a[])"),
            (&br#"{"a": {"b": null}}"#[..], r"del(.a.b.c[])"),
        ] {
            query!(json, filter,
                QueryResult::Error(e) => {
                    assert_eq!(e.message, "Cannot iterate over null (null)", "`{filter}`");
                }
            );
        }
        // Every other tail keeps the #476 no-op it already had.
        for filter in [r"del(.a.b)", r"del(.[0].a)", r"del(.[0:2].a)"] {
            query!(br"null", filter,
                QueryResult::Owned(OwnedValue::Null) => {}
            );
        }
    }

    #[test]
    fn test_del_field_through_null_is_a_no_op() {
        // #476: `null` tolerates any field key, so `null | del(.a)` is `null`
        // rather than `Cannot index null with string "a"`, matching jq.
        query!(br"null", r"del(.a)",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
        // The exemption applies mid-chain too: `.x` stays `null` untouched.
        query!(br#"{"x": null}"#, r"del(.x.a)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("x"), Some(&OwnedValue::Null));
            }
        );
        // And when `null` is the root of a 2+-element chain itself (rather
        // than reached by descending into an already-null field) — this
        // exercises the `Null` arm inside the `Expr::Pipe` chain-walk, a
        // separate code path from the single-step arm above.
        query!(br"null", r"del(.a.b)",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_del_index_through_null_is_a_no_op() {
        // #476: same exemption for a numeric index — `null | del(.[0])` is
        // `null`, matching jq's `null | .[0]` reading `null` back.
        query!(br"null", r"del(.[0])",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
        query!(br#"{"x": null}"#, r"del(.x[0])",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("x"), Some(&OwnedValue::Null));
            }
        );
        // Same chain-root case as the field test above, for the `Index`
        // arm's `Null` case inside the `Expr::Pipe` chain-walk.
        query!(br"null", r"del(.[0].a)",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_del_field_on_wrong_type_respects_optional() {
        // A genuinely wrong (non-null) type still respects `?`, both as the
        // sole path component and mid-chain: no-op rather than error.
        query!(br"5", r"del(.a?)",
            QueryResult::Owned(OwnedValue::Int(5) | OwnedValue::NumberLiteral(NumberRepr::Int(5), _)) => {}
        );
        query!(br"5", r"del(.a?.b)",
            QueryResult::Owned(OwnedValue::Int(5) | OwnedValue::NumberLiteral(NumberRepr::Int(5), _)) => {}
        );
        // Without `?`, the same wrong type still raises, mid-chain too.
        query!(br"5", r"del(.a.b)",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot index number with string \"a\"");
            }
        );
    }

    #[test]
    fn test_del_index_on_wrong_type_respects_optional() {
        // Same shape as the field test above, for a numeric index.
        query!(br"5", r"del(.[0]?)",
            QueryResult::Owned(OwnedValue::Int(5) | OwnedValue::NumberLiteral(NumberRepr::Int(5), _)) => {}
        );
        query!(br"5", r"del(.[0]?.a)",
            QueryResult::Owned(OwnedValue::Int(5) | OwnedValue::NumberLiteral(NumberRepr::Int(5), _)) => {}
        );
        query!(br"5", r"del(.[0])",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot index number with number");
            }
        );
        query!(br"5", r"del(.[0].a)",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot index number with number");
            }
        );
    }

    #[test]
    fn test_del_call_optional_prunes_output_rather_than_no_op() {
        // #537: `del(f)?` wraps the *whole call* in try/catch-empty, like jq —
        // a step error must prune the output entirely, not fall back to a
        // silent no-op that still emits the unchanged input. Contrast with
        // `test_del_field_on_wrong_type_respects_optional` above, where the
        // `?` sits *inside* the path (`del(.a?)`) and a no-op is correct.
        query!(br"5", r"del(.a)?", QueryResult::None => {});
        query!(br"5", r"del(.[0])?", QueryResult::None => {});
        query!(br#"{"a":5}"#, r"del(.a.b)?", QueryResult::None => {});
        query!(br#"{"a":{"x":1}}"#, r"del(.a.b[])?", QueryResult::None => {});

        // Without the outer `?`, the same queries still raise as before.
        query!(br"5", r"del(.a)",
            QueryResult::Error(e) => {
                assert_eq!(e.message, "Cannot index number with string \"a\"");
            }
        );
    }

    #[test]
    fn test_chained_assign() {
        // Chained: .a = 1 | .b = 2
        query!(br#"{"a": 0, "b": 0}"#, r".a = 1 | .b = 2",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let a = obj.get("a").unwrap();
                assert_eq!(*a, OwnedValue::Int(1));
                let b = obj.get("b").unwrap();
                assert_eq!(*b, OwnedValue::Int(2));
            }
        );
    }

    // ========================================================================
    // YAML Metadata Functions (yq)
    // ========================================================================

    #[test]
    fn test_tag_null() {
        query!(br"null", "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!null");
            }
        );
    }

    #[test]
    fn test_tag_bool() {
        query!(br"true", "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!bool");
            }
        );
        query!(br"false", "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!bool");
            }
        );
    }

    #[test]
    fn test_tag_int() {
        query!(br"42", "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!int");
            }
        );
    }

    #[test]
    fn test_tag_float() {
        query!(br"3.14", "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!float");
            }
        );
    }

    #[test]
    fn test_tag_string() {
        query!(br#""hello""#, "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!str");
            }
        );
    }

    #[test]
    fn test_tag_array() {
        query!(br"[1, 2, 3]", "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!seq");
            }
        );
    }

    #[test]
    fn test_tag_object() {
        query!(br#"{"a": 1}"#, "tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!map");
            }
        );
    }

    #[test]
    fn test_tag_nested() {
        // Test tag on nested values
        query!(br#"{"value": 42}"#, ".value | tag",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "!!int");
            }
        );
    }

    #[test]
    fn test_anchor_returns_empty() {
        // anchor always returns empty string (metadata not preserved)
        query!(br#"{"a": 1}"#, "anchor",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "");
            }
        );
    }

    #[test]
    fn test_style_returns_empty() {
        // style always returns empty string (metadata not preserved)
        query!(br#""hello""#, "style",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "");
            }
        );
        query!(br"[1, 2, 3]", "style",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "");
            }
        );
        query!(br#"{"a": 1}"#, "style",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "");
            }
        );
    }

    #[test]
    fn test_tag_with_map() {
        // Test tag with map function
        query!(br#"[1, "hello", true, null]"#, "map(tag)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::String("!!int".to_string()));
                assert_eq!(arr[1], OwnedValue::String("!!str".to_string()));
                assert_eq!(arr[2], OwnedValue::String("!!bool".to_string()));
                assert_eq!(arr[3], OwnedValue::String("!!null".to_string()));
            }
        );
    }

    // ============================================================================
    // kind function tests (yq YAML metadata)
    // ============================================================================

    #[test]
    fn test_kind_null() {
        query!(b"null", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "scalar");
            }
        );
    }

    #[test]
    fn test_kind_bool() {
        query!(b"true", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "scalar");
            }
        );
        query!(b"false", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "scalar");
            }
        );
    }

    #[test]
    fn test_kind_number() {
        query!(b"42", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "scalar");
            }
        );
        query!(b"3.14", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "scalar");
            }
        );
    }

    #[test]
    fn test_kind_string() {
        query!(br#""hello""#, "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "scalar");
            }
        );
    }

    #[test]
    fn test_kind_array() {
        query!(b"[1, 2, 3]", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "seq");
            }
        );
        query!(b"[]", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "seq");
            }
        );
    }

    #[test]
    fn test_kind_object() {
        query!(br#"{"a": 1}"#, "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "map");
            }
        );
        query!(b"{}", "kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "map");
            }
        );
    }

    #[test]
    fn test_kind_nested() {
        // Test kind on nested values
        query!(br#"{"items": [1, 2]}"#, ".items | kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "seq");
            }
        );
        query!(br#"[{"a": 1}]"#, ".[0] | kind",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "map");
            }
        );
    }

    #[test]
    fn test_kind_with_map() {
        // Test kind with map function
        query!(br#"[1, "hello", [1,2], {"a": 1}]"#, "map(kind)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::String("scalar".to_string()));
                assert_eq!(arr[1], OwnedValue::String("scalar".to_string()));
                assert_eq!(arr[2], OwnedValue::String("seq".to_string()));
                assert_eq!(arr[3], OwnedValue::String("map".to_string()));
            }
        );
    }

    // ============================================================================
    // key function tests (yq)
    // ============================================================================

    #[test]
    fn test_key_object() {
        // Test key on object iteration
        query!(br#"{"a": 1, "b": 2, "c": 3}"#, ".[] | key",
            QueryResult::ManyOwned(results) => {
                assert_eq!(results.len(), 3);
                // Check that all results are string keys
                let keys: Vec<String> = results.iter().filter_map(|v| {
                    if let OwnedValue::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                }).collect();
                assert!(keys.contains(&"a".to_string()));
                assert!(keys.contains(&"b".to_string()));
                assert!(keys.contains(&"c".to_string()));
            }
        );
    }

    #[test]
    fn test_key_array() {
        // Test key on array iteration - returns indices
        query!(b"[10, 20, 30]", ".[] | key",
            QueryResult::ManyOwned(results) => {
                assert_eq!(results.len(), 3);
                // Check that we get indices 0, 1, 2
                let indices: Vec<i64> = results.iter().filter_map(|v| {
                    if let OwnedValue::Int(i) = v {
                        Some(*i)
                    } else {
                        None
                    }
                }).collect();
                assert_eq!(indices, vec![0, 1, 2]);
            }
        );
    }

    #[test]
    fn test_key_at_root() {
        // Test key at root level - returns null
        query!(br#"{"a": 1}"#, "key",
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_key_nested() {
        // Test key on nested access
        query!(br#"{"outer": {"inner": 42}}"#, ".outer | .[] | key",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "inner");
            }
        );
    }

    // Phase 12 tests: Additional builtins

    #[test]
    fn test_now() {
        // now returns current Unix timestamp as a float
        query!(b"null", "now",
            QueryResult::Owned(OwnedValue::Float(n)) => {
                // Should be a reasonable Unix timestamp (after year 2020)
                assert!(n > 1577836800.0, "timestamp should be after 2020");
                // Should be before year 2100
                assert!(n < 4102444800.0, "timestamp should be before 2100");
            }
        );
    }

    #[test]
    fn test_abs() {
        // abs is an alias for fabs
        query!(b"-5", "abs", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 5.0).abs() < f64::EPSILON);
        });
        query!(b"5", "abs", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 5.0).abs() < f64::EPSILON);
        });
        query!(b"-7.25", "abs", QueryResult::Owned(OwnedValue::Float(n)) => {
            assert!((n - 7.25).abs() < f64::EPSILON);
        });
    }

    #[test]
    fn test_builtins() {
        // builtins returns an array of builtin function names
        query!(b"null", "builtins | length",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                // Should have many builtins (at least 100)
                assert!(n > 100, "should have many builtins, got {n}");
            }
        );
        // Check some known builtins exist
        query!(b"null", r#"builtins | map(select(startswith("now"))) | length"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert!(n >= 1, "should have now builtin");
            }
        );
    }

    #[test]
    fn test_normals() {
        // normals selects only normal numbers (not 0, inf, nan, subnormal)
        query!(b"5", "normals", QueryResult::One(_) => {});
        query!(b"-3.14", "normals", QueryResult::One(_) => {});
        query!(b"0", "normals", QueryResult::None => {}); // 0 is not normal
        query!(b"null", "normals", QueryResult::None => {}); // null is not a number
        query!(br#""string""#, "normals", QueryResult::None => {}); // string is not a number
    }

    #[test]
    fn test_finites() {
        // finites selects only finite numbers (not inf or nan)
        query!(b"5", "finites", QueryResult::One(_) => {});
        query!(b"-3.14", "finites", QueryResult::One(_) => {});
        query!(b"0", "finites", QueryResult::One(_) => {}); // 0 is finite
        query!(b"null", "finites", QueryResult::None => {}); // null is not a number
        query!(br#""string""#, "finites", QueryResult::None => {}); // string is not a number
    }

    #[test]
    fn test_format_urid() {
        // @urid decodes URI/percent encoding
        query!(br#""hello%20world""#, "@urid",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello world");
            }
        );
        query!(br#""foo%2Fbar""#, "@urid",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "foo/bar");
            }
        );
        // Test roundtrip with @uri
        query!(br#""hello world""#, "@uri | @urid",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello world");
            }
        );
        // Invalid percent encoding should pass through
        query!(br#""hello%GGworld""#, "@urid",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "hello%GGworld");
            }
        );
    }

    #[test]
    fn test_loc_basic() {
        // $__loc__ returns {"file": "<stdin>", "line": N}
        query!(b"null", "$__loc__",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("file"), Some(&OwnedValue::String("<stdin>".into())));
                assert_eq!(obj.get("line"), Some(&OwnedValue::Int(1)));
            }
        );
    }

    #[test]
    fn test_loc_line_number() {
        // $__loc__ should report line 1 for single-line filter
        query!(b"null", "$__loc__.line",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(1));
            }
        );
    }

    #[test]
    fn test_loc_file() {
        // $__loc__.file should be "<stdin>"
        query!(b"null", "$__loc__.file",
            QueryResult::Owned(OwnedValue::String(s)) => {
                assert_eq!(s, "<stdin>");
            }
        );
    }

    #[test]
    fn test_loc_multiline() {
        // Multi-line filter: $__loc__ on line 2 should report line 2
        let filter = ".\n| $__loc__.line";
        let json = b"null";
        let index = crate::json::JsonIndex::build(json);
        let cursor = index.root(json);
        let expr = crate::jq::parse(filter).unwrap();
        match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
            QueryResult::Owned(v) => {
                assert_eq!(
                    v,
                    OwnedValue::Int(2),
                    "expected line 2 for $__loc__ on second line"
                );
            }
            other => panic!("expected Owned, got {other:?}"),
        }
    }

    #[test]
    fn test_loc_multiline_line3() {
        // $__loc__ on line 3 should report line 3
        // Use valid jq syntax: pipe identity on separate lines
        let filter = ".\n|\n$__loc__.line";
        let json = b"null";
        let index = crate::json::JsonIndex::build(json);
        let cursor = index.root(json);
        let expr = crate::jq::parse(filter).unwrap();
        match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
            QueryResult::Owned(v) => {
                assert_eq!(
                    v,
                    OwnedValue::Int(3),
                    "expected line 3 for $__loc__ on third line"
                );
            }
            other => panic!("expected Owned, got {other:?}"),
        }
    }

    #[test]
    fn test_loc_in_function_def() {
        // $__loc__ inside a function should report the line where it appears
        let filter = "def f: $__loc__.line; f";
        let json = b"null";
        let index = crate::json::JsonIndex::build(json);
        let cursor = index.root(json);
        let expr = crate::jq::parse(filter).unwrap();
        match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
            QueryResult::Owned(v) => {
                assert_eq!(
                    v,
                    OwnedValue::Int(1),
                    "expected line 1 for $__loc__ in function on line 1"
                );
            }
            other => panic!("expected Owned, got {other:?}"),
        }
    }

    // ============================================================================
    // Phase 13: Iteration control tests
    // ============================================================================

    #[test]
    fn test_limit_stream() {
        // limit(2; .[]) - take first 2 elements
        // Use [limit(2; .[])] to collect into array
        query!(b"[1, 2, 3, 4, 5]", "[limit(2; .[])]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(2));
            }
        );
    }

    #[test]
    fn test_limit_zero() {
        // limit(0; .[]) - take no elements
        query!(b"[1, 2, 3]", "[limit(0; .[])]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 0);
            }
        );
    }

    #[test]
    fn test_limit_exceeds() {
        // limit(10; .[]) - limit exceeds array length
        query!(b"[1, 2]", "[limit(10; .[])]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[1], OwnedValue::Int(2));
            }
        );
    }

    #[test]
    fn test_first_stream() {
        // first(.[]) - get first element
        query!(b"[1, 2, 3]", "first(.[])",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    #[test]
    fn test_first_no_arg() {
        // first (no arg) - get first element of array
        query!(b"[1, 2, 3]", "first",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 1);
            }
        );
    }

    #[test]
    fn test_last_stream() {
        // last(.[]) - get last element
        query!(b"[1, 2, 3]", "last(.[])",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 3);
            }
        );
    }

    #[test]
    fn test_last_no_arg() {
        // last (no arg) - get last element of array
        // Note: last collects all elements to find the last, so returns Owned
        query!(b"[1, 2, 3]", "last",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(3));
            }
        );
    }

    #[test]
    fn test_nth_stream() {
        // nth(1; .[]) - get second element (0-indexed)
        query!(b"[10, 20, 30]", "nth(1; .[])",
            QueryResult::Owned(v) => {
                assert_eq!(v, OwnedValue::Int(20));
            }
        );
    }

    #[test]
    fn regression_issue_464_nth_stream_propagates_expr_error() {
        // builtin_nth_stream's final match on expr's result had no explicit
        // Error arm, so it fell through to the catch-all `_ => None` and
        // silently discarded the error instead of propagating it.
        query!(b"null", r#"nth(0; error("boom"))"#,
            QueryResult::Error(e) => {
                assert_eq!(e.message, "boom");
            }
        );
        // Also reachable via a comma-generator argument (the path that
        // originally surfaced this while verifying #155's fix).
        query!(b"null", r#"nth(0; 1, error("boom"))"#,
            QueryResult::Error(e) => {
                assert_eq!(e.message, "boom");
            }
        );
    }

    #[test]
    fn test_nth_stream_number_literal_n_387() {
        // A bare `.n` field access stays a lazy StandardJson::Number cursor
        // (QueryResult::One), which already worked fine -- it never goes
        // through to_owned()'s NumberLiteral conversion. getpath(["n"])
        // forces materialization through to_owned(), so `n` arrives as
        // QueryResult::Owned(OwnedValue::NumberLiteral(..)), which is the
        // arm that was actually broken. The n-arm match in
        // builtin_nth_stream must treat that the same as a plain Int/Float,
        // not fall through to a "expected number, got null" error.
        assert_eq!(
            outputs(
                br#"{"n":2,"arr":[10,20,30,40,50]}"#,
                r#"nth(getpath(["n"]); .arr[])"#
            ),
            ["30"]
        );
    }

    #[test]
    fn test_skip_number_literal_n_387() {
        // See test_nth_stream_number_literal_n_387 for why getpath (not a
        // bare `.n`) is required to reach the previously-broken arm.
        assert_eq!(
            outputs(
                br#"{"n":2,"arr":[10,20,30,40,50]}"#,
                r#"[skip(getpath(["n"]); .arr[])]"#
            ),
            ["[30,40,50]"]
        );
    }

    #[test]
    fn test_nth_no_arg() {
        // nth(1) (no second arg) - get second element of array
        query!(b"[10, 20, 30]", "nth(1)",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 20);
            }
        );
    }

    #[test]
    fn test_range_simple() {
        // range(3) - generate 0, 1, 2
        query!(b"null", "[range(3)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(0));
                assert_eq!(arr[1], OwnedValue::Int(1));
                assert_eq!(arr[2], OwnedValue::Int(2));
            }
        );
    }

    #[test]
    fn test_range_from_to() {
        // range(2; 5) - generate 2, 3, 4
        query!(b"null", "[range(2; 5)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(2));
                assert_eq!(arr[1], OwnedValue::Int(3));
                assert_eq!(arr[2], OwnedValue::Int(4));
            }
        );
    }

    #[test]
    fn test_range_from_to_by() {
        // range(0; 10; 3) - generate 0, 3, 6, 9
        query!(b"null", "[range(0; 10; 3)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::Int(0));
                assert_eq!(arr[1], OwnedValue::Int(3));
                assert_eq!(arr[2], OwnedValue::Int(6));
                assert_eq!(arr[3], OwnedValue::Int(9));
            }
        );
    }

    #[test]
    fn test_range_negative_step() {
        // range(5; 0; -2) - generate 5, 3, 1
        query!(b"null", "[range(5; 0; -2)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Int(5));
                assert_eq!(arr[1], OwnedValue::Int(3));
                assert_eq!(arr[2], OwnedValue::Int(1));
            }
        );
    }

    #[test]
    fn test_range_float_step() {
        // Issue #165: range(0; 1; 0.3) - jq accumulates doubles:
        // 0, 0.3, 0.6, 0.8999999999999999
        query!(b"null", "[range(0; 1; 0.3)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::Float(0.0));
                assert_eq!(arr[1], OwnedValue::Float(0.3));
                assert_eq!(arr[2], OwnedValue::Float(0.3 + 0.3));
                assert_eq!(arr[3], OwnedValue::Float(0.3 + 0.3 + 0.3));
            }
        );
    }

    #[test]
    fn test_range_float_from() {
        // Issue #165: range(2.5; 5) - jq: 2.5, 3.5, 4.5
        query!(b"null", "[range(2.5; 5)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Float(2.5));
                assert_eq!(arr[1], OwnedValue::Float(3.5));
                assert_eq!(arr[2], OwnedValue::Float(4.5));
            }
        );
    }

    #[test]
    fn test_range_float_single_arg() {
        // range(0.5) - jq: [0]
        query!(b"null", "[range(0.5)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0], OwnedValue::Float(0.0));
            }
        );
    }

    #[test]
    fn test_range_float_negative_step() {
        // range(5; 0; -1.5) - jq: 5, 3.5, 2, 0.5
        query!(b"null", "[range(5; 0; -1.5)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::Float(5.0));
                assert_eq!(arr[1], OwnedValue::Float(3.5));
                assert_eq!(arr[2], OwnedValue::Float(2.0));
                assert_eq!(arr[3], OwnedValue::Float(0.5));
            }
        );
    }

    #[test]
    fn test_range_document_float() {
        // Float bounds sourced from the document, not query literals
        query!(b"{\"x\": 2.5}", "[range(.x; 5)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Float(2.5));
                assert_eq!(arr[1], OwnedValue::Float(3.5));
                assert_eq!(arr[2], OwnedValue::Float(4.5));
            }
        );
    }

    #[test]
    fn test_range_zero_step_empty() {
        // jq 1.7.1 yields no values for a zero step (it does not error)
        query!(b"null", "[range(0; 1; 0)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty());
            }
        );
    }

    #[test]
    fn test_isempty_false() {
        // isempty(.[]) on non-empty array
        query!(b"[1, 2, 3]", "isempty(.[])",
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(!b);
            }
        );
    }

    #[test]
    fn test_isempty_true() {
        // isempty(.[]) on empty array
        query!(b"[]", "isempty(.[])",
            QueryResult::Owned(OwnedValue::Bool(b)) => {
                assert!(b);
            }
        );
    }

    #[test]
    fn test_first_with_select() {
        // first(.[] | select(. > 2)) - get first element > 2
        query!(b"[1, 2, 3, 4, 5]", "first(.[] | select(. > 2))",
            QueryResult::One(StandardJson::Number(n)) => {
                assert_eq!(n.as_i64().unwrap(), 3);
            }
        );
    }

    // =========================================================================
    // Phase 14 Tests: Recursive Traversal
    // =========================================================================

    #[test]
    fn test_recurse_down() {
        // recurse_down is an alias for recurse
        // Just verify it parses and returns the expected structure
        query!(br#"{"a": 1}"#, "[recurse_down]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // Should return the original object and the number 1
                assert_eq!(arr.len(), 2);
            }
        );
    }

    #[test]
    fn test_recurse_with_filter() {
        // recurse(.children[]?) - follow .children at each level
        query!(br#"{"name": "root", "children": [{"name": "a", "children": [{"name": "b"}]}, {"name": "c"}]}"#,
            "[recurse(.children[]?) | .name]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // Should collect: root, a, b, c
                assert_eq!(arr.len(), 4);
            }
        );
    }

    #[test]
    fn test_recurse_with_condition() {
        // recurse(f; cond) - recurse while condition is true
        // Stop when value >= 5
        query!(b"1", "[recurse(. + 1; . < 5)]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // Should produce: 1, 2, 3, 4
                assert_eq!(arr.len(), 4);
                assert_eq!(arr[0], OwnedValue::Int(1));
                assert_eq!(arr[3], OwnedValue::Int(4));
            }
        );
    }

    #[test]
    fn test_walk_strings() {
        // walk to uppercase all strings
        query!(br#"{"name": "alice", "nested": {"value": "bob"}}"#,
            "walk(if type == \"string\" then ascii_upcase else . end)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.get("name"), Some(&OwnedValue::String("ALICE".into())));
                if let Some(OwnedValue::Object(nested)) = obj.get("nested") {
                    assert_eq!(nested.get("value"), Some(&OwnedValue::String("BOB".into())));
                } else {
                    panic!("expected nested object");
                }
            }
        );
    }

    #[test]
    fn test_walk_arrays() {
        // walk to reverse all arrays
        query!(br#"{"items": [1, 2, 3], "nested": {"more": [4, 5]}}"#,
            "walk(if type == \"array\" then reverse else . end)",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                if let Some(OwnedValue::Array(items)) = obj.get("items") {
                    assert_eq!(items[0], OwnedValue::Int(3));
                    assert_eq!(items[1], OwnedValue::Int(2));
                    assert_eq!(items[2], OwnedValue::Int(1));
                } else {
                    panic!("expected items array");
                }
            }
        );
    }

    // Tests for indices/index/rindex

    #[test]
    fn test_indices_string() {
        // Find all occurrences of substring in string
        query!(br#""abcabc""#, r#"indices("bc")"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(1), OwnedValue::Int(4)]);
            }
        );
    }

    #[test]
    fn test_indices_array() {
        // Find all occurrences of element in array
        query!(br"[1, 2, 3, 1, 2]", "indices(1)",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(0), OwnedValue::Int(3)]);
            }
        );
    }

    #[test]
    fn test_indices_array_string() {
        // Find all occurrences of string element in array
        query!(br#"["a", "b", "a", "c"]"#, r#"indices("a")"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(0), OwnedValue::Int(2)]);
            }
        );
    }

    #[test]
    fn test_indices_not_found() {
        // No occurrences returns empty array
        query!(br#""abc""#, r#"indices("xyz")"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty());
            }
        );
    }

    #[test]
    fn test_index_string() {
        // First occurrence of substring
        query!(br#""abcabc""#, r#"index("bc")"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 1);
            }
        );
    }

    #[test]
    fn test_index_array() {
        // First occurrence of element in array
        query!(br"[1, 2, 3, 1, 2]", "index(2)",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 1);
            }
        );
    }

    #[test]
    fn test_index_not_found() {
        // Not found returns null
        query!(br#""abc""#, r#"index("xyz")"#,
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_rindex_string() {
        // Last occurrence of substring
        query!(br#""abcabc""#, r#"rindex("bc")"#,
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 4);
            }
        );
    }

    #[test]
    fn test_rindex_array() {
        // Last occurrence of element in array
        query!(br"[1, 2, 3, 1, 2]", "rindex(2)",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 4);
            }
        );
    }

    #[test]
    fn test_rindex_not_found() {
        // Not found returns null
        query!(br#""abc""#, r#"rindex("xyz")"#,
            QueryResult::Owned(OwnedValue::Null) => {}
        );
    }

    #[test]
    fn test_indices_array_object() {
        // Find objects in array
        query!(br#"[{"a":1}, {"b":2}, {"a":1}]"#, r#"indices({"a":1})"#,
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(0), OwnedValue::Int(2)]);
            }
        );
    }

    #[test]
    fn test_index_array_null() {
        // Find null in array
        query!(br"[1, null, 2, null]", "index(null)",
            QueryResult::Owned(OwnedValue::Int(n)) => {
                assert_eq!(n, 1);
            }
        );
    }

    #[test]
    fn test_omit_object() {
        // Remove keys from object
        query!(br#"{"a":1, "b":2, "c":3}"#, r#"omit(["a", "c"])"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 1);
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(2)));
                assert!(obj.get("a").is_none());
                assert!(obj.get("c").is_none());
            }
        );
    }

    #[test]
    fn test_omit_object_nonexistent_keys() {
        // Gracefully ignore non-existent keys
        query!(br#"{"a":1, "b":2}"#, r#"omit(["c", "d"])"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 2);
                assert_eq!(obj.get("a"), Some(&OwnedValue::Int(1)));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Int(2)));
            }
        );
    }

    #[test]
    fn test_omit_array() {
        // Remove indices from array
        query!(br#"["a", "b", "c", "d"]"#, "omit([0, 2])",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::String("b".to_string()));
                assert_eq!(arr[1], OwnedValue::String("d".to_string()));
            }
        );
    }

    #[test]
    fn test_omit_array_negative_index() {
        // Remove negative indices from array
        query!(br#"["a", "b", "c", "d"]"#, "omit([-1])",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::String("a".to_string()));
                assert_eq!(arr[1], OwnedValue::String("b".to_string()));
                assert_eq!(arr[2], OwnedValue::String("c".to_string()));
            }
        );
    }

    #[test]
    fn test_omit_array_out_of_bounds() {
        // Out of bounds indices are silently ignored
        query!(br#"["a", "b", "c"]"#, "omit([10, -10])",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                // No indices removed - all out of bounds
                assert_eq!(arr.len(), 3);
            }
        );
    }

    #[test]
    fn test_omit_empty_keys() {
        // Empty keys array returns full object
        query!(br#"{"a":1, "b":2}"#, "omit([])",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 2);
            }
        );
    }

    #[test]
    fn test_omit_all_keys() {
        // Omit all keys returns empty object
        query!(br#"{"a":1, "b":2}"#, r#"omit(["a", "b"])"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 0);
            }
        );
    }

    #[test]
    fn test_omit_preserves_order() {
        // Object key order is preserved (remaining keys)
        query!(br#"{"c":3, "a":1, "b":2, "d":4}"#, r#"omit(["a", "d"])"#,
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                let keys: Vec<&String> = obj.keys().collect();
                assert_eq!(keys, vec!["c", "b"]);
            }
        );
    }

    // ============================================================================
    // document_index / di tests
    // ============================================================================

    #[test]
    fn test_document_index_parses() {
        // Test that document_index parses correctly
        let expr = crate::jq::parse("document_index").unwrap();
        assert!(matches!(
            expr,
            crate::jq::Expr::Builtin(crate::jq::Builtin::DocumentIndex)
        ));
    }

    #[test]
    fn test_di_parses() {
        // Test that di (shorthand) parses correctly
        let expr = crate::jq::parse("di").unwrap();
        assert!(matches!(
            expr,
            crate::jq::Expr::Builtin(crate::jq::Builtin::DocumentIndex)
        ));
    }

    #[test]
    fn test_document_index_json_returns_zero() {
        // For JSON input, document_index returns 0 (single document assumed)
        query!(br#"{"name": "test"}"#, "document_index",
            QueryResult::Owned(OwnedValue::Int(0)) => {}
        );
    }

    #[test]
    fn test_di_json_returns_zero() {
        // For JSON input, di returns 0 (single document assumed)
        query!(br"[1, 2, 3]", "di",
            QueryResult::Owned(OwnedValue::Int(0)) => {}
        );
    }

    #[test]
    fn test_document_index_in_select() {
        // document_index can be used in select expressions
        let expr = crate::jq::parse("select(document_index == 0)").unwrap();
        // Verify it parses without error
        assert!(matches!(expr, crate::jq::Expr::Builtin(_)));
    }

    // ============================================================================
    // shuffle tests
    // ============================================================================

    #[test]
    fn test_shuffle_parses() {
        // Test that shuffle parses correctly
        let expr = crate::jq::parse("shuffle").unwrap();
        assert!(matches!(
            expr,
            crate::jq::Expr::Builtin(crate::jq::Builtin::Shuffle)
        ));
    }

    #[test]
    #[cfg(feature = "cli")]
    fn test_shuffle_returns_array_same_length() {
        // shuffle should return an array with the same length
        query!(br"[1, 2, 3, 4, 5]", "shuffle",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 5);
                // All original elements should be present (just reordered)
                assert!(arr.contains(&OwnedValue::Int(1)));
                assert!(arr.contains(&OwnedValue::Int(2)));
                assert!(arr.contains(&OwnedValue::Int(3)));
                assert!(arr.contains(&OwnedValue::Int(4)));
                assert!(arr.contains(&OwnedValue::Int(5)));
            }
        );
    }

    #[test]
    #[cfg(feature = "cli")]
    fn test_shuffle_preserves_element_types() {
        // shuffle should preserve element types (strings, numbers, objects)
        query!(br#"["a", 1, true, null]"#, "shuffle",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 4);
                assert!(arr.contains(&OwnedValue::String("a".to_string())));
                assert!(arr.contains(&OwnedValue::Int(1)));
                assert!(arr.contains(&OwnedValue::Bool(true)));
                assert!(arr.contains(&OwnedValue::Null));
            }
        );
    }

    #[test]
    #[cfg(feature = "cli")]
    fn test_shuffle_empty_array() {
        // shuffle of empty array should return empty array
        query!(br"[]", "shuffle",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty());
            }
        );
    }

    #[test]
    #[cfg(feature = "cli")]
    fn test_shuffle_single_element() {
        // shuffle of single element should return the same element
        query!(br"[42]", "shuffle",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(42)]);
            }
        );
    }

    #[test]
    fn test_shuffle_type_error_on_non_array() {
        // shuffle requires array input
        query!(br#""not an array""#, "shuffle",
            QueryResult::Error(err) => {
                let msg = format!("{err}");
                assert!(msg.contains("array") || msg.contains("cli"));
            }
        );
    }

    #[test]
    #[cfg(feature = "cli")]
    fn test_shuffle_in_pipeline() {
        // shuffle can be used in a pipeline
        query!(br"[3, 1, 2]", "shuffle | length",
            QueryResult::Owned(OwnedValue::Int(3)) => {}
        );
    }

    // ============================================================================
    // pivot tests
    // ============================================================================

    #[test]
    fn test_pivot_parses() {
        // Test that pivot parses correctly
        let expr = crate::jq::parse("pivot").unwrap();
        assert!(matches!(
            expr,
            crate::jq::Expr::Builtin(crate::jq::Builtin::Pivot)
        ));
    }

    #[test]
    fn test_pivot_array_of_arrays() {
        // Transpose array of arrays: [[a, b], [x, y]] → [[a, x], [b, y]]
        query!(br"[[1, 2], [3, 4]]", "pivot",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(3)]));
                assert_eq!(arr[1], OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Int(4)]));
            }
        );
    }

    #[test]
    fn test_pivot_array_of_arrays_3x3() {
        // 3x3 matrix transpose
        query!(br"[[1, 2, 3], [4, 5, 6], [7, 8, 9]]", "pivot",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0], OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(4), OwnedValue::Int(7)]));
                assert_eq!(arr[1], OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Int(5), OwnedValue::Int(8)]));
                assert_eq!(arr[2], OwnedValue::Array(vec![OwnedValue::Int(3), OwnedValue::Int(6), OwnedValue::Int(9)]));
            }
        );
    }

    #[test]
    fn test_pivot_array_of_arrays_ragged() {
        // Ragged arrays get null padding: [[1, 2], [3]] → [[1, 3], [2, null]]
        query!(br"[[1, 2], [3]]", "pivot",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(3)]));
                assert_eq!(arr[1], OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Null]));
            }
        );
    }

    #[test]
    fn test_pivot_array_of_objects() {
        // Transpose array of objects: [{a: 1}, {a: 2}] → {a: [1, 2]}
        query!(br#"[{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]"#, "pivot",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 2);
                assert_eq!(obj.get("name"), Some(&OwnedValue::Array(vec![
                    OwnedValue::String("Alice".to_string()),
                    OwnedValue::String("Bob".to_string())
                ])));
                assert_eq!(obj.get("age"), Some(&OwnedValue::Array(vec![
                    OwnedValue::Int(30),
                    OwnedValue::Int(25)
                ])));
            }
        );
    }

    #[test]
    fn test_pivot_array_of_objects_missing_keys() {
        // Missing keys get null: [{a: 1}, {a: 2, b: 3}] → {a: [1, 2], b: [null, 3]}
        query!(br#"[{"a": 1}, {"a": 2, "b": 3}]"#, "pivot",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert_eq!(obj.len(), 2);
                assert_eq!(obj.get("a"), Some(&OwnedValue::Array(vec![
                    OwnedValue::Int(1),
                    OwnedValue::Int(2)
                ])));
                assert_eq!(obj.get("b"), Some(&OwnedValue::Array(vec![
                    OwnedValue::Null,
                    OwnedValue::Int(3)
                ])));
            }
        );
    }

    #[test]
    fn test_pivot_empty_array() {
        // Empty array returns empty array
        query!(br"[]", "pivot",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty());
            }
        );
    }

    #[test]
    fn test_pivot_array_of_empty_arrays() {
        // Array of empty arrays returns empty array
        query!(br"[[], []]", "pivot",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert!(arr.is_empty());
            }
        );
    }

    #[test]
    fn test_pivot_array_of_empty_objects() {
        // Array of empty objects returns empty object
        query!(br"[{}, {}]", "pivot",
            QueryResult::Owned(OwnedValue::Object(obj)) => {
                assert!(obj.is_empty());
            }
        );
    }

    #[test]
    fn test_pivot_type_error_on_non_array() {
        // pivot requires array input
        query!(br#""not an array""#, "pivot",
            QueryResult::Error(err) => {
                let msg = format!("{err}");
                assert!(msg.contains("array"));
            }
        );
    }

    #[test]
    fn test_pivot_error_on_mixed_types() {
        // pivot requires all arrays or all objects, not mixed
        query!(br#"[[1], {"a": 2}]"#, "pivot",
            QueryResult::Error(err) => {
                let msg = format!("{err}");
                assert!(msg.contains("array of arrays") || msg.contains("array of objects"));
            }
        );
    }

    #[test]
    fn test_pivot_error_on_scalars() {
        // pivot requires arrays or objects, not scalar values
        query!(br"[1, 2, 3]", "pivot",
            QueryResult::Error(err) => {
                let msg = format!("{err}");
                assert!(msg.contains("array of arrays") || msg.contains("array of objects"));
            }
        );
    }

    #[test]
    fn test_pivot_in_pipeline() {
        // pivot can be used in a pipeline
        query!(br"[[1, 2], [3, 4]]", "pivot | .[0]",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr, vec![OwnedValue::Int(1), OwnedValue::Int(3)]);
            }
        );
    }

    #[test]
    fn test_pivot_double_pivot_identity() {
        // Double pivot should return original for square matrices
        query!(br"[[1, 2], [3, 4]]", "pivot | pivot",
            QueryResult::Owned(OwnedValue::Array(arr)) => {
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[0], OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]));
                assert_eq!(arr[1], OwnedValue::Array(vec![OwnedValue::Int(3), OwnedValue::Int(4)]));
            }
        );
    }

    // ========== Phase 22: load(file) tests ==========

    #[cfg(feature = "std")]
    mod load_tests {
        use super::*;
        use std::fs;

        fn with_temp_file<F, R>(name: &str, content: &str, f: F) -> R
        where
            F: FnOnce(&str) -> R,
        {
            let path = format!("/tmp/succinctly_test_{name}");
            fs::write(&path, content).unwrap();
            let result = f(&path);
            let _ = fs::remove_file(&path);
            result
        }

        #[test]
        fn test_load_json_file() {
            with_temp_file(
                "load_test.json",
                r#"{"name": "test", "value": 42}"#,
                |path| {
                    let json_bytes: &[u8] = b"null";
                    let index = JsonIndex::build(json_bytes);
                    let cursor = index.root(json_bytes);
                    let query = format!(r#"load("{path}")"#);
                    let expr = parse(&query).unwrap();
                    match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                        QueryResult::Owned(OwnedValue::Object(obj)) => {
                            assert_eq!(
                                obj.get("name"),
                                Some(&OwnedValue::String("test".to_string()))
                            );
                            assert_eq!(obj.get("value"), Some(&OwnedValue::Int(42)));
                        }
                        other => panic!("unexpected result: {other:?}"),
                    }
                },
            );
        }

        #[test]
        fn test_load_yaml_file() {
            with_temp_file("load_test.yaml", "name: test\nvalue: 42\n", |path| {
                let json_bytes: &[u8] = b"null";
                let index = JsonIndex::build(json_bytes);
                let cursor = index.root(json_bytes);
                let query = format!(r#"load("{path}")"#);
                let expr = parse(&query).unwrap();
                match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                    QueryResult::Owned(OwnedValue::Object(obj)) => {
                        assert_eq!(
                            obj.get("name"),
                            Some(&OwnedValue::String("test".to_string()))
                        );
                        assert_eq!(obj.get("value"), Some(&OwnedValue::Int(42)));
                    }
                    other => panic!("unexpected result: {other:?}"),
                }
            });
        }

        #[test]
        fn test_load_yml_extension() {
            with_temp_file("load_test.yml", "items:\n  - a\n  - b\n", |path| {
                let json_bytes: &[u8] = b"null";
                let index = JsonIndex::build(json_bytes);
                let cursor = index.root(json_bytes);
                let query = format!(r#"load("{path}")"#);
                let expr = parse(&query).unwrap();
                match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                    QueryResult::Owned(OwnedValue::Object(obj)) => {
                        let items = obj.get("items").unwrap();
                        match items {
                            OwnedValue::Array(arr) => {
                                assert_eq!(arr.len(), 2);
                                assert_eq!(arr[0], OwnedValue::String("a".to_string()));
                                assert_eq!(arr[1], OwnedValue::String("b".to_string()));
                            }
                            _ => panic!("expected array"),
                        }
                    }
                    other => panic!("unexpected result: {other:?}"),
                }
            });
        }

        #[test]
        fn test_load_nonexistent_file() {
            let json_bytes: &[u8] = b"null";
            let index = JsonIndex::build(json_bytes);
            let cursor = index.root(json_bytes);
            let expr = parse(r#"load("/tmp/nonexistent_file_12345.yaml")"#).unwrap();
            match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                QueryResult::Error(err) => {
                    assert!(
                        err.message.contains("Failed to read file")
                            || err.message.contains("No such file")
                    );
                }
                other => panic!("expected error, got: {other:?}"),
            }
        }

        #[test]
        fn test_load_optional_nonexistent() {
            // Use try-catch for optional behavior since ? operator applies to field access
            let json_bytes: &[u8] = b"null";
            let index = JsonIndex::build(json_bytes);
            let cursor = index.root(json_bytes);
            let expr = parse(r#"try load("/tmp/nonexistent_file_12345.yaml") catch null"#).unwrap();
            match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                QueryResult::Owned(OwnedValue::Null) => {}
                other => panic!("expected null, got: {other:?}"),
            }
        }

        #[test]
        fn test_load_with_try_catch() {
            let json_bytes: &[u8] = b"null";
            let index = JsonIndex::build(json_bytes);
            let cursor = index.root(json_bytes);
            let expr =
                parse(r#"try load("/tmp/nonexistent_file_12345.yaml") catch "not found""#).unwrap();
            match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                QueryResult::Owned(OwnedValue::String(s)) => {
                    assert_eq!(s, "not found");
                }
                other => panic!("expected 'not found', got: {other:?}"),
            }
        }

        #[test]
        fn test_load_combined_with_input() {
            with_temp_file("config.yaml", "setting: enabled\n", |path| {
                let json_bytes: &[u8] = br#"{"name": "main"}"#;
                let index = JsonIndex::build(json_bytes);
                let cursor = index.root(json_bytes);
                let query = format!(r#". + {{config: load("{path}")}}"#);
                let expr = parse(&query).unwrap();
                match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                    QueryResult::Owned(OwnedValue::Object(obj)) => {
                        assert_eq!(
                            obj.get("name"),
                            Some(&OwnedValue::String("main".to_string()))
                        );
                        let config = obj.get("config").unwrap();
                        match config {
                            OwnedValue::Object(cfg) => {
                                assert_eq!(
                                    cfg.get("setting"),
                                    Some(&OwnedValue::String("enabled".to_string()))
                                );
                            }
                            _ => panic!("expected config to be object"),
                        }
                    }
                    other => panic!("unexpected result: {other:?}"),
                }
            });
        }

        #[test]
        fn test_load_multi_document_yaml() {
            with_temp_file("multi.yaml", "---\nname: doc1\n---\nname: doc2\n", |path| {
                let json_bytes: &[u8] = b"null";
                let index = JsonIndex::build(json_bytes);
                let cursor = index.root(json_bytes);
                let query = format!(r#"load("{path}")"#);
                let expr = parse(&query).unwrap();
                match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                    QueryResult::Owned(OwnedValue::Array(arr)) => {
                        assert_eq!(arr.len(), 2);
                        match &arr[0] {
                            OwnedValue::Object(obj) => {
                                assert_eq!(
                                    obj.get("name"),
                                    Some(&OwnedValue::String("doc1".to_string()))
                                );
                            }
                            _ => panic!("expected first doc to be object"),
                        }
                        match &arr[1] {
                            OwnedValue::Object(obj) => {
                                assert_eq!(
                                    obj.get("name"),
                                    Some(&OwnedValue::String("doc2".to_string()))
                                );
                            }
                            _ => panic!("expected second doc to be object"),
                        }
                    }
                    other => panic!("expected array of documents, got: {other:?}"),
                }
            });
        }

        #[test]
        fn test_load_with_dynamic_path() {
            with_temp_file("dynamic.json", r#"{"loaded": true}"#, |path| {
                // Input contains the path to load
                let json_bytes = format!(r#"{{"path": "{path}"}}"#);
                let json_bytes = json_bytes.as_bytes();
                let index = JsonIndex::build(json_bytes);
                let cursor = index.root(json_bytes);
                let expr = parse(r"load(.path)").unwrap();
                match eval::<Vec<u64>, JqSemantics>(&expr, cursor) {
                    QueryResult::Owned(OwnedValue::Object(obj)) => {
                        assert_eq!(obj.get("loaded"), Some(&OwnedValue::Bool(true)));
                    }
                    other => panic!("unexpected result: {other:?}"),
                }
            });
        }
    }

    // ========== #360: the unresolved-computed-key guards ==========

    /// Every path walker refuses an unresolved [`Expr::IndexExpr`], loudly.
    ///
    /// [`resolve_dynamic_indexes`] rewrites each computed key into the static
    /// component it denotes before any of these run, so none of these arms can
    /// fire through the public API — which is exactly why they need a test of
    /// their own. They exist so that a *new* path context wired up without
    /// that pre-pass fails where the mistake is, instead of blaming the user's
    /// filter through the `_` catch-alls ("invalid path component", "cannot use
    /// expression as …") or, in [`walk_path`], silently emitting no path at
    /// all. Nothing else pins that promise: the wording *is* the signal that an
    /// install point has gone missing.
    mod computed_key_guards {
        use super::*;

        /// `.[$k]` left unresolved — what each walker below is handed.
        fn unresolved() -> Expr {
            Expr::index_by(Expr::Identity, Expr::Var("k".to_string()))
        }

        #[test]
        fn test_set_path_refuses_an_unresolved_key() {
            let mut root = OwnedValue::Null;
            let err = set_path(&mut root, &unresolved(), OwnedValue::Int(1)).unwrap_err();
            assert_eq!(
                err.message,
                "internal error: unresolved computed index in assignment path"
            );
        }

        #[test]
        fn test_get_path_mut_refuses_an_unresolved_key() {
            let mut root = OwnedValue::Null;
            let err = get_path_mut(&mut root, &[unresolved()]).unwrap_err();
            assert_eq!(
                err.message,
                "internal error: unresolved computed index in path component"
            );
        }

        #[test]
        fn test_update_path_refuses_an_unresolved_key() {
            let mut root = OwnedValue::Null;
            let err = update_path::<JqSemantics>(&mut root, &unresolved(), &Expr::Identity, false)
                .unwrap_err();
            assert_eq!(
                err.message,
                "internal error: unresolved computed index in update path"
            );
        }

        #[test]
        fn test_delete_at_path_refuses_an_unresolved_key() {
            let mut root = OwnedValue::Null;
            let err = delete_at_path(&mut root, &unresolved(), false).unwrap_err();
            assert_eq!(
                err.message,
                "internal error: unresolved computed index in delete path"
            );
        }

        // [`walk_path`] does have an error channel, but a computed key that
        // reaches it is a wiring bug rather than anything the user's filter
        // did, so its guard stays a `debug_assert!` — which only fires in a
        // build that has them enabled.

        #[test]
        #[cfg(debug_assertions)]
        #[should_panic(expected = "unresolved computed index reached path tracking")]
        fn test_path_tracking_refuses_an_unresolved_key() {
            let mut reached = Vec::new();
            let _ = walk_path::<JqSemantics>(
                &unresolved(),
                &OwnedValue::Null,
                &[],
                &mut reached,
                false,
            );
        }

        /// The same guard reached mid-pipe rather than as the last step —
        /// the position that used to have a walker, and a wording, of its own.
        #[test]
        #[cfg(debug_assertions)]
        #[should_panic(expected = "unresolved computed index reached path tracking")]
        fn test_path_tracking_refuses_an_unresolved_key_mid_pipe() {
            let mut reached = Vec::new();
            let pipe = Expr::Pipe(vec![unresolved(), Expr::Field("a".to_string())]);
            let _ = walk_path::<JqSemantics>(&pipe, &OwnedValue::Null, &[], &mut reached, false);
        }
    }
}
