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
    if detect_sve2() {
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
    if detect_bmi2() && detect_avx2() {
        return bmi2::build_index_simd(text, config);
    }

    // Fall back to AVX2 with prefix_xor
    if detect_avx2() {
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

// ============================================================================
// Feature detection (indirection point for the dispatcher fallback-arm tests)
// ============================================================================
//
// The dispatchers above call these instead of `is_*_feature_detected!` directly.
// In production each is a straight CPU probe. Under `cfg(test)` each also
// consults a thread-local mask (see `test_dispatch`) so a test can drive the
// dispatcher through its lower arms without depending on the host CPU — the arm
// selection in `mod.rs` is otherwise never exercised end-to-end, because every
// runner reports the fastest backend and the fallbacks never run (#283).

#[cfg(all(target_arch = "x86_64", feature = "std", not(test)))]
#[inline]
fn detect_bmi2() -> bool {
    is_x86_feature_detected!("bmi2")
}

#[cfg(all(target_arch = "x86_64", feature = "std", not(test)))]
#[inline]
fn detect_avx2() -> bool {
    is_x86_feature_detected!("avx2")
}

#[cfg(all(target_arch = "x86_64", test))]
#[inline]
fn detect_bmi2() -> bool {
    is_x86_feature_detected!("bmi2") && !test_dispatch::is_disabled(test_dispatch::BMI2)
}

#[cfg(all(target_arch = "x86_64", test))]
#[inline]
fn detect_avx2() -> bool {
    is_x86_feature_detected!("avx2") && !test_dispatch::is_disabled(test_dispatch::AVX2)
}

#[cfg(all(target_arch = "aarch64", feature = "std", not(test)))]
#[inline]
fn detect_sve2() -> bool {
    is_aarch64_feature_detected!("sve2-bitperm")
}

#[cfg(all(target_arch = "aarch64", feature = "std", test))]
#[inline]
fn detect_sve2() -> bool {
    is_aarch64_feature_detected!("sve2-bitperm") && !test_dispatch::is_disabled(test_dispatch::SVE2)
}

// ============================================================================
// Test-only hook to exercise the dispatcher fallback arms (#283)
// ============================================================================

/// Test-only hook that forces the runtime dispatcher down its fallback arms.
///
/// The dispatcher always selects the fastest backend the host CPU supports, so
/// on any given runner its lower arms never execute (#283). This hook lets a
/// test mask off detected features for the current thread, so the dispatcher
/// falls through to a lower backend and the arm-selection logic in `mod.rs` is
/// actually covered.
///
/// Invariant: the mask can only *remove* a feature the host has — never add one
/// it lacks. So the dispatcher can never route to a `#[target_feature]` backend
/// on a CPU without that feature (which would be undefined behavior); a backend
/// is reachable here only when the host genuinely supports it.
#[cfg(all(
    test,
    any(target_arch = "x86_64", all(target_arch = "aarch64", feature = "std"))
))]
mod test_dispatch {
    use core::cell::Cell;

    #[cfg(target_arch = "x86_64")]
    pub(super) const BMI2: u8 = 1 << 0;
    #[cfg(target_arch = "x86_64")]
    pub(super) const AVX2: u8 = 1 << 1;
    #[cfg(target_arch = "aarch64")]
    pub(super) const SVE2: u8 = 1 << 2;

    std::thread_local! {
        /// Bitmask of features masked off for the current thread.
        static DISABLED: Cell<u8> = const { Cell::new(0) };
    }

    /// Whether `flag` is currently masked off on this thread.
    pub(super) fn is_disabled(flag: u8) -> bool {
        DISABLED.with(|d| d.get() & flag != 0)
    }

    /// RAII guard that masks off `flags` until dropped, restoring the previous
    /// mask. Thread-local and scoped, so parallel tests don't interfere.
    #[must_use = "the mask is only active while the guard is alive"]
    pub(super) struct MaskGuard {
        prev: u8,
    }

    /// Mask off `flags` for the current thread until the returned guard drops.
    pub(super) fn mask(flags: u8) -> MaskGuard {
        DISABLED.with(|d| {
            let prev = d.get();
            d.set(prev | flags);
            MaskGuard { prev }
        })
    }

    impl Drop for MaskGuard {
        fn drop(&mut self) {
            DISABLED.with(|d| d.set(self.prev));
        }
    }
}

#[cfg(all(
    test,
    any(target_arch = "x86_64", all(target_arch = "aarch64", feature = "std"))
))]
mod tests {
    use super::build_index_simd;
    use crate::dsv::{build_index_scalar, DsvConfig, DsvIndex};

    /// Cumulative marker/newline rank at every position — a padding-insensitive
    /// signature of an index (same idea as the cross-backend differential
    /// tests): two indices share a signature iff they mark the same bytes.
    fn index_sig(idx: &DsvIndex, len: usize) -> (Vec<usize>, Vec<usize>) {
        (
            (0..=len).map(|i| idx.markers_rank1(i)).collect(),
            (0..=len).map(|i| idx.newlines_rank1(i)).collect(),
        )
    }

    /// Representative CSVs: plain, a delimiter and a newline inside quotes, a
    /// quoted span crossing a 64-byte chunk boundary, and empty.
    fn cases() -> Vec<(&'static str, Vec<u8>)> {
        let mut spanning = Vec::new();
        spanning.extend_from_slice(b"aaa,\"");
        spanning.extend_from_slice(&[b'x'; 80]); // quoted run longer than one 64-byte chunk
        spanning.extend_from_slice(b",\n"); // delimiter + newline swallowed by the quotes
        spanning.extend_from_slice(b"\"\nbbb,ccc\n");
        vec![
            ("plain", b"a,b,c\n1,2,3\n".to_vec()),
            ("quoted_delimiter", b"\"a,b\",c\n\"d,e\",f\n".to_vec()),
            ("quoted_newline", b"\"line1\nline2\",x\ny,z\n".to_vec()),
            ("chunk_spanning_quote", spanning),
            ("empty", Vec::new()),
        ]
    }

    /// Drive the x86_64 dispatcher through every arm it can reach on this host
    /// and assert each agrees with the scalar reference. On a modern CI runner
    /// (BMI2 + AVX2) this covers all three arms of `build_index_simd`; the
    /// masking only ever downgrades, so it never calls an unsupported backend.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn dispatch_covers_x86_arms() {
        use super::test_dispatch::{mask, AVX2, BMI2};

        let has_bmi2 = std::arch::is_x86_feature_detected!("bmi2");
        let has_avx2 = std::arch::is_x86_feature_detected!("avx2");
        let config = DsvConfig::default();

        for (label, text) in cases() {
            let want = index_sig(&build_index_scalar(&text, &config), text.len());

            // Top arm: BMI2 + AVX2. Reachable only when the host has both.
            if has_bmi2 && has_avx2 {
                let got = build_index_simd(&text, &config);
                assert_eq!(index_sig(&got, text.len()), want, "bmi2 arm / {label}");
            }

            // Middle arm: AVX2 prefix_xor. Mask BMI2 off; needs host AVX2.
            if has_avx2 {
                let _guard = mask(BMI2);
                let got = build_index_simd(&text, &config);
                assert_eq!(index_sig(&got, text.len()), want, "avx2 arm / {label}");
            }

            // Bottom arm: SSE2 baseline. Mask BMI2 + AVX2 off; SSE2 is universal.
            {
                let _guard = mask(BMI2 | AVX2);
                let got = build_index_simd(&text, &config);
                assert_eq!(index_sig(&got, text.len()), want, "sse2 arm / {label}");
            }
        }

        if !has_bmi2 {
            eprintln!("SKIPPED dsv dispatch [bmi2 arm]: bmi2 not detected (#283, see #193)");
        }
        if !has_avx2 {
            eprintln!("SKIPPED dsv dispatch [avx2 arm]: avx2 not detected (#283, see #193)");
        }
    }

    /// Drive the aarch64 dispatcher through every arm it can reach on this host
    /// and assert each agrees with the scalar reference. NEON is mandatory so
    /// its arm always runs; the SVE2 arm runs only on an `sve2-bitperm` host.
    #[cfg(all(target_arch = "aarch64", feature = "std"))]
    #[test]
    fn dispatch_covers_aarch64_arms() {
        use super::test_dispatch::{mask, SVE2};

        let has_sve2 = std::arch::is_aarch64_feature_detected!("sve2-bitperm");
        let config = DsvConfig::default();

        for (label, text) in cases() {
            let want = index_sig(&build_index_scalar(&text, &config), text.len());

            // Top arm: SVE2-BITPERM. Reachable only on a host that has it.
            if has_sve2 {
                let got = build_index_simd(&text, &config);
                assert_eq!(index_sig(&got, text.len()), want, "sve2 arm / {label}");
            }

            // Fallback arm: NEON (mandatory on aarch64). Mask SVE2 off.
            {
                let _guard = mask(SVE2);
                let got = build_index_simd(&text, &config);
                assert_eq!(index_sig(&got, text.len()), want, "neon arm / {label}");
            }
        }

        if !has_sve2 {
            eprintln!(
                "SKIPPED dsv dispatch [sve2 arm]: sve2-bitperm not detected (#283, see #194)"
            );
        }
    }
}
