//! Shared quote-mask primitives for DSV semi-indexing.
//!
//! Every DSV SIMD backend answers the same question for each 64-byte chunk:
//! *which byte positions are outside a quoted field?* Delimiters and newlines are
//! marked only where that mask is set, and a single carry bit hands the
//! inside/outside state to the next chunk.
//!
//! Two instruction families compute it:
//!
//! * **prefix-xor** (`dsv::simd::{avx2, sse2, neon}`) — the inclusive prefix XOR
//!   of the quote bitmap is the running quote parity, so complementing it (after
//!   folding in the incoming carry) gives the outside-quotes mask.
//! * **deposit + adder** (`dsv::simd::{bmi2, sve2}`) — BMI2 `PDEP` / SVE2 `BDEP`
//!   scatter [`ODDS_MASK`] onto the quote positions, and one addition propagates
//!   carries through the gaps, filling the regions between quote pairs.
//!
//! Both produce the same mask; only the instruction differs. Each backend
//! therefore supplies *just* its instruction-specific value — the prefix XOR or
//! the deposited addend — and calls [`toggle64_from_prefix_xor`] or
//! [`toggle64_from_deposit`] for the shared tail.
//!
//! ## Why this module exists
//!
//! That tail used to be copied into all five backends, along with three separate
//! copies of [`prefix_xor`]. The chunk-boundary carry was derived from the
//! adder's overflow flag, which is wrong when a quote lands on bit 63 — `addend
//! << 1` shifts that bit out, so the opener never overflows. Only the two deposit
//! backends used the adder, so the bug lived in two of the five copies and had to
//! be fixed twice (#149, #182). The carry now has exactly one definition,
//! [`next_carry`], and every backend routes through it.
//!
//! ## Quote-bit convention
//!
//! A quote byte itself takes its **post-toggle** state: an opening quote reads as
//! inside, a closing quote as outside. This never affects an index — the quote
//! character is distinct from the delimiter and the newline, so a quote position
//! is never a marker candidate — but all five backends must agree on it for the
//! cross-backend differential tests (`tests/dsv_simd_differential_tests.rs`) to
//! compare masks exactly.

/// Alternating bit pattern `0101…`, the payload the deposit backends scatter
/// onto quote positions.
///
/// Shifting it left by the incoming carry bit selects which quotes count as
/// "enters": with carry 0 the 1st, 3rd, 5th… quote opens a field; with carry 1
/// the chunk starts inside a field, so the parity is shifted by one.
pub(crate) const ODDS_MASK: u64 = 0x5555_5555_5555_5555;

/// Inclusive prefix XOR (cumulative XOR) of a 64-bit mask.
///
/// Bit `i` of the result is the XOR of bits `0..=i` of `x` — that is, the parity
/// of quotes seen up to and including position `i`. Odd parity means "inside a
/// quoted field".
///
/// Example: quotes at positions 2 and 5.
///
/// ```text
/// x          = 0b100100  (bits 2 and 5)
/// prefix_xor = 0b011100  (bits 2,3,4 are inside)
/// ```
///
/// Implemented as a doubling shift chain: after step `k`, bit `i` holds the XOR
/// of bits `max(0, i - 2^k + 1) ..= i`, so six steps cover the whole word. On
/// aarch64 the NEON backend computes the same value in one PMULL (carryless
/// multiply by all-ones); this scalar form is that kernel's test oracle there,
/// and the production path on x86_64.
#[inline]
// STYLE-0005: platform-gated. Live on x86_64 (the AVX2/SSE2 backends); on
// aarch64 the PMULL kernel supersedes it and only its equivalence test calls it.
#[allow(dead_code)]
pub(crate) fn prefix_xor(x: u64) -> u64 {
    let mut y = x;
    y ^= y << 1;
    y ^= y << 2;
    y ^= y << 4;
    y ^= y << 8;
    y ^= y << 16;
    y ^= y << 32;
    y
}

/// Quote state after a chunk: the incoming state toggled once per quote byte.
///
/// This is the single definition of the chunk-boundary carry, shared by every
/// backend. It is deliberately derived from the quote *count parity* rather than
/// from the adder's carry-out in [`toggle64_from_deposit`]: `addend << 1` drops a
/// bit deposited at position 63, so a quote that opens at bit 63 never produces
/// an overflow and an overflow-derived carry would silently lose it (#149).
#[inline]
pub(crate) fn next_carry(carry: u64, quote_mask: u64) -> u64 {
    (u64::from(quote_mask.count_ones()) + (carry & 1)) & 1
}

/// Shared tail for the prefix-xor backends (AVX2 / SSE2 / NEON).
///
/// `quote_xor` must be [`prefix_xor`] of `quote_mask`, computed by whichever
/// instruction the backend prefers. Folding in the carry is an XOR against the
/// carry broadcast to all 64 bits, and complementing turns "inside" into the
/// outside-quotes mask the caller ANDs its delimiter and newline masks with.
///
/// # Returns
///
/// `(outside_mask, new_carry)` — 1 bits mark positions outside quotes, and the
/// carry feeds the next chunk.
#[inline]
#[allow(dead_code)] // STYLE-0005: platform-gated (unused on targets with no DSV SIMD backend)
pub(crate) fn toggle64_from_prefix_xor(carry: u64, quote_mask: u64, quote_xor: u64) -> (u64, u64) {
    // Broadcast the carry bit to all 64 lanes: 0 -> 0x0000…, 1 -> 0xFFFF…
    let inside = quote_xor ^ 0u64.wrapping_sub(carry & 1);
    (!inside, next_carry(carry, quote_mask))
}

/// Shared tail for the deposit backends (BMI2 `PDEP` / SVE2 `BDEP`).
///
/// `addend` must be `ODDS_MASK << (carry & 1)` deposited onto `quote_mask` by the
/// backend's instruction. The formula is `((addend << 1) | c) + !quote_mask`:
/// shifting the deposited bits left by one and adding the complement of the quote
/// positions propagates carries through the non-quote runs, "filling in" the
/// regions between matched quote pairs.
///
/// The adder's carry-out is *not* used for the chunk carry — see [`next_carry`]
/// for why (#149).
///
/// # Returns
///
/// `(outside_mask, new_carry)`, identical to [`toggle64_from_prefix_xor`] for the
/// same input.
#[inline]
#[allow(dead_code)] // STYLE-0005: platform-gated (unused on targets with no PDEP/BDEP backend)
pub(crate) fn toggle64_from_deposit(carry: u64, quote_mask: u64, addend: u64) -> (u64, u64) {
    let c = carry & 1;
    let result = ((addend << 1) | c).wrapping_add(!quote_mask);
    (result, next_carry(carry, quote_mask))
}

/// Portable reference for the whole operation: the mask and carry every backend
/// must produce for `(carry, quote_mask)`.
///
/// Expressing it through [`prefix_xor`] states the semantics in one place, and
/// gives the per-backend unit tests a single oracle instead of a hand-rolled copy
/// each. The previous SVE2 oracle re-implemented the deposit formula and so
/// inherited its bit-63 carry bug, which is exactly why that test stayed green
/// while the code was wrong (#149).
#[inline]
// STYLE-0005: scalar reference impl kept for correctness comparison against the
// AVX2/SSE2/NEON/BMI2/SVE2 kernels; nothing in production dispatches to it.
#[allow(dead_code)]
pub(crate) fn toggle64_scalar(carry: u64, quote_mask: u64) -> (u64, u64) {
    toggle64_from_prefix_xor(carry, quote_mask, prefix_xor(quote_mask))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Independent bit-serial oracle: walk the word toggling an inside-quotes
    /// flag, exactly like the scalar DSV parser (`src/dsv/parser.rs`). Written
    /// as a loop on purpose — it shares no algebra with either kernel family, so
    /// it cannot inherit a bug from them the way the old deposit-formula
    /// "reference" did (#149).
    pub(crate) fn toggle64_bit_serial(carry: u64, quote_mask: u64) -> (u64, u64) {
        let mut inside = carry & 1 == 1;
        let mut outside_mask = 0u64;
        for i in 0..64 {
            if (quote_mask >> i) & 1 == 1 {
                inside = !inside;
            }
            if !inside {
                outside_mask |= 1 << i;
            }
        }
        (outside_mask, u64::from(inside))
    }

    /// Deterministic PRNG for wide mask coverage without a `rand` dependency.
    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Fixed edge patterns (bit 63 is the #149 regression), every single-bit
    /// mask, and 512 pseudo-random masks. Shared with the BMI2 and SVE2 kernel
    /// tests so all three sweep the same inputs.
    pub(crate) fn quote_mask_patterns() -> Vec<u64> {
        let mut patterns = vec![
            0u64,
            1,
            0b11,
            0b101,
            0b1001,
            0x8000_0000_0000_0000,
            0xC000_0000_0000_0000,
            0x8000_0000_0000_0001,
            0xAAAA_AAAA_AAAA_AAAA,
            0x5555_5555_5555_5555,
            0xFF00_FF00_FF00_FF00,
            !0u64,
        ];
        patterns.extend((0..64).map(|i| 1u64 << i));
        let mut state = 0x149u64;
        patterns.extend((0..512).map(|_| splitmix64(&mut state)));
        patterns
    }

    /// Software PDEP/BDEP: deposit the low bits of `data` into the set positions
    /// of `mask`, lowest first. Lets the deposit tail be exercised on any host,
    /// including one with neither BMI2 nor SVE2-BITPERM.
    fn deposit(data: u64, mask: u64) -> u64 {
        let mut result = 0u64;
        let mut src = 0;
        for i in 0..64 {
            if (mask >> i) & 1 == 1 {
                result |= ((data >> src) & 1) << i;
                src += 1;
            }
        }
        result
    }

    #[test]
    fn test_prefix_xor_matches_parity_loop() {
        for quote_mask in quote_mask_patterns() {
            let mut expected = 0u64;
            let mut parity = 0u64;
            for i in 0..64 {
                parity ^= (quote_mask >> i) & 1;
                expected |= parity << i;
            }
            assert_eq!(
                prefix_xor(quote_mask),
                expected,
                "prefix_xor mismatch for {quote_mask:#x}"
            );
        }
    }

    #[test]
    fn test_toggle64_scalar_matches_bit_serial_reference() {
        for quote_mask in quote_mask_patterns() {
            for carry in [0u64, 1] {
                assert_eq!(
                    toggle64_scalar(carry, quote_mask),
                    toggle64_bit_serial(carry, quote_mask),
                    "toggle64_scalar mismatch for quote_mask={quote_mask:#x}, carry={carry}"
                );
            }
        }
    }

    /// The deposit tail must agree with the prefix-xor tail bit-for-bit. This is
    /// the assertion the two families never shared before #182: it runs on every
    /// host, not just one with PDEP or BDEP hardware, so a divergence between the
    /// two formulas is caught even where neither instruction exists.
    #[test]
    fn test_toggle64_from_deposit_matches_bit_serial_reference() {
        for quote_mask in quote_mask_patterns() {
            for carry in [0u64, 1] {
                let addend = deposit(ODDS_MASK << (carry & 1), quote_mask);
                assert_eq!(
                    toggle64_from_deposit(carry, quote_mask, addend),
                    toggle64_bit_serial(carry, quote_mask),
                    "toggle64_from_deposit mismatch for quote_mask={quote_mask:#x}, carry={carry}"
                );
            }
        }
    }

    #[test]
    fn test_bit63_carry_149() {
        // A quote at bit 63 must toggle the carry. The overflow-derived carry
        // lost it, because `addend << 1` shifts the deposited bit out of the
        // word — the whole of #149 in four assertions.
        let opener = 1u64 << 63;

        let (mask, carry) = toggle64_scalar(0, opener);
        assert_eq!(carry, 1, "Opener at bit 63 must carry into the next chunk");
        assert_eq!(mask, !0u64 >> 1, "Bits 0..63 outside, bit 63 inside");

        let (_mask, carry) = toggle64_scalar(1, opener);
        assert_eq!(carry, 0, "Closer at bit 63 must clear the carry");

        let addend = deposit(ODDS_MASK, opener);
        assert_eq!(
            toggle64_from_deposit(0, opener, addend),
            (!0u64 >> 1, 1),
            "the deposit tail must agree at bit 63"
        );
    }

    #[test]
    fn test_no_quotes_is_all_outside() {
        assert_eq!(toggle64_scalar(0, 0), (!0u64, 0));
        assert_eq!(toggle64_scalar(1, 0), (0, 1));
    }
}
