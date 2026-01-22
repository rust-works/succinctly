//! SVE2-accelerated DSV indexing for ARM64.
//!
//! Uses SVE2 BDEP instruction for quote state masking, providing ~10x speedup
//! over the portable prefix_xor approach on supported processors.
//!
//! Requires: NEON (for character matching) + SVE2-BITPERM (for BDEP)
//! Supported: Azure Cobalt 100, AWS Graviton 4, Neoverse N2/V2
//!
//! ## Comparison to NEON-only implementation
//!
//! | Operation | NEON (prefix_xor) | SVE2 (BDEP) |
//! |-----------|-------------------|-------------|
//! | Quote masking | 6 XOR + 6 shift | 1 BDEP + 1 ADD |
//! | Expected speedup | 1x | ~10x |
//!
//! The character matching still uses NEON (which is always available on aarch64).
//! SVE2 BDEP replaces only the quote state computation.

#[cfg(not(test))]
use alloc::vec;

use core::arch::aarch64::*;

use super::super::config::DsvConfig;
use super::super::index::DsvIndex;
use super::super::index_lightweight::DsvIndexLightweight;
use crate::json::BitWriter;
use crate::util::simd::sve2::toggle64_sve2;

/// Build a DsvIndex using SVE2 + NEON SIMD acceleration.
///
/// This function processes 64 bytes at a time, using:
/// - NEON for finding delimiter, newline, and quote positions
/// - SVE2 BDEP for computing the in-quote mask (~10x faster than prefix_xor)
///
/// # Safety
///
/// Caller must verify SVE2-BITPERM is available via runtime detection.
pub fn build_index_simd(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if text.is_empty() {
        return DsvIndex::new_lightweight(DsvIndexLightweight::new(vec![], vec![], 0));
    }

    // SAFETY: Caller verified SVE2-BITPERM is available via runtime detection
    unsafe { build_index_sve2(text, config) }
}

#[target_feature(enable = "neon", enable = "sve2-bitperm")]
unsafe fn build_index_sve2(text: &[u8], config: &DsvConfig) -> DsvIndex {
    let num_words = text.len().div_ceil(64);
    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);

    // Track quote state across chunks using carry (same as BMI2 implementation)
    let mut qq_carry: u64 = 0;

    let delimiter = config.delimiter;
    let quote_char = config.quote_char;
    let newline = config.newline;

    let mut offset = 0;

    // Process 64-byte chunks (4x 16-byte NEON loads)
    while offset + 64 <= text.len() {
        let (markers_word, newlines_word, new_carry) = unsafe {
            process_chunk_64_sve2(
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
            process_chunk_64_sve2(padded.as_ptr(), delimiter, quote_char, newline, qq_carry)
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

/// Process a 64-byte chunk using NEON for character matching and SVE2 BDEP for quote masking.
///
/// This is the core algorithm from hw-dsv, using SVE2 BDEP instead of BMI2 PDEP.
#[inline]
#[target_feature(enable = "neon", enable = "sve2-bitperm")]
unsafe fn process_chunk_64_sve2(
    ptr: *const u8,
    delimiter: u8,
    quote_char: u8,
    newline: u8,
    qq_carry: u64,
) -> (u64, u64, u64) {
    // Load 4 x 16-byte chunks using NEON
    let chunk0 = vld1q_u8(ptr);
    let chunk1 = vld1q_u8(ptr.add(16));
    let chunk2 = vld1q_u8(ptr.add(32));
    let chunk3 = vld1q_u8(ptr.add(48));

    // Create comparison vectors
    let v_delimiter = vdupq_n_u8(delimiter);
    let v_quote = vdupq_n_u8(quote_char);
    let v_newline = vdupq_n_u8(newline);

    // Compare each chunk
    let (delim0, quote0, nl0) = compare_chunk(chunk0, v_delimiter, v_quote, v_newline);
    let (delim1, quote1, nl1) = compare_chunk(chunk1, v_delimiter, v_quote, v_newline);
    let (delim2, quote2, nl2) = compare_chunk(chunk2, v_delimiter, v_quote, v_newline);
    let (delim3, quote3, nl3) = compare_chunk(chunk3, v_delimiter, v_quote, v_newline);

    // Extract bitmasks
    let delim_mask0 = neon_movemask(delim0) as u64;
    let delim_mask1 = neon_movemask(delim1) as u64;
    let delim_mask2 = neon_movemask(delim2) as u64;
    let delim_mask3 = neon_movemask(delim3) as u64;

    let quote_mask0 = neon_movemask(quote0) as u64;
    let quote_mask1 = neon_movemask(quote1) as u64;
    let quote_mask2 = neon_movemask(quote2) as u64;
    let quote_mask3 = neon_movemask(quote3) as u64;

    let nl_mask0 = neon_movemask(nl0) as u64;
    let nl_mask1 = neon_movemask(nl1) as u64;
    let nl_mask2 = neon_movemask(nl2) as u64;
    let nl_mask3 = neon_movemask(nl3) as u64;

    // Combine into 64-bit masks
    let delim_mask = delim_mask0 | (delim_mask1 << 16) | (delim_mask2 << 32) | (delim_mask3 << 48);
    let quote_mask = quote_mask0 | (quote_mask1 << 16) | (quote_mask2 << 32) | (quote_mask3 << 48);
    let nl_mask = nl_mask0 | (nl_mask1 << 16) | (nl_mask2 << 32) | (nl_mask3 << 48);

    // Use SVE2 BDEP for quote state masking (the hw-dsv algorithm)
    // This is ~10x faster than the prefix_xor approach
    let (outside_quotes, new_carry) = toggle64_sve2(qq_carry, quote_mask);

    // Delimiters and newlines are valid only outside quotes
    let valid_delim = delim_mask & outside_quotes;
    let valid_nl = nl_mask & outside_quotes;

    // Markers = delimiters OR newlines (outside quotes)
    let markers = valid_delim | valid_nl;
    let newlines = valid_nl;

    (markers, newlines, new_carry)
}

/// Compare a 16-byte chunk against delimiter, quote, and newline characters.
#[inline]
#[target_feature(enable = "neon")]
unsafe fn compare_chunk(
    chunk: uint8x16_t,
    v_delimiter: uint8x16_t,
    v_quote: uint8x16_t,
    v_newline: uint8x16_t,
) -> (uint8x16_t, uint8x16_t, uint8x16_t) {
    let eq_delim = vceqq_u8(chunk, v_delimiter);
    let eq_quote = vceqq_u8(chunk, v_quote);
    let eq_newline = vceqq_u8(chunk, v_newline);
    (eq_delim, eq_quote, eq_newline)
}

/// Extract a bitmask from the high bit of each byte in a NEON vector.
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    // Shift right by 7 to get 0 or 1 in each byte
    let high_bits = vshrq_n_u8::<7>(v);

    // Extract the 16 bytes as two u64 values
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));

    // Pack 8 bytes into 8 bits using multiplication trick
    const MAGIC: u64 = 0x0102040810204080;

    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;

    (low_packed as u16) | ((high_packed as u16) << 8)
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
    fn test_simple_csv() {
        if !has_sve2() {
            eprintln!("Skipping SVE2 test: CPU doesn't support sve2-bitperm");
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
        if !has_sve2() {
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
        if !has_sve2() {
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
        if !has_sve2() {
            return;
        }

        let csv = b"a,b\nc,d\ne,f\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.row_count(), 3);
        assert_eq!(index.marker_count(), 6); // 3 commas + 3 newlines
    }

    #[test]
    fn test_sve2_matches_neon() {
        if !has_sve2() {
            return;
        }

        let csv = b"a,b,c\nd,e,f\n\"g,h\",i\n";
        let config = DsvConfig::default();

        let index_sve2 = build_index_simd(csv, &config);
        let index_neon = super::super::neon::build_index_simd(csv, &config);

        assert_eq!(
            index_sve2.marker_count(),
            index_neon.marker_count(),
            "Marker count mismatch"
        );
        assert_eq!(
            index_sve2.row_count(),
            index_neon.row_count(),
            "Row count mismatch"
        );
    }

    #[test]
    fn test_sve2_matches_scalar() {
        if !has_sve2() {
            return;
        }

        let csv = b"a,b,c\nd,e,f\n\"g,h\",i\n";
        let config = DsvConfig::default();

        let index_sve2 = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(
            index_sve2.marker_count(),
            index_scalar.marker_count(),
            "Marker count mismatch vs scalar"
        );
        assert_eq!(
            index_sve2.row_count(),
            index_scalar.row_count(),
            "Row count mismatch vs scalar"
        );
    }

    #[test]
    fn test_large_csv() {
        if !has_sve2() {
            return;
        }

        let csv = b"a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s,t,u,v,w,x,y,z\n\
                   1,2,3,4,5,6,7,8,9,0,1,2,3,4,5,6,7,8,9,0,1,2,3,4,5,6\n";
        let config = DsvConfig::default();

        let index_sve2 = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(index_sve2.marker_count(), index_scalar.marker_count());
        assert_eq!(index_sve2.row_count(), index_scalar.row_count());
    }

    #[test]
    fn test_quoted_spanning_chunks() {
        if !has_sve2() {
            return;
        }

        // Quote that spans a 64-byte boundary
        let mut csv = Vec::new();
        csv.push(b'"');
        #[allow(clippy::same_item_push)]
        for _ in 0..70 {
            csv.push(b'x');
        }
        csv.push(b'"');
        csv.push(b',');
        csv.push(b'b');
        csv.push(b'\n');

        let config = DsvConfig::default();
        let index_sve2 = build_index_simd(&csv, &config);
        let index_scalar = super::super::super::parser::build_index(&csv, &config);

        assert_eq!(
            index_sve2.marker_count(),
            index_scalar.marker_count(),
            "Marker count mismatch for spanning quote"
        );
        assert_eq!(
            index_sve2.row_count(),
            index_scalar.row_count(),
            "Row count mismatch for spanning quote"
        );
    }

    #[test]
    fn test_complex_quoting() {
        if !has_sve2() {
            return;
        }

        // Multiple quoted fields with various patterns
        let csv = b"\"a\",\"b\",\"c\"\n\"d\",e,\"f\"\n";
        let config = DsvConfig::default();

        let index_sve2 = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(
            index_sve2.marker_count(),
            index_scalar.marker_count(),
            "Complex quoting: marker count mismatch"
        );
        assert_eq!(
            index_sve2.row_count(),
            index_scalar.row_count(),
            "Complex quoting: row count mismatch"
        );
    }
}
