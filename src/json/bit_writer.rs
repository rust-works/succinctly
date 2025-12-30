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
}
