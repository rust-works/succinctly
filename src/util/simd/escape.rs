#![allow(unsafe_code)] // runtime SIMD feature dispatch for escape scanning
//! Shared SIMD escape scanning.
//!
//! Finds the next byte in a string that a text format would need to *escape* —
//! the hot inner loop of streaming-output transcoders. Historically this lived in
//! `yaml/simd/` while `jq/stream.rs` consumed it, making jq depend on yaml
//! internals (#125). It now lives here as a neutral, shared utility that any
//! consumer (jq streaming, jq/yq output escaping in #91, jq format functions in
//! #124) can use without a cross-module dependency.
//!
//! ## The predicate seam
//!
//! The scanning machinery — the 16/32-byte SIMD chunk loop, the movemask +
//! `trailing_zeros` position extraction, and the scalar remainder — is identical
//! across escape predicates; only the *set of bytes considered special* differs
//! (JSON escapes `"` / `\` / `< 0x20`; a future `@html` scanner would want
//! `<` / `>` / `&` / `'` / `"`, etc.). [`define_escape_scanner!`] captures the
//! machinery once and takes the predicate as a parameter: a scalar `|b| ...`
//! expression plus three per-backend mask helpers (NEON / AVX2 / SSE2). Adding a
//! new scanner (#124) is a new macro invocation, not another refactor of this
//! file. Only [`find_json_escape`] is instantiated today.
//!
//! ## Dispatch
//!
//! Mirrors the previous `yaml/simd` dispatch exactly so output stays
//! byte-identical: NEON on aarch64 (16-byte), AVX2/SSE2 on x86_64 (32/16-byte),
//! scalar everywhere else and under `--features scalar-yaml`. AVX2 vs SSE2 is
//! chosen once per process via runtime detection; both compute the same answer,
//! so the choice never affects output.

// Intrinsic types used in the per-backend mask-helper signatures below.
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
use core::arch::aarch64::*;
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
use core::arch::x86_64::*;

/// Extract a bitmask from the high bit of each byte in a NEON vector.
///
/// Returns a `u16` where bit `i` is set iff byte `i` has its high bit set. Shared
/// across escape predicates (position extraction is predicate-agnostic). This is
/// the same multiplication-trick emulation used elsewhere in the crate.
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
#[inline]
#[target_feature(enable = "neon")]
unsafe fn neon_movemask(v: uint8x16_t) -> u16 {
    // Shift right by 7 → 0 or 1 in each byte.
    let high_bits = vshrq_n_u8::<7>(v);
    // Extract as two u64 lanes.
    let low_u64 = vgetq_lane_u64::<0>(vreinterpretq_u64_u8(high_bits));
    let high_u64 = vgetq_lane_u64::<1>(vreinterpretq_u64_u8(high_bits));
    // Pack 8 bytes into 8 bits with the classic multiply.
    const MAGIC: u64 = 0x0102040810204080;
    let low_packed = (low_u64.wrapping_mul(MAGIC) >> 56) as u8;
    let high_packed = (high_u64.wrapping_mul(MAGIC) >> 56) as u8;
    (low_packed as u16) | ((high_packed as u16) << 8)
}

/// Whether the 32-byte AVX2 kernels should be dispatched.
///
/// Self-contained runtime detection (this module deliberately does not depend on
/// `yaml/simd`, so it does not honor the `SUCCINCTLY_SIMD` dispatch clamp; both
/// kernels compute the same answer, so escape output is identical either way).
/// Cached per process (STYLE-0003): dispatch is on the per-chunk hot path.
#[cfg(all(
    target_arch = "x86_64",
    not(feature = "scalar-yaml"),
    any(test, feature = "std")
))]
#[inline]
fn avx2_enabled() -> bool {
    static AVX2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVX2.get_or_init(|| is_x86_feature_detected!("avx2"))
}

/// Generate a complete escape scanner (scalar + NEON + AVX2/SSE2 + dispatch) that
/// shares the chunked-scan machinery, parameterized by an escape predicate.
///
/// `scalar` is the byte-level predicate (used by the scalar fallback and every
/// SIMD kernel's remainder tail). `neon_mask` / `avx2_mask` / `sse2_mask` name
/// `#[target_feature]` helper fns that turn a loaded chunk into an all-ones/zeros
/// per-lane match vector for that predicate. The generated `find` reproduces the
/// exact per-arch dispatch of the pre-#125 `yaml/simd::find_json_escape`.
macro_rules! define_escape_scanner {
    (
        $(#[$meta:meta])*
        mod $modname:ident;
        scalar: |$b:ident| $pred:expr;
        neon_mask: $neon_mask:ident;
        avx2_mask: $avx2_mask:ident;
        sse2_mask: $sse2_mask:ident;
    ) => {
        $(#[$meta])*
        mod $modname {
            #[cfg(all(
                target_arch = "aarch64",
                not(feature = "broadword-yaml"),
                not(feature = "scalar-yaml")
            ))]
            use core::arch::aarch64::*;
            #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
            use core::arch::x86_64::*;

            /// Scalar reference and portable fallback. Returns the index of the
            /// first special byte at or after `start`, or `bytes.len()`.
            #[inline(always)]
            #[allow(dead_code)] // reference/fallback; SIMD path used when detected
            pub(crate) fn scalar(bytes: &[u8], start: usize) -> usize {
                for (i, &$b) in bytes[start..].iter().enumerate() {
                    if $pred {
                        return start + i;
                    }
                }
                bytes.len()
            }

            // ---- aarch64 NEON (16 bytes/iter) --------------------------------
            #[cfg(all(
                target_arch = "aarch64",
                not(feature = "broadword-yaml"),
                not(feature = "scalar-yaml")
            ))]
            #[inline(always)]
            pub(crate) fn neon(bytes: &[u8], start: usize) -> usize {
                if start >= bytes.len() {
                    return bytes.len();
                }
                // NEON wins from 16 bytes; scalar is faster for shorter tails.
                if bytes.len() - start >= 16 {
                    // SAFETY: NEON is mandatory on aarch64.
                    unsafe { neon_impl(bytes, start) }
                } else {
                    scalar(bytes, start)
                }
            }

            #[cfg(all(
                target_arch = "aarch64",
                not(feature = "broadword-yaml"),
                not(feature = "scalar-yaml")
            ))]
            #[target_feature(enable = "neon")]
            unsafe fn neon_impl(bytes: &[u8], start: usize) -> usize {
                let len = bytes.len();
                let data = &bytes[start..];
                let data_len = data.len();
                let mut offset = 0;
                while offset + 16 <= data_len {
                    let chunk = vld1q_u8(data.as_ptr().add(offset));
                    let matches = super::$neon_mask(chunk);
                    let mask = super::neon_movemask(matches);
                    if mask != 0 {
                        return start + offset + mask.trailing_zeros() as usize;
                    }
                    offset += 16;
                }
                for (i, &$b) in data.iter().enumerate().skip(offset) {
                    if $pred {
                        return start + i;
                    }
                }
                len
            }

            // ---- x86_64 AVX2 (32) / SSE2 (16) --------------------------------
            /// x86 dispatch: AVX2 when detected, else the SSE2 baseline.
            #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
            #[inline(always)]
            pub(crate) fn x86(bytes: &[u8], start: usize) -> usize {
                #[cfg(any(test, feature = "std"))]
                {
                    dispatch(bytes, start, super::avx2_enabled())
                }
                // No runtime detection without std: SSE2 is the x86_64 baseline.
                #[cfg(not(any(test, feature = "std")))]
                {
                    // SAFETY: SSE2 is the x86_64 baseline.
                    unsafe { sse2(bytes, start) }.map_or(bytes.len(), |off| start + off)
                }
            }

            /// Tier selection split out so both branches are deterministically
            /// testable: on an AVX2 runner the SSE2-baseline branch is otherwise
            /// never taken by dispatch.
            #[cfg(all(
                target_arch = "x86_64",
                not(feature = "scalar-yaml"),
                any(test, feature = "std")
            ))]
            #[inline(always)]
            pub(crate) fn dispatch(bytes: &[u8], start: usize, use_avx2: bool) -> usize {
                if use_avx2 {
                    // SAFETY: callers pass true only when AVX2 is detected.
                    unsafe { avx2(bytes, start) }.map_or(bytes.len(), |off| start + off)
                } else {
                    // SAFETY: SSE2 is the x86_64 baseline.
                    unsafe { sse2(bytes, start) }.map_or(bytes.len(), |off| start + off)
                }
            }

            /// AVX2 kernel: 32-byte main loop, 16-byte SSE2 tail, scalar remainder.
            /// Returns the offset from `start` to the first special byte, or `None`.
            #[cfg(all(
                target_arch = "x86_64",
                not(feature = "scalar-yaml"),
                any(test, feature = "std")
            ))]
            #[target_feature(enable = "avx2")]
            pub(crate) unsafe fn avx2(input: &[u8], start: usize) -> Option<usize> {
                let len = input.len();
                if start >= len {
                    return None;
                }
                let data = &input[start..];
                let data_len = data.len();
                let mut offset = 0;
                while offset + 32 <= data_len {
                    let chunk = _mm256_loadu_si256(data.as_ptr().add(offset).cast::<__m256i>());
                    let mask = _mm256_movemask_epi8(super::$avx2_mask(chunk)) as u32;
                    if mask != 0 {
                        return Some(offset + mask.trailing_zeros() as usize);
                    }
                    offset += 32;
                }
                if offset + 16 <= data_len {
                    let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());
                    let mask = _mm_movemask_epi8(super::$sse2_mask(chunk)) as u32;
                    if mask != 0 {
                        return Some(offset + mask.trailing_zeros() as usize);
                    }
                    offset += 16;
                }
                for (i, &$b) in data.iter().enumerate().skip(offset) {
                    if $pred {
                        return Some(i);
                    }
                }
                None
            }

            /// SSE2 kernel: 16-byte loop plus scalar remainder.
            #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
            #[target_feature(enable = "sse2")]
            pub(crate) unsafe fn sse2(input: &[u8], start: usize) -> Option<usize> {
                let len = input.len();
                if start >= len {
                    return None;
                }
                let data = &input[start..];
                let data_len = data.len();
                let mut offset = 0;
                while offset + 16 <= data_len {
                    let chunk = _mm_loadu_si128(data.as_ptr().add(offset).cast::<__m128i>());
                    let mask = _mm_movemask_epi8(super::$sse2_mask(chunk)) as u32;
                    if mask != 0 {
                        return Some(offset + mask.trailing_zeros() as usize);
                    }
                    offset += 16;
                }
                for (i, &$b) in data.iter().enumerate().skip(offset) {
                    if $pred {
                        return Some(i);
                    }
                }
                None
            }

            /// Public entry: index of the first special byte at/after `start`, or
            /// `bytes.len()`. Fast-exits when `start` is already past the end.
            #[inline(always)]
            pub(crate) fn find(bytes: &[u8], start: usize) -> usize {
                if start >= bytes.len() {
                    return bytes.len();
                }

                #[cfg(all(
                    target_arch = "aarch64",
                    not(feature = "broadword-yaml"),
                    not(feature = "scalar-yaml")
                ))]
                {
                    neon(bytes, start)
                }

                #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
                {
                    x86(bytes, start)
                }

                // Scalar everywhere else: under `scalar-yaml`, on non-x86/arm
                // targets, and on aarch64 under `broadword-yaml` (there is no
                // broadword escape kernel, so the SWAR build falls back to
                // scalar — the pre-#125 dispatch omitted this arm and failed to
                // compile in that configuration).
                #[cfg(any(
                    feature = "scalar-yaml",
                    all(target_arch = "aarch64", feature = "broadword-yaml"),
                    not(any(target_arch = "aarch64", target_arch = "x86_64"))
                ))]
                {
                    scalar(bytes, start)
                }
            }
        }
    };
}

// ---- JSON escape predicate -------------------------------------------------
//
// A byte needs JSON escaping iff it is `"`, `\`, or a control character
// (`< 0x20`, an UNSIGNED comparison). The unsigned test is load-bearing: a signed
// compare treats every byte >= 0x80 as negative and misreads UTF-8 continuation
// bytes as controls, returning an index mid-character and panicking callers that
// slice on it (#150/#230). x86 has no unsigned "less than", so we use saturating
// unsigned subtract: `subs_epu8(b, 0x1F) == 0` exactly when `b <= 0x1F`. Never
// reach for `cmpgt_epi8` here.

/// NEON match mask for the JSON escape predicate (`"`, `\`, `< 0x20`).
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
#[inline]
#[target_feature(enable = "neon")]
unsafe fn json_neon_mask(chunk: uint8x16_t) -> uint8x16_t {
    vorrq_u8(
        vorrq_u8(
            vceqq_u8(chunk, vdupq_n_u8(b'"')),
            vceqq_u8(chunk, vdupq_n_u8(b'\\')),
        ),
        vcltq_u8(chunk, vdupq_n_u8(0x20)), // unsigned byte < 0x20
    )
}

/// AVX2 match mask for the JSON escape predicate.
#[cfg(all(
    target_arch = "x86_64",
    not(feature = "scalar-yaml"),
    any(test, feature = "std")
))]
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn json_avx2_mask(chunk: __m256i) -> __m256i {
    _mm256_or_si256(
        _mm256_or_si256(
            _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'"' as i8)),
            _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\\' as i8)),
        ),
        // byte < 0x20, unsigned — see the note above.
        _mm256_cmpeq_epi8(
            _mm256_subs_epu8(chunk, _mm256_set1_epi8(0x1F)),
            _mm256_setzero_si256(),
        ),
    )
}

/// SSE2 match mask for the JSON escape predicate.
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
#[inline]
#[target_feature(enable = "sse2")]
unsafe fn json_sse2_mask(chunk: __m128i) -> __m128i {
    _mm_or_si128(
        _mm_or_si128(
            _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'"' as i8)),
            _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'\\' as i8)),
        ),
        // byte < 0x20, unsigned — see the note above.
        _mm_cmpeq_epi8(
            _mm_subs_epu8(chunk, _mm_set1_epi8(0x1F)),
            _mm_setzero_si128(),
        ),
    )
}

define_escape_scanner! {
    /// JSON string escape scanner.
    mod json_escape;
    scalar: |b| b == b'"' || b == b'\\' || b < 0x20;
    neon_mask: json_neon_mask;
    avx2_mask: json_avx2_mask;
    sse2_mask: json_sse2_mask;
}

/// Find the next JSON-escapable byte in `bytes` at or after `start`.
///
/// Searches for `"`, `\`, or a control character (`< 0x20`) and returns the index
/// of the first match, or `bytes.len()` if none is found. This is the hot-path
/// scan for `write_json_string` in the YAML→JSON streaming transcoder and for
/// jq's streaming string output; it processes 16–32 bytes at a time with SIMD.
///
/// `#[inline(always)]` is load-bearing: without it the SIMD path regresses versus
/// scalar on short strings (O3 / #87).
#[inline(always)]
pub fn find_json_escape(bytes: &[u8], start: usize) -> usize {
    json_escape::find(bytes, start)
}

// ---- JSON string-content predicate -----------------------------------------
//
// The strict validator (`json::validate`) needs a *stricter* stop set than the
// transcoder: on top of `"` / `\` / `< 0x20` it must also stop at every byte
// `>= 0x80`, because those begin a UTF-8 sequence whose well-formedness the
// validator has to check. Everything between two stops is plain printable ASCII
// that is unconditionally valid string content, so the validator can skip the
// whole run with one `offset`/`column` addition (#123).
//
// The `>= 0x80` term is written as an explicit compare rather than by folding the
// raw chunk into the mask. Folding the chunk in would also work — the scanner
// machinery only ever `movemask`s the result, and `movemask` reads exactly the
// high bit — but it would break the all-ones/all-zeros per-lane contract these
// helpers are documented to uphold, for one instruction.

/// NEON match mask for the JSON string-content predicate (`"`, `\`, `< 0x20`, `>= 0x80`).
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
#[inline]
#[target_feature(enable = "neon")]
unsafe fn json_stop_neon_mask(chunk: uint8x16_t) -> uint8x16_t {
    // SAFETY: forwarding a loaded chunk to a sibling `neon` target-feature fn.
    unsafe {
        vorrq_u8(
            json_neon_mask(chunk),
            vcgeq_u8(chunk, vdupq_n_u8(0x80)), // unsigned byte >= 0x80
        )
    }
}

/// AVX2 match mask for the JSON string-content predicate.
#[cfg(all(
    target_arch = "x86_64",
    not(feature = "scalar-yaml"),
    any(test, feature = "std")
))]
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn json_stop_avx2_mask(chunk: __m256i) -> __m256i {
    // SAFETY: forwarding a loaded chunk to a sibling `avx2` target-feature fn.
    unsafe {
        let high = _mm256_set1_epi8(0x80u8 as i8);
        _mm256_or_si256(
            json_avx2_mask(chunk),
            // byte >= 0x80, i.e. the high bit is set. `cmpgt_epi8` would be a
            // signed compare here and is never correct — see the note above.
            _mm256_cmpeq_epi8(_mm256_and_si256(chunk, high), high),
        )
    }
}

/// SSE2 match mask for the JSON string-content predicate.
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
#[inline]
#[target_feature(enable = "sse2")]
unsafe fn json_stop_sse2_mask(chunk: __m128i) -> __m128i {
    // SAFETY: forwarding a loaded chunk to a sibling `sse2` target-feature fn.
    unsafe {
        let high = _mm_set1_epi8(0x80u8 as i8);
        _mm_or_si128(
            json_sse2_mask(chunk),
            _mm_cmpeq_epi8(_mm_and_si128(chunk, high), high),
        )
    }
}

define_escape_scanner! {
    /// JSON string-content scanner (validator stop set).
    mod json_string_stop;
    // `!(0x20..0x80)` is "not printable ASCII": control chars and UTF-8 lead /
    // continuation bytes alike. Spelled as a range so clippy::manual_range_contains
    // is satisfied; the SIMD helpers test the two halves separately.
    scalar: |b| b == b'"' || b == b'\\' || !(0x20..0x80).contains(&b);
    neon_mask: json_stop_neon_mask;
    avx2_mask: json_stop_avx2_mask;
    sse2_mask: json_stop_sse2_mask;
}

/// Find the next byte in `bytes` at or after `start` that the strict JSON
/// validator must stop and inspect.
///
/// Stops at `"` (string end), `\` (escape), a control character (`< 0x20`, always
/// an error inside a string), or any byte `>= 0x80` (the start of a UTF-8
/// sequence needing validation). Returns `bytes.len()` if none is found.
///
/// Every byte strictly between two stops is printable ASCII, which is always
/// valid JSON string content — so `json::validate` advances over the whole run at
/// once instead of byte at a time (#123).
///
/// `#[inline(always)]` matches [`find_json_escape`] and is load-bearing for the
/// same reason (O3 / #87): without it the SIMD path regresses versus scalar on
/// short strings, and JSON keys are typically short.
#[inline(always)]
pub fn find_json_string_stop(bytes: &[u8], start: usize) -> usize {
    json_string_stop::find(bytes, start)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent scalar reference (kept separate from the module's own `scalar`
    /// so a bug in the shared machinery cannot mask itself).
    fn reference(bytes: &[u8], start: usize) -> usize {
        for (i, &b) in bytes[start..].iter().enumerate() {
            if b == b'"' || b == b'\\' || b < 0x20 {
                return start + i;
            }
        }
        bytes.len()
    }

    #[test]
    fn basic_quote() {
        assert_eq!(find_json_escape(b"hello\"world", 0), 5);
    }

    #[test]
    fn basic_backslash() {
        assert_eq!(find_json_escape(b"hello\\world", 0), 5);
    }

    #[test]
    fn basic_control() {
        assert_eq!(find_json_escape(b"hello\nworld", 0), 5);
        assert_eq!(find_json_escape(b"hello\tworld", 0), 5);
        assert_eq!(find_json_escape(b"hello\x00world", 0), 5);
    }

    #[test]
    fn none_found_returns_len() {
        let input = b"hello world";
        assert_eq!(find_json_escape(input, 0), input.len());
    }

    #[test]
    fn long_string_simd_path() {
        let mut input = vec![b'a'; 100];
        input[50] = b'"';
        assert_eq!(find_json_escape(&input, 0), 50);
    }

    #[test]
    fn empty_input() {
        assert_eq!(find_json_escape(b"", 0), 0);
    }

    #[test]
    fn start_past_end() {
        let input = b"hello";
        assert_eq!(find_json_escape(input, 10), input.len());
    }

    #[test]
    fn with_offset() {
        let input = b"abc\"def\"ghi";
        assert_eq!(find_json_escape(input, 0), 3);
        assert_eq!(find_json_escape(input, 4), 7);
    }

    #[test]
    fn escape_at_chunk_and_double_chunk_boundary() {
        // Straddle the 16-byte and 32-byte chunk edges.
        for &(size, pos) in &[(32usize, 16usize), (64, 32), (32, 15)] {
            let mut input = vec![b'a'; size];
            input[pos] = if pos == 15 { b'\t' } else { b'"' };
            assert_eq!(find_json_escape(&input, 0), pos);
        }
    }

    #[test]
    fn simd_matches_scalar_including_utf8() {
        let cases: &[&[u8]] = &[
            b"",
            b"\"",
            b"\\",
            b"\n",
            b"\t",
            b"\r",
            b"\x00",
            b"no special chars here",
            b"quote at end\"",
            b"\"quote at start",
            b"has\\backslash",
            b"has both \" and \\ chars",
            b"control\x01char",
            &[b'x'; 100],
            // Non-ASCII: `byte < 0x20` must be an UNSIGNED test; a signed one
            // flags every byte >= 0x80 (#150/#230).
            "love ♥ and peace ☮".as_bytes(),
            "aaaaaaaaaaaaaaaa♥".as_bytes(),
            "日本語のテキストはここにあります".as_bytes(),
            "emoji 😁 in a string long enough for a full chunk".as_bytes(),
        ];
        for &input in cases {
            for start in 0..=input.len() {
                assert_eq!(
                    reference(input, start),
                    find_json_escape(input, start),
                    "mismatch for {input:?} at start {start}"
                );
            }
        }
    }

    /// Every byte value at every alignment, against the scalar reference.
    ///
    /// The test that would have caught #230: the x86 path once used a *signed*
    /// compare for `byte < 0x20`, so bytes >= 0x80 read as negative and were
    /// reported as control characters. Sweeping the offset matters as much as the
    /// value — SIMD engages only on full 16/32-byte chunks, so a byte is checked
    /// by SIMD at some positions and by the scalar tail at others.
    #[test]
    fn exhaustive_bytes_match_scalar() {
        for byte in 0u8..=255 {
            for pos in 0..40usize {
                let mut input = vec![b'a'; 40];
                input[pos] = byte;
                assert_eq!(
                    reference(&input, 0),
                    find_json_escape(&input, 0),
                    "mismatch for byte {byte:#04x} at offset {pos}"
                );
            }
        }
    }

    /// The contract callers rely on: the returned index either is the end of
    /// input or points at a byte that genuinely needs escaping — never mid-UTF-8.
    #[test]
    fn never_points_at_a_safe_byte() {
        let inputs: &[&str] = &[
            "love ♥ and peace ☮",
            "aaaaaaaaaaaaaaaa♥bbbbbbbbbbbbbbbb",
            "日本語のテキストはここにあります、もっと長く",
            "emoji 😁 in a string long enough for a full chunk",
            "mixed ♥ with \"quotes\" and \\slashes\\ and \nnewlines",
        ];
        for s in inputs {
            let bytes = s.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let pos = find_json_escape(bytes, i);
                assert!(pos >= i && pos <= bytes.len(), "out of range for {s:?}");
                if pos == bytes.len() {
                    break;
                }
                let b = bytes[pos];
                assert!(
                    b == b'"' || b == b'\\' || b < 0x20,
                    "index {pos} for {s:?} points at safe byte {b:#04x}"
                );
                i = pos + 1;
            }
        }
    }

    /// The module-level `scalar` fallback is only reached through the SIMD
    /// kernels' short-string path (arch-dependent) or a scalar-only build, so
    /// exercise it directly on every target for parity and coverage.
    #[test]
    fn scalar_fallback_matches_reference() {
        let cases: &[&[u8]] = &[
            b"",
            b"\"",
            b"\\",
            b"\n",
            b"plain text",
            b"quote\"in\\the\tmiddle",
            "caf\u{e9} \u{2665} unicode".as_bytes(),
            &[b'x'; 40],
        ];
        for &input in cases {
            for start in 0..=input.len() {
                assert_eq!(
                    reference(input, start),
                    super::json_escape::scalar(input, start),
                    "scalar mismatch for {input:?} at start {start}"
                );
            }
        }
    }

    // ------------------------------------------------------------------------
    // Per-kernel differential tests (#193): exercise each x86 kernel directly,
    // regardless of what the dispatcher would pick, so the SSE2 kernel is tested
    // on AVX2 hardware and both are checked against the scalar reference across
    // the byte range (the tests that would have caught #150/#230). Gated off under
    // `scalar-yaml`, which cfg-excludes the very kernels these tests call.
    // ------------------------------------------------------------------------
    #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
    mod x86_kernels {
        use super::reference;

        /// AVX2 detection guard; emits a visible `SKIPPED` line when unavailable
        /// so a fully-skipped kernel does not read as green (#193).
        fn has_avx2() -> bool {
            crate::util::simd::note_simd_skip_unless(is_x86_feature_detected!("avx2"), "avx2")
        }

        fn sse2_index(input: &[u8], start: usize) -> usize {
            // SAFETY: SSE2 is the x86_64 baseline.
            unsafe { super::super::json_escape::sse2(input, start) }
                .map_or(input.len(), |off| start + off)
        }

        fn avx2_index(input: &[u8], start: usize) -> usize {
            // SAFETY: gated on runtime AVX2 detection by callers.
            unsafe { super::super::json_escape::avx2(input, start) }
                .map_or(input.len(), |off| start + off)
        }

        /// Multibyte UTF-8 cases sized to hit the SSE2 16-byte loop, the AVX2
        /// 32-byte loop, its 16-byte tail, and the scalar tails.
        fn non_ascii_cases() -> Vec<&'static [u8]> {
            vec![
                "café au lait, s'il vous plaît".as_bytes(),
                "love ♥ and peace ☮".as_bytes(), // original #230 repro
                "日本語のテキストはここにあります".as_bytes(),
                "emoji 😁 in a string long enough for a full chunk".as_bytes(),
                "aaaaaaaaaaaaaaaa♥".as_bytes(), // multibyte at the 16-byte boundary
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa😁".as_bytes(), // at the 32-byte boundary
                "mixed \"quote\" and café and \\ backslash".as_bytes(),
                "日本語 with a \n control in the middle of CJK 文字".as_bytes(),
            ]
        }

        #[test]
        fn kernels_match_scalar_non_ascii() {
            let run_avx2 = has_avx2();
            for input in non_ascii_cases() {
                for start in [0usize, 1, 3] {
                    assert_eq!(
                        reference(input, start),
                        sse2_index(input, start),
                        "SSE2 mismatch for {input:?} at start {start}"
                    );
                    if run_avx2 {
                        assert_eq!(
                            reference(input, start),
                            avx2_index(input, start),
                            "AVX2 mismatch for {input:?} at start {start}"
                        );
                    }
                }
            }
        }

        #[test]
        fn sse2_exhaustive_bytes_match_scalar() {
            // Two 16-byte SSE2 chunks plus the scalar tail; bytes >= 0x80 are the
            // ones the signed compare misclassified (#150/#230).
            for byte in 0u8..=255 {
                for pos in 0..40 {
                    let mut input = [b'a'; 40];
                    input[pos] = byte;
                    assert_eq!(
                        reference(&input, 0),
                        sse2_index(&input, 0),
                        "SSE2 mismatch for byte 0x{byte:02x} at {pos}"
                    );
                }
            }
        }

        #[test]
        fn avx2_exhaustive_bytes_match_scalar() {
            // 56-byte buffer: one 32-byte AVX2 chunk, one 16-byte SSE2 tail, and
            // an 8-byte scalar tail — every internal path of the kernel. Skipped
            // (visibly, via has_avx2) on hardware without AVX2.
            if has_avx2() {
                for byte in 0u8..=255 {
                    for pos in 0..56 {
                        let mut input = [b'a'; 56];
                        input[pos] = byte;
                        assert_eq!(
                            reference(&input, 0),
                            avx2_index(&input, 0),
                            "AVX2 mismatch for byte 0x{byte:02x} at {pos}"
                        );
                    }
                }
            }
        }

        #[test]
        fn kernels_return_early_when_start_at_or_past_end() {
            // The `start >= len` guard in each kernel is never reached through
            // find() (which guards start earlier); exercise it directly.
            assert_eq!(sse2_index(b"", 0), 0);
            assert_eq!(sse2_index(b"abc", 3), 3);
            assert_eq!(sse2_index(b"abc", 9), 3);
            if has_avx2() {
                assert_eq!(avx2_index(b"", 0), 0);
                assert_eq!(avx2_index(b"abc", 3), 3);
                assert_eq!(avx2_index(b"abc", 9), 3);
            }
        }

        #[test]
        fn dispatch_selects_both_tiers() {
            // Cover both dispatch branches: the SSE2 baseline (always reachable)
            // and the AVX2 tier (when detected). find() only ever picks AVX2 on
            // this runner, so the SSE2-baseline branch needs an explicit call.
            let input = b"a long enough plain string \"with\" some \\ escapes inside";
            let want = reference(input, 0);
            assert_eq!(super::super::json_escape::dispatch(input, 0, false), want);
            if has_avx2() {
                assert_eq!(super::super::json_escape::dispatch(input, 0, true), want);
            }
        }
    }

    // ------------------------------------------------------------------------
    // NEON kernel exhaustive check (#186): every byte at positions covering the
    // 16-byte NEON path, the chunk boundary, and the <16-byte scalar remainder.
    // ------------------------------------------------------------------------
    #[cfg(all(
        target_arch = "aarch64",
        not(feature = "broadword-yaml"),
        not(feature = "scalar-yaml")
    ))]
    mod neon_kernel {
        use super::reference;

        #[test]
        fn exhaustive_bytes_match_scalar() {
            for b in 0u8..=255 {
                for &pos in &[0usize, 1, 7, 15, 16, 17, 31, 32, 33, 47] {
                    let mut input = vec![b'A'; 48];
                    input[pos] = b;
                    for &start in &[0usize, 3, 16] {
                        assert_eq!(
                            reference(&input, start),
                            super::super::json_escape::neon(&input, start),
                            "NEON mismatch for byte 0x{b:02x} at pos {pos}, start {start}"
                        );
                    }
                }
            }
        }

        #[test]
        fn neon_returns_len_when_start_past_end() {
            // The kernel's own `start >= len` guard, not reached through find().
            assert_eq!(super::super::json_escape::neon(b"abc", 9), 3);
        }
    }

    // ------------------------------------------------------------------------
    // `find_json_string_stop` (#123): same machinery, stricter predicate — it also
    // stops at every byte >= 0x80. The `>= 0x80` term is where a signed compare
    // would silently do the right thing for the wrong reason, so these mirror the
    // exhaustive sweeps above rather than spot-checking.
    // ------------------------------------------------------------------------
    mod string_stop {
        use super::super::find_json_string_stop;

        /// Independent scalar reference, deliberately spelled with explicit
        /// comparisons rather than the range form the scanner uses — clippy's
        /// `!(0x20..0x80).contains(&b)` rewrite would make this a copy of the
        /// implementation and defeat the point of an independent reference.
        #[allow(clippy::manual_range_contains)]
        fn reference(bytes: &[u8], start: usize) -> usize {
            for (i, &b) in bytes[start..].iter().enumerate() {
                if b == b'"' || b == b'\\' || b < 0x20 || b >= 0x80 {
                    return start + i;
                }
            }
            bytes.len()
        }

        #[test]
        fn basic_stops() {
            assert_eq!(find_json_string_stop(b"hello\"world", 0), 5);
            assert_eq!(find_json_string_stop(b"hello\\world", 0), 5);
            assert_eq!(find_json_string_stop(b"hello\nworld", 0), 5);
            assert_eq!(find_json_string_stop(b"hello\x00world", 0), 5);
            // The term `find_json_escape` does not have: a UTF-8 lead byte.
            assert_eq!(find_json_string_stop("hello♥world".as_bytes(), 0), 5);
            assert_eq!(find_json_string_stop(b"hello world", 0), 11);
        }

        #[test]
        fn empty_and_start_past_end() {
            assert_eq!(find_json_string_stop(b"", 0), 0);
            assert_eq!(find_json_string_stop(b"hello", 10), 5);
            assert_eq!(find_json_string_stop(b"hello", 5), 5);
        }

        /// Every byte value at every alignment across a 56-byte buffer: one AVX2
        /// 32-byte chunk, a 16-byte SSE2/NEON tail, and an 8-byte scalar remainder,
        /// so each byte is checked by SIMD at some offsets and scalar at others.
        #[test]
        fn exhaustive_bytes_match_scalar() {
            for byte in 0u8..=255 {
                for pos in 0..56usize {
                    let mut input = vec![b'a'; 56];
                    input[pos] = byte;
                    for &start in &[0usize, 1, 16, 33] {
                        assert_eq!(
                            reference(&input, start),
                            find_json_string_stop(&input, start),
                            "mismatch for byte {byte:#04x} at offset {pos}, start {start}"
                        );
                    }
                }
            }
        }

        /// The contract `validate_string` relies on: everything strictly between
        /// two stops is printable ASCII, so the run can be skipped wholesale.
        #[test]
        fn runs_between_stops_are_printable_ascii() {
            let inputs: &[&str] = &[
                "love ♥ and peace ☮",
                "aaaaaaaaaaaaaaaa♥bbbbbbbbbbbbbbbb",
                "日本語のテキストはここにあります、もっと長く",
                "emoji 😁 in a string long enough for a full chunk",
                "mixed ♥ with \"quotes\" and \\slashes\\ and \nnewlines",
                "plain ascii with no stops at all in this run",
            ];
            for s in inputs {
                let bytes = s.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    let pos = find_json_string_stop(bytes, i);
                    assert!(pos >= i && pos <= bytes.len(), "out of range for {s:?}");
                    for &b in &bytes[i..pos] {
                        assert!(
                            (0x20..0x80).contains(&b) && b != b'"' && b != b'\\',
                            "skipped byte {b:#04x} in {s:?} is not plain string content"
                        );
                    }
                    if pos == bytes.len() {
                        break;
                    }
                    i = pos + 1;
                }
            }
        }

        /// The module's own scalar fallback is reached only via the short-string
        /// path or a scalar-only build; exercise it directly on every target.
        #[test]
        fn scalar_fallback_matches_reference() {
            let cases: &[&[u8]] = &[
                b"",
                b"\"",
                b"\\",
                b"\n",
                b"plain text",
                b"quote\"in\\the\tmiddle",
                "caf\u{e9} \u{2665} unicode".as_bytes(),
                &[b'x'; 40],
            ];
            for &input in cases {
                for start in 0..=input.len() {
                    assert_eq!(
                        reference(input, start),
                        super::super::json_string_stop::scalar(input, start),
                        "scalar mismatch for {input:?} at start {start}"
                    );
                }
            }
        }

        /// Drive each x86 kernel directly so the SSE2 one is covered on AVX2
        /// hardware, where dispatch would never choose it.
        #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
        #[test]
        fn x86_kernels_match_scalar() {
            let run_avx2 =
                crate::util::simd::note_simd_skip_unless(is_x86_feature_detected!("avx2"), "avx2");
            for byte in 0u8..=255 {
                for pos in 0..56usize {
                    let mut input = [b'a'; 56];
                    input[pos] = byte;
                    let want = reference(&input, 0);
                    // SAFETY: SSE2 is the x86_64 baseline.
                    let got = unsafe { super::super::json_string_stop::sse2(&input, 0) }
                        .map_or(input.len(), |off| off);
                    assert_eq!(want, got, "SSE2 mismatch for {byte:#04x} at {pos}");
                    if run_avx2 {
                        // SAFETY: guarded by the AVX2 runtime detection above.
                        let got = unsafe { super::super::json_string_stop::avx2(&input, 0) }
                            .map_or(input.len(), |off| off);
                        assert_eq!(want, got, "AVX2 mismatch for {byte:#04x} at {pos}");
                    }
                }
            }
        }

        /// Drive the NEON kernel directly, including its 16-byte boundary and the
        /// sub-16-byte scalar remainder.
        #[cfg(all(
            target_arch = "aarch64",
            not(feature = "broadword-yaml"),
            not(feature = "scalar-yaml")
        ))]
        #[test]
        fn neon_kernel_matches_scalar() {
            for byte in 0u8..=255 {
                for &pos in &[0usize, 1, 7, 15, 16, 17, 31, 32, 33, 47] {
                    let mut input = vec![b'A'; 48];
                    input[pos] = byte;
                    for &start in &[0usize, 3, 16] {
                        assert_eq!(
                            reference(&input, start),
                            super::super::json_string_stop::neon(&input, start),
                            "NEON mismatch for byte {byte:#04x} at pos {pos}, start {start}"
                        );
                    }
                }
            }
        }
    }
}
