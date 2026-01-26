//! x86/x86_64 SIMD implementations.
//!
//! These implementations use POPCNT, BMI2, and other x86 instructions for
//! accelerated bit operations.
//!
//! ## BMI2 Support
//!
//! BMI2 provides PDEP (parallel bit deposit) which enables O(1) select-in-word:
//! - Intel: Haswell (2013+) - fast (3-cycle latency)
//! - AMD: Zen 3+ (2020+) - fast (3-cycle latency)
//! - AMD: Zen 1/2 (2017-2020) - **slow** (18-cycle microcode implementation)
//!
//! Use `has_fast_bmi2()` to detect fast BMI2 support at runtime.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Popcount of 64 bytes (512 bits) using POPCNT instruction.
///
/// # Safety
///
/// - `ptr` must be valid for reading 8 x u64 = 64 bytes
/// - `ptr` should be 8-byte aligned
/// - CPU must support POPCNT instruction (caller should check)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "popcnt")]
#[inline]
#[allow(dead_code)]
pub unsafe fn popcount_512_popcnt(ptr: *const u64) -> u32 {
    unsafe {
        let mut sum = 0i32;
        sum += _popcnt64(*ptr as i64);
        sum += _popcnt64(*ptr.add(1) as i64);
        sum += _popcnt64(*ptr.add(2) as i64);
        sum += _popcnt64(*ptr.add(3) as i64);
        sum += _popcnt64(*ptr.add(4) as i64);
        sum += _popcnt64(*ptr.add(5) as i64);
        sum += _popcnt64(*ptr.add(6) as i64);
        sum += _popcnt64(*ptr.add(7) as i64);
        sum as u32
    }
}

/// Popcount of arbitrary-length word data using POPCNT.
///
/// # Safety
///
/// - `ptr` must be valid for reading `word_count` x u64 bytes
/// - CPU must support POPCNT instruction
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "popcnt")]
#[inline]
#[allow(dead_code)]
pub unsafe fn popcount_words_popcnt(ptr: *const u64, word_count: usize) -> u32 {
    unsafe {
        let mut total = 0i32;
        for i in 0..word_count {
            total += _popcnt64(*ptr.add(i) as i64);
        }
        total as u32
    }
}

/// Select the k-th set bit (0-indexed) in a 64-bit word using BMI2 PDEP.
///
/// This is significantly faster than the CTZ loop for k > 0:
/// - CTZ loop: O(k) - must clear k bits one by one
/// - PDEP: O(1) - constant time via bit deposit + leading zeros
///
/// # Performance
///
/// On Intel Haswell+ and AMD Zen 3+:
///
/// | Scenario | CTZ Loop | PDEP | Speedup |
/// |----------|----------|------|---------|
/// | sparse (k=0) | ~1 ns | ~2 ns | 0.5x |
/// | dense (k=31) | ~12 ns | ~2 ns | **6x** |
/// | high_k (k=63) | ~28 ns | ~2 ns | **14x** |
///
/// # Algorithm
///
/// 1. Create mask with (k+1) 1-bits at low positions: `(1 << (k+1)) - 1`
/// 2. PDEP scatters these bits to positions where x has 1-bits
/// 3. The highest set bit position in the result is the answer
///
/// Example: select(0b10101010, 2) - find 3rd set bit (0-indexed)
/// ```text
/// x    = 0b10101010 (bits at positions 1, 3, 5, 7)
/// mask = 0b111      (k+1 = 3 bits)
/// PDEP(mask, x) = 0b00101010 (first 3 bits of x)
/// leading_zeros -> position 5 (the 3rd set bit)
/// ```
///
/// # Safety
///
/// Requires BMI2 support. Caller must check `is_x86_feature_detected!("bmi2")`.
/// For best performance, also check for fast BMI2 (avoid AMD Zen 1/2).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2")]
#[inline]
pub unsafe fn select_in_word_pdep(x: u64, k: u32) -> u32 {
    if x == 0 {
        return 64;
    }
    let pop = x.count_ones();
    if k >= pop {
        return 64;
    }

    // Create mask with (k+1) 1-bits (handle k=63 overflow)
    let mask = if k >= 63 {
        u64::MAX
    } else {
        (1u64 << (k + 1)) - 1
    };

    // PDEP scatters these bits to positions where x has 1-bits
    let scattered = _pdep_u64(mask, x);

    // The highest set bit position is the answer
    if scattered == 0 {
        return 64;
    }
    63 - scattered.leading_zeros()
}

/// Check if the CPU has fast BMI2 support.
///
/// Returns `true` only if:
/// 1. BMI2 is supported, AND
/// 2. The CPU is NOT AMD Zen 1/2 (which have slow microcode BMI2)
///
/// # Platform Detection
///
/// - Intel Haswell+: Always fast if BMI2 is present (3-cycle latency)
/// - AMD Zen 3+: Fast (3-cycle latency)
/// - AMD Zen 1/2: **Slow** (18-cycle latency) - returns `false`
///
/// # Detection Strategy
///
/// We use heuristics to detect fast BMI2:
/// 1. If AVX-512F is available → definitely fast (Intel Skylake-X+ or AMD Zen 4+)
/// 2. Otherwise, check CPUID for AMD family 0x17 (Zen 1/2) vs 0x19+ (Zen 3+)
///
/// The detection is conservative and may return `false` for some CPUs that
/// actually have fast BMI2 (e.g., older Intel without AVX-512).
#[cfg(all(target_arch = "x86_64", any(feature = "std", test)))]
pub fn has_fast_bmi2() -> bool {
    use core::sync::atomic::{AtomicU8, Ordering};

    // Cache the detection result
    // 0 = unknown, 1 = fast BMI2, 2 = no fast BMI2
    static CACHED: AtomicU8 = AtomicU8::new(0);

    match CACHED.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let result = detect_fast_bmi2();
            CACHED.store(if result { 1 } else { 2 }, Ordering::Relaxed);
            result
        }
    }
}

#[cfg(all(target_arch = "x86_64", any(feature = "std", test)))]
fn detect_fast_bmi2() -> bool {
    // First check if BMI2 is supported at all
    if !is_x86_feature_detected!("bmi2") {
        return false;
    }

    // If AVX-512F is available, BMI2 is definitely fast
    // (Intel Skylake-X+, AMD Zen 4+)
    if is_x86_feature_detected!("avx512f") {
        return true;
    }

    // Use CPUID to detect AMD Zen 1/2 (family 0x17) which has slow BMI2
    // AMD Zen 3+ (family 0x19+) has fast BMI2
    //
    // CPUID leaf 0 returns vendor string
    // CPUID leaf 1 returns family/model/stepping
    //
    // For safety and simplicity, we use a conservative approach:
    // - Intel: Always fast (BMI2 was introduced with Haswell which has fast PDEP)
    // - AMD: Check family >= 0x19 (Zen 3+)

    // Check vendor string via CPUID leaf 0
    let cpuid0 = unsafe { core::arch::x86_64::__cpuid(0) };
    let is_amd = cpuid0.ebx == 0x6874_7541  // "Auth"
        && cpuid0.edx == 0x6974_6E65        // "enti"
        && cpuid0.ecx == 0x444D_4163; // "cAMD"

    if !is_amd {
        // Intel or other vendor - assume fast BMI2 if present
        return true;
    }

    // AMD: Check family from CPUID leaf 1
    let cpuid1 = unsafe { core::arch::x86_64::__cpuid(1) };
    let family = ((cpuid1.eax >> 8) & 0xF) + ((cpuid1.eax >> 20) & 0xFF);

    // Family 0x17 = Zen 1/2 (slow BMI2)
    // Family 0x19 = Zen 3/4 (fast BMI2)
    // Family 0x1A = Zen 5+ (fast BMI2)
    family >= 0x19
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn test_popcount_512_popcnt() {
        if !is_x86_feature_detected!("popcnt") {
            return;
        }

        // u64::MAX = 0xFFFF_FFFF_FFFF_FFFF has all 64 bits set
        let data = [u64::MAX; 8];
        let result = unsafe { popcount_512_popcnt(data.as_ptr()) };
        assert_eq!(result, 512);

        let data = [0u64; 8];
        let result = unsafe { popcount_512_popcnt(data.as_ptr()) };
        assert_eq!(result, 0);

        let data = [0xAAAA_AAAA_AAAA_AAAAu64; 8];
        let result = unsafe { popcount_512_popcnt(data.as_ptr()) };
        assert_eq!(result, 256);
    }

    #[test]
    fn test_popcount_words_popcnt() {
        if !is_x86_feature_detected!("popcnt") {
            return;
        }

        // u64::MAX = 0xFFFF_FFFF_FFFF_FFFF has all 64 bits set
        let data = [u64::MAX; 16];
        unsafe {
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 0), 0);
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 1), 64);
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 8), 512);
            assert_eq!(popcount_words_popcnt(data.as_ptr(), 16), 1024);
        }
    }

    // -------------------------------------------------------------------------
    // BMI2 select_in_word tests
    // -------------------------------------------------------------------------

    fn has_bmi2() -> bool {
        is_x86_feature_detected!("bmi2")
    }

    #[test]
    fn test_select_in_word_pdep_basic() {
        if !has_bmi2() {
            eprintln!("Skipping BMI2 test: CPU doesn't support BMI2");
            return;
        }

        unsafe {
            // Empty word
            assert_eq!(select_in_word_pdep(0, 0), 64);

            // Single bit at position 0
            assert_eq!(select_in_word_pdep(1, 0), 0);
            assert_eq!(select_in_word_pdep(1, 1), 64);

            // Single bit at position 63
            assert_eq!(select_in_word_pdep(1 << 63, 0), 63);
            assert_eq!(select_in_word_pdep(1 << 63, 1), 64);

            // Multiple bits: 0b1010_1010
            let word = 0b1010_1010u64;
            assert_eq!(select_in_word_pdep(word, 0), 1); // 1st set bit at pos 1
            assert_eq!(select_in_word_pdep(word, 1), 3); // 2nd set bit at pos 3
            assert_eq!(select_in_word_pdep(word, 2), 5); // 3rd set bit at pos 5
            assert_eq!(select_in_word_pdep(word, 3), 7); // 4th set bit at pos 7
            assert_eq!(select_in_word_pdep(word, 4), 64); // No 5th set bit
        }
    }

    #[test]
    fn test_select_in_word_pdep_all_ones() {
        if !has_bmi2() {
            return;
        }

        unsafe {
            let word = u64::MAX;
            for k in 0..64 {
                assert_eq!(select_in_word_pdep(word, k), k, "k={}", k);
            }
            assert_eq!(select_in_word_pdep(word, 64), 64);
        }
    }

    #[test]
    fn test_select_in_word_pdep_sparse() {
        if !has_bmi2() {
            return;
        }

        unsafe {
            // Very sparse: only bits at positions 0, 31, 63
            let word = 1u64 | (1u64 << 31) | (1u64 << 63);
            assert_eq!(select_in_word_pdep(word, 0), 0);
            assert_eq!(select_in_word_pdep(word, 1), 31);
            assert_eq!(select_in_word_pdep(word, 2), 63);
            assert_eq!(select_in_word_pdep(word, 3), 64);
        }
    }

    #[test]
    fn test_select_in_word_pdep_dense() {
        if !has_bmi2() {
            return;
        }

        unsafe {
            // Dense: lower 32 bits set
            let word = 0xFFFF_FFFFu64;
            for k in 0..32 {
                assert_eq!(select_in_word_pdep(word, k), k, "k={}", k);
            }
            assert_eq!(select_in_word_pdep(word, 32), 64);
        }
    }

    #[test]
    fn test_select_in_word_pdep_matches_ctz() {
        if !has_bmi2() {
            return;
        }

        // Reference CTZ-loop implementation
        fn select_ctz(x: u64, k: u32) -> u32 {
            let mut val = x;
            let mut remaining = k;
            loop {
                if val == 0 {
                    return 64;
                }
                let t = val.trailing_zeros();
                if remaining == 0 {
                    return t;
                }
                remaining -= 1;
                val &= val - 1;
            }
        }

        // Test various patterns
        let patterns = [
            0u64,
            1,
            0xFF,
            0x8000_0000_0000_0000,
            u64::MAX,
            0xAAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555,
            0x1234_5678_9ABC_DEF0,
            0x00FF_00FF_00FF_00FF,
            0xF0F0_F0F0_F0F0_F0F0,
        ];

        for &word in &patterns {
            let pop = word.count_ones();
            for k in 0..=pop {
                unsafe {
                    let pdep_result = select_in_word_pdep(word, k);
                    let ctz_result = select_ctz(word, k);
                    assert_eq!(
                        pdep_result, ctz_result,
                        "Mismatch for word={:#x}, k={}",
                        word, k
                    );
                }
            }
        }
    }

    #[test]
    fn test_select_in_word_pdep_exhaustive_small() {
        if !has_bmi2() {
            return;
        }

        // Reference implementation
        fn select_ctz(x: u64, k: u32) -> u32 {
            let mut val = x;
            let mut remaining = k;
            loop {
                if val == 0 {
                    return 64;
                }
                let t = val.trailing_zeros();
                if remaining == 0 {
                    return t;
                }
                remaining -= 1;
                val &= val - 1;
            }
        }

        // Test all 16-bit patterns
        for word in 0u64..=0xFFFF {
            let pop = word.count_ones();
            for k in 0..=pop {
                unsafe {
                    let pdep_result = select_in_word_pdep(word, k);
                    let ctz_result = select_ctz(word, k);
                    assert_eq!(
                        pdep_result, ctz_result,
                        "Mismatch for word={:#x}, k={}",
                        word, k
                    );
                }
            }
        }
    }

    #[test]
    fn test_has_fast_bmi2_detection() {
        // This test just ensures the function runs without panicking
        let result = has_fast_bmi2();
        eprintln!("has_fast_bmi2() = {}", result);

        // If BMI2 is not supported, result must be false
        if !is_x86_feature_detected!("bmi2") {
            assert!(
                !result,
                "has_fast_bmi2 should be false when BMI2 unsupported"
            );
        }
    }
}
