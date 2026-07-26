#![allow(unsafe_code)] // x86_64 SSE2 SIMD intrinsics
//! SSE2-accelerated DSV indexing for x86_64.
//!
//! SSE2 is baseline for all x86_64 CPUs, providing universal availability.
//! Processes 64 bytes at a time using 4x 16-byte SSE2 loads.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use alloc::vec;

use super::super::config::DsvConfig;
use super::super::index::DsvIndex;
use super::super::index_lightweight::DsvIndexLightweight;
use crate::json::BitWriter;
use crate::util::simd::quote_mask::{prefix_xor, toggle64_from_prefix_xor};

/// Build a DsvIndex using SSE2 SIMD acceleration.
#[cfg(target_arch = "x86_64")]
pub fn build_index_simd(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if text.is_empty() {
        return DsvIndex::new_lightweight(DsvIndexLightweight::new(vec![], vec![], 0));
    }

    // SAFETY: SSE2 is mandatory on x86_64
    unsafe { build_index_sse2(text, config) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn build_index_sse2(text: &[u8], config: &DsvConfig) -> DsvIndex {
    let num_words = text.len().div_ceil(64);
    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);

    // Track quote state across chunks using carry (same convention as the
    // BMI2/SVE2 deposit backends)
    let mut qq_carry: u64 = 0;

    let delimiter = config.delimiter as i8;
    let quote_char = config.quote_char as i8;
    let newline = config.newline as i8;

    let mut offset = 0;

    // Process 64-byte chunks (4x 16-byte SSE2 loads)
    while offset + 64 <= text.len() {
        let (markers_word, newlines_word, new_carry) = unsafe {
            process_chunk_64(
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

        let (mut markers_word, mut newlines_word, _) =
            unsafe { process_chunk_64(padded.as_ptr(), delimiter, quote_char, newline, qq_carry) };

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

/// Process a 64-byte chunk and return (markers, newlines, new_carry).
#[cfg(target_arch = "x86_64")]
#[inline]
#[target_feature(enable = "sse2")]
unsafe fn process_chunk_64(
    ptr: *const u8,
    delimiter: i8,
    quote_char: i8,
    newline: i8,
    qq_carry: u64,
) -> (u64, u64, u64) {
    // Load 4 x 16-byte chunks
    let chunk0 = _mm_loadu_si128(ptr.cast::<__m128i>());
    let chunk1 = _mm_loadu_si128(ptr.add(16).cast::<__m128i>());
    let chunk2 = _mm_loadu_si128(ptr.add(32).cast::<__m128i>());
    let chunk3 = _mm_loadu_si128(ptr.add(48).cast::<__m128i>());

    // Create comparison vectors
    let v_delimiter = _mm_set1_epi8(delimiter);
    let v_quote = _mm_set1_epi8(quote_char);
    let v_newline = _mm_set1_epi8(newline);

    // Compare each chunk
    let eq_delim0 = _mm_cmpeq_epi8(chunk0, v_delimiter);
    let eq_quote0 = _mm_cmpeq_epi8(chunk0, v_quote);
    let eq_nl0 = _mm_cmpeq_epi8(chunk0, v_newline);

    let eq_delim1 = _mm_cmpeq_epi8(chunk1, v_delimiter);
    let eq_quote1 = _mm_cmpeq_epi8(chunk1, v_quote);
    let eq_nl1 = _mm_cmpeq_epi8(chunk1, v_newline);

    let eq_delim2 = _mm_cmpeq_epi8(chunk2, v_delimiter);
    let eq_quote2 = _mm_cmpeq_epi8(chunk2, v_quote);
    let eq_nl2 = _mm_cmpeq_epi8(chunk2, v_newline);

    let eq_delim3 = _mm_cmpeq_epi8(chunk3, v_delimiter);
    let eq_quote3 = _mm_cmpeq_epi8(chunk3, v_quote);
    let eq_nl3 = _mm_cmpeq_epi8(chunk3, v_newline);

    // Extract bitmasks
    let delim_mask0 = _mm_movemask_epi8(eq_delim0) as u16 as u64;
    let delim_mask1 = _mm_movemask_epi8(eq_delim1) as u16 as u64;
    let delim_mask2 = _mm_movemask_epi8(eq_delim2) as u16 as u64;
    let delim_mask3 = _mm_movemask_epi8(eq_delim3) as u16 as u64;

    let quote_mask0 = _mm_movemask_epi8(eq_quote0) as u16 as u64;
    let quote_mask1 = _mm_movemask_epi8(eq_quote1) as u16 as u64;
    let quote_mask2 = _mm_movemask_epi8(eq_quote2) as u16 as u64;
    let quote_mask3 = _mm_movemask_epi8(eq_quote3) as u16 as u64;

    let nl_mask0 = _mm_movemask_epi8(eq_nl0) as u16 as u64;
    let nl_mask1 = _mm_movemask_epi8(eq_nl1) as u16 as u64;
    let nl_mask2 = _mm_movemask_epi8(eq_nl2) as u16 as u64;
    let nl_mask3 = _mm_movemask_epi8(eq_nl3) as u16 as u64;

    // Combine into 64-bit masks
    let delim_mask = delim_mask0 | (delim_mask1 << 16) | (delim_mask2 << 32) | (delim_mask3 << 48);
    let quote_mask = quote_mask0 | (quote_mask1 << 16) | (quote_mask2 << 32) | (quote_mask3 << 48);
    let nl_mask = nl_mask0 | (nl_mask1 << 16) | (nl_mask2 << 32) | (nl_mask3 << 48);

    // Compute the outside-quotes mask from the prefix XOR of the quote bitmap
    // (shared tail: `crate::util::simd::quote_mask`)
    let (outside_quotes, new_carry) =
        toggle64_from_prefix_xor(qq_carry, quote_mask, prefix_xor(quote_mask));

    // Delimiters and newlines are valid only outside quotes
    let valid_delim = delim_mask & outside_quotes;
    let valid_nl = nl_mask & outside_quotes;

    // Markers = delimiters OR newlines (outside quotes)
    let markers = valid_delim | valid_nl;
    let newlines = valid_nl;

    (markers, newlines, new_carry)
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn test_simple_csv() {
        let csv = b"a,b,c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.marker_count(), 3);
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_quoted_delimiter() {
        let csv = b"\"a,b\",c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.marker_count(), 2);
        assert_eq!(index.row_count(), 1);
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
    fn test_large_csv() {
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
        let index_simd = build_index_simd(&csv, &config);
        let index_scalar = super::super::super::parser::build_index(&csv, &config);

        assert_eq!(index_simd.marker_count(), index_scalar.marker_count());
        assert_eq!(index_simd.row_count(), index_scalar.row_count());
    }
}
