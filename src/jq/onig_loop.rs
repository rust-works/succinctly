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
//! where it stands), or a flag switch. So does a lazy repeat that must run twice or more.

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
    source: &'p str,
    /// The longest text a piece may grow to: the rewrite copies a loop's body, and nested loops
    /// copy the copies.
    limit: usize,
    changed: bool,
}

/// `pattern` with every greedy, unbounded loop whose body can match nothing before trying a way
/// that consumes text written so that the `regex` crate leaves the loop where Oniguruma does, or
/// `None` when no loop needs it (or the pattern does not parse, or the rewrite grows too large).
pub(crate) fn rewrite(pattern: &str) -> Option<String> {
    if !has_group_repeat(pattern) {
        return None;
    }
    let ast = ast::parse::Parser::new().parse(pattern).ok()?;
    let mut rewriter = Rewriter {
        source: pattern,
        limit: pattern.len().saturating_mul(16).saturating_add(1024),
        changed: false,
    };
    let piece = rewriter.convert(&ast).ok()?;
    rewriter.changed.then_some(piece.text.text)
}

/// Whether `pattern` may hold a group under a quantifier, the only shape whose body can match
/// nothing (a single character or class always consumes one): a `)` followed, after any
/// whitespace, by `*`, `+` or `{`. A cheap test that spares every other pattern the parse.
fn has_group_repeat(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    bytes.iter().enumerate().any(|(i, &b)| {
        b == b')'
            && bytes[i + 1..]
                .iter()
                .find(|c| !c.is_ascii_whitespace())
                .is_some_and(|c| matches!(c, b'*' | b'+' | b'{'))
    })
}

impl Rewriter<'_> {
    fn text(&self, text: String, prec: Prec) -> Result<Text, GiveUp> {
        if text.len() > self.limit {
            return Err(GiveUp);
        }
        Ok(Text { text, prec })
    }

    fn verbatim(&self, span: &ast::Span) -> &str {
        &self.source[span.start.offset..span.end.offset]
    }

    fn convert(&mut self, ast: &Ast) -> Result<Piece, GiveUp> {
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
            Ast::Group(group) => self.group(group),
            Ast::Repetition(rep) => {
                let body = self.convert(&rep.ast)?;
                let (min, max) = match &rep.op.kind {
                    RepetitionKind::ZeroOrOne => (0, Some(1)),
                    RepetitionKind::ZeroOrMore => (0, None),
                    RepetitionKind::OneOrMore => (1, None),
                    RepetitionKind::Range(RepetitionRange::Exactly(n)) => (*n, Some(*n)),
                    RepetitionKind::Range(RepetitionRange::AtLeast(n)) => (*n, None),
                    RepetitionKind::Range(RepetitionRange::Bounded(m, n)) => (*m, Some(*n)),
                };
                self.repeat(body, min, max, rep.greedy)
            }
            Ast::Alternation(alt) => {
                let items = alt
                    .asts
                    .iter()
                    .map(|a| self.convert(a))
                    .collect::<Result<Vec<_>, _>>()?;
                self.alt(&items)
            }
            Ast::Concat(concat) => {
                let items = concat
                    .asts
                    .iter()
                    .map(|a| self.convert(a))
                    .collect::<Result<Vec<_>, _>>()?;
                self.concat(&items)
            }
        }
    }

    /// A group keeps its own opening and closing text. A capture group's ways are not listed; a
    /// non-capturing one's are its body's, each wrapped in the group's flags when it has any.
    fn group(&mut self, group: &ast::Group) -> Result<Piece, GiveUp> {
        let body = self.convert(&group.ast)?;
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

    fn concat(&mut self, items: &[Piece]) -> Result<Piece, GiveUp> {
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
        let mut ways = first.ways.clone();
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

    fn alt(&mut self, items: &[Piece]) -> Result<Piece, GiveUp> {
        let mut before = Vec::new();
        let mut after = Vec::new();
        let mut nullable = false;
        let mut unknown = false;
        for item in items {
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
                self.concat(&[body, rest])?.ways
            }
            (Ways::Nullable { before, after }, _, None, true) => {
                return self.exit_on_empty(before.clone(), after.clone(), text);
            }
            (Ways::Nullable { before, after }, _, Some(n), true) => {
                return self.bounded(&body, before.clone(), after.clone(), n, text);
            }
            // A lazy repeat that must run twice tries its later copies before leaving at an empty
            // iteration, as Oniguruma does not.
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
            let mut piece = self.concat(&parts)?;
            piece.text = written;
            return Ok(piece);
        };
        let mut iteration = alloc::vec![consuming(b)];
        iteration.extend(star);
        let iteration = self.concat(&iteration)?;
        parts.push(self.repeat(iteration, 0, None, false)?);
        let piece = self.concat(&parts)?;
        self.changed = true;
        Ok(Piece {
            text: self.text(format!("(?:{})", piece.text.text), Prec::Atom)?,
            ways: piece.ways,
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
                ways.push(self.concat(&[consuming(a.clone()), loop_rest.clone()])?);
            }
            ways.push(self.concat(&[])?);
            ways.push(self.concat(&[consuming(b.clone()), loop_rest])?);
            loop_rest = self.alt(&ways)?;
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
            "(?:a|b*)+",  // the empty way comes last
            "(?:b*|a)*?", // lazy
            "(?:a|b*){2}",
            "(?:b*|a){1,2}?",
            "(b*|a)+",     // a capture group
            "(?:^|a)+",    // an assertion
            "(?:b*|(a))+", // a capture group in a later way
            "(?U)(?:b*|a)+",
            "(?:a+)*",
            "abc",
            "(?:b*|a",
        ] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
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

    #[test]
    fn a_rewrite_that_grows_past_its_limit_is_abandoned() {
        let mut pattern = String::from("(?:b*|a)+");
        for _ in 0..12 {
            pattern = format!("(?:{pattern}|c*|d)+");
        }
        assert_eq!(rewrite(&pattern), None);
    }
}
