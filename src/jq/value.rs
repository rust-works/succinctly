//! Owned JSON values for jq evaluation.
//!
//! When jq expressions construct new values (arrays, objects) or perform
//! computations, we need to materialize them into owned values rather than
//! references into the original JSON bytes.

#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec::Vec;

use indexmap::IndexMap;

use super::expr::Literal;

/// An owned JSON value.
///
/// This is used for values that are constructed during evaluation
/// (array/object construction, arithmetic results, etc.) rather than
/// references into the original JSON document.
/// Equality is *jq value equality*, not structural equality -- see the
/// hand-written [`PartialEq`] impl below. In particular `Int(1) == Float(1.0)`.
#[derive(Debug, Clone)]
pub enum OwnedValue {
    /// JSON null
    Null,
    /// JSON boolean
    Bool(bool),
    /// JSON integer (stored as i64 for precision)
    Int(i64),
    /// JSON floating-point number
    Float(f64),
    /// JSON string
    String(String),
    /// JSON array
    Array(Vec<Self>),
    /// JSON object (IndexMap preserves insertion order like jq)
    Object(IndexMap<String, Self>),
}

impl OwnedValue {
    /// Create a null value.
    pub fn null() -> Self {
        Self::Null
    }

    /// Create a boolean value.
    pub fn bool(b: bool) -> Self {
        Self::Bool(b)
    }

    /// Create an integer value.
    pub fn int(n: i64) -> Self {
        Self::Int(n)
    }

    /// Create a float value.
    pub fn float(f: f64) -> Self {
        Self::Float(f)
    }

    /// Create a string value.
    pub fn string(s: impl Into<String>) -> Self {
        Self::String(s.into())
    }

    /// Create an empty array.
    pub fn array() -> Self {
        Self::Array(Vec::new())
    }

    /// Create an array from a vector of values.
    pub fn array_from(values: Vec<Self>) -> Self {
        Self::Array(values)
    }

    /// Create an empty object.
    pub fn object() -> Self {
        Self::Object(IndexMap::new())
    }

    /// Create an object from key-value pairs.
    pub fn object_from(pairs: impl IntoIterator<Item = (String, Self)>) -> Self {
        Self::Object(pairs.into_iter().collect())
    }

    /// Check if this value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Check if this value is "truthy" (not null and not false).
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Null | Self::Bool(false))
    }

    /// Get the type name of this value.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Int(_) | Self::Float(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    /// Convert to a boolean, if possible.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert to an i64, if possible.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            Self::Float(f) if (*f - (*f as i64 as f64)).abs() < f64::EPSILON => Some(*f as i64),
            _ => None,
        }
    }

    /// Convert to an f64, if possible.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Convert to a string reference, if possible.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    /// Convert to an array reference, if possible.
    pub fn as_array(&self) -> Option<&Vec<Self>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Convert to a mutable array reference, if possible.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Self>> {
        match self {
            Self::Array(arr) => Some(arr),
            _ => None,
        }
    }

    /// Convert to an object reference, if possible.
    pub fn as_object(&self) -> Option<&IndexMap<String, Self>> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Convert to a mutable object reference, if possible.
    pub fn as_object_mut(&mut self) -> Option<&mut IndexMap<String, Self>> {
        match self {
            Self::Object(obj) => Some(obj),
            _ => None,
        }
    }

    /// Get the length of this value.
    /// - null: 0
    /// - string: UTF-8 codepoint count
    /// - array: element count
    /// - object: key count
    /// - other: error (returns None)
    pub fn length(&self) -> Option<usize> {
        match self {
            Self::Null => Some(0),
            Self::String(s) => Some(s.chars().count()),
            Self::Array(arr) => Some(arr.len()),
            Self::Object(obj) => Some(obj.len()),
            _ => None,
        }
    }

    /// Format this value as JSON string.
    pub fn to_json(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(true) => "true".into(),
            Self::Bool(false) => "false".into(),
            Self::Int(n) => format!("{n}"),
            Self::Float(f) => {
                if f.is_nan() || f.is_infinite() {
                    "null".into() // JSON doesn't support NaN or Infinity
                } else {
                    format!("{f}")
                }
            }
            Self::String(s) => format!("\"{}\"", escape_json_string(s)),
            Self::Array(arr) => {
                let elements: Vec<String> = arr.iter().map(Self::to_json).collect();
                format!("[{}]", elements.join(","))
            }
            Self::Object(obj) => {
                let entries: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| format!("\"{}\":{}", escape_json_string(k), v.to_json()))
                    .collect();
                format!("{{{}}}", entries.join(","))
            }
        }
    }
}

/// jq value equality.
///
/// This is deliberately **not** `#[derive]`d. Deriving would compare the
/// representation, so `Int(1)` and `Float(1.0)` -- two spellings of the same
/// JSON number -- would be unequal, and `1 == 1.0` would evaluate to `false`
/// (jq says `true`). Since `Vec`/`IndexMap` inherit their `PartialEq` from the
/// element type, every builtin routed through equality (`==`, `!=`, array `-`,
/// `contains`, `inside`, `index`, `indices`, `rindex`) picks up this impl.
///
/// Semantics, pinned against jq-1.7.1:
///
/// - Numbers compare by value across representations: `1 == 1.0`,
///   `[1] == [1.0]`, `{"a":1} == {"a":1.0}`.
/// - `Int`/`Int` compares exactly as `i64`, so integers beyond 2^53 keep their
///   precision: `9007199254740993 == 9007199254740992` is `false`, as in jq 1.7.
/// - NaN is never equal to itself. This makes the relation partial (hence
///   `PartialEq` and no `Eq`), and matches jq: `nan == nan` is `false`. It is
///   also why this cannot be written as `compare_values(..) == Equal` -- the
///   evaluators' `compare_values` folds incomparable floats to `Equal`.
/// - `-0.0 == 0` and `-0.0 == 0.0`, as in jq.
/// - Objects compare order-insensitively (`IndexMap`'s own `PartialEq`), as in jq.
///
/// Known divergence: above 2^53 a mixed `Int`/`Float` comparison widens the
/// integer to `f64`, whereas jq 1.7 retains the decimal literal. So
/// `9007199254740993 == 9007199254740992.0` is `true` here and `false` in jq.
/// Every value representable exactly as an `f64` agrees.
impl PartialEq for OwnedValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Int(a), Self::Float(b)) => (*a as f64) == *b,
            (Self::Float(a), Self::Int(b)) => *a == (*b as f64),
            (Self::String(a), Self::String(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            _ => false,
        }
    }
}

/// Escape a string for JSON output.
fn escape_json_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
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
    result
}

impl From<Literal> for OwnedValue {
    fn from(lit: Literal) -> Self {
        match lit {
            Literal::Null => Self::Null,
            Literal::Bool(b) => Self::Bool(b),
            Literal::Int(n) => Self::Int(n),
            Literal::Float(f) => Self::Float(f),
            Literal::String(s) => Self::String(s),
        }
    }
}

impl From<bool> for OwnedValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<i64> for OwnedValue {
    fn from(n: i64) -> Self {
        Self::Int(n)
    }
}

impl From<f64> for OwnedValue {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<String> for OwnedValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&str> for OwnedValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

impl<T: Into<Self>> From<Vec<T>> for OwnedValue {
    fn from(arr: Vec<T>) -> Self {
        Self::Array(arr.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructors() {
        assert_eq!(OwnedValue::null(), OwnedValue::Null);
        assert_eq!(OwnedValue::bool(true), OwnedValue::Bool(true));
        assert_eq!(OwnedValue::int(42), OwnedValue::Int(42));
        assert_eq!(OwnedValue::float(2.5), OwnedValue::Float(2.5));
        assert_eq!(
            OwnedValue::string("hello"),
            OwnedValue::String("hello".into())
        );
    }

    #[test]
    fn test_truthy() {
        assert!(!OwnedValue::Null.is_truthy());
        assert!(!OwnedValue::Bool(false).is_truthy());
        assert!(OwnedValue::Bool(true).is_truthy());
        assert!(OwnedValue::Int(0).is_truthy()); // 0 is truthy in jq!
        assert!(OwnedValue::String(String::new()).is_truthy()); // "" is truthy in jq!
        assert!(OwnedValue::Array(vec![]).is_truthy()); // [] is truthy in jq!
    }

    #[test]
    fn test_type_name() {
        assert_eq!(OwnedValue::Null.type_name(), "null");
        assert_eq!(OwnedValue::Bool(true).type_name(), "boolean");
        assert_eq!(OwnedValue::Int(42).type_name(), "number");
        assert_eq!(OwnedValue::Float(2.5).type_name(), "number");
        assert_eq!(OwnedValue::String(String::new()).type_name(), "string");
        assert_eq!(OwnedValue::Array(vec![]).type_name(), "array");
        assert_eq!(OwnedValue::Object(IndexMap::new()).type_name(), "object");
    }

    #[test]
    fn test_length() {
        assert_eq!(OwnedValue::Null.length(), Some(0));
        assert_eq!(OwnedValue::String("hello".into()).length(), Some(5));
        assert_eq!(OwnedValue::String("héllo".into()).length(), Some(5)); // Unicode
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]).length(),
            Some(2)
        );
        assert_eq!(OwnedValue::Bool(true).length(), None);
        assert_eq!(OwnedValue::Int(42).length(), None);
    }

    #[test]
    fn test_to_json() {
        assert_eq!(OwnedValue::Null.to_json(), "null");
        assert_eq!(OwnedValue::Bool(true).to_json(), "true");
        assert_eq!(OwnedValue::Bool(false).to_json(), "false");
        assert_eq!(OwnedValue::Int(42).to_json(), "42");
        assert_eq!(OwnedValue::Float(2.5).to_json(), "2.5");
        assert_eq!(OwnedValue::String("hello".into()).to_json(), "\"hello\"");
        assert_eq!(
            OwnedValue::String("hello\nworld".into()).to_json(),
            "\"hello\\nworld\""
        );
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)]).to_json(),
            "[1,2]"
        );
    }

    #[test]
    fn test_from_literal() {
        assert_eq!(OwnedValue::from(Literal::Null), OwnedValue::Null);
        assert_eq!(
            OwnedValue::from(Literal::Bool(true)),
            OwnedValue::Bool(true)
        );
        assert_eq!(OwnedValue::from(Literal::Int(42)), OwnedValue::Int(42));
        assert_eq!(
            OwnedValue::from(Literal::Float(2.5)),
            OwnedValue::Float(2.5)
        );
        assert_eq!(
            OwnedValue::from(Literal::String("hello".into())),
            OwnedValue::String("hello".into())
        );
    }

    #[test]
    fn test_collection_constructors() {
        assert_eq!(OwnedValue::array(), OwnedValue::Array(vec![]));
        assert_eq!(
            OwnedValue::array_from(vec![OwnedValue::Int(1), OwnedValue::Int(2)]),
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
        );
        assert_eq!(OwnedValue::object(), OwnedValue::Object(IndexMap::new()));
        let obj = OwnedValue::object_from([("a".to_string(), OwnedValue::Int(1))]);
        assert_eq!(obj.as_object().unwrap().get("a"), Some(&OwnedValue::Int(1)));
    }

    #[test]
    fn test_is_null() {
        assert!(OwnedValue::Null.is_null());
        assert!(!OwnedValue::Bool(false).is_null());
        assert!(!OwnedValue::Int(0).is_null());
    }

    #[test]
    fn test_as_bool() {
        assert_eq!(OwnedValue::Bool(true).as_bool(), Some(true));
        assert_eq!(OwnedValue::Bool(false).as_bool(), Some(false));
        assert_eq!(OwnedValue::Int(1).as_bool(), None);
        assert_eq!(OwnedValue::Null.as_bool(), None);
    }

    #[test]
    fn test_as_i64() {
        assert_eq!(OwnedValue::Int(42).as_i64(), Some(42));
        // A float with an integral value converts to i64...
        assert_eq!(OwnedValue::Float(3.0).as_i64(), Some(3));
        // ...but a fractional float does not.
        assert_eq!(OwnedValue::Float(3.5).as_i64(), None);
        assert_eq!(OwnedValue::String("3".into()).as_i64(), None);
    }

    #[test]
    fn test_as_f64() {
        assert_eq!(OwnedValue::Int(42).as_f64(), Some(42.0));
        assert_eq!(OwnedValue::Float(2.5).as_f64(), Some(2.5));
        assert_eq!(OwnedValue::Bool(true).as_f64(), None);
    }

    #[test]
    fn test_as_str() {
        assert_eq!(OwnedValue::String("hi".into()).as_str(), Some("hi"));
        assert_eq!(OwnedValue::Int(1).as_str(), None);
    }

    #[test]
    fn test_as_array_and_mut() {
        let mut v = OwnedValue::Array(vec![OwnedValue::Int(1)]);
        assert_eq!(v.as_array(), Some(&vec![OwnedValue::Int(1)]));
        v.as_array_mut().unwrap().push(OwnedValue::Int(2));
        assert_eq!(
            v,
            OwnedValue::Array(vec![OwnedValue::Int(1), OwnedValue::Int(2)])
        );
        assert_eq!(OwnedValue::Null.as_array(), None);
        assert_eq!(OwnedValue::Null.as_array_mut(), None);
    }

    #[test]
    fn test_as_object_and_mut() {
        let mut map = IndexMap::new();
        map.insert("a".to_string(), OwnedValue::Int(1));
        let mut v = OwnedValue::Object(map);
        assert_eq!(v.as_object().unwrap().len(), 1);
        v.as_object_mut()
            .unwrap()
            .insert("b".to_string(), OwnedValue::Int(2));
        assert_eq!(v.as_object().unwrap().len(), 2);
        assert_eq!(OwnedValue::Int(1).as_object(), None);
        assert_eq!(OwnedValue::Int(1).as_object_mut(), None);
    }

    #[test]
    fn test_from_primitives() {
        assert_eq!(OwnedValue::from(true), OwnedValue::Bool(true));
        assert_eq!(OwnedValue::from(7i64), OwnedValue::Int(7));
        assert_eq!(OwnedValue::from(1.5f64), OwnedValue::Float(1.5));
        assert_eq!(
            OwnedValue::from(String::from("s")),
            OwnedValue::String("s".into())
        );
        assert_eq!(OwnedValue::from("s"), OwnedValue::String("s".into()));
    }

    #[test]
    fn test_eq_is_numeric_across_int_and_float() {
        // The whole point of the hand-written `PartialEq`: `1 == 1.0` (#156).
        assert_eq!(OwnedValue::Int(1), OwnedValue::Float(1.0));
        assert_eq!(OwnedValue::Float(1.0), OwnedValue::Int(1));
        assert_eq!(OwnedValue::Int(-7), OwnedValue::Float(-7.0));
        assert_ne!(OwnedValue::Int(1), OwnedValue::Float(1.5));
        assert_ne!(OwnedValue::Float(1.5), OwnedValue::Int(1));
        // Same-representation comparisons are unchanged.
        assert_eq!(OwnedValue::Int(2), OwnedValue::Int(2));
        assert_ne!(OwnedValue::Int(2), OwnedValue::Int(3));
        assert_eq!(OwnedValue::Float(2.5), OwnedValue::Float(2.5));
    }

    #[test]
    fn test_eq_int_int_keeps_i64_precision() {
        // Widening both sides to f64 would collapse these two; jq 1.7 keeps
        // integer literal precision here and so do we.
        assert_ne!(
            OwnedValue::Int(9_007_199_254_740_993),
            OwnedValue::Int(9_007_199_254_740_992)
        );
    }

    #[test]
    fn test_eq_nan_is_never_equal() {
        // Matches jq: `nan == nan` is false. This is why equality cannot be
        // expressed as `compare_values(..) == Equal`, which folds NaN to Equal.
        let nan = OwnedValue::Float(f64::NAN);
        assert_ne!(nan, nan.clone());
        assert_ne!(nan, OwnedValue::Int(0));
        // Infinities, by contrast, do compare equal to themselves.
        assert_eq!(
            OwnedValue::Float(f64::INFINITY),
            OwnedValue::Float(f64::INFINITY)
        );
        assert_ne!(
            OwnedValue::Float(f64::INFINITY),
            OwnedValue::Float(f64::NEG_INFINITY)
        );
    }

    #[test]
    fn test_eq_signed_zero() {
        // jq: `-0.0 == 0` and `-0.0 == 0.0` are both true.
        assert_eq!(OwnedValue::Float(-0.0), OwnedValue::Int(0));
        assert_eq!(OwnedValue::Float(-0.0), OwnedValue::Float(0.0));
    }

    #[test]
    fn test_eq_recurses_through_containers() {
        // `Vec`/`IndexMap` inherit element equality, so containers are numeric-aware too.
        assert_eq!(
            OwnedValue::Array(vec![OwnedValue::Int(1)]),
            OwnedValue::Array(vec![OwnedValue::Float(1.0)])
        );
        assert_ne!(
            OwnedValue::Array(vec![OwnedValue::Int(1)]),
            OwnedValue::Array(vec![OwnedValue::Float(1.0), OwnedValue::Int(2)])
        );
        assert_eq!(
            OwnedValue::object_from([("a".to_string(), OwnedValue::Int(1))]),
            OwnedValue::object_from([("a".to_string(), OwnedValue::Float(1.0))])
        );
        // Objects compare order-insensitively, as in jq.
        assert_eq!(
            OwnedValue::object_from([
                ("a".to_string(), OwnedValue::Int(1)),
                ("b".to_string(), OwnedValue::Int(2)),
            ]),
            OwnedValue::object_from([
                ("b".to_string(), OwnedValue::Float(2.0)),
                ("a".to_string(), OwnedValue::Int(1)),
            ])
        );
    }

    #[test]
    fn test_eq_across_types_is_false() {
        // Numbers are only conflated with each other -- never with other types.
        assert_ne!(OwnedValue::Int(1), OwnedValue::String("1".into()));
        assert_ne!(OwnedValue::Int(1), OwnedValue::Bool(true));
        assert_ne!(OwnedValue::Int(0), OwnedValue::Bool(false));
        assert_ne!(OwnedValue::Int(0), OwnedValue::Null);
        assert_ne!(OwnedValue::Float(0.0), OwnedValue::Null);
        assert_ne!(
            OwnedValue::Array(vec![]),
            OwnedValue::Object(IndexMap::new())
        );
    }

    #[test]
    fn test_from_vec() {
        let v: OwnedValue = vec![1i64, 2, 3].into();
        assert_eq!(
            v,
            OwnedValue::Array(vec![
                OwnedValue::Int(1),
                OwnedValue::Int(2),
                OwnedValue::Int(3)
            ])
        );
    }
}
