//! Portable broadword (SWAR) operations for YAML parsing.
//!
//! This module provides SIMD-like operations using pure u64 arithmetic,
//! which works on any platform without CPU-specific intrinsics.
//!
//! ## Algorithm
//!
//! Broadword (SWAR = SIMD Within A Register) processes 8 bytes at a time
//! using standard integer operations:
//! - XOR with broadcast byte to find matches (matching bytes become 0x00)
//! - Use `(x - 0x0101...) & ~x & 0x8080...` trick to detect zero bytes
//! - Extract high bits via shift and multiplication
//!
//! ## Performance
//!
//! On ARM64, broadword is competitive with but slightly slower than NEON
//! for typical YAML workloads (5-30 byte values). The break-even point
//! is around 50-64 bytes where the setup cost is amortized.
//!
//! For platforms without SIMD (WebAssembly, RISC-V, etc.), broadword
//! provides significant speedup over scalar byte-at-a-time scanning.

/// Broadcast a byte to all 8 positions in a u64.
#[inline(always)]
const fn broadcast_byte(b: u8) -> u64 {
    0x0101010101010101u64 * (b as u64)
}

/// Constants for broadword zero-byte detection.
const LO_BYTES: u64 = 0x0101010101010101u64;
const HI_BYTES: u64 = 0x8080808080808080u64;

/// Detect which bytes in `x` are zero using the classic broadword trick.
/// Returns a u64 where the high bit of each byte is set if that byte was zero.
///
/// Algorithm: `(x - 0x0101...) & ~x & 0x8080...`
/// - For zero bytes: subtraction causes borrow, setting high bit
/// - For non-zero bytes: either no borrow, or high bit was already set
#[inline(always)]
const fn has_zero_byte(x: u64) -> u64 {
    x.wrapping_sub(LO_BYTES) & !x & HI_BYTES
}

/// Find bytes equal to `target` in `x`.
/// Returns a u64 where the high bit of each byte is set if that byte equals target.
#[inline(always)]
const fn find_byte(x: u64, target: u8) -> u64 {
    has_zero_byte(x ^ broadcast_byte(target))
}

/// Extract a bitmask from the high bits of each byte in a u64.
/// Returns a u8 where bit i is set if byte i has its high bit set.
///
/// Uses multiplication trick: multiply by magic constant to gather bits.
/// After `has_zero_byte`, matching bytes have high bit set at positions 7, 15, 23, ...
/// We shift right by 7 to get bits at positions 0, 8, 16, ...
/// Then multiply by magic to gather them into bits 56-63.
#[inline(always)]
const fn extract_mask_u64(x: u64) -> u8 {
    // Shift high bits to bit 0 of each byte, then pack via multiplication
    // Magic constant: each byte position contributes to a different result bit
    // Bit at pos 0 -> bit 56, pos 8 -> bit 57, ..., pos 56 -> bit 63
    const MAGIC: u64 = 0x0102040810204080u64;
    ((x >> 7).wrapping_mul(MAGIC) >> 56) as u8
}

/// YAML character classification result using broadword operations.
/// Each field is a u8 bitmask for 8 bytes (one bit per byte position).
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct YamlCharClassBroadword {
    pub newlines: u8,
    pub colons: u8,
    pub hyphens: u8,
    pub spaces: u8,
    pub quotes_double: u8,
    pub quotes_single: u8,
    pub backslashes: u8,
    pub hash: u8,
}

#[allow(dead_code)]
impl YamlCharClassBroadword {
    /// Check if any structural character was found.
    #[inline(always)]
    pub fn has_any(&self) -> bool {
        (self.newlines
            | self.colons
            | self.hyphens
            | self.spaces
            | self.quotes_double
            | self.quotes_single
            | self.backslashes
            | self.hash)
            != 0
    }

    /// Get mask of all value terminators (for unquoted values).
    /// Terminators: newline, colon, space, hash
    #[inline(always)]
    pub fn value_terminators(&self) -> u8 {
        self.newlines | self.colons | self.spaces | self.hash
    }
}

/// Classify 8 bytes at once using pure broadword arithmetic.
///
/// This processes all 8 YAML structural character types simultaneously
/// using ~24 arithmetic operations.
///
/// Returns `None` if fewer than 8 bytes remain.
#[inline]
pub fn classify_yaml_chars_broadword(
    input: &[u8],
    offset: usize,
) -> Option<YamlCharClassBroadword> {
    if offset + 8 > input.len() {
        return None;
    }

    // Load 8 bytes as a u64
    let chunk = u64::from_le_bytes(input[offset..offset + 8].try_into().unwrap());

    // Find each character type using broadword operations
    let newlines = find_byte(chunk, b'\n');
    let colons = find_byte(chunk, b':');
    let hyphens = find_byte(chunk, b'-');
    let spaces = find_byte(chunk, b' ');
    let quotes_double = find_byte(chunk, b'"');
    let quotes_single = find_byte(chunk, b'\'');
    let backslashes = find_byte(chunk, b'\\');
    let hash = find_byte(chunk, b'#');

    Some(YamlCharClassBroadword {
        newlines: extract_mask_u64(newlines),
        colons: extract_mask_u64(colons),
        hyphens: extract_mask_u64(hyphens),
        spaces: extract_mask_u64(spaces),
        quotes_double: extract_mask_u64(quotes_double),
        quotes_single: extract_mask_u64(quotes_single),
        backslashes: extract_mask_u64(backslashes),
        hash: extract_mask_u64(hash),
    })
}

/// Classify 16 bytes using two broadword operations.
///
/// Returns combined u16 masks (low 8 bits from first chunk, high 8 from second).
/// Returns `None` if fewer than 16 bytes remain.
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct YamlCharClass16 {
    pub newlines: u16,
    pub colons: u16,
    pub hyphens: u16,
    pub spaces: u16,
    pub quotes_double: u16,
    pub quotes_single: u16,
    pub backslashes: u16,
    pub hash: u16,
}

impl YamlCharClass16 {
    /// Get mask of value terminators.
    #[inline(always)]
    pub fn value_terminators(&self) -> u16 {
        self.newlines | self.colons | self.spaces | self.hash
    }
}

/// Classify 16 bytes at once using two broadword operations.
#[inline]
pub fn classify_yaml_chars_16(input: &[u8], offset: usize) -> Option<YamlCharClass16> {
    if offset + 16 > input.len() {
        return None;
    }

    // Load two 8-byte chunks
    let chunk0 = u64::from_le_bytes(input[offset..offset + 8].try_into().unwrap());
    let chunk1 = u64::from_le_bytes(input[offset + 8..offset + 16].try_into().unwrap());

    // Process both chunks for each character type
    #[inline(always)]
    fn classify_both(c0: u64, c1: u64, target: u8) -> u16 {
        let m0 = extract_mask_u64(find_byte(c0, target)) as u16;
        let m1 = extract_mask_u64(find_byte(c1, target)) as u16;
        m0 | (m1 << 8)
    }

    Some(YamlCharClass16 {
        newlines: classify_both(chunk0, chunk1, b'\n'),
        colons: classify_both(chunk0, chunk1, b':'),
        hyphens: classify_both(chunk0, chunk1, b'-'),
        spaces: classify_both(chunk0, chunk1, b' '),
        quotes_double: classify_both(chunk0, chunk1, b'"'),
        quotes_single: classify_both(chunk0, chunk1, b'\''),
        backslashes: classify_both(chunk0, chunk1, b'\\'),
        hash: classify_both(chunk0, chunk1, b'#'),
    })
}

/// Find the next double-quote (`"`) or backslash (`\`) using broadword.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_quote_or_escape_broadword(input: &[u8], start: usize, end: usize) -> Option<usize> {
    if start >= end || start >= input.len() {
        return None;
    }
    let end = end.min(input.len());
    let data = &input[start..end];
    let len = data.len();
    let mut offset = 0;

    // Process 8 bytes at a time
    while offset + 8 <= len {
        let chunk = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let quotes = find_byte(chunk, b'"');
        let backslashes = find_byte(chunk, b'\\');
        let matches = quotes | backslashes;

        if matches != 0 {
            // Found a match - return position of first match
            return Some(offset + (matches.trailing_zeros() / 8) as usize);
        }

        offset += 8;
    }

    // Handle remaining bytes
    data[offset..]
        .iter()
        .position(|&b| b == b'"' || b == b'\\')
        .map(|pos| offset + pos)
}

/// Find the next single-quote (`'`) using broadword.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_single_quote_broadword(input: &[u8], start: usize, end: usize) -> Option<usize> {
    if start >= end || start >= input.len() {
        return None;
    }
    let end = end.min(input.len());
    let data = &input[start..end];
    let len = data.len();
    let mut offset = 0;

    // Process 8 bytes at a time
    while offset + 8 <= len {
        let chunk = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let matches = find_byte(chunk, b'\'');

        if matches != 0 {
            return Some(offset + (matches.trailing_zeros() / 8) as usize);
        }

        offset += 8;
    }

    // Handle remaining bytes
    data[offset..]
        .iter()
        .position(|&b| b == b'\'')
        .map(|pos| offset + pos)
}

/// Count leading spaces using broadword.
///
/// Returns the number of consecutive space characters starting at `start`.
#[inline]
pub fn count_leading_spaces_broadword(input: &[u8], start: usize) -> usize {
    if start >= input.len() {
        return 0;
    }

    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    // Process 8 bytes at a time
    while offset + 8 <= len {
        let chunk = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let _non_spaces = has_zero_byte(chunk ^ broadcast_byte(b' '));

        // If non_spaces is 0, all bytes are spaces - invert to check
        // Actually, has_zero_byte returns high bits set where bytes ARE equal to space
        // So we need to check if ALL bytes are spaces (all high bits set = 0x8080808080808080)
        let space_matches = find_byte(chunk, b' ');
        if space_matches != HI_BYTES {
            // Found a non-space - count spaces up to it
            // Invert the mask: spaces have high bit set, non-spaces don't
            // We want to find the first non-space
            let non_space_mask = !space_matches & HI_BYTES;
            if non_space_mask != 0 {
                return offset + (non_space_mask.trailing_zeros() / 8) as usize;
            }
        }

        offset += 8;
    }

    // Handle remaining bytes
    offset + data[offset..].iter().take_while(|&&b| b == b' ').count()
}

/// Find the next newline (`\n`) using broadword.
///
/// Returns offset from `start` to the newline, or `None` if not found.
#[inline]
pub fn find_newline_broadword(input: &[u8], start: usize) -> Option<usize> {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    // Process 8 bytes at a time
    while offset + 8 <= len {
        let chunk = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        let matches = find_byte(chunk, b'\n');

        if matches != 0 {
            return Some(offset + (matches.trailing_zeros() / 8) as usize);
        }

        offset += 8;
    }

    // Handle remaining bytes
    data[offset..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|pos| offset + pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_broadword_find_byte_basic() {
        let data = b"hello:world";
        let chunk = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let colon_mask = find_byte(chunk, b':');
        assert_ne!(colon_mask, 0);
        assert_eq!(colon_mask.trailing_zeros() / 8, 5);
    }

    #[test]
    fn test_broadword_classify_basic() {
        let input = b"key: value\n";
        let class = classify_yaml_chars_broadword(input, 0).unwrap();

        assert_eq!(class.colons, 0b00001000); // colon at position 3
        assert_eq!(class.spaces, 0b00010000); // space at position 4
    }

    #[test]
    fn test_broadword_classify_multiple() {
        let input = b": - # \"\n\\";
        let class = classify_yaml_chars_broadword(input, 0).unwrap();

        assert_ne!(class.colons, 0);
        assert_ne!(class.hyphens, 0);
        assert_ne!(class.hash, 0);
        assert_ne!(class.quotes_double, 0);
        assert_ne!(class.newlines, 0);
    }

    #[test]
    fn test_broadword_classify_16_basic() {
        let input = b"0123456789abcdef";
        let class = classify_yaml_chars_16(input, 0).unwrap();

        assert_eq!(class.colons, 0);
        assert_eq!(class.newlines, 0);
    }

    #[test]
    fn test_broadword_classify_16_with_matches() {
        let input = b"key: val\nmore: x\n";
        let class = classify_yaml_chars_16(input, 0).unwrap();

        assert!(class.colons & (1 << 3) != 0);
        assert!(class.colons & (1 << 13) != 0);
        assert!(class.newlines & (1 << 8) != 0);
    }

    #[test]
    fn test_broadword_find_quote_or_escape() {
        let input = b"hello\"world";
        assert_eq!(
            find_quote_or_escape_broadword(input, 0, input.len()),
            Some(5)
        );

        let input2 = b"hello\\world";
        assert_eq!(
            find_quote_or_escape_broadword(input2, 0, input2.len()),
            Some(5)
        );

        let input3 = b"no special chars";
        assert_eq!(
            find_quote_or_escape_broadword(input3, 0, input3.len()),
            None
        );

        // Long input
        let mut long = vec![b'a'; 100];
        long[50] = b'"';
        assert_eq!(
            find_quote_or_escape_broadword(&long, 0, long.len()),
            Some(50)
        );
    }

    #[test]
    fn test_broadword_find_single_quote() {
        let input = b"hello'world";
        assert_eq!(find_single_quote_broadword(input, 0, input.len()), Some(5));

        let input2 = b"no quotes";
        assert_eq!(find_single_quote_broadword(input2, 0, input2.len()), None);
    }

    #[test]
    fn test_broadword_count_leading_spaces() {
        assert_eq!(count_leading_spaces_broadword(b"  hello", 0), 2);
        assert_eq!(count_leading_spaces_broadword(b"    world", 0), 4);
        assert_eq!(count_leading_spaces_broadword(b"no spaces", 0), 0);

        // Long spaces
        let mut input = vec![b' '; 50];
        input.extend_from_slice(b"content");
        assert_eq!(count_leading_spaces_broadword(&input, 0), 50);
    }

    #[test]
    fn test_broadword_find_newline() {
        let input = b"hello\nworld";
        assert_eq!(find_newline_broadword(input, 0), Some(5));

        let input2 = b"no newline here";
        assert_eq!(find_newline_broadword(input2, 0), None);

        // Long input
        let mut long = vec![b'a'; 100];
        long[50] = b'\n';
        assert_eq!(find_newline_broadword(&long, 0), Some(50));
    }

    #[test]
    fn test_broadword_value_terminators() {
        let input = b"value: x";
        let class = classify_yaml_chars_broadword(input, 0).unwrap();

        let terminators = class.value_terminators();
        assert!(terminators & (1 << 5) != 0); // colon at 5
        assert!(terminators & (1 << 6) != 0); // space at 6
    }
}
