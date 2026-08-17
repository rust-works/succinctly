//! Owned JSON values for jq evaluation.
//!
//! When jq expressions construct new values (arrays, objects) or perform
//! computations, we need to materialize them into owned values rather than
//! references into the original JSON bytes.

#[cfg(not(test))]
use alloc::borrow::Cow;
#[cfg(not(test))]
use alloc::boxed::Box;
#[cfg(not(test))]
use alloc::format;
#[cfg(not(test))]
use alloc::string::{String, ToString};
#[cfg(not(test))]
use alloc::vec::Vec;
#[cfg(test)]
use std::borrow::Cow;

use indexmap::IndexMap;

use super::escape::{escape_json_body, write_json_body_jq};
#[cfg(test)]
use super::eval::JqSemantics;
use super::eval::{EvalSemantics, EvalTag};
use super::expr::Literal;

/// Recursion-depth ceiling for tree-walkers over an already-materialized
/// [`OwnedValue`] (#1005).
///
/// Guards [`OwnedValue::to_json`]/`to_json_for_reindex`/`==`/
/// `eval::compare_values`/`eval.rs`'s own `to_owned`/
/// `yq_runner::reconcile_presentation`/`output::format_json_impl`.
///
/// Deliberately a *separate* constant from
/// [`eval_generic::MAX_NESTING_DEPTH`](super::eval_generic::MAX_NESTING_DEPTH)
/// (256), not a reuse of it: that ceiling guards cursor-to-`OwnedValue`
/// *conversion* functions, individually tuned against their own measured
/// crash boundaries (600-700 for the tightest of that set, `print_json`).
/// This constant's own guarded functions are a different shape with
/// different measured boundaries (debug build, default 2MiB test-thread
/// stack — the more fragile of debug/release, matching how 256 was itself
/// measured): `reconcile_presentation` crashes between depth 580-600 (the
/// tightest of this set), `format_json_impl` between 650-700, and
/// `eval.rs`'s own `to_owned` between 1800-2000. Reusing 256 here would be
/// wrong in the *other* direction: 256 does not clear
/// `tests/jq_recurse_depth_tests.rs`'s deliberately-pinned depth-300
/// correctness capability for `path(..)`/`path(recurse)` (#626's de-risk
/// step for `push_recursive_branches`/`resolve_recurse`), which that test
/// exercises via the library's own `eval()` entry point and which routes
/// through `eval.rs`'s own `to_owned` — a real, tested capability of this
/// crate's public API, independent of whether the `succinctly` CLI binary's
/// own document-parsing entry point happens to hit a different, earlier
/// guard (`eval_generic::MAX_NESTING_DEPTH`, unaffected by this constant)
/// first for the same document depth.
///
/// 384, not 300: needs margin above the pinned floor for the same reason
/// `eval_generic`'s own 256 needed margin above its 200-deep `walk` floor
/// (256 is `200 * 1.28`; 384 is `300 * 1.28`, the same proportional
/// headroom) — while staying comfortably under 580, the tightest of this
/// set's measured boundaries, with margin to spare for a CI runner with a
/// smaller default stack than the dev machine this was measured on.
pub const MAX_VALUE_TREE_DEPTH: usize = 384;

/// Panics past `max` levels of nesting.
///
/// The one place every depth-guarded recursive function in the binary
/// raises, regardless of which ceiling it's checked against (#1018) --
/// before this, [`assert_value_tree_depth`] and
/// [`eval_generic::assert_nesting_depth`](super::eval_generic::assert_nesting_depth)
/// each carried their own byte-identical `assert!` body, hardcoding a
/// different constant -- the same "duplicated predicates diverge
/// silently" shape #998 had already fixed once for three earlier copies
/// of this exact check. Both are now thin wrappers around this one,
/// parameterized by `max` instead of re-deriving the assertion.
///
/// `#[track_caller]` (and on both wrappers below) so a panic reports the
/// call site inside the actual `_at_depth` recursive function that
/// overflowed, not this shared body's own line -- otherwise every one of
/// the ~15 guarded call sites collapses to the same file:line, making a
/// crash report impossible to attribute without a full backtrace (#1020
/// code review).
#[track_caller]
pub fn assert_depth(depth: usize, max: usize) {
    assert!(depth < max, "nesting depth exceeds limit of {max}");
}

/// Panics past [`MAX_VALUE_TREE_DEPTH`] levels of nesting (#1005).
///
/// See that constant's own doc comment for why this exists as a second,
/// independently-tuned ceiling alongside
/// [`eval_generic::assert_nesting_depth`](super::eval_generic::assert_nesting_depth).
#[track_caller]
pub fn assert_value_tree_depth(depth: usize) {
    assert_depth(depth, MAX_VALUE_TREE_DEPTH);
}

/// The parsed value backing a [`OwnedValue::NumberLiteral`], kept separate
/// from the source text so arithmetic/comparison can read it without
/// touching the `Box<str>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberRepr {
    Int(i64),
    Float(f64),
}

/// Try `i64` first, fall back to `f64` -- the one definition of "how does a
/// number string decide between the two representations," shared by
/// [`OwnedValue::from_number_literal_boxed`], [`OwnedValue::from_number_bytes`],
/// and `parser.rs`'s `fold_index_key` so they can't silently diverge.
pub(crate) fn parse_i64_or_f64(s: &str) -> Option<NumberRepr> {
    if let Ok(i) = s.parse::<i64>() {
        Some(NumberRepr::Int(i))
    } else if let Ok(f) = s.parse::<f64>() {
        Some(NumberRepr::Float(f))
    } else {
        None
    }
}

/// Format a raw JSON number string the way jq itself would print it.
///
/// This is jq's number formatting, not a verbatim echo of `raw`: jq always
/// renders a number through its own canonical algorithm, so `jq -c .` on
/// `1e100` prints `1E+100`, not the `1e100` the document actually contains.
/// `OwnedValue::NumberLiteral` keeps the raw text so this can be derived
/// on demand (Rust's own `f64`/`i64` `Display` can't reproduce it -- that
/// mismatch is issue #387).
///
/// - Integers: output as-is
/// - Floats with trailing zeros: preserve them (`0.10` -> `0.10`)
/// - Scientific notation: normalize mantissa (`12e2` -> `1.2E+3`), uppercase E, explicit +
/// - `e0`/`e-0`: eliminate the exponent entirely (`5.5e0` -> `5.5`)
/// - Negative exponents >= -5: convert to decimal (`1e-3` -> `0.001`)
/// - Negative exponents < -5: keep scientific (`1e-10` -> `1E-10`)
/// - Negative zero: preserved as `-0`
///
/// Shared by the CLI's `--jq-compat` output formatter
/// (`src/bin/succinctly/jq_runner.rs`) and every library-level path that
/// renders a `NumberLiteral` (`to_json`, `tostring`, `@json`, string
/// interpolation, error-message previews) -- a single definition so the two
/// cannot drift, per the #106 lesson in `CLAUDE.md`.
pub fn format_number_jq_compat(raw: &[u8]) -> String {
    let s = match core::str::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return String::from_utf8_lossy(raw).into_owned(),
    };

    // Check if it contains exponent notation
    let has_exp = s.contains('e') || s.contains('E');
    let has_dot = s.contains('.');

    if !has_exp && !has_dot {
        // Plain integer - output as-is
        return s.to_string();
    }

    if !has_exp {
        // Plain decimal without exponent - preserve as-is (keeps trailing
        // zeros), except real jq's own reader adds a leading `0` to a
        // leading-dot spelling even under plain identity -- confirmed
        // live: `.500 | .` -> `0.500` (trailing zeros still kept
        // verbatim), not `.500` (#1171). `is_valid_number`-gated callers
        // never reach this function with a leading-dot `s` in the first
        // place (strict RFC 8259 has no such spelling); this only fires
        // for `s` sourced from a `NumberLiteral` this crate's own
        // document-input scanners now recognize as jq-lenient (see
        // `number_literal_end`, `src/json/light.rs`).
        return if let Some(rest) = s.strip_prefix('.') {
            format!("0.{rest}")
        } else if let Some(rest) = s.strip_prefix("-.") {
            format!("-0.{rest}")
        } else {
            s.to_string()
        };
    }

    // Has exponent - need to reformat according to jq rules
    // Parse the full number to get the actual value
    let value: f64 = match s.parse() {
        Ok(v) => v,
        Err(_) => return s.to_string(),
    };

    // Parse exponent to check for e0/e-0. `has_exp` above guarantees `e`/`E`
    // is present, so the position is always found.
    let exp_pos = s.find(['e', 'E']).expect("has_exp guarantees e/E present");
    // `i64`, not `i32`: an exponent digit string beyond `i32` range (e.g.
    // `1e-2147483649`) used to silently become 0 via `.unwrap_or(0)`,
    // misrouting into the `exp == 0` "eliminate exponent" fast path below
    // and reintroducing #1099's exact symptom one exponent-digit past this
    // PR's own tested boundary (found in code review). `i64` comfortably
    // covers any exponent that can appear here.
    let exp: i64 = parse_literal_exponent(&s[exp_pos + 1..]);

    // A literal whose magnitude overflows f64 (e.g. `1e400`) parses to
    // +/-infinity, not a parse error - `value` above is non-finite in that
    // case. The normalized-scientific-notation branch below renormalizes via
    // `log10`/`pow` on `value`, which on an infinite input produces garbage
    // (`log10(inf).floor() as i32` saturates to `i32::MAX`, and dividing by
    // `10^i32::MAX` yields `NaN`) rather than erroring - so it must be
    // special-cased here, before that branch ever runs. Every existing
    // caller already guards non-finite `NumberLiteral`s before reaching this
    // function (see #930), so this exists purely to make the function
    // correct for a caller that stops doing that.
    //
    // `exp_pos` (not the already-parsed `exp: i32` above) is passed through:
    // an overflowed literal's own exponent digit string can itself exceed
    // `i32::MAX` (`exp`'s `unwrap_or(0)` would silently zero it), so the
    // overflow path re-parses it independently, at wider precision.
    if !value.is_finite() {
        return format_overflow_literal_mantissa(s, exp_pos, value.is_sign_negative());
    }

    // For e0 or e-0, jq eliminates the exponent
    if exp == 0 {
        // Check if result is integer
        if value.fract() == 0.0 && value.abs() < 1e15 {
            // `value as i64` truncates -0.0's sign (`-0.0 as i64 == 0`),
            // dropping it from the formatted text too -- `1e-0`/`-0e0`-style
            // literals #1008 made newly reachable from YAML hit this.
            // Real jq keeps the sign (`-0e0` -> `-0`).
            return if value.is_sign_negative() {
                format!("-{}", value.abs() as i64)
            } else {
                format!("{}", value as i64)
            };
        }
        // Format as plain decimal
        return format!("{value}");
    }

    // For negative exponents >= -5, jq converts to decimal
    if (-5..0).contains(&exp) {
        // Convert to decimal: 1e-3 → 0.001
        // Use smart rounding to avoid floating point noise
        return format_decimal_jq(value);
    }

    // For other cases, use normalized scientific notation
    // jq normalizes mantissa to have one digit before decimal point
    if value == 0.0 || value.abs() < f64::MIN_POSITIVE {
        // `value == 0.0` is true both for a genuinely-zero-mantissa literal
        // (`0e-400`) and for a *nonzero*-mantissa literal whose magnitude
        // simply underflowed `f64` during parsing (`1e-400`, far below
        // `f64::MIN_POSITIVE`) -- the two can't be told apart from `value`
        // alone (#1099). `format_underflow_literal_mantissa` (and the
        // shared `normalize_extreme_literal_mantissa` beneath it) tells
        // them apart from the literal's own source digits.
        //
        // `value.abs() < f64::MIN_POSITIVE` (nonzero but *subnormal*, #1177):
        // the finite `log10`/`pow` renormalization below breaks down at the
        // extreme low end of the subnormal range -- `libm::pow(10.0,
        // log10(abs_value).floor())` can itself underflow to exactly `0.0`
        // (its own result is smaller than the smallest representable
        // subnormal), making `abs_value / 0.0 = +inf` and rendering the
        // mantissa as the literal text `"inf"`, which isn't valid JSON.
        // Routing every subnormal through the same string-based path as
        // exact-zero underflow sidesteps this entirely: that path derives
        // the mantissa from `s`'s own source digits, never from `f64`
        // arithmetic that's already lost precision by this magnitude.
        return format_underflow_literal_mantissa(s, exp_pos, value.is_sign_negative());
    }

    let abs_value = value.abs();
    let log10 = libm::log10(abs_value).floor() as i32;
    let normalized_mantissa = abs_value / libm::pow(10.0, log10 as f64);
    let new_exp = log10;

    // Format mantissa with appropriate precision
    // Round to avoid floating point noise (e.g., 9.199999999999999 → 9.2)
    let mantissa_str = format_mantissa_jq(normalized_mantissa);

    let sign = if value < 0.0 { "-" } else { "" };
    assemble_scientific(sign, &mantissa_str, i64::from(new_exp))
}

/// jq mode's bare `Float` display: no forced decimal point, matching real
/// jq's own convention that a computed value (one with no preserved
/// `NumberLiteral` source text) loses its literal formatting entirely
/// (`1.0 + 4.0` prints `5`, not `5.0`). Named and shared, rather than a
/// hand-copied `|f| f.to_string()` closure at each call site, per the #106
/// "duplicated predicates diverge silently" lesson in `CLAUDE.md` --
/// [`to_json`](OwnedValue::to_json), [`to_json_for_reindex_at_depth`]'s
/// jq-mode fallback, and [`stream::stream_owned_value_json_jq`](crate::jq::stream)
/// all need this exact formatter.
pub(crate) fn jq_bare_float_display(f: f64) -> String {
    f.to_string()
}

/// Join a sign, an already-normalized mantissa, and an exponent into jq's
/// scientific-notation text (`{sign}{mantissa}E{+/-}{exp}`) -- the shared
/// final step of both the finite path above and the overflow path below, so
/// the two can't independently drift on the `E+`/`E-` convention.
fn assemble_scientific(sign: &str, mantissa_str: &str, exp: i64) -> String {
    let exp_sign = if exp >= 0 { "+" } else { "" };
    format!("{sign}{mantissa_str}E{exp_sign}{exp}")
}

/// Parse a literal's exponent digit string at wide (`i64`) precision,
/// saturating to `i64::MIN`/`MAX` by sign on overflow rather than erroring
/// -- the exponent digit string can itself be longer than `i64` can hold
/// (e.g. `1e99999999999999999999`), and any such value is already certain
/// to be past whichever ceiling (or lack thereof) the caller applies.
/// Shared so `format_number_jq_compat`'s own exponent dispatch and
/// [`normalize_extreme_literal_mantissa`]'s shift math can't independently
/// drift on how an out-of-range exponent is handled (#1099 code review:
/// an earlier `i32`-width, `.unwrap_or(0)` version of this parse at the
/// `format_number_jq_compat` call site silently treated an out-of-range
/// exponent as exactly `0`, misrouting extreme-underflow literals into the
/// "eliminate exponent" fast path before ever reaching this module).
fn parse_literal_exponent(exp_text: &str) -> i64 {
    exp_text.parse().unwrap_or_else(|_| {
        if exp_text.trim_start_matches('+').starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

/// Shift a literal's own mantissa digits (the text before `exp_pos`, i.e.
/// before `e`/`E`) into jq's one-digit-before-the-point normalized form,
/// and fold that shift into the literal's own written exponent -- returns
/// `Some((mantissa_str, new_exp))`, or `None` if the mantissa is genuinely
/// all-zero (`0.0e400`, `0.00e-400`), which has no magnitude to normalize
/// against. Entirely via string manipulation on `s` rather than
/// `log10`/`pow` on the parsed value, since the caller only reaches here
/// when the parsed value has already lost the precision this exists to
/// recover (`+/-infinity` on overflow, exactly `0.0` on underflow).
///
/// Shared by [`format_overflow_literal_mantissa`] and
/// [`format_underflow_literal_mantissa`] (#1099) -- the shift math is
/// identical whether the literal's written exponent is enormous-positive
/// (overflow) or enormous-negative (underflow); only what each caller does
/// with `new_exp` afterward differs (overflow caps to infinity text past a
/// ceiling; underflow has no such ceiling, oracle-verified on its own doc
/// comment). The `None` case is centralized here rather than pre-checked
/// separately by each caller, so "is this mantissa all-zero" has exactly
/// one implementation instead of two that must be kept in sync by
/// convention (#106).
fn normalize_extreme_literal_mantissa(s: &str, exp_pos: usize) -> Option<(String, i64)> {
    // `dump_truncated`'s whole design keeps preview cost independent of the
    // value's own size (see its doc comment) - a document-controlled
    // mantissa of unbounded length must not turn into unbounded work here
    // just because only a handful of its leading digits will ever survive
    // that later truncation anyway. Bounding the *copy* is enough: `shift`
    // below only ever needs `int_part.len()` (an O(1) property read, not a
    // scan), so truncating what gets copied into `rest` doesn't touch the
    // exponent math's correctness, only how many digits past the first one
    // actually get rendered.
    const MAX_RENDERED_MANTISSA_DIGITS: usize = 32;

    let mantissa = s[..exp_pos].strip_prefix('-').unwrap_or(&s[..exp_pos]);
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));

    // A leading-dot literal (`.5e400`) has an empty `int_part`, not `"0"` --
    // treat both the same way (mantissa magnitude < 1).
    let (shift, leading, rest): (i64, &str, String) = if int_part.is_empty() || int_part == "0" {
        // Shift right to the first nonzero fractional digit. Genuinely
        // all-zero mantissa (`0.0e400`/`0.0e-400`) has no nonzero digit to
        // shift to -- `?` propagates that as `None`.
        let k = frac_part.find(|c: char| c != '0')?;
        let after = &frac_part[k + 1..];
        (
            -(k as i64 + 1),
            &frac_part[k..=k],
            after[..after.len().min(MAX_RENDERED_MANTISSA_DIGITS)].to_string(),
        )
    } else {
        // Mantissa >= 1: shift left past every extra integer-part digit.
        // `int_part[1..]` is a slice (no copy); only what actually gets
        // concatenated into `rest` is capped.
        let after_leading = &int_part[1..];
        let rest = if after_leading.len() >= MAX_RENDERED_MANTISSA_DIGITS {
            after_leading[..MAX_RENDERED_MANTISSA_DIGITS].to_string()
        } else {
            let budget = MAX_RENDERED_MANTISSA_DIGITS - after_leading.len();
            format!(
                "{after_leading}{}",
                &frac_part[..frac_part.len().min(budget)]
            )
        };
        (int_part.len() as i64 - 1, &int_part[..1], rest)
    };

    let parsed_exp = parse_literal_exponent(&s[exp_pos + 1..]);
    let new_exp = parsed_exp.saturating_add(shift);

    let mantissa_str = if rest.is_empty() {
        leading.to_string()
    } else {
        format!("{leading}.{rest}")
    };
    Some((mantissa_str, new_exp))
}

/// Renormalize an overflowed literal's own mantissa digits into jq's
/// one-digit-before-the-point scientific form -- see
/// [`normalize_extreme_literal_mantissa`] for the shared shift math. `s` is
/// the full literal text and `exp_pos` the byte index of its `e`/`E`;
/// `negative` is `value.is_sign_negative()` from the caller, which (unlike
/// re-deriving a sign from `s` itself) is correct for `-0.0`-style edge
/// cases too and matches this module's usual `if value < 0.0` idiom.
///
/// Oracle-verified against real jq (issue #930): `1e400` -> `1E+400`,
/// `123e400` -> `1.23E+402`, `12.34e400` -> `1.234E+401`, `0.5e400` ->
/// `5E+399`, `100e400` -> `1.00E+402` (trailing zeros preserved, matching
/// this module's general trailing-zero rule), and beyond jq's own
/// literal-preservation ceiling, DBL_MAX text (`1e1000000000` ->
/// `1.7976931348623157e+308`, matching a computed infinity).
fn format_overflow_literal_mantissa(s: &str, exp_pos: usize, negative: bool) -> String {
    let sign = if negative { "-" } else { "" };
    // A mantissa of exactly `0` times any exponent is still exactly `0.0`,
    // never `+/-infinity` -- callers only reach this function when `value`
    // has already overflowed, which guarantees a nonzero mantissa.
    let Some((mantissa_str, new_exp)) = normalize_extreme_literal_mantissa(s, exp_pos) else {
        unreachable!("overflow implies a nonzero mantissa")
    };

    // jq's own literal-preserving text (as opposed to a computed value's
    // DBL_MAX text) only goes up to this exponent magnitude (decNumber's
    // limit) -- oracle-verified: `1e999999999` keeps `1E+999999999`,
    // `1e1000000000` switches to DBL_MAX text instead.
    if new_exp.unsigned_abs() >= 1_000_000_000 {
        return infinite_float_preview_text(negative).to_string();
    }

    assemble_scientific(sign, &mantissa_str, new_exp)
}

/// Renormalize an underflowed literal's own mantissa digits into jq's
/// one-digit-before-the-point scientific form (#1099) -- see
/// [`normalize_extreme_literal_mantissa`] for the shared shift math, and
/// [`format_overflow_literal_mantissa`] for the symmetric overflow case.
///
/// A literal whose magnitude underflows `f64` to exactly `0.0` (nonzero
/// mantissa, deeply negative exponent) can't be told apart from a
/// genuinely-zero-mantissa literal by looking at the parsed value alone --
/// both are `0.0`. The caller (`format_number_jq_compat`'s `value == 0.0`
/// branch) already checked the mantissa's own source digits before calling
/// this, so `s`'s mantissa is guaranteed nonzero here.
///
/// Oracle-verified against real jq: `1e-400` -> `1E-400`, `12.34e-400` ->
/// `1.234E-399`, `0.5e-400` -> `5E-401`, `100.5e-400` -> `1.005E-398`.
/// Unlike overflow, underflow has no *deliberate* ceiling here --
/// `1e-1000000000` (exponent magnitude *at* the overflow ceiling) still
/// prints `1E-1000000000` unchanged, not a fallback to plain `0`. Real jq
/// itself, however, breaks down for magnitudes beyond ~1,147,483,647
/// (`i32::MAX - 1e9`; an apparent internal int32-overflow bug in its own
/// decNumber), printing a fixed sentinel (`1e-1200000000` -> real jq's
/// `0E-1147483646`) rather than continuing to preserve the mantissa. This
/// function deliberately does **not** replicate that breakdown -- it keeps
/// preserving the mantissa past jq's own failure point, on the basis that
/// jq's behavior there is itself a bug, not a rule worth matching.
fn format_underflow_literal_mantissa(s: &str, exp_pos: usize, negative: bool) -> String {
    let sign = if negative { "-" } else { "" };
    match normalize_extreme_literal_mantissa(s, exp_pos) {
        Some((mantissa_str, new_exp)) => assemble_scientific(sign, &mantissa_str, new_exp),
        // Genuinely all-zero mantissa (`0.0e-400`) -- no magnitude to
        // renormalize against. `value == 0.0` is true for `-0.0` too (IEEE
        // 754), so the sign still needs preserving -- real jq keeps both
        // the sign and the source exponent for a zero mantissa, since
        // log10(0) is undefined (`-0e5` -> `-0E+5`, `-0e05` -> `-0E+5`).
        None => assemble_scientific(sign, "0", parse_literal_exponent(&s[exp_pos + 1..])),
    }
}

/// Format a mantissa value for jq-compatible output.
/// Handles floating point precision issues by rounding appropriately.
fn format_mantissa_jq(value: f64) -> String {
    // Check if it's essentially an integer
    if (value.round() - value).abs() < 1e-10 {
        return format!("{}", value.round() as i64);
    }

    // Try different precisions and pick the shortest that rounds back correctly
    for precision in 1..=15 {
        let formatted = format!("{value:.precision$}");
        if let Ok(parsed) = formatted.parse::<f64>() {
            if (parsed - value).abs() < 1e-14 {
                // Trim trailing zeros
                let trimmed = formatted.trim_end_matches('0');
                if trimmed.ends_with('.') {
                    return format!("{trimmed}0");
                }
                return trimmed.to_string();
            }
        }
    }

    // Fallback: full precision
    let formatted = format!("{value:.15}");
    let trimmed = formatted.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{trimmed}0")
    } else {
        trimmed.to_string()
    }
}

/// Format a decimal value for jq-compatible output.
/// Uses smart rounding to avoid floating point noise.
fn format_decimal_jq(value: f64) -> String {
    // `is_sign_negative()`, not `value < 0.0`: the latter is false for
    // -0.0 (IEEE 754 equality), which would silently drop the sign a
    // literal like `-0e-3` carries -- #1008 made that literal newly
    // reachable from YAML.
    let sign = if value.is_sign_negative() { "-" } else { "" };
    let abs_value = value.abs();

    // Check if it's essentially an integer
    if (abs_value.round() - abs_value).abs() < 1e-10 {
        // `value.round() as i64` truncates -0.0's sign the same way, so
        // this must build on the already-signed `abs_value` instead.
        return format!("{sign}{}", abs_value.round() as i64);
    }

    // Try different precisions and pick the shortest that rounds back correctly
    for precision in 1..=15 {
        let formatted = format!("{abs_value:.precision$}");
        if let Ok(parsed) = formatted.parse::<f64>() {
            if (parsed - abs_value).abs() < 1e-14 {
                // Trim trailing zeros
                let trimmed = formatted.trim_end_matches('0');
                if trimmed.ends_with('.') {
                    return format!("{sign}{trimmed}0");
                }
                return format!("{sign}{trimmed}");
            }
        }
    }

    // Fallback: full precision
    let formatted = format!("{abs_value:.15}");
    let trimmed = formatted.trim_end_matches('0');
    if trimmed.ends_with('.') {
        format!("{sign}{trimmed}0")
    } else {
        format!("{sign}{trimmed}")
    }
}

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
    /// A number materialized straight from a document token or a filter's
    /// own literal text, carrying jq's exact source spelling (e.g. `1e100`,
    /// `1.0`, `-0.0`) alongside its parsed value.
    ///
    /// Produced by `to_owned`-style conversions out of a document cursor,
    /// and (since #1035) by `Literal::NumberLiteral`'s own evaluation --
    /// every *other* constructor keeps using [`Int`](Self::Int)/
    /// [`Float`](Self::Float) directly. Arithmetic, comparison, and math
    /// builtins treat this exactly like `Int`/`Float` for computation --
    /// only formatting (`to_json`, `tostring`, `@json`, string
    /// interpolation, error-message previews) prefers the literal. The
    /// moment a value passes through an operation that produces a *new*
    /// number, the result collapses to plain `Int`/`Float` (see
    /// `into_plain_number`) -- matching jq, where only values that reach
    /// output untouched keep their original spelling.
    NumberLiteral(NumberRepr, Box<str>),
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

    /// Create a number that carries its document source text.
    ///
    /// Parses `literal` the same way every document-materializing conversion
    /// decides between the two representations: try `i64` first, fall back
    /// to `f64`. Falls back to [`Null`](Self::Null) if `literal` parses as
    /// neither (should not happen for a valid document number token, but
    /// matches how every other decode-failure path in this codebase
    /// represents "not actually a number" — #966).
    pub(crate) fn from_number_literal(literal: &str) -> Self {
        Self::from_number_literal_boxed(literal.into())
    }

    /// Like [`from_number_literal`](Self::from_number_literal), but takes an
    /// already-owned `Box<str>` (e.g. from `JqValue::NumberLiteral`) instead
    /// of allocating a fresh one.
    pub(crate) fn from_number_literal_boxed(literal: Box<str>) -> Self {
        let Some(repr) = parse_i64_or_f64(&literal) else {
            return Self::Null;
        };
        Self::NumberLiteral(repr, literal)
    }

    /// Materialize raw JSON number-token bytes into the correctly-gated
    /// `OwnedValue` -- the single conversion every "raw bytes -> number"
    /// call site in this crate (including the CLI binary, hence `pub` not
    /// `pub(crate)`) should go through (#966 found at least 7 independent
    /// hand-rolled copies of this decision).
    ///
    /// Checks `is_nan_sentinel` first, so every caller gets that
    /// `to_json_for_reindex`-bridge convention for free instead of having
    /// to remember its own copy of the check (an earlier draft of this
    /// function required exactly that, and three of its call sites forgot
    /// it -- caught by review).
    ///
    /// Otherwise preserves the source spelling via
    /// [`NumberLiteral`](Self::NumberLiteral) only when `bytes` is valid
    /// RFC 8259 number syntax
    /// ([`is_valid_number`](crate::json::validate::is_valid_number));
    /// otherwise degrades to a plain `Int`/`Float`, or [`Null`](Self::Null)
    /// if neither parses. Skipping straight to `from_number_literal` for an
    /// invalid span (`007`, `1.2.3`) would let
    /// [`to_json`](Self::to_json)/[`number_str`](Self::number_str) echo it
    /// back out verbatim, since both always reproduce a `NumberLiteral`'s
    /// stored text unchanged.
    pub fn from_number_bytes(bytes: &[u8]) -> Self {
        if is_nan_sentinel(bytes) {
            return Self::Float(f64::NAN);
        }
        if let Some(negative) = is_infinity_sentinel(bytes) {
            return Self::Float(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        if crate::json::validate::is_valid_number(bytes) {
            return core::str::from_utf8(bytes).map_or(Self::Null, Self::from_number_literal);
        }
        // Real jq's own number reader also accepts a leading `.` (with or
        // without a preceding `-`) when at least one digit follows (`.5`
        // -> `0.5`, `-.5` -> `-0.5`) -- not valid per strict RFC 8259
        // (`is_valid_number` stays strict on purpose, shared by callers
        // this leniency shouldn't reach), but a real jq-accepted spelling
        // this crate's own document-input scanners now recognize as a
        // number span (`number_literal_end`, #1171). Preserve it the same
        // way #1094 preserves a leading zero: check whether inserting `0`
        // right after any `-` makes the token strictly valid, and if so,
        // still materialize the *original* (un-inserted) text as the
        // literal spelling -- `bytes` was already gated by
        // `number_literal_end` at the scan site, and
        // `Self::from_number_literal` (via `parse_i64_or_f64`) parses a
        // leading-dot float natively, so no separate reparse of the
        // original text is needed here.
        let dot_prefix_len = match bytes.first() {
            Some(b'.') => Some(0),
            Some(b'-') if bytes.get(1) == Some(&b'.') => Some(1),
            _ => None,
        };
        if let Some(prefix_len) = dot_prefix_len {
            let mut prefixed = Vec::with_capacity(bytes.len() + 1);
            prefixed.extend_from_slice(&bytes[..prefix_len]);
            prefixed.push(b'0');
            prefixed.extend_from_slice(&bytes[prefix_len..]);
            if crate::json::validate::is_valid_number(&prefixed) {
                return core::str::from_utf8(bytes).map_or(Self::Null, Self::from_number_literal);
            }
        }
        let Ok(s) = core::str::from_utf8(bytes) else {
            return Self::Null;
        };
        match parse_i64_or_f64(s) {
            Some(NumberRepr::Int(i)) => Self::Int(i),
            Some(NumberRepr::Float(f)) => Self::Float(f),
            None => Self::Null,
        }
    }

    /// Collapse a [`NumberLiteral`](Self::NumberLiteral) into a plain
    /// `Int`/`Float`, dropping the source text. A no-op for every other
    /// variant.
    ///
    /// Every operation that *computes* with a number rather than passing it
    /// through untouched should normalize its operands through this first --
    /// but only immediately before the arm that actually computes, not
    /// eagerly for every arm an operator function might take. An operand
    /// that a relocation-shaped arm (`null`-passthrough, array-append, a
    /// merge no-op, ...) hands back unchanged must keep its own spelling
    /// (`arith_add`, #1143); calling this at the top of the whole function,
    /// before the match even runs, silently strips it there too.
    pub fn into_plain_number(self) -> Self {
        match self {
            Self::NumberLiteral(NumberRepr::Int(n), _) => Self::Int(n),
            Self::NumberLiteral(NumberRepr::Float(f), _) => Self::Float(f),
            other => other,
        }
    }

    /// The exact text this number should render as: the source literal when
    /// present, otherwise the parsed value's own `Display` formatting.
    /// `None` for non-numbers.
    ///
    /// Deliberately does not special-case NaN/Infinity -- callers that need
    /// JSON's "null" substitution (`to_json`, the error-preview streamer)
    /// check that themselves before falling back to this, e.g. via
    /// `self.as_f64().is_some_and(f64::is_nan)`.
    pub fn number_str(&self) -> Option<Cow<'_, str>> {
        match self {
            Self::Int(n) => Some(Cow::Owned(n.to_string())),
            Self::Float(f) => Some(Cow::Owned(f.to_string())),
            Self::NumberLiteral(_, literal) => {
                Some(Cow::Owned(format_number_jq_compat(literal.as_bytes())))
            }
            _ => None,
        }
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
            Self::Int(_) | Self::Float(_) | Self::NumberLiteral(..) => "number",
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
            Self::NumberLiteral(NumberRepr::Int(n), _) => Some(*n),
            Self::NumberLiteral(NumberRepr::Float(f), _)
                if (*f - (*f as i64 as f64)).abs() < f64::EPSILON =>
            {
                Some(*f as i64)
            }
            _ => None,
        }
    }

    /// Convert to an f64, if possible.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Float(f) => Some(*f),
            Self::NumberLiteral(NumberRepr::Int(n), _) => Some(*n as f64),
            Self::NumberLiteral(NumberRepr::Float(f), _) => Some(*f),
            _ => None,
        }
    }

    /// The parsed [`NumberRepr`] behind `Int`, `Float`, or `NumberLiteral`;
    /// `None` for every other variant.
    ///
    /// Lets a caller (e.g. the evaluators' `compare_values`) dispatch on
    /// exactly the same `(Int, Int)` / `(Float, Float)` / mixed pairing that
    /// [`numeric_repr_eq`] uses for `==`, regardless of which of the three
    /// variants either operand happens to be -- so ordering can't disagree
    /// with equality about the same pair of numbers.
    pub(crate) fn number_repr(&self) -> Option<NumberRepr> {
        match self {
            Self::Int(n) => Some(NumberRepr::Int(*n)),
            Self::Float(f) => Some(NumberRepr::Float(*f)),
            Self::NumberLiteral(repr, _) => Some(*repr),
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
    ///
    /// Panics past [`MAX_VALUE_TREE_DEPTH`] levels of nesting (#1005) — see
    /// that constant's own doc comment for why. Needed because a value
    /// `reduce`/`foreach`/`while`/`until`/`repeat` build up at
    /// query-evaluation time bypasses #998's document-input guards
    /// entirely: no adversarial document is involved, only enough loop
    /// iterations to grow the accumulator past the limit.
    pub fn to_json(&self) -> String {
        self.to_json_at_depth(
            0,
            format_number_jq_compat,
            jq_bare_float_display,
            infinite_float_preview_text,
        )
    }

    /// The yq-mode sibling of [`to_json`](Self::to_json) (#1030): identical
    /// except a finite `NumberLiteral` echoes its document-sourced spelling
    /// verbatim rather than being reformatted per jq's own rules -- real yq
    /// preserves scientific-notation literals byte-for-byte regardless of
    /// magnitude or query shape (#1008, confirmed empirically against the
    /// pinned oracle). Used by `@json`'s yq-mode branch and by string
    /// interpolation's container arm (`owned_to_string`), both of which need
    /// this same literal-preserving JSON text, not [`to_json`](Self::to_json)'s
    /// jq-normalized one.
    ///
    /// Also keeps a whole-number `Float`'s decimal point (`format_float_with_fraction`,
    /// #953) -- unlike jq, which happily prints `1.0` as `1` in JSON (matching
    /// real jq's own `tojson`), yq must not: `1.0` and `1` are different YAML
    /// types, and dropping the point on a round trip changes it (#169's own
    /// reasoning, reused here for the same class of value reached from a
    /// different path -- an i64-overflow decimal integer scalar, which
    /// `resolve_plain` also classifies as `!!float`, confirmed live against
    /// the pinned oracle: real yq's `-o json` gives `100...0.0`, not
    /// `100...0`).
    pub(crate) fn to_json_yq(&self) -> String {
        self.to_json_at_depth(
            0,
            crate::jq::stream::real_output_finite_literal,
            crate::yaml::format_float_with_fraction,
            yq_infinite_float_json_text,
        )
    }

    /// `finite_literal` formats a `NumberLiteral`'s source text, and
    /// `float_fmt` a plain `Float`'s -- jq's [`format_number_jq_compat`]/bare
    /// `Display` for [`to_json`](Self::to_json), or a verbatim echo/
    /// decimal-point-preserving format for [`to_json_yq`](Self::to_json_yq)
    /// -- mirroring the same fork
    /// [`stream::stream_owned_value_json_with`](crate::jq::stream) already
    /// threads through for the M2 streaming path (#1008), so this doesn't
    /// grow a third hand-copied jq/yq-formatting branch.
    ///
    /// `infinite_fmt` is the analogous fork for a computed (non-`NaN`)
    /// Infinity (#1087): jq mode renders `DBL_MAX` text
    /// ([`infinite_float_preview_text`]), matching real jq's own `1.7976...e+308`
    /// rather than substituting `null` (RFC 8259 forbids the literal
    /// `Infinity`, but a finite-magnitude stand-in is representable). `NaN`
    /// has no such stand-in and stays `null` unconditionally in both modes.
    /// yq mode's own correct behavior is still an open design question (real
    /// yq's Go `encoding/json` refuses to marshal Infinity at all and
    /// errors) -- `to_json_yq` keeps the pre-existing `null` substitution
    /// rather than guessing at that answer here.
    fn to_json_at_depth(
        &self,
        depth: usize,
        finite_literal: fn(&[u8]) -> String,
        float_fmt: fn(f64) -> String,
        infinite_fmt: fn(bool) -> &'static str,
    ) -> String {
        assert_value_tree_depth(depth);
        match self {
            Self::Null => "null".into(),
            Self::Bool(true) => "true".into(),
            Self::Bool(false) => "false".into(),
            Self::Int(n) => format!("{n}"),
            Self::Float(f) => {
                if f.is_nan() {
                    "null".into() // JSON doesn't support NaN
                } else if f.is_infinite() {
                    infinite_fmt(f.is_sign_negative()).into()
                } else {
                    float_fmt(*f)
                }
            }
            Self::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => "null".into(),
            // An infinite `NumberLiteral` reaching here is always a genuine
            // document literal, which still has its own source text to
            // fall back to -- `1e400 | .` (identity, no computation) echoes
            // jq's mantissa-preserving `1E+400`, not `DBL_MAX` text,
            // confirmed live against jq 1.7.1 (#1087). The reindex bridge's
            // own computed-Infinity sentinel can never reach this arm: it's
            // intercepted during reparse by `from_number_bytes`'s
            // `is_infinity_sentinel` check (#1083/#1087, mirroring
            // `is_nan_sentinel`'s sibling interception) and becomes a plain
            // `Self::Float` above instead, before it can ever masquerade as
            // a `NumberLiteral`. `finite_literal` already handles a genuine
            // overflow literal correctly in both modes despite its name:
            // jq's `format_number_jq_compat` special-cases a non-finite
            // input via `format_overflow_literal_mantissa` rather than
            // assuming finiteness, and yq's `real_output_finite_literal`
            // echoes raw text verbatim regardless of the parsed value.
            Self::NumberLiteral(_, literal) => finite_literal(literal.as_bytes()),
            Self::String(s) => format!("\"{}\"", escape_json_body(write_json_body_jq, s)),
            Self::Array(arr) => {
                let elements: Vec<String> = arr
                    .iter()
                    .map(|v| v.to_json_at_depth(depth + 1, finite_literal, float_fmt, infinite_fmt))
                    .collect();
                format!("[{}]", elements.join(","))
            }
            Self::Object(obj) => {
                let entries: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "\"{}\":{}",
                            escape_json_body(write_json_body_jq, k),
                            v.to_json_at_depth(depth + 1, finite_literal, float_fmt, infinite_fmt)
                        )
                    })
                    .collect();
                format!("{{{}}}", entries.join(","))
            }
        }
    }

    /// Serialize this value as JSON for `eval_generic`'s cursor-reindexing
    /// bridge, preserving ±Infinity via a self-overflowing literal
    /// (`1e999`/`-1e999`) and NaN via a reserved internal sentinel instead
    /// of [`to_json`](Self::to_json)'s `"null"` substitution.
    ///
    /// `to_json()`'s "null" is correct for actual JSON *output* (RFC 8259
    /// forbids Infinity/NaN), but wrong for this purely-internal round-trip:
    /// it silently destroys the NaN/Infinity information
    /// `numeric_display_string()` (in `src/jq/eval.rs`) needs downstream once
    /// the bridge re-parses this text and hands the cursor to the full
    /// evaluator (#561, #472).
    ///
    /// `S: EvalSemantics` picks the plain (non-`NumberLiteral`) `Float`
    /// fallback's spelling (#953): yq keeps a decimal point regardless of
    /// magnitude (`format_float_with_fraction`), matching real yq's own
    /// `-o json '[.a]'` output for a value with no preserved literal (e.g.
    /// an i64-overflow YAML scalar reaching this bridge via `[...]`/
    /// `map_values`/`with_entries`, which have no native `eval_generic.rs`
    /// cursor arm). jq keeps the pre-existing bare `Display` (no forced
    /// point): real jq's own convention drops a computed value's literal
    /// formatting entirely (`1.0 + 4.0` prints `5`, not `5.0`), and a bare
    /// `Float` reaching this fallback is by construction one that already
    /// lost its `NumberLiteral` text — confirmed live this must stay
    /// mode-gated, not unconditional: an earlier draft hardcoded yq's
    /// formatter here unconditionally, which silently flipped
    /// `reduce (1,2) as $i (1.0 + 4.0; [.])` in **jq** mode from the
    /// correct `[[[5]]]` to `[[[5.0]]]` (caught in code review) since
    /// `format_number_jq_compat` does not strip an explicit `.0` back off a
    /// literal it's handed after the reparse.
    pub fn to_json_for_reindex<S: EvalSemantics>(&self) -> String {
        self.to_json_for_reindex_at_depth::<S>(0)
    }

    /// Panics past [`MAX_VALUE_TREE_DEPTH`] levels of nesting (#1005) — see
    /// that constant's own doc comment for why, and
    /// [`to_json_at_depth`](Self::to_json_at_depth), which this mirrors.
    /// This function's own callers (`reduce`/`foreach`/etc.'s per-iteration
    /// reindex bridge) are exactly the ones that grow a value one level
    /// deeper per loop iteration with no adversarial document involved, so
    /// it needs the same guard [`to_json`](Self::to_json) does.
    fn to_json_for_reindex_at_depth<S: EvalSemantics>(&self, depth: usize) -> String {
        assert_value_tree_depth(depth);
        match self {
            Self::Float(f) if f.is_nan() => NAN_SENTINEL.to_string(),
            Self::NumberLiteral(NumberRepr::Float(f), _) if f.is_nan() => NAN_SENTINEL.to_string(),
            Self::Float(f) if f.is_infinite() => overflow_literal(*f).to_string(),
            // A document-sourced overflow literal (`123e400`) is, by
            // construction, already valid JSON number syntax that reparses
            // to this exact `f64` - reusing it here (instead of the generic
            // sentinel) lets `describe()`'s preview show the real source
            // text (#930) instead of a disconnected "1e999"/"-1e999"
            // placeholder (#939). This arm only ever sees a JSON-sourced
            // literal: JSON's `number_literal()` override (`json/light.rs`)
            // is unconditional, so its overflow literals reach here
            // directly, but YAML's own override (`yaml/light.rs`, #918)
            // deliberately excludes non-finite values (`.inf`/`-.inf`/
            // `.nan` never pass `is_preservable_float_literal`'s
            // JSON-syntax check, since none of those spellings start with a
            // digit or `-digit`) - a YAML `.inf`/`.nan` still becomes a
            // plain `Float`, handled by the arm above, never this one - so
            // no further shape-checking is needed for correctness.
            //
            // The length cap *is* needed regardless of shape: the literal
            // is otherwise-unbounded document text, and this function's
            // callers (`reduce`/`foreach`/etc.'s per-iteration reindex
            // bridge) can run it over the same unchanged value thousands of
            // times - a bound this loose still comfortably covers any
            // realistic overflow literal (reaching `f64::MAX` needs on the
            // order of ~300 exponent digits at most) while keeping the
            // *reused* case itself O(1)-ish rather than O(iterations x
            // literal length) for a pathological one.
            Self::NumberLiteral(NumberRepr::Float(f), literal) if f.is_infinite() => {
                const MAX_REUSED_LITERAL_LEN: usize = 256;
                if literal.len() <= MAX_REUSED_LITERAL_LEN {
                    literal.to_string()
                } else {
                    overflow_literal(*f).to_string()
                }
            }
            // A finite NumberLiteral's source text is already valid JSON
            // number syntax (guaranteed by the two `is_preservable_float_literal`
            // gates that construct one, and by JSON's own unconditional
            // `number_literal()` override), so it needs no reformatting to
            // survive this purely-internal round trip -- any valid JSON
            // spelling of the same number works equally well for the
            // reparse below, since jq mode's own final formatter
            // (`format_number_jq_compat`, via `to_json`/`number_str`)
            // re-normalizes it *after* reparsing regardless of what spelling
            // fed the round trip. Echoing verbatim here instead of routing
            // through that same formatter (`to_json_at_depth`'s fallback
            // arm below, which the NaN/infinite arms above this one already
            // bypass) closes the reindex-bridge gap #1008 left open for
            // `[...]`/assignment/other Expr shapes with no native
            // `eval_generic.rs` arm -- verified against real yq across
            // `[.a]`, `.a,.a`, `map_values(.)`, and `with_entries(.)`, with
            // zero jq-mode output change (21-query sweep against real jq).
            Self::NumberLiteral(_, literal) => literal.to_string(),
            Self::Array(arr) => {
                let elements: Vec<String> = arr
                    .iter()
                    .map(|v| v.to_json_for_reindex_at_depth::<S>(depth + 1))
                    .collect();
                format!("[{}]", elements.join(","))
            }
            Self::Object(obj) => {
                let entries: Vec<String> = obj
                    .iter()
                    .map(|(k, v)| {
                        format!(
                            "\"{}\":{}",
                            escape_json_body(write_json_body_jq, k),
                            v.to_json_for_reindex_at_depth::<S>(depth + 1)
                        )
                    })
                    .collect();
                format!("{{{}}}", entries.join(","))
            }
            // yq keeps a whole-number `Float`'s decimal point at any
            // magnitude (#953); jq keeps the original bare `Display` (see
            // this function's own doc comment for why the fork is required,
            // not optional).
            // `infinite_fmt` is unreachable from here either way -- every
            // NaN/infinite case is already handled by the arms above, before
            // this fallback -- so which one is passed only matters for
            // reading, not behavior; picked per-mode for consistency.
            other if S::TAG == EvalTag::Yq => other.to_json_at_depth(
                depth,
                format_number_jq_compat,
                crate::yaml::format_float_with_fraction,
                yq_infinite_float_json_text,
            ),
            other => other.to_json_at_depth(
                depth,
                format_number_jq_compat,
                jq_bare_float_display,
                infinite_float_preview_text,
            ),
        }
    }
}

/// The reserved JSON number literal [`OwnedValue::to_json_for_reindex`]
/// writes in place of a computed +Infinity (see [`NEG_INFINITY_SENTINEL`]
/// for -Infinity's spelling) -- #1083/#1087: originally `"1e999"`
/// (naturally overflowing, valid JSON syntax), but that let a reindexed
/// computed Infinity reparse into a `NumberLiteral(Float(inf), "1e999")`
/// bit-for-bit indistinguishable from a genuine document literal that
/// happened to be spelled the same way, misclassifying either direction
/// depending which one a given call site assumed. Redesigned to mirror
/// [`NAN_SENTINEL`]'s already-safe two-exponent-marker trick: guaranteed
/// unparseable as an ordinary number (so it can never collide with real
/// document text), intercepted early by [`is_infinity_sentinel`] the same
/// way `is_nan_sentinel` intercepts its sibling, and still built entirely
/// from `[0-9-+.eE]` so `JsonNumber::find_end()`'s span scan captures it
/// whole. Distinguished from `NAN_SENTINEL` (`"9e999e999"`) by leading
/// digit rather than a sign prefix, so the sign survives as plain text for
/// [`is_infinity_sentinel`] to read back off, uniformly for both spellings.
pub(crate) const INFINITY_SENTINEL: &str = "8e999e999";

/// The negative-Infinity sibling of [`INFINITY_SENTINEL`].
pub(crate) const NEG_INFINITY_SENTINEL: &str = "-8e999e999";

/// `true`/`false` for [`NEG_INFINITY_SENTINEL`]/[`INFINITY_SENTINEL`],
/// `None` for anything else -- the one definition every call site that
/// reads a `to_json_for_reindex`-bridge number token must check before
/// falling back to ordinary parsing, mirroring [`is_nan_sentinel`]'s own
/// doc comment and existing call-site list exactly (#1083/#1087).
pub(crate) fn is_infinity_sentinel(bytes: &[u8]) -> Option<bool> {
    if bytes == NEG_INFINITY_SENTINEL.as_bytes() {
        Some(true)
    } else if bytes == INFINITY_SENTINEL.as_bytes() {
        Some(false)
    } else {
        None
    }
}

/// The correctly-signed sentinel text, used only by
/// [`OwnedValue::to_json_for_reindex`] to smuggle ±Infinity through a
/// JSON-text round-trip -- see [`INFINITY_SENTINEL`]'s own doc comment for
/// why a guaranteed-unparseable (rather than naturally-overflowing)
/// spelling is what makes this safe.
fn overflow_literal(f: f64) -> &'static str {
    if f.is_sign_negative() {
        NEG_INFINITY_SENTINEL
    } else {
        INFINITY_SENTINEL
    }
}

/// The exact text real jq's error-message value previews use for an
/// infinite `f64` reached via computation (the `infinite`/`-infinite`
/// builtins, an arithmetic overflow, or any other value with no source
/// literal text of its own to echo) -- `DBL_MAX`'s shortest round-trip
/// decimal representation, per jq's `jv` (issue #930). Unlike
/// [`overflow_literal`] above, this is for display, not round-tripping, so
/// it uses jq's real number text rather than a smuggling sentinel.
///
/// Hardcoded rather than derived: `DBL_MAX` is a fixed constant, so its
/// shortest round-trip text never changes, and computing it via the
/// `log10`/`pow`-based path the finite branch of
/// [`format_number_jq_compat`] uses would hit the exact `NaN`-mantissa
/// failure that path only avoids by never being called on a non-finite
/// input in the first place.
pub(crate) fn infinite_float_preview_text(negative: bool) -> &'static str {
    if negative {
        "-1.7976931348623157e+308"
    } else {
        "1.7976931348623157e+308"
    }
}

/// `to_json_yq`'s `infinite_fmt` for yq mode (#1087): keeps the
/// pre-existing `null` substitution. Real yq's own Go `encoding/json`
/// refuses to marshal Infinity at all and errors instead, so replicating
/// jq's `DBL_MAX`-text behavior here would not actually match either
/// oracle -- an open design question this fn deliberately doesn't resolve,
/// see #1087's own text.
fn yq_infinite_float_json_text(_negative: bool) -> &'static str {
    "null"
}

/// The reserved JSON number literal [`OwnedValue::to_json_for_reindex`]
/// writes in place of NaN (see [`overflow_literal`] for the analogous,
/// naturally-parsing ±Infinity trick, #561). NaN has no decimal literal that
/// IEEE-754 overflow parses to, so this reserves an explicit,
/// guaranteed-unparseable-as-a-real-number token instead (#472): digit-led,
/// so `JsonCursor::value()`'s dispatcher (`src/json/light.rs`, which only
/// recognizes `-`/an ASCII digit as the start of a `Number`) routes it to
/// `StandardJson::Number` rather than `Error`; built entirely from
/// `[0-9-+.eE]`, so `JsonNumber::find_end()`'s greedy span scan captures it
/// whole; and carrying two exponent markers, so `str::parse::<f64>()`/
/// `::<i64>()` both reject it outright rather than silently overflowing to
/// something else -- see `test_nan_sentinel_is_unparseable_as_a_real_number`,
/// the load-bearing proof this design depends on.
pub(crate) const NAN_SENTINEL: &str = "9e999e999";

/// True if `bytes` is exactly [`NAN_SENTINEL`] -- the one definition every
/// jq-layer call site that reads a `to_json_for_reindex`-bridge number token
/// must check before falling back to ordinary parsing, so the comparison
/// can't diverge between call sites the way three copies of one predicate
/// did in #106.
pub(crate) fn is_nan_sentinel(bytes: &[u8]) -> bool {
    bytes == NAN_SENTINEL.as_bytes()
}

/// jq (widening, non-strict) value equality -- [`OwnedValue`]'s `PartialEq`
/// impl, and the numeric rule [`owned_value_eq`] falls back to whenever
/// `EvalSemantics::STRICT_NUMERIC_EQUALITY` is unset (jq mode) or an
/// operand isn't numeric. Under yq's stricter rule, equality-consuming
/// builtins route through [`owned_value_eq`] instead of this impl
/// directly (#950) -- see that function's doc comment for the full list
/// and for why a single shared entry point matters here.
///
/// This is deliberately **not** `#[derive]`d. Deriving would compare the
/// representation, so `Int(1)` and `Float(1.0)` -- two spellings of the same
/// JSON number -- would be unequal, and `1 == 1.0` would evaluate to `false`
/// (jq says `true`).
///
/// Semantics, pinned against jq-1.7.1:
///
/// - Numbers compare by value across representations: `1 == 1.0`,
///   `[1] == [1.0]`, `{"a":1} == {"a":1.0}`.
/// - `Int`/`Int` compares exactly as `i64`, so integers beyond 2^53 keep their
///   precision: `9007199254740993 == 9007199254740992` is `false`, as in jq 1.7.
/// - NaN is never equal to itself. This makes the relation partial (hence
///   `PartialEq` and no `Eq`), and matches jq: `nan == nan` is `false`. It is
///   also why this cannot be written as `compare_values(..) == Equal` -- since
///   #421, `compare_values` orders NaN as strictly `Less` than every number,
///   including another NaN, so `<` can match jq; reusing that for `==` would
///   make `nan == nan` true exactly when two NaNs compare `Less`, which is
///   what `<` needs, not what `==` needs.
/// - `-0.0 == 0` and `-0.0 == 0.0`, as in jq.
/// - Objects compare order-insensitively (`IndexMap`'s own `PartialEq`), as in jq.
///
/// Known divergence: above 2^53 a mixed `Int`/`Float` comparison widens the
/// integer to `f64`, whereas jq 1.7 retains the decimal literal. So
/// `9007199254740993 == 9007199254740992.0` is `true` here and `false` in jq.
/// Every value representable exactly as an `f64` agrees.
///
/// `NumberLiteral` compares purely on its parsed [`NumberRepr`], never on the
/// source text -- two spellings of the same number (`1.0` and `1e0`) are
/// equal, matching every other numeric comparison here.
pub(crate) fn numeric_repr_eq(a: NumberRepr, b: NumberRepr) -> bool {
    match (a, b) {
        (NumberRepr::Int(a), NumberRepr::Int(b)) => a == b,
        (NumberRepr::Float(a), NumberRepr::Float(b)) => a == b,
        (NumberRepr::Int(a), NumberRepr::Float(b)) => (a as f64) == b,
        (NumberRepr::Float(a), NumberRepr::Int(b)) => a == (b as f64),
    }
}

/// Like [`numeric_repr_eq`], but for `EvalSemantics::STRICT_NUMERIC_EQUALITY`
/// (yq): an `Int` and a `Float` are never equal regardless of magnitude,
/// even when the same pair would compare equal under [`numeric_repr_eq`]'s
/// widening rule -- real yq treats `2` and `2.0` as genuinely distinct
/// types, unlike jq (#950). This is exactly [`NumberRepr`]'s own derived
/// `PartialEq` (same variant and value, never equal across variants) --
/// spelled out as its own named function so every strict-mode call site
/// reads as "yq's equality rule" rather than an easy-to-miss bare `==`.
pub(crate) fn numeric_repr_eq_strict(a: NumberRepr, b: NumberRepr) -> bool {
    a == b
}

/// jq/yq value equality, generic over `EvalSemantics::STRICT_NUMERIC_EQUALITY`
/// (#950) -- the single definition every equality-consuming builtin should
/// route through (`==`, `!=`, array `-`, `contains`, `inside`, `index`,
/// `indices`, `rindex`, `IN`, `unique`, `unique_by`, `group_by`), so none
/// of them can individually diverge from `==`'s own answer about the same
/// pair of values (a real gap #950's own review round found: several of
/// these builtins still used the plain, always-widening `PartialEq` after
/// the first pass only fixed `==`/`!=` themselves).
///
/// Structural rules (`Null`/`Bool`/`String`/`Array`/`Object`) are
/// identical in both modes -- only how two *numbers* compare differs, so
/// this mirrors [`OwnedValue`]'s own `PartialEq`
/// (`owned_value_eq_at_depth`) exactly, just threading `S` into the
/// recursion so strictness applies at every nesting depth, not only the
/// top: `[2.0] == [2]` and `{"a":2.0} == {"a":2}` are `false` in yq,
/// matching jq's `true` when `S` isn't strict.
pub(crate) fn owned_value_eq<S: EvalSemantics>(a: &OwnedValue, b: &OwnedValue) -> bool {
    owned_value_eq_at_depth_generic::<S>(a, b, 0)
}

fn owned_value_eq_at_depth_generic<S: EvalSemantics>(
    a: &OwnedValue,
    b: &OwnedValue,
    depth: usize,
) -> bool {
    assert_value_tree_depth(depth);
    match (a, b) {
        (OwnedValue::Array(a), OwnedValue::Array(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| owned_value_eq_at_depth_generic::<S>(x, y, depth + 1))
        }
        (OwnedValue::Object(a), OwnedValue::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(k, v)| {
                    b.get(k)
                        .is_some_and(|bv| owned_value_eq_at_depth_generic::<S>(v, bv, depth + 1))
                })
        }
        // Every other pairing (Null/Bool/String, Int/Float/NumberLiteral,
        // and any type mismatch) has no further nesting to thread through.
        // Only checked only when *both* operands are numeric under strict
        // mode; a number-vs-non-number comparison (already `false` either
        // way) and every jq-mode comparison fall straight through to the
        // ordinary widening `PartialEq`.
        _ => {
            if S::STRICT_NUMERIC_EQUALITY {
                if let (Some(x), Some(y)) = (a.number_repr(), b.number_repr()) {
                    return numeric_repr_eq_strict(x, y);
                }
            }
            a == b
        }
    }
}

/// Order two `f64`s the way jq's own comparator does: NaN sorts strictly
/// below every other float, including another NaN (#421).
///
/// `f64::partial_cmp` returns `None` whenever either operand is NaN, which is
/// why every caller here used to fall back to `Ordering::Equal` -- silently
/// treating NaN as equal to any number instead of ordered against it. Real jq
/// (verified against jq-1.7.1) instead treats NaN as smaller than anything,
/// even itself:
///
/// ```text
/// nan <  1    -> true      nan >  1    -> false
/// nan <= 1    -> true      nan >= 1    -> false
/// 1   <  nan  -> false     1   >  nan  -> true
/// nan <  nan  -> true      nan <= nan  -> true
/// nan >= nan  -> false     nan == nan  -> false  (a separate, hand-written `PartialEq`)
/// ```
///
/// Deliberately **not** a strict weak ordering: `cmp_f64(NaN, NaN)` answers
/// `Less` regardless of which argument is which, so both directions between
/// the same two NaNs report `Less` -- a genuine antisymmetry violation. This
/// matches jq's own comparator exactly (its C `qsort`-based sort has the
/// identical property); see [`super::eval::compare_values`]'s doc comment for
/// how narrow the practical fallout of that is.
pub(crate) fn cmp_f64(a: f64, b: f64) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    if a.is_nan() {
        Ordering::Less
    } else if b.is_nan() {
        Ordering::Greater
    } else {
        a.partial_cmp(&b).expect("neither operand is NaN")
    }
}

/// Order two parsed number representations with the exact same per-pair
/// dispatch [`numeric_repr_eq`] uses for equality -- exact `i64` comparison
/// when both are `Int` (so integers beyond 2^53 keep precision, matching the
/// evaluators' plain `(Int, Int)` ordering arm), exact `f64` comparison when
/// both are `Float`, and `f64`-widening for a mixed pair (the same
/// precision-losing widen the mixed `Int`/`Float` ordering arms use).
///
/// This exists so a `NumberLiteral` operand orders consistently with how it
/// compares equal: before this, `compare_values` tried exact `i64` first for
/// *any* `NumberLiteral` pair while `==` always widened mixed pairs to `f64`,
/// so `9007199254740993 == 9007199254740992.0` could be `true` while
/// `9007199254740993 > 9007199254740992.0` was also `true` -- sort and
/// `unique`/`group_by` disagreeing with `==` about the same two numbers.
///
/// The NaN rule (#421) is centralized in [`cmp_f64`], not repeated here.
pub(crate) fn numeric_repr_cmp(a: NumberRepr, b: NumberRepr) -> core::cmp::Ordering {
    match (a, b) {
        (NumberRepr::Int(a), NumberRepr::Int(b)) => a.cmp(&b),
        (NumberRepr::Float(a), NumberRepr::Float(b)) => cmp_f64(a, b),
        (NumberRepr::Int(a), NumberRepr::Float(b)) => cmp_f64(a as f64, b),
        (NumberRepr::Float(a), NumberRepr::Int(b)) => cmp_f64(a, b as f64),
    }
}

impl PartialEq for OwnedValue {
    fn eq(&self, other: &Self) -> bool {
        owned_value_eq_at_depth(self, other, 0)
    }
}

/// Panics past [`MAX_VALUE_TREE_DEPTH`] levels of nesting (#1005) — see
/// that constant's own doc comment for why.
///
/// Can't thread a depth counter through [`PartialEq::eq`] itself (the trait
/// method's signature is fixed), so `Array`/`Object` recurse into this
/// helper directly rather than delegating to `Vec`'s/`IndexMap`'s own `==`
/// (which would call back into [`PartialEq::eq`] and silently reset the
/// depth count to 0 every level, defeating the guard). `Array` mirrors
/// `Vec`'s own positional `==`; `Object` mirrors `IndexMap`'s own
/// order-independent `==` (same length, and every key in `a` maps to an
/// equal value in `b`) — changing either's semantics here would make `==`
/// disagree with itself depending on nesting depth.
fn owned_value_eq_at_depth(a: &OwnedValue, b: &OwnedValue, depth: usize) -> bool {
    assert_value_tree_depth(depth);
    match (a, b) {
        (OwnedValue::Null, OwnedValue::Null) => true,
        (OwnedValue::Bool(a), OwnedValue::Bool(b)) => a == b,
        (OwnedValue::Int(a), OwnedValue::Int(b)) => a == b,
        (OwnedValue::Float(a), OwnedValue::Float(b)) => a == b,
        (OwnedValue::Int(a), OwnedValue::Float(b)) => (*a as f64) == *b,
        (OwnedValue::Float(a), OwnedValue::Int(b)) => *a == (*b as f64),
        (OwnedValue::NumberLiteral(a, _), OwnedValue::NumberLiteral(b, _)) => {
            numeric_repr_eq(*a, *b)
        }
        (OwnedValue::NumberLiteral(a, _), OwnedValue::Int(b))
        | (OwnedValue::Int(b), OwnedValue::NumberLiteral(a, _)) => {
            numeric_repr_eq(*a, NumberRepr::Int(*b))
        }
        (OwnedValue::NumberLiteral(a, _), OwnedValue::Float(b))
        | (OwnedValue::Float(b), OwnedValue::NumberLiteral(a, _)) => {
            numeric_repr_eq(*a, NumberRepr::Float(*b))
        }
        (OwnedValue::String(a), OwnedValue::String(b)) => a == b,
        (OwnedValue::Array(a), OwnedValue::Array(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| owned_value_eq_at_depth(x, y, depth + 1))
        }
        (OwnedValue::Object(a), OwnedValue::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(k, v)| {
                    b.get(k)
                        .is_some_and(|bv| owned_value_eq_at_depth(v, bv, depth + 1))
                })
        }
        _ => false,
    }
}

/// The single `Literal -> OwnedValue` conversion every call site should go
/// through (#1062, same pattern as `from_number_literal_boxed`'s own #966
/// fix): `eval.rs`'s `literal_to_owned` delegates here rather than
/// re-matching the same six variants itself. `lazy.rs`'s `JqValue::from_literal`
/// also delegates here for every variant except `NumberLiteral` -- `JqValue`
/// defers that one's parsing until read, a genuine target-type difference
/// this impl doesn't share, so it can't fully delegate.
impl From<Literal> for OwnedValue {
    fn from(lit: Literal) -> Self {
        match lit {
            Literal::Null => Self::Null,
            Literal::Bool(b) => Self::Bool(b),
            // #1062: `repr` was already parsed by `parse`'s tokenizer (the
            // same `parse_i64_or_f64` `from_number_literal_boxed` would run
            // again here) -- constructed directly instead of re-parsing
            // `text` and re-deriving the identical value on every visit of
            // this AST node.
            //
            // Every real construction site keeps `repr`/`text` in sync
            // (`Literal`'s tuple fields have no runtime enforcement of
            // that, since neither `Literal` nor `NumberRepr` can carry
            // private fields as a public enum) -- this debug-only check
            // catches a hand-built mismatch in tests/debug builds rather
            // than silently trusting `repr` the way release builds still
            // do, restoring some of what `from_number_literal_boxed`'s
            // unconditional re-parse used to guarantee for free.
            Literal::NumberLiteral(repr, text) => {
                debug_assert_eq!(
                    Some(repr),
                    parse_i64_or_f64(&text),
                    "Literal::NumberLiteral's repr {repr:?} doesn't match a fresh parse of its own text {text:?}"
                );
                Self::NumberLiteral(repr, text.into())
            }
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

    /// #1171: a leading-dot number literal (with or without a `-` sign)
    /// must preserve its own source spelling as a `NumberLiteral`, not
    /// degrade to a plain lossy `Float` -- direct unit coverage of both
    /// `dot_prefix_len` branches in `from_number_bytes` (0 for `.5`, 1
    /// for `-.5`), complementing the CLI-level regression tests.
    #[test]
    fn test_from_number_bytes_preserves_leading_dot_spelling() {
        assert_eq!(
            OwnedValue::from_number_bytes(b".500"),
            OwnedValue::NumberLiteral(NumberRepr::Float(0.5), ".500".into())
        );
        assert_eq!(
            OwnedValue::from_number_bytes(b"-.500"),
            OwnedValue::NumberLiteral(NumberRepr::Float(-0.5), "-.500".into())
        );
        // A bare `.` (no digit at all) still degrades to Null: `dot_prefix_len`
        // is set, but prefixing `0` doesn't make it strictly valid either.
        assert_eq!(OwnedValue::from_number_bytes(b"."), OwnedValue::Null);
        assert_eq!(OwnedValue::from_number_bytes(b"-."), OwnedValue::Null);
    }

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

    /// `to_json` is what `tojson`, `@json`, `tostring` and string interpolation
    /// all render through, so its escaping has to be jq's. Pinned against
    /// jq-1.7.1 (#385):
    ///
    /// ```console
    /// $ printf '"a\\u0001b\\u007fc\\u0085d\\u0008e\\u000cf"' | jq -r tojson | od -An -c
    ///     "   a   \   u   0   0   0   1   b   \   u   0   0   7   f   c
    ///   302 205   d   \   b   e   \   f   f   "  \n
    /// ```
    #[test]
    fn test_to_json_string_escaping_matches_jq() {
        let json = |s: &str| OwnedValue::String(s.into()).to_json();

        // Backspace and form feed take jq's short forms, not the long
        // \u0008/\u000c that yq uses.
        assert_eq!(json("\u{8}\u{c}"), "\"\\b\\f\"");
        // Other C0 controls, and DEL, take the long form.
        assert_eq!(json("\u{1}\u{1f}"), "\"\\u0001\\u001f\"");
        assert_eq!(json("\u{7f}"), "\"\\u007f\"");
        // C1 controls are emitted raw: `char::is_control()` covers them but
        // JSON does not require escaping them and jq does not escape them.
        assert_eq!(json("a\u{85}b"), "\"a\u{85}b\"");
        assert_eq!(json("\u{80}\u{9f}"), "\"\u{80}\u{9f}\"");
        // Non-ASCII is untouched, above and below the BMP.
        assert_eq!(json("café 😀"), "\"café 😀\"");
        // Object keys take the same treatment as values.
        let mut obj = IndexMap::new();
        obj.insert("k\u{85}\u{8}".to_string(), OwnedValue::Int(1));
        assert_eq!(OwnedValue::Object(obj).to_json(), "{\"k\u{85}\\b\":1}");
    }

    /// Whatever `to_json` emits has to parse back to the value it came from —
    /// the escaping change must not make a control character round-trip badly.
    #[test]
    fn test_to_json_round_trips_through_a_parser() {
        for cp in (0u32..=0xFF).chain([0x2028, 0x1F600]) {
            let c = char::from_u32(cp).unwrap();
            let value = OwnedValue::String(String::from(c));
            let json = value.to_json();
            let parsed: serde_json::Value =
                serde_json::from_str(&json).unwrap_or_else(|e| panic!("U+{cp:04X}: {json:?}: {e}"));
            assert_eq!(
                parsed.as_str(),
                Some(String::from(c).as_str()),
                "U+{cp:04X}"
            );
        }
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
        // #1035: a filter-literal number keeps its own source spelling
        // through this conversion too, same as `literal_to_owned`'s
        // sibling in eval.rs.
        match OwnedValue::from(Literal::number_literal("1.500".to_string())) {
            OwnedValue::NumberLiteral(NumberRepr::Float(f), text) => {
                assert_eq!(f, 1.5);
                assert_eq!(text.as_ref(), "1.500");
            }
            other => panic!("expected NumberLiteral, got {other:?}"),
        }
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
        // expressed as `compare_values(..) == Equal` -- `compare_values` orders
        // NaN as strictly `Less` than every number, including another NaN
        // (#421), not `Equal`.
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

    // =========================================================================
    // NumberLiteral (#387: tostring/tojson lose a number's source literal)
    // =========================================================================

    #[test]
    fn test_from_number_literal_picks_int_or_float() {
        assert_eq!(
            OwnedValue::from_number_literal("42"),
            OwnedValue::NumberLiteral(NumberRepr::Int(42), "42".into())
        );
        assert_eq!(
            OwnedValue::from_number_literal("1.0"),
            OwnedValue::NumberLiteral(NumberRepr::Float(1.0), "1.0".into())
        );
        assert_eq!(
            OwnedValue::from_number_literal("1e100"),
            OwnedValue::NumberLiteral(NumberRepr::Float(1e100), "1e100".into())
        );
    }

    #[test]
    fn test_number_literal_eq_matches_plain_int_and_float() {
        // A NumberLiteral is numerically equal to the plain variant it parses
        // to, and to the other representation of the same number -- equality
        // never looks at the source text (#387).
        assert_eq!(OwnedValue::from_number_literal("42"), OwnedValue::Int(42));
        assert_eq!(OwnedValue::Int(42), OwnedValue::from_number_literal("42"));
        assert_eq!(
            OwnedValue::from_number_literal("1.0"),
            OwnedValue::Float(1.0)
        );
        assert_eq!(OwnedValue::from_number_literal("1.0"), OwnedValue::Int(1));
        assert_eq!(
            OwnedValue::from_number_literal("1e0"),
            OwnedValue::from_number_literal("1")
        );
        assert_ne!(OwnedValue::from_number_literal("1.5"), OwnedValue::Int(1));
    }

    #[test]
    fn test_number_repr_agrees_with_partial_eq_across_variants() {
        // `number_repr` must return the same `NumberRepr` regardless of which
        // of the three numeric variants a value happens to be in, and
        // `numeric_repr_cmp` must call two values `Equal` exactly when
        // `numeric_repr_eq` calls them equal -- otherwise ordering-based
        // consumers (`sort`, `group_by`, `unique`) can disagree with `==`
        // about the very same pair (#387: this actually happened for a
        // `NumberLiteral` operand above 2^53, before `compare_values` was
        // rewritten to share this exact dispatch with equality).
        let pairs = [
            (OwnedValue::Int(42), OwnedValue::from_number_literal("42")),
            (
                OwnedValue::Float(1.5),
                OwnedValue::from_number_literal("1.5"),
            ),
            (
                OwnedValue::from_number_literal("1.0"),
                OwnedValue::from_number_literal("1e0"),
            ),
            // The precision-boundary case #387 regressed: a `NumberLiteral`
            // `Int` above 2^53 against a `Float` that collapses to the same
            // `f64` value on widening.
            (
                OwnedValue::from_number_literal("9007199254740993"),
                OwnedValue::Float(9007199254740992.0),
            ),
        ];
        for (a, b) in pairs {
            let are_eq = a == b;
            let ra = a.number_repr().expect("numeric operand");
            let rb = b.number_repr().expect("numeric operand");
            assert_eq!(
                numeric_repr_eq(ra, rb),
                are_eq,
                "numeric_repr_eq disagrees with OwnedValue::eq for {a:?} vs {b:?}"
            );
            assert_eq!(
                numeric_repr_cmp(ra, rb) == core::cmp::Ordering::Equal,
                are_eq,
                "numeric_repr_cmp disagrees with OwnedValue::eq for {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn test_number_repr_is_none_for_non_numeric_variants() {
        assert_eq!(OwnedValue::Null.number_repr(), None);
        assert_eq!(OwnedValue::Bool(true).number_repr(), None);
        assert_eq!(OwnedValue::String("x".into()).number_repr(), None);
        assert_eq!(OwnedValue::array().number_repr(), None);
        assert_eq!(OwnedValue::object().number_repr(), None);
    }

    #[test]
    fn test_number_literal_type_name_and_conversions() {
        let lit = OwnedValue::from_number_literal("1e100");
        assert_eq!(lit.type_name(), "number");
        assert_eq!(lit.as_f64(), Some(1e100));
        assert_eq!(lit.as_i64(), None); // not integral

        let int_lit = OwnedValue::from_number_literal("42");
        assert_eq!(int_lit.as_i64(), Some(42));
        assert_eq!(int_lit.as_f64(), Some(42.0));

        // A NumberLiteral backed by an integral Float representation also
        // converts to i64, just like plain OwnedValue::Float does.
        let float_int_lit = OwnedValue::from_number_literal("2.0");
        assert_eq!(float_int_lit.as_i64(), Some(2));
    }

    #[test]
    fn test_number_literal_to_json_preserves_source_spelling() {
        // Preserved verbatim where jq's own canonical formatting agrees with
        // the source text...
        assert_eq!(OwnedValue::from_number_literal("42").to_json(), "42");
        assert_eq!(OwnedValue::from_number_literal("1.0").to_json(), "1.0");
        assert_eq!(OwnedValue::from_number_literal("-0.0").to_json(), "-0.0");
        // ...and reformatted into jq's canonical spelling where it doesn't --
        // this is the exact repro from #387, pinned against jq-1.7.1.
        assert_eq!(OwnedValue::from_number_literal("1e100").to_json(), "1E+100");
        assert_eq!(OwnedValue::from_number_literal("1e-7").to_json(), "1E-7");
    }

    #[test]
    fn test_number_literal_to_json_overflow_to_infinity_echoes_mantissa_1087() {
        // "1e400" overflows f64 to infinity during parsing even though the
        // source text is a normal-looking (if extreme) JSON number token.
        // Unlike a bare computed `Float` (which has no source text and
        // renders `DBL_MAX` text), a `NumberLiteral` still has one to fall
        // back to -- confirmed live against jq 1.7.1, `1e400 | .` (identity)
        // echoes `1E+400`, not `null` (#1087; this test's own name/premise
        // predates that finding).
        let lit = OwnedValue::from_number_literal("1e400");
        assert!(matches!(
            lit,
            OwnedValue::NumberLiteral(NumberRepr::Float(f), _) if f.is_infinite()
        ));
        assert_eq!(lit.to_json(), "1E+400");
    }

    #[test]
    fn test_computed_float_to_json_infinity_is_dbl_max_text_1087() {
        // A genuinely *computed* Infinity (no source literal to echo, e.g.
        // the `infinite` builtin or an arithmetic overflow) renders jq's own
        // `DBL_MAX` text instead -- confirmed live against jq 1.7.1, `null |
        // infinite` is `1.7976931348623157e+308`.
        assert_eq!(
            OwnedValue::Float(f64::INFINITY).to_json(),
            "1.7976931348623157e+308"
        );
        assert_eq!(
            OwnedValue::Float(f64::NEG_INFINITY).to_json(),
            "-1.7976931348623157e+308"
        );
        // NaN still has no fallback text in either case and stays `null`.
        assert_eq!(OwnedValue::Float(f64::NAN).to_json(), "null");
    }

    #[test]
    fn test_nan_sentinel_is_unparseable_as_a_real_number() {
        // Load-bearing for #472: `NAN_SENTINEL` is only safe to reserve as an
        // out-of-band NaN marker because it can never arise from a
        // legitimately formatted number -- both parses must fail today.
        assert!(NAN_SENTINEL.parse::<f64>().is_err());
        assert!(NAN_SENTINEL.parse::<i64>().is_err());
    }

    #[test]
    fn test_to_json_for_reindex_preserves_nan() {
        // NaN has no natural JSON-number spelling, so `to_json_for_reindex`
        // must emit the reserved sentinel instead of falling back to
        // `to_json`'s "null" substitution (#472).
        assert_eq!(
            OwnedValue::Float(f64::NAN).to_json_for_reindex::<JqSemantics>(),
            NAN_SENTINEL
        );
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::NAN), "nan".into());
        assert_eq!(lit.to_json_for_reindex::<JqSemantics>(), NAN_SENTINEL);
    }

    /// #1083/#1087: the infinity sentinel must be as collision-safe as
    /// `NAN_SENTINEL` already is -- guaranteed unparseable as an ordinary
    /// number (so `from_number_bytes` can reliably recognize it as *the*
    /// sentinel rather than a genuine document literal that happens to
    /// share its spelling), yet still recoverable via
    /// [`is_infinity_sentinel`] once reparsed.
    #[test]
    fn test_infinity_sentinel_is_unparseable_but_recoverable_1087() {
        assert!(INFINITY_SENTINEL.parse::<f64>().is_err());
        assert!(INFINITY_SENTINEL.parse::<i64>().is_err());
        assert!(NEG_INFINITY_SENTINEL.parse::<f64>().is_err());
        assert!(NEG_INFINITY_SENTINEL.parse::<i64>().is_err());

        assert_eq!(
            is_infinity_sentinel(INFINITY_SENTINEL.as_bytes()),
            Some(false)
        );
        assert_eq!(
            is_infinity_sentinel(NEG_INFINITY_SENTINEL.as_bytes()),
            Some(true)
        );
        assert_eq!(is_infinity_sentinel(b"1e400"), None);
        assert_eq!(is_infinity_sentinel(NAN_SENTINEL.as_bytes()), None);

        // Round-trips correctly through the same public entry point real
        // document numbers go through.
        assert_eq!(
            OwnedValue::from_number_bytes(INFINITY_SENTINEL.as_bytes()),
            OwnedValue::Float(f64::INFINITY)
        );
        assert_eq!(
            OwnedValue::from_number_bytes(NEG_INFINITY_SENTINEL.as_bytes()),
            OwnedValue::Float(f64::NEG_INFINITY)
        );

        // A genuine document literal spelled coincidentally like the *old*
        // (pre-#1083/#1087) sentinel no longer collides with anything --
        // it round-trips as an ordinary overflow literal now.
        assert_eq!(
            OwnedValue::from_number_bytes(b"1e999"),
            OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "1e999".into())
        );
    }

    /// #939: an infinite `NumberLiteral` backed by real document text (any
    /// magnitude-overflow literal, e.g. `123e400`) is already guaranteed to
    /// reparse to this exact value - reusing it avoids the disconnected
    /// `1e999`/`-1e999` sentinel leaking into a downstream preview (#930's
    /// `format_overflow_literal_mantissa` can then reformat the *real* text
    /// instead of the sentinel's).
    #[test]
    fn test_to_json_for_reindex_reuses_overflow_literal_text() {
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), "123e400".into());
        assert_eq!(lit.to_json_for_reindex::<JqSemantics>(), "123e400");

        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::NEG_INFINITY), "-1e400".into());
        assert_eq!(lit.to_json_for_reindex::<JqSemantics>(), "-1e400");
    }

    /// #939 review: reusing the literal's own text is O(its length), and
    /// this function's callers (`reduce`/`foreach`'s per-iteration reindex
    /// bridge) can run it over the same unchanged value many times - so an
    /// unbounded literal must fall back to the O(1) sentinel rather than
    /// turning a loop into O(iterations x literal length). No realistic
    /// document overflow literal is anywhere near this long (#930's own
    /// tests only go up to a handful of digits before the exponent), so the
    /// cap only ever trips on a pathological/adversarial one.
    #[test]
    fn test_to_json_for_reindex_bounds_an_unrealistically_long_literal() {
        let huge_literal = format!("{}e400", "9".repeat(300));
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(f64::INFINITY), huge_literal.into());
        assert_eq!(lit.to_json_for_reindex::<JqSemantics>(), INFINITY_SENTINEL);
    }

    #[test]
    fn test_number_literal_number_str_matches_to_json() {
        let lit = OwnedValue::from_number_literal("1e100");
        assert_eq!(lit.number_str().as_deref(), Some("1E+100"));
        assert_eq!(OwnedValue::Null.number_str(), None);

        // Plain Int/Float variants (not just NumberLiteral) also produce a
        // number_str, via Display rather than format_number_jq_compat.
        assert_eq!(OwnedValue::Int(7).number_str().as_deref(), Some("7"));
        assert_eq!(OwnedValue::Float(2.5).number_str().as_deref(), Some("2.5"));
    }

    #[test]
    fn test_from_number_literal_boxed_falls_back_to_null_for_unparseable_text() {
        // Callers gate on `is_valid_number` before reaching this (#966), but
        // it's defensive: neither an i64 nor f64 parse succeeding falls back
        // to `Null` (matching every other decode-failure path in this
        // codebase) rather than panicking or silently producing `0`.
        assert_eq!(
            OwnedValue::from_number_literal("not-a-number"),
            OwnedValue::Null
        );
    }

    #[test]
    fn test_into_plain_number_drops_the_literal() {
        // Once a value is treated as a fresh computed number (what every
        // arithmetic operator does before matching), the literal is gone and
        // formatting falls back to plain Int/Float Display -- matching jq,
        // where only untouched values keep their original spelling.
        assert_eq!(
            OwnedValue::from_number_literal("1e100").into_plain_number(),
            OwnedValue::Float(1e100)
        );
        assert_eq!(
            OwnedValue::from_number_literal("42").into_plain_number(),
            OwnedValue::Int(42)
        );
        // No-op for every other variant.
        assert_eq!(
            OwnedValue::Bool(true).into_plain_number(),
            OwnedValue::Bool(true)
        );
        assert_eq!(
            OwnedValue::String("x".into()).into_plain_number(),
            OwnedValue::String("x".into())
        );
    }

    #[test]
    fn test_format_number_jq_compat_matches_jq_1_7_1() {
        // Pinned against jq-1.7.1 -- the exact cases from #387's repro.
        assert_eq!(format_number_jq_compat(b"1e100"), "1E+100");
        assert_eq!(format_number_jq_compat(b"1.0"), "1.0");
        assert_eq!(format_number_jq_compat(b"-0.0"), "-0.0");
        assert_eq!(format_number_jq_compat(b"1e-7"), "1E-7");
        assert_eq!(format_number_jq_compat(b"42"), "42");
        // e0/e-0 drop the exponent entirely.
        assert_eq!(format_number_jq_compat(b"5.5e0"), "5.5");
        // Negative exponents within [-5, -1] expand to decimal.
        assert_eq!(format_number_jq_compat(b"1e-3"), "0.001");
    }

    #[test]
    fn test_format_number_jq_compat_invalid_utf8_falls_back_to_lossy() {
        // Not a real document number (the parser only ever hands this valid
        // UTF-8), but the function is `pub` and takes raw bytes, so it must
        // not panic on garbage.
        let raw = &[0xFF, 0xFE][..];
        assert_eq!(
            format_number_jq_compat(raw),
            String::from_utf8_lossy(raw).into_owned()
        );
    }

    #[test]
    fn test_format_number_jq_compat_unparseable_exponent_falls_back_to_raw() {
        // "1e" contains 'e' but has no exponent digits, so it fails the f64
        // parse and falls back to echoing the input unchanged.
        assert_eq!(format_number_jq_compat(b"1e"), "1e");
    }

    #[test]
    fn test_format_number_jq_compat_e0_integer_drops_exponent() {
        // e0 eliminates the exponent; when the result is a whole number it
        // takes the integer-formatting branch rather than float Display.
        assert_eq!(format_number_jq_compat(b"5e0"), "5");
    }

    /// #1008 code review: this was pinned to `"0"`, but real jq keeps the
    /// exponent for a zero mantissa (verified against the pinned oracle:
    /// `echo '{"a": 0e10}' | jq '.a'` -> `0E+10`) -- the old assertion
    /// matched a bug (the scientific-notation branch's `value == 0.0` early
    /// return dropped both sign and exponent for any zero-mantissa literal,
    /// found while fixing that same branch's separate `-0.0` sign loss).
    #[test]
    fn test_format_number_jq_compat_zero_with_positive_exponent() {
        assert_eq!(format_number_jq_compat(b"0e10"), "0E+10");
    }

    #[test]
    fn test_format_number_jq_compat_non_integer_mantissa() {
        // Docstring example: `12e2` -> `1.2E+3`. Exercises the mantissa's
        // shortest-round-tripping-precision loop (the integer fast path
        // covers only whole-number mantissas like `1`).
        assert_eq!(format_number_jq_compat(b"12e2"), "1.2E+3");
    }

    #[test]
    fn test_format_number_jq_compat_negative_exponent_integer_decimal() {
        // Negative exponent that still resolves to a whole number takes
        // format_decimal_jq's integer fast path rather than its precision loop.
        assert_eq!(format_number_jq_compat(b"100e-2"), "1");
        assert_eq!(format_number_jq_compat(b"-100e-2"), "-1");
    }

    /// #930: a literal whose magnitude overflows f64 (e.g. `1e400`) used to
    /// route into the finite path's `log10`/`pow` renormalization, producing
    /// garbage (`"NaNE+2147483647"`) since that path needs a finite value to
    /// renormalize. Every case here is oracle-verified against real jq
    /// 1.7.1's `describe()`-equivalent value preview (`try (. + "s") catch
    /// .` on the given literal as JSON input, which reaches the value
    /// through `keys`/`.[]`-style operations that don't lose `NumberLiteral`
    /// status the way arithmetic does).
    #[test]
    fn test_format_number_jq_compat_overflow_literal_matches_jq_1_7_1() {
        assert_eq!(format_number_jq_compat(b"1e400"), "1E+400");
        assert_eq!(format_number_jq_compat(b"-1e400"), "-1E+400");
        assert_eq!(format_number_jq_compat(b"123e400"), "1.23E+402");
        assert_eq!(format_number_jq_compat(b"12.34e400"), "1.234E+401");
        assert_eq!(format_number_jq_compat(b"0.5e400"), "5E+399");
        assert_eq!(format_number_jq_compat(b"100e400"), "1.00E+402");
        assert_eq!(format_number_jq_compat(b"9.999e400"), "9.999E+400");
        assert_eq!(format_number_jq_compat(b"1E400"), "1E+400");
    }

    /// #930 review: a leading-dot mantissa (`.5e400`, no digit before the
    /// decimal point) has an *empty* integer part, which `int_part == "0"`
    /// alone doesn't catch - the first version of this fix took the
    /// `else` (mantissa >= 1) branch instead and panicked slicing
    /// `&int_part[..1]` on an empty string. Oracle-verified: real jq treats
    /// this identically to an explicit `0.5e400`.
    #[test]
    fn test_format_number_jq_compat_overflow_leading_dot_mantissa() {
        assert_eq!(format_number_jq_compat(b".5e400"), "5E+399");
        assert_eq!(format_number_jq_compat(b"-.5e400"), "-5E+399");
    }

    /// #930 review: jq's own literal-preservation only holds up to an
    /// exponent magnitude just under 1,000,000,000 (decNumber's limit) -
    /// beyond that, real jq falls back to `DBL_MAX` text, same as a
    /// *computed* infinity (`infinite`). The first version of this fix had
    /// no such ceiling, and separately parsed the exponent digits as `i32`
    /// with a silent `unwrap_or(0)` fallback - so an exponent that
    /// overflowed even that narrower range produced a flatly wrong `"1E+0"`
    /// instead of anything resembling the real magnitude.
    #[test]
    fn test_format_number_jq_compat_overflow_exponent_ceiling_matches_jq_1_7_1() {
        // Just at the boundary: still literal-preserving text.
        assert_eq!(format_number_jq_compat(b"1e999999999"), "1E+999999999");
        assert_eq!(format_number_jq_compat(b"9e999999999"), "9E+999999999");
        // One past the boundary: DBL_MAX text instead.
        assert_eq!(
            format_number_jq_compat(b"1e1000000000"),
            "1.7976931348623157e+308"
        );
        assert_eq!(
            format_number_jq_compat(b"-1e1000000000"),
            "-1.7976931348623157e+308"
        );
        // An exponent digit string past i64's own range: still DBL_MAX
        // text, not a garbage or silently-wrong value.
        assert_eq!(
            format_number_jq_compat(b"1e99999999999999999999"),
            "1.7976931348623157e+308"
        );
    }

    /// #930 review: the exponent-parsing fallback used to reuse
    /// `format_number_jq_compat`'s own `i32`-typed `exp`, which silently
    /// zeroed via `unwrap_or(0)` for any exponent digit string past
    /// `i32::MAX` (~10 digits) - producing `"1E+0"` for a value that's
    /// actually astronomically large, well before the real ceiling above
    /// even comes into play. `format_overflow_literal_mantissa` now
    /// re-parses the exponent text itself at `i64` precision instead.
    #[test]
    fn test_format_number_jq_compat_overflow_exponent_past_i32_matches_jq_1_7_1() {
        assert_eq!(
            format_number_jq_compat(b"1e2147483648"),
            "1.7976931348623157e+308"
        );
    }

    /// #930: a no-exponent overflow literal (e.g. a 400-digit plain integer)
    /// takes the function's pre-existing "no exponent -> output as-is" early
    /// return (before `value.parse()` is ever consulted), so it was already
    /// correct - jq itself doesn't reformat these into scientific notation
    /// either, it just echoes the digits (oracle-verified).
    #[test]
    fn test_format_number_jq_compat_overflow_no_exponent_is_unaffected() {
        let digits = "9".repeat(400);
        assert_eq!(format_number_jq_compat(digits.as_bytes()), digits);
    }

    /// #1099: the symmetric *underflow* case (a literal whose magnitude
    /// underflows `f64` to exactly `0.0`, e.g. `1e-400`) used to lose the
    /// mantissa entirely (`0E-400` instead of `1E-400`) -- `value == 0.0`
    /// can't tell a genuinely-zero-mantissa literal apart from a nonzero
    /// one that simply underflowed. In-process counterpart to
    /// `jq_cli_tests.rs`'s CLI-level coverage of the same cases: those
    /// spawn a subprocess and so are invisible to `cargo llvm-cov`, which
    /// only instruments the test binary itself.
    #[test]
    fn test_format_number_jq_compat_underflow_preserves_mantissa_1099() {
        assert_eq!(format_number_jq_compat(b"1e-400"), "1E-400");
        assert_eq!(format_number_jq_compat(b"-1e-400"), "-1E-400");
        assert_eq!(format_number_jq_compat(b"12.34e-400"), "1.234E-399");
        assert_eq!(format_number_jq_compat(b"0.5e-400"), "5E-401");
        assert_eq!(format_number_jq_compat(b"100.5e-400"), "1.005E-398");
    }

    /// #1099 code review: the exponent digit string was parsed at `i32`
    /// width for `format_number_jq_compat`'s own fast-path dispatch
    /// (`exp == 0` / `(-5..0)` checks), and an out-of-`i32`-range exponent
    /// silently became `0` via `.unwrap_or(0)` -- misrouting into the
    /// "eliminate exponent" fast path before the mantissa-preserving logic
    /// above ever ran. Widening that parse to `i64` (`parse_literal_exponent`)
    /// fixes it for any realistic exponent.
    #[test]
    fn test_format_number_jq_compat_underflow_beyond_i32_exponent_range_1099() {
        // One exponent digit past i32::MIN (-2147483648) -- used to
        // silently dispatch through `exp == 0` and print bare `0`, losing
        // sign, mantissa, and exponent entirely.
        assert_eq!(format_number_jq_compat(b"1e-2147483649"), "1E-2147483649");
        assert_eq!(format_number_jq_compat(b"-1e-2147483649"), "-1E-2147483649");
        // Right at the (now-irrelevant) old i32 boundary -- must stay
        // correct too, not just the one-past case.
        assert_eq!(format_number_jq_compat(b"1e-2147483648"), "1E-2147483648");
        // Same bug, zero-mantissa side: a genuinely-zero mantissa at an
        // out-of-i32-range exponent also mis-dispatched through `exp == 0`,
        // wrongly eliminating (rather than preserving) the huge exponent.
        assert_eq!(format_number_jq_compat(b"0e-2147483649"), "0E-2147483649");
    }

    /// `parse_literal_exponent`'s own saturating fallback: an exponent
    /// digit string that overflows even `i64` (not just `i32`) saturates to
    /// `i64::MIN`/`MAX` by sign rather than erroring. The overflow-ceiling
    /// test above already exercises the positive (`i64::MAX`) side via
    /// `1e99999999999999999999`; this covers the negative (`i64::MIN`)
    /// side, which only underflow's unlimited exponent range can reach (the
    /// overflow path's own ceiling check fires at a much smaller magnitude,
    /// long before an exponent could overflow `i64`).
    #[test]
    fn test_format_number_jq_compat_underflow_exponent_beyond_i64_range_1099() {
        assert_eq!(
            format_number_jq_compat(b"1e-99999999999999999999"),
            "1E-9223372036854775808"
        );
    }

    /// #1177: a literal that parses to a nonzero but *subnormal* `f64`
    /// (below `f64::MIN_POSITIVE`, but still representable) used to render
    /// its mantissa as the literal text `"inf"` -- not valid JSON.
    /// `libm::pow(10.0, log10(abs_value).floor())` itself underflows to
    /// `0.0` at the extreme low end of the subnormal range, making
    /// `abs_value / 0.0 = +inf`. Verified live against jq 1.7.1.
    #[test]
    fn test_format_number_jq_compat_subnormal_preserves_mantissa_1177() {
        assert_eq!(format_number_jq_compat(b"5e-324"), "5E-324");
        assert_eq!(format_number_jq_compat(b"-5e-324"), "-5E-324");
        assert_eq!(format_number_jq_compat(b"1e-323"), "1E-323");
        assert_eq!(format_number_jq_compat(b"4.9e-324"), "4.9E-324");
        assert_eq!(format_number_jq_compat(b"9.9e-324"), "9.9E-324");
        assert_eq!(format_number_jq_compat(b"2.5e-320"), "2.5E-320");
        assert_eq!(format_number_jq_compat(b"1e-315"), "1E-315");
        assert_eq!(format_number_jq_compat(b"1e-310"), "1E-310");
    }

    /// #1177 review self-check: a normal (non-subnormal) tiny value right
    /// at and just above the `f64::MIN_POSITIVE` boundary must still take
    /// the finite `log10`/`pow` path unchanged -- the new subnormal check
    /// uses `<`, not `<=`, so it doesn't over-broaden into values the
    /// existing path already handles correctly.
    #[test]
    fn test_format_number_jq_compat_normal_boundary_near_min_positive_unaffected_1177() {
        assert_eq!(format_number_jq_compat(b"1.5e-308"), "1.5E-308");
        assert_eq!(format_number_jq_compat(b"3e-308"), "3E-308");
        assert_eq!(format_number_jq_compat(b"1e-300"), "1E-300");
    }

    /// #930 review: an overflowed literal's mantissa can be arbitrarily long
    /// (it's document-controlled text), but only a handful of its leading
    /// digits ever survive `dump_truncated`'s later preview truncation - so
    /// rendering the *whole* mantissa here, only to throw almost all of it
    /// away one call up, would be unbounded work for no visible benefit.
    /// Pins that the leading digits (and thus the exponent, which is
    /// unaffected either way) stay correct, and that the rendered text
    /// itself is bounded rather than millions of bytes long.
    #[test]
    fn test_format_number_jq_compat_overflow_huge_mantissa_is_bounded() {
        let mantissa = "9".repeat(2_000_000);
        let literal = format!("{mantissa}.5e400");
        let result = format_number_jq_compat(literal.as_bytes());
        let expected_prefix = format!("9.{}", "9".repeat(32));
        assert!(
            result.starts_with(&expected_prefix),
            "leading digits must still be correct: {result}"
        );
        // The exponent shift (mantissa.len() - 1) is unaffected by the
        // rendering cap - only how many digits after the leading one get
        // copied into the output text.
        assert!(
            result.ends_with("E+2000399"),
            "exponent must reflect the mantissa's true (uncapped) length: {result}"
        );
        assert!(
            result.len() < 100,
            "must not render anywhere near the full 2,000,000-digit mantissa: got {} bytes",
            result.len()
        );
    }

    /// `depth` levels of single-element array nesting: `[[[...[Null]...]]]`.
    fn linear_array_nest(depth: usize) -> OwnedValue {
        let mut v = OwnedValue::Null;
        for _ in 0..depth {
            v = OwnedValue::Array(vec![v]);
        }
        v
    }

    /// #1005: a value built at query-evaluation time (e.g. a `reduce`
    /// accumulator growing one array level per iteration) has no adversarial
    /// *document* behind it, so #998's input-side guards never see it -
    /// `to_json`/`to_json_for_reindex`/`==` must each independently refuse
    /// to recurse past the same limit rather than overflow the stack.
    #[test]
    fn to_json_panics_past_nesting_depth_limit_1005() {
        let under = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        // Under the limit: succeeds (doesn't panic).
        let _ = under.to_json();
        let _ = under.to_json_for_reindex::<JqSemantics>();

        let over = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| over.to_json()));
        assert!(
            result.is_err(),
            "to_json should panic at MAX_VALUE_TREE_DEPTH"
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            over.to_json_for_reindex::<JqSemantics>()
        }));
        assert!(
            result.is_err(),
            "to_json_for_reindex should panic at MAX_VALUE_TREE_DEPTH"
        );
    }

    /// #1005: `==` recurses through a private depth-tracked helper instead
    /// of delegating to `Vec`'s/`IndexMap`'s own `==` (which would silently
    /// reset the depth count every level) - confirm the guard actually
    /// fires through the `PartialEq` impl, not just when called directly.
    #[test]
    fn eq_panics_past_nesting_depth_limit_1005() {
        let under_a = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        let under_b = linear_array_nest(MAX_VALUE_TREE_DEPTH - 1);
        assert_eq!(under_a, under_b);

        let over_a = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let over_b = linear_array_nest(MAX_VALUE_TREE_DEPTH);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| over_a == over_b));
        assert!(result.is_err(), "== should panic at MAX_VALUE_TREE_DEPTH");
    }
}
