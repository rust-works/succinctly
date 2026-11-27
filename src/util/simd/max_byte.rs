#![allow(unsafe_code)] // runtime SIMD feature dispatch for the largest-byte fold
//! The largest byte of a buffer, for [`crate::text::utf8::is_utf8`]'s ASCII skip.
//!
//! A buffer whose largest byte is under 0x80 is ASCII, which is UTF-8 with no further check. The
//! reading routes ask that of every piece of their input, and almost every piece of most inputs is
//! ASCII, so the fold is charged to every byte: one unsigned `max` per vector of bytes. The fold was
//! `min_max` until #7966, whose minimum found the NUL bytes jq's number parser stopped at; a NUL
//! is a parse error of its own now, so only the maximum is left.

#[cfg(target_arch = "aarch64")]
use core::arch::aarch64::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

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

/// The largest byte of `bytes`, 0 for an empty one.
#[inline]
pub fn max_byte(bytes: &[u8]) -> u8 {
    #[cfg(target_arch = "x86_64")]
    let (hi, from) = if avx2_available() {
        // SAFETY: the CPU implements AVX2, which `avx2_available` just checked.
        unsafe { max_avx2(bytes) }
    } else {
        // SAFETY: SSE2 is part of the x86_64 baseline.
        unsafe { max_sse2(bytes) }
    };
    #[cfg(target_arch = "aarch64")]
    // SAFETY: NEON is mandatory on aarch64.
    let (hi, from) = unsafe { max_neon(bytes) };
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let (hi, from) = (0, 0);
    bytes[from..].iter().fold(hi, |hi, &byte| hi.max(byte))
}

/// The largest lane of a vector, stored and folded once a call.
#[cfg(target_arch = "x86_64")]
fn lanes_max(hi: &[u8]) -> u8 {
    hi.iter().copied().max().unwrap_or(0)
}

/// AVX2: two 32-byte vectors a step, folded into one `vpmaxub` chain. Returns the maximum of the
/// bytes before the second value, which the caller folds the rest into.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn max_avx2(bytes: &[u8]) -> (u8, usize) {
    let load = |p: *const u8| _mm256_loadu_si256(p.cast::<__m256i>());
    let n = bytes.len();
    if n < 64 {
        return (0, 0);
    }
    let p = bytes.as_ptr();
    let mut hi = load(p);
    let mut at = 0;
    while at + 64 <= n {
        hi = _mm256_max_epu8(hi, _mm256_max_epu8(load(p.add(at)), load(p.add(at + 32))));
        at += 64;
    }
    let mut h = [0u8; 32];
    _mm256_storeu_si256(h.as_mut_ptr().cast::<__m256i>(), hi);
    (lanes_max(&h), at)
}

/// SSE2: two 16-byte vectors a step, folded into one `pmaxub` chain.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn max_sse2(bytes: &[u8]) -> (u8, usize) {
    let load = |p: *const u8| _mm_loadu_si128(p.cast::<__m128i>());
    let n = bytes.len();
    if n < 32 {
        return (0, 0);
    }
    let p = bytes.as_ptr();
    let mut hi = load(p);
    let mut at = 0;
    while at + 32 <= n {
        hi = _mm_max_epu8(hi, _mm_max_epu8(load(p.add(at)), load(p.add(at + 16))));
        at += 32;
    }
    let mut h = [0u8; 16];
    _mm_storeu_si128(h.as_mut_ptr().cast::<__m128i>(), hi);
    (lanes_max(&h), at)
}

/// NEON: four 16-byte vectors a step, folded by a tree of `umax`s into one chain, so a step waits on
/// one `max` of the step before.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn max_neon(bytes: &[u8]) -> (u8, usize) {
    let load = |p: *const u8| vld1q_u8(p);
    let len = bytes.len();
    if len < 64 {
        return (0, 0);
    }
    let base = bytes.as_ptr();
    let mut hi = load(base);
    let mut at = 0;
    while at + 64 <= len {
        let step = base.add(at);
        let a = vmaxq_u8(load(step), load(step.add(16)));
        let b = vmaxq_u8(load(step.add(32)), load(step.add(48)));
        hi = vmaxq_u8(hi, vmaxq_u8(a, b));
        at += 64;
    }
    (vmaxvq_u8(hi), at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Every kernel this CPU can run against a bytewise fold, at every length up to a few steps and
    /// over a long buffer, with an extreme byte at every offset near either end.
    #[test]
    fn every_max_kernel_folds_as_a_bytewise_fold_does_7966() {
        type Max = fn(&[u8]) -> (u8, usize);
        let mut kernels: Vec<(&str, Max)> = Vec::new();
        #[cfg(target_arch = "x86_64")]
        {
            kernels.push(("sse2", |b| unsafe { max_sse2(b) }));
            if crate::util::simd::note_simd_skip_unless(avx2_available(), "avx2") {
                kernels.push(("avx2", |b| unsafe { max_avx2(b) }));
            }
        }
        #[cfg(target_arch = "aarch64")]
        kernels.push(("neon", |b| unsafe { max_neon(b) }));
        let finish = |kernel: Max, bytes: &[u8]| {
            let (hi, from) = kernel(bytes);
            bytes[from..].iter().fold(hi, |hi, &b| hi.max(b))
        };
        let reference = |bytes: &[u8]| bytes.iter().fold(0, |hi, &b| hi.max(b));
        let mut lens: Vec<usize> = (0..200).collect();
        lens.extend([4095, 4096, 4097, 131_072, 131_075]);
        for &len in &lens {
            // Bytes 0x20..=0x7E, so a byte past 0x7F is the only one that moves the maximum.
            let base: Vec<u8> = (0..len).map(|i| 0x20 + (i % 95) as u8).collect();
            let mut ats: Vec<usize> = (0..len.min(140)).collect();
            ats.extend(len.saturating_sub(140)..len);
            for (name, kernel) in &kernels {
                assert_eq!(finish(*kernel, &base), reference(&base), "{name} len {len}");
                assert_eq!(max_byte(&base), reference(&base), "dispatch len {len}");
                for &at in &ats {
                    for extreme in [0x7F, 0x80, 0xC3, 0xFF] {
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
        assert_eq!(max_byte(b""), 0);
        assert_eq!(max_byte(&vec![0x41; 5000]), 0x41);
    }
}
