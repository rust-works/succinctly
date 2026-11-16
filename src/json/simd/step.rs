#![allow(unsafe_code)] // the per-block masks are SIMD intrinsics behind target features
//! The branch-free Standard Cursor kernel the x86_64 AVX2 and SSE2 builders share.
//!
//! [`build_semi_index_standard`] runs [`STEP_TABLE`] over the input, one load and
//! one shift per byte with no branch on the input, and stores each 32-byte block's
//! fields. It leaves only two questions per block to the instruction set, through
//! [`BlockMasks`]: which bytes are `"` or `\` (a block that starts inside a string
//! and holds neither is skipped), and bits 7, 6 and 5 of the 32 stored fields, as
//! the block's IB, BP-open and BP-close masks. AVX2 answers each with one 32-byte
//! `movemask`, SSE2 with two 16-byte ones. Everything else is this one definition,
//! so the two kernels cannot drift apart (#7299 introduced the step for AVX2; #7305
//! moved SSE2 onto it).

use crate::json::standard::{SemiIndex, State, STATES, STEP_CLOSE, STEP_IB, STEP_OPEN, STEP_TABLE};
use crate::json::BitWriter;

/// The two per-block questions [`build_semi_index_standard`] leaves to the
/// instruction set.
pub(super) trait BlockMasks {
    /// Bit `i` is set when `block[i]` is `"` or `\`.
    ///
    /// # Safety
    ///
    /// The implementation's target features must be available.
    unsafe fn specials(block: &[u8; 32]) -> u32;

    /// Bit 7, bit 6 and bit 5 of each `fields[i]`, as bit `i` of three masks:
    /// `[IB, BP open, BP close]`.
    ///
    /// # Safety
    ///
    /// The implementation's target features must be available.
    unsafe fn phi_masks(fields: &[u8; 32]) -> [u32; 3];
}

/// The BP bits four bytes emit, indexed by their open bits (low nibble) and close
/// bits (high nibble): the bits in the low byte, their count in the high byte.
///
/// A byte with both bits writes `10` (open, then close), so only open bits are ever
/// set and a close only advances the count.
static BP_NIBBLE: [u16; 256] = build_bp_nibble();

const fn build_bp_nibble() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut idx = 0;
    while idx < 256 {
        let (open, close) = (idx & 0xF, idx >> 4);
        let (mut bits, mut len) = (0u16, 0u16);
        let mut i = 0;
        while i < 4 {
            if (open >> i) & 1 != 0 {
                bits |= 1 << len;
                len += 1;
            }
            if (close >> i) & 1 != 0 {
                len += 1;
            }
            i += 1;
        }
        table[idx] = bits | len << 8;
        idx += 1;
    }
    table
}

/// A block's BP bits and their count from its open and close masks, byte by byte
/// in input order, an open before a close on the same byte. At most two bits per
/// byte, so 32 bytes fill at most one word.
#[inline(always)]
fn compact_bp(opens: u32, closes: u32) -> (u64, u32) {
    let mut bp_bits: u64 = 0;
    let mut bp_len: u32 = 0;
    let mut k = 0;
    while k < 32 {
        let nibble = ((opens >> k) & 0xF) | ((closes >> k) & 0xF) << 4;
        let entry = BP_NIBBLE[nibble as usize];
        bp_bits |= u64::from(entry & 0xFF) << bp_len;
        bp_len += u32::from(entry >> 8);
        k += 4;
    }
    (bp_bits, bp_len)
}

/// Low five bits of a [`STEP_TABLE`] field when the state is `InString`.
const IN_STRING_SHIFT: u32 = 8 * State::InString as u32;

/// Build a Standard Cursor semi-index, asking `M` the per-block questions.
///
/// Inlined into each instruction set's `#[target_feature]` entry point, so `M`'s
/// methods inline into a function that has their features.
///
/// # Safety
///
/// `M`'s target features must be available.
#[inline(always)]
pub(super) unsafe fn build_semi_index_standard<M: BlockMasks>(json: &[u8]) -> SemiIndex {
    let word_capacity = json.len().div_ceil(64);
    let mut ib = BitWriter::with_capacity(word_capacity);
    let mut bp = BitWriter::with_capacity(word_capacity * 2);
    // The last step's field; its low five bits are `8 * state index`.
    let mut field: u32 = 0;
    let mut fields = [0u8; 32];

    let mut blocks = json.chunks_exact(32);
    for block in &mut blocks {
        // SAFETY: `chunks_exact(32)` yields 32-byte slices, and `[u8; 32]` has
        // the alignment of `u8`.
        let block = unsafe { &*block.as_ptr().cast::<[u8; 32]>() };
        // SAFETY: the caller guarantees `M`'s features.
        if field & 0x1F == IN_STRING_SHIFT && unsafe { M::specials(block) } == 0 {
            // Every byte stays in the string: IB 0, no BP.
            ib.write_zeros(32);
            continue;
        }
        for (slot, &c) in fields.iter_mut().zip(block) {
            field = STEP_TABLE[c as usize].wrapping_shr(field);
            *slot = field as u8;
        }
        // SAFETY: the caller guarantees `M`'s features.
        let [ib_bits, opens, closes] = unsafe { M::phi_masks(&fields) };
        let (bp_bits, bp_len) = compact_bp(opens, closes);
        ib.write_bits(u64::from(ib_bits), 32);
        bp.write_bits(bp_bits, bp_len as usize);
    }

    // Fewer than 32 bytes left: step them one at a time.
    for &c in blocks.remainder() {
        field = STEP_TABLE[c as usize].wrapping_shr(field);
        ib.write_bit((field >> STEP_IB) & 1 != 0);
        if (field >> STEP_OPEN) & 1 != 0 {
            bp.write_1();
        }
        if (field >> STEP_CLOSE) & 1 != 0 {
            bp.write_0();
        }
    }

    SemiIndex {
        state: STATES[((field >> 3) & 3) as usize],
        ib: ib.finish(),
        bp: bp.finish(),
    }
}

/// The per-block questions answered a byte at a time, with no SIMD: the reference
/// the x86 implementations are tested against, and the shared kernel's stand-in
/// on targets with neither (so its block logic is tested on aarch64 too).
#[cfg(test)]
pub(super) struct Portable;

#[cfg(test)]
impl BlockMasks for Portable {
    unsafe fn specials(block: &[u8; 32]) -> u32 {
        (0..32).fold(0, |m, i| {
            m | u32::from(block[i] == b'"' || block[i] == b'\\') << i
        })
    }

    unsafe fn phi_masks(fields: &[u8; 32]) -> [u32; 3] {
        let mut masks = [0u32; 3];
        for (i, &f) in fields.iter().enumerate() {
            for (mask, bit) in masks.iter_mut().zip([STEP_IB, STEP_OPEN, STEP_CLOSE]) {
                *mask |= (u32::from(f) >> bit & 1) << i;
            }
        }
        masks
    }
}

/// The differential suite every kernel built on [`build_semi_index_standard`]
/// runs against the scalar state machine: this module's tests run it over
/// [`Portable`] on every target, and the AVX2 and SSE2 tests over their kernels.
#[cfg(test)]
pub(super) mod suite {
    use super::{compact_bp, BlockMasks, Portable};
    use crate::json::standard::{build_semi_index_scalar, SemiIndex, State, STATES};

    type Build = fn(&[u8]) -> SemiIndex;

    /// A prefix that leaves the scalar machine in state `p` (`STATES` index) after
    /// exactly `64 + off` bytes, or (`p == 4`) `off` bytes of whitespace.
    fn prefix(buf: &mut Vec<u8>, p: usize, off: usize) {
        buf.clear();
        match p {
            0 => buf.extend(std::iter::repeat(b' ').take(64 + off)),
            1 => {
                buf.push(b'"');
                buf.extend(std::iter::repeat(b'x').take(63 + off));
            }
            2 => {
                buf.push(b'"');
                buf.extend(std::iter::repeat(b'x').take(62 + off));
                buf.push(b'\\');
            }
            3 => buf.extend(std::iter::repeat(b'1').take(64 + off)),
            _ => buf.extend(std::iter::repeat(b' ').take(off)),
        }
    }

    /// Bytes that take every distinct path through the state machine.
    const ALPHABET: &[u8] = b"{}[]\",: \\a1-.+e\n\x80\x00";

    fn xorshift(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// Deterministic pseudo-random documents weighted at the kernel's edge cases:
    /// backslash runs straddling block edges, strings ending at a block edge, and
    /// quotes every 30-33 bytes.
    fn random_docs(count: usize, mut seed: u64) -> Vec<Vec<u8>> {
        let next = &mut seed;
        (0..count)
            .map(|_| {
                let len = (xorshift(next) % 400) as usize;
                let mut doc = Vec::with_capacity(len + 80);
                while doc.len() < len {
                    match xorshift(next) % 8 {
                        0 => doc.push(b'"'),
                        1 => {
                            let run = (xorshift(next) % 70) as usize;
                            doc.extend(std::iter::repeat(b'\\').take(run));
                        }
                        2 => {
                            let run = (xorshift(next) % 100) as usize;
                            doc.extend(std::iter::repeat(b'x').take(run));
                        }
                        3 => {
                            let target = (xorshift(next) % 32) as usize;
                            while doc.len() % 32 != target {
                                doc.push(b'y');
                            }
                        }
                        4 => {
                            let gap = 30 + (xorshift(next) % 4) as usize;
                            doc.extend(std::iter::repeat(b'z').take(gap));
                            doc.push(b'"');
                        }
                        5 => {
                            let pick = (xorshift(next) % ALPHABET.len() as u64) as usize;
                            doc.push(ALPHABET[pick]);
                        }
                        6 => doc.extend_from_slice(br#"{"k":[1,-2.5e3,true,null],"s":"a\"b"}"#),
                        _ => doc.push(xorshift(next) as u8),
                    }
                }
                doc.truncate(len);
                doc
            })
            .collect()
    }

    fn assert_matches_scalar(build: Build, json: &[u8]) {
        let got = build(json);
        let want = build_semi_index_scalar(json);
        assert_eq!(got.ib, want.ib, "IB for {json:?}");
        assert_eq!(got.bp, want.bp, "BP for {json:?}");
        assert_eq!(got.state, want.state, "state for {json:?}");
    }

    /// Every byte value, after a prefix ending in each state, at every offset in a
    /// block, followed by nothing, a string run, or a run that ends at a quote.
    pub fn every_byte_state_offset(build: Build) {
        let mut buf = Vec::new();
        let suffixes: [&[u8]; 3] = [b"", &[b'x'; 40], b"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\"  1]"];
        for b in 0..=255u8 {
            for p in 0..5 {
                for off in 0..64 {
                    for suffix in suffixes {
                        prefix(&mut buf, p, off);
                        buf.push(b);
                        buf.extend_from_slice(suffix);
                        assert_matches_scalar(build, &buf);
                    }
                }
            }
        }
    }

    /// Every alphabet byte next to every byte, both orders, after each state, at
    /// the offsets around a block edge.
    pub fn byte_pairs_at_block_edges(build: Build) {
        let mut buf = Vec::new();
        for &a in ALPHABET {
            for b in 0..=255u8 {
                for p in 0..5 {
                    for off in [0, 1, 30, 31, 32, 62, 63] {
                        for pair in [[a, b], [b, a]] {
                            prefix(&mut buf, p, off);
                            buf.extend_from_slice(&pair);
                            assert_matches_scalar(build, &buf);
                        }
                    }
                }
            }
        }
    }

    /// Every string of up to three alphabet bytes, after each state, at the offsets
    /// around a block edge.
    pub fn exhaustive_short_strings(build: Build) {
        let a = ALPHABET.len();
        let mut buf = Vec::new();
        for len in 0..=3u32 {
            for idx in 0..a.pow(len) {
                for p in 0..5 {
                    for off in [0, 29, 31, 32, 63] {
                        prefix(&mut buf, p, off);
                        let mut x = idx;
                        for _ in 0..len {
                            buf.push(ALPHABET[x % a]);
                            x /= a;
                        }
                        assert_matches_scalar(build, &buf);
                    }
                }
            }
        }
    }

    /// Random documents, and a prefix and a suffix of each.
    pub fn random_documents(build: Build) {
        for doc in random_docs(3000, 0x7305_7299) {
            assert_matches_scalar(build, &doc);
            for cut in [doc.len() / 3, doc.len() / 2, doc.len().saturating_sub(1)] {
                assert_matches_scalar(build, &doc[..cut]);
                assert_matches_scalar(build, &doc[cut..]);
            }
        }
    }

    /// The skip applies only in `InString`: a block that starts in `InEscape` and
    /// holds no quote or backslash still escapes its first byte, so the quote that
    /// opens the next block closes the string instead of being escaped.
    pub fn no_skip_in_escape_state(build: Build) {
        let mut json = b"[\"".to_vec();
        json.extend([b'x'; 29]);
        json.push(b'\\'); // byte 31: block 0 ends in InEscape
        json.extend([b'y'; 32]); // block 1: no quote or backslash
        json.extend_from_slice(b"\",1]"); // block 2: closes the string
        assert_matches_scalar(build, &json);
        assert_eq!(build(&json).state, State::InJson);
        assert_eq!(build(&json).bp, vec![0b01011], "[, string, 1, ]");
    }

    /// The last byte of every tail length is stepped: a closing bracket at the end
    /// of 1-31 bytes past the last full block.
    pub fn tail_last_byte(build: Build) {
        for blocks in 0..3 {
            for tail in 1..32 {
                if 32 * blocks + tail < 2 {
                    continue;
                }
                let mut json = vec![b'['];
                json.resize(32 * blocks + tail - 1, b' ');
                json.push(b']');
                assert_matches_scalar(build, &json);
                assert_eq!(build(&json).bp, vec![0b01], "BP for {json:?}");
            }
        }
    }

    /// Opens write `1` and closes `0`, in input order, inside a block and across
    /// its edge.
    pub fn open_close_order(build: Build) {
        for pad in 0..40 {
            let mut json = vec![b' '; pad];
            json.extend_from_slice(b"[{}],[]");
            assert_matches_scalar(build, &json);
            assert_eq!(build(&json).bp, vec![0b01_0011], "BP, pad {pad}");
        }
    }

    /// The state carries across every block edge and comes out right at the end:
    /// each of the four states straddles the edge, in a full block and in the tail.
    pub fn carried_and_final_state(build: Build) {
        let mut buf = Vec::new();
        for (p, &state) in STATES.iter().enumerate() {
            for off in [0, 31, 32] {
                prefix(&mut buf, p, off);
                assert_eq!(build(&buf).state, state, "end state, prefix {p} off {off}");
                assert_matches_scalar(build, &buf);
                for next in *b"\"\\x1 ]" {
                    let mut doc = buf.clone();
                    doc.push(next);
                    doc.extend([b'y'; 40]);
                    assert_matches_scalar(build, &doc);
                }
            }
        }
    }

    /// Every check above, for one kernel.
    pub fn all(build: Build) {
        every_byte_state_offset(build);
        byte_pairs_at_block_edges(build);
        exhaustive_short_strings(build);
        random_documents(build);
        no_skip_in_escape_state(build);
        tail_last_byte(build);
        open_close_order(build);
        carried_and_final_state(build);
    }

    /// `M`'s answers are [`Portable`]'s, on every single-byte block and on random
    /// blocks: the instruction sets agree on the questions the shared kernel asks.
    ///
    /// # Safety
    ///
    /// `M`'s target features must be available.
    pub unsafe fn masks_agree<M: BlockMasks>() {
        let mut blocks: Vec<[u8; 32]> = Vec::new();
        for b in 0..=255u8 {
            for i in 0..32 {
                let mut block = [0u8; 32];
                block[i] = b;
                blocks.push(block);
                blocks.push([b; 32]);
            }
        }
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        for _ in 0..20_000 {
            let mut block = [0u8; 32];
            for byte in &mut block {
                *byte = xorshift(&mut seed) as u8;
            }
            blocks.push(block);
        }
        for block in &blocks {
            // SAFETY: the caller guarantees `M`'s features; `Portable` needs none.
            unsafe {
                assert_eq!(M::specials(block), Portable::specials(block), "{block:?}");
                assert_eq!(M::phi_masks(block), Portable::phi_masks(block), "{block:?}");
            }
        }
    }

    /// [`compact_bp`] against a bit-at-a-time model.
    pub fn compact_bp_matches_model() {
        assert_eq!(compact_bp(0b1, 0b0), (0b1, 1), "open");
        assert_eq!(compact_bp(0b0, 0b1), (0b0, 1), "close");
        assert_eq!(compact_bp(0b1, 0b1), (0b01, 2), "leaf: open then close");
        assert_eq!(compact_bp(0b01, 0b10), (0b01, 2), "open, then close");
        assert_eq!(compact_bp(0b10, 0b01), (0b10, 2), "close, then open");
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        for _ in 0..20_000 {
            let r = xorshift(&mut seed);
            let (opens, closes) = (r as u32, (r >> 32) as u32);
            let (mut bits, mut len) = (0u64, 0u32);
            for i in 0..32 {
                if opens >> i & 1 != 0 {
                    bits |= 1 << len;
                    len += 1;
                }
                if closes >> i & 1 != 0 {
                    len += 1;
                }
            }
            assert_eq!(
                compact_bp(opens, closes),
                (bits, len),
                "{opens:#x} {closes:#x}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portable(json: &[u8]) -> SemiIndex {
        // SAFETY: `Portable` needs no target features.
        unsafe { build_semi_index_standard::<Portable>(json) }
    }

    #[test]
    fn test_shared_kernel_matches_scalar() {
        suite::all(portable);
    }

    #[test]
    fn test_compact_bp_matches_model() {
        suite::compact_bp_matches_model();
    }
}
