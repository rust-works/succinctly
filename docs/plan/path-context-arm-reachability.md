# Per-arm reachability of the eager path-context evaluator (spine 2416, step 4)

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

## The three doors into the eager evaluator

The gate is not the only entry, which is why reachability had to be measured
rather than derived from `path_context_needs_eager` alone:

1. **The gate.** `path_context_needs_eager(exprs)` returning `true` in
   `eval_single`'s `Expr::Pipe` arm and in `eval_each_pipe_generic`
   (`src/jq/eval_generic.rs`) bridges the whole pipe to
   `eval::eval_pipe_with_path_context`. This is the door ADR-0021 decision 3
   describes and the one step 5 closes.
2. **`--eval-all`.** `eval::eval_owned_with_file_index` (#715) calls
   `eval_pipe_with_path_context_internal` directly, on `needs_path_context(expr)`
   alone, with no gate at all. Verified live:
   `succinctly yq --eval-all '[.[] | file_index]' f1.yaml f2.yaml` fires
   `H3`, `A05`, `A09` and `A21` without `path_context_needs_eager` being
   consulted.
3. **`eval::eval_pipe`'s own diversion.** `eval_pipe` still routes any pipe with
   `needs_path_context` into `eval_pipe_with_path_context`. `jq::eval` no longer
   reaches it (ADR-0021 decision 4 sends path-context queries to the generic
   evaluator first), but the generic evaluator's own bridges back into this file
   (`eval_on_owned` / `bridge_to_full_evaluator` -> `eval_full`) can, so the
   diversion is live code, not a dead branch behind the entry point.

Doors 2 and 3 mean that lowering the arm count is gated on more than
`path_context_needs_eager`: the arms have to stop being reachable through all
three.

## Gate reasons

`path_context_needs_eager`'s single condition is three disjuncts, tested in this
order; the `Gate` column names the first that fired for that query.

| Id  | Disjunct                             | Meaning                                                                             |
|-----|--------------------------------------|-------------------------------------------------------------------------------------|
| R1  | `!is_node`                           | the value reaching the path-context stage is detached -- no cursor to read a key from |
| R2  | `can_absent && !absent_routed`       | the navigational head can miss, and `path_context_absent_split` declined the pipe     |
| R3  | `!path_context_stage_native(stage)`  | the stage is built from a construct with no native cursor-threading arm               |

R2 dominates the table because the natural probe shape starts `.a.b`, and a
`.field` step can always miss. The same shapes with a head that cannot miss trip
R3 instead -- e.g. `.a[] | key + "x"`, `.a[] | if key == "b" then key + "x" else
"y" end` and `.a[] | reduce (key) as $k (""; . + $k) | . + "x"` all report R3.
This is an ordering artefact, not a claim that R3 is rare.

## The 43 handlers

| #   | Handler (site in `eval_stage_with_path_context`)                   | Verdict   | Proof query (jq mode, document `D`)                   | Gate |
|-----|--------------------------------------------------------------------|-----------|-------------------------------------------------------|------|
| H1  | pre-match `if matches!(first, Expr::Builtin(Builtin::PathNoArg))`  | REACHABLE | `.a.b \| path + []`                                   | R2   |
| H2  | pre-match `if matches!(first, Expr::Builtin(Builtin::Key))`        | REACHABLE | `.a.b \| key + "x"`                                   | R2   |
| H3  | pre-match `if matches!(first, Expr::Builtin(Builtin::FileIndex))`  | REACHABLE | `.a.b \| file_index + 1`                              | R2   |
| H4  | pre-match `if matches!(first, Expr::Builtin(Builtin::Parent))`     | REACHABLE | `.a.b \| parent + {}`                                 | R2   |
| H5  | pre-match `if let Expr::Builtin(Builtin::ParentN(n_expr)) = first` | REACHABLE | `.a.b \| parent(0+1) + {}`                            | R2   |
| A01 | `Expr::Identity`                                                   | REACHABLE | `.a.b \| . \| key + "x"`                              | R2   |
| A02 | `Expr::Field(name)`                                                | REACHABLE | `.a.b \| key + "x"`                                   | R2   |
| A03 | `Expr::Index { idx, key }`                                         | REACHABLE | `.c[0] \| key + 1`                                    | R2   |
| A04 | `Expr::Slice { .. }`                                               | REACHABLE | `.c[0:1] \| .[0] \| key + 1`                          | R1   |
| A05 | `Expr::Iterate`                                                    | REACHABLE | `.a[] \| key + "x"`                                   | R3   |
| A06 | `Expr::Paren(inner)`                                               | REACHABLE | `(.a.b) \| key + "x"`                                 | R2   |
| A07 | `Expr::Optional(inner) if IndexExpr/SliceExpr`                     | REACHABLE | `.c[.n]? \| key + 1`                                  | R1   |
| A08 | `Expr::Optional(inner)`                                            | REACHABLE | `.a? \| key + "x"`                                    | R2   |
| A09 | `Expr::Pipe(inner) if rest.is_empty()`                             | REACHABLE | `.a.b \| -(key\|length)`                              | R2   |
| A10 | `Expr::Pipe(inner)`                                                | REACHABLE | `.a.b \| key + "x"`                                   | R2   |
| A11 | `Expr::Arithmetic { .. }`                                          | REACHABLE | `.a.b \| key + "x"`                                   | R2   |
| A12 | `Expr::And(..) \| Expr::Or(..)`                                    | REACHABLE | `.a.b \| key == "b" and true`                         | R2   |
| A13 | `Expr::Negate(operand)`                                            | REACHABLE | `.a.b \| -(key\|length)`                              | R2   |
| A14 | `Expr::Compare { .. }`                                             | REACHABLE | `.a.b \| (key + "x") \| . == "bx"`                    | R2   |
| A15 | `Expr::Builtin(Builtin::Select(cond))`                             | REACHABLE | `.a.b \| select(key == "b") \| key + "x"`             | R2   |
| A16 | `Expr::Builtin(Builtin::Map(f))`                                   | REACHABLE | `.a \| map(key + "x")`                                | R2   |
| A17 | `Expr::Builtin(Builtin::GetPath(path_expr))`                       | REACHABLE | `.a \| getpath(["b"]) \| . as $x \| key`              | R1   |
| A18 | `Expr::Builtin(_)`                                                 | REACHABLE | `.a.b \| (key + "x") \| length`                       | R2   |
| A19 | `Expr::IndexExpr { target, key }`                                  | REACHABLE | `.c[.n] \| key + 1`                                   | R1   |
| A20 | `Expr::SliceExpr { target, start, end }`                           | REACHABLE | `.c[.n:.m] \| .[0] \| key + 1`                        | R1   |
| A21 | `Expr::Array(inner) if needs_path_context(inner)`                  | REACHABLE | `.a.b \| [key] + ["x"]`                               | R2   |
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
| A33 | `Expr::Object(_) \| Expr::Array(_) \| Expr::Literal(_)`            | REACHABLE | `.a.b \| (key + "x") \| {z: .}`                       | R2   |
| A34 | `Expr::If { .. }`                                                  | REACHABLE | `.a.b \| if key == "b" then key + "x" else "y" end`   | R2   |
| A35 | `Expr::Comma(exprs)`                                               | REACHABLE | `.a.b \| (key + "x"), key`                            | R2   |
| A36 | `Expr::Try { .. }`                                                 | REACHABLE | `.a.b \| try (key + "x") catch "e"`                   | R2   |
| A37 | `Expr::Label { name, body }`                                       | REACHABLE | `.a.b \| label $out \| (key + "x", break $out)`       | R2   |
| A38 | `Expr::Break(name)`                                                | REACHABLE | `.a.b \| label $out \| (key + "x", break $out)`       | R2   |

## Result

| Metric                                            | Before | After |
|---------------------------------------------------|--------|-------|
| Named handlers in `eval_stage_with_path_context`  | 43     | 43    |
| ... proven REACHABLE by a live query               | --     | 43    |
| ... proven UNREACHABLE                             | --     | 0     |
| ... neither                                        | --     | 0     |
| `PINNED_ARM_COUNT`                                 | 43     | 43    |

Nothing is deletable at this point in the spine. The eager evaluator shrinks
when the generic evaluator gains native arms (widening
`path_context_single_native`), when the absent route widens, and when doors 2
and 3 above are closed -- not before.

### Notes for the next migration

- The five `R1` rows (`A04`, `A07`, `A17`, `A19`, `A20`) are the detached-value
  reason and are the cheapest cluster to attack: `Expr::Slice`, `Expr::IndexExpr`
  and `Expr::SliceExpr` are missing from `path_context_single_native` even though
  their literal-bound siblings (`Expr::Index`, `Expr::Slice` with folded bounds)
  are navigational.
- `A24` (`Expr::Shared`) is only ever produced by `substitute_func_param`
  (`src/jq/eval.rs`), so it dies with `A23`/`A25` and not before.
- `A38` (`Expr::Break`) cannot be reached without `A37` (`Expr::Label`): the
  proof query is the same one.
- Reachability here is a property of the current tree. Re-run the method above
  (instrument, run, revert) rather than trusting this table after the gate or
  `path_context_single_native` moves.
