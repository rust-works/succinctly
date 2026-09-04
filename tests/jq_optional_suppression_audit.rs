//! STYLE-0012's enforcement: every raw materialization call site inside a
//! function with a live `optional: bool` must either route its error through
//! the suppression machinery or carry a `// STYLE-0012:` exemption (#2334).
//!
//! # Why this is a source scan and not a behavioural test
//!
//! `eval::suppresses` is `optional && !e.is_decode_failure()`. Every error the
//! materialization family can raise today is `decode_failure`-tagged --
//! `to_owned`/`to_owned_at_depth`, `to_owned_cursor_at_depth`,
//! `to_owned_with_cursor`, `collect_cursors_checked` and
//! `yaml_value_to_owned_checked`, whose only error
//! constructors are `EvalError::decode_failure`,
//! `EvalError::colliding_display_key` (which delegates to it) and the
//! `malformed_delimiter/member/element_error` trio (#2286 retagged the last of
//! those via `EvalError::malformed_json_text`). So `suppresses` is *always
//! false* at every one of these sites, and routing a site or leaving it bare
//! produce byte-identical output.
//!
//! That is precisely why the bug recurred three times (#2231, #2280, #2327),
//! each round's review finding more sites the last sweep missed: **no
//! behavioural test can fail when a site is left unrouted**. #2327's own
//! pinning test, `test_optional_ignored_sites_2327`, asserts exit 5 both with
//! and without a trailing `?` -- it records the non-difference, and by
//! construction cannot detect the invariant regressing. Centralising the
//! *check* (`suppresses`/`suppress_or_raise`/the `*_or_suppress!` macros) did
//! not stop the *omission*, because nothing forced a call site to route
//! through them.
//!
//! Same problem, same answer as `jq_owned_only_sink_invariant_test.rs` (#2025):
//! a claim that "was re-derived by hand ... and nothing but review stops a later
//! edit from" breaking it gets a mechanical guard. The runtime half of that
//! guard is `EvalError`'s `debug_assert_materialization_error`, which pins the
//! premise above; this file is the static half, which pins the routing.
//!
//! # What it does not do
//!
//! It does not decide whether a site *should* suppress -- that depends on
//! error-tag semantics and on whether an outer layer already double-catches,
//! neither of which is visible from the call site. It only demands that
//! somebody made the decision and wrote it down.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use proc_macro2::Span;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};

/// The two evaluators. Both are scanned: #2327's worst miss (7 of the 8 sites
/// its own review found) was the "fixed one evaluator's copy, missed the
/// other's" shape, and scanning both makes that shape fail here regardless of
/// what a same-named twin in the other file does -- no fragile name pairing
/// between the files required.
const SOURCES: &[(&str, &str)] = &[
    ("src/jq/eval.rs", include_str!("../src/jq/eval.rs")),
    (
        "src/jq/eval_generic.rs",
        include_str!("../src/jq/eval_generic.rs"),
    ),
];

/// Free functions that materialize a borrowed document value into an
/// `OwnedValue`, each returning `Result<_, EvalError>`, are matched by the
/// `to_owned` *prefix* rather than an explicit roster. Default-in, exceptions
/// written down: a future `to_owned_something` is covered without anyone
/// remembering to add it here, which is the polarity this whole issue is about.
///
/// Matched as *free* calls only. Neither file calls `.to_owned()` as a method
/// today, but `ToOwned::to_owned` on a `str`/`[u8]` is unrelated to any of
/// this, so restricting the match to the call form keeps a future
/// `some_string.to_owned()` from being reported as a materialization.
const MATERIALIZER_FN_PREFIX: &str = "to_owned";

/// The `to_owned*` functions that are deliberately *not* part of the audited
/// family:
///
/// - `to_owned_lossy`/`to_owned_lossy_at_depth`/`to_owned_for_diagnostic` are
///   infallible (they return `OwnedValue`, not `Result`), so there is no error
///   for `optional` to suppress or raise. `to_owned_for_diagnostic` is the one
///   sanctioned lossy materialization left in `eval_generic.rs` (#1247).
/// - `to_owned_value` is `AnchorResolved`'s own infallible accessor
///   (`eval.rs`'s `resolve_plain(..).to_owned_value(text)`,
///   `eval_generic.rs`'s `resolved.to_owned_value(text)`) -- an anchor helper
///   that shares the prefix and nothing else. It is a *method*, so the call
///   visitor never sees it; the macro token scan below matches on bare words
///   and would, so it is excluded here (#2334 review).
/// - `to_owned_checked` is the #2299 sibling of `to_owned` that swaps
///   `assert_nesting_depth`'s panic for a *catchable* `check_nesting_depth`
///   error -- the one member of the family that genuinely can raise a
///   non-decode-failure. It has a single call site, `materialize_stream_item`
///   in the CLI crate's `jq_runner.rs`, which has no `optional` in scope and is
///   not one of the files scanned here. Leaving it out keeps the premise
///   `debug_assert_materialization_error` pins ("this family raises only decode
///   failures") true of exactly the set the audit covers -- enforced by
///   [`test_to_owned_checked_has_no_call_sites_outside_its_own_definition_2367`]
///   below, since nothing else here would notice a second call site appearing
///   inside either evaluator (#2367).
const MATERIALIZER_FN_EXCLUSIONS: &[&str] = &[
    "to_owned_lossy",
    "to_owned_lossy_at_depth",
    "to_owned_for_diagnostic",
    "to_owned_checked",
    "to_owned_checked_at_depth",
    "to_owned_value",
];

fn is_materializer_fn(name: &str) -> bool {
    if MATERIALIZER_FN_EXTRA.contains(&name) {
        return true;
    }
    name.starts_with(MATERIALIZER_FN_PREFIX) && !MATERIALIZER_FN_EXCLUSIONS.contains(&name)
}

/// Materializers whose names do not carry the `to_owned` prefix, and so would
/// otherwise slip past [`is_materializer_fn`] entirely.
///
/// `yaml_value_to_owned_checked` (`eval.rs`, `#[cfg(feature = "std")]`) is the
/// `load` builtin's own YAML walk: `YamlCursor -> Result<OwnedValue,
/// EvalError>`, recursive, raising exactly the two constructors the rest of
/// the family does (`EvalError::decode_failure` and
/// `EvalError::colliding_display_key`, which delegates to it). It sits inside
/// `builtin_load`, which has a live `optional` -- and the prefix rule could not
/// see it, because the prefix is in the *middle* of the name (#2334 review).
///
/// Keep this roster empty if you can: a name that starts with `to_owned` is
/// found by default, which is the polarity this audit is about. Anything here
/// had to be noticed by a human first, which is exactly the failure mode
/// STYLE-0012 exists to stop -- so a new materializer should be *named* into
/// the family rather than added here.
const MATERIALIZER_FN_EXTRA: &[&str] = &["yaml_value_to_owned_checked"];

/// Methods with the same materialization contract. `collect_cursors_checked`
/// is only ever a method call (`elements.collect_cursors_checked()`).
const MATERIALIZER_METHODS: &[&str] = &["collect_cursors_checked"];

/// Macros that already fold the materialization error through `suppresses`.
/// Their bodies are unparsed token streams, so a site wrapped in one is
/// invisible to the visitor below -- which is the point: routed sites cost
/// nothing to skip. Listed explicitly anyway so that a routed macro whose
/// *argument* is itself a raw call (`owned_or_suppress!(to_owned(&v),
/// optional)`) is not reported.
const ROUTED_MACROS: &[&str] = &[
    "to_owned_or_suppress",
    "to_owned_vec_or_suppress",
    "owned_or_suppress",
];

/// Textual evidence, at the call site, that the error *is* routed. The
/// hand-rolled shape is `match to_owned(..) { Ok(v) => v, Err(e) => return
/// suppress_or_raise(e, optional) }`, so the evidence sits a few lines below
/// the call.
const ROUTING_TOKENS: &[&str] = &["suppress_or_raise", "suppresses("];

/// The STYLE-0012 exemption marker, cited inline the way STYLE-0004's
/// `#[allow]` citations already are.
const EXEMPT_MARKER: &str = "STYLE-0012:";

/// How far below a call site routing evidence may sit.
///
/// Proximity, not data flow: the common shape collects a `Result` in one
/// statement (`let items: Result<_, _> = match value { .. to_owned(..) .. }`)
/// and folds it in a later one (`let items = match items { Err(e) => return
/// suppress_or_raise(e, optional) }`), with the rest of the first `match`'s
/// arms in between. Twenty lines covers every such pair in the two files
/// today (`builtin_flatten`'s is the widest, at nineteen). A site whose
/// routing genuinely sits further away -- `builtin_contains`/`builtin_inside`
/// defer their unwrap into a closure (#1800) -- uses the `// STYLE-0012:`
/// marker to say so instead; that escape hatch is why this can stay a simple
/// window rather than a data-flow analysis.
const WINDOW_AFTER: usize = 20;

/// A marker is looked for in the contiguous run of comment (and blank) lines
/// immediately above the call site, however long that run is -- not within a
/// fixed line budget.
///
/// A comment block attached to a line is unambiguously *about* that line, so
/// there is no distance to tune and no risk of one site's marker silently
/// excusing an unrelated site further down. It also suits this codebase, whose
/// explanatory comments routinely run well past any window a fixed budget
/// could safely allow.
fn marker_in_attached_comment_block(lines: &[&str], idx: usize, body_start: usize) -> bool {
    let mut i = idx;
    // Never walks above the enclosing function's own body, so a marker on the
    // *previous* function's last line cannot excuse this one.
    let floor = body_start.saturating_sub(1);
    while i > floor {
        i -= 1;
        let t = lines[i].trim_start();
        if t.is_empty() || t.starts_with("//") {
            if t.contains(EXEMPT_MARKER) {
                return true;
            }
            continue;
        }
        break;
    }
    false
}

/// Lower bounds proving the scan actually looked at something. The classic
/// failure of a grep-shaped gate is going quietly vacuous: a parser change, a
/// renamed helper or a refactor that moves the code out from under the visitor
/// leaves it passing green while checking nothing.
///
/// Today's counts are 380 functions and 151 call sites. The floors are ~90% of
/// that (#2334 review tightened them from 250/100, which left enough headroom
/// that a change halving the visitor's reach would still have passed): close
/// enough to catch a scan that has lost a whole file or a whole node kind,
/// loose enough that ordinary churn -- a handful of functions gaining or
/// losing the parameter -- does not touch them. If a legitimate refactor
/// lowers the real count past a floor, move the floor *and* say in the commit
/// message what shrank; do not lower it to make a red test green.
const MIN_LIVE_OPTIONAL_FNS: usize = 340;
const MIN_SITES_EXAMINED: usize = 134;

struct Site {
    line: usize,
    func: String,
    snippet: String,
    /// Last line of the enclosing function's body, so the routing window can
    /// be clipped to it.
    body_end: usize,
    /// First line of the enclosing function's body, so the marker walk can be.
    body_start: usize,
}

/// One frame of the enclosing-function stack.
struct Frame {
    name: String,
    /// Whether the signature binds an `optional: bool` the body can read.
    live_optional: bool,
    /// 1-based, inclusive line range of the function's body.
    ///
    /// Load-bearing, not bookkeeping: without it the routing window runs
    /// straight off the end of the function and picks up a `suppresses(..)`
    /// belonging to the *next* one, silently excusing a real gap. Caught by
    /// this file's own negative test -- a freshly-added unrouted helper passed
    /// the audit because `each_repeat_generic`, twenty lines below it, was
    /// routed.
    body_start: usize,
    body_end: usize,
}

struct Audit<'a> {
    lines: Vec<&'a str>,
    /// Innermost enclosing function last. A stack, so a nested `fn` without
    /// the parameter does not inherit its parent's.
    stack: Vec<Frame>,
    live_optional_fns: usize,
    sites_examined: usize,
    /// Functions spelling the parameter `_optional` -- the exemption marker --
    /// while still reading it. See [`binds_live_optional`]'s doc comment.
    dishonest_underscore_fns: Vec<String>,
    /// Every raw site, resolved in a second pass -- see [`Audit::resolve`].
    candidates: Vec<Site>,
}

impl<'a> Audit<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            lines: src.lines().collect(),
            stack: Vec::new(),
            live_optional_fns: 0,
            sites_examined: 0,
            dishonest_underscore_fns: Vec::new(),
            candidates: Vec::new(),
        }
    }

    fn in_live_optional_fn(&self) -> bool {
        self.stack.last().is_some_and(|f| f.live_optional)
    }

    fn push_fn(&mut self, sig: &syn::Signature, body: &syn::Block) {
        // A function that spells the parameter `_optional` but reads it anyway
        // is claiming an exemption it does not have. Audit it as live *and*
        // report the spelling, so the fix is to rename rather than to bolt a
        // marker onto every site (#2334 review).
        let dishonest = binds_underscored_optional(sig) && body_reads_ident(body, "_optional");
        if dishonest {
            self.dishonest_underscore_fns.push(sig.ident.to_string());
        }
        let live_optional = binds_live_optional(sig) || dishonest;
        if live_optional {
            self.live_optional_fns += 1;
        }
        self.stack.push(Frame {
            name: sig.ident.to_string(),
            live_optional,
            body_start: body.span().start().line,
            body_end: body.span().end().line,
        });
    }

    /// Record a materialization site for [`Audit::resolve`] to judge.
    fn check(&mut self, span: Span) {
        if !self.in_live_optional_fn() {
            return;
        }
        self.sites_examined += 1;
        let line = span.start().line; // 1-based
        let frame = self
            .stack
            .last()
            .expect("in_live_optional_fn implies a frame");
        self.candidates.push(Site {
            line,
            func: frame.name.clone(),
            snippet: self
                .lines
                .get(line.saturating_sub(1))
                .unwrap_or(&"")
                .trim()
                .to_string(),
            body_start: frame.body_start,
            body_end: frame.body_end,
        });
    }

    /// Keep only the candidates that are neither routed nor exempted.
    ///
    /// A second pass rather than a decision made during the walk, because the
    /// routing window has to be clipped at the *next* materialization site,
    /// which is not known until the walk is done. Without that clip, a
    /// `suppress_or_raise` belonging to a later site in the same function
    /// silently excuses an earlier, genuinely unrouted one -- caught by this
    /// file's own negative test, where deleting one of `eval_index_expr`'s
    /// four key-arm markers left the audit green.
    fn resolve(mut self) -> Vec<Site> {
        self.candidates.sort_by_key(|c| c.line);
        let lines = core::mem::take(&mut self.lines);
        let starts: Vec<usize> = self.candidates.iter().map(|c| c.line).collect();
        let mut out = Vec::new();
        for (i, site) in self.candidates.into_iter().enumerate() {
            let idx = site.line.saturating_sub(1);
            let next_site = starts[i + 1..]
                .iter()
                .copied()
                .find(|l| *l > site.line)
                .unwrap_or(usize::MAX);
            let after_end = (idx + WINDOW_AFTER)
                .min(site.body_end)
                .min(next_site.saturating_sub(1))
                .min(lines.len())
                .max(idx + 1);
            let after = lines[idx..after_end].join("\n");
            if ROUTING_TOKENS.iter().any(|t| after.contains(t)) {
                continue;
            }
            // The site's own line counts too: a short marker on the very line
            // of the call is the terser spelling.
            if lines[idx].contains(EXEMPT_MARKER)
                || marker_in_attached_comment_block(&lines, idx, site.body_start)
            {
                continue;
            }
            out.push(site);
        }
        out
    }
}

/// Whether `sig` binds an `optional: bool` the body can actually read.
///
/// `_optional: bool` does not count: an underscore-prefixed parameter is the
/// codebase's existing marker for "this function ignores `optional` on
/// purpose" -- `builtin_recurse_f`/`builtin_recurse_cond` (#1953) are spelled
/// that way. STYLE-0012 names it as the preferred marker when the whole
/// parameter is dead.
///
/// The unused-variable lint keeps that spelling *mostly* honest, but only
/// mostly: it fires on an unused binding, never on an underscored one that is
/// used. So a function could spell the parameter `_optional`, read it anyway,
/// and silently take its whole body out of this audit with nothing complaining
/// (#2334 review). [`binds_underscored_optional`] plus [`body_reads_ident`]
/// close that: the main test reports any function that does both.
fn binds_live_optional(sig: &syn::Signature) -> bool {
    binds_optional_named(sig, "optional")
}

/// The `_optional` spelling of the same parameter -- the exemption marker.
fn binds_underscored_optional(sig: &syn::Signature) -> bool {
    binds_optional_named(sig, "_optional")
}

fn binds_optional_named(sig: &syn::Signature, want: &str) -> bool {
    sig.inputs.iter().any(|arg| {
        let syn::FnArg::Typed(pat) = arg else {
            return false;
        };
        let syn::Pat::Ident(ident) = &*pat.pat else {
            return false;
        };
        if ident.ident != want {
            return false;
        }
        matches!(&*pat.ty, syn::Type::Path(p) if p.path.is_ident("bool"))
    })
}

/// Whether `body` reads `name` anywhere -- as a value, or inside a macro's
/// token stream (which `syn` never parses into expressions, so the visitor
/// below has to check the raw tokens too).
fn body_reads_ident(body: &syn::Block, name: &str) -> bool {
    struct Reads<'a> {
        name: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Reads<'_> {
        fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
            if node.path.is_ident(self.name) {
                self.found = true;
            }
            visit::visit_expr_path(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            let flat = node.tokens.to_string();
            if flat
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|w| w == self.name)
            {
                self.found = true;
            }
            visit::visit_macro(self, node);
        }
    }
    let mut v = Reads { name, found: false };
    v.visit_block(body);
    v.found
}

/// Whether the item is `#[cfg(test)]`-gated, and so legitimately outside the
/// audit: unit tests call the materializers directly with hand-picked
/// `optional` values.
///
/// `not(test)` is explicitly *not* this: an item compiled only in non-test
/// builds is ordinary production code, and skipping it would be a silent hole
/// exactly the shape this audit exists to close (#2334 review). Neither file
/// has one today; the check is here so that adding one is not a quiet
/// exemption.
fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if !a.path().is_ident("cfg") {
            return false;
        }
        let tokens = a.to_token_stream_string();
        tokens.contains("test") && !tokens.replace(' ', "").contains("not(test)")
    })
}

/// `syn::Attribute` has no direct "render me" method that keeps working across
/// meta shapes, so go through the token stream.
trait TokenStreamString {
    fn to_token_stream_string(&self) -> String;
}

impl TokenStreamString for syn::Attribute {
    fn to_token_stream_string(&self) -> String {
        match &self.meta {
            syn::Meta::List(list) => list.tokens.to_string(),
            other => format!("{:?}", std::mem::discriminant(other)),
        }
    }
}

impl<'ast> Visit<'ast> for Audit<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        // `#[cfg(test)] mod tests` -- unit tests legitimately call the
        // materializers directly with hand-picked `optional` values.
        if has_cfg_test(&node.attrs) {
            return;
        }
        visit::visit_item_mod(self, node);
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if has_cfg_test(&node.attrs) {
            return;
        }
        self.push_fn(&node.sig, &node.block);
        visit::visit_item_fn(self, node);
        self.stack.pop();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if has_cfg_test(&node.attrs) {
            return;
        }
        self.push_fn(&node.sig, &node.block);
        visit::visit_impl_item_fn(self, node);
        self.stack.pop();
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        let Some(body) = &node.default else {
            return;
        };
        self.push_fn(&node.sig, body);
        visit::visit_trait_item_fn(self, node);
        self.stack.pop();
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*node.func {
            if let Some(last) = path.path.segments.last() {
                if is_materializer_fn(&last.ident.to_string()) {
                    self.check(node.span());
                }
            }
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if MATERIALIZER_METHODS.contains(&node.method.to_string().as_str()) {
            self.check(node.span());
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        let name = node
            .mac
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if ROUTED_MACROS.contains(&name.as_str()) {
            // Counted, not just skipped: `sites_examined` has to be stable
            // under the very work this audit exists to cause. Counting only
            // *raw* sites would make the number fall every time somebody
            // routes one, so the anti-vacuity floor below would start
            // fighting the fix.
            if self.in_live_optional_fn() {
                self.sites_examined += 1;
            }
            return;
        }
        // An unrouted macro can still hide a materialization in its token
        // stream -- `owned_or_err!(to_owned(&v))` and `push_or_control!(..)`
        // both do, and both ignore `optional`. syn never parses macro tokens
        // into expressions, so without this the visitor would walk straight
        // past exactly the shape this audit exists to catch.
        let hides_materializer = node.mac.tokens.clone().into_iter().any(|t| {
            let name = t.to_string();
            is_materializer_fn(&name) || MATERIALIZER_METHODS.contains(&name.as_str())
        }) || {
            // Nested groups (`owned_or_err!(x.map(|v| to_owned(&v)))`) arrive as
            // a single `Group` token, so fall back to the flattened text for
            // those -- `to_string()` on the whole stream inserts spaces around
            // punctuation, so match on whitespace-delimited words.
            let flat = node.mac.tokens.to_string();
            flat.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|w| is_materializer_fn(w) || MATERIALIZER_METHODS.contains(&w))
        };
        if hides_materializer {
            self.check(node.span());
        }
        visit::visit_expr_macro(self, node);
    }
}

#[test]
fn test_every_optional_materialization_site_is_routed_or_exempt_2334() {
    let mut live_optional_fns = 0usize;
    let mut sites_examined = 0usize;
    let mut report = String::new();
    let mut violations = 0usize;
    let mut offending_files = BTreeSet::new();
    let mut dishonest: Vec<String> = Vec::new();

    for (path, src) in SOURCES {
        let file = syn::parse_file(src).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut audit = Audit::new(src);
        audit.visit_file(&file);

        live_optional_fns += audit.live_optional_fns;
        sites_examined += audit.sites_examined;
        dishonest.extend(
            audit
                .dishonest_underscore_fns
                .iter()
                .map(|f| format!("{path}: {f}")),
        );

        for site in &audit.resolve() {
            violations += 1;
            offending_files.insert(*path);
            let _ = writeln!(
                report,
                "  {path}:{} in `{}`\n      {}",
                site.line, site.func, site.snippet
            );
        }
    }

    assert!(
        report.is_empty(),
        "STYLE-0012 violation: {violations} raw materialization site(s) in \
         {} file(s), inside a function \
         with a live `optional: bool`, neither routed through the suppression \
         machinery nor exempted.\n\n{report}\nResolve each one either by:\n  \
         (a) routing it -- `to_owned_or_suppress!`/`to_owned_vec_or_suppress!` \
         (eval.rs), `owned_or_suppress!` (eval_generic.rs), or a hand-rolled \
         `Err(e) => return suppress_or_raise(e, optional)`; or\n  \
         (b) marking it `// STYLE-0012: <why this site must raise regardless of \
         optional>` on the line itself, or anywhere in the comment block \
         attached directly above it.\n\n\
         See docs/STYLE_GUIDE.md STYLE-0012 and issue #2334. This is not a \
         behaviour change either way today -- see this file's own header for \
         why that is exactly the reason the check has to be static.",
        offending_files.len(),
    );

    assert!(
        dishonest.is_empty(),
        "STYLE-0012: {} function(s) spell the parameter `_optional` -- the \
         exemption marker that takes the whole function out of this audit -- \
         and then read it anyway:\n  {}\n\nThe unused-variable lint cannot \
         catch that: it fires on an unused binding, never on an underscored \
         one that is used. Rename the parameter to `optional` (and route or \
         mark its materialization sites), or stop reading it.",
        dishonest.len(),
        dishonest.join("\n  "),
    );

    assert!(
        live_optional_fns >= MIN_LIVE_OPTIONAL_FNS,
        "audit went vacuous: only {live_optional_fns} functions with a live \
         `optional: bool` were found across {} file(s), expected at least \
         {MIN_LIVE_OPTIONAL_FNS}. The visitor has probably stopped seeing the \
         evaluators (a moved module, a renamed parameter, a syn upgrade) -- \
         fix the scan, do not lower the bound.",
        SOURCES.len(),
    );
    assert!(
        sites_examined >= MIN_SITES_EXAMINED,
        "audit went vacuous: only {sites_examined} materialization call sites \
         were examined, expected at least {MIN_SITES_EXAMINED}. See the \
         `live_optional_fns` assertion above for what usually causes this.",
    );
}

/// The two exemption spellings, pinned so a refactor cannot quietly turn either
/// into a no-op.
///
/// `builtin_recurse_f`/`builtin_recurse_cond` (#1953) must stay invisible to
/// the audit through `_optional`, and `eval_array_construction` (#2327's own
/// investigated-and-declined candidate) must stay *visible* and carry a
/// `// STYLE-0012:` marker. If the first ever gains a live `optional`, or the
/// second loses its marker, the main test above should start reporting it --
/// this test fails first, with a message saying which assumption moved.
#[test]
fn test_style_0012_exemption_spellings_are_still_in_use_2334() {
    let eval_rs = SOURCES[0].1;

    assert!(
        eval_rs.contains("fn builtin_recurse_f"),
        "builtin_recurse_f was renamed or removed; re-check the #1953 exemption",
    );
    for f in ["builtin_recurse_f", "builtin_recurse_cond"] {
        let body = &eval_rs[eval_rs.find(&format!("fn {f}")).expect(f)..];
        let sig_end = body.find(" {").expect("signature");
        assert!(
            body[..sig_end].contains("_optional: bool"),
            "{f} no longer spells its parameter `_optional`. That spelling is \
             its #1953 exemption and the only thing keeping it out of the \
             STYLE-0012 audit -- either restore it or give the function's \
             `to_owned` sites explicit `// STYLE-0012:` markers.",
        );
    }

    assert!(
        eval_rs.contains(EXEMPT_MARKER),
        "no `// {EXEMPT_MARKER}` marker left in eval.rs -- the exemption \
         convention has been removed without removing the audit",
    );
}

/// Pins the premise `MATERIALIZER_FN_EXCLUSIONS`'s `to_owned_checked` entry
/// rests on (#2367).
///
/// `to_owned_checked`/`to_owned_checked_at_depth` are excluded from the main
/// audit above because they are the one member of the materialization family
/// whose error is not always `decode_failure`-tagged (#2299) -- routing them
/// through `debug_assert_materialization_error` would misfire on exactly the
/// nesting-depth error they exist to return, not a bug. That exclusion is
/// sound only because the family's one call site,
/// `jq_runner.rs::materialize_stream_item`, sits outside both evaluators
/// scanned here and has no live `optional` in scope. Neither half of the
/// STYLE-0012 guard would notice a second call site appearing inside
/// `eval.rs`/`eval_generic.rs`: the exclusion list keeps it invisible to the
/// main audit above, and the runtime assert in `error.rs` cannot be added at
/// its call site without misfiring on the very error class it returns by
/// design. Rather than widen either half to special-case a function that
/// cannot use them, this test pins the premise directly: neither evaluator
/// may call `to_owned_checked`/`to_owned_checked_at_depth` at all, outside
/// their own mutually-recursive definitions.
#[test]
fn test_to_owned_checked_has_no_call_sites_outside_its_own_definition_2367() {
    const SELF_FAMILY: &[&str] = &["to_owned_checked", "to_owned_checked_at_depth"];

    struct Finder<'a> {
        lines: &'a [&'a str],
        path: &'a str,
        stack: Vec<String>,
        violations: Vec<String>,
    }

    impl Finder<'_> {
        fn in_self_family(&self) -> bool {
            self.stack
                .last()
                .is_some_and(|f| SELF_FAMILY.contains(&f.as_str()))
        }

        fn record(&mut self, name: &str, span: Span) {
            if self.in_self_family() {
                return;
            }
            let line = span.start().line;
            let snippet = self
                .lines
                .get(line.saturating_sub(1))
                .map(|s| s.trim())
                .unwrap_or_default();
            self.violations.push(format!(
                "{}:{line} in `{}` calls `{name}`:\n      {snippet}",
                self.path,
                self.stack.last().map_or("<top level>", String::as_str),
            ));
        }
    }

    impl<'ast> Visit<'ast> for Finder<'_> {
        fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
            // `#[cfg(test)] mod tests` -- unit tests legitimately call
            // `to_owned_checked` directly (#2299's own tests do, right next
            // to its definition), same exemption as the main audit's `Audit`
            // above.
            if has_cfg_test(&node.attrs) {
                return;
            }
            visit::visit_item_mod(self, node);
        }

        fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
            // A standalone `#[cfg(test)] fn` (not wrapped in a `#[cfg(test)]
            // mod`) is the same legitimate-unit-test case `visit_item_mod`
            // above exempts -- checked here too so one isn't missed.
            if has_cfg_test(&node.attrs) {
                return;
            }
            self.stack.push(node.sig.ident.to_string());
            visit::visit_item_fn(self, node);
            self.stack.pop();
        }

        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            if has_cfg_test(&node.attrs) {
                return;
            }
            self.stack.push(node.sig.ident.to_string());
            visit::visit_impl_item_fn(self, node);
            self.stack.pop();
        }

        fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
            if let syn::Expr::Path(path) = &*node.func {
                if let Some(last) = path.path.segments.last() {
                    let name = last.ident.to_string();
                    if SELF_FAMILY.contains(&name.as_str()) {
                        self.record(&name, node.span());
                    }
                }
            }
            visit::visit_expr_call(self, node);
        }

        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            // Same defense as the main audit's own macro scan: a call hidden
            // inside a macro's unparsed token stream would otherwise walk
            // straight past this visitor.
            let flat = node.tokens.to_string();
            let words: Vec<&str> = flat
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .collect();
            for name in SELF_FAMILY {
                if words.contains(name) {
                    self.record(name, node.span());
                }
            }
            visit::visit_macro(self, node);
        }
    }

    let mut violations = Vec::new();
    for (path, src) in SOURCES {
        let file = syn::parse_file(src).unwrap_or_else(|e| panic!("{path}: {e}"));
        let lines: Vec<&str> = src.lines().collect();
        let mut finder = Finder {
            lines: &lines,
            path,
            stack: Vec::new(),
            violations: Vec::new(),
        };
        finder.visit_file(&file);
        violations.append(&mut finder.violations);
    }

    assert!(
        violations.is_empty(),
        "#2367: {} call site(s) of `to_owned_checked`/`to_owned_checked_at_depth` \
         found outside their own definition:\n\n{}\n\n\
         `to_owned_checked` is deliberately excluded from both halves of the \
         STYLE-0012 guard (see `MATERIALIZER_FN_EXCLUSIONS`'s doc comment) \
         because it is the one member of the materialization family that can \
         raise a non-decode-failure error, and its only sanctioned call site \
         (`jq_runner.rs::materialize_stream_item`) is outside both evaluators. \
         A new call site inside `eval.rs`/`eval_generic.rs` invalidates that \
         exclusion -- see issue #2367 for what to do instead (route it through \
         the main audit like any other materializer, or give it its own \
         reasoned treatment) before adding one.",
        violations.len(),
        violations.join("\n"),
    );
}
