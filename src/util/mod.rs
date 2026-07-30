//! Internal utilities for bit manipulation and SIMD operations.
//!
//! This module contains low-level utilities used by the succinct data structures.
//! Most users should not need to use these directly.

pub(crate) mod broadword;
pub(crate) mod table;

// Scan-length instrumentation for the select word-scan loops (#40). Always
// compiled under `std` so its own logic stays covered by the ordinary test
// run; only the call sites in the hot loops are gated on `select-stats`.
#[cfg(any(feature = "std", test))]
pub mod select_stats;

// Always compiled: `simd::escape` provides the portable scalar escape scanner
// used under `scalar-yaml` and on non-x86/arm targets. The arch-specific
// popcount submodules inside stay gated to their target.
pub(crate) mod simd;

pub use broadword::select_in_word;
