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
//! # Phi Enumeration
//!
//! Instead of computing phi sequentially after determining actual states, we enumerate
//! phi outputs for ALL 4 possible starting states in parallel. At chunk boundaries,
//! we select the correct accumulated output based on the actual starting state.
//!
//! This implements the full Mytkowicz approach for output-producing FSMs.
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

/// Accumulated phi outputs for a single starting state.
///
/// Uses fixed-width accumulation: store BP decisions (not bits) during enumeration,
/// then compact to actual bitstream after path selection.
#[derive(Clone, Copy, Debug, Default)]
struct PhiAccum {
    /// IB bits: 1 bit per position (bit i = IB for byte i)
    ib: u16,
    /// BP open decisions: bit i = 1 if byte i emitted bp_open
    bp_open: u16,
    /// BP close decisions: bit i = 1 if byte i emitted bp_close
    bp_close: u16,
}

impl PhiAccum {
    /// Compact bp_open/bp_close masks into actual BP bitstream.
    ///
    /// Returns (bp_bits, bp_len) where bp_bits contains the compacted bits
    /// and bp_len is the number of valid bits.
    #[inline]
    fn compact_bp(self, count: usize) -> (u32, u8) {
        let mut bp_bits: u32 = 0;
        let mut bp_len: u8 = 0;

        for i in 0..count {
            if (self.bp_open >> i) & 1 != 0 {
                bp_bits |= 1 << bp_len;
                bp_len += 1;
            }
            if (self.bp_close >> i) & 1 != 0 {
                // close is 0, just advance length
                bp_len += 1;
            }
        }

        (bp_bits, bp_len)
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

/// Enumerate phi outputs for all 4 possible starting states.
///
/// This implements the Mytkowicz phi enumeration: instead of computing phi
/// sequentially after determining actual states, we compute phi for ALL
/// possible starting states in parallel, then select the correct one.
///
/// Returns:
/// - `accum`: Accumulated phi outputs for each of the 4 starting states
/// - `final_trans`: Composed transition function for the chunk
#[target_feature(enable = "avx2")]
unsafe fn enumerate_phi_16(bytes: &[u8; 16]) -> ([PhiAccum; 4], TransitionPack) {
    // S[i] = current state if we started from state i
    // Initialize to identity: S[i] = i
    let mut s: [u8; 4] = [0, 1, 2, 3];

    // Accumulated phi for each starting state
    let mut accum: [PhiAccum; 4] = [PhiAccum::default(); 4];

    for (pos, &byte) in bytes.iter().enumerate() {
        let t = TRANSITION_TABLE[byte as usize];
        let p = PHI_TABLE[byte as usize];

        // Enumerate phi for all 4 starting states
        for start in 0..4 {
            let state = s[start];
            let phi = ((p >> (state * 8)) & 0xFF) as u8;

            // Extract bits: bit0=bp_close, bit1=bp_open, bit2=ib
            let ib_bit = (phi >> 2) & 1;
            let bp_open = (phi >> 1) & 1;
            let bp_close = phi & 1;

            // Accumulate into fixed-width masks
            accum[start].ib |= (ib_bit as u16) << pos;
            accum[start].bp_open |= (bp_open as u16) << pos;
            accum[start].bp_close |= (bp_close as u16) << pos;
        }

        // Update S: S[i] = T[S[i]]
        for state in &mut s {
            *state = ((t >> (*state * 8)) & 0xFF) as u8;
        }
    }

    // Compute composed transition for final state calculation
    // S now contains: S[i] = final state if started from state i
    let final_trans = TransitionPack(
        (s[0] as u32) | ((s[1] as u32) << 8) | ((s[2] as u32) << 16) | ((s[3] as u32) << 24),
    );

    (accum, final_trans)
}

/// Process a 16-byte chunk using phi enumeration.
///
/// This is the optimized version that enumerates phi for all starting states,
/// then selects the correct accumulated output based on the actual starting state.
#[target_feature(enable = "avx2")]
unsafe fn process_chunk_16_enum(
    bytes: &[u8; 16],
    initial_state: u8,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> u8 {
    // Enumerate phi for all starting states
    let (accum, final_trans) = enumerate_phi_16(bytes);

    // Select the correct accumulated output based on actual starting state
    let selected = accum[initial_state as usize];

    // Write IB bits (16 bits, one per byte)
    ib.write_bits(selected.ib as u64, 16);

    // Compact and write BP bits
    let (bp_bits, bp_len) = selected.compact_bp(16);
    if bp_len > 0 {
        bp.write_bits(bp_bits as u64, bp_len as usize);
    }

    // Return final state
    final_trans.apply(initial_state)
}

/// Compute the state after each byte position using prefix composition.
///
/// Returns `states[i]` = state BEFORE processing byte `i` (for phi extraction).
///
/// Note: This is the old sequential approach, kept for reference.
/// The new enumerate_phi_16 replaces this with full phi enumeration.
#[target_feature(enable = "avx2")]
#[allow(dead_code)]
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

/// Process a 16-byte chunk using PFSM shuffle composition (old approach).
///
/// Returns the final state after processing all 16 bytes.
///
/// Note: This is the old sequential phi extraction approach.
/// Use process_chunk_16_enum for the optimized phi enumeration version.
#[target_feature(enable = "avx2")]
#[allow(dead_code)]
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
#[allow(dead_code)]
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

/// Process a 32-byte chunk using phi enumeration (two 16-byte halves).
#[target_feature(enable = "avx2")]
unsafe fn process_chunk_32_enum(
    bytes: &[u8],
    initial_state: u8,
    ib: &mut BitWriter,
    bp: &mut BitWriter,
) -> u8 {
    debug_assert!(bytes.len() == 32);

    // Process first half with enumeration
    let first_half: &[u8; 16] = bytes[0..16].try_into().unwrap();
    let mid_state = process_chunk_16_enum(first_half, initial_state, ib, bp);

    // Process second half with enumeration
    let second_half: &[u8; 16] = bytes[16..32].try_into().unwrap();
    process_chunk_16_enum(second_half, mid_state, ib, bp)
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

    // Process 32-byte chunks using phi enumeration
    while offset + 32 <= json.len() {
        state = process_chunk_32_enum(&json[offset..offset + 32], state, &mut ib, &mut bp);
        offset += 32;
    }

    // Process remaining bytes in 16-byte chunk if possible using phi enumeration
    if offset + 16 <= json.len() {
        let chunk: &[u8; 16] = json[offset..offset + 16].try_into().unwrap();
        state = process_chunk_16_enum(chunk, state, &mut ib, &mut bp);
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

    #[test]
    fn test_phi_accum_compact_bp() {
        // Test BP compaction: bp_open at pos 0, 2; bp_close at pos 1, 3
        // Expected: 1 (open), 0 (close), 1 (open), 0 (close) = 0b0101 = 5
        let accum = PhiAccum {
            ib: 0,
            bp_open: 0b0101,  // positions 0 and 2
            bp_close: 0b1010, // positions 1 and 3
        };
        let (bp_bits, bp_len) = accum.compact_bp(4);
        assert_eq!(bp_len, 4);
        assert_eq!(bp_bits, 0b0101); // open=1, close=0
    }

    #[test]
    fn test_phi_accum_compact_bp_variable_length() {
        // Test variable length: only bp_open at pos 0 and 3
        // Expected: 1 (pos 0), 1 (pos 3) = 0b11 = 3, length 2
        let accum = PhiAccum {
            ib: 0,
            bp_open: 0b1001, // positions 0 and 3
            bp_close: 0,
        };
        let (bp_bits, bp_len) = accum.compact_bp(4);
        assert_eq!(bp_len, 2);
        assert_eq!(bp_bits, 0b11);
    }

    #[test]
    fn test_enumerate_phi_16() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        unsafe {
            // Test: {"a":1}  padded to 16 bytes
            let mut bytes = [b' '; 16];
            bytes[0] = b'{';
            bytes[1] = b'"';
            bytes[2] = b'a';
            bytes[3] = b'"';
            bytes[4] = b':';
            bytes[5] = b'1';
            bytes[6] = b'}';

            let (accum, final_trans) = enumerate_phi_16(&bytes);

            // Starting from InJson (0):
            // - { : IB=1, bp_open=1
            // - " : IB=1 (entering string)
            // - a : IB=0 (in string)
            // - " : IB=1 (exiting string)
            // - : : IB=0 (colon, not IB in standard cursor... let's check)
            // - 1 : IB=1 (value start)
            // - } : IB=1, bp_close=1

            // Check final state for starting state 0
            assert_eq!(final_trans.apply(0), 0); // Back to InJson

            // The accum[0] should have the phi outputs for starting from InJson
            // IB should have bits set for structural chars
            let selected = accum[0];

            // { is at position 0, should have bp_open
            assert_ne!(selected.bp_open & 1, 0);
            // } is at position 6, should have bp_close
            assert_ne!(selected.bp_close & (1 << 6), 0);
        }
    }

    #[test]
    fn test_enumerate_phi_matches_prefix_states() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }
        unsafe {
            // Compare enumerate_phi_16 output with prefix_states_16 + manual phi extraction
            let mut bytes = [b' '; 16];
            bytes[0] = b'"';
            bytes[1] = b'a';
            bytes[2] = b'"';
            bytes[3] = b'{';
            bytes[4] = b'}';

            let initial_state: u8 = 0;

            // Old approach: prefix states then extract phi
            let states = prefix_states_16(&bytes, initial_state);
            let mut expected_ib: u16 = 0;
            let mut expected_bp_open: u16 = 0;
            let mut expected_bp_close: u16 = 0;

            for i in 0..16 {
                let phi_entry = PHI_TABLE[bytes[i] as usize];
                let state = states[i];
                let phi = ((phi_entry >> (state * 8)) & 0xFF) as u8;
                let ib_bit = (phi >> 2) & 1;
                let bp_open = (phi >> 1) & 1;
                let bp_close = phi & 1;
                expected_ib |= (ib_bit as u16) << i;
                expected_bp_open |= (bp_open as u16) << i;
                expected_bp_close |= (bp_close as u16) << i;
            }

            // New approach: enumerate phi
            let (accum, _) = enumerate_phi_16(&bytes);
            let selected = accum[initial_state as usize];

            assert_eq!(selected.ib, expected_ib, "IB mismatch");
            assert_eq!(selected.bp_open, expected_bp_open, "bp_open mismatch");
            assert_eq!(selected.bp_close, expected_bp_close, "bp_close mismatch");
        }
    }
}
