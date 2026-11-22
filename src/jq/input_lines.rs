//! Tables of input lines that keep 8 bytes per entry past 2^32 lines (#7472).
//!
//! jq counts input lines in a `size_t` and never wraps (`util.c`'s `current_line`), so a value
//! on line 4,294,967,302 is on that line, not on line 6. The tables that hold one
//! `(source, line)` per input value (the CLI's per-value locations, a `--seq` stream's record
//! places, the `input` queue) keep 8 bytes per entry on purpose (#7412), so a line is stored as
//! its low 32 bits beside a [`LineHighs`] run table of the high 32 bits. Lines never decrease
//! within a source, so the high word changes at most once per 2^32 lines of a source (and once
//! more where a source starts): a table whose lines all stay below 2^32 holds no run at all.

use alloc::vec::Vec;

/// The high 32 bits of the lines in a table that stores each line's low 32 bits (#7472).
///
/// A run is the index of the first entry whose high word differs from the entry before it
/// (the entry before the first one counts as high word 0), with that high word. Entries are
/// recorded in index order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineHighs {
    runs: Vec<(usize, u32)>,
}

impl LineHighs {
    /// No runs: every line below 2^32.
    pub const fn new() -> Self {
        Self { runs: Vec::new() }
    }

    /// Records that entry `index`, the next after every entry recorded so far, holds `line`, and
    /// returns the low word to store for it.
    pub fn split(&mut self, index: usize, line: u64) -> u32 {
        debug_assert!(self.runs.last().map_or(true, |&(start, _)| start < index));
        let high = (line >> 32) as u32;
        if high != self.runs.last().map_or(0, |&(_, high)| high) {
            self.runs.push((index, high));
        }
        line as u32
    }

    /// [`split`](Self::split) for entry `index` when it is the last entry and already recorded:
    /// its line becomes `line`.
    pub fn resplit_last(&mut self, index: usize, line: u64) -> u32 {
        if self.runs.last().is_some_and(|&(start, _)| start == index) {
            self.runs.pop();
        }
        debug_assert!(self.runs.last().map_or(true, |&(start, _)| start < index));
        let high = (line >> 32) as u32;
        if high != self.runs.last().map_or(0, |&(_, high)| high) {
            self.runs.push((index, high));
        }
        line as u32
    }

    /// The high word of entry `index`.
    pub fn high_at(&self, index: usize) -> u32 {
        // A table below 2^32 lines has no run, and this is one length check.
        if self.runs.is_empty() {
            return 0;
        }
        let after = self.runs.partition_point(|&(start, _)| start <= index);
        after.checked_sub(1).map_or(0, |run| self.runs[run].1)
    }

    /// The full line of entry `index`, whose stored low word is `low`.
    pub fn join(&self, index: usize, low: u32) -> u64 {
        (u64::from(self.high_at(index)) << 32) | u64::from(low)
    }

    /// How many runs are recorded: 0 while every line is below 2^32.
    pub fn runs(&self) -> usize {
        self.runs.len()
    }
}

/// `(source, line)` per input value, at 8 bytes per entry and a [`LineHighs`] run per change of
/// a line's high word (#7472).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineTable {
    entries: Vec<(u32, u32)>,
    highs: LineHighs,
}

impl LineTable {
    /// An empty table.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            highs: LineHighs::new(),
        }
    }

    /// An empty table with room for `capacity` entries.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            highs: LineHighs::new(),
        }
    }

    /// Appends `(source, line)`.
    pub fn push(&mut self, source: u32, line: u64) {
        let low = self.highs.split(self.entries.len(), line);
        self.entries.push((source, low));
    }

    /// Replaces the last entry with `(source, line)`; nothing happens when the table is empty.
    pub fn set_last(&mut self, source: u32, line: u64) {
        let Some(index) = self.entries.len().checked_sub(1) else {
            return;
        };
        let low = self.highs.resplit_last(index, line);
        self.entries[index] = (source, low);
    }

    /// The entry at `index`.
    pub fn get(&self, index: usize) -> Option<(u32, u64)> {
        let &(source, low) = self.entries.get(index)?;
        Some((source, self.highs.join(index, low)))
    }

    /// The last entry.
    pub fn last(&self) -> Option<(u32, u64)> {
        self.get(self.entries.len().checked_sub(1)?)
    }

    /// How many entries the table holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table holds no entry.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries, in order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, u64)> + '_ {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, &(source, low))| (source, self.highs.join(index, low)))
    }

    /// The entries, in order, as a vector.
    pub fn to_vec(&self) -> Vec<(u32, u64)> {
        self.iter().collect()
    }

    /// The stored `(source, low word)` entries, which is all the table holds below 2^32 lines.
    pub fn entries(&self) -> &[(u32, u32)] {
        &self.entries
    }

    /// The run table of high words.
    pub fn highs(&self) -> &LineHighs {
        &self.highs
    }

    /// How many entries the table has room for without growing.
    pub fn capacity(&self) -> usize {
        self.entries.capacity()
    }
}

impl FromIterator<(u32, u64)> for LineTable {
    fn from_iter<I: IntoIterator<Item = (u32, u64)>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut table = Self::with_capacity(iter.size_hint().0);
        for (source, line) in iter {
            table.push(source, line);
        }
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_32: u64 = 1 << 32;

    #[test]
    fn a_table_below_two_to_the_32_keeps_eight_bytes_an_entry_and_no_run_7472() {
        assert_eq!(core::mem::size_of::<(u32, u32)>(), 8);
        let mut table = LineTable::with_capacity(4);
        for line in [1, 2, TWO_32 - 2, TWO_32 - 1] {
            table.push(0, line);
        }
        assert_eq!(table.highs().runs(), 0);
        assert_eq!(table.capacity(), 4);
        assert_eq!(
            table.entries(),
            &[(0, 1), (0, 2), (0, u32::MAX - 1), (0, u32::MAX)]
        );
        assert_eq!(table.get(3), Some((0, TWO_32 - 1)));
    }

    #[test]
    fn lines_past_two_to_the_32_read_back_whole_7472() {
        // Two sources: the first runs past 2^32 and past 2^33, the second starts again at 1.
        let lines = [
            (0, TWO_32 - 1),
            (0, TWO_32),
            (0, TWO_32 + 5),
            (0, 2 * TWO_32 + 1),
            (1, 1),
            (1, 3),
            (1, TWO_32 + 6),
            (1, u64::MAX),
        ];
        let table: LineTable = lines.iter().copied().collect();
        assert_eq!(table.len(), lines.len());
        assert_eq!(table.iter().collect::<Vec<_>>(), lines);
        for (index, &entry) in lines.iter().enumerate() {
            assert_eq!(table.get(index), Some(entry));
        }
        assert_eq!(table.get(lines.len()), None);
        assert_eq!(table.last(), Some((1, u64::MAX)));
        // One run per change of the high word: 1, 2, 0, 1, u32::MAX.
        assert_eq!(table.highs().runs(), 5);
    }

    #[test]
    fn the_last_entry_can_be_moved_across_a_high_word_7472() {
        let mut table = LineTable::new();
        table.set_last(0, 9);
        assert!(table.is_empty());
        table.push(0, TWO_32 - 1);
        table.push(0, TWO_32 + 1);
        assert_eq!(table.highs().runs(), 1);
        table.set_last(2, 0);
        assert_eq!(table.highs().runs(), 0);
        assert_eq!(table.last(), Some((2, 0)));
        table.set_last(2, 3 * TWO_32);
        assert_eq!(table.last(), Some((2, 3 * TWO_32)));
        assert_eq!(table.get(0), Some((0, TWO_32 - 1)));
        assert_eq!(table.highs().runs(), 1);
    }
}
