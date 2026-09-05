//! Spine 2416, step 5: `path_context_needs_eager` is the **only** door into
//! the eager path-context evaluator.
//!
//! # The property
//!
//! `eval_stage_with_path_context` (`src/jq/eval.rs`) is reached through
//! exactly two entry points -- `eval_pipe_with_path_context` and its
//! `_internal` twin -- and ADR-0021 decision 3 says which pipes belong to it:
//! whatever `path_context_needs_eager` (`src/jq/eval_generic.rs`) answers
//! `true` for. `docs/plan/path-context-arm-reachability.md` found that the
//! gate was one of *three* routes in, not the only one:
//!
//! 1. the gate itself, in `eval_single`'s `Expr::Pipe` arm and in
//!    `eval_each_pipe_generic`;
//! 2. `eval::eval_owned_with_file_index` (`--eval-all`, #715), which called
//!    `eval_pipe_with_path_context_internal` directly on
//!    `needs_path_context(expr)` alone;
//! 3. `eval::eval_pipe`'s own diversion, which sent any pipe with a
//!    `needs_path_context` stage to `eval_pipe_with_path_context`, also with
//!    no gate.
//!
//! Doors 2 and 3 are closed, and this file is what keeps them closed. It
//! matters because a new ungated door is invisible: the eager evaluator
//! answers everything, so routing a pipe there produces *output*, just
//! output from the evaluator the spine is retiring -- and the arm-count pin
//! (`tests/jq_path_context_arm_guard.rs`) cannot be lowered honestly while a
//! route can reach an arm without the gate having chosen it.
//!
//! # How it is checked
//!
//! A source scan, like `jq_path_context_arm_guard.rs` and
//! `jq_member_validation_audit.rs` (STYLE-0013, #1803): the property is about
//! the call graph, not about any query's output, and a behavioural test would
//! pass just as happily with a second door open.
//!
//! The scan walks `src/` on disk rather than a fixed `include_str!` list, so a
//! door opened from a *new* file is caught too.
//!
//! **The gate call has to appear in the `if` condition itself.** Binding it
//! first (`let eager = path_context_needs_eager(exprs); if eager { .. }`)
//! reads as gated to a person and not to this scan, and following the binding
//! would mean a dataflow pass whose "gate" could then be any boolean named
//! suggestively -- which is the looseness that lets a door back in. The strict
//! rule costs one call-site convention, recorded in a comment at the site that
//! has to keep it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};

/// The eager evaluator's two entry points.
const EAGER_ENTRIES: [&str; 2] = [
    "eval_pipe_with_path_context",
    "eval_pipe_with_path_context_internal",
];

/// The one gate (ADR-0021 decision 3).
const GATE: &str = "path_context_needs_eager";

/// The file the eager evaluator lives in. Its own recursion into itself is
/// not a door, so whole-file rules do not apply to it -- see
/// [`test_eval_rs_has_exactly_one_ungated_free_entry`] for what is pinned
/// there instead.
const EAGER_FILE: &str = "src/jq/eval.rs";

/// Functions in [`EAGER_FILE`] that must not name either entry point at all:
/// each one *was* a door, or is the dispatch that fed one.
const EVAL_RS_MUST_NOT_CALL: [&str; 2] = ["eval_pipe", "eval_owned_with_file_index"];

/// The single function in [`EAGER_FILE`] allowed to call the eager
/// evaluator's public wrapper from outside the eager evaluator itself -- and
/// its call has to be gated like every other.
const EVAL_RS_SOLE_GATED_ENTRY: &str = "eval_path_context_pipe_owned";

// --- the scan ---------------------------------------------------------------

/// One reference to an eager entry point, and whether it stands inside an
/// `if` whose condition consults [`GATE`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct CallSite {
    function: String,
    callee: String,
    gated: bool,
}

/// Whether `expr` mentions the identifier `name` anywhere, macro token
/// streams included (`matches!(..)` hides its contents from `syn`'s
/// expression visitor).
fn mentions(expr: &syn::Expr, name: &str) -> bool {
    struct V<'a>(&'a str, bool);
    impl<'ast> Visit<'ast> for V<'_> {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if path.segments.last().is_some_and(|s| s.ident == self.0) {
                self.1 = true;
            }
            visit::visit_path(self, path);
        }
        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if mac
                .tokens
                .clone()
                .into_iter()
                .any(|tt| matches!(&tt, proc_macro2::TokenTree::Ident(id) if id == self.0))
            {
                self.1 = true;
            }
            visit::visit_macro(self, mac);
        }
    }
    let mut v = V(name, false);
    v.visit_expr(expr);
    v.1
}

/// Collects [`CallSite`]s inside one function body, tracking whether the
/// cursor is inside a [`GATE`]-conditioned `if`.
///
/// Only the `then` branch inherits the gate: an `else` runs precisely when
/// the gate said `false`, and a *sibling* statement after the `if` runs
/// either way. Both are pinned by the negative tests below -- a proximity
/// window over source text gets both wrong (the lesson from the
/// STYLE-0013 audit's own near-miss).
struct Collect {
    function: String,
    gated: bool,
    sites: Vec<CallSite>,
}

impl Collect {
    fn new(function: &str) -> Self {
        Self {
            function: function.to_string(),
            gated: false,
            sites: Vec::new(),
        }
    }

    fn record(&mut self, path: &syn::Path) {
        let Some(last) = path.segments.last() else {
            return;
        };
        // Longest name first: `eval_pipe_with_path_context_internal` is a
        // distinct ident from `eval_pipe_with_path_context`, and `Ident`
        // comparison is exact, so no prefix confusion is possible -- this
        // loop only has to try both.
        for entry in EAGER_ENTRIES {
            if last.ident == entry {
                self.sites.push(CallSite {
                    function: self.function.clone(),
                    callee: entry.to_string(),
                    gated: self.gated,
                });
            }
        }
    }
}

impl<'ast> Visit<'ast> for Collect {
    fn visit_expr(&mut self, expr: &'ast syn::Expr) {
        if let syn::Expr::If(if_expr) = expr {
            let gate_here = mentions(&if_expr.cond, GATE);
            self.visit_expr(&if_expr.cond);
            let outer = self.gated;
            self.gated = outer || gate_here;
            for stmt in &if_expr.then_branch.stmts {
                self.visit_stmt(stmt);
            }
            self.gated = outer;
            if let Some((_, alt)) = &if_expr.else_branch {
                self.visit_expr(alt);
            }
            return;
        }
        visit::visit_expr(self, expr);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.record(path);
        visit::visit_path(self, path);
    }

    // A nested `fn` inside a function body is its own scope, so the outer
    // function's `if` does not gate it -- and [`Functions`] already collects
    // it as an item in its own right, so descending here would count its
    // call sites twice. Skipped, not recursed.
    fn visit_item_fn(&mut self, _item: &'ast syn::ItemFn) {}
}

/// Every function item in a parsed file, by name, body included.
struct Functions(Vec<syn::ItemFn>);

impl<'ast> Visit<'ast> for Functions {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.0.push(item.clone());
        visit::visit_item_fn(self, item);
    }
}

fn call_sites_in_source(src: &str) -> Vec<CallSite> {
    let file = syn::parse_file(src).expect("source parses");
    let mut fns = Functions(Vec::new());
    fns.visit_file(&file);
    let mut out = Vec::new();
    for item in fns.0 {
        // Skip the entry points' own definitions: `fn
        // eval_pipe_with_path_context` calling `..._internal` is the
        // evaluator, not a door into it.
        let name = item.sig.ident.to_string();
        if EAGER_ENTRIES.contains(&name.as_str()) {
            continue;
        }
        let mut collect = Collect::new(&name);
        // The signature can name a type, never a call; walk the body only.
        visit::visit_block(&mut collect, &item.block);
        out.extend(collect.sites);
    }
    out
}

/// Every `.rs` file under `src/`, relative to the crate root.
fn source_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = std::fs::read_dir(dir).expect("src/ is readable");
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    walk(&root.join("src"), &mut out);
    out.sort();
    out.into_iter()
        .map(|p| {
            p.strip_prefix(&root)
                .expect("under the crate root")
                .to_path_buf()
        })
        .collect()
}

fn call_sites_by_file() -> BTreeMap<String, Vec<CallSite>> {
    let mut map = BTreeMap::new();
    for path in source_files() {
        let src = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&path))
            .expect("source file is readable");
        let sites = call_sites_in_source(&src);
        if !sites.is_empty() {
            map.insert(path.to_string_lossy().replace('\\', "/"), sites);
        }
    }
    map
}

// --- the pins ---------------------------------------------------------------

/// The scan reaches the whole crate, so a door opened from a new file cannot
/// hide behind a stale `include_str!` list.
#[test]
fn test_scan_covers_the_crate_source() {
    let files = source_files();
    assert!(
        files.len() > 30,
        "src/ walk found only {} files -- the walk is broken, and every rule \
         below would pass vacuously",
        files.len()
    );
    assert!(
        files.iter().any(|p| p.to_string_lossy() == EAGER_FILE),
        "{EAGER_FILE} not found by the walk"
    );
}

/// Door 1 of the audit -- and now the only one: every entry into the eager
/// evaluator from outside the file it lives in stands inside a
/// `path_context_needs_eager` branch.
#[test]
fn test_every_entry_outside_eval_rs_is_gated() {
    let by_file = call_sites_by_file();
    let mut external = 0;
    for (file, sites) in &by_file {
        if file == EAGER_FILE {
            continue;
        }
        for site in sites {
            external += 1;
            assert!(
                site.gated,
                "{file}: `{}` calls `{}` without a `{GATE}` branch around it. \
                 ADR-0021 decision 3 says the gate decides which pipes the eager \
                 evaluator owns; an ungated call is a new door, and \
                 docs/plan/path-context-arm-reachability.md has to stop saying \
                 there is one.",
                site.function, site.callee,
            );
        }
    }
    assert!(
        external > 0,
        "no call to the eager evaluator found outside {EAGER_FILE} at all -- either \
         the last one was removed (delete this test with the evaluator, per \
         ADR-0021 decision 8) or the entry points were renamed and \
         EAGER_ENTRIES is stale"
    );
}

/// `_internal` is private to the eager evaluator's own file. The compiler
/// already enforces that; pinning it here is what makes the rule above
/// complete rather than merely true today.
#[test]
fn test_internal_entry_is_never_named_outside_eval_rs() {
    for (file, sites) in call_sites_by_file() {
        if file == EAGER_FILE {
            continue;
        }
        for site in sites {
            assert_ne!(
                site.callee, "eval_pipe_with_path_context_internal",
                "{file}: `{}` reaches the eager evaluator's internal entry, \
                 bypassing the `&[]`/ambient-file-origin normalization the public \
                 wrapper applies",
                site.function,
            );
        }
    }
}

/// Inside `eval.rs` the eager evaluator recurses into itself constantly, so
/// the whole-file rule above would say nothing. What is pinned instead is the
/// shape of the *doors that used to be here*: the two functions that had one,
/// and the single replacement that carries the gate.
#[test]
fn test_eval_rs_has_exactly_one_ungated_free_entry() {
    let src = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(EAGER_FILE))
        .expect("eval.rs is readable");
    let sites = call_sites_in_source(&src);

    for name in EVAL_RS_MUST_NOT_CALL {
        let offenders: Vec<&CallSite> = sites.iter().filter(|s| s.function == name).collect();
        assert!(
            offenders.is_empty(),
            "`{name}` reaches the eager evaluator again: {offenders:?}. That was a door \
             (docs/plan/path-context-arm-reachability.md); it routes through \
             `{EVAL_RS_SOLE_GATED_ENTRY}` now so the gate decides."
        );
    }

    // The public wrapper is the only entry a non-eager caller may use, and
    // exactly one function in this file is such a caller.
    let wrapper_callers: Vec<String> = sites
        .iter()
        .filter(|s| s.callee == "eval_pipe_with_path_context")
        .map(|s| s.function.clone())
        .collect();
    assert_eq!(
        wrapper_callers,
        vec![EVAL_RS_SOLE_GATED_ENTRY.to_string()],
        "the set of functions calling the eager evaluator's public wrapper moved"
    );
    let gated = sites
        .iter()
        .filter(|s| s.function == EVAL_RS_SOLE_GATED_ENTRY)
        .all(|s| s.gated);
    assert!(
        gated,
        "`{EVAL_RS_SOLE_GATED_ENTRY}` no longer consults `{GATE}` before entering the \
         eager evaluator"
    );
}

// --- negative tests: every way the gate could appear to hold ----------------

fn sites(src: &str) -> Vec<CallSite> {
    call_sites_in_source(src)
}

#[test]
fn test_fixture_gated_call_is_gated() {
    let src = r"
        fn caller() {
            if path_context_needs_eager(exprs) {
                return eval_pipe_with_path_context::<W, S>(exprs);
            }
            generic()
        }
    ";
    assert_eq!(sites(src).len(), 1);
    assert!(sites(src)[0].gated);
}

#[test]
fn test_fixture_ungated_call_is_not_gated() {
    let src = r"
        fn caller() {
            eval_pipe_with_path_context::<W, S>(exprs)
        }
    ";
    assert!(!sites(src)[0].gated);
}

#[test]
fn test_fixture_a_different_condition_does_not_gate() {
    let src = r"
        fn caller() {
            if needs_path_context(expr) {
                return eval_pipe_with_path_context::<W, S>(exprs);
            }
        }
    ";
    assert!(
        !sites(src)[0].gated,
        "`needs_path_context` is the *old* condition doors 2 and 3 used; it must \
         not satisfy the rule"
    );
}

/// A sibling `if` is not a gate: this is the case a proximity window over
/// source text gets wrong, since the gate's text is a few lines above the
/// call either way.
#[test]
fn test_fixture_sibling_if_does_not_gate_a_later_call() {
    let src = r"
        fn caller() {
            if path_context_needs_eager(exprs) {
                return generic();
            }
            eval_pipe_with_path_context::<W, S>(exprs)
        }
    ";
    assert!(!sites(src)[0].gated);
}

#[test]
fn test_fixture_else_branch_is_not_gated() {
    let src = r"
        fn caller() {
            if path_context_needs_eager(exprs) {
                generic()
            } else {
                eval_pipe_with_path_context::<W, S>(exprs)
            }
        }
    ";
    assert!(
        !sites(src)[0].gated,
        "the `else` runs precisely when the gate answered `false`"
    );
}

#[test]
fn test_fixture_nesting_inherits_the_gate() {
    let src = r"
        fn caller() {
            if path_context_needs_eager(exprs) {
                let owned = to_owned(&value)?;
                if reindex_bridge_is_identity(&owned) {
                    for _ in 0..1 {
                        return eval_pipe_with_path_context::<W, S>(exprs);
                    }
                }
            }
        }
    ";
    assert!(sites(src)[0].gated);
}

/// A disjunction still consults the gate, which is what
/// `eval_path_context_pipe_owned` spells (`gate || !reindex_identity`): the
/// eager evaluator is entered when the gate says so *or* when the round trip
/// would not be lossless.
#[test]
fn test_fixture_disjunction_containing_the_gate_counts() {
    let src = r"
        fn caller() {
            if path_context_needs_eager(exprs) || !reindex_bridge_is_identity(owned) {
                return eval_pipe_with_path_context::<W, S>(exprs);
            }
        }
    ";
    assert!(sites(src)[0].gated);
}

#[test]
fn test_fixture_a_nested_fn_does_not_inherit_the_gate() {
    let src = r"
        fn caller() {
            if path_context_needs_eager(exprs) {
                fn inner() {
                    eval_pipe_with_path_context::<W, S>(exprs)
                }
                inner()
            }
        }
    ";
    let found = sites(src);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].function, "inner");
    assert!(!found[0].gated);
}

#[test]
fn test_fixture_the_entry_points_own_recursion_is_not_a_call_site() {
    let src = r"
        fn eval_pipe_with_path_context() {
            eval_pipe_with_path_context_internal(exprs)
        }
    ";
    assert!(sites(src).is_empty());
}

#[test]
fn test_fixture_internal_and_wrapper_are_told_apart() {
    let src = r"
        fn caller() {
            eval_pipe_with_path_context_internal::<W, S>(exprs)
        }
    ";
    let found = sites(src);
    assert_eq!(found.len(), 1, "the two idents must not both match");
    assert_eq!(found[0].callee, "eval_pipe_with_path_context_internal");
}

/// A call hidden in a macro's token stream would slip past
/// `visit_path`; the gate detector reads macro tokens, and so does the
/// recorder via the tokens `syn` does parse for a call inside `matches!`-style
/// macros. This pins the gate half, which is the half a door could exploit.
#[test]
fn test_fixture_gate_inside_a_macro_condition_counts() {
    let src = r"
        fn caller() {
            if matches!(path_context_needs_eager(exprs), true) {
                return eval_pipe_with_path_context::<W, S>(exprs);
            }
        }
    ";
    assert!(sites(src)[0].gated);
}
