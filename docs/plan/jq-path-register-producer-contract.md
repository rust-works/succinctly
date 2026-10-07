# The producer contract for jq's path register (#3456)

**Status: B1, B2, B2b and B3 landed ([#3456](https://github.com/rust-works/succinctly/issues/3456),
[#3428](https://github.com/rust-works/succinctly/issues/3428)); the promotions in section 10 are
open.** This note began as the design review gate before B1 (the type change). It re-confirms the
inventory on `main` at `ea2fcd19c`, settles the questions the issue left open, and lists what
B1/B2/B3 had to keep green; sections 3, 6 and 10 record what was built. Functions are cited by
name, not line: `src/jq/eval.rs` moves too fast for line numbers to survive a review round. The
doc comments on `PathBranch`, `Frame`, `cannot_move_register`, `register_after` and
`reestablishes_register` are the primary source; this note is the connective tissue between
them.

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
**false as stated.** B1 first put a probe `debug_assert!` on every `Frame` constructor that can
combine the two (`with_register`, `with_register_loss`, `extend`) and ran the jq suite: eight
tests tripped it (`test_path_catch_handler_*_843`,
`_3133`, `_2978`, `_1297`). The violating frames are the *handoff* frame `place_step` builds
with `with_register_loss` for the next stage, and that frame after `extend`. Both still carry
the register the stage was *entered* with while recording a new loss. The next stage replaces
the register through `with_register` (its `stage_frame`), and neither the suite nor the sweep
showed a reader of a mixed frame -- tested, not proven by construction. The `And`/`Or` arm's
`frame.with_register(register)` was the candidate counterexample; it did not fire, in the suite,
in the sweep, or on hand-built `reduce`/`foreach`/`try` shapes under an and/or/negate operand.

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

**As built in B2 -- two parts of the above could not be exact, and were not done.**

- *The stage rule stays, once.* `stage_preserves_register` is a rule about the whole pipe
  **stage**, and a leaf producer sees only its leaf. For a compound stage (`(.a // 1)`, an `if`
  or a `try`/`catch` mixing a navigating and a by-value part) the stage verdict is stricter than
  any leaf's, and a leaf-local verdict is the truer one: jq backtracks to the fork, so the
  by-value leaf's register is unmoved. Captured against jq 1.7.1, `path(. as $x | (.a // 1) | $x)`
  on `{"a":null}`, `path(. as $x | if true then 1 else first(.a) end | $x)` and `path(. as $x |
  try 1 catch first(.a) | $x)` on `{"a":1}` are all `[]`, where `main` refuses. Lifting the
  verdict to the leaf would therefore turn each refusal into jq's answer: correct, but a
  promotion that needs its own rows, not part of making the producers truthful. So `place_step`
  applies `cannot_move_register(element)` to the register entering the step, in exactly one
  place, and takes the stricter of the producer's and the stage's answer
  (`test_path_register_compound_stage_is_refused_as_a_whole_3456` pins the rows).
  `StepRegisterFacts::stage_preserves_register` and `carry_register` are gone: `reestablishes_register`
  reads a register the stage has already vouched for.
- *A carried register has no leaf producer.* `getpath_preserves_register` is `false` whenever the
  entry is trackable, so the per-branch getpath rule only ever concerns a register carried on an
  untracked entry, which `untracked_at_register` never sees (it is called with `trackable ==
  false` and records nothing). In yq mode the frame carries no register at all, yet the carry
  still applies. The stage rule is that carry's transfer function, applied by `place_step`.

The producers themselves are as D3 says. `leaf_register(expr, trackable, value)` is `None` /
`Unmoved(entry)` (`cannot_move_register(expr)`) / `LostAt(entry)`, read once per leaf call and
only while trackable. The drain arms (`last`, `isempty`, `INDEX`) are `LostAt(entry)` (D5), except
`last(f)` in jq mode, which #3643 promoted to `Unmoved(entry)` (section 5). An
unchecked `[E]` on a trackable entry is `LostSomewhere`, **not** `LostAt`: that is what `main`
derives today (`untracked_at_register` was handed `trackable && checked`, so it recorded nothing),
and `LostAt(entry)` would let `guess_refusal` clear a refusal of a `$x` frozen elsewhere and let a
`try` catch it -- a promotion (rows pinned in
`test_path_register_drain_producers_lose_the_register_uncatchably_3456`).

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

**As built (B2b): no helper.** Reading the arms showed every wrapper and compound node (`,`,
`//`, `if`, `try`, `?`, `label`, a `def` call, `Paren`, `Shared`) already forwards each
sub-branch's register unchanged, and those states are already truthful leaf-locally: jq
backtracks to the fork, so each leaf's register is its own. What was conservative was the
*consumers* (`register_after`'s allowlist, `place_step`'s stage rule). A node-level verdict at
the `resolve_node_sink` boundary would have re-implemented the stage rule at every compound
node, cost a sink wrapper per node, and still left `register_after` unable to collapse exactly
(a checked `[E]` is `Unmoved` for `place_step` and was lost for `register_after`). So B2b made
`register_after` read the branch instead (section 10).

### D7. `FoldRegister`

SOURCE can move the register: `foreach .[] as $k (.; getpath(["a"]); .b)` (#2896). The
fold's register at UPDATE must come from SOURCE's branch state, not from the fold's entry.
Rule: a SOURCE branch that is not `Unmoved` makes UPDATE's frame lost
(`RegisterLoss::LostAt` / `LostSomewhere`). `FoldRegister::enter` currently reads INIT's
trackability and `self.value` only. The pinned row
`test_path_register_fold_source_that_navigates_moves_the_register_3456` already covers it.

**Retired (B2b).** `FoldRegister::enter`, `resolve` and `relocate` never read
`PathBranch::register`, so there is no producer-side state for SOURCE to hand over and nothing
the refactor could flip; the pinned row passes on its own account (#2896's `getpath` mark). It
comes back only if a later change makes SOURCE's branch matter.

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
  **Promoted by #3643** (jq mode only), at two sites: the `LastExpr`/`LastStream` arm states
  `Unmoved(entry)` for the leaf (what an `and`/`or` operand reads), and
  `stage_leaves_register_in_place` admits the stage in `resolve_seq_stage`'s
  `stage_preserves_register` (what a pipe's next stage reads). It is not in
  `cannot_move_register`, whose "navigates nothing" reading `f` breaks. Pinned by
  `test_path_register_last_f_does_not_move_it_3643`; the sweep grid gained `last(f)` operands and
  the `first(f)`/`nth`/`limit` contrasts.
- `isempty(g)` is per-branch: it moves the register only when `g` emits (it breaks out of a
  `label` before backtracking). That is D4.4 and is the reason a static `Unmoved` cannot be
  used for it. Its arm states the register of the first branch `g` emitted (#3763), and the
  entry register when `g` emitted nothing (#3456, `drained_register_after`).
- `range(n)` and `paths` are not navigation, yet `cannot_move_register` said `false` for both
  (a builtin allowlist, deliberately; "add a variant only with an oracle row"). **Promoted in B3**
  with the rows in `test_path_register_range_and_paths_do_not_move_it_3456`. The two-argument
  `range(a; b)` is C-coded with closure arguments run unwrapped, so a navigating argument moves
  the register (`range(0; .a)` refuses, `range(.a)` and the three-argument form do not); the
  parser spells `range(n)` as `range(0; n)`, so `range(.a)` keeps the conservative answer.
- `any`, `all` and `isempty(g)` are D4.4's "did it" case and **are decided per value in B3**:
  jq 1.7.1 defines `isempty(g)` as `first((g | false), true)`, `any(g; c)` as
  `isempty(g | (c or empty)) | not` and `all(g; c)` as `isempty(g | (c and empty))` (read off
  `--debug-dump-disasm`), so each moves the register exactly when the generator inside emitted.
  B3 first decided that per value from the result (`register_stays_on_result`); that verdict is
  gone. Every spelling now has its own arm (`resolve_any_all_gen_cond_sink`, #3749 and #3763;
  the `isempty` arm), which states the register the *deciding branch* left, or the entry
  register when nothing decided. Both claims are sound only if `cond` cannot raise a path
  error jq would raise: jq path-checks `cond` (it is `or`'s left operand, not a subexp), and
  an arm that runs it by value cannot see that. So a `cond` that provably navigates nothing
  (`cannot_move_register`) runs by value, and any other is resolved as a pipe stage on each
  branch `gen` produced (`resolve_seq_stage`, #3757), which is `gen | cond`: its register and
  its path errors are the resolver's own, so the decided claim is the output branch's
  register (`register_after`) and the undecided one needs no extra condition, because a
  `cond` that could raise already did. An *untracked* generator element is the one thing
  the stage cannot be handed blindly: `jv_identical` makes a pass-through value
  (`ltrimstr("x")` on a non-string) or an equal `null`/`true`/`false` the register itself, and
  `cond` then raises no path error. The stage gets it only when the producer vouches
  (`Unmoved(register)`, or the frame's carried register when `gen` moves nothing): equal by
  kind, re-established at the branch's path; unequal, it raises as jq does; otherwise it is an
  uncatchable guess, which also keeps a lost-register state out of the stage's entry assertion.
  The per-result verdict made the claim for any `cond` and wrote `null` over a document for
  `all(unique_by(.))`; #3749's undecided claim did the same for `any(1; .[]?) or 1`.
  #3758: the pipe stage reads the arm's statement (`stage_states_register_per_result`, jq mode only):
  per result, a result that navigated nothing and whose leaf says `Unmoved` came from a generator
  that backtracked every branch, so `. as $x | any | $x` is the register again. A decided result
  navigated (`.[]`'s own index) and was already read through `reports_register`. The stage rule
  only *admits reading* the leaf's statement; the claim itself stays the arm's.
  #3826: on an untracked entry the leaf states nothing (the register is carried by the stage), and the
  carried copy stands unless the step states a loss.
  #3859: a destructuring `?//` bind whose body satisfies `cannot_move_register` is read the same way.
  A successful destructure takes tracked index steps, so its result navigated and is read as before;
  a result that navigated nothing came from a bare `$var` alternative reached after a failed
  destructure (restored by the fork), whose leaf states `Unmoved(entry)`. A body mixing a navigating
  and a by-value part is not admitted (#3899), for the reason D1 gives for `(.a // 1)`.

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
- **B2 -- producers state their own register (D3, D5; landed).** `BranchRegister` gains
  `LostAt(Rc<OwnedValue>)` and `LostSomewhere`; `from_option` is deleted; P1, P2, P7, P8 and the
  `resolve_seq_sink` seed state their own answer; `place_step` reads the branch, applies the
  stage rule once (section 3, "As built"), and derives `Frame::register_loss` from the lost state
  instead of re-cloning the entry register. **No behaviour change on `main`**; a flipped row means
  the lift was not the same predicate, which is a bug in the lift: stop and diff. Not done in B2,
  and why: `register_after` cannot collapse to "read the branch" while the compound/wrapper
  producers (`if`, `try`, `//`, `def`, `first` ... under an untracked ambient) are not truthful
  (it would accept where it answers `None` today); D6's pass-through and D7's `FoldRegister`
  read of SOURCE's branch both need that too. They are **B2b**, ahead of B3.
- **B2b -- `register_after` reads the branch (landed).** Not the node-level verdict this section
  first planned (D6, above); see section 10. `register_after` takes a trackable branch's value, a
  stated `Unmoved` as given, the stage's carried register for an operand that cannot move it, and
  loses everything else. `resolve_from_restored_input` no longer takes a lost register at the root
  for an `Unmoved` one, and marks a lost seed on a trackable entry so its refusals are classed as
  guesses. No behaviour change on `main`: the sweep is identical to `main` over 310,095 rows.
- **B3 -- close #3428 (landed).** `and_or_negate_resolves_live` drops its `trackable` argument and
  the `register_movement_tracked` precondition. The promotions it needed (section 10) went in the
  commit before it. **Gate** (the rev. 4 text kept for the record): no row where jq accepts and B3
  newly refuses, no new accept. It was met for the accepts and not for the refusals, which section
  10 accounts for.

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

1. ~~Should `last(f)` be promoted to `Unmoved` in its own step after B2 (section 5)?~~
   **Answered: yes**, with sweep rows, after B3 (#3643).
2. ~~Is `BranchRegister` worth carrying `Cow` for the `Lost*` variants, or is `LostAt` enough
   with an `Rc`?~~ **Answered by B1's measurement** (aarch64 macOS, release layout, throwaway
   test, not committed): `size_of::<PathBranch>()` is **112 bytes before and after B1**
   (`Option<Cow<OwnedValue>>` and `BranchRegister { None, Unmoved(Cow) }` both fit the `Cow`
   niche). A mirror struct with the same fields measured **120** with
   `{ None, Unmoved(Cow), LostAt(Cow), LostSomewhere }` and **112** with
   `{ None, Unmoved(Cow), LostAt(Rc<OwnedValue>), LostSomewhere }`. B2 should use
   `LostAt(Rc<OwnedValue>)`: it costs nothing in `PathBranch`'s size and matches
   `RegisterLoss::LostAt`, so the branch-to-frame conversion in `place_step` moves the `Rc`
   instead of cloning. (`Frame` is 40 bytes.) **Confirmed in B2 on the real type:**
   `path_branch_size_is_pinned_3456` still reads 112 bytes with all four variants.
3. The `Try` negative test in D4.3: where does a lost-frame refusal under `try` get caught
   today, if at all? B2 writes the test first; if it fails on `main`, that is a separate
   Severity-High issue, not folded into this one.

## 10. What B2b and B3 found

Revision 4 of the plan had B2b as a node-level verdict and B3 as one line. Trying B3 first, on
`main` with nothing else changed (the gate dropped, a release build, the sweep over the 94,887-row
grid of the time), showed what it needs. Revision 5 of #3456 has the whole record; the findings,
in the order they appeared:

1. **`resolve_from_restored_input` trusted depth 0 over a lost register.** A by-value operand jq
   navigates inside is emitted at the root *and* lost; the arm resolved `R` live as though `L` had
   not run, and `del(any and .a)` on `{"a":true}` returned `{}` where jq exits 5. 72 rows. The
   shortcut now needs an `Unmoved` register on a trackable entry.
2. **A lost seed's refusals were classed exact, so `try`/`?` swallowed them.**
   `could_be_lost_register` counts only a frozen snapshot or `null`, on the premise that a computed
   value is never jq's register. `R`'s input is not computed: it is the node the register entered
   on, which jq still holds if `L` did not move it. `del(try (any or .b?))` printed the document
   where jq exits 5. The seed is marked (`Snapshot::register_entry`, positional provenance that
   never certifies), and the mark is read where a step is pruned instead of refused: a
   `?`-wrapped first step in `resolve_static_tail` (`(.a)?`, `(.a | .b)?`) and a bare optional
   primitive in `resolve_optional_sink` (`.a?` is jq's `INDEX_OPT`, which suppresses a type error
   but not a path error), both found by review, not by the sweep. Without the mark, 24 rows over the boundary operands `sort` and `to_entries`
   turn into silent no-ops or echoed documents (measured with a throwaway mutant).
3. **A static `LostAt` is too coarse for operands jq leaves in place.** `range`/`paths`/`has` are
   static `Unmoved`; `add` and `map(f)` are `Unmoved` for the by-value leaf only
   (`leaves_register_in_place`), because `cannot_move_register` also means "checks nothing" and an
   `R` of `add` checks its `.[]` against a register it is not on (a first cut that put them in
   `cannot_move_register` accepted 49 rows jq refuses); `any`/`all`/`isempty` decide per value
   (section 5).
4. **The compound producers were already truthful** (D6, above); the conservative parts were the
   consumers.

The grid grew from 31 to 54 operands for this: `has`, `keys`, `add`, `map(.)`, `sort`,
`to_entries`, `flatten` and four compound stages with a by-value leaf, because a by-value
operand that loses its eager route and cannot say "unmoved" is a lost match and the old grid
could not see it; then, once `register_after` read an operand's stated `Unmoved` as given,
twelve producers a wrong `Unmoved` would hurt most (`reduce`, `foreach`, `label`, `limit`,
`first(f)`, `nth`, `recurse`, a comma, a `try` with a handler, two pipes that navigate and then
compute, a `//`). A review of the branch then found a hole the grid could not reach, a lost
`L` followed by a `?`-wrapped right operand, so it also grew three right operands (`(.a)?`,
`(.a | .b)?`, `(.. | .a?)`) and an input with an object below the root (`[{"a":1}]`), without
which "no new accepts" had been true and meaningless. Final state, `main` against the branch,
310,095 rows, jq 1.7.1 as the oracle:

| Build                         |  MATCH | ACCEPT_WRONG | REFUSE_WRONG | DIFF |
|-------------------------------|-------:|-------------:|-------------:|-----:|
| `main`                        | 293013 |          883 |        16102 |   97 |
| B2b only (gate in place)      | 293013 |          883 |        16102 |   97 |
| B3                            | 304142 |           36 |         5915 |    2 |

B3 moves 11,907 rows to a match and loses 778 that matched: 467 over `sort`, `to_entries` and
`flatten` (builtins jq leaves in place that the allowlist does not name, so `R`'s navigation is
refused where jq accepts); 182 over `any`/`all`/`isempty`, where jq catches its own error under
a `try` or a `?` and this resolver's guess is uncatchable; 91 over a `foreach` operand, whose
fold branches state no register, so the operand is read as a loss (a fold is where D7 would
have changed that); and 38 over a by-value or compound right operand after a navigating left
one on a `null` input (`del(. as $x | (.b? or isempty(.[]?)) | $x)` on `null`), where the
earlier match was luck: the eager route answered the root path where jq answers `["b"]`, `del`
of either leaves the same document, and #3579's rule now refuses a terminal `null` after a
navigation (#3713 closed the same hole for `walk(f)` over a scalar, which jq runs as `f` on the
register: it now resolves as `f`, and `walk(f)` over a container refuses whenever jq does, even when
`f` yields nothing). All are refuse-only. No row is newly accepted wrongly, and none newly differs;
the 36 that still accept are `.[0:] and R` and `.[0:0] and R` on `[]`, #3494's empty-slice rule, the
same rows `main` accepts (closed afterwards by #3647: the restored seed took `[]` for the input, see
`restored_register_is_input`), and the 2 `DIFF` rows are `main`'s too (jq prints a path before it
errors). The promotions commit on its own, measured on the 162,207-row
grid of the time, moved 498 rows to a match and none the other way.

Reach, each a throwaway mutant against the final build over the operands it touches (measured
when the grid had 41 operands): the depth-0
guard removed flips 164 rows (136 worse); the seed mark removed, 60 on the grid as it was (all in the
safe direction, so the grid could not see the mark) and 40 (24 worse) once `sort` and
`to_entries` were in it; the per-result verdict ignored, 117 (103 worse);
`register_after` ignoring a stated `Unmoved`, 1,145; `add` and `map` out of
`leaves_register_in_place`, 265.

### Open promotions

Each turns a refusal into an answer and needs its own oracle rows:

- ~~`last(f)` to `Unmoved` (section 5).~~ Done by #3643.
- ~~A `select(f)`/type-filter stage, and the wrappers around it and around `last(f)` that add no
  movement (`?`, `try`, `first(...)`).~~ Done by #3653, as two stage-level rules beside
  `cannot_move_register` in `resolve_seq_stage` (`stage_leaves_register_in_place`, which asks
  `is_last_stage` and `is_select_stage` of what `peel_register_transparent` leaves): jq
  defines `select(f)` as `if f then . else empty end`, so `f` is a subexp, and `try` and `first`
  pass the inner register on. A `select` entered on the register already passed a trackable
  branch through; only the register *carried* by an untracked entry was dropped, so the sweep
  gained contexts that enter the stage on a literal (`untracked-*`). Still refused where jq
  answers, tracked by #3767: `try ... catch H` with a navigating `H` (the rest is #3767 Part 3, below) and either stage inside a compound stage
  (`limit`/`nth` and an `[E]` collect of a type filter were lifted by #3767 Part 1, below). Not a
  promotion but found on the way,
  #3766 (fixed): `last(f)` returned a copy, so a `last` whose output is the register itself
  (`last($x)`, `last(.)`) lost its identity; the arm now forwards the last branch itself when
  it is the entry node, and keeps an untracked output's snapshot mark, so it states the
  identity of the value `f` last emitted.
- ~~`limit(n; E)` and `nth(n; E)` around a register-keeping stage, and an `[E]` collect of a type
  filter.~~ Done by #3767 Part 1. Both emit from inside `E` like `first(E)`, so
  `peel_register_transparent` reads them through (`Expr::Limit`, `Builtin::NthStream`: the
  spellings the parser builds) and the inner stage decides: `last(f)`, `select(f)` and the type filters
  leave the register where it entered, `.a` still moves it. The count is a subexp. And
  `array_contents_are_checked` asks `is_select_stage` (select or a type filter), where it named
  only `select`. Done by #3767 Part 3: `try E catch H` is `E`'s verdict when `H` cannot move the register
  (`stage_is_register_keeping`: jq runs the handler after a backtrack that restores the register, on the
  error's payload, which has no position); a handler that navigates stays refused. Still open under #3767: a compound inner stage (`,` `//`
  `if`, a pipe, a `def` call), an `[E]` of a wrapper around a type filter
  (`[first(numbers)]`; `[last(f)]` is Done by #3767 Part 2: `array_contents_are_checked` reads through
  `last(f)` to `f`), and a wrapper over an inner stage that navigates nothing (`first(.)`,
  `limit(1; .)`, `limit(1; 5)`), which jq leaves in place and the allowlist does not read through.
- ~~A `reduce` whose source or `UPDATE` navigates.~~ Done by #3732 (`reduce_leaves_register_in_place`, read by
  `resolve_reduce`'s emission and by `leaves_register_in_place`): jq's `reduce` is `INIT; FORK loop; SOURCE; UPDATE;
  BACKTRACK`, so the register is where INIT left it, and an INIT that cannot move it leaves it at the entry whatever
  the loop navigates. A navigating INIT, a destructuring pattern and `foreach` (which emits from inside the loop) stay
  refused.
- ~~A recursion's seed behind a call or a fork, the output a `catch` handler adds after it, and a caught recursion in a
  fold body (#3272's residuals).~~ Done by #3580, as a *per-output* statement and not a stage verdict: a recursion's seed
  (`recurse_family_root_seed`) and an untracked `.` state `BranchRegister::AtEntry` (no value: on an untracked entry the register
  is the stage's carried one), and a `catch` handler's output already states `Unmoved(register)` (#3133). `place_step` reads either
  per output (`states_register_at_entry`) through `entry_marker_shape`'s allowlist of wrappers whose resolver arm forwards a branch
  unchanged (`Paren`, a closure parameter, `?`, `try`, `,`, `if`, `//`, `first(f)`, `limit(n; f)`, `label`; `nth(n; f)` and a bare-variable bind's body since #3892); a producer anywhere else makes the
  stage opaque. The two fold rows are the same idea at the fold's own register: a `reduce` admits a `try` around a tracked UPDATE
  (`fold_update_movement_tracked`), and `foreach` states `Unmoved(entry)` per emission whose UPDATE output was at the entry
  (`FoldRegister::resolve_sink` keeps the statement, `advance` carries `self` on it, `foreach_states_register_per_emission` admits the
  stage on a trackable entry only: an untracked one reads an absent statement as "the carried copy stands", #3826). Still
  refused where jq answers, pinned by `test_recurse_seed_residuals_stay_loud_3580`: a literal ahead of the recursion (`(1, ..)`,
  the taken `else 1`), a recursion behind a destructuring bind or a `def` call, a pipe nested in a forwarder, a forking
  `foreach` UPDATE, and a fold inside an `[E]`. (`nth(n; ..)`, a bare-variable bind and a `reduce` UPDATE of `recurse(f)` answer
  since #3892.) A design note for whoever extends it: **do not convert the handler's `Unmoved` to
  `AtEntry`** -- the first cut did, and 44 `-(try (.a | error) catch 7)` rows lost a match, because `register_after`, an `any`/`all`
  generator and a fold source all read the value out of an `Unmoved`.
- The stage-level downgrade in `place_step`: a leaf-local verdict for `,`/`//`/`if`/`try`, which
  turns the three rows pinned by `test_path_register_compound_stage_is_refused_as_a_whole_3456`
  into jq's `[]`. Cheap now, because the producers already say it.
- `LostAt(entry)` for an unchecked `[E]`, and passing `LostAt`'s position through `register_after`
  instead of `LostSomewhere`.
- ~~An `[E]` collect of `map(f)`/`any(f)`/`all(f)` whose `f` navigates but stays on the register.~~ Done by #3724
  (`navigates_only_the_register`: a chain of register navigations, optionally ending in one stage that navigates
  nothing, forked branch by branch). Still refused where jq answers: a `//`, `try` or `first(f)` inside `f`, and
  the `and`/`or` operands below.
- ~~Widening the `[E]` claim over `and`/`or`/unary minus past `register_movement_tracked`.~~ Done by #3724 (item 2):
  an operand is claimed when `register_movement_tracked` *or* `array_contents_are_checked` says so
  (`and_or_operand_is_checked`), so a bare `first`/`last`/`add`, a `map(f)` and `last(f)` no longer refuse the
  collect: `[first and .[0]]` raises near element 0 on `[true]` and answers `[]` on `[false,1]`, as in jq. The union,
  not the replacement, because `register_movement_tracked` also covers an `as` source, which
  `array_contents_are_checked` does not. Swept over 300 `[L and R]`/`[L or R]`/`[-L]` operands built from twelve
  pieces: 266,427 rows, 0 regressions and no `ACCEPT_WRONG`, 1,424 refuse-to-match flips, 285 rows still refused.
- ~~Static `Unmoved` for the other builtins jq leaves in place (`sort`, `to_entries`, ...), one oracle
  row each.~~ Done for `sort`, `to_entries`, `flatten`, `add`, `map(f)` and `walk(f)` by #3361, at the
  leaf (`leaves_register_in_place`) and as a stage (`resolve_seq_stage`'s `stage_preserves_register`).
  #3711 did `reverse`, `min`, `max`, `min_by(f)`, `max_by(f)`, `group_by(f)`, `sort_by(f)`,
  `flatten(n)` and `join(s)`, and refused `flatten(n)`/`join(s)` on an untracked input once the
  call has produced a value (`iterates_untracked_input`; not `builtin_navigation`, #2646). Still
  refused, each needing its own row: a `map(f)`/`walk(f)` whose `f` navigates, and the rest of
  the builtins jq leaves in place (`ltrimstr`/`rtrimstr`, ...).
- A register-position-aware `try` for the guess a lost `and`/`or` operand raises, so a refusal jq
  catches can be caught here too (the `var-rebind-nav` rows). **Not built as such, and not needed
  for the operands #3645 named**: #3749 and #3763 made `any`/`all`/`isempty(g)` state the register
  they left, so the refusal of `R` is exact and a `try` catches it as jq's does, and #3361 made
  `flatten`/`sort`/`to_entries` leave it where it entered. Pinned by
  `test_try_around_and_or_by_value_operand_answers_as_jq_3645`. Re-measured on `main` at
  `4b1e9e86a` (a debug build against jq 1.7.1):

  ```console
  $ ./scripts/jq-path-register-sweep.py --candidate target/debug/succinctly --jobs 8 \
      --operand any --operand all --operand 'isempty(.[]?)' --operand 'any(.[]?; .)' \
      --operand 'all(.[]?; .)' --operand flatten --operand sort
  ```

  93,267 rows: 0 `ACCEPT_WRONG`, 0 `DIFF`, 629 `REFUSE_WRONG`, none in a `try-del`,
  `try-catch`, `try-catch-update`, `optional`, `first-wrap` or `alt-wrap` context (558 are the
  `?//` retry contexts, 34 `var-rebind-nav`, 28 `foreach-update-try-var`, 9 `foreach-source`). What
  still refuses where jq answers is not a lost `and`/`or` operand: a fold operand
  (`reduce`, `foreach`) states no register (D7); `try (X) | try ($y | .b)` refuses whenever `X` merely *contains* a navigation,
  even one that never runs (`try (true or .a)`), which is the stage-level downgrade listed above; a
  `?//` retry's guess (#3293); and the `null`-input rows, which look like the terminal-`null`
  bucket of #3579 but were not attributed individually. None of these is tracked by this item.
