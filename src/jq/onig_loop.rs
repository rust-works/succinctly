//! Oniguruma's exit from a loop at an iteration that matches nothing, written into a jq pattern
//! for the `regex` crate (#6106).
//!
//! Oniguruma tries a greedy, unbounded repeat's body one way at a time, and when the way it is
//! trying matches nothing, its empty-iteration check ends the loop there: the rest of the pattern
//! runs, and only when that fails are the body's later ways tried. The `regex` crate never makes
//! an empty iteration (the thread that would comes back to a loop state it has already visited at
//! that position, and is dropped), so it tries the later ways first and leaves the loop only after
//! all of them: `"ba" | match("(?:b*|a)+")` is `"b"` in jq and was `"ba"` here.
//!
//! The ways a body `X` can match, in the order a backtracking engine tries them, are those that
//! consume text before its first empty way (`A`), that empty way, and the consuming ways after it
//! (`B`; a later empty way only repeats an exit that has already failed). Oniguruma's `X*` is then
//! `L = A L | (exit) | B L`, which is `A*(?:B A*)*?` in a pattern no iteration of which can be
//! empty. The empty way ends the loop even before it has run its minimum count, so `X+` and
//! `X{n,}` are `L` too, and a bounded `X{m,n}` is `n` nested copies that each end at the empty way
//! instead of going on to the next copy. [`rewrite`] writes that form for every such loop with a
//! `B`, which is where the two engines can disagree; without one, they agree.
//!
//! A capture group in a way after the empty one is written once, in `B`, and the loop is rewritten
//! as any other is (#6310). Any other position needs a copy of the group (the ways before the empty
//! one are written twice) or the group's empty way, which sets it when the loop ends (#6069):
//! [`rewrite_with_groups`] runs over the pattern with its groups tagged by number, writes the loop
//! with every copy, ends it with `E` (the body's empty way, from [`Rewriter::empty_text`]), and lists
//! the copies each group has, which the evaluator reads back as the last one the match set (#6332).
//! The plain [`rewrite`] keeps a loop as written when it would copy or drop a group, and counts the
//! groups it writes ([`Rewriter::has_groups`]). A body with one way that matches nothing only where
//! an anchor holds (`\b`, `\bb?`) has a conditional exit, on both sides of the ways after it, which is
//! written from what the anchor is ([`Rewriter::anchored_repeat`], #6358). A body whose ways are not
//! listed here keeps its loop as written too: one with two such ways, an empty way after one, an
//! assertion that is not an anchor, or a flag switch that names `x` or `U`. An assertion beside an item that consumes text
//! does not matter: a way that must consume text always does, and the assertion only decides whether
//! it matches, so `\Ba` and `a\z` are consuming ways and are copied with the rest (#6300). Any other
//! flag switch reaches the rest of its group, so under a greedy loop it is written as a flag group
//! over what it reaches before the ways are listed (#6301); a lazy repeat over such a body keeps its
//! loop. A loop over a capture group is still reached: the capture rewrite (`onig_capture_loop`)
//! writes the repeated group's own loop as a plain `(?:X)*`, and the evaluator runs this rewrite
//! again over what it wrote (#6305).
//!
//! A pattern with syntax only `fancy-regex` reads (a lookaround, an atomic group, a backreference,
//! `\K`, `\G`, a comment) is parsed through `oniguruma_shadow`'s copy of it, which writes that syntax
//! plainly at the same offsets, and every piece's text is still taken from the pattern (#6128).
//! Those constructs are opaque: a body holding one keeps its loop, and a lookaround or atomic
//! group has its own body rewritten. A lazy repeat that must run twice or more is written copy by
//! copy by `Rewriter::lazy_counted`; for an unbounded one over a body with a consuming way before
//! its empty one that form still differs from Oniguruma's on a few patterns (#6128).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use regex_syntax::ast::{self, Ast, Flag, GroupKind, RepetitionKind, RepetitionRange};

/// How a piece of pattern text may be joined to others without a group around it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prec {
    /// An alternation: needs a group to be concatenated or repeated.
    Alt,
    /// A concatenation or a repeat: needs a group to be repeated.
    Concat,
    /// One item that a quantifier may follow.
    Atom,
}

/// A piece of pattern text.
#[derive(Clone)]
struct Text {
    text: String,
    prec: Prec,
}

/// The ways a piece can match, in the order they are tried.
#[derive(Clone)]
enum Ways {
    /// Every way consumes text.
    Consuming,
    /// Some way matches nothing. `before` holds the consuming ways tried before the first such
    /// way and `after` those tried after it, each `None` when there are none; both consume text.
    Nullable {
        before: Option<Text>,
        after: Option<Text>,
    },
    /// One empty way that passes an anchor, so it matches nothing only where the anchor holds
    /// (#6358): `A | c | B`, with `before` and `after` the consuming ways around it, each `None`
    /// when there are none. Read as [`Ways::Opaque`] anywhere but directly under a repeat
    /// ([`Rewriter::anchored_repeat`]).
    Anchored {
        before: Option<Text>,
        /// Boxed: a [`Piece`] stands in every frame of the recursive [`Rewriter::convert`], which
        /// a pattern nested to the parser's limit takes through a 2 MB stack.
        anchor: Box<Anchor>,
        after: Option<Text>,
    },
    /// Not listed, but its text holds only zero-width assertions and what consumes text around
    /// them: it may be copied, and a consuming item beside it makes the whole consume (`\Ba` and
    /// `a\z` are consuming ways, #6300), but whether it matches nothing depends on where it stands.
    Opaque,
    /// Not listed: a capture group whose empty way comes first or that holds a way that is not
    /// listed (#6310), a flag switch, or something holding one.
    Unknown,
}

/// An anchor that is a way of its own: `\b` where it stands, and what tells a loop how the anchor
/// may be written as a condition on the way out of it ([`Rewriter::anchored_loop`]).
#[derive(Clone)]
struct Anchor {
    text: Text,
    kind: AnchorKind,
}

#[derive(Clone)]
enum AnchorKind {
    /// `\b` or `\B`: `fails` holds exactly where the anchor does not.
    Word { fails: Text },
    /// `\A` or `^`: it holds where the text starts, so at most at a loop's first head.
    Start,
    /// `\z` or `$`: it holds where the text ends, so no way that consumes text can run there.
    End,
}

#[derive(Clone)]
struct Piece {
    text: Text,
    ways: Ways,
}

/// Why the rewrite was abandoned; the pattern is then used as written.
struct GiveUp;

struct Rewriter<'p> {
    /// The pattern as written. Every piece's text is taken from it, by the offsets of the parse.
    source: &'p str,
    /// Where each construct the parsed copy of `source` reads as something it is not starts
    /// ([`oniguruma_shadow`](super::eval::oniguruma_shadow)): it is copied as written, and its ways
    /// are not listed.
    opaque: &'p [usize],
    /// The longest text a piece may grow to: the rewrite copies a loop's body, and nested loops
    /// copy the copies.
    limit: usize,
    /// Whether a flag group of the pattern may switch case folding on, which Oniguruma reads a
    /// letter under as more than one operation (a multi-character fold such as `ss`).
    folds: bool,
    /// Whether a flag of the pattern may switch multi-line mode on, under which `^` and `$` are not
    /// the anchors [`AnchorKind`] says they are (#6358).
    multi_line: bool,
    /// Whether the parse is of a copy of the pattern that writes Oniguruma syntax plainly
    /// ([`oniguruma_shadow`](super::eval::oniguruma_shadow)): what a node there is no longer says
    /// how many operations Oniguruma makes of it.
    shadowed: bool,
    /// The bytes of text built so far, and their bound: a body of many items that can match
    /// nothing rebuilds its listed ways at each item, quadratic in the items.
    work: core::cell::Cell<usize>,
    work_limit: usize,
    changed: bool,
    /// Whether the pattern has a capture group at all (#6310): a rewrite that copies text must then
    /// leave the groups as they were, in number and order. Never set when `copies` is.
    captures: bool,
    /// Whether the groups of the pattern are tagged and a rewrite may copy them (#6332): each
    /// copy is read back into its group ([`rewrite_with_groups`]).
    copies: bool,
    /// The empty way of the loop body being converted, set for the one `repeat` call that follows
    /// when `copies` and the body has a capture group (#6332): see [`Rewriter::empty_text`].
    exit: Option<Text>,
    /// How many enclosing repeats of the part being converted are lazy and keep their text as
    /// written (#6358): a lazy `X*?` or `X+?` is not rewritten, and a loop with groups inside one,
    /// written with its exit, is not read the way Oniguruma's is, so that loop keeps its own text.
    lazy_around: u32,
}

/// Whether a flag switch (`(?i)`, not a flag group) stands anywhere in `ast`.
fn has_switch(ast: &Ast) -> bool {
    match ast {
        Ast::Flags(_) => true,
        Ast::Group(group) => has_switch(&group.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(has_switch),
        Ast::Concat(concat) => concat.asts.iter().any(has_switch),
        Ast::Repetition(rep) => has_switch(&rep.ast),
        _ => false,
    }
}

/// Whether a capture group stands anywhere in `ast`.
fn has_capture(ast: &Ast) -> bool {
    let mut found = Vec::new();
    collect_captures(ast, &mut found);
    !found.is_empty()
}

/// The capture groups of `ast` in the order they open, each as its name (`None` for an unnamed one).
fn collect_captures(ast: &Ast, out: &mut Vec<Option<String>>) {
    match ast {
        Ast::Group(group) => {
            match &group.kind {
                GroupKind::CaptureIndex(_) => out.push(None),
                GroupKind::CaptureName { name, .. } => out.push(Some(name.name.clone())),
                GroupKind::NonCapturing(_) => {}
            }
            collect_captures(&group.ast, out);
        }
        Ast::Alternation(alt) => alt.asts.iter().for_each(|a| collect_captures(a, out)),
        Ast::Concat(concat) => concat.asts.iter().for_each(|a| collect_captures(a, out)),
        Ast::Repetition(rep) => collect_captures(&rep.ast, out),
        _ => {}
    }
}

/// Whether an assertion or a flag switch stands anywhere in `ast`.
fn has_assertion_or_flags(ast: &Ast) -> bool {
    match ast {
        Ast::Assertion(_) | Ast::Flags(_) => true,
        Ast::Group(group) => has_assertion_or_flags(&group.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(has_assertion_or_flags),
        Ast::Concat(concat) => concat.asts.iter().any(has_assertion_or_flags),
        Ast::Repetition(rep) => has_assertion_or_flags(&rep.ast),
        _ => false,
    }
}

/// A piece that matches only the empty string and has no text of its own.
fn empty_piece() -> Piece {
    Piece {
        text: Text {
            text: String::new(),
            prec: Prec::Concat,
        },
        ways: Ways::Nullable {
            before: None,
            after: None,
        },
    }
}

/// `pattern` with every greedy, unbounded loop whose body can match nothing before trying a way
/// that consumes text written so that the `regex` crate leaves the loop where Oniguruma does, or
/// `None` when no loop needs it (or the pattern does not parse, or the rewrite grows too large).
pub(crate) fn rewrite(pattern: &str) -> Option<String> {
    if !may_need_rewrite(pattern) {
        return None;
    }
    rewrite_parsed(pattern, false, "")
}

/// A pattern [`rewrite_with_groups`] wrote, with how its groups read back into the input's.
pub(crate) struct GroupRewrite {
    /// The pattern to compile; every group in it is unnamed.
    pub(crate) pattern: String,
    /// For each group of the input pattern (0 is the whole match), the groups of
    /// [`pattern`](Self::pattern) that hold a copy of it, in the order they open: the loop rewrite
    /// writes the consuming ways before the empty one twice, and a counted repeat writes its body
    /// again for each count. A group's value is the last of its copies that the match set.
    pub(crate) copies: Vec<Vec<usize>>,
    /// How many groups [`pattern`](Self::pattern) has, the whole match included.
    pub(crate) compiled_groups: usize,
    /// The input pattern's group names, by group (0 is the whole match).
    pub(crate) names: Vec<Option<Box<str>>>,
}

/// `pattern` parsed by `regex-syntax`, which refuses a group name used twice where Oniguruma allows
/// it (#6430). Each later use of a name is renamed in the returned text (`n` becomes `n__d1`), and
/// the second element of the result lists the renames as (new, original), so a caller that reads
/// names out of the tree reads them back through [`original_name`]. The text is `pattern` itself
/// when no name repeats. `None` for a pattern the parser refuses for any other reason.
pub(crate) fn parse_distinct_names(pattern: &str) -> Option<DistinctNames> {
    let mut text = pattern.to_string();
    let mut renames: Vec<(String, String)> = Vec::new();
    // Each pass renames one use; a pattern with more repeated uses than this keeps its text.
    // One parse more than renames, so that the last rename is read.
    for _ in 0..=MAX_NAME_RENAMES {
        match ast::parse::Parser::new().parse(&text) {
            Ok(tree) => {
                return Some(DistinctNames {
                    text,
                    ast: tree,
                    renames,
                })
            }
            Err(error) if matches!(error.kind(), ast::ErrorKind::GroupNameDuplicate { .. }) => {
                let span = error.span();
                let (start, end) = (span.start.offset, span.end.offset);
                let original = text.get(start..end)?.to_string();
                let mut count = 1;
                let renamed = loop {
                    let candidate = format!("{original}__d{count}");
                    if !text.contains(&candidate) {
                        break candidate;
                    }
                    count += 1;
                };
                text.replace_range(start..end, &renamed);
                renames.push((renamed, original));
            }
            Err(_) => return None,
        }
    }
    None
}

/// What [`parse_distinct_names`] read.
pub(crate) struct DistinctNames {
    /// The pattern, with each repeated use of a group name renamed.
    pub(crate) text: String,
    pub(crate) ast: Ast,
    /// The renames, as (new name, name as written).
    pub(crate) renames: Vec<(String, String)>,
}

/// How many repeated uses of a group name [`parse_distinct_names`] renames before it gives up.
const MAX_NAME_RENAMES: usize = 64;

/// The name a group carried in the pattern as written: `name` itself, or the one
/// [`parse_distinct_names`] renamed it from.
pub(crate) fn original_name(name: Box<str>, renames: &[(String, String)]) -> Box<str> {
    match renames.iter().find(|(renamed, _)| **renamed == *name) {
        Some((_, original)) => original.as_str().into(),
        None => name,
    }
}

/// [`rewrite`] for a pattern whose loops have a capture group where the plain rewrite would copy or
/// drop it (#6332): the groups are tagged by number, the loops are written as they are without
/// one, and each group the rewrite copied is read back through [`GroupRewrite::copies`]. `None`
/// when the pattern has no group, needs no rewrite, or is not one the `regex` crate's parser reads.
pub(crate) fn rewrite_with_groups(input: &str) -> Option<GroupRewrite> {
    // The flags `i` and `s` of jq put a switch in front of the pattern. It reaches all of it, so it
    // stays in front and the rest is rewritten, with case folding read from it.
    let lead_len = leading_switches(input)?;
    let (lead, pattern) = input.split_at(lead_len);
    if !may_need_rewrite(pattern) || !pattern.contains('(') || pattern.contains(TAG) {
        return None;
    }
    let DistinctNames {
        text: distinct,
        ast,
        renames,
    } = parse_distinct_names(pattern)?;
    let pattern = distinct.as_str();
    let mut openers = Vec::new();
    capture_openers(&ast, &mut openers);
    if openers.is_empty() {
        return None;
    }
    let mut tagged = String::with_capacity(pattern.len() + 12 * openers.len());
    let mut at = 0;
    for (n, (start, end)) in openers.iter().enumerate() {
        tagged.push_str(&pattern[at..*start]);
        tagged.push_str(&format!("(?<{TAG}{}>", n + 1));
        at = *end;
    }
    tagged.push_str(&pattern[at..]);
    let written = rewrite_parsed(&tagged, true, lead)?;
    // Read the tags back: each group of the result is a copy of the group its tag names. The copies
    // share a name, which the parser refuses, so the tags are found by their text.
    let opening = format!("(?<{TAG}");
    let mut copies = vec![Vec::new(); openers.len() + 1];
    copies[0].push(0);
    let mut pattern = String::with_capacity(lead.len() + written.len());
    pattern.push_str(lead);
    let mut rest = written.as_str();
    let mut compiled = 1;
    while let Some(at) = rest.find(&opening) {
        let tail = &rest[at + opening.len()..];
        let digits = tail.find('>')?;
        let group: usize = tail[..digits].parse().ok()?;
        copies.get_mut(group)?.push(compiled);
        compiled += 1;
        pattern.push_str(&rest[..at]);
        pattern.push('(');
        rest = &tail[digits + 1..];
    }
    pattern.push_str(rest);
    // A group the rewrite left out would be read as unset, which is not what Oniguruma leaves, and
    // the count of the groups the result has must be the count of tags found.
    let mut found = Vec::new();
    capture_openers(&ast::parse::Parser::new().parse(&pattern).ok()?, &mut found);
    (!copies.iter().any(Vec::is_empty) && found.len() + 1 == compiled).then_some(())?;
    let mut names = Vec::new();
    capture_names(&ast, &mut names);
    names.insert(0, None);
    let names = names
        .into_iter()
        .map(|name| name.map(|name| original_name(name, &renames)))
        .collect();
    Some(GroupRewrite {
        pattern,
        copies,
        compiled_groups: compiled,
        names,
    })
}

/// The length of the flag switches (`(?i)`, `(?s)`, `(?is)`) the pattern starts with, or `None`
/// when one names `x` or `U`, which change what the rest of the text means.
fn leading_switches(pattern: &str) -> Option<usize> {
    let mut at = 0;
    while pattern[at..].starts_with("(?") {
        let Some(close) = pattern[at..].find(')') else {
            break;
        };
        let flags = &pattern[at + 2..at + close];
        if flags.is_empty() || !flags.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
            break;
        }
        if flags.contains(['x', 'U']) {
            return None;
        }
        at += close + 1;
    }
    Some(at)
}

/// The names of the capture groups of `ast` in the order they open (`None` for an unnamed one).
fn capture_names(ast: &Ast, out: &mut Vec<Option<Box<str>>>) {
    match ast {
        Ast::Group(group) => {
            match &group.kind {
                GroupKind::CaptureIndex(_) => out.push(None),
                GroupKind::CaptureName { name, .. } => out.push(Some(name.name.as_str().into())),
                GroupKind::NonCapturing(_) => {}
            }
            capture_names(&group.ast, out);
        }
        Ast::Alternation(alt) => alt.asts.iter().for_each(|a| capture_names(a, out)),
        Ast::Concat(concat) => concat.asts.iter().for_each(|a| capture_names(a, out)),
        Ast::Repetition(rep) => capture_names(&rep.ast, out),
        _ => {}
    }
}

/// A class that matches no character: `E`'s stand-in for every character, class and `.` of the body.
const NEVER: &str = r"[^\s\S]";

/// The name prefix that tags a group of the input pattern in [`rewrite_with_groups`].
const TAG: &str = "__succinctly_g";

/// The opening of every capture group of `ast` (from the `(` to where its body starts), in the order
/// the groups open.
fn capture_openers(ast: &Ast, out: &mut Vec<(usize, usize)>) {
    match ast {
        Ast::Group(group) => {
            if !matches!(group.kind, GroupKind::NonCapturing(_)) {
                out.push((group.span.start.offset, group.ast.span().start.offset));
            }
            capture_openers(&group.ast, out);
        }
        Ast::Alternation(alt) => alt.asts.iter().for_each(|a| capture_openers(a, out)),
        Ast::Concat(concat) => concat.asts.iter().for_each(|a| capture_openers(a, out)),
        Ast::Repetition(rep) => capture_openers(&rep.ast, out),
        _ => {}
    }
}

fn rewrite_parsed(pattern: &str, copies: bool, lead: &str) -> Option<String> {
    // #6128: a pattern with Oniguruma syntax only `fancy-regex` reads (lookaround, a backreference,
    // `\K`) is parsed through a copy that writes that syntax plainly at the same offsets. A pattern
    // the parser reads as written is read as written.
    let shadow;
    let (parsed, ast, opaque) = match ast::parse::Parser::new().parse(pattern) {
        Ok(ast) => (pattern, ast, &[][..]),
        Err(_) => {
            shadow = super::eval::oniguruma_shadow(pattern)?;
            let ast = ast::parse::Parser::new().parse(&shadow.text).ok()?;
            (shadow.text.as_str(), ast, shadow.opaque.as_slice())
        }
    };
    let mut rewriter = Rewriter {
        source: pattern,
        opaque,
        limit: pattern
            .len()
            .saturating_mul(16)
            .saturating_add(1024)
            .max(32 * 1024),
        folds: may_fold_case(pattern) || may_fold_case(lead),
        multi_line: super::eval::pattern_may_enable_multiline(pattern)
            || super::eval::pattern_may_enable_multiline(lead),
        shadowed: parsed != pattern,
        work: core::cell::Cell::new(0),
        work_limit: pattern
            .len()
            .saturating_mul(64)
            .saturating_add(64 * 1024)
            .max(512 * 1024),
        changed: false,
        captures: !copies && has_capture(&ast),
        copies,
        exit: None,
        lazy_around: 0,
    };
    let piece = rewriter.convert(&ast, false).ok()?;
    // The rewrite can drop a piece that matches only the empty string (`\p{Foo}{0}`), and with it
    // an error the crate would have reported for the pattern as written.
    let valid = regex_syntax::hir::translate::Translator::new()
        .translate(parsed, &ast)
        .is_ok();
    (rewriter.changed && valid).then_some(piece.text.text)
}

/// Oniguruma's `QUANTIFIER_EXPAND_LIMIT_SIZE`: a counted repeat is written out as copies of its
/// body when the body's compiled length times the count is at most this.
const ONIG_EXPAND_LIMIT: usize = 10;

/// Whether a flag group of `pattern` may turn case folding on: any `(?` followed by flag letters
/// that include an `i`. Over-reports (a `-i` that turns it off counts), which only keeps a rewrite.
fn may_fold_case(pattern: &str) -> bool {
    pattern.match_indices("(?").any(|(at, _)| {
        pattern[at + 2..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic() || *c == '-')
            .any(|c| c == 'i')
    })
}

/// Whether `ast` can match the empty string, over-reporting (an assertion or flag counts).
fn may_be_empty(ast: &Ast) -> bool {
    match ast {
        Ast::Empty(_) | Ast::Assertion(_) | Ast::Flags(_) => true,
        Ast::Literal(_)
        | Ast::Dot(_)
        | Ast::ClassUnicode(_)
        | Ast::ClassPerl(_)
        | Ast::ClassBracketed(_) => false,
        Ast::Group(group) => may_be_empty(&group.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(may_be_empty),
        Ast::Concat(concat) => concat.asts.iter().all(may_be_empty),
        Ast::Repetition(rep) => bounds(&rep.op.kind).0 == 0 || may_be_empty(&rep.ast),
    }
}

/// The least and most times a repeat operator repeats its item.
fn bounds(kind: &RepetitionKind) -> (u32, Option<u32>) {
    match kind {
        RepetitionKind::ZeroOrOne => (0, Some(1)),
        RepetitionKind::ZeroOrMore => (0, None),
        RepetitionKind::OneOrMore => (1, None),
        RepetitionKind::Range(RepetitionRange::Exactly(n)) => (*n, Some(*n)),
        RepetitionKind::Range(RepetitionRange::AtLeast(n)) => (*n, None),
        RepetitionKind::Range(RepetitionRange::Bounded(m, n)) => (*m, Some(*n)),
    }
}

/// Whether case folding is on inside an option group whose flags are `flags`, in a pattern where it
/// is `folds` outside (#6241); `None` for a flag that changes what the body reads in a way the count
/// does not follow (`x` and `U` change what the text means, `u` is not an Oniguruma option).
fn group_folds(flags: &ast::Flags, folds: bool) -> Option<bool> {
    let mut on = true;
    let mut folds = folds;
    for item in &flags.items {
        match &item.kind {
            ast::FlagsItemKind::Negation => on = false,
            ast::FlagsItemKind::Flag(Flag::CaseInsensitive) => folds = on,
            ast::FlagsItemKind::Flag(Flag::MultiLine | Flag::DotMatchesNewLine) => {}
            ast::FlagsItemKind::Flag(_) => return None,
        }
    }
    Some(folds)
}

/// Whether `class` is the bracketed class the translation writes for jq's `\w` or `\W` (and
/// `\p{Word}`), outside a class (#6263).
fn is_word_class(class: &ast::ClassBracketed) -> bool {
    let mut text = String::new();
    if ast::print::Printer::new()
        .print(&Ast::ClassBracketed(Box::new(class.clone())), &mut text)
        .is_err()
    {
        return false;
    }
    super::eval::is_translated_word_class(&text)
}

/// The number of operations Oniguruma 6.9.8 compiles `ast` to (`compile_length_tree`), or `None`
/// for a construct whose length is not worked out here: a capture group, an assertion, a flag
/// switch, a letter under case folding, and a repeat of a repeat (Oniguruma reduces those).
/// Each of the others is one operation: a run of literals (one string), a character class, `.`.
/// An alternation adds a push and a jump per way but the last, `x?` a push, `x*` a push and a
/// jump, and an `x` that may match nothing two operations for its empty check.
fn onig_ops(ast: &Ast, folds: bool) -> Option<usize> {
    match ast {
        Ast::Empty(_) => Some(0),
        // Under case folding a letter that starts a multi-character fold (`ss`, `fi`, `ʼn`) is
        // more than one operation, and so is a class holding one.
        Ast::Literal(lit) => (!(folds
            && (!lit.c.is_ascii() || "sfiltjnhwy".contains(lit.c.to_ascii_lowercase()))))
        .then_some(1),
        // #6263: jq's `\w`, `\W` and `\p{Word}` are one operation in Oniguruma, and the translation
        // writes each as the bracketed word class (`super::eval::JQ_WORD_CLASS`), which is not a class
        // that may hold a multi-character fold.
        Ast::ClassBracketed(class) if folds && is_word_class(class) => Some(1),
        Ast::ClassUnicode(_) | Ast::ClassBracketed(_) if folds => None,
        Ast::Dot(_) | Ast::ClassUnicode(_) | Ast::ClassPerl(_) | Ast::ClassBracketed(_) => Some(1),
        // #6358: each anchor is one operation.
        Ast::Assertion(_) => Some(1),
        Ast::Flags(_) => None,
        // #6241: an option group is its body (`(?s:\w)` counts as `\w` in jq, which also decides
        // whether a repeat of it is written out as copies), with the case folding the group sets.
        Ast::Group(group) => match &group.kind {
            GroupKind::NonCapturing(flags) => onig_ops(&group.ast, group_folds(flags, folds)?),
            // #6332: a capture group is its body between the two operations that start and end it.
            GroupKind::CaptureIndex(_) | GroupKind::CaptureName { .. } => {
                onig_ops(&group.ast, folds).map(|ops| ops + 2)
            }
        },
        Ast::Alternation(alt) => {
            let mut total = 2 * alt.asts.len().saturating_sub(1);
            for way in &alt.asts {
                total += onig_ops(way, folds)?;
            }
            Some(total)
        }
        Ast::Concat(concat) => {
            let mut total = 0;
            let mut in_run = false;
            for item in &concat.asts {
                // Adjacent literals are one string.
                let literal = matches!(item, Ast::Literal(_));
                if literal && in_run && folds {
                    return None;
                }
                if !(literal && in_run) {
                    total += onig_ops(item, folds)?;
                }
                in_run = literal;
            }
            Some(total)
        }
        Ast::Repetition(rep) => {
            let mut inner = &rep.ast;
            while let Ast::Group(group) = &**inner {
                match &group.kind {
                    // #6241: an option group is looked through as a plain one is.
                    GroupKind::NonCapturing(flags) if group_folds(flags, folds).is_some() => {
                        inner = &group.ast;
                    }
                    _ => return None,
                }
            }
            if matches!(**inner, Ast::Repetition(_)) {
                return None;
            }
            let tlen = onig_ops(&rep.ast, folds)?;
            if tlen == 0 {
                return Some(0);
            }
            let (min, max) = bounds(&rep.op.kind);
            let (min, max) = (min as usize, max.map(|n| n as usize));
            let mod_tlen = tlen + if may_be_empty(&rep.ast) { 2 } else { 0 };
            Some(match max {
                None if min <= 1 || tlen.checked_mul(min)? <= ONIG_EXPAND_LIMIT => {
                    let head = if min == 1 && tlen > ONIG_EXPAND_LIMIT {
                        1
                    } else {
                        tlen.saturating_mul(min)
                    };
                    head.saturating_add(2 + mod_tlen)
                }
                Some(0) => 0,
                Some(max)
                    if rep.greedy
                        && (max == 1 || (tlen + 1).checked_mul(max)? <= ONIG_EXPAND_LIMIT) =>
                {
                    tlen * min + (1 + tlen).saturating_mul(max - min)
                }
                Some(1) if min == 0 => 2 + tlen,
                _ => 2 + mod_tlen,
            })
        }
    }
}

/// Whether `pattern` may hold a loop [`rewrite`] changes, judged from its bytes so that every other
/// pattern is spared the parse, which cost a fresh compile of `(?:a|b)+` 19% on a 7950X. Such a loop
/// is a group under a quantifier (a `)` followed by `*`, `+` or `{`). Its body can match nothing
/// only through a `*`, a `?` or `{0`/`{,` quantifier, an empty alternative or an empty group, and
/// has a consuming way after the empty one only through a `|` or a lazy quantifier, or a way that is
/// only an anchor (#6358) does both. Whitespace
/// between tokens is skipped, and a `#` (a comment under `x`, which can stand between them) counts
/// for all three.
fn may_need_rewrite(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let next = |i: usize| {
        bytes[i + 1..]
            .iter()
            .find(|c| !c.is_ascii_whitespace())
            .copied()
    };
    let (mut group_repeat, mut nullable, mut later_way) = (false, false, false);
    let mut prev = None;
    // Where the run of anchors that starts a way ends, when the bytes so far are one.
    let mut anchors_end = usize::MAX;
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b')' => {
                group_repeat |= matches!(next(i), Some(b'*' | b'+' | b'{'));
                nullable |= matches!(prev, Some(b'(' | b':'));
            }
            b'*' => nullable = true,
            b'?' => {
                nullable |= prev != Some(b'(');
                later_way |= matches!(prev, Some(b'*' | b'+' | b'?' | b'}'));
            }
            b'{' => nullable |= matches!(next(i), Some(b'0' | b',')),
            b'|' => {
                later_way = true;
                nullable |= matches!(prev, None | Some(b'(' | b'|' | b':'))
                    || matches!(next(i), None | Some(b')' | b'|'));
            }
            b'#' => (group_repeat, nullable, later_way) = (true, true, true),
            // #6358: a way that is only anchors. Both ends of it are asked for, so that the `^` and
            // `$` around an ordinary pattern (`^(?:a|b)+$`) are spared the parse.
            b'^' | b'$' | b'b' | b'B' | b'A' | b'z' => {
                let escaped = matches!(c, b'b' | b'B' | b'A' | b'z');
                let starts = if !escaped {
                    i == 0 || anchors_end == i || matches!(bytes[i - 1], b'(' | b'|' | b':')
                } else {
                    i >= 1
                        && bytes[i - 1] == b'\\'
                        && (i == 1
                            || anchors_end == i - 1
                            || matches!(bytes[i - 2], b'(' | b'|' | b':'))
                };
                if starts {
                    anchors_end = i + 1;
                    nullable |= matches!(next(i), None | Some(b')' | b'|'));
                }
            }
            _ => {}
        }
        if !c.is_ascii_whitespace() {
            prev = Some(c);
        }
    }
    group_repeat && nullable && later_way
}

impl Rewriter<'_> {
    fn text(&self, text: String, prec: Prec) -> Result<Text, GiveUp> {
        let work = self.work.get() + text.len();
        self.work.set(work);
        if text.len() > self.limit || work > self.work_limit {
            return Err(GiveUp);
        }
        Ok(Text { text, prec })
    }

    fn verbatim(&self, span: &ast::Span) -> &str {
        &self.source[span.start.offset..span.end.offset]
    }

    /// `ast` as a piece. Its ways are listed only when `listed`, that is under a repeat, which is
    /// the only place they are read.
    fn convert(&mut self, ast: &Ast, listed: bool) -> Result<Piece, GiveUp> {
        if matches!(ast, Ast::Group(_) | Ast::Literal(_))
            && self.opaque.binary_search(&ast.span().start.offset).is_ok()
        {
            return self.opaque(ast);
        }
        match ast {
            Ast::Empty(_) => Ok(Piece {
                text: Text {
                    text: String::new(),
                    prec: Prec::Concat,
                },
                ways: Ways::Nullable {
                    before: None,
                    after: None,
                },
            }),
            Ast::Literal(_)
            | Ast::Dot(_)
            | Ast::ClassUnicode(_)
            | Ast::ClassPerl(_)
            | Ast::ClassBracketed(_) => Ok(Piece {
                text: self.text(self.verbatim(ast.span()).into(), Prec::Atom)?,
                ways: Ways::Consuming,
            }),
            Ast::Assertion(_) => self.assertion(ast),
            Ast::Flags(set) => {
                // `U` swaps what `*` and `*?` mean, which the greedy/lazy reading below does not
                // follow, so a pattern with it is left as written.
                if set.flags.flag_state(Flag::SwapGreed).is_some() {
                    return Err(GiveUp);
                }
                Ok(Piece {
                    text: self.text(self.verbatim(&set.span).into(), Prec::Concat)?,
                    ways: Ways::Unknown,
                })
            }
            Ast::Group(group) => self.group(group, listed),
            Ast::Repetition(rep) => {
                let (min, max) = bounds(&rep.op.kind);
                let kept = !rep.greedy && min < 2 && !matches!(max, Some(0 | 1));
                self.lazy_around += u32::from(kept);
                let body = self.convert(&rep.ast, true);
                self.lazy_around -= u32::from(kept);
                let mut body = body?;
                // #6301: the scoped switch is verified for the greedy loop. A lazy repeat decides
                // whether Oniguruma unrolls it from the body's operation count (`onig_ops`), which does
                // not read a switch, so it keeps the switch as the unknown it was.
                if !rep.greedy && has_switch(&rep.ast) {
                    body.ways = Ways::Unknown;
                }
                if let Ways::Anchored {
                    before,
                    anchor,
                    after,
                } = &body.ways
                {
                    let ways = (before, &**anchor, after);
                    return self.anchored_repeat(&rep.ast, &body.text, ways, min, max, rep.greedy);
                }
                let grouped = (self.copies || self.captures) && has_capture(&rep.ast);
                if !rep.greedy && max.is_none() && self.oniguruma_unrolls(&rep.ast, min) {
                    return self.lazy_unrolled(body, min, grouped);
                }
                if self.copies && grouped {
                    self.exit = Some(self.empty_text(&rep.ast)?);
                }
                self.repeat(body, min, max, rep.greedy)
            }
            Ast::Alternation(alt) => {
                // #6301: a flag switch (`(?i)`) reaches the rest of its group, later ways included,
                // which the ways below would move out of its reach. It is written as a flag group
                // over what it reaches: `x(?i)y|z` is `x(?i:y)|(?i:z)`.
                let mut active: Vec<String> = Vec::new();
                let mut items = Vec::with_capacity(alt.asts.len());
                for way in &alt.asts {
                    // A way that is only a switch matches nothing, as an empty way does.
                    let piece = match way {
                        Ast::Flags(set) if self.switch_open(set)?.is_some() => empty_piece(),
                        _ => self.convert(way, listed)?,
                    };
                    items.push(self.scoped(piece, &active)?);
                    let switches: &[Ast] = match way {
                        Ast::Concat(concat) => &concat.asts,
                        other => core::slice::from_ref(other),
                    };
                    for item in switches {
                        if let Ast::Flags(set) = item {
                            active.extend(self.switch_open(set)?);
                        }
                    }
                }
                self.alt(&items, listed)
            }
            Ast::Concat(concat) => self.convert_concat(&concat.asts, listed),
        }
    }

    /// An assertion, which is a way of its own when it is an anchor a loop can write (#6358).
    fn assertion(&self, ast: &Ast) -> Result<Piece, GiveUp> {
        let text = self.text(self.verbatim(ast.span()).into(), Prec::Atom)?;
        let word = |fails: &str| self.text(fails.into(), Prec::Atom);
        // The text decides, not the parse: the shadow writes `\K`, `\G` and `\Z` as `\b`.
        let kind = match text.text.as_str() {
            r"\b" => AnchorKind::Word {
                fails: word(r"\B")?,
            },
            r"\B" => AnchorKind::Word {
                fails: word(r"\b")?,
            },
            r"\A" => AnchorKind::Start,
            r"\z" => AnchorKind::End,
            // Under `(?m)` they hold at every line, so neither says what these two kinds say.
            "^" if !self.multi_line => AnchorKind::Start,
            "$" if !self.multi_line => AnchorKind::End,
            _ => {
                return Ok(Piece {
                    text,
                    ways: Ways::Opaque,
                })
            }
        };
        Ok(Piece {
            ways: Ways::Anchored {
                before: None,
                anchor: Box::new(Anchor {
                    text: text.clone(),
                    kind,
                }),
                after: None,
            },
            text,
        })
    }

    /// A concatenation. A flag switch in it reaches the items after it, so those are converted as
    /// one piece and wrapped in a flag group with the switch's flags (#6301).
    fn convert_concat(&mut self, asts: &[Ast], listed: bool) -> Result<Piece, GiveUp> {
        let mut items = Vec::with_capacity(asts.len());
        let mut empties = Vec::with_capacity(asts.len());
        for (at, ast) in asts.iter().enumerate() {
            if let Some(open) = match ast {
                Ast::Flags(set) => self.switch_open(set)?,
                _ => None,
            } {
                items.push(empty_piece());
                empties.push(None);
                let rest = &asts[at + 1..];
                if !rest.is_empty() {
                    let piece = self.convert_concat(rest, listed)?;
                    // #6358: with the groups tagged, what the switch reaches has an empty way of its
                    // own, the items' empty ways in the flag group the switch stands for.
                    let empty = self.scoped_empty_text(rest, &open)?;
                    items.push(self.scoped(piece, &[open])?);
                    empties.push(empty);
                }
                break;
            }
            items.push(self.convert(ast, listed)?);
            empties.push(self.empty_if_grouped(ast)?);
        }
        self.concat_with(&items, &empties, listed)
    }

    /// The empty way of the items `rest` that a switch reaches, in the flag group `open` that
    /// stands for it, when the groups are tagged and they hold a group (#6358).
    fn scoped_empty_text(&self, rest: &[Ast], open: &str) -> Result<Option<Text>, GiveUp> {
        if !self.copies || !rest.iter().any(has_capture) {
            return Ok(None);
        }
        let empties = rest
            .iter()
            .map(|ast| self.empty_text(ast))
            .collect::<Result<Vec<_>, _>>()?;
        let refs: Vec<&Text> = empties.iter().collect();
        let joined = self.concat_text(&refs)?;
        self.text(format!("{open}{})", joined.text), Prec::Atom)
            .map(Some)
    }

    /// The empty way of `ast` ([`Self::empty_text`]) when the groups are tagged and it holds one.
    fn empty_if_grouped(&self, ast: &Ast) -> Result<Option<Text>, GiveUp> {
        if self.copies && has_capture(ast) {
            self.empty_text(ast).map(Some)
        } else {
            Ok(None)
        }
    }

    /// The opening of the flag group that stands for the switch `set` over what it reaches
    /// (`(?i)` is `(?i:`..`)`), or `None` for a switch that cannot be scoped: `x` makes the text
    /// after it blanks and comments, which a closing parenthesis would join to the last comment.
    /// `U` swaps greedy and lazy, which the rewrite does not follow, so a pattern with it is left
    /// as written.
    fn switch_open(&self, set: &ast::SetFlags) -> Result<Option<String>, GiveUp> {
        if set.flags.flag_state(Flag::SwapGreed).is_some() {
            return Err(GiveUp);
        }
        if set.flags.flag_state(Flag::IgnoreWhitespace) == Some(true) {
            return Ok(None);
        }
        Ok(Some(format!("(?{}:", self.verbatim(&set.flags.span))))
    }

    /// `piece` inside the flag groups `opens` (outermost first), each of its listed ways too.
    fn scoped(&self, mut piece: Piece, opens: &[String]) -> Result<Piece, GiveUp> {
        for open in opens.iter().rev() {
            piece = Piece {
                text: self.text(format!("{open}{})", piece.text.text), Prec::Atom)?,
                ways: self.ways_in(piece.ways, open, true)?,
            };
        }
        Ok(piece)
    }

    /// A construct the parsed copy reads as something it is not. A comment matches nothing, as an
    /// empty group does, unless a quantifier follows it: that quantifier is Oniguruma's to read
    /// (`(?#)?` is refused, `b(?#)*` is `b*`), and the copy gives it the comment's group to repeat,
    /// so such a comment's ways are not listed. A lookaround or atomic group keeps its opening and
    /// closing text and has its body rewritten, a backreference is copied as written, and neither
    /// has its ways listed.
    fn opaque(&mut self, ast: &Ast) -> Result<Piece, GiveUp> {
        let span = ast.span();
        let comment = self.verbatim(span).starts_with("(?#");
        let quantified = || {
            self.source[span.end.offset..]
                .bytes()
                .find(|c| !c.is_ascii_whitespace())
                .is_some_and(|c| matches!(c, b'*' | b'+' | b'?' | b'{'))
        };
        if comment && !quantified() {
            return Ok(Piece {
                text: self.text(self.verbatim(span).into(), Prec::Atom)?,
                ways: Ways::Nullable {
                    before: None,
                    after: None,
                },
            });
        }
        let text = match ast {
            Ast::Group(group) if !self.verbatim(span).starts_with("(?#") => {
                let body = self.convert(&group.ast, false)?;
                let inner = group.ast.span();
                let open = &self.source[span.start.offset..inner.start.offset];
                let close = &self.source[inner.end.offset..span.end.offset];
                format!("{open}{}{close}", body.text.text)
            }
            _ => self.verbatim(span).into(),
        };
        Ok(Piece {
            text: self.text(text, Prec::Atom)?,
            ways: Ways::Unknown,
        })
    }

    /// A group keeps its own opening and closing text. A capture group's ways are not listed; a
    /// non-capturing one's are its body's, each wrapped in the group's flags when it has any.
    fn group(&mut self, group: &ast::Group, listed: bool) -> Result<Piece, GiveUp> {
        let body = self.convert(&group.ast, listed)?;
        let inner = group.ast.span();
        let open = &self.source[group.span.start.offset..inner.start.offset];
        let close = &self.source[inner.end.offset..group.span.end.offset];
        let text = self.text(format!("{open}{}{close}", body.text.text), Prec::Atom)?;
        let ways = match &group.kind {
            // #6310: a capture group has its body's ways, inside the group. Its own empty way sets
            // the group to the empty string, and when that is the loop's first empty way it is the
            // iteration Oniguruma exits at (#6069), which a rewrite that leaves the way out of the
            // loop loses. A body whose empty way comes first is therefore not listed; with a
            // consuming way before it, the group is in `A`, which a rewrite refuses
            // ([`Rewriter::groups_only_in_later_ways`]), or stands in a later way, whose empty way
            // is an exit that has already failed.
            // #6332: with the groups tagged, a copy of one is read back into it, so a group in any
            // position is fine; the empty way the rewrite leaves out of the loop is written back at
            // its exit ([`Rewriter::empty_text`]).
            GroupKind::CaptureIndex(_) | GroupKind::CaptureName { .. } => match body.ways {
                Ways::Consuming => Ways::Consuming,
                // Only with the groups tagged (`self.copies`) may the group's own empty way come
                // first: it is written back at the exit. An anchor's iteration sets the group
                // there too (#6358), so the anchor itself stays out of the group.
                Ways::Nullable { before: None, .. } | Ways::Anchored { .. } if !self.copies => {
                    Ways::Unknown
                }
                ways @ (Ways::Nullable { .. } | Ways::Anchored { .. }) => {
                    self.ways_in(ways, open, false)?
                }
                Ways::Opaque | Ways::Unknown => Ways::Unknown,
            },
            GroupKind::NonCapturing(flags) if flags.items.is_empty() => body.ways,
            GroupKind::NonCapturing(flags) => {
                if flags.flag_state(Flag::SwapGreed).is_some() {
                    return Err(GiveUp);
                }
                self.ways_in(body.ways, open, true)?
            }
        };
        Ok(Piece { text, ways })
    }

    /// What the list `first second` of two anchors is, when it is still one a loop can write: where
    /// the text starts or ends it holds at most where that end's anchor does, whatever the other
    /// is, and `\b\b` is `\b`. `None` for a pair that holds nowhere or says nothing simple.
    fn both_anchors(first: &Anchor, second: &Anchor) -> Option<AnchorKind> {
        match (&first.kind, &second.kind) {
            (AnchorKind::Start, _) | (_, AnchorKind::Start) => Some(AnchorKind::Start),
            (AnchorKind::End, _) | (_, AnchorKind::End) => Some(AnchorKind::End),
            (AnchorKind::Word { fails }, AnchorKind::Word { .. })
                if first.text.text == second.text.text =>
            {
                Some(AnchorKind::Word {
                    fails: fails.clone(),
                })
            }
            _ => None,
        }
    }

    /// `ways` inside the group that opens with `open`, each way that consumes text; with `anchor`
    /// (a flag group, whose flags reach it: `(?-u:\b)` is not `\b`) the anchor and its complement too.
    fn ways_in(&self, ways: Ways, open: &str, anchor: bool) -> Result<Ways, GiveUp> {
        let wrap = |t: Text| self.text(format!("{open}{})", t.text), Prec::Atom);
        let way = |t: Option<Text>| t.map(wrap).transpose();
        Ok(match ways {
            Ways::Nullable { before, after } => Ways::Nullable {
                before: way(before)?,
                after: way(after)?,
            },
            Ways::Anchored {
                before,
                anchor: mark,
                after,
            } => Ways::Anchored {
                before: way(before)?,
                after: way(after)?,
                anchor: match (anchor, mark) {
                    (false, mark) => mark,
                    (true, mark) => Box::new(Anchor {
                        text: wrap(mark.text)?,
                        kind: match mark.kind {
                            AnchorKind::Word { fails } => AnchorKind::Word {
                                fails: wrap(fails)?,
                            },
                            kind => kind,
                        },
                    }),
                },
            },
            ways => ways,
        })
    }

    fn concat_text(&self, items: &[&Text]) -> Result<Text, GiveUp> {
        if let [one] = items {
            return Ok((*one).clone());
        }
        let mut text = String::new();
        for item in items {
            if item.prec == Prec::Alt {
                text.push_str("(?:");
                text.push_str(&item.text);
                text.push(')');
            } else {
                text.push_str(&item.text);
            }
        }
        self.text(text, Prec::Concat)
    }

    fn alt_text(&self, items: &[Text]) -> Result<Option<Text>, GiveUp> {
        match items {
            [] => Ok(None),
            [one] => Ok(Some(one.clone())),
            _ => {
                let text = items
                    .iter()
                    .map(|t| t.text.as_str())
                    .collect::<Vec<_>>()
                    .join("|");
                self.text(text, Prec::Alt).map(Some)
            }
        }
    }

    fn repeat_text(
        &self,
        body: &Text,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    ) -> Result<Text, GiveUp> {
        let op = match (min, max) {
            (0, None) => String::from("*"),
            (1, None) => String::from("+"),
            (0, Some(1)) => String::from("?"),
            (n, None) => format!("{{{n},}}"),
            (m, Some(n)) if m == n => format!("{{{n}}}"),
            (m, Some(n)) => format!("{{{m},{n}}}"),
        };
        let lazy = if greedy { "" } else { "?" };
        let text = if body.prec == Prec::Atom {
            format!("{}{op}{lazy}", body.text)
        } else {
            format!("(?:{}){op}{lazy}", body.text)
        };
        self.text(text, Prec::Concat)
    }

    fn concat(&self, items: &[Piece], listed: bool) -> Result<Piece, GiveUp> {
        self.concat_with(items, &[], listed)
    }

    /// [`Self::concat`] where `empties[i]` is the empty way of `items[i]` when that way sets a group
    /// (#6332; none past the end): `Q` from where `P` began is `P`'s empty way and then `Q`'s ways.
    fn concat_with(
        &self,
        items: &[Piece],
        empties: &[Option<Text>],
        listed: bool,
    ) -> Result<Piece, GiveUp> {
        let Some((first, rest)) = items.split_first() else {
            return Ok(Piece {
                text: Text {
                    text: String::new(),
                    prec: Prec::Concat,
                },
                ways: Ways::Nullable {
                    before: None,
                    after: None,
                },
            });
        };
        // Left to right: the prefix `P = A1 | ε | B1` followed by `Q = A2 | ε | B2` tries
        // `A1 Q`, then `Q` from where `P` began (`A2`, `ε`, `B2`), then `B1 Q`.
        let mut ways = if listed {
            first.ways.clone()
        } else {
            Ways::Unknown
        };
        let empty_of = |at: usize| empties.get(at).cloned().flatten();
        let mut gone: Option<Text> = empty_of(0);
        for (at, next) in rest.iter().enumerate() {
            let own = empty_of(at + 1);
            ways = match (ways, &next.ways) {
                (Ways::Unknown, _) | (_, Ways::Unknown) => Ways::Unknown,
                // #6310: `A1 Q` and `B1 Q` each hold `Q` again, and so each of its groups.
                (Ways::Nullable { before, after }, Ways::Nullable { .. })
                    if (before.is_some() || after.is_some()) && self.has_groups(&next.text) =>
                {
                    Ways::Unknown
                }
                // One item that consumes text makes the whole way consume, an assertion beside it
                // included: it only decides whether that way matches (#6300).
                (Ways::Consuming, _)
                | (Ways::Opaque | Ways::Nullable { .. } | Ways::Anchored { .. }, Ways::Consuming) => {
                    Ways::Consuming
                }
                // #6358: an anchor beside what may match nothing is the same anchor's way: `\bb*`
                // is `\bb+ | \b`, and `a*\z` is `a+\z | \z`. With a group already gone past, the
                // ways would have to set it after the anchor, so they are not listed.
                (
                    Ways::Anchored {
                        before: None,
                        anchor: first,
                        after: None,
                    },
                    Ways::Anchored {
                        before: None,
                        anchor: second,
                        after: None,
                    },
                ) => match Self::both_anchors(&first, second).filter(|_| gone.is_none()) {
                    Some(kind) => Ways::Anchored {
                        before: None,
                        anchor: Box::new(Anchor {
                            text: self.concat_text(&[&first.text, &second.text])?,
                            kind,
                        }),
                        after: None,
                    },
                    None => Ways::Opaque,
                },
                (
                    Ways::Anchored {
                        before: None,
                        anchor,
                        after: None,
                    },
                    Ways::Nullable {
                        before: a2,
                        after: b2,
                    },
                ) if gone.is_none() => {
                    let then = |way: &Option<Text>| {
                        way.as_ref()
                            .map(|way| self.concat_text(&[&anchor.text, way]))
                            .transpose()
                    };
                    Ways::Anchored {
                        before: then(a2)?,
                        after: then(b2)?,
                        anchor,
                    }
                }
                (
                    Ways::Nullable {
                        before: a1,
                        after: b1,
                    },
                    Ways::Anchored {
                        before: None,
                        anchor,
                        after: None,
                    },
                ) => {
                    let then = |way: Option<Text>| {
                        way.map(|way| self.concat_text(&[&way, &anchor.text]))
                            .transpose()
                    };
                    Ways::Anchored {
                        before: then(a1)?,
                        after: then(b1)?,
                        anchor: anchor.clone(),
                    }
                }
                (Ways::Opaque | Ways::Anchored { .. }, _)
                | (_, Ways::Opaque | Ways::Anchored { .. }) => Ways::Opaque,
                (
                    Ways::Nullable {
                        before: a1,
                        after: b1,
                    },
                    Ways::Nullable {
                        before: a2,
                        after: b2,
                    },
                ) => {
                    // `Q` after `P`'s empty way: it sets the groups that way holds.
                    let after_empty = |way: &Option<Text>| -> Result<Option<Text>, GiveUp> {
                        match (&gone, way) {
                            (Some(gone), Some(way)) => self.concat_text(&[gone, way]).map(Some),
                            (None, way) => Ok(way.clone()),
                            (Some(_), None) => Ok(None),
                        }
                    };
                    let mut before = Vec::new();
                    if let Some(a1) = a1 {
                        before.push(self.concat_text(&[&a1, &next.text])?);
                    }
                    before.extend(after_empty(a2)?);
                    let mut after = Vec::new();
                    after.extend(after_empty(b2)?);
                    if let Some(b1) = b1 {
                        after.push(self.concat_text(&[&b1, &next.text])?);
                    }
                    Ways::Nullable {
                        before: self.alt_text(&before)?,
                        after: self.alt_text(&after)?,
                    }
                }
            };
            gone = match (gone, own) {
                (Some(a), Some(b)) => Some(self.concat_text(&[&a, &b])?),
                (a, b) => a.or(b),
            };
        }
        let texts: Vec<&Text> = items.iter().map(|p| &p.text).collect();
        Ok(Piece {
            text: self.concat_text(&texts)?,
            ways,
        })
    }

    fn alt(&self, items: &[Piece], listed: bool) -> Result<Piece, GiveUp> {
        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut nullable = false;
        let mut unknown = !listed;
        let mut opaque = false;
        let mut anchored: Option<Box<Anchor>> = None;
        for item in items {
            if unknown {
                break;
            }
            match &item.ways {
                Ways::Unknown => unknown = true,
                Ways::Opaque => opaque = true,
                Ways::Consuming if nullable || anchored.is_some() => after.push(item.text.clone()),
                Ways::Consuming => before.push(item.text.clone()),
                // #6358: one anchor is an exit on either side of the ways after it. A second, or an
                // empty way after it, is another exit past those ways, which the forms of
                // [`Self::anchored_loop`] do not write.
                Ways::Anchored { .. } | Ways::Nullable { .. } if anchored.is_some() => {
                    opaque = true;
                }
                // Past an empty way that holds everywhere, an anchor's exit has already failed.
                Ways::Anchored {
                    before: a,
                    after: b,
                    ..
                } if nullable => {
                    after.extend(a.clone());
                    after.extend(b.clone());
                }
                Ways::Anchored {
                    before: a,
                    anchor,
                    after: b,
                } => {
                    before.extend(a.clone());
                    anchored = Some(anchor.clone());
                    after.extend(b.clone());
                }
                // A later empty way is an exit that the first one has already tried.
                Ways::Nullable {
                    before: a,
                    after: b,
                } => {
                    if nullable {
                        after.extend(a.clone());
                    } else {
                        before.extend(a.clone());
                        nullable = true;
                    }
                    after.extend(b.clone());
                }
            }
        }
        let ways = if unknown {
            Ways::Unknown
        } else if opaque {
            Ways::Opaque
        } else if let Some(anchor) = anchored {
            Ways::Anchored {
                before: self.alt_text(&before)?,
                anchor,
                after: self.alt_text(&after)?,
            }
        } else if nullable {
            Ways::Nullable {
                before: self.alt_text(&before)?,
                after: self.alt_text(&after)?,
            }
        } else {
            Ways::Consuming
        };
        let texts: Vec<Text> = items.iter().map(|p| p.text.clone()).collect();
        let text = self.alt_text(&texts)?.unwrap_or(Text {
            text: String::new(),
            prec: Prec::Concat,
        });
        Ok(Piece { text, ways })
    }

    fn repeat(
        &mut self,
        body: Piece,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    ) -> Result<Piece, GiveUp> {
        let exit = self.exit.take();
        let text = self.repeat_text(&body.text, min, max, greedy)?;
        let ways = match (&body.ways, min, max, greedy) {
            (Ways::Unknown, ..) => Ways::Unknown,
            (Ways::Opaque | Ways::Anchored { .. }, ..) => Ways::Opaque,
            // #6310: an arm below that writes the body again, with the groups it holds.
            (Ways::Nullable { .. }, 0 | 1, Some(1) | None, false)
                if self.has_groups(&body.text) =>
            {
                Ways::Unknown
            }
            (Ways::Nullable { after: None, .. }, _, Some(_), true)
                if self.has_groups(&body.text) =>
            {
                Ways::Unknown
            }
            (_, _, Some(0), _) => Ways::Nullable {
                before: None,
                after: None,
            },
            (Ways::Consuming, 1.., ..) => Ways::Consuming,
            (Ways::Consuming, 0, _, true) => Ways::Nullable {
                before: Some(self.repeat_text(&body.text, 1, max, true)?),
                after: None,
            },
            (Ways::Consuming, 0, _, false) => Ways::Nullable {
                before: None,
                after: Some(self.repeat_text(&body.text, 1, max, false)?),
            },
            (Ways::Nullable { .. }, _, Some(1), true) | (Ways::Nullable { .. }, 1, Some(1), _) => {
                body.ways.clone()
            }
            (Ways::Nullable { before, after }, 0, Some(1) | None, false) => {
                // Lazy: leave first, then each consuming way of the body (followed by the loop).
                let ways: Vec<Text> = before.iter().chain(after.iter()).cloned().collect();
                Ways::Nullable {
                    before: None,
                    after: match self.alt_text(&ways)? {
                        Some(once) if max.is_none() => Some(self.concat_text(&[&once, &text])?),
                        once => once,
                    },
                }
            }
            // `X+?` is `X X*?`, as Oniguruma tries it.
            (Ways::Nullable { .. }, 1, None, false) => {
                let rest = self.repeat(body.clone(), 0, None, false)?;
                self.concat_with(&[body, rest], &[exit], true)?.ways
            }
            (Ways::Nullable { before, after }, _, None, true) => {
                return self.exit_on_empty(before.clone(), after.clone(), text, exit);
            }
            (Ways::Nullable { before, after }, _, Some(n), true) => {
                return self.bounded(&body, before.clone(), after.clone(), n, text, exit);
            }
            (
                Ways::Nullable {
                    before,
                    after: Some(after),
                },
                ..,
            ) if !greedy && min >= 2 => {
                return self.lazy_counted(
                    before.clone(),
                    Some(after.clone()),
                    min,
                    max,
                    text,
                    exit,
                );
            }
            // A lazy repeat that runs once at most leaves where Oniguruma does (a differential over
            // 2,800 generated patterns found no difference with or without the rewrite), and a
            // body with no consuming way after its empty one is left as it is.
            _ => Ways::Unknown,
        };
        Ok(Piece { text, ways })
    }

    /// Oniguruma's greedy `X{min,}` for a body `X = A | ε | B`, which is `A*(?:B A*)*?` whatever
    /// `min` is: an empty iteration ends the loop even before it has run `min` times, and `X`'s
    /// own empty way is there to take. Written out only when `B` is not empty; with no `B`, the
    /// loop as written already leaves where Oniguruma's does, and keeps its text.
    fn exit_on_empty(
        &mut self,
        before: Option<Text>,
        after: Option<Text>,
        written: Text,
        exit: Option<Text>,
    ) -> Result<Piece, GiveUp> {
        let consuming = |text: Text| Piece {
            text,
            ways: Ways::Consuming,
        };
        let before_text = before.clone();
        let star = before
            .map(|a| self.repeat(consuming(a), 0, None, true))
            .transpose()?;
        let mut parts: Vec<Piece> = star.iter().cloned().collect();
        let Some(b) = after else {
            let mut piece = self.concat(&parts, true)?;
            piece.text = written;
            return Ok(piece);
        };
        let b_text = b.clone();
        let mut iteration = alloc::vec![consuming(b)];
        iteration.extend(star);
        let iteration = self.concat(&iteration, true)?;
        parts.push(self.repeat(iteration, 0, None, false)?);
        let piece = self.concat(&parts, true)?;
        // #6310: only a group in `B` survives the rewrite; with the groups tagged, every one does,
        // and each copy is read back (#6332), in a loop inside another repeat too (#6358).
        let kept = if self.copies {
            !self.exit_keeps_text(&exit)
        } else {
            self.groups_only_in_later_ways(&written, &before_text, &b_text)
        };
        if !kept {
            return Ok(Self::unlisted(written));
        }
        self.changed = true;
        // #6332: the empty way the loop leaves at, with the groups it sets, ends the rewritten loop.
        let piece = match &exit {
            Some(_) => {
                let leave = self.exit_piece(&exit)?;
                self.concat(&[piece, leave], true)?
            }
            None => piece,
        };
        Ok(Piece {
            text: self.text(format!("(?:{})", piece.text.text), Prec::Atom)?,
            ways: piece.ways,
        })
    }

    /// The capture groups of `text` in the order they open, each as its name, or `None` for a text
    /// the parser cannot read (syntax only `fancy-regex` reads). The parse is charged to the work
    /// budget, as building the text was.
    fn captures_of(&self, text: &str) -> Option<Vec<Option<String>>> {
        self.work.set(self.work.get().saturating_add(text.len()));
        let ast = ast::parse::Parser::new().parse(text).ok()?;
        let mut found = Vec::new();
        collect_captures(&ast, &mut found);
        Some(found)
    }

    /// Whether the rewrite `A*(?:B A*)*?` of the loop `written` over `A | ε | B` keeps its capture
    /// groups (#6310): `B` is written once, so it may hold them, and every one of them must be there,
    /// in the same order, under the same names. `A` is written twice and the empty way not at all, so
    /// neither may hold one. A group's own text is not compared (`(a*)` is `(a+)` in `B`), and a way
    /// that would copy a group is not listed ([`Self::has_groups`]), so a way of `B` holds each group
    /// at most once. A text the parser cannot read is not kept.
    fn groups_only_in_later_ways(
        &self,
        written: &Text,
        before: &Option<Text>,
        after: &Text,
    ) -> bool {
        if !self.captures {
            return true;
        }
        let none = !before.as_ref().is_some_and(|t| self.has_groups(t));
        let all = self.captures_of(&written.text);
        none && all.is_some() && all == self.captures_of(&after.text)
    }

    /// Whether `text` holds a capture group, or cannot be read to tell (#6310).
    fn has_groups(&self, text: &Text) -> bool {
        self.captures && self.captures_of(&text.text) != Some(Vec::new())
    }

    /// `written`, the loop as it was, for a loop the rewrite leaves alone because of a capture group.
    fn unlisted(written: Text) -> Piece {
        Piece {
            text: written,
            ways: Ways::Unknown,
        }
    }

    /// Whether Oniguruma writes the lazy, unbounded `X{min,}?` out as `min` plain copies of `X`
    /// followed by `X*?`, which it does when the body's compiled length times `min` is at most
    /// ten (`QUANTIFIER_EXPAND_LIMIT_SIZE`). A longer body gets a counted loop, which leaves at
    /// the body's empty way even before the minimum, and is what [`Self::lazy_counted`] writes.
    /// `false` when the length cannot be told, which keeps the rewrite.
    fn oniguruma_unrolls(&self, body: &Ast, min: u32) -> bool {
        // A body of no operations (`(?:)`) has no empty way to take and no count to bound the
        // copies, so it is never the one this is for.
        min >= 2
            && !self.shadowed
            && onig_ops(body, self.folds)
                .filter(|&ops| ops > 0)
                .and_then(|ops| ops.checked_mul(min as usize))
                .is_some_and(|total| total <= ONIG_EXPAND_LIMIT)
    }

    /// `X{min,}?` as Oniguruma unrolls it, `X` `min` times and then `X*?`: the `regex` crate reads
    /// the repeat the same way, so the text stays as written. Only the body's ways are worked out,
    /// as the concatenation of those copies, for a loop around this one.
    fn lazy_unrolled(&mut self, body: Piece, min: u32, grouped: bool) -> Result<Piece, GiveUp> {
        let text = self.repeat_text(&body.text, min, None, false)?;
        // #6310, #6332: the copies would hold the body's groups again, and the text is as it was.
        if grouped {
            return Ok(Piece {
                text,
                ways: Ways::Unknown,
            });
        }
        let mut copies = alloc::vec![body.clone(); min as usize];
        copies.push(self.repeat(body, 0, None, false)?);
        let ways = self.concat(&copies, true)?.ways;
        Ok(Piece { text, ways })
    }

    /// Oniguruma's lazy `X{min,max}?` that must run twice or more, for a body `X = A | ε | B`: copy `k` tries `ε A R(k+1) B R(k+1)` once `k` is past `min` (leaving
    /// first, as a lazy repeat does) and `A R(k+1) ε B R(k+1)` before it, where `R(k)` is the copies
    /// from `k` on, the lazy `(?:A|B)*?` once an unbounded repeat has run its `min`, and nothing after
    /// `max`. The empty way leaves the loop at any copy, as it does a greedy one. Written out only
    /// when `B` is not empty, as [`Self::bounded`] is.
    fn lazy_counted(
        &mut self,
        before: Option<Text>,
        after: Option<Text>,
        min: u32,
        max: Option<u32>,
        written: Text,
        exit: Option<Text>,
    ) -> Result<Piece, GiveUp> {
        // #6310: each copy holds the body's groups again; with the groups tagged they are read back
        // (#6332), in a loop inside another repeat too (#6358).
        if self.has_groups(&written) || self.exit_keeps_text(&exit) {
            return Ok(Self::unlisted(written));
        }
        let consuming = |text: Text| Piece {
            text,
            ways: Ways::Consuming,
        };
        let ways: Vec<Text> = before.iter().chain(after.iter()).cloned().collect();
        let mut rest = match (max, self.alt_text(&ways)?) {
            (None, Some(body)) => self.repeat(consuming(body), 0, None, false)?,
            _ => self.concat(&[], true)?,
        };
        for k in (1..=max.unwrap_or(min)).rev() {
            let mut ways = Vec::new();
            // Past the minimum the lazy repeat leaves before it makes an iteration, which sets no
            // group; before it, the empty way is an iteration that does (#6332).
            if k > min {
                ways.push(self.concat(&[], true)?);
            }
            if let Some(a) = &before {
                ways.push(self.concat(&[consuming(a.clone()), rest.clone()], true)?);
            }
            if k <= min {
                ways.push(self.exit_piece(&exit)?);
            }
            if let Some(b) = &after {
                ways.push(self.concat(&[consuming(b.clone()), rest.clone()], true)?);
            }
            rest = self.alt(&ways, true)?;
        }
        self.changed = true;
        Ok(Piece {
            text: self.text(format!("(?:{})", rest.text.text), Prec::Atom)?,
            ways: rest.ways,
        })
    }

    /// A repeat of a body with one way that is only an anchor, `X = A | c | B` (#6358). `c` ends a
    /// loop only where it holds, and where it does not the loop goes on to `B` and may leave
    /// without an iteration, which is what the plain forms cannot say. What Oniguruma does depends
    /// on whether it writes the repeat out as copies, which hold no empty check, or counts it:
    /// - a greedy `X{min,}` is `X` `min` times and [`Self::anchored_loop`] when it copies; when it
    ///   counts, it is `R(min)`, where `R(k) = A R(k-1) | E | B R(k-1)` and `R(0)` is that loop,
    ///   since `E`, the empty way that sets the groups, leaves the counted repeat outright;
    /// - a greedy bounded repeat it copies is what the `regex` crate reads the text as; one it
    ///   counts is `T(n)`, as `R` with `T(0)` nothing, and a way of its own that leaves once `min`
    ///   iterations have run, last, since the loop tries that after the body fails;
    /// - a lazy `X{min,}?` or `X{min,max}?` that counts is [`Self::lazy_counted`], with `E` as the
    ///   exit; anything else lazy leaves first, and `c` is an exit that has already failed.
    fn anchored_repeat(
        &mut self,
        ast: &Ast,
        body: &Text,
        (before, anchor, after): (&Option<Text>, &Anchor, &Option<Text>),
        min: u32,
        max: Option<u32>,
        greedy: bool,
    ) -> Result<Piece, GiveUp> {
        let grouped = has_capture(ast);
        let ways = if grouped { Ways::Unknown } else { Ways::Opaque };
        let written = self.repeat_text(body, min, max, greedy)?;
        let kept = Piece {
            text: written.clone(),
            ways: ways.clone(),
        };
        if self.has_groups(&written) || (grouped && self.lazy_around > 0) {
            return Ok(kept);
        }
        // With no `B`, and no group for `E` to set, the next iteration can only fail where `A` did,
        // so the text already leaves where Oniguruma does (as [`Self::exit_on_empty`] says). Short
        // of a minimum of two: there `c` then `A` makes the count, where Oniguruma has left. That
        // needs an `A`, and a body of one way is one `may_need_rewrite` does not admit.
        let minimum_counts = min >= 2 && before.is_some() && written.text.contains('|');
        if after.is_none() && !grouped && !minimum_counts {
            return Ok(kept);
        }
        let exit = self.empty_text(ast)?;
        let ops = onig_ops(ast, self.folds).filter(|_| !self.shadowed);
        let copied = |length: Option<usize>, count: u32| {
            length
                .and_then(|length| length.checked_mul(count as usize))
                .is_some_and(|total| total <= ONIG_EXPAND_LIMIT)
        };
        let consuming = |this: &Self, way: &Option<Text>, rest: &Text| {
            way.as_ref()
                .map(|way| this.concat_text(&[way, rest]))
                .transpose()
        };
        let text = match (greedy, max) {
            (false, _) => {
                if min < 2 || (max.is_none() && self.oniguruma_unrolls(ast, min)) {
                    return Ok(kept);
                }
                let (before, after) = (before.clone(), after.clone());
                let piece =
                    self.lazy_counted(before, after, min, max, written, Some(exit.clone()))?;
                return Ok(Piece {
                    text: piece.text,
                    ways,
                });
            }
            (true, None) => {
                let mut rest = self.anchored_loop(before, anchor, after, &exit)?;
                if min > 0 && copied(ops, min) {
                    let copies = self.repeat_text(body, min, Some(min), true)?;
                    rest = self.concat_text(&[&copies, &rest])?;
                } else {
                    for _ in 0..min {
                        let first = consuming(self, before, &rest)?;
                        let last = consuming(self, after, &rest)?;
                        let list: Vec<Text> = first
                            .into_iter()
                            .chain([exit.clone()])
                            .chain(last)
                            .collect();
                        rest = self.alt_text(&list)?.ok_or(GiveUp)?;
                    }
                }
                rest
            }
            (true, Some(n)) => {
                if n <= 1 || copied(ops.map(|ops| ops + 1), n) {
                    return Ok(kept);
                }
                let mut rest = self.concat_text(&[])?;
                for k in 0..n {
                    let first = consuming(self, before, &rest)?;
                    let last = consuming(self, after, &rest)?;
                    let run = n - 1 - k;
                    let leave = (run >= min).then(|| self.concat_text(&[])).transpose()?;
                    let list: Vec<Text> = first
                        .into_iter()
                        .chain([exit.clone()])
                        .chain(last)
                        .chain(leave)
                        .collect();
                    rest = self.alt_text(&list)?.ok_or(GiveUp)?;
                }
                rest
            }
        };
        self.changed = true;
        Ok(Piece {
            text: self.text(format!("(?:{})", text.text), Prec::Atom)?,
            ways,
        })
    }

    /// Oniguruma's loop over `X = A | c | B` for an anchor `c`: `L = A L | c | B L | ε`, the last
    /// way being the one every loop has when its body fails. That is `A* T`, with `I = B A*` and
    /// `T = c | I T | ε`, an exit on either side of the way back, which no one star says. It is
    /// written from what the anchor is, with `E` the empty way that sets the groups, and each form
    /// tries what the recursion tries, in its order (0 of 7,040 cases differ from jq 1.7.1):
    /// - `\b`: at a head where `c` holds, `c` comes first; where its complement `n` holds, `I` does.
    ///   Neither way's text can match at the other's heads, so the two orders can share a loop:
    ///   `(?:n I)*(?:c I (?:n I)*)*?(?:E|)`, greedy over the one and lazy over the other.
    /// - `\A`: only the first head can hold it, so `T` is `(?:E|I+|)`.
    /// - `\z`: no `I` runs where it holds, so `T` is `I*(?:E|)`.
    ///
    /// `^` and `$` are written as `\A` and `\z`. In jq `$` holds at the end alone, or before a final
    /// newline, and `^` does not end a loop after a newline inside the text: `--assert --newlines`
    /// in `scripts/jq-regex-capture-loop-sweep.py` finds no pattern moved away from jq. Under `(?m)`
    /// they hold at every line and stay [`Ways::Opaque`].
    fn anchored_loop(
        &self,
        before: &Option<Text>,
        anchor: &Anchor,
        after: &Option<Text>,
        exit: &Text,
    ) -> Result<Text, GiveUp> {
        let star = |text: &Text| self.repeat_text(text, 0, None, true);
        let looped = before.as_ref().map(star).transpose()?;
        let leave = self.text(format!("(?:{}|)", exit.text), Prec::Atom)?;
        let iteration = after
            .as_ref()
            .map(|b| match &looped {
                Some(a) => self.concat_text(&[b, a]),
                None => Ok(b.clone()),
            })
            .transpose()?;
        let tail = match (iteration, &anchor.kind) {
            (None, _) => leave,
            (Some(iteration), AnchorKind::Word { fails }) => {
                let elsewhere = star(&self.concat_text(&[fails, &iteration])?)?;
                let here = self.concat_text(&[&anchor.text, &iteration, &elsewhere])?;
                let here = self.repeat_text(&here, 0, None, false)?;
                self.concat_text(&[&elsewhere, &here, &leave])?
            }
            (Some(iteration), AnchorKind::Start) => {
                let more = self.repeat_text(&iteration, 1, None, true)?;
                self.text(format!("(?:{}|{}|)", exit.text, more.text), Prec::Atom)?
            }
            (Some(iteration), AnchorKind::End) => {
                self.concat_text(&[&star(&iteration)?, &leave])?
            }
        };
        match &looped {
            Some(a) => self.concat_text(&[a, &tail]),
            None => Ok(tail),
        }
    }

    /// Whether a loop written with its exit `exit` keeps its own text instead: a lazy `*?` or `+?`
    /// stands around it (#6358), which is not rewritten, and the loop written with its exit answers
    /// differently from Oniguruma's under a repeat that is not. The one rule for every arm that
    /// writes an exit.
    fn exit_keeps_text(&self, exit: &Option<Text>) -> bool {
        exit.is_some() && self.lazy_around > 0
    }

    /// The empty way of a counted loop's copy: nothing, or `exit` (#6332).
    fn exit_piece(&self, exit: &Option<Text>) -> Result<Piece, GiveUp> {
        match exit {
            Some(text) => Ok(Piece {
                text: text.clone(),
                ways: Ways::Nullable {
                    before: None,
                    after: None,
                },
            }),
            None => self.concat(&[], true),
        }
    }

    /// `ast` with every character, class and `.` replaced by one that never matches: it can match
    /// only the empty string, and does so the way `ast` would first, setting the groups that way
    /// sets (#6332). A loop the rewrite writes without its empty way ends with it, so that those
    /// groups are set at the exit as Oniguruma's empty iteration sets them (#6069).
    fn empty_text(&self, ast: &Ast) -> Result<Text, GiveUp> {
        let mut out = String::new();
        self.empty_into(ast, &mut out);
        self.text(out, Prec::Alt)
    }

    fn empty_into(&self, ast: &Ast, out: &mut String) {
        // A part with no group and no assertion only says whether it can match nothing: it can, at
        // any position, or it cannot, so it is written as nothing or as one failing class. That is
        // every leaf (a character, a class, `.` or nothing) and every part without a group.
        if !has_capture(ast) && !has_assertion_or_flags(ast) {
            if !may_be_empty(ast) {
                out.push_str(NEVER);
            }
        } else if let Ast::Group(group) = ast {
            let inner = group.ast.span();
            out.push_str(&self.source[group.span.start.offset..inner.start.offset]);
            self.empty_into(&group.ast, out);
            out.push_str(&self.source[inner.end.offset..group.span.end.offset]);
        } else if let Ast::Alternation(alt) = ast {
            for (at, way) in alt.asts.iter().enumerate() {
                if at > 0 {
                    out.push('|');
                }
                self.empty_into(way, out);
            }
        } else if let Ast::Concat(concat) = ast {
            for item in &concat.asts {
                self.empty_into(item, out);
            }
        } else if let Ast::Repetition(rep) = ast {
            out.push_str("(?:");
            self.empty_into(&rep.ast, out);
            out.push(')');
            // The operator's span holds the `?` of a lazy one.
            out.push_str(self.verbatim(&rep.op.span));
        } else {
            // An assertion or a flag switch stays where it stands.
            out.push_str(self.verbatim(ast.span()));
        }
    }

    /// Oniguruma's greedy `X{min,n}` for a body `X = A | ε | B` and `n` of 2 or more: `T(n)`, where
    /// `T(1)` is `X` and `T(k)` is `(?:A T(k-1)||B T(k-1))`. The empty way ends the loop, whatever
    /// `min` is, where the `regex` crate's copies of `X` would go on to try the next copy's ways
    /// first. Written out only when `B` is not empty; with no `B`, the next copy can only fail
    /// where `A` failed, so the copies agree with Oniguruma and the loop keeps its text.
    fn bounded(
        &mut self,
        body: &Piece,
        before: Option<Text>,
        after: Option<Text>,
        n: u32,
        written: Text,
        exit: Option<Text>,
    ) -> Result<Piece, GiveUp> {
        let Some(b) = after else {
            let rest = self.repeat_text(&body.text, 0, Some(n - 1), true)?;
            let before = before.map(|a| self.concat_text(&[&a, &rest])).transpose()?;
            return Ok(Piece {
                text: written,
                ways: Ways::Nullable {
                    before,
                    after: None,
                },
            });
        };
        let consuming = |text: Text| Piece {
            text,
            ways: Ways::Consuming,
        };
        // #6310: each copy holds the body's groups again; with the groups tagged they are read back
        // (#6332), in a loop inside another repeat too (#6358).
        if self.has_groups(&written) || self.exit_keeps_text(&exit) {
            return Ok(Self::unlisted(written));
        }
        let mut loop_rest = body.clone();
        for _ in 1..n {
            let mut ways = Vec::new();
            if let Some(a) = &before {
                ways.push(self.concat(&[consuming(a.clone()), loop_rest.clone()], true)?);
            }
            ways.push(self.exit_piece(&exit)?);
            ways.push(self.concat(&[consuming(b.clone()), loop_rest], true)?);
            loop_rest = self.alt(&ways, true)?;
        }
        self.changed = true;
        Ok(Piece {
            text: self.text(format!("(?:{})", loop_rest.text.text), Prec::Atom)?,
            ways: loop_rest.ways,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #6128: beside syntax only `fancy-regex` reads, a loop is rewritten as it is alone, and that
    /// syntax is copied as written; inside a lookaround or atomic group, too. A loop whose body
    /// holds such syntax keeps its text.
    #[test]
    fn a_loop_beside_fancy_regex_syntax_is_rewritten_6128() {
        let looped = rewrite("(?:b*|a)+").unwrap();
        for (before, after) in [
            ("", "(?=)"),
            ("(?<=x)", ""),
            ("(?<!x)", "(?!c)"),
            (r"(b)\1", ""),
            (r"(?<n>x)\k<n>", ""),
            ("(?#é|x)", ""),
            (r"\G", r"\Z"),
            (r"x\K", ""),
        ] {
            let pattern = format!("{before}(?:b*|a)+{after}");
            assert_eq!(
                rewrite(&pattern),
                Some(format!("{before}{looped}{after}")),
                "{pattern}"
            );
        }
        assert_eq!(rewrite("(?>(?:b*|a)+)"), Some(format!("(?>{looped})")));
        assert_eq!(rewrite("(?=(?:b*|a)+c)"), Some(format!("(?={looped}c)")));
        // A quantifier after a comment is Oniguruma's to read: the comment is copied as written.
        assert_eq!(rewrite("(?:b*|a)+(?#)?"), Some(format!("{looped}(?#)?")));
        // Under `x`, a `(?#` in a `#` comment the copy cannot follow: read as written.
        assert_eq!(
            rewrite("(?x)(?:b*|a)+ # (?#"),
            rewrite("(?x)(?:b*|a)+ # c").map(|r| r.replace("# c", "# (?#"))
        );
        // A comment with no quantifier after it matches nothing, as an empty group does.
        assert_eq!(rewrite("(?:(?#c)b*|a)+"), Some(looped.clone()));
        for pattern in [
            "(?:(?#)?|b)+",
            "(?:(?#) ?|b)+",
            "(?:b(?#)*|a)+",
            "(?:(?=b)b*|a)+",
            r"(b)(?:\1|b*|a)+",
            "(?#a\nb)(?:b*|a)+",
        ] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
    }

    #[test]
    fn a_loop_with_a_consuming_way_after_its_first_empty_one_is_rewritten() {
        assert_eq!(
            rewrite("(?:b*|a)+").as_deref(),
            Some("(?:(?:b+)*(?:a(?:b+)*)*?)")
        );
        assert_eq!(
            rewrite("(?:b*|a)*").as_deref(),
            Some("(?:(?:b+)*(?:a(?:b+)*)*?)")
        );
        assert_eq!(
            rewrite("(?:b*|a){2,}").as_deref(),
            Some("(?:(?:b+)*(?:a(?:b+)*)*?)")
        );
        assert_eq!(rewrite("(?:|a)*").as_deref(), Some("(?:a*?)"));
        assert_eq!(
            rewrite("(?:b*|a){2}").as_deref(),
            Some("(?:b+(?:b*|a)||a(?:b*|a))")
        );
        assert_eq!(
            rewrite("(?:b*|a){0,3}").as_deref(),
            Some("(?:b+(?:b+(?:b*|a)||a(?:b*|a))||a(?:b+(?:b*|a)||a(?:b*|a)))")
        );
    }

    #[test]
    fn a_loop_the_engines_agree_on_keeps_its_text() {
        for pattern in [
            "(?:a|b*)+",      // the empty way comes last
            "(?:b*|a)*?",     // lazy
            "(?:b*|a){1,2}?", // lazy, runs once at most
            "(?:a|b*){2}",
            "(b*|a)+",           // a capture group
            r"(?:\A\A|a|\b\b)+", // two anchors
            "(?U)(?:b*|a)+",
            "(?:(?U:b*)|a)+",
            "(?:a+)*",
            "abc",
            "(?:b*|a",
        ] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
    }

    /// #6310: a capture group in a way after the first empty one is written once, in the iteration,
    /// and the loop is rewritten as one over plain ways is. A group the rewrite would copy (in `A`,
    /// the consuming ways before the empty one, which the rewrite writes twice, or in the body of a
    /// counted repeat) or drop (in the empty way) keeps the loop as written, as does one whose empty
    /// way comes first, which sets the group at the exit.
    #[test]
    fn a_capture_group_in_a_later_way_is_written_once_6310() {
        assert_eq!(
            rewrite("(?:b*|(a))+").as_deref(),
            Some("(?:(?:b+)*(?:(a)(?:b+)*)*?)")
        );
        assert_eq!(
            rewrite("x(?:b*|(?<n>a)|(c))*y").as_deref(),
            Some("x(?:(?:b+)*(?:(?:(?<n>a)|(c))(?:b+)*)*?)y")
        );
        // A group that may match nothing, in a later way: its empty way is an exit already tried.
        assert_eq!(
            rewrite("(?:b*|(a*))+").as_deref(),
            Some("(?:(?:b+)*(?:(a+)(?:b+)*)*?)")
        );
        // The empty way first, then the group: the exit is taken before the group is tried.
        assert_eq!(rewrite("(?:|(a))+").as_deref(), Some("(?:(a)*?)"));
        for pattern in [
            "(?:(a)|b*|c)+",   // the group is in `A`, which the rewrite writes twice
            "(?:b*|()|a)+",    // the group is in the empty way, which it leaves out
            "(?:(|a)|b)+",     // the group's empty way is the loop's first
            "(?:b*|(a)){2}",   // a counted repeat copies the group
            "(?:b*|(a)){2,}?", // so does a lazy one
            "(?:b*|(a)){0,3}",
            "(b*|(a))+", // the repeated group has a group of its own in `A`
        ] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
    }

    /// #6332: with the groups tagged, a loop is written with every group it holds copied, and the
    /// copies are listed for each group of the input. The consuming ways before the empty one are
    /// written twice, and the empty way, which the loop leaves out, ends it as `E`: the body with
    /// every character, class and `.` replaced by a class that never matches.
    #[test]
    fn a_loop_with_groups_is_written_with_the_copies_listed_6332() {
        let copied = |pattern: &str| {
            rewrite_with_groups(pattern).map(|g| (g.pattern, g.copies, g.compiled_groups))
        };
        assert_eq!(
            copied("(?:(a)|b*|c)+"),
            Some((
                String::from(r"(?:(?:(a)|b+)*(?:c(?:(a)|b+)*)*?(?:(?:([^\s\S])||[^\s\S])))"),
                vec![vec![0], vec![1, 2, 3]],
                4
            ))
        );
        // A group in the empty way is set by `E`, as the empty iteration sets it.
        assert_eq!(
            copied("(?:b*|()|a)+"),
            Some((
                String::from(r"(?:(?:b+)*(?:a(?:b+)*)*?(?:(?:|()|[^\s\S])))"),
                vec![vec![0], vec![1]],
                2
            ))
        );
        // So is a group whose own empty way comes first.
        assert_eq!(
            copied("(?:(|a)|b)+"),
            Some((
                String::from(r"(?:(?:(a)|b)*?(?:(?:()|[^\s\S])))"),
                vec![vec![0], vec![1, 2]],
                3
            ))
        );
        // Groups outside the loop stay as they were, and a name is dropped from every copy.
        assert_eq!(
            copied("(a)(?:b*|(c))+(?<n>d)"),
            Some((
                String::from(r"(a)(?:(?:b+)*(?:(c)(?:b+)*)*?(?:(?:|([^\s\S]))))(d)"),
                vec![vec![0], vec![1], vec![2, 3], vec![4]],
                5
            ))
        );
        // The flags `i` and `s` of jq are a switch in front of the pattern: it stays there, and
        // case folding is read from it.
        assert_eq!(
            copied("(?i)(?:(a)|b*|c)+"),
            copied("(?:(a)|b*|c)+").map(|(pattern, copies, groups)| (
                format!("(?i){pattern}"),
                copies,
                groups
            ))
        );
        assert_eq!(
            copied("(?is)(?-i)(?:b*|(a))+").map(|g| g.0.starts_with("(?is)(?-i)")),
            Some(true)
        );
        // An assertion beside what consumes text is copied with it, and stays in `E`; a lazy repeat
        // of a group keeps its `?` there.
        assert_eq!(
            copied(r"(?:(a)\b|b*|c)+").map(|g| g.0),
            Some(String::from(
                r"(?:(?:(a)\b|b+)*(?:c(?:(a)\b|b+)*)*?(?:(?:([^\s\S])\b||[^\s\S])))"
            ))
        );
        assert!(copied("(?:(a)+?|b*|c)+").is_some_and(|g| g.0.contains(r"(?:([^\s\S]))+?")));
        // Nothing to write: no group, no loop that needs it, only a bounded body with no `B`,
        // a switch that names `x` or `U` (or sits after the loop), or the tag itself.
        for pattern in [
            "(?:a|b){2,}",
            "(?:b*|a)+",
            "(a)(?:a|b)+",
            "(?:(a)|b*){0,3}",
            "(?x)(?:b*|(a))+",
            "(?U)(?:b*|(a))+",
            "(?:b*|(a))+__succinctly_g1",
        ] {
            assert_eq!(copied(pattern), None, "{pattern}");
        }
    }

    /// #6358: a loop with groups inside another repeat that can run it again is written with the rest,
    /// its ways listed through its exit, so the repeat around it ends at the empty iteration as
    /// Oniguruma's does; so is a counted repeat over a body that holds a loop of its own.
    #[test]
    fn a_loop_with_groups_inside_another_repeat_is_written_6358() {
        for pattern in [
            "(?:(?:(a)|b*|c)+)+",
            "(?:(?:(a|b)|(?:|a)){0,3}){2}?",
            "(?:((b|)(|c)){3})+",
            "(?:(?:b*|(a))+(a|b)|(?:b*|(a))+){2}",
            "(?:(?:(?:b+)*(?:(a)(?:b+)*)*?)(a|b)|(?:(?:b+)*(?:(a)(?:b+)*)*?)){2}",
            "((?:(b|)*|a)+)*",
            "((?:(b)*|a)+){2}",
        ] {
            let written = rewrite_with_groups(pattern).unwrap_or_else(|| panic!("{pattern}"));
            // Every group of the pattern has at least one copy, and every compiled group is read back.
            assert!(written.copies.iter().all(|c| !c.is_empty()), "{pattern}");
            assert_eq!(
                written.copies.iter().map(Vec::len).sum::<usize>(),
                written.compiled_groups,
                "{pattern}"
            );
        }
    }

    /// #6358: a lazy `*?` or `+?` around a loop with groups is not rewritten, and the loop inside it,
    /// written with its exit, is not read the way Oniguruma's is (`"abab" | match("(?:x*|(?:b*|(a))+
    /// (?:(b)*|a)+)+?b")` has no `a` in group 1 in jq), so that loop keeps its text.
    #[test]
    fn a_loop_with_groups_inside_a_lazy_repeat_keeps_its_text_6358() {
        for pattern in [
            "(?:x*|(?:b*|(a))+(?:(b)*|a)+)+?b",
            "(?:b*|(?:b*|(a))+(?:(b)*|a)+)*?b",
            "(?:(?:b*|(a))+)*?",
            "(?:(?:b*|(a))+){1,3}?",
        ] {
            assert_eq!(
                rewrite_with_groups(pattern).map(|g| g.pattern),
                None,
                "{pattern}"
            );
        }
        // A lazy repeat that is written out copy by copy still takes the loops inside it.
        assert!(rewrite_with_groups("(?:(?:b*|(a))+){2,3}?").is_some());
    }

    /// #6358: a flag switch in the body of a loop with groups is written as the flag group it stands
    /// for, over what it reaches, with the empty way of what it reaches scoped the same way.
    #[test]
    fn a_flag_switch_beside_groups_is_scoped_over_what_it_reaches_6358() {
        let copied = |pattern: &str| rewrite_with_groups(pattern).map(|g| g.pattern);
        // After the loop it reaches nothing, and is left out.
        assert_eq!(
            copied("(?:b*|(a))+(?i)").as_deref(),
            Some(r"(?:(?:b+)*(?:(a)(?:b+)*)*?(?:(?:|([^\s\S]))))")
        );
        // Before a group, in the items after it and in every later way.
        for pattern in [
            "(?:b*|(?i)(a))+",
            "(?:(?i)b*|(a))+",
            "(?:(?i)|b*|(a))+",
            "(?:b*(?i)|(a)b)+",
            "(?:b*|(a)(?i)b)+",
        ] {
            let written = copied(pattern).unwrap_or_else(|| panic!("{pattern} is written"));
            assert!(written.contains("(?i:"), "{pattern}: {written}");
        }
        // A switch that names `x` or `U` still keeps the loop as written.
        assert_eq!(copied("(?x)(?:b*|(a))+"), None);
        assert_eq!(copied("(?U)(?:b*|(a))+"), None);
    }

    /// A lazy repeat that must run twice or more (#6128): copy `k` leaves first once it is past the
    /// minimum, and takes its empty way in place before it.
    #[test]
    fn a_lazy_counted_loop_is_written_copy_by_copy() {
        assert_eq!(
            rewrite("(?:b*|a){2}?").as_deref(),
            Some("(?:b+(?:b+||a)||a(?:b+||a))")
        );
        for pattern in ["(?:b*|a){2,3}?", "(?:b*|a){2,}?", "(?:b*|a){3,4}?"] {
            assert!(rewrite(pattern).is_some(), "{pattern}");
        }
        // Past the minimum a copy leaves first: `{2,3}?` is `{2}?`'s copies, each of the second
        // preceded by the empty way.
        assert_eq!(
            rewrite("(?:b*|a){2,3}?").as_deref(),
            Some("(?:b+(?:b+(?:|b+|a)||a(?:|b+|a))||a(?:b+(?:|b+|a)||a(?:|b+|a)))")
        );
        // No consuming way after the empty one: the copies agree with Oniguruma.
        assert_eq!(rewrite("(?:a|b*){2,3}?"), None);
    }

    /// The copies multiply with the count, so a large one hits the size limit and is left as
    /// written, quickly.
    #[test]
    fn a_lazy_counted_loop_with_a_large_count_is_given_up() {
        assert_eq!(rewrite("(?:b*|a){2,1000}?"), None);
        assert_eq!(rewrite("(?:b*|a){1000,}?"), None);
    }

    fn ops(pattern: &str) -> Option<usize> {
        let ast = ast::parse::Parser::new().parse(pattern).unwrap();
        onig_ops(&ast, may_fold_case(pattern))
    }

    /// The lengths Oniguruma 6.9.8 compiles bodies to, as far as the count of a repeat in jq 1.7.1
    /// shows them (#6128): `(?:b?|a)` is five operations, since `{2,}?` over it is written out as
    /// copies (ten) and `{3,}?` is not (fifteen).
    #[test]
    fn a_body_is_counted_in_oniguruma_operations() {
        assert_eq!(ops("a"), Some(1));
        assert_eq!(ops("ab"), Some(1), "adjacent literals are one string");
        assert_eq!(ops("a\\w"), Some(2));
        assert_eq!(ops("[ab]"), Some(1));
        assert_eq!(ops("a?"), Some(2));
        assert_eq!(ops("a??"), Some(3));
        assert_eq!(ops("a*"), Some(3));
        assert_eq!(ops("a|b"), Some(4));
        assert_eq!(ops("b?|a"), Some(5));
        assert_eq!(ops("b?|ab"), Some(5));
        assert_eq!(ops("|a"), Some(3));
        // A body that may match nothing gets an empty check around a loop's body.
        assert_eq!(ops("(?:a?)*"), None, "a repeat of a repeat is reduced");
        assert_eq!(ops("(?:a|)*"), Some(2 + (3 + 2)));
        // A capture group is its body between the two operations that start and end it (#6332).
        assert_eq!(ops("(a)"), Some(3));
        assert_eq!(ops("(a|b?)"), Some(2 + 2 + 3));
        assert_eq!(ops("(a)*"), None, "a repeat of a group is not worked out");
        // Not worked out: an assertion, a flag switch, a folded letter.
        assert_eq!(ops("(a^)"), Some(4), "#6358: an anchor is one operation");
        assert_eq!(ops("^a"), Some(2));
        assert_eq!(ops("(?i)a"), None);
        // An option group is its body (#6241), with the folding it sets.
        assert_eq!(ops("(?i:a)"), Some(1));
        assert_eq!(ops("(?s:\\w)"), Some(1));
        assert_eq!(ops("a?|(?-i:\\p{L})"), Some(5));
        assert_eq!(
            ops("(?i:s)"),
            None,
            "a letter that starts a multi-character fold"
        );
        assert_eq!(ops("(?-i:s)"), Some(1), "no folding inside");
        assert_eq!(ops("(?x:a)"), None, "extended mode is not followed");
    }

    /// The repeat arms of the count, from the formulas in Oniguruma's `compile_length_quantifier_node`.
    #[test]
    fn a_repeat_is_counted_by_its_kind() {
        assert_eq!(ops("a{0}"), Some(0), "never compiled");
        assert_eq!(ops("a{2}"), Some(2), "unrolled copies");
        assert_eq!(ops("a{2,3}"), Some(4), "copies and a push for the third");
        assert_eq!(
            ops("a{2,3}?"),
            Some(3),
            "a lazy bounded repeat is a counted loop"
        );
        assert_eq!(ops("a{2,}"), Some(2 + 2 + 1), "copies, then a star");
        assert_eq!(ops("a+"), Some(1 + 2 + 1));
        assert_eq!(ops("a+?"), Some(1 + 2 + 1));
        assert_eq!(ops("a*?"), Some(3));
        assert_eq!(ops("(?:a|b)*"), Some(2 + 4));
        // An empty check costs two more, whatever makes the body nullable.
        assert_eq!(ops("(?:a?b?)*"), Some(2 + (4 + 2)));
        assert_eq!(ops("(?:a|b?)*"), Some(2 + (5 + 2)));
        assert_eq!(ops("(?:a{0,})*"), None, "a repeat of a repeat");
        assert_eq!(
            ops("(?:(?:a|b))*"),
            Some(2 + 4),
            "a plain group is looked through"
        );
        // A body past the expansion limit is entered through a jump.
        assert_eq!(ops("(?:a|b|c|d|e|f)+"), Some(1 + 2 + 16));
        // A folded literal run, a bracketed class under folding: not counted. (A flag group is its
        // body, #6241.)
        assert_eq!(ops("(?s:a)"), Some(1));
        assert_eq!(
            ops("(?s:a)*"),
            Some(3),
            "an option group is looked through (#6241)"
        );
        assert_eq!(
            ops("(?s:a?)*"),
            None,
            "a repeat of a repeat, through the group"
        );
        assert_eq!(ops("(?i)b"), None, "the flag itself is not counted");
        assert_eq!(
            ops("(?:a*b)*"),
            Some(2 + 3 + 1),
            "a body that is not nullable"
        );
        assert_eq!(ops("(?:a*b*)*"), Some(2 + (3 + 3 + 2)), "a body that is");
        assert_eq!(ops("(?:)*"), Some(0));
        // Under case folding, as the flag sets it: a run of letters may fold together, a letter
        // that starts a fold or a class may be more than one operation, any other letter and `.`
        // is one.
        let folding =
            |pattern: &str| onig_ops(&ast::parse::Parser::new().parse(pattern).unwrap(), true);
        assert_eq!(folding("b"), Some(1));
        assert_eq!(folding("."), Some(1));
        assert_eq!(folding("s"), None);
        assert_eq!(folding("\u{DF}"), None);
        assert_eq!(folding("bc"), None);
        assert_eq!(folding("[ab]"), None);
        assert_eq!(folding("\\p{L}"), None);
        assert_eq!(folding("\\w"), Some(1));
        // The bracketed class the translation writes for `\w` and `\W` is one operation too (#6263),
        // a class of the same members written any other way is not.
        let word = super::super::eval::JQ_WORD_CLASS;
        assert_eq!(folding(&format!("[{word}]")), Some(1));
        assert_eq!(folding(&format!("[^{word}]")), Some(1));
        assert_eq!(folding(&format!("[{word}a]")), None);
        assert_eq!(folding("[\\p{Alphabetic}]"), None);
    }

    /// `X{n,}?` over a body of five operations is plain copies at `{2,}?` and a counted loop from
    /// `{3,}?` on (#6128): the first keeps its text, the others are written out.
    #[test]
    fn a_lazy_repeat_oniguruma_unrolls_keeps_its_text() {
        assert_eq!(rewrite("(?:b?|a){2,}?"), None);
        assert_eq!(rewrite("(?:b?|\\w){2,}?"), None);
        assert!(rewrite("(?:b?|a){3,}?").is_some());
        assert!(rewrite("(?:b?|a){4,}?").is_some());
        // A body of more operations is a counted loop at two.
        assert!(rewrite("(?:b*|a){2,}?").is_some());
        // An ignore-case flag may fold a letter that starts a multi-character fold into more
        // operations: not worked out.
        assert!(rewrite("(?i)(?:s?|a){2,}?").is_some());
        // A body of no operations has no empty way to take, and no count to bound its copies.
        assert_eq!(rewrite("(?:){4000000000,}?"), None);
        // Under an ignore-case flag a plain letter is still one operation, and `.` too.
        assert_eq!(rewrite("(?i)(?:b?|.){2,}?"), None);
        assert!(rewrite("(?i)(?:s?|.){2,}?").is_some(), "`s` starts a fold");
        // The copies a count of six or seven makes are written out in full.
        assert!(rewrite("(?:b*|\\w){6,}?").is_some());
        assert!(rewrite("(?:b*|\\w){7,}?").is_some());
    }

    /// #6302: each count doubles the text of a lazy `{n,}?` over a body with a consuming way before
    /// its empty one (`(?:b*||\\w)`), because every level holds the next level twice with the empty
    /// way between the two, so no spelling in the `regex` crate's syntax is smaller. The text of
    /// one rewritten pattern is capped at 32 KiB (`Rewriter::limit`): this body, whose `\\w` is two
    /// characters here and a bracketed class of several hundred once the pattern has been read,
    /// passes it at `{11,}?`, and `docs/compliance/jq/limitations.md` records the counts at which
    /// the bodies it names pass it. Lifting the cap would not move that far, since the `regex` crate
    /// then compiles a pattern that doubles with every further count (0.28 s at `{8,}?` and 74 s at
    /// `{16,}?` in a debug build, with the cap lifted).
    #[test]
    fn a_lazy_counted_repeat_doubles_with_its_count_and_is_given_up_at_eleven_6302() {
        let size = |n: u32| rewrite(&format!("(?:b*||\\w){{{n},}}?")).map(|text| text.len());
        let sizes: Vec<Option<usize>> = (4..=10).map(size).collect();
        assert!(sizes.iter().all(Option::is_some), "{sizes:?}");
        // Doubling, to within the fixed text each level adds.
        for pair in sizes.windows(2) {
            let (small, large) = (pair[0].unwrap(), pair[1].unwrap());
            assert!(
                (large * 10 >= small * 19) && (large * 10 <= small * 24),
                "{small} then {large}"
            );
        }
        assert!(sizes[6].unwrap() < 32 * 1024, "{sizes:?}");
        assert!(sizes[6].unwrap() * 2 > 32 * 1024, "{sizes:?}");
        for n in 11..=14 {
            assert_eq!(size(n), None, "{{{n},}}? is past the cap");
        }
    }

    /// #6429, the loop rewrite alone (the evaluator composes it with the capture rewrite of #6069, whose
    /// groups bring the cliff to an upper bound of 7: `tests/jq_cli/i6429.rs` pins that one end to end): a
    /// counted repeat over a body that holds a loop with a group is written copy by copy, and the
    /// text doubles with each count of the upper bound (`{3,7}` 11 KB, `{4,8}` 22 KB, `{5,9}` 45 KB before the
    /// capture rewrite of #6069 adds its own groups), because every copy holds the rest of the repeat twice,
    /// once after the loop's exit and once after another iteration. No smaller spelling exists in the `regex`
    /// crate's syntax, and a larger cap only moves the cliff by one count per doubling, so the repeat past it
    /// keeps its text, which `docs/compliance/jq/limitations.md` records. Pinned here so that it stays a
    /// fallback and never an error.
    #[test]
    fn a_counted_repeat_over_a_loop_with_groups_doubles_and_is_given_up_past_the_cap_6429() {
        let size = |n: u32| {
            rewrite_with_groups(&format!("((?:(b|)*|a)+){{{n},{}}}", n + 4))
                .map(|r| r.pattern.len())
        };
        let (small, large) = (size(3).expect("{3,7} is written"), size(2).expect("{2,6}"));
        assert!(small < 32 * 1024 && large < small, "{large} then {small}");
        assert!(small * 10 >= large * 19, "doubling: {large} then {small}");
        assert_eq!(size(4), None, "{{4,8}} is past the cap");
        assert_eq!(size(8), None);
        // The unbounded and the exact forms are single loops or plain copies, far from the cap.
        assert!(rewrite_with_groups("((?:(b|)*|a)+){4,}").is_some());
        assert!(rewrite_with_groups("((?:(b|)*|a)+){4}").is_some());
    }

    #[test]
    fn a_flag_group_wraps_the_ways_taken_out_of_it() {
        assert_eq!(
            rewrite("(?:(?i:b*)|a)+").as_deref(),
            Some("(?:(?i:b+)*(?:a(?i:b+)*)*?)")
        );
    }

    #[test]
    fn a_nested_loop_is_rewritten_inside_a_capture_group() {
        assert_eq!(
            rewrite("((?:b*|a)+)").as_deref(),
            Some("((?:(?:b+)*(?:a(?:b+)*)*?))")
        );
    }

    /// The byte test never turns away a pattern the rewrite changes: checked over every pattern a
    /// small grammar of groups, alternatives and quantifiers generates from a fixed seed.
    #[test]
    fn the_byte_test_admits_every_pattern_the_rewrite_changes() {
        const ATOMS: &[&str] = &[
            "a", "b", ".", "[ab]", "\\w", "$", "", "^", "\\b", "\\B", "\\A", "\\z", "\\b\\b",
        ];
        const QUANTIFIERS: &[&str] = &[
            "*", "+", "?", "*?", "+?", "??", "{0,}", "{1,}", "{2,}", "{1,2}", "{0,2}", "{2}",
        ];
        let mut seed: u64 = 6106;
        let mut roll = |n: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) as usize % n
        };
        fn generate(depth: u32, roll: &mut dyn FnMut(usize) -> usize) -> String {
            let pick = if depth == 0 { 0 } else { roll(4) };
            match pick {
                0 => ATOMS[roll(ATOMS.len())].into(),
                1 => (0..2 + roll(2))
                    .map(|_| generate(depth - 1, roll))
                    .collect(),
                2 => {
                    let ways: Vec<String> = (0..2 + roll(2))
                        .map(|_| generate(depth - 1, roll))
                        .collect();
                    format!("(?:{})", ways.join("|"))
                }
                _ => {
                    let body = generate(depth - 1, roll);
                    let quantifier = QUANTIFIERS[roll(QUANTIFIERS.len())];
                    let space = if roll(4) == 0 { " " } else { "" };
                    format!("(?:{body}){space}{quantifier}")
                }
            }
        }
        let mut changed = 0;
        for _ in 0..20_000 {
            let extended = if roll(4) == 0 { "(?x)" } else { "" };
            let pattern = format!("{extended}{}", generate(4, &mut roll));
            if rewrite_parsed(&pattern, false, "").is_some() {
                changed += 1;
                assert!(may_need_rewrite(&pattern), "{pattern}");
            }
        }
        assert!(changed > 500, "the grammar reaches the rewrite ({changed})");
        assert!(!may_need_rewrite("(?:a|b)+"));
        assert!(!may_need_rewrite("(?:ab)*c"));
        // #6358: only a way that is nothing but anchors counts, not an anchor around the pattern.
        assert!(!may_need_rewrite("^(?:a|b)+$"));
        assert!(!may_need_rewrite(r"\b(?:a|b)+\b"));
        assert!(!may_need_rewrite(r"(?:\ba|b)+"));
        assert!(may_need_rewrite(r"(a|\b)*"));
        assert!(may_need_rewrite(r"(?:a|\B\B)+"));
        assert!(may_need_rewrite("(a|^)*"));
    }

    /// #6358: a loop with a way that is only an anchor is written from what the anchor is. The forms
    /// are checked against jq in `tests/jq_cli/i6358.rs`; these pin which form each anchor takes.
    #[test]
    fn a_way_that_is_only_an_anchor_is_written_as_its_kind_6358() {
        let written = |pattern: &str| rewrite_with_groups(pattern).map(|r| r.pattern);
        // `\b`: the iterations at heads where it fails first, greedy, then those where it holds, lazy.
        for (pattern, holds) in [
            (r"(\b|a)*", r"(?:\B(a))*(?:\b(a)(?:\B(a))*)*?"),
            (r"(\B|a)*", r"(?:\b(a))*(?:\B(a)(?:\b(a))*)*?"),
            (r"(\A|a)*", "(a)+|)"),
            (r"(^|a)*", "(a)+|)"),
            (r"(\z|a)*", "(a)*(?:("),
            ("($|a)*", "(a)*(?:("),
            // A list of anchors is the one that says most: `\z` here.
            (r"(\b\z|a)*", "(a)*(?:("),
            (r"(\b\b|a)*", r"(?:\B(a))*"),
            // The way may also consume, and what consumes before the anchor loops first.
            (r"(\bb*|a)*", r"(\bb+)*(?:\B(a)(\bb+)*)*"),
            (r"(a|\b|b)*", r"(a)*(?:\B(b)(a)*)*"),
        ] {
            let text = written(pattern).unwrap_or_else(|| panic!("{pattern} is written"));
            assert!(text.contains(holds), "{pattern}: {text}");
        }
        // A flag group's flags reach the anchor, so they reach what stands for it: its complement.
        let flagged = written(r"((?i:\b|a))*").unwrap_or_default();
        assert!(flagged.contains(r"(?i:\B)"), "{flagged}");
        assert!(flagged.contains(r"(?i:\b)"), "{flagged}");
        // An anchor with no complement is wrapped alone, and a switch reaches the later ways.
        assert!(written(r"((?i:\A|a))*").is_some_and(|t| t.contains(r"((?i:a))")));
        let scoped = written(r"((?i)a|\b|b)*").unwrap_or_default();
        assert!(scoped.contains(r"(?i:\B)"), "{scoped}");
        // A lazy repeat that may stop at once keeps its text: `c` is an exit that has already failed.
        assert_eq!(written(r"(\b|a)*?"), None);
        // A repeat that runs once is that one copy, which is what the crate reads the text as.
        assert_eq!(written(r"(\b|a){1}"), None);
        assert_eq!(written(r"(\b|a)?"), None);
        // Counted, it is written: `E` leaves all of it, where the crate's copies would go on.
        assert!(written(r"(\b|a){2}").is_some());
        // Two exits, or an empty way after one, are past what the forms write.
        assert_eq!(written(r"(\A|a|\B)*"), None);
        assert_eq!(written(r"(\b\B|a)*"), None);
        // The shadow spells `\G` as `\b`: it is not one, so nothing writes its complement.
        let shadowed = rewrite(r"(?=x)(?:a|\Gb?|b)*").unwrap_or_default();
        assert!(!shadowed.contains(r"\B"), "{shadowed}");
        // Under `(?m)` `^` and `$` hold at every line, which is neither kind.
        assert_eq!(written(r"(?m)($|a|b)*"), None);
        assert_eq!(written(r"(?m:(^|a|b)*)"), None);
    }

    /// A piece that matches only the empty string is not copied into the rewrite, so a pattern the
    /// crate refuses for it is left as written and keeps its error.
    #[test]
    fn a_pattern_the_crate_refuses_is_left_as_written() {
        assert_eq!(rewrite(r"(?:\p{Foo}{0}|a)+"), None);
        assert_eq!(rewrite(r"(?:b*|\p{Foo}{0}|a)+"), None);
        assert!(rewrite(r"(?:b*|\pL{0}|a)+").is_some());
    }

    /// Under `x`, a comment can stand between a group and its quantifier.
    #[test]
    fn a_comment_before_the_quantifier_still_admits_the_loop() {
        assert_eq!(
            rewrite("(?x)(?:b*|a) #c\n +").as_deref(),
            Some("(?x)(?:(?:b+)*(?:a(?:b+)*)*?)")
        );
    }

    /// Ways are listed only under a repeat: a long run of items that can match nothing outside any
    /// loop costs nothing, and one inside a loop's body gives up on the work budget instead of
    /// rebuilding its ways at every item.
    #[test]
    fn a_long_run_of_nullable_items_is_linear_or_given_up() {
        let outside = format!("{}(?:b*|a)+", "a?".repeat(20_000));
        let rewritten = rewrite(&outside).expect("the loop is rewritten");
        assert!(rewritten.ends_with("(?:(?:b+)*(?:a(?:b+)*)*?)"));
        let inside = format!("(?:{}|b)+", "a?".repeat(20_000));
        assert_eq!(rewrite(&inside), None);
    }

    #[test]
    fn a_rewrite_that_grows_past_its_limit_is_abandoned() {
        let mut pattern = String::from("(?:b*|a)+");
        for _ in 0..12 {
            pattern = format!("(?:{pattern}|c*|d)+");
        }
        assert_eq!(rewrite(&pattern), None);
    }

    #[test]
    fn a_repeated_group_name_is_renamed_for_the_parser_and_read_back_6430() {
        // No repeat: the text is the pattern itself and nothing is renamed.
        let read = parse_distinct_names("(?<n>a)(?<m>b)").unwrap();
        assert_eq!(
            (read.text.as_str(), read.renames.len()),
            ("(?<n>a)(?<m>b)", 0)
        );
        // Each later use is renamed, the first keeps its name, and the rename reads back.
        let read = parse_distinct_names("(?<n>a)|(?<n>b)|(?<n>c)").unwrap();
        assert_eq!(read.text, "(?<n>a)|(?<n__d1>b)|(?<n__d2>c)");
        let mut names = Vec::new();
        capture_names(&read.ast, &mut names);
        let names: Vec<_> = names
            .into_iter()
            .map(|name| name.map(|name| original_name(name, &read.renames)))
            .collect();
        assert_eq!(
            names,
            [Some("n".into()), Some("n".into()), Some("n".into())]
        );
        // A name that already ends like a rename is not taken twice.
        let read = parse_distinct_names("(?<n__d1>a)(?<n>b)(?<n>c)").unwrap();
        assert_eq!(read.text, "(?<n__d1>a)(?<n>b)(?<n__d2>c)");
        // Any other refusal stays one.
        assert!(parse_distinct_names("(?<n>a").is_none());
        // Past the cap the pattern keeps its text: the parse is refused, not answered wrongly.
        let many = "(?<n>a)".repeat(MAX_NAME_RENAMES + 2);
        assert!(parse_distinct_names(&many).is_none());
        let at_cap = "(?<n>a)".repeat(MAX_NAME_RENAMES + 1);
        assert_eq!(
            parse_distinct_names(&at_cap).unwrap().renames.len(),
            MAX_NAME_RENAMES
        );
    }

    #[test]
    fn a_loop_with_groups_in_a_pattern_that_repeats_a_name_is_written_6430() {
        let mut rewritten = 0;
        for pattern in [
            "(?:(?<n>a)|(?<n>a)|())*",
            "(?:(?<n>a)|c|(?<n>b)|(b*))+",
            "(?:b*|(?<n>a)(?<n>b))*",
            "(?:(?<n>a)|b|(?<n>c)|)+",
        ] {
            // Not every pattern needs the group copies; the ones that do must name no group but `n`.
            if let Some(written) = rewrite_with_groups(pattern) {
                rewritten += 1;
                assert!(!written.pattern.contains("?<"), "{pattern}");
                assert!(
                    written.names.iter().flatten().all(|name| &**name == "n"),
                    "{pattern}"
                );
            }
        }
        assert!(rewritten > 0, "no pattern was rewritten with its groups");
    }
}
