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
}
