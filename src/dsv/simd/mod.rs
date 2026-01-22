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
//! 2. Compute the "in-quote" mask using prefix XOR (or BMI2 PDEP / SVE2 BDEP on supported CPUs)
//! 3. Mask out delimiters and newlines that are inside quotes
//!
//! ## x86_64 Instruction Sets
//!
//! - **BMI2 + AVX2** (fastest): Uses PDEP for quote masking, ~10x faster than prefix_xor
//! - **AVX2** (fast): 32 bytes/iteration with prefix_xor, ~95% availability (2013+)
//! - **SSE2** (baseline): 16 bytes/iteration, universal availability
//!
//! ## ARM aarch64
//!
//! - **SVE2-BITPERM + NEON** (fastest): Uses BDEP for quote masking, ~10x faster than prefix_xor
//!   - Supported: Azure Cobalt 100, AWS Graviton 4, Neoverse N2/V2
//! - **NEON** (baseline): 16 bytes/iteration with prefix_xor, universal on aarch64

#[cfg(all(target_arch = "aarch64", feature = "std"))]
use std::arch::is_aarch64_feature_detected;

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "aarch64")]
pub mod sve2;

#[cfg(target_arch = "x86_64")]
pub mod avx2;

#[cfg(target_arch = "x86_64")]
pub mod bmi2;

#[cfg(target_arch = "x86_64")]
pub mod sse2;

// ============================================================================
// ARM exports with runtime dispatch (SVE2 > NEON)
// ============================================================================

/// Build a DSV index using the fastest available SIMD implementation.
///
/// Runtime dispatch order (fastest to slowest):
/// 1. SVE2-BITPERM + NEON: Uses BDEP for quote masking (~10x faster)
/// 2. NEON: Uses prefix_xor for quote masking (fallback)
#[cfg(all(target_arch = "aarch64", feature = "std"))]
pub fn build_index_simd(text: &[u8], config: &super::DsvConfig) -> super::DsvIndex {
    // Check for SVE2-BITPERM (fastest path on ARM)
    if is_aarch64_feature_detected!("sve2-bitperm") {
        return sve2::build_index_simd(text, config);
    }

    // Fall back to NEON with prefix_xor
    neon::build_index_simd(text, config)
}

// Without std feature, default to NEON (can't do runtime detection)
#[cfg(all(target_arch = "aarch64", not(feature = "std")))]
pub use neon::build_index_simd;

// ============================================================================
// x86_64 exports with runtime dispatch
// ============================================================================

/// Build a DSV index using the fastest available SIMD implementation.
///
/// Runtime dispatch order (fastest to slowest):
/// 1. BMI2 + AVX2: Uses PDEP for quote masking (~10x faster)
/// 2. AVX2: Uses prefix_xor for quote masking
/// 3. SSE2: Fallback for older CPUs
#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub fn build_index_simd(text: &[u8], config: &super::DsvConfig) -> super::DsvIndex {
    // Check for BMI2 + AVX2 (fastest path)
    if is_x86_feature_detected!("bmi2") && is_x86_feature_detected!("avx2") {
        return bmi2::build_index_simd(text, config);
    }

    // Fall back to AVX2 with prefix_xor
    if is_x86_feature_detected!("avx2") {
        return avx2::build_index_simd(text, config);
    }

    // Fall back to SSE2
    sse2::build_index_simd(text, config)
}

// Without std feature, default to SSE2 (can't do runtime detection)
#[cfg(all(target_arch = "x86_64", not(any(test, feature = "std"))))]
pub use sse2::build_index_simd;

// ============================================================================
// Fallback for other platforms
// ============================================================================

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub fn build_index_simd(text: &[u8], config: &super::DsvConfig) -> super::DsvIndex {
    super::parser::build_index(text, config)
}
