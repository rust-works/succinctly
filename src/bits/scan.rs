#![allow(unsafe_code)] // dispatches to NEON / AVX2 block-popcount kernels
//! Forward word scan for `select` (#40).
//!
//! Every `select1` in this crate ends the same way: starting from some word,
//! walk forward popcounting words until the running total passes the target
//! rank, then finish inside the crossing word with
//! [`select_in_word`](crate::util::select_in_word). Five separate copies of
//! that loop existed before this module; they now share [`scan_select`].
//!
//! # Why vectorise it
//!
//! Measured on the real-workload corpus (`succinctly dev select-stats`), the
//! YAML streaming site's scan lengths are sharply **bimodal**: about half the
//! calls traverse fewer than 4 words, but **99% of all words popcounted happen
//! in scans of 4 or more**, with p90 ≈ 232 words and a maximum near 300. So the
//! common call is trivial and the aggregate cost lives entirely in a long tail.
//!
//! That shape dictates the design below, and it is why this is not the P2.8 /
//! P3 / P5 pattern of optimising a shape real inputs do not contain: the long
//! scans are where the work is, and they are real.
//!
//! # Design: skip whole blocks, pinpoint once
//!
//! Issue #40 sketched a per-iteration in-vector prefix sum plus a comparison
//! mask to locate the crossing word. That work is unnecessary. A scan only
//! needs to *locate* once — at the very end — while every other iteration just
//! needs to know whether the target lies past the current block. So each
//! iteration computes one block **total** and either subtracts it and moves on,
//! or drops into a short scalar scan over that block's ≤ `BLOCK` words to
//! pinpoint. Block totals are much cheaper than per-lane prefix sums.
//!
//! A scalar prologue runs before the block loop, so the ~50% of calls that
//! finish within a couple of words never touch a vector register and pay no
//! setup at all. This is a structural guarantee rather than a tuned threshold
//! constant — the kind of magic number that made P2.8 fragile.

use crate::util::select_in_word;

/// Words consumed per block-loop iteration.
///
/// 8 words = 64 bytes = one cache line, and matches the natural width of both
/// kernels (four NEON `uint64x2_t`, or two AVX2 `__m256i`).
pub const BLOCK: usize = 8;

/// Words scanned scalar-first, before the block loop starts.
///
/// Set to [`BLOCK`] so that any scan short enough to finish inside it runs
/// *exactly* the code the old per-word loop ran — same instructions, same
/// cost. That makes "no regression on short scans" a structural property
/// rather than a tuning claim, which matters here: the measured crossover on
/// Apple M4 Pro sits between 8 and 16 words, and roughly half of all calls
/// fall below it.
///
/// The price is one block's worth of scalar work on long scans, which the
/// micro-benchmark prices at a few percent of a win measured in multiples.
const PROLOGUE: usize = BLOCK;

/// Locate the word holding the `remaining`-th further set bit at or after
/// `start_word`.
///
/// Returns `(word_idx, remaining_in_word)`, where `remaining_in_word` is the
/// rank of the target bit **within** `words[word_idx]` — ready to hand to
/// [`select_in_word`](crate::util::select_in_word). Returns `None` if `words`
/// does not contain enough set bits from `start_word` onward.
///
/// # Examples
///
/// ```
/// use succinctly::bits::scan_select;
///
/// // Two set bits in word 0, three in word 1.
/// let words = [0b101u64, 0b111u64];
///
/// // The 0th further set bit is in word 0, at in-word rank 0.
/// assert_eq!(scan_select(&words, 0, 0), Some((0, 0)));
/// // The 2nd is the first bit of word 1.
/// assert_eq!(scan_select(&words, 0, 2), Some((1, 0)));
/// // There is no 5th.
/// assert_eq!(scan_select(&words, 0, 5), None);
/// ```
#[inline]
pub fn scan_select(words: &[u64], start_word: usize, remaining: usize) -> Option<(usize, usize)> {
    if start_word >= words.len() {
        return None;
    }

    let mut rem = remaining;
    let mut idx = start_word;

    // Prologue: the common short scan never reaches the vector path.
    let prologue_end = (start_word + PROLOGUE).min(words.len());
    while idx < prologue_end {
        let pop = words[idx].count_ones() as usize;
        if pop > rem {
            return Some((idx, rem));
        }
        rem -= pop;
        idx += 1;
    }

    // Block loop: skip whole blocks whose total does not reach the target.
    while idx + BLOCK <= words.len() {
        let block = &words[idx..idx + BLOCK];
        let total = block_popcount(block);
        if total > rem {
            // The target is inside this block; pinpoint scalar.
            return scan_scalar(block, rem).map(|(off, r)| (idx + off, r));
        }
        rem -= total;
        idx += BLOCK;
    }

    // Tail: fewer than BLOCK words left.
    scan_scalar(&words[idx..], rem).map(|(off, r)| (idx + off, r))
}

/// Plain per-word scan over `words`, returning an offset relative to its start.
#[inline]
fn scan_scalar(words: &[u64], remaining: usize) -> Option<(usize, usize)> {
    let mut rem = remaining;
    for (off, &w) in words.iter().enumerate() {
        let pop = w.count_ones() as usize;
        if pop > rem {
            return Some((off, rem));
        }
        rem -= pop;
    }
    None
}

/// Total popcount of exactly [`BLOCK`] words.
///
/// Dispatches to a SIMD kernel where one is available. The scalar body is not
/// a poor relation: LLVM vectorises it well on several targets, and the
/// micro-benchmark (`select_scan_micro`) compares all three so the intrinsics
/// are only kept where they actually win.
#[inline]
fn block_popcount(block: &[u64]) -> usize {
    debug_assert_eq!(block.len(), BLOCK);

    #[cfg(all(target_arch = "aarch64", feature = "std"))]
    {
        // NEON is baseline on aarch64 — no runtime detection needed.
        // SAFETY: `block` has exactly BLOCK == 8 words, which is what the
        // kernel reads.
        return unsafe { block_popcount_neon(block) };
    }

    #[cfg(all(target_arch = "x86_64", any(feature = "std", test)))]
    {
        if has_avx2() {
            // SAFETY: AVX2 availability checked above; `block` has exactly
            // BLOCK == 8 words, which is what the kernel reads.
            return unsafe { block_popcount_avx2(block) };
        }
    }

    #[allow(unreachable_code)]
    block_popcount_portable(block)
}

/// Portable block popcount. Also the correctness oracle for the SIMD kernels.
#[inline]
pub fn block_popcount_portable(block: &[u64]) -> usize {
    block.iter().map(|w| w.count_ones() as usize).sum()
}

/// NEON block popcount over 8 words.
///
/// Per-byte `CNT`, summed across all four vectors while still 8-bit (each lane
/// is at most 8, so four of them cannot exceed 32 and cannot overflow), then a
/// single widening chain and one horizontal add. Keeping the accumulation
/// narrow is what makes this cheaper than popcounting each word separately.
///
/// # Safety
///
/// `block` must contain at least [`BLOCK`] words.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
pub unsafe fn block_popcount_neon(block: &[u64]) -> usize {
    use core::arch::aarch64::*;

    unsafe {
        let ptr = block.as_ptr().cast::<u8>();
        let c0 = vcntq_u8(vld1q_u8(ptr));
        let c1 = vcntq_u8(vld1q_u8(ptr.add(16)));
        let c2 = vcntq_u8(vld1q_u8(ptr.add(32)));
        let c3 = vcntq_u8(vld1q_u8(ptr.add(48)));

        // Max 8 per lane, so summing four stays well inside u8.
        let sum = vaddq_u8(vaddq_u8(c0, c1), vaddq_u8(c2, c3));

        // Widen once, then one horizontal add.
        vaddvq_u16(vpaddlq_u8(sum)) as usize
    }
}

/// Cached AVX2 detection, mirroring the pattern used by `select_in_word`.
#[cfg(all(target_arch = "x86_64", any(feature = "std", test)))]
#[inline]
fn has_avx2() -> bool {
    use core::sync::atomic::{AtomicU8, Ordering};

    // 0 = unknown, 1 = available, 2 = unavailable
    static HAS_AVX2: AtomicU8 = AtomicU8::new(0);

    match HAS_AVX2.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let detected = std::arch::is_x86_feature_detected!("avx2");
            HAS_AVX2.store(u8::from(!detected) + 1, Ordering::Relaxed);
            detected
        }
    }
}

/// AVX2 block popcount over 8 words.
///
/// `vpshufb` as a 16-entry nibble lookup table (the approach #40 sketched),
/// finished with `vpsadbw` against zero. `vpsadbw` sums bytes within each
/// 64-bit lane in one instruction, which replaces the issue's scatter-and-
/// compare step entirely — a block total is all this loop needs.
///
/// # Safety
///
/// Requires AVX2. `block` must contain at least [`BLOCK`] words.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[inline]
pub unsafe fn block_popcount_avx2(block: &[u64]) -> usize {
    use core::arch::x86_64::*;

    unsafe {
        let lut = _mm256_setr_epi8(
            0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4, //
            0, 1, 1, 2, 1, 2, 2, 3, 1, 2, 2, 3, 2, 3, 3, 4,
        );
        let low_mask = _mm256_set1_epi8(0x0F);
        let ptr = block.as_ptr().cast::<__m256i>();

        let mut acc = _mm256_setzero_si256();
        for i in 0..2 {
            let v = _mm256_loadu_si256(ptr.add(i));
            let lo = _mm256_and_si256(v, low_mask);
            let hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), low_mask);
            let counts =
                _mm256_add_epi8(_mm256_shuffle_epi8(lut, lo), _mm256_shuffle_epi8(lut, hi));
            // Per-byte counts are at most 8; two vectors keep the u8 lanes safe.
            acc = _mm256_add_epi8(acc, counts);
        }

        // Sum bytes within each 64-bit lane, then across lanes.
        let sums = _mm256_sad_epu8(acc, _mm256_setzero_si256());
        let mut lanes = [0u64; 4];
        _mm256_storeu_si256(lanes.as_mut_ptr().cast::<__m256i>(), sums);
        (lanes[0] + lanes[1] + lanes[2] + lanes[3]) as usize
    }
}

/// Reference implementation: the per-word loop every call site used before.
///
/// Retained as the benchmark baseline and as the oracle the property tests
/// compare [`scan_select`] against.
#[inline]
pub fn scan_select_scalar(
    words: &[u64],
    start_word: usize,
    remaining: usize,
) -> Option<(usize, usize)> {
    if start_word >= words.len() {
        return None;
    }
    scan_scalar(&words[start_word..], remaining).map(|(off, r)| (start_word + off, r))
}

/// Complete a scan by locating the target bit position.
///
/// Convenience for the call sites: scan to the crossing word, then finish
/// inside it. Returns the absolute bit position, or `None` if there are not
/// enough set bits.
#[inline]
pub fn select_from(words: &[u64], start_word: usize, remaining: usize) -> Option<usize> {
    let (word_idx, rem) = scan_select(words, start_word, remaining)?;
    Some(word_idx * 64 + select_in_word(words[word_idx], rem as u32) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random words (xorshift64*), so failures reproduce.
    fn pseudo_random_words(n: usize, seed: u64) -> Vec<u64> {
        let mut state = seed | 1;
        (0..n)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state.wrapping_mul(0x2545_F491_4F6C_DD1D)
            })
            .collect()
    }

    #[test]
    fn empty_input_has_nothing_to_find() {
        assert_eq!(scan_select(&[], 0, 0), None);
        assert_eq!(scan_select(&[1, 2, 3], 3, 0), None);
        assert_eq!(scan_select(&[1, 2, 3], 99, 0), None);
    }

    #[test]
    fn finds_bits_within_the_prologue() {
        let words = [0b1011u64, 0b110u64];
        assert_eq!(scan_select(&words, 0, 0), Some((0, 0)));
        assert_eq!(scan_select(&words, 0, 1), Some((0, 1)));
        assert_eq!(scan_select(&words, 0, 2), Some((0, 2)));
        // 4th set bit overall is the first of word 1.
        assert_eq!(scan_select(&words, 0, 3), Some((1, 0)));
    }

    #[test]
    fn all_zero_words_never_satisfy_a_request() {
        let words = vec![0u64; BLOCK * 4];
        assert_eq!(scan_select(&words, 0, 0), None);
    }

    #[test]
    fn skips_whole_zero_blocks_to_reach_a_distant_bit() {
        // A single set bit far past several all-zero blocks exercises the
        // block-skipping path, which is the entire point of the kernel.
        let mut words = vec![0u64; BLOCK * 5];
        let last = words.len() - 1;
        words[last] = 1 << 40;
        assert_eq!(scan_select(&words, 0, 0), Some((last, 0)));
        assert_eq!(select_from(&words, 0, 0), Some(last * 64 + 40));
    }

    #[test]
    fn matches_scalar_on_dense_words() {
        // All bits set: word i holds bits [64i, 64i+64).
        let words = vec![u64::MAX; BLOCK * 3 + 5];
        let total = words.len() * 64;
        for k in [0usize, 1, 63, 64, 65, 200, total - 1] {
            assert_eq!(
                scan_select(&words, 0, k),
                scan_select_scalar(&words, 0, k),
                "k={k}"
            );
            assert_eq!(select_from(&words, 0, k), Some(k), "k={k}");
        }
        assert_eq!(scan_select(&words, 0, total), None);
    }

    #[test]
    fn agrees_with_scalar_across_random_inputs_and_start_words() {
        for seed in [1u64, 2, 0xDEAD_BEEF] {
            // Deliberately not a multiple of BLOCK, so the tail path runs.
            let words = pseudo_random_words(BLOCK * 7 + 3, seed);
            let total: usize = words.iter().map(|w| w.count_ones() as usize).sum();

            for start in [0usize, 1, 2, 3, BLOCK, BLOCK + 1, words.len() - 1] {
                let available: usize = words[start..].iter().map(|w| w.count_ones() as usize).sum();
                for k in [0usize, 1, 5, 40, 100, available.saturating_sub(1)] {
                    assert_eq!(
                        scan_select(&words, start, k),
                        scan_select_scalar(&words, start, k),
                        "seed={seed} start={start} k={k}"
                    );
                }
                // One past the end must fail from every start word.
                assert_eq!(scan_select(&words, start, available), None);
            }

            // select_from must enumerate exactly the set bits, in order.
            let expected: Vec<usize> = (0..words.len() * 64)
                .filter(|&b| words[b / 64] >> (b % 64) & 1 == 1)
                .collect();
            assert_eq!(expected.len(), total);
            for (k, &bit) in expected.iter().enumerate() {
                assert_eq!(select_from(&words, 0, k), Some(bit), "seed={seed} k={k}");
            }
        }
    }

    #[test]
    fn block_popcount_kernels_agree_with_portable() {
        for seed in [7u64, 11, 0x5EED] {
            let words = pseudo_random_words(BLOCK, seed);
            let expected = block_popcount_portable(&words);

            assert_eq!(block_popcount(&words), expected, "dispatch, seed={seed}");

            #[cfg(target_arch = "aarch64")]
            {
                // SAFETY: NEON is baseline on aarch64; `words` has BLOCK words.
                let neon = unsafe { block_popcount_neon(&words) };
                assert_eq!(neon, expected, "neon, seed={seed}");
            }

            #[cfg(target_arch = "x86_64")]
            {
                if crate::util::simd::note_simd_skip_unless(has_avx2(), "avx2") {
                    // SAFETY: AVX2 checked; `words` has BLOCK words.
                    let avx2 = unsafe { block_popcount_avx2(&words) };
                    assert_eq!(avx2, expected, "avx2, seed={seed}");
                }
            }
        }
    }

    #[test]
    fn block_popcount_handles_the_extremes() {
        assert_eq!(block_popcount(&[0u64; BLOCK]), 0);
        assert_eq!(block_popcount(&[u64::MAX; BLOCK]), BLOCK * 64);
    }

    #[test]
    fn scan_select_scalar_rejects_out_of_range_start() {
        assert_eq!(scan_select_scalar(&[1u64], 1, 0), None);
        assert_eq!(scan_select_scalar(&[], 0, 0), None);
    }

    #[test]
    fn select_from_returns_none_when_bits_run_out() {
        let words = [0b1u64, 0u64];
        assert_eq!(select_from(&words, 0, 0), Some(0));
        assert_eq!(select_from(&words, 0, 1), None);
    }
}
