//! UTF-8 validation with detailed error reporting.
//!
//! This module provides UTF-8 validation that reports:
//! - The exact byte offset of the error
//! - The line number (1-indexed)
//! - The column number (1-indexed, in bytes)
//! - The specific type of UTF-8 violation
//!
//! ## UTF-8 Encoding Rules
//!
//! UTF-8 is a variable-width encoding that uses 1-4 bytes per character:
//!
//! | Bytes | First byte    | Continuation bytes | Code point range     |
//! |-------|---------------|-------------------|----------------------|
//! | 1     | `0xxxxxxx`    | -                 | U+0000 - U+007F      |
//! | 2     | `110xxxxx`    | `10xxxxxx`        | U+0080 - U+07FF      |
//! | 3     | `1110xxxx`    | `10xxxxxx` × 2    | U+0800 - U+FFFF      |
//! | 4     | `11110xxx`    | `10xxxxxx` × 3    | U+10000 - U+10FFFF   |
//!
//! ## Validation Checks
//!
//! The validator checks for:
//! 1. **Invalid lead bytes**: Bytes 0x80-0xBF appearing where a lead byte is expected
//! 2. **Invalid continuation bytes**: Non-continuation bytes where continuation expected
//! 3. **Overlong encodings**: Using more bytes than necessary (security vulnerability)
//! 4. **Surrogate code points**: U+D800-U+DFFF (reserved for UTF-16)
//! 5. **Out of range**: Code points above U+10FFFF
//! 6. **Truncated sequences**: Multi-byte sequence cut off at end of input

use alloc::string::String;

use crate::util::broadword::{H8, L8};

/// Error information for UTF-8 validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utf8Error {
    /// The byte offset where the error occurred (0-indexed).
    pub offset: usize,
    /// The line number where the error occurred (1-indexed).
    pub line: usize,
    /// The column (byte position within the line, 1-indexed).
    pub column: usize,
    /// The kind of UTF-8 error.
    pub kind: Utf8ErrorKind,
}

impl core::fmt::Display for Utf8Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "{} at byte {}, line {}, column {}",
            self.kind, self.offset, self.line, self.column
        )
    }
}

/// The specific type of UTF-8 validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utf8ErrorKind {
    /// A byte in the range 0x80-0xBF appeared where a lead byte was expected.
    /// These bytes are only valid as continuation bytes.
    InvalidLeadByte,

    /// A byte outside the range 0x80-0xBF appeared where a continuation byte was expected.
    InvalidContinuationByte,

    /// A character was encoded using more bytes than necessary.
    /// For example, encoding ASCII 'A' (U+0041) as `C0 81` instead of `41`.
    /// This is a security vulnerability as it can bypass validation filters.
    OverlongEncoding,

    /// A surrogate code point (U+D800-U+DFFF) was encoded.
    /// These are reserved for UTF-16 surrogate pairs and invalid in UTF-8.
    SurrogateCodepoint,

    /// A code point above U+10FFFF was encoded.
    /// Unicode only defines code points up to U+10FFFF.
    OutOfRangeCodepoint,

    /// A multi-byte sequence was truncated at the end of input.
    TruncatedSequence,
}

impl core::fmt::Display for Utf8ErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidLeadByte => write!(f, "invalid UTF-8 lead byte"),
            Self::InvalidContinuationByte => write!(f, "invalid UTF-8 continuation byte"),
            Self::OverlongEncoding => write!(f, "overlong UTF-8 encoding"),
            Self::SurrogateCodepoint => write!(f, "surrogate code point in UTF-8"),
            Self::OutOfRangeCodepoint => write!(f, "code point above U+10FFFF"),
            Self::TruncatedSequence => write!(f, "truncated UTF-8 sequence"),
        }
    }
}

/// Validate that the input is valid UTF-8.
///
/// Returns `Ok(())` if the input is valid UTF-8, or an `Err(Utf8Error)` with
/// detailed information about the first validation error.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::validate_utf8;
///
/// // Valid ASCII
/// assert!(validate_utf8(b"Hello, world!").is_ok());
///
/// // Valid multi-byte UTF-8
/// assert!(validate_utf8("日本語".as_bytes()).is_ok());
/// assert!(validate_utf8("émoji: 🎉".as_bytes()).is_ok());
///
/// // Invalid: bare continuation byte
/// assert!(validate_utf8(&[0x80]).is_err());
///
/// // Invalid: truncated sequence
/// assert!(validate_utf8(&[0xC2]).is_err());
/// ```
#[inline]
pub fn validate_utf8(input: &[u8]) -> Result<(), Utf8Error> {
    // On x86_64 with runtime feature detection available (std or test), prefer
    // the AVX2 fast path, which falls back to the scalar validator on any error
    // (and on non-AVX2 hardware) so the reported `Utf8Error` is identical.
    #[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
    {
        validate_utf8_simd(input)
    }
    #[cfg(not(all(target_arch = "x86_64", any(test, feature = "std"))))]
    {
        validate_utf8_scalar(input)
    }
}

/// Validate UTF-8 using a portable scalar algorithm with a broadword (SWAR)
/// ASCII fast path.
///
/// This is the only validation path on non-x86_64 targets, in `no_std` builds,
/// on x86_64 CPUs without AVX2, and behind the CLI's `--no-simd`; on AVX2 hosts
/// it also backs [`validate_utf8`]'s error reporting.
///
/// Runs of ASCII are skipped eight bytes at a time via [`skip_ascii`], while
/// multi-byte sequences are validated one character at a time. Errors carry the
/// exact byte offset, line number, and column position, derived from the offset
/// by [`line_and_column`] once an error is found — see its docs for why keeping
/// that state off the hot path is exact rather than approximate.
pub fn validate_utf8_scalar(input: &[u8]) -> Result<(), Utf8Error> {
    let mut pos = 0;
    let len = input.len();

    while pos < len {
        let byte = input[pos];

        // Determine sequence length from lead byte
        let seq_len = match byte {
            // ASCII: 0x00-0x7F (single byte). Skip the whole run at once.
            0x00..=0x7F => {
                pos = skip_ascii(input, pos + 1);
                continue;
            }
            // Continuation bytes appearing as lead: invalid
            0x80..=0xBF => {
                return Err(err_at(input, pos, Utf8ErrorKind::InvalidLeadByte));
            }
            // 2-byte sequence: 0xC0-0xDF
            0xC0..=0xDF => 2,
            // 3-byte sequence: 0xE0-0xEF
            0xE0..=0xEF => 3,
            // 4-byte sequence: 0xF0-0xF7
            0xF0..=0xF7 => 4,
            // Invalid lead bytes: 0xF8-0xFF
            0xF8..=0xFF => {
                return Err(err_at(input, pos, Utf8ErrorKind::InvalidLeadByte));
            }
        };

        // Check for truncation
        if pos + seq_len > len {
            return Err(err_at(input, pos, Utf8ErrorKind::TruncatedSequence));
        }

        // Validate continuation bytes and decode code point
        match seq_len {
            2 => {
                let b1 = input[pos + 1];
                if !is_continuation_byte(b1) {
                    return Err(err_at(
                        input,
                        pos + 1,
                        Utf8ErrorKind::InvalidContinuationByte,
                    ));
                }

                // Check for overlong encoding (code points < 0x80 must use 1 byte)
                // 2-byte sequences must encode U+0080 or higher
                // Lead byte 0xC0 or 0xC1 would encode < 0x80
                if byte <= 0xC1 {
                    return Err(err_at(input, pos, Utf8ErrorKind::OverlongEncoding));
                }
            }
            3 => {
                let b1 = input[pos + 1];
                let b2 = input[pos + 2];

                if !is_continuation_byte(b1) {
                    return Err(err_at(
                        input,
                        pos + 1,
                        Utf8ErrorKind::InvalidContinuationByte,
                    ));
                }
                if !is_continuation_byte(b2) {
                    return Err(err_at(
                        input,
                        pos + 2,
                        Utf8ErrorKind::InvalidContinuationByte,
                    ));
                }

                // Decode code point for validation
                let cp =
                    ((byte as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F);

                // Check for overlong encoding (code points < 0x800 must use 2 bytes)
                if cp < 0x800 {
                    return Err(err_at(input, pos, Utf8ErrorKind::OverlongEncoding));
                }

                // Check for surrogate code points (U+D800-U+DFFF)
                if (0xD800..=0xDFFF).contains(&cp) {
                    return Err(err_at(input, pos, Utf8ErrorKind::SurrogateCodepoint));
                }
            }
            4 => {
                let b1 = input[pos + 1];
                let b2 = input[pos + 2];
                let b3 = input[pos + 3];

                if !is_continuation_byte(b1) {
                    return Err(err_at(
                        input,
                        pos + 1,
                        Utf8ErrorKind::InvalidContinuationByte,
                    ));
                }
                if !is_continuation_byte(b2) {
                    return Err(err_at(
                        input,
                        pos + 2,
                        Utf8ErrorKind::InvalidContinuationByte,
                    ));
                }
                if !is_continuation_byte(b3) {
                    return Err(err_at(
                        input,
                        pos + 3,
                        Utf8ErrorKind::InvalidContinuationByte,
                    ));
                }

                // Decode code point for validation
                let cp = ((byte as u32 & 0x07) << 18)
                    | ((b1 as u32 & 0x3F) << 12)
                    | ((b2 as u32 & 0x3F) << 6)
                    | (b3 as u32 & 0x3F);

                // Check for overlong encoding (code points < 0x10000 must use 3 bytes)
                if cp < 0x10000 {
                    return Err(err_at(input, pos, Utf8ErrorKind::OverlongEncoding));
                }

                // Check for out of range (> U+10FFFF)
                if cp > 0x10FFFF {
                    return Err(err_at(input, pos, Utf8ErrorKind::OutOfRangeCodepoint));
                }
            }
            _ => unreachable!(),
        }

        pos += seq_len;
    }

    Ok(())
}

/// Advance past a run of ASCII bytes starting at `pos`, eight at a time.
///
/// Returns the index of the first non-ASCII byte at or after `pos`, or
/// `input.len()` if the rest of the input is ASCII.
///
/// A byte is ASCII exactly when its high bit is clear, so a whole 8-byte word is
/// ASCII iff `word & H8 == 0` — one load, one AND and one test per eight bytes
/// instead of per byte. When the word does contain a non-ASCII byte, its index
/// is `trailing_zeros() / 8`: `from_le_bytes` maps byte *k* to bits *8k..8k+7*
/// on every host, so this is endian-independent.
///
/// The caller only reaches this from the ASCII arm of the lead-byte dispatch, so
/// multi-byte-heavy input never executes the word loop at all.
///
/// `#[inline(never)]` is deliberate and measured. Inlining the word loop into the
/// dispatch bloats the enclosing loop body and costs multi-byte-heavy input
/// 5-12%: on an M4 Pro over three repetitions, the inlined form ranged from
/// -10.6% to +11.8% against the byte-at-a-time baseline (regressing pure emoji in
/// three of four runs), while keeping it out of line was faster on all twelve
/// (benchmark, repetition) pairs, median -13.0%. The call is amortised over a
/// whole ASCII run, so the ASCII path gives up under 1% for it.
#[inline(never)]
fn skip_ascii(input: &[u8], mut pos: usize) -> usize {
    let len = input.len();

    while pos + 8 <= len {
        let word = u64::from_le_bytes(input[pos..pos + 8].try_into().unwrap());
        let non_ascii = word & H8;
        if non_ascii != 0 {
            return pos + (non_ascii.trailing_zeros() as usize >> 3);
        }
        pos += 8;
    }

    // Fewer than 8 bytes left: finish byte-at-a-time.
    while pos < len && input[pos] < 0x80 {
        pos += 1;
    }
    pos
}

/// Build a [`Utf8Error`] for `offset`, resolving its line and column.
///
/// Marked `#[cold]` so the optimizer lays this out away from the validation loop
/// and keeps `line`/`column` bookkeeping out of the hot path entirely.
#[cold]
#[inline(never)]
fn err_at(input: &[u8], offset: usize, kind: Utf8ErrorKind) -> Utf8Error {
    let (line, column) = line_and_column(input, offset);
    Utf8Error {
        offset,
        line,
        column,
        kind,
    }
}

/// The 1-indexed line and column (in bytes) of `offset` within `input`.
///
/// Called only when validation has already failed, so this single backward-
/// looking scan replaces per-byte `line`/`line_start` bookkeeping in the hot
/// loop. That substitution is exact, not approximate:
///
/// - The line of `offset` is `1 + (newlines in input[..offset])`, because `\n` is
///   a one-byte character and so is always counted exactly once.
/// - Errors reported at `pos + k` inside a multi-byte sequence still resolve to
///   the same line as the sequence start `pos`: `input[pos]` is a lead byte
///   (`>= 0xC0`) and `input[pos + 1..pos + k]` have already been checked as
///   continuation bytes (`0x80..=0xBF`), so no `\n` can occur between them.
///
/// Newlines are located eight bytes at a time by XORing against a broadcast `\n`
/// — turning every match into a zero byte — and then applying an exact broadword
/// zero-byte test.
fn line_and_column(input: &[u8], offset: usize) -> (usize, usize) {
    /// `\n` broadcast to all eight byte lanes.
    const NEWLINES: u64 = L8.wrapping_mul(b'\n' as u64);
    /// The low seven bits of each byte lane (the complement of [`H8`]).
    const LOW7: u64 = !H8;

    let prefix = &input[..offset];
    let mut line = 1;
    let mut line_start = 0;
    let mut pos = 0;

    while pos + 8 <= prefix.len() {
        let word = u64::from_le_bytes(prefix[pos..pos + 8].try_into().unwrap());
        let zeros = word ^ NEWLINES;

        // Exact per-byte zero test: `(b & 0x7F) + 0x7F` sets a byte's high bit
        // iff its low seven bits are non-zero, and OR-ing `b` back in covers the
        // high bit, so the negation leaves a set high bit exactly where a byte
        // was zero. Because `(b & 0x7F) + 0x7F <= 0xFE`, no carry escapes a lane.
        //
        // The cheaper `(x - L8) & !x & H8` form is *not* usable here: it answers
        // "does this word contain a zero byte?" correctly, but borrow
        // propagation also flags the byte following a zero byte when that byte
        // is 0x01 — i.e. a `\n` followed by `\v` — which would over-count lines.
        let mask = !((zeros & LOW7).wrapping_add(LOW7) | zeros) & H8;

        if mask != 0 {
            // Exactly one bit is set per newline byte, so popcount is the count;
            // the highest set bit locates the last newline in the word.
            line += mask.count_ones() as usize;
            line_start = pos + (63 - mask.leading_zeros() as usize) / 8 + 1;
        }
        pos += 8;
    }

    for (i, &byte) in prefix[pos..].iter().enumerate() {
        if byte == b'\n' {
            line += 1;
            line_start = pos + i + 1;
        }
    }

    (line, offset - line_start + 1)
}

/// Check if a byte is a valid UTF-8 continuation byte (0x80-0xBF).
#[inline(always)]
fn is_continuation_byte(byte: u8) -> bool {
    (byte & 0xC0) == 0x80
}

/// Validate UTF-8 using AVX2 SIMD, falling back to the scalar validator for the
/// precise [`Utf8Error`] (and on CPUs without AVX2).
///
/// The AVX2 kernel only decides *validity* (a fast accept scan); it never
/// pinpoints errors. On any detected error — or when AVX2 is unavailable — this
/// re-runs [`validate_utf8_scalar`], so callers get byte-identical diagnostics
/// regardless of which path ran. Available on `x86_64` when runtime feature
/// detection is usable (the `std` feature, or under test).
#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub use self::simd_x86::validate_utf8_simd;

/// x86_64 AVX2 UTF-8 validation.
///
/// This is a first-principles port of the Keiser–Lemire "Validating UTF-8 In
/// Less Than One Instruction Per Byte" approach (<https://arxiv.org/abs/2010.03090>),
/// expressed with explicit range comparisons rather than packed lookup tables so
/// that every check maps directly onto the rules in [`validate_utf8_scalar`].
#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
mod simd_x86 {
    #![allow(unsafe_code)] // x86_64 AVX2 SIMD intrinsics
    #![allow(clippy::cast_possible_wrap)] // u8 byte constants deliberately reinterpreted as i8 lanes

    use super::{validate_utf8_scalar, Utf8Error};
    use core::arch::x86_64::*;

    /// Validate `input` as UTF-8 using AVX2 when available.
    ///
    /// Returns `Ok(())` only when the AVX2 accept scan proves the whole buffer
    /// valid; otherwise defers to the scalar validator for the exact error.
    pub fn validate_utf8_simd(input: &[u8]) -> Result<(), Utf8Error> {
        // SAFETY: `validate_utf8_avx2` is only entered once the CPU is confirmed
        // to support AVX2 by `is_x86_feature_detected!`.
        if is_x86_feature_detected!("avx2") && unsafe { validate_utf8_avx2(input) } {
            Ok(())
        } else {
            validate_utf8_scalar(input)
        }
    }

    /// Unsigned `a >= k` per byte lane; result lanes are `0xFF` (true) or `0x00`.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn uge(a: __m256i, k: u8) -> __m256i {
        // max_epu8(a, k) == a  iff  a >= k (unsigned). These register-only AVX2
        // intrinsics are safe to call within a `#[target_feature]` fn.
        let vk = _mm256_set1_epi8(k as i8);
        _mm256_cmpeq_epi8(_mm256_max_epu8(a, vk), a)
    }

    /// Unsigned `a < k` per byte lane; result lanes are `0xFF` (true) or `0x00`.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn ult(a: __m256i, k: u8) -> __m256i {
        unsafe { _mm256_xor_si256(uge(a, k), _mm256_set1_epi8(-1)) }
    }

    /// Accumulate UTF-8 errors for one 32-byte block into `error`.
    ///
    /// `prev_input` holds the previous block (zeros before the first block) so
    /// that `prev1`/`prev2`/`prev3` — the bytes 1/2/3 positions back — are
    /// available across the block boundary.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn check_block(chunk: __m256i, prev_input: __m256i, error: &mut __m256i) {
        unsafe {
            // Shift the 256-bit vector right by 1/2/3 bytes, pulling in the tail
            // of `prev_input` (the canonical simdjson `prev<N>` idiom).
            let shifted = _mm256_permute2x128_si256(prev_input, chunk, 0x21);
            let prev1 = _mm256_alignr_epi8(chunk, shifted, 15);
            let prev2 = _mm256_alignr_epi8(chunk, shifted, 14);
            let prev3 = _mm256_alignr_epi8(chunk, shifted, 13);

            // is_cont = (chunk & 0xC0) == 0x80
            let c0 = _mm256_set1_epi8(0xC0u8 as i8);
            let is_cont =
                _mm256_cmpeq_epi8(_mm256_and_si256(chunk, c0), _mm256_set1_epi8(0x80u8 as i8));

            // A byte MUST be a continuation iff the byte 1 back is any lead
            // (>=0xC0), or 2 back is a 3/4-byte lead (>=0xE0), or 3 back is a
            // 4-byte lead (>=0xF0). Missing OR stray continuations => is_cont
            // disagrees with must_cont.
            let must_cont = _mm256_or_si256(
                _mm256_or_si256(uge(prev1, 0xC0), uge(prev2, 0xE0)),
                uge(prev3, 0xF0),
            );
            let mut err = _mm256_xor_si256(is_cont, must_cont);

            // Bytes that are never valid anywhere: 0xC0/0xC1 (overlong 2-byte
            // leads) and 0xF5..=0xFF (beyond U+10FFFF / not UTF-8 lead bytes).
            let invalid_c0c1 =
                _mm256_cmpeq_epi8(_mm256_and_si256(chunk, _mm256_set1_epi8(0xFEu8 as i8)), c0);
            err = _mm256_or_si256(err, _mm256_or_si256(invalid_c0c1, uge(chunk, 0xF5)));

            // Special cases keyed on the lead byte (prev1) and the byte after it
            // (chunk), each firing only within the continuation range:
            //   E0 followed by <0xA0 -> overlong 3-byte
            //   ED followed by >=0xA0 -> surrogate (U+D800..U+DFFF)
            //   F0 followed by <0x90 -> overlong 4-byte
            //   F4 followed by >=0x90 -> above U+10FFFF
            let e0 = _mm256_cmpeq_epi8(prev1, _mm256_set1_epi8(0xE0u8 as i8));
            let ed = _mm256_cmpeq_epi8(prev1, _mm256_set1_epi8(0xEDu8 as i8));
            let f0 = _mm256_cmpeq_epi8(prev1, _mm256_set1_epi8(0xF0u8 as i8));
            let f4 = _mm256_cmpeq_epi8(prev1, _mm256_set1_epi8(0xF4u8 as i8));
            err = _mm256_or_si256(err, _mm256_and_si256(e0, ult(chunk, 0xA0)));
            err = _mm256_or_si256(err, _mm256_and_si256(ed, uge(chunk, 0xA0)));
            err = _mm256_or_si256(err, _mm256_and_si256(f0, ult(chunk, 0x90)));
            err = _mm256_or_si256(err, _mm256_and_si256(f4, uge(chunk, 0x90)));

            *error = _mm256_or_si256(*error, err);
        }
    }

    /// Return `true` iff `input` is entirely valid UTF-8.
    ///
    /// End-of-input truncation is caught by always processing a final
    /// zero-padded block: a dangling multi-byte lead's absent continuation lands
    /// on a `0x00` pad byte, which fails the continuation requirement.
    #[target_feature(enable = "avx2")]
    unsafe fn validate_utf8_avx2(input: &[u8]) -> bool {
        unsafe {
            let len = input.len();
            if len == 0 {
                return true;
            }

            let mut error = _mm256_setzero_si256();
            let mut prev_input = _mm256_setzero_si256();

            let mut pos = 0;
            while pos + 32 <= len {
                let chunk = _mm256_loadu_si256(input.as_ptr().add(pos).cast::<__m256i>());
                check_block(chunk, prev_input, &mut error);
                prev_input = chunk;
                pos += 32;
            }

            // Final zero-padded tail. Runs even when `len % 32 == 0` (tail all
            // zeros) so a lead byte at the very end is still flagged as truncated
            // via the carried `prev_input`.
            let mut tail = [0u8; 32];
            tail[..len - pos].copy_from_slice(&input[pos..]);
            let chunk = _mm256_loadu_si256(tail.as_ptr().cast::<__m256i>());
            check_block(chunk, prev_input, &mut error);

            // Valid iff no error bit was set anywhere.
            _mm256_testz_si256(error, error) == 1
        }
    }

    /// Test-only: run the raw AVX2 kernel over `input`.
    ///
    /// The kernel-vs-std differential test gates on `is_x86_feature_detected!`
    /// once and then calls this across its whole corpus. Exposing the raw kernel
    /// verdict (rather than the scalar-fallback [`validate_utf8_simd`] wrapper)
    /// lets the differential catch both false accepts *and* false rejects.
    ///
    /// # Safety
    /// The CPU must support AVX2; the caller checks `is_x86_feature_detected!`.
    #[cfg(test)]
    pub(crate) unsafe fn validate_utf8_avx2_unchecked(input: &[u8]) -> bool {
        // SAFETY: the caller guarantees AVX2 is available.
        unsafe { validate_utf8_avx2(input) }
    }
}

/// Get the expected sequence length from a lead byte.
/// Returns 0 for invalid lead bytes (continuation bytes or 0xF8+).
#[inline]
pub fn sequence_length(lead_byte: u8) -> usize {
    match lead_byte {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 0, // Invalid lead byte
    }
}

/// Decode a UTF-8 code point from a byte slice.
///
/// Returns `None` if the input is empty or contains an invalid sequence.
/// On success, returns the decoded code point and the number of bytes consumed.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::decode_code_point;
///
/// // ASCII
/// assert_eq!(decode_code_point(b"A"), Some(('A' as u32, 1)));
///
/// // Multi-byte
/// assert_eq!(decode_code_point("日".as_bytes()), Some((0x65E5, 3)));
///
/// // Empty input
/// assert_eq!(decode_code_point(b""), None);
/// ```
pub fn decode_code_point(input: &[u8]) -> Option<(u32, usize)> {
    if input.is_empty() {
        return None;
    }

    let lead = input[0];
    let len = sequence_length(lead);

    if len == 0 || input.len() < len {
        return None;
    }

    let cp = match len {
        1 => lead as u32,
        2 => {
            let b1 = input[1];
            if !is_continuation_byte(b1) {
                return None;
            }
            ((lead as u32 & 0x1F) << 6) | (b1 as u32 & 0x3F)
        }
        3 => {
            let b1 = input[1];
            let b2 = input[2];
            if !is_continuation_byte(b1) || !is_continuation_byte(b2) {
                return None;
            }
            ((lead as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F)
        }
        4 => {
            let b1 = input[1];
            let b2 = input[2];
            let b3 = input[3];
            if !is_continuation_byte(b1) || !is_continuation_byte(b2) || !is_continuation_byte(b3) {
                return None;
            }
            ((lead as u32 & 0x07) << 18)
                | ((b1 as u32 & 0x3F) << 12)
                | ((b2 as u32 & 0x3F) << 6)
                | (b3 as u32 & 0x3F)
        }
        _ => return None,
    };

    Some((cp, len))
}

/// Encode a Unicode code point as UTF-8.
///
/// Returns `None` if the code point is invalid (surrogate or > U+10FFFF).
/// On success, returns the UTF-8 bytes and the number of bytes used.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::encode_code_point;
///
/// // ASCII
/// let (bytes, len) = encode_code_point(0x41).unwrap();
/// assert_eq!(&bytes[..len], b"A");
///
/// // 2-byte character (é)
/// let (bytes, len) = encode_code_point(0xE9).unwrap();
/// assert_eq!(&bytes[..len], "é".as_bytes());
///
/// // 4-byte character (🎉)
/// let (bytes, len) = encode_code_point(0x1F389).unwrap();
/// assert_eq!(&bytes[..len], "🎉".as_bytes());
///
/// // Invalid: surrogate
/// assert!(encode_code_point(0xD800).is_none());
///
/// // Invalid: out of range
/// assert!(encode_code_point(0x110000).is_none());
/// ```
pub fn encode_code_point(cp: u32) -> Option<([u8; 4], usize)> {
    // Reject surrogates and out-of-range
    if (0xD800..=0xDFFF).contains(&cp) || cp > 0x10FFFF {
        return None;
    }

    let mut buf = [0u8; 4];

    let len = if cp < 0x80 {
        buf[0] = cp as u8;
        1
    } else if cp < 0x800 {
        buf[0] = 0xC0 | ((cp >> 6) as u8);
        buf[1] = 0x80 | ((cp & 0x3F) as u8);
        2
    } else if cp < 0x10000 {
        buf[0] = 0xE0 | ((cp >> 12) as u8);
        buf[1] = 0x80 | (((cp >> 6) & 0x3F) as u8);
        buf[2] = 0x80 | ((cp & 0x3F) as u8);
        3
    } else {
        buf[0] = 0xF0 | ((cp >> 18) as u8);
        buf[1] = 0x80 | (((cp >> 12) & 0x3F) as u8);
        buf[2] = 0x80 | (((cp >> 6) & 0x3F) as u8);
        buf[3] = 0x80 | ((cp & 0x3F) as u8);
        4
    };

    Some((buf, len))
}

/// Format a byte as a human-readable string for error messages.
pub fn format_byte(byte: u8) -> String {
    if byte.is_ascii_graphic() || byte == b' ' {
        alloc::format!("0x{:02X} ({:?})", byte, byte as char)
    } else {
        alloc::format!("0x{byte:02X}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // Valid UTF-8 Tests
    // =========================================================================

    mod valid_utf8 {
        use super::*;

        #[test]
        fn empty_input() {
            assert!(validate_utf8(b"").is_ok());
        }

        #[test]
        fn ascii_single_byte() {
            // All ASCII characters (0x00-0x7F)
            for byte in 0x00..=0x7F {
                assert!(
                    validate_utf8(&[byte]).is_ok(),
                    "ASCII byte 0x{byte:02X} should be valid"
                );
            }
        }

        #[test]
        fn ascii_string() {
            assert!(validate_utf8(b"Hello, world!").is_ok());
            assert!(validate_utf8(b"The quick brown fox jumps over the lazy dog").is_ok());
        }

        #[test]
        fn ascii_with_control_chars() {
            assert!(validate_utf8(b"line1\nline2\ttab\rcarriage").is_ok());
            assert!(validate_utf8(b"\x00\x01\x02\x1F").is_ok()); // Control characters
        }

        #[test]
        fn two_byte_sequences() {
            // U+0080 (first 2-byte code point)
            assert!(validate_utf8(&[0xC2, 0x80]).is_ok());

            // U+00FF (Latin Small Letter Y with Diaeresis)
            assert!(validate_utf8(&[0xC3, 0xBF]).is_ok());

            // U+07FF (last 2-byte code point)
            assert!(validate_utf8(&[0xDF, 0xBF]).is_ok());

            // Common 2-byte characters
            assert!(validate_utf8("é".as_bytes()).is_ok()); // U+00E9
            assert!(validate_utf8("ñ".as_bytes()).is_ok()); // U+00F1
            assert!(validate_utf8("ü".as_bytes()).is_ok()); // U+00FC
            assert!(validate_utf8("©".as_bytes()).is_ok()); // U+00A9
            assert!(validate_utf8("®".as_bytes()).is_ok()); // U+00AE
        }

        #[test]
        fn three_byte_sequences() {
            // U+0800 (first 3-byte code point)
            assert!(validate_utf8(&[0xE0, 0xA0, 0x80]).is_ok());

            // U+FFFF (last valid 3-byte code point in BMP)
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBF]).is_ok());

            // Japanese characters
            assert!(validate_utf8("日本語".as_bytes()).is_ok());
            assert!(validate_utf8("こんにちは".as_bytes()).is_ok());

            // Chinese characters
            assert!(validate_utf8("中文".as_bytes()).is_ok());
            assert!(validate_utf8("你好世界".as_bytes()).is_ok());

            // Korean characters
            assert!(validate_utf8("한국어".as_bytes()).is_ok());
            assert!(validate_utf8("안녕하세요".as_bytes()).is_ok());

            // Arabic
            assert!(validate_utf8("مرحبا".as_bytes()).is_ok());

            // Hebrew
            assert!(validate_utf8("שלום".as_bytes()).is_ok());

            // Thai
            assert!(validate_utf8("สวัสดี".as_bytes()).is_ok());

            // Currency symbols
            assert!(validate_utf8("€".as_bytes()).is_ok()); // U+20AC Euro
            assert!(validate_utf8("₹".as_bytes()).is_ok()); // U+20B9 Indian Rupee
            assert!(validate_utf8("₿".as_bytes()).is_ok()); // U+20BF Bitcoin
        }

        #[test]
        fn four_byte_sequences() {
            // U+10000 (first 4-byte code point)
            assert!(validate_utf8(&[0xF0, 0x90, 0x80, 0x80]).is_ok());

            // U+10FFFF (last valid code point)
            assert!(validate_utf8(&[0xF4, 0x8F, 0xBF, 0xBF]).is_ok());

            // Emoji
            assert!(validate_utf8("🎉".as_bytes()).is_ok()); // U+1F389 Party Popper
            assert!(validate_utf8("😀".as_bytes()).is_ok()); // U+1F600 Grinning Face
            assert!(validate_utf8("🚀".as_bytes()).is_ok()); // U+1F680 Rocket
            assert!(validate_utf8("🌍".as_bytes()).is_ok()); // U+1F30D Earth
            assert!(validate_utf8("💻".as_bytes()).is_ok()); // U+1F4BB Laptop
            assert!(validate_utf8("🔥".as_bytes()).is_ok()); // U+1F525 Fire

            // Mathematical symbols
            assert!(validate_utf8("𝕳".as_bytes()).is_ok()); // U+1D573 Mathematical Bold Fraktur H
            assert!(validate_utf8("𝔸".as_bytes()).is_ok()); // U+1D538 Mathematical Double-Struck A

            // Ancient scripts
            assert!(validate_utf8("𐀀".as_bytes()).is_ok()); // U+10000 Linear B Syllable B008 A

            // Music symbols
            assert!(validate_utf8("𝄞".as_bytes()).is_ok()); // U+1D11E Musical Symbol G Clef
        }

        #[test]
        fn mixed_sequences() {
            // Mix of all sequence lengths
            let mixed = "A é 日 🎉";
            assert!(validate_utf8(mixed.as_bytes()).is_ok());

            // Complex mixed text
            let complex = "Hello! 你好 مرحبا 🌍🚀 Ñoño café";
            assert!(validate_utf8(complex.as_bytes()).is_ok());
        }

        #[test]
        fn boundary_code_points() {
            // First code point of each length
            assert!(validate_utf8(&[0x00]).is_ok()); // U+0000
            assert!(validate_utf8(&[0xC2, 0x80]).is_ok()); // U+0080
            assert!(validate_utf8(&[0xE0, 0xA0, 0x80]).is_ok()); // U+0800
            assert!(validate_utf8(&[0xF0, 0x90, 0x80, 0x80]).is_ok()); // U+10000

            // Last code point of each length
            assert!(validate_utf8(&[0x7F]).is_ok()); // U+007F
            assert!(validate_utf8(&[0xDF, 0xBF]).is_ok()); // U+07FF
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBF]).is_ok()); // U+FFFF
            assert!(validate_utf8(&[0xF4, 0x8F, 0xBF, 0xBF]).is_ok()); // U+10FFFF
        }

        #[test]
        fn non_characters() {
            // Unicode non-characters are technically valid UTF-8
            // U+FFFE and U+FFFF
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBE]).is_ok()); // U+FFFE
            assert!(validate_utf8(&[0xEF, 0xBF, 0xBF]).is_ok()); // U+FFFF

            // BOM (Byte Order Mark)
            assert!(validate_utf8(&[0xEF, 0xBB, 0xBF]).is_ok()); // U+FEFF
        }

        #[test]
        fn long_valid_string() {
            // 1KB of mixed valid UTF-8
            let mut s = String::new();
            for i in 0..100 {
                s.push_str(&format!("Line {i}: Hello 世界 🎉\n"));
            }
            assert!(validate_utf8(s.as_bytes()).is_ok());
        }
    }

    // =========================================================================
    // Invalid Lead Byte Tests
    // =========================================================================

    mod invalid_lead_byte {
        use super::*;

        #[test]
        fn continuation_byte_as_lead() {
            // Continuation bytes (0x80-0xBF) cannot start a sequence
            for byte in 0x80..=0xBF {
                let result = validate_utf8(&[byte]);
                assert!(
                    result.is_err(),
                    "Byte 0x{byte:02X} should be invalid as lead"
                );
                let err = result.unwrap_err();
                assert_eq!(err.kind, Utf8ErrorKind::InvalidLeadByte);
                assert_eq!(err.offset, 0);
            }
        }

        #[test]
        fn continuation_byte_after_valid() {
            // Valid ASCII followed by bare continuation byte
            let input = [b'A', 0x80];
            let result = validate_utf8(&input);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidLeadByte);
            assert_eq!(err.offset, 1);
        }

        #[test]
        fn f8_ff_lead_bytes() {
            // 0xF8-0xFF are always invalid lead bytes
            for byte in 0xF8..=0xFF {
                let result = validate_utf8(&[byte]);
                assert!(
                    result.is_err(),
                    "Byte 0x{byte:02X} should be invalid as lead"
                );
                let err = result.unwrap_err();
                assert_eq!(err.kind, Utf8ErrorKind::InvalidLeadByte);
            }
        }

        #[test]
        fn fe_ff_bytes() {
            // 0xFE and 0xFF are never valid in UTF-8
            assert!(validate_utf8(&[0xFE]).is_err());
            assert!(validate_utf8(&[0xFF]).is_err());
            assert!(validate_utf8(&[0xFE, 0xFE, 0xFF, 0xFF]).is_err());
        }
    }

    // =========================================================================
    // Invalid Continuation Byte Tests
    // =========================================================================

    mod invalid_continuation {
        use super::*;

        #[test]
        fn missing_continuation_2byte() {
            // 2-byte lead followed by ASCII instead of continuation
            let result = validate_utf8(&[0xC2, b'A']);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
            assert_eq!(err.offset, 1);
        }

        #[test]
        fn missing_continuation_3byte_first() {
            // 3-byte lead followed by ASCII
            let result = validate_utf8(&[0xE0, b'A', 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
            assert_eq!(err.offset, 1);
        }

        #[test]
        fn missing_continuation_3byte_second() {
            // 3-byte sequence with second continuation wrong
            let result = validate_utf8(&[0xE0, 0xA0, b'A']);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
            assert_eq!(err.offset, 2);
        }

        #[test]
        fn missing_continuation_4byte() {
            // 4-byte sequence with various wrong continuations
            // Wrong first continuation
            let result = validate_utf8(&[0xF0, b'A', 0x80, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 1);

            // Wrong second continuation
            let result = validate_utf8(&[0xF0, 0x90, b'A', 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 2);

            // Wrong third continuation
            let result = validate_utf8(&[0xF0, 0x90, 0x80, b'A']);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 3);
        }

        #[test]
        fn continuation_is_another_lead() {
            // 2-byte lead followed by another lead byte
            let result = validate_utf8(&[0xC2, 0xC2]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::InvalidContinuationByte);
        }

        #[test]
        fn continuation_is_high_byte() {
            // Continuation position has 0xF0+ byte
            let result = validate_utf8(&[0xC2, 0xF0]);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().kind,
                Utf8ErrorKind::InvalidContinuationByte
            );
        }
    }

    // =========================================================================
    // Overlong Encoding Tests
    // =========================================================================

    mod overlong_encoding {
        use super::*;

        #[test]
        fn overlong_2byte_null() {
            // NUL (U+0000) encoded as 2 bytes: C0 80
            let result = validate_utf8(&[0xC0, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_2byte_ascii() {
            // ASCII 'A' (U+0041) encoded as 2 bytes: C1 81
            let result = validate_utf8(&[0xC1, 0x81]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_2byte_all_c0_c1() {
            // C0 and C1 lead bytes always indicate overlong encoding
            for lead in [0xC0, 0xC1] {
                for cont in 0x80..=0xBF {
                    let result = validate_utf8(&[lead, cont]);
                    assert!(
                        result.is_err(),
                        "0x{lead:02X} 0x{cont:02X} should be overlong"
                    );
                    assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
                }
            }
        }

        #[test]
        fn overlong_3byte_null() {
            // NUL (U+0000) encoded as 3 bytes: E0 80 80
            let result = validate_utf8(&[0xE0, 0x80, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_3byte_2byte_char() {
            // U+007F encoded as 3 bytes: E0 81 BF
            let result = validate_utf8(&[0xE0, 0x81, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);

            // U+07FF encoded as 3 bytes: E0 9F BF (should be DF BF)
            let result = validate_utf8(&[0xE0, 0x9F, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_4byte_null() {
            // NUL (U+0000) encoded as 4 bytes: F0 80 80 80
            let result = validate_utf8(&[0xF0, 0x80, 0x80, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn overlong_4byte_3byte_char() {
            // U+FFFF encoded as 4 bytes: F0 8F BF BF (should be EF BF BF)
            let result = validate_utf8(&[0xF0, 0x8F, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);
        }

        #[test]
        fn security_overlong_slash() {
            // Security test: overlong encoding of '/' (U+002F)
            // Attackers might use this to bypass path traversal filters

            // 2-byte: C0 AF
            let result = validate_utf8(&[0xC0, 0xAF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OverlongEncoding);

            // 3-byte: E0 80 AF
            let result = validate_utf8(&[0xE0, 0x80, 0xAF]);
            assert!(result.is_err());

            // 4-byte: F0 80 80 AF
            let result = validate_utf8(&[0xF0, 0x80, 0x80, 0xAF]);
            assert!(result.is_err());
        }
    }

    // =========================================================================
    // Surrogate Code Point Tests
    // =========================================================================

    mod surrogate_codepoints {
        use super::*;

        #[test]
        fn high_surrogate_start() {
            // U+D800 (first high surrogate): ED A0 80
            let result = validate_utf8(&[0xED, 0xA0, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn high_surrogate_end() {
            // U+DBFF (last high surrogate): ED AF BF
            let result = validate_utf8(&[0xED, 0xAF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn low_surrogate_start() {
            // U+DC00 (first low surrogate): ED B0 80
            let result = validate_utf8(&[0xED, 0xB0, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn low_surrogate_end() {
            // U+DFFF (last low surrogate): ED BF BF
            let result = validate_utf8(&[0xED, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
        }

        #[test]
        fn all_surrogates() {
            // Test a sample of surrogate code points
            let surrogates = [
                0xD800, 0xD801, 0xDB00, 0xDBFF, 0xDC00, 0xDC01, 0xDF00, 0xDFFF,
            ];
            for cp in surrogates {
                // Encode surrogate manually
                let bytes = [
                    0xE0 | ((cp >> 12) as u8),
                    0x80 | (((cp >> 6) & 0x3F) as u8),
                    0x80 | ((cp & 0x3F) as u8),
                ];
                let result = validate_utf8(&bytes);
                assert!(result.is_err(), "U+{cp:04X} should be invalid surrogate");
                assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::SurrogateCodepoint);
            }
        }

        #[test]
        fn surrogate_in_middle_of_valid() {
            // Valid text followed by surrogate
            let mut input = Vec::from(b"Hello ");
            input.extend_from_slice(&[0xED, 0xA0, 0x80]); // U+D800
            input.extend_from_slice(b" world");

            let result = validate_utf8(&input);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::SurrogateCodepoint);
            assert_eq!(err.offset, 6);
        }

        #[test]
        fn non_surrogate_ed_valid() {
            // U+D7FF (just below surrogates): ED 9F BF
            assert!(validate_utf8(&[0xED, 0x9F, 0xBF]).is_ok());

            // U+E000 (just above surrogates): EE 80 80
            assert!(validate_utf8(&[0xEE, 0x80, 0x80]).is_ok());
        }
    }

    // =========================================================================
    // Out of Range Code Point Tests
    // =========================================================================

    mod out_of_range {
        use super::*;

        #[test]
        fn just_above_max() {
            // U+110000 (first invalid): F4 90 80 80
            let result = validate_utf8(&[0xF4, 0x90, 0x80, 0x80]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::OutOfRangeCodepoint);
        }

        #[test]
        fn max_valid() {
            // U+10FFFF (last valid): F4 8F BF BF
            assert!(validate_utf8(&[0xF4, 0x8F, 0xBF, 0xBF]).is_ok());
        }

        #[test]
        fn very_high_codepoints() {
            // Various out-of-range code points
            // U+1FFFFF: F7 BF BF BF
            let result = validate_utf8(&[0xF7, 0xBF, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OutOfRangeCodepoint);

            // U+13FFFF: F4 BF BF BF
            let result = validate_utf8(&[0xF4, 0xBF, 0xBF, 0xBF]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::OutOfRangeCodepoint);
        }
    }

    // =========================================================================
    // Truncated Sequence Tests
    // =========================================================================

    mod truncated_sequences {
        use super::*;

        #[test]
        fn truncated_2byte() {
            // 2-byte lead with no continuation
            let result = validate_utf8(&[0xC2]);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::TruncatedSequence);
            assert_eq!(err.offset, 0);
        }

        #[test]
        fn truncated_3byte_1() {
            // 3-byte lead with no continuations
            let result = validate_utf8(&[0xE0]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_3byte_2() {
            // 3-byte lead with only 1 continuation
            let result = validate_utf8(&[0xE0, 0xA0]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_4byte_1() {
            // 4-byte lead with no continuations
            let result = validate_utf8(&[0xF0]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_4byte_2() {
            // 4-byte lead with only 1 continuation
            let result = validate_utf8(&[0xF0, 0x90]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_4byte_3() {
            // 4-byte lead with only 2 continuations
            let result = validate_utf8(&[0xF0, 0x90, 0x80]);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::TruncatedSequence);
        }

        #[test]
        fn truncated_after_valid() {
            // Valid text followed by truncated sequence
            let mut input = Vec::from(b"Hello ");
            input.push(0xC2); // Truncated 2-byte sequence

            let result = validate_utf8(&input);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind, Utf8ErrorKind::TruncatedSequence);
            assert_eq!(err.offset, 6);
        }
    }

    // =========================================================================
    // Error Position Tests
    // =========================================================================

    mod error_positions {
        use super::*;

        #[test]
        fn line_and_column_first_byte() {
            let result = validate_utf8(&[0x80]);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 0);
            assert_eq!(err.line, 1);
            assert_eq!(err.column, 1);
        }

        #[test]
        fn line_and_column_after_ascii() {
            let input = b"Hello\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 5);
            assert_eq!(err.line, 1);
            assert_eq!(err.column, 6);
        }

        #[test]
        fn line_and_column_second_line() {
            let input = b"Hello\nWorld\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 11);
            assert_eq!(err.line, 2);
            assert_eq!(err.column, 6);
        }

        #[test]
        fn line_and_column_third_line() {
            let input = b"Line 1\nLine 2\nLine \x80 3";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 19);
            assert_eq!(err.line, 3);
            assert_eq!(err.column, 6);
        }

        #[test]
        fn line_and_column_after_multibyte() {
            // "日本" followed by invalid byte
            let mut input = "日本".as_bytes().to_vec();
            input.push(0x80);
            let result = validate_utf8(&input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 6); // After 6 bytes (2 × 3-byte chars)
            assert_eq!(err.line, 1);
            assert_eq!(err.column, 7);
        }

        #[test]
        fn line_after_crlf() {
            let input = b"Line 1\r\nLine 2\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            // Line should be 2 (after \n)
            assert_eq!(err.line, 2);
        }

        #[test]
        fn multiple_newlines() {
            let input = b"\n\n\n\n\x80";
            let result = validate_utf8(input);
            let err = result.unwrap_err();
            assert_eq!(err.offset, 4);
            assert_eq!(err.line, 5);
            assert_eq!(err.column, 1);
        }
    }

    // =========================================================================
    // Decode/Encode Tests
    // =========================================================================

    mod decode_encode {
        use super::*;

        #[test]
        fn decode_ascii() {
            assert_eq!(decode_code_point(b"A"), Some((0x41, 1)));
            assert_eq!(decode_code_point(b"\x00"), Some((0x00, 1)));
            assert_eq!(decode_code_point(b"\x7F"), Some((0x7F, 1)));
        }

        #[test]
        fn decode_2byte() {
            assert_eq!(decode_code_point(&[0xC2, 0x80]), Some((0x80, 2)));
            assert_eq!(decode_code_point(&[0xDF, 0xBF]), Some((0x7FF, 2)));
            assert_eq!(decode_code_point("é".as_bytes()), Some((0xE9, 2)));
        }

        #[test]
        fn decode_3byte() {
            assert_eq!(decode_code_point(&[0xE0, 0xA0, 0x80]), Some((0x800, 3)));
            assert_eq!(decode_code_point("日".as_bytes()), Some((0x65E5, 3)));
            assert_eq!(decode_code_point("€".as_bytes()), Some((0x20AC, 3)));
        }

        #[test]
        fn decode_4byte() {
            assert_eq!(
                decode_code_point(&[0xF0, 0x90, 0x80, 0x80]),
                Some((0x10000, 4))
            );
            assert_eq!(decode_code_point("🎉".as_bytes()), Some((0x1F389, 4)));
        }

        #[test]
        fn decode_invalid() {
            assert_eq!(decode_code_point(b""), None);
            assert_eq!(decode_code_point(&[0x80]), None); // Bare continuation
            assert_eq!(decode_code_point(&[0xC2]), None); // Truncated
            assert_eq!(decode_code_point(&[0xC2, 0x00]), None); // Invalid continuation
        }

        #[test]
        fn encode_ascii() {
            let (bytes, len) = encode_code_point(0x41).unwrap();
            assert_eq!(&bytes[..len], b"A");

            let (bytes, len) = encode_code_point(0x00).unwrap();
            assert_eq!(&bytes[..len], b"\x00");
        }

        #[test]
        fn encode_2byte() {
            let (bytes, len) = encode_code_point(0x80).unwrap();
            assert_eq!(&bytes[..len], &[0xC2, 0x80]);

            let (bytes, len) = encode_code_point(0xE9).unwrap(); // é
            assert_eq!(&bytes[..len], "é".as_bytes());
        }

        #[test]
        fn encode_3byte() {
            let (bytes, len) = encode_code_point(0x800).unwrap();
            assert_eq!(&bytes[..len], &[0xE0, 0xA0, 0x80]);

            let (bytes, len) = encode_code_point(0x65E5).unwrap(); // 日
            assert_eq!(&bytes[..len], "日".as_bytes());
        }

        #[test]
        fn encode_4byte() {
            let (bytes, len) = encode_code_point(0x10000).unwrap();
            assert_eq!(&bytes[..len], &[0xF0, 0x90, 0x80, 0x80]);

            let (bytes, len) = encode_code_point(0x1F389).unwrap(); // 🎉
            assert_eq!(&bytes[..len], "🎉".as_bytes());
        }

        #[test]
        fn encode_invalid() {
            // Surrogate
            assert!(encode_code_point(0xD800).is_none());
            assert!(encode_code_point(0xDFFF).is_none());

            // Out of range
            assert!(encode_code_point(0x110000).is_none());
            assert!(encode_code_point(0xFFFFFFFF).is_none());
        }

        #[test]
        fn roundtrip() {
            // Test roundtrip encoding/decoding
            let test_points = [
                0x00, 0x41, 0x7F, // ASCII
                0x80, 0xFF, 0x7FF, // 2-byte
                0x800, 0x65E5, 0xFFFF, // 3-byte
                0x10000, 0x1F389, 0x10FFFF, // 4-byte
            ];

            for cp in test_points {
                let (encoded, len) = encode_code_point(cp).unwrap();
                let (decoded, decoded_len) = decode_code_point(&encoded[..len]).unwrap();
                assert_eq!(cp, decoded);
                assert_eq!(len, decoded_len);
            }
        }
    }

    // =========================================================================
    // Sequence Length Tests
    // =========================================================================

    mod sequence_length_tests {
        use super::*;

        #[test]
        fn ascii_length() {
            for byte in 0x00..=0x7F {
                assert_eq!(sequence_length(byte), 1);
            }
        }

        #[test]
        fn continuation_length() {
            for byte in 0x80..=0xBF {
                assert_eq!(sequence_length(byte), 0); // Invalid as lead
            }
        }

        #[test]
        fn two_byte_length() {
            for byte in 0xC0..=0xDF {
                assert_eq!(sequence_length(byte), 2);
            }
        }

        #[test]
        fn three_byte_length() {
            for byte in 0xE0..=0xEF {
                assert_eq!(sequence_length(byte), 3);
            }
        }

        #[test]
        fn four_byte_length() {
            for byte in 0xF0..=0xF7 {
                assert_eq!(sequence_length(byte), 4);
            }
        }

        #[test]
        fn invalid_lead_length() {
            for byte in 0xF8..=0xFF {
                assert_eq!(sequence_length(byte), 0);
            }
        }
    }

    // =========================================================================
    // Edge Cases and Stress Tests
    // =========================================================================

    mod edge_cases {
        use super::*;

        #[test]
        fn all_same_continuation() {
            // Many continuation bytes in a row
            let input = vec![0x80; 100];
            let result = validate_utf8(&input);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind, Utf8ErrorKind::InvalidLeadByte);
        }

        #[test]
        fn alternating_valid_invalid() {
            // Valid character followed by invalid, repeated
            let mut input = vec![b'A'; 10];
            input.push(0x80); // First invalid

            let result = validate_utf8(&input);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 10);
        }

        #[test]
        fn many_emoji() {
            // Many 4-byte sequences
            let emoji = "🎉🚀🌍💻🔥";
            let repeated: String = emoji.repeat(100);
            assert!(validate_utf8(repeated.as_bytes()).is_ok());
        }

        #[test]
        fn mixed_newline_styles() {
            let input = "Line 1\nLine 2\r\nLine 3\rLine 4";
            assert!(validate_utf8(input.as_bytes()).is_ok());
        }

        #[test]
        fn null_bytes() {
            // Null bytes are valid ASCII
            assert!(validate_utf8(&[0x00, 0x00, 0x00]).is_ok());
            assert!(validate_utf8(b"Hello\x00World").is_ok());
        }

        #[test]
        fn only_newlines() {
            assert!(validate_utf8(b"\n\n\n\n\n").is_ok());
            assert!(validate_utf8(b"\r\n\r\n\r\n").is_ok());
        }

        #[test]
        fn long_lines() {
            // Very long line without newlines
            let long_line = "A".repeat(10000);
            assert!(validate_utf8(long_line.as_bytes()).is_ok());
        }

        #[test]
        fn invalid_at_various_offsets() {
            // Test invalid byte at different positions
            for offset in [0, 1, 7, 15, 31, 63, 64, 65, 100, 127, 128, 255, 256] {
                let mut input = vec![b'A'; offset + 1];
                input[offset] = 0x80;

                let result = validate_utf8(&input);
                assert!(result.is_err());
                assert_eq!(result.unwrap_err().offset, offset);
            }
        }

        #[test]
        fn boundary_64_bytes() {
            // Test around 64-byte boundary (common SIMD width)
            let mut input = vec![b'A'; 64];
            assert!(validate_utf8(&input).is_ok());

            input.push(0x80);
            let result = validate_utf8(&input);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().offset, 64);
        }

        #[test]
        fn boundary_chunk_crossing() {
            // Multi-byte sequence crossing 64-byte boundary
            let mut input = vec![b'A'; 63];
            // Add a 3-byte character that crosses boundary
            input.extend_from_slice("日".as_bytes());
            assert!(validate_utf8(&input).is_ok());
        }
    }

    // =========================================================================
    // Comparison with std::str
    // =========================================================================

    mod std_comparison {
        use super::*;

        #[test]
        fn agree_on_valid_strings() {
            let test_cases = [
                "",
                "Hello, world!",
                "日本語",
                "🎉🚀🌍",
                "Mixed: café 日本 🎉",
                "\n\t\r",
                "\x00\x01\x02",
            ];

            for s in test_cases {
                let our_result = validate_utf8(s.as_bytes());
                assert!(our_result.is_ok(), "Should agree {s} is valid");
            }
        }

        #[test]
        fn agree_on_invalid_bytes() {
            let test_cases: &[&[u8]] = &[
                &[0x80],                   // Bare continuation
                &[0xC2],                   // Truncated 2-byte
                &[0xE0, 0x80],             // Truncated 3-byte
                &[0xC0, 0x80],             // Overlong
                &[0xED, 0xA0, 0x80],       // Surrogate
                &[0xF4, 0x90, 0x80, 0x80], // Out of range
            ];

            for bytes in test_cases {
                let our_result = validate_utf8(bytes);
                let std_result = core::str::from_utf8(bytes);

                assert!(
                    our_result.is_err() && std_result.is_err(),
                    "Should agree {bytes:?} is invalid"
                );
            }
        }
    }

    // =========================================================================
    // Error formatting and code-point decoding
    // =========================================================================

    mod error_formatting {
        use super::*;

        #[test]
        fn error_kind_display_covers_all_variants() {
            let cases = [
                (Utf8ErrorKind::InvalidLeadByte, "invalid UTF-8 lead byte"),
                (
                    Utf8ErrorKind::InvalidContinuationByte,
                    "invalid UTF-8 continuation byte",
                ),
                (Utf8ErrorKind::OverlongEncoding, "overlong UTF-8 encoding"),
                (
                    Utf8ErrorKind::SurrogateCodepoint,
                    "surrogate code point in UTF-8",
                ),
                (
                    Utf8ErrorKind::OutOfRangeCodepoint,
                    "code point above U+10FFFF",
                ),
                (Utf8ErrorKind::TruncatedSequence, "truncated UTF-8 sequence"),
            ];
            for (kind, expected) in cases {
                assert_eq!(alloc::format!("{kind}"), expected);
            }
        }

        #[test]
        fn error_display_includes_position_and_kind() {
            let err = Utf8Error {
                offset: 5,
                line: 2,
                column: 3,
                kind: Utf8ErrorKind::InvalidLeadByte,
            };
            let text = alloc::format!("{err}");
            assert!(text.contains("invalid UTF-8 lead byte"), "{text}");
            assert!(text.contains("byte 5"), "{text}");
            assert!(text.contains("line 2"), "{text}");
            assert!(text.contains("column 3"), "{text}");
        }

        #[test]
        fn format_byte_distinguishes_graphic_and_non_graphic() {
            assert_eq!(format_byte(b'A'), "0x41 ('A')");
            assert_eq!(format_byte(b' '), "0x20 (' ')");
            // Non-graphic bytes render as bare hex.
            assert_eq!(format_byte(0x00), "0x00");
            assert_eq!(format_byte(0x80), "0x80");
            assert_eq!(format_byte(0x0A), "0x0A");
        }
    }

    mod decode_code_point_tests {
        use super::*;

        #[test]
        fn decodes_all_sequence_lengths() {
            assert_eq!(decode_code_point(b"A"), Some((0x41, 1)));
            assert_eq!(decode_code_point("\u{00E9}".as_bytes()), Some((0x00E9, 2)));
            assert_eq!(decode_code_point("\u{20AC}".as_bytes()), Some((0x20AC, 3)));
            assert_eq!(
                decode_code_point("\u{1F389}".as_bytes()),
                Some((0x1F389, 4))
            );
        }

        #[test]
        fn rejects_empty_and_truncated() {
            assert_eq!(decode_code_point(b""), None);
            // 3-byte lead with only one following byte -> too short.
            assert_eq!(decode_code_point(&[0xE2, 0x82]), None);
        }

        #[test]
        fn rejects_bad_continuation_bytes() {
            // 2-byte lead, non-continuation second byte.
            assert_eq!(decode_code_point(&[0xC3, 0x28]), None);
            // 3-byte lead, bad first / second continuation byte.
            assert_eq!(decode_code_point(&[0xE2, 0x28, 0xA1]), None);
            assert_eq!(decode_code_point(&[0xE2, 0x82, 0x28]), None);
            // 4-byte lead, bad continuation bytes in each position.
            assert_eq!(decode_code_point(&[0xF0, 0x28, 0x8C, 0xBC]), None);
            assert_eq!(decode_code_point(&[0xF0, 0x9F, 0x28, 0x8C]), None);
            assert_eq!(decode_code_point(&[0xF0, 0x9F, 0x8E, 0x28]), None);
        }

        #[test]
        fn rejects_invalid_lead_byte() {
            // 0x80 is a continuation byte, never a lead -> sequence_length 0.
            assert_eq!(decode_code_point(&[0x80]), None);
        }
    }
}

/// Differential tests: every validation path must agree with the standard
/// library (`core::str::from_utf8`) on validity. The AVX2 kernel was written
/// from first principles, so this is its correctness gate — a false accept (the
/// only unsafe failure mode) surfaces here as a disagreement with std.
#[cfg(test)]
mod validate_utf8_differential_tests {
    #![allow(unsafe_code)] // the x86_64 test drives the raw AVX2 kernel directly

    use super::{validate_utf8, validate_utf8_scalar, Utf8Error, Utf8ErrorKind};
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Tiny deterministic xorshift64 RNG — keeps the corpus reproducible without
    /// a dev-dependency on `rand`.
    struct Rng(u64);
    impl Rng {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn next_u32(&mut self) -> u32 {
            (self.next_u64() >> 32) as u32
        }
        fn below(&mut self, n: u32) -> u32 {
            self.next_u32() % n
        }
    }

    /// `len` uniformly random bytes.
    fn random_bytes(rng: &mut Rng, len: usize) -> Vec<u8> {
        (0..len).map(|_| (rng.next_u32() & 0xFF) as u8).collect()
    }

    /// A pseudo-random *valid* UTF-8 buffer of at least `min_len` bytes, mixing
    /// all four sequence lengths. `char::from_u32` guarantees validity by
    /// rejecting surrogates and out-of-range code points.
    fn random_valid_utf8(rng: &mut Rng, min_len: usize) -> Vec<u8> {
        let mut s = String::new();
        while s.len() < min_len {
            let cp = match rng.below(4) {
                0 => rng.below(0x80),
                1 => 0x80 + rng.below(0x800 - 0x80),
                2 => 0x800 + rng.below(0x1_0000 - 0x800),
                _ => 0x1_0000 + rng.below(0x11_0000 - 0x1_0000),
            };
            if let Some(c) = char::from_u32(cp) {
                s.push(c);
            }
        }
        s.into_bytes()
    }

    /// Portable twin of the AVX2 kernel's *logic*: the identical per-byte
    /// predicate (`is_cont` vs `must_cont`, the never-valid bytes, and the E0/
    /// ED/F0/F4 special cases), evaluated over positions `0..=len` — where index
    /// `len` is the first byte of the kernel's always-run zero-padded tail block,
    /// which is what catches end-of-input truncation. Out-of-range indices read
    /// as `0`, matching the zero fill.
    ///
    /// Testing this against `core::str::from_utf8` validates the block algorithm
    /// on non-x86 hosts; any divergence localized purely to the intrinsics is
    /// left for the x86 [`avx2_kernel_matches_std`] leg.
    fn avx2_logic_reference(input: &[u8]) -> bool {
        let len = input.len();
        if len == 0 {
            return true;
        }
        let at = |i: isize| -> u8 {
            if i < 0 || i as usize >= len {
                0
            } else {
                input[i as usize]
            }
        };
        for i in 0..=len as isize {
            let c = at(i);
            let p1 = at(i - 1);
            let p2 = at(i - 2);
            let p3 = at(i - 3);
            let is_cont = (c & 0xC0) == 0x80;
            let must_cont = p1 >= 0xC0 || p2 >= 0xE0 || p3 >= 0xF0;
            let err = (is_cont != must_cont)
                || (c & 0xFE) == 0xC0 // 0xC0 / 0xC1 (overlong 2-byte lead)
                || c >= 0xF5 // 0xF5..=0xFF (never a valid lead)
                || (p1 == 0xE0 && c < 0xA0) // overlong 3-byte
                || (p1 == 0xED && c >= 0xA0) // surrogate
                || (p1 == 0xF0 && c < 0x90) // overlong 4-byte
                || (p1 == 0xF4 && c >= 0x90); // above U+10FFFF
            if err {
                return false;
            }
        }
        true
    }

    /// The block algorithm (portable twin of the AVX2 kernel) agrees with std
    /// over a large corpus. Runs on every architecture, so it gates the kernel's
    /// *logic* even where AVX2 can't execute.
    #[test]
    fn avx2_logic_matches_std() {
        let mut rng = Rng(0x5eed_1234_9999_0001);
        for _ in 0..60_000 {
            let len = rng.below(140) as usize;
            let bytes = random_bytes(&mut rng, len);
            assert_eq!(
                avx2_logic_reference(&bytes),
                core::str::from_utf8(&bytes).is_ok(),
                "block logic disagreed with std on {bytes:02x?}"
            );
        }
        for _ in 0..10_000 {
            let min_len = rng.below(300) as usize;
            let mut bytes = random_valid_utf8(&mut rng, min_len);
            if !bytes.is_empty() && rng.below(2) == 0 {
                let i = rng.below(bytes.len() as u32) as usize;
                bytes[i] = (rng.next_u32() & 0xFF) as u8;
            }
            assert_eq!(
                avx2_logic_reference(&bytes),
                core::str::from_utf8(&bytes).is_ok(),
            );
        }
    }

    /// Invalid fixtures: one per error kind, placed at offsets that straddle the
    /// 32/64-byte SIMD block edges and land at end-of-input.
    fn invalid_boundary_fixtures() -> Vec<Vec<u8>> {
        let bad_seqs: &[&[u8]] = &[
            &[0x80],                   // stray continuation
            &[0xC0, 0x80],             // overlong 2-byte
            &[0xC1, 0xBF],             // overlong 2-byte
            &[0xE0, 0x80, 0x80],       // overlong 3-byte
            &[0xED, 0xA0, 0x80],       // surrogate U+D800
            &[0xF0, 0x80, 0x80, 0x80], // overlong 4-byte
            &[0xF4, 0x90, 0x80, 0x80], // above U+10FFFF
            &[0xF5],                   // invalid lead
            &[0xFF],                   // invalid lead
            &[0xC2],                   // truncated 2-byte at EOF
            &[0xE0, 0xA0],             // truncated 3-byte at EOF
            &[0xF0, 0x90, 0x80],       // truncated 4-byte at EOF
        ];
        let offsets = [0usize, 1, 30, 31, 32, 33, 62, 63, 64, 65];
        let mut out = Vec::new();
        for seq in bad_seqs {
            for &off in &offsets {
                let mut buf: Vec<u8> = core::iter::repeat(b'a').take(off).collect();
                buf.extend_from_slice(seq);
                out.push(buf);
            }
        }
        out
    }

    /// Valid fixtures: valid multi-byte sequences straddling block edges (guards
    /// against false rejects at the cross-block `prev1/prev2/prev3` carry).
    fn valid_boundary_fixtures() -> Vec<Vec<u8>> {
        let good_seqs: &[&[u8]] = &[
            "é".as_bytes(),          // 2-byte
            "€".as_bytes(),          // 3-byte
            "😀".as_bytes(),         // 4-byte
            "\u{D7FF}".as_bytes(),   // just below surrogate range (3-byte)
            "\u{10FFFF}".as_bytes(), // maximum code point (4-byte)
        ];
        let offsets = [0usize, 1, 29, 30, 31, 32, 33, 60, 61, 62, 63, 64, 65];
        let mut out = Vec::new();
        for seq in good_seqs {
            for &off in &offsets {
                let mut buf: Vec<u8> = core::iter::repeat(b'a').take(off).collect();
                buf.extend_from_slice(seq);
                buf.push(b'z');
                out.push(buf);
            }
        }
        out
    }

    /// Portable oracle: the scalar validator agrees with std on validity for
    /// random byte soup (lengths straddle the 32/64-byte SIMD block edges) and
    /// for valid UTF-8 with occasional single-byte corruption.
    #[test]
    fn scalar_matches_std() {
        let mut rng = Rng(0x1234_5678_9abc_def0);
        for _ in 0..20_000 {
            let len = rng.below(80) as usize;
            let bytes = random_bytes(&mut rng, len);
            assert_eq!(
                validate_utf8_scalar(&bytes).is_ok(),
                core::str::from_utf8(&bytes).is_ok(),
                "scalar disagreed with std on {bytes:02x?}"
            );
        }
        for _ in 0..5_000 {
            let min_len = rng.below(200) as usize;
            let mut bytes = random_valid_utf8(&mut rng, min_len);
            if !bytes.is_empty() && rng.below(2) == 0 {
                let i = rng.below(bytes.len() as u32) as usize;
                bytes[i] ^= 0xFF;
            }
            assert_eq!(
                validate_utf8_scalar(&bytes).is_ok(),
                core::str::from_utf8(&bytes).is_ok(),
            );
        }
    }

    /// The public dispatcher agrees with the scalar validator on validity AND
    /// returns a byte-identical `Utf8Error` when both fail (the SIMD path falls
    /// back to the scalar validator for diagnostics, so `Result`s must match).
    #[test]
    fn dispatch_matches_scalar_including_error() {
        let mut rng = Rng(0x0f0f_0f0f_dead_beef);
        for _ in 0..20_000 {
            let len = rng.below(96) as usize;
            let bytes = random_bytes(&mut rng, len);
            assert_eq!(validate_utf8(&bytes), validate_utf8_scalar(&bytes));
        }
    }

    /// The AVX2 kernel itself agrees with std over a large random corpus and the
    /// targeted boundary fixtures — catching both false accepts and false
    /// rejects. The whole test skips on the rare non-AVX2 x86 host (a single
    /// guard, rather than a per-input check, keeps this coverage-clean).
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn avx2_kernel_matches_std() {
        if !is_x86_feature_detected!("avx2") {
            eprintln!("avx2_kernel_matches_std: AVX2 unavailable, skipping");
            return;
        }
        let check = |bytes: &[u8]| {
            // SAFETY: AVX2 availability was confirmed above.
            let valid = unsafe { super::simd_x86::validate_utf8_avx2_unchecked(bytes) };
            assert_eq!(
                valid,
                core::str::from_utf8(bytes).is_ok(),
                "AVX2 kernel disagreed with std on {bytes:02x?}"
            );
            // If these ever differ, the bug is in the intrinsics, not the logic.
            assert_eq!(
                valid,
                avx2_logic_reference(bytes),
                "AVX2 kernel disagreed with its portable twin on {bytes:02x?}"
            );
        };

        let mut rng = Rng(0xcafe_babe_1337_0042);
        for _ in 0..50_000 {
            let len = rng.below(140) as usize;
            check(&random_bytes(&mut rng, len));
        }
        for _ in 0..10_000 {
            let min_len = rng.below(300) as usize;
            let mut bytes = random_valid_utf8(&mut rng, min_len);
            if !bytes.is_empty() && rng.below(2) == 0 {
                let i = rng.below(bytes.len() as u32) as usize;
                bytes[i] = (rng.next_u32() & 0xFF) as u8;
            }
            check(&bytes);
        }
        for buf in invalid_boundary_fixtures() {
            check(&buf);
        }
        for buf in valid_boundary_fixtures() {
            check(&buf);
        }
    }

    /// Every error kind, injected at offsets straddling 32/64-byte block edges
    /// and at end-of-input, must be rejected by std and the scalar validator.
    /// (The AVX2 kernel runs the same fixtures in `avx2_kernel_matches_std`.)
    #[test]
    fn boundary_error_matrix() {
        for buf in invalid_boundary_fixtures() {
            assert!(
                core::str::from_utf8(&buf).is_err(),
                "fixture should be invalid: {buf:02x?}"
            );
            assert!(validate_utf8_scalar(&buf).is_err(), "scalar: {buf:02x?}");
        }
    }

    /// Valid multi-byte sequences straddling block edges must be accepted by std
    /// and the scalar validator.
    #[test]
    fn boundary_valid_multibyte() {
        for buf in valid_boundary_fixtures() {
            assert!(core::str::from_utf8(&buf).is_ok());
            assert!(validate_utf8_scalar(&buf).is_ok(), "scalar: {buf:02x?}");
        }
    }

    /// Invalid fixtures carrying newlines, so the `line`/`column` fields of the
    /// reported [`Utf8Error`] — not just its `offset` — are exercised.
    ///
    /// Newlines are placed at strides that straddle the 8-byte word boundary the
    /// broadword ASCII fast path steps over, and errors land on the first line,
    /// immediately after a newline, and several lines in.
    fn newline_error_fixtures() -> Vec<Vec<u8>> {
        let bad_seqs: &[&[u8]] = &[
            &[0x80],                   // stray continuation
            &[0xC0, 0x80],             // overlong 2-byte
            &[0xE0, 0x80, 0x80],       // overlong 3-byte
            &[0xED, 0xA0, 0x80],       // surrogate U+D800
            &[0xF0, 0x80, 0x80, 0x80], // overlong 4-byte
            &[0xF4, 0x90, 0x80, 0x80], // above U+10FFFF
            &[0xFF],                   // invalid lead
            &[0xC2],                   // truncated 2-byte at EOF
            &[0xE0, 0xA0],             // truncated 3-byte at EOF
            &[0xF0, 0x90, 0x80],       // truncated 4-byte at EOF
            &[0xC2, 0x41],             // bad continuation in 2-byte
            &[0xE0, 0xA0, 0x41],       // bad continuation in 3-byte
            &[0xF0, 0x90, 0x80, 0x41], // bad continuation in 4-byte
        ];

        // Prefix shapes, each `n` bytes long, covering: no newline at all; a
        // newline every k bytes (k straddling the 8-byte word); a single newline
        // at the very start; and one immediately before the error.
        let prefix = |n: usize, shape: u8| -> Vec<u8> {
            (0..n)
                .map(|i| match shape {
                    0 => b'a',
                    1 => {
                        if i % 3 == 2 {
                            b'\n'
                        } else {
                            b'a'
                        }
                    }
                    2 => {
                        if i % 8 == 7 {
                            b'\n'
                        } else {
                            b'a'
                        }
                    }
                    3 => {
                        if i == 0 {
                            b'\n'
                        } else {
                            b'a'
                        }
                    }
                    _ => {
                        if i + 1 == n {
                            b'\n'
                        } else {
                            b'a'
                        }
                    }
                })
                .collect()
        };

        let mut out = Vec::new();
        for seq in bad_seqs {
            for n in 0..24usize {
                for shape in 0..5u8 {
                    let mut buf = prefix(n, shape);
                    buf.extend_from_slice(seq);
                    out.push(buf);
                }
            }
            // Multi-byte characters before the error: the derived `line_start`
            // must not be confused by continuation bytes.
            for n in 0..6usize {
                let mut buf = Vec::new();
                for i in 0..n {
                    buf.extend_from_slice(if i % 2 == 0 {
                        "日".as_bytes()
                    } else {
                        "é\n".as_bytes()
                    });
                }
                buf.extend_from_slice(seq);
                out.push(buf);
            }
            // CRLF line endings.
            for n in 0..4usize {
                let mut buf = Vec::new();
                for _ in 0..n {
                    buf.extend_from_slice(b"line\r\n");
                }
                buf.extend_from_slice(seq);
                out.push(buf);
            }
        }
        out
    }

    /// The scalar validator agrees with the byte-by-byte reference on the *whole*
    /// `Result` — `offset`, `line`, `column` and `kind`, not merely validity.
    ///
    /// This is the safety net for deriving `line`/`column` from the error offset
    /// instead of tracking them per byte: any drift in either field fails here.
    #[test]
    fn scalar_matches_reference_including_error() {
        // Widest `line` / `column` an errored fixture reported, asserted at the
        // end so this test can never pass vacuously on all-valid or all-line-1
        // inputs — the very cases that would hide a line-tracking regression.
        let mut max_line = 0usize;
        let mut max_column = 0usize;
        let mut check = |bytes: &[u8]| {
            let actual = validate_utf8_scalar(bytes);
            assert_eq!(
                actual,
                validate_utf8_scalar_reference(bytes),
                "scalar diverged from reference on {bytes:02x?}"
            );
            if let Err(e) = actual {
                max_line = max_line.max(e.line);
                max_column = max_column.max(e.column);
            }
        };

        for buf in newline_error_fixtures() {
            check(&buf);
        }
        for buf in invalid_boundary_fixtures() {
            check(&buf);
        }
        for buf in valid_boundary_fixtures() {
            check(&buf);
        }

        let mut rng = Rng(0xfeed_face_0133_0001);
        for _ in 0..20_000 {
            let len = rng.below(96) as usize;
            check(&random_bytes(&mut rng, len));
        }
        // Byte soup biased towards ASCII and newlines, so long ASCII runs (the
        // broadword fast path) precede the error rather than random high bytes.
        for _ in 0..20_000 {
            let len = rng.below(96) as usize;
            let mut bytes: Vec<u8> = (0..len)
                .map(|_| match rng.below(10) {
                    0 => b'\n',
                    1 => (rng.next_u32() & 0xFF) as u8,
                    _ => b'a' + (rng.below(26) as u8),
                })
                .collect();
            if !bytes.is_empty() && rng.below(2) == 0 {
                let i = rng.below(bytes.len() as u32) as usize;
                bytes[i] = 0x80 | (rng.below(0x80) as u8);
            }
            check(&bytes);
        }
        for _ in 0..5_000 {
            let min_len = rng.below(200) as usize;
            let mut bytes = random_valid_utf8(&mut rng, min_len);
            if !bytes.is_empty() {
                let i = rng.below(bytes.len() as u32) as usize;
                bytes[i] = if rng.below(2) == 0 {
                    b'\n'
                } else {
                    (rng.next_u32() & 0xFF) as u8
                };
            }
            check(&bytes);
        }

        assert!(
            max_line > 5,
            "fixtures never produced an error past line 5 (max {max_line}); \
             line tracking is not actually being exercised"
        );
        assert!(
            max_column > 5,
            "fixtures never produced an error past column 5 (max {max_column}); \
             column derivation is not actually being exercised"
        );
    }

    /// The original byte-at-a-time scalar validator, kept verbatim as the oracle
    /// for [`scalar_matches_reference_including_error`]. It tracks `line` and
    /// `line_start` incrementally on the hot path; the shipping validator derives
    /// them from the error offset instead, and this pins the two to agree.
    // STYLE-0005: reference impl kept for correctness comparison
    fn validate_utf8_scalar_reference(input: &[u8]) -> Result<(), Utf8Error> {
        let mut pos = 0;
        let mut line = 1;
        let mut line_start = 0;
        let len = input.len();

        while pos < len {
            let byte = input[pos];

            if pos > 0 && input[pos - 1] == b'\n' {
                line += 1;
                line_start = pos;
            }

            let seq_len = match byte {
                0x00..=0x7F => {
                    pos += 1;
                    continue;
                }
                0x80..=0xBF => {
                    return Err(Utf8Error {
                        offset: pos,
                        line,
                        column: pos - line_start + 1,
                        kind: Utf8ErrorKind::InvalidLeadByte,
                    });
                }
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                0xF8..=0xFF => {
                    return Err(Utf8Error {
                        offset: pos,
                        line,
                        column: pos - line_start + 1,
                        kind: Utf8ErrorKind::InvalidLeadByte,
                    });
                }
            };

            if pos + seq_len > len {
                return Err(Utf8Error {
                    offset: pos,
                    line,
                    column: pos - line_start + 1,
                    kind: Utf8ErrorKind::TruncatedSequence,
                });
            }

            match seq_len {
                2 => {
                    let b1 = input[pos + 1];
                    if (b1 & 0xC0) != 0x80 {
                        return Err(Utf8Error {
                            offset: pos + 1,
                            line,
                            column: pos + 1 - line_start + 1,
                            kind: Utf8ErrorKind::InvalidContinuationByte,
                        });
                    }
                    if byte <= 0xC1 {
                        return Err(Utf8Error {
                            offset: pos,
                            line,
                            column: pos - line_start + 1,
                            kind: Utf8ErrorKind::OverlongEncoding,
                        });
                    }
                }
                3 => {
                    let b1 = input[pos + 1];
                    let b2 = input[pos + 2];
                    if (b1 & 0xC0) != 0x80 {
                        return Err(Utf8Error {
                            offset: pos + 1,
                            line,
                            column: pos + 1 - line_start + 1,
                            kind: Utf8ErrorKind::InvalidContinuationByte,
                        });
                    }
                    if (b2 & 0xC0) != 0x80 {
                        return Err(Utf8Error {
                            offset: pos + 2,
                            line,
                            column: pos + 2 - line_start + 1,
                            kind: Utf8ErrorKind::InvalidContinuationByte,
                        });
                    }
                    let cp = ((byte as u32 & 0x0F) << 12)
                        | ((b1 as u32 & 0x3F) << 6)
                        | (b2 as u32 & 0x3F);
                    if cp < 0x800 {
                        return Err(Utf8Error {
                            offset: pos,
                            line,
                            column: pos - line_start + 1,
                            kind: Utf8ErrorKind::OverlongEncoding,
                        });
                    }
                    if (0xD800..=0xDFFF).contains(&cp) {
                        return Err(Utf8Error {
                            offset: pos,
                            line,
                            column: pos - line_start + 1,
                            kind: Utf8ErrorKind::SurrogateCodepoint,
                        });
                    }
                }
                4 => {
                    let b1 = input[pos + 1];
                    let b2 = input[pos + 2];
                    let b3 = input[pos + 3];
                    if (b1 & 0xC0) != 0x80 {
                        return Err(Utf8Error {
                            offset: pos + 1,
                            line,
                            column: pos + 1 - line_start + 1,
                            kind: Utf8ErrorKind::InvalidContinuationByte,
                        });
                    }
                    if (b2 & 0xC0) != 0x80 {
                        return Err(Utf8Error {
                            offset: pos + 2,
                            line,
                            column: pos + 2 - line_start + 1,
                            kind: Utf8ErrorKind::InvalidContinuationByte,
                        });
                    }
                    if (b3 & 0xC0) != 0x80 {
                        return Err(Utf8Error {
                            offset: pos + 3,
                            line,
                            column: pos + 3 - line_start + 1,
                            kind: Utf8ErrorKind::InvalidContinuationByte,
                        });
                    }
                    let cp = ((byte as u32 & 0x07) << 18)
                        | ((b1 as u32 & 0x3F) << 12)
                        | ((b2 as u32 & 0x3F) << 6)
                        | (b3 as u32 & 0x3F);
                    if cp < 0x10000 {
                        return Err(Utf8Error {
                            offset: pos,
                            line,
                            column: pos - line_start + 1,
                            kind: Utf8ErrorKind::OverlongEncoding,
                        });
                    }
                    if cp > 0x10FFFF {
                        return Err(Utf8Error {
                            offset: pos,
                            line,
                            column: pos - line_start + 1,
                            kind: Utf8ErrorKind::OutOfRangeCodepoint,
                        });
                    }
                }
                _ => unreachable!(),
            }

            pos += seq_len;
        }

        Ok(())
    }
}
