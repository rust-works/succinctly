//! x86_64 AVX2 UTF-8 validation.
//!
//! This is a first-principles port of the Keiser–Lemire "Validating UTF-8 In
//! Less Than One Instruction Per Byte" approach (<https://arxiv.org/abs/2010.03090>),
//! expressed with explicit range comparisons rather than packed lookup tables so
//! that every check maps directly onto the rules in [`validate_utf8_scalar`].

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
