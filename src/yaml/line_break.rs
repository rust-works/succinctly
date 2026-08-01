//! The YAML line-break rule — re-exported from [`crate::text::line_break`].
//!
//! YAML 1.2 §5.4 spells a line break three ways: `\n`, a lone `\r`, and `\r\n`
//! as the two-byte spelling of a *single* break. Every scalar consumer of YAML
//! text in this module — the oracle ([`super::parser`]), the cursor
//! ([`super::light`]), and the strict validator ([`super::validate`]) — needs
//! the same two answers: *is this byte a break*, and *how wide is the break
//! here*.
//!
//! #341 collapsed roughly twenty open-coded copies of that rule onto this
//! module. #228 then added a consumer outside `yaml` —
//! [`crate::text::LineIndex`], which indexes JSON and DSV text as well — and
//! the rule is byte-identical for all three formats, so the definition moved up
//! to [`crate::text::line_break`] rather than being copied a second time. This
//! module stays as the YAML-facing name and the place the YAML rationale is
//! recorded; there is still exactly one definition.
//!
//! One deliberate exception survives: [`super::parser::Parser::skip_line_break`]
//! keeps a hand-rolled dispatch for a measured reason documented there. It is
//! pinned to [`line_break_len`] by a test rather than by the type system.
//!
//! The SIMD kernels under [`super::simd`] keep their own representation — a
//! `carriage_returns` mask cannot be phrased as a byte predicate — and stay
//! covered by the per-kernel differential tests.

pub(super) use crate::text::line_break::{is_line_break, line_break_len, line_break_len_before};
