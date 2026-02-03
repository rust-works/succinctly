//! SIMD-accelerated JSON semi-indexing.
//!
//! This module provides vectorized implementations of JSON semi-indexing
//! that process multiple bytes at once using SIMD instructions.
//!
//! The vectorized character classification techniques are influenced by:
//! - Langdale & Lemire, ["Parsing Gigabytes of JSON per Second"](https://arxiv.org/abs/1902.08318) (2019)
//!
//! ## x86_64 Instruction Set Levels
//!
//! - **SSE2** (baseline): 16 bytes/iteration, universal availability
//! - **SSE4.2** (enhanced): 16 bytes/iteration with PCMPISTRI, ~90% availability (2008+)
//! - **AVX2** (optimal): 32 bytes/iteration, ~95% availability (2013+)
//! - **BMI2** (bit manipulation): PDEP/PEXT for efficient mask operations, ~95% availability (2013+)
//!   - **Note**: AMD Zen 1/2 have slow BMI2 (18 cycle latency), use with caution
//!
//! The implementation uses runtime CPU detection to automatically select
//! the best available instruction set.
//!
//! ## ARM
//!
//! On ARM aarch64:
//! - **NEON** (default): 16 bytes/iteration with nibble lookup tables, mandatory on all aarch64
//! - **SVE2** (experimental): Character comparisons with predication
//!   - AWS Graviton 4 (Neoverse V2): 128-bit SVE2 vectors
//!   - **36% slower than NEON** on 128-bit implementations due to expensive predicate-to-bitmask conversion
//!   - May be faster on wider SVE2 implementations (256-bit, 512-bit)
//!   - Enable with `SUCCINCTLY_SVE2=1` environment variable (not recommended)
//!
//! ## Cursor Algorithms
//!
//! Both simple and standard cursor algorithms are provided:
//! - [`build_semi_index_simple`]: Simple cursor (3-state, marks structural chars)
//! - [`build_semi_index_standard`]: Standard cursor (4-state, marks value starts)

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "aarch64")]
pub mod sve2;

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "x86_64")]
pub mod sse42;

#[cfg(target_arch = "x86_64")]
pub mod avx2;

#[cfg(target_arch = "x86_64")]
pub mod avx2_pfsm;

#[cfg(target_arch = "x86_64")]
pub mod bmi2;

// ============================================================================
// ARM exports with optional runtime dispatch to SVE2 (requires std)
// ============================================================================

// Runtime dispatch when std is available
// Priority: SVE2 > NEON (SVE2 is experimental, may not be faster)
#[cfg(all(target_arch = "aarch64", any(test, feature = "std")))]
pub fn build_semi_index_standard(json: &[u8]) -> crate::json::standard::SemiIndex {
    // SVE2 dispatch disabled by default - enable with SUCCINCTLY_SVE2=1
    // SVE2 on 128-bit implementations may not be faster than NEON
    #[cfg(feature = "std")]
    if std::env::var("SUCCINCTLY_SVE2").is_ok_and(|v| v == "1")
        && std::arch::is_aarch64_feature_detected!("sve2")
    {
        return unsafe { sve2::build_semi_index_standard(json) };
    }
    neon::build_semi_index_standard(json)
}

#[cfg(all(target_arch = "aarch64", any(test, feature = "std")))]
pub fn build_semi_index_simple(json: &[u8]) -> crate::json::simple::SemiIndex {
    #[cfg(feature = "std")]
    if std::env::var("SUCCINCTLY_SVE2").is_ok_and(|v| v == "1")
        && std::arch::is_aarch64_feature_detected!("sve2")
    {
        return unsafe { sve2::build_semi_index_simple(json) };
    }
    neon::build_semi_index_simple(json)
}

// Without std feature, default to NEON (universally available on aarch64)
#[cfg(all(target_arch = "aarch64", not(any(test, feature = "std"))))]
pub use neon::build_semi_index_simple;

#[cfg(all(target_arch = "aarch64", not(any(test, feature = "std"))))]
pub use neon::build_semi_index_standard;

// ============================================================================
// x86_64 exports with optional runtime dispatch (requires std)
// ============================================================================

// Runtime dispatch when std is available (test mode or std feature)
// Priority: AVX2 > SSE4.2 > SSE2
#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub fn build_semi_index_standard(json: &[u8]) -> crate::json::standard::SemiIndex {
    if is_x86_feature_detected!("avx2") {
        avx2::build_semi_index_standard(json)
    } else if is_x86_feature_detected!("sse4.2") {
        sse42::build_semi_index_standard(json)
    } else {
        x86::build_semi_index_standard(json)
    }
}

#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub fn build_semi_index_simple(json: &[u8]) -> crate::json::simple::SemiIndex {
    if is_x86_feature_detected!("avx2") {
        avx2::build_semi_index_simple(json)
    } else if is_x86_feature_detected!("sse4.2") {
        sse42::build_semi_index_simple(json)
    } else {
        x86::build_semi_index_simple(json)
    }
}

// Without std feature, default to SSE2 (universally available on x86_64)
// Users can explicitly use avx2:: or sse42:: modules if they know CPU capabilities
#[cfg(all(target_arch = "x86_64", not(any(test, feature = "std"))))]
pub use x86::build_semi_index_simple;

#[cfg(all(target_arch = "x86_64", not(any(test, feature = "std"))))]
pub use x86::build_semi_index_standard;

// ============================================================================
// Fallback for other platforms
// ============================================================================

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::simple::build_semi_index as build_semi_index_simple;

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::standard::build_semi_index as build_semi_index_standard;
