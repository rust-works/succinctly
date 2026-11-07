//! What Oniguruma's `tune_look_behind` (`regcomp.c`, 6.9.8, the library jq 1.7.1 links) makes of a
//! look-behind's body, read off the pattern (#6519).
//!
//! The function sizes the body with `node_char_len` and then decides:
//!
//! - **A body that can match the empty string is no look-behind at all.** When the least length is
//!   `0` and `min_is_sure` holds, `(?<=` becomes an empty node and `(?<!` a failure. The body is
//!   never run, so its groups are never set: `"ab" | match("(?<=(a)|)b")` captures
//!   `{offset: -1, string: null}`. A capture group clears `min_is_sure` (`can't optimize look-behind
//!   if capture exists`), and so does an assertion; an alternation keeps the flag of the branch that
//!   sets its least length, or of any branch that ties it.
//! - **Any other body must be of one length** (`ONIG_SYNTAX_PERL_NG` allows neither alternatives of
//!   different lengths nor a variable length): `invalid pattern in look-behind`. `(?<=(a)?)b` is
//!   one such, because the group is `(0, 1)` long and not sure to be empty.
//!
//! Before it sizes the body, `alt_reduce_in_look_behind` cuts the count of a quantifier on a plain
//! item to its least (`x+` is `x`, `x?` and `x*` are removed), but only a quantifier on a string, a
//! class or a character type, and only the first item of a branch's concatenation that is not itself
//! removed. A branch whose every item is removed returns `1` from the reduction, which
//! `tune_look_behind` hands back as its error code: `undefined error code` (`(?<=a*b*)b`).
//!
//! The reader keeps to what it can size. A pattern with a case-insensitive flag (the length of a
//! folded string is another matter), a lookaround, an atomic group or a backreference inside the
//! body, an escape the `regex-syntax` shadow reads differently from the pattern, or an unbounded
//! greedy repeat of a plain item that a later item could make possessive (`a*b` is `(?>a*)b`, which
//! the reduction no longer sees) is left to the callers' other rules.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use regex_syntax::ast::{self, Ast, FlagsItemKind, GroupKind, RepetitionKind, RepetitionRange};

use super::eval::{oniguruma_shadow, OnigurumaShadow};

/// An unbounded length or count.
const INF: u64 = u64::MAX;

/// Oniguruma's `LOOK_BEHIND_MAX_CHAR_LEN`.
const MAX_LEN: u64 = 65535;

/// What `tune_look_behind` does with a look-behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// The body can match nothing and is sure to: the look-behind is an empty node (a failure for
    /// `(?<!`) and its groups are never set.
    Empty,
    /// The body has one length: an ordinary look-behind.
    Fixed,
    /// `invalid pattern in look-behind`.
    Invalid,
    /// `undefined error code`: the reduction removed every item of a branch.
    Undefined,
}

/// A look-behind of a pattern that this reader sized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tuned {
    /// The offset of its `(`.
    pub(crate) open: usize,
    /// The offset of the first byte of its body.
    pub(crate) body_at: usize,
    /// The offset of its closing `)`.
    pub(crate) close: usize,
    /// Whether it is `(?<!`.
    pub(crate) negative: bool,
    /// Whether the body holds a group that captures, as Oniguruma counts them.
    pub(crate) captures: bool,
    /// What `tune_look_behind` makes of it.
    pub(crate) verdict: Verdict,
}

/// The Oniguruma node of an item, as far as its length goes.
#[derive(Debug, Clone)]
enum Node {
    /// A string of this many characters (`0` for the empty string).
    Str(String),
    /// A class or a character type (one character), or a dot (`dot`).
    Class {
        dot: bool,
    },
    /// An assertion: no length, and not sure.
    Anchor,
    /// A group that captures.
    Capture(Box<Self>),
    /// A quantified item.
    Quant {
        body: Box<Self>,
        lower: u64,
        upper: u64,
        greedy: bool,
        /// Whether the item after it makes it possessive (`a*b` is `(?>a*)b`), which the reduction
        /// then no longer sees.
        possessive: Possessive,
    },
    List(Vec<Self>),
    Alt(Vec<Self>),
}

/// Whether `tune_next` makes a quantifier possessive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Possessive {
    No,
    Yes,
    /// It depends on what a class holds, which this does not read: both are tried.
    Maybe,
}

/// `(min, max, min_is_sure)`.
#[derive(Debug, Clone, Copy)]
struct Len {
    min: u64,
    max: u64,
    sure: bool,
}

impl Len {
    fn set(len: u64) -> Self {
        Self {
            min: len,
            max: len,
            sure: true,
        }
    }
}

fn add(a: u64, b: u64) -> u64 {
    if a == INF || b == INF {
        INF
    } else {
        a.saturating_add(b).min(INF - 1)
    }
}

fn multiply(a: u64, count: u64) -> u64 {
    if a == 0 || count == 0 {
        0
    } else if a == INF || count == INF {
        INF
    } else {
        a.saturating_mul(count).min(INF - 1)
    }
}

/// `node_char_len1`.
fn char_len(node: &Node) -> Len {
    match node {
        Node::Str(text) => Len::set(text.len() as u64),
        Node::Class { .. } => Len::set(1),
        Node::Anchor => Len {
            min: 0,
            max: 0,
            sure: false,
        },
        Node::Capture(body) => Len {
            sure: false,
            ..char_len(body)
        },
        Node::Quant {
            body, lower, upper, ..
        } => {
            if lower == upper {
                if *upper == 0 {
                    Len::set(0)
                } else {
                    let inner = char_len(body);
                    Len {
                        min: multiply(inner.min, *lower),
                        max: multiply(inner.max, *lower),
                        sure: inner.sure,
                    }
                }
            } else {
                let inner = char_len(body);
                Len {
                    min: multiply(inner.min, *lower),
                    max: if *upper == INF {
                        INF
                    } else {
                        multiply(inner.max, *upper)
                    },
                    sure: inner.sure,
                }
            }
        }
        Node::List(items) => {
            let mut total = char_len(&items[0]);
            for item in &items[1..] {
                let next = char_len(item);
                total = Len {
                    min: add(total.min, next.min),
                    max: add(total.max, next.max),
                    sure: total.sure && next.sure,
                };
            }
            total
        }
        Node::Alt(branches) => {
            let mut merged = char_len(&branches[0]);
            for branch in &branches[1..] {
                let alt = char_len(branch);
                if merged.min > alt.min {
                    merged.min = alt.min;
                    merged.sure = alt.sure;
                } else if merged.min == alt.min && alt.sure {
                    merged.sure = true;
                }
                merged.max = merged.max.max(alt.max);
            }
            merged
        }
    }
}

/// `node_reduce_in_look_behind`: `1` when the item is removed.
fn reduce_item(node: &mut Node, assume: bool) -> u8 {
    if let Node::Quant {
        body,
        lower,
        upper,
        possessive,
        ..
    } = node
    {
        let wrapped = match possessive {
            Possessive::No => false,
            Possessive::Yes => true,
            Possessive::Maybe => assume,
        };
        if matches!(**body, Node::Str(_) | Node::Class { .. }) && !wrapped {
            *upper = *lower;
            if *upper == 0 {
                return 1;
            }
        }
    }
    0
}

/// `list_reduce_in_look_behind`.
fn reduce_list(node: &mut Node, possessive: bool) -> u8 {
    match node {
        Node::Quant { .. } => {
            reduce_item(node, possessive);
            0
        }
        Node::List(items) => {
            let mut r = 0;
            for item in items.iter_mut() {
                r = reduce_item(item, possessive);
                if r == 0 {
                    break;
                }
            }
            r
        }
        _ => 0,
    }
}

/// `alt_reduce_in_look_behind`.
fn reduce(body: &mut Node, possessive: bool) -> u8 {
    match body {
        Node::Alt(branches) => {
            let mut r = 0;
            for branch in branches.iter_mut() {
                r = reduce_list(branch, possessive);
                if r != 0 {
                    break;
                }
            }
            r
        }
        other => reduce_list(other, possessive),
    }
}

/// Whether the node has a capture group, as Oniguruma counts them.
fn holds_capture(node: &Node) -> bool {
    match node {
        Node::Capture(_) => true,
        Node::Quant { body, .. } => holds_capture(body),
        Node::List(items) | Node::Alt(items) => items.iter().any(holds_capture),
        _ => false,
    }
}

/// What `tune_look_behind` does with a body, when the reading of the possessive-repeat question
/// (see [`Node::Quant`]) does not change it.
fn verdict_of(body: &Node) -> Option<Verdict> {
    let read = |possessive: bool| {
        let mut body = body.clone();
        if reduce(&mut body, possessive) != 0 {
            return Verdict::Undefined;
        }
        let len = char_len(&body);
        if (len.max != INF && len.max > MAX_LEN) || len.min > MAX_LEN {
            Verdict::Invalid
        } else if len.min == 0 && len.sure {
            Verdict::Empty
        } else if len.min == len.max {
            Verdict::Fixed
        } else {
            Verdict::Invalid
        }
    };
    let plain = read(false);
    (plain == read(true)).then_some(plain)
}

/// `is_invalid_quantifier_target`: an assertion, or an alternation with a branch that is one.
fn invalid_target(node: &Node) -> bool {
    match node {
        Node::Anchor => true,
        Node::Alt(branches) => branches.iter().any(invalid_target),
        _ => false,
    }
}

/// `quantifier_type_num`: `?`, `*`, `+` then their lazy forms, `None` for any other count.
fn popular(lower: u64, upper: u64, greedy: bool) -> Option<usize> {
    let kind = match (lower, upper) {
        (0, 1) => 0,
        (0, INF) => 1,
        (1, INF) => 2,
        _ => return None,
    };
    Some(if greedy { kind } else { kind + 3 })
}

/// A quantifier on `body`, as the parser and `tune_quant` leave it.
fn quantify(lower: u64, upper: u64, greedy: bool, body: Node) -> Node {
    let plain = |lower, upper, greedy, body| Node::Quant {
        body: Box::new(body),
        lower,
        upper,
        greedy,
        possessive: Possessive::No,
    };
    match body {
        // `onig_reduce_nested_quantifier`.
        Node::Quant {
            body: inner,
            lower: inner_lower,
            upper: inner_upper,
            greedy: inner_greedy,
            ..
        } => {
            match (
                popular(lower, upper, greedy),
                popular(inner_lower, inner_upper, inner_greedy),
            ) {
                (Some(outer), Some(target)) => {
                    // `ReduceTypeTable[target][outer]`: a letter is the quantifier that replaces
                    // both, `d` the inner one alone, `s` both as they are, `q` `(?:x+)??`.
                    const TABLE: [[char; 6]; 6] = [
                        ['d', 'a', 'a', 'Q', 'A', 's'],
                        ['d', 'd', 'd', 'q', 'q', 'd'],
                        ['a', 'a', 'd', 's', 'q', 'd'],
                        ['d', 'A', 'A', 'd', 'A', 'A'],
                        ['d', 'd', 'd', 'd', 'd', 'd'],
                        ['s', 'a', 'p', 'A', 'A', 'd'],
                    ];
                    match TABLE[target][outer] {
                        'd' => plain(inner_lower, inner_upper, inner_greedy, *inner),
                        'a' => plain(0, INF, true, *inner),
                        'p' => plain(1, INF, true, *inner),
                        'A' => plain(0, INF, false, *inner),
                        'Q' => plain(0, 1, false, *inner),
                        'q' => plain(0, 1, false, plain(1, INF, true, *inner)),
                        _ => plain(
                            lower,
                            upper,
                            greedy,
                            plain(inner_lower, inner_upper, inner_greedy, *inner),
                        ),
                    }
                }
                // Two exact counts multiply.
                _ if lower == upper && inner_lower == inner_upper => {
                    quantify(lower * inner_lower, lower * inner_lower, greedy, *inner)
                }
                _ => plain(
                    lower,
                    upper,
                    greedy,
                    plain(inner_lower, inner_upper, inner_greedy, *inner),
                ),
            }
        }
        // `tune_quant` writes an exact count of a string as the longer string, when it is at most
        // 100 bytes (the body is ASCII here, see `tune_shadow`).
        Node::Str(text)
            if lower == upper
                && (2..=100).contains(&lower)
                && (text.len() as u64).saturating_mul(lower) <= 100 =>
        {
            Node::Str(text.repeat(lower as usize))
        }
        body => plain(lower, upper, greedy, body),
    }
}

/// What `get_tree_head_literal` finds at the head of an item.
enum Head {
    /// Nothing that can be compared.
    None,
    /// A string.
    Text(String),
    /// A class or a character type.
    Class,
}

fn head_literal(node: &Node) -> Head {
    match node {
        Node::Str(text) if text.is_empty() => Head::None,
        Node::Str(text) => Head::Text(text.clone()),
        Node::Class { dot: true } => Head::None,
        Node::Class { dot: false } => Head::Class,
        Node::Anchor | Node::Alt(_) => Head::None,
        Node::Capture(body) => head_literal(body),
        Node::Quant { body, lower, .. } if *lower > 0 => head_literal(body),
        Node::Quant { .. } => Head::None,
        Node::List(items) => head_literal(&items[0]),
    }
}

/// The node of `ast`, or `None` for anything this reader cannot size. `jq` compiles with
/// `ONIG_OPTION_CAPTURE_GROUP`, so every group `(...)` captures, named or not.
fn build(ast: &Ast) -> Option<Node> {
    match ast {
        Ast::Empty(_) => Some(Node::Str(String::new())),
        Ast::Literal(literal) => Some(Node::Str(String::from(literal.c))),
        Ast::Dot(_) => Some(Node::Class { dot: true }),
        Ast::ClassUnicode(_) | Ast::ClassPerl(_) | Ast::ClassBracketed(_) => {
            Some(Node::Class { dot: false })
        }
        Ast::Assertion(_) => Some(Node::Anchor),
        Ast::Flags(_) => None,
        Ast::Group(group) => {
            let inner = build(&group.ast)?;
            match &group.kind {
                GroupKind::CaptureIndex(_) | GroupKind::CaptureName { .. } => {
                    Some(Node::Capture(Box::new(inner)))
                }
                GroupKind::NonCapturing(flags) if flags.items.is_empty() => Some(inner),
                GroupKind::NonCapturing(_) => None,
            }
        }
        Ast::Repetition(repetition) => {
            let (lower, upper) = match &repetition.op.kind {
                RepetitionKind::ZeroOrOne => (0, 1),
                RepetitionKind::ZeroOrMore => (0, INF),
                RepetitionKind::OneOrMore => (1, INF),
                RepetitionKind::Range(RepetitionRange::Exactly(n)) => {
                    (u64::from(*n), u64::from(*n))
                }
                RepetitionKind::Range(RepetitionRange::AtLeast(n)) => (u64::from(*n), INF),
                RepetitionKind::Range(RepetitionRange::Bounded(low, high)) => {
                    (u64::from(*low), u64::from(*high))
                }
            };
            let body = build(&repetition.ast)?;
            // `is_invalid_quantifier_target`: a quantified assertion is a parse error, which comes
            // before any of this; left to the other rules.
            if invalid_target(&body) {
                return None;
            }
            // `assign_quantifier_body`: a count of exactly one is no quantifier.
            if (lower, upper) == (1, 1) {
                return Some(body);
            }
            Some(quantify(lower, upper, repetition.greedy, body))
        }
        Ast::Concat(concat) => {
            let mut items: Vec<Node> = Vec::new();
            let mut after_literal = false;
            for item in &concat.asts {
                let node = build(item)?;
                let literal = matches!(item, Ast::Literal(_));
                // Adjacent literal characters are one string node.
                if literal && after_literal {
                    if let (Some(Node::Str(text)), Node::Str(more)) = (items.last_mut(), &node) {
                        text.push_str(more);
                        continue;
                    }
                }
                items.push(node);
                after_literal = literal;
            }
            // `tune_next`: a greedy unbounded quantifier on a string, class or character type is made
            // possessive when the item after it starts with something it excludes.
            for at in 1..items.len() {
                let after = head_literal(&items[at]);
                if let Node::Quant {
                    body,
                    lower,
                    upper,
                    greedy,
                    possessive,
                } = &mut items[at - 1]
                {
                    if !(*greedy && *upper == INF && *lower <= 1) {
                        continue;
                    }
                    let before = match &**body {
                        Node::Str(text) if !text.is_empty() => Head::Text(text.clone()),
                        // A dot has no head literal to compare.
                        Node::Class { dot: true } => {
                            *possessive = Possessive::No;
                            continue;
                        }
                        Node::Class { dot: false } => Head::Class,
                        _ => continue,
                    };
                    *possessive = match (&before, &after) {
                        (_, Head::None) => Possessive::No,
                        (Head::Text(x), Head::Text(y)) => {
                            let common = x.len().min(y.len());
                            if x.as_bytes()[..common] != y.as_bytes()[..common] {
                                Possessive::Yes
                            } else {
                                Possessive::No
                            }
                        }
                        _ => Possessive::Maybe,
                    };
                }
            }
            if items.len() == 1 {
                items.pop()
            } else {
                Some(Node::List(items))
            }
        }
        Ast::Alternation(alternation) => alternation
            .asts
            .iter()
            .map(build)
            .collect::<Option<Vec<_>>>()
            .map(Node::Alt),
    }
}

/// Whether the tree sets a case-insensitive flag anywhere outside the shadow's own look-behind
/// markers (`(?i:`, at the offsets in `skip`).
fn folds_case(ast: &Ast, skip: &[usize]) -> bool {
    let sets_i = |flags: &ast::Flags| {
        let mut negated = false;
        flags.items.iter().any(|item| match item.kind {
            FlagsItemKind::Negation => {
                negated = true;
                false
            }
            FlagsItemKind::Flag(ast::Flag::CaseInsensitive) => !negated,
            FlagsItemKind::Flag(_) => false,
        })
    };
    match ast {
        Ast::Flags(set) => sets_i(&set.flags),
        Ast::Group(group) => {
            let own = match &group.kind {
                GroupKind::NonCapturing(flags) => {
                    !skip.contains(&group.span.start.offset) && sets_i(flags)
                }
                _ => false,
            };
            own || folds_case(&group.ast, skip)
        }
        Ast::Repetition(repetition) => folds_case(&repetition.ast, skip),
        Ast::Concat(concat) => concat.asts.iter().any(|item| folds_case(item, skip)),
        Ast::Alternation(alternation) => alternation.asts.iter().any(|item| folds_case(item, skip)),
        _ => false,
    }
}

/// The outermost look-behinds of the tree, in order, as the groups the shadow wrote for them.
fn collect<'t>(ast: &'t Ast, behind: &[usize], found: &mut Vec<&'t ast::Group>) {
    match ast {
        Ast::Group(group) => {
            if behind.contains(&group.span.start.offset) {
                found.push(group);
            } else {
                collect(&group.ast, behind, found);
            }
        }
        Ast::Repetition(repetition) => collect(&repetition.ast, behind, found),
        Ast::Concat(concat) => {
            for item in &concat.asts {
                collect(item, behind, found);
            }
        }
        Ast::Alternation(alternation) => {
            for item in &alternation.asts {
                collect(item, behind, found);
            }
        }
        _ => {}
    }
}

/// The look-behinds of `pattern` this reader can size, in order: the outermost ones, and only those
/// whose body it follows (see the module documentation). `ignore_case` is the `i` flag of the
/// match, which the pattern text does not carry.
pub(crate) fn tune(pattern: &str, ignore_case: bool) -> Vec<Tuned> {
    if ignore_case || !pattern.contains("(?<") {
        return Vec::new();
    }
    let Some(shadow) = oniguruma_shadow(pattern) else {
        return Vec::new();
    };
    tune_shadow(pattern, &shadow)
}

fn tune_shadow(pattern: &str, shadow: &OnigurumaShadow) -> Vec<Tuned> {
    let Ok(tree) = ast::parse::Parser::new().parse(&shadow.text) else {
        return Vec::new();
    };
    let is_behind = |at: usize| {
        pattern
            .get(at..)
            .is_some_and(|rest| rest.starts_with("(?<=") || rest.starts_with("(?<!"))
    };
    let behind: Vec<usize> = shadow
        .opaque
        .iter()
        .copied()
        .filter(|&at| is_behind(at))
        .collect();
    if folds_case(&tree, &behind) {
        return Vec::new();
    }
    let mut groups = Vec::new();
    collect(&tree, &behind, &mut groups);
    let mut out = Vec::new();
    for group in groups {
        let open = group.span.start.offset;
        let close = group.span.end.offset.saturating_sub(1);
        let body_at = open + 4;
        if body_at > close || pattern.get(close..=close) != Some(")") {
            continue;
        }
        // A backreference, lookaround, atomic group or comment inside the body is opaque; an escape
        // the shadow writes as another (`\K`, `\G`, `\Z`, `\R`) makes the text differ.
        // The string expansion above counts bytes, which are characters in ASCII.
        if shadow.opaque.iter().any(|&at| at > open && at < close)
            || pattern.get(body_at..close) != shadow.text.get(body_at..close)
            || !pattern[body_at..close].is_ascii()
        {
            continue;
        }
        let Some(node) = build(&group.ast) else {
            continue;
        };
        let Some(verdict) = verdict_of(&node) else {
            continue;
        };
        let negative = pattern[open..].starts_with("(?<!");
        let captures = holds_capture(&node);
        out.push(Tuned {
            open,
            body_at,
            close,
            negative,
            captures,
            // `check_node_in_look_behind` comes first: a negative look-behind allows no group that
            // captures (#6197), whatever the lengths.
            verdict: if negative && captures {
                Verdict::Invalid
            } else {
                verdict
            },
        });
    }
    out
}

/// The pattern with each look-behind whose body can match nothing written as the node Oniguruma
/// makes of it: nothing for `(?<=`, a class that matches nothing for `(?<!`, and, where the body
/// holds a group, that group kept (behind a class that matches nothing, optional for `(?<=`) so that
/// it is never set and the groups keep their numbers. The engines do not agree with Oniguruma on a
/// body that can be empty (`"a" | test("(?<!(?:a|\\b[ab])?)")`). `None` when there is none.
pub(crate) fn rewrite_empty(pattern: &str, ignore_case: bool) -> Option<String> {
    let found: Vec<Tuned> = tune(pattern, ignore_case)
        .into_iter()
        .filter(|tuned| tuned.verdict == Verdict::Empty)
        .collect();
    if found.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(pattern.len() + found.len() * 16);
    let mut from = 0;
    for tuned in &found {
        out.push_str(&pattern[from..tuned.open]);
        match (tuned.negative, tuned.captures) {
            (false, false) => out.push_str("(?:)"),
            (true, false) => out.push_str(r"[^\s\S]"),
            (negative, true) => {
                out.push_str(r"(?:[^\s\S](?:");
                out.push_str(&pattern[tuned.body_at..tuned.close]);
                out.push_str(if negative { "))" } else { "))?" });
            }
        }
        from = tuned.close + 1;
    }
    out.push_str(&pattern[from..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdicts(pattern: &str) -> Vec<Verdict> {
        tune(pattern, false)
            .into_iter()
            .map(|tuned| tuned.verdict)
            .collect()
    }

    #[test]
    fn a_body_that_can_match_nothing_is_empty_unless_a_group_or_assertion_is_in_the_way() {
        for pattern in [
            "(?<=a|)b",
            "(?<=(a)|)b",
            "(?<=(?:(a)|))b",
            "(?<=a*)b",
            "(?<=a?)b",
            "(?<=b|c{0})b",
            "(?<=(?:a|bc)?)b",
            "(?<=(?:a|bc){0,2})b",
            "(?<=(?:a|bc)*)b",
            "(?<=(a)|b*)b",
            "(?<=)b",
        ] {
            assert_eq!(verdicts(pattern), [Verdict::Empty], "{pattern}");
        }
    }

    #[test]
    fn a_body_of_one_length_is_fixed() {
        for pattern in [
            "(?<=a)b",
            "(?<=(a))b",
            "(?<=ab|cd)b",
            "(?<=a|b)b",
            "(?<=a+)b",
            "(?<=a{1,3})b",
            "(?<=b?b)b",
            "(?<=a*a)b",
            "(?<=a*.)b",
            "(?<=(?:ab)*ab)c",
            "(?<=(?:ab)*abd)c",
            r"(?<=\b)b",
            "(?<=x{2}|yz)b",
        ] {
            assert_eq!(verdicts(pattern), [Verdict::Fixed], "{pattern}");
        }
    }

    #[test]
    fn a_body_of_several_lengths_is_invalid() {
        for pattern in [
            "(?<=(a)?)b",
            "(?<=(a)*)b",
            "(?<=(a)+)b",
            "(?<=b|()|c)b",
            "(?<=bb?)b",
            "(?<=a|bc)b",
            "(?<=a*b)c",
            "(?<=a*bc)d",
            "(?<=a+b+)c",
            "(?<=(?:ab)*ac)c",
            "(?<=(?:a|b)+)b",
            "(?<=(?:ab|cd)+)b",
            "(?<=(?:a+|bc)+)b",
            "(?<=\\ba?)b",
            "(?<=(a|b)+)b",
            "(?<=a+b+)c",
            "(?<=x(a)?)b",
            "(?<!(a)|)b",
            "(?<!(a)?)b",
        ] {
            assert_eq!(verdicts(pattern), [Verdict::Invalid], "{pattern}");
        }
    }

    #[test]
    fn a_branch_whose_every_item_is_removed_is_an_undefined_error() {
        for pattern in [
            "(?<=a*b*)b",
            "(?<=a?b?)b",
            "(?<=a|b*c*)b",
            "(?<=(?:a*)(?:b*))b",
        ] {
            assert_eq!(verdicts(pattern), [Verdict::Undefined], "{pattern}");
        }
    }

    #[test]
    fn a_pattern_it_cannot_size_is_left_alone() {
        for pattern in [
            // A possessive repeat that depends on a class: `\w*\d` is `(?>\w*)\d` or not.
            r"(?<=\w*\d)c",
            r"(?<=a*[bc])d",
            // Case folding, a backreference, a nested lookaround, an atomic group, `\K`.
            "(?i)(?<=a|bc)d",
            r"(?<=(a)\1)b",
            "(?<=(?<=a)b)c",
            "(?<=(?>a|bc))d",
            r"(?<=a\K)b",
            // A flag group, a pattern the parser refuses.
            "(?<=(?s:a)|bc)d",
            "(?<=a|bc",
        ] {
            assert_eq!(verdicts(pattern), [], "{pattern}");
        }
        assert_eq!(tune("(?<=a|bc)d", true), []);
    }

    #[test]
    fn an_empty_body_is_written_as_the_node_oniguruma_makes_of_it() {
        assert_eq!(
            rewrite_empty("x(?<=(a)|)b", false).as_deref(),
            Some(r"x(?:[^\s\S](?:(a)|))?b")
        );
        assert_eq!(
            rewrite_empty("(?<=a|)b", false).as_deref(),
            Some("(?:)b"),
            "no group to keep"
        );
        assert_eq!(
            rewrite_empty("x(?<!a?)b", false).as_deref(),
            Some(r"x[^\s\S]b")
        );
        assert_eq!(
            rewrite_empty("(?<!(a)|)c", false),
            None,
            "a group in a negative look-behind is refused, not rewritten"
        );
        assert_eq!(
            rewrite_empty("(?<=(a))b", false),
            None,
            "a body of one length"
        );
        assert_eq!(
            rewrite_empty("(?<=a|bc)b", false),
            None,
            "a body of several lengths"
        );
    }

    #[test]
    fn an_exact_count_of_a_string_is_the_longer_string() {
        for pattern in [
            "(?<=(?:c{2}){1,2})c",
            "(?<=(?:(?:c{2}){1}){1,2})c",
            "(?<=(?:(?:a){2})+)b",
            "(?<!(?:b{2}){1,2})c",
        ] {
            assert_eq!(verdicts(pattern), [Verdict::Fixed], "{pattern}");
        }
        // Not a string, or a count a bound apart.
        assert_eq!(verdicts("(?<=(?:[ab]{2}){1,2})c"), [Verdict::Invalid]);
        assert_eq!(verdicts("(?<=(?:c{2,3}){1,2})c"), [Verdict::Invalid]);
    }
}
