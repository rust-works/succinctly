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
use super::eval::{EvalSemantics, EvalTag};
#[cfg(test)]
use super::eval::{JqSemantics, YqSemantics};
use super::expr::Literal;

/// Recursion-depth ceiling for tree-walkers over an already-materialized
/// [`OwnedValue`] (#1005).
///
/// Guards [`OwnedValue::to_json`]/`to_json_for_reindex`/`==`/
/// `eval::compare_values`/`eval.rs`'s own `to_owned`/
/// `yq_runner::reconcile_presentation`/`output::format_json_impl`/
/// `jq_runner::print_json` (#1819 -- moved here from the stricter,
/// mismatched `eval_generic::MAX_NESTING_DEPTH`, see that function's own
/// doc comment).
///
/// Deliberately a *separate* constant from
/// [`eval_generic::MAX_NESTING_DEPTH`](super::eval_generic::MAX_NESTING_DEPTH)
/// (256), not a reuse of it: that ceiling guards cursor-to-`OwnedValue`
/// *conversion* functions (`to_owned`/`to_owned_cursor`/`cursor_to_owned`),
/// individually tuned against their own measured crash boundaries --
/// `to_owned`'s own sits between 1800-2000. (`print_json` used to share
/// that ceiling too; #1819 moved it to this one instead, since it was the
/// wrong pairing -- see this constant's "Guards" list above and
/// `print_json`'s own doc comment.) This constant's own guarded functions
/// are a different shape with different measured boundaries (debug build,
/// default 2MiB test-thread stack — the more fragile of debug/release,
/// matching how 256 was itself measured): `reconcile_presentation` crashes
/// between depth 580-600 (the tightest of this set), `format_json_impl`
/// between 650-700, `print_json` between 600-700, and `eval.rs`'s own
/// `to_owned` between 1800-2000. Reusing 256 here would be
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
    assert!(depth < max, "{}", nesting_depth_exceeded_message(max));
}

/// The message every depth-limit guard reports past `max` levels of nesting.
///
/// Shared by every guard -- panicking (`assert_depth`) or checked
/// ([`eval_generic::check_nesting_depth`](super::eval_generic::check_nesting_depth),
/// `jq_runner.rs`'s `print_json`) -- so a wording change can't drift
/// between the forms the way two independent copies of this exact string
/// already did once before being consolidated into `assert_depth` (#998).
pub fn nesting_depth_exceeded_message(max: usize) -> String {
    format!("nesting depth exceeds limit of {max}")
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

/// Canonicalize a non-exponent literal's insignificant leading zero/`+`
/// (#1224, mirroring #1180's identical fix for the exponent-notation
/// mantissa path -- `split_mantissa`/`normalize_extreme_literal_mantissa`
/// below), for `format_number_jq_compat`'s own two non-exponent branches --
/// one shared definition rather than two hand-copied ones, per the #106
/// "duplicated predicates diverge silently" lesson in `CLAUDE.md`.
///
/// A leading `-` is always kept, including when every remaining digit is
/// zero (`-000` -> `-0`), matching real jq's own negative-zero
/// preservation (oracle-verified: `echo -000 | jq .` -> `-0`); a leading
/// `+` is always dropped entirely, matching jq's own canonical output
/// never emitting one (oracle-verified: `echo +007 | jq .` -> `7`). Only
/// the *integer* part (before any `.`) ever loses a digit -- the
/// fractional part, trailing zeros included, is untouched, matching this
/// module's own trailing-zero-preservation rule. A leading-dot spelling
/// (`.5`) has an empty integer part to begin with, which strips to the
/// same canonical `"0"` a `007`-style one does (oracle-verified: `echo
/// 007.5 | jq .` -> `7.5`, `echo .5 | jq .` -> `0.5`) -- so this also
/// subsumes #1171's separate leading-dot-gets-a-`0`-prefix rule, rather
/// than needing its own case.
fn strip_insignificant_leading_zero_and_plus(s: &str) -> String {
    // `strip_leading_sign` (#1304 code review): this was a fourth copy of
    // the same "peel `-`, else peel `+`" shape `strip_leading_sign` was
    // introduced to consolidate, left behind in the very pass meant to
    // close that gap.
    let (negative, rest) = strip_leading_sign(s);
    let sign = if negative { "-" } else { "" };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let canonical_int = match int_part.trim_start_matches('0') {
        "" => "0",
        trimmed => trimmed,
    };
    match frac_part {
        Some(f) => format!("{sign}{canonical_int}.{f}"),
        None => format!("{sign}{canonical_int}"),
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
/// - Integers: output as-is, except an insignificant leading zero/`+` is
///   stripped (`007` -> `7`, `+7` -> `7`; #1224, only reachable via a
///   directly-constructed `NumberLiteral` -- see `is_valid_number`'s own
///   gate, which never lets this spelling through the document/query
///   input paths)
/// - Floats with trailing zeros: preserve them (`0.10` -> `0.10`); an
///   insignificant leading zero/`+` in the integer part is stripped the
///   same way integers are (`007.10` -> `7.10`)
/// - Scientific notation: normalize mantissa (`12e2` -> `1.2E+3`), uppercase E, explicit +
/// - `e0`/`e-0`: eliminate the exponent entirely (`5.5e0` -> `5.5`)
/// - Negative exponents >= -5: convert to decimal (`1e-3` -> `0.001`)
/// - Negative exponents < -5: keep scientific (`1e-10` -> `1E-10`)
/// - Negative zero: preserved as `-0` (`-000` also collapses to `-0`, not
///   `0` -- the sign survives stripping an all-zero digit string, matching
///   real jq's own `-0` preservation, oracle-verified)
///
/// Shared by the CLI's `--jq-compat` output formatter
/// (`src/bin/succinctly/jq_runner.rs`) and every library-level path that
/// renders a `NumberLiteral` (`to_json`, `tostring`, `@json`, string
/// interpolation, error-message previews) -- a single definition so the two
/// cannot drift, per the #106 lesson in `CLAUDE.md`.
///
/// # Design note (#1224)
///
/// This function is deliberately permissive, not precondition-gated:
/// every internal construction site for a `NumberLiteral`
/// (`from_number_bytes`, the jq-filter lexer, JSON's `number_literal()`,
/// YAML's `is_preservable_float_literal`) already only ever produces
/// RFC-8259-canonical number syntax or this crate's one deliberately
/// lenient exception, a leading-dot spelling (`.5`/`-.5`, #1171) -- but
/// `OwnedValue::NumberLiteral`/`Literal::NumberLiteral` are both public
/// enum variants with public fields, so nothing at the *type* level stops
/// an external caller from constructing one with arbitrary text and
/// reaching this function through 100% safe code. Rather than assert that
/// precondition (tried, and reverted: it fired on this module's own
/// existing tests for genuinely malformed input -- invalid UTF-8,
/// unparseable exponents, #1180's own leading-zero-mantissa repros --
/// which this function has always been expected to degrade gracefully on,
/// not reject), every code path here already falls back to a reasonable
/// best-effort result instead of panicking: invalid UTF-8 renders lossily,
/// an unparseable exponent echoes the raw text unchanged, and (since this
/// same fix) an insignificant leading zero/`+` on the plain-integer/
/// decimal paths canonicalizes rather than echoing verbatim. A future
/// caller passing genuinely unexpected text gets a defined, non-panicking
/// answer either way.
pub fn format_number_jq_compat(raw: &[u8]) -> String {
    let s = match core::str::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return String::from_utf8_lossy(raw).into_owned(),
    };

    // Check if it contains exponent notation
    let has_exp = s.contains('e') || s.contains('E');
    let has_dot = s.contains('.');

    if !has_exp && !has_dot {
        // Plain integer - canonicalize away an insignificant leading
        // zero/`+` (#1224, mirroring #1180's identical fix for the
        // exponent-notation mantissa path -- `split_mantissa`/
        // `normalize_extreme_literal_mantissa` below). Originally
        // reachable only via a directly-constructed `NumberLiteral`
        // bypassing `is_valid_number`'s gate (#1224's own note) -- #1149's
        // leading-zero materialization fix (`from_number_bytes`,
        // `DocumentValue::number_literal`) now makes this a real,
        // exercised path too: both store the *original* un-stripped bytes
        // as the literal spelling (matching the leading-dot case just
        // below) and rely entirely on this canonicalization to fix
        // display, rather than each independently stripping the zero
        // themselves.
        return strip_insignificant_leading_zero_and_plus(s);
    }

    if !has_exp {
        // Plain decimal without exponent - preserve trailing zeros
        // (`0.10` stays `0.10`), but canonicalize an insignificant leading
        // zero/`+` in the integer part the same way (#1224). This also
        // folds in real jq's own reader adding a leading `0` to a
        // leading-dot spelling even under plain identity -- confirmed
        // live: `.500 | .` -> `0.500` (#1171) -- for free: an *absent*
        // integer part strips to the same canonical `0` a `007`-style one
        // does. Also now the real display fix for #1149's leading-zero
        // leniency (`007.500` -> `7.500`), for the same reason as the
        // plain-integer branch above.
        return strip_insignificant_leading_zero_and_plus(s);
    }

    // Has exponent - need to reformat according to jq rules
    // Parse the full number to get the actual value
    let value: f64 = match s.parse() {
        Ok(v) => v,
        Err(_) => return s.to_string(),
    };

    // `has_exp` above guarantees `e`/`E` is present, so the position is
    // always found. The exponent digit string itself is parsed lazily, on
    // demand, by whichever of the branches below actually needs it
    // (`parse_literal_exponent`/`normalize_extreme_literal_mantissa`) --
    // #1264 removed the one call site that used to parse it eagerly here
    // just to fast-path an `exp == 0` literal, since that fast path had its
    // own precision-loss bug and turned out to be exactly equivalent to
    // (not a genuine shortcut ahead of) the shared string-based logic below.
    let exp_pos = s.find(['e', 'E']).expect("has_exp guarantees e/E present");

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
    if !value.is_finite() {
        return format_overflow_literal_mantissa(s, exp_pos, value.is_sign_negative());
    }

    // A zero-valued literal (`0.0e-400` or a genuinely-underflowed nonzero
    // mantissa like `1e-400`) picks its notation on a *different* threshold
    // than every other case below -- both `exp == 0` and the `-5..0` window
    // just below use the literal's own *written* exponent, which #1207
    // found is simply the wrong axis once a written exponent and a fraction
    // are both present (`0.0e-400`'s written exponent is `-400`, but its
    // *shifted* exponent -- what actually determines notation, matching
    // #1178's own shift math -- is `-401`). Intercepting here, before either
    // raw-exponent check, keeps this one shifted-exponent rule as the only
    // path a zero-valued literal can take (`value == 0.0` is true for
    // `-0.0` too, so this also subsumes the old `exp == 0` branch's
    // negative-zero special case below -- that branch's own sign-handling
    // stays necessary for a *nonzero* integer-valued literal like `-5e0`,
    // just no longer for `-0e0` specifically).
    if value == 0.0 {
        return format_near_zero_literal(s, exp_pos, value.is_sign_negative());
    }

    // #1264: `exp == 0` (an explicit `e0`/`e-0`/`E0` spelling) used to have
    // its own fast path here -- an exact `value as i64` cast for a small
    // integer-valued literal, else `format!("{value}")` (plain `f64`
    // `Display`) for everything else. That `Display` fallback silently lost
    // precision beyond `f64`'s own round-trip guarantee (~17 significant
    // digits) for a fractional or `>= 1e15` literal instead of preserving
    // the literal's exact source digits the way every other case in this
    // function does (`-824118596092576.85097746e0` -> `-...576.9`,
    // `99999999999999999e0` -> `100000000000000000`, both wrong -- oracle-
    // confirmed against jq 1.7.1). No special-casing needed at all: with
    // `parsed_exp == 0` folded into `normalize_extreme_literal_mantissa`'s
    // shift math below, `shifted_exp` reduces to exactly `shift` -- *not*
    // always `0` (a multi-digit integer mantissa like `500e0` shifts to a
    // positive `shifted_exp`, correctly routing through the
    // `format_positive_shifted_plain`/#1244 plain-decimal path below rather
    // than `format_shifted_mantissa`'s own `shifted_exp == 0` arm; a
    // magnitude-<1 mantissa past `-6` shifts to scientific instead, e.g.
    // `0.0000005e0` -> `5E-7`, matching real jq -- code review, #1264).
    // Falling straight through to that general logic instead of keeping a
    // dedicated fast path here gives the identical, already-verified-
    // correct answer for every case that used to take the `value as i64`
    // fast path too -- confirmed via a broad randomized differential sweep
    // against jq 1.7.1, not just the cases this comment names.

    // #1226: real jq's bare-integer/decimal-window choice for the *rest* of
    // this function (every nonzero, non-subnormal mantissa the `exp == 0`
    // branch above didn't already claim) is also decided by the mantissa's
    // *shifted* exponent, not its raw written one -- the exact same axis
    // #1207 established for the zero-mantissa case, extended here to
    // nonzero mantissas. `5e-6`'s raw exponent (`-6`) falls outside the old
    // `-5..0` window and used to fall through to scientific notation, but
    // its shift is `0` (a single-digit mantissa), so its *shifted*
    // exponent is also `-6` -- inside real jq's actual `-6..=-1` decimal
    // window (oracle-verified, matching #1207's own zero-mantissa
    // boundary). `50e-1` is the inverse mismatch: raw exponent `-1` (which
    // *was* inside the old window, so it never reached scientific
    // notation) but shift `1` (mantissa `"50"` is two digits), giving
    // shifted exponent `0` -- real jq renders this as `5.0`, keeping the
    // mantissa's own trailing zero, not the bare `5` a *value*-based
    // renderer would produce (`50e-1` and `5e0` parse to the identical
    // `f64`, so only the literal's own text can tell them apart) -- this
    // is why `format_shifted_mantissa` below is string-based, not the
    // value-based rounding-and-trimming the old `(-5..0)` window's
    // `format_decimal_jq` used (removed -- this replaced its one call
    // site).
    //
    // A nonzero `value` (checked above) guarantees at least one
    // significant mantissa digit, so this always succeeds -- the same
    // invariant `format_overflow_literal_mantissa` relies on for its own
    // `unreachable!()`.
    //
    // `new_exp`'s saturation (ignored via `.value()`): `s` already parsed
    // successfully to a finite, nonzero `f64` to reach this path at all
    // (subnormal included, since #1206 folded that case in here too), so
    // its written exponent digit string was in `i128`'s range long before
    // it was in `f64`'s -- unlike `format_near_zero_literal` (#1273), this
    // path can't actually observe a saturated exponent in practice.
    let Ok(NormalizedMantissa {
        mantissa_str,
        new_exp,
        digit_count,
    }) = normalize_extreme_literal_mantissa(s, exp_pos, Some(MAX_RENDERED_MANTISSA_DIGITS))
    else {
        unreachable!("nonzero value implies normalize_extreme_literal_mantissa succeeds")
    };
    let shifted_exp = new_exp.value();
    let sign = if value.is_sign_negative() { "-" } else { "" };
    if shifted_exp == 0 || (-6..=-1).contains(&shifted_exp) {
        // #1274: this window is *unconditionally* a decimal render --
        // `format_shifted_mantissa` never falls back to scientific notation
        // within it -- so it always needs every given digit, not just the
        // capped `mantissa_str` from above (see `full_mantissa_if_capped`'s
        // doc comment for why trying the capped one is unsafe, not just
        // wasteful).
        let full_mantissa_str = full_mantissa_if_capped(s, exp_pos, &mantissa_str, digit_count);
        return format_shifted_mantissa(sign, &full_mantissa_str, shifted_exp);
    }

    // #1244: a *positive* shifted exponent doesn't follow a comparably
    // simple window to the negative side above -- real jq keeps plain
    // decimal notation whenever the literal's own given significant digits
    // are enough to cover the value's whole integer part without implying
    // extra unstated trailing zeros (`500000e-1`, shifted exponent 4 but 6
    // given digits, stays plain `50000.0`), and only switches to scientific
    // once the shift would need to fabricate digits past what was given
    // (`99999999999999e1`, shifted exponent 14 but only 14 given digits --
    // one short -- goes scientific `9.9999999999999E+14`). `shifted_exp <
    // digit_count` is exactly that condition (oracle-verified against jq
    // 1.7.1 across a broad randomized/boundary sweep, both signs).
    //
    // #1274: `digit_count` -- not `shifted_exp` -- is what can exceed
    // `MAX_RENDERED_MANTISSA_DIGITS` here, and it can do so even for a
    // small, ordinary shift: a modest-magnitude literal with an enormous
    // fractional part (`"1".repeat(50) + "." + "9".repeat(150_000) + "e0"`,
    // parses to a perfectly ordinary finite `f64` around `1e49`, well under
    // any overflow concern) still gives real jq's own display the full
    // 150,050 given digits. Reusing the capped `mantissa_str` from above
    // for a plain render this large would silently drop trailing digits
    // (see `format_positive_shifted_plain`'s doc comment on why that
    // failure mode doesn't even return `None`) -- `try_positive_shifted_plain`
    // (shared with `format_overflow_literal_mantissa`, #1274) re-derives
    // uncapped only once eligibility is confirmed cap-independently. The
    // `shifted_exp > 0` guard stays here rather than folding into the
    // shared helper: unlike the overflow call site, this one can also see
    // small/negative shifted exponents (already routed to
    // `format_shifted_mantissa` above), so it's the one caller that
    // actually needs it.
    if shifted_exp > 0 {
        if let Some(plain) =
            try_positive_shifted_plain(sign, s, exp_pos, shifted_exp, digit_count, &mantissa_str)
        {
            return plain;
        }
    }

    // For every other case (a shifted exponent outside `-6..=0` that isn't
    // plain-eligible above), use
    // normalized scientific notation -- jq normalizes the mantissa to have
    // exactly one digit before the decimal point, which `mantissa_str`
    // computed above already is.
    //
    // #1206: reuses that same string-based `mantissa_str`/`shifted_exp`
    // rather than re-deriving a mantissa from `value` via
    // `libm::log10`/`libm::pow` arithmetic (removed here, along with its
    // sole caller `format_mantissa_jq`) -- the same string-based source-
    // digit derivation `format_overflow_literal_mantissa` and
    // `format_near_zero_literal` already use for their own scientific
    // output. That f64-based recomputation had two independent,
    // oracle-confirmed bugs across the normal-magnitude domain it covered:
    // it silently trimmed trailing zeros the mantissa's own source
    // spelling had (`1.50e10` -> `1.5E+10` instead of real jq's `1.50E+10`),
    // and its "snap to nearest integer if very close" heuristic could round
    // a mantissa up to exactly `10`, violating the `[1, 10)` single-
    // leading-digit invariant scientific notation requires
    // (`9.9999999999999e-64` -> `10E-64` instead of real jq's
    // `9.9999999999999E-64`) -- both confirmed live against jq 1.7.1.
    //
    // This also makes the standalone `!value.is_normal()` (#1177, subnormal)
    // check this function used to have here dead weight, since removed:
    // `format_near_zero_literal`'s own `Ok` arm (the only arm a nonzero
    // mantissa -- guaranteed by the `value == 0.0` check above -- ever
    // takes) was already exactly `assemble_scientific(sign, mantissa_str,
    // new_exp)` from a *second* `normalize_extreme_literal_mantissa(s,
    // exp_pos)` call on the same `s`/`exp_pos` this function normalizes
    // once, above. That redundancy only existed because the code being
    // replaced here was f64-arithmetic-based and genuinely unsafe on a
    // subnormal `value` (imprecise renormalization, or division by a
    // pow()-underflowed zero -- see git history); now that this path is
    // string-based too, subnormal and normal magnitudes take the identical
    // call with no separate handling needed (code review, #1206).
    assemble_scientific(sign, &mantissa_str, shifted_exp)
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

/// Join a sign, an already-normalized mantissa, a negative-exponent flag,
/// and the exponent's own unsigned magnitude text into jq's
/// scientific-notation text (`{sign}{mantissa}E{+/-}{magnitude}`) -- the
/// shared final step [`assemble_scientific`] and
/// [`assemble_scientific_from_raw_exponent`] both build on, so the two
/// can't independently drift on the `E+`/`E-` convention itself (#1304
/// code review) even though they derive `negative`/`exp_magnitude` from
/// different representations (a parsed `i128` vs. raw digit text).
/// `exp_magnitude: impl Display` rather than a `&str` lets the numeric
/// caller pass its `u128` magnitude directly with no extra allocation.
fn assemble_scientific_with_sign(
    sign: &str,
    mantissa_str: &str,
    negative_exp: bool,
    exp_magnitude: impl core::fmt::Display,
) -> String {
    let exp_sign = if negative_exp { "-" } else { "+" };
    format!("{sign}{mantissa_str}E{exp_sign}{exp_magnitude}")
}

/// Join a sign, an already-normalized mantissa, and an exponent into jq's
/// scientific-notation text (`{sign}{mantissa}E{+/-}{exp}`) -- the shared
/// final step of both the finite path above and the overflow path below.
/// `exp.unsigned_abs()`, not `exp` directly via `i128::Display`'s own
/// embedded `-` (#1304 code review): explicitly managing the sign the same
/// way [`assemble_scientific_from_raw_exponent`] does lets both share
/// [`assemble_scientific_with_sign`]'s one `E+`/`E-` decision instead of
/// each reimplementing it -- and `unsigned_abs()`, not `.abs()`, is what
/// stays correct at `exp == i128::MIN` (the saturation sentinel this
/// module's own `checked_add`/`checked_sub` overflow fallbacks can
/// produce), where `.abs()` would itself overflow `i128` but `u128` still
/// holds the magnitude exactly.
fn assemble_scientific(sign: &str, mantissa_str: &str, exp: i128) -> String {
    assemble_scientific_with_sign(sign, mantissa_str, exp < 0, exp.unsigned_abs())
}

/// Sibling of [`assemble_scientific`] for a saturated exponent (#1273): once
/// the source digit string didn't fit in `i128`, [`parse_literal_exponent`]'s
/// numeric return is a fixed, input-independent sentinel, not the literal's
/// real exponent -- displaying it (as [`assemble_scientific`] would) prints
/// the same wrong number for every overlong exponent regardless of what the
/// document actually wrote. This instead echoes the raw exponent digit text
/// itself, which is correct for any digit-string length and, unlike the
/// sentinel, is at least *derived from* the real input.
///
/// Not shift-adjusted: folding [`normalize_extreme_literal_mantissa`]'s
/// `shift` into an arbitrary-precision raw exponent string would need real
/// big-integer arithmetic to stay exact -- out of scope for what this issue
/// asks for (see its own "acceptance criterion is a new caller can't
/// silently inherit a fake exponent, not output X changes to Y"). `shift`
/// itself is *not* bounded in practice (#1273 review): the mantissa's
/// leading-zero-fraction-digit count it can derive from is never capped by
/// `MAX_RENDERED_MANTISSA_DIGITS` (that only bounds what gets copied into
/// the rendered mantissa text, not the position used to compute `shift`) --
/// which is exactly why `normalize_extreme_literal_mantissa` now detects
/// saturation from folding `shift` in, not just from the raw parse, and
/// routes here on either. A perfectly shift-adjusted answer isn't reachable
/// here without big-integer machinery, but an honestly-unshifted one beats
/// a fabricated one.
///
/// #1304: unlike [`normalize_extreme_literal_mantissa`]'s
/// `MAX_RENDERED_MANTISSA_DIGITS`-capped mantissa, `digits` here is echoed
/// in full, uncapped -- a deliberate decision, not an oversight. Two things
/// distinguish this from the mantissa case rather than calling for the
/// identical treatment:
/// - **Already gated behind an extreme condition.** This function only
///   ever runs once the exponent has already saturated `i128`, i.e. the
///   digit string is already at least ~39 characters long by construction
///   -- unlike the mantissa, which is unbounded starting from the very
///   *first* digit of any ordinary literal, so capping it protects the
///   overwhelmingly common case. There is no comparably common case here
///   to protect.
/// - **Measured, not assumed, to cost proportionally, not
///   super-linearly.** [`dump_truncated`](crate::jq::error) is the one
///   caller (via [`format_near_zero_literal`] -> `describe`/`error(v)`
///   messages) whose own contract wants preview cost independent of input
///   size -- live-timed a 5,000,000-digit saturated exponent through it
///   (`"0.005e-" + "9".repeat(5_000_000)` piped through `.[]` to trigger a
///   `describe`d "cannot iterate" error): 0.17s, linear in the input's own
///   size, not amplified -- the same "the input already had to contain
///   that many bytes to trigger it, not an amplification" reasoning
///   `MAX_RENDERED_MANTISSA_DIGITS`'s own doc comment already applies to
///   its `None` (uncapped) case for callers that need every given digit.
///   A cap here would additionally cost real correctness for the
///   *ordinary* (non-preview) render path, which -- matching this crate's
///   stated preserve-every-given-digit philosophy -- has no comparable
///   reason to truncate an exponent it can otherwise echo exactly.
fn assemble_scientific_from_raw_exponent(
    sign: &str,
    mantissa_str: &str,
    raw_exp_text: &str,
) -> String {
    let (negative, digits) = strip_leading_sign(raw_exp_text);
    // Canonicalize like every other exponent-rendering path in this module
    // (#1273 review): `assemble_scientific` never emits a leading zero, so
    // a raw echo that skips this strip would be the one place this
    // formatter's output isn't leading-zero-free -- e.g. `1e-007...`
    // echoing as `E-007...` instead of `E-7...`. `digits.is_empty()` is
    // unreachable in practice (this function only runs on a genuinely
    // saturated exponent, and an all-zero digit string always parses to
    // exactly `0`, never saturating), but the fallback keeps this total
    // rather than relying on that invariant.
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    assemble_scientific_with_sign(sign, mantissa_str, negative, digits)
}

/// Parse a literal's exponent digit string at wide (`i128`) precision,
/// saturating to `i128::MIN`/`MAX` by sign on overflow rather than erroring
/// -- the exponent digit string can itself be longer than even `i128` can
/// hold (e.g. `1e999999999999999999999999999999999999999`), and any such
/// value is already certain to be past whichever ceiling (or lack thereof)
/// the caller applies. `i128`, not `i64` (#1270): the near-zero/underflow
/// path (`format_near_zero_literal`) has no deliberate ceiling of its own
/// (see that function's doc comment), so an `i64`-width saturation point
/// was reachable by an ordinary (if pathological) document exponent --
/// `1e-999999999999999999999999` (24 nines) saturated to exactly
/// `i64::MIN`. `i128`'s far larger range (~38 digits) pushes that same
/// saturation point out by roughly 19 more orders of magnitude, past what
/// any realistic or plausibly-fuzzed literal would reach, without the
/// complexity of true arbitrary precision -- but a saturation point,
/// wherever it sits, is still reachable by a long enough digit string
/// (#1273), which is why the second element of the returned tuple exists:
/// callers with their own ceiling far below `i128`'s range (`format_overflow_literal_mantissa`'s
/// `>= 1_000_000_000` check, or the ordinary path's implicit bound via
/// `f64::parse` already having succeeded) can ignore it and use the
/// sentinel as "definitely past the ceiling"; [`format_near_zero_literal`],
/// which has no such ceiling to fall back on, uses it to avoid displaying
/// the sentinel as if it were the literal's real exponent (#1273 -- see
/// that function's own doc comment).
/// Shared so `format_number_jq_compat`'s own exponent dispatch and
/// [`normalize_extreme_literal_mantissa`]'s shift math can't independently
/// drift on how an out-of-range exponent is handled (#1099 code review:
/// an earlier `i32`-width, `.unwrap_or(0)` version of this parse at the
/// `format_number_jq_compat` call site silently treated an out-of-range
/// exponent as exactly `0`, misrouting extreme-underflow literals into the
/// "eliminate exponent" fast path before ever reaching this module).
fn parse_literal_exponent(exp_text: &str) -> ExpParse {
    match exp_text.parse() {
        Ok(v) => ExpParse::Exact(v),
        Err(_) => {
            let (negative, _) = strip_leading_sign(exp_text);
            let sentinel = if negative { i128::MIN } else { i128::MAX };
            ExpParse::Saturated(sentinel)
        }
    }
}

/// Peel a single leading `+`/`-` off `s`, returning whether it was
/// negative and the sign-stripped remainder -- the lexer guarantees at
/// most one leading sign character on any digit text this module works
/// with, so a leading `+`/`-` are mutually exclusive and this only ever
/// checks the first character once.
///
/// Shared so "peel a leading sign off digit text" has exactly one
/// implementation instead of four independently reimplementing it
/// slightly differently (#106, #1304 code review):
/// [`strip_insignificant_leading_zero_and_plus`]'s old
/// `strip_prefix('-')`/`strip_prefix('+')` match, [`split_mantissa`]'s old
/// `strip_prefix(['-', '+'])`, [`parse_literal_exponent`]'s old
/// `trim_start_matches('+')` + `starts_with('-')`, and
/// [`assemble_scientific_from_raw_exponent`]'s old `strip_prefix('+')`
/// then `strip_prefix('-')` were each correct today only because the
/// lexer's own invariant happens to make all four equivalent -- nothing
/// enforced that they'd stay in agreement if it ever didn't. (An initial
/// pass at this consolidation missed the first of these four -- code
/// review caught that the "three" this comment originally claimed left
/// one copy standing in the very file being cleaned up.)
fn strip_leading_sign(s: &str) -> (bool, &str) {
    match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    }
}

/// An `i128` exponent value together with whether it's the digit string's
/// *exact* parse or a fixed sentinel standing in for a digit string too
/// long for `i128` to hold at all (#1273) -- every caller of
/// [`parse_literal_exponent`] and every field of
/// [`NormalizedMantissa`] that carries an exponent uses this instead of a
/// bare `i128` alongside a separately-tracked `bool`.
///
/// #1304 code review: the two were previously a `(i128, bool)` tuple, and
/// the one call site that actually needs to tell them apart
/// (`format_near_zero_literal`, the sole caller with no ceiling of its own
/// to fall back on) matched the trailing `bool` positionally --
/// `Ok((mantissa_str, _, _, true))` vs `Ok((.., false))` -- so a future
/// edit that swapped which arm got `true`/`false` would compile cleanly
/// and silently invert exact/saturated handling. Matching
/// `ExpParse::Saturated(_)`/`ExpParse::Exact(new_exp)` by name instead
/// makes that swap a compile error (unknown/mismatched variant) rather
/// than a silent behavior inversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExpParse {
    Exact(i128),
    Saturated(i128),
}

impl ExpParse {
    /// The numeric payload either way -- for a call site (`checked_add`/
    /// `checked_sub` against a further shift, or a ceiling comparison like
    /// `format_overflow_literal_mantissa`'s) that treats an exact and a
    /// saturated value identically, since either way the value is already
    /// past whatever ceiling the caller cares about.
    fn value(self) -> i128 {
        match self {
            Self::Exact(v) | Self::Saturated(v) => v,
        }
    }

    fn is_saturated(self) -> bool {
        matches!(self, Self::Saturated(_))
    }
}

/// Split a literal's mantissa text (`s[..exp_pos]`, sign stripped) into its
/// integer and fractional parts -- the sole implementation
/// [`normalize_extreme_literal_mantissa`] builds on, so "how do we split the
/// mantissa out of `s`" has exactly one implementation rather than one
/// hand-copied at each of its own call sites (#106).
fn split_mantissa(s: &str, exp_pos: usize) -> (&str, &str) {
    let raw = &s[..exp_pos];
    // Strip either sign via `strip_leading_sign` (#1304): `+` is as
    // insignificant to the mantissa's own magnitude as `-` already was
    // here, and (unlike `-`) wasn't stripped at all before #1180 -- a
    // leading `+` in `int_part` was misread as its significant leading
    // digit.
    let (_, mantissa) = strip_leading_sign(raw);
    mantissa.split_once('.').unwrap_or((mantissa, ""))
}

/// `dump_truncated`'s whole design keeps preview cost independent of the
/// value's own size (see its doc comment) - a document-controlled mantissa
/// of unbounded length must not turn into unbounded work in
/// [`normalize_extreme_literal_mantissa`] when a caller doesn't need every
/// given digit rendered anyway (scientific notation only ever shows a
/// bounded prefix). Bounding the *copy* is enough: `shift` only ever needs
/// `int_part.len()` (an O(1) property read once leading zeros are trimmed
/// -- the `trim_start_matches('0')` itself is an O(k) scan over any
/// leading-zero run, #1180), so truncating what gets copied into `rest`
/// doesn't touch the exponent math's correctness, only how many digits past
/// the first one actually get rendered. The trim doesn't change the
/// function's overall cost class either way: `format_number_jq_compat`'s
/// own `s.parse::<f64>()` ahead of every call into this function already
/// scans every byte of `s` once.
///
/// Real jq preserves a literal's full given precision unconditionally --
/// oracle-verified exact round-trips up to exactly this many significant
/// digits (#1253; raised from an earlier `32`, which was chosen for the
/// overflow/near-zero paths specifically, then silently inherited by every
/// other caller once #1206 routed the ordinary scientific-notation case
/// through this same function). This *is* still a cap, not a proven
/// ceiling: a literal with more true significant digits than this still
/// renders truncated rather than erroring when a caller opts into it (`Some`
/// below) -- code review on #1253 confirmed the truncation is silent, not
/// that raising the number removes it. Set to what this session actually
/// verified against pinned jq 1.7.1, not a round number chosen past it, so
/// the constant and the oracle claim it rests on can't drift apart again the
/// way `32` (tuned for a different use case entirely) drifted from the
/// fidelity the ordinary case actually needs.
///
/// A caller whose own notation choice needs every given digit regardless of
/// this cap -- [`try_positive_shifted_plain`]'s and
/// [`format_shifted_mantissa`]'s callers, #1274 -- passes `None` to
/// [`normalize_extreme_literal_mantissa`] instead of this constant, since
/// plain notation's whole point is rendering every given digit; there is no
/// bounded prefix to fall back on the way scientific notation has. This
/// deliberately reintroduces document-length-proportional cost for that
/// specific case (a many-hundred-thousand-digit literal that turns out to
/// be plain-eligible pays for rendering every one of those digits) rather
/// than adding a second, independent cap -- because real jq itself has no
/// such cap either (oracle-verified past 500,000 digits, no ceiling found),
/// matching it exactly means matching its cost profile too, and the input
/// already had to contain that many bytes to trigger it in the first place
/// (linear cost in the attacker's own paid-for input, not an amplification).
const MAX_RENDERED_MANTISSA_DIGITS: usize = 100_000;

/// Shift a literal's own mantissa digits (the text before `exp_pos`, i.e.
/// before `e`/`E`) into jq's one-digit-before-the-point normalized form,
/// and fold that shift into the literal's own written exponent -- returns
/// `Ok` with a [`NormalizedMantissa`], or `Err(frac_len)` if the mantissa is
/// genuinely all-zero (`0.0e400`, `0.00e-400`), which has no magnitude to
/// normalize against (`frac_len` is the fractional-digit count the `Err`
/// caller needs for its own shift math, #1207 -- surfaced here rather than
/// re-derived by every `Err` caller via a second `split_mantissa` call).
/// `new_exp`'s [`ExpParse::Saturated`] case is [`parse_literal_exponent`]'s
/// own saturation folded through unconditionally (#1273) -- only
/// [`format_near_zero_literal`] actually inspects it (its other two callers
/// both have a ceiling `new_exp`'s value clears regardless of whether it's
/// exact or a saturation sentinel, via [`ExpParse::value`]).
/// Entirely via string manipulation on `s` rather than `log10`/`pow` on the
/// parsed value, since the caller only reaches here when the parsed value
/// has already lost the precision this exists to recover (`+/-infinity` on
/// overflow, exactly `0.0` on underflow/genuine zero).
///
/// Shared by [`format_overflow_literal_mantissa`] and
/// [`format_near_zero_literal`] (#1099) -- the shift math is identical
/// whether the literal's written exponent is enormous-positive (overflow)
/// or enormous-negative (underflow/zero); only what each caller does with
/// `new_exp` afterward differs (overflow caps to infinity text past a
/// ceiling; the near-zero case has no such ceiling, and additionally uses
/// `Err`'s `frac_len` for its own small-magnitude notation choice, #1207).
/// The all-zero-mantissa detection is centralized here rather than
/// pre-checked separately by each caller, so "is this mantissa all-zero"
/// has exactly one implementation instead of two that must be kept in sync
/// by convention (#106).
///
/// `mantissa_digit_cap` bounds how many digits of `rest` actually get
/// copied into the returned `mantissa_str` -- see
/// [`MAX_RENDERED_MANTISSA_DIGITS`] for why a cap exists at all and when a
/// caller passes `None` instead. It never affects `new_exp`/`digit_count`,
/// both derived from `shift`/`int_part.len()`/`frac_part.len()` directly,
/// not from what got copied.
fn normalize_extreme_literal_mantissa(
    s: &str,
    exp_pos: usize,
    mantissa_digit_cap: Option<usize>,
) -> Result<NormalizedMantissa, i128> {
    let (int_part, frac_part) = split_mantissa(s, exp_pos);

    // Insignificant leading zeros (`007`) don't change which digit is the
    // mantissa's significant leading one, or the magnitude class it falls
    // into (#1180) -- strip them before classifying `int_part` at all. A
    // leading-dot literal (`.5e400`) already has an empty `int_part`, and an
    // int_part of nothing-but-zeros (`0`, `000`) trims to empty the same
    // way -- both fall into the magnitude-< 1 branch below.
    let int_part = int_part.trim_start_matches('0');

    let (shift, leading, rest, digit_count): (i128, &str, String, i128) = if int_part.is_empty() {
        // Shift right to the first nonzero fractional digit. Genuinely
        // all-zero mantissa (`0.0e400`/`0.0e-400`) has no nonzero digit to
        // shift to -- return `Err(frac_len)` instead, `frac_part.len()` is
        // exactly what the `Err` caller's own shift math needs (#1207).
        let Some(k) = frac_part.find(|c: char| c != '0') else {
            // A bare `as` cast, not `try_from`/`unwrap_or` (#1270 review):
            // `frac_part.len()` is a `usize`, and no realistic (or
            // foreseeable) target has a `usize` wider than `i128` --
            // unlike the *signed*, genuinely document-controlled-and-
            // unbounded exponent digit string `parse_literal_exponent`
            // below saturates against, this widening can't lose precision,
            // so the extra `try_from`/`unwrap_or(i128::MAX)` fallback was
            // dead code carried over from this function's pre-#1270 `i64`
            // width (where a `usize` truly could exceed the target range).
            return Err(frac_part.len() as i128);
        };
        let after = &frac_part[k + 1..];
        let digit_count = (after.len() + 1) as i128;
        let rest = match mantissa_digit_cap {
            Some(cap) => after[..after.len().min(cap)].to_string(),
            None => after.to_string(),
        };
        (-(k as i128 + 1), &frac_part[k..=k], rest, digit_count)
    } else {
        // Mantissa >= 1: shift left past every extra significant
        // integer-part digit. `int_part[1..]` is a slice (no copy); only
        // what actually gets concatenated into `rest` is capped (when
        // `mantissa_digit_cap` is `Some` at all).
        let after_leading = &int_part[1..];
        let digit_count = (int_part.len() + frac_part.len()) as i128;
        let rest = match mantissa_digit_cap {
            Some(cap) if after_leading.len() >= cap => after_leading[..cap].to_string(),
            Some(cap) => {
                let budget = cap - after_leading.len();
                format!(
                    "{after_leading}{}",
                    &frac_part[..frac_part.len().min(budget)]
                )
            }
            None => format!("{after_leading}{frac_part}"),
        };
        (
            int_part.len() as i128 - 1,
            &int_part[..1],
            rest,
            digit_count,
        )
    };

    let parsed_exp = parse_literal_exponent(&s[exp_pos + 1..]);
    // `checked_add`, not `saturating_add` (#1273 review): a `parsed_exp`
    // that's exact on its own (in range, `ExpParse::Exact`) can still
    // overflow once `shift` is folded in, if it lands within `shift` of
    // `i128::MIN`/`MAX` -- `shift` is unbounded (the mantissa's own
    // leading-zero-fraction-digit count, `k` above, is never capped by
    // `MAX_RENDERED_MANTISSA_DIGITS`, which only bounds what gets copied
    // into `rest`), so this is reachable with an ordinary ~39-digit
    // exponent, not just an already-overlong one. Live-verified: before
    // this check, `0.005e-170141183460469231731687303715884105727`
    // (exponent = i128::MIN+1, a fully in-range parse; mantissa `0.005`
    // gives `shift = -2`) rendered `5E-170141183460469231731687303715884105728`
    // -- the same fabricated sentinel a bare `ExpParse::Saturated` check
    // alone was meant to catch, leaking through a second, uncaught
    // saturation point.
    let (new_exp_value, shift_saturated) = match parsed_exp.value().checked_add(shift) {
        Some(v) => (v, false),
        None => (if shift < 0 { i128::MIN } else { i128::MAX }, true),
    };
    let new_exp = if parsed_exp.is_saturated() || shift_saturated {
        ExpParse::Saturated(new_exp_value)
    } else {
        ExpParse::Exact(new_exp_value)
    };

    let mantissa_str = if rest.is_empty() {
        leading.to_string()
    } else {
        format!("{leading}.{rest}")
    };
    Ok(NormalizedMantissa {
        mantissa_str,
        new_exp,
        digit_count,
    })
}

/// [`normalize_extreme_literal_mantissa`]'s successful result: a literal's
/// mantissa renormalized to jq's one-digit-before-the-point form, together
/// with the exponent that shift folds into and the mantissa's own true
/// (never-truncated) significant-digit count.
///
/// A named struct rather than the `(String, i128, i128, bool)` positional
/// tuple this replaced (#1304 code review) -- see [`ExpParse`]'s own doc
/// comment for the specific risk this and `parse_literal_exponent`'s return
/// type both close off.
struct NormalizedMantissa {
    mantissa_str: String,
    new_exp: ExpParse,
    /// The mantissa's *true* (never truncated -- see
    /// `MAX_RENDERED_MANTISSA_DIGITS`) significant-digit count, needed by
    /// `format_number_jq_compat`'s own plain-vs-scientific notation choice
    /// for a positive shifted exponent (#1244); it costs nothing extra to
    /// compute (`str::len()` is O(1)), so it's returned unconditionally
    /// rather than only on request.
    digit_count: i128,
}

/// Re-derive `mantissa_str` without [`normalize_extreme_literal_mantissa`]'s
/// `MAX_RENDERED_MANTISSA_DIGITS` cap, but only when the cap could actually
/// have truncated something -- when it's within the cap, `mantissa_str`
/// already holds every given digit, so this avoids a wasted second pass
/// over `s` in the overwhelmingly common case. Truncation of `rest` only
/// starts at `digit_count == cap + 2` (both branches of
/// `normalize_extreme_literal_mantissa` copy up to `cap` digits *after* the
/// mandatory leading one, so `digit_count == cap + 1` -- one leading digit
/// plus exactly `cap` more -- is already complete); the `+ 1` below matches
/// that exactly rather than the more conservative `digit_count > cap`,
/// which would trigger one boundary value early on an already-untruncated
/// mantissa (#1274 review, confirmed live: both give identical output at
/// that boundary, but only this bound avoids the redundant re-derivation).
///
/// Every caller that must echo every given digit verbatim --
/// [`format_shifted_mantissa`]'s unconditional decimal window,
/// [`format_positive_shifted_plain`]'s plain branch -- shares this rather
/// than each repeating the "is this actually capped" gate and the
/// `unreachable!()` (#1274: a caller that skipped the gate and always tried
/// the capped mantissa first could still get a *wrong* answer, not just an
/// `Option::None` -- see `format_positive_shifted_plain`'s own doc comment
/// -- so this helper exists specifically so no call site can shortcut past
/// it that way again).
fn full_mantissa_if_capped<'a>(
    s: &str,
    exp_pos: usize,
    mantissa_str: &'a str,
    digit_count: i128,
) -> Cow<'a, str> {
    if digit_count > MAX_RENDERED_MANTISSA_DIGITS as i128 + 1 {
        let Ok(NormalizedMantissa {
            mantissa_str: full, ..
        }) = normalize_extreme_literal_mantissa(s, exp_pos, None)
        else {
            unreachable!("digit_count > 0 implies a nonzero mantissa was already found")
        };
        Cow::Owned(full)
    } else {
        Cow::Borrowed(mantissa_str)
    }
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
    //
    // `new_exp`'s saturation (ignored via `.value()`): this function's own
    // ceiling check just below fires regardless of whether `new_exp` is
    // exact or a saturation sentinel -- either way it's past
    // `1_000_000_000` -- so unlike `format_near_zero_literal` (#1273),
    // there is no case here where the distinction changes what gets
    // rendered.
    let Ok(NormalizedMantissa {
        mantissa_str,
        new_exp,
        digit_count,
    }) = normalize_extreme_literal_mantissa(s, exp_pos, Some(MAX_RENDERED_MANTISSA_DIGITS))
    else {
        unreachable!("overflow implies a nonzero mantissa")
    };
    let new_exp = new_exp.value();

    // jq's own literal-preserving text (as opposed to a computed value's
    // DBL_MAX text) only goes up to this exponent magnitude (decNumber's
    // limit) -- oracle-verified: `1e999999999` keeps `1E+999999999`,
    // `1e1000000000` switches to DBL_MAX text instead.
    if new_exp.unsigned_abs() >= 1_000_000_000 {
        return infinite_float_preview_text(negative).to_string();
    }

    // #1244's plain-vs-scientific digit-count rule (see
    // `format_number_jq_compat`'s own use of it) applies here too, not just
    // to the non-overflow path -- an overflowed *value* (this function's
    // whole reason for existing) doesn't mean the *literal* lacks enough
    // given digits to stay plain: `"9".repeat(400) + "e0"` overflows `f64`
    // (400 nines vastly exceeds `f64::MAX`), yet real jq still renders it
    // as a plain 400-digit integer, not scientific notation, because all
    // 400 significant digits were given (`shifted_exp` 399 `<` `digit_count`
    // 400) -- oracle-verified, code review on #1253.
    //
    // #1274: `try_positive_shifted_plain` (shared with
    // `format_number_jq_compat`) decides eligibility first (cap-independent,
    // since `new_exp`/`digit_count` never depend on `mantissa_digit_cap`),
    // and only then fetches an uncapped mantissa via `full_mantissa_if_capped`
    // -- rather than just trying `mantissa_str` (capped, from above) and
    // falling back to `None` -- since a capped mantissa can silently
    // *succeed* in `format_positive_shifted_plain` too when the split point
    // falls within the capped prefix, not just fail outright (see that
    // function's own doc comment). No `new_exp > 0` guard needed here
    // (unlike `format_number_jq_compat`'s own call site, which also sees
    // small/negative shifted exponents): overflow requires `|value| >
    // f64::MAX` (~1.8e308), so `new_exp` is always well past `300` by the
    // time this function is ever reached -- `format_positive_shifted_plain`'s
    // own `debug_assert!` on a positive shift is what would catch a
    // violation of that invariant, not a redundant check here.
    if let Some(plain) =
        try_positive_shifted_plain(sign, s, exp_pos, new_exp, digit_count, &mantissa_str)
    {
        return plain;
    }

    assemble_scientific(sign, &mantissa_str, new_exp)
}

/// Format a literal whose parsed `f64` `value` has lost precision to (or
/// started at) zero -- either genuinely equal to `0.0` (`-0.0` included), or
/// nonzero but *subnormal* (#1099/#1177). `format_number_jq_compat` routes
/// both callers here: `value == 0.0` before any of its other notation
/// checks run (#1207 found this picks notation on a different axis than
/// every other literal, see that call site's own comment), and
/// `!value.is_normal()` afterward for the subnormal case (symmetric with
/// [`format_overflow_literal_mantissa`] on the overflow side).
///
/// [`normalize_extreme_literal_mantissa`] itself distinguishes the two ways
/// a literal reaches this function:
///
/// - A genuinely all-zero mantissa (`0.0e-400`, `Err(frac_len)` here) has no
///   nonzero digit to renormalize around at all -- real jq still shifts the
///   printed exponent by the fractional-zero-digit count the same way it
///   does for a nonzero mantissa (#1178: `0.000e-400` -> `0E-403`, not
///   `0E-400`; a no-fraction spelling like `0e-400` has shift `0` and needs
///   none). That *shifted* exponent, not the literal's raw written one, is
///   what actually determines notation (#1207, oracle-verified for shifted
///   exponents `-8..=2`): `0` picks a bare integer (`0e1` shifts to `0`,
///   prints `0`); `-6..=-1` picks an expanded decimal with `-shifted` zeros
///   after the point (`0e-1` -> `0.0`, ..., `0e-6` -> `0.000000`); anything
///   else keeps the scientific form this function used unconditionally
///   before #1207 (`0e-7` -> `0E-7`, `0e2` -> `0E+2`). Only reachable from
///   the `value == 0.0` caller -- a subnormal `value` can only come from a
///   nonzero mantissa in the first place, so the `!value.is_normal()`
///   caller always takes the `Ok` arm below instead.
/// - A nonzero mantissa that simply underflowed `f64` during parsing
///   (`1e-400`, far below `f64::MIN_POSITIVE`) can't be told apart from the
///   above by the parsed value alone (#1099) -- but real jq's small-window
///   decimal/bare notation never applies to it regardless of its shifted
///   exponent's own magnitude (a mantissa with real precision to preserve
///   always stays scientific); only `assemble_scientific` on the `Ok` arm's
///   own shift math is needed, unaffected by #1207. Oracle-verified against
///   real jq: `1e-400` -> `1E-400`, `12.34e-400` -> `1.234E-399`, `0.5e-400`
///   -> `5E-401`, `100.5e-400` -> `1.005E-398`. Unlike overflow, this side
///   has no *deliberate* ceiling -- `1e-1000000000` (exponent magnitude *at*
///   the overflow ceiling) still prints `1E-1000000000` unchanged, not a
///   fallback to plain `0`. Real jq itself, however, breaks down for
///   magnitudes beyond ~1,147,483,647 (`i32::MAX - 1e9`; an apparent
///   internal int32-overflow bug in its own decNumber), printing a fixed
///   sentinel (`1e-1200000000` -> real jq's `0E-1147483646`) rather than
///   continuing to preserve the mantissa. This function deliberately does
///   **not** replicate that breakdown -- it keeps preserving the mantissa
///   past jq's own failure point, on the basis that jq's behavior there is
///   itself a bug, not a rule worth matching.
///
/// `value == 0.0` is true for `-0.0` too (IEEE 754), so `negative` (from the
/// caller's `value.is_sign_negative()`) still needs applying in every arm --
/// real jq keeps the sign throughout, since `log10(0)` is undefined
/// (`-0e5` -> `-0E+5`, `-0e0` -> `-0`, `-0.0e-1` -> `-0.00`).
///
/// **#1274 review note:** every arm below feeds its mantissa to
/// `assemble_scientific`/`assemble_scientific_from_raw_exponent`, both
/// bounded-prefix consumers that are safe with a `Some(MAX_RENDERED_MANTISSA_DIGITS)`-capped
/// `mantissa_str` -- this function is safe *because* it has no plain-decimal
/// branch, not because anything here enforces that. If a future change ever
/// adds one (mirroring `format_shifted_mantissa`'s `-6..=-1` window or
/// `format_positive_shifted_plain`'s), it must fetch an uncapped mantissa
/// via `full_mantissa_if_capped` first, the same as every other caller that
/// needs to echo every given digit -- reusing `mantissa_str` directly would
/// reintroduce exactly the silent-truncation bug this issue fixed.
fn format_near_zero_literal(s: &str, exp_pos: usize, negative: bool) -> String {
    let sign = if negative { "-" } else { "" };
    match normalize_extreme_literal_mantissa(s, exp_pos, Some(MAX_RENDERED_MANTISSA_DIGITS)) {
        // #1273/#1304: this is the one caller with no ceiling of its own
        // (see the doc comment above), so it's the one place a saturated
        // exponent must not reach `assemble_scientific` -- that would
        // display `parse_literal_exponent`'s fixed sentinel as if it were
        // the literal's real exponent. Matched by variant name, not a
        // positional trailing `bool` (#1304 code review) -- see
        // `ExpParse`'s own doc comment for why.
        Ok(NormalizedMantissa {
            mantissa_str,
            new_exp: ExpParse::Saturated(_),
            ..
        }) => assemble_scientific_from_raw_exponent(sign, &mantissa_str, &s[exp_pos + 1..]),
        Ok(NormalizedMantissa {
            mantissa_str,
            new_exp: ExpParse::Exact(new_exp),
            ..
        }) => assemble_scientific(sign, &mantissa_str, new_exp),
        Err(frac_len) => {
            let parsed_exp = parse_literal_exponent(&s[exp_pos + 1..]);
            // `checked_sub`, not `saturating_sub` (#1273 review, same
            // reasoning as `normalize_extreme_literal_mantissa`'s own
            // `checked_add` above): `frac_len` is document-controlled and
            // unbounded (an all-zero mantissa's own fractional-digit
            // count), so a `parsed_exp` that parsed exactly can still
            // underflow once `frac_len` is subtracted. `frac_len` is always
            // `>= 0` (`str::len()`), so this can only underflow toward
            // `i128::MIN`, never overflow toward `MAX`.
            let (shifted_exp_value, sub_saturated) = match parsed_exp.value().checked_sub(frac_len)
            {
                Some(v) => (v, false),
                None => (i128::MIN, true),
            };
            // Folded into an `ExpParse` and matched exhaustively below,
            // mirroring the `Ok` arm above exactly, rather than a plain
            // `if saturated || sub_saturated` boolean fork (#1304 code
            // review: that was the identical unprotected-boolean pattern
            // the `Ok` arm's own enum match exists to avoid, just
            // relocated to this arm instead of actually closed).
            let shifted_exp = if parsed_exp.is_saturated() || sub_saturated {
                ExpParse::Saturated(shifted_exp_value)
            } else {
                ExpParse::Exact(shifted_exp_value)
            };
            match shifted_exp {
                // Same reasoning as the `Ok` arm above; the all-zero-mantissa
                // shift (`frac_len`) is dropped along with the exact-value
                // path rather than folded into the raw text (see
                // `assemble_scientific_from_raw_exponent`'s own doc comment
                // on why the shift isn't attempted here).
                ExpParse::Saturated(_) => {
                    assemble_scientific_from_raw_exponent(sign, "0", &s[exp_pos + 1..])
                }
                ExpParse::Exact(0) => format!("{sign}0"),
                ExpParse::Exact(shifted_exp) if (-6..0).contains(&shifted_exp) => {
                    format!("{sign}0.{}", "0".repeat((-shifted_exp) as usize))
                }
                ExpParse::Exact(shifted_exp) => assemble_scientific(sign, "0", shifted_exp),
            }
        }
    }
}

/// Render a nonzero literal's normalized mantissa (`leading[.rest]`, from
/// [`normalize_extreme_literal_mantissa`]) as jq's bare-integer or expanded-
/// decimal form, once the caller has confirmed `shifted_exp` is `0` or in
/// `-6..=-1` (#1226) -- the same window #1207 established for the
/// zero-mantissa case, extended here to any nonzero mantissa.
///
/// String-based, not value-based: `50e-1` and `5e0` parse to the identical
/// `f64` (`5.0`), but real jq renders them differently (`5.0` vs `5`) --
/// only the literal's own mantissa digits, not the parsed value, can tell
/// them apart. This replaced the previous value-only `format_decimal_jq`
/// (rounding to the shortest round-tripping precision, then trimming
/// trailing zeros), which could never have reconstructed `50e-1`'s own
/// trailing zero for exactly this reason.
///
/// - `shifted_exp == 0`: no decimal-point shift needed -- `rest` empty
///   renders as the bare `leading` digit (`0.5e1`'s mantissa `"5"` -> `5`);
///   `rest` non-empty keeps the mantissa's own decimal point in place
///   (`50e-1`'s mantissa `"5.0"` -> `5.0`, `15e-1`'s mantissa `"1.5"` ->
///   `1.5`).
/// - `shifted_exp` in `-6..=-1`: expand to `0.` followed by
///   `-shifted_exp - 1` zeros and then the mantissa's own digits
///   (`100e-7`'s mantissa `"1.00"` at shifted exponent `-5` -> `0.` + 4
///   zeros + `"1"` + `"00"` = `0.0000100`, preserving both the leading-zero
///   padding *and* the mantissa's own trailing zeros -- oracle-verified).
fn format_shifted_mantissa(sign: &str, mantissa_str: &str, shifted_exp: i128) -> String {
    let (leading, rest) = mantissa_str.split_once('.').unwrap_or((mantissa_str, ""));
    if shifted_exp == 0 {
        return join_sign_digits_with_optional_point(sign, leading, rest);
    }
    debug_assert!(
        (-6..=-1).contains(&shifted_exp),
        "caller only reaches here for shifted_exp == 0 or -6..=-1, got {shifted_exp}"
    );
    let zero_pad = usize::try_from(-shifted_exp - 1).unwrap_or(0);
    format!("{sign}0.{}{leading}{rest}", "0".repeat(zero_pad))
}

/// Join a sign and a digit string split at the decimal point, omitting the
/// point entirely when nothing follows it -- shared by
/// [`format_shifted_mantissa`]'s `shifted_exp == 0` arm and
/// [`format_positive_shifted_plain`] below, which both need to decide
/// between a bare integer (`123`) and a decimal (`123.45`) from the same
/// `(before, after)` split, so the two can't independently drift on how an
/// empty fractional remainder is spelled (#106, code review).
fn join_sign_digits_with_optional_point(sign: &str, before: &str, after: &str) -> String {
    if after.is_empty() {
        format!("{sign}{before}")
    } else {
        format!("{sign}{before}.{after}")
    }
}

/// Render a nonzero literal's normalized mantissa in plain (non-scientific)
/// decimal form for a *positive* shifted exponent (#1244), when the
/// literal's own given significant digits (`digit_count`) are enough to
/// cover the value's whole integer part without fabricating an unstated
/// trailing digit -- i.e. `shifted_exp < digit_count`, oracle-verified
/// against jq 1.7.1 as the exact condition real jq itself uses. Returns
/// `None` when that condition doesn't hold (caller falls back to
/// scientific notation).
///
/// **Caller contract (#1274):** `mantissa_str`'s own digit count must equal
/// `digit_count` exactly -- i.e. never a `normalize_extreme_literal_mantissa`
/// mantissa truncated by `MAX_RENDERED_MANTISSA_DIGITS` (`Some(..)`), only
/// an uncapped one (`None`). This function cannot detect a violation from
/// its own inputs alone: the `split_pos > digits.len()` check below only
/// catches a truncated mantissa when the decimal point's position
/// (`shifted_exp + 1`) falls *past* the truncated prefix -- but when it
/// falls *within* the truncated prefix, the check passes and this function
/// returns `Some` anyway, silently missing whatever trailing digits the cap
/// elided rather than erroring or falling back. `format_overflow_literal_mantissa`
/// (#1274) learned this the hard way: an earlier version of that caller
/// tried the capped mantissa first and only retried uncapped when this
/// function returned `None`, which fixed the "wrongly flips to truncated
/// scientific" symptom the issue reported but missed this quieter sibling
/// -- a "successful" plain render silently short digits whenever the split
/// point happened to land inside the capped prefix (e.g. `shifted_exp`
/// small relative to a `>100,000`-digit `digit_count`). The caller now
/// decides plain-vs-scientific eligibility itself, before ever building a
/// mantissa, and only calls this function with an uncapped one when
/// eligible -- this function's own guards remain as defense in depth, not
/// the primary correctness mechanism.
///
/// `mantissa_str` is `"d"` or `"d.ddd"` (one leading digit,
/// `normalize_extreme_literal_mantissa`'s normalized form) -- concatenating
/// its digits and re-inserting the decimal point `shifted_exp + 1` places
/// in gives the value's natural, un-normalized digit layout
/// (`"1.20000000000"` at shifted exponent `9` re-splits to
/// `"1200000000.00"`, matching real jq's own `120000000000e-2` ->
/// `1200000000.00`).
fn format_positive_shifted_plain(
    sign: &str,
    mantissa_str: &str,
    shifted_exp: i128,
    digit_count: i128,
) -> Option<String> {
    debug_assert!(
        shifted_exp > 0,
        "caller only reaches here for a positive shifted exponent, got {shifted_exp}"
    );
    if shifted_exp >= digit_count {
        return None;
    }
    let (leading, rest) = mantissa_str.split_once('.').unwrap_or((mantissa_str, ""));
    let digits = format!("{leading}{rest}");
    let split_pos = usize::try_from(shifted_exp + 1).ok()?;
    if split_pos > digits.len() {
        return None;
    }
    let (before, after) = digits.split_at(split_pos);
    Some(join_sign_digits_with_optional_point(sign, before, after))
}

/// Attempt [`format_positive_shifted_plain`], fetching an uncapped mantissa
/// via [`full_mantissa_if_capped`] only once eligibility (`shifted_exp <
/// digit_count`) is confirmed -- the shared "gate on cap-independent
/// eligibility, fetch, render" sequence both [`format_number_jq_compat`]
/// and [`format_overflow_literal_mantissa`] need. `mantissa_str` is the
/// caller's own (possibly `MAX_RENDERED_MANTISSA_DIGITS`-capped) mantissa;
/// it's used as-is only to decide there's nothing to fetch (`full_mantissa_if_capped`'s
/// own within-cap fast path), never passed uncapped-required call sites
/// directly.
///
/// #1274 review: this exact sequence, plus ~15 lines of near-duplicate
/// prose explaining why the gate-then-fetch ordering matters, was
/// duplicated between the two call sites before this helper existed --
/// shared here so a future fix to this pattern lands in one place instead
/// of two.
///
/// `format_positive_shifted_plain` is provably always `Some` once reached
/// here (not just usually): `full_mantissa_str` is uncapped whenever
/// `full_mantissa_if_capped` had to re-derive at all, so its own digit
/// count equals `digit_count` exactly, and `format_positive_shifted_plain`'s
/// `split_pos > digits.len()` guard reduces to the `shifted_exp <
/// digit_count` this function already checked above. Still returns
/// `Option<String>` rather than unwrapping internally, so a future change
/// to either function's contract fails safe (caller falls back to
/// scientific notation) instead of panicking.
fn try_positive_shifted_plain(
    sign: &str,
    s: &str,
    exp_pos: usize,
    shifted_exp: i128,
    digit_count: i128,
    mantissa_str: &str,
) -> Option<String> {
    if shifted_exp >= digit_count {
        return None;
    }
    let full_mantissa_str = full_mantissa_if_capped(s, exp_pos, mantissa_str, digit_count);
    format_positive_shifted_plain(sign, &full_mantissa_str, shifted_exp, digit_count)
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

    /// Like `from_number_literal` (private, this file only), but parses
    /// straight to a plain `Int`/`Float` (or [`Null`](Self::Null) on parse
    /// failure, same fallback) without ever wrapping `literal` in
    /// [`NumberLiteral`](Self::NumberLiteral) first.
    ///
    /// `pub`, not `pub(crate)`: #999's own motivation for this function is
    /// a caller (yq's own `--input-format json` path, #978) that lives in
    /// the CLI binary crate, which only sees this library's `pub` surface.
    /// `from_number_literal(literal).into_plain_number()` would still pay
    /// for and immediately discard the `Box<str>` allocation
    /// `from_number_literal` always makes -- exactly the cost a caller that
    /// never wants the source spelling in the first place shouldn't have
    /// to pay, and the reason this exists as its own function rather than
    /// that two-call sequence.
    pub fn from_number_literal_plain(literal: &str) -> Self {
        Self::plain_number_from_repr(parse_i64_or_f64(literal))
    }

    /// The shared "parsed repr -> plain scalar" mapping
    /// [`from_number_literal_plain`](Self::from_number_literal_plain) and
    /// [`from_number_bytes`](Self::from_number_bytes)'s own final fallback
    /// both need -- previously duplicated verbatim at each site (#999
    /// review).
    fn plain_number_from_repr(repr: Option<NumberRepr>) -> Self {
        match repr {
            Some(NumberRepr::Int(i)) => Self::Int(i),
            Some(NumberRepr::Float(f)) => Self::Float(f),
            None => Self::Null,
        }
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
        // way #1094 preserves a leading zero -- `bytes` was already gated
        // by `number_literal_end` at the scan site, and
        // `Self::from_number_literal` (via `parse_i64_or_f64`) parses a
        // leading-dot float natively, so no separate reparse of the
        // original text is needed here. `has_leading_dot`, not a hand-rolled
        // check inline here: shared with `crate::json::light`'s
        // `DocumentValue::number_literal` implementation, which needs the
        // identical escape (#2240 code review).
        if crate::json::validate::has_leading_dot(bytes) {
            return core::str::from_utf8(bytes).map_or(Self::Null, Self::from_number_literal);
        }
        // Real jq's own number reader also tolerates a redundant leading
        // zero in the integer part (`007` -> `7`, `007e5` -> `7E+5`,
        // `007.500` -> `7.500`, trailing zeros and exponent notation
        // otherwise kept) -- confirmed live against jq 1.7.1. As with the
        // leading-dot case just above, materialize the *original* `bytes`
        // as the literal spelling, not a stripped copy: dropping the
        // redundant zero is purely a display-time concern
        // (`format_number_jq_compat` strips it in its plain-integer and
        // plain-decimal-without-exponent branches, the only branches that
        // echo `NumberLiteral` text verbatim -- every exponent-bearing
        // branch already reformats from the *parsed* value/exponent
        // digits, leading zeros and all, so it needs no separate
        // handling), not something the stored spelling itself needs to
        // already reflect. `bytes` was already gated by
        // `strip_redundant_leading_zeros` + `is_valid_number` below, so no
        // separate reparse of the original text is needed here (an
        // earlier draft of this fix stored the *stripped* text directly
        // here instead, duplicating the display-time fix at both
        // materializer call sites; review found it simpler to store the
        // original everywhere, matching the leading-dot case, and fix
        // display once). The *only* reason this needs handling here at
        // all: the CLI's own `--argjson`-style "normalize, retry" fix
        // (#1094, `normalize_leading_zero_numbers` in
        // `src/bin/succinctly/jq_runner.rs`) doesn't reach here -- it's a
        // CLI-arg-only helper, not wired into this shared conversion, so
        // the plain document-input path (and `--argjson` too, since it
        // also ends up here) still fell all the way through to the lossy
        // `parse_i64_or_f64` below, losing the leading zero *and* any
        // exponent notation *and* any trailing zeros in one go (#1149).
        let zero_stripped = crate::json::validate::strip_redundant_leading_zeros(bytes);
        if let Some(stripped) = &zero_stripped {
            if crate::json::validate::is_valid_number(stripped) {
                return core::str::from_utf8(bytes).map_or(Self::Null, Self::from_number_literal);
            }
        }
        // Real jq's own number reader also tolerates a trailing `.`
        // immediately before an exponent marker (`1.e999` -> `1.0e999`,
        // `-1.e5` -> `-1.0e5`) -- not valid per strict RFC 8259 (`frac`
        // requires at least one digit after `.`), but real jq accepts it
        // and, crucially, still uses the *exact* decimal value to decide
        // how to print an out-of-range exponent: `1.e999` -> `1E+999`
        // there, not the double-precision-clamped
        // `1.7976931348623157e+308` a naive f64 parse produces (confirmed
        // live against jq 1.7.1, #2220). Same pattern as the leading-dot
        // and leading-zero escapes above: [`has_trailing_dot_before_exponent`]
        // checks whether inserting `0` right after the trailing `.` makes
        // the token strictly valid, and if so, still materialize the
        // *original* text as the literal spelling -- `Self::from_number_literal`
        // (via `parse_i64_or_f64`) parses a trailing-dot float natively, so
        // no separate reparse of the original text is needed here. A bare
        // trailing `.` with no exponent (`1.`) is deliberately left alone:
        // real jq doesn't preserve that spelling either (`[1.]` -> `[1]`
        // on both sides), so the existing lossy fallback below already
        // matches jq there.
        //
        // Checked against the leading-zero-stripped form (when one
        // exists), not always `bytes` itself, so the two escapes compose:
        // a token can have both a redundant leading zero *and* a trailing
        // dot before its exponent at once (`007.e999` -> jq's `7E+999`,
        // confirmed live) -- neither escape alone fixes that, since each
        // only resolves its own half.
        let base = zero_stripped.as_deref().unwrap_or(bytes);
        if crate::json::validate::has_trailing_dot_before_exponent(base) {
            return core::str::from_utf8(bytes).map_or(Self::Null, Self::from_number_literal);
        }
        let Ok(s) = core::str::from_utf8(bytes) else {
            return Self::Null;
        };
        Self::plain_number_from_repr(parse_i64_or_f64(s))
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

    /// Whether this value is a number in any of its three representations
    /// (`Int`, `Float`, or `NumberLiteral`) -- i.e. whether
    /// `into_plain_number()` would collapse it to `Int`/`Float` rather than
    /// hand it back unchanged. A cheap, non-consuming peek (#1199, built on
    /// the existing [`Self::number_repr`] rather than re-matching the same
    /// variants a second time): lets a caller decide *before* moving an
    /// operand into `into_plain_number()` whether doing so is actually safe
    /// -- consuming a genuinely non-numeric operand (`String`/`Array`/
    /// `Object`/`Bool`/`Null`) that way is a no-op for the value itself,
    /// but discards the caller's own binding to the original, so an error
    /// message built from the post-collapse value loses a `NumberLiteral`'s
    /// source spelling for nothing -- see the arith_* functions' own
    /// catch-all arms.
    pub(crate) fn is_number(&self) -> bool {
        self.number_repr().is_some()
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
        // Shared by both `NumberLiteral` arms below (infinite and finite):
        // this function's callers (`reduce`/`foreach`/etc.'s per-iteration
        // reindex bridge) can run it over the same unchanged value
        // thousands of times, and a document-sourced literal is
        // otherwise-unbounded text -- see each arm's own comment for why
        // its particular fallback is safe.
        const MAX_REUSED_LITERAL_LEN: usize = 256;
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
            // The length cap (`MAX_REUSED_LITERAL_LEN` above) *is* needed
            // regardless of shape: a bound this loose still comfortably
            // covers any realistic overflow literal (reaching `f64::MAX`
            // needs on the order of ~300 exponent digits at most) while
            // keeping the *reused* case itself O(1)-ish rather than
            // O(iterations x literal length) for a pathological one.
            Self::NumberLiteral(NumberRepr::Float(f), literal) if f.is_infinite() => {
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
            //
            // Same length cap as the infinite-literal arm above, and the
            // same reason (#1211, found in that issue's own PR review):
            // `is_preservable_float_literal` (`src/yaml/scalar.rs`) admits
            // an arbitrarily long zero-mantissa or leading-zero-heavy
            // literal (e.g. `0.` + 100,000 zeros), so this text is no
            // longer bounded to `MAX_PRESERVABLE_FLOAT_DIGITS` the way it
            // used to be incidentally. Without a cap here, a `reduce`/
            // `foreach`/`while`/`until` loop touching such a value
            // re-serializes the full literal on every iteration --
            // measured live, wall time linear in iteration count for a
            // 200,000-digit literal. Falls back to the parsed `NumberRepr`'s
            // own bounded formatting (discarding the literal text, the
            // finite-value analogue of `overflow_literal(*f)` above) rather
            // than the general `to_json_at_depth`'s own `NumberLiteral` arm,
            // which is not length-bounded either (it exists for one-shot
            // output, not this function's per-iteration reuse).
            Self::NumberLiteral(repr, literal) if literal.len() <= MAX_REUSED_LITERAL_LEN => {
                literal.to_string()
            }
            Self::NumberLiteral(NumberRepr::Int(n), _) => format!("{n}"),
            Self::NumberLiteral(NumberRepr::Float(f), _) if S::TAG == EvalTag::Yq => {
                crate::yaml::format_float_with_fraction(*f)
            }
            Self::NumberLiteral(NumberRepr::Float(f), _) => jq_bare_float_display(*f),
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

    /// #2220: a trailing-dot mantissa immediately before an exponent marker
    /// must preserve its own source spelling as a `NumberLiteral` too, the
    /// same way the leading-dot case above does -- direct unit coverage of
    /// `from_number_bytes`'s third escape, complementing the CLI-level
    /// regression tests. `1.e999`'s magnitude overflows `f64`, so the
    /// stored `NumberRepr` is `Float(INFINITY)` -- the literal text is what
    /// makes the eventual jq-compat display (`1E+999`, not the `f64::MAX`
    /// stand-in) correct, not this stored numeric value.
    ///
    /// Code review: `OwnedValue`'s `PartialEq` for `NumberLiteral` compares
    /// only the parsed `NumberRepr`, not the stored `Box<str>` text (two
    /// literals that both overflow to the same infinity compare equal
    /// regardless of spelling) -- `assert_eq!` against a full
    /// `NumberLiteral(...)` value would silently pass even if this escape
    /// stored a mangled or wrong literal, defeating the point of this
    /// test. Match and assert on the literal text explicitly instead.
    #[test]
    fn test_from_number_bytes_preserves_trailing_dot_before_exponent_spelling() {
        for (bytes, expected_repr, expected_literal) in [
            (&b"1.e5"[..], NumberRepr::Float(100000.0), "1.e5"),
            (&b"-1.e5"[..], NumberRepr::Float(-100000.0), "-1.e5"),
            (&b"1.e999"[..], NumberRepr::Float(f64::INFINITY), "1.e999"),
            (
                &b"-1.e999"[..],
                NumberRepr::Float(f64::NEG_INFINITY),
                "-1.e999",
            ),
            // Composes with the leading-zero escape above: a token can
            // have both a redundant leading zero *and* a trailing dot
            // before its exponent at once (`007.e999`) -- neither escape
            // alone fixes it, since each only resolves its own half.
            (
                &b"007.e999"[..],
                NumberRepr::Float(f64::INFINITY),
                "007.e999",
            ),
            (
                &b"-007.e999"[..],
                NumberRepr::Float(f64::NEG_INFINITY),
                "-007.e999",
            ),
        ] {
            match OwnedValue::from_number_bytes(bytes) {
                OwnedValue::NumberLiteral(repr, literal) => {
                    assert_eq!(repr, expected_repr, "input {bytes:?}");
                    assert_eq!(&*literal, expected_literal, "input {bytes:?}");
                }
                other => panic!("input {bytes:?}: expected NumberLiteral, got {other:?}"),
            }
        }
        // A trailing dot with *no* exponent (`1.`) is untouched by this
        // escape -- real jq doesn't preserve that spelling either (`[1.]`
        // -> `[1]`), so it still degrades to a plain `Float` via the
        // pre-existing fallback below, not a `NumberLiteral`.
        assert_eq!(OwnedValue::from_number_bytes(b"1."), OwnedValue::Float(1.0));
        // Genuinely malformed shapes (two dots) stay untouched by this
        // escape too: the inserted `0` can't make `1.5.3` valid either way.
        assert_eq!(OwnedValue::from_number_bytes(b"1.5.3"), OwnedValue::Null);
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
    fn test_is_number() {
        assert!(OwnedValue::Int(1).is_number());
        assert!(OwnedValue::Float(1.5).is_number());
        assert!(OwnedValue::NumberLiteral(NumberRepr::Int(1), "1".into()).is_number());
        assert!(!OwnedValue::String("1".into()).is_number());
        assert!(!OwnedValue::Bool(true).is_number());
        assert!(!OwnedValue::Null.is_number());
        assert!(!OwnedValue::Array(vec![]).is_number());
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

    /// #1211's own PR review: `is_preservable_float_literal`
    /// (`src/yaml/scalar.rs`) now admits an arbitrarily long zero-mantissa
    /// or leading-zero-heavy literal, so a *finite* `NumberLiteral` is no
    /// longer bounded to 17-ish digits the way it used to be incidentally.
    /// Same concern, same fix shape as the infinite-literal test above:
    /// short literals still reuse their own text; a pathologically long one
    /// falls back to the parsed value's own bounded formatting instead of
    /// paying O(its length) on every reindex-bridge call.
    #[test]
    fn test_to_json_for_reindex_bounds_an_unrealistically_long_finite_literal() {
        let short_literal = "0.000e-400";
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(0.0), short_literal.into());
        assert_eq!(lit.to_json_for_reindex::<JqSemantics>(), short_literal);

        let huge_zero_literal = format!("0.{}e-400", "0".repeat(200_000));
        let lit =
            OwnedValue::NumberLiteral(NumberRepr::Float(0.0), huge_zero_literal.clone().into());
        let out = lit.to_json_for_reindex::<JqSemantics>();
        assert!(
            out.len() < 100,
            "expected a short, bounded fallback, got {} bytes",
            out.len()
        );

        // Yq mode takes a separate branch for the same fallback
        // (`format_float_with_fraction` instead of `jq_bare_float_display`,
        // gated on `S::TAG`) -- must be bounded too.
        let lit = OwnedValue::NumberLiteral(NumberRepr::Float(0.0), huge_zero_literal.into());
        let out = lit.to_json_for_reindex::<YqSemantics>();
        assert!(
            out.len() < 100,
            "expected a short, bounded fallback, got {} bytes",
            out.len()
        );
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

    /// #1224: only reachable via a directly-constructed `NumberLiteral`
    /// (see the function's own doc comment) -- `is_valid_number`'s gate
    /// never lets this spelling through document/query input, so there's
    /// no CLI repro, only this unit-level one. Oracle-verified via jq's
    /// own lenient reader (`echo 007 | jq .` -> `7`, `echo +007 | jq .` ->
    /// `7`, `echo 007.5 | jq .` -> `7.5`, `echo -007 | jq .` -> `-7`,
    /// `echo -000 | jq .` -> `-0`) -- confirms the *target* canonical
    /// spelling, not just that the old verbatim-echo was wrong.
    #[test]
    fn test_format_number_jq_compat_plain_integer_strips_leading_zero_and_plus() {
        assert_eq!(format_number_jq_compat(b"007"), "7");
        assert_eq!(format_number_jq_compat(b"+007"), "7");
        assert_eq!(format_number_jq_compat(b"+7"), "7");
        assert_eq!(format_number_jq_compat(b"-007"), "-7");
        // All-zero digits collapse to a single canonical "0" -- but the
        // sign survives even then (real jq's own negative-zero rule).
        assert_eq!(format_number_jq_compat(b"000"), "0");
        assert_eq!(format_number_jq_compat(b"-000"), "-0");
        // Already-canonical spellings are unaffected.
        assert_eq!(format_number_jq_compat(b"0"), "0");
        assert_eq!(format_number_jq_compat(b"-0"), "-0");
        assert_eq!(format_number_jq_compat(b"7"), "7");
    }

    /// Same rule, plain-decimal branch -- only the integer part loses a
    /// digit; trailing zeros in the fractional part are untouched.
    /// Oracle-verified (`echo -007.500 | jq .` -> `-7.500`).
    #[test]
    fn test_format_number_jq_compat_plain_decimal_strips_leading_zero_and_plus() {
        assert_eq!(format_number_jq_compat(b"007.5"), "7.5");
        assert_eq!(format_number_jq_compat(b"+1.5"), "1.5");
        assert_eq!(format_number_jq_compat(b"-007.500"), "-7.500");
        assert_eq!(format_number_jq_compat(b"000.000"), "0.000");
        assert_eq!(format_number_jq_compat(b"-000.0"), "-0.0");
        // Already-canonical spellings, including #1171's own leading-dot
        // case, are unaffected -- the same helper now covers both.
        assert_eq!(format_number_jq_compat(b"0.10"), "0.10");
        assert_eq!(format_number_jq_compat(b".500"), "0.500");
        assert_eq!(format_number_jq_compat(b"-.500"), "-0.500");
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
        // #1207 review: this branch's own `is_sign_negative()` arm is only
        // reachable via a genuinely negative *nonzero* integer literal now
        // that `value == 0.0` (including `-0.0`) is intercepted earlier by
        // `format_near_zero_literal` -- no existing test exercised that arm
        // through its real case rather than incidentally through `-0e0`.
        assert_eq!(format_number_jq_compat(b"-5e0"), "-5");
    }

    /// #1264: an `exp == 0` literal outside the small-integer fast path
    /// above (a fractional mantissa, or an integer whose magnitude is `>=
    /// 1e15`) used to fall back to `format!("{value}")` -- plain `f64`
    /// `Display` -- silently losing precision beyond `f64`'s own
    /// round-trip guarantee instead of preserving the literal's exact
    /// source digits the way every other case in this function does.
    /// Oracle-verified against jq 1.7.1.
    #[test]
    fn test_format_number_jq_compat_e0_fractional_and_large_integer_preserve_precision_1264() {
        // Fractional mantissa: a trailing zero used to get silently dropped
        // (f64 Display trims it).
        assert_eq!(format_number_jq_compat(b"-807.77317590e0"), "-807.77317590");
        // Fractional mantissa past f64's ~17-significant-digit round-trip
        // guarantee: used to round to the nearest representable f64 instead
        // of echoing the literal's own digits.
        assert_eq!(
            format_number_jq_compat(b"-824118596092576.85097746e0"),
            "-824118596092576.85097746"
        );
        // Integer magnitude >= 1e15 (the small-integer fast path's own
        // cutoff): used to fall to the same lossy f64 Display, rounding to
        // the nearest representable value instead of the literal's exact
        // digits.
        assert_eq!(
            format_number_jq_compat(b"99999999999999999e0"),
            "99999999999999999"
        );
        assert_eq!(
            format_number_jq_compat(b"12345678901234567e0"),
            "12345678901234567"
        );
        // `e-0`/`E0`/`E-0` spellings all parse to the identical `exp == 0`,
        // exercised through the same removed fast path.
        assert_eq!(format_number_jq_compat(b"999.99e-0"), "999.99");
        assert_eq!(format_number_jq_compat(b"1.5E0"), "1.5");
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

    /// #1226: this test previously asserted `"1"`/`"-1"`, pinning the old
    /// value-based `format_decimal_jq`'s output -- but real jq actually
    /// keeps the mantissa's own trailing zeros here (`100e-2` -> `1.00`,
    /// oracle-verified), since `100e-2` and `1e0` parse to the identical
    /// `f64` and only the literal's own text distinguishes them. The old
    /// assertion was itself wrong, not just superseded.
    #[test]
    fn test_format_number_jq_compat_negative_exponent_preserves_mantissa_trailing_zeros_1226() {
        assert_eq!(format_number_jq_compat(b"100e-2"), "1.00");
        assert_eq!(format_number_jq_compat(b"-100e-2"), "-1.00");
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
    /// re-parses the exponent text itself at wide (`i128` as of #1270,
    /// originally `i64`) precision instead.
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

    /// #1178: a genuinely-all-zero mantissa still shifts its printed
    /// exponent by the fractional-zero-digit count, the same normalization
    /// #1099's nonzero-mantissa case above already gets -- #1099's own
    /// `test_format_number_jq_compat_underflow_beyond_i32_exponent_range_1099`
    /// zero-mantissa case (`0e-2147483649`) only covers a no-fraction
    /// spelling (shift 0), which is why this gap wasn't caught there.
    /// In-process counterpart to `jq_cli_tests.rs`'s CLI-level `_1178`
    /// tests, for the same `cargo llvm-cov` visibility reason as #1099's
    /// own in-process test above.
    #[test]
    fn test_format_number_jq_compat_zero_mantissa_shifts_by_fraction_length_1178() {
        assert_eq!(format_number_jq_compat(b"0.000e-400"), "0E-403");
        assert_eq!(format_number_jq_compat(b"0.0e400"), "0E+399");
        assert_eq!(format_number_jq_compat(b"0.00e-400"), "0E-402");
        assert_eq!(format_number_jq_compat(b"-0.00e-400"), "-0E-402");
        // No-fraction spelling still has shift 0 -- unaffected regression
        // guard alongside #1099's existing one.
        assert_eq!(format_number_jq_compat(b"0e-400"), "0E-400");
    }

    /// #1099 code review: the exponent digit string was parsed at `i32`
    /// width for `format_number_jq_compat`'s own notation dispatch (at the
    /// time, an `exp == 0` / `(-5..0)` fast path, since removed by #1264 --
    /// today's equivalent is `parse_literal_exponent`'s use inside
    /// `normalize_extreme_literal_mantissa`'s shift math), and an
    /// out-of-`i32`-range exponent silently became `0` via `.unwrap_or(0)`
    /// -- misrouting into whichever "written exponent is exactly zero" path
    /// existed at the time, before the mantissa-preserving logic above ever
    /// ran. Widening that parse to `i64` (`parse_literal_exponent`) fixes it
    /// for any realistic exponent, a fix this test still guards regardless
    /// of which branch structure carries it.
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

    /// #1099: an exponent digit string beyond `i64`'s own range (but still
    /// within `i128`'s, ~38 digits) is exactly preserved, not routed
    /// through the old `i64`-width `parse_literal_exponent` at all anymore
    /// (#1270 widened it to `i128` -- see that function's own doc comment).
    /// A 22-digit exponent comfortably exceeds `i64::MAX` (~19 digits) but
    /// is nowhere near `i128`'s own range, so this is exact fidelity, not a
    /// saturated sentinel -- oracle-unverifiable (jq's own decNumber breaks
    /// down far earlier, ~1,147,483,647, a documented, deliberate
    /// divergence -- see `format_near_zero_literal`'s doc comment) but
    /// internally consistent: the exponent text round-trips untouched.
    #[test]
    fn test_format_number_jq_compat_underflow_exponent_beyond_i64_range_1099() {
        assert_eq!(
            format_number_jq_compat(b"1e-99999999999999999999"),
            "1E-99999999999999999999"
        );
    }

    /// #1270/#1273: `parse_literal_exponent`'s own saturating fallback -- an
    /// exponent digit string that overflows even `i128` (not just `i64`)
    /// saturates internally rather than erroring, but #1273 fixed the one
    /// unceilinged caller (`format_near_zero_literal`) to detect that and
    /// echo the exponent's own raw source text instead of displaying the
    /// fixed, input-independent sentinel as if it were the literal's real
    /// exponent. Before #1273, this printed `1E-{i128::MIN.unsigned_abs()}`
    /// (nonsense unrelated to either the source text or jq's own behavior)
    /// regardless of how many nines the literal actually wrote; now it
    /// echoes the 45 nines verbatim.
    ///
    /// Unlike `MAX_RENDERED_MANTISSA_DIGITS`'s own cap (which truncates to
    /// a truthful, monotonically-accurate *prefix* of the real digits), the
    /// echoed text here isn't shift-adjusted (see
    /// `assemble_scientific_from_raw_exponent`'s doc comment) -- correct for
    /// this specific case only because a single-digit mantissa (`"1"`) has
    /// shift `0`, so the unshifted and shift-adjusted answers coincide.
    #[test]
    fn test_format_number_jq_compat_underflow_exponent_beyond_i128_range_1270() {
        let huge_exponent = "9".repeat(45); // comfortably past i128::MIN's ~38 digits
        assert_eq!(
            format_number_jq_compat(format!("1e-{huge_exponent}").as_bytes()),
            format!("1E-{huge_exponent}")
        );
    }

    /// #1273: the all-zero-mantissa sibling of the test above --
    /// `normalize_extreme_literal_mantissa`'s `Err(frac_len)` arm has its
    /// own direct `parse_literal_exponent` call (`format_near_zero_literal`
    /// line ~790), a separate saturation site from the `Ok` arm the test
    /// above exercises. Before #1273 this also leaked the `i128::MIN`
    /// sentinel; now it echoes the raw exponent text with mantissa `"0"`.
    #[test]
    fn test_format_number_jq_compat_all_zero_mantissa_exponent_beyond_i128_range_1273() {
        let huge_exponent = "9".repeat(45);
        assert_eq!(
            format_number_jq_compat(format!("0.0e-{huge_exponent}").as_bytes()),
            format!("0E-{huge_exponent}")
        );
    }

    /// #1273: a multi-digit mantissa's own `shift` is real but not applied
    /// to a saturated exponent (`assemble_scientific_from_raw_exponent`'s
    /// doc comment explains why) -- `"123"`'s shift is `2` (not `0`), so the
    /// exact expected string below is deliberately *not* shift-adjusted
    /// (`1.23E-<45 nines>`, not `1.23E-<45 nines minus 2>`); no live oracle
    /// exists that reliably handles ~45-digit exponents either (per
    /// `format_near_zero_literal`'s own doc comment on jq's own breakdown
    /// past `i32::MAX`), so this pins the deterministic value this
    /// implementation actually produces, not a claim about what real jq
    /// would show.
    #[test]
    fn test_format_number_jq_compat_multidigit_mantissa_exponent_beyond_i128_range_1273() {
        let huge_exponent = "9".repeat(45);
        assert_eq!(
            format_number_jq_compat(format!("123e-{huge_exponent}").as_bytes()),
            format!("1.23E-{huge_exponent}")
        );
    }

    /// #1273 review round 2: `exp_saturated` alone only tracks whether
    /// `parse_literal_exponent`'s own `.parse()` overflowed -- it missed
    /// that the *shift-fold* (`parsed_exp.saturating_add(shift)` in
    /// `normalize_extreme_literal_mantissa`) can independently saturate
    /// even when the raw parse succeeded, if `parsed_exp` lands within
    /// `shift` of `i128::MIN`/`MAX`. `shift` is unbounded (the mantissa's
    /// leading-zero-fraction-digit count is never capped), so this is
    /// reachable with an ordinary ~39-digit exponent -- confirmed live
    /// (before the `checked_add` fix): `0.005e-170141183460469231731687303715884105727`
    /// (exponent = `i128::MIN+1`, a fully in-range parse; mantissa `0.005`
    /// gives `shift=-2`) rendered `5E-170141183460469231731687303715884105728`,
    /// the exact fabricated sentinel this issue exists to eliminate,
    /// leaking through a second, uncaught saturation point.
    #[test]
    fn test_format_number_jq_compat_shift_fold_saturates_even_when_parse_does_not_1273() {
        // exponent text = i128::MIN + 1 -- parses exactly on its own.
        let near_min = (i128::MIN + 1).to_string();
        assert_eq!(
            format_number_jq_compat(format!("0.005e{near_min}").as_bytes()),
            // shift=-2 folds parsed_exp (i128::MIN+1) below i128::MIN --
            // the checked_add fix detects this and falls back to the raw
            // exponent text unshifted (same "not shift-adjusted" limitation
            // as the sibling test above), not the old i128::MIN sentinel.
            format!("5E{near_min}")
        );
    }

    /// #1273 review round 2, `Err(frac_len)` sibling of the test above:
    /// `format_near_zero_literal`'s all-zero-mantissa arm has its own
    /// `parsed_exp.saturating_sub(frac_len)`, which can independently
    /// underflow i128 even when `exp_saturated` (checked from the raw parse
    /// alone) is `false`. Confirmed live before the `checked_sub` fix:
    /// `0.00e-170141183460469231731687303715884105727` (frac_len=2)
    /// rendered `0E-170141183460469231731687303715884105728`, the sentinel
    /// again, through this second call site.
    #[test]
    fn test_format_number_jq_compat_frac_len_sub_saturates_even_when_parse_does_not_1273() {
        let near_min = (i128::MIN + 1).to_string();
        assert_eq!(
            format_number_jq_compat(format!("0.00e{near_min}").as_bytes()),
            format!("0E{near_min}")
        );
    }

    /// #1273 review round 2: the raw-exponent echo fallback must strip
    /// insignificant leading zeros, matching every other exponent-rendering
    /// path in this module (`assemble_scientific`'s own exponent always
    /// comes from `i128::Display`, which never emits one) -- confirmed live
    /// before this fix that a zero-padded overlong exponent echoed the pad
    /// verbatim (`E-000999...`), the one place this formatter's output
    /// wasn't leading-zero-free.
    #[test]
    fn test_format_number_jq_compat_raw_exponent_echo_strips_leading_zeros_1273() {
        let huge_exponent = "9".repeat(45);
        assert_eq!(
            format_number_jq_compat(format!("1e-00000{huge_exponent}").as_bytes()),
            format!("1E-{huge_exponent}")
        );
        assert_eq!(
            format_number_jq_compat(format!("0.0e000{huge_exponent}").as_bytes()),
            format!("0E+{huge_exponent}")
        );
    }

    /// `parse_literal_exponent`'s positive-sign saturation arm
    /// (`i128::MAX`), the counterpart to the `i128::MIN` case pinned above
    /// -- reached via `format_overflow_literal_mantissa` (a written
    /// exponent with no `-`, so `f64` parsing overflows to `+infinity`
    /// long before the exponent digit string itself overflows `i128`).
    /// `new_exp` saturating to `i128::MAX` is still far past the
    /// `1_000_000_000` overflow ceiling, so this correctly falls to the
    /// same DBL_MAX preview text as any other overflowed literal --
    /// oracle-verified against jq 1.7.1.
    #[test]
    fn test_format_number_jq_compat_overflow_exponent_beyond_i128_range_1270() {
        let huge_exponent = "9".repeat(45);
        assert_eq!(
            format_number_jq_compat(format!("1e{huge_exponent}").as_bytes()),
            "1.7976931348623157e+308"
        );
    }

    /// #1270's own repro, plus the shift-adjusted variant (#1270 review):
    /// a mantissa with more than one significant digit still correctly
    /// folds its own shift into the widened `i128` exponent rather than
    /// just echoing the raw exponent text unchanged -- `12.5e<24 nines>`'s
    /// mantissa shifts by `1` (two integer-part digits), so the printed
    /// exponent is one *greater* than the literal's own written exponent,
    /// not identical to it.
    #[test]
    fn test_format_number_jq_compat_astronomically_negative_exponent_preserves_digits_1270() {
        let exp24 = "9".repeat(24);
        let raw_exp: i128 = -exp24.parse::<i128>().unwrap();
        assert_eq!(
            format_number_jq_compat(format!("1e-{exp24}").as_bytes()),
            format!("1E{raw_exp}")
        );
        // Mantissa "12" shifts left by 1 digit, so the exponent increases
        // by exactly 1 relative to the raw written one -- computed via the
        // same `i128` arithmetic the production shift math uses (#1270
        // review), not hand-derived string surgery on the digit text.
        assert_eq!(
            format_number_jq_compat(format!("12.5e-{exp24}").as_bytes()),
            format!("1.25E{}", raw_exp + 1)
        );
        // All-zero mantissa (the `Err(frac_len)` branch) shares the same
        // widened arithmetic; `frac_len` is 1 (the mantissa's own single
        // fractional `0`).
        assert_eq!(
            format_number_jq_compat(format!("0.0e-{exp24}").as_bytes()),
            format!("0E{}", raw_exp - 1)
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
    /// at and just above the `f64::MIN_POSITIVE` boundary (2.2250738585072014e-308)
    /// must still take the finite `log10`/`pow` path unchanged -- the new
    /// subnormal check uses `<`, not `<=`, so it doesn't over-broaden into
    /// values the existing path already handles correctly. All three
    /// literals here are individually confirmed `f64::is_normal() == true`
    /// (an earlier draft of this test used `1.5e-308`, which is actually
    /// *subnormal* -- `1.5 < 2.2250738585072014` -- and so silently tested
    /// the new code path instead of the old one it claimed to guard,
    /// caught in code review).
    #[test]
    fn test_format_number_jq_compat_normal_boundary_near_min_positive_unaffected_1177() {
        assert_eq!(format_number_jq_compat(b"2.3e-308"), "2.3E-308");
        assert_eq!(format_number_jq_compat(b"3e-308"), "3E-308");
        assert_eq!(format_number_jq_compat(b"1e-300"), "1E-300");
    }

    /// #930 review: an overflowed literal's mantissa can be arbitrarily long
    /// (it's document-controlled text) - rendering the *whole* thing would
    /// be unbounded work for a pathological input, so
    /// `MAX_RENDERED_MANTISSA_DIGITS` (#1253: raised from `32` to
    /// `100_000` -- matching exactly what this session verified against
    /// pinned jq 1.7.1, not a round number chosen past it; still a cap, not
    /// a proven ceiling) caps the copy. This literal's own digit count
    /// (2,000,001) still exceeds even the raised cap, and its shifted
    /// exponent (2,000,399) still exceeds its own digit count either way,
    /// so it stays on the scientific path (`format_positive_shifted_plain`,
    /// #1244's own fix, correctly declines and falls through here -- see
    /// `test_format_number_jq_compat_overflow_huge_mantissa_stays_plain_when_digits_cover_it_1244`
    /// for the sibling case where the *same* overflow path picks plain
    /// instead). Pins that the leading digits (and thus the exponent, which
    /// is unaffected either way) stay correct, and that the rendered text
    /// itself is bounded rather than millions of bytes long.
    #[test]
    fn test_format_number_jq_compat_overflow_huge_mantissa_is_bounded() {
        let mantissa = "9".repeat(2_000_000);
        let literal = format!("{mantissa}.5e400");
        let result = format_number_jq_compat(literal.as_bytes());
        let expected_prefix = format!("9.{}", "9".repeat(100_000));
        assert!(
            result.starts_with(&expected_prefix),
            "leading digits must still be correct (first/last 40 shown): {}...{}",
            &result[..40.min(result.len())],
            &result[result.len().saturating_sub(40)..]
        );
        // The exponent shift (mantissa.len() - 1) is unaffected by the
        // rendering cap - only how many digits after the leading one get
        // copied into the output text.
        assert!(
            result.ends_with("E+2000399"),
            "exponent must reflect the mantissa's true (uncapped) length, last 20 chars: {}",
            &result[result.len().saturating_sub(20)..]
        );
        assert!(
            result.len() < 200_000,
            "must not render anywhere near the full 2,000,000-digit mantissa: got {} bytes",
            result.len()
        );
    }

    /// #1180: an insignificant leading zero or `+` in the mantissa's integer
    /// part used to be misread as its own significant leading digit,
    /// splicing the decimal point in one position too early (`007e-400` ->
    /// `0.07E-398` instead of `7E-400`) and leaking an un-normalized `+`
    /// straight into the output (`+0e-400` -> `+.0E-399` instead of
    /// `0E-400`). Only reachable by calling this `pub` function directly
    /// with a non-canonical mantissa spelling -- succinctly's own JSON/YAML
    /// literal-construction grammars already reject a leading zero or `+`
    /// before ever building a `NumberLiteral`, so no CLI input reaches this
    /// path with either spelling today.
    #[test]
    fn test_format_number_jq_compat_leading_zero_or_plus_mantissa_1180() {
        // Leading zero, both underflow and overflow directions.
        assert_eq!(format_number_jq_compat(b"007e-400"), "7E-400");
        assert_eq!(format_number_jq_compat(b"007e400"), "7E+400");
        // A leading zero ahead of more than one significant digit still
        // shifts by the significant digits' own count, not the raw
        // int-part length.
        assert_eq!(format_number_jq_compat(b"0070e-400"), "7.0E-399");
        // Leading zero(s) with no significant digit at all: same as a bare
        // `0`, falls into the genuinely-all-zero-mantissa case (#1178).
        assert_eq!(format_number_jq_compat(b"000e-400"), "0E-400");
        // Leading `+`, both alone and ahead of a genuine digit.
        assert_eq!(format_number_jq_compat(b"+0e-400"), "0E-400");
        assert_eq!(format_number_jq_compat(b"+1e400"), "1E+400");
    }

    /// #1180 review (Efficiency angle): stripping leading zeros is an O(k)
    /// scan over the leading-zero run, unlike the O(1) length check it
    /// replaced (see the updated comment above `MAX_RENDERED_MANTISSA_DIGITS`).
    /// A document-controlled mantissa with a huge leading-zero run must
    /// still produce the correct, bounded output rather than hanging or
    /// blowing up -- the counterpart to
    /// `test_format_number_jq_compat_overflow_huge_mantissa_is_bounded`
    /// above, which uses a huge mantissa with *no* leading zeros and so
    /// never exercises this scan at all.
    #[test]
    fn test_format_number_jq_compat_huge_leading_zero_run_is_bounded_1180() {
        let mantissa = format!("{}9", "0".repeat(2_000_000));
        let literal = format!("{mantissa}e400");
        assert_eq!(format_number_jq_compat(literal.as_bytes()), "9E+400");
    }

    /// #1207: a genuinely-zero-mantissa literal (`value == 0.0`) picks
    /// notation on its own *shifted* exponent (written exponent minus the
    /// mantissa's fractional-zero-digit count, #1178's shift math), not the
    /// literal's raw written exponent the way `format_number_jq_compat`'s
    /// `exp == 0`/`(-5..0)` fast paths do for everything else -- before this
    /// fix, every zero-mantissa literal with a nonzero written exponent went
    /// straight to `format_near_zero_literal`'s unconditional scientific
    /// output, regardless of how small the resulting exponent magnitude
    /// actually was. Oracle-verified against jq 1.7.1 for the
    /// full boundary, shifted exponents -8..=2 (`0e{shifted}` has no
    /// fraction digits, so its shift is 0 and its shifted exponent equals
    /// its written one).
    #[test]
    fn test_format_number_jq_compat_zero_mantissa_notation_threshold_1207() {
        // Deep in scientific territory on the underflow side, and the one
        // step just past the decimal window's own boundary.
        assert_eq!(format_number_jq_compat(b"0e-8"), "0E-8");
        assert_eq!(format_number_jq_compat(b"0e-7"), "0E-7");
        // The decimal-expansion window: shifted exponent -6..=-1, i.e.
        // 6 down to 1 zeros after the point.
        assert_eq!(format_number_jq_compat(b"0e-6"), "0.000000");
        assert_eq!(format_number_jq_compat(b"0e-5"), "0.00000");
        assert_eq!(format_number_jq_compat(b"0e-4"), "0.0000");
        assert_eq!(format_number_jq_compat(b"0e-3"), "0.000");
        assert_eq!(format_number_jq_compat(b"0e-2"), "0.00");
        assert_eq!(format_number_jq_compat(b"0e-1"), "0.0");
        // Shifted exponent exactly 0: bare integer, no decimal point at all.
        assert_eq!(format_number_jq_compat(b"0e0"), "0");
        // Positive shifted exponent: scientific again.
        assert_eq!(format_number_jq_compat(b"0e1"), "0E+1");
        assert_eq!(format_number_jq_compat(b"0e2"), "0E+2");
        // Sign is preserved through every branch (`-0.0` is `value == 0.0`
        // too, and `log10(0)` is undefined so jq keeps the written sign
        // rather than trying to derive one).
        assert_eq!(format_number_jq_compat(b"-0e-8"), "-0E-8");
        assert_eq!(format_number_jq_compat(b"-0e-1"), "-0.0");
        assert_eq!(format_number_jq_compat(b"-0e0"), "-0");
        assert_eq!(format_number_jq_compat(b"-0e1"), "-0E+1");
        // A written fractional zero digit shifts the exponent exactly like
        // #1178's already-scientific case does, so the same classification
        // above is reachable via a *different* written exponent than the
        // no-fraction cases -- e.g. `0.0e-5`'s shift is -6 (written -5,
        // minus 1 fractional digit), landing in the same decimal window as
        // `0e-6` above despite a different written exponent.
        assert_eq!(format_number_jq_compat(b"0.0e-5"), "0.000000"); // shift -6
        assert_eq!(format_number_jq_compat(b"0.0e0"), "0.0"); // shift -1
        assert_eq!(format_number_jq_compat(b"0.0e1"), "0"); // shift 0
        assert_eq!(format_number_jq_compat(b"0.00e-1"), "0.000"); // shift -3
        assert_eq!(format_number_jq_compat(b"0.0000e-1"), "0.00000"); // shift -5
                                                                      // A genuinely-underflowed *nonzero* mantissa (`value == 0.0` from
                                                                      // parsing, but the literal's own digits are not all zero) never
                                                                      // enters this small-window notation regardless of its own shifted
                                                                      // exponent's magnitude -- unaffected by #1207, still always
                                                                      // scientific, matching #1099/#1178's existing coverage.
        assert_eq!(format_number_jq_compat(b"1e-400"), "1E-400");
    }

    /// #1226: extends #1207's shifted-exponent notation rule from zero
    /// mantissas to *nonzero* ones -- `format_number_jq_compat`'s `exp ==
    /// 0`/`(-5..0)` fast paths used the literal's raw written exponent for
    /// every nonzero mantissa too, so a mantissa with 2+ significant
    /// digits (shifting the true magnitude away from what's written) could
    /// land in the wrong notation. Oracle-verified against jq 1.7.1 for
    /// every case below.
    #[test]
    fn test_format_number_jq_compat_nonzero_mantissa_notation_threshold_1226() {
        // The issue's own repro set: written exponent -6 sits just outside
        // the *old*, too-narrow `-5..0` window, but shift 0 (single-digit
        // mantissa) means shifted exponent is also -6 -- inside the real
        // `-6..=-1` window, so this must be decimal, not scientific.
        assert_eq!(format_number_jq_compat(b"5e-6"), "0.000005");
        // One step further out: shifted exponent -7, still scientific.
        assert_eq!(format_number_jq_compat(b"1e-7"), "1E-7");
        // A multi-digit mantissa shifts the exponent further still: `123`
        // is 3 digits, shift 2, so `123e-6`'s shifted exponent is -4.
        assert_eq!(format_number_jq_compat(b"123e-6"), "0.000123");
        assert_eq!(format_number_jq_compat(b"999e-6"), "0.000999");
        // Trailing zeros in the mantissa survive the decimal expansion --
        // `100e-7`'s shifted exponent is -5 (shift 2), and its own
        // trailing "00" must appear after the leading "1", not be lost the
        // way a value-only renderer (`0.00001` from the parsed f64 alone)
        // would lose them.
        assert_eq!(format_number_jq_compat(b"100e-7"), "0.0000100");
        // Already-correct cases before this fix (written exponent already
        // sat inside the old window and happened to agree with the
        // shifted one) stay correct.
        assert_eq!(format_number_jq_compat(b"123e-5"), "0.00123");
        assert_eq!(format_number_jq_compat(b"1e-3"), "0.001");

        // Shifted exponent exactly 0 -- a case the old `exp == 0` fast
        // path only caught when the *written* exponent was also 0. A
        // multi-digit mantissa can reach shifted exponent 0 via a
        // *nonzero* written exponent instead: `50e-1` (shift 1, written
        // -1) keeps its own trailing zero as `5.0`, not the bare `5` a
        // value-based renderer would give (`50e-1` and `5e0` parse to the
        // identical `f64`). `15e-1` (shift 1, written -1) similarly keeps
        // its own fractional digit as `1.5`. `0.5e1` (shift -1, written 1)
        // has no fractional remainder after normalizing, so it *does*
        // collapse to the bare integer `5`.
        assert_eq!(format_number_jq_compat(b"50e-1"), "5.0");
        assert_eq!(format_number_jq_compat(b"15e-1"), "1.5");
        assert_eq!(format_number_jq_compat(b"0.5e1"), "5");
        assert_eq!(format_number_jq_compat(b"5e-1"), "0.5");

        // Many-significant-digit mantissas at shifted exponent 0 stay
        // plain decimal regardless of digit count (unlike the *positive*
        // shifted-exponent side, tracked separately as #1244).
        assert_eq!(
            format_number_jq_compat(b"99999999999999e-13"),
            "9.9999999999999"
        );
        assert_eq!(
            format_number_jq_compat(b"12345678901234e-13"),
            "1.2345678901234"
        );

        // Sign is preserved through every branch.
        assert_eq!(format_number_jq_compat(b"-5e-6"), "-0.000005");
        assert_eq!(format_number_jq_compat(b"-50e-1"), "-5.0");
        assert_eq!(format_number_jq_compat(b"-0.5e1"), "-5");

        // Already-correct positive-exponent cases (untouched by #1226,
        // which only widens the negative-side window) stay correct.
        assert_eq!(format_number_jq_compat(b"5e0"), "5");
        assert_eq!(format_number_jq_compat(b"1e1"), "1E+1");
        assert_eq!(format_number_jq_compat(b"5e1"), "5E+1");

        // A genuinely-underflowed *nonzero* mantissa (`!value.is_normal()`,
        // #1099/#1177) never enters this notation window regardless of its
        // own shifted exponent's magnitude -- unaffected by #1226, routed
        // through `format_near_zero_literal`'s subnormal path instead,
        // still always scientific.
        assert_eq!(format_number_jq_compat(b"1e-400"), "1E-400");
    }

    /// #1206 bug 1: the scientific-notation path's mantissa rendering used
    /// to re-derive the mantissa from the parsed `f64` via
    /// `libm::log10`/`libm::pow`, then unconditionally trim trailing zeros
    /// -- losing a source spelling's own trailing zeros the way #993
    /// already established `NumberLiteral` rendering must not. Fixed by
    /// reusing the `normalize_extreme_literal_mantissa`-derived,
    /// source-digit-preserving mantissa/shifted-exponent pair every other
    /// scientific-notation-producing path here already uses. Every case
    /// below oracle-verified against jq 1.7.1.
    #[test]
    fn test_format_number_jq_compat_scientific_notation_preserves_trailing_zeros_1206() {
        assert_eq!(format_number_jq_compat(b"1.50e10"), "1.50E+10");
        assert_eq!(format_number_jq_compat(b"2.500e-20"), "2.500E-20");
        assert_eq!(format_number_jq_compat(b"1.20e6"), "1.20E+6");
        assert_eq!(format_number_jq_compat(b"3.000e100"), "3.000E+100");
        assert_eq!(format_number_jq_compat(b"5.0e-7"), "5.0E-7");
        assert_eq!(format_number_jq_compat(b"-5.0e-7"), "-5.0E-7");
    }

    /// #1206 bug 2: the same `f64`-arithmetic mantissa rendering's "snap to
    /// nearest integer if very close" heuristic
    /// (`(value.round() - value).abs() < 1e-10`) didn't re-validate the
    /// snapped result was still `< 10`, so a mantissa arbitrarily close to
    /// (but strictly less than) the next power of ten could round up to
    /// exactly `10` -- violating scientific notation's own `[1, 10)`
    /// single-leading-digit invariant and silently producing a
    /// *numerically wrong* value/exponent pair, not just an imprecise
    /// spelling. The string-based mantissa this now reuses has no such
    /// snapping step at all, so the invariant can't be violated regardless
    /// of how close the source digits sit to a power-of-ten boundary.
    /// Oracle-verified against jq 1.7.1.
    #[test]
    fn test_format_number_jq_compat_scientific_notation_mantissa_stays_below_ten_1206() {
        assert_eq!(
            format_number_jq_compat(b"9.9999999999999e-64"),
            "9.9999999999999E-64"
        );
        assert_eq!(
            format_number_jq_compat(b"9.9999999999999e-307"),
            "9.9999999999999E-307"
        );
        assert_eq!(
            format_number_jq_compat(b"9.999999999999999e300"),
            "9.999999999999999E+300"
        );
        assert_eq!(
            format_number_jq_compat(b"-9.9999999999999e-64"),
            "-9.9999999999999E-64"
        );
    }

    /// #1244: a large positive-magnitude literal's plain-vs-scientific
    /// notation choice depends on real jq's own significant-digit count
    /// (`shifted_exp < digit_count`), not a simple exponent window the way
    /// the negative side (#1226) does. Both of this issue's own
    /// oracle-verified repros: `500000e-1` (shifted exponent `4`, but `6`
    /// given digits -- stays plain) and `99999999999999e1` (shifted
    /// exponent `14`, but only `14` given digits -- one short, goes
    /// scientific).
    #[test]
    fn test_format_number_jq_compat_positive_shifted_exponent_digit_count_threshold_1244() {
        assert_eq!(format_number_jq_compat(b"500000e-1"), "50000.0");
        assert_eq!(
            format_number_jq_compat(b"99999999999999e1"),
            "9.9999999999999E+14"
        );
        // Negative-sign counterpart, same digit-count/shift relationship.
        assert_eq!(format_number_jq_compat(b"-500000e-1"), "-50000.0");
        // More given digits than the shift needs: extra digits land after
        // the decimal point, matching real jq's own `1200000000.00`.
        assert_eq!(format_number_jq_compat(b"120000000000e-2"), "1200000000.00");
        // A single given digit can never cover any positive shift -- always
        // scientific, regardless of how large the shift is.
        assert_eq!(format_number_jq_compat(b"1e10"), "1E+10");
        // Exactly enough digits to cover the shift with none left over: no
        // trailing decimal point. A nonzero written exponent, so this
        // actually reaches `format_positive_shifted_plain` (a written
        // exponent of `0` is intercepted earlier by this function's own
        // `exp == 0` fast path -- see #1264).
        assert_eq!(format_number_jq_compat(b"999.99e2"), "99999");
    }

    /// #1253: `format_number_jq_compat`'s scientific-notation mantissa used
    /// to inherit `MAX_RENDERED_MANTISSA_DIGITS` (originally tuned for the
    /// overflow/near-zero paths only) via #1206's shared string-based
    /// rendering, silently truncating an *ordinary*, normal-magnitude
    /// literal's mantissa at 32 significant digits even though its notation
    /// choice (scientific, `shifted_exp >= digit_count`) was already
    /// correct. Real jq preserves full literal precision unconditionally
    /// (oracle-verified up to 100,000 significant digits) -- this pins the
    /// issue's own 44-digit repro round-tripping exactly, well past the old
    /// 32-digit cap.
    #[test]
    fn test_format_number_jq_compat_scientific_mantissa_precision_above_old_cap_1253() {
        assert_eq!(
            format_number_jq_compat(b"1.234567890123456789012345678901234567890123e50"),
            "1.234567890123456789012345678901234567890123E+50"
        );
    }

    /// #1253 review: `format_positive_shifted_plain`'s `shifted_exp <
    /// digit_count` rule was originally wired into `format_number_jq_compat`
    /// only, leaving `format_overflow_literal_mantissa` -- reached once the
    /// literal's own magnitude already overflows `f64` -- unconditionally
    /// scientific regardless of digit count. An overflowed *value* doesn't
    /// mean the *literal* lacks enough given digits to stay plain: 400
    /// given nines is itself an `f64` overflow (`9e399` alone already
    /// exceeds `f64::MAX`), yet real jq still renders every one of those
    /// 400 given digits plainly, since `shifted_exp` (399) is still less
    /// than `digit_count` (400) -- oracle-verified against jq 1.7.1.
    #[test]
    fn test_format_number_jq_compat_overflow_huge_mantissa_stays_plain_when_digits_cover_it_1244() {
        let mantissa = "9".repeat(400);
        assert_eq!(
            format_number_jq_compat(format!("{mantissa}e0").as_bytes()),
            mantissa
        );
        // One digit short of covering the shift: falls back to scientific,
        // same as the non-overflow path's own boundary (#1244).
        assert_eq!(
            format_number_jq_compat(format!("{mantissa}e1").as_bytes()),
            format!("9.{}E+400", "9".repeat(399))
        );
    }

    /// #1274 fix: `format_overflow_literal_mantissa` used to flip a literal
    /// with more than `MAX_RENDERED_MANTISSA_DIGITS + 1` significant digits
    /// from plain to (truncated) scientific notation even when the
    /// literal's own given digits were enough to cover the shift and stay
    /// plain -- `format_positive_shifted_plain`'s decision used the true
    /// `digit_count`, but `mantissa_str` had already been truncated to
    /// `MAX_RENDERED_MANTISSA_DIGITS` before that decision could see it.
    /// Fixed by deciding eligibility on the cap-independent `new_exp`/
    /// `digit_count` first and only then fetching an uncapped mantissa
    /// (`full_mantissa_if_capped`) when eligible. Real jq stays plain at
    /// this scale unconditionally (oracle-verified past 500,000 digits, no
    /// ceiling found).
    ///
    /// This test used to pin the pre-fix divergence at exactly this
    /// boundary (`git log` has the characterization-test version); it now
    /// pins the fix instead: both sides of the old boundary, plus a scale
    /// well past it, stay plain and match real jq exactly.
    #[test]
    fn test_format_number_jq_compat_overflow_plain_stays_plain_past_the_render_cap_1274() {
        let at_cap = "9".repeat(100_001); // MAX_RENDERED_MANTISSA_DIGITS + 1 given digits
        assert_eq!(
            format_number_jq_compat(format!("{at_cap}e0").as_bytes()),
            at_cap,
            "at MAX_RENDERED_MANTISSA_DIGITS + 1 given digits: stays plain"
        );

        let past_cap = "9".repeat(100_002); // MAX_RENDERED_MANTISSA_DIGITS + 2 given digits
        assert_eq!(
            format_number_jq_compat(format!("{past_cap}e0").as_bytes()),
            past_cap,
            "at MAX_RENDERED_MANTISSA_DIGITS + 2 given digits: previously \
             flipped to truncated scientific -- now stays plain, matching \
             real jq"
        );

        // Well past the old cap (2x), to prove this is genuinely unbounded
        // now, not just a boundary shifted by one -- oracle-verified
        // against real jq up to 500,000 digits, no ceiling found.
        let well_past_cap = "3".repeat(200_000);
        assert_eq!(
            format_number_jq_compat(format!("{well_past_cap}e0").as_bytes()),
            well_past_cap
        );
    }

    /// #1274 review: the magnitude-`< 1` branch of
    /// `normalize_extreme_literal_mantissa` (a leading-zero-fraction
    /// mantissa, e.g. `0.999...e400`) has its own independent digit-cap
    /// truncation site (`after[..after.len().min(cap)]`), reached via the
    /// same `format_overflow_literal_mantissa` plain-eligibility path as
    /// the magnitude-`>= 1` case `test_format_number_jq_compat_overflow_plain_stays_plain_past_the_render_cap_1274`
    /// above already covers -- but a *different* code path inside
    /// `normalize_extreme_literal_mantissa` (the `int_part.is_empty()` arm,
    /// not the `else` arm), so it needed its own coverage: a full round-trip
    /// test on only the `else` arm doesn't exercise this one at all.
    #[test]
    fn test_format_number_jq_compat_overflow_plain_stays_plain_past_the_render_cap_fraction_branch_1274(
    ) {
        // `0.` + 100,002 nines + `e400`: mantissa shift is `-1` (leading
        // nonzero fractional digit at position 0), so `new_exp = 400 - 1 =
        // 399`, still `< digit_count` (100,002) -- plain-eligible. Unlike
        // the magnitude->=1 sibling test above, `new_exp` doesn't land on
        // `digit_count - 1` here, so the correct output isn't a bare
        // integer -- it's the decimal point inserted 400 digits in
        // (oracle-verified against real jq 1.7.1).
        let digits = "9".repeat(100_002); // MAX_RENDERED_MANTISSA_DIGITS + 2 given digits
        let expected = format!("{}.{}", "9".repeat(400), "9".repeat(99_602));
        assert_eq!(
            format_number_jq_compat(format!("0.{digits}e400").as_bytes()),
            expected,
            "magnitude < 1, past the render cap: stays plain, matching real jq"
        );
    }

    /// #1274 review: `digit_count` -- not `shifted_exp` -- is what actually
    /// needs to exceed `MAX_RENDERED_MANTISSA_DIGITS` to trigger this bug
    /// class, so it's reachable even through `format_number_jq_compat`'s
    /// *ordinary* (non-overflow) path, for a perfectly ordinary-magnitude
    /// value with an enormous fractional part -- previously undetected
    /// because every other test in this area happened to pair a huge digit
    /// count with a huge shift (the overflow path's own territory). Also
    /// covers `format_shifted_mantissa`'s `shifted_exp == 0` window
    /// directly (`shift = 0` here, both cases), which -- unlike
    /// `format_positive_shifted_plain` -- has no scientific fallback to
    /// silently mask a truncated mantissa behind at all: it must always
    /// render every given digit.
    #[test]
    fn test_format_number_jq_compat_ordinary_path_plain_stays_plain_past_the_render_cap_1274() {
        let frac = "9".repeat(150_000);
        // shift = 0 (single leading digit `7`): exercises
        // `format_shifted_mantissa`'s `shifted_exp == 0` arm directly.
        assert_eq!(
            format_number_jq_compat(format!("7.{frac}e0").as_bytes()),
            format!("7.{frac}"),
            "ordinary (non-overflow) path, shifted_exp == 0, digit_count past the cap"
        );

        // A modest positive shift, still nowhere near overflowing `f64`,
        // routes through `format_positive_shifted_plain` instead --
        // exercising this bug class's other branch on the ordinary path.
        let int_part = "1".repeat(50);
        assert_eq!(
            format_number_jq_compat(format!("{int_part}.{frac}e0").as_bytes()),
            format!("{int_part}.{frac}"),
            "ordinary (non-overflow) path, positive shifted_exp, digit_count past the cap"
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
