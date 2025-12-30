//! Tests for serde serialization/deserialization.
//!
//! These tests verify that all data structures can be serialized and deserialized
//! correctly, preserving all data and functionality.

#![cfg(feature = "serde")]

use succinctly::bp::BalancedParens;
use succinctly::json::{simple, standard};
use succinctly::{BitVec, Config, RankSelect};

// ============================================================================
// BitVec serialization tests
// ============================================================================

mod bitvec_serde {
    use super::*;

    #[test]
    fn test_empty_bitvec() {
        let bv = BitVec::new();
        let json = serde_json::to_string(&bv).unwrap();
        let restored: BitVec = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 0);
        assert_eq!(restored.count_ones(), 0);
    }

    #[test]
    fn test_simple_bitvec() {
        let words = vec![0b1010_1010u64; 8];
        let bv = BitVec::from_words(words, 512);

        let json = serde_json::to_string(&bv).unwrap();
        let restored: BitVec = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), bv.len());
        assert_eq!(restored.count_ones(), bv.count_ones());

        // Verify rank/select still work correctly
        for i in 0..bv.len() {
            assert_eq!(restored.rank1(i), bv.rank1(i), "rank1 mismatch at {}", i);
        }

        for k in 0..bv.count_ones() {
            assert_eq!(
                restored.select1(k),
                bv.select1(k),
                "select1 mismatch at {}",
                k
            );
        }
    }

    #[test]
    fn test_partial_word_bitvec() {
        let bv = BitVec::from_words(vec![0xFFFF_FFFF_FFFF_FFFFu64], 37);

        let json = serde_json::to_string(&bv).unwrap();
        let restored: BitVec = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 37);
        assert_eq!(restored.count_ones(), 37);
        assert_eq!(restored.rank1(37), 37);
    }

    #[test]
    fn test_large_bitvec() {
        let words: Vec<u64> = (0u64..1000)
            .map(|i| i.wrapping_mul(0x0123_4567_89AB_CDEF))
            .collect();
        let len = 1000 * 64 - 13; // Partial last word
        let bv = BitVec::from_words(words, len);

        let json = serde_json::to_string(&bv).unwrap();
        let restored: BitVec = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), bv.len());
        assert_eq!(restored.count_ones(), bv.count_ones());

        // Spot check some values
        assert_eq!(restored.rank1(1000), bv.rank1(1000));
        assert_eq!(restored.rank1(50000), bv.rank1(50000));
        assert_eq!(restored.select1(100), bv.select1(100));
    }

    #[test]
    fn test_bitvec_with_config() {
        let words = vec![0xAAAA_AAAA_AAAA_AAAAu64; 16];
        let config = Config {
            select_sample_rate: 64,
            build_select0: false,
        };
        let bv = BitVec::with_config(words, 1024, config);

        let json = serde_json::to_string(&bv).unwrap();
        let restored: BitVec = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 1024);
        assert_eq!(restored.count_ones(), 512);

        for k in 0..512 {
            assert_eq!(restored.select1(k), bv.select1(k));
        }
    }
}

// ============================================================================
// BalancedParens serialization tests
// ============================================================================

mod balanced_parens_serde {
    use super::*;

    #[test]
    fn test_empty_bp() {
        let bp = BalancedParens::new(vec![], 0);
        let json = serde_json::to_string(&bp).unwrap();
        let restored: BalancedParens = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 0);
    }

    #[test]
    fn test_simple_bp() {
        // "(()())" = 0b001011
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        let json = serde_json::to_string(&bp).unwrap();
        let restored: BalancedParens = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 6);
        assert_eq!(restored.find_close(0), Some(5));
        assert_eq!(restored.find_close(1), Some(2));
        assert_eq!(restored.find_close(3), Some(4));
    }

    #[test]
    fn test_bp_navigation() {
        // "(()())" = 0b001011
        // Positions: 0=( 1=( 2=) 3=( 4=) 5=)
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        let json = serde_json::to_string(&bp).unwrap();
        let restored: BalancedParens = serde_json::from_str(&json).unwrap();

        // Test all navigation operations
        assert_eq!(restored.first_child(0), Some(1));
        assert_eq!(restored.next_sibling(1), Some(3));
        assert_eq!(restored.parent(1), Some(0));
        // subtree_size(0) = (find_close(0) - 0) / 2 = (5 - 0) / 2 = 2
        assert_eq!(restored.subtree_size(0), Some(2));
        // depth = excess at position, so depth(1) = excess(1) = 2 (two opens seen)
        assert_eq!(restored.depth(1), Some(2));
        assert_eq!(restored.excess(6), 0);
    }

    #[test]
    fn test_multi_word_bp() {
        // Create a larger BP spanning multiple words
        let mut words = vec![u64::MAX; 4]; // 128 opens
        words.extend(vec![0u64; 4]); // 128 closes
        let bp = BalancedParens::new(words, 512);

        let json = serde_json::to_string(&bp).unwrap();
        let restored: BalancedParens = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 512);

        // Test find_close across words
        assert_eq!(restored.find_close(0), Some(511));
        assert_eq!(restored.find_close(127), Some(384));
    }
}

// ============================================================================
// JSON SemiIndex serialization tests
// ============================================================================

mod json_semi_index_serde {
    use super::*;

    #[test]
    fn test_simple_semi_index() {
        let json_bytes = br#"{"a":1}"#;
        let semi = simple::build_semi_index(json_bytes);

        let json = serde_json::to_string(&semi).unwrap();
        let restored: simple::SemiIndex = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.state, semi.state);
        assert_eq!(restored.ib, semi.ib);
        assert_eq!(restored.bp, semi.bp);
    }

    #[test]
    fn test_standard_semi_index() {
        let json_bytes = br#"{"name":"test","value":123,"flag":true}"#;
        let semi = standard::build_semi_index(json_bytes);

        let json = serde_json::to_string(&semi).unwrap();
        let restored: standard::SemiIndex = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.state, semi.state);
        assert_eq!(restored.ib, semi.ib);
        assert_eq!(restored.bp, semi.bp);
    }

    #[test]
    fn test_nested_json_semi_index() {
        let json_bytes = br#"{"data":{"items":[1,2,3],"nested":{"deep":true}}}"#;
        let semi = standard::build_semi_index(json_bytes);

        let json = serde_json::to_string(&semi).unwrap();
        let restored: standard::SemiIndex = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.state, semi.state);
        assert_eq!(restored.ib, semi.ib);
        assert_eq!(restored.bp, semi.bp);
    }

    #[test]
    fn test_semi_index_states() {
        // Test that different states serialize correctly
        let incomplete_string = br#"{"key":"value"#; // ends in string
        let semi = simple::build_semi_index(incomplete_string);
        assert_eq!(semi.state, simple::State::InString);

        let json = serde_json::to_string(&semi).unwrap();
        let restored: simple::SemiIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.state, simple::State::InString);

        // Test InValue state
        let incomplete_value = br#"{"key":123"#;
        let semi = standard::build_semi_index(incomplete_value);
        assert_eq!(semi.state, standard::State::InValue);

        let json = serde_json::to_string(&semi).unwrap();
        let restored: standard::SemiIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.state, standard::State::InValue);
    }
}

// ============================================================================
// Config serialization tests
// ============================================================================

mod config_serde {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();

        let json = serde_json::to_string(&config).unwrap();
        let restored: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.select_sample_rate, 256);
        assert!(!restored.build_select0);
    }

    #[test]
    fn test_custom_config() {
        let config = Config {
            select_sample_rate: 64,
            build_select0: true,
        };

        let json = serde_json::to_string(&config).unwrap();
        let restored: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.select_sample_rate, 64);
        assert!(restored.build_select0);
    }
}

// ============================================================================
// Binary serialization tests (using bincode-like representation)
// ============================================================================

mod compact_serde {
    use super::*;

    #[test]
    fn test_bitvec_compact_roundtrip() {
        // Test that serialization produces reasonable output
        let words = vec![0b1010_1010u64; 4];
        let bv = BitVec::from_words(words, 256);

        let json = serde_json::to_string(&bv).unwrap();

        // Verify it's valid JSON
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));

        // Verify roundtrip
        let restored: BitVec = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.len(), bv.len());
        assert_eq!(restored.count_ones(), bv.count_ones());
    }

    #[test]
    fn test_bp_compact_roundtrip() {
        let bp = BalancedParens::new(vec![0x5555_5555_5555_5555u64; 4], 256);

        let json = serde_json::to_string(&bp).unwrap();
        let restored: BalancedParens = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), bp.len());

        // Verify some operations still work
        for p in (0..256).step_by(17) {
            if bp.is_open(p) {
                assert_eq!(
                    restored.find_close(p),
                    bp.find_close(p),
                    "find_close mismatch at {}",
                    p
                );
            }
        }
    }
}
