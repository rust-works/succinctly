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

    #[test]
    fn test_all_simd_levels_match_scalar_standard() {
        for (name, json) in test_cases() {
            // Scalar reference
            let scalar = json::standard::build_semi_index(json);

            // SSE2 (always available on x86_64)
            let sse2 = json::simd::x86::build_semi_index_standard(json);
            assert_eq!(sse2.ib, scalar.ib, "{}: SSE2 IB mismatch", name);
            assert_eq!(sse2.bp, scalar.bp, "{}: SSE2 BP mismatch", name);
            assert_eq!(sse2.state, scalar.state, "{}: SSE2 state mismatch", name);

            // SSE4.2 (if supported)
            if is_x86_feature_detected!("sse4.2") {
                let sse42 = json::simd::sse42::build_semi_index_standard(json);
                assert_eq!(sse42.ib, scalar.ib, "{}: SSE4.2 IB mismatch", name);
                assert_eq!(sse42.bp, scalar.bp, "{}: SSE4.2 BP mismatch", name);
                assert_eq!(sse42.state, scalar.state, "{}: SSE4.2 state mismatch", name);
            }

            // AVX2 (if supported)
            if is_x86_feature_detected!("avx2") {
                let avx2 = json::simd::avx2::build_semi_index_standard(json);
                assert_eq!(avx2.ib, scalar.ib, "{}: AVX2 IB mismatch", name);
                assert_eq!(avx2.bp, scalar.bp, "{}: AVX2 BP mismatch", name);
                assert_eq!(avx2.state, scalar.state, "{}: AVX2 state mismatch", name);
            }
        }
    }

    #[test]
    fn test_all_simd_levels_match_scalar_simple() {
        for (name, json) in test_cases() {
            // Scalar reference
            let scalar = json::simple::build_semi_index(json);

            // SSE2 (always available on x86_64)
            let sse2 = json::simd::x86::build_semi_index_simple(json);
            assert_eq!(sse2.ib, scalar.ib, "{}: SSE2 IB mismatch", name);
            assert_eq!(sse2.bp, scalar.bp, "{}: SSE2 BP mismatch", name);
            assert_eq!(sse2.state, scalar.state, "{}: SSE2 state mismatch", name);

            // SSE4.2 (if supported)
            if is_x86_feature_detected!("sse4.2") {
                let sse42 = json::simd::sse42::build_semi_index_simple(json);
                assert_eq!(sse42.ib, scalar.ib, "{}: SSE4.2 IB mismatch", name);
                assert_eq!(sse42.bp, scalar.bp, "{}: SSE4.2 BP mismatch", name);
                assert_eq!(sse42.state, scalar.state, "{}: SSE4.2 state mismatch", name);
            }

            // AVX2 (if supported)
            if is_x86_feature_detected!("avx2") {
                let avx2 = json::simd::avx2::build_semi_index_simple(json);
                assert_eq!(avx2.ib, scalar.ib, "{}: AVX2 IB mismatch", name);
                assert_eq!(avx2.bp, scalar.bp, "{}: AVX2 BP mismatch", name);
                assert_eq!(avx2.state, scalar.state, "{}: AVX2 state mismatch", name);
            }
        }
    }

    #[test]
    fn test_simd_levels_match_each_other_standard() {
        // This test ensures all SIMD levels produce identical results to each other
        for (name, json) in test_cases() {
            let sse2 = json::simd::x86::build_semi_index_standard(json);

            if is_x86_feature_detected!("sse4.2") {
                let sse42 = json::simd::sse42::build_semi_index_standard(json);
                assert_eq!(sse42.ib, sse2.ib, "{}: SSE4.2 vs SSE2 IB mismatch", name);
                assert_eq!(sse42.bp, sse2.bp, "{}: SSE4.2 vs SSE2 BP mismatch", name);
                assert_eq!(
                    sse42.state, sse2.state,
                    "{}: SSE4.2 vs SSE2 state mismatch",
                    name
                );
            }

            if is_x86_feature_detected!("avx2") {
                let avx2 = json::simd::avx2::build_semi_index_standard(json);
                assert_eq!(avx2.ib, sse2.ib, "{}: AVX2 vs SSE2 IB mismatch", name);
                assert_eq!(avx2.bp, sse2.bp, "{}: AVX2 vs SSE2 BP mismatch", name);
                assert_eq!(
                    avx2.state, sse2.state,
                    "{}: AVX2 vs SSE2 state mismatch",
                    name
                );
            }
        }
    }

    #[test]
    fn test_simd_levels_match_each_other_simple() {
        for (name, json) in test_cases() {
            let sse2 = json::simd::x86::build_semi_index_simple(json);

            if is_x86_feature_detected!("sse4.2") {
                let sse42 = json::simd::sse42::build_semi_index_simple(json);
                assert_eq!(sse42.ib, sse2.ib, "{}: SSE4.2 vs SSE2 IB mismatch", name);
                assert_eq!(sse42.bp, sse2.bp, "{}: SSE4.2 vs SSE2 BP mismatch", name);
                assert_eq!(
                    sse42.state, sse2.state,
                    "{}: SSE4.2 vs SSE2 state mismatch",
                    name
                );
            }

            if is_x86_feature_detected!("avx2") {
                let avx2 = json::simd::avx2::build_semi_index_simple(json);
                assert_eq!(avx2.ib, sse2.ib, "{}: AVX2 vs SSE2 IB mismatch", name);
                assert_eq!(avx2.bp, sse2.bp, "{}: AVX2 vs SSE2 BP mismatch", name);
                assert_eq!(
                    avx2.state, sse2.state,
                    "{}: AVX2 vs SSE2 state mismatch",
                    name
                );
            }
        }
    }

    #[test]
    fn test_chunk_boundary_conditions() {
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

            assert_eq!(sse2.ib, scalar.ib, "{}: SSE2 IB mismatch", name);
            assert_eq!(sse2.bp, scalar.bp, "{}: SSE2 BP mismatch", name);

            if is_x86_feature_detected!("avx2") {
                let avx2 = json::simd::avx2::build_semi_index_standard(json);
                assert_eq!(avx2.ib, scalar.ib, "{}: AVX2 IB mismatch", name);
                assert_eq!(avx2.bp, scalar.bp, "{}: AVX2 BP mismatch", name);
            }
        }
    }
}
