//! Balanced parentheses operations for succinct tree navigation.
//!
//! This module provides efficient operations on balanced parentheses (BP) sequences,
//! which are a fundamental building block for succinct tree representations.
//!
//! In BP encoding:
//! - `1` = open parenthesis `(`
//! - `0` = close parenthesis `)`
//!
//! Bit positions are indexed from LSB (position 0) to MSB (position 63).
//!
//! # Key Operations
//!
//! | Operation | Description |
//! |-----------|-------------|
//! | `find_close(i)` | Find position of `)` matching `(` at position `i` |
//! | `find_open(i)` | Find position of `(` matching `)` at position `i` |
//! | `enclose(i)` | Find position of `(` enclosing the node at position `i` (parent) |
//!
//! # Implementation
//!
//! Two implementations are provided:
//!
//! 1. **Word-level operations** (`find_close_in_word`): O(w) per word using bit scanning
//! 2. **RangeMin structure** (`BalancedParens`): O(1) operations using hierarchical min-excess
//!
//! Based on:
//! - Sadakane & Navarro, "Fully-Functional Succinct Trees", SODA 2010

#[cfg(not(test))]
use alloc::vec::Vec;

// ============================================================================
// Phase 1: Word-Level Operations
// ============================================================================

/// Find position of first unmatched close parenthesis in a 64-bit word.
///
/// Returns bit position (0-63) of the first `)` that doesn't have a matching `(`
/// to its left within the word. Returns 64 if no unmatched close exists.
///
/// Bits are: 1 = open `(`, 0 = close `)`
///
/// # Example
///
/// ```
/// use succinctly::bp::find_unmatched_close_in_word;
///
/// // All opens - no unmatched close
/// assert_eq!(find_unmatched_close_in_word(u64::MAX), 64);
///
/// // All closes - unmatched at position 0
/// assert_eq!(find_unmatched_close_in_word(0), 0);
///
/// // "()" padded with opens - no unmatched close
/// let pattern = 0b01 | (u64::MAX << 2);
/// assert_eq!(find_unmatched_close_in_word(pattern), 64);
/// ```
#[inline]
pub fn find_unmatched_close_in_word(x: u64) -> u32 {
    // Simple linear scan tracking excess
    let mut excess: i32 = 0;

    for bit in 0..64 {
        if (x >> bit) & 1 == 1 {
            excess += 1; // open
        } else {
            excess -= 1; // close
            if excess < 0 {
                return bit;
            }
        }
    }

    64 // No unmatched close found
}

/// Find matching close parenthesis within a single word.
///
/// Given an open parenthesis at bit position `p` (0-indexed from LSB),
/// returns the position of its matching close.
///
/// Returns `None` if the matching close is beyond this word.
///
/// # Example
///
/// ```
/// use succinctly::bp::find_close_in_word;
///
/// // "()" = 0b01 - open at 0 matches close at 1
/// assert_eq!(find_close_in_word(0b01, 0), Some(1));
///
/// // "(())" = 0b0011 - open at 0 matches close at 3
/// assert_eq!(find_close_in_word(0b0011, 0), Some(3));
///
/// // "(())" = 0b0011 - open at 1 matches close at 2
/// assert_eq!(find_close_in_word(0b0011, 1), Some(2));
/// ```
#[inline]
pub fn find_close_in_word(word: u64, p: u32) -> Option<u32> {
    if p >= 64 {
        return None;
    }

    // Shift so position p is at bit 0
    let shifted = word >> p;

    // If bit at p is 0 (close), it "matches itself" (degenerate case)
    if shifted & 1 == 0 {
        return Some(p);
    }

    // Skip the open at position 0, find first unmatched close after it
    let after_open = shifted >> 1;
    let remaining_bits = 63 - p;

    if remaining_bits == 0 {
        return None;
    }

    let result = find_unmatched_close_in_word(after_open);

    if result < remaining_bits {
        Some(p + 1 + result)
    } else {
        None // Match is in a later word
    }
}

// ============================================================================
// Phase 1: Vector-Level Linear Scan
// ============================================================================

/// Find matching close parenthesis in a bitvector (linear scan).
///
/// Scans word-by-word from position `p` until finding the matching close.
/// Time: O(distance to matching close / 64)
///
/// # Arguments
///
/// * `words` - The bit vector words (1=open, 0=close)
/// * `len` - Total number of valid bits
/// * `p` - Position of the open parenthesis to match
///
/// # Returns
///
/// Position of the matching close, or `None` if:
/// - `p` is out of bounds
/// - Position `p` is a close parenthesis
/// - No matching close exists (unbalanced)
///
/// # Example
///
/// ```
/// use succinctly::bp::find_close;
///
/// // "(()())" = bit0=1, bit1=1, bit2=0, bit3=1, bit4=0, bit5=0 = 0b001011
/// let words = vec![0b001011u64];
/// assert_eq!(find_close(&words, 6, 0), Some(5)); // outer pair
/// assert_eq!(find_close(&words, 6, 1), Some(2)); // first inner pair
/// assert_eq!(find_close(&words, 6, 3), Some(4)); // second inner pair
/// ```
pub fn find_close(words: &[u64], len: usize, p: usize) -> Option<usize> {
    if p >= len || words.is_empty() {
        return None;
    }

    let word_idx = p / 64;
    let bit_idx = (p % 64) as u32;

    // Check if position p is an open parenthesis
    if (words[word_idx] >> bit_idx) & 1 == 0 {
        return None; // Position p is a close, not an open
    }

    // Try to find match in the same word
    if let Some(local) = find_close_in_word(words[word_idx], bit_idx) {
        let result = word_idx * 64 + local as usize;
        if result < len {
            return Some(result);
        }
    }

    // Calculate excess from the partial first word
    let first_word_part = words[word_idx] >> bit_idx;
    let first_word_bits = (64 - bit_idx) as usize;
    let first_word_ones = first_word_part.count_ones() as i32;
    let mut excess = 2 * first_word_ones - first_word_bits as i32;
    // We started with excess = 1 (from the open at p), so adjust
    // Actually, we want excess = 1 after seeing the open at p
    // The first word after bit p has: excess = opens - closes = 2*opens - bits

    // Scan subsequent words
    for (i, &word) in words[word_idx + 1..].iter().enumerate() {
        let actual_word_idx = word_idx + 1 + i;
        let word_bits = if actual_word_idx * 64 + 64 <= len {
            64
        } else {
            len - actual_word_idx * 64
        };

        // Mask the word to only valid bits
        let masked_word = if word_bits == 64 {
            word
        } else {
            word & ((1u64 << word_bits) - 1)
        };

        // Compute the minimum excess reached within this word
        let (word_min, word_excess) = word_min_excess_i32(masked_word, word_bits);

        // Check if excess might drop to 0 within this word
        // The minimum excess reached is: excess + word_min
        if excess + word_min <= 0 {
            // The match might be in this word - do fine-grained search
            let mut bit_excess = excess;
            for bit in 0..word_bits {
                if (masked_word >> bit) & 1 == 1 {
                    bit_excess += 1;
                } else {
                    bit_excess -= 1;
                    if bit_excess == 0 {
                        return Some(actual_word_idx * 64 + bit);
                    }
                }
            }
        }

        excess += word_excess;

        if actual_word_idx * 64 >= len {
            break;
        }
    }

    None
}

/// Helper function to compute min excess and total excess for a word (i32 version).
fn word_min_excess_i32(word: u64, valid_bits: usize) -> (i32, i32) {
    let mut excess: i32 = 0;
    let mut min_excess: i32 = 0;

    for i in 0..valid_bits {
        if (word >> i) & 1 == 1 {
            excess += 1; // open
        } else {
            excess -= 1; // close
            min_excess = min_excess.min(excess);
        }
    }

    (min_excess, excess)
}

/// Find matching open parenthesis (scanning backwards).
///
/// Given a close parenthesis at position `p`, find the matching open.
///
/// # Arguments
///
/// * `words` - The bit vector words (1=open, 0=close)
/// * `len` - Total number of valid bits
/// * `p` - Position of the close parenthesis to match
///
/// # Returns
///
/// Position of the matching open, or `None` if:
/// - `p` is out of bounds
/// - Position `p` is an open parenthesis
/// - No matching open exists
pub fn find_open(words: &[u64], len: usize, p: usize) -> Option<usize> {
    if p >= len || words.is_empty() {
        return None;
    }

    let word_idx = p / 64;
    let bit_idx = p % 64;

    // Check if position p is a close parenthesis
    if (words[word_idx] >> bit_idx) & 1 == 1 {
        return None; // Position p is an open, not a close
    }

    // Scan backwards, tracking excess
    // We start with excess = -1 (we just saw a close)
    // We want to find where excess becomes 0 (the matching open)
    let mut excess: i32 = -1;

    // First, scan backwards in the current word
    for bit in (0..bit_idx).rev() {
        if (words[word_idx] >> bit) & 1 == 1 {
            excess += 1; // open
            if excess == 0 {
                return Some(word_idx * 64 + bit);
            }
        } else {
            excess -= 1; // close
        }
    }

    // Scan previous words
    for word_idx in (0..word_idx).rev() {
        let word = words[word_idx];
        let word_ones = word.count_ones() as i32;
        let word_excess = 2 * word_ones - 64;

        // Check if the match might be in this word
        if excess + word_excess >= 0 {
            // Scan this word bit by bit (backwards)
            for bit in (0..64).rev() {
                if (word >> bit) & 1 == 1 {
                    excess += 1;
                    if excess == 0 {
                        return Some(word_idx * 64 + bit);
                    }
                } else {
                    excess -= 1;
                }
            }
        } else {
            excess += word_excess;
        }
    }

    None
}

/// Find the enclosing open parenthesis (parent node).
///
/// Given an open parenthesis at position `p`, find the open parenthesis
/// that encloses it (i.e., the parent in the tree).
///
/// # Returns
///
/// Position of the enclosing open, or `None` if `p` is at the root.
pub fn enclose(words: &[u64], len: usize, p: usize) -> Option<usize> {
    if p == 0 || p >= len || words.is_empty() {
        return None;
    }

    // Check if position p is an open parenthesis
    let word_idx = p / 64;
    let bit_idx = p % 64;
    if (words[word_idx] >> bit_idx) & 1 == 0 {
        return None; // Position p is a close, not an open
    }

    // We want to find the first unmatched open before position p
    // This is equivalent to find_open(p-1) if p-1 is a close,
    // or finding where excess goes to -1 scanning backwards

    let mut excess: i32 = 0;

    // Scan backwards from position p-1
    let start_bit = if bit_idx > 0 { bit_idx - 1 } else { 63 };
    let start_word = if bit_idx > 0 {
        word_idx
    } else {
        word_idx.saturating_sub(1)
    };

    if word_idx == 0 && bit_idx == 0 {
        return None; // At the very beginning, no enclosing parent
    }

    // First, scan backwards in the starting word
    let first_word = words[start_word];
    for bit in (0..=start_bit).rev() {
        if (first_word >> bit) & 1 == 1 {
            excess += 1; // open
            if excess == 1 {
                return Some(start_word * 64 + bit);
            }
        } else {
            excess -= 1; // close
        }
    }

    // Scan previous words
    for word_idx in (0..start_word).rev() {
        let word = words[word_idx];
        let word_ones = word.count_ones() as i32;
        let word_excess = 2 * word_ones - 64;

        // Check if the match might be in this word
        if excess + word_excess >= 1 {
            for bit in (0..64).rev() {
                if (word >> bit) & 1 == 1 {
                    excess += 1;
                    if excess == 1 {
                        return Some(word_idx * 64 + bit);
                    }
                } else {
                    excess -= 1;
                }
            }
        } else {
            excess += word_excess;
        }
    }

    None
}

// ============================================================================
// Phase 2: RangeMin Structure for O(1) Operations
// ============================================================================

/// Balanced parentheses with O(1) navigation operations.
///
/// Stores auxiliary min-excess data at multiple levels for fast find_close/find_open.
/// Based on the RangeMin data structure from haskell-works.
///
/// # Space Overhead
///
/// Approximately 6% overhead:
/// - L0: 2 bytes per word (min excess + cumulative excess)
/// - L1: 4 bytes per 32 words
/// - L2: 4 bytes per 1024 words
///
/// # Example
///
/// ```
/// use succinctly::bp::BalancedParens;
///
/// // "(()())" = bit0=1, bit1=1, bit2=0, bit3=1, bit4=0, bit5=0 = 0b001011
/// let bp = BalancedParens::new(vec![0b001011u64], 6);
///
/// // Find matching close for each open
/// assert_eq!(bp.find_close(0), Some(5)); // outer pair
/// assert_eq!(bp.find_close(1), Some(2)); // first inner pair
/// assert_eq!(bp.find_close(3), Some(4)); // second inner pair
/// ```
#[derive(Clone, Debug)]
pub struct BalancedParens {
    /// The underlying bit vector (1=open, 0=close)
    words: Vec<u64>,
    /// Number of valid bits
    len: usize,

    /// L0: Per-word min excess (signed, relative to start of word)
    l0_min_excess: Vec<i8>,
    /// L0: Per-word cumulative excess (total excess at end of word)
    l0_cum_excess: Vec<i16>,

    /// L1: Per-32-word block min excess
    l1_min_excess: Vec<i16>,
    /// L1: Per-32-word block cumulative excess
    l1_cum_excess: Vec<i16>,

    /// L2: Per-1024-word block min excess
    l2_min_excess: Vec<i16>,
    /// L2: Per-1024-word block cumulative excess
    l2_cum_excess: Vec<i16>,
}

/// L1 factor: words per L1 block
const FACTOR_L1: usize = 32;
/// L2 factor: L1 blocks per L2 block
const FACTOR_L2: usize = 32;

impl BalancedParens {
    /// Build from a bitvector representing balanced parentheses.
    ///
    /// # Arguments
    ///
    /// * `words` - Bit vector where 1=open, 0=close
    /// * `len` - Number of valid bits
    pub fn new(words: Vec<u64>, len: usize) -> Self {
        if words.is_empty() || len == 0 {
            return Self {
                words: Vec::new(),
                len: 0,
                l0_min_excess: Vec::new(),
                l0_cum_excess: Vec::new(),
                l1_min_excess: Vec::new(),
                l1_cum_excess: Vec::new(),
                l2_min_excess: Vec::new(),
                l2_cum_excess: Vec::new(),
            };
        }

        let num_words = words.len();
        let num_l1 = num_words.div_ceil(FACTOR_L1);
        let num_l2 = num_l1.div_ceil(FACTOR_L2);

        // Build L0: per-word statistics
        let mut l0_min_excess = Vec::with_capacity(num_words);
        let mut l0_cum_excess = Vec::with_capacity(num_words);

        for (i, &word) in words.iter().enumerate() {
            let valid_bits = if i == num_words - 1 && !len.is_multiple_of(64) {
                len % 64
            } else {
                64
            };
            let (min_e, total_e) = word_min_excess(word, valid_bits);
            l0_min_excess.push(min_e);
            l0_cum_excess.push(total_e);
        }

        // Build L1: per-32-word block statistics
        let mut l1_min_excess = Vec::with_capacity(num_l1);
        let mut l1_cum_excess = Vec::with_capacity(num_l1);

        for block_idx in 0..num_l1 {
            let start = block_idx * FACTOR_L1;
            let end = (start + FACTOR_L1).min(num_words);

            let mut block_min: i16 = 0;
            let mut running_excess: i16 = 0;

            for i in start..end {
                let word_min = l0_min_excess[i] as i16;
                let word_excess = l0_cum_excess[i];

                // min_excess at word i = running_excess + word_min
                block_min = block_min.min(running_excess + word_min);
                running_excess += word_excess;
            }

            l1_min_excess.push(block_min);
            l1_cum_excess.push(running_excess);
        }

        // Build L2: per-1024-word block statistics
        let mut l2_min_excess = Vec::with_capacity(num_l2);
        let mut l2_cum_excess = Vec::with_capacity(num_l2);

        for block_idx in 0..num_l2 {
            let start = block_idx * FACTOR_L2;
            let end = (start + FACTOR_L2).min(num_l1);

            let mut block_min: i16 = 0;
            let mut running_excess: i16 = 0;

            for i in start..end {
                let l1_min = l1_min_excess[i];
                let l1_excess = l1_cum_excess[i];

                block_min = block_min.min(running_excess + l1_min);
                running_excess += l1_excess;
            }

            l2_min_excess.push(block_min);
            l2_cum_excess.push(running_excess);
        }

        Self {
            words,
            len,
            l0_min_excess,
            l0_cum_excess,
            l1_min_excess,
            l1_cum_excess,
            l2_min_excess,
            l2_cum_excess,
        }
    }

    /// Number of bits in the bitvector.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the bitvector is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Check if position `p` is an open parenthesis.
    #[inline]
    pub fn is_open(&self, p: usize) -> bool {
        if p >= self.len {
            return false;
        }
        (self.words[p / 64] >> (p % 64)) & 1 == 1
    }

    /// Check if position `p` is a close parenthesis.
    #[inline]
    pub fn is_close(&self, p: usize) -> bool {
        if p >= self.len {
            return false;
        }
        (self.words[p / 64] >> (p % 64)) & 1 == 0
    }

    /// Find matching close parenthesis.
    ///
    /// Given an open parenthesis at position `p`, returns the position
    /// of its matching close parenthesis.
    ///
    /// Returns `None` if:
    /// - `p` is out of bounds
    /// - Position `p` is a close parenthesis
    /// - The sequence is unbalanced
    pub fn find_close(&self, p: usize) -> Option<usize> {
        if p >= self.len {
            return None;
        }

        // If position p is a close, return None
        if self.is_close(p) {
            return None;
        }

        // Use accelerated search with RangeMin indices
        // Start after the open at position p with excess = 1
        self.find_close_from(p + 1, 1)
    }

    /// Internal: find position where excess drops to 0, starting from given position
    /// with given initial excess.
    ///
    /// This implements the RangeMin algorithm from Sadakane & Navarro using a state machine
    /// approach, matching the Haskell hw-balancedparens implementation (rm2FindClose).
    ///
    /// The key insight is that we can skip entire blocks (words, L1 blocks, L2 blocks)
    /// if we know the minimum excess in that block won't bring us to 0.
    ///
    /// State machine:
    /// - ScanBit: Process one bit at a time
    /// - CheckL0/L1/L2: Check if match is in this block (using min_excess)
    /// - FromL0/L1/L2: Decide next level based on position alignment
    fn find_close_from(&self, start_pos: usize, initial_excess: i32) -> Option<usize> {
        if start_pos >= self.len {
            return None;
        }

        // State machine states matching Haskell's FindState
        #[derive(Clone, Copy, Debug)]
        enum State {
            ScanBit, // FindBP: Scan bit-by-bit
            CheckL0, // FindL0: Check L0 (word) index
            CheckL1, // FindL1: Check L1 (32-word block) index
            CheckL2, // FindL2: Check L2 (1024-word block) index
            FromL0,  // FindFromL0: Transition from L0 - decide next level
            FromL1,  // FindFromL1: Transition from L1 - decide next level
            FromL2,  // FindFromL2: Transition from L2 - decide next level
        }

        let mut state = State::FromL0;
        let mut excess = initial_excess;
        let mut pos = start_pos;

        loop {
            match state {
                State::ScanBit => {
                    // Haskell's FindBP: Scan single bit, then transition to FromL0
                    if pos >= self.len {
                        return None;
                    }

                    let word_idx = pos / 64;
                    let bit_idx = pos % 64;
                    let bit = (self.words[word_idx] >> bit_idx) & 1;

                    if bit == 0 {
                        // Close paren
                        excess -= 1;
                        if excess == 0 {
                            return Some(pos);
                        }
                    } else {
                        // Open paren
                        excess += 1;
                    }
                    pos += 1;
                    state = State::FromL0;
                }

                State::CheckL0 => {
                    // Haskell's FindL0: Check L0 (word) index
                    // IMPORTANT: Only enter this state when pos is word-aligned!
                    debug_assert!(
                        pos.is_multiple_of(64),
                        "CheckL0 requires word-aligned position"
                    );

                    let word_idx = pos / 64;
                    if word_idx >= self.l0_min_excess.len() {
                        return None;
                    }

                    let min_e = self.l0_min_excess[word_idx] as i32;
                    if excess + min_e <= 0 {
                        // Match might be in this word - scan bit by bit
                        state = State::ScanBit;
                    } else {
                        // Match not in this word - skip entire word
                        // Check for immediate match first (Haskell does this too)
                        if pos < self.len && self.is_close(pos) && excess <= 1 {
                            return Some(pos);
                        }
                        let word_excess = self.l0_cum_excess[word_idx] as i32;
                        excess += word_excess;
                        pos += 64;
                        state = State::FromL0;
                    }
                }

                State::CheckL1 => {
                    // Haskell's FindL1: Check L1 (32-word block) index
                    // IMPORTANT: Only enter this state when pos is word-aligned!
                    debug_assert!(
                        pos.is_multiple_of(64),
                        "CheckL1 requires word-aligned position"
                    );

                    let l1_idx = pos / (64 * FACTOR_L1);
                    if l1_idx >= self.l1_min_excess.len() {
                        return None;
                    }

                    let min_e = self.l1_min_excess[l1_idx] as i32;
                    if excess + min_e <= 0 {
                        // Match might be in this L1 block - descend to L0
                        state = State::CheckL0;
                    } else if pos < self.len {
                        // Match not in this L1 block - skip it
                        if self.is_close(pos) && excess <= 1 {
                            return Some(pos);
                        }
                        let l1_excess = self.l1_cum_excess[l1_idx] as i32;
                        excess += l1_excess;
                        pos += 64 * FACTOR_L1;
                        state = State::FromL1;
                    } else {
                        return None;
                    }
                }

                State::CheckL2 => {
                    // Haskell's FindL2: Check L2 (1024-word block) index
                    let l2_idx = pos / (64 * FACTOR_L1 * FACTOR_L2);
                    if l2_idx >= self.l2_min_excess.len() {
                        return None;
                    }

                    let min_e = self.l2_min_excess[l2_idx] as i32;
                    if excess + min_e <= 0 {
                        // Match might be in this L2 block - descend to L1
                        state = State::CheckL1;
                    } else if pos < self.len {
                        // Match not in this L2 block - skip it
                        if self.is_close(pos) && excess <= 1 {
                            return Some(pos);
                        }
                        let l2_excess = self.l2_cum_excess[l2_idx] as i32;
                        excess += l2_excess;
                        pos += 64 * FACTOR_L1 * FACTOR_L2;
                        state = State::FromL2;
                    } else {
                        return None;
                    }
                }

                State::FromL0 => {
                    // Haskell's FindFromL0: At word boundary? Try L1, else scan bits
                    if pos.is_multiple_of(64) {
                        state = State::FromL1;
                    } else if pos < self.len {
                        state = State::ScanBit;
                    } else {
                        return None;
                    }
                }

                State::FromL1 => {
                    // Haskell's FindFromL1: At L1 boundary? Try L2, else check L0
                    if pos.is_multiple_of(64 * FACTOR_L1) {
                        if pos < self.len {
                            state = State::FromL2;
                        } else {
                            return None;
                        }
                    } else if pos < self.len {
                        // Not at L1 boundary, but must be at word boundary (came from FromL0)
                        state = State::CheckL0;
                    } else {
                        return None;
                    }
                }

                State::FromL2 => {
                    // Haskell's FindFromL2: At L2 boundary? Check L2, else check L1
                    if pos.is_multiple_of(64 * FACTOR_L1 * FACTOR_L2) {
                        if pos < self.len {
                            state = State::CheckL2;
                        } else {
                            return None;
                        }
                    } else if pos < self.len {
                        state = State::CheckL1;
                    } else {
                        return None;
                    }
                }
            }
        }
    }

    /// Find matching open parenthesis.
    ///
    /// Given a close parenthesis at position `p`, returns the position
    /// of its matching open parenthesis.
    pub fn find_open(&self, p: usize) -> Option<usize> {
        if p >= self.len || self.is_open(p) {
            return None;
        }

        find_open(&self.words, self.len, p)
    }

    /// Find enclosing open parenthesis (parent node).
    ///
    /// Given an open parenthesis at position `p`, returns the position
    /// of the open parenthesis that encloses it.
    pub fn enclose(&self, p: usize) -> Option<usize> {
        if p >= self.len || self.is_close(p) {
            return None;
        }

        enclose(&self.words, self.len, p)
    }

    /// Navigate to first child in tree.
    ///
    /// If position `p` is an open parenthesis and is immediately followed
    /// by another open, returns that position (the first child).
    pub fn first_child(&self, p: usize) -> Option<usize> {
        if p + 1 >= self.len {
            return None;
        }
        if self.is_open(p) && self.is_open(p + 1) {
            Some(p + 1)
        } else {
            None
        }
    }

    /// Navigate to next sibling in tree.
    ///
    /// Returns the position of the next sibling's open parenthesis,
    /// or `None` if this is the last sibling.
    pub fn next_sibling(&self, p: usize) -> Option<usize> {
        if !self.is_open(p) {
            return None;
        }
        let close = self.find_close(p)?;
        if close + 1 < self.len && self.is_open(close + 1) {
            Some(close + 1)
        } else {
            None
        }
    }

    /// Navigate to parent node.
    ///
    /// Alias for `enclose`.
    pub fn parent(&self, p: usize) -> Option<usize> {
        self.enclose(p)
    }

    /// Get excess at position p (number of opens minus closes in [0, p]).
    pub fn excess(&self, p: usize) -> i32 {
        if p >= self.len {
            return 0;
        }

        let word_idx = p / 64;
        let bit_idx = p % 64;

        // Sum up cumulative excess from previous words
        let mut total: i32 = 0;
        for i in 0..word_idx {
            total += self.l0_cum_excess[i] as i32;
        }

        // Add partial word
        let word = self.words[word_idx];
        for bit in 0..=bit_idx {
            if (word >> bit) & 1 == 1 {
                total += 1;
            } else {
                total -= 1;
            }
        }

        total
    }

    /// Get depth of node at position p.
    ///
    /// The depth is the number of ancestors (including the node itself).
    /// For a root node (position 0), depth is 1.
    /// Returns `None` if position is out of bounds.
    ///
    /// # Example
    ///
    /// For the sequence "(()(()())) = 1101101000":
    /// - depth(1) = 1 (root)
    /// - depth(2) = 2 (first child)
    /// - depth(5) = 3 (grandchild)
    pub fn depth(&self, p: usize) -> Option<usize> {
        if p >= self.len {
            return None;
        }

        // Depth is the excess at position p (for an open) or excess at the matching open
        // For 1-indexed positions in Haskell, depth = excess
        // For 0-indexed, we compute excess up to and including position p
        // The excess at an open parenthesis equals its depth
        Some(self.excess(p) as usize)
    }

    /// Get the size of the subtree rooted at position p.
    ///
    /// The subtree size is the number of nodes (opens) in the subtree,
    /// not including the node at p itself.
    /// Returns `None` if position is out of bounds or not an open.
    ///
    /// # Example
    ///
    /// For the sequence "(()(()())) = 1101101000":
    /// - subtree_size(1) = 5 (root has 5 descendants: 2 children + 2 grandchildren + their closes)
    /// - subtree_size(2) = 1 (leaf node, subtree size = 1 for the () pair)
    pub fn subtree_size(&self, p: usize) -> Option<usize> {
        if p >= self.len || self.is_close(p) {
            return None;
        }

        // Subtree size = (find_close(p) - p - 1) / 2
        // The distance to matching close, minus 1 for the open itself,
        // divided by 2 since each node has an open and close.
        let close = self.find_close(p)?;
        Some((close - p) / 2)
    }
}

/// Compute minimum excess and total excess for a single word.
///
/// Returns (min_excess, total_excess) where:
/// - min_excess: The minimum excess reached at any point in the word (negative if unmatched closes)
/// - total_excess: The total excess at the end of the word (opens - closes)
fn word_min_excess(word: u64, valid_bits: usize) -> (i8, i16) {
    let mut excess: i16 = 0;
    let mut min_excess: i16 = 0;

    for i in 0..valid_bits {
        if (word >> i) & 1 == 1 {
            excess += 1; // open
        } else {
            excess -= 1; // close
            min_excess = min_excess.min(excess);
        }
    }

    // Clamp min_excess to i8 range (should be fine for 64-bit words)
    let min_clamped = min_excess.clamp(-128, 127) as i8;

    (min_clamped, excess)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Phase 1: Word-level tests
    // ========================================================================

    // Bit pattern reference:
    // "()" = open at pos 0, close at pos 1 = 0b01 (bit 0 = 1, bit 1 = 0)
    // "(())" = open at 0, open at 1, close at 2, close at 3 = 0b0011
    // "(()())" = open at 0, open at 1, close at 2, open at 3, close at 4, close at 5
    //         = bits: 1 1 0 1 0 0 = 0b001011

    #[test]
    fn test_find_unmatched_close_simple() {
        // Note: find_unmatched_close_in_word scans all 64 bits.
        // Unused high bits are 0s (closes), so small test values need padding.
        // Use u64::MAX with low bits cleared to represent "opens" in unused positions.

        // Full word of closes - first unmatched at position 0
        assert_eq!(find_unmatched_close_in_word(0), 0);

        // Full word of opens - no unmatched close
        assert_eq!(find_unmatched_close_in_word(u64::MAX), 64);

        // "()" padded with opens: opens in high bits
        // Positions 0-1: "()", positions 2-63: all opens
        // 0b01 | (all ones from bit 2 onwards) = 0xFFFF_FFFF_FFFF_FFFD
        let pattern = 0b01 | (u64::MAX << 2);
        assert_eq!(find_unmatched_close_in_word(pattern), 64); // balanced prefix, opens after
    }

    #[test]
    fn test_find_unmatched_close_nested() {
        // "(())" padded with opens in high bits
        // Pattern: 0b0011 in low 4 bits, opens (1s) in bits 4-63
        let pattern = 0b0011 | (u64::MAX << 4);
        assert_eq!(find_unmatched_close_in_word(pattern), 64);

        // "(()" = 0b011 padded with opens
        let pattern = 0b011 | (u64::MAX << 3);
        assert_eq!(find_unmatched_close_in_word(pattern), 64);
    }

    #[test]
    fn test_find_unmatched_close_with_extra() {
        // "())" padded with opens in high bits
        // Pattern: 0b001 in low 3 bits (open, close, close)
        // Unmatched close at position 2
        let pattern = 0b001 | (u64::MAX << 3);
        assert_eq!(find_unmatched_close_in_word(pattern), 2);

        // "(()))" padded with opens
        // Pattern: 0b00011 in low 5 bits
        // Unmatched close at position 4
        let pattern = 0b00011 | (u64::MAX << 5);
        assert_eq!(find_unmatched_close_in_word(pattern), 4);
    }

    #[test]
    fn test_find_close_in_word_simple() {
        // "()" = 0b01 - open at 0, close at 1
        assert_eq!(find_close_in_word(0b01, 0), Some(1));
    }

    #[test]
    fn test_find_close_in_word_nested() {
        // "(())" = 0b0011 - opens at 0,1; closes at 2,3
        assert_eq!(find_close_in_word(0b0011, 0), Some(3)); // outer
        assert_eq!(find_close_in_word(0b0011, 1), Some(2)); // inner
    }

    #[test]
    fn test_find_close_in_word_sequential() {
        // "()()" = 0b0101 - opens at 0,2; closes at 1,3
        assert_eq!(find_close_in_word(0b0101, 0), Some(1));
        assert_eq!(find_close_in_word(0b0101, 2), Some(3));
    }

    #[test]
    fn test_find_close_in_word_complex() {
        // "(()())" = opens at 0,1,3; closes at 2,4,5
        // bits: pos0=1, pos1=1, pos2=0, pos3=1, pos4=0, pos5=0
        // binary: 0b001011
        assert_eq!(find_close_in_word(0b001011, 0), Some(5)); // outer
        assert_eq!(find_close_in_word(0b001011, 1), Some(2)); // first inner
        assert_eq!(find_close_in_word(0b001011, 3), Some(4)); // second inner
    }

    #[test]
    fn test_find_close_in_word_at_close() {
        // "()" = 0b01 - if we ask at close position 1, returns 1
        assert_eq!(find_close_in_word(0b01, 1), Some(1));
    }

    #[test]
    fn test_find_close_in_word_beyond_word() {
        // "(" padded with opens - no matching close in word
        // Pattern is all 1s (all opens), so excess never goes negative
        assert_eq!(find_close_in_word(u64::MAX, 0), None);

        // "((" padded with opens - neither has matching close in word
        assert_eq!(find_close_in_word(u64::MAX, 0), None);
        assert_eq!(find_close_in_word(u64::MAX, 1), None);
    }

    // ========================================================================
    // Phase 1: Vector-level tests
    // ========================================================================

    #[test]
    fn test_find_close_single_word() {
        // "(()())" = 0b001011
        let words = vec![0b001011u64];
        assert_eq!(find_close(&words, 6, 0), Some(5)); // outer
        assert_eq!(find_close(&words, 6, 1), Some(2)); // first inner
        assert_eq!(find_close(&words, 6, 3), Some(4)); // second inner
    }

    #[test]
    fn test_find_close_multi_word() {
        // 64 opens followed by 64 closes
        let words = vec![u64::MAX, 0u64];
        let len = 128;

        // First open (bit 0) matches close at bit 127
        assert_eq!(find_close(&words, len, 0), Some(127));

        // Second open (bit 1) matches close at bit 126
        assert_eq!(find_close(&words, len, 1), Some(126));

        // Last open (bit 63) matches close at bit 64
        assert_eq!(find_close(&words, len, 63), Some(64));
    }

    #[test]
    fn test_find_close_at_close_returns_none() {
        // "(()())" = 0b001011 - closes at positions 2, 4, 5
        let words = vec![0b001011u64];
        assert_eq!(find_close(&words, 6, 2), None);
        assert_eq!(find_close(&words, 6, 4), None);
        assert_eq!(find_close(&words, 6, 5), None);
    }

    #[test]
    fn test_find_open_single_word() {
        // "(()())" = 0b001011 - opens at 0,1,3; closes at 2,4,5
        let words = vec![0b001011u64];
        assert_eq!(find_open(&words, 6, 5), Some(0)); // close at 5 matches open at 0
        assert_eq!(find_open(&words, 6, 2), Some(1)); // close at 2 matches open at 1
        assert_eq!(find_open(&words, 6, 4), Some(3)); // close at 4 matches open at 3
    }

    #[test]
    fn test_find_open_at_open_returns_none() {
        // "(()())" = 0b001011
        let words = vec![0b001011u64];
        assert_eq!(find_open(&words, 6, 0), None);
        assert_eq!(find_open(&words, 6, 1), None);
        assert_eq!(find_open(&words, 6, 3), None);
    }

    #[test]
    fn test_enclose_simple() {
        // "(())" = 0b0011 - opens at 0,1; closes at 2,3
        let words = vec![0b0011u64];

        // The inner open at position 1 is enclosed by the outer open at position 0
        assert_eq!(enclose(&words, 4, 1), Some(0));

        // The outer open at position 0 has no enclosing parent
        assert_eq!(enclose(&words, 4, 0), None);
    }

    #[test]
    fn test_enclose_nested() {
        // "((()))" = opens at 0,1,2; closes at 3,4,5
        // bits: 1 1 1 0 0 0 = 0b000111
        let words = vec![0b000111u64];

        assert_eq!(enclose(&words, 6, 2), Some(1)); // innermost enclosed by middle
        assert_eq!(enclose(&words, 6, 1), Some(0)); // middle enclosed by outer
        assert_eq!(enclose(&words, 6, 0), None); // outer has no parent
    }

    // ========================================================================
    // Phase 2: BalancedParens struct tests
    // ========================================================================

    #[test]
    fn test_balanced_parens_new() {
        // "(()())" = 0b001011
        let bp = BalancedParens::new(vec![0b001011u64], 6);
        assert_eq!(bp.len(), 6);
        assert!(!bp.is_empty());
    }

    #[test]
    fn test_balanced_parens_empty() {
        let bp = BalancedParens::new(vec![], 0);
        assert_eq!(bp.len(), 0);
        assert!(bp.is_empty());
    }

    #[test]
    fn test_balanced_parens_is_open_close() {
        // "(()())" = 0b001011 - opens at 0,1,3; closes at 2,4,5
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        assert!(bp.is_open(0));
        assert!(bp.is_open(1));
        assert!(bp.is_close(2));
        assert!(bp.is_open(3));
        assert!(bp.is_close(4));
        assert!(bp.is_close(5));
    }

    #[test]
    fn test_balanced_parens_find_close() {
        // "(()())" = 0b001011
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        assert_eq!(bp.find_close(0), Some(5));
        assert_eq!(bp.find_close(1), Some(2));
        assert_eq!(bp.find_close(3), Some(4));

        // At close positions, returns None
        assert_eq!(bp.find_close(2), None);
        assert_eq!(bp.find_close(4), None);
        assert_eq!(bp.find_close(5), None);
    }

    #[test]
    fn test_balanced_parens_find_open() {
        // "(()())" = 0b001011
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        assert_eq!(bp.find_open(5), Some(0));
        assert_eq!(bp.find_open(2), Some(1));
        assert_eq!(bp.find_open(4), Some(3));

        // At open positions, returns None
        assert_eq!(bp.find_open(0), None);
        assert_eq!(bp.find_open(1), None);
        assert_eq!(bp.find_open(3), None);
    }

    #[test]
    fn test_balanced_parens_enclose() {
        // "(()())" = 0b001011
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        assert_eq!(bp.enclose(1), Some(0)); // inner open enclosed by outer
        assert_eq!(bp.enclose(3), Some(0)); // inner open enclosed by outer
        assert_eq!(bp.enclose(0), None); // root has no parent
    }

    #[test]
    fn test_balanced_parens_navigation() {
        // "(()())" = 0b001011
        // Tree structure: root has two children
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        // First child of root (position 0)
        assert_eq!(bp.first_child(0), Some(1));

        // Next sibling of first child
        assert_eq!(bp.next_sibling(1), Some(3));

        // No next sibling for second child
        assert_eq!(bp.next_sibling(3), None);

        // Parent navigation
        assert_eq!(bp.parent(1), Some(0));
        assert_eq!(bp.parent(3), Some(0));
        assert_eq!(bp.parent(0), None);
    }

    #[test]
    fn test_balanced_parens_multi_word() {
        // 64 opens followed by 64 closes
        let words = vec![u64::MAX, 0u64];
        let bp = BalancedParens::new(words, 128);

        // Outermost: open at 0, close at 127
        assert_eq!(bp.find_close(0), Some(127));

        // Second level: open at 1, close at 126
        assert_eq!(bp.find_close(1), Some(126));

        // Last nested: open at 63, close at 64
        assert_eq!(bp.find_close(63), Some(64));
    }

    #[test]
    fn test_balanced_parens_excess() {
        // "(())" = 0b0011 - opens at 0,1; closes at 2,3
        let bp = BalancedParens::new(vec![0b0011u64], 4);

        assert_eq!(bp.excess(0), 1); // after first open
        assert_eq!(bp.excess(1), 2); // after second open
        assert_eq!(bp.excess(2), 1); // after first close
        assert_eq!(bp.excess(3), 0); // after second close (balanced)
    }

    #[test]
    fn test_word_min_excess() {
        // "()" = 0b01 - open at 0, close at 1
        // After bit 0: excess = 1
        // After bit 1: excess = 0 (min reached)
        let (min, total) = word_min_excess(0b01, 2);
        assert_eq!(total, 0);
        assert_eq!(min, 0);

        // ")(" = 0b10 - close at 0, open at 1
        // After bit 0: excess = -1 (min)
        // After bit 1: excess = 0
        let (min, total) = word_min_excess(0b10, 2);
        assert_eq!(min, -1);
        assert_eq!(total, 0);

        // "((" = 0b11 - opens at 0,1
        let (min, total) = word_min_excess(0b11, 2);
        assert_eq!(min, 0); // never goes negative
        assert_eq!(total, 2);

        // "))" = 0b00 - closes at 0,1
        let (min, total) = word_min_excess(0b00, 2);
        assert_eq!(min, -2);
        assert_eq!(total, -2);
    }

    // ========================================================================
    // Roundtrip tests
    // ========================================================================

    #[test]
    fn test_find_close_open_roundtrip() {
        // "(()())" = 0b001011
        let bp = BalancedParens::new(vec![0b001011u64], 6);

        // For each open, find_close then find_open should return to original
        for p in [0, 1, 3] {
            let close = bp.find_close(p).unwrap();
            let open = bp.find_open(close).unwrap();
            assert_eq!(open, p, "roundtrip failed for position {}", p);
        }
    }

    #[test]
    fn test_balanced_parens_matches_linear_scan() {
        // "(()())" = 0b001011
        let words = vec![0b001011u64];
        let len = 6;
        let bp = BalancedParens::new(words.clone(), len);

        for p in 0..len {
            if bp.is_open(p) {
                let bp_result = bp.find_close(p);
                let linear_result = find_close(&words, len, p);
                assert_eq!(bp_result, linear_result, "mismatch at position {}", p);
            }
        }
    }

    // ========================================================================
    // Depth tests (matching Haskell SimpleSpec)
    // ========================================================================

    #[test]
    fn test_depth() {
        // "(()(()())) = 1101101000" in Haskell's bit order
        // Haskell uses 1-indexed positions, we use 0-indexed
        // Pattern: opens at 0,1,3,4,6; closes at 2,5,7,8,9
        // bits from LSB: 1 1 0 1 1 0 1 0 0 0 = 0b0001011011 = 91
        let bp = BalancedParens::new(vec![91u64], 10);

        // Haskell tests (1-indexed): depth 1=1, 2=2, 3=2, 4=2, 5=3, 6=3, 7=3, 8=3, 9=2, 10=1
        // Convert to 0-indexed: depth 0=1, 1=2, 2=2, 3=2, 4=3, 5=3, 6=3, 7=3, 8=2, 9=1
        assert_eq!(bp.depth(0), Some(1)); // depth 1 (root open)
        assert_eq!(bp.depth(1), Some(2)); // depth 2 (first child open)
        assert_eq!(bp.depth(2), Some(1)); // depth 1 (first child close - back to depth 1)
        assert_eq!(bp.depth(3), Some(2)); // depth 2 (second child open)
        assert_eq!(bp.depth(4), Some(3)); // depth 3 (grandchild open)
        assert_eq!(bp.depth(5), Some(2)); // depth 2 (grandchild close)
        assert_eq!(bp.depth(6), Some(3)); // depth 3 (another grandchild open)
        assert_eq!(bp.depth(7), Some(2)); // depth 2 (grandchild close)
        assert_eq!(bp.depth(8), Some(1)); // depth 1 (second child close)
        assert_eq!(bp.depth(9), Some(0)); // depth 0 (root close - balanced)
    }

    #[test]
    fn test_depth_simple() {
        // "(())" = 0b0011
        let bp = BalancedParens::new(vec![0b0011u64], 4);

        assert_eq!(bp.depth(0), Some(1)); // first open
        assert_eq!(bp.depth(1), Some(2)); // nested open
        assert_eq!(bp.depth(2), Some(1)); // first close
        assert_eq!(bp.depth(3), Some(0)); // second close (balanced)
    }

    // ========================================================================
    // Subtree size tests (matching Haskell SimpleSpec)
    // ========================================================================

    #[test]
    fn test_subtree_size() {
        // "(()(()())) = 1101101000" = 91
        // Tree structure:
        //   0 (root)
        //   ├── 1 (first child, leaf)
        //   └── 3 (second child)
        //       ├── 4 (grandchild, leaf)
        //       └── 6 (grandchild, leaf)
        let bp = BalancedParens::new(vec![91u64], 10);

        // Haskell (1-indexed): subtreeSize 1=5, 2=1, 3=0, 4=3, 5=1, 6=0, 7=1, 8=0, 9=0, 10=0
        // In Haskell, subtreeSize seems to count nodes in subtree
        // Our formula: (close - open) / 2 = number of pairs in subtree (excluding self)

        // Position 0 (root open): close at 9, subtree_size = (9-0)/2 = 4 (4 descendant nodes)
        assert_eq!(bp.subtree_size(0), Some(4));

        // Position 1 (first child open): close at 2, subtree_size = (2-1)/2 = 0 (leaf)
        assert_eq!(bp.subtree_size(1), Some(0));

        // Position 2 is a close, should return None
        assert_eq!(bp.subtree_size(2), None);

        // Position 3 (second child open): close at 8, subtree_size = (8-3)/2 = 2 (2 grandchildren)
        assert_eq!(bp.subtree_size(3), Some(2));

        // Position 4 (grandchild open): close at 5, subtree_size = (5-4)/2 = 0 (leaf)
        assert_eq!(bp.subtree_size(4), Some(0));

        // Position 5 is a close
        assert_eq!(bp.subtree_size(5), None);

        // Position 6 (grandchild open): close at 7, subtree_size = (7-6)/2 = 0 (leaf)
        assert_eq!(bp.subtree_size(6), Some(0));
    }

    #[test]
    fn test_subtree_size_simple() {
        // "(())" = 0b0011
        let bp = BalancedParens::new(vec![0b0011u64], 4);

        // Position 0 (outer open): close at 3, subtree_size = (3-0)/2 = 1
        assert_eq!(bp.subtree_size(0), Some(1));

        // Position 1 (inner open): close at 2, subtree_size = (2-1)/2 = 0 (leaf)
        assert_eq!(bp.subtree_size(1), Some(0));

        // Closes return None
        assert_eq!(bp.subtree_size(2), None);
        assert_eq!(bp.subtree_size(3), None);
    }

    #[test]
    fn test_find_close_multi_word_regression() {
        // Regression test from property test failure - tests that accelerated
        // find_close matches linear scan for a multi-word bit vector
        let words: Vec<u64> = vec![
            6148914691236517205,
            17768248446614328661,
            7652237512791978695,
            10679207262162398901,
            11775793721244512726,
            8946029940302164653,
            4836752526886871298,
            6851556326407803519,
            4920914883505671372,
            2108412058774050545,
            11226564708885533661,
            14782248937799897235,
            3628355826111158270,
            16652187963763892183,
            15371706386568957536,
            56015962,
        ];
        let len = 992;
        let p = 153;

        let bp = BalancedParens::new(words.clone(), len);
        let linear_result = find_close(&words, len, p);
        let bp_result = bp.find_close(p);

        // First, make sure we're testing an open position
        assert!(bp.is_open(p), "Position {} should be open", p);

        // The results should match
        assert_eq!(
            bp_result, linear_result,
            "find_close({}) mismatch: accelerated={:?}, linear={:?}",
            p, bp_result, linear_result
        );
    }

    // ========================================================================
    // Edge case tests for block boundaries and special cases
    // ========================================================================

    #[test]
    fn test_single_pair() {
        // Simplest possible: "()" = 0b01
        let bp = BalancedParens::new(vec![0b01], 2);
        assert_eq!(bp.find_close(0), Some(1));
        assert_eq!(bp.find_open(1), Some(0));
        assert_eq!(bp.depth(0), Some(1));
        assert_eq!(bp.depth(1), Some(0));
        assert_eq!(bp.subtree_size(0), Some(0));
    }

    #[test]
    fn test_find_close_at_word_boundary() {
        // Create a sequence where open is at bit 63 (end of word 0)
        // and close is at bit 64 (start of word 1)
        // Pattern: 63 opens, then open at 63, close at 64, then 63 closes
        // This tests the word boundary transition
        let words = vec![u64::MAX, 0u64]; // 64 opens, 64 closes
        let len = 128;
        let bp = BalancedParens::new(words.clone(), len);

        // Open at position 63 should match close at position 64
        assert_eq!(bp.find_close(63), Some(64));
        assert_eq!(find_close(&words, len, 63), Some(64));

        // Open at position 0 should match close at position 127
        assert_eq!(bp.find_close(0), Some(127));
    }

    #[test]
    fn test_find_close_spanning_multiple_words() {
        // 128 opens followed by 128 closes (4 words total)
        let words = vec![u64::MAX, u64::MAX, 0u64, 0u64];
        let len = 256;
        let bp = BalancedParens::new(words.clone(), len);

        // First open matches last close
        assert_eq!(bp.find_close(0), Some(255));
        assert_eq!(find_close(&words, len, 0), Some(255));

        // Open at 127 matches close at 128
        assert_eq!(bp.find_close(127), Some(128));
        assert_eq!(find_close(&words, len, 127), Some(128));
    }

    #[test]
    fn test_find_close_l1_boundary() {
        // L1 boundary is at 32 words = 2048 bits
        // Create: 2048 opens followed by 2048 closes
        let num_opens = 32; // words
        let mut words = vec![u64::MAX; num_opens]; // 32 words of opens
        words.extend(std::iter::repeat_n(0u64, num_opens)); // 32 words of closes
        let len = 64 * 64; // 4096 bits
        let bp = BalancedParens::new(words.clone(), len);

        // Open at position 0 should match close at position 4095
        assert_eq!(bp.find_close(0), Some(4095));
        assert_eq!(find_close(&words, len, 0), Some(4095));

        // Open at position 2047 (last open) matches close at 2048 (first close)
        assert_eq!(bp.find_close(2047), Some(2048));
        assert_eq!(find_close(&words, len, 2047), Some(2048));

        // Test some positions near L1 boundary (2048 bits)
        for p in [2040, 2044, 2046, 2047] {
            let bp_result = bp.find_close(p);
            let linear_result = find_close(&words, len, p);
            assert_eq!(
                bp_result, linear_result,
                "L1 boundary: find_close({}) mismatch",
                p
            );
        }
    }

    #[test]
    fn test_find_close_across_l1_boundary() {
        // Test where open is before L1 boundary and close is after
        // Create pattern: many opens, then balanced pairs around boundary
        // 31 words of opens (1984 bits), then "()" pairs, then closes

        // Simple version: open at bit 2000, close somewhere after L1 boundary
        let mut words = vec![u64::MAX; 64]; // Start with all opens
        let len = 64 * 64;

        // Turn some bits into closes to create a match across L1 boundary
        // Position 2048 is the L1 boundary (word 32, bit 0)
        // Put a close there to match an open before the boundary
        words[32] = 0; // Word 32 = all closes

        let bp = BalancedParens::new(words.clone(), len);

        // Open at 2047 (word 31, bit 63) should match close at 2048 (word 32, bit 0)
        assert_eq!(bp.find_close(2047), Some(2048));
        assert_eq!(find_close(&words, len, 2047), Some(2048));
    }

    #[test]
    fn test_partial_last_word() {
        // Test with len not a multiple of 64
        // "(())" but len = 4 (only 4 valid bits)
        let bp = BalancedParens::new(vec![0b0011u64], 4);
        assert_eq!(bp.find_close(0), Some(3));
        assert_eq!(bp.find_close(1), Some(2));

        // Larger partial word test: 100 bits (1 full word + 36 bits)
        // Pattern: 50 opens followed by 50 closes
        // For 50 opens then 50 closes:
        // Word 0: bits 0-49 = opens (1), bits 50-63 = closes (0)
        // Word 1: bits 0-35 = closes (0), bits 36-63 = invalid
        let word0 = (1u64 << 50) - 1; // bits 0-49 are 1
        let word1 = 0u64; // all closes
        let words = vec![word0, word1];
        let len = 100;
        let bp = BalancedParens::new(words.clone(), len);

        // Open at 0 matches close at 99
        assert_eq!(bp.find_close(0), Some(99));
        assert_eq!(find_close(&words, len, 0), Some(99));

        // Open at 49 matches close at 50
        assert_eq!(bp.find_close(49), Some(50));
        assert_eq!(find_close(&words, len, 49), Some(50));
    }

    #[test]
    fn test_deeply_nested() {
        // Create very deep nesting: (((((...))))) with 32 levels
        // 32 opens followed by 32 closes = 64 bits = 1 word
        let word = (1u64 << 32) - 1; // bits 0-31 are opens
        let bp = BalancedParens::new(vec![word], 64);

        // Open at 0 matches close at 63
        assert_eq!(bp.find_close(0), Some(63));

        // Open at 31 (innermost) matches close at 32
        assert_eq!(bp.find_close(31), Some(32));

        // Test depths
        assert_eq!(bp.depth(0), Some(1));
        assert_eq!(bp.depth(31), Some(32));
        assert_eq!(bp.depth(32), Some(31)); // After first close
    }

    #[test]
    fn test_alternating_pairs() {
        // ()()()()... - sequential pairs
        // Pattern: 0b01 repeated = 0x5555555555555555
        let word = 0x5555555555555555u64;
        let bp = BalancedParens::new(vec![word], 64);

        // Each open at even position matches close at odd position
        for i in 0..32 {
            let open = i * 2;
            let close = i * 2 + 1;
            assert_eq!(bp.find_close(open), Some(close));
            assert_eq!(bp.find_open(close), Some(open));
        }
    }

    #[test]
    fn test_out_of_bounds() {
        let bp = BalancedParens::new(vec![0b01u64], 2);

        // Out of bounds positions should return None
        assert_eq!(bp.find_close(2), None);
        assert_eq!(bp.find_close(100), None);
        assert_eq!(bp.find_open(2), None);
        assert_eq!(bp.find_open(100), None);
        assert_eq!(bp.depth(2), None);
        assert_eq!(bp.depth(100), None);
        assert_eq!(bp.subtree_size(2), None);
    }

    #[test]
    fn test_empty() {
        let bp = BalancedParens::new(vec![], 0);
        assert!(bp.is_empty());
        assert_eq!(bp.len(), 0);
        assert_eq!(bp.find_close(0), None);
        assert_eq!(bp.find_open(0), None);
    }

    #[test]
    fn test_unbalanced_sequence() {
        // "((" = 0b11 - two opens, no closes
        // This is an invalid BP sequence, but the functions should handle it gracefully
        let words = vec![0b11u64];
        let bp = BalancedParens::new(words.clone(), 2);

        // find_close should return None for unmatched opens
        assert_eq!(bp.find_close(0), None);
        assert_eq!(bp.find_close(1), None);
        assert_eq!(find_close(&words, 2, 0), None);
    }

    #[test]
    fn test_all_closes() {
        // "))" = 0b00 - two closes, no opens
        let words = vec![0u64];
        let bp = BalancedParens::new(words.clone(), 2);

        // find_close on close positions should return None
        assert_eq!(bp.find_close(0), None);
        assert_eq!(bp.find_close(1), None);
    }

    #[test]
    fn test_find_close_near_end() {
        // Test find_close where match is near the end of the bit vector
        // "(())" but the close at position 3 is near len
        let bp = BalancedParens::new(vec![0b0011u64], 4);
        assert_eq!(bp.find_close(0), Some(3));

        // Larger test: open near the beginning, close at the very end
        let words = vec![u64::MAX, 0u64];
        let len = 128;
        let bp = BalancedParens::new(words.clone(), len);
        assert_eq!(bp.find_close(0), Some(127));
    }

    #[test]
    fn test_index_structure_correctness() {
        // Verify the L0/L1/L2 indices are computed correctly
        // for a specific known pattern
        let words = vec![0b001011u64]; // "(()())" - balanced
        let bp = BalancedParens::new(words, 6);

        // L0 min_excess for this word should reflect the minimum reached
        // Scanning: 1, 2, 1, 2, 1, 0 - min is 0
        assert_eq!(bp.l0_min_excess[0], 0);

        // L0 cum_excess should be 0 (balanced)
        assert_eq!(bp.l0_cum_excess[0], 0);
    }

    #[test]
    fn test_word_min_excess_full_word() {
        // Test word_min_excess with full 64-bit patterns

        // All opens: excess goes 1,2,3,...,64, min=0
        let (min, total) = word_min_excess(u64::MAX, 64);
        assert_eq!(min, 0);
        assert_eq!(total, 64);

        // All closes: excess goes -1,-2,...,-64, min=-64
        let (min, total) = word_min_excess(0, 64);
        assert_eq!(min, -64);
        assert_eq!(total, -64);

        // Alternating ()()()...: min=0, total=0
        let (min, total) = word_min_excess(0x5555555555555555, 64);
        assert_eq!(min, 0);
        assert_eq!(total, 0);

        // Alternating )()()(... (starting with close): min=-1, total=0
        let (min, total) = word_min_excess(0xAAAAAAAAAAAAAAAAu64, 64);
        assert_eq!(min, -1);
        assert_eq!(total, 0);
    }

    #[test]
    fn test_l1_index_construction() {
        // Test that L1 indices are built correctly
        // Create exactly 32 words to have one full L1 block
        let words = vec![0x5555555555555555u64; 32]; // ()()()... - balanced
        let len = 32 * 64;
        let bp = BalancedParens::new(words, len);

        // Should have exactly 1 L1 block
        assert_eq!(bp.l1_min_excess.len(), 1);
        assert_eq!(bp.l1_cum_excess.len(), 1);

        // L1 should also show balanced (cum_excess = 0)
        assert_eq!(bp.l1_cum_excess[0], 0);
    }

    #[test]
    fn test_multiple_l1_blocks() {
        // Create 64 words = 2 L1 blocks
        // First L1 block: 32 words of opens (positive excess)
        let mut words = vec![u64::MAX; 32];
        // Second L1 block: 32 words of closes (negative excess)
        words.extend(std::iter::repeat_n(0u64, 32));
        let len = 64 * 64;
        let bp = BalancedParens::new(words.clone(), len);

        // Should have 2 L1 blocks
        assert_eq!(bp.l1_min_excess.len(), 2);

        // First L1 block: all opens, min_excess = 0, cum_excess = 32*64 = 2048
        assert_eq!(bp.l1_min_excess[0], 0);
        assert_eq!(bp.l1_cum_excess[0], 2048);

        // Second L1 block: all closes, min_excess = -2048, cum_excess = -2048
        assert_eq!(bp.l1_min_excess[1], -2048);
        assert_eq!(bp.l1_cum_excess[1], -2048);

        // Test find_close across L1 boundary
        assert_eq!(bp.find_close(2047), Some(2048));
        assert_eq!(find_close(&words, len, 2047), Some(2048));
    }

    #[test]
    fn test_find_close_immediate_match() {
        // Test cases where match is immediately after the open
        // "()" repeated
        let word = 0x5555555555555555u64;
        let bp = BalancedParens::new(vec![word], 64);

        for i in 0..32 {
            assert_eq!(bp.find_close(i * 2), Some(i * 2 + 1));
        }
    }

    #[test]
    fn test_sibling_chain() {
        // "()()()..." - chain of siblings
        let word = 0x5555555555555555u64;
        let bp = BalancedParens::new(vec![word], 64);

        // Each pair is a sibling of the next
        for i in 0..31 {
            let pos = i * 2;
            assert_eq!(bp.next_sibling(pos), Some(pos + 2));
        }
        // Last sibling has no next
        assert_eq!(bp.next_sibling(62), None);
    }

    #[test]
    fn test_no_first_child_for_leaf() {
        // "()" - leaf node has no first child
        let bp = BalancedParens::new(vec![0b01u64], 2);
        assert_eq!(bp.first_child(0), None);
    }

    #[test]
    fn test_first_child_exists() {
        // "(())" - root has first child
        let bp = BalancedParens::new(vec![0b0011u64], 4);
        assert_eq!(bp.first_child(0), Some(1));
        assert_eq!(bp.first_child(1), None); // inner leaf
    }
}
