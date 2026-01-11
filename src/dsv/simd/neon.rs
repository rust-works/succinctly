//! NEON-accelerated DSV indexing for ARM64.
//!
//! Processes 64 bytes at a time using ARM NEON SIMD instructions to find
//! delimiters, newlines, and quotes in parallel.

use core::arch::aarch64::*;

use super::super::config::DsvConfig;
use super::super::index::DsvIndex;
use crate::bits::BitVec;
use crate::json::BitWriter;
use crate::Config;

/// Build a DsvIndex using NEON SIMD acceleration.
///
/// This function processes 64 bytes at a time, using NEON to:
/// 1. Find all delimiter, newline, and quote positions
/// 2. Compute the in-quote mask using prefix XOR
/// 3. Mask out positions inside quotes
pub fn build_index_simd(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if text.is_empty() {
        return DsvIndex::new(BitVec::new(), BitVec::new(), 0);
    }

    // SAFETY: NEON is mandatory on aarch64
    unsafe { build_index_neon(text, config) }
}

#[target_feature(enable = "neon")]
unsafe fn build_index_neon(text: &[u8], config: &DsvConfig) -> DsvIndex {
    let num_words = text.len().div_ceil(64);
    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);

    // Track quote state across chunks
    let mut in_quote = false;

    let delimiter = config.delimiter;
    let quote_char = config.quote_char;
    let newline = config.newline;

    let mut offset = 0;

    // Process 64-byte chunks (4x 16-byte NEON loads)
    while offset + 64 <= text.len() {
        let (markers_word, newlines_word, new_in_quote) = unsafe {
            process_chunk_64(
                text.as_ptr().add(offset),
                delimiter,
                quote_char,
                newline,
                in_quote,
            )
        };

        markers_writer.write_bits(markers_word, 64);
        newlines_writer.write_bits(newlines_word, 64);
        in_quote = new_in_quote;
        offset += 64;
    }

    // Process remaining bytes (less than 64)
    if offset < text.len() {
        let remaining = text.len() - offset;

        // Pad with zeros
        let mut padded = [0u8; 64];
        padded[..remaining].copy_from_slice(&text[offset..]);

        let (mut markers_word, mut newlines_word, _) =
            unsafe { process_chunk_64(padded.as_ptr(), delimiter, quote_char, newline, in_quote) };

        // Mask out the padding bits
        let mask = (1u64 << remaining) - 1;
        markers_word &= mask;
        newlines_word &= mask;

        markers_writer.write_bits(markers_word, remaining);
        newlines_writer.write_bits(newlines_word, remaining);
    }

    let markers_words = markers_writer.finish();
    let newlines_words = newlines_writer.finish();

    let bit_config = Config {
        select_sample_rate: config.select_sample_rate,
    };

    DsvIndex::new(
        BitVec::with_config(markers_words, text.len(), bit_config.clone()),
        BitVec::with_config(newlines_words, text.len(), bit_config),
        text.len(),
    )
}

/// Process a 64-byte chunk and return (markers, newlines, new_in_quote).
///
/// The algorithm:
/// 1. Find all quote, delimiter, and newline positions using SIMD
/// 2. Compute the in-quote mask using prefix XOR on quote positions
/// 3. Mask delimiters and newlines to exclude those inside quotes
#[inline]
#[target_feature(enable = "neon")]
unsafe fn process_chunk_64(
    ptr: *const u8,
    delimiter: u8,
    quote_char: u8,
    newline: u8,
    in_quote: bool,
) -> (u64, u64, bool) {
    // Load 4 x 16-byte chunks
    let chunk0 = vld1q_u8(ptr);
    let chunk1 = vld1q_u8(ptr.add(16));
    let chunk2 = vld1q_u8(ptr.add(32));
    let chunk3 = vld1q_u8(ptr.add(48));

    // Create comparison vectors
    let v_delimiter = vdupq_n_u8(delimiter);
    let v_quote = vdupq_n_u8(quote_char);
    let v_newline = vdupq_n_u8(newline);

    // Compare each chunk against delimiter, quote, and newline
    let (delim0, quote0, nl0) = compare_chunk(chunk0, v_delimiter, v_quote, v_newline);
    let (delim1, quote1, nl1) = compare_chunk(chunk1, v_delimiter, v_quote, v_newline);
    let (delim2, quote2, nl2) = compare_chunk(chunk2, v_delimiter, v_quote, v_newline);
    let (delim3, quote3, nl3) = compare_chunk(chunk3, v_delimiter, v_quote, v_newline);

    // Extract bitmasks for each 16-byte chunk
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

    // Compute the in-quote mask using prefix XOR
    // The prefix XOR of the quote mask tells us which positions are inside quotes.
    // If in_quote is true, we XOR with all 1s to flip the mask.
    let quote_xor = prefix_xor(quote_mask);
    let in_quote_mask = if in_quote { !quote_xor } else { quote_xor };

    // Delimiters and newlines are valid only outside quotes
    let valid_delim = delim_mask & !in_quote_mask;
    let valid_nl = nl_mask & !in_quote_mask;

    // Markers = delimiters OR newlines (both outside quotes)
    let markers = valid_delim | valid_nl;
    let newlines = valid_nl;

    // New in_quote state: count quotes and XOR with current state
    let quote_count = quote_mask.count_ones();
    let new_in_quote = (quote_count & 1 == 1) != in_quote;

    (markers, newlines, new_in_quote)
}

/// Compare a 16-byte chunk against delimiter, quote, and newline characters.
/// Returns (delimiter_mask, quote_mask, newline_mask) as NEON vectors.
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
/// Returns a u16 where bit i is set if byte i has its high bit set.
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

/// Compute inclusive prefix XOR (cumulative XOR) of a 64-bit mask.
///
/// The prefix XOR at position i is the XOR of all bits at positions 0..=i.
/// This tells us the "parity" of quotes seen so far (including current),
/// which indicates whether we're inside a quoted field.
///
/// After prefix XOR:
/// - Positions where the bit is 1 have seen an odd number of quotes (inside)
/// - Positions where the bit is 0 have seen an even number of quotes (outside)
///
/// Example: quotes at positions 2 and 5
/// quote_mask = 0b100100 (bits 2 and 5 set)
/// prefix_xor = 0b011100 (bits 2,3,4 are "inside" quotes)
///
/// The parallel prefix XOR algorithm uses doubling with LEFT shifts.
/// Left shifts propagate information from low bits to high bits,
/// which is what we need for prefix operations in LSB-first ordering.
#[inline]
fn prefix_xor(x: u64) -> u64 {
    // Parallel prefix XOR using doubling with left shifts
    // After step k, each bit i contains XOR of bits max(0, i-2^k+1)..=i
    // After all steps, each bit i contains XOR of bits 0..=i
    let mut y = x;
    y ^= y << 1;
    y ^= y << 2;
    y ^= y << 4;
    y ^= y << 8;
    y ^= y << 16;
    y ^= y << 32;
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_xor() {
        // No quotes
        assert_eq!(prefix_xor(0b0), 0b0);

        // Single quote at position 0
        assert_eq!(prefix_xor(0b1), !0u64); // All bits after are "inside"

        // Quote at position 2
        // prefix_xor should set all bits from position 2 onwards
        let q = 0b100u64;
        let px = prefix_xor(q);
        // Bits 0,1 should be 0 (outside), bits 2+ should be 1 (inside)
        assert_eq!(px & 0b11, 0);
        assert_eq!(px & 0b100, 0b100);

        // Quotes at positions 2 and 5 (entering and exiting)
        // Bits 2,3,4 should be inside, bits 0,1,5+ should be outside
        let q = 0b100100u64; // positions 2 and 5
        let px = prefix_xor(q);
        // After prefix XOR: bit i = XOR of bits 0..=i
        // pos 0: 0
        // pos 1: 0
        // pos 2: 1 (quote at 2)
        // pos 3: 1
        // pos 4: 1
        // pos 5: 0 (quote at 5 XORs with accumulated 1)
        assert_eq!(px & 0b111111, 0b011100);
    }

    #[test]
    fn test_simple_csv() {
        let csv = b"a,b,c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        // Should have 3 markers: positions 1 (,), 3 (,), 5 (\n)
        assert_eq!(index.marker_count(), 3);
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_quoted_delimiter() {
        let csv = b"\"a,b\",c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        // The comma inside quotes should not be a marker
        assert_eq!(index.marker_count(), 2); // comma after quote + newline
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_quoted_newline() {
        let csv = b"\"a\nb\",c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        // The newline inside quotes should not be counted
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_multiple_rows() {
        let csv = b"a,b\nc,d\ne,f\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.row_count(), 3);
        // 3 commas + 3 newlines = 6 markers
        assert_eq!(index.marker_count(), 6);
    }

    #[test]
    fn test_empty() {
        let csv = b"";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.row_count(), 0);
        assert_eq!(index.marker_count(), 0);
    }

    #[test]
    fn test_simd_matches_scalar() {
        let csv = b"a,b,c\nd,e,f\n\"g,h\",i\n";
        let config = DsvConfig::default();

        let index_simd = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(index_simd.marker_count(), index_scalar.marker_count());
        assert_eq!(index_simd.row_count(), index_scalar.row_count());
    }

    #[test]
    fn test_long_quoted_field() {
        // Test with quoted field spanning multiple 16-byte chunks
        let csv = b"\"this is a very long quoted field that spans multiple NEON chunks\",b\n";
        let config = DsvConfig::default();

        let index_simd = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(index_simd.marker_count(), index_scalar.marker_count());
        assert_eq!(index_simd.row_count(), index_scalar.row_count());
    }

    #[test]
    fn test_large_csv() {
        // Test with more than 64 bytes
        let csv = b"a,b,c,d,e,f,g,h,i,j,k,l,m,n,o,p,q,r,s,t,u,v,w,x,y,z\n\
                   1,2,3,4,5,6,7,8,9,0,1,2,3,4,5,6,7,8,9,0,1,2,3,4,5,6\n";
        let config = DsvConfig::default();

        let index_simd = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(index_simd.marker_count(), index_scalar.marker_count());
        assert_eq!(index_simd.row_count(), index_scalar.row_count());
    }

    #[test]
    fn test_quoted_spanning_chunks() {
        // Quote that spans a 64-byte boundary
        let mut csv = Vec::new();
        csv.push(b'"');
        // Add content to push past 64 bytes before closing quote
        for _ in 0..70 {
            csv.push(b'x');
        }
        csv.push(b'"');
        csv.push(b',');
        csv.push(b'b');
        csv.push(b'\n');

        let config = DsvConfig::default();
        let index_simd = build_index_simd(&csv, &config);
        let index_scalar = super::super::super::parser::build_index(&csv, &config);

        assert_eq!(
            index_simd.marker_count(),
            index_scalar.marker_count(),
            "Marker count mismatch"
        );
        assert_eq!(
            index_simd.row_count(),
            index_scalar.row_count(),
            "Row count mismatch"
        );
    }
}
