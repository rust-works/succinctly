//! Oniguruma's multi-character case folding, written into a jq pattern for the `regex` crate
//! (#6225).
//!
//! Under the `i` option Oniguruma folds with the whole of Unicode's `CaseFolding.txt`, including
//! the entries that fold one character to two or three: `ß` is `ss`, `ﬁ` is `fi`, `ﬃ` is `ffi`.
//! The `regex` crate folds one character to one only, so `"SS" | test("ß"; "i")` was `false`.
//!
//! What Oniguruma does is not "the folded text is equal"; it is an expansion of the pattern, done
//! once when it is compiled (`unravel_case_fold_string` in `regcomp.c`, v6.9.8, which jq 1.7.1
//! links), and this module is a port of that expansion:
//!
//! * A **string** is a run of literal characters with nothing between them but a non-capturing
//!   group that is not repeated, an inline option that leaves `i` as it was, a comment or (under
//!   `x`) blanks. A quantifier takes the *last* character off its run, so `ß+` repeats `ß` and
//!   `ssß+` is the run `ss` then the repeated `ß`; `{1}` is no quantifier, so `s{1}s` is `ss`.
//! * The string is walked from the left. At each character the library asks for the longest fold
//!   that starts there: the next three characters (each folded one-to-one first) as an entry of
//!   `FOLDS3`; failing that, the next two as an entry of `FOLDS2`; failing that, the character
//!   alone, which expands only if it is itself a character that folds to several (`ß`).
//!   A fold found there is an alternation: the characters that fold to it (`ß`, `ẞ` for `ss`) and
//!   every case spelling of the fold (`ss`, `sS`, `Ss`, `SS`, `ſs`, ...). The walk then moves
//!   past the characters the fold covered, so `sss` is `ss` then `s`, and `"sß"` is not a match
//!   for it. Every other character is left to the `regex` crate's own one-to-one folding.
//! * A **bracketed class** gains, for each such fold, an alternative spelling it, when the class
//!   holds a character that folds to it (`[ß]` and `[à-ÿ]` do, `[s]` and `[^ß]` do not): the class
//!   `C` becomes `(?:C|ss)`.
//! * Inside a look-behind the library expands one character at a time and none of this applies.
//!
//! The tables are the library's own, in [`onig_fold_data`](super::onig_fold_data).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use regex_syntax::ast::{self, Ast, Flag, FlagsItemKind, GroupKind};

use super::eval::JQ_WORD_CLASS;
use super::onig_fold_data::{FOLDS2, FOLDS3, FOLD_KEY};

/// The `i` and `x` options in force at a point of the pattern.
#[derive(Clone, Copy)]
struct Options {
    ignore_case: bool,
    extended: bool,
}

impl Options {
    fn apply(&mut self, flags: &ast::Flags) {
        let mut on = true;
        for item in &flags.items {
            match item.kind {
                FlagsItemKind::Negation => on = false,
                FlagsItemKind::Flag(Flag::CaseInsensitive) => self.ignore_case = on,
                FlagsItemKind::Flag(Flag::IgnoreWhitespace) => self.extended = on,
                FlagsItemKind::Flag(_) => {}
            }
        }
    }
}

/// A literal character of a run, and where it stands in the pattern.
#[derive(Clone, Copy)]
struct Lit {
    c: char,
    start: usize,
    end: usize,
}

/// What a fold found at some place of a string covers.
struct Fold {
    /// How many characters of the string it covers.
    len: usize,
    /// The alternation that stands for them.
    text: String,
}

/// `pattern` with every literal and bracketed class that Oniguruma's multi-character case folding
/// reaches written as the alternation it expands to, or `None` when none is (or the pattern does
/// not parse).
pub(crate) fn rewrite(pattern: &str) -> Option<String> {
    if !may_need_rewrite(pattern) {
        return None;
    }
    // A pattern with Oniguruma syntax only `fancy-regex` reads is parsed through a copy that
    // writes that syntax plainly at the same offsets (see `onig_loop`).
    let shadow;
    let (ast, opaque) = match ast::parse::Parser::new().parse(pattern) {
        Ok(ast) => (ast, &[][..]),
        Err(_) => {
            shadow = super::eval::oniguruma_shadow(pattern)?;
            let ast = ast::parse::Parser::new().parse(&shadow.text).ok()?;
            (ast, shadow.opaque.as_slice())
        }
    };
    let mut walker = Walker {
        source: pattern,
        opaque,
        edits: Vec::new(),
    };
    let mut options = Options {
        ignore_case: false,
        extended: false,
    };
    walker.visit(&ast, &mut options, false);
    if walker.edits.is_empty() {
        return None;
    }
    let mut edits = walker.edits;
    edits.sort_by_key(|edit| edit.0);
    let mut out = String::with_capacity(pattern.len() + 64);
    let mut from = 0;
    for (start, end, text) in &edits {
        if *start < from {
            return None;
        }
        out.push_str(&pattern[from..*start]);
        out.push_str(text);
        from = *end;
    }
    out.push_str(&pattern[from..]);
    Some(out)
}

/// Whether `pattern` may hold something [`rewrite`] changes, judged from its bytes so that the
/// patterns that cannot are spared the parse: it must be able to run under `i` (an inline option
/// that sets it, which is where the modifier lands too) and must hold a character of the tables,
/// two letters that spell an ASCII fold (`ss`, `ff`, `fi`, `fl`, `st`), a bracketed class that
/// may hold one, or a hexadecimal escape or a `#`.
fn may_need_rewrite(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();
    let inline_i = pattern.match_indices("(?").any(|(at, _)| {
        let run = &bytes[at + 2..];
        let n = run
            .iter()
            .take_while(|b| b.is_ascii_alphabetic() || **b == b'-')
            .count();
        run[..n].contains(&b'i') && matches!(run.get(n), Some(b')' | b':'))
    });
    if !inline_i {
        return false;
    }
    // A `#` is a comment under `x`, whose words are not the letters of the string around it.
    if !pattern.is_ascii() || pattern.contains("\\x") || pattern.contains('#') {
        return true;
    }
    if pattern.contains('[') && pattern.contains('\\') {
        return true;
    }
    ascii_fold_pair(bytes)
}

/// Whether the letters of `bytes`, taken with whatever stands between them dropped and the letters
/// of an option group (`(?i)`) skipped, hold two that spell an ASCII fold: `ss`, `ff`, `fi`, `fl`
/// or `st`. A repeat or group between them is why the separators are dropped: `(?:s)s` is the
/// string `ss`.
fn ascii_fold_pair(bytes: &[u8]) -> bool {
    let mut prev = 0u8;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' {
            // An escape's letter (`\d`) is not a letter of the string.
            i += 2;
            prev = 0;
            continue;
        }
        if b == b'(' && bytes.get(i + 1) == Some(&b'?') {
            i += 2;
            while bytes
                .get(i)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'-')
            {
                i += 1;
            }
            continue;
        }
        if b.is_ascii_alphabetic() {
            let b = b.to_ascii_lowercase();
            if matches!((prev, b), (b's', b's' | b't') | (b'f', b'f' | b'i' | b'l')) {
                return true;
            }
            prev = b;
        }
        i += 1;
    }
    false
}

struct Walker<'a> {
    source: &'a str,
    /// Offsets of the constructs the parsed copy reads as something they are not: a lookaround,
    /// atomic or comment group (read as a plain group) and a backreference (read as a literal).
    opaque: &'a [usize],
    /// `(start, end, replacement)` over the pattern, each a range of source text.
    edits: Vec<(usize, usize, String)>,
}

impl Walker<'_> {
    fn verbatim(&self, span: &ast::Span) -> &str {
        &self.source[span.start.offset..span.end.offset]
    }

    fn is_opaque(&self, span: &ast::Span) -> bool {
        self.opaque.binary_search(&span.start.offset).is_ok()
    }

    /// Walk `ast`, whose options are `options`. A flag set (`(?i)`) changes `options` for the rest
    /// of the enclosing group.
    fn visit(&mut self, ast: &Ast, options: &mut Options, look_behind: bool) {
        match ast {
            Ast::Literal(lit) if !self.is_opaque(&lit.span) => {
                self.expand_run(&[lit_of(lit)], *options, look_behind);
            }
            Ast::Flags(set) => options.apply(&set.flags),
            Ast::Concat(concat) => {
                let mut run = Vec::new();
                let mut run_options = *options;
                self.sequence(
                    concat.asts.iter(),
                    true,
                    options,
                    look_behind,
                    &mut run,
                    &mut run_options,
                );
                self.expand_run(&run, run_options, look_behind);
            }
            Ast::Alternation(alt) => {
                for branch in &alt.asts {
                    self.visit(branch, options, look_behind);
                }
            }
            Ast::Repetition(rep) => self.visit(&rep.ast, options, look_behind),
            Ast::Group(group) => self.group(group, *options, look_behind),
            Ast::ClassBracketed(class) => self.class(class, *options, look_behind),
            _ => {}
        }
    }

    fn group(&mut self, group: &ast::Group, options: Options, look_behind: bool) {
        let text = self.verbatim(&group.span);
        // A comment holds text, not pattern.
        if text.starts_with("(?#") {
            return;
        }
        let mut inner = options;
        let mut look_behind = look_behind;
        if let GroupKind::NonCapturing(flags) = &group.kind {
            inner.apply(flags);
        }
        if self.is_opaque(&group.span) && (text.starts_with("(?<=") || text.starts_with("(?<!")) {
            look_behind = true;
        }
        self.visit(&group.ast, &mut inner, look_behind);
    }

    /// Feed `items`, the parts of a sequence, to the string being collected in `run`. A literal
    /// extends it; a non-capturing group that is not repeated is part of the sequence; anything
    /// else ends it (the string is then expanded) and is walked on its own. A repeated literal is
    /// a string of its own, since the quantifier binds the last character only.
    ///
    /// `head` is whether the first of `items` starts its branch. The library splices a group's
    /// parts into the sequence around it only from the second place on: a group that starts its
    /// branch and holds anything but literals is a list of its own, and the strings inside it do
    /// not join those outside (`(?:a\d)s`).
    fn sequence<'x>(
        &mut self,
        items: impl Iterator<Item = &'x Ast>,
        head: bool,
        options: &mut Options,
        look_behind: bool,
        run: &mut Vec<Lit>,
        run_options: &mut Options,
    ) {
        // An option set (`(?i)`) takes the rest of the branch as its body, which starts anew.
        let mut head_now = head;
        for item in items {
            let at_head = head_now;
            head_now = match item {
                Ast::Flags(_) => true,
                Ast::Group(group) if self.verbatim(&group.span).starts_with("(?#") => head_now,
                _ => false,
            };
            match item {
                Ast::Literal(lit) if !self.is_opaque(&lit.span) => {
                    if run_options.ignore_case != options.ignore_case {
                        self.expand_run(run, *run_options, look_behind);
                        run.clear();
                    }
                    if run.is_empty() {
                        *run_options = *options;
                    }
                    run.push(lit_of(lit));
                }
                Ast::Flags(set) => options.apply(&set.flags),
                // `{1}` repeats nothing: the library drops it, and what it held stays in the string.
                Ast::Repetition(rep) if is_once(rep) => {
                    self.sequence(
                        core::iter::once(&*rep.ast),
                        at_head,
                        options,
                        look_behind,
                        run,
                        run_options,
                    );
                }
                // A comment is skipped where the pattern is read, so it does not end a string.
                Ast::Group(group) if self.verbatim(&group.span).starts_with("(?#") => {}
                Ast::Group(group)
                    if matches!(&group.kind, GroupKind::NonCapturing(flags) if flags.items.is_empty())
                        && !self.is_opaque(&group.span)
                        && !matches!(*group.ast, Ast::Alternation(_))
                        && (!at_head || self.is_plain_string(&group.ast)) =>
                {
                    // Options set inside the group end with it.
                    let mut inner = *options;
                    match &*group.ast {
                        Ast::Concat(concat) => {
                            self.sequence(
                                concat.asts.iter(),
                                true,
                                &mut inner,
                                look_behind,
                                run,
                                run_options,
                            );
                        }
                        other => {
                            // A branch of one node passes it up as it is, a list included.
                            self.sequence(
                                core::iter::once(other),
                                at_head,
                                &mut inner,
                                look_behind,
                                run,
                                run_options,
                            );
                        }
                    }
                }
                other => {
                    self.expand_run(run, *run_options, look_behind);
                    run.clear();
                    self.visit(other, options, look_behind);
                }
            }
        }
    }

    /// Whether `ast` is only literals, which the library reads as one string node.
    fn is_plain_string(&self, ast: &Ast) -> bool {
        match ast {
            Ast::Literal(lit) => !self.is_opaque(&lit.span),
            Ast::Concat(concat) => concat
                .asts
                .iter()
                .all(|item| matches!(item, Ast::Literal(lit) if !self.is_opaque(&lit.span))),
            Ast::Group(group) => {
                matches!(&group.kind, GroupKind::NonCapturing(flags) if flags.items.is_empty())
                    && !self.is_opaque(&group.span)
                    && self.is_plain_string(&group.ast)
            }
            _ => false,
        }
    }

    /// Expand one string (see the module documentation).
    fn expand_run(&mut self, run: &[Lit], options: Options, look_behind: bool) {
        if run.is_empty() || !options.ignore_case || look_behind {
            return;
        }
        let chars: Vec<char> = run.iter().map(|lit| lit.c).collect();
        let mut at = 0;
        while at < chars.len() {
            match fold_at(&chars[at..]) {
                Some(fold) => {
                    let first = &run[at];
                    self.edits.push((first.start, first.end, fold.text));
                    for lit in &run[at + 1..at + fold.len] {
                        self.edits.push((lit.start, lit.end, String::new()));
                    }
                    at += fold.len;
                }
                None => at += 1,
            }
        }
    }

    /// A bracketed class gains an alternative for each multi-character fold of a character it
    /// holds (a class under `i`; a negated class never does).
    fn class(&mut self, class: &ast::ClassBracketed, options: Options, look_behind: bool) {
        if !options.ignore_case || look_behind || class.negated {
            return;
        }
        let text = self.verbatim(&class.span);
        if text.is_ascii() && !text.contains('\\') {
            return;
        }
        // The class the translation writes for `\w` is not a class of the pattern: Oniguruma reads
        // `\w` as a character type, and adds nothing to it.
        if text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) == Some(JQ_WORD_CLASS) {
            return;
        }
        let x = if options.extended { "x" } else { "" };
        let Ok(re) = regex::Regex::new(&format!("^(?i{x}:{text})$")) else {
            return;
        };
        let mut buf = [0u8; 4];
        let mut alternatives: Vec<String> = Vec::new();
        for (fold, unfolds) in FOLDS2
            .iter()
            .map(|(fold, unfolds)| (&fold[..], *unfolds))
            .chain(FOLDS3.iter().map(|(fold, unfolds)| (&fold[..], *unfolds)))
        {
            if unfolds.iter().any(|c| re.is_match(c.encode_utf8(&mut buf))) {
                let spelling = spell(fold);
                if !alternatives.contains(&spelling) {
                    alternatives.push(spelling);
                }
            }
        }
        if alternatives.is_empty() {
            return;
        }
        self.edits.push((
            class.span.start.offset,
            class.span.end.offset,
            format!("(?:{text}|{})", alternatives.join("|")),
        ));
    }
}

/// Whether `rep` is `{1}`, which Oniguruma reads as no repetition at all.
fn is_once(rep: &ast::Repetition) -> bool {
    use ast::{RepetitionKind, RepetitionRange};
    matches!(
        rep.op.kind,
        RepetitionKind::Range(RepetitionRange::Exactly(1) | RepetitionRange::Bounded(1, 1))
    )
}

fn lit_of(lit: &ast::Literal) -> Lit {
    Lit {
        c: lit.c,
        start: lit.span.start.offset,
        end: lit.span.end.offset,
    }
}

/// The fold of `c` for looking up an entry of the tables: its one-to-one fold if it has one that an
/// entry spells, else `c`.
fn key_of(c: char) -> char {
    FOLD_KEY
        .binary_search_by_key(&c, |&(unfolded, _)| unfolded)
        .map_or(c, |at| FOLD_KEY[at].1)
}

/// Every spelling of the characters `fold`: each character in each of its cases.
fn spell(fold: &[char]) -> String {
    fold.iter().map(|c| format!("(?i:{c})")).collect()
}

/// The fold Oniguruma finds at the start of `chars`, as `get_case_fold_codes_by_str` orders its
/// tries: three characters, then two, then the first alone.
fn fold_at(chars: &[char]) -> Option<Fold> {
    if chars.len() >= 3 {
        let key = [key_of(chars[0]), key_of(chars[1]), key_of(chars[2])];
        if let Some((fold, unfolds)) = FOLDS3.iter().find(|(fold, _)| *fold == key) {
            return Some(Fold {
                len: 3,
                text: alternation(&spell(fold), unfolds, None),
            });
        }
    }
    if chars.len() >= 2 {
        let key = [key_of(chars[0]), key_of(chars[1])];
        if let Some((fold, unfolds)) = FOLDS2.iter().find(|(fold, _)| *fold == key) {
            return Some(Fold {
                len: 2,
                text: alternation(&spell(fold), unfolds, None),
            });
        }
    }
    // A character that itself folds to several: itself, the others that fold the same way, then
    // every spelling of the fold.
    let c = chars[0];
    let entries = FOLDS2
        .iter()
        .map(|(fold, unfolds)| (&fold[..], *unfolds))
        .chain(FOLDS3.iter().map(|(fold, unfolds)| (&fold[..], *unfolds)));
    for (fold, unfolds) in entries {
        if unfolds.contains(&c) {
            return Some(Fold {
                len: 1,
                text: alternation(&spell(fold), unfolds, Some(c)),
            });
        }
    }
    None
}

/// `(?:spelling|u1|u2...)`: the spellings of a fold, then the characters that fold to it. `first`
/// is the character written in the pattern when it is one of them (it leads, as the library puts
/// the string as written first), and is not listed twice.
fn alternation(spelling: &str, unfolds: &[char], first: Option<char>) -> String {
    let mut out = String::from("(?:");
    if let Some(c) = first {
        out.push(c);
        out.push('|');
    }
    for unfold in unfolds.iter().filter(|u| Some(**u) != first) {
        out.push(*unfold);
        out.push('|');
    }
    out.push_str(spelling);
    out.push(')');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SS: &str = "(?:ß|ẞ|(?i:s)(?i:s))";

    #[test]
    fn a_pair_that_spells_a_fold_becomes_its_alternation() {
        assert_eq!(
            rewrite("(?i)ss").as_deref(),
            Some(format!("(?i){SS}").as_str())
        );
        assert_eq!(
            rewrite("(?i)ffi").as_deref(),
            Some("(?i)(?:ﬃ|(?i:f)(?i:f)(?i:i))")
        );
    }

    #[test]
    fn a_character_that_folds_to_several_leads_its_own_alternation() {
        // `ß` itself, the others that fold the same way, then every spelling of the fold.
        assert_eq!(
            rewrite("(?i)ß").as_deref(),
            Some(format!("(?i){SS}").as_str())
        );
        assert_eq!(
            rewrite("(?i)ẞ").as_deref(),
            Some("(?i)(?:ẞ|ß|(?i:s)(?i:s))")
        );
    }

    #[test]
    fn the_walk_goes_on_after_the_characters_a_fold_covered() {
        // `sss` is `ss` then `s`: the second `s` is not the start of another pair.
        assert_eq!(
            rewrite("(?i)sss").as_deref(),
            Some(format!("(?i){SS}s").as_str())
        );
        assert_eq!(
            rewrite("(?i)ssss").as_deref(),
            Some(format!("(?i){SS}{SS}").as_str())
        );
    }

    #[test]
    fn a_quantifier_takes_the_last_character_off_its_string() {
        assert_eq!(
            rewrite("(?i)ssß+").as_deref(),
            Some(format!("(?i){SS}{SS}+").as_str())
        );
        // `ss+` repeats the second `s`, so no pair is left.
        assert_eq!(rewrite("(?i)ss+"), None);
        // `{1}` repeats nothing.
        assert_eq!(
            rewrite("(?i)s{1}s").as_deref(),
            Some(format!("(?i){SS}{{1}}").as_str())
        );
        assert_eq!(rewrite("(?i)s{2}s"), None);
    }

    #[test]
    fn a_plain_group_a_comment_and_an_unchanged_option_join_the_string() {
        assert_eq!(
            rewrite("(?i)s(?:s)").as_deref(),
            Some(format!("(?i){SS}(?:)").as_str())
        );
        assert_eq!(
            rewrite("(?i)s(?#c)s").as_deref(),
            Some(format!("(?i){SS}(?#c)").as_str())
        );
        assert_eq!(
            rewrite("(?i)s(?m)s").as_deref(),
            Some(format!("(?i){SS}(?m)").as_str())
        );
        // A group that starts its branch and holds more than literals is a list of its own.
        assert_eq!(rewrite("(?i)(?:a\\d)s"), None);
        assert_eq!(rewrite("(?i)(?:s\\d)s"), None);
        assert!(rewrite("(?i)x(?:s\\d)s").is_some());
        assert!(rewrite("(?i)(?:ss)s").is_some());
        // A repeated group, a capture group and an option group do not.
        for pattern in [
            "(?i)s(?:s)+",
            "(?i)(s)s",
            "(?i)s(?i:s)",
            "(?i)s|s",
            "(?i)s.s",
        ] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
    }

    #[test]
    fn a_string_is_made_under_the_option_in_force_where_it_stands() {
        assert_eq!(rewrite("ss"), None);
        assert_eq!(rewrite("(?-i)ss"), None);
        assert_eq!(rewrite("(?i)s(?-i)s"), None);
        assert_eq!(
            rewrite("(?-i)(?i:ss)").as_deref(),
            Some(format!("(?-i)(?i:{SS})").as_str())
        );
        assert_eq!(
            rewrite("(?i)(?-i:ss)ss").as_deref(),
            Some(format!("(?i)(?-i:ss){SS}").as_str())
        );
    }

    #[test]
    fn a_class_holding_a_folding_character_gains_the_spelling() {
        assert_eq!(
            rewrite("(?i)[ß]").as_deref(),
            Some("(?i)(?:[ß]|(?i:s)(?i:s))")
        );
        // A range holds it, and so does a property.
        assert!(rewrite("(?i)[ß-ÿ]").is_some());
        assert!(rewrite("(?i)[\\w]").is_some());
        // Neither a negated class nor one of ASCII letters, `à-ÿ` (which starts above `ß`) or
        // `\d` does.
        for pattern in ["(?i)[^ß]", "(?i)[s]", "(?i)[a-z]", "(?i)[à-ÿ]", "(?i)[\\d]"] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
    }

    #[test]
    fn a_look_behind_expands_one_character_at_a_time() {
        for pattern in ["(?i)(?<=ss)a", "(?i)(?<!ß)a", "(?i)(?<=[ß])a"] {
            assert_eq!(rewrite(pattern), None, "{pattern}");
        }
        // A look-ahead is read as any other group.
        assert_eq!(
            rewrite("(?i)(?=ss)x").as_deref(),
            Some(format!("(?i)(?={SS})x").as_str())
        );
    }

    #[test]
    fn oniguruma_only_syntax_is_read_through_the_shadow() {
        assert_eq!(
            rewrite("(?i)ß(?=x)").as_deref(),
            Some(format!("(?i){SS}(?=x)").as_str())
        );
        assert_eq!(
            rewrite("(?i)(ß)\\1").as_deref(),
            Some(format!("(?i)({SS})\\1").as_str())
        );
    }

    #[test]
    fn a_pattern_that_does_not_parse_is_left_alone() {
        assert_eq!(rewrite("(?i)ß("), None);
        assert_eq!(rewrite("(?i)[ß"), None);
    }

    #[test]
    fn the_gate_passes_only_what_may_change() {
        assert!(!may_need_rewrite("ß"));
        assert!(!may_need_rewrite("(?i)abc"));
        assert!(!may_need_rewrite("(?i)\\d+"));
        assert!(may_need_rewrite("(?i)ß"));
        assert!(may_need_rewrite("(?ix)s # note\n s"));
        assert!(may_need_rewrite("(?i)\\x73\\x73"));
        assert!(may_need_rewrite("(?i)[\\w]"));
        assert!(may_need_rewrite("(?i)(?:s)s"));
        assert!(may_need_rewrite("(?i)s(?m)s"));
        assert!(may_need_rewrite("x(?i:ab)fl"));
        // The letters of an option group and of an escape are not the string's.
        assert!(!may_need_rewrite("(?is)a\\sb"));
        assert!(!ascii_fold_pair(b"\\s\\s"));
    }

    #[test]
    fn the_tables_agree_with_themselves() {
        assert!(FOLD_KEY.windows(2).all(|w| w[0].0 < w[1].0));
        for (fold, unfolds) in FOLDS2.iter().map(|(f, u)| (&f[..], *u)) {
            assert_ne!(unfolds.len(), 0);
            assert!(fold.iter().all(|c| key_of(*c) == *c), "{fold:?}");
        }
        for (fold, unfolds) in FOLDS3.iter().map(|(f, u)| (&f[..], *u)) {
            assert_ne!(unfolds.len(), 0);
            assert!(fold.iter().all(|c| key_of(*c) == *c), "{fold:?}");
        }
        // 104 characters fold to several.
        let mut all: Vec<char> = FOLDS2
            .iter()
            .flat_map(|(_, u)| u.iter().copied())
            .chain(FOLDS3.iter().flat_map(|(_, u)| u.iter().copied()))
            .collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 104);
        // A capital and the long s reach the key of `s`.
        assert_eq!((key_of('S'), key_of('ſ'), key_of('s')), ('s', 's', 's'));
    }
}
