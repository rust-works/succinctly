# The producer contract for jq's path register (#3456)

**Status: proposed (A2 of [#3456](https://github.com/rust-works/succinctly/issues/3456)).** This
note is the design review gate before B1 (the type change). It re-confirms the inventory on
`main` at `ea2fcd19c`, settles the questions the issue left open, and lists what B1/B2/B3 must
keep green. Functions are cited by name, not line: `src/jq/eval.rs` moves too fast for line
numbers to survive a review round. The doc comments on `PathBranch`, `Frame`,
`cannot_move_register`, `register_after` and `reestablishes_register` are the primary source;
this note is the connective tissue between them.

## 1. The defect

jq's `path()` carries a `(path, value_at_path)` register that only *navigation* advances.
`PATH_END` and every `INDEX` compare the current value with `jv_identical` against it. To
model that, `resolve_node_sink` hands each branch to its consumer with two carriers:

- `PathBranch::register: Option<Cow<OwnedValue>>` -- producer to the next stage's consumer;
- `Frame { register: Option<Rc<OwnedValue>>, register_loss: RegisterLoss }` -- a stage to its
  nested routes and to every later stage. `RegisterLoss` is `Kept | LostAt(v) | LostSomewhere`.

`PathBranch::register` is documented as "the register jq holds, still live at this path". What
the main producer, `untracked_at_register`, actually stores is the register the leaf
**entered** with. That is jq's register afterwards only if the leaf did not move it, and
nothing on the branch says whether it did. The answer lives in the *consumers*: in
`resolve_seq_stage` as `stage_preserves_register` (`cannot_move_register`, the `[E]` rule,
`getpath_preserves_register`) and, since #3425, in `register_after` /
`register_movement_tracked`. Every consumer that is not `resolve_seq_stage` reads the entry
register raw, and each has had to rediscover the rule (#3289's two review rounds: 1,535 and 15
confirmed writes where jq refuses).

The branch field also conflates two states. `None` means both "no live register entered this
leaf, so every refusal is exact" and "a register entered and its whereabouts are unknown".
`resolve_from_restored_input` already relies on the second reading (`lost = register.is_none()`).

## 2. Inventory (re-confirmed on `ea2fcd19c`)

Counts are roles, not grep hits. The issue's first plan over-counted
`untracked_at_register`'s callers (8); there are three.

### Producers of `PathBranch::register` (`Some`)

| #   | Site                                                                                                                | Records                                      | Truthful today?                                                                                                   |
|-----|---------------------------------------------------------------------------------------------------------------------|----------------------------------------------|-------------------------------------------------------------------------------------------------------------------|
| P1  | `untracked_at_register`; callers `untracked_branches`, the bounded leaf in `resolve_leaf`, `forward_drained_result` | the **entry** register, when `trackable`     | **No.** True only if the leaf did not navigate. Nothing records whether it did.                                   |
| P2  | `forward_drained_result`; callers `Array` (`[E]`, #3263), `LastExpr`/`LastStream`, `IsEmpty`, `UpperIndexStream`    | entry register, via P1                       | **No**, and each has its own jq-side register behaviour (section 5).                                              |
| P3  | `resolve_seq_sink` seed (#3133, #2046)                                                                              | the external register, or `frame.register()` | Yes if the frame was truthful: it is the pipe's own input.                                                        |
| P4  | `resolve_as_pattern` seeds (#2649, #3120)                                                                           | the register at the pattern walk's position  | Yes if the walk is exact. The two seeds are the `Some(reg) if !trackable` arm and the non-root `passthrough` arm. |
| P5  | `resolve_catch_sink` seed (#3133)                                                                                   | the `Try` arm's **entry** register           | Yes for a jq-raised error (`FORK_OPT` restores the path). Not for a resolver-guessed refusal; see D4.3.           |
| P6  | `resolve_seq_stage`'s `place_step`                                                                                  | propagates P1-P5 via `carry_register`        | This is where truth is **decided** today. It is a consumer.                                                       |
| P7  | `computed_at_register` (#3289), called from the `And`/`Or`/`Negate` arms                                            | the register `register_after` computed       | Only as good as `register_after`, which re-derives it consumer-side.                                              |
| P8  | `resolve_from_restored_input` (two seeds plus a lost frame)                                                         | the register at `L`'s branch                 | Same as P7. Its `None` means "lost", not "no register".                                                           |

Not producers: `apply_static_tail_one` (drops the register on a non-empty tail), `wrap_optional_branch`
(`?`; carries it), `into_owned_value` (carries it). Four sites in the recurse family discard
it (`register: _` / `None`); that is safe today only because loss rides the `Frame`.

### Consumers

| #   | Consumer                                                                                                                                                        | Reads                                          | Trust rule today                                                 |
|-----|-----------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------|------------------------------------------------------------------|
| C1  | `resolve_seq_stage`: `reestablishes_register`, `carry_register`                                                                                                 | `carried_register`, `step_register`            | filtered by `stage_preserves_register` -- **consumer-side**      |
| C2  | `resolve_seq_stage`: the `register_loss` derivation                                                                                                             | `register_was_live`, `register_entering_value` | sets `Frame::register_loss`                                      |
| C3  | the five `Frame::register()` readers: `Try`'s `entry_register`, `Pipe`'s seed, `resolve_catch_sink`, `resolve_seq_sink`'s seed, `step_may_reestablish`          | the frame's register                           | none: they trust what the stage handed down                      |
| C4  | `FoldRegister::enter` / `resolve` (`update_frame.with_register`)                                                                                                | INIT's trackability, `self.value`              | assumes SOURCE never moved the register (#2896's `getpath` case) |
| C5  | `guess_refusal`, `guess_refusal_flow` (about 20 call sites)                                                                                                     | `Frame::register_loss`                         | where "lost" is consumed                                         |
| C6  | `array_contents_are_checked`, `array_resolves_live`                                                                                                             | static                                         | admits `[E]` into C1                                             |
| C7  | `register_after`, `computed_at_register`, `register_movement_tracked`, `and_or_negate_resolves_live`, the `stage_reports_register` block in `resolve_seq_stage` | `branch.register`, `frame.register()`          | an allowlist plus special cases; the code that did not converge  |
| C8  | `Frame::certifies_value` via `register_identical` / `marker_identical` (#3494)                                                                                  | the marker's bind position vs the frame's      | jq mode: an empty array is identical only at the same position   |

**The core defect in one sentence:** "did the register survive this leaf?" is answered in C1
and C7, the consumers; C3 and C4 read P1's entry register without asking it.

## 3. Design

### D1. One vocabulary on the branch

```rust
enum BranchRegister<'a> {
    /// No live register entered this leaf: every refusal is exact.
    None,
    /// The leaf provably did not move jq's register: this is exactly
    /// `value_at_path` at the branch's prefix.
    Unmoved(Cow<'a, OwnedValue>),
    /// A live register entered and the leaf may have moved it: jq's register
    /// is this node or one inside it (it only moves down, #3267).
    LostAt(Cow<'a, OwnedValue>),
    /// A live register entered and nothing is known about where it went.
    LostSomewhere,
}
```

`Unmoved` is today's `Some` *after* C1's filter. `LostAt` / `LostSomewhere` correspond
one-for-one to `RegisterLoss`. A trackable branch is `None` (the existing `with_register`
`debug_assert!` holds unchanged).

### D2. Layered, not merged

`Frame::register_loss` stays the downstream carrier; `BranchRegister` is the producer's
output; `place_step` is the **single** conversion point from branch-level loss to
`Frame::register_loss`, as it already is via `register_entering_value`. Reasons:

- C5 has about 20 readers of `frame.register_loss`. Merging moves all of them for no behaviour.
- Loss must survive routes that rebuild a branch from scratch: the four recurse-family
  discards and `PathBranch::new` / `passthrough` / `untracked` / `marked` / `positioned` (all
  write `register: None`). On the `Frame` loss is already sticky; on the branch every one of
  those constructors would have to carry it, and one missed site silently makes a guessed
  refusal catchable again -- #3186 / #3267's refuse-only-under-try failure.

**An invariant the code implies, and what B1 found when it asserted it.** The claim was that
`Frame::register.is_some()` and `register_loss.is_lost()` are mutually exclusive. It is
**false as stated.** B1 put a `debug_assert!` on every `Frame` constructor that can combine the
two and ran the jq suite: eight tests tripped it (`test_path_catch_handler_*_843`,
`_3133`, `_2978`, `_1297`). The violating frames are the *handoff* frame `place_step` builds
with `with_register_loss` for the next stage, and that frame after `extend`. Both still carry
the register the stage was *entered* with while recording a new loss. The next stage replaces
the register through `with_register` (its `stage_frame`), and neither the suite nor the sweep
showed a reader of a mixed frame -- tested, not proven by construction. The `And`/`Or` arm's `frame.with_register(register)` was the candidate
counterexample; it did not fire.

What does hold is narrower, and is what B1 asserts: **a frame built by `Frame::with_register`
-- the one a stage's readers consult -- never carries a register while recording it lost.** It
held across the jq suite and the full A1 sweep (94,887 rows, built with debug assertions, no
panic and no flipped row). Two consequences for B2:

- a reader should take the register from `stage_frame`, not from the handoff `frame`, whose
  `register` is stale by design;
- the layering argument stands (the two fields stay separate), but the handoff frame is a
  place where `register` is stale by design, which is one more reason `place_step` is the
  single conversion point.

Collapsing the two frame fields into one enum remains a follow-up, not this issue.

### D3. The survival rule moves into the producer

`untracked_at_register(expr, computed, trackable, value)` takes the stage's expression:

- `!trackable` -> `None` (the untracked case's register is carried by `place_step` / the frame,
  as today);
- `trackable && preserves(expr)` -> `Unmoved(entry)`;
- `trackable && !preserves(expr)` -> `LostAt(entry)`.

`preserves` is today's `stage_preserves_register`, lifted verbatim: `cannot_move_register(expr)`,
or the `[E]` rule (`array_resolves_live` and `array_contents_are_checked`), or
`getpath_preserves_register`. C1 then trusts the branch: `carry_register` loses its
`!facts.stage_preserves_register` arm, and `StepRegisterFacts::stage_preserves_register` is
deleted. `cannot_move_register` also stays in use by `FoldRegister::advance` and
`FoldRegister::relocate`'s `identical_eligible` (#2860); D7 covers the fold.

P7/P8 follow the same shape: `register_after`'s four-way decision becomes "read the branch's
`BranchRegister`", and `computed_at_register` becomes a conversion from it.

### D4. "Could it" versus "did it"

`cannot_move_register` is a static *could it*. It is sound as a producer rule (cannot implies
did not) and incomplete only in the refuse direction. A per-branch *did it* is needed where the
static answer depends on something the expression alone cannot see. There are four such cases;
all exist as per-branch code today, so none is new:

1. `getpath`: whether the input *is* the register (#2896, `getpath_preserves_register`).
2. `[E]`: whether the array resolves live, which depends on input trackability (#3263).
3. `Try` / `catch`: backtracking restores the path at the fork, so the handler's entry
   register is `Unmoved(entry)` **only for a jq-raised error**. A resolver-guessed refusal
   (`guess_refusal`) is already uncatchable and has to stay so on the catch route -- the
   `Try` arm catches any `EvalEscape::Error` that `!is_uncatchable()`, so the note asserts B2
   must add a negative test that a lost-frame refusal reaches no handler.
4. `isempty(g)`: moves the register **iff `g` emits** (section 5). Defaulted to `LostAt`; a
   per-branch promotion is optional and only with its oracle rows.

Runtime-route refinement (which `if` / `//` / `first` branch actually ran) is out of scope:
it could only turn refusals into answers, and belongs with oracle rows of its own.

### D5. Drain producers default to `LostAt`

See section 5 for the captures. The default changes nothing on `main`, since C1 already drops
these today (`cannot_move_register` is `false` for them).

### D6. Wrappers pass the state through, in one helper

`?` (`wrap_optional_branch`), `try` without `catch`, `first` / `Keep::First`, and `//` move a
`BranchRegister` through unchanged and never rebuild with `None` (round-2 category 4). `//`'s
right operand runs after backtracking to the fork, so it sees the **entry** state, not the
left operand's. `wrap_optional_branch` currently takes the field by destructure and puts it
back; the helper exists so a future wrapper cannot do otherwise.

### D7. `FoldRegister`

SOURCE can move the register: `foreach .[] as $k (.; getpath(["a"]); .b)` (#2896). The
fold's register at UPDATE must come from SOURCE's branch state, not from the fold's entry.
Rule: a SOURCE branch that is not `Unmoved` makes UPDATE's frame lost
(`RegisterLoss::LostAt` / `LostSomewhere`). `FoldRegister::enter` currently reads INIT's
trackability and `self.value` only. The pinned row
`test_path_register_fold_source_that_navigates_moves_the_register_3456` already covers it.

### D8. The default for lost is refuse, uncatchably

A refusal on a lost branch goes through `guess_refusal`, so `try` / `?` / `?//` cannot catch
it and continue to a write. For `?//`, `mark_nonretryable_escape` is the existing mechanism
(#2649, #3293); B2 confirms by test that it covers a guessed refusal, rather than assuming it.

## 4. Consumer migration

| Consumer                         | On `Unmoved(r)`                                         | On `LostAt` / `LostSomewhere`                                                       |
|----------------------------------|---------------------------------------------------------|-------------------------------------------------------------------------------------|
| C1 `reestablishes_register`      | `register_identical(r, ...)`                            | never re-establishes                                                                |
| C1 `carry_register`              | carry `r`                                               | carry nothing                                                                       |
| C2 loss derivation               | `Kept`                                                  | `Frame::register_loss = Lost*` (the single conversion point)                        |
| C3 frame readers                 | seed with `r`                                           | seed with nothing, under a lost frame (refusal is loud)                             |
| C4 `FoldRegister`                | as today                                                | UPDATE frame lost                                                                   |
| C5 `guess_refusal`               | unchanged                                               | unchanged; it sees more lost frames, never fewer                                    |
| C7 `register_after` (B2)         | read the branch                                         | `computed_at_register` gets `None`; the `And`/`Or` arm's `R` runs under a lost seed |
| C7 `and_or_negate_resolves_live` | B2: keep the gate; B3: drop `register_movement_tracked` | `R` resolves from a lost seed and refuses loudly                                    |
| C8 `certifies_value`             | unchanged                                               | unchanged                                                                           |

## 5. Oracle captures for D5 (jq 1.7.1, `/usr/bin/jq`)

`{"a":{"b":1},"k":2}`, `path(. as $x | P | $x)`. `[]` means `$x` (frozen from the root) is
still identical to jq's register, so `P` left the register at the root; "refuses" means `P`
moved it. All captured live on this machine's `jq-1.7.1-apple`.

| `P`                                                                          | jq result                          | Register after `P`    | `cannot_move_register`    | Default in B2                                          |
|------------------------------------------------------------------------------|------------------------------------|-----------------------|---------------------------|--------------------------------------------------------|
| `last(.a)`                                                                   | `[]`                               | unmoved               | false                     | `LostAt`                                               |
| `last(.a,.k)`                                                                | `[]`                               | unmoved               | false                     | `LostAt`                                               |
| `isempty(empty)`                                                             | `[]`                               | unmoved               | false                     | `LostAt`                                               |
| `isempty(.a)`                                                                | refuses, "with result" the root    | **moved**             | false                     | `LostAt`                                               |
| `limit(1; .a)`                                                               | refuses, "with result" the root    | moved                 | false                     | `LostAt`                                               |
| `nth(0; .a)`                                                                 | refuses, "with result" the root    | moved                 | false                     | `LostAt`                                               |
| `first(.a)`                                                                  | refuses, "with result" the root    | moved                 | false                     | `LostAt`                                               |
| `any(.a; true)`                                                              | refuses, "with result" the root    | moved                 | false                     | `LostAt`                                               |
| `[.a]`                                                                       | `[]`                               | unmoved               | false (contents navigate) | `Unmoved` via the `[E]` rule when contents are checked |
| `[.a] \| first`                                                              | refuses, near element 0 of `[...]` | moved                 | false                     | `LostAt`                                               |
| `range(2)`                                                                   | `[] []`                            | unmoved               | false                     | `LostAt`                                               |
| `paths`                                                                      | `[] [] []`                         | unmoved               | false                     | `LostAt`                                               |
| `select(true)`                                                               | `[]`                               | unmoved               | (native)                  | n/a                                                    |
| `tostring`                                                                   | `[]`                               | unmoved               | true                      | `Unmoved`                                              |
| `try .a`, `.a?`, `(.a // 1)`, `if true then .a else 1 end`, `getpath(["a"])` | refuse, "with result" the root     | moved (they navigate) | false                     | `LostAt`                                               |

Consequences:

- `last(f)` (defined as `reduce f as $x (null; $x)`) keeps the register even when `f` navigates,
  because its source backtracks. It is the one drain producer where promotion to `Unmoved` is
  oracle-backed. It is **not** promoted in B2: B2 is "no behaviour change", and a promotion
  turns a refusal into an answer, which belongs in its own reviewed step with sweep rows.
- `isempty(g)` is per-branch: it moves the register only when `g` emits (it breaks out of a
  `label` before backtracking). That is D4.4 and is the reason a static `Unmoved` cannot be
  used for it.
- `range(n)` and `paths` are not navigation, yet `cannot_move_register` says `false` for both
  (a builtin allowlist, deliberately; "add a variant only with an oracle row"). Both would be
  safe `Unmoved` promotions and are listed here so a later step can do them with rows.

## 6. Delivery

Each step is safe to land on `main` by itself.

- **A1 (merged, PR #3496):** `scripts/jq-path-register-sweep.py` plus one pinned
  `*_3456` row family per round-2 category in `tests/jq_cli_tests.rs`.
- **#3494 (merged, PR #3509):** `Frame::certifies_value`.
- **A2 (this note).**
- **B1 -- type introduction, zero behaviour change (#3456).** `BranchRegister { None,
  Unmoved }` replaces the `Option`; only those two variants, because the lost states have no
  producer until B2 and an unconstructed variant would need a `dead_code` allowance
  (STYLE-0005). P1, P7, P8 and the stage placement map `Some` to `Unmoved` through
  `BranchRegister::from_option` (still the entry value, deliberately not yet truthful), so
  `git grep from_option` is B2's list of producers that must state their own answer, after
  which the adapter is deleted. The D2 assertion landed first and is narrower than the plan
  assumed (section 3, D2).
- **B2 -- move the rule (D3-D7).** Producers become truthful, `stage_preserves_register`
  leaves C1, wrappers and `FoldRegister` consume the state, `register_after` collapses to
  "read the branch". `and_or_negate_resolves_live` keeps its gate, so **no behaviour change on
  `main`**; a flipped row means the lift was not the same predicate, which is a bug in the
  lift: stop and diff.
- **B3 -- close #3428.** With `LostAt(entry)` truthful on `first` / `last` / `any` / `nth` /
  `range` / `paths` / `try` / `//` / `if` / `def` operands, drop the
  `register_movement_tracked` precondition from `and_or_negate_resolves_live`. Expected: the
  accept-where-jq-refuses rows flip to refuse. **Gate:** the sweep must show no row where jq
  accepts and B3 newly refuses (`del(first and .[0])` is the one the old gate existed to
  avoid). If it does, B3 stops and reports rather than widening the gate again.

## 7. Acceptance for B1 and B2

- A1 sweep (`scripts/jq-path-register-sweep.py`, 94,887 rows, release builds): 0 new
  accepts / writes and 0 new jq divergences against `main`, on B1 and on B2. The baseline to
  hold is 90771 MATCH / 307 ACCEPT_WRONG / 3795 REFUSE_WRONG / 14 DIFF (`main` after #3509).
- The pinned `*_3456` rows stay green unchanged.
- #3494's rows stay green: `test_and_or_path_empty_array_slice_is_not_the_register_3494` (fix
  rows, guard rows and positive controls) and `certifies_value_needs_a_position_for_an_empty_array_3494`.
  The empty-array table from #3494 is the contract for any producer-side replacement:

  | input | filter                                    | jq 1.7.1                   |
  |-------|-------------------------------------------|----------------------------|
  | `[]`  | `path(. as $x \| (.[0:] and true) \| $x)` | exit 5, `with result []`   |
  | `[]`  | `path(. as $x \| .[0:] \| $x)`            | exit 5, `with result []`   |
  | `[]`  | `del(. as $x \| (true and true) \| $x)`   | `null`, exit 0             |
  | `[1]` | `path(. as $x \| (.[0:] and true) \| $x)` | `[{"start":0,"end":null}]` |

  The position-less case (an empty array certified by value through a frame with `at ==
  None`) is covered only by the sweep today. B2's producer-side state should make it true by
  construction; if it cannot, say so in the PR rather than leaning on the sweep.
- Existing register tests unchanged and green: the `_1573` / `_2041` / `_2044` / `_2046` /
  `_2649` / `_2896` / `_3120` / `_3133` / `_3263` / `_3267` / `_2760` / `_3289` / `_3494`
  families. Every admission stays jq-only (ADR-0018).
- `cargo test --features cli,simd,regex,serde` and `--no-default-features`; clippy variants
  from `scripts/lib/clippy-variants.sh`; fmt; doc. Perf guard clean or sized.
- `docs/compliance/jq/limitations.md` (the `and` / `or` / unary minus entry): point at this
  note after B2, and drop the by-value-operand residual after B3. The `?//` and
  pointer-identity residuals stay unless B2/B3 close them, and then only with oracle rows.

## 8. Risks

- **A missed discard becomes catchable.** D2 keeps loss on the `Frame`; B2 greps every
  `PathBranch { ... }` literal and constructor for a `Lost*` that could be dropped.
- **B2 flips rows.** See section 6. Any flip needs its own oracle capture and a line in the PR.
- **"No behaviour change" can hide a refactor that never reaches the changed code.** The
  sweep needs rows that reach P1, P7 and P8 on a trackable input under each wrapper; B1/B2 check
  that the sweep's grid does, rather than assuming it.
- **The D2 invariant was false as first stated** (section 3, D2); B1 narrowed it to reader frames and
  it holds in the suite and sweep. B2 should not read a register from the handoff frame.
- **B3 widens acceptance.** Its gate is the sweep, not reasoning.

## 9. Open questions for review

1. Should `last(f)` be promoted to `Unmoved` in its own step after B2 (section 5)? Recommended:
   yes, with sweep rows, after B3.
2. Is `BranchRegister` worth carrying `Cow` for the `Lost*` variants, or is `LostAt` enough with
   an `Rc`? `Frame` already uses `Rc`. Decide with the `size_of` measurement in B1.
3. The `Try` negative test in D4.3: where does a lost-frame refusal under `try` get caught
   today, if at all? B2 writes the test first; if it fails on `main`, that is a separate
   Severity-High issue, not folded into this one.
