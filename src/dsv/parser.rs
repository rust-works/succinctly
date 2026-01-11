//! Quote-aware DSV indexing algorithm.
//!
//! This module implements the core algorithm from hw-dsv that creates bit vectors
//! marking field delimiters and newlines, with proper handling of quoted fields.

use super::config::DsvConfig;
use super::index::DsvIndex;
use super::index_lightweight::DsvIndexLightweight;
use crate::json::BitWriter;

/// Build a DsvIndex from input text.
///
/// This function scans the text and creates two bit vectors:
/// - markers: bits set at delimiter and newline positions (outside quotes)
/// - newlines: bits set at newline positions (outside quotes)
///
/// The algorithm correctly handles quoted fields where delimiters and newlines
/// inside quotes are not treated as field/row boundaries.
pub fn build_index(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if text.is_empty() {
        return DsvIndex::new_lightweight(DsvIndexLightweight::new(vec![], vec![], 0));
    }

    let num_words = text.len().div_ceil(64);

    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);

    // Track quote state across the entire file
    let mut in_quote = false;

    // Process byte by byte (simpler than chunked approach, still fast)
    for &byte in text {
        let is_quote = byte == config.quote_char;
        let is_delimiter = byte == config.delimiter;
        let is_newline = byte == config.newline;

        // Toggle quote state on quote character
        if is_quote {
            in_quote = !in_quote;
        }

        // Mark delimiters and newlines only if outside quotes
        if !in_quote {
            if is_delimiter || is_newline {
                markers_writer.write_1();
            } else {
                markers_writer.write_0();
            }

            if is_newline {
                newlines_writer.write_1();
            } else {
                newlines_writer.write_0();
            }
        } else {
            markers_writer.write_0();
            newlines_writer.write_0();
        }
    }

    let markers_words = markers_writer.finish();
    let newlines_words = newlines_writer.finish();

    let lightweight = DsvIndexLightweight::new(markers_words, newlines_words, text.len());
    DsvIndex::new_lightweight(lightweight)
}

/// Build a DsvIndex using word-at-a-time processing (faster for large files).
///
/// This version processes 8 bytes at a time using broadword techniques,
/// which is significantly faster for large files.
#[allow(dead_code)]
pub fn build_index_fast(text: &[u8], config: &DsvConfig) -> DsvIndex {
    if text.is_empty() {
        return DsvIndex::new_lightweight(DsvIndexLightweight::new(vec![], vec![], 0));
    }

    let num_words = text.len().div_ceil(64);

    let mut markers_writer = BitWriter::with_capacity(num_words);
    let mut newlines_writer = BitWriter::with_capacity(num_words);
    let mut in_quote = false;

    // Process in 8-byte chunks
    let chunks = text.chunks(8);

    for chunk in chunks {
        let mut marker_byte: u8 = 0;
        let mut newline_byte: u8 = 0;

        for (i, &byte) in chunk.iter().enumerate() {
            let is_quote = byte == config.quote_char;
            let is_delimiter = byte == config.delimiter;
            let is_newline = byte == config.newline;

            if is_quote {
                in_quote = !in_quote;
            }

            if !in_quote {
                if is_delimiter || is_newline {
                    marker_byte |= 1 << i;
                }
                if is_newline {
                    newline_byte |= 1 << i;
                }
            }
        }

        markers_writer.write_bits(marker_byte as u64, chunk.len());
        newlines_writer.write_bits(newline_byte as u64, chunk.len());
    }

    let markers_words = markers_writer.finish();
    let newlines_words = newlines_writer.finish();

    let lightweight = DsvIndexLightweight::new(markers_words, newlines_words, text.len());
    DsvIndex::new_lightweight(lightweight)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_csv() {
        let csv = b"a,b,c\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        // Should have 3 markers: positions 1 (,), 3 (,), 5 (\n)
        assert_eq!(index.marker_count(), 3);
        // Should have 1 newline: position 5
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_quoted_delimiter() {
        let csv = b"\"a,b\",c\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        // The comma inside quotes should not be a marker
        // Only the comma after "a,b" and the newline should be markers
        assert_eq!(index.marker_count(), 2);
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_quoted_newline() {
        let csv = b"\"a\nb\",c\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        // The newline inside quotes should not be counted
        assert_eq!(index.row_count(), 1);
    }

    #[test]
    fn test_multiple_rows() {
        let csv = b"a,b\nc,d\ne,f\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        assert_eq!(index.row_count(), 3);
        // 3 commas + 3 newlines = 6 markers
        assert_eq!(index.marker_count(), 6);
    }

    #[test]
    fn test_empty() {
        let csv = b"";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        assert_eq!(index.row_count(), 0);
        assert_eq!(index.marker_count(), 0);
        assert!(index.is_empty());
    }

    #[test]
    fn test_fast_matches_simple() {
        let csv = b"a,b,c\nd,e,f\n\"g,h\",i\n";
        let config = DsvConfig::default();

        let index_simple = build_index(csv, &config);
        let index_fast = build_index_fast(csv, &config);

        assert_eq!(index_simple.marker_count(), index_fast.marker_count());
        assert_eq!(index_simple.row_count(), index_fast.row_count());
    }

    #[test]
    fn test_tsv() {
        let tsv = b"a\tb\tc\n";
        let config = DsvConfig::tsv();
        let index = build_index(tsv, &config);

        assert_eq!(index.marker_count(), 3); // 2 tabs + 1 newline
        assert_eq!(index.row_count(), 1);
    }
}
