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
#[allow(dead_code)] // STYLE-0005: broadword fallback classifier; unused when SIMD is active
pub struct YamlCharClassBroadword {
    pub newlines: u8,
    /// Mask of bytes that are '\r' — a YAML 1.2 §5.4 line break in its own
    /// right, so a value terminator just like '\n' (#324).
    pub carriage_returns: u8,
    pub colons: u8,
    pub hyphens: u8,
    pub spaces: u8,
    pub quotes_double: u8,
    pub quotes_single: u8,
    pub backslashes: u8,
    pub hash: u8,
}

#[allow(dead_code)] // STYLE-0005: broadword fallback classifier; unused when SIMD is active
impl YamlCharClassBroadword {
    /// Check if any structural character was found.
    #[inline(always)]
    pub fn has_any(&self) -> bool {
        (self.newlines
            | self.carriage_returns
            | self.colons
            | self.hyphens
            | self.spaces
            | self.quotes_double
            | self.quotes_single
            | self.backslashes
            | self.hash)
            != 0
    }

    /// Bytes at which a plain (unquoted) scalar scan must stop and re-examine.
    ///
    /// See [`YamlCharClass16::plain_scalar_terminators`].
    #[inline(always)]
    pub fn plain_scalar_terminators(&self) -> u8 {
        self.newlines | self.carriage_returns | self.colons | self.hash
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
    let carriage_returns = find_byte(chunk, b'\r');
    let colons = find_byte(chunk, b':');
    let hyphens = find_byte(chunk, b'-');
    let spaces = find_byte(chunk, b' ');
    let quotes_double = find_byte(chunk, b'"');
    let quotes_single = find_byte(chunk, b'\'');
    let backslashes = find_byte(chunk, b'\\');
    let hash = find_byte(chunk, b'#');

    Some(YamlCharClassBroadword {
        newlines: extract_mask_u64(newlines),
        carriage_returns: extract_mask_u64(carriage_returns),
        colons: extract_mask_u64(colons),
        hyphens: extract_mask_u64(hyphens),
        spaces: extract_mask_u64(spaces),
        quotes_double: extract_mask_u64(quotes_double),
        quotes_single: extract_mask_u64(quotes_single),
        backslashes: extract_mask_u64(backslashes),
        hash: extract_mask_u64(hash),
    })
}

/// Classification of a 16-byte chunk: one `u16` mask per character type, with
/// bit `i` set iff byte `i` of the chunk is that character.
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)] // STYLE-0005: broadword fallback classifier; unused when SIMD is active
pub struct YamlCharClass16 {
    pub newlines: u16,
    /// Mask of bytes that are '\r' — see [`YamlCharClassBroadword`] (#324).
    pub carriage_returns: u16,
    pub colons: u16,
    pub hyphens: u16,
    pub spaces: u16,
    pub quotes_double: u16,
    pub quotes_single: u16,
    pub backslashes: u16,
    pub hash: u16,
}

impl YamlCharClass16 {
    /// Bytes at which a plain (unquoted) scalar scan must stop and re-examine.
    ///
    /// The scan may stop early and lose nothing but speed, so this only has to
    /// be a *superset* of what the parser's byte loop breaks on: `\n`, `\r`
    /// (YAML 1.2 §5.4 makes a lone CR a line break too — #324), `#`, and `:`.
    /// The loop re-checks the context-sensitive pair itself — `#` opens a
    /// comment only after whitespace, `:` ends a key only before whitespace —
    /// so the mask does not model that.
    ///
    /// Notably **not** `spaces`: a plain scalar may contain them (`key: hello
    /// world`), so stopping at each one only costs a re-entry into the byte
    /// loop. Until #185 this mask included them and the x86 classifier's did
    /// not, which meant re-enabling the ARM fast path would have silently
    /// surrendered most of its skipping to prose-like values.
    #[inline(always)]
    pub fn plain_scalar_terminators(&self) -> u16 {
        self.newlines | self.carriage_returns | self.colons | self.hash
    }
}

/// Classify 16 bytes at once using two broadword operations.
///
/// Returns combined u16 masks (low 8 bits from the first chunk, high 8 from the
/// second), or `None` if fewer than 16 bytes remain.
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
        carriage_returns: classify_both(chunk0, chunk1, b'\r'),
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
#[allow(dead_code)] // STYLE-0005: broadword scan kernel; ARM64 dispatches to the NEON one instead
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
#[allow(dead_code)] // STYLE-0005: broadword scan kernel; ARM64 dispatches to the NEON one instead
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
#[allow(dead_code)] // STYLE-0005: broadword scan kernel; ARM64 dispatches to the NEON one instead
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

        // `find_byte` sets the high bit of every byte that IS a space, so all
        // eight being spaces means exactly `HI_BYTES`.
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
    fn test_broadword_find_newline_in_remainder() {
        // Newline in bytes after last 8-byte chunk
        let mut input = vec![b'a'; 10];
        input[9] = b'\n';
        assert_eq!(find_newline_broadword(&input, 0), Some(9));
    }

    #[test]
    fn plain_scalar_terminators_stop_at_structure_but_not_at_spaces() {
        let input = b"value: x";
        let class = classify_yaml_chars_broadword(input, 0).unwrap();

        let terminators = class.plain_scalar_terminators();
        assert!(terminators & (1 << 5) != 0, "colon at 5 must terminate");
        assert_eq!(
            terminators & (1 << 6),
            0,
            "space at 6 must NOT terminate: a plain scalar may contain spaces, \
             and stopping there only costs skip distance (#185)"
        );
        // The space is still classified — it is the terminator set that excludes it.
        assert!(class.spaces & (1 << 6) != 0);
    }

    /// The broadword classifiers are the ARM64 fast path's eyes, but that path
    /// is disabled, so nothing on a live route exercises their accessors. They
    /// still have to agree that a CR terminates a value — a classifier that
    /// quietly omits it is what let a lone CR swallow a whole document (#324).
    #[test]
    fn broadword_classifiers_treat_cr_as_a_value_terminator() {
        let input = b"abcdefg\rhijklmno";
        let class = classify_yaml_chars_broadword(input, 0).expect("8 bytes available");
        assert_eq!(class.carriage_returns, 1 << 7, "CR is at offset 7");
        assert!(class.has_any());
        assert!(
            class.plain_scalar_terminators() & (1 << 7) != 0,
            "CR must terminate an unquoted value"
        );

        let class16 = classify_yaml_chars_16(input, 0).expect("16 bytes available");
        assert_eq!(class16.carriage_returns, 1 << 7);
        assert!(class16.plain_scalar_terminators() & (1 << 7) != 0);

        // A chunk with no CR reports none, so the mask is not just always set.
        let plain = b"abcdefghijklmnop";
        let none = classify_yaml_chars_broadword(plain, 0).expect("8 bytes available");
        assert_eq!(none.carriage_returns, 0);
        assert!(!none.has_any(), "no structural bytes in {plain:?}");
        assert_eq!(none.plain_scalar_terminators(), 0);
    }

    /// Both widths must agree on the set, or the 8- and 16-byte paths would
    /// disagree about where a scalar ends. Asserted as an exact channel list so
    /// adding a channel to one and not the other fails here (#185).
    #[test]
    fn both_classifier_widths_agree_on_the_terminator_set() {
        let input = b"a:b\nc\rd#e f:g\nhi";
        let c8 = classify_yaml_chars_broadword(input, 0).expect("8 bytes available");
        let c16 = classify_yaml_chars_16(input, 0).expect("16 bytes available");

        assert_eq!(
            c8.plain_scalar_terminators(),
            c8.newlines | c8.carriage_returns | c8.colons | c8.hash
        );
        assert_eq!(
            c16.plain_scalar_terminators(),
            c16.newlines | c16.carriage_returns | c16.colons | c16.hash
        );
        // The 16-byte mask's low half is the 8-byte mask.
        assert_eq!(
            c16.plain_scalar_terminators() as u8,
            c8.plain_scalar_terminators()
        );
    }
}
