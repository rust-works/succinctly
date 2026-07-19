//! Cross-level SIMD testing to ensure all instruction set levels work correctly.
//!
//! This test suite explicitly tests all SIMD levels (SSE2, SSE4.2, AVX2) and
//! verifies they produce identical results. Unlike regular tests which use
//! runtime dispatch to select the best level, these tests force-test each
//! specific level regardless of what the CPU supports.

#[cfg(target_arch = "x86_64")]
mod x86_simd_levels {
    use succinctly::json;

    /// Test data: various JSON inputs to test
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
        ]
    }

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
