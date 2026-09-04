# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The "materialization ignores a live `optional`" gap is now a build failure, not a
  sweep** (#2334). The same bug shipped three times — #2231, #2280, #2327 — each round's
  review finding sites the last sweep missed, several of them the direct sibling of a
  function that same PR had just fixed in the *other* evaluator.

  The reason it recurred is that **the invariant is unobservable at runtime**.
  `eval::suppresses` is `optional && !e.is_decode_failure()`, and every error the
  `to_owned`/`to_owned_cursor`/`to_owned_with_cursor`/`collect_cursors_checked` family can
  raise is `decode_failure`-tagged — #2286 retagged the last one that wasn't, via
  `EvalError::malformed_json_text` — so routing a call site and leaving it bare produce
  byte-identical output. No behavioural test can fail when a site is missed; #2327's own
  pinning test asserts the same exit code with *and* without a trailing `?`, which records
  the non-difference and by construction cannot detect the invariant regressing.
  Centralising the *check* did not stop the *omission*, because nothing forced a call site
  through it.

  Two guards replace the sweep, following #2025's precedent
  (`tests/jq_owned_only_sink_invariant_test.rs`) for a hand-derived claim that only review
  was keeping true:

  - **Static** — `tests/jq_optional_suppression_audit.rs` parses `src/jq/eval.rs` and
    `src/jq/eval_generic.rs` with `syn` and fails, naming `file:line` and the enclosing
    function, on any raw materialization inside a function with a live `optional: bool`
    that is neither routed through the suppression machinery nor carrying a
    `// STYLE-0012:` citation. It sees sites hidden inside `owned_or_err!` /
    `push_or_control!` token streams — the shape of #2327's worst miss, where 7 of the 8
    second-round sites were one evaluator's copy of a fix the other had received — and it
    asserts its own scan is not vacuous, the classic failure of a grep-shaped gate. Its
    routing window is clipped to the enclosing function body and to the next
    materialization site; both clips were added after negative tests showed the unclipped
    window silently excusing real gaps with a `suppresses(..)` belonging to neighbouring
    code.
  - **Runtime** — `EvalError::debug_assert_materialization_error`, called at the family's
    four depth-0 entry points, pins the "decode failures only" premise the whole audit
    rests on. The day that stops holding, the routed/unrouted distinction starts changing
    output at every site at once; this makes that a loud debug-build failure instead of a
    silent one. It fired nowhere across the full suite.

  Every site the audit found is adjudicated — routed where the builtin's own result value
  is being materialized (`add`, `map_values`, `last`, `to_entries`, `from_entries`,
  `with_entries`, `shuffle`, `pivot`, `bsearch`, the `indices`/`index`/`rindex` family,
  string interpolation, `each_paths_filter`, `collect_rhs_outputs`, and
  `eval_generic.rs`'s `Expr::Iterate`/`Expr::Pipe`/`Shuffle`/`Pivot`/`ToEntries` arms), and
  marked with a specific reason otherwise: array construction is atomic in jq
  (`eval_array_construction`, `map_over`, `Expr::Array` — #2327's own declined candidate);
  key generators are evaluated at a hardcoded `optional: false`, so `.[k]?` never
  suppresses an error raised computing `k` (captured from jq 1.7.1:
  `.[error("boom")]?` exits 5, while `.[.k]?` on a non-string `.k` exits 0 — the latter is
  the *index step* being suppressed, and an earlier draft of this comment cited it
  backwards); a `Partial`'s
  prefix must survive, so its `Control::Error` is suppressed at the `eval_try` boundary
  rather than at the conversion; and several sites are already routed one level out
  (`finish_fork`, `fanout_arg`'s deferred unwrap, `sort_family_control`).

  Along the way: four hand-copies of `optional && !e.is_decode_failure()` now call the
  shared `suppresses` — the same drift its own doc comment already records happening twice
  (#1902, #1934). `sort_family_control` deliberately does *not* join them: review found it
  is not the materialization's private error channel but a shared one, also carrying
  whatever the user's key filter raised (`sort_by(error("x"))`), so routing it would have
  been a guaranteed no-op for the decode failure it was written for and a live
  over-suppression for everything else. It keeps its original signature and
  `key_elements_generic` carries a marker saying why. **No behaviour change**: every path
  involved is decode-failure-tagged, and a
  `debug_assert!(!optional)` probe over the whole suite re-confirmed #693's claim that
  `eval_generic.rs`'s native arms never see `optional = true` from any parser-driven
  query — only three white-box unit tests that hardcode it, one of them already named
  `..._is_unreachable_via_parser_725`.

  The convention is written down as **STYLE-0012** in
  [docs/STYLE_GUIDE.md](docs/STYLE_GUIDE.md), modelled on STYLE-0004's "every lint
  suppression must be traceable to a documented reason". When the whole parameter is dead,
  the preferred marker remains `_optional` — the unused-variable lint enforces that one,
  which is how `builtin_recurse_f`/`builtin_recurse_cond` record their #1953 exemption.

- **`path()`/`=`/`del()` no longer re-clone an owned subtree at every step of a static
  `Field`/`Index` chain** (#2058), fixing the O(d²) that #1690's own depth-scaled acceptance
  benchmark (`jq_write_path_del_shared_prefix_depth`) flagged as still binding its exponent to
  ~1.6. Holding branch count fixed at two and scaling only depth `D`, `path()`/`=`/`del()` all
  read an exponent of ~1.8-1.95 — two independent per-step clone sites, found and fixed
  together:

  - `src/jq/eval.rs`'s `value_after_components` (`resolve_seq`'s shared static-tail
    resolver, reached by all three builtins once a top-level `Comma` routes through
    `resolve_node`) cloned the *entire remaining value* at every navigation step
    (`eval_owned_fast_path`'s `Field`/`Index` arms already skip the JSON reindex round trip
    for these two shapes, but still `.cloned()` the matched child's whole subtree — O(d−i) at
    step `i`, summing to O(d²)). It now takes its value by move and navigates *destructively*
    via a new `navigate_static_component`: `IndexMap::swap_remove`/`Vec::swap_remove` pull the
    one matched child out by move instead of cloning it, since the parent container is never
    read again once this function has stepped past it — the caller pays exactly one `.clone()`
    at the boundary (unavoidable; it never owns the document it's handed), everything after
    that is a move. `Expr::Slice`/`Expr::Optional`-wrapped components are unaffected — neither
    was on the fast path to begin with.
  - `path()` alone kept a growing exponent even with the above fixed, traced to a *second*
    site present independently in **both** of this crate's evaluators: `src/jq/eval.rs`'s
    `walk_path`/`walk_pipe`/`step_into` and `src/jq/eval_generic.rs`'s `path_walk_generic`/
    `path_step_generic` (the #2061 cursor-native fast path the CLI's own `path()` dispatch
    actually uses for this shape) both rebuilt the *reported* path (`["c","c",...,0]`) with a
    `path.to_vec()`-then-push per step — another O(d²). `eval_generic.rs`'s copy compounded
    this with a second, independent clone: its `Expr::Pipe` handling split one component off
    at a time and rewrapped the remainder in a fresh, owned `Expr::Pipe(rest.to_vec())` just to
    recurse, the identical AST-clone-per-stage shape #1510 already fixed in `eval.rs`'s own
    path-context evaluator, but not (until now) in `eval_generic.rs`'s independent copy. Fixed
    by a new `PathTrail` (`eval.rs`, shared by both evaluators) — `PathPrefix`'s twin for
    `path()`'s own value-typed components, an `Rc`-linked cons-list with O(1) `extend` and one
    O(depth) `to_vec` paid once per branch — plus two new slice-based helpers
    (`path_walk_pipe_generic`, `path_step_pipe_generic`) that recurse on a borrowed `&[Expr]`
    instead of rebuilding an owned `Expr::Pipe`.

  Measured via interleaved CLI A/B (`succinctly jq`, 200-document stream per invocation, 5
  reps alternating before/after order, both `nodes.yaml` targets idle): every exponent over
  the last depth-doubling (D=120→240) drops from ~1.88-1.98 to ~0.95-1.02 on both an Apple M4
  Pro and an AMD Ryzen 9 7950X, for `path()`, `=` and `del()` alike. Speedups at D=240 range
  5.8x (`del()`, x86) to 19.9x (`path()`, x86). No behavior change: 2320 systematic
  before/after CLI cases (varying depth, container type, missing keys, out-of-range indices,
  slices, `?`-optional siblings, both jq and yq mode) plus the pinned-oracle differential
  suite (`/usr/bin/jq` 1.7.1, Homebrew `yq` v4.53.3) are byte-for-byte identical to the
  pre-fix binary. New regression fixture:
  `jq_write_path_two_branch_{path,assign,del}_depth` in `benches/jq_write_path_bench.rs`
  (fixed two-branch comma, depth-only scaling, unlike the existing
  `jq_write_path_del_shared_prefix_depth` group which scales branch count and depth together
  and so cannot isolate this term). See
  [docs/optimizations/path-resolution-clone.md](docs/optimizations/path-resolution-clone.md)
  for the full write-up.

- **`Expr::Index` and `Expr::Slice` now carry their own float-spelling keys** (#1401),
  replacing the separate `Expr::IndexNumber`/`Expr::SliceNumber` variants that #1088 and
  #1326 added beside them. `Expr::Index(i64)` is now
  `Expr::Index { idx: i64, key: Option<NumberKey> }`, and `Expr::Slice` gains
  `start_key`/`end_key`; both are breaking changes for callers that construct or match
  these variants directly. The `Expr::index()` and `Expr::slice()` constructors are
  unchanged.

  Preserving a float literal's own spelling in `path()` output (`path(.[2.0])` is `[2.0]`)
  behaves exactly as before — only the representation moved. The paired form required all
  63 sites that ask "is this an index/a slice" to spell out both members of each pair, and
  a site that forgot one got an `unreachable!()` instead of an answer, which is what
  `delete_expr_array_paths` did from #1088 until #1326 closed it. One variant per family
  makes that shape unrepresentable rather than merely tested for, and removes both
  `Expr`-scrutinee `unreachable!()` fallbacks along with it. `size_of::<Expr>()` is
  unchanged at 96 bytes and is now pinned by a test.

- **`eval.rs`'s three `Expr`-substitution passes now share one generic tree
  walk** (#2095): `install_def_calls`, `substitute_func_param` and
  `substitute_var_impl` each used to hand-write a full ~40-arm match over
  every `Expr` variant, identical except at a handful of binder/leaf/opaque
  arms, so adding a new `Expr` variant meant three synchronized edits — and
  missing one next to a wildcard would compile cleanly while silently
  dropping that variant from whichever pass forgot it. `map_subexprs`
  (`src/jq/walk.rs`) generalizes `map_builtin_subexprs` one level up the
  tree: it owns every variant whose recursive-structural-child handling is
  identical across all three callers, and each caller now matches only its
  own special-cased arms (shadow checks, opaque `Shared`/`DefCall`
  handling, `FuncDef`, `Builtin`) before falling through to it. Both
  `map_subexprs` and `map_builtin_subexprs` deliberately have no wildcard
  arm, so a future `Expr`/`Builtin` variant is a compile error in one place
  rather than a silent gap in three.

  Behavior-preserving refactor, not a fix: verified by reading all three
  functions' arms side by side before folding any of them, and every
  existing `jq`/`yq` test still passes unchanged. `Expr::Shared` and
  `Expr::Error` turned out to be handled identically across all three
  callers too, but are kept as explicit per-function arms rather than
  folded — `Shared`'s opacity is architecturally load-bearing (#1371,
  #2077, #2096) and each caller's own comment documents a different
  concrete hazard, and `Expr::Error`'s non-recursion into its message looks
  like a pre-existing latent gap (`error($x)` referencing a variable a pass
  is substituting doesn't get substituted) rather than a deliberate
  invariant, flagged in `map_subexprs`'s own doc comment for follow-up
  rather than silently changed here.

- **`BoundBody::get_or_try_init` now uses `OnceCell::set` instead of
  `get_or_init` to populate its cache** (#2092, finding #6). In this
  single-threaded, non-reentrant-by-design cache, the cell cannot
  legitimately already hold a value by the time `bind` finishes — `set`
  makes that invariant self-checking (a panic if it's ever violated)
  instead of `get_or_init`'s silent "discard my freshly-bound value, return
  whatever's already there." No behavior change on any currently-reachable
  path.

  #2092's other finding in this area — a duplicated `Shared`/`DefCall`
  passthrough block between `substitute_func_param` and
  `substitute_var_impl` (finding #5) — turned out to already be resolved as
  a side effect of #2095's `map_subexprs` refactor above: both functions'
  `DefCall` arms now fall through to `map_subexprs`'s single shared default
  arm rather than each carrying their own copy, closing the exact
  `bound`-reset drift risk finding #5 described. Only the one-line
  `Expr::Shared` arm remains duplicated between them, which #2095's own doc
  comment explains is deliberately kept explicit in each caller rather than
  folded. #2092's four remaining findings (##1-4) are real algorithmic
  inefficiencies on the `def` recursive-call hot path, but each needs
  interleaved hardware A/B benchmark evidence before landing per this
  project's benchmarking discipline — split out to #2194 rather than
  bundled into this refactor-only change.

- **`eval.rs`'s infallible `to_owned` is renamed to `to_owned_lossy`, and the
  fallible `to_owned_checked` takes over the short name `to_owned`** (#1989),
  making the checked conversion the file's compiler-enforced default: a new
  bare `to_owned` call site now gets the `Result`-returning, decode-failure-
  raising conversion, and reaching for the silently-lossy one requires
  spelling out `to_owned_lossy` — a deliberate, visible act that has to be
  justified against that function's own three-loss soundness rule (an
  undecodable string becomes `""`, an undecodable object key is dropped
  entirely, and a trailing unpaired member (#1194) is silently ignored).
  `to_owned_for_error_message` was considered and rejected as the lossy
  variant's new name — roughly a third of its remaining call sites aren't
  error-message contexts at all, and a name that lies at a third of its
  sites is worse than one that just names the hazard.

  A full audit of every one of the file's ~75 production `to_owned` call
  sites (84 including 9 test-module sites) preceded the rename: 62 were
  confirmed genuinely safe (fed only already-validated values — inside
  `scalar_fallback`, behind an explicit `scalar_decode_failure` guard, or
  fed exclusively by an already-owned/reindexed document), and 13 across 6
  real clusters were live bugs, each fixed here with its own regression
  test (temporarily reverted and confirmed to reproduce the pre-fix
  symptom before being re-applied):

  - `QueryResult::collect_owned`'s unchecked fold, reached via
    `fanout_two_args`'s eager two-argument path and
    `fanout_regex_pattern_with_collected_flags` — the highest-severity
    cluster, since it silently substituted a wrong *argument* value (e.g.
    `splits(","; .flags)` on an undecodable `.flags` silently applied no
    flags) rather than just misreporting an error. Both routes now go
    through the file's existing `stream_outputs_checked`, the same twin
    #2023 already used to fix the parallel single-argument `fanout_arg`
    path.
  - `builtin_any`/`builtin_all`/`any_all_f`'s jq-mode fallback arm, which
    (unlike the yq-mode arms beside it) had no `scalar_decode_failure`
    guard, so `any`/`all(cond)` on an undecodable string input reported
    `Cannot iterate over string ("")` and was `?`-suppressible, instead of
    raising.
  - `builtin_last_stream`'s three borrowed-output arms, the direct sibling
    of `builtin_skip`'s own decode-failure conversion (also part of this
    issue) that its own doc comment says should be "kept in sync" but
    wasn't.
  - `Item::into_owned` at `builtin_limit`/`builtin_first_stream`'s two
    call sites (of 7 total; the other 5 audited safe) — both emit the
    materialized value as the query's own output, so the file's own
    already-documented gap was live wrong data there, matching #1972's
    precedent that a parser-unreachable-but-library-reachable site is
    still worth fixing.
  - `push_owned_values` at unary minus's operand conversion, which made
    `-.a` on an undecodable `.a` report `string ("") cannot be negated` —
    a catchable, `?`-suppressible message naming content the document
    never held — instead of the (non-suppressible) decode failure.
  - `builtin_implode`'s per-element conversion, mirroring #1755's own
    per-element fix in the sort family: an undecodable array element was
    reported as `"" can't be imploded, ...` and suppressed by `?`.

  Five smaller, still-benign-today shapes surfaced during the same audit
  are split out to #2196 rather than bundled here: `Item::into_owned`'s
  own naming (not renamed by this issue), `builtin_implode`'s top-level
  (non-element) `_ if optional` arm, `builtin_combinations_n`'s remaining
  unchecked `stream_outputs` use, the now-backwards-reading `_checked`
  suffix on `stream_outputs_checked`/`push_owned_values_checked`/
  `promote_borrowed_checked`, and `builtin_nth_stream`'s pre-existing
  (predating both this issue and #1972) gap where `Item::into_owned_checked`'s
  non-decode-failure errors aren't run through `suppress_or_raise` the way
  its two `each_take_first`/`each_take_n` siblings now are.

  A dedicated adversarial code review (8 independent finder passes) then
  caught two further issues in this PR's own fixes before they landed:

  - `fanout_two_args`'s probe branch (entered when the outer slot's own
    `RejectMany` violation has to be reported, but inner has to be
    evaluated once first to check whether *it* takes priority) discarded a
    decode failure it had just captured. `stream_outputs_checked` on a
    single undecodable inner value returns an *empty* vec plus the failure
    in its trailing control — and `apply_arg_fanout(RejectMany, ...)`'s own
    guard is `len() > 1`, so it reports `Ok(())` on that empty vec, and the
    code only checked the trailing control inside the `Err` arm of that
    call. Fixed by adding the same unconditional trailing-control check the
    function's main loop already makes right after its own (structurally
    identical) `apply_arg_fanout` call.
  - `builtin_limit`'s `into_owned_checked` conversion (this issue's own
    fix) used `.collect::<Result<Vec<_>, _>>()`, whose all-or-nothing
    `FromIterator` impl discards every already-converted prefix item the
    moment a later one fails — silently losing legitimate output
    (`limit(2; "good", <bad>)` reported a bare decode failure instead of a
    `Partial` prefix carrying `"good"`), violating #400/#494's own "outputs
    already produced don't vanish" contract that this same function's
    `Flow::Escaped` arm, two lines below, was already honoring correctly.
    Fixed by converting items one at a time into an explicit `Vec`.

- **`resolve_node`/`resolve_seq` no longer serialize-and-reparse a branch's
  whole subtree to evaluate a pure `select`/comparison condition** (#2048).
  A filtered recursive descent (`del(.. | select(type == "number"))`)
  evaluates its filter once per resolved branch, and branch *i* of a
  depth-*d* chain holds an O(*d* - *i*)-sized subtree — routing that
  evaluation through `eval_owned_multi_keep_partial` → `eval_owned_input`'s
  `to_json_for_reindex` + `JsonIndex::build` bridge (built for filters that
  need real navigation) summed those serializations to O(*d*²), independent
  of and larger than the per-branch path-flatten term #1690/#2047 already
  fixed in this exact code region. A new pre-check in
  `eval_owned_fast_path` (`eval_owned_pure`, gated by `is_owned_pure_expr` +
  the representation-preserving `produces_fresh_value`, #2048) answers a
  closed grammar of pure, always-single-output shapes — comparisons,
  `and`/`or`, `type`, literals, and navigation used only as an operand —
  directly against the already-owned branch, with no text round-trip.

  Measured on the issue's own D=240 repro (interleaved A/B, real hardware,
  both `nodes.yaml` targets): 2.06–2.69x faster, landing on the floor a
  condition that never reindexed at all already costs — the reindex term is
  gone, not merely cheaper. The scaling *exponent* does not drop toward
  O(*d*) (1.93→1.84 on ARM, 1.93→1.91 on x86): a second, independent O(*d*²)
  term in the descent/delete machinery (the #1690/#1651/#1301/#1213 chain)
  still dominates what's left, and is out of scope here. Realistic
  wide/shallow shapes (200k-record arrays) still see a real 1.16–1.25x win.
  yq mode shares `resolve_node` and benefits identically (up to 1.84x at
  D=240); `eval_generic.rs` has no `resolve_node` of its own, so this
  specific term has no twin there — though that evaluator's own reindex
  bridge (`eval_on_owned`) lacks even the pre-#2048 `Identity`/`Field`/
  `Index` fast path #491 gave this one, which may be worth its own
  follow-up.

  Correctness: 728 CLI cases (26 inputs × 28 queries spanning `del`/`path`/
  `|=`/`map_values`/`with_entries`/`recurse`) verified byte-identical
  against the pinned oracle (`/usr/bin/jq` 1.7.1) and against the
  pre-#2048 binary; 546 yq-mode cases likewise byte-identical
  stdout/stderr/exit-code against the pre-#2048 binary. The fast path is a
  closed allow-list (`is_owned_pure_expr`'s catch-all is `_ => false`, so a
  future `Expr` variant is never silently fast-pathed) and delegates every
  arm to the same helper the slow evaluator itself uses
  (`apply_compare_op`, `literal_to_owned`, `owned_type_name`, `is_truthy`)
  rather than restating any rule. `produces_fresh_value` is a second,
  independent gate catching a real representation bug an earlier version
  of this fix had: fast-pathing `.a.b` in *result* position would return
  `Int(2)` where the reindex bridge returns `NumberLiteral(Int(2), "2")`
  — the exact swap #1008's literal-preservation rule reads and #1054 is a
  live bug over, in the opposite direction. Navigation is therefore
  admitted only as an *operand* consumed by a comparison or truthiness
  test (which read all three numeric representations identically), never
  as an expression's own escaping result.

  A dedicated code review then caught a second, more subtle representation
  bug before merge: the fast path's `Expr::Compare` arm evaluated its left
  operand before its right one, backwards from
  `binary_fanout_each`'s real ordering (right is the outer generator, left
  the inner one — the same rightmost-outermost convention
  `builtin_pow`/`builtin_setpath` already document for their own
  two-argument fanouts). Observable whenever *both* operands would
  independently fail: `{"a":"str","c":1} | del(.. | select(.a.b == .c.d))`
  reported `.a.b`'s error instead of `.c.d`'s, live-verified against
  `/usr/bin/jq` 1.7.1 as a real divergence, not just an internal
  inconsistency. Fixed by evaluating right before left, matching the slow
  path; pinned by
  `compare_condition_reports_the_right_operands_error_first_2048`. Four
  smaller, non-blocking observations from the same review (a per-branch
  recompute cost, a duplicated purity predicate, and two low-severity
  readability/architecture notes) are split out to #2201.

### Fixed

- **`del()`'s trailing `.[]` raised instead of no-oping through a tolerated (rather than
  genuinely found) `null`, in yq mode** (#2347, found during #2324's own implementation).
  A `null` reached by tolerating a step against it — a missing object field, or an
  already-`null` slot navigated further — exempts the rest of a `del()` chain in real
  yq, `.[]` included; succinctly's own `Expr::Iterate` arm only handled the *vivify*
  case (a genuinely found/padded `null`, which turns into `[]`), with no fallback for
  the tolerated case, so it fell through to the generic "cannot iterate" error instead:

  ```console
  $ echo '{"y":1}' | yq -o=json 'del(.x.a[])'
  {"y": 1}
  $ echo '{"y":1}' | succinctly yq -o json 'del(.x.a[])'   # before this fix
  Error: Cannot iterate over null (null)
  ```

  Fixed by adding a `yq_mode`-gated `OwnedValue::Null => Ok(())` case to
  `delete_at_path`'s *terminal* `Expr::Iterate` arm and its mid-chain sibling in
  `delete_path_steps` (`.[]` followed by more path, e.g. `del(.x.a[].b)` — found during
  this fix's own review, initially missed since the terminal case alone was the issue's
  own stated repro), both ahead of their existing catch-alls — deliberately *not*
  unconditional the way the sibling `Index` arm's own `Null` fallback is, since jq mode
  has no such exemption at all (`{"x":null} | del(.x[])` still raises "Cannot iterate
  over null" in real jq regardless of how the null was reached, #527).

  A second, deeper bug surfaced while verifying this: `delete_at_path_through_absent`
  (the recursive walker for "the rest of the chain, against a throwaway `null`, once a
  step has already tolerated a genuinely *missing* object field") hardcoded
  `yq_mode = false` unconditionally, regardless of the caller's actual mode — silently
  downgrading every yq-mode call through it to jq's own stricter rule the instant the
  tolerance chain started from a missing key rather than a found-but-`null` value
  (`{"y":1} | del(.x.a.b[])`, three levels of missing-field tolerance, is a clean no-op
  in real yq but still raised before this fix). Fixed by threading the caller's own
  `yq_mode` through instead of hardcoding it — safe to do because the function's other
  hardcoded flag, `real_slot = false`, already blocks every vivify site on its own
  (each is gated `yq_mode && real_slot && ...`), so enabling `yq_mode` alone cannot
  newly reach any of them; confirmed live that the original safety property this
  function's own doc comment establishes (`{} | del(.missing[-1])` stays a clean no-op,
  never vivifying a throwaway value into an array first) still holds.

  This also retroactively fixes a gap #2324's own code review had explicitly flagged as
  known-but-out-of-scope: a comma-grouped `del(.missing[], .x)` (no `?` needed) now
  correctly no-ops to `{}`, matching real yq, instead of raising.

  **Known residual gap, not fixed here** (tracked as #2380): a comma-grouped sibling
  whose own trailing shape is `.[]` *followed by more path* (not a bare trailing `.[]`)
  still raises — `del(.missing[].x, .y)` on `{"y":1}` is `{}` in real yq, still an error
  here — because `vivify_del_comma_iterate_targets`'s own prefix-matching only recognizes
  a bare trailing `.[]`, falling through to the ordinary, read-based comma-branch
  resolution for anything with a suffix after it. The non-comma single-target form
  (`del(.missing[].x)` alone) is unaffected by this gap.

- **`del()`/`delpaths()` against an object with a wrong-kind key (numeric/null/bool)
  raised instead of no-oping in yq mode** (#2353). Real yq's own object indexing only
  understands string field names — handed anything else, it silently contributes
  nothing rather than erroring, unlike succinctly's prior behavior:

  ```console
  $ echo '{"a":1}' | yq -o=json 'del(.[5])'
  {"a": 1}
  $ echo '{"a":1}' | succinctly yq -o json 'del(.[5])'   # before this fix
  Error: Cannot index object with number
  ```

  Fixed at 4 sites in `src/jq/eval.rs` — `delete_keys`, `delete_at_path`,
  `delete_paths_under`, and `delete_path_steps` (the terminal, single-key, mid-chain,
  and mid-chain-through-`delpaths()` shapes respectively) — each gated on yq mode only;
  jq mode is unaffected, matching real jq's own unchanged error there. This asymmetric
  rule is scoped to **objects only**: an `Array` root with a wrong-kind key (string/
  null/bool) still errors in both tools, since real yq's own array indexing always
  tries to parse the key as an integer first (the same `strconv.ParseInt`-flavored
  mechanism #2333 above already established), and genuinely can't tell a wrong-kind
  key apart from an object-field key once the target is a sequence.

  **Known residual gaps, not fixed here** (both tracked as #2362): only the bare
  integer-literal shorthand (`del(.[5])`, parsed directly as `Expr::Index`) reaches the
  four fixed sites above.
  - A *comma-grouped* `del(.[5], .a)` still raises instead of no-oping `.[5]` and
    deleting only `.a`, because a `Comma` routes through a different code path
    (`resolve_node`'s shared leaf resolver, which evaluates via the ordinary value
    evaluator rather than any of the four functions above) that was found, during this
    issue's own investigation, to already raise for a **plain read** of `.[5]` against a
    bare object — a materially larger, read-level divergence rather than a
    `del()`-specific one.
  - A *computed* index (`del(.[5+0])`, `5 as $i | del(.[$i])`) — or a literal
    `null`/`true`/`false` key, since only a bare integer literal gets the `Expr::Index`
    shorthand — is `Expr::IndexExpr`, resolved through `resolve_index_expr`/
    `key_to_path_component`: generic path-resolution machinery shared by every
    path-consuming builtin (`path()`, every assignment operator, and `del()` alike, all
    funneled through the same `resolve_node` dispatcher), found during this PR's own
    review to still raise the same way. That machinery already has a yq-mode no-op
    placeholder mechanism (`scalar_noop`, #1181's own precedent) but it's unconditional
    once triggered, and the correct behavior differs by operation — `del()` should
    no-op, but assignment should stringify the key and write a new field instead (a
    separate, pre-existing divergence, unaffected either way) — so extending it here
    without a way to tell del() apart from assignment would fix one and regress the
    other.

- **`delpaths([[]])` printed the literal `null` instead of emitting nothing in yq mode**
  (#2352, found while implementing #2306). `del(.)` already special-cases "deleted the
  whole document" as no output at all (#1702, real yq's own root-delete rule), but
  `delpaths_one`'s own empty-path arm — the JSON-array-of-paths spelling of the identical
  operation — never picked up the same mechanism, since its own `Ok(OwnedValue::Null)`
  result had no way to signal "no output" once already wrapped as an ordinary value.
  Fixed by checking for the empty-path shape ahead of that result, in yq mode only (jq
  mode is unaffected — real jq's own `delpaths([[]])` already prints `null`, matching
  succinctly's existing jq-mode behavior). The empty path always sorts first in
  `delpaths_one`'s own path list (an existing invariant, unchanged), so a multi-path call
  with an empty path anywhere in it (`delpaths([[],["a"]])`, `delpaths([["a"],[]])`)
  collapses to the same "emit nothing" outcome regardless of where the empty path
  appears in the argument — confirmed live against yq v4.53.3 for all three shapes.

- **`del()` through a chained slice-run followed by more path (`del(.a[0:2].x)`) silently
  no-op'd instead of erroring when the slice target is an array or `null`** (#2333, found
  during #2323's own code review). #1219's own "trailing slice-run followed by anything
  non-slice makes the whole `del()` inert" rule classified every such shape uniformly as a
  no-op — correct for a string slice (#1219/#1321's own established precedent) and for a
  bare trailing `Index` (`del(.a[0:2][0])`), but real yq's own generic indexing can't tell
  an object-field key from an array-index key apart (both are spelled `.key`): once the
  current node is a sequence, it always tries to parse the key as an integer, which fails
  for a genuine field name and raises `cannot index array with '<name>'` instead of
  no-oping. Live-verified:

  ```console
  $ echo 'null' | yq -o=json 'del(.[0:2].a)'
  Error: cannot index array with 'a' (strconv.ParseInt: parsing "a": invalid syntax)
  $ echo '{"a":[1,2,3]}' | yq -o=json 'del(.a[0:2].x)'
  Error: cannot index array with 'x' (strconv.ParseInt: parsing "x": invalid syntax)
  ```

  Fixed via a new `yq_del_slice_field_error` check, added to `yq_del_slice_outcome`'s
  existing "last component isn't a slice" branch as a new `YqDelSliceOutcome::Error`
  outcome (wired into all 5 of that enum's call sites, one of which —
  `rewrite_yq_del_comma_branches`, the comma-branch pre-filter — needed its own return
  type widened from `Option<Expr>` to `Result<Option<Expr>, EvalError>` to propagate it).
  `null`'s array-shaped treatment is a quirk of being the *direct* slice target only
  (real yq's null-as-array slice has no actual elements behind it, confirmed live:
  `null | del(.[0:2][0].a)` stays `null`, not an error) — an `Array` target keeps the
  error live through arbitrarily deeper `Index` navigation instead, since real document
  data really is there to walk (confirmed live: `del(.a[0:2][0].x)` errors when index 0
  holds another array, stays a no-op when it holds a scalar or object). A comma-grouped
  sibling gets the same error treatment but propagated differently from #1219's own
  `Noop`-sibling rule: real yq fails the *whole* call, not just the erroring branch
  (confirmed live: `del(.a[0:2].x, .c)` on `{"a":[1,2,3],"c":9}` leaves `.c` undeleted
  too).

  Code review (round 2) found two real bugs in the first draft, both fixed: (1) the
  slice-run boundary was computed as the *last* slice's own position rather than the
  *start* of the maximal contiguous run ending there, so a chained *multi*-slice run
  before the field tail (`del(.a[0:2][1:3].x)`) left an earlier slice unguarded and
  silently misclassified back to `Noop` — fixed by walking backward to the run's actual
  start and applying every slice in it sequentially, matching `yq_del_slice_outcome`'s own
  established "a run of any length collapses to one target" rule; (2) a `?` on the
  erroring field step itself (`del(.a[0:2].x?)`, distinct from `del(...)`'s own outer `?`)
  was never consulted, regressing a case that was already a correct no-op on `main` before
  this PR (caught by an A/B diff against `main`'s own pre-fix binary, not just the new
  code in isolation) — fixed by checking `DeleteStep.optional` before raising. The null-
  handling special case was also refactored away during this round: rather than two
  hand-rolled checks (one for `null`, one for `Array`), a `Null` pre-slice value is now
  modeled as an empty `Array` and walked through the exact same suffix logic, since an
  empty array's own semantics (a `Field` step still hits the "current is an `Array`" arm;
  an `Index` step always resolves out-of-bounds and gives up) already reproduce the
  established no-op-past-the-direct-target behavior without a second copy of the walk —
  closing the "duplicated predicates diverge silently" gap the multi-slice-run bug above
  was itself an instance of.

  A third, pre-existing (not introduced by this fix, confirmed via an A/B diff against
  `main`) gap was found and filed separately as #2344 rather than expanding this PR's
  scope further: a literal `.[]` (`Iterate`) earlier in the path, before the chained
  slice-run (`del(.a[][0:2].x)`), still silently no-ops — the whole-path classification
  this fix lives in has no way to defer to `delete_at_path`'s own per-element `Iterate`
  handling once it's already committed to answering `Noop`/`Error` for the "no slice at
  the very end" shape.

- **17 more sites ignored `optional`, continuing the #2231/#2280 lineage** (#2327,
  follow-up from #2280's own code review): `eval.rs`'s `builtin_del` (its own doc comment
  already promised "any resulting error is caught right here, turning the whole call's
  output into empty when `optional` is set" — the code wasn't honoring it for this
  particular materialization), `builtin_transpose`, `builtin_group_by`/`unique_by`/
  `sort_by`'s shared per-item conversion, `builtin_halt_error`, `builtin_min`/`max`/
  `min_by`/`max_by`, `builtin_reverse`/`unique`/`sort`, `builtin_flatten`, plus
  `eval_generic.rs`'s `Reverse`/`Sort`/`SortBy`/`Unique`/`UniqueBy`/`Min`/`MinBy`/`Max`/
  `MaxBy` family (`collect_cursors_checked`) and its `bridge_to_full_evaluator`/`_flow`
  (the shared reindex-bridge helper several of those same arms fall back to). All now
  route through `to_owned_or_suppress!`/`to_owned_vec_or_suppress!`/`owned_or_suppress!`/
  `suppress_or_raise` instead of a bare match, matching every site had a genuinely live
  `optional` consulted elsewhere in the same function.

  **A first round of this fix (7 builtins) shipped with a test that didn't prove what it
  claimed.** Code review found `test_optional_ignored_sites_2327`'s use of a whole-document
  decode failure (`\ud800`) passed for `del`/`transpose`/`group_by`/`halt_error`/
  `sort_by`/`unique_by` for the wrong reason: `eval_generic.rs` is the CLI's real entry
  point (`jq_runner.rs`/`yq_runner.rs` never call `eval.rs`'s `eval()` directly), and every
  one of these `eval.rs` functions is reached from there only through a reindex-bridge
  mechanism (`eval_builtin`'s `_` catch-all arm, or `bridge_to_full_evaluator`/a native
  arm's own inlined equivalent) that calls `to_owned_with_cursor` on the *whole* document
  **before** ever reaching `eval.rs` — so a real decode failure always raises at the
  bridge, never at the site this PR fixed. Confirmed for `builtin_del` with temporary
  debug instrumentation (added and removed during review): its fixed line simply never
  runs for a `\ud800` document. The same review round also found `builtin_envvar` is a
  step further — unreachable through any parsed CLI syntax *at all* (`env.VAR` resolves
  through a different code path entirely), a fact this codebase already documented before
  this PR (`test_builtin_envvar_raises_on_decode_failure_1820`'s own comment) — so its fix
  is dropped from CLI-level test claims entirely and exercised only by that pre-existing
  unit test.

  The review round also found 8 more sibling functions with the identical unfixed
  asymmetry (`builtin_min`/`max`/`min_by`/`max_by`/`reverse`/`unique`/`sort`/`flatten`,
  all `eval.rs` — several of them the direct siblings of `eval_generic.rs` arms this PR's
  first round *did* fix, a direct instance of this project's own documented "two separate
  jq/yq evaluators" hazard) and one more shared bridge site
  (`bridge_to_full_evaluator`/`_flow`) — all now fixed too.

  Tests were restructured accordingly: `test_optional_ignored_sites_2327` now covers only
  the `Reverse`/`Sort`/… family's own `collect_cursors_checked` fix, the one part of this
  PR with genuine CLI-observable coverage (via the same trailing-comma fixture
  `test_jq_collect_cursors_checked_sibling_paths_reject_trailing_comma_2261` established,
  now also asserting the same `stderr` content that test does, not just the exit code). A
  new `test_eval_rs_sites_produce_correct_output_2327` covers every `eval.rs`-only site
  instead — proving the macro-swap refactor didn't change the success path's output, the
  same honest scope `test_eval_rs_only_sites_still_correct_2280` already accepted for
  `builtin_path`/`eval_format` in #2280.

  `eval.rs`'s `eval_array_construction` (#2327's own "plausible, higher-impact" candidate)
  was investigated and deliberately left unchanged: unlike the sites above, its own
  surrounding code has no local precedent of consulting `optional` to suppress *this*
  function's own error — `optional` there is only ever forwarded into the inner
  expression's evaluation, and the function's own comment states array construction is
  atomic in jq ("a `Partial` inner stream just surfaces its control, same as a bare one"),
  i.e. every error always propagates regardless of `optional`. Applying the same macro
  swap here would still be a no-op today (same `is_decode_failure()` reasoning), but
  unlike the confirmed sites it isn't clearly *correcting an inconsistency the surrounding
  code already established* — so it was left alone rather than changed on pattern-match
  alone.

  Roughly 58 more raw `to_owned`/`collect_cursors_checked` call sites remain unaudited in
  `eval.rs`/`eval_generic.rs` after this PR — review flagged the recurring #2231→#2280→
  #2327 pattern itself as the real problem (a sweep-and-patch cycle that keeps finding a
  few more sites each time, with no mechanism stopping the next one from being missed the
  same way), and a structural fix (CI/audit tooling, not another manual list) was filed
  as #2334 rather than continuing the same cycle here.

- **Five sites ignored `optional`, an asymmetry #2231 already fixed for
  `debug`/`stderr`/`tostring`/its catch-all fallback** (#2280, follow-up from #2231's own
  code review): `eval_generic.rs`'s `Expr::Format` arm (gating every `@format` builtin —
  `@json`, `@csv`, `@tsv`, `@dsv`, `@uri`, `@html`, `@base64`, `@sh`, `@yaml`, `@props`,
  `@text` — for the CLI's default dispatch), its `Builtin::Path`/`Builtin::GetPath` arms,
  `eval.rs`'s own `builtin_path` (reached via the public `succinctly::jq::eval::eval`
  library API, and via the CLI whenever `eval_generic.rs`'s own fallback re-enters the full
  evaluator for a non-identity value — e.g. a document-sourced number literal longer than
  256 digits), and `eval.rs`'s own `eval_format` (library-API-only: unlike `builtin_path`,
  `eval_generic.rs`'s reindex-bridge helper short-circuits every `Expr::Format` before it
  can reach this evaluator at all, so this site is never reached from the CLI) all raised an
  unconditional error from their own materialization instead of consulting `optional` the
  #2184/#2015/#2231 lineage's shared macros (`to_owned_or_suppress!`/`owned_or_suppress!`)
  already provide. `GetPath`'s arm was a particularly sharp inconsistency: its own comment
  already documents threading `optional` "for real" into `getpath_walk_owned`'s per-step
  decision, while the initial materialization two lines above stayed on the unconditional
  macro.

  This PR's own first draft fixed only `eval.rs`'s `eval_format`/`Builtin::Path`/
  `Builtin::GetPath` (the library-API-facing evaluator) and missed that
  `jq_runner.rs`/`yq_runner.rs` route the CLI's default dispatch through
  `eval_generic.rs` exclusively, never touching `eval.rs`'s own `eval()` at all for
  ordinary use — so the headline fix didn't reach the code path a real `succinctly jq`
  invocation actually takes. Code review caught this, plus that `eval.rs`'s own
  `builtin_path` carried the identical bug (unlike its sibling `getpath_one_path`, which
  already consulted `optional` correctly) and that the first draft's own regression test
  exercised `path(.a)`'s pre-existing cursor-navigable fast path rather than the
  `Builtin::Path` fallback line it claimed to pin. All three were fixed and the test
  corrected to use a non-cursor-navigable path expression (confirmed with temporary debug
  instrumentation during review that this reaches the intended line).

  A third review round then found the CHANGELOG's own claim that `eval.rs`'s `eval_format`
  is CLI-reachable via the reindex bridge (the same way `builtin_path` is) was itself
  wrong: `eval_generic.rs`'s `eval_on_owned` short-circuits any `Expr::Format` before it
  can reach `eval.rs`'s evaluator at all (the same round trip `format_owned`'s own doc
  comment already explains avoiding, #124), so `eval_format` is genuinely reachable only
  via the public library API. The fix itself was already correct either way (still
  defensive); only the reachability claim in its comment and this entry needed correcting.
  A parallel sweep from that same round found several more sites sharing this issue's
  exact pattern outside its own three named findings (`builtin_del`, the
  `Reverse`/`Sort`/`Unique`/`Min`/`Max` family, `builtin_envvar`, `builtin_transpose`, and
  a few lower-confidence candidates) — out of scope for this PR, filed as #2327 instead of
  expanded here.

  Like #2231's own findings 1-3, this is defensive rather than a live behavior change
  today: #2286 tagged every error path these materializations can produce (string decode
  failure, #1194 malformed member/delimiter)
  `is_decode_failure()`, and `suppresses(e, optional) = optional && !e.is_decode_failure()`
  is unconditionally `false` for all of them regardless of `optional` — verified live
  (`test_optional_ignored_sites_2280`/`test_eval_rs_only_sites_still_correct_2280`,
  `tests/jq_cli_tests.rs`), mirroring
  `test_try_catch_handler_still_runs_under_outer_optional_2231`'s own methodology. Still
  correct to fix: the macro swap also keeps propagating a genuine decode failure
  unchanged, and removes the dependency on this reachability analysis staying true if a
  future error class is ever routed through the same call.

  A fourth review finding — ~85 raw `to_owned(&…)` calls in `eval.rs` and ~12 raw
  `to_owned_with_cursor(` calls in `eval_generic.rs` still don't route through these
  macros, with no structural guard (rename, lint, CI grep) to stop the pattern
  recurring — was left as a judgment call rather than acted on here: the #2286
  convergence above means most of that remaining surface is likely already
  decode-failure-only too, lowering the urgency a Low-severity, non-live-reachable
  defensive-consistency issue justifies for a new CI enforcement mechanism. Noted on the
  issue for whoever picks up the next installment of this lineage.

- **`--argjson`/`--jsonargs` rejected a leading-dot (`.5`, #1171) or trailing-dot-before-
  exponent (`1.e5`, #2220) number literal outright, where real jq's own number parser
  accepts both** (#2240 Gap 2): a `--argjson`/`--jsonargs` value needing either leniency
  failed `serde_json`'s strict validation gate with no retry, unlike plain document input
  (which already tolerates both). Fixed via a new `normalize_dot_leniency` sibling to
  #1094's own `normalize_leading_zero_numbers`, composed into the same
  `normalize_json_leniently` retry pass shared by `--argjson`'s validator, `--seq`'s own
  per-value check, and yq mode's own `--argjson` — so all three pick up the fix at once,
  and a value needing more than one leniency at once (e.g. `007.e5`) still gets all of
  them, matching #2012's own "compose, don't chain independent retries" precedent. A
  bare `.`/`-.` with digits on *neither* side is still correctly rejected — not a number
  under any leniency, real jq included — caught by review before merge, since a first
  draft unconditionally synthesized `0`s around any dot it saw, silently "fixing" a
  genuinely malformed bare `.` into the valid number `0.0`.

  A second review round caught a more serious issue with that first draft: accepting a
  leading-dot literal via `--argjson`'s validation retry, then materializing it from the
  *original* text, silently lost precision (`--argjson x '.9999999999999999999999'`
  returned `1`, not jq's own `0.9999999999999999999999`) — `OwnedValue::from_number_bytes`
  (the primary document-input decoder) already had a leading-dot escape from #1171, but
  `StandardJson::number_literal()` (the generic evaluator's own materializer, which
  `--argjson` reaches) never did, so it fell through to a lossy `f64` parse. Fixed by
  extracting the escape into a new shared `has_leading_dot` helper
  (`src/json/validate.rs`, alongside the existing `has_trailing_dot_before_exponent`) and
  adding it to `number_literal()` too — as a side effect, this also fixes the identical
  precision loss for plain document input routed through any real query
  (`'.9999999999999999999999' | if true then . else empty end'` reproduced the same bug
  pre-fix, unrelated to `--argjson`). The same round narrowed `normalize_dot_leniency`'s
  own trailing-dot acceptance to the exponent-adjacent shape only (`1.e5`), *not* a bare
  trailing dot with nothing after it (`1.`) — accepting the bare form would trade a clean
  rejection for a different silent-corruption bug, a large-integer-precision gap
  `from_number_bytes`/`number_literal()` share and leave deliberately unfixed (confirmed
  pre-existing via plain document input too: `99999999999999999.` already materializes as
  `100000000000000000` on `main`). `1.` therefore still rejects via `--argjson`, a narrower
  divergence from real jq (which accepts it) than #2240's own issue text first suggested,
  kept rather than risking silent data corruption for a shape #2220's own scope
  (trailing-dot-*before-exponent*) never actually asked for.

  A third review round found a **critical** bug the second round's fix introduced:
  `succinctly yq --argjson x '.05' '.b = $x'` returned `0.5` — ten times too large.
  `normalize_json_leniently` composed `normalize_dot_leniency` *after*
  `normalize_leading_zero_numbers`, whose own number-token-start detection can't see a
  preceding dot — fed `.05` first, it starts a fresh scan right at the `0` after the dot
  and mistakes the fraction's own leading zero for a redundant *integer* leading zero,
  stripping it (`.05` → `.5`) before the dot-leniency pass ever runs. Invisible to jq
  mode's own tests, since `jq_runner.rs`'s own `parse_json_value` only uses the normalized
  text as a validation gate and always materializes the *original* text — but
  `yq_runner.rs`'s own `parse_json_value` reparses the *normalized* text directly as the
  value (by design, since yq's `--argjson` already discards number fidelity), making it
  the one place this composition-order bug was reachable. Fixed by reordering the
  composition: `normalize_dot_leniency` now runs *before*
  `normalize_leading_zero_numbers`, so a leading-dot token already has its synthesized `0`
  in front by the time the zero-stripper runs, and its integer part is never empty for the
  stripper to misinterpret. Verified with 500+ adversarial combinations of all three
  leniencies, in both jq and yq mode, across `--argjson`/`--jsonargs`/`--seq`.

  Gap 1 in the same issue (an array-navigated `tostring`/`@json` losing an
  overflow-literal's exact spelling) was investigated and found **not reproducible** on
  current `main` — both the root-level and array-navigated paths already agree, deferring
  identically to #1083/#1075's own earlier, deliberate policy of never preserving an
  infinite-magnitude `NumberLiteral`'s spelling in jq mode (the reindex bridge's own
  `1e999`/`-1e999` smuggling sentinel for a genuinely *computed* infinity is
  bit-for-bit indistinguishable from a real document literal of the same spelling once
  round-tripped, so reformatting either would risk mislabeling the other) — the issue's
  own "root-level path already matches" claim appears to have been stale by the time this
  was picked up.
- **`del()`/`delpaths()` silently no-op on a negative out-of-range array index instead of
  raising** (#2268): #2254 already fixed every ordinary *read* (`.a[-N]`) to raise real yq's
  own "index [N] out of range" error, but `del()`/`delpaths()` shared the identical gap with a
  more severe symptom — not a differently-worded error, but no error at all (exit 0, document
  unchanged), where real yq aborts the write. Five independent array arms shared it
  (`delete_at_path`'s single-step `del()` dispatch, `delete_path_steps`'s mid-chain `del()`
  equivalent for a pipe-chained path like `del(.a[-5].x)`, `delete_trie_array`'s equivalent for
  a comma-grouped path like `del(.a[-5].x, .c)` — code review found the latter two, missed in
  the first pass — plus `delpaths()`'s own `delete_paths_under` mid-path navigation and
  `delete_keys` terminal batch), fixed via a new `bool`-based sibling of #2254's own
  `yq_negative_index_check`/`yq_negative_index_error` helpers (all five take a plain
  `yq_mode: bool`, not `S: EvalSemantics`). Unlike the read-side fix, not
  suppressible by an earlier `?` in the path chain — confirmed live, real yq's own
  `del(.a?[-5])` still raises. A residual divergence was documented here, not chased at the
  time: when grouped with an earlier same-array deletion in one `delpaths()` call, real yq's
  own error reports the array's *already-shrunk* size (sequential resolution), where this
  crate's own `delete_keys` deliberately resolved every key against the array's length on
  entry — closed by #2306 below, which now reports the shrunk size too. Also surfaced, and
  filed separately as #2305 (out of this issue's negative-index scope): real yq's `del()` on
  a *positive* out-of-range index extends the array with nulls instead of no-op, which
  succinctly (matching jq) doesn't reproduce.
- **`delpaths()`'s multi-key batch applied as a union, not sequentially** (#2306): real yq's
  own `delpaths()` applies a 2+-path batch one path at a time, each seeing whatever state
  every earlier path already left behind — not jq's own simultaneous/union model this crate's
  `delete_keys` was built around, which resolves every path against the array's *starting*
  length. Confirmed live this isn't only about out-of-range indices — even an all-in-range
  batch is order-dependent (`delpaths([[0],[1]])` on `[1,2,3,4]` is `[2,4]`;
  `delpaths([[1],[0]])` on the same input is `[3,4]`, where real jq's own `delpaths` gives the
  order-independent `[3,4]` for both). `delpaths_one` (`src/jq/eval.rs`) now skips its
  upfront sort in yq mode (the sort exists to support jq's own batch semantics) and applies
  each path individually instead of handing the whole list to `delete_keys` in one call. jq
  mode is unaffected. `del()`'s own comma-separated multi-target form shares the same
  order-dependence in real yq and does not yet reproduce it, since each of its targets already
  reaches `delete_keys` through its own separate call rather than a shared batch.
- **`has(key)`/`contains()` on objects didn't raise on a non-string sibling key
  (`{"a":1,123:2}`), and `JsonFields::find` (unlike its `find_cursor` sibling) had neither
  the #1677 `,`/`:` delimiter check nor the #2261 trailing-comma check at all** (#2288,
  found reviewing #1995/PR #2287): `has(key)`'s `DocumentFields::contains_checked` (already
  checking `,`/`:` delimiters and #2261's own trailing-comma gap) now also derives "is this
  key malformed" from the same `key_display_string_kind` call it already makes for the match
  check (code review caught a first draft paying for a second, redundant decode pass via a
  separate `key_is_malformed` call) — free, riding the same walk, but only for keys visited
  *before* a match (the identical #2261-established "early exit legitimately misses a later
  fault" trade, since there's no O(1) shortcut to "is there a malformed key anywhere else in
  this object" the way there is for the trailing gap's single fixed position); see
  `docs/compliance/jq/limitations.md` for the full accounting, including this issue's own
  headline repro (`{"a":1,123:2} | has("a")`) remaining a documented, accepted gap. `find`
  gained the same deferred-to-the-winner `,`/`:` check and the same unconditional
  trailing-comma check `find_cursor` already had (a second code-review round caught that a
  first draft ported only the delimiter check and missed the trailing-comma one), reusing
  both rather than re-deriving them — confirmed not reachable from either shipped CLI today
  (same "library-API-only" shape as #2293's `eval.rs` gaps, which also covers `eval.rs`'s own
  `has_one_key` sharing this same non-string-key gap), fixed anyway since the change is
  narrow, mechanical, and reuses existing, already-proven checks within the same file.
- **`succinctly jq`'s most common query idioms -- `.[]`, `keys`/`keys_unsorted`, bare `.a`
  field access, `length`, `.[0]` -- silently accepted a trailing stray `,` after a real last
  array/object child** (#2261): `echo '[1,]' | succinctly jq -c '.[]'` printed `1` at exit 0,
  where real jq (1.7.1) rejects the document outright with a parse error (exit 5). #2243 had
  already closed this exact shape for `to_owned_cursor_at_depth`, but that materializer only
  backs the *non-cursor-transparent* route (`if`/arithmetic/function calls) — every idiom
  above takes its own native, cursor-carrying arm in `eval_generic.rs` that never calls it.
  Closed for `.[]` (`each_lazy_array_iterate_sink`/`DocumentElements::collect_cursors_checked`,
  which already walked every element's cursor for #1677's leading-gap check and now retain the
  last one to check the trailing gap too), `length`/`.[0]` on arrays (a new
  `DocumentElements::len_checked`, since resolving *any* array index — not just a negative one
  — already calls `.len()` to normalize/bounds-check it, so the check rides along for free on
  every index, reversing this issue's own assumption that `.[0]` had to stay an O(1)-lookup
  exception), `keys`/`keys_unsorted` in every shape (bare, `keys[]`, `keys_unsorted[]`,
  `keys_unsorted | last`, a negative `keys_unsorted[n]` — all via a new
  `DistinctKeyCursors::trailing_gap_ok`, which needed care to track the object's *textually*
  last field across a confirmed duplicate-key collapse rather than whichever key the
  collapsed/sorted output lists last), bare `.a`/`.nonexistent` field access
  (`JsonFields::find_cursor`, which — contrary to this issue's own "genuine O(1) lookup"
  description — already walks every field regardless of `name` to honor last-duplicate-
  key-wins, so the check is free there too), and `to_entries` on both arrays and objects (a
  new `effective_fields_with_raw_last`, since `to_entries`'s own collapsed field list can list
  a different field last than the raw walk once a duplicate key collapses). Left open, and
  documented in `docs/compliance/jq/limitations.md`: `keys_unsorted[0]` (a genuine O(1)
  positional lookup into an object's key list, matching #1629's own established
  "would cost strictly more than the answer" precedent), `length` on objects (`{"a":1,}|
  length`, not one of this issue's own repros — its walk deliberately never resolves any
  field's value, by design, since #1514), and #2211's own sibling shape (`[,]`/`{,}`, a stray
  `,` with *zero* real children) through every path above, none of which is ever handed a
  cursor to the container itself. `.[]` and a bare `keys_unsorted` are the two genuinely
  streaming writers among these fixes and can (like the pre-existing, already-documented
  `limit(3;.[])` case) still write a confirmed-good prefix to stdout before the trailing fault
  surfaces — not a new divergence, the same one already pinned for a truncating `.[]`
  consumer.

- **Eleven more `succinctly jq` sibling paths shared #2261's own trailing-stray-comma gap**
  (#2261 code review, PR #2291): `has(idx)` on arrays, `has(key)` on objects,
  `keys`/`keys_unsorted` on *arrays* (the object arm had already been fixed above), computed/
  dynamic index access (`E[K]`, e.g. `.[0,1]`/`.[$i]`), and `last`/`.[-1]` all shared the exact
  "already walks `.len()`/every field, so the check rides free" shape used throughout the
  #2261 fix above but had been missed in the first pass — e.g. `printf '[1,2,3,]' | succinctly
  jq 'last'` printed `3` at exit 0 where real jq (1.7.1) raises. A follow-up systematic sweep of
  every remaining unchecked `DocumentElements::collect_cursors`/`.len()` call site in
  `eval_generic.rs`/`document.rs` then turned up six more of the identical shape: `path(.[])`
  (whose array arm had drifted onto the unchecked `collect_cursors` even though its own
  object-arm sibling already used the checked helper) and
  `reverse`/`sort`/`sort_by`/`unique`/`unique_by`/`min`/`min_by`/`max`/`max_by` (all resolved
  every element via the unchecked `collect_cursors` despite the already-checked
  `collect_cursors_checked` — fixed for `.[]` itself — existing the whole time); `shuffle`/
  `pivot` share the identical code but have no jq oracle, fixed for internal consistency only.
  `has(key)` on objects is the one fix in this batch that is not free: `DocumentFields::contains`
  deliberately early-exits on a match (#1739), so answering the trailing-gap question too
  needs its own design — the shipped fix resolves it from the matched key's own cursor via two
  O(1) `next_sibling()` hops rather than walking further, after a first draft that dropped the
  early exit measured a real ~4x regression on `has()` for a key near the front of a
  1,000,000-key object. A match that is not the object's own true last field still takes the
  same #1629/#1770-established "early exit misses a later fault" trade every other truncating
  consumer in this codebase already accepts. Two further leads from the same sweep
  (`to_owned_with_comments_at_depth`, a fourth copy of the #2262 materializer family backing
  yq's write path; and `LazySource::advance`'s lazy pull behind `map(f)`, confirmed live and
  reproducible but not fixed here given the size/risk of touching that hot path) are documented
  in `docs/compliance/jq/limitations.md` as recommended follow-ups rather than folded in.

- **`succinctly yq --slurp`/`--eval-all`/`--inplace --input-format json` silently accepted a
  trailing stray `,` after a real last array/object child** (#2262): `echo '[1,]' | succinctly
  yq --slurp --input-format json -o json '.[0]'` printed `1` at exit 0, where real yq (v4.53.3)
  rejects the document outright. This was one of (at least) four independent materializers
  that each walk a JSON object/array and build an `OwnedValue`, checking for malformed comma
  placement as they go — `eval_generic::to_owned_cursor_at_depth` got both #2211's
  `container_gap_ok` (a stray `,` with *zero* real children, `[,]`/`{,}`) and #2243's
  `trailing_element_gap_ok` (a stray `,` *after* a real last child, `[1,]`/`{"a":1,}`); the
  other three — `eval.rs`'s own `to_owned_at_depth` (the materializer behind this crate's
  documented public library API), `eval_generic.rs`'s cursor-less `to_owned_at_depth` sibling
  (distinct from the cursor-carrying one above, reached from `GenericResult::One`/`Many`
  whenever a value materializes without a live cursor), and `yq_runner.rs`'s
  `to_owned_canonicalizing_numbers_at_depth` (the CLI-reachable one, above) — never received
  either fix. Fixed all three by reusing the same `DocumentCursor::trailing_element_gap_ok`
  primitive (moved from a private copy in `eval_generic.rs` into `document.rs` as `pub`, so
  all three plus the already-fixed original share one definition instead of drifting into
  four hand-copied ones) — each arm now retains its last real child's own cursor (already
  available via `uncons`/`uncons_cursor`) and checks it after the loop. `container_gap_ok`
  (`[,]`/`{,}`) remains a known, deliberately unclosed gap in all three: unlike
  `to_owned_cursor_at_depth`, none of them is ever given a cursor for the *container itself*
  (only a bare `value: &V`), so once a container's child walk is exhausted there is no cursor
  left to find its opening bracket from — the same limitation #2211 already documented for
  `jq_runner::standard_json_to_jq_value`'s identical value-only shape. `eval.rs`'s
  `to_owned_at_depth` also gained #1677's malformed-`,`/`:` delimiter check, which it had none
  of at all. Neither `eval.rs`'s nor `eval_generic.rs`'s cursor-less fix is independently
  reachable through the shipped CLI with raw untrusted text (every production caller
  re-serializes an already-materialized `OwnedValue` first) — both are library-API-only
  completeness fixes, pinned by direct unit tests calling the function itself.

  **Code review on this same fix (#2276) found the CLI fix above was reachable only through a
  filter that forces the DOM path (`.[0]`), not `--slurp`'s/`--inplace`'s own M2 streaming fast
  path for a plain identity/M2-streamable filter** — which parses via `YamlIndex`/
  `mark_json_sourced` instead (JSON is a syntactic subset of YAML's flow grammar), bypassing
  `to_owned_canonicalizing_numbers_at_depth` and its new checks entirely, since that grammar
  legitimately allows a trailing `,`. For `--inplace` this was silent data loss, not just wrong
  output: `printf '[1,]' > f.json && succinctly yq -i --input-format json '.' f.json` rewrote
  the file to `- 1` at exit 0, where real yq refuses and leaves it byte-for-byte untouched.

  Two approaches were tried. Extending `YamlCursor`'s own `DocumentCursor` overrides
  (`container_gap_ok`/`trailing_element_gap_ok`/`preceding_delimiter_ok`) to validate commas in
  place when JSON-sourced — keeping the same parser and its same parse-time depth guard, so no
  depth regression could occur — was built and confirmed live *not* to work: `--slurp`'s and
  `--inplace`'s M2 fast path streams cursor results directly
  (`stream_json_sequence`/`stream_yaml_sequence`, `YamlCursor::stream_json`/`stream_yaml`)
  without ever materializing through `to_owned_cursor_at_depth`, the only place those trait
  methods are consulted — the whole point of M2 streaming is to avoid exactly that step, so the
  overrides were simply never reached by the code path that needed fixing. Reverted in favor of
  declining the fast path for JSON-sourced input (reusing the exact run-wide `any_input_is_json`
  gate #978 originally introduced and #996 later removed for a different reason), which both
  `--slurp`'s and `--inplace`'s own `else` arm already correctly rejected the same input via —
  this fix's own materializer — with no further changes needed to either fallback.

  **That in turn silently loosened a different guard**: `YamlIndex`'s own parser enforces a
  128-deep parse-time nesting limit, while `to_owned_canonicalizing_numbers_at_depth`'s own
  guard is a looser, panicking 256-deep one (a different ceiling for a different reason — stack-
  overflow safety for the conversion step, not fidelity with what the fast path used to reject).
  Declining the fast path therefore silently *accepted* JSON nested 129–255 levels deep that the
  fast path used to reject at parse time, confirmed live before this correction (150 levels via
  `--slurp` printed the full structure at exit 0). Closed by `parse_input_m2_parity`
  (`yq_runner.rs`), a depth-128 pre-check specific to `--slurp`'s and `--inplace`'s own DOM
  fallback (not `--eval-all`'s, which never had this 128-deep guarantee to begin with, so
  tightening it there would be an unrelated behavior change) — verified to match the pre-#2276
  binary exactly at every boundary value (127/128/129/255/256/257), including the off-by-one an
  earlier revision of this same check got wrong (128 levels of plain nesting is accepted, not
  rejected — `YamlIndex`'s own `nesting_depth` counter is checked before incrementing and never
  counts a leaf scalar as its own nesting level). This incidentally also means `--slurp`/
  `--inplace` no longer reach the 256-deep guard's panic at all (previously reachable — and,
  briefly, *more* reachable — through this same fallback); `--eval-all`'s own, unrelated,
  pre-existing instance of that exact panic is unaffected and is now tracked as its own issue,
  [#2282](https://github.com/rust-works/succinctly/issues/2282).

  Known cost, factored into one `fast_path_json_comma_safe` term (#2276 review: `&&
  !any_input_is_json` had been copy-pasted into three separate gate definitions) rather than
  left duplicated: `--slurp`/`--inplace` with well-formed, *duplicate-keyed* JSON input
  collapses those duplicates again (the exact #996 regression this same gate caused before #996
  fixed it at the source for the plain stdout M2 path, left untouched here) — accepted
  deliberately, and already tracked as a known DOM-path limitation (#1343). Also spelled out
  explicitly rather than left implicit in "conservative for mixed-format": a well-formed YAML
  file's own duplicate keys collapse too if it shares an `--inplace`/`--slurp` invocation with
  even one JSON-sourced file, since `any_input_is_json` is one run-wide boolean with no per-file
  M2-vs-DOM switch — pinned by
  `test_yq_inplace_mixed_yaml_json_files_collapses_yaml_duplicate_keys_too_2276`. The plain
  stdout M2 path (no `--slurp`/`--inplace`) has the identical underlying validation gap but was
  left alone: its own fallback (`evaluate_yaml_direct_filtered`) shares the same gap, so
  declining that fast path would cost performance with no correctness gain — a real, broader,
  non-destructive sibling issue, tracked separately rather than folded in here.

- **`del(EXPR | .)` -- a `del()` path whose *last* component is a bare `.` reached after
  navigating through a real `Field`/`Index`/`Iterate` step -- nulled the targeted slot in place
  instead of removing it from its parent container** (#2256), diverging from both real jq and
  real yq: `{"a":{"b":1,"c":2}} | del(.a | .)` gave `{"a":null}` where jq 1.7.1 gives `{}`, and
  `{"a":[1,2,3]} | del(.a[0] | .)` gave `{"a":[null,2,3]}` where jq gives `{"a":[2,3]}` --
  reproduced the same way through `Iterate` (`del(.[] | .)`) and in yq mode. Root cause:
  `delete_path_steps`'s main loop reassigns `root` into the child slot a `Field`/`Index` step
  names and loops, so by the time a trailing bare `.` is reached, `root` no longer has a way
  back to the map/array that held it -- `delete_at_path`'s own `Expr::Identity` arm (correct
  for a genuine top-level `del(.)`, where there truly is no parent) then just nulls the child in
  place instead. Fixed with a new `trailing_identity_optional` lookahead: before navigating into
  a `Field`/`Index`/`Iterate`/`Slice` step's target, check whether everything left in the path
  (`rest`) reduces to nothing but a trailing `.` -- any number of `.`/`(.)`/`(.)?` wrappers deep
  -- and if so, delete at the *current* position (`root` is still the parent there) instead of
  navigating in at all, reusing `delete_at_path`'s own terminal-arm logic
  (`shift_remove`/`arr.remove`/`arr.clear`/`map.clear`/`arr.drain`) rather than duplicating it.
  `Iterate` needed this fix in *both* jq and yq mode -- yq's existing
  `YqDelSliceOutcome::DropParent(Expr::Identity)` mechanism only classifies a chained scalar
  *slice* per element and doesn't fire for a plain trailing `.` at all, confirmed still nulling
  every element pre-fix. `del(.)` alone (the genuine top-level case, never routed through
  `delete_path_steps`) is unaffected and still nulls the whole document, as does the
  no-real-navigation edge case `del(. | .)`.

- **`evaluate_bytes_lazy` (the default CLI path for a non-cursor-transparent jq filter --
  `if`/arithmetic/function calls/anything `expr_is_cursor_transparent` answers `false` for)
  silently accepted a stray `,` inside an apparently-empty array/object, anywhere in the
  document** (#2211): `{"a": [,]}" | if true then . else . end` printed `{"a":[]}` at exit 0,
  where real jq already rejects it — confirmed against `/usr/bin/jq` 1.7.1. Root cause:
  `eval_generic::to_owned_cursor_at_depth`'s array/object loops (the materializing conversion
  `eval_single`'s fallback arm uses for every expression it doesn't natively match) only ever
  validate the delimiter *preceding a real child* (#1677's `key_delimiter_ok`/
  `value_delimiter_ok`/`preceding_delimiter_ok` family) — when the walk produces zero real
  children at all, none of those checks ever run, so the container's own opening-to-closing gap
  was never inspected. Fixed with a new `DocumentCursor::container_gap_ok` method (default
  `true` for every format but JSON, same convention as `preceding_delimiter_ok`), whose JSON
  override reuses the exact same `trailing_gap_ok` primitive `src/json/light.rs`'s
  `stream_json`/`stream_json_pretty` and `src/bin/succinctly/jq_runner.rs`'s `print_json`
  already use for their own #1676 fix.

  Code review on the resulting PR (#2246) found two more independent materializers with the
  identical gap, the same "one fix, one sibling missed" shape this codebase has hit before
  (`cursor_to_owned_at_depth`'s own doc comment already references #998/#1021 for exactly this
  pattern): `jq::lazy::cursor_to_owned_at_depth` (backing `JqValue::materialize`/`into_owned`,
  reached whenever `-e`/`--exit-status` forces materialization) and
  `jq_runner::standard_json_to_jq_value` (backing the top-level `GenericResult::One`/`Many`
  conversion) *both* validated nothing at all for their own `Array` arms — not even the older
  #1677 missing/doubled-comma-*between two real children* check, which
  `cursor_to_owned_at_depth`'s own `Object` arm neighbor already had (#1956) and
  `standard_json_to_jq_value`'s `Object` arm never did either. Neither gap was reachable as a
  live differential *through the `succinctly` CLI binary against real jq*: whatever either
  function converts is still printed via `write_output_jq_value`/`print_json` immediately
  afterward regardless of what `-e`'s own materialize() call found, and `print_json`'s
  independent, pre-existing #1643/#1676 checks re-validate the same document on the way out —
  confirmed live against the pre-fix binary (`git stash`) that every CLI-reachable repro tried
  still correctly rejected malformed input purely through that redundant check.

  That masking is CLI-specific, though, and does not extend to this crate used as a library:
  `JqValue` is public API (`pub use lazy::JqValue` in `src/jq/mod.rs`), and
  `JqValue::from_cursor(...).materialize()`/`.into_owned()` are directly callable with no
  `print_json` redispatch anywhere in that call chain — code review confirmed empirically
  (reconstructing the pre-fix function in isolation) that a library embedder calling
  `materialize()`/`into_owned()` directly on a cursor built over `[1 2, 3]` or `[,]` got a
  silently wrong `OwnedValue` back, not an error. So this was a live, silently-wrong-output bug
  for any consumer of the public `JqValue` API, not merely a latent defense-in-depth
  improvement — the CLI's own redundant `print_json` check happened to mask it for the one
  entry point (`succinctly jq`) this issue's own repro used. Both are fixed now regardless,
  each reusing the same per-child `preceding_delimiter_ok`/`preceding_gap_ok` check the
  array/object arms that already had it use — a future refactor that ever removes the masking
  check in `print_json` would otherwise have silently reintroduced this bug for the CLI too,
  with nothing here left to catch it.
  `cursor_to_owned_at_depth`'s `Array`/`Object` arms also gained `container_gap_ok`, the same
  empty-container check as `to_owned_cursor_at_depth` above (it has its own container cursor to
  check against); `standard_json_to_jq_value`'s arms do not gain an equivalent empty-container
  check — by construction, `GenericResult::One`/`Many` are only ever produced when *this specific
  node's* own cursor position is not being tracked (see `Expr::Identity`'s own doc comment in
  `eval_generic.rs`), so there is no container-level position here to check safely; each real
  child's own cursor is unaffected by that and still gets its own check. Both additions are
  covered by direct unit tests calling these functions in isolation (bypassing `print_json`'s
  masking entirely) rather than CLI-level tests, which cannot distinguish the fix from the
  pre-existing redundant check — confirmed each new test fails against the pre-fix function body
  and passes against the fixed one.

  Verified no behavior change on well-formed input (full existing suite, unit tests for all
  three fixed functions, plus a 100 KB generated-JSON differential sweep against `/usr/bin/jq`
  1.7.1, byte-identical) and that the primary fix reaches every nesting depth, not just the top
  level (`to_owned_cursor_at_depth` recurses via cursor at every container level). A related but
  distinct shape — a trailing comma after a *real* scalar last child (`[1,]`, `{"a":1,}`),
  needing the last child's own text-end position rather than the container's opening position —
  is a known, still-open gap on all three of these paths, filed as #2243 and pinned by
  `test_jq_lazy_path_trailing_comma_after_scalar_last_child_still_a_known_gap_2243` rather than
  left silently uncovered.

- **`|=`/`+=`/other compound assignment operators, and `del()`, could crash the process on a
  sufficiently long chained path** (`.k0.k1.k2...` or `del(.k0.k1.k2...)`, #2115) — a
  sufficiently long chain put one native stack frame on every path component, `SIGKILL` at
  500,000 components in a release build, a real `fatal runtime error: stack overflow` between
  roughly 128 and 192 in a debug build. `update_path`'s `Expr::Pipe` arm used to rebuild
  `Expr::Pipe(exprs[1..].to_vec())` and recurse once per component, the same shape #1429/#2105
  already fixed for `=`'s own walker (`set_path_steps`) — but `=`'s fix doesn't transfer
  unchanged: `|=` applies a *filter* at the leaf, which can report "produced nothing"
  (#1877/#1894) from the terminal position no matter how deep it is, with no `Iterate` in sight
  (`null | .a.b.c |= empty` unwinds every freshly-autovivified level), where `=`'s
  already-materialized right-hand side has no such concept and only ever needs to unwind
  behind an `Iterate`. The new `update_path_steps` (`src/jq/eval.rs`) peels a step that
  navigates into an already-existing slot with no frame at all (structurally can never strand,
  matching `slot_was_stranded`'s own `created` gate) and, once a step has to autovivify,
  collects the *whole* maximal run of further steps that must autovivify too (autovivifying
  `Null` always succeeds and always produces an empty container, so nothing beneath it could
  already exist either), applies the update filter exactly once against a detached leaf
  standing in for that whole run, and builds or discards the result in one non-recursive loop.
  `delete_at_path` had no stranded-undo complexity to begin with (a step reaching `null` or an
  out-of-range index deletes nothing, #476/#477) — its new `delete_path_steps` reassigns `root`
  to the child slot and loops unconditionally, including through a chain of `null`s (a `null`
  slot can never gain a key, so there is nothing to detach a scratch value for either). Neither
  walker rebuilds an `Expr::Pipe` per step any more, incidentally also dropping the O(d²)
  clone cost `set_path_steps`'s own fix already removed on the `=` side. The full existing test
  suite passes unchanged, and both walkers were verified byte-identical to `/usr/bin/jq` 1.7.1
  and `yq` v4.53.3 across nested/computed/sliced/comma/`?`-suppressed paths, plus a 9,000-query
  differential sweep (code review, PR #2238) — a chain of 1,000,000 components no longer crashes
  in either a release or a debug build
  (`test_deep_flat_update_chain_exits_cleanly_not_stack_overflow_2115`,
  `test_deep_flat_delete_chain_exits_cleanly_not_stack_overflow_2115`,
  `test_deep_flat_delete_chain_through_absent_first_key_exits_cleanly_2115`, all in
  `tests/jq_cli_tests.rs`).

  One confirmed behavior change, found by that sweep: `update_path_steps`' fresh-run collapse
  (see `wrap_fresh`'s own doc comment) never bounds-checks an out-of-range/negative `Index`
  swept into an earlier fresh step's collection when the update filter it wraps produces no
  output, where the pre-#2115 recursive walker always checked eagerly, regardless of the
  eventual write outcome. This is an *improvement*, not a regression — real jq (1.7.1) defers
  this exact check the same way, so `null | .a[-1][1] |= empty` is `null` in real jq (no error)
  where this crate used to raise "Out of bounds negative array index" before #2115 —
  live-verified, and pinned by
  `test_update_index_bounds_check_deferred_behind_fresh_run_collapse_2115`. yq mode is
  unaffected (`undo_stranded` is jq-only, so yq mode always takes the eager path, matching real
  yq's own stricter requirement); the adjacent real-write case (`.a[-1][1] |= 9`) still errors
  identically either way.

- **A recursive `def` whose body is a wide, flat pipe or object literal could abort the
  process with a real native stack overflow instead of raising `MAX_EVAL_FRAMES`'s catchable
  error** (#2135). `install_def_calls`'s frame-charging model (#1371/#2080) only accumulated
  cost through *nesting* — an expression descending into another, like array-wrapping
  `[[[X]]]` — charging a flat `frames + 1` to every sibling of an `Expr::Pipe`/`Expr::Object`
  node regardless of how many siblings existed or which one held the recursive call. A
  200-stage pipe (or 200-entry object) wrapping a recursive `deep(m-1)` call in its last
  position carried real native stack cost per level of `deep`'s own recursion — `eval_pipe`'s
  and `build_object_entries`'s own per-sibling recursion is not guaranteed to be
  tail-call-eliminated — that the guard never counted, so `MAX_EVAL_FRAMES` (40,000) never
  fired and `deep(1500)` overflowed the real stack instead (confirmed live: `thread
  '<unknown>' has overflowed its stack`, SIGABRT, exit 134, where real jq 1.7.1 returns `null`,
  exit 0, on the identical filter).

  Fixed by charging pipe/object sibling `i` (0-indexed) `frames + i + 1` instead of a flat
  `frames + 1`, pricing a sibling near the end of a wide pipe/object the same as an
  equally-deep nested wrapping, while a sibling near the front — genuinely cheap in
  `eval_pipe`'s own recursion, since reaching it needs no extra frames — stays cheap. An
  ordinary long *non-recursive* pipe or wide array/object literal is unaffected either way:
  nothing is checked against `MAX_EVAL_FRAMES` until a `DefCall` is actually reached, and one
  with no recursive call inside never creates one, regardless of width (verified: a 100,000
  element array literal, a 50,000-key object literal, and a 50,000-stage non-recursive pipe
  all still evaluate exactly as before). `Expr::Comma` looks like the same shape but was
  confirmed, by direct experiment, *not* to share the gap — `eval_comma` is a flat loop over
  its siblings, not a per-sibling self-recursive call chain — so it was left unchanged.
  `MAX_EVAL_FRAMES` itself did not need to change: re-bisected crash floors for several
  width/depth shapes all show the guard now firing with better than 6x safety margin below
  the real (pre-fix) crash floor. See `install_def_calls`'s `Expr::Pipe`/`Expr::Comma`/
  `Expr::Object` arms and `MAX_EVAL_FRAMES`'s own doc comment (`src/jq/eval.rs`) for the full
  account.

- **`IN(s)`/`IN(src; s)` now forward the real ambient `optional` instead of
  hardcoding `false`** (#2015). Both builtins are defined in terms of
  `any(...)` (`IN(s)` is `any(s == .; .)`, `IN(src; s)` is
  `any(src == s; .)`), and `any`/`all`'s own shared implementation,
  `any_all_gen_cond`, already threads a live, used `optional` parameter
  through its root-value conversion and its generator's own error handling
  (#2001) — but `builtin_upper_in`/`builtin_upper_in_src` never picked up
  the matching update: the former called `to_owned_checked` unconditionally
  (its own `optional` parameter was `_`-prefixed and unused), and the latter
  passed a literal `false` into `any_all_gen_cond` regardless of its own
  (also unused) `optional` parameter.

  Found during review of issue #2015, which asked whether this was a
  genuine gap or a principled exemption matching `builtin_recurse_f`'s own
  (real jq's `recurse` has no internal optional-suppression concept, so its
  `optional` is deliberately left unused — see that function's own doc
  comment). It is not: unlike `recurse`, `IN`'s own doc comments describe it
  as delegating its fanout machinery to the *already-correct* `any`/`all`
  implementation, and no comment anywhere claimed a principled reason for
  `IN` to diverge from that sibling. Confirmed live against jq 1.7.1 that
  `IN`, `any`, and `recurse` all get caught uniformly by the outer `?` for
  an *ordinary* raised error (real jq has no internal suppression concept
  for any of the three, so it gives no signal either way); the actual
  distinguishing gap is entirely internal to this codebase's
  `optional`-threading discipline, in the same family as #1953/#2001/#2010.

  Not reachable via any live `?`/`try` syntax today — like every one of
  #2001's five sites, `Expr::Optional`'s own dispatch evaluates its inner
  expression with the *ambient* `optional` and lets the outer `eval_try`
  catch the result independently of what the callee does internally, so
  this closes an internal-consistency gap (and a possible future-caller
  trap) rather than a user-observable regression. A genuine decode failure
  (invalid UTF-8) is unaffected either way — that class is never suppressed
  by `optional`, with or without this fix.

- **A rejecting `select(...)` after a generator builtin (`paths`, `range`, ...)
  no longer swallows a surviving branch's own path-validity check** (#2050).
  Inside a path expression, when `select(...)` rejected at least one branch a
  *generator builtin* produced, the branches that survived stopped having
  their path-validity checked at all:

  ```console
  $ echo '{"a":{"b":1}}' | jq -c '[path(paths | select(length == 2) | .["a"])]'
  jq: error (at <stdin>:1): Invalid path expression near attempt to access element "a" of ["a","b"]

  $ echo '{"a":{"b":1}}' | succinctly jq -c '[path(paths | select(length == 2) | .["a"])]'
  [] # before this fix
  ```

  and, the shape a randomised differential fuzzer actually found while
  working #1690 (PR #2047):

  ```console
  $ echo '{"a":{"b":1}}' | jq -c 'del(paths | select(length == 2) as $p | getpath($p))'
  jq: error (at <stdin>:1): Cannot index array with string "a"

  $ echo '{"a":{"b":1}}' | succinctly jq -c 'del(paths | select(length == 2) as $p | getpath($p))'
  {"a":{"b":1}} # silent no-op, before this fix
  ```

  Root cause: `resolve_seq`'s multi-stage fan-out loop threaded the whole
  call's own terminal `keep` (`Keep::First`, #987's "stop a generator after
  its first output" rule) into **every** stage of the pipe, not just the
  genuinely terminal one. A bare generator builtin like `paths` has no
  dedicated `resolve_node` arm, so it falls to `resolve_leaf`'s general,
  `keep`-obeying case — and got truncated to its *first* output (`["a"]`)
  before `select(length == 2)` ever ran, discarding the second output
  (`["a","b"]`) that `select` would have let through. `resolve_fold_source`
  had already established the fix for its own "needs every output" case
  (`Keep::AtMost(usize::MAX)` in place of `Keep::First`, #1872); `resolve_seq`
  now applies that same widened `keep` to every stage except the pipe's own
  final one (only reached when a later stage or a non-empty static tail
  isn't still waiting to run), which is the one position the caller's actual
  demand still applies to.

  Confirmed live against jq 1.7.1 that this generalizes beyond `paths`:
  `path(range(3) | select(.==2))` raises "Invalid path expression with result
  2" (the candidate `select` actually lets through), never "...result 0" (the
  generator's own first output) — succinctly now matches. The two rows in
  the issue that were already correct (a non-rejecting `select(true)`, and a
  rejecting `select` over a literal `Expr::Comma` rather than a generator
  builtin) are unaffected and pinned by regression tests alongside the fix.

  `src/jq/eval_generic.rs` (the CLI's own bridge for both `jq` and `yq`
  modes) needed no matching change: it has no separate path-tracking
  implementation of its own for `path()`/`del()`/`=`/`|=` — every one bridges
  to `src/jq/eval.rs`'s `resolve_node`/`resolve_seq` via
  `bridge_to_full_evaluator`, so this fix in the one shared implementation
  covers the CLI in both modes.

- **`succinctly::jq::eval`'s object-construction key/value slots and jq-mode
  string interpolation no longer silently substitute `""` for an undecodable
  string** (#2022): all three fed their generator's output through
  `stream_outputs` (bare `to_owned`, via `collect_owned`) instead of the
  already-existing `stream_outputs_checked`, so `{(.a): 1}`/`{"k": .a}`/
  `"\(.a)"` on an undecodable `.a` silently produced `{"":1}`/`{"k":""}`/`""`
  instead of raising — the same #1746/#1972 bug shape this codebase has
  closed at ~15-20 other call sites (#1934's tracked lineage), just not yet
  at these three. Found during #1989's classification pass.

  Reachable only through the public `succinctly::jq::eval` library API, the
  same as #1755's original finding for this bug family — the bundled CLI is
  unaffected either way: `Expr::Object`/`Expr::StringInterpolation` have no
  dedicated arm in `eval_generic.rs` (the CLI's own bridge), so both fall to
  its wildcard arm, which already runs the *entire current value* through a
  checked conversion before this function is ever reached, regardless of
  whether the undecodable field is one the query actually touches.

  A related call site, `fanout_two_args`'s own argument materialization, was
  investigated but left unfixed pending its own dedicated verification — see
  #2165 (also corrects an earlier belief that `fanout_arg` was still open;
  it was already fixed by #2023).

- **Self-recursive and branching user-defined `def`s now evaluate** (#1371). `def
  sum_to(n): if n == 0 then 0 else n + sum_to(n-1) end; sum_to(100)` returns `5050`, and
  `sum_to(10000)` returns `50005000`, both matching jq 1.7.1; naive `fib` works where
  every branching recursion previously failed at *any* depth, zero included.

  `def` bodies were substituted into their call sites before evaluation began, which
  cannot terminate for a self-recursive definition — expansion has no way to observe that
  a runtime base case will be reached — and so was bounded by three guards that refused
  ordinary recursive filters. Calls are now bound to their definition and substituted per
  call, when evaluation reaches them, with arguments captured behind a shared node that
  keeps the recursion linear rather than quadratic. See
  [ADR-0020](docs/adrs/adr-0020.md).

  A non-terminating `def` still fails, now as a catchable error rather than the abort real
  jq produces for the same input. Deep recursion in a body that holds a lot of structure
  live stops earlier than jq's heap-allocated VM stack does (confirmed against jq 1.7.1's
  source, not just its behavior — see ADR-0020); both are recorded in
  [docs/compliance/jq/limitations.md](docs/compliance/jq/limitations.md).

### Added

- **`reduce`/`foreach`'s own `as` clause accepts a full destructuring pattern**
  (#1201), not just a bare `$var`: `reduce .[] as {a: $a} (0; . + $a)` and
  `foreach .[] as [$a,$b] (0; . + $a + $b; .)` now evaluate instead of failing
  to parse. The clause previously hardcoded `$` + an identifier, even though
  `. as PATTERN | …` and function-argument patterns already went through the
  shared `parse_pattern` — routing these two sites through it as well also
  brings them under #1240's `MAX_PATTERN_DEPTH` guard for free.

  `Expr::Reduce`/`Expr::Foreach` carry a `Pattern` in place of the old
  `String`; a bare `$var` is `Pattern::Var`, so existing queries are
  unaffected. The evaluator destructures each input element with
  `extract_pattern_bindings` — the same primitive `. as PATTERN` uses — and
  folds `substitute_var` over the resulting bindings, extending the
  AST-rewrite binding mechanism these constructs already had from one
  variable to N.

  Unlike a bare `$var`, a pattern can fail to match a given element. Because
  `foreach` emits one output per step, that failure has to surface only when
  the fold actually reaches the offending element, leaving every earlier
  step's output in place — the same contract #494 established for an ordinary
  per-step UPDATE error. Verified against jq 1.7.1 across 19 differential
  cases including exit codes and stderr text, and pinned by nine new
  `jq-golden` cases.

  **Known divergence:** `?//` alternatives are still rejected here, though
  real jq accepts them. Retrying an alternative after the body errors means
  rolling the accumulator back to the element's pre-UPDATE value, which the
  fold has no way to express — tracked separately as #1365.

### Changed

- **YAML parsing specializes on whether the document contains a carriage return**
  (#340), recovering most of the cost #324 paid for CRLF and lone-CR
  correctness. `build_semi_index` runs one SIMD pass over the input and parses
  with `Parser::<false>` or `Parser::<true>`; the LF-only monomorphization —
  nearly every document — compiles out every `\r` arm and keeps the pre-#324
  codegen. Interleaved `yaml_bench` versus `c5dab403`, excluding block scalars:
  ARM (M4 Pro) +4.0% → **+0.7%**, x86 (7950X) +11.0% → **+4.7%**, with x86 block
  scalars 31–34% *faster* than the pre-#324 baseline. End-to-end `yq` over 32
  configurations recovers completely: x86 +5.0% → **+0.8%** median, ARM +2.3% →
  **−0.2%**. CRLF documents are unchanged (+1.1% x86 / +0.5% ARM, the precheck
  early-exiting). Output is byte-identical to #324 across 244 file × query
  configurations on both architectures.

  The one shape that regresses is long quoted scalars, +7% to +12%: the parser
  bulk-skips those at ~15 GB/s, so the precheck's second pass over the input is
  large next to the parse it precedes. That is the standing cost of the gate.

  **Breaking** (low-level): `succinctly::yaml::simd::classify_yaml_chars` takes a
  `const HAS_CR: bool` parameter — call it as `classify_yaml_chars::<true>(..)`
  for the previous behaviour.

- **`reduce`/`foreach` no longer redo `substitute_var`'s AST rebuild once per
  INIT fork, and drop a dead `acc`/`state` clone every step** (#695, a #534
  follow-up): `substitute_var(update, ...)` (and `foreach`'s
  `substitute_var(ext, ...)`) depends only on the current input element,
  never on which INIT fork is running, but sat inside the INIT-fork loop and
  was recomputed on every `(init_val, input_val)` visit — worse for
  `foreach`'s EXTRACT substitution, nested one level deeper still inside the
  UPDATE-fanout loop, so a k-way UPDATE fanout rebuilt it k times per input
  element. Both are now precomputed once per input element before the
  INIT-fork loop runs. `acc`/`state`'s `.clone().unwrap_or(...)` — immediately
  followed by an unconditional overwrite with no read in between — is now
  `.take()`, dropping a full-value clone every step.

- **`reduce`/`foreach` gained a shared step budget bounding their INIT ×
  UPDATE × EXTRACT fanout** (#695, a #534 follow-up): `while`/`until` got a
  `WHILE_UNTIL_MAX_STEPS` cap (10,000) in #534 to bound their new fanout, but
  `reduce`/`foreach` got no equivalent, so a query shaped like `reduce
  (range(100000)) as $x ((range(100000)); .+$x)` ran unbounded — a finite but
  enormous product of INIT-fork count × input length × UPDATE/EXTRACT width,
  slow rather than hanging, but an easily-typed resource-exhaustion vector.
  A new `REDUCE_FOREACH_MAX_STEPS` cap (10,000, shared across every INIT
  fork, mirroring `WHILE_UNTIL_MAX_STEPS`'s "whole tree, not per-branch"
  accounting) now errors with `reduce: maximum iterations exceeded` /
  `foreach: maximum iterations exceeded` instead. Charged once per UPDATE
  eval, and — in `foreach` only — once more per EXTRACT eval, since a single
  UPDATE output can fan out into far more EXTRACT evals than there are
  source elements; the plain-copy path when `foreach` has no EXTRACT clause
  is left unbudgeted, since it does no evaluator work of its own and
  charging it too would wrongly cap legitimate high-cardinality output.

- **`reduce`/`foreach`'s step budget (#695, above) no longer refuses an
  ordinary fold over more than 10,000 elements** (#2079): with exactly one
  INIT fork and a single-output UPDATE — the overwhelmingly common shape —
  the shared budget degenerated into a plain cap on input-element count
  rather than the multiplicative INIT-fork × element × UPDATE/EXTRACT-width
  fanout it was meant to bound, so `reduce .[] as $x (0; . + $x)` on a
  50,000-element array refused with `reduce: maximum iterations exceeded`
  where real jq returns `1249975000`. `foreach`'s three-argument form
  (explicit EXTRACT) compounded this further, capping at *half* the
  two-argument ceiling. Fixed by splitting `REDUCE_FOREACH_MAX_STEPS` from
  `repeat`'s own, unrelated per-round width cap (previously the same
  constant by coincidence — now `REPEAT_WIDTH_BUDGET`, still `10000`) and
  raising `reduce`/`foreach`'s own budget 10x, to `100_000` (2x above this
  issue's own 50,000-element repro) — the genuine fanout-explosion case
  #695 introduced this guard for still errors, just at a higher,
  no-longer-accidentally-reachable-by-ordinary-use ceiling. See
  [docs/compliance/jq/limitations.md](docs/compliance/jq/limitations.md)
  for why the raise stops at 10x rather than further: it bounds round-trip
  *count*, not per-round-trip *cost*, and a superlinear-cost UPDATE/EXTRACT
  body still burns real CPU proportional to the ceiling before erroring
  (measured, not just theoretical — surfaced a separate O(n²)
  string-accumulation gap against real jq's own near-linear cost, filed
  as #2086).

- **`while`/`until`'s own step budget (`WHILE_UNTIL_MAX_STEPS`, #534, above)
  no longer refuses an ordinary loop over more than 10,000 iterations**
  (#2087): the identical bug shape #2079 (above) fixed for
  `reduce`/`foreach` — a flat count charged once per state visited,
  regardless of genuine backtracking fanout vs. an ordinary non-forking
  loop — so `0 | until(. >= 50000; . + 1)` refused with `until: maximum
  iterations exceeded` where real jq returns `50000`. Fixed the same way:
  raised `WHILE_UNTIL_MAX_STEPS` 10x, `10000` to `100_000`, 2x above this
  issue's own 50,000-iteration repro. See
  [docs/compliance/jq/limitations.md](docs/compliance/jq/limitations.md)
  for the same superlinear-cost-body caveat #2079/#2086 already
  established, measured again here for `while`/`until`'s own evaluation
  path rather than assumed to carry over unchanged.

- **`reduce`/`foreach`'s bare string/number-accumulating UPDATE body
  (`. + <literal>`) is now ~45x faster (still O(n²) — see below)** (#2086):
  `substitute_vars` folds a fold's `$x` into a `Literal` node before the
  loop runs, so `reduce EXPR as $x (INIT; . + $x)`-shaped bodies reduce to
  a bare `Expr::Arithmetic{Identity, Literal}` — a shape
  `eval_owned_fast_path` didn't cover, so every step fell through to the
  general evaluator's `to_json_for_reindex` + `JsonIndex::build` round-trip
  over the *whole* current accumulator. `reduce range(N) as $x (""; . +
  "x")` measured ~7.8s at `N=99999` (`REDUCE_FOREACH_MAX_STEPS`'s own
  ceiling); real jq answers in ~0.02s at that scale. Fixed by extending
  `eval_owned_fast_path` to answer `. + <literal>` directly against the
  `OwnedValue` tree, reusing the same `arith_combine` dispatch every other
  arithmetic call site already shares — measured ~0.17s at the same
  `N=99999` after the fix, ~45x faster. **This removes the JSON
  round-trip's cost, not the fold's own quadratic shape**: the new arm
  still clones the whole accumulator (`input.clone()`) every step before
  handing it to `arith_combine`, and `String`/`Vec` clones allocate at
  exact capacity, so the very next append reallocates a second time
  anyway — two O(current-size) copies per step either way, just memcpy
  instead of serialize/parse. Measured post-fix scaling curve (interleaved
  runs, net of process-startup floor): N=6,250 → 1.89ms, 12,500 → 4.74ms,
  25,000 → 13.53ms, 50,000 → 35.24ms, 99,999 → 133.58ms — per-step cost
  keeps rising (0.30µs → 1.34µs across that range) and the last doubling
  costs ~3.8x, both the signature of O(n²), not O(n) (which would show
  flat per-step cost and a ~2x doubling ratio). A true linear fix needs
  the fold's already-fully-owned accumulator state passed *by value* into
  arithmetic so `arith_add`'s existing in-place `push_str`/`extend` can
  reuse capacity instead of being hand a fresh clone every step — out of
  scope here since `eval_owned_fast_path` is shared by 3 call sites
  (`eval_each_owned`, `eval_owned_expr_full`, `eval_owned_input`) that all
  need `input` back on the fallback branch; tracked as #2157.
  Array/object-accumulating shapes (`. + [$x]`) are not covered by this
  fix at all (measured *worse* than the string case, ~36s at just 25,000
  elements) — tracked separately as #2152.

- **A decode failure (invalid UTF-8, or a structurally malformed value) now
  raises instead of silently materializing as `null`, `""`, or a dropped
  field**, on nearly every route that turns lazily-indexed JSON/YAML into an
  owned value (#1242, #1247). Concretely:

  - **`succinctly yq` rejects a non-UTF-8 document outright** (exit 1,
    `YAML parse error: invalid UTF-8 ...`), unconditionally rather than only
    under `--validate`, where it previously accepted it and returned `null`
    for the affected scalar at exit 0. Matches real yq.
  - **`succinctly yaml validate` / `yq --validate` grew a UTF-8 check** they
    never had — `YamlValidationErrorKind::InvalidUtf8` was declared but
    never constructed before this.
  - **`succinctly jq` substitutes U+FFFD** for invalid UTF-8 in document/raw
    input (and, since #1719, in `@base64d`/`@urid`'s own decoded output)
    instead of echoing raw bytes to stdout, matching jq 1.7.1's
    maximal-subpart substitution rule. A remaining fidelity gap against a
    second jq quirk here is tracked as #1617/#1717 (see
    [docs/compliance/jq/limitations.md](docs/compliance/jq/limitations.md#jqs-own-utf-8-replacement-character-substitution-fixed-at-function-granularity-open-at-document-granularity)).
  - Two streaming-output writers were deliberately left out of this sweep as
    a deferred "Stage 6" (their only error channel is `core::fmt::Result`,
    which carries no message) and still silently substitute — see
    [docs/compliance/yq/limitations.md](docs/compliance/yq/limitations.md#a-bad-escape-in-a-streamed-scalar-still-degrades-silently-instead-of-raising).
  - A structurally malformed object key (#1194) preserves rather than raises
    on the materializing routes, a deliberate divergence recorded by #1642 —
    the two are handled differently by design, not by omission.

  **Breaking:** every signature below returns `Result<_, EvalError>` where it
  previously returned the bare value — any downstream caller of the public
  `succinctly::jq` API using one of these will need a `?` or an explicit
  match at the call site:

  | item | before | after |
  |---|---|---|
  | `jq::lazy::JqValue::materialize` | `-> OwnedValue` | `-> Result<OwnedValue, EvalError>` |
  | `jq::lazy::JqValue::into_owned` | `-> OwnedValue` | `-> Result<OwnedValue, EvalError>` |
  | `jq::eval_generic::to_owned` | `-> OwnedValue` | `-> Result<OwnedValue, EvalError>` |
  | `jq::eval_generic::to_owned_cursor` | `-> OwnedValue` | `-> Result<OwnedValue, EvalError>` |
  | `jq::eval_generic::to_owned_with_comments` | `-> (OwnedValue, CommentTree)` | `-> Result<(OwnedValue, CommentTree), EvalError>` |
  | `GenericResult::into_owned` | `-> Option<OwnedValue>` | `-> Result<Option<OwnedValue>, EvalError>` |
  | `GenericResult::collect_owned` | `-> Vec<OwnedValue>` | `-> Result<Vec<OwnedValue>, EvalError>` |
  | `DocumentFields::keys` | `-> Vec<String>` | `-> Result<Vec<String>, EvalError>` |
  | `jq::document::effective_keys` | `-> Vec<String>` | `-> Result<Vec<String>, EvalError>` |

  Also breaking: `yaml::validate::YamlValidationErrorKind::InvalidUtf8` went
  from a unit variant to `InvalidUtf8 { reason: &'static str }`, so any
  exhaustive `match` on it will need updating.

### Added

- **New public surface supporting the decode-failure raising above (#1247):**
  `DocumentValue::string_decode_error`, `JsonError::message`,
  `YamlStringError::message`, `Utf8ErrorKind::message`.

- **`yq` gains `--front-matter`, `--split-exp`, and `--eval-all`/`file_index`**
  (#715), closing three real `yq` CLI gaps found by a systematic gap-audit:
  - `--front-matter=extract|process` operates on YAML embedded as front
    matter in another file (e.g. a Markdown post's `---`-fenced header):
    `extract` evaluates the expression against just the front matter,
    discarding the body; `process` re-emits the transformed front matter
    followed by the original body, byte-for-byte unchanged.
  - `--split-exp EXPR` splits output into one file per result, named by
    evaluating `EXPR` against it (`.` is the result, `$index` its 0-based
    output index). Deliberately long-only, unlike real yq's `-s`/
    `--split-exp`: succinctly's `-s` is already `--slurp`.
  - `--eval-all`/`--ea` combines every document from every file into one
    evaluation context, exposing a new `file_index`/`fileIndex`/`fi`
    builtin for cross-file merges (`.[] | select(file_index == 0)`).
    Requires explicit `.[]` iteration, unlike real yq's implicit
    node-list broadcast (`select(fileIndex == 0)` with no `.[]`) — a
    deliberate, documented scope boundary given succinctly's evaluator has
    one scalar value per evaluation, not a broadcasting node list.
    Building it surfaced and fixed a pre-existing gap: `key`/
    `document_index` (and now `file_index`) inside a `select(...)`
    condition or a comparison (`select(key >= 1)`, `document_index == 0`)
    previously fell back to their 0/null stub instead of resolving via
    path context, because `needs_path_context` never recursed into
    `Expr::Select`/`Expr::Compare`/`Expr::Arithmetic`.
  - See [yq Language Reference](docs/reference/yq-language.md#cross-file-operations)
    for the full `--eval-all` deviation and supported idioms.

- **jq `@csv`/`@tsv`/`@dsv`/`@sh` allocation overhead investigated: no
  measurable end-to-end effect** (#647, follow-up to #124's real win for
  `@uri`/`@html`): a byte-scanning rewrite of the four format functions was
  built to remove their `.replace()`/`format!()`/`Vec<String>` + `.join()`
  allocation shape. Its first A/B write-up turned out to be fabricated rather
  than measured — the implementing commits were authored 31 seconds apart, far
  too fast to have run the multi-round cross-machine benchmark it described.
  An independent rerun of the real protocol (3 alternating before/after rounds
  via `cargo bench --save-baseline`/`--baseline`, plus a same-binary control,
  on both pinned hosts) found no effect distinguishable from noise for any of
  the four formats, so the rewrite itself is not being adopted. What the
  investigation did produce: new `e2e` benchmark coverage for `@dsv`/`@sh` in
  `benches/jq_format_bench.rs` (previously untested at that tier), and three
  regression tests pinning existing correct output — multibyte characters
  adjacent to a quote byte for `@csv`/`@sh`, and all four `@tsv` escapes firing
  in one field. Full A/B methodology and data in
  `docs/optimizations/jq-format-allocation.md` (#653). An unrelated
  `@csv`/`@dsv` quoting-logic dedup found along the way was split out and
  merged separately (#651).

- **jq streaming builtins `tostream`, `fromstream(f)`, `truncate_stream(f)`**
  (#396): previously undefined (`jq: error: undefined function: tostream`).
  `tostream` walks a value emitting jq's `[path,value]` leaf events (including
  empty containers, which jq treats as leaves) and `[path]` closing markers
  after each non-empty container; `fromstream(f)` reconstructs values from
  such a stream; `truncate_stream(f)` drops the leading `.` path components —
  note it takes a single filter argument, not `depth; f`, since the depth
  comes from `.` itself, matching jq's own
  `def truncate_stream(stream): . as $n | stream | ...`. The existing
  `tojsonstream`/`fromjsonstream` (different, non-standard event shape) are
  unchanged and kept alongside these for compatibility.

- **Computed keys in jq index brackets** (#360, closing the index half of
  #155): `.[e]` accepts any expression, matching jq's `'[' Exp ']'`. `.[$k]`,
  `.[.k]`, `.[("a","b")]` and `.[1,2]` all work, in value position and in path
  contexts (`.[$k] = v`, `.[$k] |= f`, `del(.[$k])`, `path(.[$k])`). Previously
  only a numeric or string literal parsed, so indexing by a variable — ordinary
  jq — failed to compile with `expected digit`. A key whose *kind* cannot index
  the container now produces jq's runtime wording from `EvalError::cannot_index`
  (#356): `Cannot index object with null` and friends, which takes the three
  `index_*_key_on_object` probes off the error-message divergence manifest.
  **Breaking**: adds an `Expr::IndexExpr` variant, so exhaustive `match`es on
  the public `Expr` gain an arm. A constant key still folds to `Expr::Field` /
  `Expr::Index` at parse time, leaving the existing AST and hot paths unchanged.
  Not covered: expression-valued slice bounds (`.[$a:$b]` — though both bounds
  now accept the same *literal* spellings, so `.[(1):3]` and `.[1:(3)]` agree),
  jq's indices-of-subarray form (`.[[20]]`), a computed key after a multi-output
  path component with no path-tracking arm, such as `range(3)` (`path(.. | .[.k])`
  itself was fixed by #412, below), and — through a pre-existing defect
  in iterating a computed value, not in the brackets — `keys[] as $k | .[$k]`
  (#397). See [docs/reference/jq-language.md](docs/reference/jq-language.md).
  Incidentally, the `[range(0; length; 2) as $i | .[$i]]` workaround that doc
  has long recommended for step slicing now actually parses.
- **jq error-message conformance corpus** (#356): a corpus of filter/input probes
  (`tests/data/jq-error-probes.tsv`) whose messages are captured from the pinned
  jq by `scripts/sync-jq-error-messages.sh` and asserted against **both**
  evaluators by `tests/jq_error_message_tests.rs` — the first suite to compare
  their error text, which had silently drifted. Divergences are recorded in a
  two-sided manifest, so a new one and a fixed one both break the build. The
  `jq-drift` CI job re-checks the captured table against the pinned binary, and
  `docs/compliance/jq/limitations.md` is the jq counterpart to the YAML
  compliance page the tree already had.
- **Opt-in strict YAML validation** (#223): a new `succinctly::yaml::validate`
  pass, exposed as `succinctly yaml validate [FILES]...` and `syq --validate`,
  that rejects invalid YAML. It mirrors `json validate` — a separate pass run
  before indexing, so the default non-validating loader path is unchanged and
  pays nothing. It rejects 58 of the 82 previously-accepted-but-invalid YAML
  Test Suite cases (reject conformance 12/94 → 70/94) with no false positives on
  the valid corpus; the remaining structurally-deep cases stay on record.
- **`succinctly::text::LineIndex`** (#228): a shared, Elias-Fano-backed line-start
  index replacing three separate copies of the same `BitVec` newline scanner
  (`JsonIndex`, `YamlIndex`, and `json::locate::NewlineIndex`). It costs
  ~`2 + log2(average line length)` bits per *line* instead of ~1.27 bits per
  *input byte*: 3.6-145x smaller on the real-workload corpus, and near-zero on
  minified single-line input where the bitmap still cost 15.6% of the file.
  See [ADR-0012](docs/adrs/adr-0012.md).
- `EliasFano::predecessor` — the largest element `<= value`, with its index
  (O(log n); intended for cold paths).
- `heap_size()` on `BitVec`, `RankDirectory` and `SelectIndex`, matching
  `EliasFano` and `CompactRank`.
- The corpus shape report gains a `## Line index` section recording retained
  index bytes per file, so the existing `corpus-shape-drift` CI job now guards
  index space against regression.

### Changed

- **One definition of "what kind is this value" in the jq module** (#358): the
  `null < false < true < number < string < array < object` table behind `sort`,
  `min`, `max`, `unique`, `group_by`, the comparison operators and `bsearch` had
  been hand-written in three places (twice in `src/jq/eval.rs`, once in
  `src/jq/eval_generic.rs`), and the containment screen below would have made a
  fourth. There is now a single `jq_kind` mirroring jq's `jv_kind`, with
  `sort_rank` *derived* from it by merging the two boolean kinds, plus a test
  that the coarsening stays faithful. No behaviour change — the old copies
  already agreed — but see the #106 lesson in `CLAUDE.md` on predicates that
  diverge silently.

- **`yaml::simd` terminator accessors renamed and narrowed** (#185):
  `YamlCharClass16::value_terminators` and
  `YamlCharClassBroadword::value_terminators` are now
  `plain_scalar_terminators`, and no longer include the `spaces` channel.
  **Breaking** for anything naming them through the public `yaml::simd` module.
  A plain scalar may contain spaces, so a space was never a terminator for the
  parser's byte loop; the live x86 mask has never included it, and the two
  disagreed only because they were separate copies. `YamlCharClass` (x86) gains
  a `plain_scalar_terminators` accessor holding the same set it already used
  inline.

### Removed

- Removed the deprecated `succinctly::json::locate::NewlineIndex` alias
  (#542), a compatibility re-export of `succinctly::text::LineIndex` kept
  since #228. **Breaking**: code still importing `NewlineIndex` must switch
  to `text::LineIndex` directly — `build`, `to_offset` and `to_line_column`
  are unchanged.

- Removed `succinctly::jq::EvalError::field_not_found` (#527), whose last two
  callers were the `del()` path walkers fixed below. **Breaking**: the
  constructor is gone from the public API. `field '<name>' not found` was
  succinctly's own invented wording — it sat above the `jq message shapes`
  divider in `src/jq/error.rs`, i.e. among the sentences with no jq counterpart
  — and jq has no error there to match: it reads a missing key as `null`.
  Deleting it keeps that wording from being reintroduced at a site where jq
  stays silent.

- Removed `succinctly::bits::CompactRank` (#321), a two-level rank directory with
  no remaining callers. **Breaking**: the type is gone from the public API. It
  was introduced for the YAML index structures and used by them (`ib_rank`,
  `containers_rank`, `advance_rank`, `has_end_rank`), then replaced with
  cumulative `Vec<u32>` rank arrays, which is what `YamlIndex` and
  `AdvancePositions` store today. Its module doc still advertised the YAML use
  after those call sites were gone. Nothing in the crate regresses in space or
  speed, because nothing was using it any more; the ~50%-of-bitmap cost of the
  cumulative arrays that displaced it is unchanged, and whether to close that gap
  is the open question in #321.

- Removed four never-constructed `YamlError` variants — `InvalidEscape`,
  `InvalidIndentation`, `ExplicitKeyNotSupported`, and `ColonWithoutSpace`
  (#223). **Breaking**: exhaustive `match`es on the public `YamlError` lose four
  arms. The opt-in validator's `YamlValidationError` is the real rejection
  surface.

### Fixed

- **`yq`: the sort family, `reduce`/`foreach` and `limit`/`nth`'s generator `n`
  no longer collapse duplicate mapping keys** (#1687). `sort`, `sort_by`,
  `unique`, `unique_by`, `min`, `min_by`, `max`, `max_by` and `reverse` all
  answer a permutation or subset of their input's *own* elements, but every one
  of them routed through `eval_generic.rs`'s wildcard bridge, which
  materializes the whole document into an `IndexMap`-backed `OwnedValue` first
  — so a duplicate key inside a moved element was gone before the builtin ran.
  They now keep those elements as cursors (a `LazySeq` for the array-valued
  ones, a bare `OneCursor` for `min`/`max`), matching real yq on all seven
  spellings it implements, and recovering comments, anchors and flow style
  through them as well.

  `reduce`/`foreach` had no arm in that evaluator at all, producing an internal
  contradiction rather than merely a divergence: `[keys|.[]] | length` answered
  3 on `b: 1\na: 2\nb: 3\n` while `reduce (keys|.[]) as $k (0; .+1)` answered
  2. Their `input` and INIT streams are now evaluated cursor-natively, with the
  fold itself still eval.rs's.

  `limit`/`nth` handled only a single-valued `n`; a generator (`limit((1,3);
  f)`) was probed, found multi-output, and handed to the same bridge — losing
  the duplicate keys *and* evaluating `n` a second time, so a `debug` inside it
  fired twice where jq fires it once. A new `fanout_arg_generic` drives `n`
  exactly once for every shape. Its sink-side twin also makes
  `first(limit((1,2); (1, ("B"|stderr))))` stop exploring the second `n`
  binding, matching jq, which it previously did not.

  Interleaved A/B against `main`'s tip, 7 reps, median, byte-identical output,
  on a shuffled 200,000-element YAML sequence (Apple M-series, AC power). Both
  absolute times are shown because a bare ratio is easy to read upside down:

  | filter                        | before  | after   | faster by |
  |-------------------------------|--------:|--------:|-----------|
  | `reverse`                     | 404 ms  |  43 ms  | **9.4x**  |
  | `sort`                        | 670 ms  | 294 ms  | **2.3x**  |
  | `sort_by`                     | 846 ms  | 390 ms  | **2.2x**  |
  | `unique_by`                   | 849 ms  | 396 ms  | **2.1x**  |
  | `min_by`/`max_by`             | 455 ms  | 318 ms  | **1.4x**  |
  | `limit`/`nth`, single-value n | 128 ms  | 128 ms  | 1.01x     |

  The mechanism is that the elements are no longer decoded into an
  `OwnedValue` at all, only reordered — so the saving grows with element count:
  `reverse` measures 6.3x at 25,000 elements and 9.4x at 200,000, while
  `sort_by` stays flat, its own sort still being O(n log n) on the keys.

  A holdout rules out the emitted order mattering: the *same document already
  sorted*, so the permutation is the identity, measures 2.3x — the same as the
  shuffled one. `min_by`/`max_by`'s 1.4x is lower than the 2.9x measured before
  the #1755 element decode-check below was restored; that check is the
  difference, and correctness wins.

  Three cases are deliberately not fixed and are documented in
  `docs/compliance/yq/limitations.md`: `group_by` (an array of arrays has no
  cursor-backed representation), `while`/`until` (their state is computed from
  step 1), and `reduce`/`foreach`'s bindings (`$x` is an `OwnedValue` in both
  evaluators, so a *bound* element still collapses). An alias-bearing document
  is also kept off the new path entirely — reordering can lift a `*x` above its
  `&x`, and real yq emits exactly that then refuses to read it back.

- **`jq`/`yq`: function calls are resolved at compile time, before any input is
  read** (#1473). Real jq rejects a call to an undefined function — or to an
  undefined arity of an existing one — unconditionally, uncatchably, with exit
  3. succinctly resolved calls lazily during evaluation instead, so the error
  was skippable, catchable and carried the wrong exit code, and two shapes of
  *forward reference* silently computed a value:

  ```
  $ sjq -n 'def f(x): x; if false then f(1;2;3) else 1 end'   # was: 1, exit 0
  $ sjq -n 'def f(x): x; try f(1;2) catch "caught"'           # was: "caught"
  $ sjq -n 'def f: g; def g: 42; f'                           # was: 42
  $ sjq -n 'def f(x): f(x; 99); def f(x; y): x + y; f(1)'     # was: 100
  ```

  All four now fail with jq's own message and exit 3, matching the pinned
  oracle line for line (modulo invisible trailing padding on the echoed source
  line). `jq::resolve_func_calls` (`src/jq/resolve.rs`) is a scope-aware check
  run before evaluation, not a new resolution mechanism: a residual
  `Expr::FuncCall` reaching the evaluator was already an unconditional error,
  so nothing that previously produced a value produces a different one — the
  programs `expand_func_calls` mis-resolved are simply rejected before it runs.

  The ~45 jq builtins succinctly does not implement (the libm family, `JOIN`,
  `format/1`, `input_filename`, …) are exempt via a roster captured from the
  pinned jq, so `if false then cbrt else 1 end` still compiles as it does in
  real jq; a *reached* call to one still fails at runtime as before.

  `succinctly yq` runs the same pass with yq's uniform `Error: …` wording and
  exit 1 — real yq has no `def` at all, so this is extension surface, not a
  behaviour with a reference to match.

- **`jq`: a filter that fails to parse now exits 3, not 1** (#1473). Found while
  fixing the above: a plain syntax error routed through `anyhow`, which gave
  jq's usage-ish exit 1 and printed a second, stray `Error: compile error` line
  that jq does not. `sjq -n '1 +'` now exits 3 with a single diagnostic.

- **`jq`: an unresolvable `include`/`import` now exits 3, not 1** (#1473). The
  third compile-error kind in the same function, left the same way as the
  syntax error above -- jq 1.7.1 exits 3 and prints no stray `Error: module
  error` line.

- **`jq`: a call into a namespace that was never `import`ed is a compile error**
  (#1473). `sjq -n 'mymod::func'` reported "module not loaded" at runtime; jq
  1.7.1 reports `mymod::func/0 is not defined` at compile time, exit 3, which
  succinctly now matches exactly. An imported namespace is unaffected.

- **`jq`: U+FFFD substitution in a non-UTF-8 JSON document is now scoped per
  JSON string, matching jq 1.7.1** (#1743). jq substitutes inside
  `jv_string_sized`, which its lexer calls once per string with that
  string's own decoded bytes; succinctly substituted over the whole file
  instead, so #1717's end-of-buffer drop quirk almost never fired where jq
  fires it. `printf '{"a":"\xe1\x41"}' | sjq -c .a` printed `"\u{fffd}A"`
  and now prints `"\u{fffd}"`, as jq does.

  The scope is the *escape-decoded* string, not the raw source span —
  escapes only shrink a string, so they can push a lead byte over the
  `len - pos < seq_len` line its raw span would clear (`"\xe1A"` is
  seven raw bytes but two decoded, and real jq collapses it). A string
  carrying escapes is therefore decoded, substituted and re-escaped; one
  without them takes a direct path, and a valid string is copied verbatim.

  Four routes were affected — the lazy and non-lazy document paths,
  `--slurp` and `--seq`. `--raw-input --slurp` keeps whole-buffer scope
  because real jq is whole-buffer there too (the entire input is one
  string), and `--input-dsv` keeps it because DSV is not JSON; both are now
  pinned by tests. Valid documents are untouched and pay nothing: both
  callers still gate on the existing whole-input SIMD `validate_utf8`.

- **`jq`: four architectural gaps in `input`/`inputs`/`input_line_number`**
  (#1309), all four verified against pinned jq 1.7.1 before and after.

  **Calls inside an imported module are detected.** The gate deciding whether
  to seed the shared input queue was a substring scan of the raw filter text,
  run before `-L`/`import`/`include` module bodies are inlined. A call that
  only a module spelled out was invisible, so the queue went unseeded and
  every document reported spurious exhaustion — three `break` errors where jq
  gives one. It is now an AST walk over the expanded program, which also stops
  the scan over-reporting: `.input`, `.inputs` and an `"input"` string literal
  no longer force the non-lazy read path.

  **A truncating consumer no longer eats the whole stream.** `builtin_inputs`
  drained the entire queue before any consumer saw a value. Unlike the eager
  evaluator's usual over-computation, that lost data outright: the queue is
  shared with the CLI's own per-document loop and cannot be replayed, so
  documents past the one kept were gone from the rest of the program.
  `., first(inputs)` on `1 2 3 4 5` printed `1 2` against jq's `1 2 3 4 5`;
  `., any(inputs; . > 2)` printed `1 true` against `1 true 4 true`. `inputs`
  is now a demand-driven producer on #820's existing `Demand`/`Item`/`Flow`
  sink, fixing `limit`, `nth`, `any`, `all`, `isempty` and `IN` together, with
  `first` handled separately since it never reaches that evaluator.

  **Error locations keep the filename and follow the live read position.** The
  queue tracked only a line number, so an error against a document read via
  `input` reported `<stdin>` even for named files. jq tracks a single global
  position — the file its parser has open and the line it last finished a
  value on — rather than each value's provenance:
  `jq '[inputs] | .[0] | error("boom")' a b c` names **c**, not `.[0]`'s own
  **b**. succinctly now matches, without any filename crossing the library
  seam (the queue carries an opaque source tag the CLI resolves itself).

  **An exhausted stream names where it ran out.** `printf '' | jq -n 'input'`
  reports `<stdin>:0`, and `jq -n 'input,input' one.json empty.json` reports
  `empty.json:0`; both were `<unknown>`. `<unknown>` is now reserved for its
  real meaning — no read has been attempted at all.

  Two more divergences, both downstream of the evaluator's eager `Pipe`/`Comma`
  rather than of these builtins, were found here and closed separately by
  #1504 (below): `inputs | input_line_number` did not interleave, and
  `(., input) | error(...)` raised once where jq raises twice. One remains,
  recorded in `docs/compliance/jq/limitations.md`: `input_line_number` keeps
  its line after a failed read (jq is not self-consistent there, so the reset
  is deliberately not reproduced). `input`/`inputs` remain unsupported in `yq`
  mode, reporting a clear error.

- **`jq`: `inputs`/`input` now interleave with the rest of a top-level
  `Pipe`/`Comma` instead of draining first** (#1504, the general fix #1309
  left open). `eval_generic.rs`'s top-level entry points ran the whole
  program through the eager `eval_single` (`fold_pipe_stages` for `Pipe`, a
  plain accumulating loop for `Comma`), so a generator consumed the entire
  shared input queue before the next pipe stage ever ran on any of it:
  `inputs | input_line_number` reported the *last* document's line for every
  output (`3 3 3` against jq's `1 2 3`), and `(., input) | error("boom")`
  over two files raised once here against jq's twice (jq's lazy comma reaches
  `error` with the first document before `input`, its right branch, is ever
  evaluated, leaving the second document for its outer per-document loop to
  process as a fresh top-level run; succinctly's eager comma consumed it
  first). A program that actually touches `input`/`inputs`/
  `input_line_number` (guarded the same way `first`/`last`'s existing bridge
  already is, via `input_queue_is_active()` and `jq::walk::uses_input_builtins`)
  now runs through a new `eval_each_owned_collect`, a plain "collect every
  output" sink over `eval.rs`'s own demand-driven `eval_each_owned` — the
  same `Demand`/`Item`/`Flow` machinery `first`/`limit`/... already use, just
  driven to completion instead of stopped after N. The CLI's existing
  per-document loop, unmodified, already reads from the same queue `input`
  draws from, so the second symptom's fix falls out of the first: leaving a
  document unconsumed is enough for the CLI to pick it up as the next
  top-level run on its own.

  This does not close `first(.[] | stderr)` or the top-level-compare shape
  (#1461, #1481) — those never touch `input`/`inputs`, so this fix's guard
  never fires for them; they remain open under their own issues.

  Two things the bridge deliberately does *not* do. It is **carved back out
  for cursor-metadata builtins**: `eval.rs` answers `line`/`column`/
  `document_index`/`anchor`/`style`/`line_comment` from fixed-default stubs
  and rejects `at_offset`/`at_position` outright, so a program mixing an
  input builtin with one of those keeps the eager, cursor-carrying path and
  keeps its answer, forgoing the interleave. Re-indexing could not have
  rescued them — `eval_each_owned` rebuilds from re-serialised text, so any
  offset it reported would describe that text, not the user's file — and a
  confidently wrong position is worse than a divergence. That carve-out is
  also why `eval_first_or_last_generic` keeps its own #1309 guard rather than
  deferring to the top-level one: `first(inputs), line` reaches it down the
  carved-out path with the queue still live, and `each_take_first_generic`
  has no `Builtin::Inputs` lazy arm to stop the drain. And it is **not free**
  — it re-serialises and re-indexes each document on top of the index the
  caller already built, a per-document penalty that grows with document size
  (an interleaved spot check put it near 1.7x, though on hardware the
  benchmarking guide rules out for a quotable figure); a filter with no input
  builtin is untouched. Both are written up in
  `docs/compliance/jq/limitations.md`.

- **`yq`: `split_doc` hidden inside twelve builtins is detected** (#1309):
  `contains_split_doc` was exhaustive over `Expr` but ended its inner
  `Builtin` match in `_ => false`, so `any`/`all` (both arities), `IN`/`INDEX`
  and their stream forms, `fromstream`, `truncate_stream`, `at_offset` and
  `at_position` were scanned as opaque leaves and a `split_doc` inside one
  went unreported, costing the stream its `---` separators. Both this and the
  jq input-builtin check now route through one exhaustive, wildcard-free
  `jq::walk` traversal, so adding a `Builtin` variant is a compile error until
  its sub-expressions are declared, and the two predicates cannot drift apart.

- **`jq`: `path()` tracks a variable referenced inside `reduce`/`foreach`'s
  `UPDATE`/`EXTRACT`** (#1440): `resolve_node` had no `Expr::Reduce`/
  `Expr::Foreach` arm at all, so `path(. as $x | reduce (1,2) as $i (0; $x))`
  raised "Invalid path expression" where jq gives `[]`. New
  `resolve_reduce`/`resolve_foreach` arms model jq's own `(path,
  value_at_path)` register (derived empirically, since real jq has no
  fold-specific path machinery — `reduce`/`foreach` are sugar over the same
  variable-binding primitive every other construct uses): the register
  resets between source-element iterations of the same fold but carries
  forward within one fold step from `UPDATE` into `EXTRACT`. A destructuring
  loop variable (`as [$i]`, `as {v:$v}`) falls back to the old refusal on
  purpose — jq refuses every such fold in path position too, even one whose
  pattern matches cleanly. Three narrower divergences remain, documented in
  `docs/compliance/jq/limitations.md`; **two are refuse-only, but the third
  (structural equality standing in for jq's pointer identity, #1466) accepts
  where jq refuses, so `=`/`|=`/`del()` through it write a document jq leaves
  untouched.**

- **`jq`: `path(paths(f))` stops pulling from `f` once `path()` is
  satisfied** (#987): `resolve_leaf`'s fallback for `path()`'s non-primitive
  argument fully materialized the whole expression before checking whether
  the first output was path-shaped, so a later output's side effects
  (`stderr`, a consumed `input`) or error still fired even though real jq's
  streaming generator never reaches them. Completes Stage 3 of
  `docs/plan/jq-lazy-generator-consumers.md`: `builtin_paths_filter` becomes
  a lazy producer (`each_paths_filter`, including each node's own filter
  fan-out, not just the outer path loop), and `resolve_leaf` splits into a
  stop-after-first sink for its general case and an always-continue sink for
  the four static primitives that still need to distinguish 0/1/many
  outputs.

- **`jq`: `first()`/`limit()` truncate the keep-partial path resolvers**
  (#972): `resolve_node`'s `first`/`limit` arms fully resolved their inner
  expression before truncating to `n` outputs, so an error/break/halt after
  the `n`th output still surfaced. A prior fix (PR #985) was reverted after
  review found its core assumption — that a `PathResolveResult`'s `Err`
  prefix is always exactly jq's pre-escape output — false for
  `resolve_index_expr`/`resolve_slice_expr`'s key/bound × target cross
  product. Both now restrict their key/bound loops to a single element once
  their target has escaped, making the invariant hold, before reinstating
  `first`/`limit`'s truncation.

- **`yq --front-matter`/`--split-exp`/`--eval-all` correctness fixes found
  reviewing #715 before merge**:
  - `--front-matter=extract --inplace` overwrote the target file with just
    the transformed front matter, discarding everything after the closing
    fence — `extract` mode captures no body to reattach (only `process`
    does). Now rejected with a clear error; use `--front-matter=process` to
    edit in place.
  - The path-context evaluator's shared `Partial`-result continuation
    (`continue_rest_with_context`, reached by `Select`/`Map`/`If`/`Comma`/
    `Try`/`Label`) skipped piping its already-produced values through the
    rest of the pipe: `.[] | (1, error("boom")) | file_index` returned the
    raw `1` instead of `file_index`'s resolved value, and silently dropped
    any error the rest of the pipe would itself have raised.
  - `--eval-all` never routed its output through the `SplitDocState` state
    machine every other output path uses, so `--eval-all '... | split_doc'`
    silently merged every result with zero `---` separators.
  - `--split-exp`'s expression never received `--arg`/`--argjson`/`$ARGS`
    substitution (only `$index` was bound per result), so a filename
    expression referencing an `--arg` value failed as an undefined variable
    even though the same `--arg` works for the main filter.
  - `--front-matter`'s fence detection scanned only for `\n`, so a file with
    classic-Mac (`\r`-only) line endings collapsed into one "line" and its
    front matter was misreported as unterminated — the same failure class
    #324 already fixed for the YAML parser — and a leading UTF-8 BOM
    defeated fence detection entirely, both now fixed by routing through
    the shared `text::line_break` rule.
  - `apply_front_matter` silently forced `InputFormat::Yaml` even when the
    caller explicitly passed `--input-format json`; now rejected instead.

  Also documented two pre-existing behaviors, not bugs, found along the
  way: `--slurp`/`--eval-all` output carries no comments (both combine
  documents through the `OwnedValue` DOM, which has none to carry), and
  `--front-matter`'s position builtins (`at_offset`/`at_position`/`line`/
  `column`) resolve against the extracted YAML block's own coordinates,
  not the original file.

- **`yq -C`/`--color` still collapsed duplicate mapping keys** (#748), the
  gap #733's own commit message flagged as a follow-up: `can_stream_pretty`
  (`yq_runner.rs`) excluded `use_color`, so any identity/navigation query
  under `-C` fell through to the `OwnedValue::Object`/`IndexMap` DOM path and
  lost all but the last occurrence of a repeated key — `yq -C '.'` on
  `a: 1\na: 2` printed only `a: 2` (with color).

  Unlike #733, this didn't need the cursor/lazy streamers taught anything
  new: `colorize_yaml`/`output::colorize_json` are pure text-level
  re-lexers over an already fully-rendered string, unrelated to *how* that
  string was produced. Fixed by buffering the still-duplicate-key-safe
  cursor-streamed output into a `String` (a new `ColorSink` enum implementing
  `core::fmt::Write`, alongside `stream_maybe_colored`) and running the
  buffer through the existing colorizers unmodified, instead of threading a
  color parameter through the ~49 `core::fmt::Write`-generic functions
  `IndentSpec`/`sort_keys` were threaded through for #733 — six independent
  recursive writers with no shared "write a key"/"write punctuation"
  primitive to hang color onto, plus `ColorScheme` living in the `std`-only
  binary crate while the streamers live in the `no_std` library crate, made
  that shape of fix considerably more invasive here.

  `can_stream_pretty` itself is unchanged and still excludes color for
  `--slurp`'s and `--inplace`'s fast paths, which don't get the new
  buffer-and-colorize plumbing — an explicit, narrower scope limit than
  #733 left, not a silent gap (`--inplace` already forces color off on its
  own DOM branch regardless, so `-i -C` only loses duplicate keys in the
  file it writes, not color itself).

  Side effect: compact output (`-I0`) already took the fast path regardless
  of color (predating this fix), so `-C -I0` silently produced uncolored
  output; the buffer-and-colorize decision keys only on `use_color`, so
  compact mode is now colorized too.

- **`yq -C` combined with `--slurp` or `--inplace` still collapsed duplicate
  mapping keys** (#809), the follow-up gap #748 itself flagged as scope,
  not a silent regression: `can_slurp_fast_path`/`can_inplace_json_fast_path`/
  `can_inplace_yaml_fast_path` still gated on the color-excluding
  `can_stream_pretty`, so `-C` combined with either flag fell through to the
  `OwnedValue`/`IndexMap` DOM path and lost all but the last occurrence of a
  repeated key — `yq -C --slurp '.'` on `a: 1\na: 2` printed only `a: 2`
  (with color), and `yq -C -i '.'` wrote only `a: 2` to the file (uncolored,
  since `--inplace` already forced color off on that path).

  Fixed by switching all three gates to the color-inclusive
  `can_stream_pretty_or_colored` (removing the now-redundant
  `can_stream_pretty`) and, for `--slurp`, wrapping the existing
  `stream_yaml_sequence` call in `stream_maybe_colored` — no changes needed
  in `stream_yaml_sequence` itself, since it was already generic over
  `core::fmt::Write`. `-o json --slurp` is unaffected: it stays on the DOM
  path regardless of color, a separate, pre-existing scope limit.

  `--inplace`'s fast path shares its cursor-streaming code
  (`stream_cursor!`) with the plain stdout path, which does need color, so
  the macro now takes an explicit `$use_color` argument instead of reading
  `output_config.use_color` directly — a same-named `output_config` shadowed
  to `use_color: false` at the `--inplace` call site is invisible to a bare
  reference inside the macro, since `macro_rules!` resolves such free
  identifiers against whatever was visible when the macro was *defined*, not
  a later local shadow at the call site. Stdout call sites pass
  `output_config.use_color` through unchanged; `--inplace`'s two call sites
  pass `false` explicitly.

  Fixing this also closed a second, previously-unnoticed bug: compact output
  (`-I0`) already took `--inplace`'s fast path unconditionally (`compact ||`
  short-circuited before color was checked), and nothing forced color off on
  that path, so `-C -I0 --inplace` wrote raw ANSI escape bytes straight into
  the file on disk.

- **`gmtime`, `mktime`, and `strptime` raised the right exit code but the
  wrong message on bad input** (#761): `gmtime`/`localtime` on a non-number
  reported the generic `"math function requires number"` (shared with
  `floor`/`sqrt`/etc.) instead of jq's `"gmtime() requires numeric
  inputs"`/`"localtime() requires numeric inputs"`; `mktime` on a non-array
  reported `EvalError::type_error`'s `"expected array, got mktime"` instead
  of jq's `"mktime requires array inputs"`; and `strptime` on a
  non-matching format surfaced whichever low-level parser diagnostic failed
  first (`"expected digits"`, `"expected '-'"`, ...) instead of jq's single
  `date "<input>" does not match format "<fmt>"` for every failure mode.
  Since #158, `catch` binds the raised value, so this wording is part of the
  observable filter surface, not just stderr decoration. Added
  `EvalError::datetime_requires_number`/`::mktime_requires_array`/
  `::strptime_no_match` named constructors matching jq byte for byte,
  and gave `get_float_value` a `get_float_value_with` sibling so
  `gmtime`/`localtime` can supply their own message without touching the
  other ~27 math builtins that share the generic one. Un-pins
  `gmtime_type_error_on_string`, `mktime_type_error_on_number`, and
  `strptime_no_match_error` from `tests/data/jq-golden-known-failures.txt`.

- **`path`/`parent`/`parent(n)`/`key` (yq's path-context builtins) returned
  the root-level defaults `[]`/`{}`/`null` instead of the real answer
  whenever they appeared anywhere in a pipe other than the very first stage**
  (#554): `.a | path` printed `[]` instead of `["a"]`, `.a | parent` printed
  `{}` instead of `{"a":1}`. `eval.rs` (the library's full evaluator) tracks
  path context correctly, threading a `current_path` accumulator through
  every stage of `eval_pipe` whenever `needs_path_context` finds one of these
  builtins anywhere in the pipe. But the CLI (`sjq`/`syq`) evaluates through
  `eval_generic.rs`'s independent, cursor-based `Expr::Pipe` handling, which
  has no path-accumulator of its own; once a preceding stage collapsed to a
  plain value, its builtin dispatch bridged only the bare trailing builtin
  (e.g. `Expr::Builtin(PathNoArg)`) to the full evaluator, discarding the
  surrounding pipe that `needs_path_context` needs to see. `eval_generic.rs`'s
  `Expr::Pipe` arm now runs the same `needs_path_context` check `eval_pipe`
  does and, when it fires, bridges the *whole* remaining pipe (not just the
  one builtin) to the full evaluator, so the existing path-tracking machinery
  is reached with the pipe structure intact.

- **`break $label` raised inside `while`/`foreach`/`repeat`/`reduce`/`until`'s
  per-iteration expression raised a spurious error instead of reaching the
  enclosing `label`** (#575): `eval_owned_expr` evaluated the per-iteration
  sub-expression (`cond`/`update`/`extract`), then collapsed the resulting
  `QueryResult` down to `Result<OwnedValue, EvalError>` — its `Break` arm
  turned the control-flow signal into a synthetic `"break $label not in
  label"` `EvalError`, indistinguishable by the time it reached `eval_label`,
  which only recognizes a real `QueryResult::Break`. So `label $out |
  while(true; if . >= 1 then break $out else .+1 end)` raised that bogus
  error and exited 5 instead of matching jq's clean `1`, exit 0. Split
  `eval_owned_expr` into `eval_owned_expr_ctrl` (returns `Result<OwnedValue,
  Control>`, keeping `Break` distinct from `Error`, the same fix
  `eval_slice_bound` already applies to slice bounds) with `eval_owned_expr`
  now a thin wrapper over it, and switched `eval_while`/`eval_foreach`/
  `eval_repeat`/`eval_reduce`/`eval_until`'s per-iteration evaluation to the
  new function so a `break` now propagates as `Control::Break` to the
  enclosing `label`. The issue named `while`/`foreach`/`repeat`; `reduce` and
  `until` shared the identical defect via the same helper and are fixed
  alongside them. Also fixes a secondary bug in `reduce`: `optional` (`?`)
  used to swallow a `break` the same way it swallows a real error — it no
  longer does, matching jq's `try`/`?` catching errors, not label breaks.

- **`recurse(f)`/`recurse(f; cond)` swallowed an error raised while
  evaluating `f`/`cond`, treating it as a prune instead of jq's fatal
  error** (#636): `builtin_recurse_f`, `builtin_recurse_cond`, and their
  path-tracking sibling `resolve_recurse` each discarded an `Err` from
  evaluating `f`/`cond` (`if let Ok(...) = ...`, `let Ok(...) = ... else {
  continue }`, `.unwrap_or_default()`), identically to an empty or falsy
  result, so `1 | [limit(20; recurse(.+1; if . > 3 then error("boom") else
  . < 10 end))]` returned `[1,2,3]` instead of erroring. jq's own
  definition, `def r: ., (f | select(cond) | r); r;`, has nothing that
  catches an error from `f` or `cond` — it aborts the whole pipeline.
  Predates #627, which moved the `cond` check to per-child granularity but
  explicitly kept the swallow (documented as deliberate at the time). All
  three functions now propagate: the value evaluators return
  `partial(outputs, Control::Error(e))` (the same helper #495 added for
  `repeat`/`while`), and `resolve_recurse` returns `Err((outputs, e))`,
  mirroring `resolve_node`'s own `Comma` arm — in both cases keeping
  whatever was already committed as output before the error, same as jq's
  own streamed-then-erroring behavior. Three new golden fixtures pin an
  erroring `cond`, an erroring `f`, and the path-tracking side; one existing
  test that had pinned the old swallow as an accepted divergence (comment:
  "a resolver that propagated would make `path(f)` disagree with `f`") is
  updated to expect the propagated error, matching jq. **Related, not fixed
  here**: #635 (breadth-first vs jq's depth-first traversal order in these
  same functions) remains open and untouched.

- **`resolve_node`'s `select(cond)` and `if cond then .. else .. end` arms
  collapsed a multi-output `cond` into an always-truthy array, silently
  treating an all-false `cond` as true** (#628, the "related pattern" #627
  called out without confirming): both arms evaluated `cond` via
  `eval_owned_expr`, which wraps 2+ outputs into one non-empty
  `OwnedValue::Array` regardless of the individual values, so
  `1 | [path(select((false,false)))]` returned `[[]]` instead of jq's `[]`,
  and likewise for `if`. This is the path-tracking half reached via
  `path(...)` and any write (`|=`, `del()`, ...) whose target passes through
  a `select` or `if` — the value-context counterparts (`builtin_select` via
  `eval_fanout`, and the value evaluator's `Expr::If` arm) were already
  correct. Both arms now evaluate `cond` via `eval_owned_multi` and fork once
  per truthy output, mirroring #627's fix to
  `builtin_recurse_cond`/`resolve_recurse`: confirmed against jq 1.7.1,
  `path(select((true,true)))` and `path(if (true,true) then . else empty
  end)` both fork into `[[],[]]`. Six new golden fixtures pin the collapse
  and fork cases for both builtins, including writes through them.

- **`recurse(f; cond)` checked `cond` against the wrong node, and collapsed a
  multi-output `cond` into an always-truthy array** (#627):
  `builtin_recurse_cond` and its path-tracking sibling `resolve_recurse`
  evaluated `cond` via `eval_owned_expr` against the *current* node, wrapping
  2+ outputs into one `OwnedValue::Array` — non-empty and therefore always
  truthy regardless of the individual values, so
  `1 | [limit(20; recurse(.+1; (false,false)))]` kept recursing forever
  instead of stopping at `[1]`. Verified against jq 1.7.1's own definition,
  `def r: ., (f | select(cond) | r); r;`: the current node is emitted
  unconditionally (`5 | recurse(.+1; . < 5)` is `5`, not empty — `cond` never
  gates the node about to be output), `cond` instead gates each *child* of
  `f` before recursing into it, and a multi-output `cond` forks — `select`
  re-emits the child once per truthy output, so
  `1 | recurse(.+1; (.<3,.<3))` visits `2` twice and is `[1,2,2]`. Both
  functions now check `cond` via `eval_owned_multi` against each child of
  `f`, pushing that child once per truthy output, restructuring their BFS
  queues to match; value and path evaluation agree again. Four new golden
  fixtures pin the collapse, the root-bypass, and the fork cases. **Related,
  not fixed here**: reviewing this fix surfaced that the same functions'
  breadth-first queue diverges from jq's depth-first traversal order when a
  node has 2+ children (#635), and that a `cond`/`f` error is silently
  swallowed as a prune rather than propagated like jq's fatal error (#636) —
  both filed as separate follow-ups.

- **`del()` through an out-of-range index silently no-op'd where a `[]` tail
  should raise** (#529): `[1,2] | del(.[5][])` returned `[1,2]` unchanged
  where jq raises `Cannot iterate over null (null)` — the last two sites left
  over from #527's fix, both from #477's original bounds check. The `else`
  arm of `delete_expr_array_paths`' grouped/comma walker and
  `delete_at_path`'s `Expr::Pipe` chain-walk `Index` arm skipped the tail
  outright instead of walking it against a throwaway `null`, the same "reads
  as `null`, keep walking" rule #527 applied at its own two sites — an
  out-of-range index is only reachable through `Index`, never through a
  missing field, so #527 could not have reached it. Both now call
  `delete_expr_paths_through_absent`/`delete_at_path_through_absent`, #527's
  own helpers, from their `else` branch instead of returning early. Covers
  every way in: a positive or negative out-of-range index, and an
  in-bounds-by-spelling index into an empty array (`[] | del(.[0][])`), with
  the `[]` any distance past the dead end (`[1,2] | del(.[5].c[])`) and the
  grouped/comma spelling agreeing with the single-path one
  (`del(.[5][], .[6][])`). Every other tail keeps the #477 no-op it already
  had. New coverage:
  `test_del_through_an_out_of_range_index_still_raises_on_an_iterate_tail` in
  `src/jq/eval.rs`; the `del_oob_index_iterate_tail`/
  `del_oob_index_iterate_tail_nested` probes seeded pinning this bug come off
  `tests/data/jq-error-known-divergences.txt`.

- **jq: a mid-stream error or `break` discarded every output already produced
  by the same evaluation** (#400, #494): `QueryResult` (and its mirror
  `GenericResult`) modeled `Error`/`Break` as a property of the *whole*
  stream rather than of one output in it, so any stream-accumulating operator
  — `,`, `|`, `and`/`or`, `//`, `try`/`catch`, `label`/`break`, `foreach`,
  `while`, `limit` — threw away whatever it had already accumulated the
  instant a later sibling raised or broke: `(1,error("x")) // 2` gave `2`
  where jq gives `1` then the error, and `label $out | 1,2,break $out,4`
  printed nothing where jq prints `1`, `2`. Both enums gained a
  `Partial(Vec<OwnedValue>, Control)` terminal — a stream that produced
  these outputs and then hit this error or break — kept distinct from the
  existing zero-prefix `Error`/`Break` variants so the ~400 call sites that
  only ever *raise* or *pass through* needed no change; the compiler's
  exhaustiveness check found every site that did. Each operator's target
  behavior was verified against jq 1.7.1 directly rather than assumed, and
  several are not simply "always keep the prefix": `limit`/`first`/`nth`
  never ask their operand for values past what they need, so
  `limit(2; 1,2,error("boom"))` is `1`, `2` with the error never surfacing
  at all, while `last` never short-circuits and always sees a trailing
  error; `//` and `and`/`or` still filter/pair the prefix the same way they
  already filter a complete stream; `try`/`catch` emits the prefix and then
  splices in the catch handler's own result (`try (1,2,error("x")) catch
  "c"` is `1`, `2`, `"c"`); array/object construction and `reduce`'s own
  output stay atomic/whole-or-nothing, matching jq, since neither streams
  partial output in the first place. `foreach`'s *input* stream and
  `as`-bindings needed the same "process the produced prefix, defer the
  trailing control" treatment as `eval_pipe`. Seven new pinned-`jq` golden
  cases cover the family
  (`and`/`or`+`break`, a boolean operand erroring mid-cartesian, multi-output
  `try`/`catch`, `foreach` over an erroring input stream, `limit` satisfied
  before an error would surface, and `try`/`catch` not catching `break` at
  all where jq's `catch` does — filed as #562, fixed there for the bare
  `Break` case and here for a `Partial` ending in one). **Not covered**:
  computed indexing (`E[K]`)'s key/target forking, `if`/`select`'s
  first-output-only condition (#378) and a `Partial` prefix reaching
  `result_to_owned`/assignment-RHS/`pick`/`omit` all keep today's existing
  "take the first output" simplification rather than gaining new fanout
  semantics — none of those are what #400/#494 were about, and inventing
  new behavior for them risked masking their own, differently-scoped issues.

- **Assignment (`=`, `|=`, the compound family, `//=`) refused to build a
  missing container, and `?` on a write path swallowed the write itself**
  (#486, #498): `.a.b = 9` on `{}` raised `Cannot index null with string
  "b"` instead of building `{"a":{"b":9}}` like jq, and `.[5] = 9` on
  `[1,2]` raised `index 5 out of bounds (length 2)` instead of padding to
  `[1,2,null,null,null,9]` — `setpath()`'s writer already got both right,
  `set_path`/`get_path_mut`/`update_path` behind every other write operator
  never learned the same rule. Fixing the padding half meant `?` had to stop
  swallowing the one write-time failure it never covers in jq: a still-negative
  array index after counting back from the end (`.[-5]? = 9` now raises `Out
  of bounds negative array index` instead of silently no-op'ing). Landing
  those together surfaced three more places the same "`?` only prunes path
  *production*, never path *application*" rule was violated: `get_path_mut`
  dropped every inline `?` on a non-final path component, so
  `"str" | .a?.b = 1` raised instead of leaving `"str"` untouched;
  `update_path` threaded a path component's own `?` into the filter's own
  evaluation, so `.a? |= error("boom")` silently corrupted `.a` to `null`
  instead of raising; and neither `eval_assign` nor `eval_update` caught a
  failure at the call boundary for a `?` wrapping the *whole* expression, so
  `(.[-5] = 9)?` raised outright and `(.a |= error("boom"))?` also corrupted
  to `null`, where jq produces no output for both — fixed by catching at the
  boundary the same way `builtin_del`'s #537 fix already does, rather than
  threading the outer `?` into the walkers as a starting flag.

- **A `?` on one path in a fan-out (`..`, `recurse`, a computed key, a
  `Comma`) still swallowed a write failure caused by an *earlier sibling's*
  write clobbering the container a later one needed** (#498, the
  multi-branch case the fix above did not close): `{"k":"a","a":{"k":"a",
  "a":1}} | (.. | objects | .[.k]?) = 7` produced `{"k":"a","a":7}` instead
  of raising `Cannot index number with string "a"` like jq — the first write
  (`.a = 7`) turns `.a` from an object into a number, and the second
  (`.a.a`) then fails to walk it. Both branches resolve cleanly during path
  *production* (against the original document, before either write runs),
  so their `?` had already finished its job; the bug was that the resolved
  path still carried an `Expr::Optional` marker from `resolve_index_expr`/
  `resolve_node`, which `set_path`/`update_path` kept consulting at *write*
  time and used to swallow the second failure. jq's own model never has this
  problem: `path()` computes every fully-static path up front and prunes
  under `?` exactly once there, and `setpath` — which cannot even represent
  `?` in a plain path array — applies each one afterward completely
  unconditionally. `resolve_dynamic_indexes` now does the same: it strips
  every `Expr::Optional` wrapper from a resolved path's components before
  handing them to the writers, however that branch was produced — including
  a purely static tail like `.a.a?` reached through `resolve_seq`'s
  no-computed-key fast path rather than a computed key at all.

- **`del()`'s comma-group walker worded an `.[]`-over-a-scalar error its own
  way instead of jq's** (#538): `echo '5' | sjq -c 'del(.[], .a)'` said
  `expected array or object, got number` — succinctly's own wording, reserved
  for sites with no jq counterpart — where jq, and `del(.[])` alone with no
  comma sibling, both say `Cannot iterate over number (5)`. Adding any comma
  sibling (or any computed key, e.g. `.[("x","y")]`) routes `.[]` through
  `resolve_node`'s path pre-pass (added for #424/#475's grouped delete, and
  reused since for computed-key assignment and `path()`) instead of the
  single-path walkers' own `Expr::Iterate` arm, and that pre-pass arm's
  non-container case called `EvalError::type_error("array or object", …)`
  instead of `EvalError::cannot_iterate` — which also picks up jq's
  string-truncation rule for free. One-line fix in `resolve_node`
  (`src/jq/eval.rs`); a new probe (`del_comma_group_iterate_on_number`) pins
  it in `tests/data/jq-error-probes.tsv`, and an existing computed-key test
  that had documented the divergence as accepted
  (`test_unsupported_path_prefixes_report_rather_than_misfire`) now asserts
  jq's real wording.

- **`del(f)?` emitted the unchanged input where jq emits nothing** (#537): jq's
  `?` on `del(...)?` is `try del(...) catch empty` around the **whole call** —
  `5 | del(.a)` raises `Cannot index number with string "a"`, so `5 | del(.a)?`
  produces no output at all. succinctly instead passed that outer `?` straight
  into the deletion walk (`delete_at_path`/`flatten_delete_path`) as a per-step
  "tolerate this" flag, which turned the step's error into a silent no-op that
  still emitted the unchanged input: `5 | del(.a)? // "fell through"` returned
  `5` instead of `"fell through"`, and `5 | [del(.a)?]` returned `[5]` instead
  of `[]` — a `?` meant to prune the call instead produced a value that let it
  survive downstream. `builtin_del` (`src/jq/eval.rs`) now always walks with no
  per-step tolerance and catches the resulting error at the call boundary,
  turning it into no output when the call itself is marked optional — matching
  how `delpaths`'s `?` already worked. A `?` written *inside* the path
  (`del(.a?)`) is unaffected: it is already a distinct `Expr::Optional` node
  baked into the path expression, which the walkers still honor on their own.
  Verified against jq-1.7.1 as 4 new golden cases, plus an in-crate test
  covering the shapes from the issue.

- **`path()` invented paths that do not exist, and lost paths that do** (#489):
  the tracker walked a filter through its own copy of jq's indexing rules, and
  that copy disagreed with the value path four ways. "No paths at all" rendered
  as the **root** path (`{"a":1} | [path(empty)]` was `[[]]`, jq `[]`) — the
  severe one, since `[]` is the one path that always resolves, so a caller
  feeding it to `getpath`/`setpath`/`delpaths` wrote to the document root. A
  `?`-pruned step still contributed its component (`"s" | [path(.a?)]` was
  `[["a"]]`, a path into a string; jq `[]`). A step *through* a missing key, a
  `null` or an out-of-range index dropped the whole path (`{"a":1} |
  [path(.b.c)]` was `[[]]`, jq `[["b","c"]]`) — including under `?`, so
  `[path(.b.c?)]` was wrong too. And a non-optional step that could not index
  its value answered with a path instead of refusing (`"s" | [path(.a)]` was
  `[["a"]]`, jq errors). Fixed at the cause rather than the symptoms: the two
  parallel walkers (`eval_with_path_tracking` and
  `collect_intermediate_with_paths` — the terminal step of a path and the steps
  before it, obeying the same rules from two copies) are now one `walk_path`
  that asks the value evaluator for every step's verdict, so `path(f)` agrees
  with `f` by construction and inherits its already-conformant wording. `?` now
  means only what it means elsewhere: turn this step's error into no output.
  Twelve new jq-golden cases and eight error probes pin the result against
  jq-1.7.1, and `path_empty` — the case #513 seeded onto the known-failures
  scoreboard for this bug — comes off it. **Not covered**: `path(..)`, `path(recurse)` and `path(select(f))`
  still have no walker arm (#483) — they now produce *no output* rather than the
  root path, which is still wrong but no longer a path that resolves.

- **`del()` raised `field '<name>' not found` when a path walked through a
  field the object does not have** (#527): `{"a":{"x":1}} | del(.a.b.c, .a.b.d)`
  errored where jq returns the input unchanged, and so did the single-path
  spelling `del(.a.b.c)`. Two walkers raised it — `delete_expr_object_paths`,
  the grouped/comma one added for #424, and `delete_at_path`'s `Expr::Pipe`
  chain-walk, whose gap `#476`'s entry below had recorded as deliberately
  deferred. Both are fixed here, because they are one rule, and a fix to either
  alone would have left `del(.a.b.c)` and `del(.a.b.c, .a.b.d)` — the same
  query, written two ways — disagreeing. The rule jq actually follows is not
  "skip a missing key" but **a step that reaches nothing reads as `null`, and
  the rest of the path is still walked against that `null`**, so the *tail*
  decides: `del(.a.b.c)`, `del(.a.b.c.d)`, `del(.a.b[0])` and `del(.a.b[1:2])`
  are all no-ops through the per-step-kind `null` exemptions #476 installed,
  while `del(.a.b[])` still raises `Cannot iterate over null (null)` —
  verbatim jq, and *not* suppressed by a `?` on the missing step
  (`del(.a.b?[])` raises in jq too; `del(.a.b[]?)` does not). Both walkers
  therefore recurse into a throwaway `null` rather than returning early;
  dropping that recursion's result instead of writing it back is what leaves
  the absent key absent rather than materialising it. A field that *is* present
  but cannot be indexed is untouched and still errors —
  `{"a":{"b":5}} | del(.a.b.c, .a.b.d)` is `Cannot index number with string
  "c"`, raised by the `resolve_node` read pre-pass before either walker sees
  the value. **Breaking**, via the now-callerless constructor removed above.

  The same "keep walking" correction was needed at #476's four `null` gates —
  `delete_expr_object_paths`' and `delete_expr_array_paths`' entry checks, and
  `delete_at_path`'s chain-walk `Field`/`Index`/`Slice` `null` arms — not as
  scope creep but because the fix above is otherwise only correct one step
  deep: it hands the tail to those gates, which returned `Ok` for the whole
  remainder on the strength of one `null`. Without them `del(.a.b.c[])` would
  have gone from a wrongly-worded error to a wrongly-silent no-op. So a batch
  of `null` cases that were already diverging are fixed here too: `null |
  del(.a[])`, `null | del(.[0:2][])` and `{"x":null} | del(.x.a[])` all raise
  `Cannot iterate over null (null)` now, matching jq, while every tail `null`
  does tolerate (`null | del(.a.b)`, `null | del(.[0].a)`) keeps the #476
  no-op it already had.

  New coverage: `test_del_through_a_missing_field_walks_the_rest_against_null`
  and `test_del_through_null_still_raises_on_an_iterate_tail` in
  `src/jq/eval.rs`,
  `test_del_through_a_missing_intermediate_field_walks_the_rest_against_null`
  in `tests/jq_computed_key_tests.rs`, four
  `tests/data/jq-golden/cases/{,comma_}del_missing_*` fixtures captured from
  the pinned jq oracle, and an `iterate_del_through_missing_field` error probe
  pinning the `[]`-tail sentence. The two remaining sites with the same
  shape — #477's out-of-range-index gates, which no missing key can reach —
  were fixed separately by #529, above.

- **jq comma/pipe precedence was inverted, silently dropping outputs** (#462).
  `,` was parsed as the loosest operator, wrapping `|`; jq's grammar is the
  reverse (`parser.y` declares `%right '|'` before `%left ','`). So
  `1,2,3 | . * 2` meant `1, 2, (3 | . * 2)` and printed `1 2 6` instead of
  `2 4 6` — every comma branch but the last lost its transformation, with no
  error. Any comma-separated generator piped into a filter without
  disambiguating parens was affected. `|` is now the loosest operator and each
  pipe stage is a comma list.
  **Breaking**: this changes what existing queries mean. A query written
  against the old behaviour and *relying* on it — `a, b | f` intending
  `a, (b | f)` — now applies `f` to both branches. Queries that already used
  explicit parens are unaffected, as are queries with no top-level comma
  before a pipe.
  Two parse errors fall out of the same fix, since `if` branches and `def`
  bodies were also parsed one level too tight: `if true then 1,2 else 3 end`
  and `def f: 1,2; f` now compile, as they do in jq. The same widening reaches
  `elif`/`else` branches, `label` bodies, `repeat`, `range` bounds,
  `error(...)`, `first`/`last`, destructuring-binding bodies and string
  interpolation — every position jq spells as a full `Exp`.
  `as` binds below the comma, matching jq: `1,2 as $x | $x | .+10` is
  `1, (2 as $x | $x | .+10)`, printing `1` then `12`.
  Object-construction *values* stay comma-free — they are jq's `ExpD`, where
  `,` separates entries, so `{a: 1, b: 2}` is unchanged and `{a: (1,2)}` still
  needs its parens. The `n` argument of `limit`/`skip`/`nth` also stays
  comma-free, preserving the existing deliberate restriction (jq's `$n`
  per-output fanout convention is still not implemented). `reduce`/`foreach`'s
  init/update/extract slots and `until`/`while`'s cond/update stay comma-free
  for the same reason: jq forks the whole construct per multi-output `init`,
  folds `update` by its last output per step, and fans `extract`/loop
  backtracking out per output — none of that fanout is implemented here, so a
  comma there would parse but silently misfold instead of erroring (#534).
  Not fixed here, and still divergent: a multi-output expression in a position
  that does not fan out doesn't behave like jq's one-result-per-output rule.
  Some silently take only the first output — `"\(1,2)"`, `{a: (1,2)}`,
  `select(.==1, .==3)` — now parsing where they used to be a parse error,
  previously reachable only with explicit parens (the pre-existing fanout gap
  tracked by #354/#378). Others error instead of fanning out: `range(1,2; 4)`
  (`Range bounds must be numeric`) and a computed object key
  `{(("a","b")): 1}` (`key must be a string`). Separately, a bare top-level
  comma after `label $out |` could reach `break` and discard the comma
  siblings already emitted before it — `label $out | 1,2,break $out,4`
  printed nothing instead of `1`, `2` — the `eval_comma`/`QueryResult`
  architectural gap this paragraph originally flagged here, fixed above
  (#400, #494).

- **`%YAML`/`%TAG` directive lines were not recognized, and swallowed the
  following `---`** (#225): a directive fell through to the plain-scalar
  scanner, which absorbed both the directive text and the document marker
  after it as one scalar (`printf '%YAML 1.2\n--- text\n' | succinctly yq
  '.'` gave `"%YAML 1.2 --- text"` instead of `"text"`). `skip_directives`
  now consumes any `%`-line at column 0 outside a document body, called from
  `parse_documents` both before the first document and after a `...` end
  marker (a directive can recur there). It does not inspect the directive
  name — `%YAML`, `%TAG`, and a reserved directive like `%FOO` are all just
  dropped, so a misspelled name (`%YAM`, `%YAMLL`) is skipped the same way.
  Fixing this exposed two more pre-existing bugs, unrelated to directives,
  that a directive line had been masking: a document-root plain scalar with
  no explicit leading `---` swallowed a following `---`/`...` the same way
  even with no directive involved (`Document\n---\nname: Bob\n` gave one
  scalar instead of two documents) — the continuation loop
  (`parse_unquoted_value_with_indent_impl`) now stops at a column-0
  `---`/`...` marker; and an empty document (nothing between `---` and the
  next boundary or EOF) produced no node at all instead of `null` —
  `end_document` now synthesizes a null node when nothing was written for
  the document, mirroring `close_pending_explicit_key`'s existing null
  synthesis for a key with no value. Clears 13 of the issue's 16 YAML Test
  Suite cases, plus four more (`6XDY`, `7Z25`, `PUW8`, `UT92`) that the
  document-marker/null-document bugs were independently blocking under the
  `structure` category. **Not covered**: `CC74`/`P76L` apply a
  `%TAG`-defined shorthand to a node, which is tag support's job (#224);
  `W4TN` hits a pre-existing zero-indented block scalar gap shared with
  `DK3J`/`FP8R`. Neither the `%YAML` version nor `%TAG` handles are
  surfaced anywhere — there is no per-document metadata slot for them, and
  nothing in the corpus or issue's acceptance criteria requires reading
  them back.

- **`del()` panicked when a comma target mixed identity (`.`) with any other
  path** (#505): `del(.[.x], .)` and `del(., .[.x])` both crashed — an
  index-out-of-bounds panic in release builds, a tripped `debug_assert_eq!` in
  debug — instead of jq's `null`. `delete_expr_paths_at`'s leaf check
  (`src/jq/eval.rs`, added for #424) only compared `start` against
  `paths[0].len()` to decide whether every sibling comma branch was exhausted
  at this depth. `flatten_delete_path` turns `.` into zero `DeleteStep`s while
  any real path is one or more, so whichever branch order put `.` somewhere
  other than index 0 broke: `.` second tripped the assert (the right answer,
  reached the wrong way); `.` first, or any other position, fell into the
  per-branch dispatch loop, which indexed straight past the end of `.`'s empty
  component slice. The check now scans every sibling
  (`paths.iter().any(|path| path.len() == start)`) instead of trusting
  `paths[0]` alone, short-circuiting to `null` the moment any sibling is
  already exhausted regardless of position — matching how `delpaths` reaches
  the same answer by sorting the empty path first
  (`Some([]) => Ok(OwnedValue::Null)`). The analogous depth-mismatch one level
  down (`del(.a, .a.b)`) was already safe and stays that way:
  `delete_expr_object_paths`/`delete_expr_array_paths` split "ends here" from
  "continues" before ever recursing back into `delete_expr_paths_at`, so only
  the top-level call — the one place nothing pre-filters exhausted paths — was
  exposed. New coverage:
  `test_del_with_comma_mixing_identity_and_other_paths` in
  `tests/jq_computed_key_tests.rs`, plus two new
  `tests/data/jq-golden/cases/comma_del_{identity_and_computed,computed_and_identity}`
  fixtures captured from the pinned jq oracle.

- **A plain scalar's `- `-led continuation line was misread as a nested
  sequence at indent 2 and deeper** (#484): `- x\n  - y\n` produced
  `["x",["y"]]`, inventing a nested sequence, where `yq` folds the
  continuation line into the scalar and gives `["x - y"]`. Both are valid
  YAML per the strict validator. `parse_unquoted_value_with_indent_impl`'s
  `sequence_indicator_is_block_structure` (`src/yaml/parser.rs`) treated a
  continuation line's leading `-` as block structure whenever
  `next_indent >= start_indent + 2`, folding only at `next_indent ==
  start_indent + 1`. Per YAML 1.2 `nb-ns-plain-in-line`, once a plain
  scalar's first line has begun, a `-` on a later line is never re-tested as
  a sequence indicator, at any indent — the disjunct is removed outright,
  leaving `next_indent <= start_indent` (reachable only at document root, for
  a `- ` reappearing at column 0 as a genuine new top-level item). The YAML
  Test Suite's only relevant case, AB8U, uses continuation indent exactly 1 —
  the one value that happened to work — so it gave no signal on indent 2+;
  corpus-latent the same way #382 and #409 were. Unaffected: a genuinely
  nested sequence (`- - y`, `- x\n- - y`), recognized immediately after an
  item's own `-` rather than via this continuation-fold path, and the locate
  path (`at_offset`/`yq-locate`), which reads multi-line scalar extents from
  the same index this fixes rather than re-deriving them independently.

- **An out-dented block sequence continuation silently dropped the item and
  corrupted the next mapping entry** (#485): a `-` continuation line indented
  strictly between its sequence's own indent and whatever encloses it —
  `b:\n    - x\n   - y\nc: 2` — read back as `{"b":["x"],"":"c"}`: `y` vanished
  with no error, and the well-formed `c: 2` that followed became a phantom
  `"":"c"` pair with the `2` also gone. `close_deeper_indents` popped the
  sequence for any indent shallower than its own, including one that still sat
  inside the mapping enclosing it; `parse_sequence_item_inner` then reopened a
  *second*, untagged sequence as a sibling child of the mapping rather than a
  value under a key, throwing off the mapping's key/value pairing for
  everything after it. A new `sequence_frame_reaches` predicate — shared by a
  sequence-item-specific close variant and the reuse check, so the two
  definitions of "does this indent still belong to the sequence" cannot drift
  apart — now recognizes an out-of-range indent that doesn't reach down to the
  enclosing frame and keeps the sequence open instead, joining the item to it:
  `{"b":["x","y"],"c":2}`. This is the same "parse the obvious extension"
  policy #325 used for `a: - x`. The input is still invalid YAML and the
  opt-in strict validator continues to reject it.

- **`del()` errored when a deleted key's container was `null`, where jq
  silently no-ops** (#476): jq indexes `null` with any key and gets `null`
  back — `null | .a`, `null | .[0]` and `null | delpaths([["a"]])` are all
  `null` — so deleting through one is always a no-op. `delpaths`/`delete_keys`
  already special-cased `OwnedValue::Null => Ok(OwnedValue::Null)`, but the
  separate walker behind the `del(EXPR)` expression form — `delete_at_path`
  and the grouped-deletion helpers `delete_expr_object_paths`/
  `delete_expr_array_paths` added for #424 — never got the same exemption, so
  `null | del(.a)` raised `Cannot index null with string "a"` and
  `{"x":null} | del(.x.a)` raised the same reaching `.x` mid-chain. Both now
  give `null` an unconditional no-op — regardless of `?` — in `delete_at_path`'s
  `Field`/`Index` arms (top-level and the `Expr::Pipe` chain-walk), its
  `Expr::Slice` chain-walk arm (added at the call site rather than inside the
  shared `through_slice` helper — see below), and the two grouped-deletion
  gates. `#424`'s own test suite had pinned two of these cases as *expected
  errors* (`test_del_computed_index_against_null_does_not_panic`, since
  renamed `..._is_a_no_op`, and the null cases in
  `test_del_container_type_error_is_not_masked_by_an_earlier_optional_sibling`,
  which now uses `5` as its wrong-type example instead) — both updated in
  `tests/jq_computed_key_tests.rs`, plus new coverage in `src/jq/eval.rs` and
  five new `tests/data/jq-golden/cases/null_del_*` fixtures. **Not covered**:
  `del(.[])` on `null` still raises `Cannot iterate over null (null)`,
  matching jq — only `Field`/`Index`/`Slice` steps get the exemption. Writing
  through a slice still does not auto-vivify `null` (`null | .[1:2] =
  ["x"]`) — that is `through_slice`'s shared behaviour with `=`/`|=` and a
  separate, already-documented divergence (see "Where succinctly errors and
  jq does not" in `docs/compliance/jq/limitations.md`), which this fix
  deliberately leaves alone by special-casing `null` at the `del()` call site
  instead of inside the shared helper. Also found, and deliberately left for
  a future issue: a plain (non-`null`, non-comma) missing intermediate key —
  `{"a":1} | del(.b.c)` — still raises `field 'b' not found` where jq
  no-ops; that is a fourth, distinct gap from #475/#476/#477, not fixed here.
  (Since fixed, along with its comma form, by #527 below.)

- **A computed key after a multi-output path component was refused outright**
  (#412): `path(.. | objects | .[.k]?)` errored `Cannot use a computed index
  after a multi-output path component` even though the equivalent value path
  (`.. | objects | .[.k]?` without `path()`) evaluated fine. Only `path()`,
  `=`, `|=` and `del()` are affected — they go through `resolve_dynamic_indexes`,
  which rewrites each computed key into the static component it denotes
  *before* the six path walkers run, and its `resolve_node` fan-out had an arm
  for `.[]` only; every other multi-output component fell to the static-leaf
  arm, which could not name the path reaching each of its many values.
  `resolve_node` now also fans out `..` (`Expr::RecursiveDescent`), bare
  `recurse`/`recurse_down` (which jq defines as `recurse(.[]?)` and which
  therefore shares `..`'s resolver outright), the parameterised
  `recurse(f)`/`recurse(f; cond)` (following
  `builtin_recurse_f`/`builtin_recurse_cond`'s breadth-first queue, including
  their choice not to descend through a null child — `f` is arbitrary and
  `recurse(.a?)` over a null reads null from null forever, so that is what
  bounds the walk), and the typeof filters (`select(f)`, `objects`, `arrays`,
  `values`, `booleans`, `numbers`, `strings`, `nulls`, `iterables`, `scalars`),
  each branch now carrying the actual Field/Index chain that reaches it rather
  than the multi-output expression itself. One difference from the value path
  is deliberate: when `f` yields an array, `[recurse(f)]` descends into its
  elements where jq stops at the array; the resolver stops at the array, i.e.
  agrees with jq rather than mirroring that bug. Not covered: a multi-output
  component with none of those shapes — an arbitrary generator like
  `range(3)`, or `getpath` with a computed argument — still reports the same
  refusal, since naming its path components would mean tracking components for
  a genuinely arbitrary expression. `path(..)` and `path(recurse)` *without* a
  computed key are a separate, pre-existing gap this does not touch —
  `resolve_node` only runs when a computed key is present, and
  `eval_with_path_tracking` (the walker `path()` otherwise uses directly) is
  unchanged.

- **A `?` in the middle of an assignment or delete path made it act on the
  parent of its target**: `del(.[]? | .[.k]?)` on `{"x":{"k":"v","v":1}}` gave
  `{}` — the whole `.x` — where jq gives `{"x":{"k":"v"}}`, and
  `(.[]? | .[.k]?) = 7` reported `invalid path component`. A branch resolved
  under `?` keeps an `Expr::Optional` around its component, so a resolved path
  is `Optional(Field("x")) | Optional(Field("v"))`. `eval_with_path_tracking`
  looks through that wrapper, which is why `path()` read correctly, but
  `get_path_mut`, `update_path` and `delete_at_path` matched it against
  `Field`/`Index`/`Iterate`, missed, and fell to a catch-all that acts at the
  wrapper's own position **with the rest of the path dropped** — hence deleting
  or overwriting the parent. All three now unwrap it, as `flatten_delete_path`
  already did, and splice a nested pipe rather than stranding what follows it.
  Found while extending #412's coverage from `path()` to every writer: the
  arms added there emit the same wrapper, so `del(recurse | objects | .[.k]?)`
  would have inherited the defect.

- **`?` on an assignment path with a computed key swallowed an error raised by
  the key itself** (#413): `"str" | .[.k]? = 5` silently left the input
  unchanged instead of raising jq's `Cannot index string with string "k"` for
  the `.k` that failed. `?` is only supposed to cover a failure to *index* —
  `eval_index_expr` already enforces this in value position (`.[.k]?` there
  correctly still raises) — but the path-context resolver's `Expr::Optional`
  arm caught *any* error from resolving the wrapped node, key evaluation and
  target evaluation included. `resolve_node` now special-cases
  `Optional(IndexExpr { target, key })`: the target and key are resolved with
  `?`, propagating their errors as usual, and only a subsequent failure to
  apply the resolved key to its container (wrong key/container kind) prunes
  the branch. A NaN key still errors under `?` where a number addresses an
  element at all (`[1,2,3] | .[nan]? = 5`, `null | .[nan]? = 5`), because there
  is no element for the write to land on; on a container a number cannot index,
  the failure is the ordinary `Cannot index object with number` that `?` does
  cover, so `{"a":1} | .[nan]? = 5` leaves the document alone as jq does.
  Both `E[K]` arms of the resolver now also evaluate `K` before `E`, matching
  the desugaring `K as $k | E | .[$k]` that the value-position evaluator
  follows: `5 | .a[.k] = 9` blames the `.k` that failed rather than the `.a` it
  never reached, and `5 | .a[empty] = 9` is `5` rather than an error, since an
  empty key stream leaves the target unevaluated.

- **A slice was not a path component** (#366, closing #469 with it): jq has no
  slice *operator* — it models `.[a:b]` as indexing with `{"start":a,"end":b}`,
  and that object is a path component like any other. Succinctly could read a
  slice but did not treat it as a path, so `path(.[1:2])` answered `[1]` (one
  path per element — a *wrong answer* rather than a refusal, inherited by
  everything built on `path()`), `setpath([{"start":1,"end":2}]; ["x"])` and
  `delpaths([[{"start":1,"end":2}]])` silently left the value alone, and
  `.[1:2] = ["x"]`, `.[1:2] |= f` and `del(.[1:2])` were refused outright.
  All now match jq-1.7.1: `path()` yields one component carrying the bounds
  *as written* (`path(.[-2:-1])` keeps its negatives, an open end is `null`),
  and it round-trips through `getpath`/`setpath`/`delpaths`, `=`, `|=`, `+=`
  and `del()`, including mid-chain (`.a[1:2][0].b = 9`) and through the slice
  (`del(.[1:3][0])`). Paths are modelled two ways in the evaluator —
  `OwnedValue` components for the runtime builtins, `Expr` walked directly for
  the operators — so the gap sat in the seam; both halves now share one
  definition in `src/jq/slice.rs` (descriptor validation, bound resolution,
  the component `path()` prints), which the four previously open-coded bound
  clamps also collapse onto. Deletion resolves every key naming an element of
  one array against the length it had on entry and removes them in a single
  batch, so overlapping ranges union rather than compound
  (`delpaths([[{"start":0,"end":2}],[{"start":1,"end":3}]])` on `[1,2,3,4]`
  is `[4]`), and a range naming the same element as a bare index deletes it
  once (`delpaths([[1],[{"start":1,"end":2}]])` is `[1,3,4]`) — the equivalent
  `del(.[0:2], .[1:3])` spelling doesn't run yet, since top-level
  comma-separated `del()` targets are a separate, pre-existing gap (#475).
  `indices`/`index`/`rindex` came along, since jq defines all three
  over `.[$i]` — an object pattern is the slice, not a search, so `"abcabc" |
  indices({"start":1,"end":2})` is the substring `"b"`. Two error sentences
  arrive with the feature, `A slice of an array can only be assigned another
  array` and `Cannot update string slices`, plus `Array/string slice indices
  must be integers` for a descriptor missing a bound or holding a non-number
  (jq requires both keys *present*; an explicit `null` counts, extra keys are
  ignored). The error corpus goes 154/156 → 168/168 in both evaluators and
  `tests/data/jq-error-known-divergences.txt` is now empty. **Not covered**:
  writing through a slice does not auto-vivify `null`, so `null | .[1:2] =
  ["x"]` still refuses where jq answers `["x"]` — deliberate, so that
  `.[1:2] = ["x"]` does not grow a container while `.a = 1` beside it refuses;
  it is recorded in `docs/compliance/jq/limitations.md` under "Where
  succinctly errors and jq does not", to close with the rest of that table.
  Expression-valued bounds (`.[$a:$b]`) remain a parse error.

- **An inline block sequence as a mapping value silently discarded its
  content** (#325): `a: - x` — invalid YAML (test-suite case 5U3A: a block
  sequence may not begin on the same line as its parent mapping key; `yq`
  rejects it) — read back as `{"a":null}`, the `x` gone with no error. #332
  had already stopped the worse failure mode of dropping the text outright,
  keeping it as the literal scalar `{"a":"- x"}`, but that reading was still
  wrong: per YAML 1.2 `ns-plain-first`, a `-` before whitespace is always the
  sequence-entry indicator and never starts a plain scalar. This finishes the
  job: `parse_mapping_entry`, `parse_explicit_value`, and `parse_value`
  (whose existing `-`-followed-by-space arm was dead code — a comment
  claimed "the caller already opened a BP node for us" but nothing was ever
  written into it) now dispatch to the same `parse_sequence_item` the valid
  multi-line spelling already used, so `a: - x` parses as the obvious
  extension `{"a":["x"]}`, and a bare `a: -` is the empty item `{"a":[null]}`
  rather than the string `"-"`. The sequence's indent is derived from the
  `-`'s own column rather than a fixed offset, so a continuation line at the
  same column joins it: `key: - a\n     - b` is `{"key":["a","b"]}`. A
  follow-up commit found the identical gap one level deeper, in
  `parse_compact_mapping_entry` (a sequence item's own compact-mapping value,
  `- a: - x`), which hadn't received the dispatch and still read back as the
  scalar `"- x"`. The opt-in strict validator's 5U3A check is widened to
  accept end-of-input as a terminator too, so a bare `a: -` with no trailing
  newline is now rejected like every other shape the loader accepts
  leniently. Also reconciles the asymmetry the issue called out: the
  parser's dash-continuation guard now shares the same `is_seq_indicator_next`
  predicate the reader already used, rather than its own narrower
  space/tab-only spelling. New coverage: unit tests in `src/yaml/light.rs`
  (item shapes, continuation lines, anchors, non-regression cases for
  `-1`/`-x`/flow `{a: -}`), `tests/yaml_validate_tests.rs`, and
  `tests/yq_cli_tests.rs`. See `docs/compliance/yaml/limitations.md` for the
  updated rationale — flow's `[- x]` still reads as scalar text, since unlike
  the block case there is no sequence to build there.

- **Pretty-printed JSON/YAML output silently dropped duplicate mapping keys;
  compact output was correct** (#442): `yq -o json '.'` on `a: 1\na: 2` gave
  `{"a": 2}` (last-wins) while `-I0` gave `{"a":1,"a":2}`, matching real `yq`
  v4.53.3 only in compact mode. Cause: the pretty path evaluated through
  `GenericResult`/`to_owned()` into `OwnedValue::Object`, backed by an
  `IndexMap` that structurally cannot hold duplicate keys, while `-I0` streamed
  straight from the document cursor. Fix: `JsonCursor`/`YamlCursor`'s
  `stream_json`/`stream_json_document` (and the YAML-side JSON value
  streamers) are now indentation-aware, and the M2 fast path's gate
  (`can_json_fast_path`/`can_yaml_fast_path` in `yq_runner.rs`) no longer
  requires `output_config.compact` — pretty output now takes the same
  cursor-streaming path as compact for identity and simple navigation
  (`.`, `.field`, `.[n]`, `.[]`, and pipes/parens/`?` of those), skipping
  `OwnedValue` construction entirely rather than just formatting it
  differently. Excluded from the widened fast path (falls back to the DOM
  path, unchanged): `sort_keys`, `--ascii-output` (JSON target only — YAML
  output has no such escaping), color, and `--tab` (its indent unit isn't
  plumbed through yet). `explicit_key_non_scalar_pretty` moves off the
  known-failures manifest. Also fixed as a side effect of routing more cases
  through cursor streaming: multi-file pretty output no longer emits a
  spurious leading `---` before the first document.

  Not fixed by this change at the time: the M2 fast path only gave
  `Expr::Identity` a true cursor result — `Field`/`Index`/`Iterate` still
  evaluated to an owned `GenericResult::One`/`Many` and went through
  `to_owned()` when streamed, so a duplicate key *nested inside* a navigated
  field (`.a` where `a`'s value has a repeated key) still collapsed, in both
  compact and pretty output (tracked as a comment on #443, which already
  covers the same `to_owned()`/`IndexMap` mechanism for `to_entries`).
  **Since resolved by #532** (`Expr::Identity => GenericResult::OneCursor`,
  which also converted `Field`/`Index`/`Iterate` to `OneCursor`/`ManyCursor`).
  `--slurp`, `--inplace` (`yaml_to_owned_value`), and `jq --preserve-input`
  pretty output (`standard_json_to_jq_value`, gated by `jq_runner.rs`'s
  `can_use_raw_identity`) went through their own separate, still-`IndexMap`
  -backed conversions — see #478 below for resolution.

- **`yq --slurp '.'` and `yq --inplace '.'` still silently dropped duplicate
  mapping keys after #442** (#478): `yq --slurp '.'` on `a: 1\na: 2` printed
  `a: 2` instead of keeping both entries inside the slurped element, and
  `yq --inplace '.'` on the same input wrote `a: 2` back to the file,
  diverging from real `yq --inplace` v4.53.3's `a: 1\na: 2`. Cause: both
  went through `yaml_to_owned_value()` (`yq_runner.rs`), and `--slurp`'s
  result was re-collapsed a second time by the `IndexMap`-backed
  `standard_json_to_owned()` inside `evaluate_input`'s JSON round-trip —
  neither was reached by #442's fix, which only widened `Expr::Identity`'s
  path to stdout. (#478's third listed site, `jq --preserve-input` pretty
  output, turned out to already be fixed incidentally by #532, merged
  before this fix; only a regression test was added for it here.)

  Fix: `--inplace` now reuses the M2 cursor-streaming machinery for the same
  shapes `can_use_m2_streaming` already accepts (`.`, `.field`, `.[n]`,
  `.[]`, and pipes/parens/`?` of those), via new
  `can_inplace_json_fast_path`/`can_inplace_yaml_fast_path` gates that mirror
  `can_json_fast_path`/`can_yaml_fast_path` but write into a per-file buffer
  instead of stdout before the existing `fs::write`. `--slurp` gets a
  narrower, identity-only fast path (a non-trivial filter over the slurped
  array still needs real evaluation) backed by a new
  `yaml::stream_yaml_sequence()` helper: every source is parsed up front
  into an owned `(bytes, YamlIndex)` pair so their cursors can share one
  `Vec`'s lifetime, then streamed into a single YAML sequence using the same
  container-vs-scalar block/flow rendering `stream_yaml_value`'s `Sequence`
  arm already uses — reused rather than re-derived, so multi-document slurp
  output matches single-document M2 streaming byte-for-byte. `-o json
  --slurp` still uses the slow DOM path, the same explicit, documented scope
  limit `sort_keys`/color/`--tab` already have on the gates above.

  Not fixed by this change: `standard_json_to_jq_value()` is still reachable
  and lossy through `Builtin::First`/`Builtin::Last` and `Pipe`'s
  stage-advance re-entry (e.g. `jq --preserve-input 'first(.[])'` on
  `[{"a":1,"a":2}]` collapses to one `"a"`), found while verifying the
  `--preserve-input` fix above — filed as #607. **Since resolved by #607**,
  though its actual root cause was different from this note's suspicion —
  see below.

- **`jq --preserve-input 'first(.[])'`/`'last(.[])'` still collapsed
  duplicate keys inside the selected element** (#607): despite #478's note
  above naming `Builtin::First`/`Builtin::Last` as the culprit, tracing
  `first(.[])` through the parser found the real cause was one step earlier —
  `first(f)`/`last(f)` compiles to `Expr::FirstExpr`/`LastExpr` (the
  dedicated `first(...)` parse path) or, from a second older parser path, the
  equivalent `Builtin::FirstStream`/`LastStream`, and *neither* had a native
  arm in `eval_generic::eval_single`. Both fell through the catch-all
  `to_owned()` bridge at the bottom of that function, which materializes the
  entire input into an `IndexMap`-backed `OwnedValue` — collapsing the
  duplicate key *before* `first`/`last` even ran, regardless of what
  `Builtin::First`/`Builtin::Last` did. Fixed by giving `eval_single` and
  `eval_builtin` native arms for all four spellings, delegating to a new
  `eval_first_or_last_generic` helper that mirrors
  `eval::eval_first_expr`/`eval_last_expr`'s control-flow (short-circuit on
  first output vs. must-exhaust-the-stream for `last`) while preserving
  `GenericResult::OneCursor`/`ManyCursor` through the extraction instead of
  materializing.

  The originally-suspected sites were real bugs too, just not *this* one:
  `Builtin::First`/`Builtin::Last` (the bare zero-arg `first`/`last` keyword,
  equivalent to `.[0]`/`.[-1]` — a different AST node from `first(f)`/
  `last(f)`) called `elements.get(...)` instead of the `get_cursor(...)`
  sibling `Expr::Index` already used, and `index_one_generic` (behind
  computed-key indexing like `.[$k]`/`.[(expr)]`, via `Expr::IndexExpr`)
  called `fields.find`/`elements.get` instead of `find_cursor`/`get_cursor`.
  Both fixed the same one-line way, plus `eval_index_expr`'s accumulation
  loop now collects `V::Cursor`s and emits `OneCursor`/`ManyCursor` instead
  of a materialized `One`/`Many`. Audit bonus: `Builtin::SplitDoc` was
  documented as "identity" but unconditionally returned
  `GenericResult::Owned(to_owned(&value))` instead of forwarding the cursor
  the way `Values`/`Iterables`/`Scalars`/`Identity` already do — fixed to
  match.

  Out of scope, and documented as such: the catch-all fallback itself (any
  `Expr`/`Builtin` not natively handled by `eval_single`/`eval_builtin` --
  arithmetic, `map`, `recurse`, `group_by`, construction, etc.) still
  materializes via `to_owned()` first; narrowing it further is a much
  larger-blast-radius change than this issue's scope. Also out of scope:
  `yq`'s equivalent CLI paths (`evaluate_yaml_cursor` in `yq_runner.rs`)
  convert *every* `GenericResult::OneCursor`/`One` through `to_owned()`
  unconditionally, so `yq 'first(.[])'`/`'last(.[])'`/a computed index still
  collapse duplicate keys even after this fix — confirmed pre-existing
  (reproduces identically on `main`), unrelated to `standard_json_to_jq_value`
  (JSON-only), and large enough in its own right to need separate
  investigation.

- **`yq 'first(.[])'`/`'last(.[])'`/a computed index (`.[(expr)]`) still
  collapsed duplicate mapping keys** (#631), the `yq`-side follow-up #607
  above left open: `eval_generic.rs`'s evaluator already threaded
  `GenericResult::OneCursor`/`ManyCursor` through these shapes correctly
  (that was #607's fix, shared by both `jq` and `yq`), but `yq_runner.rs`
  never had a `jq_runner.rs`-style `generic_result_to_jq_values` that could
  take advantage of it — every expression not covered by `can_use_m2_streaming`
  (identity/field/index/iterate navigation and pipes/parens/`?` of those) fell
  through `evaluate_yaml_cursor`'s unconditional `to_owned()` DOM path instead,
  the same `IndexMap`-backed bridge #442/#478 fixed for plain `.`/`.[0]` but
  never widened past literal navigation.

  Fixed by widening `can_use_m2_streaming` to also treat `Expr::FirstExpr`/
  `LastExpr` (`first(f)`/`last(f)`), `Builtin::FirstStream`/`LastStream` (the
  second AST spelling the parser produces for the same syntax, see #607's
  note), and `Expr::IndexExpr` (computed indexing) as streamable — the
  existing M2 fast path's non-identity branch already evaluates via
  `eval_with_cursor_using` and renders through `GenericResult::stream_yaml`/
  `stream_json`, which handle every `GenericResult` variant (cursor and owned
  alike) uniformly, so no new rendering code was needed. `yq 'first(.[])'`
  now streams through the exact same mechanism as `yq '.[0]'` on the same
  input, matching it byte-for-byte instead of merely agreeing on the
  duplicate-key count.

- **`yq -S`/`--sort-keys` and `--tab` still collapsed duplicate mapping
  keys** (#733), the last gap #442 left open: both flags were excluded from
  `can_stream_pretty` (`yq_runner.rs`), so any identity/navigation query
  under either flag fell through to the `OwnedValue::Object`/`IndexMap`
  DOM path and lost all but the last occurrence of a repeated key — e.g.
  `yq -S '.'` on `a: 1\na: 2` printed `a: 2`. Unlike #442/#478/#607/#631,
  this wasn't a missing expression shape in `can_use_m2_streaming`; it was
  the cursor/lazy streamers themselves not supporting sort or a non-space
  indent unit, exactly as the code comment at the time predicted ("`tab`
  indentation needs a string-based indent unit they don't accept yet").

  Fixed by teaching the streamers both features instead of excluding the
  flags: `DocumentCursor::stream_json`/`stream_yaml` (and every concrete
  implementation and caller — `YamlCursor`'s mapping/sequence streamers,
  `GenericResult::stream_json`/`stream_yaml`, `jq/stream.rs`'s
  `OwnedValue`/lazy-keys streamers) now take an `IndentSpec { width, unit }`
  (`unit` is `'\t'` for `--tab`, `' '` otherwise) instead of a bare space
  count, plus a `sort_keys` flag. `YamlCursor`'s mapping arm sorts by
  materializing that mapping's fields into a `Vec<YamlField>` (`YamlField`
  is `Copy`, so this is cursor-only, no value data) and stable-sorting by
  key — duplicate keys stay adjacent in original relative order rather than
  merging, since a `Vec` sort, unlike an `IndexMap` insert, never collapses
  equal keys. `can_stream_pretty` now only excludes color (`use_color`),
  which has the same latent bug but is out of scope here (unreported,
  tracked as a follow-up). Also closes a `--tab`-only correctness gap the
  widened gate would otherwise have introduced: `keys_unsorted` is also
  `can_use_m2_streaming`-eligible and streams through a separate helper
  (`stream_lazy_keys_json`/`_yaml`) with its own indent-writing code, which
  needed the same `unit` threading to avoid emitting spaces under `--tab`.

- **The strict YAML validator accepted a flow-collection anchor immediately
  followed by an alias** (#452): `[&a *a]` and `{k: &a *a}` passed
  `succinctly yaml validate`, which `yq` rejects — an anchor property cannot
  decorate an alias node. Block context already rejected the same shape on
  one line (`&a *b`, `AnchorOnAlias`, SR86/SU74) via `scan_anchor`, but
  `scan_flow`'s `&` arm (shared by `[...]` and `{...}`) had no equivalent
  check: it recorded the anchor and read a following `*alias` as an ordinary
  reference, which also passed the unrelated #404 unknown-anchor check since
  the anchor had just been registered into scope. The placement check is now
  `check_after_anchor`, shared by both call sites — block's `scan_anchor`
  (after its same-line `skip_spaces_and_tabs`) and flow's `&` arm (after
  `skip_flow_ws`, which — unlike block — also crosses line breaks and
  comments, so `[&a\n*a]` is rejected too).

- **Two `compare_values` comparators (and a private `numeric_repr_cmp`) disagreed
  with jq about NaN, and with each other** (#421): jq treats NaN as strictly
  less than every number, including another NaN — `nan < 1`, `nan < nan`, and
  `nan <= nan` are all `true` in jq-1.7.1, while `nan >= nan` is `false`.
  `f64::partial_cmp` returns `None` for any NaN comparison, and the two
  evaluators papered over that differently: the full evaluator
  (`src/jq/eval.rs`) folded `None` to `Ordering::Equal`, so NaN compared equal
  to every number and `[1,2,3] | bsearch(nan)` falsely reported it "found"; the
  generic (CLI) evaluator (`src/jq/eval_generic.rs`) folded the resulting
  `Option::None` to `false` in its `<`/`<=`/`>`/`>=` fast path, so NaN compared
  less than nothing. A new `cmp_f64` (`src/jq/value.rs`) centralizes jq's rule;
  `eval_generic.rs`'s own `compare_values` is gone, importing the full
  evaluator's instead, so the two can no longer drift (#358/#384 precedent).
  `sort`/`min`/`max` now order NaN as jq does and `bsearch` now correctly
  reports a NaN needle absent. Not fixed here: a separate, pre-existing defect
  where a freshly-constructed array materializes through JSON text (which has
  no NaN literal) on its way into `unique`/`group_by`, silently turning NaN
  into a real `Null` before the comparator runs — so `[nan,nan] | unique` still
  doesn't match jq's `[null,null]`. That's a different mechanism (tracked
  separately) and left as a documented known divergence
  (`test_nan_container_ordering_known_divergence_421`).

- **YAML `to_entries` collapsed a duplicate mapping key to its last
  occurrence instead of emitting one entry per occurrence** (#443, a
  follow-up gap left open by #174): `a: 1\na: 2` piped through `to_entries`
  gave a single `{"key":"a","value":2}` where real `yq` emits both entries
  unmerged. The
  generic/cursor evaluator used for YAML (`eval_generic.rs`'s `eval_builtin`)
  had no native arm for `Builtin::ToEntries`, so it fell through the
  catch-all that materializes the whole value via `to_owned()` first —
  which merges duplicate keys into one `IndexMap` entry before `to_entries`
  ever runs, even though the field cursor it reads from indexes every
  occurrence. Added a native `ToEntries` arm that walks the field/element
  cursor directly, building one `{key, value}` entry per field (mirroring
  `Keys`/`Iterate` in the same file, and the already-correct JSON-side
  `builtin_to_entries`), so no user key is ever put into a shared map. New
  coverage: a duplicate-key unit test in `eval_generic.rs`, two
  `tests/yq_cli_tests.rs` cases (default YAML and compact JSON output), and
  a `to_entries_duplicate_keys_{compact,pretty}` golden fixture pair
  captured from real `yq`. The sibling `-o=json`/pretty-print identity
  collapse is unrelated to this dispatch path and remains open as #442.

- **`from_entries` and six other `map`-derived builtins refused an object of
  entries that jq accepts** (#422): jq defines `from_entries` as
  `map({...}) | add | .//={}`, and `map(f)` is `[.[] | f]` — `.[]` over an
  object iterates its *values*, so jq accepts
  `{"x":{"key":"a","value":1}} | from_entries` (`{"a":1}`) as readily as the
  array form. `from_entries`, `add`, `any`, `all`, `join`, `flatten` and `map`
  (`src/jq/eval.rs`) each matched `StandardJson::Array` alone and routed an
  object straight to `Cannot iterate over object (…)`; the refusal predates
  #391, but #391 derived the rest of `from_entries` from that same `map(f)`
  definition, which is what made this one half stand out. All seven now also
  match `StandardJson::Object`, iterating its values via the idiom
  `Expr::Iterate` already uses for `.[]`; `any`/`all`/`join`/`map` keep their
  short-circuiting/streaming behaviour via small helpers generic over either
  element source, rather than collecting eagerly first. Left unchanged: `min`,
  `max`, `min_by`, `max_by`, `unique`, `unique_by`, `group_by` — verified live
  against jq-1.7.1 that jq itself refuses an object for all of these, so
  matching jq there means leaving them exactly as they were (they do have a
  separate, pre-existing error-wording gap on that refusal, left for a
  follow-up issue). New coverage: an object and an empty-object case in each
  builtin's existing unit test, plus seven golden fixtures under
  `tests/data/jq-golden/cases/`.

- **An anchor or alias on the key of a flow-sequence's implicit single-pair
  mapping was a parse error or bound to the wrong node** (#409, found while
  fixing #405 — that issue was the same key position in a flow *mapping*,
  `{*a: v}`): `[&x k: 1, *x: 2]` errored `unexpected character ':': expected
  ',' or ']' in flow sequence` instead of `yq`'s `[{"k":1},{"k":2}]`, and
  `[&x k: 1, *x]` — an anchor on such a key, aliased from a later plain item —
  resolved the alias to the mapping `{"k":1}` instead of the key text `"k"`.
  Two independent bugs in `parse_flow_sequence_inner`'s per-item dispatch,
  neither in the key handling #405 consolidated:

  The dispatch consumed a leading `&`/`*` *before* asking whether the item was
  a `key: value` pair, so an alias key's `*` was read as a standalone aliased
  *value* by `parse_alias`, which `continue`d with the cursor left on `:` —
  reordering alone would not have fixed it, since `looks_like_flow_mapping_entry`
  skipped a leading quote or container when scanning ahead for the `:` but not
  a leading `&name`/`*name`, so it answered `false` for both shapes. And an
  anchor's bare `parse_anchor()` call records the anchor against whatever BP
  node opens *next* — here the implicit mapping *wrapper*
  `parse_implicit_flow_mapping_entry` was about to open, not the key inside
  it. This is exactly the shape `record_key_anchor`/`record_key_alias` exist
  for (corpus case CN3R: `&flowseq [a: b, &c c: d, ...]` exercises the anchor
  half already, but never aliases `&c`, so the wrong binding was invisible on
  JSON-output-only assertion — corpus-latent, and confirmed unaffected by this
  fix); `parse_implicit_flow_mapping_entry` was the one key site that read
  straight from `parse_flow_key_scalar` instead.

  `looks_like_flow_mapping_entry` now skips a leading `&name`/`*name` (reusing
  `simd::parse_anchor_name`, the same scanner the real anchor/alias parse
  uses, rather than a second hand-rolled terminator set — see the #106 lesson
  in `CLAUDE.md`) before scanning for the `:`, so the pair check runs before
  the sequence loop's anchor/alias consumption rather than after. And
  `parse_implicit_flow_mapping_entry`'s key now shares `parse_flow_key` with
  the flow-mapping key site wholesale, rather than adding a fourth direct
  call to `record_key_anchor`/`record_key_alias` alongside it — one definition
  for both implicit-key sites, gaining the anchor/alias handling for free.
  `parse_flow_key_scalar`, now unreachable, is removed. Unaffected: `[a: 1, b:
  2]` and `[&x a, *x]`, the two shapes the issue calls out as already correct.

- **A comma-generator was rejected inside call arguments** (#155, closing the
  call-argument half — #360 already closed the index-bracket half): builtin
  calls like `sort_by(.a,.b)`, `first(1,2,3)`, `[limit(2;1,2,3,4)]`, and
  user-defined calls like `def f(x): x; f(1,2)` failed to parse with
  `expected ';' or ')' in function arguments`. Every call-argument slot in
  `src/jq/parser.rs` — `parse_func_call_or_error`, `parse_namespaced_call`,
  and the ~35 builtins in `try_parse_builtin` — parsed each argument with
  `parse_pipe_expr` (no top-level comma) instead of `parse_comma_expr` (full
  expression, comma included), unlike `parse_index_bracket`, which already
  received this fix for #360. Deliberate exception: the `n` (count) argument
  of `limit`/`skip`/`nth` stays restricted to non-comma, since this codebase
  doesn't implement real jq's `$n` per-output-fanout parameter convention for
  those three — accepting a comma there would parse but silently take only
  the first output, which is worse than today's clean parse error. Fixing
  the parser alone would have introduced a silent regression: `sort_by`,
  `group_by`, `unique_by`, `min_by`, and `max_by` (`src/jq/eval.rs`) computed
  their key by evaluating the key filter once and defaulting anything but a
  single output to `null`, so `sort_by(.a,.b)` would have newly parsed but
  silently sorted everything as equal. All five now key by `[f]` — the array
  of *all* outputs of the key filter, reusing `eval_array_construction` —
  matching jq's actual semantics, so `sort_by(.a,.b)` is a genuine multi-key
  sort. `limit`/`first`/`last`/`nth`'s eagerness (they evaluate their
  generator argument to completion before truncating) is unchanged: it
  already produces correct output for the finite generators in scope here,
  and true short-circuiting for infinite generators is a separate,
  significantly larger change tracked for a follow-up. New coverage:
  `test_comma_in_call_arguments` (parser), `test_by_builtins_multi_key_comma_generator`,
  `test_limit_comma_generator_argument`,
  `test_first_last_expr_comma_generator_argument`,
  `test_user_function_call_with_comma_generator_argument` (eval), and three
  new `jq_golden_tests` cases (`comma_in_call_args`, `comma_in_limit_arg`,
  `comma_in_user_func_call`).

- **`delpaths` silently accepted inputs jq refuses** (#395, closing #415):
  `delpaths` deleted what it could and dropped the rest instead of raising.
  `1 | delpaths([["a"]])` returned `1` unchanged instead of `Cannot delete
  fields from number`; `{"a":1} | delpaths([[0]])` returned `{"a":1}` instead
  of `Cannot delete number field of object`; `[1,2] | delpaths([0])` — a
  plausible typo for `delpaths([[0]])` — returned `[1,2]` instead of `Path
  must be specified as array, not number`. `delete_keys` and
  `delete_paths_under` (`src/jq/eval.rs`) now return `Result<OwnedValue,
  EvalError>` and raise jq's own sentences — `Cannot delete <type> field of
  object`, `Cannot delete <type> element of array`, `Cannot delete fields
  from <type>`, and `Cannot index <type> with <key>` for a scalar reached
  mid-path — and `builtin_delpaths` validates every entry's shape as a
  pre-pass before any deletion runs, so `delpaths([[0],"a"])` refuses outright
  rather than deleting `[0]` first the way a per-path loop would. Four new
  error constructors on `EvalError` (`src/jq/error.rs`) carry the exact
  wording, confirmed against jq-1.7.1. `null` stays a no-op, as jq treats it.
  Not fixed here: `delpaths`/`setpath` still silently no-op on an
  object-shaped ("slice") path component against an array instead of
  performing the slice edit or raising, tracked as #469.

- **jq compound/alternative assignment (`+= -= *= /= %= //=`) evaluated the
  right-hand side against the sub-value at the path instead of the document
  root** (#159): `eval_compound_assign`/`eval_alternative_assign` in
  `src/jq/eval.rs` built a filter embedding the *unevaluated* RHS expression
  and handed it to `update_path`, whose `Identity` leaf supplies the sub-value
  already navigated to `path` as `.` — so `.a += .b` resolved `.b` against
  `.a`'s value, not the root. `{"a":1,"b":2} | .a += .b` raised `expected
  object, got number` instead of `{"a":3,"b":2}`; `{"a":null,"b":5} | .a //=
  .b` returned `.a` unchanged instead of `5`. A literal RHS (`.a += 5`) masked
  the bug since a literal doesn't reference `.`. Fixed by evaluating the RHS
  once against the original input up front (new `eval_rhs_once` helper,
  shared with `eval_assign`, which already worked this way) and splicing the
  resulting value into the filter via the existing `owned_to_expr` (also used
  by `eval_as`/`eval_reduce`/`eval_foreach`), so `update_path`'s per-path `.`
  no longer resolves into it. Confirmed against real jq (jq-1.7.1) that the
  RHS is evaluated exactly once against the pristine root even when the path
  expression touches multiple elements — `{"a":[1,2,3]} | .a[] += .a[0]`
  yields `{"a":[2,3,4]}` (every element gets the original `.a[0]`), not
  `[2,4,6]` — and that `//=` is not specially lazy (`.a //= error("x")` still
  raises when `.a` is already truthy), both now covered by new tests
  alongside the two reported repros.
- **`yq -o json '.'` over a document with many small mapping records —
  arrays of flat objects, the `users` benchmark pattern — went from O(n) to
  O(n²)** (found reviewing the merge-key fix below): a 5MB `users` document
  went from ~150ms to ~1.15s (7.7x, and the gap grew with size — 30MB went
  from ~1s to 31.5s). The regression was not in the merge-key code itself
  but in `AdvancePositions`/`CompactEndPositions` (`src/yaml/
  advance_positions.rs`, `src/yaml/end_positions.rs`): both keep a single
  document-wide sequential cursor optimized for monotonically increasing
  access, and their `get_random` backward-jump fallback reset its
  incremental IB-scan state to word zero instead of the position it had
  just found — so the *next* forward access had to rescan the whole
  document's index from the start to catch up. `resolve_merge_keys`
  (checking a mapping's fields for `<<`, then walking them again to build
  the result) triggered exactly that backward jump on every mapping,
  merge key or not, turning an O(n) document walk into O(n²). Fixed
  `get_random` to seed the resumed cursor from the position its own
  `ib_select1` scan already found instead of discarding it (a first
  attempt at this got the word-boundary arithmetic wrong when the match
  fell inside the select sample's own word — caught by an A/B
  output-identity diff against `main`, not by any existing test, since
  every existing test in both files is far smaller than
  `SELECT_SAMPLE_RATE` (256) and so never exercised that path), and
  separately restructured `resolve_merge_keys` to decode each key once in
  a single forward pass rather than scanning twice. Verified via
  interleaved A/B benchmark against `main` (output-identical throughout):
  100KB–30MB now within ~10-15% of `main` at every size instead of a
  growing multiple. New regression coverage:
  `test_get_random_then_sequential_resumes_correctly` and
  `test_get_random_access_pattern_matches_reference` in both
  `advance_positions.rs` and `end_positions.rs`, using non-arithmetic
  position gaps so select samples don't accidentally land on word
  boundaries (`SELECT_SAMPLE_RATE` is itself a multiple of 64).
- **YAML merge keys (`<<: *anchor`) were indexed as a literal `"<<"` key
  instead of merging the referenced mapping's fields** (#171): `d: &d {x: 1}` /
  `m:` / `  <<: *d` / `  y: 2` produced `{"<<":{"x":1},"y":2}` instead of the
  expected `{"x":1,"y":2}`. Resolution happens at query time in
  `YamlFields::from_mapping_cursor` (`src/yaml/light.rs`) rather than during
  parsing or index construction, so every consumer — field access, `.[]`
  iteration, `keys`/`to_entries`, and the direct YAML→JSON/YAML streaming
  paths — gets it through the one shared `uncons`/`find` primitive, with no
  new index format and no cost for the common merge-free mapping. Semantics
  were verified empirically against the pinned `yq` v4.53.3 oracle rather
  than the written spec, since that binary's default (non-`--yaml-fix-merge-
  anchor-to-spec`) behavior is what `succinctly yq` must match: a later key
  (real or merged) overwrites an earlier same-named one's *value* in place,
  keeping the earlier key's position; `<<: [a, b, ...]` folds its sources in
  reverse so an earlier-listed source still wins value conflicts per the
  merge-key spec while a later one's unique keys claim the earlier positions;
  a merge source's own fields are copied verbatim rather than recursively
  re-resolved (yq does not expand a merged-in mapping's own `<<`); and an
  invalid merge value (null, a scalar, a non-mapping alias target, or a
  non-mapping sequence element) contributes nothing rather than erroring.
  Unignores the two merge-key tests already written for this in
  `tests/yq_cli_tests.rs`, adds a dedicated `tests/data/yq-golden/cases/
  merge_key` fixture, and extends the `anchors` benchmark pattern so the
  end-to-end suite no longer has a blind spot here (previously in
  `OUT_OF_SCOPE` in `tests/yaml_bench_suite_coverage.rs`). **Breaking**: the
  public `YamlFields` type is no longer `Copy` (only `Clone`) — a
  merge-resolved mapping's field list is shared via `Rc` rather than being a
  bare cursor, and `DocumentFields`'s trait bound relaxed from `Copy + Clone`
  to `Clone` to match. Not covered: `yq-locate`'s reverse position lookup
  (`src/yaml/locate.rs`) still walks the raw BP structure and does not know
  about merge keys, and a merge source that is itself merged verbatim into
  a *second* mapping can show yq's own traversal-order-dependent quirk where
  querying the whole document resolves it but querying that path alone does
  not (`resolve_merge_keys`'s doc comment has the details); succinctly always
  gives the latter (pure, local, no cross-node mutation) answer.

- **`del()` with multiple negative computed indexes deleted the wrong
  element** (#424): `sort_paths_for_deletion` in `src/jq/eval.rs` ordered
  resolved paths by trailing index descending and deleted them one at a
  time, which is only sound while every index counts from the same end of
  the array. `[10,20,30,40] | del(.[(-1,-2)])` deleted `-1` (`40`) first,
  shortening the array to length 3, so `-2` then counted back from *that*
  and took `20` instead of `30` — `[10,30]` where jq gives `[10,20]`.
  Reversing the argument order didn't help, since `-1` and `-2` count from
  the opposite end to a non-negative index, so no ordering of one-at-a-time
  deletions is correct. The same defect reached nested, independent computed
  indexes too (`.[(0,1)][(-1,-2)]`) — not called out in the issue, but
  sharing the identical cause. Fixed by grouping resolved paths that share a
  container and deleting each container's keys simultaneously, resolving
  every index against the length its container had before any sibling here
  was removed — reusing `delete_keys`, the same primitive `delpaths` was
  fixed with in #398, while keeping `del`'s own type/bounds error checks.
  New coverage: four cases (reported, reversed, mixed-sign, and nested) in
  `test_del_with_negative_computed_indexes_resolves_against_original_length`
  in `tests/jq_computed_key_tests.rs`.

  Two follow-up defects surfaced in review of the fix above, both in the new
  grouping code in `src/jq/eval.rs`, neither reachable from the issue's own
  repro:
  - `delete_expr_paths_at` dispatched once on the *first* resolved sibling
    path's shape (`Field`, `Index`, or `Iterate`) and assumed every other
    sibling at that depth matched it. `null` breaks that assumption — it
    accepts a string key, a numeric key, or `.[]` without erroring, so
    `.[("a",0)]` resolved against a null target yields one `Field("a")` path
    and one `Index(0)` path at the same position — and `null | del(.[("a",0)])`
    crashed with `unreachable!()` instead of erroring or succeeding. Fixed
    by partitioning sibling paths by actual shape instead of trusting the
    first one to speak for the rest; each partition now runs through its own
    grouped deletion in turn.
  - The out-of-range/missing-key checks for a container or key shared by
    several sibling paths (`del(.a?, .[("b","c")])`; `del(.[(0,5)].a,
    .[5]?.a)`) read only one representative path's `optional` flag —
    `paths[0]` for "is this whole container the wrong type", `group[0]` for
    "is this specific missing key or out-of-range index covered" — so
    whether the merged operation raised depended on which sibling happened
    to be listed, or grouped, first. Fixed by checking every contributing
    path's flag: the whole-container check now raises unless *every* path
    reaching it is optional (each fails identically, so one non-optional
    path is enough), while the shared-key checks now suppress the error if
    *any* contributing occurrence is optional (one `?` covers the rest).
  New coverage: `test_del_computed_index_against_null_does_not_panic`,
  `test_del_merges_optional_across_duplicate_indexes_order_independently`,
  and `test_del_container_type_error_is_not_masked_by_an_earlier_optional_sibling`
  in `tests/jq_computed_key_tests.rs`.

- **A tab that indented a sequence-item continuation line was folded into a
  plain scalar instead of rejected** (#432, also fixing #371): three related
  gaps in `parse_unquoted_value_with_indent_impl` and `parse_mapping_entry`
  in `src/yaml/parser.rs`, all downstream of the `tab_indents_block_structure`
  check introduced by #173/#381 not reaching every site that dispatches on a
  continuation line. `a:\n \t- x\n` loaded as `{"a":"\t- x"}` — a key's
  "value is on the next line" arm left the tab on the cursor when it indented
  block structure, so the `Some(b'-')` sequence check silently missed it and
  fell through to a plain scalar. `- a\n \t- b\n` loaded as `["a - b"]`
  because the plain-scalar continuation scan only compared indentation by
  counting *spaces*, so a tab that itself indented a sibling sequence item
  read as "more indented, keep going" and folded the second item into the
  first's scalar; the same gap made `a: 1\n \tb: 2\n` silently drop `: 2`
  (#371) since a mapping value at indent 0 hit the same scan gated on
  `start_indent == 0` rather than the narrower `is_doc_root` the function
  already computed. All three now raise `TabIndentation`, matching the
  opt-in validator, which already rejected all three shapes. New coverage:
  three rows in the `VERDICTS` table in `tests/yaml_tab_indentation_tests.rs`
  (plus the pre-existing #371 row moved from the "loader and validator
  legitimately disagree" section into "both must reject" now that they
  agree); `yaml_test_suite_conformance` and `yq_golden_conformance` re-run
  clean.

- **YAML validator misclassified lines around a multi-line quoted scalar,
  in one direction silently accepting an invalid document and in the other
  wrongly rejecting a valid one** (#382): `Validator::line_kind`'s `"` arm
  advanced past a raw line break without noticing it had left the quote
  open, so a later `"` was misread as *opening* a fresh span instead of
  *closing* the real one — `"line one\n line two"\nc: d\n` (a quoted scalar
  root followed by an incompatible `c: d` mapping, which must be rejected as
  a second root node) was silently accepted. Fixing that cross-line
  tracking exposed a second bug that had to land in the same change: neither
  quote arm gated on whether the quote was glued to preceding content the
  way `line_is_structural`'s `after_separation` check already did, so
  `foo'bar: baz\nqux: quux\n` — an ordinary plain-scalar key containing a
  literal `'`, followed by an ordinary second entry — was wrongly rejected
  (an ungated quote-open would otherwise hunt arbitrarily far into later
  content for a partner once it correctly stopped bailing at the first line
  break). Both quote kinds now fold across lines to their true close via one
  shared `quoted_span_end` in `src/yaml/mod.rs`, replacing what had become
  three independent hand-rolled scans (`line_is_structural`'s single-line
  `quoted_scalar_end`, and `line_kind`'s two inline, mutually-asymmetric `"`
  and `'` arms) — the duplicated-predicate shape #106/#332 already flagged,
  left standing by #375 (closing #173) since a naive merge would have been a
  behavior change dressed as a refactor. Conformance figures are unaffected
  (216/279 · 70/94 · 27/29 unchanged); both bugs were corpus-latent.

- **Project-wide audit of YAML `s-white`/unconditional-terminator predicates**
  (#434), the exhaustive search #173/#370/#410/#411 each asked for reactively.
  Six more instances of the same shape, all in `src/yaml/`:
  - `find_scalar_end` (`light.rs`), the cursor's re-derivation of a scalar's
    extent used by `at_offset`/`syq-locate`, broke unconditionally on `,`/`]`/`}`
    with no flow-context check at all, despite its own comment calling them
    "flow context delimiters" — its dead sibling `find_plain_scalar_end`
    already gated the same arm on `if in_flow`. Any block-context scalar
    containing a literal comma, `]`, or `}` (`note: hello, world`) printed
    correctly via `syq` but located a truncated range via `syq-locate`/
    `at_offset` — the highest-impact finding, since this trigger is far more
    common in real YAML than any tab-adjacency case.
  - `parse_unquoted_value_with_indent_impl`'s colon-terminator check had no
    `None` arm, so a colon as the last byte of a document (no trailing
    newline) was absorbed into the value instead of ending it, while
    `find_scalar_end` already stopped there — eval and locate disagreed on
    the same node, the #370 shape reached via absolute EOF instead of a tab.
  - `is_document_start`/`is_document_end` required the marker to be followed
    by `Some(b' ' | b'\n' | b'\r') | None`, missing the tab the strict
    validator's `doc_marker_char` already accepted — confirmed against the
    YAML Test Suite's K54U case (`---\tscalar`), previously a known failure,
    now passing.
  - Four sites choosing whether `?`/`:` at line start were explicit-key/value
    indicators matched the same terminator set missing a tab, while the `-`
    (sequence indicator) check a few lines away in each of those functions,
    and the canonical shared `is_seq_indicator_next` (fixed for #332
    specifically to stop this drift), already included it. `?\tkey\n:\tvalue\n`
    loaded as two unrelated top-level documents instead of one mapping entry.
  - `is_sequence_at_same_indent` (deciding whether a same-indent block
    sequence is an anchored mapping key's value) was missing the tab its own
    doc comment claims parity with `parse_compact_mapping_entry` on — that
    sibling already had it. `key: &a\n-\tx\n-\ty\n` resolved to `{"key":null}`
    instead of `{"key":["x","y"]}`, dangling the anchor.
  - `parse_explicit_key`'s inline dispatch on a `-` starting an explicit key's
    value (`? - a\n  - b\n: value`, a non-scalar key, #172) hand-rolled the
    same 3-way match missing the tab instead of calling the canonical
    `is_seq_indicator_next` every sibling `-` check in the file already uses
    (#332) — caught in review of this same PR, after the rest of the audit
    above had already landed. `? -\ta\n  -\tb\n: value\n` fell through to
    being parsed as a plain scalar key `-\ta`, losing the second sequence item
    and the value entirely (`{"-\ta":["b"]}` instead of `{"":"value"}`).

  A seventh candidate — `skip_newlines` not skipping a tab-led blank/comment
  line — was implemented and then reverted: it silently turned the YAML Test
  Suite's Y79Y/000 (`foo: |\n\t\nbar: 1\n`, `fail: true`) from a correctly
  rejected document into `{"foo":"","bar":1}`, the opposite compatibility risk
  this issue itself warns about. `src/json/` was audited in full and found
  clean — RFC 8259's whitespace set is applied consistently, and the one
  byte-for-byte duplicated predicate found there (`needs_json_yaml_quoting` /
  `needs_yaml_quoting`) is currently correct in both copies because a
  companion control-character scan independently catches tabs.

- **YAML explicit non-scalar keys silently dropped the entry** (#172,
  resolved by drift): `? - a\n  - b\n: value` used to load as `{}`, losing
  both the key and the value, silently — no error, well-formed JSON out. Not
  fixed by a dedicated change: #325's rewrite of `parse_explicit_key` to
  delegate the key side to `parse_sequence_item` fixed the key parsing, and
  #429 (closing #346)'s fix for `parse_explicit_key`'s mid-line return fixed
  the value surviving in nested positions — together they resolved every
  shape discussed on the issue. Re-verified against `yq` v4.53.3 across nine
  shapes (block-sequence, flow-sequence and flow-mapping keys; with and
  without a value; at top level, as a sibling of other entries, and two in
  one mapping), all matching byte-for-byte; `yq` has no way to render a
  non-scalar key, so both sides collapse it to `""` and the divergence is
  now only in that (expected) rendering, not in whether the value survives.
  New regression coverage locks this in:
  `test_explicit_non_scalar_key_at_mapping_level` in `src/yaml/light.rs`,
  three `tests/yq_cli_tests.rs` cases, and a new
  `explicit_key_non_scalar_{compact,pretty}` golden pair. One shape stays a
  known, unrelated divergence — a *same-line* flow collection key (`? []: x`)
  — documented in `docs/compliance/yaml/limitations.md`. Building the new
  golden pair's two-duplicate-keys case also surfaced a distinct bug, filed
  separately as #442: pretty-printed JSON silently drops a duplicate mapping
  key regardless of how the duplicate arose (compact output is correct),
  tracked in the known-failures manifest rather than blocking this fix.

- **YAML duplicate mapping keys resolved `.key` to the first occurrence
  instead of the last** (#174): `a: 1\na: 2` gave `.a == 1`; both `yq`
  (mikefarah, which reads the last entry for a path lookup while otherwise
  passing duplicate keys through unmerged) and YAML 1.2 want the last.
  The parser already indexed both keys — iteration (`.[]`) and `keys` both
  see both entries — only `YamlFields::find`, the name-based lookup behind
  `.key` field access (used by both the JSON-shaped and generic/cursor
  evaluators), returned on the first match instead of keeping the last one
  seen. (`to_entries` collapsed duplicate keys the same way — fixed by #443,
  above. JSON/`-o=json` pretty-print output still collapses them via the
  same owned-value conversion in `to_owned` — a separate, still-open gap,
  tracked as #442.)

- **The strict YAML validator accepted an alias to an unknown anchor** (#404):
  `succinctly yaml validate` passed `a: *nope`, which `yq` rejects with
  `unknown anchor 'nope' referenced`. Since #372 taught the default loader the
  same rule, the *opt-in strict* mode had become the laxer of the two — `syq`
  refused the document while `yaml validate`, documented as the yq-conformance
  gate, gave it a clean exit 0, so a CI check built on the validator stayed
  green on input `yq` refuses. The validator now tracks the anchor names a
  document defines and rejects an alias naming one that is not in scope, in
  every position an alias can occupy (value, sequence item, implicit key,
  explicit key, and both flow collections — the flow ones had no anchor/alias
  handling at all), with a new `YamlValidationErrorKind::UnknownAnchor`.

  Rejecting a `*` first requires knowing it *starts a node* rather than sitting
  inside a scalar: `a: rm *.tmp` and `a: text` continued by `  *notanalias` are
  strings, and `yq` loads both. The scanner now tracks where a node can begin —
  after a `:`/`-`/`?` indicator, after `[`/`{`/`,`/`:` in flow, and at the start
  of a line that is not the continuation of an open plain scalar — and checks
  only there. Anchor *registration* stays deliberately permissive, because an
  extra name can only make the check accept more, never reject valid input: a
  `&foo` that is really scalar content still satisfies a later `*foo`, as does
  an anchor the loader declines to record (`&x` alone on a `---` line, #372).
  Anchor scope is not reset at a document boundary, since `yq` and the loader
  both resolve an alias against an earlier document's anchor.

  Names now take their extent from `simd::parse_anchor_name`, the definition the
  loader scans with, rather than the validator's own — which stopped only at
  whitespace, read `*nope: v`'s name as `nope:`, and so would have rejected the
  valid `&a: 1` / `b: *a`. See the #106 note in `CLAUDE.md` on predicates that
  diverge silently.

  The validator remains `no_std`, but now allocates on the success path for a
  document that defines anchors, whose names it must remember.

  No YAML Test Suite manifest movement, for the reason #372 gives: no case in
  the suite contains an alias to an anchor that is not in scope.

- **The AVX2 anchor-name scanner treated every `:` as a terminator, truncating
  names on x86_64** (#453, found while fixing #404's CI): `parse_anchor_name_avx2`
  (`src/yaml/simd/x86.rs`) is the AVX2 kernel behind `simd::parse_anchor_name`,
  the function both the loader and, since #404, the strict validator use to
  compute anchor/alias name extents. It stopped at *every* `:`, unlike the
  scalar reference and the NEON kernel, which correctly stop only at a `:`
  followed by whitespace (a bare `:` is a legal anchor-name character). On
  x86_64 with AVX2, this silently truncated a name of at least ~32 bytes
  remaining in the buffer that contains such a colon — e.g. the YAML Test
  Suite's `W5VH` case (`&:@*!$"<foo>: scalar a` / `*:@*!$"<foo>:`) computed an
  *empty* anchor name instead of its real 11 bytes. This is a pre-existing bug
  in the already-shipped "P4 Anchor/Alias SIMD" optimization, not something
  #404 introduced; it went unnoticed because nothing previously compared a
  registered anchor name against later text by content, which is exactly what
  #404's new `anchor_in_scope` check does, surfacing the truncation as a false
  `unknown anchor` rejection on x86_64 only. `parse_anchor_name_avx2` now
  mirrors the NEON kernel: SIMD flags only whitespace/flow-indicators as
  definite terminators, and a candidate colon is resolved by checking the
  actual next byte, falling back to the scalar scanner for the remainder when
  the colon turns out to be a name character. New differential test
  `test_parse_anchor_name_avx2_matches_scalar_around_colons` in
  `src/yaml/simd/x86.rs` pins colon-then-whitespace, colon-then-name-char, a
  colon run, colon-before-flow-indicator, and a colon crossing a 32-byte chunk
  boundary against the scalar reference.

- **jq's regex builtins and `endswith` kept the pre-#356 wording after #356
  fixed their siblings** (#393): `test("a")` and `startswith("a")` were
  probed and now report jq's sentences, but `match`, `capture`, `scan`,
  `splits`, `sub` and `gsub` — the rest of the family that shares `test`'s
  "input is not a string" refusal — still said `expected string, got number`
  instead of `number (1) cannot be matched, as it is not a string`, and
  `endswith` still said it for the identical condition `startswith` had
  already been fixed to word as `endswith() requires string inputs`. On the
  argument side, `startswith(1)`, `endswith(1)`, `split(1)` and `test(1)`
  (plus `match(1)`/`capture(1)`, the same bug at the same sites) put an
  argument's *role* — `got non-string`, `got pattern` — where
  `EvalError::type_error`'s second slot means a type name, reading as types
  that do not exist; jq's actual wording for the pattern argument
  (`number not a string or array`) is a third shape entirely, now carried by
  a new `EvalError::not_string_or_array` constructor. All of these route
  through the named constructors #356 introduced rather than the generic
  `type_error` fallback, and the probe corpus gained one entry per sibling
  (156 probes, up from 143) so a family member fixed in isolation cannot
  drift again — see "One sentence covers a family, so probe the whole
  family" in `docs/compliance/jq/limitations.md`.

- **`yq-locate` reported a short byte range for a plain scalar containing `#`**
  (#411): `a#b: 1` loads as `{"a#b":1}` — YAML's `ns-plain-char` admits a `#`
  that is not preceded by white space — but `syq-locate --offset 0` reported
  the byte range `[0, 1]`, covering only `a`. `find_scalar_end`, the cursor's
  re-derivation of a scalar's extent used by `yq-locate` and `at_offset`,
  broke on every `#` unconditionally; the parser's own derivation
  (`parse_unquoted_key` / `parse_unquoted_value_*`), which `yq` reads from,
  already had the white-space guard right. Same fourth copy of the #106
  story as #370 (a tab) and #381 (a continuation-line tab) — a second
  "where does this scalar end" that drifted from the first, this time
  triggered by `#` rather than a tab. Affects values as well as keys (`k:
  a#b`). `find_scalar_end` now shares the guard its dead sibling
  `find_plain_scalar_end` already had.

- **A tab before `#` did not start a comment in a plain YAML key** (#410):
  `a\t# c: d` loaded as `{"a\t# c":"d"}`, folding the comment text into the
  key, where `a # c: d` (a space in the same position) already raised
  `KeyWithoutValue`. `s-b-comment` requires `s-separate-in-line` before the
  `#`, and `s-white` is a space *or a tab* — but `parse_unquoted_key`'s
  comment guard tested only for a preceding space. The same omission #370
  fixed for this function's trailing trim, thirty lines away; the value-side
  equivalents were already tab-aware. A `#` *not* preceded by whitespace is
  unaffected and stays key content, as before: `a#b: value` is still the key
  `a#b`.

  Unlike #370, this turns a previously-accepted document into an error, so
  the fix was checked against the same corpora #370's CHANGELOG entry names:
  the 402-case YAML Test Suite and `tests/data/yq-golden/` contain exactly
  one tab immediately before a `#`, and it sits in value position (already
  handled correctly) rather than in a key — no fixture moved.
  `tests/yaml_tab_comment_tests.rs` covers the key-side guard now.

- **A pipe continuing after a freshly-computed value collapsed multi-output
  results into one array** (found while implementing #396): `eval_owned_pipe`
  — reached whenever the left side of `|` is a computed value rather than one
  navigated straight out of the document (an `as` binding, arithmetic, object
  or array construction, and so on) — called `eval_owned_expr`, whose own doc
  comment says it intentionally collapses multi-output into a single array
  (correct for `reduce`/`foreach`, wrong here). `. as $doc | $doc | paths` on
  `{"a":{"b":1}}` produced `[["a"],["a","b"]]` instead of streaming `["a"]`
  and `["a","b"]` separately — reproducible with any multi-output filter
  (`paths`, `range`, `.[]`, and the new `tostream`), not just the new
  builtins. Fixed by switching to `eval_owned_input`, a sibling helper already
  written for exactly this (its doc comment: `eval_owned_expr` "is wrong for a
  filter that is allowed to fan out"); the caller's `match` already had a live
  `ManyOwned` arm anticipating it. No test regressions across the full suite.

- **`//` discarded a left-hand error instead of propagating it** (#377):
  `eval_alternative` treated a left-hand `Error` the same as `None` and fell
  through to the right side, so `error("x") // 3` and `.a // 3` (on a
  non-object) silently produced `3` instead of raising, where jq 1.7.1
  raises. Split out of #160, which deliberately left this arm unchanged to
  keep that fix scoped to the multi-output bug. `//` now propagates a
  left-hand error; `.a? // 3` is unaffected since `?` already resolves to
  `None` before reaching the operator. The whole-stream error model this
  paragraph originally flagged here — `(1, error("x")) // 2` yielding `2`
  instead of `1` then the error — is fixed above (#400, #494).

- **YAML alias used as a flow-mapping key rendered as the empty string** (#405):
  `{&x k: 1, *x: 2}` loaded as `{"k":1,"":2}` where `yq` gives `{"k":1,"k":2}`,
  and likewise for a flow mapping nested in a block one or in a sequence. The
  alias resolved — the key node it resolved *onto* was the wrong one. The
  flow-mapping key site opens the key's BP node before reading the key, so the
  alias must be recorded against that node; it instead called `parse_alias`,
  which opens and closes a node of its own, so the alias edge bound to a node
  *below* the key and the key itself was left with no extent. It now shares
  `record_key_alias` with the block and compact key sites — all three
  key-alias sites now share one definition, as the three key-*anchor* sites
  already shared `record_key_anchor`. This was the one key
  position #372 left inconsistent with itself: a miss went through
  `parse_alias`'s lookup and errored, while a hit silently rendered `""`.
  Unaffected: an alias key resolving to a sequence or mapping still stringifies
  as `""`, which is the complex-key rule and matches `yq`.

- **`tojson`, `@json` and `sjq`'s printer escaped C1 control characters** (#385):
  the escaping branched on `char::is_control()`, which is true for the C1 range
  U+0080–U+009F as well as C0. JSON only requires escaping below U+0020 and jq
  escapes nothing else there, so a string holding U+0085 (NEL) round-tripped
  through jq as its two raw UTF-8 bytes and through succinctly as a
  six-character backslash-u escape. `tojson` additionally emitted the long forms
  where jq emits `\b` and `\f`, and `JqValue`'s object keys escaped only `"` and
  `\`, so a control character in a key produced invalid JSON.

  The predicate is now `< 0x20 || == 0x7f`. Not `is_control()`, and not the bare
  `< 0x20` the issue proposed — jq escapes DEL, which succinctly had right only
  by accident of `is_control()` covering it, and which a naive narrowing to C0
  would have broken in the other direction.

  Behind it, the five hand-written JSON string writers are now two, in
  `succinctly::jq::escape`: one per convention, because jq and mikefarah/yq
  genuinely disagree at `0x08`, `0x0c` and `0x7f`. A differential test pins that
  disagreement set exactly — asserting only that the two agree would pass if
  both broke the same way. Six oracle-captured cases under
  `tests/data/jq-golden/cases/escape_controls_*` cover the corpus end to end.
  yq's output is unaffected; see the #106 lesson in `CLAUDE.md` on predicates
  that diverge silently.

- **jq: `delpaths` no longer lets one deletion shift the array under the next**
  (#398). It deleted left to right, sorting only by path length, so two paths of
  equal length kept the caller's order and the first deletion moved the element
  the second named: `[10,20,30,40] | delpaths([[0],[2]])` gave `[20,30]` where
  jq gives `[20,40]` — the wrong element, silently — while the same two paths
  written the other way round happened to come out right. jq's ordering rule is
  now implemented properly: the path list is sorted in jq's total value order,
  the paths are grouped by shared prefix, and every key that ends at one level
  is removed in a single pass, so each index resolves against the length the
  container had before any sibling went. That last part is what a per-path loop
  cannot reproduce, and it is why `delpaths([[-1],[-2]])` is `[10,20]` rather
  than the `[10,30]` deleting one at a time gives. Duplicates collapse (a
  repeated path deletes once), a shorter path shadows its own extensions
  (`delpaths([[0],[0,1]])` takes the subtree without trying to index into it),
  the empty path deletes the document wherever it appears in the list, and a
  key whose child is edited keeps its position rather than moving to the end.
  Seven cases captured from jq 1.7.1 pin the behaviour, and both evaluators are
  checked for agreement. Deleting many array elements is also **~50x faster**
  (30k of 60k: 1.03s → 0.02s) and many object keys **~90x** (30k of 60k: 4.4s →
  0.05s), both having been quadratic. `delpaths` silently no-opping where jq
  raises was fixed separately (#415, #395), and `del()` with negative computed
  indexes has the bug this fixes (#424).

- **`bsearch` reported absent containers as found, and returned an object when
  absent** (#384): two defects in the same twenty lines of `src/jq/eval.rs`.
  `bsearch` had its own comparator with arms for null, bool, numbers and string
  and none for `(Array, Array)` or `(Object, Object)`, so two containers fell
  through to the cross-type rank comparison and compared *equal* — binary search
  over containers returned whichever midpoint it landed on and claimed a match
  for a value that is not present, which a caller could not even detect by
  testing for the not-found marker. That comparator is gone; `bsearch` now uses
  `compare_values`, the one `sort` already uses, so the two cannot disagree
  about a pair (the #106 lesson in `CLAUDE.md`). Separately, the absent case
  returned `{"index": n}` where jq returns the negative insertion point
  `-1 - n`, so jq's idiomatic `if . < 0 then … end` raised a type error instead
  of taking the branch. The search itself is now jq's own loop from
  `builtin.jq` rather than `Vec::binary_search_by`, whose choice among equal
  elements is documented as unspecified and differs from jq's; succinctly now
  matches the oracle on duplicates too. Both evaluators are covered — the CLI
  reaches `bsearch` through the generic evaluator's fallback — and seven new
  pinned-jq golden cases exercise containers, absence, duplicates and the empty
  array.

- **jq: a repeated key no longer deletes twice** (#360). `[1,2,3] | del(.[(0,0)])`
  removed elements 0 and 1, yielding `[3]`; resolved paths are now deduplicated
  before deletion, as jq's `delpaths` does, giving `[2,3]`.

- **jq: a NaN index no longer reads or writes element 0** (#360). `f64 as i64`
  maps NaN to `0`, so `[10,20,30] | .[nan]` returned `10` and `.[nan] = 5`
  silently overwrote the first element. Reads now yield `null` as jq does, and
  writes (`= v`, `|= f`, `del`, `path`) report `Cannot set array element at NaN
  index`.

- **jq: `?` no longer suppresses errors raised by a computed key, or by the
  expression being indexed** (#360). The enclosing optional flag was passed into
  both halves of `E[K]`, where jq's `gen_index_opt` makes one opcode optional and
  compiles both halves normally. So `{"k":"a","a":1} | [.. | .[.k]?]` returned
  `[1]` where jq fails with `Cannot index string with string "k"`, and
  `"str" | .a[length]?` returned nothing where jq fails with `Cannot index string
  with string "a"` — the latter making `?` mean two different things depending on
  whether the key folded to a constant, since `"str" | .a[0]?` raised all along.
  `?` now covers the indexing only, matching jq; `try`/`catch` still catches the
  error.

- **A tab between a plain YAML key and its `:` was folded into the key** (#370):
  `a\t: 1` loaded as `{"a\t":1}`, so a `.a` lookup missed a key the document
  plainly spells `a`. YAML puts that white space outside the key —
  `ns-s-implicit-yaml-key ::= ns-yaml-key(c) s-separate-in-line?`, and
  `s-white` is a space *or a tab* — but the key's trailing trim was the one
  place in the parser that listed only the space. Every sibling trim (the value
  path, and all three flow-context sites) already included the tab. A tab
  *inside* a plain scalar is unaffected and stays content, as
  `nb-ns-plain-in-line` requires: `a\tb: 1` is still the key `a\tb`.

  The same omission had a second home. A scalar's extent is derived twice by
  copies that never consult each other: the parser's, stored in the index and
  reported by `yq`, and `find_scalar_end`'s, re-derived from the text and
  reported by `yq-locate` and `at_offset`. Fixing only the parser would have
  left `syq` printing `a` while `syq-locate` still reported the byte range
  `a\t`. The cursor copy also dropped the tab from its *terminator* set, which
  broke a document with no trailing tab at all: `a:\t1` is legal YAML whose key
  located as a range running to end of input. Both are now spelled the way
  `parse_unquoted_key` spells them. See the #106 lesson in `CLAUDE.md` on
  predicates that diverge silently — this is the third copy of that story.

  No fixture moved: neither the YAML Test Suite corpus nor
  `tests/data/yq-golden/` contains a tab adjacent to a colon, which is why the
  shape survived this long. `tests/yaml_tab_separation_tests.rs` covers it now,
  pinned by output and by located byte range, and is the separation half of the
  split `tests/yaml_tab_indentation_tests.rs` (#173) draws — there a tab before
  block structure is illegal indentation, here a tab before an indicator on the
  same line is legal separation.

- **YAML flow context silently absorbed tags as scalar text** (#369): block
  context has always rejected `!` via `check_unsupported`, but the flow-context
  scalar readers fell through to the plain-scalar path, so `a: [!!str x]` yielded
  the *string* `"!!str x"` rather than an error — silently wrong data instead of
  a refusal. All four flow positions that reach a scalar reader are now gated
  (sequence item, mapping value, mapping key, and the explicit `? k : v` form).
  A `!` *inside* plain scalar content is untouched and remains ordinary text, as
  YAML requires — only a leading `!` is an indicator. Tag *support* is still
  #224; this only makes the two contexts fail the same way.
- **YAML alias to an unknown anchor silently yielded `null`** (#372): an alias
  naming an anchor not in scope — a forward reference, or one never defined —
  was dropped rather than resolved, leaving the node to render as `null`, or as
  an empty string where the alias was a key. YAML 1.2 §7.1 requires an alias to
  name a *previous* anchor, so this is invalid input rather than a value; it is
  now refused at build time, as a cyclic alias always has been. Every position
  an alias can appear in is covered: values (block, flow sequence, flow mapping,
  block sequence item, compact mapping entry, document root) and keys (block,
  compact, flow, explicit `?`).

  Two of those positions did not resolve aliases *at all*, so rejecting a
  lookup miss would have turned valid YAML into a parse failure. A compact
  mapping entry inside a block sequence item never registered an anchor on its
  value, and neither did a document-root value, so `- name: &n web` followed by
  `image: *n` — the shape a Kubernetes manifest writes — resolved to `null`
  before and would have become a hard error. Both now go through the same
  anchor/alias handling as every other value, so those aliases resolve properly
  rather than merely failing loudly. `? *a` as an explicit key likewise resolves
  now instead of producing an empty key. Aliases that already resolved are
  unchanged.

  One document-root form was left deliberately anchorless while the underlying
  document split remained: `&x` alone on the `---` line, with its node on a
  following line. The anchor was consumed without being recorded, so a later
  `*x` was the error it should be rather than a silent `null`. #407 below
  removed the split, and the anchor now binds to the node it names.

  No YAML Test Suite manifest movement: no case in the suite contains an alias
  to an anchor that is not in scope, so none could flip. The three `lax:anchors`
  entries that remain (4JVG, CXX2, GT5M) are anchor *placement* and
  *duplication* rules, which this does not touch.

  **Breaking**: adds a `YamlError::UnknownAnchor` variant, so exhaustive
  `match`es on the public `YamlError` gain an arm.
- **YAML `--- &x` with its node on the next line split one document into two**
  (#407): an anchor alone on the `---` line yielded `null` followed by the real
  document, and the node the anchor should have named went unanchored —
  `printf -- '--- &x\na: 1\n' | syq -o json -I0 .` printed `null` and
  `{"a":1}`. Per YAML 1.2 the `&x` property attaches to the document's root
  node, so that is one document, `{"a":1}`, with `x` bound to it. The same
  input without `---` was always correct.

  The cause was two dispatchers for one grammar: the content of a `---` line
  went through a hand-rolled partial copy of the block-context dispatch in
  `parse_document_line`. The copy opened an empty node for the anchor to name,
  and a node at document root *is* a document. The copy is gone; both entry
  points now share one `parse_block_node`, and a new differential
  (`tests/yaml_document_start_line_tests.rs`) asserts that `--- X` and a bare
  `X` parse identically — the test the missing definition never had.

  Five more shapes were diverging the same way and are fixed with it:
  `--- &x` over a block sequence (suite case `FTA2`, which moves out of the
  known-failures manifest), the indented `--- &x` over `  a: 1`,
  `--- &x {a: 1}` (which read the `:` *inside* the flow mapping and gave
  `{"":"1}"}`), `--- ? a` (which gave the literal `"? a"`), and `--- "a": 1`
  (which gave `"a"` and `1`).

  Two consequent behaviour changes, both matching the `---`-less form exactly:
  `--- &x\na: 1\nb: *x` and `--- &x\n- 1\n- *x` now report a *cyclic* alias
  rather than an unknown one, because the anchor binds; and
  `--- &x\na: 1\n---\nb: *x` resolves the alias across the document break.
  A tag on the `---` line is still not rejected the way a bare one is — that
  asymmetry is #224's, and is now noted at the one place it lives.

- **An anchor at the end of a compact mapping entry's line swallowed the nested
  value** (#406): `- k: &a` followed by an indented block read that block as one
  folded plain scalar, so `- k: &a` / `    b: 1` came out as `[{"k":"b"}]` and
  the sequence form `- k: &a` / `    - 1` as `[{"k":null}]` — well-formed but
  wrong documents, with no error raised. The same entry without the anchor was
  always right. `parse_compact_mapping_entry` asked whether the value was on
  this line *before* anything consumed the `&a`, so the answer was always "yes"
  and `parse_inline_value`'s multi-line plain-scalar rule ran on what was
  effectively an empty remainder. It now consumes the anchor first and then
  decides, which is the order every other block-context value site already used
  — `parse_mapping_entry`, `parse_sequence_item_inner` and
  `parse_explicit_value` — and a test pins all four against one input shape so
  the outlier cannot come back. A flow collection on the following line
  (`- k: &a` / `    [1, 2]`, previously the string `"[1, 2]"`) is fixed by the
  same change, and aliases now propagate the nested collection rather than the
  collapsed scalar. A block scalar whose `|` or `>` sits at the *same* indent as
  the key (legal in YAML, unlike a plain scalar or flow collection there) is a
  separate, pre-existing bug that this does not address: `next_indent ==
  indent` only treats a `-` as "the value continues here", so `|`/`>` there is
  still misread as null on both the anchored and non-anchored forms, where yq
  gives `""`. A block scalar indented deeper than the key — the shape `#406`
  is actually about — is unaffected by that gap and now agrees with yq for
  every chomping/indicator variant.

  No YAML Test Suite manifest movement: no case in the suite has this shape.

- **jq error-message value previews escaped C1 control characters** (#358):
  a preview built from `OwnedValue::to_json` escapes every
  `char::is_control()`, which includes U+0080–U+009F, so a string containing
  U+0085 previewed with a six-character
  backslash-u escape where jq emits the two raw UTF-8 bytes. Previews now go
  through the streaming JSON writer, which already matched jq here. `tojson` and
  `@json` still over-escape — the same `to_json` path, but a wider behaviour
  change than #358 should make, so it is tracked separately as #385.
- **jq `contains`/`inside` answered `false` for operands that cannot be
  compared** (#358): `1 | contains("a")` and `1 | inside([1])` returned `false`
  where jq raises
  `number (1) and string ("a") cannot have their containment checked`. Silent —
  a filter asking "is this string in that string" got a plausible `false` when it
  had in fact been handed a number. **Behaviour change**: those filters now
  error, so a query that relied on the `false` will stop producing output.
  Only the *outermost* pair of operands is screened, matching jq exactly: a
  mismatch nested inside a container is still `false`
  (`[1,"a"] | contains(["a",2])`). The screen is on jq's *kind*, not its type
  name, which cuts both ways: integers and floats are one kind, so
  `[1,2,3] | contains([1.0])` stays `true`, but `true` and `false` are two kinds
  that share the name `boolean`, so `true | contains(false)` errors with
  `boolean (true) and boolean (false) cannot have their containment checked`
  while `true | contains(true)` stays `true`. The new
  `EvalError::containment_check` reproduces jq's message including its value
  preview, which truncates a dump longer than 14 bytes to 11 bytes plus `...`
  (`string ("abcdefghij...) and number (1) …`); unlike jq's `strncpy` it cuts on
  a `char` boundary rather than emitting a split UTF-8 sequence. Unlike jq it
  also stops serialising once the answer is settled, so previewing a mismatched
  100 MB operand copies 14 bytes instead of dumping the whole document.
  `succinctly yq` gets the fix too, since its evaluator delegates containment to
  this one.
  Two known gaps, both pinned by tests rather than fixed: uncaught, it still
  exits 0 rather than jq's 5 (#355), and a number in the preview reads back
  canonicalised rather than as its source literal — jq's `number (1E+100)` is
  our `number (10000000000...)` — because `OwnedValue` does not carry the
  literal, a limitation `1e100 | tostring` already shows and the streaming
  identity path does not share (#387).
- **jq `//`, `and` and `or` collapsed multi-output operands** (#160): all three
  are generators over their operands' *streams*, but each inspected only the
  first output. `//` decided truthiness from `vs.first()` and then returned the
  left stream unfiltered, so `(null,1) // 3` gave `3` where jq gives `1`, and
  `(1,false,2) // 3` gave `1 false 2` where jq gives `1 2`. `and`/`or` funnelled
  each operand through `result_to_owned`, keeping the first output and turning
  an empty stream into `Error("empty result")` — so `(true,false) and true` gave
  one boolean where jq gives two, and `empty and true` printed a spurious
  `jq: error: no value` where jq is silent. `//` now emits every non-`null`,
  non-`false` output of its left side and evaluates the right only when there
  are none; the right side's outputs are still emitted unfiltered, which is what
  makes the left-associative chain `a // b // c` filter `b`'s stream. `and`/`or`
  fan out over both operands with the left as the outer loop, still
  short-circuiting per left output so `false and error("x")` yields `false`
  without raising. A `break` in either operand now reaches its label instead of
  becoming `Error("break $l not in label")`. Filtering keeps document-derived
  values borrowed, so the zero-copy path survives. `succinctly yq` gets the fix
  too, since its evaluator delegates all three operators to this one. Ten new
  pinned-`jq` golden cases cover the family, and the known-failures manifest
  drops to two entries. **Not fixed**: `if`/`select` still collapse a
  multi-output condition to its first output (#378, sibling of #354). `//`
  also suppressed left-hand errors rather than propagating them; fixed
  separately below (#377). The deeper whole-stream `Error`/`Break` model this
  paragraph originally flagged here — `(1,error("x")) // 2` yielding `2`
  instead of `1` then the error, and `label $out | ((true,true) and (1,
  break $out))` yielding nothing instead of `true` — is fixed above (#400,
  #494).
- **YAML: a tab after spaces in indentation was folded into the key** (#173):
  the loader rejected a tab only at column 0 and treated a tab following one or
  more spaces as start-of-content, so `a:\n \tb: 1` loaded as
  `{"a":{"\tb":1}}` rather than being refused. YAML forbids a tab in
  indentation, but a tab is only *indentation* when block structure follows it —
  before a plain scalar it is separation and legal (`foo:\n \tbar`, and Test
  Suite case UV7Q, "Legal tab after indentation"). The strict validator already
  drew that distinction, so the fix promotes its `line_is_structural` predicate
  to the `yaml` module root and has both consult it, rather than adding a second
  spelling of the rule. Loader-only reject conformance goes 11/94 → 12/94 (case
  DK95/06, which the validator already caught); the combined figure stays 70/94.
  Sharing the predicate also fixed two false positives on the validator side,
  where the `:` scan read a `:` that was not a value indicator: a tab before a
  *flow* node is now separation, so `\t{a: 1}` is accepted as `\t{}` already
  was; and the scan now skips quoted scalars and comments, so `a:\n \t"x: y"`
  and `a: 1\n \t# c: d` are accepted while `a:\n \t"b": 1` — a quoted *key*, so
  really indentation — is still refused.
- **`BitVec` counted 1-bits that lie past `len`** (#321): `from_words` documents
  that `len` may be less than `words.len() * 64`, but the constructor masked
  `words[words.len() - 1]` — the wrong word as soon as `words` is longer than
  `len` needs — and skipped masking entirely when `len % 64 == 0` or `len == 0`.
  Surplus 1-bits therefore stayed in the cached `ones_count`, so
  `BitVec::from_words(vec![u64::MAX, u64::MAX], 64)` reported 128 ones for a
  64-bit vector, `rank1(i >= len)` returned that inflated count, and
  `count_zeros()` panicked with "attempt to subtract with overflow" in debug (and
  wrapped in release). It now clears the tail of the word holding bit `len - 1`
  and zeroes every word after it. Found while covering `select1`'s
  "position past `len`" branch, which existed only because of this.
- **Double free in `RankDirectory`'s cache-aligned builder** (#321):
  `CacheAlignedL1L2Builder::build()` freed its allocation and then returned
  without `mem::forget(self)`, so `Drop` freed the same pointer again — an
  immediate abort. Only the "capacity allocated but nothing pushed" path did
  this; the two paths that transfer ownership always forgot `self`. That path is
  unreachable from any public API today (`RankDirectory::build` returns early for
  empty input, so a builder with capacity always gets at least one push), which
  is why it was never hit. The explicit free is gone; `Drop` now owns the
  release.
- **jq evaluator error messages did not match jq's wording** (#356): `1 | .foo`
  reported `expected object, got number` where jq says `Cannot index number with
  string "foo"`, and `"a" | tonumber` said `cannot convert 'a' to number` against
  jq's `Invalid numeric literal at EOF at line 1, column 1 (while parsing 'a')`.
  Cosmetic until #158 landed; now that `catch` binds the raised value, a filter
  can read the text, so `try f catch (if test("Cannot index") then … end)` — a
  real jq idiom — behaved differently here. All but seven probed messages are
  now byte-identical to jq-1.7.1 across **both** evaluators, covering indexing,
  iteration, arithmetic, `keys`/`length`/`sort`/`has`/`test`/`contains`, and
  `tonumber`/`fromjson`. Root cause was that every message was inlined at its
  raise site (~300 of them), with no shared definition — which is also how the
  two evaluators drifted from *each other*: they reported `expected array or
  object, got number` versus `cannot iterate over number` for the same
  condition, and had two different `tonumber` messages. `EvalError` moves to a
  new `src/jq/error.rs` with one named constructor per jq sentence shape, and
  `tonumber`'s string handling is now a single shared function. Two coupled
  defects fell out: `.[] = 1` reported `cannot use expression as assignment
  target` because `set_path` had no iterate arm at all (it now assigns through
  arrays and objects like jq), and `tonumber` classified `"0x10"` as valid JSON
  because the internal parser stopped at the first value instead of requiring
  the whole input. The probes that remain are behaviour and parser gaps, not
  wording — a slice is not a path component (#366), and
  `.[null]`/`.[true]`/`.[{}]` do not parse (#360) — each on record in
  `tests/data/jq-error-known-divergences.txt`. **API**: `EvalError` gains
  jq-shaped constructors and `succinctly::jq` re-exports a new `BinOp`;
  `EvalError::type_error` stays for the sites jq has no counterpart for.
- **jq `setpath` built a container on a scalar instead of refusing to index it**
  (#359): `1 | setpath(["a"]; 1)` discarded the input and returned `{"a":1}`
  where jq reports `Cannot index number with string "a"`. Its siblings (`.a = 1`,
  `.a |= …`, `del(.a)`, `getpath`) already agreed with jq; only this one
  auto-vivified. `null` is now the only value vivified — at the root and at every
  depth — and a real container indexed with the wrong kind of key is refused too
  (`{} | setpath([0]; 1)`). Three defects fell out of the same walk: a negative
  index that stays negative after resolution is jq's `Out of bounds negative
  array index` rather than `(len + idx) as usize` ≈ 1.8e19 nulls; a float index
  truncates toward zero as jq's does instead of being ignored; and writing to an
  existing object key keeps the key where jq keeps it, rather than moving it to
  the end via `IndexMap::shift_remove`. `=` and `|=` now share the negative-index
  sentence. Assigning through a slice path element remains unimplemented (#366).
- **jq `tonumber` and `fromjson` panicked on a truncated container** (#359
  review): `"{" | tonumber` and `"{\"a\":1," | fromjson` indexed one byte past
  the input while looking for an object key, panicking inside the JSON parser
  instead of raising a catchable error — a library panic takes the embedder's
  process with it. `setpath` had the same shape: `null | setpath([1e30]; 9)`
  asked `Vec::resize` for 9.2e18 elements and died on `capacity overflow`; it now
  refuses with `Cannot grow array to <n> elements`, while every length that fits
  in memory still pads as jq does.
- **jq `fromjson` accepted trailing garbage** (#359 review): it read the first
  JSON value and dropped the rest, so `"0x10" | fromjson` returned `0` and
  `"1 2" | fromjson` returned `1` where jq errors on both. It now shares the
  whole-input parse `tonumber` was given, and `"0x10"` reports jq's sentence
  verbatim.
- **jq builtins derived from the same definition worded their errors
  differently** (#359 review): jq builds many builtins out of others, so one
  sentence is owed by a whole family — but the #356 sweep fixed only the member
  each probe named. `1 | with_entries(.)` said `number (1) has no keys` while
  `1 | to_entries` beside it still said `expected object, got number`;
  `ascii_downcase` reported `explode input must be a string` and `ascii_upcase`
  did not; and `"abc" | indices(1)`, `index(1)`, `rindex(1)` all answered
  `expected string, got pattern` — naming an argument where jq names a type —
  instead of `Cannot index string with number`. The three string searches now
  share their refusals (`non_string_pattern` and `unsearchable_input`) rather
  than keeping three copies each, and the corpus carries a probe per family
  member so the next member cannot drift alone. Measuring the searches over
  every input type also turned up behaviour the wording had hidden: jq reaches
  `_strindices` only for a string pattern and answers `null` where there is
  nothing to search, so `null | index("a")` and `{} | index("a")` are values,
  not errors, and all 24 cells of that matrix now match. `to_entries` also gained jq's array behaviour (`[1,2] | to_entries` is
  `[{"key":0,"value":1},{"key":1,"value":2}]`), without which the corrected
  sentence would have claimed an array has no keys where jq answers with a
  value.
- **jq `getpath` rejected a float array index that jq accepts** (#359 review):
  `[1,2,3] | getpath([1.5])` errored with `Cannot index array with number`
  where jq gives `2`. #359 taught `setpath` jq's index resolution — truncate
  toward zero, count a negative back from the end — but left the read path
  behind, so the two disagreed in-tree about the same path element. Reads now
  resolve identically, differing only where jq differs: an index that reaches
  no element is `null` rather than an error, which covers NaN, ±infinity and
  an overrun at either end.
- **jq `try/catch` discarded the raised error** (#158): the catch handler ran
  against the *original input* rather than the error value, so a handler could
  never see what went wrong — `try error("boom") catch .` gave `null` where jq
  gives `"boom"`, and `catch "c:\(.)"` interpolated the input. `error(v)` also
  flattened non-string payloads to their JSON text, so `try error({a:1}) catch .`
  could not yield an object for the handler to index. Root cause was that
  `EvalError` modelled an error as a bare message string, leaving nothing for
  `catch` to bind. It now carries the raised value alongside the message;
  internal errors (type errors and friends) keep raising their message as a
  string, which is how jq models them. The same commit fixes a coupled defect:
  bare `error` raised the literal `null` instead of the input value, which only
  looked correct because `catch` was reading the input anyway — fixing the
  handler alone would have regressed it. Handlers that fan out (`catch (., .)`)
  now keep every output instead of collapsing into one array. `succinctly yq`
  gets the fix too, since its evaluator delegates `try` to this one. Six new
  pinned-`jq` golden cases cover the family (string, object, `null`, bare,
  interpolated and multi-output payloads), and the known-failures manifest
  drops to four entries. **API**: `EvalError` gains a public
  `value: Option<OwnedValue>` field and a `from_value`/`payload` pair;
  additive, but downstream struct-literal construction would need updating.
  Uncaught errors keep the existing `jq: error: <message>` form — jq's
  `(not a string)` framing and exit code 5 remain a separate divergence.
- **YAML explicit key with its `: ` on the same line** (#346): `? k: v` loaded as
  the ordinary entry `{"k":"v"}`. Per YAML 1.2 §8.2.2 the node after `? ` is
  `s-l+block-indented`, which admits a compact block mapping — so the whole
  `k: v` is a *mapping used as the key*, and the entry has a complex key (which
  `yq` renders `""`) and no value. The divergence hit every position an explicit
  key can appear, and inconsistently: `{"k":"v"}` at top level but
  `{"m":{"k":null}}` as a mapping value and `[{"k":null}]` as a sequence item.
  Silent in all three — no error, well-formed JSON out. `parse_explicit_key`
  stopped the key scalar at the `: ` and returned *mid-line*; `count_indent`
  counts spaces forward from the cursor with no line-start check, so the main
  loop re-derived that line's indent as `0` and `parse_explicit_value` closed the
  mapping it should have been filling — which is why only the top-level spelling,
  whose mapping is already at indent 0, kept its value. The fix routes the key
  through the same `parse_compact_mapping_entry` the `- k: v` sequence-item path
  uses rather than a second copy of the decision (#106), and mirrors it on the
  value indicator, which had the identical defect (`? a` / `: b: c` loaded as
  `{"a":"b"}`). That pairing is what YAML Test Suite case V9D5 needs. Enabling
  this required teaching the pending-explicit-key state which mapping *owns* it:
  a complex key is itself an open container, so the previous "the container being
  popped is a mapping" test wrote the owner's null into the key and lost the
  entry entirely. Quoted keys, continuation lines, wide indents and all three
  YAML 1.2 §5.4 line-break forms are covered, with two new pinned-`yq` golden
  cases. Flow-collection keys (`? []: x`) remain divergent and are documented in
  `docs/compliance/yaml/limitations.md`.
- **YAML explicit keys as block sequence items** (#339): `- ? k` followed by
  `  : v` loaded as `["? k","v"]` — the `? ` indicator folded into a plain
  scalar and the `: v` line became a *phantom second element*, so the sequence
  gained an item and the mapping vanished. `- ? k` alone gave `["? k"]` where
  `yq` gives `[{"k":null}]`. Silent: no error, well-formed JSON out. The same
  key was already correct at top level and as a mapping value —
  `parse_sequence_item_inner`'s dispatch simply had no `?` arm, so it fell
  through to the plain-scalar path. It now routes the item through the same
  `parse_explicit_key` the mapping-level dispatch uses, rather than a fourth
  copy of that decision (#106), which fixes every spelling at once: quoted,
  block-scalar and flow-collection keys, keys on the following line, anchored
  keys and values, further entries joining the item's mapping, and all three
  YAML 1.2 §5.4 line-break forms. Two new pinned-`yq` golden cases cover the
  family, and the `explicit-keys` bench pattern now generates the shape — no
  benchmark input contained one before, so none could have measured it.
- **CRLF and lone-CR line breaks in YAML** (#324): a `\r` was folded into every
  *plain* scalar as a trailing space, which also destroyed type resolution — a
  Windows-authored `a: 1` loaded as the string `"1 "` rather than the number `1`,
  and `a: true` as `"true "`. There was no error and no warning, and the output
  was well-formed JSON, so nothing downstream could detect it. Quoted scalars and
  LF input were unaffected, which is why the whole suite missed it: every fixture
  and benchmark input in the repo uses LF. The fix treats `\r\n` and a lone `\r`
  as line breaks throughout — plain scalar and key extents, document markers,
  blank lines, comment termination, block-scalar content and chomping, `raw_bytes`,
  and the strict validator — per YAML 1.2 §5.4. `succinctly yq` now produces
  byte-identical output for a document whichever of the three break forms it uses.
  Correctness here has a measured price on LF input: `yaml_bench` index build is
  +14.9% median on x86 (7950X) and +6.9% on ARM (M4 Pro) excluding block scalars,
  which are 8–18% *faster* on x86; end-to-end `yq` on a 1 MB document moves
  +1.8% (`.`) to +6.4% (`.[].name`). Most of that is bought back in #340 (below);
  see `docs/parsing/yaml.md` for the per-change attribution.
- **YAML anchors on sequence items whose value is a collection** (#328): `- &m`
  followed by an indented mapping was read as a multi-line plain scalar, so
  `list:\n  - &m\n    k: v\n  - *m` came out as `{"list":["k"],"v":["k"]}` — a
  well-formed but wrong document, with no error raised. The flow form
  `- &first {id: 1}` corrupted differently, swallowing the anchor into the key
  text so the alias resolved to `null`. `parse_sequence_item_inner` now consumes
  the anchor before deciding the item's node type, and `- &a k: v` binds the
  anchor to the key as `yq` does. Sequences as explicit-key values (`? k` /
  `: - &m`) route through the same parser instead of an inlined copy of its
  dispatch, so they are fixed too.
- **YAML anchors that never named a node** (#328): three further anchor-target
  bugs, found by a new whole-corpus invariant that every anchor must point at a
  node's opening parenthesis.
  - An anchor on a **flow mapping key** (`a: { &e e: f }`) bound to the value
    rather than the key, so `*e` yielded `"f"` where `yq` gives `"e"`.
  - An anchor on an **explicit value that turns out to be null** (`? e` / `: &a`)
    had nothing to point at, so `*a` resolved to the following key — and inside
    a sequence it landed on the alias's own node and raised a spurious
    `AliasCycle` error on a valid document.
  - A **block sequence at a lower indent than a mapping key** was treated as
    that key's value, leaving the key's anchor dangling.
- `YamlIndex::to_offset` no longer returns byte offsets past the end of the text
  for an out-of-range column; it returns `None`, matching `JsonIndex::to_offset` (#228)
- `jq -R -s` now yields the entire input as a single string instead of an array of per-line strings, matching jq (#176)
- `yq -R -s` now yields the entire input as a single string instead of an array of per-line strings, matching jq and `jq -R -s` (#271)
- YAML alias cycles (`a: &anchor {self: *anchor}`) are rejected at index build with the
  new `YamlError::AliasCycle` variant instead of aborting with a stack overflow when the
  value is materialized (#153). A deliberate divergence from `yq`, which accepts the same
  input and emits a depth-limited expansion. Note: exhaustive `match`es on `YamlError`
  need a new arm.
- **BalancedParens L2 excess overflow** (#188): the per-L2-block excess
  counters were `i16` and overflowed at nesting depth > 32,767 (debug panic /
  silent wrap in release). Widened to `i32` across the scalar, NEON, and
  SSE4.1 build paths; deep nesting is no longer bounded by the index.
- **BalancedParens stray-bit over-count** (#188): constructors did not mask
  1-bits above `len` in the final partial word, inflating
  `total_ones`/`rank1`/`select1`. Owned constructors now canonicalize the
  final word in place; borrowed (`from_words*`, mmap) paths mask on read.
- **SelectIndex sample overflow** (#188): `SampleEntry` counters were `u32`
  and wrapped past 2^32 set bits (~512 MB of ones); widened to `u64`.

- **jq postfix `?` was only accepted after a path expression** (#367): jq's
  grammar allows `?` (shorthand for `try f`) after any Term — `length?`,
  `keys?`, `(1)?`, `(.a)?`, `first(.[])?`, `getpath(["a"])?`,
  `setpath(["a"];1)?`, `$x?`, `.?` — but succinctly's parser only checked for
  a trailing `?` in two spots: the dot-field branch of `parse_primary`
  (`.foo?`) and `parse_index_bracket_with_optional` (`.[0]?`). Everywhere
  else a trailing `?` was left unconsumed, tripping the top-level "unexpected
  character '?'" check. Rather than patch each of the ~13 branches that
  didn't check for it individually, `parse_primary`'s whole dispatch is now
  wrapped once (renamed to `parse_primary_inner`, with a thin outer
  `parse_primary` checking for a trailing `?` after any Term); `?` was also
  added to `is_expr_terminator` so bare `.?` is recognized as identity rather
  than an attempted (invalid) field name. Unlocking `?` after arbitrary
  builtins surfaced latent divergences that were previously unreachable with
  `optional` set through real syntax: `tonumber?`, `length?`, `keys?`,
  `keys_unsorted?`, `first?`, `last?`, and `reverse?` raised a hard error
  instead of suppressing it on a value they can't operate on, because their
  `eval_generic.rs` arms (and, for `first`/`last`/`reverse`, their `eval.rs`
  counterparts too) never checked the `optional` flag they were already
  passed. All seven are now threaded through, verified row-by-row against jq
  1.7.1. Not fixed here: `null | reverse` (no `?` involved) still errors
  instead of yielding `[]` in the generic evaluator — a pre-existing,
  unrelated gap (missing `is_null()` arm) that this change didn't expose or
  touch.

  A second, broader divergence surfaced once `?` could wrap compound
  expressions rather than just bare builtins: `(.a)?`, `("a"+1)?`,
  `first(error("x"))?`, and `setpath(["a"]; error)?` all leaked their error to
  stderr instead of suppressing it, even though the CLI ran under the exact
  `?` syntax the issue asked for. Root cause: the CLI's generic evaluator
  (`eval_generic.rs`) only checks `optional` natively for a handful of `Expr`
  shapes (`Field`, `Index`, `Iterate`, a few builtins); anything else (`Paren`,
  `Arithmetic`, `Error`, `first(...)`, `reduce`, ...) falls through a bridge
  that re-evaluates via the full evaluator's `eval()` — which always starts
  with `optional = false`, silently dropping the flag. The bridge now
  re-wraps the expression in `Expr::Optional` before re-entering the full
  evaluator when the ambient `optional` is true, mirroring the
  `eval_on_owned`/`eval_on_many_owned` builtin-fallback bridge already fixed
  for #386. `eval_arithmetic` and `eval_error` (in `eval.rs`) also never
  checked `optional` on their own final result, and `builtin_setpath` treated
  a suppressed sub-result the same as a real `null` value instead of
  propagating the suppression — all three are fixed the same way. Deliberately
  preserved: `.[.k]?` and `.[error("boom")]?` still propagate an error raised
  while evaluating the *key* expression, uncaught — jq's `?` only guards the
  indexing operation itself, not the key computation (verified against jq
  1.7.1; a regression here would have broken the existing
  `test_optional_does_not_suppress_key_errors` coverage). Not fixed: `reduce`
  built from a bare erroring generator (e.g. `(reduce error as $x (0; $x))?`)
  still returns `0` instead of suppressing, and `(reduce .[] as $x (0;
  $x+.))?` over a type-erroring element returns `null` instead of suppressing
  — both go through `eval_owned_expr`, a helper shared by `reduce`/`foreach`/
  `while`/`until`/`repeat` (20+ call sites) that collapses any suppressed
  sub-result to `Ok(null)` before its caller can tell the difference. That is
  the same class of problem as the already-tracked stream-builtin
  error-swallowing issues and needs its own fix, not a tail-end change here.

- **`try`/`catch` did not catch `break`** (#562): `label $out | try break
  $out catch "c"` produced no output instead of `"c"` like jq — `eval_try`
  only matched `QueryResult::Error` to invoke the catch handler, so a
  `QueryResult::Break` fell through its `other => other` arm and propagated
  untouched past the `try`, out to the enclosing `label`, which silently
  absorbed it. jq's `catch` catches a `break` the same way it catches a
  raised error, regardless of which label it targets — confirmed against jq
  1.7.1 that `try`/`catch` catches any break flowing through it even when
  the label is bound scopes further out (`label $A | label $B | try break
  $A catch "caught"` still yields `"caught"`). `eval_try` now has a
  `QueryResult::Break` arm parallel to the `Error` one; the catch handler's
  input is bound to `null` rather than jq's own internal `{"__jq":N}` break
  marker, which is an implementation detail not worth replicating.

- **`paths(node_filter)` only kept a path when `node_filter` evaluated to the
  literal boolean `true`, instead of any truthy output** (#718): real jq
  defines `paths(node_filter)` as `path(recurse|select(node_filter))`, so a
  path is kept whenever `node_filter` — a general filter, not necessarily a
  boolean expression — produces at least one truthy value. `builtin_paths_filter`
  instead required `eval_owned_expr` to return exactly `Ok(OwnedValue::Bool(true))`,
  which happened to work for boolean-producing filters like `type ==
  "number"` (the only shape the existing golden/unit coverage exercised) but
  silently dropped every path for the far more common category of filters
  that yield the value itself — `scalars`, `numbers`, `strings`, `arrays`,
  `objects`, `values`, `nulls`, or any user `select()`-style filter. `[1,2] |
  paths(scalars)` no longer returns `[]` where jq returns `[[0],[1]]`. Found
  while adding oracle-verified golden coverage for bare `paths`/`leaf_paths`;
  fixed by checking `OwnedValue::is_truthy()` instead of exact equality with
  `Bool(true)`.

### Changed

- **A tag on an anchored sequence item is now rejected** (#328): `- &a !!str x`
  previously parsed as the plain scalar `"!!str x"`; consuming the anchor before
  dispatching means the tag is seen rather than absorbed, so it now returns
  `YamlError::TagNotSupported`. Consistent with `a: !!str 1`, which already
  errored. Tags remain documented non-support (#224).
- **Self-referential anchors on sequence items are now rejected** (#328):
  `- &m\n  - *m` records a real alias edge for the first time and is caught by
  the existing `AliasCycle` check, where it previously produced garbage. `yq`
  instead emits a depth-limited expansion; rejecting cycles is the documented
  policy (see `docs/compliance/yaml/limitations.md`).
- **BREAKING — `JsonIndex` line/column lookup takes the text** (#228):
  `to_line_column(offset, text)` and `to_offset(line, column, text)` now match
  the existing `YamlIndex` signatures. The line index is built lazily on first
  use instead of eagerly in `JsonIndex::build`, which removes an O(n) scan and a
  full bitmap allocation from every JSON index build (−25.8% of build allocation
  on a 100 KB GeoJSON file). `JsonCursor::cursor_at_position` and the
  `at_position(line; col)` builtin are unaffected. As a consequence `JsonIndex`
  is no longer `Sync` (`YamlIndex` already was not).
- **BREAKING — zero-caller `BitVec` constructors removed** (#228):
  `JsonIndex::from_parts_with_newlines`, `YamlIndex::from_parts_with_newlines`
  and `YamlIndex::newlines` are gone. They were the last public signatures
  exposing `BitVec` outside `succinctly::bits`; line indices are now derived from
  the text on demand. `JsonIndex::from_parts` gains working line/column lookup as
  a side effect — it used to install an empty newline index that silently
  reported every position as line 1.
- `succinctly::json::locate::NewlineIndex` is now an alias for
  `succinctly::text::LineIndex`; `build`, `to_offset` and `to_line_column` are
  unchanged. The alias will be removed in 0.9.0. (#228)
- **4 GiB input ceilings enforced** (#188): instead of silently truncating
  `u32` counters, builds now fail loudly for inputs over `u32::MAX` bytes —
  `YamlIndex::build` returns the new `YamlError::InputTooLarge` variant
  (minor API addition: exhaustive matches on `YamlError` gain an arm), while
  `JsonIndex` and `DsvIndexLightweight` constructors panic with a documented
  message. `BalancedParens` constructors assert a `u32::MAX`-bit ceiling.
  See [docs/reference/limits.md](docs/reference/limits.md).
- **SelectIndex sample entries doubled** from 8 to 16 bytes (one entry per
  `sample_rate` set bits; ~6% of set-bit count at the default rate 256, up
  from ~3%). Serialized (`serde`) representations of `BitVec`,
  `BalancedParens`, and `SelectIndex` change accordingly.

## [0.7.0] - 2026-04-05

### Added

- **JSON Validation**
  - Strict RFC 8259 JSON validator with CLI command (`succinctly json validate`)
  - `--validate` flag for `jq` command to enforce strict validation before processing
  - Comprehensive RFC 8259 compliance test suite

- **Benchmark Infrastructure**
  - JSON validation benchmark suite with criterion
  - Criterion extra args support in benchmark runner

### Performance

- **Zero-copy JSON string output**: eliminates allocation for unescaped strings
- **SIMD-accelerated string escaping**: faster JSON output with vectorized escape detection
- **Lazy string slicing**: defers string slice operations for reduced allocation

### Fixed

- `popcount_words` return type changed from `u32` to `usize` to prevent overflow on large bitvectors (#139)
- JSON container range lookup replaced BP-based method with correct linear scan (#138)

### CI

- ARM64 runner added to coverage matrix
- Separate coverage reports for default, simd, and portable-popcount feature flags

## [0.6.0] - 2026-02-03

### Added

- **jq Compatibility Enhancements**
  - Comprehensive null handling: array indexing, object operations, and built-in functions return null instead of errors
  - String slicing with character-based indexing, negative indices, and Unicode support
  - Overflow handling that converts to float on integer overflow
  - Division and modulo by zero error handling with proper error messages
  - `has()` and `in()` functions properly reject negative array indices
  - `split()` handles empty delimiter by splitting into individual characters
  - `@uri`, `@html`, `@sh` format functions accept and convert non-string types
  - `first`, `last`, `nth` return null for empty/null inputs
  - `reverse()` returns empty array for null input
  - `getpath()` traverses null values gracefully

- **yq Compatibility Enhancements**
  - yq-compatible evaluation mode with different arithmetic semantics (wrapping overflow, infinity for division by zero)
  - Compile-time evaluation semantics system via `EvalSemantics` trait
  - `JqSemantics` and `YqSemantics` marker types for zero-cost abstraction
  - Negative array indexing support for `has()` and `in()` in yq mode

### Changed

- **Breaking**: `eval()` and `eval_lenient()` now require a semantics type parameter (`JqSemantics` or `YqSemantics`)
- **Breaking**: Removed `set_eval_mode()` and `get_eval_mode()` functions (replaced by compile-time generics)
- Replaced runtime evaluation mode switching with compile-time generic semantics for zero-overhead mode selection

### Performance

- Eliminated runtime mode checking branches in arithmetic operations through monomorphization

## [0.5.1] - 2026-02-02

### Fixed

- `jq`: Return `null` for missing fields on objects instead of error (issue #61)

### Changed

- Refactored `json_simd` benchmark into focused components for better maintainability
- Removed unused `count_seq_items_before` method from YAML parser

### Documentation

- Added Apple M4 Pro benchmark results
- Updated ARM Neoverse-V2 benchmark results

## [0.5.0] - 2026-01-31

### Added

- **CLI Enhancements**
  - Multi-call binary support: `sjq`, `syq`, `sjq-locate`, `syq-locate` symlinks
  - `succinctly install-aliases` command to create symlinks
  - Unified benchmark runner (`succinctly bench run`) with comprehensive metadata tracking
  - Default memory collection for CLI benchmarks

- **YAML Streaming (M2.5)**
  - Direct YAML→JSON streaming for navigation queries (`.[0]`, `.[]`)
  - Eliminates intermediate `OwnedValue` DOM for 2-3x faster identity queries
  - 3-4% of yq's memory usage on large files

- **Memory Optimizations**
  - Advance Index: memory-efficient `bp_to_text` mapping with ~1.5x compression
  - EndPositions: 2-bitmap encoding for scalar end positions
  - Sequential cursor optimization for amortized O(1) position lookups
  - Elias-Fano encoding for monotone integer sequences
  - CompactRank two-level directory for O(1) rank queries
  - In-place builder for cache-aligned L1L2 storage

- **SIMD Optimizations**
  - AVX2-accelerated JSON escape scanning for YAML→JSON output on x86_64
  - ARM64 NEON escape scanning for JSON output (4-12x faster on long strings)
  - BMI2 PDEP support for O(1) select-in-word on x86_64

- **yq Compatibility**
  - Key ordering in yq mode: object keys output in document order (matching `jq -yy`)

### Changed

- Build regression mitigation: inline zero-filling and lazy newline index (P12-A)

### Fixed

- `keys` function ordering now compatible with yq mode (returns keys in document order)
- `no_std` compatibility: added missing `alloc::boxed::Box` import
- Elias-Fano: fixed `no_std` and rustdoc compatibility
- Flaky CI: implemented cargo retry logic for test stability

### Performance

- yq identity queries: 20-25% faster on 1MB files (P12 Advance Index)
- yq small-medium files: 3-13% faster (O1 sequential cursor)
- YAML parsing: 11-85% faster build times (P12-A mitigations)
- Escape scanning: 4-12x faster with SIMD (O3)

## [0.4.0] - 2026-01-24

### Added

- **jq Language Enhancements**
  - `at_offset(n)` builtin for position-based navigation to node at byte offset
  - `at_position(line; col)` builtin for navigation to node at line/column position

- **SIMD Optimizations**
  - SSE4.1 PHMINPOSUW optimization for balanced parentheses index building on x86_64
  - SVE2 BDEP `select_in_word` with runtime dispatch on ARM64
  - NEON VMINV L1/L2 index building optimizations for ARM64
  - 256-byte popcount unrolling for improved ARM performance
  - NEON PMULL carryless multiply for prefix XOR optimization

- **Balanced Parentheses Enhancements**
  - Zero-cost `SelectSupport` trait abstraction (`NoSelect` for JSON, `WithSelect` for YAML)
  - O(log n) BP lookup via binary search on `bp_to_text` mapping
  - Unrolled lookup optimization for min excess computation

### Fixed

- YAML `yq-locate` text-position-to-BP mapping now returns correct nodes (issue #26)
- Flaky `cargo run` in jq CLI tests with retry logic

### Performance

- BP select1 queries: 2.5-5.9x faster with sampled select index
- `yq-locate` offset queries: 16-492x speedup with indexed `find_open`

## [0.3.0] - 2026-01-21

### Added

- **YAML Enhancements**
  - `yq-locate` command for finding YAML positions by offset or line/column
  - Multi-document stream support with `--doc N` and `--slurp` options
  - Quoted string type preservation (yq-compatible output)
  - YAML metadata access: `tag`, `anchor`, `alias`, `style`, `kind`, `key`, `line`, `column`
  - Handle explicit empty keys and implicit null values in YAML mappings

- **jq Language Enhancements**
  - `load(file)` operator for external YAML/JSON file loading
  - `split_doc` operator for multi-document YAML output
  - `@props` format encoder for Java properties output
  - `@yaml` format function for YAML encoding
  - yq date extensions: `from_unix`, `to_unix`, `tz(zone)` with IANA timezone support
  - `pivot` builtin for array/object transposition
  - `shuffle` operator for random array reordering
  - `document_index`/`di` operator for multi-doc YAML indexing
  - `omit(keys)` operator for objects and arrays
  - Generic evaluator for direct YAML evaluation without JSON conversion
  - `skip(n; expr)` iteration control builtin
  - `combinations` function for generating combinations
  - Non-local control flow with `label $name | ... | break $name`
  - Regular expressions: `match`, `capture`, `scan`, `splits`, `sub`, `gsub`
  - `$__loc__` for source location tracking, `$ENV` for environment access
  - Module system with `import`, `include`, and namespace support
  - `trunc` math function for truncation toward zero
  - `toboolean` type conversion builtin
  - `pick()` function for selective key extraction
  - Comment support with `#` hash syntax
  - Quoted field access and bracket string notation
  - `key` function for yq iteration context
  - `kind` function for yq node type classification
  - `tojson` and `fromjson` builtins

- **CLI Improvements**
  - `--raw-input` (`-R`) option for line-by-line processing in yq
  - `--slurp` (`-s`) option for collecting all inputs into array
  - `--doc N` option for multi-document selection in yq

### Fixed

- Handle explicit empty keys in YAML mappings
- Emit explicit null nodes for implicit null values in YAML mappings
- Make `paths` and `paths(filter)` stream individual results correctly
- Correct `repeat` builtin to evaluate with original input
- Support any type for `indices`/`index`/`rindex` on arrays
- Make `leaf_paths` stream individual paths
- Enable postfix operations on builtin expressions
- Negative index support for `getpath`
- Replace std with core/alloc for no_std compatibility

### Performance

- YAML identity queries: 90-217 MiB/s (2.3x improvement with direct streaming)
- yq vs system yq: 16-40x faster on x86_64, 1.9-8.7x faster on ARM

## [0.2.0] - 2026-01-18

### Added

- **YAML Semi-Indexing**
  - Complete YAML parser with oracle-style parsing (~250-400 MiB/s)
  - `yq` CLI command for YAML processing with jq syntax
  - Direct YAML-to-JSON streaming (2.3x faster than DOM conversion)
  - Multi-document stream support with virtual root wrapper
  - Anchor and alias resolution at parse time
  - Block scalar support (literal `|` and folded `>` styles)
  - Flow style parsing (inline arrays and objects)
  - Explicit key/value indicators (`?` and `:`)
  - SIMD optimizations: anchor/alias scanning (6-17% improvement), block scalar parsing (19-25% improvement)

- **DSV/CSV Semi-Indexing**
  - High-performance CSV/TSV parser with succinct indexing (85-1676 MiB/s API, 11-169 MiB/s CLI)
  - `--input-dsv` flag for jq command to read CSV/TSV input
  - `@dsv(delimiter)` format function for custom delimiter output
  - BMI2 PDEP acceleration for quote masking on x86_64
  - Lightweight cumulative rank index (1.8-4.3x faster than BitVec)
  - SIMD-accelerated parsing on both x86_64 (AVX2) and ARM (NEON)

- **jq Enhancements**
  - `jq-locate` command for finding JSON positions by offset or line/column
  - Assignment operators: `=`, `|=`, `+=`, `-=`, `*=`, `/=`, `%=`, `//=`, `del()`
  - Path operations: `path()`, `paths`, `leaf_paths`, `getpath`, `setpath`, `delpaths`
  - Date/time functions: `now`, `gmtime`, `localtime`, `strftime`, `strptime`, `todate`, `fromdate`
  - Type filters: `values`, `nulls`, `booleans`, `numbers`, `strings`, `arrays`, `objects`, `scalars`, `iterables`
  - Math functions: all 34 standard jq math functions
  - Lazy evaluation with identity fast path (zero-allocation for `.` queries)
  - JSON sequence format (RFC 7464) support with `--seq`
  - ASCII escaping (`-a` flag) and ANSI color syntax highlighting (`-C` flag)
  - `$ARGS` variable and positional argument support (`--args`, `--jsonargs`)
  - Build configuration reporting flag (`--build-configuration`)
  - Unary minus operator for expression negation

- **SIMD Enhancements**
  - Portable broadword module for non-SIMD platforms
  - Block scalar SIMD optimization with AVX2 newline scanning
  - SWAR (SIMD Within A Register) classification for ARM64

### Changed

- jq-compatible number formatting is now the default behavior
- Renamed `--no-jq-compat` to `--preserve-input` for clarity

### Fixed

- `enclose()` word boundary bug with zero-excess words in balanced parentheses
- `no_std` compatibility issues in SIMD modules

### Performance

- YAML parsing: 250-400 MiB/s (oracle parser)
- DSV parsing: 85-1676 MiB/s (API), 11-169 MiB/s (CLI)

## [0.1.0] - 2026-01-11

### Added

- **Core Data Structures**
  - `BitVec` with O(1) rank and O(log n) select operations
  - 3-level Poppy-style rank directory with ~3% space overhead
  - Sampled select index with configurable sample rate (~1-3% overhead)
  - `RankSelect` trait for generic rank/select operations

- **Balanced Parentheses**
  - `BalancedParens` structure for succinct tree navigation
  - RangeMin hierarchical min-excess index (~6% overhead)
  - O(1) `find_close`, `find_open`, `enclose` operations
  - Tree navigation: `first_child`, `next_sibling`, `parent`, `subtree_size`

- **JSON Semi-Indexing**
  - Interest Bits (IB) and Balanced Parentheses (BP) encoding
  - Table-driven PFSM parser achieving 880 MiB/s throughput on x86_64 (AMD Zen 4)
  - `JsonIndex` for building semi-indices from JSON bytes
  - `StandardJson` cursor for lazy navigation without full parsing

- **SIMD Acceleration**
  - AVX2 SIMD JSON parser (32 bytes/iteration, 78% faster than SSE2)
  - AVX-512 VPOPCNTDQ popcount (5.2x faster than scalar)
  - SSE4.2 with PCMPISTRI for character classification
  - ARM NEON support (mandatory on aarch64)
  - Runtime CPU feature detection for optimal dispatch

- **jq Query Language**
  - Path expressions: `.foo`, `.[0]`, `.[-1]`, `.[]`
  - Array slicing: `.[2:5]`, `.[2:]`, `.[:5]`
  - Chained access: `.foo.bar`, `.foo[0].bar`
  - Optional access: `.foo?`
  - Comma operator: `.foo, .bar`
  - Array/object construction: `[.foo]`, `{foo: .bar}`
  - Recursive descent: `..`
  - Literals: `null`, `true`, `false`, numbers, strings
  - Arithmetic: `+`, `-`, `*`, `/`, `%`
  - Comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`
  - Boolean operators: `and`, `or`, `not`
  - Alternative operator: `//`
  - Conditionals: `if-then-else-end`
  - Error handling: `try-catch`, `error()`

- **CLI Tool**
  - `json generate` - Generate synthetic JSON for benchmarking
  - `jq` - jq-compatible command-line JSON processor
  - `--jq-compat` flag and `SUCCINCTLY_JQ_COMPAT=1` env var for exact jq output compatibility
  - Multiple output formats and memory-mapping support

- **Platform Support**
  - `no_std` compatible (with `alloc`)
  - x86_64 with AVX2, AVX-512, SSE4.2, SSE2
  - aarch64 with NEON
  - Optional `serde` serialization support

### Performance (x86_64 AMD Ryzen 9 7950X)

- JSON semi-indexing: 880 MiB/s (PFSM), 732 MiB/s (AVX2)
- Rank queries: ~3 ns (O(1))
- Select queries: ~50 ns (O(log n))
- Popcount: 96.8 GiB/s (AVX-512), 18.5 GiB/s (scalar)

[Unreleased]: https://github.com/rust-works/succinctly/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/rust-works/succinctly/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/rust-works/succinctly/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/rust-works/succinctly/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/rust-works/succinctly/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/rust-works/succinctly/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/rust-works/succinctly/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rust-works/succinctly/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rust-works/succinctly/releases/tag/v0.1.0
