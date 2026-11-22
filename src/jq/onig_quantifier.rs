//! What Oniguruma makes of a quantifier that follows a quantifier (#7682), written out for the
//! `regex` crate and `fancy-regex`.
//!
//! jq compiles with `ONIG_SYNTAX_PERL_NG`. Its tokenizer (`fetch_token`, `regparse.c`) reads a
//! quantifier (`?`, `*`, `+`, `{n}`, `{n,}`, `{n,m}`) and then looks at the character right after
//! it: a `?` makes it lazy, otherwise a `+` makes it possessive (an interval too, `{n}` included,
//! since the syntax has `ONIG_SYN_OP2_PLUS_POSSESSIVE_INTERVAL`). Nothing else belongs to it, so any
//! further `?`, `*`, `+` or interval is a new quantifier on the quantified atom: `a??++` is `a??`,
//! then `++`; `a{2,}?{2}` is `a{2,}?`, then `{2}`. A blank or a `#` comment under `x`, and a `(?#..)`
//! comment, also end a quantifier (the marker has to touch it), so `a* ?` is `a*`, then `?`.
//!
//! The parser (`assign_quantifier_body`) then applies the new quantifier to the old one, and
//! reduces the pair where it can: two of `?`, `*`, `+`, `??`, `*?` and `+?` become one by
//! `ReduceTypeTable` (`a??+` is `a*?`), two fixed counts multiply (`a{2}{3}` is `a{6}`), a bounded
//! greedy interval over `*` or `+` keeps only its least count (`(?:a*){2,3}` is `(?:a*){2}`), and a
//! `{1}` is no quantifier at all, its `+` included. A possessive quantifier is the atomic group of
//! the repeat it ends (`BAG_STOP_BACKTRACK`), which no later quantifier reduces with.
//!
//! The engines read such a run otherwise: the `regex` crate takes each quantifier as a repeat of
//! the repeat before it but has no possessive form, and `fancy-regex` in `oniguruma_mode` takes a
//! `+` after a lazy quantifier as a possessive marker. So the run is written as the tree Oniguruma
//! builds: one quantifier per item, a plain group (`(?:`) around a repeat that another repeats, and
//! an atomic group (`(?>`) for a possessive quantifier, except a greedy one that ends the run, which
//! stays `Q+` for the possessive scan in `eval.rs` to read as it reads any other. A fixed count is
//! possessive or not alike when its item matches in one way only (a character, a class, `.`), and
//! is then written plain.
//!
//! A run on an atom the reader cannot place (after an escape it does not size, a `\Q..\E` quote, an
//! anchor, a lookaround or a group it does not know) leaves the whole pattern as written, to the
//! rules it had before.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// An unbounded count.
const INF: u32 = u32::MAX;

/// Oniguruma's `ONIG_MAX_REPEAT_NUM`: a count past it is an error, which the pattern as written
/// reports.
const MAX_REPEAT: u32 = 100_000;

/// One repeat: its counts and whether it is greedy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Repeat {
    lower: u32,
    upper: u32,
    greedy: bool,
}

impl Repeat {
    fn fixed(self) -> bool {
        self.lower == self.upper
    }

    /// Oniguruma's `quantifier_type_num`: `?`, `*`, `+`, `??`, `*?`, `+?` are 0 to 5.
    fn popular(self) -> Option<usize> {
        let kind = match (self.lower, self.upper) {
            (0, 1) => 0,
            (0, INF) => 1,
            (1, INF) => 2,
            _ => return None,
        };
        Some(if self.greedy { kind } else { kind + 3 })
    }

    /// The quantifier as the engines read it. A fixed count is written without a lazy marker: it
    /// repeats its item that many times either way, and `fancy-regex` would read `{n}?` as Ruby's
    /// optional `(?:x{n})?` (#7669).
    fn write(self, out: &mut String) {
        match (self.lower, self.upper) {
            (0, 1) => out.push('?'),
            (0, INF) => out.push('*'),
            (1, INF) => out.push('+'),
            (n, m) if n == m => {
                out.push_str(&alloc::format!("{{{n}}}"));
                return;
            }
            (n, INF) => out.push_str(&alloc::format!("{{{n},}}")),
            (n, m) => out.push_str(&alloc::format!("{{{n},{m}}}")),
        }
        if !self.greedy {
            out.push('?');
        }
    }
}

/// What Oniguruma's `onig_reduce_nested_quantifier` makes of an inner repeat (the row) under an
/// outer one (the column), both popular.
#[derive(Clone, Copy)]
enum Reduce {
    AsIs,
    /// The outer repeat goes: the inner one stands alone.
    Delete,
    /// One repeat with these counts replaces both.
    To(u32, u32, bool),
    /// `(?:x+)??`.
    PlusThenLazyOptional,
}

const TABLE: [[Reduce; 6]; 6] = {
    use Reduce::{AsIs, Delete as Del, PlusThenLazyOptional as PQq};
    const A: Reduce = Reduce::To(0, INF, true);
    const P: Reduce = Reduce::To(1, INF, true);
    const AQ: Reduce = Reduce::To(0, INF, false);
    const QQ: Reduce = Reduce::To(0, 1, false);
    [
        [Del, A, A, QQ, AQ, AsIs],      // '?'
        [Del, Del, Del, PQq, PQq, Del], // '*'
        [A, A, Del, AsIs, PQq, Del],    // '+'
        [Del, AQ, AQ, Del, AQ, AQ],     // '??'
        [Del, Del, Del, Del, Del, Del], // '*?'
        [AsIs, A, P, AQ, AQ, Del],      // '+?'
    ]
};

/// The tree Oniguruma builds over an atom.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Atom,
    Quant(Repeat, Box<Self>),
    Atomic(Box<Self>),
}

/// One quantifier token: its repeat and whether a `+` made it possessive.
#[derive(Clone, Copy, Debug)]
struct Token {
    repeat: Repeat,
    possessive: bool,
}

/// `node` under one more quantifier, as `assign_quantifier_body` builds it.
fn apply(node: Node, token: Token) -> Node {
    let mut outer = token.repeat;
    // `x{1}` is `x`, and its possessive marker goes with it.
    if outer.lower == 1 && outer.upper == 1 {
        return node;
    }
    let quantified = match node {
        Node::Quant(inner, body) => match (inner.popular(), outer.popular()) {
            (Some(kind), None) => {
                // `(?:a*){n,m}` and `(?:a+){n,m}` are read as `{n,n}`.
                if (kind == 1 || kind == 2) && outer.upper != INF && outer.upper > 1 && outer.greedy
                {
                    outer.upper = outer.lower.max(1);
                }
                Node::Quant(outer, Box::new(Node::Quant(inner, body)))
            }
            (Some(row), Some(column)) => match TABLE[row][column] {
                Reduce::AsIs => Node::Quant(outer, Box::new(Node::Quant(inner, body))),
                Reduce::Delete => Node::Quant(inner, body),
                Reduce::To(lower, upper, greedy) => Node::Quant(
                    Repeat {
                        lower,
                        upper,
                        greedy,
                    },
                    body,
                ),
                Reduce::PlusThenLazyOptional => Node::Quant(
                    Repeat {
                        lower: 0,
                        upper: 1,
                        greedy: false,
                    },
                    Box::new(Node::Quant(
                        Repeat {
                            lower: 1,
                            upper: INF,
                            greedy: true,
                        },
                        body,
                    )),
                ),
            },
            (None, _) => match outer.lower.checked_mul(inner.lower) {
                Some(product) if outer.fixed() && inner.fixed() && product <= MAX_REPEAT => {
                    Node::Quant(
                        Repeat {
                            lower: product,
                            upper: product,
                            greedy: outer.greedy,
                        },
                        body,
                    )
                }
                _ => Node::Quant(outer, Box::new(Node::Quant(inner, body))),
            },
        },
        other => Node::Quant(outer, Box::new(other)),
    };
    if token.possessive {
        Node::Atomic(Box::new(quantified))
    } else {
        quantified
    }
}

/// Whether `node` matches its text in one way only, so that an atomic group around it changes
/// nothing: a fixed count of an atom that matches in one way, or a count of zero.
fn single_way(node: &Node, simple_atom: bool) -> bool {
    match node {
        Node::Atom => simple_atom,
        Node::Quant(repeat, body) => {
            repeat.upper == 0 || (repeat.fixed() && single_way(body, simple_atom))
        }
        Node::Atomic(_) => true,
    }
}

/// `node` with an atomic group that changes nothing taken off.
fn bare(node: &Node, simple_atom: bool) -> &Node {
    match node {
        Node::Atomic(inner) if single_way(inner, simple_atom) => bare(inner, simple_atom),
        other => other,
    }
}

/// The text written before and after the atom for `node`. `last` says that nothing repeats this
/// node, so that a greedy possessive repeat can stay `Q+` for the possessive scan.
fn write(node: &Node, simple_atom: bool, last: bool) -> (String, String) {
    match bare(node, simple_atom) {
        Node::Atom => (String::new(), String::new()),
        Node::Atomic(inner) => match &**inner {
            Node::Quant(repeat, body) if last && repeat.greedy && !repeat.fixed() => {
                let (before, mut after) = write_quant(*repeat, body, simple_atom);
                after.push('+');
                (before, after)
            }
            _ => {
                let (before, after) = write(inner, simple_atom, false);
                (alloc::format!("(?>{before}"), alloc::format!("{after})"))
            }
        },
        Node::Quant(repeat, body) => write_quant(*repeat, body, simple_atom),
    }
}

/// [`write`] for `repeat` over `body`: a repeat that another repeats is put in a plain group.
fn write_quant(repeat: Repeat, body: &Node, simple_atom: bool) -> (String, String) {
    // A count of zero never runs its body: the atom alone keeps its groups' numbers.
    if repeat.upper == 0 {
        return (String::new(), String::from("{0}"));
    }
    let (mut before, mut after) = write(body, simple_atom, false);
    if let Node::Quant(..) = bare(body, simple_atom) {
        before.insert_str(0, "(?:");
        after.push(')');
    }
    repeat.write(&mut after);
    (before, after)
}

/// What a group's `(` opened.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Group {
    /// A group a quantifier repeats: capturing, named, `(?:`, `(?>`, `(?~`, `(?flags:`.
    Item,
    /// A lookaround, which no quantifier may follow.
    Assertion,
}

/// A quantifier that starts at `chars[at]`, and the index past it; `Err` for one Oniguruma refuses
/// (a reversed or oversized interval), which the pattern as written reports.
fn quantifier_at(chars: &[char], at: usize) -> Result<Option<(Token, usize)>, ()> {
    let (lower, upper, mut end) = match chars.get(at) {
        Some('?') => (0, 1, at + 1),
        Some('*') => (0, INF, at + 1),
        Some('+') => (1, INF, at + 1),
        Some('{') => {
            let number = |from: usize| -> Result<(Option<u32>, usize), ()> {
                let mut j = from;
                let mut value: u32 = 0;
                while let Some(digit) = chars.get(j).and_then(|c| c.to_digit(10)) {
                    value = value.saturating_mul(10).saturating_add(digit);
                    j += 1;
                }
                if value > MAX_REPEAT {
                    return Err(());
                }
                Ok(((j > from).then_some(value), j))
            };
            // `{,n}` and anything but `{n}`, `{n,}`, `{n,m}` are literal text to jq.
            let (Some(lower), j) = number(at + 1)? else {
                return Ok(None);
            };
            let (upper, j) = if chars.get(j) == Some(&',') {
                let (upper, j) = number(j + 1)?;
                (upper.unwrap_or(INF), j)
            } else {
                (lower, j)
            };
            if chars.get(j) != Some(&'}') {
                return Ok(None);
            }
            if lower > upper {
                return Err(());
            }
            (lower, upper, j + 1)
        }
        _ => return Ok(None),
    };
    let mut token = Token {
        repeat: Repeat {
            lower,
            upper,
            greedy: true,
        },
        possessive: false,
    };
    match chars.get(end) {
        Some('?') => {
            token.repeat.greedy = false;
            end += 1;
        }
        Some('+') => {
            token.possessive = true;
            end += 1;
        }
        _ => {}
    }
    Ok(Some((token, end)))
}

/// The end of the blanks and comments at `chars[at..]` that Oniguruma skips between tokens: a
/// `(?#..)` comment, and under `x` white space and a `#` comment to the end of the line.
fn skip_gap(chars: &[char], mut at: usize, x_on: bool) -> usize {
    loop {
        match chars.get(at) {
            Some('(') if chars.get(at + 1) == Some(&'?') && chars.get(at + 2) == Some(&'#') => {
                while at < chars.len() && chars[at] != ')' {
                    at += 1;
                }
                at += 1;
            }
            Some(c) if x_on && c.is_whitespace() => at += 1,
            Some('#') if x_on => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
            }
            _ => return at.min(chars.len()),
        }
    }
}

/// The length of a POSIX bracket (`[:alpha:]`, `[:^digit:]`) at `chars[at]`, if one is there.
fn posix_bracket_len(chars: &[char], at: usize) -> Option<usize> {
    if chars.get(at + 1) != Some(&':') {
        return None;
    }
    let mut j = at + 2;
    if chars.get(j) == Some(&'^') {
        j += 1;
    }
    let name = j;
    while chars.get(j).is_some_and(char::is_ascii_alphabetic) {
        j += 1;
    }
    (j > name && chars.get(j) == Some(&':') && chars.get(j + 1) == Some(&']')).then_some(j + 2 - at)
}

/// Whether `pattern` may hold a quantifier that follows a quantifier, for the cheap exit.
fn may_stack(pattern: &str, extended: bool) -> bool {
    let bytes = pattern.as_bytes();
    bytes.windows(2).any(|pair| {
        matches!(pair[0], b'*' | b'+' | b'?' | b'}') && matches!(pair[1], b'*' | b'+' | b'?' | b'{')
    }) || pattern.contains("(?#")
        || ((extended || pattern.contains("(?"))
            && bytes.iter().any(|&b| b == b'#' || b.is_ascii_whitespace()))
}

/// Whether the run `tokens` on an atom (`simple`: one that matches in one way) is read otherwise by
/// the engines and the possessive scan in `eval.rs` than by Oniguruma. A greedy repeat of a greedy
/// repeat is the same tree in all three (the scan makes a possessive last quantifier the atomic
/// group of the whole run), so a run is written out only where a quantifier follows a lazy or a
/// possessive one, whose marker the engines and the scan take for something else; where it ends
/// in a possessive fixed count of an item that matches in several ways, which the scan reads as the
/// plain count (#6079); and where it ends in a count of zero, whose capture groups keep their
/// numbers only once the count is written on them (#6108). A `{1}` is no quantifier, its `+`
/// included, and counts for none of these.
fn needs_writing(tokens: &[Token], simple: bool) -> bool {
    let counted: Vec<&Token> = tokens
        .iter()
        .filter(|token| !(token.repeat.lower == 1 && token.repeat.upper == 1))
        .collect();
    let Some((last, rest)) = counted.split_last() else {
        return false;
    };
    let one_way = simple && rest.iter().all(|token| token.repeat.fixed());
    rest.iter()
        .any(|token| !token.repeat.greedy || token.possessive)
        || (last.possessive && last.repeat.fixed() && last.repeat.lower > 1 && !one_way)
        || (last.repeat.upper == 0 && !rest.is_empty())
}

/// `pattern` with every run of quantifiers that follow a quantifier written as Oniguruma reads it
/// (see the module comment), and with a possessive `{n}+` on an item that matches in more than one
/// way written as its atomic group. `None` when there is none, or when one stands on an atom the
/// reader cannot place.
pub(crate) fn rewrite(pattern: &str, extended: bool) -> Option<String> {
    if !may_stack(pattern, extended) {
        return None;
    }
    let chars: Vec<char> = pattern.chars().collect();
    // Text to write in place of `chars[start..end]` (an insertion where the two are equal).
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    // The atom a quantifier here would repeat: where it starts and ends, and whether it matches in
    // one way only.
    let mut atom: Option<(usize, usize, bool)> = None;
    // An escape or construct this reader does not size leaves every later atom unknown.
    let mut unsure = false;
    // Each open group: where its `(` is, what it is, and whether `x` was on where it opened.
    let mut groups: Vec<(usize, Group, bool)> = Vec::new();
    let mut x_on = extended;
    let mut i = 0;
    while i < chars.len() {
        let gap_end = skip_gap(&chars, i, x_on);
        if gap_end > i {
            i = gap_end;
            continue;
        }
        if let Some((first, mut end)) = quantifier_at(&chars, i).ok()? {
            let mut tokens = alloc::vec![first];
            while let Some((token, after)) =
                quantifier_at(&chars, skip_gap(&chars, end, x_on)).ok()?
            {
                tokens.push(token);
                end = after;
            }
            let target = atom.take();
            if needs_writing(&tokens, target.is_some_and(|(.., simple)| simple)) {
                let (start, atom_end, simple) = target.filter(|_| !unsure)?;
                let node = tokens
                    .iter()
                    .fold(Node::Atom, |node, &token| apply(node, token));
                let (before, after) = write(&node, simple, true);
                if !before.is_empty() {
                    edits.push((start, start, before));
                }
                edits.push((atom_end, end, after));
            }
            i = end;
            continue;
        }
        let start = i;
        let mut simple = true;
        match chars[i] {
            '\\' => {
                let next = chars.get(i + 1).copied();
                match next {
                    Some('Q') => {
                        // A quantifier after a quoted run repeats its last character, which a group
                        // cannot wrap without opening inside the quote.
                        i += 2;
                        while i < chars.len()
                            && !(chars[i] == '\\' && chars.get(i + 1) == Some(&'E'))
                        {
                            i += 1;
                        }
                        i = (i + 2).min(chars.len());
                        atom = None;
                        continue;
                    }
                    Some('x' | 'p' | 'P' | 'o' | 'k' | 'g' | 'N')
                        if chars.get(i + 2) == Some(&'{') =>
                    {
                        simple = !matches!(next, Some('k' | 'g'));
                        i = chars[i + 3..]
                            .iter()
                            .position(|&c| c == '}')
                            .map_or(chars.len(), |close| i + 3 + close + 1);
                    }
                    Some('x') => {
                        i += 2;
                        for _ in 0..2 {
                            if chars.get(i).is_some_and(char::is_ascii_hexdigit) {
                                i += 1;
                            }
                        }
                    }
                    Some(d) if d.is_ascii_digit() => {
                        // A back-reference or an octal escape, one atom either way.
                        simple = false;
                        i += 1;
                        while chars.get(i).is_some_and(char::is_ascii_digit) {
                            i += 1;
                        }
                    }
                    Some('k' | 'g') if matches!(chars.get(i + 2), Some('<' | '\'')) => {
                        simple = false;
                        let close = if chars[i + 2] == '<' { '>' } else { '\'' };
                        i += 3;
                        while i < chars.len() && chars[i] != close {
                            i += 1;
                        }
                        i = (i + 1).min(chars.len());
                    }
                    Some('c') if i + 2 < chars.len() => i += 3,
                    // An anchor, which no quantifier may repeat.
                    Some('b' | 'B' | 'A' | 'z' | 'Z' | 'G' | 'K' | 'y' | 'Y') => {
                        atom = None;
                        i += 2;
                        continue;
                    }
                    Some('R' | 'X') => {
                        simple = false;
                        i += 2;
                    }
                    Some(c) if c.is_ascii_alphanumeric() && !"dDwWsShHntrfvaeN".contains(c) => {
                        unsure = true;
                        atom = None;
                        i += 2;
                        continue;
                    }
                    Some(_) => i += 2,
                    None => {
                        atom = None;
                        i += 1;
                        continue;
                    }
                }
            }
            '[' => {
                i += 1;
                if chars.get(i) == Some(&'^') {
                    i += 1;
                }
                if chars.get(i) == Some(&']') {
                    i += 1;
                }
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '\\' {
                        i += 2;
                    } else if chars[i] == '[' {
                        match posix_bracket_len(&chars, i) {
                            Some(len) => i += len,
                            // A nested class: where this one ends is not read here.
                            None => {
                                unsure = true;
                                i += 1;
                            }
                        }
                    } else {
                        i += 1;
                    }
                }
                i = (i + 1).min(chars.len());
            }
            '(' => {
                let outer_x = x_on;
                atom = None;
                let kind = if chars.get(i + 1) == Some(&'?') {
                    match chars.get(i + 2).copied() {
                        Some(':' | '>' | '~') => {
                            i += 3;
                            Group::Item
                        }
                        Some('=' | '!') => {
                            i += 3;
                            Group::Assertion
                        }
                        Some('<') if matches!(chars.get(i + 3), Some('=' | '!')) => {
                            i += 4;
                            Group::Assertion
                        }
                        Some(open @ ('<' | '\'')) => {
                            let close = if open == '<' { '>' } else { '\'' };
                            i += 3;
                            while i < chars.len() && chars[i] != close {
                                i += 1;
                            }
                            i += 1;
                            Group::Item
                        }
                        Some(c) if c.is_ascii_alphabetic() || c == '-' => {
                            // Flags, then `:` for a group of its own or `)` for a switch, which
                            // sets them to the end of the group it stands in.
                            let mut j = i + 2;
                            let mut off = false;
                            let mut x = x_on;
                            while let Some(&c) = chars
                                .get(j)
                                .filter(|c| c.is_ascii_alphabetic() || **c == '-')
                            {
                                match c {
                                    '-' => off = true,
                                    'x' => x = !off,
                                    _ => {}
                                }
                                j += 1;
                            }
                            match chars.get(j) {
                                Some(':') => {
                                    groups.push((start, Group::Item, outer_x));
                                    x_on = x;
                                    i = j + 1;
                                }
                                Some(')') => {
                                    x_on = x;
                                    i = j + 1;
                                }
                                _ => {
                                    unsure = true;
                                    i = j;
                                }
                            }
                            continue;
                        }
                        // A conditional, a callout, or a group this reader does not know.
                        _ => {
                            unsure = true;
                            i += 2;
                            Group::Assertion
                        }
                    }
                } else {
                    // `(*FAIL)` and the other callouts by name.
                    unsure |= chars.get(i + 1) == Some(&'*');
                    i += 1;
                    Group::Item
                };
                groups.push((start, kind, outer_x));
                continue;
            }
            ')' => {
                i += 1;
                match groups.pop() {
                    Some((open, kind, outer_x)) => {
                        x_on = outer_x;
                        atom = (kind == Group::Item).then_some((open, i, false));
                    }
                    None => {
                        unsure = true;
                        atom = None;
                    }
                }
                continue;
            }
            '|' | '^' | '$' => {
                atom = None;
                i += 1;
                continue;
            }
            _ => i += 1,
        }
        atom = Some((start, i, simple));
    }
    if edits.is_empty() {
        return None;
    }
    edits.sort_by_key(|&(start, end, _)| (start, end));
    let mut written = String::with_capacity(pattern.len() + 8 * edits.len());
    let mut at = 0;
    for (start, end, text) in edits {
        written.extend(&chars[at..start]);
        written.push_str(&text);
        at = end;
    }
    written.extend(&chars[at..]);
    Some(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quantifier_after_a_quantifier_is_written_as_oniguruma_reads_it_7682() {
        let mut wrong = Vec::new();
        for (pattern, extended, want) in [
            // `??` then `++`: `ReduceTypeTable` makes `a??+` the lazy `a*?`, which `+` makes atomic.
            ("(a)??++", false, Some("(?>(a)*?)")),
            ("a??+", false, Some("a*?")),
            ("a*?+b", false, Some("a*?b")),
            // An interval on a lazy open interval is not reduced: a plain group.
            ("b|a{2,}?{2}", false, Some("b|(?:a{2,}?){2}")),
            ("a?*", false, None),
            ("a??*", false, Some("a*?")),
            ("a*??", false, Some("a*?")),
            ("a* ??", true, None),
            ("a*? ??", true, Some("a*?")),
            ("a+?*c", false, Some("a*c")),
            // Fixed counts multiply; `{1}` is nothing.
            ("a{2}{3}", false, None),
            ("a{2}?{3}", false, Some("a{6}")),
            ("(a|b){2}+{3}", false, Some("(?>(a|b){2}){3}")),
            ("a{1}*", false, None),
            ("a{1}?*", false, None),
            ("a{2}?++", false, Some("(?:a{2})++")),
            ("a*+{1}?", false, None),
            ("(a)*?{1}+", false, None),
            // `(?:a*){n,m}` keeps its least count.
            ("a*{2,3}b", false, None),
            ("a*+*{2,3}b", false, Some("(?:(?>a*)*){2}b")),
            ("a*?{2,3}b", false, Some("(?:a*?){2,3}b")),
            // A possessive quantifier is an atomic group, but a greedy one that ends the run
            // stays possessive text for the scan in `eval.rs`.
            ("a*+*b", false, Some("(?>a*)*b")),
            ("a?*+b", false, None),
            ("a?+?*+b", false, Some("(?>a?)*+b")),
            ("(a?){2}++(?=a)", false, Some("(?>(a?){2})+(?=a)")),
            // A fixed count of an atom that matches in one way is the same possessive or not.
            ("a{2}++", false, Some("(?:a{2})+")),
            ("a{2}+?", false, Some("(?:a{2})?")),
            ("a*{2}+", false, Some("(?>(?:a*){2})")),
            ("a{2}{3}+", false, None),
            ("(a)+{0}", false, Some("(a){0}")),
            ("(a)+?{0}*", false, Some("(?:(a){0})*")),
            ("(a){0}", false, None),
            ("(a|b){2}++", false, Some("(?>(a|b){2})+")),
            // A possessive fixed count of an item that can match in several ways.
            ("(?:a|){2}+(?=a)", false, Some("(?>(?:a|){2})(?=a)")),
            ("a{2}+b", false, None),
            // A gap under `x` or a comment ends a quantifier.
            ("a{1,2} ?", true, None),
            ("a{1,2}? ?", true, Some("(?:a{1,2}?)?")),
            ("a{1,2}?(?#c)?", false, Some("(?:a{1,2}?)?")),
            ("(?x)(a)?+ +", false, Some("(?x)(?>(a)?)+")),
            ("a{1,2} ?", false, None),
            // Inside a group, and on the last character of a string.
            ("(ab??+)+", false, Some("(ab*?)+")),
            // A run the reader cannot place leaves the pattern alone.
            ("\\Qab\\E*+*", false, None),
            ("\\Oa**", false, None),
            ("a+", false, None),
            ("a*b+c?", false, None),
        ] {
            let got = rewrite(pattern, extended);
            if got.as_deref() != want {
                wrong.push((pattern, got, want));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }
}
