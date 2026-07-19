#![allow(unsafe_code)]
// SVE2 build functions are `unsafe fn` (require runtime
// feature detection); every call site here is gated on `sve2::has_sve2()`.
//! Cross-level SIMD testing to ensure all instruction set levels work correctly.
//!
//! This test suite explicitly tests all SIMD levels (SSE2, SSE4.2, AVX2 on
//! x86_64; NEON, SVE2 on aarch64) and verifies they produce identical results.
//! Unlike regular tests which use runtime dispatch to select the best level,
//! these tests force-test each specific level regardless of what the CPU
//! supports.

/// Test data: various JSON inputs to test.
///
/// Includes invalid JSON: all backends must produce identical indexes even on
/// malformed input, otherwise the same fuzz case indexes differently per
/// architecture (#186).
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn test_cases() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("empty object", b"{}"),
        ("empty array", b"[]"),
        ("simple object", br#"{"a":"b"}"#),
        ("simple array", b"[1,2,3]"),
        ("nested", br#"{"a":{"b":1},"c":[2,3]}"#),
        ("escaped", br#"{"key":"val\"ue"}"#),
        ("numbers", br#"{"int":123,"float":45.67,"sci":1e-5}"#),
        ("whitespace", b"{  \"a\"  :  1  }"),
        (
            "long",
            br#"{"name":"value","number":12345,"array":[1,2,3],"nested":{"x":"y"}}"#,
        ),
        // Bytes adjacent to the value-char ranges (0-9 A-Z a-z + - .), which a
        // range-endpoint bug would misclassify as value chars (#186).
        ("invalid value boundary bytes", b"[@\\^_`|~\x7F]"),
        // Bytes sharing nibbles with ',' (0x2C) and ':' (0x3A).
        ("invalid delim boundary bytes", b"[*<]"),
        // Same boundary bytes straddling the 16B and 32B chunk boundaries:
        // '@' at offset 15, '`' at 16, '|' at 31, '~' at 32.
        (
            "invalid boundary bytes straddling chunks",
            b"[              @`              |~]",
        ),
    ]
}

#[cfg(target_arch = "x86_64")]
mod x86_simd_levels {
    use succinctly::json;

    use super::test_cases;

    /// Detection guard for SSE4.2; emits a visible `SKIPPED` line when
    /// unavailable so a fully-skipped level doesn't read as green (#193).
    fn has_sse42() -> bool {
        let available = is_x86_feature_detected!("sse4.2");
        if !available {
            eprintln!("SKIPPED simd level [sse4.2]: sse4.2 not detected (see #193)");
        }
        available
    }

    /// Detection guard for AVX2; emits a visible `SKIPPED` line when
    /// unavailable so a fully-skipped level doesn't read as green (#193).
    fn has_avx2() -> bool {
        let available = is_x86_feature_detected!("avx2");
        if !available {
            eprintln!("SKIPPED simd level [avx2]: avx2 not detected (see #193)");
        }
        available
    }

    #[test]
    fn test_all_simd_levels_match_scalar_standard() {
        let run_sse42 = has_sse42();
        let run_avx2 = has_avx2();
        for (name, json) in test_cases() {
            // Scalar reference
            let scalar = json::standard::build_semi_index(json);

            // SSE2 (always available on x86_64)
            let sse2 = json::simd::x86::build_semi_index_standard(json);
            assert_eq!(sse2.ib, scalar.ib, "{name}: SSE2 IB mismatch");
            assert_eq!(sse2.bp, scalar.bp, "{name}: SSE2 BP mismatch");
            assert_eq!(sse2.state, scalar.state, "{name}: SSE2 state mismatch");

            // SSE4.2 (if supported)
            if run_sse42 {
                let sse42 = json::simd::sse42::build_semi_index_standard(json);
                assert_eq!(sse42.ib, scalar.ib, "{name}: SSE4.2 IB mismatch");
                assert_eq!(sse42.bp, scalar.bp, "{name}: SSE4.2 BP mismatch");
                assert_eq!(sse42.state, scalar.state, "{name}: SSE4.2 state mismatch");
            }

            // AVX2 (if supported)
            if run_avx2 {
                let avx2 = json::simd::avx2::build_semi_index_standard(json);
                assert_eq!(avx2.ib, scalar.ib, "{name}: AVX2 IB mismatch");
                assert_eq!(avx2.bp, scalar.bp, "{name}: AVX2 BP mismatch");
                assert_eq!(avx2.state, scalar.state, "{name}: AVX2 state mismatch");
            }
        }
    }

    #[test]
    fn test_all_simd_levels_match_scalar_simple() {
        let run_sse42 = has_sse42();
        let run_avx2 = has_avx2();
        for (name, json) in test_cases() {
            // Scalar reference
            let scalar = json::simple::build_semi_index(json);

            // SSE2 (always available on x86_64)
            let sse2 = json::simd::x86::build_semi_index_simple(json);
            assert_eq!(sse2.ib, scalar.ib, "{name}: SSE2 IB mismatch");
            assert_eq!(sse2.bp, scalar.bp, "{name}: SSE2 BP mismatch");
            assert_eq!(sse2.state, scalar.state, "{name}: SSE2 state mismatch");

            // SSE4.2 (if supported)
            if run_sse42 {
                let sse42 = json::simd::sse42::build_semi_index_simple(json);
                assert_eq!(sse42.ib, scalar.ib, "{name}: SSE4.2 IB mismatch");
                assert_eq!(sse42.bp, scalar.bp, "{name}: SSE4.2 BP mismatch");
                assert_eq!(sse42.state, scalar.state, "{name}: SSE4.2 state mismatch");
            }

            // AVX2 (if supported)
            if run_avx2 {
                let avx2 = json::simd::avx2::build_semi_index_simple(json);
                assert_eq!(avx2.ib, scalar.ib, "{name}: AVX2 IB mismatch");
                assert_eq!(avx2.bp, scalar.bp, "{name}: AVX2 BP mismatch");
                assert_eq!(avx2.state, scalar.state, "{name}: AVX2 state mismatch");
            }
        }
    }

    #[test]
    fn test_simd_levels_match_each_other_standard() {
        let run_sse42 = has_sse42();
        let run_avx2 = has_avx2();
        // This test ensures all SIMD levels produce identical results to each other
        for (name, json) in test_cases() {
            let sse2 = json::simd::x86::build_semi_index_standard(json);

            if run_sse42 {
                let sse42 = json::simd::sse42::build_semi_index_standard(json);
                assert_eq!(sse42.ib, sse2.ib, "{name}: SSE4.2 vs SSE2 IB mismatch");
                assert_eq!(sse42.bp, sse2.bp, "{name}: SSE4.2 vs SSE2 BP mismatch");
                assert_eq!(
                    sse42.state, sse2.state,
                    "{name}: SSE4.2 vs SSE2 state mismatch"
                );
            }

            if run_avx2 {
                let avx2 = json::simd::avx2::build_semi_index_standard(json);
                assert_eq!(avx2.ib, sse2.ib, "{name}: AVX2 vs SSE2 IB mismatch");
                assert_eq!(avx2.bp, sse2.bp, "{name}: AVX2 vs SSE2 BP mismatch");
                assert_eq!(
                    avx2.state, sse2.state,
                    "{name}: AVX2 vs SSE2 state mismatch"
                );
            }
        }
    }

    #[test]
    fn test_simd_levels_match_each_other_simple() {
        let run_sse42 = has_sse42();
        let run_avx2 = has_avx2();
        for (name, json) in test_cases() {
            let sse2 = json::simd::x86::build_semi_index_simple(json);

            if run_sse42 {
                let sse42 = json::simd::sse42::build_semi_index_simple(json);
                assert_eq!(sse42.ib, sse2.ib, "{name}: SSE4.2 vs SSE2 IB mismatch");
                assert_eq!(sse42.bp, sse2.bp, "{name}: SSE4.2 vs SSE2 BP mismatch");
                assert_eq!(
                    sse42.state, sse2.state,
                    "{name}: SSE4.2 vs SSE2 state mismatch"
                );
            }

            if run_avx2 {
                let avx2 = json::simd::avx2::build_semi_index_simple(json);
                assert_eq!(avx2.ib, sse2.ib, "{name}: AVX2 vs SSE2 IB mismatch");
                assert_eq!(avx2.bp, sse2.bp, "{name}: AVX2 vs SSE2 BP mismatch");
                assert_eq!(
                    avx2.state, sse2.state,
                    "{name}: AVX2 vs SSE2 state mismatch"
                );
            }
        }
    }

    #[test]
    fn test_chunk_boundary_conditions() {
        let run_avx2 = has_avx2();
        // Test inputs that align with and cross SIMD chunk boundaries

        // Exactly 16 bytes (SSE2/SSE4.2 boundary)
        let json_16 = br#"{"ab":"cdefghi"}"#;
        assert_eq!(json_16.len(), 16);

        // Exactly 32 bytes (AVX2 boundary)
        let json_32 = br#"{"a":"b","c":"d","e":"fghijklm"}"#;
        assert_eq!(json_32.len(), 32);

        // 31 bytes (just under AVX2 boundary)
        let json_31 = br#"{"a":"b","c":"d","e":"fghijkl"}"#;
        assert_eq!(json_31.len(), 31);

        // 33 bytes (just over AVX2 boundary)
        let json_33 = br#"{"a":"b","c":"d","e":"fghijklmn"}"#;
        assert_eq!(json_33.len(), 33);

        for (name, json) in [
            ("16-byte", json_16.as_slice()),
            ("32-byte", json_32.as_slice()),
            ("31-byte", json_31.as_slice()),
            ("33-byte", json_33.as_slice()),
        ] {
            let scalar = json::standard::build_semi_index(json);
            let sse2 = json::simd::x86::build_semi_index_standard(json);

            assert_eq!(sse2.ib, scalar.ib, "{name}: SSE2 IB mismatch");
            assert_eq!(sse2.bp, scalar.bp, "{name}: SSE2 BP mismatch");

            if run_avx2 {
                let avx2 = json::simd::avx2::build_semi_index_standard(json);
                assert_eq!(avx2.ib, scalar.ib, "{name}: AVX2 IB mismatch");
                assert_eq!(avx2.bp, scalar.bp, "{name}: AVX2 BP mismatch");
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod aarch64_simd_levels {
    use succinctly::json;

    use super::test_cases;

    /// Detection guard for SVE2; emits a visible `SKIPPED` line when
    /// unavailable so a fully-skipped level doesn't read as green (#193).
    fn has_sve2() -> bool {
        let available = json::simd::sve2::has_sve2();
        if !available {
            eprintln!("SKIPPED simd level [sve2]: sve2 not detected (see #193)");
        }
        available
    }

    #[test]
    fn test_all_simd_levels_match_scalar_standard() {
        let run_sve2 = has_sve2();
        for (name, json_bytes) in test_cases() {
            // Scalar reference
            let scalar = json::standard::build_semi_index(json_bytes);

            // NEON (always available on aarch64)
            let neon = json::simd::neon::build_semi_index_standard(json_bytes);
            assert_eq!(neon.ib, scalar.ib, "{name}: NEON IB mismatch");
            assert_eq!(neon.bp, scalar.bp, "{name}: NEON BP mismatch");
            assert_eq!(neon.state, scalar.state, "{name}: NEON state mismatch");

            // SVE2 (if supported)
            if run_sve2 {
                let sve2 = unsafe { json::simd::sve2::build_semi_index_standard(json_bytes) };
                assert_eq!(sve2.ib, scalar.ib, "{name}: SVE2 IB mismatch");
                assert_eq!(sve2.bp, scalar.bp, "{name}: SVE2 BP mismatch");
                assert_eq!(sve2.state, scalar.state, "{name}: SVE2 state mismatch");
            }
        }
    }

    #[test]
    fn test_all_simd_levels_match_scalar_simple() {
        let run_sve2 = has_sve2();
        for (name, json_bytes) in test_cases() {
            // Scalar reference
            let scalar = json::simple::build_semi_index(json_bytes);

            // NEON (always available on aarch64)
            let neon = json::simd::neon::build_semi_index_simple(json_bytes);
            assert_eq!(neon.ib, scalar.ib, "{name}: NEON IB mismatch");
            assert_eq!(neon.bp, scalar.bp, "{name}: NEON BP mismatch");
            assert_eq!(neon.state, scalar.state, "{name}: NEON state mismatch");

            // SVE2 (if supported)
            if run_sve2 {
                let sve2 = unsafe { json::simd::sve2::build_semi_index_simple(json_bytes) };
                assert_eq!(sve2.ib, scalar.ib, "{name}: SVE2 IB mismatch");
                assert_eq!(sve2.bp, scalar.bp, "{name}: SVE2 BP mismatch");
                assert_eq!(sve2.state, scalar.state, "{name}: SVE2 state mismatch");
            }
        }
    }
}

/// Exhaustive per-byte differential guard (#186).
///
/// Every byte value 0..=255, embedded in several contexts (bare, mid-chunk,
/// straddling the 16B and 32B chunk boundaries, inside strings, escaped), must
/// produce exactly the scalar index on every available SIMD backend — including
/// on invalid JSON. Divergence on invalid input means the same fuzz case
/// indexes differently per architecture.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod per_byte_differential {
    use succinctly::json;

    /// Contexts in which each byte is embedded before indexing.
    fn contexts(b: u8) -> Vec<(&'static str, Vec<u8>)> {
        let at_offset = |len: usize, offset: usize| {
            let mut v = vec![b' '; len];
            v[offset] = b;
            v
        };
        vec![
            ("bare byte", vec![b]),
            ("mid chunk", at_offset(17, 8)),
            ("before 16B boundary", at_offset(33, 15)),
            ("at 16B boundary", at_offset(33, 16)),
            ("before 32B boundary", at_offset(48, 31)),
            ("at 32B boundary", at_offset(48, 32)),
            ("inside string", vec![b'"', b, b'"']),
            ("escaped in string", vec![b'"', b'\\', b, b'"']),
            (
                "as object value",
                [br#"{"k":"#.as_slice(), &[b], b"}"].concat(),
            ),
        ]
    }

    type StandardBackend = (
        &'static str,
        Box<dyn Fn(&[u8]) -> json::standard::SemiIndex>,
    );
    type SimpleBackend = (&'static str, Box<dyn Fn(&[u8]) -> json::simple::SemiIndex>);

    fn standard_backends() -> Vec<StandardBackend> {
        let mut backends: Vec<StandardBackend> = Vec::new();
        #[cfg(target_arch = "x86_64")]
        {
            backends.push(("sse2", Box::new(json::simd::x86::build_semi_index_standard)));
            if is_x86_feature_detected!("sse4.2") {
                backends.push((
                    "sse4.2",
                    Box::new(json::simd::sse42::build_semi_index_standard),
                ));
            }
            if is_x86_feature_detected!("avx2") {
                backends.push((
                    "avx2",
                    Box::new(json::simd::avx2::build_semi_index_standard),
                ));
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            backends.push((
                "neon",
                Box::new(json::simd::neon::build_semi_index_standard),
            ));
            if json::simd::sve2::has_sve2() {
                backends.push((
                    "sve2",
                    Box::new(|j: &[u8]| unsafe { json::simd::sve2::build_semi_index_standard(j) }),
                ));
            }
        }
        backends
    }

    fn simple_backends() -> Vec<SimpleBackend> {
        let mut backends: Vec<SimpleBackend> = Vec::new();
        #[cfg(target_arch = "x86_64")]
        {
            backends.push(("sse2", Box::new(json::simd::x86::build_semi_index_simple)));
            if is_x86_feature_detected!("sse4.2") {
                backends.push((
                    "sse4.2",
                    Box::new(json::simd::sse42::build_semi_index_simple),
                ));
            }
            if is_x86_feature_detected!("avx2") {
                backends.push(("avx2", Box::new(json::simd::avx2::build_semi_index_simple)));
            }
        }
        #[cfg(target_arch = "aarch64")]
        {
            backends.push(("neon", Box::new(json::simd::neon::build_semi_index_simple)));
            if json::simd::sve2::has_sve2() {
                backends.push((
                    "sve2",
                    Box::new(|j: &[u8]| unsafe { json::simd::sve2::build_semi_index_simple(j) }),
                ));
            }
        }
        backends
    }

    #[test]
    fn test_every_byte_matches_scalar_standard() {
        let backends = standard_backends();
        for b in 0..=255u8 {
            for (ctx, input) in contexts(b) {
                let scalar = json::standard::build_semi_index(&input);
                for (level, build) in &backends {
                    let simd = build(&input);
                    assert_eq!(
                        simd.ib, scalar.ib,
                        "byte 0x{b:02X} [{ctx}]: {level} IB mismatch"
                    );
                    assert_eq!(
                        simd.bp, scalar.bp,
                        "byte 0x{b:02X} [{ctx}]: {level} BP mismatch"
                    );
                    assert_eq!(
                        simd.state, scalar.state,
                        "byte 0x{b:02X} [{ctx}]: {level} state mismatch"
                    );
                }
            }
        }
    }

    #[test]
    fn test_every_byte_matches_scalar_simple() {
        let backends = simple_backends();
        for b in 0..=255u8 {
            for (ctx, input) in contexts(b) {
                let scalar = json::simple::build_semi_index(&input);
                for (level, build) in &backends {
                    let simd = build(&input);
                    assert_eq!(
                        simd.ib, scalar.ib,
                        "byte 0x{b:02X} [{ctx}]: {level} IB mismatch"
                    );
                    assert_eq!(
                        simd.bp, scalar.bp,
                        "byte 0x{b:02X} [{ctx}]: {level} BP mismatch"
                    );
                    assert_eq!(
                        simd.state, scalar.state,
                        "byte 0x{b:02X} [{ctx}]: {level} state mismatch"
                    );
                }
            }
        }
    }
}
