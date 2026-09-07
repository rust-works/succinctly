# Per-arm reachability of the eager path-context evaluator (spine 2416, steps 4-5)

`eval_stage_with_path_context` (`src/jq/eval.rs`) is the eager, materialising
path-context evaluator ADR-0021 is retiring. It handles 44 expression shapes by
name -- 5 pre-match `if` handlers plus 39 arms of its top-level `match first`
(the split `tests/jq_path_context_arm_guard.rs` pins) -- and each one was added
to fix a shape the generic evaluator could not answer.

Step 4 asks, of every one of those: is there still a query that reaches it?

**Answer: all of them are reachable.** None was deleted. `PINNED_ARM_COUNT` sat
at 43 through step 5 and moved to 44 with #2522, which *added* an arm rather
than migrating one (see its section below). The rest of this page is the
evidence.

## Method

Each of the 43 handlers was instrumented with a temporary
`eprintln!("ARMHIT <id>")` as the first statement of its body (expression-bodied
arms -- `Expr::Shared`, `Expr::Break`, the four that tail-call a
`*_with_path_context` helper, and `Expr::DefCall`'s `match` -- were wrapped in a
block for the purpose). The instrumented CLI was built with `--features cli` and
each candidate query run through it, with stderr captured. A handler is
**REACHABLE** iff its marker appeared. The instrumentation was reverted before
this document was committed; nothing in the tree carries it.

`path_context_needs_eager` (`src/jq/eval_generic.rs`) was instrumented the same
way, splitting its single three-way `if` into one `eprintln!` per disjunct, so
each proof query also records *why* the gate handed the pipe over.

Every query below is jq mode against one document:

```
D = {"a":{"b":1},"c":[10,20],"n":0,"m":1}
```

and was run as

```bash
printf '%s' '{"a":{"b":1},"c":[10,20],"n":0,"m":1}' | succinctly jq '<filter>'
```

## The one door into the eager evaluator

Step 4 found three; step 5 closed two of them. What is left is the gate:

1. **The gate.** `path_context_needs_eager(exprs)` returning `true` in
   `eval_single`'s `Expr::Pipe` arm and in `eval_each_pipe_generic`
   (`src/jq/eval_generic.rs`) bridges the whole pipe to
   `eval::eval_pipe_with_path_context`. This is the door ADR-0021 decision 3
   describes, and it is now the only one.
2. ~~**`--eval-all`.**~~ *Closed (step 5.)*
   `eval::eval_owned_with_file_index` (#715) used to call
   `eval_pipe_with_path_context_internal` directly, on `needs_path_context(expr)`
   alone, with no gate at all -- verified live at step 4 that
   `succinctly yq --eval-all '[.[] | file_index]' f1.yaml f2.yaml` fired
   `H3`, `A05`, `A09` and `A21` without `path_context_needs_eager` being
   consulted. It reindexes the combined array and evaluates it through the
   generic evaluator now, like every other entry. The file-origin table that
   forced the direct call is an ambient scope (`eval_generic::with_file_origin`)
   the cursor route, the absent route, the owned-identity route and the eager
   evaluator all read, so `file_index` is answered the same way wherever the
   gate sends the pipe (#2427's `file_index` half, for the `--eval-all` route).
3. ~~**`eval::eval_pipe`'s own diversion.**~~ *Closed (step 5.)*
   `eval_pipe` used to route any pipe with `needs_path_context` into
   `eval_pipe_with_path_context`. `jq::eval` does not reach it (ADR-0021
   decision 4), but the generic evaluator's own bridges back into that file
   (`eval_on_owned` / `bridge_to_full_evaluator` -> `eval_full`) do, so it was
   live code rather than a dead branch -- measured, not argued: with the
   diversion instrumented, `reduce .c[] as $x (.a; [.[] | key])` and
   `.x = [.a[] | key]` both reach it, because `needs_path_context` deliberately
   does not recurse into a fold's UPDATE/EXTRACT or an assignment's right-hand
   side, so the enclosing program's routing decision was made against a
   `false`. It now hands the pipe to the generic evaluator over a root cursor
   for a reindexed copy of its input -- the eager evaluator's own
   "`root` is the value I was handed, `current_path` is `[]`" position,
   expressed as a node -- and `path_context_needs_eager` decides.

`tests/jq_path_context_single_door_guard.rs` is what keeps this section true:
it scans `src/` and fails if any entry into `eval_pipe_with_path_context`/
`_internal` from outside `src/jq/eval.rs` is not inside a
`path_context_needs_eager` branch, and pins that `eval_pipe` and
`eval_owned_with_file_index` name neither symbol.

**No arm became unreachable.** Every one of the 43 rows below is proved by a
query that enters through the gate (the `Gate` column names which disjunct
fired), and step 5 did not touch that route -- the proof queries' outputs are
pinned unchanged in
`tests/jq_evaluator_parity_tests.rs::test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416`.
The four arms step 4 attributed to the `--eval-all` door (`H3`, `A05`, `A09`,
`A21`) each have a gate-route proof query in the table as well, so closing that
door removed no arm's only route. `PINNED_ARM_COUNT` therefore stays at 43.

## Gate reasons

`path_context_needs_eager`'s single condition is three disjuncts, tested in this
order; the `Gate` column names the first that fired for that query.

| Id  | Disjunct                             | Meaning                                                                             |
|-----|--------------------------------------|-------------------------------------------------------------------------------------|
| R1  | `!is_node`                           | the value reaching the path-context stage is detached -- no cursor to read a key from |
| R2  | `can_absent && !absent_routed`       | the navigational head can miss, and `path_context_absent_split` declined the pipe     |
| R3  | `!path_context_stage_native(stage)`  | the stage is built from a construct with no native cursor-threading arm               |

R2 still dominates the table, but for a narrower reason since #2472 (below): a
`.field` step can always miss, and a *head that can miss* is only handed over
now when neither the constant route nor the owned identity pipe can take the
stages after it. The probe shape that trips it was `.a?` until #2558 (below)
made `?` over navigation navigational; what is left is a stage with no
`owned_identity_rule` after a head that can miss (`.a.b | . as $x | ...`,
`map`, `reduce`/`foreach`, `label`/`def`) or a bare
`parent` operand (which no route can name a position for). The same shapes with
a head that cannot miss trip R3 instead -- e.g. `.a[] | (key | tostring)`,
`.a[] | (key | length)`, `.a[] | {"k": (key | tostring)}` and
`.a[] | ((key | tostring) and true)` all report R3 (re-derived after #2473,
below). This is an ordering artefact, not a claim that R3 is rare.

Step 1b (below) moved two of the three shapes this paragraph used to name --
`.a[] | key + "x"` and `.a[] | if key == "b" then key + "x" else "y" end` -- off
R3 and onto the generic route, which is what an admission to
`path_context_single_native` is *for*, and #2473 moved two more:
`.a[] | select(key == "b" and true)` and
`.a[] | reduce (key) as $k (""; . + $k) | . + "x"` both report no gate at all
now. Re-derive this paragraph's examples after any further admission rather than
trusting them.

## The 44 handlers

| #   | Handler (site in `eval_stage_with_path_context`)                   | Verdict   | Proof query (jq mode, document `D`)                   | Gate |
|-----|--------------------------------------------------------------------|-----------|-------------------------------------------------------|------|
| H1  | pre-match `if matches!(first, Expr::Builtin(Builtin::PathNoArg))`  | REACHABLE | `.a.b \| parent(0+1) \| path + []` | R2   |
| H2  | pre-match `if matches!(first, Expr::Builtin(Builtin::Key))`        | REACHABLE | `.a.b \| parent(0+1) \| (key and parent)`   | R2   |
| H3  | pre-match `if matches!(first, Expr::Builtin(Builtin::FileIndex))`  | REACHABLE | `.a.b \| parent(0+1) \| file_index + 1` | R2 |
| H4  | pre-match `if matches!(first, Expr::Builtin(Builtin::Parent))`     | REACHABLE | `.a.b \| parent(0+1) \| (key and parent)`                                 | R2   |
| H5  | pre-match `if let Expr::Builtin(Builtin::ParentN(n_expr)) = first` | REACHABLE | `.a.b \| parent(0+1) + {}`                            | R2   |
| A01 | `Expr::Identity`                                                   | REACHABLE | `.a.b \| parent(0+1) \| . \| key`                            | R2   |
| A02 | `Expr::Field(name)`                                                | REACHABLE | `.a.b \| parent(0+1) + {}`                                 | R2   |
| A03 | `Expr::Index { idx, key }`                                         | REACHABLE | `.c[0] \| parent(0+1) \| key`                                | R2   |
| A04 | `Expr::Slice { .. }`                                               | REACHABLE | `.c[0:1] \| .[0] \| key + 1`                          | R1   |
| A05 | `Expr::Iterate`                                                    | REACHABLE | `.a[] \| parent(0+1) \| key`                            | R3   |
| A06 | `Expr::Paren(inner)`                                               | REACHABLE | `.a.b \| parent(0+1) \| (key and parent)`                               | R2   |
| A07 | `Expr::Optional(inner) if IndexExpr/SliceExpr`                     | REACHABLE | `.c[.n]? \| parent(0+1) \| key`                          | R2   |
| A08 | `Expr::Optional(inner)`                                            | REACHABLE | `.a? \| parent(0+1) \| key`                              | R2   |
| A09 | `Expr::Pipe(inner) if rest.is_empty()`                             | REACHABLE | `.a.b \| parent(0+1) \| -(key\|length)`                            | R2   |
| A10 | `Expr::Pipe(inner)`                                                | REACHABLE | `.a.b \| parent(0+1) + {}`                                 | R2   |
| A11 | `Expr::Arithmetic { .. }`                                          | REACHABLE | `.a.b \| parent(0+1) + {}`                                 | R2   |
| A12 | `Expr::And(..) \| Expr::Or(..)`                                    | REACHABLE | `.a.b \| parent(0+1) \| (key and parent)`                  | R2   |
| A13 | `Expr::Negate(operand)`                                            | REACHABLE | `.a.b \| parent(0+1) \| -(key\|length)`                            | R2   |
| A14 | `Expr::Compare { .. }`                                             | REACHABLE | `.a.b \| parent(0+1) \| (key == "a")`                      | R2   |
| A15 | `Expr::Builtin(Builtin::Select(cond))`                             | REACHABLE | `.a.b \| parent(0+1) \| select(key == "a") \| key`           | R2   |
| A16 | `Expr::Builtin(Builtin::Map(f))`                                   | REACHABLE | `.a.b \| parent(0+1) \| map(key)`                                | R2   |
| A17 | `Expr::Builtin(Builtin::GetPath(path_expr))`                       | REACHABLE | `getpath(["a","b"]) \| (key and parent)`            | R1   |
| A18 | `Expr::Builtin(_)`                                                 | REACHABLE | `.a.b \| parent(0+1) \| -(key\|length)`                             | R2   |
| A19 | `Expr::IndexExpr { target, key }`                                  | REACHABLE | `.c[(0,1)] \| (key and parent)`                           | R1   |
| A20 | `Expr::SliceExpr { target, start, end }`                           | REACHABLE | `.c[.n:.m] \| .[0] \| key + 1`                        | R1   |
| A21 | `Expr::Array(inner) if needs_path_context(inner)`                  | REACHABLE | `.a.b \| parent(0+1) \| [key] + ["x"]`                   | R2   |
| A22 | `Expr::StringInterpolation(parts) if ..`                           | REACHABLE | `.a.b \| parent(0+1) \| "\(key)"`                       | R2   |
| A23 | `Expr::DefCall { .. }`                                             | REACHABLE | `.a.b \| parent(0+1) \| def f: key; f + "x"`                         | R2   |
| A24 | `Expr::Shared(inner)`                                              | REACHABLE | `def f(x): x; .a.b \| f(key) + "z"`                   | R2   |
| A25 | `Expr::FuncDef { .. }`                                             | REACHABLE | `.a.b \| parent(0+1) \| def f: key; f + "x"`                         | R2   |
| A26 | `Expr::As { .. } if ..`                                            | REACHABLE | `.a.b \| parent(0+1) \| . as $x \| key`                  | R2   |
| A27 | `Expr::AsPattern { .. } if ..`                                     | REACHABLE | `.c[0] \| parent(0+1) \| . as [$x] \| key`                        | R2   |
| A28 | `Expr::Limit { n, expr } if ..`                                    | REACHABLE | `.a.b \| parent(0+1) \| limit(1; key + "x")`                         | R2   |
| A29 | `Expr::FirstExpr(expr) if ..`                                      | REACHABLE | `.a.b \| parent(0+1) \| first(key + "x")`                            | R2   |
| A30 | `Expr::LastExpr(expr) if ..`                                       | REACHABLE | `.a.b \| parent(0+1) \| last(key + "x")`                             | R2   |
| A31 | `Expr::Reduce { .. } if ..`                                        | REACHABLE | `.a.b \| parent(0+1) \| reduce (key) as $k (""; . + $k)`  | R2   |
| A32 | `Expr::Foreach { .. } if ..`                                       | REACHABLE | `.a.b \| parent(0+1) \| foreach (key) as $k (""; . + $k)` | R2   |
| A33 | `Expr::Object(_) \| Expr::Array(_) \| Expr::Literal(_)`            | REACHABLE | `.a.b \| parent(0+1) \| {z: key}`         | R2   |
| A34 | `Expr::If { .. }`                                                  | REACHABLE | `.a.b \| parent(0+1) \| if key == "a" then key + "x" else "y" end`   | R2   |
| A35 | `Expr::Comma(exprs)`                                               | REACHABLE | `.a.b \| parent(0+1) \| (key + "x"), key`                            | R2   |
| A36 | `Expr::Try { .. }`                                                 | REACHABLE | `.a.b \| parent(0+1) \| try (key + "x") catch "e"`                   | R2   |
| A37 | `Expr::Label { name, body }`                                       | REACHABLE | `.a.b \| parent(0+1) \| label $out \| (key + "x", break $out)`       | R2   |
| A38 | `Expr::Break(name)`                                                | REACHABLE | `.a.b \| parent(0+1) \| label $out \| (key + "x", break $out)`       | R2   |
| A39 | `Expr::Update \| CompoundAssign \| AlternativeAssign`               | REACHABLE | `.a.b \| parent(0+1) \| .b \|= key`                                    | R2   |

## Step 1b: `Expr::Arithmetic` admitted, and the pin holds at 43

Spine 2416 step 1b gave `eval_generic`'s own `eval_single` a native
`Expr::Arithmetic` arm (`collect_each_generic` over the sink evaluator's
existing `binary_fanout_each_generic` arm) and added `Expr::Arithmetic` to
`path_context_single_native`. Two shapes had blocked that admission and both are
now closed:

- yq's rule for a binary operator whose operand produces *zero* outputs
  ([#2460](https://github.com/rust-works/succinctly/issues/2460)). `key + 1` at
  the document root is `1` in real yq, not nothing, and the eager route used to
  answer it from a `null`/`{}` placeholder. Admitting arithmetic would have moved
  that shape to a route with a *different* answer. It is now one definition
  (`jq::eval::yq_empty_operand_output`) that both fanouts consult, so the two
  routes agree before the move rather than after it.
- `try (1+1) catch "x"` on a #1194-malformed document (`{123: 1}`), where jq
  1.7.1 rejects the whole input regardless of what the filter reads. The eager
  bridge's ambient materialization is what produces that failure, so the new arm
  is gated on `needs_path_context`: arithmetic that reads no path context stays
  on the bridge and keeps paying the decode.
  `test_try_catch_contains_a_genuinely_catchable_malformed_key_error_1812` is
  the guard.

**`PINNED_ARM_COUNT` stays at 43.** `A11` (`Expr::Arithmetic`) is the arm the
admission was aimed at and it is still reachable: re-run with the method above,
its marker fires for 33 of the 36 queries probed, including its own listed proof
query `.a.b | key + "x"` (R2) and every `.c[...]`-headed R1 row. What the
admission removes is R3 for an `Expr::Iterate` head, not the arm -- `.a[] | key +
"x"` no longer reaches the eager evaluator at all, which is why `A05`'s proof
query above was re-derived (`.a[] | (key | tostring)`, still R3, marker
confirmed). Nothing became unreachable, so nothing was deleted.

## #2471: gate reason 1 narrowed, and the pin still holds at 43

[#2471](https://github.com/rust-works/succinctly/issues/2471) took four of the
owned-domain shapes gate reason `R1` still hands over -- computed
`IndexExpr`/`SliceExpr` (and `?` over them) as navigation, `getpath(p)` as a
navigational stage, a path-context read after `key`/`path`/`file_index`
replaced the value, and a read under arithmetic inside a ruled stage -- and
moved them onto the owned identity pipe (ADR-0021 decision 7). Each row is an
oracle capture; `test_owned_identity_rules_match_yq_2416` (yq mode) and
`test_computed_navigation_keeps_path_context_2471` /
`test_path_context_after_key_or_path_2471` (jq mode) carry them.

**`PINNED_ARM_COUNT` stays at 43.** Re-run of the method above, on the
post-#2471 tree, with the five `R1` arms instrumented and each fed its own
proof query from the table (document `D`):

| Id  | Proof query                                | Marker  | Output  |
|-----|--------------------------------------------|---------|---------|
| A04 | `.c[0:1] \| .[0] \| key + 1`                | fired   | `1`     |
| A07 | `.c[.n]? \| key + 1`                        | fired   | `1`     |
| A17 | `.a \| getpath(["b"]) \| . as $x \| key`    | fired   | `"b"`   |
| A19 | `.c[.n] \| key + 1`                         | fired   | `1`     |
| A20 | `.c[.n:.m] \| .[0] \| key + 1`              | fired   | `1`     |

All five still fire, with their pinned outputs unchanged
(`test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416` is green). The
reason is structural rather than incidental: #2471 widened what the *owned
identity pipe* accepts once a pipe has already left the cursor domain, but a
computed bracket or a `getpath` standing at the **head** of a pipe is still a
detaching stage with no `owned_identity_rule`, so `owned_identity_pipe_applies`
declines it and `path_context_needs_eager` answers `true` exactly as before.
Widening that would mean teaching the cursor walk itself
(`path_context_is_navigational` + `path_step_generic`) to evaluate a computed
component -- which it cannot do uniformly, because an *absent* position
(`PathNode::Absent`) carries no cursor to evaluate the component against. That
is the next step for this cluster, and it is what would make the five `R1` rows
unreachable.

*(Superseded by "#2471 remainder: a computed bracket at the head" below. The
absent position is not the obstacle it looks like here -- the component is
evaluated against the `null` such a position holds, exactly as the owned
identity pipe does -- and the walk took `Expr::IndexExpr` on those terms. Two
of the five rows still fire and three of them never could: a slice and a
`getpath` can stand on a value that is not a document node at all, which is the
real obstacle and is `PathNode`'s, not the component's.)*

## #2472: gate reason 2 narrowed, and the pin still holds at 43

[#2472](https://github.com/rust-works/succinctly/issues/2472) took two of the
three shapes gate reason `R2` still hands over -- a `parent`/`parent(n)` read
from a possibly-absent position inside a non-navigational stage, and navigation
*after* a non-navigational stage -- and moved them onto the owned identity pipe
(ADR-0021 decision 7). An absent position is the deepest real ancestor it hangs
under plus the components taken past it, which is exactly an `OwnedIdentity`, so
`parent` has a node to answer with and the position may move. The third shape, a
fan-out head, stays eager by design: one position per element is the memory cost
`path_context_fans_out` exists to refuse, and ADR-0021 records it as the exit
condition's residue.

**`PINNED_ARM_COUNT` stays at 43.** Re-run of the method above on the post-#2472
tree, in two passes. Pass 1 instrumented `path_context_needs_eager` alone
(one `eprintln!` per disjunct) and ran all 43 listed proof queries: **29 still
enter the gate, 14 no longer do.** The 14 are `H1`, `H2`, `H3`, `A01`, `A02`,
`A03`, `A06`, `A10`, `A11`, `A14`, `A15`, `A18`, `A21` and `A33` -- every one
of them a `.a.b`-headed `key`/`path`/`file_index` shape, which is precisely the
class the identity route now answers.

Pass 2 instrumented those 14 handlers and swept the 29 surviving queries plus
candidates. **All 14 still fire**, and the table above now carries the
re-derived query for each (outputs pinned in
`test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416`, alongside the old
spellings). Two shapes do the work:

- **`?` is the reliable `R2` head now.** `path_context_is_navigational`
  deliberately excludes `Expr::Optional` (the walk's own documented exclusion:
  `?` in path context is not `?` in a path expression), so the absent split's
  head stops before it, there is no position to walk, and the pipe still trips
  `can_absent && !absent_routed`. `.a? | key + "x"` fires `H1`, `H2`, `A01`,
  `A02`, `A11` and `A21` in one query.
- **`parent` outside a ruled stage is still not routable.** A bare
  `parent + {}` has `parent` as an arithmetic *operand*, and
  `owned_identity_operand_resolvable` refuses a read it cannot name a position
  for -- #2471's own pinned row. So `.a.b | parent + {}` still enters at `R2`
  and reaches `A02`, `A10` and `A11`.

The 29 unmoved rows need no re-derivation and none was done: this change does
not touch `eval_stage_with_path_context` at all, only which pipes reach it, so a
query that still enters the gate fires exactly the arms it fired before.

**Nothing became unreachable, so nothing was deleted.** The reason is
structural, not incidental: every arm listed here is reached by *some* stage
shape, and the identity route accepts a `rest` only when every stage of it has
an `owned_identity_rule` or is navigation the pipe can name a component for.
`and`/`or` (`A12`), unary minus outside a ruled stage (`A13`), `map` (`A16`),
`reduce`/`foreach` (`A31`/`A32`), `as`/`label`/`def` (`A23`-`A27`, `A37`,
`A38`) and the bounded consumers (`A28`-`A30`) have no rule, so a pipe
containing one still hands over -- and once it does, the generic structural
arms (`A01`, `A02`, `A06`, `A10`, `A11`) run inside it. Deleting an arm needs
those *stage* shapes migrated, not this route widened.

## #2473: gate reason 3 narrowed, and the pin still holds at 43

[#2473](https://github.com/rust-works/succinctly/issues/2473) is the gate's
third disjunct: a stage built from a construct the generic evaluator has no
cursor-threading arm for. Four constructs were on the list; three moved and one
did not.

- **`and`/`or`** got `eval_single` arms of their own (`eval_boolean_generic`
  over `eval::boolean_fanout_bools`, shared with the eager route so #2460's
  zero-output-operand rule and #2470's read-only operand scope keep one
  definition), and joined `path_context_single_native`. Gated on
  `needs_path_context`, exactly as `Expr::Arithmetic` is, so an `and`/`or` that
  reads no path context still pays the bridge's ambient decode on a malformed
  document.
- **`Expr::Object` inside `needs_path_context`** -- #1332's other half. This is
  not an admission (`path_context_single_native` has had an `Expr::Object` arm
  since #2439) but a *routing* fix: an object literal's key and value slots
  were invisible to `needs_path_context`, so a pipe containing one was never
  recognised as a path-context pipe and no route was ever asked. The generic
  evaluator's own arm reads the cursor, which is why a live-node shape happened
  to be right; every shape that needs a route -- an absent position, a value
  that had left the cursor domain -- answered `null` (jq mode) or nothing (yq
  mode). `path_context_resolvable` and `path_context_resolve_constants` gained
  matching `Expr::Object` arms, and the eager evaluator's existing
  value-construction arm now evaluates the slots at the position it was given
  instead of resetting the context first (the same split `Expr::Array(inner) if
  needs_path_context(inner)` has made since #1302). That last change is a
  pluggable slot evaluator on `build_object_entries`, **not** a new handler, so
  the arm count is unmoved.
- **`reduce`/`foreach`** joined `path_context_single_native`: the generic arms
  already evaluate INPUT and INIT through `stream_owned_outputs_generic` with
  the cursor, so the admission is the same "the arm already exists, say so"
  shape `Expr::As`/`Expr::Label` had. UPDATE and EXTRACT are not part of the
  gate -- they evaluate against the accumulator, a value with no document
  position, which is why `needs_path_context` does not descend into them
  either.
- **`range`/`repeat`/`while`/`until` got no arm.** Real yq's lexer rejects all
  four (v4.53.3), and jq 1.7.1 has no path-context builtin to put inside one,
  so there is no captured behaviour to migrate toward -- only succinctly's own
  extension surface. Their current answers are pinned instead
  (`test_range_repeat_while_until_stay_extension_only_2473`).

**`PINNED_ARM_COUNT` stays at 43.** Re-run of the method above on the
post-#2473 tree, in two passes.

Pass 1 instrumented `path_context_needs_eager` alone (one `eprintln!` per
disjunct) and ran all 43 listed proof queries: **42 still enter the gate, 1
does not.** The one is `A12` (`Expr::And(..) | Expr::Or(..)`), whose listed
query `.a.b | key == "b" and true` is exactly the shape the first bullet
migrated. Every other row reports the same disjunct it reported before, and
every output is unchanged (`test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416`).

Pass 2 instrumented `A12` and swept candidates. **It still fires**, on several
shapes and through all three disjuncts:

| Candidate                                       | Gate | Marker | Output      |
|-------------------------------------------------|------|--------|-------------|
| `.a? \| key == "a" and true`                     | R2   | fired  | `true`      |
| `.a? \| (key and parent)`                        | R2   | fired  | `true`      |
| `.[] \| ((key \| tostring) and true)`             | R3   | fired  | `true` (x4) |
| `.a \| to_entries \| .[0] \| key == 0 and true`    | R1   | fired  | `true`      |
| `.[] \| .k \| (key == "k" and true)`              | R2   | fired  | `true`      |

The table above carries the first of those as `A12`'s query now; the old
spelling stays pinned alongside it, which is what makes the move readable as
"same outputs, different route" rather than as a silent change.

The three arms this change could plausibly have starved were checked
explicitly and all still fire: `A21` (`Expr::Array(inner) if
needs_path_context(inner)`) and `A33` (`Expr::Object(_) | Expr::Array(_) |
Expr::Literal(_)`) on `.a? | {"k": key}` and `.a? | [key] + ["x"]`, and
`A31`/`A32` (`reduce`/`foreach`) on their own listed queries, which still enter
at `R2` because a `.a.b` head can miss.

**Nothing became unreachable, so nothing was deleted.** The reason is the same
structural one #2472 recorded, seen from the other side: an admission to
`path_context_single_native` removes a *reason* for one stage shape, not the
arm. `.a?` remains a head no route can walk, so any construct placed after one
still reaches its arm -- and the eager evaluator's structural arms
(`Expr::Field`, `Expr::Pipe`, `Expr::Paren`, ...) run inside whatever does.
Deleting an arm needs the `?` head and the fan-out head routed, not this list
widened further.

## #2471 remainder: map-family bodies and an assignment's right side, and the pin still holds at 43

The rest of [#2471](https://github.com/rust-works/succinctly/issues/2471) closed two
shapes that were invisible to routing altogether, not merely handed to the eager
evaluator: `needs_path_context` did not descend into a `map_values`/`with_entries`
body, nor into an assignment's right-hand side, so no route was ever asked and
`key`/`path` answered with nothing. Both now descend, and both have somewhere to go
(ADR-0021 decision 7): an assignment's right side is a stage-wide constant, answered
by the existing constant/identity rewrite; a map-family body is one position *per
member*, answered by `eval_map_family_positioned` from the container's position plus
the component.

**`PINNED_ARM_COUNT` stays at 43,** and the argument is the same shape as #2471's own
above. Re-run of the method with the five `R1` arms instrumented, each fed its proof
query from the table (document `D`):

| Id  | Proof query                                | Marker  | Output  |
|-----|--------------------------------------------|---------|---------|
| A04 | `.c[0:1] \| .[0] \| key + 1`                | fired   | `1`     |
| A07 | `.c[.n]? \| key + 1`                        | fired   | `1`     |
| A17 | `.a \| getpath(["b"]) \| . as $x \| key`    | fired   | `"b"`   |
| A19 | `.c[.n] \| key + 1`                         | fired   | `1`     |
| A20 | `.c[.n:.m] \| .[0] \| key + 1`              | fired   | `1`     |

All five still fire with their pinned outputs, for the reason #2471's own section
already gives: a computed bracket or a `getpath` at the *head* of a pipe is still a
detaching stage with no `owned_identity_rule`, which this change does not touch.

The two arms the change could plausibly have starved were instrumented in the same
run, since the map-family stages used to reach the eager evaluator through them:

| Id  | Proof query                | Marker  | Output    |
|-----|----------------------------|---------|-----------|
| A16 | `.a \| map(key + "x")`      | fired   | `["bx"]`  |
| A18 | `.a[] \| (key \| length)`   | fired   | `1`       |

`A16` (`Builtin::Map`) is deliberately kept reachable: a `map` over a *live* cursor is
already exact on the eager route, so `path_context_needs_eager` admits only
`map_values`/`with_entries` for a live head, and `map` reaches the positioned route
only once its input has already left the cursor domain (`.a | to_entries | map(.value =
key)`). Moving a correct answer to a new route buys nothing and risks something.

The migration is real rather than nominal even so — with the same instrumentation,
`.a | map_values(key)`, `.a | with_entries(.value = key)`, `.a | to_entries | map(.value
= key)`, `.a | to_entries | map(key)`, `.a | map(key)` and `.a | .b = key` all fire *no*
marker at all: none of them enters the eager evaluator any more. The instrumentation was
reverted before this document was committed.

### What the change is measured against

A differential over 531 map-family and assignment queries (nine heads x three families
x seventeen bodies, plus the assignment rows) on `a: {b: 1, e: 2}`, `n: [1, 2]`,
`d: {x: {y: 3}, z: [4, 5]}`, `s: hi`, `u: null`, comparing the pre-change binary, the
post-change binary and yq v4.53.3: **142 answers changed, 102 of them from disagreeing
with yq to agreeing with it, and none the other way.** The remaining 40 are three
pre-existing gaps this change makes *reachable* rather than causes, each demonstrable
with no path context anywhere in the filter:

- `map_values(select(...))` — real yq keeps a member whose filter produced nothing (its
  zero-output update rule, #2484), succinctly drops it. `.a | map_values(select(. == 1))`
  diverges identically. The new answer is strictly closer (`{"b":1}` where it was `{}`).
- `with_entries(<body that is not an entry>)` — the body now produces a value where it
  used to produce nothing, so `from_entries` refuses it. Real yq refuses too, with its own
  wording, so the exit code now agrees where it previously did not.
- `from_entries` on a non-string key — real yq stringifies (`.n | to_entries |
  from_entries` is `{"0":1,"1":2}` in v4.53.3), succinctly raises `Cannot use number (0)
  as object key` in both modes, matching jq 1.7.1. That refusal is now reachable from
  `.a | with_entries(.key = key)`, which yq answers `{"0":1,"1":2}`. A separate yq
  fidelity gap in `from_entries`, deliberately not fixed here.

## #2522: an assignment arm added, and the pin moves to 44

`|=` evaluates its filter at the **target's** position, not at the assignment's
input (yq v4.53.3, on `a: {b: 1, e: 2}`: `.a | .b |= key` is `{"b":"b","e":2}`,
where `.a | .b = key` is `{"b":"a","e":2}`). `eval::update_path` is the only
place that knows the components walked to reach a target, but not where the
value it was handed sits in the document -- so `.a | .b |= path` had to answer
`["a","b"]` with only `["b"]` in hand.

`needs_path_context` therefore gained `Expr::Update`/`Expr::CompoundAssign`/
`Expr::AlternativeAssign` arms (gated on their right side, so an ordinary
`.a |= . + 1` routes exactly as before), and `eval_stage_with_path_context`
gained `A39`, which supplies the two positions involved:

- for a compound assignment, whose right side is evaluated **once against this
  stage's input**, the reads in it are constants for `current_path` and are
  rewritten there -- the same rewrite the owned identity pipe already applies to
  `=`'s right side (#2471);
- for `|=`, the stage's `current_path` becomes an ambient prefix
  (`eval_generic::path_base`) that `update_path` extends with the components it
  walks.

**This is an addition, not a migration.** Nothing moved off the cursor walk, and
no shape that used to reach another arm now reaches `A39`: before it, an
assignment stage fell to the `_` fallback, which evaluates it with no position
at all. `PINNED_ARM_COUNT` goes 43 -> 44 for that reason, which is the "raise it
only in a PR that says why" case rule 2 of "Hygiene going forward" allows.

### Reachability of `A39`, and of the other 43

`A39` instrumented and run against `D`:

| Proof query           | Marker | Gate            | Output      |
|-----------------------|--------|-----------------|-------------|
| `.a \| .b \|= key`      | fired  | `R2` (and `R3`) | `{"b":"b"}` |
| `.a \| .b += key`      | fired  | `R2` (and `R3`) | error (`number (1) and string ("a") cannot be added`) |
| `.c[] \| .x //= key`   | fired  | `R3`            | error (`Cannot index number with string "x"`) |

The listed query for `A39` is the first row. `R3` also holds for it (an
assignment stage is not in `path_context_single_native`), so the row is `R2`
only in the sense every other `R2` row is: that is the disjunct that would fire
on its own.

No other row is starved: none of the 43 proof queries above contains an
assignment operator, so none of them can have moved onto `A39`.

### What the change is measured against

A differential over 4,896 assignment queries (eight heads x four operators x nine
targets x seventeen bodies) on `a: {b: 1, e: 2}`, `n: [1, 2]`,
`d: {x: {y: 3}, z: [4, 5]}`, `s: hi`, `u: null`, comparing the pre-change binary,
the post-change binary and yq v4.53.3: **1,859 answers changed, 1,460 of them
from disagreeing with yq to agreeing with it, and none the other way**
(agreement 2,449 -> 3,909 of 4,896). The 399 that changed without reaching
agreement break down as:

- **228** where both sides raise and only succinctly's own wording moved,
  because the resolved value now appears in it: `.a | .b += path` reports
  `number (1) and array (["a"]) cannot be added` where it used to say
  `array ([])`. yq's wording (`!!seq (a) cannot be added to a !!int (a.b)`) is a
  separate, pre-existing gap.
- **110** where yq's own **lexer** rejects the filter (`{k: key}`, a bare `if`
  inside an update filter), so there is no yq answer to agree with; succinctly
  answers as a superset, exactly as it did for those spellings before.
- **42** where both answer and the values differ. All are `parent` reached
  somewhere succinctly cannot follow: through an auto-vivified chain
  (`.a | .x.y |= (parent|type)` is `""` in yq, `!!map` here -- yq vivifies the
  intermediate node as an untagged null, #2435's gap), or above a value a
  previous stage rebuilt (`.a | to_entries | .[0] | .b += (parent|type)` is
  `!!seq` in yq, `!!map` here -- the eager evaluator climbs the *original*
  document at `current_path`, which `to_entries` has since replaced). Both were
  `null` before the change, so none of the 42 is worse than it was.
- **19** where yq errors and succinctly answers, unchanged in that respect
  before and after.

The jq-mode half of the same method -- 4,900 queries built only from filters real
jq defines (`path(.)`, `[paths]`, `del(...)`, arithmetic), across seven
operators including `/=`/`%=`/`//=` -- reports **0 changed** and 4,900/4,900
agreement with jq 1.7.1 before and after.

## #2558: `?` at the head admitted, and the pin still holds at 44

[#2558](https://github.com/rust-works/succinctly/issues/2558) removed what the
"Notes for the next migration" section below had measured as reason `R2`'s
single biggest source: `path_context_is_navigational` excluded
`Expr::Optional`, so `path_context_absent_split` found an *empty* navigational
head, declined, and `path_context_needs_eager` handed the whole pipe over
whatever followed the `?`.

`?` over navigation is now navigational. The step is one arm in
`path_context_step_generic` -- run the inner step, keep the positions it
reached, drop the error it raised -- which is `path_step_generic`'s own
`Expr::Optional` arm one level up, not a second set of semantics. Two errors
are exempt, the same two the eager evaluator's `Expr::Optional` arm exempts
(`EvalError::is_uncatchable_at_value_position`): a yq-mode negative index still
negative after resolving, and a string-decode failure.
`path_context_fans_out` gained a matching `Expr::Optional` arm, so `.[]?` still
counts as the fan-out it is.

Every row was captured first from Homebrew yq v4.53.3 and `/usr/bin/jq` 1.7.1,
on a flow document and its block spelling, which yq answers identically; the
captures are in the doc comments of `test_optional_head_is_walkable_2558`
(`tests/yq_cli_tests.rs` and `tests/jq_cli_tests.rs`). Of the 128 probed
answers, **four changed and all four moved from disagreeing with yq to
agreeing with it**, none the other way:

| query | before | after | yq v4.53.3 |
|---|---|---|---|
| `.c[-1]? \| key` (flow) | `-1` | `1` | `1` |
| `.c[-1]? \| key` (block) | `-1` | `1` | `1` |
| `.a? \| .b = key` (flow) | `{"b":1}` | `{"b":"a"}` | `{"b":"a"}` |
| `.a? \| .b = key` (block) | `{"b":1}` | `{"b":"a"}` | `{"b":"a"}` |

The first pair is the row
`test_negative_index_out_of_range_survives_path_context_2254` had pinned as
`-1` with the note "this row flips when the eager evaluator retires"; the
second is ADR-0021 decision 7's assignment rule reaching a `?` head for the
first time. `.c[-5]? | key` still raises, which is the row that says the `?`
does not swallow everything.

**`PINNED_ARM_COUNT` stays at 44.** Nine of the table's rows had a `?` head in
their listed proof query (`H1`, `H2`, `H3`, `A07`, `A08`, `A12`, `A14`, `A21`,
`A33`), so each was re-derived with the method above (instrument, run, revert;
instrumentation applied to those nine handlers and to
`path_context_needs_eager`'s three disjuncts, and reverted before this document
was committed). Document `D`:

| Id  | Listed query before            | Re-derived query                            | Gate | Marker | Output       |
|-----|--------------------------------|---------------------------------------------|------|--------|--------------|
| H1  | `.a? \| path + []`             | `.a.b \| . as $x \| path + []`               | R2   | fired  | `["a","b"]`  |
| H2  | `.a? \| key + "x"`             | `.a.b \| . as $x \| key + "x"`               | R2   | fired  | `"bx"`       |
| H3  | `.a? \| file_index + 1`        | `.a.b \| . as $x \| file_index + 1`          | R2   | fired  | `1`          |
| A07 | `.c[.n]? \| key + 1`           | unchanged                                   | R1   | fired  | `1`          |
| A08 | `.a? \| key + "x"`             | `.a? \| . as $x \| key`                      | R2   | fired  | `"a"`        |
| A12 | `.a? \| key == "a" and true`   | `.a.b \| . as $x \| (key and parent)`        | R2   | fired  | `true`       |
| A14 | `.a? \| (key + "x") \| . == "bx"` | `.a.b \| . as $x \| (key + "x") \| . == "bx"` | R2 | fired | `true`      |
| A21 | `.a? \| [key] + ["x"]`         | `.a.b \| . as $x \| [key] + ["x"]`           | R2   | fired  | `["b","x"]`  |
| A33 | `.a? \| (key + "x") \| {z: .}` | `.a.b \| . as $x \| (key + "x") \| {z: .}`   | R2   | fired  | `{"z":"bx"}` |

Both spellings are pinned in
`test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416`, which is what
makes the move readable as "same outputs, different route".

Three things are worth recording about the re-run:

- **`A07` is unaffected on purpose.** Its head is `.c[.n]?` -- `?` over a
  *computed* bracket, which `path_context_is_navigational` still does not admit
  (an absent `PathNode` carries no cursor to evaluate the component against).
  That is #2471's own "next step for this cluster" and is not what #2558
  changed; its gate is `R1`, not `R2`, and both are unmoved. *(That step landed:
  see "#2471 remainder: a computed bracket at the head" below, where `A07`'s
  query is re-derived to `.c[.n]? | . as $x | key` and its gate moves to `R2`.)*
- **`A08` is still reachable, and by a `?` head.** `?` over navigation being
  *walkable* does not make a pipe routable: `.a? | . as $x | key` still hands
  over, because `as` has no `owned_identity_rule`, and the eager evaluator's
  own `Expr::Optional` arm is what steps the `?` once it does. The arm dies
  with the `R2` stage cluster, not with this change.
- **`H1`/`H2`/`A21` fire as a side effect whenever `A08` does.** The eager
  `Expr::Optional` arm appends `path_probe_stage`'s `[path, .]` to isolate the
  inner pipe (#1409), and that probe is an `Expr::Array` containing `path`. The
  re-derived rows above avoid the artefact by using an `as` stage rather than
  a `?` one, so each marker is the handler the row is claiming.

**Nothing became unreachable, so nothing was deleted**, and the reason is
structural rather than a matter of finding another query: `path_context_is_
navigational`, `path_context_fans_out` and `path_context_step_generic` are the
only three functions this change touches, and each gained an `Expr::Optional`
arm and nothing else. A pipe with no `Expr::Optional` anywhere in it is
therefore routed *identically* before and after, which is why the 35 rows
whose listed query has no `?` need no re-derivation and none was done.

### A caveat about the gate instrumentation

`path_context_needs_eager` is consulted in two unrelated places: at the routing
decision, and from `path_context_single_native`'s own `Expr::Pipe` arm, which
asks it of a *nested* pipe. An `eprintln!` per disjunct therefore fires for
filters that never reach the eager evaluator at all -- `.a? | [key] | .[0] |
path` prints `R1` and `R2` and hits no arm. The `ARMHIT` markers, not the
`GATE` ones, are what says a handler ran; the `Gate` column records the
disjunct that fired for a query the markers confirm got there.

## #2471 remainder: a computed bracket at the head, and the pin still holds at 44

The last of [#2471](https://github.com/rust-works/succinctly/issues/2471)'s list, and
the lever every one of the five `R1` rows above named: a *computed* bracket standing at
the **head** of a pipe. `path_context_is_navigational` admitted only a literal
`Expr::Index`, so `.c[.n] | key` left the cursor domain at a stage with no
`owned_identity_rule` and the gate handed the whole pipe over.

`Expr::IndexExpr` is navigation there now. The component is one more value to evaluate
-- against the stage's *own* input, jq's `K as $k | E | .[$k]` model (`.a[.a.b]` reads
`.a.b` from the position the bracket stands at, not from `.a`) -- and each component it
produces is then taken by the walk's **literal** `Expr::Field`/`Expr::Index` step, spelled
as the node that carries it (`path_component_step_expr`). That is the whole of the
change's fidelity argument: every mode-specific indexing rule -- yq's negative-index
resolution (#2254), its numeric index on a mapping (#2459), its scalar target (#2482),
its absent key inside a read-only operand (#2470), and jq's raises for the same three --
stays one definition instead of being re-derived for the computed spelling.

Three shapes are deliberately **not** admitted, each for a reason the walk's own data
model gives:

- **`Expr::SliceExpr` and `getpath(p)`.** Both can land on a value that is not a document
  node: a slice builds a fresh container, and a `getpath` segment may be jq's own
  `{"start":s,"end":e}` descriptor (`eval::getpath_walk_owned`'s `Object`-segment arms).
  A walked stage may have to *emit* the node it stands on (`path_context_emit_node`), and
  `PathNode` carries only a cursor or an absence -- `.a.b | parent | .[0:1]` would emit
  the container instead of the slice. They stay on the owned identity pipe (#2493) and
  the eager evaluator, which carry an owned value.
- **A component that can escape** (`.c[halt] | key`). The walk's step returns
  `Result<(), EvalError>`, which has no room for `Control::Halt`/`Control::Break`;
  #2495's `Control` plumbing lives on the eager route, so a component containing
  `halt`/`halt_error`/`break` stays there and `.c[halt] | key` still exits silently.
- **A component that fans out** (`.c[(0,1)] | tostring | key`). A comma inside the
  component makes the bracket a fan-out head, and a fan-out head standing on *absent*
  positions loses its position once the pipe leaves the walk. That is pre-existing and
  demonstrable with no computed bracket anywhere in the filter: on `c: []`,
  `(.c[0], .c[1]) | key` prints nothing here where yq v4.53.3 answers `0` and `1`.
  Admitting one would have moved `.c[(0,1)] | tostring | key` -- which the eager
  evaluator answers correctly -- onto that gap, so it is refused instead. Measured: the
  admission cost four rows that moved *away* from yq, and refusing it costs nothing (the
  same shapes keep their pre-change answers).

**`PINNED_ARM_COUNT` stays at 44.** Re-run of the method above on the post-change tree,
with the five `R1` arms and the gate's three disjuncts instrumented, each arm fed its
listed proof query (document `D`):

| Id  | Listed query before          | Re-derived query               | Gate | Marker | Output |
|-----|------------------------------|--------------------------------|------|--------|--------|
| A04 | `.c[0:1] \| .[0] \| key + 1`    | unchanged                      | R1   | fired  | `1`    |
| A07 | `.c[.n]? \| key + 1`          | `.c[.n]? \| . as $x \| key`       | R2   | fired  | `0`    |
| A17 | `.a \| getpath(["b"]) \| . as $x \| key` | unchanged           | R1   | fired  | `"b"`  |
| A19 | `.c[.n] \| key + 1`           | `.c[.n] \| . as $x \| key`        | R2   | fired  | `0`    |
| A20 | `.c[.n:.m] \| .[0] \| key + 1`  | unchanged                      | R1   | fired  | `1`    |

`A04`, `A17` and `A20` are unaffected because their heads are the three shapes the
admission refuses. `A07` and `A19` needed re-derivation for the reason every earlier
re-derivation in this document needed one: their listed queries now report **no gate at
all** -- `.c[.n] | key + 1` and `.c[.n]? | key + 1` are answered by the absent route,
because a computed bracket is a navigational head the absent split can take. The
re-derivations keep the same head and put an `as` stage -- which has no
`owned_identity_rule` -- where the arithmetic was, so the reason moves from `R1` to `R2`
while the arm is unchanged. Both spellings are pinned in
`test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416`.

Two more `R1` proof queries were confirmed alongside, both still firing their arm on the
post-change tree: `.c[(0,1)] | key + 1` fires `A19` at `R1` and `.c[(0,1)]? | key + 1`
fires `A07` at `R1` -- the refused fan-out component keeping the original reason intact.

**Nothing became unreachable, so nothing was deleted**, and the reason is structural: the
change adds an `Expr::IndexExpr` arm to `path_context_is_navigational`,
`path_context_fans_out` and `path_context_step_generic` and nothing else, so a pipe with
no `Expr::IndexExpr` anywhere in it is routed *identically* before and after. That is why
the 39 rows whose listed query has no computed bracket need no re-derivation and none was
done. The instrumentation was reverted before this document was committed.

### What the change is measured against

A differential over 2,070 queries (nineteen computed-bracket heads x eighteen tails plus
a tail of assignment, `path()` and nested-bracket shapes) on
`{"a":{"b":1,"e":2},"c":[10,20,30],"n":0,"m":2,"s":"hi","u":null,"d":{...},"k":"b",
"neg":-1}` and on `{"c":[],"a":{},"n":0}`, in both modes and in both YAML and JSON
spellings: **154 answers changed** before the fan-out component was refused, **57 after**.
Classified three-way against yq v4.53.3 on the yq-mode half: **42 moved from disagreeing
with yq to agreeing with it, 11 changed without reaching agreement, and 4 read as moving
away** -- of which two are filters yq's own lexer rejects (`try ... catch`, `{k: key}`),
so there is no yq answer to move away from, and the other two are the *literal*
spelling's own pre-existing gap, reached by the computed spelling for the first time:
on `c: []`, `.c[0] | parent | key | path` and `.c[0] | parent | tostring | key` print
nothing here (and `["c"]` / `"c"` in yq) before this change as much as after it.

The five yq-mode rows the change *fixes* are pinned in
`test_computed_bracket_head_is_walkable_2471` (`tests/yq_cli_tests.rs`), with the capture
for each; the jq-mode half of the same test pins the ten `path(...)` rows real jq 1.7.1
answers, all unchanged.

One walk-versus-bridge divergence is left open rather than pinned, and is recorded in
`test_walk_vs_bridge_path_context_parity_2416`'s own comment: `[.a[("b"+"")] | key]` on a
scalar-valued `.a` is `[]` on the walk (matching yq v4.53.3, and matching succinctly's own
*value* route, which already answers `[]` for `[.a[("b"+"")]]`) and raises `Cannot index
number with string "b"` on the eager route, whose `Expr::IndexExpr` arm never got #2482's
scalar rule. The walk is the side that matches the oracle, so the row is not asserted as
"the routes agree"; the eager arm's missing rule is a separate fix.

## #2563: an `as` binding gets an identity rule, and the pin still holds at 44

[#2563](https://github.com/rust-works/succinctly/issues/2563) is the first of the
`R2` cluster's *stage* shapes -- the ones with no `owned_identity_rule`, which the
"Notes for the next migration" section below names as the remaining move for that
reason. Every re-derived proof query the last three sections produced was spelled
through the same shape (`.a.b | . as $x | key`), because `Expr::As` had no rule: a
binding made `owned_identity_pipe_supported` decline, so `path_context_absent_split`
found no route for `rest` and the gate handed the whole pipe over.

An `as` stage keeps the input's identity for its body -- the body's `.` is the node
the stage stood on, which is what real yq does (`.a.b | . as $x | key` is `"b"` in
v4.53.3, `. as $x | parent | key` is `"a"`) -- and the bound variable is a value.
`eval_owned_identity_as` evaluates the bind source at the stage's own position (its
`key`/`path`/`file_index` reads resolve to constants, the same rewrite a computed
navigation component gets), substitutes each bound value into the body with the
`substitute_bound_var` the generic route already uses, and splices the substituted
body in front of `rest` through the same pipe -- so every stage rule stays a single
definition. `Expr::AsPattern` (destructuring, `?//`) is deliberately **not**
admitted: its alternative-fallthrough rule lives in
`each_pattern_alternatives_generic` and has no second definition here, and real yq's
lexer rejects both spellings outright.

**`PINNED_ARM_COUNT` stays at 44.** Re-run of the method above on the post-change
tree, with all 44 handlers *and* the gate's three disjuncts instrumented, against a
pre-change build of the same worktree (base `b05069ad5`) instrumented identically --
so "moved" is measured, not inferred. Eleven of the 44 listed proof queries stop
reaching any arm; **all eleven arms are still reachable** and the table above now
carries the re-derived query for each (outputs pinned in
`test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416` alongside the old
spellings). Document `D`:

| Id  | Listed query before                         | Re-derived query                                       | Gate | Marker | Output          |
|-----|---------------------------------------------|--------------------------------------------------------|------|--------|-----------------|
| H1  | `.a.b \| . as $x \| path + []`               | `.a.b \| reduce (path) as $p ([]; . + $p) \| . + ["x"]` | R2   | fired  | `["a","b","x"]` |
| H2  | `.a.b \| . as $x \| key + "x"`               | `.a.b \| reduce (key) as $k (""; . + $k) \| . + "x"`    | R2   | fired  | `"bx"`          |
| H3  | `.a.b \| . as $x \| file_index + 1`          | `.a.b \| reduce (file_index) as $f (0; . + $f) \| . + 1` | R2  | fired  | `1`             |
| A07 | `.c[.n]? \| . as $x \| key`                  | `.c[.n]? \| (key and parent)`                           | R2   | fired  | `true`          |
| A08 | `.a? \| . as $x \| key`                      | `.a? \| (key and parent)`                               | R2   | fired  | `true`          |
| A14 | `.a.b \| . as $x \| (key + "x") \| . == "bx"` | `.a.b \| (parent \| length) == 1`                       | R2   | fired  | `true`          |
| A17 | `.a \| getpath(["b"]) \| . as $x \| key`     | `.a \| getpath(["b"]) \| (key and parent)`              | R1   | fired  | `true`          |
| A19 | `.c[.n] \| . as $x \| key`                   | `.c[.n] \| (key and parent)`                            | R2   | fired  | `true`          |
| A21 | `.a.b \| . as $x \| [key] + ["x"]`           | `.a.b \| [(key and parent)] + ["x"]`                    | R2   | fired  | `[true,"x"]`    |
| A26 | `.a.b \| . as $x \| key + "x"`               | `.a.b \| . as $x \| (key and parent)`                   | R2   | fired  | `true`          |
| A33 | `.a.b \| . as $x \| (key + "x") \| {z: .}`   | `.a.b \| . as $x \| (key and parent) \| {z: .}`         | R2   | fired  | `{"z":true}`    |

Two things this run found that the change did **not** cause, recorded because they
were wrong on the page rather than in the code:

- **`A09` and `A13` were already stale.** Their listed query
  (`.a.b | -(key|length)`) reaches *no* arm on the pre-change build either -- the
  gate's `R3` marker fires from `path_context_single_native`'s own nested
  consultation (the caveat below), while the pipe itself is answered by the owned
  identity pipe, which has had an `Expr::Negate` arm since #2471. Both are still
  reachable and both are re-derived above: `A09` on `.a[] | (key | tostring)`
  (`R3`, `"b"`) and `A13` on `.a.b | -(parent|length)` (`R2`, `-1`), a `parent`
  *operand* being the one read no route can name a position for.
- **`A26` is the arm the migration was aimed at and it survives.** `and`/`or` has
  no `owned_identity_rule`, so a body built from one still keeps the binding's
  whole pipe eager, and the eager `Expr::As` arm is what steps the binding once it
  does. The same holds for `map`, `reduce`/`foreach`, `label`/`def` and the bounded
  consumers -- the rest of the `R2` stage cluster. **This change lowers no arm's
  reachability to zero**, and the reason is the same structural one #2472 and #2473
  recorded: an admission removes a *reason* for one stage shape, not the arm.

The other 33 rows report the same disjunct and the same output they reported before,
verified in the same run.

### What the change is measured against

A differential over 2,544 queries per mode (sixteen heads x five binders x
thirty-three bodies, minus the `$x` rows whose binder does not bind one) on
`{"a":{"b":1,"e":2},"c":[10,20],"n":0,"m":1,"s":"hi","u":null}`, comparing the
pre-change binary, the post-change binary and the oracle. In yq mode: **136 answers
changed, 114 of them from disagreeing with yq v4.53.3 to agreeing with it, 5 read as
moving away and 17 changed without reaching agreement.**

All 5 "away" rows and 11 of the 17 "neither" rows have the same head, `.c[-1]`, and
the same cause -- a pre-existing walk defect this change makes reachable rather than
introduces. `path_step_generic`'s `Expr::Index` arm resolves a negative index for the
path *component* (`.c[-1] | key` is `1`, ADR-0021 decision 5) but still looks the
element up with the index **as written** (`usize::try_from(idx)`, which fails for a
negative `idx`), so the position it produces is `PathNode::Absent` and the value
`rest` reads is `null`. It is demonstrable with no `as` anywhere in the filter, on
the pre-change binary: `.c[-1] | [., key]` is `[null,1]` where yq answers `[20,1]`.
The `as` rule routes more shapes through that position, so `.c[-1] | . as $x |
map(key)` goes from `20` (eager, right) to `[]` (walked, wrong). Even on that head
the change is net positive (48 toward, 5 away, 11 neither -- the `key` resolution
dominates); fixing the lookup is a separate correctness change with its own
differential, not part of this migration.

The remaining 6 "neither" rows are two shapes, both sanctioned: four are
`.a.x.y | ... | parent` answering `null` where the eager route answered `{}`, which
ADR-0021 decision 7 records as the same answer for the same position (#2472), and
two are `key | key` after a `getpath` head now applying `OwnedIdentityRule::KeyNode`
(`.a.b | key | key` prints nothing in yq v4.53.3, where the eager evaluator prints
`"b"`).

The jq-mode half of the same matrix reports **168 changed**, of which 68 share the
`.c[-1]` head above. jq 1.7.1 defines none of `key`/`parent`/`path/0`, so there is no
oracle row to classify the rest against; what it *does* define -- `path(...)`,
`paths`, destructuring and `?//` binding -- is pinned unchanged in
`test_as_binding_keeps_the_input_identity_2563` (`tests/jq_cli_tests.rs`), all
thirteen rows captured from `/usr/bin/jq` 1.7.1. The yq-mode captures are in the
same-named test in `tests/yq_cli_tests.rs`, on a flow document and its block
spelling, which yq answers identically.

The instrumentation was reverted before this document was committed; nothing in the
tree carries it.

## Identity pass (spine 2416): every stage shape gets a rule, and the pin still holds at 44

The identity pass closes the `R2` stage cluster the #2563 section left -- every
stage shape with no `owned_identity_rule` -- and most of `R3`: `if`, comma,
`try`/`?`, `label`/`break`, `limit`/`first`/`last`, single-pattern destructuring,
`def`/`DefCall`/`Shared`, a nested pipe and the map-family bodies are run through
the owned identity pipe as stages (`eval_owned_identity_stages`, with a
pair-emitting `OwnedIdentityTail` so a bounded consumer or a `try` can continue
`rest` from each output's identity); `and`/`or` got a `LeftOperand` rule and
string interpolation `Detaches` (both captured from yq v4.53.3), folds and the
compound assignments `Keeps`; a `parent` operand is placed by running the operand
through the pipe; an assignment's right side is evaluated at the position against
the vivified document (#2481) inside the read-only scope (#2470); a nested pipe
that moves before it reads is prefetched at the position
(`PathContextAt::prefetch`). `path_context_single_native` admits anything that
reads no path context, the assignment family and string interpolation
(`eval_positioned_stage_generic`), and a pipe the absent route accepts is never
this evaluator's. A cursor-less pipe takes the owned door instead of being
decided by the gate alone.

**`PINNED_ARM_COUNT` stays at 44.** Re-run of the method above on the post-change
tree with all 44 handlers, the gate's three disjuncts *and* the three handover
sites instrumented (`HANDOVER`, so a depth-1 gate line that is only the absent
route asking about `rest` is not mistaken for a handover), against the base
(`36ed01506`) instrumented identically. **Forty of the 44 listed proof queries
stop reaching any arm** (33 of the 72 distinct queries in
`test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416` report no arm at
all); **all 44 arms are still reachable**, and the table above carries the
re-derived query for each. What re-derives them is the residue this pass leaves
(next section): a computed `parent(n)` (`parent(0+1)`, refused by every route
because the hop count would be evaluated against a position that may not
exist), a fan-out component (`.c[(0,1)]`), a slice head, a `getpath` head.
Outputs are pinned alongside the old spellings. Document `D`:

| Id  | Listed query before | Re-derived query | Gate | Marker | Output |
|-----|---------------------|------------------|------|--------|--------|
| H1 | `.a.b \| reduce (path) as $p ([]; . + $p) \| . + ["x"]` | `.a.b \| parent(0+1) \| path + []` | R2 | fired | `["a"]` |
| H2 | `.a.b \| reduce (key) as $k (""; . + $k) \| . + "x"` | `.a.b \| parent(0+1) \| (key and parent)` | R2 | fired | `true` |
| H3 | `.a.b \| reduce (file_index) as $f (0; . + $f) \| . + 1` | `.a.b \| parent(0+1) \| file_index + 1` | R2 | fired | `1` |
| H4 | `.a.b \| parent + {}` | `.a.b \| parent(0+1) \| (key and parent)` | R2 | fired | `true` |
| A01 | `.a.b \| . \| parent + {}` | `.a.b \| parent(0+1) \| . \| key` | R2 | fired | `"a"` |
| A02 | `.a.b \| parent + {}` | `.a.b \| parent(0+1) + {}` | R2 | fired | `{"b":1}` |
| A03 | `.c[0] \| parent + []` | `.c[0] \| parent(0+1) \| key` | R2 | fired | `"c"` |
| A05 | `.a[] \| (key \| tostring)` | `.a[] \| parent(0+1) \| key` | R3 | fired | `"a"` |
| A06 | `.a.b \| (parent) + {}` | `.a.b \| parent(0+1) \| (key and parent)` | R2 | fired | `true` |
| A07 | `.c[.n]? \| (key and parent)` | `.c[.n]? \| parent(0+1) \| key` | R2 | fired | `"c"` |
| A08 | `.a? \| (key and parent)` | `.a? \| parent(0+1) \| key` | R2 | fired | `null` |
| A09 | `.a[] \| (key \| tostring)` | `.a.b \| parent(0+1) \| -(key\|length)` | R2 | fired | `-1` |
| A10 | `.a.b \| parent + {}` | `.a.b \| parent(0+1) + {}` | R2 | fired | `{"b":1}` |
| A11 | `.a.b \| parent + {}` | `.a.b \| parent(0+1) + {}` | R2 | fired | `{"b":1}` |
| A12 | `.a.b \| . as $x \| (key and parent)` | `.a.b \| parent(0+1) \| (key and parent)` | R2 | fired | `true` |
| A13 | `.a.b \| -(parent\|length)` | `.a.b \| parent(0+1) \| -(key\|length)` | R2 | fired | `-1` |
| A14 | `.a.b \| (parent \| length) == 1` | `.a.b \| parent(0+1) \| (key == "a")` | R2 | fired | `true` |
| A15 | `.a.b \| select(key == "b") \| parent + {}` | `.a.b \| parent(0+1) \| select(key == "a") \| key` | R2 | fired | `"a"` |
| A16 | `.a \| map(key + "x")` | `.a.b \| parent(0+1) \| map(key)` | R2 | fired | `["b"]` |
| A17 | `.a \| getpath(["b"]) \| (key and parent)` | `getpath(["a","b"]) \| (key and parent)` | R1 | fired | `true` |
| A18 | `.a[] \| (key \| length)` | `.a.b \| parent(0+1) \| -(key\|length)` | R2 | fired | `-1` |
| A19 | `.c[.n] \| (key and parent)` | `.c[(0,1)] \| (key and parent)` | R1 | fired | `true, true` |
| A21 | `.a.b \| [(key and parent)] + ["x"]` | `.a.b \| parent(0+1) \| [key] + ["x"]` | R2 | fired | `["a","x"]` |
| A22 | `.a.b \| ("\(key)") \| . + "x"` | `.a.b \| parent(0+1) \| "\(key)"` | R2 | fired | `"a"` |
| A23 | `def f: key; .a.b \| f + "x"` | `.a.b \| parent(0+1) \| def f: key; f + "x"` | R2 | fired | `"ax"` |
| A25 | `.a.b \| def f: key; f + "x"` | `.a.b \| parent(0+1) \| def f: key; f + "x"` | R2 | fired | `"ax"` |
| A26 | `.a.b \| . as $x \| (key and parent)` | `.a.b \| parent(0+1) \| . as $x \| key` | R2 | fired | `"a"` |
| A27 | `.c \| . as [$x] \| key + "x"` | `.c[0] \| parent(0+1) \| . as [$x] \| key` | R2 | fired | `"c"` |
| A28 | `.a.b \| limit(1; key + "x")` | `.a.b \| parent(0+1) \| limit(1; key + "x")` | R2 | fired | `"ax"` |
| A29 | `.a.b \| first(key + "x")` | `.a.b \| parent(0+1) \| first(key + "x")` | R2 | fired | `"ax"` |
| A30 | `.a.b \| last(key + "x")` | `.a.b \| parent(0+1) \| last(key + "x")` | R2 | fired | `"ax"` |
| A31 | `.a.b \| reduce (key) as $k (""; . + $k) \| . + "x"` | `.a.b \| parent(0+1) \| reduce (key) as $k (""; . + $k)` | R2 | fired | `"a"` |
| A32 | `.a.b \| foreach (key) as $k (""; . + $k) \| . + "x"` | `.a.b \| parent(0+1) \| foreach (key) as $k (""; . + $k)` | R2 | fired | `"a"` |
| A33 | `.a.b \| . as $x \| (key and parent) \| {z: .}` | `.a.b \| parent(0+1) \| {z: key}` | R2 | fired | `{"z":"a"}` |
| A34 | `.a.b \| if key == "b" then key + "x" else "y" end` | `.a.b \| parent(0+1) \| if key == "a" then key + "x" else "y" end` | R2 | fired | `"ax"` |
| A35 | `.a.b \| (key + "x"), key` | `.a.b \| parent(0+1) \| (key + "x"), key` | R2 | fired | `"ax", "a"` |
| A36 | `.a.b \| try (key + "x") catch "e"` | `.a.b \| parent(0+1) \| try (key + "x") catch "e"` | R2 | fired | `"ax"` |
| A37 | `.a.b \| label $out \| (key + "x", break $out)` | `.a.b \| parent(0+1) \| label $out \| (key + "x", break $out)` | R2 | fired | `"ax"` |
| A38 | `.a.b \| label $out \| (key + "x", break $out)` | `.a.b \| parent(0+1) \| label $out \| (key + "x", break $out)` | R2 | fired | `"ax"` |
| A39 | `.a \| .b \|= key` | `.a.b \| parent(0+1) \| .b \|= key` | R2 | fired | `{"b":"b"}` |

`H5`, `A04`, `A20` and `A24` keep their listed queries (a computed `parent(n)`
head, the two slice heads and a call with a path-context *argument* -- the
last one is reached through `Expr::Shared`, and #1371's substitution is what
the walk still has no arm for).

### What the change is measured against

The construct matrix in the pass's capture set (`cap/ext1.txt`: every construct
above in a pipe with a read after it, at a present position, an absent one,
inside an `as` body and after a synthesized node), run in both modes against
the base binary and yq v4.53.3: **29 rows moved toward yq, none away**, and 39
changed where yq's lexer rejects the construct (every one of them to the answer
the tree-structural model gives the un-wrapped stage -- `.a.b | (try key catch
"c") | key` prints nothing now because `.a.b | key | key` already did). 212 yq
rows were captured for the constructs yq does accept; 202 match and are pinned
in `IDENTITY_PASS_ROWS_2416` (`tests/yq_cli_tests.rs`), and the 10 that do not
are all pre-existing divergences, identical on the base binary (#2377, #2428,
#2435, container-in-string formatting, `.b |= parent` and `map_values(parent)`
-- the last recorded in `docs/compliance/yq/limitations.md` by this pass, since
it was `{}` before and is a visible artefact now -- and `. |= key`). The sweep
(`scripts/jq-path-context-oracle-sweep.sh`) reports 0 unexpected divergences on
the same manifest.

Three jq-mode extension assertions moved (`.a | (def f: 5; f) | key`,
`.a | "\(key)" | key` and the `"\(key, ..)" | key` rows of #1403), each from
`null` to nothing: jq 1.7.1 has no `key`, the same binary already printed
nothing for `.a | 5 | key` and `.a | "x" | key`, and yq v4.53.3 prints nothing
for `.a.b | "k=\(key)" | key`. The routing pins in
`path_context_needs_eager_pins_the_three_reasons_2416` flipped for every shape
the pass moved.

### What still hands over

Measured with the handover sites instrumented, on the proof queries, the sweep
and the CLI suites, `path_context_needs_eager` answers `true` for exactly these
shapes now:

- a **fan-out head** with a stage after it that is not native at the cursor
  (`.[] | .k | select(key == "k")`, `[.[] | .k | parent] | length`) -- the residue
  ADR-0021 decision 7 records, refused by `path_context_fans_out` at the two
  routing sites;
- a **fan-out component** (`.c[(0,1)] | key`, `.[("a","b")] | key`) and a
  component that can **escape** (`.c[halt] | key`, `.[break $out]`): `R1`,
  because `path_context_is_navigational` refuses both;
- a **slice head** (`.c[0:1] | .[0] | key + 1`, `.c[.n:.m] | ...`) and a
  **`getpath` head** (`getpath(["a","b"]) | key`): `R1`, the third `PathNode`
  variant the previous notes name -- not attempted here, since every one of them
  is answered by the walk's *absence* rather than by any evaluator's model;
- **`..`** (`[.. | key]`): `R1`, #2428's known divergence, untouched;
- a **builtin with no identity rule** (`.a | explode | key`, `ltrimstr(..) |
  [length, key]`, `sub("x";"y") | key`): `R1`;
- a **computed `parent(n)`** (`parent(0+1)`), a **bare `$x` stage** after a
  binding (`. as $x | $x | key`), and a **`repeat` in a fold's INPUT/INIT**: `R2`/`R3`
  by the closed lists;
- the `--eval-all` shapes whose file table cannot be made ambient
  (`FILE_ORIGIN_SCOPE_AVAILABLE`), unchanged.

Nothing else. The instrumentation was reverted before this section was
committed; nothing in the tree carries it.

## Walk residue (spine 2416): every head the walk refused, and the pin still holds at 44

The identity pass's "What still hands over" list above is this section's
scope. Its members were enumerated first, with `path_context_needs_eager`'s
`return true` sites and the three handover sites instrumented (a temporary
trace to a file named by an environment variable, so the CLI suites' own
stderr assertions kept their meaning), over the 44 re-derived proof queries,
the four listed ones, the brief's member list, the sweep and the two CLI
suites. Only a `HANDOVER` line counts (the caveat above: a `GATE` line alone
is the absent route asking about `rest`). The distinct (reason, construct)
pairs the base tree produced:

| Reason | Construct                                                   | Proof queries                                                      |
|--------|-------------------------------------------------------------|--------------------------------------------------------------------|
| `R2`   | a computed `parent(n)` after a head that can miss            | 37 of the 44 re-derived rows (`.a.b \| parent(0+1) \| STAGE`)       |
| `R2`   | a literal `parent` after a head that can miss                | `.a.b \| parent \| key + "x"` (found in the suites; not in the list) |
| `R3`   | a computed `parent(n)` after a head that cannot miss         | `A05`                                                              |
| `R1`   | a slice head, a `getpath` head                               | `A04`, `A20`, `A17`                                                |
| `R1`   | a fan-out component, an escaping component                   | `A19`, `.c[halt] \| key`, `label $out \| .[break $out] \| key`      |
| `R1`   | navigation inside a transparent wrapper at the head          | `(try .[] catch "C") \| key`, `(if true then .[] else empty end) \| key`, `(label $o \| .[]) \| key`, `(def f: .[]; f) \| key`, `first((.[], error("x"))) \| key`, `(.[], empty) \| key` |
| `R1`   | `..`                                                        | `[.. \| key]` (#2428)                                              |
| `R1`   | a builtin with no identity rule                              | `.a \| explode \| key`, `.a \| ltrimstr("x") \| [length, key]`, `.s \| sub("x";"y") \| key` |
| `R1`   | a bare `$x` stage                                            | `. as $x \| $x \| key`                                             |
| `R2`   | a call with a path-context argument (#1371's substitution)   | `A24`                                                              |
| --     | a fan-out head that can miss                                 | `.[] \| .k \| select(key == "k")`, `[.[] \| .k \| parent] \| length` |

A `repeat` in a fold's INIT/SOURCE (the brief's last item) does **not** hand
over on the base: `.a.b | reduce (repeat(.)) as $r (0; . + 1) | key` and
`.a.b | reduce (1) as $r (limit(1; repeat(.)); . + 1) | key` both report no
gate line at all, the fold being answered natively since #2473.

Every member but the last was closed, each by the smallest walk arm or rule
(the captures are in `test_walk_residue_rows_match_yq_2416`,
`tests/yq_cli_tests.rs`, and `test_walk_residue_constructs_jq_2416`,
`tests/jq_cli_tests.rs`; the reference docs and ADR-0021 decision 7's last
paragraph record the rules):

- **The walk's step carries a `Control`** (`path_context_step_generic` returns
  `Result<(), Control>`), so a component that halts or breaks is no longer
  refused (`path_context_component_can_escape` is gone), and the prefix a
  component stream produced before its escape is delivered first
  (`path_context_component_values` returns the values *and* the control), with
  yq mode's rule that the prefix is discarded on an `Error`/`Break` and kept on
  a halt (`path_context_component_escape`, #2371/#2351/#2328).
- **The transparent wrappers are stepped**: `try`/`?` (a `catch` handler's
  outputs stand at the `try`'s own input, as owned nodes), `if` (its condition
  evaluated at the position), `label`/`break`, `def`/a bound call (bound
  exactly as evaluation binds them, up to the identity gate's unfold limit),
  `first`/`limit` (branch by branch over a comma, so `first((.[], halt_error))`
  never evaluates the halt), `last` (`null` at the position when empty),
  `empty`, `error` and the halts. `path_context_is_navigational` admits each
  on the same terms, and `path_context_fans_out`, `step_can_yield_absent` and
  `path_context_stage_preserves_node` follow the body.
- **`PathNode::Owned`**, the third variant: a slice (literal or computed,
  `?` included), `getpath(p)` (every component taken by the literal step that
  spells it, a `{"start":s,"end":e}` segment as jq's own slice) and `..`
  (`path_context_step_recurse`, pre-order, `recurse(.[]?)`'s suppression) are
  navigation. Navigation inside an owned node descends the value with the
  components the owned identity pipe names (`owned_nav_children`, shared with
  `owned_identity_step`), `path_context_absent_identity` builds the identity
  through owned ancestors, and the absent route evaluates `rest` over the
  owned value. The generic evaluator gained a cursor-threading `..` arm on
  both routes (`each_recurse_cursor_generic`), which is what closed #2428's
  sweep rows and the three `path_ctx_recurse_*` goldens.
- **A fan-out component is a fan-out head**: `path_context_fans_out`'s
  `IndexExpr`/`SliceExpr`/`GetPath`/`ParentN` arms read the component
  (`path_context_component_fans_out`, with `[...]`/`{...}` as barriers so
  `getpath(["a","b"])` is one path), the admission no longer excludes it, and
  `path_context_needs_eager` asks `fanout_head_can_lose_position` *first*: a
  head that fans out and can be absent after the fan-out is refused before the
  owned identity pipe's gate can take it. That closed the pre-existing gap the
  admission's comment named: `(.c[0], .c[5]) | tostring | key` answered `0`
  alone on the base (the absent branch left the walk as a `null` with no
  cursor) and answers `0`, `5` now, which is yq v4.53.3's answer.
- **A computed `parent(n)`** is evaluated at the position on every route --
  the walk, `eval_builtin` (with the cursor; `path_context_single_native`
  admits it) and the owned identity pipe (through the prefetch hook, which
  now returns values) -- one hop per output, in order, the fan-out the eager
  evaluator gave `parent((1,2))` (it used to array-collapse the stream,
  `docs/compliance/jq/limitations.md`'s fourth site). A literal `parent` after
  a head that can miss went with it: the absent split walks a head when any
  position it passes *through* can be absent (`head_passes_through_absent`),
  not only when the final one can, and stops short of a stage that fans out
  so the identity route runs the rest per position.
- **Every builtin has an identity rule**, in an exhaustive match: the
  yq-accepted ones captured from v4.53.3 (`all`, `anchor`, `document_index`,
  `line_comment`, `split_doc`, `shuffle`, `trim`, `tonumber`, `to_unix`,
  `match`, `capture`, `sub`, `unique_by`, `omit`, `pick`, every `@format` keep;
  `now`, `env`/`strenv`, `load`, `pivot` detach), `first`/`last`/`nth(n)` as
  the navigation jq defines them as, `min_by`/`max_by` as `Extremum`,
  `input`/`inputs`/`nan`/`infinite`/`builtins`/`$__loc__` detached like a
  literal, `at_offset`/`at_position` node-preserving on the cursor route, and
  the jq-only rest keeping. `Range`/`Repeat`/`While`/`Until` keep, `$ENV` and
  a module call detach.
- **A bare `$x` stage** has `OwnedIdentityRule::Bound`: the input's position
  when the bound value *is* the input (`. as $x | $x`), detached otherwise.
  `Expr::Var` carries the rule before substitution so the gate can see through
  a body; the rows real yq's node-valued variables answer differently are in
  `docs/compliance/yq/limitations.md`.
- **A call with a path-context argument** (`A24`) and any other transparent
  construct with no rewriter arm are evaluated at the position through the
  prefetch hook, where the owned identity pipe binds the call exactly as
  evaluation does; the rewriter gained arms for `IndexExpr`/`SliceExpr`/
  `getpath`/`error`, leaves a map-family stage and a `|=` as written, and its
  `_` arm is an assertion again.

**`PINNED_ARM_COUNT` stays at 44.** Re-run of the method above with the 44
handlers instrumented (`ARMHIT`) on the post-change tree: every listed and
re-derived proof query but the fan-out ones stops reaching any arm, and
**all 44 arms are still reachable** -- through a fan-out head that can miss,
which hands the *whole* pipe to the eager evaluator, so any stage after
`.[] | .b?` runs the arm of its own shape there. The table carries one such
re-derivation per arm (document `D` extended with `"s":"hi"`; `.[] | .b?`
stands on `.a.b`, the position every previous re-derivation used, so the
outputs are the listed ones):

| Id  | Re-derived query (identity pass) | Re-derived query (walk residue) | Gate | Marker | Output |
|-----|----------------------------------|---------------------------------|------|--------|--------|
| H1 | `.a.b \| parent(0+1) \| path + []` | `.[] \| .b? \| path + []` | fan-out | fired | `["a","b"], ["u","b"]` |
| H2 | `.a.b \| parent(0+1) \| (key and parent)` | `.[] \| .b? \| (key and parent)` | fan-out | fired | `true, false` |
| H3 | `.a.b \| parent(0+1) \| file_index + 1` | `.[] \| .b? \| file_index + 1` | fan-out | fired | `1, 1` |
| H4 | `.a.b \| parent(0+1) \| (key and parent)` | `.[] \| .b? \| (key and parent)` | fan-out | fired | `true, false` |
| H5 | `.a.b \| parent(0+1) + {}` | `.[] \| .b? \| parent(0+1) + {}` | fan-out | fired | `{"b":1,"e":2}, {}` |
| A01 | `.a.b \| parent(0+1) \| . \| key` | `.[] \| .b? \| . \| (key and parent)` | fan-out | fired | `true, false` |
| A02 | `.a.b \| parent(0+1) + {}` | `.[] \| .b? \| (key and parent)` | fan-out | fired | `true, false` |
| A03 | `.c[0] \| parent(0+1) \| key` | `.[] \| .[0]? \| (key and parent)` | fan-out | fired | `true, false` |
| A04 | `.c[0:1] \| .[0] \| key + 1` | `.[] \| .[0:1]? \| .[0] \| key + 1` | fan-out | fired | `1` |
| A05 | `.a[] \| parent(0+1) \| key` | `.[] \| .b? \| (key and parent)` | fan-out | fired | `true, false` |
| A06 | `.a.b \| parent(0+1) \| (key and parent)` | `.[] \| .b? \| (key and parent)` | fan-out | fired | `true, false` |
| A07 | `.c[.n]? \| parent(0+1) \| key` | `.[] \| .[(0+0)]? \| (key and parent)` | fan-out | fired | `true, false` |
| A08 | `.a? \| parent(0+1) \| key` | `.[] \| .b? \| (key and parent)` | fan-out | fired | `true, false` |
| A09 | `.a.b \| parent(0+1) \| -(key\|length)` | `.[] \| .b? \| -(key\|length)` | fan-out | fired | `-1, -1` |
| A10 | `.a.b \| parent(0+1) + {}` | `.[] \| .b? \| (parent(0+1) \| .) \| (key and parent)` | fan-out | fired | `true, true` |
| A11 | `.a.b \| parent(0+1) + {}` | `.[] \| .b? \| parent(0+1) + {}` | fan-out | fired | `{"b":1,"e":2}, {}` |
| A12 | `.a.b \| parent(0+1) \| (key and parent)` | `.[] \| .b? \| (key and parent)` | fan-out | fired | `true, false` |
| A13 | `.a.b \| parent(0+1) \| -(key\|length)` | `.[] \| .b? \| -(key\|length)` | fan-out | fired | `-1, -1` |
| A14 | `.a.b \| parent(0+1) \| (key == "a")` | `.[] \| .b? \| (key == "b")` | fan-out | fired | `true, true` |
| A15 | `.a.b \| parent(0+1) \| select(key == "a") \| key` | `.[] \| .b? \| select(key == "b") \| key` | fan-out | fired | `"b", "b"` |
| A16 | `.a.b \| parent(0+1) \| map(key)` | `.[] \| .[0:1]? \| map(key)` | fan-out | fired | `[0]` |
| A17 | `getpath(["a","b"]) \| (key and parent)` | `.[] \| getpath(["b"])? \| (key and parent)` | fan-out | fired | `true, false` |
| A18 | `.a.b \| parent(0+1) \| -(key\|length)` | `.[] \| .b? \| -(key\|length)` | fan-out | fired | `-1, -1` |
| A19 | `.c[(0,1)] \| (key and parent)` | `.c[(0,1)] \| (key and parent)` | fan-out | fired | `true, true` |
| A20 | `.c[.n:.m] \| .[0] \| key + 1` | `.c[(0,1):2] \| .[0] \| (key and parent)` | fan-out | fired | `true, true` |
| A21 | `.a.b \| parent(0+1) \| [key] + ["x"]` | `.[] \| .b? \| [key] + ["x"]` | fan-out | fired | `["b","x"], ["b","x"]` |
| A22 | `.a.b \| parent(0+1) \| "\(key)"` | `.[] \| .b? \| "\(key)"` | fan-out | fired | `"b", "b"` |
| A23 | `.a.b \| parent(0+1) \| def f: key; f + "x"` | `.[] \| .b? \| def f: key; f + "x"` | fan-out | fired | `"bx", "bx"` |
| A24 | `def f(x): x; .a.b \| f(key) + "z"` | `def f(x): x; .[] \| .b? \| f(key) + "z"` | fan-out | fired | `"bz", "bz"` |
| A25 | `.a.b \| parent(0+1) \| def f: key; f + "x"` | `.[] \| .b? \| def f: key; f + "x"` | fan-out | fired | `"bx", "bx"` |
| A26 | `.a.b \| parent(0+1) \| . as $x \| key` | `.[] \| .b? \| . as $x \| key` | fan-out | fired | `"b", "b"` |
| A27 | `.c[0] \| parent(0+1) \| . as [$x] \| key` | `.[] \| .[0:1]? \| . as [$x] \| key` | fan-out | fired | `{"start":0,"end":1}` |
| A28 | `.a.b \| parent(0+1) \| limit(1; key + "x")` | `.[] \| .b? \| limit(1; key + "x")` | fan-out | fired | `"bx", "bx"` |
| A29 | `.a.b \| parent(0+1) \| first(key + "x")` | `.[] \| .b? \| first(key + "x")` | fan-out | fired | `"bx", "bx"` |
| A30 | `.a.b \| parent(0+1) \| last(key + "x")` | `.[] \| .b? \| last(key + "x")` | fan-out | fired | `"bx", "bx"` |
| A31 | `.a.b \| parent(0+1) \| reduce (key) as $k (""; . + $k)` | `.[] \| .b? \| reduce (key) as $k (""; . + $k)` | fan-out | fired | `"b", "b"` |
| A32 | `.a.b \| parent(0+1) \| foreach (key) as $k (""; . + $k)` | `.[] \| .b? \| foreach (key) as $k (""; . + $k)` | fan-out | fired | `"b", "b"` |
| A33 | `.a.b \| parent(0+1) \| {z: key}` | `.[] \| .b? \| {z: key}` | fan-out | fired | `{"z":"b"}, {"z":"b"}` |
| A34 | `.a.b \| parent(0+1) \| if key == "a" then key + "x" else "y" end` | `.[] \| .b? \| if key == "b" then key + "x" else "y" end` | fan-out | fired | `"bx", "bx"` |
| A35 | `.a.b \| parent(0+1) \| (key + "x"), key` | `.[] \| .b? \| (key + "x"), key` | fan-out | fired | `"bx", "b", "bx", "b"` |
| A36 | `.a.b \| parent(0+1) \| try (key + "x") catch "e"` | `.[] \| .b? \| try (key + "x") catch "e"` | fan-out | fired | `"bx", "bx"` |
| A37 | `.a.b \| parent(0+1) \| label $out \| (key + "x", break $out)` | `.[] \| .b? \| label $out \| (key + "x", break $out)` | fan-out | fired | `"bx", "bx"` |
| A38 | `.a.b \| parent(0+1) \| label $out \| (key + "x", break $out)` | `.[] \| .b? \| label $out \| (key + "x", break $out)` | fan-out | fired | `"bx", "bx"` |
| A39 | `.a.b \| parent(0+1) \| .b \|= key` | `(.a, .x) \| .b \|= key` | fan-out | fired | `{"b":"b","e":2}, {"b":"b"}` |

### What the change is measured against

The construct matrix in the pass's capture set (`cap/` in the scratch
directory: slices, fan-out components, comma and paren wrappers, `..`, every
nullary builtin yq's lexer accepts at a present and an absent position and
under `parent`, the argument-taking builtins on a second document, a bare
`$x` after every binding shape, `parent(n)` at every literal) was captured
from yq v4.53.3 and jq 1.7.1 first; 234 yq rows are captured for the
constructs yq accepts, 207 match and are pinned in `WALK_RESIDUE_ROWS_2416`,
and the 27 that do not are all recorded elsewhere (`first(f)` #2377; node-
valued variables, `docs/compliance/yq/limitations.md`; a comma over an
absent branch in yq's context-list model, #2451; the jq-modeled `fromjson`/
`min`/`max`/`sort`/`to_entries` type errors). Against the base binary, 96
of the 288-shape probe corpus moved, every one toward the reference or the
model: the `..` family (48 rows, #2428), the slice family in yq mode (its
container-position rule), `first`/`last`/`nth`/`min_by`/`max_by` to jq's
own `path(f)` model, the detached builtins to nothing, the computed
`parent(n)` fan-out, and the four jq-mode rows where the eager evaluator's
`null` placeholder for `key` at the document root became the walk's nothing
(`.a? | parent(0+1) | key`, `(try error("x") catch key) | key`, `(if key ==
"a" then .a else .c end) | key`, `last(empty) | key` -- #2421's rule, the
move `.a | 5 | key` made in the identity pass). The sweep
(`scripts/jq-path-context-oracle-sweep.sh`) reports 0 unexpected
divergences; its four `descend` manifest rows (`key`/`parent`/`path` under
`..`, and `file_index`'s comma ordering) went stale and were removed.

Tests moved, each with its capture: `test_parent_n_positive_infinite_still_overshoots_to_empty_object_1487`
(`{}` to nothing: yq prints nothing for `.a.b | parent(3)`), the
`.a? | parent(0+1) | key` row of `test_arm_audit_proof_queries_are_unmoved_by_the_gate_2416`
(`null` to nothing, same rule), the `.d[(0+0):(0+1)] | key` and `[.. | path]`
rows of `test_walk_vs_bridge_path_context_parity_2416` (the walk matches the
oracle where the bridge does not: yq's slice keeps the container's position,
and a duplicate mapping key is one position under `..`, `[[],["a"],["d"],
["d",0],["d",1]]` in v4.53.3), and the routing pins in
`path_context_needs_eager_pins_the_three_reasons_2416`,
`path_context_absent_split_pins_its_four_conditions_2416`,
`path_context_gate_admits_emitting_stages_only_last_2416` and
`path_context_absent_resolution_clears_path_context_2416` for every shape the
pass routed.

### What still hands over

Measured with the handover sites instrumented, on the proof queries, the
probe corpus, the sweep and the CLI suites, `path_context_needs_eager`
answers `true` for exactly these shapes now:

- a **fan-out head that can miss**, followed by a read -- `.[] | .k |
  select(key == "k")`, `[.[] | .k | parent] | length`, `.c[(0,5)] | tostring |
  key`, `(.c[0], .c[5]) | tostring | key`, `.[] | .b? | parent` -- the residue
  ADR-0021 decision 7 records, refused by `path_context_fans_out` at the two
  routing sites and by `fanout_head_can_lose_position` at the gate (a fan-out
  component counts, and so does a fan-out head whose *last* stage is the
  `parent` read, since the head then has no `rest` for the absent route);
- `at_offset`/`at_position` followed by a read, in the owned domain only
  (`.a | at_offset(0) | key` raises `at_offset requires document cursor
  context` on every route, base included);
- the `--eval-all` shapes whose file table cannot be made ambient
  (`FILE_ORIGIN_SCOPE_AVAILABLE`), unchanged.

Nothing else. The instrumentation was reverted before this section was
committed; nothing in the tree carries it.

## Result

| Metric                                            | Before | After                 |
|---------------------------------------------------|--------|-----------------------|
| Named handlers in `eval_stage_with_path_context`  | 43     | 44 (`A39`, #2522)     |
| ... proven REACHABLE by a live query               | --     | 44                    |
| ... proven UNREACHABLE                             | --     | 0                     |
| ... neither                                        | --     | 0                     |
| ... whose listed proof query moved off the gate    | --     | 1 (#2473); 14 (#2472); 8 (#2558) |
| ... starved by #2471's remainder                   | --     | 0 (`A16`/`A18` re-run) |
| ... starved by #2522                               | --     | 0 (no row's query assigns) |
| ... starved by #2558                               | --     | 0 (9 `?`-headed rows re-derived) |
| ... starved by #2471's head-of-pipe half           | --     | 0 (`A07`/`A19` re-derived)       |
| ... starved by #2563                                | --     | 0 (11 `as`-spelled rows re-derived) |
| ... starved by the identity pass                    | --     | 0 (40 listed rows re-derived) |
| ... starved by the walk residue                     | --     | 0 (all 44 re-derived through a fan-out head) |
| `PINNED_ARM_COUNT`                                 | 43     | 44                    |

Nothing is deletable at this point in the spine -- and the walk residue is
the measurement that says why: with every *head* the walk refused now carried
(the identity pass had already given every stage a rule), the 44 arms are all
still reached through the one head that stays eager by design, a fan-out that
can miss, which hands the whole pipe over and so runs the arm of every stage
after it. What deletes them is the decision about that head (ADR-0021 decision
8's exit condition), not another walk arm. Doors 2 and 3 are
closed as of step 5, which is a precondition rather than a deletion: the eager evaluator
shrinks when the generic evaluator gains native arms (widening
`path_context_single_native`) and when the absent route widens. What the
closure buys is that `path_context_needs_eager` is now the whole answer to
"which pipes reach an arm" -- so a migration that narrows the gate narrows the
evaluator, with no second route to check.

### Notes for the next migration

- The five `R1` rows (`A04`, `A07`, `A17`, `A19`, `A20`) are the detached-value
  reason. #2471 took it in three passes -- the owned-domain half, the map-family
  and assignment-right-side shapes, and the head-of-pipe computed bracket
  (sections above) -- and all five arms are still reachable after all three.
  What is left of the cluster is the two head shapes the walk's `PathNode`
  cannot carry: `Expr::Slice`/`Expr::SliceExpr` (`A04`, `A20`) and `getpath(p)`
  (`A17`), both of which can stand on a value that is not a document node.
  Giving the walk a third `PathNode` variant for an owned value is what would
  make those reachable by the walk; `A07`/`A19` need the *stage* shapes behind
  a computed head (`as`, and a fan-out component) migrated, not this head
  admission widened.
- `A24` (`Expr::Shared`) is only ever produced by `substitute_func_param`
  (`src/jq/eval.rs`), so it dies with `A23`/`A25` and not before.
- `A38` (`Expr::Break`) cannot be reached without `A37` (`Expr::Label`): the
  proof query is the same one.
- Reachability here is a property of the current tree. Re-run the method above
  (instrument, run, revert) rather than trusting this table after the gate or
  `path_context_single_native` moves. #2472 is the worked example: 14 of the 43
  rows needed a re-derived query and *none* of them was unreachable, so a table
  read without re-running would have claimed 14 deletable arms.
- The `R2` cluster is now the *stage* shapes with no `owned_identity_rule`
  (`and`/`or`, `map`, `reduce`/`foreach`, `label`/`def`, `as PATTERN`, the
  bounded consumers) plus a bare `parent` operand. #2473 gave `and`/`or` and
  `reduce`/`foreach` native `eval_single` arms, which removes `R3` for them but
  **not** `R2`: a native stage still hands over when the head can miss and no
  route can name the position. Giving those stages an `owned_identity_rule` is
  the remaining move for this reason; widening the absent route again buys
  nothing, because it already accepts every `rest` those stages are missing
  from. **`as $var` came off that list with #2563** (section above), and it is
  the worked example of what such a rule buys and what it does not: eleven of
  the 44 proof queries had to be re-derived and not one arm died, because every
  other member of the cluster is still a body an `as` can hold.
- **A defect this page's method surfaced, not caused (#2563's differential).**
  `path_step_generic`'s `Expr::Index` arm resolves a negative index for the path
  *component* but looks the element up with the index as written, so the walk
  stands on `PathNode::Absent` where the element exists: on `c: [10, 20]`,
  `.c[-1] | [., key]` is `[null,1]` here and `[20,1]` in yq v4.53.3, with no
  `as`, no computed bracket and no `?` in the filter. It predates #2563 and is
  reproducible on `b05069ad5`; every route that widens makes it reachable from
  more shapes.
- **`?` at the head was the single biggest source of `R2`; #2558 closed it.**
  It is admitted to `path_context_is_navigational` now, and what it does on a
  *scalar* input is the question that had to be answered: the step is
  suppressed and produces no position at all, which is what both oracles do
  (`jq -c 'path(.s.b?)'` and `yq '.s.b? | key'` both print nothing). What is
  left of the `R2` cluster is the bullet above -- the stage shapes with no
  `owned_identity_rule` -- plus the fan-out head, which is by design.
- The step-5 closure did *not* change what the sweep
  (`scripts/jq-path-context-oracle-sweep.sh`) sees for `file_index`. Its
  yq-mode cases run `succinctly yq FILTER one.yaml two.yaml` with no
  `--eval-all`, and that path evaluates each document independently through
  `evaluate_input`, which has no file-origin table to install -- so the
  manifest's `yq/file_index/*` row (24 divergences) still stands, and the
  `--eval-all` half of #2427 is the half this step closed. Answering
  `file_index` on the ordinary multi-file path needs a per-file scalar, not a
  top-level-index table: `path.first()` there is a key of the document, not a
  file number.
