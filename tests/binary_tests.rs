//! Tests for binary serialization.

use succinctly::binary::{bytes_to_words, bytes_to_words_vec, try_bytes_to_words, words_to_bytes};
use succinctly::json::{simple, standard};

// ============================================================================
// Basic word/byte conversion tests
// ============================================================================

#[test]
fn test_empty_roundtrip() {
    let words: Vec<u64> = vec![];
    let bytes = words_to_bytes(&words);
    let recovered = bytes_to_words_vec(bytes);
    assert_eq!(words, recovered);
}

#[test]
fn test_single_word_roundtrip() {
    let words = vec![0xDEAD_BEEF_CAFE_BABEu64];
    let bytes = words_to_bytes(&words);
    let recovered = bytes_to_words_vec(bytes);
    assert_eq!(words, recovered);
}

#[test]
fn test_multiple_words_roundtrip() {
    let words: Vec<u64> = (0..100).map(|i| i * 0x0123_4567_89AB_CDEF).collect();
    let bytes = words_to_bytes(&words);
    let recovered = bytes_to_words_vec(bytes);
    assert_eq!(words, recovered);
}

#[test]
fn test_all_zeros() {
    let words = vec![0u64; 1000];
    let bytes = words_to_bytes(&words);
    let recovered = bytes_to_words_vec(bytes);
    assert_eq!(words, recovered);
}

#[test]
fn test_all_ones() {
    let words = vec![u64::MAX; 1000];
    let bytes = words_to_bytes(&words);
    let recovered = bytes_to_words_vec(bytes);
    assert_eq!(words, recovered);
}

#[test]
fn test_alternating_pattern() {
    let words = vec![0xAAAA_AAAA_AAAA_AAAAu64, 0x5555_5555_5555_5555u64];
    let bytes = words_to_bytes(&words);
    let recovered = bytes_to_words_vec(bytes);
    assert_eq!(words, recovered);
}

#[test]
fn test_try_bytes_valid() {
    let bytes = [0u8; 64];
    assert!(try_bytes_to_words(&bytes).is_some());
    assert_eq!(try_bytes_to_words(&bytes).unwrap().len(), 8);
}

#[test]
fn test_try_bytes_invalid() {
    let bytes = [0u8; 7];
    assert!(try_bytes_to_words(&bytes).is_none());
}

#[test]
#[should_panic(expected = "must be a multiple of 8")]
fn test_bytes_to_words_invalid_length() {
    let bytes = [0u8; 13];
    let _ = bytes_to_words(&bytes);
}

// ============================================================================
// JSON semi-index binary serialization tests
// ============================================================================

#[test]
fn test_simple_semi_index_roundtrip() {
    let json = br#"{"name":"test","value":42}"#;
    let semi = simple::build_semi_index(json);

    let ib_bytes = semi.ib_as_bytes();
    let bp_bytes = semi.bp_as_bytes();

    let restored = simple::SemiIndex::from_bytes(ib_bytes, bp_bytes);

    assert_eq!(semi.ib, restored.ib);
    assert_eq!(semi.bp, restored.bp);
}

#[test]
fn test_standard_semi_index_roundtrip() {
    let json = br#"{"name":"test","values":[1,2,3],"active":true}"#;
    let semi = standard::build_semi_index(json);

    let ib_bytes = semi.ib_as_bytes();
    let bp_bytes = semi.bp_as_bytes();

    let restored = standard::SemiIndex::from_bytes(ib_bytes, bp_bytes);

    assert_eq!(semi.ib, restored.ib);
    assert_eq!(semi.bp, restored.bp);
}

#[test]
fn test_large_json_roundtrip() {
    // Create a larger JSON to span multiple words
    let json = format!(
        r#"{{"items":[{}]}}"#,
        (0..1000)
            .map(|i| format!(r#"{{"id":{},"name":"item{}"}}"#, i, i))
            .collect::<Vec<_>>()
            .join(",")
    );
    let json_bytes = json.as_bytes();

    let semi = standard::build_semi_index(json_bytes);

    let ib_bytes = semi.ib_as_bytes();
    let bp_bytes = semi.bp_as_bytes();

    // Verify byte lengths are multiples of 8
    assert_eq!(ib_bytes.len() % 8, 0);
    assert_eq!(bp_bytes.len() % 8, 0);

    let restored = standard::SemiIndex::from_bytes(ib_bytes, bp_bytes);

    assert_eq!(semi.ib, restored.ib);
    assert_eq!(semi.bp, restored.bp);
}

#[test]
fn test_empty_json_object() {
    let json = b"{}";
    let semi = simple::build_semi_index(json);

    let ib_bytes = semi.ib_as_bytes();
    let bp_bytes = semi.bp_as_bytes();

    // Even small data produces at least one word (8 bytes)
    assert!(ib_bytes.len() >= 8 || ib_bytes.is_empty());

    let restored = simple::SemiIndex::from_bytes(ib_bytes, bp_bytes);
    assert_eq!(semi.ib, restored.ib);
    assert_eq!(semi.bp, restored.bp);
}

#[test]
fn test_deeply_nested_json() {
    // Create deeply nested JSON
    let depth = 50;
    let opens: String = "[".repeat(depth);
    let closes: String = "]".repeat(depth);
    let json = format!("{}1{}", opens, closes);

    let semi = standard::build_semi_index(json.as_bytes());

    let ib_bytes = semi.ib_as_bytes();
    let bp_bytes = semi.bp_as_bytes();

    let restored = standard::SemiIndex::from_bytes(ib_bytes, bp_bytes);

    assert_eq!(semi.ib, restored.ib);
    assert_eq!(semi.bp, restored.bp);
}

// ============================================================================
// File I/O tests (with std)
// ============================================================================

#[test]
fn test_write_read_file() {
    use std::fs;
    use std::io::Write;

    let json = br#"{"test":true}"#;
    let semi = simple::build_semi_index(json);

    let dir = std::env::temp_dir();
    let ib_path = dir.join("test_ib.bin");
    let bp_path = dir.join("test_bp.bin");

    // Write
    let mut ib_file = fs::File::create(&ib_path).unwrap();
    ib_file.write_all(semi.ib_as_bytes()).unwrap();

    let mut bp_file = fs::File::create(&bp_path).unwrap();
    bp_file.write_all(semi.bp_as_bytes()).unwrap();

    // Read
    let ib_bytes = fs::read(&ib_path).unwrap();
    let bp_bytes = fs::read(&bp_path).unwrap();

    let restored = simple::SemiIndex::from_bytes(&ib_bytes, &bp_bytes);

    assert_eq!(semi.ib, restored.ib);
    assert_eq!(semi.bp, restored.bp);

    // Cleanup
    let _ = fs::remove_file(&ib_path);
    let _ = fs::remove_file(&bp_path);
}

// ============================================================================
// Memory-mapped tests (feature-gated)
// ============================================================================

#[cfg(feature = "mmap-tests")]
mod mmap_tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use succinctly::binary::mmap::MmapWords;

    #[test]
    fn test_mmap_words_open() {
        let words = vec![0x1234_5678_9ABC_DEF0u64; 100];
        let bytes = words_to_bytes(&words);

        let dir = std::env::temp_dir();
        let path = dir.join("test_mmap.bin");

        // Write file
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        drop(file);

        // Memory map
        let mmap = MmapWords::open(&path).unwrap();

        assert_eq!(mmap.len(), 100);
        assert_eq!(mmap.words(), &words[..]);

        // Cleanup
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_mmap_json_index() {
        let json = br#"{"data":[1,2,3,4,5]}"#;
        let semi = standard::build_semi_index(json);

        let dir = std::env::temp_dir();
        let ib_path = dir.join("test_mmap_ib.bin");
        let bp_path = dir.join("test_mmap_bp.bin");

        // Write index files
        fs::write(&ib_path, semi.ib_as_bytes()).unwrap();
        fs::write(&bp_path, semi.bp_as_bytes()).unwrap();

        // Memory map and verify
        let ib_mmap = MmapWords::open(&ib_path).unwrap();
        let bp_mmap = MmapWords::open(&bp_path).unwrap();

        assert_eq!(ib_mmap.words(), &semi.ib[..]);
        assert_eq!(bp_mmap.words(), &semi.bp[..]);

        // Cleanup
        let _ = fs::remove_file(&ib_path);
        let _ = fs::remove_file(&bp_path);
    }

    #[test]
    fn test_mmap_invalid_size() {
        let dir = std::env::temp_dir();
        let path = dir.join("test_invalid_size.bin");

        // Write file with non-multiple-of-8 size
        fs::write(&path, [0u8; 13]).unwrap();

        // Should fail to open
        let result = MmapWords::open(&path);
        assert!(result.is_err());

        // Cleanup
        let _ = fs::remove_file(&path);
    }
}
