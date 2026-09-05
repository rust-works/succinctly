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
use alloc::{borrow::Cow, format, string::String, string::ToString};
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
    /// Three independent `ResolvedScalar -> OwnedValue` match arms (issue
    /// #907) collapse into this one: `crate::jq::eval_generic`'s
    /// tagged-scalar materialization, the yq CLI's DOM conversion
    /// (`yq_runner.rs`), and the `load()` builtin's YAML loader
    /// (`eval.rs`) all had their own copy before, which is exactly how the
    /// yq-CLI and `load()` copies missed #918's literal-preservation fix
    /// when it landed only in `eval_generic.rs`. Two further siblings that
    /// produce JSON *text* directly rather than an `OwnedValue`
    /// (`light.rs`'s `write_resolved_scalar_as_json`/
    /// `stream_resolved_scalar_as_json`) are a different output type and
    /// weren't folded in here — see the #907 follow-up issue.
    ///
    /// A tag-forced float (`!!float 2`) stays a bare
    /// [`OwnedValue::Float`] here, keeping the value's *text* out of it.
    /// Only the one caller whose round trip cannot carry float-ness any
    /// other way needs the re-spelling — see
    /// [`to_owned_value_for_json_bridge`](Self::to_owned_value_for_json_bridge),
    /// and that function's doc comment for why giving it to every caller
    /// is wrong.
    #[must_use]
    pub fn to_owned_value(self, text: Cow<'_, str>) -> OwnedValue {
        self.to_owned_value_with_bridge_respelling(text, false)
    }

    /// [`to_owned_value`](Self::to_owned_value) for a value about to cross
    /// `yq_runner.rs`'s `evaluate_input` reindex bridge, which re-spells a
    /// tag-forced float so its type survives that round trip (#1176).
    ///
    /// A *finite* float whose text a plain reader would type as something
    /// other than `!!float` — i.e. one that is only a float because an
    /// explicit tag forced it (`!!float 2`, `!!float 0x2A`) — has nowhere
    /// left to carry that float-ness: neither spelling gate in
    /// [`to_owned_value`](Self::to_owned_value) accepts an integer-shaped
    /// mantissa, so it arrives at that bridge as a bare
    /// [`OwnedValue::Float`], serializes as `2`, and reparses as an `Int`.
    /// Reproducible with no `-i` at all, via `--slurp '.[0]'` on
    /// `a: !!float 2`. Re-spelling with a forced decimal point puts the
    /// type back into the literal itself, where the round trip preserves
    /// it.
    ///
    /// **Only that one caller.** `evaluate_input` is the sole bridge in
    /// the codebase that hardcodes `to_json_for_reindex::<JqSemantics>`
    /// (deliberately — `YqSemantics` there re-breaks #978's
    /// JSON-sourced-`1e2`-renders-as-`100` rule), and jq's plain-`Float`
    /// fallback is the one that drops the point. Every other bridge is
    /// `S`-gated and spells a bare `Float` with `format_float_yq` under
    /// `YqSemantics`, whose everyday-magnitude branch is
    /// `format_float_with_fraction` (#2438) -- and a tag-forced float past
    /// that magnitude threshold has already picked up its own spelling from
    /// [`OwnedValue::from_document_float`] in the arm below -- so a
    /// tag-forced float survives those untouched either way. Handing the re-spelling to
    /// `eval_generic`'s cursor materialization and to `load()` as well
    /// costs real oracle fidelity: their values reach string-producing
    /// builtins, where real yq prints the scalar's own text, so
    /// `!!float 2 | tostring` would answer `2.0` against yq's `2` (and
    /// likewise `@yaml`, `@props`) — pinned by
    /// `test_explicit_float_tag_respelling_stays_out_of_string_builtins_1090`
    /// in `tests/yq_cli_tests.rs`.
    ///
    /// `.inf`/`.nan` stay on the bare-`Float` path: they have no decimal
    /// spelling, and `resolve_plain` already types them `!!float`, so they
    /// never need this anyway.
    #[must_use]
    pub fn to_owned_value_for_json_bridge(self, text: Cow<'_, str>) -> OwnedValue {
        self.to_owned_value_with_bridge_respelling(text, true)
    }

    fn to_owned_value_with_bridge_respelling(
        self,
        text: Cow<'_, str>,
        respell_tag_forced_float: bool,
    ) -> OwnedValue {
        match self {
            Self::Null => OwnedValue::Null,
            Self::Bool(b) => OwnedValue::Bool(b),
            Self::Int(n) => OwnedValue::Int(n),
            Self::Float(_) if is_preservable_float_literal(&text) => {
                OwnedValue::from_number_literal(&text)
            }
            Self::Float(f) => match preservable_float_literal_text(&text) {
                Some(normalized) => OwnedValue::from_number_literal(&normalized),
                // The tag-forced-float re-spelling (#1176) -- gated to
                // `to_owned_value_for_json_bridge`'s single caller, whose
                // doc comment carries the full reasoning and the cost of
                // applying it any wider.
                None if respell_tag_forced_float
                    && f.is_finite()
                    && needs_explicit_float_tag(&text) =>
                {
                    OwnedValue::from_number_literal(&super::format_float_with_fraction(f))
                }
                // #2438: same document boundary as `to_owned_at_depth`'s own
                // bare-float arm (`src/jq/eval_generic.rs`) -- an explicitly
                // tagged `!!float 100000000000000000000` reaches `OwnedValue`
                // here instead, and needs the identical provenance record.
                None => OwnedValue::from_document_float(f),
            },
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
/// leading `+` (`+.5`), a leading zero followed by more digits (`007.5`),
/// or a bare trailing dot (`1.`) all resolve here but are not valid JSON
/// number text, and hex/octal ints (`0x2A`) obviously aren't either. A
/// caller that ignores that warning and hands such text to something
/// expecting JSON syntax — as `OwnedValue::NumberLiteral`'s downstream
/// reindexing bridge does — gets a value silently misclassified as a parse
/// error instead of a number (confirmed via the `tag` builtin, which maps
/// that error node to `!!null` for a scalar that plainly has a `!!float`
/// tag) rather than erroring loudly. This predicate is how
/// [`super::light`]'s `number_literal()` override decides which literals
/// are safe to echo *verbatim*; [`preservable_float_literal_text`] widens
/// beyond it by normalizing first, for text this rejects outright.
///
/// This grammar is the same one `crate::json::validate::is_valid_number`
/// implements (extracted from this exact function, #957/#966) — delegate
/// rather than hand-roll a second copy.
#[must_use]
fn is_json_number_syntax(s: &str) -> bool {
    crate::json::validate::is_valid_number(s.as_bytes())
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
/// - **A literal `.` or an exponent (`e`/`E`).** A bare digit run only
///   resolves to [`Float`](ResolvedScalar::Float) when it overflows `i64`
///   (`parse_int_or_float`'s fallback) — that's not a value someone spelled
///   as a float, it's an integer too big for `i64`, and echoing its raw
///   digits back verbatim would silently claim more precision than the
///   `f64` it parsed to can actually hold (the same concern the digit-count
///   cap below targets, just via a different trigger). Either a decimal
///   point or an exponent is unambiguous float syntax on its own (`1e2`
///   has no `.` but is still a float, not an overflowed integer), so
///   either is sufficient here.
/// - **At most [`MAX_PRESERVABLE_FLOAT_DIGITS`] *significant* digits in the
///   mantissa** ([`significant_mantissa_digit_count`]) -- every digit from
///   the mantissa's first nonzero digit onward, the same "significant
///   figures" rule scientific notation itself uses: a leading zero run
///   (`0.007`, magnitude, not precision) never counts, however long it is,
///   the same way an *integer* literal's own leading zeros wouldn't. Two
///   consequences, both #1211:
///   - A **zero-valued mantissa** has no nonzero digit at all, so it has
///     zero significant digits by this same rule -- always within the cap,
///     needing no separate carve-out. `0.00000000000000000000e-400`
///     (issue #1211's own repro) stays preserved at any length, matching
///     real yq; before this fix, counting every digit *including* the
///     leading zeros silently fell back to a lossy `0` past 17 of them.
///   - A mantissa with a **long leading-zero run before a handful of real
///     digits** (`0.000000000000000012345678901234567`, 17 significant
///     digits behind 17 leading zeros) is preserved too, on the identical
///     reasoning -- confirmed live against the pinned oracle; the raw-count
///     predecessor of this check rejected it purely because of magnitude,
///     the same miscount #1211 reported, just needing one nonzero digit
///     instead of zero to trigger.
///
///   Exponent digits carry no precision (they're a magnitude, not a
///   significand) and must not count toward this cap either -- an earlier
///   version of this predicate counted every digit in `s` including the
///   exponent's, which rejected exactly-round-trippable literals like
///   `1.2345678901234567e10` (17 mantissa digits, but 19 counted) and
///   reproduced #1008's catastrophic-decimal-expansion symptom for any
///   long-enough exponent (caught in that PR's own code review).
///
/// Exponent notation used to be excluded here entirely (issue #1008's
/// original symptom): the reasoning was that `format_number_jq_compat` —
/// one formatter this text can flow through — re-normalizes exponents
/// (uppercase `E`, forced sign) regardless, so "there was nothing gained"
/// by preserving the source spelling. That premise doesn't hold for every
/// caller: several yq output paths (`emit_yaml_value_at_depth`,
/// `format_json_impl` in yq mode, `stream_owned_value_json`'s finite-literal
/// hook) echo a `NumberLiteral`'s text directly rather than routing it
/// through jq's reformatter, and real yq preserves scientific-notation
/// literals byte-for-byte regardless of magnitude — confirmed empirically
/// against the pinned oracle (`1e100` stays `1e100`, `1E5` stays `1E5`).
///
/// A YAML-legal-but-JSON-unsafe spelling (`+2.0`, `1.`, `007e2`) still
/// fails this directly, even after #954 -- see
/// [`preservable_float_literal_text`], which normalizes to an equivalent
/// JSON-safe spelling before falling back to this same check.
#[must_use]
pub(super) fn is_preservable_float_literal(s: &str) -> bool {
    let mantissa = match s.find(['e', 'E']) {
        Some(exp_pos) => &s[..exp_pos],
        None => s,
    };
    (s.contains('.') || s.contains(['e', 'E']))
        && significant_mantissa_digit_count(mantissa) <= MAX_PRESERVABLE_FLOAT_DIGITS
        && is_json_number_syntax(s)
}

/// Count of `mantissa`'s *significant* digits: every digit from the first
/// nonzero digit onward (including any zero after it, whether between
/// digits or trailing), ignoring a leading run of zeros before that first
/// nonzero digit and ignoring sign/`.` characters throughout -- the same
/// "significant figures" rule scientific notation itself uses (#1211). A
/// mantissa with no nonzero digit at all -- a zero-valued spelling -- has
/// zero significant digits by this definition, which is why it needs no
/// separate carve-out from [`MAX_PRESERVABLE_FLOAT_DIGITS`]: it's already
/// within any nonnegative cap.
///
/// Ignores (does not count, does not reject) anything outside
/// `0`-`9`/`.`/`+`/`-` -- this runs *before*
/// [`is_preservable_float_literal`]'s own trailing `is_json_number_syntax`
/// check (short-circuit `&&` evaluates left to right), so `mantissa` is
/// **not** yet known to be valid number syntax at this point; this
/// function stays total over arbitrary input rather than relying on a
/// precondition its own caller doesn't actually establish until
/// afterward. A malformed mantissa this function undercounts would still
/// be caught by that later `is_json_number_syntax` check before `true`
/// could ever propagate out of `is_preservable_float_literal` as a whole.
fn significant_mantissa_digit_count(mantissa: &str) -> usize {
    let mut count = 0usize;
    let mut seen_nonzero = false;
    for b in mantissa.bytes() {
        match b {
            b'1'..=b'9' => {
                seen_nonzero = true;
                count += 1;
            }
            b'0' if seen_nonzero => count += 1,
            _ => {} // leading zero, sign, '.', or (unreachable in practice) anything else
        }
    }
    count
}

/// A normalized, JSON-safe equivalent spelling for `s`, for a `Float`
/// scalar whose text is YAML-legal but rejected outright by
/// [`is_preservable_float_literal`] (#954, the residual scope #918
/// deliberately left open) -- a companion fallback to that predicate, not
/// a replacement for it: callers check `is_preservable_float_literal(s)`
/// first and only reach for this on that check's `false` (this function
/// itself returns `None`, not `Some(s)`, when `s` was already
/// preservable, so it can't be mistaken for the primary check). Handles a
/// leading `+` stripped (`+1.0` -> `1.0`), a trailing bare `.` completed
/// with a `0` (`1.` -> `1.0`), and/or a redundant leading zero stripped
/// (`007e2` -> `7e2`, reusing
/// [`crate::json::validate::strip_redundant_leading_zeros`], #1149's own
/// JSON-side helper for the identical problem).
///
/// Normalizing (rather than preserving verbatim, the way
/// `is_preservable_float_literal` does) is required, not a style choice:
/// `OwnedValue::NumberLiteral`'s downstream JSON-reindexing bridge parses
/// its stored text as if it *were* JSON, so a genuinely non-JSON-safe
/// spelling passed through unchanged corrupts that round trip. This was
/// caught live during this fix's own development (code review self-check):
/// an earlier draft widened `is_preservable_float_literal` itself to
/// accept anything this crate's own lenient semi-index scanner
/// (`number_literal_end`) could find the boundaries of, on the theory that
/// scanner-safety implied output-safety -- it doesn't. The scanner finding
/// a clean span only means it won't mis-parse *already-embedded* text; it
/// says nothing about whether that same text is valid to *emit* as new
/// JSON output. That draft made `-o json` on `a: 1.`/`a: 007e2` literally
/// emit `1.`/`007e2` as JSON number text -- invalid per RFC 8259 (confirmed
/// via a real JSON parser rejecting it) -- for the same query shapes that
/// happened to route through this text before any other validation caught
/// it. Every consumer needs the stored spelling to be actual valid JSON,
/// which normalizing up front guarantees uniformly.
///
/// This is a real, permanent divergence from real yq's own verbatim-echo
/// `tostring`/`join`/`-o yaml` output for these spellings (`+1.0`, `1.`,
/// `007e2` all echo completely unchanged in real yq, oracle-confirmed) --
/// accepted, not fixed by this function, matching #954's own root-cause
/// framing (real yq's Go-based number model has no equivalent internal
/// JSON-reindexing constraint forcing it to normalize).
///
/// `None` when nothing here helps (not `.`-or-exponent-shaped at all, the
/// digit cap is exceeded, or the normalized text is still invalid, e.g.
/// `+1.2.3`) -- callers fall back to their own pre-existing bare-`Float`
/// handling unchanged.
///
/// Splits on the exponent marker first, mirroring
/// [`is_preservable_float_literal`]'s own mantissa/exponent split just
/// above, so the trailing-dot completion applies to the *mantissa*, not
/// the whole string -- an earlier draft completed a trailing `.` only at
/// the very end of `s`, missing a bare dot immediately before an exponent
/// (`1.e5`; code review caught this live, since it silently reproduced
/// #954's own self-inconsistency symptom for that one shape: falling
/// through to the bare-`Float` path left `tostring`/`join` disagreeing
/// with each other again, exactly what this function exists to prevent).
///
/// A mantissa whose own digit count is right at
/// [`MAX_PRESERVABLE_FLOAT_DIGITS`] can still lose preservation here if it
/// also needs the trailing-dot completion (the appended `0` pushes it one
/// digit over) -- accepted as a narrow, safe edge case: the digit cap's
/// own re-check after normalizing means this never emits a wrong value,
/// only occasionally declines to preserve an already-rare spelling
/// (17-significant-digit mantissa *and* a bare trailing dot), falling
/// back to the always-value-correct bare-`Float` reconstruction instead.
#[must_use]
pub(super) fn preservable_float_literal_text(s: &str) -> Option<String> {
    if is_preservable_float_literal(s) {
        return None;
    }
    let stripped_plus = s.strip_prefix('+').unwrap_or(s);
    let (mantissa, exponent) = match stripped_plus.find(['e', 'E']) {
        Some(exp_pos) => stripped_plus.split_at(exp_pos),
        None => (stripped_plus, ""),
    };
    let mantissa = match mantissa.strip_suffix('.') {
        Some(_) => format!("{mantissa}0"),
        None => mantissa.to_string(),
    };
    let mantissa = crate::json::validate::strip_redundant_leading_zeros(mantissa.as_bytes())
        .and_then(|stripped| String::from_utf8(stripped).ok())
        .unwrap_or(mantissa);
    let normalized = format!("{mantissa}{exponent}");
    is_preservable_float_literal(&normalized).then_some(normalized)
}

/// Whether emitting `text` as a plain YAML scalar would lose its
/// float-ness, so a `!!float` tag (or a float-shaped respelling) is needed
/// to keep the value's type stable across a round trip.
///
/// Deliberately defined as "what [`resolve_plain`] — this crate's own
/// reader — would say", rather than a hand-rolled scan for `.`/`e`. Two
/// callers need the identical question answered and a second, independent
/// spelling of YAML's float grammar would drift from the first (CLAUDE.md's
/// #106 lesson: duplicated predicates diverge silently):
/// - `format_float_yq_yaml_nested` (`light.rs`), deciding whether nested
///   YAML output must precede a computed float with `!!float ` (#1090).
/// - [`ResolvedScalar::to_owned_value`] below, deciding whether a
///   tag-forced float needs a float-shaped literal to survive
///   `to_json_for_reindex`'s JSON round trip (#1176).
///
/// Anchoring on `resolve_plain` also guarantees the emitter and the reader
/// agree: whatever spelling this crate writes, this crate reads back at the
/// same type. That costs one byte against real yq on exactly one value —
/// yq resolves `-0` as `!!float` while `resolve_plain` calls it `!!int`, so
/// a computed negative zero emits `!!float -0` here versus yq's bare `-0`.
/// Tagging it is the type-safe side of that divergence, and fixing
/// `resolve_plain`'s `-0` classification later makes both callers
/// oracle-exact with no change to either.
#[must_use]
pub(crate) fn needs_explicit_float_tag(text: &str) -> bool {
    !matches!(resolve_plain(text), ResolvedScalar::Float(_))
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
        // exponent form -- exercised directly here rather than only through
        // `is_preservable_float_literal` (which does route exponent text
        // through it as of #1008; see that predicate's own test below).
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
    fn preservable_float_literal_requires_a_dot_or_an_exponent() {
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
        // Exponent form, with or without a dot (#1008): unambiguous float
        // syntax on its own, and real yq preserves it verbatim regardless
        // of magnitude -- confirmed empirically against the pinned oracle.
        for s in ["2e2", "-0e10", "1.5e-3", "1e100", "1E5"] {
            assert!(
                is_preservable_float_literal(s),
                "expected preservable: {s:?}"
            );
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

    /// #1008 code review: an earlier version of the digit cap counted
    /// exponent digits along with the mantissa's, so a full-precision
    /// (17-digit) mantissa paired with any multi-digit exponent was
    /// wrongly rejected -- reproducing #1008's catastrophic-decimal-expansion
    /// symptom for exactly the round-trip-exact literals the cap exists to
    /// protect. Only the mantissa's own digit count should matter.
    #[test]
    fn preservable_float_literal_digit_cap_ignores_exponent_digits() {
        let mantissa_at_cap = "1.".to_string() + &"1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS - 1);
        assert!(is_preservable_float_literal(&format!(
            "{mantissa_at_cap}e100"
        )));
        assert!(is_preservable_float_literal(&format!(
            "{mantissa_at_cap}e-300"
        )));

        let mantissa_over_cap = "1.".to_string() + &"1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS);
        assert!(!is_preservable_float_literal(&format!(
            "{mantissa_over_cap}e1"
        )));
    }

    /// #1211: a zero-mantissa literal has no "significant digits" for the
    /// digit cap to bound -- every zero-mantissa spelling represents the
    /// same value (`0`) regardless of length, so it stays preservable at
    /// any length. Real yq preserves it verbatim too, confirmed live
    /// (`0.00000000000000000000e-400` -- the issue's own repro).
    #[test]
    fn preservable_float_literal_zero_mantissa_ignores_the_digit_cap() {
        let long_zero_mantissa = "0.".to_string() + &"0".repeat(100);
        assert!(is_preservable_float_literal(&format!(
            "{long_zero_mantissa}e-400"
        )));
        // A short zero mantissa (already worked before #1211) must stay
        // preservable too -- the fix must not narrow this case.
        assert!(is_preservable_float_literal("0.000e-400"));
    }

    /// #1211: a leading run of zeros (before the first nonzero digit) never
    /// counts toward the cap, the same "significant figures" rule the
    /// all-zero case above gets -- confirmed live against the pinned oracle
    /// for a case with real significant digits behind many leading zeros,
    /// not just the all-zero case #1211 itself reported. The cap still
    /// applies once there are genuinely too many *significant* digits.
    #[test]
    fn preservable_float_literal_leading_zeros_never_count_toward_the_cap() {
        // A single nonzero digit, however many leading zeros precede it, is
        // one significant digit -- well within the cap, same "significant
        // figures" rule a zero mantissa gets (#1211). This is *not* a
        // regression of the pre-#1211 raw-digit-count behavior: it's the
        // adjacent bug that behavior also had (a real, live divergence from
        // the pinned oracle, confirmed during this fix's own review), now
        // fixed by the same change.
        let mostly_zeros_one_nonzero_digit = "0.".to_string() + &"0".repeat(30) + "1";
        assert!(is_preservable_float_literal(&format!(
            "{mostly_zeros_one_nonzero_digit}e-400"
        )));
        // The cap still applies once there are genuinely too many
        // *significant* digits, leading zeros or not.
        let over_cap_significant_digits =
            "0.".to_string() + &"0".repeat(30) + &"1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS + 1);
        assert!(!is_preservable_float_literal(&format!(
            "{over_cap_significant_digits}e-400"
        )));
        // Exactly at the cap, leading zeros or not: preservable.
        let at_cap_significant_digits =
            "0.".to_string() + &"0".repeat(30) + &"1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS);
        assert!(is_preservable_float_literal(&format!(
            "{at_cap_significant_digits}e-400"
        )));
        // No leading zeros at all: unaffected by #1211, same as before.
        let short_mostly_zeros = "0.".to_string() + &"0".repeat(10) + "1";
        assert!(is_preservable_float_literal(&format!(
            "{short_mostly_zeros}e-400"
        )));
    }

    // ========================================================================
    // preservable_float_literal_text tests (#954)
    // ========================================================================

    #[test]
    fn preservable_float_literal_text_returns_none_when_already_preservable() {
        // Callers check `is_preservable_float_literal` first; this function
        // is only the fallback, so it must not also claim the already-ok case.
        for s in ["2.0", "-2.0", "1e100", "0.5"] {
            assert_eq!(preservable_float_literal_text(s), None, "input: {s:?}");
        }
    }

    #[test]
    fn preservable_float_literal_text_strips_leading_plus() {
        assert_eq!(
            preservable_float_literal_text("+1.0"),
            Some("1.0".to_string())
        );
        assert_eq!(
            preservable_float_literal_text("+2.5e10"),
            Some("2.5e10".to_string())
        );
    }

    #[test]
    fn preservable_float_literal_text_completes_a_bare_trailing_dot() {
        assert_eq!(
            preservable_float_literal_text("1."),
            Some("1.0".to_string())
        );
        assert_eq!(
            preservable_float_literal_text("-1."),
            Some("-1.0".to_string())
        );
    }

    #[test]
    fn preservable_float_literal_text_strips_redundant_leading_zero() {
        assert_eq!(
            preservable_float_literal_text("007e2"),
            Some("7e2".to_string())
        );
        assert_eq!(
            preservable_float_literal_text("-007e2"),
            Some("-7e2".to_string())
        );
        assert_eq!(
            preservable_float_literal_text("007.500"),
            Some("7.500".to_string())
        );
    }

    #[test]
    fn preservable_float_literal_text_composes_multiple_transforms() {
        // Leading `+` AND a redundant leading zero, in one literal.
        assert_eq!(
            preservable_float_literal_text("+007e2"),
            Some("7e2".to_string())
        );
        // Leading `+` AND a bare trailing dot.
        assert_eq!(
            preservable_float_literal_text("+1."),
            Some("1.0".to_string())
        );
    }

    /// Code review: a naive whole-string trailing-dot check (`s.strip_suffix('.')`)
    /// misses a bare dot immediately before an exponent marker, since the
    /// exponent digits are the actual string suffix, not the dot. Confirms
    /// the mantissa/exponent split fixes this for every combination of
    /// leading `+` and redundant leading zero too.
    #[test]
    fn preservable_float_literal_text_completes_a_trailing_dot_before_an_exponent() {
        assert_eq!(
            preservable_float_literal_text("1.e5"),
            Some("1.0e5".to_string())
        );
        assert_eq!(
            preservable_float_literal_text("+1.e5"),
            Some("1.0e5".to_string())
        );
        assert_eq!(
            preservable_float_literal_text("007.e2"),
            Some("7.0e2".to_string())
        );
        assert_eq!(
            preservable_float_literal_text("-1.e5"),
            Some("-1.0e5".to_string())
        );
    }

    #[test]
    fn preservable_float_literal_text_none_when_normalized_form_still_invalid() {
        // `+1.2.3` normalizes (strip `+`) to `1.2.3`, which is still not a
        // single valid number -- #966's own multi-dot precedent, not
        // something this function should paper over.
        assert_eq!(preservable_float_literal_text("+1.2.3"), None);
        // No dot or exponent at all after stripping -- not float-shaped.
        assert_eq!(preservable_float_literal_text("+5"), None);
    }

    #[test]
    fn preservable_float_literal_text_respects_the_digit_cap() {
        let over_cap = "+1.".to_string() + &"1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS);
        assert_eq!(preservable_float_literal_text(&over_cap), None);
    }

    /// Code review: a mantissa with exactly [`MAX_PRESERVABLE_FLOAT_DIGITS`]
    /// digits *and* a bare trailing dot (so its digit count is under the
    /// cap *before* normalization) still gets rejected, because completing
    /// the dot appends a `0` that pushes the count one over. Documented as
    /// an accepted, narrow edge case in this function's own doc comment --
    /// this test exists to pin the *safety* property (falls back to the
    /// value-correct bare-`Float` path, never a wrong number or invalid
    /// JSON), not to claim the spelling gets preserved.
    #[test]
    fn preservable_float_literal_text_digit_cap_boundary_with_trailing_dot_is_a_safe_miss() {
        let digits_at_cap = "1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS);
        // One digit short of the cap plus the completed dot's `0` lands
        // exactly at the cap -- still preserved.
        let one_under = "1".repeat(MAX_PRESERVABLE_FLOAT_DIGITS - 1) + ".";
        assert_eq!(
            preservable_float_literal_text(&one_under),
            Some(one_under.clone() + "0")
        );
        // At the cap already, so completing the dot pushes one over --
        // declines to preserve, rather than silently exceeding the cap.
        let at_cap = digits_at_cap.clone() + ".";
        assert_eq!(preservable_float_literal_text(&at_cap), None);
    }
}
