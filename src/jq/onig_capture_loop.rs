//! Oniguruma's last iteration of a repeat whose body can match empty, written into a pattern for
//! the `regex` crate (#6069).
//!
//! Oniguruma runs a greedy unbounded repeat (`*`, `+`, `{0,}`, `{1,}`) until an iteration matches
//! empty, and that empty iteration is the one its captures keep: `"ab" | match("(a*)*b")` has the
//! group at `[1,0,""]` in jq. The `regex` crate never makes an empty iteration (the thread meets a
//! loop state it has already visited at that position and is dropped), so it reports the last
//! non-empty one, `[0,1,"a"]`. Fixing the capture after the match cannot work, because where the
//! loop ends depends on backtracking from the rest of the pattern (`"aab" | match("(a*)*ab")` gives
//! a character back and is `[1,0,""]` in jq).
//!
//! So the pattern says it instead. Without a backreference, an Oniguruma match of such a loop always
//! ends with an empty iteration, and that iteration is the body's *first* way to match empty: the
//! rest of the pattern runs at the same position whichever empty way it is, so a later one would
//! fail where the first did. [`rewrite`] therefore writes `X*` as
//!
//! ```text
//! (?:X*E)      E = X with every character, class and `.` replaced by one that never matches
//! ```
//!
//! The loop still decides where the repeat ends, and `E`, which can only match empty and does so
//! the way `X` would first, is the final iteration. A group inside `X` now has two copies: the one
//! in the loop keeps what earlier iterations captured (Oniguruma keeps a group an iteration skipped:
//! `"abc" | match("((a)|b*)*c")` has `(a)` at `[0,1]`), and the one in `E` holds what the empty
//! iteration set. A group reads the last of its copies that the match set
//! ([`LoopRewrite::copies`]). The repeated group itself is captured in `E` only, since `E` always
//! matches and so always has the last word.
//!
//! The crate's capture engine pays for every state and slot over the length of a match, so `E` is
//! kept small: a part of it with no group only says whether it can match empty there, and is written
//! as nothing when it always can and as one failing class when it never can (`(\w*\s*)*` ends with
//! `()`). A part that sets a flag is kept, as `(?m)` changes what a later `$` means.
//!
//! Only a body that matches empty at every position is rewritten: one whose every empty way passes an
//! assertion (`(a|$)*`) matches empty at some positions and not others, so `E` could fail where
//! Oniguruma's loop would simply stop. An assertion on a body that also has a way around it stays in
//! `E` wherever a group needs it, since which empty way succeeds depends on the position only.
//! Neither is a lazy repeat rewritten (`(a*)*?b` already agrees) nor a counted one from `{2,}` up,
//! which the crate expands into copies whose last one matches empty. A projected loop (`X*` inside
//! some `E`) is written as `E` alone: its loop copy could only make empty iterations, which the crate
//! drops, so it would never set a group. That keeps the rewritten pattern linear in the nesting depth
//! instead of doubling per level; [`MAX_REWRITTEN_LEN`] bounds it all the same.
//!
//! `test` and `is_match` run the rewritten pattern too, and a second, plain regex for them is not
//! worth its compile (#6175). They never read a capture, and the crate's search for a match without
//! captures does not walk the extra groups: 40 `test`s of `(\w*\s*)*Q` over a 10 MB string (no match,
//! so every byte is scanned) retired 7,647M instructions on the rewritten pattern and 7,653M with
//! the plain one compiled beside it, the 6M being that second compile. `(\w*\s*)*c`, `((\w+)\s*|x*)*Q`
//! and a four-deep nest scan the same, and 300,000 short strings differ by less than the run
//! varies by itself (about 2%). Apple M5 Max, retired instructions, binaries interleaved; x86_64 was
//! not measured. The 30% above is the capture engine, which `match` and `capture` use and `test`
//! does not.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use regex_syntax::ast::{
    self, Ast, Flag, FlagsItemKind, GroupKind, RepetitionKind, RepetitionRange,
};

/// A class that matches no character: `E`'s stand-in for every character, class and `.` of `X`.
const NEVER: &str = r"[^\s\S]";

/// The longest rewritten pattern [`rewrite`] writes. A longer one keeps the pattern as it was,
/// with its old captures, rather than hand the crate a pattern it may refuse for size.
const MAX_REWRITTEN_LEN: usize = 1 << 16;

/// A pattern rewritten by [`rewrite`], with what reading its groups back takes.
pub(crate) struct LoopRewrite {
    /// The pattern to compile.
    pub(crate) pattern: String,
    /// For each group of the original pattern (0 is the whole match), the groups of [`pattern`]
    /// that hold a copy of it, in the order they open. The group's value is the last one set.
    ///
    /// [`pattern`]: Self::pattern
    pub(crate) copies: Box<[Box<[usize]>]>,
    /// The original pattern's group names, by group: every copy in [`pattern`](Self::pattern) is
    /// unnamed, since the crate refuses a name used twice.
    pub(crate) names: Box<[Option<Box<str>>]>,
    /// How many groups [`pattern`](Self::pattern) has, the whole match included.
    pub(crate) compiled_groups: usize,
}

/// `pattern` with every greedy unbounded repeat of a group that can match empty written so that the
/// `regex` crate reports Oniguruma's captures (see the module docs), or `None` when no repeat
/// needs it, the pattern does not parse, it uses `U` (which swaps what greedy means), or the result
/// would pass [`MAX_REWRITTEN_LEN`].
pub(crate) fn rewrite(pattern: &str) -> Option<LoopRewrite> {
    if !pattern.contains('(') || !pattern.contains(['*', '+', '{']) {
        return None;
    }
    let ast = ast::parse::Parser::new().parse(pattern).ok()?;
    if swaps_greed(&ast) || !has_candidate(&ast) {
        return None;
    }
    let mut names = vec![None];
    collect_names(&ast, &mut names);
    let mut emitter = Emitter {
        src: pattern,
        out: String::with_capacity(pattern.len() * 2),
        copies: vec![Vec::new(); names.len()],
        next: 1,
    };
    emitter.copies[0].push(0);
    emitter.emit(&ast, false).ok()?;
    Some(LoopRewrite {
        pattern: emitter.out,
        copies: emitter
            .copies
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect(),
        names: names.into_boxed_slice(),
        compiled_groups: emitter.next,
    })
}

/// The pattern grew past [`MAX_REWRITTEN_LEN`].
struct TooLong;

struct Emitter<'p> {
    src: &'p str,
    out: String,
    copies: Vec<Vec<usize>>,
    next: usize,
}

impl Emitter<'_> {
    fn push(&mut self, s: &str) -> Result<(), TooLong> {
        self.out.push_str(s);
        if self.out.len() > MAX_REWRITTEN_LEN {
            return Err(TooLong);
        }
        Ok(())
    }

    fn text(&self, span: &ast::Span) -> &str {
        &self.src[span.start.offset..span.end.offset]
    }

    /// Write `ast`, or with `projected` its `E`: every character, class and `.` never matches.
    fn emit(&mut self, ast: &Ast, projected: bool) -> Result<(), TooLong> {
        // In `E` a part with no group matters only by whether it can match empty where it stands,
        // so one that always can is written as nothing and one that never can as one failing
        // class. Fewer states make the capture engine faster on a long match (#6069: a 1.4 MB
        // match of `(\w*\s*)*c` took 30% longer with the part projected class by class). A part
        // that sets a flag is kept whole, since the flag reaches what follows it.
        if projected && !has_group(ast) && !sets_flags(ast) {
            if always_matches_empty(ast) {
                return Ok(());
            }
            if !may_match_empty(ast) {
                return self.push(NEVER);
            }
        }
        match ast {
            Ast::Empty(_) => Ok(()),
            Ast::Flags(set) => {
                let text = self.text(&set.span);
                let text = String::from(text);
                self.push(&text)
            }
            Ast::Literal(_)
            | Ast::Dot(_)
            | Ast::ClassUnicode(_)
            | Ast::ClassPerl(_)
            | Ast::ClassBracketed(_)
            | Ast::Assertion(_) => {
                let text = String::from(self.text(ast.span()));
                self.push(&text)
            }
            Ast::Group(group) => {
                match &group.kind {
                    GroupKind::CaptureIndex(index) => {
                        self.record(*index as usize);
                        self.push("(")?;
                    }
                    GroupKind::CaptureName { name, .. } => {
                        self.record(name.index as usize);
                        self.push("(")?;
                    }
                    GroupKind::NonCapturing(flags) => {
                        let open = alloc::format!("(?{}:", self.text(&flags.span));
                        self.push(&open)?;
                    }
                }
                self.emit(&group.ast, projected)?;
                self.push(")")
            }
            Ast::Repetition(rep) if is_candidate(rep) => {
                self.push("(?:")?;
                if !projected {
                    // The repeated group's own loop copy is never read: `E` always matches, so
                    // its copy of the group is always set and always the last. Leaving it
                    // uncaptured keeps the crate's capture slots at the pattern's count for the
                    // common `(a*)*`, which a long match pays for per slot.
                    match &*rep.ast {
                        Ast::Group(group) if group.capture_index().is_some() => {
                            self.push("(?:")?;
                            self.emit(&group.ast, false)?;
                            self.push(")")?;
                        }
                        body => self.emit(body, false)?,
                    }
                    self.push("*")?;
                }
                self.emit(&rep.ast, true)?;
                self.push(")")
            }
            Ast::Repetition(rep) => {
                self.emit(&rep.ast, projected)?;
                let op = String::from(self.text(&rep.op.span));
                self.push(&op)
            }
            Ast::Alternation(alt) => {
                for (i, branch) in alt.asts.iter().enumerate() {
                    if i > 0 {
                        self.push("|")?;
                    }
                    self.emit(branch, projected)?;
                }
                Ok(())
            }
            Ast::Concat(concat) => {
                for item in &concat.asts {
                    self.emit(item, projected)?;
                }
                Ok(())
            }
        }
    }

    /// The next group opened is a copy of the original pattern's group `original`.
    fn record(&mut self, original: usize) {
        self.copies[original].push(self.next);
        self.next += 1;
    }
}

/// A greedy `*`, `+`, `{0,}` or `{1,}` of a body that holds a group and matches empty at every
/// position: the repeats whose captures the crate and Oniguruma report differently.
fn is_candidate(rep: &ast::Repetition) -> bool {
    let unbounded = matches!(
        rep.op.kind,
        RepetitionKind::ZeroOrMore
            | RepetitionKind::OneOrMore
            | RepetitionKind::Range(RepetitionRange::AtLeast(0 | 1))
    );
    unbounded && rep.greedy && has_group(&rep.ast) && always_matches_empty(&rep.ast)
}

fn has_candidate(ast: &Ast) -> bool {
    match ast {
        Ast::Repetition(rep) => is_candidate(rep) || has_candidate(&rep.ast),
        Ast::Group(group) => has_candidate(&group.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(has_candidate),
        Ast::Concat(concat) => concat.asts.iter().any(has_candidate),
        _ => false,
    }
}

/// The fewest times `rep` repeats its body.
fn min_repeats(rep: &ast::Repetition) -> u32 {
    match &rep.op.kind {
        RepetitionKind::ZeroOrOne | RepetitionKind::ZeroOrMore => 0,
        RepetitionKind::OneOrMore => 1,
        RepetitionKind::Range(
            RepetitionRange::Exactly(n)
            | RepetitionRange::AtLeast(n)
            | RepetitionRange::Bounded(n, _),
        ) => *n,
    }
}

/// Whether `ast` matches empty wherever it is tried: some way through it consumes nothing and passes
/// no assertion, which can fail.
fn always_matches_empty(ast: &Ast) -> bool {
    match ast {
        Ast::Empty(_) | Ast::Flags(_) => true,
        Ast::Assertion(_)
        | Ast::Literal(_)
        | Ast::Dot(_)
        | Ast::ClassUnicode(_)
        | Ast::ClassPerl(_)
        | Ast::ClassBracketed(_) => false,
        Ast::Repetition(rep) => min_repeats(rep) == 0 || always_matches_empty(&rep.ast),
        Ast::Group(group) => always_matches_empty(&group.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(always_matches_empty),
        Ast::Concat(concat) => concat.asts.iter().all(always_matches_empty),
    }
}

/// Whether `ast` can match empty somewhere: some way through it consumes nothing, assertions
/// allowed.
fn may_match_empty(ast: &Ast) -> bool {
    match ast {
        Ast::Empty(_) | Ast::Flags(_) | Ast::Assertion(_) => true,
        Ast::Literal(_)
        | Ast::Dot(_)
        | Ast::ClassUnicode(_)
        | Ast::ClassPerl(_)
        | Ast::ClassBracketed(_) => false,
        Ast::Repetition(rep) => min_repeats(rep) == 0 || may_match_empty(&rep.ast),
        Ast::Group(group) => may_match_empty(&group.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(may_match_empty),
        Ast::Concat(concat) => concat.asts.iter().all(may_match_empty),
    }
}

/// Whether `ast` holds a flag-setting item (`(?m)`), whose flags reach past it in its group.
fn sets_flags(ast: &Ast) -> bool {
    match ast {
        Ast::Flags(_) => true,
        Ast::Group(group) => sets_flags(&group.ast),
        Ast::Repetition(rep) => sets_flags(&rep.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(sets_flags),
        Ast::Concat(concat) => concat.asts.iter().any(sets_flags),
        _ => false,
    }
}

fn has_group(ast: &Ast) -> bool {
    match ast {
        Ast::Group(group) => group.capture_index().is_some() || has_group(&group.ast),
        Ast::Repetition(rep) => has_group(&rep.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(has_group),
        Ast::Concat(concat) => concat.asts.iter().any(has_group),
        _ => false,
    }
}

/// Whether `U` is set anywhere: it makes `*` lazy and `*?` greedy, so `rep.greedy` would not say
/// which the crate runs.
fn swaps_greed(ast: &Ast) -> bool {
    let has_u = |flags: &ast::Flags| {
        flags
            .items
            .iter()
            .any(|item| matches!(item.kind, FlagsItemKind::Flag(Flag::SwapGreed)))
    };
    match ast {
        Ast::Flags(set) => has_u(&set.flags),
        Ast::Group(group) => {
            matches!(&group.kind, GroupKind::NonCapturing(flags) if has_u(flags))
                || swaps_greed(&group.ast)
        }
        Ast::Repetition(rep) => swaps_greed(&rep.ast),
        Ast::Alternation(alt) => alt.asts.iter().any(swaps_greed),
        Ast::Concat(concat) => concat.asts.iter().any(swaps_greed),
        _ => false,
    }
}

/// Every group's name, by group index (`None` for an unnamed one).
fn collect_names(ast: &Ast, names: &mut Vec<Option<Box<str>>>) {
    match ast {
        Ast::Group(group) => {
            let slot = match &group.kind {
                GroupKind::CaptureIndex(index) => Some((*index as usize, None)),
                GroupKind::CaptureName { name, .. } => {
                    Some((name.index as usize, Some(Box::from(name.name.as_str()))))
                }
                GroupKind::NonCapturing(_) => None,
            };
            if let Some((index, name)) = slot {
                if names.len() <= index {
                    names.resize(index + 1, None);
                }
                names[index] = name;
            }
            collect_names(&group.ast, names);
        }
        Ast::Repetition(rep) => collect_names(&rep.ast, names),
        Ast::Alternation(alt) => alt.asts.iter().for_each(|a| collect_names(a, names)),
        Ast::Concat(concat) => concat.asts.iter().for_each(|a| collect_names(a, names)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewritten(pattern: &str) -> Option<String> {
        rewrite(pattern).map(|rw| rw.pattern)
    }

    #[test]
    fn rewrites_a_repeat_of_a_group_that_can_match_empty() {
        assert_eq!(rewritten("(a*)*b").as_deref(), Some(r"(?:(?:a*)*())b"));
        assert_eq!(rewritten("(a*)+b").as_deref(), Some(r"(?:(?:a*)*())b"));
        assert_eq!(rewritten("(a?){0,}").as_deref(), Some(r"(?:(?:a?)*())"));
        assert_eq!(
            rewritten("(?:b*|(a))+").as_deref(),
            Some(r"(?:(?:b*|(a))*(?:|([^\s\S])))")
        );
    }

    #[test]
    fn leaves_every_other_repeat_alone() {
        // No group, a body that cannot match empty, a lazy repeat, a counted repeat from 2 up,
        // a bounded one, an assertion in the body, and `U`.
        for pattern in [
            "a*b",
            "(?:a*)*b",
            "(a+)*b",
            "(a*)*?b",
            "(a*){2,}b",
            "(a*){0,5}b",
            "(a|$)*",
            r"(a|\b)*b",
            "(?U)(a*)*b",
            "plain",
            "(a)",
        ] {
            assert_eq!(rewritten(pattern), None, "{pattern}");
        }
    }

    #[test]
    fn maps_every_group_to_its_copies_and_keeps_the_names() {
        let rw = rewrite("(?<x>(a)|b*)*(c)").expect("rewritten");
        // `x`'s own loop copy is left uncaptured: the last iteration's copy always wins.
        assert_eq!(rw.pattern, r"(?:(?:(a)|b*)*(([^\s\S])|))(c)");
        let copies: Vec<&[usize]> = rw.copies.iter().map(|c| &c[..]).collect();
        assert_eq!(copies, [&[0][..], &[2], &[1, 3], &[4]]);
        assert_eq!(
            rw.names.iter().map(|n| n.as_deref()).collect::<Vec<_>>(),
            [None, Some("x"), None, None]
        );
        assert_eq!(rw.compiled_groups, 5);
    }

    #[test]
    fn a_nested_repeat_is_projected_once_not_doubled() {
        assert_eq!(
            rewritten("((a*)*)*").as_deref(),
            Some(r"(?:(?:(?:(?:a*)*()))*((?:())))")
        );
    }

    #[test]
    fn the_last_iteration_keeps_only_what_a_group_or_a_flag_needs() {
        // No group in the body: the last iteration is the empty group, whichever empty way the
        // body has (`\b` where it holds, `a*` where not).
        assert_eq!(
            rewritten(r"(\b|a*)*b").as_deref(),
            Some(r"(?:(?:\b|a*)*())b")
        );
        // A part that cannot match empty fails as one class; one that sets a flag is kept,
        // since `(?m)` changes what the `$` after it means.
        assert_eq!(
            rewritten(r"(ab(c)|((?m)$|d*))*").as_deref(),
            Some(r"(?:(?:ab(c)|((?m)$|d*))*([^\s\S][^\s\S]([^\s\S])|((?m)$|)))")
        );
    }

    #[test]
    fn every_rewrite_compiles_with_the_groups_it_counted() {
        for pattern in [
            "(a*)*b",
            "((a*)*)*",
            "(?<x>(a)|b*)*(c)",
            r"(?x)( a* | \  )+",
            r"(?i)(\x41*?|[b-c]\.)*",
            r"(\p{L}*|\d)*",
        ] {
            let rw = rewrite(pattern).expect("rewritten");
            let re = regex::Regex::new(&rw.pattern).expect("compiles");
            assert_eq!(re.captures_len(), rw.compiled_groups, "{pattern}");
            assert_eq!(
                rw.copies.iter().map(|c| c.len()).sum::<usize>(),
                rw.compiled_groups
            );
        }
    }

    #[test]
    fn keeps_flags_escapes_and_lazy_inner_repeats_verbatim() {
        assert_eq!(
            rewritten(r"(?i)(\x41*?|[b-c]\.)*").as_deref(),
            Some(r"(?i)(?:(?:\x41*?|[b-c]\.)*())")
        );
        assert_eq!(
            rewritten("(?s:(.*))*").as_deref(),
            Some(r"(?:(?s:(.*))*(?s:()))")
        );
    }

    #[test]
    fn gives_up_past_the_length_cap() {
        let deep = "(a*)*".repeat(MAX_REWRITTEN_LEN / 4);
        assert!(rewrite(&deep).is_none());
    }
}
