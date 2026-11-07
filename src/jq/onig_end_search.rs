//! jq 1.7.1's Oniguruma (6.9.8) searches a pattern that matches only the empty string and is
//! anchored at the end of the text at that end alone, and on that route a look-behind sees no
//! further than one character past where the search began (#6510).
//!
//! `search_in_range` (`regexec.c`) moves the start of a search to `end - anc_dist_max` when the
//! pattern's right anchor (`opt.anc.right`, set by `optimize_nodes` in `regcomp.c`) is `\z`
//! (`ANCR_END_BUF`), or `$` and `\Z` (`ANCR_SEMI_END_BUF`) on a text with no final newline. A
//! pattern that consumes nothing has `anc_dist_max == 0`, so the start lands on the end of its
//! range, and a search whose start is not before its range is a backward one. The backward
//! search hands `match_at` the search's own start plus one character as its right range, and
//! `OP_STEP_BACK_START`, the fixed-length look-behind (every look-behind is fixed-length in jq's
//! `ONIG_SYNTAX_PERL_NG`, after `tune_look_behind` cuts `x?` to nothing and `x+` to `x`), reads
//! its body against that range. A body that consumes a character therefore fails once two or
//! more characters follow the search's start: `(?<=b)\z` never matches `"ab"`, `(?<!b)\z`
//! always does, and `"b"` alone reads as written.
//!
//! [`rewrite`] reads that condition off a pattern, and writes the pattern as that search reads
//! it: each look-behind that consumes becomes a group that never matches (`(?<=X)`) or always
//! does (`(?<!X)`), with its body kept behind a `(?!)` so that the groups keep their numbers.
//! The right anchor is Oniguruma's: an item's anchor reaches the pattern's right edge across
//! items that consume nothing, a group passes on its body's, an alternation keeps only what all
//! its ways have, and a lookaround passes on none.
//!
//! The pattern is the one the jq pre-pass wrote (classes rewritten, flags prefixed). Anything the
//! reader below does not know, a quantifier on an item that consumes nothing among them, leaves
//! the pattern as it was.

use alloc::string::String;
use alloc::vec::Vec;

/// `\z`: the end of the text.
const END: u8 = 1;
/// `$` and `\Z`: the end of the text, or before a final newline.
const SEMI_END: u8 = 2;

/// A pattern that the end-anchored search reads differently.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EndSearch {
    /// The pattern as that search reads it.
    pub(crate) pattern: String,
    /// Whether the anchor is only `$` or `\Z`, so that a text ending in a newline is searched as
    /// usual (from before that newline, forward).
    pub(crate) semi_only: bool,
}

#[derive(Debug)]
enum Node {
    /// An item that consumes a character, or a backreference, whose length is not known here.
    Consumes,
    /// An item that consumes nothing, with the right anchor it is.
    Zero { anchor: u8 },
    /// A group, by what it is and where it stands in the pattern (`(` to `)`, both included).
    Group {
        kind: Kind,
        body: Vec<Vec<Self>>,
        open: usize,
        body_at: usize,
        close: usize,
    },
    /// A quantified item, by its least count.
    Repeat { item: Box<Self>, min: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A capturing, non-capturing, atomic or flag group: the body's own reading.
    Plain,
    /// `(?=` or `(?!`.
    Ahead,
    /// `(?<=` (`negative == false`) or `(?<!`.
    Behind { negative: bool },
}

struct Reader<'p> {
    text: &'p [u8],
    at: usize,
}

/// Whether `(?m)` is on: Perl's, under which `$` is the end of a line (`ANCR_END_LINE`), no
/// anchor of the text.
type MultiLine = bool;

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        let found = self.peek() == Some(byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn starts(&self, prefix: &[u8]) -> bool {
        self.text[self.at..].starts_with(prefix)
    }

    /// The ways of an alternation, up to its `)` (left unread) or the end of the pattern.
    fn ways(&mut self, mut multi_line: MultiLine) -> Option<Vec<Vec<Node>>> {
        let mut ways = Vec::new();
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some(b')') => break,
                Some(b'|') => {
                    self.at += 1;
                    ways.push(core::mem::take(&mut items));
                }
                Some(b'*' | b'+' | b'?') if items.is_empty() => return None,
                Some(b'*' | b'?') => {
                    self.at += 1;
                    self.repeat(&mut items, 0)?;
                }
                Some(b'+') => {
                    self.at += 1;
                    self.repeat(&mut items, 1)?;
                }
                // `{,n}` is a literal in Perl's syntax and a count in Ruby's: not read here.
                Some(b'{') if self.starts(b"{,") => return None,
                Some(b'{') => match self.interval() {
                    Some(_) if items.is_empty() => return None,
                    Some(min) => self.repeat(&mut items, min)?,
                    None => {
                        self.at += 1;
                        items.push(Node::Consumes);
                    }
                },
                Some(_) => {
                    let node = self.item(&mut multi_line)?;
                    items.push(node);
                }
            }
        }
        ways.push(items);
        Some(ways)
    }

    /// The quantifier just read applies to the last item, and a lazy `?` or possessive `+` may
    /// follow it.
    fn repeat(&mut self, items: &mut Vec<Node>, min: u32) -> Option<()> {
        let item = items.pop()?;
        if !self.eat(b'?') {
            self.eat(b'+');
        }
        items.push(Node::Repeat {
            item: Box::new(item),
            min,
        });
        Some(())
    }

    /// `{n}`, `{n,}`, `{n,m}` or `{,m}` at the reader, read through, by its least count; `None`
    /// (nothing read) where the brace is a literal one.
    fn interval(&mut self) -> Option<u32> {
        let rest = &self.text[self.at + 1..];
        let close = rest.iter().position(|&b| b == b'}')?;
        let inside = core::str::from_utf8(&rest[..close]).ok()?;
        let (low, high) = match inside.split_once(',') {
            Some((low, high)) => (low, Some(high)),
            None => (inside, None),
        };
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        // `{,n}` is read before this is called; an empty or non-numeric count is a literal brace.
        if low.is_empty() || !digits(low) || !high.map_or(true, digits) {
            return None;
        }
        let min = low.parse().ok()?;
        self.at += close + 2;
        Some(min)
    }

    /// One item other than a quantifier or `|`. A flag switch or a comment is an item that
    /// consumes nothing and is no anchor.
    fn item(&mut self, multi_line: &mut MultiLine) -> Option<Node> {
        let byte = self.peek()?;
        self.at += 1;
        Some(match byte {
            b'\\' => self.escape()?,
            b'[' => {
                self.class()?;
                Node::Consumes
            }
            b'^' => Node::Zero { anchor: 0 },
            b'$' => Node::Zero {
                anchor: if *multi_line { 0 } else { SEMI_END },
            },
            b'(' => return self.group(multi_line),
            // `x` (extended) gives whitespace and `#` a meaning this reader does not follow; it
            // is seen as a flag below, never here. Any other byte is a literal.
            _ => Node::Consumes,
        })
    }

    fn escape(&mut self) -> Option<Node> {
        let byte = self.peek()?;
        self.at += 1;
        Some(match byte {
            b'z' => Node::Zero { anchor: END },
            b'Z' => Node::Zero { anchor: SEMI_END },
            b'A' | b'G' | b'b' | b'B' => Node::Zero { anchor: 0 },
            // `\K`, quoting, control escapes, calls and the anchors this reader has not been
            // checked against.
            b'K' | b'Q' | b'E' | b'c' | b'C' | b'M' | b'g' | b'y' | b'Y' | b'R' | b'X' | b'O' => {
                return None;
            }
            b'p' | b'P' | b'x' | b'o' | b'u' | b'k' | b'N'
                if self.peek() == Some(b'{') || byte == b'k' =>
            {
                // A braced argument (`\p{L}`, `\x{41}`), or a named backreference (`\k<n>`).
                let close = match byte {
                    b'k' => {
                        let open = self.peek()?;
                        let close = match open {
                            b'<' => b'>',
                            b'\'' => b'\'',
                            _ => return None,
                        };
                        self.at += 1;
                        close
                    }
                    _ => {
                        self.at += 1;
                        b'}'
                    }
                };
                let end = self.text[self.at..].iter().position(|&b| b == close)?;
                self.at += end + 1;
                Node::Consumes
            }
            b'x' => {
                self.skip_while(2, |b| b.is_ascii_hexdigit());
                Node::Consumes
            }
            b'u' => {
                self.skip_while(4, |b| b.is_ascii_hexdigit());
                Node::Consumes
            }
            b'0' => {
                self.skip_while(2, |b| (b'0'..=b'7').contains(&b));
                Node::Consumes
            }
            // A backreference, or an octal escape Oniguruma reads in its place.
            b'1'..=b'9' => {
                self.skip_while(2, |b| b.is_ascii_digit());
                Node::Consumes
            }
            _ => Node::Consumes,
        })
    }

    fn skip_while(&mut self, most: usize, keep: impl Fn(u8) -> bool) {
        let mut taken = 0;
        while taken < most && self.peek().is_some_and(&keep) {
            self.at += 1;
            taken += 1;
        }
    }

    /// A bracket class, its `[` read, through its `]`.
    fn class(&mut self) -> Option<()> {
        self.eat(b'^');
        // A `]` first in the class is a literal one.
        self.eat(b']');
        let mut depth = 1;
        while depth > 0 {
            match self.peek()? {
                b'\\' => self.at += 1,
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            self.at += 1;
        }
        Some(())
    }

    /// A group, its `(` read, through its `)`, or a flag switch or a comment.
    fn group(&mut self, multi_line: &mut MultiLine) -> Option<Node> {
        let open = self.at - 1;
        let mut inner = *multi_line;
        let kind = if !self.eat(b'?') || self.eat(b':') || self.eat(b'>') {
            Kind::Plain
        } else if self.eat(b'=') || self.eat(b'!') {
            Kind::Ahead
        } else if self.starts(b"<=") || self.starts(b"<!") {
            let negative = self.text[self.at + 1] == b'!';
            self.at += 2;
            Kind::Behind { negative }
        } else if self.eat(b'#') {
            let close = self.text[self.at..].iter().position(|&b| b == b')')?;
            self.at += close + 1;
            return Some(Node::Zero { anchor: 0 });
        } else if self.eat(b'<') || self.eat(b'\'') || self.starts(b"P<") {
            self.eat(b'P');
            self.eat(b'<');
            let close = self.text[self.at..]
                .iter()
                .position(|&b| b == b'>' || b == b'\'')?;
            self.at += close + 1;
            Kind::Plain
        } else {
            // Flags: `(?im-sx)` switches them to the end of the group it stands in, `(?im-sx:`
            // for its own body.
            let mut on = true;
            loop {
                match self.peek()? {
                    b'-' => on = false,
                    b'i' | b's' => {}
                    b'm' => inner = on,
                    b')' => {
                        self.at += 1;
                        *multi_line = inner;
                        return Some(Node::Zero { anchor: 0 });
                    }
                    b':' => break,
                    // `x` and the flags this reader has not been checked against.
                    _ => return None,
                }
                self.at += 1;
            }
            self.at += 1;
            Kind::Plain
        };
        let body_at = self.at;
        let body = self.ways(inner)?;
        if !self.eat(b')') {
            return None;
        }
        Some(Node::Group {
            kind,
            body,
            open,
            body_at,
            close: self.at - 1,
        })
    }
}

/// Whether the ways consume nothing, and if so the right anchor they share; `None` when they
/// may consume, or hold what this reading does not follow.
fn zero_anchor(ways: &[Vec<Node>]) -> Option<u8> {
    let mut shared = END | SEMI_END;
    for way in ways {
        let mut anchor = 0;
        for node in way {
            anchor |= match node {
                Node::Consumes | Node::Repeat { .. } => return None,
                Node::Zero { anchor } => *anchor,
                Node::Group {
                    kind: Kind::Plain,
                    body,
                    ..
                } => zero_anchor(body)?,
                Node::Group { .. } => 0,
            };
        }
        shared &= anchor;
    }
    Some(shared)
}

/// Whether a look-behind's body consumes a character on every way. A body that can match the
/// empty string is no look-behind to Oniguruma: `tune_look_behind` writes it as an empty node
/// (`ci.min == 0`), which always holds, or for `(?<!` never does, on either search (`(?<=b|)`,
/// `(?<=b|c?)`). A count of zero drops its item (`x?` is cut to `x{0}`).
fn consumes(ways: &[Vec<Node>]) -> bool {
    fn item(node: &Node) -> bool {
        match node {
            Node::Consumes => true,
            Node::Zero { .. } => false,
            Node::Group {
                kind: Kind::Plain,
                body,
                ..
            } => consumes(body),
            Node::Group { .. } => false,
            Node::Repeat { item: inner, min } => *min > 0 && item(inner),
        }
    }
    ways.iter().all(|way| way.iter().any(item))
}

/// Writes each look-behind that consumes, inside `ways`, as the end-anchored search reads it.
fn write(ways: &[Vec<Node>], text: &str, out: &mut String, from: &mut usize) {
    for node in ways.iter().flatten() {
        write_node(node, text, out, from);
    }
}

fn write_node(node: &Node, text: &str, out: &mut String, from: &mut usize) {
    match node {
        Node::Group {
            kind: Kind::Behind { negative },
            body,
            open,
            body_at,
            close,
        } if consumes(body) => {
            out.push_str(&text[*from..*open]);
            out.push_str(if *negative {
                "(?:|(?!)(?:"
            } else {
                "(?:(?!)(?:"
            });
            out.push_str(&text[*body_at..*close]);
            out.push_str("))");
            *from = close + 1;
        }
        Node::Group { body, .. } => write(body, text, out, from),
        Node::Repeat { item, .. } => write_node(item, text, out, from),
        Node::Consumes | Node::Zero { .. } => {}
    }
}

/// The pattern as Oniguruma's end-anchored search reads it, when that search is the one it gets
/// and a look-behind in it reads differently there (see the module documentation).
pub(crate) fn rewrite(pattern: &str) -> Option<EndSearch> {
    if !pattern.contains("(?<") {
        return None;
    }
    let mut reader = Reader {
        text: pattern.as_bytes(),
        at: 0,
    };
    let ways = reader.ways(false)?;
    if reader.at != pattern.len() {
        return None;
    }
    let anchor = zero_anchor(&ways)?;
    if anchor == 0 {
        return None;
    }
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut from = 0;
    write(&ways, pattern, &mut out, &mut from);
    if from == 0 {
        return None;
    }
    out.push_str(&pattern[from..]);
    Some(EndSearch {
        pattern: out,
        semi_only: anchor & END == 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(pattern: &str) -> Option<(String, bool)> {
        rewrite(pattern).map(|search| (search.pattern, search.semi_only))
    }

    #[test]
    fn a_consuming_look_behind_before_the_end_is_written_as_the_search_reads_it() {
        let cases: &[(&str, Option<(&str, bool)>)] = &[
            (r"(?<=b)\z", Some((r"(?:(?!)(?:b))\z", false))),
            (r"(?<!b)\z", Some((r"(?:|(?!)(?:b))\z", false))),
            (r"(?<=b)$", Some((r"(?:(?!)(?:b))$", true))),
            (r"\Z(?<=b)", Some((r"\Z(?:(?!)(?:b))", true))),
            (r"(?<=a|b)\z", Some((r"(?:(?!)(?:a|b))\z", false))),
            (r"(?=(?<=b))\z", Some((r"(?=(?:(?!)(?:b)))\z", false))),
            (r"(?<=(?<=b))\z", Some((r"(?<=(?:(?!)(?:b)))\z", false))),
            (r"((?<=(b)))\b\z", Some((r"((?:(?!)(?:(b))))\b\z", false))),
            (r"(?<=b)\z$", Some((r"(?:(?!)(?:b))\z$", false))),
            (
                r"(?:(?<=b)|(?<=a))\z",
                Some((r"(?:(?:(?!)(?:b))|(?:(?!)(?:a)))\z", false)),
            ),
            (r"(?<=[ab]{2})\z", Some((r"(?:(?!)(?:[ab]{2}))\z", false))),
            (r"(?<=\p{L}+)\z", Some((r"(?:(?!)(?:\p{L}+))\z", false))),
        ];
        for (pattern, want) in cases {
            let want = want.map(|(text, semi)| (text.to_string(), semi));
            assert_eq!(written(pattern), want, "{pattern}");
        }
    }

    #[test]
    fn the_search_is_left_alone_where_oniguruma_searches_as_usual() {
        for pattern in [
            // Consumes: the start is not moved onto the end.
            r"b(?<=b)\z",
            r"(?<=b)\z|x",
            // No anchor at the right edge, or not one every way has.
            r"(?<=b)(?=\z)",
            r"(?<=b)\z|(?<=a)\Z",
            r"(?<=b)(?:\z|\Z)",
            r"(?<=b)",
            // `(?m)` makes `$` the end of a line.
            r"(?m)(?<=b)$",
            r"(?m:(?<=b)$)",
            // A look-behind that consumes nothing reads the same on either search.
            r"(?<=\b)\z",
            r"(?<=b?)\z",
            r"(?<=b{0})\z",
            // A way that matches nothing makes the look-behind an empty node (#6516 review).
            r"(?<=b|)\z",
            r"(?<=|b)\z",
            r"(?<=b|c?)\z",
            r"(?<=(?:b|))\z",
            r"(?<!b?|c)\z",
            // What this reader does not follow.
            r"(?x)(?<=b)\z",
            r"(?:(?<=b)\z)+",
            r"(?<=b)\K\z",
            r"(?<=b)\z)",
            r"\z",
        ] {
            assert_eq!(written(pattern), None, "{pattern}");
        }
    }

    #[test]
    fn a_switch_reaches_the_rest_of_its_group() {
        assert_eq!(written(r"(?:(?m))(?<=b)$").map(|w| w.1), Some(true));
        assert_eq!(written(r"(?m)(?-m)(?<=b)$").map(|w| w.1), Some(true));
        assert_eq!(written(r"(?i)(?<=B)\z").map(|w| w.1), Some(false));
        assert_eq!(written(r"(?m)(?<=b)\z").map(|w| w.1), Some(false));
        assert_eq!(written(r"(?#c)(?<=b)\z").map(|w| w.1), Some(false));
        assert_eq!(written(r"(?<n>(?<=b))\z").map(|w| w.1), Some(false));
    }
}
