//! Internal utilities for bit manipulation and SIMD operations.
//!
//! This module contains low-level utilities used by the succinct data structures.
//! Most users should not need to use these directly.

pub(crate) mod broadword;
pub(crate) mod table;

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub(crate) mod simd;

pub use broadword::select_in_word;
