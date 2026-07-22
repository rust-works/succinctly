//! Regression and property tests for DSV lightweight select1 (issue #196).
//!
//! `markers_select1`/`newlines_select1` underflowed (`k - rank_before`) when a
//! 64-bit word contained no markers: the cumulative rank array then holds
//! duplicate values, and `binary_search` returns an arbitrary index among
//! equal elements, which can land one word past the one containing the k-th
//! bit.

use proptest::prelude::*;
use succinctly::dsv::{build_index_scalar, DsvConfig};

/// Positions of set bits in `words`, restricted to `[0, len)`.
fn set_bit_positions(words: &[u64], len: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for (w, &word) in words.iter().enumerate() {
        let mut bits = word;
        while bits != 0 {
            let pos = w * 64 + bits.trailing_zeros() as usize;
            if pos < len {
                out.push(pos);
            }
            bits &= bits - 1;
        }
    }
    out
}

/// The exact reproducer from issue #196: marker in word 0, marker-free word 1,
/// markers in word 2, yielding `markers_rank == [0, 1, 1, 3]`.
#[test]
fn select1_zero_marker_word_reproducer_196() {
    let mut text = vec![b'a'; 130];
    text[0] = b',';
    text[128] = b',';
    text[129] = b'\n';
    let idx = build_index_scalar(&text, &DsvConfig::default());

    assert_eq!(idx.markers_select1(0), Some(0));
    assert_eq!(idx.markers_select1(1), Some(128));
    assert_eq!(idx.markers_select1(2), Some(129));
    assert_eq!(idx.markers_select1(3), None);

    assert_eq!(idx.newlines_select1(0), Some(129));
    assert_eq!(idx.newlines_select1(1), None);
}

proptest! {
    /// select1 agrees with a naive scan of the index's bit words, and
    /// round-trips through rank1, on texts biased toward long marker-free
    /// runs (so zero-marker words are common).
    #[test]
    fn select1_matches_naive_scan_and_rank1_roundtrip(
        text in prop::collection::vec(
            prop_oneof![
                100 => Just(b'a'),
                1 => Just(b','),
                1 => Just(b'\n'),
                1 => Just(b'"'),
            ],
            0..512,
        )
    ) {
        let idx = build_index_scalar(&text, &DsvConfig::default());
        let lw = idx.as_lightweight();

        let markers = set_bit_positions(&lw.markers, text.len());
        prop_assert_eq!(idx.marker_count(), markers.len());
        for (k, &pos) in markers.iter().enumerate() {
            prop_assert_eq!(idx.markers_select1(k), Some(pos));
            prop_assert_eq!(idx.markers_rank1(pos + 1), k + 1);
        }
        prop_assert_eq!(idx.markers_select1(markers.len()), None);

        let newlines = set_bit_positions(&lw.newlines, text.len());
        prop_assert_eq!(idx.row_count(), newlines.len());
        for (k, &pos) in newlines.iter().enumerate() {
            prop_assert_eq!(idx.newlines_select1(k), Some(pos));
            prop_assert_eq!(idx.newlines_rank1(pos + 1), k + 1);
        }
        prop_assert_eq!(idx.newlines_select1(newlines.len()), None);
    }
}
