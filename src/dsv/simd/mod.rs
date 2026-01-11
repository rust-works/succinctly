//! SIMD-accelerated DSV semi-indexing.
//!
//! This module provides vectorized implementations of DSV (CSV/TSV) parsing
//! that process multiple bytes at once using SIMD instructions.
//!
//! The algorithm is based on the hw-dsv approach:
//! - Use SIMD to find all quotes, delimiters, and newlines in parallel
//! - Use arithmetic carry propagation to mask out characters inside quotes
//! - The trick: quote positions create a mask where odd quotes "open" and even quotes "close"
//!
//! ## Algorithm
//!
//! For each 64-byte chunk:
//! 1. Find all quote, delimiter, and newline positions using SIMD comparisons
//! 2. Compute the "in-quote" mask using prefix XOR on the quote positions
//! 3. Mask out delimiters and newlines that are inside quotes
//!
//! ## x86_64 Instruction Sets
//!
//! - **SSE2** (baseline): 16 bytes/iteration, universal availability
//! - **AVX2** (optimal): 32 bytes/iteration, ~95% availability (2013+)
//!
//! ## ARM
//!
//! On ARM aarch64, NEON intrinsics process 16 bytes at a time.

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "x86_64")]
pub mod avx2;

#[cfg(target_arch = "x86_64")]
pub mod sse2;

// ============================================================================
// ARM exports (NEON)
// ============================================================================

#[cfg(target_arch = "aarch64")]
pub use neon::build_index_simd;

// ============================================================================
// x86_64 exports with runtime dispatch
// ============================================================================

#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub fn build_index_simd(
    text: &[u8],
    config: &super::DsvConfig,
) -> super::DsvIndex {
    if is_x86_feature_detected!("avx2") {
        avx2::build_index_simd(text, config)
    } else {
        sse2::build_index_simd(text, config)
    }
}

// Without std feature, default to SSE2
#[cfg(all(target_arch = "x86_64", not(any(test, feature = "std"))))]
pub use sse2::build_index_simd;

// ============================================================================
// Fallback for other platforms
// ============================================================================

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub fn build_index_simd(
    text: &[u8],
    config: &super::DsvConfig,
) -> super::DsvIndex {
    super::parser::build_index(text, config)
}
