//! STYLE-0012's enforcement: every raw materialization call site inside a
//! function with a live `optional: bool` must either route its error through
//! the suppression machinery or carry a `// STYLE-0012:` exemption (#2334).
//!
//! # Why this is a source scan and not a behavioural test
//!
//! `eval::suppresses` is `optional && !e.is_decode_failure()`. Every error the
//! materialization family can raise today is `decode_failure`-tagged --
//! `to_owned`/`to_owned_at_depth`, `to_owned_cursor_at_depth`,
//! `to_owned_with_cursor` and `collect_cursors_checked`, whose only error
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
/// - `to_owned_checked` is the #2299 sibling of `to_owned` that swaps
///   `assert_nesting_depth`'s panic for a *catchable* `check_nesting_depth`
///   error -- the one member of the family that genuinely can raise a
///   non-decode-failure. It has a single call site, `materialize_stream_item`
///   in the CLI crate's `jq_runner.rs`, which has no `optional` in scope and is
///   not one of the files scanned here. Leaving it out keeps the premise
///   `debug_assert_materialization_error` pins ("this family raises only decode
///   failures") true of exactly the set the audit covers.
const MATERIALIZER_FN_EXCLUSIONS: &[&str] = &[
    "to_owned_lossy",
    "to_owned_lossy_at_depth",
    "to_owned_for_diagnostic",
    "to_owned_checked",
    "to_owned_checked_at_depth",
];

fn is_materializer_fn(name: &str) -> bool {
    name.starts_with(MATERIALIZER_FN_PREFIX) && !MATERIALIZER_FN_EXCLUSIONS.contains(&name)
}

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
/// Today's counts are 381 functions and 148 call sites. These floors sit well
/// below that -- low enough not to churn on ordinary edits, high enough that a
/// scan which has stopped seeing the evaluators cannot pass.
const MIN_LIVE_OPTIONAL_FNS: usize = 250;
const MIN_SITES_EXAMINED: usize = 100;

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

struct Audit<'a> {
    lines: Vec<&'a str>,
    /// Innermost enclosing function: name, whether it binds a live
    /// `optional: bool`, and the 1-based line range of its body. A stack, so a
    /// nested `fn` without the parameter does not inherit its parent's.
    ///
    /// The line range is load-bearing, not bookkeeping: without it the
    /// routing window below runs straight off the end of the function and
    /// picks up a `suppresses(..)` belonging to the *next* one, silently
    /// excusing a real gap. Caught by this file's own negative test -- a
    /// freshly-added unrouted helper passed the audit because
    /// `each_repeat_generic`, twenty lines below it, was routed.
    stack: Vec<(String, bool, usize, usize)>,
    live_optional_fns: usize,
    sites_examined: usize,
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
            candidates: Vec::new(),
        }
    }

    fn in_live_optional_fn(&self) -> bool {
        self.stack.last().is_some_and(|(_, live, _, _)| *live)
    }

    fn enclosing(&self) -> String {
        self.stack
            .last()
            .map(|(name, _, _, _)| name.clone())
            .unwrap_or_else(|| "<unknown>".to_string())
    }

    /// The enclosing function's own body lines, as a 1-based inclusive range.
    fn body_range(&self) -> (usize, usize) {
        self.stack
            .last()
            .map(|(_, _, start, end)| (*start, *end))
            .unwrap_or((1, self.lines.len()))
    }

    fn push_fn(&mut self, sig: &syn::Signature, body: &syn::Block) {
        let live = binds_live_optional(sig);
        if live {
            self.live_optional_fns += 1;
        }
        self.stack.push((
            sig.ident.to_string(),
            live,
            body.span().start().line,
            body.span().end().line,
        ));
    }

    /// Record a materialization site for [`Audit::resolve`] to judge.
    fn check(&mut self, span: Span) {
        if !self.in_live_optional_fn() {
            return;
        }
        self.sites_examined += 1;
        let line = span.start().line; // 1-based
        let (body_start, body_end) = self.body_range();
        self.candidates.push(Site {
            line,
            func: self.enclosing(),
            snippet: self
                .lines
                .get(line.saturating_sub(1))
                .unwrap_or(&"")
                .trim()
                .to_string(),
            body_end,
            body_start,
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
/// codebase's existing, *compiler-enforced* marker for "this function ignores
/// `optional` on purpose" -- `builtin_recurse_f`/`builtin_recurse_cond` (#1953)
/// are spelled that way, and the unused-variable lint keeps the spelling honest
/// in a way no comment can. STYLE-0012 names it as the preferred marker when
/// the whole parameter is dead.
fn binds_live_optional(sig: &syn::Signature) -> bool {
    sig.inputs.iter().any(|arg| {
        let syn::FnArg::Typed(pat) = arg else {
            return false;
        };
        let syn::Pat::Ident(ident) = &*pat.pat else {
            return false;
        };
        if ident.ident != "optional" {
            return false;
        }
        matches!(&*pat.ty, syn::Type::Path(p) if p.path.is_ident("bool"))
    })
}

fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("cfg") && a.to_token_stream_string().contains("test"))
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

    for (path, src) in SOURCES {
        let file = syn::parse_file(src).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut audit = Audit::new(src);
        audit.visit_file(&file);

        live_optional_fns += audit.live_optional_fns;
        sites_examined += audit.sites_examined;

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
