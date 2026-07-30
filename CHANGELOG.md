# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
  path component (`path(.. | .[.k])`, #412), and — through a pre-existing defect
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

  Not fixed by this change, since the M2 fast path only gives `Expr::Identity`
  a true cursor result — `Field`/`Index`/`Iterate` still evaluate to an owned
  `GenericResult::One`/`Many` and go through `to_owned()` when streamed, so a
  duplicate key *nested inside* a navigated field (`.a` where `a`'s value has
  a repeated key) still collapses, in both compact and pretty output, exactly
  as before this fix (tracked as a comment on #443, which already covers the
  same `to_owned()`/`IndexMap` mechanism for `to_entries`); `--slurp`,
  `--inplace` (`yaml_to_owned_value`), and `jq --preserve-input` pretty output
  (`standard_json_to_jq_value`, gated by `jq_runner.rs`'s
  `can_use_raw_identity`) go through their own separate, still-`IndexMap`-backed
  conversions and are tracked in #478.

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
  `None` before reaching the operator. **Not fixed**: the whole-stream error
  model means `(1, error("x")) // 2` still yields `2` where jq yields `1` —
  `QueryResult::Error` is a property of the stream, not of one output, so
  partial-then-error is unrepresentable; a faithful fix is the same larger
  change `eval_comma` needs for #400.

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
  drops to two entries. **Not fixed**: `QueryResult` still models an error or a
  `break` as a property of the whole stream rather than of one output, so
  `(1,error("x")) // 2` yields `2` where jq yields `1`, and a mid-stream error
  or `break` in `and`/`or` discards the outputs already computed —
  `label $out | ((true,true) and (1, break $out))` yields nothing where jq
  yields `true` (#400); and `if`/`select` still collapse a multi-output condition
  to its first output (#378, sibling of #354). `//` also suppressed left-hand
  errors rather than propagating them; fixed separately below (#377).
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
  +1.8% (`.`) to +6.4% (`.[].name`). See `docs/parsing/yaml.md` for the
  per-change attribution and the const-generic option that would buy it back.
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
