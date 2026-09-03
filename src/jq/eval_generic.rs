//! Generic expression evaluator for jq-like queries.
//!
//! This module provides a document-agnostic evaluator that works with any type
//! implementing the `DocumentValue` trait, enabling direct evaluation of both
//! JSON and YAML without intermediate conversion.

// #1670: see `eval.rs`'s own copy of this attribute for the full
// rationale. Use `crate::jq::eval::vec_with_capacity`/`string_with_capacity`
// for a legitimate single-length site here.
#![warn(clippy::disallowed_methods)]

#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::rc::Rc;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;
#[cfg(test)]
use std::rc::Rc;

use indexmap::IndexMap;

use super::document::{
    collapsed_fields, collapsed_fields_if, effective_fields_checked,
    effective_fields_with_raw_last, effective_keys, effective_len_checked, key_delimiter_ok,
    key_display_string, key_display_string_kind, key_is_malformed, resolve_display_key,
    trailing_element_gap_ok, value_delimiter_ok, DisplayKeyGuard, DistinctKeyCursors,
    DocumentCursor, DocumentElements, DocumentFields, DocumentValue, IndentSpec, JsonConvention,
};
use super::eval::{
    apply_compare_op, arith_combine, as_var_refs, bind_def, bind_def_call,
    cannot_reserve_cross_product, classify_limit_n, classify_nth_n, classify_parent_n,
    collapse_vec, collect_pattern_var_names, compare_values, enter_def_call_frame,
    eval as full_eval, eval_each_owned, eval_foreach_with_values, eval_reduce_with_values,
    extract_pattern_bindings, format_owned, has_type_mismatch_is_permissive, index_component_value,
    index_in_array_bounds, index_one_owned as index_owned_by_key, is_pure_chain_link,
    is_retryable_stop, literal_to_owned, needs_path_context, numeric_key_to_array_index,
    numeric_key_to_index, owned_bound_to_i64, owned_to_string, slice_object_as_yq_children,
    slice_owned_value_read, substitute_bound_var, substitute_vars, suppresses, tonumber_from_str,
    vec_with_capacity, yq_negative_index_check, Control, Demand, EvalError, EvalSemantics, EvalTag,
    Flow, JqSemantics, LimitN, PathTrail, QueryResult, YqSemantics,
};
#[cfg(test)]
use super::expr::FuncDefBound;
use super::expr::{Builtin, CompareOp, Expr, FormatType, Pattern};
use super::slice::{slice_str, SliceBounds};
use super::value::{owned_value_eq, NumberRepr, OwnedValue};
use crate::json::JsonIndex;

/// Recursion-depth ceiling for [`to_owned`]/[`to_owned_cursor`]/
/// [`to_owned_with_comments`] (#998).
///
/// JSON's semi-index parser (`src/json/simple.rs`/`src/json/standard.rs`)
/// is a flat, non-recursive scan with no depth limit of its own, so
/// adversarially deep JSON parses/indexes fine -- it's specifically this
/// tree-materialization step, recursing once per nesting level, that needs
/// the guard.
///
/// A clean, catchable error (matching real jq's own parse-time depth limit —
/// confirmed live, `jq '.'` on 500+ levels of `[[[...]]]` raises "Exceeds
/// depth limit for parsing") is the ideal failure mode here, but these
/// functions are called pervasively throughout the core evaluator's hot path
/// (truthiness checks, streaming, comparisons — dozens of call sites, not
/// just at a single "materialize the final result" boundary), so threading a
/// `Result` through their signatures would ripple across the whole
/// evaluator. A controlled panic closes the actual safety concern (a raw
/// stack overflow is undefined behavior; a panic is not) without that
/// blast radius — see #998's own text, which names a hard process abort as
/// an acceptable outcome alongside a clean error.
///
/// 256, not the 128 `src/yaml/parser.rs`/`src/json/validate.rs` use for
/// their own (unrelated) guards: `tests/jq_cli_tests.rs`'s
/// `test_walk_deep_nesting_does_not_overflow_the_stack` already pins
/// `walk(.)` working at depth 200 as a deliberate, previously-measured
/// capability, and that query's *output* still passes through this same
/// materialization step — a 128 limit broke that test outright. Measured
/// directly on a debug build (the more fragile of the two, larger
/// per-frame stack cost than release): the real crash boundary for this
/// function sits between depth 2000 (safe) and 3000 (overflow) — 256 clears
/// the 200 floor with margin and sits comfortably under that boundary,
/// including headroom for a CI runner with a smaller default stack than the
/// dev machine this was measured on.
///
/// `pub`, not private: every recursive tree-materialization function that
/// isn't itself one of this module's own (`to_owned`/`to_owned_cursor`/
/// `to_owned_with_comments`, all guarded via [`assert_nesting_depth`] below)
/// needs the same ceiling -- `lazy.rs`'s `cursor_to_owned` imports this
/// rather than hand-rolling its own copy, so the whole binary has exactly
/// one number to retune for this guard family (#998 review: two independent
/// copies of this same constant had already drifted apart in wording, if
/// not value, before this consolidation).
///
/// `jq_runner.rs`'s `print_json` also imports this constant, but only for an
/// unrelated purpose: recognizing `to_owned_cursor_at_depth`'s own panic
/// message text (see `nesting_depth_panic_message`) so it can be
/// distinguished from an unrelated panic. `print_json`'s *own* recursion
/// guard was moved to [`super::value::MAX_VALUE_TREE_DEPTH`] by #1819 --
/// see that constant's doc comment and `print_json`'s own for why one
/// shared ceiling was wrong for this specific pairing: `print_json` prints
/// values this crate has already promised (via `MAX_VALUE_TREE_DEPTH`) to
/// support up to that depth, and its own measured crash boundary (600-700,
/// noted above) has room to match that promise instead of failing 128
/// levels earlier.
pub const MAX_NESTING_DEPTH: usize = 256;

/// Panics past [`MAX_NESTING_DEPTH`] levels of nesting (#998).
///
/// A thin wrapper around `value::assert_depth` (#1018) -- the
/// underlying `assert!` used to be hand-copied here (fixing a #998 review
/// finding that `to_owned`/`to_owned_cursor`/`to_owned_with_comments` each
/// carried their own byte-identical copy), and again independently in
/// `value.rs` for [`value::MAX_VALUE_TREE_DEPTH`](super::value::MAX_VALUE_TREE_DEPTH)'s
/// own guard -- the same duplication shape recurring one level up.
///
/// `#[track_caller]` so a panic reports the guarded function's own call
/// site rather than collapsing to `assert_depth`'s shared body (#1020
/// code review).
#[track_caller]
pub fn assert_nesting_depth(depth: usize) {
    super::value::assert_depth(depth, MAX_NESTING_DEPTH);
}

/// Checked sibling of [`assert_nesting_depth`] (#1818).
///
/// Same [`MAX_NESTING_DEPTH`] ceiling as `assert_depth`'s own `panic!`,
/// sharing its message-building via `value::nesting_depth_exceeded_message`
/// (also used by `print_json`'s own `anyhow::ensure!`, though that guard
/// checks against the separate, wider `MAX_VALUE_TREE_DEPTH` ceiling since
/// #1819 -- the shared function takes `max` as a parameter precisely so the
/// message text can't drift between guards that legitimately check
/// different ceilings). This one is a catchable `EvalError` instead of a
/// panic -- for a caller that isn't the evaluator's own hot recursion
/// (where a panic is deliberate, see `to_owned`'s own doc comment on why an
/// `EvalError` there would make a stack-overflow guard catchable by
/// `try`/`catch`) but a CLI-level input-validation walk (`jq_runner.rs`'s
/// `validate_json_delimiters`, reached before any user filter runs) that
/// already threads a `Result` and has no such concern.
pub fn check_nesting_depth(depth: usize) -> Result<(), EvalError> {
    if depth < MAX_NESTING_DEPTH {
        Ok(())
    } else {
        Err(EvalError::new(
            super::value::nesting_depth_exceeded_message(MAX_NESTING_DEPTH),
        ))
    }
}

/// Checked sibling of [`to_owned`] (#2299): identical materialization
/// logic, `check_nesting_depth` (catchable) in place of
/// `assert_nesting_depth` (panic).
///
/// For the one call site (`materialize_stream_item`'s CLI-output boundary,
/// `jq_runner.rs`) that isn't the evaluator's own hot-path recursion and
/// already threads a `Result` to receive a clean error on instead of a
/// panic. A fused, single-walk duplicate rather than a lightweight
/// depth-only pre-check run ahead of `to_owned` (an earlier revision of
/// this fix took that approach): `materialize_stream_item` is reached by
/// every ordinary `succinctly jq` invocation's default per-document output
/// path, not just `--slurp` -- a pre-check would silently double the
/// materialization cost of every `.`-style query, this codebase's single
/// most benchmarked shape (see this crate's own `CLAUDE.md` performance
/// tables), for the sake of a guard that only ever fires past 256 levels
/// of nesting. Mirrors `yq_runner.rs`'s own
/// `to_owned_canonicalizing_numbers_at_depth`, the established precedent
/// for this exact "same recursion shape, panic swapped for a checked
/// guard" duplication -- accepting the same drift risk that precedent
/// already accepts, in exchange for the same zero-added-walk cost.
pub fn to_owned_checked<V: DocumentValue>(value: &V) -> Result<OwnedValue, EvalError> {
    to_owned_checked_at_depth(value, 0)
}

fn to_owned_checked_at_depth<V: DocumentValue>(
    value: &V,
    depth: usize,
) -> Result<OwnedValue, EvalError> {
    check_nesting_depth(depth)?;
    if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut guard = DisplayKeyGuard::default();
        let mut f = fields;
        let mut is_first = true;
        let mut last_field: Option<V::Cursor> = None;
        while let Some((field, rest)) = f.uncons() {
            let Some(key) = resolve_display_key(&field.key, &map, &mut guard)? else {
                return Err(f.malformed_member_error());
            };
            if !key_delimiter_ok::<V::Fields>(&field.key, &field.key_cursor, is_first)
                || !value_delimiter_ok::<V::Fields>(Some(&field.value), &field.value_cursor)
            {
                return Err(f.malformed_member_error());
            }
            map.insert(key, to_owned_checked_at_depth(&field.value, depth + 1)?);
            last_field = Some(field.value_cursor);
            f = rest;
            is_first = false;
        }
        if f.ends_unpaired() {
            return Err(f.malformed_member_error());
        }
        if let Some(last) = &last_field {
            if !trailing_element_gap_ok(last, b'}') {
                return Err(last.malformed_delimiter_error());
            }
        }
        Ok(OwnedValue::Object(map))
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut elems = elements;
        let mut is_first = true;
        let mut last_elem: Option<V::Cursor> = None;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            if let Some(pos) = elem_cursor.text_position() {
                let expected = if is_first { None } else { Some(b',') };
                if !elem_cursor.preceding_delimiter_ok(pos, expected) {
                    return Err(elem_cursor.malformed_delimiter_error());
                }
            }
            items.push(to_owned_checked_at_depth(&elem_cursor.value(), depth + 1)?);
            last_elem = Some(elem_cursor);
            elems = rest;
            is_first = false;
        }
        if let Some(last) = &last_elem {
            if !trailing_element_gap_ok(last, b']') {
                return Err(last.malformed_delimiter_error());
            }
        }
        Ok(OwnedValue::Array(items))
    } else if value.is_null() {
        Ok(OwnedValue::Null)
    } else if let Some(b) = value.as_bool() {
        Ok(OwnedValue::Bool(b))
    } else if let Some(literal) = value.number_literal() {
        Ok(OwnedValue::from_number_literal(&literal))
    } else if let Some(i) = value.as_i64() {
        Ok(OwnedValue::Int(i))
    } else if let Some(f) = value.as_f64() {
        Ok(OwnedValue::Float(f))
    } else if let Some(s) = value.as_str() {
        Ok(OwnedValue::String(s.into_owned()))
    } else if let Some(reason) = value.string_decode_error() {
        Err(EvalError::decode_failure(reason))
    } else if value.is_error() {
        Err(EvalError::decode_failure(
            value
                .error_message()
                .unwrap_or("malformed value in document"),
        ))
    } else {
        Ok(OwnedValue::Null)
    }
}

/// Convert a DocumentValue to an OwnedValue.
///
/// This enables the evaluator to work with both JSON and YAML inputs.
/// Note: The order of checks is important! Check containers first, then scalars,
/// because YAML scalars may have type coercion (e.g., unquoted "true" is a bool).
///
/// Panics past [`MAX_NESTING_DEPTH`] levels of nesting (#998) rather than
/// recursing unbounded and overflowing the call stack. That guard stays a
/// `panic!` even though this function is now fallible: unbounded recursion is
/// a different failure class from a data error, and routing it through
/// `EvalError` would make a stack-overflow guard catchable by `try`/`catch`.
///
/// Returns `Err` when a scalar the semi-index accepted as a string token
/// cannot be *decoded* (#1098, #1247) -- see
/// [`DocumentValue::string_decode_error`].
pub fn to_owned<V: DocumentValue>(value: &V) -> Result<OwnedValue, EvalError> {
    to_owned_at_depth(value, 0)
}

/// The error for an object member the format's index accepted but its grammar
/// does not -- a key that will not stringify, or a child with nothing to pair
/// it with (#1194). `None` when every member is well formed.
///
/// For a caller that has an error channel but no walk of its own to hang the
/// check on -- `to_entries`, whose `effective_fields` reports an unpaired
/// trailing child as plain exhaustion. A key-only walk, negligible there next
/// to materializing every value, which that builtin does anyway.
///
/// `length` deliberately does **not** use this: it reaches
/// [`effective_len_checked`], which folds the same check into the walk it was
/// already making. A pre-check in front of that call measured +64% on a 20 MB
/// `wide` document, and answered only for the `length` spelling -- never
/// `keys | length`, which reaches `effective_len` by a different route.
///
/// `ends_unpaired` alone would be O(1), but it sees only a trailing orphan,
/// never a bad key.
fn malformed_object_member<F: DocumentFields>(fields: &F) -> Option<EvalError> {
    let mut walk = fields.clone();
    let mut is_first = true;
    while let Some((key, cursor, rest)) = walk.uncons_key() {
        // `key_is_malformed` rather than a `key_string().is_none()` test
        // written out here: it is the one definition of the distinction
        // between #1194's structurally-impossible key and #1247's merely
        // undecodable one, which want opposite answers and which the first
        // cut of this function conflated. See its own doc comment.
        if key_is_malformed(&key) {
            return Some(walk.malformed_member_error());
        }
        // #1677: comma-before-key rides this same key-only walk for free.
        if !key_delimiter_ok::<F>(&key, &cursor, is_first) {
            return Some(walk.malformed_member_error());
        }
        walk = rest;
        is_first = false;
    }
    walk.ends_unpaired().then(|| walk.malformed_member_error())
}

fn to_owned_at_depth<V: DocumentValue>(value: &V, depth: usize) -> Result<OwnedValue, EvalError> {
    assert_nesting_depth(depth);
    // Check containers first (arrays and objects have no type ambiguity)
    if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut guard = DisplayKeyGuard::default();
        let mut f = fields;
        let mut is_first = true;
        // #2262: the last real field's own cursor, retained past the loop
        // so the trailing-gap check below (a stray `,` *after* a real last
        // field, `{"a":1,}`) has something to check from -- mirrors
        // `to_owned_cursor_at_depth`'s own `last_field` (#2243).
        let mut last_field: Option<V::Cursor> = None;
        while let Some((field, rest)) = f.uncons() {
            // A key that will not *decode* (#1247/#1385) is preserved via
            // its raw source span rather than raised on (#1642), matching
            // `length`/`keys_unsorted`/`.`. A key that will not stringify
            // at all is a different, structural fault -- a key JSON's
            // grammar never allowed (`{123: 1}`), which the semi-index
            // accepted because `:` and `,` mean the same nothing to it --
            // and that still raises: dropping the field silently is #1194,
            // the disagreement #1385's postmortem names as the thing to
            // avoid.
            let Some(key) = resolve_display_key(&field.key, &map, &mut guard)? else {
                return Err(f.malformed_member_error());
            };
            // #1677: same delimiter class as #1194 above, one layer up --
            // free here since `uncons` already resolved both key and value.
            if !key_delimiter_ok::<V::Fields>(&field.key, &field.key_cursor, is_first)
                || !value_delimiter_ok::<V::Fields>(Some(&field.value), &field.value_cursor)
            {
                return Err(f.malformed_member_error());
            }
            map.insert(key, to_owned_at_depth(&field.value, depth + 1)?);
            last_field = Some(field.value_cursor);
            f = rest;
            is_first = false;
        }
        // The walk ran out -- but on an unpaired child, or genuinely? Only
        // the list it *finished* on can tell those apart, and `uncons`
        // reports both as `None` (#1194). `false` for every format whose
        // parser validates, so this costs YAML nothing.
        if f.ends_unpaired() {
            return Err(f.malformed_member_error());
        }
        // #2262: #2211's `container_gap_ok` (a stray `,` with zero real
        // fields, `{,}`) needs the *container's own* cursor to find its
        // opening `{` -- this function only ever receives a bare `value: &V`
        // (never a cursor for the container itself, unlike
        // `to_owned_cursor_at_depth`'s `cursor` parameter), and once `f` is
        // exhausted there is no way to reconstruct one. Same limitation
        // #2211 already documented for `jq_runner::standard_json_to_jq_value`'s
        // identical "value only, no container position" shape -- `{,}`
        // remains unchecked here for that reason. #2243's
        // `trailing_element_gap_ok` *is* checkable, though: it only needs
        // the last real field's own cursor, retained above.
        if let Some(last) = &last_field {
            if !trailing_element_gap_ok(last, b'}') {
                return Err(last.malformed_delimiter_error());
            }
        }
        Ok(OwnedValue::Object(map))
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut elems = elements;
        let mut is_first = true;
        // #2262: same reasoning as the object arm's own `last_field` above.
        let mut last_elem: Option<V::Cursor> = None;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            // #1677: no bare-value walk over `DocumentElements` carries a
            // position, so this switches to the cursor-yielding sibling of
            // `uncons` -- always available, same navigation underneath --
            // purely to reach `text_position()` for the gap check.
            if let Some(pos) = elem_cursor.text_position() {
                let expected = if is_first { None } else { Some(b',') };
                if !elem_cursor.preceding_delimiter_ok(pos, expected) {
                    return Err(elem_cursor.malformed_delimiter_error());
                }
            }
            items.push(to_owned_at_depth(&elem_cursor.value(), depth + 1)?);
            last_elem = Some(elem_cursor);
            elems = rest;
            is_first = false;
        }
        // #2262: same reasoning as the object arm's own check above --
        // `[,]` remains unchecked here (no container cursor available),
        // but `[1,]` is, via the last real element's own cursor.
        if let Some(last) = &last_elem {
            if !trailing_element_gap_ok(last, b']') {
                return Err(last.malformed_delimiter_error());
            }
        }
        Ok(OwnedValue::Array(items))
    // Then check scalars in order of specificity
    } else if value.is_null() {
        Ok(OwnedValue::Null)
    } else if let Some(b) = value.as_bool() {
        Ok(OwnedValue::Bool(b))
    } else if let Some(literal) = value.number_literal() {
        Ok(OwnedValue::from_number_literal(&literal))
    } else if let Some(i) = value.as_i64() {
        Ok(OwnedValue::Int(i))
    } else if let Some(f) = value.as_f64() {
        Ok(OwnedValue::Float(f))
    } else if let Some(s) = value.as_str() {
        Ok(OwnedValue::String(s.into_owned()))
    } else if let Some(reason) = value.string_decode_error() {
        // The case this function used to swallow. `as_str` above answered
        // `None`, but the value *is* a string token -- its bytes just don't
        // decode. Raising here is #1247's core fix; the deferral comment that
        // used to sit in the `else` below (naming #1098 and PR #1190's
        // reverted `panic!`) described exactly this and is now resolved.
        Err(EvalError::decode_failure(reason))
    } else if value.is_error() {
        // A *structurally* malformed value -- `[xyz123]`, `[tru]` -- which
        // the semi-index accepted as a span but could not classify as any
        // JSON token. It used to materialize as `null`, so `[xyz123]` came
        // back as `[null]` at exit 0 where real jq raises a parse error
        // (#1194). The semi-index's own message is more specific than
        // anything reconstructible here, so it is passed through verbatim.
        // #2286: `decode_failure`, not `new` -- same uncatchable class as
        // every other `StandardJson::Error`/`is_error()` site this issue
        // fixed (confirmed live: this exact generic-evaluator `to_owned_at_depth`
        // is what `resolve_fold_source`/`reduce`'s fold-source path calls,
        // so `[1, xyz123] | try add catch "caught"` stayed wrongly catchable
        // until this arm was retagged too).
        Err(EvalError::decode_failure(
            value
                .error_message()
                .unwrap_or("malformed value in document"),
        ))
    } else {
        // A genuinely unknown type: no format implements one today, so this
        // is exhaustiveness rather than a live path.
        Ok(OwnedValue::Null)
    }
}

/// Converts a `DocumentCursor`'s value to an `OwnedValue`, resolving an
/// explicit YAML tag along the way (e.g. `!!str 1` materializes as the
/// string `"1"`, not the number `1` — issue #747).
///
/// A bare [`DocumentValue`] has already lost its tag by the time it reaches
/// [`to_owned`] — tag lookup is keyed by byte position, which only a cursor
/// carries ([`DocumentCursor::explicit_tag`]). Mirrors `to_owned`'s
/// container-first structure, recursing via cursors
/// (`field.value_cursor`/`elems.uncons_cursor`, as in
/// [`to_owned_with_comments`]) so the tag check reaches every scalar, not
/// just the top-level one. Call sites that already hold a cursor (rather
/// than a bare value) should use this instead of `to_owned(&cursor.value())`.
///
/// Panics past [`MAX_NESTING_DEPTH`] levels of nesting (#998), same as
/// [`to_owned`].
pub fn to_owned_cursor<C: DocumentCursor>(cursor: &C) -> Result<OwnedValue, EvalError> {
    to_owned_cursor_at_depth(cursor, 0)
}

/// Map every value through [`to_owned`], short-circuiting at the first
/// decode failure.
///
/// One definition for the
/// `.iter().map(to_owned).collect::<Result<Vec<_>, _>>()` shape 13 call
/// sites across this file spelled out with their own turbofish (#1824).
/// #1824 itself, filed against an earlier revision of this file, counted
/// 19 (18 of which were this shape); most had already folded onto
/// `collect_cursors()`/`collect_values()` by the time this landed, leaving
/// 13 real sites for this pair of helpers to absorb.
pub fn to_owned_all<'a, V: DocumentValue + 'a>(
    values: impl IntoIterator<Item = &'a V>,
) -> Result<Vec<OwnedValue>, EvalError> {
    values.into_iter().map(to_owned).collect()
}

/// The cursor-collecting sibling of [`to_owned_all`], for [`to_owned_cursor`].
pub fn to_owned_all_cursors<'a, C: DocumentCursor + 'a>(
    cursors: impl IntoIterator<Item = &'a C>,
) -> Result<Vec<OwnedValue>, EvalError> {
    cursors.into_iter().map(to_owned_cursor).collect()
}

fn to_owned_cursor_at_depth<C: DocumentCursor>(
    cursor: &C,
    depth: usize,
) -> Result<OwnedValue, EvalError> {
    assert_nesting_depth(depth);
    let value = cursor.value();
    if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut guard = DisplayKeyGuard::default();
        let mut f = fields;
        let mut is_first = true;
        // #2243: the last real field's own cursor, retained past the loop so
        // the trailing-gap check below (a stray `,` *after* a real last
        // field, `{"a":1,}`) has something to check from -- distinct from
        // #2211's `map.is_empty()` check just below, which only ever catches
        // a stray `,` with no real field at all (`{,}`). Mirrors the array
        // arm's own `last_elem`: only the cursor is kept, and
        // `trailing_element_gap_ok` resolves a value from it (if it even
        // needs to) once, after the loop, not per field.
        let mut last_field: Option<C> = None;
        while let Some((field, rest)) = f.uncons() {
            // Same key handling as `to_owned_at_depth` above, same reasons --
            // these two conversions are copies of each other and a fix that
            // moved only one would leave the cursor and value domains
            // disagreeing about whether a document is valid.
            let Some(key) = resolve_display_key(&field.key, &map, &mut guard)? else {
                return Err(f.malformed_member_error());
            };
            // Same #1677 checks as `to_owned_at_depth` above, same reason.
            if !key_delimiter_ok::<<C::Value as DocumentValue>::Fields>(
                &field.key,
                &field.key_cursor,
                is_first,
            ) || !value_delimiter_ok::<<C::Value as DocumentValue>::Fields>(
                Some(&field.value),
                &field.value_cursor,
            ) {
                return Err(f.malformed_member_error());
            }
            map.insert(
                key,
                to_owned_cursor_at_depth(&field.value_cursor, depth + 1)?,
            );
            last_field = Some(field.value_cursor);
            f = rest;
            is_first = false;
        }
        if f.ends_unpaired() {
            return Err(f.malformed_member_error());
        }
        // #2211: `key_delimiter_ok`/`value_delimiter_ok` above only ever run
        // against a real field -- a stray `,` with no real field at all
        // (`{,}`) leaves the loop never having run, so nothing above catches
        // it. `map.is_empty()` after the walk is exactly that case (a
        // genuine `{}` also reaches here, and `container_gap_ok` answers
        // `true` for it -- see that method's own doc comment).
        if map.is_empty() {
            if !cursor.container_gap_ok(b'}') {
                return Err(cursor.malformed_delimiter_error());
            }
        } else {
            let value_cursor = last_field.expect("map non-empty implies a real field was inserted");
            if !trailing_element_gap_ok(&value_cursor, b'}') {
                return Err(cursor.malformed_delimiter_error());
            }
        }
        Ok(OwnedValue::Object(map))
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut elems = elements;
        let mut is_first = true;
        // #2243: same reasoning as the object arm's own `last_field` above.
        let mut last_elem: Option<C> = None;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            // #1677: same gap check as `to_owned_at_depth`'s array loop.
            if let Some(pos) = elem_cursor.text_position() {
                let expected = if is_first { None } else { Some(b',') };
                if !elem_cursor.preceding_delimiter_ok(pos, expected) {
                    return Err(elem_cursor.malformed_delimiter_error());
                }
            }
            items.push(to_owned_cursor_at_depth(&elem_cursor, depth + 1)?);
            last_elem = Some(elem_cursor);
            elems = rest;
            is_first = false;
        }
        // #2211: same reasoning as the object arm's own check just above,
        // for a stray `,` with no real element (`[,]`).
        if items.is_empty() {
            if !cursor.container_gap_ok(b']') {
                return Err(cursor.malformed_delimiter_error());
            }
        } else {
            let last = last_elem.expect("items non-empty implies a real element was pushed");
            if !trailing_element_gap_ok(&last, b']') {
                return Err(cursor.malformed_delimiter_error());
            }
        }
        Ok(OwnedValue::Array(items))
    } else {
        // An applicable explicit tag resolves from the raw text and so can
        // succeed where a plain decode would not; only the untagged fallback
        // can raise.
        match cursor
            .explicit_tag()
            .and_then(|tag| tagged_scalar_to_owned(tag, &value))
            .or_else(|| {
                // A JSON-sourced number literal never keeps its own
                // spelling (#978, #1398) -- checked before the ordinary
                // `to_owned_at_depth` fallback below, which preserves it
                // (correct for genuine YAML, #918). `explicit_tag` still
                // takes precedence: a tag-forced type is a stronger,
                // narrower signal than the document's own source format.
                if cursor.canonicalize_numbers() {
                    value
                        .number_literal()
                        .map(|literal| OwnedValue::from_number_literal_plain(&literal))
                } else {
                    None
                }
            }) {
            Some(owned) => Ok(owned),
            None => to_owned_at_depth(&value, depth),
        }
    }
}

/// Resolves a scalar's explicit YAML tag (`!!str`, `!!int`, ...) against its
/// raw source text, or `None` if the value isn't a scalar with text (a
/// container, already excluded by [`to_owned_cursor`]'s caller) or the tag
/// doesn't apply to this text (e.g. `!!int` on `"abc"` — [`resolve_tagged`]
/// itself decides applicability).
fn tagged_scalar_to_owned<V: DocumentValue>(tag: &str, value: &V) -> Option<OwnedValue> {
    let text = value.as_str()?;
    let resolved = crate::yaml::resolve_tagged(&text, tag)?;
    Some(resolved.to_owned_value(text))
}

/// Materializes `value`, preferring the cursor-aware, tag-resolving
/// [`to_owned_cursor`] when a cursor is available (issue #747) and falling
/// back to the cursor-less [`to_owned`] otherwise — e.g. a computed value
/// with no navigated position (see [`DocumentCursor::line`]'s "not
/// available" contract, which the same computed-vs-navigated distinction
/// applies to). Since `cursor.value()` and a separately-threaded `value`
/// always describe the same node on every call site in this module, `value`
/// itself is only needed for the `None` fallback.
fn to_owned_with_cursor<V: DocumentValue>(
    value: &V,
    cursor: Option<V::Cursor>,
) -> Result<OwnedValue, EvalError> {
    match cursor {
        Some(c) => to_owned_cursor(&c),
        None => to_owned(value),
    }
}

/// [`to_owned_with_cursor`] for the one job that must not fail: rendering a
/// value into the text of an error that is *already* being raised.
///
/// `EvalError::cannot_iterate_with(tag, &v)` and friends materialize `v` only to quote
/// it back to the user. Propagating a decode failure out of those call sites
/// would mean abandoning a real, correctly-diagnosed error in order to report
/// "the error message could not be built", which is strictly less useful --
/// so a decode failure degrades to `null` here, on purpose, and the outer
/// error still surfaces with its own accurate wording. This is the *only*
/// sanctioned lossy materialization left in this module (#1247); every other
/// caller takes the fallible one above.
fn to_owned_for_diagnostic<V: DocumentValue>(value: &V, cursor: Option<V::Cursor>) -> OwnedValue {
    to_owned_with_cursor(value, cursor).unwrap_or(OwnedValue::Null)
}

/// The shared "is this a decode failure, an optional no-op, or a genuine
/// type error" three-way check every native-arm dispatcher (`Iterate`,
/// `Map`, `Length`, `Keys`, `KeysUnsorted`, `ToEntries`, `ToNumber`) needs
/// once its own type-specific branches are exhausted -- decode-failure must
/// be checked *before* `optional` (#1620), not after, so `.a?` on an
/// undecodable string still raises rather than silently swallowing it.
/// `ToEntries` passes `optional: false` unconditionally rather than
/// threading it through -- see that call site's own comment for why.
///
/// `fallback` is only called for the genuine type-error case, so it can
/// build a builtin-specific `EvalError` (`cannot_iterate_with`,
/// `has_no_length`, ...) without paying for it on the two more common
/// exits above.
fn decode_failure_or<V: DocumentValue>(
    value: &V,
    optional: bool,
    fallback: impl FnOnce() -> GenericResult<V>,
) -> GenericResult<V> {
    if let Some(reason) = value.string_decode_error() {
        GenericResult::Error(EvalError::decode_failure(reason))
    } else if optional {
        GenericResult::None
    } else {
        fallback()
    }
}

/// The jq `type` name for `value` at `cursor`, resolving an explicit YAML
/// tag first (e.g. `!!str 1` is `"string"`, not `"number"` — issue #747)
/// and falling back to [`DocumentValue::type_name`] otherwise. Mirrors
/// [`tagged_scalar_to_owned`]'s tag lookup, but only needs
/// [`crate::yaml::ResolvedScalar::type_name`], not the resolved value
/// itself.
fn tagged_type_name<V: DocumentValue>(value: &V, cursor: Option<V::Cursor>) -> &'static str {
    cursor
        .and_then(|c| {
            // Not `c.explicit_tag().and_then(...)` chained outside this
            // closure: `c` is a local `V::Cursor` moved in by `and_then`,
            // so a `&str` returned through `&c` (an elided-lifetime `&self`
            // method) can't outlive this closure even though the
            // underlying text can — resolve it to an owned `ResolvedScalar`
            // before returning, same as `to_owned_cursor`'s scalar arm.
            let tag = c.explicit_tag()?;
            let text = value.as_str()?;
            crate::yaml::resolve_tagged(&text, tag)
        })
        .map_or_else(|| value.type_name(), crate::yaml::ResolvedScalar::type_name)
}

/// Which YAML anchor/alias syntax a node carried in the source document
/// (issue #763, ADR-0017's mechanism 2).
///
/// A node is at most one of the two: [`DocumentCursor::anchor`] returns
/// `None` for an alias node, and an alias node cannot itself declare an
/// anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorMark {
    /// `&name` — this node declares an anchor. The name is stored without
    /// the leading `&`.
    Declares(String),
    /// `*name` — this node is an alias referring to anchor `name`, stored
    /// without the leading `*`. A node marked this way renders as `*name`
    /// and its own value is not written at all.
    Aliases(String),
}

/// Per-node presentation metadata: everything about *how* a node was
/// written that its `OwnedValue` doesn't record.
///
/// Grouped into a struct rather than kept as loose tuple fields on
/// [`CommentTree`] so the remaining slot this side-channel still owes —
/// the explicit source tag the DOM path drops (#1132/#747) — can be added
/// without a third pass over every construction site.
#[derive(Debug, Clone, Default)]
pub struct NodeMeta {
    /// This node's trailing same-line comment, `#` and all (issue #710),
    /// or `None` if it has none.
    pub comment: Option<String>,
    /// This node's YAML style (issue #739): `""` for block/plain, or
    /// `"flow"`/`"double"`/`"single"`/`"literal"`/`"folded"` per
    /// [`DocumentCursor::style`].
    pub style: &'static str,
    /// This node's `&anchor`/`*alias` syntax (issue #763), or `None` if it
    /// carried neither.
    pub anchor: Option<AnchorMark>,
}

impl NodeMeta {
    /// Metadata-free: no comment, no style, no anchor. `const` so the
    /// module's empty-tree `static` can be built from it.
    pub const fn empty() -> Self {
        Self {
            comment: None,
            style: "",
            anchor: None,
        }
    }

    /// The comment/style pair read straight off a live cursor, with no
    /// anchor mark — the shape every caller predating #763 built.
    pub fn from_comment_and_style(comment: Option<String>, style: &'static str) -> Self {
        Self {
            comment,
            style,
            anchor: None,
        }
    }
}

/// A shape-parallel tree of per-node presentation metadata.
///
/// Trailing same-line comments (issue #710), YAML style (issue #739;
/// `""` for block/plain, `"flow"`/`"double"`/`"single"`/`"literal"`/
/// `"folded"` per [`DocumentCursor::style`]), and `&anchor`/`*alias`
/// syntax (issue #763) — built alongside an `OwnedValue` by
/// [`to_owned_with_comments`], one [`NodeMeta`] per node.
///
/// ADR-0017 originally specified that anchor/alias identity must *not*
/// ride this tree, on the grounds that a shape-parallel side-tree "has no
/// notion of cross-tree identity". That holds for deciding whether a mark
/// is still *valid* after a write — which is why that stays separate logic
/// in `yq_runner.rs` — but not for *emitting* one: what the writer needs is
/// purely per-node ("write `&x` here", "write `*x` there"), the same shape
/// as comments and style. Carrying it here reuses the threading this tree
/// already has through capture, reconciliation, emission and `--split-exp`;
/// see that ADR's amendment for the full argument.
///
/// Deliberately *not* `--inplace`, which never builds one of these at all:
/// it evaluates through `evaluate_input` with [`CommentTree::empty`], so it
/// drops comments, style and anchors alike (and skips #711's alias value
/// sync). That is issue #1349, a gap in that route's own wiring rather than
/// anything this tree can reach.
///
/// `OwnedValue` itself carries no metadata — extending its enum would ripple
/// through every match site in both the JSON and YAML evaluators for a
/// feature only the YAML write path needs. This tree is a separate,
/// additive structure consulted only by `emit_yaml_value` in
/// `yq_runner.rs`, the DOM writer used once a query's `GenericResult` has
/// been materialized to `OwnedValue` (`yq_runner.rs`'s
/// `evaluate_yaml_cursor`, the only place that calls
/// `to_owned_with_comments`). It is a distinct mechanism from
/// `YamlCursor::stream_yaml_value`/`stream_yaml_as_document` in
/// `light.rs`, which stream comments/style straight from a *live cursor*
/// and never go through `CommentTree` at all — `stream_owned_value_yaml` in
/// `stream.rs` streams plain `OwnedValue` (no cursor, no metadata) and
/// isn't part of either mechanism.
/// A query that reshapes the tree (`map`, object/array construction, ...)
/// simply has no `to_owned_with_comments` call in its chain, so metadata is
/// dropped there exactly as they are today — out of scope for this issue,
/// tracked separately (see the issue's own scope note). A query that
/// rewrites specific paths (`=`, `|=`, `del()`, ...) instead gets a
/// reconciled tree from `evaluate_yaml_cursor`'s `reconcile_presentation`,
/// which pairs the pristine (pre-write) tree with the post-write value and
/// keeps metadata for every node whose value the write didn't touch.
#[derive(Debug, Clone)]
pub enum CommentTree {
    /// A scalar (or any node with no comment-bearing children).
    Leaf(NodeMeta),
    /// An array: this node's own metadata (e.g. the comment in
    /// `a: [1,2] # c`, which trails the whole array, not an element) plus
    /// one subtree per element in order.
    Array(NodeMeta, Vec<Self>),
    /// An object: this node's own metadata, one subtree per field (keyed
    /// the same as the parallel `OwnedValue::Object`), plus one key-scoped
    /// comment per field for a comment trailing the *key's* own line when
    /// its value is deferred to a following line (issue #765, e.g.
    /// `a: # comment\n  b: 1`) - distinct from the field's value subtree's
    /// own comment, which `.a | line_comment` etc. read instead. The `bool`
    /// alongside each comment is whether the deferred value materialized as
    /// nothing at all (a sibling key follows, or EOF) - see
    /// [`Self::key_comment_if_value_absent`].
    Object(
        NodeMeta,
        IndexMap<String, Self>,
        IndexMap<String, (String, bool)>,
    ),
}

impl CommentTree {
    /// The empty tree: no metadata at this node, and (for containers) no
    /// children — used where a caller has no cursor at all (metadata-less
    /// by construction, e.g. a computed value).
    pub const fn empty() -> Self {
        Self::Leaf(NodeMeta::empty())
    }

    /// This node's own presentation metadata.
    pub fn meta(&self) -> &NodeMeta {
        match self {
            Self::Leaf(m) | Self::Array(m, _) | Self::Object(m, _, _) => m,
        }
    }

    /// This node's own presentation metadata, mutably — used by the
    /// post-evaluation passes in `yq_runner.rs` that clear one slot
    /// (`-P`'s style strip, #763's soundness gate) without rebuilding the
    /// surrounding tree.
    pub fn meta_mut(&mut self) -> &mut NodeMeta {
        match self {
            Self::Leaf(m) | Self::Array(m, _) | Self::Object(m, _, _) => m,
        }
    }

    /// This node's own trailing comment, if any.
    pub fn own(&self) -> Option<&str> {
        self.meta().comment.as_deref()
    }

    /// This node's own YAML style (`""`, `"flow"`, `"double"`, `"single"`,
    /// `"literal"`, or `"folded"` — see [`DocumentCursor::style`]).
    pub fn style(&self) -> &'static str {
        self.meta().style
    }

    /// This node's own `&anchor`/`*alias` mark, if it carried one (#763).
    pub fn anchor_mark(&self) -> Option<&AnchorMark> {
        self.meta().anchor.as_ref()
    }

    /// The anchor name this node is an alias *reference* to (`*name`), or
    /// `None` if it declares an anchor or carries no mark at all. The
    /// writer treats such a node as rendering to `*name` instead of its
    /// own value, so this is the check that must come before any
    /// value-shape dispatch.
    pub fn alias_name(&self) -> Option<&str> {
        match self.meta().anchor.as_ref() {
            Some(AnchorMark::Aliases(name)) => Some(name),
            _ => None,
        }
    }

    /// The anchor name this node *declares* (`&name`), or `None`.
    pub fn declared_anchor(&self) -> Option<&str> {
        match self.meta().anchor.as_ref() {
            Some(AnchorMark::Declares(name)) => Some(name),
            _ => None,
        }
    }

    /// The subtree for array index `i`, or the empty tree if this isn't an
    /// `Array` or the index is out of range.
    pub fn at_index(&self, i: usize) -> &Self {
        match self {
            Self::Array(_, items) => items.get(i).unwrap_or(&EMPTY_COMMENT_TREE),
            _ => &EMPTY_COMMENT_TREE,
        }
    }

    /// The subtree for object key `key`, or the empty tree if this isn't an
    /// `Object` or has no such key.
    pub fn field(&self, key: &str) -> &Self {
        match self {
            Self::Object(_, fields, _) => fields.get(key).unwrap_or(&EMPTY_COMMENT_TREE),
            _ => &EMPTY_COMMENT_TREE,
        }
    }

    /// A comment trailing object field `key`'s own *key* line, when its
    /// value is deferred to a following line (issue #765) - or `None` if
    /// this isn't an `Object`, has no such key, or the key has no such
    /// comment. Distinct from `field(key).own()`, which is the value's own
    /// trailing comment (issue #710).
    pub fn key_comment(&self, key: &str) -> Option<&str> {
        match self {
            Self::Object(_, _, key_comments) => key_comments.get(key).map(|(c, _)| c.as_str()),
            _ => None,
        }
    }

    /// The same key-scoped comment as [`Self::key_comment`], but only when
    /// the deferred value itself materialized as nothing at all (issue
    /// #765) - a sibling key follows at the same or lower indent, or EOF.
    ///
    /// `None` both when there's no key comment at all, and when there is
    /// one but the deferred value has real content of its own (a
    /// container - use `key_comment` plus the value's own rendering there
    /// instead - or a folded scalar continuation like `a: # c\n  null`,
    /// which real `yq` places the comment after rather than right after
    /// the key, a different, unhandled case).
    pub fn key_comment_if_value_absent(&self, key: &str) -> Option<&str> {
        match self {
            Self::Object(_, _, key_comments) => key_comments
                .get(key)
                .filter(|(_, absent)| *absent)
                .map(|(c, _)| c.as_str()),
            _ => None,
        }
    }
}

/// The empty tree, as a genuine `'static` place (not a local `const`, which
/// can't be borrowed as `'static` from inside a generic method call
/// argument like `Option::unwrap_or`) — used by [`CommentTree::at_index`]/
/// [`CommentTree::field`] wherever there's no comment data to return.
static EMPTY_COMMENT_TREE: CommentTree = CommentTree::Leaf(NodeMeta::empty());

/// Convert a `DocumentValue` to an `OwnedValue` alongside a parallel [`CommentTree`].
///
/// Uses a live cursor to read each node's trailing comment (issue #710).
/// See [`to_owned`] for the value-only conversion this mirrors and
/// delegates scalar handling to. Panics past [`MAX_NESTING_DEPTH`] levels of
/// nesting (#998), same as `to_owned`.
pub fn to_owned_with_comments<V: DocumentValue>(
    value: &V,
    cursor: Option<&V::Cursor>,
) -> Result<(OwnedValue, CommentTree), EvalError> {
    to_owned_with_comments_at_depth(value, cursor, 0)
}

fn to_owned_with_comments_at_depth<V: DocumentValue>(
    value: &V,
    cursor: Option<&V::Cursor>,
    depth: usize,
) -> Result<(OwnedValue, CommentTree), EvalError> {
    assert_nesting_depth(depth);
    // The raw (`#`-prefixed) form, not the stripped `line_comment` builtin
    // getter: the write path re-emits this verbatim after one space.
    let own_comment = cursor.and_then(DocumentCursor::line_comment_raw);
    let own_style = cursor.map_or("", DocumentCursor::style);
    // `&anchor`/`*alias` (#763). Checked alias-first: `anchor()` already
    // returns `None` on an alias node, so the order can't actually
    // mis-classify, but it states the precedence the writer relies on --
    // an aliased node renders as `*name` and never writes its own value.
    let own_anchor = cursor.and_then(|c| {
        DocumentCursor::alias(c)
            .map(|name| AnchorMark::Aliases(name.to_string()))
            .or_else(|| {
                DocumentCursor::anchor(c).map(|name| AnchorMark::Declares(name.to_string()))
            })
    });
    let own_meta = NodeMeta {
        comment: own_comment,
        style: own_style,
        anchor: own_anchor,
    };
    if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut comment_map = IndexMap::new();
        let mut key_comment_map = IndexMap::new();
        let mut guard = DisplayKeyGuard::default();
        let mut f = fields;
        while let Some((field, rest)) = f.uncons() {
            // Same key handling as `to_owned_at_depth`, same reasons
            // (#1247/#1642): preserve a decode-failure key via its raw
            // source span; still raise on a key the format's grammar never
            // allowed at all (#1194) -- this arm used to silently drop such
            // a field instead of raising, the same swallow #1194 names.
            let Some(key) = resolve_display_key(&field.key, &map, &mut guard)? else {
                return Err(f.malformed_member_error());
            };
            let (v, c) = to_owned_with_comments_at_depth(
                &field.value,
                Some(&field.value_cursor),
                depth + 1,
            )?;
            map.insert(key.clone(), v);
            comment_map.insert(key.clone(), c);
            // A comment trailing the key's own line, when the value is
            // deferred to a following line (issue #765) - distinct from
            // `c`'s own comment above, which belongs to the value.
            // Recorded alongside whether the deferred value
            // materialized as nothing at all (a sibling key follows, or
            // EOF): a folded scalar continuation that merely *reads* as
            // null (`a: # c\n  null`) is a different, unhandled case
            // where real yq places the comment after the folded value
            // instead of right after the key, which `v`/`is_null()`
            // alone can't tell apart from true absence - both collapse
            // through the same semantic check `to_owned` uses - so this
            // also checks the raw text is empty, not just null-ish.
            if let Some(kc) = field.key_cursor.line_comment_raw() {
                let value_absent =
                    field.value.is_null() && field.value.as_str().map_or(true, |s| s.is_empty());
                key_comment_map.insert(key, (kc, value_absent));
            }
            f = rest;
        }
        if f.ends_unpaired() {
            return Err(f.malformed_member_error());
        }
        Ok((
            OwnedValue::Object(map),
            CommentTree::Object(own_meta, comment_map, key_comment_map),
        ))
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut comment_items = Vec::new();
        // Iterate by cursor (`uncons_cursor`, not `uncons`, which yields
        // values only) so each element's own comment is reachable.
        let mut elems = elements;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            let elem_value = elem_cursor.value();
            let (v, c) =
                to_owned_with_comments_at_depth(&elem_value, Some(&elem_cursor), depth + 1)?;
            items.push(v);
            comment_items.push(c);
            elems = rest;
        }
        Ok((
            OwnedValue::Array(items),
            CommentTree::Array(own_meta, comment_items),
        ))
    } else {
        Ok((
            to_owned_at_depth(value, depth)?,
            CommentTree::Leaf(own_meta),
        ))
    }
}

/// Materialize a key/slice-bound candidate just enough to classify it.
///
/// Mirrors `eval::to_owned_key_shape` (#626/#670): `index_one_generic`'s
/// Array/Object rejection and `slice::bound`'s non-numeric rejection never
/// inspect a candidate's *contents*, only its shape, so a full recursive
/// `to_owned` of a large navigated container is pure waste when it can only
/// ever be rejected on type (#669).
fn to_owned_key_shape<V: DocumentValue>(value: &V) -> Result<OwnedValue, EvalError> {
    if value.is_array() {
        Ok(OwnedValue::Array(Vec::new()))
    } else if value.is_object() {
        Ok(OwnedValue::Object(IndexMap::new()))
    } else {
        to_owned(value)
    }
}

/// Cursor-carrying sibling of [`to_owned_key_shape`] (#903 review): a
/// computed-index key or slice bound reached via `OneCursor`/`ManyCursor`
/// still needs its scalar resolved through `to_owned_cursor` rather than the
/// bare `to_owned`, or an explicit tag on the key/bound expression itself
/// (`.a[.k]` where `.k` is `!!str 1`) is silently ignored.
fn to_owned_key_shape_cursor<C: DocumentCursor>(cursor: &C) -> Result<OwnedValue, EvalError> {
    let value = cursor.value();
    if value.is_array() {
        Ok(OwnedValue::Array(Vec::new()))
    } else if value.is_object() {
        Ok(OwnedValue::Object(IndexMap::new()))
    } else {
        to_owned_cursor(cursor)
    }
}

/// Materialize a `GenericResult::LazyKeys` fallback: decode every key exactly
/// as eager `Builtin::Keys`/`Builtin::KeysUnsorted` did before either stayed
/// lazy, sorting first iff `sorted` (#683).
///
/// Every consumer other than the native fast paths (`length` always; `.[]`,
/// `.[n]`, `first`, `last` only when `!sorted` — see the `Pipe` dispatch
/// below) goes through here; this is the escape hatch #140 anticipated for
/// everything else (`map`, `select`, comparisons, ...).
fn materialize_lazy_keys<V: DocumentValue>(
    fields: &V::Fields,
    sorted: bool,
    collapse: bool,
) -> Result<OwnedValue, EvalError> {
    let mut keys = effective_keys(fields, collapse)?;
    if sorted {
        keys.sort();
    }
    Ok(OwnedValue::Array(
        keys.into_iter().map(OwnedValue::String).collect(),
    ))
}

/// An object's key cursors in document order, with a repeated key dropped
/// after its first occurrence when the mode collapses (#1514), refusing an
/// object whose members the format's grammar never allowed (#1194, #1629).
///
/// Materializes what [`DistinctKeyCursors`] streams. `.[]` over a key array
/// has to hand back every cursor at once, but the dedup still happens during
/// the single walk that collects them -- where before, the caller ran a
/// whole separate `document::census` and then walked again to build this.
///
/// The #1194 check rides along in that same walk, the same way
/// [`effective_keys`] already checks during its own single
/// [`DistinctKeyCursors`] walk -- one pass, not a second one bolted on top,
/// which on a wide object would double the walk this arm already pays for
/// (the exact mistake PR #1768's own perf-guard catch was about, earlier
/// this same session -- a different check, `string_decode_error`, but the
/// identical shape of mistake: a validation pass added ahead of an
/// already-cheap path instead of folded into it). A malformed `,`/`:`
/// delimiter (#1677) rides the same single walk too, via
/// [`DistinctKeyCursors::delimiter_fault`] -- checked only once the
/// iterator is exhausted, per its own "ask only at the end" contract,
/// alongside [`DistinctKeyCursors::ended_unpaired`]'s identical #1194
/// unpaired-trailing-child check.
fn distinct_key_cursors_checked<V: DocumentValue>(
    fields: &V::Fields,
    collapse: bool,
) -> Result<Vec<V::Cursor>, EvalError> {
    let mut out = Vec::new();
    walk_distinct_keys_checked::<V>(fields, collapse, |cursor| out.push(cursor))?;
    Ok(out)
}

/// [`distinct_key_cursors_checked`]'s check-only sibling, for a caller that
/// needs to know *whether* an object raises without ever reading a single
/// cursor back -- `try_single_generic`'s `LazyKeys` arm (#1936) hands the
/// original, still-lazy value back unchanged on success, so collecting a
/// `Vec<V::Cursor>` there would be a pure O(n)-space cost for an answer
/// that's thrown away every time (review already caught this exact
/// `distinct_key_cursors_checked`-for-a-yes/no-answer mistake once, on the
/// first version of this fix). `on_cursor` is a no-op closure here, which
/// monomorphizes away to nothing -- no `Vec`, no allocation.
fn keys_are_well_formed<V: DocumentValue>(
    fields: &V::Fields,
    collapse: bool,
) -> Result<(), EvalError> {
    walk_distinct_keys_checked::<V>(fields, collapse, |_cursor| {})
}

/// Shared walk behind [`distinct_key_cursors_checked`] and
/// [`keys_are_well_formed`]: both need the identical `DistinctKeyCursors`
/// walk and #1194/#1677 exhaustion check, differing only in whether the
/// caller wants each cursor or just the pass/fail verdict -- `on_cursor`
/// is that difference, not a new allocation (a `Vec::push` for the former,
/// a no-op for the latter). Pulled out so the check itself has exactly one
/// definition instead of two copies that could silently drift apart (the
/// codebase's own precedent for why this matters: `Builtin::Last`'s inline
/// walk below once diverged from both of these by omitting the
/// `delimiter_fault()` half of the check -- #1956 fixed that arm, plus two
/// more sites found the same way, by adding
/// [`DistinctKeyCursors::is_malformed`] so a caller can no longer check one
/// half of the pair and forget the other).
fn walk_distinct_keys_checked<V: DocumentValue>(
    fields: &V::Fields,
    collapse: bool,
    mut on_cursor: impl FnMut(V::Cursor),
) -> Result<(), EvalError> {
    let mut cursors = DistinctKeyCursors::new(fields, collapse);
    for (key, cursor) in cursors.by_ref() {
        if key_is_malformed(&key) {
            return Err(fields.malformed_member_error());
        }
        on_cursor(cursor);
    }
    if cursors.is_malformed() {
        return Err(fields.malformed_member_error());
    }
    // #2261: trailing stray comma after a real last key (`{"a":1,}`) --
    // `cursors` already retained the last key cursor this walk saw, so this
    // is one more O(1) `next_sibling()` hop, not a further walk. Covers
    // both callers: `distinct_key_cursors_checked` (`keys_unsorted[]`, a
    // negative `keys_unsorted[n]`) and `keys_are_well_formed`
    // (`try_single_generic`'s `LazyKeys` probe).
    if !cursors.trailing_gap_ok(b'}') {
        return Err(fields.malformed_member_error());
    }
    Ok(())
}

/// Materialize a `GenericResult::LazyIndexRange` fallback: build the
/// `[0, 1, ..., len-1]` array exactly as eager array `keys`/`keys_unsorted`
/// did before it stayed lazy (#684).
fn materialize_lazy_index_range(len: usize) -> OwnedValue {
    OwnedValue::Array((0..len).map(|i| OwnedValue::Int(i as i64)).collect())
}

/// Unwrap any number of `(...)`-parens to reach the underlying expression.
///
/// The `Pipe` dispatch below pattern-matches the *literal* AST shape of the
/// next stage to decide whether a `LazyKeys` fast path applies —
/// without this, `keys_unsorted | (length)` would miss the fast path that
/// `keys_unsorted | length` hits, even though parens are semantically
/// transparent everywhere else in this evaluator (see `Expr::Paren`'s own
/// arm above).
fn unwrap_paren(mut expr: &Expr) -> &Expr {
    while let Expr::Paren(inner) = expr {
        expr = inner;
    }
    expr
}

/// One pending element of a `LazySeq`: still a live pointer into the source
/// document, or a value an earlier `map` stage computed that no longer
/// corresponds to one node in the source.
///
/// Must be `pub`, like `LazySeq` itself: it's the `Item` of `LazySeq`'s
/// `Iterator` impl below, and `Iterator` is a standard-library trait always
/// in scope, so anyone who can name the (necessarily `pub`) `LazySeq` type
/// can call `.next()` on it and observe this type.
#[derive(Clone)]
pub enum LazyElem<V: DocumentValue> {
    Cursor(V::Cursor),
    Owned(OwnedValue),
}

// Hand-written rather than derived: deriving would require `V::Cursor: Debug`
// generically, which `DocumentCursor` doesn't guarantee (unlike `Clone`, which
// it does via a supertrait bound). Kept opaque on purpose, not just to satisfy
// the compiler -- printing a cursor's full backing state is a real footgun in
// this codebase (a YAML cursor's `Debug` walks the whole shared index,
// including the O1/O2 sequential-cursor `Cell` cache).
impl<V: DocumentValue> core::fmt::Debug for LazyElem<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cursor(_) => f.write_str("LazyElem::Cursor(..)"),
            Self::Owned(o) => f.debug_tuple("LazyElem::Owned").field(o).finish(),
        }
    }
}

/// The starting point of a `LazySeq` chain — forward-only by construction
/// (consumed cons cells are simply gone, no rewind method exists on any
/// variant).
#[derive(Clone)]
enum LazySource<V: DocumentValue> {
    /// Bare `arr | map(f)` (#725).
    Elements(V::Elements),
    /// Bare `obj | map(f)` (#725).
    Values(V::Fields),
    /// `keys_unsorted | map(f)` (#724).
    ///
    /// Carries the mode's duplicate-key rule into the pull itself (#1514)
    /// instead of probing the whole object before the first element is
    /// asked for -- see [`DistinctKeyCursors`] for why that is sound on a
    /// forward-only consumer.
    Keys(DistinctKeyCursors<V::Fields>),
    /// Array `keys_unsorted | map(f)` (#724) — synthetic `[0, 1, ..., len-1]`,
    /// no cursor to point at.
    IndexRange { next: usize, len: usize },
    /// An explicit, already-ordered list of element cursors -- the general
    /// "this array's elements are these document nodes, in this order"
    /// source. Unlike every variant above it does not walk a live cons-list,
    /// so the order and membership are whatever the producer chose.
    ///
    /// Two producers today:
    ///
    /// - `Values`'s duplicate-key fallback (#1398): `obj | map(f)` is `[.[] |
    ///   f]`, and `.[]` collapses a repeated key to its first position but
    ///   last-seen value in both modes (see `Expr::Iterate`'s identical
    ///   rule). That requires seeing every occurrence before any value can
    ///   be emitted, so it can't stay a `Fields` cons-list walk. Constructed
    ///   only when `document::collapsed_fields` actually finds a repeat, so
    ///   the ordinary duplicate-free `Values` path above is unaffected.
    /// - The reordering/selecting builtins (#1687): `sort`, `sort_by`,
    ///   `unique`, `unique_by` and `reverse` all answer a *permutation or
    ///   subset of their input's own elements*, so their result is exactly
    ///   this -- a `Vec<V::Cursor>` in the new order. Returning it as a
    ///   `LazySeq` rather than an `OwnedValue::Array` is what keeps a
    ///   duplicate mapping key *inside* a moved element alive, since
    ///   `OwnedValue::Object` is `IndexMap`-backed and cannot represent one.
    ///   `Builtin::Map`'s own long-standing `LazySeq` return (#724/#725) is
    ///   the working precedent: `map(.)` is the one filter in this family
    ///   that already preserved duplicates, for exactly this reason.
    Cursors {
        cursors: Vec<V::Cursor>,
        next: usize,
    },
}

// See `LazyElem`'s `Debug` impl above for why this is hand-written, not derived.
impl<V: DocumentValue> core::fmt::Debug for LazySource<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Elements(_) => f.write_str("LazySource::Elements(..)"),
            Self::Values(_) => f.write_str("LazySource::Values(..)"),
            Self::Keys(_) => f.write_str("LazySource::Keys(..)"),
            Self::IndexRange { next, len } => f
                .debug_struct("LazySource::IndexRange")
                .field("next", next)
                .field("len", len)
                .finish(),
            Self::Cursors { next, cursors } => f
                .debug_struct("LazySource::Cursors")
                .field("next", next)
                .field("len", &cursors.len())
                .finish(),
        }
    }
}

impl<V: DocumentValue> LazySource<V> {
    /// A `keys_unsorted` source that honours the mode's duplicate-key
    /// rule as it pulls (#1514).
    fn keys(fields: V::Fields, collapse: bool) -> Self {
        Self::Keys(DistinctKeyCursors::new(&fields, collapse))
    }

    /// A [`Self::Cursors`] source positioned at its first element.
    ///
    /// The `next: 0` is the only thing every producer would otherwise have
    /// to remember to write, so it lives here rather than at each call site.
    fn cursors(cursors: Vec<V::Cursor>) -> Self {
        Self::Cursors { cursors, next: 0 }
    }
    /// Pull one element forward, storing "the rest" back into `self`. Once a
    /// variant's underlying cons-list is empty (or `next == len`), every
    /// subsequent call returns `Ok(None)` forever -- except `Keys`, which can
    /// still raise on that same exhaustion (#1956: a `keys_unsorted | map(f)`
    /// chain sourced its elements straight from `DistinctKeyCursors::next()`
    /// without ever asking `is_malformed()`, the exact check every other
    /// `keys_unsorted` consumer already made -- confirmed live, `keys_unsorted
    /// | map(.)` over a document with a missing `,`/`:` silently succeeded
    /// while every sibling spelling correctly raised).
    fn advance(&mut self) -> Result<Option<LazyElem<V>>, EvalError> {
        Ok(match self {
            Self::Elements(elements) => {
                let Some((cursor, rest)) = elements.uncons_cursor() else {
                    return Ok(None);
                };
                *elements = rest;
                Some(LazyElem::Cursor(cursor))
            }
            Self::Values(fields) => {
                let Some((field, rest)) = fields.uncons() else {
                    return Ok(None);
                };
                *fields = rest;
                Some(LazyElem::Cursor(field.value_cursor))
            }
            Self::Keys(keys) => match keys.next() {
                Some((key, cursor)) => {
                    if key_is_malformed(&key) {
                        return Err(keys.malformed_member_error());
                    }
                    Some(LazyElem::Cursor(cursor))
                }
                None if keys.is_malformed() => return Err(keys.malformed_member_error()),
                None => None,
            },
            Self::IndexRange { next, len } => {
                if *next >= *len {
                    return Ok(None);
                }
                let i = *next;
                *next += 1;
                Some(LazyElem::Owned(OwnedValue::Int(i as i64)))
            }
            Self::Cursors { cursors, next } => {
                let Some(cursor) = cursors.get(*next).copied() else {
                    return Ok(None);
                };
                *next += 1;
                Some(LazyElem::Cursor(cursor))
            }
        })
    }
}

/// One deferred `map(f)` stage. `select(g)` composes as a plain pipe stage
/// evaluating `g` and testing truthiness the same way `Builtin::Select`
/// already does elsewhere — this design adds no dedicated `select` variant.
#[derive(Debug, Clone)]
struct Instruction {
    f: Rc<Expr>,
    tag: EvalTag,
}

/// A composed, not-yet-materialized `map` chain (#724, #725).
///
/// See `docs/plan/jq-lazy-map-select.md`. `source` never rewinds.
/// `instructions` grows by one `Rc`-shared entry per composed stage, so an
/// arbitrary-length chain (`map(f) | map(g) | map(h)`) is one value, not one
/// type per depth. `pending` buffers only the current source element's own
/// fan-out (0..N outputs from `,`/`empty` inside one stage), never an
/// earlier or later element — in reverse push order so `Vec::pop` yields
/// them in original order.
///
/// Must be `pub`: it appears inside the public `GenericResult::LazySeq`
/// variant, and the CLI binary crate (`jq_runner.rs`/`yq_runner.rs`) depends
/// on this library crate, so cross-crate reachability is a real constraint.
/// Fields stay private; only `materialize_atomic` is `pub`.
#[derive(Clone)]
pub struct LazySeq<V: DocumentValue> {
    source: LazySource<V>,
    instructions: Rc<Vec<Instruction>>,
    pending: Vec<LazyElem<V>>,
}

// See `LazyElem`'s `Debug` impl above for why this is hand-written, not
// derived -- and why it stays opaque rather than exposing `pending`'s
// buffered elements.
impl<V: DocumentValue> core::fmt::Debug for LazySeq<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LazySeq")
            .field("source", &self.source)
            .field("instructions_len", &self.instructions.len())
            .field("pending_len", &self.pending.len())
            .finish()
    }
}

impl<V: DocumentValue> LazySeq<V> {
    fn new(source: LazySource<V>) -> Self {
        Self {
            source,
            instructions: Rc::new(Vec::new()),
            pending: Vec::new(),
        }
    }

    /// Push one more `map(f)` stage onto this chain, in place. `f` is cloned
    /// once per *stage* (not per element) into a fresh `Rc` — `Builtin::Map`
    /// only ever hands us a borrowed `&Expr` (from `unwrap_paren` in the
    /// `Pipe` fold, or `&Builtin` in `eval_builtin`), so there is nothing to
    /// move out of. Split out from [`Self::push_map`] (#789 code review
    /// follow-up) so `fold_lazy_seq_stage`'s `Map` arm can mutate an
    /// already-boxed `LazySeq` through `&mut` instead of moving it out of
    /// its `Box` (freeing the allocation) only to immediately `Box::new` a
    /// same-size replacement.
    fn push_map_in_place(&mut self, f: &Expr, tag: EvalTag) {
        Rc::make_mut(&mut self.instructions).push(Instruction {
            f: Rc::new(f.clone()),
            tag,
        });
    }

    /// Builder-style wrapper around [`Self::push_map_in_place`], for the
    /// fresh-construction call sites that chain straight off `LazySeq::new`.
    fn push_map(mut self, f: &Expr, tag: EvalTag) -> Self {
        self.push_map_in_place(f, tag);
        self
    }

    /// An array whose elements are exactly `cursors`, with no `map` stage on
    /// top -- the lossless answer for a builtin that reorders or selects its
    /// input's own elements without computing new ones (#1687).
    ///
    /// The instruction chain is deliberately empty: `fold_one` then folds
    /// each element through zero stages and hands the cursor straight back,
    /// so `stream_json`/`stream_yaml` render every element from its own live
    /// document position. That is the whole point -- it is what preserves a
    /// duplicate mapping key inside a moved element, which an
    /// `OwnedValue::Array` of `IndexMap`-backed objects cannot.
    fn from_cursors(cursors: Vec<V::Cursor>) -> Self {
        Self::new(LazySource::cursors(cursors))
    }

    /// Run one `Instruction` against one pending item, re-dispatching to
    /// whichever `EvalSemantics` the stage was pushed with. `LazyElem::Cursor`
    /// stays inside the generic cursor evaluator — the actual win;
    /// `LazyElem::Owned` bridges through `eval_on_owned`, only ever asked to
    /// reindex one already-small computed/synthetic scalar (a map-produced
    /// value or an array-`keys_unsorted` index), never the whole document.
    fn eval_one(instr: &Instruction, elem: LazyElem<V>) -> GenericResult<V> {
        match (elem, instr.tag) {
            (LazyElem::Cursor(c), EvalTag::Jq) => {
                eval_single::<JqSemantics, V>(&instr.f, c.value(), false, Some(c))
            }
            (LazyElem::Cursor(c), EvalTag::Yq) => {
                eval_single::<YqSemantics, V>(&instr.f, c.value(), false, Some(c))
            }
            (LazyElem::Owned(o), EvalTag::Jq) => {
                eval_on_owned::<JqSemantics, V>(&instr.f, o, false)
            }
            (LazyElem::Owned(o), EvalTag::Yq) => {
                eval_on_owned::<YqSemantics, V>(&instr.f, o, false)
            }
        }
    }

    /// Fold one source element through every instruction in order, fanning
    /// out via `,`/dropping via `empty` at each stage. Atomic at *this
    /// element's own* granularity: any stage's error/break for this one
    /// element aborts the whole element, mirroring `eval::map_over`'s
    /// per-array-construction atomicity.
    fn fold_one(&self, elem: LazyElem<V>) -> Result<Vec<LazyElem<V>>, Control> {
        let mut items = vec![elem];
        for instr in self.instructions.iter() {
            let mut next_items = vec_with_capacity(items.len());
            for item in items {
                next_items.extend(into_lazy_items(Self::eval_one(instr, item))?);
            }
            items = next_items;
        }
        Ok(items)
    }

    /// Pull every remaining element to completion *without* converting the
    /// still-live cursors among them, discarding everything collected so far
    /// on the first error/break.
    ///
    /// The atomicity primitive both public consumers share (#757).
    /// `materialize_atomic` below is this plus a `to_owned_cursor` per item;
    /// `GenericResult::stream_json`/`stream_yaml`'s `LazySeq` arms skip that
    /// conversion entirely and render each `LazyElem::Cursor` straight from
    /// the source document, which is what lets CLI output keep duplicate
    /// mapping keys, comments, anchors and flow style through `map` the way
    /// `.[]` already does.
    ///
    /// Draining *before* writing a byte is also what preserves `map`'s
    /// all-or-nothing output contract for a streaming consumer: the whole
    /// chain's success is known up front, so a failing element can never
    /// leave a truncated prefix in the writer. A `Vec<LazyElem<V>>` is a
    /// cheap thing to hold for that — `V::Cursor` is `Copy` and pointer-sized
    /// — unlike the `OwnedValue` tree `materialize_atomic` builds, which for
    /// `map(.)` is a full deep copy of every element it touches.
    pub fn drain_atomic(self) -> Result<Vec<LazyElem<V>>, Control> {
        let mut out = Vec::new();
        for item in self {
            out.push(item?);
        }
        Ok(out)
    }

    /// Pull every remaining element to completion, discarding the whole
    /// in-progress array on the first error/break (mirrors `map_over`'s
    /// atomicity — real jq's array construction is all-or-nothing:
    /// `[1,2,"x"]|map(.+1)` prints nothing to stdout, only the stderr
    /// diagnostic).
    pub fn materialize_atomic(self) -> Result<OwnedValue, Control> {
        let items = self.drain_atomic()?;
        let mut out = vec_with_capacity(items.len());
        for item in &items {
            out.push(lazy_elem_to_owned(item).map_err(Control::Error)?);
        }
        Ok(OwnedValue::Array(out))
    }
}

/// The drained elements of a `LazySeq` as a cursor slice, or `None` when this
/// sequence cannot render straight from the source document (#757).
///
/// `None` on two counts, both real rather than defensive:
///
/// - The cursor type has no sequence writer (`JsonCursor` takes
///   `DocumentCursor`'s defaults — `jq`'s CLI has no M2 streaming path to
///   reach this from anyway).
/// - Some element is a computed value rather than a live cursor. The
///   clearest case is `keys_unsorted | map(f)` over an *array*, which sources
///   from `LazySource::IndexRange`: every element is a synthetic
///   `LazyElem::Owned(Int)` with no node in the document to point at.
///
/// Both fall back to materializing an `OwnedValue::Array`, which is what this
/// arm did unconditionally before #757.
fn sequence_streamable_cursors<V: DocumentValue>(items: &[LazyElem<V>]) -> Option<Vec<V::Cursor>> {
    if !V::Cursor::supports_sequence_streaming() {
        return None;
    }
    items
        .iter()
        .map(|elem| match elem {
            LazyElem::Cursor(c) => Some(*c),
            LazyElem::Owned(_) => None,
        })
        .collect()
}

/// Convert one drained `LazyElem` to an `OwnedValue`.
///
/// The single conversion point shared by `materialize_atomic` and the
/// `LazySeq` streaming arms' non-cursor fallback (#757), so the two can never
/// disagree about how a drained item becomes a value.
fn lazy_elem_to_owned<V: DocumentValue>(elem: &LazyElem<V>) -> Result<OwnedValue, EvalError> {
    match elem {
        LazyElem::Cursor(c) => to_owned_cursor(c),
        LazyElem::Owned(o) => Ok(o.clone()),
    }
}

impl<V: DocumentValue> Iterator for LazySeq<V> {
    type Item = Result<LazyElem<V>, Control>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.pending.pop() {
                return Some(Ok(item));
            }
            let elem = match self.source.advance() {
                Ok(Some(elem)) => elem,
                Ok(None) => return None,
                Err(e) => return Some(Err(Control::Error(e))),
            };
            match self.fold_one(elem) {
                Ok(mut items) => {
                    items.reverse();
                    self.pending = items;
                }
                Err(control) => return Some(Err(control)),
            }
        }
    }
}

/// Normalize any `GenericResult<V>` shape a `map` stage can produce into the
/// `LazyElem` items it contributes to the chain — the single point every
/// stage's output funnels through.
fn into_lazy_items<V: DocumentValue>(
    result: GenericResult<V>,
) -> Result<Vec<LazyElem<V>>, Control> {
    match result {
        // `One`/`Many` (this arm and the one below) are exhaustiveness only,
        // not reachable through this function's one call site
        // (`LazySeq::fold_one`): every element it hands `eval_single` here
        // carries a concrete `Some(cursor)` (`c.value()`/`Some(c)`), and
        // every native arm that can construct a bare `One`/`Many`
        // (`Identity`, `Builtin::Select` and its cursor-preserving siblings)
        // forwards a `Some` cursor into `OneCursor`/`ManyCursor` instead of
        // `One`/`Many` -- `eval_on_owned` (the other `LazyElem` kind's path,
        // `LazyElem::Owned`) never returns `One`/`Many` either, per its own
        // `unreachable!` arms in `eval_on_many_owned` below.
        GenericResult::One(v) => Ok(vec![LazyElem::Owned(to_owned(&v).map_err(Control::Error)?)]),
        // Stays lazy: a `map(.foo)`-style navigational sub-expr keeps
        // composing without forcing materialization.
        GenericResult::OneCursor(c) => Ok(vec![LazyElem::Cursor(c)]),
        GenericResult::Many(vs) => vs
            .iter()
            .map(|v| Ok(LazyElem::Owned(to_owned(v).map_err(Control::Error)?)))
            .collect(),
        GenericResult::ManyCursor(cs) => Ok(cs.into_iter().map(LazyElem::Cursor).collect()),
        GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        } => Ok(vec![LazyElem::Owned(
            materialize_lazy_keys::<V>(&fields, sorted, collapse).map_err(Control::Error)?,
        )]),
        GenericResult::LazyIndexRange(len) => {
            Ok(vec![LazyElem::Owned(materialize_lazy_index_range(len))])
        }
        // Recursive laziness (`map(map(f))` where the *inner* `map` also
        // stays lazy) is an explicit non-goal — force it here, same
        // one-forward-pass cost `materialize_atomic` pays elsewhere.
        GenericResult::LazySeq(seq) => Ok(vec![LazyElem::Owned(seq.materialize_atomic()?)]),
        GenericResult::None => Ok(vec![]),
        GenericResult::Owned(o) => Ok(vec![LazyElem::Owned(o)]),
        GenericResult::ManyOwned(os) => Ok(os.into_iter().map(LazyElem::Owned).collect()),
        GenericResult::Error(e) => Err(Control::Error(e)),
        GenericResult::Break(label) => Err(Control::Break(label)),
        GenericResult::Halt(code) => Err(Control::Halt(code)),
        // Same atomicity as `map_over`: a `Partial`'s already-succeeded
        // prefix is discarded, not kept, when it's one stage's output inside
        // a larger atomic array construction.
        GenericResult::Partial(_, control) => Err(control),
    }
}

/// Convert a StandardJson value to an OwnedValue.
///
/// Panics past [`MAX_NESTING_DEPTH`] levels of nesting (#1017) -- a third,
/// independent copy of the cursor-to-`OwnedValue` conversion `to_owned`/
/// `to_owned_cursor` already guard in this same file (#998), the same gap
/// #998's own guards on the other two copies were added to close.
///
/// Named distinctly from the crate-wide, `DocumentValue`-generic [`to_owned`]
/// above (#965) rather than merged into it: this one is used only by this
/// module's own `eval_on_owned`/`eval_single` fallback arm on a value it
/// constructs internally (e.g. a `reduce`/`foreach` accumulator), and its
/// Number arm carries a real behavioral difference from `to_owned` that a
/// naive merge would lose -- NaN/Infinity-sentinel decoding (its callers
/// round-trip through `to_json_for_reindex`, which bakes those as a sentinel
/// number literal `to_owned`'s plain `number_literal()`/`as_f64()` path
/// doesn't decode). Before this rename it also happened to share a name with
/// an unrelated, now-deleted `yq_runner.rs` helper (#907) -- a real
/// naming-confusion risk for anyone grepping the codebase, even though the
/// two never collided at compile time (module-private on both sides).
///
/// Errors (#1192) rather than silently degrading when a string scalar (or an
/// object key) passes structural validation but fails to *decode* (invalid
/// UTF-8, an invalid escape, an invalid `\u` codepoint) -- this used to
/// return `OwnedValue::String("")` for such a value and silently drop the
/// whole field for such a key. `to_owned`/`to_owned_cursor` (this file) and
/// `cursor_to_owned` (`lazy.rs`) still have the older silent-degrade
/// behavior for this same failure; making those three fallible too needs a
/// much larger signature change (each is called from 100+ sites, many mid-
/// evaluation, not just at an output boundary -- the same blast-radius
/// tradeoff `MAX_NESTING_DEPTH`'s panic-not-`Result` design already made) --
/// tracked separately, not attempted here.
fn owned_from_standard_json<W: Clone + AsRef<[u64]>>(
    value: &crate::json::light::StandardJson<'_, W>,
) -> Result<OwnedValue, EvalError> {
    owned_from_standard_json_at_depth(value, 0)
}

fn owned_from_standard_json_at_depth<W: Clone + AsRef<[u64]>>(
    value: &crate::json::light::StandardJson<'_, W>,
    depth: usize,
) -> Result<OwnedValue, EvalError> {
    use crate::json::light::StandardJson;
    assert_nesting_depth(depth);
    Ok(match value {
        StandardJson::Null => OwnedValue::Null,
        StandardJson::Bool(b) => OwnedValue::Bool(*b),
        StandardJson::Number(n) => OwnedValue::from_number_bytes(n.raw_bytes()),
        StandardJson::String(s) => OwnedValue::String(
            s.as_str()
                .map_err(|e| EvalError::decode_failure(format!("{e}")))?
                .to_string(),
        ),
        StandardJson::Array(elements) => {
            let mut items = Vec::new();
            for e in *elements {
                items.push(owned_from_standard_json_at_depth(&e, depth + 1)?);
            }
            OwnedValue::Array(items)
        }
        StandardJson::Object(fields) => {
            let mut map = IndexMap::new();
            let mut remaining = *fields;
            while let Some((field, rest)) = remaining.uncons() {
                // A key that isn't `StandardJson::String` at all (e.g.
                // `StandardJson::Error`, a *structurally* malformed key like
                // a bare non-string token) used to `continue`, dropping the
                // field silently while `length` went on counting it. That is
                // #1194, distinct from #1192's decode failures, and it raises
                // now rather than degrading.
                let key = match field.key() {
                    StandardJson::String(s) => match s.as_str() {
                        Ok(cow) => cow.to_string(),
                        Err(e) => {
                            return Err(EvalError::decode_failure(format!("{e} in object key")))
                        }
                    },
                    _ => return Err(EvalError::malformed_json_text(field.key_cursor().text())),
                };
                let value = owned_from_standard_json_at_depth(&field.value(), depth + 1)?;
                map.insert(key, value);
                remaining = rest;
            }
            // A child with no sibling to pair as a value -- `{invalid}`,
            // `{"a"}`. `uncons` reports that as exhaustion, so without this
            // the object materialized as `{}` (#1194).
            if let Some(tail) = remaining.unpaired_tail() {
                return Err(EvalError::malformed_json_text(tail.text()));
            }
            OwnedValue::Object(map)
        }
        // See `to_owned_at_depth`'s own `is_error` arm (#1194/#1247): a
        // structurally malformed value raises rather than becoming `null`.
        // #2286: decode_failure, not new -- a bareword-garbage token is the
        // same "real jq rejects this at parse time" class as the malformed
        // member/delimiter errors #2286 already tagged; leaving this arm
        // ordinary/catchable was a real, live gap review caught (`[1,
        // xyz123] | try add catch "caught"` wrongly printed `"caught"`
        // instead of raising, unlike real jq).
        StandardJson::Error(msg) => return Err(EvalError::decode_failure(*msg)),
    })
}

/// Apply a format to an owned value and wrap it as a `GenericResult`.
fn format_result<S: EvalSemantics, V: DocumentValue>(
    format_type: &FormatType,
    owned: &OwnedValue,
    optional: bool,
) -> GenericResult<V> {
    match format_owned::<S>(format_type, owned, optional) {
        Ok(s) => GenericResult::Owned(OwnedValue::String(s)),
        Err(e) => GenericResult::Error(e),
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
    // Formats need neither an index nor a cursor, so the round-trip below is
    // pure overhead for them (#124). No non-finite-float guard is needed here
    // either: every non-JSON format bottoms out in `numeric_display_string`,
    // `owned_to_yaml`, or `props_value_to_string` (eval.rs), which already
    // render NaN/Infinity directly from an `OwnedValue::Float` or
    // `NumberLiteral` -- jq mode's `"null"`/`DBL_MAX`-text substitution
    // (#1075) or yq mode's `.nan`/`.inf`/`-.inf` -- with no dependence on
    // having passed through a JSON round-trip first.
    if let Expr::Format(format_type) = expr {
        return format_result::<S, _>(format_type, &owned, optional);
    }

    // `tostring` needs the same bypass, for correctness here, not just
    // speed (#1054): without it, `EXPR | tostring` on a genuinely computed
    // value (e.g. `(1e10 * 2)`) serializes through `to_json_for_reindex`'s
    // decimal-only float spelling below first, reparses as a
    // document-sourced-*looking* number, and echoes that baked text
    // verbatim per #1008's literal-preservation rule once `Builtin::
    // ToString`'s own arm (this file's `eval_builtin`, which now calls
    // `owned_to_string` directly too) finally runs -- permanently losing
    // the scientific-notation spelling real yq applies to a computed float
    // before the round trip ever has a chance to run.
    //
    // Narrow on purpose, not exhaustive: this only matches `tostring` as
    // the *immediately next* stage. Any intervening stage (even a no-op
    // `.`), a parenthesized `(tostring)`, `tostring?`, or `map(...|
    // tostring)` all still fall through to the round-trip below and
    // reproduce the original bug, since each intervening stage re-enters
    // this function with its own, different `expr` and bakes the float
    // into a `NumberLiteral` before `tostring` ever sees it in its
    // original form. Fixing that needs either threading "was this ever
    // reindexed" through every intermediate call here, or the same
    // origin-tracking mechanism #1128 already identifies as the real fix
    // for `@json`'s sibling gap -- tracked as #1134, not attempted here.
    if let Expr::Builtin(Builtin::ToString) = expr {
        return GenericResult::Owned(OwnedValue::String(owned_to_string::<S>(&owned)));
    }

    let json_str = owned.to_json_for_reindex::<S>();
    let json_bytes = json_str.as_bytes();
    let index = JsonIndex::build(json_bytes);
    let cursor = index.root(json_bytes);

    // No `if optional { wrap in Expr::Optional }` here: every public entry
    // point (`eval`/`eval_using`/`eval_with_cursor`/`eval_with_cursor_using`)
    // starts a fresh evaluation at `optional = false`, and after #693 the
    // only place this module ever forces `optional = true` is the
    // `IndexExpr`/`SliceExpr` special case in `eval_single`'s `Expr::
    // Optional` arm — which only threads it into `index_one_generic`/
    // `slice_one_generic`/`index_owned_by_key`, never into a call that
    // reaches this function. So `optional` is always `false` here; an
    // `Error` this bridge returns is caught, if at all, by the *caller's*
    // own `Expr::Optional`/`eval_try`-style boundary, not by wrapping it a
    // second time here.
    // Every `Err(e)` arm below (#1192) is defense-in-depth rather than a
    // reachable path: `cursor` is always rooted at `owned.to_json_for_reindex
    // ::<S>()`, a fresh serialization of an already-decoded `OwnedValue`
    // (a Rust `String`, which by construction can't hold invalid UTF-8, and
    // whose escapes this crate's own serializer writes). `owned_from_
    // standard_json` can only fail on genuinely malformed source bytes, which
    // this function never sees -- confirmed empirically (~25 jq expressions
    // over documents with real malformed UTF-8, none reached these `Err`
    // arms) as well as structurally (same "can this document ever be
    // malformed" argument `eval.rs`'s own textually-similar `to_owned`
    // copy relies on to stay out of #1192's scope entirely). Kept as `Err`
    // arms rather than `.unwrap()`/`.expect()` because the *type* (`Result`)
    // is what lets a real failure, if this invariant is ever violated by a
    // future change, surface as a normal `EvalError` instead of a panic.
    query_result_to_generic::<V>(full_eval::<Vec<u64>, S>(expr, cursor))
}

/// Materialize `value` (with its `cursor`, if any) and hand `expr` to the
/// full `OwnedValue` evaluator -- the "give up on the cursor-native path"
/// bridge every deferring native arm ends in.
///
/// Six call sites grew an identical hand-written copy of this three-step
/// shape (`to_owned_with_cursor` + reconstruct the AST node + `eval_on_owned`)
/// before it was extracted (#1687 item 4): `Expr::Limit`, `Expr::NthExpr`,
/// `Builtin::NthStream` and `Builtin::Has` in the two dispatch matches,
/// `eval_first_or_last_generic`'s input-queue guard, and
/// `each_limit_generic`'s `Flow`-returning local closure (now
/// [`bridge_to_full_evaluator_flow`]). CLAUDE.md's own #106 lesson --
/// duplicated predicates diverge silently -- applies directly: these copies
/// differ only in which `Expr` they rebuild, and nothing but review was
/// stopping one of them from acquiring a subtly different `optional` or
/// error-conversion rule.
///
/// **This bridge is lossy by construction and always has been.**
/// `to_owned_with_cursor` funnels through `to_owned_at_depth`, whose object
/// arm is an `IndexMap`, so a duplicate mapping key is gone before `expr`
/// runs -- regardless of `S::COLLAPSE_DUPLICATE_KEYS`. Reaching this
/// function at all is what every native arm in this file exists to avoid;
/// centralizing it does not make it cheaper or more faithful, it only makes
/// the remaining call sites countable.
fn bridge_to_full_evaluator<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    value: V,
    cursor: Option<V::Cursor>,
    optional: bool,
) -> GenericResult<V> {
    // Spelled as a `match` rather than `owned_or_err!`: this function sits
    // above that macro's own definition point in the file, and `macro_rules!`
    // is only in scope textually after it.
    match to_owned_with_cursor(&value, cursor) {
        Ok(owned) => eval_on_owned::<S, V>(expr, owned, optional),
        Err(e) => GenericResult::Error(e),
    }
}

/// [`bridge_to_full_evaluator`] for a sink-driven caller: same materialize +
/// delegate, then drain whatever the full evaluator produced into `sink`.
///
/// A decode failure surfaces as `Flow::Escaped(Control::Error(..))` rather
/// than `GenericResult::Error`, matching what `each_limit_generic`'s
/// hand-written closure did before this replaced it.
fn bridge_to_full_evaluator_flow<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    value: V,
    cursor: Option<V::Cursor>,
    optional: bool,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    match to_owned_with_cursor(&value, cursor) {
        Ok(owned) => drain_result_generic(eval_on_owned::<S, V>(expr, owned, optional), sink),
        Err(e) => Flow::Escaped(Control::Error(e)),
    }
}

/// Convert a `QueryResult` produced by `eval.rs` into the generic
/// evaluator's own `GenericResult`.
///
/// Extracted from [`eval_on_owned`]'s tail (#1909) so the path-context
/// bypasses below -- which call into `eval.rs` directly, without a reindex
/// bridge to go through -- share one conversion with it rather than growing
/// a second copy that can drift.
fn query_result_to_generic<V: DocumentValue>(
    result: QueryResult<'_, Vec<u64>>,
) -> GenericResult<V> {
    match result {
        QueryResult::One(v) => match owned_from_standard_json(&v) {
            Ok(o) => GenericResult::Owned(o),
            Err(e) => GenericResult::Error(e),
        },
        QueryResult::OneCursor(c) => match owned_from_standard_json(&c.value()) {
            Ok(o) => GenericResult::Owned(o),
            Err(e) => GenericResult::Error(e),
        },
        // Stops at the first element that fails to decode, keeping the
        // already-converted prefix (`partial_generic`) -- matching how an
        // ordinary `error`/`break` mid-generator stops the rest of a stream
        // elsewhere in this evaluator (#1164), not a "skip the bad one and
        // keep going" semantic (no precedent for that at this granularity).
        QueryResult::Many(vs) => {
            let mut out = Vec::new();
            let mut failure = None;
            for v in &vs {
                match owned_from_standard_json(v) {
                    Ok(o) => out.push(o),
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                }
            }
            match failure {
                Some(e) => partial_generic(out, Control::Error(e)),
                None => GenericResult::ManyOwned(out),
            }
        }
        QueryResult::None => GenericResult::None,
        QueryResult::Error(e) => GenericResult::Error(e),
        QueryResult::Owned(v) => GenericResult::Owned(v),
        QueryResult::ManyOwned(vs) => GenericResult::ManyOwned(vs),
        QueryResult::Break(label) => GenericResult::Break(label),
        QueryResult::Halt(code) => GenericResult::Halt(code),
        QueryResult::Partial(vs, control) => GenericResult::Partial(vs, control),
    }
}

/// The `MAX_REUSED_LITERAL_LEN` cap in [`OwnedValue::to_json_for_reindex`]
/// (`src/jq/value.rs`, #1211): past it, a `NumberLiteral`'s source text is
/// discarded and replaced by its parsed `NumberRepr`'s own formatting, so the
/// bridge stops being an identity on that node.
///
/// Duplicated from that function rather than shared because it is a private
/// `const` inside its body. `test_reindex_bridge_identity_predicate_agrees_1909`
/// is what keeps the two from drifting: it round-trips a literal either side
/// of this length through the real bridge and checks the predicate never
/// claims identity where the bridge did not deliver one. If the cap there
/// ever *shrinks*, a short literal in that test's corpus starts being
/// rewritten while this predicate still admits it, and the test fails.
const REINDEX_LITERAL_LEN_CAP: usize = 256;

/// Whether `eval_on_owned`'s reindex bridge -- serialize with
/// [`OwnedValue::to_json_for_reindex`], `JsonIndex::build`, then
/// `owned_from_standard_json` back on the far side -- is a **semantic identity**
/// on `value`, so skipping it cannot change what the evaluator downstream sees.
///
/// #1909: the path-context arms below hand `eval.rs` the `OwnedValue` they
/// already built instead of round-tripping it, which is only sound where the
/// round trip was pure overhead. It usually is -- but not always, and the
/// exceptions are all numeric, because `to_json_for_reindex` is a *formatter*
/// as much as a serializer:
///
/// - A **bare `Float`** is re-spelled by that formatter's mode-forked rule
///   (yq keeps a whole number's decimal point at any magnitude, jq keeps the
///   bare `Display` spelling, #953) and comes back as a `NumberLiteral`
///   carrying that new text. This is the case the bridge is genuinely
///   load-bearing for: without the guard, `.outer.big | parent` on
///   `10000000000000000000.0` prints `1e+19` in yq mode.
/// - A **NaN** `NumberLiteral` is replaced by `NAN_SENTINEL`.
/// - A `NumberLiteral` whose source text exceeds
///   [`REINDEX_LITERAL_LEN_CAP`] is discarded (#1211).
///
/// Everything else -- `null`, booleans, strings (escaped and unescaped
/// symmetrically), object keys, and the overwhelmingly common
/// document-sourced `NumberLiteral` with short source text, which
/// `to_json_for_reindex` echoes verbatim -- survives the trip unchanged, so
/// the bypass applies to it.
///
/// A bare **`Int`** is the one node that is *normalized* rather than
/// preserved and is still allowed through: it comes back as
/// `NumberLiteral(Int(n), "n")`. That is sound where a bare `Float` isn't,
/// because `to_json_for_reindex` writes an `Int` as exactly `format!("{n}")`
/// in both modes -- the only spelling an `i64` has -- so the literal the
/// bridge bakes in is the same text the bare `Int` renders as anyway, and
/// #1008's literal preservation has nothing new to echo. A `Float`'s
/// spelling, by contrast, is mode-forked and genuinely differs from what the
/// bare value would produce, which is exactly the `1e+19` breakage the guard
/// exists to prevent.
///
/// Excluding `Int` is not merely conservative here, it is the difference
/// between this fix applying to `succinctly yq` and not (code review):
/// **every** YAML integer materializes as a bare `Int` (YAML's
/// `number_literal()` override returns `Some` only for a preservable
/// *float*), so one `count: 3` line anywhere in a manifest would have
/// disabled the bypass for the whole document. Same for any
/// `--input-format json` document, whose `canonicalize_numbers` also
/// produces plain `Int`/`Float`.
///
/// Deliberately an **input-side** predicate rather than an output-side fixup.
/// A first version of this fix re-applied `yq_float_fidelity_fixup` to the
/// *result* instead; code review showed that isn't the same transformation at
/// all, and got it wrong in both directions -- re-spell-then-evaluate is not
/// evaluate-then-re-spell once the pipe does any computing.
/// `.outer.big|parent|.big|tostring` lost the document spelling
/// (`"1e+19"`), while `.outer.big|parent|.big+0` wrongly *gained* it for a
/// value the pipe had just computed. Only a value the bridge would have left
/// alone can safely skip the bridge.
fn reindex_bridge_is_identity(value: &OwnedValue) -> bool {
    match value {
        OwnedValue::Float(_) => false,
        OwnedValue::Int(_) => true,
        OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => false,
        OwnedValue::NumberLiteral(_, literal) => literal.len() <= REINDEX_LITERAL_LEN_CAP,
        OwnedValue::Array(items) => items.iter().all(reindex_bridge_is_identity),
        OwnedValue::Object(fields) => fields.values().all(reindex_bridge_is_identity),
        OwnedValue::Null | OwnedValue::Bool(_) | OwnedValue::String(_) => true,
    }
}

/// Re-derives every `Float`'s spelling in `values` through
/// [`eval_on_owned`]'s reindex bridge (`to_json_for_reindex`'s `S`-gated
/// formatter, #953), without touching anything else in the input document.
///
/// `Expr::Array`/`Expr::Comma`'s own native `eval_single` arms (#1168) build
/// `values` straight from `to_owned`/`to_owned_cursor` -- or, for a builtin
/// with its own native construction (`to_entries`, ...), from whatever *that*
/// builtin's arm produced, which uses the identical `to_owned`/`to_owned_cursor`
/// conversion internally. Neither has any notion of "this bare `Float` came
/// from a document-sourced literal that overflowed `i64`, keep its decimal
/// point regardless of magnitude" -- only `to_json_for_reindex`'s own
/// `S`-gated fallback applies that rule (see its doc comment,
/// `src/jq/value.rs`), and *only* because, before `Expr::Array`/`Expr::Comma`
/// had native arms, `[...]`/`,` had no choice but to fall through to this
/// same bridge for lack of one. Adding native arms without also keeping this
/// fix regressed #953 for a direct cursor result (`[.a]`, caught by its own
/// regression test) *and*, less obviously, for a value one layer removed
/// through a builtin's own construction (`[to_entries]` on an overflow
/// field, caught in code review -- `Builtin::ToEntries` also reads the field
/// via `to_owned_cursor`, just wrapped in an entry object before `Expr::Array`
/// ever sees it, so scoping the fixup to only direct `GenericResult::
/// OneCursor`/`ManyCursor` results missed this case entirely).
///
/// This is why the fixup applies unconditionally to the *whole* constructed
/// result rather than trying to track, per value, whether it's document-
/// sourced or genuinely computed (e.g. `1e10 * 2`) -- that distinction isn't
/// recoverable from a `GenericResult` variant once a value has passed through
/// even one further construction step (`to_entries`'s object wrapping looks
/// identical, from here, to freshly computed arithmetic). The trade-off this
/// accepts, matching an already-documented precedent
/// (`test_yq_array_wrapped_computed_float_keeps_scientific_notation_known_gap_1168`,
/// same shape as #1124/#1144's `join` gap): a genuinely computed float
/// wrapped directly in `[...]`/`,` (`[1e10 * 2]`) also gets its decimal point
/// forced, when real yq would keep scientific notation there. Getting both
/// right needs provenance tagged at `OwnedValue::Float`'s own construction
/// site, not reconstructed after the fact here -- out of scope for this fix.
///
/// Round-tripping just `values` (not the whole input document, unlike the
/// old wildcard fallback these two arms replace) keeps `Expr::Array`'s and
/// `Expr::Comma`'s actual fix — duplicate mapping keys survive a builtin's
/// own cursor-native conversion (`to_entries`, etc.) intact. Safe with
/// respect to that fix: nothing in `values` still has a duplicate *mapping
/// key* left to lose by round-tripping again — any genuine YAML duplicate
/// was already collapsed the moment `to_owned`/`to_owned_cursor` first
/// converted its mapping to an `IndexMap`-backed `Object`, before this ever
/// runs; a builtin with its own dedup-preserving fix has already turned its
/// duplicates into distinct array elements by this point instead, which
/// round-trip through JSON text with no collision to lose.
///
/// A no-op in jq mode (`to_json_for_reindex`'s own `S`-gate already makes
/// the round trip itself a no-op there — jq drops a computed float's
/// literal formatting unconditionally), and a no-op whenever `values` has no
/// `Float`/`NumberLiteral(Float, _)` anywhere in its tree (`contains_float`)
/// — skipped outright rather than paying for a round trip that has nothing
/// to fix, which is the common case (`[.a, .b, .c]` over strings/objects/
/// plain ints). `Ok` carries the fixed-up values back for the caller to
/// package (`Expr::Array` always wraps them in one `OwnedValue::Array`;
/// `Expr::Comma` collapses via [`owned_vec_to_generic_result`]); `Err`
/// carries an already-terminal `GenericResult` (an `Error`, in practice —
/// see [`eval_on_owned`]'s own doc comment for why this is defense-in-depth
/// rather than a reachable path for internally-constructed input) for the
/// caller to return as-is.
fn yq_float_fidelity_fixup<S: EvalSemantics, V: DocumentValue>(
    values: Vec<OwnedValue>,
) -> Result<Vec<OwnedValue>, GenericResult<V>> {
    if S::TAG != EvalTag::Yq || values.is_empty() || !values.iter().any(contains_float) {
        return Ok(values);
    }
    match eval_on_owned::<S, V>(&Expr::Identity, OwnedValue::Array(values), false) {
        GenericResult::Owned(OwnedValue::Array(fixed)) => Ok(fixed),
        GenericResult::Error(e) => Err(GenericResult::Error(e)),
        other => Err(other),
    }
}

/// Whether `value`'s tree contains a `Float`/`NumberLiteral(Float, _)`
/// anywhere -- the only shapes [`yq_float_fidelity_fixup`]'s round trip can
/// possibly change (see `to_json_for_reindex`'s own `S`-gated fallback,
/// which is scoped identically). A cheap pre-check so a document with no
/// float anywhere in the wrapped result skips the round trip entirely,
/// rather than paying for one that has nothing to do.
fn contains_float(value: &OwnedValue) -> bool {
    match value {
        OwnedValue::Float(_) => true,
        OwnedValue::NumberLiteral(NumberRepr::Float(_), _) => true,
        OwnedValue::Array(items) => items.iter().any(contains_float),
        OwnedValue::Object(fields) => fields.values().any(contains_float),
        OwnedValue::Null
        | OwnedValue::Bool(_)
        | OwnedValue::Int(_)
        | OwnedValue::NumberLiteral(NumberRepr::Int(_), _)
        | OwnedValue::String(_) => false,
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
            Control::Halt(code) => GenericResult::Halt(code),
        }
    } else {
        GenericResult::Partial(prefix, control)
    }
}

/// Normalize a `Vec<OwnedValue>` accumulator into the smallest `GenericResult`
/// shape that represents it. Mirrors [`super::eval::owned_vec_to_result`];
/// both share [`collapse_vec`]'s one definition of the actual collapse (#1067).
fn owned_vec_to_generic_result<V: DocumentValue>(vs: Vec<OwnedValue>) -> GenericResult<V> {
    collapse_vec(
        vs,
        || GenericResult::None,
        GenericResult::Owned,
        GenericResult::ManyOwned,
    )
}

/// Splice `prefix` in front of whatever `result` yields (#1812) -- the
/// `eval_single`/`GenericResult` twin of [`super::eval::prepend`], used by
/// `Expr::Try`'s own arm above the same way `eval::eval_try` uses its
/// mirror: the body's outputs before an error/break, then the catch
/// handler's own result spliced in after them.
///
/// `result` here is always [`eval_each_owned_collect`]'s own return value,
/// whose contract (`owned_vec_to_generic_result`/`partial_generic`, both
/// called there) only ever produces `None`/`Owned`/`ManyOwned`/`Error`/
/// `Break`/`Halt`/`Partial` -- never `One`/`OneCursor`/`Many`/`ManyCursor`/
/// a lazy variant, so this covers exactly that set rather than
/// `GenericResult`'s full one.
fn prepend_generic<V: DocumentValue>(
    mut prefix: Vec<OwnedValue>,
    result: GenericResult<V>,
) -> GenericResult<V> {
    if prefix.is_empty() {
        return result;
    }
    match result {
        GenericResult::None => owned_vec_to_generic_result(prefix),
        GenericResult::Owned(v) => {
            prefix.push(v);
            owned_vec_to_generic_result(prefix)
        }
        GenericResult::ManyOwned(vs) => {
            prefix.extend(vs);
            owned_vec_to_generic_result(prefix)
        }
        GenericResult::Error(e) => partial_generic(prefix, Control::Error(e)),
        GenericResult::Break(label) => partial_generic(prefix, Control::Break(label)),
        GenericResult::Halt(code) => partial_generic(prefix, Control::Halt(code)),
        GenericResult::Partial(more, control) => {
            prefix.extend(more);
            partial_generic(prefix, control)
        }
        other => other,
    }
}

/// Evaluate `expr` against `input` through `eval.rs`'s demand-driven
/// `eval_each_owned`, collecting every output instead of stopping after the
/// first (that's [`each_take_first_generic`]'s job). This is what lets a
/// plain top-level `inputs | input_line_number` or `(., input) | error(...)`
/// interleave `input`/`inputs` with the rest of the pipe/comma the way real
/// jq does, instead of `eval_single`'s `fold_pipe_stages`/`Expr::Comma` arms
/// draining a generator fully before the next stage ever runs (#1504).
///
/// `eval_each_owned` already dispatches through `eval.rs`'s full native lazy
/// arm set (`Comma`/`Pipe`/`Paren`/`Compare`/`If`/`Try`/`Label`/
/// `Builtin::Inputs`/...), not just the three `eval_each_generic` mirrors --
/// so this is a thin collecting sink over already-tested machinery, not a
/// new evaluator.
///
/// `Flow::Stopped` is unreachable here in practice (the sink below never
/// returns `Demand::Stop`), but is folded into the same arm as `Exhausted`
/// rather than asserted, since nothing here can prove a nested consumer
/// could never surface it.
fn eval_each_owned_collect<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    input: &OwnedValue,
    optional: bool,
) -> GenericResult<V> {
    let mut collected: Vec<OwnedValue> = Vec::new();
    let flow = eval_each_owned::<S>(expr, input, optional, &mut |v| {
        collected.push(v);
        Demand::Continue
    });
    match flow {
        Flow::Exhausted | Flow::Stopped { .. } => owned_vec_to_generic_result(collected),
        Flow::Escaped(control) => partial_generic(collected, control),
    }
}

/// Normalize a `Vec<V::Cursor>` accumulator into the smallest `GenericResult`
/// shape that represents it -- the borrowed-cursor counterpart of
/// [`owned_vec_to_generic_result`], for the same reason (#1048): a caller
/// with zero results must collapse to `None`, not `ManyCursor(vec![])`.
fn cursor_vec_to_generic_result<V: DocumentValue>(cs: Vec<V::Cursor>) -> GenericResult<V> {
    collapse_vec(
        cs,
        || GenericResult::None,
        GenericResult::OneCursor,
        GenericResult::ManyCursor,
    )
}

/// Finalize a fork's accumulated `outputs`, given an optional terminating
/// `Control`. Mirrors [`super::eval::finish_fork`]: a trailing `Error` is
/// silenced (keeping whatever outputs already succeeded) when `optional` is
/// set; `Break` always propagates via [`partial_generic`], uncaught by
/// `optional`, for the same reason `finish_fork`'s doc comment gives.
fn finish_fork_generic<V: DocumentValue>(
    outputs: Vec<OwnedValue>,
    control: Option<Control>,
    optional: bool,
) -> GenericResult<V> {
    match control {
        None => owned_vec_to_generic_result(outputs),
        // A decode failure (#1620) is never silenced by an ambient
        // `optional`, unlike an ordinary trailing error -- see
        // `Expr::Optional`'s own arm above for the full rationale.
        Some(Control::Error(ref e)) if optional && !e.is_decode_failure() => {
            owned_vec_to_generic_result(outputs)
        }
        Some(control) => partial_generic(outputs, control),
    }
}

/// Unwrap a fallible materialization inside a `GenericResult<V>`-returning
/// function, turning a decode failure (#1247) into the `GenericResult::Error`
/// arm that function's callers already handle.
///
/// A macro rather than `?` because these functions return `GenericResult<V>`
/// directly -- the evaluator carries its errors in the value domain, not in a
/// `Result` (see `GenericResult::Error`'s own docs).
macro_rules! owned_or_err {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return GenericResult::Error(e),
        }
    };
}

/// Unwrap a fallible materialization inside an `Option<Control>`-returning
/// helper, turning a decode failure (#1247) into the same `Control::Error`
/// those helpers already forward for `GenericResult::Error`.
///
/// A macro rather than a function because it has to `return` from the
/// *caller*; `?` can't, since these helpers return `Option<Control>` where
/// `None` means success.
macro_rules! push_or_control {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return Some(Control::Error(e)),
        }
    };
}

/// [`owned_or_err`]'s `optional`-consulting twin: an ordinary (non-decode-
/// failure) error is suppressed to `GenericResult::None` when `optional` is
/// set, matching [`super::eval::to_owned_or_suppress`]'s exact behavior for
/// the `eval.rs` side of this same materialization (#2231, code review --
/// this file's `Builtin::ToString` arm and its catch-all wildcard fallback
/// each hand-rolled this three-line match independently before this macro
/// existed, the exact "rediscovering missing at one more call site" pattern
/// `to_owned_or_suppress`'s own doc comment already names for `eval.rs`).
macro_rules! owned_or_suppress {
    ($e:expr, $optional:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) if suppresses(&e, $optional) => return GenericResult::None,
            Err(e) => return GenericResult::Error(e),
        }
    };
}

/// Append every output of a `GenericResult` stream to `out`, returning any
/// terminating `Control` instead of collapsing to the first output the way
/// an earlier `Expr::Compare` arm did before #768. Mirrors
/// [`super::eval::push_owned_values`] for the generic evaluator's
/// cursor-aware result type — used to fork `Expr::Comma`'s operands into
/// every output, the same way [`push_generic_truthiness`] already forks
/// `select`'s condition. `Expr::Compare` moved off this helper for #1481
/// (see `eval_compare_generic`).
fn push_generic_owned_values<V: DocumentValue>(
    result: GenericResult<V>,
    out: &mut Vec<OwnedValue>,
) -> Option<Control> {
    match result.materialize_lazy() {
        // A decode failure here is an uncaught error like any other the
        // `GenericResult::Error` arm below already forwards (#1247).
        GenericResult::One(v) => out.push(push_or_control!(to_owned(&v))),
        GenericResult::OneCursor(c) => out.push(push_or_control!(to_owned_cursor(&c))),
        GenericResult::Many(vs) => {
            for v in &vs {
                out.push(push_or_control!(to_owned(v)));
            }
        }
        GenericResult::ManyCursor(cs) => {
            for c in &cs {
                out.push(push_or_control!(to_owned_cursor(c)));
            }
        }
        GenericResult::None => {}
        GenericResult::Owned(v) => out.push(v),
        GenericResult::ManyOwned(vs) => out.extend(vs),
        GenericResult::Error(e) => return Some(Control::Error(e)),
        GenericResult::Break(label) => return Some(Control::Break(label)),
        GenericResult::Halt(code) => return Some(Control::Halt(code)),
        GenericResult::Partial(vs, control) => {
            out.extend(vs);
            return Some(control);
        }
        GenericResult::LazyKeys { .. }
        | GenericResult::LazyIndexRange(_)
        | GenericResult::LazySeq(_) => {
            unreachable!("materialize_lazy() already normalized every lazy variant")
        }
    }
    None
}

/// Whether `c`'s subtree, at any depth, contains a value that cannot be
/// honestly answered "truthy or falsy" at all -- a decode failure (a
/// string token whose bytes don't decode), a structural error (a token
/// the semi-index accepted as a span but couldn't classify, #1194), or a
/// #1642 colliding-decode-failure-key -- and if so, the `Control::Error`
/// a caller should raise instead of falling through to
/// [`DocumentCursor::is_falsy`]'s own silent "not falsy" default for
/// exactly these cases.
///
/// This is [`to_owned_cursor_at_depth`]'s own traversal and validation
/// (same object/array unconsing, same key-collision guard) -- but it
/// builds no `OwnedValue` container or scalar payload anywhere, only
/// walking and checking, raising on a value corrupted at *any* depth
/// below `c`'s own top level -- not just when `c` itself is the bad
/// scalar (an earlier version of this fix only checked `c.value()`
/// directly, silently missing anything nested inside a well-formed
/// container; caught by review, #1645). Once a scalar is reached, this
/// runs only the two checks that can actually raise
/// (`string_decode_error`/`is_error`) -- not `to_owned_cursor_at_depth`'s
/// preceding `explicit_tag`/`canonicalize_numbers` attempt, since tag
/// resolution only ever changes a successfully-decoded value's inferred
/// *type*, never turns a decode failure or structural error into success
/// or vice versa (`tagged_scalar_to_owned` requires `as_str()` to already
/// have succeeded), so skipping it here cannot skip a real raise.
///
/// For a scalar condition this keeps #1645's O(1) win in full: neither
/// this walk nor `is_falsy()` afterward materializes anything. For a
/// container condition, jq's own truthiness rule needs none of this --
/// any object/array is unconditionally truthy regardless of contents --
/// but this walk still visits the whole subtree anyway, because a
/// corrupted value can be anywhere inside it and the #1247/#1194
/// invariant this function exists to enforce is a property of the whole
/// subtree, not of the truthiness question alone. That walk still
/// allocates one `String` per object key (`resolve_display_key`'s own
/// #1642 guard), so "no allocation" only ever describes the *value*
/// side (no `OwnedValue`/`Vec` payload) -- not a claim that a
/// container-shaped condition is free.
fn push_generic_truthiness_cursor_error<C: DocumentCursor>(c: &C, depth: usize) -> Option<Control> {
    assert_nesting_depth(depth);
    let value = c.value();
    if let Some(fields) = value.as_object() {
        // The #1642 collision map is built lazily, only from the point an
        // undecodable key actually appears (#2061).
        //
        // `DisplayKeyGuard::check` reports a collision only when
        // `map.contains_key(key)` *and* a fallback key is involved -- either
        // this one, or an earlier one it recorded. So until the first
        // fallback key, `collides` is false for every key no matter what the
        // map holds, and the only thing building it accomplishes is one
        // `String` allocation per key. That was the entire cost of this
        // walk: it is what made `path(.[0])` over a 1,000,000-object array
        // take 577ms, against 50ms with the walk removed altogether.
        //
        // When a fallback key *does* appear at position `k`, the map has to
        // hold keys `0..k` before that key can be checked against them -- a
        // later fallback can collide with an earlier clean key -- so the
        // prefix is re-walked then, and only then. That keeps the error
        // *order* identical to the eager version (a value's error at an
        // earlier field still wins over a collision at a later one), which
        // a cheaper "check all keys afterwards" split would have changed.
        // Same cheap-probe shape `collapsed_fields` uses (#1514).
        let mut map: IndexMap<String, ()> = IndexMap::new();
        let mut guard = DisplayKeyGuard::default();
        let mut seen_fallback = false;
        let mut index = 0usize;
        let mut f = fields;
        while let Some((field, rest)) = f.uncons() {
            if !seen_fallback {
                match key_display_string_kind(&field.key) {
                    None => return Some(Control::Error(f.malformed_member_error())),
                    // Clean key, no fallback seen yet: no collision is
                    // possible, so nothing is recorded and nothing allocated.
                    Some((_, false)) => {}
                    Some((_, true)) => {
                        seen_fallback = true;
                        // Re-walk this object's first `index` keys to seed
                        // the map, then let the guarded path below handle
                        // this key and every one after it.
                        if let Some(prefix) = value.as_object() {
                            let mut p = prefix;
                            for _ in 0..index {
                                let Some((earlier, rest)) = p.uncons() else {
                                    break;
                                };
                                match resolve_display_key(&earlier.key, &map, &mut guard) {
                                    Ok(Some(key)) => {
                                        map.insert(key, ());
                                    }
                                    Ok(None) => {
                                        return Some(Control::Error(p.malformed_member_error()))
                                    }
                                    Err(e) => return Some(Control::Error(e)),
                                }
                                p = rest;
                            }
                        }
                    }
                }
            }
            if seen_fallback {
                match resolve_display_key(&field.key, &map, &mut guard) {
                    Ok(Some(key)) => {
                        map.insert(key, ());
                    }
                    Ok(None) => return Some(Control::Error(f.malformed_member_error())),
                    Err(e) => return Some(Control::Error(e)),
                }
            }
            index += 1;
            if let Some(control) =
                push_generic_truthiness_cursor_error(&field.value_cursor, depth + 1)
            {
                return Some(control);
            }
            f = rest;
        }
        if f.ends_unpaired() {
            return Some(Control::Error(f.malformed_member_error()));
        }
        None
    } else if let Some(elements) = value.as_array() {
        let mut elems = elements;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            if let Some(control) = push_generic_truthiness_cursor_error(&elem_cursor, depth + 1) {
                return Some(control);
            }
            elems = rest;
        }
        None
    } else if let Some(reason) = value.string_decode_error() {
        Some(Control::Error(EvalError::decode_failure(reason)))
    } else if value.is_error() {
        // #2286: `decode_failure`, not `new` -- see `to_owned_at_depth`'s
        // own `is_error()` arm above for the full rationale; this is the
        // truthiness-check sibling of that same class.
        Some(Control::Error(EvalError::decode_failure(
            value
                .error_message()
                .unwrap_or("malformed value in document"),
        )))
    } else {
        None
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
        GenericResult::One(v) => out.push(push_or_control!(to_owned(&v)).is_truthy()),
        // `DocumentCursor::is_falsy` answers this in O(1) without
        // materializing the value at all -- an arbitrarily deep object/
        // array previously paid a full recursive `to_owned_cursor` copy
        // just to learn it isn't `null`/`false` (#1645). `is_falsy` itself
        // is deliberately silent on a value that fails to decode (its own
        // doc comment: "conservative assumption", matching `--exit-status`'s
        // pre-existing, best-effort use of it) -- `select`'s own filtering
        // needs the real #1247/#1194 raise instead, so that check runs
        // first, same as `to_owned_at_depth`'s own two checks in the same
        // order, just without materializing on the ordinary-value path.
        GenericResult::OneCursor(c) => {
            if let Some(control) = push_generic_truthiness_cursor_error(&c, 0) {
                return Some(control);
            }
            // `select`'s condition truthiness is about the real value, not
            // about what a later `-e`/JSON-output convention would sanitize
            // it to -- `Preserve` keeps a malformed number truthy here the
            // same way it always has (see `StreamableValue::is_falsy`'s own
            // doc comment for the convention this parameter selects).
            out.push(!c.is_falsy(JsonConvention::Preserve));
        }
        GenericResult::Many(vs) => {
            for v in &vs {
                out.push(push_or_control!(to_owned(v)).is_truthy());
            }
        }
        GenericResult::ManyCursor(cs) => {
            for c in &cs {
                if let Some(control) = push_generic_truthiness_cursor_error(c, 0) {
                    return Some(control);
                }
                out.push(!c.is_falsy(JsonConvention::Preserve));
            }
        }
        // A lazy keys array, materialized or not, sorted or not, is
        // array-shaped and therefore always truthy in jq (only `null`/
        // `false` are falsy) — no need to materialize just to answer this.
        GenericResult::LazyKeys { .. } => out.push(true),
        // Same reasoning as `LazyKeys` above — the array-index-range result
        // of `keys`/`keys_unsorted` on an array is always truthy.
        GenericResult::LazyIndexRange(_) => out.push(true),
        // Unlike `LazyKeys`/`LazyIndexRange` above, a `LazySeq` CAN fail
        // (arbitrary `map(f)`), and that failure must surface here rather
        // than being reported as "truthy" before construction is even known
        // to succeed — do NOT replace this with a blind
        // `.materialize_lazy()` call, which would also force materializing
        // the two variants above on every `select`, undoing their whole
        // point.
        GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
            Ok(_array) => out.push(true),
            Err(control) => return Some(control),
        },
        GenericResult::None => {}
        GenericResult::Owned(v) => out.push(v.is_truthy()),
        GenericResult::ManyOwned(vs) => out.extend(vs.iter().map(OwnedValue::is_truthy)),
        GenericResult::Error(e) => return Some(Control::Error(e)),
        GenericResult::Break(label) => return Some(Control::Break(label)),
        GenericResult::Halt(code) => return Some(Control::Halt(code)),
        GenericResult::Partial(vs, control) => {
            out.extend(vs.iter().map(OwnedValue::is_truthy));
            return Some(control);
        }
    }
    None
}

/// Flatten a batch of per-element results into one `Vec<OwnedValue>`.
///
/// A bare `Error`/`Break`/`Partial` must never appear in `items` — callers
/// that build up a per-element batch route those variants to an early return
/// (folding whatever was already flattened into a `Partial` of their own via
/// [`partial_generic`]) instead of pushing them here. A `LazySeq`/`LazyKeys`
/// item CAN still fail once materialized here, though (its failure isn't
/// known until `materialize_lazy()` actually pulls it) — that's why this
/// returns `Result`, not a plain `Vec` as it used to before `LazySeq`
/// existed (#725): `LazySeq` runs arbitrary `map(f)`, and `LazyKeys` can
/// carry a #1194 malformed (non-string) object key (#1936) that only
/// surfaces on materialization -- both can be reached here (e.g. via
/// `.[] | (keys_unsorted | map(f))`) before that materialization has
/// happened. `LazyIndexRange` is the only one of the three that genuinely
/// can never fail: its value is fully described by the array's length
/// alone (#684).
fn flatten_generic_results<V: DocumentValue>(
    items: Vec<GenericResult<V>>,
) -> Result<Vec<OwnedValue>, Control> {
    let mut results = Vec::new();
    for r in items {
        match r.materialize_lazy() {
            // `One`/`Many` are exhaustiveness only here too, same as
            // `into_lazy_items`'s identical comment above: `items`' one
            // source (`fold_pipe_stages`'s `ManyCursor(cs)` arm) evaluates
            // `rest` per element via `eval_single(&rest, c.value(), optional,
            // Some(c))`, and `c: V::Cursor` is always concrete -- the same
            // "ambient cursor is always `Some`" invariant that rules out a
            // bare `One`/`Many` reaching `into_lazy_items` rules it out here.
            GenericResult::One(v) => results.push(to_owned(&v).map_err(Control::Error)?),
            GenericResult::OneCursor(c) => {
                results.push(to_owned_cursor(&c).map_err(Control::Error)?);
            }
            GenericResult::Many(rs) => {
                for r in &rs {
                    results.push(to_owned(r).map_err(Control::Error)?);
                }
            }
            GenericResult::ManyCursor(cs) => {
                for c in &cs {
                    results.push(to_owned_cursor(c).map_err(Control::Error)?);
                }
            }
            GenericResult::None => {}
            GenericResult::Owned(o) => results.push(o),
            GenericResult::ManyOwned(os) => results.extend(os),
            // Only reachable via a `LazySeq` item that failed to
            // materialize — a bare `Error`/`Break` still can't appear here
            // per this function's own precondition above.
            GenericResult::Error(e) => return Err(Control::Error(e)),
            GenericResult::Break(label) => return Err(Control::Break(label)),
            GenericResult::Halt(code) => return Err(Control::Halt(code)),
            GenericResult::Partial(..) => {
                unreachable!("Partial already routed to an early return above")
            }
            GenericResult::LazyKeys { .. }
            | GenericResult::LazyIndexRange(_)
            | GenericResult::LazySeq(_) => {
                unreachable!("materialize_lazy() already normalized every lazy variant")
            }
        }
    }
    Ok(results)
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
            // `eval_on_owned` re-evaluates through the JSON-only `QueryResult`
            // path (`eval.rs`), which has no lazy-keys concept — only this
            // module's own `Builtin::Keys`/`Builtin::KeysUnsorted` arms ever
            // produce `LazyKeys`.
            GenericResult::LazyKeys { .. } => {
                unreachable!("eval_on_owned never returns LazyKeys")
            }
            GenericResult::LazyIndexRange(_) => {
                unreachable!("eval_on_owned never returns LazyIndexRange")
            }
            GenericResult::LazySeq(_) => unreachable!("eval_on_owned never returns LazySeq"),
            GenericResult::None => {}
            // The outputs already produced no longer vanish (#400, #494).
            GenericResult::Error(e) => return partial_generic(results, Control::Error(e)),
            GenericResult::Owned(o) => results.push(o),
            GenericResult::ManyOwned(os) => results.extend(os),
            GenericResult::Break(label) => return partial_generic(results, Control::Break(label)),
            GenericResult::Halt(code) => return partial_generic(results, Control::Halt(code)),
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

    /// Lazy object keys (`keys`/`keys_unsorted` on an object), not yet
    /// materialized into strings (#140, #683). `sorted` distinguishes `keys`
    /// (needs lexicographic order for anything but `length`) from
    /// `keys_unsorted` (document order is always fine). `length` answers
    /// from `fields.len()` regardless of `sorted`; `.[]`, `.[n]`, `first`,
    /// and `last` only fast-path when `!sorted` — those need order this
    /// variant hasn't computed yet, via the `Pipe` dispatch below. Every
    /// other consumer falls back to materializing (sorting first iff
    /// `sorted`) exactly as eager `keys`/`keys_unsorted` did before #140.
    /// `collapse` carries `S::COLLAPSE_DUPLICATE_KEYS` from the construction
    /// site (#1385). `GenericResult<V>` is deliberately generic over `V`
    /// alone, so the consumers below -- `materialize_lazy`, `stream_json`,
    /// `stream_yaml` -- have no `S` in scope; riding the flag on the variant
    /// is the same bridge `EvalTag` provides for `LazySeq`'s buffered stages.
    LazyKeys {
        fields: V::Fields,
        sorted: bool,
        collapse: bool,
    },

    /// Lazy array-index range (`keys`/`keys_unsorted` on an array), not yet
    /// materialized into `OwnedValue::Int`s (#684). The value is fully
    /// described by `len` alone — `[0, 1, ..., len-1]` — so `length`, `.[]`,
    /// `.[n]`, `first`, and `last` all answer with plain arithmetic (no
    /// allocation at all) via the `Pipe` dispatch below; every other
    /// consumer falls back to materializing exactly as eager array
    /// `keys`/`keys_unsorted` did.
    LazyIndexRange(usize),

    /// A composed, not-yet-materialized `map` chain (#724, #725) — plain
    /// `arr`/`obj | map(f)` and `keys_unsorted | map(f)` alike, and any
    /// further `| map(g)` stages pushed onto the same chain by the `Pipe`
    /// fold's composability arm below. See `LazySeq`'s own docs for the
    /// mechanism and `docs/plan/jq-lazy-map-select.md` for the design.
    ///
    /// Boxed (#789): `LazySeq<V>` embedded directly grew `GenericResult<V>`
    /// from 120 to 184 bytes (measured via `size_of_val`, x86_64), and every
    /// arm of this enum -- `select`'s own trivial boolean-test result
    /// included -- pays that larger copy on every return, since Rust enum
    /// size is the discriminant plus the *widest* variant regardless of
    /// which one is active. That extra copying is flat per call, not
    /// size-scaling, matching the issue's own "flat ~6-7% slower across
    /// 100kb-100mb" measurement on AMD Ryzen 9 7950X exactly (Apple M4 Pro
    /// showed no effect, consistent with #106's precedent that small
    /// constant-factor costs don't always port between architectures).
    /// Boxing this one variant is the minimal fix: every other variant's own
    /// size is unaffected, and `LazySeq<V>` itself only grows a single
    /// pointer wherever it's already handled by reference/move.
    LazySeq(Box<LazySeq<V>>),

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

    /// `halt`/`halt_error(n)`: exit the whole process with this code (#791).
    /// Mirrors [`QueryResult::Halt`] — not caught by `try`/`catch` or
    /// `label`/`break`, unlike `Error`/`Break`.
    Halt(i32),

    /// One or more outputs were produced before the stream terminated in an
    /// error, a `break`, or a `halt` (#400, #494, #791). Mirrors
    /// [`QueryResult::Partial`].
    Partial(Vec<OwnedValue>, Control),
}

impl<V: DocumentValue> GenericResult<V> {
    /// Whether streaming this result would write at least one value to
    /// output.
    ///
    /// An exhaustive match, not a blocklist: callers like `stream_cursor!`'s
    /// multi-doc `---` placement need this answer *before* actually
    /// streaming, so the separator lands ahead of real content and never in
    /// front of an empty result. This enum has grown new variants several
    /// times (`LazyKeys`/`LazyIndexRange`/`LazySeq`, then `Halt` for #791),
    /// and a hand-maintained exclusion list missed `Halt` the first time
    /// (fixed by d259fba4, after a stray `---` was emitted for it) — an
    /// exhaustive match turns the next such miss into a compile error
    /// instead of a repeat of that bug.
    pub fn produces_output(&self) -> bool {
        match self {
            Self::One(_)
            | Self::OneCursor(_)
            | Self::LazyKeys { .. }
            | Self::LazyIndexRange(_)
            | Self::LazySeq(_)
            | Self::Error(_)
            | Self::Owned(_)
            | Self::Partial(_, _) => true,
            Self::Many(vs) => !vs.is_empty(),
            Self::ManyCursor(cs) => !cs.is_empty(),
            Self::ManyOwned(vs) => !vs.is_empty(),
            Self::None | Self::Break(_) | Self::Halt(_) => false,
        }
    }

    /// Materialize any lazy variant (`LazyKeys`/`LazyIndexRange`/`LazySeq`)
    /// into `Owned`/`Error`/`Break`, leaving every other variant unchanged.
    /// The shared collapse point for every consumer that was always going to
    /// materialize a lazy result anyway (as opposed to `push_generic_truthiness`
    /// and `eval_first_or_last_generic`, which have their own bespoke `LazySeq`
    /// handling below specifically to avoid forcing materialization here).
    fn materialize_lazy(self) -> Self {
        match self {
            Self::LazyKeys {
                fields,
                sorted,
                collapse,
            } => match materialize_lazy_keys::<V>(&fields, sorted, collapse) {
                Ok(owned) => Self::Owned(owned),
                Err(e) => Self::Error(e),
            },
            Self::LazyIndexRange(len) => Self::Owned(materialize_lazy_index_range(len)),
            Self::LazySeq(seq) => match seq.materialize_atomic() {
                Ok(owned) => Self::Owned(owned),
                Err(Control::Error(e)) => Self::Error(e),
                Err(Control::Break(label)) => Self::Break(label),
                Err(Control::Halt(code)) => Self::Halt(code),
            },
            other => other,
        }
    }

    /// Convert to OwnedValue for output.
    ///
    /// `Ok(None)` means "no single value to represent" (`None`, `Break`,
    /// `Error`, `Halt`, `Partial` -- unchanged); `Err` means a value *was*
    /// there but a scalar in it could not be decoded (#1247), which is a
    /// different answer and must not collapse into the same `None`.
    pub fn into_owned(self) -> Result<Option<OwnedValue>, EvalError> {
        Ok(match self.materialize_lazy() {
            Self::One(v) => Some(to_owned(&v)?),
            Self::OneCursor(c) => Some(to_owned_cursor(&c)?),
            Self::Many(vs) => Some(OwnedValue::Array(to_owned_all(&vs)?)),
            Self::ManyCursor(cs) => Some(OwnedValue::Array(to_owned_all_cursors(&cs)?)),
            Self::None => None,
            Self::Error(_) => None,
            Self::Owned(o) => Some(o),
            Self::ManyOwned(os) => Some(OwnedValue::Array(os)),
            Self::Break(_) => None,
            Self::Halt(_) => None,
            // A `Partial` prefix is not representable as a single value —
            // same "not representable" answer as `Break`/`Error` here.
            Self::Partial(..) => None,
            Self::LazyKeys { .. } | Self::LazyIndexRange(_) | Self::LazySeq(_) => {
                unreachable!("materialize_lazy() already normalized every lazy variant")
            }
        })
    }

    /// Collect all results into a Vec of OwnedValues.
    ///
    /// A `Partial` collects its prefix — the whole point of #400/#494 is
    /// that those outputs are no longer discarded.
    /// A decode failure is the one thing this does *not* swallow: `Err` says
    /// a value was present but undecodable (#1247), which the existing
    /// deliberate `Error(_) => vec![]` swallow above would otherwise hide.
    pub fn collect_owned(self) -> Result<Vec<OwnedValue>, EvalError> {
        Ok(match self.materialize_lazy() {
            Self::One(v) => vec![to_owned(&v)?],
            Self::OneCursor(c) => vec![to_owned_cursor(&c)?],
            Self::Many(vs) => to_owned_all(&vs)?,
            Self::ManyCursor(cs) => to_owned_all_cursors(&cs)?,
            Self::None => vec![],
            Self::Error(_) => vec![],
            Self::Owned(o) => vec![o],
            Self::ManyOwned(os) => os,
            Self::Break(_) => vec![],
            Self::Halt(_) => vec![],
            Self::Partial(vs, _control) => vs,
            Self::LazyKeys { .. } | Self::LazyIndexRange(_) | Self::LazySeq(_) => {
                unreachable!("materialize_lazy() already normalized every lazy variant")
            }
        })
    }

    /// Check if this is an error.
    ///
    /// A `Partial` prefix followed by an error still counts.
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_) | Self::Partial(_, Control::Error(_)))
    }

    /// Whether evaluating this ended in *any* control -- an error, a
    /// `break`, or a `halt` -- rather than only an error like
    /// [`Self::is_error`].
    ///
    /// The generic twin of `eval::QueryResult::is_escape`, and needed for
    /// the same reason: it answers "must the caller stop pulling?" without
    /// consuming the result, which is what lets [`fanout_arg_generic`]
    /// decide whether a `body` result can be buffered for its single-output
    /// fast path before it has been flattened (#1531).
    fn is_escape(&self) -> bool {
        matches!(
            self,
            Self::Error(_) | Self::Break(_) | Self::Halt(_) | Self::Partial(_, _)
        )
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
    /// - `indent`: indentation width/unit (`IndentSpec::COMPACT` for compact)
    /// - `sort_keys`: sort mapping/object keys before writing (`-S`/`--sort-keys`)
    /// - `numbers`: which value-formatting convention to use (#1576) — see
    ///   [`JsonConvention`]'s own doc comment. `yq_runner.rs` always passes
    ///   `Preserve`; `jq_runner.rs`'s own M2 fast path passes `JqCompat`
    ///   unless `--preserve-input` is set.
    ///
    /// Returns the number of values streamed and whether the last was falsy.
    pub fn stream_json<W: core::fmt::Write>(
        &self,
        out: &mut W,
        indent: IndentSpec,
        sort_keys: bool,
        numbers: JsonConvention,
        mut on_value: impl FnMut(&mut W) -> core::fmt::Result,
    ) -> Result<crate::jq::stream::StreamStats, core::fmt::Error> {
        use crate::jq::stream::{StreamStats, StreamableValue};

        let mut stats = StreamStats::default();

        match self {
            Self::One(v) => {
                // Convert to owned for streaming. A decode failure takes the
                // same route as `Self::Error` below -- nothing reaches `out`,
                // the diagnostic goes back for stderr and the exit code
                // (#355, #1247) -- rather than streaming a silent `null`.
                let Some(owned) = owned_or_stream_error(to_owned(v), &mut stats) else {
                    return Ok(stats);
                };
                owned.stream_json(out, indent, sort_keys, numbers)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy(numbers);
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait
                if let Err(e) = c.stream_json(out, indent, sort_keys, numbers) {
                    absorb_stream_failure(e, &mut stats)?;
                    return Ok(stats);
                }
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy(numbers);
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::Many(vs) => {
                for (i, v) in vs.iter().enumerate() {
                    // Keep the outputs already streamed and report the
                    // failure, the same shape `Partial` uses for a mid-stream
                    // `Control` (#400/#494, #1247) -- `count` is how many
                    // actually reached `out`, not how many were asked for.
                    let Some(owned) = owned_or_stream_error(to_owned(v), &mut stats) else {
                        stats.count = i;
                        return Ok(stats);
                    };
                    owned.stream_json(out, indent, sort_keys, numbers)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy(numbers);
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = vs.len();
            }
            Self::LazyKeys {
                fields,
                sorted,
                collapse,
            } => {
                if *sorted {
                    // Fallback: materialize+sort. Sorting requires seeing
                    // every key first, so this can't stream lazily like the
                    // unsorted case below.
                    let Some(owned) = owned_or_stream_error(
                        materialize_lazy_keys::<V>(fields, true, *collapse),
                        &mut stats,
                    ) else {
                        return Ok(stats);
                    };
                    owned.stream_json(out, indent, sort_keys, numbers)?;
                } else {
                    // Genuinely lazy (#685): each key is pulled from
                    // `fields` and written straight to `out` as it's
                    // produced — no `Vec<String>` or `OwnedValue::Array` is
                    // ever built. Reachable from `yq_runner.rs`'s M2 fast
                    // path now that `can_use_m2_streaming` admits
                    // `Builtin::KeysUnsorted`. `sort_keys` (`-S`) is a no-op
                    // here: this is a flat array of key names, not a
                    // mapping, so there's nothing for `-S` to reorder.
                    //
                    // #1679: a #1194 key stops the writer mid-stream rather
                    // than silently dropping it; whatever keys already
                    // reached `out` stay written (same `Partial` idiom as
                    // `owned_or_stream_error` above), and the failure
                    // travels back via `stats.error` instead.
                    let mut malformed_key = None;
                    crate::jq::stream::stream_lazy_keys_json(
                        fields,
                        *collapse,
                        out,
                        indent,
                        &mut malformed_key,
                    )?;
                    if let Some(e) = malformed_key {
                        stats.error = Some(stream_error(&e));
                        return Ok(stats);
                    }
                }
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = false;
                stats.any_truthy = true;
            }
            // The actual #684 win: writes `[0,1,...,len-1]` straight to
            // `out`, no `Vec<OwnedValue>`/`OwnedValue::Array` ever built.
            // Arrays are always truthy in jq, even `[]` (only `null`/`false`
            // are falsy), so `last_was_falsy` is unconditionally `false`.
            Self::LazyIndexRange(len) => {
                write_index_range_json(out, *len, indent)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = false;
                stats.any_truthy = true;
            }
            // Still not a byte-at-a-time writer like `LazyKeys`'s unsorted
            // arm above, and it can't be: `map`'s array construction is
            // atomic in real jq (`[1,2,"x"]|map(.+1)` prints nothing to
            // stdout, only the stderr diagnostic), so a writer that emitted
            // as it pulled would already have flushed `[1,2,` before
            // discovering element 3 fails — wrong, and unfixable after the
            // fact.
            //
            // `drain_atomic` (#757) is what buys the rest of the win anyway:
            // it settles the whole chain's success up front, holding only
            // `LazyElem`s (a `V::Cursor` is `Copy` and pointer-sized) rather
            // than the deep `OwnedValue` copy `materialize_atomic` builds.
            // Elements that are still live cursors then render straight from
            // the source document, which is what keeps duplicate mapping
            // keys, comments, anchors and flow style through `map` the way
            // `.[]` already does — see `can_use_m2_streaming`'s `Builtin::Map`
            // arm (yq_runner.rs) for what actually routes a query here.
            Self::LazySeq(seq) => match seq.clone().drain_atomic() {
                Ok(items) => {
                    let owned = match sequence_streamable_cursors(&items) {
                        Some(cursors) => {
                            if let Err(e) = V::Cursor::stream_sequence_json(
                                &cursors, out, indent, sort_keys, numbers,
                            ) {
                                absorb_stream_failure(e, &mut stats)?;
                                return Ok(stats);
                            }
                            true
                        }
                        None => match items
                            .iter()
                            .map(lazy_elem_to_owned)
                            .collect::<Result<Vec<_>, _>>()
                        {
                            Ok(items) => {
                                OwnedValue::Array(items)
                                    .stream_json(out, indent, sort_keys, numbers)?;
                                true
                            }
                            Err(e) => {
                                stats.error = Some(stream_error(&e));
                                false
                            }
                        },
                    };
                    if owned {
                        on_value(out)?;
                        stats.count = 1;
                        // A `map` result is always an array, and every array is
                        // truthy in jq — only `null`/`false` are falsy — so this
                        // needs no per-value check (same reasoning as the
                        // `LazyIndexRange` arm above, which is also always an
                        // array).
                        stats.last_was_falsy = false;
                        stats.any_truthy = true;
                    }
                }
                Err(control) => {
                    let (error, halt) = control_to_stream_outcome(&control);
                    stats.error = error;
                    stats.halt = halt;
                }
            },
            Self::ManyCursor(cs) => {
                for (i, c) in cs.iter().enumerate() {
                    // `count` is how many results actually reached `out`, not
                    // how many were asked for -- same contract as `Many`'s own
                    // mid-stream failure above (#400/#494, #1247, #1615).
                    if let Err(e) = c.stream_json(out, indent, sort_keys, numbers) {
                        absorb_stream_failure(e, &mut stats)?;
                        stats.count = i;
                        return Ok(stats);
                    }
                    on_value(out)?;
                    stats.last_was_falsy = c.is_falsy(numbers);
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
                o.stream_json(out, indent, sort_keys, numbers)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = o.is_falsy(numbers);
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::ManyOwned(os) => {
                for o in os {
                    o.stream_json(out, indent, sort_keys, numbers)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy(numbers);
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
            }
            Self::Break(label) => {
                stats.error = Some(crate::jq::stream::StreamError {
                    message: format!("break ${label} not in label"),
                    not_a_string: false,
                });
            }
            Self::Halt(code) => {
                stats.halt = Some(*code);
            }
            // The prefix streams like `ManyOwned` above, then the control is
            // reported the same way `Error`/`Break` are (#400, #494) — the
            // outputs already produced no longer vanish behind the failure.
            Self::Partial(os, control) => {
                for o in os {
                    o.stream_json(out, indent, sort_keys, numbers)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy(numbers);
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
                let (error, halt) = control_to_stream_outcome(control);
                stats.error = error;
                stats.halt = halt;
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
        indent: IndentSpec,
        sort_keys: bool,
        mut on_value: impl FnMut(&mut W) -> core::fmt::Result,
    ) -> Result<crate::jq::stream::StreamStats, core::fmt::Error> {
        use crate::jq::stream::{StreamStats, StreamableValue};

        let mut stats = StreamStats::default();

        match self {
            Self::One(v) => {
                // See `stream_json`'s own `One` arm (#355, #1247).
                let Some(owned) = owned_or_stream_error(to_owned(v), &mut stats) else {
                    return Ok(stats);
                };
                owned.stream_yaml(out, indent, sort_keys)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy(JsonConvention::Preserve);
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait.
                // `stream_yaml_as_document` (not the bare `stream_yaml`): a
                // navigated container result keeps its own trailing comment
                // just like the whole document does, unlike a navigated
                // scalar (#793a).
                if let Err(e) = c.stream_yaml_as_document(out, indent, sort_keys) {
                    absorb_stream_failure(e, &mut stats)?;
                    return Ok(stats);
                }
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy(JsonConvention::Preserve);
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::Many(vs) => {
                for (i, v) in vs.iter().enumerate() {
                    // Keep the outputs already streamed and report the
                    // failure, the same shape `Partial` uses for a mid-stream
                    // `Control` (#400/#494, #1247) -- `count` is how many
                    // actually reached `out`, not how many were asked for.
                    let Some(owned) = owned_or_stream_error(to_owned(v), &mut stats) else {
                        stats.count = i;
                        return Ok(stats);
                    };
                    owned.stream_yaml(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy(JsonConvention::Preserve);
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = vs.len();
            }
            Self::ManyCursor(cs) => {
                for (i, c) in cs.iter().enumerate() {
                    // See `OneCursor` above: each streamed result keeps its
                    // own trailing comment if it's a container (#793a). See
                    // the JSON twin on why `count` is set to `i` here (#1615).
                    if let Err(e) = c.stream_yaml_as_document(out, indent, sort_keys) {
                        absorb_stream_failure(e, &mut stats)?;
                        stats.count = i;
                        return Ok(stats);
                    }
                    on_value(out)?;
                    stats.last_was_falsy = c.is_falsy(JsonConvention::Preserve);
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = cs.len();
            }
            Self::LazyKeys {
                fields,
                sorted,
                collapse,
            } => {
                if *sorted {
                    // Fallback: materialize+sort. See `stream_json`'s
                    // `LazyKeys` arm above — same reasoning.
                    let Some(owned) = owned_or_stream_error(
                        materialize_lazy_keys::<V>(fields, true, *collapse),
                        &mut stats,
                    ) else {
                        return Ok(stats);
                    };
                    owned.stream_yaml(out, indent, sort_keys)?;
                } else {
                    // Genuinely lazy (#685): see `stream_json`'s
                    // `LazyKeys` arm above — same reasoning, YAML target.
                    // #1679: see `stream_json`'s `LazyKeys` arm above —
                    // same reasoning.
                    let mut malformed_key = None;
                    crate::jq::stream::stream_lazy_keys_yaml(
                        fields,
                        *collapse,
                        out,
                        indent,
                        &mut malformed_key,
                    )?;
                    if let Some(e) = malformed_key {
                        stats.error = Some(stream_error(&e));
                        return Ok(stats);
                    }
                }
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = false;
                stats.any_truthy = true;
            }
            // Same allocation-free approach as `stream_json` above (#684).
            Self::LazyIndexRange(len) => {
                write_index_range_yaml(out, *len, indent)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = false;
                stats.any_truthy = true;
            }
            // Same atomicity and cursor-streaming reasoning as
            // `stream_json`'s `LazySeq` arm above — read it there; this is
            // the identical shape with the YAML writers substituted.
            Self::LazySeq(seq) => match seq.clone().drain_atomic() {
                Ok(items) => {
                    let owned = match sequence_streamable_cursors(&items) {
                        Some(cursors) => {
                            if let Err(e) =
                                V::Cursor::stream_sequence_yaml(&cursors, out, indent, sort_keys)
                            {
                                absorb_stream_failure(e, &mut stats)?;
                                return Ok(stats);
                            }
                            true
                        }
                        None => match items
                            .iter()
                            .map(lazy_elem_to_owned)
                            .collect::<Result<Vec<_>, _>>()
                        {
                            Ok(items) => {
                                OwnedValue::Array(items).stream_yaml(out, indent, sort_keys)?;
                                true
                            }
                            Err(e) => {
                                stats.error = Some(stream_error(&e));
                                false
                            }
                        },
                    };
                    if owned {
                        on_value(out)?;
                        stats.count = 1;
                        stats.last_was_falsy = false;
                        stats.any_truthy = true;
                    }
                }
                Err(control) => {
                    let (error, halt) = control_to_stream_outcome(&control);
                    stats.error = error;
                    stats.halt = halt;
                }
            },
            Self::None => {
                // No output
            }
            Self::Error(e) => {
                // See `stream_json`: diagnostics never go to `out` (#355).
                stats.error = Some(stream_error(e));
            }
            Self::Owned(o) => {
                o.stream_yaml(out, indent, sort_keys)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = o.is_falsy(JsonConvention::Preserve);
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::ManyOwned(os) => {
                for o in os {
                    o.stream_yaml(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy(JsonConvention::Preserve);
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
            }
            Self::Break(label) => {
                stats.error = Some(crate::jq::stream::StreamError {
                    message: format!("break ${label} not in label"),
                    not_a_string: false,
                });
            }
            Self::Halt(code) => {
                stats.halt = Some(*code);
            }
            // Same treatment as `stream_json` (#400, #494): the prefix
            // streams first, then the control is reported.
            Self::Partial(os, control) => {
                for o in os {
                    o.stream_yaml(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy(JsonConvention::Preserve);
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = os.len();
                let (error, halt) = control_to_stream_outcome(control);
                stats.error = error;
                stats.halt = halt;
            }
        }

        Ok(stats)
    }
}

/// Stream a `GenericResult::LazyIndexRange(len)` as JSON — `[0, 1, ...,
/// len-1]` — writing each index straight to `out` with no intermediate
/// `Vec<OwnedValue>`/`OwnedValue::Array` at all (#684). Mirrors the array arm
/// of `stream_owned_value_json_with` (`src/jq/stream.rs`) for indentation, but
/// since every element is a plain `usize` there's no need for that function's
/// per-element type dispatch or escaping.
fn write_index_range_json<W: core::fmt::Write>(
    out: &mut W,
    len: usize,
    indent: IndentSpec,
) -> core::fmt::Result {
    if len == 0 {
        return out.write_str("[]");
    }
    out.write_char('[')?;
    for i in 0..len {
        if i > 0 {
            out.write_char(',')?;
        }
        if indent.width > 0 {
            out.write_char('\n')?;
            for _ in 0..indent.width {
                out.write_char(indent.unit)?;
            }
        }
        write!(out, "{i}")?;
    }
    if indent.width > 0 {
        out.write_char('\n')?;
    }
    out.write_char(']')
}

/// Stream a `GenericResult::LazyIndexRange(len)` as YAML, same allocation-free
/// approach as [`write_index_range_json`]. Mirrors the array arm of
/// `stream_owned_value_yaml` (`src/jq/stream.rs`): flow style (`[0, 1, ...]`)
/// in compact mode, block style (`- 0\n- 1\n...`) otherwise.
fn write_index_range_yaml<W: core::fmt::Write>(
    out: &mut W,
    len: usize,
    indent: IndentSpec,
) -> core::fmt::Result {
    if len == 0 {
        return out.write_str("[]");
    }
    if indent.is_compact() {
        out.write_char('[')?;
        for i in 0..len {
            if i > 0 {
                out.write_str(", ")?;
            }
            write!(out, "{i}")?;
        }
        out.write_char(']')
    } else {
        for i in 0..len {
            if i > 0 {
                out.write_char('\n')?;
            }
            out.write_str("- ")?;
            write!(out, "{i}")?;
        }
        Ok(())
    }
}

/// Render an [`EvalError`] into the payload a streaming caller needs to
/// reproduce jq's diagnostic on stderr (#355).
/// Route a fallible materialization into [`StreamStats::error`], returning
/// `None` when it failed so the caller can stop streaming.
///
/// Streaming never writes a diagnostic to `out` -- `out` is stdout, where it
/// would be indistinguishable from a result -- so a decode failure (#1247)
/// has to travel back in `stats` exactly as `GenericResult::Error` already
/// does (#355).
fn owned_or_stream_error(
    result: Result<OwnedValue, EvalError>,
    stats: &mut crate::jq::stream::StreamStats,
) -> Option<OwnedValue> {
    match result {
        Ok(owned) => Some(owned),
        Err(e) => {
            stats.error = Some(stream_error(&e));
            None
        }
    }
}

/// Route a [`StreamFailure`](crate::jq::stream::StreamFailure) from a cursor
/// streamer into `stats`, the way #1679's `LazyKeys` arm routes a malformed
/// key (#1615).
///
/// A decode failure is a *data* diagnostic: it belongs on stderr with an exit
/// code, so it goes back through `stats.error` and whatever already reached
/// `out` stays written (the `Partial` idiom). A genuine writer failure is not
/// diagnosable and propagates as before. Returns `true` when the caller should
/// stop and hand `stats` back immediately.
fn absorb_stream_failure(
    e: crate::jq::stream::StreamFailure,
    stats: &mut crate::jq::stream::StreamStats,
) -> Result<bool, core::fmt::Error> {
    match e {
        crate::jq::stream::StreamFailure::Decode(err) => {
            stats.error = Some(stream_error(&err));
            // The cursor stream was cut off part-way through a value, unlike
            // an ordinary evaluation error which writes nothing (#1615).
            stats.truncated = true;
            Ok(true)
        }
        crate::jq::stream::StreamFailure::Fmt => Err(core::fmt::Error),
    }
}

fn stream_error(e: &EvalError) -> crate::jq::stream::StreamError {
    crate::jq::stream::StreamError {
        message: e.message.clone(),
        not_a_string: e.payload_is_not_a_string(),
    }
}

/// Split a terminating [`Control`] into the `(error, halt)` pair
/// [`crate::jq::stream::StreamStats`] carries.
///
/// A halt must not become a `StreamError`: that channel is a rendered
/// message string with no room for the real exit code, so a caller that only
/// checked `error` would report it as an ordinary uncaught failure instead of
/// halting with the right code (#791) — see `StreamStats::halt`'s doc
/// comment. Shared by every `stream_json`/`stream_yaml` site that terminates
/// on a `Control` (`Break`'s message is duplicated here too, purely to keep
/// all three arms of the match in one place rather than splitting `Break`
/// out on its own).
fn control_to_stream_outcome(
    control: &Control,
) -> (Option<crate::jq::stream::StreamError>, Option<i32>) {
    match control {
        Control::Error(e) => (Some(stream_error(e)), None),
        Control::Break(label) => (
            Some(crate::jq::stream::StreamError {
                message: format!("break ${label} not in label"),
                not_a_string: false,
            }),
            None,
        ),
        Control::Halt(code) => (None, Some(*code)),
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
///
/// Gated on `input`/`inputs`/`input_line_number` appearing anywhere in `expr`
/// (#1504): a top-level `Comma`/`Pipe` that never touches the shared input
/// queue behaves identically either way, so the eager `eval_single` path
/// (and its cursor/zero-copy fast paths) stays the default; only a filter
/// that actually shares state with `jq_runner`'s input queue pays for the
/// re-index round trip through `eval_each_owned_collect`, which re-serialises
/// and re-indexes on top of the index the caller already built. That cost is
/// per document and scales with document size -- an interleaved spot check
/// put it near 1.7x wall clock; see `docs/compliance/jq/limitations.md` for
/// the numbers and what they are and aren't worth.
///
/// **Carved back out for cursor-metadata builtins.** The bridge hands the
/// program to `eval.rs`, which answers `line`/`column`/`document_index`/
/// `anchor`/`style`/`line_comment` from fixed-default stubs and rejects
/// `at_offset`/`at_position` outright -- all eight answers live only in this
/// module's `Option<V::Cursor>` threading. Re-indexing cannot rescue
/// them either: `eval_each_owned` rebuilds from re-serialised text, so any
/// offset or line/column it could report would describe that text rather
/// than the file the user passed. So a program that mixes an input builtin
/// with a cursor-metadata builtin keeps the eager path and keeps its cursor,
/// at the cost of #1504's interleave -- a divergence, where the bridge would
/// instead give a wrong answer or an error. See
/// [`crate::jq::walk::uses_cursor_metadata_builtins`].
pub fn eval_using<S: EvalSemantics, V: DocumentValue>(expr: &Expr, value: V) -> GenericResult<V> {
    if takes_input_queue_bridge(expr) {
        let owned = owned_or_err!(to_owned_with_cursor(&value, None));
        return eval_each_owned_collect::<S, V>(expr, &owned, false);
    }
    eval_single::<S, V>(expr, value, false, None)
}

/// Whether `expr` should be handed to `eval.rs`'s demand-driven evaluator
/// instead of this module's eager `eval_single` (#1504).
///
/// One definition shared by [`eval_using`] and [`eval_with_cursor_using`], so
/// the two top-level entry points cannot drift apart on which programs take
/// the bridge. `input_queue_is_active` (one TLS load) comes first so yq mode,
/// library embedders and every jq filter that never mentions an input builtin
/// pay nothing for the two AST walks behind it.
///
/// See [`eval_using`]'s doc comment for why the cursor-metadata carve-out is
/// part of the condition rather than something the bridge could handle.
fn takes_input_queue_bridge(expr: &Expr) -> bool {
    crate::jq::input_queue_is_active()
        && crate::jq::walk::uses_input_builtins(expr)
        && !crate::jq::walk::uses_cursor_metadata_builtins(expr)
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
///
/// Same `takes_input_queue_bridge` condition as [`eval_using`] (#1504),
/// cursor-metadata carve-out included; see its doc comment for why the
/// bridge is gated rather than unconditional.
pub fn eval_with_cursor_using<S: EvalSemantics, C: DocumentCursor>(
    expr: &Expr,
    cursor: C,
) -> GenericResult<C::Value> {
    if takes_input_queue_bridge(expr) {
        // `to_owned_cursor` directly rather than `to_owned_with_cursor`: the
        // latter ignores its value argument whenever the cursor is `Some`, so
        // routing through it would compute a `cursor.value()` only to drop it.
        let owned = owned_or_err!(to_owned_cursor(&cursor));
        return eval_each_owned_collect::<S, C::Value>(expr, &owned, false);
    }
    eval_single::<S, C::Value>(expr, cursor.value(), false, Some(cursor))
}

/// Streaming counterpart of [`eval_with_cursor`] (#1653): deliver each output
/// to `on_value` as soon as it is produced, instead of collecting a whole
/// input's results into a `Vec` first.
///
/// Real jq's evaluator is a lazy generator, so a filter that both writes to
/// stdout and triggers a stderr side effect (`debug`, `stderr`, `halt_error`)
/// or raises mid-stream interleaves the two in real time:
///
/// ```console
/// $ jq --unbuffered -cn '1, debug, 2'   2>&1
/// 1
/// ["DEBUG:",null]
/// null
/// 2
/// ```
///
/// Collecting first cannot reproduce that ordering *however the writes are
/// buffered*, because every stderr write has already happened by the time the
/// first stdout write runs -- which is why `--unbuffered`'s per-write
/// `flush()` alone never fixed it.
///
/// `on_value` returns `false` to stop the generator early (the CLI uses this
/// for a write error). Each output arrives as a single-output
/// [`GenericResult`], so every consumer's existing per-result handling --
/// including the lazy `LazyKeys`/`LazyIndexRange`/`LazySeq` variants -- keeps
/// working unchanged. Items arrive via the same `generic_item_to_result`
/// conversion every other sink consumer uses (private, so named rather than
/// linked -- a public item may not link to it).
///
/// Returns `Some(control)` iff evaluation terminated in a control (an uncaught
/// error, `break`, or `halt`); everything produced *before* it has already
/// been delivered, matching jq, which writes `1` to stdout before reporting
/// the error for `1, error("x"), 3`.
pub fn eval_each_with_cursor<C: DocumentCursor>(
    expr: &Expr,
    cursor: C,
    on_value: &mut dyn FnMut(GenericResult<C::Value>) -> bool,
) -> Option<Control> {
    eval_each_with_cursor_using::<JqSemantics, C>(expr, cursor, on_value)
}

/// Streaming counterpart of [`eval_with_cursor_using`] (#1653).
///
/// Same `takes_input_queue_bridge` gate as the eager entry point (#1504), but
/// it does *not* cost the streaming: the bridge needs the input owned, not the
/// outputs collected, so that branch drives `eval_each_owned` -- itself
/// demand-driven -- with this function's own sink rather than
/// `eval_each_owned_collect`'s accumulating one. `jq -n 'inputs, debug'`
/// interleaves like jq because of this branch, not despite it.
pub fn eval_each_with_cursor_using<S: EvalSemantics, C: DocumentCursor>(
    expr: &Expr,
    cursor: C,
    on_value: &mut dyn FnMut(GenericResult<C::Value>) -> bool,
) -> Option<Control> {
    if takes_input_queue_bridge(expr) {
        let owned = match to_owned_cursor(&cursor) {
            Ok(o) => o,
            Err(e) => return Some(Control::Error(e)),
        };
        let mut owned_sink = |v: OwnedValue| -> Demand {
            if on_value(GenericResult::Owned(v)) {
                Demand::Continue
            } else {
                Demand::Stop
            }
        };
        return match crate::jq::eval::eval_each_owned::<S>(expr, &owned, false, &mut owned_sink) {
            Flow::Exhausted => None,
            // Kept for the same reason as the streaming branch below.
            Flow::Stopped { pending } => pending,
            Flow::Escaped(c) => Some(c),
        };
    }

    let mut sink = |item: GenericItem<C::Value>| -> Demand {
        if on_value(generic_item_to_result::<C::Value>(item)) {
            Demand::Continue
        } else {
            Demand::Stop
        }
    };
    match eval_each_generic::<S, C::Value>(expr, cursor.value(), false, Some(cursor), &mut sink) {
        Flow::Exhausted => None,
        // `pending` is *kept*, unlike every Stage 2 consumer of `Stopped`
        // (`each_take_first`, `each_take_n`, ...), which drop it. Those are
        // early-exit consumers in the middle of a filter, where a control
        // raised past what the consumer asked for must not escape --
        // `first(1, ("BOOM"|halt_error(3)))` is `1`, exit 0. This is the
        // opposite position: the final consumer, where a control that *was*
        // already raised still has to reach the exit code. Dropping it here
        // would silently lose a `halt`'s code, matching `resolve_leaf`'s own
        // reasoning (#987) rather than the early-exit consumers'.
        Flow::Stopped { pending } => pending,
        Flow::Escaped(c) => Some(c),
    }
}

/// Fold an already-evaluated first pipe stage through every remaining stage,
/// eagerly. Extracted verbatim from `eval_single`'s own `Expr::Pipe` arm
/// (`current` is `exprs[0]`'s own result, `stages` is `exprs[1..]`) so
/// `eval_each_pipe_generic`'s lazy `LazyKeys`/`LazyIndexRange`/`LazySeq`
/// items (#1461) can reuse this per-`GenericResult`-variant switch --
/// including its `map`/`select`/`first`/`.[n]` composability fast paths
/// (#724/#725) -- instead of duplicating it.
fn fold_pipe_stages<S: EvalSemantics, V: DocumentValue>(
    mut current: GenericResult<V>,
    stages: &[Expr],
    optional: bool,
) -> GenericResult<V> {
    for (j, expr) in stages.iter().enumerate() {
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
                    | GenericResult::Break(_)
                    | GenericResult::Halt(_)) => p,
                    GenericResult::None => partial_generic(Vec::new(), outer_control),
                    GenericResult::ManyOwned(results) => partial_generic(results, outer_control),
                    _ => unreachable!(
                        "eval_on_many_owned only returns None/ManyOwned/Error/Break/Halt/Partial"
                    ),
                }
            }
            GenericResult::One(v) => eval_single::<S, _>(expr, v, optional, None),
            GenericResult::OneCursor(c) => eval_single::<S, _>(expr, c.value(), optional, Some(c)),
            GenericResult::Many(vs) => {
                let mut results = Vec::new();
                for v in vs {
                    match eval_single::<S, _>(expr, v, optional, None).materialize_lazy() {
                        // A decode failure keeps the prefix already
                        // piped through, exactly as the `Error` arm
                        // below does (#400/#494, #1247) -- not
                        // `owned_or_err!`, which would discard it.
                        GenericResult::One(r) => match to_owned(&r) {
                            Ok(o) => results.push(o),
                            Err(e) => return partial_generic(results, Control::Error(e)),
                        },
                        GenericResult::OneCursor(c) => match to_owned_cursor(&c) {
                            Ok(o) => results.push(o),
                            Err(e) => return partial_generic(results, Control::Error(e)),
                        },
                        GenericResult::Many(rs) => {
                            for r in &rs {
                                match to_owned(r) {
                                    Ok(o) => results.push(o),
                                    Err(e) => return partial_generic(results, Control::Error(e)),
                                }
                            }
                        }
                        GenericResult::ManyCursor(cs) => {
                            for c in &cs {
                                match to_owned_cursor(c) {
                                    Ok(o) => results.push(o),
                                    Err(e) => return partial_generic(results, Control::Error(e)),
                                }
                            }
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
                        GenericResult::Halt(code) => {
                            return partial_generic(results, Control::Halt(code));
                        }
                        GenericResult::Partial(vs2, control) => {
                            results.extend(vs2);
                            return partial_generic(results, control);
                        }
                        GenericResult::LazyKeys { .. }
                        | GenericResult::LazyIndexRange(_)
                        | GenericResult::LazySeq(_) => {
                            unreachable!("materialize_lazy() already normalized every lazy variant")
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
                let rest = Expr::Pipe(stages[j..].to_vec());
                let mut per_element = vec_with_capacity(cs.len());
                for c in cs {
                    match eval_single::<S, _>(&rest, c.value(), optional, Some(c)) {
                        // The elements already piped through no
                        // longer vanish (#400, #494). If an earlier
                        // element's own buffered `LazySeq` also fails
                        // once `flatten_generic_results` materializes
                        // it, that earlier failure wins -- it's
                        // chronologically first in evaluation order.
                        GenericResult::Error(e) => {
                            return match flatten_generic_results(per_element) {
                                Ok(prefix) => partial_generic(prefix, Control::Error(e)),
                                Err(Control::Error(earlier)) => GenericResult::Error(earlier),
                                Err(Control::Break(label)) => GenericResult::Break(label),
                                Err(Control::Halt(code)) => GenericResult::Halt(code),
                            };
                        }
                        GenericResult::Break(label) => {
                            return match flatten_generic_results(per_element) {
                                Ok(prefix) => partial_generic(prefix, Control::Break(label)),
                                Err(Control::Error(earlier)) => GenericResult::Error(earlier),
                                Err(Control::Break(earlier_label)) => {
                                    GenericResult::Break(earlier_label)
                                }
                                Err(Control::Halt(code)) => GenericResult::Halt(code),
                            };
                        }
                        // Same immediate-stop treatment as `Error`/`Break`
                        // above — halt is an opaque terminal signal, not
                        // catchable by anything downstream, so this must
                        // not fall through to the `other => ...` wildcard
                        // below (which would keep evaluating later cursor
                        // elements instead of stopping immediately).
                        GenericResult::Halt(code) => {
                            return match flatten_generic_results(per_element) {
                                Ok(prefix) => partial_generic(prefix, Control::Halt(code)),
                                Err(Control::Error(earlier)) => GenericResult::Error(earlier),
                                Err(Control::Break(label)) => GenericResult::Break(label),
                                Err(Control::Halt(earlier_code)) => {
                                    GenericResult::Halt(earlier_code)
                                }
                            };
                        }
                        GenericResult::Partial(vs, control) => {
                            return match flatten_generic_results(per_element) {
                                Ok(mut prefix) => {
                                    prefix.extend(vs);
                                    partial_generic(prefix, control)
                                }
                                Err(Control::Error(earlier)) => GenericResult::Error(earlier),
                                Err(Control::Break(label)) => GenericResult::Break(label),
                                Err(Control::Halt(code)) => GenericResult::Halt(code),
                            };
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
                    match flatten_generic_results(per_element) {
                        Ok(results) if results.is_empty() => GenericResult::None,
                        Ok(results) => GenericResult::ManyOwned(results),
                        Err(Control::Error(e)) => GenericResult::Error(e),
                        Err(Control::Break(label)) => GenericResult::Break(label),
                        Err(Control::Halt(code)) => GenericResult::Halt(code),
                    }
                };
            }
            // The whole point of `LazyKeys`: `length` always, and
            // `.[]`, `.[n]`, `first`, and `last` when `!sorted`, all
            // answer directly from the field iterator (no decode, no
            // `Vec`, no reserialize/reindex round-trip) by mirroring
            // the same shapes' handling for real arrays elsewhere in
            // this file (`Expr::Index`/`Expr::Iterate` above,
            // `Builtin::First`/`Builtin::Last`/`Builtin::Length` in
            // `eval_builtin`). `map`/`select` and everything else
            // fall back to materializing exactly as eager `keys`/
            // `keys_unsorted` did — `eval_generic.rs` has no native
            // lazy `map`/`select` for *any* value today (even a
            // materialized array's `map` round-trips through
            // `eval_on_owned`), so there's no cheap win available
            // here without a broader, unrelated architecture change.
            GenericResult::LazyKeys {
                fields,
                sorted,
                collapse,
            } => fold_lazy_keys_stage::<S, V>(fields, sorted, collapse, expr, optional),
            // The array counterpart of `LazyKeys` above
            // (#684): the index range `[0, 1, ..., len-1]` is fully
            // determined by `len` alone, so `length`, `.[]`, `.[n]`,
            // `first`, and `last` are plain arithmetic on `len` — no
            // allocation at all, not even a `Vec<V::Cursor>` (there's
            // no cursor to point at: array-index "keys" are
            // synthetic, not bytes in the source document).
            GenericResult::LazyIndexRange(len) => {
                fold_lazy_index_range_stage::<S, V>(len, expr, optional)
            }
            // The composability engine (#724, #725): every further
            // `| map(g)` stage just pushes onto the same chain
            // (self-recursive by construction — an arbitrary-length
            // `map(f) | map(g) | map(h)` stays one `LazySeq`, not one
            // type per depth). A handful of consumers get a genuine
            // single-forward-pass native fast path; everything else
            // materializes once (`materialize_atomic`) and hands off
            // to the full evaluator — still one pass, not the
            // original four-pass round trip.
            GenericResult::LazySeq(seq) => fold_lazy_seq_stage::<S, V>(seq, expr, optional),
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
            GenericResult::Halt(code) => return GenericResult::Halt(code),
        };
    }

    current
}

/// One step of folding a `GenericResult::LazyKeys` through a single further
/// pipe stage. Extracted verbatim from `fold_pipe_stages`'s own `LazyKeys`
/// arm (#1565) so [`fold_pipe_stages_sink`] can share the exact same
/// composability fast paths (`length`, `.[]`, `.[n]`, `first`, `last`,
/// `map` when `!sorted`) instead of duplicating them -- only the eager
/// `fold_pipe_stages` and the demand-aware `fold_pipe_stages_sink` differ in
/// how they consume an `Expr::Iterate` result, and `fold_pipe_stages_sink`
/// intercepts that case before ever calling this function (see its own
/// per-lazy-variant element iteration instead).
fn fold_lazy_keys_stage<S: EvalSemantics, V: DocumentValue>(
    fields: V::Fields,
    sorted: bool,
    collapse: bool,
    expr: &Expr,
    optional: bool,
) -> GenericResult<V> {
    match unwrap_paren(expr) {
        // #1514: the collapse probe used to run in guard
        // position, ahead of this match, so every arm paid a
        // full `document::census` -- including the ones that
        // never needed the answer (`first`, and the
        // materializing fallback, which applies the rule
        // itself through `effective_keys`) and the ones that
        // can settle it during a walk they were making
        // anyway. It now runs only where the answer is read
        // *positionally*, which is the only shape that cannot
        // be decided as the walk goes.
        //
        // Order-independent for both `keys` and
        // `keys_unsorted` — the one fast path #683 adds for
        // sorted `keys`. `effective_len_checked` counts
        // distinct keys in one walk without materializing the
        // field list, and is a plain counting walk outright
        // when `collapse` is false.
        //
        // `_checked` for the same reason `Builtin::Length`'s own object arm
        // uses it: this is the *other* spelling of "how many members does
        // this object have", and while it answered from the unchecked
        // `effective_len`, `{invalid} | length` raised while `{invalid} |
        // keys | length` said `0` -- the two-answers-for-one-document split
        // #1385's postmortem names, reintroduced one pipe stage further
        // along. The check costs nothing extra: it rides that same walk
        // (#1194).
        Expr::Builtin(Builtin::Length) => match effective_len_checked(&fields, collapse) {
            Ok(len) => GenericResult::Owned(OwnedValue::Int(len as i64)),
            Err(err) => GenericResult::Error(err),
        },
        // Document order is a valid answer only for
        // `keys_unsorted`. `keys` needs lexicographic order
        // for these and falls through to the shared
        // materialize-(and-sort) fallback below. Do not drop
        // the `if !sorted` guard on a new arm here without
        // re-deriving why document order would still be a
        // correct answer.
        // #1629: unlike `first`/`.[0]` below, this arm already walks the
        // whole object to collect every cursor, so the #1194/#1677 checks
        // (`distinct_key_cursors_checked`) ride along for free -- same
        // reasoning as `Builtin::Length`'s arm above.
        Expr::Iterate if !sorted => match distinct_key_cursors_checked::<V>(&fields, collapse) {
            Ok(cursors) if cursors.is_empty() => GenericResult::None,
            Ok(cursors) => GenericResult::ManyCursor(cursors),
            Err(err) => GenericResult::Error(err),
        },
        // `.[0]` is `first` by another spelling, so it takes
        // the `Builtin::First` arm's reasoning below rather
        // than the general positional one: collapsing keeps a
        // repeated key at its *first* position, so field 0 in
        // document order is the answer whatever the rest of
        // the object turns out to hold. Only a *non-zero*
        // index can be displaced by an earlier key collapsing
        // away, which is why the arm below still probes
        // (#1599 -- #1514 moved the probe off the guard, but
        // left this spelling paying it).
        Expr::Index { idx, .. } if !sorted && *idx == 0 => match fields.uncons_key() {
            Some((_, key_cursor, _)) => GenericResult::OneCursor(key_cursor),
            None => GenericResult::Owned(OwnedValue::Null),
        },
        // #1629: a negative index always has to walk the whole object
        // anyway (to normalize against its length), so the #1194 check
        // rides along for free -- same reasoning as `Expr::Iterate` above,
        // and indeed the same helper: `distinct_key_cursors_checked` needs
        // no value decoding (unlike `effective_fields_checked`, an earlier
        // version of this arm's choice -- this arm only ever reads a
        // cursor, never a field's value). This mirrors real jq's own rule:
        // it can't even *parse* an object with a malformed member, so every
        // access into it raises, not just the ones that happen to touch the
        // bad member (verified live against pinned jq 1.7.1:
        // `{"a":1,"b":2,123:3} | keys_unsorted[0]` raises the same parse
        // error as `keys_unsorted[2]` would).
        Expr::Index { idx, .. } if !sorted && *idx < 0 => {
            match distinct_key_cursors_checked::<V>(&fields, collapse) {
                Ok(cursors) => {
                    let target = usize::try_from(cursors.len() as i64 + idx).ok();
                    match target.and_then(|t| cursors.into_iter().nth(t)) {
                        Some(cursor) => GenericResult::OneCursor(cursor),
                        None => GenericResult::Owned(OwnedValue::Null),
                    }
                }
                Err(err) => GenericResult::Error(err),
            }
        }
        // A *positive* index, unlike the negative arm above, does not
        // otherwise need to know the whole object -- it walks only as far
        // as the target position (or, when collapsed, the free probe
        // `collapsed_fields_if` already runs). Adding the #1194 check here
        // would mean walking the rest of the object purely to validate it,
        // undoing exactly the early-exit #1514/#1599 built. Left
        // unchecked, same as `Builtin::First`/`.[0]` below -- see #1629's
        // own accounting of this tradeoff (its "Option 1") and
        // `docs/compliance/jq/limitations.md`.
        Expr::Index { idx, .. } if !sorted => {
            let collapsed = collapsed_fields_if(&fields, collapse);
            if let Some(eff) = collapsed {
                match eff.into_iter().nth(*idx as usize) {
                    Some(field) => GenericResult::OneCursor(field.key_cursor),
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else {
                let target = *idx as usize;
                let mut current = fields;
                let mut found = None;
                let mut i = 0usize;
                while let Some((field, rest)) = current.uncons() {
                    if i == target {
                        found = Some(field.key_cursor);
                        break;
                    }
                    current = rest;
                    i += 1;
                }
                match found {
                    Some(c) => GenericResult::OneCursor(c),
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            }
        }
        // No probe: collapsing keeps every key at its *first*
        // position, and the first field is nobody's repeat, so
        // it survives whatever the rest of the object does.
        //
        // #1629: deliberately still unchecked, same reasoning as the
        // positive-index arm above -- this is the one arm that exists to
        // be genuinely O(1) (#1514/#1599), and a #1194 check would cost
        // strictly more than the answer itself. See
        // `docs/compliance/jq/limitations.md`.
        Expr::Builtin(Builtin::First) if !sorted => match fields.uncons() {
            Some((field, _)) => GenericResult::OneCursor(field.key_cursor),
            None => GenericResult::Owned(OwnedValue::Null),
        },
        // The other positional arm: the last field *is*
        // droppable — if its key repeats an earlier one it
        // collapses away and some other field ends up last.
        //
        // #1629: unlike `First`/`.[0]` above, this arm already walks the
        // whole object regardless (there is no way to find "the last
        // field" without reaching the end), so the #1194 check rides along
        // for free. Tracks only the running last cursor rather than calling
        // `distinct_key_cursors_checked` -- unlike the negative-index arm
        // above, this one never needs random access into the collected
        // list, so collecting one would be a pure O(n)-space cost for an
        // O(1)-space answer (review caught this on the first version of
        // this fix, which called that helper here too). An earlier version
        // still hand-rolled the walk separately per branch and missed the
        // check entirely on the collapsed one (`collapsed_fields_if`
        // returning `Some`, i.e. jq mode with a genuine duplicate key) --
        // also caught by code review before merge. `DistinctKeyCursors`
        // itself already handles collapse=true/false, so there is no
        // branch left to miss.
        Expr::Builtin(Builtin::Last) if !sorted => {
            let mut cursors = DistinctKeyCursors::new(&fields, collapse);
            let mut last_cursor = None;
            for (key, cursor) in cursors.by_ref() {
                if key_is_malformed(&key) {
                    return GenericResult::Error(fields.malformed_member_error());
                }
                last_cursor = Some(cursor);
            }
            // #1956: matches this function's own `distinct_key_cursors_checked`/
            // `keys_are_well_formed` siblings via the same
            // `DistinctKeyCursors::is_malformed` they use.
            if cursors.is_malformed() {
                return GenericResult::Error(fields.malformed_member_error());
            }
            // #2261: same trailing-comma check those two siblings now run
            // too -- this arm already walks the whole object regardless
            // (there is no way to find "the last key" without reaching the
            // end), so it rides along for free, same reasoning as the rest
            // of this arm's own #1956 fix.
            if !cursors.trailing_gap_ok(b'}') {
                return GenericResult::Error(fields.malformed_member_error());
            }
            match last_cursor {
                Some(c) => GenericResult::OneCursor(c),
                None => GenericResult::Owned(OwnedValue::Null),
            }
        }
        // Slice 1 (#724): stay lazy instead of falling
        // through to the `_` materializing fallback below —
        // reuses the composability arm (`GenericResult::LazySeq`
        // below) for everything past this first `map` stage.
        // Same `!sorted` guard as the other fast-path arms
        // above: sorted `keys` still needs a full decode+sort
        // first. `LazySource::keys` carries the collapse rule
        // into the pull itself (#1514), so no probe runs here
        // either.
        Expr::Builtin(Builtin::Map(f)) if !sorted => GenericResult::LazySeq(Box::new(
            LazySeq::new(LazySource::keys(fields, collapse)).push_map(f, S::TAG),
        )),
        // No probe: `materialize_lazy_keys` applies the
        // collapse rule itself, through `effective_keys`.
        _ => match materialize_lazy_keys::<V>(&fields, sorted, collapse) {
            Ok(owned) => eval_on_owned::<S, _>(expr, owned, optional),
            Err(e) => GenericResult::Error(e),
        },
    }
}

/// One step of folding a `GenericResult::LazyIndexRange` through a single
/// further pipe stage. Extracted verbatim from `fold_pipe_stages`'s own
/// `LazyIndexRange` arm (#1565); see [`fold_lazy_keys_stage`]'s own doc
/// comment for why this split exists.
fn fold_lazy_index_range_stage<S: EvalSemantics, V: DocumentValue>(
    len: usize,
    expr: &Expr,
    optional: bool,
) -> GenericResult<V> {
    match unwrap_paren(expr) {
        Expr::Builtin(Builtin::Length) => GenericResult::Owned(OwnedValue::Int(len as i64)),
        Expr::Iterate => {
            if len == 0 {
                GenericResult::None
            } else {
                GenericResult::ManyOwned((0..len).map(|i| OwnedValue::Int(i as i64)).collect())
            }
        }
        Expr::Index { idx, .. } => {
            // Same normalization/OOB-is-null semantics as
            // `LazyKeys`'s `Expr::Index` arm above.
            let target = if *idx < 0 {
                let normalized = len as i64 + idx;
                if normalized < 0 {
                    None
                } else {
                    Some(normalized as usize)
                }
            } else {
                Some(*idx as usize)
            };
            match target {
                Some(i) if i < len => GenericResult::Owned(OwnedValue::Int(i as i64)),
                _ => GenericResult::Owned(OwnedValue::Null),
            }
        }
        Expr::Builtin(Builtin::First) => {
            if len == 0 {
                GenericResult::Owned(OwnedValue::Null)
            } else {
                GenericResult::Owned(OwnedValue::Int(0))
            }
        }
        Expr::Builtin(Builtin::Last) => {
            if len == 0 {
                GenericResult::Owned(OwnedValue::Null)
            } else {
                GenericResult::Owned(OwnedValue::Int(len as i64 - 1))
            }
        }
        // Slice 1 (#724), array counterpart of `LazyKeys`'s
        // own `Builtin::Map` arm above. No `!sorted` guard —
        // array "keys" are never sorted.
        Expr::Builtin(Builtin::Map(f)) => GenericResult::LazySeq(Box::new(
            LazySeq::new(LazySource::IndexRange { next: 0, len }).push_map(f, S::TAG),
        )),
        _ => eval_on_owned::<S, _>(expr, materialize_lazy_index_range(len), optional),
    }
}

/// One step of folding a `GenericResult::LazySeq` through a single further
/// pipe stage. Extracted verbatim from `fold_pipe_stages`'s own `LazySeq`
/// arm (#1565); see [`fold_lazy_keys_stage`]'s own doc comment for why this
/// split exists. `fold_pipe_stages_sink` intercepts `Expr::Iterate` before
/// ever calling this function, but every *other* stage shape here --
/// including `Expr::Builtin(Builtin::First) | Expr::index(0)`'s own
/// pull-one-and-stop fast path -- is identical between the eager and
/// demand-aware callers, since none of them fan out beyond the single `seq`
/// they were already holding.
fn fold_lazy_seq_stage<S: EvalSemantics, V: DocumentValue>(
    mut seq: Box<LazySeq<V>>,
    expr: &Expr,
    optional: bool,
) -> GenericResult<V> {
    match unwrap_paren(expr) {
        // In place (#789 code review follow-up): reuses the `Box`'s
        // existing heap slot instead of freeing it and allocating a
        // same-size replacement -- the hot path for a chained
        // `map(f) | map(g) | map(h)`, where this arm runs once per stage.
        Expr::Builtin(Builtin::Map(h)) => {
            seq.push_map_in_place(h, S::TAG);
            GenericResult::LazySeq(seq)
        }

        // Count-and-discard: every element still runs (so a
        // `map(f)` that errors partway still errors), but no
        // `OwnedValue` is ever built for any of them. Atomic,
        // same as `materialize_atomic` -- `length` of an
        // array construction that fails is itself a failure,
        // not a partial count.
        Expr::Builtin(Builtin::Length) => {
            let mut count: i64 = 0;
            for item in seq {
                match item {
                    Ok(_) => count += 1,
                    Err(Control::Error(e)) => return GenericResult::Error(e),
                    Err(Control::Break(label)) => return GenericResult::Break(label),
                    Err(Control::Halt(code)) => return GenericResult::Halt(code),
                }
            }
            GenericResult::Owned(OwnedValue::Int(count))
        }

        // `.[]` iterates the array `map`'s own construction
        // already built, not the raw source -- and that
        // construction is atomic in real jq
        // (`[1,2,"x"]|map(.+1)` prints nothing on error, not
        // a truncated prefix), so a failure here discards
        // every already-yielded element too, same atomicity
        // boundary as `Length`/the `_` fallback below. This
        // is NOT the same case as elementwise
        // `.[] | select(g)` (a structurally distinct,
        // out-of-scope case per the design doc): there,
        // `.[]` is the *source* of the pipe and each element
        // is independent; here it's a *consumer* of an
        // already-atomic `map` result.
        Expr::Iterate => {
            let mut items = Vec::new();
            for item in seq {
                match item {
                    Ok(elem) => items.push(elem),
                    Err(Control::Error(e)) => return GenericResult::Error(e),
                    Err(Control::Break(label)) => return GenericResult::Break(label),
                    Err(Control::Halt(code)) => return GenericResult::Halt(code),
                }
            }
            let all_cursor = items.iter().all(|item| matches!(item, LazyElem::Cursor(_)));
            if items.is_empty() {
                GenericResult::None
            } else if all_cursor {
                GenericResult::ManyCursor(
                    items
                        .into_iter()
                        .map(|item| match item {
                            LazyElem::Cursor(c) => c,
                            LazyElem::Owned(_) => {
                                unreachable!("checked all_cursor above")
                            }
                        })
                        .collect(),
                )
            } else {
                GenericResult::ManyOwned(owned_or_err!(items
                    .into_iter()
                    .map(|item| match item {
                        LazyElem::Cursor(c) => to_owned_cursor(&c),
                        LazyElem::Owned(o) => Ok(o),
                    })
                    .collect::<Result<Vec<_>, _>>()))
            }
        }

        // Pull-one-and-stop: at most one element of `seq` is
        // ever evaluated. Accepted, deliberate divergence
        // from real jq's strict semantics: real jq's `map`
        // eagerly builds the *whole* array before `first`/
        // `.[0]` can observe it, so `map(f)|first` errors if
        // *any* element of `f` fails, even ones past the
        // first. This fast path only evaluates what's
        // actually needed, so `[1,2,"x"]|map(.+1)|first`
        // succeeds here (returns `2`) where real jq raises a
        // type error -- the entire point of making `first`/
        // `.[0]` lazy is to skip evaluating elements that
        // don't affect the requested output, and an error on
        // a skipped element is one such element. Pinned by
        // `test_generic_lazy_seq_first_after_map_skips_later_error_725`.
        // #1401: `key: None` rather than `..` on purpose -- this arm is
        // deliberately restricted to the *integer* spelling of `.[0]`.
        //
        // The fold that merged `IndexNumber` into `Index` would otherwise
        // silently widen it: pre-fold this read `Expr::Index(0)`, which a
        // float-spelled `.[0.0]` could not match, so `.[0.0]` fell through
        // to the eager evaluator and *did* raise the later element's error
        // (matching jq 1.7.1). Taking this path instead would suppress it,
        // spreading the deliberate-but-divergent #725 skip to one more
        // spelling -- the wrong direction under ADR-0018, which permits a
        // divergence only where matching jq is impossible.
        //
        // That leaves `.[0]` and `.[0.0]` genuinely disagreeing here, which
        // is the drift class #1401/#1827 exist to surface rather than one
        // this refactor should quietly resolve either way -- filed as #2174.
        // The real fix is to stop diverging on #725, not to diverge twice.
        Expr::Builtin(Builtin::First) | Expr::Index { idx: 0, key: None } => match seq.next() {
            None => GenericResult::Owned(OwnedValue::Null),
            Some(Ok(LazyElem::Cursor(c))) => GenericResult::OneCursor(c),
            Some(Ok(LazyElem::Owned(o))) => GenericResult::Owned(o),
            Some(Err(Control::Error(e))) => GenericResult::Error(e),
            Some(Err(Control::Break(label))) => GenericResult::Break(label),
            Some(Err(Control::Halt(code))) => GenericResult::Halt(code),
        },

        // `last`, nonzero `.[n]`, whole-value `select`,
        // comparisons, everything else: one atomic forward
        // pass, then hand off to the full evaluator -- still
        // one pass, not the original four-pass round trip.
        // `select` deliberately gets no dedicated arm here —
        // it materializes once and runs through
        // `eval_on_owned`'s already-correct `Builtin::Select`
        // handling, same as any other computed value.
        _ => match seq.materialize_atomic() {
            Ok(owned) => eval_on_owned::<S, _>(expr, owned, optional),
            Err(Control::Error(e)) => GenericResult::Error(e),
            Err(Control::Break(label)) => GenericResult::Break(label),
            Err(Control::Halt(code)) => GenericResult::Halt(code),
        },
    }
}

/// Pull items one at a time from `elements`, threading each through `rest`
/// via [`continue_pipe_element_generic`] and stopping the moment that isn't
/// `Flow::Exhausted`. Shared by every `Expr::Iterate` fan-out case in
/// [`fold_pipe_stages_sink`] (#1565) so `first` only ever pays for the
/// elements it actually needed downstream, never the ones after it stopped.
/// An `Err(control)` from the source itself (only `LazySeq` can produce one,
/// mid-`map`) is an immediate stop, matching `fold_lazy_seq_stage`'s own
/// treatment of a mid-drain error.
fn drive_pipe_elements_generic<S: EvalSemantics, V: DocumentValue>(
    elements: impl Iterator<Item = Result<GenericItem<V>, Control>>,
    rest: &[Expr],
    optional: bool,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    // One `RestPipe` for the whole drive, so an owned copy is built on the
    // first `Owned` element and reused by every element after it (#1598).
    let mut rest = RestPipe::new(rest);
    for element in elements {
        match element {
            Ok(item) => {
                match continue_pipe_element_generic::<S, V>(item, &mut rest, optional, sink) {
                    Flow::Exhausted => continue,
                    other => return other,
                }
            }
            Err(Control::Error(e)) => return drain_result_generic(GenericResult::Error(e), sink),
            Err(Control::Break(label)) => {
                return drain_result_generic(GenericResult::Break(label), sink);
            }
            Err(Control::Halt(code)) => {
                return drain_result_generic(GenericResult::Halt(code), sink)
            }
        }
    }
    Flow::Exhausted
}

/// Demand-aware `Expr::Iterate` fan-out for a `LazyIndexRange` (#1565): no
/// allocation at all, mirroring `fold_lazy_index_range_stage`'s own
/// `Expr::Iterate` arm's zero-cost arithmetic -- the only difference is each
/// index is driven through `rest` one at a time instead of all being
/// collected into a `ManyOwned` first.
fn each_lazy_index_range_iterate_sink<S: EvalSemantics, V: DocumentValue>(
    len: usize,
    rest: &[Expr],
    optional: bool,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    drive_pipe_elements_generic::<S, V>(
        (0..len).map(|i| Ok(GenericItem::Owned(OwnedValue::Int(i as i64)))),
        rest,
        optional,
        sink,
    )
}

/// Demand-aware `Expr::Iterate` fan-out for a `LazyKeys` (#1565).
///
/// `!sorted` (`keys_unsorted`): streams [`DistinctKeyCursors`] straight
/// into the driver. This arm used to run `document::collapsed_fields`
/// (whose probe is a whole-object `census`: decode and fingerprint every
/// key, then sort the fingerprints) and then collect every key cursor into
/// a `Vec`, both *before* handing the first element downstream -- so
/// `first(keys_unsorted | .[] | f)` on a wide object paid O(n log n) plus a
/// full cons-list walk to produce one key (#1599).
///
/// Neither is needed here. "First occurrence wins" is an *online* rule, so
/// the dedup rides along with the walk instead of preceding it -- exactly
/// what `fold_lazy_keys_stage`'s own `Expr::Iterate` arm switched to in
/// #1514, via the `distinct_key_cursors_checked` wrapper. That arm has to
/// hand back every cursor at once to build a `ManyCursor`, so it collects;
/// this one drives elements one at a time and can consume the iterator
/// directly.
///
/// **The early exit is not unconditional.** It holds until a repeated key
/// is *reached*: at that point `DistinctKeyCursors` calls
/// `collapse_confirmed_repeat`, which walks the rest of the object and
/// owns a `String` per distinct key. So `first` over an object whose
/// second field repeats its first still pays a full walk. That is never
/// worse than the old path, which paid `census` plus `collapse_repeated`
/// unconditionally -- but it is not O(1) either, and sizing this arm's
/// worst case from the paragraph above alone would get it wrong.
///
/// The *sequence* of keys is identical to what collecting produced, and
/// for the same reason the resume is sound: `DistinctKeyCursors` emits
/// first occurrences in document order, and on confirming a real repeat
/// switches to the exact collapsed list resuming at the count already
/// yielded -- which lines up, because that list opens with those same
/// first occurrences. Unlike the `LazySeq` arm below, streaming raises no
/// atomicity question: producing a key cursor runs no user filter, so
/// there is no per-element failure that eager collection would have
/// masked.
///
/// **The cursor a collapsed key carries did change**, which is why this is
/// not purely a cost change. `collapse_repeated` overwrites the surviving
/// slot with the *last* occurrence's `key_cursor`, so the old path
/// reported the later position; streaming reports the first occurrence.
/// Observable through the position builtins and through `anchor`/`style`
/// in yq mode. The new answer is the one the collecting path has always
/// given and the one jq's first-occurrence-wins rule points at, so this
/// settles a disagreement between two spellings rather than creating one
/// -- pinned by `test_lazy_keys_streaming_reports_first_occurrence_cursor_1599`.
///
/// `sorted` (`keys`): lexicographic order needs every key decoded and
/// sorted first, unavoidably -- the same cost `materialize_lazy_keys` always
/// paid. Reuses `effective_keys` (the same helper `materialize_lazy_keys`
/// calls) rather than re-deriving key decoding, then walks the sorted `Vec`
/// one at a time as `Owned` strings -- matching `materialize_lazy_keys`'s
/// existing behaviour of not preserving a cursor for sorted keys (no new
/// capability, no regression, just demand-aware instead of eager).
///
/// **`!sorted` yields `OneCursorValue`, not `OneCursor`** (#1609):
/// `DistinctKeyCursors::next` already decodes each key's value for its own
/// duplicate-key hashing, so throwing it away here and letting
/// `continue_pipe_element_generic` re-derive it via `OneCursor`'s
/// `c.value()` would decode every key twice -- for YAML a real cost (a full
/// scalar resolve, not a cheap re-read), measurable as a ~5% regression on
/// a query that walks every key and matches none. Carrying the value
/// through instead removes that redundancy without changing output.
/// **Deliberate divergence, sibling of `each_lazy_seq_iterate_sink`'s own
/// (#725/#1565): a malformed key past whatever the consumer pulls is never
/// detected *when the consumer stops early*.** #1629 taught the
/// non-demand-aware `keys_unsorted` arms (`fold_lazy_keys_stage`) to raise
/// on a #1194 malformed member by walking the whole object -- correct there
/// because those arms already pay for a full walk regardless. This function
/// exists specifically so `first(keys_unsorted[])`/`limit(n;
/// keys_unsorted[])` do NOT pay for a full walk, so an unconditional
/// whole-object check would defeat the point.
///
/// Two halves, and only one of them is a gap. **Per key**, for free:
/// `DistinctKeyCursors::next` already decodes every key it yields (to hash
/// it), so a key the consumer actually pulls is checked below at no extra
/// cost. **Terminally** -- `ended_unpaired`/`delimiter_fault`, which have no
/// per-key signal at all -- the check runs when, and only when, the walk
/// reached exhaustion (#1653): a consumer that ran to the end already paid
/// for the whole walk, so asking costs it two bool reads, while a consumer
/// that stopped early is never charged. What remains a gap is therefore
/// exactly what #1770 scoped it to: a malformed key, or an unpaired tail,
/// sitting *after* the last element a truncating consumer pulled -- the same
/// way `each_lazy_seq_iterate_sink`'s doc comment above describes for a
/// `map(f)` failure past what `first` needed. Pinned by the `..._1770`
/// tests below; recorded in `docs/compliance/jq/limitations.md`.
fn each_lazy_keys_iterate_sink<S: EvalSemantics, V: DocumentValue>(
    fields: &V::Fields,
    sorted: bool,
    collapse: bool,
    rest: &[Expr],
    optional: bool,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    if !sorted {
        // Held by `by_ref` rather than moved into the `map` closure so the
        // *terminal* half of the #1194 check is still reachable once the
        // walk ends: `ended_unpaired`/`delimiter_fault` have no per-key
        // signal (`uncons_key` returns the same `None` either way), so only
        // the walk that finished can tell them apart. `DistinctKeyCursors`
        // owns clones of `fields`, so this borrow conflicts with nothing.
        let mut cursors = DistinctKeyCursors::new(fields, collapse);
        let flow = drive_pipe_elements_generic::<S, V>(
            cursors.by_ref().map(|(value, cursor)| {
                if key_is_malformed(&value) {
                    Err(Control::Error(fields.malformed_member_error()))
                } else {
                    Ok(GenericItem::OneCursorValue(cursor, value))
                }
            }),
            rest,
            optional,
            sink,
        );
        // Only on exhaustion. A consumer that stopped early (`first`,
        // `limit`) returns `Flow::Stopped` without the walk ever reaching
        // the tail, and charging it for one would restore exactly the whole-
        // object probe such a consumer exists to avoid (#1514/#1599) --
        // which is the divergence #1770 accepted, scoped to early exit.
        if matches!(flow, Flow::Exhausted) {
            if cursors.is_malformed() {
                return drain_result_generic(
                    GenericResult::Error(cursors.malformed_member_error()),
                    sink,
                );
            }
            // #2261: trailing stray comma after a real last key
            // (`{"a":1,}`) -- `cursors` already retained the last key
            // cursor this walk saw (`DistinctKeyCursors::last_key_cursor`),
            // so this is one more O(1) `next_sibling()` hop, not a further
            // walk. Same early-exit exemption as the `is_malformed()` check
            // just above.
            if !cursors.trailing_gap_ok(b'}') {
                return drain_result_generic(
                    GenericResult::Error(cursors.malformed_member_error()),
                    sink,
                );
            }
        }
        return flow;
    }

    let mut keys = match effective_keys(fields, collapse) {
        Ok(keys) => keys,
        Err(e) => return Flow::Escaped(Control::Error(e)),
    };
    keys.sort();
    drive_pipe_elements_generic::<S, V>(
        keys.into_iter()
            .map(|k| Ok(GenericItem::Owned(OwnedValue::String(k)))),
        rest,
        optional,
        sink,
    )
}

/// Demand-aware `Expr::Iterate` fan-out for a `LazySeq` (#1565): pulls
/// directly from `seq`'s own `Iterator` impl one element at a time, unlike
/// `fold_lazy_seq_stage`'s own `Expr::Iterate` arm, which drains it fully
/// (running every buffered `map(f)` closure) before returning -- draining is
/// exactly the O(n) cost `first(map(f) | .[] | g)` should not pay. `seq`
/// yields `Result<LazyElem<V>, Control>`; only [`LazyElem::Cursor`]/
/// [`LazyElem::Owned`] are real elements, an `Err` is a mid-`map` failure
/// handled by [`drive_pipe_elements_generic`] itself.
///
/// **Deliberate divergence from real jq, and from the eager arm's own
/// atomicity.** `fold_lazy_seq_stage`'s `Expr::Iterate` arm drains first, so
/// a `map(f)` that fails on *any* element discards every element it already
/// yielded -- matching real jq, where `map` builds the whole array before
/// `.[]` can observe it. Pulling one at a time cannot preserve that: an
/// element already handed to `sink` is already downstream. So
/// `[1,"x",3] | first(map(.+1) | .[])` yields `2` here where jq errors.
///
/// This is the same trade #725 already made for `map(f) | first` (pinned by
/// `test_generic_lazy_seq_first_after_map_skips_later_error_725`), widened
/// from `first`/`.[0]` to `.[]`-under-demand: the point of a lazy `first` is
/// to skip elements that cannot affect the requested output, and an element
/// that errors is one such element. Restoring atomicity would mean draining
/// the whole `seq` before emitting anything, which is exactly the O(n) cost
/// #1565 exists to remove. Pinned by
/// `test_first_over_lazy_seq_iterate_skips_later_error_1565`; recorded in
/// `docs/compliance/jq/limitations.md`.
fn each_lazy_seq_iterate_sink<S: EvalSemantics, V: DocumentValue>(
    seq: LazySeq<V>,
    rest: &[Expr],
    optional: bool,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    drive_pipe_elements_generic::<S, V>(
        seq.map(|item| {
            item.map(|elem| match elem {
                LazyElem::Cursor(c) => GenericItem::OneCursor(c),
                LazyElem::Owned(o) => GenericItem::Owned(o),
            })
        }),
        rest,
        optional,
        sink,
    )
}

/// Demand-aware `Expr::Iterate` fan-out for an array (#1597 part 2): walks
/// `uncons_cursor()` one element at a time, pushing each straight to `sink`,
/// instead of `eval_single`'s `Expr::Iterate` arm, which calls
/// `collect_cursors_checked` -- a full upfront walk into a `Vec` -- before
/// the first cursor ever reaches a consumer. `first(.[])`/`first(.[] | f)`
/// paid for materializing every element's cursor on a 2M-element array to
/// answer a query needing exactly one (#1597).
///
/// No `rest`/pipe-stage parameter, unlike [`each_lazy_keys_iterate_sink`]/
/// [`each_lazy_seq_iterate_sink`]: those exist to unpack a `GenericResult::
/// LazyKeys`/`LazySeq` marker `fold_pipe_stages_sink` already knows is
/// followed immediately by `.[]` in a *known* stage list. A bare `.[]`
/// reached through [`eval_each_generic`]'s own per-node dispatch has no such
/// marker or stage list -- `sink` here already *is* "whatever the caller
/// needs done with each element" (chaining through the rest of a `Pipe`,
/// feeding a `Comma` branch, etc.), the same as every other native arm in
/// that match (`each_if_generic` and siblings) that calls `sink` directly.
///
/// Shares [`DocumentCursor::element_gap_ok`]'s #1677 gap check with
/// [`DocumentElements::collect_cursors_checked`] rather than duplicating
/// it -- the loop shape differs (this one yields as it goes instead of
/// only returning once finished, since that is the entire point), but the
/// check itself is the same one definition either way.
///
/// **Deliberate divergence**, the same shape [`each_lazy_seq_iterate_sink`]'s
/// own doc comment already accepts for `LazySeq`: a malformed comma *after*
/// whatever the consumer actually pulled is never detected, since the walk
/// that would find it never runs past what was asked for. `first(.[])` on
/// `[1,,3]` still raises correctly (the fault is on the very first element
/// examined); `first(.[])` on a well-formed `1` followed by a *later*
/// malformed gap does not see it -- including the #2261 trailing-comma
/// shape below (`[1,]`), which by definition only shows up once the walk
/// reaches the container's real last element. Every eager caller of
/// `collect_cursors_checked` (`.[]` reached through the rest of the
/// evaluator, `to_entries`) does see it (#2261 closed that gap in
/// `collect_cursors_checked` itself) -- only this demand-aware path takes
/// the early-exit trade. Recorded in `docs/compliance/jq/limitations.md`.
fn each_lazy_array_iterate_sink<V: DocumentValue>(
    elements: V::Elements,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let mut elems = elements;
    let mut is_first = true;
    // #2261: the last real element's own cursor, retained past the loop so
    // the trailing-gap check below (a stray `,` *after* a real last
    // element, `[1,]`) has something to check from once the walk exhausts
    // -- mirrors `to_owned_cursor_at_depth`'s own `last_elem` (#2243) and
    // `collect_cursors_checked`'s identical addition (#2261).
    let mut last_cursor: Option<V::Cursor> = None;
    while let Some((cursor, next)) = elems.uncons_cursor() {
        if !cursor.element_gap_ok(is_first) {
            // `elements.malformed_element_error()`, not `elems` (the
            // per-iteration state at the point of failure) -- matching
            // `collect_cursors_checked`'s own `self.malformed_element_error()`
            // convention (#1597 code review). Every format shipped today
            // ignores the receiver entirely (the default) or re-derives from
            // the whole document regardless of which cursor calls it (JSON),
            // so this has no observable effect now, but keeps both call
            // sites' conventions identical for a future format where it might.
            return Flow::Escaped(Control::Error(elements.malformed_element_error()));
        }
        is_first = false;
        elems = next;
        last_cursor = Some(cursor);
        if sink(GenericItem::OneCursor(cursor)) == Demand::Stop {
            return Flow::Stopped { pending: None };
        }
    }
    // #2261: only on exhaustion -- see this function's own doc comment
    // above for why an early-stopped consumer never reaches this check.
    if let Some(last) = last_cursor {
        if !trailing_element_gap_ok(&last, b']') {
            return Flow::Escaped(Control::Error(elements.malformed_element_error()));
        }
    }
    Flow::Exhausted
}

/// Demand-aware twin of `fold_pipe_stages` (#1565): folds an already-produced
/// `LazyKeys`/`LazyIndexRange`/`LazySeq` item through the remaining pipe
/// stages one at a time, honoring `sink`'s `Demand` the moment `Expr::Iterate`
/// would otherwise fan out into multiple elements -- so a `first`-driven pull
/// stops as soon as the sink says so, instead of `fold_pipe_stages`'s
/// unconditional full materialization (`first(keys | .[] | stderr)` visiting
/// every key). Every *other* stage shape reuses `fold_lazy_keys_stage`/
/// `fold_lazy_index_range_stage`/`fold_lazy_seq_stage` -- the exact same
/// composability fast paths `fold_pipe_stages` itself uses (#724/#725) --
/// rather than duplicating them, and the moment folding resolves to a
/// genuinely single value, control hands off to `eval_each_pipe_generic`/
/// `eval_each_owned`, which already know how to thread an arbitrary-length
/// remaining pipe through one value without re-deriving cursor threading here
/// (the #1503 lesson).
fn fold_pipe_stages_sink<S: EvalSemantics, V: DocumentValue>(
    mut current: GenericResult<V>,
    stages: &[Expr],
    optional: bool,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let mut j = 0usize;
    loop {
        if j == stages.len() {
            return drain_result_generic(current, sink);
        }
        let expr = &stages[j];
        let rest = &stages[j + 1..];
        let is_iterate = matches!(unwrap_paren(expr), Expr::Iterate);
        current = match current {
            GenericResult::LazyKeys {
                fields,
                sorted,
                collapse,
            } if is_iterate => {
                return each_lazy_keys_iterate_sink::<S, V>(
                    &fields, sorted, collapse, rest, optional, sink,
                );
            }
            GenericResult::LazyKeys {
                fields,
                sorted,
                collapse,
            } => fold_lazy_keys_stage::<S, V>(fields, sorted, collapse, expr, optional),
            GenericResult::LazyIndexRange(len) if is_iterate => {
                return each_lazy_index_range_iterate_sink::<S, V>(len, rest, optional, sink);
            }
            GenericResult::LazyIndexRange(len) => {
                fold_lazy_index_range_stage::<S, V>(len, expr, optional)
            }
            GenericResult::LazySeq(seq) if is_iterate => {
                return each_lazy_seq_iterate_sink::<S, V>(*seq, rest, optional, sink);
            }
            GenericResult::LazySeq(seq) => fold_lazy_seq_stage::<S, V>(seq, expr, optional),
            // Resolved to a genuinely single value/cursor -- hand off to the
            // already-correct, arbitrary-length-pipe-aware demand driver
            // rather than re-deriving its cursor-threading logic here
            // (#1503).
            //
            // These three hand off `&stages[j..]`, NOT `rest`: `current` is
            // the value *entering* `stages[j]`, since only the lazy arms
            // above consume `expr` (they are the arms that assign back to
            // `current` and fall through to `j += 1`). Handing off `rest`
            // here silently skipped `stages[j]` altogether -- `first(keys |
            // length | tostring)` printed `3` instead of `"3"` -- so the
            // `stages[j..]` the `other` arm below already used is the
            // correct slice for every non-folding arm. Pinned by
            // `test_first_over_lazy_prefix_applies_every_stage_1565`.
            GenericResult::One(v) => {
                return eval_each_pipe_generic::<S, V>(&stages[j..], v, optional, None, sink);
            }
            GenericResult::OneCursor(c) => {
                return eval_each_pipe_generic::<S, V>(
                    &stages[j..],
                    c.value(),
                    optional,
                    Some(c),
                    sink,
                );
            }
            GenericResult::Owned(o) => {
                let rest_pipe = Expr::Pipe(stages[j..].to_vec());
                return eval_each_owned::<S>(&rest_pipe, &o, optional, &mut |o| {
                    sink(GenericItem::Owned(o))
                });
            }
            // Nothing further to fold; push (or don't) and stop, same as
            // `drain_result_generic`'s own handling of these.
            terminal @ (GenericResult::None
            | GenericResult::Error(_)
            | GenericResult::Break(_)
            | GenericResult::Halt(_)) => {
                return drain_result_generic(terminal, sink);
            }
            // Genuinely reachable, not a safety net -- do not turn this
            // into `unreachable!()`. `Expr::Iterate` is not the only stage
            // shape that can fan out: `fold_lazy_keys_stage`'s own
            // materializing `_` fallback hands `expr` to `eval_on_owned`,
            // which returns `ManyOwned` for any multi-output stage that
            // isn't one of the native fast paths. `first(keys | .[0,1] |
            // stderr)` is the shortest witness -- `.[0,1]` is an
            // `Expr::IndexExpr` with a `Comma` key, so it lands here with
            // `current` already a two-element `ManyOwned`.
            //
            // Folding the rest eagerly from here is a deliberate
            // correct-but-less-lazy fallback: once a stage has already
            // fanned out into a materialized `Vec`, its side effects have
            // all fired anyway, so there is no demand left to honor for
            // *that* stage. Pinned by
            // `test_first_over_lazy_prefix_applies_every_stage_1565`'s
            // `.[0,1]` row.
            other => {
                return drain_result_generic(
                    fold_pipe_stages::<S, V>(other, &stages[j..], optional),
                    sink,
                );
            }
        };
        j += 1;
    }
}

/// `try INNER catch CATCH` (`CATCH = None` is `?`'s own desugaring, #1812
/// code review) -- `eval_single`'s twin of [`each_try_generic`], which
/// already unifies `Expr::Optional`/`Expr::Try` this same way for
/// `eval_each_generic`'s sink-based sibling. `Expr::Optional`/`Expr::Try`
/// used to be two independently hand-rolled ~65-line matches here, each
/// re-deriving the same decode-failure/`Error`/`Break`/`Partial`/`LazySeq`
/// dispatch -- a duplicate the codebase's own precedent (this function's
/// sink-based twin, 650 lines below) had already resolved the same way.
///
/// **This function and [`each_try_generic`] are still two separate
/// implementations of the identical dispatch** (one for `eval_single`'s
/// pull/`GenericResult` model, one for `eval_each_generic`'s push/`Flow`
/// model, mirroring `eval.rs`'s own `eval_single`/`eval_each` split) — a
/// future change to the catchability rules (another `#1620`-style
/// exclusion, a new `Control` variant) needs applying to *both*, the same
/// way `eval.rs`'s and `eval_generic.rs`'s own evaluator pair already needs
/// any reindex-bridge fix applied in parallel. Consistent on the properties
/// both were built with (verified: same `is_decode_failure()` exclusion,
/// `Break` bound to `null`, `catch = None` suppression, `Halt` passthrough)
/// -- but **not** on lazy-variant forcing: this function's own `LazyKeys`
/// arm below (#1936) has no counterpart in `each_try_generic`, a known,
/// tracked divergence (#1948), not an oversight in this doc paragraph.
///
/// A decode failure (#1247) is never caught by either spelling, any more
/// than real jq's own parse-time rejection could ever be caught (#1620) --
/// checked before the ordinary `Error`/`Break` catch below so it falls
/// through to `other => other` instead. `catch` runs bound to the raised
/// payload for `Error`, or `null` for `Break` (#562, matching real jq
/// binding its own internal break marker there instead, not worth
/// replicating) -- `catch = None` collapses straight to
/// [`GenericResult::None`], exactly `?`'s own suppression.
///
/// `prefix` is never empty in the `Partial` arms: `partial_generic` (and
/// `eval::partial`, its mirror) already collapse an empty prefix to the
/// bare `Error`/`Break` variant above before a `Partial` ever gets
/// constructed (#400, #494). [`prepend_generic`] is routed through
/// regardless (#1067): a future change that ever violates that invariant
/// degrades gracefully instead of silently misbehaving.
///
/// Neither `LazyKeys` nor `LazySeq` have necessarily failed *yet* -- both
/// are lazy, so neither matches the `Error`/`Break`/`Partial` arms above
/// even when pulling one would fail (#724, #725, #683). This boundary needs
/// to know *now* whether `inner` fails, so force both here -- same one-pass
/// cost every other materializing boundary in this file already pays.
/// Without this, `inner`'s lazy result falls through to `other => other`
/// and escapes the boundary entirely: the error only surfaces later, at
/// whatever downstream site finally pulls it, by which point this
/// `try`/`catch`/`?` is long gone (confirmed against real jq for `LazySeq`:
/// `[1,2,"x"]|map(.+1)?` is empty/exit-0 in jq, but errored/exit-5 here
/// before that arm existed; `LazyKeys`' own #1936 case has no jq oracle --
/// `keys_unsorted` on a non-string key is a document real jq's own parser
/// rejects outright -- so it's pinned by comparing against this same
/// boundary's already-correct `sort?`/`try (sort) catch` handling of the
/// identical document instead). `halt` is never caught here either way,
/// matching `Control`'s own pass-through guarantee and the `other => other`
/// wildcard's identical treatment of a bare `GenericResult::Halt`.
///
/// `LazyKeys` is checked via [`keys_are_well_formed`] rather than
/// [`GenericResult::materialize_lazy`]/`materialize_lazy_keys`: the latter
/// would decode and collect every key into an owned `Vec<String>` even on
/// the ordinary, non-failing path, permanently forfeiting `fold_pipe_stages`'s
/// `!sorted` O(1)/no-allocation fast paths (`.[]`, `.[n]`, `first`, `last`)
/// for any `keys_unsorted` merely *wrapped* in `?`/`try`/`catch` -- a real
/// cost with no correctness benefit, caught in review before this landed.
/// `keys_are_well_formed` walks the same fields doing the identical #1194
/// check but collects nothing (an earlier version of this fix called
/// [`distinct_key_cursors_checked`] here instead, collecting a `Vec<V::Cursor>`
/// purely to discard it -- the exact O(n)-space-for-an-O(1)-space-answer
/// mistake `Builtin::Last` below already hit and fixed once; also caught by
/// review), and on success this arm hands back the *original*, still-lazy
/// `LazyKeys` unchanged, so a downstream `fold_pipe_stages` fast-path arm
/// still gets to run.
///
/// **This does not make the check free, only as cheap as it can be while
/// still being correct.** Handing back a still-lazy `LazyKeys` means
/// anything downstream that goes on to actually read the keys -- the CLI's
/// own streaming writer for a bare `keys_unsorted?`, or `fold_lazy_keys_stage`'s
/// own materializing fallback for any continuation that isn't one of its
/// narrow `!sorted` fast paths -- walks the object a *second* time to get
/// there, since nothing here caches or tags the object as already-validated.
/// That is not avoidable within this fix's scope: this boundary must know
/// *now* whether the object contains a malformed key, and the only way to
/// know that is to look at every key once, so a `?`/`try`-wrapped
/// `keys`/`keys_unsorted` unavoidably costs at least one extra full walk
/// over the unwrapped form -- the same trade-off `LazySeq` already makes,
/// just without `LazySeq`'s consolation of retiring laziness for good on
/// success. Threading an "already validated" fact through `LazyKeys` itself
/// so a later consumer could skip re-checking is a real follow-up
/// optimization, tracked as #1951 rather than attempted here to keep this
/// fix reviewable at the size of a narrow bug fix rather than a `LazyKeys`
/// redesign.
///
/// `LazyIndexRange` needs no arm at all: its value is fully described by
/// `len` alone (#684), so unlike `LazyKeys` it can never actually fail --
/// forcing it here would cost real allocation for zero correctness
/// benefit, so it stays on the `other => other` wildcard, exactly as
/// before this fix.
///
/// [`each_try_generic`] below, this function's push-model twin, has a
/// **similar but distinct, still-open gap**: `eval_each_generic`'s own
/// wildcard fallback pushes a `GenericResult::LazyKeys`/`LazyIndexRange`/
/// `LazySeq` straight to `sink` unmaterialized whenever the immediate
/// consumer isn't the narrow `keys_unsorted | .[]` shape
/// `each_lazy_keys_iterate_sink` (#1770) covers -- confirmed live:
/// `first(keys_unsorted?)` and `first(try (map(error("x"))) catch "c")`
/// both still raise uncaught on `main` (the latter predates this fix
/// entirely, since it's `LazySeq`, not `LazyKeys`). Tracked separately as
/// #1948 rather than folded in here: closing it needs `eval_each_generic`
/// itself to carry a "must materialize now" signal through arbitrarily
/// nested push-model consumers (`first`, `limit`, ...), not just a new arm
/// in this function's own dispatch.
fn try_single_generic<S: EvalSemantics, V: DocumentValue>(
    inner: &Expr,
    catch: Option<&Expr>,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    let run_catch = |payload: &OwnedValue| -> GenericResult<V> {
        match catch {
            Some(catch_expr) => eval_each_owned_collect::<S, V>(catch_expr, payload, optional),
            None => GenericResult::None,
        }
    };
    match eval_single::<S, _>(inner, value, optional, cursor) {
        // #2254: a yq negative-index-out-of-range error is unsuppressible
        // the same way a decode failure is (see
        // `EvalError::is_yq_negative_index_error`'s own doc comment) --
        // confirmed live that `.a[-5]?` still raises in real yq. Real yq has
        // no `try`/`catch` syntax at all (lexer-rejected outright), so this
        // arm only ever matters for succinctly's own `--jq-extensions`
        // surface in yq mode; excluding it from `catch` there too keeps one
        // consistent "never caught" rule rather than one that depends on
        // whether a catch handler happens to be present.
        GenericResult::Error(e) if e.is_uncatchable_at_value_position() => GenericResult::Error(e),
        GenericResult::Error(e) => run_catch(&e.payload()),
        GenericResult::Break(_) => run_catch(&OwnedValue::Null),
        GenericResult::Partial(prefix, Control::Error(e))
            if e.is_uncatchable_at_value_position() =>
        {
            GenericResult::Partial(prefix, Control::Error(e))
        }
        GenericResult::Partial(prefix, Control::Error(e)) => {
            prepend_generic(prefix, run_catch(&e.payload()))
        }
        GenericResult::Partial(prefix, Control::Break(_)) => {
            prepend_generic(prefix, run_catch(&OwnedValue::Null))
        }
        GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
            Ok(owned) => GenericResult::Owned(owned),
            Err(Control::Error(e)) if e.is_uncatchable_at_value_position() => {
                GenericResult::Error(e)
            }
            Err(Control::Error(e)) => run_catch(&e.payload()),
            Err(Control::Break(_)) => run_catch(&OwnedValue::Null),
            Err(Control::Halt(code)) => GenericResult::Halt(code),
        },
        GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        } => match keys_are_well_formed::<V>(&fields, collapse) {
            Ok(()) => GenericResult::LazyKeys {
                fields,
                sorted,
                collapse,
            },
            // Unreachable for every format shipped today: `malformed_member_error`
            // (the default and JSON's override alike) always builds a plain,
            // catchable `EvalError::new(...)`, never a `DecodeFailure`-kind
            // error, so this arm's condition is never true. Kept anyway,
            // matching the `LazySeq` arm above -- defensive symmetry for a
            // future format whose `malformed_member_error` ever does tag one.
            Err(e) if e.is_decode_failure() => GenericResult::Error(e),
            Err(e) => run_catch(&e.payload()),
        },
        other => other,
    }
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
                    Ok(Some(c)) => GenericResult::OneCursor(c),
                    // jq returns null for missing fields on objects (not an error)
                    Ok(None) => GenericResult::Owned(OwnedValue::Null),
                    // Either #1677 (the field this lookup resolved to has a
                    // malformed `,`/`:` delimiter) or #1995 (some sibling's
                    // key isn't string-shaped at all) -- see
                    // `find_cursor`'s own doc comment.
                    Err(err) => GenericResult::Error(err),
                }
            } else if value.is_null() {
                // jq returns null for field access on null
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_field(
                    tagged_type_name(&value, cursor),
                    name,
                ))
            }
        }

        Expr::Index { idx, .. } => {
            if let Some(elements) = value.as_array() {
                // #2261: `_checked`, not the bare `len()` this used to call
                // -- free here, unlike a *positive*-only lookup elsewhere
                // in this evaluator (`Expr::Iterate`'s sorted-key positive-
                // index arm, #1629's own precedent for "would cost strictly
                // more than the answer"): resolving *any* index here,
                // negative or positive, already walks the whole array via
                // `len()` first (to normalize a negative index and to raise
                // yq's own out-of-range error), so the trailing/leading gap
                // checks ride along for free on every call, not just the
                // negative-index ones.
                let len = match elements.len_checked() {
                    Ok(len) => len,
                    Err(err) => return GenericResult::Error(err),
                };
                let resolved = if *idx < 0 { len as i64 + idx } else { *idx };
                // yq mode only (#2254): a negative index still negative
                // after resolving against the length raises in real yq --
                // unconditionally, not suppressed by `optional` (see
                // `EvalError::yq_negative_index_out_of_range`'s own doc
                // comment). Checked ahead of the cast below, which would
                // otherwise wrap a still-negative `resolved` into a huge
                // `usize` that `get_cursor` simply misses (`None`, same
                // observable `null` as any other OOB read) -- correct for
                // jq mode, but not distinguishable from an ordinary miss
                // for yq's own error.
                if let Some(e) = yq_negative_index_check::<S>(*idx, resolved, len) {
                    return GenericResult::Error(e);
                }
                match elements.get_cursor(resolved as usize) {
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
            } else {
                // No `optional`-guarded arm here (unlike `index_one_generic`,
                // the computed-key sibling this literal `.[N]` form doesn't
                // share code with): `.[N]?` parses straight to
                // `Expr::Optional(Expr::index(N))`, which isn't
                // `IndexExpr`/`SliceExpr`, so #693's dispatch never forces
                // `optional = true` into this arm — it evaluates `Expr::
                // Index` at the ambient `optional` (normally `false`) and
                // lets the outer `Expr::Optional`/`eval_try`-style catch
                // convert the resulting `Error` to `None` once, same
                // externally-observable result via a different path.
                GenericResult::Error(EvalError::cannot_index_with_type(
                    tagged_type_name(&value, cursor),
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

        // Handled natively for the same two reasons as `Expr::IndexExpr`
        // above: the fallback re-enters `full_eval`, which restarts with
        // `optional = false` and so loses the `?` in `.[.a:.b]?`; and it
        // serialises and re-indexes the whole document per evaluation (#615).
        Expr::SliceExpr { target, start, end } => {
            eval_slice_expr::<S, V>(target, start, end, value, optional, cursor)
        }

        Expr::Iterate => {
            if let Some(elements) = value.as_array() {
                // #1677: `.[]` over an array reaches into every element
                // without ever re-serializing the container whole, so it
                // needs its own gap check between siblings.
                match elements.collect_cursors_checked() {
                    Ok(cursors) if cursors.is_empty() => GenericResult::None,
                    Ok(cursors) => GenericResult::ManyCursor(cursors),
                    Err(err) => GenericResult::Error(err),
                }
            } else if let Some(fields) = value.as_object() {
                // `.[]` collapses a repeated key to its first position but
                // last-seen value in *both* modes (#1398) -- unlike every
                // other builtin `S::COLLAPSE_DUPLICATE_KEYS` governs, real
                // yq is inconsistent here and does collapse under `.[]`
                // traversal alone (confirmed live against yq v4.53.3), so
                // this always passes `true` rather than the mode flag.
                //
                // `_checked`: an object member the semi-index accepted but
                // JSON does not (a bareword key, an unpaired trailing child,
                // #1194) used to reach here silently, because
                // `effective_fields`'s walk discards the exhausted tail that
                // is the only place `ends_unpaired` can still answer (#1641).
                // The check is free here -- it rides the same walk this arm
                // already ran unconditionally, same as `effective_len_checked`
                // does for `length`.
                match effective_fields_checked(&fields, true) {
                    Ok(effective) => {
                        let cursors: Vec<_> = effective
                            .into_iter()
                            .map(|field| field.value_cursor)
                            .collect();
                        if cursors.is_empty() {
                            GenericResult::None
                        } else {
                            GenericResult::ManyCursor(cursors)
                        }
                    }
                    Err(err) => GenericResult::Error(err),
                }
            } else {
                decode_failure_or(&value, optional, || {
                    GenericResult::Error(EvalError::cannot_iterate_with(
                        S::TAG,
                        &to_owned_for_diagnostic(&value, cursor),
                    ))
                })
            }
        }

        // `.[EXPR]?`/`.[S:E]?`: mirrors `eval::eval_single`'s identical
        // special case (see its comment for the jq-1.7.1-verified reasoning)
        // — `?` on a bare bracket-index/slice postfix guards only the final
        // index/slice step, not the bracket's own key/bounds sub-expression.
        // This file's own `eval_index_expr`/`eval_slice_expr` (below)
        // already evaluate key/bounds with a hardcoded `optional: false` and
        // only consult the ambient `optional` for their final step, so
        // preserve the direct forwarding dispatch here instead of the
        // catch-everything arm below, which would catch the key/bounds
        // error too.
        Expr::Optional(inner)
            if matches!(**inner, Expr::IndexExpr { .. } | Expr::SliceExpr { .. }) =>
        {
            eval_single::<S, _>(inner, value, true, cursor)
        }

        // `E?` is sugar for `try E`: evaluate `inner` with the ambient
        // `optional` (not forced `true`) and catch the aggregate result
        // exactly once here, mirroring `eval::eval_try` (there is no local
        // `Expr::Try`/combinator handling in this file to delegate to —
        // those already bridge to `eval::eval`, which implements this same
        // pattern). Forcing `optional = true` down the whole subtree let a
        // masked error inside a natively-evaluated `Pipe` fan-out look like
        // ordinary `empty`, so the fan-out wrongly kept going instead of
        // stopping (#693).
        // `E?` is `try E` with no catch handler -- see `try_single_generic`'s
        // own doc comment for the full dispatch this delegates to.
        Expr::Optional(inner) => try_single_generic::<S, V>(inner, None, value, optional, cursor),

        // `try EXPR catch HANDLER` (#1812) -- see `try_single_generic`'s own
        // doc comment. Without this arm, `Expr::Try` fell to the wildcard
        // bridge below, which materializes the ambient value via
        // `owned_or_err!` *before* ever reaching `full_eval`'s own
        // catchability-aware `eval_try` -- an uncatchable-by-design error (a
        // decode failure) was already correctly uncaught either way, but a
        // genuinely catchable one (e.g. a #1194 malformed-key error) was
        // wrongly left uncaught too, since `owned_or_err!` has no
        // catchability check of its own at all. Confirmed live: `sort?` on
        // `{123: 1}` already suppressed cleanly (`Expr::Optional`'s own
        // `Error` case), while `try (1+1) catch "x"` raised uncaught before
        // this fix.
        Expr::Try { expr: inner, catch } => {
            try_single_generic::<S, V>(inner, catch.as_deref(), value, optional, cursor)
        }

        // Parens are transparent to cursor-based evaluation: handled natively
        // (like `Expr::Optional` above) so `(.)` and friends keep threading
        // the cursor instead of falling to the `to_owned()` bridge below,
        // which collapses duplicate mapping keys (#614).
        Expr::Paren(inner) => eval_single::<S, _>(inner, value, optional, cursor),

        Expr::Pipe(exprs) => {
            // `path`/`parent`/`parent(n)`/`key` need the path accumulated
            // across every stage of this pipe, which only the full evaluator
            // tracks (`eval::eval_pipe`'s own `needs_path_context` routing,
            // `eval.rs:6441-6444`). Bridge the *whole* pipe there rather than
            // letting a later stage fall through `eval_builtin`'s per-builtin
            // fallback in isolation, which has no path to give it (#554).
            if exprs.iter().any(needs_path_context) {
                // #2061: `key`/`path`/`parent` answer from the path the walk
                // accumulates, so for a purely navigational pipe there is no
                // reason to build an `OwnedValue` tree for the document
                // first. `.[0] | key` cost 519 MiB on a 20 MB array to
                // answer `0`. Anything the walk does not model returns
                // `None` and falls through to the bridge below, unchanged.
                if let Some(root) = cursor {
                    if let Some(result) = try_path_context_cursor_walk::<S, V>(exprs, root) {
                        return result;
                    }
                }
                let owned = owned_or_err!(to_owned_with_cursor(&value, cursor));
                // #1909: straight into `eval.rs`'s path-context evaluator
                // with the tree we just built, rather than through
                // `eval_on_owned`'s reindex bridge -- which lands in
                // `eval::eval_pipe`, whose own `needs_path_context` gate
                // (the same predicate checked just above) materializes the
                // whole document a *second* time before calling exactly this
                // function. Only where that bridge was a semantic no-op
                // (`reindex_bridge_is_identity`); otherwise unchanged.
                //
                // `optional` is deliberately not threaded into the bypass,
                // for the same reason `eval_on_owned` doesn't thread it into
                // its own `full_eval` call: that entry point restarts every
                // evaluation at `false` regardless of what its caller
                // passed, so `false` is what the bridge actually delivered.
                if reindex_bridge_is_identity(&owned) {
                    return query_result_to_generic::<V>(
                        crate::jq::eval::eval_pipe_with_path_context::<Vec<u64>, S>(
                            exprs,
                            &owned,
                            &[],
                            false,
                        ),
                    );
                }
                return eval_on_owned::<S, _>(&Expr::Pipe(exprs.clone()), owned, optional);
            }

            if exprs.is_empty() {
                return GenericResult::One(value);
            }

            let current = eval_single::<S, _>(&exprs[0], value, optional, cursor);
            fold_pipe_stages::<S, V>(current, &exprs[1..], optional)
        }

        // Handled natively rather than through the `_` fallback below: the
        // fallback materializes the whole input via `to_owned()` before
        // `expr` ever runs, so `first(.[])`/`last(.[])` on `[{"a":1,"a":2}]`
        // lost the duplicate key before the first/last extraction even
        // started (#607). `first`/`last` never change position -- the
        // selected output IS one of `inner`'s own outputs -- so forward
        // whatever cursor that output already carries.
        Expr::FirstExpr(inner) => {
            eval_first_or_last_generic::<S, _>(inner, value, optional, cursor, false)
        }
        Expr::LastExpr(inner) => {
            eval_first_or_last_generic::<S, _>(inner, value, optional, cursor, true)
        }

        // Same reasoning as `FirstExpr`/`LastExpr` above (#607), a second
        // instance of it (#1607): the `_` fallback below materializes the
        // whole input via `to_owned()` before `expr` ever runs, so
        // `limit(n; keys|.[])` on a document with a duplicate mapping key
        // lost every duplicate — `OwnedValue::Object` is `IndexMap`-backed
        // and cannot represent them, even though `keys|.[]` alone (via
        // `LazyKeys`/`DistinctKeyCursors`) already preserves them correctly.
        // `eval_limit_generic` only takes over the common case (`n_expr`
        // evaluates to one plain value); see its own doc comment for why a
        // generator `n` is left on the pre-existing path below.
        Expr::Limit { n, expr } => {
            match eval_limit_generic::<S, _>(n, expr, value.clone(), optional, cursor) {
                Some(result) => result,
                None => bridge_to_full_evaluator::<S, _>(
                    &Expr::Limit {
                        n: n.clone(),
                        expr: expr.clone(),
                    },
                    value,
                    cursor,
                    optional,
                ),
            }
        }

        // A CLI `nth(n; expr)` always parses to `Builtin::NthStream` (see
        // that arm in `eval_builtin`, where the real fix lives) --
        // `Expr::NthExpr` itself is never freshly constructed by the parser,
        // only passed through unchanged by AST rewrites like
        // `substitute_var` (see `eval.rs`'s `eval_nth_expr_n_argument_
        // propagates_halt` for the same note on its `eval.rs` counterpart).
        // Handled here anyway, at the same cost as the arm above, so a
        // rewrite-preserved `NthExpr` gets the identical duplicate-key fix
        // rather than silently falling back to the lossy path depending on
        // which of the two spellings it happens to carry.
        Expr::NthExpr { n, expr } => {
            match eval_nth_generic::<S, _>(n, expr, value.clone(), optional, cursor) {
                Some(result) => result,
                None => bridge_to_full_evaluator::<S, _>(
                    &Expr::NthExpr {
                        n: n.clone(),
                        expr: expr.clone(),
                    },
                    value,
                    cursor,
                    optional,
                ),
            }
        }

        // #1062: was a hand-rolled arm-for-arm copy of the same six
        // `Literal` variants (one of three targeting `OwnedValue` -- see
        // `literal_to_owned`'s own doc comment for the other two); delegates
        // to the shared conversion now instead.
        Expr::Literal(lit) => GenericResult::Owned(literal_to_owned(lit)),

        // Formats are pure functions of the value, so evaluate them here rather
        // than falling through to the catch-all, which would serialize the
        // value to JSON and rebuild a `JsonIndex` for every one (#124).
        //
        // #2280: owned_or_suppress!, not owned_or_err! -- `optional` is
        // passed to `format_result` on this very line, so ignoring it in the
        // materialization one line above was a real asymmetry. This is the
        // arm the CLI's own default jq/yq dispatch actually reaches for
        // every `@format` builtin (`jq_runner.rs`/`yq_runner.rs` route
        // through `eval_generic`, never `eval.rs`'s own sibling
        // `eval_format`) -- found by code review to have been missed by
        // this issue's first pass, which fixed `eval_format` without
        // noticing it isn't on the path ordinary CLI usage takes.
        Expr::Format(format_type) => format_result::<S, _>(
            format_type,
            &owned_or_suppress!(to_owned_with_cursor(&value, cursor), optional),
            optional,
        ),

        Expr::Builtin(builtin) => eval_builtin::<S, _>(builtin, value, optional, cursor),

        // Comparison operations: routed through the shared lazy fanout
        // machinery (#1481) rather than a hand-rolled eager loop --
        // `eval_compare_generic` drives `binary_fanout_each_generic` with
        // `eval_each_generic` as the operand strategy and collects its
        // demand-driven output into a `GenericResult`, mirroring `eval.rs`'s
        // split between `eval_each`'s lazy `Expr::Compare` arm and a
        // collecting wrapper over the same loop (`binary_fanout_core`/
        // `eval_binary_fanout`). Right operand outer, left operand inner,
        // forking over every pairing (#768) -- unchanged from before; what
        // changes is *when* each operand runs: `right`'s next candidate is no
        // longer pulled before `left` has fully run against the previous one,
        // so `("A"|stderr) == (("B"|stderr), ("C"|stderr))` now writes
        // `B A C A`, matching jq, instead of finishing right first (`B C A
        // A`).
        Expr::Compare { op, left, right } => {
            eval_compare_generic::<S, V>(*op, left, right, value, optional, cursor)
        }

        // Array construction: collect every output of the inner expression
        // into one array. Handled natively (mirrors `eval::eval_array_construction`)
        // so a builtin with its own cursor-native, duplicate-key-preserving fix
        // (e.g. #443's `to_entries`) keeps that fix when wrapped in `[...]`,
        // instead of losing it to the wildcard fallback's whole-document
        // `to_owned()`, which collapses duplicate mapping keys before the
        // wrapped expression ever runs (#1168).
        // #1687: `reduce`/`foreach` had no arm here at all, so every one of
        // them bridged the whole document through an `IndexMap`-backed
        // `OwnedValue` before `input` was evaluated -- collapsing duplicate
        // mapping keys the *input generator itself* would have walked.
        // `reduce (keys|.[]) as $k (0; .+1)` on `b: 1\na: 2\nb: 3\n`
        // answered 2 where `[keys|.[]] | length` on the same document
        // answers 3, an internal contradiction rather than merely a
        // divergence. Evaluating `input`/INIT through the generic sink here
        // fixes the count; the fold itself is then `eval.rs`'s, unchanged.
        //
        // See `stream_owned_outputs_generic` for what this does *not* fix:
        // the accumulator and every bound `$x` stay `OwnedValue`, so a
        // duplicate key inside a bound element still collapses at the bind.
        Expr::Reduce {
            input,
            patterns,
            init,
            update,
        } if !streams_unbounded(input) && !streams_unbounded(init) => {
            let (input_values, input_control) =
                stream_owned_outputs_generic::<S, V>(input, value.clone(), optional, cursor);
            // `reduce`'s output is single-shot -- it emits only the final
            // accumulator, never an intermediate -- so a control anywhere in
            // the input stream discards the prefix and propagates alone.
            // Mirrors `eval::eval_reduce`'s identical arms, `optional`
            // suppression included (a decode failure is never suppressed,
            // #1620/#1902, which `suppresses` already encodes).
            if let Some(control) = input_control {
                return match control {
                    Control::Error(e) if suppresses(&e, optional) => GenericResult::None,
                    Control::Error(e) => GenericResult::Error(e),
                    Control::Break(label) => GenericResult::Break(label),
                    Control::Halt(code) => GenericResult::Halt(code),
                };
            }
            let (init_values, init_control) =
                stream_owned_outputs_generic::<S, V>(init, value, optional, cursor);
            query_result_to_generic::<V>(eval_reduce_with_values::<Vec<u64>, S>(
                patterns,
                update,
                input_values,
                init_values,
                init_control,
                optional,
            ))
        }

        // `foreach`'s twin of the arm above, with `eval::eval_foreach`'s own
        // difference preserved: unlike `reduce` it emits per step, so a
        // `Partial` input stream's already-produced prefix is still iterated
        // and `input_control` is carried alongside it rather than replacing
        // it. `eval_foreach_with_values` owns that precedence (it folds
        // `input_control` in ahead of INIT's), so it is passed through
        // untouched here rather than re-decided.
        Expr::Foreach {
            input,
            patterns,
            init,
            update,
            extract,
        } if !streams_unbounded(input) && !streams_unbounded(init) => {
            let (input_values, input_control) =
                stream_owned_outputs_generic::<S, V>(input, value.clone(), optional, cursor);
            let (init_values, init_control) =
                stream_owned_outputs_generic::<S, V>(init, value, optional, cursor);
            query_result_to_generic::<V>(eval_foreach_with_values::<Vec<u64>, S>(
                patterns,
                update,
                extract.as_deref(),
                input_values,
                input_control,
                init_values,
                init_control,
                optional,
            ))
        }

        Expr::Array(inner) => {
            let items: Vec<OwnedValue> = match eval_single::<S, _>(inner, value, optional, cursor)
                .materialize_lazy()
            {
                GenericResult::One(v) => vec![owned_or_err!(to_owned(&v))],
                GenericResult::OneCursor(c) => vec![owned_or_err!(to_owned_cursor(&c))],
                GenericResult::Many(vs) => owned_or_err!(to_owned_all(&vs)),
                GenericResult::ManyCursor(cs) => owned_or_err!(to_owned_all_cursors(&cs)),
                GenericResult::None => Vec::new(),
                GenericResult::Owned(v) => vec![v],
                GenericResult::ManyOwned(vs) => vs,
                // `Break`/`Partial`-`Break` (below): kept for exhaustiveness
                // over `GenericResult`, mirroring `eval::eval_array_construction`'s
                // own arms, but not reachable via any query this CLI can
                // currently parse -- `break $out` needs an enclosing
                // `label $out`, and `Expr::Label` has no native `eval_single`
                // arm of its own (see #1168's scope-widening comment), so any
                // query where a `break` could reach *past* this arm's own
                // boundary necessarily puts `Label` above `Array` in the
                // tree, which routes the *whole* expression through the
                // wildcard fallback below before this arm ever runs. Same
                // "unreachable but exhaustive" shape #1064 documents
                // elsewhere in this codebase.
                //
                // Array construction is atomic in jq (verified:
                // `[1,error("x"),3]` produces no output at all, not a
                // partial array) -- a bare `Error`/`Break`/`Halt` and its
                // `Partial` sibling return the identical result (the
                // `Partial` prefix is unconditionally discarded), so they
                // merge via or-patterns. Mirrors `eval::eval_array_construction`'s
                // identical arms.
                GenericResult::Error(e) | GenericResult::Partial(_, Control::Error(e)) => {
                    return GenericResult::Error(e)
                }
                GenericResult::Break(label) | GenericResult::Partial(_, Control::Break(label)) => {
                    return GenericResult::Break(label)
                }
                GenericResult::Halt(code) | GenericResult::Partial(_, Control::Halt(code)) => {
                    return GenericResult::Halt(code)
                }
                GenericResult::LazyKeys { .. }
                | GenericResult::LazyIndexRange(_)
                | GenericResult::LazySeq(_) => {
                    unreachable!("materialize_lazy() already normalized every lazy variant")
                }
            };
            // Fixed up as one whole unit, not per-source (#953/#1168, see
            // `yq_float_fidelity_fixup`'s own doc comment for why -- in
            // short, a builtin's own construction around a document value
            // (`to_entries`, ...) is indistinguishable here from a genuinely
            // computed one, so the fixup can't be scoped any narrower than
            // this without missing that case).
            match yq_float_fidelity_fixup::<S, _>(items) {
                Ok(fixed) => GenericResult::Owned(OwnedValue::Array(fixed)),
                Err(result) => result,
            }
        }

        // Comma: evaluate each operand in source order against the ambient
        // `optional` (mirrors `eval::eval_comma`, minus its borrowed/owned
        // fast path -- `Expr::Compare` above already establishes the
        // "collect into an owned Vec" shape for a multi-operand native arm
        // in this file). A sibling's own `Error`/`Break`/`Halt`/`Partial`
        // always propagates as a `Partial` carrying whatever prefix already
        // ran (#400) -- unlike `Expr::Array` above, comma is not atomic, and
        // unlike `Expr::Compare`'s `finish_fork_generic` calls, this never
        // consults `optional` itself: an ambient `?` catches the aggregate
        // result exactly once at `Expr::Optional`'s own arm, same as
        // `eval::eval_comma` defers to `eval::eval_try`. Handled natively so
        // a cursor-native builtin's fix doesn't lose it to the wildcard
        // fallback just for being joined with `,` either (#1168).
        //
        // `yq_float_fidelity_fixup` runs once over the whole collected `out`
        // after the loop, not per-sibling -- same reasoning as `Expr::Array`
        // above, and cheaper (one round trip for `.a, .b, .c` instead of up
        // to three).
        Expr::Comma(exprs) => {
            let mut out: Vec<OwnedValue> = Vec::new();
            for expr in exprs {
                let result = eval_single::<S, _>(expr, value.clone(), optional, cursor);
                if let Some(control) = push_generic_owned_values(result, &mut out) {
                    return match yq_float_fidelity_fixup::<S, _>(out) {
                        Ok(fixed) => partial_generic(fixed, control),
                        Err(result) => result,
                    };
                }
            }
            match yq_float_fidelity_fixup::<S, _>(out) {
                Ok(fixed) => owned_vec_to_generic_result(fixed),
                Err(result) => result,
            }
        }

        // Fall back to the full evaluator for complex expressions
        _ => {
            // Convert to OwnedValue, then to JSON, then evaluate with full evaluator
            let owned = owned_or_err!(to_owned_with_cursor(&value, cursor));
            let json_str = owned.to_json_for_reindex::<S>();
            let json_bytes = json_str.as_bytes();
            let index = JsonIndex::build(json_bytes);
            let cursor = index.root(json_bytes);

            // No `if optional { wrap in Expr::Optional }` here (see
            // `eval_on_owned`'s matching comment): this `_` arm is only
            // reached when `expr` itself isn't one of the natively-matched
            // variants above, and after #693 the only way this function
            // (`eval_single`) is ever called with `optional = true` is the
            // `IndexExpr`/`SliceExpr` special case a few arms up — which
            // matches *before* falling through to this wildcard, so it never
            // lands here. Every other caller threads the ambient `optional`,
            // which starts `false` at every public entry point. Whatever
            // `Error` `full_eval` returns below is caught, if at all, by the
            // *caller's* own `Expr::Optional`/`eval_try`-style boundary.

            // Evaluate using the full evaluator
            //
            // Every `Err(e)` arm below (#1192) is defense-in-depth, same as
            // `eval_on_owned`'s matching comment: `cursor` is rooted at a
            // fresh serialization (`to_json_for_reindex`) of `owned`, and
            // `owned` itself already went through `to_owned_with_cursor`
            // above -- any malformed string in the *original* document was
            // already silently degraded there (that gap is #1192's
            // remaining, out-of-scope half, tracked as #1247), not carried
            // through as malformed bytes for `owned_from_standard_json` to
            // ever encounter here.
            match full_eval::<Vec<u64>, S>(expr, cursor) {
                QueryResult::One(v) => {
                    // Convert StandardJson back to OwnedValue
                    match owned_from_standard_json(&v) {
                        Ok(o) => GenericResult::Owned(o),
                        Err(e) => GenericResult::Error(e),
                    }
                }
                QueryResult::OneCursor(c) => match owned_from_standard_json(&c.value()) {
                    Ok(o) => GenericResult::Owned(o),
                    Err(e) => GenericResult::Error(e),
                },
                // See the matching arm in `eval_on_owned` above for why this
                // stops at the first decode failure instead of skipping it.
                QueryResult::Many(vs) => {
                    let mut out = Vec::new();
                    let mut failure = None;
                    for v in &vs {
                        match owned_from_standard_json(v) {
                            Ok(o) => out.push(o),
                            Err(e) => {
                                failure = Some(e);
                                break;
                            }
                        }
                    }
                    match failure {
                        Some(e) => partial_generic(out, Control::Error(e)),
                        None => GenericResult::ManyOwned(out),
                    }
                }
                QueryResult::None => GenericResult::None,
                QueryResult::Error(e) => GenericResult::Error(e),
                QueryResult::Owned(v) => GenericResult::Owned(v),
                QueryResult::ManyOwned(vs) => GenericResult::ManyOwned(vs),
                QueryResult::Break(label) => GenericResult::Break(label),
                QueryResult::Halt(code) => GenericResult::Halt(code),
                QueryResult::Partial(vs, control) => GenericResult::Partial(vs, control),
            }
        }
    }
}

/// One output on its way to the sink installed by [`eval_each_generic`]
/// (#1461, mirroring `eval::Item`).
///
/// Matches the six single-output shapes of [`GenericResult`] plus one extra,
/// `OneCursorValue` -- `None`/`Error`/`Break`/`Halt`/`Partial`/`Many`/
/// `ManyCursor`/`ManyOwned` are multi-output or terminal and are never
/// represented as one item; [`drain_result_generic`] handles those directly.
/// `LazyKeys`/`LazyIndexRange`/`LazySeq` are carried opaque rather than
/// decomposed: only [`fold_pipe_stages`]'s own per-variant switch knows how
/// to thread one of them through a *further* pipe stage (its
/// `map`/`select`/`first`/`.[n]` composability fast paths, #724/#725) --
/// collapsing one to `Owned`/`One` before that switch runs is exactly the
/// regression #1503 review found and reverted. No `Debug` derive, matching
/// `eval::Item`'s own omission: `V::Cursor` has no guaranteed `Debug` bound
/// (see `LazyElem`'s own comment above for why that is deliberate here too).
///
/// `OneCursorValue(V::Cursor, V)` has no [`GenericResult`] counterpart --
/// it exists purely as a per-item optimization at one construction site,
/// [`each_lazy_keys_iterate_sink`]'s `!sorted` arm. `DistinctKeyCursors`
/// already decodes each key's value as a side effect of walking (it needs
/// it for duplicate-key hashing), so pairing that value with its cursor
/// here lets [`continue_pipe_element_generic`] skip a second, identical
/// `V::Cursor::value()` resolve -- for YAML specifically not a cheap
/// re-read but a full scalar decode (#1609). **Do not use this variant
/// anywhere else.** Every other `OneCursor` site (`.[]` iteration via
/// `uncons_cursor`, etc.) has no value decoded yet, so pairing one in would
/// be new cost, not a freebie -- `OneCursor` alone stays correct there.
enum GenericItem<V: DocumentValue> {
    One(V),
    OneCursor(V::Cursor),
    /// See the enum doc comment -- only ever constructed by
    /// [`each_lazy_keys_iterate_sink`]'s streaming `keys_unsorted` arm.
    OneCursorValue(V::Cursor, V),
    Owned(OwnedValue),
    LazyKeys {
        fields: V::Fields,
        sorted: bool,
        collapse: bool,
    },
    LazyIndexRange(usize),
    /// Boxed for the same reason as `GenericResult::LazySeq` (#789) -- see
    /// that variant's doc comment for the mechanism. This sibling enum is
    /// pushed once per *item* through every sink in the streaming path
    /// (`push_one_generic`, `each_*_iterate_sink`), an even hotter path than
    /// `GenericResult`'s own per-stage one.
    LazySeq(Box<LazySeq<V>>),
}

/// Push one item to `sink`, translating its `Demand` into a terminal `Flow`.
fn push_one_generic<V: DocumentValue>(
    item: GenericItem<V>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    match sink(item) {
        Demand::Continue => Flow::Exhausted,
        Demand::Stop => Flow::Stopped { pending: None },
    }
}

/// Push a sequence of already-materialized items to `sink`, stopping as soon
/// as it says to. Shared by [`drain_result_generic`]'s `Many`/`ManyCursor`/
/// `ManyOwned`/`Partial` arms, which only differ in how the `Vec` they
/// already hold gets wrapped into a `GenericItem`.
fn push_many_generic<V: DocumentValue>(
    items: impl Iterator<Item = GenericItem<V>>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    for item in items {
        if sink(item) == Demand::Stop {
            return Flow::Stopped { pending: None };
        }
    }
    Flow::Exhausted
}

/// Generic-evaluator twin of `eval::drain_result` (#1461): adapt an
/// already-computed [`GenericResult`] into one-at-a-time sink pushes,
/// checking `Demand` between values. The fallback for every `Expr` shape
/// [`eval_each_generic`] has no dedicated lazy arm for.
///
/// `Many`/`ManyCursor`/`ManyOwned` are already a `Vec` by the time
/// `eval_single` returns them (e.g. `.[]`'s cursor enumeration is cheap
/// navigation, not evaluation of side-effecting code), so iterating with an
/// early `Demand::Stop` check is correct and sufficient -- only feeding an
/// element to `sink`, which may recurse into more of the pipe, can run
/// side-effecting code, and that is demand-checked per element.
/// `LazyKeys`/`LazyIndexRange`/`LazySeq` are pushed as one opaque item each,
/// never decomposed here -- see [`GenericItem`]'s own doc comment for why.
fn drain_result_generic<V: DocumentValue>(
    result: GenericResult<V>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    match result {
        GenericResult::None => Flow::Exhausted,
        GenericResult::One(v) => push_one_generic(GenericItem::One(v), sink),
        GenericResult::OneCursor(c) => push_one_generic(GenericItem::OneCursor(c), sink),
        GenericResult::Owned(o) => push_one_generic(GenericItem::Owned(o), sink),
        GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        } => push_one_generic(
            GenericItem::LazyKeys {
                fields,
                sorted,
                collapse,
            },
            sink,
        ),
        GenericResult::LazyIndexRange(len) => {
            push_one_generic(GenericItem::LazyIndexRange(len), sink)
        }
        GenericResult::LazySeq(seq) => push_one_generic(GenericItem::LazySeq(seq), sink),
        GenericResult::Many(vs) => push_many_generic(vs.into_iter().map(GenericItem::One), sink),
        GenericResult::ManyCursor(cs) => {
            push_many_generic(cs.into_iter().map(GenericItem::OneCursor), sink)
        }
        GenericResult::ManyOwned(os) => {
            push_many_generic(os.into_iter().map(GenericItem::Owned), sink)
        }
        GenericResult::Error(e) => Flow::Escaped(Control::Error(e)),
        GenericResult::Break(label) => Flow::Escaped(Control::Break(label)),
        GenericResult::Halt(code) => Flow::Escaped(Control::Halt(code)),
        // `vs` is delivered first, exactly as `push_generic_owned_values`
        // already delivers it; a sink that stops part-way carries `control`
        // forward as `pending` rather than dropping it, same as
        // `eval::drain_result`'s own `Partial` arm.
        GenericResult::Partial(vs, control) => {
            match push_many_generic(vs.into_iter().map(GenericItem::Owned), sink) {
                Flow::Exhausted => Flow::Escaped(control),
                Flow::Stopped { .. } => Flow::Stopped {
                    pending: Some(control),
                },
                Flow::Escaped(_) => unreachable!("push_many_generic never returns Escaped"),
            }
        }
    }
}

/// Generic-evaluator twin of `eval::eval_each` (#1461): a demand-driven sink
/// over `Comma`/`Pipe`/`Paren`, so a consumer like `first` can stop pulling
/// from the *rest* of one of these shapes as soon as it has what it needs,
/// instead of `eval_single`'s own eager, always-materialize-everything
/// dispatch for the same three variants.
///
/// Scoped to `Comma`/`Pipe`/`Paren`/`Compare`/`Arithmetic`/`If`/`Try`/
/// `Optional`/`Label`/`As`/`AsPattern`/`FuncDef`/`Limit` -- every other
/// `Expr` shape still falls to the eager `_` fallback below. `Range`'s own
/// `from`/`to`/`step` bounds are a known, out-of-scope residual: `first(
/// range(1, ("B"|stderr); 5))` still leaks (live-verified against jq 1.7.1),
/// tracked separately rather than folded into this arm set.
///
/// `Compare` and `Arithmetic` both gained native arms for #1481, mirroring
/// `eval.rs`'s own pair (#1459/Stage 4 for `Compare`, #1481 for
/// `Arithmetic`). They close the interleaving gap both for a bare top-level
/// binary operator (`eval_single`'s own `Expr::Compare` arm routes through
/// this same machinery via `eval_compare_generic`; `Expr::Arithmetic` has no
/// native `eval_single` arm and instead reaches `eval.rs`'s already-fixed
/// `eval_binary_fanout` through the `eval_on_owned` bridge) and for one
/// reached through this module's own lazy consumers (`first`/`last`), which
/// previously fell through to the eager `_` fallback for any
/// non-`Comma`/`Pipe`/`Paren` argument.
///
/// **Both operators need arms, not just `Compare`.** They share one loop, so
/// a fix that stopped at `Compare` left the exactly-parallel arithmetic
/// spelling divergent: `first(10 + (1, ("B"|stderr)))` still ran the
/// side-effecting candidate `first` never asked for, while
/// `first(10 == (1, ("B"|stderr)))` no longer did (both oracle-verified
/// against pinned jq 1.7.1; pinned together in
/// `test_short_circuit_side_effect_shapes_already_match_jq_820`).
///
/// `If`/`Try`/`Optional`/`Label`/`As`/`AsPattern`/`FuncDef`/`Limit` gained
/// native arms for #1596, mirroring `eval.rs`'s own Stage 5 arm set
/// (`docs/plan/jq-lazy-generator-consumers.md`) -- see [`each_if_generic`],
/// [`each_try_generic`], [`each_label_generic`], [`each_as_generic`],
/// [`each_as_pattern_generic`] and [`each_limit_generic`]. `first`/`last` are
/// the only consumers routed through this file's own native fast path rather
/// than bouncing to `eval.rs`'s already-lazy `eval_each`, so they were the
/// only ones Stage 5 (`eval.rs`'s own widening) never reached; the seven
/// shapes are pinned in `test_short_circuit_side_effect_shapes_already_match_jq_820`,
/// not the leaks table.
fn eval_each_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    match expr {
        // Mirrors `eval::eval_each`'s own `Comma` arm exactly: the sink *is*
        // the accumulator, so there is nothing to promote/collect here.
        Expr::Comma(exprs) => {
            for e in exprs {
                match eval_each_generic::<S, V>(e, value.clone(), optional, cursor, sink) {
                    Flow::Exhausted => {}
                    stopped_or_escaped => return stopped_or_escaped,
                }
            }
            Flow::Exhausted
        }
        Expr::Pipe(exprs) => eval_each_pipe_generic::<S, V>(exprs, value, optional, cursor, sink),
        Expr::Paren(inner) => eval_each_generic::<S, V>(inner, value, optional, cursor, sink),
        // #1481: mirrors `eval.rs`'s own `eval_each` `Expr::Compare` arm
        // (#1459, Stage 4) -- `binary_fanout_each_generic` owns the loop
        // order; passing `eval_each_generic` as the operand strategy is what
        // lets the *right* operand -- jq's outer loop -- stop producing
        // candidates a consumer (e.g. `first`) never asked for.
        //
        // `left`/`right` are evaluated at a hardcoded `optional: false`, not
        // the ambient `optional` -- matching this arm's pre-#1481 eager
        // predecessor and `eval_single`'s own `Expr::IndexExpr` key/bounds
        // split (#693): an enclosing `?` must not mask an unrelated error
        // deep in an operand's own subtree. The ambient `optional` is still
        // consulted, exactly once, by whichever caller collects this arm's
        // `Flow` (`eval_compare_generic`'s `finish_fork_generic` call, or a
        // wrapping consumer's own demand sink).
        Expr::Compare { op, left, right } => binary_fanout_each_generic::<V>(
            |operand, operand_sink| {
                eval_each_generic::<S, V>(operand, value.clone(), false, cursor, operand_sink)
            },
            left,
            right,
            optional,
            |left_val, right_val| {
                Ok(OwnedValue::Bool(apply_compare_op::<S>(
                    *op, &left_val, &right_val,
                )))
            },
            sink,
        ),
        // #1481: the `Expr::Compare` arm's twin, identical but for the
        // `combine` it supplies -- they share `binary_fanout_each_generic`
        // the same way `eval.rs`'s `Expr::Compare`/`Expr::Arithmetic` arms
        // share `binary_fanout_each`, so every note above (loop order,
        // hardcoded `optional: false` on the operands, where the ambient
        // `optional` is consulted instead) applies verbatim here.
        //
        // Unlike `apply_compare_op`, [`arith_combine`] is genuinely fallible
        // (`"a" + 1`, `1 / 0`), so this is the arm that first exercises
        // `binary_fanout_each_generic`'s `Err(e)` path -- the one its own
        // comment said was written generically and left ready for a fallible
        // caller.
        Expr::Arithmetic { op, left, right } => binary_fanout_each_generic::<V>(
            |operand, operand_sink| {
                eval_each_generic::<S, V>(operand, value.clone(), false, cursor, operand_sink)
            },
            left,
            right,
            optional,
            |left_val, right_val| arith_combine::<S>(*op, left_val, right_val),
            sink,
        ),

        // #1596: `cond` stays eager (branch *selection* was already lazy --
        // only the taken branch's own body wasn't), mirroring `eval.rs`'s
        // `each_if`/`Expr::If` pair exactly -- see [`each_if_generic`].
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => each_if_generic::<S, V>(
            cond,
            then_branch,
            else_branch,
            value,
            optional,
            cursor,
            sink,
        ),

        // Same `.[EXPR]?`/`.[S:E]?` carve-out as `eval_single`'s identical
        // arm above (#693) -- `?` here guards only the final index/slice
        // step, not the key/bounds sub-expression, so route straight through
        // rather than via [`each_try_generic`], which would catch the
        // key/bounds error too.
        Expr::Optional(inner)
            if matches!(**inner, Expr::IndexExpr { .. } | Expr::SliceExpr { .. }) =>
        {
            eval_each_generic::<S, V>(inner, value, true, cursor, sink)
        }
        // `E?` is sugar for `try E` (no catch handler) -- same ambient-
        // `optional` forwarding as `eval_single`'s own arm (#693).
        Expr::Optional(inner) => {
            each_try_generic::<S, V>(inner, None, value, optional, cursor, sink)
        }
        Expr::Try { expr, catch } => {
            each_try_generic::<S, V>(expr, catch.as_deref(), value, optional, cursor, sink)
        }

        Expr::Label { name, body } => {
            each_label_generic::<S, V>(name, body, value, optional, cursor, sink)
        }

        // The parser builds `Expr::As` for the bare-`$var` spelling and
        // reserves `Expr::AsPattern` for anything destructured -- both need
        // an arm, same as `eval.rs`'s own pair (see that pair's doc comments
        // for the live-verified parse confirmation).
        Expr::As { expr, var, body } => {
            each_as_generic::<S, V>(expr, var, body, value, optional, cursor, sink)
        }
        Expr::AsPattern {
            expr,
            patterns,
            body,
        } => each_as_pattern_generic::<S, V>(expr, patterns, body, value, optional, cursor, sink),

        // `def name(params): body; then` is pure AST substitution -- no
        // producer logic of its own to make demand-aware -- so routing the
        // expanded tree back through `eval_each_generic` instead of
        // `eval_single` is the whole fix, mirroring `eval.rs`'s identical
        // arm.
        Expr::FuncDef {
            name,
            params,
            body,
            then,
            bound,
        } => {
            let bound_then = bind_def(name, params, body, then, bound);
            eval_each_generic::<S, V>(&bound_then, value, optional, cursor, sink)
        }

        // #1371: mirrors `eval.rs`'s own `DefCall`/`Shared` pair -- see there
        // for why the generic fallback is not good enough (a consumer that
        // stops early must not have already run the rest of the body).
        Expr::DefCall {
            def,
            args,
            frames,
            bound,
        } => match bind_def_call(def, args, *frames, bound) {
            Ok(bound) => {
                let _guard = enter_def_call_frame(*frames);
                eval_each_generic::<S, V>(bound, value, optional, cursor, sink)
            }
            Err(e) => Flow::Escaped(Control::Error(e)),
        },
        // Same split as `eval.rs`'s own `Shared` arm, for the same measured
        // reason -- see there. A link in a recursion's own argument chain
        // takes the cheaper eager path (it is single-valued, so no demand is
        // lost); an argument the user wrote stays lazy, because *its*
        // laziness is observable. A bare pass-through (`inner` is itself a
        // `Shared` -- a parameter threaded unchanged through another level,
        // not a computed chain link) can be arbitrary user code underneath,
        // so it is peeled by re-entering this same arm rather than taken as
        // settled -- see `eval.rs`'s identical split for the full reasoning.
        Expr::Shared(inner) if matches!(&**inner, Expr::Shared(_)) => {
            eval_each_generic::<S, V>(inner, value, optional, cursor, sink)
        }
        Expr::Shared(inner) => {
            if is_pure_chain_link(inner) {
                drain_result_generic(eval_single::<S, V>(inner, value, optional, cursor), sink)
            } else {
                eval_each_generic::<S, V>(inner, value, optional, cursor, sink)
            }
        }

        // See `each_limit_generic`'s own doc comment.
        Expr::Limit { n, expr } => {
            each_limit_generic::<S, V>(n, expr, value, optional, cursor, sink)
        }

        // #1597 part 2: `.[]` over an array, walked lazily -- see
        // `each_lazy_array_iterate_sink`'s own doc comment. Every other
        // shape (object, non-container, decode failure) still falls back
        // to `eval_single`'s existing, correct-but-eager handling -- an
        // object's own duplicate-key collapse semantics need
        // `DistinctKeyCursors`' streaming-collapse machinery extended to
        // also carry value cursors, a separate and larger change not
        // attempted here.
        Expr::Iterate => match value.as_array() {
            Some(elements) => each_lazy_array_iterate_sink(elements, sink),
            None => drain_result_generic(eval_single::<S, V>(expr, value, optional, cursor), sink),
        },

        // #2014: `repeat(f)` has no native arm in this evaluator at all --
        // the `_` fallback below evaluates it *eagerly*, which bridges all
        // the way to `eval.rs`'s own `eval_repeat`, whose `MAX_ITERATIONS`
        // round cap exists only as a hang backstop for when nothing
        // demand-drives it (see that function's own doc comment). Reached
        // through this fallback, `limit`/`first`/etc. above already stopped
        // pulling by the time they'd see any of it -- `eval_repeat` ran to
        // its own 1000-round cap (now raising, not silently truncating)
        // before this arm's own caller ever got a chance to ask for fewer.
        // Fixing this needs a *native* arm here, mirroring `eval.rs`'s own
        // `each_repeat` fix for the identical bug on that evaluator's side
        // (`Expr::Repeat`'s own arm in `eval_each`) -- one bridging to the
        // owned-domain `eval_each_owned` per round rather than reproducing
        // the whole pull loop against `V: DocumentValue` directly.
        Expr::Repeat(f) => each_repeat_generic::<S, V>(f, value, optional, cursor, sink),

        _ => drain_result_generic(eval_single::<S, V>(expr, value, optional, cursor), sink),
    }
}

/// Demand-driven `Expr::Repeat` arm for the generic evaluator (#2014) --
/// the generic-evaluator twin of `eval.rs`'s own `each_repeat`. Decodes
/// `value` once (checked, matching `eval.rs`'s `eval_repeat`'s identical
/// up-front decode), then pulls one round of `f`'s outputs at a time via
/// the already-lazy `eval_each_owned` (the owned-domain twin of `eval_each`
/// this file already imports for exactly this "loop back into eval.rs's
/// lazy machinery on an owned snapshot" shape), stopping as soon as the
/// wrapping `sink` does.
///
/// Two independent caps, mirroring `eval.rs`'s own `each_repeat` exactly:
/// a per-*round* `REPEAT_WIDTH_BUDGET` budget, reset at the start of
/// every round and charged one output at a time via `charge_budget` --
/// bounding how many values a single round may fork into at once (a
/// succinctly-only memory-safety net for a wide `f` like `.[]`, not jq
/// fidelity -- see `eval.rs`'s `each_repeat` doc comment for the live
/// oracle checks: real jq has no such cap, and charging this cumulatively
/// across rounds instead of resetting per round was tried and silently
/// reintroduces the exact `limit(80000; repeat(1))`-style truncation #2014
/// exists to fix). This *raises* on exhaustion, matching
/// `resolve_repeat_bounded`'s path-context sibling so `path(repeat(f))`
/// and `repeat(f)` agree on the same wide-round wall at the same count
/// (`test_path_repeat_width_budget_matches_value_mode_1933`). A separate
/// per-round-count `MAX_EMPTY_REPEAT_ROUNDS` cap counts consecutive rounds
/// that produce nothing at all (reset by any productive round), which
/// *silently* returns `Flow::Exhausted` instead -- `repeat`'s `f` reruns
/// against the same unchanging input every round, so once a round produces
/// nothing it never will, and no wrapping consumer's `Demand::Stop` can
/// ever fire to save it (`sink` is never called on a zero-output round).
/// Raising there instead would contradict
/// `test_repeat_empty_expr_yields_nothing_instead_of_looping_forever_on_nulls`'s
/// own pinned convention: a bare `repeat(empty)` has no jq oracle answer
/// (it hangs forever there too), so succinctly silently yields nothing
/// rather than erroring.
fn each_repeat_generic<S: EvalSemantics, V: DocumentValue>(
    f: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let owned = match to_owned_with_cursor(&value, cursor) {
        Ok(v) => v,
        Err(e) if optional && !e.is_decode_failure() => return Flow::Exhausted,
        Err(e) => return Flow::Escaped(Control::Error(e)),
    };
    let mut empty_rounds = 0usize;
    loop {
        let mut stopped = false;
        let mut produced_any = false;
        let mut budget_control = None;
        let mut budget = super::eval::REPEAT_WIDTH_BUDGET;
        let flow = eval_each_owned::<S>(f, &owned, optional, &mut |v| {
            produced_any = true;
            if let Some(control) = super::eval::charge_budget(&mut budget, "repeat") {
                budget_control = Some(control);
                stopped = true;
                return Demand::Stop;
            }
            match sink(GenericItem::Owned(v)) {
                Demand::Continue => Demand::Continue,
                Demand::Stop => {
                    stopped = true;
                    Demand::Stop
                }
            }
        });
        if let Some(control) = budget_control {
            return Flow::Escaped(control);
        }
        if stopped {
            return Flow::Stopped { pending: None };
        }
        if !produced_any && matches!(flow, Flow::Exhausted) {
            empty_rounds += 1;
            if empty_rounds >= super::eval::MAX_EMPTY_REPEAT_ROUNDS {
                return Flow::Exhausted;
            }
            continue;
        }
        empty_rounds = 0;
        match flow {
            Flow::Exhausted => continue,
            other => return other,
        }
    }
}

/// The stages after the current one, plus the owned `Expr::Pipe` copy an
/// `Owned` element needs -- built at most once, and only if such an element
/// actually arrives (#1598).
///
/// `eval_each_owned` takes an `&Expr`, and the only way to present a `rest`
/// *slice* as one is to own a copy: a `Vec` allocation plus a recursive
/// `Expr` clone per stage. Doing that inside the per-element call meant
/// paying it once per element; a driver builds one of these instead and
/// reuses it for its whole loop.
///
/// The slice and its owned copy are one value rather than two parameters on
/// purpose. Correctness requires that a cached pipe is only ever used with
/// the `rest` it was built from -- a mismatch would evaluate elements
/// against the wrong stages, which is a wrong answer rather than a crash.
/// Pairing them here makes that mismatch unrepresentable instead of relying
/// on every call site to keep two arguments in step.
struct RestPipe<'a> {
    stages: &'a [Expr],
    owned: Option<Expr>,
}

impl<'a> RestPipe<'a> {
    fn new(stages: &'a [Expr]) -> Self {
        Self {
            stages,
            owned: None,
        }
    }

    /// The stages themselves, for the arms that can consume a slice.
    fn stages(&self) -> &'a [Expr] {
        self.stages
    }

    /// The same stages as one owned `Expr`, cloning on first use only.
    fn owned(&mut self) -> &Expr {
        let stages = self.stages;
        self.owned
            .get_or_insert_with(|| Expr::Pipe(stages.to_vec()))
    }
}

/// Continue a pipe through one already-produced item: recurse the *rest* of
/// the pipe against it, honoring `sink`'s `Demand` throughout. Shared by
/// [`eval_each_pipe_generic`]'s own driver and [`fold_pipe_stages_sink`]'s
/// per-element fan-out handling (#1565), so "thread one pulled value through
/// an arbitrary-length remaining pipe" exists in exactly one place.
///
/// An `Owned` item bridges to the already-lazy `eval::eval_each_owned` so
/// laziness (and thus `input`/`inputs` demand) continues through the rest of
/// the pipe instead of stopping at the owned/cursor boundary, mirroring
/// `eval_each_pipe`'s own `Item::Owned` arm (PR #1450 fixed the equivalent
/// bug on the `eval.rs` side). A `LazyKeys`/`LazyIndexRange`/`LazySeq` item
/// (never produced for a single pulled element by either caller today, but
/// handled for robustness) folds through the remaining stages via
/// [`fold_pipe_stages_sink`] rather than being decomposed or materialized
/// here -- the #1503-safe move, same rationale as the top-level `Pipe`
/// dispatch below.
fn continue_pipe_element_generic<S: EvalSemantics, V: DocumentValue>(
    item: GenericItem<V>,
    rest: &mut RestPipe<'_>,
    optional: bool,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    match item {
        GenericItem::One(v) => {
            eval_each_pipe_generic::<S, V>(rest.stages(), v, optional, None, sink)
        }
        GenericItem::OneCursor(c) => {
            eval_each_pipe_generic::<S, V>(rest.stages(), c.value(), optional, Some(c), sink)
        }
        // Same as the `OneCursor` arm above, minus the `c.value()` resolve
        // -- `v` was already decoded by whoever built this item (#1609), so
        // re-deriving it here would just repeat that work.
        GenericItem::OneCursorValue(c, v) => {
            eval_each_pipe_generic::<S, V>(rest.stages(), v, optional, Some(c), sink)
        }
        GenericItem::Owned(o) => eval_each_owned::<S>(rest.owned(), &o, optional, &mut |o| {
            sink(GenericItem::Owned(o))
        }),
        item @ (GenericItem::LazyKeys { .. }
        | GenericItem::LazyIndexRange(_)
        | GenericItem::LazySeq(_)) => {
            // Note: this arm cannot use the cache -- `fold_pipe_stages_sink`
            // owns its own stage cursor and rebuilds a pipe from
            // `stages[j..]`, a different slice than `rest`. It is documented
            // as reachable only once per pulled element, so it is not a
            // per-element clone today; #1611 tracks folding it in if that
            // ever changes.
            fold_pipe_stages_sink::<S, V>(
                generic_item_to_result(item),
                rest.stages(),
                optional,
                sink,
            )
        }
    }
}

/// Lazy twin of `eval::each_if` (#1596): `cond` is evaluated eagerly --
/// branch *selection* was already lazy (a fanout over `cond`'s own outputs
/// picks the taken branch per bit) -- but each taken branch's own body is
/// now pushed through [`eval_each_generic`] rather than materialized via
/// `eval_single`, so a generator inside it honours the wrapping consumer's
/// demand (`first(if true then (1,("B"|stderr)) else 9 end)` no longer
/// evaluates the `stderr` candidate). Mirrors `eval.rs`'s `each_if`
/// bit-by-bit walk over every one of `cond`'s outputs (multi-output `cond`,
/// e.g. `if (true,false) then "a" else "b" end`, #378) minus the
/// borrowed/owned accumulator -- `sink` *is* the accumulator here, same as
/// `eval_each_generic`'s own `Comma` arm.
fn each_if_generic<S: EvalSemantics, V: DocumentValue>(
    cond: &Expr,
    then_branch: &Expr,
    else_branch: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let cond_result = eval_single::<S, V>(cond, value.clone(), optional, cursor);
    let mut bits: Vec<bool> = Vec::new();
    let cond_control = push_generic_truthiness(cond_result, &mut bits);

    for bit in bits {
        let branch = if bit { then_branch } else { else_branch };
        match eval_each_generic::<S, V>(branch, value.clone(), optional, cursor, sink) {
            Flow::Exhausted => {}
            stopped_or_escaped => return stopped_or_escaped,
        }
    }

    match cond_control {
        Some(control) => Flow::Escaped(control),
        None => Flow::Exhausted,
    }
}

/// Lazy twin of `eval::each_try` (#1596): pushes `expr`'s own outputs
/// straight to `sink`, then -- only on a bare `Flow::Escaped`, whose own
/// contract guarantees every output produced before it was already
/// delivered -- runs the catch handler (if any) and forwards its outputs
/// too. `catch` runs bound to the raised payload for `Error`, or `null` for
/// `Break` (#562, matching `eval::each_try`'s own note that real jq binds
/// its own internal break marker there instead, not worth replicating).
///
/// A decode failure (#1247) must never be caught here, same #1620 exclusion
/// as `eval_single`'s own `Expr::Optional` arm above. `Halt` is never
/// caught, matching `Control`'s own pass-through guarantee.
///
/// If `sink` itself is satisfied before `expr` would have errored,
/// `eval_each_generic` returns `Flow::Stopped` rather than `Escaped`, and
/// this function propagates it verbatim without ever running `catch` --
/// matching jq exactly: `first(try (1, error("x")) catch "c")` is `1`, and
/// jq never even reaches `error`, let alone `catch`.
///
/// **#1948: a still-lazy `GenericItem` pushed to `sink` must be checked
/// *here*, before this boundary can close.** `eval_each_generic`'s wildcard
/// fallback (`drain_result_generic`) forwards `LazyKeys`/`LazySeq` items
/// opaque and unmaterialized -- the same #1194-class escape #1936/#1812
/// already closed for the pull-model `try_single_generic`, reached through a
/// different dispatch path here. `sink`'s own `Demand` return has no error
/// channel, so [`check_lazy_item_for_try`] reports a fault via the
/// `lazy_fault` side channel below instead (the same "capture, then check
/// once the driver returns" idiom `stream.rs`'s own writers use for a
/// mid-push fault) rather than plumbing one through `Demand` itself.
fn each_try_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    catch: Option<&Expr>,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let mut lazy_fault: Option<Control> = None;
    let flow = eval_each_generic::<S, V>(expr, value, optional, cursor, &mut |item| {
        match check_lazy_item_for_try(item) {
            Ok(item) => sink(item),
            Err(control) => {
                lazy_fault = Some(control);
                Demand::Stop
            }
        }
    });
    let flow = match lazy_fault {
        Some(control) => Flow::Escaped(control),
        None => flow,
    };
    match flow {
        // Same #2254 yq-negative-index-error exclusion as `eval_try`/
        // `each_try`/`try_single_generic` (`src/jq/eval.rs`/this file) --
        // reached from the same `any(.a[-5]?; .)` shape those cover, via
        // this file's own generic dispatch.
        Flow::Escaped(Control::Error(e)) if e.is_uncatchable_at_value_position() => {
            Flow::Escaped(Control::Error(e))
        }
        Flow::Escaped(Control::Error(e)) => match catch {
            Some(catch_expr) => {
                eval_each_owned::<S>(catch_expr, &e.payload(), optional, &mut |o| {
                    sink(GenericItem::Owned(o))
                })
            }
            None => Flow::Exhausted,
        },
        Flow::Escaped(Control::Break(_)) => match catch {
            Some(catch_expr) => {
                eval_each_owned::<S>(catch_expr, &OwnedValue::Null, optional, &mut |o| {
                    sink(GenericItem::Owned(o))
                })
            }
            None => Flow::Exhausted,
        },
        // Halt is never caught (`Control`'s own guarantee); other terminal
        // shapes (`Exhausted`, `Stopped`) pass straight through.
        other => other,
    }
}

/// Forces a #1194-class check on a still-lazy [`GenericItem`] *before* it
/// reaches a push-model sink guarded by `try`/`catch`/`?` (#1948) -- the
/// push-model twin of [`try_single_generic`]'s identical per-variant switch
/// for `GenericResult` (#1936/#1812).
///
/// `LazyKeys` keeps its laziness on success: [`keys_are_well_formed`] only
/// walks and checks, never collects, so a well-formed object still reaches
/// `sink` as a live `LazyKeys` item -- collapsing it to `Owned` here
/// regardless of outcome would be the exact regression #1503's review found
/// and reverted (see [`GenericItem`]'s own doc comment). `LazySeq` cannot be
/// checked without running its buffered `map(f)` closures, so it fully
/// materializes via `materialize_atomic` -- matching `try_single_generic`'s
/// own `LazySeq` arm, and sound for the same reason that one is: a
/// `try`/`catch`/`?` boundary must know *now* whether its body raised, so
/// laziness cannot survive past it regardless of push or pull. `LazyIndexRange`
/// can never fail (`0..len`, pure arithmetic) and passes through unchanged.
///
/// Nested `try`/`catch` boundaries each re-walk the same still-forwarded
/// `LazyKeys` item once per boundary (`try (try (keys_unsorted) catch empty)
/// catch "c"` walks it twice) -- inherited from, not introduced by, this
/// function: `try_single_generic`'s own pull-model `LazyKeys` arm has the
/// identical per-boundary cost already, tracked as a future optimization
/// under #1951 (cache an "already validated" fact on `LazyKeys` itself)
/// rather than fixed at either call site.
fn check_lazy_item_for_try<V: DocumentValue>(
    item: GenericItem<V>,
) -> Result<GenericItem<V>, Control> {
    match item {
        GenericItem::LazyKeys {
            fields,
            sorted,
            collapse,
        } => keys_are_well_formed::<V>(&fields, collapse)
            .map(|()| GenericItem::LazyKeys {
                fields,
                sorted,
                collapse,
            })
            .map_err(Control::Error),
        GenericItem::LazySeq(seq) => seq.materialize_atomic().map(GenericItem::Owned),
        other => Ok(other),
    }
}

/// Lazy twin of `eval::each_label` (#1596): pushes `body`'s outputs straight
/// to `sink`, then -- only on a bare `Flow::Escaped(Control::Break(name))`
/// matching this label, whose contract guarantees every prior output was
/// already delivered -- swallows it. Every other outcome -- a non-matching
/// break, a bare or trailing `Error`/`Halt`, `Exhausted`, or a satisfied
/// `Stopped` -- propagates unchanged.
fn each_label_generic<S: EvalSemantics, V: DocumentValue>(
    name: &str,
    body: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    match eval_each_generic::<S, V>(body, value, optional, cursor, sink) {
        Flow::Escaped(Control::Break(label)) if label == name => Flow::Exhausted,
        other => other,
    }
}

/// Generic-evaluator twin of `eval::materialize_bound_values` (#1596):
/// unpacks a bind expression's own [`GenericResult`] into the values
/// [`each_as_generic`]/[`each_as_pattern_generic`] loop the shared
/// `body`/pattern-match logic over, plus the control the bind itself trails
/// (#400, #494: a `Partial` bind still has its produced prefix bound and run
/// through the body). `Err(flow)` carries the caller's own early return for a
/// bind that produced no values at all, or that raised without producing
/// any -- mirroring that function's `Err(Flow::Exhausted)`/bare-error arms.
///
/// [`push_generic_owned_values`] plays the role `eval::materialize_bound_values`'s
/// own `QueryResult` match plays there: it already folds every `GenericResult`
/// shape (including the three lazy variants, via `materialize_lazy`) into
/// `(Vec<OwnedValue>, Option<Control>)`, so this wrapper only needs the
/// empty-vs-non-empty split `eval.rs`'s version encodes as separate arms.
/// Shared by both [`each_as_generic`] and [`each_as_pattern_generic`] (code
/// review, #1596) rather than each inlining its own copy of this split --
/// the same duplication `eval::materialize_bound_values`'s own doc comment
/// says it was extracted to eliminate between `eval::each_as`/`eval::each_as_pattern`.
fn materialize_bound_values_generic<V: DocumentValue>(
    bound_result: GenericResult<V>,
) -> Result<(Vec<OwnedValue>, Option<Control>), Flow> {
    let mut bound_values: Vec<OwnedValue> = Vec::new();
    let bound_control = push_generic_owned_values(bound_result, &mut bound_values);
    if bound_values.is_empty() {
        return Err(match bound_control {
            Some(control) => Flow::Escaped(control),
            None => Flow::Exhausted,
        });
    }
    Ok((bound_values, bound_control))
}

/// Lazy twin of `eval::each_as` (#1596): the bind expression (`expr`) is
/// evaluated eagerly, exactly as `eval::each_as` already does -- this fix is
/// scoped to what runs *per bound value*, not to the binding itself. Each
/// bound value's `body` is then pushed through [`eval_each_generic`] rather
/// than materialized, so `isempty((1,2) as $x | ($x, ("B"|stderr)))`-shaped
/// binds no longer evaluate the `stderr` branch. The parser reserves this
/// bare-`$var` node for `Expr::As`; [`each_as_pattern_generic`] below is its
/// destructuring sibling (`Expr::AsPattern`, `?//`-chains included).
fn each_as_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    var: &str,
    body: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let bound_result = eval_single::<S, V>(expr, value.clone(), optional, cursor);
    let (bound_values, bound_control) = match materialize_bound_values_generic(bound_result) {
        Ok(pair) => pair,
        Err(flow) => return flow,
    };

    for bound_val in bound_values {
        let substituted_body = substitute_bound_var(expr, body, var, &bound_val);
        match eval_each_generic::<S, V>(&substituted_body, value.clone(), optional, cursor, sink) {
            Flow::Exhausted => {}
            other => return other,
        }
    }

    match bound_control {
        Some(control) => Flow::Escaped(control),
        None => Flow::Exhausted,
    }
}

/// Lazy twin of `eval::each_as_pattern` (#1596): the bind expression
/// (`expr`) is evaluated eagerly, exactly as `eval::each_as_pattern` already
/// does -- same reasoning as [`each_as_generic`] above, its non-destructuring
/// sibling. Each bound value's `body` (after `?//`-alternative substitution)
/// is then pushed through [`eval_each_generic`] rather than materialized.
fn each_as_pattern_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    patterns: &[Pattern],
    body: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let bound_result = eval_single::<S, V>(expr, value.clone(), optional, cursor);
    let (bound_values, bound_control) = match materialize_bound_values_generic(bound_result) {
        Ok(pair) => pair,
        Err(flow) => return flow,
    };

    let mut all_var_names: Vec<String> = Vec::new();
    for pattern in patterns {
        collect_pattern_var_names(pattern, &mut all_var_names);
    }
    all_var_names.sort_unstable();
    all_var_names.dedup();

    for bound_val in bound_values {
        match each_pattern_alternatives_generic::<S, V>(
            patterns,
            &all_var_names,
            body,
            &bound_val,
            &value,
            optional,
            cursor,
            sink,
        ) {
            Flow::Exhausted => {}
            other => return other,
        }
    }

    match bound_control {
        Some(control) => Flow::Escaped(control),
        None => Flow::Exhausted,
    }
}

/// Sink-based twin of `eval::each_pattern_alternatives` (#1596): same
/// `?//`-alternative fallthrough rule (a pattern-match failure, a body
/// error, or a body break tries the next alternative unless this is the
/// last one; halt never falls through), but each alternative's own
/// successful outputs are pushed to `sink` as they're produced instead of
/// collected.
///
/// If `sink` itself is satisfied partway through an alternative,
/// `eval_each_generic` returns `Flow::Stopped` rather than `Escaped`. #1519:
/// that is the *same event* as jq's escaping `break` -- succinctly's
/// short-circuiting builtins are native Rust, so they signal satisfaction as
/// `Demand::Stop` where real jq's `builtin.jq` macros raise `break $out` --
/// so it falls through to the next alternative on exactly the same `is_last`
/// rule, via `eval::is_retryable_stop`. See
/// `eval::each_pattern_alternatives`, whose arm this mirrors.
///
/// **Wraps `sink` the same way `each_try_generic` does (#1948 review):**
/// `eval_each_generic`'s wildcard fallback forwards a still-lazy
/// `GenericItem::LazyKeys`/`LazySeq` to `sink` opaque, which returns
/// `Flow::Exhausted` regardless of whether the item is actually
/// well-formed -- so this loop would otherwise treat a malformed
/// `keys_unsorted`/`map(f)` tail as "this alternative succeeded" and never
/// try the next one, the identical boundary-closes-too-soon bug
/// `each_try_generic` closed for `try`/`catch`/`?`, just for `?//`'s own
/// fallthrough decision instead. [`check_lazy_item_for_try`] runs the same
/// check; a fault it finds is folded into this loop's own
/// `Flow::Escaped(Control::Error/Break/Halt)` handling below via the same
/// side-channel idiom, so it participates in the *same* `is_last`
/// fallthrough/decode-failure-exclusion rules as an ordinary error.
#[allow(clippy::too_many_arguments)] // STYLE-0004: mirrors `eval::each_pattern_alternatives`'s
                                     // own 7-argument shape plus this module's `cursor` --
                                     // every param is threaded straight through to the
                                     // recursive `eval_each_generic` call, a struct would just
                                     // rename the same fields.
fn each_pattern_alternatives_generic<S: EvalSemantics, V: DocumentValue>(
    patterns: &[Pattern],
    all_var_names: &[String],
    body: &Expr,
    bound_val: &OwnedValue,
    value: &V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let last_idx = patterns.len() - 1;
    // #1366: a genuine `?//`-chain (2+ patterns) inverts real jq's
    // duplicate-binding dedup rule relative to a bare pattern -- see
    // `extract_pattern_bindings`'s own doc comment.
    let invert_dedup = patterns.len() > 1;

    for (i, pattern) in patterns.iter().enumerate() {
        let is_last = i == last_idx;

        let bindings = match extract_pattern_bindings(pattern, bound_val, invert_dedup) {
            Ok(b) => b,
            Err(e) => {
                if is_last {
                    return Flow::Escaped(Control::Error(e));
                }
                continue;
            }
        };

        let null_value = OwnedValue::Null;
        let substituted_body = substitute_vars(
            body,
            as_var_refs(&bindings).chain(
                all_var_names
                    .iter()
                    .filter(|name| !bindings.iter().any(|(n, _)| n == *name))
                    .map(|name| (name.as_str(), &null_value)),
            ),
        );

        let mut lazy_fault: Option<Control> = None;
        let flow = eval_each_generic::<S, V>(
            &substituted_body,
            value.clone(),
            optional,
            cursor,
            &mut |item| match check_lazy_item_for_try(item) {
                Ok(item) => sink(item),
                Err(control) => {
                    lazy_fault = Some(control);
                    Demand::Stop
                }
            },
        );
        let flow = match lazy_fault {
            Some(control) => Flow::Escaped(control),
            None => flow,
        };

        match flow {
            Flow::Exhausted => return Flow::Exhausted,
            // #1519: a satisfied consumer is jq's escaping `break`, so it
            // retries the next alternative just like `Control::Break` below.
            // `pending` is dropped on the retry, matching
            // `eval::each_pattern_alternatives`'s own arm.
            Flow::Stopped { pending } => {
                if is_retryable_stop(is_last) {
                    continue;
                }
                return Flow::Stopped { pending };
            }
            // #1620/#1660: same decode-failure exclusion as
            // `eval::each_pattern_alternatives` -- always propagates,
            // `is_last` or not. Live and load-bearing, not merely
            // stale-twin parity: `first([.p,.q] as [$y] ?// [$z,$y] | ...)`
            // reaches this loop via `eval_each_generic`'s own native
            // `Expr::AsPattern` arm (`each_as_pattern_generic`), not
            // `eval_single`'s wildcard fallback -- confirmed by removing
            // this arm and observing the exact silently-wrong-value bug
            // #1660 fixes elsewhere reappear here too.
            Flow::Escaped(Control::Error(e)) if e.is_decode_failure() => {
                return Flow::Escaped(Control::Error(e));
            }
            // #1457: `Break` falls through like `Error`, not immediately
            // like `Halt` -- same live-verified correction
            // `eval::each_pattern_alternatives` itself documents.
            Flow::Escaped(Control::Error(e)) => {
                if is_last {
                    return Flow::Escaped(Control::Error(e));
                }
                continue;
            }
            Flow::Escaped(Control::Break(label)) => {
                if is_last {
                    return Flow::Escaped(Control::Break(label));
                }
                continue;
            }
            Flow::Escaped(Control::Halt(code)) => return Flow::Escaped(Control::Halt(code)),
        }
    }

    unreachable!(
        "patterns is always non-empty by construction (the parser never \
         builds an empty AsPattern), and every loop iteration above \
         returns on its `is_last` pass"
    )
}

/// Demand-forwarding twin of [`eval_limit_generic`] (#1596, mirroring
/// `eval::each_limit`, #1462): forwards every output of `expr` straight to
/// `sink`, stopping the generator as soon as *either* `n` outputs have been
/// forwarded or `sink` itself says to stop -- whichever comes first.
/// [`eval_limit_generic`]'s own batch-collect (#1607) still answers a bare
/// `limit(n; expr)` correctly, but a wrapping consumer satisfied sooner
/// (`first(limit(2; (1,("B"|stderr))))`) had no way to say so until this arm
/// existed, because `eval_limit_generic` always collects up to `n` items as
/// one batch before `eval_each_generic`'s `_` fallback ever got a chance to
/// forward a smaller demand.
///
/// A generator `n` (anything beyond the common single-value shapes) bridges
/// to the full evaluator and drains its answer -- the same "give up on the
/// fast path, hand the whole node to `eval.rs`" policy as
/// `eval_limit_generic`'s own `None` return, except this function has no
/// caller to hand a `None` back to, so it performs the bridge/drain itself.
///
/// Guarded exactly like [`eval_limit_generic`] (see that function's own doc
/// comment): [`limit_or_nth_uses_live_input_queue`] defers to the bridge
/// *before* touching a live `input`/`inputs` queue, so the bridge's own
/// single `eval_on_owned` call is the only evaluation of `n_expr`.
///
/// That guard used to have a sibling -- a static "is `n_expr` a bare
/// top-level `Comma`?" check, deferring without evaluating so the bridge's
/// evaluation would not be stacked on top of a probe this function had
/// already made (`first(limit((1,("N"|debug)); 42))` wrote `"N"` twice
/// before it existed, the class of leak #1596 closed). #1687 removed the
/// need for it: [`fanout_arg_each_generic`] drives `n_expr` exactly once for
/// every shape, so there is no probe to double up on and nothing to detect
/// statically.
///
/// **A generator `n_expr` used to be a documented residual here; #1687 closed
/// it.** The bridge this arm used to take for that shape answers with a fully
/// materialized `QueryResult`, collecting every output across every `n_expr`
/// binding before `drain_result_generic` could apply a wrapping `first`/`nth`'s
/// smaller demand -- so `first(limit((1,2); (1, ("B"|stderr))))` wrote `B` to
/// stderr where jq never explores the `$n=2` binding at all. Driving `n_expr`
/// through [`fanout_arg_each_generic`] instead keeps the whole nest
/// demand-driven, and both tools now write nothing there, while the
/// genuinely-every-output shape (`[limit((1,2); (1, ("B"|stderr)))]`) still
/// correctly writes `B` in both.
fn each_limit_generic<S: EvalSemantics, V: DocumentValue>(
    n_expr: &Expr,
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    // Rebuilt once, up front: all three deferral points below hand the same
    // node to the same bridge (#1687 item 4).
    let bridged = Expr::Limit {
        n: Box::new(n_expr.clone()),
        expr: Box::new(expr.clone()),
    };

    if limit_or_nth_uses_live_input_queue(n_expr, expr) {
        return bridge_to_full_evaluator_flow::<S, V>(&bridged, value, cursor, optional, sink);
    }

    fanout_arg_each_generic::<S, V, _>(n_expr, value.clone(), optional, cursor, |n_value| {
        each_limit_with_n_generic::<S, V>(n_value, expr, value.clone(), optional, cursor, sink)
    })
}

/// `limit(n; expr)`'s sink-driven work for one already-resolved `n` --
/// [`each_limit_generic`]'s per-`n` body, split out for the fan-out exactly
/// as [`limit_with_n_generic`] was split out of [`eval_limit_generic`].
fn each_limit_with_n_generic<S: EvalSemantics, V: DocumentValue>(
    n_value: OwnedValue,
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let n = match classify_limit_n(n_value) {
        Ok(LimitN::Unlimited) => {
            return eval_each_generic::<S, V>(expr, value, optional, cursor, sink)
        }
        Ok(LimitN::Take(n)) => n,
        Err(e) => return Flow::Escaped(Control::Error(e)),
    };

    if n == 0 {
        return Flow::Exhausted;
    }

    let mut count = 0usize;
    let mut outer_stopped = false;
    let flow = eval_each_generic::<S, V>(expr, value, optional, cursor, &mut |item| {
        count += 1;
        if sink(item) == Demand::Stop {
            outer_stopped = true;
            Demand::Stop
        } else if count >= n {
            Demand::Stop
        } else {
            Demand::Continue
        }
    });

    // The wrapping consumer, not our own `n` cap, is why the generator
    // stopped -- propagate its verdict (and whatever `pending` came with it)
    // verbatim, exactly as every other lazy arm does.
    if outer_stopped {
        return flow;
    }
    match flow {
        Flow::Stopped { .. } | Flow::Exhausted => Flow::Exhausted,
        Flow::Escaped(control) => Flow::Escaped(control),
    }
}

/// Generic-evaluator twin of `eval::eval_each_pipe` (#1461): the "stop
/// pulling from stage 1 once the rest of the pipe has enough" mechanism.
///
/// `needs_path_context` bridges the whole remaining pipe eagerly, exactly
/// matching `eval_single`'s own `Expr::Pipe` arm's guard (#554) -- reused
/// directly rather than re-derived, so `path`/`parent`/`key` never silently
/// see a stubbed-zero default. Otherwise: evaluate stage 1 lazily via
/// [`eval_each_generic`], and for every item it produces, recursively drive
/// the *whole remaining pipe* (not just one more stage) through a driver
/// closure that forwards items to the outer `sink` (via
/// [`continue_pipe_element_generic`]) and translates whatever `Flow` that
/// recursive call terminates in back into a `Demand` for stage 1's own
/// generator. Recursing on the full remaining slice, rather than one stage at
/// a time, is what lets cursor threading (`.[] | select(...) | line`) survive
/// an arbitrary-length remaining pipe -- the exact property a narrower,
/// 2-stage-only attempt lost (#1503 review).
///
/// A `LazyKeys`/`LazyIndexRange`/`LazySeq` item folds through the remaining
/// stages via [`fold_pipe_stages_sink`] (#1565) -- demand-aware from the
/// stage it was produced at, so `first(keys | .[] | stderr)` stops after one
/// key instead of visiting every one, unlike the eager `fold_pipe_stages`
/// this replaced for this call site. `fold_pipe_stages_sink` itself reuses
/// `fold_pipe_stages`'s own per-variant composability fast paths (#724/#725)
/// rather than duplicating them -- see its own doc comment.
fn eval_each_pipe_generic<S: EvalSemantics, V: DocumentValue>(
    exprs: &[Expr],
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    if exprs.iter().any(needs_path_context) {
        return drain_result_generic(
            eval_single::<S, _>(&Expr::Pipe(exprs.to_vec()), value, optional, cursor),
            sink,
        );
    }

    let Some((first, rest)) = exprs.split_first() else {
        // Same behaviour as `eval_single`'s own empty-pipe short-circuit:
        // `value`'s cursor, if any, is not preserved -- an empty `Expr::Pipe`
        // is not a shape real syntax produces, only a synthesized "rest"
        // slice that never actually reaches zero length in practice.
        return push_one_generic(GenericItem::One(value), sink);
    };
    if rest.is_empty() {
        return eval_each_generic::<S, V>(first, value, optional, cursor, sink);
    }

    let mut downstream: Option<Flow> = None;
    // Same once-per-driver cache as `drive_pipe_elements_generic` (#1598):
    // this closure also runs once per item stage 1 produces.
    let mut rest = RestPipe::new(rest);
    let upstream = {
        let mut driver = |item: GenericItem<V>| -> Demand {
            let flow = continue_pipe_element_generic::<S, V>(item, &mut rest, optional, &mut *sink);
            match flow {
                Flow::Exhausted => Demand::Continue,
                other => {
                    downstream = Some(other);
                    Demand::Stop
                }
            }
        };
        eval_each_generic::<S, V>(first, value, optional, cursor, &mut driver)
    };

    match downstream {
        Some(flow) => flow,
        None => upstream,
    }
}

/// Generic-evaluator twin of `eval::each_take_first` (#1461): pull at most
/// one output of `inner` via [`eval_each_generic`], then stop the generator
/// -- jq's `def first(f): label $out | (f, break $out);`.
fn each_take_first_generic<S: EvalSemantics, V: DocumentValue>(
    inner: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> (Vec<GenericItem<V>>, Flow) {
    // #1519: a `Vec`, not an `Option`, for the same reason `eval`'s
    // `each_take_first` takes one -- jq's `def first(f): label $out | (f,
    // break $out);` emits *before* it breaks, and a `?//` chain catches that
    // break and re-runs the generator, so `first` legitimately emits once per
    // alternative. Under a single alternative this is still entered exactly
    // once and returns exactly one item.
    let mut taken: Vec<GenericItem<V>> = Vec::new();
    let flow = eval_each_generic::<S, V>(inner, value, optional, cursor, &mut |item| {
        taken.push(item);
        Demand::Stop
    });
    (taken, flow)
}

/// Convert a captured [`GenericItem`] back to the [`GenericResult`] shape it
/// came from -- the inverse of [`drain_result_generic`]'s single-item arms.
///
/// `OneCursorValue` has no `GenericResult` counterpart to convert to, so it
/// collapses to plain `OneCursor` here, dropping the pre-decoded value
/// (#1609). That's fine: this function's only caller for a single captured
/// item is `each_take_first_generic`'s `first(...)`/`last(...)` path, once
/// per builtin call rather than once per key, so re-deriving the value via
/// `GenericResult::OneCursor`'s later `.value()` costs nothing that matters
/// -- adding a matching `GenericResult` variant just to avoid that one cold
/// re-decode isn't worth `GenericResult`'s much larger consumer set.
fn generic_item_to_result<V: DocumentValue>(item: GenericItem<V>) -> GenericResult<V> {
    match item {
        GenericItem::One(v) => GenericResult::One(v),
        GenericItem::OneCursor(c) => GenericResult::OneCursor(c),
        GenericItem::OneCursorValue(c, _v) => GenericResult::OneCursor(c),
        GenericItem::Owned(o) => GenericResult::Owned(o),
        GenericItem::LazyKeys {
            fields,
            sorted,
            collapse,
        } => GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        },
        GenericItem::LazyIndexRange(len) => GenericResult::LazyIndexRange(len),
        GenericItem::LazySeq(seq) => GenericResult::LazySeq(seq),
    }
}

/// Convert a [`GenericItem`] pulled from a fanout operand into the
/// `OwnedValue` a `combine` step needs, forcing whatever materialization a
/// lazy variant still owes via [`generic_item_to_result`] +
/// `GenericResult::materialize_lazy` -- the same conversion
/// [`push_generic_owned_values`] already applies, reused here rather than
/// re-derived.
///
/// Fallible, unlike `eval::Item::into_owned_lossy`: a `LazySeq` item -- a buffered
/// `map`/`select` chain, #724/#725 -- can itself error/break/halt on the
/// materialization this forces (e.g. `map(1/0) == 1`), where `eval.rs`'s
/// `QueryResult` has no lazy variant to force in the first place.
/// [`binary_fanout_each_generic`] routes that failure through the same
/// "abort the whole fanout" `Flow::Escaped` path a `combine` error already
/// uses, ungated by `optional` -- matching `eval.rs`'s convention that an
/// operand-evaluation error is caught one level up, by `Expr::Optional`/
/// `try`, not here.
fn generic_item_into_owned<V: DocumentValue>(item: GenericItem<V>) -> Result<OwnedValue, Control> {
    match generic_item_to_result(item).materialize_lazy() {
        GenericResult::One(v) => to_owned(&v).map_err(Control::Error),
        GenericResult::OneCursor(c) => to_owned_cursor(&c).map_err(Control::Error),
        GenericResult::Owned(v) => Ok(v),
        GenericResult::Error(e) => Err(Control::Error(e)),
        GenericResult::Break(label) => Err(Control::Break(label)),
        GenericResult::Halt(code) => Err(Control::Halt(code)),
        // Provably dead, not just unlikely: `generic_item_to_result` can only
        // ever produce `One`/`OneCursor`/`Owned`/`LazyKeys`/`LazyIndexRange`/
        // `LazySeq` -- `GenericItem`'s own variant set -- and
        // `materialize_lazy` folds the three lazy ones into
        // `Owned`/`Error`/`Break`/`Halt`. That leaves exactly the six arms
        // above; `GenericResult`'s other variants (`Many`/`ManyCursor`/
        // `ManyOwned`/`None`/`Partial`) have no `GenericItem` counterpart and
        // can never reach this match. No coverage-diff test can close this
        // arm without adding a new `GenericItem` variant to alias into one of
        // them -- see #1064 for the established precedent of documenting
        // rather than forcing coverage of a structurally unreachable arm.
        _ => {
            unreachable!("a single GenericItem never materializes to a multi-output or lazy shape")
        }
    }
}

/// Generic-evaluator twin of `eval::binary_fanout_each` (#1481): the one
/// right-outer/left-inner fanout loop for the `V: DocumentValue`-generic
/// evaluator, parameterized over *how* an operand is enumerated -- reusing
/// `eval.rs`'s `pub(crate)` `Demand`/`Flow` directly (they carry no generic
/// parameters), the same way this module's whole lazy-sink family already
/// does.
///
/// No `'a`/`W` parameter, unlike `eval.rs`'s version: `GenericItem<V>` owns
/// its cursor (`V::Cursor: Copy`), so nothing here borrows from an arena.
///
/// `each_operand` is `Fn`, not `FnMut`, for the same re-entrancy reason
/// `eval.rs`'s `binary_fanout_each` gives: the per-right-value call on `left`
/// happens while the call on `right` is still on the stack.
fn binary_fanout_each_generic<V: DocumentValue>(
    each_operand: impl Fn(&Expr, &mut dyn FnMut(GenericItem<V>) -> Demand) -> Flow,
    left: &Expr,
    right: &Expr,
    optional: bool,
    mut combine: impl FnMut(OwnedValue, OwnedValue) -> Result<OwnedValue, EvalError>,
    sink: &mut dyn FnMut(GenericItem<V>) -> Demand,
) -> Flow {
    let mut abort: Option<Flow> = None;

    let outer = each_operand(right, &mut |right_item: GenericItem<V>| {
        let right_val = match generic_item_into_owned(right_item) {
            Ok(v) => v,
            Err(control) => {
                abort = Some(Flow::Escaped(control));
                return Demand::Stop;
            }
        };

        let inner = each_operand(left, &mut |left_item: GenericItem<V>| {
            let left_val = match generic_item_into_owned(left_item) {
                Ok(v) => v,
                Err(control) => {
                    abort = Some(Flow::Escaped(control));
                    return Demand::Stop;
                }
            };
            match combine(left_val, right_val.clone()) {
                Ok(v) => sink(GenericItem::Owned(v)),
                // Reached by the `Expr::Arithmetic` caller only: the two
                // `Expr::Compare` call sites wrap the infallible
                // `apply_compare_op` in `Ok(...)` (`CompareOp` has no failure
                // mode), while `arith_combine` fails on `"a" + 1`, `1 / 0`,
                // and friends. Same policy as `eval.rs`'s
                // `binary_fanout_each`: a `combine` error aborts the *whole*
                // fanout rather than skipping just that pairing, whatever was
                // already pushed stands, and `optional` decides whether the
                // failure itself survives.
                Err(e) => {
                    abort = Some(if optional {
                        Flow::Exhausted
                    } else {
                        Flow::Escaped(Control::Error(e))
                    });
                    Demand::Stop
                }
            }
        });

        if abort.is_some() {
            return Demand::Stop;
        }

        match inner {
            Flow::Exhausted => Demand::Continue,
            other => {
                abort = Some(other);
                Demand::Stop
            }
        }
    });

    abort.unwrap_or(outer)
}

/// Collecting wrapper over [`binary_fanout_each_generic`] for `eval_single`'s
/// `Expr::Compare` arm (#1481) -- mirrors `eval.rs`'s
/// `eval_binary_fanout`/`binary_fanout_core` pairing, wired directly to the
/// lazy `eval_each_generic` operand strategy since this file never had an
/// eager sibling that still needs one kept around.
///
/// `Expr::Arithmetic` needs no counterpart here: it has no native
/// `eval_single` arm at all, so a bare top-level `a + b` reaches `eval.rs`'s
/// own (already interleaved) `eval_binary_fanout` through the `eval_on_owned`
/// bridge. Only the *lazy* side needed an arm, and that one lives in
/// [`eval_each_generic`].
fn eval_compare_generic<S: EvalSemantics, V: DocumentValue>(
    op: CompareOp,
    left: &Expr,
    right: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    let mut out: Vec<OwnedValue> = Vec::new();
    // A control raised while converting an item the sink was handed, kept
    // out-of-band because the sink can only answer `Demand` -- same shape as
    // `binary_fanout_each_generic`'s own `abort`.
    let mut stray: Option<Control> = None;
    let flow = binary_fanout_each_generic::<V>(
        |operand, operand_sink| {
            eval_each_generic::<S, V>(operand, value.clone(), false, cursor, operand_sink)
        },
        left,
        right,
        optional,
        |left_val, right_val| {
            Ok(OwnedValue::Bool(apply_compare_op::<S>(
                op, &left_val, &right_val,
            )))
        },
        &mut |item: GenericItem<V>| {
            // Total, rather than a `GenericItem::Owned` match with an
            // `unreachable!()` fallback: `binary_fanout_each_generic` only
            // ever calls `sink` with an `Owned` today, and for that variant
            // `generic_item_into_owned` is an identity passthrough -- so
            // this costs nothing on the only path that runs, and leaves no
            // arm that could take the process down if a second `sink` call
            // site is ever added. Same choice `eval.rs`'s
            // `binary_fanout_core` makes for its own impossible
            // `Flow::Stopped` ("an answer built from the outputs already
            // produced is a better failure mode than a panic"), and
            // deliberately *not* the choice the two `unreachable!()`s nearby
            // make -- those enumerate a closed variant set, where this would
            // be asserting an invariant about callers. The `Err` arm has no
            // caller that can reach it today and so reads as uncovered;
            // that is the accepted cost of not panicking here.
            match generic_item_into_owned(item) {
                Ok(v) => {
                    out.push(v);
                    Demand::Continue
                }
                Err(control) => {
                    stray = Some(control);
                    Demand::Stop
                }
            }
        },
    );

    let control = match flow {
        Flow::Exhausted | Flow::Stopped { .. } => None,
        Flow::Escaped(control) => Some(control),
    };
    finish_fork_generic(out, control.or(stray), optional)
}

/// Shared resolution of [`each_take_first_generic`]'s and
/// [`nth_with_n_generic`]'s two outputs into a [`GenericResult`], mirroring
/// [`super::eval::take_stopping_items_to_result`] -- so their call sites
/// cannot drift on the dropped-versus-raised trailing-control rule (#1519).
/// Both sinks stop on every item they keep, so both have the identical rule.
fn take_stopping_items_to_generic_result<V: DocumentValue>(
    mut items: Vec<GenericItem<V>>,
    flow: Flow,
) -> GenericResult<V> {
    // #1519: items *and* an escape means a later `?//` alternative failed
    // after an earlier one had already answered. jq reaches that failure, so
    // the prefix is kept and the control still raises.
    if let Flow::Escaped(control) = flow {
        if items.is_empty() {
            return partial_generic(Vec::new(), control);
        }
        let mut owned = vec_with_capacity(items.len());
        for item in items {
            match generic_item_into_owned(item) {
                Ok(v) => owned.push(v),
                // Not reachable through a `?//` retry today: a cursor-backed
                // batch defers its decode (see
                // `test_generic_first_nth_retry_batch_still_raises_decode_failure_1519`),
                // so the failure surfaces at materialization instead. Handled
                // because the conversion's signature requires it.
                Err(c) => return partial_generic(owned, c),
            }
        }
        return partial_generic(owned, control);
    }
    // `Flow::Stopped`/`Flow::Exhausted`: an undecodable kept item must raise
    // rather than silently being dropped, and a `?//` retry must not be able
    // to launder that away -- the same rule `eval::items_to_result_checked`
    // enforces for the borrowed evaluator's twin.
    match items.len() {
        0 => GenericResult::None,
        // The lone-item case keeps `generic_item_to_result`'s cursor-backed
        // conversion so a duplicate key inside it survives (#607).
        1 => generic_item_to_result(items.remove(0)),
        _ => match items_to_generic_result(items) {
            Ok(result) => result,
            Err((prefix, control)) => partial_generic(prefix, control),
        },
    }
}

/// Evaluate `first(inner)`/`last(inner)`. `Expr::FirstExpr`/`Expr::LastExpr`
/// are the only spellings any parseable query produces -- `Builtin::
/// FirstStream`/`LastStream` are never constructed by the parser (#1986;
/// see `builtin_first_stream_propagates_bare_halt`), so the call sites in
/// `eval_single`/`eval_builtin` that also route here for those two variants
/// are defensive symmetry, not a second live production. Preserves a cursor
/// through the extraction when `inner`'s stream carries one.
///
/// Mirrors `eval::eval_first_expr`/`eval::eval_last_expr`'s control-flow
/// exactly (short-circuit-on-first-output vs must-exhaust-the-stream for
/// `Partial`/`Error`/`Break`), just adding `OneCursor`/`ManyCursor` arms so a
/// selected output that is itself a cursor-backed document node keeps its
/// cursor -- and with it, any duplicate keys inside it -- instead of being
/// forced through this module's `to_owned()` bridge (#607).
fn eval_first_or_last_generic<S: EvalSemantics, V: DocumentValue>(
    inner: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    want_last: bool,
) -> GenericResult<V> {
    // `first(f)` where `f` can consume input documents must not run `f` to
    // completion. [`eval_each_generic`] (#1461) is only a native lazy arm for
    // `Comma`/`Pipe`/`Paren` -- it has no `Builtin::Inputs`-aware arm of its
    // own, so a bare `first(inputs)`/`first(inputs, 1)`/`first(inputs | f)`
    // would still fall to its eager `_` fallback and drain the shared queue.
    // `eval::eval_first_expr` has been wired to `eval.rs`'s own sink since
    // #820 Stage 2, so hand the whole `first(...)` over rather than
    // evaluating `inner` here (#1309) -- one guard covering every shape
    // `inputs` can appear in, rather than one `eval_each_generic` arm per
    // shape.
    //
    // **Why this is not dead code under #1504's top-level bridge.** That
    // bridge does subsume this guard for most programs -- `inner` is always a
    // subterm of the tree `takes_input_queue_bridge` already walked -- but
    // only for the programs it actually takes. Its cursor-metadata carve-out
    // deliberately declines it for a program that mixes an input builtin with
    // `line`/`at_offset`/..., and such a program reaches `eval_single` and
    // then this function with the shared queue still live:
    // `first(inputs), line` is the shape. Without this guard that
    // `first(inputs)` falls to the eager `_` fallback described above and
    // drains the queue -- exactly #1309's silent data loss, reintroduced
    // through the carve-out. Pinned by
    // `test_jq_cursor_metadata_carve_out_keeps_first_inputs_lazy_1504`.
    //
    // Gated rather than unconditional because the bridge costs this subtree
    // its cursor, and with it #607's duplicate-key fidelity. The one-load
    // `input_queue_is_active` check comes first specifically so yq mode,
    // library embedders and the common `first(.[])` never pay for the AST
    // walk.
    if !want_last
        && crate::jq::input_queue_is_active()
        && crate::jq::walk::uses_input_builtins(inner)
    {
        return bridge_to_full_evaluator::<S, _>(
            &Expr::FirstExpr(Box::new(inner.clone())),
            value,
            cursor,
            optional,
        );
    }

    // `first` stops pulling from `inner` as soon as it has one output, so
    // anything past that point -- a later comma sibling, a later pipe stage's
    // continuation, an unpulled element of a `.[]` -- is never evaluated
    // (#820, #1461). `last` cannot short-circuit -- it does not know a value
    // is the last until the stream is exhausted -- so it keeps the eager
    // path below.
    if !want_last {
        // #1519: normally one item; a `?//` chain under this consumer
        // legitimately yields one per alternative. See
        // `take_stopping_items_to_generic_result`, whose rule this uses
        // directly -- `each_take_first_generic` stops on every item it
        // keeps, exactly like `nth_with_n_generic`'s own sink.
        let (items, flow) = each_take_first_generic::<S, V>(inner, value, optional, cursor);
        return take_stopping_items_to_generic_result(items, flow);
    }

    // No local `optional` handling is needed for `last(f)?` here: post-#693,
    // `Expr::Optional(inner)`'s own dispatch evaluates `inner` with the
    // *ambient* `optional` (never forced `true`) and catches the resulting
    // `Error`/`Partial(_, Error)` itself, one layer out. So `optional` is
    // always `false` on every dispatch path that reaches this function
    // today, and `last(empty)?` (`null`) vs. `last(error("x"))?` (empty)
    // stays correct entirely via that outer boundary, not anything here.
    match eval_single::<S, V>(inner, value, optional, cursor) {
        GenericResult::One(v) => GenericResult::One(v),
        GenericResult::OneCursor(c) => GenericResult::OneCursor(c),
        GenericResult::Many(vs) => match vs.into_iter().next_back() {
            Some(v) => GenericResult::One(v),
            None => GenericResult::Owned(OwnedValue::Null),
        },
        GenericResult::ManyCursor(cs) => match cs.into_iter().next_back() {
            Some(c) => GenericResult::OneCursor(c),
            None => GenericResult::Owned(OwnedValue::Null),
        },
        GenericResult::Owned(v) => GenericResult::Owned(v),
        GenericResult::ManyOwned(vs) => match vs.into_iter().next_back() {
            Some(v) => GenericResult::Owned(v),
            None => GenericResult::Owned(OwnedValue::Null),
        },
        // `inner`'s stream has exactly one output (the whole `keys`/
        // `keys_unsorted` result) — forward it unchanged, same as
        // `Owned`/`OneCursor` above, so laziness survives `last(...)`.
        GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        } => GenericResult::LazyKeys {
            fields,
            sorted,
            collapse,
        },
        GenericResult::LazyIndexRange(len) => GenericResult::LazyIndexRange(len),
        // Same forwarding, same reasoning: `first(inner)`/`last(inner)`
        // only need to know *which* of `inner`'s outputs this is, never
        // inspect the value itself, so forwarding doesn't swallow
        // anything -- whoever consumes the returned `LazySeq` next
        // materializes it, and any error surfaces there.
        GenericResult::LazySeq(seq) => GenericResult::LazySeq(seq),
        // jq's `last(f)` is `reduce f as $x (null; $x)`, which always
        // produces exactly one output -- an empty operand answers the
        // seed. `first` is deliberately not symmetric (#1521).
        GenericResult::None => GenericResult::Owned(OwnedValue::Null),
        GenericResult::Error(e) => GenericResult::Error(e),
        GenericResult::Break(label) => GenericResult::Break(label),
        GenericResult::Halt(code) => GenericResult::Halt(code),
        // `last` cannot short-circuit -- it doesn't know a value is the
        // last one until the stream is exhausted -- so a `Partial` just
        // surfaces its trailing control, dropping the prefix (matches
        // `eval::eval_last_expr`).
        GenericResult::Partial(_, Control::Error(e)) => GenericResult::Error(e),
        GenericResult::Partial(_, Control::Break(label)) => GenericResult::Break(label),
        GenericResult::Partial(_, Control::Halt(code)) => GenericResult::Halt(code),
    }
}

/// Whether `expr` reaches [`crate::jq::walk::uses_input_builtins`] while the
/// shared `input`/`inputs` queue is live -- shared by [`eval_limit_generic`]/
/// [`eval_nth_generic`] for the same reason [`eval_first_or_last_generic`]
/// checks it locally (#1309): neither `limit`/`nth` run their body to
/// completion either, so an `inputs`-consuming `n_expr` or `expr` reaching
/// this function's own-pull path (below) would drain the queue eagerly via
/// `eval.rs`'s `builtin_inputs` instead of stopping at `n`/index `n`, same
/// failure class as the un-guarded `first` once had. The one-load
/// `input_queue_is_active` check comes first so yq mode, library embedders
/// and the common `limit(3; .[])` never pay for the AST walk.
fn limit_or_nth_uses_live_input_queue(n_expr: &Expr, expr: &Expr) -> bool {
    crate::jq::input_queue_is_active()
        && (crate::jq::walk::uses_input_builtins(n_expr)
            || crate::jq::walk::uses_input_builtins(expr))
}

/// Native `Expr::Limit` arm for the generic evaluator (#1607) — the same
/// fix `eval_first_or_last_generic` already applies to `first`/`last`
/// (#607), a second instance of the same root cause: `eval_single`'s `_`
/// fallback materializes the whole input into an `OwnedValue` before `expr`
/// ever runs, and `OwnedValue::Object` is `IndexMap`-backed and cannot
/// represent a duplicate mapping key. `limit(n; keys|.[])` on a document
/// with one silently dropped every duplicate that `keys|.[]` alone (via
/// `GenericResult::LazyKeys`/`DistinctKeyCursors`) already preserves
/// correctly, regardless of `S::COLLAPSE_DUPLICATE_KEYS`.
///
/// Also guards `n_expr`/`expr` against a live `input`/`inputs` queue
/// (`limit_or_nth_uses_live_input_queue`, mirroring
/// `eval_first_or_last_generic`'s own #1309 guard): `limit`/`nth` stop
/// early just like `first` does, so an `inputs`-consuming operand reaching
/// the eager `_` fallback below would drain the whole remaining queue
/// instead of stopping at `n`/index `n`.
///
/// Returns `None` only for the live `input`/`inputs` queue, which must reach
/// `eval.rs` untouched. Every other shape is handled natively, via
/// [`eval_each_generic`]'s own demand-driven `Pipe`/`Iterate`/`LazyKeys`
/// machinery -- what keeps `expr`'s duplicate keys alive.
///
/// **A generator `n_expr` is one of those shapes now (#1687 items 2 and 3).**
/// `limit((1,2); f)` re-runs `expr` once per `n` value (jq's own rarer
/// laziness contract, #1279); this used to probe `n_expr` with `eval_single`,
/// discover it was not a single value, and defer -- which cost both the
/// duplicate keys the native path existed to preserve *and* a second
/// evaluation of `n_expr`, so a `debug` inside it fired twice where jq fires
/// it once. [`fanout_arg_generic`] drives `n_expr` once and runs
/// [`limit_with_n_generic`] per value instead, so neither cost remains, and
/// the shallow "is it a bare top-level `Comma`?" check that partially
/// mitigated the second one is gone with them.
/// Run `body` once per output of `arg_expr`, concatenating the results --
/// the generic-evaluator twin of `eval::fanout_arg`'s `ArgFanout::All` arm
/// (#1279), which is the only fan-out mode this file's callers need.
///
/// Before this existed, `eval_limit_generic`/`eval_nth_generic`/
/// `eval_has_generic` each *probed* their argument with `eval_single`, and
/// on discovering more than one output gave up and re-evaluated the entire
/// expression through the lossy `OwnedValue` bridge. That cost two things
/// #1687 names as separate defects: the argument was evaluated twice, so a
/// `debug`/`stderr` inside it fired twice where jq fires once (item 3); and
/// the bridge collapsed every duplicate mapping key the native path existed
/// to preserve (item 2). Driving the argument through [`eval_each_generic`]
/// once and calling `body` per value fixes both at once, and lets all three
/// callers drop their private, shallow "is the argument a bare top-level
/// `Comma`?" guard -- the third copy of which `eval_has_generic`'s own doc
/// comment already flagged as the wrong pattern.
///
/// `pending_first` is load-bearing, not an optimization: it holds `body`'s
/// result for a first argument value whose successor has not arrived yet, so
/// the overwhelmingly common single-output case returns that result
/// *unflattened*. Without it every `limit(3; .[])` would be forced through
/// `Vec<OwnedValue>` and would lose exactly the `ManyCursor`/`LazySeq` shape
/// #1607 added it to keep. A second value flushes it into `out` before that
/// value's own result is appended, so ordering is unaffected.
/// [`fanout_arg_generic`] for a sink-driven caller: run `body` once per
/// output of `arg_expr`, and stop pulling further argument values the moment
/// `body` reports the downstream consumer has had enough.
///
/// That last property is the whole reason this exists separately rather than
/// the eager version being reused. `first(limit((1,2); (1, ("B"|stderr))))`
/// must never explore the `$n=2` binding at all -- jq does not, because
/// `first` satisfies itself from `$n=1`'s own single output and the whole
/// nest is demand-driven. Routing that shape through the eager
/// `OwnedValue` bridge, as `each_limit_generic` did before #1687, ran every
/// binding to completion before any demand could apply, writing `B` to
/// stderr where jq writes nothing -- a divergence `each_limit_generic`'s own
/// doc comment recorded as a residual and this closes.
/// Materialize every output of `expr` through the *generic* evaluator,
/// alongside whatever control terminated the stream.
///
/// The generic twin of `eval::stream_outputs_checked`, and the whole of
/// #1687's `reduce`/`foreach` fix: those two constructs had no arm in this
/// file at all, so every `reduce`/`foreach` query bridged the entire document
/// into an `OwnedValue` before `input` was so much as looked at -- and
/// `reduce (keys|.[]) as $k (0; .+1)` over `b: 1\na: 2\nb: 3\n` therefore
/// counted 2 keys where `[keys|.[]]` on the same document correctly counts 3.
/// Evaluating `input`/`INIT` here instead keeps the *stream* faithful.
///
/// **The elements are still owned, and that is a real limit, not an
/// oversight.** `reduce`'s accumulator, and every `$x` a pattern binds, are
/// `OwnedValue` throughout both evaluators -- `substitute_bound_var` takes
/// `&OwnedValue`, and no duplicate-key-capable owned representation exists in
/// this crate. So a duplicate mapping key inside an element that gets *bound*
/// is still collapsed at the bind, exactly as it is in `eval.rs`. Only the
/// number and order of the elements is recovered here. Recorded in
/// `docs/compliance/yq/limitations.md`.
/// Whether `expr` can produce an unbounded stream when pulled to exhaustion,
/// and so must not be driven through [`stream_owned_outputs_generic`].
///
/// Only `repeat` qualifies today, and the reason is asymmetric on purpose.
/// `Expr::Repeat`'s *eager* evaluation (`eval::eval_repeat`) stops after
/// `MAX_ITERATIONS` rounds and raises `repeat: maximum iterations exceeded`;
/// its *demand-driven* sink arm (`each_repeat_generic`, #2014) deliberately
/// does not, because its whole purpose is to let a wrapping `limit`/`first`
/// stop it at the source. Every other consumer of that sink stops; an eager
/// one does not, so `reduce repeat(1) as $x (0; .+1)` would spin forever
/// rather than raising the way `eval.rs`'s own `reduce` does.
///
/// `range(infinite)` needs no entry here -- it self-caps -- and `while`/
/// `until` have no sink arm at all, so both reach `eval.rs`'s bounded
/// evaluation regardless.
///
/// Consulted for *both* operands this file drives eagerly -- `input` and
/// `INIT`. Guarding only `input` still hung on
/// `reduce .[] as $x (repeat(1); .+$x)`: the guard's scope has to match every
/// call site it protects, not just the one in the repro. UPDATE needs no
/// guard, since `eval.rs`'s fold evaluates it.
///
/// Conservative in the safe direction: a false positive costs only the
/// duplicate-key fidelity this arm adds, falling back to exactly the
/// behaviour `reduce`/`foreach` had before #1687.
fn streams_unbounded(expr: &Expr) -> bool {
    crate::jq::walk::any_subexpr(expr, &mut |e| matches!(e, Expr::Repeat(_)))
}

fn stream_owned_outputs_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> (Vec<OwnedValue>, Option<Control>) {
    let mut out: Vec<OwnedValue> = Vec::new();
    let mut decode_err: Option<Control> = None;
    let flow = eval_each_generic::<S, V>(expr, value, optional, cursor, &mut |item| {
        match generic_item_into_owned(item) {
            Ok(owned) => {
                out.push(owned);
                Demand::Continue
            }
            // The prefix already converted is kept, matching
            // `stream_outputs_checked`'s `promote_borrowed_checked` arm.
            Err(control) => {
                decode_err = Some(control);
                Demand::Stop
            }
        }
    });
    let control = decode_err.or(match flow {
        Flow::Exhausted | Flow::Stopped { .. } => None,
        Flow::Escaped(control) => Some(control),
    });
    (out, control)
}

fn fanout_arg_each_generic<S: EvalSemantics, V: DocumentValue, B>(
    arg_expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    mut body: B,
) -> Flow
where
    B: FnMut(OwnedValue) -> Flow,
{
    // Tracked out-of-band for the usual reason: the sink can only answer
    // `Demand`, so "why did the pull stop" has to be recorded beside it.
    let mut escape: Option<Control> = None;
    let mut consumer_stopped = false;

    let flow = eval_each_generic::<S, V>(arg_expr, value, optional, cursor, &mut |item| {
        let owned = match generic_item_into_owned(item) {
            Ok(owned) => owned,
            Err(control) => {
                escape = Some(control);
                return Demand::Stop;
            }
        };
        match body(owned) {
            // This `n`'s own walk finished; go on to the next one.
            Flow::Exhausted => Demand::Continue,
            // The downstream consumer said stop. Its verdict outranks the
            // argument generator's, exactly as it does inside
            // `each_limit_generic`'s own inner sink.
            Flow::Stopped { .. } => {
                consumer_stopped = true;
                Demand::Stop
            }
            Flow::Escaped(control) => {
                escape = Some(control);
                Demand::Stop
            }
        }
    });

    match escape {
        Some(control) => Flow::Escaped(control),
        // `pending` is dropped for the reason every other lazy consumer
        // drops it: it belongs to an eager fallback jq would never reach.
        None if consumer_stopped => Flow::Stopped { pending: None },
        None => flow,
    }
}

fn fanout_arg_generic<S: EvalSemantics, V: DocumentValue, B>(
    arg_expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
    mut body: B,
) -> GenericResult<V>
where
    B: FnMut(OwnedValue) -> GenericResult<V>,
{
    let mut out: Vec<OwnedValue> = Vec::new();
    let mut body_control: Option<Control> = None;
    let mut pending_first: Option<GenericResult<V>> = None;
    // An argument value that cannot be decoded at all (#1247). Tracked
    // out-of-band because the sink can only answer `Demand`, and reported
    // ahead of `flow` below since it is the reason the pull stopped.
    let mut decode_err: Option<Control> = None;

    let flow = eval_each_generic::<S, V>(arg_expr, value, optional, cursor, &mut |item| {
        let owned = match generic_item_into_owned(item) {
            Ok(owned) => owned,
            Err(control) => {
                decode_err = Some(control);
                return Demand::Stop;
            }
        };
        if let Some(previous) = pending_first.take() {
            if let Some(control) = push_generic_owned_values(previous, &mut out) {
                body_control = Some(control);
                return Demand::Stop;
            }
        }
        let result = body(owned);
        // A `body` failure stops the pull *here*, so the argument's
        // remaining outputs are never evaluated -- `eval::fanout_arg`'s
        // rules 2/4. Checked before buffering, or an escaping first result
        // would be parked and the sink would ask for another value anyway.
        if result.is_escape() {
            if let Some(control) = push_generic_owned_values(result, &mut out) {
                body_control = Some(control);
            }
            return Demand::Stop;
        }
        if out.is_empty() && pending_first.is_none() {
            pending_first = Some(result);
        } else if let Some(control) = push_generic_owned_values(result, &mut out) {
            body_control = Some(control);
            return Demand::Stop;
        }
        Demand::Continue
    });

    // Whatever `body` already produced still stands in front of the control,
    // exactly as it does for the argument's own escape below.
    let flush_pending =
        |pending: Option<GenericResult<V>>, out: &mut Vec<OwnedValue>| -> Option<Control> {
            pending.and_then(|p| push_generic_owned_values(p, out))
        };

    if let Some(control) = decode_err {
        let control = flush_pending(pending_first, &mut out).unwrap_or(control);
        return partial_generic(out, control);
    }
    match flow {
        // `pending_first` is `Some` here exactly when one value was
        // delivered and `body` did not escape -- the single-output fast path.
        Flow::Exhausted => match pending_first {
            Some(result) => result,
            None => owned_vec_to_generic_result(out),
        },
        // Our sink is the only thing that stops this pull, and it does so
        // only after recording `body_control`. `pending` is dropped for the
        // reason every other lazy consumer drops it: it belongs to an eager
        // fallback jq would never have reached.
        Flow::Stopped { .. } => match body_control {
            Some(control) => partial_generic(out, control),
            None => owned_vec_to_generic_result(out),
        },
        // The argument's own control fires only after every body result
        // already produced, so a buffered first must be flushed ahead of it.
        Flow::Escaped(control) => {
            let control = flush_pending(pending_first, &mut out).unwrap_or(control);
            partial_generic(out, control)
        }
    }
}

fn eval_limit_generic<S: EvalSemantics, V: DocumentValue>(
    n_expr: &Expr,
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> Option<GenericResult<V>> {
    if limit_or_nth_uses_live_input_queue(n_expr, expr) {
        return None;
    }
    // `n` is the OUTER loop and `expr` the inner one, re-evaluated once per
    // `n` output -- `[limit((1,2); (10,20,30))]` is `[10,10,20]`, not
    // `[10,20,10]` (live-verified against jq 1.7.1, and the rule
    // `eval::eval_limit`'s own `fanout_arg` call already implements).
    Some(fanout_arg_generic::<S, V, _>(
        n_expr,
        value.clone(),
        optional,
        cursor,
        |n_value| limit_with_n_generic::<S, V>(n_value, expr, value.clone(), optional, cursor),
    ))
}

/// `limit(n; expr)`'s work for one already-resolved `n` -- the body of
/// [`eval_limit_generic`], run once per output of its `n` generator.
///
/// Split out for #1687 exactly as `eval::limit_with_n` was split out of
/// `eval::eval_limit` for #1279, and for the same reason: the fan-out needs
/// a per-`n` body to call.
fn limit_with_n_generic<S: EvalSemantics, V: DocumentValue>(
    n_value: OwnedValue,
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    let n = match classify_limit_n(n_value) {
        Ok(LimitN::Unlimited) => {
            return eval_single::<S, V>(expr, value, optional, cursor);
        }
        Ok(LimitN::Take(n)) => n,
        Err(e) => return GenericResult::Error(e),
    };
    if n == 0 {
        return GenericResult::None;
    }

    // Pull at most `n`, then stop the generator -- mirrors `eval.rs`'s own
    // `each_take_n`. Items stay as `GenericItem`s, not decoded yet, so
    // `items_to_generic_result` below can keep a cursor-backed batch as
    // `ManyCursor` -- preserving a duplicate key *inside* a captured item
    // too (`limit(2; .[])` on a sequence of duplicate-keyed mappings), not
    // only across the `limit`/`nth` walk itself.
    let mut out: Vec<GenericItem<V>> = Vec::new();
    let flow = eval_each_generic::<S, V>(expr, value, optional, cursor, &mut |item| {
        out.push(item);
        if out.len() >= n {
            Demand::Stop
        } else {
            Demand::Continue
        }
    });
    match flow {
        // The sink returns `Demand::Stop` the instant `out.len() >= n`, and
        // every `eval_each_generic` arm stops pulling as soon as `Demand`
        // says to -- so an escape can only fire *before* that point, never
        // after: `flow` alone already tells us whether `n` was reached,
        // with no need to separately track/re-check `out.len()` against
        // `n`. Short of `n`, there is no lazy `GenericResult` shape that
        // carries both a prefix and a pending control, so decode the
        // captured prefix eagerly and surface the trailing control
        // alongside it -- matches `limit_with_n`'s identical rule
        // (`[limit(3; 1,2,error("x"),4)]` raises). A decode failure while
        // draining that prefix is itself reported in place of the
        // generator's own control, the same priority a mid-pull `stray`
        // took before this batch conversion existed.
        Flow::Escaped(control) => {
            let mut owned = vec_with_capacity(out.len());
            let mut decode_err = None;
            for item in out {
                match generic_item_into_owned(item) {
                    Ok(v) => owned.push(v),
                    Err(c) => {
                        decode_err = Some(c);
                        break;
                    }
                }
            }
            partial_generic(owned, decode_err.unwrap_or(control))
        }
        // Either a clean stop (`n` outputs arrived, or the generator ran
        // dry on its own) or an escape *past* `n` -- jq's own
        // `foreach ... break $out` fires once `n` outputs exist, dropping
        // any trailing control, so both resolve the same way.
        _ => match items_to_generic_result(out) {
            Ok(result) => result,
            Err((prefix, control)) => partial_generic(prefix, control),
        },
    }
}

/// Native `Builtin::NthStream`/`Expr::NthExpr` arm for the generic evaluator
/// (#1607) — [`eval_limit_generic`]'s twin, same root cause, same deferral
/// contract (see its doc comment, including the input-queue guard and the
/// generator-`n` fan-out). jq defines `nth($n; f)` as
/// `last(limit($n + 1; f))`, so this pulls only as far as index `n` then
/// stops, mirroring `eval.rs`'s own `each_take_nth`.
///
/// `n`'s classification ([`classify_nth_n`], shared with `builtin_nth_stream`
/// and `eval_nth_expr` so none of the three can silently drift apart the
/// way `classify_limit_n` was extracted to prevent, #1313) matches
/// `Builtin::NthStream` exactly — `Builtin::NthStream` is the arm real
/// `nth(n; expr)` calls actually reach (see this function's call sites),
/// so its classification is the one this native path must reproduce
/// bug-for-bug.
fn eval_nth_generic<S: EvalSemantics, V: DocumentValue>(
    n_expr: &Expr,
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> Option<GenericResult<V>> {
    if limit_or_nth_uses_live_input_queue(n_expr, expr) {
        return None;
    }
    // Same outer/inner nesting as `limit` -- `[nth((0,1); (10,20,30))]` is
    // `[10,20]`, one full walk of `expr` per `n`.
    Some(fanout_arg_generic::<S, V, _>(
        n_expr,
        value.clone(),
        optional,
        cursor,
        |n_value| nth_with_n_generic::<S, V>(n_value, expr, value.clone(), optional, cursor),
    ))
}

/// `nth(n; expr)`'s work for one already-resolved `n` -- [`eval_nth_generic`]'s
/// per-`n` body, split out for the fan-out exactly as
/// [`limit_with_n_generic`] was.
fn nth_with_n_generic<S: EvalSemantics, V: DocumentValue>(
    n_value: OwnedValue,
    expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    let n = match classify_nth_n(n_value) {
        Ok(n) => n,
        Err(e) => return GenericResult::Error(e),
    };

    let mut seen = 0usize;
    let mut wanted: Vec<GenericItem<V>> = Vec::new();
    let mut skipped_err: Option<Control> = None;
    let flow = eval_each_generic::<S, V>(expr, value, optional, cursor, &mut |item| {
        // #1519: `>=`, not `==`, and a `Vec` rather than a latch -- see
        // `eval::each_take_nth`'s doc comment for why jq's own counter keeps
        // rising across a `?//` retry and emits again
        // (`[nth(1; 1 as $x ?// $y | 5, 6)]` is `[6,5]`). Outside a `?//`
        // chain the sink is never re-entered after its stop, so this is
        // exactly the previous behaviour.
        let at_or_past = seen >= n;
        seen += 1;
        if at_or_past {
            wanted.push(item);
            return Demand::Stop;
        }
        // jq defines `nth($n; f)` as `last(limit($n + 1; f))`: every output
        // of `f` up to index `n` is genuinely produced, not just the one
        // ultimately kept -- a *skipped* item's own lazy computation (a
        // buffered `map`/`select` chain, `GenericItem::LazySeq`, #724/#725)
        // must still run for its side effects/errors even though its value
        // is discarded here. `each_take_nth` in `eval.rs` gets this for
        // free (its `Item`s are never themselves lazy, only ever borrowed
        // or already-owned); this sink has to force it explicitly, or
        // `nth(2; .[] | map(10/.))` over `[[1],[0],[2]]` would silently
        // skip the division-by-zero `map` never ran on `[0]` and answer
        // from `[2]` instead of erroring. The item *at* index `n` is
        // deliberately exempted from this force -- see below.
        if let Err(control) = generic_item_into_owned(item) {
            skipped_err = Some(control);
            return Demand::Stop;
        }
        Demand::Continue
    });
    // Mirrors `each_take_nth` + its caller's own priority exactly: a
    // captured item at index `n` always wins, even over a trailing escape
    // from beyond that point (and, by construction, over `skipped_err` too
    // -- once `wanted` is set the sink returns `Demand::Stop` immediately,
    // so no later item, and no `skipped_err`, can still be pending);
    // short of reaching index `n` at all, whatever ended the pull
    // (exhaustion, a skipped item's own forced failure, or the generator's
    // own escape before index `n`) decides. The *wanted* item alone is
    // kept cursor-backed ([`generic_item_to_result`], #607's own
    // conversion) rather than forced through `OwnedValue`, so a duplicate
    // key *inside* it survives too, not just across the walk to reach it.
    if !wanted.is_empty() {
        // #1519: a later `?//` alternative's own error is genuinely reached by
        // jq after an earlier one answered, so it raises rather than being
        // dropped -- see `take_stopping_items_to_generic_result`, the rule
        // `first` and `nth` share.
        return take_stopping_items_to_generic_result(wanted, flow);
    }
    if let Some(control) = skipped_err {
        partial_generic(Vec::new(), control)
    } else {
        match flow {
            Flow::Stopped { .. } | Flow::Exhausted => GenericResult::None,
            Flow::Escaped(control) => partial_generic(Vec::new(), control),
        }
    }
}

/// Convert a batch of pulled [`GenericItem`]s into the smallest
/// [`GenericResult`] shape that represents it, preferring `ManyCursor` --
/// which keeps every item cursor-backed, so a duplicate key *inside* a
/// captured item (not only across the pull that captured it) survives --
/// over eagerly decoding into `ManyOwned`. Used by
/// [`eval_limit_generic`]'s own success path so `limit` extends #607's
/// duplicate-key fidelity to what it *captures*, not only to how it walks.
///
/// `OneCursorValue` gets its own arm rather than folding into the
/// cursor-only one below: its value is always an already-decoded key
/// string, the exact thing `each_lazy_keys_iterate_sink`'s `!sorted` arm
/// (its one construction site, #1514/#1609) exists to avoid re-decoding.
/// Collapsing straight to `ManyCursor` would throw that decode away, only
/// for `cursor_vec_to_generic_result`'s eventual `to_owned_cursor` to redo
/// it later -- fine once for `first`/`last` (`generic_item_to_result`'s own
/// doc comment already accepts that single cost), but `limit(n; keys|.[])`
/// pays it once *per captured key*, silently reintroducing O5's own
/// double-decode on the exact shape it was measured against. Since a key
/// string has no nested containers of its own, converting it directly with
/// `to_owned` carries no duplicate-key risk to reintroduce.
///
/// On a mid-batch decode failure, the error comes back paired with
/// whatever prefix already decoded cleanly, not bare -- `limit(5; f)` must
/// still stream the outputs `f` produced before a later one failed
/// (`[400/0.4]`, err) rather than losing them, matching real jq's
/// streaming-before-error contract (#400/#494) and `limit_with_n`'s own
/// identical rule. A plain `Result<_, Control>` here would have silently
/// discarded that prefix at the `?` the moment any item failed.
fn items_to_generic_result<V: DocumentValue>(
    items: Vec<GenericItem<V>>,
) -> Result<GenericResult<V>, (Vec<OwnedValue>, Control)> {
    if items
        .iter()
        .all(|item| matches!(item, GenericItem::OneCursorValue(_, _)))
    {
        let mut owned = vec_with_capacity(items.len());
        for item in items {
            let GenericItem::OneCursorValue(_, v) = item else {
                unreachable!("checked by the all() above")
            };
            match to_owned(&v) {
                Ok(o) => owned.push(o),
                Err(e) => return Err((owned, Control::Error(e))),
            }
        }
        return Ok(owned_vec_to_generic_result(owned));
    }
    if items
        .iter()
        .all(|item| matches!(item, GenericItem::OneCursor(_)))
    {
        let cursors: Vec<V::Cursor> = items
            .into_iter()
            .map(|item| match item {
                GenericItem::OneCursor(c) => c,
                _ => unreachable!("checked by the all() above"),
            })
            .collect();
        return Ok(cursor_vec_to_generic_result(cursors));
    }
    let mut owned = vec_with_capacity(items.len());
    for item in items {
        match generic_item_into_owned(item) {
            Ok(v) => owned.push(v),
            Err(control) => return Err((owned, control)),
        }
    }
    Ok(owned_vec_to_generic_result(owned))
}

/// Apply one resolved key to one target value.
///
/// Mirrors `eval::index_one`: the key *kind* is checked before the container,
/// so the error is jq's `Cannot index <container> with <key>`, and the
/// null-input passthrough is reached only for a valid key kind (`null | .["a"]`
/// is `null`, `null | .[null]` errors).
fn index_one_generic<S: EvalSemantics, V: DocumentValue>(
    target: V,
    key: &OwnedValue,
    optional: bool,
) -> GenericResult<V> {
    match key {
        OwnedValue::String(s) => {
            if let Some(fields) = target.as_object() {
                match fields.find_cursor(s) {
                    Ok(Some(c)) => GenericResult::OneCursor(c),
                    Ok(None) => GenericResult::Owned(OwnedValue::Null),
                    // #1677/#1995: same checks as `Expr::Field`'s own arm.
                    Err(err) => GenericResult::Error(err),
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
                // #2261: `len_checked`, not the bare `len()` -- this
                // resolves *any* index (`E[K]`, e.g. `.[0,1]`, `.[$i]`),
                // not just a negative one, so the length walk (needed to
                // normalize a negative index and to raise yq's own
                // out-of-range error below) already happens unconditionally
                // on every call, same reasoning as the literal-index
                // sibling arm in `eval_single`'s own `Expr::Index`.
                let len = match elements.len_checked() {
                    Ok(len) => len,
                    Err(err) => return GenericResult::Error(err),
                };
                let resolved = idx.map(|idx| if idx < 0 { len as i64 + idx } else { idx });
                // yq mode only (#2254): same rule as `eval_single`'s
                // `Expr::Index` arm above -- a negative index still
                // negative after resolving against the length raises in
                // real yq, unconditionally (see
                // `EvalError::yq_negative_index_out_of_range`'s own doc
                // comment). `resolved` is `idx.map(...)`, so `Some`/`None`
                // on the two always agree -- one `if let` on `resolved`
                // alone is enough, `idx` re-derived via `.expect` rather
                // than re-checked.
                if let Some(r) = resolved {
                    if let Some(e) = yq_negative_index_check::<S>(
                        idx.expect("resolved is Some only when idx is Some"),
                        r,
                        len,
                    ) {
                        return GenericResult::Error(e);
                    }
                }
                match resolved
                    .and_then(|r| usize::try_from(r).ok())
                    .and_then(|i| elements.get_cursor(i))
                {
                    Some(c) => GenericResult::OneCursor(c),
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
    //
    // `pending_halt` carries a halt that arrived after some keys were
    // already produced (`.[(1,2,halt)]`) — unlike `Error`/`Break`, which stay
    // conservative (see the comment on the `Partial` arm below), a halt's
    // already-produced keys still owe real jq's output before the process
    // exits: real jq's key-outer/target-inner generator model (see
    // `eval::eval_index_expr`'s doc comment) evaluates `target` once per key
    // already yielded before the key generator halts on its *next* attempt,
    // so those keys' indexed output must still reach stdout (#791).
    let mut pending_halt: Option<i32> = None;
    let keys = match eval_single::<S, V>(key, value.clone(), false, cursor) {
        GenericResult::Error(e) => return GenericResult::Error(e),
        GenericResult::Break(label) => return GenericResult::Break(label),
        // Not folded into the `other => other.collect_owned()` wildcard
        // below: `collect_owned()` treats `Halt` like `Break`/`Error` and
        // quietly returns an empty `Vec`, which here would misread a halted
        // key stream as an *empty* one (`keys.is_empty()` -> `None`),
        // silently discarding the halt instead of propagating it — unlike
        // `Break`, which already gets its own explicit early return above.
        GenericResult::Halt(code) => return GenericResult::Halt(code),
        GenericResult::None => return GenericResult::None,
        // A `Partial`'s trailing control must abort here too, not silently
        // truncate to its prefix (#694) -- mirrors the target match below.
        GenericResult::Partial(_, Control::Error(e)) => return GenericResult::Error(e),
        GenericResult::Partial(_, Control::Break(label)) => return GenericResult::Break(label),
        // Computed indexing's key/target forking isn't part of #400/#494's
        // verified semantics for `Error`/`Break` — conservatively matching
        // those arms rather than inventing new partial-key behavior for
        // them. `Halt` is different: its prefix is kept and threaded through
        // as `pending_halt` instead of discarded, per the comment above.
        GenericResult::Partial(vs, Control::Halt(code)) => {
            pending_halt = Some(code);
            vs
        }
        GenericResult::One(v) => vec![owned_or_err!(to_owned_key_shape(&v))],
        GenericResult::OneCursor(c) => vec![owned_or_err!(to_owned_key_shape_cursor(&c))],
        GenericResult::Many(vs) => owned_or_err!(vs
            .iter()
            .map(to_owned_key_shape)
            .collect::<Result<Vec<_>, _>>()),
        GenericResult::ManyCursor(cs) => owned_or_err!(cs
            .iter()
            .map(to_owned_key_shape_cursor)
            .collect::<Result<Vec<_>, _>>()),
        other => owned_or_err!(other.collect_owned()),
    };
    if keys.is_empty() {
        // `partial_generic`'s invariant (a non-empty prefix by construction)
        // means this is only reachable with `pending_halt` unset.
        return GenericResult::None;
    }

    // Key outer, target inner -- and, since #2032, `target` (`E`) is
    // re-evaluated fresh for *every* key rather than once for all of them,
    // matching jq's own `K as $k | E | .[$k]` compilation: a side effect
    // inside `E` fires once per key, not once total, and each key's own
    // output count is independent. See `eval::eval_index_expr`'s identical
    // fix for the fuller rationale and the `jq -n '[(input)[("a","b")]]'`
    // live-verified repro; this is the sibling that actually handles an
    // ordinary CLI `.[$keys]` read (see the old comment this replaced, on
    // the now-removed `owned @ (...)` arm, for why this file -- not
    // `eval::eval_index_expr` -- is what a real invocation hits).
    let mut cursors: Vec<V::Cursor> = Vec::new();
    let mut owned: Vec<OwnedValue> = Vec::new();
    let mut any_owned = false;

    // Folds the running `cursors`/`owned` accumulator into a `Partial`'s
    // prefix -- the shared exit every escape arm below funnels through, so
    // "what already indexed successfully across earlier keys" can't be
    // dropped by one arm and kept by another. Unlike the promotion step
    // below (`ensure_owned!`), a *secondary* failure converting `cursors`
    // here can't be allowed to silently discard the original `$control`:
    // `resolve_terminal_prefix` (`src/jq/eval.rs`) already establishes the
    // rule this mirrors -- a `Halt` must survive a promotion failure (jq
    // never turns a halt into a loud decode error just because rendering
    // its own already-abandoned prefix hit an unrelated problem; confirmed
    // live: `jq -n '(input | if . == "STOP" then halt else . end)
    // [("x","y")]'` on a bad-then-STOP input stream exits 0 silently in
    // real jq), while `Error`/`Break` downgrade to the promotion's own
    // error, same as `resolve_terminal_prefix`'s matching arm.
    macro_rules! escape_generic {
        ($control:expr) => {{
            let control = $control;
            let out = if any_owned {
                owned
            } else {
                match to_owned_all_cursors(&cursors) {
                    Ok(vs) => vs,
                    Err(e) => {
                        let control = match control {
                            Control::Halt(code) => Control::Halt(code),
                            Control::Error(_) | Control::Break(_) => Control::Error(e),
                        };
                        return partial_generic(Vec::new(), control);
                    }
                }
            };
            return partial_generic(out, control);
        }};
    }

    // Promotes `cursors` into `owned` on the first Owned-kind result seen,
    // shared by both `KeyTargets` arms below (#2032 review: this exact
    // 4-line sequence used to be written out twice). `owned_or_err!`'s bare
    // return on a promotion failure here is pre-existing, unchanged by
    // #2032 -- unlike `escape_generic!` above, this path was already
    // reachable before this fix (any ordinary indexing result could be the
    // first `Owned` one), so its error-only-ever handling isn't new
    // territory this fix needs to harden.
    macro_rules! ensure_owned {
        () => {
            if !any_owned {
                any_owned = true;
                owned = owned_or_err!(to_owned_all_cursors(&cursors));
                cursors.clear();
            }
        };
    }

    for k in &keys {
        // Normalized the same way the old once-for-all-keys `targets` match
        // did, minus the arms that used to return early: those now escape
        // through `escape_generic!` so the running accumulator survives.
        enum KeyTargets<V> {
            Native(Vec<V>),
            Owned(Vec<OwnedValue>),
        }
        let key_targets = match eval_single::<S, V>(target, value.clone(), false, cursor) {
            GenericResult::Error(e) => escape_generic!(Control::Error(e)),
            GenericResult::Break(label) => escape_generic!(Control::Break(label)),
            GenericResult::Halt(code) => escape_generic!(Control::Halt(code)),
            // Zero outputs for *this* key indexes to zero results for it —
            // not an error, break, or halt — so this key simply contributes
            // nothing and the loop moves on to the next one (#2032: unlike
            // the old once-for-all-keys evaluation, this no longer implies
            // every other key is empty too).
            GenericResult::None => continue,
            // #2226: `target`'s own generator produced `vs` before its own
            // mid-stream escape -- real jq's key-outer/target-inner model
            // indexes each already-produced value by `k` as it flows out,
            // the same per-value operation the `Owned`/`Native` arms below
            // already apply to a *successful* target result, so those
            // values are not "never part of the indexed output either
            // way" the way the old comment here assumed; only `target`'s
            // own escape stops it from producing more. This is jq-only
            // (review finding): real yq does not stream a target's own
            // escaped generator's prefix at all -- live-verified against yq
            // v4.53.3, `([1,2],[3,4],error("x"))[(0,1)]` prints only
            // `Error: x`, no prefix -- so yq mode keeps the old
            // conservative discard here (matches ADR-0018's "the mode
            // decides" rule; see `eval::eval_index_expr`'s identical gate).
            //
            // Mirrors `KeyTargets::Owned`'s own loop below exactly (this
            // prefix is already `Vec<OwnedValue>`, the same shape `Owned`
            // handles): an indexing failure on an earlier-produced value
            // fires before `target`'s own later escape ever would in the
            // real generator order, so it outranks `control` here the same
            // way a later key's index error already outranks an earlier
            // key's pending halt in the `Owned`/`Native` arms; only once
            // every value in `vs` indexes cleanly does `control` --
            // `target`'s own termination -- get to fire.
            //
            // Does *not* reuse `ensure_owned!()`: that macro's own
            // `owned_or_err!`-based promotion bare-returns
            // `GenericResult::Error` on a decode failure, unconditionally
            // discarding whatever `control` this arm is holding --
            // including an uncatchable `Halt`, which `escape_generic!`
            // above this loop already goes out of its way to preserve
            // through the identical promotion step. Inlined here instead,
            // mirroring `escape_generic!`'s own Halt-survives rule exactly
            // rather than reintroducing the bug it was written to avoid.
            //
            // The `Err` arm below is not live-repro-tested (review): same
            // reasoning as `eval::eval_index_expr`'s identical arm -- an
            // *earlier* key populating `cursors` and a *later* key reaching
            // this `Partial` arm both require the one fixed `target`
            // expression to behave non-deterministically across two fresh
            // evaluations against the same document, and the only stateful
            // builtin that could do that (`input`) always resolves to an
            // owned value, never a cursor. Kept for the same defensive,
            // future-proofing reason `resolve_terminal_prefix` keeps its own
            // identical handling.
            GenericResult::Partial(vs, control) => {
                if S::TAG == EvalTag::Yq {
                    escape_generic!(control);
                }
                if !any_owned {
                    any_owned = true;
                    owned = match to_owned_all_cursors(&cursors) {
                        Ok(vs) => vs,
                        Err(e) => {
                            let control = match control {
                                Control::Halt(code) => Control::Halt(code),
                                Control::Error(_) | Control::Break(_) => Control::Error(e),
                            };
                            return partial_generic(Vec::new(), control);
                        }
                    };
                    cursors.clear();
                }
                if owned.try_reserve(vs.len()).is_err() {
                    escape_generic!(Control::Error(cannot_reserve_cross_product(&[vs.len()])));
                }
                for t in &vs {
                    match index_owned_by_key(t, k, optional) {
                        Ok(Some(v)) => owned.push(v),
                        Ok(None) => {}
                        Err(e) => escape_generic!(Control::Error(e)),
                    }
                }
                escape_generic!(control)
            }
            GenericResult::One(v) => KeyTargets::Native(vec![v]),
            GenericResult::Many(vs) => KeyTargets::Native(vs),
            GenericResult::OneCursor(c) => KeyTargets::Native(vec![c.value()]),
            GenericResult::ManyCursor(cs) => {
                KeyTargets::Native(cs.iter().map(DocumentCursor::value).collect())
            }
            // An owned target (a computed, non-navigational left side) has
            // no borrowed representation here; round-trip it through the
            // shared owned-value path. `LazySeq` shares this arm too:
            // `collect_owned()` materializes it (via `materialize_lazy()`)
            // the same as the other lazy variants. Known, narrow,
            // pre-existing gap, not a new regression (unchanged by #2032):
            // `collect_owned()` already silently swallows any error into an
            // empty `Vec` for every variant that can error (`Partial`, and
            // a failing `LazySeq`).
            owned_kind @ (GenericResult::Owned(_)
            | GenericResult::ManyOwned(_)
            | GenericResult::LazyKeys { .. }
            | GenericResult::LazyIndexRange(_)
            | GenericResult::LazySeq(_)) => match owned_kind.collect_owned() {
                Ok(vs) => KeyTargets::Owned(vs),
                Err(e) => escape_generic!(Control::Error(e)),
            },
        };
        match key_targets {
            KeyTargets::Native(ts) => {
                // Reserved once per key, ahead of that key's own indexing
                // loop, mirroring `eval::eval_index_expr`'s identical fix
                // (#2032 review: the old upfront `try_reserve_product(&[
                // keys.len(), targets.len()])`, visible as deleted in this
                // diff, is no longer computable now that a key's own
                // target length varies per key -- but this file is the
                // one an ordinary CLI `.[$keys]` read actually reaches, so
                // dropping the guard entirely here (as an earlier revision
                // of this fix did) reopens the #1017/#1612/#1634/#1669
                // abort-on-allocation-failure class of bug for real
                // traffic, not just a narrower fallback path). Reserved on
                // whichever accumulator is *currently* live -- this covers
                // every push below only when that liveness doesn't change
                // mid-loop; the per-push `try_reserve(1)` calls on each arm
                // below are what cover the case where it does (an earlier
                // target in this same `ts` flips `any_owned` partway
                // through, past the point this reservation could see it).
                let reserved = if any_owned {
                    owned.try_reserve(ts.len())
                } else {
                    cursors.try_reserve(ts.len())
                };
                if reserved.is_err() {
                    escape_generic!(Control::Error(cannot_reserve_cross_product(&[ts.len()])));
                }
                for t in &ts {
                    match index_one_generic::<S, V>(t.clone(), k, optional) {
                        GenericResult::OneCursor(c) => {
                            if any_owned {
                                // Not `owned_or_err!`: that bare-returns and
                                // would discard every key/target pair
                                // already resolved into `owned` ahead of
                                // this one. The already-indexed prefix must
                                // survive as `Partial`, same as the
                                // `GenericResult::Error` arm below.
                                //
                                // The reservation above only covers `owned`
                                // when `any_owned` was *already* true before
                                // this key's loop started -- if a sibling
                                // target earlier in this same `ts` flipped
                                // it mid-loop (via the `Owned` arm below),
                                // that upfront reservation landed on
                                // `cursors` instead and covers none of the
                                // pushes here. `try_reserve(1)` is a cheap
                                // capacity check when the upfront batch
                                // reservation already covers this push (the
                                // common case), and the only real guard when
                                // it doesn't (review finding: an unreserved
                                // `push` after a mid-loop flip re-admits the
                                // #1017/#1612/#1634/#1669 abort class this
                                // guard exists to close).
                                match to_owned_cursor(&c) {
                                    Ok(v) => {
                                        if owned.try_reserve(1).is_err() {
                                            escape_generic!(Control::Error(
                                                cannot_reserve_cross_product(&[1])
                                            ));
                                        }
                                        owned.push(v);
                                    }
                                    Err(e) => escape_generic!(Control::Error(e)),
                                }
                            } else {
                                cursors.push(c);
                            }
                        }
                        GenericResult::Owned(v) => {
                            // Same mid-loop-flip gap as the cursor arm above
                            // -- `ensure_owned!` converts what `cursors`
                            // already held, but has no knowledge of the
                            // still-pending items in *this* `ts` loop, so it
                            // reserves nothing for them.
                            ensure_owned!();
                            if owned.try_reserve(1).is_err() {
                                escape_generic!(Control::Error(cannot_reserve_cross_product(&[1])));
                            }
                            owned.push(v);
                        }
                        GenericResult::None => {}
                        // A later key's index error outranks an earlier
                        // key's still-pending halt (verified against jq
                        // 1.7.1/1.8.2: `{"a":1} | .[("a", 5, halt)]` prints
                        // `1`, then the "Cannot index object with number"
                        // error, and never reaches `halt`) — the
                        // already-indexed prefix must still survive as
                        // `Partial`, not vanish with it.
                        GenericResult::Error(e) => escape_generic!(Control::Error(e)),
                        _ => unreachable!("index_one_generic yields OneCursor/Owned/None/Error"),
                    }
                }
            }
            KeyTargets::Owned(ts) => {
                // Ensures `owned` exists *before* reserving onto it, so the
                // reservation lands on the vec that will actually receive
                // this batch -- `ensure_owned!`'s own promotion (via
                // `to_owned_all_cursors`) has no knowledge of `ts.len()`'s
                // pending pushes, only of `cursors`'s existing length.
                ensure_owned!();
                if owned.try_reserve(ts.len()).is_err() {
                    escape_generic!(Control::Error(cannot_reserve_cross_product(&[ts.len()])));
                }
                for t in &ts {
                    match index_owned_by_key(t, k, optional) {
                        Ok(Some(v)) => owned.push(v),
                        Ok(None) => {}
                        // Same reasoning as the `Native` arm above: a later
                        // key's index error outranks an earlier key's
                        // pending halt, and the already-indexed prefix
                        // survives it.
                        Err(e) => escape_generic!(Control::Error(e)),
                    }
                }
            }
        }
    }

    if let Some(code) = pending_halt {
        // Known fidelity gap, same family as #631: `GenericResult::Partial`
        // is `Vec<OwnedValue>`, so a cursor prefix has to go through
        // `to_owned()` here — which collapses duplicate YAML mapping keys
        // that streaming the cursors directly (the no-halt path below)
        // preserves. `.[(0,1)]` on a document with a duplicate-keyed element
        // keeps both keys; `.[(0,1,halt)]` on the same document silently
        // loses one, because appending `halt` routes the same prefix through
        // this arm instead. Fixing it for real needs `Partial` itself to
        // carry cursors, not just owned values — the same rework #631 is
        // already deferred on — so this only documents the trade-off rather
        // than papering over it.
        let out = if any_owned {
            owned
        } else {
            owned_or_err!(to_owned_all_cursors(&cursors))
        };
        return partial_generic(out, Control::Halt(code));
    }

    // #1048: a zero-result collapse here (every key/target pair
    // optional-suppressed) must be `None`, not `ManyOwned`/`ManyCursor(vec![])`.
    if any_owned {
        owned_vec_to_generic_result(owned)
    } else {
        cursor_vec_to_generic_result(cursors)
    }
}

/// Evaluate `E[S:T]` — slicing by computed bounds.
///
/// The counterpart of `eval::eval_slice_expr`; mirrors `eval_index_expr`
/// immediately above — `start`/`end` are evaluated first (outermost) against
/// the original value/cursor, not the target's output, and an empty bound
/// stream short-circuits before the target is evaluated at all. See #615.
///
/// #2143: `target` (`E`) is now re-evaluated fresh for every `(s, e)` pair
/// rather than once overall, mirroring #2032/#2142's identical fix for
/// `eval_index_expr` one nesting level deeper — jq's own
/// `S as $s | T as $t | E | .[$s:$t]` desugaring puts `E` inside both
/// bindings, so a side effect in `E` (`stderr`, `input`) must fire once per
/// `(s, e)` pair, not once total (verified against jq 1.7.1: `[10,20,30] |
/// [(stderr)[(0,1):(2,3)]]` writes `[10,20,30]` to stderr four times, once
/// per pair). `target`'s own escape (`Error`/`Break`/`Halt`/`Partial`) can
/// now fire after *earlier pairs* already contributed real output to `out`,
/// so it folds `out` in as a `Partial` prefix instead of discarding it —
/// this cross-pair prefix was provably always empty before this fix (`E`
/// used to be evaluated once, before the loop could produce anything), so
/// there was nothing to lose by discarding it there; now there can be. The
/// per-target slice-application escape one level in got the identical
/// cross-pair fix for the identical reason (review). #2226 later closed the
/// sibling gap this comment used to describe as left deliberately
/// unaddressed here: a `Partial(prefix, control)` target result no longer
/// just discards `prefix` — the values `E`'s *own* generator produced
/// before erroring *within* one (s, e) pair (as opposed to the cross-pair
/// `out` this fix already threads through) are now sliced and folded into
/// `out` too, in jq mode (real yq does not stream this prefix at all; see
/// the `Partial` arm's own comment for the live-verified yq-mode carve-out)
/// — e.g. `E = ([1,2],[3,4],error("x"))` now emits the `[1]`/`[3]` slices
/// before raising `x`, matching jq 1.7.1.
///
/// #2225: `T` (`end`) is *also* now re-evaluated fresh for every `s`, not
/// once overall as an earlier revision of this doc comment claimed ("its
/// own (prefix, escape) pair is identical on every notional re-run" — false
/// whenever `T` involves a stateful generator like `input`, and jq's own
/// desugaring puts `T` inside `S`'s binding scope for exactly this reason).
/// Confirmed live against jq 1.7.1 with a stateful `T`
/// (`input as $a | $a[(0,1):(input)]` fed two distinct bound values on
/// stdin, one per `s`): each `s` gets its own fresh `T` result, not a
/// shared one. An empty `T` for a given `s` contributes nothing for that
/// `s` and moves on to the next one — not a whole-function short-circuit
/// the way an entirely-empty `S` is (verified live: a `T` that's empty only
/// for `s == 0` still lets `s == 1` run). A `T` that produces some values
/// before escaping processes those first, then folds the running `out` in
/// as the escape's prefix, same as every other per-iteration escape here
/// (verified live: `.[0:(1,2,error("boom"))]` prints the slices for `1`
/// and `2` before raising).
fn eval_slice_expr<S: EvalSemantics, V: DocumentValue>(
    target: &Expr,
    start: &Option<Box<Expr>>,
    end: &Option<Box<Expr>>,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    // Bounds first: an empty start stream must not evaluate `end` or the
    // target at all.
    //
    // #1528: keeps each bound generator's own partial prefix instead of
    // discarding it on escape (same fix #1517 applied to the path-mode
    // resolver, re-derived here for value mode).
    let (starts, starts_escape) =
        match eval_slice_bound::<S, V>(start, value.clone(), cursor, f64::floor) {
            Ok(v) => v,
            Err(control) => return partial_generic(Vec::new(), control),
        };
    if starts.is_empty() {
        return match starts_escape {
            None => GenericResult::None,
            Some(control) => partial_generic(Vec::new(), control),
        };
    }

    // Borrowed and owned targets are kept apart so the common (borrowed) case
    // never materializes the document — mirrors `eval_index_expr`.
    enum Targets<V> {
        Borrowed(Vec<V>),
        Owned(Vec<OwnedValue>),
    }

    // Start outer, end middle, target inner (#2143: target is now
    // re-evaluated once per (s, e) pair, so it is genuinely the innermost
    // stage, not just looped as if it were). `end` (#2225) is now
    // re-evaluated once per `s`, so it is genuinely nested inside `start`'s
    // own binding scope too, not looped as if it were. The result is
    // always owned: slicing constructs a fresh array/string, same
    // invariant as `eval::eval_slice_expr`. This is the actual dispatch
    // path for an ordinary `.[$s:$e]` CLI read (see the comment on
    // `Expr::SliceExpr`'s own match arm above), so this site -- not
    // `eval::eval_slice_expr`'s sibling -- is what a real
    // `succinctly jq`/`succinctly yq` invocation hits.
    //
    // No upfront `starts.len() * ends.len()` reservation baseline anymore:
    // `ends.len()` is no longer known until each `s` iteration evaluates
    // `end`, so there is nothing fixed to reserve before the loop starts.
    // The per-target `try_reserve` calls inside the loop still protect
    // every real allocation; only the pre-#2225 upfront-baseline
    // optimization is gone, not the overflow protection itself.
    let mut out: Vec<OwnedValue> = Vec::new();

    // The shared exit every escape arm below funnels through, so folding
    // the running `out` in as a `Partial` prefix can't drift between arms
    // -- the single-accumulator counterpart of `eval_index_expr`'s
    // `escape_generic!` above (that macro's own cursor/owned promotion has
    // nothing to do here: `out` is always `Vec<OwnedValue>`, never a
    // separate cursor accumulator).
    macro_rules! escape {
        ($control:expr) => {
            return partial_generic(out, $control)
        };
    }

    for s in &starts {
        // #2225: `end` evaluated fresh for this `s`, not once overall.
        let (ends, ends_escape) =
            match eval_slice_bound::<S, V>(end, value.clone(), cursor, f64::ceil) {
                Ok(v) => v,
                Err(control) => escape!(control),
            };
        for e in &ends {
            let targets = match eval_single::<S, V>(target, value.clone(), false, cursor) {
                GenericResult::Error(e) => escape!(Control::Error(e)),
                GenericResult::Break(label) => escape!(Control::Break(label)),
                GenericResult::Halt(code) => escape!(Control::Halt(code)),
                // Zero outputs for *this* (s, e) pair contributes nothing to
                // it -- not a whole-function short-circuit, now that E's own
                // output count can vary across pairs (mirrors
                // `eval_index_expr`'s identical per-key treatment).
                GenericResult::None => continue,
                // #2226: this (s, e) pair's own target generator may have
                // produced some values before escaping (a break/halt/error
                // partway through its own stream) -- apply the slice to each
                // of those already-produced values and fold them into `out`
                // before escaping, mirroring `eval_index_expr`'s identical
                // fix for its own target `Partial` arm, rather than
                // discarding them. jq-only (review finding, same gate as
                // `eval_index_expr` above): real yq does not stream a
                // target's own escaped generator's prefix -- live-verified
                // against yq v4.53.3, `([1,2],[3,4],error("x"))[(0,1):(1,2)]`
                // prints only `Error: x`. yq mode keeps the old conservative
                // discard.
                GenericResult::Partial(vs, control) => {
                    if S::TAG == EvalTag::Yq {
                        escape!(control);
                    }
                    if out.try_reserve(vs.len()).is_err() {
                        escape!(Control::Error(cannot_reserve_cross_product(&[vs.len()])));
                    }
                    for t in &vs {
                        match slice_owned_value_read::<S>(t, *s, *e, optional) {
                            Ok(Some(v)) => out.push(v),
                            Ok(None) => {}
                            Err(e) => escape!(Control::Error(e)),
                        }
                    }
                    escape!(control)
                }
                GenericResult::One(v) => Targets::Borrowed(vec![v]),
                GenericResult::Many(vs) => Targets::Borrowed(vs),
                GenericResult::OneCursor(c) => Targets::Borrowed(vec![c.value()]),
                GenericResult::ManyCursor(cs) => {
                    Targets::Borrowed(cs.iter().map(DocumentCursor::value).collect())
                }
                // See `eval_index_expr`'s identical arm for the accepted,
                // pre-existing error-swallowing note that also applies to
                // `LazySeq` here.
                owned @ (GenericResult::Owned(_)
                | GenericResult::ManyOwned(_)
                | GenericResult::LazyKeys { .. }
                | GenericResult::LazyIndexRange(_)
                | GenericResult::LazySeq(_)) => match owned.collect_owned() {
                    Ok(vs) => Targets::Owned(vs),
                    Err(err) => escape!(Control::Error(err)),
                },
            };
            match &targets {
                Targets::Borrowed(ts) => {
                    if out.try_reserve(ts.len()).is_err() {
                        escape!(Control::Error(cannot_reserve_cross_product(&[ts.len()])));
                    }
                    for t in ts {
                        match slice_one_generic::<S, V>(t.clone(), *s, *e, optional) {
                            GenericResult::Owned(v) => out.push(v),
                            GenericResult::None => {}
                            // #2143 (review): a later (s, e) pair's/target's
                            // slice-application error must not discard the
                            // values already produced by earlier ones --
                            // same "later step's error outranks an earlier
                            // already-produced prefix, which still survives
                            // as Partial" rule this function's own
                            // target-evaluation escape above follows,
                            // mirroring `eval_index_expr`'s identical
                            // treatment of a later key's index error.
                            // Pre-existing gap (predates #2143, confirmed
                            // live against jq 1.7.1: `[1,2,3,4] |
                            // (.,5)[(1-1):(1+1)]` prints `[1,2]` before
                            // raising "Cannot index number with object";
                            // this arm used to discard it), fixed here now
                            // that it sits directly beneath the
                            // target-evaluation fix above making the same
                            // claim.
                            GenericResult::Error(e) => escape!(Control::Error(e)),
                            _ => unreachable!("slice_one_generic yields Owned/None/Error"),
                        }
                    }
                }
                Targets::Owned(ts) => {
                    if out.try_reserve(ts.len()).is_err() {
                        escape!(Control::Error(cannot_reserve_cross_product(&[ts.len()])));
                    }
                    for t in ts {
                        match slice_owned_value_read::<S>(t, *s, *e, optional) {
                            Ok(Some(v)) => out.push(v),
                            Ok(None) => {}
                            // #2143 (review): same fix as the Borrowed arm
                            // above.
                            Err(e) => escape!(Control::Error(e)),
                        }
                    }
                }
            }
        }
        // #2225: this `s`'s own `end` evaluation may have produced some
        // values before escaping (a break/halt/error partway through its
        // own stream) -- fold the running `out` in as that escape's
        // prefix, same as every other per-iteration escape above, rather
        // than discarding it or deferring it past values a *later* `s`
        // might still contribute.
        if let Some(control) = ends_escape {
            escape!(control);
        }
    }
    // #1528: `start`'s own trailing escape info still has to reach the
    // final result -- a successful loop doesn't mean `start` itself didn't
    // escape after producing `out`'s own values. `end`'s own escape is now
    // handled per-`s` above (#2225), not here.
    match starts_escape {
        // #1048: a zero-result collapse here (every (start, end) pair
        // optional-suppressed) must be `None`, not `ManyOwned(vec![])`.
        None => owned_vec_to_generic_result(out),
        Some(control) => partial_generic(out, control),
    }
}

/// Evaluate one slice bound (`start` or `end`) against `value`/`cursor`.
/// `round` is `f64::floor` for a start bound, `f64::ceil` for an end bound, so
/// a fractional dynamic bound still widens the slice the way a literal one
/// does — see `eval::eval_slice_bound`. A missing bound (`None`) is a single
/// `None` ("open on this side"), not an empty stream.
///
/// Keeps the bound generator's own partial prefix rather than discarding it
/// on escape (#1528, mirroring #1517's identical fix for the path-mode
/// resolver): the `Ok` tuple's second element is the escape (if any) that
/// ended the stream, with `raw`/the converted `Vec` still holding whatever
/// came before it. `Err` is reserved for a genuine type failure converting an
/// already-resolved value (`owned_bound_to_i64` rejecting a non-numeric
/// bound) -- same residual gap #1517's own doc comment leaves open: this
/// still discards any prefix converted cleanly before that one bad value,
/// a different failure shape from a generator's own escape that needs its
/// own priority-ordering pass to fix, not a quick addition here.
fn eval_slice_bound<S: EvalSemantics, V: DocumentValue>(
    bound: &Option<Box<Expr>>,
    value: V,
    cursor: Option<V::Cursor>,
    round: fn(f64) -> f64,
) -> Result<(Vec<Option<i64>>, Option<Control>), Control> {
    let Some(expr) = bound else {
        return Ok((vec![None], None));
    };
    let (raw, escape): (Vec<OwnedValue>, Option<Control>) =
        match eval_single::<S, V>(expr, value, false, cursor) {
            GenericResult::Error(e) => (Vec::new(), Some(Control::Error(e))),
            GenericResult::Break(label) => (Vec::new(), Some(Control::Break(label))),
            // Same "keep whatever prefix came before it" treatment as `Error`/
            // `Break` -- a dynamic slice bound like `.[:halt_error(3)]` must
            // still surface the halt (#791), it just no longer has to discard a
            // successfully-produced prefix to do so.
            GenericResult::Halt(code) => (Vec::new(), Some(Control::Halt(code))),
            GenericResult::None => (Vec::new(), None),
            GenericResult::Partial(vs, control) => (vs, Some(control)),
            GenericResult::One(v) => (vec![to_owned_key_shape(&v).map_err(Control::Error)?], None),
            GenericResult::OneCursor(c) => (
                vec![to_owned_key_shape_cursor(&c).map_err(Control::Error)?],
                None,
            ),
            GenericResult::Many(vs) => (
                vs.iter()
                    .map(to_owned_key_shape)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(Control::Error)?,
                None,
            ),
            GenericResult::ManyCursor(cs) => (
                cs.iter()
                    .map(to_owned_key_shape_cursor)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(Control::Error)?,
                None,
            ),
            other => (other.collect_owned().map_err(Control::Error)?, None),
        };
    let converted = raw
        .iter()
        .map(|v| owned_bound_to_i64(v, round).map_err(Control::Error))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((converted, escape))
}

/// Apply resolved bounds to one borrowed target. Mirrors `eval::eval_single`'s
/// `Expr::Slice` arm (arrays/strings/null) directly against `DocumentValue` —
/// there is no native `Expr::Slice` arm here to delegate to, unlike the
/// non-generic evaluator. Always returns owned, since slicing always
/// constructs a fresh value; parallels `index_one_generic` returning `One` or
/// `Owned`.
///
/// Only called from `eval_slice_expr`'s *read* path, never for an
/// assignment target — no `S`-gated equivalent of `is_yq_slice_empty_container_scalar`'s
/// caution about `resolve_slice_expr` applies here.
fn slice_one_generic<S: EvalSemantics, V: DocumentValue>(
    target: V,
    start: Option<i64>,
    end: Option<i64>,
    optional: bool,
) -> GenericResult<V> {
    if let Some(elements) = target.as_array() {
        let items = elements.collect_values();
        let range = SliceBounds::from_literals(start, end).resolve(items.len());
        // #2001 (code review): a #1194 malformed-member error nested
        // inside a sliced element respects `optional` here too -- this is
        // the `eval_generic` twin of `eval.rs`'s own `eval_single`
        // literal-bounds `Expr::Slice` array arm, which had the identical
        // gap this same PR fixed (see that arm's own #2001 comment). Can't
        // reuse `eval.rs`'s `suppress_or_raise`/`to_owned_or_suppress!`
        // directly: different result type (`GenericResult` vs
        // `QueryResult`), so the equivalent check is inlined instead.
        return match to_owned_all(items[range].iter()) {
            Ok(v) => GenericResult::Owned(OwnedValue::Array(v)),
            Err(e) if optional && !e.is_decode_failure() => GenericResult::None,
            Err(e) => GenericResult::Error(e),
        };
    }
    // yq's object AST-child-layout slicing rule (#1102) — mirrors
    // `eval.rs`'s cursor-backed `Expr::Slice` arm for the same target type;
    // see `slice::SliceBounds::resolve_object_children`'s doc comment for
    // the full, oracle-verified rule. Materializes via `to_owned` since
    // `DocumentFields` only exposes a cons-list walk (`uncons`), not the
    // `IndexMap` `slice_object_as_yq_children` needs — same technique as
    // `eval.rs`'s own arm, for the same reason.
    if S::TAG == EvalTag::Yq && target.as_object().is_some() {
        // #2001 (code review): same fix as the Array arm above -- a #1194
        // malformed-member error nested inside the object respects
        // `optional`.
        let owned = match to_owned(&target) {
            Ok(v) => v,
            Err(e) if optional && !e.is_decode_failure() => return GenericResult::None,
            Err(e) => return GenericResult::Error(e),
        };
        let OwnedValue::Object(map) = owned else {
            unreachable!("target.as_object() just confirmed this materializes to an Object")
        };
        return GenericResult::Owned(slice_object_as_yq_children(&map, start, end));
    }
    // yq's empty-container slicing rule (#1065) — see
    // `is_yq_slice_empty_container_scalar`'s doc comment for the full
    // rationale and why Object is excluded. Checked before the jq-only
    // `is_null` arm below so it wins under yq mode. Classified via
    // `type_name()`, a variant-based check, not by chaining
    // `as_bool()`/`as_i64()`/`as_f64()` — those are parseability checks,
    // and a JSON number whose scanner-accepted span isn't valid number
    // syntax (e.g. `1.2.3`, #966) fails all three despite `type_name()`
    // correctly still reporting `"number"`, which would silently disagree
    // with `is_yq_slice_empty_container_scalar`'s type-based `OwnedValue`
    // match. For YAML this also collapses what was up to two separate
    // `resolve_plain` re-derivations of the same scalar into the one
    // `type_name()` already performs.
    if S::TAG == EvalTag::Yq && matches!(target.type_name(), "null" | "boolean" | "number") {
        return GenericResult::Owned(OwnedValue::Array(Vec::new()));
    }
    if target.is_null() {
        return GenericResult::Owned(OwnedValue::Null);
    }
    if let Some(s) = target.as_str() {
        let len = s.chars().count();
        let range = SliceBounds::from_literals(start, end).resolve(len);
        return GenericResult::Owned(OwnedValue::String(slice_str(&s, range)));
    }
    if optional {
        GenericResult::None
    } else {
        GenericResult::Error(EvalError::cannot_index_with_type(
            target.type_name(),
            "object",
        ))
    }
}

/// Evaluate a builtin function.
/// Recursively collect paths in `value`'s tree, cursor-native (#868) — the
/// shared walk behind both `paths` (`leaves_only: false`, a path for every
/// node) and `leaf_paths` (`leaves_only: true`, a path only for a childless
/// node -- `null` and empty `{}`/`[]` count as leaves too, unlike the
/// `paths(scalars)` community recipe; see #771).
///
/// Mirrors `eval.rs`'s `collect_paths`/`collect_leaf_paths`, but walks `V`
/// directly via `as_object`/`as_array` and `effective_fields`/`uncons`
/// instead of an already-materialized `OwnedValue` -- the same reasoning
/// `ToEntries`'s own native arm above gives: `builtin_paths`'s
/// `to_owned(&value)` collapses duplicate YAML mapping keys into one
/// `IndexMap` entry *before* the walk ever starts, so a repeated key only
/// ever contributes one path there. Using `effective_fields` here applies
/// the evaluation *mode*'s duplicate-key rule during the walk itself (yq:
/// every occurrence; jq: first position, last value) instead of an
/// unconditional collapse. That axis is the mode, not the input format
/// (#1385, ADR-0018 rule 2): real yq preserves a repeated key whether the
/// document arrived as YAML or as JSON, which the format-gated predicate
/// this replaced got wrong for JSON input.
/// One shared walker rather than two near-copies
/// (code review, #868): the two modes differ only in *when* a path is
/// recorded, not in how the tree is traversed.
///
/// `current_path.push`/`.pop()` around each recursive call (rather than a
/// clone-and-extend style) is safe here despite an unbalanced `push` being
/// reachable: the `?` on a nested recursive call (#1829) can now return
/// `Err` right after a `push`, with no matching `pop`. That's fine only
/// because `current_path` is never read again afterward -- every caller
/// (`Builtin::Paths`/`Builtin::LeafPaths` in `eval_builtin`) passes a fresh
/// `&mut Vec::new()` that is discarded whole the instant `Err` propagates
/// out, not reused for a later path. A caller that ever needed to keep
/// walking after absorbing this error (unlike today's "abort the whole
/// builtin" policy) would have to restore the balance on that path too.
///
/// The array branch walks `uncons()` directly rather than materializing
/// `collect_values()`'s `Vec` first (code review, #868) -- arrays have no
/// duplicate-key concept to reconcile, so there's nothing `effective_fields`
/// buys here that a plain cons-list walk doesn't already give for free.
///
/// Panics past [`MAX_NESTING_DEPTH`] levels of nesting, the same guard
/// `to_owned`/`to_owned_cursor` already carry -- `current_path.len()` tracks
/// depth without a separate counter, same shape as `collect_paths`'s own
/// `#1021` guard.
///
/// #1829: `effective_fields_checked`, not the infallible `effective_fields`
/// -- this is the CLI's own native `Builtin::Paths`/`Builtin::LeafPaths`
/// dispatch (`eval_generic.rs`'s `eval_builtin`), reached directly by
/// `succinctly jq`/`succinctly yq`, not through `eval.rs`'s
/// `StandardJson`-specific `builtin_paths`/`builtin_leaf_paths` (those are
/// reachable only via the public library entry point
/// `succinctly::jq::eval`, per #1755's own precedent comment on the
/// sort/min/max family having the identical library-only gap). This
/// function's own doc comment used to admit "no error channel to raise
/// through" for a structurally malformed key (#1194) -- fixed by giving it
/// one, matching `keys`/`to_entries`/`map_values`'s already-established
/// #1194/#1677 policy on the same document.
fn collect_paths_generic<S: EvalSemantics, V: DocumentValue>(
    value: &V,
    current_path: &mut Vec<OwnedValue>,
    paths: &mut Vec<OwnedValue>,
    leaves_only: bool,
) -> Result<(), EvalError> {
    assert_nesting_depth(current_path.len());
    if let Some(fields) = value.as_object() {
        let checked = effective_fields_checked(&fields, S::COLLAPSE_DUPLICATE_KEYS)?;
        if checked.is_empty() {
            if leaves_only {
                paths.push(OwnedValue::Array(current_path.clone()));
            }
            return Ok(());
        }
        for field in checked {
            // Unreachable in practice -- `effective_fields_checked` already
            // rejected a structurally malformed key (#1194) -- checked
            // anyway, matching `effective_keys`'s own defensive style
            // rather than assuming.
            let Some(key) = key_display_string(&field.key) else {
                return Err(fields.malformed_member_error());
            };
            current_path.push(OwnedValue::String(key.into_owned()));
            if !leaves_only {
                paths.push(OwnedValue::Array(current_path.clone()));
            }
            collect_paths_generic::<S, _>(&field.value, current_path, paths, leaves_only)?;
            current_path.pop();
        }
    } else if let Some(elements) = value.as_array() {
        if elements.is_empty() {
            if leaves_only {
                paths.push(OwnedValue::Array(current_path.clone()));
            }
            return Ok(());
        }
        let mut i = 0i64;
        let mut rest = elements;
        while let Some((elem, tail)) = rest.uncons() {
            current_path.push(OwnedValue::Int(i));
            if !leaves_only {
                paths.push(OwnedValue::Array(current_path.clone()));
            }
            collect_paths_generic::<S, _>(&elem, current_path, paths, leaves_only)?;
            current_path.pop();
            i += 1;
            rest = tail;
        }
    } else if leaves_only {
        paths.push(OwnedValue::Array(current_path.clone()));
    }
    Ok(())
}

/// Native `Builtin::Has` arm for the generic evaluator (#1739): checks
/// membership directly against `value`'s existing object/array structure
/// instead of paying for the `_` fallback's full materialize + re-serialize +
/// re-index round trip. Same match shape as `eval::has_one_key`, ported to
/// `V: DocumentValue` rather than reused directly since that function is
/// `StandardJson`-specific.
///
/// Returns `None` when `key_expr` doesn't evaluate to a single plain value
/// (a generator key, `empty`, a decode failure, ...) -- the caller falls back
/// to the pre-existing round-trip path, which already implements jq's
/// per-output fan-out and yq's first-only truncation correctly (see this
/// function's own call site for why that shape isn't reproduced here).
///
/// Guards `key_expr` against a live `input`/`inputs` queue before ever
/// probing it, mirroring `eval_limit_generic`/`eval_first_or_last_generic`'s
/// own #1309 guard (`limit_or_nth_uses_live_input_queue`) rather than a new
/// copy of the same check: `has(inputs)` must fan out over the *rest* of the
/// queue as a generator key on the round-trip path below, exactly like a
/// bare `inputs` does elsewhere -- probing it here first via `eval_single`
/// would drain the whole queue as a side effect before this function ever
/// discovers it isn't a single value, leaving the fallback to run against an
/// already-empty queue (code review, #1739).
///
/// Also keeps a static bare-top-level-`Comma` check: `has(("a","b"))`'s key
/// is a generator by construction, so probing it here would run any side
/// effect inside it (`has(("a"|stderr),"z")`) once during the probe and a
/// second time when the fallback below re-evaluates the whole, unmodified
/// `key_expr` from scratch -- detecting the shape statically, without ever
/// calling `eval_single` on it, keeps the fallback's own evaluation the only
/// one.
///
/// The check is deliberately shallow: it only recognizes `key_expr` itself
/// being an (optionally parenthesized) top-level `Comma`, not one buried
/// inside some other construct (`if true then ("a"|stderr),"z" else "q" end`,
/// or any `key_expr` that raises an error rather than yielding a plain
/// value). Such a shape still gets probed once here, discovers it isn't a
/// single value, and falls back to a second full evaluation.
///
/// **`limit`/`nth` no longer share this residual; `has` deliberately still
/// does.** #1687 built [`fanout_arg_generic`] -- the shared "run the body
/// once per argument output, without double-evaluating a probe" primitive
/// #1739's own review asked for -- and moved `eval_limit_generic`/
/// `eval_nth_generic`/`each_limit_generic` onto it, deleting their copies of
/// this guard. `has` is not moved with them because it needs a fan-out mode
/// that primitive does not model: real yq takes only a multi-output key's
/// *first* output where jq fans out (`ArgFanout::yq_native`), and
/// `fanout_arg_generic` implements `ArgFanout::All` alone, which is all its
/// three callers need. Reproducing yq's truncation (and
/// `clear_values_when_yq_argument_escaped`, which rides with it) here would
/// be a second copy of `eval::fanout_arg`'s non-`All` half -- so `has`
/// keeps handing that shape to the one existing implementation instead.
fn eval_has_generic<S: EvalSemantics, V: DocumentValue>(
    key_expr: &Expr,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> Option<GenericResult<V>> {
    if crate::jq::input_queue_is_active() && crate::jq::walk::uses_input_builtins(key_expr) {
        return None;
    }
    if matches!(unwrap_paren(key_expr), Expr::Comma(_)) {
        return None;
    }
    let key_owned = match eval_single::<S, V>(key_expr, value.clone(), optional, cursor) {
        GenericResult::One(v) => to_owned(&v).ok()?,
        GenericResult::OneCursor(c) => to_owned_cursor(&c).ok()?,
        GenericResult::Owned(v) => v,
        _ => return None,
    };
    Some(eval_has_one_key::<S, V>(
        &value, cursor, key_owned, optional,
    ))
}

/// `has(key)`'s check for one already-resolved key -- the body of
/// [`eval_has_generic`]. Mirrors `eval::has_one_key`'s match order exactly
/// (null receiver, then object+string key, then array+numeric key, then
/// yq's permissive type-mismatch fallback, then `optional`, then error).
fn eval_has_one_key<S: EvalSemantics, V: DocumentValue>(
    value: &V,
    cursor: Option<V::Cursor>,
    key_owned: OwnedValue,
    optional: bool,
) -> GenericResult<V> {
    if value.is_null() {
        return GenericResult::Owned(OwnedValue::Bool(false));
    }
    match (&key_owned, value.as_object(), value.as_array()) {
        // #2261: `contains_checked`, not the bare `contains` -- catches a
        // trailing stray comma (`{"a":1,} | has("a")`), a #1194 unpaired
        // tail, and a #1677 delimiter fault, none of which `contains`
        // itself ever checked. See `contains_checked`'s own doc comment
        // for why this walks to completion (a real cost increase for the
        // "key found early" case) rather than riding an already-mandatory
        // walk for free like every other #2261 fix.
        (OwnedValue::String(key), Some(fields), _) => match fields.contains_checked(key) {
            Ok(found) => GenericResult::Owned(OwnedValue::Bool(found)),
            Err(err) => GenericResult::Error(err),
        },
        (
            OwnedValue::Int(_) | OwnedValue::Float(_) | OwnedValue::NumberLiteral(..),
            _,
            Some(elements),
        ) => {
            // #2261: `len_checked`, not the bare `len()` -- `elements.len()`
            // already needed the whole array's cursor walk (`in_bounds`
            // reads only the count, never a value), so the #1677/#2261 gap
            // checks ride along for free, same reasoning as
            // `Expr::Index`'s own array arm in `eval_single`.
            let len = match elements.len_checked() {
                Ok(len) => len,
                Err(err) => return GenericResult::Error(err),
            };
            let in_bounds = match numeric_key_to_array_index::<S>(&key_owned) {
                None => false,
                Some(idx) => index_in_array_bounds::<S>(idx, len as i64),
            };
            GenericResult::Owned(OwnedValue::Bool(in_bounds))
        }
        _ if has_type_mismatch_is_permissive::<S>() => {
            GenericResult::Owned(OwnedValue::Bool(false))
        }
        _ if optional => GenericResult::None,
        _ => GenericResult::Error(EvalError::cannot_check_has(
            tagged_type_name(value, cursor),
            key_owned.type_name(),
        )),
    }
}

/// jq's sort key for one element: `[f]`, the array of *every* output of the
/// key filter, not just its first (#155).
///
/// The generic twin of `eval::eval_array_construction`'s use inside
/// `builtin_min_by`/`sort_by`/`group_by`/`unique_by`. It deliberately does
/// *not* route through this file's own `Expr::Array` arm, which additionally
/// applies `yq_float_fidelity_fixup` -- that fixup exists to make a computed
/// float *print* the way real yq prints it, and running it here would let an
/// output-formatting rule change which element sorts first.
///
/// Atomic, matching `eval_array_construction`: any control from `f` -- error,
/// break, halt, or the trailing control of a `Partial` -- discards the prefix
/// and aborts the whole builtin, rather than keying the element on a partial
/// array.
fn sort_key_generic<S: EvalSemantics, V: DocumentValue>(
    f: &Expr,
    elem: &V::Cursor,
    optional: bool,
) -> Result<OwnedValue, Control> {
    let mut out: Vec<OwnedValue> = Vec::new();
    let result = eval_single::<S, V>(f, elem.value(), optional, Some(*elem));
    match push_generic_owned_values(result, &mut out) {
        Some(control) => Err(control),
        None => Ok(OwnedValue::Array(out)),
    }
}

/// Pair every element of `cursors` with its comparison key, keeping the
/// cursor itself untouched.
///
/// `key` is `None` for the bare `sort`/`unique`/`min`/`max` spellings, which
/// compare elements by their own decoded value; `Some(f)` for the `_by`
/// forms, which compare by [`sort_key_generic`]. Either way the *element*
/// stays a `V::Cursor` -- that is the whole point of #1687's fix for this
/// family, since only a cursor can still name a duplicate mapping key.
///
/// The key is an `OwnedValue` in both cases and unavoidably so: `compare_values`
/// has no cursor-domain equivalent, and a `_by` key is a computed value with no
/// document position at all. So a duplicate key *inside a comparison key* is
/// still collapsed -- exactly as it is in `eval.rs` today. Only the emitted
/// element is lossless.
///
/// **Both arms carry `eval.rs`'s #1755 rule**, by different means: the `None`
/// arm's own conversion is already the checked one, and the `Some(f)` arm
/// runs `push_generic_truthiness_cursor_error` -- the same validation with the
/// `OwnedValue` construction removed. #2069 filed this as an accepted gap on
/// the grounds that no live repro was known and the check would cost a full
/// decode per element; both premises were wrong. The repro is
/// `[{"a":2,"s":"\ud800"},{"a":1,"s":"ok"}] | sort_by(.a) | length`, which
/// answered 2 where `main` raised, and the validation-only walk costs no
/// materialization.
fn key_elements_generic<S: EvalSemantics, V: DocumentValue>(
    cursors: Vec<V::Cursor>,
    key: Option<&Expr>,
    optional: bool,
) -> Result<Vec<(OwnedValue, V::Cursor)>, Control> {
    let mut keyed = vec_with_capacity(cursors.len());
    for cursor in cursors {
        let k = match key {
            Some(f) => {
                // #1755's rule for the `_by` forms. `eval::builtin_sort_by`/
                // `unique_by`/`min_by`/`max_by` each `to_owned` (`eval.rs`'s
                // own checked conversion, renamed from `to_owned_checked` by
                // #1989) the *element* as well as computing its key, so an
                // undecodable element raises rather than silently sorting in
                // as `""`.
                // Omitting that here was a live regression, not a theoretical
                // one: on `[{"a":2,"s":"\ud800"},{"a":1,"s":"ok"}]`,
                // `sort_by(.a) | length` answered 2 where `main` raised --
                // the bad element never reaches the output, so nothing else
                // would ever have surfaced it.
                //
                // `push_generic_truthiness_cursor_error`, not
                // `to_owned_cursor`: it is that function's own traversal and
                // validation with the `OwnedValue` construction removed, so
                // the `_by` forms keep the point of this arm -- never
                // materializing an element they only ever reorder -- while
                // still raising on one that cannot be decoded. (Not free: it
                // still allocates one `String` per object key, per
                // `resolve_display_key`'s #1642 guard.)
                if let Some(control) = push_generic_truthiness_cursor_error(&cursor, 0) {
                    return Err(control);
                }
                sort_key_generic::<S, V>(f, &cursor, optional)?
            }
            // The bare forms compare elements by their own decoded value, so
            // the conversion below *is* the check -- `to_owned_cursor` is
            // already the checked one.
            None => to_owned_cursor(&cursor).map_err(Control::Error)?,
        };
        keyed.push((k, cursor));
    }
    Ok(keyed)
}

/// Whether the reordering builtins may keep this input's elements as
/// cursors, or must hand the whole thing to the DOM path instead.
///
/// See [`DocumentCursor::document_has_aliases`] for the full reasoning: an
/// alias is only sound while it still follows a declaration of the same name,
/// and reordering, selecting or dropping nodes can break that. The DOM path's
/// `enforce_anchor_soundness` is what normally prevents it, and the
/// cursor-streaming path cannot reach that pass (#1350). So an alias-bearing
/// document keeps exactly the behaviour it had before #1687 -- sound output,
/// at the cost of the duplicate keys this fix would otherwise have saved --
/// rather than gaining faithful marks it could not read back.
///
/// `cursor` is `None` when the array being reordered is itself a computed
/// value with no document position; there are no marks to get wrong then.
fn reordering_may_keep_cursors<V: DocumentValue>(cursor: Option<&V::Cursor>) -> bool {
    !cursor.is_some_and(DocumentCursor::document_has_aliases)
}

/// Turn a `Control` raised while keying elements back into a `GenericResult`.
///
/// Every builtin in this family is atomic -- `eval.rs`'s own arms return
/// bare `Error`/`Break`/`Halt` with no partial prefix -- so this never
/// produces a `Partial`.
fn sort_family_control<V: DocumentValue>(control: Control) -> GenericResult<V> {
    match control {
        Control::Error(e) => GenericResult::Error(e),
        Control::Break(label) => GenericResult::Break(label),
        Control::Halt(code) => GenericResult::Halt(code),
    }
}

/// The shared body of `sort`/`sort_by`/`unique`/`unique_by`/`min`/`min_by`/
/// `max`/`max_by`/`reverse` for the array case (#1687).
///
/// Only the array case: every one of those builtins gives a non-array input
/// its own mode-specific error wording (`object_pair_type_error`'s jq pairing
/// bug for `min_by`, `yq_only_arrays_supported_for` for `unique`,
/// `cannot_be_sorted` for `sort`, and `scalar_fallback`'s decode-failure
/// precedence in front of `optional` for all of them). Reproducing that here
/// would be a second copy of a decision tree #929/#995/#1755/#1901 have each
/// corrected once already, so the caller bridges a non-array input to
/// `eval.rs` verbatim instead. This function is reached only once the input
/// is known to be an array.
///
/// `reorder` receives the keyed elements and returns the elements the result
/// should contain, in order. Returning `GenericResult::LazySeq` over those
/// cursors -- rather than an `OwnedValue::Array` -- is what keeps a duplicate
/// mapping key inside a moved element alive.
fn sort_family_array_generic<S: EvalSemantics, V: DocumentValue>(
    cursors: Vec<V::Cursor>,
    key: Option<&Expr>,
    optional: bool,
    reorder: impl FnOnce(Vec<(OwnedValue, V::Cursor)>) -> Vec<V::Cursor>,
) -> GenericResult<V> {
    let keyed = match key_elements_generic::<S, V>(cursors, key, optional) {
        Ok(keyed) => keyed,
        Err(control) => return sort_family_control(control),
    };
    GenericResult::LazySeq(Box::new(LazySeq::from_cursors(reorder(keyed))))
}

/// `sort`/`sort_by`'s ordering step, shared with `unique`/`unique_by`, which
/// jq defines as a sort followed by a dedup.
///
/// `sort_by` (not `sort_unstable_by`) on purpose: jq's sort is stable, so two
/// elements with equal keys keep their input order. That is observable here
/// in a way it is not in `eval.rs` -- there the tied elements have already
/// been flattened to equal `OwnedValue`s, whereas the cursors this returns
/// still point at distinct document positions that can print differently
/// (two mappings with the same collapsed form but different duplicate keys).
fn sort_keyed_elements<V: DocumentValue>(keyed: &mut [(OwnedValue, V::Cursor)]) {
    keyed.sort_by(|(a, _), (b, _)| compare_values(a, b));
}

/// Whether `path(expr)` can be resolved by walking cursors instead of
/// materializing the whole document (#2061).
///
/// Deliberately narrow: only the pure-navigation shapes, whose emitted paths
/// depend on the document's *structure* along the path and never on a value
/// it has to compute. Anything else -- a computed index (`.[$k]`), a slice, a
/// builtin, `getpath`, `first`/`last`, a comparison -- defers to
/// `builtin_path_on_owned` unchanged, so this adds a fast path rather than a
/// second implementation of path resolution.
///
/// `Expr::Comma` is included: `path(.a, .b)` is just both walks, and the
/// walk below already fans out for `Iterate`.
fn path_expr_is_cursor_navigable(expr: &Expr) -> bool {
    match expr {
        Expr::Identity | Expr::Iterate => true,
        Expr::Field(_) => true,
        // A negative index needs no array length: `path(.[-1])` is `[-1]`
        // verbatim in both jq and succinctly today (verified live), so the
        // sign costs nothing here.
        Expr::Index { .. } => true,
        Expr::Paren(inner) | Expr::Optional(inner) => path_expr_is_cursor_navigable(inner),
        Expr::Pipe(exprs) | Expr::Comma(exprs) => exprs.iter().all(path_expr_is_cursor_navigable),
        _ => false,
    }
}

/// One node reached during a [`path_walk_generic`] step.
///
/// `Absent` is not an error: jq yields a path for a component that does not
/// exist (`{} | path(.a)` is `["a"]`) and keeps navigating as if the value
/// were `null` (`{} | path(.a.b)` is `["a","b"]`), so the walk has to carry
/// "no such node" as a first-class position rather than stopping.
enum PathNode<V: DocumentValue> {
    At(V::Cursor),
    Absent,
}

impl<V: DocumentValue> Clone for PathNode<V> {
    fn clone(&self) -> Self {
        match self {
            Self::At(c) => Self::At(*c),
            Self::Absent => Self::Absent,
        }
    }
}

/// The type name to report in an indexing error, for a node that may be
/// absent. An absent node is `null`, which both jq and succinctly allow every
/// navigation through.
fn path_node_type_name<V: DocumentValue>(node: &PathNode<V>) -> &'static str {
    match node {
        PathNode::At(c) => {
            let v = c.value();
            tagged_type_name(&v, Some(*c))
        }
        PathNode::Absent => "null",
    }
}

/// Walk `expr` from `node`, appending one `OwnedValue::Array` per emitted
/// path to `out` (#2061).
///
/// The whole point is that no `OwnedValue` tree is ever built for the
/// *document*: only the path components themselves are owned, and those are
/// bounded by the query's own depth and fan-out rather than the input's size.
///
/// Error texts are taken from the same constructors the materializing path
/// uses, so the two agree exactly -- verified live against every shape in
/// `test_path_cursor_native_matches_the_materializing_path_2061`.
///
/// #2147: this walk passes `S::COLLAPSE_DUPLICATE_KEYS` (the mode's own
/// rule) to the shared `path_step_generic`, where
/// [`path_context_step_generic`]'s own `key`/`path`/`parent` walk below
/// passes `true` unconditionally -- a deliberate divergence, not an
/// oversight. `path(f)` has no yq oracle at all (real yq rejects it as a
/// parse error; `path(f)` is a jq builtin succinctly exposes as an
/// extension there, so ADR-0018's divergence rule doesn't apply), where
/// `[.[] | key]` does (`{"a":1,"a":2} | [.[] | key]` is `["a"]` in real yq
/// v4.53.3, matching `path_context_step_generic`'s `true`). See that
/// function's own doc comment for the full reasoning, and
/// [docs/compliance/yq/limitations.md](../../docs/compliance/yq/limitations.md)'s
/// "Duplicate mapping keys" section for the oracle evidence.
fn path_walk_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    node: &PathNode<V>,
    path: &Rc<PathTrail>,
    out: &mut Vec<OwnedValue>,
) -> Result<(), EvalError> {
    // Panics past `MAX_NESTING_DEPTH` levels (code review on #2058), the
    // same guard `collect_paths_generic` already carries for its own
    // recursive path-array walk in this file. `PathTrail` has no depth cap
    // of its own -- a static chain (`path(.a.a.a...)`) recurses this
    // function's own call graph (`path_walk_pipe_generic`/`path_step_
    // generic`/`path_step_pipe_generic` below all funnel back through it)
    // once per component with no limit, and a million-component chain
    // overflowed the native stack and aborted the process outright
    // (confirmed live) once this PR's fix made `path()` fast enough to
    // reach that depth in practice -- pre-fix, the O(d^2) cost made the
    // same input time out long before ever getting there.
    assert_nesting_depth(path.depth());
    match expr {
        Expr::Identity => {
            out.push(OwnedValue::Array(path.to_vec()));
            Ok(())
        }
        Expr::Paren(inner) => path_walk_generic::<S, V>(inner, node, path, out),
        Expr::Optional(inner) => {
            // `?` swallows the error but *not* the outputs the branch had
            // already produced before it: `[1] | path((.[0],.a)?)` emits
            // `[0]` and then stops (jq 1.7.1). Discarding the whole branch
            // made that emit nothing, where the non-navigable fallback
            // (`path((.[0],.a[0:1])?)`, same shape) correctly emitted `[0]`.
            //
            // `path(.a?)` on an array still emits nothing, because there the
            // error comes before anything is produced -- which is the case
            // the discarded-branch version was checked against.
            let mut branch = Vec::new();
            let _ = path_walk_generic::<S, V>(inner, node, path, &mut branch);
            out.append(&mut branch);
            Ok(())
        }
        Expr::Comma(exprs) => {
            for e in exprs {
                path_walk_generic::<S, V>(e, node, path, out)?;
            }
            Ok(())
        }
        Expr::Pipe(exprs) => path_walk_pipe_generic::<S, V>(exprs, node, path, out),
        // A terminal navigation step: take it, then emit each resulting path.
        _ => {
            let mut heads = Vec::new();
            let stepped =
                path_step_generic::<S, V>(expr, node, path, S::COLLAPSE_DUPLICATE_KEYS, &mut heads);
            for (p, _) in heads {
                out.push(OwnedValue::Array(p.to_vec()));
            }
            stepped
        }
    }
}

/// [`path_walk_generic`]'s own `Expr::Pipe` case, split out to work on a
/// borrowed `&[Expr]` slice rather than a single `&Expr` (#2058).
///
/// Before this, the `Pipe` arm split one component off at a time and
/// rewrapped the remainder in a *new*, owned `Expr::Pipe(rest.to_vec())` just
/// to recurse — an O(remaining length) clone of `Expr` nodes at every one of
/// a `d`-element pipe's `d` stages, summing to O(d^2) for a flat
/// `.c.c.c...c[0]`-shaped chain (exactly the AST-clone-per-stage pattern
/// #1510 already fixed in `eval.rs`'s own path-context evaluator — see this
/// crate's top-level `CHANGELOG.md`). Passing `rest` straight through as a
/// slice is O(1): no clone, just a pointer/length pair. Combined with
/// [`PathTrail`] (an O(1)-extend `Rc`-list replacing the identical
/// `path.to_vec()`-then-push clone this arm's `path` parameter used to pay
/// every stage too), a `d`-deep static chain is now O(d) end to end here.
fn path_walk_pipe_generic<S: EvalSemantics, V: DocumentValue>(
    exprs: &[Expr],
    node: &PathNode<V>,
    path: &Rc<PathTrail>,
    out: &mut Vec<OwnedValue>,
) -> Result<(), EvalError> {
    // See `path_walk_generic`'s own doc comment -- this function recurses
    // into itself once per pipe stage without ever passing back through
    // that entry check, so it needs its own (#2058 code review).
    assert_nesting_depth(path.depth());
    let Some((first, rest)) = exprs.split_first() else {
        out.push(OwnedValue::Array(path.to_vec()));
        return Ok(());
    };
    if rest.is_empty() {
        return path_walk_generic::<S, V>(first, node, path, out);
    }
    // Each output of `first` is a distinct position to continue the rest of
    // the pipe from, so the step is re-walked rather than batched -- the
    // same per-output shape `Expr::Iterate` needs.
    let mut heads = Vec::new();
    let stepped =
        path_step_generic::<S, V>(first, node, path, S::COLLAPSE_DUPLICATE_KEYS, &mut heads);
    // Heads the step produced *before* failing are earlier in jq's generator
    // order than its own error, so they are walked first and only then is
    // the error propagated -- the same "never un-emit an output already
    // produced" rule the `Builtin::Path` arm applies at the top level.
    for (next_path, next_node) in heads {
        path_walk_pipe_generic::<S, V>(rest, &next_node, &next_path, out)?;
    }
    stepped
}

/// One navigation step: from `node` at `path`, produce every (path, node)
/// position the step reaches.
fn path_step_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    node: &PathNode<V>,
    path: &Rc<PathTrail>,
    collapse_duplicate_keys: bool,
    out: &mut Vec<(Rc<PathTrail>, PathNode<V>)>,
) -> Result<(), EvalError> {
    // See `path_walk_generic`'s own doc comment (#2058 code review).
    assert_nesting_depth(path.depth());
    match expr {
        Expr::Identity => {
            out.push((Rc::clone(path), node.clone()));
            Ok(())
        }
        Expr::Paren(inner) => {
            path_step_generic::<S, V>(inner, node, path, collapse_duplicate_keys, out)
        }
        Expr::Field(name) => {
            let next = match node {
                PathNode::Absent => PathNode::Absent,
                PathNode::At(c) => {
                    let v = c.value();
                    if v.is_null() {
                        PathNode::Absent
                    } else if let Some(fields) = v.as_object() {
                        match fields.find_cursor(name)? {
                            Some(fc) => PathNode::At(fc),
                            None => PathNode::Absent,
                        }
                    } else {
                        return Err(EvalError::cannot_index_with_field(
                            path_node_type_name::<V>(node),
                            name,
                        ));
                    }
                }
            };
            out.push((
                PathTrail::extend(path, OwnedValue::String(name.clone())),
                next,
            ));
            Ok(())
        }
        Expr::Index { idx, key } => {
            // The component is reported with its own source spelling
            // (`path(.[2.0])` is `[2.0]`, not `[2]` -- #1088), carried by
            // `key` when the literal had one. `index_component_value` is
            // `eval.rs`'s own renderer for exactly this, shared rather than
            // re-derived.
            //
            // #1401: `idx`/`key` bind straight from the arm. While this was
            // a *pair* of variants the arm could not bind them, so it
            // re-matched over both with an `unreachable!()` fallback -- the
            // same shape that let a missing arm slip through twice.
            let (idx, key) = (*idx, key.as_ref());
            let next = match node {
                PathNode::Absent => PathNode::Absent,
                PathNode::At(c) => {
                    let v = c.value();
                    if v.is_null() {
                        PathNode::Absent
                    } else if let Some(elements) = v.as_array() {
                        usize::try_from(idx)
                            .ok()
                            .and_then(|i| elements.get_cursor(i))
                            .map_or(PathNode::Absent, PathNode::At)
                    } else {
                        return Err(EvalError::cannot_index_with_type(
                            path_node_type_name::<V>(node),
                            "number",
                        ));
                    }
                }
            };
            out.push((
                PathTrail::extend(path, index_component_value(idx, key)),
                next,
            ));
            Ok(())
        }
        Expr::Iterate => match node {
            // `null` and a missing node are not iterable, matching
            // `path(.[])` on `null` raising rather than yielding nothing.
            PathNode::Absent => Err(EvalError::cannot_iterate_with(
                EvalTag::Jq,
                &OwnedValue::Null,
            )),
            PathNode::At(c) => {
                let v = c.value();
                if let Some(fields) = v.as_object() {
                    // `effective_fields_checked` with the mode's own
                    // duplicate-key rule, plus `key_display_string` -- the
                    // same two helpers `collect_paths_generic` uses, reused
                    // rather than re-derived. Walking `all_fields()` instead
                    // emitted `[["a"],["a"]]` for `[path(.[])]` on
                    // `{"a":1,"a":2}`, where jq mode collapses to `[["a"]]`
                    // (#1385); caught by the evaluator-parity suite.
                    for field in effective_fields_checked(&fields, collapse_duplicate_keys)? {
                        let Some(key) = key_display_string(&field.key) else {
                            return Err(fields.malformed_member_error());
                        };
                        out.push((
                            PathTrail::extend(path, OwnedValue::String(key.into_owned())),
                            PathNode::At(field.value_cursor),
                        ));
                    }
                    Ok(())
                } else if let Some(elements) = v.as_array() {
                    // #2261 (systematic sweep): `collect_cursors_checked`,
                    // not the unchecked `collect_cursors` this arm used --
                    // the object arm just above already routes through the
                    // checked `effective_fields_checked`; this array
                    // sibling had drifted onto the wrong one, so
                    // `[path(.[])]` on `[1,2,3,]` silently answered
                    // `[[0],[1],[2]]` instead of raising like every other
                    // `.[]` consumer already does.
                    for (i, ec) in elements.collect_cursors_checked()?.into_iter().enumerate() {
                        out.push((
                            PathTrail::extend(path, OwnedValue::Int(i as i64)),
                            PathNode::At(ec),
                        ));
                    }
                    Ok(())
                } else {
                    // `to_owned_cursor`, not `to_owned`: only the cursor
                    // resolves an explicit YAML tag (#747), and the
                    // materializing resolver quotes the *tagged* value back.
                    // With `to_owned` here, `a: !!str 5 | .[] | .[]` read
                    // "Cannot iterate over number (5)" where every other
                    // route says `string ("5")` -- the sibling
                    // `path_node_type_name` above already goes through the
                    // cursor for exactly this reason.
                    Err(EvalError::cannot_iterate_with(
                        EvalTag::Jq,
                        &to_owned_cursor(c)?,
                    ))
                }
            }
        },
        // `Paren` is transparent, so a parenthesised pipe/comma/optional
        // reaches this function as the *head* of an outer pipe:
        // `path(((.a|.b)|.c))` steps `Pipe([.a, .b])`, `path((.a,.b)|.c)`
        // steps `Comma([.a, .b])`, and `path((.a?)|.c)` steps
        // `Optional(.a)`. `path_expr_is_cursor_navigable` accepts all three,
        // so before these arms existed each one hit the `unreachable!` below
        // and aborted the process with exit 101 -- a live, CLI-reachable
        // panic, not a can't-happen.
        Expr::Pipe(exprs) => {
            path_step_pipe_generic::<S, V>(exprs, node, path, collapse_duplicate_keys, out)
        }
        Expr::Comma(exprs) => {
            for e in exprs {
                path_step_generic::<S, V>(e, node, path, collapse_duplicate_keys, out)?;
            }
            Ok(())
        }
        Expr::Optional(inner) => {
            // Same rule as `path_walk_generic`'s own `Optional` arm: the
            // error is swallowed, the positions already reached are not.
            let mut branch = Vec::new();
            let _ =
                path_step_generic::<S, V>(inner, node, path, collapse_duplicate_keys, &mut branch);
            out.append(&mut branch);
            Ok(())
        }
        // `path_expr_is_cursor_navigable` gates every caller, so nothing else
        // can arrive here.
        other => unreachable!("non-navigable path expression reached the cursor walk: {other:?}"),
    }
}

/// [`path_step_generic`]'s own `Expr::Pipe` case, split out the same way
/// [`path_walk_pipe_generic`] is (#2058): a nested nested pipe reached as one
/// step inside an *outer* pipe (`path(((.a|.b)|.c))`, see the doc comment
/// where this arm used to live) is rare, but recurses on a borrowed `&[Expr]`
/// slice here for the identical reason -- no owned `Expr::Pipe(rest.to_vec())`
/// rebuilt per stage, and `path`'s own O(1) `PathTrail::extend` instead of an
/// O(depth) `Vec` clone-and-push.
fn path_step_pipe_generic<S: EvalSemantics, V: DocumentValue>(
    exprs: &[Expr],
    node: &PathNode<V>,
    path: &Rc<PathTrail>,
    collapse_duplicate_keys: bool,
    out: &mut Vec<(Rc<PathTrail>, PathNode<V>)>,
) -> Result<(), EvalError> {
    // See `path_walk_generic`'s own doc comment -- like `path_walk_pipe_
    // generic`, this recurses into itself once per pipe stage (#2058 code
    // review).
    assert_nesting_depth(path.depth());
    let Some((first, rest)) = exprs.split_first() else {
        out.push((Rc::clone(path), node.clone()));
        return Ok(());
    };
    if rest.is_empty() {
        return path_step_generic::<S, V>(first, node, path, collapse_duplicate_keys, out);
    }
    let mut heads = Vec::new();
    let stepped = path_step_generic::<S, V>(first, node, path, collapse_duplicate_keys, &mut heads);
    // Positions completed through the whole chain land in `out` before
    // `stepped`'s own error surfaces, for the same generator-order reason as
    // `path_walk_pipe_generic`.
    for (p, n) in heads {
        path_step_pipe_generic::<S, V>(rest, &n, &p, collapse_duplicate_keys, out)?;
    }
    stepped
}

/// Whether a pipe stage carrying path context can be resolved by walking
/// cursors instead of materializing the whole document (#2061).
///
/// The sibling of [`path_expr_is_cursor_navigable`], for the *other* half of
/// #2061: `path` (no-arg), `key`, `parent` and `parent(n)` need the path
/// accumulated across a pipe rather than a path expression resolved in one
/// go, so `Expr::Pipe`'s bridge -- not `Builtin::Path`'s -- is what they
/// reach.
///
/// Two deliberate exclusions, both narrower than the `path()` predicate:
///
/// * `Expr::Optional`. `?` in path context is not the same rule as `?` in a
///   path expression -- `eval_pipe_with_path_context_internal` gives it three
///   separate arms, one of them a bracket carve-out shared with the plain
///   evaluator. Reproducing that here would be re-deriving semantics rather
///   than reusing them, so `?` defers.
/// * `parent(n)` with a computed `n`. `n` is evaluated against the *current*
///   value, which the walk would have to materialize to do -- exactly the
///   cost being removed. A literal covers the real uses.
///
/// `Expr::Array` is included so `[.[] | key]` -- whose outputs are one
/// bounded array, not one per element -- stays on the walk too.
fn path_context_is_cursor_walkable(expr: &Expr) -> bool {
    match expr {
        Expr::Identity | Expr::Iterate | Expr::Field(_) => true,
        Expr::Index { .. } => true,
        Expr::Paren(inner) | Expr::Array(inner) => path_context_is_cursor_walkable(inner),
        Expr::Pipe(exprs) | Expr::Comma(exprs) => exprs.iter().all(path_context_is_cursor_walkable),
        Expr::Builtin(Builtin::PathNoArg | Builtin::Key | Builtin::Parent) => true,
        Expr::Builtin(Builtin::ParentN(n)) => matches!(**n, Expr::Literal(_)),
        _ => false,
    }
}

/// Whether a walked expression can emit more than one *materialized* value.
///
/// Materializing an ancestor is a **backward** jump against the document-wide
/// `Cell<SequentialCursor>` in `AdvancePositions`/`CompactEndPositions`: it
/// falls into `get_random`, which resets the incremental scan to position
/// zero, so the next sequential call rescans from there. A fan-out that
/// materializes once per element pays that reset once per element, where the
/// bridge decodes the document forwards exactly once. Measured on YAML,
/// `[.[] | .k.x | parent] | length` was +23%/+27% (1 MB/6 MB, M4 Pro).
///
/// The decision has to be **static**, taken before the O(document) validity
/// gate runs. A dynamic "stop at the second materialization" cap was measured
/// and came out *worse* than no guard at all -- +29%/+38% -- because bailing
/// then pays the gate and a partial walk and *then* the whole bridge.
fn path_context_fans_out(expr: &Expr) -> bool {
    match expr {
        Expr::Iterate => true,
        Expr::Comma(exprs) => exprs.len() > 1 || exprs.iter().any(path_context_fans_out),
        Expr::Pipe(exprs) => exprs.iter().any(path_context_fans_out),
        Expr::Paren(inner) | Expr::Array(inner) => path_context_fans_out(inner),
        _ => false,
    }
}

/// Whether every branch of `expr` ends in a stage that answers from the path
/// itself (`key`/`path`) rather than by materializing the node it stands on.
///
/// A fan-out of these is free -- they emit a path component and decode
/// nothing -- which is what keeps `[.[] | key]` and `[.[] | .k | parent |
/// key]` on the fast path while `[.[] | .k | parent]` is handed back.
fn path_context_emits_paths_only(expr: &Expr) -> bool {
    match expr {
        Expr::Builtin(Builtin::PathNoArg | Builtin::Key) => true,
        Expr::Paren(inner) | Expr::Array(inner) => path_context_emits_paths_only(inner),
        Expr::Comma(exprs) => exprs.iter().all(path_context_emits_paths_only),
        // Only a pipe's last stage emits; the earlier ones just move.
        Expr::Pipe(exprs) => exprs.last().is_some_and(path_context_emits_paths_only),
        _ => false,
    }
}

/// A position reached during a path-context walk: the node, the path that
/// reached it, and the node at every proper prefix of that path.
///
/// `ancestors[i]` is the node at `path[..i]`, so `ancestors.len() ==
/// path.len()` and `parent(n)` is a truncation of both rather than a
/// re-navigation from the root -- which is what `resolve_ancestor_path` has
/// to do once the document is an `OwnedValue` tree.
///
/// Retaining a stack of cursors is cheap and already an established pattern
/// here: `V::Cursor` is `Copy` and 32 bytes, and `LazySource::Cursors` and
/// `GenericResult::ManyCursor` both hold arbitrary cursor vectors across
/// evaluation.
/// The absent case is deliberately *not* representable. A step that reaches
/// one is refused at the single boundary where it can arise (see
/// [`path_context_step_generic`]), so every position the walk holds is a real
/// document node, and neither `parent` nor value emission carries an
/// "absent" branch that could not be reached to be tested.
struct PathContextPos<V: DocumentValue> {
    node: V::Cursor,
    path: Vec<OwnedValue>,
    ancestors: Vec<V::Cursor>,
}

impl<V: DocumentValue> Clone for PathContextPos<V> {
    fn clone(&self) -> Self {
        Self {
            node: self.node,
            path: self.path.clone(),
            ancestors: self.ancestors.clone(),
        }
    }
}

/// Why a path-context cursor walk stopped.
enum PathContextAbort {
    /// A genuine evaluation error, worded by the same constructors the
    /// materializing evaluator uses.
    Error(EvalError),
    /// A shape this walk does not model. The caller discards whatever the
    /// walk produced and re-runs the unmodified pipe through the
    /// materializing bridge.
    ///
    /// Safe precisely because every shape the walk accepts is pure
    /// navigation -- no builtins that compute, no `stderr`, no user
    /// functions -- so re-running cannot repeat a side effect. That is the
    /// constraint that sank #2053's `getpath` prototype
    /// (`getpath(("a"|stderr))` printed `a` twice); it does not apply here.
    Unsupported,
}

/// Emit the value at `node`, which is what a path-context pipe yields once
/// its stages run out.
///
/// `to_owned_cursor` is the same helper the bridge's own
/// `to_owned_with_cursor` reaches, so the `OwnedValue` is identical by
/// construction -- same tag resolution (#747), same `canonicalize_numbers`
/// (#978), same duplicate-key collapse. The difference is only that it is
/// applied to the node actually being returned rather than to the whole
/// document.
///
/// An absent node is `null`, matching the bridge continuing a missing field
/// with `OwnedValue::Null`.
fn path_context_emit_value<V: DocumentValue>(
    pos: &PathContextPos<V>,
    out: &mut Vec<OwnedValue>,
) -> Result<(), PathContextAbort> {
    // Emitting the *root* means materializing the whole document, which is
    // exactly what the bridge already does -- and the walk would then pay
    // the validity gate on top of it, for a net loss. Measured on a 5.9 MB
    // array, `.[0] | parent | length` went 0.56s -> 0.67s before this guard.
    // So a `parent` that lands back at the root stays on the bridge; one
    // that lands on a proper subtree still skips the whole-document tree.
    if pos.path.is_empty() {
        return Err(PathContextAbort::Unsupported);
    }
    out.push(to_owned_cursor(&pos.node).map_err(PathContextAbort::Error)?);
    Ok(())
}

/// Hop `n` levels towards the root, by truncating the path and the ancestor
/// stack rather than re-navigating from the document root.
///
/// `resolve_ancestor_path` answers an **empty object** -- not the root, and
/// not the ancestor -- both when `n` overshoots the root and when the
/// ancestor path does not resolve. That synthetic value is not a document
/// node, so there is no cursor to continue from, and a following
/// `.b`/`.[0]`/`.[]` would behave differently from an absent one. Hand both
/// cases back to the bridge instead of modelling them.
fn path_context_hop<V: DocumentValue>(
    pos: &PathContextPos<V>,
    n: usize,
) -> Result<PathContextPos<V>, PathContextAbort> {
    let len = pos
        .path
        .len()
        .checked_sub(n)
        .ok_or(PathContextAbort::Unsupported)?;
    let node = if len == pos.path.len() {
        pos.node
    } else {
        pos.ancestors[len]
    };
    Ok(PathContextPos {
        node,
        path: pos.path[..len].to_vec(),
        ancestors: pos.ancestors[..len].to_vec(),
    })
}

/// One path-context pipe stage that leaves the walk at a document node:
/// a navigation step, or an ancestor hop.
///
/// Navigation delegates to [`path_step_generic`] rather than re-deriving it,
/// so the mode's duplicate-key rule (#1385), the float index spelling
/// (#1088) and every error message stay shared with the `path()` walk. Only
/// the four steps that extend the path by exactly one component are
/// delegated -- `Pipe`/`Comma` are composed here instead, because
/// `path_step_generic` would grow the path by an unknown amount and the
/// ancestor stack could not be kept in step with it.
fn path_context_step_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    pos: &PathContextPos<V>,
    out: &mut Vec<PathContextPos<V>>,
) -> Result<(), PathContextAbort> {
    match expr {
        Expr::Identity => {
            out.push(pos.clone());
            Ok(())
        }
        Expr::Paren(inner) => path_context_step_generic::<S, V>(inner, pos, out),
        Expr::Field(_) | Expr::Index { .. } | Expr::Iterate => {
            let mut heads = Vec::new();
            // `true`, not `S::COLLAPSE_DUPLICATE_KEYS`: the bridge this
            // replaces materializes the document into an
            // `OwnedValue::Object(IndexMap<String, _>)`, which collapses a
            // repeated key structurally in *both* modes, and real yq agrees
            // (`{"a":1,"a":2} | [.[] | key]` is `["a"]` in v4.53.3).
            // `path()`'s own walk ([`path_walk_generic`]) keeps the mode's
            // flag instead, because it is not replacing a materialization
            // and has no yq oracle to match in the first place (`path(f)`
            // is a jq-only extension there) -- a deliberate divergence
            // between the two walks, recorded as #2147, not an oversight;
            // see that function's doc comment for the full cross-reference.
            let stepped = path_step_generic::<S, V>(
                expr,
                &PathNode::At(pos.node),
                &PathTrail::from_slice(&pos.path),
                true,
                &mut heads,
            );
            for (path, node) in heads {
                // `path_step_generic` now hands back an `Rc<PathTrail>`
                // (#2058); this caller's own `path` stays a plain
                // `Vec<OwnedValue>` (see `PathTrail::from_slice`'s own doc
                // comment for why), so it flattens back out here.
                let path = path.to_vec();
                // An absent node still bails to the bridge, even though
                // #2213 fixed the bridge's own `key`/`path` answers to match
                // real yq (`{} | .a | key` is `"a"`, `[] | .[0] | key` is
                // `0`, both confirmed against v4.53.3) rather than the
                // stale `null`/raise this comment used to describe. `parent`
                // is why the walk still can't continue on its own: real yq
                // auto-vivifies the missing node into `parent`'s return
                // value (`{} | .a | parent` is `{"a":null}`), and this
                // walk's `PathContextPos::node` has nowhere to hold a
                // synthesized value -- only real document nodes. #2146
                // (defect 3, still open) is the design question that would
                // let this stop bailing for the cases that don't touch
                // `parent`.
                let PathNode::At(node) = node else {
                    return Err(PathContextAbort::Unsupported);
                };
                // All four steps append exactly one component, so the node
                // they were reached from is the new position's last
                // ancestor. Anything else means this function and
                // `path_step_generic` have drifted apart; bail rather than
                // record an ancestor stack that lies.
                if path.len() != pos.path.len() + 1 {
                    return Err(PathContextAbort::Unsupported);
                }
                let mut ancestors = pos.ancestors.clone();
                ancestors.push(pos.node);
                out.push(PathContextPos {
                    node,
                    path,
                    ancestors,
                });
            }
            stepped.map_err(PathContextAbort::Error)
        }
        Expr::Comma(exprs) => {
            for e in exprs {
                path_context_step_generic::<S, V>(e, pos, out)?;
            }
            Ok(())
        }
        Expr::Pipe(exprs) => match exprs.split_first() {
            None => {
                out.push(pos.clone());
                Ok(())
            }
            Some((first, [])) => path_context_step_generic::<S, V>(first, pos, out),
            Some((first, rest)) => {
                let mut heads = Vec::new();
                let stepped = path_context_step_generic::<S, V>(first, pos, &mut heads);
                for head in heads {
                    path_context_step_generic::<S, V>(&Expr::Pipe(rest.to_vec()), &head, out)?;
                }
                stepped
            }
        },
        // `parent`/`parent(n)` move the position without emitting anything,
        // so `.a.b | parent | key` and `parent | parent | path` never build
        // a value at all -- they are pure stack arithmetic.
        Expr::Builtin(Builtin::Parent) => {
            out.push(path_context_hop(pos, 1)?);
            Ok(())
        }
        Expr::Builtin(Builtin::ParentN(n_expr)) => {
            let Expr::Literal(lit) = &**n_expr else {
                return Err(PathContextAbort::Unsupported);
            };
            // `classify_parent_n` carries yq mode's negative-wraparound rule
            // and the `-0.5` hard error (#791/#1476), shared rather than
            // re-derived.
            let n = classify_parent_n::<S>(&literal_to_owned(lit), pos.path.len())
                .map_err(PathContextAbort::Error)?;
            out.push(path_context_hop(pos, n)?);
            Ok(())
        }
        _ => Err(PathContextAbort::Unsupported),
    }
}

/// Walk one path-context expression from `pos`, appending each emitted value
/// to `out` (#2061).
fn path_context_walk_generic<S: EvalSemantics, V: DocumentValue>(
    expr: &Expr,
    pos: &PathContextPos<V>,
    out: &mut Vec<OwnedValue>,
) -> Result<(), PathContextAbort> {
    match expr {
        Expr::Paren(inner) => path_context_walk_generic::<S, V>(inner, pos, out),
        Expr::Pipe(exprs) => path_context_walk_pipe::<S, V>(exprs, pos, out),
        Expr::Comma(exprs) => {
            for e in exprs {
                path_context_walk_generic::<S, V>(e, pos, out)?;
            }
            Ok(())
        }
        Expr::Array(inner) => {
            // An error inside the collection discards the array entirely --
            // `[path(.[]|.b)]` on a mixed object emits nothing and raises,
            // where the same body outside brackets emits its resolved prefix
            // first (jq 1.7.1).
            let mut items = Vec::new();
            path_context_walk_generic::<S, V>(inner, pos, &mut items)?;
            out.push(OwnedValue::Array(items));
            Ok(())
        }
        // The three stages that answer from the path itself. Each is bounded
        // by the query's own depth, never by the document's size -- which is
        // the whole point of #2061.
        Expr::Builtin(Builtin::PathNoArg) => {
            out.push(OwnedValue::Array(pos.path.clone()));
            Ok(())
        }
        Expr::Builtin(Builtin::Key) => {
            // At the root there is no key; the bridge answers `null` there.
            out.push(pos.path.last().cloned().unwrap_or(OwnedValue::Null));
            Ok(())
        }
        // Everything else is a step, after which the reached value is what
        // the pipe yields.
        _ => {
            let mut heads = Vec::new();
            let stepped = path_context_step_generic::<S, V>(expr, pos, &mut heads);
            for head in heads {
                path_context_emit_value(&head, out)?;
            }
            stepped
        }
    }
}

/// [`path_context_walk_generic`] over a pipe's stages, without cloning them
/// into an `Expr::Pipe` first.
fn path_context_walk_pipe<S: EvalSemantics, V: DocumentValue>(
    exprs: &[Expr],
    pos: &PathContextPos<V>,
    out: &mut Vec<OwnedValue>,
) -> Result<(), PathContextAbort> {
    match exprs.split_first() {
        None => path_context_emit_value(pos, out),
        Some((first, [])) => path_context_walk_generic::<S, V>(first, pos, out),
        Some((first, rest)) => {
            // `key` and `path` replace the position with a *computed* value,
            // so no document node is left for a later stage to navigate
            // from. The bridge continues such a pipe against the ambient
            // path; the walk does not model that, so hand it back.
            if matches!(first, Expr::Builtin(Builtin::Key | Builtin::PathNoArg)) {
                return Err(PathContextAbort::Unsupported);
            }
            let mut heads = Vec::new();
            let stepped = path_context_step_generic::<S, V>(first, pos, &mut heads);
            // Positions reached before a failing sibling are earlier in jq's
            // generator order than the failure, so they are walked first.
            for head in heads {
                path_context_walk_pipe::<S, V>(rest, &head, out)?;
            }
            stepped
        }
    }
}

/// #2061: answer a path-context pipe from cursors, or return `None` to leave
/// it to the materializing bridge.
///
/// `Expr::Pipe`'s bridge calls `to_owned_with_cursor` over the **whole
/// document** before the first stage runs -- plus a second O(document) scan
/// in `reindex_bridge_is_identity` -- because
/// `eval_pipe_with_path_context_internal` operates on an `OwnedValue` tree.
/// `.[0] | key` cost 519 MiB on a 20 MB array to answer `0`.
///
/// Two accepted shapes:
///
/// * the whole pipe is walkable; or
/// * stage 0 is walkable and no later stage needs path context, so the rest
///   folds through the ordinary evaluator. This is what covers
///   `[.[] | key] | length`, whose stage 0 is an `Expr::Array`.
///
/// A *navigable prefix* is deliberately never split off the front: the path
/// accumulates across stages, so evaluating `.a` separately would make
/// `.a | (.b | path)` report `["b"]` instead of `["a","b"]`.
fn try_path_context_cursor_walk<S: EvalSemantics, V: DocumentValue>(
    exprs: &[Expr],
    root: V::Cursor,
) -> Option<GenericResult<V>> {
    let first = exprs.first()?;
    let (walked, rest): (&[Expr], &[Expr]) = if exprs.iter().all(path_context_is_cursor_walkable) {
        (exprs, &[])
    } else if path_context_is_cursor_walkable(first) && !exprs[1..].iter().any(needs_path_context) {
        (&exprs[..1], &exprs[1..])
    } else {
        return None;
    };

    // A fan-out that materializes a node per branch loses to the bridge on
    // YAML, and that has to be decided here -- before the gate below -- not
    // mid-walk. A fan-out of `key`/`path` is free: they materialize nothing.
    // Only a pipe's *last* stage emits; the earlier ones just move the
    // position. Testing every stage instead rejected `(.a, .b) | key`,
    // because stage 0's own branches are `Field`s -- a lost optimization the
    // suite could not see, because falling back produces identical output.
    // Patch coverage caught it: the arms that shape reaches stayed at zero
    // hits after a test was added for it.
    if walked.iter().any(path_context_fans_out)
        && !walked.last().is_some_and(path_context_emits_paths_only)
    {
        return None;
    }

    // The whole-document walk is not skipped, only the tree build:
    // `to_owned_with_cursor` doubles as a validity gate (#1755/#1953), so
    // dropping it would make these pipes start accepting documents they
    // reject today. `push_generic_truthiness_cursor_error` is that same
    // traversal and validation with the `OwnedValue` construction removed --
    // the same gate `Builtin::Path` runs, for the same reason.
    if let Some(control) = push_generic_truthiness_cursor_error(&root, 0) {
        return Some(match control {
            Control::Error(e) => GenericResult::Error(e),
            Control::Break(label) => GenericResult::Break(label),
            Control::Halt(code) => GenericResult::Halt(code),
        });
    }

    let root_pos = PathContextPos {
        node: root,
        path: Vec::new(),
        ancestors: Vec::new(),
    };
    let mut out = Vec::new();
    let walked_result = path_context_walk_pipe::<S, V>(walked, &root_pos, &mut out);

    // #953/#1909's hazard, and the second constraint #2061's own body names.
    // When the materialized document is *not* a reindex-bridge identity, the
    // bridge does not merely evaluate -- it re-serializes through
    // `to_json_for_reindex`, which re-spells a bare `Float` per mode, and
    // every later stage sees that respelling. A YAML
    // `10000000000000000000.0` is exactly such a float (past what
    // `is_preservable_float_literal` keeps), so `.outer.big | parent | .big
    // | tostring` answers with the document's spelling rather than `1e+19`.
    //
    // The walk emits values straight from the cursor and never crosses that
    // bridge, so anything it produces that the bridge would have re-spelled
    // has to go back. Checking the *emitted* values rather than the whole
    // document keeps `key`/`path` -- whose outputs are path components --
    // fast on a document that contains such a float somewhere else.
    if !out.iter().all(reindex_bridge_is_identity) {
        return None;
    }

    match walked_result {
        Err(PathContextAbort::Unsupported) => None,
        // Whatever resolved before the failure still stands: jq's generator
        // never un-emits an output it already produced.
        Err(PathContextAbort::Error(e)) => Some(partial_generic(out, Control::Error(e))),
        Ok(()) => {
            let resolved = owned_vec_to_generic_result::<V>(out);
            Some(if rest.is_empty() {
                resolved
            } else {
                // `false`, not the caller's `optional`, for the same reason
                // the bridge this replaces passes `false`: that entry point
                // restarts every evaluation at `false` regardless of what
                // its caller passed, so `false` is what it actually
                // delivered.
                fold_pipe_stages::<S, V>(resolved, rest, false)
            })
        }
    }
}

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

        Builtin::Anchor => {
            // `c.anchor()`'s `&str` borrows from `c`, a value local to this
            // closure — must own it via `to_string` before it escapes.
            let anchor = cursor
                .and_then(|c| c.anchor().map(str::to_string))
                .unwrap_or_default();
            GenericResult::Owned(OwnedValue::String(anchor))
        }

        Builtin::Style => {
            let style = cursor.map_or("", |c| c.style());
            GenericResult::Owned(OwnedValue::String(style.to_string()))
        }

        Builtin::LineComment => match cursor.map(|c| c.line_comment_checked()) {
            Some(Err(_)) => GenericResult::Error(EvalError::invalid_utf8_in_comment()),
            Some(Ok(comment)) => {
                GenericResult::Owned(OwnedValue::String(comment.unwrap_or_default()))
            }
            None => GenericResult::Owned(OwnedValue::String(String::new())),
        },

        Builtin::Select(cond) => {
            // Evaluate condition with cursor context preserved.
            // This is critical for select(di == N) to work correctly.
            let cond_result = eval_single::<S, _>(cond, value.clone(), false, cursor);

            let mut bits = Vec::new();
            let cond_control = push_generic_truthiness(cond_result, &mut bits);
            // Under `S::SELECT_EMITS_ONCE_IF_ANY_TRUTHY` (yq, #1613), the
            // republish count collapses to at most one whenever *any* bit is
            // truthy, rather than one per truthy bit — `cond` is still
            // walked to completion either way, so a later error/break in
            // `cond` still escapes via `cond_control` below with a correctly
            // truncated `Partial` prefix.
            let truthy_count = if S::SELECT_EMITS_ONCE_IF_ANY_TRUTHY {
                usize::from(bits.iter().any(|&b| b))
            } else {
                bits.iter().filter(|&&b| b).count()
            };

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
                // No `optional`-guarded arm here: `eval_builtin`'s own
                // `optional` (as opposed to `cond`'s, which is hardcoded
                // `false` above regardless) is never `true` for
                // `Builtin::Select` after #693 — `select(...)?` isn't
                // `IndexExpr`/`SliceExpr`, so it goes through the generic
                // `Expr::Optional`/`eval_try`-style catch below at the
                // *ambient* `optional`, not a forced one. That catch takes
                // the `Partial` this arm below still constructs and keeps
                // only its prefix — cursor-preserving `pass_n(truthy_count)`
                // would have been the more precise answer (line/column
                // survive on the already-produced outputs), but nothing can
                // observe the difference: reinstate it if some future
                // dispatch path starts forcing `optional = true` here.
                Some(control) => {
                    let prefix: Vec<OwnedValue> = match cursor {
                        Some(c) => owned_or_err!(core::iter::repeat_with(|| to_owned_cursor(&c))
                            .take(truthy_count)
                            .collect::<Result<Vec<_>, _>>()),
                        None => owned_or_err!(core::iter::repeat_with(|| to_owned(&value))
                            .take(truthy_count)
                            .collect::<Result<Vec<_>, _>>()),
                    };
                    partial_generic(prefix, control)
                }
            }
        }

        // Slice 2 (#725): `Builtin::Map`'s first-ever native arm -- plain
        // `arr | map(f)` / `obj | map(f)` on containers that never touch
        // `keys_unsorted`/`keys`, the dominant 75-95% share of the
        // to_owned->reserialize->reindex->re-evaluate fallback's measured
        // cost (#686). `map(f)` is `[.[] | f]`; `.[]` over an object
        // iterates its *values* (#422), matching `eval.rs`'s
        // `builtin_map`/`map_over`. `Builtin::MapValues` is an explicit
        // non-goal and stays on the wildcard fallback below.
        Builtin::Map(f) => {
            if let Some(elements) = value.as_array() {
                GenericResult::LazySeq(Box::new(
                    LazySeq::new(LazySource::Elements(elements)).push_map(f, S::TAG),
                ))
            } else if let Some(fields) = value.as_object() {
                // `.[]` collapses a repeated key to its first position but
                // last-seen value in both modes (#1398), which needs every
                // occurrence seen before any value can be emitted --
                // incompatible with `Values`'s incremental cons-list pull.
                // `collapsed_fields` gates the materializing fallback
                // behind a cheap fingerprint probe (`document::census`),
                // so the duplicate-free case (by far the common one) keeps
                // #724/#725's lazy pull unchanged.
                let source = match collapsed_fields(&fields) {
                    Some(collapsed) => LazySource::cursors(
                        collapsed
                            .into_iter()
                            .map(|field| field.value_cursor)
                            .collect(),
                    ),
                    None => LazySource::Values(fields),
                };
                GenericResult::LazySeq(Box::new(LazySeq::new(source).push_map(f, S::TAG)))
            } else if S::TAG == EvalTag::Yq && value.is_null() {
                // #1907: real yq no-ops a scalar target for `map` entirely
                // -- `f` never even runs (confirmed live, v4.53.3: `5 |
                // map(error("boom"))` succeeds silently, `5`) -- unlike
                // jq, which always errors here. `null` gets yq's usual
                // empty-container treatment instead of a literal
                // passthrough (matches the `*=` merge rule documented in
                // CLAUDE.md: "null acts as an empty container on either
                // side of a yq-mode merge").
                GenericResult::Owned(OwnedValue::Array(Vec::new()))
            } else if S::TAG == EvalTag::Yq {
                // Every other scalar passes through byte-for-byte
                // unchanged -- no decode-check needed, nothing ever reads
                // its content (the "uniform fix regressed content-
                // independent ops" lesson, #1820's own review).
                GenericResult::One(value)
            } else {
                decode_failure_or(&value, optional, || {
                    GenericResult::Error(EvalError::cannot_iterate_with(
                        S::TAG,
                        &to_owned_for_diagnostic(&value, cursor),
                    ))
                })
            }
        }

        Builtin::Shuffle => {
            #[cfg(feature = "cli")]
            {
                use rand::seq::SliceRandom;
                use rand::SeedableRng;
                use rand_chacha::ChaCha8Rng;

                if let Some(elements) = value.as_array() {
                    // #2261 (systematic sweep): `collect_cursors_checked`,
                    // not the unchecked `collect_cursors` this arm used --
                    // this walk already materializes every element via
                    // `to_owned_all_cursors` regardless, so the #1677/#2261
                    // gap checks ride along for free.
                    let cursors = owned_or_err!(elements.collect_cursors_checked());
                    let mut values: Vec<OwnedValue> = owned_or_err!(to_owned_all_cursors(&cursors));
                    let mut rng = ChaCha8Rng::from_rng(&mut rand::rng());
                    values.shuffle(&mut rng);
                    GenericResult::Owned(OwnedValue::Array(values))
                } else {
                    GenericResult::Error(EvalError::new(format!(
                        "shuffle requires array, got {}",
                        tagged_type_name(&value, cursor)
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
                // #2261 (systematic sweep): same `collect_cursors_checked`
                // fix as `Shuffle` just above -- free here too, for the
                // same reason.
                let cursors = owned_or_err!(elements.collect_cursors_checked());
                let items: Vec<OwnedValue> = owned_or_err!(to_owned_all_cursors(&cursors));
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

                    let mut result = vec_with_capacity(max_len);
                    for col_idx in 0..max_len {
                        let mut column = vec_with_capacity(items.len());
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
                        let mut values = vec_with_capacity(items.len());
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
                GenericResult::Error(EvalError::type_error(
                    "array",
                    tagged_type_name(&value, cursor),
                ))
            }
        }

        Builtin::SplitDoc => {
            // split_doc is identity - the output formatting (--- separators)
            // is handled by the yq runner, not here. Forward the cursor when
            // available so duplicate keys survive, same as Values/Iterables/
            // Scalars/Identity above.
            cursor.map_or(GenericResult::One(value), GenericResult::OneCursor)
        }

        Builtin::Type => {
            let type_name = tagged_type_name(&value, cursor);
            GenericResult::Owned(OwnedValue::String(type_name.to_string()))
        }

        Builtin::Length => {
            if value.is_null() {
                GenericResult::Owned(OwnedValue::Int(0))
            } else if let Some(s) = value.as_str() {
                GenericResult::Owned(OwnedValue::Int(s.chars().count() as i64))
            } else if let Some(elements) = value.as_array() {
                // #2261: `_checked` refuses a trailing stray comma after a
                // real last element (`[1,]`) -- the array counterpart of
                // `effective_len_checked` just below, walking with
                // `uncons_cursor` (needed for the check's own position,
                // `len`'s plain `uncons` doesn't carry one) but keeping no
                // `Vec`, same reasoning as `collect_cursors_checked`'s own
                // O(1)-space sibling `len_checked`.
                match elements.len_checked() {
                    Ok(len) => GenericResult::Owned(OwnedValue::Int(len as i64)),
                    Err(err) => GenericResult::Error(err),
                }
            } else if let Some(fields) = value.as_object() {
                // #1194: `effective_len` counts a member whose key never
                // stringifies (`census`'s `unkeyed`), so `length` answered 1
                // for `{invalid: 1}` while `keys` listed none. Refuse rather
                // than pick one of two wrong numbers.
                //
                // `_checked` rather than a guard in front of the call: the
                // check rides the census walk this already makes, and it is
                // shared with the `keys | length` spelling in
                // `fold_lazy_keys_stage`, which reaches `effective_len` by a
                // route no guard here can see.
                //
                // Distinct keys in jq mode (#1385) -- `{"a":1,"a":2}|length`
                // is 1, because the object jq built only ever had one member.
                match effective_len_checked(&fields, S::COLLAPSE_DUPLICATE_KEYS) {
                    Ok(len) => GenericResult::Owned(OwnedValue::Int(len as i64)),
                    Err(err) => GenericResult::Error(err),
                }
            } else if let Some(i) = value.as_i64() {
                // checked_abs: i64::MIN has no i64 absolute value; use f64
                GenericResult::Owned(match i.checked_abs() {
                    Some(a) => OwnedValue::Int(a),
                    None => OwnedValue::Float(-(i as f64)),
                })
            } else if let Some(f) = value.as_f64() {
                GenericResult::Owned(OwnedValue::Float(f.abs()))
            } else {
                decode_failure_or(&value, optional, || {
                    GenericResult::Error(EvalError::has_no_length(&to_owned_for_diagnostic(
                        &value, cursor,
                    )))
                })
            }
        }

        Builtin::Keys => {
            if let Some(fields) = value.as_object() {
                // Stay lazy here too (#683) — `length` can answer from
                // `fields.len()` without decoding or sorting a single key.
                // `.[]`/`.[n]`/`first`/`last`/bare output still need the
                // full decode+sort (see the `Pipe` dispatch's `if !sorted`
                // guards), so they fall back to materializing exactly as
                // eager `Keys` did before.
                GenericResult::LazyKeys {
                    fields,
                    sorted: true,
                    collapse: S::COLLAPSE_DUPLICATE_KEYS,
                }
            } else if let Some(elements) = value.as_array() {
                // `[0, 1, ..., len-1]` is already sorted, so `Keys` needs no
                // extra `.sort()` here, unlike the object branch above. Stay
                // lazy (#684) — don't materialize a `Vec<OwnedValue::Int>`
                // yet; `length`, `.[]`, `.[n]`, `first`, and `last` can all
                // answer directly from `len` (see the `Pipe` dispatch below).
                //
                // #2261: `len_checked`, not the bare `len()` -- this call
                // already walks the whole array to answer `len` (same as
                // `Builtin::Length`'s own array arm), so the #1677/#2261
                // gap checks ride along for free, and `[1,2,3,] | keys`
                // now agrees with real jq (parse error) instead of
                // silently returning `[0,1,2]`.
                match elements.len_checked() {
                    Ok(len) => GenericResult::LazyIndexRange(len),
                    Err(err) => GenericResult::Error(err),
                }
            } else {
                decode_failure_or(&value, optional, || {
                    GenericResult::Error(EvalError::has_no_keys(&to_owned_for_diagnostic(
                        &value, cursor,
                    )))
                })
            }
        }

        Builtin::KeysUnsorted => {
            if let Some(fields) = value.as_object() {
                // Stay lazy here — don't decode every key into an owned
                // String yet. `length`, `.[]`, `.[n]`, `first`, and `last`
                // can all answer directly from `fields` (see the `Pipe`
                // dispatch below); anything else falls back to materializing
                // exactly as before (#140).
                GenericResult::LazyKeys {
                    fields,
                    sorted: false,
                    collapse: S::COLLAPSE_DUPLICATE_KEYS,
                }
            } else if let Some(elements) = value.as_array() {
                // Same laziness as the array branch of `Keys` above (#684),
                // and the same #2261 fix: `len_checked` rides the same
                // already-mandatory walk.
                match elements.len_checked() {
                    Ok(len) => GenericResult::LazyIndexRange(len),
                    Err(err) => GenericResult::Error(err),
                }
            } else {
                decode_failure_or(&value, optional, || {
                    GenericResult::Error(EvalError::has_no_keys(&to_owned_for_diagnostic(
                        &value, cursor,
                    )))
                })
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
        // map, so YAML duplicates can't collapse. `effective_fields` (#1170)
        // applies the evaluation *mode*'s duplicate-key rule during this
        // same direct walk: yq keeps every occurrence (`#443`'s point,
        // above); jq collapses a repeated key to its first position but
        // last-seen value (`{"a":1,"a":2}|to_entries` is one entry, value
        // `2`) -- previously this arm showed jq's raw, undeduplicated token
        // occurrences instead. The gate was the input *format* until #1385;
        // ADR-0018 rule 2 moved it to the mode, which is the axis the
        // reference tools actually decide on -- real yq preserves a
        // repeated key for a JSON-sourced document too.
        Builtin::ToEntries => {
            if let Some(elements) = value.as_array() {
                // A loop, not `.map(...).collect()`: `owned_or_err!` has to
                // return from *this* function, which it cannot do from inside
                // a closure (#1247).
                //
                // `_checked`: same #1677 gap check as `.[]`'s array arm --
                // this walk visits every element regardless, so it's free
                // to ride here too.
                let cursors = owned_or_err!(elements.collect_cursors_checked());
                let mut entries: Vec<OwnedValue> = Vec::new();
                for (i, elem_cursor) in cursors.into_iter().enumerate() {
                    let mut entry = IndexMap::new();
                    entry.insert("key".to_string(), OwnedValue::Int(i as i64));
                    entry.insert(
                        "value".to_string(),
                        owned_or_err!(to_owned_cursor(&elem_cursor)),
                    );
                    entries.push(OwnedValue::Object(entry));
                }
                GenericResult::Owned(OwnedValue::Array(entries))
            } else if let Some(fields) = value.as_object() {
                // Up front, because `effective_fields` reports an unpaired
                // trailing child as plain exhaustion -- the loop below would
                // simply see one member fewer and never notice (#1194). The
                // key-only walk is negligible here next to materializing
                // every value, which this builtin does anyway.
                if let Some(err) = malformed_object_member(&fields) {
                    return GenericResult::Error(err);
                }
                // A loop for the same reason as the array arm above.
                let mut entries: Vec<OwnedValue> = Vec::new();
                // #2261: the *raw, pre-collapse* walk's own textually last
                // field's value cursor -- not derivable from the collapsed
                // list this loop iterates below, whose own last element can
                // be a different, earlier-in-source field once a repeated
                // key collapses (see `effective_fields_with_raw_last`'s own
                // doc comment for why, and the well-formed case this would
                // otherwise wrongly reject).
                let (effective, last_value_cursor) =
                    effective_fields_with_raw_last(&fields, S::COLLAPSE_DUPLICATE_KEYS);
                for field in effective {
                    // A key that will not *decode* is preserved via its raw
                    // source span rather than raised on (#1247/#1642),
                    // matching `length`/`keys_unsorted`/`.`. `continue` here
                    // was #1194's third drop site: an entry vanished from
                    // the array while `length` still counted the member it
                    // came from.
                    //
                    // The `else` is unreachable as it stands --
                    // `malformed_object_member` above has already proved
                    // every key stringifies -- but `key_display_string` is
                    // an `Option` and something has to be written here.
                    // Report the same cause rather than invent a second one,
                    // so that if the pre-check is ever moved this arm is
                    // still right rather than merely quiet.
                    let Some(key) = key_display_string(&field.key) else {
                        return GenericResult::Error(fields.malformed_member_error());
                    };
                    // #1677: `malformed_object_member` above only checked
                    // the comma before each key; this loop already resolves
                    // every field's value regardless (`to_owned_cursor`
                    // below), so the colon check rides along for free.
                    if !value_delimiter_ok::<V::Fields>(Some(&field.value), &field.value_cursor) {
                        return GenericResult::Error(fields.malformed_member_error());
                    }
                    let mut entry = IndexMap::new();
                    entry.insert("key".to_string(), OwnedValue::String(key.into_owned()));
                    entry.insert(
                        "value".to_string(),
                        owned_or_err!(to_owned_cursor(&field.value_cursor)),
                    );
                    entries.push(OwnedValue::Object(entry));
                }
                // #2261: trailing stray comma after a real last field
                // (`{"a":1,}`).
                if let Some(last) = last_value_cursor {
                    if !trailing_element_gap_ok(&last, b'}') {
                        return GenericResult::Error(fields.malformed_member_error());
                    }
                }
                GenericResult::Owned(OwnedValue::Array(entries))
            } else {
                // `optional: false` unconditionally: `Builtin::ToEntries`
                // isn't `IndexExpr`/`SliceExpr`, so #693's dispatch never
                // forces `optional = true` into this native arm --
                // `to_entries?` evaluates it at the ambient `optional`
                // (normally `false`) and lets the outer
                // `Expr::Optional`/`eval_try`-style catch convert the
                // resulting `Error` to `None` once instead.
                decode_failure_or(&value, false, || {
                    GenericResult::Error(EvalError::has_no_keys(&to_owned_for_diagnostic(
                        &value, cursor,
                    )))
                })
            }
        }

        // Handled natively for the same reason as `ToEntries` above (#868):
        // the fallback's `to_owned_with_cursor` merges duplicate YAML
        // mapping keys before `collect_paths`/`collect_leaf_paths` ever walk
        // the tree, so a repeated key only ever contributes one path there.
        // `collect_paths_generic` walks `value` directly, applying each
        // format's own `effective_fields` rule at every nesting level, not
        // just the root.
        Builtin::Paths => {
            let mut paths = Vec::new();
            match collect_paths_generic::<S, _>(&value, &mut Vec::new(), &mut paths, false) {
                Ok(()) => collapse_vec(
                    paths,
                    || GenericResult::None,
                    GenericResult::Owned,
                    GenericResult::ManyOwned,
                ),
                Err(e) => GenericResult::Error(e),
            }
        }

        Builtin::LeafPaths => {
            let mut paths = Vec::new();
            match collect_paths_generic::<S, _>(&value, &mut Vec::new(), &mut paths, true) {
                Ok(()) => collapse_vec(
                    paths,
                    || GenericResult::None,
                    GenericResult::Owned,
                    GenericResult::ManyOwned,
                ),
                Err(e) => GenericResult::Error(e),
            }
        }

        // The `is*` family reads through `tagged_type_name` rather than
        // `DocumentValue::is_null`/`is_bool`/`is_number`/`is_string`
        // directly: those default to shape-only checks with no tag lookup
        // (same gap `Type` had before #747), and — for YAML specifically —
        // `is_number`/`is_string` can both independently answer `true` for
        // an untagged plain scalar (`as_str()` always succeeds on a
        // `YamlValue::String` node, whatever its resolved type), which
        // `type_name()`'s single-answer match doesn't have.
        Builtin::IsNull => {
            GenericResult::Owned(OwnedValue::Bool(tagged_type_name(&value, cursor) == "null"))
        }

        Builtin::IsBoolean => GenericResult::Owned(OwnedValue::Bool(
            tagged_type_name(&value, cursor) == "boolean",
        )),

        Builtin::IsNumber => GenericResult::Owned(OwnedValue::Bool(
            tagged_type_name(&value, cursor) == "number",
        )),

        Builtin::IsString => GenericResult::Owned(OwnedValue::Bool(
            tagged_type_name(&value, cursor) == "string",
        )),

        Builtin::IsArray => GenericResult::Owned(OwnedValue::Bool(
            tagged_type_name(&value, cursor) == "array",
        )),

        Builtin::IsObject => GenericResult::Owned(OwnedValue::Bool(
            tagged_type_name(&value, cursor) == "object",
        )),

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
                match elements.get_cursor(0) {
                    Some(c) => GenericResult::OneCursor(c),
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if value.is_null() {
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    tagged_type_name(&value, cursor),
                    "number",
                ))
            }
        }

        Builtin::Last => {
            // jq: last == .[-1], so [] and null both yield null
            if let Some(elements) = value.as_array() {
                // #2261: `len_checked`, not the bare `len()` -- unlike
                // `Builtin::First` just above (genuinely O(1) via
                // `get_cursor(0)`, left unchecked per #1629's precedent),
                // `last` already has to walk the whole array to find its
                // own length, so the #1677/#2261 gap checks ride along for
                // free on that same mandatory walk.
                let len = match elements.len_checked() {
                    Ok(len) => len,
                    Err(err) => return GenericResult::Error(err),
                };
                match len.checked_sub(1).and_then(|i| elements.get_cursor(i)) {
                    Some(c) => GenericResult::OneCursor(c),
                    None => GenericResult::Owned(OwnedValue::Null),
                }
            } else if value.is_null() {
                GenericResult::Owned(OwnedValue::Null)
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    tagged_type_name(&value, cursor),
                    "number",
                ))
            }
        }

        // `first(f)`/`last(f)`: never produced by the parser (#1986) --
        // `parse_first_expr`/`parse_last_expr` intercept this syntax before
        // `try_parse_builtin` ever runs, always producing `Expr::FirstExpr`/
        // `LastExpr` instead (see their arms in `eval_single`). Kept as
        // defensive symmetry, cursor-preserving for the same reason (#607).
        Builtin::FirstStream(inner) => {
            eval_first_or_last_generic::<S, _>(inner, value, optional, cursor, false)
        }
        Builtin::LastStream(inner) => {
            eval_first_or_last_generic::<S, _>(inner, value, optional, cursor, true)
        }

        // `nth(n; expr)` (arity 2) parses straight to this `Builtin`
        // spelling, not `Expr::NthExpr` -- unlike `first`/`last`/`limit`,
        // whose parser rules build the dedicated `Expr` variant directly
        // (confirmed in `parser.rs`: `nth`'s two-arg form returns
        // `Builtin::NthStream`, no `Expr::NthExpr` construction site exists
        // there at all). So this is the arm real `nth(n; expr)` calls
        // actually reach, not `eval_single`'s `Expr::NthExpr` arm -- same
        // #607/#1607 duplicate-key reasoning applies here, reusing
        // `eval_nth_generic` rather than duplicating its logic.
        Builtin::NthStream(n, expr) => {
            match eval_nth_generic::<S, _>(n, expr, value.clone(), optional, cursor) {
                Some(result) => result,
                None => bridge_to_full_evaluator::<S, _>(
                    &Expr::Builtin(Builtin::NthStream(n.clone(), expr.clone())),
                    value,
                    cursor,
                    optional,
                ),
            }
        }

        // #1687: `reverse` already held the element cursors in hand
        // (`collect_cursors()`) and then threw them away, decoding each into
        // an `IndexMap`-backed `OwnedValue` purely to hand back an
        // `OwnedValue::Array`. Every duplicate mapping key inside a reversed
        // element died there, even though reversing moves elements without
        // computing a single new value. Returning the cursors as a `LazySeq`
        // instead keeps them alive -- real yq preserves them (v4.53.3,
        // `reverse | .[0]` on a duplicate-keyed mapping element).
        Builtin::Reverse => {
            if !reordering_may_keep_cursors::<V>(cursor.as_ref()) {
                return bridge_to_full_evaluator::<S, _>(
                    &Expr::Builtin(builtin.clone()),
                    value,
                    cursor,
                    optional,
                );
            }
            if let Some(elements) = value.as_array() {
                // #2261 (systematic sweep): `collect_cursors_checked`, not
                // the unchecked `collect_cursors` this arm used -- this
                // walk already resolves every element's cursor to reverse
                // them, so the #1677/#2261 gap checks ride along for free.
                let mut cursors = owned_or_err!(elements.collect_cursors_checked());
                cursors.reverse();
                GenericResult::LazySeq(Box::new(LazySeq::from_cursors(cursors)))
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    tagged_type_name(&value, cursor),
                    "number",
                ))
            }
        }

        // #1687: `sort`/`sort_by`/`unique`/`unique_by`/`min`/`min_by`/`max`/
        // `max_by` all answer a permutation or subset of their *input's own*
        // elements, computing nothing new except the comparison keys -- so
        // there is no reason for any of them to route through the `_` bridge
        // below, which materializes the whole document into an `IndexMap`-
        // backed `OwnedValue` first and collapses every duplicate mapping key
        // in the process. Real yq preserves them (verified live against
        // v4.53.3 for `sort`, `sort_by`, `unique`, `unique_by`, `min`, `max`
        // and `reverse`; `min_by`/`max_by` are lexer-rejected there, so they
        // follow their siblings and `keys`, which yq does implement and which
        // preserves).
        //
        // Array inputs only, on purpose -- see `sort_family_array_generic`'s
        // doc comment for why a non-array input bridges instead of growing a
        // second copy of `eval.rs`'s per-builtin error wording.
        Builtin::Sort | Builtin::SortBy(_) | Builtin::Unique | Builtin::UniqueBy(_) => {
            let Some(elements) = value
                .as_array()
                .filter(|_| reordering_may_keep_cursors::<V>(cursor.as_ref()))
            else {
                return bridge_to_full_evaluator::<S, _>(
                    &Expr::Builtin(builtin.clone()),
                    value,
                    cursor,
                    optional,
                );
            };
            let key = match builtin {
                Builtin::SortBy(f) | Builtin::UniqueBy(f) => Some(&**f),
                _ => None,
            };
            let dedup = matches!(builtin, Builtin::Unique | Builtin::UniqueBy(_));
            // #2261 (systematic sweep): `collect_cursors_checked`, not the
            // unchecked `collect_cursors` this arm used -- every one of
            // these builtins already resolves each element's cursor to
            // sort/dedup them, so the #1677/#2261 gap checks ride along
            // for free.
            let cursors = owned_or_err!(elements.collect_cursors_checked());
            sort_family_array_generic::<S, _>(cursors, key, optional, |mut keyed| {
                sort_keyed_elements::<V>(&mut keyed);
                if dedup {
                    // `owned_value_eq::<S>`, not `compare_values(..) ==
                    // Equal`: the sort above stays widening, but two
                    // elements only count as duplicates under `==`'s own
                    // yq-mode strict Int/Float distinction (#950). Same
                    // choice `eval::builtin_unique` makes, reused rather
                    // than re-derived.
                    keyed.dedup_by(|(a, _), (b, _)| owned_value_eq::<S>(a, b));
                }
                keyed.into_iter().map(|(_, cursor)| cursor).collect()
            })
        }

        // The single-element half of the same family. `min`/`max` answer one
        // of the input's own elements, so the winner is returned as a bare
        // `OneCursor` -- the same shape `eval_first_or_last_generic` already
        // uses to keep `first(.[])` lossless (#607).
        Builtin::Min | Builtin::MinBy(_) | Builtin::Max | Builtin::MaxBy(_) => {
            let Some(elements) = value
                .as_array()
                .filter(|_| reordering_may_keep_cursors::<V>(cursor.as_ref()))
            else {
                return bridge_to_full_evaluator::<S, _>(
                    &Expr::Builtin(builtin.clone()),
                    value,
                    cursor,
                    optional,
                );
            };
            // #2261 (systematic sweep): `collect_cursors_checked`, not the
            // unchecked `collect_cursors` this arm used -- every one of
            // these builtins already resolves each element's cursor to
            // compare them, so the #1677/#2261 gap checks ride along for
            // free.
            let cursors = owned_or_err!(elements.collect_cursors_checked());
            // jq answers `null` for an empty array, for all four spellings.
            if cursors.is_empty() {
                return GenericResult::Owned(OwnedValue::Null);
            }
            let key = match builtin {
                Builtin::MinBy(f) | Builtin::MaxBy(f) => Some(&**f),
                _ => None,
            };
            let keyed = match key_elements_generic::<S, V>(cursors, key, optional) {
                Ok(keyed) => keyed,
                Err(control) => return sort_family_control(control),
            };
            // `min_by`/`max_by` on ties: jq's own definitions keep the
            // *first* minimum and the *last* maximum (`min_by` uses `<`,
            // `max_by` uses `<=` internally), which is exactly what
            // `Iterator::min_by`/`max_by` do. Reusing them rather than
            // hand-rolling the comparison keeps that asymmetry from being
            // re-derived and getting it backwards.
            let winner = if matches!(builtin, Builtin::Min | Builtin::MinBy(_)) {
                keyed
                    .into_iter()
                    .min_by(|(a, _), (b, _)| compare_values(a, b))
            } else {
                keyed
                    .into_iter()
                    .max_by(|(a, _), (b, _)| compare_values(a, b))
            };
            match winner {
                Some((_, cursor)) => GenericResult::OneCursor(cursor),
                None => unreachable!("the empty case returned above"),
            }
        }

        // #1739: the `_` fallback below pays for a full materialize +
        // re-serialize + re-index round trip of `value` on every call, just
        // to answer a single-key membership check. Native only when
        // `key_expr` evaluates to one plain value -- mirrors
        // `eval_limit_generic`'s own established precedent (#1607): a
        // generator key (`has(("a","b"))`) needs jq's per-output fan-out and
        // `ArgFanout::yq_native`'s yq-mode first-only truncation, both
        // already correctly implemented by `eval::fanout_arg`/`builtin_has`,
        // so that shape is left on the existing round-trip path rather than
        // re-implemented here. `Builtin::In` is not covered by this slice:
        // its own receiver (`.`) is the *key*, not the container, so it
        // rarely carries the large-document cost this fixes for `has`.
        Builtin::Has(key_expr) => {
            match eval_has_generic::<S, _>(key_expr, value.clone(), optional, cursor) {
                Some(result) => result,
                None => bridge_to_full_evaluator::<S, _>(
                    &Expr::Builtin(builtin.clone()),
                    value,
                    cursor,
                    optional,
                ),
            }
        }

        // #1909: `path(f)`'s output is a bounded set of path arrays, but the
        // `_` fallback below charged it a whole-document materialize +
        // re-serialize + re-index round trip *and* a second materialize
        // inside `eval::builtin_path` itself. `builtin_path_on_owned` is that
        // function with its own `to_owned` lifted out, so the tree built here
        // is the only one -- taken only where the round trip it
        // replaces was a semantic no-op, and falling back to that round trip
        // verbatim otherwise. See `reindex_bridge_is_identity`, and the
        // `Expr::Pipe` arm above for why `optional` isn't threaded in.
        Builtin::Path(path_expr) => {
            // #2061: `path(...)` output is small and bounded, but this arm
            // materialized the whole document to produce it -- `path(.[0])`
            // on a 20 MB array cost 0.78s and 519 MiB against `.[0]`'s 0.09s
            // and 34 MiB, and `path(.)` cost 1003 MiB to answer `[]`.
            //
            // For a purely navigational path expression the answer depends
            // on the document's *structure* along the path, never on a value
            // that has to be computed, so it can be walked with cursors.
            //
            // The whole-document walk is not skipped, only the tree build:
            // `to_owned_with_cursor` doubles as a validity gate (#1755/
            // #1953), and dropping it would make `path(.d)` start accepting
            // documents it rejects today. `push_generic_truthiness_cursor_error`
            // is that same traversal and validation with the `OwnedValue`
            // construction removed, so error behaviour is unchanged while the
            // allocation is not paid.
            if let Some(root) = cursor.filter(|_| path_expr_is_cursor_navigable(path_expr)) {
                if let Some(control) = push_generic_truthiness_cursor_error(&root, 0) {
                    return match control {
                        Control::Error(e) => GenericResult::Error(e),
                        Control::Break(label) => GenericResult::Break(label),
                        Control::Halt(code) => GenericResult::Halt(code),
                    };
                }
                let mut out = Vec::new();
                return match path_walk_generic::<S, V>(
                    path_expr,
                    &PathNode::At(root),
                    &PathTrail::root(),
                    &mut out,
                ) {
                    Ok(()) => owned_vec_to_generic_result(out),
                    // Whatever resolved before the failure still stands:
                    // jq's generator never un-emits an output it already
                    // produced, so `path(.a.b, .c.d)` on `{"a":{"b":1},"c":1}`
                    // emits `["a","b"]` and *then* errors. Discarding the
                    // prefix here is what `builtin_path_on_owned`'s own doc
                    // comment warns against, and the evaluator-parity and
                    // golden suites both caught it.
                    Err(e) => partial_generic(out, Control::Error(e)),
                };
            }
            // #2280: owned_or_suppress!, not owned_or_err! -- `optional` is a
            // live parameter of this arm (threaded into `eval_on_owned`
            // below), so ignoring it in this materialization specifically
            // was a real asymmetry, matching the sibling fix already applied
            // to `Builtin::ToString`'s arm and its catch-all fallback (#2231).
            //
            // Defensive, not a live behavior change today, for the same
            // reason #2231's own findings 1-3 became defensive after #2286:
            // this materialization's only error paths are all
            // `is_decode_failure()`-tagged now, so `suppresses()` is always
            // `false` here regardless of `optional` -- verified live, see
            // `test_optional_ignored_sites_2280`.
            let owned = owned_or_suppress!(to_owned_with_cursor(&value, cursor), optional);
            if reindex_bridge_is_identity(&owned) {
                return query_result_to_generic::<V>(crate::jq::eval::builtin_path_on_owned::<
                    Vec<u64>,
                    S,
                >(path_expr, &owned, false));
            }
            eval_on_owned::<S, _>(&Expr::Builtin(builtin.clone()), owned, optional)
        }

        // #2053: mirrors `Builtin::Path`'s #1909 treatment above -- the `_`
        // fallback below pays a materialize + re-serialize + re-index round
        // trip of `value` *and* a second whole-document decode inside
        // `eval::getpath_one_path`'s own `to_owned`, just to answer
        // a single node lookup. Unlike `Path`, `getpath`'s root
        // materialization is not waste to begin with (#1755/#1953): it is
        // the exact same validity gate `to_owned_with_cursor` below already
        // pays for -- `{"a":"\ud800","d":5} | getpath(["d"])` must still
        // raise even though the walk never reaches `.a`, where `.d`/`keys`/
        // `length` on the same document do not. Only the *second* decode
        // and the reindex round trip go, not the first.
        //
        // The bypass-vs-fallback decision below is made purely from
        // `owned`'s own shape (`reindex_bridge_is_identity`), *before*
        // `path_expr` is touched at all. A withdrawn earlier attempt at
        // this fix (PR #2045) instead probed `path_expr`'s output shape and
        // fell back to a second, full re-evaluation when the probe didn't
        // fit a recognized pattern, which fired any side effect inside
        // `path_expr` twice: `getpath(("a"|stderr))` printed `aa` instead
        // of jq's `a`. `fanout_arg_generic` is the fix for that shape
        // (#1687, already used by `limit`/`nth`/`has` above): it drives
        // `path_expr` lazily, exactly once, against the *original* cursor
        // `value` -- so a `path_expr` that never touches `.` (the
        // overwhelmingly common `getpath([0])`/`getpath(["a","b"])` shape)
        // costs nothing extra beyond evaluating a small literal, and one
        // that does touch `.` reads it through the same cheap cursor
        // navigation any other builtin's argument already gets, rather than
        // a second reindex of the whole document.
        //
        // Each resolved path then walks `owned` -- already materialized
        // above -- directly via `getpath_walk_owned`, which is
        // `eval::getpath_one_path`'s own walk with its `to_owned`
        // call lifted out (mirrors `builtin_path_on_owned`'s identical
        // relationship to `builtin_path`). `optional` is threaded through
        // for real here, unlike `Path`/`Key`/`Parent`/`FileIndex`'s
        // hardcoded `false` above -- `getpath_walk_owned` uses it to decide
        // whether an indexing failure partway through a path raises or
        // suppresses, so hardcoding it would be a real correctness risk if
        // dispatch ever changed to reach here with `optional = true`.
        // Review found no such reachable path today (`Expr::Optional`'s
        // catch-all evaluates its inner expression at the *ambient*
        // `optional`, same as the already-documented `Map`/`Select`
        // precedent at `test_generic_plain_map_optional_on_non_container_is_unreachable_via_parser_725`,
        // so `getpath(P)?` never actually forces `optional = true` into
        // this arm) -- confirmed by swapping in a hardcoded `false` here
        // and finding it byte-identical across the full `getpath` test
        // suite. Threading it for real costs nothing and removes the
        // dependency on that reachability analysis staying true, so it
        // stays -- but it is defensive, not currently load-bearing.
        Builtin::GetPath(path_expr) => {
            // #2280: owned_or_suppress!, not owned_or_err! -- this arm's own
            // walk below already threads `optional` "for real" into
            // `getpath_walk_owned`'s per-step suppress/raise decision, so
            // leaving the initial materialization unconditional was an
            // inconsistency inside the very arm whose comment documents the
            // opposite intent. Defensive today for the same #2286 reason as
            // the `Path` arm above -- see `test_optional_ignored_sites_2280`.
            let owned = owned_or_suppress!(to_owned_with_cursor(&value, cursor), optional);
            if reindex_bridge_is_identity(&owned) {
                return fanout_arg_generic::<S, V, _>(
                    path_expr,
                    value.clone(),
                    optional,
                    cursor,
                    |path_owned| {
                        query_result_to_generic::<V>(crate::jq::eval::getpath_walk_owned::<
                            Vec<u64>,
                            S,
                        >(
                            &owned, &path_owned, optional
                        ))
                    },
                );
            }
            eval_on_owned::<S, _>(&Expr::Builtin(builtin.clone()), owned, optional)
        }

        Builtin::Empty => GenericResult::None,

        Builtin::ToString => {
            // #2231: `owned_or_suppress!`, not the bare `owned_or_err!` --
            // the `eval.rs` twin of this arm (`builtin_tostring`, #2184)
            // already consults `optional`, and this file's own
            // `owned_or_err!` unconditionally errored regardless of it, the
            // same way `eval.rs`'s pre-#2184 sites did. Same non-live-
            // reachable defensive-consistency class as the rest of this
            // lineage.
            let owned = owned_or_suppress!(to_owned_with_cursor(&value, cursor), optional);
            GenericResult::Owned(OwnedValue::String(owned_to_string::<S>(&owned)))
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
            } else {
                decode_failure_or(&value, optional, || {
                    GenericResult::Error(EvalError::cannot_parse_as_number(
                        &to_owned_for_diagnostic(&value, cursor),
                    ))
                })
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

        // For other builtins, fall back to full evaluator via JSON. Reached
        // by every `Builtin` variant with no dedicated arm above -- dozens,
        // not just `ToJson`/`UpperIndex`/`UpperIndexStream`/`ToJsonStream`/
        // `ToStream` (code review, #2231: an earlier revision of this
        // comment named only those five, which undersells how many
        // builtins actually funnel through here).
        _ => {
            // #2231: `owned_or_suppress!`, not `owned_or_err!` -- this
            // bridging materialization ran unconditionally regardless of
            // `optional` before this fix, an inconsistency with no live-
            // reachable effect today (`eval_try` already suppresses one
            // level up) for whichever of the five #2184-fixed builtins
            // above reach here; unverified for the rest of this arm's
            // callers (tracked separately as #2280).
            let owned = owned_or_suppress!(to_owned_with_cursor(&value, cursor), optional);
            eval_on_owned::<S, _>(&Expr::Builtin(builtin.clone()), owned, optional)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::expr::{CompareOp, FormatType, Literal};
    use super::super::value::NumberRepr;
    use super::*;

    /// #1909: [`reindex_bridge_is_identity`] claims to describe exactly when
    /// `eval_on_owned`'s serialize + `JsonIndex::build` +
    /// `owned_from_standard_json` round trip leaves a value unchanged — the whole basis for the
    /// path-context arms skipping that round trip. A predicate that says
    /// "identity" about a value the bridge actually rewrites is a silent
    /// correctness bug, so assert the two against each other directly rather
    /// than trusting the reasoning in its doc comment (CLAUDE.md:
    /// "duplicated predicates diverge silently").
    ///
    /// The corpus straddles every boundary the predicate draws: a bare
    /// `Float` (re-spelled), a bare `Int` (normalized to a `NumberLiteral`
    /// carrying the same text it already rendered as — the one sanctioned
    /// exception, spelled out in `round_trips_unchanged`), a NaN literal
    /// (replaced by `NAN_SENTINEL`), and a `NumberLiteral` either side of
    /// `REINDEX_LITERAL_LEN_CAP`, which is duplicated from a private `const`
    /// inside `to_json_for_reindex`'s body and cannot be shared.
    ///
    /// One case is *not* checked against the bridge here, and saying so
    /// matters more than the check it replaces: `round_trips_unchanged`
    /// normalizes a bare `Int` before comparing, so `Int` passes this test by
    /// construction rather than by observation. The equality that
    /// normalization assumes is asserted on its own in
    /// [`test_reindex_bridge_int_normalization_1909`] — otherwise the most
    /// load-bearing claim the guard makes would be the one nothing can
    /// falsify (code review).
    ///
    /// What is asserted is **soundness** — predicate ⟹ the bridge really was
    /// an identity — not equivalence. Conservatism is free here (a `false`
    /// for something the bridge happens to leave alone costs a missed
    /// optimization, never a wrong answer), and the predicate genuinely is
    /// conservative for a very long literal, which Rust's `Display` for a
    /// small-magnitude float re-emits in the same decimal spelling. A
    /// separate reachability block stops that latitude from being abused: a
    /// predicate that stayed sound by answering `false` to everything would
    /// silently un-fix #1909 while every other test still passed.
    #[test]
    fn test_reindex_bridge_identity_predicate_agrees_1909() {
        // A literal at, and one past, the length cap. Both parse to the same
        // number; only the longer one loses its text on the round trip.
        let at_cap = format!("0.{}1", "0".repeat(super::REINDEX_LITERAL_LEN_CAP - 3));
        let past_cap = format!("0.{}1", "0".repeat(super::REINDEX_LITERAL_LEN_CAP));
        assert_eq!(at_cap.len(), super::REINDEX_LITERAL_LEN_CAP);
        assert!(past_cap.len() > super::REINDEX_LITERAL_LEN_CAP);
        assert!(super::reindex_bridge_is_identity(
            &OwnedValue::from_number_literal(&at_cap)
        ));
        assert!(!super::reindex_bridge_is_identity(
            &OwnedValue::from_number_literal(&past_cap)
        ));

        let corpus: Vec<OwnedValue> = vec![
            OwnedValue::Null,
            OwnedValue::Bool(true),
            OwnedValue::String(String::new()),
            OwnedValue::String("a \" b \\ c \n \u{1f600} \u{7f}".to_string()),
            OwnedValue::from_number_literal("1"),
            OwnedValue::from_number_literal("-0"),
            OwnedValue::from_number_literal("3.5"),
            OwnedValue::from_number_literal("1e18"),
            OwnedValue::from_number_literal("10000000000000000000.0"),
            OwnedValue::from_number_literal("123e400"),
            OwnedValue::from_number_literal(&at_cap),
            OwnedValue::from_number_literal(&past_cap),
            OwnedValue::Int(7),
            OwnedValue::Float(3.5),
            OwnedValue::Float(1e19),
            OwnedValue::Float(f64::NAN),
            OwnedValue::Array(Vec::new()),
            OwnedValue::Object(IndexMap::new()),
            OwnedValue::Array(vec![
                OwnedValue::from_number_literal("1"),
                OwnedValue::String("x".to_string()),
            ]),
            // A rewritten value nested under an otherwise-clean container
            // must make the whole tree fail the predicate.
            OwnedValue::Object(IndexMap::from([
                ("ok".to_string(), OwnedValue::from_number_literal("1")),
                ("bad".to_string(), OwnedValue::Float(1e19)),
            ])),
            OwnedValue::Object(IndexMap::from([
                ("a \" b".to_string(), OwnedValue::from_number_literal("2")),
                ("\u{1f600}".to_string(), OwnedValue::Bool(false)),
            ])),
        ];

        // Soundness, the direction that matters: whenever the predicate
        // says "identity", the real round trip must actually be one. The
        // converse is deliberately *not* asserted -- the predicate is allowed
        // to be conservative (`false` for something the bridge happens to
        // leave alone costs a missed optimization, never a wrong answer), and
        // it genuinely is for a very long literal, which Rust's `Display` for
        // a small-magnitude float re-emits in the same decimal spelling.
        for value in &corpus {
            for (tag, actually_identity) in [
                ("jq", round_trips_unchanged::<JqSemantics>(value)),
                ("yq", round_trips_unchanged::<YqSemantics>(value)),
            ] {
                assert!(
                    !super::reindex_bridge_is_identity(value) || actually_identity,
                    "reindex_bridge_is_identity claims the {tag}-mode bridge \
                     leaves {value:?} unchanged, but it does not"
                );
            }
        }

        // ...and reachability, so a predicate that stays sound by answering
        // `false` to everything (which would silently un-fix #1909 while
        // every other test still passed) fails here instead.
        for value in [
            &corpus[0],
            &OwnedValue::from_number_literal("1"),
            &OwnedValue::from_number_literal("3.5"),
            &OwnedValue::Array(vec![
                OwnedValue::from_number_literal("1"),
                OwnedValue::String("x".to_string()),
            ]),
            &OwnedValue::Object(IndexMap::from([(
                "ok".to_string(),
                OwnedValue::from_number_literal("1"),
            )])),
            // The shape every real YAML document has: a bare `Int`. Rejecting
            // it left `succinctly yq` with no bypass at all (code review),
            // since YAML's `number_literal()` override only preserves floats.
            &OwnedValue::Int(7),
            &OwnedValue::Object(IndexMap::from([
                ("count".to_string(), OwnedValue::Int(3)),
                ("name".to_string(), OwnedValue::String("x".to_string())),
            ])),
        ] {
            assert!(
                super::reindex_bridge_is_identity(value),
                "the bypass must fire for an ordinary document-sourced value: {value:?}"
            );
        }

        // The shape the guard exists for: a bare `Float`, which #953's
        // mode-forked re-spelling rewrites. Asserted in both directions --
        // the predicate refuses it, *and* the bridge really does change it,
        // so this case can't quietly stop being a real one.
        let respelled = OwnedValue::Float(1e19);
        assert!(!super::reindex_bridge_is_identity(&respelled));
        assert!(
            !round_trips_unchanged::<YqSemantics>(&respelled),
            "sanity: the yq-mode bridge really does rewrite {respelled:?}"
        );
        assert!(
            !round_trips_unchanged::<JqSemantics>(&respelled),
            "sanity: the jq-mode bridge really does rewrite {respelled:?}"
        );
    }

    /// The one thing [`reindex_bridge_is_identity`]'s `Int` arm rests on, and
    /// the one thing `round_trips_unchanged` cannot check because it
    /// normalizes `Int` before comparing: `to_json_for_reindex` spells a bare
    /// `Int` as exactly `format!("{n}")`, in **both** modes.
    ///
    /// That is what makes the resulting `NumberLiteral(Int(n), "n")` carry no
    /// text the bare value didn't already render as — the difference from a
    /// bare `Float`, whose spelling is mode-forked (#953) and genuinely
    /// differs, which is why the guard refuses that one. If the `Int` arm in
    /// `to_json_for_reindex` (`src/jq/value.rs`, currently reached through
    /// `to_json_at_depth`'s own `Self::Int(n) => format!("{n}")` fallback)
    /// ever grows a mode fork or a separator, this fails here rather than
    /// silently changing what `succinctly yq` prints for a `parent`/`key`
    /// query on any document with an integer in it.
    #[test]
    fn test_reindex_bridge_int_normalization_1909() {
        for n in [0i64, 1, -1, 7, -7, 1000, i64::MIN, i64::MAX] {
            let value = OwnedValue::Int(n);
            let expected = alloc::format!("{n}");
            assert_eq!(
                value.to_json_for_reindex::<JqSemantics>(),
                expected,
                "jq-mode reindex spelling for Int({n})"
            );
            assert_eq!(
                value.to_json_for_reindex::<YqSemantics>(),
                expected,
                "yq-mode reindex spelling for Int({n})"
            );
            // ...and the predicate really does admit it, so this is the arm
            // the bypass takes rather than a case it quietly falls back on.
            assert!(super::reindex_bridge_is_identity(&value));
        }
    }

    /// The real thing `reindex_bridge_is_identity` predicts: run `value`
    /// through `eval_on_owned`'s exact bridge (`to_json_for_reindex`,
    /// `JsonIndex::build`, `owned_from_standard_json`) and report whether it
    /// came back unchanged.
    fn round_trips_unchanged<S: EvalSemantics>(value: &OwnedValue) -> bool {
        use crate::json::JsonIndex;
        let json = value.to_json_for_reindex::<S>();
        let bytes = json.as_bytes();
        let index = JsonIndex::build(bytes);
        let cursor = index.root(bytes);
        let round_tripped = owned_from_standard_json(&cursor.value())
            .expect("the bridge's own serialization must reparse");

        // Structural equality, not `==`: `OwnedValue`'s `PartialEq` compares
        // a `NumberLiteral` by its parsed `NumberRepr` and ignores the source
        // text, but that text is exactly what #1008's literal preservation
        // echoes back on output -- so a value whose spelling the bridge
        // rewrote (`0.000...1` -> `1e-257`) compares *equal* while behaving
        // differently. The derived `Debug` shows the text; that is the
        // identity this predicate has to be about.
        //
        // The single sanctioned exception is `Int(n)` -> `NumberLiteral(
        // Int(n), format!("{n}"))`: a normalization that bakes in the only
        // spelling an `i64` has, so it introduces no text the bare value
        // didn't already render as. Spelled out here rather than folded into
        // a looser "renders the same" comparison, because rendering equality
        // alone would also wave through a bare `Float`, whose re-spelling is
        // precisely what this guard exists to catch. Normalizing here does
        // mean `Int` cannot *fail* this helper, so the equality it assumes
        // (`to_json_for_reindex` spells an `Int` as `format!("{n}")`) is
        // asserted on its own in `test_reindex_bridge_int_normalization_1909`
        // rather than left to this comment.
        let normalized = match value {
            OwnedValue::Int(n) => OwnedValue::from_number_literal(&alloc::format!("{n}")),
            other => other.clone(),
        };
        format!("{round_tripped:?}") == format!("{normalized:?}")
    }

    /// #1098/#1247: `JsonIndex::build`'s semi-index scan finds string
    /// quote/escape *boundaries* but never decodes/validates the bytes
    /// between them -- that's deferred to `JsonString::as_str()`, called
    /// lazily by `to_owned_at_depth`. So invalid UTF-8 inside a string span
    /// parses and indexes fine (`JsonIndex::build` never errors), and the
    /// failure only becomes observable once materialization reaches it.
    ///
    /// This test used to assert the *degrade* (`Some(&OwnedValue::Null)`),
    /// pinning it as a known, deliberate gap. #1247 closed it: the failure
    /// now travels as an `EvalError` through `GenericResult::Error` /
    /// `Control::Error`, which is what `to_owned_at_depth`'s own reverted
    /// `panic!` attempt (PR #1190) was trying and failing to achieve --
    /// evaluation still continues past the error rather than aborting the
    /// process, so the `ErrorSink` convention (#355) holds.
    #[test]
    fn test_to_owned_errors_on_string_decode_failure_1247() {
        use crate::json::JsonIndex;
        let json: &[u8] = b"{\"a\": \"\xff\xfe\"}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = to_owned(&value).expect_err("an undecodable string must not materialize");
        assert!(
            err.message.contains("invalid UTF-8"),
            "message: {}",
            err.message
        );
    }

    /// #1247 used to raise on an undecodable *key* here (closing the
    /// original fault: it used to drop the whole field instead of
    /// degrading its value). #1642 preserves it instead, via its raw
    /// source span (lossily decoded), matching `length`/`keys_unsorted`/
    /// `.`.
    #[test]
    fn test_to_owned_preserves_object_key_decode_failure_1642() {
        use crate::json::JsonIndex;
        let json: &[u8] = b"{\"\xff\xfe\": 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let owned = to_owned(&value).expect("an undecodable key is preserved, not raised on");
        assert_eq!(
            owned,
            OwnedValue::Object(IndexMap::from([(
                "\u{FFFD}\u{FFFD}".to_string(),
                OwnedValue::from_number_literal("1")
            )]))
        );
    }

    /// #2299 code review: `to_owned_checked_at_depth`'s recursion shape must
    /// answer the depth question identically to `to_owned_at_depth`'s own
    /// panicking guard, on both branches it mirrors. This pins the array
    /// branch: `to_owned` panics on 256-deep nesting (the guard this
    /// function stands in for), `to_owned_checked` reports a clean error
    /// instead, at the exact same boundary.
    #[test]
    fn test_to_owned_checked_reports_clean_error_past_nesting_limit_2299() {
        use crate::json::JsonIndex;
        let json = format!("{}1{}", "[".repeat(256), "]".repeat(256));
        let json = json.as_bytes();
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err =
            to_owned_checked(&value).expect_err("256 levels of array nesting exceeds the limit");
        assert!(
            err.message.contains("nesting depth exceeds limit of 256"),
            "message: {}",
            err.message
        );
    }

    /// Companion to the test above: legitimately-nested input well under
    /// the limit must still materialize normally through the checked path.
    #[test]
    fn test_to_owned_checked_accepts_nesting_under_limit_2299() {
        use crate::json::JsonIndex;
        let json = format!("{}1{}", "[".repeat(100), "]".repeat(100));
        let json = json.as_bytes();
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        to_owned_checked(&value).expect("100 levels of array nesting is well under the limit");
    }

    /// #2299 code review: the array branch alone doesn't exercise
    /// `to_owned_checked_at_depth`'s object branch (`as_object`/`uncons`,
    /// recursing on `field.value`) -- a future edit that only breaks that
    /// branch (e.g. recursing on `field.key` instead, or dropping
    /// `depth + 1`) would pass every other test in this file, since none of
    /// them build a deeply *object*-nested document. Pinned here with the
    /// object-shaped analog of the two tests above.
    #[test]
    fn test_to_owned_checked_object_branch_reports_clean_error_past_nesting_limit_2299() {
        use crate::json::JsonIndex;
        let json = format!("{}1{}", "{\"a\":".repeat(256), "}".repeat(256));
        let json = json.as_bytes();
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err =
            to_owned_checked(&value).expect_err("256 levels of object nesting exceeds the limit");
        assert!(
            err.message.contains("nesting depth exceeds limit of 256"),
            "message: {}",
            err.message
        );
    }

    /// Object-branch companion to the under-limit array test above.
    #[test]
    fn test_to_owned_checked_object_branch_accepts_nesting_under_limit_2299() {
        use crate::json::JsonIndex;
        let json = format!("{}1{}", "{\"a\":".repeat(100), "}".repeat(100));
        let json = json.as_bytes();
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let owned =
            to_owned_checked(&value).expect("100 levels of object nesting is well under the limit");
        // Not just "didn't error" -- confirm the object branch actually
        // materialized real content, not an early-exit stub.
        assert!(matches!(owned, OwnedValue::Object(_)));
    }

    /// #2231 finding 3: `Builtin::ToString`'s own dedicated arm and the
    /// catch-all wildcard fallback arm both raised an ordinary #1194
    /// malformed-member error unconditionally, ignoring `optional` --
    /// the `eval_generic.rs` twin of `eval.rs`'s `builtin_tostring`
    /// (#2184) and findings 1-3's other `eval.rs` sites -- true when #2231
    /// landed.
    ///
    /// #2286 has since tagged #1194 malformed-member/delimiter errors
    /// `is_decode_failure()`, making both sites' `optional`-consulting a
    /// no-op for this specific error class -- see
    /// `test_debug_stderr_fromjsonstream_respect_optional_2231`'s own
    /// revised doc comment (`eval.rs`) for the full reasoning, which
    /// applies identically here. Still called directly through
    /// `eval_builtin`, not through `eval`/`?`: that distinction no longer
    /// matters for *this* error class (uncatchable at every level either
    /// way now), but stays for consistency with the sibling decode-failure
    /// test just below, which does still need it.
    #[test]
    fn test_generic_tostring_and_wildcard_respect_optional_2231() {
        use crate::json::JsonIndex;
        let json: &[u8] = br#"{"a":1,"b"}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        for optional in [true, false] {
            match eval_builtin::<JqSemantics, _>(
                &Builtin::ToString,
                value.clone(),
                optional,
                Some(cursor),
            ) {
                GenericResult::Error(e) => {
                    assert!(e.is_decode_failure(), "expected decode failure, got: {e:?}");
                }
                other => panic!(
                    "expected a decode failure regardless of optional={optional}, got: {other:?}"
                ),
            }
        }

        // The catch-all wildcard arm, reached via a builtin with no
        // dedicated arm of its own (e.g. `ToJson`).
        for optional in [true, false] {
            match eval_builtin::<JqSemantics, _>(
                &Builtin::ToJson,
                value.clone(),
                optional,
                Some(cursor),
            ) {
                GenericResult::Error(e) => {
                    assert!(e.is_decode_failure(), "expected decode failure, got: {e:?}");
                }
                other => panic!(
                    "expected a decode failure regardless of optional={optional}, got: {other:?}"
                ),
            }
        }
    }

    /// #2231 finding 3: a genuine decode failure must still survive
    /// `optional` for both arms above -- same rule #2184's own sites keep.
    #[test]
    fn test_generic_tostring_and_wildcard_decode_failure_survives_optional_2231() {
        use crate::json::JsonIndex;
        let json: &[u8] = &b"\"\xff\xfe\""[..];
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        match eval_builtin::<JqSemantics, _>(&Builtin::ToString, value.clone(), true, Some(cursor))
        {
            GenericResult::Error(e) => assert!(e.is_decode_failure()),
            other => panic!("expected a decode failure to survive `optional`, got: {other:?}"),
        }
        match eval_builtin::<JqSemantics, _>(&Builtin::ToJson, value, true, Some(cursor)) {
            GenericResult::Error(e) => assert!(e.is_decode_failure()),
            other => panic!("expected a decode failure to survive `optional`, got: {other:?}"),
        }
    }

    /// #1620: `?` must not suppress a decode failure -- `Expr::Optional`'s
    /// own arm now excludes it explicitly (see its doc comment), instead of
    /// folding it into the ordinary `Error`/`Break` -> `None` catch every
    /// other error takes.
    #[test]
    fn test_generic_optional_does_not_suppress_decode_failure_1620() {
        use crate::json::JsonIndex;
        let json: &[u8] = b"\"\xff\xfe\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(
            &Expr::Optional(Box::new(Expr::Builtin(Builtin::Length))),
            value,
        );

        match result {
            GenericResult::Error(e) => assert!(
                e.is_decode_failure() && e.message.contains("invalid UTF-8"),
                "message: {}",
                e.message
            ),
            other => panic!("expected an uncaught decode-failure error, got {other:?}"),
        }
    }

    /// #1620: `try`/`catch` -- routed through the reindex-bridge wildcard
    /// since this module has no native `Expr::Try` arm (see that arm's own
    /// doc comment) -- must not catch a decode failure either.
    #[test]
    fn test_generic_try_catch_does_not_catch_decode_failure_1620() {
        use crate::json::JsonIndex;
        let json: &[u8] = b"{\"a\": \"\xff\xfe\"}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(
            &Expr::Try {
                expr: Box::new(Expr::Pipe(vec![
                    Expr::Field("a".to_string()),
                    Expr::Builtin(Builtin::Length),
                ])),
                catch: Some(Box::new(Expr::Literal(Literal::String(
                    "caught".to_string(),
                )))),
            },
            value,
        );

        match result {
            GenericResult::Error(e) => assert!(
                e.is_decode_failure() && e.message.contains("invalid UTF-8"),
                "message: {}",
                e.message
            ),
            other => panic!("expected an uncaught decode-failure error, got {other:?}"),
        }
    }

    /// #1660: `each_pattern_alternatives_generic`'s own decode-failure
    /// exclusion, reached via this module's *native* `Expr::AsPattern` arm
    /// (`each_as_pattern_generic`) rather than the reindex-bridge wildcard
    /// the two tests above exercise -- `first(...)` drives evaluation
    /// through `eval_each_generic` directly, which is what actually calls
    /// into this function. Before the fix, a decode failure inside a
    /// non-last `?//` alternative's body silently fell through to the next
    /// alternative here too, exactly like `eval::each_pattern_alternatives`.
    #[test]
    fn test_generic_pattern_alternative_retry_does_not_swallow_decode_failure_1660() {
        use crate::json::JsonIndex;
        let json: &[u8] = b"{\"a\": \"\xff\xfe\", \"p\": 1, \"q\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(
            &parse("first([.p,.q] as [$y] ?// [$z,$y] | (if $y==1 then (.a|length) else $y end))")
                .unwrap(),
            value,
        );

        match result {
            GenericResult::Error(e) => assert!(
                e.is_decode_failure() && e.message.contains("invalid UTF-8"),
                "message: {}",
                e.message
            ),
            other => panic!("expected an uncaught decode-failure error, got {other:?}"),
        }
    }

    /// #1519: the generic evaluator's own `?//` retry loop
    /// (`each_pattern_alternatives_generic`) and its own reshaped terminal
    /// sinks (`each_take_first_generic`, `nth_with_n_generic`) are separate
    /// code from `eval.rs`'s, and `first`/`last` additionally route through
    /// this module *before* `eval.rs`'s `eval_each` ever sees them (#1461) --
    /// so this exercises them directly rather than relying on the shared arms.
    /// Companion to
    /// `eval::tests::test_pattern_alternatives_retry_on_consumer_stop_1519`;
    /// expectations are real jq 1.7.1's.
    #[test]
    fn test_generic_pattern_alternatives_retry_on_consumer_stop_1519() {
        use crate::json::JsonIndex;

        fn got(json: &[u8], filter: &str) -> Vec<String> {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            eval(&parse(filter).unwrap(), cursor.value())
                .collect_owned()
                .expect("materializes")
                .iter()
                .map(OwnedValue::to_json)
                .collect()
        }

        for (json, filter, want) in [
            (&b"null"[..], "[first(1 as $x ?// $y | 5, 6)]", "[5,5]"),
            (
                &b"null"[..],
                "[isempty(1 as $x ?// $y | 5)]",
                "[false,false]",
            ),
            (&b"null"[..], "[limit(1; 1 as $x ?// $y | 5)]", "[5,5]"),
            (&b"null"[..], "[nth(1; 1 as $x ?// $y | 5, 6)]", "[6,5]"),
            (&b"null"[..], "[limit(2; 1 as $x ?// $y | 5,6)]", "[5,6,5]"),
            // Cursor-backed source values, so the multi-item retry goes
            // through `items_to_generic_result` rather than the lone-item
            // `generic_item_to_result` shortcut.
            (
                &br#"{"a": [1, 2]}"#[..],
                "[first(.a as [$p] ?// $p | $p, 9)]",
                "[1,[1,2]]",
            ),
            (
                &br#"{"a": [1, 2]}"#[..],
                "[nth(1; .a as [$p] ?// $p | $p, 7)]",
                "[7,[1,2]]",
            ),
            // Negative controls: no consumer, and a single (non-`?//`)
            // pattern, must both stay single-answer.
            (&b"null"[..], "[1 as $x ?// $y | 5]", "[5]"),
            (&br#"{"a": [1]}"#[..], "[first(.a as [$p] | $p, 9)]", "[1]"),
        ] {
            assert_eq!(got(json, filter), vec![want.to_string()], "{filter}");
        }
    }

    /// #1519: a control raised by a *retried* alternative is genuinely reached
    /// by jq, so `each_take_first_generic`'s caller keeps the earlier
    /// alternative's output and still raises -- the generic twin of
    /// `eval::tests::test_pattern_alternative_retry_trailing_control_1519`.
    #[test]
    fn test_generic_first_retried_alternative_error_raises_1519() {
        use crate::json::JsonIndex;
        let json: &[u8] = b"null";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let expr = parse(
            "first([1] as [$x] ?// $x | if ($x|type)==\"number\" then 5 else error(\"boom\") end)",
        )
        .unwrap();
        match eval(&expr, cursor.value()) {
            GenericResult::Partial(prefix, Control::Error(e)) => {
                assert_eq!(
                    prefix.iter().map(OwnedValue::to_json).collect::<Vec<_>>(),
                    vec!["5".to_string()]
                );
                assert!(e.message.contains("boom"), "message: {}", e.message);
            }
            other => panic!("expected Partial([5], Error(boom)), got {other:?}"),
        }
    }

    /// #1519 + #1620/#1660: a `?//` retry hands `first`/`nth` a *batch* of
    /// items, and when the source values are cursor-backed that batch stays
    /// lazy (`GenericResult::ManyCursor`) rather than being decoded on the
    /// spot -- so the decode failure surfaces later, at materialization.
    ///
    /// The rule that matters is that it still surfaces at all: a decode
    /// failure is never retryable, and the retry must not be able to turn a
    /// raise into a silent success. Pinned for both the single-alternative and
    /// the `?//` spelling, so the two cannot diverge.
    #[test]
    fn test_generic_first_nth_retry_batch_still_raises_decode_failure_1519() {
        use crate::json::JsonIndex;
        let json: &[u8] = b"{\"a\": \"\xff\xfe\", \"p\": 1}";

        for filter in [
            // Single-alternative control: already raised before #1519.
            "first([.p] as [$y] | .a, 9)",
            // The batch spellings a `?//` retry produces.
            "first([.p] as [$y] ?// $y | .a, 9)",
            "nth(0; [.p] as [$y] ?// $y | .a, 9)",
        ] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let err = eval(&parse(filter).unwrap(), cursor.value())
                .collect_owned()
                .expect_err(filter);
            assert!(
                err.is_decode_failure(),
                "{filter}: expected a decode failure, got {}",
                err.message
            );
        }
    }

    /// #1247: a valid document still materializes unchanged -- the guard
    /// against the new check firing on anything that decodes cleanly.
    #[test]
    fn test_to_owned_still_materializes_valid_input_1247() {
        use crate::json::JsonIndex;
        let json: &[u8] = br#"{"a": "x", "b": [1, "\u00e9"]}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let owned = to_owned(&cursor.value()).expect("valid input must materialize");
        let OwnedValue::Object(map) = owned else {
            panic!("expected an object");
        };
        assert_eq!(map.get("a"), Some(&OwnedValue::String("x".to_string())));
    }
    use crate::jq::parse;
    use crate::json::JsonIndex;

    /// #1048: the yq/generic-document counterpart to #1043's `eval.rs` fix
    /// had the identical missing `0 => None` bug in 3 places -- a computed
    /// index/slice whose optional (`?`) form produces zero results
    /// collapsed to `Many`/`ManyOwned`/`ManyCursor(vec![])` instead of
    /// `None`.
    #[test]
    fn test_1048_computed_index_and_slice_zero_results_collapse_to_none() {
        let json = br"5";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        // Key-outer/target-inner loop, borrowed target (`.` is the document
        // itself, a number -- not indexable, so `?` suppresses to zero
        // results per key).
        let result = eval(&parse(r#".[("a", "b")]?"#).unwrap(), value.clone());
        assert!(
            matches!(result, GenericResult::None),
            "expected None, got {result:?}"
        );

        // Owned-target arm: `(1)` constructs an owned value rather than
        // borrowing from the document.
        let result = eval(&parse(r#"(1)[("a", "b")]?"#).unwrap(), value.clone());
        assert!(
            matches!(result, GenericResult::None),
            "expected None, got {result:?}"
        );

        // eval_slice_expr's final collapse: slicing a non-array/string
        // target with `?` suppresses to zero results per (start, end) pair.
        let result = eval(&parse(r".[0:1]?").unwrap(), value);
        assert!(
            matches!(result, GenericResult::None),
            "expected None, got {result:?}"
        );
    }

    /// #1634 review: `eval.rs`'s own end-to-end regression test for the
    /// computed-index capacity guard
    /// (`test_computed_index_ordinary_cross_product_unaffected_1634`) goes
    /// through `eval::eval()`, a *different* entry point from the one
    /// `src/bin/succinctly/{jq,yq}_runner.rs` actually call for an
    /// ordinary `.[$keys]` read -- `Expr::IndexExpr` is handled natively
    /// here, in this module's own `eval_index_expr` (see the comment on
    /// its match arm above), so that other test never touched this
    /// module's own guard at all. This test exercises this module's real
    /// dispatch path directly, targeting the specific arm the guard was
    /// added to (an *owned* target -- `({"x":1,"y":2})`, a constructed
    /// value rather than a document navigation, takes `eval_index_expr`'s
    /// `KeyTargets::Owned` arm). Since #2032, both arms carry their own
    /// per-key `try_reserve` guard (target length can vary per key now, so
    /// the single upfront `try_reserve_product` this comment used to name
    /// no longer applies to either arm) -- see the current per-key
    /// reservation logic on each arm for the mechanism this test actually
    /// exercises.
    #[test]
    fn test_generic_computed_index_ordinary_cross_product_unaffected_1634() {
        let json = br"null";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&parse(r#"({"x":1,"y":2})[("x","y")]"#).unwrap(), value);
        match result {
            GenericResult::ManyOwned(vs) => {
                assert_eq!(vs, vec![OwnedValue::Int(1), OwnedValue::Int(2)]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_generic_identity() {
        let json = br#"{"name": "Alice", "age": 30}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Identity, value);
        let owned = result.into_owned().unwrap().unwrap();

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
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(owned, OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_generic_array_index() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::index(1), value);
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(owned, OwnedValue::Int(2));
    }

    #[test]
    fn test_generic_iterate() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Iterate, value);
        let owned = result.collect_owned().unwrap();

        assert_eq!(
            owned,
            vec![OwnedValue::Int(1), OwnedValue::Int(2), OwnedValue::Int(3)]
        );
    }

    /// `GenericResult::LazyIndexRange`'s `collect_owned()` (#684) -- the
    /// fallback path used by consumers other than the `Pipe`/M2-streaming
    /// fast paths covered by the CLI-level tests in `tests/jq_cli_tests.rs`.
    #[test]
    fn test_generic_lazy_index_range_collect_owned() {
        let json = br"[10, 20, 30]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::KeysUnsorted), value);
        assert!(matches!(result, GenericResult::LazyIndexRange(3)));

        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(0),
                OwnedValue::Int(1),
                OwnedValue::Int(2)
            ])]
        );

        let empty_json = br"[]";
        let empty_index = JsonIndex::build(empty_json);
        let empty_cursor = empty_index.root(empty_json);
        let empty_result = eval(&Expr::Builtin(Builtin::KeysUnsorted), empty_cursor.value());
        assert_eq!(
            empty_result.collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![])]
        );
    }

    /// #789: embedding `LazySeq<V>` directly in `GenericResult::LazySeq`
    /// grew the enum from 120 to 184 bytes (`size_of_val`, x86_64) -- every
    /// arm pays the widest variant's size on every return, so `select`'s own
    /// trivial boolean-test result got 64 bytes heavier to copy on every
    /// call, a flat per-call cost matching the issue's measured "flat ~6-7%
    /// slower across all sizes on AMD Ryzen 9 7950X" regression exactly.
    /// Boxing the variant recovered the original 120 bytes. Pinned here as a
    /// permanent regression guard against a future variant doing the same
    /// thing silently -- Rust enum size is easy to grow by accident one
    /// field at a time with no compiler warning. Exact-value assertion,
    /// gated to 64-bit, matches this crate's existing size-guard convention
    /// (`error::tests::eval_error_size_is_pinned_for_the_1021_stack_overflow_fix`);
    /// 32-bit targets shrink pointer-sized fields, so this exact count
    /// doesn't hold there.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_generic_result_size_stays_bounded_789() {
        assert_eq!(
            core::mem::size_of::<GenericResult<crate::json::light::StandardJson<'_, Vec<u64>>>>(),
            120,
            "GenericResult<V>'s size regressed (was 120 pre-#740, 184 with the #789 \
             regression, 120 again after boxing LazySeq) -- every variant now pays this on \
             every return; investigate before accepting a bigger enum"
        );
    }

    /// #789 (code review follow-up): `GenericItem<V>`'s own `LazySeq`
    /// variant had the identical unboxed-widest-variant defect as
    /// `GenericResult`'s -- and on a hotter path, since `GenericItem` is
    /// pushed once per *item* through every sink in the streaming path
    /// (`push_one_generic`, `each_*_iterate_sink`), not once per pipe
    /// stage. Measured at 184 bytes pre-fix (identical to raw `LazySeq<V>`
    /// itself), 80 bytes after boxing. Pinned as the `GenericItem` twin of
    /// [`test_generic_result_size_stays_bounded_789`] above, same
    /// exact-value-plus-64-bit-gate convention.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_generic_item_size_stays_bounded_789() {
        assert_eq!(
            core::mem::size_of::<GenericItem<crate::json::light::StandardJson<'_, Vec<u64>>>>(),
            80,
            "GenericItem<V>'s size regressed (was 184 with the #789 defect, 80 after \
             boxing LazySeq) -- every variant now pays this on every push; investigate \
             before accepting a bigger enum"
        );
    }

    /// #789's own fix boxed `GenericResult`/`GenericItem`'s `LazySeq`
    /// variant, but left `LazySeq<V>` itself unboxed and just as wide as
    /// before (184 bytes for `StandardJson`, 200 for `YamlValue`, since
    /// YAML's field-cursor type is bigger than JSON's). #1973 traced this to
    /// `LazySource::Keys(DistinctKeyCursors<V::Fields>)` -- `DistinctKeyCursors`
    /// carried two full field-cursor copies (`rest`/`all`) plus the rare-path
    /// `seen`/`collapsed` fields for #1514's duplicate-key collapse, unboxed,
    /// on every `LazySeq` regardless of whether that document ever hits the
    /// collapse path. Boxing `collapsed` (#1973) shrank this to 184 bytes --
    /// `seen` deliberately stays unboxed (see its own doc comment: boxing it
    /// would force an allocation on every jq-mode walk, not just the ones
    /// that collapse), and `rest`/`all`'s own two full cursor copies remain
    /// unboxed and out of scope here too (a materially larger change; see
    /// #1973's own "why deferred" section) -- so this isn't the struct's
    /// floor, just its next pinned value. Pinned on the YAML instantiation
    /// since it's the larger (worse-case) of the two document kinds.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_lazyseq_size_is_pinned_1973() {
        assert_eq!(
            core::mem::size_of::<LazySeq<crate::yaml::YamlValue<'_, Vec<u64>>>>(),
            216,
            "LazySeq<YamlValue>'s size changed -- if it grew, a new unboxed-wide field \
             snuck onto LazySeq/LazySource/DistinctKeyCursors (investigate before \
             accepting); if it shrank, DistinctKeyCursors shrank further -- update this \
             pinned value to match. 184 -> 216 (#2261) is accounted for: \
             `DistinctKeyCursors::last_key_cursor` below grew by one cursor's worth."
        );
    }

    /// The YAML twin of [`test_lazyseq_size_is_pinned_1973`] for
    /// `DistinctKeyCursors<YamlFields>` directly -- the actual struct #1973
    /// identifies as the real driver of `LazySeq<V>`'s size, not just the
    /// enum variant wrapping it. `collapsed: Option<Box<Vec<_>>>` (#1973)
    /// replaces its previously unboxed form, shrinking this from 168 to 152
    /// bytes -- already niche-optimized as `Option<Vec<_>>`, so boxing
    /// removes only the `Vec`'s own size from every walk that never
    /// collapses, at the cost of one extra allocation on the already-rare
    /// confirmed-collapse path. `seen` deliberately stays unboxed (its own
    /// doc comment has the full reasoning: boxing it would allocate on
    /// every jq-mode walk, not just the rare ones that collapse -- caught
    /// in code review before this landed). `rest`/`all`'s own two full
    /// field-cursor copies remain unboxed and out of scope here too
    /// (#1973's own "why deferred" section).
    ///
    /// 152 -> 184 (#2261): `last_key_cursor: Option<F::Cursor>` is a new
    /// field, deliberately unboxed and updated on *every* non-collapsed
    /// iteration (the #2261 trailing-comma-after-real-last-key check needs
    /// it once the walk exhausts) -- the same "common path, not rare path"
    /// reasoning `seen` above already established, not the mistake this
    /// test exists to catch. Reviewed and accepted, not merely updated to
    /// match.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn test_distinct_key_cursors_size_is_pinned_1973() {
        assert_eq!(
            core::mem::size_of::<
                crate::jq::document::DistinctKeyCursors<crate::yaml::YamlFields<'_, Vec<u64>>>,
            >(),
            184,
            "DistinctKeyCursors<YamlFields>'s size changed -- if it grew, a new field \
             landed unboxed on the rare-path (seen/collapsed) or copied cursor (rest/all) \
             members (investigate before accepting); if it shrank, update this pinned \
             value to match"
        );
    }

    /// `GenericResult::produces_output()`'s exhaustive match (added after
    /// #791's `Halt` variant was once missed by a hand-maintained exclusion
    /// list, see the method's own doc comment): four of its `true` arms —
    /// `One`, `LazyIndexRange`, `LazySeq`, `Owned` — needed direct coverage
    /// distinct from the `OneCursor`/`LazyKeys`/`Error`/`Partial` siblings
    /// they share a source line with, which other tests already reach.
    #[test]
    fn test_produces_output_covers_one_lazy_index_range_lazyseq_owned() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let one = eval(&Expr::Identity, cursor.value());
        assert!(matches!(one, GenericResult::One(_)));
        assert!(one.produces_output());

        let lazy_index_range = eval(&Expr::Builtin(Builtin::KeysUnsorted), cursor.value());
        assert!(matches!(lazy_index_range, GenericResult::LazyIndexRange(3)));
        assert!(lazy_index_range.produces_output());

        let map_expr = parse("map(. + 1)").unwrap();
        let lazy_seq = eval(&map_expr, cursor.value());
        assert!(matches!(lazy_seq, GenericResult::LazySeq(_)));
        assert!(lazy_seq.produces_output());

        let owned_expr = parse(".[0] + 1").unwrap();
        let owned = eval(&owned_expr, cursor.value());
        assert!(matches!(owned, GenericResult::Owned(_)));
        assert!(owned.produces_output());

        // `Self::Many(vs) => !vs.is_empty()` -- its own line, not part of the
        // `true`-arm OR-pattern above. `select`'s `pass_n` closure
        // constructs a bare (cursor-less) `GenericResult::Many` when its
        // condition yields more than one truthy output and `eval()`'s
        // top-level entry point always calls in with `cursor: None`.
        let select_expr = parse("select(true, true)").unwrap();
        let many = eval(&select_expr, cursor.value());
        assert!(matches!(many, GenericResult::Many(ref vs) if vs.len() == 2));
        assert!(many.produces_output());
    }

    /// `GenericResult::collect_owned()`'s `Self::Halt(_) => vec![]` arm: a
    /// bare halt collects as no outputs, the same as `Break`/`Error` right
    /// above it, rather than being folded in as a value.
    #[test]
    fn test_collect_owned_treats_halt_as_no_output() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let halt_expr = parse("halt_error(9)").unwrap();
        let halted = eval(&halt_expr, cursor.value());
        assert!(matches!(halted, GenericResult::Halt(9)));
        assert_eq!(halted.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    /// `eval_index_expr`'s `keys` match (#694): a `Partial`'s trailing
    /// control was silently dropped there, keeping only its prefix instead
    /// of propagating the error. Confirmed against real jq 1.7.1:
    /// `.[("a", error("boom"))]` on `{"a":1,"b":2}` errors with "boom".
    #[test]
    fn test_generic_computed_index_read_propagates_partial_error_694() {
        let json = br#"{"a":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = parse(r#".[("a", error("boom"))]"#).unwrap();
        let result = eval_using::<JqSemantics, _>(&expr, value);
        if let GenericResult::Error(e) | GenericResult::Partial(_, Control::Error(e)) = result {
            assert_eq!(e.message, "boom");
        } else {
            panic!("unexpected result: {result:?}");
        }
    }

    /// `eval_index_expr`'s `keys` match, `Partial`+`Break` sibling to the
    /// `Partial`+`Error` case above (#694): a `break` after some keys have
    /// already streamed collapses to the bare control instead of resolving
    /// those keys against the target. Confirmed against real jq 1.7.1:
    /// `label $out | .[("a", break $out)]` on `{"a":1,"b":2}` breaks out of
    /// the label rather than indexing with `"a"`.
    #[test]
    fn test_generic_computed_index_read_propagates_partial_break_694() {
        let json = br#"{"a":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = parse(r#".[("a", break $out)]"#).unwrap();
        let result = eval_using::<JqSemantics, _>(&expr, value);
        assert!(matches!(result, GenericResult::Break(ref label) if label == "out"));
    }

    /// `GenericResult::LazyIndexRange`'s `stream_json`/`stream_yaml` (#684).
    /// Unreachable via the yq CLI today -- `can_use_m2_streaming`'s
    /// whitelist excludes `Builtin::Keys`/`Builtin::KeysUnsorted`, same as
    /// `LazyKeys`'s sibling fallback arm -- so this exercises the writer
    /// directly, covering both the compact/flow and indented/block styles
    /// plus the empty-array short circuit.
    #[test]
    fn test_generic_lazy_index_range_stream_json_and_yaml() {
        let json = br"[10, 20, 30]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&Expr::Builtin(Builtin::KeysUnsorted), value);
        assert!(matches!(result, GenericResult::LazyIndexRange(3)));

        let mut compact_json = String::new();
        result
            .stream_json(
                &mut compact_json,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(compact_json, "[0,1,2]");

        let mut indented_json = String::new();
        result
            .stream_json(
                &mut indented_json,
                IndentSpec::spaces(2),
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(indented_json, "[\n  0,\n  1,\n  2\n]");

        let mut flow_yaml = String::new();
        result
            .stream_yaml(&mut flow_yaml, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(flow_yaml, "[0, 1, 2]");

        let mut block_yaml = String::new();
        result
            .stream_yaml(&mut block_yaml, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert_eq!(block_yaml, "- 0\n- 1\n- 2");

        let empty_json = br"[]";
        let empty_index = JsonIndex::build(empty_json);
        let empty_cursor = empty_index.root(empty_json);
        let empty_result = eval(&Expr::Builtin(Builtin::KeysUnsorted), empty_cursor.value());

        let mut empty_json_out = String::new();
        empty_result
            .stream_json(
                &mut empty_json_out,
                IndentSpec::spaces(2),
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(empty_json_out, "[]");

        let mut empty_yaml_out = String::new();
        empty_result
            .stream_yaml(
                &mut empty_yaml_out,
                IndentSpec::spaces(2),
                false,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(empty_yaml_out, "[]");
    }

    /// `eval_pipe_generic`'s `GenericResult::Many(vs)` stage arm's own
    /// `LazyIndexRange` sub-arm (#684): each of `select`'s two cursor-less
    /// truthy outputs (an array, so a bare `Many` rather than `ManyCursor` --
    /// see `test_json_multi_stage_pipe_first_stage_bare_many_without_cursor`
    /// above) is piped independently into `keys_unsorted`, which resolves to
    /// `LazyIndexRange` per element and must be materialized before folding
    /// into the accumulated `ManyOwned` result.
    #[test]
    fn test_json_multi_stage_pipe_first_stage_bare_many_lazy_index_range_684() {
        let json = br"[10, 20]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(
            &crate::jq::parse("select(true,true) | keys_unsorted").unwrap(),
            value,
        );
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![
                OwnedValue::Array(vec![OwnedValue::Int(0), OwnedValue::Int(1)]),
                OwnedValue::Array(vec![OwnedValue::Int(0), OwnedValue::Int(1)]),
            ]
        );
    }

    #[test]
    fn test_generic_type() {
        let json = br#"{"name": "Alice"}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::Type), value);
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(owned, OwnedValue::String("object".to_string()));
    }

    #[test]
    fn test_generic_tostring_overflow_literal_renders_correctly() {
        // Mirrors eval.rs's test_number_literal_overflow_renders_correctly_not_garbage
        // (#561, #1075): the generic evaluator's ToString arm had the same
        // bug, first as raw-text-reformatting garbage, then as Rust's own
        // `f64::Display` ("inf") instead of jq's `DBL_MAX`-text substitution.
        let json = br"1e400";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::ToString), value);
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(
            owned,
            OwnedValue::String("1.7976931348623157e+308".to_string())
        );
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
        // overflowed literal before `eval.rs`'s `@uri` formatting ever saw it
        // (#561), then rendering Rust's own `f64::Display` ("inf") instead of
        // jq's `DBL_MAX`-text substitution (#1075). This exercises that
        // bridge directly, independent of the CLI.
        let json = br"1e400";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Format(FormatType::Uri), value);
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(
            owned,
            OwnedValue::String("1.7976931348623157e%2B308".to_string())
        );
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
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(
            owned,
            OwnedValue::String("-1.7976931348623157e%2B308".to_string())
        );
    }

    #[test]
    fn test_generic_length() {
        let json = br"[1, 2, 3, 4, 5]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::Length), value);
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(owned, OwnedValue::Int(5));
    }

    #[test]
    fn test_generic_keys() {
        let json = br#"{"b": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::KeysUnsorted), value);
        let owned = result.into_owned().unwrap().unwrap();

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
    fn test_generic_keys_unsorted_lazy_length() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(3));
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_length_through_parens() {
        // Regression: the `Pipe` fast path must unwrap `(...)` so
        // `keys_unsorted | (length)` hits the same lazy path as
        // `keys_unsorted | length`, not the materialize fallback.
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | (length)").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(3));
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_iterate() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        let result = eval(&expr, value);
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![
                OwnedValue::String("b".to_string()),
                OwnedValue::String("a".to_string()),
                OwnedValue::String("c".to_string()),
            ]
        );
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_iterate_empty_object() {
        let json = br"{}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_index() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | .[-1]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("c".to_string())
        );

        // Out of bounds is `null`, never an error (#307), matching plain
        // array indexing.
        let expr = crate::jq::parse("keys_unsorted | .[10]").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Null
        );
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_first_last() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String("c".to_string())
        );
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_first_last_empty_object() {
        let json = br"{}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Null
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Null
        );
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_index_then_continue() {
        // The fast path must produce a real cursor, not just a bare value
        // -- downstream operations (`ascii_upcase` here) need to keep
        // working after `.[]`/`.[0]` on a lazy keys array.
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[] | ascii_upcase").unwrap();
        let result = eval(&expr, value.clone());
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![
                OwnedValue::String("B".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("C".to_string()),
            ]
        );

        let expr = crate::jq::parse("keys_unsorted | .[0] | ascii_upcase").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String("B".to_string())
        );
    }

    #[test]
    fn test_generic_keys_unsorted_map_stays_lazy_724() {
        // `keys_unsorted | map(f)` now takes the `LazySeq` fast path (#724)
        // instead of the `to_owned`->reserialize->reindex->re-evaluate
        // fallback -- assert the intermediate shape *before* materializing,
        // then confirm materializing still produces the same values the old
        // fallback-pinning version of this test checked.
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | map(ascii_upcase)").unwrap();
        let result = eval(&expr, value.clone());
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("B".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("C".to_string()),
            ])
        );

        // `select` gets no dedicated lazy arm by design (it materializes
        // once via the composability arm's `_` fallback, then runs through
        // the already-correct `Builtin::Select`) -- still correct, still one
        // pass instead of the four-pass round trip.
        let expr = crate::jq::parse("keys_unsorted | select(length == 3)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("b".to_string()),
                OwnedValue::String("a".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_large_object() {
        // No allocation-count assertion here (that's covered by the A/B
        // memory measurement) -- just correctness at a size well past any
        // small-N special case.
        let mut json = String::from("{");
        for i in 0..10_000 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(r#""k{i}":{i}"#));
        }
        json.push('}');
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(10_000)
        );

        let expr = crate::jq::parse("keys_unsorted | .[9999]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("k9999".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String("k9999".to_string())
        );
    }

    #[test]
    fn test_generic_keys_sorted_lazy_length() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys | length").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(3));
    }

    #[test]
    fn test_generic_keys_sorted_lazy_length_through_parens() {
        // Regression: the `Pipe` fast path must unwrap `(...)` for `keys`
        // too, same as it already does for `keys_unsorted`.
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys | (length)").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(3));
    }

    #[test]
    fn test_generic_keys_sorted_lazy_length_empty_object() {
        let json = br"{}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys | length").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(0)
        );
    }

    #[test]
    fn test_generic_keys_sorted_still_fully_sorted() {
        // The regression guard: `sorted: true` must suppress the raw
        // document-order fast paths that `keys_unsorted` uses for `.[]`,
        // `.[n]`, `first`, and `last`, falling through to materialize+sort
        // instead. If the `if !sorted` guard on the `Pipe` dispatch is ever
        // dropped, these assertions fail (they'd start seeing document
        // order `b,a,c` instead of sorted order `a,b,c`).
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );

        let expr = crate::jq::parse("keys | .[]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).collect_owned().unwrap(),
            vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ]
        );

        let expr = crate::jq::parse("keys | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("a".to_string())
        );

        let expr = crate::jq::parse("keys | .[-1]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("c".to_string())
        );

        let expr = crate::jq::parse("keys | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("a".to_string())
        );

        let expr = crate::jq::parse("keys | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String("c".to_string())
        );
    }

    #[test]
    fn test_generic_keys_sorted_map_select_stays_eager() {
        // Sorted `keys | map/select` deliberately stays on the eager
        // fallback (#724 doesn't change this): sorting requires observing
        // every key before emitting the first one, a different complexity
        // class than the `!sorted` guard's document-order fast path -- see
        // the non-goals in docs/plan/jq-lazy-map-select.md. Confirm the
        // guard actually excludes this: `keys` (sorted) must NOT produce a
        // `LazySeq`.
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys | map(ascii_upcase)").unwrap();
        let result = eval(&expr, value.clone());
        assert!(!matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("A".to_string()),
                OwnedValue::String("B".to_string()),
                OwnedValue::String("C".to_string()),
            ])
        );

        let expr = crate::jq::parse("keys | select(length == 3)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );
    }

    #[test]
    fn test_generic_keys_sorted_lazy_large_object() {
        // No allocation-count assertion here either -- that's the A/B's job
        // (see `test_generic_keys_unsorted_lazy_large_object` above).
        let mut json = String::from("{");
        for i in 0..10_000 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&format!(r#""k{i}":{i}"#));
        }
        json.push('}');
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let value = cursor.value();

        let expr = crate::jq::parse("keys | length").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(10_000)
        );

        // Derive the expected lexicographically-first key the same way the
        // input was generated, rather than hand-computing string order.
        let mut expected_keys: Vec<String> = (0..10_000).map(|i| format!("k{i}")).collect();
        expected_keys.sort();

        let expr = crate::jq::parse("keys | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String(expected_keys[0].clone())
        );
    }
    #[test]
    fn test_generic_array_keys_and_keys_unsorted_bare() {
        // `keys` and `keys_unsorted` on an array are identical -- the index
        // range `[0, 1, ..., len-1]` is already sorted -- and both must
        // still materialize to a plain `OwnedValue::Array` of ints when
        // there's no further pipe stage to hit a fast path.
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expected = OwnedValue::Array(vec![
            OwnedValue::Int(0),
            OwnedValue::Int(1),
            OwnedValue::Int(2),
        ]);

        let expr = crate::jq::parse("keys").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            expected
        );

        let expr = crate::jq::parse("keys_unsorted").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap().unwrap(), expected);
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_length() {
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(3)
        );
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_length_through_parens() {
        // Same regression coverage as the object case: the `Pipe` fast path
        // must unwrap `(...)`.
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | (length)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(3)
        );
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_iterate() {
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        let result = eval(&expr, value);
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(0), OwnedValue::Int(1), OwnedValue::Int(2)]
        );
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_iterate_empty_array() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_index() {
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | .[-1]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(2)
        );

        // Out of bounds is `null`, never an error (#307), matching plain
        // array indexing and the object `keys_unsorted` fast path.
        let expr = crate::jq::parse("keys_unsorted | .[10]").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Null
        );
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_first_last() {
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(2)
        );
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_first_last_empty_array() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Null
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Null
        );
    }

    #[test]
    fn test_generic_array_keys_unsorted_map_stays_lazy_724() {
        // Array counterpart of `test_generic_keys_unsorted_map_stays_lazy_724`
        // above: `keys_unsorted | map(f)` on an array's synthetic index
        // range also takes the `LazySeq` fast path (#724).
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | map(. * 10)").unwrap();
        let result = eval(&expr, value.clone());
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(0),
                OwnedValue::Int(10),
                OwnedValue::Int(20),
            ])
        );

        let expr = crate::jq::parse("keys_unsorted | select(length == 3)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(0),
                OwnedValue::Int(1),
                OwnedValue::Int(2),
            ])
        );
    }

    #[test]
    fn test_generic_plain_array_map_stays_lazy_725() {
        // Slice 2 (#725): `Builtin::Map`'s first-ever native arm -- plain
        // `arr | map(f)`, no `keys_unsorted` involved at all.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(. * 2)").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(4),
                OwnedValue::Int(6),
            ])
        );
    }

    #[test]
    fn test_generic_plain_object_map_stays_lazy_725() {
        // `obj | map(f)` is `[.[] | f]`; `.[]` over an object iterates its
        // *values* (#422), matching `eval.rs`'s `builtin_map`.
        let json = br#"{"a":1,"b":2,"c":3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(. * 2)").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(4),
                OwnedValue::Int(6),
            ])
        );
    }

    #[test]
    fn test_generic_plain_map_empty_containers_725() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse("map(.)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![])
        );

        let json = br"{}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse("map(.)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![])
        );
    }

    #[test]
    fn test_generic_plain_map_non_container_errors_725() {
        // Message must match `eval.rs`'s `builtin_map`/`map_over` exactly
        // (both dispatch through `EvalError::cannot_iterate_with`).
        let json = br"1";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(.)").unwrap();
        let result = eval(&expr, value.clone());
        assert!(result.is_error());

        let owned = crate::jq::eval_generic::to_owned(&value).unwrap();
        let expected = EvalError::cannot_iterate_with(EvalTag::Jq, &owned);
        match result {
            GenericResult::Error(e) => assert_eq!(e.message, expected.message),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_generic_lazy_seq_optional_suppresses_map_error() {
        // Regression test: `map(f)?` must suppress an error raised inside
        // `f`, same as any other `try`/`?` boundary. Before the
        // `GenericResult::LazySeq` arm was added to `Expr::Optional`'s match,
        // evaluating `inner` (here, bare `Builtin::Map`) returned an
        // unmaterialized `LazySeq` that matched neither `Error`/`Break`/
        // `Partial` nor got forced -- it fell through the `other => other`
        // arm and escaped the `?` entirely, so the error only surfaced later
        // at whatever site finally pulled the `LazySeq`, well past this
        // `try`/`catch` boundary. Verified against real jq (`jq 1.7.1`):
        // `[1,2,"x"]|map(.+1)?` is empty, exit 0.
        let json = br#"[1,2,"x"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(. + 1)?").unwrap();
        let result = eval(&expr, value);
        assert!(!result.is_error());
        assert_eq!(result.into_owned().unwrap(), None);

        // Non-erroring case still returns the mapped array, not suppressed.
        let expr = crate::jq::parse("map(. + 1)").unwrap();
        let json_ok = br"[1,2,3]";
        let index_ok = JsonIndex::build(json_ok);
        let value_ok = index_ok.root(json_ok).value();
        assert_eq!(
            eval(&expr, value_ok).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(3),
                OwnedValue::Int(4),
            ])
        );

        // `keys_unsorted | map(f)?` -- Slice 1's composed chain -- suppresses
        // the same way.
        let json2 = br#"{"a":1,"b":2}"#;
        let index2 = JsonIndex::build(json2);
        let value2 = index2.root(json2).value();
        let expr2 =
            crate::jq::parse(r#"keys_unsorted | map(if . == "a" then error("x") else . end)?"#)
                .unwrap();
        let result2 = eval(&expr2, value2);
        assert!(!result2.is_error());
        assert_eq!(result2.into_owned().unwrap(), None);
    }

    #[test]
    fn test_generic_plain_map_atomicity_725() {
        // Real jq's array construction is all-or-nothing: `map`'s error
        // partway through discards the whole in-progress array, mirroring
        // `eval::map_over` (#725's `materialize_atomic`).
        let json = br#"[1,2,"x"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(. + 1)").unwrap();
        let result = eval(&expr, value.clone());
        // Errors only surface once pulled -- laziness means construction
        // alone can't have failed yet.
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert!(result.materialize_lazy().is_error());

        // Nothing streams to `out` for a failing `map` -- matches real jq's
        // own all-or-nothing output and this file's `#355` convention that
        // diagnostics never go to `out`.
        let expr = crate::jq::parse("map(. + 1)").unwrap();
        let result = eval(&expr, value);
        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "");
        assert!(stats.error.is_some());
        assert_eq!(stats.count, 0);
    }

    #[test]
    fn test_generic_lazy_seq_map_pipe_iterate_atomicity_725() {
        // Regression test: `.[]` piped after a plain `map(f)` must not leak
        // the already-succeeded prefix before `map`'s own atomic-construction
        // error -- `.[]` here iterates the array `map` already built, not
        // the raw source, so it inherits `map`'s atomicity. Verified against
        // real jq (`jq 1.7.1`): `[1,2,"x"]|map(.+1)|.[]` prints nothing to
        // stdout, only the diagnostic.
        let json = br#"[1,2,"x"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(. + 1) | .[]").unwrap();
        let result = eval(&expr, value.clone());
        assert!(result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());

        // Same check at the `stream_json` boundary the CLI actually uses:
        // nothing streams to `out` before the diagnostic.
        let expr = crate::jq::parse("map(. + 1) | .[]").unwrap();
        let result = eval(&expr, value);
        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "");
    }

    #[test]
    fn test_generic_lazy_seq_composability_map_map_725() {
        // `arr | map(f) | map(g)`: two pushed instructions on one `LazySeq`
        // -- composability without materializing between stages.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(. + 1) | map(. * 10)").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(20),
                OwnedValue::Int(30),
                OwnedValue::Int(40),
            ])
        );
    }

    #[test]
    fn test_generic_lazy_seq_composability_native_consumers_725() {
        // `length`, `.[]`, `first`, `.[0]` after a `map` all get a native,
        // single-forward-pass path in the composability arm (asserted by
        // shape, not just correctness): none of these materialize to
        // `Owned`/`Error` via the generic `_ => materialize_atomic()`
        // fallback in a way that would be indistinguishable from the
        // dedicated arms, so this mainly pins the returned *values*.
        let json = br#"["b","a","c"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(ascii_upcase) | length").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(3)
        );

        let expr = crate::jq::parse("map(ascii_upcase) | .[]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).collect_owned().unwrap(),
            vec![
                OwnedValue::String("B".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("C".to_string()),
            ]
        );

        let expr = crate::jq::parse("map(ascii_upcase) | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("B".to_string())
        );

        let expr = crate::jq::parse("map(ascii_upcase) | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("B".to_string())
        );

        // `.[2]`/`last` intentionally fall to `materialize_atomic` +
        // `eval_on_owned` in this initial design (open risk (c)) --
        // asserting correctness only, not laziness.
        let expr = crate::jq::parse("map(ascii_upcase) | .[2]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::String("C".to_string())
        );

        let expr = crate::jq::parse("map(ascii_upcase) | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String("C".to_string())
        );
    }

    #[test]
    fn test_generic_lazy_seq_first_after_map_skips_later_error_725() {
        // Accepted, deliberate divergence from real jq (see the
        // `Expr::Builtin(Builtin::First) | Expr::index(0)` arm's own doc
        // comment above `eval_single`'s `Pipe` fold): real jq's `map`
        // eagerly builds the whole array first, so `map(f)|first` errors if
        // *any* element fails, even ones past the first. `first`/`.[0]`'s
        // pull-one-and-stop fast path only evaluates what's needed, so a
        // failure on a later, un-pulled element is invisible here -- verified
        // against real jq (`jq 1.7.1`): `[1,2,"x"]|map(.+1)|first` errors
        // there (`string ("x") and number (1) cannot be added`) but succeeds
        // below.
        let json = br#"[1,2,"x"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(. + 1) | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(2)
        );

        let expr = crate::jq::parse("map(. + 1) | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(2)
        );
    }

    #[test]
    fn test_generic_first_pipe_iterate_after_lazy_stops_at_one_element_1565() {
        // Durable, non-flaky proxy for #1565's perf claim: a later element
        // that would *error* if evaluated proves it was never pulled, the
        // same technique `test_generic_lazy_seq_first_after_map_skips_later_error_725`
        // uses -- a wall-clock assertion would be flaky, this instead pins
        // the *shape* of laziness. Covers all three lazy sources
        // `fold_pipe_stages_sink` fans out over, plus the `keys` (sorted)
        // case, which routes through owned strings instead of cursors.

        // `LazySeq` (`map(f)`, #724/#725): `"x" + 1` on the second element
        // would error if `first`'s `.[] | ...` tail ever pulled past the
        // first mapped element.
        let json = br#"[1, "x", "y"]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("first(map(. + 1) | .[] | (. + 100))").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(102)
        );

        // `LazyIndexRange` (an array's own `keys`/`keys_unsorted`, #684):
        // `error` on index `2` would fire if the iterate-after-`keys` tail
        // ever pulled past the first index.
        let json = br"[10, 20, 30]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr =
            crate::jq::parse(r#"first(keys | .[] | (if . == 2 then error("touched") else . end))"#)
                .unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(0)
        );

        // `LazyKeys { sorted: false }` (`keys_unsorted`): document order is
        // `"z"`, `"a"` -- `error` on `"a"` would fire if the tail pulled
        // past the first (document-order-first) key.
        let json = br#"{"z": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse(
            r#"first(keys_unsorted | .[] | (if . == "a" then error("touched") else . end))"#,
        )
        .unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String("z".to_string())
        );

        // `LazyKeys { sorted: true }` (`keys`): lexicographic order is
        // `"a"`, `"z"` -- `error` on `"z"` would fire if the tail pulled
        // past the first (lexicographically-first) key. Exercises the
        // owned-string element path (`keys` doesn't preserve a cursor for
        // sorted keys, matching `materialize_lazy_keys`'s existing
        // behaviour), not the cursor path the other three cases exercise.
        let json = br#"{"z": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse(
            r#"first(keys | .[] | (if . == "z" then error("touched") else . end))"#,
        )
        .unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::String("a".to_string())
        );
    }

    #[test]
    fn test_generic_lazy_seq_composability_keys_unsorted_map_select_724() {
        // The actual point of this design: `keys_unsorted | map(f) | select(g)`
        // stays lazy through the `map` stage, materializes once at `select`.
        let json = br#"{"bb": 1, "a": 2, "ccc": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr =
            crate::jq::parse("keys_unsorted | map(ascii_upcase) | select(length == 3)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("BB".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("CCC".to_string()),
            ])
        );
    }

    #[test]
    fn test_generic_lazy_seq_map_atomicity_extends_through_iterate_724() {
        // `map`'s array construction is atomic in real jq
        // (`[1,2,"x"]|map(.+1)` prints nothing on error, not a truncated
        // prefix) -- and `.[]` piped after `map` iterates the array `map`
        // already built, not the raw source, so that same atomicity applies
        // whether or not a `.[]` follows. Verified against real jq
        // (`jq 1.7.1`): `{"a":1,"b":2,"c":3}|keys_unsorted|map(if .=="b" then
        // error("boom") else . end)|.[]` prints nothing to stdout, only the
        // diagnostic -- even though `"a"` (document order's first key)
        // already succeeded before `"b"` failed.
        let json = br#"{"a":1,"b":2,"c":3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr =
            crate::jq::parse(r#"keys_unsorted | map(if . == "b" then error("boom") else . end)"#)
                .unwrap();
        let result = eval(&expr, value.clone());
        assert!(matches!(result, GenericResult::LazySeq(_)));
        // No partial prefix, even though `"a"` already succeeded.
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
        assert!(eval(&expr, value.clone()).materialize_lazy().is_error());

        let expr = crate::jq::parse(
            r#"keys_unsorted | map(if . == "b" then error("boom") else . end) | .[]"#,
        )
        .unwrap();
        let result = eval(&expr, value);
        // Same atomicity boundary as the `map`-alone case above: `"a"` does
        // NOT survive as a partial prefix once `.[]` is piped after `map`.
        assert!(result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    /// Known, narrow, pre-existing gap (not a new regression from #724):
    /// `collect_owned()` already silently swallows any error into an empty
    /// `Vec` for every variant that can error -- a failing `LazySeq` reached
    /// through a computed index just adds one more path into that same
    /// accepted lossy contract. Documented here, not fixed.
    #[test]
    fn test_generic_lazy_seq_computed_index_swallows_error_724() {
        let json = br#"{"a":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr =
            crate::jq::parse(r#"(keys_unsorted | map(error("x")))[("a" | length - 1)]"#).unwrap();
        let result = eval(&expr, value);
        assert!(!result.is_error());
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_large_array() {
        // No allocation-count assertion here (that's covered by the A/B
        // memory measurement) -- just correctness at a size well past any
        // small-N special case, mirroring the object equivalent above.
        let mut json = String::from("[");
        for i in 0..10_000 {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&i.to_string());
        }
        json.push(']');
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(10_000)
        );

        let expr = crate::jq::parse("keys_unsorted | .[9999]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Int(9999)
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Int(9999)
        );
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
            Expr::index(0),
            Expr::Field("name".to_string()),
        ]);

        let result = eval(&expr, value);
        let owned = result.into_owned().unwrap().unwrap();

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
        let owned = result.into_owned().unwrap().unwrap();

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

    /// #747: mirrors `test_json_multi_stage_pipe_first_stage_bare_many_lazy_index_range_684`'s
    /// shape — `select(true,true)` on a cursor-less top-level `eval()` call
    /// produces a bare `GenericResult::Many` (`Builtin::Select`'s `pass_n`
    /// closure, `(n, None)` arm), whose per-item re-evaluation in the pipe
    /// stage's `Many(vs)` arm goes through `to_owned_cursor` when the
    /// per-item result comes back as `OneCursor` (`.a`, single field) or
    /// `ManyCursor` (`.[]`, all fields) — exercising the exact two sub-arms
    /// `to_owned_cursor` replaced `to_owned(&c.value())` in. Each duplicated
    /// per-item cursor must still resolve its own explicit tag correctly,
    /// not just the top-level query's own cursor.
    #[test]
    fn test_yaml_multi_stage_pipe_bare_many_per_item_cursor_resolves_tag_747() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: !!str 1\nb: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        let single_field = eval(
            &crate::jq::parse("select(true,true) | .a | type").unwrap(),
            value.clone(),
        );
        assert_eq!(
            single_field.collect_owned().unwrap(),
            vec![
                OwnedValue::String("string".to_string()),
                OwnedValue::String("string".to_string()),
            ]
        );

        let iterate_fields = eval(
            &crate::jq::parse("select(true,true) | .[] | type").unwrap(),
            value,
        );
        assert_eq!(
            iterate_fields.collect_owned().unwrap(),
            vec![
                OwnedValue::String("string".to_string()),
                OwnedValue::String("number".to_string()),
                OwnedValue::String("string".to_string()),
                OwnedValue::String("number".to_string()),
            ]
        );
    }

    /// #903 review round: `Builtin::Shuffle`'s array branch now materializes
    /// via `collect_cursors`/`to_owned_cursor` instead of
    /// `collect_values`/`to_owned`, the same fix as `to_entries`/`reverse`/
    /// `pivot`. An in-process unit test (rather than a CLI subprocess test,
    /// like the sibling cases in `tests/yq_cli_tests.rs`) because
    /// `cargo-llvm-cov`'s workspace report doesn't reliably attribute
    /// coverage back through `Command::new(env!("CARGO_BIN_EXE_succinctly"))`
    /// for this arm specifically, despite the CLI binary demonstrably taking
    /// it when run directly. Order isn't checked (`shuffle` permutes), only
    /// that every element still carries its own resolved type.
    ///
    /// `#[cfg(feature = "cli")]`: `Builtin::Shuffle`'s array-materializing
    /// arm this test exercises only exists under that feature (see its own
    /// `#[cfg(feature = "cli")]` a few hundred lines up) — CI's plain
    /// `cargo test --verbose` leg (no `cli`) hits the sibling
    /// `#[cfg(not(feature = "cli"))]` error arm instead, which isn't what
    /// this test is for.
    #[cfg(feature = "cli")]
    #[test]
    fn test_yaml_shuffle_resolves_explicit_tag_903() {
        use crate::yaml::YamlIndex;

        let yaml = b"a:\n  - !!str 1\n  - !!str 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        let result = eval(&crate::jq::parse(".a | shuffle").unwrap(), value);
        match result.into_owned().unwrap().unwrap() {
            OwnedValue::Array(items) => {
                assert_eq!(items.len(), 2);
                for item in &items {
                    assert_eq!(item.type_name(), "string", "{item:?}");
                }
            }
            other => panic!("expected array, got {other:?}"),
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
        let owned = result.into_owned().unwrap().unwrap();

        assert_eq!(owned, OwnedValue::String("Alice".to_string()));
    }

    #[test]
    fn test_yaml_generic_to_entries_duplicate_keys() {
        // Duplicate YAML mapping keys must survive `to_entries` unmerged,
        // matching real `yq` -- not collapse to the last occurrence via the
        // `to_owned()` fallback's `IndexMap` (#443).
        //
        // Explicitly `YqSemantics` since #1385: preservation is a property
        // of yq *mode*, not of the YAML format. The bare `eval` helper this
        // used to call is `JqSemantics`, which now collapses whatever format
        // it is handed -- no CLI reaches that pairing (`sjq` reads JSON,
        // `syq` evaluates under `YqSemantics`), but the library API allows
        // it, and the mode is what decides.
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\na: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        let result = eval_using::<YqSemantics, _>(&Expr::Builtin(Builtin::ToEntries), value);
        let owned = result.into_owned().unwrap().unwrap();

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

    /// #1385: the duplicate-key rule follows the evaluation *mode*, not the
    /// input format. The same YAML mapping that preserves under
    /// `YqSemantics` (the test above) collapses under `JqSemantics`.
    ///
    /// No CLI reaches this pairing -- `sjq` reads JSON and `syq` evaluates
    /// under `YqSemantics` -- but the library API exposes it, and pinning it
    /// is what stops the format gate from creeping back in: before #1385
    /// `DocumentFields::keys_dedup()` answered `false` for `YamlFields`
    /// whatever mode was asking, which is the violation ADR-0018 rule 2
    /// names.
    #[test]
    fn test_yaml_generic_to_entries_collapses_under_jq_semantics_1385() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\na: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        let result = eval_using::<JqSemantics, _>(&Expr::Builtin(Builtin::ToEntries), value);
        let owned = result.into_owned().unwrap().unwrap();

        let mut entry = IndexMap::new();
        entry.insert("key".to_string(), OwnedValue::String("a".to_string()));
        entry.insert("value".to_string(), OwnedValue::Int(2));
        assert_eq!(owned, OwnedValue::Array(vec![OwnedValue::Object(entry)]));
    }

    /// #1168: `Expr::Array` had no native `eval_single` arm, so wrapping a
    /// cursor-native, duplicate-key-preserving builtin (`to_entries`, #443
    /// above) in `[...]` fell to the wildcard fallback, which materializes
    /// the *whole document* into an `OwnedValue` first -- silently
    /// re-collapsing the very duplicates `to_entries` had just preserved.
    /// In-process (not just the CLI test in `tests/yq_cli_tests.rs`) for the
    /// same coverage-attribution reason as [`test_yaml_shuffle_resolves_explicit_tag_903`].
    #[test]
    fn test_yaml_generic_array_wrapped_to_entries_preserves_duplicate_keys_1168() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\na: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        // `YqSemantics` since #1385 -- see the #443 test above.
        let result = eval_using::<YqSemantics, _>(
            &Expr::Array(Box::new(Expr::Builtin(Builtin::ToEntries))),
            value,
        );
        let owned = result.into_owned().unwrap().unwrap();

        let expected_entry = |v: i64| {
            let mut entry = IndexMap::new();
            entry.insert("key".to_string(), OwnedValue::String("a".to_string()));
            entry.insert("value".to_string(), OwnedValue::Int(v));
            OwnedValue::Object(entry)
        };
        assert_eq!(
            owned,
            OwnedValue::Array(vec![OwnedValue::Array(vec![
                expected_entry(1),
                expected_entry(2)
            ])])
        );
    }

    /// #1168, comma sibling of the `Array` case above: `Expr::Comma` had no
    /// native arm either, so `to_entries, to_entries` hit the same
    /// whole-document-materializing fallback for both operands.
    #[test]
    fn test_yaml_generic_comma_wrapped_to_entries_preserves_duplicate_keys_1168() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\na: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);

        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        // `YqSemantics` since #1385 -- see the #443 test above.
        let result = eval_using::<YqSemantics, _>(
            &Expr::Comma(vec![
                Expr::Builtin(Builtin::ToEntries),
                Expr::Builtin(Builtin::ToEntries),
            ]),
            value,
        );
        let owned = result.collect_owned().unwrap();

        let expected_entry = |v: i64| {
            let mut entry = IndexMap::new();
            entry.insert("key".to_string(), OwnedValue::String("a".to_string()));
            entry.insert("value".to_string(), OwnedValue::Int(v));
            OwnedValue::Object(entry)
        };
        let expected_array = OwnedValue::Array(vec![expected_entry(1), expected_entry(2)]);
        assert_eq!(owned, vec![expected_array.clone(), expected_array]);
    }

    /// #1170: unlike YAML's genuine duplicates (above), a duplicate JSON
    /// key must collapse to one entry -- keeping the *first* occurrence's
    /// position but the *last* occurrence's value, matching real jq
    /// (`{"a":1,"b":2,"a":3}|to_entries` is `[{"key":"a","value":3},
    /// {"key":"b","value":2}]`, oracle-verified against jq 1.7.1).
    #[test]
    fn test_json_generic_to_entries_deduplicates_repeated_key_1170() {
        let json: &[u8] = br#"{"a":1,"b":2,"a":3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let result = eval(&Expr::Builtin(Builtin::ToEntries), value);
        let owned = result.into_owned().unwrap().unwrap();

        let entry = |k: &str, v: i64| {
            let mut entry = IndexMap::new();
            entry.insert("key".to_string(), OwnedValue::String(k.to_string()));
            entry.insert("value".to_string(), OwnedValue::Int(v));
            OwnedValue::Object(entry)
        };
        assert_eq!(owned, OwnedValue::Array(vec![entry("a", 3), entry("b", 2)]));
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
        let owned = result.into_owned().unwrap().unwrap();

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
    fn test_generic_optional_around_native_pipe_fanout_stops_at_the_first_error() {
        // #693: `Expr::Optional`'s own native arm (not the `eval_on_owned`/
        // wildcard bridge to `eval::eval`) used to force `optional = true`
        // down the whole wrapped subtree, so each element of the native
        // `Iterate` -> `Pipe` fan-out independently self-suppressed its own
        // error via the bridge's own `optional`-aware wrapping, and the
        // fan-out wrongly kept going past it. Confirmed reachable through
        // the actual `succinctly jq`/`yq` CLIs, which call this native path
        // (`eval_with_cursor`) by default, not just via `eval()` directly:
        // `echo '[1,2,3]' | succinctly jq '(.[] | if .==2 then
        // error("boom") else . end)?'` printed `1` and `3` pre-fix; real jq
        // 1.7.1 prints only `1`.
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse(r#"(.[] | if .==2 then error("boom") else . end)?"#).unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.collect_owned().unwrap(), vec![OwnedValue::Int(1)]);
    }

    #[test]
    fn test_generic_optional_around_native_pipe_fanout_error_on_first_element() {
        // Sibling to the test above: the error hits on the very *first*
        // fan-out element, so there's no prefix to collect at all — the
        // native `Many`/`ManyCursor` fan-out arms return a bare
        // `GenericResult::Error` directly rather than ever constructing a
        // `Partial`, so this lands on `Expr::Optional`'s
        // `GenericResult::Error(_) => GenericResult::None` arm
        // (eval_generic.rs), not the `Partial` arm below it. (There is no
        // `prefix.len() == 0` case to exercise: `partial_generic` collapses
        // an empty prefix to a bare `Error`/`Break` before `Partial` is ever
        // constructed, so that arm doesn't exist.) Confirmed against real jq
        // 1.7.1: `(.[] | if .==1 then error("boom") else . end)?` on
        // `[1,2,3]` prints nothing.
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse(r#"(.[] | if .==1 then error("boom") else . end)?"#).unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_generic_optional_around_native_pipe_fanout_multi_element_prefix() {
        // Sibling to the two tests above: the error hits after more than
        // one fan-out element has already succeeded, so the `Partial`
        // prefix holds multiple values. Exercises `Expr::Optional`'s
        // `prefix.len() > 1` arm (eval_generic.rs), which the one-element
        // case above can't reach. Confirmed against real jq 1.7.1:
        // `(.[] | if .==3 then error("boom") else . end)?` on `[1,2,3,4]`
        // prints `1` then `2`.
        let json = br"[1, 2, 3, 4]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse(r#"(.[] | if .==3 then error("boom") else . end)?"#).unwrap();
        let result = eval(&expr, value);
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(1), OwnedValue::Int(2)]
        );
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

        let result = eval(&Expr::index(1), value);
        let owned = result.into_owned().unwrap().unwrap();

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
        let owned = result.into_owned().unwrap().unwrap();

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
        let owned = result.into_owned().unwrap().unwrap();

        // Mapping starts at column 1
        assert_eq!(owned, OwnedValue::Int(1));
    }

    // Regression tests for #709: `anchor`/`style` had no cursor-aware arm at
    // all in this evaluator, so they always fell through to the OwnedValue
    // fallback (`eval_on_owned`) and lost YAML metadata even for direct,
    // un-navigated cursor access.

    #[test]
    fn test_yaml_anchor_builtin_with_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: &x 1\nb: *x\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".a | anchor").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String("x".to_string())
        );
    }

    #[test]
    fn test_yaml_anchor_builtin_empty_when_absent() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".a | anchor").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String(String::new())
        );
    }

    #[test]
    fn test_yaml_style_builtin_with_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: [1, 2, 3]\nb: \"quoted\"\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".a | style").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String("flow".to_string())
        );

        let expr = crate::jq::parse(".b | style").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String("double".to_string())
        );
    }

    #[test]
    fn test_yaml_anchor_style_without_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"&x [1, 2, 3]\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");
        let value = doc_cursor.value();

        // Using eval (not eval_with_cursor) loses anchor/style metadata,
        // same as `line`/`column` above.
        let result = eval(&Expr::Builtin(Builtin::Anchor), value.clone());
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String(String::new())
        );

        let result = eval(&Expr::Builtin(Builtin::Style), value);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String(String::new())
        );
    }

    // JSON has no anchor/style concept, so `JsonCursor` doesn't override
    // `DocumentCursor::anchor`/`style` and falls through to the trait's
    // default `None`/`""` impl (`document.rs`). The YAML tests above only
    // exercise the *overridden* impls in `yaml/light.rs` — these cover the
    // default itself, reached here via a real navigated cursor (not the
    // cursor-less fallback `test_yaml_anchor_style_without_cursor` covers).
    #[test]
    fn test_json_anchor_builtin_default_empty() {
        use crate::json::JsonIndex;

        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let doc_cursor = index.root(json);

        let expr = parse(".a | anchor").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String(String::new())
        );
    }

    #[test]
    fn test_json_style_builtin_default_empty() {
        use crate::json::JsonIndex;

        let json = br#"{"a": [1, 2, 3]}"#;
        let index = JsonIndex::build(json);
        let doc_cursor = index.root(json);

        let expr = parse(".a | style").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String(String::new())
        );
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
        let owned = result.into_owned().unwrap().unwrap();

        // Without cursor, line returns 0
        assert_eq!(owned, OwnedValue::Int(0));
    }

    // Tests for `line_comment` (#710): a getter for a node's trailing
    // same-line comment. Unlike `line`/`column`, the "not available"
    // default is `""`, matching real `yq` (verified empirically), not `0`.

    #[test]
    fn test_yaml_line_comment_builtin_with_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1 # keep this\nb: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".a | line_comment").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String("keep this".to_string())
        );
    }

    #[test]
    fn test_yaml_line_comment_builtin_no_comment() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1\nb: 2\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".a | line_comment").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String(String::new())
        );
    }

    #[test]
    fn test_yaml_line_comment_builtin_invalid_utf8_is_error_797() {
        use crate::yaml::YamlIndex;

        // "caf\xE9" - a comment with an invalid UTF-8 byte. Must surface as
        // an error, not silently collapse to "" as if there were no comment
        // at all (issue #797).
        let yaml = b"a: 1 # caf\xE9\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".a | line_comment").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert!(result.is_error());
    }

    #[test]
    fn test_yaml_line_comment_builtin_no_space_after_hash_keeps_hash() {
        use crate::yaml::YamlIndex;

        // No space after '#' means nothing to strip - the whole thing,
        // '#' included, is the comment text (verified against real yq).
        let yaml = b"a: 1 #keep this\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".a | line_comment").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String("#keep this".to_string())
        );
    }

    #[test]
    fn test_yaml_line_comment_without_cursor() {
        use crate::yaml::YamlIndex;

        let yaml = b"a: 1 # keep this\n";
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index.root(yaml);
        let mapping_cursor = cursor
            .first_child()
            .expect("YAML document should have content");
        let value = mapping_cursor.value();

        // Using eval (not eval_with_cursor) loses position metadata, so
        // line_comment falls back to "" even though the source has one.
        let result = eval(&Expr::Builtin(Builtin::LineComment), value);
        let owned = result.into_owned().unwrap().unwrap();
        assert_eq!(owned, OwnedValue::String(String::new()));
    }

    #[test]
    fn test_json_line_comment_is_always_empty() {
        // JSON has no comments; the DocumentCursor default (None) applies
        // unconditionally regardless of cursor presence.
        let json = b"{\"a\": 1}";
        let index = crate::json::JsonIndex::build(json);
        let cursor = index.root(json);
        let field_cursor = cursor
            .first_child()
            .expect("JSON object should have a field");

        let result = eval_with_cursor(&Expr::Builtin(Builtin::LineComment), field_cursor);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::String(String::new())
        );
    }

    /// `to_owned_with_comments` reads the raw (`#`-and-all) form via
    /// [`DocumentCursor::line_comment_raw`] (issue #710), not the stripped
    /// `line_comment` builtin getter the test above exercises. JSON never
    /// overrides `line_comment_raw` (only `YamlCursor` does), so this is the
    /// only route that reaches the trait's default `None` implementation for
    /// that method - unlike `line_comment`, which the plain `jq` `line_comment`
    /// builtin already exercises on JSON above.
    #[test]
    fn test_json_to_owned_with_comments_uses_line_comment_raw_default() {
        let json = b"{\"a\": 1}";
        let index = crate::json::JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let (owned, comments) = to_owned_with_comments(&value, Some(&cursor)).unwrap();
        assert_eq!(
            owned,
            OwnedValue::Object(IndexMap::from([("a".to_string(), OwnedValue::Int(1))]))
        );
        assert_eq!(comments.own(), None);
        assert_eq!(comments.field("a").own(), None);
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
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(2));
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
            result.into_owned().unwrap().unwrap(),
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
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3),
            ])
        );
    }

    #[test]
    fn test_yaml_keys_unsorted_lazy_fast_paths() {
        // `LazyKeys`/its `Pipe` fast paths are generic over
        // `V: DocumentValue`, so a YAML mapping goes through the exact same
        // evaluator arms as a JSON object (#140). The bare-array case (no
        // `Pipe` fast path applies) is covered separately by
        // `test_yaml_keys_unsorted_stream_lazy_685`, which now also streams
        // lazily at the CLI output boundary (#685).
        use crate::yaml::YamlIndex;

        let yaml = b"b: 1\na: 2\nc: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::Int(3)
        );

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).collect_owned().unwrap(),
            vec![
                OwnedValue::String("b".to_string()),
                OwnedValue::String("a".to_string()),
                OwnedValue::String("c".to_string()),
            ]
        );

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::String("c".to_string())
        );
    }

    #[test]
    fn test_yaml_keys_sorted_lazy_length() {
        // Sorted `keys | length` mirror of the `keys_unsorted` test above
        // (#683), proving the `Pipe` dispatch's `Length` fast path is
        // generic over `V: DocumentValue` for the sorted case too.
        use crate::yaml::YamlIndex;

        let yaml = b"b: 1\na: 2\nc: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse("keys | length").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::Int(3)
        );

        // Regression guard: bare `keys` and `keys | first`/`last` must
        // still be fully sorted (`a,b,c`), not document order (`b,a,c`).
        let expr = crate::jq::parse("keys").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );

        let expr = crate::jq::parse("keys | first").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::String("a".to_string())
        );

        let expr = crate::jq::parse("keys | last").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::String("c".to_string())
        );
    }

    #[test]
    fn test_yaml_array_keys_unsorted_lazy_fast_paths() {
        // `LazyIndexRange`/its `Pipe` fast paths are generic over
        // `V: DocumentValue` too (#684) -- a YAML sequence goes through the
        // exact same evaluator arms as a JSON array.
        use crate::yaml::YamlIndex;

        let yaml = b"- x\n- y\n- z\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::Int(3)
        );

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).collect_owned().unwrap(),
            vec![OwnedValue::Int(0), OwnedValue::Int(1), OwnedValue::Int(2)]
        );

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            OwnedValue::Int(2)
        );
    }

    #[test]
    fn test_yaml_keys_unsorted_stream_lazy_685() {
        // `GenericResult::stream_json`/`stream_yaml`'s `LazyKeys` arm
        // now writes each key straight from `fields` instead of falling back
        // to `materialize_lazy_keys` — this asserts the exact bytes produced,
        // for both output formats and both compact/indented modes (#685).
        use crate::yaml::YamlIndex;

        let yaml = b"b: 1\na: 2\nc: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse("keys_unsorted").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert!(matches!(
            result,
            GenericResult::LazyKeys { sorted: false, .. }
        ));

        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"["b","a","c"]"#);

        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::spaces(2),
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "[\n  \"b\",\n  \"a\",\n  \"c\"\n]");

        let mut out = String::new();
        result
            .stream_yaml(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "[b, a, c]");

        let mut out = String::new();
        result
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "- b\n- a\n- c");
    }

    #[test]
    fn test_yaml_keys_unsorted_stream_lazy_merge_key_685() {
        // Same as above but resolved through a `<<: *anchor` merge key,
        // exercising `YamlFields`'s `Merged` variant (an `Rc`-shared entry
        // list) through the new streaming path rather than the plain
        // cursor-walk `Direct` variant JSON's `JsonFields` always uses.
        use crate::yaml::YamlIndex;

        let yaml = b"defaults: &defaults\n  b: 1\n  a: 2\nitem:\n  <<: *defaults\n  c: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".item | keys_unsorted").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert!(matches!(
            result,
            GenericResult::LazyKeys { sorted: false, .. }
        ));

        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"["b","a","c"]"#);

        let mut out = String::new();
        result
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "- b\n- a\n- c");
    }

    #[test]
    fn test_yaml_keys_unsorted_map_stays_lazy_724() {
        // YAML counterpart of `test_generic_keys_unsorted_map_stays_lazy_724`
        // -- the `LazySeq`/`Pipe`-fold fast path is generic over
        // `V: DocumentValue`, so a YAML mapping goes through the exact same
        // evaluator arms as a JSON object.
        use crate::yaml::YamlIndex;

        let yaml = b"b: 1\na: 2\nc: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse("keys_unsorted | map(ascii_upcase)").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("B".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("C".to_string()),
            ])
        );
    }

    #[test]
    fn test_yaml_array_keys_unsorted_map_stays_lazy_724() {
        use crate::yaml::YamlIndex;

        let yaml = b"- x\n- y\n- z\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse("keys_unsorted | map(. * 10)").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(0),
                OwnedValue::Int(10),
                OwnedValue::Int(20),
            ])
        );
    }

    #[test]
    fn test_yaml_keys_unsorted_map_merge_key_lazy_724() {
        // Same as `test_yaml_keys_unsorted_map_stays_lazy_724` but resolved
        // through a `<<: *anchor` merge key, exercising `YamlFields`'s
        // `Merged` variant (an `Rc`-shared entry list, `Clone` but not
        // `Copy`) through `LazySource::Keys`'s forward-only `uncons()`
        // pulling, not just the plain cursor-walk `Direct` variant.
        use crate::yaml::YamlIndex;

        let yaml = b"defaults: &defaults\n  b: 1\n  a: 2\nitem:\n  <<: *defaults\n  c: 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse(".item | keys_unsorted | map(ascii_upcase)").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("B".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("C".to_string()),
            ])
        );
    }

    #[test]
    fn test_yaml_plain_map_stays_lazy_725() {
        // YAML counterpart of `test_generic_plain_array_map_stays_lazy_725`
        // -- plain `arr | map(f)` on YAML, no `keys_unsorted` involved.
        use crate::yaml::YamlIndex;

        let yaml = b"- 1\n- 2\n- 3\n";
        let index = YamlIndex::build(yaml).unwrap();
        let doc_cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");

        let expr = crate::jq::parse("map(. * 2)").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(4),
                OwnedValue::Int(6),
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
            result.into_owned().unwrap().unwrap(),
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
            result.into_owned().unwrap().unwrap(),
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
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(1));
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
            Expr::index(0),
            Expr::Field("name".to_string()),
        ]);

        let result = eval(&expr, value);
        let owned = result.into_owned().unwrap().unwrap();

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
        let owned = result.into_owned().unwrap().unwrap();

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
        assert_eq!(result1.into_owned().unwrap().unwrap(), OwnedValue::Int(0));

        let result2 = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), doc2);
        assert_eq!(result2.into_owned().unwrap().unwrap(), OwnedValue::Int(1));

        let result3 = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), doc3);
        assert_eq!(result3.into_owned().unwrap().unwrap(), OwnedValue::Int(2));
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
            assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(1));
        } else {
            // Just test the doc directly
            let result = eval_with_cursor(&Expr::Builtin(Builtin::DocumentIndex), doc2);
            assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(1));
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
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(0));
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
                    GenericResult::One(v) => results.push(to_owned(&v).unwrap()),
                    // `select`'s truthy branch now forwards the cursor it
                    // was given (needed for `line`/`column` to survive a
                    // `select(...)`), so a match here is `OneCursor`, not
                    // `One` — see the `Builtin::Select` cursor-forwarding
                    // fix in `eval_builtin`.
                    GenericResult::OneCursor(c) => results.push(to_owned_cursor(&c).unwrap()),
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
            result.collect_owned().unwrap(),
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

    #[test]
    fn test_json_select_yq_collapses_to_at_most_one_1613() {
        // yq-mode counterpart of the test just above: `Builtin::Select`'s
        // native arm here (not `eval.rs`'s `eval_fanout`) computes
        // `truthy_count` directly from the collected bits, so the fix is a
        // one-line change to that count rather than a per-bit closure -- but
        // the observable contract is identical: under
        // `S::SELECT_EMITS_ONCE_IF_ANY_TRUTHY`, two truthy elements forward
        // the outer cursor *once*, not twice. Live-verified against yq
        // v4.53.3.
        let json = b"[true,false,true]";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("select(.[])").unwrap();

        let result = eval_with_cursor_using::<YqSemantics, _>(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Bool(true),
                OwnedValue::Bool(false),
                OwnedValue::Bool(true)
            ])]
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
        let owned = result.into_owned().unwrap().unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));

        // at_offset(10) should be inside the "Alice" string (offset 10 = 'l' in "Alice")
        let expr = crate::jq::parse("at_offset(10)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap().unwrap();
        assert!(matches!(owned, OwnedValue::String(ref s) if s == "Alice"));

        // at_offset(27) should be the age number (30)
        let expr = crate::jq::parse("at_offset(27)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap().unwrap();
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
        let owned = result.into_owned().unwrap().unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));

        // at_position(2; 3) should be the "name" key (line 2, col 3 = start of "name")
        let expr = crate::jq::parse("at_position(2; 3)").unwrap();
        let result = eval_with_cursor(&expr, cursor);
        let owned = result.into_owned().unwrap().unwrap();
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
            eval_with_cursor(&via_getpath, cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            eval_with_cursor(&via_literal, cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
        );

        let via_getpath =
            crate::jq::parse(r#"at_position(getpath(["l"]); getpath(["c"]))"#).unwrap();
        let via_literal = crate::jq::parse("at_position(1; 1)").unwrap();
        assert_eq!(
            eval_with_cursor(&via_getpath, cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
            eval_with_cursor(&via_literal, cursor)
                .into_owned()
                .unwrap()
                .unwrap(),
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
        let owned = result.into_owned().unwrap().unwrap();
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
        let c2 = index.root(doc);
        let c3 = index.root(doc);
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
            // Appended at the end (#140, #683) so they don't disturb the
            // fixed positional indices every other assertion in this test
            // relies on: `LazyKeys` with `sorted: false` then `sorted:
            // true`, both exercised the same way as every other variant by
            // the `stream_json`/`stream_yaml`/`into_owned` loops below.
            eval(&Expr::Builtin(Builtin::KeysUnsorted), c2.value()),
            eval(&Expr::Builtin(Builtin::Keys), c3.value()),
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
            r.stream_json(
                &mut j,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
            let mut y = String::new();
            r.stream_yaml(&mut y, IndentSpec::spaces(2), false, |_| Ok(()))
                .unwrap();
        }
        // Spot-check the owned and error stream output.
        let mut owned_json = String::new();
        results[2]
            .stream_json(
                &mut owned_json,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(owned_json, "5");
        // An error writes nothing to `out` — `out` is stdout, and a diagnostic
        // there would be indistinguishable from a result. It comes back through
        // `stats.error` instead, for the caller to print to stderr (#355).
        let mut err_json = String::new();
        let err_stats = results[5]
            .stream_json(
                &mut err_json,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(err_json, "", "diagnostics must never reach stdout");
        assert_eq!(
            err_stats.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );
        assert_eq!(err_stats.count, 0);

        let mut err_yaml = String::new();
        let err_stats = results[5]
            .stream_yaml(&mut err_yaml, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert_eq!(err_yaml, "", "diagnostics must never reach stdout");
        assert_eq!(
            err_stats.error.as_ref().map(|e| e.message.as_str()),
            Some("boom")
        );

        // Break escapes its label: an uncaught error like any other, and it too
        // stays off stdout.
        let mut brk = String::new();
        let brk_stats = results[6]
            .stream_json(
                &mut brk,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
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
            .stream_json(
                &mut partial_json,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| {
                    seen += 1;
                    Ok(())
                },
            )
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
            .stream_yaml(&mut partial_yaml, IndentSpec::spaces(2), false, |w| {
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
            .stream_json(
                &mut partial_brk,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(partial_brk, "3");
        assert!(brk_stats
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("not in label")));
        let mut partial_brk_yaml = String::new();
        let brk_stats = results[8]
            .stream_yaml(&mut partial_brk_yaml, IndentSpec::spaces(2), false, |_| {
                Ok(())
            })
            .unwrap();
        assert_eq!(partial_brk_yaml, "3");
        assert!(brk_stats
            .error
            .as_ref()
            .is_some_and(|e| e.message.contains("not in label")));

        // into_owned consumes; check the owned-family variants.
        let owned: Vec<Option<OwnedValue>> = results
            .into_iter()
            .map(|r| r.into_owned().unwrap())
            .collect();
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
        assert_eq!(
            owned[9],
            Some(OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
            ]))
        ); // LazyKeys { sorted: false }
        assert_eq!(
            owned[10],
            Some(OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
            ]))
        ); // LazyKeys { sorted: true }
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
            .map(|r| r.collect_owned().unwrap())
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
        result
            .stream_json(
                &mut j,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(j, "1");
        let mut y = String::new();
        result
            .stream_yaml(&mut y, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();

        let result2 = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result2.collect_owned().unwrap(), vec![OwnedValue::Int(1)]);
    }

    #[test]
    fn test_json_cursor_stream_json_pretty_indent_1576() {
        // #1576: JsonCursor::stream_json now supports indented (pretty)
        // JSON->JSON cursor streaming too, exercised through the generic
        // OneCursor arm, same as the compact case above -- it no longer
        // falls back to the DOM path (#442) for this.
        let json = br#"{"a": {"b": 1}}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("at_offset(6)").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut j = String::new();
        result
            .stream_json(
                &mut j,
                IndentSpec::spaces(2),
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(j, "{\n  \"b\": 1\n}");
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
        result
            .stream_json(
                &mut j,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(j, "1");
        let mut y = String::new();
        result
            .stream_yaml(&mut y, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
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
        assert_eq!(jq_result.into_owned().unwrap(), Some(OwnedValue::Int(1)));

        let yq_result = eval_using::<YqSemantics, _>(&expr, index.root(json).value());
        assert_eq!(
            yq_result.into_owned().unwrap(),
            Some(OwnedValue::Float(1.5))
        );
    }

    #[test]
    fn test_yq_slice_scalar_empty_array_classifies_by_type_not_parseability_1065() {
        // `slice_one_generic`'s yq empty-container check (#1065) must
        // classify "is this a number" the same way `eval.rs`'s own
        // `StandardJson::Number(_)` match and `is_yq_slice_empty_container_scalar`
        // do: by variant, not by whether the raw text happens to parse as
        // i64/f64. `1.2.3` is a JSON number span the semi-index scanner
        // accepts leniently (#966) but that fails `as_i64`/`as_f64` parsing
        // — an earlier version of this check used exactly that parseability
        // test and returned an error here instead of `[]`, disagreeing with
        // the concrete evaluator on the identical input. Uses a computed
        // (non-literal-folding) slice so evaluation stays inside
        // `eval_generic.rs`'s own `slice_one_generic` rather than bridging
        // out to `eval.rs`, which is what let the divergence hide.
        use crate::jq::YqSemantics;

        let json = b"1.2.3";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".[(1-1):(1+0)]").unwrap();

        let result = eval_using::<YqSemantics, _>(&expr, index.root(json).value());
        assert_eq!(
            result.into_owned().unwrap(),
            Some(OwnedValue::Array(Vec::new()))
        );
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
            result.collect_owned().unwrap(),
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
    fn test_json_iterate_then_single_key_index_expr_yields_many_cursor() {
        // Fixed by #607: `index_one_generic` (behind computed indexing like
        // `.["a"]`) used to call `fields.find`/`elements.get` instead of the
        // `find_cursor`/`get_cursor` siblings `Expr::Field`/`Expr::Index`
        // already used, so a computed single-key index yielded a plain `One`
        // per element -- forcing the `ManyCursor` stage arm below to flatten
        // through its heterogeneous-`One` branch instead of staying
        // `ManyCursor`, which silently dropped duplicate keys inside the
        // selected value. It now yields `OneCursor` per element just like
        // `Expr::Field`/`Expr::Index`, so `all_single_cursor` holds and the
        // whole pipe stays `ManyCursor`. Built directly as `Expr::IndexExpr`
        // rather than parsed from `.["a"]`, since the parser folds a
        // literal-string bracket index into a plain `Expr::Field` (which
        // already took the all-`OneCursor` path before this fix).
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
        assert!(matches!(result, GenericResult::ManyCursor(_)));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(1), OwnedValue::Int(2)]
        );
    }

    #[test]
    fn test_json_first_expr_preserves_duplicate_keys_in_selected_element() {
        // #607's actual root cause for its own `first(.[])` repro:
        // `Expr::FirstExpr` (what `crate::jq::parse("first(...)")` builds --
        // distinct from the zero-arg `Builtin::First` bare-keyword form) had
        // no native arm in `eval_single`, so it fell through the catch-all
        // `to_owned()` bridge, which collapses duplicate keys in *every*
        // nested value -- including the selected element -- before `.[]`
        // even ran. Now handled natively via `eval_first_or_last_generic`,
        // forwarding whatever cursor `.[]`'s `ManyCursor` carries for its
        // first element. `stream_json` (not `collect_owned`, which itself
        // round-trips through `to_owned`) is the only way to observe this at
        // the unit level.
        let json = br#"[{"a":1,"a":2},{"b":3,"b":4}]"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("first(.[])").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"{"a":1,"a":2}"#);
    }

    #[test]
    fn test_json_last_expr_preserves_duplicate_keys_in_selected_element() {
        // Mirror of the `first(.[])` test above for `last(.[])`.
        let json = br#"[{"a":1,"a":2},{"b":3,"b":4}]"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("last(.[])").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"{"b":3,"b":4}"#);
    }

    #[test]
    fn test_json_first_stream_builtin_preserves_duplicate_keys() {
        // `Builtin::FirstStream`/`LastStream` is never constructed by the
        // parser (#1986): `first(f)`/`last(f)` always parse to
        // `Expr::FirstExpr`/`Expr::LastExpr` instead (see
        // `builtin_first_stream_propagates_bare_halt` in eval.rs) -- not
        // reachable from `crate::jq::parse` for any top-level user syntax,
        // so built directly here, mirroring how the `IndexExpr` test above
        // bypasses the parser's own folding. `eval::resolve_node`'s matching
        // `Expr::FirstExpr(inner) | Expr::Builtin(Builtin::FirstStream(inner))`
        // arm still handles this variant, purely as defensive symmetry.
        let json = br#"[{"a":1,"a":2},{"b":3,"b":4}]"#;
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::FirstStream(Box::new(Expr::Iterate)));

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"{"a":1,"a":2}"#);
    }

    #[test]
    fn test_json_last_stream_builtin_preserves_duplicate_keys() {
        // Mirror of `test_json_first_stream_builtin_preserves_duplicate_keys`
        // for `Builtin::LastStream` -- the other `eval_first_or_last_generic`
        // call site, also unreachable from `crate::jq::parse` (see that
        // test's comment), so built directly here too.
        let json = br#"[{"a":1,"a":2},{"b":3,"b":4}]"#;
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::LastStream(Box::new(Expr::Iterate)));

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"{"b":3,"b":4}"#);
    }

    #[test]
    fn test_json_first_and_last_of_identity_without_cursor_yield_bare_one() {
        // `eval_first_or_last_generic`'s `One(v) => One(v)` passthrough arm:
        // reached when `inner` itself has no cursor to carry (the top-level
        // `eval()` entry point starts with `cursor = None`, unlike
        // `eval_with_cursor`), so `.`'s `cursor.map_or(One, OneCursor)`
        // yields a bare `One` that `first`/`last` must forward unchanged.
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let first = eval(&crate::jq::parse("first(.)").unwrap(), value.clone());
        assert!(matches!(first, GenericResult::One(_)));
        assert_eq!(
            first.collect_owned().unwrap(),
            vec![OwnedValue::Object(
                core::iter::once(("a".to_string(), OwnedValue::Int(1))).collect()
            )]
        );

        let last = eval(&crate::jq::parse("last(.)").unwrap(), value);
        assert!(matches!(last, GenericResult::One(_)));
    }

    #[test]
    fn test_json_first_and_last_of_identity_with_cursor_yield_one_cursor() {
        // Same shape as above, but with a cursor available: `.`'s
        // `cursor.map_or` now takes the `OneCursor` side, exercising
        // `eval_first_or_last_generic`'s `OneCursor(c) => OneCursor(c)`
        // passthrough arm rather than its `One` sibling.
        let json = br#"{"a":1,"a":2}"#;
        let index = JsonIndex::build(json);

        let first = eval_with_cursor(&crate::jq::parse("first(.)").unwrap(), index.root(json));
        assert!(first.is_single_cursor());
        let mut out = String::new();
        first
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"{"a":1,"a":2}"#);

        let last = eval_with_cursor(&crate::jq::parse("last(.)").unwrap(), index.root(json));
        assert!(last.is_single_cursor());
    }

    #[test]
    fn test_json_first_and_last_of_multi_truthy_select_without_cursor_yield_bare_many() {
        // `eval_first_or_last_generic`'s `Many(vs) => One(...)` arm: reached
        // when `inner` yields a bare (cursor-less) `Many`, which only
        // happens when `select`'s own incoming cursor is `None` (see
        // `Builtin::Select`'s `pass_n` closure) -- so this also goes through
        // the cursor-less `eval()` entry point like the `One` case above.
        let json = br"1";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let first = eval(
            &crate::jq::parse("first(select(true,true))").unwrap(),
            value.clone(),
        );
        assert!(matches!(first, GenericResult::One(_)));
        assert_eq!(first.collect_owned().unwrap(), vec![OwnedValue::Int(1)]);

        let last = eval(&crate::jq::parse("last(select(true,true))").unwrap(), value);
        assert!(matches!(last, GenericResult::One(_)));
        assert_eq!(last.collect_owned().unwrap(), vec![OwnedValue::Int(1)]);
    }

    // `GenericResult::stream_json`/`stream_yaml`'s `Self::One`/`Self::Many`
    // arms (bare, cursor-less `V`) convert to `OwnedValue` via `to_owned`
    // and stream that -- distinct from `Self::OneCursor`/`ManyCursor`'s
    // direct cursor streaming, which is what the M2 CLI fast path actually
    // exercises for real navigation queries. Reached the same way as
    // `test_json_first_and_last_of_identity_without_cursor_yield_bare_one`/
    // `..._of_multi_truthy_select_without_cursor_yield_bare_many` above: via
    // the cursor-less `eval()` entry point.
    #[test]
    fn test_stream_json_and_yaml_bare_one_streams_via_owned_value() {
        let json = br#"{"b": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(&crate::jq::parse("first(.)").unwrap(), value);
        assert!(matches!(result, GenericResult::One(_)));

        let mut json_out = String::new();
        result
            .stream_json(
                &mut json_out,
                IndentSpec::COMPACT,
                true,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(json_out, r#"{"a":2,"b":1}"#);

        let mut yaml_out = String::new();
        result
            .stream_yaml(&mut yaml_out, IndentSpec::COMPACT, true, |_| Ok(()))
            .unwrap();
        assert_eq!(yaml_out, "{a: 2, b: 1}");
    }

    #[test]
    fn test_stream_json_and_yaml_bare_many_streams_via_owned_values() {
        let json = br#"{"b": 1, "a": 2}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(&crate::jq::parse("select(true,true)").unwrap(), value);
        assert!(matches!(result, GenericResult::Many(_)));

        let mut json_out = String::new();
        result
            .stream_json(
                &mut json_out,
                IndentSpec::COMPACT,
                true,
                JsonConvention::Preserve,
                |w| core::fmt::Write::write_str(w, ";"),
            )
            .unwrap();
        assert_eq!(json_out, r#"{"a":2,"b":1};{"a":2,"b":1};"#);

        let mut yaml_out = String::new();
        result
            .stream_yaml(&mut yaml_out, IndentSpec::COMPACT, true, |w| {
                core::fmt::Write::write_str(w, ";")
            })
            .unwrap();
        assert_eq!(yaml_out, "{a: 2, b: 1};{a: 2, b: 1};");
    }

    #[test]
    fn test_json_first_and_last_of_single_literal_yield_owned() {
        // `eval_first_or_last_generic`'s `Owned(v) => Owned(v)` passthrough
        // arm: a literal falls through `eval_single`'s generic bridge
        // straight to `GenericResult::Owned`, with no `Many`/cursor wrapping
        // to unwrap.
        let json = br"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let first = eval(&crate::jq::parse("first(1)").unwrap(), value.clone());
        assert!(matches!(
            first,
            GenericResult::Owned(
                OwnedValue::Int(1) | OwnedValue::NumberLiteral(NumberRepr::Int(1), _)
            )
        ));

        let last = eval(&crate::jq::parse("last(1)").unwrap(), value);
        assert!(matches!(
            last,
            GenericResult::Owned(
                OwnedValue::Int(1) | OwnedValue::NumberLiteral(NumberRepr::Int(1), _)
            )
        ));
    }

    #[test]
    fn test_json_first_and_last_of_comma_literals_yield_many_owned() {
        // `eval_first_or_last_generic`'s `ManyOwned(vs) => Owned(...)` arm:
        // `1,2,3` falls through the generic bridge to `ManyOwned`, and
        // `first`/`last` must pick the front/back element respectively.
        let json = br"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let first = eval(&crate::jq::parse("first(1,2,3)").unwrap(), value.clone());
        assert!(matches!(
            first,
            GenericResult::Owned(
                OwnedValue::Int(1) | OwnedValue::NumberLiteral(NumberRepr::Int(1), _)
            )
        ));

        let last = eval(&crate::jq::parse("last(1,2,3)").unwrap(), value);
        assert!(matches!(
            last,
            GenericResult::Owned(
                OwnedValue::Int(3) | OwnedValue::NumberLiteral(NumberRepr::Int(3), _)
            )
        ));
    }

    #[test]
    fn test_json_first_and_last_of_error_propagate_error() {
        // `eval_first_or_last_generic`'s `Error(e) => Error(e)` arm in both
        // directions: an inner expression that errors outright (no
        // preceding successful output, so no `Partial` prefix) must still
        // surface as `Error` through `first`/`last`.
        let json = br"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let first = eval(
            &crate::jq::parse(r#"first(error("boom"))"#).unwrap(),
            value.clone(),
        );
        assert!(first.is_error());

        let last = eval(&crate::jq::parse(r#"last(error("boom"))"#).unwrap(), value);
        assert!(last.is_error());
    }

    #[test]
    fn test_json_first_and_last_of_break_propagate_break() {
        // `eval_first_or_last_generic`'s `Break(label) => Break(label)` arm
        // in both directions. Built directly as `Expr::Break`, same as
        // `test_json_iterate_then_break_propagates` above, since a bare
        // `break $out` needs no enclosing `label` to produce this signal at
        // the `GenericResult` level -- label-catching happens above this
        // module.
        let json = br"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let first = eval(
            &Expr::FirstExpr(Box::new(Expr::Break("out".to_string()))),
            value.clone(),
        );
        assert!(matches!(first, GenericResult::Break(label) if label == "out"));

        let last = eval(
            &Expr::LastExpr(Box::new(Expr::Break("out".to_string()))),
            value,
        );
        assert!(matches!(last, GenericResult::Break(label) if label == "out"));
    }

    #[test]
    fn test_json_first_of_partial_prefix_returns_first_owned_value() {
        // `eval_first_or_last_generic`'s `Partial(vs, _control) =>
        // Owned(vs[0])` arm (the `first` direction only -- `first` never
        // asks for values past the first, so the trailing control is
        // dropped; see the arm's own doc comment). `(1,2,error("x"))`
        // produces outputs `1`,`2` before erroring, i.e. exactly the
        // `Partial([1,2], Error)` this arm exists to handle.
        let json = br"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(
            &crate::jq::parse(r#"first(1,2,error("x"))"#).unwrap(),
            value,
        );
        assert!(matches!(
            result,
            GenericResult::Owned(
                OwnedValue::Int(1) | OwnedValue::NumberLiteral(NumberRepr::Int(1), _)
            )
        ));
    }

    #[test]
    fn test_json_last_of_partial_prefix_drops_prefix_and_keeps_control() {
        // `eval_first_or_last_generic`'s two `Partial` arms in the `last`
        // direction: `last` must exhaust the stream to know which output is
        // last, so a `Partial` prefix can never be "the last output" -- only
        // its trailing control (`Error` or `Break`) surfaces, with the
        // prefix silently dropped.
        let json = br"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let error_case = eval(
            &crate::jq::parse(r#"last(1,2,error("x"))"#).unwrap(),
            value.clone(),
        );
        assert!(error_case.is_error());

        let break_case = eval(
            &Expr::LastExpr(Box::new(Expr::Comma(vec![
                Expr::Literal(Literal::Int(1)),
                Expr::Literal(Literal::Int(2)),
                Expr::Break("out".to_string()),
            ]))),
            value,
        );
        assert!(matches!(break_case, GenericResult::Break(label) if label == "out"));
    }

    #[test]
    fn test_json_split_doc_forwards_cursor() {
        // Bonus finding from #607's audit: `Builtin::SplitDoc` was
        // documented as "identity" but unconditionally returned
        // `GenericResult::Owned(to_owned(&value))`, unlike `Values`/
        // `Iterables`/`Scalars`/`Identity`, which all forward the incoming
        // cursor via `cursor.map_or(...)`. Fixed to match.
        let json = br#"{"a":1,"a":2}"#;
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::SplitDoc);

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"{"a":1,"a":2}"#);
    }

    #[test]
    fn test_json_multi_stage_pipe_first_stage_bare_one_without_cursor() {
        // `eval_pipe_generic`'s `GenericResult::One(v) => eval_single(expr, v,
        // optional, None)` arm: reached when an *earlier* pipe stage yields a
        // bare (cursor-less) `One`, which only happens starting from the
        // cursor-less `eval()` entry point (`eval_with_cursor` always starts
        // with `Some`). Indirectly a #607-adjacent finding: before #607,
        // computed-key indexing (`.[$k]`) used to feed this same arm
        // regardless of cursor presence; after #607 it always carries a
        // cursor instead, so this arm is now reachable only through a
        // genuinely cursor-less root.
        let json = br#"{"a": 1}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(&crate::jq::parse(". | length").unwrap(), value);
        assert!(matches!(result, GenericResult::Owned(OwnedValue::Int(1))));
    }

    #[test]
    fn test_json_multi_stage_pipe_first_stage_bare_many_without_cursor() {
        // Sibling of the `One` case above for `eval_pipe_generic`'s
        // `GenericResult::Many(vs)` stage arm: each of `select`'s two
        // cursor-less truthy outputs is piped independently into `length`,
        // accumulating a `ManyOwned` result.
        let json = br"1";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(
            &crate::jq::parse("select(true,true) | length").unwrap(),
            value,
        );
        // `length` of the integer `1` is its absolute value, `1`.
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(1), OwnedValue::Int(1)]
        );
    }

    #[test]
    fn test_generic_result_collect_owned_bare_many_variant() {
        // `GenericResult::collect_owned`'s `Many(vs)` arm: `first`/`last`
        // always collapse a `Many` down to `One` internally (see
        // `eval_first_or_last_generic`), so this arm needs a bare `Many`
        // observed directly at the top level instead -- `select`'s two
        // cursor-less truthy outputs, same source as the pipe test above.
        let json = br"1";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(&crate::jq::parse("select(true,true)").unwrap(), value);
        assert!(matches!(result, GenericResult::Many(_)));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(1), OwnedValue::Int(1)]
        );
    }

    #[test]
    fn test_json_slice_target_bare_one_and_many_without_cursor() {
        // `eval_slice_expr`'s `Targets::Borrowed` construction from a bare
        // `One`/`Many` target: same cursor-less-root precondition as the
        // pipe-stage tests above, this time for the slice target position
        // (`E[S:T]`'s `E`) rather than a pipe stage. `eval_slice_expr` (the
        // `Expr::SliceExpr` arm) is only reachable when a bound doesn't fold
        // to a literal (#499) -- `[0:2]` with literal bounds instead parses
        // to the postfix `Expr::Slice` form, which isn't natively handled
        // here at all and falls through the `to_owned()` bridge. `(1-1)`/
        // `(1+1)` keep the *values* 0/2 while forcing the `SliceExpr` shape.
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let one = eval(&crate::jq::parse(".[(1-1):(1+1)]").unwrap(), value.clone());
        assert_eq!(
            one.collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2)
            ])]
        );

        let many = eval(
            &crate::jq::parse("select(true,true)[(1-1):(1+1)]").unwrap(),
            value,
        );
        assert_eq!(
            many.collect_owned().unwrap(),
            vec![
                OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
                OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
            ]
        );
    }

    #[test]
    fn test_json_iterate_then_multi_key_index_expr_yields_many_cursor_and_many_owned() {
        // Same flatten path, but the per-element computed index now yields
        // `ManyCursor` (both keys found on the second item, #607) or
        // `ManyOwned` (a mix of found/missing keys forces the owned fallback
        // on the first and third items), exercising both arms.
        let json = br#"{"items": [{"a": 1}, {"a": 1, "b": 2}, {"c": 3}]}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(r#".items[] | .[("a","b")]"#).unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
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
    fn test_json_computed_slice_bounds_via_eval_generic() {
        // #615: exercises Expr::SliceExpr through eval_generic's actual
        // dispatch path (eval_with_cursor), not eval.rs's outcome()/eval()
        // helper — which is the path the CLI (jq_runner/yq_runner) actually
        // uses, and which previously fell into the `_` fallback's full
        // serialize-and-reindex round trip for every node visited.
        let json = br#"{"a":[1,2,3,4,5],"k1":1,"k2":3}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".a[.k1:.k2]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(3)
            ])]
        );
    }

    #[test]
    fn test_json_computed_slice_bounds_realistic_iterate_pattern() {
        // The issue's own motivating shape: `.items[] | .data[.from:.to]`,
        // where bounds are computed per-element from sibling fields.
        let json = br#"{"items": [
            {"data": [1,2,3,4,5], "from": 0, "to": 2},
            {"data": [10,20,30,40], "from": 1, "to": 3}
        ]}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".items[] | .data[.from:.to]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![
                OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
                OwnedValue::Array(vec![OwnedValue::Int(20), OwnedValue::Int(30)]),
            ]
        );
    }

    #[test]
    fn test_json_computed_slice_bounds_owned_target() {
        // The target itself is computed (not a navigational path), so
        // `eval_single(target, ...)` yields `Owned`/`ManyOwned` rather than a
        // borrowed document value — exercises `eval_slice_expr`'s
        // `Targets::Owned` branch, which delegates to `slice_owned_value`
        // instead of `slice_one_generic`.
        let json = br#"{"a":[1,2],"b":[3,4],"k1":1,"k2":3}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse("(.a + .b)[.k1:.k2]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(3)
            ])]
        );
    }

    #[test]
    fn test_json_computed_slice_bounds_optional_suppresses_non_sliceable_target() {
        // A trailing `?` covers only the final "target isn't sliceable"
        // refusal, same rule `eval::eval_slice_expr` documents. The bounds
        // themselves (`.a`) resolve fine against the same root object.
        let json = br#"{"a":0}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".[(.a):2]?").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(!result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_json_computed_slice_bounds_empty_end_bound_short_circuits_before_target() {
        // An empty bound stream must short-circuit *before* the target is
        // evaluated: `error("boom")` here would raise if the target were
        // reached at all.
        let json = b"null";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(r#"(error("boom"))[0:empty]"#).unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(!result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_json_computed_slice_bounds_empty_start_bound_short_circuits_before_target() {
        // Symmetric with the end-bound short circuit above: an empty
        // *start* stream must return `None` before the target — and the end
        // bound — are ever reached.
        let json = b"null";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(r#"(error("boom"))[empty:2]"#).unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(!result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_json_computed_slice_bounds_bound_error_and_break() {
        // A start or end bound that itself errors or breaks propagates
        // directly out of `eval_slice_expr` — one arm per side, mirroring
        // `eval_index_expr`'s key-evaluation `Error`/`Break` arms.
        let json = br#"{"a":[1,2,3]}"#;
        let index = JsonIndex::build(json);

        let expr = crate::jq::parse(r#".a[(error("start-boom")):2]"#).unwrap();
        assert!(eval_with_cursor(&expr, index.root(json)).is_error());

        let expr = crate::jq::parse(".a[(break $out):2]").unwrap();
        assert!(matches!(
            eval_with_cursor(&expr, index.root(json)),
            GenericResult::Break(label) if label == "out"
        ));

        let expr = crate::jq::parse(r#".a[0:(error("end-boom"))]"#).unwrap();
        assert!(eval_with_cursor(&expr, index.root(json)).is_error());

        let expr = crate::jq::parse(".a[0:(break $out)]").unwrap();
        assert!(matches!(
            eval_with_cursor(&expr, index.root(json)),
            GenericResult::Break(label) if label == "out"
        ));
    }

    #[test]
    fn test_json_computed_slice_bounds_partial_bound_keeps_its_prefix_1528() {
        // A bound stream can itself be `Partial` (some outputs, then a
        // control) — `eval_slice_bound` keeps that prefix instead of
        // collapsing to the bare control (#1528, mirroring #1517's identical
        // fix for the path-mode resolver). Confirmed against jq 1.7.1:
        // `.a[(1,2,error("x")):2]`/`.a[(1,2,break $out):2]` on
        // `{"a":[1,2,3]}` both print `[2]` then `[]` (from `$s=1`/`$s=2`
        // against the fixed end bound `2`) before the error/break fires on
        // `$s`'s third candidate.
        let json = br#"{"a":[1,2,3]}"#;
        let index = JsonIndex::build(json);

        let expr = crate::jq::parse(r#".a[(1,2,error("x")):2]"#).unwrap();
        match eval_with_cursor(&expr, index.root(json)) {
            GenericResult::Partial(prefix, Control::Error(e)) => {
                assert_eq!(
                    prefix.iter().map(OwnedValue::to_json).collect::<Vec<_>>(),
                    vec!["[2]", "[]"]
                );
                assert_eq!(e.message, "x");
            }
            other => panic!("expected Partial(_, Error), got {other:?}"),
        }

        let expr = crate::jq::parse(".a[(1,2,break $out):2]").unwrap();
        match eval_with_cursor(&expr, index.root(json)) {
            GenericResult::Partial(prefix, Control::Break(label)) => {
                assert_eq!(label, "out");
                assert_eq!(
                    prefix.iter().map(OwnedValue::to_json).collect::<Vec<_>>(),
                    vec!["[2]", "[]"]
                );
            }
            other => panic!("expected Partial(_, Break), got {other:?}"),
        }
    }

    #[test]
    fn test_json_computed_slice_bounds_open_start_and_open_end() {
        // A missing bound short-circuits `eval_slice_bound` to a single
        // `None` ("open on this side") without evaluating anything — on
        // either side, independently of which bound is the one forcing the
        // `SliceExpr` fast path.
        let json = br#"{"a":[1,2,3,4,5],"k1":1,"k2":3}"#;
        let index = JsonIndex::build(json);

        let expr = crate::jq::parse(".a[:.k2]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, index.root(json))
                .collect_owned()
                .unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3)
            ])]
        );

        let expr = crate::jq::parse(".a[.k1:]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, index.root(json))
                .collect_owned()
                .unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(3),
                OwnedValue::Int(4),
                OwnedValue::Int(5)
            ])]
        );
    }

    #[test]
    fn test_json_computed_slice_bounds_target_error_break_none() {
        // The target's `Error`/`Break`/`None` arms — mirrors
        // `eval_index_expr`'s own target-evaluation arms directly above
        // `eval_slice_expr` in the source.
        let json = b"null";
        let index = JsonIndex::build(json);

        let expr = crate::jq::parse(r#"(error("boom"))[(0+0):2]"#).unwrap();
        assert!(eval_with_cursor(&expr, index.root(json)).is_error());

        let expr = crate::jq::parse("(break $out)[(0+0):2]").unwrap();
        assert!(matches!(
            eval_with_cursor(&expr, index.root(json)),
            GenericResult::Break(label) if label == "out"
        ));

        let expr = crate::jq::parse("(empty)[(0+0):2]").unwrap();
        let result = eval_with_cursor(&expr, index.root(json));
        assert!(!result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_json_computed_slice_bounds_single_key_target_is_borrowed_one() {
        // `.[(0+0)]` — a single computed key — collapses to `One(v)` inside
        // `eval_index_expr`, so it lands in `eval_slice_expr`'s target match
        // as a plain borrowed `One`, not `OneCursor`.
        let json = br"[[1,2,3],[4,5]]";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".[(0+0)][(0+0):2]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2)
            ])]
        );
    }

    #[test]
    fn test_json_computed_slice_bounds_multi_key_target_is_borrowed_many_and_errors_per_element() {
        // `.[(0,1)]` — a multi-key computed index — collapses to a plain
        // borrowed `Many`, the only shape that reaches that arm (see the
        // comment on `eval_index_expr`'s own `Many`-producing test). Slicing
        // its elements (bare numbers) also exercises the borrowed loop's
        // `Error` arm and `slice_one_generic`'s "not sliceable" refusal, in
        // the same call.
        let json = br"[10,20,30]";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".[(0,1)][(0+0):2]").unwrap();

        assert!(eval_with_cursor(&expr, index.root(json)).is_error());
    }

    #[test]
    fn test_json_computed_slice_bounds_iterate_target_is_many_cursor() {
        // `.[]` over a document array yields `ManyCursor`, not a plain
        // `Many` — built directly, since the parser has no bare `.[]`
        // slice-target spelling to mirror the computed-index cases above.
        let json = br"[[1,2,3],[4,5,6]]";
        let index = JsonIndex::build(json);
        let expr = Expr::slice_by(
            Expr::Iterate,
            Some(Expr::Literal(Literal::Int(0))),
            Some(Expr::Literal(Literal::Int(2))),
        );

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![
                OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
                OwnedValue::Array(vec![OwnedValue::Int(4), OwnedValue::Int(5)]),
            ]
        );
    }

    #[test]
    fn test_json_computed_slice_bounds_owned_target_error_and_optional_none() {
        // A computed, non-array/string/null owned target: `slice_owned_value`
        // errors without `?`, and returns `None` (silently) with it —
        // exercising the owned loop's two non-success arms.
        let json = b"null";
        let index = JsonIndex::build(json);

        let expr = crate::jq::parse("(1+1)[(0+0):2]").unwrap();
        assert!(eval_with_cursor(&expr, index.root(json)).is_error());

        let expr = crate::jq::parse("(1+1)[(0+0):2]?").unwrap();
        let result = eval_with_cursor(&expr, index.root(json));
        assert!(!result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_json_computed_slice_bounds_null_target_yields_null() {
        // A borrowed target that resolves to `null` short-circuits inside
        // `slice_one_generic` before the array/string checks below it.
        let json = br#"{"a":null,"k1":0,"k2":1}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".a[.k1:.k2]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.collect_owned().unwrap(), vec![OwnedValue::Null]);
    }

    #[test]
    fn test_json_computed_slice_bounds_string_target() {
        // A borrowed string target — the arm below the null check and above
        // the "not sliceable" refusal in `slice_one_generic`.
        let json = br#"{"a":"hello","k1":1,"k2":3}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".a[.k1:.k2]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::String("el".to_string())]
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
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(10), OwnedValue::Int(20)]
        );
    }

    #[test]
    fn test_json_compare_left_many_cursor_forks_over_every_element_768() {
        // `Compare`'s `ManyCursor`-operand arm: `.[]` on the left side yields
        // a `ManyCursor`, and the comparison forks over every element rather
        // than only the first (#768) -- verified against jq: `[1,2] | .[] ==
        // 1` is `true`, `false`.
        let json = br"[1, 2]";
        let index = JsonIndex::build(json);
        let expr = Expr::Compare {
            op: CompareOp::Eq,
            left: Box::new(Expr::Iterate),
            right: Box::new(Expr::Literal(Literal::Int(1))),
        };

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Bool(true), OwnedValue::Bool(false)]
        );
    }

    #[test]
    fn test_json_compare_right_many_cursor_forks_over_every_element_768() {
        // Mirror of the above for the right-hand operand -- right is the
        // outer loop (#768), but with only one left output the ordering
        // still comes out element-order: `[1,2] | 1 == .[]` is `true`, `false`.
        let json = br"[1, 2]";
        let index = JsonIndex::build(json);
        let expr = Expr::Compare {
            op: CompareOp::Eq,
            left: Box::new(Expr::Literal(Literal::Int(1))),
            right: Box::new(Expr::Iterate),
        };

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Bool(true), OwnedValue::Bool(false)]
        );
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
        assert_eq!(result.into_owned().unwrap(), Some(OwnedValue::Int(5)));
    }

    #[test]
    fn test_json_select_cond_one_cursor_truthy() {
        // Same as above, but evaluated with cursor context, so the condition
        // result is `OneCursor` instead of `One`.
        let json = b"5";
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::Select(Box::new(Expr::Identity)));

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned().unwrap(), Some(OwnedValue::Int(5)));
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
            result.into_owned().unwrap(),
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
        assert_eq!(result.into_owned().unwrap(), Some(OwnedValue::Int(5)));
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
        assert_eq!(result.into_owned().unwrap(), Some(OwnedValue::Int(2)));
    }

    #[test]
    fn test_json_column_builtin_with_cursor() {
        // JSON counterpart of `test_yaml_column_builtin_with_cursor`,
        // exercising `JsonCursor`'s `DocumentCursor::column()` delegation.
        let json = b"{\n  \"foo\": 1\n}";
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".foo | column").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned().unwrap(), Some(OwnedValue::Int(10)));
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
            other => Summary::Values(json_of(&other.collect_owned().unwrap())),
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

        // #2226: a computed *target* that is itself a multi-output generator
        // and is `Partial` used to surface the control alone, discarding
        // whatever `E` had already produced this key/pair — pinned in a
        // dedicated `generic_value_position_partial_collapses_to_its_control`
        // test, not endorsed, before the fix. `eval_index_expr`'s/
        // `eval_slice_expr`'s target `Partial` arm now applies the per-key
        // index/slice to each already-produced value instead, matching jq
        // 1.7.1 exactly (`(.[0],(.[1],error("x")))[(0+0)]` prints `7` then
        // `8` before failing; the slice sibling below prints `[7]` then
        // `[8]`, since `.[0]`/`.[1]` here are the one-element arrays `[7]`/
        // `[8]`, not the bare numbers).
        assert_eq!(
            summarize(b"[[7],[8]]", r#"(.[0],(.[1],error("x")))[(0+0)]"#),
            partial_err(&["7", "8"], "x")
        );
        assert_eq!(
            summarize(b"[[7],[8]]", "(.[0],(.[1],break $out))[(0+0)]"),
            partial_break(&["7", "8"])
        );
        // Computed slicing (#615) shares the same target-position `Partial`
        // fix as computed indexing above — one bound (`(0+0)`) is enough to
        // force `eval_slice_expr`'s fast path.
        assert_eq!(
            summarize(b"[[7],[8]]", r#"(.[0],(.[1],error("x")))[(0+0):2]"#),
            partial_err(&["[7]", "[8]"], "x")
        );
        assert_eq!(
            summarize(b"[[7],[8]]", "(.[0],(.[1],break $out))[(0+0):2]"),
            partial_break(&["[7]", "[8]"])
        );
    }

    #[test]
    fn generic_compare_partial_operand_keeps_prefix_768() {
        // A `Partial` operand on either side now keeps every pairing
        // computed before the trailing control, instead of taking the first
        // output and dropping the control entirely (#768) — matching real
        // jq exactly: `null | (1,error("x")) == 1` prints `true`, then fails.
        assert_eq!(
            summarize(b"null", r#"(1,error("x")) == 1"#),
            partial_err(&["true"], "x")
        );
        assert_eq!(
            summarize(b"null", r#"1 == (1,error("x"))"#),
            partial_err(&["true"], "x")
        );
        assert_eq!(
            summarize(b"null", "(1,break $out) == 1"),
            partial_break(&["true"])
        );
    }

    #[test]
    fn generic_compare_first_stops_before_unneeded_right_candidate_1481() {
        // `first` is satisfied by the very first pairing (`1 == 10` ->
        // `false`), so `eval_each_generic`'s new native `Expr::Compare` arm
        // (#1481) must never let right's generator reach its second, failing
        // candidate -- mirrors `eval.rs`'s own `IN`/`any`/`all`-wrapped
        // equivalents, now also true for a bare `first(...)`.
        assert_eq!(
            summarize(b"null", r#"first((1,2) == (10, error("x")))"#),
            Summary::Values(vec!["false".to_string()])
        );
    }

    #[test]
    fn generic_compare_first_still_reaches_right_when_left_is_empty_1481() {
        // The complementary case: `left=empty` means `combine` never runs for
        // right's first candidate (`10`), so the fanout must still continue
        // to right's second candidate -- confirming the new lazy arm's
        // `Demand::Continue` path is exercised, not just its stop path.
        assert_eq!(
            summarize(b"null", r#"first(empty == (10, error("x")))"#),
            Summary::Error("x".to_string())
        );
    }

    #[test]
    fn test_generic_compare_operand_bare_one_via_cursor_less_entry_1481() {
        // Same cursor-less-entry mechanism as
        // `test_computed_index_key_bare_one_and_many_via_cursor_less_entry`
        // below: entering through `eval()` rather than the CLI's
        // `eval_with_cursor()` gives `Expr::Identity` a `None` ambient
        // cursor, so each operand of `. == .` yields `GenericItem::One`, not
        // `OneCursor`. That's the only way to reach
        // `generic_item_into_owned`'s `One` arm -- `jq_runner.rs` always
        // threads a real cursor through `eval_with_cursor`, so no CLI
        // invocation can reach it.
        let json = b"5";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse(". == .").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Bool(true)
        );
    }

    #[test]
    fn test_generic_compare_left_operand_lazy_seq_error_propagates_1481() {
        // A `LazySeq` operand (`map(f)`) that errors on materialization --
        // forced by `generic_item_into_owned`, which
        // `binary_fanout_each_generic` calls on every pulled item -- aborts
        // the whole fanout rather than just that pairing. On the *left*
        // operand this exercises `binary_fanout_each_generic`'s left-error
        // propagation (the inner sink's own `Err(control)` arm, plus the
        // `if abort.is_some() { return Demand::Stop }` check once `inner`
        // returns).
        let json = b"[1,2]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(1/0) == 1").unwrap();
        // `assert!(matches!(..))` rather than a `match` with a `panic!`
        // fallback: the fallback arm is only ever reached by a *failing*
        // test, so it reads as a permanently uncovered line in the coverage
        // diff (#1601). Same style as
        // `test_generic_compare_operand_lazy_seq_break_and_halt_propagate_1481`
        // just above.
        assert!(
            matches!(eval(&expr, value), GenericResult::Error(ref e) if e.message.contains("divided"))
        );
    }

    #[test]
    fn test_generic_compare_right_operand_lazy_seq_error_propagates_1481() {
        // Mirror of the above with the erroring `LazySeq` on the *right*
        // operand, exercising `binary_fanout_each_generic`'s other
        // error-propagation arm (`right_item`'s own `Err(control)` match,
        // reached before `left` is ever pulled).
        let json = b"[1,2]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("1 == map(1/0)").unwrap();
        // `assert!(matches!(..))` rather than a `match` with a `panic!`
        // fallback: the fallback arm is only ever reached by a *failing*
        // test, so it reads as a permanently uncovered line in the coverage
        // diff (#1601). Same style as
        // `test_generic_compare_operand_lazy_seq_break_and_halt_propagate_1481`
        // just above.
        assert!(
            matches!(eval(&expr, value), GenericResult::Error(ref e) if e.message.contains("divided"))
        );
    }

    #[test]
    fn test_generic_compare_operand_lazy_seq_break_and_halt_propagate_1481() {
        // `generic_item_into_owned`'s `Break`/`Halt` arms: a `break`/
        // `halt_error` inside a `map(f)` operand aborts materialization with
        // `Control::Break`/`Control::Halt`, not `Control::Error` -- and
        // `binary_fanout_each_generic` carries either straight out as the
        // fanout's own `Flow::Escaped`.
        let json = b"[1,2]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(if . == 2 then break $out else . end) == 1").unwrap();
        assert!(
            matches!(eval(&expr, value.clone()), GenericResult::Break(ref label) if label == "out")
        );

        let expr = crate::jq::parse("map(halt_error(7)) == 1").unwrap();
        assert!(matches!(eval(&expr, value), GenericResult::Halt(7)));
    }

    #[test]
    fn generic_arithmetic_fanout_combine_error_escapes_1481() {
        // `binary_fanout_each_generic`'s `Err(e)` arm -- the one only the
        // `Expr::Arithmetic` caller can reach, since both `Expr::Compare`
        // call sites wrap the infallible `apply_compare_op` in `Ok(...)`.
        // `first(...)` is what routes a top-level `+` through
        // `eval_each_generic` at all (`Expr::Arithmetic` has no native
        // `eval_single` arm, so a *bare* `("a",1) + 1` bridges out to
        // `eval.rs`'s own fanout instead).
        //
        // Right is the outer loop, so the single right candidate `1` pairs
        // with left's first candidate `"a"` -- `arith_combine` fails, and
        // the failure aborts the whole fanout as the caller's error rather
        // than skipping that one pairing and going on to `1 + 1`. Oracle:
        // pinned jq 1.7.1 `jq -n 'first(("a",1) + 1)'` is exit-5 with
        // `string ("a") and number (1) cannot be added`, *not* `2`.
        assert!(matches!(
            summarize(b"null", r#"first(("a",1) + 1)"#),
            Summary::Error(ref m) if m.contains("cannot be added")
        ));
    }

    #[test]
    fn generic_arithmetic_fanout_combine_error_beats_later_right_candidate_1481() {
        // The same `Err(e)` arm, this time proving the abort really is an
        // abort of the *outer* (right) loop and not just of the inner one:
        // right's first candidate `"a"` already fails to combine, so right's
        // second candidate -- `error("x")`, which would report a different
        // message -- is never pulled. Oracle: pinned jq 1.7.1
        // `jq -n 'first(1 + ("a", error("x")))'` reports the arithmetic
        // error, not `x`.
        assert!(matches!(
            summarize(b"null", r#"first(1 + ("a", error("x")))"#),
            Summary::Error(ref m) if m.contains("cannot be added")
        ));
    }

    #[test]
    fn generic_arithmetic_fanout_first_stops_before_the_failing_pairing_1481() {
        // The complement of the two above: when the *first* pairing succeeds,
        // `first` stops the generator before the pairing that would have
        // failed is ever combined, so the `Err(e)` arm is not reached and no
        // error surfaces. Oracle: pinned jq 1.7.1
        // `jq -n 'first((1,"a") + 1)'` prints `2`.
        assert_eq!(
            summarize(b"null", r#"first((1,"a") + 1)"#),
            Summary::Values(vec!["2".to_string()])
        );
    }

    #[test]
    fn generic_arithmetic_fanout_combine_error_optional_is_unreachable_via_parser_1481() {
        // The other half of `binary_fanout_each_generic`'s `Err(e)` arm: when
        // `optional` is set, the failed pairing aborts the fanout as a plain
        // `Flow::Exhausted` (empty output) instead of `Flow::Escaped`.
        //
        // No real query reaches it. Post-#693 the only place this module ever
        // forces `optional = true` is `eval_single`'s
        // `Expr::Optional(IndexExpr | SliceExpr)` special case, which threads
        // it into `index_one_generic`/`slice_one_generic` only -- never into
        // `eval_each_generic`. Every other caller passes the ambient
        // `optional`, which starts `false` at every public entry point (the
        // same observation `eval_first_or_last_generic` records for its own
        // dispatch, and `eval_on_owned`/`eval_single`'s `_` arm for theirs).
        // So drive the private dispatcher directly to pin the arm, exactly as
        // `test_generic_plain_map_optional_on_non_container_is_unreachable_via_parser_725`
        // already does for `Builtin::Map`'s matching `optional` guard.
        let json = b"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse(r#"("a",1) + 1"#).unwrap();
        assert!(matches!(expr, Expr::Arithmetic { .. }));

        let mut seen = 0usize;
        let flow = eval_each_generic::<JqSemantics, _>(&expr, value, true, None, &mut |_item| {
            seen += 1;
            Demand::Continue
        });

        // Swallowed, not escaped -- and swallowed *before* the second (valid)
        // `1 + 1` pairing, since the whole fanout aborts rather than skipping
        // just the pairing that failed.
        assert!(matches!(flow, Flow::Exhausted));
        assert_eq!(seen, 0);
    }

    #[test]
    fn test_computed_index_key_bare_one_and_many_via_cursor_less_entry() {
        // `eval_index_expr`'s key match has a `One`/`Many` arm alongside its
        // `OneCursor`/`ManyCursor` ones (#699 coverage gap): a key stream
        // whose values aren't attached to any cursor at all, not just one
        // whose per-element cursor was dropped. That only happens when the
        // *ambient* ("." at the point the whole `IndexExpr` is evaluated)
        // cursor is `None` to begin with -- i.e. entering through the
        // cursor-less `eval()`/`eval_using()` API (as plenty of this
        // module's own tests do, e.g. `test_generic_identity` above) rather
        // than `eval_with_cursor()`. `Expr::Identity` under a `None` ambient
        // cursor returns bare `One(value)`; `select(true,true)` (whose
        // `pass_n` also forwards the ambient cursor) returns bare `Many`.
        let json = b"0";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("([10,20,30])[.]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).collect_owned().unwrap(),
            vec![OwnedValue::Int(10)]
        );

        let expr = crate::jq::parse("([10,20,30])[select(true,true)]").unwrap();
        assert_eq!(
            eval(&expr, value).collect_owned().unwrap(),
            vec![OwnedValue::Int(10), OwnedValue::Int(10)]
        );
    }

    #[test]
    fn test_computed_slice_bound_bare_one_and_many_via_cursor_less_entry() {
        // Mirrors the `eval_index_expr` key case directly above, but for
        // `eval_slice_bound`'s own `One`/`Many` arms (#699 coverage gap) --
        // same cursor-less-entry mechanism, same `.`/`select(true,true)`
        // bound expressions, just feeding a slice's start bound instead of
        // an index key.
        let json = b"0";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("([10,20,30])[.:2]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).collect_owned().unwrap(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(10),
                OwnedValue::Int(20)
            ])]
        );

        let expr = crate::jq::parse("([10,20,30])[select(true,true):2]").unwrap();
        assert_eq!(
            eval(&expr, value).collect_owned().unwrap(),
            vec![
                OwnedValue::Array(vec![OwnedValue::Int(10), OwnedValue::Int(20)]),
                OwnedValue::Array(vec![OwnedValue::Int(10), OwnedValue::Int(20)]),
            ]
        );
    }

    #[test]
    fn test_computed_slice_bound_many_cursor() {
        // `eval_slice_bound`'s `ManyCursor` arm (#699 coverage gap): unlike
        // the `One`/`Many` cases above, this needs no cursor-less trickery --
        // `.starts[]` iterating a real document array yields cursors
        // regardless of the ambient cursor, so a normal `eval_with_cursor`
        // call reaches it directly.
        let json = br#"{"arr":[1,2,3,4,5],"starts":[0,1]}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".arr[.starts[]:3]").unwrap();

        assert_eq!(
            eval_with_cursor(&expr, index.root(json))
                .collect_owned()
                .unwrap(),
            vec![
                OwnedValue::Array(vec![
                    OwnedValue::Int(1),
                    OwnedValue::Int(2),
                    OwnedValue::Int(3)
                ]),
                OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Int(3)]),
            ]
        );
    }

    // Coverage follow-ups for #725: the tests above pin the common
    // `LazySeq` shapes (plain `map`, composed `map | map`, `keys_unsorted |
    // map`, atomicity), but a handful of less-common `GenericResult`
    // combinations the `LazySeq` machinery touches weren't reached by any
    // existing test.

    #[test]
    fn test_generic_lazy_seq_debug_format_725() {
        // `LazySeq`/`LazySource`'s hand-written `Debug` impls (see their doc
        // comments for why they're not derived) were never exercised by any
        // `{:?}` formatting -- pin the shape for all four `LazySource`
        // variants (`Elements`/`Values`/`Keys`/`IndexRange`).
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(.)").unwrap();
        let result = eval(&expr, value);
        let GenericResult::LazySeq(seq) = result else {
            panic!("expected LazySeq");
        };
        let debug = format!("{seq:?}");
        assert!(debug.contains("LazySource::Elements"), "{debug}");
        assert!(debug.contains("pending_len"), "{debug}");

        let json = br#"{"a":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(.)").unwrap();
        let GenericResult::LazySeq(seq) = eval(&expr, value) else {
            panic!("expected LazySeq");
        };
        assert!(format!("{seq:?}").contains("LazySource::Values"));

        let expr = crate::jq::parse("keys_unsorted | map(.)").unwrap();
        let json = br#"{"a":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let GenericResult::LazySeq(seq) = eval(&expr, value) else {
            panic!("expected LazySeq");
        };
        assert!(format!("{seq:?}").contains("LazySource::Keys"));

        let json = br#"["x","y"]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let GenericResult::LazySeq(seq) = eval(&expr, value) else {
            panic!("expected LazySeq");
        };
        let debug = format!("{seq:?}");
        assert!(debug.contains("LazySource::IndexRange"), "{debug}");
        assert!(debug.contains("next"), "{debug}");
        assert!(debug.contains("len"), "{debug}");
    }

    #[test]
    fn test_generic_lazy_elem_debug_format_725() {
        // `LazyElem`'s hand-written `Debug` impl (see its doc comment: it's
        // `pub` because anyone who can name the `pub` `LazySeq` type can call
        // `.next()` on it and observe a `LazyElem`) -- pull one element of
        // each kind directly and format it, since `LazySeq`'s own `Debug`
        // above only ever prints `pending`'s *length*, never its elements.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        // `map(.)`: identity preserves the cursor, so the first pulled item
        // is `LazyElem::Cursor`.
        let expr = crate::jq::parse("map(.)").unwrap();
        let GenericResult::LazySeq(mut seq) = eval(&expr, value.clone()) else {
            panic!("expected LazySeq");
        };
        let elem = seq.next().unwrap().unwrap();
        assert_eq!(format!("{elem:?}"), "LazyElem::Cursor(..)");

        // `map(.+1)`: arithmetic computes a fresh value, so the first pulled
        // item is `LazyElem::Owned`.
        let expr = crate::jq::parse("map(. + 1)").unwrap();
        let GenericResult::LazySeq(mut seq) = eval(&expr, value) else {
            panic!("expected LazySeq");
        };
        let elem = seq.next().unwrap().unwrap();
        assert_eq!(format!("{elem:?}"), "LazyElem::Owned(Int(2))");
    }

    #[test]
    fn test_generic_lazy_seq_yq_semantics_threaded_725() {
        // `LazySeq::eval_one` re-dispatches per `Instruction::tag`
        // (`EvalTag::Jq` vs `EvalTag::Yq`) so yq keeps yq arithmetic
        // semantics through a lazy `map` chain -- neither tagged arm's
        // `Yq` branch (`LazyElem::Cursor`+`Yq` for the first stage,
        // `LazyElem::Owned`+`Yq` for the second, composed stage) was
        // exercised by any test using `eval_using`/`eval_with_cursor` (both
        // hardcode `JqSemantics`).
        let json = br"[10.5]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        // First stage runs on `LazyElem::Cursor` (from the array source);
        // second stage runs on `LazyElem::Owned` (the first stage's output).
        let expr = crate::jq::parse("map(. % 3) | map(. + 0)").unwrap();
        let result = eval_using::<YqSemantics, _>(&expr, value);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            // yq keeps float modulo (1.5), unlike jq's truncating modulo (1).
            OwnedValue::Array(vec![OwnedValue::Float(1.5)])
        );
    }

    #[test]
    fn test_generic_plain_map_materializes_cursor_elements_725() {
        // `LazySeq::materialize_atomic`'s `LazyElem::Cursor` arm, and
        // `into_lazy_items`'s `GenericResult::OneCursor` arm: `map(.)` on
        // an empty container (the existing #725 empty-container test) never
        // actually iterates, so neither line ever ran. A non-empty array
        // does: `.` preserves the cursor for every element.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(.)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3),
            ])
        );
    }

    #[test]
    fn test_generic_lazy_seq_inner_map_result_shapes_725() {
        // `into_lazy_items` normalizes every `GenericResult` shape a `map`
        // stage's function can produce for one element -- most of its arms
        // were never reached by any existing test (only `Owned` and
        // `Error` were, via plain arithmetic/`error(...)`).

        // `keys_unsorted` on an object element -> `LazyKeys`.
        let json = br#"[{"a":1},{"bb":2,"c":3}]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(keys_unsorted)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Array(vec![OwnedValue::String("a".to_string())]),
                OwnedValue::Array(vec![
                    OwnedValue::String("bb".to_string()),
                    OwnedValue::String("c".to_string()),
                ]),
            ])
        );

        // `keys_unsorted` on an array element -> `LazyIndexRange`.
        let json = br"[[1,2],[3,4,5]]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(keys_unsorted)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Array(vec![OwnedValue::Int(0), OwnedValue::Int(1)]),
                OwnedValue::Array(vec![
                    OwnedValue::Int(0),
                    OwnedValue::Int(1),
                    OwnedValue::Int(2)
                ]),
            ])
        );

        // `empty` -> `GenericResult::None`, dropping the element entirely.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(select(. > 1))").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Int(3)])
        );

        // A comma of literals -> `GenericResult::ManyOwned`, fanning one
        // source element into several output elements.
        let json = br"[1]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(1, 2)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
        );

        // `.[]` on an array-valued element -> `GenericResult::ManyCursor`
        // (the native `Expr::Iterate` arm), fanning one source element into
        // several *cursor* output elements -- distinct from the comma case
        // above, which only ever produces owned values.
        let json = br"[[1,2],[3]]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(.[])").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3),
            ])
        );

        // A per-element function that is itself `keys_unsorted | map(g)`
        // -> `GenericResult::LazySeq` (recursive laziness). Explicit non-goal
        // (see `into_lazy_items`'s doc comment): forced via `materialize_atomic`
        // right there instead of composing further.
        let json = br#"[{"a":1},{"bb":2,"c":3}]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(keys_unsorted | map(ascii_upcase))").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Array(vec![OwnedValue::String("A".to_string())]),
                OwnedValue::Array(vec![
                    OwnedValue::String("BB".to_string()),
                    OwnedValue::String("C".to_string()),
                ]),
            ])
        );

        // A per-element function whose own output is itself `Partial`
        // (some outputs before an error) -> the whole `map` construction
        // discards them and fails, same atomicity as a bare error.
        let json = br"[1]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse(r#"map(1, 2, error("x"))"#).unwrap();
        assert!(eval(&expr, value).materialize_lazy().is_error());
    }

    #[test]
    fn test_generic_plain_map_optional_on_non_container_is_unreachable_via_parser_725() {
        // `Builtin::Map`'s own `optional`-guarded arm mirrors `Expr::Iterate`'s
        // (both fall back to `None` instead of erroring when `optional` is
        // set) but, unlike `Expr::Iterate`, is provably unreachable through
        // the parser: `map(f)?` always parses to `Expr::Optional(Builtin::Map(f))`
        // (see `parser.rs`), and `Expr::Optional`'s own generic catch (above
        // in this file) evaluates its inner expression at the *ambient*
        // `optional` -- never forcing `true` into a bare `Builtin::Map` --
        // then converts the resulting `Error` to `None` itself. So this arm
        // never sees `optional = true` from any real query; call the
        // (private, module-internal) dispatcher directly to pin it anyway,
        // matching the same observation already made about `Builtin::Select`
        // just above this arm in the source.
        let json = br"5";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(. + 1)").unwrap();
        let result = eval_single::<JqSemantics, _>(&expr, value, true, None);
        assert!(matches!(result, GenericResult::None));
    }

    #[test]
    fn test_generic_select_condition_lazy_seq_725() {
        // `select(map(f))`: the condition itself takes the `LazySeq` fast
        // path, so `push_generic_truthiness` must materialize it once to
        // answer truthiness -- distinct from `LazyKeys`/`LazyIndexRange`'s
        // truthy-without-materializing shortcut just above it (a `LazySeq`
        // can fail, so it can't reuse that shortcut).
        let json = br"[1,2]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("select(map(. + 1))").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
        );

        let expr = crate::jq::parse(r#"select(map(error("boom")))"#).unwrap();
        assert!(eval(&expr, value).is_error());
    }

    #[test]
    fn test_generic_lazy_seq_length_propagates_control_725() {
        // The composability arm's `length`: count-and-discard over the
        // `LazySeq`, but an error/break partway through must still surface
        // as `length`'s own result, not a partial count.
        let json = br#"["a","b"]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse(r#"map(error("boom")) | length"#).unwrap();
        assert!(eval(&expr, value.clone()).is_error());

        let expr = crate::jq::parse(r"map(break $out) | length").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::Break(ref label) if label == "out"));
    }

    #[test]
    fn test_generic_lazy_seq_iterate_all_cursor_725() {
        // The composability arm's `.[]`: when every pulled element stayed a
        // cursor (e.g. `map(.)`'s identity), the whole thing answers as
        // `GenericResult::ManyCursor` -- no `to_owned` round trip.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(.) | .[]").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::ManyCursor(_)));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(1), OwnedValue::Int(2), OwnedValue::Int(3)]
        );
    }

    #[test]
    fn test_generic_lazy_seq_iterate_mixed_cursor_and_owned_725() {
        // Same `.[]` arm, heterogeneous case: one element resolves to a
        // cursor (`.foo` present), another to a computed/missing value
        // (`.foo` absent is owned `null`) -- not all-cursor, so the whole
        // thing materializes to `GenericResult::ManyOwned`, exercising the
        // `LazyElem::Cursor` sub-arm of that conversion (the existing
        // `map(ascii_upcase) | .[]` test only ever produced `Owned` items).
        let json = br#"[{"foo":1},{"other":2}]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(.foo) | .[]").unwrap();
        let result = eval(&expr, value);
        assert!(!matches!(result, GenericResult::ManyCursor(_)));
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::Int(1), OwnedValue::Null]
        );
    }

    #[test]
    fn test_generic_lazy_seq_iterate_atomicity_discards_cursor_prefix_725() {
        // Same `.[]` arm, failing case: the already-succeeded prefix (here,
        // one element whose `.foo` access resolved to a cursor, not an owned
        // value -- unlike the other atomicity test's `if/else` map function,
        // which isn't cursor-preserving) is discarded, not kept, on a later
        // element's error -- `map`'s array construction is atomic in real
        // jq, and `.[]` piped after `map` iterates the array `map` already
        // built, so it inherits that atomicity regardless of whether the
        // discarded items were cursors or owned values.
        let json = br#"[{"foo":1},42]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(.foo) | .[]").unwrap();
        let result = eval(&expr, value);
        assert!(result.is_error());
        assert_eq!(result.collect_owned().unwrap(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_generic_lazy_seq_first_and_index_zero_all_shapes_725() {
        // The composability arm's `first`/`.[0]` ("pull-one-and-stop"):
        // every arm of its own `match seq.next()` -- empty, cursor, error,
        // break -- but the existing test only ever exercised the `Owned`
        // shape (`map(ascii_upcase) | first`).
        let json = br"[]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(.) | first").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap().unwrap(),
            OwnedValue::Null
        );

        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(.) | first").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::OneCursor(_)));
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(1));

        let json = br"[42]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(.foo) | first").unwrap();
        assert!(eval(&expr, value).is_error());

        let json = br"[1]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(break $out) | first").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::Break(ref label) if label == "out"));
    }

    #[test]
    fn test_generic_first_last_expr_forward_lazy_seq_725() {
        // `first(EXPR)`/`last(EXPR)` (the explicit-argument form, distinct
        // from the no-argument `first`/`last` builtins covered above):
        // `eval_first_or_last_generic` forwards a `LazySeq` result
        // unchanged in both directions instead of materializing it, since
        // it only needs to know *which* output this is, not inspect it.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("first(map(. * 2))").unwrap();
        let result = eval(&expr, value.clone());
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(4),
                OwnedValue::Int(6),
            ])
        );

        let expr = crate::jq::parse("last(map(. * 2))").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::LazySeq(_)));
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(2),
                OwnedValue::Int(4),
                OwnedValue::Int(6),
            ])
        );
    }

    #[test]
    fn test_generic_lazy_seq_stream_json_yaml_725() {
        // `GenericResult::stream_json`/`stream_yaml`'s own `LazySeq` arm
        // (distinct from `LazyKeys`/`LazyIndexRange`'s zero-buffer writers
        // just above it -- `map`'s array construction is atomic, so this one
        // pulls the whole `LazySeq` via `materialize_atomic` first): neither
        // the success nor the `break`-control path was exercised by any
        // existing test (only the `error`-control path, via
        // `test_generic_plain_map_atomicity_725`).
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(. + 1)").unwrap();
        let result = eval(&expr, value.clone());
        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "[2,3,4]");
        assert_eq!(stats.count, 1);
        assert!(stats.any_truthy);

        let mut out = String::new();
        let stats = result
            .stream_yaml(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "[2, 3, 4]");
        assert_eq!(stats.count, 1);
        assert!(stats.any_truthy);

        let expr = crate::jq::parse("map(break $out)").unwrap();
        let result = eval(&expr, value.clone());
        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "");
        assert_eq!(stats.count, 0);
        assert!(stats.error.is_some());

        let result = eval(&expr, value);
        let mut out = String::new();
        let stats = result
            .stream_yaml(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "");
        assert!(stats.error.is_some());
    }

    #[test]
    fn test_generic_lazy_seq_materialize_lazy_break_725() {
        // `GenericResult::materialize_lazy`'s own `LazySeq` arm converts a
        // `Control::Break` into `Self::Break` -- distinct from `stream_json`/
        // `stream_yaml`'s own bespoke `LazySeq` handling above (which never
        // calls `materialize_lazy`), and from the composability arm's
        // native `length`/`first` short-circuits (which return `Break`
        // directly, before `materialize_lazy` ever runs). `into_owned`/
        // `collect_owned` are the only two callers that reach it, and both
        // route through the plain fallback shape (`_ =>
        // materialize_atomic()+eval_on_owned`), not through a top-level bare
        // `map`.
        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse("map(break $out) | last").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::Break(ref label) if label == "out"));
    }

    #[test]
    fn test_generic_lazy_seq_composability_fallback_propagates_control_725() {
        // The composability arm's final `_` fallback (`last`, `.[n]` for
        // `n != 0`, `select`, comparisons, ...): one atomic
        // `materialize_atomic` pull, then hand off to `eval_on_owned` -- but
        // an error/break during that pull must surface directly, never
        // reaching `eval_on_owned` at all. The existing test for this
        // fallback (`test_generic_lazy_seq_composability_native_consumers_725`)
        // only ever exercised the success path.
        let json = br#"[1,2,"x"]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(. + 1) | last").unwrap();
        assert!(eval(&expr, value).is_error());

        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(break $out) | last").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::Break(ref label) if label == "out"));
    }

    #[test]
    fn test_generic_lazy_seq_pipe_continuation_earlier_lazy_failure_wins_725() {
        // `Expr::Pipe`'s `ManyCursor` continuation (`.[] | REST`, `REST`
        // itself producing a fresh `LazySeq` per element via `keys_unsorted |
        // map(f)`): each cursor element is evaluated independently and
        // buffered into `per_element`. When materializing that buffer
        // (`flatten_generic_results`) later discovers an *earlier* buffered
        // `LazySeq` also fails, that earlier failure must win over whatever
        // triggered the buffer to be flattened -- it's chronologically first
        // in evaluation order. None of these interactions (earlier-fails
        // alongside a later immediate error, or alongside a normal
        // non-early-returning flatten) were exercised by any existing test.
        let json = br#"[{"b":1,"a":2}, 42]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        // Element 0 (`{"b":1,"a":2}`) takes the `keys_unsorted | map(f)`
        // fast path and stays a raw, unmaterialized `LazySeq` in
        // `per_element` -- `f` fails once actually pulled (on key `"b"`).
        // Element 1 (`42`, not an object/array) fails `keys_unsorted`
        // immediately, triggering the early return that must first flatten
        // (and thus fail on) element 0's still-buffered `LazySeq` -- whose
        // error wins over element 1's own.
        let expr = crate::jq::parse(
            r#".[] | (keys_unsorted | map(if . == "b" then error("early") else . end))"#,
        )
        .unwrap();
        let result = eval(&expr, value.clone());
        assert!(result.is_error());
        assert_eq!(result.error().unwrap().message, "early");

        // Same shape, but element 0's buffered `LazySeq` fails via `break`
        // instead of `error` -- the earlier `Break` must still win over
        // element 1's immediate `Error`.
        let expr = crate::jq::parse(
            r#".[] | (keys_unsorted | map(if . == "b" then break $out else . end))"#,
        )
        .unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::Break(ref label) if label == "out"));
    }

    #[test]
    fn test_generic_lazy_seq_pipe_continuation_flatten_after_full_scan_725() {
        // Same `ManyCursor` continuation as above, but with no element
        // triggering an early return at all: every element resolves to a
        // raw, buffered `LazySeq`, so `flatten_generic_results` only runs
        // once, after the loop over every cursor finishes. A failure
        // discovered there (as opposed to the early-return paths covered by
        // `..._earlier_lazy_failure_wins_725` above) is a distinct code
        // path (the fallthrough at the bottom of the `ManyCursor` arm).
        let json = br#"[{"b":1},{"a":2}]"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse(
            r#".[] | (keys_unsorted | map(if . == "b" then error("boom") else . end))"#,
        )
        .unwrap();
        let result = eval(&expr, value.clone());
        assert!(result.is_error());
        assert_eq!(result.error().unwrap().message, "boom");

        let expr = crate::jq::parse(
            r#".[] | (keys_unsorted | map(if . == "b" then break $out else . end))"#,
        )
        .unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::Break(ref label) if label == "out"));
    }

    #[test]
    fn test_generic_result_into_owned_halt_variant() {
        // `GenericResult::into_owned`'s `Self::Halt(_) => None` arm (#791):
        // mirrors the `Error`/`Break`/`Partial` arms around it -- a halt
        // isn't representable as a single output value regardless of its
        // code. Reachable only from this module's own `eval()` (cursor-less)
        // entry point: `jq_runner.rs`/`yq_runner.rs` only ever call
        // `eval_with_cursor`/`eval_with_cursor_using`, and `into_owned()`
        // itself is never called from either CLI runner at all -- only from
        // this file's own tests (`GenericResult::collect_owned()` is the
        // method the CLI actually uses).
        let json = br"null";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(&crate::jq::parse("halt_error(3)").unwrap(), value);
        match &result {
            GenericResult::Halt(code) => assert_eq!(*code, 3),
            other => panic!("expected Halt(3), got {other:?}"),
        }
        assert_eq!(result.into_owned().unwrap(), None);
    }

    #[test]
    fn test_generic_lazy_seq_computed_index_never_reaches_a_later_halt() {
        // Same LazySeq-over-a-computed-index family as
        // `test_generic_lazy_seq_computed_index_swallows_error_724` above,
        // but for `halt`/`halt_error` (#791) instead of `error(...)`: `[0]`
        // only needs the map's first element ("a"), so
        // `materialize_lazy()`/`collect_owned()` never advance the
        // underlying `LazySeq` far enough to touch the second element's
        // `halt_error(7)` at all -- it isn't swallowed, it's simply never
        // evaluated. Diverges from real jq, which builds the whole array
        // eagerly before indexing and so *does* hit the halt: `jq -c
        // '(keys_unsorted | map(if . == "b" then halt_error(7) else . end))
        // [0]'` on `{"a":1,"b":2}` exits 7 with `b` on stderr (verified
        // live). Confirmed live against this binary too: `succinctly jq -c`
        // on the same query prints `"a"` and exits 0 -- documented here as
        // an accepted laziness-driven divergence, not fixed.
        let json = br#"{"a":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let expr = crate::jq::parse(
            r#"(keys_unsorted | map(if . == "b" then halt_error(7) else . end))[0]"#,
        )
        .unwrap();
        let result = eval(&expr, value);
        assert!(
            !matches!(result, GenericResult::Halt(_)),
            "the halt is never reached, not swallowed after being reached: {result:?}"
        );
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![OwnedValue::String("a".to_string())]
        );
    }

    #[test]
    fn test_generic_pipe_bare_many_stage_halts_immediately() {
        // `eval_single`'s `Expr::Pipe` handling, the `GenericResult::
        // Many(vs)` stage arm's own `GenericResult::Halt(code) => return
        // partial_generic(results, Control::Halt(code))` sub-case (#791) --
        // sibling of
        // `test_json_multi_stage_pipe_first_stage_bare_many_without_cursor`
        // above (same `select(true,true)` source for a bare, cursor-less
        // `Many`), but piping into `halt` instead of `length`. Reachable
        // only from the cursor-less `eval()` entry point: `select`'s
        // `pass_n` only ever constructs a bare `Many` when its own `cursor`
        // parameter is `None`, which only happens once an *earlier* stage
        // has already produced a bare `One` -- and `eval_with_cursor`/
        // `eval_with_cursor_using` (what both CLI runners actually call)
        // always start with `Some`.
        let json = br"1";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();

        let result = eval(
            &crate::jq::parse("select(true,true) | halt").unwrap(),
            value,
        );
        assert!(matches!(result, GenericResult::Halt(0)));
    }

    /// `{"k":{"k":...{}...}}`, `depth` levels of `"k"` nesting, terminating
    /// in `{}` — mirrors `jq_recurse_depth_tests.rs`'s own `linear_nest`.
    fn linear_nest(depth: usize) -> String {
        format!("{}{{}}{}", "{\"k\":".repeat(depth), "}".repeat(depth))
    }

    /// #998: `to_owned`/`to_owned_cursor` must not recurse unbounded on
    /// adversarially deep input — confirmed live, `succinctly jq '.'` on a
    /// 200,000-level-deep document used to abort with a raw stack overflow
    /// (SIGABRT) before this guard existed. 255 levels (just under the
    /// limit) must still materialize normally; 256 must panic cleanly
    /// rather than recurse further.
    #[test]
    fn to_owned_panics_past_nesting_depth_limit_998() {
        let json = linear_nest(255);
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        // Under the limit: succeeds.
        let owned = to_owned(&cursor.value()).unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));
        let owned = to_owned_cursor(&cursor).unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));

        let json = linear_nest(256);
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let value = cursor.value();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| to_owned(&value)));
        assert!(result.is_err(), "to_owned should panic at depth 256");
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| to_owned_cursor(&cursor)));
        assert!(result.is_err(), "to_owned_cursor should panic at depth 256");
    }

    /// #1017: `owned_from_standard_json` is a third, independent copy of the
    /// cursor-to-`OwnedValue` conversion `to_owned`/`to_owned_cursor` are
    /// already guarded above (#998) -- same limit, same construction, its
    /// own guard.
    #[test]
    fn owned_from_standard_json_panics_past_nesting_depth_limit_1017() {
        let json = linear_nest(255);
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let owned = owned_from_standard_json(&cursor.value()).unwrap();
        assert!(matches!(owned, OwnedValue::Object(_)));

        let json = linear_nest(256);
        let index = JsonIndex::build(json.as_bytes());
        let cursor = index.root(json.as_bytes());
        let value = cursor.value();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            owned_from_standard_json(&value)
        }));
        assert!(
            result.is_err(),
            "owned_from_standard_json should panic at depth 256"
        );
    }

    /// #1192: unlike `to_owned`/`to_owned_cursor` (this file, still silently
    /// degrading to `OwnedValue::Null` -- see
    /// `test_to_owned_degrades_to_null_on_string_decode_failure_1098` above)
    /// and `cursor_to_owned` (`lazy.rs`, still degrading to an empty
    /// string), `owned_from_standard_json` now surfaces a genuinely
    /// undecodable string as an `EvalError` instead of silently
    /// materializing `""`. A nested occurrence (inside an array) propagates
    /// the same way, since the whole containing value can't be represented
    /// once any of its scalars can't be.
    ///
    /// No CLI-level regression test accompanies this: extensive live probing
    /// (~25 distinct jq expressions over a document containing exactly this
    /// malformed byte sequence, covering navigation, construction,
    /// `reduce`/`foreach`, assignment, and every builtin category this
    /// crate implements) found no ordinary top-level jq syntax that reaches
    /// `owned_from_standard_json` for a *document-sourced* value --
    /// `eval_generic.rs`'s own native dispatch (which uses `to_owned`,
    /// unaffected by this fix) already handles every case tried. This
    /// function is reached only via the "reindex bridge" (`eval_on_owned`/
    /// `eval_single`'s full-evaluator fallback) on a value *constructed*
    /// mid-evaluation (e.g. a `reduce`/`foreach` accumulator) that itself
    /// then needs an expression `eval_generic` can't handle natively --
    /// unlike `to_owned`'s doc comment (#1098), which explicitly confirms
    /// live-CLI reachability "through completely ordinary CLI usage (any
    /// non-identity query over a document containing such a string)", this
    /// function's own doc comment makes no such claim.
    #[test]
    fn test_owned_from_standard_json_errors_on_string_decode_failure_1192() {
        let json: &[u8] = b"{\"a\": \"\xff\xfe\"}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = owned_from_standard_json(&value).unwrap_err();
        assert!(err.message.contains("invalid UTF-8"), "{err:?}");

        let json: &[u8] = b"[1, \"\xff\xfe\", 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = owned_from_standard_json(&value).unwrap_err();
        assert!(err.message.contains("invalid UTF-8"), "{err:?}");
    }

    /// #1192: an object key that passes structural validation but fails to
    /// *decode* now errors too, instead of silently dropping the whole
    /// field (the pre-#1192 behavior, still true of `to_owned`/
    /// `to_owned_cursor`/`cursor_to_owned` -- see the sibling test above). A
    /// key that is *not* a string at all (structurally malformed, e.g. a
    /// bare non-string token) is a separate, still-open gap (#1194) and is
    /// deliberately left alone -- this fix only covers a string-shaped key
    /// that failed to decode.
    #[test]
    fn test_owned_from_standard_json_errors_on_object_key_decode_failure_1192() {
        let json: &[u8] = b"{\"\xff\xfe\": 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = owned_from_standard_json(&value).unwrap_err();
        assert!(
            err.message.contains("invalid UTF-8") && err.message.contains("object key"),
            "{err:?}"
        );
    }

    /// #1194: a key that isn't `StandardJson::String` at all (structurally
    /// malformed, not a decode failure) raises instead of silently dropping
    /// the field.
    ///
    /// This test asserted the drop when #1192 wrote it, as that fix's
    /// deliberately-out-of-scope neighbour. It is inverted here, not merely
    /// updated: dropping the field deleted it from output while `length` went
    /// on counting it -- the same disagreement #1385's own postmortem records
    /// as the failure mode to avoid.
    ///
    /// `{123: 1, "b": 2}` is used rather than #1194's own `{invalid}` repro
    /// because the two reach *different* checks: a bare numeric key with a
    /// valid sibling exercises the per-field key test below, while
    /// `{invalid}`'s lone child never pairs into a `JsonField` at all and is
    /// caught by the `unpaired_tail` check after the loop (covered by
    /// `test_owned_from_standard_json_raises_on_unpaired_field_1194`).
    #[test]
    fn test_owned_from_standard_json_raises_on_malformed_key_1194() {
        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = owned_from_standard_json(&value).expect_err("a bare numeric key is not JSON");
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
        // The strict validator's own diagnosis, not a message reconstructed
        // at the materializer.
        assert!(
            err.message.contains("expected string key"),
            "message: {}",
            err.message
        );
    }

    /// #1194: an object whose children don't pair -- `{invalid}`, `{"a"}` --
    /// raises rather than materializing as `{}`.
    ///
    /// The sibling of the test above, covering the *other* swallow point:
    /// `JsonFields::uncons` collapses "no sibling to pair as a value" into
    /// "no more fields", which is indistinguishable from a genuinely empty
    /// object unless someone asks `unpaired_tail`.
    #[test]
    fn test_owned_from_standard_json_raises_on_unpaired_field_1194() {
        for json in [&b"{invalid}"[..], &b"{\"a\"}"[..]] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let value = cursor.value();
            let err = owned_from_standard_json(&value).expect_err("an unpaired member is not JSON");
            assert!(
                err.message.contains("Invalid JSON text"),
                "input {:?} gave: {}",
                core::str::from_utf8(json),
                err.message
            );
        }
    }

    /// #1192: the `Ok` side of `eval_on_owned`'s `QueryResult::One` arm --
    /// the decode-failure tests above only exercise its `Err` side.
    /// `keys_unsorted | sort` isn't one of `LazyKeys`'s dedicated fast-path
    /// arms (only `.[]`/computed-index/`first`/`last`/a leading `map` are),
    /// so it materializes and routes through `eval_on_owned`'s "reindex
    /// bridge" -- confirmed by direct instrumentation while developing this
    /// test, not just inferred from the doc comments.
    #[test]
    fn test_eval_on_owned_one_ok_via_keys_sort_1192() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse("keys_unsorted | sort").unwrap();
        let result = eval(&expr, value);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );
    }

    /// #1192: the `Ok` (all-succeed) side of `eval_on_owned`'s `QueryResult::
    /// Many` loop -- same reasoning as the `One` test above, but for a query
    /// that produces multiple outputs through the same bridge.
    #[test]
    fn test_eval_on_owned_many_ok_via_keys_comma_index_1192() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse("keys_unsorted | (.[0], .[1])").unwrap();
        let result = eval(&expr, value);
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![
                OwnedValue::String("b".to_string()),
                OwnedValue::String("a".to_string()),
            ]
        );
    }

    /// #1192: the `Ok` side of `eval_single`'s *own* full-evaluator fallback
    /// arm (a duplicate of `eval_on_owned`'s bridge, see the comment on that
    /// arm) -- `reduce`'s own accumulator handling routes here directly,
    /// without going through `eval_on_owned` at all (confirmed by direct
    /// instrumentation), so it needs its own dedicated coverage.
    #[test]
    fn test_eval_single_fallback_one_ok_via_reduce_1192() {
        let json = br"[1, 2, 3]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse("reduce .[] as $x (0; $x)").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(3));
    }

    /// #1192: the `Ok` (all-succeed) side of `eval_single`'s fallback
    /// `QueryResult::Many` loop -- `foreach`'s per-step output list routes
    /// here directly, same as the `reduce` case above.
    #[test]
    fn test_eval_single_fallback_many_ok_via_foreach_1192() {
        let json = br"null";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse(r#"foreach range(3) as $i (""; . + "x"; .)"#).unwrap();
        let result = eval(&expr, value);
        assert_eq!(
            result.collect_owned().unwrap(),
            vec![
                OwnedValue::String("x".to_string()),
                OwnedValue::String("xx".to_string()),
                OwnedValue::String("xxx".to_string()),
            ]
        );
    }

    /// #1192: the `Ok` side of `eval_on_owned`'s `QueryResult::OneCursor`
    /// arm -- `keys_unsorted | .` re-evaluates the identity against the
    /// freshly-reindexed materialized-keys JSON inside the bridge, which
    /// `full_eval` resolves as a cursor rather than a decoded value
    /// (confirmed by direct instrumentation while developing this test).
    #[test]
    fn test_eval_on_owned_onecursor_ok_via_keys_identity_1192() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse("keys_unsorted | .").unwrap();
        let result = eval(&expr, value);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("b".to_string()),
                OwnedValue::String("a".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );
    }

    // ---- #1247 coverage: decode-failure arms added by the fallible
    // `to_owned`/`to_owned_cursor` signature change (PR #1391). Every test
    // below constructs a genuinely-reachable path to its target arm rather
    // than forcing coverage artificially -- see each test's own doc comment
    // for the trace. Several deliberately call the crate's cursor-less
    // [`eval`] (not [`eval_with_cursor`]) directly: that is a real, `pub`,
    // heavily-exercised entry point elsewhere in this test module (see
    // `test_1048_computed_index_and_slice_zero_results_collapse_to_none`
    // above and dozens like it), and it is the only way to construct a bare
    // `GenericResult::One`/`Many` (as opposed to `OneCursor`/`ManyCursor`) --
    // every native arm that can build one forwards a `Some` cursor into the
    // cursor-carrying variant instead, so `One`/`Many` only ever arise when
    // the ambient cursor is `None` to begin with.

    /// `to_owned_with_comments` mirrors `to_owned`'s key handling -- this
    /// covers the object-key case specifically, since #1247 added its check
    /// to this function too, not just `to_owned_at_depth`, and #1642
    /// changed both from raising to preserving in the same way.
    #[test]
    fn test_to_owned_with_comments_preserves_object_key_decode_failure_1642() {
        let json: &[u8] = b"{\"\xff\xfe\": 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let (owned, comments) = to_owned_with_comments(&value, Some(&cursor))
            .expect("an undecodable key is preserved, not raised on");
        assert_eq!(
            owned,
            OwnedValue::Object(IndexMap::from([(
                "\u{FFFD}\u{FFFD}".to_string(),
                OwnedValue::from_number_literal("1")
            )]))
        );
        let CommentTree::Object(_, comment_map, key_comment_map) = comments else {
            panic!("expected CommentTree::Object: {comments:?}");
        };
        assert!(
            comment_map.contains_key("\u{FFFD}\u{FFFD}"),
            "{comment_map:?}"
        );
        assert!(key_comment_map.is_empty(), "{key_comment_map:?}");
    }

    /// The value-side sibling of the test above: a field whose *value* (not
    /// key) fails to decode propagates through the recursive
    /// `to_owned_with_comments_at_depth` call for that field, not just the
    /// top-level scalar case `to_owned`'s own tests already cover.
    #[test]
    fn test_to_owned_with_comments_errors_on_field_value_decode_failure_1247() {
        let json: &[u8] = b"{\"a\": \"\xff\xfe\"}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = to_owned_with_comments(&value, Some(&cursor))
            .expect_err("an undecodable value must not materialize");
        assert!(err.message.contains("invalid UTF-8"), "{err:?}");
    }

    /// #1194: `to_owned_with_comments` mirrors `to_owned`/`to_owned_cursor`'s
    /// non-string-key check (`test_both_owned_conversions_raise_on_non_string_key_1194`)
    /// -- this is the third conversion reaching the same `key_display_string`
    /// `else` arm, for a key the format's grammar never allowed at all, as
    /// opposed to #1642's decode-failure preservation covered above.
    #[test]
    fn test_to_owned_with_comments_raises_on_non_string_key_1194() {
        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = to_owned_with_comments(&value, Some(&cursor))
            .expect_err("a bare numeric key is not JSON");
        assert!(
            err.message.contains("expected string key"),
            "message: {}",
            err.message
        );
    }

    /// #1194: an object whose children don't pair (`{invalid}`) raises from
    /// `to_owned_with_comments` too -- this function's own `ends_unpaired`
    /// check, added alongside the #1642 preserve-not-raise change and until
    /// now untested. `to_owned`/`to_owned_cursor` already had this guard; see
    /// `test_owned_from_standard_json_raises_on_unpaired_field_1194` for the
    /// value-domain sibling using the same repro.
    #[test]
    fn test_to_owned_with_comments_raises_on_unpaired_field_1194() {
        let json: &[u8] = b"{invalid}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let err = to_owned_with_comments(&value, Some(&cursor))
            .expect_err("an unpaired member is not JSON");
        assert!(err.message.contains("Invalid JSON text"), "{err:?}");
    }

    /// `to_owned_key_shape`'s array/object branches (#626/#670/#903) are a
    /// shape-only fast path for a computed index/slice-bound candidate --
    /// reached from `eval_index_expr`'s `keys` match when the key expression
    /// resolves to a bare `GenericResult::One` (not `OneCursor`), which only
    /// happens when the ambient cursor is `None`: `.[.]` at the top level,
    /// evaluated through the cursor-less `eval()`, makes the key expression
    /// (`.`, i.e. `Expr::Identity`) evaluate against a `None` cursor, giving
    /// `GenericResult::One(document)`. An array/object document then hits
    /// `to_owned_key_shape`'s array/object branch respectively; the
    /// synthesized empty container is then rejected by `index_one_owned`
    /// (neither branch there handles an array/object key), confirming the
    /// fast path's synthesized shape reaches real indexing logic rather than
    /// disappearing into an unused value.
    #[test]
    fn test_computed_index_self_as_key_rejects_array_and_object_shapes_1247() {
        let array_json: &[u8] = b"[1,2,3]";
        let index = JsonIndex::build(array_json);
        let cursor = index.root(array_json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse(".[.]").unwrap(), value);
        assert!(
            result.is_error(),
            "array self-index should be rejected, got {result:?}"
        );

        let object_json: &[u8] = b"{\"a\":1}";
        let index = JsonIndex::build(object_json);
        let cursor = index.root(object_json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse(".[.]").unwrap(), value);
        assert!(
            result.is_error(),
            "object self-index should be rejected, got {result:?}"
        );
    }

    /// `Expr::Array`'s native construction (#1168) materializes its inner
    /// expression's `One`/`Many` results through `to_owned` -- reached the
    /// same cursor-less way as the test above: `[.]` through `eval()`
    /// (cursor `None`) makes `.` resolve to a bare `One`, and `[select(true,
    /// true)]` makes the two-truthy-output `select` resolve to a bare
    /// `Many` (`Builtin::Select`'s `pass_n` forwards a `None` cursor into
    /// `Many`, not `ManyCursor`) -- both over a document that is itself an
    /// undecodable string.
    #[test]
    fn test_array_construction_one_and_many_arms_decode_failure_1247() {
        let json: &[u8] = b"\"\xff\xfe\"";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("[.]").unwrap(), value);
        assert!(result.is_error(), "[.] should surface the decode failure");

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("[select(true,true)]").unwrap(), value);
        assert!(
            result.is_error(),
            "[select(true,true)] should surface the decode failure"
        );
    }

    /// `push_generic_owned_values` (`Expr::Compare`'s operand forker, #768)
    /// materializes each operand's outputs through `to_owned`/`to_owned`;
    /// its `One` arm is reached by `. == 1` (left operand is a bare `One`,
    /// same cursor-less mechanism as the tests above), its `Many` arm by
    /// `select(true,true) == 1` (left operand is a bare `Many`).
    #[test]
    fn test_compare_operand_one_and_many_arms_decode_failure_1247() {
        let json: &[u8] = b"\"\xff\xfe\"";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse(". == 1").unwrap(), value);
        assert!(
            result.is_error(),
            ". == 1 should surface the decode failure"
        );

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("select(true,true) == 1").unwrap(), value);
        assert!(
            result.is_error(),
            "select(true,true) == 1 should surface the decode failure"
        );
    }

    /// `push_generic_truthiness`'s `Many` arm (`select`'s condition forker,
    /// #378) is reached when the condition itself is a bare `Many` --
    /// `select(select(true,true))` (the inner `select` supplies the outer
    /// one's condition, and resolves to a bare `Many` the same way the
    /// `Compare` test above does).
    #[test]
    fn test_select_condition_many_arm_decode_failure_1247() {
        let json: &[u8] = b"\"\xff\xfe\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(
            &crate::jq::parse("select(select(true,true))").unwrap(),
            value,
        );
        assert!(
            result.is_error(),
            "select(select(true,true)) should surface the decode failure"
        );
    }

    /// `GenericResult::into_owned`'s `Many` arm (public API, used by every
    /// non-streaming consumer) -- `select(true,true)` over an undecodable
    /// document gives a bare `Many` (see the tests above for why), and
    /// `.into_owned()` must report the decode failure as `Err`, not silently
    /// drop it into the `Ok(None)` bucket `Error`/`Break` share.
    #[test]
    fn test_into_owned_many_arm_decode_failure_1247() {
        let json: &[u8] = b"\"\xff\xfe\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("select(true,true)").unwrap(), value);
        let err = result
            .into_owned()
            .expect_err("a Many of undecodable values must not materialize");
        assert!(err.message.contains("invalid UTF-8"), "{err:?}");
    }

    /// `stream_json`/`stream_yaml`'s `One`/`Many` arms (#355, #1247) report a
    /// decode failure through `stats.error` rather than writing to `out` --
    /// same bare-`One`/bare-`Many` construction as the tests above
    /// (`select(true)`/`select(true,true)` through the cursor-less `eval()`),
    /// exercised against both output formats since the match arms (and the
    /// shared `owned_or_stream_error` helper, #2532) are format-independent.
    #[test]
    fn test_stream_one_and_many_arms_report_decode_failure_1247() {
        let json: &[u8] = b"\"\xff\xfe\"";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let one = eval(&crate::jq::parse("select(true)").unwrap(), value);
        assert!(matches!(one, GenericResult::One(_)), "{one:?}");
        let mut out = String::new();
        let stats = one
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "", "a decode failure must never reach stdout");
        assert!(stats.error.is_some());
        let mut out = String::new();
        let stats = one
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "");
        assert!(stats.error.is_some());

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let many = eval(&crate::jq::parse("select(true,true)").unwrap(), value);
        assert!(matches!(many, GenericResult::Many(_)), "{many:?}");
        let mut out = String::new();
        let stats = many
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "");
        assert!(stats.error.is_some());
        assert_eq!(stats.count, 0);
        let mut out = String::new();
        let stats = many
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "");
        assert!(stats.error.is_some());
        assert_eq!(stats.count, 0);
    }

    /// `stream_json`/`stream_yaml`'s `LazyKeys { sorted: true, .. }` arm
    /// falls back to `materialize_lazy_keys`, which decodes every key --
    /// `keys` (not `keys_unsorted`, which stays lazy) over a document with an
    /// undecodable key reaches this on both output formats. CLI-reachable:
    /// `sjq keys`/`syq keys` over such a document. #1247 raised here;
    /// #1642 preserves the key via its raw source span instead, matching
    /// every other `keys`/`length`/`.` route.
    #[test]
    fn test_stream_sorted_lazy_keys_arm_preserves_decode_failure_1642() {
        let json: &[u8] = b"{\"\xff\xfe\": 1}";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("keys").unwrap(), cursor);
        assert!(matches!(
            result,
            GenericResult::LazyKeys { sorted: true, .. }
        ));

        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert!(stats.error.is_none(), "{:?}", stats.error);
        assert_eq!(out, "[\"\u{FFFD}\u{FFFD}\"]");

        let mut out = String::new();
        let stats = result
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert!(stats.error.is_none(), "{:?}", stats.error);
        assert_eq!(out, "- \u{FFFD}\u{FFFD}");
    }

    /// The sibling of the test above with a genuinely #1194-malformed key:
    /// `stream_json`/`stream_yaml`'s `LazyKeys { sorted: true, .. }` arm
    /// still raises through `materialize_lazy_keys`'s `Err` side (#1642 only
    /// relaxed the decode-failure case above, not this structural one) --
    /// nothing reaches `out` on either output format, matching every other
    /// `keys`/`length`/`.` route's agreement on a malformed member (#1194).
    #[test]
    fn test_stream_sorted_lazy_keys_arm_raises_on_malformed_key_1194() {
        let json: &[u8] = b"{123: 1}";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("keys").unwrap(), cursor);
        assert!(matches!(
            result,
            GenericResult::LazyKeys { sorted: true, .. }
        ));

        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert!(stats.error.is_some());
        assert_eq!(out, "");

        let mut out = String::new();
        let stats = result
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert!(stats.error.is_some());
        assert_eq!(out, "");
    }

    /// #1679: the `sorted: false` sibling of the test above -- the genuinely
    /// lazy path (`stream_lazy_keys_json`/`stream_lazy_keys_yaml`, #685)
    /// used to silently skip a #1194 key instead of raising, disagreeing
    /// with `keys`'s `sorted: true` arm on the very same document. Both
    /// output formats must now report the failure via `stats.error` too,
    /// keeping whatever prefix (nothing, here, since the malformed key is
    /// the only one) was already written.
    #[test]
    fn test_stream_unsorted_lazy_keys_arm_raises_on_malformed_key_1679() {
        let json: &[u8] = b"{123: 1}";

        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("keys_unsorted").unwrap(), cursor);
        assert!(matches!(
            result,
            GenericResult::LazyKeys { sorted: false, .. }
        ));

        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert!(stats.error.is_some());
        assert_eq!(out, "[]");

        let mut out = String::new();
        let stats = result
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert!(stats.error.is_some());
        assert_eq!(out, "");
    }

    /// `GenericResult::materialize_lazy`'s `LazyKeys` arm `Err` side:
    /// reached whenever a `LazyKeys` result is materialized by a consumer
    /// other than `stream_json`/`stream_yaml`/`fold_lazy_keys_stage` (which
    /// each have their own dedicated tests) -- here, `push_generic_owned_values`
    /// inside `Expr::Comma`'s evaluation. `keys, 1` puts a bare `keys` as
    /// the comma's first operand over a document with a #1194-malformed
    /// key; `push_generic_owned_values` calls `materialize_lazy` on it
    /// before `to_owned`/`to_owned_cursor` ever run, converting the
    /// `LazyKeys` into `GenericResult::Error` directly.
    #[test]
    fn test_materialize_lazy_keys_arm_raises_on_malformed_key_1194() {
        let json: &[u8] = b"{123: 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("keys, 1").unwrap(), cursor);
        assert!(result.is_error(), "{result:?}");
    }

    /// `stream_json`'s `LazySeq` arm falls back to materializing each
    /// `LazyElem` via `lazy_elem_to_owned` whenever
    /// `sequence_streamable_cursors` answers `None` -- for JSON, since #1576,
    /// that means at least one drained item is `LazyElem::Owned` rather than
    /// `LazyElem::Cursor` (`JsonCursor::supports_sequence_streaming` is now
    /// `true`, mirroring `YamlCursor`'s own #757 answer), exactly the same
    /// condition the YAML sibling test below documents. `map(.x)` over
    /// `[{"x": <undecodable>}, null]` produces exactly that mix: `.x` on the
    /// object element stays a lazy `OneCursor` (into `LazyElem::Cursor`),
    /// while `.x` on `null` returns `GenericResult::Owned(Null)` (jq's
    /// "field access on null is null" rule, into `LazyElem::Owned`) without
    /// ever decoding anything -- forcing the fallback to run, which is
    /// atomic by construction (`lazy_elem_to_owned` collects every element
    /// into one `Result<Vec<_>, _>` before `stream_json` is ever called on
    /// the result, so a decode failure anywhere means nothing reaches `out`
    /// at all). A *plain* `map(.)` no longer reaches this arm at all -- see
    /// `test_stream_json_lazyseq_cursor_path_keeps_partial_prefix_1576`
    /// just below for that path's own (different, already-accepted)
    /// contract.
    #[test]
    fn test_stream_json_lazyseq_fallback_reports_decode_failure_1247() {
        let json: &[u8] = b"[{\"x\": \"\xff\xfe\"}, null]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("map(.x)").unwrap(), cursor);
        assert!(matches!(result, GenericResult::LazySeq(_)));

        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "");
        assert!(stats.error.is_some());
    }

    /// The cursor-streaming counterpart of the fallback test just above
    /// (#1576): a plain `map(.)` over `["\xff\xfe"]` is now all-`Cursor`
    /// (identity never produces an `Owned` element), so
    /// `sequence_streamable_cursors` answers `Some` and this renders
    /// through `JsonCursor::stream_sequence_json` instead of the atomic
    /// fallback. That writer streams each element to `out` as it goes, the
    /// same non-atomic, keep-the-prefix-and-diagnose trade
    /// `stream_maybe_colored`'s own doc comment documents for YAML's
    /// identical cursor path (#1641/#1679) -- `[` is already written by the
    /// time the one element's decode failure is discovered, and it stays
    /// written rather than being retroactively unwritten. This pins that
    /// this is the deliberate, accepted contract for the cursor path (not a
    /// regression to notice and "fix" later): whole-array atomicity is a
    /// property of the *fallback* arm specifically, not of `stream_json`'s
    /// `LazySeq` handling as a whole.
    #[test]
    fn test_stream_json_lazyseq_cursor_path_keeps_partial_prefix_1576() {
        let json: &[u8] = b"[\"\xff\xfe\"]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("map(.)").unwrap(), cursor);
        assert!(matches!(result, GenericResult::LazySeq(_)));

        let mut out = String::new();
        let stats = result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, "[", "the opening bracket stays written on failure");
        assert!(stats.error.is_some());
    }

    /// `stream_yaml`'s `LazySeq` arm equivalent of the JSON test above, but
    /// `YamlCursor::supports_sequence_streaming` is `true`, so
    /// `sequence_streamable_cursors` only answers `None` when at least one
    /// drained item is `LazyElem::Owned` rather than `LazyElem::Cursor`.
    /// `map(.x)` over `[{x: "\ud800"}, null]` produces exactly that mix:
    /// `Expr::Field` on the object element stays a lazy `OneCursor` (into
    /// `LazyElem::Cursor`), while `Expr::Field` on `null` returns
    /// `GenericResult::Owned(Null)` (jq's "field access on null is null"
    /// rule, into `LazyElem::Owned`) without ever decoding anything --
    /// forcing the fallback to run and discover the first element's decode
    /// failure (a lone UTF-16 surrogate, same invalid-escape shape used
    /// throughout this file's JSON-side #1247 tests) only once it actually
    /// materializes.
    #[test]
    fn test_stream_yaml_lazyseq_fallback_reports_decode_failure_1247() {
        use crate::yaml::YamlIndex;

        let yaml: &[u8] = br#"[{x: "\ud800"}, null]"#;
        let index = YamlIndex::build(yaml).unwrap();
        let cursor = index
            .root(yaml)
            .first_child()
            .expect("YAML document should have content");
        let result = eval_with_cursor(&crate::jq::parse("map(.x)").unwrap(), cursor);
        assert!(matches!(result, GenericResult::LazySeq(_)));

        let mut out = String::new();
        let stats = result
            .stream_yaml(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, "");
        assert!(stats.error.is_some());
    }

    /// `fold_pipe_stages`'s `GenericResult::Many(vs)` arm re-evaluates the
    /// next pipe stage per element with the ambient cursor forced to `None`
    /// (`vs: Vec<V>` carries no cursor for its elements) -- so the *next*
    /// stage's own result can itself be a bare `One`/`Many`/`ManyCursor`,
    /// covering every nested-decode-failure arm this loop has. All four
    /// cases below start from `select(true,true)` (bare `Many` of two copies
    /// of the current value, via the cursor-less `eval()`) piped into a
    /// second stage chosen so that stage's *own* evaluation (with the forced
    /// `None` cursor) lands in a specific `GenericResult` shape:
    /// - `.` (`Expr::Identity`) with `None` cursor => bare `One`.
    /// - `.x` (`Expr::Field`) ignores the ambient cursor entirely, deriving
    ///   its own fresh cursor from the object structurally => `OneCursor`.
    /// - `select(true,true)` again (nested) => bare `Many`.
    /// - `.[]` (`Expr::Iterate`) also ignores the ambient cursor => `ManyCursor`.
    #[test]
    fn test_fold_pipe_stages_many_arm_nested_shapes_decode_failure_1247() {
        // `.` => bare `One`, decoding *successfully* -- the `Ok` side of
        // the same match arm the failing case right below exercises the
        // `Err` side of. Both copies `select(true,true)` produces are the
        // same decodable value, so this must collect both rather than
        // erroring.
        let json: &[u8] = b"\"hello\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("select(true,true) | .").unwrap(), value);
        assert_eq!(
            result.into_owned().unwrap().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("hello".to_string()),
                OwnedValue::String("hello".to_string()),
            ])
        );

        // `.` => bare `One`.
        let json: &[u8] = b"\"\xff\xfe\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("select(true,true) | .").unwrap(), value);
        assert!(result.is_error(), "{result:?}");

        // `.x` => `OneCursor`.
        let json: &[u8] = b"{\"x\": \"\xff\xfe\"}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("select(true,true) | .x").unwrap(), value);
        assert!(result.is_error(), "{result:?}");

        // Nested `select(true,true)` => bare `Many`.
        let json: &[u8] = b"\"\xff\xfe\"";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(
            &crate::jq::parse("select(true,true) | select(true,true)").unwrap(),
            value,
        );
        assert!(result.is_error(), "{result:?}");

        // `.[]` => `ManyCursor`.
        let json: &[u8] = b"[\"\xff\xfe\"]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let result = eval(&crate::jq::parse("select(true,true) | .[]").unwrap(), value);
        assert!(result.is_error(), "{result:?}");
    }

    /// `fold_lazy_keys_stage`'s catch-all `_` arm (#1565) materializes every
    /// key via `materialize_lazy_keys` for any following stage that isn't
    /// one of the dedicated fast-path arms (`first`/`last`/`.[n]`/a leading
    /// `map`, all guarded `!sorted`) -- `keys` (sorted) always falls here
    /// regardless of what follows, since every one of those fast-path
    /// guards is `if !sorted`. Not `keys | length`: `Builtin::Length` has
    /// its own dedicated arm *without* a `!sorted` guard
    /// (`effective_len`, #1514), which counts distinct keys without
    /// decoding any of them and so never reaches this catch-all at all --
    /// confirmed live (an earlier draft of this test used `length` and
    /// failed to observe the decode error for exactly this reason). `.[0]`
    /// does reach it: its own dedicated fast-path arm is guarded `!sorted`
    /// too, so with `keys` (sorted) it falls through the same as any other
    /// non-`length` stage. CLI-reachable: `sjq 'keys | .[0]'`.
    #[test]
    fn test_fold_lazy_keys_stage_catchall_preserves_decode_failure_1642() {
        let json: &[u8] = b"{\"\xff\xfe\": 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("keys | .[0]").unwrap(), cursor);
        match result {
            GenericResult::Owned(OwnedValue::String(s)) => assert_eq!(s, "\u{FFFD}\u{FFFD}"),
            other => panic!("expected Owned(String(..)): {other:?}"),
        }
    }

    /// The sibling of the test above with a genuinely #1194-malformed key
    /// rather than a mere decode failure: `materialize_lazy_keys`'s `Err`
    /// arm inside `fold_lazy_keys_stage`'s catch-all still raises, since
    /// #1642 only relaxed the decode-failure case, not this one. Same
    /// `keys | .[0]` shape reaching the same catch-all arm (see that
    /// test's own doc comment for why `.[0]` falls here rather than one of
    /// the dedicated fast-path arms).
    #[test]
    fn test_fold_lazy_keys_stage_catchall_raises_on_malformed_key_1194() {
        let json: &[u8] = b"{123: 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("keys | .[0]").unwrap(), cursor);
        assert!(result.is_error(), "{result:?}");
    }

    /// `each_lazy_keys_iterate_sink`'s sorted branch (#1599) decodes and
    /// sorts every key via `effective_keys` before iterating -- reached by
    /// `keys | .[]` (the demand-driven `Expr::Iterate` fan-out over a lazy
    /// keys result) over a document with an undecodable key. #1247 raised
    /// here; #1642 preserves the key instead.
    /// CLI-reachable: `sjq 'keys | .[]'`.
    #[test]
    fn test_each_lazy_keys_iterate_sink_sorted_preserves_decode_failure_1642() {
        let json: &[u8] = b"{\"\xff\xfe\": 1}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("keys | .[]").unwrap(), cursor);
        match result {
            GenericResult::ManyOwned(vs) => {
                assert_eq!(vs, vec![OwnedValue::String("\u{FFFD}\u{FFFD}".to_string())]);
            }
            other => panic!("expected ManyOwned([..]): {other:?}"),
        }
    }

    /// `eval_index_expr`'s "key outer, target inner" loop takes its
    /// `OneCursor`/`to_owned_cursor` conversion branch once at least one
    /// earlier key/target pair has already materialized into `owned` (`any_owned`)
    /// -- reached by `.[("z","a")]` where `"z"` is missing (materializes as
    /// `Owned(Null)`, setting `any_owned`) and `"a"` exists with an
    /// undecodable value (`OneCursor`, now hitting the `any_owned` branch's
    /// `to_owned_cursor` call). The already-materialized prefix (`[null]`)
    /// must survive as `Partial`, not vanish, matching #400/#494. This is
    /// CLI-reachable: `sjq '.[("z","a")]'`.
    #[test]
    fn test_eval_index_expr_any_owned_cursor_conversion_decode_failure_1247() {
        let json: &[u8] = b"{\"a\": \"\xff\xfe\"}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse(r#".[("z","a")]"#).unwrap(), cursor);
        match result {
            GenericResult::Partial(vs, Control::Error(e)) => {
                assert_eq!(vs, vec![OwnedValue::Null]);
                assert!(e.message.contains("invalid UTF-8"), "{e:?}");
            }
            other => panic!("expected Partial([null], Error(..)), got {other:?}"),
        }
    }

    /// `Builtin::ToEntries` raises on a key that isn't string-shaped at all,
    /// rather than dropping the entry (#1194).
    ///
    /// This asserted the drop when #1247's coverage pass wrote it, as a
    /// known gap deliberately left alone there. It is the older, distinct
    /// sibling of #1247's own `string_decode_error` check just above it:
    /// that one fires for a string-*shaped* span whose bytes won't decode,
    /// while `{123: 1, "b": 2}` has a key JSON's grammar never allowed. The
    /// lenient semi-index represents it structurally -- `uncons` hands back
    /// a field whose `.key` is `StandardJson::Number` -- so nothing but an
    /// explicit check can tell it from a well-formed object.
    ///
    /// Inverted rather than deleted: dropping the entry left `to_entries`
    /// one shorter than `length` counted, the disagreement #1385's own
    /// postmortem names as the failure mode to avoid.
    #[test]
    fn test_to_entries_raises_on_non_string_key_1194() {
        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("to_entries").unwrap(), cursor);
        let GenericResult::Error(err) = result else {
            panic!("expected an error, got {result:?}");
        };
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
        // The strict validator's own diagnosis, not a message invented here.
        assert!(
            err.message.contains("expected string key"),
            "message: {}",
            err.message
        );
    }

    /// #1194: an object whose children don't pair raises from `to_entries`
    /// too, not just one with a bad key.
    ///
    /// The two conditions reach different checks -- a bad key is caught
    /// per-field, an orphan only once the walk ends -- so a fix for one can
    /// silently miss the other.
    #[test]
    fn test_to_entries_raises_on_unpaired_member_1194() {
        let json: &[u8] = b"{invalid}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse("to_entries").unwrap(), cursor);
        let GenericResult::Error(err) = result else {
            panic!("expected an error, got {result:?}");
        };
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #1194 must not hijack #1247's decode failures: an *undecodable* key
    /// has no stringified name either, but it is a different fault with a
    /// different answer -- sometimes deliberately preserved verbatim
    /// (#1385's "a key that will not decode is never a duplicate").
    ///
    /// Regression guard for a real bug in this fix's first cut, which tested
    /// only `key_string().is_none()` and so reported an invalid escape as
    /// `expected string key` -- the wrong cause, and at the wrong severity.
    #[test]
    fn test_malformed_member_check_leaves_decode_failures_alone_1194() {
        let json: &[u8] = br#"{"a\q":1,"b":2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let fields = cursor.value().as_object().expect("an object");
        assert!(
            malformed_object_member(&fields).is_none(),
            "an undecodable key is #1247's fault, not #1194's"
        );
    }

    /// #1194: both materializing conversions raise on a non-string key.
    ///
    /// Driven directly rather than through a filter because these two are
    /// copies of each other reached from different domains -- `to_owned`
    /// from a value, `to_owned_cursor` from a cursor -- and which one a
    /// given CLI invocation lands on depends on the filter's shape, so a
    /// filter-level test cannot pin *both* arms on purpose. Both were
    /// uncovered when this check was added; a fix that moved only one would
    /// leave the value and cursor domains disagreeing about whether the same
    /// document is valid.
    ///
    /// The orphan half (`{invalid}`) is exercised by the CLI tests; this is
    /// the bad-key half, which reaches the other check entirely.
    #[test]
    fn test_both_owned_conversions_raise_on_non_string_key_1194() {
        let json: &[u8] = b"{123: 1, \"b\": 2}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let value_err = to_owned(&cursor.value()).expect_err("the value domain must raise");
        let cursor_err = to_owned_cursor(&cursor).expect_err("the cursor domain must raise");

        for err in [&value_err, &cursor_err] {
            assert!(
                err.message.contains("expected string key"),
                "message: {}",
                err.message
            );
        }
        // One document, one cause, however it is reached -- the reason
        // `malformed_member_error` is a method rather than a literal per
        // call site.
        assert_eq!(value_err.message, cursor_err.message);
    }

    /// #1677: #1643's missing/doubled `,`/`:` check used to live only in
    /// the CLI's `print_json`, so a filter that never re-serializes the
    /// malformed container whole read straight through it. Each of these
    /// builtins now raises the same way `.` already did.
    #[test]
    fn test_nonreserializing_builtins_raise_on_missing_delimiter_1677() {
        let json: &[u8] = br#"{"a" 1, "b": 2}"#;
        for filter in ["keys", "keys_unsorted", "length", "to_entries"] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let result = eval_with_cursor(&crate::jq::parse(filter).unwrap(), cursor);
            // `keys`/`keys_unsorted` stay lazy (`LazyKeys`) until something
            // forces them -- `materialize_lazy` is the same forcing step
            // every real consumer (printing, `collect_owned`) applies.
            let materialized = result.materialize_lazy();
            let GenericResult::Error(err) = &materialized else {
                panic!("{filter}: expected an error, got {materialized:?}");
            };
            assert!(
                err.message.contains("Invalid JSON text"),
                "{filter}: message: {}",
                err.message
            );
        }
    }

    /// #1677: same check for array elements, via `.[]` (a bare walk) and
    /// `add` (falls through to the `to_owned_cursor` bridge). Both go
    /// through a different code path than the object cases above.
    #[test]
    fn test_array_filters_raise_on_missing_delimiter_1677() {
        let json: &[u8] = b"[1 2, 3]";
        for filter in [".[]", "add"] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            let result = eval_with_cursor(&crate::jq::parse(filter).unwrap(), cursor);
            let GenericResult::Error(err) = result else {
                panic!("{filter}: expected an error, got {result:?}");
            };
            assert!(
                err.message.contains("Invalid JSON text"),
                "{filter}: message: {}",
                err.message
            );
        }
    }

    /// #1677: a targeted field lookup (`.a`) validates the delimiters of
    /// the field it actually resolves to, even for a top-level scalar that
    /// never reaches any printer's object/array arm -- `find_cursor` itself
    /// has to check this, since nothing downstream will.
    #[test]
    fn test_field_lookup_raises_on_missing_delimiter_1677() {
        let json: &[u8] = br#"{"a" 1}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let result = eval_with_cursor(&crate::jq::parse(".a").unwrap(), cursor);
        let GenericResult::Error(err) = result else {
            panic!("expected an error, got {result:?}");
        };
        assert!(
            err.message.contains("Invalid JSON text"),
            "message: {}",
            err.message
        );
    }

    /// #1677: both materializing conversions raise on a missing delimiter,
    /// mirroring `test_both_owned_conversions_raise_on_non_string_key_1194`'s
    /// same-document, same-cause pattern for the delimiter class instead.
    #[test]
    fn test_both_owned_conversions_raise_on_missing_delimiter_1677() {
        let json: &[u8] = br#"{"a" 1, "b": 2}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let value_err = to_owned(&cursor.value()).expect_err("the value domain must raise");
        let cursor_err = to_owned_cursor(&cursor).expect_err("the cursor domain must raise");

        for err in [&value_err, &cursor_err] {
            assert!(
                err.message.contains("Invalid JSON text"),
                "message: {}",
                err.message
            );
        }
        assert_eq!(value_err.message, cursor_err.message);
    }

    /// #2262: `to_owned_at_depth` (the cursor-less sibling `GenericResult::
    /// One`/`Many` reach whenever a value materializes without a live
    /// cursor) never checked for a trailing stray `,` after a real last
    /// child at all, unlike `to_owned_cursor_at_depth` (#2243). Both
    /// conversions now agree, mirroring
    /// `test_both_owned_conversions_raise_on_missing_delimiter_1677`'s same
    /// same-document, same-cause pattern for this delimiter class instead.
    #[test]
    fn test_both_owned_conversions_raise_on_trailing_comma_2262() {
        for json in [b"[1,]".as_slice(), br#"{"a":1,}"#.as_slice()] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);

            let value_err = match to_owned(&cursor.value()) {
                Err(e) => e,
                Ok(v) => panic!(
                    "{json:?}: a trailing comma after a real last child is not JSON, got {v:?}"
                ),
            };
            let cursor_err = match to_owned_cursor(&cursor) {
                Err(e) => e,
                Ok(v) => panic!("{json:?}: the cursor domain must agree, got {v:?}"),
            };

            for err in [&value_err, &cursor_err] {
                assert!(
                    err.message.contains("Invalid JSON text"),
                    "{json:?}: message: {}",
                    err.message
                );
            }
            assert_eq!(
                value_err.message, cursor_err.message,
                "{json:?}: one document, one cause"
            );
        }
    }

    /// #2262: unlike the trailing-comma case above, a stray `,` with *zero*
    /// real children (`[,]`, `{,}`) is a known, deliberately unclosed gap
    /// for `to_owned`/`to_owned_at_depth` specifically -- #2211's
    /// `container_gap_ok` needs a cursor to the *container itself* to find
    /// its opening bracket, and this cursor-less conversion is only ever
    /// given a bare `value: &V`, never a cursor for the container (only
    /// per-child cursors, once a child actually exists to hold one). Once
    /// the child walk is exhausted there is nothing left to check against.
    /// `to_owned_cursor` -- which *is* given the container's own cursor --
    /// already closed this via #2211 and continues to reject both shapes,
    /// so the two conversions deliberately disagree here. This pins that
    /// disagreement as the documented, still-open half of #2262 rather
    /// than leaving it silently uncovered.
    #[test]
    fn test_to_owned_stray_comma_in_empty_container_remains_a_known_gap_2262() {
        for json in [b"[,]".as_slice(), b"{,}".as_slice()] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);

            if let Err(e) = to_owned(&cursor.value()) {
                panic!("{json:?}: known gap -- silently accepted, got error {e:?}");
            }
            if let Ok(v) = to_owned_cursor(&cursor) {
                panic!("{json:?}: the cursor domain already rejects this, got {v:?}");
            }
        }
    }

    /// #2262: well-formed arrays/objects (including a multi-element array
    /// and a multi-field object, so both the "at least one real child, no
    /// trailing gap" branch and the untouched empty-container branch are
    /// each pinned) are unaffected by the new trailing-comma check above.
    #[test]
    fn test_to_owned_wellformed_containers_unaffected_by_trailing_comma_check_2262() {
        for (json, expected) in [
            (b"[]".as_slice(), OwnedValue::Array(vec![])),
            (b"{}".as_slice(), OwnedValue::Object(IndexMap::new())),
            (
                b"[1,2,3]".as_slice(),
                OwnedValue::Array(vec![
                    OwnedValue::from_number_literal("1"),
                    OwnedValue::from_number_literal("2"),
                    OwnedValue::from_number_literal("3"),
                ]),
            ),
            (
                br#"{"a":1,"b":2}"#.as_slice(),
                OwnedValue::Object(IndexMap::from([
                    ("a".to_string(), OwnedValue::from_number_literal("1")),
                    ("b".to_string(), OwnedValue::from_number_literal("2")),
                ])),
            ),
        ] {
            let index = JsonIndex::build(json);
            let cursor = index.root(json);
            assert_eq!(
                to_owned(&cursor.value()).unwrap(),
                expected,
                "{json:?}: to_owned"
            );
            assert_eq!(
                to_owned_cursor(&cursor).unwrap(),
                expected,
                "{json:?}: to_owned_cursor must agree"
            );
        }
    }

    /// #1687: the sort family's array-valued results (`sort`, `sort_by`,
    /// `unique`, `unique_by`, `reverse`) answer a `LazySeq` over the
    /// reordered element cursors rather than a materialized
    /// `OwnedValue::Array`. That shape is what carries a duplicate mapping
    /// key through, so it is worth pinning directly.
    ///
    /// **The duplicate keys themselves are not observable on a JSON index**,
    /// and deliberately so: only `YamlCursor` overrides
    /// `DocumentCursor::supports_sequence_streaming`, so
    /// `sequence_streamable_cursors` returns `None` for JSON and
    /// `stream_json`'s `LazySeq` arm falls back to `lazy_elem_to_owned` --
    /// an `IndexMap`. That is pre-existing and shared with `map(.)`, which
    /// has answered a `LazySeq` since #724/#725; it is why plain jq mode
    /// still collapses here (correct, per #1385) and why even
    /// `--preserve-input` does. The preservation this arm buys is pinned on
    /// the YAML path instead, by
    /// `test_sort_family_preserves_duplicate_keys_in_moved_elements_1687`
    /// in `tests/yq_cli_tests.rs`.
    #[test]
    fn test_sort_family_streams_reordered_cursors_1687() {
        let json = br#"[{"b":2,"b":9},{"a":1,"a":8}]"#;
        let index = JsonIndex::build(json);

        for (filter, want) in [
            // Ordering is the observable half here: jq's total order puts an
            // object keyed `a` before one keyed `b`.
            ("sort_by(.a)", r#"[{"b":9},{"a":8}]"#),
            ("reverse", r#"[{"a":8},{"b":9}]"#),
            ("unique_by(.a)", r#"[{"b":9},{"a":8}]"#),
        ] {
            let expr = crate::jq::parse(filter).unwrap();
            let result = eval_with_cursor(&expr, index.root(json));
            assert!(
                matches!(result, GenericResult::LazySeq(_)),
                "{filter} should answer a LazySeq, not a materialized array"
            );
            let mut out = String::new();
            result
                .stream_json(
                    &mut out,
                    IndentSpec::COMPACT,
                    false,
                    // `eval_with_cursor` above is jq semantics (see its own
                    // doc comment), which is what this test's expected
                    // `want` strings assume: duplicate object keys collapse
                    // (#1576's `JsonConvention::JqCompat` is what carries
                    // `EvalSemantics::COLLAPSE_DUPLICATE_KEYS` into this
                    // cursor-streaming writer -- see that enum's own doc
                    // comment).
                    JsonConvention::JqCompat,
                    |_| Ok(()),
                )
                .unwrap();
            assert_eq!(out, want, "{filter}");
        }
    }

    /// #1687: `min`/`max`/`min_by`/`max_by` select one of the input's own
    /// elements, so the winner is returned as a bare `OneCursor` -- the same
    /// shape `first(.[])` uses (#607).
    #[test]
    fn test_min_max_family_returns_the_winning_cursor_1687() {
        let json = br#"[{"a":2,"a":9},{"a":1,"a":8}]"#;
        let index = JsonIndex::build(json);

        for (filter, want) in [
            ("min_by(.a)", r#"{"a":1,"a":8}"#),
            ("max_by(.a)", r#"{"a":2,"a":9}"#),
        ] {
            let expr = crate::jq::parse(filter).unwrap();
            let result = eval_with_cursor(&expr, index.root(json));
            assert!(result.is_single_cursor(), "{filter}");
            let mut out = String::new();
            result
                .stream_json(
                    &mut out,
                    IndentSpec::COMPACT,
                    false,
                    JsonConvention::Preserve,
                    |_| Ok(()),
                )
                .unwrap();
            assert_eq!(out, want, "{filter}");
        }
    }

    /// #1687: `limit`/`nth` had no unit-level coverage at all -- #1607/#1686
    /// pinned them only through the CLI. A single-valued `n` must keep the
    /// batch cursor-backed (`ManyCursor`), which is what preserves a
    /// duplicate key *inside* a captured element, and the generator-`n` path
    /// added here must not lose that.
    #[test]
    fn test_limit_keeps_captured_elements_cursor_backed_1687() {
        let json = br#"[{"a":1,"a":2},{"b":3,"b":4},{"c":5}]"#;
        let index = JsonIndex::build(json);

        let expr = crate::jq::parse("limit(2; .[])").unwrap();
        let result = eval_with_cursor(&expr, index.root(json));
        assert!(
            matches!(result, GenericResult::ManyCursor(_)),
            "a single-valued n must not flatten the batch"
        );
        let mut out = String::new();
        result
            .stream_json(
                &mut out,
                IndentSpec::COMPACT,
                false,
                JsonConvention::Preserve,
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(out, r#"{"a":1,"a":2}{"b":3,"b":4}"#);

        // Generator `n`: outer loop over n, inner over `.[]`, so n=1 keeps
        // one element and n=2 keeps two.
        let expr = crate::jq::parse("[limit((1,2); .[])] | length").unwrap();
        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.into_owned().unwrap().unwrap(), OwnedValue::Int(3));
    }

    /// #1371: `eval_each_generic`'s own `Expr::DefCall` arm (mirroring
    /// `eval.rs`'s eager `eval_def_call`) has to turn a `bind_def_call`
    /// failure into `Flow::Escaped(Control::Error(_))` rather than, say,
    /// unwrapping or silently stopping -- a consumer stacked on top of the
    /// generic/lazy sink (`limit`, `first`, `[...]`) must see the same
    /// recursion-depth error the eager path raises, not a truncated or empty
    /// result.
    ///
    /// Reaches `MAX_EVAL_FRAMES` via the ambient-depth guard directly, the
    /// same white-box shortcut `eval.rs`'s own
    /// `test_ambient_frame_depth_composes_with_defcall_guard_1371` uses,
    /// rather than actually recursing 40,000 levels deep. `eval_each_generic`
    /// is reached (rather than `eval_single`'s "fall back to the full
    /// evaluator" wildcard arm) by driving the constructed `Expr::DefCall`
    /// straight through [`eval_each_with_cursor_using`], which calls
    /// `eval_each_generic` as its own entry point.
    ///
    /// `#[cfg(feature = "std")]`: reaches `MAX_EVAL_FRAMES` via the
    /// ambient-depth guard (`eval.rs`'s `ambient_frame_depth` module),
    /// which is a no-op under `no_std` by design -- there is no no_std
    /// variant of this mechanism to test.
    #[test]
    #[cfg(feature = "std")]
    fn eval_each_generic_reports_defcall_binding_failure_1371() {
        let Expr::FuncDef {
            name,
            params,
            body,
            then,
            ..
        } = parse("def f: f; f").unwrap()
        else {
            panic!("expected a top-level FuncDef");
        };

        let _guard = enter_def_call_frame(crate::jq::eval::MAX_EVAL_FRAMES);
        let cache = FuncDefBound::default();
        let defcall = bind_def(&name, &params, &body, &then, &cache);
        assert!(
            matches!(&*defcall, Expr::DefCall { frames, .. } if *frames == crate::jq::eval::MAX_EVAL_FRAMES),
            "expected bind_def to seed frames from the ambient depth"
        );

        let json = b"null";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);

        let mut on_value = |_item: GenericResult<_>| true;
        let control =
            eval_each_with_cursor_using::<JqSemantics, _>(&defcall, cursor, &mut on_value);

        match control {
            Some(Control::Error(err)) => assert!(
                err.message.contains("exceeded maximum recursion depth"),
                "message: {}",
                err.message
            ),
            other => panic!("expected Control::Error, got {other:?}"),
        }
    }

    /// Drive `expr` through [`eval_each_with_cursor_using`] against `json`,
    /// returning how many outputs reached the sink and the terminating
    /// control, if any. The demand-aware `keys_unsorted[]` arm
    /// ([`each_lazy_keys_iterate_sink`]) is reachable only from this entry
    /// point -- the CLI's default M2 route still uses the eager
    /// `eval_with_cursor` (#1653) -- so these tests call it directly rather
    /// than through a CLI round trip that would exercise the other evaluator.
    fn drive_each(json: &[u8], expr: &str) -> (usize, Option<Control>) {
        let expr = parse(expr).unwrap();
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let mut count = 0usize;
        let mut on_value = |_item: GenericResult<_>| {
            count += 1;
            true
        };
        let control = eval_each_with_cursor_using::<JqSemantics, _>(&expr, cursor, &mut on_value);
        (count, control)
    }

    /// #1653: the terminal half of the #1194 check -- an unpaired tail or a
    /// #1677 delimiter fault, neither of which has a per-key signal -- is
    /// asked once the `keys_unsorted[]` walk reaches exhaustion, so a
    /// consumer that walked the whole object gets the same verdict
    /// `fold_lazy_keys_stage`'s eager arms already gave via
    /// `walk_distinct_keys_checked`.
    ///
    /// The outputs produced *before* the fault are asserted too: the walk
    /// finds it only on arrival, so the prefix has already reached the sink.
    /// That is the same shape #1770 accepted for `limit(2;
    /// keys_unsorted[])`, and it is what the eager route (which returns one
    /// batched `Error` and emits nothing) does not do.
    #[test]
    fn each_lazy_keys_iterate_sink_raises_on_terminal_fault_at_exhaustion_1653() {
        for json in [
            &b"{\"a\":1,\"b\"}"[..],
            &b"{\"a\":1,\"a\":2,\"b\"}"[..],
            &b"{\"a\":1, invalid}"[..],
        ] {
            let (count, control) = drive_each(json, "keys_unsorted[]");
            match control {
                Some(Control::Error(err)) => assert!(
                    err.message.contains("Invalid JSON text"),
                    "json {json:?}: message: {}",
                    err.message
                ),
                other => panic!("json {json:?}: expected Control::Error, got {other:?}"),
            }
            assert_eq!(count, 1, "json {json:?}: the prefix before the fault");
        }
    }

    /// #1770's divergence, still scoped to early exit after #1653's terminal
    /// check: a truncating consumer stops before the walk reaches the tail,
    /// so it returns `Flow::Stopped` and is never charged for the
    /// whole-object probe it exists to avoid. The same three documents that
    /// raise for a bare `keys_unsorted[]` above answer at no error here.
    #[test]
    fn each_lazy_keys_iterate_sink_early_exit_still_skips_the_terminal_fault_1770() {
        for json in [
            &b"{\"a\":1,\"b\"}"[..],
            &b"{\"a\":1,\"a\":2,\"b\"}"[..],
            &b"{\"a\":1, invalid}"[..],
        ] {
            let (count, control) = drive_each(json, "first(keys_unsorted[])");
            assert!(control.is_none(), "json {json:?}: control: {control:?}");
            assert_eq!(count, 1, "json {json:?}");
        }
    }

    /// The check costs a well-formed object nothing but a verdict: every key
    /// still reaches the sink and no control is raised. Guards against the
    /// `is_malformed()` call being asked before exhaustion, where
    /// `ended_unpaired` is not yet meaningful.
    #[test]
    fn each_lazy_keys_iterate_sink_well_formed_object_is_unaffected_1653() {
        let (count, control) = drive_each(b"{\"a\":1,\"b\":2,\"c\":3}", "keys_unsorted[]");
        assert!(control.is_none(), "control: {control:?}");
        assert_eq!(count, 3);
    }

    /// #2261: a trailing stray comma after a real last key (`{"a":1,}`) is
    /// the same "only checked on exhaustion" shape #1653 already pins above
    /// for `ended_unpaired`/`delimiter_fault`, extended to
    /// `DistinctKeyCursors::trailing_gap_ok`. Every real key still reaches
    /// the sink first (the same "prefix before the fault" contract).
    #[test]
    fn each_lazy_keys_iterate_sink_raises_on_trailing_comma_at_exhaustion_2261() {
        let (count, control) = drive_each(br#"{"a":1,}"#, "keys_unsorted[]");
        match control {
            Some(Control::Error(err)) => assert!(
                err.message.contains("Invalid JSON text"),
                "message: {}",
                err.message
            ),
            other => panic!("expected Control::Error, got {other:?}"),
        }
        assert_eq!(count, 1, "the one real key before the fault");
    }

    /// #2261: the identical shape, but with a duplicate key ahead of the
    /// trailing comma (`{"a":1,"b":2,"a":3,}`) -- pins that
    /// `last_key_cursor` survives a confirmed collapse and still points at
    /// the object's *textually* last field, not whichever key the
    /// collapsed walk happened to yield last.
    #[test]
    fn each_lazy_keys_iterate_sink_raises_on_trailing_comma_with_duplicate_key_2261() {
        let (count, control) = drive_each(br#"{"a":1,"b":2,"a":3,}"#, "keys_unsorted[]");
        match control {
            Some(Control::Error(err)) => assert!(
                err.message.contains("Invalid JSON text"),
                "message: {}",
                err.message
            ),
            other => panic!("expected Control::Error, got {other:?}"),
        }
        assert_eq!(count, 2, "both distinct keys before the fault");
    }

    /// #2261's own early-exit exemption, mirroring #1770's above: a
    /// truncating consumer never reaches the tail, so it never sees the
    /// trailing comma either.
    #[test]
    fn each_lazy_keys_iterate_sink_early_exit_still_skips_the_trailing_comma_2261() {
        let (count, control) = drive_each(br#"{"a":1,}"#, "first(keys_unsorted[])");
        assert!(control.is_none(), "control: {control:?}");
        assert_eq!(count, 1);
    }
}
