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
    DocumentCursor, DocumentElements, DocumentFields, DocumentValue, IndentSpec,
};
use super::eval::{
    compare_values, eval as full_eval, format_owned, index_one_owned as index_owned_by_key,
    needs_path_context, numeric_display_string, numeric_key_to_index, owned_bound_to_i64,
    slice_owned_value, tonumber_from_str, Control, EvalError, EvalSemantics, EvalTag, JqSemantics,
    QueryResult, YqSemantics,
};
use super::expr::{Builtin, CompareOp, Expr, FormatType, Literal};
use super::slice::{slice_str, SliceBounds};
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
pub fn to_owned_cursor<C: DocumentCursor>(cursor: &C) -> OwnedValue {
    let value = cursor.value();
    if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut f = fields;
        while let Some((field, rest)) = f.uncons() {
            if let Some(key) = field.key_str() {
                map.insert(key.into_owned(), to_owned_cursor(&field.value_cursor));
            }
            f = rest;
        }
        OwnedValue::Object(map)
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut elems = elements;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            items.push(to_owned_cursor(&elem_cursor));
            elems = rest;
        }
        OwnedValue::Array(items)
    } else {
        cursor
            .explicit_tag()
            .and_then(|tag| tagged_scalar_to_owned(tag, &value))
            .unwrap_or_else(|| to_owned(&value))
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
    Some(match resolved {
        crate::yaml::ResolvedScalar::Null => OwnedValue::Null,
        crate::yaml::ResolvedScalar::Bool(b) => OwnedValue::Bool(b),
        crate::yaml::ResolvedScalar::Int(n) => OwnedValue::Int(n),
        crate::yaml::ResolvedScalar::Float(f) => OwnedValue::Float(f),
        crate::yaml::ResolvedScalar::Str => OwnedValue::String(text.into_owned()),
    })
}

/// Materializes `value`, preferring the cursor-aware, tag-resolving
/// [`to_owned_cursor`] when a cursor is available (issue #747) and falling
/// back to the cursor-less [`to_owned`] otherwise — e.g. a computed value
/// with no navigated position (see [`DocumentCursor::line`]'s "not
/// available" contract, which the same computed-vs-navigated distinction
/// applies to). Since `cursor.value()` and a separately-threaded `value`
/// always describe the same node on every call site in this module, `value`
/// itself is only needed for the `None` fallback.
fn to_owned_with_cursor<V: DocumentValue>(value: &V, cursor: Option<V::Cursor>) -> OwnedValue {
    match cursor {
        Some(c) => to_owned_cursor(&c),
        None => to_owned(value),
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

/// A shape-parallel tree of per-node presentation metadata.
///
/// Trailing same-line comments (issue #710) and YAML style (issue #739;
/// `""` for block/plain, `"flow"`/`"double"`/`"single"`/`"literal"`/
/// `"folded"` per [`DocumentCursor::style`]) — built alongside an
/// `OwnedValue` by [`to_owned_with_comments`].
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
    /// A scalar (or any node with no comment-bearing children): this node's
    /// own trailing comment, if any, and its style.
    Leaf(Option<String>, &'static str),
    /// An array: this node's own trailing comment (e.g. `a: [1,2] # c`,
    /// which trails the whole array, not an element) and style, plus one
    /// subtree per element in order.
    Array(Option<String>, &'static str, Vec<Self>),
    /// An object: this node's own trailing comment and style, one subtree
    /// per field (keyed the same as the parallel `OwnedValue::Object`),
    /// plus one key-scoped comment per field for a comment trailing the
    /// *key's* own line when its value is deferred to a following line
    /// (issue #765, e.g. `a: # comment\n  b: 1`) - distinct from the
    /// field's value subtree's own comment, which `.a | line_comment` etc.
    /// read instead. The `bool` alongside each comment is whether the
    /// deferred value materialized as nothing at all (a sibling key
    /// follows, or EOF) - see [`Self::key_comment_if_value_absent`].
    Object(
        Option<String>,
        &'static str,
        IndexMap<String, Self>,
        IndexMap<String, (String, bool)>,
    ),
}

impl CommentTree {
    /// The empty tree: no comment or style at this node, and (for
    /// containers) no children — used where a caller has no cursor at all
    /// (metadata-less by construction, e.g. a computed value).
    pub const fn empty() -> Self {
        Self::Leaf(None, "")
    }

    /// This node's own trailing comment, if any.
    pub fn own(&self) -> Option<&str> {
        match self {
            Self::Leaf(c, _) | Self::Array(c, _, _) | Self::Object(c, _, _, _) => c.as_deref(),
        }
    }

    /// This node's own YAML style (`""`, `"flow"`, `"double"`, `"single"`,
    /// `"literal"`, or `"folded"` — see [`DocumentCursor::style`]).
    pub fn style(&self) -> &'static str {
        match self {
            Self::Leaf(_, s) | Self::Array(_, s, _) | Self::Object(_, s, _, _) => s,
        }
    }

    /// The subtree for array index `i`, or the empty tree if this isn't an
    /// `Array` or the index is out of range.
    pub fn at_index(&self, i: usize) -> &Self {
        match self {
            Self::Array(_, _, items) => items.get(i).unwrap_or(&EMPTY_COMMENT_TREE),
            _ => &EMPTY_COMMENT_TREE,
        }
    }

    /// The subtree for object key `key`, or the empty tree if this isn't an
    /// `Object` or has no such key.
    pub fn field(&self, key: &str) -> &Self {
        match self {
            Self::Object(_, _, fields, _) => fields.get(key).unwrap_or(&EMPTY_COMMENT_TREE),
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
            Self::Object(_, _, _, key_comments) => key_comments.get(key).map(|(c, _)| c.as_str()),
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
            Self::Object(_, _, _, key_comments) => key_comments
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
static EMPTY_COMMENT_TREE: CommentTree = CommentTree::Leaf(None, "");

/// Convert a `DocumentValue` to an `OwnedValue` alongside a parallel [`CommentTree`].
///
/// Uses a live cursor to read each node's trailing comment (issue #710).
/// See [`to_owned`] for the value-only conversion this mirrors and
/// delegates scalar handling to.
pub fn to_owned_with_comments<V: DocumentValue>(
    value: &V,
    cursor: Option<&V::Cursor>,
) -> (OwnedValue, CommentTree) {
    // The raw (`#`-prefixed) form, not the stripped `line_comment` builtin
    // getter: the write path re-emits this verbatim after one space.
    let own_comment = cursor.and_then(DocumentCursor::line_comment_raw);
    let own_style = cursor.map_or("", DocumentCursor::style);
    if let Some(fields) = value.as_object() {
        let mut map = IndexMap::new();
        let mut comment_map = IndexMap::new();
        let mut key_comment_map = IndexMap::new();
        let mut f = fields;
        while let Some((field, rest)) = f.uncons() {
            if let Some(key) = field.key_str() {
                let key = key.into_owned();
                let (v, c) = to_owned_with_comments(&field.value, Some(&field.value_cursor));
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
                    let value_absent = field.value.is_null()
                        && field.value.as_str().map_or(true, |s| s.is_empty());
                    key_comment_map.insert(key, (kc, value_absent));
                }
            }
            f = rest;
        }
        (
            OwnedValue::Object(map),
            CommentTree::Object(own_comment, own_style, comment_map, key_comment_map),
        )
    } else if let Some(elements) = value.as_array() {
        let mut items = Vec::new();
        let mut comment_items = Vec::new();
        // Iterate by cursor (`uncons_cursor`, not `uncons`, which yields
        // values only) so each element's own comment is reachable.
        let mut elems = elements;
        while let Some((elem_cursor, rest)) = elems.uncons_cursor() {
            let elem_value = elem_cursor.value();
            let (v, c) = to_owned_with_comments(&elem_value, Some(&elem_cursor));
            items.push(v);
            comment_items.push(c);
            elems = rest;
        }
        (
            OwnedValue::Array(items),
            CommentTree::Array(own_comment, own_style, comment_items),
        )
    } else {
        (to_owned(value), CommentTree::Leaf(own_comment, own_style))
    }
}

/// Materialize a key/slice-bound candidate just enough to classify it.
///
/// Mirrors `eval::to_owned_key_shape` (#626/#670): `index_one_generic`'s
/// Array/Object rejection and `slice::bound`'s non-numeric rejection never
/// inspect a candidate's *contents*, only its shape, so a full recursive
/// `to_owned` of a large navigated container is pure waste when it can only
/// ever be rejected on type (#669).
fn to_owned_key_shape<V: DocumentValue>(value: &V) -> OwnedValue {
    if value.is_array() {
        OwnedValue::Array(Vec::new())
    } else if value.is_object() {
        OwnedValue::Object(IndexMap::new())
    } else {
        to_owned(value)
    }
}

/// Cursor-carrying sibling of [`to_owned_key_shape`] (#903 review): a
/// computed-index key or slice bound reached via `OneCursor`/`ManyCursor`
/// still needs its scalar resolved through `to_owned_cursor` rather than the
/// bare `to_owned`, or an explicit tag on the key/bound expression itself
/// (`.a[.k]` where `.k` is `!!str 1`) is silently ignored.
fn to_owned_key_shape_cursor<C: DocumentCursor>(cursor: &C) -> OwnedValue {
    let value = cursor.value();
    if value.is_array() {
        OwnedValue::Array(Vec::new())
    } else if value.is_object() {
        OwnedValue::Object(IndexMap::new())
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
fn materialize_lazy_keys<V: DocumentValue>(fields: &V::Fields, sorted: bool) -> OwnedValue {
    let mut keys = fields.keys();
    if sorted {
        keys.sort();
    }
    OwnedValue::Array(keys.into_iter().map(OwnedValue::String).collect())
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
    Keys(V::Fields),
    /// Array `keys_unsorted | map(f)` (#724) — synthetic `[0, 1, ..., len-1]`,
    /// no cursor to point at.
    IndexRange { next: usize, len: usize },
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
        }
    }
}

impl<V: DocumentValue> LazySource<V> {
    /// Pull one element forward, storing "the rest" back into `self`. Once a
    /// variant's underlying cons-list is empty (or `next == len`), every
    /// subsequent call returns `None` forever.
    fn advance(&mut self) -> Option<LazyElem<V>> {
        match self {
            Self::Elements(elements) => {
                let (cursor, rest) = elements.uncons_cursor()?;
                *elements = rest;
                Some(LazyElem::Cursor(cursor))
            }
            Self::Values(fields) => {
                let (field, rest) = fields.uncons()?;
                *fields = rest;
                Some(LazyElem::Cursor(field.value_cursor))
            }
            Self::Keys(fields) => {
                let (field, rest) = fields.uncons()?;
                *fields = rest;
                Some(LazyElem::Cursor(field.key_cursor))
            }
            Self::IndexRange { next, len } => {
                if *next >= *len {
                    return None;
                }
                let i = *next;
                *next += 1;
                Some(LazyElem::Owned(OwnedValue::Int(i as i64)))
            }
        }
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

    /// Push one more `map(f)` stage onto this chain. `f` is cloned once per
    /// *stage* (not per element) into a fresh `Rc` — `Builtin::Map` only ever
    /// hands us a borrowed `&Expr` (from `unwrap_paren` in the `Pipe` fold, or
    /// `&Builtin` in `eval_builtin`), so there is nothing to move out of.
    fn push_map(mut self, f: &Expr, tag: EvalTag) -> Self {
        Rc::make_mut(&mut self.instructions).push(Instruction {
            f: Rc::new(f.clone()),
            tag,
        });
        self
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
            let mut next_items = Vec::with_capacity(items.len());
            for item in items {
                next_items.extend(into_lazy_items(Self::eval_one(instr, item))?);
            }
            items = next_items;
        }
        Ok(items)
    }

    /// Pull every remaining element to completion, discarding the whole
    /// in-progress array on the first error/break (mirrors `map_over`'s
    /// atomicity — real jq's array construction is all-or-nothing:
    /// `[1,2,"x"]|map(.+1)` prints nothing to stdout, only the stderr
    /// diagnostic).
    pub fn materialize_atomic(self) -> Result<OwnedValue, Control> {
        let mut out = Vec::new();
        for item in self {
            match item {
                Ok(LazyElem::Cursor(c)) => out.push(to_owned_cursor(&c)),
                Ok(LazyElem::Owned(o)) => out.push(o),
                Err(control) => return Err(control),
            }
        }
        Ok(OwnedValue::Array(out))
    }
}

impl<V: DocumentValue> Iterator for LazySeq<V> {
    type Item = Result<LazyElem<V>, Control>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.pending.pop() {
                return Some(Ok(item));
            }
            let elem = self.source.advance()?;
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
        GenericResult::One(v) => Ok(vec![LazyElem::Owned(to_owned(&v))]),
        // Stays lazy: a `map(.foo)`-style navigational sub-expr keeps
        // composing without forcing materialization.
        GenericResult::OneCursor(c) => Ok(vec![LazyElem::Cursor(c)]),
        GenericResult::Many(vs) => Ok(vs.iter().map(|v| LazyElem::Owned(to_owned(v))).collect()),
        GenericResult::ManyCursor(cs) => Ok(cs.into_iter().map(LazyElem::Cursor).collect()),
        GenericResult::LazyKeys { fields, sorted } => Ok(vec![LazyElem::Owned(
            materialize_lazy_keys::<V>(&fields, sorted),
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

/// Apply a format to an owned value and wrap it as a `GenericResult`.
fn format_result<V: DocumentValue>(
    format_type: &FormatType,
    owned: &OwnedValue,
    optional: bool,
) -> GenericResult<V> {
    match format_owned(format_type, owned, optional) {
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
    // render NaN/Infinity as `"inf"`/`".nan"`/etc. directly from an
    // `OwnedValue::Float` or `NumberLiteral`, with no dependence on having
    // passed through a JSON round-trip first.
    if let Expr::Format(format_type) = expr {
        return format_result(format_type, &owned, optional);
    }

    let json_str = owned.to_json_for_reindex();
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
        QueryResult::Halt(code) => GenericResult::Halt(code),
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
            Control::Halt(code) => GenericResult::Halt(code),
        }
    } else {
        GenericResult::Partial(prefix, control)
    }
}

/// Normalize a `Vec<OwnedValue>` accumulator into the smallest `GenericResult`
/// shape that represents it. Mirrors [`super::eval::owned_vec_to_result`].
fn owned_vec_to_generic_result<V: DocumentValue>(mut vs: Vec<OwnedValue>) -> GenericResult<V> {
    match vs.len() {
        0 => GenericResult::None,
        1 => GenericResult::Owned(vs.pop().unwrap()),
        _ => GenericResult::ManyOwned(vs),
    }
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
        Some(Control::Error(_)) if optional => owned_vec_to_generic_result(outputs),
        Some(control) => partial_generic(outputs, control),
    }
}

/// Append every output of a `GenericResult` stream to `out`, returning any
/// terminating `Control` instead of collapsing to the first output the way
/// the old `Expr::Compare` arm did. Mirrors [`super::eval::push_owned_values`]
/// for the generic evaluator's cursor-aware result type — used to fork
/// `Expr::Compare`'s operands into every output instead of only the first
/// (#768), the same way [`push_generic_truthiness`] already forks `select`'s
/// condition.
fn push_generic_owned_values<V: DocumentValue>(
    result: GenericResult<V>,
    out: &mut Vec<OwnedValue>,
) -> Option<Control> {
    match result.materialize_lazy() {
        GenericResult::One(v) => out.push(to_owned(&v)),
        GenericResult::OneCursor(c) => out.push(to_owned_cursor(&c)),
        GenericResult::Many(vs) => out.extend(vs.iter().map(to_owned)),
        GenericResult::ManyCursor(cs) => out.extend(cs.iter().map(to_owned_cursor)),
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
        GenericResult::OneCursor(c) => out.push(to_owned_cursor(&c).is_truthy()),
        GenericResult::Many(vs) => out.extend(vs.iter().map(|v| to_owned(v).is_truthy())),
        GenericResult::ManyCursor(cs) => {
            out.extend(cs.iter().map(|c| to_owned_cursor(c).is_truthy()));
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
/// [`partial_generic`]) instead of pushing them here. A `LazySeq` item CAN
/// still fail once materialized here, though (its failure isn't known until
/// `materialize_lazy()` actually pulls it) — that's why this returns
/// `Result`, not a plain `Vec` as it used to before `LazySeq` existed (#725):
/// unlike `LazyKeys`/`LazyIndexRange`, which can never fail to materialize, a
/// `LazySeq` runs arbitrary `map(f)` and can be reached here (e.g. via
/// `.[] | (keys_unsorted | map(f))`) before any materialization has
/// happened.
fn flatten_generic_results<V: DocumentValue>(
    items: Vec<GenericResult<V>>,
) -> Result<Vec<OwnedValue>, Control> {
    let mut results = Vec::new();
    for r in items {
        match r.materialize_lazy() {
            GenericResult::One(v) => results.push(to_owned(&v)),
            GenericResult::OneCursor(c) => results.push(to_owned_cursor(&c)),
            GenericResult::Many(rs) => results.extend(rs.iter().map(to_owned)),
            GenericResult::ManyCursor(cs) => {
                results.extend(cs.iter().map(to_owned_cursor));
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
    LazyKeys { fields: V::Fields, sorted: bool },

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
    LazySeq(LazySeq<V>),

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
            Self::LazyKeys { fields, sorted } => {
                Self::Owned(materialize_lazy_keys::<V>(&fields, sorted))
            }
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
    pub fn into_owned(self) -> Option<OwnedValue> {
        match self.materialize_lazy() {
            Self::One(v) => Some(to_owned(&v)),
            Self::OneCursor(c) => Some(to_owned_cursor(&c)),
            Self::Many(vs) => Some(OwnedValue::Array(vs.iter().map(to_owned).collect())),
            Self::ManyCursor(cs) => {
                Some(OwnedValue::Array(cs.iter().map(to_owned_cursor).collect()))
            }
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
        }
    }

    /// Collect all results into a Vec of OwnedValues.
    ///
    /// A `Partial` collects its prefix — the whole point of #400/#494 is
    /// that those outputs are no longer discarded.
    pub fn collect_owned(self) -> Vec<OwnedValue> {
        match self.materialize_lazy() {
            Self::One(v) => vec![to_owned(&v)],
            Self::OneCursor(c) => vec![to_owned_cursor(&c)],
            Self::Many(vs) => vs.iter().map(to_owned).collect(),
            Self::ManyCursor(cs) => cs.iter().map(to_owned_cursor).collect(),
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
    /// - `indent`: indentation width/unit (`IndentSpec::COMPACT` for compact)
    /// - `sort_keys`: sort mapping/object keys before writing (`-S`/`--sort-keys`)
    ///
    /// Returns the number of values streamed and whether the last was falsy.
    pub fn stream_json<W: core::fmt::Write>(
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
                // Convert to owned for streaming
                let owned = to_owned(v);
                owned.stream_json(out, indent, sort_keys)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait
                c.stream_json(out, indent, sort_keys)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::Many(vs) => {
                for v in vs {
                    let owned = to_owned(v);
                    owned.stream_json(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = vs.len();
            }
            Self::LazyKeys { fields, sorted } => {
                if *sorted {
                    // Fallback: materialize+sort. Sorting requires seeing
                    // every key first, so this can't stream lazily like the
                    // unsorted case below.
                    let owned = materialize_lazy_keys::<V>(fields, true);
                    owned.stream_json(out, indent, sort_keys)?;
                } else {
                    // Genuinely lazy (#685): each key is pulled from
                    // `fields` and written straight to `out` as it's
                    // produced — no `Vec<String>` or `OwnedValue::Array` is
                    // ever built. Reachable from `yq_runner.rs`'s M2 fast
                    // path now that `can_use_m2_streaming` admits
                    // `Builtin::KeysUnsorted`. `sort_keys` (`-S`) is a no-op
                    // here: this is a flat array of key names, not a
                    // mapping, so there's nothing for `-S` to reorder.
                    crate::jq::stream::stream_lazy_keys_json(fields, out, indent)?;
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
            // Deliberately NOT a zero-buffer writer like `LazyKeys`'s
            // unsorted arm above: `map`'s array construction is atomic in
            // real jq (`[1,2,"x"]|map(.+1)` prints nothing to stdout, only
            // the stderr diagnostic), but a byte-at-a-time writer would
            // already have flushed `[1,2,` to `out` before discovering
            // element 3 fails — wrong, unfixable-after-the-fact output. One
            // atomic forward pass via `materialize_atomic`, then reuse the
            // already-tested `OwnedValue::stream_json` — this still avoids
            // the reserialize+reindex+separate-evaluator round trip that was
            // the actual expensive part; a single `Vec<OwnedValue>` for the
            // final array was never it.
            Self::LazySeq(seq) => match seq.clone().materialize_atomic() {
                Ok(owned) => {
                    owned.stream_json(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.count = 1;
                    stats.last_was_falsy = owned.is_falsy();
                    stats.any_truthy = !stats.last_was_falsy;
                }
                Err(control) => {
                    let (error, halt) = control_to_stream_outcome(&control);
                    stats.error = error;
                    stats.halt = halt;
                }
            },
            Self::ManyCursor(cs) => {
                for c in cs {
                    c.stream_json(out, indent, sort_keys)?;
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
                o.stream_json(out, indent, sort_keys)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = o.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::ManyOwned(os) => {
                for o in os {
                    o.stream_json(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
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
                    o.stream_json(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
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
                let owned = to_owned(v);
                owned.stream_yaml(out, indent, sort_keys)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = owned.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::OneCursor(c) => {
                // Stream directly from cursor using DocumentCursor trait.
                // `stream_yaml_as_document` (not the bare `stream_yaml`): a
                // navigated container result keeps its own trailing comment
                // just like the whole document does, unlike a navigated
                // scalar (#793a).
                c.stream_yaml_as_document(out, indent, sort_keys)?;
                on_value(out)?;
                stats.count = 1;
                stats.last_was_falsy = c.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::Many(vs) => {
                for v in vs {
                    let owned = to_owned(v);
                    owned.stream_yaml(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = owned.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = vs.len();
            }
            Self::ManyCursor(cs) => {
                for c in cs {
                    // See `OneCursor` above: each streamed result keeps its
                    // own trailing comment if it's a container (#793a).
                    c.stream_yaml_as_document(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = c.is_falsy();
                    stats.any_truthy |= !stats.last_was_falsy;
                }
                stats.count = cs.len();
            }
            Self::LazyKeys { fields, sorted } => {
                if *sorted {
                    // Fallback: materialize+sort. See `stream_json`'s
                    // `LazyKeys` arm above — same reasoning.
                    let owned = materialize_lazy_keys::<V>(fields, true);
                    owned.stream_yaml(out, indent, sort_keys)?;
                } else {
                    // Genuinely lazy (#685): see `stream_json`'s
                    // `LazyKeys` arm above — same reasoning, YAML target.
                    crate::jq::stream::stream_lazy_keys_yaml(fields, out, indent)?;
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
            // Same atomicity reasoning as `stream_json`'s `LazySeq` arm
            // above — no zero-buffer writer, one atomic forward pass then
            // reuse `OwnedValue::stream_yaml`.
            Self::LazySeq(seq) => match seq.clone().materialize_atomic() {
                Ok(owned) => {
                    owned.stream_yaml(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.count = 1;
                    stats.last_was_falsy = owned.is_falsy();
                    stats.any_truthy = !stats.last_was_falsy;
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
                stats.last_was_falsy = o.is_falsy();
                stats.any_truthy = !stats.last_was_falsy;
            }
            Self::ManyOwned(os) => {
                for o in os {
                    o.stream_yaml(out, indent, sort_keys)?;
                    on_value(out)?;
                    stats.last_was_falsy = o.is_falsy();
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
                    stats.last_was_falsy = o.is_falsy();
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
                GenericResult::Error(EvalError::cannot_index_with_field(
                    tagged_type_name(&value, cursor),
                    name,
                ))
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
            } else {
                // No `optional`-guarded arm here (unlike `index_one_generic`,
                // the computed-key sibling this literal `.[N]` form doesn't
                // share code with): `.[N]?` parses straight to
                // `Expr::Optional(Expr::Index(N))`, which isn't
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
                GenericResult::Error(EvalError::cannot_iterate(&to_owned_with_cursor(
                    &value, cursor,
                )))
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
        Expr::Optional(inner) => match eval_single::<S, _>(inner, value, optional, cursor) {
            GenericResult::Error(_) | GenericResult::Break(_) => GenericResult::None,
            // `prefix` is never empty here: `partial_generic` (and
            // `eval::partial`, its mirror) already collapse an empty prefix
            // to the bare `Error`/`Break` variant above before a `Partial`
            // ever gets constructed (#400, #494) — the same invariant the
            // unconditional `.next().unwrap()` elsewhere in this file (e.g.
            // `eval_first_or_last_generic`) relies on.
            GenericResult::Partial(prefix, Control::Error(_) | Control::Break(_)) => {
                match prefix.len() {
                    1 => GenericResult::Owned(prefix.into_iter().next().unwrap()),
                    _ => GenericResult::ManyOwned(prefix),
                }
            }
            // A `LazySeq` hasn't necessarily failed *yet* -- it's lazy, so
            // it never matches the `Error`/`Break`/`Partial` arms above even
            // when pulling it would fail (#724, #725). `E?` needs to know
            // *now* whether `E` fails, so force it here -- same one-pass
            // cost `materialize_atomic` already pays at every other
            // materializing boundary in this file. Without this arm, `inner`
            // falls through to `other => other` below and escapes this `?`
            // entirely: the error only surfaces later, at whatever
            // downstream site finally pulls the `LazySeq`, by which point
            // this `try`/`catch` boundary is long gone (confirmed against
            // real jq: `[1,2,"x"]|map(.+1)?` is empty/exit-0 in jq, but
            // errored/exit-5 here before this arm existed).
            GenericResult::LazySeq(seq) => match seq.materialize_atomic() {
                Ok(owned) => GenericResult::Owned(owned),
                Err(Control::Error(_) | Control::Break(_)) => GenericResult::None,
                // Unlike `Error`/`Break` just above, `?` must NOT catch a
                // `halt` (verified against real jq: `("x"|halt_error)? //
                // "fallback"` still exits 5, it does not fall back) — so this
                // deliberately does not join the caught pattern; it
                // propagates instead, mirroring how the `other => other`
                // wildcard below already lets a bare `GenericResult::Halt`
                // (and a `Partial` ending in `Control::Halt`) pass through
                // uncaught.
                Err(Control::Halt(code)) => GenericResult::Halt(code),
            },
            other => other,
        },

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
                let owned = to_owned_with_cursor(&value, cursor);
                return eval_on_owned::<S, _>(&Expr::Pipe(exprs.clone()), owned, optional);
            }

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
                            | GenericResult::Break(_)
                            | GenericResult::Halt(_)) => p,
                            GenericResult::None => partial_generic(Vec::new(), outer_control),
                            GenericResult::ManyOwned(results) => {
                                partial_generic(results, outer_control)
                            }
                            _ => unreachable!(
                                "eval_on_many_owned only returns None/ManyOwned/Error/Break/Halt/Partial"
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
                            match eval_single::<S, _>(expr, v, optional, None).materialize_lazy() {
                                GenericResult::One(r) => results.push(to_owned(&r)),
                                GenericResult::OneCursor(c) => results.push(to_owned_cursor(&c)),
                                GenericResult::Many(rs) => {
                                    results.extend(rs.iter().map(to_owned));
                                }
                                GenericResult::ManyCursor(cs) => {
                                    results.extend(cs.iter().map(to_owned_cursor));
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
                                    unreachable!(
                                        "materialize_lazy() already normalized every lazy variant"
                                    )
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
                                // longer vanish (#400, #494). If an earlier
                                // element's own buffered `LazySeq` also fails
                                // once `flatten_generic_results` materializes
                                // it, that earlier failure wins -- it's
                                // chronologically first in evaluation order.
                                GenericResult::Error(e) => {
                                    return match flatten_generic_results(per_element) {
                                        Ok(prefix) => partial_generic(prefix, Control::Error(e)),
                                        Err(Control::Error(earlier)) => {
                                            GenericResult::Error(earlier)
                                        }
                                        Err(Control::Break(label)) => GenericResult::Break(label),
                                        Err(Control::Halt(code)) => GenericResult::Halt(code),
                                    };
                                }
                                GenericResult::Break(label) => {
                                    return match flatten_generic_results(per_element) {
                                        Ok(prefix) => {
                                            partial_generic(prefix, Control::Break(label))
                                        }
                                        Err(Control::Error(earlier)) => {
                                            GenericResult::Error(earlier)
                                        }
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
                                        Err(Control::Error(earlier)) => {
                                            GenericResult::Error(earlier)
                                        }
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
                                        Err(Control::Error(earlier)) => {
                                            GenericResult::Error(earlier)
                                        }
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
                    GenericResult::LazyKeys { fields, sorted } => match unwrap_paren(expr) {
                        // Order-independent for both `keys` and
                        // `keys_unsorted` — the one fast path #683 adds for
                        // sorted `keys`.
                        Expr::Builtin(Builtin::Length) => {
                            GenericResult::Owned(OwnedValue::Int(fields.len() as i64))
                        }
                        // Document order is a valid answer only for
                        // `keys_unsorted`. `keys` needs lexicographic order
                        // for these and falls through to the shared
                        // materialize-(and-sort) fallback below. Do not drop
                        // the `if !sorted` guard on a new arm here without
                        // re-deriving why document order would still be a
                        // correct answer.
                        Expr::Iterate if !sorted => {
                            let mut cursors = Vec::new();
                            let mut current = fields;
                            while let Some((field, rest)) = current.uncons() {
                                cursors.push(field.key_cursor);
                                current = rest;
                            }
                            if cursors.is_empty() {
                                GenericResult::None
                            } else {
                                GenericResult::ManyCursor(cursors)
                            }
                        }
                        Expr::Index(idx) if !sorted => {
                            // Negative indices need the length to normalize
                            // against, same as `Expr::Index`'s array arm
                            // above; positive indices skip straight to the
                            // walk. Out-of-bounds is `null`, never an error
                            // (#307), matching that same arm.
                            let target = if *idx < 0 {
                                let len = fields.len();
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
                                Some(target) => {
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
                                None => GenericResult::Owned(OwnedValue::Null),
                            }
                        }
                        Expr::Builtin(Builtin::First) if !sorted => match fields.uncons() {
                            Some((field, _)) => GenericResult::OneCursor(field.key_cursor),
                            None => GenericResult::Owned(OwnedValue::Null),
                        },
                        Expr::Builtin(Builtin::Last) if !sorted => {
                            let mut current = fields;
                            let mut last_cursor = None;
                            while let Some((field, rest)) = current.uncons() {
                                last_cursor = Some(field.key_cursor);
                                current = rest;
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
                        // first.
                        Expr::Builtin(Builtin::Map(f)) if !sorted => GenericResult::LazySeq(
                            LazySeq::new(LazySource::Keys(fields)).push_map(f, S::TAG),
                        ),
                        _ => eval_on_owned::<S, _>(
                            expr,
                            materialize_lazy_keys::<V>(&fields, sorted),
                            optional,
                        ),
                    },
                    // The array counterpart of `LazyKeys` above
                    // (#684): the index range `[0, 1, ..., len-1]` is fully
                    // determined by `len` alone, so `length`, `.[]`, `.[n]`,
                    // `first`, and `last` are plain arithmetic on `len` — no
                    // allocation at all, not even a `Vec<V::Cursor>` (there's
                    // no cursor to point at: array-index "keys" are
                    // synthetic, not bytes in the source document).
                    GenericResult::LazyIndexRange(len) => match unwrap_paren(expr) {
                        Expr::Builtin(Builtin::Length) => {
                            GenericResult::Owned(OwnedValue::Int(len as i64))
                        }
                        Expr::Iterate => {
                            if len == 0 {
                                GenericResult::None
                            } else {
                                GenericResult::ManyOwned(
                                    (0..len).map(|i| OwnedValue::Int(i as i64)).collect(),
                                )
                            }
                        }
                        Expr::Index(idx) => {
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
                                Some(i) if i < len => {
                                    GenericResult::Owned(OwnedValue::Int(i as i64))
                                }
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
                        Expr::Builtin(Builtin::Map(f)) => GenericResult::LazySeq(
                            LazySeq::new(LazySource::IndexRange { next: 0, len })
                                .push_map(f, S::TAG),
                        ),
                        _ => {
                            eval_on_owned::<S, _>(expr, materialize_lazy_index_range(len), optional)
                        }
                    },
                    // The composability engine (#724, #725): every further
                    // `| map(g)` stage just pushes onto the same chain
                    // (self-recursive by construction — an arbitrary-length
                    // `map(f) | map(g) | map(h)` stays one `LazySeq`, not one
                    // type per depth). A handful of consumers get a genuine
                    // single-forward-pass native fast path; everything else
                    // materializes once (`materialize_atomic`) and hands off
                    // to the full evaluator — still one pass, not the
                    // original four-pass round trip.
                    GenericResult::LazySeq(mut seq) => match unwrap_paren(expr) {
                        Expr::Builtin(Builtin::Map(h)) => {
                            GenericResult::LazySeq(seq.push_map(h, S::TAG))
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
                                    Err(Control::Break(label)) => {
                                        return GenericResult::Break(label)
                                    }
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
                                    Err(Control::Break(label)) => {
                                        return GenericResult::Break(label)
                                    }
                                    Err(Control::Halt(code)) => return GenericResult::Halt(code),
                                }
                            }
                            let all_cursor =
                                items.iter().all(|item| matches!(item, LazyElem::Cursor(_)));
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
                                GenericResult::ManyOwned(
                                    items
                                        .into_iter()
                                        .map(|item| match item {
                                            LazyElem::Cursor(c) => to_owned_cursor(&c),
                                            LazyElem::Owned(o) => o,
                                        })
                                        .collect(),
                                )
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
                        Expr::Builtin(Builtin::First) | Expr::Index(0) => match seq.next() {
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
                    },
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

        Expr::Literal(lit) => match lit {
            Literal::Null => GenericResult::Owned(OwnedValue::Null),
            Literal::Bool(b) => GenericResult::Owned(OwnedValue::Bool(*b)),
            Literal::Int(i) => GenericResult::Owned(OwnedValue::Int(*i)),
            Literal::Float(f) => GenericResult::Owned(OwnedValue::Float(*f)),
            Literal::String(s) => GenericResult::Owned(OwnedValue::String(s.clone())),
        },

        // Formats are pure functions of the value, so evaluate them here rather
        // than falling through to the catch-all, which would serialize the
        // value to JSON and rebuild a `JsonIndex` for every one (#124).
        Expr::Format(format_type) => {
            format_result(format_type, &to_owned_with_cursor(&value, cursor), optional)
        }

        Expr::Builtin(builtin) => eval_builtin::<S, _>(builtin, value, optional, cursor),

        // Comparison operations - handle locally to preserve cursor context,
        // over every pairing jq's cartesian fanout produces rather than just
        // the first (#768): right operand outer, left operand inner, mirroring
        // `eval::eval_binary_fanout`. `left`/`right` are evaluated at a
        // hardcoded `optional: false` (unchanged from before this fix) --
        // only the final comparison step below consults the ambient
        // `optional`, same split as `Expr::IndexExpr`'s key/bounds above.
        Expr::Compare { op, left, right } => {
            let mut right_vals = Vec::new();
            let right_control = push_generic_owned_values(
                eval_single::<S, _>(right, value.clone(), false, cursor),
                &mut right_vals,
            );

            let mut out: Vec<OwnedValue> = Vec::new();
            for right_val in &right_vals {
                let mut left_vals = Vec::new();
                let left_control = push_generic_owned_values(
                    eval_single::<S, _>(left, value.clone(), false, cursor),
                    &mut left_vals,
                );

                for left_val in left_vals {
                    let result = match op {
                        CompareOp::Eq => left_val == *right_val,
                        CompareOp::Ne => left_val != *right_val,
                        CompareOp::Lt => {
                            compare_values(&left_val, right_val) == core::cmp::Ordering::Less
                        }
                        CompareOp::Le => matches!(
                            compare_values(&left_val, right_val),
                            core::cmp::Ordering::Less | core::cmp::Ordering::Equal
                        ),
                        CompareOp::Gt => {
                            compare_values(&left_val, right_val) == core::cmp::Ordering::Greater
                        }
                        CompareOp::Ge => matches!(
                            compare_values(&left_val, right_val),
                            core::cmp::Ordering::Greater | core::cmp::Ordering::Equal
                        ),
                    };
                    out.push(OwnedValue::Bool(result));
                }

                if let Some(control) = left_control {
                    return finish_fork_generic(out, Some(control), optional);
                }
            }

            finish_fork_generic(out, right_control, optional)
        }

        // Fall back to the full evaluator for complex expressions
        _ => {
            // Convert to OwnedValue, then to JSON, then evaluate with full evaluator
            let owned = to_owned_with_cursor(&value, cursor);
            let json_str = owned.to_json_for_reindex();
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
                QueryResult::Halt(code) => GenericResult::Halt(code),
                QueryResult::Partial(vs, control) => GenericResult::Partial(vs, control),
            }
        }
    }
}

/// Evaluate `first(inner)`/`last(inner)` (and the `Builtin::FirstStream`/
/// `LastStream` spelling the parser sometimes produces for the same syntax --
/// see the call sites in `eval_single`/`eval_builtin`), preserving a cursor
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
    let result = eval_single::<S, V>(inner, value, optional, cursor);
    if want_last {
        match result {
            GenericResult::One(v) => GenericResult::One(v),
            GenericResult::OneCursor(c) => GenericResult::OneCursor(c),
            GenericResult::Many(vs) => match vs.into_iter().next_back() {
                Some(v) => GenericResult::One(v),
                None => GenericResult::None,
            },
            GenericResult::ManyCursor(cs) => match cs.into_iter().next_back() {
                Some(c) => GenericResult::OneCursor(c),
                None => GenericResult::None,
            },
            GenericResult::Owned(v) => GenericResult::Owned(v),
            GenericResult::ManyOwned(vs) => match vs.into_iter().next_back() {
                Some(v) => GenericResult::Owned(v),
                None => GenericResult::None,
            },
            // `inner`'s stream has exactly one output (the whole `keys`/
            // `keys_unsorted` result) — forward it unchanged, same as
            // `Owned`/`OneCursor` above, so laziness survives `last(...)`.
            GenericResult::LazyKeys { fields, sorted } => {
                GenericResult::LazyKeys { fields, sorted }
            }
            GenericResult::LazyIndexRange(len) => GenericResult::LazyIndexRange(len),
            // Same forwarding, same reasoning: `first(inner)`/`last(inner)`
            // only need to know *which* of `inner`'s outputs this is, never
            // inspect the value itself, so forwarding doesn't swallow
            // anything -- whoever consumes the returned `LazySeq` next
            // materializes it, and any error surfaces there.
            GenericResult::LazySeq(seq) => GenericResult::LazySeq(seq),
            GenericResult::None => GenericResult::None,
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
    } else {
        match result {
            GenericResult::One(v) => GenericResult::One(v),
            GenericResult::OneCursor(c) => GenericResult::OneCursor(c),
            GenericResult::Many(vs) => match vs.into_iter().next() {
                Some(v) => GenericResult::One(v),
                None => GenericResult::None,
            },
            GenericResult::ManyCursor(cs) => match cs.into_iter().next() {
                Some(c) => GenericResult::OneCursor(c),
                None => GenericResult::None,
            },
            GenericResult::Owned(v) => GenericResult::Owned(v),
            GenericResult::ManyOwned(vs) => match vs.into_iter().next() {
                Some(v) => GenericResult::Owned(v),
                None => GenericResult::None,
            },
            // Same forwarding as the `want_last` branch above.
            GenericResult::LazyKeys { fields, sorted } => {
                GenericResult::LazyKeys { fields, sorted }
            }
            GenericResult::LazyIndexRange(len) => GenericResult::LazyIndexRange(len),
            GenericResult::LazySeq(seq) => GenericResult::LazySeq(seq),
            GenericResult::None => GenericResult::None,
            GenericResult::Error(e) => GenericResult::Error(e),
            GenericResult::Break(label) => GenericResult::Break(label),
            GenericResult::Halt(code) => GenericResult::Halt(code),
            // jq's generator-based `first` never asks for values past the
            // first (verified: `first(1,2,error("x"))` is `1`, exit 0 -- the
            // error is never reached), so a non-empty prefix always
            // satisfies it and the trailing control is dropped. `Partial`'s
            // prefix is never empty by construction (matches
            // `eval::eval_first_expr`).
            GenericResult::Partial(vs, _control) => {
                GenericResult::Owned(vs.into_iter().next().expect("Partial prefix non-empty"))
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
                match fields.find_cursor(s) {
                    Some(c) => GenericResult::OneCursor(c),
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
        GenericResult::One(v) => vec![to_owned_key_shape(&v)],
        GenericResult::OneCursor(c) => vec![to_owned_key_shape_cursor(&c)],
        GenericResult::Many(vs) => vs.iter().map(to_owned_key_shape).collect(),
        GenericResult::ManyCursor(cs) => cs.iter().map(to_owned_key_shape_cursor).collect(),
        other => other.collect_owned(),
    };
    if keys.is_empty() {
        // `partial_generic`'s invariant (a non-empty prefix by construction)
        // means this is only reachable with `pending_halt` unset.
        return GenericResult::None;
    }

    let targets = match eval_single::<S, V>(target, value, false, cursor) {
        GenericResult::Error(e) => return GenericResult::Error(e),
        GenericResult::Break(label) => return GenericResult::Break(label),
        // Same reasoning as the `keys` match above: kept out of the
        // `owned @ (...)` group below, whose `collect_owned()` call would
        // otherwise quietly turn a halted target stream into an empty `Vec`.
        GenericResult::Halt(code) => return GenericResult::Halt(code),
        // A target with zero outputs indexes to zero results for every key
        // — not an error, break, or halt of its own — so it has nothing
        // that "happens first" to preempt a pending halt from the key side
        // (unlike the other arms here, each itself a terminating event
        // during target evaluation for the first already-known key, and so
        // legitimately preempting a key halt not yet reached). The key
        // generator's own halt is still the only terminating event here and
        // must still fire (#791) — mirrors `eval::eval_index_expr`.
        GenericResult::None => {
            return match pending_halt {
                Some(code) => GenericResult::Halt(code),
                None => GenericResult::None,
            };
        }
        // Computed indexing's key/target forking isn't part of #400/#494's
        // verified semantics — conservatively matching the Error/Break arms
        // above rather than inventing new partial-target behavior, same as
        // `eval::eval_index_expr`.
        GenericResult::Partial(_, Control::Error(e)) => return GenericResult::Error(e),
        GenericResult::Partial(_, Control::Break(label)) => return GenericResult::Break(label),
        GenericResult::Partial(_, Control::Halt(code)) => return GenericResult::Halt(code),
        GenericResult::One(v) => vec![v],
        GenericResult::Many(vs) => vs,
        GenericResult::OneCursor(c) => vec![c.value()],
        GenericResult::ManyCursor(cs) => cs.iter().map(DocumentCursor::value).collect(),
        // An owned target (a computed, non-navigational left side) has no
        // borrowed representation here; round-trip it through the shared
        // owned-value path by re-entering with the materialized document.
        // `LazySeq` shares this arm too: `collect_owned()` materializes it
        // (via `materialize_lazy()`) the same as the other lazy variants.
        // Known, narrow, pre-existing gap, not a new regression:
        // `collect_owned()` already silently swallows any error into an
        // empty `Vec` for every variant that can error (`Partial`, and now
        // a failing `LazySeq`) -- `(keys_unsorted|map(error("x")))[$k]`
        // swallows the error into "no results" rather than propagating it,
        // same as it already did for other error-producing shapes here.
        owned @ (GenericResult::Owned(_)
        | GenericResult::ManyOwned(_)
        | GenericResult::LazyKeys { .. }
        | GenericResult::LazyIndexRange(_)
        | GenericResult::LazySeq(_)) => {
            let targets = owned.collect_owned();
            let mut out = Vec::with_capacity(keys.len() * targets.len());
            for k in &keys {
                for t in &targets {
                    match index_owned_by_key(t, k, optional) {
                        Ok(Some(v)) => out.push(v),
                        Ok(None) => {}
                        // A later key's index error outranks an earlier key's
                        // still-pending halt (verified against jq 1.7.1/1.8.2:
                        // `{"a":1} | .[("a", 5, halt)]` prints `1`, then the
                        // "Cannot index object with number" error, and never
                        // reaches `halt`) — the already-indexed prefix must
                        // still survive as `Partial`, not vanish with it.
                        Err(e) => return partial_generic(out, Control::Error(e)),
                    }
                }
            }
            return match pending_halt {
                Some(code) => partial_generic(out, Control::Halt(code)),
                None => match out.len() {
                    1 => GenericResult::Owned(out.pop().expect("len checked")),
                    _ => GenericResult::ManyOwned(out),
                },
            };
        }
    };

    // Key outer, target inner.
    let mut cursors: Vec<V::Cursor> = Vec::new();
    let mut owned: Vec<OwnedValue> = Vec::new();
    let mut any_owned = false;
    for k in &keys {
        for t in &targets {
            match index_one_generic::<V>(t.clone(), k, optional) {
                GenericResult::OneCursor(c) => {
                    if any_owned {
                        owned.push(to_owned_cursor(&c));
                    } else {
                        cursors.push(c);
                    }
                }
                GenericResult::Owned(v) => {
                    if !any_owned {
                        any_owned = true;
                        owned = cursors.iter().map(to_owned_cursor).collect();
                        cursors.clear();
                    }
                    owned.push(v);
                }
                GenericResult::None => {}
                // Same reasoning as the `owned @ (...)` arm above: a later
                // key's index error outranks an earlier key's pending halt,
                // and the already-indexed prefix survives it as `Partial`.
                GenericResult::Error(e) => {
                    let out = if any_owned {
                        owned
                    } else {
                        cursors.iter().map(to_owned_cursor).collect()
                    };
                    return partial_generic(out, Control::Error(e));
                }
                _ => unreachable!("index_one_generic yields OneCursor/Owned/None/Error"),
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
            cursors.iter().map(to_owned_cursor).collect()
        };
        return partial_generic(out, Control::Halt(code));
    }

    if any_owned {
        match owned.len() {
            1 => GenericResult::Owned(owned.pop().expect("len checked")),
            _ => GenericResult::ManyOwned(owned),
        }
    } else {
        match cursors.len() {
            1 => GenericResult::OneCursor(cursors.pop().expect("len checked")),
            _ => GenericResult::ManyCursor(cursors),
        }
    }
}

/// Evaluate `E[S:T]` — slicing by computed bounds.
///
/// The counterpart of `eval::eval_slice_expr`; mirrors `eval_index_expr`
/// immediately above — `start`/`end` are evaluated first (outermost) against
/// the original value/cursor, not the target's output, and an empty bound
/// stream short-circuits before the target is evaluated at all. See #615.
fn eval_slice_expr<S: EvalSemantics, V: DocumentValue>(
    target: &Expr,
    start: &Option<Box<Expr>>,
    end: &Option<Box<Expr>>,
    value: V,
    optional: bool,
    cursor: Option<V::Cursor>,
) -> GenericResult<V> {
    // Bounds first: an empty start or end stream must not evaluate the
    // target at all.
    let starts = match eval_slice_bound::<S, V>(start, value.clone(), cursor, f64::floor) {
        Ok(v) => v,
        Err(Control::Error(e)) => return GenericResult::Error(e),
        Err(Control::Break(label)) => return GenericResult::Break(label),
        Err(Control::Halt(code)) => return GenericResult::Halt(code),
    };
    if starts.is_empty() {
        return GenericResult::None;
    }
    let ends = match eval_slice_bound::<S, V>(end, value.clone(), cursor, f64::ceil) {
        Ok(v) => v,
        Err(Control::Error(e)) => return GenericResult::Error(e),
        Err(Control::Break(label)) => return GenericResult::Break(label),
        Err(Control::Halt(code)) => return GenericResult::Halt(code),
    };
    if ends.is_empty() {
        return GenericResult::None;
    }

    // Borrowed and owned targets are kept apart so the common (borrowed) case
    // never materializes the document — mirrors `eval_index_expr`.
    enum Targets<V> {
        Borrowed(Vec<V>),
        Owned(Vec<OwnedValue>),
    }
    let targets = match eval_single::<S, V>(target, value, false, cursor) {
        GenericResult::Error(e) => return GenericResult::Error(e),
        GenericResult::Break(label) => return GenericResult::Break(label),
        // Same reasoning as `eval_index_expr`'s target match: kept out of
        // the `owned @ (...)` group below so a halted target stream isn't
        // quietly swallowed into an empty `Vec` by `collect_owned()`.
        GenericResult::Halt(code) => return GenericResult::Halt(code),
        GenericResult::None => return GenericResult::None,
        // Same conservative Error/Break-only handling as `eval_index_expr`'s
        // target Partial arm above.
        GenericResult::Partial(_, Control::Error(e)) => return GenericResult::Error(e),
        GenericResult::Partial(_, Control::Break(label)) => return GenericResult::Break(label),
        GenericResult::Partial(_, Control::Halt(code)) => return GenericResult::Halt(code),
        GenericResult::One(v) => Targets::Borrowed(vec![v]),
        GenericResult::Many(vs) => Targets::Borrowed(vs),
        GenericResult::OneCursor(c) => Targets::Borrowed(vec![c.value()]),
        GenericResult::ManyCursor(cs) => {
            Targets::Borrowed(cs.iter().map(DocumentCursor::value).collect())
        }
        // See `eval_index_expr`'s identical arm for the accepted, pre-existing
        // error-swallowing note that also applies to `LazySeq` here.
        owned @ (GenericResult::Owned(_)
        | GenericResult::ManyOwned(_)
        | GenericResult::LazyKeys { .. }
        | GenericResult::LazyIndexRange(_)
        | GenericResult::LazySeq(_)) => Targets::Owned(owned.collect_owned()),
    };

    // Start outer, end middle, target inner. The result is always owned:
    // slicing constructs a fresh array/string, same invariant as
    // `eval::eval_slice_expr`.
    let mut out: Vec<OwnedValue> = Vec::with_capacity(starts.len() * ends.len());
    match &targets {
        Targets::Borrowed(ts) => {
            for s in &starts {
                for e in &ends {
                    for t in ts {
                        match slice_one_generic::<V>(t.clone(), *s, *e, optional) {
                            GenericResult::Owned(v) => out.push(v),
                            GenericResult::None => {}
                            GenericResult::Error(e) => return GenericResult::Error(e),
                            _ => unreachable!("slice_one_generic yields Owned/None/Error"),
                        }
                    }
                }
            }
        }
        Targets::Owned(ts) => {
            for s in &starts {
                for e in &ends {
                    for t in ts {
                        match slice_owned_value(t, *s, *e, optional) {
                            Ok(Some(v)) => out.push(v),
                            Ok(None) => {}
                            Err(e) => return GenericResult::Error(e),
                        }
                    }
                }
            }
        }
    }
    match out.len() {
        1 => GenericResult::Owned(out.pop().expect("len checked")),
        _ => GenericResult::ManyOwned(out),
    }
}

/// Evaluate one slice bound (`start` or `end`) against `value`/`cursor`.
/// `round` is `f64::floor` for a start bound, `f64::ceil` for an end bound, so
/// a fractional dynamic bound still widens the slice the way a literal one
/// does — see `eval::eval_slice_bound`. A missing bound (`None`) is a single
/// `None` ("open on this side"), not an empty stream.
fn eval_slice_bound<S: EvalSemantics, V: DocumentValue>(
    bound: &Option<Box<Expr>>,
    value: V,
    cursor: Option<V::Cursor>,
    round: fn(f64) -> f64,
) -> Result<Vec<Option<i64>>, Control> {
    let Some(expr) = bound else {
        return Ok(vec![None]);
    };
    let raw = match eval_single::<S, V>(expr, value, false, cursor) {
        GenericResult::Error(e) => return Err(Control::Error(e)),
        GenericResult::Break(label) => return Err(Control::Break(label)),
        // Falling into `other => other.collect_owned()` below would discard
        // the halt into an empty bound list instead of propagating it, so a
        // dynamic slice bound like `.[:halt_error(3)]` would silently exit 0
        // (#791).
        GenericResult::Halt(code) => return Err(Control::Halt(code)),
        GenericResult::None => return Ok(Vec::new()),
        GenericResult::Partial(_, control) => return Err(control),
        GenericResult::One(v) => vec![to_owned_key_shape(&v)],
        GenericResult::OneCursor(c) => vec![to_owned_key_shape_cursor(&c)],
        GenericResult::Many(vs) => vs.iter().map(to_owned_key_shape).collect(),
        GenericResult::ManyCursor(cs) => cs.iter().map(to_owned_key_shape_cursor).collect(),
        other => other.collect_owned(),
    };
    raw.iter()
        .map(|v| owned_bound_to_i64(v, round).map_err(Control::Error))
        .collect()
}

/// Apply resolved bounds to one borrowed target. Mirrors `eval::eval_single`'s
/// `Expr::Slice` arm (arrays/strings/null) directly against `DocumentValue` —
/// there is no native `Expr::Slice` arm here to delegate to, unlike the
/// non-generic evaluator. Always returns owned, since slicing always
/// constructs a fresh value; parallels `index_one_generic` returning `One` or
/// `Owned`.
fn slice_one_generic<V: DocumentValue>(
    target: V,
    start: Option<i64>,
    end: Option<i64>,
    optional: bool,
) -> GenericResult<V> {
    if let Some(elements) = target.as_array() {
        let items = elements.collect_values();
        let range = SliceBounds::from_literals(start, end).resolve(items.len());
        return GenericResult::Owned(OwnedValue::Array(
            items[range].iter().map(to_owned).collect(),
        ));
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
                        Some(c) => core::iter::repeat_with(|| to_owned_cursor(&c))
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
                GenericResult::LazySeq(
                    LazySeq::new(LazySource::Elements(elements)).push_map(f, S::TAG),
                )
            } else if let Some(fields) = value.as_object() {
                GenericResult::LazySeq(LazySeq::new(LazySource::Values(fields)).push_map(f, S::TAG))
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_iterate(&to_owned_with_cursor(
                    &value, cursor,
                )))
            }
        }

        Builtin::Shuffle => {
            #[cfg(feature = "cli")]
            {
                use rand::seq::SliceRandom;
                use rand::SeedableRng;
                use rand_chacha::ChaCha8Rng;

                if let Some(elements) = value.as_array() {
                    let mut values: Vec<OwnedValue> = elements
                        .collect_cursors()
                        .iter()
                        .map(to_owned_cursor)
                        .collect();
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
                let items: Vec<OwnedValue> = elements
                    .collect_cursors()
                    .iter()
                    .map(to_owned_cursor)
                    .collect();
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
                GenericResult::Error(EvalError::has_no_length(&to_owned_with_cursor(
                    &value, cursor,
                )))
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
                }
            } else if let Some(elements) = value.as_array() {
                // `[0, 1, ..., len-1]` is already sorted, so `Keys` needs no
                // extra `.sort()` here, unlike the object branch above. Stay
                // lazy (#684) — don't materialize a `Vec<OwnedValue::Int>`
                // yet; `length`, `.[]`, `.[n]`, `first`, and `last` can all
                // answer directly from `len` (see the `Pipe` dispatch below).
                GenericResult::LazyIndexRange(elements.len())
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::has_no_keys(&to_owned_with_cursor(
                    &value, cursor,
                )))
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
                }
            } else if let Some(elements) = value.as_array() {
                // Same laziness as the array branch of `Keys` above (#684).
                GenericResult::LazyIndexRange(elements.len())
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::has_no_keys(&to_owned_with_cursor(
                    &value, cursor,
                )))
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
                    .collect_cursors()
                    .into_iter()
                    .enumerate()
                    .map(|(i, elem_cursor)| {
                        let mut entry = IndexMap::new();
                        entry.insert("key".to_string(), OwnedValue::Int(i as i64));
                        entry.insert("value".to_string(), to_owned_cursor(&elem_cursor));
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
                        entry.insert("value".to_string(), to_owned_cursor(&field.value_cursor));
                        entries.push(OwnedValue::Object(entry));
                    }
                    f = rest;
                }
                GenericResult::Owned(OwnedValue::Array(entries))
            } else {
                // No `optional`-guarded arm here: `Builtin::ToEntries` isn't
                // `IndexExpr`/`SliceExpr`, so #693's dispatch never forces
                // `optional = true` into this native arm — `to_entries?`
                // evaluates it at the ambient `optional` (normally `false`)
                // and lets the outer `Expr::Optional`/`eval_try`-style catch
                // convert the resulting `Error` to `None` once instead.
                GenericResult::Error(EvalError::has_no_keys(&to_owned_with_cursor(
                    &value, cursor,
                )))
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
                let len = elements.len();
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

        // `first(f)`/`last(f)`: the parser's `parse_call` builtin-name path
        // produces this spelling for the same syntax `Expr::FirstExpr`/
        // `LastExpr` cover (see their arms in `eval_single`) -- both must
        // stay cursor-preserving for the same reason (#607).
        Builtin::FirstStream(inner) => {
            eval_first_or_last_generic::<S, _>(inner, value, optional, cursor, false)
        }
        Builtin::LastStream(inner) => {
            eval_first_or_last_generic::<S, _>(inner, value, optional, cursor, true)
        }

        Builtin::Reverse => {
            if let Some(elements) = value.as_array() {
                let values: Vec<OwnedValue> = elements
                    .collect_cursors()
                    .iter()
                    .rev()
                    .map(to_owned_cursor)
                    .collect();
                GenericResult::Owned(OwnedValue::Array(values))
            } else if optional {
                GenericResult::None
            } else {
                GenericResult::Error(EvalError::cannot_index_with_type(
                    tagged_type_name(&value, cursor),
                    "number",
                ))
            }
        }

        Builtin::Empty => GenericResult::None,

        Builtin::ToString => {
            let owned = to_owned_with_cursor(&value, cursor);
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
                GenericResult::Error(EvalError::cannot_parse_as_number(&to_owned_with_cursor(
                    &value, cursor,
                )))
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
            let owned = to_owned_with_cursor(&value, cursor);
            eval_on_owned::<S, _>(&Expr::Builtin(builtin.clone()), owned, optional)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::expr::FormatType;
    use super::*;
    use crate::jq::parse;
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
            result.collect_owned(),
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
            empty_result.collect_owned(),
            vec![OwnedValue::Array(vec![])]
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
        assert_eq!(halted.collect_owned(), Vec::<OwnedValue>::new());
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
            .stream_json(&mut compact_json, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(compact_json, "[0,1,2]");

        let mut indented_json = String::new();
        result
            .stream_json(&mut indented_json, IndentSpec::spaces(2), false, |_| Ok(()))
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
            result.collect_owned(),
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
    fn test_generic_keys_unsorted_lazy_length() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        let result = eval(&expr, value);
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(3));
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
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(3));
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
            result.collect_owned(),
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_index() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | .[-1]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("c".to_string())
        );

        // Out of bounds is `null`, never an error (#307), matching plain
        // array indexing.
        let expr = crate::jq::parse("keys_unsorted | .[10]").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Null);
    }

    #[test]
    fn test_generic_keys_unsorted_lazy_first_last() {
        let json = br#"{"b": 1, "a": 2, "c": 3}"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Null
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Null);
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
            result.collect_owned(),
            vec![
                OwnedValue::String("B".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("C".to_string()),
            ]
        );

        let expr = crate::jq::parse("keys_unsorted | .[0] | ascii_upcase").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(10_000)
        );

        let expr = crate::jq::parse("keys_unsorted | .[9999]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("k9999".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(3));
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
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(3));
    }

    #[test]
    fn test_generic_keys_sorted_lazy_length_empty_object() {
        let json = br"{}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys | length").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Int(0));
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
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );

        let expr = crate::jq::parse("keys | .[]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).collect_owned(),
            vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ]
        );

        let expr = crate::jq::parse("keys | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("a".to_string())
        );

        let expr = crate::jq::parse("keys | .[-1]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("c".to_string())
        );

        let expr = crate::jq::parse("keys | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("a".to_string())
        );

        let expr = crate::jq::parse("keys | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
            result.into_owned().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("A".to_string()),
                OwnedValue::String("B".to_string()),
                OwnedValue::String("C".to_string()),
            ])
        );

        let expr = crate::jq::parse("keys | select(length == 3)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(10_000)
        );

        // Derive the expected lexicographically-first key the same way the
        // input was generated, rather than hand-computing string order.
        let mut expected_keys: Vec<String> = (0..10_000).map(|i| format!("k{i}")).collect();
        expected_keys.sort();

        let expr = crate::jq::parse("keys | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
        assert_eq!(eval(&expr, value.clone()).into_owned().unwrap(), expected);

        let expr = crate::jq::parse("keys_unsorted").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), expected);
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_length() {
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | length").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Int(3));
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
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Int(3));
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
            result.collect_owned(),
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_index() {
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | .[-1]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(2)
        );

        // Out of bounds is `null`, never an error (#307), matching plain
        // array indexing and the object `keys_unsorted` fast path.
        let expr = crate::jq::parse("keys_unsorted | .[10]").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Null);
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_first_last() {
        let json = br#"["x","y","z"]"#;
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Int(2));
    }

    #[test]
    fn test_generic_array_keys_unsorted_lazy_first_last_empty_array() {
        let json = br"[]";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Null
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Null);
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
            result.into_owned().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::Int(0),
                OwnedValue::Int(10),
                OwnedValue::Int(20),
            ])
        );

        let expr = crate::jq::parse("keys_unsorted | select(length == 3)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
            OwnedValue::Array(vec![])
        );

        let json = br"{}";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();
        let expr = crate::jq::parse("map(.)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
            OwnedValue::Array(vec![])
        );
    }

    #[test]
    fn test_generic_plain_map_non_container_errors_725() {
        // Message must match `eval.rs`'s `builtin_map`/`map_over` exactly
        // (both dispatch through `EvalError::cannot_iterate`).
        let json = br"1";
        let index = JsonIndex::build(json);
        let cursor = index.root(json);
        let value = cursor.value();

        let expr = crate::jq::parse("map(.)").unwrap();
        let result = eval(&expr, value.clone());
        assert!(result.is_error());

        let owned = crate::jq::eval_generic::to_owned(&value);
        let expected = EvalError::cannot_iterate(&owned);
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
        assert_eq!(result.into_owned(), None);

        // Non-erroring case still returns the mapped array, not suppressed.
        let expr = crate::jq::parse("map(. + 1)").unwrap();
        let json_ok = br"[1,2,3]";
        let index_ok = JsonIndex::build(json_ok);
        let value_ok = index_ok.root(json_ok).value();
        assert_eq!(
            eval(&expr, value_ok).into_owned().unwrap(),
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
        assert_eq!(result2.into_owned(), None);
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());

        // Same check at the `stream_json` boundary the CLI actually uses:
        // nothing streams to `out` before the diagnostic.
        let expr = crate::jq::parse("map(. + 1) | .[]").unwrap();
        let result = eval(&expr, value);
        let mut out = String::new();
        result
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
            result.into_owned().unwrap(),
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
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(3)
        );

        let expr = crate::jq::parse("map(ascii_upcase) | .[]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).collect_owned(),
            vec![
                OwnedValue::String("B".to_string()),
                OwnedValue::String("A".to_string()),
                OwnedValue::String("C".to_string()),
            ]
        );

        let expr = crate::jq::parse("map(ascii_upcase) | first").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("B".to_string())
        );

        let expr = crate::jq::parse("map(ascii_upcase) | .[0]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("B".to_string())
        );

        // `.[2]`/`last` intentionally fall to `materialize_atomic` +
        // `eval_on_owned` in this initial design (open risk (c)) --
        // asserting correctness only, not laziness.
        let expr = crate::jq::parse("map(ascii_upcase) | .[2]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::String("C".to_string())
        );

        let expr = crate::jq::parse("map(ascii_upcase) | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
            OwnedValue::String("C".to_string())
        );
    }

    #[test]
    fn test_generic_lazy_seq_first_after_map_skips_later_error_725() {
        // Accepted, deliberate divergence from real jq (see the
        // `Expr::Builtin(Builtin::First) | Expr::Index(0)` arm's own doc
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
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(2)
        );

        let expr = crate::jq::parse("map(. + 1) | .[0]").unwrap();
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Int(2));
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
            eval(&expr, value).into_owned().unwrap(),
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
        assert!(eval(&expr, value.clone()).materialize_lazy().is_error());

        let expr = crate::jq::parse(
            r#"keys_unsorted | map(if . == "b" then error("boom") else . end) | .[]"#,
        )
        .unwrap();
        let result = eval(&expr, value);
        // Same atomicity boundary as the `map`-alone case above: `"a"` does
        // NOT survive as a partial prefix once `.[]` is piped after `map`.
        assert!(result.is_error());
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
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
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(10_000)
        );

        let expr = crate::jq::parse("keys_unsorted | .[9999]").unwrap();
        assert_eq!(
            eval(&expr, value.clone()).into_owned().unwrap(),
            OwnedValue::Int(9999)
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
            single_field.collect_owned(),
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
            iterate_fields.collect_owned(),
            vec![
                OwnedValue::String("string".to_string()),
                OwnedValue::String("number".to_string()),
                OwnedValue::String("string".to_string()),
                OwnedValue::String("number".to_string()),
            ]
        );
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
        assert_eq!(result.collect_owned(), vec![OwnedValue::Int(1)]);
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
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
            result.collect_owned(),
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
            OwnedValue::String("flow".to_string())
        );

        let expr = crate::jq::parse(".b | style").unwrap();
        let result = eval_with_cursor(&expr, doc_cursor);
        assert_eq!(
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
            OwnedValue::String(String::new())
        );

        let result = eval(&Expr::Builtin(Builtin::Style), value);
        assert_eq!(
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
        let owned = result.into_owned().unwrap();

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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
        let owned = result.into_owned().unwrap();
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
            result.into_owned().unwrap(),
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

        let (owned, comments) = to_owned_with_comments(&value, Some(&cursor));
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
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::Int(3)
        );

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).collect_owned(),
            vec![
                OwnedValue::String("b".to_string()),
                OwnedValue::String("a".to_string()),
                OwnedValue::String("c".to_string()),
            ]
        );

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::String("b".to_string())
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
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
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::Int(3)
        );

        // Regression guard: bare `keys` and `keys | first`/`last` must
        // still be fully sorted (`a,b,c`), not document order (`b,a,c`).
        let expr = crate::jq::parse("keys").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::Array(vec![
                OwnedValue::String("a".to_string()),
                OwnedValue::String("b".to_string()),
                OwnedValue::String("c".to_string()),
            ])
        );

        let expr = crate::jq::parse("keys | first").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::String("a".to_string())
        );

        let expr = crate::jq::parse("keys | last").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
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
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::Int(3)
        );

        let expr = crate::jq::parse("keys_unsorted | .[]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).collect_owned(),
            vec![OwnedValue::Int(0), OwnedValue::Int(1), OwnedValue::Int(2)]
        );

        let expr = crate::jq::parse("keys_unsorted | .[0]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | first").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
            OwnedValue::Int(0)
        );

        let expr = crate::jq::parse("keys_unsorted | last").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, doc_cursor).into_owned().unwrap(),
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, r#"["b","a","c"]"#);

        let mut out = String::new();
        result
            .stream_json(&mut out, IndentSpec::spaces(2), false, |_| Ok(()))
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
                    GenericResult::OneCursor(c) => results.push(to_owned_cursor(&c)),
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
            r.stream_json(&mut j, IndentSpec::COMPACT, false, |_| Ok(()))
                .unwrap();
            let mut y = String::new();
            r.stream_yaml(&mut y, IndentSpec::spaces(2), false, |_| Ok(()))
                .unwrap();
        }
        // Spot-check the owned and error stream output.
        let mut owned_json = String::new();
        results[2]
            .stream_json(&mut owned_json, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(owned_json, "5");
        // An error writes nothing to `out` — `out` is stdout, and a diagnostic
        // there would be indistinguishable from a result. It comes back through
        // `stats.error` instead, for the caller to print to stderr (#355).
        let mut err_json = String::new();
        let err_stats = results[5]
            .stream_json(&mut err_json, IndentSpec::COMPACT, false, |_| Ok(()))
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
            .stream_json(&mut brk, IndentSpec::COMPACT, false, |_| Ok(()))
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
            .stream_json(&mut partial_json, IndentSpec::COMPACT, false, |_| {
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
            .stream_json(&mut partial_brk, IndentSpec::COMPACT, false, |_| Ok(()))
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
        result
            .stream_json(&mut j, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(j, "1");
        let mut y = String::new();
        result
            .stream_yaml(&mut y, IndentSpec::spaces(2), false, |_| Ok(()))
            .unwrap();

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
        assert!(result
            .stream_json(&mut j, IndentSpec::spaces(2), false, |_| Ok(()))
            .is_err());
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
            .stream_json(&mut j, IndentSpec::COMPACT, false, |_| Ok(()))
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
            result.collect_owned(),
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
            .unwrap();
        assert_eq!(out, r#"{"b":3,"b":4}"#);
    }

    #[test]
    fn test_json_first_stream_builtin_preserves_duplicate_keys() {
        // `Builtin::FirstStream`/`LastStream` is the second, older internal
        // spelling of `first(f)`/`last(f)` that `eval::resolve_node` already
        // treats as equivalent to `Expr::FirstExpr` (see its comment at
        // eval.rs's `Expr::FirstExpr(inner) | Expr::Builtin(Builtin::FirstStream(inner))`
        // arm) -- not reachable from `crate::jq::parse` for top-level user
        // syntax, so built directly here, mirroring how the `IndexExpr` test
        // above bypasses the parser's own folding.
        let json = br#"[{"a":1,"a":2},{"b":3,"b":4}]"#;
        let index = JsonIndex::build(json);
        let expr = Expr::Builtin(Builtin::FirstStream(Box::new(Expr::Iterate)));

        let result = eval_with_cursor(&expr, index.root(json));
        assert!(result.is_single_cursor());
        let mut out = String::new();
        result
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
            first.collect_owned(),
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
        assert_eq!(first.collect_owned(), vec![OwnedValue::Int(1)]);

        let last = eval(&crate::jq::parse("last(select(true,true))").unwrap(), value);
        assert!(matches!(last, GenericResult::One(_)));
        assert_eq!(last.collect_owned(), vec![OwnedValue::Int(1)]);
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
            .stream_json(&mut json_out, IndentSpec::COMPACT, true, |_| Ok(()))
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
            .stream_json(&mut json_out, IndentSpec::COMPACT, true, |w| {
                core::fmt::Write::write_str(w, ";")
            })
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
        assert!(matches!(first, GenericResult::Owned(OwnedValue::Int(1))));

        let last = eval(&crate::jq::parse("last(1)").unwrap(), value);
        assert!(matches!(last, GenericResult::Owned(OwnedValue::Int(1))));
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
        assert!(matches!(first, GenericResult::Owned(OwnedValue::Int(1))));

        let last = eval(&crate::jq::parse("last(1,2,3)").unwrap(), value);
        assert!(matches!(last, GenericResult::Owned(OwnedValue::Int(3))));
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
        assert!(matches!(result, GenericResult::Owned(OwnedValue::Int(1))));
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
            result.collect_owned(),
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
            result.collect_owned(),
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
            one.collect_owned(),
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
            many.collect_owned(),
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
            result.collect_owned(),
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
            result.collect_owned(),
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
            result.collect_owned(),
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
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
    fn test_json_computed_slice_bounds_partial_bound_collapses_to_its_control() {
        // A bound stream can itself be `Partial` (some outputs, then a
        // control) — `eval_slice_bound` collapses that to the bare control,
        // the same reduction a `Partial` *target* gets elsewhere (#400/#494).
        let json = br#"{"a":[1,2,3]}"#;
        let index = JsonIndex::build(json);

        let expr = crate::jq::parse(r#".a[(1,2,error("x")):2]"#).unwrap();
        assert!(eval_with_cursor(&expr, index.root(json)).is_error());

        let expr = crate::jq::parse(".a[(1,2,break $out):2]").unwrap();
        assert!(matches!(
            eval_with_cursor(&expr, index.root(json)),
            GenericResult::Break(label) if label == "out"
        ));
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
            eval_with_cursor(&expr, index.root(json)).collect_owned(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3)
            ])]
        );

        let expr = crate::jq::parse(".a[.k1:]").unwrap();
        assert_eq!(
            eval_with_cursor(&expr, index.root(json)).collect_owned(),
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
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
            result.collect_owned(),
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
            result.collect_owned(),
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
    }

    #[test]
    fn test_json_computed_slice_bounds_null_target_yields_null() {
        // A borrowed target that resolves to `null` short-circuits inside
        // `slice_one_generic` before the array/string checks below it.
        let json = br#"{"a":null,"k1":0,"k2":1}"#;
        let index = JsonIndex::build(json);
        let expr = crate::jq::parse(".a[.k1:.k2]").unwrap();

        let result = eval_with_cursor(&expr, index.root(json));
        assert_eq!(result.collect_owned(), vec![OwnedValue::Null]);
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
            result.collect_owned(),
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
            result.collect_owned(),
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
            result.collect_owned(),
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
            result.collect_owned(),
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
        // Computed indexing and `select` each consult their operand once, so
        // a `Partial` there is reduced the same way a multi-output operand
        // already is. Comparison used to share this reduction too, but now
        // forks over every output instead (#768) — see
        // `generic_compare_partial_operand_keeps_prefix_768` below.

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

        // Computed slicing (#615) shares the same target-position `Partial`
        // reduction as computed indexing above — one bound (`(0+0)`) is
        // enough to force `eval_slice_expr`'s fast path.
        assert_eq!(
            summarize(b"[[7],[8]]", r#"(.[0],(.[1],error("x")))[(0+0):2]"#),
            Summary::Error("x".to_string())
        );
        assert_eq!(
            summarize(b"[[7],[8]]", "(.[0],(.[1],break $out))[(0+0):2]"),
            Summary::Break("out".to_string())
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
            eval(&expr, value.clone()).collect_owned(),
            vec![OwnedValue::Int(10)]
        );

        let expr = crate::jq::parse("([10,20,30])[select(true,true)]").unwrap();
        assert_eq!(
            eval(&expr, value).collect_owned(),
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
            eval(&expr, value.clone()).collect_owned(),
            vec![OwnedValue::Array(vec![
                OwnedValue::Int(10),
                OwnedValue::Int(20)
            ])]
        );

        let expr = crate::jq::parse("([10,20,30])[select(true,true):2]").unwrap();
        assert_eq!(
            eval(&expr, value).collect_owned(),
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
            eval_with_cursor(&expr, index.root(json)).collect_owned(),
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
            result.into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
            OwnedValue::Array(vec![OwnedValue::Int(2), OwnedValue::Int(3)])
        );

        // A comma of literals -> `GenericResult::ManyOwned`, fanning one
        // source element into several output elements.
        let json = br"[1]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(1, 2)").unwrap();
        assert_eq!(
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value).into_owned().unwrap(),
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
            eval(&expr, value.clone()).into_owned().unwrap(),
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
            result.collect_owned(),
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
            result.collect_owned(),
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
        assert_eq!(result.collect_owned(), Vec::<OwnedValue>::new());
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
        assert_eq!(eval(&expr, value).into_owned().unwrap(), OwnedValue::Null);

        let json = br"[1,2,3]";
        let index = JsonIndex::build(json);
        let value = index.root(json).value();
        let expr = crate::jq::parse("map(.) | first").unwrap();
        let result = eval(&expr, value);
        assert!(matches!(result, GenericResult::OneCursor(_)));
        assert_eq!(result.into_owned().unwrap(), OwnedValue::Int(1));

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
            result.into_owned().unwrap(),
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
            result.into_owned().unwrap(),
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
            .stream_json(&mut out, IndentSpec::COMPACT, false, |_| Ok(()))
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
        assert_eq!(result.into_owned(), None);
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
            result.collect_owned(),
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
}
