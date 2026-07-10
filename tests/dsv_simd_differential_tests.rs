//! Cross-backend differential tests for DSV SIMD index construction
//! (#149, belongs with #182).
//!
//! The scalar reference parser (`src/dsv/parser.rs`) and every SIMD backend
//! implement the *same* quote semantics: quote state toggles on every quote
//! byte, and delimiters/newlines are marked only outside quotes. So for any
//! input the scalar index and each SIMD index must be identical.
//!
//! The critical case is a quote at bit offset 63 of a 64-bit chunk: the
//! chunk-boundary carry must propagate, or an embedded delimiter/newline in the
//! next chunk leaks out of the quoted span (bug #149). The pre-existing
//! per-backend `test_quoted_spanning_chunks` only places a quote at offset 0 —
//! which is exactly why the bit-63 bug slipped through. Here we sweep a quote
//! across *every* offset and fuzz randomized CSV.
//!
//! On this aarch64 host the sweep exercises scalar vs NEON (and SVE2 when
//! detected). The x86 backends (SSE2/AVX2/BMI2) run these same assertions on
//! x86 CI — tracked in #193; SVE2 under emulation — tracked in #194. Backends
//! that are compiled but whose hardware feature is absent print a `SKIPPED`
//! line so a fully-skipped run does not read as "passed".

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use succinctly::dsv::{build_index_scalar, DsvConfig, DsvIndex};

type Builder = fn(&[u8], &DsvConfig) -> DsvIndex;

/// SIMD backends compiled for this target whose required CPU features are
/// present. Absent features are reported as `SKIPPED` (see #193 / #194).
fn simd_backends() -> Vec<(&'static str, Builder)> {
    #[allow(unused_mut)]
    let mut backends: Vec<(&'static str, Builder)> = Vec::new();

    #[cfg(target_arch = "aarch64")]
    {
        // NEON is mandatory on aarch64, so it always runs.
        backends.push(("neon", succinctly::dsv::simd::neon::build_index_simd));
        // SVE2 does not exist on Apple Silicon; validated under emulation (#194).
        // NOTE: #194 should tighten this to the exact `sve2-bitperm` feature.
        if std::arch::is_aarch64_feature_detected!("sve2") {
            backends.push(("sve2", succinctly::dsv::simd::sve2::build_index_simd));
        } else {
            eprintln!("SKIPPED dsv differential [sve2]: sve2 not detected (see #194)");
        }
    }

    #[cfg(target_arch = "x86_64")]
    {
        // SSE2 is baseline on x86_64, so it always runs.
        backends.push(("sse2", succinctly::dsv::simd::sse2::build_index_simd));
        if std::arch::is_x86_feature_detected!("avx2") {
            backends.push(("avx2", succinctly::dsv::simd::avx2::build_index_simd));
        } else {
            eprintln!("SKIPPED dsv differential [avx2]: avx2 not detected (see #193)");
        }
        if std::arch::is_x86_feature_detected!("bmi2") {
            backends.push(("bmi2", succinctly::dsv::simd::bmi2::build_index_simd));
        } else {
            eprintln!("SKIPPED dsv differential [bmi2]: bmi2 not detected (see #193)");
        }
    }

    backends
}

/// A semantic signature of an index: the cumulative marker and newline rank at
/// every position in `[0, len]`. This fully determines both bitvectors over the
/// text (two indices have the same signature iff they mark the same bytes),
/// ignores don't-care padding bits in the final word, and — unlike `select1`
/// based navigation — is O(1) per position and side-steps unrelated navigation
/// paths.
fn index_sig(idx: &DsvIndex, len: usize) -> (Vec<usize>, Vec<usize>) {
    let markers = (0..=len).map(|i| idx.markers_rank1(i)).collect();
    let newlines = (0..=len).map(|i| idx.newlines_rank1(i)).collect();
    (markers, newlines)
}

/// Assert every available SIMD backend builds the same index as the scalar
/// reference for `text` under `config`.
fn assert_matches_scalar(label: &str, text: &[u8], config: &DsvConfig) {
    let scalar = build_index_scalar(text, config);
    let scalar_sig = index_sig(&scalar, text.len());
    for (name, build) in simd_backends() {
        let simd = build(text, config);
        assert_eq!(
            index_sig(&simd, text.len()),
            scalar_sig,
            "{name}/{label}: index differs from scalar for input {:?}",
            String::from_utf8_lossy(text)
        );
    }
}

/// Build an input whose quoted span opens at byte `open`, is longer than one
/// 64-byte chunk (so it always crosses a boundary), and swallows delimiters and
/// newlines that must therefore be masked.
fn spanning_quote_input(open: usize) -> Vec<u8> {
    const SPAN: usize = 70; // > 64 => the quoted region crosses a chunk boundary
    let total = open + SPAN + 8;
    let mut buf = vec![b'a'; total];
    // Sprinkle delimiters and newlines everywhere so masking is exercised at
    // and around the chunk boundary.
    for i in (0..total).step_by(5) {
        buf[i] = b',';
    }
    for i in (0..total).step_by(17) {
        buf[i] = b'\n';
    }
    buf[open] = b'"'; // opens the quoted span
    buf[open + SPAN] = b'"'; // closes it in a later chunk
    buf
}

#[test]
fn test_quote_at_every_offset_matches_scalar() {
    let config = DsvConfig::default();
    // Opening quote at every byte offset across three 64-byte chunk boundaries.
    for open in 0..192usize {
        let text = spanning_quote_input(open);
        assert_matches_scalar(&format!("open={open}"), &text, &config);
    }
}

#[test]
fn test_bit63_carry_regression() {
    // Regression for #149: a quote at offset 63 (last bit of chunk 0) must set
    // the carry so the delimiter at offset 64 (first bit of chunk 1) stays
    // masked. Also check the neighboring boundary bits 62/64 and the next
    // boundary at 127/128.
    let config = DsvConfig::default();
    for open in [62usize, 63, 64, 65, 126, 127, 128, 129] {
        let mut text = vec![b'a'; open + 80];
        text[open] = b'"'; // opens quoted span at/near the chunk boundary
        text[open + 1] = b','; // delimiter immediately after -> masked
        text[open + 7] = b'\n'; // newline inside quotes -> masked
        text[open + 40] = b'"'; // closes the span in a later chunk
        text[open + 41] = b','; // real delimiter, outside quotes
        assert_matches_scalar(&format!("boundary open={open}"), &text, &config);
    }
}

#[test]
fn test_random_csv_matches_scalar() {
    // Deterministic fuzz: randomized bytes over an alphabet of ordinary chars
    // plus every special byte used by the configs below (delimiters, newline,
    // quote, CR). Chunk-spanning quoted regions arise naturally.
    let mut rng = ChaCha8Rng::seed_from_u64(0x9E37_79B9_7F4A_7C15);
    let alphabet = [b'a', b'b', b',', b'\t', b';', b'\n', b'\r', b'"', b' '];
    let configs = [
        DsvConfig::default(),
        DsvConfig::tsv(),
        DsvConfig::default().with_delimiter(b';'),
    ];

    for _ in 0..3000 {
        let len = rng.gen_range(0..300);
        let text: Vec<u8> = (0..len)
            .map(|_| alphabet[rng.gen_range(0..alphabet.len())])
            .collect();
        let config = &configs[rng.gen_range(0..configs.len())];
        assert_matches_scalar("fuzz", &text, config);
    }
}
