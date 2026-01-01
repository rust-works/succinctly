//! Binary serialization for succinct data structures.
//!
//! This module provides zero-copy binary serialization compatible with memory-mapped files.
//! The format is simple: raw little-endian u64 words with no header.
//!
//! ## Format
//!
//! - **Bit vectors**: Raw `[u64]` as little-endian bytes. Length = file_size / 8 words.
//! - **JSON semi-index**: Two separate files for IB and BP vectors.
//!
//! ## Example
//!
//! ```
//! use succinctly::binary;
//!
//! // Convert words to bytes for storage
//! let words = vec![0b1010_1010u64; 4];
//! let bytes = binary::words_to_bytes(&words);
//! assert_eq!(bytes.len(), 32); // 4 words * 8 bytes
//!
//! // Convert bytes back to words
//! let loaded = binary::bytes_to_words_vec(bytes);
//! assert_eq!(words, loaded);
//! ```

#[cfg(not(test))]
use alloc::vec::Vec;

use bytemuck::cast_slice;

/// Write a slice of u64 words to raw bytes.
///
/// The output is the raw little-endian representation of the words.
/// No header or length prefix is written - the length is implicit from the byte count.
#[inline]
pub fn words_to_bytes(words: &[u64]) -> &[u8] {
    cast_slice(words)
}

/// Read u64 words from raw bytes.
///
/// The byte slice length must be a multiple of 8.
///
/// # Panics
///
/// Panics if `bytes.len()` is not a multiple of 8.
#[inline]
pub fn bytes_to_words(bytes: &[u8]) -> &[u64] {
    if bytes.is_empty() {
        return &[];
    }
    assert!(
        bytes.len() % 8 == 0,
        "byte slice length must be a multiple of 8, got {}",
        bytes.len()
    );
    cast_slice(bytes)
}

/// Read u64 words from raw bytes, returning owned Vec.
///
/// The byte slice length must be a multiple of 8.
///
/// # Panics
///
/// Panics if `bytes.len()` is not a multiple of 8.
pub fn bytes_to_words_vec(bytes: &[u8]) -> Vec<u64> {
    bytes_to_words(bytes).to_vec()
}

/// Try to read u64 words from raw bytes.
///
/// Returns `None` if `bytes.len()` is not a multiple of 8.
#[inline]
pub fn try_bytes_to_words(bytes: &[u8]) -> Option<&[u64]> {
    if bytes.is_empty() {
        return Some(&[]);
    }
    if bytes.len() % 8 == 0 {
        Some(cast_slice(bytes))
    } else {
        None
    }
}

/// Memory-mapped file support for zero-copy access.
///
/// This module requires the `memmap2` feature and implicitly enables std.
#[cfg(feature = "memmap2")]
pub mod mmap {
    extern crate std;

    use memmap2::Mmap;
    use std::fs::File;
    use std::io;
    use std::path::Path;

    /// A memory-mapped word vector.
    ///
    /// Provides zero-copy access to u64 words stored in a file.
    pub struct MmapWords {
        mmap: Mmap,
    }

    impl MmapWords {
        /// Open a file and memory-map it.
        ///
        /// # Safety
        ///
        /// The file must not be modified while the mapping is active.
        pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
            let file = File::open(path)?;
            let mmap = unsafe { Mmap::map(&file)? };

            if mmap.len() % 8 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "file size is not a multiple of 8",
                ));
            }

            Ok(Self { mmap })
        }

        /// Get the words as a slice.
        #[inline]
        pub fn words(&self) -> &[u64] {
            super::bytes_to_words(&self.mmap)
        }

        /// Get the number of words.
        #[inline]
        pub fn len(&self) -> usize {
            self.mmap.len() / 8
        }

        /// Check if empty.
        #[inline]
        pub fn is_empty(&self) -> bool {
            self.mmap.is_empty()
        }

        /// Get the underlying bytes.
        #[inline]
        pub fn bytes(&self) -> &[u8] {
            &self.mmap
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_words_to_bytes_empty() {
        let words: &[u64] = &[];
        let bytes = words_to_bytes(words);
        assert!(bytes.is_empty());
    }

    #[test]
    fn test_words_to_bytes_single() {
        let words = [0x0123_4567_89AB_CDEFu64];
        let bytes = words_to_bytes(&words);
        assert_eq!(bytes.len(), 8);
        // Little-endian: least significant byte first
        assert_eq!(bytes, &[0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01]);
    }

    #[test]
    fn test_words_to_bytes_multiple() {
        let words = [0x0000_0000_0000_0001u64, 0x0000_0000_0000_0002u64];
        let bytes = words_to_bytes(&words);
        assert_eq!(bytes.len(), 16);
        assert_eq!(
            bytes,
            &[
                0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // first word
                0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // second word
            ]
        );
    }

    #[test]
    fn test_bytes_to_words_empty() {
        let bytes: &[u8] = &[];
        let words = bytes_to_words(bytes);
        assert!(words.is_empty());
    }

    #[test]
    fn test_bytes_to_words_single() {
        let bytes = [0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23, 0x01];
        let words = bytes_to_words(&bytes);
        assert_eq!(words, &[0x0123_4567_89AB_CDEFu64]);
    }

    #[test]
    fn test_roundtrip() {
        let original = vec![
            0x1111_1111_1111_1111u64,
            0x2222_2222_2222_2222,
            0x3333_3333_3333_3333,
            0xFFFF_FFFF_FFFF_FFFF,
            0x0000_0000_0000_0000,
        ];
        let bytes = words_to_bytes(&original);
        let recovered = bytes_to_words_vec(bytes);
        assert_eq!(original, recovered);
    }

    #[test]
    #[should_panic(expected = "must be a multiple of 8")]
    fn test_bytes_to_words_invalid_length() {
        let bytes = [0u8; 7];
        let _ = bytes_to_words(&bytes);
    }

    #[test]
    fn test_try_bytes_to_words_valid() {
        let bytes = [0u8; 16];
        assert!(try_bytes_to_words(&bytes).is_some());
    }

    #[test]
    fn test_try_bytes_to_words_invalid() {
        let bytes = [0u8; 7];
        assert!(try_bytes_to_words(&bytes).is_none());
    }
}
