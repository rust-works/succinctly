# Deferred trackability checks for `resolve_node`'s path resolver (#986, #989)

**Status: design only, not implemented.** This document is the deliverable for issues
[#986](https://github.com/rust-works/succinctly/issues/986) and
[#989](https://github.com/rust-works/succinctly/issues/989), which a prior investigation
(see #986's own comment thread) diagnosed as sharing one root cause and recommended for
Tier 3 design treatment rather than a same-session implementation, after an earlier attempt
(PR #985, on #972) was reverted for underestimating this exact code region's invariants. No
code in this repo has been changed to implement this; see "Follow-up issues" at the bottom
for where implementation should be tracked once this is reviewed.

## Problem

`resolve_seq`/`resolve_node`'s path resolver (`src/jq/eval.rs`) treats **every** element that
needs a fan-out pass as if it must independently resolve to something with its own path
shape — including an element that is only producing a *value* for a later pipe stage or
`Comma` sibling to consume, not a navigational step of its own. Two independent call sites
hit this:

**`resolve_seq`'s fan-out loop** — for a `Pipe` with more than one element needing fan-out,
every element up to and including the last one is resolved via `resolve_node`, which for a
non-primitive expression (a bare literal, a builtin call, arithmetic — anything that isn't
`Field`/`Index`/`Slice`/`Identity`/`Comma`/`Optional`/... ) falls through to `resolve_leaf`'s
catch-all arm. That arm raises `#530`'s "Invalid path expression" **immediately**, before
`resolve_seq`'s loop ever reaches the *next* element — even when a later stage exists that
was only ever going to consume this element's *value*:

```
$ echo '{"a":10}' | jq -c 'path(1|halt_error(3))'
1
(exit 3 — halts)
$ echo '{"a":10}' | succinctly jq -c 'path(1|halt_error(3))'
jq: error (at <stdin>:1): Invalid path expression with result 1
(exit 5 — wrong: raises an ordinary, catchable error instead of halting)
```

**`resolve_index_expr`'s `target` resolution** — `.[K]`'s own `target` is resolved via the
same `resolve_node::<S>(target, value, trackable)` call, hitting the identical catch-all when
`target` is a `Pipe` whose own last element isn't itself path-shaped:

```
$ echo '{"a":1}' | jq -c '(1 | .[("x","y")]) = 9'
jq: error (at <stdin>:1): Invalid path expression near attempt to access element "x" of 1
$ echo '{"a":1}' | succinctly jq -c '(1 | .[("x","y")]) = 9'
jq: error (at <stdin>:1): Invalid path expression with result 1
```

Both repros are oracle-verified against jq 1.7.1. The symptom class is: a `Halt`/`Break`
that real jq lets through gets silently downgraded into a catchable `Error` (violating this
codebase's own documented invariant that `Halt` must never be caught), and/or the error
*wording* is wrong in a way that actively misleads a user debugging the real failure (jq's
"near attempt to access element K of V" names the actual problem; succinctly's "Invalid path
expression with result V" describes a symptom one step removed from it).

## Root cause (verified by reading, not guessed)

`resolve_static_tail` (`src/jq/eval.rs:14267`) **already implements the correct pattern** for
a closely related case — the static tail applied after `resolve_seq`'s fan-out loop finishes.
It does not raise immediately on an untracked value; it *defers*, and only decides whether to
raise (and which of the two error wordings to use) once it knows whether `components` (what
comes next) is empty or not:

```rust
fn resolve_static_tail<'a, S: EvalSemantics>(
    components: &[Expr],
    value: &OwnedValue,
    trackable: bool,
) -> Result<OwnedValue, (Vec<PathBranch<'a>>, EvalEscape)> {
    if !trackable {
        let error = match components.first().and_then(navigation_element) {
            Some(element) => EvalError::invalid_path_expression_near_access(&element, value),
            None => EvalError::invalid_path_expression(value),
        };
        return Err((Vec::new(), error.into()));
    }
    value_after_components::<S>(components, value).map_err(|e| (Vec::new(), e))
}
```

`resolve_leaf`'s catch-all (`src/jq/eval.rs:13242`, reached by `resolve_node` for any
expression none of its other arms recognize) does **not** follow this pattern — it raises
`#530` unconditionally the moment it decides the expression isn't one of the four bare
primitives, with no way for its caller to say "there's more coming, just give me the value."
`Expr::Comma`'s own arm in `resolve_node` (`src/jq/eval.rs:12500`) has the same shape: it
calls `reject_if_untracked` on each sibling's result **immediately**, inside the `Comma` arm
itself, rather than deferring to whatever comes after the `Comma` in the enclosing pipe.

`reject_if_untracked`'s own doc comment states its two call sites (`Comma`'s own sibling
output, and `resolve_catch`'s top-level result) are "exactly the two places in the whole
resolver where a value becomes final without anything else in the same expression left to
navigate into it." **That premise is false whenever `Comma` appears as a non-terminal element
of a pipe** — `(.a, 1) | .c` is exactly this shape, and `.c` is precisely "something else in
the same expression left to navigate into it." The premise was apparently true when
`reject_if_untracked` was introduced (#843) and has not been re-checked since a `Comma`
mid-pipe became reachable through the same fan-out machinery.

### A significant simplification found while re-verifying the prior investigation

#986's own comment thread found a second, independent finding ("Finding #2"): that
`resolve_seq`'s fan-out loop applied a `Comma` sibling's static tail (`.c` in the example
above) only when every sibling succeeded, silently dropping it for branches that had already
succeeded when a *different* sibling failed. **Live-testing against the current `main`
confirms this is already fixed**, apparently as a side effect of the later `#977` fix (visible
in `resolve_seq`'s own code comment, "apply the static tail to whatever branches survived up
to this escape"):

```
$ echo '{"a":{"c":1},"x":2}' | jq -c 'path((.a, 1)|.c)'
["a","c"]
jq: error (at <stdin>:1): Invalid path expression near attempt to access element "c" of 1
$ echo '{"a":{"c":1},"x":2}' | succinctly jq -c 'path((.a, 1)|.c)'
["a","c"]
jq: error (at <stdin>:1): Invalid path expression with result 1
```

The path output (`["a","c"]`) already matches jq exactly — the *only* remaining divergence is
the error wording, which is the same single root cause described above, not a second,
independent architectural gap. **This means the design below only has to solve one problem,
not two** — a materially smaller and lower-risk scope than the prior investigation's own
"per-branch trackability + streamed per-branch tail application" framing suggested. Before
relying on this finding for scoping, an implementer should re-confirm it still holds at the
point of implementation (main will have moved on), and should specifically check whether
`#977`'s fix generalizes to *more* than two siblings and to a `Comma` nested one level deeper
(`((.a, 1), .b) | .c`) — not verified here, flagged as an explicit open risk below.

## Scope decision

**Defer, don't invent new machinery.** The fix is not a new laziness mechanism — it is
extending the pattern `resolve_static_tail` already proves correct (defer the trackable
check until the true terminal position is known) to the two places that don't yet use it.
Concretely:

1. `PathBranch` (`src/jq/eval.rs:12416`, currently `(Vec<Expr>, Cow<'a, OwnedValue>)`) gains a
   third field: each branch's own `trackable: bool`, alongside its path and value. This is
   mechanical — `PathBranch` is already threaded per-branch through every call site in the
   file, so adding a field to the type (not inventing a new parallel structure) means the
   compiler enumerates every construction site that needs updating.
2. `resolve_leaf`'s catch-all stops raising `#530` itself. It evaluates the expression for
   its value (propagating `Halt`/`Break`/`Error` exactly as it already threads through
   `eval_owned_multi`/`eval_owned_multi_keep_partial` today — no change to *that* part) and
   returns each output as `(Vec::new(), value, /* trackable */ false)`, deferring the
   decision of whether this is actually an error to whoever asked for it.
3. `Expr::Comma`'s arm stops calling `reject_if_untracked` on each sibling internally. Each
   sibling's own branches (now carrying their own `trackable` per Comma's existing
   independent-per-sibling resolution — see "Why Comma doesn't need new machinery" below)
   flow straight through to `Comma`'s own caller.
4. The **one remaining place** that decides "is this actually the terminal position" is
   `resolve_seq`: after its fan-out loop finishes (`i == last_dynamic`), if `tail` is empty,
   this *is* the terminal position — apply `reject_if_untracked`-equivalent logic per branch,
   using each branch's own carried `trackable` flag, not a single input parameter. If `tail`
   is non-empty, `resolve_static_tail` (unchanged in spirit, just fed each branch's own flag
   instead of a single `trackable` input) already does the right thing.
5. `resolve_catch`'s own top-level `reject_if_untracked` call (the non-`Comma`, bare
   `catch expr` case) is unaffected — it already *is* a genuinely terminal position by
   construction (nothing in `Expr::Try`'s own shape can have something after the catch
   handler within the same node), so it keeps calling `reject_if_untracked` exactly as today.

### Why `Comma` doesn't need new per-sibling machinery, only deferral

`Comma`'s arm already resolves each sibling **independently** via its own recursive
`resolve_node::<S>(e, value, trackable)` call (`src/jq/eval.rs:12509`) — it is not currently
missing the ability to tell siblings apart, it is missing the ability to *defer* what it does
with that information. Once `resolve_leaf`'s catch-all returns `trackable: false` instead of
raising, and `Comma` stops calling `reject_if_untracked` immediately, `(.a, 1)`'s two siblings
naturally come back as `[(["a"], <the .a value>, true), ([], 1, false)]` — already
distinguished per-branch, for free, from the existing per-sibling loop. This is the concrete
reason Finding #1 (per-branch trackability) turned out not to need the "thread a richer
per-branch signal through dozens of construction sites" scope the prior investigation
feared — `Comma`'s own resolution was already doing the right per-sibling work; it just
needed to stop discarding that information via an immediate check.

### Non-goals (explicit, to prevent scope creep)

- **`resolve_recurse`'s interaction with a mixed-trackability `f`.** `resolve_recurse`
  currently runs under a `debug_assert!(trackable)` invariant (only ever called with
  `trackable: true`, enforced by an earlier guard in `resolve_node`) and threads a single
  `trackable: true` through its own recursive `resolve_against_cow` calls. Once `Comma`
  siblings can independently be `false`, `f` producing a `(.a, 1)`-shaped set of children
  means `resolve_recurse`'s own recursion would, for the first time, need to decide what
  happens when it re-applies `f` to an *already-untracked* child. Not investigated here —
  flagged as an open risk below, and may turn out to need its own narrower follow-up rather
  than blocking this design.
- **`Builtin::GetPath`'s existing exemption** (a value from `getpath(...)` may resolve
  successfully even while untracked, per `resolve_node`'s own doc comment on that builtin) is
  unaffected — it already correctly returns a genuinely-successful, non-error branch, and the
  design above changes nothing about *that* code path, only the catch-all/`Comma` cases that
  currently raise instead of deferring.
- **#820** (`eval_comma`'s own value-position evaluation isn't lazy — a side-effecting
  builtin in a later `Comma` branch fires even when a short-circuiting consumer never needed
  it) and **#1013** (`resolve_seq`'s own multi-dynamic-element fan-out doesn't reproduce jq's
  exact stream order, "known simplification" per its own code comment; already has active
  implementation attempts in flight — see #1013's own issue thread) are **related siblings
  from the same "eval.rs isn't a true lazy generator" architectural gap**, not the same
  mechanism as this document's own root cause, and are explicitly out of scope here. #820
  lives in `eval_comma` (value evaluation), not `resolve_seq`/`resolve_node` (path
  resolution) — a different function family entirely, sharing only the general theme.
- **A full audit of every `resolve_node` arm's own trackable-output correctness.** This
  document establishes the mechanism and traces it through the arms directly implicated by
  #986/#989's own repros (`resolve_leaf`'s catch-all, `Comma`, `resolve_seq`'s loop,
  `resolve_index_expr`'s `target`). It does **not** claim to have exhaustively verified every
  other arm (`Expr::As`, `Expr::Try`/`catch`, `Expr::Optional`, `Builtin::Select`/`If`, the
  `recurse` family beyond the one open risk above) computes the *correct* per-branch
  trackable value for its own output — see "Open risks" below.

## Staged delivery

**Stage 1** — `PathBranch` gains `trackable: bool`; `resolve_leaf`'s catch-all defers instead
of raising; `resolve_seq`'s loop performs the terminal-position check (only when `tail` is
empty and `i == last_dynamic`) using each branch's own flag instead of the outer parameter.
*Why first:* fixes #986's core 4 repros (no `Comma` involved) and both of #989's repros — the
smallest diff that fixes the reported bugs, touching the fewest arms.

**Stage 2** — `Expr::Comma`'s arm stops calling `reject_if_untracked` internally, letting
per-sibling `trackable` flow through (already computed correctly per-sibling once Stage 1
lands — see above). *Why second:* fixes the `(.a, 1) | .c` wording gap. Genuinely small once
Stage 1 exists — mostly *removing* a now-premature check, not adding new logic.

**Stage 3** *(only if Stage 1/2's own test sweep finds it's actually needed)* — audit and, if
necessary, fix `resolve_recurse`'s handling of a mixed-trackability `f` result. *Why
deferred:* pending Stage 1/2's own oracle sweep actually exercising `recurse` with a
`Comma`-shaped `f` — may turn out to already be fine, or may need its own narrower fix once
the actual failure mode (if any) is characterized against real jq.

Each stage should land as its own PR with its own full oracle-verified test sweep before the
next begins, mirroring how `docs/plan/jq-lazy-map-select.md`'s own staged delivery was
measured slice-by-slice rather than as one combined change.

## Open risks for an implementer to sanity-check

1. **Re-verify the "Finding #2 already fixed" claim** (see above) at implementation time,
   specifically: 3+ Comma siblings with mixed trackability, and a `Comma` nested inside
   another `Comma`/pipe stage rather than at the top level of one. Not checked here.
2. **`resolve_recurse` + mixed-trackability `f`** (see non-goals above) — construct a live
   repro (`path([1,2] | recurse((.a, 1)))`-shaped) against real jq 1.7.1 before assuming any
   particular behavior; the `debug_assert!(trackable)` at the top of `resolve_recurse` will
   itself need re-examining once a per-branch flag exists upstream of it.
3. **Every other `resolve_node` arm's own per-branch trackable computation** (`Expr::As`,
   `Expr::Try`, `Expr::Optional`'s several sub-cases, `Builtin::Select`/`If`/`GetPath`,
   `resolve_index_expr`/`resolve_slice_expr`'s own key/target handling beyond what's traced
   above) needs its own pass to confirm each arm sets the *correct* trackable value on its
   output branches, not just that the type compiles. `Expr::Optional`'s own arm in particular
   has several already-subtle, individually-commented carve-outs (the `bare_navigation_primitive`
   check, the `is_untracked_navigation_error` distinction) that a mechanical "just propagate
   whatever came in" migration could silently break — read that arm's own doc comment in full
   before touching it.
4. **`del`/assignment write-path semantics**, not just `path()`'s read-only output. #972's own
   revert (on a related but different fix in this same code region) was specifically about
   silent data corruption on the write path (`del`, `=`) once multi-key index/slice fan-outs
   were involved — this design's own scope is about *when an error is raised and how it's
   worded*, not about which branches get written, so it should be lower-risk than #972's own
   attempt by construction. Still: run the full oracle-verified differential sweep (this
   session's own established discipline — see "Verification approach" below) against `del`
   and `=`/`|=`, not just `path()`, before merging any stage.
5. **Halt/Break propagation through the now-deferred catch-all.** Confirm `resolve_leaf`'s
   deferred version still correctly propagates `Halt`/`Break` as escapes (not as ordinary
   values) when the *expression itself* halts/breaks while being evaluated for its value —
   distinct from the case this document is about, where the expression evaluates fine and
   it's the *downstream* consumption that used to wrongly raise `#530`. `1|halt_error(3)`'s
   own repro should exercise this correctly once implemented (the `1` succeeds trivially;
   `halt_error(3)` is the *next* fan-out element, and Stage 1 alone should let its own
   existing `Halt` propagation reach the top uninterrupted) — but confirm no other repro
   conflates "value evaluation itself escaped" with "value evaluation succeeded but isn't
   path-shaped."

## Verification approach for the follow-up implementation PR(s)

This is a correctness fix, not a performance one — no benchmark gate, but an equivalently
rigorous *oracle differential* gate, matching this session's own established practice for
number-formatting fixes earlier in this backlog pass:

- Confirm every repro in #986 and #989's own issue bodies matches jq 1.7.1 exactly (output
  *and* exit code — `Halt`'s exit code is part of the contract, not just its message).
- Build a randomized/systematic sweep of `E1 | E2` and `(E1, E2) | E3` shapes where `E1`
  (or a `Comma` sibling) is drawn from: a bare literal, a builtin call, `select(...)`,
  `getpath(...)`, `try...catch`, and a genuine navigation primitive — crossed with `E2`/`E3`
  drawn from: nothing (terminal), a navigation primitive, another `Comma`, `error(...)`,
  `halt_error(...)`, `break $label`. Compare `path(...)`, `del(...)`, and `... = X` against
  real jq for each combination, not just `path()` alone (per Open Risk 4).
- Gate each stage on the *existing* `path()`/`del()`/assignment golden fixtures and CLI test
  suite showing zero regressions, in addition to the new oracle sweep above.
- Re-run this document's own two confirmed-live repros (the `halt_error` exit-code case, and
  the `(.a, 1) | .c` wording case) as permanent regression tests, not just ad hoc probes.

## Critical files

- `src/jq/eval.rs`:
  - `PathBranch` (`type` alias, line ~12416) and `PathResolveResult` (~12433) — the shared
    return shape every arm below constructs.
  - `resolve_leaf` (~13242) — the catch-all this document's core fix targets.
  - `reject_if_untracked` (~13211) — stays, but its `Comma`-sibling call site is removed
    (Stage 2); its `resolve_catch` call site is unaffected.
  - `resolve_node`'s `Expr::Comma` arm (~12500) — Stage 2's target.
  - `resolve_seq` (~14330), `apply_static_tail` (~14294), `resolve_static_tail` (~14267),
    `value_after_components` (~14219) — Stage 1's terminal-position check target; the
    already-correct template this design generalizes.
  - `resolve_against_cow` (~13422) — the thin `Cow`-dispatch wrapper every fan-out element
    goes through; needs to pass each branch's own `trackable` instead of a single parameter.
  - `resolve_index_expr` (~13908), specifically its `target` resolution — #989's own call
    site, fixed for free once `resolve_leaf`'s catch-all defers (Stage 1).
  - `resolve_recurse` (~13715) — Open Risk 2's target, not modified unless Stage 3 proves
    necessary.
  - `needs_fanout_pass`/`needs_path_prepass` (~12163/12119) — unaffected by this design (they
    decide *whether* an element needs the fan-out loop at all, not what happens once it's
    there), but worth re-reading their own extensive doc comments before touching anything
    nearby, since they document several already-subtle, previously-regression-tested
    invariants (#682's O(n²) regression from an earlier over-broad version).

## Related

- [#986](https://github.com/rust-works/succinctly/issues/986) and
  [#989](https://github.com/rust-works/succinctly/issues/989) — the two issues this document
  resolves the design question for.
- [#972](https://github.com/rust-works/succinctly/issues/972) — a related-but-distinct fix in
  the same code region, whose own PR (#985) was reverted for silent data corruption on the
  write path when multi-key index/slice fan-outs were involved; the caution in Open Risk 4
  and the staged-delivery table above are directly informed by that incident.
- [#820](https://github.com/rust-works/succinctly/issues/820) — a sibling laziness gap in
  `eval_comma` (value evaluation), explicitly out of scope here (see Non-goals).
- [#1013](https://github.com/rust-works/succinctly/issues/1013) — `resolve_seq`'s own
  multi-dynamic-element stream-order simplification; explicitly out of scope here, and has
  active implementation attempts already in flight on its own issue thread.
- [`docs/plan/jq-lazy-map-select.md`](jq-lazy-map-select.md) — the closest prior art in this
  repo for a "design doc, staged delivery, explicit non-goals and open risks" deliverable for
  a similar `eval.rs`/`eval_generic.rs` laziness gap; this document follows its structure.

## Follow-up issues

Not yet filed. Once this document is reviewed, file one implementation issue per stage above
(mirroring #700 → #724/#725's own slice-per-issue pattern), linking back to this document and
to #986/#989 for the original repros.
