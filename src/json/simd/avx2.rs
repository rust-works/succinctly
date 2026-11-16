#![allow(unsafe_code)] // x86_64 AVX2 SIMD intrinsics
//! AVX2-accelerated JSON semi-indexing for x86_64.
//!
//! Processes 32 bytes at a time using x86_64 AVX2 SIMD instructions.
//! AVX2 is available on Intel Haswell (2013+) and AMD Excavator (2015+).

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use super::step::BlockMasks;
use crate::json::simple::{SemiIndex as SimpleSemiIndex, State as SimpleState};
use crate::json::standard::SemiIndex;
use crate::json::BitWriter;

/// ASCII byte constants
const DOUBLE_QUOTE: i8 = b'"' as i8;
const BACKSLASH: i8 = b'\\' as i8;
const OPEN_BRACE: i8 = b'{' as i8;
const CLOSE_BRACE: i8 = b'}' as i8;
const OPEN_BRACKET: i8 = b'[' as i8;
const CLOSE_BRACKET: i8 = b']' as i8;
const COMMA: i8 = b',' as i8;
const COLON: i8 = b':' as i8;

/// Character classification results for a 32-byte chunk.
#[derive(Debug, Clone, Copy)]
struct CharClass {
    /// Mask of bytes that are '"'
    quotes: u32,
    /// Mask of bytes that are '\'
    backslashes: u32,
    /// Mask of bytes that are '{' or '['
    opens: u32,
    /// Mask of bytes that are '}' or ']'
    closes: u32,
    /// Mask of bytes that are ',' or ':'
    delims: u32,
}

/// Classify 32 bytes at once using AVX2.
#[inline]
#[target_feature(enable = "avx2")]
#[cfg(target_arch = "x86_64")]
unsafe fn classify_chars(chunk: __m256i) -> CharClass {
    // Create comparison vectors for each character
    let v_quote = _mm256_set1_epi8(DOUBLE_QUOTE);
    let v_backslash = _mm256_set1_epi8(BACKSLASH);
    let v_open_brace = _mm256_set1_epi8(OPEN_BRACE);
    let v_close_brace = _mm256_set1_epi8(CLOSE_BRACE);
    let v_open_bracket = _mm256_set1_epi8(OPEN_BRACKET);
    let v_close_bracket = _mm256_set1_epi8(CLOSE_BRACKET);
    let v_comma = _mm256_set1_epi8(COMMA);
    let v_colon = _mm256_set1_epi8(COLON);

    // Compare and get masks using _mm256_cmpeq_epi8
    let eq_quote = _mm256_cmpeq_epi8(chunk, v_quote);
    let eq_backslash = _mm256_cmpeq_epi8(chunk, v_backslash);
    let eq_open_brace = _mm256_cmpeq_epi8(chunk, v_open_brace);
    let eq_close_brace = _mm256_cmpeq_epi8(chunk, v_close_brace);
    let eq_open_bracket = _mm256_cmpeq_epi8(chunk, v_open_bracket);
    let eq_close_bracket = _mm256_cmpeq_epi8(chunk, v_close_bracket);
    let eq_comma = _mm256_cmpeq_epi8(chunk, v_comma);
    let eq_colon = _mm256_cmpeq_epi8(chunk, v_colon);

    // Combine opens and closes
    let opens = _mm256_or_si256(eq_open_brace, eq_open_bracket);
    let closes = _mm256_or_si256(eq_close_brace, eq_close_bracket);
    let delims = _mm256_or_si256(eq_comma, eq_colon);

    // Convert to bitmasks using _mm256_movemask_epi8
    CharClass {
        quotes: _mm256_movemask_epi8(eq_quote) as u32,
        backslashes: _mm256_movemask_epi8(eq_backslash) as u32,
        opens: _mm256_movemask_epi8(opens) as u32,
        closes: _mm256_movemask_epi8(closes) as u32,
        delims: _mm256_movemask_epi8(delims) as u32,
    }
}

/// The AVX2 answers to the shared kernel's per-block questions: one 32-byte
/// `movemask` each.
struct Avx2;

impl BlockMasks for Avx2 {
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn specials(block: &[u8; 32]) -> u32 {
        unsafe {
            let chunk = _mm256_loadu_si256(block.as_ptr().cast::<__m256i>());
            let specials = _mm256_or_si256(
                _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(DOUBLE_QUOTE)),
                _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(BACKSLASH)),
            );
            _mm256_movemask_epi8(specials) as u32
        }
    }

    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn phi_masks(fields: &[u8; 32]) -> [u32; 3] {
        unsafe {
            // `movemask` reads bit 7 of each byte, so shift the open (bit 6) and
            // close (bit 5) bits up to it; a 16-bit lane shift is fine because
            // only each byte's own lower bits move into its bit 7.
            let v = _mm256_loadu_si256(fields.as_ptr().cast::<__m256i>());
            [
                _mm256_movemask_epi8(v) as u32,
                _mm256_movemask_epi8(_mm256_slli_epi16::<1>(v)) as u32,
                _mm256_movemask_epi8(_mm256_slli_epi16::<2>(v)) as u32,
            ]
        }
    }
}

/// Build a semi-index from JSON bytes using SIMD-accelerated Standard Cursor algorithm.
///
/// The state machine runs one `STEP_TABLE` load and one shift per byte, with no
/// branch on the input; AVX2 then pulls each 32-byte block's IB, BP-open and
/// BP-close bits out of the stored fields with three `movemask`s, and skips a block
/// that lies wholly inside a string without stepping it. The kernel is
/// [`super::step`]'s, shared with the SSE2 builder. The per-byte `match` it
/// replaced (#7299) kept both writers' state in memory and ran about three
/// data-dependent branches per byte. It was 1.2x (all-backslash strings) to about
/// 50x (long strings) slower, and its speed on a Ryzen 9 7950X moved by up to 10% with
/// where unrelated changes placed it; this kernel's moves by 1-4%.
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    // SAFETY: Caller must ensure AVX2 is available
    unsafe { build_semi_index_standard_avx2(json) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn build_semi_index_standard_avx2(json: &[u8]) -> SemiIndex {
    // SAFETY: this function's own `avx2` feature is `Avx2`'s.
    unsafe { super::step::build_semi_index_standard::<Avx2>(json) }
}

// ============================================================================
// Simple Cursor SIMD Implementation
// ============================================================================

/// Process a 32-byte chunk for simple cursor and update IB/BP writers.
/// Returns the new state after processing all bytes.
///
/// Simple cursor differences from standard:
/// - 3 states: InJson, InString, InEscape (no InValue)
/// - IB marks all structural chars: { } [ ] : ,
/// - BP encoding: { or [ → 11, } or ] → 00, : or , → 01
#[inline]
fn process_chunk_simple(
    class: CharClass,
    mut state: SimpleState,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
    bytes: &[u8],
) -> SimpleState {
    for i in 0..bytes.len().min(32) {
        let bit = 1u32 << i;

        let is_quote = (class.quotes & bit) != 0;
        let is_backslash = (class.backslashes & bit) != 0;
        let is_open = (class.opens & bit) != 0;
        let is_close = (class.closes & bit) != 0;
        let is_delim = (class.delims & bit) != 0;

        match state {
            SimpleState::InJson => {
                if is_open {
                    // { or [: write BP=11, IB=1
                    bp.write_1();
                    bp.write_1();
                    ib.write_1();
                } else if is_close {
                    // } or ]: write BP=00, IB=1
                    bp.write_0();
                    bp.write_0();
                    ib.write_1();
                } else if is_delim {
                    // , or :: write BP=01, IB=1
                    bp.write_0();
                    bp.write_1();
                    ib.write_1();
                } else if is_quote {
                    // Start of string: IB=0, transition to InString
                    ib.write_0();
                    state = SimpleState::InString;
                } else {
                    // Whitespace or other: IB=0
                    ib.write_0();
                }
            }
            SimpleState::InString => {
                // Always IB=0 inside strings
                ib.write_0();
                if is_quote {
                    state = SimpleState::InJson;
                } else if is_backslash {
                    state = SimpleState::InEscape;
                }
            }
            SimpleState::InEscape => {
                // Escaped character: IB=0, return to InString
                ib.write_0();
                state = SimpleState::InString;
            }
        }
    }

    state
}

/// Build a semi-index from JSON bytes using SIMD-accelerated Simple Cursor algorithm.
///
/// Processes 32 bytes at a time using x86_64 AVX2 instructions for character
/// classification, then processes the state machine transitions.
#[cfg(target_arch = "x86_64")]
pub fn build_semi_index_simple(json: &[u8]) -> SimpleSemiIndex {
    // SAFETY: Caller must ensure AVX2 is available
    unsafe { build_semi_index_simple_avx2(json) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn build_semi_index_simple_avx2(json: &[u8]) -> SimpleSemiIndex {
    unsafe {
        let word_capacity = json.len().div_ceil(64);
        let mut ib = BitWriter::with_capacity(word_capacity);
        let mut bp = BitWriter::with_capacity(word_capacity * 2);
        let mut state = SimpleState::InJson;

        let mut offset = 0;

        // Process 32-byte chunks
        while offset + 32 <= json.len() {
            let chunk = _mm256_loadu_si256(json.as_ptr().add(offset).cast::<__m256i>());
            let class = classify_chars(chunk);
            state =
                process_chunk_simple(class, state, &mut ib, &mut bp, &json[offset..offset + 32]);
            offset += 32;
        }

        // Process remaining bytes (less than 32)
        if offset < json.len() {
            // Pad with zeros and process
            let mut padded = [0u8; 32];
            let remaining = json.len() - offset;
            padded[..remaining].copy_from_slice(&json[offset..]);

            let chunk = _mm256_loadu_si256(padded.as_ptr().cast::<__m256i>());
            let class = classify_chars(chunk);
            state = process_chunk_simple(class, state, &mut ib, &mut bp, &json[offset..]);
        }

        SimpleSemiIndex {
            state,
            ib: ib.finish(),
            bp: bp.finish(),
        }
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    /// Extract bit at position i from a word vector (LSB-first within each word)
    fn get_bit(words: &[u64], i: usize) -> bool {
        let word_idx = i / 64;
        let bit_idx = i % 64;
        if word_idx < words.len() {
            (words[word_idx] >> bit_idx) & 1 == 1
        } else {
            false
        }
    }

    /// Convert bit vector to string of '0' and '1' for first n bits
    fn bits_to_string(words: &[u64], n: usize) -> String {
        (0..n)
            .map(|i| if get_bit(words, i) { '1' } else { '0' })
            .collect()
    }

    #[test]
    fn test_classify_chars() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        unsafe {
            let input = br#"{"hello":123}               "#; // 32 bytes
            let chunk = _mm256_loadu_si256(input.as_ptr().cast::<__m256i>());
            let class = classify_chars(chunk);

            // Position 0 is '{', position 12 is '}'
            assert_ne!(class.opens & (1 << 0), 0, "'{{' at position 0");
            assert_ne!(class.closes & (1 << 12), 0, "'}}' at position 12");

            // Position 1 and 7 are '"'
            assert_ne!(class.quotes & (1 << 1), 0, "'\"' at position 1");
            assert_ne!(class.quotes & (1 << 7), 0, "'\"' at position 7");

            // Position 8 is ':'
            assert_ne!(class.delims & (1 << 8), 0, "':' at position 8");
        }
    }

    #[test]
    fn test_avx2_matches_scalar_empty_object() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = b"{}";
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len())
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_avx2_matches_scalar_simple_object() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = br#"{"a":"b"}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_avx2_matches_scalar_long_input() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        // Test with input > 32 bytes to exercise chunk processing
        let json = br#"{"name":"value","number":12345,"array":[1,2,3]}"#;
        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for long input"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_avx2_matches_scalar_exact_32_bytes() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        // Exactly 32 bytes - one full chunk
        let json = br#"{"a":"b","c":"d","e":"fghijklm"}"#;
        assert_eq!(json.len(), 32, "Test JSON should be exactly 32 bytes");

        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 32-byte input"
        );
    }

    #[test]
    fn test_avx2_matches_scalar_64_bytes() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        // 64 bytes - two full chunks
        let json = br#"{"k1":"v1","k2":"v2","k3":"v3","k4":"v4","k5":"val5","k6":"vvv"}"#;
        assert_eq!(json.len(), 64, "Test JSON should be exactly 64 bytes");

        let simd_result = build_semi_index_standard(json);
        let scalar_result = crate::json::standard::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 64-byte input"
        );
    }

    /// The branch-free kernel (#7299) against the scalar state machine: IB, BP and
    /// the final state, for every length from 0 to 300 over three alphabets.
    #[test]
    fn test_avx2_branch_free_matches_scalar_on_random_input() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        // Few quotes, so many 32-byte blocks lie wholly inside a string and take
        // the skip; backslashes so an escape can straddle a block edge.
        let long_strings: Vec<u8> = {
            let mut v = vec![b'"', b'\\'];
            v.extend(std::iter::repeat_n(b'x', 60));
            v
        };
        let alphabets: [&[u8]; 3] = [
            &(0..=255).collect::<Vec<u8>>(),
            br#"{}[]",:\ a1-.+e"#,
            &long_strings,
        ];
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for alphabet in alphabets {
            for len in 0..=300 {
                for _ in 0..4 {
                    let json: Vec<u8> = (0..len)
                        .map(|_| {
                            seed ^= seed << 13;
                            seed ^= seed >> 7;
                            seed ^= seed << 17;
                            alphabet[(seed % alphabet.len() as u64) as usize]
                        })
                        .collect();
                    let simd = build_semi_index_standard(&json);
                    let scalar = crate::json::standard::build_semi_index_scalar(&json);
                    assert_eq!(simd.ib, scalar.ib, "IB for {json:?}");
                    assert_eq!(simd.bp, scalar.bp, "BP for {json:?}");
                    assert_eq!(simd.state, scalar.state, "state for {json:?}");
                }
            }
        }
    }

    /// A block skipped as wholly inside a string must not swallow the escape that
    /// ends the block before it, or the escaped quote that opens the next one.
    #[test]
    fn test_avx2_branch_free_string_skip_at_block_edges() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        for pad in 0..64 {
            let mut json = br#"{"k":""#.to_vec();
            json.extend(std::iter::repeat_n(b'a', pad));
            json.extend_from_slice(br#"\""#);
            json.extend(std::iter::repeat_n(b'b', 96));
            json.extend_from_slice(br#"","n":12,"v":[true,{}]}"#);
            let simd = build_semi_index_standard(&json);
            let scalar = crate::json::standard::build_semi_index_scalar(&json);
            assert_eq!(simd.ib, scalar.ib, "IB, pad {pad}");
            assert_eq!(simd.bp, scalar.bp, "BP, pad {pad}");
            assert_eq!(simd.state, scalar.state, "state, pad {pad}");
        }
    }

    /// The shared kernel's differential suite, over the AVX2 builder (#7305).
    #[test]
    fn test_avx2_standard_kernel_suite() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        crate::json::simd::step::suite::all(build_semi_index_standard);
    }

    /// AVX2's answers to the shared kernel's per-block questions are the portable
    /// reference's (#7305).
    #[test]
    fn test_avx2_block_masks_agree_with_portable() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        // SAFETY: AVX2 was detected above.
        unsafe { crate::json::simd::step::suite::masks_agree::<Avx2>() };
    }

    // ========================================================================
    // Simple Cursor AVX2 Tests
    // ========================================================================

    #[test]
    fn test_simple_avx2_matches_scalar_empty_object() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = b"{}";
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(
            bits_to_string(&simd_result.bp, 4),
            bits_to_string(&scalar_result.bp, 4),
            "BP mismatch"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_simple_avx2_matches_scalar_simple_object() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = br#"{"a":"b"}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(
            bits_to_string(&simd_result.bp, 6),
            bits_to_string(&scalar_result.bp, 6),
            "BP mismatch"
        );
    }

    #[test]
    fn test_simple_avx2_matches_scalar_long_input() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        // Test with input > 32 bytes
        let json = br#"{"name":"value","number":12345,"array":[1,2,3]}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for long input"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }

    #[test]
    fn test_simple_avx2_matches_scalar_exact_32_bytes() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = br#"{"a":"b","c":"d","e":"fghijklm"}"#;
        assert_eq!(json.len(), 32, "Test JSON should be exactly 32 bytes");

        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch for 32-byte input"
        );
    }

    #[test]
    fn test_simple_avx2_matches_scalar_escaped() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = br#"{"a":"b\"c"}"#;
        let simd_result = build_semi_index_simple(json);
        let scalar_result = crate::json::simple::build_semi_index(json);

        assert_eq!(
            bits_to_string(&simd_result.ib, json.len()),
            bits_to_string(&scalar_result.ib, json.len()),
            "IB mismatch"
        );
        assert_eq!(simd_result.state, scalar_result.state);
    }
}
