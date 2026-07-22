#![allow(unsafe_code)] // runtime SIMD feature dispatch
//! SIMD-accelerated operations.
//!
//! This module provides platform-specific SIMD implementations for
//! performance-critical operations like popcount.

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "aarch64")]
pub mod sve2;

#[cfg(target_arch = "x86_64")]
pub mod x86;

/// Popcount of a 512-bit (64-byte) block.
///
/// Uses the best available implementation for the current platform.
#[inline]
#[allow(dead_code)] // STYLE-0005: reference popcount; unused on some targets
pub fn popcount_512(data: &[u8; 64]) -> u32 {
    #[cfg(target_arch = "aarch64")]
    {
        // NEON is always available on aarch64
        unsafe { neon::popcount_512_neon(data.as_ptr()) }
    }

    #[cfg(target_arch = "x86_64")]
    {
        // Use scalar implementation which LLVM optimizes to POPCNT when available.
        // Runtime feature detection (is_x86_feature_detected!) requires std.
        popcount_512_scalar(data)
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        popcount_512_scalar(data)
    }
}

/// Scalar fallback for 512-bit popcount.
#[inline]
#[allow(dead_code)] // STYLE-0005: scalar reference popcount
pub fn popcount_512_scalar(data: &[u8; 64]) -> u32 {
    // Process byte-by-byte to avoid alignment issues
    let mut total = 0u32;
    for byte in data {
        total += byte.count_ones();
    }
    total
}

/// Emit a standardized `SKIPPED` line when a SIMD test bails out because the
/// running CPU lacks the required feature.
///
/// Feature-gated SIMD tests self-skip with `if !detected { return; }`, which
/// otherwise reports as a silent pass — so on an ARM host the x86 BMI2/AVX2/SSE2
/// suites (and, absent emulation, SVE2) read as "passed" without asserting
/// anything. Routing every skip through this helper makes the skips visible and
/// countable (grep the test output for `SKIPPED`), so a fully-skipped SIMD suite
/// no longer looks green. See #191; wired into the x86 BMI2/AVX2/POPCNT sites
/// (#193) and the aarch64 SVE2 sites (#194) via each test module's local
/// feature-guard helper.
///
/// CI additionally pins the expected feature set hard via the
/// `SUCCINCTLY_EXPECT_SIMD` expectation test (`tests/simd_expectation_tests.rs`),
/// so a runner that stops exposing a feature fails the leg instead of skipping.
#[cfg(test)]
pub(crate) fn note_simd_skip(feature: &str) {
    eprintln!("SKIPPED: SIMD test - CPU feature `{feature}` unavailable");
}

/// Note a skip when `available` is false, passing the flag through.
///
/// Feature guards call this as a single always-executed expression, so the
/// skip branch lives here — covered by its own test even on hardware that has
/// every feature — rather than as a dead line in each guard that coverage on
/// fully-featured CI runners can never reach (#193).
#[cfg(test)]
pub(crate) fn note_simd_skip_unless(available: bool, feature: &str) -> bool {
    if !available {
        note_simd_skip(feature);
    }
    available
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_popcount_512_all_zeros() {
        let data = [0u8; 64];
        assert_eq!(popcount_512(&data), 0);
    }

    #[test]
    fn test_note_simd_skip_emits() {
        // Exercise the shared skip-visibility helper directly so it is covered
        // even on arches where no feature-gated SIMD test calls it (see #191).
        note_simd_skip("test-feature");
    }

    #[test]
    fn test_note_simd_skip_unless_both_branches() {
        // Both arms of the shared guard helper, so per-module feature guards
        // stay fully covered even on hardware that has every feature (#193).
        assert!(note_simd_skip_unless(true, "test-feature"));
        assert!(!note_simd_skip_unless(false, "test-feature"));
    }

    #[test]
    fn test_popcount_512_all_ones() {
        let data = [0xFFu8; 64];
        assert_eq!(popcount_512(&data), 512);
    }

    #[test]
    fn test_popcount_512_pattern() {
        let mut data = [0u8; 64];
        // Set alternating bits: 0b10101010 = 4 bits per byte
        data.fill(0xAA);
        assert_eq!(popcount_512(&data), 256);
    }

    #[test]
    fn test_popcount_512_matches_scalar() {
        let test_patterns: &[u8] = &[0x00, 0xFF, 0xAA, 0x55, 0x0F, 0xF0, 0x12, 0x34];
        for &pattern in test_patterns {
            let data = [pattern; 64];
            assert_eq!(
                popcount_512(&data),
                popcount_512_scalar(&data),
                "pattern={pattern:#04x}"
            );
        }
    }
}
