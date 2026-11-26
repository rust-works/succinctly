#![allow(unsafe_code)] // runtime SIMD feature dispatch for the NUL search
//! The first NUL byte of a buffer (#7437).
//!
//! jq's number parser reads a token as a C string, so every JSON reading route looks for a NUL in
//! each source it reads ([`crate::jq::blank_nul_number_tails`]). Almost no input holds one, so the
//! search is charged to every byte of every ordinary input, and what it costs there is all that
//! matters: the first version, an OR of byte compares reduced every 64 bytes, was ~0.37
//! instructions a byte, 1.4-2.8% of a whole-file `.id`, `length` or `-c .`.
//!
//! A NUL is the smallest byte, so a block holds one exactly when the unsigned minimum of its
//! bytes is zero. The kernels fold a [`BLOCK`] into one vector with an unsigned `min` per vector
//! of bytes (on AVX2 one instruction per 32 bytes, its load folded in) and test the minimum once
//! per block; the block that holds a NUL is then searched a vector at a time, and the last vector
//! a byte at a time.

#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// The bytes folded between two tests of the minimum: a block with no NUL costs one `min` per
/// vector and one test, so the test's cost is spread over this many bytes.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
const BLOCK: usize = 4096;

/// The offset of the first NUL byte in `bytes`, or `None` when it holds none.
#[inline]
pub fn find_nul(bytes: &[u8]) -> Option<usize> {
    let from = nul_free_prefix(bytes);
    bytes[from..]
        .iter()
        .position(|&byte| byte == 0)
        .map(|at| from + at)
}

/// How many bytes at the front of `bytes` the kernel has seen hold no NUL: the first NUL, if any,
/// is in the vector that starts there or in the bytes after it that are fewer than a vector.
#[inline]
fn nul_free_prefix(bytes: &[u8]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if avx2_available() {
            // SAFETY: the CPU implements AVX2, which `avx2_available` just checked.
            return unsafe { nul_free_prefix_avx2(bytes) };
        }
        // SAFETY: SSE2 is part of the x86_64 baseline.
        unsafe { nul_free_prefix_sse2(bytes) }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is mandatory on aarch64.
        unsafe { nul_free_prefix_neon(bytes) }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = bytes;
        0
    }
}

/// Is AVX2 usable here? A runtime check with `std` (one cached load), else whether the build
/// enables it at compile time, as `json::simd::avx2` decides for the index builders.
#[cfg(target_arch = "x86_64")]
#[inline]
fn avx2_available() -> bool {
    #[cfg(any(test, feature = "std"))]
    {
        is_x86_feature_detected!("avx2")
    }
    #[cfg(not(any(test, feature = "std")))]
    {
        cfg!(target_feature = "avx2")
    }
}

/// Folds `$count` vectors from `$p` on into `$acc` with `$min`, one statement each, so that a block
/// is straight-line code with no loop counter of its own.
macro_rules! fold_min {
    ($acc:ident, $p:ident, $min:ident, $load:expr, $width:expr, [$($k:literal)*]) => {
        $( $acc = $min($acc, $load($p.add($k * $width))); )*
    };
}

/// AVX2: 4096 bytes a test, one `vpminub` (with its load) per 32 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn nul_free_prefix_avx2(bytes: &[u8]) -> usize {
    let load = |p: *const u8| _mm256_loadu_si256(p.cast::<__m256i>());
    let zero = _mm256_setzero_si256();
    let n = bytes.len();
    let mut at = 0;
    while at + BLOCK <= n {
        let p = bytes.as_ptr().add(at);
        let mut m = load(p);
        fold_min!(m, p, _mm256_min_epu8, load, 32, [
            1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
            32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60
            61 62 63 64 65 66 67 68 69 70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 85 86 87 88 89
            90 91 92 93 94 95 96 97 98 99 100 101 102 103 104 105 106 107 108 109 110 111 112 113
            114 115 116 117 118 119 120 121 122 123 124 125 126 127
        ]);
        if _mm256_movemask_epi8(_mm256_cmpeq_epi8(m, zero)) != 0 {
            break;
        }
        at += BLOCK;
    }
    while at + 32 <= n {
        let m = load(bytes.as_ptr().add(at));
        if _mm256_movemask_epi8(_mm256_cmpeq_epi8(m, zero)) != 0 {
            return at;
        }
        at += 32;
    }
    at
}

/// SSE2, the x86_64 baseline: 4096 bytes a test, one `pminub` per 16 bytes.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn nul_free_prefix_sse2(bytes: &[u8]) -> usize {
    let load = |p: *const u8| _mm_loadu_si128(p.cast::<__m128i>());
    let zero = _mm_setzero_si128();
    let n = bytes.len();
    let mut at = 0;
    while at + BLOCK <= n {
        let base = bytes.as_ptr().add(at);
        let mut m = load(base);
        // Four runs of 64 vectors: the macro's offsets are per run.
        for run in 0..4 {
            let p = base.add(run * 1024);
            fold_min!(m, p, _mm_min_epu8, load, 16, [
                0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30
                31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58
                59 60 61 62 63
            ]);
        }
        if _mm_movemask_epi8(_mm_cmpeq_epi8(m, zero)) != 0 {
            break;
        }
        at += BLOCK;
    }
    while at + 16 <= n {
        let m = load(bytes.as_ptr().add(at));
        if _mm_movemask_epi8(_mm_cmpeq_epi8(m, zero)) != 0 {
            return at;
        }
        at += 16;
    }
    at
}

/// NEON: 4096 bytes a test, one `umin` per 16 bytes and one `uminv` per block.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn nul_free_prefix_neon(bytes: &[u8]) -> usize {
    let load = |p: *const u8| vld1q_u8(p);
    let n = bytes.len();
    let mut at = 0;
    while at + BLOCK <= n {
        let base = bytes.as_ptr().add(at);
        let mut m = load(base);
        for run in 0..4 {
            let p = base.add(run * 1024);
            fold_min!(m, p, vminq_u8, load, 16, [
                0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30
                31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58
                59 60 61 62 63
            ]);
        }
        if vminvq_u8(m) == 0 {
            break;
        }
        at += BLOCK;
    }
    while at + 16 <= n {
        if vminvq_u8(load(bytes.as_ptr().add(at))) == 0 {
            return at;
        }
        at += 16;
    }
    at
}

/// The smallest and the largest byte of `bytes`, `(u8::MAX, 0)` when there are none (#7437).
///
/// What the reading routes ask of each piece of a text they validate
/// ([`crate::jq::scan_utf8_and_nul`]): a smallest byte of 0 is a NUL, and a largest under 0x80
/// is text that is all ASCII, which is UTF-8 with no further check. Two folds, an unsigned `min`
/// and an unsigned `max`, over every vector of bytes.
#[inline]
pub fn min_max(bytes: &[u8]) -> (u8, u8) {
    #[cfg(target_arch = "x86_64")]
    let (lo, hi, from) = if avx2_available() {
        // SAFETY: the CPU implements AVX2, which `avx2_available` just checked.
        unsafe { min_max_avx2(bytes) }
    } else {
        // SAFETY: SSE2 is part of the x86_64 baseline.
        unsafe { min_max_sse2(bytes) }
    };
    #[cfg(target_arch = "aarch64")]
    // SAFETY: NEON is mandatory on aarch64.
    let (lo, hi, from) = unsafe { min_max_neon(bytes) };
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let (lo, hi, from) = (u8::MAX, 0, 0);
    bytes[from..]
        .iter()
        .fold((lo, hi), |(lo, hi), &byte| (lo.min(byte), hi.max(byte)))
}

/// The smallest and largest lanes of a vector, stored and folded once a call.
#[cfg(target_arch = "x86_64")]
fn lanes_min_max(lo: &[u8], hi: &[u8]) -> (u8, u8) {
    (
        lo.iter().copied().min().unwrap_or(u8::MAX),
        hi.iter().copied().max().unwrap_or(0),
    )
}

/// AVX2: two 32-byte vectors a step, folded into a `vpminub` and a `vpmaxub` chain. Returns the
/// minimum and maximum of the bytes before the third value, which the caller folds the rest into.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn min_max_avx2(bytes: &[u8]) -> (u8, u8, usize) {
    let load = |p: *const u8| _mm256_loadu_si256(p.cast::<__m256i>());
    let n = bytes.len();
    if n < 64 {
        return (u8::MAX, 0, 0);
    }
    let p = bytes.as_ptr();
    let (mut lo, mut hi) = (load(p), load(p));
    let mut at = 0;
    while at + 64 <= n {
        let (a, b) = (load(p.add(at)), load(p.add(at + 32)));
        lo = _mm256_min_epu8(lo, _mm256_min_epu8(a, b));
        hi = _mm256_max_epu8(hi, _mm256_max_epu8(a, b));
        at += 64;
    }
    let (mut l, mut h) = ([0u8; 32], [0u8; 32]);
    _mm256_storeu_si256(l.as_mut_ptr().cast::<__m256i>(), lo);
    _mm256_storeu_si256(h.as_mut_ptr().cast::<__m256i>(), hi);
    let (lo, hi) = lanes_min_max(&l, &h);
    (lo, hi, at)
}

/// SSE2: two 16-byte vectors a step, folded into a `pminub` and a `pmaxub` chain.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn min_max_sse2(bytes: &[u8]) -> (u8, u8, usize) {
    let load = |p: *const u8| _mm_loadu_si128(p.cast::<__m128i>());
    let n = bytes.len();
    if n < 32 {
        return (u8::MAX, 0, 0);
    }
    let p = bytes.as_ptr();
    let (mut lo, mut hi) = (load(p), load(p));
    let mut at = 0;
    while at + 32 <= n {
        let (a, b) = (load(p.add(at)), load(p.add(at + 16)));
        lo = _mm_min_epu8(lo, _mm_min_epu8(a, b));
        hi = _mm_max_epu8(hi, _mm_max_epu8(a, b));
        at += 32;
    }
    let (mut l, mut h) = ([0u8; 16], [0u8; 16]);
    _mm_storeu_si128(l.as_mut_ptr().cast::<__m128i>(), lo);
    _mm_storeu_si128(h.as_mut_ptr().cast::<__m128i>(), hi);
    let (lo, hi) = lanes_min_max(&l, &h);
    (lo, hi, at)
}

/// NEON: four 16-byte vectors a step, each folded by a tree of `umin`s and of `umax`s into one
/// chain of each, so a step waits on one `min` and one `max` of the step before.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn min_max_neon(bytes: &[u8]) -> (u8, u8, usize) {
    let load = |p: *const u8| vld1q_u8(p);
    let len = bytes.len();
    if len < 64 {
        return (u8::MAX, 0, 0);
    }
    let base = bytes.as_ptr();
    let (mut lo, mut hi) = (load(base), load(base));
    let mut at = 0;
    while at + 64 <= len {
        let step = base.add(at);
        let v = [
            load(step),
            load(step.add(16)),
            load(step.add(32)),
            load(step.add(48)),
        ];
        lo = vminq_u8(lo, vminq_u8(vminq_u8(v[0], v[1]), vminq_u8(v[2], v[3])));
        hi = vmaxq_u8(hi, vmaxq_u8(vmaxq_u8(v[0], v[1]), vmaxq_u8(v[2], v[3])));
        at += 64;
    }
    (vminvq_u8(lo), vmaxvq_u8(hi), at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// A kernel: how many bytes at the front hold no NUL.
    type Kernel = fn(&[u8]) -> usize;

    fn reference(bytes: &[u8]) -> Option<usize> {
        bytes.iter().position(|&byte| byte == 0)
    }

    /// Every kernel this CPU can run, each with the same answer as a bytewise search: no NUL, one
    /// NUL at every offset around the block and vector edges, and two NULs.
    #[test]
    fn every_kernel_finds_the_first_nul_as_a_bytewise_search_does_7437() {
        let mut kernels: Vec<(&str, Kernel)> = Vec::new();
        #[cfg(target_arch = "x86_64")]
        {
            kernels.push(("sse2", |b| unsafe { nul_free_prefix_sse2(b) }));
            if crate::util::simd::note_simd_skip_unless(avx2_available(), "avx2") {
                kernels.push(("avx2", |b| unsafe { nul_free_prefix_avx2(b) }));
            }
        }
        #[cfg(target_arch = "aarch64")]
        kernels.push(("neon", |b| unsafe { nul_free_prefix_neon(b) }));
        kernels.push(("dispatch", nul_free_prefix));
        let find = |kernel: Kernel, bytes: &[u8]| {
            let from = kernel(bytes);
            assert!(reference(&bytes[..from]).is_none(), "a NUL before {from}");
            bytes[from..]
                .iter()
                .position(|&b| b == 0)
                .map(|at| from + at)
        };
        let lens = [
            0,
            1,
            15,
            16,
            17,
            31,
            32,
            33,
            4095,
            4096,
            4097,
            8192 + 40,
            3 * 4096 + 17,
        ];
        for (name, kernel) in &kernels {
            for &len in &lens {
                // Bytes 1..=255, never a NUL; the high bytes test the unsigned compare.
                let none: Vec<u8> = (0..len).map(|i| (i % 255 + 1) as u8).collect();
                assert_eq!(find(*kernel, &none), None, "{name} len {len}");
                let mut ats: Vec<usize> = (0..len.min(70)).collect();
                ats.extend((len.saturating_sub(70)..len).filter(|&at| at >= 70));
                for edge in [4095, 4096, 4097, 8191, 8192, 8193] {
                    if edge < len {
                        ats.push(edge);
                    }
                }
                for &at in &ats {
                    let mut one = none.clone();
                    one[at] = 0;
                    assert_eq!(find(*kernel, &one), Some(at), "{name} len {len} at {at}");
                    if at + 5 < len {
                        one[at + 5] = 0;
                        assert_eq!(
                            find(*kernel, &one),
                            Some(at),
                            "{name} two, len {len} at {at}"
                        );
                    }
                }
            }
        }
        assert_eq!(find_nul(&vec![0xFF; 5000]), None);
        assert_eq!(find_nul(b"ab\0c"), Some(2));
    }

    /// Every `min_max` kernel this CPU can run against a bytewise fold, at every length up to a few
    /// steps and over a long buffer, with the extreme bytes at every offset.
    #[test]
    fn every_min_max_kernel_folds_as_a_bytewise_fold_does_7437() {
        type MinMax = fn(&[u8]) -> (u8, u8, usize);
        let mut kernels: Vec<(&str, MinMax)> = Vec::new();
        #[cfg(target_arch = "x86_64")]
        {
            kernels.push(("sse2", |b| unsafe { min_max_sse2(b) }));
            if crate::util::simd::note_simd_skip_unless(avx2_available(), "avx2") {
                kernels.push(("avx2", |b| unsafe { min_max_avx2(b) }));
            }
        }
        #[cfg(target_arch = "aarch64")]
        kernels.push(("neon", |b| unsafe { min_max_neon(b) }));
        let finish = |kernel: MinMax, bytes: &[u8]| {
            let (lo, hi, from) = kernel(bytes);
            bytes[from..]
                .iter()
                .fold((lo, hi), |(lo, hi), &b| (lo.min(b), hi.max(b)))
        };
        let reference = |bytes: &[u8]| {
            bytes
                .iter()
                .fold((u8::MAX, 0), |(lo, hi), &b| (lo.min(b), hi.max(b)))
        };
        let mut lens: Vec<usize> = (0..200).collect();
        lens.extend([4095, 4096, 4097, 131_072, 131_075]);
        for &len in &lens {
            // Bytes 0x20..=0x7E, so a 0 or a byte past 0x7F is the only extreme.
            let base: Vec<u8> = (0..len).map(|i| 0x20 + (i % 95) as u8).collect();
            let mut ats: Vec<usize> = (0..len.min(140)).collect();
            ats.extend(len.saturating_sub(140)..len);
            for (name, kernel) in &kernels {
                assert_eq!(finish(*kernel, &base), reference(&base), "{name} len {len}");
                assert_eq!(min_max(&base), reference(&base), "dispatch len {len}");
                for &at in &ats {
                    for extreme in [0x00, 0x01, 0x80, 0xFF] {
                        let mut one = base.clone();
                        one[at] = extreme;
                        assert_eq!(
                            finish(*kernel, &one),
                            reference(&one),
                            "{name} len {len} at {at} {extreme:#x}"
                        );
                    }
                }
            }
        }
        assert_eq!(min_max(b""), (u8::MAX, 0));
        assert_eq!(min_max(&vec![0x41; 5000]), (0x41, 0x41));
    }
}
