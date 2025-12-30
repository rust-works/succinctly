//! SIMD-accelerated JSON semi-indexing.
//!
//! This module provides vectorized implementations of JSON semi-indexing
//! that process multiple bytes at once using SIMD instructions.
//!
//! On ARM, NEON intrinsics process 16 bytes at a time.
//! On x86_64, SSE2 intrinsics process 16 bytes at a time.

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "aarch64")]
pub use neon::build_semi_index_standard;

#[cfg(target_arch = "x86_64")]
pub use x86::build_semi_index_standard;

// Fallback to scalar for non-SIMD platforms (e.g., 32-bit x86, WASM, etc.)
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::standard::build_semi_index as build_semi_index_standard;
