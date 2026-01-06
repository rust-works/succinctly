//! BitWriter for incremental bit vector construction.

#[cfg(not(test))]
use alloc::vec::Vec;

/// A writer for incrementally building a bit vector.
///
/// Bits are written from LSB to MSB within each word.
pub struct BitWriter {
    words: Vec<u64>,
    current_word: u64,
    bit_position: u32, // Position within current_word (0-63)
}

impl BitWriter {
    /// Create a new BitWriter with specified initial capacity in words.
    pub fn with_capacity(word_capacity: usize) -> Self {
        Self {
            words: Vec::with_capacity(word_capacity),
            current_word: 0,
            bit_position: 0,
        }
    }

    /// Create a new BitWriter with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(64)
    }

    /// Write a single bit (0 or 1).
    #[inline]
    pub fn write_bit(&mut self, bit: bool) {
        if bit {
            self.current_word |= 1u64 << self.bit_position;
        }
        self.bit_position += 1;
        if self.bit_position == 64 {
            self.words.push(self.current_word);
            self.current_word = 0;
            self.bit_position = 0;
        }
    }

    /// Write a 0 bit.
    #[inline]
    pub fn write_0(&mut self) {
        self.write_bit(false);
    }

    /// Write a 1 bit.
    #[inline]
    pub fn write_1(&mut self) {
        self.write_bit(true);
    }

    /// Write multiple bits from a u64 value efficiently.
    ///
    /// Writes the lowest `count` bits from `bits` to the output.
    /// This is faster than calling `write_bit()` in a loop because it
    /// handles word boundaries with a single check.
    ///
    /// # Arguments
    /// * `bits` - The bits to write (only lowest `count` bits are used)
    /// * `count` - Number of bits to write (must be <= 64)
    ///
    /// # Panics
    /// Panics if `count > 64`.
    #[inline]
    pub fn write_bits(&mut self, bits: u64, count: usize) {
        debug_assert!(count <= 64, "count must be <= 64");
        if count == 0 {
            return;
        }

        let pos = self.bit_position as usize;
        let space_in_word = 64 - pos;

        if count <= space_in_word {
            // All bits fit in current word
            // Mask to only use the lowest `count` bits
            let mask = if count == 64 { u64::MAX } else { (1u64 << count) - 1 };
            self.current_word |= (bits & mask) << pos;
            self.bit_position += count as u32;

            if self.bit_position == 64 {
                self.words.push(self.current_word);
                self.current_word = 0;
                self.bit_position = 0;
            }
        } else {
            // Bits span two words
            // First, fill the rest of current word
            let mask = if count == 64 { u64::MAX } else { (1u64 << count) - 1 };
            let masked_bits = bits & mask;
            self.current_word |= masked_bits << pos;
            self.words.push(self.current_word);

            // Then write remaining bits to new word
            let remaining = count - space_in_word;
            self.current_word = masked_bits >> space_in_word;
            self.bit_position = remaining as u32;
        }
    }

    /// Write multiple zero bits efficiently.
    ///
    /// This is faster than calling `write_0()` in a loop because it can
    /// skip full words and only update the bit position.
    #[inline]
    pub fn write_zeros(&mut self, count: usize) {
        if count == 0 {
            return;
        }

        let mut remaining = count;

        // First, fill up the current word if we're mid-word
        if self.bit_position > 0 {
            let space_in_word = 64 - self.bit_position as usize;
            if remaining < space_in_word {
                // All zeros fit in current word - just advance position
                self.bit_position += remaining as u32;
                return;
            }
            // Fill rest of current word and flush
            self.words.push(self.current_word);
            self.current_word = 0;
            remaining -= space_in_word;
            self.bit_position = 0;
        }

        // Push full words of zeros
        let full_words = remaining / 64;
        for _ in 0..full_words {
            self.words.push(0);
        }

        // Handle remaining bits in new word
        let leftover = remaining % 64;
        self.bit_position = leftover as u32;
        // current_word is already 0, so no need to clear
    }

    /// Finalize and return the completed bit vector.
    ///
    /// Any partial word is included with remaining bits set to 0.
    pub fn finish(mut self) -> Vec<u64> {
        if self.bit_position > 0 {
            self.words.push(self.current_word);
        }
        self.words
    }

    /// Get the total number of bits written.
    pub fn len(&self) -> usize {
        self.words.len() * 64 + self.bit_position as usize
    }

    /// Check if no bits have been written.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.bit_position == 0
    }
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_single_bits() {
        let mut writer = BitWriter::new();
        writer.write_1();
        writer.write_0();
        writer.write_1();
        writer.write_1();
        let words = writer.finish();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], 0b1101); // LSB first
    }

    #[test]
    fn test_write_64_bits() {
        let mut writer = BitWriter::new();
        for _ in 0..64 {
            writer.write_1();
        }
        let words = writer.finish();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], u64::MAX);
    }

    #[test]
    fn test_write_65_bits() {
        let mut writer = BitWriter::new();
        for _ in 0..65 {
            writer.write_1();
        }
        let words = writer.finish();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], u64::MAX);
        assert_eq!(words[1], 1);
    }

    #[test]
    fn test_alternating_bits() {
        let mut writer = BitWriter::new();
        for i in 0..8 {
            writer.write_bit(i % 2 == 0);
        }
        let words = writer.finish();
        assert_eq!(words[0] & 0xFF, 0b01010101); // LSB first: positions 0,2,4,6 are 1
    }

    #[test]
    fn test_empty() {
        let writer = BitWriter::new();
        assert!(writer.is_empty());
        let words = writer.finish();
        assert!(words.is_empty());
    }

    #[test]
    fn test_len() {
        let mut writer = BitWriter::new();
        assert_eq!(writer.len(), 0);
        writer.write_1();
        assert_eq!(writer.len(), 1);
        for _ in 0..63 {
            writer.write_0();
        }
        assert_eq!(writer.len(), 64);
        writer.write_1();
        assert_eq!(writer.len(), 65);
    }

    #[test]
    fn test_write_zeros_small() {
        let mut writer = BitWriter::new();
        writer.write_1();
        writer.write_zeros(5);
        writer.write_1();
        let words = writer.finish();
        assert_eq!(words.len(), 1);
        // Position 0: 1, positions 1-5: 0, position 6: 1
        assert_eq!(words[0], 0b1000001);
    }

    #[test]
    fn test_write_zeros_cross_word() {
        let mut writer = BitWriter::new();
        writer.write_1();
        writer.write_zeros(64); // Should push one word and leave position at 1
        writer.write_1();
        let words = writer.finish();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], 1); // Just the first 1 at bit 0
        assert_eq!(words[1], 2); // 1 at bit 1 (after 64 zeros, we're at position 1)
    }

    #[test]
    fn test_write_zeros_multiple_words() {
        let mut writer = BitWriter::new();
        writer.write_zeros(128);
        writer.write_1();
        let words = writer.finish();
        // 128 zeros + 1 bit = 129 bits, needs ceil(129/64) = 3 words
        assert_eq!(words.len(), 3);
        assert_eq!(words[0], 0);
        assert_eq!(words[1], 0);
        // Bit 128 is at word 2, bit position 0 (128 % 64 = 0)
        assert_eq!(words[2], 1);
    }

    #[test]
    fn test_write_zeros_matches_loop() {
        // Verify write_zeros produces same result as loop
        let mut writer1 = BitWriter::new();
        writer1.write_1();
        for _ in 0..100 {
            writer1.write_0();
        }
        writer1.write_1();

        let mut writer2 = BitWriter::new();
        writer2.write_1();
        writer2.write_zeros(100);
        writer2.write_1();

        assert_eq!(writer1.finish(), writer2.finish());
    }

    #[test]
    fn test_write_bits_small() {
        let mut writer = BitWriter::new();
        // Write 0b1101 (4 bits)
        writer.write_bits(0b1101, 4);
        let words = writer.finish();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], 0b1101);
    }

    #[test]
    fn test_write_bits_with_offset() {
        let mut writer = BitWriter::new();
        writer.write_1(); // bit 0
        writer.write_bits(0b110, 3); // bits 1-3: should be 0b1100 at positions 1,2,3
        let words = writer.finish();
        assert_eq!(words.len(), 1);
        // Position 0: 1, positions 1-3: 110 (LSB first) = 0b1101
        assert_eq!(words[0], 0b1101);
    }

    #[test]
    fn test_write_bits_cross_word() {
        let mut writer = BitWriter::new();
        // Write 60 zeros to get near word boundary
        writer.write_zeros(60);
        // Now write 8 bits that span the boundary
        writer.write_bits(0b11110000, 8);
        let words = writer.finish();
        assert_eq!(words.len(), 2);
        // First word: bits 60-63 should be 0b0000 (low 4 bits of 0b11110000)
        assert_eq!(words[0] >> 60, 0b0000);
        // Second word: bits 0-3 should be 0b1111 (high 4 bits of 0b11110000)
        assert_eq!(words[1] & 0xF, 0b1111);
    }

    #[test]
    fn test_write_bits_full_word() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xDEADBEEF_CAFEBABE, 64);
        let words = writer.finish();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0], 0xDEADBEEF_CAFEBABE);
    }

    #[test]
    fn test_write_bits_matches_loop() {
        // Verify write_bits produces same result as individual writes
        let mut writer1 = BitWriter::new();
        writer1.write_1();
        writer1.write_0();
        writer1.write_1();
        writer1.write_1();
        writer1.write_0();

        let mut writer2 = BitWriter::new();
        writer2.write_bits(0b01101, 5); // LSB first: 1,0,1,1,0

        assert_eq!(writer1.finish(), writer2.finish());
    }

    #[test]
    fn test_write_bits_exact_word_boundary() {
        let mut writer = BitWriter::new();
        // Write exactly 64 bits
        writer.write_bits(u64::MAX, 64);
        // Then write 4 more
        writer.write_bits(0b1010, 4);
        let words = writer.finish();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0], u64::MAX);
        assert_eq!(words[1], 0b1010);
    }
}
