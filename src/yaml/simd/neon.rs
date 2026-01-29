//! NEON-accelerated string scanning for YAML parsing on ARM64.
//!
//! Uses 128-bit NEON vectors to process 16 bytes at a time.
//!
//! ## Broadword (SWAR) Operations
//!
//! For bulk character classification, we use pure broadword arithmetic instead of
//! NEON intrinsics. This avoids the expensive `neon_movemask` emulation which requires
//! SIMD→scalar lane extractions and multiplication tricks (~10 instructions per call).
//!
//! Broadword operations process 8 bytes at a time using u64 arithmetic:
//! - XOR with broadcast byte to find matches (matching bytes become 0x00)
//! - Use `(x - 0x0101...) & ~x & 0x8080...` trick to detect zero bytes
//! - Extract high bits via shift and mask
//!
//! This approach is ~5x faster than calling `neon_movemask` 8 times for classification.

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

// ============================================================================
// Broadword (SWAR) Character Classification
// ============================================================================
//
// These functions use pure u64 arithmetic to classify characters without
// the expensive NEON movemask emulation. Process 8 bytes at a time.

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
///
/// NOTE: Currently unused - broadword integration is disabled. See P4 analysis.
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
    /// Terminators: newline, colon, space, hash, comma (flow), brackets (flow)
    #[inline(always)]
    pub fn value_terminators(&self) -> u8 {
        self.newlines | self.colons | self.spaces | self.hash
    }
}

/// Classify 8 bytes at once using pure broadword arithmetic.
///
/// This is the core optimization: instead of 8 NEON movemask calls (~80 instructions),
/// we use ~24 arithmetic operations to classify all 8 character types.
///
/// Returns `None` if fewer than 8 bytes remain.
///
/// NOTE: Currently unused - broadword integration is disabled. See P4 analysis.
#[inline]
#[allow(dead_code)]
pub fn classify_yaml_chars_broadword(
    input: &[u8],
    offset: usize,
) -> Option<YamlCharClassBroadword> {
    if offset + 8 > input.len() {
        return None;
    }

    // Load 8 bytes as a u64 (unaligned load is fine on ARM64)
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
///
/// NOTE: Currently unused - broadword integration is disabled. See P4 analysis.
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

#[allow(dead_code)]
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

/// Find the next newline using broadword operations.
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
            // Found a newline - return position of first match
            // Each matching byte has its high bit set, so count trailing zeros / 8
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

// ============================================================================
// P4 Optimization: Anchor/Alias SIMD Parsing
// ============================================================================

/// Parse anchor/alias name using NEON SIMD to find terminator characters.
///
/// Searches for YAML anchor name terminators:
/// - Whitespace: space, tab, newline, CR
/// - Flow indicators: [ ] { } ,
/// - Colons (terminates anchor names)
///
/// Returns the position of the first terminator, or end of input.
#[inline]
pub fn parse_anchor_name_neon(input: &[u8], start: usize) -> usize {
    if start >= input.len() {
        return start;
    }

    // Use NEON for 16+ bytes
    if start + 16 <= input.len() {
        // SAFETY: NEON is mandatory on aarch64
        unsafe { parse_anchor_name_neon_impl(input, start) }
    } else {
        parse_anchor_name_scalar(input, start)
    }
}

#[target_feature(enable = "neon")]
unsafe fn parse_anchor_name_neon_impl(input: &[u8], start: usize) -> usize {
    let len = input.len();
    let mut pos = start;

    // Create comparison vectors for terminator characters
    // Note: We don't include colon here because it's only a terminator
    // if followed by whitespace. We'll check that in the scalar fallback.
    let space = vdupq_n_u8(b' ');
    let tab = vdupq_n_u8(b'\t');
    let newline = vdupq_n_u8(b'\n');
    let cr = vdupq_n_u8(b'\r');
    let lbracket = vdupq_n_u8(b'[');
    let rbracket = vdupq_n_u8(b']');
    let lbrace = vdupq_n_u8(b'{');
    let rbrace = vdupq_n_u8(b'}');
    let comma = vdupq_n_u8(b',');
    let colon = vdupq_n_u8(b':');

    // Process 16 bytes at a time
    while pos + 16 <= len {
        let chunk = vld1q_u8(input.as_ptr().add(pos));

        // Check for all terminator types (except colon which needs special handling)
        let is_space = vceqq_u8(chunk, space);
        let is_tab = vceqq_u8(chunk, tab);
        let is_newline = vceqq_u8(chunk, newline);
        let is_cr = vceqq_u8(chunk, cr);
        let is_lbracket = vceqq_u8(chunk, lbracket);
        let is_rbracket = vceqq_u8(chunk, rbracket);
        let is_lbrace = vceqq_u8(chunk, lbrace);
        let is_rbrace = vceqq_u8(chunk, rbrace);
        let is_comma = vceqq_u8(chunk, comma);
        let is_colon = vceqq_u8(chunk, colon);

        // Combine all terminator checks (whitespace and flow indicators are definite terminators)
        let ws = vorrq_u8(is_space, is_tab);
        let ws = vorrq_u8(ws, is_newline);
        let ws = vorrq_u8(ws, is_cr);

        let flow = vorrq_u8(is_lbracket, is_rbracket);
        let flow = vorrq_u8(flow, is_lbrace);
        let flow = vorrq_u8(flow, is_rbrace);
        let flow = vorrq_u8(flow, is_comma);

        let definite_terminators = vorrq_u8(ws, flow);

        // Check for definite terminators first
        let definite_mask = neon_movemask(definite_terminators);
        let colon_mask = neon_movemask(is_colon);

        if definite_mask != 0 || colon_mask != 0 {
            // Found potential terminator - need to check each position
            let combined_mask = definite_mask | colon_mask;
            let first_pos = combined_mask.trailing_zeros() as usize;

            // If it's a definite terminator, return immediately
            if (definite_mask >> first_pos) & 1 != 0 {
                return pos + first_pos;
            }

            // It's a colon - check if followed by whitespace
            let colon_pos = pos + first_pos;
            if colon_pos + 1 < len {
                let next = input[colon_pos + 1];
                if next == b' ' || next == b'\t' || next == b'\n' || next == b'\r' {
                    return colon_pos;
                }
            }

            // Colon not followed by whitespace - continue scanning from colon_pos + 1
            // Use scalar to handle the complex colon logic correctly
            return parse_anchor_name_scalar(input, colon_pos + 1);
        }

        pos += 16;
    }

    // Handle remaining bytes with scalar fallback
    parse_anchor_name_scalar(input, pos)
}

/// Scalar fallback for parsing anchor names.
fn parse_anchor_name_scalar(input: &[u8], start: usize) -> usize {
    let mut pos = start;
    while pos < input.len() {
        let b = input[pos];
        match b {
            // Stop at flow indicators, whitespace, and newlines
            b' ' | b'\t' | b'\n' | b'\r' | b'[' | b']' | b'{' | b'}' | b',' => break,
            // Colon is allowed in anchor names if not followed by whitespace
            b':' => {
                if pos + 1 < input.len() {
                    let next = input[pos + 1];
                    if next == b' ' || next == b'\t' || next == b'\n' || next == b'\r' {
                        break;
                    }
                }
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    pos
}

// ============================================================================
// Issue #87: JSON Escape Scanning for Streaming Output
// ============================================================================

/// Find the next JSON escapable character using NEON.
///
/// Searches for characters that need escaping in JSON strings:
/// - Double quote (`"`)
/// - Backslash (`\`)
/// - Control characters (bytes < 0x20)
///
/// Returns the offset from `start` to the found character, or the length
/// of the slice if no escapable character is found.
///
/// This is used for fast-path scanning in `write_json_string` during
/// YAML→JSON streaming output.
#[inline(always)]
pub fn find_json_escape_neon(bytes: &[u8], start: usize) -> usize {
    if start >= bytes.len() {
        return bytes.len();
    }

    // Use NEON for 16+ bytes, otherwise scalar is faster
    if bytes.len() - start >= 16 {
        // SAFETY: NEON is mandatory on aarch64
        unsafe { find_json_escape_neon_impl(bytes, start) }
    } else {
        find_json_escape_scalar(bytes, start)
    }
}

/// Scalar implementation for short strings
#[inline(always)]
fn find_json_escape_scalar(bytes: &[u8], start: usize) -> usize {
    for (i, &b) in bytes[start..].iter().enumerate() {
        if b == b'"' || b == b'\\' || b < 0x20 {
            return start + i;
        }
    }
    bytes.len()
}

#[target_feature(enable = "neon")]
unsafe fn find_json_escape_neon_impl(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let data = &bytes[start..];
    let data_len = data.len();
    let mut offset = 0;

    // Create comparison vectors
    let quote_vec = vdupq_n_u8(b'"');
    let backslash_vec = vdupq_n_u8(b'\\');
    let control_threshold = vdupq_n_u8(0x20);

    // Process 16-byte chunks
    while offset + 16 <= data_len {
        let chunk = vld1q_u8(data.as_ptr().add(offset));

        // Check for quote and backslash
        let quotes = vceqq_u8(chunk, quote_vec);
        let backslashes = vceqq_u8(chunk, backslash_vec);

        // Check for control characters (bytes < 0x20)
        let controls = vcltq_u8(chunk, control_threshold);

        // Combine all matches
        let matches = vorrq_u8(vorrq_u8(quotes, backslashes), controls);

        // Extract bitmask
        let mask = neon_movemask(matches);

        if mask != 0 {
            // Found a match - return absolute position
            return start + offset + mask.trailing_zeros() as usize;
        }

        offset += 16;
    }

    // Handle remaining bytes with scalar fallback
    for i in offset..data_len {
        let b = data[i];
        if b == b'"' || b == b'\\' || b < 0x20 {
            return start + i;
        }
    }

    // No escapable character found
    len
}

// ============================================================================
// P2.7 Optimization: Block Scalar SIMD Parsing
// ============================================================================

/// Find the end of a block scalar using NEON SIMD.
///
/// Scans for newlines and checks indentation on each line.
/// Returns the position where the block ends (start of line with insufficient indent),
/// or input.len() if EOF is reached.
#[inline]
pub fn find_block_scalar_end_neon(input: &[u8], start: usize, min_indent: usize) -> usize {
    if start >= input.len() {
        return input.len();
    }

    // SAFETY: NEON is mandatory on aarch64
    unsafe { find_block_scalar_end_neon_impl(input, start, min_indent) }
}

#[target_feature(enable = "neon")]
unsafe fn find_block_scalar_end_neon_impl(input: &[u8], start: usize, min_indent: usize) -> usize {
    let newline_vec = vdupq_n_u8(b'\n');
    let space_vec = vdupq_n_u8(b' ');

    let mut pos = start;

    // Process in 16-byte chunks, looking for newlines
    while pos + 16 < input.len() {
        let chunk = vld1q_u8(input.as_ptr().add(pos));
        let nl_matches = vceqq_u8(chunk, newline_vec);
        let mut nl_mask = neon_movemask(nl_matches);

        if nl_mask != 0 {
            // Found newline(s) in this chunk - check indentation after each
            while nl_mask != 0 {
                let offset = nl_mask.trailing_zeros() as usize;
                let line_start = pos + offset + 1; // Position after newline

                if line_start >= input.len() {
                    return input.len(); // EOF
                }

                // Count leading spaces on next line
                let mut indent = 0;
                let remaining = input.len() - line_start;

                // Use SIMD to count spaces if we have 16+ bytes
                if remaining >= 16 {
                    let next_chunk = vld1q_u8(input.as_ptr().add(line_start));
                    let space_matches = vceqq_u8(next_chunk, space_vec);
                    let space_mask = neon_movemask(space_matches);

                    if space_mask != 0xFFFF {
                        indent = (!space_mask).trailing_zeros() as usize;
                    } else {
                        indent = 16;
                        // Continue counting if all 16 were spaces
                        let mut check_pos = line_start + 16;
                        while check_pos < input.len() && input[check_pos] == b' ' {
                            indent += 1;
                            check_pos += 1;
                        }
                    }
                } else {
                    // Less than 16 bytes remaining, count scalar
                    while line_start + indent < input.len() && input[line_start + indent] == b' ' {
                        indent += 1;
                    }
                }

                // Check if this line has sufficient indent
                if line_start + indent < input.len() {
                    let next_char = input[line_start + indent];
                    if next_char != b'\n' && next_char != b'\r' && indent < min_indent {
                        // Content at insufficient indent - block ends here
                        return line_start;
                    }
                }

                // Clear this bit and check next newline
                nl_mask &= nl_mask - 1;
            }
        }

        pos += 16;
    }

    // Handle remainder with scalar code
    find_block_scalar_end_scalar(input, pos, min_indent)
}

/// Scalar fallback for find_block_scalar_end.
fn find_block_scalar_end_scalar(input: &[u8], start: usize, min_indent: usize) -> usize {
    let mut pos = start;

    while pos < input.len() {
        if input[pos] == b'\n' {
            let line_start = pos + 1;

            if line_start >= input.len() {
                return input.len();
            }

            // Count leading spaces
            let mut indent = 0;
            while line_start + indent < input.len() && input[line_start + indent] == b' ' {
                indent += 1;
            }

            // Check if this line has content at insufficient indent
            if line_start + indent < input.len() {
                let next_char = input[line_start + indent];
                if next_char != b'\n' && next_char != b'\r' && indent < min_indent {
                    return line_start;
                }
            }
        }
        pos += 1;
    }

    input.len()
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

    // ========================================================================
    // Broadword tests
    // ========================================================================

    #[test]
    fn test_broadword_find_byte_basic() {
        // Test the core find_byte function
        let data = b"hello:world";
        let chunk = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let colon_mask = find_byte(chunk, b':');
        // Colon is at position 5, so bit 5*8+7 = 47 should be set
        assert_ne!(colon_mask, 0);
        assert_eq!(colon_mask.trailing_zeros() / 8, 5);
    }

    #[test]
    fn test_broadword_classify_basic() {
        let input = b"key: value\n";
        let class = classify_yaml_chars_broadword(input, 0).unwrap();

        // Check specific character positions
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

        // No special characters
        assert_eq!(class.colons, 0);
        assert_eq!(class.newlines, 0);
    }

    #[test]
    fn test_broadword_classify_16_with_matches() {
        let input = b"key: val\nmore: x\n";
        let class = classify_yaml_chars_16(input, 0).unwrap();

        // Colon at position 3 and 13
        assert!(class.colons & (1 << 3) != 0);
        assert!(class.colons & (1 << 13) != 0);

        // Newline at position 8
        assert!(class.newlines & (1 << 8) != 0);
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
    fn test_broadword_value_terminators() {
        let input = b"value: x";
        let class = classify_yaml_chars_broadword(input, 0).unwrap();

        let terminators = class.value_terminators();
        // Should have colon at 5 and space at 6
        assert!(terminators & (1 << 5) != 0);
        assert!(terminators & (1 << 6) != 0);
    }

    // ========================================================================
    // P4: Anchor/Alias NEON tests
    // ========================================================================

    #[test]
    fn test_parse_anchor_name_basic() {
        // Simple anchor name terminated by space
        assert_eq!(parse_anchor_name_neon(b"anchor_name value", 0), 11);

        // Colon NOT followed by whitespace - NOT a terminator (colon allowed in anchor names)
        assert_eq!(parse_anchor_name_neon(b"anchor:value", 0), 12);

        // Colon followed by space - IS a terminator
        assert_eq!(parse_anchor_name_neon(b"anchor: value", 0), 6);

        // Colon followed by newline - IS a terminator
        assert_eq!(parse_anchor_name_neon(b"anchor:\nvalue", 0), 6);

        // Terminated by newline
        assert_eq!(parse_anchor_name_neon(b"anchor\nvalue", 0), 6);

        // Terminated by tab
        assert_eq!(parse_anchor_name_neon(b"anchor\tvalue", 0), 6);
    }

    #[test]
    fn test_parse_anchor_name_flow_indicators() {
        // Terminated by flow indicators
        assert_eq!(parse_anchor_name_neon(b"anchor[0]", 0), 6);
        assert_eq!(parse_anchor_name_neon(b"anchor]end", 0), 6);
        assert_eq!(parse_anchor_name_neon(b"anchor{key}", 0), 6);
        assert_eq!(parse_anchor_name_neon(b"anchor}end", 0), 6);
        assert_eq!(parse_anchor_name_neon(b"anchor,next", 0), 6);
    }

    #[test]
    fn test_parse_anchor_name_long() {
        // Long anchor name (>16 bytes to exercise SIMD path)
        let mut input = vec![b'a'; 50];
        input.push(b' ');
        input.extend_from_slice(b"value");
        assert_eq!(parse_anchor_name_neon(&input, 0), 50);
    }

    #[test]
    fn test_parse_anchor_name_no_terminator() {
        // No terminator - should return end of input
        assert_eq!(parse_anchor_name_neon(b"anchor_name", 0), 11);
    }

    #[test]
    fn test_parse_anchor_name_with_offset() {
        // Start from offset
        assert_eq!(parse_anchor_name_neon(b"&anchor_name value", 1), 12);
    }

    // ========================================================================
    // P2.7: Block Scalar NEON tests
    // ========================================================================

    #[test]
    fn test_find_block_scalar_end_basic() {
        // Block scalar with proper indentation
        let input = b"|\n  line1\n  line2\nnext_key:";
        // min_indent=2, so "next_key:" (indent=0) should terminate
        let result = find_block_scalar_end_neon(input, 2, 2);
        assert_eq!(result, 18); // Position of 'n' in "next_key"
    }

    #[test]
    fn test_find_block_scalar_end_eof() {
        // Block scalar that ends at EOF
        let input = b"|\n  line1\n  line2";
        let result = find_block_scalar_end_neon(input, 2, 2);
        assert_eq!(result, input.len());
    }

    #[test]
    fn test_find_block_scalar_end_long() {
        // Long block scalar (>16 bytes per line to exercise SIMD path)
        let mut input = b"|\n".to_vec();
        for _ in 0..5 {
            input.extend_from_slice(b"  ");
            input.extend_from_slice(&[b'x'; 20]);
            input.push(b'\n');
        }
        input.extend_from_slice(b"next:");

        let result = find_block_scalar_end_neon(&input, 2, 2);
        // Should find "next:" at the end
        assert_eq!(result, input.len() - 5);
    }

    #[test]
    fn test_find_block_scalar_end_empty_lines() {
        // Empty lines should be ignored
        let input = b"|\n  line1\n\n  line2\nnext:";
        let result = find_block_scalar_end_neon(input, 2, 2);
        assert_eq!(result, 19); // Position of 'n' in "next:"
    }

    #[test]
    fn test_find_block_scalar_matches_scalar() {
        // Compare NEON vs scalar for various inputs
        let test_cases: &[(&[u8], usize)] = &[
            (b"|\n  line1\n  line2\nnext:", 2),
            (b"|\n    deep\n    indent\nshallow:", 4),
            (b"|\n  a\n  b\n  c\n", 2),
        ];

        for &(input, min_indent) in test_cases {
            let neon_result = find_block_scalar_end_neon(input, 2, min_indent);
            let scalar_result = find_block_scalar_end_scalar(input, 2, min_indent);
            assert_eq!(
                neon_result,
                scalar_result,
                "Mismatch for input {:?} with min_indent={}",
                String::from_utf8_lossy(input),
                min_indent
            );
        }
    }

    // ========================================================================
    // Issue #87: JSON Escape Scanning NEON tests
    // ========================================================================

    #[test]
    fn test_find_json_escape_quote() {
        let input = b"hello\"world";
        assert_eq!(find_json_escape_neon(input, 0), 5);
    }

    #[test]
    fn test_find_json_escape_backslash() {
        let input = b"hello\\world";
        assert_eq!(find_json_escape_neon(input, 0), 5);
    }

    #[test]
    fn test_find_json_escape_control_char() {
        // Tab is 0x09 (< 0x20)
        let input = b"hello\tworld";
        assert_eq!(find_json_escape_neon(input, 0), 5);

        // Newline is 0x0A
        let input2 = b"hello\nworld";
        assert_eq!(find_json_escape_neon(input2, 0), 5);

        // Null byte
        let input3 = b"hello\x00world";
        assert_eq!(find_json_escape_neon(input3, 0), 5);
    }

    #[test]
    fn test_find_json_escape_no_escape() {
        let input = b"hello world";
        assert_eq!(find_json_escape_neon(input, 0), input.len());
    }

    #[test]
    fn test_find_json_escape_long_string() {
        // Test with > 16 bytes to exercise SIMD path
        let mut input = vec![b'a'; 100];
        input[50] = b'"';
        assert_eq!(find_json_escape_neon(&input, 0), 50);
    }

    #[test]
    fn test_find_json_escape_at_chunk_boundary() {
        // Escape at exactly byte 16 (second chunk)
        let mut input = vec![b'a'; 32];
        input[16] = b'"';
        assert_eq!(find_json_escape_neon(&input, 0), 16);
    }

    #[test]
    fn test_find_json_escape_in_remainder() {
        // Escape in the remainder bytes (< 16)
        let mut input = vec![b'a'; 20];
        input[18] = b'\\';
        assert_eq!(find_json_escape_neon(&input, 0), 18);
    }

    #[test]
    fn test_find_json_escape_with_offset() {
        let input = b"abc\"def\"ghi";
        assert_eq!(find_json_escape_neon(input, 0), 3);
        assert_eq!(find_json_escape_neon(input, 4), 7);
    }

    #[test]
    fn test_find_json_escape_control_chars_throughout() {
        // Test various control chars at different positions
        for ctrl in 0u8..0x20 {
            let mut input = vec![b'x'; 50];
            input[25] = ctrl;
            assert_eq!(
                find_json_escape_neon(&input, 0),
                25,
                "Failed for control char 0x{:02x}",
                ctrl
            );
        }
    }

    #[test]
    fn test_find_json_escape_empty() {
        assert_eq!(find_json_escape_neon(b"", 0), 0);
    }

    #[test]
    fn test_find_json_escape_start_past_end() {
        let input = b"hello";
        assert_eq!(find_json_escape_neon(input, 10), input.len());
    }

    /// Scalar reference implementation for comparison testing
    fn find_json_escape_scalar(bytes: &[u8], start: usize) -> usize {
        for (i, &b) in bytes[start..].iter().enumerate() {
            if b == b'"' || b == b'\\' || b < 0x20 {
                return start + i;
            }
        }
        bytes.len()
    }

    #[test]
    fn test_find_json_escape_matches_scalar() {
        let test_cases: &[&[u8]] = &[
            b"",
            b"\"",
            b"\\",
            b"\t",
            b"\n",
            b"\r",
            b"\x00",
            b"no escape chars here",
            b"escape at end\"",
            b"\"escape at start",
            b"has\\backslash",
            b"has\ttab",
            b"has\nnewline",
            b"multiple \"escapes\" here\\",
            // Long strings
            &[b'x'; 100],
        ];

        for &input in test_cases {
            let scalar = find_json_escape_scalar(input, 0);
            let neon = find_json_escape_neon(input, 0);
            assert_eq!(
                scalar,
                neon,
                "Mismatch for {:?}: scalar={}, neon={}",
                String::from_utf8_lossy(input),
                scalar,
                neon
            );
        }

        // Test with various offsets
        let input = b"abc\"def\\ghi\tjkl";
        for start in 0..input.len() {
            let scalar = find_json_escape_scalar(input, start);
            let neon = find_json_escape_neon(input, start);
            assert_eq!(
                scalar, neon,
                "Mismatch at offset {}: scalar={}, neon={}",
                start, scalar, neon
            );
        }
    }
}
