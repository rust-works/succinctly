//! #2416's regrowth guard: the number of expression shapes that
//! `eval_stage_with_path_context` (`src/jq/eval.rs`) handles by name is
//! pinned here, and this test fails in **both** directions when it moves.
//!
//! # Why a count, and why pinned rather than bounded
//!
//! #2416 retires that function by strangler-fig migration: each shape it
//! handles is re-implemented once, in `try_path_context_cursor_walk`
//! (`src/jq/eval_generic.rs`), and its arm here is deleted when the walk
//! covers it. The function's `_` fallback silently loses path context, so
//! adding an arm was the *only* way to fix a shape, and 61 of the 76 commits
//! that touched the arm block landed in the five weeks before the decision.
//! Nothing in the build noticed any of them.
//!
//! A ceiling alone would let a migration that removes three arms hand back
//! three free slots to the next standalone fix. Pinning the exact count means
//! a migration lowers the pin in the same PR, and a PR that adds an arm has
//! to raise it -- and #2416's hygiene rule says such a PR states why it is
//! not a migration. The pin is that statement's enforcement.
//!
//! # What counts as an arm
//!
//! Two things, both dispatch on `first`:
//!
//! - every arm of the function's top-level `match first { .. }` except the
//!   `_` fallback -- a guarded arm (`Expr::Array(inner) if
//!   needs_path_context(inner)`) is a shape, so it counts; a nested `match`
//!   inside an arm is not, so it does not;
//! - every top-level `if` statement *before* that match whose condition
//!   reads `first` -- `if matches!(first, Expr::Builtin(Builtin::Key))` and
//!   `if let Expr::Builtin(Builtin::ParentN(n)) = first` are arms in
//!   everything but syntax, and a guard that only saw the `match` would be
//!   sidestepped by the next one of these.
//!
//! Same shape as `jq_member_validation_audit.rs` (STYLE-0013, #1803): a
//! source scan over `include_str!`, because the property is about the
//! source, not about any query's output.

use syn::visit::{self, Visit};

/// The pinned count. Lower it in the PR that migrates an arm to the cursor
/// walk; raise it only in a PR that says why the new arm is not a migration
/// (#2416, "Hygiene going forward", rule 2).
///
/// Step 4 of that spine audited every one of these handlers for reachability
/// and found no dead ones: `docs/plan/path-context-arm-reachability.md` has a
/// live proof query and the gate reason for each. Step 5 then closed the two
/// routes that entered the eager evaluator *without* consulting
/// `path_context_needs_eager` (`--eval-all` and `eval::eval_pipe`'s own
/// diversion), so the gate is now the whole answer to which pipes reach an
/// arm -- `tests/jq_path_context_single_door_guard.rs` keeps it that way.
/// Closing them removed no arm's only route, so the pin did not move. Read
/// that page before assuming an arm is deletable.
const PINNED_ARM_COUNT: usize = 43;

const TARGET_FN: &str = "eval_stage_with_path_context";
const EVAL_RS: &str = include_str!("../src/jq/eval.rs");

/// What the scan found, split so a failure message can say which half moved.
#[derive(Debug, PartialEq, Eq)]
struct ArmCount {
    /// Top-level `if` statements before the `match` whose condition reads
    /// `first`.
    pre_match_handlers: usize,
    /// Arms of the top-level `match first`, excluding the `_` fallback.
    match_arms: usize,
    /// Whether the `_` fallback is present -- its disappearance is the exit
    /// condition, not a regression, but it should be a deliberate change.
    has_wildcard_fallback: bool,
}

impl ArmCount {
    fn total(&self) -> usize {
        self.pre_match_handlers + self.match_arms
    }
}

/// Finds the `fn` item named `name` anywhere in the file.
struct FindFn<'a> {
    name: &'a str,
    found: Option<syn::ItemFn>,
}

impl<'ast> Visit<'ast> for FindFn<'_> {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if self.found.is_none() && item.sig.ident == self.name {
            self.found = Some(item.clone());
        }
        visit::visit_item_fn(self, item);
    }
}

/// Whether an expression mentions the bare path `first` anywhere inside it.
struct MentionsFirst(bool);

impl<'ast> Visit<'ast> for MentionsFirst {
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path.path.is_ident("first") {
            self.0 = true;
        }
        visit::visit_expr_path(self, path);
    }

    // `matches!(first, ..)` keeps `first` inside a macro's token stream, which
    // `syn` does not parse. Scan the tokens instead.
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac
            .tokens
            .clone()
            .into_iter()
            .any(|tt| matches!(&tt, proc_macro2::TokenTree::Ident(id) if id == "first"))
        {
            self.0 = true;
        }
        visit::visit_macro(self, mac);
    }
}

fn mentions_first(expr: &syn::Expr) -> bool {
    let mut v = MentionsFirst(false);
    v.visit_expr(expr);
    v.0
}

/// Whether a `match` scrutinee is the bare identifier `first`.
fn is_match_on_first(m: &syn::ExprMatch) -> bool {
    matches!(&*m.expr, syn::Expr::Path(p) if p.path.is_ident("first"))
}

fn count_arms(func: &syn::ItemFn) -> ArmCount {
    let mut pre_match_handlers = 0;
    for stmt in &func.block.stmts {
        match stmt {
            // The match is the function's tail expression: everything before
            // it that dispatches on `first` is a handler in disguise.
            syn::Stmt::Expr(syn::Expr::Match(m), None) if is_match_on_first(m) => {
                let match_arms = m
                    .arms
                    .iter()
                    .filter(|arm| !matches!(arm.pat, syn::Pat::Wild(_)))
                    .count();
                let has_wildcard_fallback = m.arms.len() != match_arms;
                return ArmCount {
                    pre_match_handlers,
                    match_arms,
                    has_wildcard_fallback,
                };
            }
            syn::Stmt::Expr(syn::Expr::If(i), _) if mentions_first(&i.cond) => {
                pre_match_handlers += 1;
            }
            _ => {}
        }
    }
    panic!("{TARGET_FN} has no top-level `match first`; the guard needs updating, not deleting");
}

fn count_in_source(src: &str, name: &str) -> ArmCount {
    let file = syn::parse_file(src).expect("source parses");
    let mut finder = FindFn { name, found: None };
    finder.visit_file(&file);
    let func = finder
        .found
        .unwrap_or_else(|| panic!("no `fn {name}` in source"));
    count_arms(&func)
}

#[test]
fn test_eval_stage_with_path_context_arm_count_is_pinned() {
    let count = count_in_source(EVAL_RS, TARGET_FN);
    assert!(
        count.has_wildcard_fallback,
        "{TARGET_FN} lost its `_` fallback: if every shape is now handled by name, #2416's \
         exit condition is met and this guard retires with the function; otherwise a \
         missing fallback is a compile error, not a design change"
    );
    assert_eq!(
        count.total(),
        PINNED_ARM_COUNT,
        "{TARGET_FN} handles {} shapes by name ({} pre-match handlers + {} match arms), pin is \
         {PINNED_ARM_COUNT}. Lower PINNED_ARM_COUNT if this PR migrates an arm to \
         try_path_context_cursor_walk (#2416 phase 3). Raise it only if the PR states why the \
         new arm is not a migration -- the `_` fallback is why an added arm never fails a test \
         on its own.",
        count.total(),
        count.pre_match_handlers,
        count.match_arms,
    );
}

/// The two halves are pinned separately as well, so an arm cannot move from
/// the `match` into a pre-match `if` (or back) without the diff saying so.
#[test]
fn test_eval_stage_with_path_context_arm_split_is_pinned() {
    let count = count_in_source(EVAL_RS, TARGET_FN);
    assert_eq!(
        (count.pre_match_handlers, count.match_arms),
        (5, 38),
        "pre-match handlers / match arms moved: {count:?}"
    );
}

// --- negative tests: every way the count could be gamed --------------------

const FIXTURE: &str = r"
    fn eval_stage_with_path_context(first: &Expr, rest: &[Expr]) -> R {
        if matches!(first, Expr::Builtin(Builtin::Key)) {
            return key();
        }
        if let Expr::Builtin(Builtin::ParentN(n)) = first {
            return parent_n(n);
        }
        if rest.is_empty() {
            return fast_path();
        }
        match first {
            Expr::Identity => identity(),
            Expr::Field(name) => field(name),
            Expr::Array(inner) if needs_path_context(inner) => {
                match inner.as_ref() {
                    Expr::Iterate => nested_a(),
                    _ => nested_b(),
                }
            }
            _ => fallback(),
        }
    }
";

#[test]
fn test_fixture_counts_handlers_and_arms_not_nested_matches() {
    let count = count_in_source(FIXTURE, TARGET_FN);
    assert_eq!(
        count,
        ArmCount {
            // `if rest.is_empty()` does not read `first`, so it is not one.
            pre_match_handlers: 2,
            // The nested `match inner` contributes nothing; the guarded arm
            // counts once.
            match_arms: 3,
            has_wildcard_fallback: true,
        }
    );
}

#[test]
fn test_adding_a_match_arm_moves_the_count() {
    let grown = FIXTURE.replace(
        "Expr::Identity => identity(),",
        "Expr::Identity => identity(),\n            Expr::Iterate => iterate(),",
    );
    assert_eq!(count_in_source(&grown, TARGET_FN).match_arms, 4);
}

#[test]
fn test_adding_a_pre_match_handler_moves_the_count() {
    let grown = FIXTURE.replace(
        "if rest.is_empty() {",
        "if matches!(first, Expr::Builtin(Builtin::FileIndex)) {\n            return fi();\n        }\n        if rest.is_empty() {",
    );
    assert_eq!(count_in_source(&grown, TARGET_FN).pre_match_handlers, 3);
}

#[test]
fn test_removing_the_wildcard_is_reported() {
    let no_fallback = FIXTURE.replace("_ => fallback(),", "");
    let count = count_in_source(&no_fallback, TARGET_FN);
    assert!(!count.has_wildcard_fallback);
    assert_eq!(count.match_arms, 3);
}

#[test]
fn test_a_match_on_something_else_is_not_the_dispatch() {
    // A `match rest` before the real dispatch must be skipped over, not
    // mistaken for it.
    let decoy = FIXTURE.replace(
        "if rest.is_empty() {",
        "match rest { [] => {} _ => {} }\n        if rest.is_empty() {",
    );
    assert_eq!(count_in_source(&decoy, TARGET_FN).match_arms, 3);
}
