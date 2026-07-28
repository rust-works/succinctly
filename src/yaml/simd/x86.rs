#![allow(unsafe_code)] // x86_64 SSE2/AVX2 SIMD intrinsics
//! x86_64 SIMD-accelerated string scanning for YAML parsing.
//!
//! Uses SSE2 (baseline, 16 bytes) with optional AVX2 (32 bytes) when available.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use super::scalar::{find_block_scalar_end_scalar, parse_anchor_name_scalar};

// ============================================================================
// Dispatch clamp (SUCCINCTLY_SIMD)
// ============================================================================

/// Parsed `SUCCINCTLY_SIMD` value: does it clamp x86 dispatch below AVX2?
///
/// - `Some(true)` — a recognized level below AVX2 (`scalar`, `sse2`, `sse42`,
///   `sse4.2`): use the 16-byte SSE2 kernels. (`scalar` still means SSE2 here;
///   scalar YAML parsing is compile-time only, via `--features scalar-yaml`.)
/// - `Some(false)` — recognized no-op (`avx2`, empty): keep detected dispatch.
/// - `None` — unrecognized. The runtime ignores it (no clamp), but the
///   `test_succinctly_simd_env_contract` test fails loudly on it so a typo in
///   a CI leg cannot silently un-clamp the suite.
#[cfg(any(test, feature = "std"))]
fn parse_simd_clamp(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "scalar" | "sse2" | "sse42" | "sse4.2" => Some(true),
        "avx2" | "" => Some(false),
        _ => None,
    }
}

/// Whether `SUCCINCTLY_SIMD` requests clamping dispatch below AVX2.
#[cfg(any(test, feature = "std"))]
fn clamp_below_avx2() -> bool {
    std::env::var("SUCCINCTLY_SIMD").is_ok_and(|v| parse_simd_clamp(&v) == Some(true))
}

/// Whether the 32-byte AVX2 kernels should be dispatched.
///
/// Combines runtime detection with the clamp-down-only `SUCCINCTLY_SIMD`
/// override: `SUCCINCTLY_SIMD=sse2` forces the 16-byte SSE2 kernels even on an
/// AVX2 CPU, which is how CI executes the SSE2 classify/skip-width path on
/// AVX2 runners (#247). Clamping can only lower the level, never raise it —
/// dispatching an undetected feature would be undefined behaviour.
///
/// Cached per STYLE-0003 (see `use_sve2`, `src/json/simd/mod.rs`): dispatch is
/// on the per-chunk hot path, so the env var is read once per process;
/// mutating it mid-run has no effect.
#[cfg(any(test, feature = "std"))]
#[inline]
fn avx2_enabled() -> bool {
    static AVX2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVX2.get_or_init(|| is_x86_feature_detected!("avx2") && !clamp_below_avx2())
}

// ============================================================================
// Multi-Character Classification (P0 Optimization)
// ============================================================================

/// Character classification results for a 32-byte chunk (AVX2).
/// Some fields are not currently used but are part of the classification infrastructure.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // STYLE-0005: SIMD classifier type; used only on some paths
pub struct YamlCharClass {
    /// Mask of bytes that are '\n'
    pub newlines: u32,
    /// Mask of bytes that are '\r'.
    ///
    /// YAML 1.2 §5.4 makes `\r` a line break in its own right, so scalar scans
    /// must stop at one exactly as they stop at `\n`. Kept separate from
    /// `newlines` so callers that genuinely mean LF still get LF (#324).
    pub carriage_returns: u32,
    /// Mask of bytes that are ':'
    pub colons: u32,
    /// Mask of bytes that are '-'
    pub hyphens: u32,
    /// Mask of bytes that are ' ' (space)
    pub spaces: u32,
    /// Mask of bytes that are '"'
    pub quotes_double: u32,
    /// Mask of bytes that are '\''
    pub quotes_single: u32,
    /// Mask of bytes that are '\\'
    pub backslashes: u32,
    /// Mask of bytes that are '#'
    pub hash: u32,
    /// Number of bytes actually classified: 32 (AVX2) or 16 (SSE2). Only the
    /// low `width` bits of each mask are meaningful. Callers must not assume
    /// 32 bytes were scanned — deriving the width from input length alone
    /// skipped structural bytes 16..31 on non-AVX2 CPUs (#193).
    pub width: usize,
}

impl YamlCharClass {
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
    /// loop.
    ///
    /// Kept verbatim in sync with `YamlCharClass16::plain_scalar_terminators`
    /// in `yaml::simd::broadword`, which is the same set for the ARM64 variant
    /// of `skip_unquoted_simd`. (Not an intra-doc link: that module is not
    /// compiled on x86_64.) The two disagreed until #185 — this one omitted
    /// `\r` before #324, and the broadword one added `spaces`.
    #[inline(always)]
    pub fn plain_scalar_terminators(&self) -> u32 {
        self.newlines | self.carriage_returns | self.colons | self.hash
    }
}

/// Classify YAML structural characters in a 32-byte chunk using AVX2.
///
/// This is the main P0 optimization - bulk classification of YAML characters.
/// Falls back to SSE2 (16 bytes) for inputs smaller than 32 bytes.
#[inline]
pub fn classify_yaml_chars(input: &[u8], offset: usize) -> Option<YamlCharClass> {
    // Require at least 16 bytes for SSE2
    if offset + 16 > input.len() {
        return None;
    }

    #[cfg(any(test, feature = "std"))]
    {
        if offset + 32 <= input.len() && avx2_enabled() {
            return Some(unsafe { classify_yaml_chars_avx2(input, offset) });
        }
    }

    // Fallback to SSE2 (requires 16 bytes minimum)
    if offset + 16 <= input.len() {
        return Some(unsafe { classify_yaml_chars_sse2(input, offset) });
    }

    None
}

/// AVX2 implementation of character classification (32 bytes at a time).
#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn classify_yaml_chars_avx2(input: &[u8], offset: usize) -> YamlCharClass {
    let chunk = _mm256_loadu_si256(input.as_ptr().add(offset).cast::<__m256i>());

    // Create comparison vectors for each character
    let v_newline = _mm256_set1_epi8(b'\n' as i8);
    let v_carriage_return = _mm256_set1_epi8(b'\r' as i8);
    let v_colon = _mm256_set1_epi8(b':' as i8);
    let v_hyphen = _mm256_set1_epi8(b'-' as i8);
    let v_space = _mm256_set1_epi8(b' ' as i8);
    let v_quote_double = _mm256_set1_epi8(b'"' as i8);
    let v_quote_single = _mm256_set1_epi8(b'\'' as i8);
    let v_backslash = _mm256_set1_epi8(b'\\' as i8);
    let v_hash = _mm256_set1_epi8(b'#' as i8);

    // Compare and extract masks
    let eq_newline = _mm256_cmpeq_epi8(chunk, v_newline);
    let eq_carriage_return = _mm256_cmpeq_epi8(chunk, v_carriage_return);
    let eq_colon = _mm256_cmpeq_epi8(chunk, v_colon);
    let eq_hyphen = _mm256_cmpeq_epi8(chunk, v_hyphen);
    let eq_space = _mm256_cmpeq_epi8(chunk, v_space);
    let eq_quote_double = _mm256_cmpeq_epi8(chunk, v_quote_double);
    let eq_quote_single = _mm256_cmpeq_epi8(chunk, v_quote_single);
    let eq_backslash = _mm256_cmpeq_epi8(chunk, v_backslash);
    let eq_hash = _mm256_cmpeq_epi8(chunk, v_hash);

    YamlCharClass {
        newlines: _mm256_movemask_epi8(eq_newline) as u32,
        carriage_returns: _mm256_movemask_epi8(eq_carriage_return) as u32,
        colons: _mm256_movemask_epi8(eq_colon) as u32,
        hyphens: _mm256_movemask_epi8(eq_hyphen) as u32,
        spaces: _mm256_movemask_epi8(eq_space) as u32,
        quotes_double: _mm256_movemask_epi8(eq_quote_double) as u32,
        quotes_single: _mm256_movemask_epi8(eq_quote_single) as u32,
        backslashes: _mm256_movemask_epi8(eq_backslash) as u32,
        hash: _mm256_movemask_epi8(eq_hash) as u32,
        width: 32,
    }
}

/// SSE2 implementation of character classification (16 bytes at a time).
#[target_feature(enable = "sse2")]
unsafe fn classify_yaml_chars_sse2(input: &[u8], offset: usize) -> YamlCharClass {
    let chunk = _mm_loadu_si128(input.as_ptr().add(offset).cast::<__m128i>());

    // Create comparison vectors for each character
    let v_newline = _mm_set1_epi8(b'\n' as i8);
    let v_carriage_return = _mm_set1_epi8(b'\r' as i8);
    let v_colon = _mm_set1_epi8(b':' as i8);
    let v_hyphen = _mm_set1_epi8(b'-' as i8);
    let v_space = _mm_set1_epi8(b' ' as i8);
    let v_quote_double = _mm_set1_epi8(b'"' as i8);
    let v_quote_single = _mm_set1_epi8(b'\'' as i8);
    let v_backslash = _mm_set1_epi8(b'\\' as i8);
    let v_hash = _mm_set1_epi8(b'#' as i8);

    // Compare and extract masks
    let eq_newline = _mm_cmpeq_epi8(chunk, v_newline);
    let eq_carriage_return = _mm_cmpeq_epi8(chunk, v_carriage_return);
    let eq_colon = _mm_cmpeq_epi8(chunk, v_colon);
    let eq_hyphen = _mm_cmpeq_epi8(chunk, v_hyphen);
    let eq_space = _mm_cmpeq_epi8(chunk, v_space);
    let eq_quote_double = _mm_cmpeq_epi8(chunk, v_quote_double);
    let eq_quote_single = _mm_cmpeq_epi8(chunk, v_quote_single);
    let eq_backslash = _mm_cmpeq_epi8(chunk, v_backslash);
    let eq_hash = _mm_cmpeq_epi8(chunk, v_hash);

    YamlCharClass {
        newlines: _mm_movemask_epi8(eq_newline) as u32,
        carriage_returns: _mm_movemask_epi8(eq_carriage_return) as u32,
        colons: _mm_movemask_epi8(eq_colon) as u32,
        hyphens: _mm_movemask_epi8(eq_hyphen) as u32,
        spaces: _mm_movemask_epi8(eq_space) as u32,
        quotes_double: _mm_movemask_epi8(eq_quote_double) as u32,
        quotes_single: _mm_movemask_epi8(eq_quote_single) as u32,
        backslashes: _mm_movemask_epi8(eq_backslash) as u32,
        hash: _mm_movemask_epi8(eq_hash) as u32,
        width: 16,
    }
}

/// Find the next newline using SIMD.
///
/// Returns offset from `start` to the newline, or `None` if not found.
#[inline]
pub fn find_newline_x86(input: &[u8], start: usize) -> Option<usize> {
    #[cfg(any(test, feature = "std"))]
    {
        if avx2_enabled() {
            return unsafe { find_newline_avx2(input, start) };
        }
    }

    unsafe { find_newline_sse2(input, start) }
}

#[target_feature(enable = "sse2")]
unsafe fn find_newline_sse2(input: &[u8], start: usize) -> Option<usize> {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let newline_vec = _mm_set1_epi8(b'\n' as i8);

    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());
        let matches = _mm_cmpeq_epi8(chunk, newline_vec);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes
    (offset..len).find(|&i| data[i] == b'\n')
}

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn find_newline_avx2(input: &[u8], start: usize) -> Option<usize> {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let newline_vec = _mm256_set1_epi8(b'\n' as i8);

    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset).cast::<__m256i>());
        let matches = _mm256_cmpeq_epi8(chunk, newline_vec);
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 32;
    }

    // Handle remaining bytes with SSE2
    if offset + 16 <= len {
        let newline_vec_sse = _mm_set1_epi8(b'\n' as i8);
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());
        let matches = _mm_cmpeq_epi8(chunk, newline_vec_sse);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    // Handle remaining bytes
    (offset..len).find(|&i| data[i] == b'\n')
}

// ============================================================================
// Original SIMD Functions (Enhanced)
// ============================================================================

/// Find the next double-quote or backslash using x86 SIMD.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_quote_or_escape_x86(input: &[u8], start: usize, end: usize) -> Option<usize> {
    // Runtime dispatch to best available implementation
    #[cfg(any(test, feature = "std"))]
    {
        if avx2_enabled() {
            // SAFETY: We just checked for AVX2 support
            return unsafe { find_quote_or_escape_avx2(input, start, end) };
        }
    }

    // SAFETY: SSE2 is guaranteed on x86_64
    unsafe { find_quote_or_escape_sse2(input, start, end) }
}

/// Find the next single-quote using x86 SIMD.
///
/// Returns offset from `start` to the found character, or `None` if not found.
#[inline]
pub fn find_single_quote_x86(input: &[u8], start: usize, end: usize) -> Option<usize> {
    // Runtime dispatch to best available implementation
    #[cfg(any(test, feature = "std"))]
    {
        if avx2_enabled() {
            // SAFETY: We just checked for AVX2 support
            return unsafe { find_single_quote_avx2(input, start, end) };
        }
    }

    // SAFETY: SSE2 is guaranteed on x86_64
    unsafe { find_single_quote_sse2(input, start, end) }
}

// ============================================================================
// SSE2 implementations (baseline, 16 bytes at a time)
// ============================================================================

#[target_feature(enable = "sse2")]
unsafe fn find_quote_or_escape_sse2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm_set1_epi8(b'"' as i8);
    let backslash_vec = _mm_set1_epi8(b'\\' as i8);

    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());

        // Compare against both targets
        let quotes = _mm_cmpeq_epi8(chunk, quote_vec);
        let backslashes = _mm_cmpeq_epi8(chunk, backslash_vec);

        // OR the results
        let matches = _mm_or_si128(quotes, backslashes);

        // Extract bitmask (one bit per byte)
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes
    (offset..len).find(|&i| {
        let b = data[i];
        b == b'"' || b == b'\\'
    })
}

#[target_feature(enable = "sse2")]
unsafe fn find_single_quote_sse2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm_set1_epi8(b'\'' as i8);

    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());

        // Compare against single quote
        let matches = _mm_cmpeq_epi8(chunk, quote_vec);

        // Extract bitmask
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 16;
    }

    // Handle remaining bytes
    (offset..len).find(|&i| data[i] == b'\'')
}

// ============================================================================
// AVX2 implementations (32 bytes at a time)
// ============================================================================

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn find_quote_or_escape_avx2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm256_set1_epi8(b'"' as i8);
    let backslash_vec = _mm256_set1_epi8(b'\\' as i8);

    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset).cast::<__m256i>());

        // Compare against both targets
        let quotes = _mm256_cmpeq_epi8(chunk, quote_vec);
        let backslashes = _mm256_cmpeq_epi8(chunk, backslash_vec);

        // OR the results
        let matches = _mm256_or_si256(quotes, backslashes);

        // Extract bitmask (one bit per byte)
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 32;
    }

    // Handle remaining bytes (16-31 bytes) with SSE2
    if offset + 16 <= len {
        let quote_vec_sse = _mm_set1_epi8(b'"' as i8);
        let backslash_vec_sse = _mm_set1_epi8(b'\\' as i8);

        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());
        let quotes = _mm_cmpeq_epi8(chunk, quote_vec_sse);
        let backslashes = _mm_cmpeq_epi8(chunk, backslash_vec_sse);
        let matches = _mm_or_si128(quotes, backslashes);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    // Handle remaining bytes (< 16)
    (offset..len).find(|&i| {
        let b = data[i];
        b == b'"' || b == b'\\'
    })
}

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn find_single_quote_avx2(input: &[u8], start: usize, end: usize) -> Option<usize> {
    let len = end - start;
    let data = &input[start..end];
    let mut offset = 0;

    let quote_vec = _mm256_set1_epi8(b'\'' as i8);

    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset).cast::<__m256i>());

        // Compare against single quote
        let matches = _mm256_cmpeq_epi8(chunk, quote_vec);

        // Extract bitmask
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }

        offset += 32;
    }

    // Handle remaining bytes (16-31 bytes) with SSE2
    if offset + 16 <= len {
        let quote_vec_sse = _mm_set1_epi8(b'\'' as i8);

        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());
        let matches = _mm_cmpeq_epi8(chunk, quote_vec_sse);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0 {
            return Some(offset + mask.trailing_zeros() as usize);
        }
        offset += 16;
    }

    // Handle remaining bytes (< 16)
    (offset..len).find(|&i| data[i] == b'\'')
}

/// Count leading spaces (indentation) using x86 SIMD.
///
/// Returns the number of consecutive space characters starting at `start`.
#[inline]
pub fn count_leading_spaces_x86(input: &[u8], start: usize) -> usize {
    // Runtime dispatch to best available implementation
    #[cfg(any(test, feature = "std"))]
    {
        if avx2_enabled() {
            // SAFETY: We just checked for AVX2 support
            return unsafe { count_leading_spaces_avx2(input, start) };
        }
    }

    // SAFETY: SSE2 is guaranteed on x86_64
    unsafe { count_leading_spaces_sse2(input, start) }
}

#[target_feature(enable = "sse2")]
unsafe fn count_leading_spaces_sse2(input: &[u8], start: usize) -> usize {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let space_vec = _mm_set1_epi8(b' ' as i8);

    // Process 16-byte chunks
    while offset + 16 <= len {
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());

        // Compare against space
        let matches = _mm_cmpeq_epi8(chunk, space_vec);

        // Extract bitmask (one bit per byte)
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0xFFFF {
            // Found a non-space - count trailing ones (consecutive spaces from start)
            return offset + (!mask).trailing_zeros() as usize;
        }

        offset += 16;
    }

    // Handle remaining bytes
    offset + data[offset..].iter().take_while(|&&b| b == b' ').count()
}

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn count_leading_spaces_avx2(input: &[u8], start: usize) -> usize {
    let data = &input[start..];
    let len = data.len();
    let mut offset = 0;

    let space_vec = _mm256_set1_epi8(b' ' as i8);

    // Process 32-byte chunks
    while offset + 32 <= len {
        let chunk = _mm256_loadu_si256(data.as_ptr().add(offset).cast::<__m256i>());

        // Compare against space
        let matches = _mm256_cmpeq_epi8(chunk, space_vec);

        // Extract bitmask (one bit per byte)
        let mask = _mm256_movemask_epi8(matches) as u32;

        if mask != 0xFFFF_FFFF {
            // Found a non-space - count trailing ones (consecutive spaces from start)
            return offset + (!mask).trailing_zeros() as usize;
        }

        offset += 32;
    }

    // Handle remaining bytes (16-31 bytes) with SSE2
    if offset + 16 <= len {
        let space_vec_sse = _mm_set1_epi8(b' ' as i8);
        let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());
        let matches = _mm_cmpeq_epi8(chunk, space_vec_sse);
        let mask = _mm_movemask_epi8(matches) as u32;

        if mask != 0xFFFF {
            return offset + (!mask).trailing_zeros() as usize;
        }
        offset += 16;
    }

    // Handle remaining bytes (< 16)
    offset + data[offset..].iter().take_while(|&&b| b == b' ').count()
}

// ============================================================================
// Block Scalar Optimization
// ============================================================================

/// Find the end of a block scalar by scanning for a line with insufficient indentation.
///
/// Uses SIMD to find newlines and check indentation efficiently.
/// Returns the position where the block ends (start of line with insufficient indent),
/// or input.len() if EOF is reached.
#[inline]
pub fn find_block_scalar_end(input: &[u8], start: usize, min_indent: usize) -> Option<usize> {
    if start >= input.len() {
        return Some(input.len());
    }

    #[cfg(any(test, feature = "std"))]
    {
        if avx2_enabled() {
            return Some(unsafe { find_block_scalar_end_avx2(input, start, min_indent) });
        }
    }

    // Fall back to SSE2
    Some(unsafe { find_block_scalar_end_sse2(input, start, min_indent) })
}

#[cfg(any(test, feature = "std"))]
#[target_feature(enable = "avx2")]
unsafe fn find_block_scalar_end_avx2(input: &[u8], start: usize, min_indent: usize) -> usize {
    let newline_vec = _mm256_set1_epi8(b'\n' as i8);
    let carriage_return_vec = _mm256_set1_epi8(b'\r' as i8);
    let space_vec = _mm256_set1_epi8(b' ' as i8);

    let mut pos = start;

    // Process in 32-byte chunks, looking for newlines
    while pos + 32 < input.len() {
        let chunk = _mm256_loadu_si256(input.as_ptr().add(pos).cast::<__m256i>());
        // Match either line-break byte (#324). A CRLF sets both bits; the CR's
        // "next line" is the LF itself, which the empty-line guard below skips.
        let nl_matches = _mm256_or_si256(
            _mm256_cmpeq_epi8(chunk, newline_vec),
            _mm256_cmpeq_epi8(chunk, carriage_return_vec),
        );
        let mut nl_mask = _mm256_movemask_epi8(nl_matches) as u32;

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

                // Use SIMD to count spaces
                if remaining >= 32 {
                    let next_chunk =
                        _mm256_loadu_si256(input.as_ptr().add(line_start).cast::<__m256i>());
                    let space_matches = _mm256_cmpeq_epi8(next_chunk, space_vec);
                    let space_mask = _mm256_movemask_epi8(space_matches) as u32;

                    if space_mask != 0xFFFF_FFFF {
                        indent = (!space_mask).trailing_zeros() as usize;
                    } else {
                        indent = 32;
                        // Continue counting if all 32 were spaces
                        let mut check_pos = line_start + 32;
                        while check_pos < input.len() && input[check_pos] == b' ' {
                            indent += 1;
                            check_pos += 1;
                        }
                    }
                } else {
                    // Less than 32 bytes remaining, count scalar
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

        pos += 32;
    }

    // Handle remainder with scalar code
    find_block_scalar_end_scalar(input, pos, min_indent)
}

#[target_feature(enable = "sse2")]
unsafe fn find_block_scalar_end_sse2(input: &[u8], start: usize, min_indent: usize) -> usize {
    let newline_vec = _mm_set1_epi8(b'\n' as i8);
    let carriage_return_vec = _mm_set1_epi8(b'\r' as i8);
    let space_vec = _mm_set1_epi8(b' ' as i8);

    let mut pos = start;

    // Process in 16-byte chunks
    while pos + 16 < input.len() {
        let chunk = _mm_loadu_si128(input.as_ptr().add(pos).cast::<__m128i>());
        // Match either line-break byte — see the AVX2 path (#324).
        let nl_matches = _mm_or_si128(
            _mm_cmpeq_epi8(chunk, newline_vec),
            _mm_cmpeq_epi8(chunk, carriage_return_vec),
        );
        let mut nl_mask = _mm_movemask_epi8(nl_matches) as u32;

        if nl_mask != 0 {
            while nl_mask != 0 {
                let offset = nl_mask.trailing_zeros() as usize;
                let line_start = pos + offset + 1;

                if line_start >= input.len() {
                    return input.len();
                }

                // Count leading spaces (SSE2 version)
                let mut indent = 0;
                let remaining = input.len() - line_start;

                if remaining >= 16 {
                    let next_chunk =
                        _mm_loadu_si128(input.as_ptr().add(line_start).cast::<__m128i>());
                    let space_matches = _mm_cmpeq_epi8(next_chunk, space_vec);
                    let space_mask = _mm_movemask_epi8(space_matches) as u32;

                    if space_mask != 0xFFFF {
                        indent = (!space_mask).trailing_zeros() as usize;
                    } else {
                        indent = 16;
                        let mut check_pos = line_start + 16;
                        while check_pos < input.len() && input[check_pos] == b' ' {
                            indent += 1;
                            check_pos += 1;
                        }
                    }
                } else {
                    while line_start + indent < input.len() && input[line_start + indent] == b' ' {
                        indent += 1;
                    }
                }

                if line_start + indent < input.len() {
                    let next_char = input[line_start + indent];
                    if next_char != b'\n' && next_char != b'\r' && indent < min_indent {
                        return line_start;
                    }
                }

                nl_mask &= nl_mask - 1;
            }
        }

        pos += 16;
    }

    find_block_scalar_end_scalar(input, pos, min_indent)
}

// ============================================================================
// Anchor/Alias Name Parsing (P4 Optimization)
// ============================================================================

/// Parse anchor/alias name using AVX2 SIMD to find terminator characters.
///
/// Searches for YAML anchor name terminators:
/// - Whitespace: space, tab, newline, CR
/// - Flow indicators: [ ] { } ,
/// - Colons followed by whitespace (a bare `:` is a legal name character,
///   mirroring [`super::neon::parse_anchor_name_neon`] and
///   [`parse_anchor_name_scalar`] — see #404's follow-up fix: this function
///   used to treat every `:` as an unconditional terminator, which shrank an
///   anchor name like `:@*!$"<foo>` to empty and made an alias to it read as
///   unresolved).
///
/// Returns the position of the first terminator, or end of input.
#[target_feature(enable = "avx2")]
unsafe fn parse_anchor_name_avx2(input: &[u8], start: usize) -> usize {
    let len = input.len();
    if start >= len {
        return start;
    }

    let mut pos = start;
    let end = len;

    // Prepare comparison vectors for all terminator characters
    let space = _mm256_set1_epi8(b' ' as i8);
    let tab = _mm256_set1_epi8(b'\t' as i8);
    let newline = _mm256_set1_epi8(b'\n' as i8);
    let cr = _mm256_set1_epi8(b'\r' as i8);
    let lbracket = _mm256_set1_epi8(b'[' as i8);
    let rbracket = _mm256_set1_epi8(b']' as i8);
    let lbrace = _mm256_set1_epi8(b'{' as i8);
    let rbrace = _mm256_set1_epi8(b'}' as i8);
    let comma = _mm256_set1_epi8(b',' as i8);
    let colon = _mm256_set1_epi8(b':' as i8);

    // Process 32 bytes at a time with AVX2
    while pos + 32 <= end {
        let chunk = _mm256_loadu_si256(input.as_ptr().add(pos).cast::<__m256i>());

        // Check for all terminator types
        let is_space = _mm256_cmpeq_epi8(chunk, space);
        let is_tab = _mm256_cmpeq_epi8(chunk, tab);
        let is_newline = _mm256_cmpeq_epi8(chunk, newline);
        let is_cr = _mm256_cmpeq_epi8(chunk, cr);
        let is_lbracket = _mm256_cmpeq_epi8(chunk, lbracket);
        let is_rbracket = _mm256_cmpeq_epi8(chunk, rbracket);
        let is_lbrace = _mm256_cmpeq_epi8(chunk, lbrace);
        let is_rbrace = _mm256_cmpeq_epi8(chunk, rbrace);
        let is_comma = _mm256_cmpeq_epi8(chunk, comma);
        let is_colon = _mm256_cmpeq_epi8(chunk, colon);

        // Whitespace and flow indicators are unconditional terminators. A
        // colon is only a terminator when the byte after it is whitespace, so
        // it cannot be decided from this chunk alone — resolve it below.
        let ws = _mm256_or_si256(is_space, is_tab);
        let ws = _mm256_or_si256(ws, is_newline);
        let ws = _mm256_or_si256(ws, is_cr);

        let flow = _mm256_or_si256(is_lbracket, is_rbracket);
        let flow = _mm256_or_si256(flow, is_lbrace);
        let flow = _mm256_or_si256(flow, is_rbrace);
        let flow = _mm256_or_si256(flow, is_comma);

        let definite = _mm256_or_si256(ws, flow);

        let definite_mask = _mm256_movemask_epi8(definite) as u32;
        let colon_mask = _mm256_movemask_epi8(is_colon) as u32;
        let combined_mask = definite_mask | colon_mask;

        if combined_mask != 0 {
            let first_pos = combined_mask.trailing_zeros() as usize;

            // A definite terminator at or before the first colon candidate
            // wins outright — it needs no lookahead.
            if (definite_mask >> first_pos) & 1 != 0 {
                return pos + first_pos;
            }

            // The first candidate is a colon: it terminates only if the very
            // next byte is whitespace.
            let colon_pos = pos + first_pos;
            if colon_pos + 1 < end {
                if let b' ' | b'\t' | b'\n' | b'\r' = input[colon_pos + 1] {
                    return colon_pos;
                }
            }

            // Not a terminator — the colon is a name character. Hand the rest
            // of the scan to the scalar version rather than re-deriving the
            // same colon-lookahead rule in SIMD for every subsequent byte.
            return parse_anchor_name_scalar(input, colon_pos + 1);
        }

        pos += 32;
    }

    // Handle remaining bytes with scalar fallback
    parse_anchor_name_scalar(input, pos)
}

/// Public API: Parse anchor/alias name with runtime SIMD dispatch.
///
/// Returns the position of the first terminator character.
#[inline]
pub fn parse_anchor_name(input: &[u8], start: usize) -> usize {
    // Use SIMD for longer names (16+ bytes expected)
    if start + 16 <= input.len() {
        #[cfg(any(test, feature = "std"))]
        {
            if avx2_enabled() {
                return unsafe { parse_anchor_name_avx2(input, start) };
            }
        }

        #[cfg(not(any(test, feature = "std")))]
        {
            return unsafe { parse_anchor_name_avx2(input, start) };
        }
    }

    // Fallback to scalar for short names or no SIMD
    parse_anchor_name_scalar(input, start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse2_find_quote_basic() {
        let input = b"hello\"world";
        unsafe {
            assert_eq!(find_quote_or_escape_sse2(input, 0, input.len()), Some(5));
        }
    }

    #[test]
    fn test_sse2_find_backslash() {
        let input = b"hello\\world";
        unsafe {
            assert_eq!(find_quote_or_escape_sse2(input, 0, input.len()), Some(5));
        }
    }

    #[test]
    fn test_sse2_find_single_quote() {
        let input = b"hello'world";
        unsafe {
            assert_eq!(find_single_quote_sse2(input, 0, input.len()), Some(5));
        }
    }

    #[test]
    fn test_sse2_long_string() {
        let mut input = vec![b'a'; 100];
        input[50] = b'"';
        unsafe {
            assert_eq!(find_quote_or_escape_sse2(&input, 0, input.len()), Some(50));
        }
    }

    #[test]
    fn test_dispatched_find_quote() {
        let input = b"hello\"world";
        assert_eq!(find_quote_or_escape_x86(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_dispatched_find_single_quote() {
        let input = b"hello'world";
        assert_eq!(find_single_quote_x86(input, 0, input.len()), Some(5));
    }

    #[test]
    fn test_sse2_count_leading_spaces_basic() {
        unsafe {
            assert_eq!(count_leading_spaces_sse2(b"  hello", 0), 2);
            assert_eq!(count_leading_spaces_sse2(b"    world", 0), 4);
            assert_eq!(count_leading_spaces_sse2(b"no spaces", 0), 0);
        }
    }

    #[test]
    fn test_sse2_count_leading_spaces_long() {
        // Test with > 16 bytes to exercise SIMD path
        let mut input = vec![b' '; 50];
        input.extend_from_slice(b"content");
        unsafe {
            assert_eq!(count_leading_spaces_sse2(&input, 0), 50);
        }
    }

    #[test]
    fn test_dispatched_count_leading_spaces() {
        assert_eq!(count_leading_spaces_x86(b"  hello", 0), 2);
        assert_eq!(count_leading_spaces_x86(b"    world", 0), 4);
        assert_eq!(count_leading_spaces_x86(b"no spaces", 0), 0);

        // Test long string
        let mut input = vec![b' '; 50];
        input.extend_from_slice(b"content");
        assert_eq!(count_leading_spaces_x86(&input, 0), 50);
    }

    // ========================================================================
    // Tests for P0 optimizations
    // ========================================================================

    #[test]
    fn test_classify_yaml_chars_basic() {
        // Keep input under 16 bytes for SSE2 test
        let input = b"key: #val-item\nmore";
        let class = classify_yaml_chars(input, 0).unwrap();

        // Check colon at position 3
        assert_ne!(class.colons & (1 << 3), 0, "Colon not found at position 3");

        // Check space at position 4
        assert_ne!(class.spaces & (1 << 4), 0, "Space not found at position 4");

        // Check hash at position 5
        assert_ne!(class.hash & (1 << 5), 0, "Hash not found at position 5");

        // Check hyphen at position 9
        assert_ne!(
            class.hyphens & (1 << 9),
            0,
            "Hyphen not found at position 9"
        );

        // Check newline at position 14
        assert_ne!(
            class.newlines & (1 << 14),
            0,
            "Newline not found at position 14"
        );
    }

    #[test]
    fn test_classify_yaml_chars_quotes() {
        let input = b"a: \"val\" 'x'end."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Check double quote at position 3
        assert_ne!(
            class.quotes_double & (1 << 3),
            0,
            "Double quote not found at position 3"
        );

        // Check double quote at position 7
        assert_ne!(
            class.quotes_double & (1 << 7),
            0,
            "Double quote not found at position 7"
        );

        // Check single quote at position 9
        assert_ne!(
            class.quotes_single & (1 << 9),
            0,
            "Single quote not found at position 9"
        );

        // Check single quote at position 11
        assert_ne!(
            class.quotes_single & (1 << 11),
            0,
            "Single quote not found at position 11"
        );
    }

    #[test]
    fn test_classify_yaml_chars_backslash() {
        let input = b"text: \"esc\\n\"ok."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Check backslash at position 10
        assert_ne!(
            class.backslashes & (1 << 10),
            0,
            "Backslash not found at position 10"
        );
    }

    #[test]
    fn test_find_newline_basic() {
        let input = b"line1\nline2\nline3";
        assert_eq!(find_newline_x86(input, 0), Some(5));
        assert_eq!(find_newline_x86(input, 6), Some(5)); // 5 bytes from offset 6 = position 11
    }

    #[test]
    fn test_find_newline_long() {
        let mut input = vec![b'x'; 100];
        input[50] = b'\n';
        assert_eq!(find_newline_x86(&input, 0), Some(50));
    }

    #[test]
    fn test_find_newline_not_found() {
        let input = b"no newline here";
        assert_eq!(find_newline_x86(input, 0), None);
    }

    #[test]
    fn test_classify_context_sensitive() {
        // Test ": " pattern (colon followed by space)
        let input = b"key: val t:1:2x."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Colon at position N followed by space at N+1 means:
        // - Colon bit is set at position N
        // - Space bit is set at position N+1
        // - Shift space mask right by 1 to align with colon position
        // - AND with colon mask: colons & (spaces >> 1)
        let colon_space_pattern = class.colons & (class.spaces >> 1);

        // Position 3: colon followed by space at 4 should match
        assert_ne!(
            colon_space_pattern & (1 << 3),
            0,
            "Colon-space pattern not found at position 3"
        );

        // Positions 11, 13: colons not followed by space should not match
        assert_eq!(
            colon_space_pattern & (1 << 11),
            0,
            "False positive: colon-space at position 11"
        );
        assert_eq!(
            colon_space_pattern & (1 << 13),
            0,
            "False positive: colon-space at position 13"
        );
    }

    #[test]
    fn test_classify_hyphen_space_pattern() {
        // Test "- " pattern (hyphen followed by space)
        let input = b"- item\nval-nosp."; // 16 bytes
        let class = classify_yaml_chars(input, 0).unwrap();

        // Hyphen at position N followed by space at N+1
        let hyphen_space_pattern = class.hyphens & (class.spaces >> 1);

        // Position 0: hyphen followed by space at 1 should match
        assert_ne!(
            hyphen_space_pattern & (1 << 0),
            0,
            "Hyphen-space pattern not found at position 0"
        );

        // Position 11: hyphen not followed by space should not match
        assert_eq!(
            hyphen_space_pattern & (1 << 11),
            0,
            "False positive: hyphen-space at position 11"
        );
    }

    /// Detection guard for AVX2; emits a visible `SKIPPED` line when
    /// unavailable so a fully-skipped kernel doesn't read as green (#193).
    fn has_avx2() -> bool {
        crate::util::simd::note_simd_skip_unless(is_x86_feature_detected!("avx2"), "avx2")
    }

    // ========================================================================
    // SUCCINCTLY_SIMD dispatch clamp (#247)
    // ========================================================================

    #[test]
    fn test_parse_simd_clamp_values() {
        // Recognized levels below AVX2 clamp (whitespace/case-insensitive).
        for v in ["scalar", "sse2", "sse42", "sse4.2", " SSE2 ", "Sse4.2"] {
            assert_eq!(parse_simd_clamp(v), Some(true), "{v:?} should clamp");
        }
        // Recognized no-ops.
        for v in ["avx2", "AVX2", ""] {
            assert_eq!(parse_simd_clamp(v), Some(false), "{v:?} should be a no-op");
        }
        // Unrecognized values parse to None (runtime ignores; contract test
        // fails loudly when the env var is actually set to one of these).
        for v in ["banana", "neon", "1", "sse", "avx512"] {
            assert_eq!(parse_simd_clamp(v), None, "{v:?} should be unrecognized");
        }
    }

    /// Contract test for the `SUCCINCTLY_SIMD` clamp, in the spirit of
    /// `SUCCINCTLY_EXPECT_SIMD` (#192): when a CI leg sets the variable, the
    /// dispatcher must observably honor it, and a typo'd value must fail the
    /// leg instead of silently un-clamping the suite. Reads the environment
    /// only — never sets it — so it is race-free across test threads.
    #[test]
    fn test_succinctly_simd_env_contract() {
        let buf = [b'a'; 64];
        let width = classify_yaml_chars(&buf, 0).unwrap().width;

        match std::env::var("SUCCINCTLY_SIMD") {
            Ok(v) => match parse_simd_clamp(&v) {
                Some(true) => {
                    assert!(!avx2_enabled(), "SUCCINCTLY_SIMD={v} must clamp dispatch");
                    assert_eq!(width, 16, "SUCCINCTLY_SIMD={v} must force 16-byte classify");
                }
                Some(false) => assert_eq!(
                    width,
                    if is_x86_feature_detected!("avx2") {
                        32
                    } else {
                        16
                    }
                ),
                None => panic!(
                    "SUCCINCTLY_SIMD={v:?} is not a recognized level \
                     (scalar|sse2|sse42|sse4.2|avx2) — fix the caller so the \
                     clamp actually applies"
                ),
            },
            Err(_) => assert_eq!(
                width,
                if is_x86_feature_detected!("avx2") {
                    32
                } else {
                    16
                },
                "unclamped dispatch must classify at the detected width"
            ),
        }
    }

    /// All nine mask channels of a `YamlCharClass`, paired with names for
    /// assertion messages.
    ///
    /// `carriage_returns` was missing here until #185, so the channel #324 added
    /// to the live terminator mask was the one channel the sweep below never
    /// checked.
    fn classify_channels(class: &YamlCharClass) -> [(u32, &'static str); 9] {
        [
            (class.newlines, "newlines"),
            (class.carriage_returns, "carriage_returns"),
            (class.colons, "colons"),
            (class.hyphens, "hyphens"),
            (class.spaces, "spaces"),
            (class.quotes_double, "quotes_double"),
            (class.quotes_single, "quotes_single"),
            (class.backslashes, "backslashes"),
            (class.hash, "hash"),
        ]
    }

    /// Per-kernel differential sweep for the classify path (#247, option 2 of
    /// the issue): a lone structural byte at every position 0..40 of an
    /// otherwise-inert buffer, asserted against both kernels directly. The
    /// 16..31 window is exactly where the length-derived skip width swallowed
    /// terminators after a 16-byte SSE2 classify (#231).
    #[test]
    fn test_classify_kernels_differential_terminator_sweep() {
        // Second field indexes into `classify_channels`; keep the two in step.
        let structural: [(u8, usize); 9] = [
            (b'\n', 0),
            (b'\r', 1),
            (b':', 2),
            (b'-', 3),
            (b' ', 4),
            (b'"', 5),
            (b'\'', 6),
            (b'\\', 7),
            (b'#', 8),
        ];
        let run_avx2 = has_avx2();

        for &(byte, channel) in &structural {
            for pos in 0..40usize {
                let mut buf = [b'a'; 48];
                buf[pos] = byte;

                // SSE2 kernel at offset 0: sees bytes 0..16 only.
                let sse2 = unsafe { classify_yaml_chars_sse2(&buf, 0) };
                assert_eq!(sse2.width, 16, "SSE2 must report a 16-byte width");
                for (i, (mask, name)) in classify_channels(&sse2).iter().enumerate() {
                    let expected = if i == channel && pos < 16 {
                        1u32 << pos
                    } else {
                        0
                    };
                    assert_eq!(
                        *mask, expected,
                        "SSE2@0 {name} mask for 0x{byte:02x} at {pos}"
                    );
                }

                // SSE2 kernel at offset 16: covers the #231 window 16..31.
                let sse2_hi = unsafe { classify_yaml_chars_sse2(&buf, 16) };
                for (i, (mask, name)) in classify_channels(&sse2_hi).iter().enumerate() {
                    let expected = if i == channel && (16..32).contains(&pos) {
                        1u32 << (pos - 16)
                    } else {
                        0
                    };
                    assert_eq!(
                        *mask, expected,
                        "SSE2@16 {name} mask for 0x{byte:02x} at {pos}"
                    );
                }

                if run_avx2 {
                    let avx2 = unsafe { classify_yaml_chars_avx2(&buf, 0) };
                    assert_eq!(avx2.width, 32, "AVX2 must report a 32-byte width");
                    let sse2_lo = classify_channels(&sse2);
                    for (i, (mask, name)) in classify_channels(&avx2).iter().enumerate() {
                        let expected = if i == channel && pos < 32 {
                            1u32 << pos
                        } else {
                            0
                        };
                        assert_eq!(
                            *mask, expected,
                            "AVX2 {name} mask for 0x{byte:02x} at {pos}"
                        );
                        assert_eq!(
                            *mask & 0xFFFF,
                            sse2_lo[i].0,
                            "AVX2 low 16 bits must equal SSE2 ({name}, 0x{byte:02x} at {pos})"
                        );
                    }
                }
            }
        }
    }

    /// `plain_scalar_terminators` is the mask the *live* `skip_unquoted_simd`
    /// runs on ([`crate::yaml::parser`]), so it is the copy that matters and the
    /// one nothing tested before #185. Asserted per byte rather than as an OR of
    /// the channel fields, which would restate the implementation: a channel
    /// added to or dropped from the set changes which bytes stop the skip, and
    /// that is what this pins.
    ///
    /// The twin of this test for the broadword widths is
    /// `plain_scalar_terminators_stop_at_structure_but_not_at_spaces` in
    /// `yaml::simd::broadword`; the two sets must stay identical because the
    /// parser's byte loop is shared.
    #[test]
    fn plain_scalar_terminators_is_exactly_line_break_colon_hash() {
        // A terminator must be a superset of what the byte loop breaks on, and
        // must exclude space — a plain scalar may contain one (`key: hello
        // world`), so stopping there only shortens the skip.
        let terminating = *b"\n\r:#";
        let passing = *b" \t-\"'\\a0";

        for byte in terminating.into_iter().chain(passing) {
            let mut buf = [b'a'; 48];
            buf[3] = byte;
            let class = classify_yaml_chars(&buf, 0).expect("48 bytes available");
            let stops = class.plain_scalar_terminators() & (1 << 3) != 0;

            assert_eq!(
                stops,
                terminating.contains(&byte),
                "0x{byte:02x} ({:?}) must {} a plain scalar",
                byte as char,
                if terminating.contains(&byte) {
                    "terminate"
                } else {
                    "not terminate"
                }
            );
        }

        // An inert chunk sets nothing, so the mask is not simply always hot.
        let inert = [b'a'; 48];
        let class = classify_yaml_chars(&inert, 0).expect("48 bytes available");
        assert_eq!(class.plain_scalar_terminators(), 0);
    }

    // ========================================================================
    // parse_anchor_name AVX2/scalar differential tests (#404 follow-up)
    // ========================================================================

    /// The AVX2 kernel used to treat every `:` as an unconditional terminator
    /// (missing the "only when followed by whitespace" rule the doc comment
    /// itself described), shrinking a name like `:@*!$"<foo>` to empty. Confirmed
    /// against the YAML Test Suite's W5VH case ("Allowed characters in alias"),
    /// which the strict validator's alias-scope check (#404) turned into a
    /// spurious `unknown anchor` rejection on x86_64 only — `parse_anchor_name`
    /// is shared with the loader (`src/yaml/parser.rs`), so this also risked
    /// silently breaking real anchor resolution on x86_64 hardware, just never
    /// observed because typical anchor names don't contain a bare `:`.
    ///
    /// Differential against [`parse_anchor_name_scalar`] — the reference both
    /// this kernel and [`super::neon::parse_anchor_name_neon`] are meant to
    /// match — over every placement class the colon rule must distinguish:
    /// colon-then-whitespace (terminates before the colon), colon-then-name-char
    /// (colon is content, scan continues), a colon run before whitespace, and
    /// colon-then-flow-indicator (the colon rule only checks for whitespace, so
    /// the flow indicator terminates and the colon stays in the name). Buffers
    /// are 64 bytes so every case is reached through the 32-byte AVX2 loop
    /// rather than its scalar tail, including a colon placed as the last byte of
    /// the first chunk so the next-byte lookahead crosses the chunk boundary.
    #[test]
    fn test_parse_anchor_name_avx2_matches_scalar_around_colons() {
        if !has_avx2() {
            return;
        }

        let case = |input: &[u8], start: usize| {
            let scalar = parse_anchor_name_scalar(input, start);
            let avx2 = unsafe { parse_anchor_name_avx2(input, start) };
            assert_eq!(
                avx2,
                scalar,
                "AVX2/scalar mismatch for {:?} from {start}",
                String::from_utf8_lossy(input)
            );
            avx2
        };

        // The exact W5VH name, embedded so the colon lands inside the first
        // 32-byte AVX2 chunk: `:@*!$"<foo>` followed by `: ` (colon-space,
        // terminates before the colon) then filler.
        let mut buf = b":@*!$\"<foo>: ".to_vec();
        buf.extend(std::iter::repeat_n(b'a', 64 - buf.len()));
        assert_eq!(case(&buf, 0), 11, "name must stop before the `: ` pair");

        // A colon immediately followed by whitespace at the very start.
        let mut buf = b": ".to_vec();
        buf.extend(std::iter::repeat_n(b'a', 62));
        assert_eq!(
            case(&buf, 0),
            0,
            "empty name: `:` is followed by whitespace"
        );

        // A colon followed by a non-whitespace name character: content, not a
        // terminator; the scan continues to the real terminator (a space).
        let mut buf = b"a:b c".to_vec();
        buf.extend(std::iter::repeat_n(b'a', 59));
        assert_eq!(case(&buf, 0), 3, "`a:b` is all name; stops at the space");

        // A run of colons before the whitespace that actually terminates.
        let mut buf = b"foo::: bar".to_vec();
        buf.extend(std::iter::repeat_n(b'a', 54));
        assert_eq!(case(&buf, 0), 5, "stops at the colon adjacent to the space");

        // A colon followed by a flow indicator: the colon rule only checks for
        // whitespace, so the colon is content and the flow indicator (not the
        // colon) is the terminator.
        for term in [b',', b']', b'}'] {
            let mut buf = vec![b'a', b':'];
            buf.push(term);
            buf.extend(std::iter::repeat_n(b'a', 61));
            assert_eq!(
                case(&buf, 0),
                2,
                "colon before {:?} is content; {:?} terminates",
                term as char,
                term as char
            );
        }

        // A colon as the last byte of the first 32-byte chunk, with the
        // terminating whitespace as the first byte of the next chunk — the
        // next-byte check must read across the chunk boundary correctly.
        let mut buf = vec![b'a'; 31];
        buf.push(b':');
        buf.push(b' ');
        buf.extend(std::iter::repeat_n(b'a', 32));
        assert_eq!(case(&buf, 0), 31, "boundary-crossing colon-space");

        // The same shape, but the colon is not followed by whitespace across
        // the boundary: it stays content and scanning continues past it.
        let mut buf = vec![b'a'; 31];
        buf.push(b':');
        buf.push(b'b');
        buf.push(b' ');
        buf.extend(std::iter::repeat_n(b'a', 31));
        assert_eq!(case(&buf, 0), 33, "boundary-crossing colon-then-name-char");
    }
}
