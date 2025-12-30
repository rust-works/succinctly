//! SIMD-accelerated JSON semi-indexing.
//!
//! This module provides vectorized implementations of JSON semi-indexing
//! that process multiple bytes at once using SIMD instructions.
//!
//! On ARM, NEON intrinsics process 16 bytes at a time.
//! On x86_64, AVX2 intrinsics could process 32 bytes at a time (not yet implemented).

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "aarch64")]
pub use neon::build_semi_index_standard;

// Fallback to scalar for non-SIMD platforms
#[cfg(not(target_arch = "aarch64"))]
pub use super::standard::build_semi_index as build_semi_index_standard;
