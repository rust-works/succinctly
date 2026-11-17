#![allow(unsafe_code)] // runtime SIMD feature dispatch
//! SIMD-accelerated JSON semi-indexing.
//!
//! This module provides vectorized implementations of JSON semi-indexing
//! that process multiple bytes at once using SIMD instructions.
//!
//! The vectorized character classification techniques are influenced by:
//! - Langdale & Lemire, ["Parsing Gigabytes of JSON per Second"](https://arxiv.org/abs/1902.08318) (2019)
//!
//! ## x86_64 Instruction Set Levels
//!
//! - **SSE2** (baseline): 16 bytes/iteration, universal availability
//! - **AVX2** (optimal): 32 bytes/iteration, ~95% availability (2013+)
//!
//! The implementation uses runtime CPU detection to automatically select
//! the best available instruction set.
//!
//! ## ARM
//!
//! On ARM aarch64:
//! - **NEON** (default): 16 bytes/iteration with nibble lookup tables, mandatory on all aarch64
//! - **SVE2** (experimental): Character comparisons with predication
//!   - AWS Graviton 4 (Neoverse V2): 128-bit SVE2 vectors
//!   - **36% slower than NEON** on 128-bit implementations due to expensive predicate-to-bitmask conversion
//!   - May be faster on wider SVE2 implementations (256-bit, 512-bit)
//!   - Enable with `SUCCINCTLY_SVE2=1` environment variable (not recommended);
//!     see `docs/reference/environment-variables.md`
//!
//! ## Cursor Algorithms
//!
//! Both simple and standard cursor algorithms are provided:
//! - [`build_semi_index_simple`]: Simple cursor (3-state, marks structural chars)
//! - [`build_semi_index_standard`]: Standard cursor (4-state, marks value starts)

#[cfg(target_arch = "aarch64")]
pub mod neon;

#[cfg(target_arch = "aarch64")]
pub mod sve2;

#[cfg(target_arch = "x86_64")]
pub mod x86;

#[cfg(target_arch = "x86_64")]
pub mod avx2;

// The branch-free Standard Cursor kernel the two x86 builders share; compiled in
// tests on every target, so its block logic is also tested on aarch64.
#[cfg(any(target_arch = "x86_64", test))]
mod step;

// ============================================================================
// ARM exports with optional runtime dispatch to SVE2 (requires std)
// ============================================================================

/// Should SVE2 be used instead of NEON?
///
/// True only when `SUCCINCTLY_SVE2=1` is set *and* the CPU implements SVE2, so on
/// hardware without SVE2 the variable is silently a no-op. SVE2 is documented in
/// `docs/reference/environment-variables.md` as experimental and slower than NEON
/// on the 128-bit implementations shipping today.
///
/// Per STYLE-0003 the decision is cached: dispatch sits on the per-document build
/// path, and `env::var` there costs ~64ns per build (a linear scan of the
/// environment block plus a lock and an allocation) to re-derive an answer that
/// cannot usefully change. That is ~6% of a 1KB index build and noise by 100KB.
/// The consequence is that the variable is read once per process, so mutating it
/// mid-run has no effect.
#[cfg(all(target_arch = "aarch64", feature = "std"))]
fn use_sve2() -> bool {
    static USE_SVE2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *USE_SVE2.get_or_init(|| {
        std::env::var("SUCCINCTLY_SVE2").is_ok_and(|v| v == "1")
            && std::arch::is_aarch64_feature_detected!("sve2")
    })
}

// Runtime dispatch when std is available
// Priority: SVE2 > NEON (SVE2 is experimental, may not be faster)
#[cfg(all(target_arch = "aarch64", any(test, feature = "std")))]
pub fn build_semi_index_standard(json: &[u8]) -> crate::json::standard::SemiIndex {
    // SVE2 dispatch disabled by default - enable with SUCCINCTLY_SVE2=1
    // SVE2 on 128-bit implementations may not be faster than NEON
    #[cfg(feature = "std")]
    if use_sve2() {
        return unsafe { sve2::build_semi_index_standard(json) };
    }
    neon::build_semi_index_standard(json)
}

#[cfg(all(target_arch = "aarch64", any(test, feature = "std")))]
pub fn build_semi_index_simple(json: &[u8]) -> crate::json::simple::SemiIndex {
    #[cfg(feature = "std")]
    if use_sve2() {
        return unsafe { sve2::build_semi_index_simple(json) };
    }
    neon::build_semi_index_simple(json)
}

// Without std feature, default to NEON (universally available on aarch64)
#[cfg(all(target_arch = "aarch64", not(any(test, feature = "std"))))]
pub use neon::build_semi_index_simple;

#[cfg(all(target_arch = "aarch64", not(any(test, feature = "std"))))]
pub use neon::build_semi_index_standard;

// ============================================================================
// x86_64 exports with optional runtime dispatch (requires std)
// ============================================================================

// Runtime dispatch when std is available (test mode or std feature)
// Priority: AVX2 > SSE2

/// Whether the AVX2 index builders should be dispatched: AVX2 is detected and the
/// clamp-down-only `SUCCINCTLY_SIMD` override (`docs/reference/environment-variables.md`) does
/// not ask for a lower level.
///
/// `SUCCINCTLY_SIMD=sse2` runs the SSE2 builders on an AVX2 machine. That is how CI executes them
/// through the CLI, which nothing else selects (#7318): without `std` they are the only x86_64
/// builders, and with it they run only on a CPU without AVX2.
///
/// Cached per process (STYLE-0003), like `is_x86_feature_detected!` itself: the build sits on the
/// per-document path, and `env::var` there would cost a scan of the environment block each time.
/// Mutating the variable mid-run has no effect.
#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
#[inline]
fn avx2_enabled() -> bool {
    static AVX2: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVX2.get_or_init(|| is_x86_feature_detected!("avx2") && !crate::util::simd::clamp_below_avx2())
}

#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub fn build_semi_index_standard(json: &[u8]) -> crate::json::standard::SemiIndex {
    if avx2_enabled() {
        // SAFETY: `avx2_enabled` implies AVX2 was detected. The unchecked builder, not the safe
        // `avx2::build_semi_index_standard`, so the check is made once (#7317).
        unsafe { avx2::build_semi_index_standard_avx2(json) }
    } else {
        x86::build_semi_index_standard(json)
    }
}

#[cfg(all(target_arch = "x86_64", any(test, feature = "std")))]
pub fn build_semi_index_simple(json: &[u8]) -> crate::json::simple::SemiIndex {
    if avx2_enabled() {
        // SAFETY: `avx2_enabled` implies AVX2 was detected (see `build_semi_index_standard`).
        unsafe { avx2::build_semi_index_simple_avx2(json) }
    } else {
        x86::build_semi_index_simple(json)
    }
}

// Without std feature, default to SSE2 (universally available on x86_64).
// `avx2::build_semi_index_*` use AVX2 in a no_std build only when it is enabled
// at compile time, and fall back to SSE2 otherwise.
#[cfg(all(target_arch = "x86_64", not(any(test, feature = "std"))))]
pub use x86::build_semi_index_simple;

#[cfg(all(target_arch = "x86_64", not(any(test, feature = "std"))))]
pub use x86::build_semi_index_standard;

// ============================================================================
// Fallback for other platforms
// ============================================================================

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::simple::build_semi_index as build_semi_index_simple;

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub use super::standard::build_semi_index as build_semi_index_standard;

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    /// Contract test for the JSON half of the `SUCCINCTLY_SIMD` clamp (#7318), in the spirit of
    /// YAML's `test_succinctly_simd_env_contract`: when a CI leg sets the variable, the dispatcher
    /// must observably honor it, and a typo'd value fails the leg instead of silently un-clamping
    /// it. Reads the environment only, never sets it, so it is race-free across test threads.
    #[test]
    fn test_succinctly_simd_env_contract_json() {
        let detected = is_x86_feature_detected!("avx2");
        match std::env::var("SUCCINCTLY_SIMD") {
            Ok(v) => match crate::util::simd::clamp::parse(&v) {
                Some(true) => assert!(
                    !avx2_enabled(),
                    "SUCCINCTLY_SIMD={v} must clamp the JSON dispatch to SSE2"
                ),
                Some(false) => assert_eq!(avx2_enabled(), detected),
                None => panic!(
                    "SUCCINCTLY_SIMD={v:?} is not a recognized level \
                     (scalar|sse2|sse42|sse4.2|avx2) -- fix the caller so the clamp actually applies"
                ),
            },
            Err(_) => assert_eq!(
                avx2_enabled(),
                detected,
                "unclamped dispatch must use AVX2 whenever it is detected"
            ),
        }
    }

    /// Whichever builder the dispatcher picks, it agrees with the scalar reference, so a CI leg
    /// that clamps it (`SUCCINCTLY_SIMD=sse2`) is checking the SSE2 kernels' answers here.
    #[test]
    fn the_dispatchers_answer_is_the_scalar_references() {
        let long_string = format!(
            "[\"{}\", {{\"k\": \"a\\\"b\"}}, [1, 2.5e3, null, true]]",
            "x".repeat(97)
        );
        let cases: [&[u8]; 5] = [
            b"{}",
            b"[1,2,3]",
            br#"{"a":{"b":[true,false,null,"s\\"]},"c":"d"}"#,
            long_string.as_bytes(),
            b"  [ { \"a\" : 1 } , { \"b\" : [ ] } ]  ",
        ];
        for json in cases {
            let std = build_semi_index_standard(json);
            let want = crate::json::standard::build_semi_index_scalar(json);
            assert_eq!(std.ib, want.ib, "{json:?}");
            assert_eq!(std.bp, want.bp, "{json:?}");
            assert_eq!(std.state, want.state, "{json:?}");
            let simple = build_semi_index_simple(json);
            let want = crate::json::simple::build_semi_index(json);
            assert_eq!(simple.ib, want.ib, "{json:?}");
            assert_eq!(simple.bp, want.bp, "{json:?}");
            assert_eq!(simple.state, want.state, "{json:?}");
        }
    }
}
