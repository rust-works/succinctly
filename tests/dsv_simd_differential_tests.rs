//! Cross-backend differential tests for DSV SIMD index construction
//! (#149, belongs with #182).
//!
//! The scalar reference parser (`src/dsv/parser.rs`) and every SIMD backend
//! must build the same index for any input: quote state toggles on every quote
//! byte, and delimiters/newlines are marked only outside quotes.
//!
//! The critical case is a quote at bit offset 63 of a 64-bit chunk: the
//! chunk-boundary carry must propagate, or an embedded delimiter/newline in the
//! next chunk leaks out of the quoted span. The pre-existing per-backend
//! `test_quoted_spanning_chunks` only places a quote at offset 0 — which is
//! exactly why the bit-63 bug slipped through.
//!
//! The SIMD backends fall into two families:
//!
//! * **prefix-xor** (`neon`, `sse2`, `avx2`) — track quote state with a running
//!   `in_quote` flag. These have always agreed with scalar, including at bit 63.
//! * **toggle64** (`bmi2` PDEP, `sve2` BDEP) — historically derived the
//!   chunk-boundary carry from the adder's overflow, which dropped a quote
//!   opening at bit 63 (`addend << 1` loses the top bit) — bug #149 (with
//!   #182). The carry now comes from quote-count parity, and these backends are
//!   asserted against scalar exactly like the prefix-xor family, plus a named
//!   #149 regression sweep at the bottom.
//!
//! On aarch64 hosts the direct assertions run scalar vs NEON (SVE2 where
//! available). On x86 CI they run scalar vs SSE2/AVX2/BMI2. Absent CPU features
//! print a `SKIPPED` line so a fully-skipped run does not read as "passed". See
//! #193 (x86) / #194 (SVE2).

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use succinctly::dsv::{build_index_scalar, DsvConfig, DsvIndex};

type Builder = fn(&[u8], &DsvConfig) -> DsvIndex;

/// The prefix-xor family (running `in_quote` flag). These have always agreed
/// with the scalar reference, including at the bit-63 chunk boundary.
fn prefix_xor_backends() -> Vec<(&'static str, Builder)> {
    #[allow(unused_mut)]
    let mut backends: Vec<(&'static str, Builder)> = Vec::new();

    #[cfg(target_arch = "aarch64")]
    {
        // NEON is mandatory on aarch64, so it always runs.
        backends.push(("neon", succinctly::dsv::simd::neon::build_index_simd));
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
    }

    backends
}

/// The `toggle64`-based backends (BMI2 PDEP / SVE2 BDEP). Their shared carry
/// formula used to drop a quote opening at bit 63 (#149, with #182); the carry
/// is now derived from quote-count parity and they must agree with scalar
/// everywhere, like the prefix-xor family.
fn toggle64_backends() -> Vec<(&'static str, Builder)> {
    #[allow(unused_mut)]
    let mut backends: Vec<(&'static str, Builder)> = Vec::new();

    #[cfg(target_arch = "aarch64")]
    {
        // SVE2-BITPERM is absent on Apple Silicon; validated on the
        // Neoverse-N2 CI runner and locally under emulation via
        // scripts/test-sve2-qemu.sh (#194). `build_index_sve2` uses BDEP, so
        // this gates on the exact `sve2-bitperm` feature it requires.
        if std::arch::is_aarch64_feature_detected!("sve2-bitperm") {
            backends.push(("sve2", succinctly::dsv::simd::sve2::build_index_simd));
        } else {
            eprintln!(
                "SKIPPED dsv toggle64 differential [sve2]: sve2-bitperm not detected (see #194)"
            );
        }
    }

    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("bmi2") {
            backends.push(("bmi2", succinctly::dsv::simd::bmi2::build_index_simd));
        } else {
            eprintln!("SKIPPED dsv toggle64 differential [bmi2]: bmi2 not detected (see #193)");
        }
    }

    backends
}

/// Every SIMD backend available on this host, both families.
fn all_backends() -> Vec<(&'static str, Builder)> {
    let mut backends = prefix_xor_backends();
    backends.extend(toggle64_backends());
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

/// Assert every backend in `backends` builds the same index as the scalar
/// reference for `text` under `config`.
fn assert_backends_match_scalar(
    backends: &[(&'static str, Builder)],
    label: &str,
    text: &[u8],
    config: &DsvConfig,
) {
    let scalar = build_index_scalar(text, config);
    let scalar_sig = index_sig(&scalar, text.len());
    for (name, build) in backends {
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
    // Detect backends once so a missing feature prints one SKIPPED line per
    // test rather than one per assertion (#193).
    let backends = all_backends();
    let config = DsvConfig::default();
    // Opening quote at every byte offset across three 64-byte chunk boundaries.
    for open in 0..192usize {
        let text = spanning_quote_input(open);
        assert_backends_match_scalar(&backends, &format!("open={open}"), &text, &config);
    }
}

#[test]
fn test_bit63_carry_regression() {
    // A quote at offset 63 (last bit of chunk 0) must set the carry so the
    // delimiter at offset 64 (first bit of chunk 1) stays masked. Also check the
    // neighboring boundary bits 62/64 and the next boundary at 127/128. This is
    // the exact case the toggle64 overflow carry used to get wrong (#149).
    let backends = all_backends();
    let config = DsvConfig::default();
    for open in [62usize, 63, 64, 65, 126, 127, 128, 129] {
        let mut text = vec![b'a'; open + 80];
        text[open] = b'"'; // opens quoted span at/near the chunk boundary
        text[open + 1] = b','; // delimiter immediately after -> masked
        text[open + 7] = b'\n'; // newline inside quotes -> masked
        text[open + 40] = b'"'; // closes the span in a later chunk
        text[open + 41] = b','; // real delimiter, outside quotes
        assert_backends_match_scalar(&backends, &format!("boundary open={open}"), &text, &config);
    }
}

#[test]
fn test_random_csv_matches_scalar() {
    // Deterministic fuzz: randomized bytes over an alphabet of ordinary chars
    // plus every special byte used by the configs below (delimiters, newline,
    // quote, CR). Chunk-spanning quoted regions arise naturally.
    let backends = all_backends();
    let mut rng = ChaCha8Rng::seed_from_u64(0x9E37_79B9_7F4A_7C15);
    let alphabet = *b"ab,\t;\n\r\" ";
    let configs = [
        DsvConfig::default(),
        DsvConfig::tsv(),
        DsvConfig::default().with_delimiter(b';'),
    ];

    for _ in 0..3000 {
        let len = rng.random_range(0..300);
        let text: Vec<u8> = (0..len)
            .map(|_| alphabet[rng.random_range(0..alphabet.len())])
            .collect();
        let config = &configs[rng.random_range(0..configs.len())];
        assert_backends_match_scalar(&backends, "fuzz", &text, config);
    }
}

/// Named regression sweep for #149: the toggle64 (BMI2 PDEP / SVE2 BDEP) carry
/// formula used to drop a quote opening at bit 63, so bmi2/sve2 disagreed with
/// scalar at the chunk boundary. Kept as a dedicated test (in addition to the
/// `all_backends()` sweeps above) so a reintroduction fails with #149 in the
/// test name.
#[test]
fn test_toggle64_backends_match_scalar_149() {
    let backends = toggle64_backends();
    if backends.is_empty() {
        eprintln!("SKIPPED dsv toggle64 differential: no toggle64 backend on this host");
        return;
    }
    let config = DsvConfig::default();
    for open in 0..192usize {
        let text = spanning_quote_input(open);
        assert_backends_match_scalar(&backends, &format!("open={open}"), &text, &config);
    }
}
