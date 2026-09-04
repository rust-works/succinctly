//! STYLE-0013's enforcement: every call to a member-validation primitive
//! outside its own definition site must either be inside the shared helper
//! that owns it, or carry a `// STYLE-0013:` exemption (#1803).
//!
//! # Why this is a source scan and not a behavioural test
//!
//! The member rules -- #1642's colliding-key guard, #1194's structural-key
//! check, #1677's `,`/`:` delimiter pair -- accumulated across four issues,
//! and each round added its check to some of the walks that needed it and
//! not others. #1975 found the CLI bridge with none of it; #2211 and #2243
//! each reached a different subset; #2349 is the wrong-answer bug the drift
//! finally produced in `select`/`sort_by`/`path()`.
//!
//! Extraction alone was already tried, and already failed to hold.
//! `DocumentCursor::element_gap_ok` was pulled out by #1597's review for
//! exactly this reason, and five sites went on hand-rolling its four lines
//! -- two of them written *after* it existed. Centralising a check does not
//! stop the omission, because nothing forces a call site through it.
//!
//! And a behavioural test cannot close the gap either, for two structural
//! reasons rather than by bad luck:
//!
//! - **The rules are JSON-only.** Every delimiter check is a
//!   `true`-returning `DocumentCursor` default, and YAML never overrides
//!   one, because its parser validates while it parses. A walk reachable
//!   only from YAML shows no difference whether it checks or not.
//! - **The corruption need never be emitted.** A subtree a query merely
//!   *validates* and never prints is where #2349 hid: the materializing
//!   sibling raises on the same document, so nothing looks wrong until you
//!   ask that one walk directly.
//!
//! #1962's cross-site consistency net did drive three of the sites and still
//! missed it, because its `FuzzMalformation` alphabet has only
//! decode-failure, structural and collision variants and its generator emits
//! only well-formed containers -- it cannot express a delimiter corruption at
//! any case count (#2350). A static audit does not depend on a corruption
//! being reachable, expressible, or observable.
//!
//! Same answer, same shape as `jq_optional_suppression_audit.rs` (STYLE-0012,
//! #2334), which this file is modelled on.
//!
//! # What it does not do
//!
//! It audits the **member** primitives only -- the pair `checked_key` and
//! `delimiters_ok` own. It deliberately does not audit the post-loop tail
//! (`container_gap_ok`, `trailing_element_gap_ok`), even though #2211 and
//! #2243 drifted there too: those have many legitimate direct callers
//! outside the walk shape (`json::light`'s streaming writers, `document.rs`'s
//! key-only walks, the tail helpers themselves), so a name-based rule would
//! be mostly noise and would train readers to add the marker reflexively.
//! Auditing that half wants a rule shaped around *the walk* -- "a loop that
//! unconses children must end in a tail helper" -- which is a different and
//! larger piece of analysis than this file does.
//!
//! It also does not decide whether a site *should* route. It only demands
//! that somebody decided and wrote the reason down.

use proc_macro2::Span;
use syn::spanned::Spanned as _;
use syn::visit::{self, Visit};

/// Every file that walks a document's members. `document.rs` is excluded on
/// purpose: it *defines* the primitives and the helpers that own them, so
/// every call in it is either a definition or the one legitimate use.
///
/// `yq_runner.rs` is in the `succinctly-cli` crate, not this library --
/// included via `include_str!` for the same reason it is in the audited set
/// at all: it is the walk that has fallen behind twice (#1975 found it
/// missing the delimiter checks *and* silently dropping undecodable keys),
/// and a scan that stopped at the crate boundary would have caught neither.
///
/// `jq_runner.rs` (its JSON-only sibling, also `succinctly-cli`) hand-rolls
/// the same `,`/`:` checks via `preceding_gap_ok` across four functions
/// (`standard_json_to_jq_value`, `check_preceding_delimiter`,
/// `validate_json_delimiters`, `print_json`), each with its own
/// `// STYLE-0013:` exemption citing the perf rationale already documented
/// beside every one of those calls (#1643/#1676). Left out of `SOURCES`
/// until now for no recorded reason -- the same "stopped at the crate
/// boundary" gap `yq_runner.rs`'s own inclusion above exists to close.
const SOURCES: &[(&str, &str)] = &[
    ("src/jq/eval.rs", include_str!("../src/jq/eval.rs")),
    (
        "src/jq/eval_generic.rs",
        include_str!("../src/jq/eval_generic.rs"),
    ),
    ("src/jq/lazy.rs", include_str!("../src/jq/lazy.rs")),
    ("src/json/light.rs", include_str!("../src/json/light.rs")),
    (
        "src/bin/succinctly/yq_runner.rs",
        include_str!("../src/bin/succinctly/yq_runner.rs"),
    ),
    (
        "src/bin/succinctly/jq_runner.rs",
        include_str!("../src/bin/succinctly/jq_runner.rs"),
    ),
];

/// The primitives `DocumentField::checked_key` and
/// `DocumentField::delimiters_ok` exist to own (#1803).
///
/// `preceding_delimiter_ok` is here for the `is_first` -> `Option<b','>`
/// mapping specifically: `element_gap_ok`/`element_gap_ok_at` are the two
/// definitions of it, and a third re-derivation at a call site is the shape
/// #1597 extracted and five sites re-grew anyway.
///
/// `preceding_gap_ok` is `preceding_delimiter_ok`'s own JSON backing
/// implementation (`JsonCursor::preceding_delimiter_ok` is a one-line
/// wrapper around it) -- a call to it outside that wrapper is the exact
/// same `is_first -> Option<b','>` re-derivation one layer lower, so it
/// belongs on this list for the same reason `preceding_delimiter_ok` does.
const AUDITED: &[&str] = &[
    "resolve_display_key",
    "key_delimiter_ok",
    "value_delimiter_ok",
    "preceding_delimiter_ok",
    "preceding_gap_ok",
];

/// The STYLE-0013 exemption marker, cited inline the way STYLE-0004's
/// `#[allow]` citations and STYLE-0012's routing exemptions already are.
const EXEMPT_MARKER: &str = "STYLE-0013:";

/// Lower bounds proving the scan actually looked at something.
///
/// The classic failure of a grep-shaped gate is going quietly vacuous: a
/// renamed helper, a `syn` upgrade, or a refactor that moves code out from
/// under the visitor leaves it passing green while checking nothing. Today
/// the scan sees 20 audited call sites across 6 files (verified by
/// instrumenting `run_audit()` directly, not hand-counted -- the exact
/// per-walk breakdown drifts too easily to keep current in prose, which is
/// the same lesson this whole file exists to enforce on the production
/// code); the floors sit just under that. If a legitimate refactor lowers
/// the real count past a floor, move the floor *and* say in the commit
/// message what shrank -- do not lower it to make a red test green.
const MIN_SITES_EXAMINED: usize = 12;
const MIN_FILES_PARSED: usize = 6;

struct Site {
    file: &'static str,
    line: usize,
    func: String,
    callee: String,
}

/// One frame of the enclosing-function stack.
struct Frame {
    name: String,
    /// 1-based, inclusive line range of the function's body.
    ///
    /// Load-bearing, not bookkeeping: the marker search is clipped to it, so
    /// a `// STYLE-0013:` belonging to one walk cannot excuse an unmarked
    /// call in the next function down. This file's own negative test pins
    /// that -- see `test_marker_does_not_leak_across_function_boundaries`.
    body_start: usize,
    body_end: usize,
}

struct Audit<'a> {
    file: &'static str,
    lines: Vec<&'a str>,
    /// Innermost enclosing function last, so a nested `fn` does not inherit
    /// its parent's marker.
    stack: Vec<Frame>,
    sites_examined: usize,
    violations: Vec<Site>,
}

/// Whether a `// STYLE-0013:` marker appears anywhere inside this function's
/// own body.
///
/// Function-level, not "the comment block attached to the call": a walk that
/// cannot route is exempt *as a walk*, and its reason is one paragraph, not
/// one per primitive it touches. `push_generic_truthiness_cursor_error` is
/// the case that settles it -- one exemption, two `resolve_display_key`
/// calls, and requiring the paragraph twice would say less, not more.
///
/// The clipping to `body_start..=body_end` is what keeps that from being
/// laxer than the per-call form in the way that matters: the marker must be
/// inside the *same* function as the call.
fn marker_in_body(lines: &[&str], frame: &Frame) -> bool {
    let lo = frame.body_start.saturating_sub(1);
    let hi = frame.body_end.min(lines.len());
    lines[lo..hi].iter().any(|l| is_marker_line(l))
}

/// Whether one line *is* a STYLE-0013 citation, as opposed to prose that
/// merely mentions the rule.
///
/// The marker must **open** the comment -- `// STYLE-0013: <reason>` -- not
/// appear anywhere in it. That is the documented citation form (STYLE-0004's
/// `#[allow]` citations and STYLE-0012's exemptions are both written this
/// way), and requiring it is load-bearing rather than pedantic: the first
/// draft of this file used `contains`, and a sentence in
/// `push_generic_truthiness_cursor_error`'s own comment -- "This marker is
/// the point of STYLE-0013: ..." -- silently exempted that function. The
/// audit passed with its real marker deleted. A gate that a rule's own
/// *explanation* can satisfy is not a gate; see
/// `test_prose_mentioning_the_rule_does_not_exempt`.
fn is_marker_line(line: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix("//") else {
        return false;
    };
    rest.trim_start().starts_with(EXEMPT_MARKER)
}

/// The bare name a call expression resolves to, for both `foo(..)` /
/// `path::foo(..)` and `x.foo(..)`.
fn callee_name(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Call(c) => match &*c.func {
            syn::Expr::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
            _ => None,
        },
        syn::Expr::MethodCall(m) => Some(m.method.to_string()),
        _ => None,
    }
}

fn line_of(span: Span) -> usize {
    span.start().line
}

impl<'a> Audit<'a> {
    fn new(file: &'static str, src: &'a str) -> Self {
        Self {
            file,
            lines: src.lines().collect(),
            stack: Vec::new(),
            sites_examined: 0,
            violations: Vec::new(),
        }
    }

    fn enter(&mut self, name: String, body: Span) {
        self.stack.push(Frame {
            name,
            body_start: body.start().line,
            body_end: body.end().line,
        });
    }

    fn check_call(&mut self, expr: &syn::Expr) {
        let Some(callee) = callee_name(expr) else {
            return;
        };
        if !AUDITED.contains(&callee.as_str()) {
            return;
        }
        // A call inside the function that *is* the primitive, or inside a
        // trait impl of it, is the definition, not a hand-copy.
        let Some(frame) = self.stack.last() else {
            return;
        };
        if AUDITED.contains(&frame.name.as_str()) {
            return;
        }
        self.sites_examined += 1;
        if !marker_in_body(&self.lines, frame) {
            self.violations.push(Site {
                file: self.file,
                line: line_of(expr.span()),
                func: frame.name.clone(),
                callee,
            });
        }
    }
}

impl<'ast> Visit<'ast> for Audit<'_> {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.enter(node.sig.ident.to_string(), node.block.span());
        visit::visit_item_fn(self, node);
        self.stack.pop();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.enter(node.sig.ident.to_string(), node.block.span());
        visit::visit_impl_item_fn(self, node);
        self.stack.pop();
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        if let Some(block) = &node.default {
            self.enter(node.sig.ident.to_string(), block.span());
            visit::visit_trait_item_fn(self, node);
            self.stack.pop();
        } else {
            visit::visit_trait_item_fn(self, node);
        }
    }

    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        self.check_call(node);
        visit::visit_expr(self, node);
    }
}

fn run_audit() -> (Vec<Site>, usize, usize) {
    let mut violations = Vec::new();
    let mut examined = 0usize;
    let mut files = 0usize;
    for (path, src) in SOURCES {
        let parsed = syn::parse_file(src)
            .unwrap_or_else(|e| panic!("STYLE-0013 audit could not parse {path}: {e}"));
        let mut audit = Audit::new(path, src);
        audit.visit_file(&parsed);
        examined += audit.sites_examined;
        violations.extend(audit.violations);
        files += 1;
    }
    (violations, examined, files)
}

#[test]
fn style_0013_member_validation_is_routed_or_exempted() {
    let (violations, examined, files) = run_audit();

    assert!(
        files >= MIN_FILES_PARSED,
        "STYLE-0013 audit parsed only {files} of {MIN_FILES_PARSED} expected files -- the \
         scan has lost a source, not found a clean tree"
    );
    assert!(
        examined >= MIN_SITES_EXAMINED,
        "STYLE-0013 audit examined only {examined} call sites, expected at least \
         {MIN_SITES_EXAMINED}. A vacuous scan passes green: check that the audited names in \
         `AUDITED` still exist and that the visitor still reaches them, before lowering this \
         floor."
    );

    if !violations.is_empty() {
        let mut report = String::new();
        for v in &violations {
            report.push_str(&format!(
                "\n  {}:{} in `{}` calls `{}`",
                v.file, v.line, v.func, v.callee
            ));
        }
        panic!(
            "STYLE-0013 violation: {} member-validation call site(s) neither route through the \
             shared helper nor carry a `// STYLE-0013:` exemption:{}\n\n\
             Fix by either (a) routing through `DocumentField::checked_key` (key resolution + \
             delimiters) or `DocumentField::delimiters_ok` (delimiters only, allocation-free), \
             or (b) writing `// STYLE-0013: <why this walk cannot route>` inside the function. \
             See docs/STYLE_GUIDE.md.",
            violations.len(),
            report
        );
    }
}

/// The audit must fire on a hand-rolled site. Without this, a visitor that
/// reaches nothing passes for the same reason a clean tree does.
#[test]
fn test_audit_fires_on_an_unmarked_hand_rolled_walk() {
    let src = r"
        fn walks_an_object<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                let Some(key) = resolve_display_key(&field.key, &map, &mut guard)? else {
                    return Err(f.malformed_member_error());
                };
                f = rest;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(
        audit.sites_examined, 1,
        "the visitor must reach the hand-rolled call"
    );
    assert_eq!(
        audit.violations.len(),
        1,
        "an unmarked hand-rolled walk must be reported"
    );
    assert_eq!(audit.violations[0].func, "walks_an_object");
}

/// The same fixture with the marker inside the function must pass -- the
/// escape hatch has to actually work, or the rule is unusable and people
/// will delete the test rather than the duplication.
#[test]
fn test_audit_accepts_a_marked_walk() {
    let src = r"
        fn walks_an_object<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            // STYLE-0013: key-only walk, no value resolved to check a `:` against.
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                let Some(key) = resolve_display_key(&field.key, &map, &mut guard)? else {
                    return Err(f.malformed_member_error());
                };
                f = rest;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.sites_examined, 1);
    assert!(
        audit.violations.is_empty(),
        "a marked walk must be accepted: {:?}",
        audit.violations.iter().map(|v| v.line).collect::<Vec<_>>()
    );
}

/// A marker in one function must not excuse an unmarked call in the next.
///
/// This is the failure the STYLE-0012 audit hit for real: without clipping to
/// the enclosing function's body, a freshly-added unrouted helper passed
/// because a *neighbour* twenty lines below was routed. Same trap, pinned
/// here before it can be sprung.
#[test]
fn test_marker_does_not_leak_across_function_boundaries() {
    let src = r"
        fn exempt_walk<F: DocumentFields>(fields: &F) -> bool {
            // STYLE-0013: this one has a real reason.
            key_delimiter_ok::<F>(&key, &cursor, is_first)
        }

        fn unmarked_walk<F: DocumentFields>(fields: &F) -> bool {
            value_delimiter_ok::<F>(Some(&field.value), &field.value_cursor)
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.sites_examined, 2, "both calls must be examined");
    assert_eq!(
        audit.violations.len(),
        1,
        "exactly the unmarked function must be reported, not both and not neither"
    );
    assert_eq!(audit.violations[0].func, "unmarked_walk");
}

/// A nested `fn` must not inherit its parent's marker. The stack is what
/// makes this true; a single "current function" field would not.
#[test]
fn test_nested_fn_does_not_inherit_the_parent_marker() {
    let src = r"
        fn outer<F: DocumentFields>(fields: &F) -> bool {
            // STYLE-0013: the outer walk's reason.
            fn inner<F: DocumentFields>(fields: &F) -> bool {
                key_delimiter_ok::<F>(&key, &cursor, is_first)
            }
            inner(fields)
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(
        audit.violations.len(),
        1,
        "the nested fn must be reported despite the parent's marker"
    );
    assert_eq!(audit.violations[0].func, "inner");
}

/// Prose that merely mentions the rule must not exempt a function -- only a
/// comment that *opens* with the citation does.
///
/// Not hypothetical: this is the false negative the first draft of this file
/// actually had, found by deleting a real marker from
/// `push_generic_truthiness_cursor_error` and watching the audit stay green,
/// because that function's own explanation contains the words "the point of
/// STYLE-0013:". The rule's rationale is exactly the prose most likely to
/// name the rule, so this is the failure mode a `contains` check is *most*
/// prone to, not least.
#[test]
fn test_prose_mentioning_the_rule_does_not_exempt() {
    let src = r"
        fn walks_an_object<F: DocumentFields>(fields: &F) -> bool {
            // This walk is the reason STYLE-0013: exists, historically.
            // Some more prose about STYLE-0013: and what it is for.
            key_delimiter_ok::<F>(&key, &cursor, is_first)
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(
        audit.violations.len(),
        1,
        "prose mentioning the rule must not count as a citation"
    );
}

/// The citation form itself, pinned in both directions so a future tweak to
/// [`is_marker_line`] cannot quietly loosen or tighten it.
#[test]
fn test_marker_line_recognition() {
    assert!(is_marker_line("// STYLE-0013: a reason"));
    assert!(is_marker_line("        // STYLE-0013: indented"));
    assert!(is_marker_line("//STYLE-0013: no space after slashes"));
    assert!(!is_marker_line("// see STYLE-0013: for why"));
    assert!(!is_marker_line("// STYLE-0012: a different rule"));
    assert!(!is_marker_line("let s = \"STYLE-0013: not a comment\";"));
}

/// A method call spelling (`cursor.preceding_delimiter_ok(..)`) must be
/// caught too, not only the free-function form -- three of the five inline
/// copies #1803 folded were method calls.
#[test]
fn test_audit_catches_the_method_call_spelling() {
    let src = r"
        fn walks_elements<C: DocumentCursor>(c: &C, is_first: bool) -> bool {
            let expected = if is_first { None } else { Some(b',') };
            c.preceding_delimiter_ok(pos, expected)
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.violations.len(), 1);
    assert_eq!(audit.violations[0].callee, "preceding_delimiter_ok");
}
