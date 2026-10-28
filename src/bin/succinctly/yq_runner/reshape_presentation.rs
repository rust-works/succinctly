//! Bounded provenance for inexpensive YAML reshaping filters (#3615).
//!
//! This is a presentation pass, not an alternative evaluator. Only pure,
//! single-output shapes are replayed, and the caller checks the resulting
//! value against the authoritative generic result before using its tree.

use super::{
    collect_write_targets, is_closed_literal, reconcile_presentation, CommentTree, Expr, IndexMap,
    NodeMeta, NumberRepr, OwnedValue, ResultWithComments,
};
use succinctly::jq::document::{DocumentCursor, DocumentElements, DocumentFields, DocumentValue};
use succinctly::jq::eval_generic::{to_owned_with_comments, AnchorMark, KeyMeta, LazyElem};
use succinctly::jq::YqSemantics;
use succinctly::jq::{ArithOp, Builtin, EvalError, ObjectKey, MAX_VALUE_TREE_DEPTH};

/// A deliberately small allowlist: no variables, environment reads, I/O,
/// clock/random functions, path registers, user functions or control effects.
pub(super) fn supported(expr: &Expr) -> bool {
    if is_closed_literal(expr) {
        return true;
    }
    match expr {
        Expr::Identity | Expr::Field(_) | Expr::Index { .. } => true,
        Expr::Paren(inner) => supported(inner),
        Expr::Pipe(parts) => parts.iter().all(supported),
        Expr::Arithmetic {
            op, left, right, ..
        } => {
            !matches!(op, ArithOp::Mul(flags) if flags.append_arrays || flags.only_existing
                || flags.only_new || flags.deep_merge_arrays || flags.clobber_tags)
                && supported(left)
                && supported(right)
        }
        Expr::Builtin(
            Builtin::Keys | Builtin::KeysUnsorted | Builtin::ToEntries | Builtin::FromEntries,
        ) => true,
        Expr::Builtin(
            Builtin::WithEntries(f) | Builtin::Map(f) | Builtin::MapValues(f) | Builtin::SortBy(f),
        ) => supported(f),
        Expr::Builtin(Builtin::Sort | Builtin::Flatten) => true,
        Expr::Builtin(Builtin::FlattenDepth(depth)) => is_closed_literal(depth),
        // Paths given as a literal; one built from the document is for the plain route.
        Expr::Builtin(Builtin::Pick(paths)) => is_closed_literal(paths),
        Expr::Assign { path, value } => static_path(path) && supported(value),
        Expr::Update { path, filter } => static_path(path) && supported(filter),
        _ => false,
    }
}

/// Whether `expr` replays a `pick`. The replay reads the presentation of the whole document, where
/// the plain route reads what is picked, so the caller bounds it by the document's size (#5003).
pub(super) fn contains_pick(expr: &Expr) -> bool {
    match expr {
        Expr::Builtin(Builtin::Pick(_)) => true,
        Expr::Paren(inner) => contains_pick(inner),
        Expr::Pipe(parts) => parts.iter().any(contains_pick),
        _ => false,
    }
}

/// yq prints a non-empty flow mapping read into a constructed mapping as a block once it is not
/// the first entry, and so every mapping nested in it by mappings (`{"a": .a, "c": .c}` over
/// `c: {k: {z: 1}, w: [1]}` is `c:` / `  k:` / `    z: 1` / `  w: [1]`): a sequence keeps its own
/// style, and what it holds. An empty `{}` stays flow (#865).
fn unflow_mappings(value: &OwnedValue, tree: &mut CommentTree) {
    let (OwnedValue::Object(fields), CommentTree::Object(meta, children, _)) = (value, tree) else {
        return;
    };
    if meta.style == "flow" && !fields.is_empty() {
        // The comment that trailed the closing brace has no place in a block mapping: yq drops it.
        *meta = meta.with_style("");
        meta.comment = None;
    }
    for (key, child) in children.iter_mut() {
        if let Some(field) = fields.get(key.as_str()) {
            unflow_mappings(field, child);
        }
    }
}

/// The standalone comment above a node belongs to the key that introduces it (or to the document),
/// not to the node, so a node taken into a construction leaves it behind; its foot stays (#865).
fn drop_head_comment(tree: &mut CommentTree) {
    let meta = tree.meta_mut();
    if !meta.head_comment().is_empty() {
        *meta = meta.with_head_foot(Vec::new(), meta.foot_comment().to_vec());
    }
}

/// The operands of `[...]`: the branches of a top-level comma, or the one expression.
fn array_operands(inner: &Expr) -> Vec<&Expr> {
    match inner {
        Expr::Comma(parts) => parts.iter().collect(),
        // `[(a, b)]` is `[a, b]` (#5429).
        Expr::Paren(inner) if matches!(**inner, Expr::Comma(_) | Expr::Paren(_)) => {
            array_operands(inner)
        }
        other => vec![other],
    }
}

fn static_path(expr: &Expr) -> bool {
    match expr {
        Expr::Identity | Expr::Field(_) | Expr::Index { .. } => true,
        Expr::Pipe(parts) => parts.iter().all(static_path),
        Expr::Paren(inner) => static_path(inner),
        _ => false,
    }
}

/// Whether `expr` is a construction whose every entry is a static read of the document, a literal,
/// the length of a static read, arithmetic on such entries or a construction of those (#865). Such
/// a construction is replayed from the nodes it names ([`trace_navigation_construction`]), not
/// from a presentation tree of the whole document: the cost is that of the nodes it reads.
pub(super) fn navigation_construction(expr: &Expr) -> bool {
    fn entry(expr: &Expr) -> bool {
        match expr {
            Expr::Paren(inner) => entry(inner),
            Expr::Arithmetic { left, right, .. } => entry(left) && entry(right),
            Expr::Pipe(parts) => match parts.split_last() {
                Some((Expr::Builtin(Builtin::Length), path)) => path.iter().all(static_path),
                _ => static_path(expr),
            },
            _ => is_closed_literal(expr) || static_path(expr) || navigation_construction(expr),
        }
    }
    match expr {
        // `.c | [.]`: the construction reads the node the path lands on.
        Expr::Pipe(parts) => match parts.split_last() {
            Some((last, path)) => {
                !path.is_empty() && path.iter().all(static_path) && navigation_construction(last)
            }
            None => false,
        },
        Expr::Paren(inner) => navigation_construction(inner),
        // `.a[1:]`: the elements of a sequence a slice keeps are its own nodes (#5429).
        Expr::Slice { .. } => true,
        // `[.a] + [.b]` (#5429): the sum of constructions, at least one of them a construction.
        Expr::Arithmetic { left, right, .. } => {
            (navigation_construction(left) || navigation_construction(right))
                && entry(left)
                && entry(right)
        }
        Expr::Object(entries) => {
            !is_closed_literal(expr)
                && entries
                    .iter()
                    .all(|e| matches!(e.key, ObjectKey::Literal(_)) && entry(&e.value))
        }
        Expr::Array(inner) => {
            !is_closed_literal(expr) && array_operands(inner).iter().all(|e| entry(e))
        }
        _ => false,
    }
}

/// What a static read lands on: the node, or `None` for a position the document does not hold.
struct Landed<C> {
    node: Option<C>,
    /// The last step was an index: the node is an element of a sequence, whose standalone comments
    /// are its own, where a mapping member's are its key's.
    element: bool,
}

/// Follow the static path `expr` from `from`. `None` is a path this does not read, or a step
/// that is not a node of the shape it expects (the plain route decides those).
fn land<C: DocumentCursor>(expr: &Expr, from: Landed<C>) -> Option<Landed<C>> {
    match expr {
        Expr::Identity => Some(from),
        Expr::Paren(inner) => land(inner, from),
        Expr::Pipe(parts) => parts.iter().try_fold(from, |at, part| land(part, at)),
        Expr::Field(name) => {
            let Some(node) = from.node else {
                return Some(Landed {
                    node: None,
                    element: false,
                });
            };
            let found = node.value().as_object()?.find_cursor(name).ok()?;
            Some(Landed {
                node: found,
                element: false,
            })
        }
        Expr::Index { idx, .. } => {
            let Some(node) = from.node else {
                return Some(Landed {
                    node: None,
                    element: false,
                });
            };
            let elements = node.value().as_array()?;
            let len = i64::try_from(elements.len()).ok()?;
            let at = if *idx < 0 { len + *idx } else { *idx };
            let found = usize::try_from(at)
                .ok()
                .filter(|i| i64::try_from(*i).is_ok_and(|i| i < len))
                .and_then(|i| elements.get_cursor(i));
            Some(Landed {
                node: found,
                element: true,
            })
        }
        _ => None,
    }
}

/// The value and presentation of one landed read: an absent position is `null` with none, as yq
/// reads it.
fn landed_pair<C: DocumentCursor>(at: &Landed<C>) -> Option<ResultWithComments> {
    let Some(node) = &at.node else {
        return Some((OwnedValue::Null, CommentTree::empty()));
    };
    node_pair_as(node, at.element).ok()
}

/// A document node taken into a constructed sequence (#5429): its value and the presentation it
/// carries there, as [`landed_pair`] reads a node a construction names. `element` is whether the
/// node is an element of a sequence, whose standalone comments are its own, where a mapping
/// member's are its key's.
pub(super) fn node_pair<C: DocumentCursor>(node: &C) -> Result<ResultWithComments, EvalError> {
    let element = node
        .document_parent()
        .is_some_and(|parent| parent.value().as_array().is_some());
    node_pair_as(node, element)
}

fn node_pair_as<C: DocumentCursor>(
    node: &C,
    element: bool,
) -> Result<ResultWithComments, EvalError> {
    let (value, mut tree) = to_owned_with_comments::<_, YqSemantics>(&node.value(), Some(node))?;
    // A mapping member's standalone comments live on its key, which a construction does not carry:
    // `d:` / `  x: 2` / `# f` read as `[.d]` leaves `# f` (the key's foot) behind, while a comment
    // inside the container (`  # f`) is a foot of one of its own keys and rides in the tree (#865).
    if !element {
        drop_head_comment(&mut tree);
    } else if node.document_has_standalone_comments() {
        // A sequence element keeps the comment above it, which the walk of a lone scalar clears.
        let head = node.head_comment_raw();
        if !head.is_empty() {
            *tree.meta_mut() = tree
                .meta()
                .with_head_foot(head, tree.meta().foot_comment().to_vec());
        }
    }
    Ok((value, tree))
}

/// The sequence a `LazySeq` stands for, each element with the presentation of the node it still
/// is (#5429); a value an earlier stage computed has none.
pub(super) fn sequence_of_nodes<C: DocumentCursor>(
    items: Vec<LazyElem<impl DocumentValue<Cursor = C>>>,
) -> Result<ResultWithComments, EvalError> {
    let mut values = Vec::with_capacity(items.len());
    let mut trees = Vec::with_capacity(items.len());
    for item in items {
        let (value, tree) = match item {
            LazyElem::Cursor(node) => node_pair(&node)?,
            LazyElem::Owned(value) => (value, CommentTree::empty()),
        };
        values.push(value);
        trees.push(tree);
    }
    Ok((
        OwnedValue::array_from(values),
        CommentTree::Array(NodeMeta::empty(), trees),
    ))
}

/// Replay a construction for which [`navigation_construction`] holds, from the nodes it reads
/// (#865): the value and the presentation of each entry, as [`trace`] builds them from a tree of
/// the whole document, at the cost of the nodes the construction names.
pub(super) fn trace_navigation_construction<C: DocumentCursor>(
    expr: &Expr,
    root: &C,
) -> Option<ResultWithComments> {
    match expr {
        Expr::Paren(inner) => trace_navigation_construction(inner, root),
        Expr::Pipe(parts) => {
            let (last, path) = parts.split_last()?;
            let at = land(
                &Expr::Pipe(path.to_vec().into()),
                Landed {
                    node: Some(*root),
                    element: false,
                },
            )?;
            // An absent node is for the plain route to answer.
            construct(last, &at.node?, 0)
        }
        Expr::Arithmetic { .. } => entry_pair(expr, root, 0),
        _ => construct(expr, root, 0),
    }
}

/// One entry of a navigation construction, with the presentation it brings.
fn entry_pair<C: DocumentCursor>(e: &Expr, root: &C, depth: usize) -> Option<ResultWithComments> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    let start = || Landed {
        node: Some(*root),
        element: false,
    };
    match e {
        Expr::Paren(inner) => entry_pair(inner, root, depth + 1),
        Expr::Arithmetic {
            op, left, right, ..
        } => {
            let l = entry_pair(left, root, depth + 1)?;
            let r = entry_pair(right, root, depth + 1)?;
            let sum = Expr::arithmetic(
                *op,
                Expr::tracked_value(l.0.clone()),
                Expr::tracked_value(r.0.clone()),
            );
            let result = evaluated(&sum, &OwnedValue::Null)?;
            let tree = if matches!(op, ArithOp::Add | ArithOp::Mul(_)) && l.0 == OwnedValue::Null {
                // `null + x` is `x`, node and all.
                r.1.clone()
            } else if matches!(op, ArithOp::Add | ArithOp::Mul(_)) {
                merge_tree(&l, &r, &result, matches!(op, ArithOp::Mul(_)), depth + 1)?
            } else if matches!(result, OwnedValue::Array(_) | OwnedValue::Object(_)) {
                CommentTree::empty()
            } else {
                // A number computed from a node keeps the node's comment and style.
                CommentTree::Leaf(l.1.meta().clone())
            };
            Some((result, tree))
        }
        Expr::Pipe(parts) if matches!(parts.last(), Some(Expr::Builtin(Builtin::Length))) => {
            let path = Expr::Pipe(parts[..parts.len() - 1].to_vec().into());
            let at = land(&path, start())?;
            let length = match &at.node {
                None => 0,
                Some(node) => node_length(node)?,
            };
            Some((OwnedValue::Int(length), CommentTree::empty()))
        }
        _ if is_closed_literal(e) => Some((evaluated(e, &OwnedValue::Null)?, CommentTree::empty())),
        _ if static_path(e) => landed_pair(&land(e, start())?),
        _ => construct(e, root, depth + 1),
    }
}

/// `length` of a node, counted from the node without building it; `None` for a kind this does not
/// count (the plain route decides those).
fn node_length<C: DocumentCursor>(node: &C) -> Option<i64> {
    let value = node.value();
    if let Some(elements) = value.as_array() {
        return i64::try_from(elements.len()).ok();
    }
    // The entries the mapping spells, a merge key's included, as yq's `length` counts them.
    if let Some(fields) = value.as_object_listed() {
        return i64::try_from(fields.len()).ok();
    }
    let text = value.as_str()?;
    i64::try_from(text.chars().count()).ok()
}

fn construct<C: DocumentCursor>(expr: &Expr, root: &C, depth: usize) -> Option<ResultWithComments> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    let entry = |e: &Expr| entry_pair(e, root, depth + 1);
    match expr {
        Expr::Object(entries) => {
            let mut values = IndexMap::new();
            let mut trees = IndexMap::new();
            for (position, e) in entries.iter().enumerate() {
                let ObjectKey::Literal(key) = &e.key else {
                    return None;
                };
                if values.contains_key(key.as_str()) {
                    return None;
                }
                let (v, mut t) = entry(&e.value)?;
                // What stood above a sequence element is printed below the entry it lands in.
                if !t.meta().head_comment().is_empty() {
                    let mut foot = t.meta().head_comment().to_vec();
                    foot.extend(t.meta().foot_comment().iter().cloned());
                    *t.meta_mut() = t.meta().with_head_foot(Vec::new(), foot);
                }
                if position > 0 {
                    unflow_mappings(&v, &mut t);
                }
                values.insert(key.clone(), v);
                trees.insert(key.clone(), t);
            }
            Some((
                OwnedValue::Object(values.into()),
                CommentTree::Object(NodeMeta::empty(), trees, IndexMap::new()),
            ))
        }
        Expr::Slice { start, end, .. } => {
            let elements = root.value().as_array()?;
            let range = succinctly::jq::literal_slice_range(*start, *end, elements.len());
            let mut values = Vec::with_capacity(range.len());
            let mut trees = Vec::with_capacity(range.len());
            for at in range {
                let (v, t) = node_pair_as(&elements.get_cursor(at)?, true).ok()?;
                values.push(v);
                trees.push(t);
            }
            Some((
                OwnedValue::Array(values.into_iter().collect()),
                CommentTree::Array(NodeMeta::empty(), trees),
            ))
        }
        Expr::Array(inner) => {
            let mut values = Vec::new();
            let mut trees = Vec::new();
            for operand in array_operands(inner) {
                let (v, t) = entry(operand)?;
                values.push(v);
                trees.push(t);
            }
            Some((
                OwnedValue::Array(values.into_iter().collect()),
                CommentTree::Array(NodeMeta::empty(), trees),
            ))
        }
        _ => None,
    }
}

/// Skip ordinary navigation and writes, whose existing paths already carry
/// metadata. A supported full expression must contain an actual reshape.
pub(super) fn needed(expr: &Expr) -> bool {
    if !supported(expr) {
        return false;
    }
    match expr {
        Expr::Builtin(_) | Expr::Arithmetic { .. } => true,
        Expr::Pipe(parts) => parts.iter().any(needed),
        Expr::Paren(inner) => needed(inner),
        _ => false,
    }
}

fn evaluated(expr: &Expr, value: &OwnedValue) -> Option<OwnedValue> {
    use succinctly::jq::{self, JqSemantics, QueryResult, YqSemantics};
    // Never report speculative failures: the authoritative evaluation owns
    // diagnostics. In particular, a bad merge must produce one error, not two.
    let doc = std::rc::Rc::new(value.reindexed_without_provenance::<JqSemantics>().ok()?);
    match jq::eval_reindexed_document::<YqSemantics>(expr, &doc) {
        QueryResult::Owned(v) => Some(v),
        QueryResult::One(v) => jq::eval_generic::to_owned::<YqSemantics, _>(&v).ok(),
        QueryResult::OneCursor(c) => jq::eval_generic::to_owned_cursor::<YqSemantics, _>(&c).ok(),
        _ => None,
    }
}

pub(super) fn trace(expr: &Expr, input: &ResultWithComments) -> Option<ResultWithComments> {
    trace_at_depth(expr, input, 0)
}

fn trace_at_depth(
    expr: &Expr,
    input: &ResultWithComments,
    depth: usize,
) -> Option<ResultWithComments> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    let (value, tree) = input;
    let next = |e: &Expr, pair: &ResultWithComments| trace_at_depth(e, pair, depth + 1);
    match expr {
        Expr::Identity => Some(input.clone()),
        Expr::Paren(inner) => next(inner, input),
        Expr::Field(key) => Some((evaluated(expr, value)?, tree.field(key).clone())),
        Expr::Index { idx, .. } => {
            let OwnedValue::Array(items) = value else {
                return None;
            };
            let index = if *idx < 0 {
                items.len() as i64 + idx
            } else {
                *idx
            };
            let meta = usize::try_from(index)
                .ok()
                .map_or_else(CommentTree::empty, |i| tree.at_index(i).clone());
            Some((evaluated(expr, value)?, meta))
        }
        Expr::Pipe(parts) => {
            let mut pair = input.clone();
            for part in parts {
                pair = next(part, &pair)?;
            }
            Some(pair)
        }
        Expr::Builtin(Builtin::Keys | Builtin::KeysUnsorted | Builtin::ToEntries) => {
            let result = evaluated(expr, value)?;
            let OwnedValue::Array(outputs) = &result else {
                return None;
            };
            let mut children = Vec::new();
            for (i, output) in outputs.iter().enumerate() {
                let is_entries = matches!(expr, Expr::Builtin(Builtin::ToEntries));
                let key = if is_entries {
                    let OwnedValue::Object(entry) = output else {
                        return None;
                    };
                    entry.get("key")?
                } else {
                    output
                };
                let (key_tree, value_tree) = match value {
                    OwnedValue::Object(_) => {
                        let OwnedValue::String(key) = key else {
                            return None;
                        };
                        // The key node is reused as the entry's `key` string, so
                        // its `&anchor` rides along (`key: &k key`, #2598).
                        let mut key_meta =
                            NodeMeta::from_comment_and_style(None, tree.key_style(key));
                        key_meta.anchor = tree
                            .key_anchor(key)
                            .map(|name| AnchorMark::Declares(name.to_string()));
                        (CommentTree::Leaf(key_meta), tree.field(key).clone())
                    }
                    OwnedValue::Array(_) => (CommentTree::empty(), tree.at_index(i).clone()),
                    _ => return None,
                };
                children.push(if is_entries {
                    CommentTree::Object(
                        NodeMeta::empty(),
                        IndexMap::from([
                            ("key".to_string(), key_tree),
                            ("value".to_string(), value_tree),
                        ]),
                        IndexMap::new(),
                    )
                } else {
                    key_tree
                });
            }
            Some((result, CommentTree::Array(NodeMeta::empty(), children)))
        }
        Expr::Builtin(Builtin::FromEntries) => {
            let result = evaluated(expr, value)?;
            let OwnedValue::Array(entries) = value else {
                return None;
            };
            let mut fields = IndexMap::new();
            let mut keys = IndexMap::new();
            for (i, entry) in entries.iter().enumerate() {
                let OwnedValue::Object(entry) = entry else {
                    return None;
                };
                // yq reads exactly a `key` and a `value` (#4399): `evaluated` above already
                // refused any entry that lacks either.
                let key_field = "key";
                let OwnedValue::String(key) = entry.get(key_field)? else {
                    return None;
                };
                let value_field = "value";
                let entry_tree = tree.at_index(i);
                fields.insert(key.to_string(), entry_tree.field(value_field).clone());
                // Last occurrence wins, including an unstyled key. Never
                // leave the earlier occurrence's style behind on collision.
                keys.shift_remove(key.as_ref());
                let key_tree = entry_tree.field(key_field);
                if let Some(meta) = KeyMeta::with_anchor(
                    None,
                    false,
                    key_tree.style(),
                    key_tree.declared_anchor().map(str::to_string),
                ) {
                    keys.insert(key.to_string(), meta);
                }
            }
            Some((result, CommentTree::Object(NodeMeta::empty(), fields, keys)))
        }
        // #4386: a sorted mapping is the same entries in another order, each keeping the comments,
        // style, anchor and tag of its own node and its key. A sorted array already keeps its
        // items' (the value alignment of #870), so only a mapping is replayed here.
        Expr::Builtin(Builtin::Sort | Builtin::SortBy(_)) => {
            let OwnedValue::Object(_) = value else {
                return None;
            };
            let result = evaluated(expr, value)?;
            let OwnedValue::Object(sorted) = &result else {
                return None;
            };
            let mut fields = IndexMap::new();
            let mut keys = IndexMap::new();
            for key in sorted.keys() {
                fields.insert(key.clone(), tree.field(key).clone());
                if let Some(meta) = KeyMeta::with_properties(
                    None,
                    false,
                    tree.key_style(key),
                    tree.key_anchor(key).map(str::to_string),
                    tree.key_tag(key).map(str::to_string),
                ) {
                    keys.insert(key.clone(), meta);
                }
            }
            Some((
                result,
                CommentTree::Object(tree.meta().clone(), fields, keys),
            ))
        }
        // #5003: a picked entry is the document's own node, key and all: its comments, style,
        // anchor and tag come with it, as do the document's head comment and the key's.
        Expr::Builtin(Builtin::Pick(keys)) => pick_pair(keys, input),
        Expr::Builtin(Builtin::WithEntries(f)) => {
            let entries = next(&Expr::Builtin(Builtin::ToEntries), input)?;
            let mapped = next(&Expr::Builtin(Builtin::Map(f.clone())), &entries)?;
            next(&Expr::Builtin(Builtin::FromEntries), &mapped)
        }
        // `map_values(f)` keeps each key, its comments and its position (#5429); on a sequence it
        // is `map(f)`.
        Expr::Builtin(Builtin::MapValues(f)) => match value {
            OwnedValue::Object(members) => {
                let mut values = IndexMap::new();
                let mut fields = IndexMap::new();
                let mut keys = IndexMap::new();
                for (key, item) in members {
                    let (v, t) = next(f, &(item.clone(), tree.field(key).clone()))?;
                    values.insert(key.clone(), v);
                    fields.insert(key.clone(), t);
                    if let Some(meta) = tree.key_meta(key) {
                        keys.insert(key.clone(), meta.clone());
                    }
                }
                Some((
                    OwnedValue::Object(values.into()),
                    CommentTree::Object(tree.meta().clone(), fields, keys),
                ))
            }
            _ => next(&Expr::Builtin(Builtin::Map(f.clone())), input),
        },
        // `flatten` keeps every leaf's own node (#5429); the result is checked against the
        // authoritative one, so a depth this does not read the same way is the plain route's.
        Expr::Builtin(Builtin::Flatten | Builtin::FlattenDepth(_)) => {
            let depth = match expr {
                Expr::Builtin(Builtin::FlattenDepth(depth)) => {
                    match evaluated(depth, &OwnedValue::Null)? {
                        OwnedValue::Int(n) | OwnedValue::NumberLiteral(NumberRepr::Int(n), _) => {
                            usize::try_from(n).ok()?
                        }
                        _ => return None,
                    }
                }
                _ => 256,
            };
            let result = evaluated(expr, value)?;
            let mut trees = Vec::new();
            flatten_trees(value, tree, depth, &mut trees)?;
            let OwnedValue::Array(flat) = &result else {
                return None;
            };
            // A trailing comment of the sequence is not carried: yq drops it, as it does the
            // comment above a sequence that `flatten` dissolves.
            let meta = tree
                .meta()
                .with_head_foot(tree.meta().head_comment().to_vec(), Vec::new());
            (flat.len() == trees.len()).then(|| (result.clone(), CommentTree::Array(meta, trees)))
        }
        Expr::Builtin(Builtin::Map(f)) => {
            let OwnedValue::Array(items) = value else {
                return None;
            };
            let mut values = Vec::new();
            let mut trees = Vec::new();
            for (i, item) in items.iter().enumerate() {
                let (v, t) = next(f, &(item.clone(), tree.at_index(i).clone()))?;
                values.push(v);
                trees.push(t);
            }
            Some((
                OwnedValue::Array(values.into_iter().collect()),
                CommentTree::Array(tree.meta().clone(), trees),
            ))
        }
        Expr::Arithmetic {
            op, left, right, ..
        } => {
            let l = next(left, input)?;
            let r = next(right, input)?;
            let result = evaluated(expr, value)?;
            let meta = if matches!(op, ArithOp::Add | ArithOp::Mul(_)) {
                merge_tree(&l, &r, &result, matches!(op, ArithOp::Mul(_)), depth + 1)?
            } else {
                CommentTree::empty()
            };
            Some((result, meta))
        }
        Expr::Assign { .. } | Expr::Update { .. } => {
            let result = evaluated(expr, value)?;
            let targets = collect_write_targets(expr)?;
            let meta = reconcile_presentation(value, tree, &result, &targets);
            Some((result, meta))
        }
        _ if is_closed_literal(expr) => Some((evaluated(expr, value)?, CommentTree::empty())),
        _ => None,
    }
}

/// The trees of the elements `flatten` keeps, in order: those of the nested sequences' own
/// elements down to `depth` levels, and of every other element as it stands (#5429).
fn flatten_trees(
    value: &OwnedValue,
    tree: &CommentTree,
    depth: usize,
    out: &mut Vec<CommentTree>,
) -> Option<()> {
    let OwnedValue::Array(items) = value else {
        return None;
    };
    for (i, item) in items.iter().enumerate() {
        match item {
            OwnedValue::Array(_) if depth > 0 => {
                flatten_trees(item, tree.at_index(i), depth - 1, out)?;
            }
            _ => out.push(tree.at_index(i).clone()),
        }
    }
    Some(())
}

/// `pick(keys)` replayed from the document's own tree (#5003): the keys it lists that the mapping
/// holds, or the elements at the indices it lists, in the listed order, each with the presentation
/// of its own node and, for a mapping, of its own key. The value is built here too, so the caller's
/// comparison with the authoritative result decides whether the tree is used (a repeated key, which
/// yq prints twice and a mapping cannot hold, is the plain route's).
fn pick_pair(keys: &Expr, input: &ResultWithComments) -> Option<ResultWithComments> {
    let (value, tree) = input;
    let OwnedValue::Array(keys) = evaluated(keys, &OwnedValue::Null)? else {
        return None;
    };
    match value {
        OwnedValue::Object(members) => {
            let mut picked = IndexMap::new();
            let mut fields = IndexMap::new();
            let mut key_metas = IndexMap::new();
            for key in &keys {
                let OwnedValue::String(key) = key else {
                    return None;
                };
                let Some(member) = members.get(key.as_ref()) else {
                    continue;
                };
                picked.insert(key.to_string(), member.clone());
                fields.insert(key.to_string(), tree.field(key).clone());
                if let Some(meta) = tree.key_meta(key) {
                    key_metas.insert(key.to_string(), meta.clone());
                }
            }
            Some((
                OwnedValue::Object(picked.into()),
                CommentTree::Object(tree.meta().clone(), fields, key_metas),
            ))
        }
        OwnedValue::Array(items) => {
            let mut picked = Vec::new();
            let mut children = Vec::new();
            for index in &keys {
                let (OwnedValue::Int(index) | OwnedValue::NumberLiteral(NumberRepr::Int(index), _)) =
                    index
                else {
                    return None;
                };
                let index = usize::try_from(*index).ok()?;
                let Some(item) = items.get(index) else {
                    continue;
                };
                picked.push(item.clone());
                children.push(tree.at_index(index).clone());
            }
            Some((
                OwnedValue::Array(picked.into_iter().collect()),
                CommentTree::Array(tree.meta().clone(), children),
            ))
        }
        _ => None,
    }
}

fn merge_tree(
    left: &ResultWithComments,
    right: &ResultWithComments,
    result: &OwnedValue,
    deep: bool,
    depth: usize,
) -> Option<CommentTree> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    let (lv, lt) = left;
    let (rv, rt) = right;
    if let (OwnedValue::Array(l), OwnedValue::Array(r)) = (lv, rv) {
        if !deep {
            let children = (0..l.len())
                .map(|i| lt.at_index(i).clone())
                .chain((0..r.len()).map(|i| rt.at_index(i).clone()))
                .collect();
            return Some(CommentTree::Array(lt.meta().clone(), children));
        }
    }
    if let (OwnedValue::Object(l), OwnedValue::Object(r), OwnedValue::Object(out)) =
        (lv, rv, result)
    {
        let mut fields = IndexMap::new();
        let mut keys = IndexMap::new();
        for (key, value) in out {
            let lvalue = l.get(key);
            let rvalue = r.get(key);
            let child = match (lvalue, rvalue) {
                (Some(lvalue), Some(rvalue)) if deep => merge_tree(
                    &(lvalue.clone(), lt.field(key).clone()),
                    &(rvalue.clone(), rt.field(key).clone()),
                    value,
                    true,
                    depth + 1,
                )?,
                (_, Some(_)) => rt.field(key).clone(),
                (Some(_), None) => lt.field(key).clone(),
                _ => return None,
            };
            fields.insert(key.clone(), child);
            let key_tree = if lvalue.is_some() { lt } else { rt };
            if let Some(meta) = KeyMeta::with_properties(
                None,
                false,
                key_tree.key_style(key),
                key_tree.key_anchor(key).map(str::to_string),
                key_tree.key_tag(key).map(str::to_string),
            ) {
                keys.insert(key.clone(), meta);
            }
        }
        Some(CommentTree::Object(lt.meta().clone(), fields, keys))
    } else if !matches!(lv, OwnedValue::Array(_) | OwnedValue::Object(_))
        && !matches!(rv, OwnedValue::Array(_) | OwnedValue::Object(_))
    {
        Some(CommentTree::Leaf(lt.meta().clone()))
    } else {
        // Replaced arrays take the RHS children, unlike recursive mapping
        // merges. Kind changes likewise have no left child provenance.
        Some(rt.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reshape_presentation_rejects_effects_and_context_reads() {
        for filter in [
            "with_entries(.value = env(HOME))",
            "with_entries(.value = key)",
            "with_entries(.value = now)",
            "to_entries | map(.value = env(HOME))",
            "to_entries | map(error(\"stop\"))",
            ".a *+ .b",
        ] {
            let expr =
                succinctly::jq::parse_with_mode(filter, succinctly::jq::ParserMode::Yq).unwrap();
            assert!(!needed(&expr), "must not replay {filter}");
        }
    }

    #[test]
    fn reshape_presentation_depth_limit_falls_back() {
        let input = (OwnedValue::Null, CommentTree::empty());
        assert!(trace_at_depth(&Expr::Identity, &input, MAX_VALUE_TREE_DEPTH).is_none());
    }

    #[test]
    fn reshape_presentation_last_entry_clears_previous_key_style() {
        let entry = |v| {
            OwnedValue::Object(
                IndexMap::from([
                    ("key".to_string(), OwnedValue::string("a")),
                    ("value".to_string(), OwnedValue::Int(v)),
                ])
                .into(),
            )
        };
        let styled = CommentTree::Object(
            NodeMeta::empty(),
            IndexMap::from([(
                "key".to_string(),
                CommentTree::Leaf(NodeMeta::from_comment_and_style(None, "single")),
            )]),
            IndexMap::new(),
        );
        let input = (
            OwnedValue::Array([entry(1), entry(2)].into_iter().collect()),
            CommentTree::Array(NodeMeta::empty(), vec![styled, CommentTree::empty()]),
        );
        let (value, tree) = trace(&Expr::Builtin(Builtin::FromEntries), &input).unwrap();
        assert_eq!(
            value,
            OwnedValue::Object(IndexMap::from([("a".to_string(), OwnedValue::Int(2))]).into())
        );
        assert_eq!(tree.key_style("a"), "");
    }
}
