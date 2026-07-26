#![allow(unsafe_code)] // ARM64 NEON SIMD intrinsics
//! NEON-accelerated string scanning for YAML parsing on ARM64.
//!
//! Uses 128-bit NEON vectors to process 16 bytes at a time.
//!
//! ## What is *not* here
//!
//! Bulk character classification uses pure broadword (SWAR) arithmetic rather
//! than NEON, because `neon_movemask` emulation costs a SIMD→scalar lane
//! extraction plus a multiply per channel (~10 instructions), and a classifier
//! needs nine of them. Newline scanning is broadword for the same reason. Both
//! live in [`super::broadword`], which ARM64 compiles and dispatches to
//! alongside this module — see the `mod broadword` gate in [`super`].
//!
//! Until #185 this file carried its own copy of that broadword layer. The two
//! copies could never be compiled together, so they drifted: the copy here
//! still documented terminators (`,`, `[`, `]`) it never classified.

use core::arch::aarch64::*;

use super::scalar::{find_block_scalar_end_scalar, parse_anchor_name_scalar};

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
    let carriage_return_vec = vdupq_n_u8(b'\r');
    let space_vec = vdupq_n_u8(b' ');

    let mut pos = start;

    // Process in 16-byte chunks, looking for newlines
    while pos + 16 < input.len() {
        let chunk = vld1q_u8(input.as_ptr().add(pos));
        // Match either line-break byte (#324). A CRLF sets both bits; the CR's
        // "next line" is the LF itself, which the empty-line guard below skips.
        let nl_matches = vorrq_u8(
            vceqq_u8(chunk, newline_vec),
            vceqq_u8(chunk, carriage_return_vec),
        );
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

    // Broadword tests live with the broadword code in `super::broadword` (#185).

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
}
