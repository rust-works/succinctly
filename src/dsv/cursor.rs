//! Cursor and navigation for DSV data.

use super::index::DsvIndex;

/// Lightweight cursor for navigating DSV data.
///
/// The cursor maintains a position in the text and provides methods to
/// navigate to fields and rows using the pre-built index.
#[derive(Clone, Copy, Debug)]
pub struct DsvCursor<'a> {
    text: &'a [u8],
    index: &'a DsvIndex,
    /// Current position in text (byte offset)
    position: usize,
}

impl<'a> DsvCursor<'a> {
    /// Create a new cursor at the start of the text.
    pub fn new(text: &'a [u8], index: &'a DsvIndex) -> Self {
        Self {
            text,
            index,
            position: 0,
        }
    }

    /// Current byte position in text.
    #[inline]
    pub fn position(&self) -> usize {
        self.position
    }

    /// Are we at end of data?
    #[inline]
    pub fn at_end(&self) -> bool {
        self.position >= self.text.len()
    }

    /// Move to the next field (after the next delimiter or newline).
    ///
    /// Returns true if successful, false if at end of data.
    pub fn next_field(&mut self) -> bool {
        if self.at_end() {
            return false;
        }

        // Find rank of current position in markers
        let current_rank = self.index.markers_rank1(self.position);

        // Find position of next marker
        if let Some(next_pos) = self.index.markers_select1(current_rank) {
            if next_pos < self.text.len() {
                self.position = next_pos + 1; // Move past the delimiter
                return !self.at_end();
            }
        }

        // No more fields, move to end
        self.position = self.text.len();
        false
    }

    /// Move to the start of the next row.
    ///
    /// Returns true if successful, false if at end of data.
    pub fn next_row(&mut self) -> bool {
        if self.at_end() {
            return false;
        }

        // Find rank of current position in newlines
        let current_rank = self.index.newlines_rank1(self.position);

        // Find position of next newline
        if let Some(next_pos) = self.index.newlines_select1(current_rank) {
            if next_pos < self.text.len() {
                self.position = next_pos + 1; // Move past the newline
                return !self.at_end();
            }
        }

        // No more rows
        self.position = self.text.len();
        false
    }

    /// Move to the start of row `n` (0-indexed).
    ///
    /// Returns true if successful, false if row doesn't exist.
    pub fn goto_row(&mut self, n: usize) -> bool {
        if n == 0 {
            self.position = 0;
            return !self.text.is_empty();
        }

        // Row n starts after newline (n-1), which is the (n-1)th newline (0-indexed)
        if let Some(newline_pos) = self.index.newlines_select1(n - 1) {
            let new_pos = newline_pos + 1;
            if new_pos <= self.text.len() {
                self.position = new_pos;
                return !self.at_end();
            }
        }

        false
    }

    /// Get field at current position (up to next delimiter or newline).
    pub fn current_field(&self) -> &'a [u8] {
        if self.at_end() {
            return &[];
        }

        let start = self.position;

        // Find next marker (delimiter or newline)
        let current_rank = self.index.markers_rank1(start);
        let end = self
            .index
            .markers_select1(current_rank)
            .unwrap_or(self.text.len());

        &self.text[start..end]
    }

    /// Get current field as a string slice.
    #[cfg(feature = "std")]
    pub fn current_field_str(&self) -> Result<&'a str, core::str::Utf8Error> {
        core::str::from_utf8(self.current_field())
    }

    /// Check if the current byte is a newline marker.
    fn at_newline(&self) -> bool {
        if self.position == 0 || self.position > self.text.len() {
            return false;
        }
        // Check if the previous position was a newline
        let prev_pos = self.position - 1;
        let rank_before = self.index.newlines_rank1(prev_pos);
        let rank_at = self.index.newlines_rank1(self.position);
        rank_at > rank_before
    }
}

/// A single row in DSV data.
#[derive(Clone, Copy, Debug)]
pub struct DsvRow<'a> {
    cursor: DsvCursor<'a>,
    row_start: usize,
}

impl<'a> DsvRow<'a> {
    /// Create a row from a cursor positioned at the row start.
    pub(crate) fn from_cursor(cursor: DsvCursor<'a>) -> Self {
        Self {
            row_start: cursor.position,
            cursor,
        }
    }

    /// Iterate over fields in this row.
    pub fn fields(&self) -> DsvFields<'a> {
        DsvFields {
            cursor: DsvCursor {
                position: self.row_start,
                ..self.cursor
            },
            row_start: self.row_start,
            started: false,
            finished: false,
        }
    }

    /// Get field at column index (0-indexed).
    pub fn get(&self, column: usize) -> Option<&'a [u8]> {
        let mut cursor = DsvCursor {
            position: self.row_start,
            ..self.cursor
        };

        for _ in 0..column {
            // Check if we hit a newline before reaching the column
            let field = cursor.current_field();
            if field.is_empty() && cursor.at_end() {
                return None;
            }

            if !cursor.next_field() {
                return None;
            }

            // Check if we moved to the next row
            if cursor.at_newline() || cursor.at_end() {
                return None;
            }
        }

        let field = cursor.current_field();
        if field.is_empty() && cursor.at_end() {
            None
        } else {
            Some(field)
        }
    }
}

/// Iterator over rows in DSV data.
pub struct DsvRows<'a> {
    cursor: DsvCursor<'a>,
    started: bool,
}

impl<'a> DsvRows<'a> {
    /// Create a new row iterator.
    pub fn new(text: &'a [u8], index: &'a DsvIndex) -> Self {
        Self {
            cursor: DsvCursor::new(text, index),
            started: false,
        }
    }
}

impl<'a> Iterator for DsvRows<'a> {
    type Item = DsvRow<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            self.started = true;
            if self.cursor.at_end() {
                return None;
            }
            return Some(DsvRow::from_cursor(self.cursor));
        }

        if self.cursor.next_row() {
            Some(DsvRow::from_cursor(self.cursor))
        } else {
            None
        }
    }
}

/// Iterator over fields in a row.
pub struct DsvFields<'a> {
    cursor: DsvCursor<'a>,
    #[allow(dead_code)]
    row_start: usize,
    started: bool,
    finished: bool,
}

impl<'a> Iterator for DsvFields<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        if !self.started {
            self.started = true;
            let field = self.cursor.current_field();

            // Check if this is the last field in the row
            let field_end = self.cursor.position + field.len();
            if field_end >= self.cursor.text.len() {
                self.finished = true;
            } else {
                // Check if the next character is a newline
                let next_rank = self.cursor.index.newlines_rank1(field_end + 1);
                let curr_rank = self.cursor.index.newlines_rank1(field_end);
                if next_rank > curr_rank {
                    self.finished = true;
                }
            }

            return Some(field);
        }

        // Move to next field
        if !self.cursor.next_field() {
            self.finished = true;
            return None;
        }

        // Check if we've moved past the current row
        if self.cursor.at_newline() {
            self.finished = true;
            return None;
        }

        let field = self.cursor.current_field();

        // Check if this is the last field in the row
        let field_end = self.cursor.position + field.len();
        if field_end >= self.cursor.text.len() {
            self.finished = true;
        } else {
            // Check if the next character is a newline
            let next_rank = self.cursor.index.newlines_rank1(field_end + 1);
            let curr_rank = self.cursor.index.newlines_rank1(field_end);
            if next_rank > curr_rank {
                self.finished = true;
            }
        }

        Some(field)
    }
}

/// Strip surrounding quotes from a field if present.
#[allow(dead_code)]
pub fn strip_quotes(field: &[u8]) -> &[u8] {
    if field.len() >= 2 && field[0] == b'"' && field[field.len() - 1] == b'"' {
        &field[1..field.len() - 1]
    } else {
        field
    }
}

#[cfg(test)]
mod tests {
    use super::super::parser::build_index;
    use super::super::DsvConfig;
    use super::*;

    #[test]
    fn test_cursor_navigation() {
        let csv = b"a,b,c\n1,2,3\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        let mut cursor = DsvCursor::new(csv, &index);

        assert_eq!(cursor.position(), 0);
        assert_eq!(cursor.current_field(), b"a");

        assert!(cursor.next_field());
        assert_eq!(cursor.current_field(), b"b");

        assert!(cursor.next_field());
        assert_eq!(cursor.current_field(), b"c");

        assert!(cursor.next_field()); // Move to next row
        assert_eq!(cursor.current_field(), b"1");
    }

    #[test]
    fn test_goto_row() {
        let csv = b"a,b\nc,d\ne,f\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        let mut cursor = DsvCursor::new(csv, &index);

        assert!(cursor.goto_row(0));
        assert_eq!(cursor.current_field(), b"a");

        assert!(cursor.goto_row(1));
        assert_eq!(cursor.current_field(), b"c");

        assert!(cursor.goto_row(2));
        assert_eq!(cursor.current_field(), b"e");

        assert!(!cursor.goto_row(3)); // Row 3 doesn't exist
    }

    #[test]
    fn test_row_fields() {
        let csv = b"a,b,c\n1,2,3\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        let cursor = DsvCursor::new(csv, &index);
        let row = DsvRow::from_cursor(cursor);

        let fields: Vec<_> = row.fields().collect();
        assert_eq!(
            fields,
            vec![b"a".as_slice(), b"b".as_slice(), b"c".as_slice()]
        );
    }

    #[test]
    fn test_row_get() {
        let csv = b"a,b,c\n";
        let config = DsvConfig::default();
        let index = build_index(csv, &config);

        let cursor = DsvCursor::new(csv, &index);
        let row = DsvRow::from_cursor(cursor);

        assert_eq!(row.get(0), Some(b"a".as_slice()));
        assert_eq!(row.get(1), Some(b"b".as_slice()));
        assert_eq!(row.get(2), Some(b"c".as_slice()));
        assert_eq!(row.get(3), None);
    }

    #[test]
    fn test_strip_quotes() {
        assert_eq!(strip_quotes(b"\"hello\""), b"hello");
        assert_eq!(strip_quotes(b"hello"), b"hello");
        assert_eq!(strip_quotes(b"\"\""), b"");
        assert_eq!(strip_quotes(b"\""), b"\"");
    }
}
