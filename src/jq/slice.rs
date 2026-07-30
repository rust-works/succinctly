//! jq's slice path component, `{"start":s,"end":e}`.
//!
//! jq does not have a slice *operator* — it models `.[a:b]` as indexing with
//! an object, and that object is a first-class path component. It comes out of
//! `path(.[a:b])`, goes into `getpath`/`setpath`/`delpaths`, and drives `=`,
//! `|=` and `del()`. So the same three questions get asked from a dozen call
//! sites across two path representations (`Expr::Slice` for a static path,
//! `OwnedValue::Object` for a runtime one), and they are answered here once:
//!
//! - is this object a well-formed descriptor? ([`SliceBounds::from_descriptor`])
//! - what element range does it name against a container of length `n`?
//!   ([`SliceBounds::resolve`])
//! - what does `path()` print for it? ([`literal_component`])
//!
//! The bounds are `f64` rather than `i64` because a runtime descriptor can
//! carry any number — `getpath([{"start":1.7,"end":2.9}])` is legal — even
//! though the parser folds `Expr::Slice`'s bounds to integer literals.

use core::ops::Range;

use indexmap::IndexMap;

use super::error::EvalError;
use super::value::{NumberRepr, OwnedValue};

/// A validated slice path component, before it is resolved against a length.
///
/// `None` is jq's `null` bound, which means "the end of the container on this
/// side" — and is what an omitted `.[1:]`/`.[:2]` bound becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SliceBounds {
    start: Option<f64>,
    end: Option<f64>,
}

impl SliceBounds {
    /// The `Expr::Slice` form, whose bounds the parser already folded to
    /// integer literals.
    pub(crate) fn from_literals(start: Option<i64>, end: Option<i64>) -> Self {
        Self {
            start: start.map(|i| i as f64),
            end: end.map(|i| i as f64),
        }
    }

    /// Read a runtime path component as jq's slice descriptor.
    ///
    /// jq looks up `start` and `end` and accepts each only as a number or
    /// `null`. A *missing* key is neither — `jv_object_get` hands back an
    /// invalid value, which fails the same check — so `{"start":1}` is
    /// rejected while `{"start":1,"end":null}` is fine. Extra keys are never
    /// looked at.
    pub(crate) fn from_descriptor(obj: &IndexMap<String, OwnedValue>) -> Result<Self, EvalError> {
        Ok(Self {
            start: bound(obj.get("start"))?,
            end: bound(obj.get("end"))?,
        })
    }

    /// The element range this names against a container of length `len`.
    ///
    /// jq's `parse_slice`: floor the start and ceil the end (so a fractional
    /// bound widens the slice rather than truncating it — `[1,2,3,4,5] |
    /// .[1.7:2.9]` is `[2,3]`), fold a negative bound against the length,
    /// clamp both into `0..=len`, then pull `end` up to `start` if they
    /// crossed.
    ///
    /// The last step is why this returns a range rather than a pair: crossed
    /// bounds are an *insertion point* on the write side, not an empty read.
    /// `[1,2,3] | setpath([{"start":2,"end":1}]; ["x"])` is `[1,2,"x",3]`.
    pub(crate) fn resolve(&self, len: usize) -> Range<usize> {
        let start = clamp(self.start.map_or(0.0, f64::floor), len);
        let end = clamp(self.end.map_or(len as f64, f64::ceil), len);
        start..end.max(start)
    }
}

/// One `start`/`end` slot of a descriptor.
///
/// `None` covers both spellings of "this side is open": the key held `null`,
/// or the value is not a number at all — the latter only after the key was
/// present, which the `?` on the caller's side has already established.
fn bound(slot: Option<&OwnedValue>) -> Result<Option<f64>, EvalError> {
    match slot {
        Some(OwnedValue::Null) => Ok(None),
        Some(OwnedValue::Int(i)) => Ok(Some(*i as f64)),
        Some(OwnedValue::Float(f)) => Ok(Some(*f)),
        Some(OwnedValue::NumberLiteral(NumberRepr::Int(i), _)) => Ok(Some(*i as f64)),
        Some(OwnedValue::NumberLiteral(NumberRepr::Float(f), _)) => Ok(Some(*f)),
        // A missing key and a wrong-typed one are the same refusal in jq,
        // which never distinguishes "absent" from "not a number" here.
        _ => Err(EvalError::slice_indices_not_integers()),
    }
}

/// Fold a resolved bound against `len` and clamp it into `0..=len`.
///
/// Written over `f64` so that a bound too large for a `usize` saturates at
/// `len` instead of wrapping: `[1,2,3] | .[1:1e100]` is `[2,3]`, and a NaN
/// bound — reachable only through a runtime descriptor — falls to `0`, since
/// every comparison against it is false.
fn clamp(bound: f64, len: usize) -> usize {
    let folded = if bound < 0.0 {
        bound + len as f64
    } else {
        bound
    };
    if folded < 0.0 || folded.is_nan() {
        0
    } else if folded >= len as f64 {
        len
    } else {
        folded as usize
    }
}

/// The path component `path(.[a:b])` prints.
///
/// The bounds are the literals as written, *not* resolved against anything:
/// `[1,2,3] | path(.[-2:-1])` is `[{"start":-2,"end":-1}]`, and an omitted
/// bound is `null`. jq resolves only when the component is finally applied to
/// a container, which is what lets a path outlive the value it was taken from.
pub(crate) fn literal_component(start: Option<i64>, end: Option<i64>) -> OwnedValue {
    let mut obj = IndexMap::with_capacity(2);
    obj.insert(
        "start".into(),
        start.map_or(OwnedValue::Null, OwnedValue::Int),
    );
    obj.insert("end".into(), end.map_or(OwnedValue::Null, OwnedValue::Int));
    OwnedValue::Object(obj)
}

/// The character range `range` of `s`, as an owned string.
///
/// jq indexes string slices by *character*, not byte, so the caller resolves
/// against `s.chars().count()` and this walks the same units back out.
pub(crate) fn slice_str(s: &str, range: Range<usize>) -> alloc::string::String {
    s.chars().skip(range.start).take(range.len()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(pairs: &[(&str, OwnedValue)]) -> IndexMap<String, OwnedValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn descriptor_needs_both_keys_present() {
        // An explicit null counts as present; a missing key does not. Both
        // spellings of "open on this side" resolve to the whole container.
        let both_null = desc(&[("start", OwnedValue::Null), ("end", OwnedValue::Null)]);
        assert_eq!(
            SliceBounds::from_descriptor(&both_null).unwrap().resolve(3),
            0..3
        );

        for missing in [
            desc(&[("start", OwnedValue::Int(1))]),
            desc(&[("end", OwnedValue::Int(2))]),
            desc(&[]),
            desc(&[("foo", OwnedValue::Int(1))]),
        ] {
            assert_eq!(
                SliceBounds::from_descriptor(&missing).unwrap_err().message,
                "Array/string slice indices must be integers"
            );
        }
    }

    #[test]
    fn descriptor_bounds_must_be_numbers() {
        for bad in [
            OwnedValue::String("a".into()),
            OwnedValue::Bool(true),
            OwnedValue::Array(Vec::new()),
        ] {
            let obj = desc(&[("start", bad), ("end", OwnedValue::Int(2))]);
            assert_eq!(
                SliceBounds::from_descriptor(&obj).unwrap_err().message,
                "Array/string slice indices must be integers"
            );
        }
    }

    #[test]
    fn descriptor_ignores_extra_keys() {
        let obj = desc(&[
            ("start", OwnedValue::Int(1)),
            ("end", OwnedValue::Int(2)),
            ("extra", OwnedValue::Int(9)),
        ]);
        assert_eq!(SliceBounds::from_descriptor(&obj).unwrap().resolve(3), 1..2);
    }

    #[test]
    fn fractional_bounds_widen_the_range() {
        // jq floors the start and ceils the end, so `.[1.7:2.9]` on a
        // 5-element array is `[2,3]` — indices 1..3, not 1..2.
        let b = SliceBounds {
            start: Some(1.7),
            end: Some(2.9),
        };
        assert_eq!(b.resolve(5), 1..3);
    }

    #[test]
    fn negative_bounds_fold_after_rounding() {
        // floor(-3.5) = -4, +5 = 1; ceil(-1.5) = -1, +5 = 4.
        let b = SliceBounds {
            start: Some(-3.5),
            end: Some(-1.5),
        };
        assert_eq!(b.resolve(5), 1..4);
    }

    #[test]
    fn crossed_bounds_become_an_insertion_point() {
        // Not an empty range at 0 — an empty range *at the start*, which is
        // where the write side splices.
        let b = SliceBounds::from_literals(Some(2), Some(1));
        assert_eq!(b.resolve(3), 2..2);
    }

    #[test]
    fn out_of_range_bounds_clamp_rather_than_fail() {
        assert_eq!(
            SliceBounds::from_literals(Some(5), Some(9)).resolve(3),
            3..3
        );
        assert_eq!(
            SliceBounds::from_literals(Some(-9), Some(-8)).resolve(3),
            0..0
        );
        let huge = SliceBounds {
            start: Some(1.0),
            end: Some(1e100),
        };
        assert_eq!(huge.resolve(3), 1..3);
    }

    #[test]
    fn literal_component_keeps_raw_bounds_and_spells_absent_as_null() {
        let OwnedValue::Object(obj) = literal_component(Some(-2), None) else {
            panic!("expected an object");
        };
        assert_eq!(obj.get("start"), Some(&OwnedValue::Int(-2)));
        assert_eq!(obj.get("end"), Some(&OwnedValue::Null));
        // `start` before `end`, as jq prints it.
        assert_eq!(
            obj.keys().map(String::as_str).collect::<Vec<_>>(),
            ["start", "end"]
        );
    }

    #[test]
    fn slice_str_counts_characters_not_bytes() {
        assert_eq!(slice_str("héllo", 1..3), "él");
    }
}
