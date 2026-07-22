//! Plain-scalar type resolution for the YAML 1.2 core schema.
//!
//! This module is the single source of truth for deciding whether a *plain*
//! (unquoted) YAML scalar is a null, bool, int, float, or string, per the
//! [YAML 1.2 core schema](https://yaml.org/spec/1.2.2/#103-core-schema).
//! It is used by tag inference, the YAML→JSON transcoders, the typed getters
//! (`as_i64`, `as_bool`, …), and the yq CLI's DOM conversion, so those paths
//! cannot drift apart (issue #226).
//!
//! Callers are responsible for gating: only plain scalars resolve. Quoted and
//! block scalars are always strings and must not be passed here.
//!
//! Deliberate deviations from `yq` (which resolves YAML 1.1 legacy forms via
//! go-yaml): underscored numbers (`1_000`), uppercase base prefixes (`0X2A`),
//! binary (`0b101`), and signed hex/octal (`-0x2A`) all stay strings here, as
//! the 1.2 core schema requires. Hex/octal that overflows `i64` also stays a
//! string (`yq` errors on its own JSON output for such values). See
//! `docs/compliance/yaml/1.2.md` for the full table.
//!
//! # Examples
//!
//! ```
//! use succinctly::yaml::{resolve_plain, ResolvedScalar};
//!
//! assert_eq!(resolve_plain("Null"), ResolvedScalar::Null);
//! assert_eq!(resolve_plain("0x2A"), ResolvedScalar::Int(42));
//! assert_eq!(resolve_plain(".5"), ResolvedScalar::Float(0.5));
//! // Bare `nan`/`inf` require the leading dot in 1.2 core; these are strings.
//! assert_eq!(resolve_plain("nan"), ResolvedScalar::Str);
//! assert_eq!(resolve_plain("1_000"), ResolvedScalar::Str);
//! ```

/// The resolved type (and parsed value) of a plain YAML scalar.
///
/// Numeric variants carry the parsed value because some spellings (`0x2A`)
/// cannot be re-parsed by the consumer with `str::parse` — emitters must use
/// the carried value, never echo the source text.
///
/// Non-finite `Float`s arise only from the explicit `.inf`/`.nan` family;
/// numeric syntax that overflows to infinity (`1e999`) resolves to [`Str`],
/// matching both the 1.2 core schema boundary and go-yaml's behaviour.
///
/// [`Str`]: ResolvedScalar::Str
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedScalar {
    /// `null`, `Null`, `NULL`, `~`, or the empty string.
    Null,
    /// `true`/`True`/`TRUE` or `false`/`False`/`FALSE`.
    Bool(bool),
    /// Decimal (`42`, `+42`, `-7`), hex (`0x2A`), or octal (`0o52`) integer.
    Int(i64),
    /// Finite float (`3.14`, `.5`, `1e-2`) or the `.inf`/`.nan` family.
    Float(f64),
    /// Anything else — including YAML 1.1 legacy forms (`yes`, `1_000`,
    /// `0b101`) and bare `nan`/`inf`/`Infinity`.
    Str,
}

impl ResolvedScalar {
    /// Returns the YAML tag for this resolution (`"!!int"`, `"!!str"`, …).
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Null => "!!null",
            Self::Bool(_) => "!!bool",
            Self::Int(_) => "!!int",
            Self::Float(_) => "!!float",
            Self::Str => "!!str",
        }
    }

    /// Returns the jq-style type name for this resolution (`"number"`, …).
    #[must_use]
    pub fn type_name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Int(_) | Self::Float(_) => "number",
            Self::Str => "string",
        }
    }
}

/// Resolves a plain (unquoted) YAML scalar under the 1.2 core schema.
///
/// Dispatches on the first byte so that non-matching scalars (the common
/// case in the transcode hot path) exit after at most a couple of byte
/// comparisons; full-string comparisons and numeric parses only run inside
/// the arm that the first byte selects.
#[must_use]
#[inline(always)]
pub fn resolve_plain(s: &str) -> ResolvedScalar {
    let bytes = s.as_bytes();
    let Some(&first) = bytes.first() else {
        return ResolvedScalar::Null;
    };
    match first {
        b'n' => keyword(s == "null", ResolvedScalar::Null),
        b'N' => keyword(s == "Null" || s == "NULL", ResolvedScalar::Null),
        b'~' => keyword(bytes.len() == 1, ResolvedScalar::Null),
        b't' => keyword(s == "true", ResolvedScalar::Bool(true)),
        b'T' => keyword(s == "True" || s == "TRUE", ResolvedScalar::Bool(true)),
        b'f' => keyword(s == "false", ResolvedScalar::Bool(false)),
        b'F' => keyword(s == "False" || s == "FALSE", ResolvedScalar::Bool(false)),
        b'.' => resolve_dot(s),
        b'+' | b'-' => resolve_signed(s, bytes),
        b'0'..=b'9' => resolve_number(s, bytes),
        _ => ResolvedScalar::Str,
    }
}

#[inline(always)]
fn keyword(matched: bool, resolved: ResolvedScalar) -> ResolvedScalar {
    if matched {
        resolved
    } else {
        ResolvedScalar::Str
    }
}

/// Resolves scalars starting with `.`: the `.inf`/`.nan` family, else a
/// leading-dot float such as `.5`.
#[inline(always)]
fn resolve_dot(s: &str) -> ResolvedScalar {
    match s {
        ".inf" | ".Inf" | ".INF" => ResolvedScalar::Float(f64::INFINITY),
        ".nan" | ".NaN" | ".NAN" => ResolvedScalar::Float(f64::NAN),
        _ => parse_float(s),
    }
}

/// Resolves scalars starting with `+` or `-`: signed infinities, else a
/// signed number. Signed hex/octal (`-0x2A`) is not core schema and falls
/// through the decimal parses to `Str`.
#[inline(always)]
fn resolve_signed(s: &str, bytes: &[u8]) -> ResolvedScalar {
    match bytes.get(1) {
        Some(b'.') => match s {
            "+.inf" | "+.Inf" | "+.INF" => ResolvedScalar::Float(f64::INFINITY),
            "-.inf" | "-.Inf" | "-.INF" => ResolvedScalar::Float(f64::NEG_INFINITY),
            // `-.5` / `+.5` are floats; `.nan` takes no sign, so `-.nan`
            // fails the parse and resolves to `Str`.
            _ => parse_float(s),
        },
        Some(b'0'..=b'9') => parse_int_or_float(s),
        // `+inf`, `-_1`, a bare sign, … — never numeric in the core schema.
        _ => ResolvedScalar::Str,
    }
}

/// Resolves scalars starting with a digit: `0x`/`0o` based integers, else a
/// decimal int or float.
#[inline(always)]
fn resolve_number(s: &str, bytes: &[u8]) -> ResolvedScalar {
    if bytes[0] == b'0' && bytes.len() > 2 {
        match bytes[1] {
            b'x' => return parse_radix(&s[2..], 16),
            b'o' => return parse_radix(&s[2..], 8),
            _ => {}
        }
    }
    parse_int_or_float(s)
}

/// Parses the digit part of a `0x`/`0o` scalar.
///
/// The core schema allows no sign inside based integers, but
/// `i64::from_str_radix` accepts a leading `+`/`-`, so reject those before
/// delegating. Invalid digits and `i64` overflow both resolve to `Str`.
#[inline(always)]
fn parse_radix(digits: &str, radix: u32) -> ResolvedScalar {
    if matches!(digits.as_bytes().first(), None | Some(b'+' | b'-')) {
        return ResolvedScalar::Str;
    }
    match i64::from_str_radix(digits, radix) {
        Ok(n) => ResolvedScalar::Int(n),
        Err(_) => ResolvedScalar::Str,
    }
}

#[inline(always)]
fn parse_int_or_float(s: &str) -> ResolvedScalar {
    if let Ok(n) = s.parse::<i64>() {
        return ResolvedScalar::Int(n);
    }
    parse_float(s)
}

/// Parses a general float, requiring a finite result.
///
/// The finite guard is what keeps Rust's over-accepting `f64` parser inside
/// the core schema: overflow like `1e999` is rejected (go-yaml likewise
/// rejects it), while underflow like `1e-999` resolves to `Float(0.0)` —
/// exactly go-yaml's accept/reject boundary. The spellings `inf`/`nan`/
/// `Infinity` never reach this function (first-byte dispatch), and signed
/// forms like `+inf` that do reach it parse non-finite and are rejected here.
#[inline(always)]
fn parse_float(s: &str) -> ResolvedScalar {
    match s.parse::<f64>() {
        Ok(f) if f.is_finite() => ResolvedScalar::Float(f),
        _ => ResolvedScalar::Str,
    }
}

/// Returns true if a plain scalar could resolve to null or bool at all.
///
/// A cheap pre-filter for callers that only need the null/bool answer
/// (`is_null`, `is_falsy`, `as_bool`): scalars starting with a digit, sign,
/// or dot can only be numeric or string, so those callers can skip the
/// numeric parses `resolve_plain` would run just to conclude "neither".
#[must_use]
#[inline(always)]
pub fn could_be_null_or_bool(s: &str) -> bool {
    !matches!(s.as_bytes().first(), Some(b'0'..=b'9' | b'+' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ResolvedScalar::{Bool, Float, Int, Null, Str};

    #[track_caller]
    fn assert_resolves(input: &str, expected: ResolvedScalar) {
        assert_eq!(resolve_plain(input), expected, "input: {input:?}");
    }

    #[test]
    fn null_spellings() {
        for s in ["null", "Null", "NULL", "~", ""] {
            assert_resolves(s, Null);
        }
        // Mixed case and other 1.1-isms stay strings.
        for s in ["NuLl", "nULL", "~x", "nil", "none"] {
            assert_resolves(s, Str);
        }
    }

    #[test]
    fn bool_spellings() {
        for s in ["true", "True", "TRUE"] {
            assert_resolves(s, Bool(true));
        }
        for s in ["false", "False", "FALSE"] {
            assert_resolves(s, Bool(false));
        }
        // The Norway problem stays solved; mixed case stays a string.
        for s in ["yes", "no", "on", "off", "y", "n", "TrUe", "FALSe"] {
            assert_resolves(s, Str);
        }
    }

    #[test]
    fn decimal_ints() {
        assert_resolves("0", Int(0));
        assert_resolves("42", Int(42));
        assert_resolves("+42", Int(42));
        assert_resolves("-7", Int(-7));
        assert_resolves("-0", Int(0));
        assert_resolves("052", Int(52)); // decimal with leading zero, not octal
        assert_resolves("9223372036854775807", Int(i64::MAX));
        assert_resolves("-9223372036854775808", Int(i64::MIN));
    }

    #[test]
    fn based_ints() {
        assert_resolves("0x2A", Int(42));
        assert_resolves("0x2a", Int(42));
        assert_resolves("0o52", Int(42));
        assert_resolves("0xDEADBEEF", Int(0xDEAD_BEEF));
    }

    #[test]
    fn based_int_rejections() {
        // Not core schema: empty digits, invalid digits, signs (either side
        // of the prefix), uppercase prefixes, binary, underscores.
        for s in [
            "0x", "0o", "0x+2A", "0o+52", "0o-52", "-0x2A", "+0x2A", "0X2A", "0O52", "0b101",
            "0xG", "0o8", "0x2A.5", "0x_2A",
        ] {
            assert_resolves(s, Str);
        }
        // i64 overflow stays a string (yq errors on its own JSON output here).
        assert_resolves("0xFFFFFFFFFFFFFFFF", Str);
    }

    #[test]
    fn floats() {
        assert_resolves("2.75", Float(2.75));
        assert_resolves("-1.5", Float(-1.5));
        assert_resolves(".5", Float(0.5));
        assert_resolves("+.5", Float(0.5));
        assert_resolves("-.5", Float(-0.5));
        assert_resolves("5.", Float(5.0));
        assert_resolves("1e2", Float(100.0));
        assert_resolves("1E2", Float(100.0));
        assert_resolves("1e-2", Float(0.01));
        // Underflow rounds to zero, matching go-yaml.
        assert_resolves("1e-999", Float(0.0));
        // Decimal i64 overflow falls through to a finite float.
        assert_resolves("9223372036854775808", Float(9_223_372_036_854_775_808.0));
    }

    #[test]
    fn dot_special_floats() {
        for s in [".inf", ".Inf", ".INF", "+.inf", "+.Inf", "+.INF"] {
            assert_resolves(s, Float(f64::INFINITY));
        }
        for s in ["-.inf", "-.Inf", "-.INF"] {
            assert_resolves(s, Float(f64::NEG_INFINITY));
        }
        for s in [".nan", ".NaN", ".NAN"] {
            assert!(
                matches!(resolve_plain(s), Float(f) if f.is_nan()),
                "input: {s:?}"
            );
        }
    }

    #[test]
    fn special_float_rejections() {
        // Bare (dotless) spellings are strings in 1.2 core; Rust's f64
        // parser would accept several of these, hence the explicit guard.
        for s in [
            "nan",
            "NaN",
            "NAN",
            "inf",
            "Inf",
            "INF",
            "+inf",
            "-inf",
            "Infinity",
            "-Infinity",
            "infinity",
        ] {
            assert_resolves(s, Str);
        }
        // Sign/case variants outside the spec list.
        for s in ["-.nan", "+.nan", ".iNf", ".nAn", ".INF2", ".NAN2", ".infx"] {
            assert_resolves(s, Str);
        }
        // Float overflow to infinity is rejected (go-yaml boundary).
        assert_resolves("1e999", Str);
        assert_resolves("-1e999", Str);
    }

    #[test]
    fn strings() {
        for s in [
            "hello", "1_000", "1__0", "1_", "_1", "1,5", "1.2.3", "e5", "+", "-", ".", "-.",
            "0.0.0", "12abc", " 42", "42 ",
        ] {
            assert_resolves(s, Str);
        }
    }

    #[test]
    fn tag_names() {
        assert_eq!(Null.tag(), "!!null");
        assert_eq!(Bool(true).tag(), "!!bool");
        assert_eq!(Int(1).tag(), "!!int");
        assert_eq!(Float(1.0).tag(), "!!float");
        assert_eq!(Str.tag(), "!!str");
    }

    #[test]
    fn type_names() {
        assert_eq!(Null.type_name(), "null");
        assert_eq!(Bool(false).type_name(), "boolean");
        assert_eq!(Int(1).type_name(), "number");
        assert_eq!(Float(1.0).type_name(), "number");
        assert_eq!(Str.type_name(), "string");
    }
}
