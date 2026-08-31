# jq Error Message Conformance and Known Limitations

[Home](../../../) > [Docs](../../) > [Compliance](../) > jq Limitations

This page records how closely succinctly's evaluator reproduces jq's *error messages*,
measured against the pinned `jq` binary rather than asserted. It is one of the two pages
[ADR-0018](../../adrs/adr-0018.md) obliges: that record makes jq-fidelity the rule for jq
mode, permits divergence only under four named conditions, and requires every divergence to
be written down — so **this page is the enumeration of exceptions to ADR-0018** for jq mode,
as [yq Limitations](../yq/limitations.md) is for yq mode. Since
[#158](https://github.com/rust-works/succinctly/issues/158) bound the raised value as
`catch`'s input, the message text is readable from a filter — `try f catch (if
test("Cannot index") then … else … end)` is a real jq idiom — so the wording is part of
the observable surface, not stderr decoration.

Every expectation is captured from jqlang/jq at the version in
[`tests/data/jq-golden/JQ_VERSION`](../../../tests/data/jq-golden/JQ_VERSION); regenerate
and verify with:

```bash
./scripts/sync-jq-error-messages.sh          # recapture from the pinned jq
./scripts/sync-jq-error-messages.sh --check  # verify the table has not drifted
cargo test --features cli,regex --test jq_error_message_tests -- --nocapture
```

For jq *feature* coverage rather than error wording, see
[jq Language Support](../../reference/jq-language.md).

## Summary

Measured against jq-1.7.1 over the 221 probes in
[`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv), through
**both** evaluators — the full one (`src/jq/eval.rs`) and the generic one
(`src/jq/eval_generic.rs`, which the CLI uses):

| Dimension                                    | Result              | Meaning                                                |
|----------------------------------------------|---------------------|--------------------------------------------------------|
| **Message text** (both evaluators, verbatim) | **219/221 = 99.1%** | Byte-identical to jq                                   |
| **Wording divergences**                      | **2**               | Both evaluators raise, but word it differently from jq |
| **Behaviour / parser gaps**                  | **0**               | succinctly does not raise the error at all             |

These three numbers are asserted, not maintained by hand: `jq_error_message_tests.rs`
parses them back out of this page and fails if they drift from the corpus (they went stale
twice while #356 was being written).

The non-passing probes are enumerated individually, with a category, reason and issue
link, in
[`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt).
That file is the machine-readable source of truth; the test asserts it matches reality
exactly in both directions — a newly diverging probe and a newly matching one both break
the build — so it cannot silently drift from this page.

Crucially, **none of them is a wording bug**. In each case succinctly returns a value
or fails to compile the filter, so there is no message to compare; the wording is already
correct in `src/jq/error.rs` and will be reached once the underlying bug is fixed.

What the corpus *cannot* see is the mirror image — a filter on which succinctly raises an
error and jq returns a value — because a probe is only admitted if jq errors on it. Those
are listed in [Where succinctly errors and jq does not](#where-succinctly-errors-and-jq-does-not).

## The message vocabulary

jq reuses a small number of sentence shapes across many operations — `.a`, `.a = 1`,
`del(.a)`, `getpath(["a"])` and `. as {a:$a}` all report the *same* indexing error. Each
shape has one named constructor in [`src/jq/error.rs`](../../../src/jq/error.rs), so the
vocabulary is enumerable in one place rather than inlined at the ~300 raise sites where
it used to live. That inlining is how the two evaluators drifted from each other as well
as from jq: before #356 they reported `expected array or object, got number` and `cannot
iterate over number` for the same condition, and `cannot parse 'a' as number` against
`cannot convert 'a' to number` for `tonumber`.

| Shape                                                    | Constructor                                   |
|----------------------------------------------------------|-----------------------------------------------|
| `Cannot index <t> with string "<k>"`                     | `cannot_index`, `cannot_index_with_field`     |
| `Cannot index <t> with <key-type>`                       | `cannot_index_with_type`                      |
| `Cannot iterate over <t> (<v>)`                          | `cannot_iterate`                              |
| `<a> and <b> cannot be added\|subtracted\|…`             | `binary_op`                                   |
| `… because the divisor is zero`                          | `divisor_is_zero`                             |
| `<a> and <b> cannot have their containment checked`      | `containment_check`                           |
| `<a> and <b> cannot be iterated over`                    | `pair_cannot_be_iterated`                     |
| `<v> has no keys` / `has no length`                      | `has_no_keys`, `has_no_length`                |
| `<v> cannot be sorted, as it is not an array`            | `cannot_be_sorted`                            |
| `<v> cannot be matched, as it is not a string`           | `cannot_be_matched`                           |
| `<t> not a string or array`                              | `not_string_or_array`                         |
| `<v> is not a string`                                    | `is_not_a_string`                             |
| `<v> cannot be parsed as a number`                       | `cannot_parse_as_number`                      |
| `<v> only strings can be parsed`                         | `only_strings_can_be_parsed`                  |
| `<v> only strings have UTF-8 byte length`                | `no_utf8_byte_length`                         |
| `Cannot check whether <t> has a <key-type> key`          | `cannot_check_has`                            |
| `Cannot use <t> (<v>) as object key`                     | `cannot_use_as_object_key`                    |
| `Invalid numeric literal at EOF at line 1, column <n> …` | `invalid_numeric_literal`                     |
| `Invalid path expression with result <v>`                | `invalid_path_expression`                     |

`EvalError::type_error` ("expected X, got Y") survives for the raise sites jq has no
counterpart for — succinctly extensions (`at_offset`, `@dsv`, `pick`/`omit`, module
loading) and builtins jq does not define. Anything jq also reports should use a named
constructor instead.

### One sentence covers a family, so probe the whole family

jq derives many builtins from others (`ascii_upcase` and `ascii_downcase` are both
`explode | map(…) | implode`; `to_entries` and `keys` both go through `keys_unsorted`;
`indices`, `index` and `rindex` all index their input with the pattern), so a sentence
fixed for one member is owed by every member. Fixing only the member a probe named left
`1 | with_entries(.)` saying `number (1) has no keys` while `1 | to_entries` beside it
still said `expected object, got number`. The corpus now carries at least one probe per
member for these families, and the sites that shared wording share a definition —
`non_string_pattern` and `unsearchable_input` in `src/jq/eval.rs` are the two refusals
behind all three string searches.

The families are worth naming, because "fix the site the probe names" caught this twice
in a row: the first pass fixed the *pattern* half of `indices`/`index`/`rindex` and left
their *input* half saying `expected string or array, got number`, one arm below. The rule
that holds is per family, not per raise site.

Sometimes jq's own source names the family outright. `strings` on the pinned binary yields:

```jq
def from_entries: map({(.key // .Key // .name // .Name): (if has("value") then .value else .Value end)}) | add | .//={};
def with_entries(f): to_entries | map(f) | from_entries;
```

So `from_entries` *is* object construction, and `with_entries` *is* `from_entries` — one
raise site in jq behind `{(0):1}`, `[{"key":0}] | from_entries` and
`{"a":1} | with_entries(.key = 0)` alike, which is why all three share
`cannot_use_as_object_key`. Succinctly had it as three: two hand-written copies of the
entry lookup (`from_entries` and `with_entries` reimplemented it separately, and neither
called the other) plus `key must be a string` in `eval_object_construction`. Both copies
*dropped* the entry rather than refusing it, so the caller got a smaller object with no
indication anything was lost — #391. The lookup now has one definition,
`entries_to_object` in `src/jq/eval.rs`, and `with_entries` is composed from
`builtin_to_entries` and it rather than restating either.

That composition is also what fixed `[1,2] | with_entries(.)`. jq reaches
`Cannot use number (0) as object key` because `to_entries` accepts an array — its keys are
the indices — and hands those number keys to `from_entries`. Succinctly's `with_entries`
matched only an object, so it reported `array ([1,2]) has no keys` from a type check jq
has not got. Deriving the builtin from the two it is defined over is what makes the right
sentence arrive without anyone choosing it.

The same source line pins the *alias* semantics, and its two halves disagree on purpose:
the key is a `//` chain (`key`, `Key`, `name`, `Name` — an alias holding `null` or `false`
is passed over in favour of a later one), while the value is a presence test (`value`, then
`Value` — an explicit `"value": null` beats a `"Value"` beside it). Succinctly had accepted
`k` and `v`, which jq does not, and neither `Key`/`Name`/`Value`, which it does. Correcting
the chain was a precondition for raising the error rather than a tidy-up beside it:
refusing a non-string key while still reading the wrong aliases would have failed
`[{"Key":"a","value":1}]` and `[{"key":null,"name":"a","value":1}]`, both of which jq
answers. The chain is pinned by golden cases (`from_entries_key_aliases`,
`from_entries_alias_falls_through_null`, `from_entries_value_aliases`) rather than probes,
because it is value behaviour and no probe can hold a filter jq does not error on.

"Passed over in favour of a later one" is exact, and *not* the same as falling through: the
chain's **last** alias has nothing later to be preferred, so `a // b` yields `b` whatever
`b` is. A falsy `.Name` is therefore the key, and `[{"Name":false}] | from_entries` is
`Cannot use boolean (false) as object key` — not the `null (null)` a uniform fall-through
would produce. Reading the tail as falling through too is the easy mistake, and the case
that hides it is a tail that is merely *absent*, which really is `null`; the probe pair
`from_entries_falsy_tail_key` / `from_entries_absent_tail_key` exists to separate them, with
`from_entries_alias_falsy_tail` pinning the same case end-to-end through the CLI. This half
of the chain *can* be probed, unlike the rest of it, precisely because jq errors here.

### Value rendering and truncation

Most shapes embed the offending value as `<type> (<json>)`. jq truncates that dump to a
fixed-width buffer (`jv_dump_string_trunc` with `char errbuf[15]`): a dump of at most 14
bytes is used verbatim, anything longer keeps its first 11 bytes and gains a `...`
suffix. Reproduced exactly:

```bash
$ echo '"abcdefghijkl"'  | sjq -c 'try .[] catch .'
"Cannot iterate over string (\"abcdefghijkl\")"     # 14 bytes — verbatim
$ echo '"abcdefghijklm"' | sjq -c 'try .[] catch .'
"Cannot iterate over string (\"abcdefghij...)"      # 15 bytes — truncated
```

String *keys* in indexing messages are not truncated; `.["aaaaaaaaaaaaaaaaaaaa"]` on a
number reports the whole twenty-character key.

## Truncation that splits a multi-byte character

jq cuts the dump at a byte offset and will happily split a UTF-8 sequence, emitting
invalid UTF-8:

```bash
$ echo '"あああああ"' | jq '.[]' | cat -v
jq: error (at <stdin>:1): Cannot iterate over string ("M-cM-^AM-^BM-cM-^AM-^BM-cM-^AM-^BM-oM-?M-=...)
                                                     # "あああ + a lone 0xE3 byte
```

A Rust `String` cannot hold that, so `dump_truncated` snaps back to the nearest character
boundary and emits `"あああ...` — one byte shorter, no replacement character. The two
agree whenever the cut lands on a boundary, which includes every 2-byte character
(`"ααααα...`, exactly 11 bytes).

This case is deliberately **absent from the probe corpus**: the captured table is a UTF-8
file read with `include_str!`, so jq's byte-exact output here is not representable. It is
recorded in prose instead rather than dropped silently.

## jq's own UTF-8 replacement-character substitution: matched, at each caller's own granularity

`substitute_invalid_utf8_jq_style` ([src/text/utf8/mod.rs](../../../src/text/utf8/mod.rs),
#1617) matches jq 1.7.1's maximal-subpart substitution rule for document/raw-input decode,
and — since #1719 — for `@base64d`/`@urid`'s own invalid-UTF-8 output too, collapsing a
structurally-valid overlong/surrogate/out-of-range 3-/4-byte lead to a single U+FFFD where
`String::from_utf8_lossy`'s WHATWG rule gives one per byte.

jq has a second, separate quirk here (#1717): for `InvalidContinuationByte`, jq's actual
rule is simply `len - pos < seq_len` — if `seq_len` bytes aren't all physically present
from the lead byte's own position onward, jq collapses the *entire* remaining tail into
one U+FFFD, **regardless of why** they're short (the buffer genuinely ends, or one of the
bytes that *is* present fails the continuation check partway through). Only once
`seq_len` bytes are actually present does jq validate each continuation byte and fall
back to WHATWG-style rescan-at-the-bad-byte. An earlier version of this fix instead
conditioned the drop on "the offending byte is `input`'s own last byte", which
undercounts: `[0xF0, b'A', b'B']` (a 4-byte lead, one invalid continuation, *one* byte of
headroom before end-of-input — so the offending byte is *not* `input`'s last byte) still
collapses in real jq. Live-verified across every 2-/3-/4-byte lead shape and headroom
amount up to 3 bytes (a 2-byte lead's headroom-0 case is its only failure mode at all —
`\xc2\x41` -> `"\u{FFFD}A"`, kept, since `len - pos == seq_len` there), plus a 750-case
differential sweep against varied lead/continuation/trailing combinations, all matching:

```bash
$ printf '"4UE="' | jq -c '@base64d | explode'          # base64 "4UE=" decodes to [0xE1, 0x41]
[65533]
$ printf '"4UE="' | sjq -c '@base64d | explode'
[65533]
$ printf '"8EFC"' | jq -c '@base64d | explode'          # decodes to [0xF0, 0x41, 0x42] -- the
[65533]                                                 # previously-missed, one-byte-headroom shape
$ printf '"8EFC"' | sjq -c '@base64d | explode'
[65533]
```

**The algorithm is granularity-independent — it only asks how many bytes remain in the
slice it was handed — so what a caller passes decides where the quirk fires.** Real jq's
own trigger is scoped to *each JSON string's own decoded bytes* (document mode, inside
`jv_string_sized`) or *each line* (`--raw-input`, confirmed live: `printf 'a\xe1\x41\n' |
jq -R '.'` drops the byte even though the file's own trailing newline follows). Neither is
"the whole buffer's own end" in a realistic multi-field document or multi-line file, and
jq's trigger is not rare — it fires on *any* string/line ending in the right byte shape,
however much more content follows elsewhere in the file — so a whole-file caller would
essentially never reproduce it. Every caller is now scoped the way jq scopes it:

| Caller                                                 | Scope                                                          | Issue |
|--------------------------------------------------------|----------------------------------------------------------------|-------|
| `@base64d`/`@urid` (`owned_string_from_decoded_bytes`) | One decoded string, already                                    | #1719 |
| `--raw-input` (non-slurp)                              | Per line, split before substituting                            | #1742 |
| JSON document / `--slurp` / `--seq`                    | Per JSON string                                                | #1743 |
| `--raw-input --slurp`                                  | Whole buffer — **matching real jq**, which has one string here | —     |

`--input-dsv` also stays whole-buffer: DSV is not JSON (its fields are `""`-doubled, not
backslash-escaped), and neither oracle reads DSV at all.

```bash
$ printf '{"a":"\xe1\x41","b":1}' | jq -c '.a'         # jq drops the 'A'
"�"
$ printf '{"a":"\xe1\x41","b":1}' | sjq -c '.a'        # since #1743: matches
"�"
```

Two things about #1743 are worth recording, because its own issue text got both wrong:

- **The scope is the escape-*decoded* string, not the raw source span.** Escapes only ever
  shrink a string, so they can push a lead byte over the `len - pos < seq_len` line that
  its raw span would clear. `"\xe1A"` is seven raw bytes but two decoded — one short
  of the three the `0xE1` lead declares — and real jq collapses it to a bare U+FFFD,
  dropping the `A` (oracle-verified, as is the 4-byte analogue `"\xf0\x90A"`). A
  repair scoped to the raw span keeps the `A` and is wrong.
- **It needed no per-string substitution *timing*, and did not touch semi-indexing.** The
  issue assumed decoding had to move to after structural parsing. It did not: both callers
  already gate on a whole-input SIMD `validate_utf8`, so a valid document never enters the
  repair at all, and the repair still produces a valid-UTF-8 buffer — preserving the
  invariant ([docs/plan/decode-failure-routing.md](../../plan/decode-failure-routing.md))
  that after the input-boundary pass, `as_str()` can only fail on an *escape* problem,
  which is what lets the cursor borrow and the printer echo raw spans. Locating string
  boundaries in non-UTF-8 text needs only a byte-level quote/backslash scan, which is sound
  because UTF-8 is self-synchronising: `"` and `\` can never occur inside a multi-byte
  sequence, so the scan agrees with jq's own byte-oriented lexer by construction.

Likely an off-by-one in jq's own end-of-buffer lookahead rather than a designed rule; per
ADR-0018 rule 4 the correct resolution is bug-for-bug replication rather than "fixing" the
substitution into the more sensible WHATWG-consistent shape. See
[docs/plan/decode-failure-routing.md](../../plan/decode-failure-routing.md) for the fuller
substitution-mechanism history and
[#1717](https://github.com/rust-works/succinctly/issues/1717) for the algorithm fix itself.

## Conversion diagnostics beyond a single token

jq implements `tonumber` and `fromjson` by handing the string to its JSON parser, so a
failure surfaces as that parser's diagnostic. succinctly reproduces the single-token
form exactly:

```bash
$ echo '"0x10"' | sjq -c 'try tonumber catch .'
"Invalid numeric literal at EOF at line 1, column 4 (while parsing '0x10')"
```

and distinguishes it from a string that *is* valid JSON but is not a number, which jq
reports as the string itself:

```bash
$ echo '"null"' | sjq -c 'try tonumber catch .'
"string (\"null\") cannot be parsed as a number"
```

Inputs that fail *after* a complete token get a different jq diagnostic that succinctly
approximates to the EOF form:

| Input     | jq                                                     | succinctly                                                           |
|-----------|--------------------------------------------------------|----------------------------------------------------------------------|
| `"1 2"`   | `Unexpected extra JSON values (while parsing '1 2')`   | `Invalid numeric literal at EOF at line 1, column 3 (while parsing '1 2')` |
| `"1,2"`   | `Expected value before ',' at line 1, column 2 …`      | as above, with the EOF column                                        |
| `"{"`     | `Unfinished JSON term at EOF at line 1, column 1 …`    | as above                                                             |
| `"  a  "` | `Invalid numeric literal at line 1, column 4 …`        | as above, but with `at EOF` and column 5                             |

Matching these needs a position-reporting JSON parser reporting jq's exact failure
classes; the hand-rolled `parse_complete_json` in `src/jq/eval.rs` does not carry offsets.
The shapes a filter is likely to branch on (`Invalid numeric literal`, `cannot be parsed
as a number`) are exact, so this is left as a deliberate approximation.

Both builtins do require the *whole* string to be one JSON value, which is the part that
matters for the result rather than the message: `"0x10" | fromjson` errors as jq does
instead of returning `0`, and `"1 2" | fromjson` errors instead of returning `1`.

## Float literals lose their source spelling

jq's arithmetic messages echo the literal as written, because it keeps the number's
original text:

```bash
$ echo null | jq '1 / 0.0'
jq: error (at <stdin>:1): number (1) and number (0.0) cannot be divided because the divisor is zero
```

`OwnedValue::Float(0.0).to_json()` renders `0`, so succinctly says `number (0)` there.
This is the general "JSON numbers are re-rendered, not echoed" property of the evaluator
rather than anything specific to errors, and it only shows for float literals whose
shortest rendering differs from their source spelling.

## Behaviour and parser gaps

None remain open in this section's own narrative, tracked in
[`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt).
That file's check is two-sided — a probe that starts diverging without a line there fails
the build, and so does a line for a probe that starts matching — so this section cannot
silently drift from the corpus.

What used to be listed here, and what closed it:

`repeat_error_swallowed` (`repeat(if . > 3 then error("boom") else .+1 end)` on `5`) was
here: jq propagates the generator's error the first time it is raised — here, on the very
first iteration, since `5 > 3` immediately. `eval_repeat` in
[`src/jq/eval.rs`](../../../src/jq/eval.rs) discarded the error (`Err(_) => break`) instead
of propagating it, so both evaluators produced no output at all rather than erroring.
[#495](https://github.com/rust-works/succinctly/issues/495) closed it: an error with no
prior output now surfaces as `QueryResult::Error` (via the same `partial()` helper #494 used
for `while`/`foreach`/`limit`), and output already produced before the error is no longer
discarded either.

`optional_write_negative_oob` (`.[-5]? = 9` on `[1,2]`) was here: `?` only suppresses
errors raised while *collecting* a path, not the write-time bounds check on a
still-negative array index, but succinctly treated `?` as suppressing the write too, so
`.[-5]? = 9` silently left the array unchanged instead of raising.
[#498](https://github.com/rust-works/succinctly/issues/498) closed it, landing together
with [#486](https://github.com/rust-works/succinctly/issues/486) — see "Where succinctly
errors and jq does not" below for the auto-vivification gap #498 depended on.

`slice_assign_non_array` and `slice_indices_not_integers` were the last two.
[#366](https://github.com/rust-works/succinctly/issues/366) made a slice a real path
component — `{"start":s,"end":e}` now comes out of `path()` and goes into
`getpath`/`setpath`/`delpaths`, `=`, `|=` and `del()` — so both sentences have somewhere to
be raised from. See "A slice is a path component" below for what that does and does not
cover.

`index_null_key_on_object`, `index_bool_key_on_object` and `index_object_key_on_object`
were on this list too, as parser gaps: `.[null]`, `.[true]` and `.[{}]` did not parse, so
no runtime error was reached. [#360](https://github.com/rust-works/succinctly/issues/360)
made index brackets take an arbitrary expression, and all three now raise jq's
`Cannot index object with <type>`.

`setpath_on_number` was on this list too. [#359](https://github.com/rust-works/succinctly/issues/359)
fixed it: `setpath` now auto-vivifies only `null`, as jq does, and refuses to index
anything else at any depth. The `setpath_*` probes added alongside it pin the rest of that
surface — wrong-key-type on a real container, out-of-bounds negative and NaN indices, and a
non-array path argument.

## Where succinctly errors and jq does not

A probe is only admitted to the corpus if jq errors on it, so the corpus is blind to the
opposite divergence: a filter that jq answers with a value and succinctly refuses. Those
have to be recorded here.

| Filter             | Input   | jq                                  | succinctly                        |
|--------------------|---------|-------------------------------------|-----------------------------------|
| `@uri`             | `[1,2]` | `"%5B1%2C2%5D"`                     | `expected string, got array`      |
| `@base64`          | `5`     | `"NQ=="`                            | `expected string, got number`     |
| `flatten("x")`     | `[1,2]` | `[1,2]` (ignores non-integer depth) | `expected number, got non-number` |

[#929](https://github.com/rust-works/succinctly/issues/929) found these while auditing
`EvalError::type_error` wording: real jq's `@uri`/`@base64` (and every other format string
except `@csv`/`@tsv`/`@sh`) auto-`tostring`s a non-string argument before formatting rather
than refusing it outright, and `flatten`'s depth argument is silently ignored if it isn't a
number rather than validated. `@base64d` is a related but distinct case — jq *does* still
error on `5 | @base64d` (`string ("5") trailing base64 byte found`), just from attempting to
base64-decode the auto-stringified `"5"` rather than refusing the number up front, so it
isn't purely a "jq doesn't error" gap; matching it needs the same underlying
auto-`tostring`-first change `@uri`/`@base64` do, not just a different error message.

Both rows are a slice write walking a path *in place*, and the gap is the same
auto-vivification jq performs everywhere else it writes through a path: `null` grows into
whatever container the path names. Four non-slice rows used to sit above these — `.a = 1`
on `null`, `.a.b = 1` on `{}`, `.[5] = 9` and `.[5] |= 9` on `[1,2]`, all reported as
`Cannot index …`/`index N out of bounds (length M)` where jq builds or pads the container —
closed together by [#486](https://github.com/rust-works/succinctly/issues/486)
(`set_path`/`get_path_mut`/`update_path` now vivify `null` and pad past the array end, the
way `set_value_at_path` already did for `setpath()`) and
[#498](https://github.com/rust-works/succinctly/issues/498) (the write-time negative-index
bounds check now survives `?`, since padding is what makes the positive case succeed rather
than needing `?` to swallow a failure). No write operator produces `index N out of bounds
(length M)` any more; a numeric index past the end is not an error jq raises, so there is no
longer a positive case for succinctly's own wording to cover.

`del()` used to sit here too — `del(.[5])` and `del(.[-5])` on `[1,2]`, plus a missing
intermediate key or an out-of-range index — but every one of those is a silent no-op now,
matching jq, after [#477](https://github.com/rust-works/succinctly/issues/477),
[#527](https://github.com/rust-works/succinctly/issues/527) and
[#529](https://github.com/rust-works/succinctly/issues/529). A step that reaches nothing
reads as `null` and the rest of the path is walked against it, so only an `[]` tail still
raises (`{"a":{"x":1}} | del(.a.b[])` and `[1,2] | del(.[5][])` are both `Cannot iterate over
null (null)`).

The two slice rows were added by [#366](https://github.com/rust-works/succinctly/issues/366)
deliberately. Writing through a slice could have vivified `null` on its own — `setpath`
does, and the shared code was right there — but that would have left `.[1:2] = ["x"]`
growing a container while `.a = 1` beside it still refused, inside one feature; they matched
their neighbours instead. Now that #486/#498 closed the rest of this table, the slice rows
are the odd ones out rather than the ones fitting in — `.a = 1` and `.[5] = 9` vivify `null`
today, and `.[1:2] = ["x"]` still deliberately does not. See "A slice is a path component"
below for why that gap stays open.

An object key that yields something other than exactly one value is a second, unrelated
group — it is the key half of
[#354](https://github.com/rust-works/succinctly/issues/354):

| Filter        | Input       | jq                       | succinctly             |
|---------------|-------------|--------------------------|------------------------|
| `{(empty):1}` | `0`         | *(no output)*            | `key must be a string` |
| `{(.[]):1}`   | `["a","b"]` | `{"a":1}` then `{"b":1}` | `key must be a string` |

jq's object construction takes the cartesian product over each key's outputs, so a key
producing nothing produces no object and a key producing two produces two. Succinctly
evaluates the key to a single value and refuses anything else, with wording of its own —
the one sentence left in `eval_object_construction`, because reaching jq's answer here
means generating a stream, not renaming an error. The sentence stays succinctly's until
#354 is built.

`sub`/`gsub`'s replacement filter emitting 2+ outputs for one match is **no longer a
divergence** — [#840](https://github.com/rust-works/succinctly/issues/840) was closed by
#1279's generator-argument fan-out. jq forks the whole `sub`/`gsub` call, one whole-string
output per replacement value, and succinctly now does the same:

| Filter                               | Input           | jq and succinctly                     |
|--------------------------------------|-----------------|---------------------------------------|
| `sub("(?<x>[aeiou])"; (.x, .x+"!"))` | `"hello world"` | `"hello world"` then `"he!llo world"` |

The fork is a **transpose**, not a cartesian product: `gsub("(?<x>[aeiou])"; (.x, .x+"!"))`
on a three-vowel input still gives 2 outputs, not 2³, because row *k* takes each match's
*k*-th replacement value. Uneven lists are padded by *absence* rather than `null` — a match
with no *k*-th value contributes nothing to row *k*, dropping its own preceding gap along
with its text, so `"a-b" | [gsub("(?<c>[ab])"; if .c=="a" then ("1","2") else "9" end)]` is
`["1-9","2"]` and row 1 is `"2"`, not `"2-"`. `stitch_replacement_rows` in
`src/jq/eval.rs` implements this, and non-global `sub` reaches it as the one-match case of
the same transpose.

#840 also covers a *zero*-output replacement filter (`sub("a"; empty)`), which turned out
**not** to need this treatment: re-deriving jq's own `reduce`-based definition (and
verifying empirically, jq 1.7.1) showed a simple, fully portable rule — if *every* match
in the call has an empty replacement, the whole input is returned unchanged; otherwise
each empty match drops its own text *and* its own immediately-preceding gap, while every
non-empty match is processed normally. `eval_sub_replacement` and
`stitch_replacement_rows` implement this rule
directly rather than erroring — see those functions' doc comments in `src/jq/eval.rs` for
the mechanism.

`setpath` is the same operation without the syntax, and after #359 it does follow jq —
`[1,2] | setpath([5]; 9)` is `[1,2,null,null,null,9]`, and `null | setpath(["a"]; 1)` is
`{"a":1}`. The two used to disagree with each other in-tree; [#486](https://github.com/rust-works/succinctly/issues/486)
closed that by teaching `set_path`/`get_path_mut`/`update_path` to vivify the way
`set_value_at_path` already did, so `=`/`|=`/the compound operators/`//=` now agree with
`setpath` on every one of the four shapes removed from the table earlier in this section.
`delete_at_path` deliberately keeps its own,
different mechanism — a step that reaches `null` or an out-of-range index is a no-op rather
than a container to build (#476/#477), since `del` never needs to invent structure to delete
through — so it is not a fourth function taught the same rule, it is a different rule for a
different operation.

Where the same walk *does* error in jq, the sentence matches. A still-negative index is
jq's `Out of bounds negative array index` in `=` and `|=` as well as `setpath`, pinned by
the probes `assign_negative_index_oob`, `update_negative_index_oob` and
`assign_negative_index_nested`; `del` raises the same sentence for a negative index too.
Unlike the positive case above, `?` does not suppress this one —
[#498](https://github.com/rust-works/succinctly/issues/498) — because it is a write-time
bounds check, not a failure to collect the path.

A variable bound outside `reduce`/`foreach` and referenced from inside `UPDATE`/`EXTRACT`
used to be a third, narrower gap, found reviewing
[#844](https://github.com/rust-works/succinctly/issues/844) — closed by
[#1440](https://github.com/rust-works/succinctly/issues/1440), which added
`resolve_reduce`/`resolve_foreach` arms to `resolve_node` (`src/jq/eval.rs`) modeling jq's
own `(path, value_at_path)` register, derived empirically since real jq has no fold-specific
path machinery at all (`reduce`/`foreach` are sugar over the same variable-binding primitive
every other construct uses). Three narrower shapes remain open, **all of them refuse-only**
— succinctly declines a filter jq accepts, visibly (exit 5) and without touching any
document. There used to be a fourth that ran the other way, accepting a filter jq rejects so
that `=`/`|=`/`del()` wrote where jq raises;
[#1466](https://github.com/rust-works/succinctly/issues/1466) closed it, and shape #3 below
is the price it paid. Refusing is the safe direction: [#985](https://github.com/rust-works/succinctly/issues/985)
is the revert that established what the other one costs.

1. **Variable-rooted navigation off a mismatched accumulator** —
   `path(. as $x \| foreach (1,2) as $i (0; $x.a; .b))` on `{"a":{"b":1},"c":2}` is
   `["a","b"]` ×2 in jq; succinctly refuses.

   The *plain-pipe* half of this shape is closed.
   [#1573](https://github.com/rust-works/succinctly/issues/1573) established that jq carries
   a `(path, value_at_path)` **register** which only navigation advances — a literal never
   moves it, so a `$var` frozen from where it still points steps back onto it — and
   `resolve_seq` now threads that register (`PathBranch::register`,
   `reestablishes_register`, `src/jq/eval.rs`). `path(. as $x \| 5 \| $x.a)` on `{"a":1}`,
   which this list previously recorded as refusing, answers `["a"]` like jq.

   What remains here is that [`FoldRegister`](../../../src/jq/eval.rs) does not seed *its*
   register into the resolution of its own `UPDATE`/`EXTRACT`: the accumulator (`0`) is
   resolved with tracking already off and no register to compare against, so `$x.a` cannot
   re-establish the way it does in a plain pipe. Closing it means threading the register
   into `resolve_node`/`resolve_leaf` as a parameter rather than carrying it on the branch,
   which is a materially larger change across that function's ~20-call-site dispatch graph.

   The register is also carried **only across stages this resolver can prove did not move
   it** (`cannot_move_register`, `src/jq/eval.rs`). jq's register advances on any `INDEX`
   its own bytecode executes, which includes the ones hidden inside a jq-*defined* builtin
   (`first` is `.[0]`, `add` is `reduce .[] as $x ...`) or a user function body — stages
   that reach the resolver as one opaque computed value. Assuming those left the register
   alone made `path(. as $x \| ([.a]\|first) \| $x)` answer `[]` where jq refuses, and
   `=`/`|=`/`del()` then wrote through the fabricated path, so the allowlist is deliberately
   narrow and everything outside it drops the register. Three shapes jq answers therefore
   refuse here: a construction or interpolation that itself navigates
   (`path(. as $x \| [.a] \| $x)`, `{k:.a}`, `"\(.a)"` — excluded because jq *also* raises
   for a `.a` applied to a computed value, which this resolver never sees inside a
   construction: `path(. as $x \| {k:.a} \| [.a] \| $x)` raises in jq on the second `.a`),
   an `as` whose bind source navigates, and a `def` whose body is a constant
   (`path(. as $x \| (def f: 5; f) \| $x)` — resolving a call to its body is not something a
   syntactic predicate can do from a name). All three are refuse-only.

   A fourth refusal is the `null`/`false` half of `register_identical` applied one stage
   earlier than succinctly reaches: `path(.a \| null)` on `{"a":null}` and `path(.a \| false)`
   on `{"a":false}` are `["a"]` in jq — the literal never moved the register, and a `null` is
   `jv_identical` to a `null` whatever node it came from — while succinctly refuses, because
   `reestablishes_register` only re-establishes a branch that is *already* untracked and a
   stage sitting directly on a trackable one never gets there. Pre-existing (it predates the
   register threading), refuse-only, and closing it is a widening that needs its own live
   matrix; tracked as [#2044](https://github.com/rust-works/succinctly/issues/2044), which
   carries the same root cause under its own repro (`path(null \| .a)` on `null`).

   Separately, a variable bound from a *navigated* position (`.a as $y`) still carries no
   marker at all, because `substitute_var_tracked` remains gated on
   `is_identity_passthrough` — so `path(.a as $y \| reduce (1) as $i (.a; $y))`, jq `["a"]`,
   refuses too. That gate **cannot simply be widened**: measured against jq 1.7.1, doing so
   makes `path(.a as $y \| .c \| $y)` on `{"a":{"b":1},"c":{"b":1}}` answer `["c"]` where jq
   refuses — reopening the accept-where-jq-refuses class #1466 closed, since
   `register_identical`'s provenance bit records "never rebuilt", not "from *this*
   position", and `OwnedValue` has no node identity. The marker needs a bind-time path
   first; tracked as [#2042](https://github.com/rust-works/succinctly/issues/2042). Golden
   `fold_register_var_from_navigated_binding`.

2. **`?//`-alternatives folds aren't path-tracked at all** (refuse-only) —
   `path(. as $x \| reduce (1) as $y ?// $z (0; $x))` on `{"a":1}` is `[]` in jq; succinctly
   refuses. [#1365](https://github.com/rust-works/succinctly/issues/1365) (`?//`-alternatives
   support for `reduce`/`foreach`'s own `as` clause) landed after `resolve_reduce`/
   `resolve_foreach` were designed, adding a retry-with-rollback matrix
   (`try_reduce_step_alternatives`/`try_foreach_step_alternatives`) neither function threads
   path-tracking through. `resolve_node`'s dispatch arm admits exactly `[Pattern::Var(_)]`
   and falls to `resolve_leaf`'s catch-all otherwise.
3. **jq's pointer-identity artifacts on `*`/`+` with an empty operand** —
   `path(. as $x \| reduce (1) as $i (0; $x + {}))` on `{"a":1}` is `[]` in jq; succinctly
   refuses (likewise `$x * {}` and `$x + null`). This is not a rule jq implements but an
   artifact of how it is written: merging an *empty* right operand returns the left
   operand's pointer unchanged, so `jv_identical` still holds. The mirrored `{} + $x`
   refuses in jq itself, which is what shows there is nothing here to model. Golden
   `fold_register_empty_merge_artifact`; not planned for closure.

`FoldRegister`'s own register check models jq's `jv_identical` since
[#1466](https://github.com/rust-works/succinctly/issues/1466), and that check is **not**
structural equality. jq compares the two `jv`s' kind, then — for a string, array or object —
their *pointers*, and for a number its raw representation, which a reconstruction never
reproduces; only `null`/`true`/`false` reach its "same kind is identical" arm. So
`path(reduce (1) as $i (.a; 1))` on `{"a":1}` raises even though `1 == 1`, while
`path(reduce (1) as $i (.a; null))` on `{"a":null}` is `["a"]`. `OwnedValue` has no pointer
to compare, so a `PathBranch` instead carries a `snapshot` mark for the one provenance
succinctly can prove — a frozen `. as $x` binding that was never rebuilt, which is the same
value jq is still holding — and `FoldRegister::identical` admits a branch that either
navigated back to the register, is such a snapshot, or is a `null`/`true`/`false` equal to
it. Before #1466 the check compared values alone, so any reconstruction landing on the
register's value was promoted: `(reduce (1) as $i (.; {a:.a})) = 9` wrote `9` where jq
raises and leaves the input untouched. That was the one divergence in this section that ran
in the unsafe direction, and it is closed.

The fold's own **loop-variable destructuring pattern** (`as [$i]`, `as {v:$v}`) is *not* a
behavioural divergence: real jq refuses every such fold in path position, even when the
pattern matches cleanly (`path(. as $x \| reduce ([1]) as [$i] (0; $x))` raises "near attempt
to access element 0 of [1]", confirmed against jq 1.7.1, while the bare-`$var` spelling of
the same fold is `[]`), and `resolve_node`'s `[Pattern::Var(_)]` guard reproduces that
refusal — same outcome, same exit 5, no document written either side. Only the **wording**
differs: falling through to `resolve_leaf`'s catch-all names the whole fold's own value
("Invalid path expression with result `{"a":1}`") rather than the destructuring step jq
blames. That is the same catch-all wording every unresolvable filter already gets here, and
is the general message-fidelity gap covered above, not a fold-specific one.

**Fixed by [#1467](https://github.com/rust-works/succinctly/issues/1467):** the fold's
**source expression** is now path-checked the same way jq checks it — `resolve_reduce`/
`resolve_foreach` route the source through `resolve_node` (discarding its branches, keeping
only the escape) before taking the actual values from the untracked evaluator, but only when
the source's own AST contains a real navigation step (`Field`/`Index`/`Slice`/`Iterate`)
anywhere — checked via `any_subexpr` — since only those can ever reach one of
`resolve_node`'s raising arms in the first place. jq evaluates the source "with tracking on"
and fails only where it navigates *through* an untrackable value, so `reduce (1,2) / (.a) /
(.[]) / (keys) / (range(2)) as $i (0; $x)` all resolve in both, and `reduce (keys[]) as $k
(.; .)` now raises "near attempt to iterate through" in both too (live-verified against jq
1.7.1, both the `path(...)` read side and the `(reduce ...) = 9` write side).

That fix has two accepted costs, both confined to a source that actually contains
navigation — a non-navigating source (`range(n)`, `keys`, a literal, or critically
`input`/`inputs`) skips the check entirely and pays neither:

1. `resolve_node`'s general, non-primitive leaf (`resolve_leaf`, #986's deliberate
   defer-not-raise case) narrows a multi-output source to its first value alone, so its
   branches can't be reused as the fold's real values — the source's first output is
   therefore evaluated *twice*, once for the path-check, once more for the real value. A
   navigating source whose first output has a side effect fires it twice where jq's single
   tracked evaluation fires it once: `. as $x | path(reduce (.[] | stderr) as $i (0; $x))`
   on `[1]` writes `1` once to stderr in jq, twice in succinctly (measured directly, not by
   line count — `stderr` writes no trailing newline). The alternative — threading a new
   "keep every output while tracking" mode through `resolve_node`'s entire recursive
   dispatch so the fold's real values could be taken from the same single evaluation — was
   judged a materially larger, more invasive change to a widely-shared traversal for a
   narrow, side-effect-only divergence, and left as a documented trade-off rather than
   attempted. The `any_subexpr` gate above was *not* part of the original fix — an earlier
   version ran this check unconditionally, which corrupted an `input`/`inputs`-sourced fold
   (not merely double-firing a cosmetic side effect): the check's own re-evaluation consumed
   one document, then `eval_owned_expr_fork`'s real evaluation consumed the *next* one,
   silently desynchronizing which document the fold actually ran against. Caught in code
   review before merging.
2. `foreach`'s own per-element streaming loses a legitimate pre-error prefix when the
   *navigating* source itself is a multi-element comma list: `path(foreach (1,2,keys[]) as
   $k (.; .))` on `{"a":1}` streams `[]` twice before raising in jq (elements `1`, `2`
   succeed; `keys[]` fails third), prints nothing in succinctly — the whole-source check runs
   to completion (or failure) before `eval_owned_expr_fork` ever gets a chance to stream
   anything. Same underlying cause as cost 1 (two separate evaluations instead of one
   interleaved one) and the same fix would close it; tracked separately as
   [#1872](https://github.com/rust-works/succinctly/issues/1872) rather than folded into
   #1467's own fix.

## Duplicate object keys collapse, except under `--preserve-input`

jq collapses a repeated object key when it *builds* the value — first position kept, last
value wins, exactly `IndexMap::insert` semantics. Since
[#1385](https://github.com/rust-works/succinctly/issues/1385) `succinctly jq` does the same
on every path, including the cursor-native ones that never materialize a value, so
`{"b":1,"a":2,"b":3}` answers `{"b":3,"a":2}` for `.`, `2` for `length`, `["a","b"]` for
`keys` and `[3,2]` for `[.[]]` — all captured from the pin. Default jq mode therefore has
**no divergence to record here**; this section exists for the one flag that opts out, and
for the one input class where "the same key" is undecidable.

**`--preserve-input` preserves duplicate keys on *output only*.** The flag is a succinctly
extension whose purpose is echoing the input's original spelling, and ADR-0018 rule 5
exempts it because it perturbs no reference-defined filter. That exemption reaches the
printer, not the evaluator:

| filter          | `sjq`           | `sjq --preserve-input` |
|-----------------|-----------------|------------------------|
| `.`             | `{"b":3,"a":2}` | `{"b":1,"a":2,"b":3}`  |
| `.x` (nested)   | `{"b":3,"a":2}` | `{"b":1,"a":2,"b":3}`  |
| `length`        | `2`             | `2`                    |
| `keys`          | `["a","b"]`     | `["a","b"]`            |
| `[.[]]`         | `[3,2]`         | `[3,2]`                |
| `to_entries`    | 2 entries       | 2 entries              |

This is the split the flag already has for numbers, not a new one: `--preserve-input`
echoes `4e4` as written while `.n + 0` still evaluates to `40000`. The flag chooses how a
value is *spelled* on the way out; it never changes what the value model *is*. Gating the
evaluator on it as well would mean a third `EvalSemantics` implementor and a third
monomorphization of the whole generic evaluator, for one CLI flag — so the boundary is
drawn at the printer deliberately, and
`test_preserve_input_duplicate_keys_are_output_only_1385` pins both halves so it cannot
drift silently.

**A malformed object member is caught when it is read, not when the document is.** The
semi-index recovers an object's members by pairing the container's parenthesis-tree children
two at a time; `:` and `,` carry the same (absent) meaning to it, so `{invalid: 1}` indexes
exactly as `{"a":1}` does, and neither the key's type nor the child count's parity is
checked while indexing. Reading such a member raises `Invalid JSON text: <the strict
validator's own reason>` and exits 5, matching jq's exit code — but only once something
actually reads it:

```
$ echo '{"ok":1,"x":{bad}}' | jq  .ok        # parse error: Invalid numeric literal, exit 5
$ echo '{"ok":1,"x":{bad}}' | sjq .ok        # 1, exit 0
$ echo '{invalid}' | jq  empty               # parse error, exit 5
$ echo '{invalid}' | sjq empty               # no output, exit 0
```

jq builds a DOM before the program runs, so it rejects the document whatever the filter is.
Reproducing that means validating every document up front, which costs roughly a second
index-building pass over the input — the whole advantage this crate is built for. `--validate`
is the opt-in form for callers who want it (exit 3, its own separately-pinned code).

**What still does not raise.** Every route that *materializes* the object now does —
`to_entries`, `keys`, `length` (in all three of its spellings, including `keys | length`),
`--slurp`, `--sort-keys`, `--ascii-output`, and any filter shape that costs the value its cursor
(a comma, an `if`) — because #1247 made the `to_owned`/`cursor_to_owned` family fallible and
this check rides along with it. Bare `.[]` and the identity printer raise now too (#1641, below).

This is `keys_unsorted`'s **positional** fast paths — `[]`, `[0]`, `[n]`, `first`, `last` — where
`#1514` and `#1599` deliberately took the whole-object probe *off* the arms that don't otherwise
need it. #1629 restored the check on every arm that already pays for a walk regardless (`[]`,
`last`, and a *negative* `[n]`, which has to know the object's length to normalize against) —
free, since the check rides the walk rather than adding one:

```
$ echo '{123: 1, "b": 2}' | sjq -c 'keys_unsorted[]'      # exit 5 now (was: 123 then "b", exit 0)
$ echo '{123: 1, "b": 2}' | sjq -c 'keys_unsorted | last' # exit 5 now (was: "b",           exit 0)
$ echo '{123: 1, "b": 2}' | sjq -c 'keys_unsorted[-1]'    # exit 5 now (was: "b",           exit 0)
```

**Still not fixed: `first`, `[0]`, and a *positive* `[n]`.** These three answer from one
`uncons_key`/a short early-exit walk (collapsing keeps every key at its first position, so a small
positive index never needs to look past it) — restoring the check here means restoring exactly the
walk #1514/#1599 removed, on the one arm shape built to avoid it entirely. #1629 left this part of
#1514/#1599's original tradeoff as it found it rather than widening its own scope to relitigate it:

```
$ echo '{123: 1, "b": 2}' | sjq -c 'keys_unsorted[0]'      # 123, exit 0 -- unfixed
$ echo '{123: 1, "b": 2}' | sjq -c 'keys_unsorted | first' # 123, exit 0 -- unfixed
$ echo '{123: 1, "b": 2}' | sjq -c 'keys_unsorted[1]'      # "b",  exit 0 -- unfixed
```

**Also not fixed: any of the four raising arms above, once wrapped in an early-exit demand
combinator** (`first(keys_unsorted[])`, `limit(1; keys_unsorted[])`). `fold_pipe_stages_sink`
routes `Expr::Iterate` through a separate demand-driven `each_lazy_keys_iterate_sink`, not
`fold_lazy_keys_stage`'s arm #1629 fixed, and that sink still streams the raw, unchecked
`DistinctKeyCursors` iterator directly:

```
$ echo '{123: 1, "b": 2}' | sjq -c 'keys_unsorted[]'              # exit 5 (fixed above)
$ echo '{123: 1, "b": 2}' | sjq -c 'first(keys_unsorted[])'       # 123, exit 0 -- unfixed
```

Found during #1629's own review. Unlike the early-exit arms above, this isn't obviously the same
tradeoff: whether an early-exit *consumer* should pay to validate object shape it may never finish
reading is a real, separate design question — the same one #725/#1565 already answered "no" to for
a `map(f)` error under `first` (`docs/compliance/jq/limitations.md`, `each_lazy_seq_iterate_sink`'s
own doc comment) — so resolving it here, one way or the other, is left to its own issue (#1770)
rather than folded into #1629's own scope.

`keys_unsorted | length` is not in this set at all — it reaches `effective_len`, which walks, and
so has carried the check for free since #1628.

**A separate catchability dimension, now fixed for both the pull model and the push model
(#1936, #1948).** Whether a malformed key *raises* (documented above) is distinct from
whether `?`/`try`/`catch` can *suppress or catch* that raise — `keys`/`keys_unsorted` stay
lazy (`GenericResult`/`GenericItem::LazyKeys`) until something actually materializes them,
so wrapping a bare `keys_unsorted` in `?` used to let the error escape past a boundary that
had already closed by the time materialization happened. #1936 fixed this for the pull-model
dispatch (`try_single_generic`, reached via plain `eval_single`) by checking for a malformed
key before the boundary's match runs, without forcing a full decode on success (preserving
`fold_pipe_stages`'s lazy fast paths for `.[]`/`.[n]`/`first`/`last`/`length`):

```
$ echo '{123: 1}' | sjq -c 'keys_unsorted?'                      # suppressed, exit 0
$ echo '{123: 1}' | sjq -c 'try (keys_unsorted) catch "c"'       # "c", exit 0
```

The push-model dispatch (`each_try_generic`, reached via `eval_each_generic` for
`first`/`limit`) had the identical bug class, plus a wider one: it also affected `LazySeq`
(`map(f)`), a gap that predated #1936 entirely, since #1812 only fixed `LazySeq`'s
pull-model catchability. #1948 closed both by wrapping `sink` itself inside
`each_try_generic` — `check_lazy_item_for_try` runs the identical `keys_are_well_formed`
check (staying lazy on success, same as `try_single_generic`) or, for `LazySeq`, the
identical `materialize_atomic` call `try_single_generic` already made, capturing any fault
via a side channel checked once the push loop returns (`Demand` has no error channel of its
own to carry it through directly):

```
$ echo '{123: 1}' | sjq -c 'first(keys_unsorted?)'                       # suppressed, exit 0
$ echo '[1,2,3]' | sjq -c 'first(try (map(error("x"))) catch "c")'       # "c", exit 0
```

**`?//`'s own push-model fallthrough decision had the identical gap** (found reviewing #1948):
`each_pattern_alternatives_generic` — the sink-based dispatch for a `?//`-chain, reached via
`first`/`limit` wrapping one — decided whether the *taken* alternative "succeeded" purely from
the `Flow` `eval_each_generic` returned, and a still-lazy item forwarded to `sink` unmaterialized
answers `Flow::Exhausted` regardless of whether it would later raise. So a malformed
`keys_unsorted`/`map(f)` tail on the taken alternative's body made this loop believe that
alternative succeeded and never try the next one — the exact boundary-closes-too-soon bug
`each_try_generic` closed for `try`/`catch`/`?`, just for `?//`'s fallthrough rule instead. Fixed
the same way, reusing `check_lazy_item_for_try` unchanged:

```
$ echo '[1,2,3]' | sjq -c 'first(. as [$a] ?// $b |
    (if $a == 1
     then (. | map(if . == 2 then error("boom") else . + 10 end))
     else (. | map(. + 100))
     end))'
[101,102,103]                                                            # falls through, exit 0
```

Bare `keys_unsorted` — no stage after it — does raise, but only part-way through: it streams,
and finds a non-string key once its `[` is already out, so a **truncated** array can reach
stdout beside the exit 5. Pre-checking would mean a second walk over every key on a path
`scripts/perf-guard.py` measures.

A malformed object *nested* inside a well-formed one is found only once its parent's opening
bytes are written, so that case truncates the same way:

```
$ echo '{invalid}'        | sjq -c .   # (nothing),  exit 5
$ echo '{"a": {invalid}}' | sjq -c .   # `{"a":`,    exit 5
```

**Bare `.[]` and the identity printer now raise too (#1641).** Both were previously misdiagnosed
(here and in the issue that fixed them) as blocked on "no error channel to raise into" — tracing
the actual code disproved that for both.

Bare `.[]`'s object arm is `Expr::Iterate` in `eval_generic.rs`, not `LazySource::Values` (that
machinery is reached only by `obj | map(f)`, a distinct construct — see below). `Expr::Iterate`
already walks every field eagerly via `effective_fields`, so there was no laziness to preserve;
`effective_fields`'s underlying `all_fields()` walk just never exposed whether it ended on an
unpaired child. `effective_fields_checked` folds the check into that same walk, the way
`effective_len_checked` already does for `length`:

```
$ echo '{invalid: 1}' | sjq -c '.[]'   # exit 5 now (was: 1,          exit 0)
$ echo '{invalid}'    | sjq -c '.[]'   # exit 5 now (was: no output, exit 0)
```

The walk is atomic — it builds the whole field list before `.[]` starts emitting — so a malformed
member anywhere in the object, including *after* a valid field, raises before any output at all:
`{"a":1, invalid} | .[]` prints nothing, not `1`.

`print_json`'s `StandardJson::Error` arm (`jq_runner.rs`) — reached by the identity path on a
structurally malformed *value* (`[xyz123]`, `[tru]`) rather than a malformed *member* — now raises
through the same `MalformedJsonError` convention the object-member check above uses. This one
**does** truncate, the same accepted trade `keys_unsorted` and a nested `{invalid}` already make
above: the writer streams child cursors as it walks, so an earlier sibling and the opening bracket
are already out by the time a later error is found:

```
$ echo '[xyz123]'  | sjq -c .   # `[`,   exit 5 now (was: [null],       exit 0)
$ echo '[1,zzz,3]' | sjq -c .   # `[1,`, exit 5 now (was: [1,null,3], exit 0)
```

This exact fix was tried once before and reverted: the earlier attempt predated
`MalformedJsonError`, so bailing surfaced as a generic exit 1 instead of jq's own exit 5 — worse
than the silent `null` it replaced. Reusing the now-established convention keeps the exit code and
diagnostic clean; the truncation itself was already the accepted trade, not a new one.

**Not fixed: `obj | map(f)` has the identical latent gap.** `{invalid: 1} | map(.)` goes through
`LazySource::Values` in `eval_generic.rs`, whose `uncons` still cannot tell "no more fields" from
"the last field never got a value" apart — the same ambiguity `effective_fields_checked` closes for
bare `.[]`. Left open by #1641, mirroring the `#1629` precedent above rather than silently
expanding that PR's scope.

`LazySource::Keys` -- `keys_unsorted | map(f)`, the sibling variant of the same `advance()`
match, sourcing from a `DistinctKeyCursors` walk rather than raw `uncons` -- is *not* this
same gap and is not left open: #1956 found it had no check at all (neither
`ended_unpaired()` nor `delimiter_fault()`, unlike every other `keys_unsorted` consumer,
which had at least one), confirmed live (`keys_unsorted | map(.)` silently succeeded where
`keys_unsorted | last` correctly raised), and fixed by threading `is_malformed()` through
`advance()`'s now-fallible return.

`--preserve-input` is not an exception to any of this: it changes how values are *rendered*
(number literals and escape sequences kept as written), not whether the document is accepted,
so a malformed member raises under it exactly as it does without it.

**A key that will not decode is never a duplicate.** succinctly semi-indexes rather than
validates, so a key carrying an invalid escape or a lone *high* surrogate — input real jq
rejects outright with `Invalid escape` / `Invalid \uXXXX\uXXXX surrogate pair escape` —
still reaches the evaluator. (A lone *low* surrogate is not this case since #2008: real jq
accepts it and substitutes U+FFFD, and so does succinctly, so such a key decodes normally
and participates in collapse like any other.) A key that genuinely won't decode has no
name to compare, so it participates in no collapse and is echoed verbatim from its source
span:

```
$ echo '{"a\q":1,"b":2,"b":3}' | jq -c  .        # parse error: Invalid escape
$ echo '{"a\q":1,"b":2,"b":3}' | sjq -c .        # {"a\q":1,"b":3}
```

`b` collapses; `a\q` survives, and `length`, `keys` and `[.[]]` all agree that the object
has two members. Dropping it instead — which is what the first cut of #1385 did — deleted
the field from output while `length` went on counting it.

`to_entries` and `has` agree too (#1642). PR #1391 (the #1247 fix) made `keys`/`to_entries`
*raise* on exactly this key instead of preserving it, without noticing that now contradicted
#1385's own rule above — so the same document gave four different answers depending on
which builtin was asked. `has` raised for an unrelated reason: it had no native handling and
fell back to fully materializing the object first, which failed on the unrelated bad key
before `has` ever got to check the one it was actually asked about. All five now agree,
pinned together in one test (`test_undecodable_key_builtins_agree_1642`) rather than
trusting each builtin's own prose to stay in sync:

```
$ echo '{"a\q":1,"b":2}' | sjq -c 'length, keys, keys_unsorted, to_entries, has("b")'
2
["a\\q","b"]
["a\\q","b"]
[{"key":"a\\q","value":1},{"key":"b","value":2}]
true
```

A comma is enough to cost `keys_unsorted` its genuinely-lazy raw-byte path (#140's
`materialize_lazy_keys`/`effective_keys` escape hatch), so every result above is a real
materialized string — `a\q`'s one source backslash doubles to `\\`, which is why `keys`
and this `keys_unsorted` agree byte-for-byte. **Bare** `keys_unsorted` (the sole top-level
filter) stays lazy and echoes the exact source bytes verbatim instead:

```
$ echo '{"a\q":1,"b":2}' | sjq -c 'keys_unsorted'
["a\q","b"]
```

Both spellings agree the key is present and the count is 2 — the literal escaping
differing between a raw-byte echo and a materialized value is an inherent property of the
two representations, not a new inconsistency #1642 introduces.

`paths` and `leaf_paths` agree with the same five now too: both were left on the
pre-#1642 `field.key_str()` (`None` for a decode-failure key, indistinguishable from
#1194's genuinely absent one) when the rest of this file's builtins were rewired onto the
shared `key_display_string` fallback, so a bad key's path silently vanished from `paths`'s
output while `keys` went on reporting it.

**One exception, on the materializing routes only** (`to_owned`/`materialize` — `-S`,
`-s`, a multi-result filter like `.,.`). Two *different* decode-failure keys can share the
same display fallback (byte-identical raw escapes, or two distinct bad `\u` escapes that
both lossy-decode to the same replacement text) — never the same key under #1385's rule
above, but a plain `IndexMap<String, _>` cannot hold two entries under one string. Silently
keeping only the last value would be *quieter* data loss than #1247's original raise, not a
fix, so `DisplayKeyGuard` ([src/jq/document.rs](../../../src/jq/document.rs)) makes this
specific collision raise instead:

```
$ echo '{"\ud800":1,"\ud800":2}' | sjq -Sc .
jq: error (at <stdin>:0): object key "\ud800" is ambiguous: an undecodable key's display
form collides with another key of the same name and cannot be represented
```

An *ordinary* repeated key (no decode failure on either side) is unaffected and still
collapses to its last value, matching jq's normal duplicate-key handling.

**A missing or doubled `,`/`:` is now caught by the same routes as an unpaired member
(#1677).** #1643 added this check (`preceding_gap_ok`) but placed it only in the CLI's own
`print_json`, so a filter that never re-serializes the malformed container whole — `.[]`,
`length`, `keys`/`keys_unsorted`, `add`, `to_entries`, or a plain field lookup that doesn't
reach a leaf — read straight through it:

```
$ echo '{"a" 1, "b": 2}' | jq  -c 'keys'    # parse error, exit 5
$ echo '{"a" 1, "b": 2}' | sjq -c 'keys'    # exit 5 now (was: ["a","b"], exit 0)
$ echo '[1 2, 3]'        | sjq -c '.[]'     # exit 5 now (was: 1␊2␊3,     exit 0)
$ echo '{"a" 1}'         | sjq -c '.a'      # exit 5 now (was: 1,         exit 0)
```

#1677 threaded the same `,`/`:` scan (relocated to `succinctly::json::light::preceding_gap_ok`,
shared with the CLI printer rather than duplicated) into the object/array walk primitives
`eval_generic.rs`/`document.rs` already share for the #1194 class above —
`effective_fields_checked`, `census`/`checked_len`, `DistinctKeyCursors`, `to_owned`/
`to_owned_cursor`, and `JsonFields::find_cursor` — so a `succinctly::jq::eval_generic` caller
gets the same protection a CLI user does, not just `sjq -c .`. The residual gaps are exactly
the ones already named above for the unpaired-member class, since both checks now ride the
same walks: `obj | map(f)` (`LazySource::Values`, left open by #1641), `keys_unsorted`'s
still-deliberately-unchecked positional fast paths (`.[0]`, `first`, `.[n]`, tracked
separately as #1629 -- `last` is no longer one of these, see below), and bare
`keys_unsorted`'s streaming truncation (a partial array can reach stdout beside the exit 5,
for the same "cannot rewind a byte-at-a-time writer" reason).

**Partly covered: `succinctly::jq::eval`'s own separate evaluator.** `src/jq/eval.rs` defines
a second, independent `pub fn eval` — the function `succinctly::jq::eval` actually re-exports,
and the one used in `src/jq/mod.rs`'s own module-doc example — with its own separate, unchecked
`to_owned`/`effective_len`/`effective_fields`. Two different checks are in play, and their
coverage differs:

- **The #1194/#1642 decode-failure and structural-key policy** already reaches most of this
  file's materializing builtins via `to_owned_checked`/`to_owned_checked_at_depth` (`in(xs)`,
  `ltrimstr`/`rtrimstr`, the sort family, `min`/`max`/`unique`/`group_by`, `add`, `join`,
  `flatten`, and every assignment RHS, among others) — not the wholesale gap the previous
  revision of this paragraph implied.
- **#1902 widened this to `. as $var`/`reduce`/`foreach`.** Their bound/INIT/input value and
  body-output conversions switched from the unchecked `to_owned`/`promote_borrowed` to
  `to_owned_checked`/`promote_borrowed_checked` — so a #1194 malformed member or #1642
  collision key that used to be silently dropped (`reduce . as $x (0; .+1)` on `{"a":1,"b"}`
  used to succeed with `1`) now raises there too, same as the builtins listed above. Not a new
  divergence unique to #1902: this is `to_owned_checked`'s own established contract from
  #1755 onward, at three call sites this one just hadn't named yet (documented here per
  #1934 item 6, which also closed a related gap: five bare, non-`to_owned_checked`
  `Error`/`Partial` arms across these same three functions' `input`/INIT streams didn't
  exclude a genuine decode failure from `optional`'s suppression the way `finish_fork`
  already does — an internal-consistency fix, not a further widening of this policy).
- **The #1677 malformed-`,`/`:`-delimiter check is the narrower gap.**
  `to_owned_checked_at_depth` itself never calls `key_delimiter_ok`/`value_delimiter_ok`, so
  every builtin routed through it still misses this one check. `Builtin::Keys` (`keys`/
  `keys_unsorted`, #1829/#1835) and `to_entries` (#1829) are the exceptions so far: `keys`/
  `keys_unsorted` delegate to `effective_keys` (`document.rs`) — the same `DistinctKeyCursors`
  walk `eval_generic.rs`'s own `keys_unsorted` writer is built on — and `to_entries` to
  `effective_fields_checked` (the value-carrying sibling of the same shared walk family), which
  gives both #1677 protection alongside #1194/#1642's.

A library caller who follows the documented `eval()` example and evaluates straight off a
fresh cursor gets #1677 protection only from `keys`/`keys_unsorted`/`to_entries`; every other
builtin in this file, checked or not on the decode-failure/#1194 axis, still misses it.

**`keys_unsorted`'s positional fast paths split three ways.** `.[0]`/`first`/`.[n]` (a
positive index) stay *deliberately* unchecked -- they are the arms built to answer in O(1)
without walking the rest of the object, and adding this check would cost strictly more than
the answer itself (#1629's own accounting of that tradeoff, "Option 1"). `last` is not one
of these: it already walks the whole object regardless (there is no way to find "the last
field" without reaching the end), so #1956 folded this same check into that arm too, at no
extra cost -- `keys_unsorted | last` on a malformed `,`/`:` delimiter now raises the same as
`keys`/`keys_unsorted` themselves.

**#1829 closed the remaining gap** (`map_values` via #1835/#1848/#1854; `with_entries`
confirmed to inherit `to_entries`'s fix for free, needing no separate change; `paths`,
`leaf_paths`, and the `pick`/`omit` pair via #1862) -- but not uniformly through this file's own
`to_owned`/`effective_fields` family. `map_values`/`pick`/`omit` did route through this file's
own `effective_fields_checked`, matching the paragraph above. `paths`/`leaf_paths` did not: their
*actual* fix landed in `eval_generic.rs`'s `collect_paths_generic`, the CLI's own native
`Builtin::Paths`/`Builtin::LeafPaths` dispatch -- **the "CLI unaffected" claim this paragraph
used to make here was wrong for those two.** `sjq`/`syq paths`/`leaf_paths` do *not* reach this
file at all; they bypass the reindex bridge entirely via their own `eval_generic.rs` arm, which
carried the identical #1194 silent-drop bug independently (confirmed live against a built
release binary: `printf '{"a":1,"b"}' | succinctly jq paths` returned `["a"]` at exit 0 even
after this file's own `builtin_paths`/`builtin_leaf_paths` were fixed, until `collect_paths_generic`
was fixed too). `pick`/`omit` have no native `eval_generic.rs` arm, so the "CLI unaffected" claim
does hold for them -- the reindex bridge they fall through to already fed them an
already-validated `OwnedValue` before this file's own fix, making that fix real for the library
API but largely redundant for `sjq`/`syq`.

**The durable lesson**: a unit test against `succinctly::jq::eval` (this file's own entry point)
proves a fix reaches library callers, never that it reaches the CLI -- only a build-and-run
check against the actual binary proves that, and is worth doing for any future builtin that has
(or might grow) its own native `eval_generic.rs` implementation rather than falling through to
this file via the bridge.

## Refusing an allocation jq does not survive

`setpath` takes its array index from the document, so the array it pads is sized by the
input: `null | setpath([1e30]; 9)` asks for 9.2e18 elements. jq dies on that filter (it is
killed on the allocation, with no message to reproduce), and succinctly used to panic with
`capacity overflow`, which for a library means taking the embedder's process down. It now
refuses with `Cannot grow array to <n> elements` — succinctly's own wording, since there is
no jq sentence to copy. Only the impossible is refused; every length that fits in memory
still pads, so `[1,2] | setpath([5]; 9)` still agrees with jq.

String repetition (`s * n`) has the identical shape (#1612): `n` comes from the document, so
`"ab" * 1e30` asks for a byte length `String::repeat` cannot even represent, which used to
panic with `capacity overflow` rather than the `EvalError` `setpath`'s own case above already
gets. Confirmed live, jq itself does not error on this filter at all — it just keeps running
rather than answering promptly (never observed to terminate one way or the other; there is no
jq sentence to reproduce because no output was ever captured to reproduce). succinctly now
refuses with `Cannot repeat string to <n> bytes` — succinctly's own wording — once the
requested allocation cannot actually be made, checked via `String::try_reserve_exact` rather
than the infallible `String::repeat` (the same technique the `setpath` case above already
uses): this covers both an unrepresentable byte length (past `isize::MAX`) and a
representable-but-genuinely-unallocatable one alike, in one guard. Every length that fits
still repeats, so `"ab" * 3` and #1230's float-count cases are unaffected.

This guard is jq-mode-only. See [yq Limitations](../yq/limitations.md) for `succinctly yq`
mode, which refuses much earlier via its own, separate cap.

Computed-index/-slice expansion (`.[$keys]`, `.[$s:$e]`, both in value position and under
`path()`) has the identical shape again, at seven call sites (#1634): five in
[src/jq/eval.rs](../../../src/jq/eval.rs) (`eval_index_expr` ×2 arms, `eval_slice_expr`,
`resolve_index_expr`, `resolve_slice_expr`) plus two more in
[src/jq/eval_generic.rs](../../../src/jq/eval_generic.rs)'s own independent
`eval_index_expr`/`eval_slice_expr` — the latter two are what a real `succinctly jq`/
`succinctly yq` CLI invocation actually dispatches an ordinary `.[$keys]`/`.[$s:$e]` read
through (the `eval.rs` siblings are reached only via the direct library API, or via
`eval_generic.rs`'s own fallback for expressions it doesn't handle natively). Each site
pre-sizes its output with a product of two or three independent, generator-controlled
`Vec::len()`s (e.g. `keys.len() * targets.len()`), previously handed straight to an
infallible `Vec::with_capacity`. A large enough cross product — e.g. two independent
100,000-element generators feeding the same `.[$keys]` — asks for more elements than the
allocator can satisfy even though neither input list is individually unreasonable to
materialize. succinctly now refuses with `Cannot allocate <factors joined by " * "> elements
for a computed-index expansion` via the same `Vec::try_reserve_exact` technique as the
`setpath`/string-repeat cases above, applied through a shared `try_reserve_product` helper.
Confirmed live against the pinned jq 1.7.1 binary for this exact shape (not just analogized
from the string-repeat case above): a genuinely large-but-indexable cross product
(`[range(50000) | [1,2]] | .[][(range(50000))]`) neither errors nor crashes — jq streams
results one at a time instead of pre-allocating a single buffer, so it just keeps producing
output rather than answering promptly or refusing.

Unlike `s * n` above, this is symmetric across both modes rather than a yq-specific
divergence to record — but not because a live check for a yq-side cap came back empty.
Real yq v4.53.3's lexer rejects `range` outright (confirmed live, `range(5)` →
`lexer: invalid input text` — matching the `--jq-extensions`-gated builtin table above), so
there is no *generator*-driven way to construct a comparably large cross product in real
yq's own grammar at all: the query shape needed to test this doesn't exist there, as
opposed to existing and having been tested clean. A large *document-sourced* array of
literal keys could in principle still reach comparable scale in yq; that variant wasn't
pursued within this issue's scope. Either way, the guard converts a host-process crash into
a catchable error uniformly in both modes, with no cap-specific divergence to record in
[yq Limitations](../yq/limitations.md).

`combinations`/`combinations(n)` (#1669) have the identical shape a third time, at three
independent sites: `builtin_combinations_n`'s own `n`-sized bookkeeping (both the `indices`
array and, independently, each combination row it builds — `checked_combinations_len`'s
`base <= 1` short-circuit makes the output *count* permanently `1` regardless of `n`, so
that alone never bounds the row width), `cartesian_product`'s own row width
(`arrays.len()`, unbounded by its length-*product* guard whenever many factor arrays are
length 1), and the actual combinatorial output count (`base_array.len().pow(n)` for
`combinations(n)`, or the product of every input array's length for bare `combinations`).
Confirmed live before this fix: `[1] | combinations(288230376151711744)` aborted the
process (SIGABRT), not a catchable error. succinctly now refuses with a `Cannot allocate
...` message via the same `Vec::try_reserve_exact` technique as the cases above, whichever
site first detects the excess.

The two arities diverge from real jq differently. Bare `combinations` matches the
`.[$keys]` cross-product case above exactly: confirmed live against the pinned jq 1.7.1
binary, `[range(100000)] as $a | first([$a,$a,$a,$a,$a] | combinations)` returns instantly
rather than erroring or hanging — jq streams results one at a time instead of
pre-allocating, so succinctly's eager refusal is a real behavioural divergence for this
arity, not just a difference in wording. `combinations(n)` does not get the same lazy
treatment in jq's own standard-library definition (`def combinations(n): . as $dot |
[range(n)] | map($dot) | combinations;`): `[range(n)]` eagerly materializes an
`n`-element array *before* the lazy recursive `combinations` is ever reached, so jq itself
pays the same eager cost succinctly does — confirmed live, `[1] | first(combinations(
288230376151711744))` does not return within 6 seconds against the pinned jq 1.7.1 binary
either, rather than the instant response the bare-`combinations` case gets. succinctly's
guard makes this fail fast with a catchable error instead of hanging (and eventually
exhausting memory) the way jq's own definition does — an improvement in kind, not just a
faster failure, but still the "would take the host process down" exception ADR-0018
carves out, since jq's own hang is exactly the failure mode being prevented, just reached
by resource exhaustion rather than a clean abort. Every combination count that fits in
memory is still produced in full for both arities, so ordinary uses of both builtins are
unaffected. This guard is symmetric across jq and yq mode (yq reaches it only behind
`--jq-extensions`, per #1650), with no additional yq-specific cap to record in
[yq Limitations](../yq/limitations.md).

`combinations(n)` has a fourth, independent overflow site (#1720), found reviewing #1669's
own fix: a multi-output `n` expression (e.g. `combinations((a, b, c))`) sums each output's
arity into a running `usize` total *before* any of the guards above ever run, and that sum
itself can overflow with as few as two large outputs — `[1] |
combinations((9223372036854775807, 9223372036854775807, 3))` aborted a debug build
(`attempt to add with overflow`) and silently wrapped to a wrong answer in release, neither
of which the allocation guards above touch, since they all assume an already-summed,
already-valid `n`. Real jq hangs on this exact shape rather than erroring (confirmed live,
`timeout 10 jq -c 'combinations((9223372036854775807, 9223372036854775807, 3))' <<< '[1]'`
against the pinned jq 1.7.1 binary), so refusing with a catchable error is the same
"would take the host process down" exception as the rest of this entry, not a new kind of
divergence.

## A too-deeply-nested document is caught at a different stage, with different wording

Real jq refuses a document nested past 256 levels at *parse* time: `jq -c sort` on a
260-level-deep array gives `jq: parse error: Exceeds depth limit for parsing at line 1,
column 257` (exit 5) before evaluation ever starts, regardless of which filter is applied
— confirmed live against the pinned 1.7.1 binary. Succinctly's semi-indexing accepts the
same document at parse/index time (its own architectural point, #1793's own investigation
confirmed: `succinctly jq -c length`/`.[1]` on the identical document already succeed,
since navigating to or measuring a value doesn't require materializing it), so the
equivalent guard only fires later, when a builtin with no native lazy fast path (`sort`,
`join`) or one that still has to materialize its result before printing (`map(.)`) forces
a full `to_owned_cursor` conversion of the value it's handed — `nesting depth exceeds
limit of 256` (also exit 5, matching jq's own exit code by coincidence of both picking the
same conventional "filter failed" code, not by design — an internal architectural ceiling
being reported through the same channel as an ordinary filter/type error, deliberately,
so a `sort`/`join`/`map(.)` result is never distinguishable-by-exit-code from any other
uncaught `EvalError` on this path). succinctly's own wording, since there is no equivalent
jq sentence to copy for an evaluation-time guard jq has no counterpart to (its own check
never gets this far). Falls under ADR-0018's "would take the host process down" exception
— the pre-fix behavior was an uncaught panic, not merely different wording. See #1793 for
the fix that turned this from an uncaught process panic into a clean, catchable
diagnostic.

#1793's own fix was scoped to the CLI's default (lazy) per-document dispatch, matching its
own repro. #1818 closed the identical gap on the CLI's *other* top-level branch — a CLI
flag that forces whole-batch materialization up front (`--slurp`, `-S`/`--sort-keys`,
`-C`/`--color-output`, `--ascii-output`, `--slurpfile`), or a filter using
`input`/`inputs`/`input_line_number`, which routes to that same materializing branch with
*no flag at all* — via `validate_json_delimiters`'s own checked guard
(`check_nesting_depth`, `src/jq/eval_generic.rs`) rather than `catch_unwind`, since that
walk already threads a `Result` and runs before any user filter evaluates at all.

Still uncaught, tracked separately, not fixed by either #1793 or #1818: `-e`/
`--exit-status`'s own separate materializer (`src/jq/lazy.rs`, already pinned uncaught by
`test_exit_status_query_rejects_adversarial_nesting_998`); and `succinctly yq`, which has
no equivalent guard on either its default or materializing path at all (#1817). Confirmed
live, also pre-existing and unrelated to either fix: `print_json`'s own guard can flush
corrupted/truncated JSON to stdout before it fires (#1819).

## Regex flags `l` and `n`

[ADR-0019](../../adrs/adr-0019.md) accepted two regex-flag gaps as permanent — rule 4(d)
of [ADR-0018](../../adrs/adr-0018.md): no dependency in the `regex`/`regex-automata` stack
expresses oniguruma's search policies, and of the alternatives evaluated, the ones that do
(`onig`, `pcre2`) cost more than closing #920/#922 is worth (both are C FFI, breaking pure-
`cargo build` portability), while the one that doesn't cost that (`fancy-regex`) closes
neither gap at all. `l` (POSIX leftmost-longest) is accepted as valid flag syntax but has
no effect; `n` (suppress empty matches) still misses a non-empty match reachable only by
backtracking to a different alternative — lazy quantifiers (`a*?`) and alternations with an
empty-matching branch listed first (`(?:|a)`).

Both gaps route through `first_captures`/`global_captures`/`next_match_step`
(`src/jq/eval.rs`), the one choke point every regex builtin shares, so the divergence is
not confined to `match`/`test` — it reaches the string-transformation builtins too, and
there it is not a wrong-answer-with-a-clear-error case, but a silent no-op or wrong-span
replacement:

```console
$ echo '"aaa"'  | jq            -c 'match("a|aa|aaa";"l").string'   =>  "aaa"
$ echo '"aaa"'  | succinctly jq -c 'match("a|aa|aaa";"l").string'   =>  "a"
$ echo '"aaa"'  | jq            -c 'sub("a|aa|aaa";"X";"l")'        =>  "X"
$ echo '"aaa"'  | succinctly jq -c 'sub("a|aa|aaa";"X";"l")'        =>  "Xaa"   # wrong span
$ echo '"xaab"' | jq            -c 'gsub("a*?";"X";"gn")'           =>  "xXXb"
$ echo '"xaab"' | succinctly jq -c 'gsub("a*?";"X";"gn")'           =>  "xaab"  # no-op
$ echo '"xaab"' | jq            -c 'sub("a*?";"X";"n")'             =>  "xXab"
$ echo '"xaab"' | succinctly jq -c 'sub("a*?";"X";"n")'             =>  "xaab"  # no-op
```

`split`/`splits` inherit the same two gaps, since both are built on the same match
discovery as `sub`/`gsub`. Every probe above exits 0 with empty stderr — silently wrong
output, not a wrong-error-wording case. The flag combinations that trigger it are narrow:
greedy quantifiers and non-empty-first alternation are unaffected and agree with jq —
`[scan("a*";"gn")]` on `"xaab"` is `["aa"]` in both, and `[match("(?:a|)";"gn").string]` on
`"xa"` is `["a"]` in both.

Like [Where succinctly errors and jq does not](#where-succinctly-errors-and-jq-does-not)
above, this divergence is **not** tracked by
[`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt) —
that corpus pins probes where jq *errors* and succinctly does not, so it is blind to any
shape where jq itself doesn't error. That section's shape is succinctly erroring where jq
returns a value; this one is a third shape neither of the two error-corpus-driven sections
covers — both jq and succinctly return a value, exit 0, with empty stderr, and the values
just disagree. Both sections are hand-maintained for the same reason: no error message
exists for the two-sided check to pin, the same caveat
[ADR-0018](../../adrs/adr-0018.md)'s Consequences already record for
[yq Limitations](../yq/limitations.md).

## A slice is a path component

jq models `.[1:2]` as indexing with `{"start":1,"end":2}`, and treats that object as a
first-class path component: it comes out of `path()`, goes into `getpath`/`setpath`/
`delpaths`, and drives `=`, `|=` and `del()`.
[#366](https://github.com/rust-works/succinctly/issues/366) built that, closing
[#469](https://github.com/rust-works/succinctly/issues/469) with it. Before it,
`path(.[1:2])` answered `[1]` — one path per element, a wrong answer rather than a refusal,
inherited by everything built on `path()` — while `setpath` and `delpaths` silently left
the value alone.

The bounds a path carries are the ones written, not the ones resolved: `[1,2,3] |
path(.[-2:-1])` is `[{"start":-2,"end":-1}]`, because a path is only resolved against a
container when it is applied. Resolution then follows jq's `parse_slice` — floor the start,
ceil the end (`.[1.7:2.9]` on five elements is `[2,3]`, reachable through a runtime
descriptor even though the parser only folds integer literals), fold negatives against the
length, clamp, and pull `end` up to `start` if they crossed. That last step matters only
when writing, where crossed bounds are an insertion point: `[1,2,3] |
setpath([{"start":2,"end":1}]; ["x"])` is `[1,2,"x",3]`.

Deleting is the case worth knowing about. Every key naming an element of one array is
resolved against the length it had on entry and removed in a single pass, so overlapping
ranges union rather than compound — `[1,2,3,4] | del(.[0:2], .[1:3])` is `[4]`, and a slice
naming the same element as a bare index deletes it once
(`delpaths([[1],[{"start":1,"end":2}]])` is `[1,3,4]`). Doing the deletions one at a time
would resolve the second range against an already-shortened array.

`indices`/`index`/`rindex` came with it, since jq defines all three over `.[$i]` and so an
object pattern is the slice rather than a search: `"abcabc" | indices({"start":1,"end":2})`
is the substring `"b"`, and `index`/`rindex` are `.[0]`/`.[-1:][0]` of that, which is why
they report `Cannot index string with number`.

Two sentences arrived with the feature — `A slice of an array can only be assigned another
array` and `Array/string slice indices must be integers` — and both were already pinned as
probes, so the two-sided manifest check forced them to start matching in the same change.

What #366 did *not* build: computed bounds. `.[$a:$b]` is still a parse error, because the
parser folds slice bounds to integer literals; see
[docs/reference/jq-language.md](../../reference/jq-language.md). Writing through a slice
against `null` used to raise `Cannot index null with object` instead of auto-vivifying —
[#1340](https://github.com/rust-works/succinctly/issues/1340) brought that in line with
jq's own `setpath()` behavior, and [#1873](https://github.com/rust-works/succinctly/issues/1873)
later fixed a gap in that same auto-vivification for a slice with more path *after* it
(`.a[0:1][]? = 9` on a missing `.a` now no-ops instead of raising a write-time error, matching
jq).

## Reading a path is indexing

`path(f)` used to walk `f` through its own copy of jq's indexing rules, and that copy
disagreed with the value path in four ways at once — all fixed by
[#489](https://github.com/rust-works/succinctly/issues/489), which replaced both walkers
with one that asks the value evaluator for every step's verdict:

| Filter          | Input     | was       | jq, and now       |
|-----------------|-----------|-----------|-------------------|
| `[path(empty)]` | `{"a":1}` | `[[]]`    | `[]`              |
| `[path(.a?)]`   | `"s"`     | `[["a"]]` | `[]`              |
| `[path(.b.c)]`  | `{"a":1}` | `[[]]`    | `[["b","c"]]`     |
| `[path(.a)]`    | `"s"`     | `[["a"]]` | the sentence below |

The first row was the severe one: `[]` is a real answer — it is what `path(.)` returns —
and the one path that always resolves, so rendering "no paths at all" as it aimed a
caller's `getpath`/`setpath`/`delpaths` at the document root.

Three rules, and none of them lives here any more:

- **A step that reads `null` keeps its component.** A missing key, an out-of-range index
  and any step through `null` all read as `null`, and `null` accepts a further step — so
  the path exists even though nothing is stored along it. That is what `setpath`'s
  auto-vivification consumes.
- **A step that cannot index its value raises jq's sentence.** The eight `path_*` probes in
  the corpus are the `index_*`/`iterate_*` rows above wrapped in `path(...)`, and they
  report identically because the wording comes from the same place.
- **`?` suppresses that error and nothing else.** A pruned step names no path (it never
  happened), while a step that read `null` never errored and so is untouched by `?`.

What remained after that — [#483](https://github.com/rust-works/succinctly/issues/483) and
[#530](https://github.com/rust-works/succinctly/issues/530) — was the walker's catch-all
conflating two different questions. `resolve_node` (the pre-pass that turns a computed key
or a control-flow shape into concrete `Field`/`Index`/`Slice` components before the walker
ever runs) now has arms for every shape jq treats as path-capable: `..`, `recurse(f)`,
`recurse(f; cond)`, `select(f)`, the typeof filters, `first(f)`, `if/then/else`, `//`,
`limit(n; f)`, `try/catch`, `label $x | ...`, `E as $x | body`, and `getpath([...])` with a
literal array argument. `needs_path_prepass` — the gate deciding whether that pre-pass runs
at all — was rewritten from a whitelist of "known complex shapes" (which is what let
`resolve_node` grow support for `..`/`recurse`/`select`/the typeof filters without the gate
ever routing to it) to an exclusion check: everything needs the pre-pass except the bare
primitives the walker already handles natively.

Whatever is left over — a value-producing filter that is not a path expression at all, like
`1`, `length`, `keys`, `.a + 1` or `{a:1}` — now raises `Invalid path expression with result
<v>` (`EvalError::invalid_path_expression`, the `path_non_path_*` probes), matching jq by
name rather than answering `[]`. Confirmed live: `?` does not suppress it (`path(("a")?)`
still raises in jq, because this is a statement about the filter, not a value error raised
while collecting a path), so neither does this resolver's — the one call site is
`resolve_node`'s bare-`?` arm and `Expr::Try`'s, both checking
`EvalError::is_invalid_path_expression`.

A *multi-output* non-path leaf used bare (`range(3)` with nothing consuming its outputs as a
further computed index) also raises `invalid_path_expression`, naming the first output —
matching real jq's own per-output check, which raises on that first output alone and never
even learns whether a second one would have existed (#891; before that fix this reported a
bespoke "Cannot use a computed index after a multi-output path component" instead, the same
`test_unsupported_path_prefixes_report_rather_than_misfire` boundary #412 drew). Every
`path_non_path_*` probe is single-output, matching #530's own repro list, but the multi-output
shape now shares the same code path and message. One case stays out of scope: the same value
used as an assignment *target* for further indexing (`(range(3) | .[.k]) = 9`) gets this
"with result" wording too, where jq instead uses its "near attempt to access element ... of
..." phrasing — `resolve_leaf` has no way to tell that context apart from `path(...)`'s own
leaf today (#989).

A closely related gap surfaced while verifying this: `path()` used to discard outputs
already streamed before a later sibling errors (`path(.a, 1)` produced nothing at all,
where jq prints `["a"]` then raises). `resolve_node` and friends now carry that prefix
alongside a resolve-time failure, and `builtin_path` reports it as a partial result
(`QueryResult::Partial`) instead of a bare error — confirmed against jq for both the
uncaught case and `?`/`try` suppressing just the trailing error while keeping the prefix.
This does not extend to `=`/`|=`/`del()`: jq's write-side path resolution is atomic
(`(.a, 1) = 5` produces no output at all in jq), so those three still discard any partial
prefix exactly as before.

## Undefined functions and arity mismatches — closed by #1473

Real jq resolves every function call at compile time, before any input is read: a call to
an undefined function, or an undefined arity of an existing one, fails immediately and
unconditionally — `jq: error: f/2 is not defined at <top-level>, line 1: ...`, exit **3**
— regardless of whether that call site is ever reached at runtime, and it cannot be
caught by `try`/`catch` or `?` (compilation fails before evaluation, and thus before
`try`, ever begins).

`succinctly jq` resolved user-defined `def` calls only through `expand_func_calls`'s static
AST substitution (`src/jq/eval.rs`), which runs *during* evaluation, so an unresolvable call
became an `Expr::Error` node discovered lazily. That diverged four ways: an unreached branch
never errored; `try`/`?` swallowed the error; the exit code was 5, not 3; and two shapes of
*forward reference* silently computed a value, because substitution has no notion of lexical
position — a body substituted into a later call site is indistinguishable from a call
genuinely written there, so a later `def`'s own expansion pass resolves it.

[#1473](https://github.com/rust-works/succinctly/issues/1473) closed all four with
`jq::resolve_func_calls` (`src/jq/resolve.rs`), a scope-aware pass the runners call before
evaluation begins. It is a *check*, not a new resolution mechanism: `src/jq/eval.rs` has
exactly one evaluation arm for `Expr::FuncCall` and it always errors, so a residual call was
already an error — the pass only moves the error earlier and extends it to the cases jq also
rejects. `expand_func_calls`'s substitution model is unchanged; the programs it mis-resolves
are now rejected before it runs.

Two deliberate remainders:

- **The reported line is located by searching the filter source for the offending
  identifier**, since `Expr::FuncCall` carries no source position. A filter mentioning the
  same undefined name more than once cites the first occurrence's line, and only the first
  unresolvable call is reported (`jq: 1 compile error`) where jq reports all of them. Adding
  a position to the AST would perturb `format!("{body:?}").len()`, which #1381's
  `MAX_FUNC_EXPANSION_WEIGHTED_COST` is calibrated against.
- **A call reached through an `include`d module or `~/.jq` reports no location at all.** It
  has no occurrence in the filter source to locate, so the line marker and source echo are
  dropped rather than a position invented: `nosuchfn/0 is not defined at <top-level>` where
  jq says `... is not defined at /path/mymod.jq, line 1:` and echoes the module's own line.
  Name, arity and exit code match. Naming the file would additionally need the originating
  module threaded through `ModuleLoader`.
- **jq's trailing padding on the echoed source line is not reproduced exactly.** jq pads with
  a `%*s` whose width follows the failing node's start column for a simple undefined name but
  points elsewhere for an arity mismatch; succinctly reproduces the column rule. It is
  trailing whitespace either way.

The ~45 jq builtins succinctly does not implement (the libm family, `JOIN`, `format/1`,
`input_filename`, …) are exempt from the pass via a roster captured from the pinned oracle
(`tests/data/jq-builtin-names.txt`, regenerated by `./scripts/sync-jq-builtin-names.sh`):
real jq compiles a mention of one, so rejecting it would be a regression. A *reached* call to
one still fails at runtime as before.

`succinctly yq` runs the same pass, keeping yq's uniform `Error: …` wording and exit 1. Real
yq has no `def` at all — its lexer rejects `def f: 42; f` outright — so succinctly's `def`
support there is an extension (ADR-0018 rule 5) rather than a behaviour with a reference to
match.

## `input`/`inputs` residuals after #1309

[#1309](https://github.com/rust-works/succinctly/issues/1309) closed four of the five gaps
#723's implementation left behind: `-L`/`import`/`include` module detection, the eager
`inputs` drain that lost documents under `first`/`limit`/`any`/`all`/`isempty`/`nth`, the
missing filename in error locations, and `<unknown>` where jq names an exhausted file.
[#1504](https://github.com/rust-works/succinctly/issues/1504) closed two more — `inputs | f`
not interleaving, and a generator branch past a raised error still consuming a document —
both consequences of the evaluator's eager `Expr::Pipe`/`Expr::Comma` rather than of the
input builtins themselves. A top-level program that uses `input`/`inputs`/
`input_line_number` now runs through `eval.rs`'s demand-driven `eval_each_owned` (the same
`Demand`/`Item`/`Flow` sink `first`/`limit`/... already used) instead of `eval_single`'s
eager fold, so `inputs | input_line_number` reports `1 2 3` and `(., input) | error(...)`
raises once per top-level document, matching jq — see
[`docs/plan/jq-lazy-generator-consumers.md`](../../plan/jq-lazy-generator-consumers.md) for
the mechanism.

**The bridge is not free, and it is not universal.** It re-serialises and re-indexes each
document on top of the index `evaluate_input` already built, so its cost scales with document
size.

Measured on both pinned boxes, idle, interleaved within each repetition, medians of 5, with an
output-identity gate (#1603). Isolating the bridge needs **four** variants over one corpus, not
two, because a naive "with `input`" vs "without `input`" pairing varies document count, pipeline
count *and* the bridge all at once:

| | variant | docs | pipelines | bridge |
|---|---|---|---|---|
| **A** | `[.users[] \| select(.id != null) \| .id]`, 1-doc file | 1 | 1 | no |
| **C** | that filter plus `, ([.users[] \| .id])`, 1-doc file | 1 | 2 | no |
| **D** | the same two-pipeline filter, 2-doc file | 2 | 2 | no |
| **B** | `..., (input \| [.users[] \| .id])`, 2-doc file | 2 | 2 | **yes** |

14 MB `-p users`:

| box | A | C | D | B | C/A | D/C | **B/D — the bridge** |
|---|---|---|---|---|---|---|---|
| M4 Pro | 125 ms | 176 ms | 340 ms | 929 ms | 1.41x | 1.93x | **2.73x** |
| 7950X | 158 ms | 223 ms | 439 ms | 1407 ms | 1.41x | 1.97x | **3.21x** |

`D/C ≈ 1.95` is just "two documents cost about twice one document" — the term a two-variant
comparison folds into the bridge. The bridge itself is **2.7x on ARM, 3.2x on x86_64**, not the
~1.7x recorded here previously; that figure came from a battery-powered laptop run whose
comparison was not work-matched, and it understated the cost. Comparing whole commands (B/A)
instead gives 7.4x/8.9x, which overstates it by the same conflation in the other direction.

The multiplier is **stable across document size**, not growing: 7950X B/D measures 3.14x at
4 MB, 3.25x at 14 MB and 3.36x at 40 MB, with both D and B themselves growing linearly. So the
overhead is proportional to document size — the earlier "growing linearly" wording described the
absolute cost, which is true but says nothing the multiplier does not.

A filter with no input builtin is untouched, so the guard itself is free — only the programs it
fires for pay.

It is also **carved back out for cursor-metadata builtins**. `eval.rs` has no cursor to
answer position questions from: `line`/`column`/`document_index`/`anchor`/`style`/
`line_comment` are fixed-default stubs there and `at_offset`/`at_position` are
unconditional `requires document cursor context` errors. So a program mixing an input
builtin with one of those keeps the eager, cursor-carrying path and keeps its answer,
forgoing the interleave:

```
$ echo '{"a":1}' | succinctly jq -c 'at_offset(1), input_line_number'   # "a"  1
$ echo '{"a":1}' | succinctly jq -c 'line, column, input_line_number'   # 1  1  1
```

Re-indexing could not have rescued them: `eval_each_owned` rebuilds from re-serialised
text, so any offset or line/column it reported would describe that text rather than the
file the user passed. A confidently wrong position is worse than the divergence, so the
divergence is what these filters get. `at_offset`/`at_position`/`line`/`column` are
succinctly extensions, so no jq-compliance question arises either way.

One divergence remains, unrelated to the eager-evaluator root cause above.

**`input_line_number` keeps its line after a failed read.** jq resets it to 0 after an
`input` that finds nothing, but *not* after an `[inputs]` that exhausts the same stream:

```
$ printf '1\n2\n' | jq -cn '[inputs]|length, input_line_number, (try input catch "e"), input_line_number'
2  2  "e"  0                      # jq
2  2  "e"  2                      # succinctly
```

Deliberately not matched. jq is not self-consistent between the two exhaustion paths, and
a single probe admitting two readings is not a model worth encoding; the reset is not
reproduced until the rule behind it is known.

## A truncating consumer of `map(f)` skips the elements it never needed

Real jq's `map(f)` is `[.[] | f]` — an array construction, and array construction is
atomic: every element runs before anything downstream can observe the result, so a single
failing element fails the whole expression even when the consumer only ever wanted the
first output.

`succinctly jq` evaluates `map(f)` as a lazy sequence (#724, #725) and pulls from it on
demand, so a consumer that stops early never runs the elements past its stopping point —
and an element that would have errored is one such element:

```
$ echo '[1,"x",3]' | jq          -c 'map(.+1) | first'          # error, exit 5
$ echo '[1,"x",3]' | succinctly jq -c 'map(.+1) | first'        # 2, exit 0
$ echo '[1,"x",3]' | jq          -c 'first(map(.+1) | .[])'     # error, exit 5
$ echo '[1,"x",3]' | succinctly jq -c 'first(map(.+1) | .[])'   # 2, exit 0
```

Deliberate, and the whole point of the laziness: skipping elements that cannot affect the
requested output is the optimization, and restoring jq's atomicity would mean draining the
sequence before emitting anything — reinstating exactly the O(n) cost
[#1565](https://github.com/rust-works/succinctly/issues/1565) removed (a 2M-element
`first(map(.+1) | .[] | .+1)` went from ~1.8 s to ~0.04 s).

The divergence is bounded to consumers that genuinely truncate. Anything that has to see
the whole array still errors, in both tools:

```
$ echo '[1,"x",3]' | succinctly jq -c 'map(.+1)'         # error, exit 5
$ echo '[1,"x",3]' | succinctly jq -c 'map(.+1) | last'  # error, exit 5
$ echo '[1,"x",3]' | succinctly jq -c '[map(.+1) | .[]]' # error, exit 5
```

Pinned by [`test_generic_lazy_seq_first_after_map_skips_later_error_725`](../../../tests/jq_cli_tests.rs)
(the `map(f) | first` / `map(f) | .[0]` spelling) and
[`test_first_over_lazy_seq_iterate_skips_later_error_1565`](../../../tests/jq_cli_tests.rs)
(the `first(map(f) | .[] | g)` spelling, plus the draining counter-cases above).

## A truncating consumer of `keys_unsorted[]` skips a malformed member it never needed

Sibling of the `map(f)` divergence directly above, for the same underlying reason: a
[#1194](https://github.com/rust-works/succinctly/issues/1194) malformed object member
(a non-string key, or an unpaired trailing member with no value) is detected by walking
the object, and a demand-aware consumer that stops pulling early never walks past its own
stopping point.

[#1629](https://github.com/rust-works/succinctly/issues/1629) made every `keys_unsorted`
arm that already walks the whole object regardless (`keys_unsorted[]`, `| last`, a
*negative* `[n]`) raise on a malformed member, riding that walk for free.
[#1770](https://github.com/rust-works/succinctly/issues/1770) extended the same check to
`first(keys_unsorted[])`/`limit(n; keys_unsorted[])` for a malformed key the sink *actually
pulls* — `DistinctKeyCursors::next` already decodes every key it yields to hash it, so
checking it there costs nothing extra:

```
$ echo '{123: 1, "b": 2}' | succinctly jq -c 'first(keys_unsorted[])'      # error, exit 5
$ echo '{"a":1, 123:2}'   | succinctly jq -c 'limit(2; keys_unsorted[])'   # "a", then error, exit 5
```

What neither fix reaches: a malformed member sitting *after* whatever the consumer
actually pulled. Detecting an unpaired tail specifically requires reaching exhaustion
(there is no per-key signal for "the object ends improperly" the way there is for "this
key isn't a string") — which a truncating consumer may never do by design:

```
$ echo '{"a":1,123:2}' | succinctly jq -c 'first(keys_unsorted[])'   # "a", exit 0
$ echo '{"a":1,"b"}'   | succinctly jq -c 'first(keys_unsorted[])'   # "a", exit 0
$ echo '{"a":1,123:2}' | succinctly jq -c 'keys_unsorted[]'          # error, exit 5 (unaffected -- always exhausts)
```

Deliberate, for the same reason the `map(f)` divergence above is: restoring the check here
would mean walking past the point the demand-aware sink stopped, defeating the reason
`each_lazy_keys_iterate_sink` (`src/jq/eval_generic.rs`) exists rather than routing through
the already-checked, always-walks `fold_lazy_keys_stage`. Pinned by
[`test_jq_keys_unsorted_demand_aware_raises_on_pulled_malformed_key_1770`](../../../tests/jq_cli_tests.rs)
and
[`test_jq_keys_unsorted_demand_aware_still_known_gap_past_what_it_pulled_1770`](../../../tests/jq_cli_tests.rs).

## A truncating consumer of a plain array `.[]` skips a malformed comma it never needed

Sibling of the `map(f)`/`keys_unsorted[]` divergences above, for the same underlying
reason. [#1597](https://github.com/rust-works/succinctly/issues/1597) gave `.[]` over a
real array its own demand-aware walk (`each_lazy_array_iterate_sink`,
`src/jq/eval_generic.rs`) so `first(.[])`/`limit(n; .[])` stop pulling cursors after `n`
elements instead of materializing every one first (a 2M-element array's `first(.[] | .)`
went from ~92 MB peak RSS to matching the ~28 MB `length` control). The walk still checks
each element's own preceding `,` (#1677) as it goes — an element the sink *actually pulls*
is checked exactly as before — but a malformed comma sitting *after* whatever the consumer
stopped at is never reached:

```
$ printf '[,1,3]' | succinctly jq -c 'first(.[])'   # error, exit 5 (malformed comma IS the first thing examined)
$ printf '[1,,3]' | succinctly jq -c 'first(.[])'   # 1, exit 0    (malformed comma is past element 1)
$ printf '[1,,3]' | succinctly jq -c '.[]'          # error, exit 5 (unaffected -- always exhausts)
```

A bound *larger* than what the array can supply before the defect still reaches it, and
still streams whatever was already confirmed good first — the same shape the
`keys_unsorted[]` entry above already documents for `limit(2; keys_unsorted[])`:

```
$ printf '[1,2,,4]' | succinctly jq -c 'limit(3;.[])'   # 1, 2, then error, exit 5
```

This is not a separate divergence from the one above — it is the same "only checked as
far as the sink actually pulled" rule, just landing on a bound the sink *did* need rather
than one it stopped short of. The `keys_unsorted[]` entry's own `limit(2; ...)` example
already established this exact shape for objects; this is its array counterpart, not a new
kind of gap.

Deliberate, for the same reason the two divergences above are: restoring the check here
would mean walking past the point the demand-aware sink stopped, defeating the reason
`each_lazy_array_iterate_sink` exists rather than routing through the always-eager
`collect_cursors_checked`. Objects are unaffected by this specific change — a plain `.[]`
over an object still takes the original eager path (`.[]`'s duplicate-key collapse
semantics need `DistinctKeyCursors`'s streaming-collapse machinery extended to also carry
value cursors, deferred as separate, larger remaining scope on #1597).

Format-generic, not jq-mode-specific: `each_lazy_array_iterate_sink` takes no
`S: EvalSemantics` and is reached identically via `succinctly yq`. Currently inert for
YAML, though — every YAML cursor's `preceding_delimiter_ok` uses the trait default (always
`true`), since YAML's own parser validates delimiters while parsing rather than deferring
to this evaluator-level check the way JSON's semi-index does — so there is no YAML input
this gap check can actually catch or skip either way, today.

## `limit`'s own `n` argument is not demand-aware when it is itself a generator

Real jq passes `limit($n; f)`'s `$n` through the same backtracking arg-passing convention
as any other filter argument, so a *generator* `n` (`limit((1,2); f)`, #1279's own canonical
example) re-runs the whole `limit` body once per bound value of `$n` — but only as many
times as the wrapping consumer actually needs:

```
$ succinctly jq -cn 'first(limit((1,2); (1, ("B"|stderr))))'      # jq: 1, no stderr
1
$ succinctly jq -cn '[limit((1,2); (1, ("B"|stderr)))]'           # jq: [1,1,"B"], stderr B (both agree)
[1,1,"B"]
B
```

The second row is not a typo: an *unbounded* consumer genuinely needs both of `$n`'s
bindings (`$n=1` keeps `expr`'s first output alone; `$n=2` keeps its first two, so
`("B"|stderr)`'s own value ends up in the array too), so exploring `expr`'s second output
there is correct in both tools. The first row is where they diverge — jq's own `first`
stops after `$n=1`'s single output and never even considers the `$n=2` binding, so
`stderr` stays empty; `succinctly jq` still writes `B`.

**Root cause: no demand-aware entry point exists yet for a generator `n`.** `limit`'s fast,
demand-forwarding paths (`eval::each_limit`, `eval_generic.rs`'s `eval_limit_generic`/
`each_limit_generic`, #1607/#1596) all defer to `eval.rs`'s plain, fully-materializing
`eval()`/`full_eval` the moment `n` is anything beyond a single plain value — the same
"give up on the fast path, hand the whole node to the collecting evaluator" policy `map(f)`
above used to have before #724/#725 gave it a genuinely lazy sequence type. `limit`'s own
generator-`n` case has no equivalent lazy machinery to defer to: `eval()` collects every
output across every `$n` binding into one `QueryResult` before any wrapping consumer's
`Demand` is ever consulted, so a `first`/`nth` sitting outside a generator-`n` `limit` gets
the answer only after paying (and leaking any side effect from) work it never asked for.

Deliberately not fixed here: it needs `eval.rs` itself to gain a demand-aware path for a
generator `n`, not another consumer-side guard — the same order of work #724/#725 needed
for `map(f)`, not a small patch. `n_expr` as a generator is jq's own rarer laziness
contract to begin with (most real `limit`/`nth` calls pass a scalar), so the common case is
unaffected; only a bare, side-effecting, multi-output `n` under a truncating consumer sees
this. Tracked in
[`docs/plan/jq-lazy-generator-consumers.md`](../../plan/jq-lazy-generator-consumers.md)
(item 9) as a residual, not silently reintroduced — `each_limit_generic`'s own doc comment
carries the same note at the call site.

## A `?//`-alternatives bind under a short-circuiting consumer runs once, not once per alternative

Real jq's short-circuiting builtins (`first`, `isempty`, `limit`, `any`, `all`, ...) are defined
in `builtin.jq` as `label $out | ... break $out`. When the generator argument is a
`?//`-alternatives bind and that `break` unwinds through it, jq's `?//` treats *any* escaping
break as "this alternative failed, try the next" — it does not distinguish a label declared
outside the whole `?//` from one declared inside it. The consumer's whole computation therefore
runs once per alternative, even though only the first alternative's output is ever kept. Every
row confirmed live against jq 1.7.1:

| filter                              | jq 1.7.1             | succinctly jq |
|-------------------------------------|----------------------|---------------|
| `isempty(1 as $x ?// $y \| 5)`      | `false` then `false` | `false`       |
| `[first(1 as $x ?// $y \| 5, 6)]`   | `[5,5]`              | `[5]`         |
| `[limit(1; 1 as $x ?// $y \| 5)]`   | `[5,5]`              | `[5]`         |
| `1 as $x ?// $y \| 5` (no consumer) | `5`                  | `5`           |

`succinctly jq` resolves `?//` via static Rust-side pattern-alternative resolution
(`try_pattern_alternatives`/`each_pattern_alternatives`, `src/jq/eval.rs`) with no concept of an
outer `label`/`break` at all — its builtins are native Rust, not user-space `builtin.jq` macro
expansions — so it always produces exactly one output regardless of alternative count. This is
the *non-duplicating* direction: succinctly's own output is the smaller, arguably more intuitive
one, but it is a real divergence from jq's own defined semantics.

Not fixed here — whether to reproduce jq's per-alternative re-run behaviour is an open
implementation question tracked in
[#1519](https://github.com/rust-works/succinctly/issues/1519), which this entry was split from
([#1831](https://github.com/rust-works/succinctly/issues/1831)) to record the divergence
unconditionally regardless of how that question is resolved. Piggybacks on the `?//` retry rule
from [#1457](https://github.com/rust-works/succinctly/issues/1457); unrelated to the other `?//`
divergence already recorded above ([#1365](https://github.com/rust-works/succinctly/issues/1365),
`?//`-alternatives folds not being path-tracked).

## `--seq`'s malformed-record warning covers one of jq's several message shapes

Real jq warns on stderr ("`jq: ignoring parse error: ...`") whenever it silently drops a
malformed `--seq` (RFC 7464) record; `succinctly jq` used to drop the record with no
diagnostic at all. [#1525](https://github.com/rust-works/succinctly/issues/1525) added the
warning for exactly one of jq's message templates: content with no RS byte anywhere in the
input, where jq's own reader never resyncs onto anything and reports the abandonment
unconditionally at EOF ("`Unfinished abandoned text at EOF at line L, column C`") regardless
of what the content actually is, even fully valid JSON:

```
$ printf '1 2' | jq --seq -c '.'                  # jq:          jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3
$ printf '1 2' | succinctly jq --seq -c '.'       # succinctly:  same (#1525)
```

A malformed record that *does* start with an RS byte gets one of at least three other jq
message templates instead, depending on exactly how it's malformed — none implemented yet:

```
$ printf '\x1e"unterminated\n' | jq --seq -c '.'
jq: ignoring parse error: Unfinished string at EOF at line 2, column 0
$ printf '\x1e[1,2\n'          | jq --seq -c '.'
jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 0
$ printf '\x1exyz\n'           | jq --seq -c '.'
jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)
```

`succinctly jq --seq` emits nothing on stderr for any of these, and its *output* also
diverges: real jq emits the values it managed to read before the malformed point, while
succinctly drops the whole record.

```
$ printf '\x1e1 {invalid\n' | jq            --seq -c '.'   # warns, then prints 1
$ printf '\x1e1 {invalid\n' | succinctly jq --seq -c '.'   # prints nothing
$ printf '\x1e1,2\n'        | jq            --seq -c '.'   # prints 2
$ printf '\x1e1,2\n'        | succinctly jq --seq -c '.'   # prints nothing
```

**The divergence from *this* rule is one-directional: succinctly's output is a subset of
jq's, not a superset** -- 0 superset violations across 6,000 randomly generated single
records, where `main` has 787.

That is a statement about malformed-record handling, not a guarantee about `--seq` as a
whole. On multi-record streams a *separate, pre-existing* rule still diverges in the other
direction, and it is **not** simply "the trailing record lacks a newline" -- a first draft of
this note said that, and the oracle contradicts it:

```
$ printf '\x1e"a"\x1e"b"'    | jq --seq -c '.'   # "a" and "b"  -- no newline anywhere, both kept
$ printf '\x1e"a"\n\x1e"b"'  | jq --seq -c '.'   # "a" only     -- a newline earlier changes it
$ printf '\x1e"a"\n\x1e"b"\n'| jq --seq -c '.'   # "a" and "b"
```

The trigger is a newline appearing *earlier in the stream*: once jq's reader has seen one, a
later record must itself be newline-terminated to be emitted. succinctly keeps it either way
(`printf '\x1e0007\n\x1e[]'` is `7` in jq, `7` and `[]` here) -- identical on `main`, so not
introduced by this work. Measured over 2,000 multi-record streams: 133 such cases here
against `main`'s 610, the reduction coming entirely from the malformed-record rules above. Reproducing the prefix needs the same thing the diagnostics
do -- jq's `--seq` reader is a streaming lexer/parser with error recovery, and knowing which
values survive means knowing where its parser gave up. A token scanner cannot substitute for
it, and trying was actively harmful: an attempt to emit the prefix by scanning tokens and
skipping bad ones **fabricated output**, printing values real jq never prints (`\x1e1-2\n`
became `1` and `-2`, where jq lexes one malformed number and prints nothing). Requiring a
*pending* token -- a number or bare `true`/`false`/`null`, neither of which carries a closing
delimiter -- to be confirmed by whitespace or the start of a self-delimiting value is what
rules that out, at the cost of dropping records jq can partially read. Strings, objects and
arrays are exempt: they self-terminate, so `\x1e{"a":1}{"b":2}\n` and `\x1e1[1]\n` are two
values apiece in both tools. Requiring whitespace after *every* token instead, as a first
draft did, dropped 332 records real jq reads fully.

The same pending-token rule applies at a record boundary, where `\x1etrue\x1e3\n` yields
only `3` -- a bare literal is as truncatable as a bare number. Both directions are pinned by
`test_seq_adjacent_tokens_never_fabricate_1723` and
`test_seq_partially_readable_record_is_dropped_whole_1723`
([tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs)).

A record holding several *well-formed* values is likewise unaffected: #1723 fixed
`succinctly jq --seq` dropping those in full (`\x1e1 "x" [2]\n` is three outputs, as in real
jq), which was silent data loss rather than a diagnostic gap. Matching these needs enough
of jq's own incremental-parser failure classification to know which of its internal states a
segment would have failed in, not just whether it parses — deliberately deferred, tracked in
[#1723](https://github.com/rust-works/succinctly/issues/1723).

A second, narrower gap `succinctly jq --seq` also deliberately stays silent on: `-n`
combined with a filter that forces a real read (`input`/`inputs`) turns the identical
no-RS-byte condition into a **fatal** error on real jq (exit 5), not a warning:

```
$ printf '1 2' | jq -n --seq -c '[inputs]'
jq: error (at <stdin>:0): Unfinished abandoned text at EOF at line 1, column 3
$ printf '1 2' | succinctly jq -n --seq -c '[inputs]'
[]
```

`succinctly jq` has never implemented this fatal-vs-warning distinction (predates #1525, not
a regression); #1525 specifically avoids printing the warning's exact wording in this one
mode (`!args.null_input` in `seq_no_rs_byte_warning`'s call site,
[src/bin/succinctly/jq_runner.rs](../../../src/bin/succinctly/jq_runner.rs)), since doing so
would make the still-wrong exit code/output look like it now matched jq's message. Fixing the
underlying exit-code divergence properly needs `get_inputs` to be able to raise a real,
fatal `EvalError` from this specific condition rather than only ever warning or returning
values — not attempted here.

## Real-time stdout/stderr interleaving: every route but the M2 lazy path

Real jq is a lazy generator, so a filter that both writes to stdout and triggers a stderr
side effect (`debug`, `stderr`, `halt_error`) or raises mid-stream interleaves the two as it
goes. `succinctly jq` evaluated a whole input's filter into a `Vec` before writing *any* of
it, so every stderr write had already happened by the time the first stdout write ran —
which no amount of buffering could fix, `--unbuffered`'s per-write `flush()` included
([#1653](https://github.com/rust-works/succinctly/issues/1653)).

Fixed for every route except the M2 lazy path below — `-n`, `--slurp`, `--input-dsv`, the
`input`/`inputs` bridge, and any flag that forces materialization (`-S`, `-a`, `-R`,
`--seq`, `--args`) — all of which now write each output as the evaluator produces it
(`evaluate_input_streaming`,
[src/bin/succinctly/jq_runner.rs](../../../src/bin/succinctly/jq_runner.rs), over
`eval_each_with_cursor`, [src/jq/eval_generic.rs](../../../src/jq/eval_generic.rs)):

```
$ jq            --unbuffered -cn '1, debug, 2'   2>&1     # 1 / ["DEBUG:",null] / null / 2
$ succinctly jq --unbuffered -cn '1, debug, 2'   2>&1     # same
```

**Still batched: a document read from a file or stdin** — the M2 lazy path:

```
$ echo '[1,2]' | jq            --unbuffered -c '.[]|debug'   2>&1
["DEBUG:",1]
1
["DEBUG:",2]
2
$ echo '[1,2]' | succinctly jq --unbuffered -c '.[]|debug'   2>&1
["DEBUG:",1]
["DEBUG:",2]
1
2
```

Not an oversight, and not merely unfinished: streaming that path means routing the CLI's
*default* route through the demand-driven evaluator, whose `each_lazy_keys_iterate_sink`
deliberately skips the malformed-key check (#1194) that
[#1629](https://github.com/rust-works/succinctly/issues/1629) added so that bare
`keys_unsorted[]` *would* raise. [#1770](https://github.com/rust-works/succinctly/issues/1770)
closed that gap as an accepted trade for early-exit consumers like `first(...)` — see
"A truncating consumer of `keys_unsorted[]` skips a malformed member it never needed" above
— on the reasoning that detecting it requires the full walk such a consumer exists to avoid.
Making it the default route would silently generalise that trade to every query. Tried and
reverted while fixing #1653: it regressed six tests across #1642/#1629/#1770's
undecodable-key handling, including `.,.` on a document with colliding undecodable keys
emitting them twice at exit 0 instead of raising.

`test_streamed_ordering_not_yet_reached_on_m2_path_1653`
([tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs)) pins the current batched output, so
this stops being silent the moment it changes.

## Deliberate divergences (ADR-0018 rule 4)

### A structurally malformed value doesn't abort the rest of a multi-value stream — no carve-out; this one is out of policy

Real jq DOM-parses the whole input before evaluating anything, so a parse error on one
value in a multi-value stream kills the run outright — nothing after the bad value is
even attempted:

```console
$ printf '1\n[xyz123]\n3\n' | jq -c 'to_entries'
jq: error (at <stdin>:1): number (1) has no keys
jq: parse error: Invalid numeric literal at line 2, column 8
$ printf '1\n[xyz123]\n3\n' | succinctly jq -c 'to_entries'
jq: error (at <stdin>:1): number (1) has no keys
jq: error (at <stdin>:2): unexpected character
jq: error (at <stdin>:3): number (3) has no keys
```

succinctly never parses the whole input up front (semi-indexing is per-record), and a
structurally malformed value (#1194) or an undecodable string (#1247) now surfaces
through the same `EvalError`/`ErrorSink` per-record-error-then-continue convention every
other jq error already uses (`ErrorSink`, [#355](https://github.com/rust-works/succinctly/issues/355))
— so `3` still gets processed here where real jq never reaches it. This is not new to
[#1247](https://github.com/rust-works/succinctly/issues/1247): `ErrorSink`'s
continue-past-one-error batch semantics predate it and already diverge from real jq's
abort-on-parse-error for every ordinary evaluation error, not just a decode failure.
[#1247](https://github.com/rust-works/succinctly/issues/1247)'s own design doc
([`docs/plan/decode-failure-routing.md`](../../plan/decode-failure-routing.md), Stage 4)
calls succinctly's behaviour "the more useful of the two" and treats it as settled — but
none of ADR-0018's four permitted conditions actually cover it (the output is readable,
nothing is corrupted or discarded, and the process does not die either way), so per rule 4
it is recorded here as a still-open policy question, matching
[yq Limitations](../yq/limitations.md)'s "Merge-flag `+` and `d` combined" precedent for a
divergence accepted on its merits without fitting the letter of rule 4.

### `--raw-output0`'s new NUL-content error (#1830) inherits `ErrorSink`'s pre-existing exit-code stickiness (#1855) — not fixed here

[#1830](https://github.com/rust-works/succinctly/issues/1830) added a check rejecting a
`--raw-output0` string whose own content contains a NUL byte, matching real jq's own refusal.
That new error reports through the same `ErrorSink`/`sink.hit()` convention every other jq
error in this file already uses — which means it also inherits `ErrorSink`'s existing,
separately-tracked divergence: real jq's own exit code for an uncaught error reflects only the
*last* top-level document processed, not "was any document rejected", where succinctly's
`sink.hit()` is sticky for the whole run once set:

```console
$ printf '"a"\n"b\u0000c"\n"d"\n' | jq -r --raw-output0 '.'
a␀d␀                                      # stderr: the NUL error, for the middle document
$ echo $?
0                                         # the LAST document ("d") succeeded
$ printf '"a"\n"b\u0000c"\n"d"\n' | succinctly jq -r --raw-output0 '.'
a␀d␀                                      # identical stdout and stderr
$ echo $?
5                                         # sink.hit() stays sticky from the middle document
```

This is not specific to the NUL check -- it is the general `ErrorSink` stickiness behavior,
already filed as [#1855](https://github.com/rust-works/succinctly/issues/1855) ("jq: exit code
is sticky (any-error) across multi-document input; real jq uses only the last document's
outcome") before #1830 added this particular error class to the set of things that can trigger
it. #1830 deliberately did not attempt a fix here, to avoid duplicating #1855's own scope.

### `ascii_downcase`/`ascii_upcase` inside `path()` name the outer value, not jq's inner exploded-array step — no carve-out; recorded on its merits

```console
$ echo '{"a":"xyz"}' | jq -c 'path(.a | ascii_downcase)'
jq: error (at <stdin>:1): Invalid path expression near attempt to iterate through [120,121,122]
$ echo '{"a":"xyz"}' | succinctly jq -c 'path(.a | ascii_downcase)'
jq: error (at <stdin>:1): Invalid path expression with result "xyz"
```

(same for `ascii_upcase`; both refuse with the same exit code, `5`, and the same
`invalid_path_expression` message class -- only the parenthetical detail differs.)

Real jq's `ascii_downcase`/`ascii_upcase` are `builtin.jq` prelude definitions
(`explode | map(...) | implode`), so the path-expression check fails *inside*
that definition, at the exploded array's iterate step, naming the codepoint
array. succinctly implements both natively for performance, so its check
fails at the outer call boundary and names the original string instead. The
control case confirms it is the definition boundary and not the wording
itself: `path(.a | explode)`, where both implementations are a single native
step, agrees byte-for-byte in both tools.

This does not fit any of ADR-0018 rule 4's four conditions: the output is not
unreadable (a), nothing is corrupted or discarded (b), no process dies (c),
and (d) is specifically about a dependency choice -- there is no crate to
evaluate here, only an internal implementation choice between two options,
both considered and rejected on cost:

- **Re-derive `ascii_downcase`/`ascii_upcase` from the prelude definition**
  (`explode | map(...) | implode`) instead of a native implementation.
  Closes the gap, but degrades *every* ordinary (non-`path()`) call to these
  two hot-path string builtins -- the common case pays a real performance
  cost for an edge case that already errors either way.
- **Special-case `path()` to detect these two builtins and report a
  synthetic exploded-array step.** Closes the gap, but adds a second,
  dedicated code path duplicating `explode`'s output just to build an error
  message on an already-refused expression -- fragile (reports evaluation
  that didn't actually happen) and only covers these two names, not the
  general prelude-definition-vs-native-implementation gap other builtins
  could hit the same way.

Both candidates cost real complexity or a real performance regression to fix
wording on an expression that already fails identically in both tools (same
exit code, same error class) -- calling a string-transform builtin inside
`path()` is already a user error jq itself refuses, so the value of matching
its exact internal step name is near zero. Recorded here as a divergence
accepted on its merits, following this page's own "A structurally malformed
value doesn't abort the rest of a multi-value stream" entry above as
precedent for a gap that doesn't fit the letter of rule 4 but is kept rather
than silently left unrecorded. See
[#1561](https://github.com/rust-works/succinctly/issues/1561).

### `repeat(f)` silently caps at 1000 rounds, in both value and path mode

`repeat(f)` (`def repeat(f): f, repeat(f);`) has no base case at all: an `f`
that never errors and never produces a value on some round recurses forever.
Confirmed live that real jq itself hangs on this rather than raising or
terminating:

```console
$ timeout 3 jq -cn 'limit(3; repeat(empty))'; echo "exit: $?"
exit: 124                                 # real jq hangs -- 124 is `timeout`'s own code
```

`eval_repeat` (value mode, `src/jq/eval.rs`) already backstops this with a
`MAX_ITERATIONS = 1000` round cap, so `limit(3; repeat(empty))` returns no
output and exits 0 in succinctly instead of hanging the process --
unquestionably permitted under ADR-0018 rule 4c (matching a reference's hang
is not something this project attempts). [#1906](https://github.com/rust-works/succinctly/issues/1906)
added the identical cap to `resolve_repeat_bounded` (path mode, the
`path(repeat(f))`/`path(limit(n; repeat(f)))` route) for the same reason and
to keep the two modes consistent with each other.

The cap has a real side effect neither call site's own doc comment
mentioned before now: it also silently truncates a *legitimate*, `n` greater
than 1000 request, with no error and no warning, where jq itself handles a
large `n` trivially:

```console
$ jq -cn '[limit(1500; repeat(1))] | length'
1500
$ succinctly jq -cn '[limit(1500; repeat(1))] | length'
1000
```

This half of the behavior doesn't fit any of ADR-0018's four conditions on
its own (the output is readable, nothing is corrupted, and unlike the
`repeat(empty)` case this input does not threaten to hang the process) --
but it is the unavoidable other side of the same guard that rule 4c does
cover, not a second, separable design choice: a per-round cap cannot
distinguish "this round produced nothing because `f` is degenerate" from
"this round produced nothing yet because we haven't gotten to round 1001",
so raising the cap only moves where a sufficiently large `n` starts silently
truncating rather than removing the tradeoff. Recorded here rather than left
implicit in a doc comment that named only the case it was actually written
to prevent.

### `path(repeat(f))` tracking (#1906/#1935) only reaches `limit`/`first`'s *direct* child, not `nth` or a combinator-nested `repeat`

[#1906](https://github.com/rust-works/succinctly/issues/1906)/PR #1933 added `Expr::Repeat`
interception for `path(limit(n; repeat(f)))` (`resolve_limit_one_n`);
[#1935](https://github.com/rust-works/succinctly/issues/1935) extended the identical
interception to `path(first(repeat(f)))` (`first(f)` being `limit(1; f)` for this purpose).
Both fixes only fire when `Expr::Repeat` is the *direct* (paren-unwrapped) child of the
bounded consumer's own body expression:

```console
$ echo '{"a":{"a":2}}' | jq -c 'path(nth(2; repeat(.a)))'
["a"]
$ echo '{"a":{"a":2}}' | succinctly jq -c 'path(nth(2; repeat(.a)))'
jq: error (at <stdin>:1): Invalid path expression with result {"a":2}

$ echo '{"a":1}' | jq -c 'path(limit(2; if true then repeat(.) else 1 end))'
[]
[]
$ echo '{"a":1}' | succinctly jq -c 'path(limit(2; if true then repeat(.) else 1 end))'
jq: error (at <stdin>:1): Invalid path expression with result {"a":1}
```

`nth(n; f)` (`Builtin::NthStream`) has no `resolve_node` arm at all today -- a pre-existing
gap independent of `repeat` (`path(nth(1; .a,.b))` on an ordinary, non-`repeat` generator
already diverges the same way). Extending `repeat`-specific interception to `nth` has
nothing to attach to until `nth` has *any* path-context support to extend. `Expr::If`/
`Expr::Alternative`/(likely) `Expr::Comma` all recurse into `resolve_node` directly on
their branches rather than through a bounded consumer, so a `repeat` reached only after
passing through one of these has no way to know, at that point, how many outputs will
actually be needed -- closing this generally would need a bound threaded *through*
arbitrary combinator nesting before ever calling `resolve_node` on the `repeat` itself,
which its current single `(expr, value, trackable, snapshot) -> Vec<PathBranch>` signature
has no parameter for. Tracked as its own standalone design question in
[#1952](https://github.com/rust-works/succinctly/issues/1952) (a general sink/early-stop
protocol for `resolve_node`, the same shape `eval_each_owned` already gives value-mode
evaluation) rather than attempted here; the narrow `first`/`limit` fixes already close
the two shapes named in #1906/#1935's own repros.

### A generator-argument expression used to fan out normally, then silently narrow to a single array the moment `key`/`parent`/`file_index` showed up anywhere else in the same pipe -- fixed for 3 of 4 sites

[#1277](https://github.com/rust-works/succinctly/issues/1277)'s clusters 1-3
(closed by [#1522](https://github.com/rust-works/succinctly/issues/1522)/[#1279](https://github.com/rust-works/succinctly/issues/1279))
gave generator-argument builtins real fan-out: a builtin whose argument is a
generator now produces one output per argument output, matching real jq.
Four call sites inside `eval_pipe_with_path_context_internal`
(`src/jq/eval.rs`) -- `ParentN`'s own `n` argument, the `Expr::Builtin(_)`
arm, the `Expr::Object`/`Array`/`Literal` arm, and the generic `_` fallback
-- were an explicit non-goal of that fix, since giving them the same real
fan-out looked like a materially larger change
(`docs/plan/jq-generator-argument-fanout.md`).

```console
$ echo '{"a":"xax"}' | succinctly jq -c '.a | ltrimstr(("x","z"))'
"ax"
"xax"
$ echo '{"a":"xax"}' | succinctly jq -c '.a | ltrimstr(("x","z")) | key'
"a"
```

The first query (no `key`, so the ordinary fan-out-aware evaluator handles
it) correctly produces 2 outputs, matching real jq's own fan-out for this
exact query (confirmed against jq 1.7.1). The second query is identical
except for the trailing `key`, which forces the whole pipe through
`eval_pipe_with_path_context_internal` since `key` needs path tracking --
`key` itself has no jq oracle (succinctly extension), so this specific
combination can't be demonstrated as a *jq* divergence in isolation, but it
was a genuine, demonstrated internal inconsistency: the same sub-expression
fanned out correctly or silently collapsed into one array-shaped value
depending on whether an unrelated path-tracking builtin happened to be
anywhere else in the pipe.

**[#1937](https://github.com/rust-works/succinctly/issues/1937) fixed one
narrow, safe piece first**: a *zero*-output generator (`(empty)`-style) now
correctly contributes zero outputs to the enclosing computation, rather than
an `OwnedValue::Array([])` value -- matching `result_to_owned_full`'s
identical `#1045` rule for the same `Many`/`ManyOwned` shapes.

**A first attempt at #1937 also tried taking the *first* output instead of
array-collapsing for the 2+-output case (matching `result_to_owned_full`'s
policy for that shape too) -- this was implemented, tested, and then
rejected during `/code-review` before merging, because it introduced a
strictly worse regression than the one it fixed.** `result_to_owned_full`'s
take-first policy is correct for *its* callers -- a builtin's single
generator argument, always consumed exactly once by the builtin's own body.
But `eval_owned_expr_full` (what that attempt changed) is not scoped that
narrowly: its `Expr::Builtin(_)` arm also evaluates *zero-arg* generator
builtins (`recurse`, `range`, `inputs`, ...) whenever they're reached
through path-context routing, and its generic `_` fallback evaluates
arbitrary comma branches, `..`, `paths`, `limit`, and anything else with no
dedicated arm. Take-first would have silently and permanently discarded 2 of
`recurse`'s 3 outputs with no error and no trace -- reverted before merging.

**[#1964](https://github.com/rust-works/succinctly/issues/1964) closed the
gap properly for 3 of the 4 sites**, without take-first's regression: the
`Expr::Builtin(_)` arm, the generic `_` fallback, and (added during that
issue's own `/code-review`, after a finder disproved the "not known to be
live-reachable" claim below) the `Expr::Object`/`Array`/`Literal` arm now
route through `eval_owned_input` (which preserves `Many`/`ManyOwned`)
instead of `eval_owned_expr_opt` (which collapsed to one value).
`continue_rest_with_context`/`accumulate_path_context_step` -- the functions
that actually fan a `Comma` branch's output back out into the enclosing
computation -- already handled `ManyOwned` correctly; they just never used
to receive one from these three arms. No change was needed to either of
them, or to `eval_owned_expr_full`/`result_to_owned_full` at all -- #1937's
mistake was treating this as a policy question for a shared helper, when it
was really a "the wrong function got called" bug local to a handful of arms.
The `Expr::Object`/`Array`/`Literal` arm couldn't reuse
`continue_rest_with_context`'s own `ManyOwned` handling directly, though:
that shares one `root`/`current_path` across every element, correct for
`Builtin`/the fallback (whose navigational position never moves) but wrong
here, where *each* constructed value must become its own root -- so that
arm loops by hand instead, mirroring how `Expr::Comma` accumulates its own
branches.

```console
$ echo '{"a":{"b":{"c":1}}}' | succinctly jq -c '.a | (recurse, key)'
{"b":{"c":1}}
{"c":1}
1
"a"
$ echo '{"a":"x"}' | succinctly jq -c '.a | [(range(0;5), key)]'
[0,1,2,3,4,"a"]
$ echo '{"a":"x"}' | succinctly jq -c '.a | ({x:(1,2)}, key)'
{"x":1}
{"x":2}
"a"
```

All three now match real jq's own fan-out shape (confirmed against jq 1.7.1
with a literal substituted for `key`, which has no oracle):
`jq -c '.a | [recurse]'`, `jq -c '.a | [(range(0;5), "a")]'`, and
`jq -c '.a | ({x:(1,2)}, "a")'` give the identical value sequences.

**Fixing the `Expr::Builtin(_)`/fallback arms surfaced one further latent
bug, in shared code neither #1937 nor #1964's first draft touched**:
`continue_rest_with_context` (and its twin, `continue_rest_with_fresh_root`)
each have a `Partial` arm that pipes an already-produced prefix through
`rest` before reattaching the trailing `Control` -- but neither one gated
that reattachment on their own `optional` parameter, so `?` correctly kept
the prefix but then failed to suppress the trailing error:

```console
$ echo '{"a":{"b":{"c":1}}}' | succinctly jq -c \
    '.a | (recurse(if type=="object" then error("boom") else empty end))? | key'
# before this fix: "a" printed, then "boom" raised anyway, exit 5 -- `?` didn't suppress it
# after:           "a"                                                exit 0 -- matches jq (substituting a literal for `key`)
```

This was unreachable before #1964 gave these two arms a real `Partial` to
hand `continue_rest_with_context` in the first place (previously,
`eval_owned_expr_opt` intercepted the shape itself and discarded the prefix
entirely -- a different, separately-known bug). Fixed by routing both
functions' `Partial` arms through the existing `catch_error_under_optional`
helper (the same one `Expr::Iterate`/`Builtin::Map` already use for this
exact "keep the prefix, gate only the error" policy) instead of a bare
`partial(...)` call.

**One site remains unfixed**: `ParentN`'s own `n` argument (`parent((1,2))`
still array-collapses `n` to `[1,2]`, then errors on the wrong type -- a
narrower, lower-traffic case than the three arms fixed above, not attempted
here). Full fan-out for it, if ever needed, remains out of scope per
#1522's design doc.

## Provenance

| Artifact           | Path                                                                                                       |
|--------------------|------------------------------------------------------------------------------------------------------------|
| Probes             | [`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv)                                 |
| Captured messages  | [`tests/data/jq-error-messages.tsv`](../../../tests/data/jq-error-messages.tsv)                             |
| Known divergences  | [`tests/data/jq-error-known-divergences.txt`](../../../tests/data/jq-error-known-divergences.txt)           |
| Harness            | [`tests/jq_error_message_tests.rs`](../../../tests/jq_error_message_tests.rs)                               |
| Sync script        | [`scripts/sync-jq-error-messages.sh`](../../../scripts/sync-jq-error-messages.sh)                           |
| Message vocabulary | [`src/jq/error.rs`](../../../src/jq/error.rs)                                                               |
| Version pin        | [`tests/data/jq-golden/JQ_VERSION`](../../../tests/data/jq-golden/JQ_VERSION)                               |
| End-to-end goldens | [`tests/data/jq-golden/cases/`](../../../tests/data/jq-golden/cases/) (the `error_msg_*` cases)             |

The captured table is committed, so `cargo test` runs hermetically with no `jq` on PATH.
The `jq-drift` CI job re-checks it against the pinned binary, in the same step group as
the golden fixtures — so a jq upgrade surfaces as churn in the table and the divergence
manifest rather than as a silent mismatch.

To move to a newer jq, bump `JQ_VERSION`, install that version, run
`./scripts/sync-jq-error-messages.sh` and `./scripts/sync-jq-golden.sh`, and review the
diff.

## Depends On

- [ADR-0018](../../adrs/adr-0018.md) - the fidelity rule this page enumerates exceptions to
- [ADR-0019](../../adrs/adr-0019.md) - the decision that made the regex `l`/`n` gaps permanent
- [jq Evaluator](../../reference/jq-evaluator.md) - the evaluator raising these errors
- [jq Language Support](../../reference/jq-language.md) - feature coverage matrix
- [yq Limitations](../yq/limitations.md) - the yq-mode counterpart to this page

## Used By

- [jq benchmarks](../../benchmarks/jq.md) - comparison against `jq`

## Source & Docs

- [`src/jq/error.rs`](../../../src/jq/error.rs) - the message vocabulary
- [`src/jq/eval.rs`](../../../src/jq/eval.rs) - full evaluator
- [`src/jq/eval_generic.rs`](../../../src/jq/eval_generic.rs) - generic evaluator (CLI path)
- [jq manual](https://jqlang.github.io/jq/manual/) - upstream reference
