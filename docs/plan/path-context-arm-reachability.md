# Per-arm reachability of the eager path-context evaluator (spine 2416, steps 4-5)

`eval_stage_with_path_context` (`src/jq/eval.rs`) is the eager, materialising
path-context evaluator ADR-0021 is retiring. It handles 43 expression shapes by
name -- 5 pre-match `if` handlers plus 38 arms of its top-level `match first`
(the split `tests/jq_path_context_arm_guard.rs` pins) -- and each one was added
to fix a shape the generic evaluator could not answer.

Step 4 asks, of every one of those 43: is there still a query that reaches it?

**Answer: all 43 are reachable.** None was deleted; `PINNED_ARM_COUNT` stays at
43. The rest of this page is the evidence.

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
stages after it. The probe shape that trips it is `.a?` (which
`path_context_is_navigational` excludes, so no position is walked) or a bare
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

## The 43 handlers

| #   | Handler (site in `eval_stage_with_path_context`)                   | Verdict   | Proof query (jq mode, document `D`)                   | Gate |
|-----|--------------------------------------------------------------------|-----------|-------------------------------------------------------|------|
| H1  | pre-match `if matches!(first, Expr::Builtin(Builtin::PathNoArg))`  | REACHABLE | `.a? \| path + []`                                    | R2   |
| H2  | pre-match `if matches!(first, Expr::Builtin(Builtin::Key))`        | REACHABLE | `.a? \| key + "x"`                                    | R2   |
| H3  | pre-match `if matches!(first, Expr::Builtin(Builtin::FileIndex))`  | REACHABLE | `.a? \| file_index + 1`                               | R2   |
| H4  | pre-match `if matches!(first, Expr::Builtin(Builtin::Parent))`     | REACHABLE | `.a.b \| parent + {}`                                 | R2   |
| H5  | pre-match `if let Expr::Builtin(Builtin::ParentN(n_expr)) = first` | REACHABLE | `.a.b \| parent(0+1) + {}`                            | R2   |
| A01 | `Expr::Identity`                                                   | REACHABLE | `.a.b \| . \| parent + {}`                            | R2   |
| A02 | `Expr::Field(name)`                                                | REACHABLE | `.a.b \| parent + {}`                                 | R2   |
| A03 | `Expr::Index { idx, key }`                                         | REACHABLE | `.c[0] \| parent + []`                                | R2   |
| A04 | `Expr::Slice { .. }`                                               | REACHABLE | `.c[0:1] \| .[0] \| key + 1`                          | R1   |
| A05 | `Expr::Iterate`                                                    | REACHABLE | `.a[] \| (key \| tostring)`                            | R3   |
| A06 | `Expr::Paren(inner)`                                               | REACHABLE | `.a.b \| (parent) + {}`                               | R2   |
| A07 | `Expr::Optional(inner) if IndexExpr/SliceExpr`                     | REACHABLE | `.c[.n]? \| key + 1`                                  | R1   |
| A08 | `Expr::Optional(inner)`                                            | REACHABLE | `.a? \| key + "x"`                                    | R2   |
| A09 | `Expr::Pipe(inner) if rest.is_empty()`                             | REACHABLE | `.a.b \| -(key\|length)`                              | R2   |
| A10 | `Expr::Pipe(inner)`                                                | REACHABLE | `.a.b \| parent + {}`                                 | R2   |
| A11 | `Expr::Arithmetic { .. }`                                          | REACHABLE | `.a.b \| parent + {}`                                 | R2   |
| A12 | `Expr::And(..) \| Expr::Or(..)`                                    | REACHABLE | `.a? \| key == "a" and true`                          | R2   |
| A13 | `Expr::Negate(operand)`                                            | REACHABLE | `.a.b \| -(key\|length)`                              | R2   |
| A14 | `Expr::Compare { .. }`                                             | REACHABLE | `.a? \| (key + "x") \| . == "bx"`                     | R2   |
| A15 | `Expr::Builtin(Builtin::Select(cond))`                             | REACHABLE | `.a.b \| select(key == "b") \| parent + {}`           | R2   |
| A16 | `Expr::Builtin(Builtin::Map(f))`                                   | REACHABLE | `.a \| map(key + "x")`                                | R2   |
| A17 | `Expr::Builtin(Builtin::GetPath(path_expr))`                       | REACHABLE | `.a \| getpath(["b"]) \| . as $x \| key`              | R1   |
| A18 | `Expr::Builtin(_)`                                                 | REACHABLE | `.a[] \| (key \| length)`                             | R3   |
| A19 | `Expr::IndexExpr { target, key }`                                  | REACHABLE | `.c[.n] \| key + 1`                                   | R1   |
| A20 | `Expr::SliceExpr { target, start, end }`                           | REACHABLE | `.c[.n:.m] \| .[0] \| key + 1`                        | R1   |
| A21 | `Expr::Array(inner) if needs_path_context(inner)`                  | REACHABLE | `.a? \| [key] + ["x"]`                                | R2   |
| A22 | `Expr::StringInterpolation(parts) if ..`                           | REACHABLE | `.a.b \| ("\(key)") \| . + "x"`                       | R2   |
| A23 | `Expr::DefCall { .. }`                                             | REACHABLE | `def f: key; .a.b \| f + "x"`                         | R2   |
| A24 | `Expr::Shared(inner)`                                              | REACHABLE | `def f(x): x; .a.b \| f(key) + "z"`                   | R2   |
| A25 | `Expr::FuncDef { .. }`                                             | REACHABLE | `.a.b \| def f: key; f + "x"`                         | R2   |
| A26 | `Expr::As { .. } if ..`                                            | REACHABLE | `.a.b \| . as $x \| key + "x"`                        | R2   |
| A27 | `Expr::AsPattern { .. } if ..`                                     | REACHABLE | `.c \| . as [$x] \| key + "x"`                        | R2   |
| A28 | `Expr::Limit { n, expr } if ..`                                    | REACHABLE | `.a.b \| limit(1; key + "x")`                         | R2   |
| A29 | `Expr::FirstExpr(expr) if ..`                                      | REACHABLE | `.a.b \| first(key + "x")`                            | R2   |
| A30 | `Expr::LastExpr(expr) if ..`                                       | REACHABLE | `.a.b \| last(key + "x")`                             | R2   |
| A31 | `Expr::Reduce { .. } if ..`                                        | REACHABLE | `.a.b \| reduce (key) as $k (""; . + $k) \| . + "x"`  | R2   |
| A32 | `Expr::Foreach { .. } if ..`                                       | REACHABLE | `.a.b \| foreach (key) as $k (""; . + $k) \| . + "x"` | R2   |
| A33 | `Expr::Object(_) \| Expr::Array(_) \| Expr::Literal(_)`            | REACHABLE | `.a? \| (key + "x") \| {z: .}`                        | R2   |
| A34 | `Expr::If { .. }`                                                  | REACHABLE | `.a.b \| if key == "b" then key + "x" else "y" end`   | R2   |
| A35 | `Expr::Comma(exprs)`                                               | REACHABLE | `.a.b \| (key + "x"), key`                            | R2   |
| A36 | `Expr::Try { .. }`                                                 | REACHABLE | `.a.b \| try (key + "x") catch "e"`                   | R2   |
| A37 | `Expr::Label { name, body }`                                       | REACHABLE | `.a.b \| label $out \| (key + "x", break $out)`       | R2   |
| A38 | `Expr::Break(name)`                                                | REACHABLE | `.a.b \| label $out \| (key + "x", break $out)`       | R2   |

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

## Result

| Metric                                            | Before | After                 |
|---------------------------------------------------|--------|-----------------------|
| Named handlers in `eval_stage_with_path_context`  | 43     | 43                    |
| ... proven REACHABLE by a live query               | --     | 43                    |
| ... proven UNREACHABLE                             | --     | 0                     |
| ... neither                                        | --     | 0                     |
| ... whose listed proof query moved off the gate    | --     | 1 (#2473); 14 (#2472) |
| `PINNED_ARM_COUNT`                                 | 43     | 43                    |

Nothing is deletable at this point in the spine. Doors 2 and 3 are closed as
of step 5, which is a precondition rather than a deletion: the eager evaluator
shrinks when the generic evaluator gains native arms (widening
`path_context_single_native`) and when the absent route widens. What the
closure buys is that `path_context_needs_eager` is now the whole answer to
"which pipes reach an arm" -- so a migration that narrows the gate narrows the
evaluator, with no second route to check.

### Notes for the next migration

- The five `R1` rows (`A04`, `A07`, `A17`, `A19`, `A20`) are the detached-value
  reason and are the cheapest cluster to attack: `Expr::Slice`, `Expr::IndexExpr`
  and `Expr::SliceExpr` are missing from `path_context_single_native` even though
  their literal-bound siblings (`Expr::Index`, `Expr::Slice` with folded bounds)
  are navigational. #2471 (section above) took the *owned-domain* half of that
  cluster and left all five arms reachable: what remains is the head-of-pipe
  half, which needs the cursor walk to evaluate a computed component.
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
  (`and`/`or`, `map`, `reduce`/`foreach`, `as`/`label`/`def`, the bounded
  consumers) plus a bare `parent` operand. #2473 gave `and`/`or` and
  `reduce`/`foreach` native `eval_single` arms, which removes `R3` for them but
  **not** `R2`: a native stage still hands over when the head can miss and no
  route can name the position. Giving those stages an `owned_identity_rule` is
  the remaining move for this reason; widening the absent route again buys
  nothing, because it already accepts every `rest` those stages are missing
  from.
- **`?` at the head is now the single biggest source of `R2`.** It is excluded
  from `path_context_is_navigational` on purpose (`?` in path context is not
  `?` in a path expression), so `path_context_absent_split` finds an empty head
  and declines, and the pipe goes eager whatever follows. Teaching the walk to
  step `.a?` -- which means deciding what it does on a *scalar* input, where
  `.a` errors and `.a?` yields nothing -- is what would empty that cluster.
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
