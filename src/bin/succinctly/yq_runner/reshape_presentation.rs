//! Bounded provenance for inexpensive YAML reshaping filters (#3615).
//!
//! This is a presentation pass, not an alternative evaluator. Only pure,
//! single-output shapes are replayed, and the caller checks the resulting
//! value against the authoritative generic result before using its tree.

use super::{
    collect_write_targets, is_closed_literal, reconcile_presentation, CommentTree, Expr, IndexMap,
    KeyCounts, NodeMeta, NumberRepr, OwnedValue, ResultWithComments,
};
use succinctly::jq::document::{
    DistinctKeyCursors, DocumentCursor, DocumentElements, DocumentFields, DocumentValue,
};
use succinctly::jq::eval_generic::{
    to_owned_with_comments, AnchorMark, KeyMeta, LazyElem, KEY_STYLE_STRING,
};
use succinctly::jq::{
    ArithOp, Builtin, EvalError, MergeFlags, MetaSlot, ObjectKey, MAX_VALUE_TREE_DEPTH,
};
use succinctly::jq::{EvalSemantics, YqSemantics};

/// Whether yq's key listing folds a repeated key into one (it does not: both are listed).
const COLLAPSE: bool = YqSemantics::COLLAPSE_DUPLICATE_KEYS;

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
        } => plain_op(op) && supported(left) && supported(right),
        Expr::Builtin(
            Builtin::Keys | Builtin::KeysUnsorted | Builtin::ToEntries | Builtin::FromEntries,
        ) => true,
        Expr::Builtin(Builtin::WithEntries(f) | Builtin::Map(f)) => {
            supported(f) || select_condition(f).is_some()
        }
        Expr::Builtin(Builtin::MapValues(f)) => supported(f),
        Expr::Builtin(Builtin::SortBy(f)) => supported(f) || sort_key_filter(f),
        Expr::Builtin(Builtin::Sort | Builtin::Flatten | Builtin::Pivot) => true,
        Expr::Builtin(Builtin::FlattenDepth(depth)) => is_closed_literal(depth),
        // Paths given as a literal; one built from the document is for the plain route.
        Expr::Builtin(Builtin::Pick(paths) | Builtin::Omit(paths)) => is_closed_literal(paths),
        Expr::Assign { path, value } => static_path(path) && supported(value),
        // #6089: a key rename, whose target is a path and whose new name is a supported expression.
        Expr::MetaAssign {
            target,
            slot: MetaSlot::KeyName,
            value,
            ..
        } => {
            target
                .key_node_prefix()
                .is_some_and(|prefix| rename_target(&prefix))
                && supported(value)
        }
        Expr::Update { path, filter } => static_path(path) && supported(filter),
        _ => false,
    }
}

/// Put the head comment of `original` (a mapping's tree) on `rebuilt`, the mapping a rewrite of it
/// produced, keeping the mark of being the header read before the first document (#6674).
fn restore_head(rebuilt: &mut CommentTree, original: &CommentTree) {
    *rebuilt.meta_mut() = rebuilt.meta().with_head_foot_of(
        original.meta().head_comment().to_vec(),
        Vec::new(),
        original.meta(),
    );
}

/// The condition of a `select(c)` filter whose `c` is a pure test of the item, or `None` for any
/// other filter (#6674). `map(select(c))` and `with_entries(select(c))` keep each item that passes
/// as it is, so its comments, spelling and style come with it; the condition only decides which.
fn select_condition(f: &Expr) -> Option<&Expr> {
    match f {
        Expr::Builtin(Builtin::Select(cond)) if pure_condition(cond) => Some(cond),
        Expr::Paren(inner) => select_condition(inner),
        _ => None,
    }
}

/// A test the replay may run on an item: field and index reads, closed literals, comparisons and
/// `and`/`or`/`not` over them. Nothing here reads a variable, the environment or the clock, so
/// running it on a copy of the item has no effect beyond its answer.
fn pure_condition(expr: &Expr) -> bool {
    match expr {
        Expr::And(l, r) | Expr::Or(l, r) => pure_condition(l) && pure_condition(r),
        Expr::Not => true,
        Expr::Compare { left, right, .. } => pure_condition(left) && pure_condition(right),
        Expr::Paren(inner) => pure_condition(inner),
        Expr::Pipe(parts) => parts.iter().all(pure_condition),
        // Reads of the item and closed literals; a write or a rebuilding builtin is not a test.
        Expr::Identity | Expr::Field(_) | Expr::Index { .. } => true,
        _ => is_closed_literal(expr),
    }
}

/// A `sort_by` filter that reads the entry's own `key` (#6099): `key`, `key | f`, a comma of those
/// and of supported filters. A `key` below another step is not one, and stays unsupported.
fn sort_key_filter(f: &Expr) -> bool {
    match f {
        Expr::Builtin(Builtin::Key) => true,
        Expr::Paren(inner) => sort_key_filter(inner),
        Expr::Pipe(parts) => match parts.split_first() {
            Some((first, rest)) => {
                sort_key_filter(first)
                    && rest.iter().all(|stage| {
                        supported(stage)
                            || matches!(
                                stage,
                                Expr::Builtin(Builtin::Length | Builtin::Tag | Builtin::Type)
                            )
                    })
            }
            None => false,
        },
        Expr::Comma(parts) => parts.iter().all(|p| supported(p) || sort_key_filter(p)),
        _ => false,
    }
}

/// Whether `op` is one whose result tree is built by [`merge_tree`]: `+`, a plain `*`, and a `*`
/// with exactly one of the merge flags `+`, `?`, `n` or `d` (#6900), whose rules it follows. A
/// combination of flags and `c` (clobber custom tags) are not, and keep the route that carries no
/// presentation.
fn plain_op(op: &ArithOp) -> bool {
    match op {
        ArithOp::Mul(flags) => {
            !flags.clobber_tags
                && [
                    flags.append_arrays,
                    flags.only_existing,
                    flags.only_new,
                    flags.deep_merge_arrays,
                ]
                .into_iter()
                .filter(|set| *set)
                .count()
                    <= 1
        }
        _ => true,
    }
}

/// The merge flags of `op`: those of a `*`, none for anything else.
fn merge_flags(op: &ArithOp) -> MergeFlags {
    match op {
        ArithOp::Mul(flags) => *flags,
        _ => MergeFlags::default(),
    }
}

/// Whether `expr` replays a `pick` or an `omit`. The replay reads the presentation of the whole
/// document, where the plain route reads what is kept, so the caller bounds it by the document's
/// size (#5003, #6906).
pub(super) fn contains_pick(expr: &Expr) -> bool {
    match expr {
        Expr::Builtin(Builtin::Pick(_) | Builtin::Omit(_)) => true,
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
        let mut unflowed = meta.with_style("");
        unflowed.comment = None;
        meta.set(unflowed);
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
    if !tree.meta().head_comment().is_empty() {
        let kept = tree
            .meta()
            .with_head_foot(Vec::new(), tree.meta().foot_comment().to_vec());
        tree.set_meta(kept);
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

/// The member a key rename names (#6089): a path of fields, indexes and iterations. The tree is
/// read off what the write changed, not off this path, so it may fan out where a write's own target
/// may not.
fn rename_target(expr: &Expr) -> bool {
    match expr {
        Expr::Identity | Expr::Field(_) | Expr::Index { .. } | Expr::Iterate => true,
        Expr::Pipe(parts) => parts.iter().all(rename_target),
        Expr::Paren(inner) => rename_target(inner),
        _ => false,
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
                Some((Expr::Builtin(Builtin::Length | Builtin::KeysUnsorted), path)) => {
                    path.iter().all(static_path)
                }
                _ => static_path(expr),
            },
            // `[keys]`: the keys of the node the construction reads (#5469).
            Expr::Builtin(Builtin::KeysUnsorted) => true,
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

/// The presentation of a `keys` result (#5469): one leaf per key, each the key node's own, so
/// `&k key`, `!!str t`, a quoted `"1"` and an alias key `*a` print as yq prints them. A plain
/// `<<` takes the `!!merge` tag yq gives a merge key. `None` if a key fails to convert; the
/// caller then prints the keys bare, as it did before.
pub(super) fn key_nodes<F: DocumentFields>(fields: &F, collapse: bool) -> Option<CommentTree> {
    let (_, trees) = key_list(fields, collapse)?;
    Some(CommentTree::array(NodeMeta::empty(), trees))
}

/// A `keys` result built from the mapping's key nodes, value and presentation: what
/// [`key_nodes`] gives the result, with each key's value alongside it. `None` if a key node
/// fails to convert (the plain route decides those).
fn keys_pair<F: DocumentFields>(fields: &F, collapse: bool) -> Option<ResultWithComments> {
    let (values, trees) = key_list(fields, collapse)?;
    Some((
        OwnedValue::Array(values.into_iter().collect()),
        CommentTree::array(NodeMeta::empty(), trees),
    ))
}

/// `keys` of a document node: a mapping's key nodes ([`keys_pair`]), or a sequence's indices,
/// which carry no presentation.
fn node_keys_pair<C: DocumentCursor>(node: &C) -> Option<ResultWithComments> {
    let value = node.value();
    if let Some(fields) = value.as_object_listed() {
        return keys_pair(&fields, COLLAPSE);
    }
    let len = value.as_array()?.len();
    Some((
        OwnedValue::Array((0..len).map(|i| OwnedValue::Int(i as i64)).collect()),
        CommentTree::array(NodeMeta::empty(), vec![CommentTree::empty(); len]),
    ))
}

fn key_list<F: DocumentFields>(
    fields: &F,
    collapse: bool,
) -> Option<(Vec<OwnedValue>, Vec<CommentTree>)> {
    let mut values = Vec::new();
    let mut trees = Vec::new();
    for (key, cursor) in DistinctKeyCursors::new(fields, collapse) {
        let (key_value, tree) = node_pair_as(&cursor, false).ok()?;
        let text = key.as_str();
        let merge = tree.meta().tag().is_none()
            && !cursor.canonicalize_numbers()
            && cursor.style().is_empty()
            && cursor.alias().is_none()
            && text.as_deref() == Some("<<");
        let mut meta = tree.meta().clone();
        if merge {
            meta = meta.with_tag(Some("!!merge".to_string()));
        }
        // yq drops the non-specific `!` (`! z` is `- z`).
        if meta.tag() == Some("!") {
            meta = meta.with_tag(None);
        }
        // A plain key that decodes to a line break (`? !!str a` / `` / `  b`) cannot be written
        // as it was spelled, so the writer quotes it instead.
        if text
            .as_deref()
            .is_some_and(|t| t.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}']))
        {
            meta = meta.with_spelling(None);
        }
        // The key's node value: a key that is not a string (`1:`, `true:`) lists as its scalar,
        // as yq lists it. The caller checks the result against the evaluated one.
        values.push(key_value);
        trees.push(CommentTree::leaf(meta));
    }
    Some((values, trees))
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
        CommentTree::array(NodeMeta::empty(), trees),
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
            // `.c | keys + [.b]`: an entry of any kind, read from the node.
            entry_pair(last, &at.node?, 0)
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
        } if plain_op(op) => {
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
                merge_tree(
                    &l,
                    &r,
                    &result,
                    matches!(op, ArithOp::Mul(_)),
                    merge_flags(op),
                    depth + 1,
                )?
            } else if matches!(result, OwnedValue::Array(_) | OwnedValue::Object(_)) {
                CommentTree::empty()
            } else {
                // A number computed from a node keeps the node's comment and style.
                CommentTree::leaf(l.1.meta().clone())
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
        // `[.c | keys]`, `{x: (.c | keys)}`: each key is its node, as a bare `.c | keys` prints
        // it (#5469). An absent node and a scalar are the plain route's.
        Expr::Pipe(parts) if matches!(parts.last(), Some(Expr::Builtin(Builtin::KeysUnsorted))) => {
            let path = Expr::Pipe(parts[..parts.len() - 1].to_vec().into());
            node_keys_pair(&land(&path, start())?.node?)
        }
        Expr::Builtin(Builtin::KeysUnsorted) => node_keys_pair(root),
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
                CommentTree::object(NodeMeta::empty(), trees, IndexMap::new()),
            ))
        }
        Expr::Slice { start, end, .. } => {
            // One walk of the sequence: `get_cursor` per kept element walks it again each time.
            let cursors = root.value().as_array()?.collect_cursors();
            let range = succinctly::jq::literal_slice_range(*start, *end, cursors.len());
            let mut values = Vec::with_capacity(range.len());
            let mut trees = Vec::with_capacity(range.len());
            for node in &cursors[range] {
                let (v, t) = node_pair_as(node, true).ok()?;
                values.push(v);
                trees.push(t);
            }
            Some((
                OwnedValue::Array(values.into_iter().collect()),
                CommentTree::array(NodeMeta::empty(), trees),
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
                CommentTree::array(NodeMeta::empty(), trees),
            ))
        }
        _ => None,
    }
}

/// Whether every `omit` in `expr` runs on the document itself: the whole filter, or the first stage
/// of its pipe (#6906). The replay hands `omit` the tree of its input, and for a node below the root
/// that tree carries the document's trailing comment, which yq does not print there
/// (`.b | omit(["x"])` over a document with a foot comment); the plain route is right for those.
fn omit_is_rooted(expr: &Expr) -> bool {
    let holds_omit = |e: &Expr| {
        succinctly::jq::walk::any_subexpr(e, &mut |e| matches!(e, Expr::Builtin(Builtin::Omit(_))))
    };
    match expr {
        Expr::Builtin(Builtin::Omit(_)) => true,
        Expr::Paren(inner) => omit_is_rooted(inner),
        Expr::Pipe(parts) => parts
            .split_first()
            .is_some_and(|(first, rest)| omit_is_rooted(first) && !rest.iter().any(holds_omit)),
        other => !holds_omit(other),
    }
}

/// Skip ordinary navigation and writes, whose existing paths already carry
/// metadata. A supported full expression must contain an actual reshape.
pub(super) fn needed(expr: &Expr) -> bool {
    if !supported(expr) || !omit_is_rooted(expr) {
        return false;
    }
    match expr {
        Expr::Builtin(_)
        | Expr::Arithmetic { .. }
        | Expr::MetaAssign {
            slot: MetaSlot::KeyName,
            ..
        } => true,
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
            // #6674: yq keeps a mapping's head comment across `to_entries | from_entries` with
            // nothing between the two, where a `map` between them builds a new sequence and drops it.
            if matches!(
                parts.as_slice(),
                [
                    Expr::Builtin(Builtin::ToEntries),
                    Expr::Builtin(Builtin::FromEntries)
                ]
            ) && matches!(value, OwnedValue::Object(_))
            {
                restore_head(&mut pair.1, tree);
            }
            Some(pair)
        }
        Expr::Builtin(Builtin::Keys | Builtin::KeysUnsorted | Builtin::ToEntries) => {
            let result = evaluated(expr, value)?;
            let OwnedValue::Array(outputs) = &result else {
                return None;
            };
            let mut children = Vec::new();
            // #6335: `to_entries` lists a repeated key once per entry, and the by-key tree holds
            // the live one's presentation. `totals` counts the entries of each key and `earlier`
            // those listed so far, so an entry before the last is a shadow, found by its position
            // among them, as the writer finds it (`KeyCounts`). Counted once, and only for a
            // mapping whose tree kept a shadow's spelling.
            let has_shadows = matches!(value, OwnedValue::Object(o) if o.has_shadows())
                && tree.has_shadow_leaves()
                && matches!(expr, Expr::Builtin(Builtin::ToEntries));
            let mut totals = KeyCounts::default();
            if has_shadows {
                for output in outputs {
                    if let OwnedValue::Object(entry) = output {
                        if let Some(OwnedValue::String(key)) = entry.get("key") {
                            totals.bump(key.as_ref());
                        }
                    }
                }
            }
            let mut earlier = KeyCounts::default();
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
                        // its `&anchor` rides along (`key: &k key`, #2598), and so
                        // does an alias used as the key (`key: *a`, #5469).
                        // And its explicit tag (`!foo q`, #5469), as the `keys` writers give it.
                        let mut key_meta =
                            NodeMeta::from_comment_and_style(None, tree.key_style(key)).with_tag(
                                tree.key_tag(key).filter(|t| *t != "!").map(str::to_string),
                            );
                        key_meta.anchor = tree
                            .key_anchor(key)
                            .map(|name| AnchorMark::Declares(name.to_string()))
                            .or_else(|| {
                                tree.key_alias(key)
                                    .map(|name| AnchorMark::Aliases(name.to_string()))
                            });
                        let value_tree = if has_shadows {
                            let ordinal = earlier.bump(key.as_ref());
                            let total = totals.of(key.as_ref());
                            if ordinal + 1 < total {
                                // As the writer: a tree that kept another number of shadows
                                // than the mapping holds cannot be aligned by position.
                                if tree.key_meta(key).map_or(0, |meta| meta.shadows().len())
                                    == total - 1
                                {
                                    tree.shadow_field(key, ordinal).clone()
                                } else {
                                    CommentTree::empty()
                                }
                            } else {
                                tree.field(key).clone()
                            }
                        } else {
                            tree.field(key).clone()
                        };
                        (CommentTree::leaf(key_meta), value_tree)
                    }
                    OwnedValue::Array(_) => (CommentTree::empty(), tree.at_index(i).clone()),
                    _ => return None,
                };
                children.push(if is_entries {
                    CommentTree::object(
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
            Some((result, CommentTree::array(NodeMeta::empty(), children)))
        }
        Expr::Builtin(Builtin::FromEntries) => {
            let result = evaluated(expr, value)?;
            let OwnedValue::Array(entries) = value else {
                return None;
            };
            let mut fields = IndexMap::new();
            let mut keys = IndexMap::new();
            // #6335: an entry a later one of the same key replaces is a shadow of it.
            let mut shadow_leaves: IndexMap<String, Vec<CommentTree>> = IndexMap::new();
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
                if let Some(earlier) =
                    fields.insert(key.to_string(), entry_tree.field(value_field).clone())
                {
                    shadow_leaves
                        .entry(key.to_string())
                        .or_default()
                        .push(KeyMeta::shadow_leaf(&earlier));
                }
                // Last occurrence wins, including an unstyled key. Never
                // leave the earlier occurrence's style behind on collision.
                keys.shift_remove(key.as_ref());
                let key_tree = entry_tree.field(key_field);
                if let Some(meta) = KeyMeta::with_alias(
                    None,
                    false,
                    key_tree.style(),
                    key_tree.declared_anchor().map(str::to_string),
                    None,
                    key_tree.alias_name().map(str::to_string),
                ) {
                    keys.insert(key.to_string(), meta);
                }
            }
            KeyMeta::record_shadows(&mut keys, shadow_leaves);
            Some((result, CommentTree::object(NodeMeta::empty(), fields, keys)))
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
                if let Some(meta) = KeyMeta::with_shadows(
                    None,
                    false,
                    tree.key_style(key),
                    tree.key_anchor(key).map(str::to_string),
                    tree.key_tag(key).map(str::to_string),
                    tree.key_alias(key).map(str::to_string),
                    // #6335
                    tree.key_meta(key)
                        .map_or_else(Vec::new, |meta| meta.shadows().to_vec()),
                ) {
                    keys.insert(key.clone(), meta);
                }
            }
            Some((
                result,
                CommentTree::object(tree.meta().clone(), fields, keys),
            ))
        }
        // #5003: a picked entry is the document's own node, key and all: its comments, style,
        // anchor and tag come with it, as do the document's head comment and the key's.
        Expr::Builtin(Builtin::Pick(keys)) => pick_pair(keys, input),
        // #6906: what `omit` keeps is the document's own nodes too, the complement of a pick.
        Expr::Builtin(Builtin::Omit(keys)) => omit_pair(keys, input),
        Expr::Builtin(Builtin::WithEntries(f)) => {
            let entries = next(&Expr::Builtin(Builtin::ToEntries), input)?;
            let mapped = next(&Expr::Builtin(Builtin::Map(f.clone())), &entries)?;
            let (result, mut rebuilt) = next(&Expr::Builtin(Builtin::FromEntries), &mapped)?;
            // #6674: yq's `with_entries` keeps the document's head comment even when its filter
            // drops the first entry, where the same steps written out (`to_entries | map(...) |
            // from_entries`) drop it with that entry.
            if matches!(value, OwnedValue::Object(_)) {
                restore_head(&mut rebuilt, tree);
            }
            Some((result, rebuilt))
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
                    CommentTree::object(tree.meta().clone(), fields, keys),
                ))
            }
            _ => next(&Expr::Builtin(Builtin::Map(f.clone())), input),
        },
        // `flatten` keeps every leaf's own node (#5429). The value is built here too, and the caller
        // compares it with the authoritative one, so a depth this reads differently is the plain
        // route's.
        Expr::Builtin(Builtin::Flatten | Builtin::FlattenDepth(_)) => {
            let depth = match expr {
                Expr::Builtin(Builtin::FlattenDepth(depth)) => {
                    match evaluated(depth, &OwnedValue::Null)? {
                        OwnedValue::NumberLiteral(NumberRepr::Int(n), _) => {
                            usize::try_from(n).ok()?
                        }
                        _ => return None,
                    }
                }
                _ => usize::MAX,
            };
            let mut values = Vec::new();
            let mut trees = Vec::new();
            flatten_pairs(value, tree, depth, &mut values, &mut trees)?;
            // A trailing comment of the sequence is not carried: yq drops it, as it does the
            // comment above a sequence that `flatten` dissolves.
            let meta = tree
                .meta()
                .with_head_foot(tree.meta().head_comment().to_vec(), Vec::new());
            Some((
                OwnedValue::Array(values.into_iter().collect()),
                CommentTree::array(meta, trees),
            ))
        }
        // #6102: `pivot` moves each cell into a new sequence or mapping, with its comment, style,
        // anchor and tag, and pads a short row with an empty node, which yq prints as nothing.
        Expr::Builtin(Builtin::Pivot) => pivot_pair(expr, input),
        Expr::Builtin(Builtin::Map(f)) => {
            let OwnedValue::Array(items) = value else {
                return None;
            };
            let mut values = Vec::new();
            let mut trees = Vec::new();
            // yq keeps a document's head comment on its first key, so it goes with that entry.
            let mut first_dropped = false;
            let select = select_condition(f);
            for (i, item) in items.iter().enumerate() {
                if let Some(cond) = select {
                    // #6674: an item that passes is kept as it is, with its own tree; one that
                    // fails is dropped. A test with no single answer is the plain route's.
                    let keep = evaluated(cond, item)?;
                    if matches!(keep, OwnedValue::Null | OwnedValue::Bool(false)) {
                        first_dropped |= i == 0;
                        continue;
                    }
                    values.push(item.clone());
                    trees.push(tree.at_index(i).clone());
                    continue;
                }
                let (v, t) = next(f, &(item.clone(), tree.at_index(i).clone()))?;
                values.push(v);
                trees.push(t);
            }
            let meta = if first_dropped {
                tree.meta()
                    .with_head_foot(Vec::new(), tree.meta().foot_comment().to_vec())
            } else {
                tree.meta().clone()
            };
            Some((
                OwnedValue::Array(values.into_iter().collect()),
                CommentTree::array(meta, trees),
            ))
        }
        Expr::Arithmetic {
            op, left, right, ..
        } => {
            let l = next(left, input)?;
            let r = next(right, input)?;
            let result = evaluated(expr, value)?;
            let meta = if matches!(op, ArithOp::Add | ArithOp::Mul(_)) {
                merge_tree(
                    &l,
                    &r,
                    &result,
                    matches!(op, ArithOp::Mul(_)),
                    merge_flags(op),
                    depth + 1,
                )?
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
        // #6089: a renamed key keeps its node: its quoting, anchor and comments move to the new name,
        // and the value under it is untouched.
        Expr::MetaAssign {
            slot: MetaSlot::KeyName,
            value: rhs,
            is_update,
            ..
        } => {
            let result = evaluated(expr, value)?;
            // Whether the name a key was given is a string, which decides whether a plain key that
            // reads as another type is quoted (`= "1"` is `"1":`, `= 1` is `1:`).
            let is_string = |old: &str| {
                let operand = if *is_update {
                    OwnedValue::String(old.into())
                } else {
                    value.clone()
                };
                evaluated(rhs, &operand).map(|name| matches!(name, OwnedValue::String(_)))
            };
            let tree = renamed_tree(value, &result, tree, &is_string, depth + 1)?;
            Some((result, tree))
        }
        _ if is_closed_literal(expr) => Some((evaluated(expr, value)?, CommentTree::empty())),
        _ => None,
    }
}

/// The tree of a node `pivot` creates to pad a short row (#6102): a `null` with an empty spelling,
/// which yq prints as nothing (`-` with no value, `key:` bare) and `-o=json` as `null`.
fn pivot_padding() -> CommentTree {
    CommentTree::leaf(NodeMeta::empty().with_spelling(Some(String::new())))
}

/// `pivot` replayed from the cells' own nodes (#6102): every cell keeps its comment, style, anchor
/// and tag, the sequences and mapping it builds are new block nodes, and a cell the row lacks is
/// [`pivot_padding`]. The value is built by the authoritative evaluation, which the caller
/// compares with the traced one.
fn pivot_pair(expr: &Expr, input: &ResultWithComments) -> Option<ResultWithComments> {
    let (value, tree) = input;
    let OwnedValue::Array(rows) = value else {
        return None;
    };
    let result = evaluated(expr, value)?;
    // A cell of the row at `row` and position `at`, from the row's own tree.
    let cell = |row: usize, at: &dyn Fn(&CommentTree) -> CommentTree, present: bool| {
        if present {
            at(tree.at_index(row))
        } else {
            pivot_padding()
        }
    };
    let column = |cells: Vec<CommentTree>| CommentTree::array(NodeMeta::empty(), cells);
    match &result {
        OwnedValue::Array(columns) => {
            let mut trees = Vec::with_capacity(columns.len());
            for col in 0..columns.len() {
                let cells = rows
                    .iter()
                    .enumerate()
                    .map(|(row, item)| {
                        let present = matches!(item, OwnedValue::Array(r) if col < r.len());
                        cell(row, &|t| t.at_index(col).clone(), present)
                    })
                    .collect();
                trees.push(column(cells));
            }
            Some((result, CommentTree::array(NodeMeta::empty(), trees)))
        }
        OwnedValue::Object(columns) => {
            let mut fields = IndexMap::new();
            let mut keys = IndexMap::new();
            for key in columns.keys() {
                // The key is a new string node, so one that reads as another type stays quoted.
                if let Some(meta) = KeyMeta::new(None, false, KEY_STYLE_STRING) {
                    keys.insert(key.clone(), meta);
                }
                let cells = rows
                    .iter()
                    .enumerate()
                    .map(|(row, item)| {
                        let present = matches!(item, OwnedValue::Object(o) if o.contains_key(key));
                        cell(row, &|t| t.field(key).clone(), present)
                    })
                    .collect();
                fields.insert(key.clone(), column(cells));
            }
            Some((result, CommentTree::object(NodeMeta::empty(), fields, keys)))
        }
        _ => None,
    }
}

/// The tree of `new`, the document `old` became when a key rename gave some of its mapping members
/// new names (#6089): the same tree with each renamed member's field and key entry moved to the new
/// name, in the same position. `None` where the two differ in anything but the names (a member added
/// by the write, say), which the caller's plain route answers.
fn renamed_tree(
    old: &OwnedValue,
    new: &OwnedValue,
    tree: &CommentTree,
    is_string: &dyn Fn(&str) -> Option<bool>,
    depth: usize,
) -> Option<CommentTree> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    match (old, new) {
        (OwnedValue::Object(before), OwnedValue::Object(after)) => {
            if before.len() != after.len() {
                return None;
            }
            let mut fields = IndexMap::new();
            let mut keys = IndexMap::new();
            for ((old_key, old_value), (new_key, new_value)) in before.iter().zip(after.iter()) {
                let child = renamed_tree(
                    old_value,
                    new_value,
                    tree.field(old_key),
                    is_string,
                    depth + 1,
                )?;
                fields.insert(new_key.clone(), child);
                let meta = if old_key == new_key {
                    tree.key_meta(old_key).cloned()
                } else {
                    // A quoted key stays quoted; a plain one is a string key when it was given a
                    // string, so that it is quoted if it reads as another type.
                    let style = if !is_string(old_key)? {
                        ""
                    } else if tree.key_meta(old_key).is_some_and(KeyMeta::is_quoted) {
                        tree.key_style(old_key)
                    } else {
                        KEY_STYLE_STRING
                    };
                    KeyMeta::with_alias(
                        tree.key_comment(old_key).map(str::to_string),
                        false,
                        style,
                        tree.key_anchor(old_key).map(str::to_string),
                        tree.key_tag(old_key).map(str::to_string),
                        tree.key_alias(old_key).map(str::to_string),
                    )
                };
                if let Some(meta) = meta {
                    keys.insert(new_key.clone(), meta);
                }
            }
            Some(CommentTree::object(tree.meta().clone(), fields, keys))
        }
        (OwnedValue::Array(before), OwnedValue::Array(after)) => {
            if before.len() != after.len() {
                return None;
            }
            let mut items = Vec::with_capacity(before.len());
            for (i, (old_item, new_item)) in before.iter().zip(after.iter()).enumerate() {
                items.push(renamed_tree(
                    old_item,
                    new_item,
                    tree.at_index(i),
                    is_string,
                    depth + 1,
                )?);
            }
            Some(CommentTree::array(tree.meta().clone(), items))
        }
        (old, new) => (old == new).then(|| tree.clone()),
    }
}

/// The elements `flatten` keeps, with their trees, in order: those of the nested sequences' own
/// elements down to `depth` levels, and every other element as it stands (#5429).
fn flatten_pairs(
    value: &OwnedValue,
    tree: &CommentTree,
    depth: usize,
    values: &mut Vec<OwnedValue>,
    trees: &mut Vec<CommentTree>,
) -> Option<()> {
    // patchcov: coverage tolerate reason="unreachable: the first call holds a sequence (`flatten` of anything else raised in the authoritative evaluation, which decides before this tree is used) and a recursive call is made on one (#5429)"
    let OwnedValue::Array(items) = value else {
        return None;
    };
    // patchcov: coverage end
    for (i, item) in items.iter().enumerate() {
        match item {
            OwnedValue::Array(_) if depth > 0 => {
                flatten_pairs(item, tree.at_index(i), depth - 1, values, trees)?;
            }
            _ => {
                values.push(item.clone());
                trees.push(tree.at_index(i).clone());
            }
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
                CommentTree::object(tree.meta().clone(), fields, key_metas),
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
                CommentTree::array(tree.meta().clone(), children),
            ))
        }
        _ => None,
    }
}

/// `omit(KEYS)` over `input`: every member whose key is not listed, or every item of a sequence
/// whose index is not (#6906). What stays is the document's own node, key and all, as for
/// [`pick_pair`]: its comments, style, anchor and tag come with it, and so do the document's head
/// comment and the keys'. Anything but a list of string keys over a mapping, or of non-negative
/// integers over a sequence, is the plain route's; so is a value that differs from the
/// authoritative result, which the caller checks.
fn omit_pair(keys: &Expr, input: &ResultWithComments) -> Option<ResultWithComments> {
    let (value, tree) = input;
    let OwnedValue::Array(keys) = evaluated(keys, &OwnedValue::Null)? else {
        return None;
    };
    match value {
        OwnedValue::Object(members) => {
            let mut listed = std::collections::BTreeSet::new();
            for key in &keys {
                // A key that is not a string matches no member (the evaluator skips it too).
                if let OwnedValue::String(key) = key {
                    listed.insert(key.as_ref());
                }
            }
            let mut kept = IndexMap::new();
            let mut fields = IndexMap::new();
            let mut key_metas = IndexMap::new();
            for (key, member) in members {
                if listed.contains(key.as_str()) {
                    continue;
                }
                kept.insert(key.clone(), member.clone());
                fields.insert(key.clone(), tree.field(key).clone());
                if let Some(meta) = tree.key_meta(key) {
                    key_metas.insert(key.clone(), meta.clone());
                }
            }
            Some((
                OwnedValue::Object(kept.into()),
                CommentTree::object(tree.meta().clone(), fields, key_metas),
            ))
        }
        OwnedValue::Array(items) => {
            let mut listed = std::collections::BTreeSet::new();
            for index in &keys {
                // Only a non-negative integer removes anything: a negative index does not wrap and
                // a string or float key matches nothing (#4257).
                if let OwnedValue::Int(index)
                | OwnedValue::NumberLiteral(NumberRepr::Int(index), _) = index
                {
                    if let Ok(index) = usize::try_from(*index) {
                        listed.insert(index);
                    }
                }
            }
            let mut kept = Vec::new();
            let mut children = Vec::new();
            for (index, item) in items.iter().enumerate() {
                if listed.contains(&index) {
                    continue;
                }
                kept.push(item.clone());
                children.push(tree.at_index(index).clone());
            }
            Some((
                OwnedValue::Array(kept.into_iter().collect()),
                CommentTree::array(tree.meta().clone(), children),
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
    flags: MergeFlags,
    depth: usize,
) -> Option<CommentTree> {
    if depth >= MAX_VALUE_TREE_DEPTH {
        return None;
    }
    let (lv, lt) = left;
    let (rv, rt) = right;
    if let (OwnedValue::Array(l), OwnedValue::Array(r)) = (lv, rv) {
        // An empty left array is no node to keep: `*`, `*+` and `*d` answer the right one as it is
        // (`[]` merged with `[1]` prints a block sequence in yq, not the `[]`'s flow style).
        if deep && l.is_empty() && !flags.only_new && !flags.only_existing {
            return (result == rv).then(|| rt.clone());
        }
        if !deep || flags.append_arrays {
            // `+`, and `*+` (#6900): the result is the left items and then the right ones. They append
            // to the left node, which keeps its `&anchor`; a merge that replaces or rebuilds one does not.
            let OwnedValue::Array(out) = result else {
                return None;
            };
            if out.len() != l.len() + r.len() {
                return None;
            }
            let children = (0..l.len())
                .map(|i| lt.at_index(i).clone())
                .chain((0..r.len()).map(|i| rt.at_index(i).clone()))
                .collect();
            return Some(CommentTree::array(lt.meta().clone(), children));
        }
        if flags.deep_merge_arrays {
            // `*d` (#6900): the arrays merge by index, an element on both sides merging as the
            // values do and an element on one side keeping its own presentation.
            let OwnedValue::Array(out) = result else {
                return None;
            };
            if out.len() != l.len().max(r.len()) {
                return None;
            }
            let mut children = Vec::with_capacity(out.len());
            for (i, value) in out.iter().enumerate() {
                children.push(match (l.get(i), r.get(i)) {
                    (Some(lvalue), Some(rvalue)) => merge_tree(
                        &(lvalue.clone(), lt.at_index(i).clone()),
                        &(rvalue.clone(), rt.at_index(i).clone()),
                        value,
                        true,
                        flags,
                        depth + 1,
                    )?,
                    (Some(_), None) => lt.at_index(i).clone(),
                    (None, Some(_)) => rt.at_index(i).clone(),
                    (None, None) => return None,
                });
            }
            return Some(CommentTree::array(
                lt.meta().without_declared_anchor(),
                children,
            ));
        }
        // `*n` keeps the left array that is there: its tree, and no replay when the result is not
        // that array. `*?` replaces it as `*` does, below.
        if flags.only_new {
            return (result == lv).then(|| lt.clone());
        }
        // An array the merge replaces keeps the left node's own presentation (a flow style, a
        // comment) around the right items, whose presentation they bring along.
        if result == rv {
            let children = (0..r.len()).map(|i| rt.at_index(i).clone()).collect();
            return Some(CommentTree::array(
                lt.meta().without_declared_anchor(),
                children,
            ));
        }
        // A `*?` result that is not the right array is not modelled; a plain `*` keeps the right
        // tree below, as it did before the flags were followed.
        if flags.only_existing {
            return None;
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
                    flags,
                    depth + 1,
                )?,
                (_, Some(_)) => rt.field(key).clone(),
                (Some(_), None) => lt.field(key).clone(),
                _ => return None,
            };
            fields.insert(key.clone(), child);
            let key_tree = if lvalue.is_some() { lt } else { rt };
            if let Some(meta) = KeyMeta::with_shadows(
                None,
                false,
                key_tree.key_style(key),
                key_tree.key_anchor(key).map(str::to_string),
                key_tree.key_tag(key).map(str::to_string),
                key_tree.key_alias(key).map(str::to_string),
                // #6335: the key's earlier entries are the ones the merge left where they were.
                key_tree
                    .key_meta(key)
                    .map_or_else(Vec::new, |meta| meta.shadows().to_vec()),
            ) {
                keys.insert(key.clone(), meta);
            }
        }
        Some(CommentTree::object(
            lt.meta().without_declared_anchor(),
            fields,
            keys,
        ))
    } else if !matches!(lv, OwnedValue::Array(_) | OwnedValue::Object(_))
        && !matches!(rv, OwnedValue::Array(_) | OwnedValue::Object(_))
    {
        Some(CommentTree::leaf(lt.meta().clone()))
    } else if flags.only_new && *lv != OwnedValue::Null {
        // `*n` (#6900): a left value that is there stays, whatever the right one is.
        Some(lt.clone())
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
            ".a *+d .b",
            ".a *c .b",
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
        let styled = CommentTree::object(
            NodeMeta::empty(),
            IndexMap::from([(
                "key".to_string(),
                CommentTree::leaf(NodeMeta::from_comment_and_style(None, "single")),
            )]),
            IndexMap::new(),
        );
        let input = (
            OwnedValue::Array([entry(1), entry(2)].into_iter().collect()),
            CommentTree::array(NodeMeta::empty(), vec![styled, CommentTree::empty()]),
        );
        let (value, tree) = trace(&Expr::Builtin(Builtin::FromEntries), &input).unwrap();
        assert_eq!(
            value,
            OwnedValue::Object(IndexMap::from([("a".to_string(), OwnedValue::Int(2))]).into())
        );
        assert_eq!(tree.key_style("a"), "");
    }
}
