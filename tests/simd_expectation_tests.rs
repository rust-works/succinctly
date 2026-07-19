//! Hard assertion that the CPU features CI expects are actually detected.
//!
//! The SIMD test suites self-skip when a feature is missing (emitting a
//! visible `SKIPPED` line — see `note_simd_skip`, #191/#193). That keeps local
//! runs green on any hardware, but on CI it would let a runner-fleet change
//! silently skip entire suites while the leg stays green. Setting
//! `SUCCINCTLY_EXPECT_SIMD` pins the contract: every named feature must be
//! runtime-detected or this test fails the leg.
//!
//! The value is a comma-separated feature list, e.g.
//! `SUCCINCTLY_EXPECT_SIMD=sse2,sse4.2,avx2,bmi2,popcnt` on x86_64 or
//! `SUCCINCTLY_EXPECT_SIMD=neon` on aarch64. Unset, the test is a no-op, so
//! local `cargo test` behaves as before. Unknown names fail (a typo must not
//! silently satisfy the expectation). See
//! `docs/reference/environment-variables.md`.

/// Runtime-detects `name` on the current target.
///
/// Returns `None` for names this target cannot detect, which the caller
/// treats as a hard failure — the env var is set per CI job, so its contents
/// must match the job's architecture.
fn feature_detected(name: &str) -> Option<bool> {
    #[cfg(target_arch = "x86_64")]
    {
        Some(match name {
            "sse2" => is_x86_feature_detected!("sse2"),
            "sse4.2" => is_x86_feature_detected!("sse4.2"),
            "avx2" => is_x86_feature_detected!("avx2"),
            "bmi2" => is_x86_feature_detected!("bmi2"),
            "popcnt" => is_x86_feature_detected!("popcnt"),
            _ => return None,
        })
    }
    #[cfg(target_arch = "aarch64")]
    {
        Some(match name {
            "neon" => std::arch::is_aarch64_feature_detected!("neon"),
            "sve2" => std::arch::is_aarch64_feature_detected!("sve2"),
            "sve2-bitperm" => std::arch::is_aarch64_feature_detected!("sve2-bitperm"),
            _ => return None,
        })
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = name;
        None
    }
}

#[test]
fn test_expected_simd_features_are_detected() {
    let Ok(expected) = std::env::var("SUCCINCTLY_EXPECT_SIMD") else {
        eprintln!("SUCCINCTLY_EXPECT_SIMD unset: skipping SIMD feature expectation check");
        return;
    };

    let mut missing = Vec::new();
    let mut unknown = Vec::new();
    for name in expected.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match feature_detected(name) {
            Some(true) => eprintln!("SIMD feature expectation met: `{name}` detected"),
            Some(false) => missing.push(name),
            None => unknown.push(name),
        }
    }

    assert!(
        unknown.is_empty(),
        "SUCCINCTLY_EXPECT_SIMD lists features unknown on this target: {unknown:?}"
    );
    assert!(
        missing.is_empty(),
        "SUCCINCTLY_EXPECT_SIMD features not detected on this CPU: {missing:?} — \
         the SIMD suites guarded by these features would silently self-skip (see #193)"
    );
}
