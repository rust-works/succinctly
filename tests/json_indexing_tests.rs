//! Tests for JSON semi-indexing (both scalar and SIMD versions).

use succinctly::json::{simple, standard};

#[cfg(target_arch = "aarch64")]
use succinctly::json::simd;

// ============================================================================
// Helper functions
// ============================================================================

/// Extract bit at position i from a word vector (LSB-first within each word)
fn get_bit(words: &[u64], i: usize) -> bool {
    let word_idx = i / 64;
    let bit_idx = i % 64;
    if word_idx < words.len() {
        (words[word_idx] >> bit_idx) & 1 == 1
    } else {
        false
    }
}

/// Convert bit vector to string of '0' and '1' for first n bits
fn bits_to_string(words: &[u64], n: usize) -> String {
    (0..n)
        .map(|i| if get_bit(words, i) { '1' } else { '0' })
        .collect()
}

/// Count the number of 1 bits in the first n bits
fn count_ones(words: &[u64], n: usize) -> usize {
    (0..n).filter(|&i| get_bit(words, i)).count()
}

// ============================================================================
// Simple Cursor Tests
// ============================================================================

mod simple_cursor {
    use super::*;

    #[test]
    fn test_empty_json() {
        // Empty object
        let semi = simple::build_semi_index(b"{}");
        assert_eq!(bits_to_string(&semi.ib, 2), "11");
        assert_eq!(bits_to_string(&semi.bp, 4), "1100");

        // Empty array
        let semi = simple::build_semi_index(b"[]");
        assert_eq!(bits_to_string(&semi.ib, 2), "11");
        assert_eq!(bits_to_string(&semi.bp, 4), "1100");
    }

    #[test]
    fn test_simple_values() {
        // Array with numbers
        let semi = simple::build_semi_index(b"[1,2,3]");
        // IB marks [ , , ]
        assert_eq!(count_ones(&semi.ib, 7), 4); // [, ,, ,, ]

        // Object with string value
        let json = br#"{"key":"value"}"#;
        let semi = simple::build_semi_index(json);
        // IB marks { : }
        assert_eq!(count_ones(&semi.ib, json.len()), 3);
    }

    #[test]
    fn test_nested_structures() {
        // Nested objects
        let json = br#"{"a":{"b":{"c":1}}}"#;
        let semi = simple::build_semi_index(json);
        // Should have balanced BP
        let bp_len = semi.bp.iter().map(|w| w.count_ones()).sum::<u32>() as usize * 2;
        assert!(bp_len > 0);

        // Nested arrays
        let json = b"[[[]]]";
        let semi = simple::build_semi_index(json);
        assert_eq!(bits_to_string(&semi.ib, 6), "111111");
        // Each [ adds 11, each ] adds 00
        assert_eq!(bits_to_string(&semi.bp, 12), "111111000000");
    }

    #[test]
    fn test_mixed_structures() {
        let json = br#"{"arr":[1,2],"obj":{"x":3}}"#;
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);
    }

    #[test]
    fn test_string_escapes() {
        // Escaped quote
        let json = br#"{"a":"b\"c"}"#;
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);

        // Escaped backslash
        let json = br#"{"a":"b\\c"}"#;
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);

        // Multiple escapes
        let json = br#"{"a":"b\\\"c"}"#;
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);
    }

    #[test]
    fn test_unicode_in_strings() {
        // Unicode should be treated as regular string content
        // Using escaped unicode since raw byte strings must be ASCII
        let json = b"{\"emoji\":\"\\u1F389\"}";
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);

        // Escaped unicode
        let json = br#"{"unicode":"\u0041"}"#;
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);
    }

    #[test]
    fn test_whitespace_variations() {
        // Minimal whitespace
        let json1 = br#"{"a":1}"#;
        let semi1 = simple::build_semi_index(json1);

        // Lots of whitespace
        let json2 = b"{ \"a\" : 1 }";
        let semi2 = simple::build_semi_index(json2);

        // Both should have same number of structural chars marked
        // (though at different positions)
        assert_eq!(count_ones(&semi1.ib, json1.len()), 3); // { : }
        assert_eq!(count_ones(&semi2.ib, json2.len()), 3); // { : }
    }

    #[test]
    fn test_unterminated_states() {
        // Unterminated string
        let json = br#"{"key":"value"#;
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InString);

        // Unterminated escape
        let json = br#"{"key":"\"#;
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InEscape);
    }

    #[test]
    fn test_large_json() {
        // Generate a large JSON array
        let mut json = Vec::new();
        json.push(b'[');
        for i in 0..1000 {
            if i > 0 {
                json.push(b',');
            }
            json.extend_from_slice(format!("{}", i).as_bytes());
        }
        json.push(b']');

        let semi = simple::build_semi_index(&json);
        assert_eq!(semi.state, simple::State::InJson);

        // Should have IB marks for [ , , , ... , ]
        // 1 for [, 999 for commas, 1 for ]
        assert_eq!(count_ones(&semi.ib, json.len()), 1001);
    }
}

// ============================================================================
// Standard Cursor Tests
// ============================================================================

mod standard_cursor {
    use super::*;

    #[test]
    fn test_empty_json() {
        // Empty object
        let semi = standard::build_semi_index(b"{}");
        assert_eq!(bits_to_string(&semi.ib, 2), "10"); // { marked, } not
        assert_eq!(bits_to_string(&semi.bp, 2), "10"); // open, close

        // Empty array
        let semi = standard::build_semi_index(b"[]");
        assert_eq!(bits_to_string(&semi.ib, 2), "10");
        assert_eq!(bits_to_string(&semi.bp, 2), "10");
    }

    #[test]
    fn test_values_as_leaves() {
        // Array with numbers - each number is a leaf
        let json = b"[1,2,3]";
        let semi = standard::build_semi_index(json);
        // IB marks: [ 1 2 3 (opening bracket and start of each value)
        assert_eq!(bits_to_string(&semi.ib, 7), "1101010");
        // BP: [ -> 1, each number -> 10, ] -> 0
        assert_eq!(bits_to_string(&semi.bp, 8), "11010100");
    }

    #[test]
    fn test_string_values() {
        let json = br#"{"a":"b"}"#;
        let semi = standard::build_semi_index(json);
        // IB marks: { " (key start) " (value start)
        // Position: 0 1 2 3 4 5 6 7 8
        // Char:     { " a " : " b " }
        // IB:       1 1 0 0 0 1 0 0 0
        assert_eq!(bits_to_string(&semi.ib, 9), "110001000");
    }

    #[test]
    fn test_boolean_null_values() {
        // true
        let json = b"[true]";
        let semi = standard::build_semi_index(json);
        // Only 't' is marked as value start
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");

        // false
        let json = b"[false]";
        let semi = standard::build_semi_index(json);
        assert_eq!(bits_to_string(&semi.ib, 7), "1100000");

        // null
        let json = b"[null]";
        let semi = standard::build_semi_index(json);
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");
    }

    #[test]
    fn test_number_formats() {
        // Negative number
        let json = b"[-123]";
        let semi = standard::build_semi_index(json);
        // Only '-' is marked as value start
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");

        // Decimal
        let json = b"[3.14]";
        let semi = standard::build_semi_index(json);
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");

        // Scientific notation
        let json = b"[1e10]";
        let semi = standard::build_semi_index(json);
        assert_eq!(bits_to_string(&semi.ib, 6), "110000");

        // Scientific with sign
        let json = b"[1e+10]";
        let semi = standard::build_semi_index(json);
        assert_eq!(bits_to_string(&semi.ib, 7), "1100000");
    }

    #[test]
    fn test_nested_structures() {
        let json = br#"{"a":{"b":1}}"#;
        let semi = standard::build_semi_index(json);
        // Position:  0 1 2 3 4 5 6 7 8 9 10 11 12
        // Char:      { " a " : { " b " :  1  }  }
        // IB:        1 1 0 0 0 1 1 0 0 0  1  0  0
        assert_eq!(bits_to_string(&semi.ib, 13), "1100011000100");
    }

    #[test]
    fn test_value_state_transitions() {
        // Value followed by close bracket
        let json = b"[123]";
        let semi = standard::build_semi_index(json);
        assert_eq!(semi.state, standard::State::InJson);

        // Value followed by comma
        let json = b"[1,2]";
        let semi = standard::build_semi_index(json);
        assert_eq!(semi.state, standard::State::InJson);

        // Value followed by whitespace then close
        let json = b"[123 ]";
        let semi = standard::build_semi_index(json);
        assert_eq!(semi.state, standard::State::InJson);
    }

    #[test]
    fn test_complex_json() {
        let json = br#"{"items":[{"id":1,"name":"foo"},{"id":2,"name":"bar"}],"count":2}"#;
        let semi = standard::build_semi_index(json);
        assert_eq!(semi.state, standard::State::InJson);

        // BP should be non-empty
        assert!(!semi.bp.is_empty());
    }

    #[test]
    fn test_large_json() {
        // Generate large JSON with mixed types
        let mut json = Vec::new();
        json.extend_from_slice(br#"{"items":["#);
        for i in 0..500 {
            if i > 0 {
                json.push(b',');
            }
            json.extend_from_slice(format!(r#"{{"id":{},"value":"item{}"}}"#, i, i).as_bytes());
        }
        json.extend_from_slice(br#"]}"#);

        let semi = standard::build_semi_index(&json);
        assert_eq!(semi.state, standard::State::InJson);
    }
}

// ============================================================================
// SIMD vs Scalar Comparison Tests
// ============================================================================

#[cfg(target_arch = "aarch64")]
mod simd_comparison {
    use super::*;
    use rand::prelude::*;

    fn compare_results(json: &[u8]) {
        let scalar = standard::build_semi_index(json);
        let simd = simd::build_semi_index_standard(json);

        assert_eq!(
            bits_to_string(&scalar.ib, json.len()),
            bits_to_string(&simd.ib, json.len()),
            "IB mismatch for JSON: {:?}",
            String::from_utf8_lossy(json)
        );

        assert_eq!(
            scalar.state,
            simd.state,
            "State mismatch for JSON: {:?}",
            String::from_utf8_lossy(json)
        );

        // Compare BP (note: length may vary based on structure)
        let scalar_bp_len = scalar.bp.len() * 64;
        let simd_bp_len = simd.bp.len() * 64;
        let min_len = scalar_bp_len.min(simd_bp_len);

        assert_eq!(
            bits_to_string(&scalar.bp, min_len),
            bits_to_string(&simd.bp, min_len),
            "BP mismatch for JSON: {:?}",
            String::from_utf8_lossy(json)
        );
    }

    #[test]
    fn test_simd_empty_structures() {
        compare_results(b"{}");
        compare_results(b"[]");
        compare_results(b"[[]]");
        compare_results(b"{{}}"); // Invalid JSON but still tests state machine
    }

    #[test]
    fn test_simd_simple_values() {
        compare_results(b"[1]");
        compare_results(b"[1,2,3]");
        compare_results(b"[true,false,null]");
        compare_results(b"[-1,+2,3.14]");
    }

    #[test]
    fn test_simd_strings() {
        compare_results(br#"{"a":"b"}"#);
        compare_results(br#"["hello","world"]"#);
        compare_results(br#"{"key":"value with spaces"}"#);
    }

    #[test]
    fn test_simd_escapes() {
        compare_results(br#"{"a":"b\"c"}"#);
        compare_results(br#"{"a":"b\\c"}"#);
        compare_results(br#"{"a":"b\nc"}"#);
        compare_results(br#"{"a":"\\\"\\\"}"#);
    }

    #[test]
    fn test_simd_nested() {
        compare_results(br#"{"a":{"b":{"c":1}}}"#);
        compare_results(br#"[[[1,2],[3,4]],[[5,6]]]"#);
        compare_results(br#"{"a":[1,{"b":2}]}"#);
    }

    #[test]
    fn test_simd_whitespace() {
        compare_results(b"{ }");
        compare_results(b"[ ]");
        compare_results(b"{ \"a\" : 1 }");
        compare_results(b"[\n  1,\n  2\n]");
    }

    #[test]
    fn test_simd_boundary_sizes() {
        // Test various sizes around 16-byte boundaries
        for size in [1, 15, 16, 17, 31, 32, 33, 47, 48, 49, 63, 64, 65] {
            let mut json = Vec::with_capacity(size);
            json.push(b'[');
            for i in 0..(size.saturating_sub(2)) {
                if i > 0 && i % 2 == 0 {
                    json.push(b',');
                } else {
                    json.push(b'1');
                }
            }
            if json.len() < size {
                json.push(b']');
            }
            while json.len() < size {
                json.insert(1, b' ');
            }
            json.truncate(size);

            // Ensure valid-ish JSON
            if json.last() != Some(&b']') && json.last() != Some(&b' ') {
                *json.last_mut().unwrap() = b']';
            }

            compare_results(&json);
        }
    }

    #[test]
    fn test_simd_large_json() {
        // Test with > 16 bytes
        let json = br#"{"name":"value","number":12345,"array":[1,2,3]}"#;
        compare_results(json);

        // Test with > 32 bytes
        let json = br#"{"long_key_name":"long_value_string","another":"field"}"#;
        compare_results(json);

        // Test with > 64 bytes
        let json = br#"{"items":[{"id":1,"name":"first"},{"id":2,"name":"second"},{"id":3,"name":"third"}]}"#;
        compare_results(json);
    }

    #[test]
    fn test_simd_all_characters() {
        // Test all JSON structural characters
        compare_results(br#"{"a":{"b":[1,2]},"c":[{"d":3}]}"#);

        // Test value types
        compare_results(br#"[1,-2,3.14,-1.5e+10,true,false,null,"str"]"#);
    }

    #[test]
    fn test_simd_long_strings() {
        // String longer than 16 bytes
        let json = br#"{"key":"this is a very long string value that exceeds 16 bytes"}"#;
        compare_results(json);

        // Multiple long strings
        let json = br#"{"a":"0123456789012345","b":"0123456789012345"}"#;
        compare_results(json);
    }

    #[test]
    fn test_simd_escape_at_boundary() {
        // Try to create escapes at various positions relative to 16-byte chunks
        for padding in 0..20 {
            let mut json = Vec::new();
            json.extend_from_slice(br#"{"#);
            json.extend(std::iter::repeat_n(b'x', padding));
            json.extend_from_slice(br#"":"a\"b"}"#);
            compare_results(&json);
        }
    }

    #[test]
    fn test_simd_random_valid_json() {
        use rand::prelude::*;
        use rand_chacha::ChaCha8Rng;

        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..100 {
            let json = generate_random_json(&mut rng, 3, 50);
            compare_results(&json);
        }
    }

    fn generate_random_json<R: Rng>(rng: &mut R, max_depth: usize, max_size: usize) -> Vec<u8> {
        let mut result = Vec::new();
        generate_random_value(rng, &mut result, max_depth, max_size);
        result
    }

    fn generate_random_value<R: Rng>(
        rng: &mut R,
        out: &mut Vec<u8>,
        depth: usize,
        max_size: usize,
    ) {
        if out.len() >= max_size || depth == 0 {
            // Generate a simple value
            match rng.gen_range(0..5) {
                0 => out.extend_from_slice(b"null"),
                1 => out.extend_from_slice(b"true"),
                2 => out.extend_from_slice(b"false"),
                3 => out.extend_from_slice(format!("{}", rng.gen_range(-100..100)).as_bytes()),
                _ => {
                    out.push(b'"');
                    let len = rng.gen_range(0..10);
                    for _ in 0..len {
                        let c = rng.gen_range(b'a'..=b'z');
                        out.push(c);
                    }
                    out.push(b'"');
                }
            }
            return;
        }

        match rng.gen_range(0..7) {
            0 => out.extend_from_slice(b"null"),
            1 => out.extend_from_slice(b"true"),
            2 => out.extend_from_slice(b"false"),
            3 => out.extend_from_slice(format!("{}", rng.gen_range(-100..100)).as_bytes()),
            4 => {
                out.push(b'"');
                let len = rng.gen_range(0..10);
                for _ in 0..len {
                    let c = rng.gen_range(b'a'..=b'z');
                    out.push(c);
                }
                out.push(b'"');
            }
            5 => {
                // Array
                out.push(b'[');
                let count = rng.gen_range(0..4);
                for i in 0..count {
                    if i > 0 {
                        out.push(b',');
                    }
                    generate_random_value(rng, out, depth - 1, max_size);
                }
                out.push(b']');
            }
            _ => {
                // Object
                out.push(b'{');
                let count = rng.gen_range(0..3);
                for i in 0..count {
                    if i > 0 {
                        out.push(b',');
                    }
                    out.push(b'"');
                    out.push(rng.gen_range(b'a'..=b'z'));
                    out.push(b'"');
                    out.push(b':');
                    generate_random_value(rng, out, depth - 1, max_size);
                }
                out.push(b'}');
            }
        }
    }
}

// ============================================================================
// BP Structure Correctness Tests
// ============================================================================

mod bp_structure {
    use super::*;
    use succinctly::bp::BalancedParens;

    #[test]
    fn test_simple_bp_navigation() {
        // Simple object: {}
        let semi = standard::build_semi_index(b"{}");
        // BP should be: 10 (open, close)
        assert_eq!(bits_to_string(&semi.bp, 2), "10");

        let bp = BalancedParens::new(semi.bp, 2);
        assert!(bp.is_open(0));
        assert!(!bp.is_open(1));
        assert_eq!(bp.find_close(0), Some(1));
    }

    #[test]
    fn test_nested_bp_navigation() {
        // Nested: {"a":1}
        let semi = standard::build_semi_index(br#"{"a":1}"#);
        // BP: { -> 1, "a" -> 10, 1 -> 10, } -> 0
        // So: 1 10 10 0

        let bp_actual_len = 6; // 1 + 10 + 10 + 0
        let bp = BalancedParens::new(semi.bp.clone(), bp_actual_len);

        // The root should close at the end
        assert!(bp.is_open(0));
        assert_eq!(bp.find_close(0), Some(5));
    }

    #[test]
    fn test_array_bp_navigation() {
        // Array: [1,2]
        let semi = standard::build_semi_index(b"[1,2]");
        // BP: [ -> 1, 1 -> 10, 2 -> 10, ] -> 0
        // So: 1 10 10 0 = 1 1 0 1 0 0

        let bp = BalancedParens::new(semi.bp.clone(), 6);
        assert!(bp.is_open(0));
        assert_eq!(bp.find_close(0), Some(5)); // [ closes at ]

        // First element (1)
        assert!(bp.is_open(1));
        assert_eq!(bp.find_close(1), Some(2)); // leaf

        // Second element (2)
        assert!(bp.is_open(3));
        assert_eq!(bp.find_close(3), Some(4)); // leaf
    }

    #[test]
    fn test_deeply_nested_bp() {
        // Deeply nested: [[[[]]]]
        let semi = simple::build_semi_index(b"[[[[]]]]");
        // Simple cursor: each [ -> 11, each ] -> 00
        // BP: 11 11 11 11 00 00 00 00

        let bp_len = 16;
        let bp = BalancedParens::new(semi.bp.clone(), bp_len);

        // Outermost brackets
        assert!(bp.is_open(0));
        assert_eq!(bp.find_close(0), Some(15));

        // Second level
        assert!(bp.is_open(2));
        assert_eq!(bp.find_close(2), Some(13));

        // Innermost
        assert!(bp.is_open(6));
        assert_eq!(bp.find_close(6), Some(9));
    }
}

// ============================================================================
// Edge Cases and Stress Tests
// ============================================================================

mod edge_cases {
    use super::*;

    #[test]
    fn test_empty_input() {
        let semi = simple::build_semi_index(b"");
        assert!(semi.ib.is_empty() || semi.ib[0] == 0);
        assert!(semi.bp.is_empty() || semi.bp[0] == 0);

        let semi = standard::build_semi_index(b"");
        assert!(semi.ib.is_empty() || semi.ib[0] == 0);
        assert!(semi.bp.is_empty() || semi.bp[0] == 0);
    }

    #[test]
    fn test_single_characters() {
        // Just a brace
        let semi = simple::build_semi_index(b"{");
        assert_eq!(bits_to_string(&semi.ib, 1), "1");

        let semi = standard::build_semi_index(b"{");
        assert_eq!(bits_to_string(&semi.ib, 1), "1");
    }

    #[test]
    fn test_only_whitespace() {
        let semi = simple::build_semi_index(b"   ");
        assert_eq!(count_ones(&semi.ib, 3), 0);

        let semi = standard::build_semi_index(b"   ");
        assert_eq!(count_ones(&semi.ib, 3), 0);
    }

    #[test]
    fn test_consecutive_escapes() {
        // Multiple backslashes (\\\\\ -> three backslashes in the string)
        // Raw string: {"a":"\\\\\\"}
        // Actual bytes: {"a":"\\\"} which has: \\ (escaped backslash) + \" (escaped quote)
        // This is actually incomplete - the string never closes
        // Let's use a valid case:
        let json = br#"{"a":"\\\\"}"#; // {"a":"\\\\"}  -> value is two backslashes
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);

        let semi = standard::build_semi_index(json);
        assert_eq!(semi.state, standard::State::InJson);

        // Test escaped quote followed by more content
        let json = br#"{"a":"\"test\""}"#; // {"a":"\"test\""} -> value is "test"
        let semi = simple::build_semi_index(json);
        assert_eq!(semi.state, simple::State::InJson);

        let semi = standard::build_semi_index(json);
        assert_eq!(semi.state, standard::State::InJson);
    }

    #[test]
    fn test_long_string() {
        // Very long string without escapes
        let mut json = Vec::new();
        json.extend_from_slice(br#"{"data":""#);
        json.extend(std::iter::repeat_n(b'x', 1000));
        json.extend_from_slice(br#""}"#);

        let semi = standard::build_semi_index(&json);
        assert_eq!(semi.state, standard::State::InJson);
    }

    #[test]
    fn test_many_small_values() {
        // Many small values
        let mut json = Vec::new();
        json.push(b'[');
        for i in 0..1000 {
            if i > 0 {
                json.push(b',');
            }
            json.push(b'1');
        }
        json.push(b']');

        let semi = standard::build_semi_index(&json);
        assert_eq!(semi.state, standard::State::InJson);

        // Should have 1001 IB marks ([ and 1000 values)
        assert_eq!(count_ones(&semi.ib, json.len()), 1001);
    }

    #[test]
    fn test_deep_nesting() {
        // Very deep nesting
        let depth = 100;
        let mut json = Vec::new();
        json.extend(std::iter::repeat_n(b'[', depth));
        json.extend(std::iter::repeat_n(b']', depth));

        let semi = standard::build_semi_index(&json);
        assert_eq!(semi.state, standard::State::InJson);
    }

    #[test]
    fn test_special_number_formats() {
        // Various number formats
        let cases = [
            b"[0]".as_slice(),
            b"[-0]",
            b"[0.0]",
            b"[1e0]",
            b"[1E0]",
            b"[1e+0]",
            b"[1e-0]",
            b"[1.0e+10]",
        ];

        for json in cases {
            let semi = standard::build_semi_index(json);
            assert_eq!(
                semi.state,
                standard::State::InJson,
                "Failed for: {}",
                String::from_utf8_lossy(json)
            );
        }
    }
}
