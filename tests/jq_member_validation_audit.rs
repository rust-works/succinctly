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
//! # The second rule: the post-loop tail (#2404)
//!
//! This file now carries two rules. The member rule above fires on a *call*
//! -- a primitive reached for directly. The tail rule
//! (`style_0013_tail_gap_is_routed_or_exempted`) fires on an *absence*: a
//! walk that validated every member and then never checked its tail.
//!
//! A second rule was needed because the first cannot express that. #2211
//! (`{,}`/`[,]` never checked) and #2243 (`{"a":1,}` accepted) were walks
//! with no offending call anywhere in them -- nothing for a callee-keyed
//! scan to catch -- and #2349 was the wrong answer that drift produced.
//!
//! The obvious formulation, auditing `container_gap_ok`/
//! `trailing_element_gap_ok` by name, was rejected while writing #1803 and
//! is still the wrong shape: those have many legitimate direct callers
//! outside any walk (`json::light`'s streaming writers, the unchecked
//! `len`/`keys`/`collect_cursors` fast paths, the tail helpers themselves),
//! so it would be mostly noise and would train readers to add the marker
//! reflexively.
//!
//! Keying on *the walk* avoids that. The gate is "does this function loop
//! over a document's children (`WALK_METHODS`) **and** validate them
//! (`MEMBER_VALIDATION`)?", and only such a function is asked to account for
//! its tail. Everything named above falls outside it without an exemption,
//! because none of them validates members -- the rule asks nothing of a walk
//! that never decided to check its children in the first place.
//!
//! The tail marker is `// STYLE-0013-TAIL:`, deliberately not the member
//! one. See [`TAIL_EXEMPT_MARKER`] for why sharing it would have made the
//! rule vacuous on arrival.
//!
//! # What neither rule does
//!
//! Neither decides whether a site *should* route. They only demand that
//! somebody decided and wrote the reason down.

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

/// The helpers `checked_key`/`delimiters_ok` own, plus the primitives
/// themselves: calling any of these is what makes a function a *member-
/// validating* walk, and so what puts it in scope for the tail rule below.
///
/// Deliberately wider than [`AUDITED`], which lists only the primitives a
/// call site must not reach for directly. A walk that routes properly
/// through `checked_key`/`delimiters_ok` is invisible to the member rule --
/// and it is exactly as obliged to check its tail as one that does not.
const MEMBER_VALIDATION: &[&str] = &[
    "checked_key",
    "delimiters_ok",
    "resolve_display_key",
    "key_delimiter_ok",
    "value_delimiter_ok",
    "preceding_delimiter_ok",
    "preceding_gap_ok",
];

/// The tail helpers a member-validating walk may end in.
///
/// `last_field_trailing_gap_ok` is here for the key-only walks (`census`,
/// `checked_len`) that never hold a value cursor to hand a general helper.
const TAIL_ROUTES: &[&str] = &[
    "child_tail_gap_ok",
    "container_tail_gap_ok",
    "tail_gap_ok",
    "last_field_trailing_gap_ok",
];

/// The low-level tail primitives the helpers in [`TAIL_ROUTES`] own.
///
/// Listed only to keep a definition out of its own audit -- a call to one is
/// not itself a violation. What the tail rule checks is the *absence* of a
/// [`TAIL_ROUTES`] call, which catches hand-rolling one of these and
/// omitting the tail entirely as the same finding, because they are the same
/// bug: #2211 and #2243 were omissions, #2409's six copies were hand-rolls,
/// and both shipped the same wrong answer.
const TAIL_PRIMITIVES: &[&str] = &[
    "trailing_element_gap_ok",
    "container_gap_ok",
    "trailing_gap_ok",
    "empty_container_gap_ok",
    "element_gap_ok",
    "element_gap_ok_at",
];

/// Method names whose call in a loop head means "this loop unconses a
/// document's children one at a time" -- the walk shape the tail rule keys
/// on.
///
/// `uncons`/`uncons_cursor`/`uncons_key` are the three cursor-domain
/// spellings; `children`/`cursor_iter` are the two iterator-domain ones.
///
/// A `for x in <collection bound earlier>` loop is **out of scope**, and this
/// is the rule's known boundary rather than a claim that such loops are
/// safe. An earlier draft of this comment justified the carve-out by saying
/// those members were validated wherever the collection was built; review
/// showed that is false for at least one site -- `eval_builtin`'s
/// `to_entries` arm validates members inside the loop and hand-rolls its own
/// tail, and is invisible here because its iterator is a local binding, not
/// a call this scan can see in the loop head.
///
/// Covering it needs the binding traced back to its initializer, which is
/// real dataflow rather than the syntactic match everything else here does.
/// Left out deliberately: the shape this rule was written for is the
/// cursor-domain walk, where all six of its current findings live. The gap
/// is recorded in #2594 rather than papered over.
const WALK_METHODS: &[&str] = &[
    "uncons",
    "uncons_cursor",
    "uncons_key",
    "children",
    "cursor_iter",
];

/// The STYLE-0013 exemption marker, cited inline the way STYLE-0004's
/// `#[allow]` citations and STYLE-0012's routing exemptions already are.
const EXEMPT_MARKER: &str = "STYLE-0013:";

/// The tail rule's own marker, deliberately *not* [`EXEMPT_MARKER`].
///
/// Sharing one marker would have made this rule vacuous on the day it
/// landed: five of the six walks it currently finds already carry a
/// `// STYLE-0013:` exemption for their member half, so a shared marker
/// would have let the member decision silently stand in for a tail decision
/// nobody made. That substitution is precisely the drift being audited --
/// #2349's fix hand-copied *both* halves, and the member half is the one
/// that got the attention.
const TAIL_EXEMPT_MARKER: &str = "STYLE-0013-TAIL:";

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
    /// How many loops in this function head on one of [`WALK_METHODS`].
    ///
    /// A count, not a flag: a function with an object arm and an array arm
    /// has two walks and owes two tail checks. A single boolean let either
    /// arm's tail check exempt the other, so deleting `to_owned_at_depth`'s
    /// array-arm `tail_gap_ok` -- reintroducing #2211's `[,]` exactly --
    /// left the audit green. Found in review of this rule; pinned by
    /// `test_tail_audit_counts_each_arm_separately_2404`.
    walk_loops: usize,
    /// Set when this function validates members at all (see
    /// [`MEMBER_VALIDATION`]) -- routed or hand-rolled, both count.
    validates_members: bool,
    /// How many [`TAIL_ROUTES`] calls this function makes, compared against
    /// `walk_loops` rather than merely being non-zero.
    tail_routes: usize,
    /// Line of the first walk loop, for the violation report -- the loop is
    /// the thing missing a tail, so it is the useful place to point.
    walk_line: usize,
    /// Line ranges of `fn` items nested inside this one, excluded from the
    /// marker scan so a marker belonging to a nested helper cannot exempt
    /// its enclosing walk.
    nested: Vec<(usize, usize)>,
}

struct Audit<'a> {
    file: &'static str,
    lines: Vec<&'a str>,
    /// Innermost enclosing function last, so a nested `fn` does not inherit
    /// its parent's marker.
    stack: Vec<Frame>,
    sites_examined: usize,
    violations: Vec<Site>,
    /// Member-validating walks seen, whether or not they routed -- the tail
    /// rule's own anti-vacuity counter.
    walks_examined: usize,
    tail_violations: Vec<Site>,
}

/// Whether a `// STYLE-0013:` marker appears anywhere inside this function's
/// own body.
///
/// Function-level, not "the comment block attached to the call": a walk that
/// cannot route is exempt *as a walk*, and its reason is one paragraph, not
/// one per primitive it touches. `push_generic_document_validation_error` is
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
/// `push_generic_document_validation_error`'s own comment -- "This marker is
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

/// Whether one line *is* a STYLE-0013-TAIL citation.
///
/// Same open-the-comment rule as [`is_marker_line`], and load-bearing for
/// the same reason -- with the extra wrinkle that `// STYLE-0013:` is a
/// prefix of nothing here: the two markers are checked independently, so a
/// member exemption never reads as a tail one.
fn is_tail_marker_line(line: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix("//") else {
        return false;
    };
    rest.trim_start().starts_with(TAIL_EXEMPT_MARKER)
}

/// Whether a `// STYLE-0013-TAIL:` marker appears inside this function's own
/// body. Clipped to the frame exactly as [`marker_in_body`] is.
fn tail_marker_in_body(lines: &[&str], frame: &Frame) -> bool {
    let lo = frame.body_start.saturating_sub(1);
    let hi = frame.body_end.min(lines.len());
    (lo..hi).any(|i| {
        let line_no = i + 1;
        // A marker inside a nested `fn` belongs to that helper, not to the
        // walk that happens to enclose it. The member rule's own
        // `test_nested_fn_does_not_inherit_the_parent_marker` pins the
        // other direction; this is the same principle read upwards.
        let in_nested = frame
            .nested
            .iter()
            .any(|(s, e)| line_no >= *s && line_no <= *e);
        !in_nested && is_tail_marker_line(lines[i])
    })
}

/// Whether an expression subtree contains a call to one of
/// [`WALK_METHODS`], used on a loop's head to decide "is this a walk?".
///
/// Scans the whole head rather than matching one shape, because the same
/// walk is spelled several ways across the audited files --
/// `while let Some((f, rest)) = fields.uncons()`,
/// `while let Some((c, next)) = elems.uncons_cursor()`,
/// `for child in cursor.children()`. A subtree scan covers all of them
/// without enumerating each.
fn contains_walk_call(expr: &syn::Expr) -> bool {
    struct Finder(bool);
    impl<'ast> Visit<'ast> for Finder {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let Some(name) = callee_name(node) {
                if WALK_METHODS.contains(&name.as_str()) {
                    self.0 = true;
                }
            }
            visit::visit_expr(self, node);
        }
    }
    let mut f = Finder(false);
    f.visit_expr(expr);
    f.0
}

impl<'a> Audit<'a> {
    fn new(file: &'static str, src: &'a str) -> Self {
        Self {
            file,
            lines: src.lines().collect(),
            stack: Vec::new(),
            sites_examined: 0,
            violations: Vec::new(),
            walks_examined: 0,
            tail_violations: Vec::new(),
        }
    }

    fn enter(&mut self, name: String, body: Span) {
        self.stack.push(Frame {
            name,
            body_start: body.start().line,
            body_end: body.end().line,
            walk_loops: 0,
            validates_members: false,
            tail_routes: 0,
            walk_line: 0,
            nested: Vec::new(),
        });
    }

    /// Evaluate the tail rule as the frame is popped.
    ///
    /// Deferred to the pop rather than checked at a call, because the
    /// question is about the function as a whole: the tail call necessarily
    /// comes *after* the loop, so at no single call site is the answer
    /// known yet.
    fn leave(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        if let Some(parent) = self.stack.last_mut() {
            parent.nested.push((frame.body_start, frame.body_end));
        }
        // A definition of one of the primitives or helpers is not a walk
        // that owes a tail check -- it is the thing the walk routes through.
        if MEMBER_VALIDATION.contains(&frame.name.as_str())
            || TAIL_ROUTES.contains(&frame.name.as_str())
            || TAIL_PRIMITIVES.contains(&frame.name.as_str())
        {
            return;
        }
        if !(frame.walk_loops > 0 && frame.validates_members) {
            return;
        }
        self.walks_examined += 1;
        if frame.tail_routes >= frame.walk_loops || tail_marker_in_body(&self.lines, &frame) {
            return;
        }
        self.tail_violations.push(Site {
            file: self.file,
            line: frame.walk_line,
            func: frame.name.clone(),
            callee: "<no tail helper after this walk>".to_string(),
        });
    }

    /// Record what this expression tells us about the innermost frame.
    fn note_expr(&mut self, expr: &syn::Expr) {
        let is_walk_head = match expr {
            syn::Expr::While(w) => contains_walk_call(&w.cond),
            syn::Expr::ForLoop(f) => contains_walk_call(&f.expr),
            _ => false,
        };
        let callee = callee_name(expr);
        let Some(frame) = self.stack.last_mut() else {
            return;
        };
        if is_walk_head {
            if frame.walk_loops == 0 {
                frame.walk_line = line_of(expr.span());
            }
            frame.walk_loops += 1;
        }
        if let Some(name) = callee {
            if MEMBER_VALIDATION.contains(&name.as_str()) {
                frame.validates_members = true;
            }
            if TAIL_ROUTES.contains(&name.as_str()) {
                frame.tail_routes += 1;
            }
        }
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
        self.leave();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.enter(node.sig.ident.to_string(), node.block.span());
        visit::visit_impl_item_fn(self, node);
        self.leave();
    }

    fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
        if let Some(block) = &node.default {
            self.enter(node.sig.ident.to_string(), block.span());
            visit::visit_trait_item_fn(self, node);
            self.leave();
        } else {
            visit::visit_trait_item_fn(self, node);
        }
    }

    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        self.check_call(node);
        self.note_expr(node);
        visit::visit_expr(self, node);
    }
}

/// Everything one pass over `SOURCES` produces, for both rules.
struct AuditReport {
    violations: Vec<Site>,
    examined: usize,
    files: usize,
    tail_violations: Vec<Site>,
    walks_examined: usize,
}

fn run_audit_full() -> AuditReport {
    let mut r = AuditReport {
        violations: Vec::new(),
        examined: 0,
        files: 0,
        tail_violations: Vec::new(),
        walks_examined: 0,
    };
    for (path, src) in SOURCES {
        let parsed = syn::parse_file(src)
            .unwrap_or_else(|e| panic!("STYLE-0013 audit could not parse {path}: {e}"));
        let mut audit = Audit::new(path, src);
        audit.visit_file(&parsed);
        r.examined += audit.sites_examined;
        r.violations.extend(audit.violations);
        r.walks_examined += audit.walks_examined;
        r.tail_violations.extend(audit.tail_violations);
        r.files += 1;
    }
    r
}

fn run_audit() -> (Vec<Site>, usize, usize) {
    let r = run_audit_full();
    (r.violations, r.examined, r.files)
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
/// `push_generic_document_validation_error` and watching the audit stay green,
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

/// Lower bound proving the tail scan is not vacuous.
///
/// Today it sees 14 member-validating walks across `SOURCES` (counted by
/// instrumenting `run_audit_full`, not by hand). Same rule as
/// `MIN_SITES_EXAMINED`: if a real refactor lowers the count past this, move
/// the floor *and* say in the commit message what shrank.
const MIN_WALKS_EXAMINED: usize = 10;

/// STYLE-0013's second half (#2404): a walk that validates its members must
/// also validate its tail.
///
/// The member rule above cannot express this. It fires on a *call*, and the
/// bug here is an *absence* -- #2211 (`{,}`/`[,]` never checked) and #2243
/// (`{"a":1,}` accepted) were both walks that validated every member and
/// then simply stopped, with no call anywhere for a callee-keyed rule to
/// catch. #2349 was the wrong answer that drift finally produced.
///
/// Keyed on the walk, which is what makes it low-noise where a name-based
/// rule on `container_gap_ok`/`trailing_element_gap_ok` would not be: the
/// streaming writers in `json::light`, the unchecked `len`/`keys`/
/// `collect_cursors` fast paths, and the YAML-only walks all fall outside
/// it without needing an exemption each, because none of them validates
/// members. Only a walk that already decided to check its children is asked
/// to account for its tail.
#[test]
fn style_0013_tail_gap_is_routed_or_exempted() {
    let r = run_audit_full();

    assert!(
        r.walks_examined >= MIN_WALKS_EXAMINED,
        "STYLE-0013 tail audit examined only {} member-validating walks, expected at least \
         {MIN_WALKS_EXAMINED}. A vacuous scan passes green: check that `WALK_METHODS` and \
         `MEMBER_VALIDATION` still match the code before lowering this floor.",
        r.walks_examined
    );

    if !r.tail_violations.is_empty() {
        let mut report = String::new();
        for v in &r.tail_violations {
            report.push_str(&format!("\n  {}:{} in `{}`", v.file, v.line, v.func));
        }
        panic!(
            "STYLE-0013 tail violation: {} walk(s) validate their members but never reach a \
             tail helper, and carry no `// STYLE-0013-TAIL:` exemption:{}\n\n\
             A walk that checks every member and skips the tail accepts `{{,}}`, `[,]` and \
             `{{\"a\":1,}}` -- see #2211/#2243/#2349. Fix by either (a) ending the walk in \
             `tail_gap_ok` (holds a container cursor or not), `container_tail_gap_ok` (holds \
             one; also catches the zero-child `{{,}}` case) or `child_tail_gap_ok` (children \
             only), or (b) writing `// STYLE-0013-TAIL: <why this walk needs no tail check, \
             or cannot use the helper>` inside the function. Note this marker is separate \
             from `// STYLE-0013:` on purpose -- a member exemption is not a tail decision. \
             See docs/STYLE_GUIDE.md.",
            r.tail_violations.len(),
            report
        );
    }
}

/// The tail rule fires on a member-validating walk that never reaches a
/// tail helper -- the #2211/#2243 shape, where every member is checked and
/// the loop simply ends.
#[test]
fn test_tail_audit_fires_on_a_walk_with_no_tail_check_2404() {
    let src = r"
        fn walks_an_object<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                field.delimiters_ok(&cursor)?;
                f = rest;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.walks_examined, 1, "the walk must be examined");
    assert_eq!(audit.tail_violations.len(), 1);
    assert_eq!(audit.tail_violations[0].func, "walks_an_object");
}

/// Ending the same walk in a tail helper clears it, with no marker needed.
/// This is the path the rule wants people to take, so it must actually work.
#[test]
fn test_tail_audit_accepts_a_walk_routed_through_a_helper_2404() {
    let src = r"
        fn walks_an_object<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            let mut f = fields.clone();
            let mut last = None;
            while let Some((field, rest)) = f.uncons() {
                field.delimiters_ok(&cursor)?;
                last = Some(field.value_cursor);
                f = rest;
            }
            container_tail_gap_ok(&c, last.as_ref(), b'}')?;
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.walks_examined, 1);
    assert!(
        audit.tail_violations.is_empty(),
        "a walk routed through a tail helper must pass"
    );
}

/// The tail marker is the documented escape hatch, so it has to work.
#[test]
fn test_tail_audit_accepts_a_tail_marked_walk_2404() {
    let src = r"
        fn walks_an_object<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            // STYLE-0013-TAIL: key-only walk, no cursor to hand a helper.
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                field.delimiters_ok(&cursor)?;
                f = rest;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert!(audit.tail_violations.is_empty());
}

/// **The reason the two markers are separate.**
///
/// A `// STYLE-0013:` exemption answers "why this walk reaches for the
/// primitive directly". It says nothing about the tail, and must not be
/// allowed to stand in for a tail decision -- five of the six walks this
/// rule currently finds already carry one, so sharing a marker would have
/// made the rule vacuous on the day it landed.
#[test]
fn test_member_marker_does_not_exempt_the_tail_2404() {
    let src = r"
        fn walks_an_object<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            // STYLE-0013: key-only walk, no value resolved to check a `:` against.
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                if !key_delimiter_ok::<F>(&field.key, &cursor, is_first) {
                    return Err(f.malformed_member_error());
                }
                f = rest;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert!(
        audit.violations.is_empty(),
        "the member half is exempted by its own marker"
    );
    assert_eq!(
        audit.tail_violations.len(),
        1,
        "but the tail half is still unanswered"
    );
}

/// A walk that does not validate members is out of scope entirely -- no
/// marker required. This is what keeps the rule low-noise where a rule keyed
/// on `container_gap_ok`/`trailing_element_gap_ok` by name would not be: the
/// unchecked `len`/`keys`/`collect_cursors` fast paths and the YAML-only
/// walks all land here.
#[test]
fn test_tail_audit_ignores_a_walk_that_validates_nothing_2404() {
    let src = r"
        fn counts<F: DocumentFields>(fields: &F) -> usize {
            let mut f = fields.clone();
            let mut n = 0;
            while let Some((_field, rest)) = f.uncons() {
                n += 1;
                f = rest;
            }
            n
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.walks_examined, 0);
    assert!(audit.tail_violations.is_empty());
}

/// A loop over an already-materialized collection is not a walk: its members
/// were validated wherever that collection was built, and its tail with
/// them. Pinned so the loop test stays keyed on the uncons/children shape
/// rather than on "contains a loop".
#[test]
fn test_tail_audit_ignores_a_loop_over_a_materialized_vec_2404() {
    let src = r"
        fn over_a_vec(items: Vec<Field>) -> Result<(), EvalError> {
            for field in items {
                field.delimiters_ok(&cursor)?;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(
        audit.walks_examined, 0,
        "a `for x in vec` loop is not a child walk"
    );
    assert!(audit.tail_violations.is_empty());
}

/// A tail marker in one function must not excuse the next, exactly as the
/// member marker must not -- same clipping, pinned separately because it is
/// a separate marker with its own scan.
#[test]
fn test_tail_marker_does_not_leak_across_function_boundaries_2404() {
    let src = r"
        fn exempt_walk<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            // STYLE-0013-TAIL: nothing to check here.
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                field.delimiters_ok(&cursor)?;
                f = rest;
            }
            Ok(())
        }

        fn unmarked_walk<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                field.delimiters_ok(&cursor)?;
                f = rest;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.walks_examined, 2);
    assert_eq!(audit.tail_violations.len(), 1);
    assert_eq!(audit.tail_violations[0].func, "unmarked_walk");
}

/// A function with an object arm and an array arm has **two** walks and owes
/// **two** tail checks.
///
/// Found in review: with `routes_tail` as one boolean, either arm's tail
/// check exempted the other. Deleting `to_owned_at_depth`'s array-arm
/// `tail_gap_ok` -- reintroducing #2211's `[,]` bug exactly -- left the audit
/// green. Verified by mutation that the counting form catches it.
#[test]
fn test_tail_audit_counts_each_arm_separately_2404() {
    let src = r"
        fn two_arms<F: DocumentFields>(v: &V) -> Result<(), EvalError> {
            match v {
                Object(fields) => {
                    let mut f = fields.clone();
                    let mut last = None;
                    while let Some((field, rest)) = f.uncons() {
                        field.delimiters_ok(&cursor)?;
                        last = Some(field.value_cursor);
                        f = rest;
                    }
                    tail_gap_ok(cursor, last.as_ref(), b'}')?;
                }
                Array(elems) => {
                    let mut e = elems.clone();
                    let mut last = None;
                    while let Some((elem, rest)) = e.uncons_cursor() {
                        elem.delimiters_ok(&cursor)?;
                        last = Some(elem);
                        e = rest;
                    }
                }
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(audit.walks_examined, 1, "one function, examined once");
    assert_eq!(
        audit.tail_violations.len(),
        1,
        "two walks and one tail check must not pass -- the array arm is unguarded"
    );
}

/// A marker inside a nested `fn` belongs to that helper, not to the walk
/// enclosing it.
///
/// `test_nested_fn_does_not_inherit_the_parent_marker` pins the downward
/// direction for the member rule; this is the same principle read upwards,
/// and it was a real hole -- the frame's line range spans its nested items,
/// so an unrelated nested marker silently exempted the outer walk.
#[test]
fn test_tail_marker_in_a_nested_fn_does_not_exempt_the_outer_walk_2404() {
    let src = r"
        fn outer_walk<F: DocumentFields>(fields: &F) -> Result<(), EvalError> {
            fn inner_helper() -> bool {
                // STYLE-0013-TAIL: this helper has no walk of its own.
                true
            }
            let mut f = fields.clone();
            while let Some((field, rest)) = f.uncons() {
                field.delimiters_ok(&cursor)?;
                f = rest;
            }
            Ok(())
        }
    ";
    let parsed = syn::parse_file(src).expect("fixture parses");
    let mut audit = Audit::new("fixture.rs", src);
    audit.visit_file(&parsed);
    assert_eq!(
        audit.tail_violations.len(),
        1,
        "the nested helper's marker must not exempt `outer_walk`"
    );
    assert_eq!(audit.tail_violations[0].func, "outer_walk");
}
