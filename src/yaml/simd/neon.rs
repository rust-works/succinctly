//! NEON-accelerated string scanning for YAML parsing on ARM64.
//!
//! Uses 128-bit NEON vectors to process 16 bytes at a time.

use core::arch::aarch64::*;

/// Extract a bitmask from the high bit of each byte in a NEON vector.
/// Returns a u16 where bit i is set if byte i has its high bit set.
///
/// Uses the optimized multiplication trick (same as JSON SIMD).
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    // Step 1: Shift right by 7 to get 0 or 1 in each byte
    let high_bits = vshrq_n_u8::<7>(v);

    // Step 2: Extract the 16 bytes as two u64 values
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));

    // Step 3: Pack 8 bytes into 8 bits using multiplication trick
    const MAGIC: u64 = 0x0102040810204080;
    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;

    (low_packed as u16) | ((high_packed as u16) << 8)
}

/// Find the next double-quote or backslash using NEON.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_quote_or_escape_neon(input: &[u8], start: usize, end: usize) -> Option<usize> {
    // SAFETY: We check bounds and target_arch = aarch64 guarantees NEON
    unsafe { find_quote_or_escape_neon_impl(input, start, end) }
}

#[target_feature(enable = "neon")]
unsafe fn find_quote_or_escape_neon_impl(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    // Process 16-byte chunks
    let quote_vec = vdupq_n_u8(b'"');
    let backslash_vec = vdupq_n_u8(b'\\');

    while offset + 16 <= len {
        let chunk = vld1q_u8(data.as_ptr().add(offset));

        // Compare against both targets
        let quotes = vceqq_u8(chunk, quote_vec);
        let backslashes = vceqq_u8(chunk, backslash_vec);

        // OR the results
        let matches = vorrq_u8(quotes, backslashes);

        // Extract bitmask
        let mask = neon_movemask(matches);

        if mask != 0 {
            // Found a match - return position of first match
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes with iterator
    data[offset..]
        .iter()
        .position(|&b| b == b'"' || b == b'\\')
        .map(|pos| offset + pos)
}

/// Find the next single-quote using NEON.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_single_quote_neon(input: &[u8], start: usize, end: usize) -> Option<usize> {
    // SAFETY: We check bounds and target_arch = aarch64 guarantees NEON
    unsafe { find_single_quote_neon_impl(input, start, end) }
}

#[target_feature(enable = "neon")]
unsafe fn find_single_quote_neon_impl(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    // Process 16-byte chunks
    let quote_vec = vdupq_n_u8(b'\'');

    while offset + 16 <= len {
        let chunk = vld1q_u8(data.as_ptr().add(offset));

        // Compare against single quote
        let matches = vceqq_u8(chunk, quote_vec);

        // Extract bitmask
        let mask = neon_movemask(matches);

        if mask != 0 {
            // Found a match - return position of first match
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes with iterator
    data[offset..]
        .iter()
        .position(|&b| b == b'\'')
        .map(|pos| offset + pos)
}

/// Count leading spaces (indentation) using NEON.
///
/// Returns the number of consecutive space characters starting at `start`.
#[inline]
pub fn count_leading_spaces_neon(input: &[u8], start: usize) -> usize {
    // SAFETY: target_arch = aarch64 guarantees NEON
    unsafe { count_leading_spaces_neon_impl(input, start) }
}

#[target_feature(enable = "neon")]
unsafe fn count_leading_spaces_neon_impl(input: &[u8], start: usize) -> usize {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let space_vec = vdupq_n_u8(b' ');

    // Process 16-byte chunks
    while offset + 16 <= len {
        let chunk = vld1q_u8(data.as_ptr().add(offset));

        // Compare against space
        let matches = vceqq_u8(chunk, space_vec);

        // Extract bitmask (1 bit per byte where match occurred)
        let mask = neon_movemask(matches);

        if mask != 0xFFFF {
            // Found a non-space - count trailing ones (consecutive spaces from start)
            // Invert mask: 1s become 0s where spaces were, then count trailing zeros
            return offset + (!mask).trailing_zeros() as usize;
        }

        offset += 16;
    }

    // Handle remaining bytes
    offset + data[offset..].iter().take_while(|&&b| b == b' ').count()
}

/// Find the next structural character for unquoted values: `\n`, `#`, or `:`.
///
/// Returns offset from `start` to the found character, or `None` if not found before `end`.
/// This is used for fast-path scanning in unquoted value parsing.
#[inline]
pub fn find_unquoted_structural_neon(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;

    // Fast path: for short inputs, scalar is faster than SIMD setup overhead
    // (NEON movemask uses expensive multiplication trick)
    if len < 32 {
        let data = &input[start..end];
        for (i, &b) in data.iter().enumerate() {
            match b {
                b'\n' | b'#' | b':' => return Some(i),
                _ => {}
            }
        }
        return None;
    }

    // SAFETY: target_arch = aarch64 guarantees NEON
    unsafe { find_unquoted_structural_neon_impl(input, start, end) }
}

#[target_feature(enable = "neon")]
unsafe fn find_unquoted_structural_neon_impl(
    input: &[u8],
    start: usize,
    end: usize,
) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    // Vectors for the three structural characters
    let newline_vec = vdupq_n_u8(b'\n');
    let hash_vec = vdupq_n_u8(b'#');
    let colon_vec = vdupq_n_u8(b':');

    // Process 16-byte chunks
    while offset + 16 <= len {
        let chunk = vld1q_u8(data.as_ptr().add(offset));

        // Compare against all three targets
        let newlines = vceqq_u8(chunk, newline_vec);
        let hashes = vceqq_u8(chunk, hash_vec);
        let colons = vceqq_u8(chunk, colon_vec);

        // OR all results together
        let matches = vorrq_u8(vorrq_u8(newlines, hashes), colons);

        // Extract bitmask
        let mask = neon_movemask(matches);

        if mask != 0 {
            // Found a match - return position of first match
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes with scalar loop
    for (i, &b) in data[offset..len].iter().enumerate() {
        match b {
            b'\n' | b'#' | b':' => return Some(offset + i),
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_neon_find_quote_basic() {
        let input = b"hello\"world";
        assert_eq!(find_quote_or_escape_neon(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_neon_find_backslash() {
        let input = b"hello\\world";
        assert_eq!(find_quote_or_escape_neon(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_neon_find_single_quote() {
        let input = b"hello'world";
        assert_eq!(find_single_quote_neon(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_neon_long_string() {
        // Test with > 16 bytes
        let mut input = vec![b'a'; 100];
        input[50] = b'"';
        assert_eq!(find_quote_or_escape_neon(&input, 0, input.len()), Some(50));
    }

    #[test]
    fn test_neon_at_chunk_boundary() {
        // Quote at exactly byte 16 (second chunk)
        let mut input = vec![b'a'; 32];
        input[16] = b'"';
        assert_eq!(find_quote_or_escape_neon(&input, 0, input.len()), Some(16));
    }

    #[test]
    fn test_neon_in_remainder() {
        // Quote in the remainder bytes (< 16)
        let mut input = vec![b'a'; 20];
        input[18] = b'"';
        assert_eq!(find_quote_or_escape_neon(&input, 0, input.len()), Some(18));
    }

    #[test]
    fn test_neon_count_leading_spaces_basic() {
        assert_eq!(count_leading_spaces_neon(b"  hello", 0), 2);
        assert_eq!(count_leading_spaces_neon(b"    world", 0), 4);
        assert_eq!(count_leading_spaces_neon(b"no spaces", 0), 0);
    }

    #[test]
    fn test_neon_count_leading_spaces_long() {
        // Test with > 16 bytes to exercise SIMD path
        let mut input = vec![b' '; 50];
        input.extend_from_slice(b"content");
        assert_eq!(count_leading_spaces_neon(&input, 0), 50);
    }

    #[test]
    fn test_neon_count_leading_spaces_at_boundary() {
        // Spaces ending exactly at 16-byte boundary
        let mut input = vec![b' '; 16];
        input.push(b'x');
        assert_eq!(count_leading_spaces_neon(&input, 0), 16);

        // Spaces ending at 32-byte boundary
        let mut input32 = vec![b' '; 32];
        input32.push(b'x');
        assert_eq!(count_leading_spaces_neon(&input32, 0), 32);
    }

    #[test]
    fn test_neon_count_leading_spaces_in_remainder() {
        // Non-space in remainder bytes (< 16)
        let mut input = vec![b' '; 20];
        input.push(b'x');
        assert_eq!(count_leading_spaces_neon(&input, 0), 20);
    }
}
