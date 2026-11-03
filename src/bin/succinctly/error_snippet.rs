//! The one-line context a validation error prints under its location (#5988).
//!
//! `jq --validate`, `json validate`, `yaml validate` and `text validate` each cut a long line down
//! to a window around the error. The error's column counts bytes, so a cut at `column - 40` or at
//! byte 80 can fall inside a multi-byte character, and slicing there panics (`end byte index 80 is
//! not a char boundary`, exit 101, on input that is merely invalid). They share this one cut, and
//! each cut point is moved down to a character boundary.

/// The widest line shown whole; a longer one is cut to a window of this many bytes.
const MAX_WIDTH: usize = 80;

/// `line_content` as it is shown under an error at the 1-based byte `column`, and how many
/// characters of the shown line (the `...` prefix included) come before the caret.
///
/// A line of [`MAX_WIDTH`] bytes or fewer is shown whole. A longer one shows its first
/// [`MAX_WIDTH`] bytes when the error is in the first half of one, its last [`MAX_WIDTH`] bytes
/// when it is in the last half, and a window centred on the error otherwise, each side that was
/// cut marked with `...`.
pub(crate) fn truncate_line(line_content: &str, column: usize) -> (String, usize) {
    let floor_boundary = |mut at: usize| {
        at = at.min(line_content.len());
        while !line_content.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let chars_between = |from: usize, to: usize| line_content[from..to].chars().count();

    let error_at = floor_boundary(column.saturating_sub(1));
    if line_content.len() <= MAX_WIDTH {
        return (line_content.to_string(), chars_between(0, error_at));
    }
    if error_at < MAX_WIDTH / 2 {
        // Error is near the start, show the beginning.
        let truncated = &line_content[..floor_boundary(MAX_WIDTH)];
        (format!("{truncated}..."), chars_between(0, error_at))
    } else if error_at >= line_content.len().saturating_sub(MAX_WIDTH / 2) {
        // Error is near the end, show the end. The cut moves down, so the window can be a few
        // bytes wider than `MAX_WIDTH`.
        let start = floor_boundary(line_content.len() - MAX_WIDTH);
        let truncated = &line_content[start..];
        (
            format!("...{truncated}"),
            chars_between(start, error_at) + 3,
        )
    } else {
        // Error is in the middle, centre on it.
        let start = floor_boundary(error_at - MAX_WIDTH / 2);
        let end = floor_boundary(start + MAX_WIDTH);
        let truncated = &line_content[start..end];
        (
            format!("...{truncated}..."),
            chars_between(start, error_at) + 3,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_line_is_shown_whole() {
        assert_eq!(truncate_line("abc def", 5), ("abc def".to_string(), 4));
        // A column past the end of the line puts the caret at its end.
        assert_eq!(truncate_line("abc", 99), ("abc".to_string(), 3));
        assert_eq!(truncate_line("", 1), (String::new(), 0));
    }

    #[test]
    fn a_long_ascii_line_is_cut_around_the_error() {
        let line = "x".repeat(200);
        // Near the start: the first 80 bytes.
        let (shown, caret) = truncate_line(&line, 11);
        assert_eq!((shown.len(), caret), (83, 10));
        assert!(shown.ends_with("..."));
        // In the middle: 80 bytes centred on the error, marked on both sides.
        let (shown, caret) = truncate_line(&line, 101);
        assert_eq!((shown.len(), caret), (86, 43));
        assert!(shown.starts_with("...") && shown.ends_with("..."));
        // Near the end: the last 80 bytes, marked on the left only.
        let (shown, caret) = truncate_line(&line, 195);
        assert_eq!((shown.len(), caret), (83, 77));
        assert!(shown.starts_with("...") && !shown.ends_with("..."));
    }

    /// #5988: every cut point of a line of 3-byte characters, at every column, lands on a boundary.
    #[test]
    fn a_multibyte_line_is_never_cut_inside_a_character() {
        for width in [27usize, 40, 41, 80, 3000] {
            let line = "€".repeat(width);
            for column in 0..=line.len() + 2 {
                let (shown, caret) = truncate_line(&line, column);
                assert!(
                    shown.chars().filter(|&c| c == '€').count() > 0,
                    "{width} {column}"
                );
                assert!(
                    caret <= shown.chars().count(),
                    "{width} {column}: caret {caret}"
                );
            }
        }
        // Two- and four-byte characters, and a mix with ASCII.
        for unit in ["é", "𝄞", "aé€𝄞"] {
            let line = unit.repeat(120);
            for column in 0..=line.len() + 2 {
                let (shown, caret) = truncate_line(&line, column);
                assert!(caret <= shown.chars().count(), "{unit} {column}");
            }
        }
    }

    /// The caret counts characters, so it stands under the character the error names even when
    /// the characters before it take several bytes each.
    #[test]
    fn the_caret_counts_characters_not_bytes() {
        // `"` + 40 euros + `" 1`: the error is at the `1`, byte 123, column 124.
        let line = format!("\"{}\" 1", "€".repeat(40));
        assert_eq!(line.len(), 124);
        let (shown, caret) = truncate_line(&line, 124);
        assert_eq!(shown.chars().nth(caret), Some('1'));
        // Whole short line: the third euro sign is at column 7 (bytes 6..9).
        let (shown, caret) = truncate_line("€€€€", 7);
        assert_eq!((shown.as_str(), caret), ("€€€€", 2));
    }
}
