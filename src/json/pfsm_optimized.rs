//! Optimized single-pass PFSM implementation.
//!
//! This eliminates the intermediate phi_values vector and does everything in one pass.

use crate::json::bit_writer::BitWriter;
use crate::json::pfsm_tables::{PfsmState, PHI_TABLE, TRANSITION_TABLE};

/// Single-pass PFSM: no intermediate vector allocation
pub fn pfsm_process_chunk_optimized(
    json: &[u8],
    initial_state: PfsmState,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> PfsmState {
    let mut state = initial_state;

    for &byte in json {
        // Table lookups
        let transition_entry = TRANSITION_TABLE[byte as usize];
        let phi_entry = PHI_TABLE[byte as usize];

        // Extract phi and next state
        let phi = PfsmState::extract_phi(phi_entry, state);
        state = PfsmState::extract_next_state(transition_entry, state);

        // Extract and write bits immediately
        let bp_close = phi & 1;
        let bp_open = (phi >> 1) & 1;
        let ib_bit = (phi >> 2) & 1;

        ib.write_bit(ib_bit != 0);

        if bp_open != 0 {
            bp.write_1();
        }
        if bp_close != 0 {
            bp.write_0();
        }
    }

    state
}
