//! AVX2 DPFSM (Data-Parallel Finite State Machine) transition composition.
//!
#![allow(dead_code)] // Validation-only module, code exercised by tests
//!
//! This module implements the Mytkowicz et al. "Data-Parallel Finite-State Machines"
//! technique using AVX2 `vpshufb` to compose state transitions in parallel.
//!
//! # Purpose
//!
//! This is a **validation-only** implementation demonstrating that DPFSM transition
//! composition produces correct final states. It is not used in production because
//! phi (output) extraction remains sequential and negates any parallelism benefit.
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

use crate::json::pfsm_tables::TRANSITION_TABLE;

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

/// Compute final state after processing bytes using DPFSM composition.
///
/// This validates that parallel composition produces the same result as
/// sequential state machine execution.
#[cfg(test)]
#[target_feature(enable = "avx2")]
unsafe fn compute_final_state(bytes: &[u8], initial_state: u8) -> u8 {
    let mut state = initial_state;
    let mut offset = 0;

    // Process 16-byte chunks using SIMD composition
    while offset + 16 <= bytes.len() {
        let chunk: &[u8; 16] = bytes[offset..offset + 16].try_into().unwrap();
        let composed = compose_16(chunk);
        state = composed.apply(state);
        offset += 16;
    }

    // Process remaining bytes sequentially
    for &byte in &bytes[offset..] {
        let t = TRANSITION_TABLE[byte as usize];
        state = ((t >> (state * 8)) & 0xFF) as u8;
    }

    state
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
            // id . t = t
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

            let result = compose_16(&bytes);
            // Starting InJson: " -> InString, spaces stay InString, " -> InJson, spaces stay InJson
            assert_eq!(result.apply(0), 0); // Back to InJson
        }
    }

    #[test]
    fn test_final_state_matches_scalar() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        let json = br#"{"name": "Alice", "age": 30}"#;

        // Compute with scalar
        let mut scalar_state: u8 = 0;
        for &byte in json.iter() {
            let t = TRANSITION_TABLE[byte as usize];
            scalar_state = ((t >> (scalar_state * 8)) & 0xFF) as u8;
        }

        // Compute with DPFSM
        let dpfsm_state = unsafe { compute_final_state(json, 0) };

        assert_eq!(scalar_state, dpfsm_state);
    }

    #[test]
    fn test_final_state_larger() {
        if !is_x86_feature_detected!("avx2") {
            return;
        }

        // Test with input larger than 16 bytes
        let json = br#"{"users": [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]}"#;

        // Compute with scalar
        let mut scalar_state: u8 = 0;
        for &byte in json.iter() {
            let t = TRANSITION_TABLE[byte as usize];
            scalar_state = ((t >> (scalar_state * 8)) & 0xFF) as u8;
        }

        // Compute with DPFSM
        let dpfsm_state = unsafe { compute_final_state(json, 0) };

        assert_eq!(scalar_state, dpfsm_state);
    }
}
