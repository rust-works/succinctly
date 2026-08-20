# Deferred trackability checks for `resolve_node`'s path resolver (#986, #989)

**Status: design only, not implemented.** This document is the deliverable for issues
[#986](https://github.com/rust-works/succinctly/issues/986) and
[#989](https://github.com/rust-works/succinctly/issues/989), which a prior investigation
(see #986's own comment thread) diagnosed as sharing one root cause and recommended for
Tier 3 design treatment rather than a same-session implementation, after an earlier attempt
(PR #985, on #972) was reverted for underestimating this exact code region's invariants. No
code in this repo has been changed to implement this; see "Follow-up issues" at the bottom
for where implementation should be tracked once this is reviewed.

**Revision note.** An earlier draft of this document placed the deferred check inside
`resolve_seq`. That was wrong, and would have shipped a silent `del()` data-corruption
regression — see "Where the terminal check belongs" below, which supersedes it. The core
thesis (defer, don't invent new machinery) is unchanged.

## Problem

`resolve_seq`/`resolve_node`'s path resolver (`src/jq/eval.rs`) treats **every** element that
needs a fan-out pass as if it must independently resolve to something with its own path
shape — including an element that is only producing a *value* for a later pipe stage or
`Comma` sibling to consume, not a navigational step of its own.

For a `Pipe` with more than one element needing fan-out, every element up to and including
the last is resolved via `resolve_node`, which for a non-primitive expression (a bare
literal, a builtin call, arithmetic — anything that isn't
`Field`/`Index`/`Slice`/`Identity`/`Comma`/`Optional`/...) falls through to `resolve_leaf`'s
catch-all arm. That arm raises `#530`'s "Invalid path expression" **immediately**, before
`resolve_seq`'s loop ever reaches the *next* element — even when a later stage exists that
was only ever going to consume this element's *value*:

```bash
$ echo '{"a":10}' | jq -c 'path(1|halt_error(3))'
1                                                                 # stderr (halt_error echoes its own input)
                                                                  # exit 3 — halts
$ echo '{"a":10}' | succinctly jq -c 'path(1|halt_error(3))'
jq: error (at <stdin>:1): Invalid path expression with result 1    # stderr
                                                                  # exit 5 — wrong: catchable error instead of halting
```

The symptom class is: a `Halt`/`Break` that real jq lets through gets silently downgraded
into a catchable `Error` (violating this codebase's own documented invariant that `Halt` must
never be caught), and/or the error *wording* is wrong in a way that actively misleads a user
debugging the real failure (jq's "near attempt to access element K of V" names the actual
problem; succinctly's "Invalid path expression with result V" describes a symptom one step
removed from it).

### One call site, not two

An earlier draft claimed a second, independent call site in `resolve_index_expr`'s `target`
resolution, citing #989's own repro. **That attribution is wrong.** Dumping the AST for
#989's repro shows the `IndexExpr`'s target is `Identity`, not a `Pipe`:

```text
(1 | .[("x","y")]) = 9
  => Assign { path: Paren(Pipe([ Literal(1),
                                 IndexExpr { target: Identity, key: Paren(Comma(["x","y"])) } ])),
              value: Literal(9) }
```

So #989's repro fails in the *same* fan-out loop as #986's, on `Literal(1)`. What makes it
present differently is what happens *downstream* once the check is deferred, not a different
raise site — see "Why #989 needs no second context parameter" below.

The shape the earlier draft *described* (a `Pipe` in genuine target position) does exist and
is a real, previously uncited divergence — `(1 | .a)[("x","y")] = 9` — now tracked in the
behavior tables below as its own case.

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

```bash
$ echo '{"a":{"c":1},"x":2}' | jq -c 'path((.a, 1)|.c)'
["a","c"]                                                                                  # stdout
jq: error (at <stdin>:1): Invalid path expression near attempt to access element "c" of 1   # stderr
$ echo '{"a":{"c":1},"x":2}' | succinctly jq -c 'path((.a, 1)|.c)'
["a","c"]                                                                                  # stdout — already matches
jq: error (at <stdin>:1): Invalid path expression with result 1                             # stderr — only the wording diverges
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
check until the true terminal position is known) to the places that don't yet use it.
Concretely:

1. `PathBranch` (`src/jq/eval.rs:12416`, currently `(Vec<Expr>, Cow<'a, OwnedValue>)`) gains a
   third field: each branch's own `trackable: bool`, alongside its path and value. See
   "Blast radius" below — this is mechanical but **not** small.
2. `resolve_leaf`'s catch-all stops raising `#530` itself. It evaluates the expression for
   its value (propagating `Halt`/`Break`/`Error` exactly as it already threads through
   `eval_owned_multi`/`eval_owned_multi_keep_partial` today — no change to *that* part) and
   returns each output as `(Vec::new(), value, /* trackable */ false)`, deferring the
   decision of whether this is actually an error to whoever asked for it.
3. `Expr::Comma`'s arm stops calling `reject_if_untracked` on each sibling internally. Each
   sibling's own branches (now carrying their own `trackable` per Comma's existing
   independent-per-sibling resolution — see "Why Comma doesn't need new machinery" below)
   flow straight through to `Comma`'s own caller.
4. `resolve_seq` threads each branch's own carried `trackable` through its fan-out loop and
   into `apply_static_tail`/`resolve_static_tail`, instead of the single outer scalar it uses
   today. It performs **no** terminal-position check of its own — see below.

### Where the terminal check belongs

An earlier draft named `resolve_seq` as "the one remaining place that decides terminal
position — if `tail` is empty, this *is* the terminal position." **That is wrong in two
independent ways, both verified live.**

**`resolve_seq` never runs for a non-`Pipe` expression.** `path(1)`, `del(1)`,
`del(range(2))` and friends all error correctly today *precisely because* `resolve_leaf`'s
catch-all raises. Once it defers, the resulting `trackable: false` branch flows into
`resolve_dynamic_indexes`' `assemble()` (`src/jq/eval.rs:14467`), which maps a zero-component
branch to `Expr::Identity`. `path(1)` would silently become `[]`, and **`del(1)` would become
`del(.)` → `null`, destroying the whole document with no error** — the same write-path
corruption class that got PR #985 reverted.

**`resolve_seq` cannot know whether it is terminal.** It is also reached as an
`IndexExpr`/`SliceExpr` *target*, where its local "tail is empty" is true but the enclosing
key is still to be navigated. `path((1|.)[("x","y")])` is exactly this: jq blames `"x"`, but
a terminal check local to `resolve_seq` would produce "with result 1". This is the *same*
false-premise trap this document diagnoses in `reject_if_untracked`'s own doc comment,
reappearing inside the proposed fix.

The check therefore belongs only at sites that genuinely know nothing follows:

- **`resolve_dynamic_indexes` (`src/jq/eval.rs:14459`)** — the true top-level terminal,
  reached from all four entry points: `eval_assign` (`:11154`), `|=` (`:11269`),
  `builtin_path` (`:18041`), `builtin_del` (`:19660`). It performs no trackability check
  today, delegating entirely to the catch-all. **This is the check's new home**, applied
  per-branch using each branch's own carried flag.
- **`resolve_catch`** — already a genuinely terminal position by construction (nothing in
  `Expr::Try`'s own shape can have something after the catch handler within the same node).
  It keeps calling `reject_if_untracked`, which must itself become per-branch aware rather
  than consulting a single scalar and `branches.first()`.
- **Not `resolve_seq`, and not `Comma`.**

This placement is simpler than the earlier draft *and* strictly more faithful to the
document's own thesis: the decision moves all the way up to the caller that actually knows
there is nothing left to navigate into. It also fixes `path((1|.)[("x","y")])` for free,
which the earlier placement would have left broken.

### Why #989 needs no second context parameter

#989's own suggested fix direction proposes threading a second piece of context ("am I a
target or a leaf?") parallel to `trackable`. **The deferral makes that unnecessary.** Once
`resolve_leaf`'s catch-all returns `trackable: false` instead of raising, that flag reaches
the *next* pipe element, where `resolve_index_expr`'s **pre-existing** #843 checks already
produce jq's exact wording:

- `!trackable && is_passthrough_target(target)` (`src/jq/eval.rs:13934`) fires for
  `(1 | .[("x","y")]) = 9`, where the target is bare `Identity`, giving
  "near attempt to access element `"x"` of 1".
- the post-target `!trackable` check (`src/jq/eval.rs:13965`) fires for
  `path((1|.)[("x","y")])`, giving the same wording from the resolved target branch.
- `resolve_static_tail` (already correct) covers `path(1|.foo)` and
  `(1 | .a)[("x","y")] = 9`.

Both `resolve_index_expr` checks consult a single scalar `trackable` today and must become
per-branch aware; that is the only change either needs.

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

### Blast radius

`PathBranch` is a plain 2-tuple **type alias**, not a struct. Widening it forces a hard
compile error at every construction and destructure site, and each needs a *deliberately
chosen* trackable value, not merely something that compiles. Counted by pattern match on
current `main`, so treat these as lower bounds rather than an exhaustive audit:

| Kind of site                                      | Lower bound in `src/jq/eval.rs` |
|---------------------------------------------------|---------------------------------|
| `PathBranch`-shaped tuple construction            | >= 12                           |
| Tuple-destructure closures over branches          | >= 24                           |
| Lines mentioning `PathBranch`/`PathResolveResult` | 33                              |

Known constructors needing a deliberate value include the `Iterate` fan-out, `Select`/
type-filter passthrough, `resolve_against_cow`'s two `.map` closures, `push_recursive_branches`,
`resolve_index_expr`'s post-target push, `apply_static_tail`, and `resolve_catch`/
`resolve_recurse`'s stack-init lines. Stage 1 is small in *concept*; it is not a small diff.

### Non-goals (explicit, to prevent scope creep)

- **`resolve_recurse`'s interaction with a mixed-trackability `f`.** `resolve_recurse`
  currently runs under a `debug_assert!(trackable)` invariant and threads a single
  `trackable: true` through its own recursive `resolve_against_cow` calls. Once `Comma`
  siblings can independently be `false`, `f` producing a `(.a, 1)`-shaped set of children
  means `resolve_recurse`'s own recursion would, for the first time, need to decide what
  happens when it re-applies `f` to an *already-untracked* child. Not investigated here —
  flagged as an open risk below, and may turn out to need its own narrower follow-up rather
  than blocking this design.
- **`Builtin::GetPath`'s existing exemption** (a value from `getpath(...)` may resolve
  successfully even while untracked, per `resolve_node`'s own doc comment on that builtin) is
  unaffected — it already correctly returns a genuinely-successful, non-error branch, and the
  design above changes nothing about *that* code path.
- **#820** (`eval_comma`'s own value-position evaluation isn't lazy — a side-effecting
  builtin in a later `Comma` branch fires even when a short-circuiting consumer never needed
  it) and **#1013** (`resolve_seq`'s fan-out escape still truncates output produced by
  earlier elements when a later one escapes; already has active implementation attempts in
  flight — see #1013's own issue thread) are **related siblings from the same "eval.rs isn't
  a true lazy generator" architectural gap**, not the same mechanism as this document's own
  root cause, and are explicitly out of scope here. #820 lives in `eval_comma` (value
  evaluation), not `resolve_seq`/`resolve_node` (path resolution) — a different function
  family entirely, sharing only the general theme.
- **A full audit of every `resolve_node` arm's own trackable-output correctness.** This
  document establishes the mechanism and traces it through the arms directly implicated by
  #986/#989's own repros. It does **not** claim to have exhaustively verified every other arm
  (`Expr::As`, `Expr::Try`/`catch`, `Expr::Optional`, `Builtin::Select`/`If`, the `recurse`
  family beyond the one open risk above) computes the *correct* per-branch trackable value —
  see "Open risks" below, and note the construction-site counts in "Blast radius".

## Behavior tables (oracle-verified, current `main`)

Captured live against pinned jq 1.7.1 and a `--release --features cli` build of merge-base
`07b5c4b75`. All messages below are on **stderr** and elide the shared
`jq: error (at <stdin>:1): Invalid path expression ` prefix.

### Must not regress — these already agree with jq

These are the cases the earlier draft's placement would have silently broken. Every one of
them exercises a **non-`Pipe`** expression, so `resolve_seq` never runs at all.

| Query           | jq 1.7.1 and succinctly today (both exit 5) |
|-----------------|---------------------------------------------|
| `path(1)`       | `with result 1`                             |
| `path("x")`     | `with result "x"`                           |
| `path([1])`     | `with result [1]`                           |
| `path(1+1)`     | `with result 2`                             |
| `del(1)`        | `with result 1`                             |
| `del(range(2))` | `with result 0`                             |
| `(1) = 9`       | `with result 1`                             |

### Divergent today — the fix's targets

| Query                      | jq 1.7.1                                     | succinctly today                 | Mechanism once deferred                      |
|----------------------------|----------------------------------------------|----------------------------------|----------------------------------------------|
| `path(1\|halt_error(3))`   | halts, exit 3                                | `with result 1`                  | escape propagates during value evaluation    |
| `path(1\|2)`               | `with result 2`                              | `with result 1`                  | terminal check reads the *last* branch value |
| `path(1\|.foo)`            | `near attempt ... "foo" of 1`                | `with result 1`                  | `resolve_static_tail`, already correct       |
| `(1 \| .[("x","y")]) = 9`  | `near attempt ... "x" of 1`                  | `with result 1`                  | `is_passthrough_target` check (`:13934`)     |
| `(1 \| .a)[("x","y")] = 9` | `near attempt ... "a" of 1`                  | `with result 1`                  | `resolve_static_tail` inside the target      |
| `path((1\|.)[("x","y")])`  | `near attempt ... "x" of 1`                  | `with result 1`                  | post-target `!trackable` check (`:13965`)    |
| `path((.a, 1)\|.c)`        | `["a","c"]` then `near attempt ... "c" of 1` | `["a","c"]` then `with result 1` | per-branch static tail (Stage 2)             |

### Already agreeing — guard against new breakage

| Query                                              | jq 1.7.1 and succinctly today                   |
|----------------------------------------------------|-------------------------------------------------|
| `path((1, .a) \| recurse(.b))`                     | `with result 1`                                 |
| `path(try (.a, error({y:99})) catch (.b, 1))`      | `["a"]` then `near attempt ... "b" of {"y":99}` |
| `path(try (.a, error({y:99})) catch select(true))` | `["a"]` then `with result {"y":99}`             |

## Staged delivery

**Stage 1** — `PathBranch` gains `trackable: bool`; `resolve_leaf`'s catch-all defers instead
of raising; `resolve_seq` threads each branch's own flag through its loop and
`apply_static_tail`; `resolve_dynamic_indexes` gains the per-branch terminal check;
`reject_if_untracked` and `resolve_index_expr`'s two `!trackable` checks become per-branch
aware. *Why first:* fixes #986's core 4 repros (no `Comma` involved), both of #989's repros,
and the three previously uncited divergences in the table above. Note the construction-site
counts in "Blast radius" — this is conceptually one change but a wide diff.

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

1. **The non-`Pipe` regression set is the highest-risk part of Stage 1.** Every row of the
   "Must not regress" table above passes today and would break under a terminal check placed
   anywhere below `resolve_dynamic_indexes`. `del(1)` silently returning `null` is the worst
   case and would not be caught by any sweep built only from pipe shapes. Pin all seven rows
   as regression tests **before** touching `resolve_leaf`.
2. **Re-verify the "Finding #2 already fixed" claim** at implementation time, specifically:
   3+ `Comma` siblings with mixed trackability, and a `Comma` nested inside another
   `Comma`/pipe stage rather than at the top level of one. Not checked here.
3. **`resolve_recurse` + mixed-trackability `f`.** The `debug_assert!(trackable)` at the top
   of `resolve_recurse` **cannot** fire from Stage 1 or 2 as scoped: the shared recurse-family
   untracked guard (`src/jq/eval.rs:12713`) intercepts an untracked value before any
   recurse-specific arm runs, returning `#530`'s classic wording — which already matches jq
   for `path((1, .a) | recurse(.b))` (see the table above). The genuine Stage-3 risk is
   therefore *silent wrong output* (recurse re-applying `f` to an already-untracked child as
   if trackable), **not** a panic. Construct a live repro against real jq 1.7.1 before
   assuming any particular behavior.
4. **Every other `resolve_node` arm's own per-branch trackable computation** (`Expr::As`,
   `Expr::Try`, `Expr::Optional`'s several sub-cases, `Builtin::Select`/`If`/`GetPath`,
   `resolve_slice_expr`'s own bound handling) needs its own pass to confirm each arm sets the
   *correct* trackable value on its output branches, not just that the type compiles.
   `Expr::Optional`'s arm in particular has several already-subtle, individually-commented
   carve-outs (the `bare_navigation_primitive` check, the `is_untracked_navigation_error`
   distinction) that a mechanical "just propagate whatever came in" migration could silently
   break — read that arm's own doc comment in full before touching it.
5. **`del`/assignment write-path semantics**, not just `path()`'s read-only output. #972's own
   revert (on a related but different fix in this same code region) was specifically about
   silent data corruption on the write path once multi-key index/slice fan-outs were involved.
   This design's scope is about *when an error is raised and how it's worded*, not about which
   branches get written — but Open Risk 1 above shows that boundary is easier to cross than it
   looks. Run the full differential sweep against `del`, `=` and `|=`, not just `path()`.
6. **Halt/Break propagation through the now-deferred catch-all.** Confirm `resolve_leaf`'s
   deferred version still correctly propagates `Halt`/`Break` as escapes (not as ordinary
   values) when the *expression itself* halts/breaks while being evaluated for its value —
   distinct from the case this document is about, where the expression evaluates fine and
   it's the *downstream* consumption that used to wrongly raise `#530`.

## Verification approach for the follow-up implementation PR(s)

This is a correctness fix, not a performance one — no benchmark gate, but an equivalently
rigorous *oracle differential* gate:

- Pin all three behavior tables above as permanent regression tests, comparing **output, exit
  code, and stream** (`Halt`'s exit code is part of the contract, not just its message).
- Build a systematic sweep over **both** shape families. The earlier draft's sweep generated
  only `E1 | E2` and `(E1, E2) | E3` — every case has a pipe, so a bare `del(1)` never
  appeared and the Open Risk 1 regression would have shipped. Required additions:
  - **Bare non-pipe shapes**: `path(L)` / `del(L)` / `L = X` for `L` drawn from a literal,
    arithmetic, `range(n)`, a string, and an array.
  - **Target-position shapes**: `(E)[K]` and `(E)[S:T]` where `E` is itself a pipe ending in a
    non-path-shaped value.
  - The original pipe families, with `E1` (or a `Comma` sibling) drawn from a bare literal, a
    builtin call, `select(...)`, `getpath(...)`, `try...catch`, and a genuine navigation
    primitive — crossed with `E2`/`E3` drawn from nothing (terminal), a navigation primitive,
    another `Comma`, `error(...)`, `halt_error(...)`, and `break $label`.
- Compare `path(...)`, `del(...)`, `... = X` and `... |= f` against real jq for **every**
  combination, not `path()` alone (per Open Risk 5).
- Gate each stage on the existing `path()`/`del()`/assignment golden fixtures and CLI test
  suite showing zero regressions, in addition to the new oracle sweep.

## Critical files

- `src/jq/eval.rs`:
  - `PathBranch` (`type` alias, line ~12416) and `PathResolveResult` (~12433) — the shared
    return shape; see "Blast radius" for how many sites widening it touches.
  - `resolve_leaf` (~13242) — the catch-all this document's core fix targets.
  - `reject_if_untracked` (~13211) — stays, but its `Comma`-sibling call site is removed
    (Stage 2) and it must become per-branch aware rather than testing a scalar and
    `branches.first()`.
  - `resolve_node`'s `Expr::Comma` arm (~12500) — Stage 2's target.
  - **`resolve_dynamic_indexes` (~14459) and its `assemble()` (~14467)** — Stage 1's new
    terminal-check home; callers at `eval_assign` (~11154), `|=` (~11269), `builtin_path`
    (~18041), `builtin_del` (~19660).
  - `resolve_seq` (~14330), `apply_static_tail` (~14294), `resolve_static_tail` (~14267),
    `value_after_components` (~14219) — the already-correct template this design generalizes;
    `resolve_seq`'s loop and `apply_static_tail` switch from the outer scalar to each branch's
    own carried flag.
  - `resolve_index_expr` (~13908) — its two `!trackable` checks (~13934 pre-target, ~13965
    post-target) already produce jq's exact wording and need only to become per-branch aware.
  - `resolve_recurse` (~13715) and the shared recurse-family untracked guard (~12713) — Open
    Risk 3's target, not modified unless Stage 3 proves necessary.
  - `resolve_against_cow` (~13422) — **no signature change needed.** It has exactly three call
    sites with fixed values (`resolve_catch` ~13503: always `false`; `resolve_recurse` ~13769:
    always `true`; `resolve_seq`'s loop ~14394: the outer scalar). The real work is at
    `resolve_seq`'s call site and `apply_static_tail`'s loop.
  - `needs_fanout_pass`/`needs_path_prepass` (~12163/12119) — unaffected by this design (they
    decide *whether* an element needs the fan-out loop at all, not what happens once it's
    there), but worth re-reading their own extensive doc comments before touching anything
    nearby, since they document several already-subtle, previously-regression-tested
    invariants (an early, rejected fix attempt for #682 caused an O(n²) regression there).

## Related

- [#986](https://github.com/rust-works/succinctly/issues/986) and
  [#989](https://github.com/rust-works/succinctly/issues/989) — the two issues this document
  resolves the design question for.
- [#972](https://github.com/rust-works/succinctly/issues/972) — a related-but-distinct fix in
  the same code region, whose own PR (#985) was reverted for silent data corruption on the
  write path when multi-key index/slice fan-outs were involved; the caution in Open Risks 1
  and 5 is directly informed by that incident.
- [#682](https://github.com/rust-works/succinctly/issues/682) — a data-loss bug
  (`path(recurse(f))` dropping elements when a trailing iterate reached a 2+-element
  container). Cited here only because an early, rejected fix attempt for it introduced an
  O(n²) regression in `needs_path_prepass`, which is why that predicate's doc comment is
  worth reading before editing anything near it.
- [#820](https://github.com/rust-works/succinctly/issues/820) — a sibling laziness gap in
  `eval_comma` (value evaluation), explicitly out of scope here (see Non-goals).
- [#1013](https://github.com/rust-works/succinctly/issues/1013) — `resolve_seq`'s fan-out
  escape truncating earlier elements' output; explicitly out of scope here, and has active
  implementation attempts already in flight on its own issue thread.
- [`docs/plan/jq-lazy-map-select.md`](jq-lazy-map-select.md) — the closest prior art in this
  repo for a "design doc, staged delivery, explicit non-goals and open risks" deliverable for
  a similar `eval.rs`/`eval_generic.rs` laziness gap; this document follows its structure.

## Follow-up issues

Not yet filed. Once this document is reviewed, file one implementation issue per stage above
(mirroring #700 → #724/#725's own slice-per-issue pattern), linking back to this document and
to #986/#989 for the original repros.
