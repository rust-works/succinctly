#![allow(unsafe_code)] // SIMD intrinsics for the 64-byte specials classifier
//! Block classification of the bytes a canonical JSON string scan must decide on.
//!
//! The canonical-echo gate (`canonical_compact_jq_span_end`, `src/json/light.rs`,
//! #2608) walks every string between its quotes. A byte inside a string is *plain*
//! unless it is one of the **specials**: `"` (ends the string), `\` (starts an
//! escape), a control byte `< 0x20` or DEL `0x7F` (never legal unescaped in jq's
//! own output). Everything else, including every byte `>= 0x80`, is literal content.
//!
//! A SIMD skip over the plain bytes of *each* string measured as a loss on both
//! pinned boxes (#3168): the vector scan's entry cost is paid once per string and
//! once per escape, so it wins on 64-byte strings and loses on short keys and
//! escape-dense text. [`SpecialMask`] takes the entry out. It classifies one
//! 64-byte block into a `u64` bitmap, once, and answers every "next special at or
//! after `from`" query inside that block with a shift and a `trailing_zeros`.
//! A short string costs a bit scan of a mask a neighbouring string already paid
//! for; the block is classified again only when a query leaves it (#3340).
//!
//! The classification is **lazy and forward-only** by design. The caller's text
//! runs to the end of the document even when the value being checked is a small
//! sub-value, and a document of numbers has no strings at all, so an eager pass
//! over the whole text would be charged to inputs it cannot help (the O6/#1514
//! precheck lesson). A block is classified when a string asks for it, never
//! before.

// The kernels are gated on the target architecture alone. `escape.rs` also
// switches its kernels off under `scalar-yaml` / `broadword-yaml`, but those are
// YAML-parser selectors; this module serves the JSON gate, whose own SIMD
// (`json/simd`) ignores them, so a YAML measurement build keeps the JSON path
// it ships with.
#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Whether `b` is a special: the predicate every kernel below reproduces.
#[inline(always)]
const fn is_special(b: u8) -> bool {
    b == b'"' || b == b'\\' || b < 0x20 || b == 0x7F
}

/// Scalar reference for [`classify_64`]: bit `k` of the result is
/// `is_special(block[k])`. The portable fallback, and the oracle the SIMD
/// kernels are tested against.
#[inline(always)]
#[allow(dead_code)] // fallback/oracle; the SIMD kernel is used where one exists
pub(crate) fn classify_64_scalar(block: &[u8; 64]) -> u64 {
    let mut mask = 0u64;
    for (k, &b) in block.iter().enumerate() {
        mask |= u64::from(is_special(b)) << k;
    }
    mask
}

/// aarch64 NEON: four 16-byte compares reduced to one 64-bit mask. One
/// vector-to-GPR transfer per 64 bytes (the `vpaddq` reduction), not one per
/// 16 as with the nibble mask `escape.rs` uses for a single chunk (#2963).
#[cfg(target_arch = "aarch64")]
#[inline]
#[target_feature(enable = "neon")]
unsafe fn classify_64_neon(block: &[u8; 64]) -> u64 {
    #[inline(always)]
    unsafe fn special(v: uint8x16_t) -> uint8x16_t {
        vorrq_u8(
            vorrq_u8(
                vceqq_u8(v, vdupq_n_u8(b'"')),
                vceqq_u8(v, vdupq_n_u8(b'\\')),
            ),
            vorrq_u8(vcltq_u8(v, vdupq_n_u8(0x20)), vceqq_u8(v, vdupq_n_u8(0x7F))),
        )
    }
    const WEIGHTS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
    let w = vld1q_u8(WEIGHTS.as_ptr());
    let p = block.as_ptr();
    let m0 = vandq_u8(special(vld1q_u8(p)), w);
    let m1 = vandq_u8(special(vld1q_u8(p.add(16))), w);
    let m2 = vandq_u8(special(vld1q_u8(p.add(32))), w);
    let m3 = vandq_u8(special(vld1q_u8(p.add(48))), w);
    // Each pairwise add folds eight lanes' weights into one byte: after three
    // rounds byte `i` of the low half is the 8-bit mask of bytes `8i..8i+8`.
    let s01 = vpaddq_u8(m0, m1);
    let s23 = vpaddq_u8(m2, m3);
    let s = vpaddq_u8(vpaddq_u8(s01, s23), vdupq_n_u8(0));
    vget_lane_u64::<0>(vreinterpret_u64_u8(vget_low_u8(s)))
}

/// x86_64 SSE2 (the baseline, so no runtime dispatch and `no_std`-safe): four
/// 16-byte compares and `movemask`s combined into one 64-bit mask.
#[cfg(target_arch = "x86_64")]
#[inline]
#[target_feature(enable = "sse2")]
unsafe fn classify_64_sse2(block: &[u8; 64]) -> u64 {
    #[inline(always)]
    unsafe fn special(v: __m128i) -> u32 {
        let any = _mm_or_si128(
            _mm_or_si128(
                _mm_cmpeq_epi8(v, _mm_set1_epi8(b'"' as i8)),
                _mm_cmpeq_epi8(v, _mm_set1_epi8(b'\\' as i8)),
            ),
            _mm_or_si128(
                // byte < 0x20, unsigned: saturating-subtract 0x1F is zero.
                _mm_cmpeq_epi8(_mm_subs_epu8(v, _mm_set1_epi8(0x1F)), _mm_setzero_si128()),
                _mm_cmpeq_epi8(v, _mm_set1_epi8(0x7F)),
            ),
        );
        _mm_movemask_epi8(any) as u32
    }
    let p = block.as_ptr().cast::<__m128i>();
    let m0 = u64::from(special(_mm_loadu_si128(p)));
    let m1 = u64::from(special(_mm_loadu_si128(p.add(1))));
    let m2 = u64::from(special(_mm_loadu_si128(p.add(2))));
    let m3 = u64::from(special(_mm_loadu_si128(p.add(3))));
    m0 | (m1 << 16) | (m2 << 32) | (m3 << 48)
}

/// Bit `k` of the result is set iff `block[k]` is a special (`"`, `\`, `< 0x20`
/// or DEL).
///
/// On an architecture with no kernel above (riscv, wasm, ...) this is the
/// 64-iteration scalar loop, which has not been measured against the byte walk
/// it replaces: a short string there classifies a whole block where the old
/// loop stopped at its closing quote.
#[inline(always)]
pub(crate) fn classify_64(block: &[u8; 64]) -> u64 {
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is mandatory on aarch64.
        unsafe { classify_64_neon(block) }
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: SSE2 is the x86_64 baseline.
        unsafe { classify_64_sse2(block) }
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        classify_64_scalar(block)
    }
}

/// A forward-only, lazily filled cache of one 64-byte specials block.
///
/// `mask` bit `k` is set iff `bytes[base + k]` is a special; `mask == 0` with
/// `len == 0` is the empty cache. Queries must be made against the same `bytes`
/// for the cache's whole life, and may move backwards only at the cost of a
/// re-classification (the scan never does).
pub(crate) struct SpecialMask {
    base: usize,
    /// Number of real bytes the cached block covers: 64, or fewer for the tail
    /// of `bytes`. Bits at or above `len` are always clear.
    len: usize,
    mask: u64,
}

impl SpecialMask {
    /// An empty cache: the first query classifies.
    #[inline(always)]
    pub(crate) const fn new() -> Self {
        Self {
            base: 0,
            len: 0,
            mask: 0,
        }
    }

    /// Classifies the block that starts at `at` and caches it. A full block
    /// goes through the SIMD kernel; the final partial block (fewer than 64
    /// bytes left) is classified byte by byte so nothing past `bytes.len()` is
    /// read.
    #[inline(always)]
    fn fill(&mut self, bytes: &[u8], at: usize) {
        self.base = at;
        match bytes
            .get(at..at + 64)
            .and_then(|s| <&[u8; 64]>::try_from(s).ok())
        {
            Some(block) => {
                self.len = 64;
                self.mask = classify_64(block);
            }
            None => self.fill_tail(bytes, at),
        }
    }

    /// The partial final block (fewer than 64 bytes left), classified byte by
    /// byte. It runs at most once per cache, i.e. once per
    /// `canonical_compact_jq_span_end` call, and only for a string that starts
    /// in the buffer's last 63 bytes -- but a *small* value near the end of a
    /// document, or a document under 64 bytes, always does, so it is not
    /// `#[cold]`. Out of line only to keep `fill`'s hot path small.
    #[inline(never)]
    fn fill_tail(&mut self, bytes: &[u8], at: usize) {
        let tail = bytes.get(at..).unwrap_or(&[]);
        self.len = tail.len();
        self.mask = tail
            .iter()
            .enumerate()
            .fold(0u64, |m, (k, &b)| m | (u64::from(is_special(b)) << k));
    }

    /// The index of the first special at or after `from`, or `None` if there is
    /// none before the end of `bytes`.
    #[inline(always)]
    pub(crate) fn next_special(&mut self, bytes: &[u8], from: usize) -> Option<usize> {
        // A query inside the cached block reuses it; anything else (before the
        // block, past it, or the first query) classifies a block *starting at
        // `from`*, so a short string begins a block exactly where it needs one.
        if from < self.base || from >= self.base + self.len {
            if from >= bytes.len() {
                return None;
            }
            self.fill(bytes, from);
        }
        loop {
            let rest = self.mask >> (from.max(self.base) - self.base);
            if rest != 0 {
                return Some(from.max(self.base) + rest.trailing_zeros() as usize);
            }
            // No special in what is left of this block: the next one is
            // contiguous with it. `len < 64` means the tail, so we are done.
            if self.len < 64 {
                return None;
            }
            let next = self.base + 64;
            if next >= bytes.len() {
                return None;
            }
            self.fill(bytes, next);
            // `from` is now at or before the new block's base.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift so the sweeps are reproducible without a dependency.
    fn rng(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    /// An oracle written independently of `is_special` and of every kernel, so
    /// the scalar fallback (the only arm on some targets) is checked against
    /// something other than itself.
    fn oracle_mask(block: &[u8; 64]) -> u64 {
        let mut mask = 0u64;
        for (k, b) in block.iter().enumerate() {
            let special = *b == 0x22 || *b == 0x5C || *b <= 0x1F || *b == 0x7F;
            if special {
                mask |= 1 << k;
            }
        }
        mask
    }

    fn naive_next(bytes: &[u8], from: usize) -> Option<usize> {
        (from..bytes.len()).find(|&i| is_special(bytes[i]))
    }

    #[test]
    fn predicate_is_exactly_quote_backslash_control_and_del() {
        for b in 0u16..=255 {
            let b = b as u8;
            let expect = matches!(b, b'"' | b'\\' | 0x7F) || b < 0x20;
            assert_eq!(is_special(b), expect, "byte {b:#04x}");
        }
    }

    /// Every byte value in every lane, against an independent oracle: a lane
    /// permutation bug (the NEON weight reduction, the SSE2 shifts) shows up as
    /// a single wrong bit position here.
    #[test]
    fn classify_64_matches_scalar_for_every_byte_in_every_lane() {
        for lane in 0..64 {
            for b in 0u16..=255 {
                let mut block = [b'a'; 64];
                block[lane] = b as u8;
                let expect = oracle_mask(&block);
                assert_eq!(classify_64(&block), expect, "lane {lane} byte {b:#04x}");
                assert_eq!(
                    classify_64_scalar(&block),
                    expect,
                    "scalar lane {lane} byte {b:#04x}"
                );
            }
        }
    }

    #[test]
    fn classify_64_matches_scalar_on_random_blocks() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..20_000 {
            let mut block = [0u8; 64];
            // Mix: fully random, and mostly-plain with a few specials, so
            // both dense and sparse masks are exercised.
            let sparse = rng(&mut seed) & 1 == 0;
            for b in &mut block {
                let r = rng(&mut seed);
                *b = if sparse && r % 9 != 0 {
                    b'a' + (r >> 8) as u8 % 26
                } else {
                    (r >> 16) as u8
                };
            }
            let expect = oracle_mask(&block);
            assert_eq!(classify_64(&block), expect);
            assert_eq!(classify_64_scalar(&block), expect);
        }
    }

    #[test]
    fn next_special_matches_a_naive_scan_for_every_start_and_length() {
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        for len in [0usize, 1, 7, 63, 64, 65, 127, 128, 129, 200] {
            for density in [0u64, 1, 5, 40] {
                let bytes: Vec<u8> = (0..len)
                    .map(|_| {
                        let r = rng(&mut seed);
                        if density != 0 && r % (density + 1) == 0 {
                            [b'"', b'\\', 0x00, 0x1F, 0x7F][(r >> 32) as usize % 5]
                        } else {
                            b'a' + (r >> 8) as u8 % 26
                        }
                    })
                    .collect();
                // One cache, queried at ascending starts like the real scan,
                // and a fresh cache per start (the cold path).
                let mut warm = SpecialMask::new();
                for from in 0..=len + 2 {
                    let expect = naive_next(&bytes, from);
                    assert_eq!(
                        warm.next_special(&bytes, from),
                        expect,
                        "warm len {len} density {density} from {from}"
                    );
                    assert_eq!(
                        SpecialMask::new().next_special(&bytes, from),
                        expect,
                        "cold len {len} density {density} from {from}"
                    );
                }
            }
        }
    }

    /// A special sitting exactly on, just before and just after each block
    /// boundary, found from a start in the previous block: the carry into the
    /// next block, and the last lane of a full block, are the off-by-one
    /// sites.
    #[test]
    fn next_special_finds_a_special_on_every_block_boundary_offset() {
        for at in 0..200usize {
            for len in [at + 1, at + 2, at + 64, 256] {
                if len <= at {
                    continue;
                }
                let mut bytes = vec![b'x'; len];
                bytes[at] = b'\\';
                for from in [
                    0,
                    at.saturating_sub(65),
                    at.saturating_sub(64),
                    at.saturating_sub(1),
                    at,
                ] {
                    let mut m = SpecialMask::new();
                    assert_eq!(
                        m.next_special(&bytes, from),
                        Some(at),
                        "special at {at}, len {len}, from {from}"
                    );
                }
                let mut m = SpecialMask::new();
                assert_eq!(m.next_special(&bytes, at + 1), None, "past it, at {at}");
            }
        }
    }
}
