//! SIMD-accelerated JSON semi-indexing.
//!
//! This module provides vectorized implementations of JSON semi-indexing
//! that process multiple bytes at once using SIMD instructions.
//!
//! On ARM, NEON intrinsics process 16 bytes at a time.
//! On x86_64, SSE2 intrinsics process 16 bytes at a time.
//!
//! Both simple and standard cursor algorithms are provided:
//! - [`build_semi_index_simple`]: Simple cursor (3-state, marks structural chars)
//! - [`build_semi_index_standard`]: Standard cursor (4-state, marks value starts)

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "x86_64")]
pub mod x86;

// Standard cursor SIMD exports
#[cfg(target_arch = "aarch64")]
pub use neon::build_semi_index_standard;

#[cfg(target_arch = "x86_64")]
pub use x86::build_semi_index_standard;

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::standard::build_semi_index as build_semi_index_standard;

// Simple cursor SIMD exports
#[cfg(target_arch = "aarch64")]
pub use neon::build_semi_index_simple;

#[cfg(target_arch = "x86_64")]
pub use x86::build_semi_index_simple;

// Fallback to scalar for simple cursor on other platforms
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::simple::build_semi_index as build_semi_index_simple;
