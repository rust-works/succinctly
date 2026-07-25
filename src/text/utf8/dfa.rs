//! Table-driven UTF-8 validation DFA.
//!
//! A nine-state deterministic automaton that accepts exactly the well-formed
//! UTF-8 byte sequences of [Unicode Table 3-7] / [RFC 3629] — the same language
//! [`core::str::from_utf8`] accepts. It is the multi-byte fallback for the
//! broadword scan in [`super::broadword`], which handles ASCII runs eight bytes
//! at a time and hands off here at the first non-ASCII byte.
//!
//! ## Structure
//!
//! Bytes are mapped to one of twelve *classes* by [`class_of`], and a
//! `(state, class)` pair selects the next state from [`TRANSITIONS`]. This is
//! [Bjoern Höhrmann's DFA], and the class numbering is deliberately his, so
//! that both tables can be checked against his published `utf8d` array — see
//! `transition_table_matches_hoehrmann` in the tests below. (His "12 states"
//! are row *offsets* `0, 12, .., 96` into a flattened table, i.e. the same nine
//! states used here at stride 12.)
//!
//! ## Why twelve classes and not ten
//!
//! Four states exist to constrain the byte *following* a specific lead:
//! `E0` requires `A0..=BF`, `ED` requires `80..=9F`, `F0` requires `90..=BF`,
//! and `F4` requires `80..=8F`. A transition table indexed by `(state, class)`
//! can only enforce those if the continuation range `80..=BF` is split at the
//! two cut points `0x90` and `0xA0` — with a single continuation class the
//! table cannot tell `E0 80` (overlong) from `E0 A0` (valid), and the validator
//! would accept overlong encodings and surrogates. Both cuts are necessary:
//! `F0` rejects `80..=8F` where `F4` requires it, and `E0` rejects `90..=9F`
//! where `ED` requires it.
//!
//! [Unicode Table 3-7]: https://www.unicode.org/versions/Unicode15.0.0/ch03.pdf
//! [RFC 3629]: https://datatracker.ietf.org/doc/html/rfc3629
//! [Bjoern Höhrmann's DFA]: https://bjoern.hoehrmann.de/utf-8/decoder/dfa/

/// Ground state: `pos` sits on a code-point boundary and everything before it
/// is well-formed. Also the start state and the only accepting state.
pub(crate) const ACCEPT: u32 = 0;

/// Error state. Sticky — every byte maps `REJECT` back to `REJECT`.
pub(crate) const REJECT: u32 = 1;

/// Number of DFA states; also the number of nibbles packed into each
/// [`STEP`] entry.
const STATES: usize = 9;

/// Number of byte classes.
const CLASSES: usize = 12;

/// Map a byte to its class.
///
/// | Bytes     | Class | Role                        |
/// |-----------|-------|-----------------------------|
/// | `00..=7F` | 0     | ASCII                       |
/// | `80..=8F` | 1     | continuation, low third     |
/// | `90..=9F` | 9     | continuation, middle third  |
/// | `A0..=BF` | 7     | continuation, high third    |
/// | `C0..=C1` | 8     | never valid (overlong lead) |
/// | `C2..=DF` | 2     | 2-byte lead                 |
/// | `E0`      | 10    | 3-byte lead, next `A0..=BF` |
/// | `E1..=EC` | 3     | 3-byte lead                 |
/// | `ED`      | 4     | 3-byte lead, next `80..=9F` |
/// | `EE..=EF` | 3     | 3-byte lead                 |
/// | `F0`      | 11    | 4-byte lead, next `90..=BF` |
/// | `F1..=F3` | 6     | 4-byte lead                 |
/// | `F4`      | 5     | 4-byte lead, next `80..=8F` |
/// | `F5..=FF` | 8     | never valid (> U+10FFFF)    |
#[inline]
const fn class_of(byte: u8) -> u8 {
    match byte {
        0x00..=0x7F => 0,
        0x80..=0x8F => 1,
        0x90..=0x9F => 9,
        0xA0..=0xBF => 7,
        0xC0..=0xC1 => 8,
        0xC2..=0xDF => 2,
        0xE0 => 10,
        0xE1..=0xEC => 3,
        0xED => 4,
        0xEE..=0xEF => 3,
        0xF0 => 11,
        0xF1..=0xF3 => 6,
        0xF4 => 5,
        0xF5..=0xFF => 8,
    }
}

/// Next state by `(state, class)`.
///
/// State meanings, in the index order Höhrmann's table implies:
///
/// | Idx | Name       | Meaning                                    |
/// |-----|------------|--------------------------------------------|
/// | 0   | `ACCEPT`   | on a code-point boundary                   |
/// | 1   | `REJECT`   | error, sticky                              |
/// | 2   | `TAIL1`    | one more continuation, any `80..=BF`       |
/// | 3   | `TAIL2`    | two more continuations, any                |
/// | 4   | `AFTER_E0` | next must be `A0..=BF`, then `TAIL1`       |
/// | 5   | `AFTER_ED` | next must be `80..=9F`, then `TAIL1`       |
/// | 6   | `AFTER_F0` | next must be `90..=BF`, then `TAIL2`       |
/// | 7   | `TAIL3`    | three more continuations, any              |
/// | 8   | `AFTER_F4` | next must be `80..=8F`, then `TAIL2`       |
///
/// Every `(state, class)` pair not leading somewhere useful maps to `REJECT`,
/// from which no path returns — so the set of `ACCEPT`-to-`ACCEPT` loops is
/// exactly the nine rows of Unicode Table 3-7.
#[rustfmt::skip]
const TRANSITIONS: [[u8; CLASSES]; STATES] = [
    //  c0  c1  c2  c3  c4  c5  c6  c7  c8  c9 c10 c11
    //  ASC 8x  C2. E1. ED  F4  F1. Ax  bad 9x  E0  F0
    [    0,  1,  2,  3,  5,  8,  7,  1,  1,  1,  4,  6 ], // 0 ACCEPT
    [    1,  1,  1,  1,  1,  1,  1,  1,  1,  1,  1,  1 ], // 1 REJECT
    [    1,  0,  1,  1,  1,  1,  1,  0,  1,  0,  1,  1 ], // 2 TAIL1
    [    1,  2,  1,  1,  1,  1,  1,  2,  1,  2,  1,  1 ], // 3 TAIL2
    [    1,  1,  1,  1,  1,  1,  1,  2,  1,  1,  1,  1 ], // 4 AFTER_E0
    [    1,  2,  1,  1,  1,  1,  1,  1,  1,  2,  1,  1 ], // 5 AFTER_ED
    [    1,  1,  1,  1,  1,  1,  1,  3,  1,  3,  1,  1 ], // 6 AFTER_F0
    [    1,  3,  1,  1,  1,  1,  1,  3,  1,  3,  1,  1 ], // 7 TAIL3
    [    1,  3,  1,  1,  1,  1,  1,  1,  1,  1,  1,  1 ], // 8 AFTER_F4
];

/// Per-byte transition rows, packed one nibble per state.
///
/// Nibble `s` of `STEP[b]` is the state reached from state `s` on byte `b`;
/// nine states at four bits each need 36 bits, so a row fits in a `u64`.
///
/// The point of the packing is dependency structure, not size. With a flat
/// `[u8; 108]` indexed by `12 * state + class`, the loop-carried chain through
/// [`step`] is `add` then *load* — around five or six cycles. Here the only
/// load is `STEP[byte]`, which depends solely on the input byte and so sits off
/// the carried chain entirely, leaving `shift` then `and` (two or three cycles)
/// between one state and the next.
const STEP: [u64; 256] = build_step();

/// Build [`STEP`] from [`class_of`] and [`TRANSITIONS`] at compile time.
const fn build_step() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut byte = 0usize;
    while byte < 256 {
        #[allow(clippy::cast_possible_truncation)] // STYLE-0005: `byte < 256` by loop bound
        let class = class_of(byte as u8) as usize;
        let mut state = 0usize;
        let mut row = 0u64;
        while state < STATES {
            row |= (TRANSITIONS[state][class] as u64) << (4 * state);
            state += 1;
        }
        table[byte] = row;
        byte += 1;
    }
    table
}

/// Advance the DFA one byte.
///
/// `state` must be a valid state index (`0..9`); every value [`step`] returns
/// satisfies that, and [`ACCEPT`] is the intended starting point, so callers
/// that thread the result back in cannot leave the range.
#[inline(always)]
#[allow(clippy::cast_possible_truncation)] // STYLE-0005: the nibble mask keeps the result in 0..16
pub(crate) fn step(state: u32, byte: u8) -> u32 {
    ((STEP[byte as usize] >> (state * 4)) & 0xF) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bjoern Höhrmann's published `utf8d` table, verbatim, as an independent
    /// oracle for both of ours.
    ///
    /// The first 256 bytes are his byte classes; the remaining 108 are his
    /// transition table, holding row *offsets* (multiples of 12) rather than
    /// state indices. Source: <https://bjoern.hoehrmann.de/utf-8/decoder/dfa/>
    /// (MIT licensed).
    #[rustfmt::skip]
    const UTF8D: [u8; 364] = [
        // 00..1F
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        // 20..3F
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        // 40..5F
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        // 60..7F
        0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
        // 80..9F
        1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, 9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,9,
        // A0..BF
        7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7, 7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,7,
        // C0..DF
        8,8,2,2,2,2,2,2,2,2,2,2,2,2,2,2, 2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
        // E0..FF
        10,3,3,3,3,3,3,3,3,3,3,3,3,4,3,3, 11,6,6,6,5,8,8,8,8,8,8,8,8,8,8,8,
        // transition table: 9 rows of 12, values are row offsets (state * 12)
         0,12,24,36,60,96,84,12,12,12,48,72,
        12,12,12,12,12,12,12,12,12,12,12,12,
        12, 0,12,12,12,12,12, 0,12, 0,12,12,
        12,24,12,12,12,12,12,24,12,24,12,12,
        12,12,12,12,12,12,12,24,12,12,12,12,
        12,24,12,12,12,12,12,12,12,24,12,12,
        12,12,12,12,12,12,12,36,12,36,12,12,
        12,36,12,12,12,12,12,36,12,36,12,12,
        12,36,12,12,12,12,12,12,12,12,12,12,
    ];

    /// Both tables reproduce Höhrmann's published `utf8d` exactly.
    ///
    /// This is the strongest single gate on the DFA: an external, independently
    /// authored oracle. A transcription slip in either `class_of` or
    /// `TRANSITIONS` fails here even if it happens to be self-consistent.
    #[test]
    fn transition_table_matches_hoehrmann() {
        for byte in 0..=255u8 {
            assert_eq!(
                class_of(byte),
                UTF8D[byte as usize],
                "class mismatch for byte {byte:#04X}"
            );
        }
        for state in 0..STATES {
            for class in 0..CLASSES {
                assert_eq!(
                    u16::from(TRANSITIONS[state][class]) * 12,
                    u16::from(UTF8D[256 + 12 * state + class]),
                    "transition mismatch at state {state}, class {class}"
                );
            }
        }
    }

    /// The nine rows of Unicode Table 3-7, transcribed directly as a
    /// `(state, byte)` match.
    ///
    /// Deliberately shares no code with `class_of`/`TRANSITIONS` so that a typo
    /// common to both tables cannot hide behind a self-consistent check.
    fn reference_step(state: u32, byte: u8) -> u32 {
        const TAIL1: u32 = 2;
        const TAIL2: u32 = 3;
        const AFTER_E0: u32 = 4;
        const AFTER_ED: u32 = 5;
        const AFTER_F0: u32 = 6;
        const TAIL3: u32 = 7;
        const AFTER_F4: u32 = 8;

        match state {
            ACCEPT => match byte {
                0x00..=0x7F => ACCEPT,
                0xC2..=0xDF => TAIL1,
                0xE0 => AFTER_E0,
                0xE1..=0xEC | 0xEE..=0xEF => TAIL2,
                0xED => AFTER_ED,
                0xF0 => AFTER_F0,
                0xF1..=0xF3 => TAIL3,
                0xF4 => AFTER_F4,
                _ => REJECT,
            },
            TAIL1 => match byte {
                0x80..=0xBF => ACCEPT,
                _ => REJECT,
            },
            TAIL2 => match byte {
                0x80..=0xBF => TAIL1,
                _ => REJECT,
            },
            TAIL3 => match byte {
                0x80..=0xBF => TAIL2,
                _ => REJECT,
            },
            AFTER_E0 => match byte {
                0xA0..=0xBF => TAIL1,
                _ => REJECT,
            },
            AFTER_ED => match byte {
                0x80..=0x9F => TAIL1,
                _ => REJECT,
            },
            AFTER_F0 => match byte {
                0x90..=0xBF => TAIL2,
                _ => REJECT,
            },
            AFTER_F4 => match byte {
                0x80..=0x8F => TAIL2,
                _ => REJECT,
            },
            _ => REJECT,
        }
    }

    /// `step` agrees with a direct transcription of Unicode Table 3-7 for every
    /// one of the 9 x 256 state/byte pairs.
    #[test]
    fn step_matches_table_3_7() {
        for state in 0..STATES as u32 {
            for byte in 0..=255u8 {
                assert_eq!(
                    step(state, byte),
                    reference_step(state, byte),
                    "state {state}, byte {byte:#04X}"
                );
            }
        }
    }

    /// The packed nibble table reproduces `TRANSITIONS[state][class_of(byte)]`.
    ///
    /// Pins the `build_step` const-fn packing independently of what the tables
    /// themselves say.
    #[test]
    fn packed_step_matches_reference_tables() {
        for state in 0..STATES {
            for byte in 0..=255u8 {
                #[allow(clippy::cast_possible_truncation)] // STYLE-0005: state < STATES == 9
                let packed = step(state as u32, byte);
                assert_eq!(
                    packed,
                    u32::from(TRANSITIONS[state][class_of(byte) as usize]),
                    "state {state}, byte {byte:#04X}"
                );
            }
        }
    }

    /// Every lead/second-byte pair agrees with `core::str::from_utf8`.
    ///
    /// Exhaustively pins the four range-restricted transitions — `E0 A0..BF`,
    /// `ED 80..9F`, `F0 90..BF`, `F4 80..8F` — which are exactly the cases a
    /// single-continuation-class table cannot express, and whose failure mode
    /// is silently accepting overlongs and surrogates.
    #[test]
    fn dfa_second_byte_matrix() {
        for lead in 0xC0..=0xFFu8 {
            for second in 0..=255u8 {
                // Build a sequence that is well-formed iff the lead/second pair
                // is: pad with valid continuations out to the lead's length.
                let len = match lead {
                    0xC0..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    _ => 4,
                };
                let seq = [lead, second, 0x80, 0x80];
                let bytes = &seq[..len];

                let mut state = ACCEPT;
                for &byte in bytes {
                    state = step(state, byte);
                }
                let dfa_ok = state == ACCEPT;
                let std_ok = core::str::from_utf8(bytes).is_ok();

                assert_eq!(
                    dfa_ok, std_ok,
                    "lead {lead:#04X}, second {second:#04X}, seq {bytes:02X?}"
                );
            }
        }
    }

    /// `REJECT` absorbs every byte.
    ///
    /// Not required for correctness given the scan returns early, but it makes
    /// `step` total (so the nibble shift can never leave range) and is the
    /// precondition for any future batched reject check.
    #[test]
    fn reject_is_sticky() {
        for byte in 0..=255u8 {
            assert_eq!(step(REJECT, byte), REJECT, "byte {byte:#04X}");
        }
    }

    /// Every transition lands on a real state, so `step` never shifts past the
    /// packed nibbles.
    #[test]
    fn all_transitions_in_range() {
        for (state, row) in TRANSITIONS.iter().enumerate() {
            for (class, &next) in row.iter().enumerate() {
                assert!(
                    (next as usize) < STATES,
                    "state {state}, class {class} -> {next} is out of range"
                );
            }
        }
    }

    /// Class assignment covers all 256 bytes with the documented ranges.
    #[test]
    fn class_table_covers_all_bytes() {
        for byte in 0..=255u8 {
            let expected = match byte {
                0x00..=0x7F => 0,
                0x80..=0x8F => 1,
                0x90..=0x9F => 9,
                0xA0..=0xBF => 7,
                0xC0..=0xC1 | 0xF5..=0xFF => 8,
                0xC2..=0xDF => 2,
                0xE0 => 10,
                0xE1..=0xEC | 0xEE..=0xEF => 3,
                0xED => 4,
                0xF0 => 11,
                0xF1..=0xF3 => 6,
                0xF4 => 5,
            };
            assert_eq!(class_of(byte), expected, "byte {byte:#04X}");
            assert!((class_of(byte) as usize) < CLASSES);
        }
    }
}
