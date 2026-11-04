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
//! A body whose ways are not listed here keeps its loop as written: one with a capture group (an
//! empty iteration also sets the group, #6069), a zero-width assertion (its empty way depends on
//! where it stands), or a flag switch.
//!
//! A pattern with syntax only `fancy-regex` reads (a lookaround, an atomic group, a backreference,
//! `\K`, `\G`, a comment) is parsed through `oniguruma_shadow`'s copy of it, which writes that syntax
//! plainly at the same offsets, and every piece's text is still taken from the pattern (#6128).
//! Those constructs are opaque: a body holding one keeps its loop, and a lookaround or atomic
//! group has its own body rewritten. A lazy repeat that must run twice or more is written copy by
//! copy by `Rewriter::lazy_counted`; for an unbounded one over a body with a consuming way before
//! its empty one that form still differs from Oniguruma's on a few patterns (#6128).

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
    /// Not listed: a capture group, an assertion, a flag switch, or something holding one.
    Unknown,
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
    /// The bytes of text built so far, and their bound: a body of many items that can match
    /// nothing rebuilds its listed ways at each item, quadratic in the items.
    work: core::cell::Cell<usize>,
    work_limit: usize,
    changed: bool,
}

/// `pattern` with every greedy, unbounded loop whose body can match nothing before trying a way
/// that consumes text written so that the `regex` crate leaves the loop where Oniguruma does, or
/// `None` when no loop needs it (or the pattern does not parse, or the rewrite grows too large).
pub(crate) fn rewrite(pattern: &str) -> Option<String> {
    if !may_need_rewrite(pattern) {
        return None;
    }
    rewrite_parsed(pattern)
}

fn rewrite_parsed(pattern: &str) -> Option<String> {
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
            .saturating_mul(64)
            .saturating_add(1024)
            .max(32 * 1024),
        folds: may_fold_case(pattern),
        work: core::cell::Cell::new(0),
        work_limit: pattern
            .len()
            .saturating_mul(64)
            .saturating_add(64 * 1024)
            .max(512 * 1024),
        changed: false,
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
        Ast::Repetition(rep) => {
            may_be_empty(&rep.ast)
                || matches!(
                    rep.op.kind,
                    RepetitionKind::ZeroOrOne
                        | RepetitionKind::ZeroOrMore
                        | RepetitionKind::Range(
                            RepetitionRange::AtLeast(0) | RepetitionRange::Bounded(0, _)
                        )
                )
        }
    }
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
        Ast::Literal(lit) => (!(folds && lit.c.is_alphabetic())).then_some(1),
        Ast::Dot(_) | Ast::ClassUnicode(_) | Ast::ClassPerl(_) | Ast::ClassBracketed(_) => Some(1),
        Ast::Assertion(_) | Ast::Flags(_) => None,
        Ast::Group(group) => match &group.kind {
            GroupKind::NonCapturing(flags) if flags.items.is_empty() => onig_ops(&group.ast, folds),
            _ => None,
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
                    GroupKind::NonCapturing(flags) if flags.items.is_empty() => inner = &group.ast,
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
            let (min, max) = match &rep.op.kind {
                RepetitionKind::ZeroOrOne => (0, Some(1)),
                RepetitionKind::ZeroOrMore => (0, None),
                RepetitionKind::OneOrMore => (1, None),
                RepetitionKind::Range(RepetitionRange::Exactly(n)) => {
                    (*n as usize, Some(*n as usize))
                }
                RepetitionKind::Range(RepetitionRange::AtLeast(n)) => (*n as usize, None),
                RepetitionKind::Range(RepetitionRange::Bounded(m, n)) => {
                    (*m as usize, Some(*n as usize))
                }
            };

            let mod_tlen = tlen + if may_be_empty(&rep.ast) { 2 } else { 0 };
            Some(match max {
                None if min <= 1 || tlen.checked_mul(min)? <= ONIG_EXPAND_LIMIT => {
                    let head = if min == 1 && tlen > ONIG_EXPAND_LIMIT {
                        1
                    } else {
                        tlen * min
                    };
                    head + 2 + mod_tlen
                }
                Some(0) => 0,
                Some(max)
                    if rep.greedy
                        && (max == 1 || (tlen + 1).checked_mul(max)? <= ONIG_EXPAND_LIMIT) =>
                {
                    tlen * min + (1 + tlen) * (max - min)
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
/// has a consuming way after the empty one only through a `|` or a lazy quantifier. Whitespace
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
            Ast::Assertion(_) => Ok(Piece {
                text: self.text(self.verbatim(ast.span()).into(), Prec::Atom)?,
                ways: Ways::Unknown,
            }),
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
                let body = self.convert(&rep.ast, true)?;
                let (min, max) = match &rep.op.kind {
                    RepetitionKind::ZeroOrOne => (0, Some(1)),
                    RepetitionKind::ZeroOrMore => (0, None),
                    RepetitionKind::OneOrMore => (1, None),
                    RepetitionKind::Range(RepetitionRange::Exactly(n)) => (*n, Some(*n)),
                    RepetitionKind::Range(RepetitionRange::AtLeast(n)) => (*n, None),
                    RepetitionKind::Range(RepetitionRange::Bounded(m, n)) => (*m, Some(*n)),
                };
                if !rep.greedy && max.is_none() && self.oniguruma_unrolls(&rep.ast, min) {
                    return self.lazy_unrolled(body, min);
                }
                self.repeat(body, min, max, rep.greedy)
            }
            Ast::Alternation(alt) => {
                let items = alt
                    .asts
                    .iter()
                    .map(|a| self.convert(a, listed))
                    .collect::<Result<Vec<_>, _>>()?;
                self.alt(&items, listed)
            }
            Ast::Concat(concat) => {
                let items = concat
                    .asts
                    .iter()
                    .map(|a| self.convert(a, listed))
                    .collect::<Result<Vec<_>, _>>()?;
                self.concat(&items, listed)
            }
        }
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
            GroupKind::CaptureIndex(_) | GroupKind::CaptureName { .. } => Ways::Unknown,
            GroupKind::NonCapturing(flags) if flags.items.is_empty() => body.ways,
            GroupKind::NonCapturing(flags) => {
                if flags.flag_state(Flag::SwapGreed).is_some() {
                    return Err(GiveUp);
                }
                match body.ways {
                    Ways::Nullable { before, after } => {
                        let wrap = |t: Option<Text>| -> Result<Option<Text>, GiveUp> {
                            t.map(|t| self.text(format!("{open}{})", t.text), Prec::Atom))
                                .transpose()
                        };
                        Ways::Nullable {
                            before: wrap(before)?,
                            after: wrap(after)?,
                        }
                    }
                    ways => ways,
                }
            }
        };
        Ok(Piece { text, ways })
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
        for next in rest {
            ways = match (ways, &next.ways) {
                (Ways::Unknown, _) | (_, Ways::Unknown) => Ways::Unknown,
                (Ways::Consuming, _) | (Ways::Nullable { .. }, Ways::Consuming) => Ways::Consuming,
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
                    let mut before = Vec::new();
                    if let Some(a1) = a1 {
                        before.push(self.concat_text(&[&a1, &next.text])?);
                    }
                    before.extend(a2.clone());
                    let mut after = Vec::new();
                    after.extend(b2.clone());
                    if let Some(b1) = b1 {
                        after.push(self.concat_text(&[&b1, &next.text])?);
                    }
                    Ways::Nullable {
                        before: self.alt_text(&before)?,
                        after: self.alt_text(&after)?,
                    }
                }
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
        for item in items {
            if unknown {
                break;
            }
            match &item.ways {
                Ways::Unknown => unknown = true,
                Ways::Consuming if nullable => after.push(item.text.clone()),
                Ways::Consuming => before.push(item.text.clone()),
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
        let text = self.repeat_text(&body.text, min, max, greedy)?;
        let ways = match (&body.ways, min, max, greedy) {
            (Ways::Unknown, ..) => Ways::Unknown,
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
                self.concat(&[body, rest], true)?.ways
            }
            (Ways::Nullable { before, after }, _, None, true) => {
                return self.exit_on_empty(before.clone(), after.clone(), text);
            }
            (Ways::Nullable { before, after }, _, Some(n), true) => {
                return self.bounded(&body, before.clone(), after.clone(), n, text);
            }
            (
                Ways::Nullable {
                    before,
                    after: Some(after),
                },
                ..,
            ) if !greedy && min >= 2 => {
                return self.lazy_counted(before.clone(), after.clone(), min, max);
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
    ) -> Result<Piece, GiveUp> {
        let consuming = |text: Text| Piece {
            text,
            ways: Ways::Consuming,
        };
        let star = before
            .map(|a| self.repeat(consuming(a), 0, None, true))
            .transpose()?;
        let mut parts: Vec<Piece> = star.iter().cloned().collect();
        let Some(b) = after else {
            let mut piece = self.concat(&parts, true)?;
            piece.text = written;
            return Ok(piece);
        };
        let mut iteration = alloc::vec![consuming(b)];
        iteration.extend(star);
        let iteration = self.concat(&iteration, true)?;
        parts.push(self.repeat(iteration, 0, None, false)?);
        let piece = self.concat(&parts, true)?;
        self.changed = true;
        Ok(Piece {
            text: self.text(format!("(?:{})", piece.text.text), Prec::Atom)?,
            ways: piece.ways,
        })
    }

    /// Whether Oniguruma writes the lazy, unbounded `X{min,}?` out as `min` plain copies of `X`
    /// followed by `X*?`, which it does when the body's compiled length times `min` is at most
    /// ten (`QUANTIFIER_EXPAND_LIMIT_SIZE`). A longer body gets a counted loop, which leaves at
    /// the body's empty way even before the minimum, and is what [`Self::lazy_counted`] writes.
    /// `false` when the length cannot be told, which keeps the rewrite.
    fn oniguruma_unrolls(&self, body: &Ast, min: u32) -> bool {
        min >= 2
            && onig_ops(body, self.folds)
                .and_then(|ops| ops.checked_mul(min as usize))
                .is_some_and(|total| total <= ONIG_EXPAND_LIMIT)
    }

    /// `X{min,}?` as Oniguruma unrolls it, `X` `min` times and then `X*?`: the `regex` crate reads
    /// the repeat the same way, so the text stays as written. Only the body's ways are worked out,
    /// as the concatenation of those copies, for a loop around this one.
    fn lazy_unrolled(&mut self, body: Piece, min: u32) -> Result<Piece, GiveUp> {
        let text = self.repeat_text(&body.text, min, None, false)?;
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
        after: Text,
        min: u32,
        max: Option<u32>,
    ) -> Result<Piece, GiveUp> {
        let consuming = |text: Text| Piece {
            text,
            ways: Ways::Consuming,
        };
        let mut rest = match max {
            Some(_) => self.concat(&[], true)?,
            None => {
                let ways: Vec<Text> = before.iter().cloned().chain([after.clone()]).collect();
                let body = self.alt_text(&ways)?.ok_or(GiveUp)?;
                self.repeat(consuming(body), 0, None, false)?
            }
        };
        for k in (1..=max.unwrap_or(min)).rev() {
            let exit = self.concat(&[], true)?;
            let mut ways = Vec::new();
            if k > min {
                ways.push(exit.clone());
            }
            if let Some(a) = &before {
                ways.push(self.concat(&[consuming(a.clone()), rest.clone()], true)?);
            }
            if k <= min {
                ways.push(exit);
            }
            ways.push(self.concat(&[consuming(after.clone()), rest.clone()], true)?);
            rest = self.alt(&ways, true)?;
        }
        self.changed = true;
        Ok(Piece {
            text: self.text(format!("(?:{})", rest.text.text), Prec::Atom)?,
            ways: rest.ways,
        })
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
        let mut loop_rest = body.clone();
        for _ in 1..n {
            let mut ways = Vec::new();
            if let Some(a) = &before {
                ways.push(self.concat(&[consuming(a.clone()), loop_rest.clone()], true)?);
            }
            ways.push(self.concat(&[], true)?);
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
            "(b*|a)+",     // a capture group
            "(?:^|a)+",    // an assertion
            "(?:b*|(a))+", // a capture group in a later way
            "(?U)(?:b*|a)+",
            "(?:(?U:b*)|a)+",
            "(?:a+)*",
            "abc",
            "(?:b*|a",
        ] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
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
        // Not worked out: a capture group, an assertion, a flag switch, a folded letter.
        assert_eq!(ops("(a)"), None);
        assert_eq!(ops("^a"), None);
        assert_eq!(ops("(?i)a"), None);
        assert_eq!(ops("(?i:a)"), None);
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
        // An ignore-case flag may fold the letters into more operations: not worked out.
        assert!(rewrite("(?i)(?:b?|a){2,}?").is_some());
        // The copies a count of six or seven makes are written out in full.
        assert!(rewrite("(?:b*|\\w){6,}?").is_some());
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
        const ATOMS: &[&str] = &["a", "b", ".", "[ab]", "\\w", "$", ""];
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
            if rewrite_parsed(&pattern).is_some() {
                changed += 1;
                assert!(may_need_rewrite(&pattern), "{pattern}");
            }
        }
        assert!(changed > 500, "the grammar reaches the rewrite ({changed})");
        assert!(!may_need_rewrite("(?:a|b)+"));
        assert!(!may_need_rewrite("(?:ab)*c"));
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
}
