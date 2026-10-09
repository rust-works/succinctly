//! Bounded provenance for inexpensive YAML reshaping filters (#3615).
//!
//! This is a presentation pass, not an alternative evaluator. Only pure,
//! single-output shapes are replayed, and the caller checks the resulting
//! value against the authoritative generic result before using its tree.

use super::{
    collect_write_targets, is_closed_literal, reconcile_presentation, CommentTree, Expr, IndexMap,
    NodeMeta, OwnedValue, ResultWithComments,
};
use succinctly::jq::eval_generic::{AnchorMark, KeyMeta};
use succinctly::jq::{ArithOp, Builtin, MAX_VALUE_TREE_DEPTH};

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
        Expr::Arithmetic { op, left, right } => {
            !matches!(op, ArithOp::Mul(flags) if flags.append_arrays || flags.only_existing
                || flags.only_new || flags.deep_merge_arrays || flags.clobber_tags)
                && supported(left)
                && supported(right)
        }
        Expr::Builtin(
            Builtin::Keys | Builtin::KeysUnsorted | Builtin::ToEntries | Builtin::FromEntries,
        ) => true,
        Expr::Builtin(Builtin::WithEntries(f) | Builtin::Map(f)) => supported(f),
        Expr::Assign { path, value } => static_path(path) && supported(value),
        Expr::Update { path, filter } => static_path(path) && supported(filter),
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
                let key_field = ["key", "Key", "name", "Name"]
                    .into_iter()
                    .find(|k| entry.contains_key(*k))?;
                let OwnedValue::String(key) = entry.get(key_field)? else {
                    return None;
                };
                let value_field = ["value", "Value"]
                    .into_iter()
                    .find(|k| entry.contains_key(*k))
                    .unwrap_or("value");
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
        Expr::Builtin(Builtin::WithEntries(f)) => {
            let entries = next(&Expr::Builtin(Builtin::ToEntries), input)?;
            let mapped = next(&Expr::Builtin(Builtin::Map(f.clone())), &entries)?;
            next(&Expr::Builtin(Builtin::FromEntries), &mapped)
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
        Expr::Arithmetic { op, left, right } => {
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
