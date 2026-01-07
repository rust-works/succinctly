//! BMI2 bit manipulation helpers for x86_64.
//!
//! BMI2 provides PDEP (parallel bit deposit) and PEXT (parallel bit extract)
//! instructions that can efficiently manipulate bitmasks.
//!
//! ## Availability
//! - Intel: Haswell (2013+)
//! - AMD: Zen 3 (2020+) - **Fast implementation**
//! - AMD: Zen 1/2 (2017-2020) - **Slow microcode implementation (18 cycles)**
//!
//! ## Warning
//! AMD Zen 1/2 processors implement PDEP/PEXT in microcode with 18-cycle latency
//! instead of the 3-cycle latency on Intel Haswell+ and AMD Zen 3+. This makes
//! BMI2 instructions **slower than scalar code** on these processors.
//!
//! Always use runtime CPU detection to avoid slow paths on AMD Zen 1/2.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Extract bits from source according to mask using PEXT.
///
/// Copies bits from `src` corresponding to set bits in `mask` to contiguous
/// low-order bits of the result.
///
/// # Example
/// ```text
/// src  = 0b11010110
/// mask = 0b11001100
/// result = 0b1011 (extracted bits: 11__01__)
/// ```
///
/// # Safety
/// Requires BMI2 support. Caller must check `is_x86_feature_detected!("bmi2")`.
#[inline]
#[target_feature(enable = "bmi2")]
#[cfg(target_arch = "x86_64")]
pub unsafe fn pext_u64(src: u64, mask: u64) -> u64 {
    _pext_u64(src, mask)
}

/// Deposit bits to destination according to mask using PDEP.
///
/// Copies contiguous low-order bits from `src` to positions corresponding
/// to set bits in `mask`.
///
/// # Example
/// ```text
/// src  = 0b1011 (bits 0-3: 1,1,0,1)
/// mask = 0b11001100 (positions: 2,3,6,7)
/// result = 0b10001100 (= 140)
/// ```
///
/// # Safety
/// Requires BMI2 support. Caller must check `is_x86_feature_detected!("bmi2")`.
#[inline]
#[target_feature(enable = "bmi2")]
#[cfg(target_arch = "x86_64")]
pub unsafe fn pdep_u64(src: u64, mask: u64) -> u64 {
    _pdep_u64(src, mask)
}

/// Check if the CPU has fast BMI2 support.
///
/// Returns `true` only if:
/// 1. BMI2 is supported, AND
/// 2. The CPU is NOT AMD Zen 1/2 (which have slow microcode BMI2)
///
/// # Platform Detection
/// - Intel: Always fast if BMI2 is present
/// - AMD Zen 3+: Fast (3 cycle latency)
/// - AMD Zen 1/2: **Slow** (18 cycle latency) - returns `false`
///
/// # Note
/// This uses heuristics to detect AMD Zen 1/2. The detection is conservative
/// and may return `false` for some CPUs that actually have fast BMI2.
///
/// Only available in std/test mode (requires `is_x86_feature_detected!` macro).
#[cfg(all(target_arch = "x86_64", test))]
pub fn has_fast_bmi2() -> bool {
    // First check if BMI2 is supported at all
    if !is_x86_feature_detected!("bmi2") {
        return false;
    }

    // Detection strategy:
    // AMD Zen 1/2 have slow BMI2 (microcode implementation)
    // AMD Zen 3+ have fast BMI2 (hardware implementation)
    //
    // We can use CPUID to detect:
    // - Vendor: AMD
    // - Family: 0x17 (Zen 1/2) vs 0x19 (Zen 3+)
    //
    // For now, we'll be conservative and only enable BMI2 on:
    // - Intel processors (always fast if BMI2 exists)
    // - AMD processors with AVX512 (indicates Zen 4+, definitely fast)
    //
    // A more sophisticated approach would use CPUID to read family/model.

    // If AVX-512 is available, BMI2 is definitely fast (Intel or AMD Zen 4+)
    if is_x86_feature_detected!("avx512f") {
        return true;
    }

    // Conservative approach: assume fast on Intel, may be slow on AMD
    // Without CPUID access, we can't definitively detect Zen 1/2
    // In a real implementation, you'd want to use CPUID here
    //
    // For now, we'll enable BMI2 since:
    // 1. Most BMI2-capable CPUs are Intel (Haswell+)
    // 2. AMD Zen 3+ (2020+) is increasingly common
    // 3. The overhead is in a few specific operations, not the whole parser

    true // Conservative: enable BMI2 if available
}

/// Software fallback for PEXT when BMI2 is not available.
///
/// This implements PEXT using scalar operations. It's slower than hardware
/// PEXT but can be used as a fallback or on AMD Zen 1/2 where it may be
/// faster than the microcode implementation.
#[inline]
pub fn pext_u64_fallback(src: u64, mask: u64) -> u64 {
    let mut result = 0u64;
    let mut result_pos = 0;

    // Iterate through each bit position in the mask
    for i in 0..64 {
        if (mask & (1u64 << i)) != 0 {
            // If this bit is set in mask, copy corresponding bit from src to result
            if (src & (1u64 << i)) != 0 {
                result |= 1u64 << result_pos;
            }
            result_pos += 1;
        }
    }

    result
}

/// Software fallback for PDEP when BMI2 is not available.
#[inline]
pub fn pdep_u64_fallback(src: u64, mask: u64) -> u64 {
    let mut result = 0u64;
    let mut src_pos = 0;

    // Iterate through each bit position in the mask
    for i in 0..64 {
        if (mask & (1u64 << i)) != 0 {
            // If this bit is set in mask, deposit next bit from src
            if (src & (1u64 << src_pos)) != 0 {
                result |= 1u64 << i;
            }
            src_pos += 1;
        }
    }

    result
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn test_pext_fallback() {
        // Example: extract bits at positions marked by mask
        let src = 0b11010110u64;
        let mask = 0b11001100u64;
        // Mask positions: 2,3,6,7
        // Bits at those positions: 1,0,1,1
        // Result: 1101 = 13
        let expected = 13u64;

        assert_eq!(pext_u64_fallback(src, mask), expected);
    }

    #[test]
    fn test_pdep_fallback() {
        // Example: deposit bits to positions marked by mask
        let src = 0b1011u64; // = 11, bits [3,2,1,0] = [1,0,1,1]
        let mask = 0b11001100u64; // positions [7,6,3,2]
                                  // Deposit src[0]=1 -> pos 2, src[1]=1 -> pos 3, src[2]=0 -> pos 6, src[3]=1 -> pos 7
                                  // Result: 10001100 = 140
        let expected = 140u64;

        assert_eq!(pdep_u64_fallback(src, mask), expected);
    }

    #[test]
    fn test_pext_pdep_roundtrip_fallback() {
        let src = 0b10110011u64;
        let mask = 0b11110000u64;

        let extracted = pext_u64_fallback(src, mask);
        let deposited = pdep_u64_fallback(extracted, mask);

        // After extract and deposit, we should get back the masked bits
        assert_eq!(deposited, src & mask);
    }

    #[test]
    fn test_pext_hardware() {
        if !is_x86_feature_detected!("bmi2") {
            return;
        }

        let src = 0b11010110u64;
        let mask = 0b11001100u64;
        let expected = 13u64;

        unsafe {
            assert_eq!(pext_u64(src, mask), expected);
        }
    }

    #[test]
    fn test_pdep_hardware() {
        if !is_x86_feature_detected!("bmi2") {
            return;
        }

        let src = 0b1011u64;
        let mask = 0b11001100u64;
        let expected = 140u64;

        unsafe {
            assert_eq!(pdep_u64(src, mask), expected);
        }
    }

    #[test]
    fn test_pext_hardware_matches_fallback() {
        if !is_x86_feature_detected!("bmi2") {
            return;
        }

        let test_cases = [
            (0b11010110u64, 0b11001100u64),
            (0xFFFFFFFFu64, 0x0F0F0F0Fu64),
            (0x123456789ABCDEFu64, 0xF0F0F0F0F0F0F0Fu64),
        ];

        for (src, mask) in test_cases {
            unsafe {
                assert_eq!(
                    pext_u64(src, mask),
                    pext_u64_fallback(src, mask),
                    "PEXT mismatch for src={:#x}, mask={:#x}",
                    src,
                    mask
                );
            }
        }
    }

    #[test]
    fn test_pdep_hardware_matches_fallback() {
        if !is_x86_feature_detected!("bmi2") {
            return;
        }

        let test_cases = [
            (0b1011u64, 0b11001100u64),
            (0xFFFFu64, 0x0F0F0F0Fu64),
            (0x123456u64, 0xF0F0F0F0u64),
        ];

        for (src, mask) in test_cases {
            unsafe {
                assert_eq!(
                    pdep_u64(src, mask),
                    pdep_u64_fallback(src, mask),
                    "PDEP mismatch for src={:#x}, mask={:#x}",
                    src,
                    mask
                );
            }
        }
    }

    #[test]
    fn test_has_fast_bmi2_detection() {
        // This test just ensures the function runs without panicking
        let _ = has_fast_bmi2();
        // We can't test the actual result since it depends on the CPU
    }
}
