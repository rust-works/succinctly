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

Measured against jq-1.7.1 over the 223 probes in
[`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv), through
**both** evaluators — the full one (`src/jq/eval.rs`) and the generic one
(`src/jq/eval_generic.rs`, which the CLI uses):

| Dimension                                    | Result              | Meaning                                                |
|----------------------------------------------|---------------------|--------------------------------------------------------|
| **Message text** (both evaluators, verbatim) | **221/223 = 99.1%** | Byte-identical to jq                                   |
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

### `path()`'s own "Invalid path expression..." family truncates wider (#2179)

The three `path()`-triggered messages — `Invalid path expression with result <v>`,
`... near attempt to access element <k> of <v>`, `... near attempt to iterate through <v>`
— do **not** share the 11-byte truncation above. Confirmed by reading jq 1.7.1's C source
(`execute.c`): the `PATH_END`/`EACH`/`EACH_OPT` cases, and `INDEX`/`INDEX_OPT`'s own
*container* argument, call `jv_dump_string_trunc` with `char errbuf[30]`/`objbuf[30]` —
keeping 26 bytes before the `...`, not 11. `INDEX`/`INDEX_OPT`'s *key* argument is the one
exception within this family: it uses `char keybuf[15]`, the same narrow 11-byte width as
every other message above. Reproduced exactly:

```bash
$ jq -n -c 'path({"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":1})'
jq: error (at <unknown>): Invalid path expression with result {"aaaaaaaaaaaaaaaaaaaaaaaa...
                                                              # 26 bytes kept, not 11
$ jq -n -c 'path(try (.a, error({"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":1})) catch .b?)'
jq: error (at <unknown>): Invalid path expression near attempt to access element "b" of {"aaaaaaaaaaaaaaaaaaaaaaaa...
                                                                                        # container: 26 bytes
$ jq -n -c 'path(try (.a, error({"z":1})) catch .aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?)'
jq: error (at <unknown>): Invalid path expression near attempt to access element "aaaaaaaaaa... of {"z":1}
                                                                             # key: still 11 bytes
```

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

[#929](https://github.com/rust-works/succinctly/issues/929) found these (a third row,
`flatten("x")` on `[1,2]`, is below) while auditing `EvalError::type_error` wording: real
jq's `@uri`/`@base64` (and every other format string except `@csv`/`@tsv`/`@sh`)
auto-`tostring`s a non-string argument before formatting rather than refusing it outright.
`@base64d` is a related but distinct case — jq *does* still error on `5 | @base64d`
(`string ("5") trailing base64 byte found`), just from attempting to base64-decode the
auto-stringified `"5"` rather than refusing the number up front, so it isn't purely a "jq
doesn't error" gap; matching it needs the same underlying auto-`tostring`-first change
`@uri`/`@base64` do, not just a different error message.

`flatten("x")` on `[1,2]` (`[1,2]` in jq, `expected number, got non-number` here) was the
third #929 row: jq's own `flatten` never validates its depth argument up front, only ever
inspecting it lazily (`!= 0`, `- 1`) once per recursion level, so a non-numeric depth is
silent whenever the input has no nesting deep enough to actually reach a real `- 1`
attempt. [#2755](https://github.com/rust-works/succinctly/issues/2755) closed it:
`flatten`'s depth argument now threads through as the raw value (`OwnedValue`) rather than
an eagerly-converted count, reusing the same `compare_values`/`arith_sub` machinery real
subtraction and comparison already use, so it only ever errors — with jq's own
subtraction-type wording, not a generic message — once nesting deep enough to reach it is
actually present. jq mode only; real yq's own grammar accepts nothing but a bare
non-negative integer literal for `flatten`'s argument, so there is no reachable yq
behaviour for this widening to match, and yq mode keeps its exact pre-#2755 eager
rejection.

Both rows are a slice write walking a path *in place*, and the gap is the same
auto-vivification jq performs everywhere else it writes through a path: `null` grows into
whatever container the path names. Four non-slice rows used to sit above these — `.a = 1`
on `null`, `.a.b = 1` on `{}`, `.[5] = 9` and `.[5] |= 9` on `[1,2]`, all reported as
`Cannot index …`/`index N out of bounds (length M)` where jq builds or pads the container —
closed together by [#486](https://github.com/rust-works/succinctly/issues/486)
(`set_path`/`update_path`, and `=`'s own chain walker — then `get_path_mut`, now
`set_path_steps` since [#1429](https://github.com/rust-works/succinctly/issues/1429) —
now vivify `null` and pad past the array end, the
way `set_value_at_path` already did for `setpath()`) and
[#498](https://github.com/rust-works/succinctly/issues/498) (the write-time negative-index
bounds check now survives `?`, since padding is what makes the positive case succeed rather
than needing `?` to swallow a failure). No write operator produces `index N out of bounds
(length M)` any more; a numeric index past the end is not an error jq raises, so there is no
longer a positive case for succinctly's own wording to cover. The one remaining read-side
producer -- `eval_stage_with_path_context`'s `Expr::Index` arm (that evaluator was
deleted by spine 2416's exit; the surviving routes inherit the rule), reached only when a
path-context builtin (`key`/`parent`/`path`/`file_index`) sits downstream of an out-of-bounds
`.[N]` -- was closed the same way by
[#2213](https://github.com/rust-works/succinctly/issues/2213): the arm now continues with
`null` at the extended path instead of raising, matching plain `.a[5]`. `EvalError::
index_out_of_bounds` (the constructor for this exact wording) is now unused and was removed.

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
closed that by teaching `set_path`/`update_path` and `=`'s chain walker (then
`get_path_mut`, now `set_path_steps`) to vivify the way
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
every other construct uses). Three narrower shapes were tracked below; #2046 closed the
headline example of the first, leaving a smaller residue in its place. Every remaining piece
across all three is **refuse-only** — succinctly declines a filter jq accepts, visibly
(exit 5) and without touching any document. There used to be a fourth that ran the other
way, accepting a filter jq rejects so
that `=`/`|=`/`del()` wrote where jq raises;
[#1466](https://github.com/rust-works/succinctly/issues/1466) closed it, and shape #3 below
is the price it paid. Refusing is the safe direction: [#985](https://github.com/rust-works/succinctly/issues/985)
is the revert that established what the other one costs.

1. **Variable-rooted navigation off a mismatched accumulator** — **closed by
   [#2046](https://github.com/rust-works/succinctly/issues/2046)** for the shape that gave
   this entry its name: `path(. as $x \| foreach (1,2) as $i (0; $x.a; .b))` on
   `{"a":{"b":1},"c":2}` now answers `["a","b"]` ×2, matching jq, instead of refusing.

   The *plain-pipe* half of this shape closed first.
   [#1573](https://github.com/rust-works/succinctly/issues/1573) established that jq carries
   a `(path, value_at_path)` **register** which only navigation advances — a literal never
   moves it, so a `$var` frozen from where it still points steps back onto it — and
   `resolve_seq` threads that register (`PathBranch::register`,
   `reestablishes_register`, `src/jq/eval.rs`). `path(. as $x \| 5 \| $x.a)` on `{"a":1}`
   answers `["a"]` like jq. [`FoldRegister`](../../../src/jq/eval.rs) (the fold's *own*
   register) had no way to reach that same mechanism, because it is seeded from *outside*
   any one `resolve_seq` call — `FoldRegister::resolve` had nothing to hand `resolve_seq` to
   seed its first stage from. #2046 added exactly that: `resolve_seq` takes an extra
   `register: Option<&'a OwnedValue>` parameter, consulted only when seeding its own fan-out
   loop's first branch (mirroring how it already seeds `trackable`/`snapshot`), and
   `FoldRegister::resolve` supplies `self.value` there whenever `UPDATE`/`EXTRACT` is (under
   any wrapping `Paren`) an `Expr::Pipe` — the shape `$x.a`, `$x.a.b`, `$x[0]`, `5 \| $x.a`,
   ... all parse into. Once seeded, the *existing*, already-oracle-verified stage-to-stage
   carrying (`reestablishes_register`/`carry_register`, both from #1573/#2041/#2044) takes
   over unchanged — #2046 adds a new *source* for the carried register, not a new rule for
   recognising it. A bare `$var` with nothing chained after it needed none of this: it was
   already reestablishing directly through `FoldRegister::relocate`'s own `identical()` check.

   **Scope limit, deliberately not closed**: the register is threaded only as far as
   `resolve_seq`'s own seed — a `$var` reference nested *inside* `if`/`try`/`select`/an
   alternative/a construction at UPDATE or EXTRACT's own top level still refuses, because
   `resolve_node`'s own recursive dispatch (the ~20-call-site graph the original design note
   here estimated threading through) was deliberately left untouched. Confirmed live:
   `path(. as $x \| foreach (1) as $i (0; if true then $x.a else 1 end; .))` on
   `{"a":{"b":1},"c":2}` is `["a"]` in jq; succinctly still refuses. Widening this further
   would mean the fuller ~20-call-site threading the original note described, with its own
   oracle matrix per arm — left for a follow-up rather than attempted alongside the
   already-substantial change above.

   The register is also carried **only across stages this resolver can prove did not move
   it** (`cannot_move_register`, `src/jq/eval.rs`) — the same allowlist now gates both
   `resolve_seq`'s plain-pipe carrying *and* #2046's fold-seeded carrying, since both reach
   the identical stage-to-stage machinery. jq's register advances on any `INDEX` its own
   bytecode executes, which includes the ones hidden inside a jq-*defined* builtin (`first`
   is `.[0]`, `add` is `reduce .[] as $x ...`) or a user function body — stages that reach
   the resolver as one opaque computed value. Assuming those left the register alone made
   `path(. as $x \| ([.a]\|first) \| $x)` answer `[]` where jq refuses, and `=`/`|=`/`del()`
   then wrote through the fabricated path, so the allowlist is deliberately narrow and
   everything outside it drops the register. Two shapes jq answers therefore refuse here:
   a construction or interpolation that itself navigates
   (`path(. as $x \| [.a] \| $x)`, `{k:.a}`, `"\(.a)"` — excluded because jq *also* raises
   for a `.a` applied to a computed value, which this resolver never sees inside a
   construction: `path(. as $x \| {k:.a} \| [.a] \| $x)` raises in jq on the second `.a`),
   and a `def` whose body is a constant (`path(. as $x \| (def f: 5; f) \| $x)` — resolving a
   call to its body is not something a syntactic predicate can do from a name). Both are
   refuse-only. (A third shape used to sit here too — an `as` whose bind source navigates —
   but [#2042](https://github.com/rust-works/succinctly/issues/2042) established that jq
   evaluates an `as` source with path tracking suspended, so the source alone never moves the
   register; `cannot_move_register`'s `Expr::As` arm now consults only the body, and
   `path(. as $x \| (.a as $q \| 1) \| $x)` — the shape that used to cost this exclusion — is
   jq's `[]`, matched, not refused. See below.)

   A fourth case, the `null`/`false` half of `register_identical` applied one stage earlier
   than succinctly used to reach — `path(.a \| null)` on `{"a":null}` and `path(.a \| false)`
   on `{"a":false}` are `["a"]` in jq, because the literal never moved the register and a
   `null` is `jv_identical` to a `null` whatever node it came from — is now closed for jq
   mode: [#2044](https://github.com/rust-works/succinctly/issues/2044) widened
   `reestablishes_register` to also cover a still-trackable branch's own step, sourcing the
   register from what `resolve_leaf` records on the exact transition that drops
   trackability. **yq mode deliberately keeps the pre-#2044 refuse-only behavior for this
   shape** rather than gaining the same widening: real yq no-ops a field/index access
   against a scalar (#1181's convention) instead of navigating through it, a rule this
   widening doesn't know about — the `resolve_leaf` fallback this call site ultimately
   reaches for a literal is part of the "dynamic-prepass" family
   [#1419](https://github.com/rust-works/succinctly/issues/1419) already tracks as having
   no yq-mode exception anywhere — so applying jq's own
   write-through answer to yq mode here would silently corrupt data (confirmed live against
   yq v4.53.3: `(null \| .a) = 5` on `null` is a no-op, not the write jq's own `{"a":5}`
   answer would become) rather than merely refuse, which is the direction ADR-0018 never
   permits. Refuse-only for yq mode remains pre-existing and tracked the same way the two
   shapes above are.

   **#2046 review finding, also closed**: `FoldRegister::advance` — which builds `foreach`'s
   per-step `EXTRACT` register from `UPDATE`'s own output — carried its *pre-UPDATE* register
   forward unconditionally whenever `UPDATE`'s own branch ended untracked, without checking
   whether `UPDATE`'s own expression could have moved jq's real register along the way. That
   was already live on `main`, reachable through `FoldRegister::relocate`'s pre-existing
   `identical()` check without needing #2046's own new register-seeding at all — differential
   fuzzing found it independently on a pre-#2046 build (1, 3 and 4 fabrications across three
   4,000-shape seeds, always the same shape): `path(. as $x \| foreach (1,2,3) as $k (null;
   try $x.c catch 1; select(true) \| $x))` on `{"a":"s","c":"s"}` printed `[]` three times
   where jq raises "Invalid path expression with result {\"a\":\"s\",\"c\":\"s\"}" — `try
   $x.c catch 1` attempts (and jq's real bytecode executes) a genuine `INDEX` on `$x` before
   the error is caught, which moves jq's real `value_at_path` regardless of the `catch`.
   Fixed by gating `advance`'s carry-forward on `cannot_move_register(update_expr)`, the same
   allowlist described above — bundled with #2046 rather than filed separately since it sits
   in the exact function #2046's own fix required understanding in full, and #2046's own
   verification requirement (a direction-classified differential fuzz showing zero new
   accept-where-jq-refuses cases) would not have been satisfiable while leaving a *known*
   instance of exactly that class in the same subsystem.

   Separately, a variable bound from a *navigated* position (`.a as $y`) used to carry no
   marker at all, so `path(.a as $y \| reduce (1) as $i (.a; $y))`, jq `["a"]`, refused too —
   and the naive fix (widen `substitute_var_tracked`'s gate to cover it) reopens the
   accept-where-jq-refuses class #1466 closed: `path(.a as $y \| .c \| $y)` on
   `{"a":{"b":1},"c":{"b":1}}` would answer `["c"]` where jq refuses, since
   `register_identical`'s old provenance bit recorded "never rebuilt", not "from *this*
   position", and `OwnedValue` has no node identity to tell the two equal-valued siblings
   apart. **Closed by [#2042](https://github.com/rust-works/succinctly/issues/2042)**, which
   gave the marker a bind-time path: `Expr::TrackedVar`'s payload
   (`Tracked { value, origin }`, `src/jq/expr.rs`) carries an `Origin`, either
   `Origin::Snapshot` (the pre-#2042 value witness `. as $x` still uses, unchanged) or
   `Origin::At { invocation, path }` — the resolver invocation the binding was made in (one
   `path()`/`del()`/assignment-target resolution; a nested `path()` is a second invocation
   with its own root) and the absolute path the source resolved to within it. The path
   register's own absolute position is tracked per-invocation as a `Frame`
   (`src/jq/eval.rs`), threaded alongside `trackable`/`snapshot` through the same recursive
   dispatch; `Frame::certifies` admits an `Origin::At` marker only where its
   `(invocation, path)` matches the frame's own exactly, and admits `Origin::Snapshot`
   unconditionally, leaving that half of `register_identical`'s rule as it was. A `null`/
   `true`/`false` is still admitted by value regardless of origin (jq's own unallocated `jv`
   rule) — a third disjunct alongside the two origin checks, not a replacement for either.

   The bind path itself comes from resolving the `as` source in path position
   (`resolve_bind_source`, `src/jq/eval.rs`) as a witness only: jq evaluates an `as` source
   with path tracking suspended, so the source never moves the register
   (`path(.a as $y \| .b)` is `["b"]`) and never raises a path error of its own — the body
   still resolves against the arm's own ambient value, trackability and frame, exactly as
   before #2042. The witness therefore runs only where it is provably the same computation
   as value evaluation: for a source on the closed pure-navigation grammar
   (`is_pure_navigation` — `.`, `.a`, `.[n]`, a slice, `.[]`, `..`/`recurse`, `getpath` of a
   literal, pipes and commas of those, a literal, `error`), and only when `$var` can reach a
   position the resolver dispatches on in the body (`var_reaches_path_position`; a `$y` used
   only inside `select(..)`, an `if` condition or an operand gains nothing from an origin).
   Any other source — `try`, `?`, `//`, `select`, `if`, `first`, a construction, a builtin —
   binds by value with no origin, exactly as before #2042. The first cut ran the witness on
   every source and relied on its refusal escaping to a value-mode fallback; the review
   showed that a `try`/`?`/`//` *inside* the source caught that refusal (jq never raises it)
   and bound the handler's value — `del(([1] \| try .[0] catch "c") as $y \| .[$y\|tostring])`
   deleted key `"c"` where jq deletes `"1"` — that `getpath`/`..` on a construction raised the
   resolver's other refusal kind as a real error, and that the fallback repeated every side
   effect (`input` consumed twice). On the closed grammar none of those can happen.

   A slice witnesses a node only for a non-empty array (`slice_witnesses_node`): jq's
   `jv_identical` is allocation identity, and a non-empty array slice shares its parent's
   buffer while an empty one (`.a[1:1]`, `.a[5:9]`) is a fresh `jv_array()` and a string slice
   a fresh string — `path(.a[0:2] as $y \| .a[0:2] \| $y)` on `{"a":[1,2,3]}` is
   `["a",{"start":0,"end":2}]` in both, and `.a[1:1]` refuses in both. `cannot_move_register`'s `Expr::As` arm follows the same evidence and now
   consults only the body (see above), so `path(. as $x \| (.a as $q \| 1) \| $x)` — listed
   above as a refuse-only cost of the register-carry allowlist — is now jq's `[]` too: an
   `as` source can no longer block carrying the register through it, because it was never the
   register moving in the first place.

   `path(.a as $y \| .a \| $y)` on `{"a":{"b":1},"c":{"b":1}}` now answers jq's `["a"]`; the
   equal-valued sibling `path(.a as $y \| .c \| $y)` still refuses — exactly the #1466 class
   the frame witness exists to keep closed, now for navigated bindings too. yq mode is
   unchanged: `substitute_bound_var`'s widening is jq-mode only
   ([#2643](https://github.com/rust-works/succinctly/issues/2643)).

   Fourteen rows stay refuse-only, each pinned in `test_path_bind_origin_matrix_refuse_only_2042`
   (`src/jq/eval.rs`) and `scripts/jq-bind-origin-oracle-sweep.sh`'s own `REFUSE_ONLY` list:

   | Filter                                                   | jq                          | Why succinctly still refuses                                                                                                                                                                                              |
   | -------------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
   | `.a as $y \| path(.a \| $y)`                             | `["a"]`                     | value-mode binding — `eval_as` never resolves its source in path position; the accepting-direction twin of #2642                                                                                                          |
   | `path(.a as $y \| .a \| tojson \| fromjson \| $y)`       | `["a"]`                     | `tojson`/`fromjson` are not on `cannot_move_register`'s proven allowlist (#2041)                                                                                                                                          |
   | `path(.a as $y \| def f: $y; .a \| f)`                   | `["a"]`                     | a `def` inside `path()` resolves as an opaque leaf                                                                                                                                                                        |
   | `path(.a as $y \| ([$y] \| .[0]) as $z \| .a \| $z)`     | `["a"]`                     | the source navigates inside a construction, which the resolver refuses where jq's suspended tracking allows it, so it falls back to a plain value                                                                         |
   | `path((.a \| select(.b)) as $y \| .a \| $y)`             | `["a"]`                     | the witness grammar is pure navigation; a `select`-wrapped source binds by value                                                                                                                                          |
   | `path((.a // 1) as $y \| .a \| $y)`                      | `["a"]`                     | same: a `//` source binds by value                                                                                                                                                                                        |
   | `path((if .a then .a else .b end) as $y \| .a \| $y)`    | `["a"]`                     | same: an `if` source binds by value                                                                                                                                                                                       |
   | `path(.a? as $y \| .a \| $y)`                            | `["a"]`                     | a `?` step is a distinct path component, so it never matches the plain spelling on either side (spelling, not node identity)                                                                                              |
   | `path(.[0] as $y \| .[-2] \| $y)` on `[{"b":1},{"b":1}]` | `[-2]`                      | a negative index is stored as written, so `.[-2]` never matches `.[0]`'s path                                                                                                                                             |
   | `path(.a[0:3] as $y \| .a \| $y)` on `{"a":[1,2,3]}`     | `["a"]`                     | jq's full slice *is* the array; the bind path ends in a slice component and `.a` does not                                                                                                                                 |
   | `path(.a as $y \| (.c \| $y \| .b) as $w \| .a.b \| $w)` | `["a","b"]`                 | a marker is re-rooted only at the head of a source (`$y.b as $w`, `(($y \| .b) \| .c) as $w`); elsewhere it is certified against the ambient position                                                                     |
   | `path(.a[1:] as $y \| .a[1:3] \| $y)` on `{"a":[1,2,3]}` | `["a",{"start":1,"end":3}]` | jq's `.a[1:]` and `.a[1:3]` of a 3-array are the same jv (same offset and length); the slice components differ, so the spelling never matches                                                                             |
   | `path(.a as $y \| .a \| try error("x") catch $y)`        | `["a"]`                     | the handler resolves under an unknown frame and a raising `try` stage does not carry the register — pre-existing: `path(. as $x \| try error("x") catch $x)` refuses too (jq `[]`)                                        |
   | `path(.a as $y \| .a \| 5 \| reduce (1) as $i (0; $y))`  | `["a"]`                     | after a literal the register is only *carried*, and a fold whose INIT is untracked seeds its own register from the ambient literal — pre-existing: `path(. as $x \| 5 \| reduce (1) as $i (0; $x))` refuses too (jq `[]`) |

   Two related divergences are pre-existing and out of scope for #2042, tracked separately:
   [#2642](https://github.com/rust-works/succinctly/issues/2642) — the *root* `Origin::Snapshot`
   marker already fabricates a path across a rebuilt-copy boundary (`. as $x \| {a:1} \|
   ($x.a) = 9` on `{"a":1}` writes `{"a":9}`, jq refuses) — the same bug class as #1466,
   reached via a rebuild instead of a sibling; not widened or fixed by #2042. And
   [#2646](https://github.com/rust-works/succinctly/issues/2646) — `first`/`last`/`add`
   navigating inside their own jq-level definitions against a *constructed* value inside
   `path()` never raise, found by `scripts/jq-bind-origin-fuzz.py`'s differential fuzz and
   confirmed pre-existing (reproduces byte-for-byte on the commit before #2042), not caused by
   this change. **#2646 is now fixed** — `builtin_navigation` (`src/jq/eval.rs`) answers what
   each jq-defined navigating builtin indexes, and `resolve_leaf`'s `!trackable` guard raises
   on it. Three groups remain, deliberately, and are divergences in their own right:

   | Still diverging                                                                     | jq 1.7.1                                        | succinctly | Tracked                                                                  |
   | ----------------------------------------------------------------------------------- | ----------------------------------------------- | ---------- | ------------------------------------------------------------------------ |
   | `[path(([1] \| unique) \| empty)]`, and `unique_by`/`map_values`/`with_entries`/`fromstream`, `walk` on an *object*, `ascii_downcase`/`ascii_upcase`, the `match`/`sub`/`gsub` family | raises, naming a **derived** container (`iterate through [[1]]`) | `[]`       | [#2743](https://github.com/rust-works/succinctly/issues/2743) |
   | `[path(([1,2,3] \| nth(2)) \| empty)]`, and `reverse`/`indices`/`index`/`rindex`; `INDEX(f)` and `transpose` | raises; element depends on an argument or `length` | `[]`       | [#2744](https://github.com/rust-works/succinctly/issues/2744) |
   | `[path(([1] \| last(.[])) \| empty)]`, and `isempty(f)`/`any(gen;cond)`/`all(gen;cond)`/`INDEX(gen;f)` | raises — the *argument* navigates                | `[]`       | [#2746](https://github.com/rust-works/succinctly/issues/2746) |

   The rule is **jq-mode only** (ADR-0018): `map`, `any`, `all` and `flatten` are real yq
   builtins, and yq v4.53.3 raises for none of them, so `succinctly yq` does not either.

   [#2072](https://github.com/rust-works/succinctly/issues/2072) supplied the missing
   half of that but deliberately did not spend it here. `Expr::TrackedVar` now carries a
   `BoundVar` (`src/jq/expr.rs`) whose `origin: Option<BindOrigin>` names the node the
   value was bound from — a `DocumentCursor::node_id` plus the owning document's
   `document_token`, or an owned-tree position — so the bind-time node *is* available at
   every `TrackedVar` the resolver meets. The `path()` resolver still consults only
   `BoundVar::tracked`, the `is_identity_passthrough` bit moved onto the node by #2072,
   and never `origin`: a navigated binding therefore resolves exactly as the bare literal
   it used to be spliced as, and every jq-mode row still matches jq 1.7.1. The widening
   this bullet describes is what #2042 is for — it now needs a rule for *when* an origin
   certifies the register (`path(.a as $y | .c | $y)` must keep refusing, since jq's own
   `jv_identical` compares the register's pointer, not the bind site), not a new place to
   get the node from.

   [#2649](https://github.com/rust-works/succinctly/issues/2649) closed the last shape in this
   family with no resolver arm at all: a **destructuring bind**. jq compiles
   `SRC as PATTERN \| BODY` so that `SRC` runs with tracking suspended — #2042's rule, above —
   while every index step *inside* `PATTERN` runs tracked: `gen_object_matcher`/
   `gen_array_matcher` emit ordinary `INDEX` ops, so each step first checks its input is the
   register's own node (`path_intact`) and then moves the register onto the matched member.
   Object entries run in source order, array elements in **reverse** index order (jq nests each
   earlier element inside the later one), and `BODY` then sees the ambient `.` as its input
   while the register sits at the pattern's final position. succinctly had no `Expr::AsPattern`
   arm at all, so every such filter fell to `resolve_leaf`'s catch-all: `path(. as {a:$q} \| $q)`,
   jq's `["a"]`, refused "with result [1,2,3]".

   `walk_pattern` (`src/jq/eval.rs`) now performs the steps and hands back the moved register
   plus one binding per name, carrying an `Origin::At` marker (`Frame::origin_at`) wherever the
   bound value is provably the register's node; `resolve_as_pattern` seeds `BODY` as a pipe
   whose register sits at the pattern's final position while its input is the ambient value
   (`resolve_seq_from_seed`), and the stage rules above do the rest unchanged — `$q`
   re-establishes through `reestablishes_register`, `.a` raises from `resolve_leaf`, a literal
   stays untracked. On `{"a":[1,2,3],"b":{"c":5}}`, `path(. as {a:$q} \| $q)` is `["a"]`,
   `path(. as {b:{c:$r}} \| $r)` and `path(. as {b:$b} \| $b.c)` are `["b","c"]`,
   `path(. as {a:$q} \| .a)` raises jq's own `Invalid path expression near attempt to access
   element "a" of {"a":[1,2,3],"b":{"c":5}}`, `path(. as {a:$q, b:$r} \| .)` raises at the
   *second* entry, `path(.a as [$x,$y] \| .)` raises at element `1` (reverse order), and
   `del(. as {a:$q} \| $q)` / `(. as {a:$q} \| $q[1]) += 10` write through `["a"]`. A `null`
   input still steps, so the value-mode null short-circuit is deliberately not reused here: on
   `{}`, `path(. as {a:{b:$q}} \| $q)` is `["a","b"]`, and on `null`,
   `path(. as {a:$q,b:$r} \| $r)` is `["a","b"]` too, because a `null` parent is `jv_identical`
   to the `null` register. `{$b: P}` is **one** index step in jq's grammar rather than two, so
   `PatternEntry` gained a `bind: Option<String>` and the parser emits a single entry for it:
   `path(. as {$b: {c:$x}} \| $x)` is `["b","c"]`, while the explicit `{b:$m, b:{c:$x}}` still
   performs two steps and refuses at the second, as jq does. The `?//` row the table above used
   to carry (`path(.a as $y \| .a \| . as [$q] ?// $q \| $y)`) is one this closed: both tools now
   answer `["a"]`, and it moved into #2042's *accepting* matrix.

   The arm is **jq mode only.** Real yq v4.53.3's lexer rejects any destructuring pattern at all
   (`. as {a:$q} \| $q` on `a: 1` is `Error: 1:7: lexer: invalid input text "a:$q} \| $q"`), so
   there is no oracle to model; `succinctly yq --jq-extensions 'path(. as {a:$q} \| $q)'` keeps
   its pre-existing fall-through, yielding nothing at exit 0 rather than raising, because the
   generic evaluator's path bridge has no case for a `Pattern` other than a single bare `Var`.
   `test_as_pattern_arm_is_jq_mode_only_2649` pins that outcome as unaffected by this change.

   `?//` alternatives retry as they do in value mode, with one deliberate exception: the
   **artefact guard**. This resolver raises refusals jq never raises — a nested `Pipe` carries
   no register, so a `$q[0]` under an `if` or a `,` raises near-access even though the register
   is standing right there — and retrying on one of those lands on the *wrong* alternative,
   which a write then goes through. `path(. as {a:$q} ?// $z \| if $q then $q[0] else $z end)`
   is `["a",0]` in jq and would otherwise have answered `[]`, and the matching
   `del(. as {a:$q} ?// $z \| if $q then $q[0] else $z end)` on `{"a":[1,2,3]}` is `{"a":[2,3]}`
   in jq and would have deleted through that fabricated `[]`. So a body error of the resolver's
   own two kinds (`UntrackedNavigation`/`InvalidPathExpression`) never retries, and on an
   untracked stage a multi-alternative pattern keeps the old opaque-leaf fall-through, since
   such a stage cannot see the register the first step is compared against. Genuine jq errors
   (`Cannot index X with Y` from the walk or the body, `error(..)`, `Break`) retry as before,
   and branches already emitted stay emitted, as in jq
   (`path(. as {a:$q} ?// {b:$r} \| $q, $r)` prints `["a"]` and then raises, in both). Both
   rows above are pinned as soundness rows in `test_destructuring_moves_path_register_2649`
   (`tests/jq_cli_tests.rs`).

   The guard's price, and the rest of the residue, is **refuse-only** — succinctly declines
   where jq answers, never the reverse, and no row writes. Each is pinned in that same CLI test
   or in `test_path_destructure_matrix_refuses_2649` (`src/jq/eval.rs`), on
   `{"a":[1,2,3],"b":{"c":5}}`:

   | Filter                                            | jq                 | Why succinctly still refuses                                                                                                                 |
   | ------------------------------------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
   | `path(. as {a:$q} \| .a as $z \| $z)`             | `["a"]`            | a bind whose source navigates needs a trackable stage, and the pattern's body stage is untracked by construction                             |
   | `path(. as {a:$q} \| $q as [$x] \| $x)`           | `["a",0]`          | same, for a nested pattern on the bound copy                                                                                                 |
   | `path(. as $x \| 5 \| $x as {a:$q} \| $q)`        | `["a"]`            | a marker-headed source on an untracked stage: the arm cannot see the register the pattern's first step is compared against                   |
   | `path(.b as $y \| .b \| 5 \| $y as {c:$w} \| $w)` | `["b","c"]`        | same                                                                                                                                         |
   | `path(. as {a:$q} ?// $z \| .a)`                  | `["a"]`            | jq's fork catches the body's own near-access error and tries the next alternative; here that error is indistinguishable from a resolver artefact, so the guard refuses instead (a `PATH_END` refusal, by contrast, reaches the loop as a sink stop and does retry)                                                             |
   | `path(. as {a:$q} \| $q[0], $q)`                  | `["a",0]`, `["a"]` | pre-existing: a `$var` nested under `,`/`if` gets no register (the scope limit above) — `path(.a as $y \| .a \| 5 \| $y[0], $y)` refuses too |
   | `path(. as {a:$q} \| select(true) \| $q)`          | `["a"]`            | pre-existing: a `select`/`label`/`first(.)`/`getpath([])` passthrough on an untracked stage re-seeds the carried register from the ambient value — `path(.a as $y \| .a \| 5 \| select(true) \| $y)` refuses too; `if`/`try`/`. as $q \| .` carry it |
   | `path((., .) as {a:$q} \| $q)`                    | `["a"]`, `["a"]`   | the identity premise is decided from the source's spelling (`.`, a certified marker, null/bool by value); an identity-*equivalent* source (`(., .)`, `getpath([])`, `first(recurse)`, a `def` parameter bound to `.`) binds by value and the first step refuses |
   | `path(. as {a:$q} \| $q[($q\|length)-1])`          | `["a",2]`          | a computed index on a marker head resolves the key off the ambient input, so the marker never re-establishes; `$q \| .[length-1]` answers                                                                                   |
   | `path(. as {a:$q} ?// {b:$r} \| if ([3,1]\|sort\|.[0]==1) then $q else $r end)` | `["a"]` | pre-existing: `cannot_move_register` recurses into an `if` condition, so a jq-defined builtin there drops the carried register even though jq compiles the condition as a subexp that can never move it — `path(.a as $y \| .a \| 5 \| if ([3,1]\|sort\|.[0]==1) then $y else . end)` refuses too |

   Two further rows differ only in **wording**, both tools refusing and neither writing:
   `path(. as {a:$q} \| $q as [$h] \| $q)` (jq names the whole value, succinctly names the step)
   and `[path(. as {a:$q} ?// {b:$r} \| $q[0], $r)]` (the two name different containers). Two
   others are pre-existing gaps shared with a plain `as` over an untracked stage, not caused by
   this change: `path(. as {a:$q} \| $q \| first)`, jq `["a",0]`
   ([#2646](https://github.com/rust-works/succinctly/issues/2646)), refuses exactly as
   `path(.a as $y \| .a \| 5 \| $y \| first)` does, and a `def` inside `path()`
   (`path(def f: . as {a:$q} \| $q; f)`, jq `["a"]`) resolves as an opaque leaf — the same `def`
   limitation that table already records.

   One follow-up is filed rather than closed here.
   [#2678](https://github.com/rust-works/succinctly/issues/2678) is a parse gap that predates
   path mode entirely: a computed-key pattern (`. as {("a"): $q} \| $q`, jq `["a"]`) is
   `parse error at position 11: expected identifier, found '('` here, exit 3. The fold's own
   loop variable (`reduce`/`foreach`'s own `as PATTERN`, one level up from a plain `as` bind) is
   closed by [#2676](https://github.com/rust-works/succinctly/issues/2676) — see the fold
   paragraph below.

2. **`?//`-alternatives folds aren't path-tracked at all** (refuse-only) —
   `path(. as $x \| reduce (1) as $y ?// $z (0; $x))` on `{"a":1}` is `[]` in jq; succinctly
   refuses. [#1365](https://github.com/rust-works/succinctly/issues/1365) (`?//`-alternatives
   support for `reduce`/`foreach`'s own `as` clause) landed after `resolve_reduce`/
   `resolve_foreach` were designed, adding a retry-with-rollback matrix
   (`try_reduce_step_alternatives`/`try_foreach_step_alternatives`) neither function threads
   path-tracking through. `resolve_node`'s dispatch arm admits a single pattern alternative
   only (`patterns.len() == 1`) — a bare `$var` in any mode, or, since
   [#2676](https://github.com/rust-works/succinctly/issues/2676), a destructuring
   `Pattern::Object`/`Pattern::Array` in jq mode — and falls to `resolve_leaf`'s catch-all for
   a `?//` chain of any pattern shape.
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

The fold's own **loop-variable destructuring pattern** (`as [$i]`, `as {v:$v}`) is tracked
since [#2676](https://github.com/rust-works/succinctly/issues/2676), one level up from #2649's
own plain-`as` fix: jq compiles a fold's pattern with the same tracked `INDEX` matchers a plain
`. as PATTERN` bind gets, and the fold's SOURCE is itself tracked whenever it resolves through
the path register — so destructuring a register-derived source element moves the register the
same way `. as {..}` does. On `{"a":[1,2,3],"b":{"c":5}}`, `path(foreach .b as {c:$x} (.; .;
$x))` is `["b","c"]` and `path(reduce .b as {c:$x} (.; .))` is `[]`, both now matching jq. The
walk reuses #2649's own `walk_pattern`, seeded at the source element's own register path
(`elem.register_path`, #2031) instead of at `PathPrefix::root()`; its final position becomes a
fresh per-step `FoldRegister` that UPDATE/EXTRACT resolve against, in `resolve_foreach`, in
place of the naive whole-element `step_reg` the bare-`$var` arm still builds. `resolve_reduce`
does not need the analogous override — its own accumulator is checked against the fold's
*persistent*, INIT-seeded register unconditionally (unaffected by SOURCE or pattern shape, a
pre-existing #2031 rule) — so a destructured `$var` there is only ever recognised when it
happens to be `register_identical` to that persistent register, exactly the same test a bare
`$var` bound from a register-derived element already fails whenever its value differs (`path
(reduce .b as $x (.; $x))` refuses the same way `path(reduce .b as {c:$x} (.; $x))` does); the
walk still runs there, purely to reproduce jq's own refusal for the pattern's own step.

**Two residual, refuse-only gaps remain**, both pinned by
`test_reduce_foreach_path_dispatch_falls_back_1440` and
`test_fold_pattern_destructuring_tracks_register_2676` (`src/jq/eval.rs`) plus
`test_fold_destructuring_pattern_moves_path_register_2676` (`tests/jq_cli_tests.rs`):

- **A destructuring pattern whose SOURCE is not register-derived still refuses
  unconditionally**, even when the bound name goes unused downstream — jq's own path_intact
  check compares the pattern's own step against wherever `value_at_path` already is, regardless
  of whether any bound name is ever read: `path(. as $x \| reduce ([1]) as [$i] (0; $x))`
  raises "Invalid path expression near attempt to access element 0 of [1]" in jq 1.7.1 even
  though `$i` goes unused in UPDATE (`$x`, a *different*, outer-bound var). `walk_pattern` is
  seeded from the fold's own persistent register in this case (mirroring the bare-`$var` `None`
  arm), so it still answers correctly through the `null`/`bool` identity exception —
  `path(foreach (null) as {a:$x} (.; .; $x))` is `["a"]` on a `null` document (the ambient
  register is `null` too) but refuses once the document isn't itself null/bool, or once the
  source element isn't (`path(foreach (5) as {a:$x} (.; .; .))` refuses on any document, `$x`
  unused, exactly the message-fidelity gap covered above — the catch-all names the whole fold's
  value rather than the destructuring step jq blames).
- **A multi-element array pattern refuses in jq even when its source *is* the register**, for
  the reverse-order reason #2649 records above —
  `path(foreach .a as [$x,$y] (.; .; $x))` on `{"a":[1,2,3],"b":{"c":5}}` raises at element `0`
  of `[1,2,3]` in jq, and `walk_pattern` reproduces that exactly (verbatim wording, not just the
  verdict), since it's the same function #2649 already uses.

`?//`-alternatives stay on the refuse-only path regardless of pattern shape (item 2 above) —
`patterns.len() > 1` never reaches the walk.

**One further gap is not refuse-only, and is not new here** —
[#2860](https://github.com/rust-works/succinctly/issues/2860): `foreach`'s own EXTRACT
fabricates a tracked path (accept-where-jq-refuses, corrupting a write) when it references the
loop variable again after navigating through a missing object key or other null-absorbing step
on it first — confirmed with a **bare `$var` pattern, no destructuring**, reproducing
byte-for-byte on a pre-#2676 build: `path(foreach .a as $v0 (.; $v0; (.zzz \| $v0)))` on
`{"a":{"b":1}}` is `["a"]` here, "Invalid path expression with result `{"b":1}`" in jq. The
defect lives entirely in `FoldRegister`'s pre-existing UPDATE→EXTRACT chain
(`FoldRegister::advance`/`resolve`, `resolve_seq`'s #2046 carried-register mechanism) — #2676
does not touch that machinery, only gives a destructured loop variable a tracked marker
(`apply_pattern_bindings`) for the first time, which inherits this pre-existing defect the same
splice already had for a bare `$var`. Not fixed here: the root cause needs tracing
`resolve_seq_stage`'s handling of a missing-key/null-absorbing step (real jq tracks a missing
key as a genuine, null-valued navigation — `path({} \| .foo)` is `["foo"]` — which this
resolver's own carried-register fallback does not appear to model correctly), which is a
separate, pre-existing investigation from this issue's own scope.

A destructuring pattern's own **computed key** (`{(EXPR): P}`, or an interpolated string
`{"\(EXPR)": P}`) parses and evaluates since
[#2677](https://github.com/rust-works/succinctly/issues/2677) — `PatternEntry.key` widened from
a plain `String` to `ObjectKey` (the same type object construction already used), and
`extract_pattern_bindings` (value mode)/`walk_pattern` (path mode) resolve the key expression
against the pattern's own current node, using the same "Cannot index `<type>` with `<type>`"
wording jq's own `INDEX` bytecode gives for any key, computed or literal — confirmed live in
both positions, including nested computed keys and through `|=`/`del()`. **Scoped to a key
expression that yields exactly one string** — the common case (`{(.k): $q}`, a string
interpolation, which by construction can only ever yield one value). A key expression that is
itself a multi-output *generator* (`{("a","b"): $q}` — real jq fans out one full pattern-match,
and one full run of the surrounding body, per key it yields: `{"a":1,"b":2} \| . as
{("a","b"):$q} \| $q` is `1` then `2`) refuses clearly instead — `extract_pattern_bindings`
itself models the full cartesian-fold generator semantics real jq has, but every one of its
eight call sites (across both evaluators, plus `reduce`/`foreach`'s own pre-loop substitution-
matrix builders) collapses back to a single result via `extract_single_pattern_binding`, since
fanning out correctly needs each call site's own `?//`-alternative-retry/fold-matrix machinery
threaded through, not just the core function — not yet done. The same refusal covers a key
expression producing **zero** outputs (`{(empty): $q}`, where real jq legitimately binds
nothing and the body never runs, `[. as {(empty):$q} \| $q]` is `[]`) and one interrupted by
`break`/`halt`, rather than risking a silently wrong answer at any of the eight sites
individually. `test_pattern_computed_key_evaluates_2677`/
`test_pattern_computed_key_multi_output_refuses_cleanly_2677` (`tests/jq_cli_tests.rs`) pin both
halves.

A related, narrower gap the fix surfaced and closed along the way: `map_subexprs`
(`src/jq/walk.rs`) — the shared tree-rewrite primitive `install_def_calls`/`bind_def` and
`substitute_var_impl` both build on — cloned a pattern's own `patterns` field verbatim in its
`Reduce`/`Foreach`/`AsPattern` arms rather than descending into it, a premise ("a pattern holds
only destructuring names, never an `Expr`") that held before #2677 and silently broke once a
computed key gave a pattern entry a real `Expr` to hold. Concretely, `def f: "a"; . as {(f):$q}
\| $q` wrongly raised "undefined function: f/0" even though `f` *is* defined, and an outer
`$var` referenced inside a computed key was left unsubstituted — both fixed by a new
`map_pattern_subexprs` helper. Three siblings with the analogous gap — `any_subexpr`,
`node_reads_ambient`, and `resolve::check` (`src/jq/resolve.rs`) not descending into a computed
key either — are, confirmed live, in the safe direction only (a missed static-analysis
optimization, or jq's own compile-time "undefined function" check surfacing one stage later as
a runtime error instead of at parse time) and are left for #2677's own remaining
walker-invariant audit.

**Fixed by [#1467](https://github.com/rust-works/succinctly/issues/1467),
[#1872](https://github.com/rust-works/succinctly/issues/1872),
[#2031](https://github.com/rust-works/succinctly/issues/2031) and
[#2235](https://github.com/rust-works/succinctly/issues/2235):** the fold's **source
expression** is now evaluated the same way jq evaluates it — `resolve_reduce`/
`resolve_foreach` route the source through `drive_fold_source`, which resolves it with
tracking on via `resolve_node_sink`, one element at a time as the fold consumes them, but
only when the source's own AST contains a real navigation step
(`Field`/`Index`/`Slice`/`Iterate`) anywhere — checked via `any_subexpr` — since only those
can ever reach one of `resolve_node`'s raising arms in the first place. jq
evaluates the source "with tracking on" and fails only where it navigates *through* an
untrackable value, so `reduce (1,2) / (.a) / (.[]) / (keys) / (range(2)) as $i (0; $x)` all
resolve in both, and `reduce (keys[]) as $k (.; .)` raises "near attempt to iterate through"
in both too (live-verified against jq 1.7.1, both the `path(...)` read side and the
`(reduce ...) = 9` write side).

A source with no navigation step (`range(n)`, `keys`, a literal, or critically
`input`/`inputs`) skips the resolver entirely and is driven by value through
`eval_each_owned`, the same demand-driven producer the value evaluator uses. That gate is
not an optimization: an earlier, ungated version of #1467's check evaluated an
`input`/`inputs`-sourced fold *twice* — once for the check, once for the real value —
silently desynchronizing the two evaluations' shared input-reader position, so the fold ran
against the *second* input document while reporting an error that named the first (`reduce
input as $x (0; $x)` over `10\n20\n30\n`: real jq names `10`, the ungated version named
`20`). Caught in code review before merging. Since #2235 the gate is also what keeps
`inputs` lazy: the resolver's leaf collects a generator before delivering it, where
value-mode `each_inputs` pulls one document per demand (`[try path(foreach inputs as $i (.;
error("u"))) catch .], input` over `1 2 3` answers `["u"]` then `2` in both).

A source that *does* navigate is resolved **once**, in jq mode, and — when every resolved
branch is *untracked* — those branches' values are the fold's real values; `Keep::AtMost`
makes `resolve_leaf`'s general case keep every output rather than #987's first one. That
closed both of #1467's original costs:

1. A navigating source's outputs fire their side effects exactly as often as in jq: `. as $x
   | path(reduce (.[] | stderr) as $i (0; $x))` on `[1]` writes `1` once in both (measured
   directly, not by line count — `stderr` writes no trailing newline), where #1467's
   two-pass shape wrote it twice.
2. `foreach` keeps the outputs a source legitimately streamed before the element that
   raises: `path(foreach (1,2,keys[]) as $k (.; .))` on `{"a":1}` streams `[]` twice before
   raising, matching jq. The escape is applied after the drive, once every element before
   it has been folded. `reduce`'s output is single-shot, so it still emits nothing — but
   since #2235 its prefix steps run first, as jq's do: `path(reduce (1,2,error("s")) as $i
   (.; stderr))` on `{"a":1}` writes the state twice and then raises `s` in both.

Keeping every output also made the check itself accurate. A stop-after-first resolver could
miss the navigation entirely when it happened on a later output of the same leaf:
`path(foreach (range(3) | (., if . == 2 then .a else empty end)) as $k (.; .))` on `{"a":1}`
reported the untracked `Cannot index number with string "a"` where jq reports the tracked
`near attempt to access element "a" of 2`; both now agree.

**Since [#2235](https://github.com/rust-works/succinctly/issues/2235) the source is pulled by
demand, and the resolver's stream is the fold's stream.** Until then `drive_fold_source`'s
predecessor collected every resolved branch before the fold ran a single step, so a source's
side effects all fired before the first step could refuse: `. as $x | path(foreach (.[] |
stderr) as $i (0; $x))` on `[1,2,3]` wrote `123` where jq writes `1` once and raises on the
first step's own untrackable emission. Three things had to move together. The source is
resolved through `resolve_node_sink`, each element folded inside the sink before the next is
pulled; the folds themselves emit through a sink, so `path()`'s terminal check
(`resolve_terminal`) refuses the first untracked output before the fold pulls again; and the
old fallback that re-evaluated an *erroring* source untracked (any escape that was neither
`Halt` nor an untracked-navigation error) is gone. That fallback was itself the larger
divergence: it double-fired side effects (`path(foreach ((.[]|stderr|empty), error("s")) as
$i (.; .))` on `[1,2]` wrote `1212` for jq's `12`) and, by discarding the per-step register,
turned a jq refusal into an answer — `[label $out | path(foreach (.[], break $out) as $i (.;
.))]` on `[1,2]` printed `[[],[]]` at exit 0 where jq raises "Invalid path expression with
result [1,2]", a write-side hazard. Every escape the resolver raises is now the fold's
escape, which holds the resolver's arms to a stricter contract: an arm that escapes must
already have delivered everything jq would have bound (`Expr::Alternative` forwarded its
left side's escape prefix *unfiltered*, falsy branches included, and was fixed alongside).

A *trackable* branch means the source is itself a path expression in jq's eyes, and — since
[#2031](https://github.com/rust-works/succinctly/issues/2031) — that is no longer treated as
an all-or-nothing gate on the whole source stream. Every resolved branch becomes a real fold
value regardless of its own trackability, and a trackable branch's own path rides along
(`FoldSourceValue::register_path`) so `resolve_reduce`/`resolve_foreach` can seed a
**per-step register from that element's own navigated position** while resolving *that*
element's own UPDATE call, standing in for the fold's persistent, INIT-seeded register —
reproducing jq's own single shared path register, which SOURCE's own navigation moves just as
INIT's does. `foreach` applies this per UPDATE emission (its own doc comment on
`resolve_foreach` has the full derivation, several confirmed-live examples included);
`reduce` does not — its existing final re-entry-boundary re-check against the fold's
*persistent* register (unrelated to #2031, predates it) already reproduces jq's own behavior
there, and an earlier draft of this fix that also overrode `reduce`'s per-step register was
caught regressing exactly that case by the differential fuzzer
(`path(reduce (.[]) as $k (.; .))` on `{"a":[1,2]}` is `[]` in jq; the over-broad draft made
it raise). A cruder, still-earlier attempt — streaming the resolved source values without
threading the per-step register at all — is what originally established the "all-untracked"
restriction this section used to describe: randomised differential fuzzing against jq 1.7.1
found it regressed eleven of four thousand generated folds, every one of them a source with a
trackable branch. #2031's fix keeps the values-streaming behavior that regression required but
adds the register threading that closes it instead of reopening it; re-run against the same
600-case seed (`.ai/scratch/sweep-path-fold-differential.py`), the "jq errors, succinctly
succeeds" count for this whole fold-source area dropped from 67 to 3, with no increase in any
other divergence category.

Five residual divergences remain in this area:

- **A fold source whose navigation is discarded by a later non-navigating stage still
  clobbers jq's real path register, undetected.** `path(foreach (.a|tostring) as $k (.; .a))`
  on `{"a":1,"b":{"c":2}}` raises `Invalid path expression near attempt to access element "a"
  of {"a":1,"b":{"c":2}}` in jq; succinctly prints `["a"]`. `.a` genuinely navigates (moving
  jq's real register) before `tostring` — not itself a path primitive — discards that
  provenance from the *source element's own* final value; `drive_fold_source` only inspects
  each element's final `PathBranch.trackable`, so a source like this looks exactly like an
  ordinary computed value to it. Distinct from — and not closed by — #2031's fix, confirmed via
  `git stash` A/B against the pre-#2031 build on `main` too. Tracked as
  [#2159](https://github.com/rust-works/succinctly/issues/2159).

- **A generator the resolver reaches only through an eager arm is still collected before
  its first element is folded.** `drive_fold_source` pulls by demand only as far as
  `resolve_node_sink`'s lazy arms reach; `resolve_leaf`'s general case, an `if`/`select`
  condition (`eval_owned_multi_keep_partial`), an `as` source (`resolve_bind_source`) and a
  fold nested inside the source all materialize their own generator first. So `path(foreach
  (inputs | .a) as $i (.; .))` drains every remaining input document where jq consumes one,
  and `path(reduce (if (.[]|stderr) then 1 else 2 end) as $i (.; error("u")))` on
  `[1,2,3]` writes `123` for jq's `1`. Same cause as [#2466](https://github.com/rust-works/succinctly/pull/2466)'s
  own left-out list: `resolve_leaf` has no sink form. The consumer side has the same edge:
  `path()` itself collects every path before its own consumer sees one, so a bound *outside*
  it cannot stop the fold (`[limit(1; path(foreach (1 as $x ?// $y | (stderr|1)) as $v (.;
  .)))]` is `[[],[]]` with two writes in jq — its `limit` break is retried by the source's
  `?//` and the retried output lands past the bound — and `[[]]` with one write here).
  Tracked as [#2694](https://github.com/rust-works/succinctly/issues/2694).
- **`recurse(f)`/`recurse(f; cond)` still collects one node's own `f` in full.**
  `resolve_recurse_sink` (#2235) streams each visited node to a bounded consumer as soon as
  it is popped, and defers `f`/`cond` for a node until its own delivery is accepted — so
  `path(limit(1; recurse(if (.|debug) < 3 then .+1 else empty end)))` now runs `debug` zero
  times in both jq and here, where the pre-#2235 collecting version ran it for every node up
  to `RECURSE_MAX_ITEMS` regardless of the bound. What remains: `f` itself is still resolved
  for an accepted node via `resolve_against_cow`, not streamed, so a multi-output `f`'s later
  outputs fire even when only the first is ever consumed — confirmed live,
  `path(limit(2; recurse((.a|debug), (.b|debug))))` on `{"a":1,"b":2}` writes `debug` for
  `.a` only in jq (the bound is satisfied by `.a`'s own self-emission before `.b` is ever
  asked for); both fire here. Same underlying cause as the bullet above — `resolve_against_cow`
  has no sink form either — narrower in practice since it only over-fires a node's own
  later `f` outputs, not an entire subtree. Tracked as
  [#2693](https://github.com/rust-works/succinctly/issues/2693). Bare `..`/`recurse`, which
  predates #2235 and was not part of that migration at all, has the same gap one level up —
  tracked separately as [#2696](https://github.com/rust-works/succinctly/issues/2696).
- **`E[K]` evaluates its target once where jq re-runs it per key.** jq compiles `E[K]` as
  `K as $k | E | .[$k]`, so a side effect in `E` fires once per output of `K`; here it fires
  once total — `[(.[] | stderr)[("a","b")]?]` on `[1,2]` writes `1212` in jq and `12` here.
  Affects both the value path (`eval_index_expr`) and the resolver
  (`resolve_index_expr`); tracked as
  [#2032](https://github.com/rust-works/succinctly/issues/2032).
- **Yq mode keeps #1467's original two-pass shape.** Real yq's lexer rejects `reduce`,
  `foreach` *and* `path` outright (confirmed live against yq v4.53.3), so `succinctly yq`
  has no oracle for a fold's values, and `resolve_index_expr`'s yq-only `scalar_noop` arm
  would hand the *unchanged target* to the fold where the evaluator raises. `succinctly yq`
  therefore still path-checks with `Keep::First` and then drives its values by value
  through `eval_each_owned`; only the jq-mode value reuse is new.

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
two representations, not a new inconsistency #1642 introduces. Since
[#2103](https://github.com/rust-works/succinctly/issues/2103) that is the stated rule rather
than an observation: **an undecodable key is echoed as its raw source bytes wherever the
value is never materialized, and as `key_display_string`'s fallback (the source `\` doubled)
wherever it is.** Which of the two a filter gets follows from whether it materializes, never
from which evaluator route it took — see the #2103 entry under "Real-time stdout/stderr
interleaving".

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
and the one used in `src/jq/mod.rs`'s own module-doc example — with its own separate
`to_owned`/`to_owned_lossy`/`effective_len`/`effective_fields` family (#1989 renamed the
checked conversion to the short `to_owned` and the lossy one to `to_owned_lossy`, so that a
bare `to_owned` at a new call site is the checked default; the names below are the current
ones). Two different checks are in play, and their coverage differs:

- **The #1194/#1642 decode-failure and structural-key policy** already reaches most of this
  file's materializing builtins via `to_owned`/`to_owned_at_depth` (`in(xs)`,
  `ltrimstr`/`rtrimstr`, the sort family, `min`/`max`/`unique`/`group_by`, `add`, `join`,
  `flatten`, and every assignment RHS, among others) — not the wholesale gap the previous
  revision of this paragraph implied.
- **#1902 widened this to `. as $var`/`reduce`/`foreach`.** Their bound/INIT/input value and
  body-output conversions switched from a lossy fold (`to_owned_lossy`, and the multi-value
  promotion path this file no longer keeps an unchecked twin of) to the checked
  `to_owned`/`promote_borrowed` — so a #1194 malformed member or #1642
  collision key that used to be silently dropped (`reduce . as $x (0; .+1)` on `{"a":1,"b"}`
  used to succeed with `1`) now raises there too, same as the builtins listed above. Not a new
  divergence unique to #1902: this is `to_owned`'s own established contract from
  #1755 onward, at three call sites this one just hadn't named yet (documented here per
  #1934 item 6, which also closed a related gap: five bare, unchecked
  `Error`/`Partial` arms across these same three functions' `input`/INIT streams didn't
  exclude a genuine decode failure from `optional`'s suppression the way `finish_fork`
  already does — an internal-consistency fix, not a further widening of this policy).
- **#2188 fixed `fromstream(f)`'s own event-collecting fallback.** `collect_owned`'s
  `One`/`Many` arms use the lossy `to_owned_lossy`, so `builtin_fromstream`'s
  `result.collect_owned()` fallback silently substituted `""` for an undecodable event leaf
  instead of raising — found via a research audit for #1989 that classified every bare
  `to_owned(` call site in this file. Fixed by routing through the pre-existing checked
  `stream_outputs` (already had the exact `(Vec<OwnedValue>, Option<Control>)` shape
  the call site destructures into) rather than adding a new function.
  `builtin_truncate_stream` has the identical `collect_owned` call shape but is safe by
  construction, not just untested: its `stream_expr` is evaluated against the same ambient
  value that also becomes `depth`, and any value complex enough for `stream_expr` to reach a
  nested undecodable string through is necessarily non-scalar, which always outranks an `Int`
  path length in jq's ordering — so every event is unconditionally dropped before a corrupted
  leaf could reach output.
- **#1800 changed *when* the check fires for `contains`/`inside`, not whether.** Both
  materialize the primary input before fanning their argument out, and that conversion used
  to `return` its `Err` immediately — so an undecodable input preempted the argument
  expression's own error or side effect even where jq's `. as $x | b as $y | ($x |
  contains($y))` desugar would have reached the argument first. The `Result` is now consulted
  inside the fan-out body instead, which makes an argument-side escape win
  (`contains(error("boom"), 1)` raises `boom`) and, in the zero-candidate case
  (`contains(empty)`), leaves the decode failure undemanded and the call empty — matching what
  real jq answers for a decodable input, and pinned by
  `test_builtin_contains_empty_argument_never_demands_the_input_1800`. No oracle exists for
  the undecodable variant itself: jq substitutes U+FFFD upstream, so this whole scenario is
  succinctly's own semi-indexing artifact and is reachable through the library API only, not
  the CLI. `in`/`IN` have the identical shape but evaluate their argument against the decoded
  input itself, so they keep the eager early return (tracked as #2202).
- **The #1677 malformed-`,`/`:`-delimiter check is the narrower gap.**
  `to_owned_at_depth` itself never calls `key_delimiter_ok`/`value_delimiter_ok`, so
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

## A trailing comma after a genuine last child, scoped to one materializer (#2243)

`[1,]`/`{"a":1,}` -- a stray trailing `,` after the container's own *last real
member*, with every member present otherwise well-formed -- is a distinct
malformed-delimiter shape from #1677's missing/doubled `,`/`:` between two
members above, and from #2211's `[,]`/`{,}` (a stray `,` with **no** real
member at all). #2243 closed it for `eval_generic::to_owned_cursor_at_depth`
(the materializer behind what was then the `evaluate_bytes_lazy`/
non-cursor-transparent path -- `if`/arithmetic/function calls, anything
`expr_is_cursor_transparent` answered `false` for, same gate #2211 used; both
are gone since #2103, and the same materializer now backs `eval_single`'s
wildcard bridge for the shapes without a native streaming arm) by adding
`DocumentCursor::trailing_element_gap_ok`/`DocumentValue::scalar_text_end` to
the shared trait system ([src/jq/document.rs](../../../src/jq/document.rs)),
implemented for JSON by reusing the CLI printer's own existing
`trailing_gap_ok`/`scalar_end_pos` primitives (#1676/#1576) rather than a
third hand-copied pair:

```
$ echo '[1,]'      | jq  -c 'if true then . else . end'   # parse error, exit 5
$ echo '[1,]'      | sjq -c 'if true then . else . end'   # exit 5 now (was: [1], exit 0)
$ echo '{"a":1,}'  | jq  -c 'if true then . else . end'   # parse error, exit 5
$ echo '{"a":1,}'  | sjq -c 'if true then . else . end'   # exit 5 now (was: {"a":1}, exit 0)
```

**Residual gap: a *container-typed* last child still passes through.**
`scalar_text_end` only knows how to answer a scalar's own end position from
its start (`start + raw_bytes().len()`, or a fixed literal length for
`Bool`/`Null`) -- a container's own end position is not derivable from its
start alone without a further cursor walk, so it answers `None` there by
design, and `trailing_element_gap_ok` treats that the same as "can't
determine, skip" (the same convention every other gap check in this family
already follows):

```
$ echo '[1,[2,3],]'          | jq  -c '.'   # parse error, exit 5
$ echo '[1,[2,3],]'          | sjq -c '.'   # [1,[2,3]], exit 0 -- still open
$ echo '{"a":1,"b":[1,2],}'  | jq  -c '.'   # parse error, exit 5
$ echo '{"a":1,"b":[1,2],}'  | sjq -c '.'   # {"a":1,"b":[1,2]}, exit 0 -- still open
```

This is not new: the cursor-transparent `.` path (`stream_json_pretty`'s own
`scalar_end_pos`, #1676) already has the identical gap for the identical
reason, confirmed live against `main` prior to #2243. Pinned by
`test_jq_lazy_path_trailing_comma_after_container_last_child_still_a_known_gap_2243`
in [tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs) rather than left
untested.

**Much narrower reach than #1677/#2211's own checks, tracked as three
follow-ups rather than folded in here** (same "one materializer at a time"
practice #2243's own issue text cites for not folding into #2211):

- **#2261 (fixed, mostly)**: every *cursor-transparent* fast path -- `.[]`,
  `keys`/`keys_unsorted`/`to_entries`, bare `.a`/`.[0]` field/index access,
  `length` -- took a route through `eval_generic.rs` that never reached
  `to_owned_cursor_at_depth` at all, so this fix did not reach jq's most
  common query idioms; only the non-cursor-transparent shape #2243's own
  repro uses (`if`/arithmetic/function calls) was covered. #2261 closed
  this for every one of those idioms except a genuinely O(1) object-key
  lookup (`keys_unsorted[0]`) -- see its own section below for the full
  accounting, including a case (`.[0]`/`Expr::Index` on an array) this
  issue's own repro assumed would stay open but turned out to be free.
- **#2262 (fixed)**: the three sibling materializers named above --
  `eval.rs`'s own `to_owned_at_depth` (behind this crate's documented public
  `eval()` API), `eval_generic.rs`'s own cursor-less `to_owned_at_depth`, and
  `yq_runner.rs`'s `to_owned_canonicalizing_numbers_at_depth` (behind
  `succinctly yq --slurp`/`--eval-all`/`--inplace --input-format json`) --
  now all share #2243's `DocumentCursor::trailing_element_gap_ok` (moved to
  `document.rs` as `pub` for this) via the same "retain the last real
  child's own cursor, check it after the loop" shape #2211/#2243 already
  established, closing the trailing-comma case (`[1,]`, `{"a":1,}`) for all
  three. #2211's `container_gap_ok` (`[,]`, `{,}`) remains open in all
  three, and is expected to stay that way: unlike `to_owned_cursor_at_depth`,
  none of the three is ever given a cursor for the *container itself* --
  only a bare `value`, so once a container's child walk is exhausted there
  is nothing left to find its opening bracket from. `eval.rs`'s and
  `eval_generic.rs`'s cursor-less `to_owned_at_depth` are both library-API-
  only (not reachable through the shipped CLI with raw untrusted text);
  `yq_runner.rs`'s was live and CLI-reachable, and review of its own fix
  (filed as #2276) found the *fast-path* routes `--slurp`/`--inplace` each
  have for a plain identity/M2-streamable filter bypass this materializer
  entirely (they parse via `YamlIndex`/`mark_json_sourced` instead, whose
  flow-sequence grammar legitimately allows a trailing `,`) -- confirmed
  live as a real, `--inplace`-destructive gap (`succinctly yq -i '.'` on
  `[1,]` rewrote the file instead of refusing), closed by declining those
  two fast paths entirely for JSON-sourced input (`any_input_is_json` in
  `yq_runner.rs`), since their own `else` arm already routes back through
  this now-fixed materializer. Declining the fast path this way surfaced
  a second, independent gap in review: it silently traded `YamlIndex`'s
  128-deep parse-time nesting guard for `to_owned_canonicalizing_numbers_
  at_depth`'s own looser, panicking 256-deep one, so JSON nested 129-255
  levels deep went from "rejected at parse time" to "silently accepted" --
  closed by `parse_input_m2_parity`'s own depth-128 pre-check, verified
  against a pre-fix build at every boundary value (127/128/129/255/256/
  257). See [docs/compliance/yq/limitations.md](../yq/limitations.md)'s
  own `#1975`/`#2262` section for the full account, including a
  `YamlCursor`-trait-override approach that was tried first and found not
  to work (M2 streams cursors directly without ever materializing through
  the trait methods it would have added). The *plain* stdout fast path
  (`can_json_fast_path`/`can_yaml_fast_path`) has the identical underlying
  gap but was left alone: its own `else` arm is `evaluate_yaml_direct_filtered`
  (#1398's cursor-native evaluator, used for *every* ordinary
  `--input-format json` filter, not just fast-path-eligible ones), which
  shares the same validation hole, so declining the fast path there would
  cost M2's performance with no correctness gain -- tracked as a further,
  broader follow-up rather than folded into #2276.

  **Update (#2358)**: "expected to stay that way" above turned out to be true only for
  the true top level, not the permanent limitation it was assumed to be. Every recursive
  call into `eval_generic.rs`'s `to_owned_at_depth` already resolves the child's own
  cursor for an unrelated reason (`field.value_cursor`, `elem_cursor`) and simply
  discarded it -- threading that cursor through as a new function parameter lets
  `container_tail_gap_ok` close `container_gap_ok`'s `{,}`/`[,]` check for every *nested*
  container the same way `to_owned_cursor_at_depth` always could. Only the true top level
  (the one caller with no cursor to give in the first place, `to_owned`'s own depth-0
  entry point) keeps the gap. Separately, `lazy.rs`'s own independent JSON-cursor
  materializer (`cursor_to_owned_at_depth`, not one of the three named above -- it was
  never in the "no container cursor" situation at all) had its own, narrower gap of the
  same shape: it always holds a real cursor but had never adopted
  `container_tail_gap_ok` in place of a hand-inlined `container_gap_ok`-only check, so
  its trailing-comma case (`[1,]`, `{"a":1,}`) was unchecked at *every* depth, not just
  nested ones -- now closed unconditionally there, with no top-level caveat.
  `eval.rs`'s own `to_owned_at_depth` and `yq_runner.rs`'s
  `to_owned_canonicalizing_numbers_at_depth` likely have the identical "recursive calls
  already resolve a cursor" opportunity `eval_generic.rs` did, but neither was touched by
  #2358, whose scope named only that one function; tracked as #2403 rather than assumed
  to close the same way without checking.

  **Update (#2403)**: confirmed and closed. Both functions had the identical shape --
  `elem_cursor`/`field.value_cursor` already resolved in the recursive call and discarded
  before the tail check -- and now thread it through the same way, via a shared
  `tail_gap_ok` helper (`document.rs`) extracted in review rather than a third and fourth
  hand-copy of the `container_tail_gap_ok`/`child_tail_gap_ok` dispatch. Only the true top
  level (`to_owned`'s own depth-0 entry point, no cursor to give) keeps the gap, matching
  `eval_generic.rs`'s own residual scope above.

  **Update (#2781, yq mode)**: `to_owned_canonicalizing_numbers`'s own depth-0 entry point
  -- the true-top-level exception this note originally named alongside `to_owned`'s -- is
  closed. `parse_input`'s JSON arm (`yq_runner.rs`, the only caller reaching this
  materializer, for `--slurp`/`--eval-all`/`--inplace --input-format json`) already held the
  root cursor and simply was not threading it through; `to_owned_canonicalizing_numbers`
  is now folded into `to_owned_canonicalizing_numbers_at_depth`, whose `cursor` parameter is
  a bare `&V::Cursor` (no longer `Option`) since every call site in that file has one.
  `to_owned`'s own top-level gap (jq mode, `eval.rs`/`eval_generic.rs`) is untouched by
  #2781 -- currently not independently reachable there (parser-level validation intercepts
  a top-level `[,]`/`{,}` before `to_owned` runs on every route probed), but that is
  incidental protection from a different layer, not a guarantee `to_owned_at_depth`'s own
  signature enforces the way `to_owned_canonicalizing_numbers_at_depth`'s now does.
- **#2263**: `jq_runner.rs` still carries its own independent, hand-copied
  `trailing_gap_ok`/`scalar_end_pos` pair rather than the new trait methods --
  a cleanup, not a behavior gap, but the same "duplicated predicates diverge
  silently" shape (#106) this fix was written to reduce, not add to.

## The trailing-comma check reaches jq's own most common idioms (#2261)

#2243 closed the trailing-stray-comma-after-a-real-last-child shape (`[1,]`,
`{"a":1,}`) for `to_owned_cursor_at_depth` -- but that materializer backs
only the *non-cursor-transparent* route (`if`/arithmetic/function calls).
The far more common idioms -- `.[]`, `keys`/`keys_unsorted`/`to_entries`,
bare `.a`, `length`, `.[0]` -- each take their own native, cursor-carrying
arm in `eval_generic.rs` and never call it at all:

```
$ echo '[1,]'     | jq  -c '.[]'          # parse error, exit 5
$ echo '[1,]'     | sjq -c '.[]'          # exit 5 now (was: 1, exit 0)
$ echo '[1,]'     | jq  -c 'length'       # parse error, exit 5
$ echo '[1,]'     | sjq -c 'length'       # exit 5 now (was: 1, exit 0)
$ echo '[1,]'     | jq  -c '.[0]'         # parse error, exit 5
$ echo '[1,]'     | sjq -c '.[0]'         # exit 5 now (was: 1, exit 0)
$ echo '{"a":1,}' | jq  -c 'keys'         # parse error, exit 5
$ echo '{"a":1,}' | sjq -c 'keys'         # exit 5 now (was: ["a"], exit 0)
$ echo '{"a":1,}' | jq  -c '.a'           # parse error, exit 5
$ echo '{"a":1,}' | sjq -c '.a'           # exit 5 now (was: 1, exit 0)
```

**Every one of these turned out to be free or near-free**, riding a walk the
arm already had to perform for an unrelated reason -- with exactly one
deliberate exception. The full per-path accounting:

**`.[]` (arrays).** Both the demand-aware `each_lazy_array_iterate_sink`
(`eval_generic.rs`, #1597) and the eager `DocumentElements::collect_cursors_checked`
(`document.rs`, #1677) already walk every element's own cursor to check its
*leading* `,` -- retaining the last cursor seen and checking
`trailing_element_gap_ok` once after the loop exhausts costs nothing new.
`each_lazy_array_iterate_sink`'s own pre-existing early-exit divergence
(#1629-shaped: a truncating consumer like `first(.[])` never walks past its
own stopping point) still applies unchanged -- the new check runs only on
`Flow::Exhausted`, the same gate `is_malformed`-style checks elsewhere in
this file already use.

**`length` (arrays) and `.[0]`/`Expr::Index` (arrays).** A new
`DocumentElements::len_checked` (the array counterpart of the object side's
long-standing `effective_len_checked`) walks with `uncons_cursor` instead of
`len`'s plain `uncons`, checking the same #1677 leading-gap plus the new
trailing one, and `Builtin::Length`'s array arm now calls it. `.[0]`
(`Expr::Index` in `eval_generic.rs`) turned out to need the identical fix,
for a reason this issue's own description did not anticipate: resolving
*any* array index -- positive or negative -- already calls `.len()` first
(to normalize a negative index against the array's length, and to raise
yq's own out-of-range error), so switching that call to `len_checked` closes
the gap on every index, not just negative ones. **This reverses the
plan the rest of this section still follows for the object-key
counterpart below** (`keys_unsorted[0]`, which really is O(1) and stays
open) -- the array case looked like the same shape from the issue text
alone, and only reading `Expr::Index`'s own body (not the issue's
characterization of it) surfaced that `len()` was already unconditional.

**`keys`/`keys_unsorted` (objects, both bare and piped: `keys[]`,
`keys_unsorted[]`, `keys_unsorted | last`, a negative `keys_unsorted[n]`).**
Every one of these walks `DistinctKeyCursors` (`document.rs`), which now
tracks `last_key_cursor` -- the textually last field's own *key* cursor, in
raw document order -- alongside its existing `ended_unpaired`/
`delimiter_fault` bookkeeping, and exposes it via a new
`DistinctKeyCursors::trailing_gap_ok(close_char)`. The check itself needs
the last field's *value* cursor, not its key cursor, to compare against
`}` -- resolved via `key_cursor.next_sibling()`, an O(1) BP hop mirroring
how `JsonFields::uncons` itself derives a value cursor from a key cursor
(`let value_cursor = key_cursor.next_sibling()?;`), run once here rather
than during the walk.

This needed real care around a duplicate key under jq's default collapse
rule ("first position, last value"). `DistinctKeyCursors`' own *yielded*
order is first-occurrence order, which for `{"a":1,"b":2,"a":3}` is `a`
(now carrying the *second* `a`'s cursor, once collapse detects and
resolves the repeat), then `b` -- so naively tracking "the last cursor this
loop yielded" would land on `b`'s value (`2`), and checking the gap after
it against `}` would see the perfectly ordinary `,"a":3}` that follows and
wrongly reject this well-formed document. The actual fix: `next()`'s
non-collapsed branch updates `last_key_cursor` on every raw field it
examines, in document order; the moment a repeat is confirmed,
`collapse_confirmed_repeat`'s own from-scratch re-walk (which already
covers the whole object, needed anyway to build the exact collapsed list)
hands back its own `last_key_cursor` too, and `next()` overwrites with
that authoritative answer rather than whatever the raw walk had reached so
far. An earlier version of this fix (caught reviewing this same issue,
before it shipped) used the naive "last yielded" cursor and would have
regressed exactly this case -- pinned by
`test_jq_cursor_transparent_fast_paths_wellformed_unaffected_2261`'s own
duplicate-key rows in [tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs).

The bare-`keys`/`keys_unsorted` materializer (`document.rs::effective_keys`,
which already walks `DistinctKeyCursors` for an unrelated reason -- decoding
every key's display spelling) and several call sites each needed their own
one-line addition once `DistinctKeyCursors::trailing_gap_ok` existed:
`bail_if_keys_malformed` (`jq_runner.rs`, the CLI's own `JqValue::LazyKeysArray`
writer behind a *bare* `keys_unsorted`), `eval_generic.rs`'s own
`walk_distinct_keys_checked`/`Expr::Builtin(Builtin::Last)` arm (covering
`keys_unsorted[]`/`keys_unsorted[-1]` and `keys_unsorted | last` respectively,
reached through `eval_single`'s eager path when #2261 landed, since
`keys_unsorted` alone was not one of the shapes `jq_runner.rs`'s M2 gate
admitted), and `each_lazy_keys_iterate_sink`'s own `!sorted` branch (the
demand-aware sink, reached back then only once a pipe's *first* stage already
descended off the root -- `.x | keys_unsorted[]`, whose `Expr::Field` first
stage the gate treated as descending and so admitted everything after
unconditionally, taking the M2/`eval_each_with_cursor` route instead of the
eager one). Since #2103 there is no gate and every M2 filter takes that
demand-aware route, so both spellings reach the sink; each site keeps its own
check because each is still reachable from some caller.

`stream_lazy_keys_json` (`src/jq/stream.rs`) also received the same
one-line addition, but **turns out not to be reachable from either shipped
CLI today, the identical shape `test_stream_lazy_keys_honors_collapse_1514`'s
own review already established for this exact function**: `succinctly jq`
excludes bare `Builtin::KeysUnsorted` from its own M2 *output* gate
unconditionally (`m2_json_fallback_safe`, `jq_runner.rs` -- a *different*
gate from the eager-vs-demand-aware *evaluation* one described above, which
#2103 removed; this one decides which writer prints the result and is still
in place), regardless of AST shape, so this function is never invoked from
`succinctly jq` at all.
`succinctly yq --input-format json` has no such exclusion and does reach
this function, but only with YAML-sourced `fields` (JSON parses through
`YamlIndex` there, per #1975/#2262's own account), whose `trailing_gap_ok`
always answers `true` by design (YAML's flow-mapping grammar legitimately
allows a trailing `,`) -- so a genuinely JSON-sourced `fields` never reaches
this function from either CLI as things stand today. A first draft of this
fix's own test suite wrongly attributed a CLI-level test
(`.x | keys_unsorted[]`) to this function; code review, disabling each
candidate check in turn and re-running the exact query, found it actually
exercises `each_lazy_keys_iterate_sink` above instead -- retitled as
`test_jq_descended_pipe_prefix_reaches_demand_aware_keys_sink_2261` in
[tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs), and
`stream_lazy_keys_json`'s own fix is pinned instead by two direct unit tests
in `src/jq/stream.rs` itself (`test_stream_lazy_keys_raises_on_trailing_comma_2261`/
`test_stream_lazy_keys_wellformed_unaffected_by_trailing_comma_check_2261`),
calling the function the same way `test_stream_lazy_keys_honors_collapse_1514`
already does.

**Bare `.a`/`.nonexistent` field access (`JsonFields::find_cursor`,
`src/json/light.rs`).** This issue's own text characterized this as a
genuine O(1) lookup, the same shape #1629 already established should stay
unchecked for cost reasons -- **verifying that assumption rather than
trusting it found it was wrong**. `find_cursor` must resolve
last-duplicate-key-wins semantics (#1251), so its `while` loop always walks
every field in the object regardless of `name` or where a match sits --
never returning early on a match, because a later same-named field could
still supersede it. That means it is already paying the O(n) cost this
issue assumed only `.[]`/`keys`/`length` paid, and the trailing-gap check
rides along for free: track the last field's value cursor (regardless of
match), check it once after the loop, and raise for `.a` *and*
`.nonexistent` alike on `{"a":1,}` -- matching real jq, which can't parse
the document at all, so every field access into it raises, not just the
ones that happen to touch a real key.

**`to_entries` (both arrays and objects).** The array arm already used
`collect_cursors_checked` and so was fixed for free by that function's own
change above. The object arm needed one more piece: it already resolves
every field's value cursor (`to_owned_cursor` is called on each one
regardless), so retaining the last one costs nothing -- but it iterates the
*already-collapsed* list `effective_fields` returns, which (per the
`keys`/`keys_unsorted` account above) can list a different, earlier field
last once a duplicate key collapses. A new `effective_fields_with_raw_last`
(`document.rs`) hands back the *raw, pre-collapse* walk's own last field's
cursor alongside the (possibly reordered) collapsed list `to_entries`
still iterates to build its output -- free, since `all_fields()`'s `Vec`
already exists in raw document order before any collapsing runs, so
reading its last element costs nothing beyond the call already made.

### What stayed open, and why (the genuine O(1) exception)

**`keys_unsorted[0]`** (a *positive* index into an object's key list,
`fold_lazy_keys_stage`'s `Expr::Index { idx: 0, .. }` arm) is the one case
in this issue that really does match #1629's own "would cost strictly more
than the answer" reasoning: it reads only `fields.uncons_key()`'s first
field and returns, never touching the rest of the object -- the same
positional-access shape #1629 itself already left unchecked for
`Builtin::First`/a positive `Expr::Index` on `LazyKeys`, for the identical
reason (checking would mean walking the whole object purely to validate
it, undoing the point of the O(1) lookup). Confirmed still open, and
pinned rather than silently left uncovered, by
`test_jq_keys_unsorted_positive_index_trailing_comma_remains_a_known_gap_2261`
in [tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs).

**#2211's own sibling shape (`[,]`, `{,}` -- a stray comma with *zero* real
elements/fields) -- closed by #2594.** It was open through every path this
section fixed, for the same reason it remains open through
`to_owned_at_depth`'s cursor-less callers (#2262's own account above):
`container_gap_ok` needs a cursor to the *container itself* to find its
opening bracket, and none of
`each_lazy_array_iterate_sink`/`collect_cursors_checked`/`len_checked`
(elements only), `effective_fields_checked`/`effective_keys`/
`DistinctKeyCursors`/`find_cursor` (fields only) ever receive one -- only
per-child cursors, once a child exists to hold one.

That is still true of the walks, and #2594 did not change any of them. What
it changed is where the check runs: every one of those walks is dispatched
from an `eval_single`/`eval_builtin`/`path_step_generic` arm that already
holds `cursor: Option<V::Cursor>` for the container (for type-name
diagnostics), so the arm now calls `empty_fields_tail_gap_ok`/
`empty_elements_tail_gap_ok` (`document.rs`) ahead of the walk that cannot.
The observation recorded here -- that `eval_each_generic`'s `Expr::Iterate`
arm had a container cursor sitting unused at its call site -- generalised:
so did all the others.

Per-arm rather than one guard at the top of either dispatch function,
because #2173's closed terms (`empty`, `true and true`, `1 + 1`) dispatch
through them too and must answer exactly as `empty` does on every document,
malformed ones included; a hoisted guard fails
`test_ambient_validation_agrees_with_bridge_2476`. Pinned by
`test_jq_cursor_transparent_fast_paths_empty_container_stray_comma_now_raises_2594`,
with `test_jq_genuinely_empty_containers_unaffected_2594` and
`test_jq_closed_terms_unaffected_by_empty_container_check_2594` for the two
things the fix must *not* do.

Still open, unchanged by #2594: the same shape reached through
`to_owned_at_depth`'s cursor-less top-level callers (#2262 above) and
through `collect_paths_generic`'s value-domain recursion for a container
nested *inside* a `paths`/`leaf_paths` walk -- neither has a container
cursor to check against.

**`length` (objects) -- fixed by #2307/#2311.** Not one of this issue's own
five repros, originally left open here with reasoning that turned out to be
wrong: this entry used to claim closing the gap would need either paying to
resolve an extra value cursor per call, or a "larger architectural change."
Neither was true. `census`/`checked_len` (`document.rs`) already retain the
last-visited key's *cursor* at zero extra cost (same as
`DistinctKeyCursors`/`contains_checked`); once the walk finishes naturally
that cursor's own `next_sibling()` is an O(1) BP hop to its value -- the
exact same "free" derivation `DocumentFields::uncons` already performs to
turn a key cursor into a value cursor in the first place, not a fresh
resolve. `{"a":1,} | length` now raises in both modes (`census`'s
`collapse: true` path and `checked_len`'s `collapse: false` one),
consolidated into a shared `last_field_trailing_gap_ok` helper alongside
`DistinctKeyCursors::trailing_gap_ok`/`contains_checked`'s own identical
hop (code review on #2311, found by mistake while writing #2293's own
parity test -- neither PR's original scope named this).

**Residual gap, inherited from `trailing_element_gap_ok` itself, not new
here:** a trailing comma is only caught when the object's real last
field's *value* is a scalar. When it's a container (array/object, empty or
not), `trailing_element_gap_ok`'s own unconditional `is_container()` early
return means no text position is ever resolved to check against -- the
same #2243 residual every other #2261-family fix in this file already has
(`array` `length`/`len_checked`, `has(key)`, `keys`, ...), now shared by
object `length` too. `{"a":{"x":1},} | length` still answers `1` instead
of raising. Pinned as still open by
`test_jq_length_object_trailing_comma_container_last_value_still_a_known_gap_2307`.

### Partial output before the error, on the two genuinely streaming writers

Two of the newly-checked paths can write real, already-confirmed-good
output to stdout before the trailing-comma fault surfaces -- not a new
divergence, but the same one already pinned for `limit(3;.[])` on
`[1,2,,4]` (see "A truncating consumer of a plain array `.[]` skips a
malformed comma it never needed", above): a streaming writer emits each
element/key as it confirms it, and only learns about a trailing fault once
the walk exhausts.

```
$ echo '[1,]'     | jq  -c '.[]'            # (parses nothing) exit 5
$ echo '[1,]'     | sjq -c '.[]'            # 1, then error, exit 5
$ echo '{"a":1,}' | jq  -c 'keys_unsorted'   # (parses nothing) exit 5
$ echo '{"a":1,}' | sjq -c 'keys_unsorted'   # ["a" (no closing bracket), then error, exit 5
```

Real jq's parser is atomic (whole-document parse before any evaluation, so
a malformed document produces no output at all); succinctly's semi-index
validates incrementally as each element/writer step confirms itself good.
Pinned by
`test_jq_lazy_array_iterate_trailing_comma_streams_confirmed_prefix_first_2261`
and
`test_jq_lazy_keys_array_trailing_comma_leaves_truncated_bracket_2261`.
Every other path this section fixed (`length`, `to_entries`, `keys`, `.a`,
`keys_unsorted[]`/`| last`/`[-1]`) resolves to a single owned result or a
fully-collected `Vec` before printing anything, so a mid-walk failure there
never leaks a partial value.

### PR #2291 code review: eleven more sibling paths, found by a systematic sweep

Code review on the PR carrying the section above found and live-confirmed
five more sibling paths sharing the exact "already walks `.len()`/every
field, so the check rides free" shape used throughout this section, missed
in the first pass: `has(idx)` on arrays, `has(key)` on objects,
`keys`/`keys_unsorted` on *arrays* (the object arm above was fixed; this
array arm two branches below it, returning `GenericResult::LazyIndexRange`,
was not), computed/dynamic index access (`E[K]`, e.g. `.[0,1]`/`.[$i]` --
`index_one_generic`, distinct from the literal-index `Expr::Index` arm
fixed above), and `last`/`.[-1]` (`Builtin::Last`; unlike `first`, which is
genuine O(1) via `get_cursor(0)` and stays the documented exception per
#1629's own precedent this section already established).

A follow-up systematic sweep -- every `DocumentElements`/`DocumentFields`
binding in `eval_generic.rs`, checked for an unchecked `.len()` or an
unchecked `collect_cursors()` sitting alongside an already-checked sibling
-- turned up **six more real, live, jq-oracle-backed bugs** of the second
shape: `path(.[])`'s array arm (`path_step_generic`) had drifted onto the
unchecked `collect_cursors()` even though its own object-arm sibling three
lines above already used the checked `effective_fields_checked`; and
`reverse`/`sort`/`sort_by`/`unique`/`unique_by`/`min`/`min_by`/`max`/`max_by`
all resolved every element via the unchecked `collect_cursors()` before
this fix, despite `collect_cursors_checked` (fixed for `.[]` itself) having
existed the whole time. `shuffle`/`pivot` share the identical code
(`collect_cursors()` feeding `to_owned_all_cursors`) but have no jq oracle
(`pivot/0 is not defined`/`shuffle/0 is not defined` in real jq 1.7.1) --
fixed anyway for internal consistency with every other builtin in this
list, not for jq parity.

```
$ printf '[1,2,3,]' | jq          'has(0)'        # parse error, exit 5
$ printf '[1,2,3,]' | sjq         'has(0)'        # exit 5 now (was: true, exit 0)
$ printf '{"a":1,}' | jq          'has("a")'      # parse error, exit 5
$ printf '{"a":1,}' | sjq         'has("a")'      # exit 5 now (was: true, exit 0)
$ printf '[1,2,3,]' | jq       -c 'keys'          # parse error, exit 5
$ printf '[1,2,3,]' | sjq      -c 'keys'          # exit 5 now (was: [0,1,2], exit 0)
$ printf '[1,2,3,]' | jq       -c '.[0,1]'        # parse error, exit 5
$ printf '[1,2,3,]' | sjq      -c '.[0,1]'        # exit 5 now (was: 1\n2, exit 0)
$ printf '[1,2,3,]' | jq          'last'          # parse error, exit 5
$ printf '[1,2,3,]' | sjq         'last'          # exit 5 now (was: 3, exit 0)
$ printf '[1,2,3,]' | jq       -c '[path(.[])]'   # parse error, exit 5
$ printf '[1,2,3,]' | sjq      -c '[path(.[])]'   # exit 5 now (was: [[0],[1],[2]], exit 0)
$ printf '[1,2,3,]' | jq          'reverse'       # parse error, exit 5
$ printf '[1,2,3,]' | sjq         'reverse'       # exit 5 now (was: [3,2,1], exit 0)
```

(`sort`/`unique`/`min`/`max`/`sort_by`/`unique_by`/`min_by`/`max_by` all show
the identical shape as `reverse`.)

**`has(key)` on objects is the one fix in this whole issue that is not
free**, and needed two drafts to get right. `DocumentFields::contains`
early-exits the instant it finds a match -- a documented, deliberate
design (#1739) this fix does not get to ignore: `has(key)`'s existence
question and the trailing-gap question are two different questions that
can only both be answered by walking to the object's true end, and a match
can sit anywhere. A first draft dropped the early exit entirely (walk
fully regardless, matching `keys`'s own shape) and measured a real **~4x**
regression on `has("k0")` (the object's very first key) over a
1,000,000-key well-formed object: ~0.03s → ~0.12s, interleaved A/B, release
build, three reps. The shipped fix instead resolves the trailing gap from
the *matched* key's own cursor alone -- `key_cursor.next_sibling()` is the
matched field's value (an O(1) BP hop, no walk), and *that* cursor's own
`next_sibling()` answers "is there another field after this one" for free.
Only when the match *is* the object's real last field (`{"a":1,} |
has("a")`, this issue's own repro) does checking cost anything at all, and
even then it costs one more O(1) hop, not a walk. A match that is **not**
the last field takes the exact #1629/#1770-established "early exit
legitimately misses a later fault" trade every other truncating consumer
in this file already takes:

```
$ printf '{"a":1,"b":2,}' | jq  'has("a")'   # parse error, exit 5
$ printf '{"a":1,"b":2,}' | sjq 'has("a")'   # true, exit 0 -- known gap, matches "a" before "b"'s own comma is ever reached
$ printf '{"a":1,"b":2,}' | sjq 'has("z")'   # parse error, exit 5 -- a non-match still walks to the true end, so it still catches it
```

A first-draft version of `has(key)`'s CLI test also wrongly attributed a
result to `stream_lazy_keys_json` a second time (`pivot` on
`[{},{},]`) -- this one for a different reason than the earlier
`stream_lazy_keys_json` misattribution above: `pivot` only accepts an
array of arrays/objects, so its own last element is always container-typed,
which the #2243 "container-typed last child still passes through" residual
gap (documented earlier in this file) already leaves open regardless of
this fix. Corrected to use a *leading*-gap malformed input instead
(`[{"a":1},,{"a":2}]`), which the identical `collect_cursors`-to-`_checked`
fix also closes and which the residual container gap does not affect.

Every fix above except `has(key)` on objects was confirmed to show **no
measurable difference** in an interleaved A/B (a build from immediately
before this round vs. after, three reps each, release profile) at 2M
array elements / 1M object keys -- consistent with each one already
walking the whole container for an unrelated reason before this fix, the
same "free" pattern established throughout this file.

**Confirmed still open, and out of scope for this round**, from the same
sweep, for reasons unrelated to `.len()`/`collect_cursors()` (each is
missing *every* gap check, not specifically the trailing one, a broader
and differently-shaped problem than this section's own fixes):

- **`to_owned_with_comments_at_depth`** (`eval_generic.rs`) -- a *fourth*
  copy of the `to_owned_at_depth`/`to_owned_cursor_at_depth` materializer
  family #2262 already found three of, backing the write path's
  comment/anchor-preserving conversion (`succinctly yq`'s own `=`/`|=`/
  `del()` machinery). Has no gap checks of any kind. Confirmed **not**
  reachable from either shipped CLI with a genuinely JSON-sourced cursor
  today: `succinctly jq`'s own write path uses the already-fixed
  `to_owned_at_depth`/`to_owned_cursor_at_depth` instead (confirmed live:
  `[1,2,3,] | .[0] = 99` already correctly raises), and `succinctly yq
  --input-format json` parses through `YamlIndex` rather than `JsonIndex`
  (per #1975/#2262's own account), so its own trailing-gap check would be
  a permanent no-op there regardless. The identical "library-API-only,
  inert for genuine JSON input via either CLI" shape already established
  for `stream_lazy_keys_json` above. Recommended as a `#2262`-scoped
  follow-up (a fourth materializer copy), not folded in here.
- **`LazySource::advance`'s `Self::Elements`/`Self::Values` arms**
  (`eval_generic.rs`), the lazy pull behind `map(f)`/`select(f)`/etc:
  genuinely live and CLI-reachable, confirmed both for this issue's own
  trailing-comma shape and for the older, broader #1677 leading-gap shape
  that predates #2261 entirely:
  ```
  $ printf '[1,,3]'   | jq  -c 'map(.+1)'   # parse error, exit 5 (#1677, predates this issue)
  $ printf '[1,,3]'   | sjq -c 'map(.+1)'   # [2,4], exit 0 -- pre-existing, not new
  $ printf '[1,2,3,]' | jq  -c 'map(.+1)'   # parse error, exit 5
  $ printf '[1,2,3,]' | sjq -c 'map(.+1)'   # [2,3,4], exit 0
  ```
  Not attempted here: this is `map(f)`'s own performance-tuned hot pull
  loop (#1565/#1599), and adding gap-checking to it is a materially larger,
  riskier change than swapping an already-checked sibling in for an
  unchecked one -- the same "no free ride" caution `has(key)` above
  required, but for a colder, more central path. Recommended as its own,
  separate follow-up issue.

- **`src/jq/eval.rs`'s parallel evaluator** -- a second, independently-tested
  evaluator behind the crate's public `jq::eval`/`jq::eval_lenient`/
  `jq::eval_owned_with_file_index` entry points (see
  `tests/jq_evaluator_parity_tests.rs`), not `eval_generic.rs`, backed `has(key)`
  (object arm, a raw `.any()` with zero gap checks), `length` (array arm
  `.count()`, object arm bare `effective_len`), and `keys`/`keys_unsorted`
  (array arm `.count()`) -- the identical unchecked shape this whole issue
  chain fixes elsewhere, confirmed by code review on this PR (round 2).
  **Confirmed not reachable from either shipped CLI**: `succinctly jq`'s
  `jq_runner.rs` never calls into `eval.rs` at all (only `eval_generic.rs`);
  `succinctly yq`'s one call site (`yq_runner.rs`'s `evaluate_input`, and
  `eval_owned_with_file_index` for `--eval-all`) always hands `eval.rs` a
  cursor built by re-serializing an *already-materialized* `OwnedValue`
  (`to_json_for_reindex`) into a fresh index -- a value that reached that
  point only by surviving the CLI's own (already-fixed) parse/materialize
  step, so the re-serialized text this function actually navigates can never
  contain a stray trailing comma regardless of query. Verified live: `succinctly
  yq --input-format json --slurp/--eval-all` on both a top-level and a nested
  `[1,2,3,] | has(0)`/`length`/`keys` already raises correctly, before
  `eval.rs`'s own unchecked arms are ever reached. The identical
  "library-API-only, inert for genuine JSON input via either CLI" shape
  already established for `to_owned_with_comments_at_depth` above -- a caller
  of the public library API who builds their own cursor directly from raw,
  unvalidated document bytes (bypassing both CLIs' own materialization step)
  would still observe the gap.

  **Fixed by #2293**: `has_one_key`'s object arm now uses `contains_checked`
  (also closing the #2288 non-string-key gap item 3 below used to track
  separately -- `contains_checked` already checks for that as part of its
  own walk), its array arm and `builtin_length`/`builtin_keys`'s array arms
  now use `len_checked`, and `builtin_length`'s object arm now uses
  `effective_len_checked`. Parity pinned in
  `tests/jq_evaluator_parity_tests.rs`'s
  `test_parity_eval_rs_trailing_comma_sibling_gaps_2293`. Still inert for
  both CLIs today, same reachability analysis as above -- real only for a
  library consumer building a cursor directly from raw bytes. The broader
  "systematic sweep of the rest of `eval.rs`" #2293's own issue also
  suggested (dozens of further `StandardJson::Array`/`StandardJson::Object`
  match sites in that file, most already covered by other routes) remains
  unaddressed and would need its own follow-up issue if pursued.

  Round-2 code review on #2311 found two more real gaps, neither folded into
  that PR: `has_one_key`'s array arm called `numeric_key_to_array_index`
  *before* `len_checked`, short-circuiting to `Bool(false)` on `None` (a NaN
  key in jq mode, any Float key in yq mode) without ever running the gap
  check -- fixed in #2311 itself by reordering to match `eval_generic.rs`'s
  own unconditional-`len_checked`-first shape exactly. Two further siblings
  sharing the identical unchecked-`.count()` shape were found, both now
  **fixed**: `builtin_keys` used to never collapse duplicate keys in
  *either* mode (it had no `S: EvalSemantics` parameter to derive
  `S::COLLAPSE_DUPLICATE_KEYS` from, unlike `eval_generic.rs`'s own
  `Keys`/`KeysUnsorted` arms) -- filed as #2313 and fixed there, by adding
  that parameter and its one call site's turbofish; `eval.rs`'s copy of
  `keys`/`keys_unsorted` now agrees with `eval_generic.rs`'s on this shape
  in both modes. `count_elements` (backs `get_element_at_index`'s index
  resolution, `.foo[N]`/`.foo[-N]`) -- filed as #2312, fixed by
  restructuring `get_element_at_index` to match `eval_generic.rs`'s own
  `Expr::Index` arm exactly: `count_elements`/`len_checked()` now runs
  unconditionally, before branching on sign (a first draft only checked
  the negative branch, leaving `.[0]` on a malformed array unchecked --
  found by #2312's own code review), and the negative-still-negative
  decision (#2254) reuses the same shared `yq_negative_index_check` helper
  `eval_generic.rs` already calls, rather than a bespoke error type.
  Parity pinned in `test_parity_negative_index_trailing_comma_2312`
  (negative) and `test_parity_positive_index_trailing_comma_2312`
  (positive).

  Writing that parity test also surfaced a further, unrelated, **live and
  CLI-reachable** bug shared by *both* evaluators: `{"a":1,} | length` in jq
  mode (`collapse: true`) silently returned `1` instead of raising --
  `effective_len_checked`'s `census` path never called `trailing_gap_ok` at
  all, unlike every sibling helper in this file. Not fixed by #2293, which
  only brought `eval.rs` in line with `eval_generic.rs`'s *existing*
  behavior, not a bug both evaluators already shared -- filed and fixed
  separately as #2307/#2316; see this doc's own "`length` (objects)"
  entry above for the fix and its residual gap.

Two further leads from the same sweep were investigated and **not**
reproduced live (so not reported as confirmed bugs, only as ruled out) at
the time: `push_generic_document_validation_error` (`if COND then ... end`
on a container condition) and `owned_from_standard_json_at_depth` (the
`input`/`inputs`-with-cursor-metadata-builtins bridge, #1504) both lack
their own internal gap checks by inspection, but every constructed repro
against each (`{"a":[1,2,3,]} | if .a then ... end`;
`[1,2,3,] | [inputs, line]` with `-n`) already raised correctly, meaning
something upstream of either function already validates for the shapes
tried. Not chased further given no live reproduction *of those specific
repros* — **update (#2349): a live, CLI-reachable gap in
`push_generic_document_validation_error` was found and fixed after all**,
just not via `if...then...end` (whose own `Control::from(cond)` path
apparently validates upstream, matching what this entry found). `select`,
`sort_by`/`unique_by`/`min_by`/`max_by`, and `path()` all route through
this same function without that upstream validation, and a trailing-comma/
missing-colon/stray-comma corruption in a subtree the query only ever
validates (never emits) silently passed — `{"c":{"a":1,},"t":5} |
select(.c) | .t` answered `5` instead of raising, confirmed live. (`path()` left
that list again in #2168, which decided that naming a position should not
validate what is inside it, and `select` left it again in #2692, which decided that
*testing* a value does not read it either; the `sort_by` family keeps the checks this fix
added.) See
this doc's "The validate-only traversal..." entry in `CHANGELOG.md` for
the fix; the underlying lesson stands even though this specific
conclusion didn't: a function lacking its own checks by inspection can
still be live-exploitable through a caller this investigation's own
repro set didn't try.

**Update (#2400): `owned_from_standard_json_at_depth`'s half re-checked with a wider net,
this time confirmed structurally unreachable, not just "no live repro found yet".**
#2349's own finding above ("a function lacking its own checks by inspection can still be
live-exploitable through a caller this investigation's own repro set didn't try") made this
half worth re-checking properly rather than leaving it on the same "not reproducible (so
far)" footing #2349 turned out to be wrong about. Traced every live call site
(`input`/`inputs`'s cursor-metadata-builtin bridge, and `query_result_to_generic`'s
`path()`/`getpath`/`key`/`reduce`/`foreach` callers) and confirmed each one feeds this
function bytes that are already clean by construction: `input`/`inputs` validates upfront
in the input-reading pipeline before this function ever runs, and every other caller passes
a fresh `to_json_for_reindex` re-serialization of an already-decoded `OwnedValue` -- text
succinctly's own serializer produces, which by construction never carries a malformed
delimiter. This is the structural difference from `push_generic_document_validation_error`'s
real gap: that function walks the *original* document cursor directly, with no round-trip
in between, so genuine source corruption reaches it; nothing here does. Recorded directly
on the function (`src/jq/eval_generic.rs`) so a future re-check starts from "confirmed
unreachable, here's why" rather than repeating this trace from scratch.

The wider repro net used to check this did surface real jq divergences --
`{"c":{"a":1,},"t":5} | .t as $x | $x` (also `reduce`/`foreach`, also
`.t | path(.)`) answers `5`/`[]` where real jq's own eager, whole-document parser rejects
the document outright before evaluation starts -- but these are the well-known,
already-documented semi-indexing trade-off (`CLAUDE.md`'s "Semi-indexing performs minimal
validation compared to full parsers" note): the query never touches the corrupted `.c` at
all, so its being unvalidated is the intended lazy-validation behavior, not a gap in this
function or any specific evaluator path. Not filed as a new issue on that basis.

## `has(key)`/`contains()` and `find()` get two more pre-existing gaps closed (#2288)

Code review of #1995/PR #2287 found two further, unrelated gaps, both predating that PR:

**1. `has(key)`/`contains()` didn't raise on a non-string sibling key at all** (not the
trailing-comma shape #2261 above fixes -- a genuinely different malformed-JSON class,
#1194/#1995's own `key_is_malformed`):

```
$ echo '{"a":1,123:2}' | jq  -c 'has("a")'   # parse error, exit 5
$ echo '{"a":1,123:2}' | sjq -c 'has("a")'   # true, exit 0 -- was WRONG before this fix
```

Fixed the same way #2261's own `has(key)` fix was shaped: `contains_checked`'s existing
per-key walk (already checking `,`/`:` delimiters for #1677) now also checks
`key_is_malformed` on every key it visits -- free, since it's the identical walk, not an
added pass. That "visits" qualifier matters: **a malformed key strictly *after* the match
is the same class of accepted early-exit gap #2261 already established for the trailing
comma**, and for the identical reason -- there is no O(1) shortcut from the matched key's
own cursor to "is there a malformed key anywhere else in this object," unlike the trailing
gap (a single, fixed, cheaply-reachable position). Closing it in general would mean
walking to the object's true end on every `has()` call, the exact ~4x regression #2261's
own `has(key)` fix (documented above) already measured and rejected.

```
$ echo '{123:1,"a":2}' | jq  -c 'has("a")'   # parse error, exit 5
$ echo '{123:1,"a":2}' | sjq -c 'has("a")'   # parse error, exit 5 -- fixed: bad key visited before the match
$ echo '{"a":1,123:2}' | jq  -c 'has("a")'   # parse error, exit 5
$ echo '{"a":1,123:2}' | sjq -c 'has("a")'   # true, exit 0 -- still a known gap: bad key visited after the match
$ echo '{"z":1,123:2}' | sjq -c 'has("a")'   # parse error, exit 5 -- no match, so the walk reaches 123 regardless
```

So this issue's own headline repro (`{"a":1,123:2} | has("a")`) is **not** fully closed --
only the general class (a non-string key existing at all) is now caught in every case
except "the match itself happens to come first." Recorded here rather than left as an
unstated side effect of the fix, matching how #2261's own `has(key)` fix above documents
its identical trade-off.

Code review on this fix also caught that the naive shape (a separate `key_is_malformed`
call alongside the pre-existing `key_display_string` call) paid for **two** independent
key decodes per key -- `key_is_malformed(k) == key_display_string_kind(k).is_none()` by
construction, so the fix calls `key_display_string_kind` once and derives both "is this key
malformed" and "does it match `name`" from that single result, rather than adding a second
decode pass on top of the one `contains`/`contains_checked` already paid.

**2. `JsonFields::find` (unlike its `find_cursor` sibling) had neither the #1677 delimiter
check nor the #2261 trailing-comma check at all** -- a missing `:` before the winning
occurrence's value, or a trailing stray comma after the object's real last field, both used
to slip through silently (`find("a")` on `{"a" 1}` returned `Ok(Some(1))` with no error,
and `find("a")` on `{"a":1,}` also returned `Ok(Some(1))`, where `find_cursor` on the
identical documents already correctly raised for both). Fixed by threading the same
`preceding_gap_ok`/`trailing_element_gap_ok` checks `find_cursor` already runs -- the
`,`/`:` check deferred to the actual winning occurrence (last-duplicate-key-wins), the
trailing-comma check unconditional on whether `name` matched anything, exactly mirroring
`find_cursor`'s own two checks and reusing them rather than re-deriving either (#106). A
first draft of this PR ported only the `,`/`:` check and missed the trailing-comma one
entirely -- a second code-review round on this same PR caught the residual asymmetry
between two functions this file's own doc comments describe as sharing "the same
last-duplicate-key-wins semantics."

**Confirmed not reachable from either shipped CLI**, the identical "library-API-only, inert
for genuine JSON input via either CLI" shape #2293 already established for `eval.rs`'s
whole parallel evaluator: `find`'s only production caller is `eval.rs`'s `find_field`/
`index_object_by_name` (`.field` navigation in that evaluator), and `eval.rs` itself is
only ever reached, from either CLI, via `succinctly yq`'s `evaluate_input`/
`eval_owned_with_file_index` -- both of which hand it a cursor re-serialized from an
already-materialized `OwnedValue`, which can never contain a missing `:` or a stray
trailing `,` regardless of query (confirmed live: `succinctly yq --input-format json
--slurp/--eval-all` on both `{"a" 1}` and `{"a":1,}` already raise during the initial
parse, before `find` is ever reached). Real only for a library consumer building their own
cursor directly from raw, unvalidated document bytes. Fixed anyway (unlike #2293's own
`eval.rs` gaps, left as a follow-up) because the change is narrow, mechanical, and reuses
existing, already-proven checks within the same file -- not the broader, riskier
`eval.rs`-wide sweep #2293 recommends as its own issue.

**3. `eval.rs`'s own `has_one_key` (its object arm, a raw `.any()` with zero gap checks)
shared this same #2288 non-string-key gap** -- tracked as part of #2293's own
"`eval.rs`'s parallel evaluator" umbrella finding (filed alongside `builtin_length`/
`builtin_keys`'s identical shape during #2261's own round-2 review), not a new discovery
here. **Fixed by #2293**: the object arm now uses `contains_checked`, which already
performs this exact check as part of its own walk, so no separate fix was needed here
beyond switching helpers. Same reachability analysis applies: `succinctly yq`'s one call
site into `eval.rs` only ever sees an already-materialized, pre-validated `OwnedValue`, so
this remains inert for both CLIs today, same as the rest of #2293's fix.

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
`path()`) has the identical shape again, originally at seven call sites (#1634): five in
[src/jq/eval.rs](../../../src/jq/eval.rs) (`eval_index_expr` ×2 arms, `eval_slice_expr`,
`resolve_index_expr`, `resolve_slice_expr`) plus two more in
[src/jq/eval_generic.rs](../../../src/jq/eval_generic.rs)'s own independent
`eval_index_expr`/`eval_slice_expr` — the latter two are what a real `succinctly jq`/
`succinctly yq` CLI invocation actually dispatches an ordinary `.[$keys]`/`.[$s:$e]` read
through (the `eval.rs` siblings are reached only via the direct library API, or via
`eval_generic.rs`'s own fallback for expressions it doesn't handle natively). Each site
originally pre-sized its output with a single upfront product of two or three independent,
generator-controlled `Vec::len()`s (e.g. `keys.len() * targets.len()`), previously handed
straight to an infallible `Vec::with_capacity`. A large enough cross product — e.g. two
independent 100,000-element generators feeding the same `.[$keys]` — asks for more elements
than the allocator can satisfy even though neither input list is individually unreasonable
to materialize. succinctly now refuses with `Cannot allocate <factors joined by " * "> elements
for a computed-index expansion` via the same `Vec::try_reserve_exact` technique as the
`setpath`/string-repeat cases above, applied through a shared `try_reserve_product` helper.
Confirmed live against the pinned jq 1.7.1 binary for this exact shape (not just analogized
from the string-repeat case above): a genuinely large-but-indexable cross product
(`[range(50000) | [1,2]] | .[][(range(50000))]`) neither errors nor crashes — jq streams
results one at a time instead of pre-allocating a single buffer, so it just keeps producing
output rather than answering promptly or refusing.

Both `eval_index_expr` arms (#2032/#2142) and, since #2143, `eval_slice_expr` (in both
files) have since moved off that single *full* upfront product: their own target (the
left side of `.[$keys]`/`.[$s:$e]`) is now re-evaluated once per key/`(s, e)` pair rather
than once overall (see this doc's own no-longer-applicable earlier framing corrected —
target length can vary per key/pair now, so a product including it is no longer even
computable), so each also reserves incrementally per key/pair via `Vec::try_reserve`
against `cannot_reserve_cross_product`'s identical error, regardless of the target's own
length. The two functions differ on what's reserved *before* that incremental loop even
starts: `eval_index_expr` reserves nothing upfront (purely incremental from an empty
`Vec`), while `eval_slice_expr` reserves a `starts.len() * ends.len()` baseline first, via
the same `try_reserve_product` helper this section already describes (both factors are
already known non-empty by this point, so it never takes that helper's own zero-factor
fast return) — both factors are still fully known before the loop, so this recovers a
single allocation for the common one-output-per-pair case instead of paying
amortized-doubling reallocation/copy costs on every slice query, and reuses
`try_reserve_product`'s existing overflow/refusal handling and its existing unit test
coverage for both, rather than adding a parallel, practically-untestable check of its own
(a `starts * ends` pair count large enough to organically overflow this product would
first exhaust memory building the `starts`/`ends` bound streams themselves, long before
this reservation could ever run). The refusal guarantee this section describes is
unchanged by any of this — every push remains behind a fallible reservation, so the
failure mode stays "clean refusal," never a panic — only the moment(s) a check runs and
the factor(s) named in the error message
changed. `resolve_index_expr`/`resolve_slice_expr` (the `path()`/write-path siblings,
target still evaluated once — tracked separately as #2139) are the two sites that still
call `try_reserve_product` directly with the original multi-factor product.

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

**Closed by [#2850](https://github.com/rust-works/succinctly/issues/2850).** `-e`/
`--exit-status`'s own separate materializer (`src/jq/lazy.rs`) now reports a clean, catchable
diagnostic instead of panicking, via a new checked twin (`JqValue::try_materialize`) rather
than the panicking `materialize` #1818 above left untouched. `test_exit_status_query_rejects_
adversarial_nesting_998` (`tests/jq_cli_tests.rs`) is unaffected and stays green either way --
it exercises `-e -c .[0]`, a shape #1793's own `catch_unwind` already reported cleanly before
#2850 (only the exit code and message were pinned, not the absence of leaked panic text).
`test_exit_status_over_depth_document_reports_cleanly_not_panic_2850` is a new, separate test
that does assert `!stderr.contains("panicked")` -- the specific property this issue fixes.

That same fix also closes a gap #2662 (unrelated to #2850, landed independently) opened in
`-S`/`-C`/`--ascii-output`'s own #1818 protection above: moving those three flags onto the
lazy/`JqValue::Cursor` dispatch bypasses `validate_json_delimiters`'s earlier checked guard,
the one #1818 actually credited for closing their gap — reaching `write_output_jq_value`'s own
materialize call directly, the identical panicking route `-e` always took. #2850's
`try_materialize` closes that call site too, so the net effect across both issues landing
together is unchanged from #1818's original guarantee: `-S`/`-a`/`-C` still cannot panic on
over-deep input, now via a different, more direct mechanism than the one #2662 stepped past.

`succinctly yq` still has no equivalent guard on either its default or materializing path at
all (#1817) — untouched by #2850, which is jq-mode only. Confirmed live, also pre-existing and
unrelated to any of #1793/#1818/#2850: `print_json`'s own guard can flush corrupted/truncated
JSON to stdout before it fires (#1819).

[#2692](https://github.com/rust-works/succinctly/issues/2692) widens the "accepts at
parse/index time" story above to more filters, by the same mechanism as `.[1]`/`length`:
`not`, `select`, `if`, `and`, `or`, `//` and zero-arity `any`/`all` used to reach this guard
too, via `push_generic_document_validation_error`'s own `assert_nesting_depth` — the walk
that validated on their behalf before #2692 removed it. None of them do enough work to need
that guard on their own account (`DocumentCursor::is_falsy` never recurses into a
container's children), so a document nested past 256 levels is no longer a reason for any of
them to fail: `nested_arrays(300) | not` now answers `false` at exit 0, where it used to
raise `nesting depth exceeds limit of 256`. This is a corollary of the same rule the rest of
this section's #2692 instance rests on, not a fresh decision, and it does not touch the
guard itself — `sort_by`/`unique_by`/`min_by`/`max_by` still materialize a comparison key
through `key_elements_generic`, the one caller left, and still trip it on the same document.
Pinned by `test_truthiness_probes_do_not_trip_the_depth_guard_2692`
(`tests/jq_cli_tests.rs`).

**#2669 applied that same rule to the one `and`/`or` route that had kept the old
behaviour.** #2692 removed the validation walk, and the lazy arms took that immediately —
but the *collecting* entry points (`eval_boolean`, `eval_boolean_generic`) still passed the
eager operand strategy, whose `push_generic_truthiness` ran
`push_generic_document_validation_error` per item. So on a malformed document the same
expression answered differently depending only on which consumer wrapped it, measured on
`main` before #2669:

| filter (input `{"a":1,}`)     | before #2669 | after   |
|-------------------------------|--------------|---------|
| `(.,.) and true` (bare)       | `true` `true` | `true` `true` |
| `first((.,.) and true)`       | `true`       | `true`  |
| `[limit(2; (.,.) and true)]`  | `[true,true]`| `[true,true]` |
| `[(.,.) and true]`            | **exit 5**   | `[true,true]` |

Three of the four already accepted, so #2669's switch to the lazy strategy moved the fourth
to join them rather than the reverse — the direction #2692 chose. Real jq rejects all four
(it cannot parse the document at all), so this is the existing #2692 divergence applied
consistently, not a new one. Whether a truthiness-only read should validate at all remains
open as [#2701](https://github.com/rust-works/succinctly/issues/2701); #2669 does not
settle that, it only removes the route-dependence. A route that genuinely reads a member
(`.a and true`) still raises. Pinned by
`test_boolean_routes_agree_on_a_malformed_document_2669`.

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

What #366 did *not* build: computed bounds. `.[$a:$b]` was a parse error at the time,
because the parser folds slice bounds to integer literals (see
[docs/reference/jq-language.md](../../reference/jq-language.md));
[#499](https://github.com/rust-works/succinctly/issues/499) added them later. Writing
through a slice against `null` used to raise `Cannot index null with object` instead of
auto-vivifying — [#1340](https://github.com/rust-works/succinctly/issues/1340) brought
that in line with jq's own `setpath()` behavior, and
[#1873](https://github.com/rust-works/succinctly/issues/1873) later fixed a gap in that
same auto-vivification for a slice with more path *after* it (`.a[0:1][]? = 9` on a
missing `.a` now no-ops instead of raising a write-time error, matching jq).

### A computed bound is ruled on at the slice step, after the target's kind (#2546)

jq's `INDEX` opcode looks at the target before it parses a slice descriptor's bounds:
`null | .["x":]` is `null`, `{"a":1} | .["x":]` is `Cannot index object with object`, and
only an array/string target ever raises `Array/string slice indices must be integers`. The
postfix `?` is the same opcode's `INDEX_OPT` form, so it suppresses the slice step of *one*
`(start, end, target)` triple and the generators resume — `[1,2] | [.[(0,"x",1):]?]` is
`[[1,2],[2]]` where `[try .[(0,"x",1):]]` is `[[1,2]]` — and the bound generators are pulled
lazily, `start` outermost and `end` per start value, so a pair's error stops them before the
next value's side effect: `.[("x",(1|debug)):]` raises with no DEBUG line, and
`.[(0,1|debug):(2,3|debug)]` writes DEBUG `0 2 3 1 2 3`. All captured live from jq 1.7.1;
succinctly used to classify every bound eagerly at the pull site, outside the per-pair `?`
and ahead of the target, so every one of those rows raised the slice-indices error (or, for
the DEBUG rows, ran the generator to exhaustion first). Both value-mode evaluators and the
path-mode resolver now follow jq's order.

One corner of path mode is still open: a non-numeric bound over a **null** target. jq
resolves that path (`null | path(.["x":])` is `[{"start":"x","end":null}]`, `?` or not) and
leaves it to the write to refuse it — `= 5` and `|= 5` raise `Array/string slice indices must
be integers`, `del()` no-ops to `null`. succinctly's `Expr::Slice` path component holds only
integer bounds, so the resolver raises that same error at resolution instead, unsuppressed by
`?`: identical for every write jq refuses, wrong only for `path()` (jq reports the descriptor)
and `del()` (jq answers `null`). Tracked as
[#2853](https://github.com/rust-works/succinctly/issues/2853). The path-mode resolver also
still drains each bound generator eagerly before slicing — the same gap
[#2267](https://github.com/rust-works/succinctly/issues/2267) records for `resolve_index_expr`
— so `path(.[("x",(1|debug)):])` still prints the DEBUG line jq never reaches.

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
  identifier**, since `Expr::FuncCall` carries no source position. Adding a position to
  the AST would perturb `format!("{body:?}").len()`, which #1381's
  `MAX_FUNC_EXPANSION_WEIGHTED_COST` is calibrated against, so this stays a textual search
  rather than a real position lookup. [#2037](https://github.com/rust-works/succinctly/issues/2037)
  closed two edges of that: every unresolvable call is now reported, not just the first
  (`jq: N compile errors`, matching jq's own count — and, matching real jq's own compiler,
  an unresolved callee's *arguments* are no longer independently checked and reported,
  since jq itself never compiles them without a resolved callee to bind them to), and a
  name mentioned more than once has each of its occurrences located independently — the
  search for the *k*-th reported call of a given name resumes right after the (*k*-1)-th
  one's match, rather than re-finding the first occurrence every time.

  This is still a pure textual heuristic, not a proof from real positions, and it can
  misfire two distinct ways: a construct whose traversal order disagrees with a
  left-to-right text scan (none exist among today's `Expr` variants, but nothing enforces
  that going forward), and — already true before #2037, unchanged by it — an unrelated
  occurrence of the same spelling (an object key, a variable) earlier in the source is
  indistinguishable from the real call site to a pure text scan: `{nosuch: 1} | nosuch`
  cites the harmless object key on line 1 instead of the actual failing call on line 2 —
  tracked separately as
  [#2085](https://github.com/rust-works/succinctly/issues/2085), since it predates #2037
  and neither of #2037's fixes touch it. Closing either gap for real needs the same source
  position on `Expr::FuncCall` this whole approach exists to avoid adding.
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

### The other three compile-error paths — one closed, two still open (#2703)

[#2703](https://github.com/rust-works/succinctly/issues/2703) found that the shape above
(`jq: error: … at <top-level>, line N:`, the echoed source, and the `jq: N compile error(s)`
trailer) was reproduced only on the undefined-name/arity path this section describes — three
sibling compile-error paths used a flat `jq: compile error: {e}` or `jq: module error: {e}`
instead, disagreeing with the runner's own correct sibling.

**Closed for a single unresolvable `include`/`import` (`module not found`).** Fully
deterministic when only one module fails to resolve — no source position to compute, so
nothing blocks matching jq byte for byte: `jq: error: module not found: {name}`, a blank
line (jq's own stand-in for the missing source echo), then the trailer.
`ModuleLoader::ensure_module_loaded`'s not-found case now reports this exactly; verified
against jq 1.7.1 for both `include` and `import`.

**Residual gap: which module is named when *more than one* is unresolvable.** jq reports
the last failing declaration in true source order, regardless of `include`/`import` kind
or how many resolvable declarations sit between the failures (confirmed live: `include
"AAA"; include "BBB"; include "CCC"; 1` names `CCC`; interleaving kinds --
`include "CCC"; import "AAA" as a; 1` names `AAA`, and reversing the two names `CCC` --
always whichever comes last in the source, not last within its own directive kind). Two
architectural reasons this codebase can't reproduce that: `Program` stores `includes` and
`imports` as two separate `Vec`s with no shared position or interleaving order between
them (unlike `Expr::FuncCall`'s own well-known missing-position problem elsewhere in this
document, extending `Import`/`Include` the same way is unexplored, not deliberately
declined); and `ModuleLoader::unqualified_def_names` (#2395) resolves every `include`
*before* `process_program` ever looks at an `import`, so a failing `import` is never even
reached when an earlier-declared `include` also fails, regardless of which one jq itself
would report. Succinctly instead reports the first `include` failure it encounters, or (if
every `include` resolves) the first failing declaration in whichever of the two
`process_program` loops runs into one -- includes, then imports, each in their own list
order. This is a gap #2703's own single-module repro never exercised; tracked separately
as [#2857](https://github.com/rust-works/succinctly/issues/2857).

**Still open: a syntax error in the main filter, and a syntax error inside an `include`d
module.** Unlike the deterministic not-found case, jq's trailing padding for a *syntax*
error is not the same fixed rule as the undefined-name case above — probing several shapes
(`1 +`, `.foo[`, `{a:`, `1,,`, `.foo | 1,, | .bar`) all echoed the line plus `len - 1` trailing
spaces, but `if 1 then` breaks that pattern: it produces *two* separate compile errors from
one parse, one padded per the `len - 1` rule and the second with none at all. jq's real rule
depends on the specific diagnostic's own token span, which is bison/lexer internals this
codebase has no access to; reproducing it needs either reading jq's C parser source
(`parser.y`/`scanner.l`) or a much wider oracle sweep than #2703 did. Also unresolved: jq's
parser can report *multiple* compile errors from a single malformed filter (again, `if 1 then`
→ 2 errors); succinctly's parser returns a single `Result<_, ParseError>` and has no
architecture for accumulating more than one, which is a separate, larger gap than the message
wording alone.

Until that lands, these two paths keep their pre-#2703 shape: `jq: compile error: parse error
at position N: …` for the main filter, `jq: module error: parse error in module '{path}': …`
for a module (naming the path as given on the command line, not jq's resolved absolute path).
Both still exit 3, matching jq, and neither corrupts output or crashes — the two conditions
that would otherwise make this an accepted ADR-0018 divergence rather than an open gap.

## Recursive `def`s: what a native call stack costs that jq's own does not — #1371

> The `MAX_EVAL_FRAMES` raise is uncatchable by `?`/`try`/`catch` since #2132 -- see "Every resource cap is uncatchable" below.

`succinctly jq` used to substitute a `def`'s body into each call site before evaluating
anything, which cannot terminate for a self-recursive `def` (expansion has no way to see
that `n == 0` will eventually hold) and so had to be bounded by three guards. The
consequence was not a deep-recursion edge case: `sum_to(100)` was refused where jq returns
`5050`, and a *branching* body — naive `fib` — failed at every depth including zero,
because expansion unrolled the `else` arm it could not know a given input would never take.

[#1371](https://github.com/rust-works/succinctly/issues/1371) replaced that with real
runtime calls (ADR-0020): a call is bound to its definition when the `def` is evaluated and
substituted when evaluation reaches it, with each argument captured behind a shared,
substitution-opaque node. Recursion now stops at the base case the program itself
evaluates. `sum_to(100)`, `sum_to(10000)`, `fib`, and #1381's chained-`def` repro all match
jq 1.7.1 byte for byte.

Three differences remain, all in the direction of erroring rather than aborting:

- **A non-terminating `def` errors; jq dies.** `def deep: [deep]; deep` and `def f: [f, f];
  f` exceed `MAX_EVAL_FRAMES` and raise a catchable error, exit 5. Real jq aborts on both
  with `cannot allocate memory` and exit 134 (confirmed live). Divergence in the only
  direction ADR-0018 permits: matching would take the process down. [#2737](https://github.com/rust-works/succinctly/issues/2737)
  fixed a shadow bug (a nested zero-arg `def` sharing a name with an enclosing
  parameter wasn't in scope inside its own body, so it silently resolved to the
  stale parameter instead of recursing) that had been keeping some of these shapes
  terminating with a wrong answer instead of reaching this same divergence:
  `def f(a): def a: a+1; a; f(1)` now hits `MAX_EVAL_FRAMES` where it used to answer
  `2`.
- **A heavy body runs out of depth sooner than jq's does.** jq evaluates on a
  heap-allocated VM stack (confirmed against jq 1.7.1's source: `exec_stack.h`'s
  `struct stack` grows via `realloc` through `jv_mem_realloc`, and `execute.c`'s
  `CALL_JQ` opcode pushes a frame onto it and continues the same bytecode-dispatch loop
  rather than recursing through a C function call), so its recursion depth is unaffected
  by how much structure a body holds live across its own recursive call; this evaluator
  recurses natively, so it is not. A body wrapping its call in 40 array constructors stops
  at ~900 levels where jq reaches 12,000+. The ceiling counts live frames rather than
  calls precisely because the two differ by 10x across body shapes — see
  `MAX_EVAL_FRAMES` (`src/jq/eval.rs`).
- **A recursively-built value can exceed `MAX_VALUE_TREE_DEPTH` (384) where jq has no such
  limit.** `def deep(m): if m == 0 then . else [[…]]deep(m-1)[[…]] end; deep(60)` builds
  1,200 levels; jq prints it, succinctly reports `nesting depth exceeds limit of 384` and
  exits 5. Before #1371 this shape could not recurse far enough to reach the ceiling at
  all.
- **A self-recursive comma generator streams for thousands of elements, not
  indefinitely.** `def naturals: 0, (naturals|.+1); [limit(100000; naturals)]` errors
  (`naturals/0 exceeded maximum recursion depth`) somewhere between 10,000 and 20,000
  pulled elements; jq streams it without limit. Every element the demand-driven evaluator
  (`eval_each`) pulls from a self-recursive generator is a genuinely deeper native call —
  that evaluator's own sink-based, non-tail-recursive design measures at ~70x the native
  stack per level that the plain evaluator's does (see `Expr::Shared`'s doc comment,
  `src/jq/eval.rs`) — so the guard is catching a real, not merely counted, native-stack
  cost. Fixing this needs a trampolined or otherwise bounded-stack `eval_each`, not an
  accounting change; tracked separately as a future architectural improvement, not part of
  #1371's scope.

Recursion is quadratic in time in both tools — a call-by-name parameter is re-evaluated at
each use, so reading one at depth `d` costs `O(d)`. Measured interleaved on one machine,
`sum_to(8000)` is 10.8 s here against jq's 4.3 s: same complexity, ~2.5x constant.

`MAX_EVAL_FRAMES`'s ceiling is calibrated against the 256 MB (release) / 2 GB (debug)
stack the CLI reserves for evaluation (`EVAL_STACK_SIZE`, `src/bin/succinctly/main.rs`).
A library caller invoking `succinctly::jq::eval` directly, on a thread sized for anything
smaller, does not get that guarantee — the same recursion that errors cleanly under the CLI
can still abort the process with a native stack overflow on an ordinary (e.g. default 8 MB)
thread. Callers embedding this crate and expecting to run recursive `def`s at any real depth
should evaluate on a thread reserved at a comparable size.

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

**The bound is enforced, not accidental (#2666).** Before #2666 the bound was wherever the
consumer happened to stop, and `map(f) | .[]` with *no* truncating consumer at all — bare, in
a pipe, under `try`/`?`/`//`, in a `def` body, as a `foreach` source — leaked the prefix
`2` to stdout before raising, because the iterate consumer streamed one element at a time
regardless of who was downstream. It now asks: every sink carries a
[`Budget`](../../../src/jq/eval_generic.rs) (`Unbounded` unless a truncating consumer says
`AtMost(n)`), and `each_lazy_seq_iterate_sink` validates the first `n` elements of `map(f)`
*before* the first output leaves — all of them when unbounded. So the divergence is bounded
by:

> **at least** the first `n` elements of `map(f)` are validated before the first output, where
> `n` is the truncating consumer's count; an element past that window is run only if the
> consumer's remaining stages pull it.

"At least", not "exactly": a stage after `.[]` that *expands* an element (`(.,.)`) or that a
`try`/`//`/`as` closure interposes resets the count upward, so more may be validated than the
consumer strictly needed — the safe direction. `nth(k; ..)` and `.[k]` validate `k + 1`
elements whatever bound the consumer behind them carries, because that is how many they must
examine; `limit(n; ..)` composes with the consumer behind it (`first(limit(3; ..))` validates
one).

`limit(2; ..)` on `[1,2,"x"]` is the worked example of what still diverges — two elements
are pulled, both good, the consumer stops, the third is never run — while `limit(2; ..)` on
`[1,"x",3]` now errors like jq because the failing element is inside the window. Anything
that has to see the whole array still errors, in both tools, and bare `map(f) | .[]` has
moved into this block:

```
$ echo '[1,"x",3]' | succinctly jq -c 'map(.+1)'                    # error, exit 5
$ echo '[1,"x",3]' | succinctly jq -c 'map(.+1) | .[]'              # error, exit 5, no output
$ echo '[1,"x",3]' | succinctly jq -c 'try (map(.+1)|.[]) catch "c"' # "c"
$ echo '[1,"x",3]' | succinctly jq -c 'map(.+1) | last'             # error, exit 5
$ echo '[1,"x",3]' | succinctly jq -c '[map(.+1) | .[]]'            # error, exit 5
$ echo '[1,"x",3]' | succinctly jq -c 'limit(3; map(.+1) | .[])'    # error, exit 5
```

Two shapes that used to diverge now match jq for free: `first(try (map(f)|.[]) catch c)`
and `first((map(f)|.[]) // 0)`. A `try`/`//` interposes its own closure between `first` and
the producer, and that closure carries the `Unbounded` default — so `map` is atomic inside
it, and `try` sees an error and no output, as in jq. The cost is real and accepted under
ADR-0018's decision order: those shapes, and `label $o | (map(f) | .[] | ., break $o)` (jq's
own `first`, whose `break` no static budget can see), run in ~0.58 s on 2M elements against
the flat `first(map(f) | .[])`'s 0.047 s — roughly jq's own 0.48 s, rather than the 11× lead
the divergence used to buy them. A forwarding wrapper *is* applied at one place, for the
opposite reason: the parenthesised `first((map(f) | .[]) | g)` is the same program as the flat
spelling and must answer and run alike, which it now does. See
[ADR-0022](../../adrs/adr-0022.md).

Pinned by [`test_map_iterate_atomicity_outside_truncators_2666`](../../../tests/jq_cli_tests.rs)
(every row above, both directions, one assertion each — the preserved rows carry the
divergent literal on purpose so the boundary is pinned from both sides),
[`test_generic_lazy_seq_first_after_map_skips_later_error_725`](../../../tests/jq_cli_tests.rs)
(the `map(f) | first` / `map(f) | .[0]` spelling) and
[`test_first_over_lazy_seq_iterate_skips_later_error_1565`](../../../tests/jq_cli_tests.rs)
(the `first(map(f) | .[] | g)` spelling, plus the draining counter-cases above).

**One more wrinkle: `.[0]` and `.[0.0]` don't agree.** `eval_generic.rs`'s lazy-sequence fold
keys the skip-the-rest fast arm off the literal AST shape `Expr::Index { idx: 0, key: None }`
(#1401), which only a bare integer-literal `.[0]` parses to -- `.[0.0]` folds to a different
node and falls through to the eager evaluator instead, so it does *not* get the skip and
raises exactly like real jq does for both spellings:

```
$ echo '[1,2,3]' | succinctly jq -c 'map(if . > 1 then error("boom") else . end) | .[0]'   # 1, exit 0
$ echo '[1,2,3]' | succinctly jq -c 'map(if . > 1 then error("boom") else . end) | .[0.0]' # error, exit 5
$ echo '[1,2,3]' | jq          -c 'map(if . > 1 then error("boom") else . end) | .[0]'     # error, exit 5 (both spellings)
```

`.[0]` and `.[0.0]` are equivalent everywhere else in this codebase -- that equivalence is
the whole premise of #1088's spelling preservation, asserted for reading, `del`,
`setpath`/`=`/`|=` and `getpath` by
[`tests/jq_index_number_invariant_tests.rs`](../../../tests/jq_index_number_invariant_tests.rs)
-- this lazy-fold fast arm is the one place the spelling changes the answer. Pinned by
[`test_lazy_seq_first_index_zero_vs_float_spelling_disagree_2174`](../../../tests/jq_cli_tests.rs).
Left as-is
rather than widened to also match `.[0.0]` ([#2174](https://github.com/rust-works/succinctly/issues/2174)):
widening would make both spellings agree, but on the *divergent* side, spreading this
limitation to a second spelling where ADR-0018 says matching jq is plainly possible instead
(#1401's own comment on the fast arm records the same reasoning). `Expr::Index { idx: 0, key:
None }`'s pinning is correct and does not need to change even if this limitation is ever
lifted for `.[0]` itself -- only then would widening the arm become the right move, not
before.

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

## A generator `n` under a truncating consumer: fixed for `limit`, residual for `nth`/`isempty`

Real jq passes `limit($n; f)`'s `$n` through the same backtracking arg-passing convention
as any other filter argument, so a *generator* `n` (`limit((1,2); f)`, #1279's own canonical
example) re-runs the whole `limit` body once per bound value of `$n` — but only as many
times as the wrapping consumer actually needs.

**`limit` now matches jq here (#1687).** `eval_generic.rs`'s `fanout_arg_generic`/
`fanout_arg_each_generic` drive `n` through the demand-driven sink, so a wrapping
`first`/`nth`/`isempty`/`any` stops the `n` generator itself:

```
$ succinctly jq -cn 'first(limit((1,2); (1, ("B"|stderr))))'      # jq: 1, no stderr — agrees
1
$ succinctly jq -cn '[limit((1,2); (1, ("B"|stderr)))]'           # jq: [1,1,"B"], stderr B — agrees
[1,1,"B"]
B
```

The second row is not a typo: an *unbounded* consumer genuinely needs both of `$n`'s
bindings (`$n=1` keeps `expr`'s first output alone; `$n=2` keeps its first two, so
`("B"|stderr)`'s own value ends up in the array too), so exploring `expr`'s second output
there is correct in both tools.

**One shape still diverges** — verified live against jq 1.7.1, and unchanged by #1687:

| filter                                       | jq 1.7.1 | `succinctly jq` |
|----------------------------------------------|----------|-----------------|
| `isempty(limit((1,("N"\|debug)); 42))`        | no stderr | writes `["DEBUG:","N"]` |

`isempty` has no native arm in `eval_generic.rs` at all, so the whole expression is
evaluated by `eval.rs`, whose own `each_limit` still classifies `n` with a single eager
`eval_single` rather than a fan-out. It is the same shape of gap `limit` had before #1687
and is fixable the same way — port `fanout_arg` to `eval::each_limit`, which is not
`limit`'s own scope. Tracked in
[`docs/plan/jq-lazy-generator-consumers.md`](../../plan/jq-lazy-generator-consumers.md)
(item 9).

Its sibling `first(nth((0,1); (1, ("B"\|stderr))))` — recorded here as also writing `B` —
**closed as a side effect of #2180 WP1**: `nth` gained both the sink-side twin it lacked
(`each_nth`/`each_nth_generic`) and, with it, a demand-forwarding fan-out over its own `n`
(`fanout_arg_each`, the `eval.rs` mirror of `fanout_arg_each_generic`), so the wrapping
`first`'s stop now reaches the `$n=1` binding and neither tool writes anything. Confirmed
live against jq 1.7.1.

## A `?//`-alternatives bind sees a short-circuiting consumer's stop only when nothing materializes it first

Real jq's short-circuiting builtins (`first`, `isempty`, `limit`, `nth`, `any`, `all`, `IN`) are
defined in `builtin.jq` as `label $out | ... break $out`. When the generator argument is a
`?//`-alternatives bind and that `break` unwinds through it, jq's `?//` treats *any* escaping
break as "this alternative failed, try the next" — it does not distinguish a label declared
outside the whole `?//` from one declared inside it. The consumer's whole computation therefore
runs once per alternative.

[#1519](https://github.com/rust-works/succinctly/issues/1519) implemented that rule.
succinctly's builtins are native Rust rather than macro expansions, so they signal satisfaction
as `Demand::Stop`/`Flow::Stopped` instead of raising a break — the *same event*, so
`each_pattern_alternatives` (`src/jq/eval.rs`) and `each_pattern_alternatives_generic`
(`src/jq/eval_generic.rs`) now fall through to the next alternative on it, under the same
`is_last` rule they already applied to `Control::Break` since
[#1457](https://github.com/rust-works/succinctly/issues/1457) (see `eval::is_retryable_stop`).
The terminal sinks were reshaped to jq's own macro shape at the same time — emit, *then* stop —
so a retried alternative emits again. All of the following now match, confirmed live against
jq 1.7.1 and pinned as golden fixtures (`tests/data/jq-golden/cases/alt_pattern_*`):

| filter                                                     | jq 1.7.1 and `succinctly jq` |
|------------------------------------------------------------|------------------------------|
| `isempty(1 as $x ?// $y \| 5)`                             | `false` then `false`         |
| `[first(1 as $x ?// $y \| 5, 6)]`                          | `[5,5]`                      |
| `[limit(2; 1 as $x ?// $y \| 5,6)]`                        | `[5,6,5]`                    |
| `[nth(1; 1 as $x ?// $y \| 5, 6)]`                         | `[6,5]`                      |
| `[any(1 as $x ?// $y \| true; .)]`                         | `[true,true]`                |
| `1 as $x ?// $y \| 5` (no consumer)                        | `5`                          |

**A nested short-circuiting consumer no longer diverges — #2180 WP1 closed that whole group.**
`first`, `nth`, `isempty`, `any`/`all(gen; cond)` and `IN(s)`/`IN(src; s)` each gained the
demand-forwarding arm `limit` already had, in both evaluators: `each_first`, `each_nth`,
`each_isempty`, `each_any_all_gen_cond` and `each_upper_in`/`each_upper_in_src` in
`src/jq/eval.rs` (sharing `finish_short_circuit` and `counted_bool_flow_to_flow`, plus
`fanout_arg_each` for `nth`'s own `n` argument), and `each_first_generic`/`each_nth_generic`
plus `bridge_to_each_owned_flow` in `src/jq/eval_generic.rs`. Every one of these now matches
jq 1.7.1 under both wrappers (captured live, input `1`):

| filter                                                                           | jq 1.7.1 and `succinctly jq` |
|------------------------------------------------------------------------------------|------------------------------|
| `[first(first(1 as $x ?// $y \| 1))]`, `... nth(0; ...)`                            | `[1,1]`                      |
| `[isempty(first(...))]`, `[isempty(nth(0; ...))]`                                  | `[false,false]`              |
| `[first(isempty(...))]`, `[isempty(isempty(...))]`                                 | `[false,false]`              |
| `[first(any(...; .))]`                                                             | `[true,true]`                |
| `[first(IN(...))]`, `[first(IN(1; ...))]`, `[limit(1; IN(...))]`                   | `[true,true]`                |
| `[first(isempty([1] as [$x] ?// $x \| if ($x\|type)=="number" then 9 else empty end))]` | `[false,true]`          |

The last row is the rule the group's shared terminal helper exists for: the outer consumer's
stop unwinds into the `?//`, the retried final alternative runs the generator dry, and *that*
exhaustion is what reaches `isempty`'s trailing `, true` — so the identity element fires even
though the outer sink already said stop. `nth`'s `n` fan-out follows the same demand rule:
`[first(nth((0,1); (10,20)))]` is `[10]` in both (the `$n=1` binding is never explored), while
a bare `[nth((0,1); (10,20))]` is still `[10,20]`.

**`//`, `and` and `or` no longer diverge either — #2180 WP2a closed that group.** `//`
(`Expr::Alternative`) gained `each_alternative` in `src/jq/eval.rs`: a truthy-filter sink over the
left operand (`retain_truthy`'s rule, one output at a time) that forwards every surviving output
to the wrapping consumer and evaluates the right side only when the left forwarded nothing and ran
to exhaustion. `and`/`or` gained `each_boolean`, over a new **shared** demand-driven
`boolean_fanout_each` — the `and`/`or` twin of what #1459/#1481 did to `binary_fanout_core` for
`Compare`/`Arithmetic`: one loop, parameterised over how an operand's truthiness bits are
enumerated. WP2a left the pre-existing callers (`eval_boolean`, `eval_generic.rs`'s
`eval_boolean_generic`) on an eager strategy while the new arms passed
`eval_each`/`eval_each_generic`; #2669 moved those two onto the lazy strategy as well and removed
the eager one, so the parameter now selects only *what is done with the bits* (collect into a
`Vec<bool>` vs forward to a sink), never how the operands are enumerated. `eval_each_generic` gained its own
`Expr::Alternative`/`Expr::And`/`Expr::Or` arms too, since `first(...)` never reaches `eval.rs`'s.
Every row confirmed live against jq 1.7.1 under both wrappers, input `1`:

| filter                                                                                                      | jq 1.7.1 and `succinctly jq` |
|-------------------------------------------------------------------------------------------------------------|------------------------------|
| `[first((1 as $x ?// $y \| 5)//9)]`, `[isempty(...)]`                                                       | `[5,5]` / `[false,false]`    |
| `[first(null // (1 as $x ?// $y \| 1))]`                                                                    | `[1,1]`                      |
| `[first((1 as $x ?// $y \| 1) and true)]`, `... true and (...)`, `... (...) or false`, `... false or (...)` | `[true,true]`                |
| `[first((1 as $x ?// $y \| false, 5) // 9)]`                                                                | `[5,5]`                      |
| `[first((1 as $x ?// $y \| false) // 9)]`                                                                   | `[9]`                        |
| `[first(false and (1 as $x ?// $y \| 1))]`, `[first(true or (...))]`                                        | `[false]` / `[true]`         |

The last two rows are the group's own rules, which the `?//` rows alone do not pin. `//` still
*filters*: a falsy output does not answer, so the bind keeps being retried; and with no truthy
output at all the right side answers exactly once, because the consumer's stop never reached the
bind in the first place. `and`/`or` still genuinely short-circuit per left output: a falsy left
(`and`) or truthy left (`or`) never evaluates the other operand, so no bind inside it is ever
retried.

WP2a also closed ordinary side-effect leaks of the shape Stage 2 closed elsewhere, with no `?//`
involved — `first((1, ("B"|stderr)) // 9)`, `first((false,false) // (("A"|stderr), ("B"|stderr)))`
and all four `and`/`or` operand positions each ran a branch jq never reaches. Those rows, plus the
destructive `input` spellings (`[first((1, input) // 9), input]` was `[1,"b"]`, jq's `[1,"a"]`),
are pinned in `test_short_circuit_side_effect_shapes_already_match_jq_820`.

**The loop shape was captured, not assumed.** jq 1.7.1, `-cn`:
`[("A"|debug, "B"|debug) and ("C"|debug, "D"|debug)]` writes `A A C C D B C C D` to stderr — the
**left** operand is the outer loop and the right one is re-evaluated per non-short-circuiting left
output, interleaved. succinctly's eager route wrote `A A B C C D C C D` (left finished first),
exactly as `Expr::Compare` did between #1459 and #1481. `[(false,true) and ("C"|debug)]` writes `C`
once, not twice, which is the short-circuit rule.

**That residual is closed by #2669 — `and`/`or` now match jq on every route.** WP2a's own
`eval_each` arms took the lazy operand strategy while the *collecting* entry points
(`eval_boolean`, `eval_generic.rs`'s `eval_boolean_generic`) kept the eager one, so
`[("A"|stderr,"B"|stderr) and ("C"|stderr,"D"|stderr)]` wrote `AABCCDCCD` where jq writes
`AACCDBCCD` — while the *same operands* reached through a lazy consumer (`first(...)`,
`limit(...)`), as a binary operand (`0 + (...)`), or simply unwrapped at the top level already
matched, since those routes take the lazy arm. #2669 put both collecting entry points on the same
lazy strategy, which is the `eval_boolean` half of what #1481 did for `eval_binary_fanout`, and
deleted the eager strategy outright so there is no second one left to drift. The rows moved from
`test_short_circuit_side_effect_leaks_820_932_987` to
`test_short_circuit_side_effect_shapes_already_match_jq_820`.

The delivered values never differed here — it was stderr ordering only, for side-effect-free
operands. With `input`/`inputs` operands the same ordering decides which value each operand
consumes, which is #1481's own argument for closing it rather than documenting it.

**`if`'s condition, `as`/`as`-pattern's bound source, `select`, unary minus, an index key, string
interpolation, an object value and `range`'s bound no longer diverge either — #2180 WP2b closed that
group.** `each_if`/`each_if_generic` now drive `cond` through the sink, and `each_as`/
`each_as_pattern` (plus their generic twins) drive the bound source through
`fanout_arg_each`/`fanout_arg_each_generic` — the same demand-forwarding argument fan-out `nth`'s
own `n` already used — so `materialize_bound_values` and its generic twin are gone. New arms:
`Builtin::Select` (`each_select`/`each_select_generic`, native and cursor-preserving in both files,
since `select` never changes position), `Expr::Negate` (`each_negate`; the generic side bridges the
non-path-context case through `bridge_to_each_owned_flow` and leaves the existing
`needs_path_context`-gated native arm alone), `Expr::IndexExpr`'s key (`each_index_expr`/
`each_index_expr_generic`, jq mode only — yq mode's key stream keeps its retroactive
discard-on-later-escape rule, which a sink push that has already been delivered cannot undo),
`Expr::StringInterpolation` (jq mode only, bridged — yq mode is not a fan-out generator there at
all) and `Expr::Object` (`each_object_entries`/`each_object_value`, native in both files,
mirroring `build_object_entries`'s own entries-recurse/key-encloses-value nesting).
`eval_each_generic` also gained the `Expr::Range` arm `eval.rs`'s `each_range` (#1556) already
had, closing the one row the two arm sets had drifted on. Every row confirmed live against
jq 1.7.1 under both wrappers:

| filter                                                                                      | jq 1.7.1 and `succinctly jq` |
|---------------------------------------------------------------------------------------------|------------------------------|
| `[first(if (1 as $x ?// $y \| 1) then 5 else 6 end)]`                                       | `[5,5]`                      |
| `[first((1 as $x ?// $y \| 1) as $v \| $v)]`, `... as [$a] ?// $a \| $a`                    | `[1,1]`                      |
| `[first(select((1 as $x ?// $y \| 1) == 1))]`                                               | `[1,1]`                      |
| `[first(-(1 as $x ?// $y \| 1))]`                                                           | `[-1,-1]`                    |
| `[1] \| [first(.[(1 as $x ?// $y \| 1)-1])]`                                                | `[1,1]`                      |
| `[first("\(1 as $x ?// $y \| 1)")]`                                                         | `["1","1"]`                  |
| `[first({a:(1 as $x ?// $y \| 1)} \| .a)]`                                                  | `[1,1]`                      |
| `[first(range((1 as $x ?// $y \| 1); 3))]`                                                  | `[1,1]`                      |

One over-stopping trap in this group is worth stating, because it is the opposite of a leak:
`first({a:1, b:(("B"|stderr), 2)} | .a)` writes `B` in jq **and** in succinctly. Object
construction cannot deliver any combination until every entry has produced its first value
(key encloses value, entries recurse left to right), so `b`'s side effect genuinely fires even
though `b` is never read — a lazy `Object` arm that skipped it would be wrong. Pinned alongside
the closed leaks (`first(if (true, ("B"|stderr)) then 1 else 2 end)`,
`first((1, ("B"|stderr)) as $v | $v)`, `first(select((true, ("B"|stderr))))`, and the
destructive `input` spellings) in `test_short_circuit_side_effect_shapes_already_match_jq_820`.

**`foreach` no longer diverges in its source, its pattern or its EXTRACT — #2180 WP3 closed
that group, and with it the last of the originally-filed rows.** `each_foreach` (`src/jq/eval.rs`)
and `each_foreach_generic` (`src/jq/eval_generic.rs`) drive the source through
`eval_each`/`eval_each_generic` one element at a time, and `try_foreach_step_alternatives` drives
each step's EXTRACT through `eval_each_owned`, pushing its outputs as they are produced. Since
WP3's review **every** `foreach` entry point — the two demand-forwarding arms and both eager ones
— shares one fold, `foreach_forks`, and drives the source the same way, so the two routes cannot
answer differently: `[foreach (1 as $x ?// $y | 1) as $v (0; if . == 0 then error("x") else
"OK:\(.)" end; .)]` used to raise from the eager route where jq and every `first`/`limit`
spelling answer `["OK:null"]`.

**The rule this group turns on: a sink's `Demand::Stop` is `Control::Break` in different clothes,
state threading included.** `foreach`'s own `?//` retries on the stop under `is_retryable_stop`,
the sibling of `is_retryable_control`, and the retried alternative resumes from the accumulator
the stopped attempt had already produced — which is why jq answers `[1,2]` and not `[1,1]`. That
is the same state-threading rule #1458 established for a `Control::Break` escaping EXTRACT (the
retry is seeded with the failed EXTRACT call's own input). The two rules agree on *state*; they
do **not** agree on what may be retried at all, which is the next section. Every row confirmed
live against jq 1.7.1 under both wrappers, input `1`:

| filter                                                                                                     | jq 1.7.1 and `succinctly jq` |
|------------------------------------------------------------------------------------------------------------|------------------------------|
| `[first(foreach (1) as $x ?// $y (0;.+1;.), "z")]`, `[first(foreach (1) as $x ?// $y (0;.+1))]`            | `[1,2]`                      |
| `[first(foreach (1) as $x ?// $y (0;.+1;., 99))]`                                                          | `[1,2]`                      |
| `[first(foreach (1 as $x ?// $y \| 1) as $v (0; .+$v; .))]` (source, not pattern)                          | `[1,2]`                      |
| `[isempty(foreach (1 as $x ?// $y \| 1) as $v (0; .+$v; .))]`                                              | `[false,false]`              |
| `[first(foreach (1) as $v (0; .; (1 as $x ?// $y \| 1)))]` (EXTRACT)                                       | `[1,1]`                      |
| `[first(foreach (1) as $x ?// $y (0; .+1; (1 as $a ?// $b \| .)))]` (both)                                 | `[1,1,2,2]`                  |
| `[first(foreach (1) as $x ?// $y ((0,100); .+1; .))]` (generator INIT)                                     | `[1,2]`                      |
| `[first(foreach (1) as [$x] ?// $y (0;.+1;.), "z")]` (first alternative cannot match)                      | `[1]`                        |
| `[limit(3; foreach (1 as $x ?// $y \| 1, 2) as $v ((0,100); .+$v; .))]` (a later fork's own stop)          | `[1,3,101,102]`              |
| `[foreach (1 as $x ?// $y \| 1) as $v (0; if . == 0 then error("x") else "OK:\(.)" end; .)]` (eager route) | `["OK:null"]`                |

The last four rows are the group's own rules, which the plain rows do not pin. A generator INIT
still fans out eagerly and outermost (#534), but a stop inside the *first* fork still reaches the
pattern's `?//`, and then ends the whole `foreach` — the `100` fork is never attempted. A first
alternative that cannot match the source element is skipped without consuming a retry, leaving
only the last one, whose stop is not retryable at all. And **every INIT fork drives the source
afresh**: WP3 recorded the first fork's elements and replayed them for later forks, which lost
that later fork's own retry (`[1,3,101]`) and double-recorded any element a source-side `?//`
re-offered. jq re-evaluates the source per fork anyway, side effects included — `[foreach (1,
("B"|stderr)) as $x ((0,100); .+1)]` writes `B` twice in jq 1.7.1, and now here (it wrote it once
under both the recording and the pre-WP3 eager path, a divergence WP3 inherited rather than
introduced). Removing the recording, together with the #695 substitution matrix it replaced, took
peak RSS on `[foreach .[] as $x (0; .+$x)] | length` over a 1,000,000-element array from 522.1 MB
to 30.4 MB.

**A `Halt` behind a stop is not retryable, and neither is a decode failure.** A driver that ends
its drive because something *escaped* can only answer `Demand`, so it stashes the escape out of
band and returns `Demand::Stop`; every `?//` between it and the consumer then saw a bare
`Flow::Stopped` and retried the alternative that had already halted. `nonretryable_stop`
(`src/jq/eval.rs`) records `is_retryable_control`'s two position-independent exclusions beside
the stop, `is_retryable_stop` consults it, and `stop_with_escape` is the one place that sets it.
This is the shared signal [#2567](https://github.com/rust-works/succinctly/issues/2567) asked
for, and it closes that issue's own residual too (`nth`'s illegitimate retry after a skipped
item's forced decode failure no longer runs the next alternative's body at all). Captured live,
`-cn`:

| filter                                                                                                                   | jq 1.7.1 and `succinctly jq`                              |
|--------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------|
| `[limit(1; (1 as $x ?// $y \| 1) \| ("h"\|halt_error(3)))]`                                                              | stderr `h`, exit 3 (wrote `hh`)                           |
| `first((1 as $x ?// $y \| 1) as $v \| ("h"\|halt_error(3)))`                                                             | stderr `h`, exit 3 (wrote `hh`)                           |
| `foreach (1 as $x ?// $y \| 1) as $v (0; ("h"\|halt_error(3)))`                                                          | stderr `h`, exit 3                                        |
| `first(foreach (1 as $x ?// $y \| 1) as $v (0; ("E"\|stderr) \| ("H"\|halt_error(3)); .))`                               | stderr `EH`, exit 3 (wrote `EHEH`)                        |
| `[limit(10; foreach (1 as $x ?// $y \| 1) as $v ((0, 100); if . == 0 then ("x"\|halt_error(3)) else "OK:\(.)" end; .))]` | stderr `x`, exit 3 (the retry swallowed the halt: exit 0) |
| `[limit(1; (1 as $x ?// $y \| 1) \| ("e"\|stderr) \| error("boom"))]`                                                    | stderr `ee` + the error, exit 5 (unchanged)               |

The last row is the control: an ordinary `error` (and a `break`) genuinely *does* retry through a
source `?//` in jq, so the signal carries `is_retryable_control`'s rule rather than refusing every
escape-backed stop.

WP3 closed ordinary side-effect leaks with the same arms: `first(foreach (1, ("B"|stderr)) as $x
(0; .+1))` and `first(foreach (1) as $x (0; .+1; ., ("E"|stderr)))` each ran a branch jq never
reaches, and the destructive `input` spelling `[first(foreach (1, input) as $x (0; .+$x)), input]`
over `"a" "b"` was `[1,"b"]` where jq answers `[1,"a"]`. Its over-stopping guard is pinned
alongside them: a side effect *before* EXTRACT's first output (`first(foreach (1) as $x (0; .+1;
("E"|stderr), .))`) is genuinely reached, in jq and here. All in
`test_short_circuit_side_effect_shapes_already_match_jq_820`. The `path(...)`/`halt_error` pair
that pins `binary_fanout_each`'s inner/outer `Flow::Stopped { pending }` asymmetry moved *out* of
`foreach` with the same change: that pair needs an operand still reaching the eager fallback that
*produces* a `pending`, so it used `if` until #1462, `foreach` until WP3, and now `recurse`, which
`eval_each` has no arm for. The asymmetry itself is unchanged.

**What still diverges: `foreach`'s UPDATE and its INIT, and only those.** Two positions inside
`foreach` are still evaluated eagerly, so a `?//` bind in either never sees the stop, and a side
effect in either fires where jq never reaches it. Confirmed live under both wrappers:

| filter                                                                     | jq 1.7.1        | succinctly jq |
|----------------------------------------------------------------------------|-----------------|---------------|
| `[first(foreach (1) as $v (0; . + (1 as $x ?// $y \| 1)))]` (UPDATE)       | `[1,1]`         | `[1]`         |
| `[isempty(foreach (1) as $v (0; . + (1 as $x ?// $y \| 1)))]`              | `[false,false]` | `[false]`     |
| `[first(foreach (1) as $v ((1 as $x ?// $y \| 1); .+1; .))]` (INIT)        | `[2,2]`         | `[2]`         |
| `[isempty(foreach (1) as $v ((1 as $x ?// $y \| 1); .+1; .))]`             | `[false,false]` | `[false]`     |
| `first(foreach (1) as $x (0; (.+1, ("U"\|stderr)); .))` (stderr in UPDATE) | no write        | writes `U`    |
| `first(foreach (1) as $x ((0, ("I"\|stderr)); .+1))` (stderr in INIT)      | no write        | writes `I`    |

Both are deliberate. Driving **UPDATE** means reshaping `fold_step_via_accumulator_or_fork`,
which `reduce`'s own O(n) accumulator fix (#2157) shares, and whose outputs are simultaneously
the fold's next state — a separate change from WP3's rows. **INIT** is jq's outermost loop
(#534: each INIT output is an independent run over the source) and must be evaluated before the
source is ever pulled (#2440: `foreach halt_error as $x (empty; .)` exits 0), so `foreach_forks`
collects it and returns before its source drive is ever called. The rows above are pinned in
`test_nested_short_circuit_consumer_hides_the_stop_2180` and
`test_short_circuit_side_effect_leaks_820_932_987` (`tests/jq_cli_tests.rs`); they are
deliberately *not* in `scripts/jq-alt-retry-oracle-sweep.sh`, which reports 0 unexpected and 0
known over 819 cases and would lose that contract if permanent divergences were swept.

**Two rows recorded here as of #2180's filing have since closed** — the issue text was stale on
them. `1 | [label $o | (1 as $x ?// $y | 5) | (., break $o)]` closed at
[`bcb41f74f`](https://github.com/rust-works/succinctly/commit/bcb41f74f49a64953f66cf03fab41c84899b1629)
("route `label` through the generic evaluator", spine 2416), and
`def m(g): label $o | g | ., break $o; [m(1 as $x ?// $y | 5, 6)]` closed separately at
[`8809f2b85`](https://github.com/rust-works/succinctly/commit/8809f2b85dd156491882792613004851eba71dc4)
("route bound function calls through the generic evaluator", spine 2416) — both bisected live
(#2180 WP0): `Expr::Label`/`Expr::DefCall` previously bridged their whole construct to the eager,
owned evaluator, which lost the cursor a `break` needs to unwind through on its way back to
`each_pattern_alternatives_generic`; routing them natively through `eval_generic.rs` fixed that as
a side effect, with no `?//`-specific work at either commit. Neither is the pipe rework the
original filing guessed at.

The cause was the same one every closed group above had, and it was not about `?//` at all:
`foreach` lacked a demand-forwarding `eval_each`/`eval_each_generic` arm, so it evaluated the bind
eagerly and absorbed the stop before it could reach `each_pattern_alternatives`. No work package
in this issue changed `each_pattern_alternatives`/`each_pattern_alternatives_generic` at all —
each one only gave its constructs the arm `limit` already had.

`limit` was the one consumer that already had such an arm (`each_limit`, #1462/#1596), and it was
exactly the one that worked when nested — `1 | [first(limit(1; 1 as $x ?// $y | 1))]` was `[1,1]` in
both, while substituting any other consumer for the inner `limit` diverged. That contrast is the
direct evidence for the mechanism, and WP1 acted on it: giving the other five consumers the same
arm closed their rows with no `?//`-specific work at all, exactly as predicted, which is the
strongest confirmation available that each remaining construct closes its own row the same way.
Demand-forwarding and unparenthesised spellings likewise all
already match (`[first((1 as $x ?// $y | 5)|.)]`, `[first(if true then 1 as $x ?// $y | 5 else 9
end)]`, `[first(limit(5; 1 as $x ?// $y | 5))]`, `[label $o | 1 as $x ?// $y | (5, break $o)]`), as
do collectors, `reduce`, `last`, `try`/`?`, `+`/`==` (`binary_fanout_each`), `def`/`DefCall`, and
`if`'s *branches* (as opposed to its condition) — none of those materialize their generator
argument the way the diverging constructs above do.

This is the same missing-lazy-arm class as items 9 and 10 of
[`docs/plan/jq-lazy-generator-consumers.md`](../../plan/jq-lazy-generator-consumers.md), tracked
in [#2180](https://github.com/rust-works/succinctly/issues/2180), whose own plan (2026-09-08)
split the residual into four work packages — WP1 (nested consumers), WP2a (`//`/`and`/`or`), WP2b
(the remaining eager sub-expression sites) and WP3 (`foreach`) — each closing its own rows, and
**all four have landed**. WP1 took `scripts/jq-alt-retry-oracle-sweep.sh` from 93 known
divergences attributed to it to none (612 cases, 0 unexpected, 258 known across WP2a/WP2b/WP3);
WP2a then took its own 96 to none (162 known across WP2b/WP3); WP2b took its own 144 to none (612
cases, 0 unexpected, 18 known, all WP3's `foreach`); WP3 took the last 18 to none and added
pattern-position and EXTRACT-position wrapper entries, and its review added a `limit(3; ...)`
consumer plus three multi-INIT-fork wrappers — the only shapes that reach a `foreach`'s second
fork at all — leaving **819 cases, 0 unexpected, 0 known**. All four left
`scripts/jq-fanout-oracle-sweep.sh` at 490/490.

Unrelated to the other `?//` divergence recorded above
([#1365](https://github.com/rust-works/succinctly/issues/1365), `?//`-alternatives folds not being
path-tracked), and to the path-context refusal
([#1663](https://github.com/rust-works/succinctly/issues/1663)), which independently makes
`{a:1} | [path(first(1 as $x ?// $y | .a))]` raise where jq answers `[["a"],["a"]]`. This entry
was originally split out as [#1831](https://github.com/rust-works/succinctly/issues/1831) to
record the divergence unconditionally, before it was known to be fixable this cheaply.

**`nth(n; f)`'s generic-evaluator twin has a narrower, separate residual**, tracked in
[#2199](https://github.com/rust-works/succinctly/issues/2199): `nth_with_n_generic`
(`src/jq/eval_generic.rs`) has to force the decode of a *skipped* (index `< n`) item for its
side effects, exactly as jq's own `nth($n; f) == last(limit($n + 1; f))` desugaring requires. A
decode failure there is recorded out-of-band and signalled as an ordinary `Demand::Stop`, which
`?//` retry (correctly) cannot tell apart from a genuine consumer-satisfied stop — so a skipped
item's decode failure on a non-last alternative gets silently retried into the next one instead
of propagating, unlike the equivalent `Flow::Escaped(Control::Error(e))` case, which
`is_retryable_control` already excludes via `is_decode_failure()`. `eval::each_take_nth` (the
borrowed evaluator's twin) is not at risk — its `Item` is never lazy, so it never needs to force a
skipped item's decode in the first place. Fixing this needs `Demand` (or an equivalent channel) to
let a sink mark its own stop as unretryable, which is a real design change rather than a local
one, so it is tracked separately instead of folded into this fix.

## `--seq`'s malformed-record warnings and recoverable values match jq

Real jq warns on stderr ("`jq: ignoring parse error: ...`") whenever it silently drops a
malformed `--seq` (RFC 7464) record; `succinctly jq` used to drop the record with no
diagnostic at all. [#1525](https://github.com/rust-works/succinctly/issues/1525) added the
warning for one of jq's message templates: content with no RS byte anywhere in the
input, where jq's own reader never syncs onto anything and reports the abandonment
unconditionally at EOF ("`Unfinished abandoned text at EOF at line L, column C`") regardless
of what the content actually is, even fully valid JSON.
[#1723](https://github.com/rust-works/succinctly/issues/1723) added all the rest.

```
$ printf '1 2' | jq --seq -c '.'                  # jq:          jq: ignoring parse error: Unfinished abandoned text at EOF at line 1, column 3
$ printf '1 2' | succinctly jq --seq -c '.'       # succinctly:  same (#1525)
```

**The messages are not a property of the record; they are a property of where jq's reader
gave up.** Four earlier attempts looked for a table mapping "what is wrong with this record"
onto jq's wording and correctly concluded that no such table exists — the *same* malformed
record draws different messages depending on whether jq reached real EOF, an RS byte, or an
ordinary byte first. `src/bin/succinctly/jq_seq_reader.rs` models jq's own scanner instead,
and composes the message from three facts:

| detected on | rendering |
|---|---|
| an ordinary byte | `{category} at line L, column C (need RS to resync)` |
| real EOF | `{category} at EOF at line L, column C` |
| an RS byte | `Truncated value` (or `Potentially truncated top-level numeric value`) |

```
$ printf '\x1e"unterminated\n' | succinctly jq --seq -c '.'
jq: ignoring parse error: Unfinished string at EOF at line 2, column 0
$ printf '\x1e[1,2\n'          | succinctly jq --seq -c '.'
jq: ignoring parse error: Unfinished JSON term at EOF at line 2, column 0
$ printf '\x1exyz\n'           | succinctly jq --seq -c '.'
jq: ignoring parse error: Invalid numeric literal at line 2, column 0 (need RS to resync)
$ printf '\x1e"a\x1e"ok"\n'    | succinctly jq --seq -c '.'
jq: ignoring parse error: Truncated value at line 1, column 4
```

Two pieces of jq's own wording are misleading, and are reproduced rather than corrected
(ADR-0018's bug-for-bug default):

- **`(need RS to resync)` never resyncs.** jq's `jv_parse.c` assigns
  `JV_PARSER_WAITING_FOR_RS` and *then* calls `parser_reset()`, which writes
  `JV_PARSER_NORMAL` straight back over it. Parsing resumes at the very next byte, so one
  record can warn several times and content after the error is still read:
  ```
  $ printf '\x1enot valid json\n' | jq --seq -c '.'   # three warnings, one record
  $ printf '\x1e} 5\n'            | jq --seq -c '.'   # warns, then prints 5
  ```
- **jq's numbers are decNumber's, not RFC 8259's.** `1.`, `.5`, `+1`, `nan`, `NaN5`, `snan`
  and `-Infinity` are all numbers to `--seq`'s parser, while `1e` and `1.2.3` are not.

There is also a genuine quirk in jq's classifier that succinctly matches: `check_literal`
reads `tokenbuf[1]` without consulting `tokenpos`, so a bare `n` is measured against
whatever byte an earlier token left behind. `printf '\x1en'` reports `Invalid numeric
literal`, but the same `n` after an `iu` token reports `Invalid literal`.

The value path uses the same reader: values jq hands off before a malformed suffix are
materialized, while values invalidated by the scan call that discovers the error are not.

```
$ printf '\x1e1 {invalid\n' | jq            --seq -c '.'   # warns, then prints 1
$ printf '\x1e1 {invalid\n' | succinctly jq --seq -c '.'   # warns, then prints 1
$ printf '\x1e1,2\n'        | jq            --seq -c '.'   # prints 2
$ printf '\x1e1,2\n'        | succinctly jq --seq -c '.'   # prints 2
```

Verified by `scripts/jq-seq-oracle-sweep.py`, which compares stderr, stdout and exit code
jointly: **0 stderr mismatches and 0 stdout supersets** over 3,062 generated streams per
seed.

Two `--seq` divergences that remain are artifacts of jq's 4096-byte `fgets` *line reader*
rather than its parser, and neither is reachable from a newline-terminated stream:

1. A trailing unterminated RS-record after an earlier newline is dropped unread below the
   buffer boundary and parsed above it (`printf '\x1e1\n\x1e{bad'` is silent, the same input
   with 4,100 bytes of padding warns) — the value-side half of this is the same artifact
   already described below.
2. A record yielding no value makes jq's `jv_parser_next` return invalid-with-no-message,
   which its input loop reads as end-of-input and stops the whole stream — but only when it
   lands in the buffer that hit EOF. `printf '\x1e\x1e"a"'` prints nothing in real jq;
   `printf '\x1e\x1e{"a":1}\n'` prints the object.
3. `jq_util_input_read_more` measures each chunk with `strlen`, so a NUL byte truncates the
   input — again only in a chunk holding no newline. `printf '\x1eA+\x008e'` reports at
   column 3 in real jq (which stopped at the NUL) and column 6 here.

Running the sweep with `--expect-artifacts` puts these shapes back: 28 of 3,063 streams
diverge, and every one is attributable to those artifacts — none unexplained.

A **malformed** BOM — a byte sequence that begins one and then contradicts it, such as
`\xef\xbb` — is *not* in that category and is matched exactly. jq consumes the bytes that did
match without counting them as columns, and then re-runs `parser_reset` at the top of every
read, which leaves it parsing the bytes before the first RS instead of discarding them and
wipes its state after every value, every newline, and at end of input:

```
$ printf '\xef\xbb1 2' | jq --seq -c '.'   # Potentially truncated top-level numeric value at EOF at line 1, column 3
                                          # -- not the abandoned-text template, despite no RS byte anywhere
```

`succinctly jq` reproduces this on both stderr and stdout, including the pre-RS `1` jq reads.

Real-time interleaving of the warnings against stdout is also not reproduced: succinctly
materializes `--seq` input before evaluating, so all warnings precede all values. jq's
default block-buffered stdout produces the same ordering, but `jq --unbuffered` does not.

The differential guard remains deliberately asymmetric: **0 stdout supersets** across the
randomized corpus. A fabricated value is a correctness failure even where an unrelated jq
line-reader artifact still leaves succinctly with extra output.

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
introduced by this work. The malformed-suffix path no longer shares that divergence: it is
driven by [`jq_seq_reader`](../../../src/bin/succinctly/jq_seq_reader.rs), with
[`scripts/jq-seq-oracle-sweep.py`](../../../scripts/jq-seq-oracle-sweep.py) guarding against
stdout supersets.

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

## Real-time stdout/stderr interleaving

Real jq is a lazy generator, so a filter that both writes to stdout and triggers a stderr
side effect (`debug`, `stderr`, `halt_error`) or raises mid-stream interleaves the two as it
goes. `succinctly jq` evaluated a whole input's filter into a `Vec` before writing *any* of
it, so every stderr write had already happened by the time the first stdout write ran —
which no amount of buffering could fix, `--unbuffered`'s per-write `flush()` included
([#1653](https://github.com/rust-works/succinctly/issues/1653)).

Every route now writes each output as the evaluator produces it. `-n`, `--slurp`,
`--input-dsv`, the `input`/`inputs` bridge and the materializing flags (`-S`, `-a`, `-R`,
`--seq`, `--args`) go through `evaluate_input_streaming`; the M2 lazy path — a document read
from a file or stdin, the default — goes through `evaluate_bytes_streaming`, both in
[src/bin/succinctly/jq_runner.rs](../../../src/bin/succinctly/jq_runner.rs), over
`eval_each_with_cursor` in [src/jq/eval_generic.rs](../../../src/jq/eval_generic.rs).

```
$ echo '[1,2]' | jq            --unbuffered -c '.[]|debug'   2>&1
["DEBUG:",1]
1
["DEBUG:",2]
2
$ echo '[1,2]' | succinctly jq --unbuffered -c '.[]|debug'   2>&1     # same
```

Two things about the M2 path are worth stating, because neither is obvious from the output
above.

### Every M2 filter streams — a filter validates only what it reads; recorded on its merits (#2103)

Until [#2103](https://github.com/rust-works/succinctly/issues/2103) the M2 path streamed
only a *cursor-transparent* filter (`expr_is_cursor_transparent`, since deleted). A filter
that could emit the **root** cursor unchanged, or never read it at all — `.,.`, `(., debug)`,
`try (1+1)`, `1+1` — stayed on the eager evaluator, whose wildcard arm materializes the
ambient value through `to_owned_with_cursor` before bridging into `eval.rs`. That one call
was doing two jobs beyond producing a value: **validating the document** (#1194's structural
checks, #1642's colliding-display-key raise) and **choosing an undecodable key's spelling**
(a materialized `String`, so a source `\` doubles). The demand-driven evaluator's native
arms (#1596) do neither, so the two routes disagreed and the gate kept the disagreeing
shapes eager. Every filter now streams; the gate and the eager M2 route are gone.

**This loses fidelity that succinctly previously had, and the decision order does not
license it.** Real jq 1.7.1 rejects every document below at parse time, exit 5, whatever the
filter, so the reference *does* have a behaviour here and it is *reject*. Measured with
`scripts/jq-m2-streaming-sweep.sh` against the pinned binary — 19 rows, all of them a filter
that reads nothing:

| filter                                    | document                                                                        | jq 1.7.1 | eager route (before)   | streaming route (now)      |
|-------------------------------------------|---------------------------------------------------------------------------------|----------|------------------------|----------------------------|
| `1+1`, `try (1+1)`, `try (1+1) catch "x"` | `{123:1,"b":2}`, `{"a":1,"b"}`, `{"a":1, invalid}`, `{"a" 1, "b":2}`, `[1,,3]` | error    | error — **matched jq** | `2` — **diverges**         |
| `1+1`, `try (1+1)`, `try (1+1) catch "x"` | `{"\ud800":1,"\ud800":2}`                                                       | error    | error — **matched jq** | `2` — **diverges**         |
| `.,.`                                     | `{"\ud800":1,"\ud800":2}`                                                       | error    | error — **matched jq** | echoes twice — **diverges** |

[#2692](https://github.com/rust-works/succinctly/issues/2692) extended the same divergence to
every filter that reads only a value's **truthiness**. These rows moved for the same reason
and are recorded against the same amendment:

| filter                                                  | document                            | jq 1.7.1 | before #2692           | now                     |
|---------------------------------------------------------|-------------------------------------|----------|------------------------|-------------------------|
| `true and true`, `1 and 1`, `. and true`, `false or true` | `{123: 1}`, `["\x"]`, `{"\ud800":1,"\ud800":2}` | error | error — **matched jq** | `true` — **diverges**   |
| `false // 1`, `1 // 2`                                  | same three                          | error    | error — **matched jq** | `1` — **diverges**      |
| `not`, `.[] \| not`                                     | same three                          | error    | error — **matched jq** | `false` — **diverges**  |
| `select(.) \| 1`, `if . then 1 else 2 end`              | same three                          | error    | error — **matched jq** | `1` — **diverges**      |
| `any`, `all`                                            | `["\x"]`, `{"\ud800":1,"\ud800":2}` | error    | error — **matched jq** | `true` — **diverges**   |

The last row's document list is shorter on purpose: `any`/`all` iterate an *object's* values
under jq's own last-occurrence duplicate-key rule (#422), which resolves the keys — so
`{123: 1} | any` still raises, because resolving a key reads it. That is the rule applying
inside a single builtin, not an exception to it.

Step 2 of ADR-0018's decision order therefore separates the two options and favours the
behaviour being given up. No rule-4 condition applies — the output is readable, nothing is
corrupted or discarded, and neither choice takes the process down. **This is a deliberate
divergence taken against the order's own answer**, on the same footing as the
`path`/`getpath` entry below (#2168) and sanctioned with it by ADR-0018's #2103 amendment,
and for the same three reasons:

1. *The agreement being given up was an accident.* `to_owned_with_cursor` validated because
   it needed a value to bridge with, not because anyone decided `1+1` should validate its
   input. The check fired only for the spellings that happened to reach the wildcard arm.
2. *It made the answer depend on how the filter was spelled.* On `{"\ud800":1,"\ud800":2}`,
   one binary answered `.` (echo), `.b` (`null`), `length` (`2`), `keys`, and `label $x | .`
   at exit 0, while `.,.`, `1+1`, `debug`, `if . then . else . end` and `. as $x | $x` all
   exited 5 with `object key "\ud800" is ambiguous`. That is the shape #1629/#1642/#2168
   exist to remove.
3. *It cost a whole-document materialization on a filter that reads nothing.* 10 MB
   `json generate` input, indicative single runs: `.,.` went from 0.40 s / 374 MB RSS on the
   eager route to 0.18 s / 25 MB streaming; `1+1` from 0.23 s / 237 MB to 0.01 s / 20 MB.

The rule #2168 recorded — *a builtin that navigates to a position validates only what it
reads; one that materializes validates everything it materializes* — is what decides this.
Since #2692 only its second half is needed, because "reads" turned out to be doing two jobs:
navigating to a position and *testing* a value both leave the bytes undecoded, and only
materializing looks at them. The rule is therefore now stated as **a filter validates only
what it materializes, and everything it materializes**, with no exceptions — `select`, `if`,
`not`, `and`/`or`, `//` and zero-arity `any`/`all` all answer through
`DocumentCursor::is_falsy`, which is O(1) and decodes nothing, and so all validate nothing.
`sort_by`/`unique_by`/`min_by`/`max_by` remain the one validating family, because they
materialize a comparison key for an element they otherwise only reorder.

`1+1` reads nothing and validates nothing. `.,.` forwards the root cursor to the printer,
which walks it exactly as `.` does, so a structural fault is still found where the walk
reaches it (`{123:1,"b":2}` still exits 5 under `.,.`, as under `.`) and a colliding
undecodable key, which the walk never has to resolve, is echoed as `.` echoes it. A filter
whose only route is still the wildcard bridge — `. as $x | $x`, `if . then . else . end`,
`[.]`, `{k: .}`, anything without a native streaming arm **that reads `.`** — still
materializes and so still validates, which is the same rule applied to a route that reads
the whole document, not an exception to it. (`label $x | .` is *not* one of those: it has a
native arm and already answered at exit 0 on the eager route, as the matrix in 2 above
records.) The four examples just named all read `.`, and all still materialize; what
changed under #2173 is the bridge's behaviour for a filter that does **not** — see the
next entry.

The context that makes the loss survivable is the one #2168's entry states: succinctly
already diverges on this whole class of document through `.` itself, deliberately, and a
user who wants jq's rejection has `--validate` and `succinctly json validate`.

**The materializing flag routes still validate whatever the filter — down to `-s` and
`-n`/`input` now, closed by [#2662](https://github.com/rust-works/succinctly/issues/2662)
for `-S`/`-a`/`-C`.** `-S` (sort keys), `-a` (ASCII output) and `-C` (color) used to force
`evaluate_input_streaming`, which materializes the whole input into an `OwnedValue` *before*
evaluating — so `1+1` on `{123:1,"b":2}` exited 5 under any of them where the default route
already answered `2`, purely because those routes materialized the input for historical
reasons, not because sorting/escaping/coloring *output* needs the *input* validated. None of
the three actually need it: `write_output_jq_value` already materializes just the one
*output* value being printed when `sort_keys`/`color_output`/`ascii_output` is set (reusing
`format_json`'s existing `ascii`/`sort_keys` options, which already handled all three
correctly for the pre-existing DOM route), so moving them onto the lazy route costs nothing
in *this* fidelity concern and validates only the value a filter actually reads, same as the
default. (A separate, pre-existing number-formatting gap in that same `format_json` call --
`--preserve-input` silently reformatting numbers under any of `-S`/`-a`/`-C`/`-s` -- predated
this change and was tracked independently as
[#2852](https://github.com/rust-works/succinctly/issues/2852), not fixed here at the time.
**Closed by #2852**: two independent root causes, both in play for the four flags above.
`format_json`/`JsonFormatOpts` had no `jq_compat`/preserve split on a `NumberLiteral` at all
(`-S`/`-a`/`-C`'s own *output*-formatting call, unconditionally reformatting regardless of the
flag) -- now threads `config.jq_compat` through, mirroring the `json_sourced` field's existing
yq-only-meaningful pattern. Separately, `evaluate_input_streaming`'s own input-side reindex
round-trip (needed for `-s`/`--slurp` and any query touching `input`/`inputs`, both of which
must materialize the whole document before the filter can run) always called the
unconditionally-reformatting `OwnedValue::to_json()` on the outer input value -- a *second*
document read via the `input`/`inputs` builtin itself was never affected, since those resolve
from an already-materialized queue that never reaches either `to_json` variant. A new
`to_json_jq_preserve()` (the jq-mode sibling of the existing yq-mode `to_json_yq()`) closes
that half.)
`-s` (slurp) and the `-n`/`input` bridge stay materializing — `-s` needs a real
offset-mapping redesign to slurp without building a DOM (tracked, not done), and `input`
must hand the evaluator an owned value it can return from a builtin by construction, so
validating what it materializes there is the rule working as intended, not an accident left
to close. `-e` streams like the default but materializes each *output* to decide the exit
status, so `-e '.,.'` on `{"\ud800":1,"\ud800":2}` still raises the collision where `-c
'.,.'` echoes — unaffected by this change, `-e` was never one of the routes being moved.
Pinned by `test_materializing_flag_routes_still_validate_2103` (rewritten for the new
`-S`/`-a`/`-C` behavior, `-s`/`-n` rows unchanged).

**`-a` wins over `-r`/`-j` for a *string* value — a real jq quirk, reproduced exactly.**
Confirmed live against jq 1.7.1: `-acr '"café"'` prints `"café"`, quoted and ASCII-escaped,
not the unquoted raw string `-r` alone would give (`-ar '42'` on a non-string value is
unaffected: `42` either way). Both `write_output_jq_value` (the route #2662 moved) and
`write_output` (the still-materializing `-s`/`-n` route) had the same latent gap — `-a -r`
printed the raw, unescaped string bytes on either, before this fix — closed the same way on
both: skip the raw-string branch entirely when `ascii_output` is set, falling through to the
ordinary quoted/escaped write exactly as if `-r` had never been passed.

**A malformed-document failure reached during `write_output_jq_value`'s own materialize
step used to skip jq's diagnostic channel entirely.** Its `sort_keys`/`color_output`
materialize arm converted a decode failure straight to a bare `anyhow::anyhow!`, not a
`MalformedJsonError` — before #2662, unreachable in practice, since `sort_keys`/
`color_output` were the only two flags that could reach that branch at all, and both were
already excluded from the lazy path entirely. Moving `-S`/`-C`/`-a` onto the lazy path here
is what first exercises it: `printf '{invalid}' | succinctly jq -cS .` exited 1 with a bare
`Error: ...` where real jq (and this crate's own default lazy route) exits 5 with `jq: error
(at ...)`. Fixed by wrapping the decode failure in `MalformedJsonError`, the same type
`route_write_error`'s downcast already looks for everywhere else.

**Two `first`/`limit` value spellings moved too, and belong to this same divergence.** On
`[{"x":"\ud800"}]` — an element whose `.x` is a lone high surrogate, which jq rejects at
parse time for every spelling — `first(map(.x) | .[])` and `limit(1; map(.x) | .[])` now echo
the raw value at exit 0, where `main` raised. This is not a new decision: succinctly echoes a
decode-failure value raw on navigation and validates only on materialization
([`docs/plan/decode-failure-routing.md`](../../plan/decode-failure-routing.md)), so `.[].x`,
`first(.[].x)` and the fused `map(.x) | .[]` already echoed this value at exit 0 on `main`
and on `baa26d72e`, while `map(.x)` alone (which materializes the array) raised and still
does. Only `first`/`limit` *wrapping* the fused stream used to fall to a materializing path —
so `main` echoed `map(.x) | .[]` but raised `first(map(.x) | .[])` on the same document, the
spelling-dependence #1629/#1642/#2168 exist to remove. Making them echo is the #2168 answer
(diverge uniformly, not by spelling), the same one the `path`/`getpath` entry below took;
restoring validation here would reintroduce the inconsistency. Pinned by
`test_first_limit_echo_a_decode_failure_value_like_navigation_2103`, and covered in the
sweep by the `[{"x":"\ud800"}]` document.

**The spelling half.** The other job the eager bridge did was pick an undecodable key's
spelling. No new spelling was introduced; the rule the #1642 entry above already states
applies: **raw source bytes wherever the value is never materialized, `key_display_string`'s
fallback (the source `\` doubled) wherever it is.** `.,.` now echoes `{"\ud800":1,"b":2}`
byte-for-byte, like `.`; `-S`, `-s`, `keys` and `debug`'s stderr line still print
`"\\ud800"`, because each builds a `String`-keyed map. `(., debug)` therefore prints the raw
form for `.` and the doubled form for everything `debug` emits, its stdout pass-through
included — consistent under the rule, since `debug` materializes its input to build the
message and forwards what it built. Unifying on raw everywhere would need a raw-key
representation carried through both evaluators, the printers and `yq_runner`'s bridge;
unifying on doubled is the lossy direction and would put materialization back on the hot
path. Neither is planned. A third spelling — the streaming route *raising* `invalid unicode
escape sequence` on `keys_unsorted[]` — was a bug, not a candidate:
`eval_each_pipe_generic`'s empty-stages arm dropped the cursor and decoded the key. Fixed in
the same change.

**#2710 checked the rule against a route that changed under it, and the rule held.**
`scripts/jq-m2-streaming-sweep.expected` recorded the *doubled* spelling for
`. as $x | $x` while the binary emitted the raw one. The binary is right: a plain `$x`
reference forwards the cursor rather than materializing, so raw is exactly what the rule
above prescribes — the wording needs no narrowing, and what had gone stale was the golden,
against a route that became a passthrough. (The mismatch is no longer observable: #2692
regenerated the golden as a side effect of adding filters, and recorded the drift as
pre-existing rather than absorbing it silently. Running the sweep today shows a clean
table — the rows below, not the sweep, are what now holds the answer.) Measured across the boundary on
`{"a\q":1,"b":2}`:

| raw (never materialized) | doubled (materialized) |
|---|---|
| `.`, `. \| .`, `first(.)`, `getpath([])` | `[.] \| .[0]`, `[.] as [$x] \| $x` |
| `. as $x \| $x`, `. as {a:$v} \| .` | `. as $x \| [$x] \| .[0]`, `. as $x \| $x + {}` |
| `if . then . else . end` | `tojson`, `to_entries`, `keys`, `with_entries(.)` |

Every raw row forwards a cursor; every doubled row builds an `OwnedValue`. No filter on
either side contradicts the rule, which is what makes "the golden was stale" the answer
rather than "the rule is too broad". jq 1.7.1 has no opinion to appeal to — it rejects both
documents at parse time — so this is succinctly's own rule throughout.

The sweep is a verification tool, not a CI gate, which is how the drift survived long
enough to be absorbed by an unrelated `--update`;
`test_undecodable_key_spelling_follows_materialization_2710` (`tests/jq_cli_tests.rs`) now
pins both halves of the boundary where CI runs them. The sweep's own `FILTERS` grouping
("materializing eager twin") is a historical label from the two-route era and no longer
describes which routes materialize — several rows in that group forward a cursor today.

Six further cells move off jq's exit 5 as a consequence of that rule, beyond the 19 rows
above: `limit(1; keys_unsorted[])` and `limit(2; keys_unsorted[])` on each of
`{"\ud800":1,"\ud800":2}`, `{"\ud800":1,"b":2}` and `{"a\q":1,"b":2}` now echo the key raw
at exit 0, where the eager route materialized `limit`'s results through
`standard_json_to_jq_value`, decoded the key and raised. That is exactly the inconsistency
the issue's first comment recorded — bare `keys_unsorted[]` echoing while
`limit(1; keys_unsorted[])` decoded, on one binary, one document — and it goes away with the
route that caused it. They are counted separately from the 19 because they follow from the
spelling rule, not from the validity decision.

Pinned by `test_streaming_keys_unsorted_iterate_echoes_undecodable_key_raw_2103`,
`test_root_forwarding_filters_stream_without_validating_2103` and the revised rows of
`test_materializing_route_raises_on_colliding_decode_failure_keys_1642` and
`test_try_catch_contains_a_genuinely_catchable_malformed_key_error_1812`
(`tests/jq_cli_tests.rs`). The #2692 rows are pinned by
`test_truthiness_probes_validate_nothing_2692` (33 documents × every truthiness reader, each
asserted to agree with a reads-nothing reference *and* to have the exit code the row
requires) and its yq twin in `tests/yq_cli_tests.rs`, with the boundary cases in
`test_select_passes_through_corruption_it_only_tests_1645_2692`,
`test_any_all_validate_only_the_keys_they_resolve_2476_2692`,
`test_alternative_validates_only_what_its_operands_read_2476_2692` and the
`select`-answers rows of `test_lazy_validation_boundary_2168`.
`scripts/jq-m2-streaming-sweep.sh`, which found the divergence,
lost its route-against-route comparison along with the eager route. It now runs the one
shipped route against pinned jq 1.7.1 over the same documents and filters and records every
cell — jq's exit code, succinctly's exit code, succinctly's stdout — in a checked-in golden
table, `scripts/jq-m2-streaming-sweep.expected` (`--update` regenerates it). A clean run is
a table that matches; any cell that moves, in either direction, fails with a diff. The rows
above are in that table at their post-#2103 values, so they are pinned rather than merely
tolerated.

### The wildcard bridge stops materializing for a filter that reads nothing (#2173)

[#2103](https://github.com/rust-works/succinctly/issues/2103) above left one route out: a
filter with no native streaming arm still fell to `eval_single`'s wildcard, and the
wildcard's first act is `to_owned_with_cursor` on the *ambient* value — a complete
`OwnedValue` copy of the document, built purely so the eager evaluator has an input to run
against. Since that walk visits everything, it validated everything, so those spellings
kept jq's whole-document rejection while the ones with a native arm had already lost it.
On `{123: 1, "b": 2}`, one binary answered `1+1` with `2` at exit 0 and rejected `[1+1]` at
exit 5. That is the spelling-dependence #1629/#1642/#2168 exist to remove, arrived at from
a third direction.

`bridge_ambient_input` hands a **closed term** — one that `jq::walk::reads_ambient_value`
proves cannot consult `.` — `OwnedValue::Null` instead. Nothing is materialized, so nothing
is validated. Captured against pinned jq 1.7.1 on `{123: 1, "b": 2}`, which it rejects at
parse time for every filter:

| filter                                          | jq 1.7.1 | succinctly before | succinctly now      |
|-------------------------------------------------|----------|-------------------|---------------------|
| `1+1`                                           | error    | `2` (exit 0)      | `2` — unchanged     |
| `[1+1]`                                         | error    | error             | `[2]`               |
| `[range(3)]`, `range(3)`                        | error    | error             | `[0,1,2]`, `0 1 2`  |
| `now\|floor`, `$__loc__`                        | error    | error             | the value           |
| `true and true`, `false or true`, `false // 1`  | error    | error             | `true`, `true`, `1` |

The same three reasons #2103 records apply unchanged, and the first is the strongest here:
*the agreement being given up was an accident.* Nothing decided that `[1+1]` should validate
its input; `to_owned_with_cursor` validated because it needed a value to bridge with, and
`1+1` escaped only because someone had written it a native arm. Sanctioned by ADR-0018's
#2103 amendment, and now listed there as its own instance.

**Widened by [#2699](https://github.com/rust-works/succinctly/issues/2699): a pipe stage
rebinds `.`, so more filters are closed than the predicate used to admit.** `1 | .`,
`[1,2,3] | length`, `reduce (1,2,3) as $x (0; . + $x)` and friends read the previous stage's
output, never the document, and are now recognised as such. Where that reaches a bridge, the
rows above gain siblings — `repeat` bridges its *inner* `f`, so on the same
`{123: 1, "b": 2}`:

| filter                                                   | jq 1.7.1 | before #2699 | now         |
|----------------------------------------------------------|----------|--------------|-------------|
| `[limit(1; repeat(1))]`                                  | error    | `[1]`        | `[1]` — unchanged |
| `[limit(1; repeat(1 \| .))]`                             | error    | error        | `[1]`       |
| `[limit(1; repeat([1,2,3] \| length))]`                   | error    | error        | `[3]`       |
| `[limit(1; repeat(reduce (1,2,3) as $x (0; . + $x)))]`   | error    | error        | `[6]`       |
| `[limit(1; repeat([foreach (1,2) as $x (0; . + $x)]))]`  | error    | error        | `[[1,3]]`   |
| `[limit(1; repeat(.))]` (control, reads)                 | error    | error        | error — unchanged |

Same sanction, same reasoning: which spelling kept the rejection was still an accident, now
one level further in. Pinned by the corresponding rows in
`test_closed_terms_do_not_validate_2173`. The materialization this skips is not small —
on a 13 MB document that filter goes from 354 MB peak and 8.9 s to 30 MB and 1.1 s.

**It also cost what #2103's own point 3 costs.** On a 16 MB `json generate` document,
indicative single runs: `[1+1]` 443 MB / 0.75 s → 29 MB / 0.03 s; `[range(3)]` 446 MB /
0.89 s → 29 MB; `false // 1` 445 MB / 0.86 s → 29 MB. In yq mode, where every filter takes
`eval_single`, `1+1` went 476 MB / 0.90 s → 126 MB / 0.11 s on a 30 MB document. The
baseline for both is a filter that already had a native arm.

**What did not change.** A filter that reads `.` still materializes and still validates
everything it materializes — `. as $x | $x`, `if . then . else . end`, `[.]`, `{k: .}`,
`. and true`, `range(length; 3)`, and `.` itself all keep their exit 5. So do the
materializing flag routes (`-S`, `-a`, `-s`, `-C`, `-n`), which never reach these sites and
are #2662's. And a document whose *root* the reader cannot delimit at all — `xyz123`,
`[1,2`, `["a`, `[1] x` — still fails for every filter including `empty`, because there is
no value to skip reading; that is the reader, not a filter, and it matches jq.

**#2476's arms moved with it.** `and`/`or`/`//` had been given `ambient_validation_error`
— the same rejection as a walk that allocates nothing — days earlier, for the same shapes.
Keeping it there while the bridge stopped raising would have re-created the split by hand,
so the walk is now charged only to an operand that reads `.`. `not` is unchanged: it reads
`.`'s truthiness, so it validates.

Pinned by `test_closed_terms_do_not_validate_2173` (12 closed spellings x 6 malformed
documents, plus 5 reading spellings that must still raise), the split probe lists in
`test_ambient_validation_agrees_with_bridge_2476` and its yq twin, and
`test_wildcard_bridge_over_alias_fanout_completes_2173`. The predicate's own soundness —
if it calls a filter closed, the filter's output must not depend on the document — is
`tests/jq_closed_term_tests.rs`.

**Widened again by [#2794](https://github.com/rust-works/succinctly/issues/2794): `isempty(f)`,
`any(gen; cond)` and `all(gen; cond)` are argument-transparent, so a closed argument closes
the whole builtin.** `node_reads_ambient`'s `Builtin` arm was a negative allowlist that
forced `true` for every builtin except ten, regardless of its arguments — right for a
builtin that reads `.` itself (`length`, `add`, ...), wrong for one whose whole answer comes
from a generator it consumes. `isempty(f)`'s argument sees the true ambient, so it joins the
allowlist directly; `any(gen; cond)`/`all(gen; cond)` desugar to `gen | cond` (confirmed
live: `any(1,2,3; .a > 1)` on `{"a":99}` errors "Cannot index number with string \"a\"",
naming gen's own numeric output, not the document), the same rebinding `reduce`'s `update`
and `foreach`'s `extract` already get, so they need a dedicated `reads_ambient_value` arm
rather than the flat allowlist. On the same `{123: 1, "b": 2}`:

| filter                          | jq 1.7.1 | succinctly before | succinctly now |
|----------------------------------|----------|--------------------|-----------------|
| `isempty(1)`, `isempty(empty)`  | error    | error              | `false`, `true` |
| `any(range(3); . > 1)`          | error    | error              | `true`          |
| `all(range(3); . >= 0)`         | error    | error              | `true`          |
| `isempty(1 \| .)`                | error    | error              | `false`         |

The last row is its own instance of the #2699 widening, one level in: `isempty`'s own arm
recurses through `reads_ambient_value` rather than the flat allowlist specifically so a
rebinding construct nested inside its argument (a pipe, a `reduce`, a `foreach`) gets the
same refinement a top-level filter would, not just a bare `.`.

Measured on a 96 MB `json generate` document (Apple M4 Pro, release build; the baseline for
a filter that reads nothing is ~110 MB): `isempty(empty)` 2020 MB → 110 MB,
`isempty(range(9))` 2020 MB → 110 MB, `isempty(1 | .)` 2118 MB → 110 MB,
`any(range(3); . > 1)` 3763 MB → 110 MB, `all(range(3); . >= 0)` 3764 MB → 110 MB.

`any`/`all`'s arity-0/1 forms (`any`, `all`, `any(cond)`, `all(cond)`) are deliberately
unaffected: those desugar to an implicit `.[]` generator and genuinely read `.` on their own
account. Pinned by the same `test_closed_terms_do_not_validate_2173` and
`tests/jq_closed_term_tests.rs` the #2699 rows above are.

### Arithmetic and unary minus validate only what they read (#2626)

[#2173](https://github.com/rust-works/succinctly/issues/2173) above stopped the wildcard
bridge materializing for a *closed* term. `eval_single`'s `Expr::Arithmetic` and
`Expr::Negate` arms were still gated on `needs_path_context`, so an arithmetic that **does**
read `.` — `length + 0` — kept falling to that bridge and validating the whole document,
while the same read spelled without the arithmetic did not. That is the same
spelling-dependence one level in: not between filters, but between two spellings of one.

On `{"a": "bad\x", "b": 5}`, which jq 1.7.1 rejects at parse time for every filter:

| filter          | jq 1.7.1 | succinctly before | succinctly now |
|-----------------|----------|-------------------|----------------|
| `length`        | error    | `2` (exit 0)      | `2` — unchanged |
| `length + 0`    | error    | `2` — unchanged   | `2` — unchanged |
| `[length + 0]`  | error    | error             | `[2]`          |
| `(-length)`     | error    | error             | `-2`           |
| `.b + 0`        | error    | `5`               | `5` — unchanged |

Dropping the gate is what closes it: both arms now take their native, cursor-threaded route
on every input, so nothing materializes and nothing validates beyond what the operands
actually read. Nothing replaces the bridge's ambient decode, and that is the point rather
than an omission — it is the accident #2173's own entry describes, arrived at from a fourth
direction. An operand that *does* read the malformed scalar still raises: `.a + ""` and
`(.a + "") | length` both exit 5, exactly as bare `.a` does.

`eval_each_generic`'s own `Expr::Negate` arm bridged too, and had to move with them, or
`-length` and `first(-length)` would have disagreed.

Sanctioned by ADR-0018's #2103 amendment, like the two entries above. Pinned by
`test_arithmetic_validates_only_what_it_reads_2626` in `tests/yq_cli_tests.rs` (where the
duplicate-key half of #2626 is also pinned) and by the arithmetic rows added to
`test_closed_terms_do_not_validate_2173`'s closed-probe lists in both suites.

### `range` bounds validate only what they read (#2698)

The same mechanism as the [#2626](https://github.com/rust-works/succinctly/issues/2626) entry
above, one construct over. Both of `range`'s dispatch sites reached the eager evaluator
through `bridge_ambient_input`, so a bound that reads the document — `range(length)`,
`range(0; .b)` — materialized a full `OwnedValue` copy of it, and that walk validated every
byte. [#2698](https://github.com/rust-works/succinctly/issues/2698) gives both sites a native,
cursor-threaded arm (`each_range_generic`), so the bound now reads exactly what it reads and
nothing else is validated.

On `{"a": "bad\x", "b": 5}`, which jq 1.7.1 rejects at parse time for every filter:

| filter                                | jq 1.7.1 | succinctly before | succinctly now |
|---------------------------------------|----------|-------------------|----------------|
| `length`                              | error    | `2` (exit 0)      | `2` — unchanged |
| `[range(3)]`                          | error    | `[0,1,2]`         | `[0,1,2]` — unchanged |
| `[range(length)]`                     | error    | error             | `[0,1]`        |
| `first(range(length))`                | error    | error             | `0`            |
| `[range(.b)]`                         | error    | error             | `[0,1,2,3,4]`  |
| `reduce range(length) as $x (0;.+$x)` | error    | error             | `1`            |
| `[range(.a \| length)]`               | error    | error — unchanged | error — unchanged |

The last row is the rule stated positively: a bound that *does* read the malformed scalar
still raises, exactly as bare `.a` does. Only the validation the bridge performed as a side
effect of building an input it never needed is gone — the accident #2173's own entry
describes, arrived at from a fifth direction. Same in yq mode (`--jq-extensions`), where
real yq rejects the document at read time and succinctly now answers.

**This entry exists because the first version of the PR claimed the opposite.** It was
checked against `{123: 1, "b": 2}` — a *structural* fault at the root, which `length` itself
reads and so still raises on both binaries — and that was taken as "unchanged in both
directions". A fault the bound does *not* read is the case that moves, and the check has to
be built from one. The #2797 review caught it with a bad escape inside a leaf value.

Sanctioned by ADR-0018's #2103 amendment, like the entries above. Pinned by the `range`
rows in `test_closed_terms_do_not_validate_2173` (jq) and
`range_bounds_validate_only_what_they_read_2698` (yq).

### A fault found by walking to it leaves the prefix on stdout

succinctly is a semi-index and finds a malformed document only when the walk reaches the
fault, so outputs produced before it are already written. Real jq emits nothing at all here,
because its strict parser rejects the document before any filter runs:

```
$ printf '[1,,3]' | jq            --unbuffered -c '.[]'   2>&1
jq: parse error: Expected value before ',' at line 1, column 4      # exit 5, no stdout
$ printf '[1,,3]' | succinctly jq --unbuffered -c '.[]'   2>&1
1                                                                   # exit 5, with the prefix
jq: error (at <stdin>:0): Invalid JSON text: expected JSON value, found ','
```

(`--unbuffered` only so the merged order above is deterministic; the prefix is on stdout
either way.)

The exit code agrees; only the prefix differs. This is the same shape
[#1770](https://github.com/rust-works/succinctly/issues/1770) already accepted for
`limit(2; keys_unsorted[])`, which exits 5 with `"a"` already on stdout — see "A truncating
consumer of `keys_unsorted[]` skips a malformed member it never needed" above. Suppressing it
would mean validating the whole document up front, which is the cost semi-indexing exists to
avoid.

Pinned by `test_unbuffered_interleaves_stdout_and_stderr_1653`,
`test_array_iterate_lazy_skips_later_malformed_comma_1597` and
`test_jq_missing_delimiter_raises_through_nonreserializing_filters_1677`
([tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs)).

The same trade-off also covers a fault found *partway through writing a single result's
own value*, not just one discovered by a later top-level result's generator advance —
`print_json`/`write_output_jq_value` stream byte-by-byte with no rewind, so `{"a": [,]}`
under bare `.` writes `{"a":` before its own recursive walk reaches the stray comma nested
inside the empty array and raises (#2210), the identical shape `test_jq_identity_on_
malformed_array_element_errors_1641` above already pins for `[xyz123]`/`[tru]`/
`[1,zzz,3]`/`{"a": xyz123}` — a bareword-garbage token instead of a stray comma, reaching
the fault through the same writer the same way. A first pass at this fix buffered
`write_output_jq_value`'s per-call output and only committed it to real stdout once known
good (mirroring `evaluate_m2_fast_path`'s own identical contract for its own single-result
case) — reverted once the existing #1641 test above caught it as a real regression against
an already-established, deliberately tested contract for this exact writer, not a
previously-undocumented gap. It would also have needed carving out `--unbuffered`, whose
own flush (`write_terminator`) is embedded inside this same call tree keyed on whatever
writer it's given — buffering would make that flush a silent no-op on a temporary buffer
instead of the real, immediate per-value flush `--unbuffered` promises, breaking its
interleaving guarantee with side effects from later results. Pinned by
`test_jq_general_streaming_path_leaks_prefix_before_nested_comma_fault_2210`
([tests/jq_cli_tests.rs](../../../tests/jq_cli_tests.rs)).

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

### Exit code is sticky (any-error) across multi-document input — accepted divergence, ADR-0018 rule 4 (#1855)

`succinctly jq`'s exit code is a **sticky any-error** flag: once any uncaught error is
reported anywhere during the run, the final exit code is forced to 5
(`ErrorSink::hit()`, checked once after the whole input loop finishes, `jq_runner.rs`).
Real jq's exit code instead reflects only the **most recently processed top-level input
document's own outcome** — an error on an earlier document does not affect the exit code
if a later document processes cleanly:

```console
$ printf '{"a":1,"b":0}\n{"a":4,"b":2}\n' | jq '.a / .b'
2
$ echo $?
0                              # first doc errored, second succeeded -- exit 0
$ printf '{"a":1,"b":0}\n{"a":4,"b":2}\n' | succinctly jq '.a / .b'
2
$ echo $?
5                              # sink.hit() stays sticky from the first document
```

The stickiness *does* apply across multiple outputs produced by one document
(`.[] | 1/.`), but real jq resets it at the next top-level document boundary; succinctly's
`sink.hit()` never resets for the life of the run. [#1830](https://github.com/rust-works/succinctly/issues/1830)'s
`--raw-output0` NUL-content check inherits the same stickiness through the same
`ErrorSink`/`sink.hit()` convention every other jq error in this file already uses — it is
not a separate divergence, just another error class that can trigger this one:

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

**This is a deliberate, considered divergence, not an oversight.** [#355](https://github.com/rust-works/succinctly/issues/355)
originally found `succinctly jq` exiting 0 on *every* error — a strictly worse bug than the
one described here — and its fix introduced today's sticky-any-error behavior on the
reasoning that a shell script checking `$?` after a multi-document run generally wants "did
*anything* fail," not just "did the *last* thing fail." [#1855](https://github.com/rust-works/succinctly/issues/1855)
re-examined that decision, implemented and fully verified a jq-exact (last-document-wins)
fix, and then closed as won't-fix rather than merge it: none of ADR-0018's four permitted
divergence conditions literally cover this case (jq's own output is readable and nothing is
corrupted), so per rule 4 it is recorded here as an intentional exception on its merits —
the same footing as the `docs/plan/decode-failure-routing.md` Stage-4 precedent immediately
above, and unlike that one, this one has a name: it is a user-facing CLI exit-code contract
that real scripts may already depend on in its current form, and reversing it is a behavior
change rather than a bug fix in the ordinary sense. `succinctly yq`'s own exit-code semantics
are unaffected by any of this — real yq genuinely *is* sticky-any-error across documents,
unlike jq, so `yq_runner.rs`'s own use of `sink.hit()` at its final exit-code check already
matches and needs no exception recorded in [yq Limitations](../yq/limitations.md).

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

### `repeat(f)` under `limit`/`first` (value mode): fixed (#2014); the eager fallback and path mode still cap, in two different ways

> Both `MAX_ITERATIONS` raises are uncatchable by `?`/`try`/`catch` since #2132 -- see "Every resource cap is uncatchable" below.

`repeat(f)` (`def repeat(f): f, repeat(f);`) has no base case at all: an `f`
that never errors and never produces a value on some round recurses forever.
Confirmed live that real jq itself hangs on this rather than raising or
terminating:

```console
$ timeout 3 jq -cn 'limit(3; repeat(empty))'; echo "exit: $?"
exit: 124                                 # real jq hangs -- 124 is `timeout`'s own code
```

Before [#2014](https://github.com/rust-works/succinctly/issues/2014), value
mode's `eval_repeat` ran `repeat`'s body *eagerly*, capped at a flat
`MAX_ITERATIONS = 1000` rounds regardless of a wrapping `limit`/`first`'s own
requested count, and silently returned whatever it had accumulated once that
cap was hit -- so `limit(1500; repeat(1))` returned only 1000 values, exit 0,
no error, where real jq returns all 1500. #2014 gave `eval_each`'s lazy
dispatch (in both evaluators -- see the module-level note on the two
evaluators elsewhere in this repo) a native `Expr::Repeat` arm
(`each_repeat`/`each_repeat_generic`) that pulls one round at a time and
stops exactly when the wrapping consumer's own `Demand::Stop` does, so a
`limit`/`first`-bounded `repeat` is now uncapped in round count and matches
jq exactly, confirmed at every scale the original issue's own table named:

```console
$ succinctly jq -cn '[limit(1500; repeat(1))] | length'
1500
$ succinctly jq -cn '[limit(80000; repeat(1))] | length'
80000
```

Two narrower caps remain by necessity, both accepted under ADR-0018 rule 4c
(preventing a hang, not matching one) rather than removed:

- **A per-*round* width budget** (`REPEAT_WIDTH_BUDGET = 10000`, reset
  at the start of every round): bounds how many values a *single* round may
  fork into at once -- a memory-safety net for a wide `f` like `.[]`, since
  a round is materialized into a `Vec` before being handed to the sink. Real
  jq has no such cap (`limit(50000; repeat(.[]))` over a 20001-element array
  succeeds with all 50000 in jq 1.7.1); succinctly raises
  `"repeat: maximum iterations exceeded"` once a single round's own output
  passes 10000. This is scoped to one round, not the whole stream, so it
  does not reintroduce the round-count cap #2014 removed -- a `repeat`
  producing a handful of values per round runs indefinitely regardless of
  total round count.
- **A round-*emptiness* cap** (`MAX_EMPTY_REPEAT_ROUNDS = 1000` *consecutive*
  empty rounds, reset by any productive round): `repeat`'s `expr` reruns
  every round, so a round producing zero outputs will, if `expr` is pure
  over the repeated value, produce zero outputs forever -- no wrapping
  `Demand::Stop` can ever fire to stop it, since `sink` is never called on
  an empty round. Exhausting this silently ends the stream (matching
  `repeat(empty)`'s hang-instead-of-diverging precedent above) rather than
  raising. **This cap is inherently approximate for an *impure* `f`** (one
  that reads `input`/`inputs`, so a later round can differ even though the
  repeated value itself does not): a body that happens to need 1000 or more
  consecutive empty rounds before its first real output is silently cut
  short with no error, exactly like the width budget above but with no
  diagnostic at all. Any finite bound has this same edge at whatever number
  it picks -- raising the constant only moves the cliff, per this section's
  own prior finding about the old round-count cap -- so it is recorded here
  as an accepted, structural limitation rather than something a bigger
  number would fix.

The **eager fallback** -- `repeat(f)` reached through any consumer that
doesn't dispatch through `eval_each`'s lazy path (bare `[repeat(f)]`,
`reduce repeat(f) as $x (...)`, `foreach repeat(f) as $x (...)`,
`last(repeat(f))`) -- still runs `eval_repeat`'s original eager, capped loop,
since none of those consumers can express a demand to stop early. #2014 also
changed *its* cap-exhaustion behavior, from the same silent truncation
described above to raising `"repeat: maximum iterations exceeded"`:

```console
$ succinctly jq -cn '[repeat(1)] | length'
jq: error (at <unknown>): repeat: maximum iterations exceeded
$ succinctly jq -cn 'last(repeat(1))'
jq: error (at <unknown>): repeat: maximum iterations exceeded
```

Real jq hangs on all four of these shapes instead (there is no wrapping
consumer to stop it early, and no #2014 fix changes that). Raising is a
*third* outcome -- neither the old silent truncation nor jq's hang -- chosen
deliberately: ADR-0018 rule 4c permits not matching a hang, and a loud,
immediate error is strictly preferable to either alternative for these four
shapes, which have no legitimate large-`n` use case in the first place
(nothing here can ever stop the eager loop early, unlike the `limit`/`first`
case above).

**Path mode is unchanged and still has the original bug.**
[#2014](https://github.com/rust-works/succinctly/issues/2014) only touched
value mode's two evaluators; `resolve_repeat_sink` (path mode, the
`path(repeat(f))`/`path(limit(n; repeat(f)))` route added by
[#1906](https://github.com/rust-works/succinctly/issues/1906)) still runs
its own flat `MAX_ITERATIONS = 1000` round cap regardless of the wrapping
consumer's requested count, and still silently truncates on exhaustion:

```console
$ succinctly jq -cn 'path(limit(1500; repeat(.))) | length' # counts output lines
1000
```

This is the same bug value mode had before #2014, now isolated to path mode
alone -- left open rather than fixed here since #2014's own scope and
verification are both about value mode; a follow-up applying the identical
demand-driven treatment to `resolve_repeat_sink` would close this the
same way.

### `reduce`/`foreach`'s own step budget (#695/#2079): bounds genuine fanout, not ordinary element count

> The budget's raise is uncatchable by `?`/`try`/`catch` since #2132 -- see "Every resource cap is uncatchable" below.

`REDUCE_FOREACH_MAX_STEPS` (`src/jq/eval.rs`) is a shared step budget charged
against `reduce`/`foreach` once per UPDATE eval, and -- `foreach` only --
once more per EXTRACT eval, independently in value mode and path mode.
[#695](https://github.com/rust-works/succinctly/issues/695) introduced it
(originally `10000`) as a resource-exhaustion guard against a genuinely
unbounded product: `reduce (range(100000)) as $x ((range(100000)); .+$x)`
forks INIT into 100,000 independent runs, each walking a 100,000-element
stream -- 1e10 round trips, with no cap at all before #695.

Charging one flat unit per invocation regardless of *why* it happened
degenerates into a plain cap on input-element count for the overwhelmingly
common single-INIT-fork, single-output shape -- not the multiplicative fanout
#695 was actually guarding against, and not something real jq caps at all
(it streams the fold, bounded only by memory).
[#2079](https://github.com/rust-works/succinctly/issues/2079) found the
original `10000` refusing everyday folds as a result (`reduce .[] as $x (0; .
+ $x)` on a 50,000-element array; `succinctly` refused, real jq answered
`1249975000`), with `foreach`'s three-argument form (explicit EXTRACT)
compounding it further -- EXTRACT's second charge per element halved its
effective ceiling again, so a filter's element limit silently depended on
whether it passed two or three arguments to `foreach`.

**Fix**: split the previously-coincidental sharing between this budget and
`repeat`'s own unrelated per-round width cap (`REPEAT_WIDTH_BUDGET`, previous
section — raising one to fix #2079 must not silently loosen the other), then
raised `REDUCE_FOREACH_MAX_STEPS` 10x, `10000` to `100000` -- 2x above this
issue's own 50,000-element repro. Accepted under ADR-0018 rule 4c (preventing
a resource-exhaustion vector, not matching real jq's own memory-only bound)
rather than removing the cap outright.

**Why 10x and not more, and why a flat count rather than a true fanout-product
bound**: raising the flat per-invocation count also raises the worst-case CPU
burned *before* the cap fires, for any UPDATE/EXTRACT body whose own
per-invocation cost is superlinear -- concretely, repeated string growth:
`reduce range(N) as $x (""; . + "x")` originally measured O(N²) in succinctly
(`""` concatenation re-serialized and reparsed the *whole* accumulator every
step, via `eval_owned_input`'s general-evaluator fallback -- `Expr::Arithmetic`
wasn't one of `eval_owned_fast_path`'s covered shapes), where real jq answers
the identical query in apparently-linear time (`jq -cn '[range(N)] | length'`-scale
cost, confirmed live: N=50,000/100,000/1,000,000 measure 0.01s/0.02s/0.15s
against jq 1.7.1, vs. succinctly's original 1.9s/7.8s at just the first two of
those). [#2086](https://github.com/rust-works/succinctly/issues/2086) fixed
the specific mechanism named above (the JSON round-trip) for exactly this
bare shape (`. + <literal>`, `$x` already folded into a `Literal` by
`substitute_vars` before the fold runs) by extending `eval_owned_fast_path`
to answer it directly against the `OwnedValue` tree, reusing the same
`arith_combine` dispatch every other arithmetic call site already shares --
measured directly at `REDUCE_FOREACH_MAX_STEPS` (`100000`) after that fix:
~0.17s, down from ~7.8s, a ~45x constant-factor win.

**#2086 was not an asymptotic fix -- the fold remained O(n²), just with a
much smaller constant** (the new arm still called `input.clone()` on the
whole accumulator every step before handing it to `arith_combine`, since
`eval_owned_fast_path`'s `&OwnedValue` signature left no other option, and
`String`/`Vec::clone()` allocates at exact capacity with no spare headroom,
so the very next `push_str`/`extend` inside `arith_add` reallocated a
second time regardless -- two O(current-size) copies per step either way,
just memcpy instead of serialize/parse).

[#2157](https://github.com/rust-works/succinctly/issues/2157) fixed the
remaining O(n²) shape for `try_reduce_step_alternatives`'s own fold loop,
where the accumulator `state: OwnedValue` is confirmed unaliased
(unconditionally overwritten immediately after this call, with no live
borrow of the old value surviving) -- via a new `try_owned_accumulator_step`
that recognizes the same `. + <literal>`-shaped UPDATE body
(`owned_arith_accumulator_shape`) but, unlike `eval_owned_fast_path`, takes
`state` *by value* and moves it straight into `arith_combine`, so
`arith_add`'s in-place `push_str`/`extend`/`Object` merge can reuse the
accumulator's own spare capacity instead of being handed a fresh
exact-capacity clone every step. `eval_owned_fast_path`'s own arm (and its
other 3 call sites -- `eval_each_owned`, `eval_owned_expr_full`,
`eval_owned_input`, none of which hold state this unaliased) is deliberately
left unchanged and still clones; the fix is scoped to the fold loop, not a
signature change to the shared function.

Measured post-fix scaling curve (`reduce range(N) as $x (""; . + "x") |
length`, wall-clock via bash's `time` builtin, output verified identical to
pre-fix at every N):

| N      | pre-#2157 | post-#2157 |
|--------|-----------|------------|
| 6,250  | 6ms       | 6ms        |
| 12,500 | 9ms       | 7ms        |
| 25,000 | 18ms      | 10ms       |
| 50,000 | 38ms      | 16ms       |
| 99,999 | 135ms     | 29ms       |

Pre-fix time roughly doubles-plus per doubling of N throughout (the O(n²)
signature); post-fix growth is close to linear once the ~5-6ms
process-startup floor is netted out (net time 6,250->99,999: ~1ms->~24ms,
a 16x range of N for a ~24x time increase, vs. pre-fix's ~1ms->~130ms net,
a ~130x increase over the same range).

**`try_foreach_step_alternatives` shares the identical `try_owned_accumulator_step`
call (the same unaliased-`state` reasoning applies equally to its own fold
loop) but does not reach the same O(n) result, and this doc originally
claimed otherwise -- corrected here per review.** `reduce` only ever reads
its accumulator once per step (as the next step's input); `foreach` reads
it *twice* -- once to hand to EXTRACT (or push directly to `outputs` when
EXTRACT is omitted, `foreach`'s default per-step-emit behavior), and again
as the next step's `state` -- and a plain by-value move can eliminate only
one of the two reads, not both, since each needs its own owned copy. A
first version of this fix moved `state` into `try_owned_accumulator_step`
but then still cloned `update_vals.last()` for the next `state` *after* the
EXTRACT loop had already needed to borrow `update_vals` -- paying both
clones anyway (review caught this: interleaved benchmarking of that version
against the pre-fix baseline showed no measurable difference, confirming
zero benefit had actually landed). The corrected version defers taking
`state` from `update_vals` until after the EXTRACT loop no longer needs to
borrow it, so that one read becomes a real move -- landing one clone per
step instead of two, not zero. Measured (`foreach range(99999) as $x (""; .
+ "x") | empty`, wall-clock, 5 interleaved reps): pre-fix ~6.48s average,
post-fix ~6.05s average, a real but modest **~7%** improvement -- nowhere
near `reduce`'s near-linear result, because the EXTRACT/output-emit clone
is structurally unavoidable here and dominates the total cost regardless.
Emitting a growing accumulator once per step is itself O(n²) in total
output size no matter how efficiently `state` is threaded internally (the
same "quadratic by output shape" property noted elsewhere in this
codebase's own benchmarking discipline) -- genuinely fixing `foreach`'s own
asymptotic behavior would need `eval_owned_fast_path` extended to cover
common EXTRACT shapes too (starting with `empty`, currently absent from its
match and so still round-tripping through the reindex bridge per step
regardless of this fix), which is out of scope here.

**Bonus, not part of #2157's own stated scope**: `owned_arith_accumulator_shape`
calls the same `literal_shaped_expr_to_owned` [#2152](https://github.com/rust-works/succinctly/issues/2152)
already extended to recognize literal-shaped `Expr::Array`/`Expr::Object`
right-hand sides, so the array/object-accumulator idioms #2152 closed at
the *constant-factor* level (still cloning via `eval_owned_fast_path`) get
the same by-value treatment `reduce`'s own fold loop gets above, for free --
confirmed live: `reduce range(25000) as $x ([]; . + [$x]) | length` dropped
from ~1.25s to ~0.013s (~96x), and `reduce (range(5000) | tostring) as $x
({}; . + {($x): $x}) | length` dropped from ~0.42s to ~0.010s (~42x), both
with output verified identical pre/post-fix. A dynamic object key that
itself isn't literal-shaped after substitution (e.g. `($x | tostring)`
evaluated *inside* the fold body rather than on the outer generator) still
falls through to the slow general path in both builds -- `literal_shaped_expr_to_owned`
was, and remains, unchanged by #2157.

**One correction to the AST shape this issue's own text guessed, confirmed
live via a debug probe against the real repro rather than assumed**: a
*single*-element array literal like `[$x]` does **not** wrap its substituted
element in `Expr::Comma` -- `parse_array_construction` only introduces a
`Comma` node when an actual `,` token is present inside `[...]`, so `[$x]`
parses as `Expr::Array(Box::new(Expr::Var("x")))`, and after `substitute_var`
folds `$x` into a `Literal` the shape is
`Expr::Array(Box::new(Expr::Literal(...)))` -- a *bare* `Literal` directly
inside `Array`, not `Expr::Array(Box::new(Expr::Comma(vec![Expr::Literal(...)])))`
as this issue's own "Suggested fix direction" section stated. `literal_shaped_expr_to_owned`
handles both shapes (a bare non-`Comma` inner expression, and an actual
multi-element `Comma`), confirmed by a dedicated unit test
(`test_2152_literal_shaped_expr_to_owned_single_element_array_has_no_comma_wrapper`)
rather than relying on the single repro's own AST happening to exercise the
right arm. A precomputed fanout-product bound
(INIT-fork count x element count, both known before either loop starts)
would not avoid this either way -- for the single-INIT-fork case #2079
reports, fork count is 1, so the product collapses to the element count and
the effective ceiling is identical to the flat count's; a product bound only
changes *when* multi-fork cases are rejected (upfront vs. after partial
work), not the single-fork ceiling this issue is about. A materially better
bound would need to weight each invocation by its own body's cost, not just
count invocations -- out of scope here; `10000 -> 100000` was chosen as a
value with real headroom over #2079's own repro while keeping the
pathological-body worst case in the single-digit seconds (measured, not
extrapolated) rather than the tens of minutes a further 10x would cost at
the same pathological shape.

### `while`/`until`'s own step budget (#534/#2087): the identical bug #2079 already fixed for `reduce`/`foreach`

> The budget's raise is uncatchable by `?`/`try`/`catch` since #2132 -- see "Every resource cap is uncatchable" below.

`WHILE_UNTIL_MAX_STEPS` (`src/jq/eval.rs`) is a shared step budget for
`while`/`until`'s backtracking-generator evaluation, decremented once per
state visited across the whole recursion tree regardless of branching --
[#534](https://github.com/rust-works/succinctly/issues/534) introduced it
(originally `10000`) as the same class of resource-exhaustion guard
`REDUCE_FOREACH_MAX_STEPS` was introduced for (previous section): without
it, a multi-output `cond`/`update` forks the rest of the loop per output,
and a genuinely all-forking loop is unbounded.

Exactly the same degeneration [#2079](https://github.com/rust-works/succinctly/issues/2079)
found for `reduce`/`foreach` applies here unchanged: a flat count charged
once per state visited, regardless of whether the visit came from genuine
branching or an ordinary single-output loop, collapses into a plain cap on
iteration count for the overwhelmingly common non-forking case -- not the
fanout explosion #534 was actually guarding against, and not something real
jq caps at all. [#2087](https://github.com/rust-works/succinctly/issues/2087)
found the original `10000` refusing an everyday counting loop as a result:

```console
$ echo 'null' | jq -c '0 | until(. >= 50000; . + 1)'
50000
$ echo 'null' | succinctly jq -c '0 | until(. >= 50000; . + 1)'
jq: error (at <stdin>:1): until: maximum iterations exceeded
```

**Fix**: raised `WHILE_UNTIL_MAX_STEPS` 10x, `10000` to `100000`, matching
#2079's own precedent exactly -- same 2x-headroom-over-the-repro reasoning,
same "flat count over a true fanout-product bound" tradeoff (a product
bound wouldn't change the single-fork ceiling this issue is about, only
when a genuinely multi-fork case is rejected), same ADR-0018 rule 4c
acceptance (bounding a resource-exhaustion vector, not matching real jq's
own memory-only bound).

**The same superlinear-cost caveat #2079/#2086 already found for
`reduce`/`foreach` applies here too, via the identical underlying
mechanism** (`eval_owned_input`'s general-evaluator fallback re-serializes
and reparses the whole accumulator every step, for any `Expr::Arithmetic`
shape `eval_owned_fast_path` doesn't cover): `[0,""] | until(.[0] >= N;
[.[0]+1, .[1] + "x"]) | .[1] | length` measures ~15-16s at N approaching the
new `100000` ceiling in this build, against real jq's own apparently-linear
~0.1-0.2s at the same N (confirmed live). #2086's fix (see the previous
section -- a constant-factor win, not an asymptotic one; #2157 fixed the
remaining O(n²) shape for `reduce`'s own fold loop specifically (see the
previous section's own correction for why `foreach`'s otherwise-identical
call site doesn't reach the same result), not `eval_owned_fast_path`
itself, so `until`/`while`'s separate step mechanism below is unaffected by
it either way) brought that analogous `reduce` figure down from ~7.8s to
~0.17s for `reduce`/`foreach`'s *bare* accumulator shape (`. + <literal>`)
-- but this `until`/`while` repro's own state is
array-wrapped (`[.[0]+1, .[1] + "x"]`, since a loop needs a counter *and* an
accumulator, unlike `reduce`/`foreach`'s cleanly separate INIT-vs-`$x`
shape), so neither inner arithmetic's left operand is the bare `.`
`eval_owned_fast_path`'s new arm requires -- confirmed still ~15s after
#2086 landed, not improved by it at all. A *bare* single-value `until`/
`while` state (`"" | until(length >= N; . + "x")`) does hit the new fast
path for its UPDATE, but only partially helps (~7.5-8.4s, confirmed live) --
`until`/`while`'s own COND (`length >= N` here) is evaluated fresh every
step too, and `length` isn't one of `eval_owned_fast_path`'s covered shapes
either, so COND evaluation remains its own uncounted, unfixed round-trip.
Tracked as [#2152](https://github.com/rust-works/succinctly/issues/2152)
(array/object accumulation) and noted here rather than filing a third,
narrower until/while-COND-specific issue for what is the same underlying
"which `Expr` shapes does `eval_owned_fast_path` cover" question.

### A generator-argument expression used to fan out normally, then silently narrow to a single array the moment `key`/`parent`/`file_index` showed up anywhere else in the same pipe -- fixed for all 4 sites

[#1277](https://github.com/rust-works/succinctly/issues/1277)'s clusters 1-3
(closed by [#1522](https://github.com/rust-works/succinctly/issues/1522)/[#1279](https://github.com/rust-works/succinctly/issues/1279))
gave generator-argument builtins real fan-out: a builtin whose argument is a
generator now produces one output per argument output, matching real jq.
Four call sites inside `eval_pipe_with_path_context_internal`
(`src/jq/eval.rs`; that evaluator was deleted by spine 2416's exit, and the
surviving routes fan out natively) -- `ParentN`'s own `n` argument, the `Expr::Builtin(_)`
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
`eval_pipe_with_path_context_internal` since `key` needed path tracking (that
evaluator is gone since spine 2416's exit; the shape now fans out on the
walk) --
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

**The output above is unchanged since #2073, but the mechanism behind it
moved.** That fix stopped `Expr::Optional` handing `rest` a forced
`optional = true`, so `continue_rest_with_context`'s own `optional` gate no
longer fires for this query -- the suppression now happens one level up, in
`Expr::Optional`'s own structural catch of the isolated `recurse(...)`'s
`Partial`. Nothing left in the path-context evaluator produces
`optional == true` at all; see #2212.

**The fourth site, `ParentN`'s own `n` argument, fans out since spine 2416's
walk residue.** `parent((1,2))` used to array-collapse `n` to `[1,2]` and
then error on the wrong type (`expected number, got array`); a computed `n`
is evaluated at the position now, on every route -- the walk
(`path_context_step_generic`), the owned identity pipe and `eval_builtin`
-- with one hop per output, in order: `.a.b | parent((1,2)) | path` is
`["a"]` then `[]`, and `.a.b | [parent((0,1)) | path]` is
`[["a","b"],["a"]]`. Neither reference can express the shape (jq has no
`parent`; real yq accepts only a literal `n`, `parent(0+1)` is `bad
expression` in v4.53.3), so this is the tree-structural model's answer, the
same one a computed bracket's component stream gets.

### `range`'s own accumulation cap (#2089): a resource-exhaustion guard, now raising instead of silently truncating

> The cap's raise is uncatchable by `?`/`try`/`catch` since #2132 -- see "Every resource cap is uncatchable" just below.

`MAX_RANGE` (`src/jq/eval.rs`, `100000`, shared by `eval_range_values` and
`eval_range_values_f64`) caps how many values a single `(from, to, step)`
combination will accumulate before raising, mirroring
`REDUCE_FOREACH_MAX_STEPS`'s own resource-exhaustion rationale two sections
above -- real jq's `range` is bounded only by memory, and a single
materializing call (`[range(1e18)]`, `reduce range(1e18) as $x (0;.+$x)`)
could otherwise try to allocate an unbounded `Vec` in one shot.

```console
$ echo null | jq -c '[range(100001)] | length'             # jq 1.7.1
100001
$ echo null | succinctly jq -c '[range(100001)] | length'
jq: error (at <stdin>:1): range: maximum iterations exceeded    # exit 5
```

Before this fix, exceeding the cap silently returned the truncated prefix as
a **success** (`100000`, wrong, exit 0, empty stderr) instead of raising --
the one guard in this file that chose that failure mode, found auditing
#2079's own fix. `range: maximum iterations exceeded` now matches the
`until`/`while`/`reduce`/`foreach`/`repeat` wording convention. Accepted
under ADR-0018 rule 4c, same as `REDUCE_FOREACH_MAX_STEPS`.

**The cap is per-combination and demand-aware, not a hard ceiling on
`range`'s total output**: whether truncation actually raises depends on
whether the *consumer* still wanted more once the capped batch was
delivered. `first`/`limit`/`nth`'s own early stop never sees it —
`first(range(1e18))` still returns `0` instantly, no error, because the
consumer stopped asking within the first few (well under `100000`) values
`each_range`'s sink was ever offered. A demand-driven consumer that *does*
ask past the cap (`[limit(200000; range(1000000))]`, `nth(150000;
range(1000000))`) raises just like the eager materializing shapes above —
real jq answers `200000`/`150000` for both (confirmed live), so these are
genuine, if narrower, instances of the same pre-existing divergence #2089's
own repro table didn't happen to list, not new ones introduced by this fix.

**Two known gaps this fix doesn't close, found reviewing it:**

- `range(...)?`/`try range(...)` still lets the truncated prefix through as
  ordinary output (`[range(100001)?] | length` => `100000`, exit 0) — `?`
  suppresses the raised error, but `drain_result` has already delivered the
  capped batch to the sink before `emit` decides whether to raise, so
  nothing distinguishes it from a genuine result once the error itself is
  swallowed. Not specific to `range` (`[while(true;.+1)?]` shows the same
  pattern for `WHILE_UNTIL_MAX_STEPS`), so tracked as a general evaluator
  question rather than here: [#2132](https://github.com/rust-works/succinctly/issues/2132).
- `eval_range_values`'s integer stepping (`i += step`) has no overflow
  guard — pre-existing, reproduces identically before this fix too, and
  needs its own oracle verification before choosing a direction. Tracked as
  [#2131](https://github.com/rust-works/succinctly/issues/2131). **Fixed**:
  see the next section.

### Every resource cap is uncatchable by `?`/`try`/`catch` (#2132) -- shared by the five cap sections

The five evaluator-imposed caps documented in this file -- `range`'s `MAX_RANGE` (#2089, above),
`while`/`until`'s `WHILE_UNTIL_MAX_STEPS` (#534/#2087), `reduce`/`foreach`'s
`REDUCE_FOREACH_MAX_STEPS` (#695/#2079), `repeat`'s `MAX_ITERATIONS` (#2014), and a recursive
`def` past `MAX_EVAL_FRAMES` (#1371) -- raise as `ErrorKind::ResourceLimit`
(`src/jq/error.rs`), which every catch boundary treats exactly like a decode failure: never
suppressed by `?`, never handed to `catch`, and never a reason for `?//` to try the next
destructuring alternative. Before #2132 they were plain errors, and the catch swallowed them:

```console
$ echo null | jq -c '[range(100001)?] | length'            # jq 1.7.1 -- no cap at all
100001
$ echo null | succinctly jq -c '[range(100001)?] | length'  # before #2132
100000                                                       # exit 0 -- silently truncated
$ echo null | succinctly jq -c 'def f: f; try f catch "caught"'  # before #2132
"caught"
```

That was exactly the silent-wrong-data class #2089 closed for the *un-suppressed* spelling,
re-opened one `?` away. The caps are succinctly's own, with no jq counterpart, so a `?`/`try`
written against jq semantics cannot have meant "accept a truncated result" -- under ADR-0018's
decision order this is rule 4(b) (matching the reference's catch semantics for an error the
reference cannot raise would corrupt data), decided identically in both modes since the caps
are not a reference behaviour in either.

**Ordinary errors are unchanged, and that is jq's own rule, not a residual.** Real jq keeps the
prefix a generator produced and catches only the terminal error -- `[(1,2,error("x"),3)?]` is
`[1,2]`, `[try (1,2,error("x"),3) catch "c"]` is `[1,2,"c"]` -- and succinctly matches it.
The issue's broad reading ("discard what already came out") would have been a divergence.
Nor does the tag look at message text: a user's own
`error("range: maximum iterations exceeded")` is still caught (#1660's lesson).

**The prefix still streams.** A cap is an error that ends the stream, not one that
retroactively discards it: `range(100001)?` prints `0` through `99999` and *then* the error on
stderr at exit 5, as #2089 established for the un-suppressed form. An early-stopping
consumer never reaches the cap at all (`[limit(5; range(100001))?]` is `[0,1,2,3,4]`).

**`?//` is a catch boundary too.** Review of the first cut found the destructuring-alternative
retry (`try_pattern_alternatives`, `each_pattern_alternatives` and its generic twin, and
`reduce`/`foreach`'s `is_retryable_control`) still consulting `is_decode_failure()` alone --
#1620/#1660's own exclusion -- so a cap raised inside a `?//` body read as "try the next
alternative": `[. as $x ?// $y | if $x != null then range(100002) else 1 end] | length`
answered `100001` at exit 0. All of them, and `suppresses` (the ambient-`?` rule every fold
construct shares), now use the one value-position predicate the `?`/`try` dispatch points
always did.

Pinned by `test_resource_limit_caps_are_uncatchable_2132`,
`test_resource_limit_prefix_still_streams_before_the_cap_2132`,
`test_resource_limit_tag_leaves_ordinary_errors_catchable_2132` and
`test_resource_limit_is_not_a_destructuring_retry_2132` (`tests/jq_cli_tests.rs`),
`resource_limit_is_uncatchable_by_tag_not_message_2132` (`src/jq/error.rs`), and the yq
twin `test_resource_limit_caps_are_uncatchable_in_yq_mode_2132`.

### `range`'s `i64` fast path at extreme magnitude (#2131, #2219): overflow-checked in place, extending the same #2089 hang-avoidance divergence

The gap the previous section left open: `eval_range_values` (the exact-`i64`
fast path taken when `from`/`to`/`step` are all integers) computed
`i += step` in plain `i64` arithmetic with no overflow guard. Near
`i64::MIN`/`i64::MAX` this wrapped via two's-complement and streamed garbage
instead of erroring or matching jq:

```console
$ echo null | succinctly jq 'range(-9223372036854775758; -9223372036854775808; -100)' | head -3   # before this fix
-9223372036854775758
9223372036854775758     # wrapped past i64::MIN
9223372036854775658
```

Real jq represents every number as `f64`, so it has no `i64`-overflow concept
at all — only precision loss: two integers close enough together at a large
enough magnitude round to the *same* double, so a `range` between them is a
zero-iteration loop by construction (`from > to` is false from the start).

**First fix (superseded below):** `eval_range_values` only took the `i64`
path when `from`, `to`, *and* `step` were all within `±2^53` (the magnitude
where every `i64` round-trips through `f64` losslessly), routing anything
outside that blanket cutoff to the pre-existing, `MAX_RANGE`-capped
`eval_range_values_f64`. This was correct — it made overflow impossible by
construction — but far more conservative than the actual overflow risk
(`i64::MIN`/`i64::MAX`-adjacent, `~±9.2 * 10^18`, six orders of magnitude
looser than `±2^53`), with a real, disclosed-but-understated side effect: an
ordinary large-but-safe integer range — a nanosecond-epoch timestamp span, a
large database ID range — got silently routed to `f64` purely on magnitude,
where at that scale every value in a short range can round to the *same*
double and the range degenerates to `[]`. For a realistic input, that is
worse than the original overflow bug it replaced (wrong-looking wrapped
output, at least visibly wrong; a silent empty result at exit 0 is not).

**Current fix:** `eval_range_values` now uses `i.checked_add(step)` in place
of the bare `i += step`, bailing out (`None`) the instant an addition would
leave `i64`'s range; `each_range`'s `emit` closure reacts to `None` by
discarding whatever that call already pushed and recomputing the *entire*
range from scratch via `eval_range_values_f64`, exactly as the `±2^53` gate
did for its own out-of-bound cases. The blanket magnitude cutoff and its
`range_i64_fast_path_is_safe`/`I64_F64_EXACT_BOUND` gate are gone; there is
no separate threshold to prove correct, because the check *is* the same
arithmetic that would otherwise overflow, performed at the exact point it
could occur. The safety property is now: **no possible sequence of
`i += step`, starting at `from` and stopping at or before `to`, can ever
overflow `i64`** — narrower than "agrees with `f64`", and (unlike the
`±2^53` proof) not something that needs `from`/`to`/`step` bounded in
advance, since `checked_add` detects the exact condition directly rather
than a sufficient-but-loose approximation of it. Verified two ways: an
analytical argument in `eval_range_values`'s own doc comment (`src/jq/eval.rs`),
and `test_eval_range_values_matches_i128_reference_sweep_2131`, which
differentially checks it against an independent `i128`-arithmetic
reimplementation of the same loop across a magnitude-diverse grid anchored
at `i64::MIN`/`i64::MAX`, `±2^53`, and the nanosecond-epoch scale.

**Round-3 refinement, cap-boundary decoupling.** A review of the
`checked_add`-based fix above found one more gap in *how* it was applied:
the loop checked the cap (`values.len() < MAX_RANGE`) and attempted the
advance in the same unconditional step, so on the iteration whose push
reached the cap it still computed the next candidate the exact same way
every other iteration does — including bailing the *whole call* via `?` if
that candidate overflowed `i64`, discarding every already-correct value it
had pushed for an `f64` recomputation none of them needed — confirmed live:
`nth(250; range(0; 9223372036854775807; 92234642714974))` returned a wrong,
`f64`-derived answer (`23058660678743610`) instead of the exact
`250 * 92234642714974 = 23058660678743500`. The fix still computes that one
lookahead at the cap boundary — there is no way to know whether one more
value exists without it — but interprets its overflow *locally* rather than
bailing: `to` is always representable in `i64`, so a candidate that
overflows `i64` can never be less than `to` (greater than `to` on the
descending arm) either, meaning it's provably not part of the range —
indistinguishable from natural exhaustion. A cap-terminated exit therefore
never *discards* on that overflow, only ever resolves its own `truncated`
verdict to `false` in exactly the cases where that's providably correct
(and still resolves to `true` when a cap-hit range's lookahead doesn't
overflow and genuinely has more values, e.g. `range(0;100005;1)`).
`reference_range_i64` shared the original revision's same unconditional
structure (so the sweep above validated "matches an equally flawed model,"
not the real property); it was corrected the same way, and the sweep's own
`steps` grid gained a dedicated entry at this magnitude
(`92234642714974`) so this class of bug is caught automatically rather
than by inspection alone.

**#2219 refinement: the same proof extends to the loop's own per-iteration
advance, not just the cap-boundary lookahead.** Round 3 above fixed one call
to `i.checked_add(step)` (the cap-boundary lookahead) but left the other
(the ordinary advance that keeps a non-cap-terminated loop going) gated by
`?`, discarding every already-pushed value on overflow there too:
`range(9223372036854775802; 9223372036854775804; 20)` returned `[]` instead
of the exact `[9223372036854775802]` its own first (and only) value already
computed correctly. #2219 recognized this is the identical soundness
argument, not a new one: `to` is always a valid `i64`, so an overflowing
candidate can never satisfy `< to`/`> to` regardless of *which* call site
computed it — cap-terminated or not. Both call sites now end the loop and
keep whatever has already been pushed. A `range` between two valid `i64`
endpoints always pushes at least one value (`from` itself) before any
possible overflow, so **the `None => eval_range_values_f64(...)` fallback
`each_range`'s `emit` closure used to reach for `Int`/`Int`/`Int` operands
is now provably unreachable**. `eval_range_values` therefore drops its
`Option` return type entirely — it's a private function with exactly two
in-file call sites (production and its own test module), so there was no
external caller for an `Option` signature to stay compatible with — see its
own doc comment, and
`test_range_dispatch_keeps_exact_int_prefix_through_overflow_2219`, which
pins this for both the original repro and a fresh near-`i64::MAX` case.
`eval_range_values_f64` remains real, reachable production code; it's just
reached only via `emit`'s other match arm now, for genuinely non-integer
operands.

This changes what the original repro now produces:

```console
$ echo null | succinctly jq 'range(-9223372036854775758; -9223372036854775808; -100)'
-9223372036854775758
```

(`from` is only 50 above `i64::MIN` and `step` subtracts 100 more, so
`checked_add` overflows on the very first application -- but `from` itself
was already pushed before that overflow ends the loop, so it's kept.) Real
jq 1.7.1 still answers empty here, but only because its own unary minus
collapses a literal this large to a `double` before `range` ever sees it
(confirmed live: `jq -nc '-9223372036854775758'` prints
`-9223372036854776000`, already rounded) -- an unrelated, pre-existing
divergence (see "Unary minus in filter text destroys literal preservation"
below, #2357), not something
#2219 changed. The pre-#2219 empty answer matched jq's only by coincidence
of that unrelated quirk; feeding the same magnitudes in as *data* instead of
literals (`echo '[-9223372036854775758,-9223372036854775808,-100]' | jq -c
'[range(.[0];.[1];.[2])]'`) already showed jq answering
`[-9223372036854775758]` even before this fix -- the single value #2219 now
also gives from the literal spelling.

`eval_range_values_i64(i64::MAX - 10, i64::MAX, 1000)` and its mirror at
`i64::MIN` (overflow on the very first add, one value already pushed) and
`eval_range_values_i64(0, i64::MAX, big_step)` (overflow on the *second*
add, two values already pushed) are pinned the same way in
`test_eval_range_values_overflow_detection_2131`, which #2219 retargeted
from asserting `None` to asserting the exact kept prefix at each of these
three magnitude combinations.

But the fix below still recovers the nanosecond-scale case the `±2^53` gate
used to silently break:

```console
$ echo null | succinctly jq -c '[range(1700000000000000000; 1700000000000000010; 1)]'
[1700000000000000000,1700000000000000001,1700000000000000002,1700000000000000003,1700000000000000004,1700000000000000005,1700000000000000006,1700000000000000007,1700000000000000008,1700000000000000009]
```

This range has no overflow risk whatsoever (a 10-step walk, nowhere near
`i64::MAX`), so `checked_add` never trips and the exact `i64` answer comes
back untouched — the fix this section documents.

It also changes the confirmed-hangs-in-real-jq case from the previous
section: once `i` reaches `2^53` with `step == 1`, `i + 1` rounds back to
exactly `i` in `f64` arithmetic (the next representable double after `2^53`
is `2^53 + 2`, not `2^53 + 1`), so real jq's own `while(. < $upto; . + $by)`
never advances and never terminates — confirmed live and killed by hand
while investigating this issue (`timeout 3 jq -c 'range(9007199254740990;
9007199254740994; 1)'` hangs until killed, repeating `9007199254740992`
forever). Under the `±2^53` gate this range (`to` exceeds the cutoff) was
routed to `f64` same as any out-of-bound range, and the pre-existing
`MAX_RANGE` cap (#2089) raised instead of looping forever — the same
ADR-0018 rule 4c shape `repeat(f)`'s hang precedent documents above, reached
through `range`'s dispatch rather than through `repeat`'s own round loop.
Under the current fix this range has no `i64` overflow risk at all (`i64`
has no precision-loss problem at this magnitude the way `f64` does: every
integer here is exactly representable, and `i.checked_add(1)` always
advances by exactly 1), so it now stays on the exact `i64` path and
completes correctly and quickly, well under `MAX_RANGE`, rather than needing
the cap to save it:

```console
$ echo null | succinctly jq -c '[range(9007199254740990; 9007199254740994; 1)]'
[9007199254740990,9007199254740991,9007199254740992,9007199254740993]
```

A beneficial side effect of the narrower gate, not a new problem — `MAX_RANGE`
still raises on any range that genuinely needs more than 100000 values,
overflow or not.

**Widened divergence from jq's own arithmetic, covering the *entire*
`2^53`–`i64::MAX` range, not a narrow band near the top.** The divergence
starts exactly at `2^53` (`9007199254740992`) — not "near `i64::MAX`" as an
earlier revision of this section implied by leading with a single
boundary-adjacent example. jq 1.7.1's actual number model above `2^53` is
not plain `f64`: it preserves each literal as a `decNumber` (`src/jv.c`,
`DEC_INIT_BASE`) and only rounds through a 17-significant-digit decimal
intermediate when a value crosses into arithmetic (`. + $by` inside
`while`'s definition), so its behavior differs from a bare `i64`-to-`f64`
cast, and differs *again* depending on where in that ~3-order-of-magnitude
range (`9007199254740992` to `9223372036854775807`) the values fall. A
sweep from just above `2^53` through `i64::MAX` (all against the pinned jq
1.7.1 oracle, every probe near this magnitude `timeout`-guarded — this has
hung a live session twice in this issue's own history) found the divergence
is close to universal across the whole range, not occasional, in two
distinct shapes:

- **Hang sub-band, `2^53` up to approximately `2^57`
  (`144115188075855872`, `~1.441 * 10^17`):** real jq's own
  `while(. < $upto; . + $by)` never advances and never terminates. Once `.`
  rounds through `decNumber`-to-`double` conversion, `. + 1` rounds back to
  exactly `.` (the increment is too small relative to the representable
  gap at this magnitude to move to a different double), so the loop
  condition never changes and jq spins forever, repeating the same value —
  this is the mechanism the previous revision of this section documented
  for the single literal `range(9007199254740990; 9007199254740994; 1)`,
  but confirmed here to hold across a ~16x (roughly `2^4`) span of
  magnitude, not one specific value: live-confirmed (`timeout`-killed) at
  `9007199254740992`, `9007199254740993`, `10000000000000000`,
  `20000000000000000`, `50000000000000000`, `80000000000000000`, several
  points between `1.2 * 10^17` and `1.43 * 10^17`, and non-round
  17-digit values (`12345678901234567`, `67891234567890123`).
- **Truncate sub-band, approximately `2^57` up through `i64::MAX`:** real jq
  terminates, but after emitting only its starting value — `. + $by` still
  rounds through `decNumber`/`double` conversion, but now the rounded
  increment is large enough to jump `.` past `$upto` in a single step
  instead of rounding back to `.` itself, so the `while` loop exits after
  exactly one iteration rather than spinning. Live-confirmed single-value
  output (`[from]`, not the full walk) at `200000000000000000`,
  `500000000000000000`, `800000000000000000`, `900000000000000000`,
  `1000000000000000000`, `1230000000000000000`, `1700000000000000000`,
  `4600000000000000000`, and this section's own pre-existing example,
  `range(9223372036854775800; 9223372036854775807; 1)`.
- **The boundary between the two sub-bands sits close to `2^57`,** not
  precisely pinned to it: bisecting with a fixed `range(v; v+10; 1)` shape
  found `144110000000000000` still hangs while `144115188075855871`
  (`2^57 - 1`) already truncates to one value — a span of under
  `10^10` around `2^57` itself. This is reported as *approximately* located
  rather than an exact global constant because the true cutover is a
  property of each specific value's bit pattern relative to its own
  magnitude's rounding granularity (which doubles every power of two), not
  a single sharp threshold that holds for every `from`/`to`/`step`
  combination — a different span or step could shift exactly where a given
  magnitude lands between the two shapes.

**The nanosecond-epoch example from the fix above is itself an instance of
this same divergence, not a case of matching jq.** `range(1700000000000000000;
1700000000000000010; 1)` sits at `~1.7 * 10^18`, well inside the truncate
sub-band just described. succinctly's overflow-free `i64` walk gives the
exact 10-value enumeration documented above; real jq 1.7.1 gives a single
value, `[1700000000000000000]`, live-confirmed. Presenting that example
earlier as an unqualified fix would overstate it: succinctly did not
*regain* parity with jq at this magnitude (jq never enumerated all 10
values in the first place, both before and after this fix), it became
*internally exact and consistent* where it previously produced a different
wrong answer (silently empty, from the superseded `±2^53` gate). That is a
real improvement — predictable, exact `i64` arithmetic beats silent `f64`
rounding into an empty result — but it is not jq parity, and every other
large-but-`i64`-safe integer range above `2^53` falls into the same
category: succinctly now returns the mathematically exact enumeration,
which is *usually not* what jq 1.7.1 itself would produce at that
magnitude, whichever of the two sub-bands above it lands in.

Two further shapes, both unaffected by this fix's own logic since they're
both about *jq's* arithmetic or a separate, pre-existing part of
succinctly's own dispatch rather than the exact-`i64`-path divergence just
described:

- **Both sides use `f64`** (any `from`/`to`/`step` combination where at
  least one operand isn't a plain integer, e.g. a float literal — since
  #2219, an `i64` range can no longer reach `eval_range_values_f64` via
  overflow, only via this non-integer dispatch arm): reproducible via
  `range(72057594037927936.0; 72057594037927944.0; 1)` — jq 1.7.1 gives one
  value, `[72057594037927936.0]`, where succinctly's own (unchanged)
  `eval_range_values_f64` gives `[]`. This is `eval_range_values_f64`'s own
  pre-existing fidelity gap, exactly as recorded before this revision,
  independent of the dispatch mechanism above it.
- **succinctly stays on the exact `i64` path where jq's own arithmetic
  would round, stall, or hang** — new to this revision's scope, since the
  `±2^53` gate used to route every such case to `f64` too (matching jq's
  *answer* incidentally, by sharing its precision loss, though not its
  hang in the lower sub-band — the `MAX_RANGE` cap raised there instead,
  the same ADR-0018 rule 4c trade-off `repeat(f)`'s hang precedent
  documents elsewhere in this file). Whenever `eval_range_values` has no
  overflow risk, its answer is the exact integer computation, which is
  *always* what jq would compute too below `2^53`, but is no longer proven
  to bit-match jq's own `f64`/`decNumber` arithmetic at any magnitude above
  it — not just near the true overflow zone the way the `±2^53` gate's
  property held. This is the redesign's own trade-off, stated directly in
  the issue that requested it: the new safety criterion is "no `i64`
  overflow", not "bit-identical to jq's own number model" — and is accepted
  for the same reason the `2^53` hang case is accepted above (ADR-0018 rule
  4c: not a case of succinctly emitting unreadable output, corrupting data,
  or discarding a write, and the alternative — silently degrading
  large-but-safe integers to `f64`, as the `±2^53` gate did, or reproducing
  jq's own hang, as ADR-0018 does not require — is the worse failure mode
  for the realistic inputs this magnitude range covers).

### Unary minus in filter text destroys literal preservation — inherits the `range` divergence's own rule 4(c) grant (#2357)

Real jq preserves a number literal's exact source spelling through to output (`jq -n
'1.0'` → `1.0`, `1e10` → `1E+10`), but a *unary minus written in the filter text* breaks
that: `-1.0` is `negate(1.0)`, a computed `double`, not a preserved literal, so jq's own
answer at extreme magnitude no longer matches the value as written. succinctly keeps the
exact value regardless of a leading `-` in the filter. Confirmed live against jq 1.7.1:

```console
$ jq  -nc -- '-9223372036854775758'      # -9223372036854776000
$ sjq -nc -- '-9223372036854775758'      # -9223372036854775758
$ jq  -nc -- '-9007199254740993'         # -9007199254740992
$ sjq -nc -- '-9007199254740993'         # -9007199254740993
$ jq  -nc -- '-1.10'                     # -1.1  (agrees -- magnitude-specific)
$ sjq -nc -- '-1.10'                     # -1.1
```

Only reachable above `2^53` (where a `double` can no longer hold the literal exactly) — the
same magnitude floor the `range` divergence above starts at. It is specifically the
*filter-text* unary minus: the identical value arriving as **data** keeps its exact spelling
in both tools (unrelated to this divergence, and not the general large-integer class it might
first look like):

```console
$ echo '[-9223372036854775758]' | jq  -c '.[0]'   # -9223372036854775758
$ echo '[-9223372036854775758]' | sjq -c '.[0]'   # -9223372036854775758
```

**Why this is worth stating explicitly rather than leaving as an obvious consequence of
jq's own parser:** it silently changes the answer of anything downstream of a negative
literal at that magnitude, and it produced a real analysis error in this repo before this
was recorded — the `range` divergence above (`range(-9223372036854775758;
-9223372036854775808; -100)`) used to read as evidence that succinctly's overflow handling
agreed with jq's, when the agreement was actually two independent bugs' outputs coinciding:
jq's `[]` came entirely from unary minus collapsing `from` to exactly `i64::MIN` before
`range` ever ran, not from anything `range` itself computed. Feeding the identical
magnitudes in as data (where jq keeps its literals) already showed the two tools
disagreeing on `range` itself, underneath the unary-minus coincidence.

**Accepted rather than matched — not a fresh rule-4 exemption, the same one already
granted above.** This is not an independent divergence needing its own justification: it is
the necessary consequence of the `range` divergence's own accepted trade-off ("succinctly
stays on the exact `i64` path where jq's own arithmetic would round, stall, or hang",
above), which already covers this exact magnitude class under ADR-0018 rule 4(c) —
avoiding **the same named failure mode** rule 4(c) rejected there: "silently degrading
large-but-safe integers to `f64`". Making unary minus specifically force that same
degradation, for the one syntactic shape "a literal token immediately preceded by `-`" and
no other, would reintroduce precisely that failure mode for this operator alone — while
`9223372036854775758` negated via `0 - 9223372036854775758`, or the identical magnitude
arriving as data with its sign already attached, both keep the exact value today. A
special case narrow enough to catch only the unary-minus spelling would need to specifically
detect and degrade a literal it would otherwise preserve exactly, which is the regression
the `range` fix's own rule-4(c) argument already ruled out for this magnitude class, not a
new argument invented for this operator. ("The other operator does the same thing" is not
itself a rule-4 condition — the condition being invoked here is 4(c), inherited from the
`range` entry above, not re-derived from consistency alone.) Pinned by
`test_unary_minus_destroys_literal_preservation_2357` (`tests/jq_cli_tests.rs`).

### `--argjson`/`--jsonargs` still reject a bare trailing decimal point with no exponent (`1.`) — accepted divergence, ADR-0018 rule 4c (#2240)

Real jq's own number reader accepts a bare trailing `.` with nothing after it at all,
treating it the same as if the dot weren't there:

```console
$ jq -n --argjson x '1.' '$x'
1
$ succinctly jq -n --argjson x '1.' '$x'
Error: Invalid JSON for --argjson x
```

#2240 fixed `--argjson`/`--jsonargs` to accept the *other* two number leniencies real jq's
own parser tolerates that strict RFC 8259 doesn't — a leading decimal point (`.5`, #1171)
and a trailing decimal point immediately *before an exponent marker* (`1.e5`, #2220) — but
deliberately not this third, narrower shape. Accepting it would route the literal through
`OwnedValue::from_number_bytes`/`StandardJson::number_literal()`'s shared, pre-existing
large-integer-precision gap: a bare trailing dot with no exponent on an integer too large
for `f64` to represent exactly materializes as the wrong integer regardless of entry point
(confirmed live via plain document input, unrelated to `--argjson`):

```console
$ jq -c '.' <<< '99999999999999999.'
99999999999999999
$ succinctly jq -c '.' <<< '99999999999999999.'
100000000000000000
```

`from_number_bytes`'s own doc comment already documents this trade-off for the *general*
(non-`--argjson`) case as deliberate, reasoning that real jq doesn't preserve this exact
spelling either (`[1.] -> [1]` on both sides) — true only for a small value where the lossy
`f64` round-trip happens to be lossless, not for a large one. Extending `--argjson`'s own
acceptance to this shape would mean trading a clean rejection (the pre-#2240 status quo)
for a value that's silently wrong rather than absent, for the one case where the
underlying gap actually bites. Per ADR-0018 rule 4c: matching real jq's own acceptance here
would risk corrupting data, the worse failure mode for the input shapes this actually
covers — so `--argjson x '1.'` stays a clean, catchable error rather than a silent
magnitude error, a narrower divergence from real jq than #2240's own issue text first
scoped this fix to. Fixing the underlying large-integer-precision gap itself (so `1.` could
be accepted safely at every magnitude) is out of scope for #2240 and not separately filed.

### `foreach`/`reduce`'s INIT-fork re-entry: SOURCE reads real jq's synthetic `null`, not the ambient input — no carve-out; recorded as a still-open policy question (#534, #2163)

`foreach`/`reduce`'s parser accepted a top-level comma in the INIT slot from #534 onward
(`foreach SOURCE as $x (INIT1, INIT2; ...)`), which forks the whole construct once per INIT
output, each fork re-running over the same SOURCE stream. Real jq has an internal quirk here
that this file never documented until now, even though the divergence itself has shipped
since #534 and is already pinned by `test_reduce_comma_slots`/`test_foreach_comma_slots`
(`src/jq/eval.rs`) — an ADR-0018 rule 6 gap this entry closes:

```console
$ echo '[1,2,3]' | jq -c '[reduce .[] as $x (0,1; .+$x)]'
jq: error (at <stdin>:1): Cannot iterate over null (null)
$ echo '[1,2,3]' | succinctly jq -c '[reduce .[] as $x (0,1; .+$x)]'
[6,7]
```

(real jq's exit code is 5, and — because `[...]` fully buffers its generator before printing
anything — stdout is completely empty here, not a partial `[6]`; the pre-existing
`test_reduce_comma_slots`/`test_foreach_comma_slots` code comments in `src/jq/eval.rs`
understated this the same way, corrected alongside this entry).

Real jq's own bytecode compiler re-enters SOURCE's evaluation once per INIT fork, and for
every fork after the first, the ambient input (`.`) SOURCE sees there is a synthetic `null`
— not the real document — an artifact of how jq's VM threads the input register across a
backtrack boundary, not a designed feature. `.[]` on that synthetic `null` raises "Cannot
iterate over null"; a null-tolerant field read instead answers `null` silently
([#2163](https://github.com/rust-works/succinctly/issues/2163) extended the original #534
finding to this shape):

```console
$ echo '{"a":1,"c":2}' | jq -c '[foreach (.a) as $k ((0,.c); $k)]'
[1,null]
$ echo '{"a":1,"c":2}' | succinctly jq -c '[foreach (.a) as $k ((0,.c); $k)]'
[1,1]
```

Two of succinctly's three jq evaluators — `eval.rs`'s value evaluator and
`eval_generic.rs`'s generic/CLI evaluator — share one core per construct
(`eval_reduce_with_values`, `foreach_forks`), and neither ever substitutes a synthetic `null`
for the ambient document: `reduce` computes SOURCE's values once, upfront, and reuses them
across every INIT fork, while `foreach` (since #2180 WP3's review) re-drives SOURCE per fork
— against the *real* ambient input every time, which is exactly the half jq does not do. That front-end-level agreement (not independent
double-implementation of the fold itself, since both front ends call the same shared
functions) is what `test_parity_foreach_reduce_init_fork_source_reads_ambient_input_2163`
(`tests/jq_evaluator_parity_tests.rs`) pins.

The **third** evaluator — the path-context evaluator's `resolve_reduce`/`resolve_foreach`
(`src/jq/eval.rs`) — is not part of that shared core and is not consistent with it here: its
own SOURCE resolution (`drive_fold_source`, added by #2031 for path-trackability, not for
this divergence) already re-runs once per INIT fork, inside the per-branch loop, using the
real document every time rather than a cached value — architecturally the closest thing in
this codebase to what a jq-matching fix would need, but it does not inject a synthetic
`null`, and in path/assignment position it does not reach the value evaluator's `[1,1]`
answer at all:

```console
$ echo '{"a":1,"c":2}' | succinctly jq -c 'path(reduce (.a) as $k ((0,.c); $k))'
jq: error (at <stdin>:1): Invalid path expression with result 1
```

This pre-existing three-way inconsistency (a #2031 restriction on navigating INIT forks,
unrelated to #2163) is filed separately as
[#2388](https://github.com/rust-works/succinctly/issues/2388) rather than folded into this
entry, since it is a succinctly-internal-consistency gap, not a jq-fidelity question.

This divergence does not fit any of ADR-0018 rule 4's four named conditions letter-for-letter
(the output is readable, nothing is corrupted, and the process doesn't die either way), so
per rule 4 it is recorded here as a still-open policy question — matching this file's own "A
structurally malformed value doesn't abort the rest of a multi-value stream" entry, which is
itself still open rather than a settled precedent to build on. Matching jq's quirk exactly
would mean re-deriving jq's own undocumented VM register-threading rule (which expression
shapes get a fresh backtrack point, and what `.` reads as across one) and reshaping the
shared `eval_reduce_with_values`/`foreach_forks` core so it evaluates SOURCE per-fork against
a synthetic `null` document — for `foreach` that is now only the synthetic-`null` half, since
the per-fork re-evaluation itself already landed with #2180 WP3's review; for `reduce` it is
still both halves. A prior attempt at a narrower version of this fix (nulling only the bound
pattern variable, not re-deriving SOURCE itself) regressed a passing test
(`test_eval_reduce_init_partial_prefix_still_forks_when_trailing_error_suppressed_1934`) —
but that test's own SOURCE is a constant (`5`) that never reads `.` at all, so the regression
shows only that *that particular* patch shape was wrong, not that a correctly-shaped
per-fork-null fix is itself infeasible; no such fix has actually been attempted. Given `.[]`'s
identical case has already shipped since #534 with no user-visible complaint, and
succinctly's own value-evaluator behavior is arguably more useful than jq's here (one
consistent value instead of a silent `null`/hard-error split depending on fork position) —
this is recorded now, with regression coverage in place, as an open question for whoever
next has reason to attempt the real per-fork re-evaluation, rather than as either a queued
fix or a closed decision.

### A caller-supplied `OwnedValue` holding a NaN or an over-cap numeric literal can still be re-spelled by a stage the owned identity route hands to the owned evaluator (spine 2416's exit)

`jq::eval_owned_with_file_index` is the one public entry that hands the
path-context machinery an `OwnedValue` the caller built rather than one read
from a document, so it is the only way a bare `Float`, a NaN, or a
`NumberLiteral` longer than `REINDEX_LITERAL_LEN_CAP` (256 chars) can reach
`eval::eval_path_context_pipe_owned`. That door refuses the reindex bridge for
exactly those values (`reindex_bridge_is_identity`) and runs the pipe through
`eval_generic::eval_path_context_pipe_detached` instead, which never
serializes -- so `.[0] | parent`, `[.[0] | parent]` and `.[0] | parent | .[1]`
all hand the literal back exactly as the caller spelled it
(`test_owned_door_keeps_a_value_the_reindex_would_respell_2419`,
`src/jq/eval.rs`).

A stage that route hands to the *ordinary owned evaluator* -- `.[] |
select(key == 1)`, whose `select` runs through `eval_owned_input_reindexed`
-- still takes that evaluator's own `to_json_for_reindex` round trip, so a
300-digit literal comes back as `1E+299` there. Until spine 2416's exit the
eager path-context evaluator answered these shapes without any round trip;
with it deleted, this is the residue. It is not reachable from either CLI: a
document-read number materializes as a short `NumberLiteral`, and yq mode's
`.nan` renders as `null`/`.nan` on both sides of the trip. Real jq has no
`key`/`parent` at all, so there is no oracle for the shape; what changed is a
spelling succinctly used to preserve internally. Fixing it means giving the
owned evaluator a non-reindexing path, not this door.

### `path`/`key`/`parent`/`getpath` validate only what they touch — no carve-out; recorded on its merits (#2168)

These four read a position rather than a value, and as of
[#2168](https://github.com/rust-works/succinctly/issues/2168) they inspect only the nodes
they navigate through. A corrupted value elsewhere in the document — an undecodable escape,
a trailing comma, a colliding undecodable key — no longer makes them raise:

```console
$ printf '%s' '{"a":"\ud800","d":5}' | succinctly jq -c 'path(.d)'
["d"]
$ printf '%s' '{"a":"\ud800","d":5}' | succinctly jq -c 'getpath(["d"])'
5
$ printf '%s' '{"c":{"a":1,},"t":5}' | succinctly jq -c '.t | key'
"t"
```

**This loses fidelity that succinctly previously had, and the decision order does not
license it.** An earlier draft of this entry claimed the reference "separates none of these
rows", so that the order fell through to its performance step. That was wrong, and the
correction is the important part of this record. Real jq 1.7.1 rejects every document above
while parsing it, before any filter runs, and real yq v4.53.3 does the same on the YAML
spelling — so the reference does have a behaviour here, and it is *reject*. Measured against
the pinned binaries:

| filter on `{"a":"\ud800","d":5}` | jq 1.7.1 | succinctly before #2168 | succinctly after |
|---|---|---|---|
| `.d` | error | `5` — already diverged | `5` |
| `path(.d)` | error | error — **matched jq** | `["d"]` — **diverges** |
| `getpath(["d"])` | error | error — **matched jq** | `5` — **diverges** |

Step 2 of ADR-0018's decision order therefore *does* separate the two options, and it favours
the behaviour being given up: keeping the gate matched jq's observable outcome on those rows,
and dropping it does not. The performance step is never reached. No rule-4 condition applies
either — the output is readable, nothing is corrupted or discarded, and neither choice takes
the process down. **This is a deliberate divergence taken against the order's own answer**,
on the same footing as this section's other two entries recorded on their merits rather than
under a carve-out. This class of divergence, and #2103's, are now sanctioned by
[ADR-0018](../../adrs/adr-0018.md)'s #2103 amendment — the reference rejects the whole
document at parse time, and a semi-index that deliberately does not validate up front takes
the uniform divergence over a per-spelling one — rather than resting on prose precedent alone.

`key` and `parent` are the exception within the exception: neither is a jq builtin at all
(`jq: error: key/0 is not defined`, exit 3), so for those two there is no jq oracle and no
fidelity to lose. Only `path` and `getpath` are real divergences from jq. All four diverge
from yq, which has `key`/`parent` and rejects these documents too.

**Why it was taken anyway.** Three reasons, none of which is "the order allowed it":

1. *The agreement being given up was an accident of an implementation detail.* Until
   [#2151](https://github.com/rust-works/succinctly/issues/2151) these builtins materialized
   the whole document to answer, and that materialization doubled as the #1755/#1953 validity
   check. When the tree went, an equivalent whole-document walk was kept in its place
   specifically so a performance change would not move semantics
   ([#2061](https://github.com/rust-works/succinctly/issues/2061),
   [#2075](https://github.com/rust-works/succinctly/issues/2075) and #2151 each say so, and
   each declined the semantics question as the project's to answer). No one ever decided that
   `path()` should validate: it re-derived jq's rejection as a side effect, for a different
   reason than jq has, and only for the spellings that happened to materialize.
2. *It made one read's answer depend on how it was spelled.* `.d` answered `5` and `path(.d)`
   refused, on the same document, in the same run. Nothing can depend on that coherently — a
   caller wanting jq's rejection would have to route through `path` and avoid `.d` — and it is
   the shape [#1629](https://github.com/rust-works/succinctly/issues/1629)/[#1642](https://github.com/rust-works/succinctly/issues/1642)
   exist to remove.
3. *It cost 67-75% of such a query's runtime*, making `.[0] | key` 3.0-4.1x the price of the
   `.[0]` it wraps. That is what made the question worth asking, but it is a reason for
   acting, not a licence under the order.

**The context that makes the loss survivable, and its limit.** succinctly already diverges on
this whole class of document, through `.d` itself, and has since long before #2168: the
semi-index accepts input jq's parser rejects, deliberately and at a measured price — always-on
strict validation costs two thirds of the runtime of a cheap navigation query
([`docs/plan/decode-failure-routing.md`](../../plan/decode-failure-routing.md), "Why not
upfront validation for everyone"), which is the trade `CLAUDE.md`'s own "minimal validation"
note describes. So the real choice was never "match jq or not" but "diverge uniformly, or
diverge in a way that depends on the spelling". That is context, **not** a licence: ADR-0018's
scope note explicitly warns that an evaluator behaviour may not be reframed as a spec-level
input question to escape rule 4, and this entry does not claim otherwise. A user who wants
jq's rejection has `--validate` and `succinctly json validate`, which run at roughly 1 GB/s —
an order of magnitude cheaper than the walk this removes.

**What still validates.** The rule is about reading, not about the builtin's name:

| shape | behaviour | why |
|---|---|---|
| `path(.d)`, `.d \| key`, `.d \| parent`, `getpath(["d"])` | answer | never read `.a` |
| `path(.a)`, `getpath(["a"])` | answer, echoing raw bytes like `.a` | naming a position is not reading it |
| `getpath(["a"]) \| length`, `path(.a[])`, `.a[] \| key` | raise | the value is read |
| `path(if . then .d else null end)`, `path(.d \| select(true))`, `path(..)` | raise | not cursor-navigable; the fallback materializes, and validates all of it |
| `select(f)`, `sort_by(f)`, `unique_by(f)`, `min_by(f)`, `max_by(f)` | raise | the subtree walked *is* the value tested or emitted |
| `to_entries`, `map_values(f)`, `. as $x`, `.a \|= 1`, `[.[]]` | raise | materialize |

The two `path(...)` spellings disagreeing is the residue of this change, not an oversight:
the materializing fallback validates everything it materializes, which is the same rule
applied to a route that reads the whole document.

**What else stopped firing.** The #1194/#1677/#2211/#2243 structural checks and #1642's
colliding-display-key raise rode on the same walk, so they no longer fire for these builtins
on a subtree the query never reads. `[path(.[])]` now names exactly the members
`keys_unsorted` lists, element for element, on a document with two undecodable keys of the
same display spelling — where before, one answered and the other raised. The materializing
routes (`-S`, `-s`) still raise on that document, because rendering both keys into one map
is what the collision *is*; `.,.` no longer does, since #2103 it forwards the cursor to the
printer without building a map (see its entry above).

**One row moved away from jq, with no ADR-0018 carve-out.** A `NumberLiteral` longer than
`REINDEX_LITERAL_LEN_CAP` disqualified a document from `getpath`'s native arm, sending the
call through the reindex round trip, which re-spells it. jq prints `1E-301` for a
303-character literal and so did `getpath(["big"])`; `.big` printed all 303 characters,
because preserving a document number's written form is deliberate
(`DocumentValue::number_literal`,
[#387](https://github.com/rust-works/succinctly/issues/387)/[#966](https://github.com/rust-works/succinctly/issues/966)).
Both spellings now print the source form. No rule-4 condition applies here, so ADR-0018's
decision order does not license preferring this on its own terms — absent an exemption, step
2 says the reference's `1E-301` should have stood. This is recorded as an accepted, if
imperfect, side effect of unifying `getpath`'s number handling with `.big`'s rather than as a
decision the order above actually reaches; the pre-existing divergence it joins
is the entry above on the owned route's re-spelling.

Pinned by `test_lazy_validation_boundary_2168` (the table above, as one test),
`test_path_answers_past_an_undecodable_sibling_2168`,
`test_path_answers_past_a_colliding_key_2168`,
`test_path_answers_past_a_structural_fault_it_never_reads_2168`,
`test_getpath_touches_only_the_nodes_it_navigates_2168`,
`test_getpath_cursor_walk_matches_jq_2168`,
`test_getpath_keeps_a_document_numbers_spelling_2168` (`tests/jq_cli_tests.rs`), and
`test_path_context_cursor_walk_skips_an_undecodable_sibling_2168` plus its yq-mode sibling
(`tests/yq_cli_tests.rs`).

**Settled since.** [#2103](https://github.com/rust-works/succinctly/issues/2103) applied
this rule to the *ambient* value: the eager M2 route's `to_owned_with_cursor` validated a
whole document that a streaming arm never read, and the M2 path was gated to keep the two
from disagreeing. The gate is gone; the entry under "Real-time stdout/stderr interleaving"
records the 19 rows that moved away from jq and the spelling rule it left in place.

### `break $x` shadowed by a `def error:` observes arbitrary ambient pipe state, not jq's own context-independent `{"__jq":N}` sentinel (#2687)

[#2687](https://github.com/rust-works/succinctly/issues/2687): real jq does not treat `break`
as a primitive — its parser desugars `break $x` into a named call to `error/0`
(`gen_call("error", gen_noop())` in `parser.y`), applied to a synthetic sentinel value
(`{"__jq":N}`, `N` the label's compile-time index) as `.`. Name resolution then runs as it
would for any other call, so a `def error:` (arity 0) in scope at the break's own position
shadows it: the label stops catching it, the def's own value becomes an ordinary output, and
evaluation continues past the break instead of unwinding.

succinctly reproduces this (`src/jq/parser.rs`'s `parse_break_expr` wraps `break $x` as a
shadowable `error` call the same way `error`, `limit`, `until`, ... already are, per
`Parser::shadowable_defs`/`wrap_shadowable_call`, #2036), but does **not** carry jq's
`{"__jq":N}` sentinel as the wrapped call's input — the shadowing def instead sees the ambient
`.` the break itself was reached with. jq's sentinel is **fixed and context-independent**
(tied only to the label's own compile-time index, never to any pipeline value), where
succinctly's substitute is **unbounded**: whatever `.` happens to be at the break's textual
position, however unrelated to the label:

```console
$ jq            -nc 'def error: .; label $out | break $out'
{"__jq":0}
$ succinctly jq -nc 'def error: .; label $out | break $out'
null

$ jq            -nc 'def error: {seen: .}; 5 as $x | label $out | ($x | break $out)'
{"seen":{"__jq":0}}
$ succinctly jq -nc 'def error: {seen: .}; 5 as $x | label $out | ($x | break $out)'
{"seen":5}
```

The second row is not "sentinel vs. null" — it is jq's own fixed marker vs. an arbitrary,
unrelated upstream value (`5`, bound three pipe stages earlier) leaking through a construct
whose entire premise is "produce a value with no real connection to the input in scope."
`test_break_shadowed_by_def_error_sees_ambient_input_not_jqs_sentinel_2687`
(`tests/jq_cli_tests.rs`) pins this specific shape so a future change to the mechanism can't
silently make it worse than documented here.

**Why not carried.** The sentinel is observable only through a `def error:` whose body reads
`.` — `def error: "S";` (the shape #2687 was actually filed over, and every case in its own
repro table) is unaffected, since a literal body never looks at its input. Carrying it would
mean the break can no longer fall back to a bare `Expr::Break` node when nothing shadows it —
the overwhelmingly common case, and the one this fix's whole design (`Parser::shadowable_defs`'s
text-only fast-reject) keeps at zero cost and byte-identical AST. It would instead need
`Expr::Pipe(vec![Expr::Literal(<sentinel>), <the wrapped call>])`, widening the shape every
break-aware evaluator arm has to recognize (`needs_path_context`, `owned_identity_step`,
`path_context_step_generic`, `step_can_yield_absent`, `path_context_stage_preserves_node`, …)
for a construction with no purpose outside probing this one divergence. Per ADR-0018's decision
order: the reference behavior here is readable and nothing is corrupted, discarded, or takes the
process down (no rule-4 condition), so the order reaches step 2 (closer match) — but the cost
this row would add to every break site, shadowed or not, makes the narrower fix (#2687 itself,
closing the *shadowing* gap) the one taken, with this payload detail recorded rather than chased.

A second, related gap is **not** covered by #2687 or this entry: `label`'s own error re-raise
(when a caught escape isn't a matching `Control::Break`) is *also* `gen_call("error",
gen_noop())` in jq, resolved at the **label's** lexical position — so a shadowing `def error:`
also intercepts an uncaught `error(...)`/division-by-zero/etc. escaping through a `label` block,
which succinctly does not reproduce at all:

```console
$ jq            -nc 'def error: "S"; label $out | (1, error("x"), 2)'
1
"S"
$ succinctly jq -nc 'def error: "S"; label $out | (1, error("x"), 2)'
1
jq: error (at <unknown>): x
```

Filed as [#2840](https://github.com/rust-works/succinctly/issues/2840) — a materially larger
change (`eval_label`/`each_label`/`each_label_generic` and both owned-identity/path-context
`Label` arms would each need their non-matching-escape fallthrough routed through a resolvable
call), out of #2687's stated scope.

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
