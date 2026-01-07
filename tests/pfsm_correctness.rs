use std::fs;
/// Correctness tests: Verify PFSM produces identical output to standard implementations
use succinctly::json::{pfsm, pfsm_tables::PfsmState, standard, BitWriter};

#[test]
fn test_pfsm_vs_standard_10kb() {
    verify_pfsm_correctness("data/bench/generated/comprehensive/10kb.json");
}

#[test]
fn test_pfsm_vs_standard_100kb() {
    verify_pfsm_correctness("data/bench/generated/comprehensive/100kb.json");
}

#[test]
fn test_pfsm_vs_standard_1mb() {
    verify_pfsm_correctness("data/bench/generated/comprehensive/1mb.json");
}

#[test]
fn test_pfsm_vs_standard_users() {
    verify_pfsm_correctness("data/bench/generated/users/10kb.json");
}

#[test]
fn test_pfsm_vs_standard_nested() {
    verify_pfsm_correctness("data/bench/generated/nested/10kb.json");
}

#[test]
fn test_pfsm_vs_standard_arrays() {
    verify_pfsm_correctness("data/bench/generated/arrays/10kb.json");
}

fn verify_pfsm_correctness(path: &str) {
    let json = fs::read(path).expect("Failed to read test file");

    // Generate PFSM output
    let mut ib_pfsm = BitWriter::new();
    let mut bp_pfsm = BitWriter::new();
    pfsm::pfsm_process_chunk(&json, PfsmState::InJson, &mut ib_pfsm, &mut bp_pfsm);
    let (ib_pfsm_bits, bp_pfsm_bits) = (ib_pfsm.finish(), bp_pfsm.finish());

    // Generate standard output
    let standard_index = standard::build_semi_index(&json);
    let ib_standard_bits = &standard_index.ib;
    let bp_standard_bits = &standard_index.bp;

    // Compare IB bitvectors
    assert_eq!(
        ib_pfsm_bits.len(),
        ib_standard_bits.len(),
        "IB bitvector length mismatch in {}",
        path
    );

    for (i, (pfsm_word, std_word)) in ib_pfsm_bits.iter().zip(ib_standard_bits.iter()).enumerate() {
        assert_eq!(
            pfsm_word, std_word,
            "IB bitvector mismatch at word {} in {}: PFSM={:064b}, Standard={:064b}",
            i, path, pfsm_word, std_word
        );
    }

    // Compare BP bitvectors
    assert_eq!(
        bp_pfsm_bits.len(),
        bp_standard_bits.len(),
        "BP bitvector length mismatch in {}",
        path
    );

    for (i, (pfsm_word, std_word)) in bp_pfsm_bits.iter().zip(bp_standard_bits.iter()).enumerate() {
        assert_eq!(
            pfsm_word, std_word,
            "BP bitvector mismatch at word {} in {}: PFSM={:064b}, Standard={:064b}",
            i, path, pfsm_word, std_word
        );
    }

    println!(
        "✓ {} - IB: {} words, BP: {} words match exactly",
        path,
        ib_pfsm_bits.len(),
        bp_pfsm_bits.len()
    );
}
