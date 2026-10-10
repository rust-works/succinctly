//! yq's `COLLECT_OBJECT` for a `{...}` that holds a bare entry (#2783).
//!
//! yq has no object-construction shorthand. Its `{ expr : expr , ... }` is the
//! `COLLECT_OBJECT` operator over a `UNION` of entries, and a `key: value` pair
//! is the binary `CREATE_MAP` operator. A lone expression such as `$a` is
//! therefore a legal entry that simply is not a pair, and `COLLECT_OBJECT`
//! handles it with whatever its children happen to be -- which is how
//! `1 as $a | {$a}` ends up printing nothing while `{"a": 1, "b"}` fails with a
//! node-size error. This module is that operator, written against
//! [`OwnedValue`] and kept literal to `collectObjectOperator` in yq v4.53.3
//! (`pkg/yqlib/operator_collect_object.go`), quirks included, because the
//! quirks are the behaviour being reproduced (ADR-0018 rule 3).
//!
//! A construction with only pair entries never comes here: its cross product is
//! what this operator computes, and the evaluators' own fan-out already produces
//! it without the round trip through nodes. A repeated key is where the fold shows,
//! and the fan-out reproduces it where it assembles each object
//! ([`object_from_pairs`](super::eval::object_from_pairs), #4182), so the two
//! routes agree.

use alloc::string::String;
use alloc::vec::Vec;

use indexmap::IndexMap;

use super::error::EvalError;
use super::eval::{arith_mul, cannot_reserve_cross_product, vec_with_capacity, EvalSemantics};
use super::expr::MergeFlags;
use super::value::OwnedValue;

/// What one entry of a `{...}` contributes to the `UNION` `COLLECT_OBJECT` reads.
pub(crate) enum UnionEntry {
    /// A `key: value` pair: the one-key maps `CREATE_MAP` produced, one per
    /// combination of the key's and the value's outputs.
    Pair(Vec<OwnedValue>),
    /// A bare expression: its outputs, each a node of the union in its own right.
    Bare(Vec<OwnedValue>),
}

/// The node's `Content`: a sequence's items, a mapping's flat `[key, value, ...]`
/// list, nothing for a scalar.
fn content(node: &OwnedValue) -> Vec<OwnedValue> {
    match node {
        OwnedValue::Array(items) => items.iter().cloned().collect(),
        OwnedValue::Object(map) => {
            let mut flat = vec_with_capacity(map.len() * 2);
            for (key, value) in map {
                flat.push(OwnedValue::String(String::clone(key).into()));
                flat.push(value.clone());
            }
            flat
        }
        _ => Vec::new(),
    }
}

/// yq's `splat` without map keys: a sequence's items, a mapping's values, and
/// nothing at all for a scalar.
fn splat(node: &OwnedValue) -> Vec<OwnedValue> {
    match node {
        OwnedValue::Array(items) => items.iter().cloned().collect(),
        OwnedValue::Object(map) => map.iter().map(|(_, value)| value.clone()).collect(),
        _ => Vec::new(),
    }
}

/// [`collect_object`] for a construction of pairs only (#4193): the same fold, without
/// wrapping each pair's maps as a union node and splatting them back out.
///
/// Every pair is one child of its node, so `N` is 1 and the size check cannot fail; the
/// aggregate is the first pair's maps, and each later pair multiplies into it. A pair
/// with no maps empties the aggregate, and the next one seeds it afresh -- the restart
/// `collect_object`'s general loop describes -- so `{"a": 1, "b": empty, "c": 3}` is
/// `c: 3` and a trailing empty pair leaves nothing.
fn fold_pairs<S: EvalSemantics>(entries: Vec<UnionEntry>) -> Result<Vec<OwnedValue>, EvalError> {
    let mut aggregate: Vec<OwnedValue> = Vec::new();
    for entry in entries {
        if let UnionEntry::Pair(maps) = entry {
            if aggregate.is_empty() {
                aggregate = maps;
                continue;
            }
            // One map meeting one map is the common case. Moving the held map in, not
            // a clone of it, leaves its storage unshared so the merge extends it in place
            // instead of copying it once per pair.
            if let ([_], [_]) = (aggregate.as_slice(), maps.as_slice()) {
                let mut maps = maps;
                if let (Some(held), Some(addition)) = (aggregate.pop(), maps.pop()) {
                    aggregate.push(arith_mul::<S>(held, addition, MergeFlags::default())?);
                }
                continue;
            }
            let mut next: Vec<OwnedValue> = Vec::new();
            for held in &aggregate {
                next.try_reserve(maps.len())
                    .map_err(|_| cannot_reserve_cross_product(&[aggregate.len(), maps.len()]))?;
                for addition in &maps {
                    next.push(arith_mul::<S>(
                        held.clone(),
                        addition.clone(),
                        MergeFlags::default(),
                    )?);
                }
            }
            aggregate = next;
        }
    }
    Ok(aggregate)
}

/// `collectObjectOperator` over the union of `entries`, evaluated for one input.
///
/// `CREATE_MAP` wraps a pair's maps in a one-element sequence (one inner
/// sequence per matching input node, and there is one), so a pair contributes a
/// union node with exactly one child. That keeps `N` -- the child count of the
/// first union node, which every other node must at least match -- equal to
/// what yq computes when every entry is a pair, and lets a bare entry's own
/// child count disagree with it.
pub(crate) fn collect_object<S: EvalSemantics>(
    entries: Vec<UnionEntry>,
) -> Result<Vec<OwnedValue>, EvalError> {
    if !entries.is_empty() && entries.iter().all(|e| matches!(e, UnionEntry::Pair(_))) {
        return fold_pairs::<S>(entries);
    }
    let mut union: Vec<OwnedValue> = vec_with_capacity(entries.len());
    for entry in entries {
        match entry {
            UnionEntry::Pair(maps) => {
                union.push(OwnedValue::Array(
                    alloc::vec![OwnedValue::Array(maps.into())].into(),
                ));
            }
            UnionEntry::Bare(nodes) => union.extend(nodes),
        }
    }

    // `context.MatchingNodes.Len() == 0`: every entry produced nothing.
    let Some(first) = union.first() else {
        return Ok(alloc::vec![OwnedValue::Object(IndexMap::new().into())]);
    };
    let width = content(first).len();

    let mut children = vec_with_capacity(union.len());
    for node in &union {
        let kids = content(node);
        if kids.len() < width {
            return Err(EvalError::new(
                "CollectObject: mismatching node sizes; are you creating a map with \
                 mismatching key value pairs?",
            ));
        }
        children.push(kids);
    }

    let mut out = Vec::new();
    for i in 0..width {
        // `collect`: the first candidate to splat to something seeds the
        // aggregate; every later one is cross-multiplied into each member of it.
        // A later candidate that splats to nothing empties the aggregate, and the
        // next one then seeds it afresh -- yq's own recursion does exactly that.
        let mut aggregate: Vec<OwnedValue> = Vec::new();
        for kids in &children {
            let splatted = splat(&kids[i]);
            if aggregate.is_empty() {
                aggregate = splatted;
                continue;
            }
            // The product of two data-controlled lengths: reserved a row at a time,
            // so an oversized one is refused rather than aborting the process.
            let mut next: Vec<OwnedValue> = Vec::new();
            for held in &aggregate {
                next.try_reserve(splatted.len()).map_err(|_| {
                    cannot_reserve_cross_product(&[aggregate.len(), splatted.len()])
                })?;
                for addition in &splatted {
                    next.push(arith_mul::<S>(
                        held.clone(),
                        addition.clone(),
                        MergeFlags::default(),
                    )?);
                }
            }
            aggregate = next;
        }
        out.extend(aggregate);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jq::eval::YqSemantics;

    fn object(pairs: &[(&str, OwnedValue)]) -> OwnedValue {
        OwnedValue::Object(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect::<IndexMap<_, _>>()
                .into(),
        )
    }

    fn array(items: &[OwnedValue]) -> OwnedValue {
        OwnedValue::Array(items.to_vec().into())
    }

    fn int(n: i64) -> OwnedValue {
        OwnedValue::Int(n)
    }

    #[test]
    fn an_empty_union_is_the_empty_map() {
        assert_eq!(
            collect_object::<YqSemantics>(Vec::new()).unwrap(),
            alloc::vec![object(&[])]
        );
    }

    #[test]
    fn pairs_merge_and_a_fan_out_yields_one_object_per_combination() {
        let out = collect_object::<YqSemantics>(alloc::vec![
            UnionEntry::Pair(alloc::vec![
                object(&[("a", int(1))]),
                object(&[("b", int(1))])
            ]),
            UnionEntry::Pair(alloc::vec![object(&[("c", int(2))])]),
        ])
        .unwrap();
        assert_eq!(
            out,
            alloc::vec![
                object(&[("a", int(1)), ("c", int(2))]),
                object(&[("b", int(1)), ("c", int(2))]),
            ]
        );
    }

    #[test]
    fn a_scalar_splats_to_nothing_and_a_mapping_to_its_values() {
        // A scalar first node has no children, so `N` is 0 and nothing is collected.
        assert_eq!(
            collect_object::<YqSemantics>(alloc::vec![UnionEntry::Bare(alloc::vec![int(1)])])
                .unwrap(),
            Vec::<OwnedValue>::new()
        );
        // A mapping's children are `[key, value]`: the key splats to nothing, the value
        // (a mapping) to its own values.
        let out =
            collect_object::<YqSemantics>(alloc::vec![UnionEntry::Bare(alloc::vec![object(&[(
                "k",
                object(&[("x", int(1))])
            )])])])
            .unwrap();
        assert_eq!(out, alloc::vec![int(1)]);
    }

    #[test]
    fn a_later_node_with_fewer_children_is_refused() {
        let err = collect_object::<YqSemantics>(alloc::vec![
            UnionEntry::Bare(alloc::vec![array(&[int(1), int(2)])]),
            UnionEntry::Bare(alloc::vec![array(&[int(3)])]),
        ])
        .unwrap_err();
        assert!(err
            .to_string()
            .starts_with("CollectObject: mismatching node sizes"));
    }

    #[test]
    fn an_empty_splat_empties_the_aggregate_and_the_next_one_reseeds_it() {
        // `[[1], [], [2]]` as one `i = 0` column: [1] seeds, [] empties it by cross
        // product, [2] seeds it again -- yq's own recursion.
        let out = collect_object::<YqSemantics>(alloc::vec![UnionEntry::Bare(alloc::vec![
            array(&[array(&[int(1)])]),
            array(&[array(&[])]),
            array(&[array(&[int(2)])]),
        ])])
        .unwrap();
        assert_eq!(out, alloc::vec![int(2)]);
    }

    #[test]
    fn a_map_cannot_be_multiplied_by_a_scalar() {
        let err = collect_object::<YqSemantics>(alloc::vec![
            UnionEntry::Pair(alloc::vec![object(&[("z", int(0))])]),
            UnionEntry::Bare(alloc::vec![array(&[array(&[int(1)])])]),
        ])
        .unwrap_err();
        assert!(
            err.to_string().contains("cannot be multiplied"),
            "{}",
            err.to_string()
        );
    }
}
