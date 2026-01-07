//! Parallel Finite State Machine (PFSM) implementation for JSON parsing.
//!
//! Based on the haskellworks hw-json-simd table-driven approach.
//!
//! Processing happens in 3 stages:
//! 1. Sequential state machine with table lookups → phi values
//! 2. Batch extraction of IB/OP/CL bits from phi values (using PEXT on x86_64)
//! 3. BP construction using PDEP to interleave OP/CL bits

use crate::json::bit_writer::BitWriter;
use crate::json::pfsm_tables::{PfsmState, PHI_TABLE, TRANSITION_TABLE};

#[cfg(not(test))]
use alloc::vec::Vec;

/// Process a chunk of JSON using table-based PFSM
///
/// Returns the final state after processing all bytes
pub fn pfsm_process_chunk(
    json: &[u8],
    initial_state: PfsmState,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> PfsmState {
    // Stage 1: Compute phi values for all bytes (sequential state machine)
    let mut phi_values = Vec::with_capacity(json.len());
    let mut state = initial_state;

    for &byte in json {
        let transition_entry = TRANSITION_TABLE[byte as usize];
        let phi_entry = PHI_TABLE[byte as usize];

        // Extract phi for current state
        let phi = PfsmState::extract_phi(phi_entry, state);
        phi_values.push(phi);

        // Transition to next state
        state = PfsmState::extract_next_state(transition_entry, state);
    }

    // Stage 2 & 3: Extract bits and construct IB/BP
    // TODO: Re-implement BMI2/AVX2 path with correct BP extraction logic
    // The SIMD path needs to handle conditional BP bit writing:
    // - bp_open writes a 1 (not conditional on IB)
    // - bp_close writes a 0 (not conditional on IB)
    // This is more complex than the original "2 bits per IB" approach.
    // #[cfg(all(target_arch = "x86_64", feature = "std"))]
    // {
    //     if is_x86_feature_detected!("bmi2") && is_x86_feature_detected!("avx2") {
    //         unsafe {
    //             return pfsm_extract_bits_bmi2(&phi_values, state, ib, bp);
    //         }
    //     }
    // }

    // Scalar extraction (correct implementation)
    pfsm_extract_bits_scalar(&phi_values, state, ib, bp)
}

/// Scalar implementation: extract IB/BP bits from phi values
fn pfsm_extract_bits_scalar(
    phi_values: &[u8],
    final_state: PfsmState,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> PfsmState {
    for &phi in phi_values.iter() {
        // Extract bits from the phi value (matching src/json/standard.rs encoding)
        // Bit 0 (0b001): BP close
        // Bit 1 (0b010): BP open
        // Bit 2 (0b100): Interest bit
        let bp_close = phi & 1; // Bit 0
        let bp_open = (phi >> 1) & 1; // Bit 1
        let ib_bit = (phi >> 2) & 1; // Bit 2

        // Write IB bit
        ib.write_bit(ib_bit != 0);

        // Write BP bits independently (matching standard cursor logic)
        // Open paren: write 1, Close paren: write 0
        if bp_open != 0 {
            bp.write_1();
        }
        if bp_close != 0 {
            bp.write_0();
        }
    }

    final_state
}

/// BMI2 + AVX2 implementation: batch extract bits using PEXT/PDEP
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "bmi2,avx2")]
unsafe fn pfsm_extract_bits_bmi2(
    phi_values: &[u8],
    final_state: PfsmState,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> PfsmState {
    use core::arch::x86_64::{
        __m256i, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_slli_epi16, _pdep_u64, _pext_u64,
    };

    // Process 32 phi values at a time using AVX2 (32 bytes = 256 bits)
    let mut i = 0;
    while i + 32 <= phi_values.len() {
        unsafe {
            // Load 32 phi bytes directly
            let v_32 = _mm256_loadu_si256(phi_values[i..].as_ptr() as *const __m256i);

            // Extract bits using correct encoding:
            // Bit 0: BP_close, Bit 1: BP_open, Bit 2: IB

            // Extract IB bits (bit 2 from each of 32 phi bytes)
            // Shift left by 5 to move bit 2 to bit 7 (MSB), then use movemask
            let v_ib_32 = _mm256_slli_epi16(v_32, 5);
            let all_ibs = _mm256_movemask_epi8(v_ib_32) as u32;

            // Extract BP_open bits (bit 1)
            // Shift left by 6 to move bit 1 to bit 7
            let v_bp_open_32 = _mm256_slli_epi16(v_32, 6);
            let all_bp_opens = _mm256_movemask_epi8(v_bp_open_32) as u32;

            // Extract BP_close bits (bit 0)
            // Shift left by 7 to move bit 0 to bit 7
            let v_bp_close_32 = _mm256_slli_epi16(v_32, 7);
            let all_bp_closes = _mm256_movemask_epi8(v_bp_close_32) as u32;

            // Write all 32 IB bits
            ib.write_bits(all_ibs as u64, 32);

            // Construct BP using PDEP (interleave BP_open and BP_close)
            // Process in chunks since we have 32 bits
            let bp_open_lo = (all_bp_opens & 0xFFFF) as u16;
            let bp_open_hi = (all_bp_opens >> 16) as u16;
            let bp_close_lo = (all_bp_closes & 0xFFFF) as u16;
            let bp_close_hi = (all_bp_closes >> 16) as u16;

            // Interleave low 16 bits
            let bp_open_lo_interleaved = _pdep_u64(bp_open_lo as u64, 0x5555555555555555);
            let bp_close_lo_interleaved = _pdep_u64(bp_close_lo as u64, 0xAAAAAAAAAAAAAAAA);
            let bp_bits_lo = bp_open_lo_interleaved | bp_close_lo_interleaved;

            // Interleave high 16 bits
            let bp_open_hi_interleaved = _pdep_u64(bp_open_hi as u64, 0x5555555555555555);
            let bp_close_hi_interleaved = _pdep_u64(bp_close_hi as u64, 0xAAAAAAAAAAAAAAAA);
            let bp_bits_hi = bp_open_hi_interleaved | bp_close_hi_interleaved;

            // Extract the actual BP bits using PEXT (only where IB=1)
            let ib_lo = (all_ibs & 0xFFFF) as u16;
            let ib_hi = (all_ibs >> 16) as u16;

            let ib_mask_lo = _pdep_u64(ib_lo as u64, 0x5555555555555555)
                | _pdep_u64(ib_lo as u64, 0xAAAAAAAAAAAAAAAA);
            let ib_mask_hi = _pdep_u64(ib_hi as u64, 0x5555555555555555)
                | _pdep_u64(ib_hi as u64, 0xAAAAAAAAAAAAAAAA);

            let bp_extracted_lo = _pext_u64(bp_bits_lo, ib_mask_lo);
            let bp_extracted_hi = _pext_u64(bp_bits_hi, ib_mask_hi);

            // Count IB bits to know how many BP bits to write
            let ib_count_lo = ib_lo.count_ones() as usize;
            let ib_count_hi = ib_hi.count_ones() as usize;

            // Write BP bits (2 bits per IB)
            if ib_count_lo > 0 {
                bp.write_bits(bp_extracted_lo, ib_count_lo * 2);
            }
            if ib_count_hi > 0 {
                bp.write_bits(bp_extracted_hi, ib_count_hi * 2);
            }
        }

        i += 32;
    }

    // Handle remaining bytes with scalar (matching encoding)
    for &phi in &phi_values[i..] {
        let bp_close = phi & 1; // Bit 0
        let bp_open = (phi >> 1) & 1; // Bit 1
        let ib_bit = (phi >> 2) & 1; // Bit 2

        ib.write_bit(ib_bit != 0);

        if ib_bit != 0 {
            bp.write_bit(bp_open != 0);
            bp.write_bit(bp_close != 0);
        }
    }

    final_state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pfsm_simple_array() {
        let json = b"[1,2,3]";
        let mut ib = BitWriter::new();
        let mut bp = BitWriter::new();

        let final_state = pfsm_process_chunk(json, PfsmState::InJson, &mut ib, &mut bp);

        assert_eq!(final_state, PfsmState::InJson);

        let ib_bits = ib.finish();
        let bp_bits = bp.finish();

        assert!(!ib_bits.is_empty());
        assert!(!bp_bits.is_empty());
    }

    #[test]
    fn test_pfsm_empty_object() {
        let json = b"{}";
        let mut ib = BitWriter::new();
        let mut bp = BitWriter::new();

        let final_state = pfsm_process_chunk(json, PfsmState::InJson, &mut ib, &mut bp);

        assert_eq!(final_state, PfsmState::InJson);

        let ib_bits = ib.finish();
        let bp_bits = bp.finish();

        assert!(!ib_bits.is_empty());
        assert!(!bp_bits.is_empty());
    }

    #[test]
    fn test_pfsm_empty_array() {
        let json = b"[]";
        let mut ib = BitWriter::new();
        let mut bp = BitWriter::new();

        let final_state = pfsm_process_chunk(json, PfsmState::InJson, &mut ib, &mut bp);

        assert_eq!(final_state, PfsmState::InJson);

        let ib_bits = ib.finish();
        let bp_bits = bp.finish();

        assert!(!ib_bits.is_empty());
        assert!(!bp_bits.is_empty());
    }

    #[test]
    fn test_pfsm_simple_object() {
        let json = br#"{"a":1}"#;
        let mut ib = BitWriter::new();
        let mut bp = BitWriter::new();

        let final_state = pfsm_process_chunk(json, PfsmState::InJson, &mut ib, &mut bp);

        assert_eq!(final_state, PfsmState::InJson);

        let ib_bits = ib.finish();
        let bp_bits = bp.finish();

        assert!(!ib_bits.is_empty());
        assert!(!bp_bits.is_empty());
    }

    #[test]
    fn test_pfsm_string_with_escape() {
        let json = br#"["a\"b"]"#;
        let mut ib = BitWriter::new();
        let mut bp = BitWriter::new();

        let final_state = pfsm_process_chunk(json, PfsmState::InJson, &mut ib, &mut bp);

        assert_eq!(final_state, PfsmState::InJson);

        let ib_bits = ib.finish();
        let bp_bits = bp.finish();

        assert!(!ib_bits.is_empty());
        assert!(!bp_bits.is_empty());
    }
}
