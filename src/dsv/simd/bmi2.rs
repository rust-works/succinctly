#![allow(unsafe_code)] // x86_64 BMI2 (PDEP) SIMD intrinsics
//! BMI2-accelerated DSV indexing for x86_64.
//!
//! Uses BMI2 PDEP instruction for quote state masking, providing ~10x speedup
//! over the portable prefix_xor approach on supported processors.
//!
//! Requires: AVX2 (for character matching) + BMI2 (for PDEP)
//! Supported: Intel Haswell+ (2013), AMD Zen+ (2018)

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use alloc::vec;

use super::super::config::DsvConfig;
use super::super::index::DsvIndex;
use super::super::index_lightweight::DsvIndexLightweight;

use crate::json::BitWriter;

/// Build a DsvIndex using AVX2 + BMI2 acceleration.
///
/// This is the fastest path on x86_64 processors that support both AVX2 and BMI2.
/// Uses PDEP for quote state masking instead of prefix_xor.
#[cfg(target_arch = "x86_64")]
pub fn build_index_simd(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if text.is_empty() {
        return DsvIndex::new_lightweight(DsvIndexLightweight::new(vec![], vec![], 0));
    }

    // SAFETY: Caller verified AVX2 and BMI2 are available via runtime detection
    unsafe { build_index_bmi2(text, config) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2", enable = "bmi2")]
unsafe fn build_index_bmi2(text: &[u8], config: &DsvConfig) -> DsvIndex {
    let num_words = text.len().div_ceil(64);
    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);

    // Track quote state across chunks using carry
    let mut qq_carry: u64 = 0;

    let delimiter = config.delimiter as i8;
    let quote_char = config.quote_char as i8;
    let newline = config.newline as i8;

    let mut offset = 0;

    // Process 64-byte chunks (2x 32-byte AVX2 loads)
    while offset + 64 <= text.len() {
        let (markers_word, newlines_word, new_carry) = unsafe {
            process_chunk_64_bmi2(
                text.as_ptr().add(offset),
                delimiter,
                quote_char,
                newline,
                qq_carry,
            )
        };

        markers_writer.write_bits(markers_word, 64);
        newlines_writer.write_bits(newlines_word, 64);
        qq_carry = new_carry;
        offset += 64;
    }

    // Process remaining bytes
    if offset < text.len() {
        let remaining = text.len() - offset;

        let mut padded = [0u8; 64];
        padded[..remaining].copy_from_slice(&text[offset..]);

        let (mut markers_word, mut newlines_word, _) = unsafe {
            process_chunk_64_bmi2(padded.as_ptr(), delimiter, quote_char, newline, qq_carry)
        };

        let mask = (1u64 << remaining) - 1;
        markers_word &= mask;
        newlines_word &= mask;

        markers_writer.write_bits(markers_word, remaining);
        newlines_writer.write_bits(newlines_word, remaining);
    }

    let markers_words = markers_writer.finish();
    let newlines_words = newlines_writer.finish();

    let lightweight = DsvIndexLightweight::new(markers_words, newlines_words, text.len());
    DsvIndex::new_lightweight(lightweight)
}
/// Alternating bit pattern used by toggle64: 0101...
const ODDS_MASK: u64 = 0x5555_5555_5555_5555;

/// Process a 64-byte chunk using AVX2 for character matching and BMI2 PDEP for quote masking.
///
/// This is the core algorithm from hw-dsv, ported to Rust.
#[cfg(target_arch = "x86_64")]
#[inline]
#[target_feature(enable = "avx2", enable = "bmi2")]
unsafe fn process_chunk_64_bmi2(
    ptr: *const u8,
    delimiter: i8,
    quote_char: i8,
    newline: i8,
    qq_carry: u64,
) -> (u64, u64, u64) {
    // Load 2 x 32-byte chunks using AVX2
    let chunk0 = _mm256_loadu_si256(ptr.cast::<__m256i>());
    let chunk1 = _mm256_loadu_si256(ptr.add(32).cast::<__m256i>());

    // Create comparison vectors
    let v_delimiter = _mm256_set1_epi8(delimiter);
    let v_quote = _mm256_set1_epi8(quote_char);
    let v_newline = _mm256_set1_epi8(newline);

    // Compare chunk0
    let eq_delim0 = _mm256_cmpeq_epi8(chunk0, v_delimiter);
    let eq_quote0 = _mm256_cmpeq_epi8(chunk0, v_quote);
    let eq_nl0 = _mm256_cmpeq_epi8(chunk0, v_newline);

    // Compare chunk1
    let eq_delim1 = _mm256_cmpeq_epi8(chunk1, v_delimiter);
    let eq_quote1 = _mm256_cmpeq_epi8(chunk1, v_quote);
    let eq_nl1 = _mm256_cmpeq_epi8(chunk1, v_newline);

    // Extract bitmasks
    let delim_mask0 = _mm256_movemask_epi8(eq_delim0) as u32 as u64;
    let delim_mask1 = _mm256_movemask_epi8(eq_delim1) as u32 as u64;
    let quote_mask0 = _mm256_movemask_epi8(eq_quote0) as u32 as u64;
    let quote_mask1 = _mm256_movemask_epi8(eq_quote1) as u32 as u64;
    let nl_mask0 = _mm256_movemask_epi8(eq_nl0) as u32 as u64;
    let nl_mask1 = _mm256_movemask_epi8(eq_nl1) as u32 as u64;

    // Combine into 64-bit masks
    let delim_mask = delim_mask0 | (delim_mask1 << 32);
    let quote_mask = quote_mask0 | (quote_mask1 << 32);
    let nl_mask = nl_mask0 | (nl_mask1 << 32);

    // Use BMI2 PDEP for quote state masking (the hw-dsv algorithm)
    let (outside_quotes, new_carry) = toggle64_bmi2(qq_carry, quote_mask);

    // Delimiters and newlines are valid only outside quotes
    let valid_delim = delim_mask & outside_quotes;
    let valid_nl = nl_mask & outside_quotes;

    // Markers = delimiters OR newlines (outside quotes)
    let markers = valid_delim | valid_nl;
    let newlines = valid_nl;

    (markers, newlines, new_carry)
}

/// Compute the quote mask using BMI2 PDEP instruction.
///
/// This is the critical function from hw-dsv that provides ~50-100x speedup
/// over iterative approaches. It uses carry propagation to track which
/// positions are inside vs outside quotes.
///
/// # Algorithm
///
/// Given a bitmask `w` where each 1-bit marks a quote position:
/// 1. Use PDEP to scatter an alternating pattern (0101...) to quote positions
/// 2. Shift and add to create carry propagation
/// 3. The result has 1-bits where we're outside quotes, 0-bits inside
///
/// # Arguments
/// * `carry` - Carry from previous chunk (0 = outside quotes, 1 = inside quotes)
/// * `w` - Bitmask of quote positions in this chunk
///
/// # Returns
/// * `(outside_mask, new_carry)` - Mask and carry for next chunk
#[cfg(target_arch = "x86_64")]
#[inline]
#[target_feature(enable = "bmi2")]
unsafe fn toggle64_bmi2(carry: u64, w: u64) -> (u64, u64) {
    // Extract the carry bit (0 or 1)
    let c = carry & 0x1;

    // PDEP scatters the alternating pattern to quote positions
    // If c=0: places 1s at odd quotes (1st, 3rd, 5th...) - these are "enters"
    // If c=1: places 1s at even quotes (2nd, 4th, 6th...) - shifted by 1
    let addend = _pdep_u64(ODDS_MASK << c, w);

    // This is the key insight: we use addition with carry propagation
    // to create a mask that "fills in" between quote pairs.
    //
    // The formula: ((addend << 1) | c) + !w
    //
    // - addend << 1: shift the deposited bits left by 1
    // - | c: include the carry from previous chunk
    // - + !w: add the complement of quote positions
    //
    // The addition propagates carries through non-quote regions,
    // effectively "filling" the regions between matched quote pairs.
    let comp_w = !w;
    let shifted = (addend << 1) | c;
    let result = shifted.wrapping_add(comp_w);

    // Quote state after this chunk = incoming state XOR quote-count parity.
    // The adder's carry-out cannot be used here: `addend << 1` drops a bit
    // deposited at position 63, so a quote that opens at bit 63 never
    // produces an overflow and the carry would be lost (#149).
    let new_carry = (w.count_ones() as u64 + c) & 1;

    (result, new_carry)
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    /// Detection guard for the BMI2+AVX2 backend; emits a visible `SKIPPED`
    /// line when unavailable so a fully-skipped run doesn't read as green (#193).
    fn has_bmi2() -> bool {
        crate::util::simd::note_simd_skip_unless(
            is_x86_feature_detected!("bmi2") && is_x86_feature_detected!("avx2"),
            "bmi2+avx2",
        )
    }

    #[test]
    fn test_simple_csv() {
        if !has_bmi2() {
            return;
        }

        let csv = b"a,b,c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.marker_count(), 3);
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_quoted_delimiter() {
        if !has_bmi2() {
            return;
        }

        let csv = b"\"a,b\",c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        // The comma inside quotes should not be a marker
        assert_eq!(index.marker_count(), 2); // comma after quote + newline
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_quoted_newline() {
        if !has_bmi2() {
            return;
        }

        let csv = b"\"a\nb\",c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        // The newline inside quotes should not be counted
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_multiple_rows() {
        if !has_bmi2() {
            return;
        }

        let csv = b"a,b\nc,d\ne,f\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.row_count(), 3);
        assert_eq!(index.marker_count(), 6); // 3 commas + 3 newlines
    }

    #[test]
    fn test_bmi2_matches_avx2() {
        if !has_bmi2() {
            return;
        }

        let csv = b"a,b,c\nd,e,f\n\"g,h\",i\n";
        let config = DsvConfig::default();

        let index_bmi2 = build_index_simd(csv, &config);
        let index_avx2 = super::super::avx2::build_index_simd(csv, &config);

        assert_eq!(
            index_bmi2.marker_count(),
            index_avx2.marker_count(),
            "Marker count mismatch"
        );
        assert_eq!(
            index_bmi2.row_count(),
            index_avx2.row_count(),
            "Row count mismatch"
        );
    }

    #[test]
    fn test_bmi2_matches_scalar() {
        if !has_bmi2() {
            return;
        }

        let csv = b"a,b,c\nd,e,f\n\"g,h\",i\n";
        let config = DsvConfig::default();

        let index_bmi2 = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(
            index_bmi2.marker_count(),
            index_scalar.marker_count(),
            "Marker count mismatch vs scalar"
        );
        assert_eq!(
            index_bmi2.row_count(),
            index_scalar.row_count(),
            "Row count mismatch vs scalar"
        );
    }

    #[test]
    fn test_large_csv() {
        if !has_bmi2() {
            return;
        }

        let csv = b"a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s,t,u,v,w,x,y,z\n\
                   1,2,3,4,5,6,7,8,9,0,1,2,3,4,5,6,7,8,9,0,1,2,3,4,5,6\n";
        let config = DsvConfig::default();

        let index_bmi2 = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(index_bmi2.marker_count(), index_scalar.marker_count());
        assert_eq!(index_bmi2.row_count(), index_scalar.row_count());
    }

    #[test]
    fn test_quoted_spanning_chunks() {
        if !has_bmi2() {
            return;
        }

        // Quote that spans a 64-byte boundary
        let mut csv = Vec::new();
        csv.push(b'"');
        #[allow(clippy::same_item_push)]
        // STYLE-0004: builds a test CSV fixture inline; explicit pushes read as the field being constructed
        for _ in 0..70 {
            csv.push(b'x');
        }
        csv.push(b'"');
        csv.push(b',');
        csv.push(b'b');
        csv.push(b'\n');

        let config = DsvConfig::default();
        let index_bmi2 = build_index_simd(&csv, &config);
        let index_scalar = super::super::super::parser::build_index(&csv, &config);

        assert_eq!(
            index_bmi2.marker_count(),
            index_scalar.marker_count(),
            "Marker count mismatch for spanning quote"
        );
        assert_eq!(
            index_bmi2.row_count(),
            index_scalar.row_count(),
            "Row count mismatch for spanning quote"
        );
    }

    #[test]
    fn test_toggle64_basic() {
        if !has_bmi2() {
            return;
        }

        unsafe {
            // No quotes - everything is outside
            let (mask, carry) = toggle64_bmi2(0, 0);
            assert_eq!(carry, 0, "No quotes should not change carry");
            assert_eq!(mask, !0u64, "No quotes means all outside");

            // Single quote at position 0 - everything after is inside
            let (_mask, carry) = toggle64_bmi2(0, 1);
            // After the quote, we're inside, so the mask should have 0s
            assert_eq!(carry, 1, "Odd quotes should set carry");

            // #149 regression: a quote at bit 63 must still toggle the carry.
            // The overflow-based carry lost an opener at bit 63 because
            // `addend << 1` shifts the deposited bit out of the word.
            let (mask, carry) = toggle64_bmi2(0, 1 << 63);
            assert_eq!(carry, 1, "Opener at bit 63 must carry into next chunk");
            assert_eq!(mask, !0u64 >> 1, "Bits 0..63 outside, bit 63 inside");
            let (_mask, carry) = toggle64_bmi2(1, 1 << 63);
            assert_eq!(carry, 0, "Closer at bit 63 must clear the carry");
        }
    }

    #[test]
    fn test_toggle64_matches_bit_serial_reference() {
        if !has_bmi2() {
            return;
        }

        // Independent bit-serial oracle: walk the word tracking an inside-quotes
        // flag, exactly like the scalar DSV parser. A quote bit takes its
        // post-toggle state — an opening quote reads as inside (0), a closing
        // quote as outside (1). See the SVE2 twin of this test in
        // `src/util/simd/sve2.rs` (#149).
        fn toggle64_reference(carry: u64, quote_mask: u64) -> (u64, u64) {
            let mut inside = carry & 1 == 1;
            let mut outside_mask = 0u64;
            for i in 0..64 {
                if (quote_mask >> i) & 1 == 1 {
                    inside = !inside;
                }
                if !inside {
                    outside_mask |= 1 << i;
                }
            }
            (outside_mask, u64::from(inside))
        }

        // Deterministic PRNG for wide mask coverage without a rand dependency.
        fn splitmix64(state: &mut u64) -> u64 {
            *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = *state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        let mut patterns = vec![
            0u64,
            1,
            0b11,
            0b101,
            0b1001,
            0x8000_0000_0000_0000,
            0xC000_0000_0000_0000,
            0x8000_0000_0000_0001,
            0xAAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555,
            0xFF00_FF00_FF00_FF00,
            !0u64,
        ];
        patterns.extend((0..64).map(|i| 1u64 << i));
        let mut state = 0x149u64;
        patterns.extend((0..512).map(|_| splitmix64(&mut state)));

        for &quote_mask in &patterns {
            for carry in [0u64, 1] {
                unsafe {
                    let (bmi2_mask, bmi2_carry) = toggle64_bmi2(carry, quote_mask);
                    let (ref_mask, ref_carry) = toggle64_reference(carry, quote_mask);

                    assert_eq!(
                        bmi2_mask, ref_mask,
                        "Mask mismatch for quote_mask={quote_mask:#x}, carry={carry}"
                    );
                    assert_eq!(
                        bmi2_carry, ref_carry,
                        "Carry mismatch for quote_mask={quote_mask:#x}, carry={carry}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_complex_quoting() {
        if !has_bmi2() {
            return;
        }

        // Multiple quoted fields with various patterns
        let csv = b"\"a\",\"b\",\"c\"\n\"d\",e,\"f\"\n";
        let config = DsvConfig::default();

        let index_bmi2 = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(
            index_bmi2.marker_count(),
            index_scalar.marker_count(),
            "Complex quoting: marker count mismatch"
        );
        assert_eq!(
            index_bmi2.row_count(),
            index_scalar.row_count(),
            "Complex quoting: row count mismatch"
        );
    }
}
