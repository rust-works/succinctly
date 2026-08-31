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

impl Utf8ErrorKind {
    /// The human-readable reason, as a `&'static str`.
    ///
    /// Split out of [`Display`](core::fmt::Display) (which now defers to it)
    /// so a caller needing the text without allocating shares one definition
    /// with the formatter -- same shape as `JsonError::message` and
    /// `YamlStringError::message`.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidLeadByte => "invalid UTF-8 lead byte",
            Self::InvalidContinuationByte => "invalid UTF-8 continuation byte",
            Self::OverlongEncoding => "overlong UTF-8 encoding",
            Self::SurrogateCodepoint => "surrogate code point in UTF-8",
            Self::OutOfRangeCodepoint => "code point above U+10FFFF",
            Self::TruncatedSequence => "truncated UTF-8 sequence",
        }
    }
}

impl core::fmt::Display for Utf8ErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.message())
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
    // the AVX2 fast path. Everywhere else — aarch64, wasm, riscv, and any
    // `no_std` build — use the scalar validator, which already skips ASCII
    // runs eight bytes at a time (#133). `validate_utf8_broadword` is available
    // separately for callers who know their input is ASCII-dominant: measured
    // against the current scalar validator it wins clearly on long ASCII runs
    // but loses geometric mean across realistic mixed content, so it is not
    // the default — see docs/benchmarks/utf8-validate.md#engine-comparison-134.
    #[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
    {
        validate_utf8_simd(input)
    }
    #[cfg(not(all(target_arch = "x86_64", any(test, feature = "std"))))]
    {
        validate_utf8_scalar(input)
    }
}

/// Which RFC 3629 / Unicode bound (if any) a decoded code point violates for
/// its sequence length -- shared by [`validate_utf8_scalar`] (whole-buffer
/// scan, needs to report *which* rule failed) and
/// [`decode_code_point`](crate::text::utf8::decode_code_point)
/// (single-sequence decode, only needs pass/fail), so the actual bounds
/// (0x80/0x800/0x10000/0x10FFFF/0xD800..=0xDFFF) live in exactly one place
/// rather than silently drifting between the two, as `decode_code_point`
/// once did (#1423, found and fixed after `decode_code_point` had already
/// drifted from this function once).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodePointBoundsViolation {
    Overlong,
    Surrogate,
    OutOfRange,
}

impl From<CodePointBoundsViolation> for Utf8ErrorKind {
    fn from(violation: CodePointBoundsViolation) -> Self {
        match violation {
            CodePointBoundsViolation::Overlong => Self::OverlongEncoding,
            CodePointBoundsViolation::Surrogate => Self::SurrogateCodepoint,
            CodePointBoundsViolation::OutOfRange => Self::OutOfRangeCodepoint,
        }
    }
}

/// Check a decoded code point (from a `len`-byte sequence, `len` in
/// `2..=4`; a 1-byte/ASCII sequence needs no check and isn't a valid input
/// here) against the bound(s) that apply at that length. `None` means the
/// code point is valid.
fn code_point_bounds_violation(cp: u32, len: usize) -> Option<CodePointBoundsViolation> {
    match len {
        2 if cp < 0x80 => Some(CodePointBoundsViolation::Overlong),
        3 if cp < 0x800 => Some(CodePointBoundsViolation::Overlong),
        3 if (0xD800..=0xDFFF).contains(&cp) => Some(CodePointBoundsViolation::Surrogate),
        4 if cp < 0x10000 => Some(CodePointBoundsViolation::Overlong),
        4 if cp > 0x10FFFF => Some(CodePointBoundsViolation::OutOfRange),
        _ => None,
    }
}

/// Whether `lead_byte` (a `2`/`3`/`4`-byte lead per [`sequence_length`]) can
/// *never* pass [`code_point_bounds_violation`], regardless of which
/// continuation bytes follow it -- i.e. RFC 3629 doesn't list it as a valid
/// lead byte at any length. `0xC0`/`0xC1` always decode to `cp < 0x80`
/// (their maximum, with continuation `0xBF`, is `0x7F`); `0xF5`-`0xF7`
/// always decode to `cp > 0x10FFFF` (their minimum, with continuation
/// `0x80`, is `0x140000`). Every other 2/3/4-byte lead (`0xC2`-`0xDF`,
/// `0xE0`-`0xEF`, `0xF0`-`0xF4`) has at least one continuation-byte choice
/// that decodes in range.
///
/// A genuine, standalone classification of `lead_byte` alone -- no
/// precondition on having already seen (or counted) any continuation
/// bytes. [`substitute_invalid_utf8_jq_style`] relies on that: jq treats
/// `0xC0`/`0xC1`/`0xF5`-`0xF7` as a one-byte error unconditionally, even
/// when there isn't enough remaining input to know what the continuation
/// bytes *would* have been (`\xf5\x80` at end-of-input is 2 separate
/// one-byte errors in jq, not one 2-of-4-bytes-present truncated
/// sequence) -- so this must be checked, and answer correctly, before any
/// continuation-byte counting or truncation check runs, not only after a
/// bounds violation is otherwise confirmed. Exists as one named function,
/// rather than an independent byte-range literal at each call site, so it
/// can't silently drift from `code_point_bounds_violation`'s own bounds
/// (see that function's doc comment and #1423 for a case where exactly
/// that happened between two different functions).
#[inline]
fn is_never_valid_lead_byte(lead_byte: u8, seq_len: usize) -> bool {
    match seq_len {
        2 => matches!(lead_byte, 0xC0 | 0xC1),
        4 => lead_byte >= 0xF5,
        _ => false, // seq_len == 3 (0xE0-0xEF): always a valid lead
    }
}

/// Validate UTF-8 using a portable scalar algorithm with a broadword (SWAR)
/// ASCII fast path.
///
/// This is the only validation path on non-x86_64 targets, in `no_std` builds,
/// on x86_64 CPUs without AVX2, and behind the CLI's `--no-simd`; on AVX2 hosts
/// it also backs [`validate_utf8`]'s error reporting.
///
/// Runs of ASCII are skipped eight bytes at a time via `skip_ascii`, while
/// multi-byte sequences are validated one character at a time. Errors carry the
/// exact byte offset, line number, and column position, derived from the offset
/// by `line_and_column` once an error is found — see its docs for why keeping
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

                let cp = ((byte as u32 & 0x1F) << 6) | (b1 as u32 & 0x3F);
                if let Some(violation) = code_point_bounds_violation(cp, 2) {
                    return Err(err_at(input, pos, violation.into()));
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

                let cp =
                    ((byte as u32 & 0x0F) << 12) | ((b1 as u32 & 0x3F) << 6) | (b2 as u32 & 0x3F);
                if let Some(violation) = code_point_bounds_violation(cp, 3) {
                    return Err(err_at(input, pos, violation.into()));
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

                let cp = ((byte as u32 & 0x07) << 18)
                    | ((b1 as u32 & 0x3F) << 12)
                    | ((b2 as u32 & 0x3F) << 6)
                    | (b3 as u32 & 0x3F);
                if let Some(violation) = code_point_bounds_violation(cp, 4) {
                    return Err(err_at(input, pos, violation.into()));
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

#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
mod simd_x86;

mod broadword;

pub use self::broadword::validate_utf8_broadword;

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
/// Returns `None` if the input is empty or contains an invalid sequence --
/// including an overlong encoding (a code point spelled with more bytes
/// than RFC 3629 requires), a surrogate-range code point (U+D800-U+DFFF,
/// reserved for UTF-16 and never valid in UTF-8), or a 4-byte sequence
/// decoding past Unicode's own maximum (U+10FFFF). Shares
/// `code_point_bounds_violation` with [`validate_utf8_scalar`]'s own
/// byte-scan path, so these bounds live in exactly one place rather than
/// silently drifting between the two, which is how this function ended up
/// missing them in the first place (#1423).
///
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
///
/// // Overlong 2-byte encoding of U+0000 -- rejected, not decoded as NUL
/// assert_eq!(decode_code_point(&[0xC0, 0x80]), None);
///
/// // Surrogate code point (U+D800) -- rejected
/// assert_eq!(decode_code_point(&[0xED, 0xA0, 0x80]), None);
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

    if code_point_bounds_violation(cp, len).is_some() {
        return None;
    }

    Some((cp, len))
}

/// Decode the character at `offset` in `bytes` for embedding in an error message.
///
/// Three-way fallback (settled by #1187 for `yaml/parser.rs`'s
/// `err_unexpected_char`, generalized here so `yaml/validate.rs` and
/// `json/validate.rs` can share it instead of each re-deriving their own):
/// a byte that starts a valid, complete UTF-8 sequence decodes to its real
/// character; a byte that's present but doesn't decode (a bare continuation
/// byte, an invalid lead byte, or a sequence truncated before `bytes` ends)
/// falls back to a Latin-1 cast of that one byte, not `'\0'` -- a NUL would
/// silently misrepresent a present-but-malformed byte as absent; only a
/// fully out-of-bounds `offset` (true EOF) is `'\0'`.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::decode_char_at;
///
/// // ASCII
/// assert_eq!(decode_char_at(b"abc", 1), 'b');
///
/// // Multi-byte
/// assert_eq!(decode_char_at("a日b".as_bytes(), 1), '日');
///
/// // Truncated/invalid sequence -- Latin-1 fallback on the lead byte
/// assert_eq!(decode_char_at(&[0x41, 0xE6], 1), 0xE6 as char);
///
/// // Past the end of input -- true EOF
/// assert_eq!(decode_char_at(b"abc", 3), '\0');
/// ```
pub fn decode_char_at(bytes: &[u8], offset: usize) -> char {
    match bytes.get(offset) {
        None => '\0',
        Some(&byte) => bytes
            .get(offset..)
            .and_then(decode_code_point)
            .and_then(|(cp, _len)| char::from_u32(cp))
            .unwrap_or(byte as char),
    }
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

/// Lossily decode `input` as UTF-8, substituting invalid sequences with
/// U+FFFD using jq 1.7.1's own maximal-subpart rule rather than
/// `String::from_utf8_lossy`'s WHATWG rule (#1617).
///
/// The two rules agree on every `Utf8ErrorKind` except a bounds violation
/// (overlong/surrogate/out-of-range) on a *structurally valid* 3- or
/// 4-byte lead byte -- one that RFC 3629 lists as legitimate (`0xE0`-`0xEF`,
/// `0xF0`-`0xF4`), just with a narrowed continuation-byte range. There, jq
/// consumes the whole sequence as one unit (one U+FFFD); WHATWG's maximal
/// subpart restarts after just the lead byte (one U+FFFD per byte), since
/// the sequence's *decoded value*, not its shape, is what's wrong. A bounds
/// violation on a lead RFC 3629 never lists as valid at any length
/// (`0xC0`/`0xC1`, `0xF5`-`0xF7`) is not included in that collapse -- those
/// can never decode in range regardless of continuation bytes, so jq (like
/// WHATWG) treats the lead byte alone as immediately invalid, same as any
/// other bad lead. Live-verified against the pinned jq 1.7.1 binary for
/// every kind, including the case #1617's own issue didn't test
/// (`0xF5`-`0xF7`, which reports the identical `Utf8ErrorKind` as the
/// collapsing `0xF0`-`0xF4` case and would be wrongly collapsed by a rule
/// keyed on the error kind alone).
///
/// The "agree on every other kind" claim above has one more exception,
/// fixed by #1717: for `InvalidContinuationByte`, when `seq_len` bytes
/// are not all physically present from the lead's own position onward
/// (`len - pos < seq_len`) -- whether because `input` genuinely ends
/// early, or because one of the bytes that *is* present fails the
/// continuation check -- real jq collapses the *entire* remaining tail
/// into one U+FFFD, dropping every byte in it, rather than keeping and
/// rescanning whatever came after the invalid one. This function now
/// replicates that (see `invalid_subpart_end`'s own doc comment for the
/// exact condition and a live-verified byte-shape matrix). Likely an
/// off-by-one in jq's own end-of-buffer lookahead rather than a designed
/// rule -- reproduced bug-for-bug per ADR-0018 rule 4, not "fixed" into
/// the more sensible WHATWG-consistent shape.
///
/// This function's own fix is granularity-independent -- it only cares
/// about how many bytes remain in `input` from the lead's own position --
/// so *what a caller passes as `input` is what decides where the quirk
/// fires*. Real jq's own condition is scoped to each JSON string's own
/// decoded bytes (document mode, inside `jv_string_sized`) or to each line
/// (raw-input mode, confirmed live: `printf 'a\xe1\x41\n' | jq -R '.'`
/// drops the byte even though it's followed by the file's own trailing
/// newline). Neither is "the whole buffer's own end" in a realistic
/// multi-field document or multi-line file, and jq's trigger is not rare
/// at all -- it fires on *any* string/line ending in the right byte shape,
/// however much more content follows in the rest of the file -- so a
/// whole-file caller would essentially never reproduce it.
///
/// Every caller is therefore scoped the way jq scopes it:
/// `@base64d`/`@urid` are already handed one decoded string (#1719),
/// `jq_runner.rs`'s `--raw-input` decode splits on `\n` first (#1742),
/// and JSON document input goes through
/// [`substitute_invalid_utf8_jq_document`](crate::jq::utf8_document::substitute_invalid_utf8_jq_document),
/// which segments the document and calls this function once per JSON
/// string (#1743). `--raw-input --slurp` is the one caller that stays
/// whole-buffer, because real jq is whole-buffer there too: the entire
/// input is a single string, so the buffer's own end genuinely *is* that
/// string's end. See `docs/compliance/jq/limitations.md` for the
/// live-verified detail.
///
/// A single left-to-right scan, not a loop over [`validate_utf8`]: that
/// function's AVX2 path has no early exit (it scans every 32-byte block of
/// its input before checking for an error at all, since its job is a
/// whole-buffer accept/reject decision, not locating the first error
/// cheaply), so calling it repeatedly on a shrinking suffix to find one
/// error at a time is O(n) *per call*, and a dense run of single-byte
/// errors (e.g. a binary file with no real UTF-8 in it) drove that into
/// O(n^2) overall in an earlier version of this function -- found by
/// review before it shipped. This mirrors [`validate_utf8_scalar`]'s own
/// dispatch instead (lead-byte-driven, ASCII-skipped via the same
/// broadword `skip_ascii` helper), continuing past an error rather than
/// returning on the first one.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::substitute_invalid_utf8_jq_style;
///
/// // Overlong 3-byte sequence: one U+FFFD for the whole sequence, not one per byte.
/// assert_eq!(substitute_invalid_utf8_jq_style(&[0xE0, 0x80, 0x80]), "\u{FFFD}");
///
/// // Already-agreeing case (invalid lead byte): unchanged from WHATWG's rule.
/// assert_eq!(substitute_invalid_utf8_jq_style(&[0xFF, 0xFE]), "\u{FFFD}\u{FFFD}");
/// ```
pub fn substitute_invalid_utf8_jq_style(input: &[u8]) -> String {
    let len = input.len();
    let mut out = String::with_capacity(len);
    let mut pos = 0;
    // `input[run_start..pos]` is confirmed valid UTF-8 not yet flushed to
    // `out`.
    let mut run_start = 0;

    while pos < len {
        let byte = input[pos];

        if byte < 0x80 {
            pos = skip_ascii(input, pos + 1);
            continue;
        }

        match invalid_subpart_end(input, pos, len) {
            Some(resume_at) => {
                out.push_str(
                    core::str::from_utf8(&input[run_start..pos]).expect("already validated"),
                );
                out.push('\u{FFFD}');
                pos = resume_at;
                run_start = pos;
            }
            // A fully valid `sequence_length(byte)`-byte sequence,
            // confirmed by `invalid_subpart_end` -- stays part of the
            // current run.
            None => pos += sequence_length(byte),
        }
    }

    out.push_str(core::str::from_utf8(&input[run_start..pos]).expect("already validated"));
    out
}

/// The end of the invalid maximal subpart starting at `input[pos]` (which
/// is not ASCII -- callers skip that case via [`skip_ascii`] before
/// reaching here), or `None` if `input[pos]` instead leads a fully valid
/// [`sequence_length`]-byte sequence. Each of the four `Some` cases below
/// is a distinct, mutually exclusive reason a byte can't be part of one --
/// see [`substitute_invalid_utf8_jq_style`]'s own doc comment for which
/// jq/WHATWG rule each one implements.
#[inline]
fn invalid_subpart_end(input: &[u8], pos: usize, len: usize) -> Option<usize> {
    let byte = input[pos];
    let seq_len = sequence_length(byte);

    if seq_len == 0 {
        // A stray continuation byte (0x80-0xBF) or a lead RFC 3629 never
        // lists as valid at any length (0xF8-0xFF): one byte, no sequence
        // in progress.
        return Some(pos + 1);
    }

    if is_never_valid_lead_byte(byte, seq_len) {
        // 0xC0/0xC1 or 0xF5-0xF7: can never decode in range regardless of
        // what follows, so -- like any other bad lead -- this is a single
        // byte, checked before even counting continuation bytes or
        // comparing against remaining length. Must run before the
        // truncation check below: review found live against jq that
        // `\xf5\x80` at end-of-input is 2 one-byte errors, not one
        // "2-of-4-bytes-present" truncated subpart -- jq never tries to
        // accumulate continuations for a lead that could never succeed.
        return Some(pos + 1);
    }

    // #1617/#1717, unified: real jq's rule is not "was every available byte
    // a valid continuation" -- it's simpler than that. jq first checks
    // whether `seq_len` bytes are even physically present from `pos`
    // onward at all. If not, it can never complete this sequence no matter
    // what those bytes contain, so it collapses the *entire* remaining
    // tail into one U+FFFD unconditionally -- even when one of the
    // available bytes would, on its own, have been a perfectly good ASCII
    // character to keep and rescan. Only once `seq_len` bytes are actually
    // present does jq bother validating each continuation byte
    // individually and doing WHATWG-style rescan-at-the-bad-byte.
    //
    // This one condition subsumes what used to be two separate branches
    // here (a "ran out of bytes, every byte present was a good
    // continuation" case, and a narrower "shortfall >= 2 AND at the
    // absolute last byte" special case for #1717): both turned out to be
    // instances of the same simpler rule. The previous, narrower #1717
    // condition undercounted -- live-verified against jq 1.7.1, which
    // also collapses `[0xF0, b'A', b'B']` (a 4-byte lead, one invalid
    // continuation, *one* byte of headroom before end-of-input) to a
    // single U+FFFD, which `rescan_pos == len - 1` alone does not catch
    // (`rescan_pos` there is `len - 2`, not `len - 1`).
    if len - pos < seq_len {
        return Some(len);
    }

    // `seq_len` bytes are confirmed present; find the first one (if any)
    // that isn't a valid continuation byte. Safe to index unconditionally
    // for `i` in `1..seq_len`: the check above guarantees `pos + seq_len
    // - 1 <= len - 1`.
    for i in 1..seq_len {
        if !is_continuation_byte(input[pos + i]) {
            // WHATWG-style: the subpart is the lead plus whatever
            // continuations came before this one; the offending byte
            // itself is rescanned next iteration, not skipped.
            return Some(pos + i);
        }
    }

    // Every continuation byte already confirmed present and well-formed
    // above -- the only way `decode_code_point` can still return `None`
    // here is a bounds violation on a structurally valid lead
    // (`0xE0`-`0xEF`/`0xF0`-`0xF4`; a never-valid lead already returned
    // above), which collapses the whole sequence into one U+FFFD.
    // Sharing the decode (rather than re-deriving the same per-length
    // bit-packing arithmetic inline) keeps it in exactly two places in
    // the file (`validate_utf8_scalar`'s own unrolled arms, which need
    // their per-byte error offsets and so can't easily call out to
    // this), not three -- see `code_point_bounds_violation`'s own doc
    // comment and #1423 for a case where a third independent copy of
    // this exact arithmetic drifted silently.
    if decode_code_point(&input[pos..]).is_some() {
        None
    } else {
        Some(pos + seq_len)
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

        /// #1423: an overlong encoding decodes "successfully" to its would-be
        /// code point unless explicitly rejected -- same shapes as
        /// `overlong_encoding`'s `validate_utf8` tests above, checked here
        /// against `decode_code_point` specifically.
        #[test]
        fn rejects_overlong_encodings() {
            // NUL (U+0000) as an overlong 2-byte sequence: C0 80.
            assert_eq!(decode_code_point(&[0xC0, 0x80]), None);
            // DEL (U+007F) as an overlong 2-byte sequence: C1 BF -- the
            // issue's own repro, the highest code point still representable
            // (and thus still tempting to "successfully" decode) in 2 bytes.
            assert_eq!(decode_code_point(&[0xC1, 0xBF]), None);
            // NUL as an overlong 3-byte sequence: E0 80 80.
            assert_eq!(decode_code_point(&[0xE0, 0x80, 0x80]), None);
            // U+07FF (highest 2-byte-representable code point) spelled as an
            // overlong 3-byte sequence: E0 9F BF.
            assert_eq!(decode_code_point(&[0xE0, 0x9F, 0xBF]), None);
            // NUL as an overlong 4-byte sequence: F0 80 80 80.
            assert_eq!(decode_code_point(&[0xF0, 0x80, 0x80, 0x80]), None);
            // U+FFFF (highest 3-byte-representable code point) spelled as an
            // overlong 4-byte sequence: F0 8F BF BF.
            assert_eq!(decode_code_point(&[0xF0, 0x8F, 0xBF, 0xBF]), None);
        }

        /// #1423: a structurally well-formed 3-byte sequence that decodes
        /// into the surrogate range must still be rejected -- surrogates are
        /// reserved for UTF-16 and are never valid standalone UTF-8, even
        /// though `char::from_u32` alone would already catch this one step
        /// later (defense in depth, not a redundant check: this function's
        /// own contract is "reject invalid UTF-8," and a surrogate sequence
        /// is invalid UTF-8 regardless of what the caller does with the
        /// numeric result).
        #[test]
        fn rejects_surrogate_code_points() {
            // U+D800: first high surrogate, ED A0 80 -- the issue's own repro.
            assert_eq!(decode_code_point(&[0xED, 0xA0, 0x80]), None);
            // U+DBFF: last high surrogate.
            assert_eq!(decode_code_point(&[0xED, 0xAF, 0xBF]), None);
            // U+DC00: first low surrogate.
            assert_eq!(decode_code_point(&[0xED, 0xB0, 0x80]), None);
            // U+DFFF: last low surrogate.
            assert_eq!(decode_code_point(&[0xED, 0xBF, 0xBF]), None);
            // U+D7FF and U+E000 (just outside the surrogate range on each
            // side) must still decode successfully -- the check is a closed
            // range, not an off-by-one over- or under-reach.
            assert_eq!(decode_code_point(&[0xED, 0x9F, 0xBF]), Some((0xD7FF, 3)));
            assert_eq!(decode_code_point(&[0xEE, 0x80, 0x80]), Some((0xE000, 3)));
        }

        /// #1423: a 4-byte sequence can structurally encode past Unicode's
        /// own maximum code point (U+10FFFF); this crate's `validate_utf8`
        /// already rejects that shape (`Utf8ErrorKind::OutOfRangeCodepoint`)
        /// and `decode_code_point` should agree rather than silently
        /// decoding a value no real Unicode string could ever contain.
        #[test]
        fn rejects_out_of_range_code_point() {
            // U+10FFFF (the real maximum) still decodes.
            assert_eq!(
                decode_code_point(&[0xF4, 0x8F, 0xBF, 0xBF]),
                Some((0x10FFFF, 4))
            );
            // U+110000 (one past the maximum) must not.
            assert_eq!(decode_code_point(&[0xF4, 0x90, 0x80, 0x80]), None);
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

    use super::{decode_code_point, validate_utf8, validate_utf8_scalar, Utf8Error, Utf8ErrorKind};
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

    /// Block-edge offsets shared by both fixture builders.
    ///
    /// `0..=7` covers every residue mod 8, which is what the broadword scan
    /// needs: its ASCII skip can leave `pos` anywhere inside a word, and a bug
    /// that only fires at, say, a 3-byte skip would otherwise go unseen. (The
    /// original offsets were chosen for the 32-byte AVX2 blocks and hit only
    /// residues 0, 1, 6 and 7.) `8..=17` adds a second word plus a guard for a
    /// possible future 16-byte block, and the larger values keep the AVX2
    /// 32/64-byte edges covered.
    const BOUNDARY_OFFSETS: [usize; 26] = [
        0, 1, 2, 3, 4, 5, 6, 7, // every residue mod 8 (broadword word edges)
        8, 9, 10, 11, 12, 13, 14, 15, 16, 17, // second word / 16-byte guard
        29, 30, 31, 32, 33, 62, 63, 64, // AVX2 32/64-byte block edges
    ];

    /// Invalid fixtures: one per error kind, placed at offsets that straddle the
    /// 8/16-byte broadword word edges and the 32/64-byte SIMD block edges, and
    /// land at end-of-input.
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
        let mut out = Vec::new();
        for seq in bad_seqs {
            for &off in &BOUNDARY_OFFSETS {
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
        let mut out = Vec::new();
        for seq in good_seqs {
            for &off in &BOUNDARY_OFFSETS {
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

    /// #1423 review: `decode_code_point`'s Some/None verdict on a byte
    /// buffer must agree with whether `validate_utf8_scalar` accepts that
    /// *same* buffer, provided the buffer is exactly one sequence long
    /// (its length equals `sequence_length` of its own first byte) — the
    /// only case where the two functions are actually answering the same
    /// question. A buffer longer than one sequence isn't a fair
    /// comparison: `decode_code_point` only ever looks at the first
    /// sequence and ignores any bytes after it, while
    /// `validate_utf8_scalar` scans the *whole* buffer as a sequence of
    /// characters, so extra trailing bytes it happens to reject (or a
    /// short lead byte's true `sequence_length` not matching however long
    /// the buffer was built to be) would make the two functions disagree
    /// for reasons that have nothing to do with either one's own
    /// correctness.
    ///
    /// Covers, exhaustively: every one of the 6 invalid-lead-byte values
    /// (0x80-0xBF continuation-as-lead, 0xF8-0xFF) as a lone byte; every
    /// valid multi-byte lead byte (0xC0-0xDF/0xE0-0xEF/0xF0-0xF7) paired
    /// with continuation bytes at their two boundary extremes (0x80/0xBF)
    /// in every position -- the overlong/surrogate/out-of-range edge cases
    /// this issue is about all live at these extremes, e.g. an overlong
    /// 2-byte sequence is exactly `lead in 0xC0..=0xC1` with *any*
    /// continuation byte. Plus a random-fuzzing pass with non-boundary
    /// continuation bytes (including invalid ones) as a second,
    /// independent check.
    #[test]
    fn decode_code_point_matches_scalar_for_single_sequences() {
        fn check(seq: &[u8]) {
            assert_eq!(
                decode_code_point(seq).is_some(),
                validate_utf8_scalar(seq).is_ok(),
                "decode_code_point/validate_utf8_scalar disagreed on {seq:02x?}"
            );
        }

        // Invalid lead bytes: sequence_length is 0, so a lone byte is
        // already "one full (non-)sequence" for both functions.
        for lead in (0x80u16..=0xBF).chain(0xF8..=0xFF) {
            check(&[lead as u8]);
        }

        let boundary_continuations = [0x80u8, 0xBF];
        for (lead_range, len) in [(0xC0u16..=0xDF, 2), (0xE0..=0xEF, 3), (0xF0..=0xF7, 4)] {
            for lead in lead_range {
                let lead = lead as u8;
                for c1 in boundary_continuations {
                    let combos: &[&[u8]] = match len {
                        2 => &[&[lead, c1]],
                        3 => &[&[lead, c1, 0x80], &[lead, c1, 0xBF]],
                        4 => &[
                            &[lead, c1, 0x80, 0x80],
                            &[lead, c1, 0x80, 0xBF],
                            &[lead, c1, 0xBF, 0x80],
                            &[lead, c1, 0xBF, 0xBF],
                        ],
                        _ => unreachable!(),
                    };
                    for seq in combos {
                        check(seq);
                    }
                }
            }
        }

        // Random-fuzzing pass: pick a random lead byte first, then fill
        // exactly its own natural sequence_length with random continuation
        // bytes (which may themselves be invalid) -- so every generated
        // buffer is still a fair one-sequence comparison.
        let mut rng = Rng(0xdec0_de5e_c0de_5eed);
        for _ in 0..20_000 {
            let lead = rng.below(256) as u8;
            let len = super::sequence_length(lead).max(1);
            let mut bytes = alloc::vec![lead];
            bytes.extend((1..len).map(|_| (rng.next_u32() & 0xFF) as u8));
            check(&bytes);
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

    /// The broadword kernel agrees with std over a large corpus.
    ///
    /// The counterpart of [`avx2_kernel_matches_std`], but unguarded by any
    /// `cfg` — the broadword scan is portable, so this gates it on every
    /// architecture. It drives the raw `bool` kernels rather than the public
    /// wrappers so that a false *reject* is caught too: the wrappers mask those
    /// by falling back to the scalar validator, which would then return `Ok`
    /// and hide the disagreement.
    #[test]
    fn broadword_kernel_matches_std() {
        let mut rng = Rng(0xb70a_d000_d1a5_0007);
        let check = |bytes: &[u8]| {
            let expected = core::str::from_utf8(bytes).is_ok();
            assert_eq!(
                super::broadword::accepts(bytes),
                expected,
                "broadword kernel disagreed with std on {bytes:02x?}"
            );
        };

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

    /// Every input of length 0, 1 or 2 — all 65,793 of them — validates the same
    /// as std.
    ///
    /// Exhaustive rather than random, and short enough that the main loop never
    /// runs, so it pins the sub-word tail path on its own.
    #[test]
    fn broadword_exhaustive_short_inputs() {
        let check = |bytes: &[u8]| {
            let expected = core::str::from_utf8(bytes).is_ok();
            assert_eq!(super::broadword::accepts(bytes), expected, "{bytes:02x?}");
        };

        check(&[]);
        for a in 0..=255u8 {
            check(&[a]);
            for b in 0..=255u8 {
                check(&[a, b]);
            }
        }
    }

    /// Sequences cut off at end-of-input are rejected at every distance from a
    /// word boundary.
    ///
    /// Truncation is the one condition the main loop cannot see coming — it is
    /// detected by running out of bytes mid-sequence, or by the tail loop ending
    /// in a non-ground state — so it is checked at every `len % 8`.
    #[test]
    fn broadword_truncation_at_word_boundaries() {
        let truncated: &[&[u8]] = &[
            &[0xC2],             // 2-byte lead, no continuation
            &[0xE0, 0xA0],       // 3-byte, one continuation short
            &[0xF0, 0x90, 0x80], // 4-byte, one continuation short
            &[0xF1],             // 4-byte lead, three continuations short
        ];
        for seq in truncated {
            for pad in 0..=17usize {
                let mut buf: Vec<u8> = core::iter::repeat(b'a').take(pad).collect();
                buf.extend_from_slice(seq);
                assert!(
                    core::str::from_utf8(&buf).is_err(),
                    "fixture should be invalid: {buf:02x?}"
                );
                assert!(!super::broadword::accepts(&buf), "{buf:02x?}");
            }
        }
    }

    /// The broadword wrapper returns byte-identical `Utf8Error`s to the scalar
    /// validator — offset, line, column and kind.
    ///
    /// True by construction today, since it delegates to
    /// [`validate_utf8_scalar`] on rejection. The test exists so that any later
    /// attempt to build errors inside the kernel fails loudly rather than
    /// silently shifting a reported line or column.
    #[test]
    fn broadword_matches_scalar_including_error() {
        let mut rng = Rng(0xb70a_d000_e770_0011);
        for _ in 0..20_000 {
            let len = rng.below(96) as usize;
            let bytes = random_bytes(&mut rng, len);
            let expected = validate_utf8_scalar(&bytes);
            assert_eq!(
                super::validate_utf8_broadword(&bytes),
                expected,
                "broadword differed from scalar on {bytes:02x?}"
            );
        }
    }
}

/// Regression tests for #1617: `substitute_invalid_utf8_jq_style` must match
/// jq 1.7.1's own maximal-subpart rule, not `String::from_utf8_lossy`'s
/// WHATWG rule. Every expected value below was live-verified against the
/// pinned `/usr/bin/jq` 1.7.1 binary (see the issue for the full table).
#[cfg(test)]
mod substitute_invalid_utf8_jq_style_tests {
    use super::*;

    /// Regression test for a critical bug review found: the truncation
    /// check (`pos + seq_len > len`) fired on remaining *byte count*
    /// alone, before ever checking whether the bytes that *are* present
    /// validate as continuation bytes. When a genuinely-invalid
    /// continuation byte happened to fall near the end of `input` --
    /// short of `seq_len` total remaining bytes, but not because input
    /// ran out -- the old code misclassified it as truncation and
    /// swallowed the *whole* remaining tail into one U+FFFD by the wrong
    /// mechanism (a truncation collapse, not a rescanned-byte decision).
    ///
    /// The four assertions below all happen to land in #1717's own
    /// drop-the-whole-tail territory (`len - pos < seq_len`) -- confirmed
    /// live against jq 1.7.1 -- so their *answer* now agrees with a
    /// one-U+FFFD collapse too. That is not a coincidence this test stops
    /// distinguishing: it still exercises the original bug's actual
    /// failure mode, because the old truncation-misclassification bug did
    /// not require end-of-input at all, and would have collapsed these
    /// same vectors even with trailing content following (see
    /// `keeps_and_rescans_once_seq_len_bytes_are_present`, which pins
    /// exactly that: trailing content is kept, not swallowed, once enough
    /// bytes are present for the full sequence).
    #[test]
    fn invalid_continuation_boundary_detection_matches_jq_near_eof() {
        // #1717: whenever fewer than `seq_len` bytes remain from the lead
        // onward, jq (and now this function) collapses the whole tail
        // rather than keeping and rescanning whatever's left -- see the
        // dedicated `drops_the_whole_tail_when_fewer_than_seq_len_bytes_remain_from_the_lead`
        // test below for the isolated case-by-case breakdown.
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xE1, b'A']), "\u{FFFD}");
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xF0, b'A']), "\u{FFFD}");
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, 0x90, b'A']),
            "\u{FFFD}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[b'X', 0xE1, b'A']),
            "X\u{FFFD}"
        );
        // Genuine truncation (every available byte IS a valid
        // continuation, there just aren't enough of them) must still
        // collapse to one U+FFFD, not regress to per-byte. Same
        // `len - pos < seq_len` condition #1717 fixed above -- there's no
        // separate "ran out of bytes" branch left to distinguish it from,
        // just a different reason the condition happens to hold.
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xE1, 0x80]), "\u{FFFD}");
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, 0x90, 0x80]),
            "\u{FFFD}"
        );
    }

    /// Regression test for a second bug review found on top of the first:
    /// a never-valid lead byte (`0xC0`/`0xC1`/`0xF5`-`0xF7`) with fewer
    /// than `seq_len` bytes remaining was *also* misrouted into the
    /// truncation branch, collapsing the whole tail into one U+FFFD --
    /// but jq treats these leads as a one-byte error unconditionally, per
    /// byte, regardless of how much input remains (it never attempts to
    /// accumulate continuations for a lead that could never succeed).
    /// Live-verified against the pinned jq 1.7.1 binary: `\xf5\x80` at
    /// end-of-input is 2 separate U+FFFD, not 1.
    #[test]
    fn never_valid_lead_stays_per_byte_even_when_truncated() {
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF5, 0x80]),
            "\u{FFFD}\u{FFFD}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF5, 0x80, 0x80]),
            "\u{FFFD}\u{FFFD}\u{FFFD}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF6, 0x80, 0x80]),
            "\u{FFFD}\u{FFFD}\u{FFFD}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF5, 0xF6, 0xF7]),
            "\u{FFFD}\u{FFFD}\u{FFFD}"
        );
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xC0]), "\u{FFFD}");
    }

    #[test]
    fn valid_input_is_unchanged() {
        assert_eq!(substitute_invalid_utf8_jq_style(b"hello"), "hello");
        assert_eq!(
            substitute_invalid_utf8_jq_style("日本語".as_bytes()),
            "日本語"
        );
        assert_eq!(substitute_invalid_utf8_jq_style(b""), "");
    }

    #[test]
    fn invalid_lead_byte_is_one_fffd_per_byte() {
        // Already agreed with jq before #1617; must stay unchanged.
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xFF]), "\u{FFFD}");
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xFF, 0xFE]),
            "\u{FFFD}\u{FFFD}"
        );
        assert_eq!(substitute_invalid_utf8_jq_style(&[0x80]), "\u{FFFD}");
    }

    #[test]
    fn truncated_sequences_collapse_to_one_fffd() {
        // Already agreed with jq before #1617; must stay unchanged.
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xC2]), "\u{FFFD}");
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xE1, 0x80]), "\u{FFFD}");
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, 0x90, 0x80]),
            "\u{FFFD}"
        );
    }

    #[test]
    fn two_byte_overlong_c0_c1_is_one_fffd_per_byte() {
        // 0xC0/0xC1 can never produce a valid (non-overlong) 2-byte
        // sequence at any continuation value -- jq treats the lead byte
        // itself as immediately invalid, same as any other never-valid
        // lead, not as "collapse the whole sequence" (unlike 3-/4-byte
        // overlong on an otherwise-valid lead). Already agreed with jq
        // before #1617; must stay unchanged.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xC0, 0xAF]),
            "\u{FFFD}\u{FFFD}"
        );
    }

    #[test]
    fn three_byte_overlong_collapses_to_one_fffd() {
        // The #1617 fix: jq collapses the whole structurally-valid 3-byte
        // sequence into one U+FFFD; WHATWG/`from_utf8_lossy` would give 3.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xE0, 0x80, 0x80]),
            "\u{FFFD}"
        );
        // Second byte below the narrowed A0 threshold, still a
        // structurally valid lead -- same collapse.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xE0, 0x9F, 0xBF]),
            "\u{FFFD}"
        );
    }

    #[test]
    fn four_byte_overlong_collapses_to_one_fffd() {
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, 0x8F, 0xBF, 0xBF]),
            "\u{FFFD}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, 0x80, 0x80, 0x80]),
            "\u{FFFD}"
        );
    }

    #[test]
    fn surrogate_codepoints_collapse_to_one_fffd() {
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xED, 0xA0, 0x80]),
            "\u{FFFD}"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xED, 0xBF, 0xBF]),
            "\u{FFFD}"
        );
    }

    #[test]
    fn surrogate_pair_cesu8_gives_two_fffd_not_six() {
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xED, 0xA0, 0x80, 0xED, 0xB0, 0x80]),
            "\u{FFFD}\u{FFFD}"
        );
    }

    #[test]
    fn out_of_range_on_valid_lead_collapses_to_one_fffd() {
        // F4 90 80 80 decodes to U+110000, one past U+10FFFF -- F4 itself
        // is a structurally valid 4-byte lead (F0-F4 per RFC 3629).
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF4, 0x90, 0x80, 0x80]),
            "\u{FFFD}"
        );
    }

    #[test]
    fn out_of_range_on_never_valid_lead_is_one_fffd_per_byte() {
        // F5-F7 can NEVER decode to <= U+10FFFF regardless of
        // continuation bytes (minimum for F5 is already 0x140000) -- RFC
        // 3629 excludes them from the valid-lead-byte set entirely, so
        // jq treats them like C0/C1: one U+FFFD per byte, not a
        // whole-sequence collapse. This is the one case #1617's own issue
        // didn't test and would have been wrongly collapsed by a naive
        // "any OutOfRangeCodepoint on 0xF0-0xF7" rule.
        for lead in [0xF5u8, 0xF6, 0xF7] {
            let input = [lead, 0x80, 0x80, 0x80];
            assert_eq!(
                substitute_invalid_utf8_jq_style(&input),
                "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}",
                "lead byte {lead:#04X}"
            );
        }
    }

    #[test]
    fn invalid_lead_f8_ff_is_one_fffd_per_byte() {
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF8, 0x80, 0x80, 0x80, 0x80]),
            "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"
        );
    }

    #[test]
    fn invalid_continuation_byte_rescans_the_offending_byte() {
        // Already agreed with jq before #1617; must stay unchanged. The
        // offending byte is rescanned (not skipped), so it reappears as
        // its own literal character when it's plain ASCII.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xE1, 0x41, b'b', b'c']),
            "\u{FFFD}Abc"
        );
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xE1, 0x80, 0x41, b'b', b'c']),
            "\u{FFFD}Abc"
        );
    }

    /// #1717: real jq's rule is `len - pos < seq_len` -- if `seq_len`
    /// bytes aren't all physically present from the lead's own position
    /// onward, jq collapses the *entire* remaining tail into one U+FFFD,
    /// regardless of *why* they're short (buffer truly ends, or one byte
    /// present fails the continuation check). An earlier, narrower version
    /// of this fix conditioned the drop on "the offending byte is
    /// `input`'s last byte", which undercounts: `[0xF0, b'A', b'B']`
    /// (4-byte lead, invalid continuation, *one* byte of headroom before
    /// end-of-input -- so the offending byte is *not* the last byte) still
    /// collapses in real jq, live-verified, and is the case that earlier
    /// version got wrong. The shortfall=2-via-3-byte-lead case
    /// (`[0xE1, b'A']`), shortfall=3-via-4-byte-lead zero-headroom case
    /// (`[0xF0, b'A']`), and 4-byte-lead-good=1-zero-headroom case
    /// (`[0xF0, 0x80, b'A']`, indistinguishable from `[0xF0, 0x90, b'A']`
    /// there since `is_continuation_byte` treats the whole 0x80-0xBF range
    /// alike) are already pinned by
    /// `invalid_continuation_boundary_detection_matches_jq_near_eof` above;
    /// this adds the one shape that test doesn't cover -- the previously-
    /// missed, one-byte-of-headroom case.
    #[test]
    fn drops_the_whole_tail_when_fewer_than_seq_len_bytes_remain_from_the_lead() {
        // 4-byte lead, good=0, *one byte of headroom* (len - pos == 3 <
        // seq_len == 4) -- the shape the narrower pre-fix condition missed.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, b'A', b'B']),
            "\u{FFFD}"
        );
        // The dropped bytes need not be plain ASCII -- they're consumed
        // wholesale, not re-decoded, so they get no U+FFFD of their own.
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xE1, 0xFF]), "\u{FFFD}");
    }

    #[test]
    fn keeps_and_rescans_once_seq_len_bytes_are_present() {
        // 2-byte lead, good=0 (len - pos == 2 == seq_len): kept.
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xC2, b'A']), "\u{FFFD}A");
        // 3-byte lead, good=1 (len - pos == 3 == seq_len): kept.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xE1, 0x80, b'A']),
            "\u{FFFD}A"
        );
        // 4-byte lead, good=2 (len - pos == 4 == seq_len): kept.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, 0x90, 0x80, b'A']),
            "\u{FFFD}A"
        );
        // 4-byte lead, good=1, *one* byte of headroom (len - pos == 4 ==
        // seq_len) -- the sibling of the previously-missed drop case above,
        // one byte further along the same axis: kept, not dropped.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, 0x80, b'A', b'B']),
            "\u{FFFD}AB"
        );
        // 4-byte lead, good=0, two bytes of headroom (len - pos == 4 ==
        // seq_len): kept, exactly as
        // `invalid_continuation_byte_rescans_the_offending_byte` above
        // already covers for the 3-byte-lead sibling.
        assert_eq!(
            substitute_invalid_utf8_jq_style(&[0xF0, b'A', b'B', b'C']),
            "\u{FFFD}ABC"
        );
    }

    #[test]
    fn never_valid_lead_byte_at_end_is_unaffected_by_the_drop_quirk() {
        // #1717 is specific to InvalidContinuationByte on a structurally-
        // valid lead; is_never_valid_lead_byte (0xC0/0xC1/0xF5-0xF7) is
        // handled earlier in invalid_subpart_end and never reaches the
        // `len - pos < seq_len` check at all, so this stays exactly as
        // before.
        assert_eq!(substitute_invalid_utf8_jq_style(&[0xC0, b'A']), "\u{FFFD}A");
    }

    #[test]
    fn valid_prefix_and_suffix_around_a_bad_sequence_are_preserved() {
        let mut input = b"before ".to_vec();
        input.extend_from_slice(&[0xE0, 0x80, 0x80]);
        input.extend_from_slice(b" after");
        assert_eq!(
            substitute_invalid_utf8_jq_style(&input),
            "before \u{FFFD} after"
        );
    }

    #[test]
    fn multiple_bad_sequences_each_get_their_own_substitution() {
        let mut input = vec![0xE0, 0x80, 0x80];
        input.push(b'-');
        input.extend_from_slice(&[0xED, 0xA0, 0x80]);
        assert_eq!(
            substitute_invalid_utf8_jq_style(&input),
            "\u{FFFD}-\u{FFFD}"
        );
    }

    /// `substitute_invalid_utf8_jq_style` must always produce valid UTF-8
    /// (a `String`, not just "doesn't panic") on arbitrary bytes, never
    /// panicking on the internal `.expect("already validated")` calls.
    #[test]
    fn never_panics_on_random_bytes() {
        // Tiny deterministic xorshift64 RNG, same shape as
        // `validate_utf8_differential_tests`'s own (not shared across
        // modules -- that one is private to it).
        struct Rng(u64);
        impl Rng {
            fn next_u32(&mut self) -> u32 {
                let mut x = self.0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.0 = x;
                (x >> 32) as u32
            }
            fn below(&mut self, n: u32) -> u32 {
                self.next_u32() % n
            }
        }

        let mut rng = Rng(0x1617_1617_1617_1617);
        for _ in 0..5_000 {
            let len = rng.below(64) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| (rng.next_u32() & 0xFF) as u8).collect();
            let _ = substitute_invalid_utf8_jq_style(&bytes);
        }
    }
}
