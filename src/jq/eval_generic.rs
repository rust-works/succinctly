//! Generic expression evaluator for jq-like queries.
//!
//! This module provides a document-agnostic evaluator that works with any type
//! implementing the `DocumentValue` trait, enabling direct evaluation of both
//! JSON and YAML without intermediate conversion.

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

use super::document::{DocumentCursor, DocumentElements, DocumentFields, DocumentValue};
use super::eval::{
    compare_values, eval as full_eval, index_one_owned as index_owned_by_key,
    numeric_display_string, numeric_key_to_index, tonumber_from_str, Control, EvalError,
    EvalSemantics, JqSemantics, QueryResult,
};
use super::expr::{Builtin, CompareOp, Expr, Literal};
use super::value::{is_nan_sentinel, OwnedValue};
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
    } else if let Some(literal) = value.number_literal() {
        OwnedValue::from_number_literal(&literal)
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
            if is_nan_sentinel(n.raw_bytes()) {
                OwnedValue::Float(f64::NAN)
            } else {
                match core::str::from_utf8(n.raw_bytes()) {
                    Ok(s) => OwnedValue::from_number_literal(s),
                    Err(_) => OwnedValue::Null,
                }
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
fn eval_on_owned<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    owned: OwnedValue,
    optional: bool,
) -> GenericResult<V> {
    let json_str = owned.to_json_for_reindex();
    let json_bytes = json_str.as_bytes();
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    let wrapped;
    let expr = if optional {
        wrapped = Expr::Optional(Box::new(expr.clone()));
        &wrapped
    } else {
        expr
    };

    match full_eval::<Vec<u64>, S>(expr, cursor) {
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
        QueryResult::Partial(vs, control) => GenericResult::Partial(vs, control),
    }
}

/// Normalize a prefix and its terminator into a `GenericResult` (#400, #494).
/// Mirrors [`super::eval::partial`] (the `QueryResult` equivalent) — an empty
/// prefix collapses to the bare `Error`/`Break` variant.
fn partial_generic<V: DocumentValue>(
    prefix: Vec<OwnedValue>,
    control: Control,
) -> GenericResult<V> {
    if prefix.is_empty() {
        match control {
            Control::Error(e) => GenericResult::Error(e),
            Control::Break(label) => GenericResult::Break(label),
        }
    } else {
        GenericResult::Partial(prefix, control)
    }
}

/// Append one truthiness bit per output of a `GenericResult` stream to
/// `out`. Mirrors [`super::eval::push_truthiness`] for the generic
/// evaluator's cursor-aware result type — used to fan `select`'s condition
/// out over every output instead of only its first (#378).
fn push_generic_truthiness<V: DocumentValue>(
    result: GenericResult<V>,
    out: &mut Vec<bool>,
) -> Option<Control> {
    match result {
        GenericResult::One(v) => out.push(to_owned(&v).is_truthy()),
        GenericResult::OneCursor(c) => out.push(to_owned(&c.value()).is_truthy()),
        GenericResult::Many(vs) => out.extend(vs.iter().map(|v| to_owned(v).is_truthy())),
        GenericResult::ManyCursor(cs) => {
            out.extend(cs.iter().map(|c| to_owned(&c.value()).is_truthy()));
        }
        GenericResult::None => {}
        GenericResult::Owned(v) => out.push(v.is_truthy()),
        GenericResult::ManyOwned(vs) => out.extend(vs.iter().map(OwnedValue::is_truthy)),
        GenericResult::Error(e) => return Some(Control::Error(e)),
        GenericResult::Break(label) => return Some(Control::Break(label)),
        GenericResult::Partial(vs, control) => {
            out.extend(vs.iter().map(OwnedValue::is_truthy));
            return Some(control);
        }
    }
    None
}

/// Flatten a batch of per-element results into one `Vec<OwnedValue>`.
///
/// `items` must never contain `Error`/`Break`/`Partial` — callers that build
/// up a per-element batch route those variants to an early return (folding
/// whatever was already flattened into a `Partial` of their own via
/// [`partial_generic`]) instead of pushing them here, so this only ever sees
/// the variants that still need materializing.
fn flatten_generic_results<V: DocumentValue>(items: Vec<GenericResult<V>>) -> Vec<OwnedValue> {
    let mut results = Vec::new();
    for r in items {
        match r {
            GenericResult::One(v) => results.push(to_owned(&v)),
            GenericResult::OneCursor(c) => results.push(to_owned(&c.value())),
            GenericResult::Many(rs) => results.extend(rs.iter().map(to_owned)),
            GenericResult::ManyCursor(cs) => {
                results.extend(cs.iter().map(|c| to_owned(&c.value())));
            }
            GenericResult::None => {}
            GenericResult::Owned(o) => results.push(o),
            GenericResult::ManyOwned(os) => results.extend(os),
            GenericResult::Error(_) | GenericResult::Break(_) | GenericResult::Partial(..) => {
                unreachable!("Error/Break/Partial already routed to an early return above")
            }
        }
    }
    results
}

/// Evaluate an expression on multiple OwnedValues using the full evaluator.
fn eval_on_many_owned<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    owned_values: Vec<OwnedValue>,
    optional: bool,
) -> GenericResult<V> {
    let mut results = Vec::new();
    for owned in owned_values {
        match eval_on_owned::<S, V>(expr, owned, optional) {
            GenericResult::One(_) => unreachable!("eval_on_owned never returns One"),
            GenericResult::OneCursor(_) => unreachable!("eval_on_owned never returns OneCursor"),
            GenericResult::Many(_) => unreachable!("eval_on_owned never returns Many"),
            GenericResult::ManyCursor(_) => {
                unreachable!("eval_on_owned never returns ManyCursor")
            }
            GenericResult::None => {}
            // The outputs already produced no longer vanish (#400, #494).
            GenericResult::Error(e) => return partial_generic(results, Control::Error(e)),
            GenericResult::Owned(o) => results.push(o),
            GenericResult::ManyOwned(os) => results.extend(os),
            GenericResult::Break(label) => return partial_generic(results, Control::Break(label)),
            GenericResult::Partial(vs, control) => {
                results.extend(vs);
                return partial_generic(results, control);
            }
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

    /// Multiple cursor results (from iteration with position preserved,
    /// e.g. `.[]`), enabling `line`/`column` on each element.
    ManyCursor(Vec<V::Cursor>),

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

    /// One or more outputs were produced before the stream terminated in an
    /// error or a `break` (#400, #494). Mirrors [`QueryResult::Partial`].
    Partial(Vec<OwnedValue>, Control),
}

impl<V: DocumentValue> GenericResult<V> {
    /// Convert to OwnedValue for output.
    pub fn into_owned(self) -> Option<OwnedValue> {
        match self {
            Self::One(v) => Some(to_owned(&v)),
            Self::OneCursor(c) => Some(to_owned(&c.value())),
            Self::Many(vs) => Some(OwnedValue::Array(vs.iter().map(to_owned).collect())),
            Self::ManyCursor(cs) => Some(OwnedValue::Array(
                cs.iter().map(|c| to_owned(&c.value())).collect(),
            )),
            Self::None => None,
            Self::Error(_) => None,
            Self::Owned(o) => Some(o),
            Self::ManyOwned(os) => Some(OwnedValue::Array(os)),
            Self::Break(_) => None,
            // A `Partial` prefix is not representable as a single value —
            // same "not representable" answer as `Break`/`Error` here.
            Self::Partial(..) => None,
        }
    }

    /// Collect all results into a Vec of OwnedValues.
    ///
    /// A `Partial` collects its prefix — the whole point of #400/#494 is
    /// that those outputs are no longer discarded.
    pub fn collect_owned(self) -> Vec<OwnedValue> {
        match self {
            Self::One(v) => vec![to_owned(&v)],
            Self::OneCursor(c) => vec![to_owned(&c.value())],
            Self::Many(vs) => vs.iter().map(to_owned).collect(),
            Self::ManyCursor(cs) => cs.iter().map(|c| to_owned(&c.value())).collect(),
            Self::None => vec![],
            Self::Error(_) => vec![],
            Self::Owned(o) => vec![o],
            Self::ManyOwned(os) => os,
            Self::Break(_) => vec![],
            Self::Partial(vs, _control) => vs,
        }
    }

    /// Check if this is an error.
    ///
    /// A `Partial` prefix followed by an error still counts.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_) | Self::Partial(_, Control::Error(_)))
    }

    /// Get the error if this is an error result.
    pub fn error(&self) -> Option<&EvalError> {
        match self {
            Self::Error(e) => Some(e),
            _ => None,
        }
    }

    /// Stream results as JSON to the output writer.
    ///
    /// This is the M2 streaming fast path that avoids OwnedValue materialization
    /// for cursor-based results. For owned results, uses the StreamableValue impl.
    /// - `indent_spaces`: Spaces per indentation level (0 for compact)
    ///
    /// Returns the number of values streamed and whether the last was falsy.
    pub fn stream_json<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent_spaces: usize,
        mut on_value: impl FnMut(&mut W) -> core::fmt::Result,
    ) -> Result<crate::jq::stream::StreamStats, core::fmt::Error> {
        use crate::jq::stream::{StreamError, StreamStats, StreamableValue};

        let mut stats = StreamStats::default();

        match self {
            Self::One(v) => {
                // Convert to owned for streaming
                let owned = to_owned(v);
                owned.stream_json(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait
                c.stream_json(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::Many(vs) => {
                for v in vs {
                    let owned = to_owned(v);
                    owned.stream_json(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = vs.len();
            }
            Self::ManyCursor(cs) => {
                for c in cs {
                    c.stream_json(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = c.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = cs.len();
            }
            Self::None => {
                // No output
            }
            Self::Error(e) => {
                // Nothing goes to `out`: `out` is stdout, and a diagnostic
                // written there is indistinguishable from a result. Hand it
                // back so the caller can print to stderr and fail (#355).
                stats.error = Some(stream_error(e));
            }
            Self::Owned(o) => {
                o.stream_json(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = o.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::ManyOwned(os) => {
                for o in os {
                    o.stream_json(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
            }
            Self::Break(label) => {
                stats.error = Some(StreamError {
                    message: format!("break ${label} not in label"),
                    not_a_string: false,
                });
            }
            // The prefix streams like `ManyOwned` above, then the control is
            // reported the same way `Error`/`Break` are (#400, #494) — the
            // outputs already produced no longer vanish behind the failure.
            Self::Partial(os, control) => {
                for o in os {
                    o.stream_json(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
                stats.error = Some(match control {
                    Control::Error(e) => stream_error(e),
                    Control::Break(label) => StreamError {
                        message: format!("break ${label} not in label"),
                        not_a_string: false,
                    },
                });
            }
        }

        Ok(stats)
    }

    /// Check if this result produces a single streamable cursor result.
    ///
    /// This is used to detect if M2 streaming can be applied for navigation queries.
    pub fn is_single_cursor(&self) -> bool {
        matches!(self, Self::OneCursor(_))
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
        use crate::jq::stream::{StreamError, StreamStats, StreamableValue};

        let mut stats = StreamStats::default();

        match self {
            Self::One(v) => {
                let owned = to_owned(v);
                owned.stream_yaml(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait
                c.stream_yaml(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::Many(vs) => {
                for v in vs {
                    let owned = to_owned(v);
                    owned.stream_yaml(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = vs.len();
            }
            Self::ManyCursor(cs) => {
                for c in cs {
                    c.stream_yaml(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = c.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = cs.len();
            }
            Self::None => {
                // No output
            }
            Self::Error(e) => {
                // See `stream_json`: diagnostics never go to `out` (#355).
                stats.error = Some(stream_error(e));
            }
            Self::Owned(o) => {
                o.stream_yaml(out, indent_spaces)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = o.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::ManyOwned(os) => {
                for o in os {
                    o.stream_yaml(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
            }
            Self::Break(label) => {
                stats.error = Some(StreamError {
                    message: format!("break ${label} not in label"),
                    not_a_string: false,
                });
            }
            // Same treatment as `stream_json` (#400, #494): the prefix
            // streams first, then the control is reported.
            Self::Partial(os, control) => {
                for o in os {
                    o.stream_yaml(out, indent_spaces)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
                stats.error = Some(match control {
                    Control::Error(e) => stream_error(e),
                    Control::Break(label) => StreamError {
                        message: format!("break ${label} not in label"),
                        not_a_string: false,
                    },
                });
            }
        }

        Ok(stats)
    }
}

/// Render an [`EvalError`] into the payload a streaming caller needs to
/// reproduce jq's diagnostic on stderr (#355).
fn stream_error(e: &EvalError) -> crate::jq::stream::StreamError {
    crate::jq::stream::StreamError {
        message: e.message.clone(),
        not_a_string: e.payload_is_not_a_string(),
    }
}

/// Evaluate an expression against a document value.
///
/// This is the main entry point for generic evaluation. It uses jq semantics;
/// use [`eval_using`] to select yq semantics.
pub fn eval<V: DocumentValue>(expr: &Expr, value: V) -> GenericResult<V> {
    eval_using::<JqSemantics, V>(expr, value)
}

/// Evaluate an expression against a document value with explicit semantics.
///
/// Arithmetic that falls back to the full evaluator (division, modulo, overflow)
/// follows `S`, so yq keeps yq numeric behavior instead of jq's.
pub fn eval_using<S: EvalSemantics, V: DocumentValue>(expr: &Expr, value: V) -> GenericResult<V> {
    eval_single::<S, V>(expr, value, false, None)
}

/// Evaluate an expression against a cursor.
///
/// This entry point preserves cursor position metadata, enabling
/// `line` and `column` builtins to return actual values. It uses jq semantics;
/// use [`eval_with_cursor_using`] to select yq semantics.
pub fn eval_with_cursor<C: DocumentCursor>(expr: &Expr, cursor: C) -> GenericResult<C::Value> {
    eval_with_cursor_using::<JqSemantics, C>(expr, cursor)
}

/// Evaluate an expression against a cursor with explicit semantics.
///
/// Like [`eval_with_cursor`] but arithmetic follows `S` (jq vs yq), so yq's
/// modulo/division/overflow behavior is preserved on the cursor path.
pub fn eval_with_cursor_using<S: EvalSemantics, C: DocumentCursor>(
    expr: &Expr,
    cursor: C,
) -> GenericResult<C::Value> {
    eval_single::<S, C::Value>(expr, cursor.value(), false, Some(cursor))
}

/// Evaluate a single expression against a value with optional cursor context.
fn eval_single<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    match expr {
        // Forward the cursor when we have one, so a bare `line`/`column`
        // downstream of a no-op navigation step (`. | line`) still resolves
        // a real position instead of falling to the `One`->`None` default.
        Expr::Identity => cursor.map_or(GenericResult::One(value), GenericResult::OneCursor),

        Expr::Field(name) => {
            if let Some(fields) = value.as_object() {
                match fields.find_cursor(name) {
                    Some(c) => GenericResult::OneCursor(c),
                    // jq returns null for missing fields on objects (not an error)
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if value.is_null() {
                // jq returns null for field access on null
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_field(value.type_name(), name))
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
                match elements.get_cursor(actual_idx) {
                    Some(c) => GenericResult::OneCursor(c),
                    // jq returns null for out-of-bounds array indices (positive
                    // or negative), not an error. `.[n]` and `.[n]?` both yield
                    // null since there is no error for `?` to suppress. See #307.
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if value.is_null() {
                // jq returns null for index on null, as the `Expr::Field` arm
                // above already does for `.foo`. Without this, `null | .[0]`
                // errored while `null | .[$n]` — the same query, and the same
                // rule in `index_one_generic` — returned null.
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    value.type_name(),
                    "number",
                ))
            }
        }

        // Handled natively rather than through the `_` fallback below, for two
        // reasons: the fallback re-enters `full_eval`, which restarts with
        // `optional = false` and so loses the `?` in `.[$k]?`; and it
        // serialises and re-indexes the whole document per evaluation.
        Expr::IndexExpr { target, key } => {
            eval_index_expr::<S, V>(target, key, value, optional, cursor)
        }

        Expr::Iterate => {
            if let Some(elements) = value.as_array() {
                let cursors = elements.collect_cursors();
                if cursors.is_empty() {
                    GenericResult::None
                } else {
                    GenericResult::ManyCursor(cursors)
                }
            } else if let Some(fields) = value.as_object() {
                let mut cursors = Vec::new();
                let mut f = fields;
                while let Some((field, rest)) = f.uncons() {
                    cursors.push(field.value_cursor);
                    f = rest;
                }
                if cursors.is_empty() {
                    GenericResult::None
                } else {
                    GenericResult::ManyCursor(cursors)
                }
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_iterate(&to_owned(&value)))
            }
        }

        Expr::Optional(inner) => eval_single::<S, _>(inner, value, true, cursor),

        Expr::Pipe(exprs) => {
            if exprs.is_empty() {
                return GenericResult::One(value);
            }

            let mut current = eval_single::<S, _>(&exprs[0], value, optional, cursor);

            for (i, expr) in exprs.iter().enumerate().skip(1) {
                current = match current {
                    // The previous stage produced `vs` before terminating in
                    // `outer_control` (#400, #494): pipe that prefix through
                    // this stage first — `eval_on_many_owned` already
                    // propagates any control the piping itself hits — and
                    // only attach `outer_control` once that's done cleanly.
                    GenericResult::Partial(vs, outer_control) => {
                        match eval_on_many_owned::<S, _>(expr, vs, optional) {
                            p @ (GenericResult::Partial(..)
                            | GenericResult::Error(_)
                            | GenericResult::Break(_)) => p,
                            GenericResult::None => partial_generic(Vec::new(), outer_control),
                            GenericResult::ManyOwned(results) => {
                                partial_generic(results, outer_control)
                            }
                            _ => unreachable!(
                                "eval_on_many_owned only returns None/ManyOwned/Error/Break/Partial"
                            ),
                        }
                    }
                    GenericResult::One(v) => eval_single::<S, _>(expr, v, optional, None),
                    GenericResult::OneCursor(c) => {
                        eval_single::<S, _>(expr, c.value(), optional, Some(c))
                    }
                    GenericResult::Many(vs) => {
                        let mut results = Vec::new();
                        for v in vs {
                            match eval_single::<S, _>(expr, v, optional, None) {
                                GenericResult::One(r) => results.push(to_owned(&r)),
                                GenericResult::OneCursor(c) => results.push(to_owned(&c.value())),
                                GenericResult::Many(rs) => {
                                    results.extend(rs.iter().map(to_owned));
                                }
                                GenericResult::ManyCursor(cs) => {
                                    results.extend(cs.iter().map(|c| to_owned(&c.value())));
                                }
                                GenericResult::None => {}
                                // The outputs already piped through no longer
                                // vanish (#400, #494).
                                GenericResult::Error(e) => {
                                    return partial_generic(results, Control::Error(e));
                                }
                                GenericResult::Owned(o) => results.push(o),
                                GenericResult::ManyOwned(os) => results.extend(os),
                                GenericResult::Break(label) => {
                                    return partial_generic(results, Control::Break(label));
                                }
                                GenericResult::Partial(vs2, control) => {
                                    results.extend(vs2);
                                    return partial_generic(results, control);
                                }
                            }
                        }
                        if results.is_empty() {
                            GenericResult::None
                        } else {
                            GenericResult::ManyOwned(results)
                        }
                    }
                    // Unlike the `Many` arm above, don't flatten after just
                    // this one stage: a flattened `ManyOwned` can't carry a
                    // per-element cursor into any *further* stage — including
                    // one in an *enclosing* pipe, since a dot-chain like
                    // `.a[].b` parses as its own nested `Pipe` (see
                    // `parse_postfix`), so `.a[].b | line` only reaches
                    // `line` after this whole inner pipe returns. Instead,
                    // run the rest of the pipe (`expr` and everything after
                    // it) against each cursor independently and, when every
                    // element's result is itself a single cursor (the common
                    // `.[] | .foo` / nested dot-chain shape), stay as
                    // `ManyCursor` so an enclosing pipe or `line`/`column`
                    // can still resolve a position. Only degrade to
                    // materialized `ManyOwned` when a result is
                    // heterogeneous (multiple values, filtered out, or a
                    // computed value).
                    GenericResult::ManyCursor(cs) => {
                        let rest = Expr::Pipe(exprs[i..].to_vec());
                        let mut per_element = Vec::with_capacity(cs.len());
                        for c in cs {
                            match eval_single::<S, _>(&rest, c.value(), optional, Some(c)) {
                                // The elements already piped through no
                                // longer vanish (#400, #494).
                                GenericResult::Error(e) => {
                                    return partial_generic(
                                        flatten_generic_results(per_element),
                                        Control::Error(e),
                                    );
                                }
                                GenericResult::Break(label) => {
                                    return partial_generic(
                                        flatten_generic_results(per_element),
                                        Control::Break(label),
                                    );
                                }
                                GenericResult::Partial(vs, control) => {
                                    let mut prefix = flatten_generic_results(per_element);
                                    prefix.extend(vs);
                                    return partial_generic(prefix, control);
                                }
                                other => per_element.push(other),
                            }
                        }

                        let all_single_cursor = !per_element.is_empty()
                            && per_element
                                .iter()
                                .all(|r| matches!(r, GenericResult::OneCursor(_)));

                        return if all_single_cursor {
                            GenericResult::ManyCursor(
                                per_element
                                    .into_iter()
                                    .map(|r| match r {
                                        GenericResult::OneCursor(c) => c,
                                        _ => unreachable!("checked all_single_cursor above"),
                                    })
                                    .collect(),
                            )
                        } else {
                            let results = flatten_generic_results(per_element);
                            if results.is_empty() {
                                GenericResult::None
                            } else {
                                GenericResult::ManyOwned(results)
                            }
                        };
                    }
                    GenericResult::None => GenericResult::None,
                    GenericResult::Error(e) => return GenericResult::Error(e),
                    GenericResult::Owned(o) => {
                        // Continue piping from owned value via JSON round-trip
                        eval_on_owned::<S, _>(expr, o, optional)
                    }
                    GenericResult::ManyOwned(os) => {
                        // Continue piping from owned values via JSON round-trip
                        eval_on_many_owned::<S, _>(expr, os, optional)
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

        Expr::Builtin(builtin) => eval_builtin::<S, _>(builtin, value, optional, cursor),

        // Comparison operations - handle locally to preserve cursor context
        Expr::Compare { op, left, right } => {
            // Evaluate left and right with cursor context preserved
            let left_result = eval_single::<S, _>(left, value.clone(), false, cursor);
            let right_result = eval_single::<S, _>(right, value, false, cursor);

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
                GenericResult::ManyCursor(cs) => {
                    if let Some(first) = cs.first() {
                        to_owned(&first.value())
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
                // Same "take the first output" policy as `Many`/`ManyOwned`
                // above; the trailing control is dropped, consistent with
                // how comparison already doesn't fork over a multi-output
                // operand.
                GenericResult::Partial(vs, _control) => vs.into_iter().next().unwrap(),
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
                GenericResult::ManyCursor(cs) => {
                    if let Some(first) = cs.first() {
                        to_owned(&first.value())
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
                // Same "take the first output" policy as `Many`/`ManyOwned`
                // above; the trailing control is dropped.
                GenericResult::Partial(vs, _control) => vs.into_iter().next().unwrap(),
            };

            // Perform the comparison
            let result = match op {
                CompareOp::Eq => left_owned == right_owned,
                CompareOp::Ne => left_owned != right_owned,
                CompareOp::Lt => {
                    compare_values(&left_owned, &right_owned) == core::cmp::Ordering::Less
                }
                CompareOp::Le => matches!(
                    compare_values(&left_owned, &right_owned),
                    core::cmp::Ordering::Less | core::cmp::Ordering::Equal
                ),
                CompareOp::Gt => {
                    compare_values(&left_owned, &right_owned) == core::cmp::Ordering::Greater
                }
                CompareOp::Ge => matches!(
                    compare_values(&left_owned, &right_owned),
                    core::cmp::Ordering::Greater | core::cmp::Ordering::Equal
                ),
            };

            GenericResult::Owned(OwnedValue::Bool(result))
        }

        // Fall back to the full evaluator for complex expressions
        _ => {
            // Convert to OwnedValue, then to JSON, then evaluate with full evaluator
            let owned = to_owned(&value);
            let json_str = owned.to_json_for_reindex();
            let json_bytes = json_str.as_bytes();
            let index = JsonIndex::build(json_bytes);
            let cursor = index.root(json_bytes);

            // The full evaluator always starts a fresh `eval()` with
            // `optional = false`, so an ambient `optional = true` here (e.g.
            // `(.a + .b)?`, `first(.[])?`) would otherwise be silently
            // dropped at this bridge instead of suppressing the error, as it
            // does for the natively-handled arms above. Re-wrap in
            // `Expr::Optional` so the full evaluator's own (nuanced) handling
            // of `?` sees it, same as the `eval_on_owned` builtin-fallback
            // bridge below (#367, #386).
            let wrapped;
            let expr = if optional {
                wrapped = Expr::Optional(Box::new(expr.clone()));
                &wrapped
            } else {
                expr
            };

            // Evaluate using the full evaluator
            match full_eval::<Vec<u64>, S>(expr, cursor) {
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
                QueryResult::Partial(vs, control) => GenericResult::Partial(vs, control),
            }
        }
    }
}

/// Apply one resolved key to one target value.
///
/// Mirrors `eval::index_one`: the key *kind* is checked before the container,
/// so the error is jq's `Cannot index <container> with <key>`, and the
/// null-input passthrough is reached only for a valid key kind (`null | .["a"]`
/// is `null`, `null | .[null]` errors).
fn index_one_generic<V: DocumentValue>(
    target: V,
    key: &OwnedValue,
    optional: bool,
) -> GenericResult<V> {
    match key {
        OwnedValue::String(s) => {
            if let Some(fields) = target.as_object() {
                match fields.find(s) {
                    Some(v) => GenericResult::One(v),
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if target.is_null() {
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index(target.type_name(), key))
            }
        }
        OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
            // Truncation is toward zero, matching jq: `.[-1.5]` is the last
            // element. Out-of-range floats saturate through `as`, yielding null,
            // and NaN has no index at all — also null.
            let idx = numeric_key_to_index(key);
            if let Some(elements) = target.as_array() {
                let resolved = idx.map(|idx| {
                    if idx < 0 {
                        elements.len() as i64 + idx
                    } else {
                        idx
                    }
                });
                match resolved
                    .and_then(|r| usize::try_from(r).ok())
                    .and_then(|i| elements.get(i))
                {
                    Some(v) => GenericResult::One(v),
                    // Out-of-bounds is null, not an error (#307).
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if target.is_null() {
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index(target.type_name(), key))
            }
        }
        _ if optional => GenericResult::None,
        _ => GenericResult::Error(EvalError::cannot_index(target.type_name(), key)),
    }
}

/// Evaluate `E[K]` — indexing by a computed key.
///
/// The counterpart of `eval::eval_index_expr`; see that function for why the
/// key stream is evaluated first and iterated outermost, and why a trailing `?`
/// reaches neither the key nor the target.
fn eval_index_expr<S: EvalSemantics, V: DocumentValue>(
    target: &Expr,
    key: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    // Keys first: an empty key stream must not evaluate the target at all.
    let keys = match eval_single::<S, V>(key, value.clone(), false, cursor) {
        GenericResult::Error(e) => return GenericResult::Error(e),
        GenericResult::Break(label) => return GenericResult::Break(label),
        GenericResult::None => return GenericResult::None,
        other => other.collect_owned(),
    };
    if keys.is_empty() {
        return GenericResult::None;
    }

    let targets = match eval_single::<S, V>(target, value, false, cursor) {
        GenericResult::Error(e) => return GenericResult::Error(e),
        GenericResult::Break(label) => return GenericResult::Break(label),
        GenericResult::None => return GenericResult::None,
        // Computed indexing's key/target forking isn't part of #400/#494's
        // verified semantics — conservatively matching the Error/Break arms
        // above rather than inventing new partial-target behavior, same as
        // `eval::eval_index_expr`.
        GenericResult::Partial(_, Control::Error(e)) => return GenericResult::Error(e),
        GenericResult::Partial(_, Control::Break(label)) => return GenericResult::Break(label),
        GenericResult::One(v) => vec![v],
        GenericResult::Many(vs) => vs,
        GenericResult::OneCursor(c) => vec![c.value()],
        GenericResult::ManyCursor(cs) => cs.iter().map(DocumentCursor::value).collect(),
        // An owned target (a computed, non-navigational left side) has no
        // borrowed representation here; round-trip it through the shared
        // owned-value path by re-entering with the materialized document.
        owned @ (GenericResult::Owned(_) | GenericResult::ManyOwned(_)) => {
            let targets = owned.collect_owned();
            let mut out = Vec::with_capacity(keys.len() * targets.len());
            for k in &keys {
                for t in &targets {
                    match index_owned_by_key(t, k, optional) {
                        Ok(Some(v)) => out.push(v),
                        Ok(None) => {}
                        Err(e) => return GenericResult::Error(e),
                    }
                }
            }
            return match out.len() {
                1 => GenericResult::Owned(out.pop().expect("len checked")),
                _ => GenericResult::ManyOwned(out),
            };
        }
    };

    // Key outer, target inner.
    let mut borrowed: Vec<V> = Vec::new();
    let mut owned: Vec<OwnedValue> = Vec::new();
    let mut any_owned = false;
    for k in &keys {
        for t in &targets {
            match index_one_generic::<V>(t.clone(), k, optional) {
                GenericResult::One(v) => {
                    if any_owned {
                        owned.push(to_owned(&v));
                    } else {
                        borrowed.push(v);
                    }
                }
                GenericResult::Owned(v) => {
                    if !any_owned {
                        any_owned = true;
                        owned = borrowed.iter().map(to_owned).collect();
                        borrowed.clear();
                    }
                    owned.push(v);
                }
                GenericResult::None => {}
                GenericResult::Error(e) => return GenericResult::Error(e),
                _ => unreachable!("index_one_generic yields One/Owned/None/Error"),
            }
        }
    }

    if any_owned {
        match owned.len() {
            1 => GenericResult::Owned(owned.pop().expect("len checked")),
            _ => GenericResult::ManyOwned(owned),
        }
    } else {
        match borrowed.len() {
            1 => GenericResult::One(borrowed.pop().expect("len checked")),
            _ => GenericResult::Many(borrowed),
        }
    }
}

/// Evaluate a builtin function.
fn eval_builtin<S: EvalSemantics, V: DocumentValue>(
    builtin: &Builtin,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    match builtin {
        Builtin::Line => {
            let line = cursor.map_or(0, |c| c.line());
            GenericResult::Owned(OwnedValue::Int(line as i64))
        }

        Builtin::Column => {
            let column = cursor.map_or(0, |c| c.column());
            GenericResult::Owned(OwnedValue::Int(column as i64))
        }

        Builtin::DocumentIndex => {
            let doc_index = cursor.and_then(|c| c.document_index()).unwrap_or(0);
            GenericResult::Owned(OwnedValue::Int(doc_index as i64))
        }

        Builtin::Select(cond) => {
            // Evaluate condition with cursor context preserved.
            // This is critical for select(di == N) to work correctly.
            let cond_result = eval_single::<S, _>(cond, value.clone(), false, cursor);

            let mut bits = Vec::new();
            let cond_control = push_generic_truthiness(cond_result, &mut bits);
            let truthy_count = bits.iter().filter(|&&b| b).count();

            // `select` never changes position — every truthy output IS the
            // input node, so forward the incoming cursor (if any) rather
            // than dropping it via a plain `One`/`Many`. One republish per
            // truthy output of a possibly multi-output condition (#378).
            let pass_n = |n: usize| -> GenericResult<V> {
                match (n, cursor) {
                    (0, _) => GenericResult::None,
                    (1, Some(c)) => GenericResult::OneCursor(c),
                    (1, None) => GenericResult::One(value.clone()),
                    (n, Some(c)) => {
                        GenericResult::ManyCursor(core::iter::repeat(c).take(n).collect())
                    }
                    (n, None) => {
                        GenericResult::Many(core::iter::repeat(value.clone()).take(n).collect())
                    }
                }
            };

            match cond_control {
                None => pass_n(truthy_count),
                // `select(...)? ` swallows the condition's error the same
                // way `try select(...) catch empty` would — per #400/#494,
                // that keeps whatever the truthy bits already produced
                // rather than discarding it too. `break` always propagates,
                // `optional` or not.
                Some(Control::Error(_)) if optional => pass_n(truthy_count),
                Some(control) => {
                    let prefix: Vec<OwnedValue> = match cursor {
                        Some(c) => core::iter::repeat_with(|| to_owned(&c.value()))
                            .take(truthy_count)
                            .collect(),
                        None => core::iter::repeat_with(|| to_owned(&value))
                            .take(truthy_count)
                            .collect(),
                    };
                    partial_generic(prefix, control)
                }
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
                    let mut rng = ChaCha8Rng::from_os_rng();
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
                // checked_abs: i64::MIN has no i64 absolute value; use f64
                GenericResult::Owned(match i.checked_abs() {
                    Some(a) => OwnedValue::Int(a),
                    None => OwnedValue::Float(-(i as f64)),
                })
            } else if let Some(f) = value.as_f64() {
                GenericResult::Owned(OwnedValue::Float(f.abs()))
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::has_no_length(&to_owned(&value)))
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
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::has_no_keys(&to_owned(&value)))
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
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::has_no_keys(&to_owned(&value)))
            }
        }

        Builtin::Values => {
            // jq: values == select(. != null)
            if value.is_null() {
                GenericResult::None
            } else {
                cursor.map_or(GenericResult::One(value), GenericResult::OneCursor)
            }
        }

        // Handled natively rather than through the `_` fallback below: the
        // fallback materializes the whole value via `to_owned()` first,
        // which merges duplicate YAML mapping keys into one `IndexMap`
        // entry before this builtin ever runs (#443). Building one entry
        // object per field directly off the field cursor -- like `Keys`/
        // `Iterate` above -- means no user key is ever put into a shared
        // map, so duplicates can't collapse.
        Builtin::ToEntries => {
            if let Some(elements) = value.as_array() {
                let entries: Vec<OwnedValue> = elements
                    .collect_values()
                    .into_iter()
                    .enumerate()
                    .map(|(i, elem)| {
                        let mut entry = IndexMap::new();
                        entry.insert("key".to_string(), OwnedValue::Int(i as i64));
                        entry.insert("value".to_string(), to_owned(&elem));
                        OwnedValue::Object(entry)
                    })
                    .collect();
                GenericResult::Owned(OwnedValue::Array(entries))
            } else if let Some(fields) = value.as_object() {
                let mut entries: Vec<OwnedValue> = Vec::new();
                let mut f = fields;
                while let Some((field, rest)) = f.uncons() {
                    if let Some(key) = field.key_str() {
                        let mut entry = IndexMap::new();
                        entry.insert("key".to_string(), OwnedValue::String(key.into_owned()));
                        entry.insert("value".to_string(), to_owned(&field.value));
                        entries.push(OwnedValue::Object(entry));
                    }
                    f = rest;
                }
                GenericResult::Owned(OwnedValue::Array(entries))
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::has_no_keys(&to_owned(&value)))
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
                cursor.map_or(GenericResult::One(value), GenericResult::OneCursor)
            } else {
                GenericResult::None
            }
        }

        Builtin::Scalars => {
            // Returns input if scalar, empty otherwise
            if !value.is_iterable() {
                cursor.map_or(GenericResult::One(value), GenericResult::OneCursor)
            } else {
                GenericResult::None
            }
        }

        Builtin::First => {
            // jq: first == .[0], so [] and null both yield null
            if let Some(elements) = value.as_array() {
                match elements.get(0) {
                    Some(v) => GenericResult::One(v),
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if value.is_null() {
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    value.type_name(),
                    "number",
                ))
            }
        }

        Builtin::Last => {
            // jq: last == .[-1], so [] and null both yield null
            if let Some(elements) = value.as_array() {
                let len = elements.len();
                match len.checked_sub(1).and_then(|i| elements.get(i)) {
                    Some(v) => GenericResult::One(v),
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if value.is_null() {
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    value.type_name(),
                    "number",
                ))
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
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    value.type_name(),
                    "number",
                ))
            }
        }

        Builtin::Empty => GenericResult::None,

        Builtin::ToString => {
            let owned = to_owned(&value);
            let s = match &owned {
                OwnedValue::Null => "null".to_string(),
                OwnedValue::Bool(b) => b.to_string(),
                OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..) => {
                    numeric_display_string(&owned)
                }
                OwnedValue::String(s) => s.clone(),
                OwnedValue::Array(_) | OwnedValue::Object(_) => owned.to_json(),
            };
            GenericResult::Owned(OwnedValue::String(s))
        }

        Builtin::ToNumber => {
            // Already a number: a passthrough, not a computation, so (like
            // `.`) it keeps the source literal.
            if let Some(literal) = value.number_literal() {
                GenericResult::Owned(OwnedValue::from_number_literal(&literal))
            } else if let Some(i) = value.as_i64() {
                GenericResult::Owned(OwnedValue::Int(i))
            } else if let Some(f) = value.as_f64() {
                GenericResult::Owned(OwnedValue::Float(f))
            } else if let Some(s) = value.as_str() {
                match tonumber_from_str(s.as_ref()) {
                    Ok(n) => GenericResult::Owned(n),
                    Err(_) if optional => GenericResult::None,
                    Err(e) => GenericResult::Error(e),
                }
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_parse_as_number(&to_owned(&value)))
            }
        }

        // Phase 23: Position-based navigation (succinctly extension)
        Builtin::AtOffset(offset_expr) => {
            // Evaluate the offset expression
            let offset_result = eval_single::<S, _>(offset_expr, value.clone(), false, cursor);
            let offset = match offset_result {
                GenericResult::Owned(v) => match v.as_i64() {
                    Some(i) if i >= 0 => i as usize,
                    _ => {
                        return GenericResult::Error(EvalError::new(
                            "at_offset requires a non-negative integer".to_string(),
                        ))
                    }
                },
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
                        GenericResult::Error(EvalError::new(format!("no node at offset {offset}")))
                    }
                }
            }
        }

        Builtin::AtPosition(line_expr, col_expr) => {
            // Evaluate the line expression
            let line_result = eval_single::<S, _>(line_expr, value.clone(), false, cursor);
            let line = match line_result {
                GenericResult::Owned(v) => match v.as_i64() {
                    Some(i) if i > 0 => i as usize,
                    _ => {
                        return GenericResult::Error(EvalError::new(
                            "at_position requires positive integers for line".to_string(),
                        ))
                    }
                },
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
            let col_result = eval_single::<S, _>(col_expr, value.clone(), false, cursor);
            let col = match col_result {
                GenericResult::Owned(v) => match v.as_i64() {
                    Some(i) if i > 0 => i as usize,
                    _ => {
                        return GenericResult::Error(EvalError::new(
                            "at_position requires positive integers for column".to_string(),
                        ))
                    }
                },
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
                            "no node at position line {line} column {col}"
                        )))
                    }
                }
            }
        }

        // For other builtins, fall back to full evaluator via JSON
        _ => {
            let owned = to_owned(&value);
            eval_on_owned::<S, _>(&Expr::Builtin(builtin.clone()), owned, optional)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::expr::FormatType;
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
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Index(1), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::Int(2));
    }

    #[test]
    fn test_generic_iterate() {
        let json = br"[1, 2, 3]";
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
    fn test_generic_tostring_overflow_literal_renders_as_inf() {
        // Mirrors eval.rs's test_number_literal_overflow_renders_as_inf_not_garbage
        // (#561): the generic evaluator's ToString arm had the same bug.
        let json = br"1e400";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::ToString), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("inf".to_string()));
    }

    #[test]
    fn test_generic_format_overflow_literal_via_reindex_bridge() {
        // `Expr::Format` (unlike `Expr::Builtin(ToString)` above) has no
        // native arm in the generic evaluator, so it falls through the
        // catch-all bridge that reserializes the value to JSON text and
        // re-parses it before handing off to the full evaluator. That
        // reserialization used to call `OwnedValue::to_json()`, which
        // substitutes "null" for ±Infinity (correct for real JSON output,
        // but not for this internal round-trip) -- silently destroying the
        // overflowed literal before `eval.rs`'s (already-fixed) `@uri`
        // formatting ever saw it (#561). This exercises that bridge
        // directly, independent of the CLI.
        let json = br"1e400";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Format(FormatType::Uri), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("inf".to_string()));
    }

    #[test]
    fn test_generic_format_overflow_literal_negative_via_reindex_bridge() {
        // Mirrors the test above but with a negative overflow, exercising
        // `overflow_literal`'s negative-sign branch (`-1e999`) in
        // `OwnedValue::to_json_for_reindex`, which the positive-only case
        // above never reaches (#561).
        let json = br"-1e400";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Format(FormatType::Uri), value);
        let owned = result.into_owned().unwrap();

        assert_eq!(owned, OwnedValue::String("-inf".to_string()));
    }

    #[test]
    fn test_generic_length() {
        let json = br"[1, 2, 3, 4, 5]";
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
            _ => panic!("Expected object, got {owned:?}"),
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
    fn test_yaml_generic_to_entries_duplicate_keys() {
        // Duplicate YAML mapping keys must survive `to_entries` unmerged,
        // matching real `yq` -- not collapse to the last occurrence via the
        // `to_owned()` fallback's `IndexMap` (#443).
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\na: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        let result = eval(&Expr::Builtin(Builtin::ToEntries), value);
        let owned = result.into_owned().unwrap();

        let expected_entry = |v: i64| {
            let mut entry = IndexMap::new();
            entry.insert("key".to_string(), OwnedValue::String("a".to_string()));
            entry.insert("value".to_string(), OwnedValue::Int(v));
            OwnedValue::Object(entry)
        };
        assert_eq!(
            owned,
            OwnedValue::Array(vec![expected_entry(1), expected_entry(2)])
        );
    }

    #[test]
    fn test_yaml_generic_to_entries_array() {
        // `to_entries` on a YAML sequence takes the array branch (mirroring
        // the mapping branch above), producing {key: <index>, value: <elem>}
        // entries with integer keys -- matching real `yq`/`jq`.
        use crate::yaml::YamlIndex;

        let yaml = b"- a\n- b\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        let seq_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = seq_cursor.value();

        let result = eval(&Expr::Builtin(Builtin::ToEntries), value);
        let owned = result.into_owned().unwrap();

        let expected_entry = |i: i64, v: &str| {
            let mut entry = IndexMap::new();
            entry.insert("key".to_string(), OwnedValue::Int(i));
            entry.insert("value".to_string(), OwnedValue::String(v.to_string()));
            OwnedValue::Object(entry)
        };
        assert_eq!(
            owned,
            OwnedValue::Array(vec![expected_entry(0, "a"), expected_entry(1, "b")])
        );
    }

    #[test]
    fn test_yaml_generic_to_entries_optional_on_scalar() {
        // `try to_entries` (built as Expr::Optional here) on a scalar is
        // neither an array nor an object, so it must yield no result instead
        // of propagating `has_no_keys`.
        use crate::yaml::YamlIndex;

        let yaml = b"42";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        let scalar_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = scalar_cursor.value();

        let result = eval(
            &Expr::Optional(Box::new(Expr::Builtin(Builtin::ToEntries))),
            value,
        );

        assert!(matches!(result, GenericResult::None));
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

    // Regression tests for #532: `line`/`column` returned 0 for anything
    // downstream of `.foo`/`.[]`/`select(...)`, because those Expr/Builtin
    // arms received a cursor but never forwarded it — only a bare `line`
    // with zero preceding navigation (like the tests above) worked. These
    // go through a real `.foo`/`.[]`/`select(...)`/dot-chain pipeline
    // instead of calling `Builtin::Line` directly on a root cursor.

    #[test]
    fn test_yaml_field_then_line() {
        use crate::yaml::YamlIndex;

        // `foo` is the second key, so a correct result (2) can't be
        // confused with the `line() == 0`/`line() == 1` defaults.
        let yaml = b"other: 1\nfoo: bar\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".foo | line").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(2));
    }

    #[test]
    fn test_yaml_iterate_array_then_line() {
        use crate::yaml::YamlIndex;

        let yaml = b"- a\n- b\n- c\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".[] | line").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3),
            ])
        );
    }

    #[test]
    fn test_yaml_iterate_object_then_line() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\nb: 2\nc: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        // The issue's exact repro.
        let expr = crate::jq::parse(".[] | line").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3),
            ])
        );
    }

    #[test]
    fn test_yaml_select_then_line_preserves_position() {
        use crate::yaml::YamlIndex;

        let yaml = b"- 1\n- 2\n- 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".[] | select(. > 1) | line").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap(),
            OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Int(3)])
        );
    }

    #[test]
    fn test_yaml_dot_chain_then_line() {
        use crate::yaml::YamlIndex;

        // `.containers[].image | line` parses as a *nested* `Pipe` (the
        // dot-chain `.containers[].image` is its own Pipe, itself one
        // element of the outer `| line` pipe) — a stricter test than a
        // fully `|`-separated query, since the cursor must survive
        // `ManyCursor` propagating out of an inner pipe's return.
        let yaml = b"containers:\n  - image: a\n  - image: b\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".containers[].image | line").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap(),
            OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Int(3)])
        );
    }

    #[test]
    fn test_yaml_identity_pipe_then_line() {
        use crate::yaml::YamlIndex;

        let yaml = b"foo: bar\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(". | line").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(1));
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
                    // `select`'s truthy branch now forwards the cursor it
                    // was given (needed for `line`/`column` to survive a
                    // `select(...)`), so a match here is `OneCursor`, not
                    // `One` — see the `Builtin::Select` cursor-forwarding
                    // fix in `eval_builtin`.
                    GenericResult::OneCursor(c) => results.push(to_owned(&c.value())),
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
                    // See the sibling `test_yaml_select_di_eq_n` comment:
                    // `select`'s truthy branch now returns `OneCursor` when
                    // given one, not `One`.
                    GenericResult::One(_)
                    | GenericResult::OneCursor(_)
                    | GenericResult::Owned(_) => count += 1,
                    _ => {}
                }
                docs = rest;
            }
        }

        // Should match 2 documents (index 1 and 2)
        assert_eq!(count, 2);
    }

    #[test]
    fn test_json_select_many_cursor_condition_forks() {
        // A condition that iterates (`.[]`) evaluates through the
        // `ManyCursor` arm of `push_generic_truthiness`. `select` forwards
        // the *outer* cursor once per truthy element, not the elements
        // themselves (#378). Pinned against jq 1.7.1.
        let json = b"[true,false,true]";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("select(.[])").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned(),
            vec![
                OwnedValue::Array(vec![
                    OwnedValue::Bool(true),
                    OwnedValue::Bool(false),
                    OwnedValue::Bool(true)
                ]);
                2
            ]
        );
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
        assert_eq!(owned, OwnedValue::Int(30));
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
    fn test_at_offset_and_at_position_accept_a_document_sourced_argument() {
        // #387 wraps a materialized document number in `OwnedValue::NumberLiteral`
        // instead of plain `Int`, so `getpath` (which returns `Owned`, not a lazy
        // cursor) now hands `AtOffset`/`AtPosition` a `NumberLiteral` -- unhandled
        // here previously, it fell to the `_` arm and errored "requires a
        // non-negative integer" even though the value was a perfectly good 0.
        let json = br#"{"n": 2, "l": 1, "c": 1}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let via_getpath = crate::jq::parse(r#"at_offset(getpath(["n"]))"#).unwrap();
        let via_literal = crate::jq::parse("at_offset(2)").unwrap();
        assert_eq!(
            eval_with_cursor(&via_getpath, cursor).into_owned().unwrap(),
            eval_with_cursor(&via_literal, cursor).into_owned().unwrap(),
        );

        let via_getpath =
            crate::jq::parse(r#"at_position(getpath(["l"]); getpath(["c"]))"#).unwrap();
        let via_literal = crate::jq::parse("at_position(1; 1)").unwrap();
        assert_eq!(
            eval_with_cursor(&via_getpath, cursor).into_owned().unwrap(),
            eval_with_cursor(&via_literal, cursor).into_owned().unwrap(),
        );
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

    #[test]
    fn test_generic_result_conversions_all_variants() {
        // A real document-backed result fixes the generic `V` for the whole
        // Vec, letting the owned / none / error / break variants sit alongside.
        let doc = br#"{"a": 1, "b": [1, 2, 3]}"#;
        let index = JsonIndex::build(doc);
        let c0 = index.root(doc);
        let c1 = index.root(doc);
        let results = vec![
            eval(&Expr::Field("a".to_string()), c0.value()), // single value
            eval(
                &Expr::pipe(vec![Expr::Field("b".to_string()), Expr::Iterate]),
                c1.value(),
            ), // Many
            GenericResult::Owned(OwnedValue::Int(5)),
            GenericResult::ManyOwned(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
            GenericResult::None,
            GenericResult::Error(EvalError::new("boom")),
            GenericResult::Break("lbl".to_string()),
            // #400/#494: a prefix that reached the caller, then a control.
            // Both terminators get their own arm everywhere `Error`/`Break`
            // do, so both shapes sit in the vec.
            GenericResult::Partial(
                vec![OwnedValue::Int(1), OwnedValue::Int(2)],
                Control::Error(EvalError::new("late")),
            ),
            GenericResult::Partial(vec![OwnedValue::Int(3)], Control::Break("lbl".to_string())),
        ];

        // is_error / error / is_single_cursor (borrow &self)
        assert!(results[5].is_error());
        assert!(results[5].error().is_some());
        assert!(!results[2].is_error());
        assert!(results[2].error().is_none());
        assert!(!results[2].is_single_cursor());

        // stream_json / stream_yaml exercise every variant's match arm.
        for r in &results {
            let mut j = String::new();
            r.stream_json(&mut j, 0, |_| Ok(())).unwrap();
            let mut y = String::new();
            r.stream_yaml(&mut y, 2, |_| Ok(())).unwrap();
        }
        // Spot-check the owned and error stream output.
        let mut owned_json = String::new();
        results[2]
            .stream_json(&mut owned_json, 0, |_| Ok(()))
            .unwrap();
        assert_eq!(owned_json, "5");
        // An error writes nothing to `out` — `out` is stdout, and a diagnostic
        // there would be indistinguishable from a result. It comes back through
        // `stats.error` instead, for the caller to print to stderr (#355).
        let mut err_json = String::new();
        let err_stats = results[5]
            .stream_json(&mut err_json, 0, |_| Ok(()))
            .unwrap();
        assert_eq!(err_json, "", "diagnostics must never reach stdout");
        assert_eq!(
            err_stats.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );
        assert_eq!(err_stats.count, 0);

        let mut err_yaml = String::new();
        let err_stats = results[5]
            .stream_yaml(&mut err_yaml, 2, |_| Ok(()))
            .unwrap();
        assert_eq!(err_yaml, "", "diagnostics must never reach stdout");
        assert_eq!(
            err_stats.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );

        // Break escapes its label: an uncaught error like any other, and it too
        // stays off stdout.
        let mut brk = String::new();
        let brk_stats = results[6].stream_json(&mut brk, 0, |_| Ok(())).unwrap();
        assert_eq!(brk, "");
        assert!(brk_stats
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("not in label")));

        // A `Partial` streams its prefix to `out` and reports the control
        // through `stats.error` — the prefix is what #400/#494 stopped
        // discarding, and the diagnostic still stays off stdout.
        let mut partial_json = String::new();
        let mut seen = 0usize;
        let partial_stats = results[7]
            .stream_json(&mut partial_json, 0, |_| {
                seen += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(partial_json, "12");
        assert_eq!(seen, 2, "on_value runs once per streamed prefix value");
        assert_eq!(partial_stats.count, 2);
        assert!(partial_stats.any_truthy);
        assert_eq!(
            partial_stats.error.as_ref().map(|e| e.message.as_str()),
            Some("late")
        );

        let mut partial_yaml = String::new();
        let partial_stats = results[7]
            .stream_yaml(&mut partial_yaml, 2, |w| {
                use core::fmt::Write;
                writeln!(w)
            })
            .unwrap();
        assert_eq!(partial_yaml, "1\n2\n");
        assert_eq!(partial_stats.count, 2);
        assert_eq!(
            partial_stats.error.as_ref().map(|e| e.message.as_str()),
            Some("late")
        );

        // A `Partial` ending in a break reports the same "not in label"
        // diagnostic the bare `Break` arm does, after its prefix.
        let mut partial_brk = String::new();
        let brk_stats = results[8]
            .stream_json(&mut partial_brk, 0, |_| Ok(()))
            .unwrap();
        assert_eq!(partial_brk, "3");
        assert!(brk_stats
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("not in label")));
        let mut partial_brk_yaml = String::new();
        let brk_stats = results[8]
            .stream_yaml(&mut partial_brk_yaml, 2, |_| Ok(()))
            .unwrap();
        assert_eq!(partial_brk_yaml, "3");
        assert!(brk_stats
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("not in label")));

        // into_owned consumes; check the owned-family variants.
        let owned: Vec<Option<OwnedValue>> =
            results.into_iter().map(GenericResult::into_owned).collect();
        assert_eq!(owned[2], Some(OwnedValue::Int(5))); // Owned
        assert_eq!(
            owned[3],
            Some(OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2)
            ]))
        ); // ManyOwned
        assert_eq!(owned[4], None); // None
        assert_eq!(owned[5], None); // Error
        assert_eq!(owned[6], None); // Break

        // A prefix plus a control is not representable as one value, so
        // `into_owned` answers `None` the way `Error`/`Break` do — unlike
        // `collect_owned`, which keeps the prefix (checked separately).
        assert_eq!(owned[7], None); // Partial(_, Error)
        assert_eq!(owned[8], None); // Partial(_, Break)
    }

    #[test]
    fn test_generic_result_collect_owned_all_variants() {
        let doc = br#"{"b": [1, 2, 3]}"#;
        let index = JsonIndex::build(doc);
        let c = index.root(doc);
        let results = vec![
            eval(
                &Expr::pipe(vec![Expr::Field("b".to_string()), Expr::Iterate]),
                c.value(),
            ), // Many
            GenericResult::ManyOwned(vec![OwnedValue::Int(9)]),
            GenericResult::None,
            GenericResult::Error(EvalError::new("e")),
            GenericResult::Break("l".to_string()),
            GenericResult::Owned(OwnedValue::Bool(true)),
            GenericResult::Partial(
                vec![OwnedValue::Int(1), OwnedValue::Int(2)],
                Control::Error(EvalError::new("late")),
            ),
        ];
        let collected: Vec<Vec<OwnedValue>> = results
            .into_iter()
            .map(GenericResult::collect_owned)
            .collect();
        assert_eq!(collected[0].len(), 3); // Many -> 3 elements
        assert_eq!(collected[1], vec![OwnedValue::Int(9)]); // ManyOwned
        assert!(collected[2].is_empty()); // None
        assert!(collected[3].is_empty()); // Error
        assert!(collected[4].is_empty()); // Break
        assert_eq!(collected[5], vec![OwnedValue::Bool(true)]); // Owned

        // Unlike `into_owned`, this keeps the prefix — #400/#494's whole point.
        assert_eq!(collected[6], vec![OwnedValue::Int(1), OwnedValue::Int(2)]); // Partial
    }

    #[test]
    fn test_generic_result_one_cursor_streaming() {
        // at_offset lands on a node and yields a OneCursor result, exercising
        // the OneCursor arms of collect_owned / stream_json / stream_yaml.
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("at_offset(6)").unwrap(); // offset 6 == the `1`

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut j = String::new();
        result.stream_json(&mut j, 0, |_| Ok(())).unwrap();
        assert_eq!(j, "1");
        let mut y = String::new();
        result.stream_yaml(&mut y, 2, |_| Ok(())).unwrap();

        let result2 = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result2.collect_owned(), vec![OwnedValue::Int(1)]);
    }

    #[test]
    fn test_json_cursor_stream_json_rejects_pretty_indent() {
        // JsonCursor::stream_json only supports compact (indent_spaces == 0)
        // output; indented JSON->JSON cursor streaming isn't implemented, so
        // callers must fall back to the DOM path (#442). Exercised through
        // the generic OneCursor arm, same as the compact case above.
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("at_offset(6)").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut j = String::new();
        assert!(result.stream_json(&mut j, 2, |_| Ok(())).is_err());
    }

    #[test]
    fn test_yaml_generic_result_one_cursor_streaming() {
        // YAML counterpart of `test_generic_result_one_cursor_streaming`:
        // at_offset lands on a YamlCursor node and yields a OneCursor result,
        // exercising the YamlCursor `DocumentCursor::stream_json`/`stream_yaml`
        // trait delegation (as opposed to the inherent `YamlCursor` methods
        // called directly elsewhere).
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\n";
        let index = YamlIndex::build(yaml).unwrap();
        let expr = crate::jq::parse("at_offset(3)").unwrap(); // offset 3 == the `1`

        let result = eval_with_cursor(&expr, index.root(yaml));
        assert!(result.is_single_cursor());
        let mut j = String::new();
        result.stream_json(&mut j, 0, |_| Ok(())).unwrap();
        assert_eq!(j, "1");
        let mut y = String::new();
        result.stream_yaml(&mut y, 2, |_| Ok(())).unwrap();
        assert_eq!(y, "1");
    }

    #[test]
    fn test_at_position_no_node_and_tonumber_error() {
        // at_position with an out-of-range line yields the "no node" error.
        let json = br#"{"a": "xyz"}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("at_position(99; 1)").unwrap();
        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_error());

        // tonumber on a non-numeric string yields a conversion error.
        let expr = crate::jq::parse(".a | tonumber").unwrap();
        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_error());
    }

    #[test]
    fn test_arithmetic_semantics_are_threaded() {
        // The generic evaluator delegates arithmetic to the full evaluator; the
        // semantics parameter must reach that fallback so jq truncates float
        // modulo (issue #164) while yq keeps float modulo.
        use crate::jq::YqSemantics;

        let expr = crate::jq::parse("10.5 % 3").unwrap();

        let json = b"null";
        let index = JsonIndex::build(json);

        let jq_result = eval_using::<JqSemantics, _>(&expr, index.root(json).value());
        assert_eq!(jq_result.into_owned(), Some(OwnedValue::Int(1)));

        let yq_result = eval_using::<YqSemantics, _>(&expr, index.root(json).value());
        assert_eq!(yq_result.into_owned(), Some(OwnedValue::Float(1.5)));
    }

    // Coverage follow-ups for #532: the tests above exercise the common
    // `.foo`/`.[]`/`select(...)` shapes, but a few less-common `GenericResult`
    // combinations at Pipe/Compare/Select/Iterables/Scalars stage boundaries
    // weren't reached by any existing test. `Error`/`Break` in these arms
    // return immediately, so those two get their own dedicated tests instead
    // of being combined with the success-path ones.

    #[test]
    fn test_json_many_stage_result_produces_many_cursor_per_element() {
        // `.[("a","b")]` is a computed multi-key index: on an object with
        // both keys present as borrowed (non-owned) values, it resolves to a
        // plain `Many` (not `ManyCursor`) pipe-stage result. Piping that into
        // `.[]` evaluates the next stage per element, and each element's
        // `.[]` yields its own `ManyCursor` — exercising the `Many(vs)`
        // stage-transition arm's `ManyCursor` sub-case.
        let json = br#"{"a": [1, 2], "b": [3, 4]}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(r#".[("a","b")] | .[]"#).unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned(),
            vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3),
                OwnedValue::Int(4),
            ]
        );
    }

    #[test]
    fn test_json_iterate_then_field_error_propagates() {
        // When `.[]` yields a `ManyCursor` and a later stage errors on one
        // element (`.x` on a non-object), the error must propagate out of
        // the whole evaluation rather than being dropped silently.
        let json = br#"{"items": [{"x": 1}, 5]}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".items[] | .x").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_error());
    }

    #[test]
    fn test_json_iterate_then_break_propagates() {
        // Same shape as the error case above, but for `break`, which has its
        // own early-return arm alongside `Error`.
        let json = br#"{"items": [1, 2]}"#;
        let index = JsonIndex::build(json);
        let expr = Expr::Pipe(vec![
            Expr::Field("items".to_string()),
            Expr::Iterate,
            Expr::Break("out".to_string()),
        ]);

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(matches!(result, GenericResult::Break(label) if label == "out"));
    }

    #[test]
    fn test_json_iterate_then_single_key_index_expr_yields_one() {
        // Heterogeneous-flatten path of the `ManyCursor` stage arm: a
        // computed single-key index yields a plain `One` per element rather
        // than `OneCursor`, so the per-element results can't stay
        // `ManyCursor` and must flatten through the `One` arm. Built directly
        // as `Expr::IndexExpr` rather than parsed from `.["a"]`, since the
        // parser folds a literal-string bracket index into a plain
        // `Expr::Field` (which would instead take the all-`OneCursor` path).
        let json = br#"{"items": [{"a": 1}, {"a": 2}]}"#;
        let index = JsonIndex::build(json);
        let expr = Expr::Pipe(vec![
            Expr::Field("items".to_string()),
            Expr::Iterate,
            Expr::IndexExpr {
                target: Box::new(Expr::Identity),
                key: Box::new(Expr::Literal(Literal::String("a".to_string()))),
            },
        ]);

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned(),
            vec![OwnedValue::Int(1), OwnedValue::Int(2)]
        );
    }

    #[test]
    fn test_json_iterate_then_multi_key_index_expr_yields_many_and_many_owned() {
        // Same flatten path, but the per-element computed index now yields
        // `Many` (both keys found on the second item) or `ManyOwned` (a mix
        // of found/missing keys forces the owned fallback on the first and
        // third items), exercising both arms.
        let json = br#"{"items": [{"a": 1}, {"a": 1, "b": 2}, {"c": 3}]}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(r#".items[] | .[("a","b")]"#).unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned(),
            vec![
                OwnedValue::Int(1),
                OwnedValue::Null,
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Null,
                OwnedValue::Null,
            ]
        );
    }

    #[test]
    fn test_json_iterate_then_nested_iterate_yields_many_cursor_in_flatten() {
        // Same flatten path with a per-element `ManyCursor` (from a nested
        // `.x[]`) alongside a `None` (empty array), exercising the
        // `ManyCursor` arm of the heterogeneous flatten.
        let json = br#"{"items": [{"x": [10, 20]}, {"x": []}]}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".items[] | .x[]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned(),
            vec![OwnedValue::Int(10), OwnedValue::Int(20)]
        );
    }

    #[test]
    fn test_json_compare_left_many_cursor_uses_first_element() {
        // `Compare`'s `ManyCursor`-operand arm: `.[]` on the left side yields
        // a `ManyCursor`, and the comparison uses its first element.
        let json = br"[1, 2]";
        let index = JsonIndex::build(json);
        let expr = Expr::Compare {
            op: CompareOp::Eq,
            left: Box::new(Expr::Iterate),
            right: Box::new(Expr::Literal(Literal::Int(1))),
        };

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned(), Some(OwnedValue::Bool(true)));
    }

    #[test]
    fn test_json_compare_right_many_cursor_uses_first_element() {
        // Mirror of the above for the right-hand operand.
        let json = br"[1, 2]";
        let index = JsonIndex::build(json);
        let expr = Expr::Compare {
            op: CompareOp::Eq,
            left: Box::new(Expr::Literal(Literal::Int(1))),
            right: Box::new(Expr::Iterate),
        };

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned(), Some(OwnedValue::Bool(true)));
    }

    #[test]
    fn test_json_select_cond_one_truthy() {
        // `select`'s condition-result matching: a bare `One` (not
        // `OneCursor`) condition result arises when `eval` is used without a
        // cursor context, hitting a distinct arm from the `OneCursor` case
        // below.
        let json = b"5";
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::Select(Box::new(Expr::Identity)));

        let result = eval(&expr, index.root(json).value());
        assert_eq!(result.into_owned(), Some(OwnedValue::Int(5)));
    }

    #[test]
    fn test_json_select_cond_one_cursor_truthy() {
        // Same as above, but evaluated with cursor context, so the condition
        // result is `OneCursor` instead of `One`.
        let json = b"5";
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::Select(Box::new(Expr::Identity)));

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned(), Some(OwnedValue::Int(5)));
    }

    #[test]
    fn test_json_iterables_forwards_cursor() {
        // `iterables`/`scalars` forward an incoming cursor when present,
        // hitting the `OneCursor` arm of `cursor.map_or` instead of the
        // cursor-less default.
        let json = br"[1, 2]";
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::Iterables);

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.into_owned(),
            Some(OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2)
            ]))
        );
    }

    #[test]
    fn test_json_scalars_forwards_cursor() {
        let json = b"5";
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::Scalars);

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned(), Some(OwnedValue::Int(5)));
    }

    #[test]
    fn test_json_line_builtin_with_cursor() {
        // JSON counterpart of `test_yaml_line_builtin_with_cursor`: exercises
        // `JsonCursor`'s `DocumentCursor::line()` trait delegation (as
        // opposed to the inherent method called directly by
        // `test_cursor_line_column` in `json::light`).
        let json = b"{\n  \"foo\": 1\n}";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".foo | line").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned(), Some(OwnedValue::Int(2)));
    }

    #[test]
    fn test_json_column_builtin_with_cursor() {
        // JSON counterpart of `test_yaml_column_builtin_with_cursor`,
        // exercising `JsonCursor`'s `DocumentCursor::column()` delegation.
        let json = b"{\n  \"foo\": 1\n}";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".foo | column").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned(), Some(OwnedValue::Int(10)));
    }

    /// A `GenericResult` reduced to the parts these tests assert on, so the
    /// borrowed document can be dropped before the comparison.
    #[derive(Debug, PartialEq, Eq)]
    enum Summary {
        /// Every output, as compact JSON.
        Values(Vec<String>),
        Error(String),
        Break(String),
        /// The prefix (compact JSON) and the control that ended it.
        Partial(Vec<String>, Box<Self>),
        None,
    }

    /// Evaluate `filter` on `json` through the generic (CLI) evaluator.
    fn summarize(json: &[u8], filter: &str) -> Summary {
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(filter).unwrap();
        let json_of = |vs: &[OwnedValue]| vs.iter().map(OwnedValue::to_json).collect::<Vec<_>>();
        match eval_with_cursor(&expr, index.root(json)) {
            GenericResult::None => Summary::None,
            GenericResult::Error(e) => Summary::Error(e.message),
            GenericResult::Break(l) => Summary::Break(l),
            GenericResult::Partial(vs, Control::Error(e)) => {
                Summary::Partial(json_of(&vs), Box::new(Summary::Error(e.message)))
            }
            GenericResult::Partial(vs, Control::Break(l)) => {
                Summary::Partial(json_of(&vs), Box::new(Summary::Break(l)))
            }
            other => Summary::Values(json_of(&other.collect_owned())),
        }
    }

    /// `Summary::Partial` of `prefix` then an error with `message`.
    fn partial_err(prefix: &[&str], message: &str) -> Summary {
        Summary::Partial(
            prefix.iter().map(|s| (*s).to_string()).collect(),
            Box::new(Summary::Error(message.to_string())),
        )
    }

    /// `Summary::Partial` of `prefix` then `break $out`.
    fn partial_break(prefix: &[&str]) -> Summary {
        Summary::Partial(
            prefix.iter().map(|s| (*s).to_string()).collect(),
            Box::new(Summary::Break("out".to_string())),
        )
    }

    #[test]
    fn generic_pipe_stages_keep_the_prefix_before_a_control() {
        // The generic evaluator has its own copy of the #400/#494 pipe
        // handling, one arm per shape the previous stage can take. Every
        // expectation here was verified against jq 1.7.1.

        // Previous stage is a `Partial`: its prefix is piped through this
        // stage before the held control is re-attached.
        assert_eq!(
            summarize(b"null", r#"(1,2,error("x")) | .+10"#),
            partial_err(&["11", "12"], "x")
        );
        // A control hit *while* piping the prefix wins over the held one.
        assert_eq!(
            summarize(b"null", r#"(1,2,error("x")) | (., error("z"))"#),
            partial_err(&["1"], "z")
        );
        assert_eq!(
            summarize(
                b"null",
                r#"(1,2,error("x")) | if . == 2 then error("y") else . end"#
            ),
            partial_err(&["1"], "y")
        );
        assert_eq!(
            summarize(
                b"null",
                "(1,2,error(\"x\")) | if . == 2 then break $out else . end"
            ),
            partial_break(&["1"])
        );
        // Piping the prefix through `empty` leaves nothing, so `partial_generic`
        // normalizes the held control back to a bare `Error`.
        assert_eq!(
            summarize(b"null", r#"(1,2,error("x")) | empty"#),
            Summary::Error("x".to_string())
        );

        // Previous stage is a `ManyCursor` (`.[]` over a document array): the
        // elements already piped through survive a later element's control.
        assert_eq!(
            summarize(b"[1,2]", r#".[] | (., error("x"))"#),
            partial_err(&["1"], "x")
        );
        assert_eq!(
            summarize(b"[1,2]", ".[] | if . == 2 then break $out else . end"),
            partial_break(&["1"])
        );

        // Previous stage is a borrowed `Many` — produced by a computed index
        // with more than one key, the only shape that reaches that arm.
        assert_eq!(
            summarize(b"[10,20,30]", r#".[(0,1)] | (., error("x"))"#),
            partial_err(&["10"], "x")
        );
        assert_eq!(
            summarize(
                b"[10,20,30]",
                r#".[(0,1)] | if . == 20 then error("y") else . end"#
            ),
            partial_err(&["10"], "y")
        );
        assert_eq!(
            summarize(
                b"[10,20,30]",
                ".[(0,1)] | if . == 20 then break $out else . end"
            ),
            partial_break(&["10"])
        );

        // `select` fans out over its condition's outputs (#378), so — like
        // every other stream-forwarding construct above — a `Partial`
        // condition keeps whatever the truthy bits already produced before
        // the trailing control surfaces. Matches jq 1.7.1
        // (`select((true,error("x")))` on `5` prints `5`, then fails).
        assert_eq!(
            summarize(b"5", r#"select((true,error("x")))"#),
            partial_err(&["5"], "x")
        );
        assert_eq!(
            summarize(b"5", "select((true,break $out))"),
            partial_break(&["5"])
        );
        // `select(...)? ` swallows the condition's error like `try select(...)
        // catch empty` would, but still keeps the truthy bits' output.
        assert_eq!(
            summarize(b"5", r#"select((true,error("x")))?"#),
            Summary::Values(vec!["5".to_string()])
        );
    }

    #[test]
    fn generic_value_position_partial_collapses_to_its_control() {
        // Comparison, computed indexing and `select` each consult their
        // operand once, so a `Partial` there is reduced the same way a
        // multi-output operand already is.

        // Comparison keeps the prefix's first output on either side and drops
        // the control. (jq 1.7.1 prints `true` and *then* fails; succinctly
        // exits 0 — a divergence this pins rather than endorses.)
        let truthy = Summary::Values(vec!["true".to_string()]);
        assert_eq!(summarize(b"null", r#"(1,error("x")) == 1"#), truthy);
        assert_eq!(summarize(b"null", r#"1 == (1,error("x"))"#), truthy);

        // A computed *target* that is a `Partial` surfaces the control alone.
        // (jq 1.7.1 prints 7 and 8 first; pinned here, not endorsed.)
        assert_eq!(
            summarize(b"[[7],[8]]", r#"(.[0],(.[1],error("x")))[(0+0)]"#),
            Summary::Error("x".to_string())
        );
        assert_eq!(
            summarize(b"[[7],[8]]", "(.[0],(.[1],break $out))[(0+0)]"),
            Summary::Break("out".to_string())
        );
    }
}
