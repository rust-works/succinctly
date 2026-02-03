//! AVX2 PFSM shuffle composition for JSON semi-indexing.
//!
//! This module implements the Mytkowicz et al. "Data-Parallel Finite-State Machines"
//! technique using AVX2 `vpshufb` to compose state transitions in parallel.
//!
//! # Background
//!
//! The PFSM tables pack 4 possible next-states into each table entry:
//! - `TRANSITION_TABLE[byte]` returns u32 with next-state for all 4 starting states
//! - State encoding: InJson=0, InString=1, InEscape=2, InValue=3
//!
//! # Composition Algorithm
//!
//! For two transitions T0 and T1:
//! ```text
//! T_composed[s] = T1[T0[s]]
//! ```
//!
//! Using `vpshufb`: `_mm_shuffle_epi8(T1, T0)` computes `T1[T0[i]]` for each byte.
//!
//! # Binary Tree Reduction
//!
//! Instead of 16 sequential compositions, use binary tree:
//! - Level 1: 8 compositions (T01, T23, ...)
//! - Level 2: 4 compositions (T0123, ...)
//! - Level 3: 2 compositions
//! - Level 4: 1 composition
//!
//! Total: 15 `vpshufb` ops with O(log n) sequential dependency depth.

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

use crate::json::pfsm_tables::{PHI_TABLE, TRANSITION_TABLE};
use crate::json::standard::{SemiIndex, State};
use crate::json::BitWriter;

/// Packed transition function for 4 states.
///
/// Layout: `[T[0], T[1], T[2], T[3]]` where `T[s]` is next state when starting from state `s`.
/// Only the low 4 bytes are used; high bytes are ignored.
#[derive(Clone, Copy, Debug)]
struct TransitionPack(u32);

impl TransitionPack {
    /// Identity transition: T[s] = s for all states.
    #[inline]
    const fn identity() -> Self {
        // [0, 1, 2, 3] = 0x03020100
        Self(0x03020100)
    }

    /// Load transition for a given input byte.
    #[inline]
    fn from_byte(byte: u8) -> Self {
        Self(TRANSITION_TABLE[byte as usize])
    }

    /// Convert to __m128i for shuffle operations.
    /// The 4-byte transition is replicated across the low 32 bits.
    #[inline]
    #[target_feature(enable = "avx2")]
    unsafe fn as_m128i(self) -> __m128i {
        _mm_cvtsi32_si128(self.0 as i32)
    }

    /// Extract the next state for a given starting state.
    #[inline]
    fn apply(self, state: u8) -> u8 {
        ((self.0 >> (state * 8)) & 0xFF) as u8
    }
}

/// Compose two transition functions: result[s] = t1[t0[s]]
///
/// Uses `vpshufb` where t0 provides indices and t1 is the lookup table.
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn compose(t0: TransitionPack, t1: TransitionPack) -> TransitionPack {
    // vpshufb: for each byte i in t0, result[i] = t1[t0[i] & 0x0F]
    // Since our states are 0-3, this gives exactly T1[T0[s]]
    let t0_vec = t0.as_m128i();
    let t1_vec = t1.as_m128i();
    let composed = _mm_shuffle_epi8(t1_vec, t0_vec);
    TransitionPack(_mm_cvtsi128_si32(composed) as u32)
}

/// Compose 16 transitions using binary tree reduction.
///
/// Returns the composed transition function: starting state -> final state after 16 bytes.
#[target_feature(enable = "avx2")]
unsafe fn compose_16(bytes: &[u8; 16]) -> TransitionPack {
    // Load all 16 transition packs
    let t: [TransitionPack; 16] = [
        TransitionPack::from_byte(bytes[0]),
        TransitionPack::from_byte(bytes[1]),
        TransitionPack::from_byte(bytes[2]),
        TransitionPack::from_byte(bytes[3]),
        TransitionPack::from_byte(bytes[4]),
        TransitionPack::from_byte(bytes[5]),
        TransitionPack::from_byte(bytes[6]),
        TransitionPack::from_byte(bytes[7]),
        TransitionPack::from_byte(bytes[8]),
        TransitionPack::from_byte(bytes[9]),
        TransitionPack::from_byte(bytes[10]),
        TransitionPack::from_byte(bytes[11]),
        TransitionPack::from_byte(bytes[12]),
        TransitionPack::from_byte(bytes[13]),
        TransitionPack::from_byte(bytes[14]),
        TransitionPack::from_byte(bytes[15]),
    ];

    // Level 1: Compose pairs (8 compositions)
    let l1_0 = compose(t[0], t[1]);
    let l1_1 = compose(t[2], t[3]);
    let l1_2 = compose(t[4], t[5]);
    let l1_3 = compose(t[6], t[7]);
    let l1_4 = compose(t[8], t[9]);
    let l1_5 = compose(t[10], t[11]);
    let l1_6 = compose(t[12], t[13]);
    let l1_7 = compose(t[14], t[15]);

    // Level 2: Compose quads (4 compositions)
    let l2_0 = compose(l1_0, l1_1);
    let l2_1 = compose(l1_2, l1_3);
    let l2_2 = compose(l1_4, l1_5);
    let l2_3 = compose(l1_6, l1_7);

    // Level 3: Compose octets (2 compositions)
    let l3_0 = compose(l2_0, l2_1);
    let l3_1 = compose(l2_2, l2_3);

    // Level 4: Final composition
    compose(l3_0, l3_1)
}

/// Compute the state after each byte position using prefix composition.
///
/// Returns `states[i]` = state BEFORE processing byte `i` (for phi extraction).
#[target_feature(enable = "avx2")]
unsafe fn prefix_states_16(bytes: &[u8; 16], initial_state: u8) -> [u8; 16] {
    // For phi extraction, we need the state BEFORE each byte.
    // This requires a prefix scan, which we compute using parallel prefix.
    //
    // Define prefix[i] = composition of transitions 0..i (exclusive)
    // Then states[i] = prefix[i].apply(initial_state)

    let t: [TransitionPack; 16] = [
        TransitionPack::from_byte(bytes[0]),
        TransitionPack::from_byte(bytes[1]),
        TransitionPack::from_byte(bytes[2]),
        TransitionPack::from_byte(bytes[3]),
        TransitionPack::from_byte(bytes[4]),
        TransitionPack::from_byte(bytes[5]),
        TransitionPack::from_byte(bytes[6]),
        TransitionPack::from_byte(bytes[7]),
        TransitionPack::from_byte(bytes[8]),
        TransitionPack::from_byte(bytes[9]),
        TransitionPack::from_byte(bytes[10]),
        TransitionPack::from_byte(bytes[11]),
        TransitionPack::from_byte(bytes[12]),
        TransitionPack::from_byte(bytes[13]),
        TransitionPack::from_byte(bytes[14]),
        TransitionPack::from_byte(bytes[15]),
    ];

    // Parallel prefix (exclusive scan):
    // prefix[0] = identity
    // prefix[1] = t[0]
    // prefix[2] = t[1] ∘ t[0]
    // ...
    // prefix[i] = t[i-1] ∘ ... ∘ t[0]

    let mut prefix = [TransitionPack::identity(); 16];

    // Sequential computation of prefix (parallel prefix is complex for composition)
    // This is O(n) but the compositions use vpshufb
    for i in 1..16 {
        prefix[i] = compose(prefix[i - 1], t[i - 1]);
    }

    // Apply initial state to get actual states
    let mut states = [0u8; 16];
    for i in 0..16 {
        states[i] = prefix[i].apply(initial_state);
    }

    states
}

/// Process a 16-byte chunk using PFSM shuffle composition.
///
/// Returns the final state after processing all 16 bytes.
#[target_feature(enable = "avx2")]
unsafe fn process_chunk_16(
    bytes: &[u8; 16],
    initial_state: u8,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> u8 {
    // Compute final state using composition
    let final_trans = compose_16(bytes);
    let final_state = final_trans.apply(initial_state);

    // Compute state at each position for phi extraction
    let states = prefix_states_16(bytes, initial_state);

    // Extract phi bits (sequential - this is where NEON failed)
    for i in 0..16 {
        let phi_entry = PHI_TABLE[bytes[i] as usize];
        let state = states[i];
        let phi = ((phi_entry >> (state * 8)) & 0xFF) as u8;

        // Extract bits: bit0=bp_close, bit1=bp_open, bit2=ib
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

    final_state
}

/// Process a 32-byte chunk by processing two 16-byte halves.
#[target_feature(enable = "avx2")]
unsafe fn process_chunk_32(
    bytes: &[u8],
    initial_state: u8,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> u8 {
    debug_assert!(bytes.len() == 32);

    // Process first half
    let first_half: &[u8; 16] = bytes[0..16].try_into().unwrap();
    let mid_state = process_chunk_16(first_half, initial_state, ib, bp);

    // Process second half
    let second_half: &[u8; 16] = bytes[16..32].try_into().unwrap();
    process_chunk_16(second_half, mid_state, ib, bp)
}

/// Build JSON semi-index using AVX2 PFSM shuffle composition.
///
/// Enable with `SUCCINCTLY_AVX2_PFSM=1` environment variable.
pub fn build_semi_index_standard(json: &[u8]) -> SemiIndex {
    // Safety: we check for AVX2 support at runtime in mod.rs
    unsafe { build_semi_index_standard_avx2_pfsm(json) }
}

#[target_feature(enable = "avx2")]
unsafe fn build_semi_index_standard_avx2_pfsm(json: &[u8]) -> SemiIndex {
    let word_capacity = json.len().div_ceil(64);
    let mut ib = BitWriter::with_capacity(word_capacity);
    let mut bp = BitWriter::with_capacity(word_capacity * 2);

    let mut state: u8 = 0; // InJson
    let mut offset = 0;

    // Process 32-byte chunks
    while offset + 32 <= json.len() {
        state = process_chunk_32(&json[offset..offset + 32], state, &mut ib, &mut bp);
        offset += 32;
    }

    // Process remaining bytes in 16-byte chunk if possible
    if offset + 16 <= json.len() {
        let chunk: &[u8; 16] = json[offset..offset + 16].try_into().unwrap();
        state = process_chunk_16(chunk, state, &mut ib, &mut bp);
        offset += 16;
    }

    // Process tail bytes one at a time
    for &byte in &json[offset..] {
        let transition_entry = TRANSITION_TABLE[byte as usize];
        let phi_entry = PHI_TABLE[byte as usize];

        let phi = ((phi_entry >> (state * 8)) & 0xFF) as u8;
        state = ((transition_entry >> (state * 8)) & 0xFF) as u8;

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

    SemiIndex {
        state: match state {
            0 => State::InJson,
            1 => State::InString,
            2 => State::InEscape,
            3 => State::InValue,
            _ => State::InJson,
        },
        ib: ib.finish(),
        bp: bp.finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transition_pack_identity() {
        let id = TransitionPack::identity();
        assert_eq!(id.apply(0), 0);
        assert_eq!(id.apply(1), 1);
        assert_eq!(id.apply(2), 2);
        assert_eq!(id.apply(3), 3);
    }

    #[test]
    fn test_transition_pack_from_byte() {
        // Quote (") should: InJson->InString, InString->InJson, InEscape->InString, InValue->InJson
        let t = TransitionPack::from_byte(b'"');
        assert_eq!(t.apply(0), 1); // InJson -> InString
        assert_eq!(t.apply(1), 0); // InString -> InJson
        assert_eq!(t.apply(2), 1); // InEscape -> InString
        assert_eq!(t.apply(3), 0); // InValue -> InJson
    }

    #[test]
    fn test_compose_identity() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        unsafe {
            let id = TransitionPack::identity();
            let t = TransitionPack::from_byte(b'"');
            let composed = compose(id, t);
            // id ∘ t = t
            assert_eq!(composed.apply(0), t.apply(0));
            assert_eq!(composed.apply(1), t.apply(1));
            assert_eq!(composed.apply(2), t.apply(2));
            assert_eq!(composed.apply(3), t.apply(3));
        }
    }

    #[test]
    fn test_compose_two_bytes() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        unsafe {
            // Test: "a" (quote then 'a')
            let t_quote = TransitionPack::from_byte(b'"');
            let t_a = TransitionPack::from_byte(b'a');
            let composed = compose(t_quote, t_a);

            // Starting from InJson (0):
            // After quote: InString (1)
            // After 'a' in InString: InString (1)
            assert_eq!(composed.apply(0), 1);

            // Starting from InString (1):
            // After quote: InJson (0)
            // After 'a' in InJson: InValue (3)
            assert_eq!(composed.apply(1), 3);
        }
    }

    #[test]
    fn test_compose_16_simple() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        unsafe {
            // Test with all spaces (no state changes from InJson)
            let bytes = [b' '; 16];
            let result = compose_16(&bytes);
            // All spaces: InJson stays InJson
            assert_eq!(result.apply(0), 0);
        }
    }

    #[test]
    fn test_compose_16_string() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        unsafe {
            // Test: "hello world"... (starts and ends string)
            let mut bytes = [b' '; 16];
            bytes[0] = b'"';
            bytes[6] = b'"';
            // Positions: 0=" 1=h 2=e 3=l 4=l 5=o 6="
            // But we fill with spaces, so: 0=" 1-5=space 6=" 7-15=space

            let result = compose_16(&bytes);
            // Starting InJson: " -> InString, spaces stay InString, " -> InJson, spaces stay InJson
            assert_eq!(result.apply(0), 0); // Back to InJson
        }
    }

    #[test]
    fn test_prefix_states() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        unsafe {
            // Test: "ab" (quote, then a, then b in string context)
            let mut bytes = [b' '; 16];
            bytes[0] = b'"';
            bytes[1] = b'a';
            bytes[2] = b'b';
            bytes[3] = b'"';

            let states = prefix_states_16(&bytes, 0);

            // states[i] = state BEFORE processing bytes[i]
            assert_eq!(states[0], 0); // Before quote: InJson
            assert_eq!(states[1], 1); // After quote: InString
            assert_eq!(states[2], 1); // After 'a' in string: InString
            assert_eq!(states[3], 1); // After 'b' in string: InString
            assert_eq!(states[4], 0); // After closing quote: InJson
        }
    }

    #[test]
    fn test_semi_index_matches_scalar() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = br#"{"name": "Alice", "age": 30}"#;

        // Build with scalar PFSM
        let scalar = crate::json::standard::build_semi_index(json);

        // Build with AVX2 PFSM
        let avx2 = build_semi_index_standard(json);

        assert_eq!(scalar.state, avx2.state);
        assert_eq!(scalar.ib, avx2.ib);
        assert_eq!(scalar.bp, avx2.bp);
    }

    #[test]
    fn test_semi_index_larger() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        // Test with input larger than 32 bytes to exercise full chunking
        let json = br#"{"users": [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]}"#;

        let scalar = crate::json::standard::build_semi_index(json);
        let avx2 = build_semi_index_standard(json);

        assert_eq!(scalar.state, avx2.state);
        assert_eq!(scalar.ib, avx2.ib);
        assert_eq!(scalar.bp, avx2.bp);
    }
}
