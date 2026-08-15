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

#[cfg(not(test))]
use alloc::borrow::Cow;
#[cfg(test)]
use std::borrow::Cow;

use crate::jq::OwnedValue;

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

    /// Converts this resolution to jq's [`OwnedValue`], given `text` — the
    /// scalar's original source text (used for the `Str` case, and, for a
    /// `Float` that also passes `is_preservable_float_literal`, to
    /// preserve a whole-number float's spelling through materialization —
    /// issue #918: `2.0`'s decimal point otherwise vanishes via bare
    /// `f64::Display`, since `Int`/`Float`/`Bool`/`Null` carry no source
    /// text of their own to fall back on).
    ///
    /// The single source of truth this crate's three independent
    /// `ResolvedScalar -> OwnedValue` match arms (issue #907) collapse
    /// into: `crate::jq::eval_generic`'s tagged-scalar materialization,
    /// the yq CLI's DOM conversion (`yq_runner.rs`), and the `load()`
    /// builtin's YAML loader (`eval.rs`) all called this out independently
    /// before, which is exactly how the yq-CLI and `load()` copies missed
    /// #918's literal-preservation fix when it landed only in
    /// `eval_generic.rs`.
    #[must_use]
    pub fn to_owned_value(self, text: Cow<'_, str>) -> OwnedValue {
        match self {
            Self::Null => OwnedValue::Null,
            Self::Bool(b) => OwnedValue::Bool(b),
            Self::Int(n) => OwnedValue::Int(n),
            Self::Float(_) if is_preservable_float_literal(&text) => {
                OwnedValue::from_number_literal(&text)
            }
            Self::Float(f) => OwnedValue::Float(f),
            Self::Str => OwnedValue::String(text.into_owned()),
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

/// True if `s` is valid JSON number syntax (RFC 8259) end to end: optional
/// leading `-`, then `0` or a non-zero-led digit run, optional `.`-fraction
/// (at least one digit), optional exponent (at least one digit).
///
/// This module's own numeric [`ResolvedScalar`] variants "carry the parsed
/// value... emitters must use the carried value, never echo the source
/// text" (see the type's doc comment) precisely because YAML's core-schema
/// number grammar is looser than JSON's: a leading-dot float (`.5`), a
/// leading `+` (`+.5`), or a leading zero followed by more digits (`007.5`)
/// all resolve here but are not valid JSON number text, and hex/octal ints
/// (`0x2A`) obviously aren't either. A caller that ignores that warning and
/// hands such text to something expecting JSON syntax — as
/// `OwnedValue::NumberLiteral`'s downstream reindexing bridge does — gets a
/// value silently misclassified as a parse error instead of a number
/// (confirmed via the `tag` builtin, which maps that error node to
/// `!!null` for a scalar that plainly has a `!!float` tag) rather than
/// erroring loudly. This predicate is how [`super::light`]'s
/// `number_literal()` override decides which literals are safe to echo.
///
/// This grammar duplicates `crate::json::validate`'s `Validator::validate_number`
/// (reachable via the public `crate::json::validate::validate`) — hand-rolled
/// here rather than reused to avoid adding a `yaml`-to-`json` module
/// dependency for this fix; tracked as a follow-up dedup opportunity (#957).
#[must_use]
fn is_json_number_syntax(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    if bytes.first() == Some(&b'-') {
        i += 1;
    }
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while matches!(bytes.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
        }
        _ => return false,
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == bytes.len()
}

/// The maximum count of ASCII digit characters (integer + fraction part
/// combined) [`is_preservable_float_literal`] allows through. 17 significant
/// decimal digits is the documented bound beyond which distinct `f64`
/// values can round to the same printed digits (and, symmetrically, below
/// which every `f64` round-trips uniquely) — more digits than that means
/// the source text already carries more precision than the `f64` it parsed
/// to actually holds, so echoing it back verbatim would silently overstate
/// precision the parse step already discarded.
const MAX_PRESERVABLE_FLOAT_DIGITS: usize = 17;

/// True if `s` is safe *and worthwhile* to preserve verbatim as a
/// document-sourced float's `NumberLiteral` text — used by both YAML's
/// [`super::light`] `number_literal()` override (a plain scalar) and its
/// `!!float`-tag resolution path (an explicitly-tagged one), so the two
/// don't drift into re-answering this question differently.
///
/// Requires, beyond [`is_json_number_syntax`]:
/// - **A literal `.`.** A bare digit run only resolves to
///   [`Float`](ResolvedScalar::Float) when it overflows `i64`
///   (`parse_int_or_float`'s fallback) — that's not a value someone spelled
///   as a float, it's an integer too big for `i64`, and echoing its raw
///   digits back verbatim would silently claim more precision than the
///   `f64` it parsed to can actually hold (the same concern the digit-count
///   cap below targets, just via a different trigger).
/// - **No exponent.** `format_number_jq_compat` — the formatter this text
///   eventually flows through — re-normalizes exponent notation into a
///   different spelling (uppercase `E`, forced sign, no `e0` elimination)
///   rather than preserving it, so there is nothing gained by echoing
///   exponent text through that path; it gets overwritten anyway. This also
///   sidesteps that formatter's separate, pre-existing loss of `-0.0`'s
///   sign on the exponent branch, which the source-text-echo path can't
///   reach without an exponent in play.
/// - **At most [`MAX_PRESERVABLE_FLOAT_DIGITS`] significant digits.**
///
/// Deliberately conservative: legal YAML core-schema spellings that don't
/// meet this bar (`+2.0`, a trailing-dot `1.`) fall through to the
/// pre-existing `as_f64` path unchanged rather than being force-fit here,
/// leaving the #918 symptom open for that narrower spelling class — see
/// the issue tracker for the follow-up.
#[must_use]
pub(super) fn is_preservable_float_literal(s: &str) -> bool {
    s.contains('.')
        && !s.contains(['e', 'E'])
        && s.bytes().filter(u8::is_ascii_digit).count() <= MAX_PRESERVABLE_FLOAT_DIGITS
        && is_json_number_syntax(s)
}

/// Force-resolves a scalar's value under an explicit YAML tag.
///
/// Handles the 5 core-schema tags (`!!str`, `!!null`, `!!bool`, `!!int`,
/// `!!float`), matching real `yq`'s behavior of applying tag coercion
/// *regardless of quoting style* — even a quoted `!!int "5"` becomes the
/// number `5`, not the string `"5"`. Returns `None` for any other tag (a
/// custom tag, `!!seq`, `!!map`, `!!set`, `!!omap`, verbatim, or no tag at
/// all), meaning "no override — resolve naturally instead" (`resolve_plain`
/// for a plain scalar, `Str` for quoted/block).
///
/// Divergence from `yq`: content that cannot be coerced to the forced numeric
/// type (`!!int "abc"`, `!!int -0x2A`) resolves to [`Str`](ResolvedScalar::Str)
/// here rather than reproducing `yq`'s behavior for that input, which is to
/// accept it at parse time and then crash formatting JSON output
/// (`strconv.ParseInt: parsing "abc": invalid syntax`). This loader is
/// non-validating by design and absorbs what it cannot make sense of rather
/// than erroring — see `docs/compliance/yaml/limitations.md`.
///
/// `!!int`/`!!float` reuse [`resolve_plain`]'s core-schema numeric grammar
/// (so `!!int 0x2A` is `42`, matching `yq`) rather than a bare
/// `str::parse`, and `!!float` additionally accepts int-shaped text
/// (`!!float 5` is `5.0`, matching `yq`) by widening a resolved `Int` —
/// including a hex/octal one (`!!float 0x2A` is `42.0`), where real `yq`
/// instead crashes: its float parser, unlike its int parser, does not
/// understand the `0x`/`0o` prefix.
///
/// `!!bool` matches the classic YAML 1.1 word list
/// (`y`/`yes`/`true`/`on`, and their `n`/`no`/`false`/`off` negatives)
/// case-insensitively, which is *broader* than `resolve_plain`'s core
/// schema `true`/`True`/`TRUE`/`false`/`False`/`FALSE` — an explicit `!!bool`
/// tag opts back into the legacy spellings the core schema otherwise
/// excludes to avoid the Norway problem. Anything else (`t`, `1`, `xyz`, …)
/// resolves to `false`, matching `yq`'s zero-value fallback.
#[must_use]
pub fn resolve_tagged(text: &str, tag: &str) -> Option<ResolvedScalar> {
    match tag {
        "!!str" => Some(ResolvedScalar::Str),
        "!!null" => Some(ResolvedScalar::Null),
        "!!bool" => Some(ResolvedScalar::Bool(matches!(
            text.to_ascii_lowercase().as_str(),
            "y" | "yes" | "true" | "on"
        ))),
        "!!int" => Some(match resolve_plain(text) {
            ResolvedScalar::Int(n) => ResolvedScalar::Int(n),
            _ => ResolvedScalar::Str,
        }),
        "!!float" => Some(match resolve_plain(text) {
            ResolvedScalar::Float(f) => ResolvedScalar::Float(f),
            ResolvedScalar::Int(n) => ResolvedScalar::Float(n as f64),
            _ => ResolvedScalar::Str,
        }),
        _ => None,
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

    // Every case below was checked against real `yq` v4.53.3
    // (`echo 'a: !!TAG CONTENT' | yq -o=json -`) while writing `resolve_tagged`.

    #[test]
    fn tagged_str_forces_string_regardless_of_content() {
        assert_eq!(resolve_tagged("1", "!!str"), Some(Str));
        assert_eq!(resolve_tagged("true", "!!str"), Some(Str));
        assert_eq!(resolve_tagged("null", "!!str"), Some(Str));
        assert_eq!(resolve_tagged("", "!!str"), Some(Str));
    }

    #[test]
    fn tagged_null_forces_null_regardless_of_content() {
        assert_eq!(resolve_tagged("foo", "!!null"), Some(Null));
        assert_eq!(resolve_tagged("", "!!null"), Some(Null));
        assert_eq!(resolve_tagged("~", "!!null"), Some(Null));
    }

    #[test]
    fn tagged_bool_accepts_the_yaml_11_word_list_case_insensitively() {
        for s in [
            "y", "Y", "yes", "Yes", "YES", "true", "True", "TrUe", "on", "On", "ON",
        ] {
            assert_eq!(
                resolve_tagged(s, "!!bool"),
                Some(Bool(true)),
                "input: {s:?}"
            );
        }
        for s in [
            "n",
            "no",
            "No",
            "NO",
            "false",
            "False",
            "off",
            "Off",
            "OFF",
            "t",
            "T",
            "1",
            "0",
            "randomjunk",
            "",
        ] {
            assert_eq!(
                resolve_tagged(s, "!!bool"),
                Some(Bool(false)),
                "input: {s:?}"
            );
        }
    }

    #[test]
    fn tagged_int_reuses_core_schema_numeric_grammar() {
        assert_eq!(resolve_tagged("5", "!!int"), Some(Int(5)));
        assert_eq!(resolve_tagged("-5", "!!int"), Some(Int(-5)));
        assert_eq!(resolve_tagged("0x2A", "!!int"), Some(Int(42)));
        // Content that doesn't parse as an int falls back to Str rather than
        // reproducing yq's marshal-time crash for this input.
        assert_eq!(resolve_tagged("abc", "!!int"), Some(Str));
        assert_eq!(resolve_tagged("3.5", "!!int"), Some(Str));
        assert_eq!(resolve_tagged("", "!!int"), Some(Str));
        assert_eq!(resolve_tagged("-0x2A", "!!int"), Some(Str));
    }

    #[test]
    fn tagged_float_widens_int_shaped_text_and_falls_back_on_failure() {
        assert_eq!(resolve_tagged("3", "!!float"), Some(Float(3.0)));
        assert_eq!(resolve_tagged("3.5", "!!float"), Some(Float(3.5)));
        assert_eq!(resolve_tagged("abc", "!!float"), Some(Str));
        // Widening reuses resolve_plain's Int arm uniformly, so a hex int
        // widens too - real `yq`'s plain float parser cannot read "0x2A" and
        // crashes formatting output for it, one of the divergences this
        // function's doc comment calls out.
        assert_eq!(resolve_tagged("0x2A", "!!float"), Some(Float(42.0)));
    }

    #[test]
    fn untagged_and_non_core_schema_tags_return_none() {
        // No override for a custom tag, a collection tag, or no tag at all -
        // the caller falls back to natural resolution.
        for tag in [
            "!custom",
            "!!set",
            "!!omap",
            "!!map",
            "!!seq",
            "!<tag:x,2000:y>",
        ] {
            assert_eq!(resolve_tagged("1", tag), None, "tag: {tag}");
        }
    }

    #[test]
    fn json_number_syntax_accepts_full_rfc_8259_grammar() {
        // Its own doc comment claims full RFC 8259 support, including the
        // exponent form - `is_preservable_float_literal` (its only current
        // caller) never actually routes exponent text through it, since it
        // rejects those upstream for an unrelated reason (`format_number_
        // jq_compat` doesn't preserve YAML's exponent spelling), so this
        // exercises the claim directly rather than through that caller.
        for s in [
            "0", "-0", "42", "-42", "0.0", "2.0", "-2.5", "0.50", "1e10", "1E10", "1e+10", "1e-10",
            "-1.5e-3", "0e0",
        ] {
            assert!(is_json_number_syntax(s), "expected valid: {s:?}");
        }
    }

    #[test]
    fn json_number_syntax_rejects_yaml_legal_json_illegal_spellings() {
        for s in [
            "",      // empty
            "-",     // sign with no digits
            ".5",    // leading dot
            "+2.0",  // leading plus
            "007",   // leading zero, more digits
            "007.5", // leading zero, more digits, with fraction
            "1.",    // trailing dot, no fraction digit
            "2.5e",  // exponent marker, no digits
            "2.5e+", // exponent sign, no digits
            "0x2A",  // hex
            "1e",    // bare exponent marker
            "1.2.3", // trailing garbage
            "1 ",    // trailing whitespace
        ] {
            assert!(!is_json_number_syntax(s), "expected invalid: {s:?}");
        }
    }

    #[test]
    fn preservable_float_literal_requires_a_dot_and_rejects_exponents() {
        // The core #918 case: a plain decimal float with a literal `.`.
        for s in ["2.0", "-2.0", "0.5", "3.140", "0.0"] {
            assert!(
                is_preservable_float_literal(s),
                "expected preservable: {s:?}"
            );
        }
        // No dot at all - either a plain int, or (per #953) an i64-overflow
        // integer that only resolves to Float as a fallback; echoing its
        // digits verbatim would overstate the f64's actual precision.
        for s in ["2", "-2", "99999999999999999999"] {
            assert!(!is_preservable_float_literal(s), "expected rejected: {s:?}");
        }
        // Exponent form - format_number_jq_compat doesn't preserve this
        // spelling, so there's nothing to gain by echoing it (see #918's
        // eval_generic.rs comment for the -0.0 sign-loss this also avoids).
        for s in ["2e2", "-0e10", "1.5e-3"] {
            assert!(!is_preservable_float_literal(s), "expected rejected: {s:?}");
        }
        // JSON-unsafe spellings, deliberately out of scope (#954).
        for s in [".5", "+2.0", "1."] {
            assert!(!is_preservable_float_literal(s), "expected rejected: {s:?}");
        }
    }

    #[test]
    fn preservable_float_literal_rejects_beyond_the_digit_cap() {
        let just_over = "1.".to_string() + &"1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS);
        assert!(!is_preservable_float_literal(&just_over));
        let at_cap = "1.".to_string() + &"1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS - 1);
        assert!(is_preservable_float_literal(&at_cap));
    }
}
