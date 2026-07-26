//! Scalar kernels shared by every YAML SIMD backend.
//!
//! These are not "the no-SIMD fallback" — each vector kernel calls them to
//! finish the sub-chunk remainder its vector loop cannot cover, so they run on
//! every target. They also stand in as the whole implementation on platforms
//! with no backend at all, and under `--features scalar-yaml`.
//!
//! They lived once per backend file (`mod.rs`, `x86.rs`, `neon.rs`) until #185.
//! The copies were byte-identical apart from `find_block_scalar_end_scalar`'s
//! return type, which `mod.rs` wrapped in an `Option` that was always `Some`.
//! Nothing forced them to stay in step: a build compiles at most one backend, so
//! a fix applied to one copy left the others untouched and the compiler silent.

/// Scan forward to the end of a YAML anchor or alias name.
///
/// Returns the position of the first terminator, or `input.len()` if the name
/// runs to end of input. Terminators are whitespace (space, tab, LF, CR), the
/// flow indicators `[ ] { } ,`, and a `:` that is followed by whitespace — a
/// bare `:` is legal inside an anchor name.
pub(super) fn parse_anchor_name_scalar(input: &[u8], start: usize) -> usize {
    let mut pos = start;
    while pos < input.len() {
        let b = input[pos];
        match b {
            // Stop at flow indicators, whitespace, and newlines
            b' ' | b'\t' | b'\n' | b'\r' | b'[' | b']' | b'{' | b'}' | b',' => break,
            // Colon is allowed in anchor names if not followed by whitespace
            b':' => {
                if pos + 1 < input.len() {
                    let next = input[pos + 1];
                    if next == b' ' || next == b'\t' || next == b'\n' || next == b'\r' {
                        break;
                    }
                }
                pos += 1;
            }
            _ => pos += 1,
        }
    }
    pos
}

/// Scan forward to the end of a `|` or `>` block scalar.
///
/// Returns the start of the first line whose content sits at less than
/// `min_indent` spaces, or `input.len()` if no such line exists. Blank lines
/// belong to the block however they are indented, so only a line with real
/// content can end it.
///
/// Both `\n` and `\r` open a new line: YAML 1.2 §5.4 makes a lone CR a line
/// break in its own right, and scanning for LF alone ran a classic-Mac document
/// to EOF as a single block (#324).
pub(super) fn find_block_scalar_end_scalar(input: &[u8], start: usize, min_indent: usize) -> usize {
    let mut pos = start;

    while pos < input.len() {
        if matches!(input[pos], b'\n' | b'\r') {
            let line_start = pos + 1;

            if line_start >= input.len() {
                return input.len();
            }

            // Count leading spaces
            let mut indent = 0;
            while line_start + indent < input.len() && input[line_start + indent] == b' ' {
                indent += 1;
            }

            // Check if this line has content at insufficient indent
            if line_start + indent < input.len() {
                let next_char = input[line_start + indent];
                if next_char != b'\n' && next_char != b'\r' && indent < min_indent {
                    return line_start;
                }
            }
        }
        pos += 1;
    }

    input.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_anchor_name_stops_at_each_terminator_class() {
        // Whitespace, flow indicators, and `: ` all end a name; a bare `:` does not.
        for (input, expected, what) in [
            (b"anchor rest".as_slice(), 6, "space"),
            (b"anchor\trest".as_slice(), 6, "tab"),
            (b"anchor\nrest".as_slice(), 6, "LF"),
            (b"anchor\rrest".as_slice(), 6, "CR"),
            (b"anchor]rest".as_slice(), 6, "close bracket"),
            (b"anchor}rest".as_slice(), 6, "close brace"),
            (b"anchor,rest".as_slice(), 6, "comma"),
            (b"anchor: rest".as_slice(), 6, "colon then space"),
            (
                b"anchor:rest".as_slice(),
                11,
                "bare colon is part of the name",
            ),
            (b"anchor".as_slice(), 6, "end of input"),
        ] {
            assert_eq!(
                parse_anchor_name_scalar(input, 0),
                expected,
                "{what}: {:?}",
                core::str::from_utf8(input).unwrap_or("<non-utf8>")
            );
        }
    }

    #[test]
    fn parse_anchor_name_honours_the_start_offset() {
        // A trailing colon with nothing after it is still part of the name.
        assert_eq!(parse_anchor_name_scalar(b"*alias more", 1), 6);
        assert_eq!(parse_anchor_name_scalar(b"name:", 0), 5);
    }

    #[test]
    fn find_block_scalar_end_stops_at_the_first_dedented_content_line() {
        // Same document under each break form: the block ends at `c` every time.
        // Offsets differ because CRLF is two bytes, so assert on the byte landed
        // on rather than the number (#324).
        for (input, form) in [
            (b"  a\n  b\nc\n".as_slice(), "LF"),
            (b"  a\r\n  b\r\nc\r\n".as_slice(), "CRLF"),
            (b"  a\r  b\rc\r".as_slice(), "lone CR"),
        ] {
            let end = find_block_scalar_end_scalar(input, 0, 2);
            assert_eq!(
                input.get(end).copied(),
                Some(b'c'),
                "{form}: ended at {end}, which is {:?}",
                input.get(end).map(|&b| b as char)
            );
        }
    }

    #[test]
    fn find_block_scalar_end_keeps_blank_lines_and_runs_to_eof_without_a_dedent() {
        // A blank line is not content, so it cannot end the block...
        let input = b"  a\n\n  b\nc\n";
        assert_eq!(
            input.get(find_block_scalar_end_scalar(input, 0, 2)),
            Some(&b'c')
        );

        // ...and with nothing dedented, the block owns the rest of the input.
        for input in [
            b"  a\n  b\n".as_slice(),
            b"  a\r  b\r".as_slice(),
            b"".as_slice(),
        ] {
            assert_eq!(find_block_scalar_end_scalar(input, 0, 2), input.len());
        }
    }
}
