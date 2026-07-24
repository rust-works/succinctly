//! Internal utilities for bit manipulation and SIMD operations.
//!
//! This module contains low-level utilities used by the succinct data structures.
//! Most users should not need to use these directly.

pub(crate) mod broadword;
pub(crate) mod table;

// Always compiled: `simd::escape` provides the portable scalar escape scanner
// used under `scalar-yaml` and on non-x86/arm targets. The arch-specific
// popcount submodules inside stay gated to their target.
pub(crate) mod simd;

pub use broadword::select_in_word;
