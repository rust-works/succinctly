//! Precomputed lookup tables for select operations.
//!
//! The `SELECT_IN_BYTE_TABLE` provides O(1) lookup for the position of the k-th
//! set bit within a single byte.

/// Lookup table for select-in-byte operation.
///
/// For a byte value `b` and rank `k`, `SELECT_IN_BYTE_TABLE[b * 8 + k]` gives
/// the position (0-7) of the k-th set bit in `b`, or 8 if there are fewer than
/// k+1 set bits.
///
/// Table size: 256 bytes × 8 positions = 2048 bytes
pub static SELECT_IN_BYTE_TABLE: [u8; 2048] = {
    let mut table = [8u8; 2048];
    let mut byte = 0u16;
    while byte < 256 {
        let mut pos = 0u8;
        let mut count = 0u8;
        while pos < 8 {
            if (byte >> pos) & 1 == 1 {
                table[(byte as usize) * 8 + count as usize] = pos;
                count += 1;
            }
            pos += 1;
        }
        byte += 1;
    }
    table
};

/// Select the k-th set bit (0-indexed) in a byte.
///
/// Returns the bit position (0-7), or 8 if there are fewer than k+1 set bits.
#[inline]
pub fn select_in_byte(byte: u8, k: u32) -> u32 {
    if k >= 8 {
        return 8;
    }
    SELECT_IN_BYTE_TABLE[(byte as usize) * 8 + k as usize] as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_select_in_byte_single_bit() {
        // Single bit set at each position
        for pos in 0..8 {
            let byte = 1u8 << pos;
            assert_eq!(select_in_byte(byte, 0), pos, "byte={:08b}", byte);
            assert_eq!(select_in_byte(byte, 1), 8, "byte={:08b}, k=1", byte);
        }
    }

    #[test]
    fn test_select_in_byte_all_ones() {
        let byte = 0xFF;
        for k in 0..8 {
            assert_eq!(select_in_byte(byte, k), k);
        }
        assert_eq!(select_in_byte(byte, 8), 8); // Out of range
    }

    #[test]
    fn test_select_in_byte_zero() {
        assert_eq!(select_in_byte(0, 0), 8);
    }

    #[test]
    fn test_select_in_byte_alternating() {
        // 0b10101010 = bits at positions 1, 3, 5, 7
        let byte = 0b1010_1010;
        assert_eq!(select_in_byte(byte, 0), 1);
        assert_eq!(select_in_byte(byte, 1), 3);
        assert_eq!(select_in_byte(byte, 2), 5);
        assert_eq!(select_in_byte(byte, 3), 7);
        assert_eq!(select_in_byte(byte, 4), 8);
    }

    #[test]
    fn test_table_correctness() {
        // Verify table correctness for all byte values
        for byte in 0u8..=255 {
            let pop = byte.count_ones();
            for k in 0..pop {
                let pos = SELECT_IN_BYTE_TABLE[(byte as usize) * 8 + k as usize];
                // Verify the k-th bit is actually set at position pos
                assert!(
                    (byte >> pos) & 1 == 1,
                    "byte={:08b}, k={}, pos={}",
                    byte,
                    k,
                    pos
                );
            }
            // Verify positions beyond popcount return 8
            for k in pop..8 {
                assert_eq!(
                    SELECT_IN_BYTE_TABLE[(byte as usize) * 8 + k as usize],
                    8,
                    "byte={:08b}, k={}",
                    byte,
                    k
                );
            }
        }
    }
}
