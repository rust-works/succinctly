# Decode-failure error routing for `to_owned`/`cursor_to_owned` (#1247, #1242, #1194)

**Status: implemented — Stages 1–6 all landed.** This
document is the deliverable for
[#1247](https://github.com/rust-works/succinctly/issues/1247), which was tiered Tier 3
("needs a design decision before implementation, not a same-pattern continuation of the
just-merged partial fix") and whose own tier-review comment asked for one design doc
covering #1247 and [#1194](https://github.com/rust-works/succinctly/issues/1194) together.
[#1242](https://github.com/rust-works/succinctly/issues/1242) was traced to the same
swallow points and is covered here too. See "Staged delivery" for what has landed and
"Follow-up issues" for what still needs tracking.

Every claim below was verified against `main` @ `cedee4cbb` with a `--release --features
cli` build and the pinned oracles (`/usr/bin/jq` 1.7.1, Homebrew `yq` v4). Where this
document contradicts an issue's own text, the contradiction is stated explicitly rather
than quietly corrected.

## Problem

A JSON or YAML string scalar — or an object/mapping key — that passes structural
validation but fails to **decode** (invalid UTF-8, an invalid escape, an invalid `\u`
codepoint) is silently swallowed. It becomes `null`, or `""`, or the field disappears, and
the process exits 0.

```console
$ printf 'a: 1\nb: "x\xe4y"\n' | succinctly yq -o=json '.'
{"a":1,"b":null}                                  # exit 0
$ printf 'a: 1\nb: "x\xe4y"\n' | yq -o=json '.'
Error: bad file '-': yaml: offset 11: invalid trailing UTF-8 octet    # exit 1
```

The swallow is not confined to the value that failed. Two consequences are worse than a
silent `null`, and neither is recorded in any issue:

**Output that no parser can read.** The YAML→JSON transcoders write the opening `"` (and
any successfully transcoded prefix) into the output *before* they can fail; the caller's
`Err(_)` fallback then appends `null` or `""` after that partial token:

```console
$ printf 'b: "x\qy"\n' | succinctly yq -o=json '.'
{"b": "xnull}                                     # exit 0, unparseable

$ printf '"a\qb": 1\n' | succinctly yq -o=json '.'
{"a"": 1}                                         # exit 0, unparseable
```

**Valid, unrelated sibling fields becoming invisible.** `JsonFields::find`/`find_cursor`
and `YamlFields::find`/`find_cursor` abort the *whole* field search on
`key.as_str().ok()?`, so one undecodable key hides every field after it — while `keys` and
`length` still report them:

```console
$ printf '{"\ud800":1,"b":2}' | succinctly jq '.b'
null                                              # but "b":2 is right there
$ printf '{"\ud800":1,"b":2}' | succinctly jq -c 'keys_unsorted, length'
["\ud800","b"]
2
```

## Root cause

`to_owned`/`to_owned_at_depth`/`to_owned_cursor`/`to_owned_cursor_at_depth`
(`src/jq/eval_generic.rs`) and `cursor_to_owned`/`cursor_to_owned_at_depth`
(`src/jq/lazy.rs`) all return a bare `OwnedValue`. They are the point at which a lazily
borrowed document scalar becomes an owned one, so they are where a decode failure is first
observable — and they have nowhere to put it.

`to_owned_at_depth`'s own doc comment is the clearest statement of the deferral anywhere
in the tree. It names #1098, records that PR #1190's `panic!()` attempt was reverted as
live-reachable through ordinary CLI usage, and states the intended shape:

> A correct fix needs to route through `EvalError`/`ErrorSink` instead, applied
> consistently across every sibling copy of this conversion and to object/mapping keys
> too.

#1192 fixed two sibling copies this way (`owned_from_standard_json_at_depth` in
`eval_generic.rs`, `standard_json_to_jq_value` in `jq_runner.rs`), establishing the
wording convention this design reuses: `EvalError::new(format!("{e}"))` for a value,
`format!("{e} in object key")` for a key.

## Three corrections to the issues' own premises

### 1. The "58+ call sites" cost estimate overstates the work

#1247 and its tier review both cite 58 `to_owned*` call sites in `eval_generic.rs` as the
reason making the family fallible is architecturally expensive. The count is real as a
`grep` count but is not a count of sites needing edits: of 62 matches, **13 are doc
comments, 8 are in `#[cfg(test)]`, and 7 are the recursive definitions themselves.**

Of the **53 remaining call sites, 40 already sit in a fallible context**:

| Enclosing return type | Sites | Edit needed |
|-----------------------|-------|-------------|
| `GenericResult<V>` (has an `Error(EvalError)` variant) | 21 | `GenericResult::Error(e)` |
| `Result<_, _>` | 11 | `?` |
| `Option<Control>` (has `Control::Error`) | 8 | `Some(Control::Error(e))` |
| Genuinely infallible helpers | 13 | see below |

The framing "thread `Result` through every one of 58+ call sites and their own callers,
transitively" assumes `Result` is the channel. It is not. The generic evaluator's own
error channel is **`GenericResult::Error(EvalError)`**, carried in the value domain — 
`eval_single` and `eval_builtin` both return a bare `GenericResult<V>`, not a `Result`.
Every one of those 40 sites sits in a `match` that *already* has an
`GenericResult::Error(e) => …` or `Control::Error` arm next to it.

The 13 genuinely infallible sites live in six functions, and several are error-message
previews that should stay infallible on purpose (see "Where the family stays infallible").

### 2. #1191, the stated prerequisite, is already fixed

#1247 states that #1191 (`YamlValue::Alias::as_str()` not recursing through nested
aliases) "is a direct confound specifically for `to_owned`'s detection logic on the YAML
side" and blocks the YAML half. **#1191 is closed.** `as_str()`'s `Alias` arm now resolves
the entire chain via `resolve_alias_chain` (`src/yaml/light.rs`, see
`MAX_ALIAS_CHAIN_DEPTH`'s doc comment). A valid double-alias-to-string no longer returns
`None`, so it can no longer be mistaken for a decode failure. Nothing here is blocked.

Note the related, still-open gap this does *not* cover: #1193 tracks the eight other typed
accessors (`as_bool`, `as_i64`, `as_f64`, `number_literal`, `as_object`, `as_array`,
`type_name`, `is_null`) that still resolve alias chains by genuine self-recursion.
Out of scope here.

### 3. #1194 is only half in scope, and the half that is not is the one its title names

`[xyz123] → [null]` goes through `StandardJson::Error(_) => OwnedValue::Null` and is fixed
by this design. **`{invalid} → {}` is not.** It never reaches any of these arms:
`JsonFields::uncons`'s `let value_cursor = key_cursor.next_sibling()?;` collapses "lone
bareword leaf, no sibling to pair as a value" into "no more fields" at the semi-index
layer, indistinguishable from a genuinely empty object. No field object is ever
constructed, so there is nothing to raise an error on.

Fixing that means `uncons` must be able to yield a malformed field, changing a public
API's contract and rippling through every caller. That is a separate design question and
is filed as a follow-up rather than absorbed here.

## Oracle behaviour (verified live)

Captured against `/usr/bin/jq` 1.7.1 and Homebrew `yq` v4, and a `--release --features
cli` build at `cedee4cbb`. Exit codes captured directly, not through a pipeline.

### JSON / `jq`

| Input | real jq | succinctly today |
|-------|---------|------------------|
| `{"a":"\xff"}` (raw invalid UTF-8, value) | `{"a":"�"}`, exit 0 | raw bytes echoed verbatim, exit 0 |
| `{"\xff":1,"b":2}` (raw invalid UTF-8, key) | `{"�":1,"b":2}`, exit 0 | raw bytes echoed, `.b` returns `null` |
| `{"a":"\ud800"}` (lone surrogate) | parse error, **exit 5** | echoed verbatim, exit 0 |
| `{"\ud800":1,"b":2}` | parse error, **exit 5** | echoed, `.b` returns `null` |
| `{"a":"\q"}` (invalid escape) | parse error, **exit 5** | echoed verbatim, exit 0 |
| `[xyz123]` | parse error, **exit 5** | `[null]`, exit 0 |
| `{invalid}` | parse error, **exit 5** | `{}`, exit 0 |

**jq does not reject raw invalid UTF-8 — it lossily replaces each bad byte with U+FFFD and
exits 0.** Only escape errors and barewords are parse errors. This asymmetry is
load-bearing for the design below and is not mentioned in any of the three issues.

Note also that succinctly's *identity* path echoes the malformed text verbatim: the
degrade to `null`/`""` only happens once a value is **materialized**. So the same document
behaves differently under `.` than under `length`, `to_entries`, `sort`, `-e`, or a field
lookup:

```console
$ printf '{"a":"\ud800"}' | succinctly jq '.a | length'
jq: error (at <stdin>:0): null (null) has no length     # wrong error, wrong cause
$ printf '{"a":"\ud800"}' | succinctly jq -c 'to_entries'
[{"key":"a","value":null}]
```

### YAML / `yq`

| Input | real yq | succinctly today |
|-------|---------|------------------|
| `b: "x\xe4y"` (raw invalid UTF-8) | `invalid trailing UTF-8 octet`, **exit 1** | `b: null`, exit 0 |
| `b: "x\qy"` (invalid escape) | `found unknown escape character`, **exit 1** | `{"b": "xnull}` — unparseable, exit 0 |
| `"a\qb": 1` (invalid escape in key) | `found unknown escape character`, **exit 1** | `{"a"": 1}` — unparseable, exit 0 |
| `"a\xe4b": 1` + `b: 2` | parse error, **exit 1** | key becomes `""`; `.b` returns `null` |

**yq does reject raw invalid UTF-8.** So the same upfront pass must reject in yq mode and
lossily replace in jq mode.

### What the existing validators already catch

| | invalid UTF-8 | invalid escape | lone surrogate | bareword |
|---|---|---|---|---|
| `succinctly json validate` | ✅ exit 1 | ✅ | ✅ | ✅ |
| `succinctly jq --validate` | ✅ exit 3 | ✅ | ✅ | ✅ |
| `succinctly yaml validate` | ❌ **exit 0** | ✅ exit 1 | n/a | n/a |
| `succinctly yq --validate` | ❌ **exit 0** | ✅ | n/a | n/a |

The JSON strict validator already produces exactly the right diagnostic for every JSON
repro in all three issues. The YAML validator has **no UTF-8 check at all** — that gap is
#1242's, and it means `--validate` is not a workaround for it today.

## Why not upfront validation for everyone

The tier review floats "re-run the existing strict validator only when materialization
hits a degrade point" as a cheaper third option. Costing it surfaced the reason the naive
version of it — validating every document upfront — is not viable.

Apple M5 Max, 8.4 MB generated JSON, min-of-7, process-spawn baseline (3.21 ms)
subtracted. Indicative sizing only: not an interleaved A/B per
[docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method), and the
machine was not idle. Re-measure properly before relying on these.

| | net cost | vs `jq '.users[0].name'` (13.1 ms) | vs `jq '.'` (187 ms) |
|---|---|---|---|
| whole-input UTF-8 validation (`text validate utf8`) | **1.1 ms** | +8% | +0.6% |
| full strict JSON validation (`json validate`) | **8.6 ms** | **+66%** | +4.6% |

Always-on strict validation costs two thirds of the runtime of a cheap navigation query.
That is disqualifying for a project whose documented advantage is being 1.5–2.5× faster
than jq, and it trades away exactly the "minimal validation" property
[docs/architecture/semi-indexing.md](../architecture/semi-indexing.md) documents as
deliberate.

Always-on **UTF-8** validation is a different proposition: 1.1 ms on 8.4 MB (~7.7 GiB/s,
the SIMD path), and `Utf8Error` already carries `offset`/`line`/`column` plus a `Display`
impl — precisely the diagnostic yq's message needs.

The tier review's own trigger-based variant ("re-validate only when a degrade fires") was
considered and rejected as unnecessary: at each swallow point the failure is *already*
known locally — we hold the failing bytes and the concrete `YamlStringError`/`Utf8Error`.
The difficulty was never detection, only reporting. Re-running a whole-document validator
would buy a slightly better message at the cost of a second full pass, and would still
need the same signal to know when to run.

## Design

**Detect where it happens; report through the channel that already exists; handle UTF-8 at
the boundary because that is the half the oracles handle at the boundary.**

### Part A — UTF-8 at the input boundary

Validate the whole input with `crate::text::utf8::validate_utf8` once, at the point the
runner reads the document.

- **yq**: on failure, report with `Utf8Error`'s offset/line/column through the existing
  `print_validation_error`-shaped formatter and exit 1, matching real yq. Additionally
  close the gap in `src/yaml/validate.rs` so `yaml validate` and `yq --validate` catch it
  too.
- **jq**: on failure, drive **U+FFFD replacement**, not rejection — jq exits 0 here. This
  also fixes a divergence in the opposite direction: succinctly currently emits raw
  non-UTF-8 bytes on the identity path where jq emits U+FFFD.
- Scope: **document input only.** Not `--raw-input`/`-R`, not DSV input, not `--arg`/
  `--args` strings.

Once this pass has run, `as_str()` can only fail on an **escape** problem, which
materially narrows what the rest of the design has to handle.

### Part B — escape/decode failures through `EvalError`

Make the six materializers fallible:

```rust
to_owned / to_owned_at_depth / to_owned_cursor / to_owned_cursor_at_depth  // eval_generic.rs
cursor_to_owned / cursor_to_owned_at_depth                                 // lazy.rs
```

all returning `Result<OwnedValue, EvalError>`, and route the error out through
`GenericResult::Error` / `Control::Error` / `?` per the table in "Correction 1".

Object and mapping **keys** get the same treatment: the current
`if let Some(key) = field.key_str()` and `if let Ok(cow) = key_str.as_str()` guards
silently drop the whole field, which is what
`to_owned_at_depth`'s doc comment means by "and to object/mapping keys too".

### Where the family stays infallible (deliberately)

`to_owned_with_cursor`'s call sites split. Four are genuine value positions inside
`eval_single`/`eval_builtin` and become fallible. The rest are **error-message previews**:

```rust
GenericResult::Error(EvalError::cannot_iterate(&to_owned_with_cursor(&value, cursor)))
GenericResult::Error(EvalError::has_no_length(&to_owned_with_cursor(…)))
GenericResult::Error(EvalError::has_no_keys(&to_owned_with_cursor(…)))
GenericResult::Error(EvalError::cannot_parse_as_number(&to_owned_with_cursor(…)))
```

An error is already being raised at each of those; the materialization exists only to
render the offending value into the message. Making them fallible would mean "failed to
build the error message for a failure", with no better outcome available. Split into two
functions — a fallible one for value positions and an explicitly lossy
`to_owned_for_diagnostic` for previews — and say so in the doc comment, so a later reader
does not read the surviving lossy call as an oversight.

`assert_nesting_depth`'s `panic!()` also stays. #1247 frames this design as either
reversing that precedent or avoiding it; it is neither. Depth overflow is a different
failure class — unbounded recursion that would otherwise abort the process — and its
guard is unaffected by making the same functions fallible for a *data* error.

### Part C — stop emitting invalid JSON

The YAML→JSON transcoders (`transcode_double_quoted_to_json`,
`transcode_single_quoted_to_json` and their `stream_` variants) are already fallible, but
they push the opening `"` into the output before they can fail. There are **six** call
sites with the same partial-write-then-fallback shape, in three pairs
(value arm, then key arm):

| Sink | value arm | key arm |
|------|-----------|---------|
| `stream_json_value` (`impl fmt::Write`) | ~1960 / fallback ~1979 | ~2010 / fallback ~2012 |
| buffered `&mut String` (A) | ~2114 / fallback ~2133 | ~2147 / fallback ~2149 |
| buffered `&mut String` (B) | ~2716 / fallback ~2736 | ~2750 / fallback ~2752 |

**Refined during implementation.** The prescription above was "record `output.len()`
before the call and truncate at each of the four buffered call sites". That is the wrong
place: it asks four callers to remember a rule none of them can see the need for. The
rollback belongs *inside* `write_yaml_string_to_json`, which is the function that makes
the partial write — it takes `&mut String`, so it can mark its own entry length and
`truncate` back to it on any error path, fixing all four call sites at once and leaving
nothing for a fifth caller to get wrong. Implemented that way.

The two streaming sites take `Out: fmt::Write`, which cannot be rewound, so those do need
a scratch `String` committed on success. Only the *slow* branch pays for it: the fast path
(no `\`, `\n` or `\r` in the scalar) decodes before it writes anything and cannot
partially fail, so it stays allocation-free — which matters, because this is P9's direct
YAML→JSON streaming path.

**Honest limitation:** for the streaming path, bytes already flushed for *earlier* parts
of the record cannot be retracted. jq and yq can reject cleanly because they DOM-parse the
whole input first; succinctly streams by design. The record will be truncated at the
failure point and the process will exit non-zero with a diagnostic. That is strictly
better than silent corruption but is a real divergence and should be documented as one,
not papered over.

### Part D — the sibling-hiding `find` loops

Four near-identical loops abort the entire search on a single undecodable key:

```rust
if key.as_str().ok()? == name {     // the `?` returns from `find`, not from this iteration
```

Replace with a skip, so an undecodable key no longer truncates the search. This is
independent of everything above, is the highest-severity item in the set (a *valid* value
silently becomes invisible), and carries no API or performance implication. It is also a
prerequisite for Part B: once decode failures raise errors, these loops must not swallow
them first.

## Site inventory

Verified by reading and by live repro at `cedee4cbb`. ✱ = not recorded in any issue.

| # | Site | Today | Stage |
|---|------|-------|-------|
| 1 | `json/light.rs:1023` `JsonFields::find` ✱ | aborts search, hides later fields | 1 |
| 2 | `json/light.rs:1042` `JsonFields::find_cursor` ✱ | same | 1 |
| 3 | `yaml/light.rs:4216` `YamlFields::find` ✱ | same | 1 |
| 4 | `yaml/light.rs:4235` `YamlFields::find_cursor` ✱ | same | 1 |
| 5 | `yaml/light.rs:1979` `stream_json_value` value arm ✱ | **invalid JSON** | 2 |
| 6 | `yaml/light.rs:2012` `stream_json_value` key arm ✱ | **invalid JSON** | 2 |
| 7 | `yaml/light.rs:2133` buffered value arm ✱ | **invalid JSON** | 2 |
| 8 | `yaml/light.rs:2149` buffered key arm ✱ | **invalid JSON** | 2 |
| 9 | `yaml/light.rs:2736` buffered value arm (2nd copy) ✱ | **invalid JSON** | 2 |
| 10 | `yaml/light.rs:2752` buffered key arm (2nd copy) ✱ | **invalid JSON** | 2 |
| 11 | `eval_generic.rs:166` `to_owned_at_depth` catch-all | `OwnedValue::Null` | 3 |
| 12 | `eval_generic.rs:122` `to_owned_at_depth` key guard | field dropped | 3 |
| 13 | `lazy.rs:707` `cursor_to_owned_at_depth` string arm | `String::new()` | 3 |
| 14 | `lazy.rs:721` `cursor_to_owned_at_depth` key guard | field dropped | 3 |
| 15 | `lazy.rs:658` `lazy_keys_array_to_owned` ✱ | key dropped from `keys` | 3 |
| 16 | `lazy.rs:606` raw-bytes key fallback ✱ | documented-defensive; align for consistency | 3 |
| 17 | `eval_generic.rs:1040` `StandardJson::Error` | `Null` | 4 |
| 18 | `lazy.rs:733` `StandardJson::Error` | `Null` | 4 |
| 19 | `eval.rs:427` `StandardJson::Error` | `Null` | 4 |
| 20 | `json/light.rs:2098` `StandardJson::Error` (JSON→YAML out) | `null` | 4 |
| 21 | `yaml/validate.rs` — no UTF-8 check at all ✱ | `yaml validate` exits 0 | 5 |
| 22 | `yq_runner.rs` / `jq_runner.rs` input read | no UTF-8 pass | 5 |

Two existing tests assert the buggy behaviour and must be inverted, not merely updated:

- `test_to_owned_degrades_to_null_on_string_decode_failure_1098` (`eval_generic.rs:4481`)
- `test_materialize_degrades_to_empty_string_on_decode_failure_1098` (`lazy.rs:914`)

`to_owned_at_depth`'s `#1098` deferral doc comment (`eval_generic.rs:150-166`) should be
deleted or rewritten in the same change that resolves it — it is the tree's clearest
statement of this deferral and should not outlive it.

## Staged delivery

Each stage lands as its own PR with its own oracle-verified sweep. Stages 1 and 2 are
plain bug fixes with no signature churn and go first: they are the highest severity and
lowest risk, and neither depends on the fallibility change.

**Stage 1 — sibling-hiding `find` loops (sites 1–4). ✅ landed.** No API change, no perf
implication. *Why first:* a valid value silently disappearing from a lookup is worse than
a malformed one becoming `null`, and this is a prerequisite for Stage 3.

**Stage 2 — invalid JSON output (sites 5–10). ✅ landed.** Rollback inside
`write_yaml_string_to_json` for the four buffered sinks; commit-on-success scratch buffer
for the two streaming ones. Semantics are unchanged — the value still degrades to `null`
and the key to `""` — but the emitted document is parseable. *Why second:* emitting
unparseable output is a correctness bug independent of the whole `to_owned` question, and
fixing it first means Stage 3's new error paths have a clean sink to fail into.

**Stage 3 — make the `to_owned` family fallible (sites 11–15). ✅ landed.** The main
change. *Why third:* everything it can raise now has somewhere correct to go.

**Site 16 (`lazy.rs:606`) is not actually landed, despite an earlier version of this doc
claiming it under this stage's "✅ landed."** `write_json_at_depth`'s raw-bytes key
fallback (`LazyKeysArray`'s defensive branch for a key with no text range) still reads
`if let Ok(s) = k.as_str() { write_json_body_jq(out, &s)?; }` with no `else` — on a decode
failure it writes nothing, silently rendering the key as `""` rather than raising, same as
before this stage. Confirmed by re-reading the site directly, not inferred. Low practical
severity — the comment there is right that a JSON object key is always a text-range token,
so this branch is close to unreachable — but "align for consistency" was never actually
done, and the site inventory's own "Today" column already said so; only the stage-level
"landed" summary overclaimed it.

Implementation notes, all discovered while doing it:

- **Detection needed a real signal, not an inference.** The plan assumed the catch-all
  could infer "decode failure" from `type_name() == "string" && as_str().is_none()`.
  That works but is indirect, and it re-creates exactly the ambiguity #1191 was about.
  Added `DocumentValue::string_decode_error() -> Option<&'static str>` instead — a
  provided method defaulting to `None`, overridden by JSON and YAML — so the two cases
  `as_str()` conflates ("not a string" vs "a string this document can't hand back") are
  separable at the source. The message text comes from `JsonError::message` /
  `YamlStringError::message`, split out of each type's `Display` so there is one
  definition rather than a second copy at an allocation-free call site.
- **Three more silent-drop sites, all found by the compiler once the types changed.**
  `DocumentFields::effective_fields`' dedup walk dropped any field whose key wouldn't
  decode, so `to_entries` returned one entry fewer than the document had — the field
  vanished before `to_entries`' own new check could see it. `lazy_keys_array_to_owned`
  dropped the key from `keys`. `to_owned_with_comments` had the same key guard as its
  two siblings.
- **A misleading-error class, not just a silent one.** Every builtin's type dispatch is
  a chain of `as_str()`/`as_array()`/… tests, so an undecodable string fails all of them
  and lands in the same `else` as a genuine type mismatch: `.a | length` reported
  `null (null) has no length` about a perfectly good string token. Added
  `type_error_or_decode_failure`, used at the seven sites that report such a fall-through,
  so the cause is named instead of a symptom two steps removed from it.
- **One sanctioned lossy materialization remains**, `to_owned_for_diagnostic`, for the
  call sites that materialize a value only to quote it into an error already being
  raised. Split out as its own function with the rationale in its doc comment, so the
  surviving lossy call doesn't read as an oversight.
- **Not everything that looks like a degrade is one.** `length` on an object answers from
  `fields.len()` without decoding, and `keys_unsorted` streams each key's raw byte span
  verbatim. Neither loses data — both are the same raw-passthrough class as `jq '.'` —
  so both were left alone, and the tests say why. **This stage's own key guards did not
  yet draw that line consistently**: `keys`/`to_entries` (and `has`, indirectly, via this
  stage's `to_owned`/`cursor_to_owned` family) raised on a decode-failure key instead of
  joining `length`/`keys_unsorted`/`.` in the raw-passthrough class — contradicting #1385's
  own "never a duplicate" rule one section up, on the same document. Closed by
  [#1642](https://github.com/rust-works/succinctly/issues/1642), which moved the
  substitution point into `DocumentValue::key_raw_source_span`/`document::key_display_string`
  so every key guard in this stage (and `effective_keys`'s own dedup step) shares one
  definition instead of each re-deriving the #1194-vs-decode-failure distinction.

**Stage 4 — the `StandardJson::Error` arms (sites 17–18). ✅ landed.** Fixes
`[xyz123] → [null]`, i.e. #1194's in-scope half. Small once Stage 3 exists: the arms
raise with the semi-index's own message, which is more specific than anything
reconstructible at the materializer.

**Sites 19–20 are not actually landed, despite an earlier version of this doc claiming
all of 17–20 under this stage's "✅ landed."** Re-reading both directly:

- **Site 19 (`eval.rs:427`)** still reads `StandardJson::Error(_) => OwnedValue::Null`,
  unchanged. Low live risk *today*: `eval.rs`'s evaluator only processes JSON re-serialized
  from an already-decoded `OwnedValue`, which by construction can't contain a structurally
  malformed value, so this arm is dead in practice — but the doc's own site-inventory
  rationale for including site 19 "anyway, as a cheap consistency fix" was never carried
  out, and a future code path that starts feeding raw, unvalidated JSON through
  `eval.rs::eval()` would silently reintroduce the bug with no test guarding against it.
- **Site 20 (`json/light.rs:2098`, `stream_json_as_yaml`'s `StandardJson::Error` arm —
  the JSON→YAML streaming output path)** also still writes `null` unchanged. This one is
  architecturally the same class of gap Stage 6 below is about (a `core::fmt::Write`-only
  error channel that cannot cleanly raise mid-stream) — it was miscategorized as a simple
  Stage-4 fix rather than folded into Stage 6 alongside its YAML→JSON and YAML→YAML
  siblings. **Its string arm is now fixed with those siblings by #1615** (Stage 6 below);
  the `StandardJson::Error(_)` arm itself still writes `null`, since that is a
  *structural* malformation (#1194's class), not a decode failure, and answering it here
  alone would split #1194's decision across two issues. See [yq Limitations § A key that
  will not decode is preserved](../compliance/yq/limitations.md#a-key-that-will-not-decode-is-preserved-as--rather-than-raising).

One site was reverted after being written, and later fixed for real by
[#1641](https://github.com/rust-works/succinctly/issues/1641). `print_json`'s
`StandardJson::Error` arm (`jq_runner.rs`) walks child cursors lazily and streams as it
goes, so by the time a nested error is reached it has already written the opening `[`.
The first attempt to raise there produced a truncated document plus a *generic* exit 1 —
worse on every axis than the silent `null` it replaced — and was reverted, with the
reasoning folded into Stage 6 below on the theory that this site needed the same new
error channel the YAML streaming siblings do.

**That theory was wrong.** `print_json` already returns `anyhow::Result<()>`, and by the
time #1641 revisited this site, the sibling object-member check a few lines above it
(Stage 3/#1194) had already established `MalformedJsonError` — a thin `anyhow`-downcastable
wrapper around `EvalError` that lets `run_jq` tell a diagnosable data error from a real I/O
failure and choose exit 5 over exit 1. The channel this arm was "missing" already existed
in the same function; the original revert predated that convention, not a genuine
architectural gap. Reusing it needed no new error type, only the one-line change from
`out.write_all(b"null")?` to raising through it. The truncation trade itself was never in
question — it is the same one `keys_unsorted` and a nested `{invalid}` already accepted;
see
[jq Limitations](../compliance/jq/limitations.md#duplicate-object-keys-collapse-except-under---preserve-input).

Site 20 (`json/light.rs:2098`, the JSON→YAML streaming output path) is a genuinely
different case from this one, not covered by #1641: it is built on `core::fmt::Write`,
which really does carry no richer error type, so it was folded into Stage 6 below
alongside its YAML→JSON and YAML→YAML siblings, and landed there with them (#1615).

Evaluation still continues past the bad value, so the good documents either side of it in
a multi-value stream are still processed (`ErrorSink`, #355). Real jq aborts the whole run
at the parse error instead; succinctly's behaviour here is the more useful of the two and
is a deliberate, tested divergence.

**Stage 5 — UTF-8 at the boundary (sites 21–22, #1242). ✅ landed.** yq rejects, jq
replaces with U+FFFD, `yaml validate` grows a UTF-8 check. *Why last:* it is the only
stage that adds work to every run, so it should be measured against a tree that is
otherwise already correct, and it can be reverted independently if the perf gate fails.

- **yq** rejects at the same byte offset and exit code as real yq, worded in this crate's
  own `YAML parse error:` convention rather than yq's `bad file '-':` shape, which
  succinctly has never matched for any other parse error. The check is unconditional, not
  gated on `--validate`: YAML 1.2 requires a UTF-8/16/32 stream, so this is a parse error,
  not an opt-in strictness preference.
- **`yaml validate`** grew the check too, reusing the `InvalidUtf8` variant that had been
  declared but never constructed — the scanner reads bytes and never decodes them, so
  nothing could ever have produced it.
- **jq** substitutes U+FFFD and exits 0, byte-identical to jq 1.7.1 on the probes above --
  but "the probes above" turned out not to cover every `Utf8ErrorKind`, and the
  substitution *algorithm* itself was wrong for three of them. This landed using
  `String::from_utf8_lossy`, which follows WHATWG's maximal-subpart rule: one U+FFFD per
  byte once a sequence is known bad. jq 1.7.1 instead collapses a whole *structurally
  valid* 3- or 4-byte sequence (a lead byte RFC 3629 lists as legitimate, i.e.
  `0xE0`-`0xEF` or `0xF0`-`0xF4`) into a single U+FFFD when its *decoded value* is
  overlong, a surrogate, or out of range -- `\xe0\x80\x80` gave three U+FFFD here against
  jq's one. Fixed by #1617 (`substitute_invalid_utf8_jq_style`,
  [src/text/utf8/mod.rs](../../src/text/utf8/mod.rs)), which special-cases exactly those
  three `Utf8ErrorKind`s on a structurally-valid lead and falls back to the
  already-agreeing WHATWG rule everywhere else (`0xC0`/`0xC1`/`0xF5`-`0xF7`, which RFC
  3629 excludes from the valid-lead-byte set at any length, included -- jq treats those
  like any other never-valid lead, unchanged).
  This also fixed two bugs in the opposite direction: the lazy path echoed the raw bytes,
  writing invalid UTF-8 to stdout, and the non-lazy path refused the file outright with
  `Failed to read file` when the read had in fact succeeded.
  **Not byte-identical even now, for this document-decode path specifically**: #1617's
  own review found a separate, narrower jq quirk -- jq's actual rule for
  `InvalidContinuationByte` is `len - pos < seq_len` (not enough total bytes remain from
  the lead byte's own position to satisfy the declared sequence length, whether the
  buffer genuinely ends or an intervening byte is simply invalid); when that holds, jq
  silently drops the *entire* remaining tail rather than keeping and rescanning it
  (`\xe1\x41` at a string's end -> `"�"` in jq, `"�A"` here, matching
  `String::from_utf8_lossy`'s own answer). Filed and later fixed *in the algorithm
  itself* as [#1717](https://github.com/rust-works/succinctly/issues/1717) -- likely an
  off-by-one in jq's own end-of-buffer lookahead rather than a designed rule, reproduced
  bug-for-bug per ADR-0018 rule 4. The fix reached `@base64d`/`@urid` immediately (#1719,
  whose own `input` is already scoped to one decoded string) but not this whole-document
  decode path, which substituted the entire file's bytes in one pass, so jq's own
  per-string trigger point essentially never coincided with "end of the whole buffer" --
  the `"\u{fffd}A"` example above was accurate right up to
  [#1743](https://github.com/rust-works/succinctly/issues/1743), which closed it.
  **#1743 needed no change to substitution *timing*, and this document's invariants are
  why.** The prediction recorded here -- that closing it would require deferring the
  decode until after structural parsing -- was wrong: `utf8_lossy_document` and
  `get_inputs` both gate on a whole-input SIMD `validate_utf8`, so a valid document never
  enters the repair at all, and the repair still emits a valid-UTF-8 buffer, preserving
  the invariant this whole document is built around (after the input boundary's pass,
  `as_str()` can only fail on an *escape* problem -- what lets `StandardJson::as_str`
  borrow out of the document, lets the printer echo a raw string span, and lets the
  raw-identity fast path exist). Locating string boundaries in non-UTF-8 text needs only
  a byte-level quote/backslash scan, sound because UTF-8 is self-synchronising: `"` and
  `\` can never occur inside a multi-byte sequence, so the scan agrees with jq's own
  byte-oriented lexer by construction. Only *how* the replacement buffer is built
  changed; see `src/jq/utf8_document.rs`. One subtlety the issue also got wrong: the
  scope is the escape-*decoded* string, not the raw source span, since escapes only
  shrink a string and so can push a lead byte over the `len - pos < seq_len` line its raw
  span would clear.
- Scope was document input only until #1719 also routed `@base64d`/`@urid`'s jq-mode
  output through this same `substitute_invalid_utf8_jq_style` call (for the
  overlong/surrogate/out-of-range case). At the time, this inherited the #1717 quirk
  above unfixed -- not new exposure, since `String::from_utf8_lossy` already gave the
  identical wrong answer for #1717's specific shape before #1719 (byte-identical output
  pre/post, confirmed live). #1717's later fix changed that: `@base64d`/`@urid` now match
  jq exactly for this quirk too, since the function's own `input` is naturally scoped to
  one decoded string already. `--raw-input` shares the jq path too, and real jq's own
  `-R` (non-slurp) trigger is scoped *per line*, not per whole file (live-verified:
  `printf 'a\xe1\x41\n' | jq -R '.'` drops the byte on a single-line file with an
  ordinary trailing newline); `get_inputs` originally substituted the whole raw buffer
  before splitting into lines, fixed by
  [#1742](https://github.com/rust-works/succinctly/issues/1742) as a caller-side reorder
  (split on `\n`, then substitute each line). `--raw-input --slurp` deliberately stays
  whole-buffer, because real jq is whole-buffer there too -- the entire input is one
  string, so the buffer's own end genuinely *is* that string's end. DSV input,
  `--arg`/`--argjson` and `--rawfile` remain untouched.
- **`--validate` is excluded, and getting that wrong was a live regression** caught in
  review. The substitution originally ran at read time, before `validate_json_input`, so
  the strict validator was handed an already-repaired document: `sjq --validate` exited 0
  on a file `succinctly json validate` still rejects with exit 1, silently dropping RFC
  8259 §8.1's mandatory UTF-8 check from the one flag whose entire purpose is strictness —
  and contradicting this document's own "What the existing validators already catch" table
  two sections up. Both routes had it (the lazy path substituted into `raw_inputs`; the
  materializing one decoded to a `String` in `get_inputs` first). Fixed by skipping the
  substitution under `--validate` entirely: nothing is lost, because any document that
  would have been substituted is a document the validator rejects. Pinned by
  `test_validate_still_rejects_invalid_utf8_1247` and, for the modes that never validate,
  `test_raw_input_still_substitutes_under_validate_1247`.
- Our `Utf8Error` offsets were checked against yq's for all six error kinds (invalid lead,
  bad continuation, overlong, surrogate, out-of-range, truncated) and agree exactly.

**Superseded — laptop numbers, kept for context only.** Interleaved A/B of the Stage 4
binary against the Stage 5 one, alternating order every repetition, min-of-9, gated first
on byte-identical stdout and exit code for every case (all five identical):

| case | before | after | delta |
|------|--------|-------|-------|
| `jq '.users[0].name'` 1 MB | 5.37 ms | 5.13 ms | −4.5% |
| `jq '.users[0].name'` 8.4 MB | 17.23 ms | 17.96 ms | **+4.2%** |
| `jq '.'` 1 MB | 16.91 ms | 16.98 ms | +0.4% |
| `yq -o json '.'` 1 MB | 16.62 ms | 16.65 ms | +0.2% |
| `yq -o json '.'` 10 MB | 126.19 ms | 126.80 ms | +0.5% |

This ran on an Apple M5 Max laptop under a load average near 10, which CLAUDE.md's
benchmarking discipline explicitly disqualifies for a final number — kept here only to
show the pinned run below (which found roughly 2–4× this cost) is not a contradiction of
this table, just a more sensitive instrument.

**The formal interleaved A/B, on both pinned machines — run, and it found a real
regression this table missed.** Method per
[docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method):
alternating order every repetition, reps=9, output identity gated before any timing is
read, a `--control` noise floor measured per group per machine. Full detail in PR #1391's
review thread.

*As Stage 5 first landed* (pair `201c3db14` → `c97c66d7f`, later renumbered to
`4713aa7cb` → `ea735849c` by an unrelated rebase — source patch verified byte-identical
across the rename):

| group | Apple M4 Pro | AMD Ryzen 9 7950X |
|---|---|---|
| `jq` on JSON | +0.94% | **+9.47%** (worst +17.9%) |
| `yq -o json` on ASCII YAML | −0.43% | +0.46% |
| `yq -o json` on 45%-non-ASCII YAML | **+3.57%** (worst +4.8%) | +0.06% |

This breached the document's own +8% ceiling on x86_64 — a real result the laptop run
above could not have found (the two architectures fail in *opposite* places: x86 on `jq`,
ARM on non-ASCII `yq`). Attribution (7950X, `jq` group, validation pass isolated by
rebuilding with it disabled): the UTF-8 pass itself costs **~1%**; the other ~6 points
came from an unrelated change in the same commit — `read_stdin`/`read_file` moving to
`String::from_utf8_lossy(&bytes).into_owned()`, which allocates and copies the *entire*
document even when it is already valid UTF-8 (`.into_owned()` still clones a
`Cow::Borrowed`).

**Fixed** by commit
[`03069c6a9`](https://github.com/rust-works/succinctly/commit/03069c6a9d5fc2782af20cb5d1ee50be706863ab)
(taking ownership of the existing buffer on the valid path instead of re-copying it),
measured before landing:

| comparison | AMD Ryzen 9 7950X | Apple M4 Pro |
|---|---|---|
| Stage 5 → Stage 5+fix | **−7.04%** (worst −13.2%) | +0.45% (neutral) |
| Stage 4 → Stage 5+fix | **+0.48%** — inside the control floor | +0.83% |

Post-fix, the always-on pass costs under 1% on both pinned machines — comfortably inside
the +8% ceiling Open Risk 5 below names as "the ceiling to defend." The one real
remaining cost is non-ASCII YAML on ARM (no NEON arm for `validate_utf8`, +4.8%/+5.8%
net of control on a 45%-non-ASCII 10 MB document) — still under the ceiling, ARM-only,
and a targeted NEON fix if it ever matters, not blocking.

**Stage 6 — make the YAML streaming path loud (new, split out of Stage 2). ✅ landed**
via [#1615](https://github.com/rust-works/succinctly/issues/1615). After Stages 2 and 3,
`succinctly yq -o=json '.'` on a document with a bad *escape* still emitted `{"b":null}`
at exit 0: the streaming transcoder's only error channel was `core::fmt::Result`, which
carries no message, so Stage 2 could make its output valid but not make it raise. Every
*materializing* route (a `--arg`-forced DOM, a multi-result filter, `to_entries`,
`length`) already raised. Stage 5 removed the invalid-UTF-8 half at the input boundary, so
what Stage 6 was left with is bad escapes only.

The richer error this stage was waiting on is `StreamFailure`
([src/jq/stream.rs](../../src/jq/stream.rs)) — `Fmt` for a genuine writer failure,
`Decode(EvalError)` for a scalar that will not decode. Because `From<core::fmt::Error>`
converts into it, every existing `?` inside the writers kept working unchanged; only the
family's return types and its leaf substitution sites needed touching, not each of its
recursive call sites. There is deliberately **no** `From<StreamFailure> for
core::fmt::Error`: that impl would let a plain `?` silently re-collapse a decode failure
into the message-less error this type exists to escape, so its absence is what makes every
such site a compile error instead.

Four value-writer sites, not the three the issue scoped:

1. `stream_json_value` (`yaml/light.rs`) — YAML→JSON, the site this doc originally named;
2. `stream_yaml_string_value` (`yaml/light.rs`) — YAML→YAML, the *default* invocation;
3. `stream_json_as_yaml`'s string arm (`json/light.rs`) — JSON→YAML (site 20 above), whose
   failure was a message-less `core::fmt::Error` rather than a silent substitution;
4. `stream_yaml_as_document`'s **root-scalar shortcut** (`yaml/light.rs`) — found only by
   testing, not by the site inventory. It deliberately bypasses 1–3 (see #996/#852) and so
   carried its own `unwrap_or(Cow::Borrowed("\"\""))` swallow, which meant the single most
   common navigated shape, `yq '.b'`, still printed `""` at exit 0 while `yq -o=json '.b'`
   on the same document raised.

Because the prefix can be left unterminated mid-value, a streamed decode failure stops the
run rather than continuing to the next document (welding the next `---` onto a truncated
line produces output that re-reads as valid YAML with a fabricated value), and `--inplace`
leaves the file byte-identical. `StreamStats::truncated` is what separates this from an
ordinary uncaught evaluation error, which writes nothing to `out` and so still continues
per #355 — gating on `stats.error` instead would have silently narrowed that divergence.

Mapping **keys** are excluded on purpose (`Undecodable::PreserveEmpty` in
`yaml/light.rs`): #1642 settled that a key with no scalar form is preserved as `""` on
every route, and `keys`/`to_entries`/`length` all report it, so raising in a streamed
identity alone would have re-opened the same one-document-many-answers split on the key
axis. The output already written before a failure is kept rather than buffered and
discarded, matching #1641/#1679 and the "buffering the whole record" non-goal below.

## Non-goals (explicit, to prevent scope creep)

- **`{invalid} → {}` (#1194's headline repro).** Requires `JsonFields::uncons` to yield a
  malformed field rather than collapsing to `None`; a public API contract change with its
  own call-site sweep. Filed as a follow-up.
- **#1193's eight self-recursive alias accessors.** Adjacent to #1191, unrelated to decode
  failure, unaffected either way.
- **#965** (remaining `ResolvedScalar`/`StandardJson` → `OwnedValue` duplication). The
  five textually-similar copies of this conversion are exactly what #965 is about, and
  this work touches four of them — but consolidating them is a refactor with its own risk
  profile and should follow, not accompany, a behaviour change. Sequenced after this per
  #1283's cluster E.
- **`eval.rs`'s private copy of the `to_owned` family** — corrected by #1746 (this premise
  was false). #1247's "not live-reachable" claim held for the shipped CLI (`jq_runner.rs`/
  `yq_runner.rs` route reads through `eval_generic::eval_with_cursor`, and every write path
  eagerly, strictly materializes upfront), but the crate's public library API
  (`succinctly::jq::eval`/`eval_using`) feeds raw, untrusted bytes straight into
  `JsonIndex::build` and this evaluator, with no re-serialization step and no upstream
  decode-failure check to have already caught it. #1746 added a new fallible
  `to_owned_checked`/`to_owned_checked_at_depth` sibling, mirroring
  `eval_generic::to_owned_at_depth`'s already-fixed shape, and fixed the whole-document
  read/write family (`builtin_path`, `eval_assign`, `eval_update`, `yq_assign_noop_check`,
  `builtin_setpath`, `builtin_del`, `delpaths_one`) plus `map`/`map_values`/`to_entries`.
  A systematic audit run during that PR's own review found roughly 50 more builtins with
  the identical shape (a container's own elements/fields materialized straight into a
  returned `QueryResult::Owned`/`ManyOwned`, uncaught) — `flatten`/`pick`/`omit`/`add`/
  `sort`/`unique`/`group_by`/`min_by`/`max_by`/`getpath`/`recurse`/`walk`/`tostream`/format
  functions (`@base64` etc.) among them — filed as #1755 rather than folded into #1746's
  own PR, which was already a substantial diff closing the originally-reported repro. The
  plain, infallible `to_owned`/`to_owned_at_depth` remains in place for those sites plus
  this file's remaining ~150 genuine `QueryResult`-collector-boilerplate/error-message-
  rendering sites (audited and confirmed safe or out of scope, per #1755's own site list).
- **Buffering the whole record so the streaming path can reject cleanly.** That would
  reverse P9 (direct YAML-to-JSON streaming, a 2.3× win) for a malformed-input edge case.

## Open risks for an implementer to sanity-check

1. **`GenericResult::Error` is catchable.** A decode failure routed through it becomes
   suppressible by `.a?` or `try .a catch`, where jq's parse error is not catchable at all
   (it happens before the program runs). Decide explicitly whether that is acceptable or
   whether these need `Halt`-like uncatchability. This is a genuine semantic difference
   from both oracles and the design above does not resolve it.

   **Resolved by #1620**: uncatchable both ways, but *not* via `Halt` -- `Halt` is
   whole-process-fatal (aborts every remaining record in a multi-document stream), which
   would trade this divergence for a worse one against the already-accepted "a malformed
   value doesn't abort the rest of the stream" divergence below. Instead, `EvalError` grew a
   `decode_failure`/`is_decode_failure` pair (`src/jq/error.rs`), mirroring the existing
   `is_invalid_path_expression` precedent: a decode failure still travels through the
   ordinary `Control::Error`/`GenericResult::Error`/`QueryResult::Error` channel (so it stays
   per-record), but every `?`/`try`/`catch` boundary in both evaluators now checks
   `is_decode_failure()` first and lets it pass through unmatched instead of suppressing or
   catching it.
2. **JSONTestSuite `i_` cases.** `tests/json_test_suite.rs` covers implementation-defined
   inputs; making decode failures loud can flip them in either direction. Check what the
   suite currently expects *before* regenerating anything.
3. **Golden fixtures.** `tests/jq_golden_tests.rs`, `tests/yq_golden_tests.rs`,
   `tests/cli_golden_tests.rs`, plus the `yq-drift` CI job. A behaviour change here is
   exactly the kind that shows up as unrelated-looking golden churn.
4. **`yq --validate` is a documented gate today.** It currently passes invalid-UTF-8
   documents. Making it reject them is the point of Stage 5, but it is a user-visible
   behaviour change to call out in the changelog, not just a bug fix.
5. **Stage 5's pass is charged to every input, including the ones it cannot help.** This
   is the exact trap the O6 `HAS_CR` work documented (see CLAUDE.md): measure against the
   workload where succinctly is *already fastest*, not the one where it is slowest. The
   +8%-on-the-cheapest-query figure above is the ceiling to defend; anything worse means
   the pass is in the wrong place.
6. **`-R`/`--raw-input` and DSV must be excluded from the UTF-8 pass.** Raw input is
   explicitly byte-oriented; rejecting there would be a regression, not a fix.

## Verification approach for the implementation PRs

- **Oracle matrix as a test, not a one-off.** Script both behaviour tables above against
  `/usr/bin/jq` (the 1.7.1 pin — Homebrew's `jq` is 1.8.2 with different error text) and
  Homebrew `yq` (which *is* the yq pin), asserting **exit codes** as well as stdout, and
  covering both the identity path and the materializing paths (`length`, `to_entries`,
  `sort`, `-e`, field lookup) — the divergence only appears on the latter.
- **Output must be parseable.** For every yq repro, pipe `-o=json` output through `jq .`
  and require either a clean parse or a non-zero exit. This is the assertion that would
  have caught sites 5–10.
- **Per-site regression tests** named `_1247`/`_1242`/`_1194`, following the existing
  `_1192` tests in `jq_runner.rs` and `eval_generic.rs`.
- **Interleaved A/B for Stage 5** on both ARM and x86_64, reporting the curve over 2–3
  sizes ≥ 1 MB, per [docs/guides/benchmarking.md](../guides/benchmarking.md#ab-benchmarking-method).
- **Full CI gate derived from `.github/workflows/ci.yml`**, not hand-listed — including
  `cargo fmt --check`, *both* clippy invocations, and `cargo check --no-default-features`
  for `no_std` (nothing in this design needs `std`, but the UTF-8 pass lives near the
  boundary where that is easy to break).

## Critical files

| File | Role |
|------|------|
| `src/jq/eval_generic.rs` | `to_owned` family, `GenericResult`, `Control`, 40 of the call sites, the `#1098` deferral comment |
| `src/jq/lazy.rs` | `cursor_to_owned` family, backs `JqValue::materialize()`/`into_owned()` |
| `src/jq/eval.rs` | third `StandardJson::Error` copy (site 19), not live-reachable |
| `src/json/light.rs` | `JsonFields::find`/`find_cursor`/`uncons`, JSON→YAML output |
| `src/yaml/light.rs` | `YamlFields::find`/`find_cursor`, `stream_json_value`, all six transcode call sites |
| `src/yaml/validate.rs` | strict YAML validator — missing the UTF-8 check entirely |
| `src/json/validate.rs` | strict JSON validator — already correct, the model for the YAML one |
| `src/text/utf8/mod.rs` | `validate_utf8`, `Utf8Error` (offset/line/column + `Display`) |
| `src/bin/succinctly/jq_runner.rs` | input read, `--validate`, `print_validation_error`, `_1192` test precedent |
| `src/bin/succinctly/yq_runner.rs` | input read, DOM path, `to_owned_with_comments` |
| `src/bin/succinctly/output.rs` | `ErrorSink` — the "evaluation continues, exit code remembers" convention (#355) |

## Related

- [#1247](https://github.com/rust-works/succinctly/issues/1247) — the issue this document
  is the deliverable for.
- [#1620](https://github.com/rust-works/succinctly/issues/1620) (resolved) — Open Risk 1,
  decode-failure catchability, filed and settled per this document's own follow-up list.
- [#1642](https://github.com/rust-works/succinctly/issues/1642) (resolved) — Stage 3's key
  guards raised inconsistently with `length`/`keys_unsorted`/`.`, contradicting #1385;
  fixed by moving every key guard onto one shared substitution point.
- [#1192](https://github.com/rust-works/succinctly/issues/1192) (closed) — fixed two
  sibling copies and established the `EvalError` wording convention reused here.
- [#1098](https://github.com/rust-works/succinctly/issues/1098) (closed) — the original
  report; PR #1190's `panic!()` fix was reverted from it as live-reachable, which is why
  `to_owned_at_depth`'s doc comment exists.
- [#1194](https://github.com/rust-works/succinctly/issues/1194) — half in scope (see
  Correction 3).
- [#1641](https://github.com/rust-works/succinctly/issues/1641) — bare `.[]` and
  `print_json`'s `StandardJson::Error` arm, the two routes left after #1194/#1608/#1628
  closed the materializing side. Corrected this document's own claim that the `print_json`
  site needed Stage 6's new error channel — see the "One site was reverted" discussion
  above.
- [#1242](https://github.com/rust-works/succinctly/issues/1242) — the YAML invalid-UTF-8
  repro; Stage 5.
- [#1191](https://github.com/rust-works/succinctly/issues/1191) (closed) — the stated
  prerequisite, already satisfied.
- [#1193](https://github.com/rust-works/succinctly/issues/1193) — the eight remaining
  self-recursive alias accessors; out of scope.
- [#965](https://github.com/rust-works/succinctly/issues/965) — the duplication this work
  touches but does not consolidate; sequenced after.
- [#1283](https://github.com/rust-works/succinctly/issues/1283) — the jq cluster
  sequencing plan that groups these issues.
- [`docs/plan/jq-path-trackability-deferral.md`](jq-path-trackability-deferral.md) — the
  #1282 deliverable this document's structure follows.

## Follow-up issues

1. ~~One implementation issue per stage above, linking back here.~~ Stage 6 (the streaming
   YAML-output gap this document's own text names) filed as
   [#1615](https://github.com/rust-works/succinctly/issues/1615). The other stages landed
   directly rather than through a separate tracking issue each.
2. ~~**`JsonFields::uncons` cannot represent a malformed field** — #1194's headline repro
   (`{invalid} → {}`), see Correction 3.~~ Filed and fixed for the two routes that
   materialize nothing today as
   [#1641](https://github.com/rust-works/succinctly/issues/1641), by working around the
   ambiguity at each caller (`effective_fields_checked`, `MalformedJsonError`) rather than
   changing `uncons`'s own contract. One instance remains open: `obj | map(f)`
   (`LazySource::Values` in `eval_generic.rs`) has the identical gap and was deliberately
   left unfixed there — see
   [jq Limitations](../compliance/jq/limitations.md#duplicate-object-keys-collapse-except-under---preserve-input).
   [#1194](https://github.com/rust-works/succinctly/issues/1194) itself (the original
   headline repro this design doc grew out of) is closed.
3. ~~**Decide catchability of decode errors** — Open Risk 1, if it is not settled during
   Stage 3's review.~~ Filed as [#1620](https://github.com/rust-works/succinctly/issues/1620),
   resolved there -- see Open Risk 1 above.
