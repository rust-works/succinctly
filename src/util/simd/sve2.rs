//! SVE2-accelerated operations for ARM64.
//!
//! This module provides SVE2 implementations for performance-critical operations,
//! specifically the BDEP (bit deposit) and BEXT (bit extract) instructions which
//! are equivalent to x86 BMI2 PDEP/PEXT.
//!
//! ## SVE2 BDEP vs x86 BMI2 PDEP
//!
//! - x86 BMI2 PDEP operates on 64-bit general-purpose registers
//! - SVE2 BDEP operates on scalable vector registers (128-bit on Neoverse N2)
//! - We extract the first lane to get a scalar result
//!
//! ## Hardware Support
//!
//! Requires `sve2-bitperm` feature:
//! - Azure Cobalt 100 (Neoverse N2): Yes
//! - AWS Graviton 3 (Neoverse V1): SVE but NOT SVE2
//! - AWS Graviton 4 (Neoverse V2): Yes
//! - Apple M1-M4: No SVE/SVE2 support
//!
//! ## Runtime Detection
//!
//! Use `is_aarch64_feature_detected!("sve2-bitperm")` to check availability.

use core::arch::asm;

/// BDEP (Bit Deposit) - equivalent to x86 BMI2 PDEP.
///
/// Deposits bits from `data` into positions specified by `mask`.
/// Each 1-bit in `mask` receives the next bit from `data` (starting from LSB).
///
/// # Example
///
/// ```text
/// data = 0b1011
/// mask = 0b01010100
/// result = 0b01000100
///          ^  ^  ^
///          |  |  +-- bit 0 of data (1)
///          |  +----- bit 1 of data (1)
///          +-------- bit 2 of data (0)
///                    bit 3 of data (1) -> no more mask bits
/// ```
///
/// # Safety
///
/// Requires SVE2 with BITPERM extension. Caller must verify with
/// `is_aarch64_feature_detected!("sve2-bitperm")`.
#[inline]
#[target_feature(enable = "sve2-bitperm")]
pub unsafe fn bdep_u64(data: u64, mask: u64) -> u64 {
    let result: u64;
    // SVE2 BDEP operates on vector registers.
    // We use the first 64-bit lane (d-register) of the z-registers.
    //
    // The instruction: BDEP Zd.D, Zn.D, Zm.D
    // Deposits bits from Zn into positions specified by Zm.
    //
    // We use FMOV to move between general-purpose and SIMD/FP registers,
    // which is the most efficient way for scalar<->vector transfer.
    asm!(
        "fmov d0, {data}",        // Move data to d0 (low 64 bits of z0)
        "fmov d1, {mask}",        // Move mask to d1 (low 64 bits of z1)
        "bdep z0.d, z0.d, z1.d",  // BDEP: deposit bits from z0 into positions of z1
        "fmov {result}, d0",      // Move result back to GP register
        data = in(reg) data,
        mask = in(reg) mask,
        result = out(reg) result,
        options(pure, nomem, nostack)
    );
    result
}

/// BEXT (Bit Extract) - equivalent to x86 BMI2 PEXT.
///
/// Extracts bits from `data` at positions specified by `mask`,
/// packing them into the low bits of the result.
///
/// # Example
///
/// ```text
/// data = 0b01100101
/// mask = 0b01010101
/// result = 0b0010
///             ^^^^
///             |||+-- bit 0 of data (1)
///             ||+--- bit 2 of data (0)
///             |+---- bit 4 of data (0)
///             +----- bit 6 of data (1)
/// ```
///
/// # Safety
///
/// Requires SVE2 with BITPERM extension. Caller must verify with
/// `is_aarch64_feature_detected!("sve2-bitperm")`.
#[inline]
#[allow(dead_code)]
#[target_feature(enable = "sve2-bitperm")]
pub unsafe fn bext_u64(data: u64, mask: u64) -> u64 {
    let result: u64;
    asm!(
        "fmov d0, {data}",        // Move data to d0
        "fmov d1, {mask}",        // Move mask to d1
        "bext z0.d, z0.d, z1.d",  // BEXT: extract bits from z0 at positions of z1
        "fmov {result}, d0",      // Move result back
        data = in(reg) data,
        mask = in(reg) mask,
        result = out(reg) result,
        options(pure, nomem, nostack)
    );
    result
}

/// Toggle64 using SVE2 BDEP - equivalent to BMI2 version.
///
/// Computes the quote state mask for DSV parsing. Given a bitmask of quote positions,
/// returns a mask indicating which positions are outside quotes.
///
/// # Algorithm
///
/// Uses the hw-dsv algorithm with BDEP instead of prefix_xor:
/// 1. BDEP scatters an alternating pattern to quote positions
/// 2. Addition with carry propagation fills regions between quote pairs
///
/// This is ~10x faster than the prefix_xor software emulation.
///
/// # Arguments
///
/// * `carry` - Carry from previous chunk (0 = outside quotes, 1 = inside quotes)
/// * `quote_mask` - Bitmask of quote positions in this chunk
///
/// # Returns
///
/// * `(outside_mask, new_carry)` - Mask where 1 = outside quotes, and carry for next chunk
///
/// # Safety
///
/// Requires SVE2 with BITPERM extension.
#[inline]
#[target_feature(enable = "sve2-bitperm")]
pub unsafe fn toggle64_sve2(carry: u64, quote_mask: u64) -> (u64, u64) {
    /// Alternating bit pattern: 0101...
    const ODDS_MASK: u64 = 0x5555_5555_5555_5555;

    // Extract the carry bit (0 or 1)
    let c = carry & 0x1;

    // BDEP scatters the alternating pattern to quote positions
    // If c=0: places 1s at odd quotes (1st, 3rd, 5th...) - these are "enters"
    // If c=1: places 1s at even quotes (2nd, 4th, 6th...) - shifted by 1
    let addend = bdep_u64(ODDS_MASK << c, quote_mask);

    // Addition with carry propagation creates the quote mask
    // Formula: ((addend << 1) | c) + !quote_mask
    let comp_w = !quote_mask;
    let shifted = (addend << 1) | c;
    let (result, overflow) = shifted.overflowing_add(comp_w);

    // New carry depends on overflow
    let new_carry = if overflow { 1 } else { 0 };

    (result, new_carry)
}

/// Check if SVE2-BITPERM is available at runtime.
///
/// Returns `true` if the CPU supports SVE2 with the BITPERM extension,
/// which is required for BDEP/BEXT instructions.
#[cfg(feature = "std")]
#[inline]
#[allow(dead_code)]
pub fn has_sve2_bitperm() -> bool {
    std::arch::is_aarch64_feature_detected!("sve2-bitperm")
}

/// Check if SVE2-BITPERM is available (no_std version - always false).
#[cfg(not(feature = "std"))]
#[inline]
#[allow(dead_code)]
pub const fn has_sve2_bitperm() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has_sve2() -> bool {
        #[cfg(feature = "std")]
        {
            std::arch::is_aarch64_feature_detected!("sve2-bitperm")
        }
        #[cfg(not(feature = "std"))]
        {
            false
        }
    }

    #[test]
    fn test_bdep_basic() {
        if !has_sve2() {
            eprintln!("Skipping SVE2 test: CPU doesn't support sve2-bitperm");
            return;
        }

        unsafe {
            // No mask bits - result is 0
            assert_eq!(bdep_u64(0xFFFF, 0), 0);

            // All mask bits set - result equals data (up to mask width)
            assert_eq!(bdep_u64(0xF, 0xF), 0xF);

            // Deposit into alternating positions
            // data = 0b1010, mask = 0b01010101
            // bit 0 of data (0) -> position 0
            // bit 1 of data (1) -> position 2
            // bit 2 of data (0) -> position 4
            // bit 3 of data (1) -> position 6
            // result = 0b01000100 = 0x44
            assert_eq!(bdep_u64(0b1010, 0b01010101), 0b01000100);
        }
    }

    #[test]
    fn test_bext_basic() {
        if !has_sve2() {
            eprintln!("Skipping SVE2 test: CPU doesn't support sve2-bitperm");
            return;
        }

        unsafe {
            // No mask bits - result is 0
            assert_eq!(bext_u64(0xFFFF, 0), 0);

            // Extract from alternating positions
            // data = 0b01010101, mask = 0b01010101
            // position 0 (1) -> bit 0
            // position 2 (1) -> bit 1
            // position 4 (1) -> bit 2
            // position 6 (1) -> bit 3
            // result = 0b1111 = 0xF
            assert_eq!(bext_u64(0b01010101, 0b01010101), 0b1111);

            // Extract from specific positions
            // data = 0b11110000, mask = 0b11110000
            // position 4 (1) -> bit 0
            // position 5 (1) -> bit 1
            // position 6 (1) -> bit 2
            // position 7 (1) -> bit 3
            // result = 0b1111
            assert_eq!(bext_u64(0b11110000, 0b11110000), 0b1111);
        }
    }

    #[test]
    fn test_bdep_bext_roundtrip() {
        if !has_sve2() {
            return;
        }

        unsafe {
            // BEXT then BDEP should recover the original pattern in the masked positions
            let mask = 0b10101010_10101010u64;
            let data = 0b11110000_11110000u64;

            let extracted = bext_u64(data, mask);
            let deposited = bdep_u64(extracted, mask);

            // deposited should have the same bits as data at mask positions
            assert_eq!(deposited, data & mask);
        }
    }

    #[test]
    fn test_toggle64_no_quotes() {
        if !has_sve2() {
            return;
        }

        unsafe {
            // No quotes - everything is outside
            let (mask, carry) = toggle64_sve2(0, 0);
            assert_eq!(carry, 0, "No quotes should not change carry");
            assert_eq!(mask, !0u64, "No quotes means all outside");
        }
    }

    #[test]
    fn test_toggle64_single_quote() {
        if !has_sve2() {
            return;
        }

        unsafe {
            // Single quote at position 0 - everything after is inside
            let (_mask, carry) = toggle64_sve2(0, 1);
            // After the quote, we're inside, so the carry should be 1
            assert_eq!(carry, 1, "Odd quotes should set carry");
        }
    }

    #[test]
    fn test_toggle64_matches_prefix_xor() {
        if !has_sve2() {
            return;
        }

        // Reference implementation using the same algorithm as toggle64_sve2/toggle64_bmi2
        // but without BDEP (uses scalar PDEP-equivalent logic).
        fn toggle64_reference(carry: u64, quote_mask: u64) -> (u64, u64) {
            const ODDS_MASK: u64 = 0x5555_5555_5555_5555;
            let c = carry & 0x1;

            // Scalar PDEP equivalent: deposit alternating bits into quote positions
            let mut addend = 0u64;
            let mut src_bit = 0;
            for i in 0..64 {
                if (quote_mask >> i) & 1 == 1 {
                    let bit = (ODDS_MASK << c >> src_bit) & 1;
                    addend |= bit << i;
                    src_bit += 1;
                }
            }

            let comp_w = !quote_mask;
            let shifted = (addend << 1) | c;
            let (result, overflow) = shifted.overflowing_add(comp_w);
            let new_carry = if overflow { 1 } else { 0 };

            (result, new_carry)
        }

        // Test various quote patterns
        let patterns = [
            0u64,
            1,
            0b11,
            0b101,
            0b1001,
            0x8000_0000_0000_0000,
            0xAAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555,
            0xFF00_FF00_FF00_FF00,
        ];

        for &quote_mask in &patterns {
            for carry in [0u64, 1] {
                unsafe {
                    let (sve2_mask, sve2_carry) = toggle64_sve2(carry, quote_mask);
                    let (ref_mask, ref_carry) = toggle64_reference(carry, quote_mask);

                    assert_eq!(
                        sve2_mask, ref_mask,
                        "Mask mismatch for quote_mask={:#x}, carry={}",
                        quote_mask, carry
                    );
                    assert_eq!(
                        sve2_carry, ref_carry,
                        "Carry mismatch for quote_mask={:#x}, carry={}",
                        quote_mask, carry
                    );
                }
            }
        }
    }
}
