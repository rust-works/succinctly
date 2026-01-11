//! AVX2-accelerated DSV indexing for x86_64.
//!
//! Processes 64 bytes at a time using x86_64 AVX2 SIMD instructions to find
//! delimiters, newlines, and quotes in parallel.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use super::super::config::DsvConfig;
use super::super::index::DsvIndex;
use super::super::index_lightweight::DsvIndexLightweight;
use crate::json::BitWriter;

/// Build a DsvIndex using AVX2 SIMD acceleration.
#[cfg(target_arch = "x86_64")]
pub fn build_index_simd(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if text.is_empty() {
        let empty = DsvIndexLightweight::new(vec![], vec![], 0);
        return DsvIndex::new_lightweight(empty);
    }

    // SAFETY: Caller verified AVX2 is available via runtime detection
    unsafe { build_index_avx2(text, config) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn build_index_avx2(text: &[u8], config: &DsvConfig) -> DsvIndex {
    let num_words = text.len().div_ceil(64);
    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);

    let mut in_quote = false;

    let delimiter = config.delimiter as i8;
    let quote_char = config.quote_char as i8;
    let newline = config.newline as i8;

    let mut offset = 0;

    // Process 64-byte chunks (2x 32-byte AVX2 loads)
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

    // Process remaining bytes
    if offset < text.len() {
        let remaining = text.len() - offset;

        let mut padded = [0u8; 64];
        padded[..remaining].copy_from_slice(&text[offset..]);

        let (mut markers_word, mut newlines_word, _) =
            unsafe { process_chunk_64(padded.as_ptr(), delimiter, quote_char, newline, in_quote) };

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

/// Process a 64-byte chunk and return (markers, newlines, new_in_quote).
#[cfg(target_arch = "x86_64")]
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn process_chunk_64(
    ptr: *const u8,
    delimiter: i8,
    quote_char: i8,
    newline: i8,
    in_quote: bool,
) -> (u64, u64, bool) {
    // Load 2 x 32-byte chunks
    let chunk0 = _mm256_loadu_si256(ptr as *const __m256i);
    let chunk1 = _mm256_loadu_si256(ptr.add(32) as *const __m256i);

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

    // Compute the in-quote mask using prefix XOR
    let quote_xor = prefix_xor(quote_mask);
    let in_quote_mask = if in_quote { !quote_xor } else { quote_xor };

    // Delimiters and newlines are valid only outside quotes
    let valid_delim = delim_mask & !in_quote_mask;
    let valid_nl = nl_mask & !in_quote_mask;

    // Markers = delimiters OR newlines (outside quotes)
    let markers = valid_delim | valid_nl;
    let newlines = valid_nl;

    // New in_quote state
    let quote_count = quote_mask.count_ones();
    let new_in_quote = (quote_count & 1 == 1) != in_quote;

    (markers, newlines, new_in_quote)
}

/// Compute inclusive prefix XOR (cumulative XOR) of a 64-bit mask.
///
/// The prefix XOR at position i is the XOR of all bits at positions 0..=i.
/// This tells us the "parity" of quotes seen so far (including current),
/// which indicates whether we're inside a quoted field.
#[inline]
fn prefix_xor(x: u64) -> u64 {
    let mut y = x;
    y ^= y << 1;
    y ^= y << 2;
    y ^= y << 4;
    y ^= y << 8;
    y ^= y << 16;
    y ^= y << 32;
    y
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    #[test]
    fn test_simple_csv() {
        if !is_x86_feature_detected!("avx2") {
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
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let csv = b"\"a,b\",c\n";
        let config = DsvConfig::default();
        let index = build_index_simd(csv, &config);

        assert_eq!(index.marker_count(), 2);
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_simd_matches_scalar() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let csv = b"a,b,c\nd,e,f\n\"g,h\",i\n";
        let config = DsvConfig::default();

        let index_simd = build_index_simd(csv, &config);
        let index_scalar = super::super::super::parser::build_index(csv, &config);

        assert_eq!(index_simd.marker_count(), index_scalar.marker_count());
        assert_eq!(index_simd.row_count(), index_scalar.row_count());
    }

    #[test]
    fn test_large_csv() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

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
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let mut csv = Vec::new();
        csv.push(b'"');
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
