#![allow(unsafe_code)]
// runtime SIMD feature dispatch for escape scanning
// Each scalar predicate below is written as an explicit OR of the same lanes its
// SIMD mask helper computes, one term per compare, so the two can be read against
// each other. Clippy would fold `b < 0x20 || b >= 0x7F` into a range test, which
// is equivalent but severs that correspondence — and the correspondence is the
// only thing keeping the scalar reference and the kernels honest.
#![allow(clippy::manual_range_contains)]
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
//! file.
//!
//! Three are instantiated today, one per JSON escaping convention (#91):
//!
//! | Scanner | Predicate | Used by |
//! |---|---|---|
//! | [`find_json_escape`] | `"` `\` `< 0x20` | yq-style output; the YAML→JSON transcoder |
//! | [`find_jq_escape`] | + DEL (`0x7F`) | jq-style output |
//! | [`find_ascii_escape`] | `"` `\` `< 0x20` `>= 0x7F` | both `--ascii-output` modes |
//!
//! The first two are pure-ASCII predicates, so their returned index is always a
//! UTF-8 char boundary. [`find_ascii_escape`] matches continuation bytes too and
//! carries a caller contract instead — see its docs.
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
//
// The same trick expresses the *other* direction for `find_ascii_escape` below,
// but with the OPERANDS REVERSED: `subs_epu8(0x7F, b) == 0` exactly when
// `b >= 0x7F`. Getting the constant wrong by one (`0x7E`) silently widens the
// predicate to include `~`; getting the order wrong inverts it entirely. Both
// are caught by `exhaustive_bytes_match_scalar`, which is why every scanner
// gets one.

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

// ---- jq escape predicate ---------------------------------------------------
//
// jq escapes `"`, `\`, the C0 controls (`< 0x20`), and DEL (`0x7F`). It does NOT
// escape the C1 block (U+0080..U+009F) — verified against jq-1.7.1, which emits
// those raw in UTF-8 output; see the `escape_del_and_c1` golden case (#91).
//
// That makes this predicate pure ASCII: no byte >= 0x80 can ever match, so the
// returned index is unconditionally a UTF-8 char boundary, exactly as for
// `find_json_escape`. Keep it that way. Matching the C1 block would mean
// flagging its lead byte `0xC2`, which also leads U+00A0..U+00BF (NBSP and the
// Latin-1 punctuation `¡ ¢ £ ° » ¿`) — turning every accented-Latin document
// into a stream of false-positive stops, and making boundary safety a caller
// obligation rather than a property. Never flag the C1 *continuation* bytes
// `0x80..0x9F` directly: they occur inside unrelated multi-byte characters
// (U+2665 is `E2 99 A5`, and `0x99` is in that range), so the scanner would
// return an index mid-character and panic every caller that slices on it.

/// NEON match mask for the jq escape predicate (`"`, `\`, `< 0x20`, `0x7F`).
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
#[inline]
#[target_feature(enable = "neon")]
unsafe fn jq_neon_mask(chunk: uint8x16_t) -> uint8x16_t {
    vorrq_u8(
        vorrq_u8(
            vceqq_u8(chunk, vdupq_n_u8(b'"')),
            vceqq_u8(chunk, vdupq_n_u8(b'\\')),
        ),
        vorrq_u8(
            vcltq_u8(chunk, vdupq_n_u8(0x20)), // unsigned byte < 0x20
            vceqq_u8(chunk, vdupq_n_u8(0x7F)),
        ),
    )
}

/// AVX2 match mask for the jq escape predicate.
#[cfg(all(
    target_arch = "x86_64",
    not(feature = "scalar-yaml"),
    any(test, feature = "std")
))]
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn jq_avx2_mask(chunk: __m256i) -> __m256i {
    _mm256_or_si256(
        _mm256_or_si256(
            _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'"' as i8)),
            _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\\' as i8)),
        ),
        _mm256_or_si256(
            // byte < 0x20, unsigned — see the note above.
            _mm256_cmpeq_epi8(
                _mm256_subs_epu8(chunk, _mm256_set1_epi8(0x1F)),
                _mm256_setzero_si256(),
            ),
            // `cmpeq` is bitwise, so signedness does not apply to the DEL lane.
            _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(0x7Fu8 as i8)),
        ),
    )
}

/// SSE2 match mask for the jq escape predicate.
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
#[inline]
#[target_feature(enable = "sse2")]
unsafe fn jq_sse2_mask(chunk: __m128i) -> __m128i {
    _mm_or_si128(
        _mm_or_si128(
            _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'"' as i8)),
            _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'\\' as i8)),
        ),
        _mm_or_si128(
            // byte < 0x20, unsigned — see the note above.
            _mm_cmpeq_epi8(
                _mm_subs_epu8(chunk, _mm_set1_epi8(0x1F)),
                _mm_setzero_si128(),
            ),
            _mm_cmpeq_epi8(chunk, _mm_set1_epi8(0x7Fu8 as i8)),
        ),
    )
}

define_escape_scanner! {
    /// jq-style JSON string escape scanner.
    mod jq_escape;
    scalar: |b| b == b'"' || b == b'\\' || b < 0x20 || b == 0x7F;
    neon_mask: jq_neon_mask;
    avx2_mask: jq_avx2_mask;
    sse2_mask: jq_sse2_mask;
}

/// Find the next jq-escapable byte in `bytes` at or after `start`.
///
/// Searches for `"`, `\`, a C0 control (`< 0x20`), or DEL (`0x7F`), returning the
/// index of the first match or `bytes.len()` if none is found. This is the
/// hot-path scan for jq-style JSON string output (`EscapeStyle::Jq`).
///
/// Every matched byte is ASCII, so the returned index is always a UTF-8 char
/// boundary and every stop is a byte the escaper genuinely rewrites — there are
/// no false positives to handle.
///
/// `#[inline(always)]` is load-bearing: without it the SIMD path regresses versus
/// scalar on short strings (O3 / #87).
#[inline(always)]
pub fn find_jq_escape(bytes: &[u8], start: usize) -> usize {
    jq_escape::find(bytes, start)
}

// ---- ASCII-output escape predicate -----------------------------------------
//
// `"`, `\`, `< 0x20`, or `>= 0x7F`. The `>= 0x7F` lane subsumes DEL and every
// non-ASCII byte at once, so a single scanner serves both ASCII output modes:
// jq's `--ascii-output` (which escapes DEL as ``) and yq's (which leaves
// DEL raw — a deliberate false-positive stop whose handler re-emits the byte
// verbatim, preserving #262 semantics).
//
// BOUNDARY SAFETY IS CONDITIONAL HERE, unlike every other scanner in this file.
// `>= 0x7F` matches UTF-8 continuation bytes (`0x80..=0xBF`) as well as lead
// bytes. It is sound only because in valid UTF-8 every multi-byte character
// begins with a lead byte >= 0xC2, which also matches — so a left-to-right scan
// that STARTS on a char boundary always stops on the lead byte first and never
// reaches the continuations. The obligation that transfers to callers: after
// handling a stop, advance by the character's full width (`len_utf8()`), never
// by one byte. Advancing by one lands mid-character, and the next call returns a
// continuation-byte index that panics any caller slicing on it.

/// NEON match mask for the ASCII-output escape predicate.
#[cfg(all(
    target_arch = "aarch64",
    not(feature = "broadword-yaml"),
    not(feature = "scalar-yaml")
))]
#[inline]
#[target_feature(enable = "neon")]
unsafe fn ascii_neon_mask(chunk: uint8x16_t) -> uint8x16_t {
    vorrq_u8(
        vorrq_u8(
            vceqq_u8(chunk, vdupq_n_u8(b'"')),
            vceqq_u8(chunk, vdupq_n_u8(b'\\')),
        ),
        vorrq_u8(
            vcltq_u8(chunk, vdupq_n_u8(0x20)), // unsigned byte <  0x20
            vcgeq_u8(chunk, vdupq_n_u8(0x7F)), // unsigned byte >= 0x7F
        ),
    )
}

/// AVX2 match mask for the ASCII-output escape predicate.
#[cfg(all(
    target_arch = "x86_64",
    not(feature = "scalar-yaml"),
    any(test, feature = "std")
))]
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn ascii_avx2_mask(chunk: __m256i) -> __m256i {
    _mm256_or_si256(
        _mm256_or_si256(
            _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'"' as i8)),
            _mm256_cmpeq_epi8(chunk, _mm256_set1_epi8(b'\\' as i8)),
        ),
        _mm256_or_si256(
            // byte < 0x20, unsigned.
            _mm256_cmpeq_epi8(
                _mm256_subs_epu8(chunk, _mm256_set1_epi8(0x1F)),
                _mm256_setzero_si256(),
            ),
            // byte >= 0x7F, unsigned — OPERANDS REVERSED, and 0x7F not 0x7E.
            _mm256_cmpeq_epi8(
                _mm256_subs_epu8(_mm256_set1_epi8(0x7Fu8 as i8), chunk),
                _mm256_setzero_si256(),
            ),
        ),
    )
}

/// SSE2 match mask for the ASCII-output escape predicate.
#[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
#[inline]
#[target_feature(enable = "sse2")]
unsafe fn ascii_sse2_mask(chunk: __m128i) -> __m128i {
    _mm_or_si128(
        _mm_or_si128(
            _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'"' as i8)),
            _mm_cmpeq_epi8(chunk, _mm_set1_epi8(b'\\' as i8)),
        ),
        _mm_or_si128(
            // byte < 0x20, unsigned.
            _mm_cmpeq_epi8(
                _mm_subs_epu8(chunk, _mm_set1_epi8(0x1F)),
                _mm_setzero_si128(),
            ),
            // byte >= 0x7F, unsigned — OPERANDS REVERSED, and 0x7F not 0x7E.
            _mm_cmpeq_epi8(
                _mm_subs_epu8(_mm_set1_epi8(0x7Fu8 as i8), chunk),
                _mm_setzero_si128(),
            ),
        ),
    )
}

define_escape_scanner! {
    /// ASCII-output JSON string escape scanner.
    mod ascii_escape;
    scalar: |b| b == b'"' || b == b'\\' || b < 0x20 || b >= 0x7F;
    neon_mask: ascii_neon_mask;
    avx2_mask: ascii_avx2_mask;
    sse2_mask: ascii_sse2_mask;
}

/// Find the next byte requiring attention in ASCII-only JSON output, at or after
/// `start`.
///
/// Searches for `"`, `\`, a C0 control (`< 0x20`), or any byte `>= 0x7F`,
/// returning the index of the first match or `bytes.len()` if none is found.
/// Serves both ASCII output modes; yq's leaves DEL raw, so a stop does not
/// always mean a rewrite.
///
/// # Caller contract
///
/// `start` must be a UTF-8 char boundary, and callers must advance past a stop by
/// the full character width (`len_utf8()`), not one byte — see the boundary note
/// on the predicate above. Under that contract the returned index is always a
/// char boundary.
///
/// `#[inline(always)]` is load-bearing: without it the SIMD path regresses versus
/// scalar on short strings (O3 / #87).
#[inline(always)]
pub fn find_ascii_escape(bytes: &[u8], start: usize) -> usize {
    ascii_escape::find(bytes, start)
}

/// Generate the full differential battery for one escape scanner.
///
/// Every scanner gets the same checks — exhaustive byte × offset parity against
/// an independent scalar reference, chunk-boundary straddles, the char-boundary
/// contract, and per-kernel tests that call SSE2/AVX2/NEON directly rather than
/// whatever the dispatcher happens to pick. Sharing them by macro keeps the three
/// scanners in lockstep; hand-copying was how the second one would quietly get a
/// thinner suite than the first.
///
/// `reference` is written out independently at each invocation and deliberately
/// NOT derived from the `define_escape_scanner!` predicate: the two must be able
/// to disagree, or a bug in the shared machinery could mask itself.
#[cfg(test)]
macro_rules! escape_scanner_tests {
    (
        mod $name:ident;
        scanner: $scanner:ident;
        find: $find:ident;
        reference: |$b:ident| $pred:expr;
    ) => {
        mod $name {
            use super::UNICODE_CORPUS;
            use crate::util::simd::escape::{$find, $scanner};

            /// Independent scalar reference for this scanner's predicate.
            fn reference(bytes: &[u8], start: usize) -> usize {
                for (i, &$b) in bytes[start..].iter().enumerate() {
                    if $pred {
                        return start + i;
                    }
                }
                bytes.len()
            }

            #[test]
            fn basic_matches() {
                assert_eq!($find(b"hello\"world", 0), 5);
                assert_eq!($find(b"hello\\world", 0), 5);
                assert_eq!($find(b"hello\nworld", 0), 5);
                assert_eq!($find(b"hello\tworld", 0), 5);
                assert_eq!($find(b"hello\x00world", 0), 5);
            }

            #[test]
            fn none_found_returns_len() {
                let input = b"hello world";
                assert_eq!($find(input, 0), input.len());
            }

            #[test]
            fn empty_input() {
                assert_eq!($find(b"", 0), 0);
            }

            #[test]
            fn start_past_end() {
                let input = b"hello";
                assert_eq!($find(input, 10), input.len());
            }

            #[test]
            fn with_offset() {
                let input = b"abc\"def\"ghi";
                assert_eq!($find(input, 0), 3);
                assert_eq!($find(input, 4), 7);
            }

            #[test]
            fn long_string_simd_path() {
                let mut input = vec![b'a'; 100];
                input[50] = b'"';
                assert_eq!($find(&input, 0), 50);
            }

            /// Straddle every chunk edge the kernels have: the NEON/SSE2 16-byte
            /// loop, the AVX2 32-byte loop, its 16-byte tail, and the scalar
            /// remainder past both.
            #[test]
            fn match_at_every_chunk_boundary() {
                for &(size, pos) in &[
                    (32usize, 16usize),
                    (64, 32),
                    (32, 15),
                    (56, 32), // AVX2 32-byte loop -> 16-byte SSE2 tail
                    (56, 48), // that tail -> scalar remainder
                    (40, 39), // last byte of the scalar remainder
                ] {
                    let mut input = vec![b'a'; size];
                    input[pos] = b'"';
                    assert_eq!($find(&input, 0), pos, "size {size}, pos {pos}");
                }
            }

            /// Every byte value at every alignment, against the scalar reference.
            ///
            /// The test that would have caught #230: the x86 path once used a
            /// *signed* compare for `byte < 0x20`, so bytes >= 0x80 read as
            /// negative and were reported as control characters. Sweeping the
            /// offset matters as much as the value — SIMD engages only on full
            /// 16/32-byte chunks, so a byte is checked by SIMD at some positions
            /// and by the scalar tail at others. It is also what catches a
            /// mis-signed or off-by-one `>= 0x7F` lane.
            #[test]
            fn exhaustive_bytes_match_scalar() {
                for byte in 0u8..=255 {
                    for pos in 0..40usize {
                        let mut input = vec![b'a'; 40];
                        input[pos] = byte;
                        assert_eq!(
                            reference(&input, 0),
                            $find(&input, 0),
                            "mismatch for byte {byte:#04x} at offset {pos}"
                        );
                    }
                }
            }

            #[test]
            fn simd_matches_scalar_including_utf8() {
                let mut cases: Vec<&[u8]> = vec![
                    b"",
                    b"\"",
                    b"\\",
                    b"\n",
                    b"\t",
                    b"\r",
                    b"\x00",
                    b"\x7f",
                    b"no special chars here",
                    b"quote at end\"",
                    b"\"quote at start",
                    b"has\\backslash",
                    b"has both \" and \\ chars",
                    b"control\x01char",
                    &[b'x'; 100],
                ];
                // Non-ASCII: `byte < 0x20` must be an UNSIGNED test; a signed one
                // flags every byte >= 0x80 (#150/#230).
                cases.extend(UNICODE_CORPUS.iter().map(|s| s.as_bytes()));
                for input in cases {
                    for start in 0..=input.len() {
                        assert_eq!(
                            reference(input, start),
                            $find(input, start),
                            "mismatch for {input:?} at start {start}"
                        );
                    }
                }
            }

            /// The contract every caller relies on when it slices `&s[i..pos]`.
            ///
            /// Walked exactly as a caller must: start on a char boundary, and
            /// advance past a stop by the character's full width. For the ASCII
            /// scanner that advance is load-bearing — its predicate matches
            /// continuation bytes, so a one-byte advance would desync the walk
            /// (pinned by `advancing_one_byte_desyncs_the_ascii_scanner`).
            #[test]
            fn stop_is_a_char_boundary_and_a_real_match() {
                for s in UNICODE_CORPUS {
                    let bytes = s.as_bytes();
                    let mut i = 0;
                    while i < bytes.len() {
                        assert!(s.is_char_boundary(i), "walk desynced at {i} in {s:?}");
                        let pos = $find(bytes, i);
                        assert!(pos >= i && pos <= bytes.len(), "out of range for {s:?}");
                        assert!(
                            s.is_char_boundary(pos),
                            "index {pos} is mid-character in {s:?}"
                        );
                        // The slice callers actually take; panics if the above lies.
                        let _ = &s[i..pos];
                        if pos == bytes.len() {
                            break;
                        }
                        let $b = bytes[pos];
                        assert!(
                            !(0x80..=0xBF).contains(&$b),
                            "index {pos} for {s:?} points at continuation byte {:#04x}",
                            $b
                        );
                        assert!($pred, "index {pos} for {s:?} points at a safe byte");
                        i = pos + s[pos..].chars().next().unwrap().len_utf8();
                    }
                }
            }

            /// The module-level `scalar` fallback is only reached through the SIMD
            /// kernels' short-string path (arch-dependent) or a scalar-only build,
            /// so exercise it directly on every target for parity and coverage.
            #[test]
            fn scalar_fallback_matches_reference() {
                let mut cases: Vec<&[u8]> = vec![
                    b"",
                    b"\"",
                    b"\\",
                    b"\n",
                    b"\x7f",
                    b"plain text",
                    b"quote\"in\\the\tmiddle",
                    &[b'x'; 40],
                ];
                cases.extend(UNICODE_CORPUS.iter().map(|s| s.as_bytes()));
                for input in cases {
                    for start in 0..=input.len() {
                        assert_eq!(
                            reference(input, start),
                            $scanner::scalar(input, start),
                            "scalar mismatch for {input:?} at start {start}"
                        );
                    }
                }
            }

            // ----------------------------------------------------------------
            // Per-kernel differential tests (#193): exercise each x86 kernel
            // directly, regardless of what the dispatcher would pick, so the SSE2
            // kernel is tested on AVX2 hardware and both are checked against the
            // scalar reference across the byte range (the tests that would have
            // caught #150/#230). Gated off under `scalar-yaml`, which cfg-excludes
            // the very kernels these tests call.
            // ----------------------------------------------------------------
            #[cfg(all(target_arch = "x86_64", not(feature = "scalar-yaml")))]
            mod x86_kernels {
                use super::{reference, $scanner, UNICODE_CORPUS};

                /// AVX2 detection guard; emits a visible `SKIPPED` line when
                /// unavailable so a fully-skipped kernel does not read as green.
                fn has_avx2() -> bool {
                    crate::util::simd::note_simd_skip_unless(
                        is_x86_feature_detected!("avx2"),
                        "avx2",
                    )
                }

                fn sse2_index(input: &[u8], start: usize) -> usize {
                    // SAFETY: SSE2 is the x86_64 baseline.
                    unsafe { $scanner::sse2(input, start) }.map_or(input.len(), |off| start + off)
                }

                fn avx2_index(input: &[u8], start: usize) -> usize {
                    // SAFETY: gated on runtime AVX2 detection by callers.
                    unsafe { $scanner::avx2(input, start) }.map_or(input.len(), |off| start + off)
                }

                #[test]
                fn kernels_match_scalar_non_ascii() {
                    let run_avx2 = has_avx2();
                    for input in UNICODE_CORPUS.iter().map(|s| s.as_bytes()) {
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
                    // Two 16-byte SSE2 chunks plus the scalar tail; bytes >= 0x80
                    // are the ones the signed compare misclassified (#150/#230).
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
                    // 56-byte buffer: one 32-byte AVX2 chunk, one 16-byte SSE2
                    // tail, and an 8-byte scalar tail — every internal path of the
                    // kernel. Skipped (visibly, via has_avx2) without AVX2.
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
                    // The `start >= len` guard in each kernel is never reached
                    // through find() (which guards start earlier); exercise it.
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
                    // Cover both dispatch branches: the SSE2 baseline (always
                    // reachable) and the AVX2 tier (when detected). find() only
                    // ever picks AVX2 on this runner, so the SSE2-baseline branch
                    // needs an explicit call.
                    let input = b"a long enough plain string \"with\" some \\ escapes inside";
                    let want = reference(input, 0);
                    assert_eq!($scanner::dispatch(input, 0, false), want);
                    if has_avx2() {
                        assert_eq!($scanner::dispatch(input, 0, true), want);
                    }
                }
            }

            // ----------------------------------------------------------------
            // NEON kernel exhaustive check (#186): every byte at positions
            // covering the 16-byte NEON path, the chunk boundary, and the
            // <16-byte scalar remainder.
            // ----------------------------------------------------------------
            #[cfg(all(
                target_arch = "aarch64",
                not(feature = "broadword-yaml"),
                not(feature = "scalar-yaml")
            ))]
            mod neon_kernel {
                use super::{reference, $scanner};

                #[test]
                fn exhaustive_bytes_match_scalar() {
                    for b in 0u8..=255 {
                        for &pos in &[0usize, 1, 7, 15, 16, 17, 31, 32, 33, 47] {
                            let mut input = vec![b'A'; 48];
                            input[pos] = b;
                            for &start in &[0usize, 3, 16] {
                                assert_eq!(
                                    reference(&input, start),
                                    $scanner::neon(&input, start),
                                    "NEON mismatch for byte 0x{b:02x} at pos {pos}, start {start}"
                                );
                            }
                        }
                    }
                }

                #[test]
                fn neon_returns_len_when_start_past_end() {
                    // The kernel's own `start >= len` guard, not reached via find().
                    assert_eq!($scanner::neon(b"abc", 9), 3);
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Multibyte corpus shared by every scanner's battery.
    ///
    /// Sized to hit the SSE2/NEON 16-byte loop, the AVX2 32-byte loop, its
    /// 16-byte tail, and the scalar remainders, with multibyte characters landing
    /// both inside chunks and straddling their edges. `love ♥ and peace ☮` is the
    /// original #230 repro. The last three carry DEL, the C1 block, and the
    /// Latin-1 punctuation just above it — the bytes that separate the three
    /// predicates from one another.
    const UNICODE_CORPUS: &[&str] = &[
        "café au lait, s'il vous plaît",
        "love ♥ and peace ☮",
        "日本語のテキストはここにあります",
        "emoji 😁 in a string long enough for a full chunk",
        "aaaaaaaaaaaaaaaa♥",                  // multibyte at the 16-byte boundary
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa😁", // at the 32-byte boundary
        "mixed \"quote\" and café and \\ backslash",
        "日本語 with a \n control in the middle of CJK 文字",
        "mixed ♥ with \"quotes\" and \\slashes\\ and \nnewlines",
        "del \u{7f} and c1 \u{85} and \u{9f} in one string",
        "nbsp\u{a0}and\u{a1}punct\u{bf}just above the c1 block",
        "\u{7f}\u{85}\u{a0}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{9f}",
    ];

    escape_scanner_tests! {
        mod json_escape_tests;
        scanner: json_escape;
        find: find_json_escape;
        reference: |b| b == b'"' || b == b'\\' || b < 0x20;
    }

    escape_scanner_tests! {
        mod jq_escape_tests;
        scanner: jq_escape;
        find: find_jq_escape;
        reference: |b| b == b'"' || b == b'\\' || b < 0x20 || b == 0x7F;
    }

    escape_scanner_tests! {
        mod ascii_escape_tests;
        scanner: ascii_escape;
        find: find_ascii_escape;
        reference: |b| b == b'"' || b == b'\\' || b < 0x20 || b >= 0x7F;
    }

    /// The three predicates must actually differ, or the batteries above are
    /// testing one scanner three times.
    #[test]
    fn predicates_are_distinct() {
        // DEL: jq and ASCII escape it, plain JSON does not.
        assert_eq!(find_json_escape(b"a\x7fb", 0), 3);
        assert_eq!(find_jq_escape(b"a\x7fb", 0), 1);
        assert_eq!(find_ascii_escape(b"a\x7fb", 0), 1);

        // Non-ASCII: only the ASCII scanner stops.
        let e_acute = "aéb".as_bytes();
        assert_eq!(find_json_escape(e_acute, 0), e_acute.len());
        assert_eq!(find_jq_escape(e_acute, 0), e_acute.len());
        assert_eq!(find_ascii_escape(e_acute, 0), 1);
    }

    /// `find_jq_escape` is pure ASCII by construction — no byte >= 0x80 may ever
    /// match. That is what keeps its char-boundary safety unconditional (unlike
    /// `find_ascii_escape`) and what a future "escape the C1 block" change would
    /// silently break by introducing a `0xC2` lane.
    #[test]
    fn jq_scanner_never_matches_a_high_byte() {
        for byte in 0x80u8..=0xFF {
            let input = [b'a', byte, b'b'];
            assert_eq!(
                find_jq_escape(&input, 0),
                3,
                "jq scanner stopped on high byte {byte:#04x}"
            );
        }
    }

    /// Pins the caller contract on `find_ascii_escape` as a fact, not folklore:
    /// advancing one byte past a multibyte stop DOES land on a continuation byte.
    /// If this test ever fails, the contract has been weakened and every
    /// `i += 1` caller became safe by accident — re-derive it before relaxing.
    #[test]
    fn advancing_one_byte_desyncs_the_ascii_scanner() {
        let s = "aé";
        let bytes = s.as_bytes();
        let first = find_ascii_escape(bytes, 0);
        assert_eq!(first, 1, "expected to stop on the é lead byte");
        assert!(s.is_char_boundary(first));

        // The wrong advance: +1 instead of len_utf8().
        let next = find_ascii_escape(bytes, first + 1);
        assert_eq!(next, 2, "expected to stop on the é continuation byte");
        assert!(
            !s.is_char_boundary(next),
            "continuation-byte index must NOT be a char boundary"
        );
    }
}
