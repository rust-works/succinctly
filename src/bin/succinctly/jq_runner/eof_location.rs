//! Where jq places a compile error found at the end of a program's text (#7711).
//!
//! jq's lexer (`src/lexer.l`) sets the location of every match it makes in `YY_USER_ACTION`,
//! whitespace and comments included, and its `<<EOF>>` makes no match, so the end-of-file token a
//! syntax error at the end names keeps the location of the last match. jq reports the error at the
//! start of that match: the run of trailing whitespace (which starts right after the last token or
//! comment), or else the last comment or token itself. `printf '[1,\n 2,\n\n\n'` is line 2, ` 2,`
//! echoed and padded to the whitespace run after the `,`, in jq 1.7.1 and 1.8.2 alike.
//!
//! [`last_match_start`] finds that start by matching the text the way jq 1.7.1's flex rules do:
//! longest match, a comment `#[^\n]*` as one match, a whitespace run as one match, and inside a
//! string its own rules (an interpolation's `\(`, a run of escapes, a run of other text). Only
//! where matches begin matters here, not which token each is. A `\r` is a character jq 1.7.1
//! refuses, not whitespace (jq 1.8.2 changed that), as `succinctly jq` refuses it.
//!
//! This runs only to report an error, never on a program that compiles.

/// The lexer state jq's `enter`/`try_exit` keep on flex's state stack: which closer pops it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Paren,
    Bracket,
    Brace,
    /// Inside `\( ... )`: a `)` returns to the string.
    Interpolation,
    String,
}

/// The byte offset at which jq reports a compile error found at the end of `text`: the start of
/// the last match its lexer made. `0` for a text with nothing in it.
pub(super) fn last_match_start(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut stack: Vec<State> = Vec::new();
    let mut at = 0;
    let mut last = 0;
    while at < bytes.len() {
        last = at;
        let length = if stack.last() == Some(&State::String) {
            string_match(bytes, at, &mut stack)
        } else {
            code_match(bytes, at, &mut stack)
        };
        at += length.max(1);
    }
    last
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The length of `([a-zA-Z_][a-zA-Z_0-9]*::)*[a-zA-Z_][a-zA-Z_0-9]*` at `at`, or 0.
fn ident_length(bytes: &[u8], at: usize) -> usize {
    let word = |from: usize| -> usize {
        if !bytes.get(from).copied().is_some_and(is_ident_start) {
            return 0;
        }
        1 + bytes[from + 1..]
            .iter()
            .take_while(|&&b| is_ident(b))
            .count()
    };
    let mut end = at + word(at);
    if end == at {
        return 0;
    }
    while bytes.get(end..end + 2) == Some(b"::") {
        let next = word(end + 2);
        if next == 0 {
            break;
        }
        end += 2 + next;
    }
    end - at
}

fn digits(bytes: &[u8], at: usize) -> usize {
    bytes[at.min(bytes.len())..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count()
}

/// The length of `([0-9]+(\.[0-9]*)?|\.[0-9]+)([eE][+-]?[0-9]+)?` at `at`, or 0.
fn number_length(bytes: &[u8], at: usize) -> usize {
    let mut end = at;
    let whole = digits(bytes, end);
    if whole > 0 {
        end += whole;
        if bytes.get(end) == Some(&b'.') {
            end += 1 + digits(bytes, end + 1);
        }
    } else if bytes.get(end) == Some(&b'.') && digits(bytes, end + 1) > 0 {
        end += 1 + digits(bytes, end + 1);
    } else {
        return 0;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        let exponent = digits(bytes, end + 1 + sign);
        if exponent > 0 {
            end += 1 + sign + exponent;
        }
    }
    end - at
}

/// One match outside a string, its length; `stack` follows `enter` and `try_exit`.
fn code_match(bytes: &[u8], at: usize, stack: &mut Vec<State>) -> usize {
    let rest = &bytes[at..];
    match rest[0] {
        b'#' => return rest.iter().take_while(|&&b| b != b'\n').count(),
        b' ' | b'\n' | b'\t' => {
            return rest
                .iter()
                .take_while(|&&b| matches!(b, b' ' | b'\n' | b'\t'))
                .count()
        }
        b'"' => {
            stack.push(State::String);
            return 1;
        }
        b'(' | b'[' | b'{' => {
            stack.push(match rest[0] {
                b'(' => State::Paren,
                b'[' => State::Bracket,
                _ => State::Brace,
            });
            return 1;
        }
        b')' | b']' | b'}' => {
            let closes = match stack.last() {
                Some(State::Paren | State::Interpolation) => b')',
                Some(State::Bracket) => b']',
                Some(State::Brace) => b'}',
                _ => 0,
            };
            if closes == rest[0] {
                stack.pop();
            }
            return 1;
        }
        _ => {}
    }
    // Longest of the remaining rules; a keyword and an identifier of the same length start at
    // the same place, so which one wins does not matter here.
    const OPERATORS: &[&[u8]] = &[
        b"$__loc__",
        b"?//",
        b"//=",
        b"!=",
        b"==",
        b"//",
        b"|=",
        b"+=",
        b"-=",
        b"*=",
        b"/=",
        b"%=",
        b"<=",
        b">=",
        b"..",
    ];
    let operator = OPERATORS
        .iter()
        .find(|op| rest.starts_with(op))
        .map_or(0, |op| op.len());
    let field = match rest[0] {
        b'.' => match ident_length(bytes, at + 1) {
            0 => 0,
            // FIELD is `\.[a-zA-Z_][a-zA-Z_0-9]*`, with no `::`.
            _ => 1 + 1 + rest[2..].iter().take_while(|&&b| is_ident(b)).count(),
        },
        _ => 0,
    };
    let binding = match rest[0] {
        b'$' => match ident_length(bytes, at + 1) {
            0 => 0,
            length => 1 + length,
        },
        _ => 0,
    };
    let format = match rest[0] {
        b'@' => match rest[1..].iter().take_while(|&&b| is_ident(b)).count() {
            0 => 0,
            length => 1 + length,
        },
        _ => 0,
    };
    [
        operator,
        field,
        binding,
        format,
        number_length(bytes, at),
        ident_length(bytes, at),
        1,
    ]
    .into_iter()
    .max()
    .unwrap_or(1)
}

/// One match inside a string, its length; `\(` and the closing `"` move `stack`.
fn string_match(bytes: &[u8], at: usize, stack: &mut Vec<State>) -> usize {
    let rest = &bytes[at..];
    match rest {
        [b'\\', b'(', ..] => {
            stack.push(State::Interpolation);
            2
        }
        [b'"', ..] => {
            stack.pop();
            1
        }
        [b'\\', ..] => {
            // `(\\[^u(]|\\u[a-zA-Z0-9]{0,4})+`; a lone `\` at the end is a character of its own.
            let mut end = 0;
            loop {
                match &rest[end..] {
                    [b'\\', b'u', tail @ ..] => {
                        end += 2 + tail
                            .iter()
                            .take(4)
                            .take_while(|b| b.is_ascii_alphanumeric())
                            .count();
                    }
                    [b'\\', b'(', ..] | [b'\\'] | [] => break,
                    [b'\\', _, ..] => end += 2,
                    _ => break,
                }
            }
            end.max(1)
        }
        _ => rest
            .iter()
            .take_while(|&&b| b != b'\\' && b != b'"')
            .count(),
    }
}

#[cfg(test)]
mod tests {
    use super::last_match_start;

    /// From jq 1.7.1 (`/usr/bin/jq -n PROGRAM`, the reported offset being the padding of the
    /// echoed line plus that line's start), except `a::` (jq stops at its first `:`), `.foo`,
    /// `..`, `@base64` (no error) and the empty text, which pin the matching rules alone.
    #[test]
    fn the_start_of_jqs_last_match_7711() {
        for (program, offset) in [
            ("1 +", 2),
            ("1 + ", 3),
            ("1 + # c", 4),
            ("\"abc", 1),
            ("\"a\\(1", 4),
            ("[1,", 2),
            (".a.", 2),
            (". as [$", 6),
            ("if 1 then 2", 10),
            ("[1,\n 2,\n\n\n", 7),
            ("def f: 1 +\n", 10),
            ("1 + # c\n\n", 7),
            ("\"x\n\n", 1),
            ("\"a\\n\\q", 2),
            ("\"a\\u12", 2),
            ("$__loc__ +", 9),
            ("a::b::c", 0),
            ("a::", 2),
            ("1.5e+3 +", 7),
            ("1e", 1),
            (".foo", 0),
            ("..", 0),
            ("@base64", 0),
            ("(1))", 3),
            ("", 0),
        ] {
            assert_eq!(last_match_start(program), offset, "{program:?}");
        }
    }
}
