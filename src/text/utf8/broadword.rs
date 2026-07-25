//! Broadword (SWAR) UTF-8 accept scan.
//!
//! A portable validation kernel that clears ASCII in 32- and 8-byte strides
//! using ordinary 64-bit integer arithmetic — no SIMD intrinsics, no runtime
//! feature detection, and available on every target including `no_std` builds.
//! This is the default engine everywhere the AVX2 path in [`super::simd_x86`]
//! is unavailable: aarch64, wasm, riscv, pre-AVX2 x86_64, and any build without
//! the `std` feature.
//!
//! ## Why this exists
//!
//! [`super::validate_utf8_scalar`] costs one loop iteration per *byte* of
//! ASCII, so it is slower on ASCII-heavy input than on multi-byte input: a
//! 4-byte emoji costs one iteration where four ASCII bytes cost four. Measured
//! on an M4 Pro over the realistic corpus, that put pure ASCII at 1.81 GiB/s
//! and log files at 1.38 GiB/s. Clearing 32 ASCII bytes with a single high-bit
//! test attacks exactly that, taking those to 58.0 and 10.6 GiB/s.
//!
//! ## Diagnostics
//!
//! [`accepts`] is a pure accept scan returning `bool`. It carries no line or
//! column state and never classifies an error, because a validator that must
//! also produce [`Utf8ErrorKind`](super::Utf8ErrorKind) cannot keep its hot
//! loop this tight. On rejection [`validate_utf8_broadword`] re-runs
//! [`super::validate_utf8_scalar`], which is the sole producer of
//! [`Utf8Error`] — so diagnostics are byte-identical no matter which engine
//! ran. This mirrors the AVX2 path exactly. The cost is that invalid input is
//! scanned twice; valid input, the overwhelmingly common case, is scanned once.
//!
//! ## Why not a DFA
//!
//! Issue #134 specified a table-driven [Höhrmann] DFA for the multi-byte step.
//! That was implemented, benchmarked head-to-head against the whole-sequence
//! approach used here, and **rejected**: across the eleven realistic patterns on
//! an M4 Pro it averaged 1.73x the scalar validator against this kernel's
//! 2.18x, losing on nine of the eleven.
//!
//! The cause is dependency structure. A DFA carries `state -> step -> state`
//! around its loop, so one byte retires per two-to-three cycle chain no matter
//! how compact the table. Validating a whole sequence at once issues its range
//! comparisons independently, retiring three or four bytes per iteration — the
//! same shape that already made the scalar validator faster on emoji than on
//! ASCII. See `docs/benchmarks/utf8-validate.md` for the full comparison.
//!
//! [Höhrmann]: https://bjoern.hoehrmann.de/utf-8/decoder/dfa/

use super::{validate_utf8_scalar, Utf8Error};

/// High bit of each byte in a 64-bit word.
///
/// `word & HI == 0` iff all eight bytes are ASCII.
const HI: u64 = 0x8080_8080_8080_8080;

/// Bytes consumed per broadword iteration.
const WORD: usize = 8;

/// Bytes tested per iteration of the wide ASCII loop: four `u64` words.
///
/// Four words are OR-ed together before a single high-bit test, which gives
/// LLVM a shape it can lower to a handful of NEON/SSE ops. Eight bytes at a
/// time is enough to beat the byte-at-a-time scalar validator many times over,
/// but not enough to match `core::str::from_utf8` on long ASCII runs — std
/// tests sixteen bytes per iteration, and on a 1MB pure-ASCII buffer that gap
/// measured as 14.9 GiB/s against 52.0 GiB/s.
const BLOCK: usize = 4 * WORD;

/// Load eight bytes at `pos` as a native-endian word, or `None` if fewer than
/// eight remain.
///
/// The `None` case is the normal loop exit, not an error path — callers drive
/// the main loop with `while let Some(word) = load_word(..)` so the bounds
/// check and the loop condition are the same test.
#[inline(always)]
fn load_word(input: &[u8], pos: usize) -> Option<u64> {
    let chunk = input.get(pos..pos.checked_add(WORD)?)?;
    let mut bytes = [0u8; WORD];
    bytes.copy_from_slice(chunk);
    Some(u64::from_ne_bytes(bytes))
}

/// OR together the four words at `pos`, or `None` if fewer than [`BLOCK`]
/// bytes remain.
///
/// Only the high bits of the result are meaningful: `load_block(..) & HI == 0`
/// iff all 32 bytes are ASCII. Callers fall back to [`load_word`] once this
/// returns `None` or reports a non-ASCII byte somewhere in the block.
#[inline(always)]
fn load_block(input: &[u8], pos: usize) -> Option<u64> {
    let chunk = input.get(pos..pos.checked_add(BLOCK)?)?;
    let mut acc = 0u64;
    for word in chunk.chunks_exact(WORD) {
        let mut bytes = [0u8; WORD];
        bytes.copy_from_slice(word);
        acc |= u64::from_ne_bytes(bytes);
    }
    Some(acc)
}

/// Index in memory order (`0..8`) of the first non-ASCII byte, given
/// `hi = word & HI` from a [`load_word`] result.
///
/// Requires `hi != 0`; the result is then always `< 8`, which is what lets
/// callers index `input[pos]` afterwards without a further bounds test.
#[inline(always)]
fn first_high_byte(hi: u64) -> usize {
    if cfg!(target_endian = "little") {
        first_high_byte_le(hi)
    } else {
        first_high_byte_be(hi)
    }
}

/// Little-endian half of [`first_high_byte`].
///
/// Memory byte `k` occupies bits `8k..8k+8`, so its high bit is bit `8k + 7`
/// and `(8k + 7) >> 3 == k` for every `k` in `0..8`.
#[inline(always)]
fn first_high_byte_le(hi: u64) -> usize {
    (hi.trailing_zeros() >> 3) as usize
}

/// Big-endian half of [`first_high_byte`].
///
/// Memory byte `k` occupies the opposite end, so its high bit is bit
/// `63 - 8k` and `leading_zeros() >> 3 == k`.
#[inline(always)]
fn first_high_byte_be(hi: u64) -> usize {
    (hi.leading_zeros() >> 3) as usize
}

/// Is `byte` a UTF-8 continuation byte (`0x80..=0xBF`)?
#[inline(always)]
fn is_continuation(byte: u8) -> bool {
    (byte & 0xC0) == 0x80
}

/// Validate one whole sequence starting at `pos`, returning its length.
///
/// Returns `None` if the sequence is malformed or runs off the end of `input`.
/// Encodes Unicode Table 3-7 directly: the four range-restricted leads (`E0`,
/// `ED`, `F0`, `F4`) constrain their second byte, which is what rules out
/// overlong encodings, surrogates, and code points above U+10FFFF.
#[inline(always)]
fn validate_sequence(input: &[u8], pos: usize) -> Option<usize> {
    let len = input.len();
    let b0 = *input.get(pos)?;

    match b0 {
        0x00..=0x7F => Some(1),
        0xC2..=0xDF => {
            if pos + 1 >= len || !is_continuation(input[pos + 1]) {
                return None;
            }
            Some(2)
        }
        0xE0..=0xEF => {
            if pos + 2 >= len {
                return None;
            }
            let b1 = input[pos + 1];
            let b1_ok = match b0 {
                0xE0 => (0xA0..=0xBF).contains(&b1), // else overlong (< U+0800)
                0xED => (0x80..=0x9F).contains(&b1), // else surrogate (U+D800..U+DFFF)
                _ => is_continuation(b1),
            };
            if !b1_ok || !is_continuation(input[pos + 2]) {
                return None;
            }
            Some(3)
        }
        0xF0..=0xF4 => {
            if pos + 3 >= len {
                return None;
            }
            let b1 = input[pos + 1];
            let b1_ok = match b0 {
                0xF0 => (0x90..=0xBF).contains(&b1), // else overlong (< U+10000)
                0xF4 => (0x80..=0x8F).contains(&b1), // else above U+10FFFF
                _ => is_continuation(b1),
            };
            if !b1_ok || !is_continuation(input[pos + 2]) || !is_continuation(input[pos + 3]) {
                return None;
            }
            Some(4)
        }
        // 0x80..=0xBF (stray continuation), 0xC0..=0xC1 (overlong 2-byte lead),
        // 0xF5..=0xFF (beyond U+10FFFF, or not a lead byte at all).
        _ => None,
    }
}

/// Return `true` iff `input` is well-formed UTF-8.
///
/// Two nested ASCII skips, then whole-sequence validation for everything else:
///
/// 1. A wide pass clearing [`BLOCK`] (32) bytes per high-bit test.
/// 2. A narrow pass clearing [`WORD`] (8) bytes, which also covers the
///    sub-block tail and lands `pos` on the first non-ASCII byte.
/// 3. [`validate_sequence`], consuming one whole multi-byte sequence with
///    independent range comparisons.
///
/// The loop maintains one invariant: **at the top of each iteration `pos` sits
/// on a code-point boundary** and `input[..pos]` is well-formed. That is what
/// makes it legal to reload a word at an arbitrary offset each time round, and
/// why a sequence straddling a word boundary needs no special case — step 3
/// indexes the whole slice rather than the loaded word, so it simply runs on
/// past the end of the word.
pub(crate) fn accepts(input: &[u8]) -> bool {
    let len = input.len();
    let mut pos = 0usize;

    while pos < len {
        // See `accepts`: guard the skip on the current byte being ASCII, so
        // multi-byte-heavy text does not pay for one that cannot fire.
        if input[pos] < 0x80 {
            // Wide pass first: clear 32 bytes at a time. This must `continue`
            // rather than fall through, so that the `input[pos] < 0x80` guard
            // above is re-evaluated at the new position — the narrow pass below
            // relies on it, and skipping the re-check let a stray continuation
            // byte immediately after a cleared block be consumed unvalidated.
            if let Some(block) = load_block(input, pos) {
                if block & HI == 0 {
                    pos += BLOCK;
                    continue;
                }
            }
            // Narrow pass, which also handles the sub-block tail.
            if let Some(word) = load_word(input, pos) {
                let hi = word & HI;
                if hi == 0 {
                    pos += WORD;
                    continue;
                }
                pos += first_high_byte(hi);
            } else {
                pos += 1;
                continue;
            }
        }

        match validate_sequence(input, pos) {
            Some(consumed) => pos += consumed,
            None => return false,
        }
    }

    true
}

/// Validate UTF-8 with the broadword accept scan, falling back to the scalar
/// validator for the precise [`Utf8Error`].
///
/// Portable: no SIMD intrinsics and no runtime feature detection, so it is
/// available on every target and under `no_std`.
///
/// Note that invalid input is scanned twice — once by the accept kernel, then
/// again by [`super::validate_utf8_scalar`] to pinpoint the error. Valid input
/// is scanned once.
///
/// # Examples
///
/// ```
/// use succinctly::text::utf8::validate_utf8_broadword;
///
/// // Valid ASCII
/// assert!(validate_utf8_broadword(b"Hello, world!").is_ok());
///
/// // Valid multi-byte UTF-8
/// assert!(validate_utf8_broadword("日本語".as_bytes()).is_ok());
/// assert!(validate_utf8_broadword("émoji: 🎉".as_bytes()).is_ok());
///
/// // Invalid: bare continuation byte
/// assert!(validate_utf8_broadword(&[0x80]).is_err());
///
/// // Invalid: truncated sequence
/// assert!(validate_utf8_broadword(&[0xC2]).is_err());
/// ```
pub fn validate_utf8_broadword(input: &[u8]) -> Result<(), Utf8Error> {
    if accepts(input) {
        Ok(())
    } else {
        validate_utf8_scalar(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The high-bit test detects a non-ASCII byte anywhere in the word.
    #[test]
    fn ascii_word_has_no_high_bytes() {
        assert_eq!(u64::from_ne_bytes(*b"abcdefgh") & HI, 0);
        for k in 0..WORD {
            let mut bytes = *b"abcdefgh";
            bytes[k] = 0xC2;
            assert_ne!(u64::from_ne_bytes(bytes) & HI, 0, "byte {k}");
        }
    }

    /// `first_high_byte` returns the memory-order index of the first non-ASCII
    /// byte, for every position in the word.
    #[test]
    fn first_high_byte_finds_first_non_ascii() {
        for k in 0..WORD {
            let mut bytes = *b"abcdefgh";
            bytes[k] = 0xC2;
            let hi = u64::from_ne_bytes(bytes) & HI;
            assert_eq!(first_high_byte(hi), k, "single high byte at {k}");

            // With a later byte also set, the *first* one must still win.
            if k + 1 < WORD {
                bytes[k + 1] = 0xE0;
                let hi = u64::from_ne_bytes(bytes) & HI;
                assert_eq!(first_high_byte(hi), k, "two high bytes from {k}");
            }
        }
    }

    /// Both endian halves are exercised on any host by driving them with
    /// synthetic masks, so neither arm of the `cfg!` in `first_high_byte` can
    /// rot unnoticed on a little-endian CI runner.
    #[test]
    fn first_high_byte_le_and_be_agree_on_synthetic_masks() {
        for k in 0..WORD {
            // Little-endian: memory byte k has its high bit at 8k + 7.
            assert_eq!(first_high_byte_le(1u64 << (8 * k + 7)), k, "le {k}");
            // Big-endian: memory byte k has its high bit at 63 - 8k.
            assert_eq!(first_high_byte_be(1u64 << (63 - 8 * k)), k, "be {k}");
        }
    }

    /// `load_word` yields words while eight bytes remain and `None` after,
    /// which is what terminates the main loop.
    #[test]
    fn load_word_stops_at_the_tail() {
        let input = b"abcdefghij";
        assert!(load_word(input, 0).is_some());
        assert!(load_word(input, 2).is_some());
        assert!(load_word(input, 3).is_none(), "only 7 bytes remain");
        assert!(load_word(input, 10).is_none(), "at end of input");
        assert!(load_word(b"", 0).is_none(), "empty input");
    }

    /// `load_word` cannot overflow when `pos` is near `usize::MAX`.
    #[test]
    fn load_word_handles_pos_overflow() {
        assert!(load_word(b"abcdefgh", usize::MAX).is_none());
    }

    /// `validate_sequence` reports the length of each well-formed sequence and
    /// rejects the malformed ones.
    #[test]
    fn validate_sequence_lengths_and_rejections() {
        assert_eq!(validate_sequence(b"a", 0), Some(1));
        assert_eq!(validate_sequence("é".as_bytes(), 0), Some(2));
        assert_eq!(validate_sequence("日".as_bytes(), 0), Some(3));
        assert_eq!(validate_sequence("🎉".as_bytes(), 0), Some(4));

        assert_eq!(validate_sequence(&[0x80], 0), None, "stray continuation");
        assert_eq!(validate_sequence(&[0xC0, 0x80], 0), None, "overlong lead");
        assert_eq!(validate_sequence(&[0xC1, 0x80], 0), None, "overlong lead");
        assert_eq!(validate_sequence(&[0xF5, 0x80, 0x80, 0x80], 0), None);
        assert_eq!(validate_sequence(&[0xC2], 0), None, "truncated");
        assert_eq!(validate_sequence(&[0xE0, 0xA0], 0), None, "truncated");
        assert_eq!(validate_sequence(&[0xF0, 0x90, 0x80], 0), None, "truncated");
        assert_eq!(validate_sequence(b"", 0), None, "empty");

        // The four range-restricted leads.
        assert_eq!(validate_sequence(&[0xE0, 0x9F, 0x80], 0), None, "overlong");
        assert_eq!(validate_sequence(&[0xE0, 0xA0, 0x80], 0), Some(3));
        assert_eq!(validate_sequence(&[0xED, 0xA0, 0x80], 0), None, "surrogate");
        assert_eq!(validate_sequence(&[0xED, 0x9F, 0x80], 0), Some(3));
        assert_eq!(validate_sequence(&[0xF0, 0x8F, 0x80, 0x80], 0), None);
        assert_eq!(validate_sequence(&[0xF0, 0x90, 0x80, 0x80], 0), Some(4));
        assert_eq!(validate_sequence(&[0xF4, 0x90, 0x80, 0x80], 0), None);
        assert_eq!(validate_sequence(&[0xF4, 0x8F, 0x80, 0x80], 0), Some(4));
    }

    /// An invalid byte immediately after a cleared wide block is still caught.
    ///
    /// Regression test. The wide ASCII pass advances `pos` by a whole block, so
    /// it must re-enter the loop rather than fall through to the narrow pass —
    /// the narrow pass's "fewer than eight bytes remain, consume one ASCII
    /// byte" branch assumes `input[pos]` was checked as ASCII, and once the
    /// block advance invalidated that assumption a stray continuation byte at
    /// `BLOCK` was consumed unvalidated. That is a *false accept* of invalid
    /// UTF-8, the worst failure direction for a validator.
    #[test]
    fn invalid_byte_immediately_after_a_cleared_block() {
        for tail in [0x80u8, 0xC0, 0xF5, 0xFF] {
            for pad in [BLOCK, BLOCK + 1, 2 * BLOCK, BLOCK + WORD] {
                let mut buf = vec![b'a'; pad];
                buf.push(tail);
                assert!(
                    core::str::from_utf8(&buf).is_err(),
                    "fixture should be invalid: pad {pad}, tail {tail:#04X}"
                );
                assert!(!accepts(&buf), "pad {pad}, tail {tail:#04X}");
            }
        }
    }

    /// The kernel agrees with `core::str::from_utf8` on a sequence that
    /// straddles the eight-byte word boundary — the case the invariant-based
    /// loop handles with no special code.
    #[test]
    fn multibyte_straddling_the_word_boundary() {
        for pad in 0..WORD {
            let mut good = vec![b'a'; pad];
            good.extend_from_slice("日本語".as_bytes());
            assert!(accepts(&good), "pad {pad}");
            assert!(core::str::from_utf8(&good).is_ok(), "pad {pad}");

            let mut bad = vec![b'a'; pad];
            bad.extend_from_slice(&[0xE0, 0x80, 0x80]); // overlong
            assert!(!accepts(&bad), "pad {pad}");
            assert!(core::str::from_utf8(&bad).is_err(), "pad {pad}");
        }
    }
}
