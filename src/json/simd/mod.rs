//! SIMD-accelerated JSON semi-indexing.
//!
//! This module provides vectorized implementations of JSON semi-indexing
//! that process multiple bytes at once using SIMD instructions.
//!
//! ## x86_64 Instruction Set Levels
//!
//! - **SSE2** (baseline): 16 bytes/iteration, universal availability
//! - **SSE4.2** (enhanced): 16 bytes/iteration with PCMPISTRI, ~90% availability (2008+)
//! - **AVX2** (optimal): 32 bytes/iteration, ~95% availability (2013+)
//!
//! The implementation uses runtime CPU detection to automatically select
//! the best available instruction set.
//!
//! ## ARM
//!
//! On ARM aarch64, NEON intrinsics process 16 bytes at a time (mandatory).
//!
//! ## Cursor Algorithms
//!
//! Both simple and standard cursor algorithms are provided:
//! - [`build_semi_index_simple`]: Simple cursor (3-state, marks structural chars)
//! - [`build_semi_index_standard`]: Standard cursor (4-state, marks value starts)

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "x86_64")]
pub mod sse42;

#[cfg(target_arch = "x86_64")]
pub mod avx2;

// ============================================================================
// ARM exports (NEON only)
// ============================================================================

#[cfg(target_arch = "aarch64")]
pub use neon::build_semi_index_simple;

#[cfg(target_arch = "aarch64")]
pub use neon::build_semi_index_standard;

// ============================================================================
// x86_64 exports with optional runtime dispatch (requires std)
// ============================================================================

// In std mode (tests), use runtime dispatch to select best SIMD level
// Priority: AVX2 > SSE4.2 > SSE2
#[cfg(all(target_arch = "x86_64", test))]
pub fn build_semi_index_standard(json: &[u8]) -> crate::json::standard::SemiIndex {
    if is_x86_feature_detected!("avx2") {
        avx2::build_semi_index_standard(json)
    } else if is_x86_feature_detected!("sse4.2") {
        sse42::build_semi_index_standard(json)
    } else {
        x86::build_semi_index_standard(json)
    }
}

#[cfg(all(target_arch = "x86_64", test))]
pub fn build_semi_index_simple(json: &[u8]) -> crate::json::simple::SemiIndex {
    if is_x86_feature_detected!("avx2") {
        avx2::build_semi_index_simple(json)
    } else if is_x86_feature_detected!("sse4.2") {
        sse42::build_semi_index_simple(json)
    } else {
        x86::build_semi_index_simple(json)
    }
}

// In no_std mode, default to SSE2 (universally available)
// Users can explicitly use avx2:: module if they know AVX2 is available
#[cfg(all(target_arch = "x86_64", not(test)))]
pub use x86::build_semi_index_simple;

#[cfg(all(target_arch = "x86_64", not(test)))]
pub use x86::build_semi_index_standard;

// ============================================================================
// Fallback for other platforms
// ============================================================================

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::simple::build_semi_index as build_semi_index_simple;

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::standard::build_semi_index as build_semi_index_standard;
