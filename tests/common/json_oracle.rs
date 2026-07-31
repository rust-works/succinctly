//! Shared oracle logic for JSON-validator checking.
//!
//! Included by both `tests/json_validate_properties.rs` and the fuzz targets in
//! `fuzz/fuzz_targets/`, via `#[path = ...] mod`. The fuzz crate is a separate
//! workspace on a separate toolchain, so it cannot depend on the test crate —
//! sharing the source file is what keeps the two from drifting apart, which
//! matters most for [`classify_divergence`], where a stale copy would silently
//! excuse a real bug.
//!
//! Lives under `tests/common/` because cargo auto-discovers `tests/*.rs` as test
//! binaries but not files in subdirectories.

#![allow(dead_code)] // Each consumer uses a different subset.

use succinctly::json::validate::Position;

/// Recompute a [`Position`] from scratch for a byte offset.
///
/// The validator tracks `line`/`column` incrementally, but it only ever mutates
/// `line` inside `skip_whitespace`; `advance` bumps `column` alone, and a raw
/// `\n`/`\r` can never reach it (inside a string those bytes hit the `< 0x20`
/// control-character arm and error first). So position is fully determined by
/// the prefix, and this reimplementation must agree everywhere.
///
/// CRLF counts as a single line break, matching `skip_whitespace`.
pub fn position_of(input: &[u8], offset: usize) -> Position {
    let mut line = 1usize;
    let mut line_start = 0usize;
    let mut i = 0usize;
    while i < offset.min(input.len()) {
        match input[i] {
            b'\r' => {
                line += 1;
                i += if input.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                line_start = i;
            }
            b'\n' => {
                line += 1;
                i += 1;
                line_start = i;
            }
            _ => i += 1,
        }
    }
    Position {
        offset,
        line,
        column: offset - line_start + 1,
    }
}

/// A disagreement with `serde_json` that is understood and deliberate.
#[derive(Debug, PartialEq, Eq)]
pub enum KnownDivergence {
    /// serde_json's recursion budget starts at 128 and errors when it hits 0,
    /// so its deepest accepted nesting is 127; ours is 128 (`enter_nested`
    /// errors on `>= 128` *before* incrementing). The two disagree at exactly
    /// depth 128.
    DepthLimit,
    /// serde_json errors when a number overflows f64 (it rejects an infinite
    /// `f64_from_parts` result). RFC 8259 §6 places no range or precision limit
    /// on numbers — it only constrains the grammar — so we accept them.
    /// JSONTestSuite agrees this is implementation-defined (the `i_number_*`
    /// cases; see `tests/data/json-test-suite-i-decisions.txt`).
    NumberRange,
}

/// Explain a disagreement with `serde_json`, or `None` if it is a real bug.
///
/// A code-level classifier rather than a data allowlist on purpose: the
/// divergence set is closed and reasoned, and extending it should require a
/// human writing down an RFC clause, not appending a line to a text file.
pub fn classify_divergence(input: &[u8], ours: bool, theirs: bool) -> Option<KnownDivergence> {
    // We accept, serde rejects.
    if ours && !theirs {
        // Exactly 128, not `>= 127`: serde's deepest accepted nesting is 127 and
        // ours is 128, so 128 is the *only* depth at which the two can disagree.
        // A looser bound would excuse any disagreement in a deeply-nested
        // document, whatever its real cause — and this classifier is the
        // fuzzer's oracle, where a false "known divergence" hides a bug forever.
        if max_nesting_depth(input) == 128 {
            return Some(KnownDivergence::DepthLimit);
        }
        if has_f64_overflowing_number(input) {
            return Some(KnownDivergence::NumberRange);
        }
    }
    None
}

/// Maximum bracket nesting depth, ignoring brackets inside strings.
pub fn max_nesting_depth(input: &[u8]) -> usize {
    let (mut depth, mut max, mut i) = (0usize, 0usize, 0usize);
    while i < input.len() {
        match input[i] {
            b'"' => {
                i += 1;
                while i < input.len() {
                    match input[i] {
                        b'\\' => i += 2,
                        b'"' => break,
                        _ => i += 1,
                    }
                }
            }
            b'{' | b'[' => {
                depth += 1;
                max = max.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    max
}

/// Whether any number literal outside a string overflows or underflows `f64`.
pub fn has_f64_overflowing_number(input: &[u8]) -> bool {
    let mut i = 0usize;
    while i < input.len() {
        match input[i] {
            b'"' => {
                i += 1;
                while i < input.len() {
                    match input[i] {
                        b'\\' => i += 2,
                        b'"' => break,
                        _ => i += 1,
                    }
                }
                i += 1;
            }
            b'-' | b'0'..=b'9' => {
                let start = i;
                i += 1;
                while i < input.len()
                    && matches!(input[i], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    i += 1;
                }
                if let Ok(text) = core::str::from_utf8(&input[start..i]) {
                    match text.parse::<f64>() {
                        Ok(v) if v.is_infinite() => return true,
                        // Underflow: rounds to zero despite a non-zero mantissa.
                        // The mantissa test is what keeps `0e0` — which is
                        // exactly zero, not an underflow — from classifying, and
                        // with it every divergence in any document containing a
                        // zero-with-exponent literal.
                        Ok(v) if v == 0.0 && is_underflow_literal(text) => return true,
                        Err(_) => return true,
                        _ => {}
                    }
                }
            }
            _ => i += 1,
        }
    }
    false
}

/// Whether a literal that parsed to `0.0` did so by underflowing.
///
/// True only if the mantissa holds a non-zero digit, so `1e-400` counts and
/// `0e0` / `-0.0e5` do not.
fn is_underflow_literal(text: &str) -> bool {
    let mantissa = text.split(['e', 'E']).next().unwrap_or(text);
    mantissa.bytes().any(|b| b.is_ascii_digit() && b != b'0')
}

/// Render bytes for an assertion message: lossy text plus a hex dump, because
/// much of what reaches these assertions is deliberately not UTF-8 and a
/// lossy-only rendering is unreadable exactly where debugging matters most.
pub fn render(bytes: &[u8]) -> String {
    let n = bytes.len().min(64);
    let hex: Vec<String> = bytes[..n].iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{:?}{} [{}{}]",
        String::from_utf8_lossy(&bytes[..n]),
        if bytes.len() > n { "..." } else { "" },
        hex.join(" "),
        if bytes.len() > n { " ..." } else { "" },
    )
}

/// Assert we and `serde_json` agree on validity, modulo classified divergences.
pub fn assert_serde_agreement(label: &str, input: &[u8]) {
    let ours = succinctly::json::validate::validate(input).is_ok();
    let theirs = serde_json::from_slice::<serde_json::Value>(input).is_ok();
    if ours == theirs {
        return;
    }
    assert!(
        classify_divergence(input, ours, theirs).is_some(),
        "{label}: unclassified disagreement with serde_json\n  \
         succinctly: {}\n  serde_json: {}\n  input: {}",
        if ours { "accept" } else { "reject" },
        if theirs { "accept" } else { "reject" },
        render(input),
    );
}

/// Assert the reported position is reconstructible from its offset alone.
pub fn assert_position_invariant(label: &str, input: &[u8]) {
    if let Err(e) = succinctly::json::validate::validate(input) {
        assert_eq!(
            e.position,
            position_of(input, e.position.offset),
            "{label}: position disagrees with recomputation for {}",
            render(input)
        );
        assert!(
            e.position.offset <= input.len(),
            "{label}: offset {} past end {}",
            e.position.offset,
            input.len()
        );
    }
}
