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

Measured against jq-1.7.1 over the 240 probes in
[`tests/data/jq-error-probes.tsv`](../../../tests/data/jq-error-probes.tsv), through
**both** evaluators — the full one (`src/jq/eval.rs`) and the generic one
(`src/jq/eval_generic.rs`, which the CLI uses):

| Dimension                                    | Result              | Meaning                                                |
|----------------------------------------------|---------------------|--------------------------------------------------------|
| **Message text** (both evaluators, verbatim) | **238/240 = 99.2%** | Byte-identical to jq                                   |
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
counterpart for — succinctly extensions (`at_offset`, `@dsv`, `omit`, module
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
classes; the jq-mode `parse_complete_json` path validates the input but does not
translate validation errors into jq's position-specific diagnostics.
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

## The math builtins print the platform libm's last bit, as jq does (#3045)

jq does not compute `sin`, `exp`, `pow`, `sqrt`, ... itself: each is a one-line call into
the C library it was linked against. Those libraries are not correctly rounded and do not
agree with each other. Over 400 inputs (`range(1;401) | . * 0.137 - 20`), the pinned
`/usr/bin/jq` 1.7.1-apple and the `jq-linux-amd64` 1.7.1 release binary print a different
last digit of `tan` on 161, of `cosh` on 69, of `acos` on 60; `exp`, `log` and `pow` agree
on all but 0-1. **The reference's floating-point digits are therefore a property of the
platform**, and [ADR-0018](../../adrs/adr-0018.md)'s pinned reference is "jq 1.7.1 on the
platform succinctly runs on" (its #3045 amendment).

Before #3045 succinctly used the pure-Rust `libm` crate (a musl port) everywhere, and so
matched *neither*: 163/400 on `tan`, 92 on `cosh`, 43 on `exp` against Apple's; 19, 41 and
42 against glibc's. Its `sqrt` was a Newton iteration, off on 161/400 against every libm.
Since #3045, `jq::math` (`src/jq/math.rs`) binds the platform's C symbols directly under
`std` — bit-exact on every sampled input of every builtin against the platform's own jq
(`scripts/jq-libm-oracle-sweep.sh`, 0 mismatches on Apple silicon and on x86_64 glibc 2.35;
the pre-fix build reports 1092). The Rust `f64` inherent methods were rejected for the same
reason the crate was: `f64::atanh` is a formula, several ulps from the C `atanh` on both
platforms.

What remains, recorded here:

- **`no_std` builds keep the `libm` crate** — there is no platform libm to link — and so
  differ from every jq build in the last bit at the rates above. No reference exists for
  such a build; this is the closest available value, not a divergence anyone can measure.
- **"The platform" is the C library the succinctly binary itself links**, not the OS's
  jq. A musl-static succinctly on Linux carries musl's `tan`, one ulp from the glibc-built
  `jq` next to it on 19/400 inputs; a glibc-dynamic build matches that jq exactly. This is
  the same property real jq has (a musl-built jq prints musl's digits) and is not
  something succinctly can paper over. It also means a musl build fails the
  `math_platform_libm_bits` golden and 5 of 11 rows of `math.rs`'s unit test by
  construction — both pin inputs on which the crate (musl's algorithms) is the odd one out —
  so the unit test is gated to glibc/macOS and there is no musl CI leg.
- **The libm's *version* is part of "the platform" too, and the jq-drift reference is
  static.** `jq-linux-amd64` 1.7.1 embeds glibc 2.35. glibc 2.39 — Ubuntu 24.04, i.e.
  every Linux CI leg — rewrote `exp10`: it differs from 2.35's on 137/400 inputs, and now
  agrees with `pow(10, x)` on 400/400 and with Apple's `__exp10` on 399/400. Every other
  builtin is bit-identical between 2.35 and 2.39. A dynamically linked succinctly on a 2.39
  host therefore matches the *system* jq 1.7.1 (which links the same 2.39) on `exp10`, but
  not the static release binary; this is the one function where "the same platform"
  and "the jq-drift reference" name different libms. The golden's four `exp10` inputs were
  checked to agree across Apple, glibc 2.35 and glibc 2.39; any future `exp10` golden row
  must be too, or jq-drift and the test legs will disagree on it roughly one time in three.
- **`exp10`** is spelled the way jq spells it: `__exp10` on Apple platforms (jq's own
  `builtin.c` renames it), `exp10` on glibc and musl. On a platform with neither —
  Android's bionic, the BSDs, MSVC — jq's build defines `exp10/0` as a runtime error ("not
  found at build time"); succinctly answers `pow(10, x)` there instead, which is not
  bit-exact against anything but beats refusing. Note that `pow(10, x)` and `exp10(x)` are
  *different* results on glibc before 2.39 (137/400), so `pow(10; .)` is not a spelling of
  `exp10` against the release jq either.
- **Hermetic goldens can only pin the last bit where every libm agrees.** The
  `math_platform_libm_bits` case was built from a per-function sweep, choosing inputs on
  which Apple's libm and glibc agree and the `libm` crate does not, so the one
  `expected.out` verifies against both jq builds *and* has regression power. The
  `ubuntu-24.04-arm` leg (glibc aarch64) was not measured before that case was captured —
  glibc is not built with `-ffp-contract=off`, so a fused multiply-add could in principle
  move a last bit there; the leg running green is the measurement. The older
  `math_sin_samples`/`math_cos_samples`/`math_atan_pi` goldens floor to six decimals and
  stay as they are: they are not wrong, they just cannot see this.
- **`cbrt`, `tgamma`, `lgamma`, `erf`, `j0`/`y0` and the rest of #3042** take the same
  route and are bit-exact against the platform's jq on every sampled input of every
  function (the sweep's 32 further rows, 0/400 each on Apple silicon and x86_64 glibc).
  One trap: on `linux-gnu`, `compiler_builtins` exports its own weak `cbrt`, `fmax` and
  `fmin` (alongside the IEEE-exact `sqrt`/`floor`/`fma` family, which are
  indistinguishable) and the linker binds an `extern "C"` declaration to those *before*
  glibc's — `cbrt(27)` came out `3` where glibc's prints `3.0000000000000004` (192/400
  off the Linux jq), and `fmax` returned the other zero on a `-0`/`0` tie. `jq::math`
  reaches glibc's through `dlsym(RTLD_NEXT, ..)` for those three
  (`glibc_next_fns!`); a musl build keeps `compiler_builtins`' copies, which *are* musl's.
- **Where real jq's own answer is a property of the platform, succinctly prints that
  platform's**, keyed on what actually decides it, all captured live from `/usr/bin/jq`
  1.7.1-apple and `jq-linux-amd64` 1.7.1:
  - `gamma` is `tgamma` on Apple (jq's `builtin.c` `#define`s it, since Apple's own
    `gamma` is deprecated) and `lgamma` on glibc: `5 | gamma` is `24` there, `3.178...`
    here.
  - `scalb(x; y)` takes a *double* exponent, and glibc returns NaN for a non-integral one
    where Apple's truncates: `scalb(3; 2.9)` is `12` on macOS, `null` on Linux.
  - `ldexp`/`scalbln`/`jn`/`yn` convert their exponent or order with a C `(int)` cast,
    which is undefined for NaN, infinities and out-of-range values, and the
    *architectures* differ: x86-64 yields `INT_MIN` for all of them (`ldexp(3; infinite)`
    is `0`, `jn(1e10; 0.5)` is `null`), AArch64 saturates and maps NaN to 0
    (`1.7976931348623157e+308` and `-0`). `math::c_int`/`c_long` reproduce each, keyed
    on `target_arch`; a Linux AArch64 jq would print the Apple column.
  - `lgamma_r(0)` reports the sign as `0` on Apple and `1` on glibc.
  - **The sign of zero on an `fmax`/`fmin` tie is not even stable within a platform.**
    C leaves it unspecified; glibc 2.35's `fmax(-0.0; 0)` is `0` and 2.39's is `-0`,
    and Ubuntu 24.04's own `jq` 1.7.1 prints `-0` where a dynamically linked succinctly
    on the same host prints `0` — the compiler inlined jq's call as a `maxsd`, which
    never consults `libm.so` at all. Nothing pins these; the golden avoids them and the
    sweep's inputs never tie.
  - `pow10` is a runtime error, `Error: pow10/0 not found at build time`, in every jq
    1.7.1 build (glibc dropped the symbol in 2.27; Apple never had it), and here too.
- **The hermetic golden `math_libm_family`** was built the same way as
  `math_platform_libm_bits` above: every row was run through the Apple jq, the static
  glibc-2.35 jq *and* Ubuntu 24.04's glibc-2.39 jq, compared as text (a JSON-level
  compare is blind to `-0`), and only the rows all three agree on stayed; the
  platform-dependent digits (`cbrt(27)`, `lgamma(5)`, `erf(1)`, ...) are pinned per
  platform by `math.rs`'s gated unit test instead. glibc on **AArch64** is a fourth libm
  again — the same C sources compiled with fused multiply-adds — and PR #3067's first CI
  run on `ubuntu-24.04-arm` found it one ulp from x86-64 glibc on `cbrt(3)` and `y0(10)`
  while agreeing on every other row; those two inputs left the golden, and the `jn`/`yn`
  rows (a multiply-add recurrence over `j0`/`j1`) were reduced to their `n = 0, ±1`
  cases plus the order rules as equalities, since no AArch64 Linux host was available to
  measure them on.

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
   everything outside it drops the register. Four shapes jq compiles as subexps are in the
   allowlist whatever they contain ([#3186](https://github.com/rust-works/succinctly/issues/3186)):
   an object construction, a string interpolation, both operands of an arithmetic or
   comparison operator, and an `if` condition. Nothing within `SUBEXP_BEGIN`/`SUBEXP_END`
   moves jq's register or raises a path error, so `path(. as $x \| {k:.a} \| $x)` is `[]`
   in both tools. jq has more subexps than these four (a C-implemented builtin's arguments,
   so `path(. as $x \| has("a") \| $x)` is `[]` in jq), and a `reduce`/`foreach` stage or a
   `//` also leaves jq's register where it was on inputs this predicate can't tell apart
   statically. Those still drop the register here. Before #3186 these refused, and under `try` the refusal was caught as if it
   were jq's own: `del(. as $v \| {k: .a} \| try ($v \| .[]?))` echoed the document where jq
   deletes every key. An array is different: jq collects it without a subexp, so its contents
   are path-checked (`path(. as $x \| {k:.a} \| [.k] \| $x)` raises on the `.k`), and then
   backtracks the register to where the collect began. Since
   [#3263](https://github.com/rust-works/succinctly/issues/3263) an array carries the register
   too when the resolver both resolves it live and checks everything jq checks inside it:
   navigation, `..`, `select`, an `if`'s branches, both of jq's `?`s and `try`/`catch`,
   `recurse(f)`/`recurse(f; cond)` (#2764), and pipes, commas and subexp shapes of those. So
   `path(. as $x \| [.a] \| $x)` and `path(. as $x \| [try .a] \| $x)` are `[]` in both
   tools. Any other array carries no register. Where it holds navigation this resolver can't
   see (a builtin jq defines in jq, such as `with_entries` or `walk`, or an update
   assignment's `_modify`, as in `[.k \|= 1]`), jq raises too and only the wording differs
   (`… with result …` here). Three shapes jq answers still refuse here. The first is an array
   holding anything outside that list that jq accepts: a shape the resolver still evaluates
   by value (`getpath`:
   `path(. as $x \| [.a \| getpath(["b"])] \| $x)`), or one it resolves but does not count as
   checked (`//`, `first(f)`: `path(. as $x \| [first(.a)] \| $x)`). The second is an array
   nested in another stage's expression (`if true then [.a] else 1 end`, `([.a], [.k])`). The
   third is a `def` whose body is a constant (`path(. as $x \| (def f: 5; f) \| $x)` —
   resolving a call to its body is not something a syntactic predicate can do from a name).
   Every such refusal is
   refuse-only, and since [#3267](https://github.com/rust-works/succinctly/issues/3267) it stays
   refuse-only under `try`/`?` too. It used to be caught as if it were jq's own path error, so
   the write was silently lost at exit 0 where jq writes:
   `del(. as $x \| has("a") \| try ($x \| .a))` returned the document unchanged, and jq
   returns `{"k":1}`. A navigation refusal is now uncatchable when both hold:

   - a live register did not come out of a stage upstream that did not navigate (the stage is
     one this resolver can't see inside, or its route hands back no register, like a `reduce`
     stage);
   - the value being refused is one jq could still have held as the register: a frozen
     `$var` snapshot or a `null`, equal to the lost register's last known value or to one
     inside it. jq's register only moves down from where it was lost, so a `$x` frozen from
     the root can't be a register lost at `.a`, and a `null` can't be one if the lost value
     holds none;
   - the refused step would have navigated had the value been the register. A step jq fails
     with a type error wherever its register is (`$x \| .[0]` on an object, iterating
     `null`, an array pattern over an object) is refused exactly, and stays catchable.

   Uncatchable here means by the `try` beside the refusal, by any `try` further out, and by a
   value-position `?` around the whole `del`/assignment, so the refusal is a loud exit 5
   (ADR-0018 rule 4). The test is made on the value refused, where it is raised, so a refusal
   jq raises too stays catchable: a computed string, number, boolean or container can never be
   the register (`$x \| try (5 \| .a)` is caught in both), and after `.a` the register
   provably moved.

   The price is the case this resolver can't tell apart: a value at or inside the lost register
   that jq's register did *not* land on. There jq's refusal is exact and caught, and this refuses
   loudly, because the stage in between is opaque and might have moved the register there:
   - `del(.a as $y \| has("z") \| try ($y \| .b))` returns the document unchanged in jq, since
     `has` left the register at the root, but `first(.a)` would have moved it onto `$y`;
   - after a `reduce`/`foreach` stage, whose route hands back no register value, where the
     register was lost isn't known at all, so every `$var` or `null` refusal after one is loud:
     `path((.a \| ..) as $v0 \| reduce (1) as $i (.; $v0) \| ($v0 \| .b?)?)` is empty in jq
     and refuses here.

   A `def` call used to be a third, opaque-by-construction case here too (`del(. as $x \|
   (def f: .a; f) \| try ($x \| .k))` refused loudly where jq's own refusal is exact and
   caught) — [#3297](https://github.com/rust-works/succinctly/issues/3297) gave the path
   resolver a native `Expr::FuncDef` arm (it previously had none at all, falling to the eager
   value fallback for *any* `def` reached inside `path()`), so a `def`'s body is no longer
   opaque to this tracking: it is bound and resolved in path mode like any other reachable
   node, and whether it moved the register is now known exactly, the same as `if`/`select`.
   `del(. as $x \| (def f: .a; f) \| try ($x \| .k))` now matches jq exactly (`{"a":{"b":1},"k":1}`,
   exit 0). A `def` whose body is a *constant* (`def f: 5; f`) is unrelated to this: it is the
   "third shape" the array-refusal predicate above still can't resolve without evaluating the
   call, and stays refuse-only.

   The wording can differ too, both tools refusing: jq ends
   `del(. as $x \| has("a") \| reduce (1) as $i ($x; try ($x \| .zz)))` with its try-caught
   fold's `Invalid path expression with result null`, while this raises the refusal itself
   (`… near attempt to access element "zz" of …`). Likewise a guess raised in an earlier
   branch pre-empts the error jq reports from a later one:
   `del(. as $x \| has("a") \| (try ($x \| .a)), .k)` fails in jq on the `.k` (applied to
   `true`), and here on the guessed `$x \| .a`.

   One residual keeps the silent drop. A *terminal* refusal (the pipe's last value is a `$var`,
   with no navigation after it) is still decided where the per-branch knowledge is gone, so a
   value-position `?` catches it: `[path(. as $x \| has("a") \| $x)?]` is `[]` here and
   `[[]]` in jq, as it was before #3267. (A third shape used to sit here too — an `as` whose bind source navigates —
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

   **[#2860](https://github.com/rust-works/succinctly/issues/2860), also closed**: the
   `identical()` check itself (not `advance`'s carry-forward above) had its own, separate
   instance of the same class. `FoldRegister::resolve`'s epilogue calls `self.relocate(..)`
   unconditionally on every branch `resolve_seq`/`resolve_node` returns for UPDATE/EXTRACT,
   and `relocate`'s `identical()` fallback re-derives trackability from `resolve()`'s own
   *entry-time* register (`self.value`/`self.frame`) by nothing more than a value-equality
   check — a tautology whenever the branch's final value is the loop variable itself, which it
   always is when `$var` is referenced anywhere in the pipe. `path(foreach .a as $v0 (.; $v0;
   (.zzz \| $v0)))` on `{"a":{"b":1}}` answered `["a"]` where jq raises "Invalid path
   expression with result {\"b\":1}", and `=`/`\|=`/`del()` through the identical filter wrote
   to (`{"a":999}`) or deleted from (`{}`) a document jq refuses to touch at all — because
   `.zzz` (a missing-key access, a genuine navigation step to `null`) already, correctly, left
   the branch untracked one layer in, in `resolve_seq`'s own finer-grained tracking; `relocate`
   asked the identical question again with only the stale, pre-navigation register available
   and answered wrong. Fixed the same way as `advance` above: `identical()` is now gated on
   `cannot_move_register(expr)` for `expr` the expression just resolved, so it stays available
   only for the bare-`$var`/arithmetic/construction shapes it exists for, and defers entirely
   to `resolve_seq`/`resolve_node`'s own already-correct verdict whenever `expr` could have
   navigated. `resolve_reduce`'s own final-emission call site (which builds its branch directly
   from the fold's accumulator, not from a `resolve()` call over one expression) keeps
   `identical()` unconditionally available, unaffected by this gate.

   **Known residual of #2860's own fix, refuse-only**: `cannot_move_register` is a *syntactic*
   allowlist — for `if`/`try` it requires every branch to be navigation-free, not only the one
   actually taken (deliberately conservative for its two pre-existing callers, `resolve_seq`'s
   own carrying and `advance`'s above, where a wrong `false` only ever costs a refusal there
   too). Gating `identical()` on it inherits that same conservatism, so a navigating branch
   sitting *unreached* alongside a safe one now also costs a refusal here, even when the
   register genuinely never moved on the path actually executed: `path(. as $x \| foreach
   (1,2) as $i (0; (if false then .a else $x end); .))` is jq's `[]` twice — the `else` branch
   never navigates — but succinctly now refuses, where the pre-#2860 unconditional `identical()`
   happened to accept it. Not a fresh gap: it is the identical "scope limit, deliberately not
   closed" already accepted just above for a `$var` referenced directly inside `if`/`try` with
   no register threaded in at all — #2860's fix reaches it through a different door (a gate
   that didn't exist before) rather than opening a new one. A write through the affected filter
   fails rather than corrupting anything (jq's own `99`; succinctly raises), the same safe
   direction every entry in this section keeps to.

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
   unchanged: `substitute_bound_var_from`'s widening is jq-mode only
   ([#2643](https://github.com/rust-works/succinctly/issues/2643)). The sibling refusal is
   itself the general (non-`null`/`bool`) case: jq's own `jv_identical` admits
   `null`/`true`/`false` by value regardless of node, so the same shape on `{"a":true,"c":true}`
   answers `["c"]` on both
   ([#3136](https://github.com/rust-works/succinctly/issues/3136)).

   Eleven rows stay refuse-only, each pinned in `test_path_bind_origin_matrix_refuse_only_2042`
   (`src/jq/eval.rs`) and `scripts/jq-bind-origin-oracle-sweep.sh`'s own `REFUSE_ONLY` list:

   #3049 moved `path(.a as $y | .a | tojson | fromjson | $y)` to the accepting
   matrix: `fromjson` does not navigate, so it can preserve the register for `$y`.

   | Filter                                                   | jq                          | Why succinctly still refuses                                                                                                                                                                                              |
   | -------------------------------------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
   | `.a as $y \| path(.a \| $y)`                             | `["a"]`                     | value-mode binding — `eval_as` never resolves its source in path position; the *positional* half of #3037 (its root-of-invocation half is closed)                                                                                                          |
   | `path(.a as $y \| def f: $y; .a \| f)`                   | `["a"]`                     | a `def` inside `path()` resolves as an opaque leaf                                                                                                                                                                        |
   | `path(.a as $y \| ([$y] \| .[0]) as $z \| .a \| $z)`     | `["a"]`                     | the source navigates inside a construction, which the resolver refuses where jq's suspended tracking allows it, so it falls back to a plain value                                                                         |
   | `path((.a \| select(.b)) as $y \| .a \| $y)`             | `["a"]`                     | the witness grammar is pure navigation; a `select`-wrapped source binds by value                                                                                                                                          |
   | `path((.a // 1) as $y \| .a \| $y)`                      | `["a"]`                     | same: a `//` source binds by value                                                                                                                                                                                        |
   | `path((if .a then .a else .b end) as $y \| .a \| $y)`    | `["a"]`                     | same: an `if` source binds by value                                                                                                                                                                                       |
   | `path(.a? as $y \| .a \| $y)`                            | `["a"]`                     | a `?` step is a distinct path component, so it never matches the plain spelling on either side (spelling, not node identity)                                                                                              |
   | `path(.a[0:3] as $y \| .a \| $y)` on `{"a":[1,2,3]}`     | `["a"]`                     | jq's full slice *is* the array; the bind path ends in a slice component and `.a` does not                                                                                                                                 |
   | `path(.a as $y \| (.c \| $y \| .b) as $w \| .a.b \| $w)` | `["a","b"]`                 | a marker is re-rooted only at the head of a source (`$y.b as $w`, `(($y \| .b) \| .c) as $w`); elsewhere it is certified against the ambient position                                                                     |
   | `path(.a[1:] as $y \| .a[1:3] \| $y)` on `{"a":[1,2,3]}` | `["a",{"start":1,"end":3}]` | jq's `.a[1:]` and `.a[1:3]` of a 3-array are the same jv (same offset and length); the slice components differ, so the spelling never matches                                                                             |
   | `path(.a as $y \| .a \| 5 \| reduce (1) as $i (0; $y))`  | `["a"]`                     | after a literal the register is only *carried*, and a fold whose INIT is untracked seeds its own register from the ambient literal — pre-existing: `path(. as $x \| 5 \| reduce (1) as $i (0; $x))` refuses too (jq `[]`) |

   Two related divergences were pre-existing and out of scope for #2042, tracked separately:
   **[#2642](https://github.com/rust-works/succinctly/issues/2642), now closed.** The *root*
   `Origin::Snapshot` marker used to fabricate a path across a rebuilt-copy boundary (`. as $x
   \| {a:1} \| ($x.a) = 9` on `{"a":1}` wrote `{"a":9}`; jq refuses) — the same bug class as
   #1466, reached via a rebuild instead of a sibling. Fixed by `RootWitness`/
   `demote_rebuilt_markers` (`src/jq/eval.rs`): every "funnel" call site handing an expression
   from the generic (cursor-based) evaluator to the owned-value evaluator now demotes a
   `Snapshot` marker not proven to name that call's own document node to `Untracked` first, so
   `Frame::certifies`'s unconditional `Snapshot => true` never gets a chance to admit the
   rebuilt copy — refuse-only by construction, since the only transition is `Snapshot →
   Untracked` and `Untracked` never certifies by node identity (a `null`/`bool` rebuilt copy
   still certifies by jq's own value-identity rule, #3136 — sound, since jq's `jv_identical`
   has no pointer identity for those three values at all, rebuilt or not). A residual
   remained here: jq's own reference-counted `jv` passes an embedded node
   through *without copying it* — `{k:.} \| .k`, `. + {}`/`. * {}` (and `. + null`/
   `null + .`) when the other operand is empty/`null`, a fold whose UPDATE returns its
   accumulator unchanged, `add`/`min`/`max` over any container whose winning element is
   that node (`[.] \| add`, `[.,.] \| max`, `[null,.] \| add`, `{a:.} \| add`),
   `[.,.] \| .[1]`, `[[.]] \| .[0][0]`, and `{k:.} \| getpath(["k"])` all hand back the *same* `jv`
   `$x` was bound from — so `path($x)`/`($x...) = ...`/`del($x...)` afterward should
   answer exactly where jq's own `jv_identical` holds, but every one of these refused
   rather than silently accept a copy, since succinctly's `OwnedValue`-cloning model had
   no way to tell "this position embeds the original node" from "this position merely
   happens to be value-equal".

   **[#2889](https://github.com/rust-works/succinctly/issues/2889) closes the
   funnel-crossing half of this residual with an "owned embed map".** A thread-local,
   bind-scoped `embed_table` (`eval_generic::embed_table`, std-only) records, for the
   dynamic extent of a bind's body, the `(node, document)` pair a `BindOrigin::Node`
   bind froze its value from, keyed to the very `Rc` the bind holds. A later
   materialization of that same document node reuses that `Rc` instead of building a
   fresh one, so `{k:.}` embedding `.` shares storage with the bind's own copy. The reuse
   is taken at the *depth-0 entry* of each converter — `to_owned_cursor`
   (`eval_generic.rs`) and `to_owned` (`eval.rs`) — never inside
   `to_owned_at_depth`/`to_owned_cursor_at_depth`'s own recursion: depth 0 is where every
   embedding construction materializes its operand, and a reuse deeper in some *other*
   node's walk would skip the `MAX_NESTING_DEPTH` accounting for the shared subtree.
   `RootWitness::of_owned` (jq mode only) then looks an owned root's storage up in the
   table to recover a `Node` witness where the existing machinery previously always saw
   `Owned`, and `marker_needs_demotion`'s `Node`/`Node` arm certifies it exactly as it
   already does for a live cursor. Because the table's entry holds its own strong
   reference, a write through *any* handle to that storage still copies first
   (`Rc::make_mut`) — the same condition under which jq's `jv_identical($x, .)` holds,
   since the bind's own reference is what forces jq's copy-on-write too — so the write
   direction is exactly as sound as the read direction: `{k:.} \| .k \| ($x.a) = 9`
   writes `{"a":9}`, and `{k:.} \| .k \| .b = 2 \| path($x)` (a write happens first,
   breaking the sharing) still refuses. `arith_add` gained an empty-right-operand fast
   path (return `left` untouched, mirroring jq's own `jv_array_concat`/
   `jv_object_merge` loop over the right operand) so `. + {}`/`[.] + []` reuse the same
   `Rc`; `arith_mul`'s existing empty-right-operand return already did. An empty
   **left** operand (`{} + .`, `[] + .`) still builds a new container in jq and
   correctly keeps refusing.

   **Perf**: interleaved A/B on 2 MB and 10 MB `users`/`wide` corpora (identity gate, 0
   diffs), Apple M4 Pro and AMD 7950X. The embed shapes that used to materialize a second
   copy of the same node — `. as $x | {k:.} | .k`, `. as $x | . + {}`,
   `.[] | . as $r | {k:.} | .k` — got 30-57% faster on both chips, since the second
   materialization is gone; every shape with no `as` binding, and every scalar-bind shape,
   stayed inside the control run's noise floor.

   **Code review widened the fold and narrowed the peel.** `eval_owned_relocating_fold`
   was reached only from the generic evaluator's `eval_on_owned`, which sees a container
   built by `[.]` and nothing wider: every array or object with more than one element
   collapses to an owned value and re-enters through `eval.rs`'s own
   `eval_each_owned` instead, where the fold was never consulted. It now runs from both
   re-entries, and as a pipe *stage* (`max \| path($x)` is how it arrives there, not a
   bare `max`), so `[.,.] \| max`, `[{a:1},.] \| max`, `[.,{a:1}] \| min`,
   `[null,.] \| add`, `[.,null] \| add` and `{a:.} \| add` all answer — with a
   pre-gate that declines before cloning anything unless some element already shares a
   table entry's storage, so `[1,2,3] \| add` costs one scan and takes the bridge
   exactly as before. Which of two *equal* elements wins is jq's own and is pinned both
   ways: `max` keeps the last (`[{a:1},.] \| max` answers, `[.,{a:1}] \| max` refuses
   in jq too), `min` the first. The peel gained the matching narrowing: it fires only
   where the navigated value, or an iterated child, still shares a table entry's
   storage — the process-wide "a bind is in scope" flag alone had re-shaped every owned
   navigation inside any container bind's dynamic extent. Two escapes keep that from
   costing anything: a navigation whose next stage is another navigation (a chained
   `.k.j` reaches its embed only at the last step), and a `.[]` whose tail the peeled
   children can answer with no index of their own — gating that shape unconditionally
   measured +20% on a 100k-element owned array (`. as $x | [.items[]] | .[] | .a`),
   where gating only the re-indexing tails gained ~2.5%.

   Residuals that stay refuse-only, each with why (pinned in
   `scripts/jq-bind-origin-oracle-sweep.sh`'s `owned-embed-refuse-*` rows and exercised
   by `scripts/jq-bind-origin-fuzz.py`'s `EMBEDS`/`REBUILDS` pools):

   | Filter                                                                                                                                                                                                                            | Why still refused                                                                                                                                                                                                                                                                                                                                                                                                                                         |
   | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
   | `{k:.} \| scalars (`OwnedValue::String`, numbers) are not `Rc`-backed, so they never enter the embed table and have no storage for #3177's clause to share; giving them one (ADR-0024's option D) was built, measured in #3182 and rejected (+8% to +50% peak RSS on scalar-heavy rows). The sweep's `scalar-*` family is the full matrix                                                                                                                           |
   | `[.] \| (path(.[0] \| $x), path(.[0] \| $x \| .a))`, `[.] \| [limit(1; path(.[] \| $x))]`, `[.] \| path(.[0] \| $x)?` (the last swallows the refusal into no output)                                                              | a `path()` reached only through a wrapper (a comma, an array constructor, `limit`, `?`/`try`) is not at the head of the owned re-entry's pipe, so #3177's door does not open and the value still crosses the bridge; taking it natively would mean re-implementing the wrapper's own driver over an owned value — [#3189](https://github.com/rust-works/succinctly/issues/3189)                                                                           |
   | `[.] \| path(.[0] \| $x) as $p \| $p`, `[.] \| reduce path(.[0] \| $x) as $p (0; 1)`, `[.] \| select(path(.[0] \| $x) == [0])`, `[.] \| first(path(.[] \| $x) \| .[0])`, `[.] \| label $out \| path(.[0] \| $x) \| ., break $out` | the same rule from the other side: a `path()` that is a bind's *source*, a binary operand, or the body of `first`/`label` is in no position the door looks at either — the door opens only for the head of the pipe the owned re-entry is handed, and `as`/`reduce`/`==`/`first`/`label` each own the driver that runs their inner pipe (#3189)                                                                                                           |
   | `[.] \| sort\|reverse\|unique \| path(.[0] \| $x)`, `{k:.} \| to_entries \| path(.[0].value \| $x)`, `{k:.} \| with_entries(.) \| path(.k \| $x)`, `[[.]] \| add \| path(.[0] \| $x)`, `[.] \| .[0] \|= . \| path(.[0] \| $x)`    | jq answers all of these (`[0]`, `[0,"value"]`, `["k"]`): its `sort`/`to_entries`/`add`/`setpath` move the element's own `jv` into the new container. succinctly refuses because the stage *ahead* of `path()` folds through `eval_on_owned`'s JSON round trip first, so the array the resolver is then handed holds fresh copies and the storage clause has nothing to match — the same bridge that keeps `[.] \| sort \| .[0] \| path($x)` refused below |
   | `[.] \| del(.[0] \| $x)`, `[.] \| (.[0] \| $x) = 5`                                                                                                                                                                               | the `del` and assignment resolvers still cross the bridge; #3177's storage clause answers there as soon as they are routed like `path()` — [#3188](https://github.com/rust-works/succinctly/issues/3188)                                                                                                                                                                                                                                                  |
   | `.a as $y \| . as $x \| [.] \| path(.[0].a \| $y)`                                                                                                                                                                                | reuse is depth-0 only: `$y`'s value is a separate materialization of `.a`, not the `.a` inside `$x`'s storage, so the position `path()` walks to shares nothing with `$y` (jq `[0,"a"]`)                                                                                                                                                                                                                                                                  |
   | `[.] \| sort\|unique\|reverse \| .[0]`, `{k:.} \| to_entries \| .[0].value`, `{k:.} \| getpath(["k"])`, `[.] \| .[0] \|= . \| .[0]`, `[1,2] \| .[0:2]` (all `\| path($x)`)                                                        | none is one of `embed_peel_step`'s recognized head stages (`.foo`/`.[n]`/`.[]`/`add`/`min`/`max`, after leading `.` stages are skipped), and a write (`\|=`) always runs through the assignment resolver first — so each re-indexes on the owned route before the read reaches the shared value                                                                                                                                                           |
   | `reduce (1) as $i (.; if true then . else 1 end) \| path($x)`                                                                                                                                                                     | the UPDATE is not one of the owned fast paths (`eval_owned_navigation`/`eval_owned_relocating_fold`), so the fold's own hoisted per-step reroot rebuilds the accumulator before `path($x)` reads it                                                                                                                                                                                                                                                       |
   | a marker inside a fold's UPDATE naming the accumulator                                                                                                                                                                            | `reduce_forks`/`foreach_forks`'s hoisted per-step reroot demotes UPDATE against `Owned` once by design, not upgraded here — an owned witness lookup on every fold step priced +3% (#3036)                                                                                                                                                                                                                                                                 |
   | `.a as $y \| . as $x \| {k:.} \| .k.a \| path($y)` (an embed reached through a container built from an *ancestor* of the bound node)                                                                                              | reuse is taken only at a materializer's own depth 0, never inside its recursion, so `.k.a`'s deeper position never asks the table — the same rule that keeps `MAX_NESTING_DEPTH` accounting exact for the shared subtree                                                                                                                                                                                                                                  |
   | `no_std` builds                                                                                                                                                                                                                   | `embed_table` is thread-local; without `std` it is a no-op, refuse-only like `file_index`                                                                                                                                                                                                                                                                                                                                                                 |
   | `succinctly yq`                                                                                                                                                                                                                   | unchanged by design — `RootWitness::of_owned` is gated on `S::TAG == EvalTag::Jq`; yq's node model is #2643's business (ADR-0018)                                                                                                                                                                                                                                                                                                                         |

   **[#2575](https://github.com/rust-works/succinctly/issues/2575)
   closed one row of this residual as a side effect, not a targeted fix**: `[.] \| .[0] \|
   path($x)` now stays accepted, because `Expr::Array`'s generic-evaluator arm no longer
   materializes a cursor-shaped inner result into a fresh `OwnedValue` copy — `[.]`'s single
   element is a `LazySeq` pointing at the same cursor `.` came from, which is exactly the node
   identity jq's own `jv` already had. `[.] \| path(.[0] \| $x)` (the same construction, but
   with the navigation happening inside `path()`'s own argument) was a genuinely different
   mechanism, closed by #3177 below.
   **[#3177](https://github.com/rust-works/succinctly/issues/3177), now closed: the embed
   navigated to *inside* `path()`'s argument, and the `.` stage over a one-element `[.]`.**
   jq's `jv_identical` on a container is pointer equality, and every construction built
   inside a bind embeds the bind's own `Rc` (#2889), so `. as $x \| [.] \| path(.[0] \| $x)`
   is `[0]` in jq — the path navigated *inside* the call — and so are `[.,.] \| path(.[1] \|
   $x)` (`[1]`), `{k:.} \| path(.k \| $x)` (`["k"]`), `[[.]] \| path(.[0][0] \| $x)`,
   `[.] \| path(.[] \| $x)` and `[.] \| path(.[0] \| $x \| .a)` (`[0,"a"]`). #2889's root
   witness can only speak for the root, and `Origin::SnapshotAt` cannot express a position
   the resolver certifies (`Frame::certifies` admits it by value). Two pieces close the
   class: `marker_identical` now certifies a marker whose value shares the container storage
   the resolver stands on, whatever its origin (`OwnedValue::shares_storage_with` — the rule
   itself rather than a witness of it, sound because the marker's own strong reference
   forces every write to copy first, and jq-only per ADR-0018); and `owned_path_door` hands
   a `path(f)`-headed owned re-entry the caller's own `OwnedValue` instead of serializing it
   through `to_json_for_reindex` and re-indexing, which is what let the resolver see the
   pointer at all (the same route the generic evaluator's cursor-side `path()` arm always
   took, now one shared helper, `path_over_owned`). A rebuilt equal value shares nothing and
   still refuses as jq does (`[.,{a:1}] \| path(.[1] \| $x)`). The `.`-stage row
   (`[.] \| . \| max \| path($x)`, `[.] \| . \| .[0] \| path($x)`, `[.] \| (. \| max) \|
   path($x)`) was separate: `fold_lazy_seq_stage` now materializes an instruction-free cursor
   sequence at an identity stage and hands the array on as it is, instead of through
   `eval_on_owned`'s round trip, so the fold sees the same array the wider spellings did. It
   materializes rather than passing the sequence on lazily, because the `.` is a
   materialization barrier in both orders: `[.[]] \| map(f) \| . \| first` *and* `[.[]] \| .
   \| map(f) \| first` raise on a failing second element as jq's eager `map` does, where a
   lazy sequence's `first` would not (#725's documented divergence, which must not spread to
   a new spelling — the first cut of this fix passed the sequence on and did exactly that for
   the second order). Materializing routed the one-element `(. \| .[0])` onto a gap the
   wider spellings already had: `embed_peel_step` matched a bare `Pipe` only, so a
   parenthesised stage on the owned route (`[.,.] \| (.[0]) \| path($x)`, `[.,.] \| (. \|
   max) \| path($x)`, `{k:.} \| (. \| .k) \| path($x)`) bridged and refused where jq answers
   `[]`. It now unwraps the parentheses and peels a stage that is the whole pipe (an
   `Expr::Identity` tail), under the same payoff gates. What stays refused is in the table
   above: scalars, a `path()` in any
   position but the head of the owned re-entry's pipe (#3189), the `del`/assignment
   resolvers (#3188), a stage ahead of `path()` that bridges first (`sort`, `to_entries`,
   `\|=`, ...), and an embed reached through an ancestor's bind. The same clause closed #2042's
   `path(.[0] as $y \| .[-2] \| $y)` residual (jq `[-2]`): the negative spelling never matched
   the bind path, but `.[-2]` stands on the very storage `$y` was bound from.
   **[#3036](https://github.com/rust-works/succinctly/issues/3036), now closed: the same
   fabrication through the routes that never cross a funnel.** #2642's check ran only where
   an expression is handed from the generic evaluator to `eval.rs`; when the bind *and* the
   rebuild both run inside `eval.rs` — the whole-program input-queue bridge (any program
   mentioning `input`/`inputs`/`input_line_number`), a fold's UPDATE, a `|=` right-hand side,
   `with_entries`' body, a `catch` handler, the owned-surface consumers' arguments — no funnel
   ran and `input | . as $x | {a:1} | ($x.a = 9)` still wrote `{"a":9}` (jq exit 5). Closed
   at the *re-entry* rather than by node identity: every place `eval.rs` re-indexes an owned
   value into a throwaway document (`eval_each_owned`, `eval_owned_input_reindexed`,
   `eval_owned_expr_full`, `eval_path_context_pipe_owned`, `builtin_with_entries`) demotes
   every `Snapshot` marker the expression carries first, because a freshly rebuilt document
   cannot be a node any earlier binding was frozen from — the one fact a `StandardJson` can
   always establish, where a positive witness cannot (an empty container or a scalar carries
   no cursor to recover a node id from). The generic funnels hand the re-entry the witness
   of the live cursor they bridge from (`Reentry::Against`, #3122) — only a caller that has
   already run that same demotion for this document passes `Reentry::Proven` — so the proof
   is kept (and a re-entry whose expression holds no assignment, builtin or call skips the
   rebuild, since only a resolver invocation can read a demotion). The two classes #2642
   deliberately excluded are closed with it: the owned-identity
   route (`eval_owned_identity_stages`/`owned_identity_values`, the `OwnedIdentity`/#2072
   machinery a `key`/`parent`/`path` sibling routes through) now checks against
   `OwnedIdentity::root_witness` — the base node itself while the value is that node's own,
   unrebuilt value (`OwnedIdentity::exact`), and otherwise the owned position's own token
   (`OwnedIdentity::root`, fresh for every root the pipe starts and every value a stage
   rebuilds at a position — `sort`, `to_entries`, a write, but not `select`/`objects`/`debug`,
   which hand their input on — and derived per child position), never `ancestors.is_empty()` alone,
   which would certify `sort`'s new array; `.foo | . as $x | (parent, ($x.a = 9))` still
   writes, `input | . as $x | ((path | empty), ($x.a = 9))` (a bind at a detached root, nothing
   rebuilt since) still writes, and `.foo | . as $x | sort | (parent, ($x[0] = 9))` now
   refuses — and a `catch` handler's
   markers are checked against the payload's own node (`try_payload_root`): `error($x)` and
   `$x | error` raise the marker's node verbatim, as jq's `catch` then sees the same `jv`, so
   `. as $x | try error($x) catch path($x)` stays `[]` wherever the raise sits in the body
   (a marker inside a resolved `def` body is reached too — `map_subexprs` leaves such a body
   alone, the demotion walk does not: `. as $x | def f: ($x.a = 9); {a:1} | f` wrote on `main`)
   (`try (.a | error($x)) catch …`, `try (if .a then error($x) else . end) catch …`), while a
   value-equal payload (`try error({a:1}) catch ($x.a = 9)` on `{"a":1}`) or a body with two
   raise sites naming different values no longer certifies. `eval.rs`'s own consumers that
   hand their *unrebuilt ambient input* to the owned bridge — `any`/`all`/`IN`'s generator,
   `until`/`while`/`repeat`'s first round, `recurse(f)`'s level 0, `debug(msg)`, `|=` on a
   first root path (`.`, `(.)`, `getpath([])`), and every leaf, condition, computed key or bind
   source a resolver evaluates while its ambient is still the register (`trackable`) — take
   the non-demoting bridge, so `. as $x | any(($x.a = 9); true)`, `. as $x | . |= ($x.a =
   9)`, `. as $x | until(($x.a = 9) | true; .)` and `. as $x | path(select(($x.a = 9) |
   true))` keep answering;
   a later round, a sub-path, or a second path of the same `|=` (whose root jq's `setpath`
   has already copied: `. as $x | (.b, .) |= (if type == "object" then ($x.a = 9) else .
   end)` wrote `{"a":9,"b":2}` before this, jq refuses) demotes. The refuse-only flips this
   made were the in-evaluator twins of the embed residual above — a stage that hands its
   input on as an owned copy is the same node to jq and a fresh document to `eval.rs`. At
   the time #3036 landed, `eval.rs`'s own bind sites (`eval_as`/`each_as`) minted no node at
   all — they called a bare `substitute_bound_var` with nothing for the embed table to key
   on — so every one of these refused regardless of the embed table #2642 already had:
   `input | . as $x | S | ($x.a = 9)` for S ∈ {`[.] | .[0]`, `{k:.} | .k`, `[., .] | .[0]`,
   `reduce empty as $i (.; .)`, `getpath([])`, `nth(0; .)`, `until(true; .)`,
   `ltrimstr("x")`, ...} and `input | . as $x | try error($x) catch path($x)`.
   `input | reduce (.) as $x (.; ($x.a = 9))` — a fold's own loop variable, demoted with the
   accumulator's re-index, as the generic fold has done since #2642 — is a different,
   unrelated residual: the loop variable is `Snapshot` with no node at all, embed table or
   not, so it is untouched by anything below.
   Pinned in `tests/jq_cli_tests.rs` (`*_3036`), swept by
   `scripts/jq-bind-origin-oracle-sweep.sh`'s `in-evaluator-*` rows and fuzzed by
   `scripts/jq-bind-origin-fuzz.py`'s `ROUTES` family.

   **[#2889](https://github.com/rust-works/succinctly/issues/2889) Stage B, now landed,
   mints that missing node.** `eval_as`/`each_as` mint a `BindOrigin::Node` from a
   `StandardJson` bind source's own cursor (`eval::standard_json_bind_origin`,
   `substitute_bound_var_from` — the bare, node-less `substitute_bound_var` this used to
   call had no other callers left once this landed, and was removed rather than kept
   alongside it) and push the same embed-table entry `each_as_generic` does
   (`eval::standard_json_node_cursor`/`JsonFields`/`JsonElements::whole_container_cursor`
   recover a container's own cursor from the *child* cursor `StandardJson` actually
   retains — the one `parent()` hop `eval_generic`'s own `V::Cursor` never needed). Every
   row in the `S ∈ {...}` set above now answers, read and write alike, along with the
   navigated-bind twins (`input | .a as $y | {k:.a} | .k | path($y)` and the `($y.b) = 9`
   write) — pinned in
   `test_owned_embed_keeps_node_identity_on_the_input_bridge_2889`. Three residuals remain,
   all specific to this route and pinned in
   `test_input_bridge_embed_residuals_refuse_cleanly_2889`:
   - An **empty container** (`input | . as $x | {k:.} | .k | path($x)` on `{}` or `[]`)
     binds no node at all: `whole_container_cursor` has no retained child cursor to hop
     `parent()` from, so the table never gets an entry. jq answers `[]`; the generic
     evaluator, handed a real cursor rather than one recovered after the fact, still
     answers it — the one place the two routes still disagree, in the safe direction.
   - A **further re-entry between the embed and the read** that carries no witness —
     collecting the pipe into an array (`[input | . as $x | {k:.} | .k | path($x)]`) or
     routing the embedded node through `getpath([])` first (`... | .k | getpath([]) |
     path($x)`) — loses the table hit the bare twin of each gets.
   - `input | .a as $y | .a | ($y.b) = 9`: the bind and the use are both plain cursor
     navigations with no owned re-entry between them at all, so no embed lookup is ever
     consulted (unchanged by Stage B — jq accepts, this refused identically before it too).
   And
   [#2646](https://github.com/rust-works/succinctly/issues/2646) — `first`/`last`/`add`
   navigating inside their own jq-level definitions against a *constructed* value inside
   `path()` never raise, found by `scripts/jq-bind-origin-fuzz.py`'s differential fuzz and
   confirmed pre-existing (reproduces byte-for-byte on the commit before #2042), not caused by
   this change. **#2646 is now fixed** — `builtin_navigation` (`src/jq/eval.rs`) answers what
   each jq-defined navigating builtin indexes, and `resolve_leaf`'s `!trackable` guard raises
   on it. Three groups remain, deliberately, and are divergences in their own right:

   | Still diverging                                                                     | jq 1.7.1                                        | succinctly | Tracked                                                                  |
   | ----------------------------------------------------------------------------------- | ----------------------------------------------- | ---------- | ------------------------------------------------------------------------ |
   | `[path(([1] \| unique) \| empty)]`, and `unique_by`/`map_values`/`with_entries`, `walk` on an *object*, `sub`/`gsub` (with or without flags), and every update assignment (`\|=`, a compound `op=`, `//=`; plain `=` is exempt, #3186) | raises, naming a **derived** container (`iterate through [[1]]`) once the construct's own type-check and argument evaluation have already succeeded | raises the same `ErrorKind::UntrackedNavigation` (ordinarily catchable, same as jq — confirmed live, `try with_entries(.)` and `try (.k \|= 3)` are both caught in both tools), naming the value the construct actually produced rather than jq's `to_entries`/`group_by`/`match`/`_modify` intermediate. Checked only *after* by-value evaluation succeeds, so jq's own type error, or an `error(...)`/`empty` from an argument or right-hand side, still wins exactly as it did before — a first cut of this fix checked `expr`'s bare shape before evaluating it and wrongly pre-empted all of those | [#3271](https://github.com/rust-works/succinctly/issues/3271), narrowing [#2743](https://github.com/rust-works/succinctly/issues/2743) |
   | `[path(([1] \| fromstream(...)) \| empty)]`, and `ascii_downcase`/`ascii_upcase`, the `match`/`scan`/`capture`/`splits` family (not `sub`/`gsub`, narrowed above by #3271) | raises, naming a **derived** container | `[]`       | [#2743](https://github.com/rust-works/succinctly/issues/2743) |
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

   **[#3037](https://github.com/rust-works/succinctly/issues/3037), now closed for the
   root-of-invocation rows: the accepting-direction twin of #2642.** #2072's navigated bind
   (`.a as $y`, `Origin::Untracked`) refused everywhere, including where the path register
   really *is* that node — `.a as $y | .a | path($y)` is `[]` in jq 1.7.1, and `($y.b) = 9`,
   `del($y.b)`, `$y |= 5`, `+=`, `//=` all write. At every funnel that hands a cursor's value to
   the resolver, the same node-identity proof #2642 reads in the demoting direction now runs
   in the accepting one (`reroot_markers`, `src/jq/eval.rs`): an `Untracked` marker whose
   recorded node *is* the funnel's own live cursor is the invocation root, and `Origin::Snapshot`'s
   value rule is sound for it by that variant's own argument (a root can value-match only itself
   among its proper descendants), so it is promoted to a `Snapshot` for that call — jq mode only,
   since real yq's assignment through a variable is a no-op that prints the document unchanged,
   where succinctly refuses loudly (a write there would diverge in the corrupting direction). A
   marker at an equal-valued sibling (`.a as $y | .c | ($y.b) = 9`), an ancestor or descendant,
   after a rebuild, or a multi-output source's other element (`.a[1] as $y | .a[] | …`) has a
   different node id and stays refused, as jq does. Closing it surfaced the last unrebased
   funnel: the assignment family (`=`, `|=`, `+=`, …) with a cursor falls through
   `eval_generic::eval_single`'s catch-all into `eval_full` whenever the owned identity pipe
   declines it, and that arm never demoted — so `. as $r | .a | . as $x | $r | .c | ($x.b) = 9`
   on `{"a":{"b":1},"c":{"b":1}}` wrote `{"b":9}` (and `$x |= 5` wrote `5`; also through
   `limit(1; …)` and a `def`) where jq refuses, while `del`/`path` of the same shape refused
   through the demoting funnels. Rebased like every other funnel now. **Still refusing**, each
   pinned in `test_navigated_bind_residuals_refuse_cleanly_3037` and the sweep's
   `navigated-bind-*` refuse-only rows: the positional rows (`.a as $y | path(.a | $y)`,
   `(.a | ($y.b)) = 9` — the marker must certify at a non-root register position, which needs
   a document-absolute bind path, the #2042 `Origin::At` machinery reached from a value-mode
   bind; scoped separately). The routes that re-enter the eager evaluator with an *owned*
   accumulator (`reduce (1) as $i (.; .a as $y | .a | ($y.b) = 9)`, a `catch` handler) were
   listed here too — `eval.rs`'s own `eval_as` carries no node for a navigated bind, so no
   witness could promote it — until #3177's storage clause certified the marker by the `Rc`
   it shares with the register, which needs no witness at all. The embed row this list used to carry (`.a as $y | {k:.a} | .k | path($y)`, #2889) is
   recovered by the same embed table described under #2642 above: `each_as_generic`
   pushes an entry for a navigated bind's node exactly as it does for an identity bind's,
   so `marker_is_root`'s existing promotion succeeds once `RootWitness::of_owned` reports
   the funnel's owned root as that same node. Of the rows that remain refused above, the
   `reduce`/`catch` pair is a route through `eval.rs`'s own evaluator whose bind source is
   already an owned accumulator (`Item::Owned`), not a `StandardJson` cursor — Stage B
   (`docs/plan/jq-bind-origin-frame.md`) mints a node only from the latter
   (`eval::item_bind_origin`'s `Item::Borrowed` arm), so these two stay exactly as they
   were, unrelated to whether Stage B has landed. A navigated bind on an *owned-rooted*
   document (`input | .a as $y | .a | path($y)`, `jq -n '{a:{b:1}} | .a as $y | .a | ($y.b) = 9'`,
   a `tojson|fromjson`-rebuilt root) was a different limitation again, which neither Stage B
   nor the embed table touched — the marker's node is an `OwnedIdentity` position, not a
   document node — until [#3135](https://github.com/rust-works/succinctly/issues/3135): such a
   pipe left the generic evaluator as an owned value and crossed into `eval.rs` under
   `Reentry::REBUILT`, whose own `eval_as` cannot certify a navigated bind at a register, so
   nothing promoted it. It now runs on the owned identity pipe
   (`eval_generic::owned_identity_bind_door`, opened by every owned re-entry that is still
   `REBUILT` after `Reentry::witnessed_by` — an embed-table hit keeps its node proof instead —
   when a navigated `as` is followed by a resolver that reads its variable; jq mode only),
   whose position tokens name the bind's node, and `marker_is_root` reads them against the
   funnel's `OwnedRoot` witness exactly as `marker_needs_demotion` already did in the demoting
   direction. A sibling, a rebuild or a construction between the bind and the use derives a
   different token and keeps refusing, as jq does; the sweep's `owned-root-*` rows pin both
   directions.
   A `null`/`bool` marker at an equal-valued sibling (`.a as $y | .c | $y |= 5` on
   `{"a":true,"c":true}`) used to be refuse-only the same way — jq's `jv_identical` admits
   those by value regardless of node, but the resolver's `TrackedVar` arm consulted the
   origin first — closed by
   [#3136](https://github.com/rust-works/succinctly/issues/3136), which ORs in the same
   null/bool value-identity carve-out `register_identical` already makes, gated the same way
   by the existing node-identity check. The library entry point `succinctly::jq::eval`
   takes the eager evaluator for a program `needs_path_context` does not route (a `path(f)`
   with an argument, a write), so through it these rows keep refusing as before; the CLI's
   route is the generic evaluator, where they answer.

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
   **artefact guard**. This resolver can raise refusals jq never raises — until
   [#3133](https://github.com/rust-works/succinctly/issues/3133) a nested `Pipe` carried no
   register, so a `$q[0]` under an `if` or a `,` raised near-access even though the register
   was standing right there — and retrying on one of those lands on the *wrong* alternative,
   which a write then goes through. `path(. as {a:$q} ?// $z \| if $q then $q[0] else $z end)`
   is `["a",0]` in jq and would otherwise have answered `[]`, and the matching
   `del(. as {a:$q} ?// $z \| if $q then $q[0] else $z end)` on `{"a":[1,2,3]}` is `{"a":[2,3]}`
   in jq and would have deleted through that fabricated `[]`. So a body error of the resolver's
   own two kinds (`UntrackedNavigation`/`InvalidPathExpression`) never retries. (Since #3133
   the untracked stage's `Frame` carries the register into that nested pipe, so both rows
   answer as jq does — the guard is kept for the refusals that remain artefacts, such as a
   register lost to an opaque stage.) (On an
   untracked stage the arm once kept the old opaque-leaf fall-through, since it could not see
   the register the first step is compared against; since
   [#3120](https://github.com/rust-works/succinctly/issues/3120) `resolve_seq_stage` hands it
   the register the pipe carried in, see the #2979 paragraph below.) Genuine jq errors
   (`Cannot index X with Y` from the walk or the body, `error(..)`, `Break`) retry as before,
   and branches already emitted stay emitted, as in jq
   (`path(. as {a:$q} ?// {b:$r} \| $q, $r)` prints `["a"]` and then raises, in both). Both
   rows above are pinned as soundness rows in `test_destructuring_moves_path_register_2649`
   (`tests/jq_cli_tests.rs`).

   The guard's price, and the rest of the residue, is **refuse-only** — succinctly declines
   where jq answers, never the reverse, and no row writes. Each is pinned in that same CLI test
   or in `test_path_destructure_matrix_refuses_2649` (`src/jq/eval.rs`), on
   `{"a":[1,2,3],"b":{"c":5}}`. (Three rows this table once carried — a nested pattern on the
   bound copy, `path(. as {a:$q} \| $q as [$x] \| $x)`, and the two marker-headed sources on an
   untracked stage, `path(. as $x \| 5 \| $x as {a:$q} \| $q)` and `path(.b as $y \| .b \| 5
   \| $y as {c:$w} \| $w)` — answer since #3120 gave the arm the carried register. A fourth,
   `path(. as {a:$q} ?// {b:$r} \| if ([3,1]\|sort\|.[0]==1) then $q else $r end)`, answers
   `["a"]` since #3186 stopped `cannot_move_register` recursing into an `if` condition, which
   jq compiles as a subexp.)

   | Filter                                            | jq                 | Why succinctly still refuses                                                                                                                 |
   | ------------------------------------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------- |
   | `path(. as {a:$q} \| .a as $z \| $z)`             | `["a"]`            | a bind whose source navigates needs a trackable stage, and the pattern's body stage is untracked by construction                             |
   | `path(. as {a:$q} ?// $z \| .a)`                  | `["a"]`            | jq's fork catches the body's own near-access error and tries the next alternative; here that error is indistinguishable from a resolver artefact, so the guard refuses instead (a `PATH_END` refusal, by contrast, reaches the loop as a sink stop and does retry)                                                             |
   | `path(. as {a:$q} \| $q[0], $q)`                  | `["a",0]`, `["a"]` | pre-existing: a `$var` nested under `,`/`if` gets no register (the scope limit above) — `path(.a as $y \| .a \| 5 \| $y[0], $y)` refuses too |
   | `path(. as {a:$q} \| select(true) \| $q)`          | `["a"]`            | pre-existing: a `select`/`label`/`first(.)`/`getpath([])` passthrough on an untracked stage re-seeds the carried register from the ambient value — `path(.a as $y \| .a \| 5 \| select(true) \| $y)` refuses too; `if`/`try`/`. as $q \| .` carry it |
   | `path((., .) as {a:$q} \| $q)`                    | `["a"]`, `["a"]`   | the identity premise is decided from the source's spelling (`.`, a certified marker, null/bool by value); an identity-*equivalent* source (`(., .)`, `getpath([])`, `first(recurse)`, a `def` parameter bound to `.`) binds by value and the first step refuses |
   | `path(. as {a:$q} \| $q[($q\|length)-1])`          | `["a",2]`          | a computed index on a marker head resolves the key off the ambient input, so the marker never re-establishes; `$q \| .[length-1]` answers                                                                                   |

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

2. **`?//`-alternatives folds are path-tracked** since
   [#2979](https://github.com/rust-works/succinctly/issues/2979), with jq's backtracking
   rules: a pattern-walk refusal on a non-last alternative retries the next one with the
   accumulator untouched; an UPDATE escape retries with the accumulator at UPDATE's last
   output before it (`null` when there was none); for `foreach`, an escape after the state
   was stored (EXTRACT, or `path()`'s own terminal refusal) retries from that stored state
   with what was already emitted kept; the last alternative's escape propagates, and `halt`
   never retries. `path(. as $x \| reduce (1) as $y ?// $z (0; $x))` on `{"a":1}` is `[]`, as
   in jq, and `del(foreach .a as {b:$v} ?// [$v] (.; .; $v))` on `{"a":[1]}` writes
   `{"a":[]}`. Before #2979 a chain fell to the by-value catch-all, which refuses only on an
   output it emits, so `path(foreach .a as {a:$v} ?// $v (.c; .; empty))` exited 0 where jq
   refuses partway through the construct, and `del`/`=`/`\|=` wrote through it.

   Two refusals remain where jq might answer, and one refuses with different wording, all
   deliberate: the resolver's *own* refusals
   never retry (before #3133 a nested pipe carried no register, so `$q[0]` inside an `if`
   refused here where jq navigates, and retrying on that artefact would have bound a different
   alternative than jq and written through it; the rule stays for the artefacts that remain),
   and a walk refusal retries only when it is jq's own verdict.
   For a source element that is not register-derived, the walk compares the element with
   the register by value where jq compares nodes; when the two are equal and not
   `null`/boolean, or the register is lost, the refusal is a guess and propagates instead:
   `path(foreach .a as {a:$v} ?// $v (.c; .; $v))` on `{"a":2,"c":2}` refuses on both, jq at
   the pattern step and succinctly without retrying `$v`.

   The third is the same "resolver's own refusal never retries" rule landing on a bare
   `$var` alternative instead of a destructuring one: `path(foreach (1) as $x ?// [$z]
   (null; .; $x))` refuses here on `$x`'s own refusal ("with result 1"; `$x` is bound to a
   SOURCE value that is not itself register-derived — here a literal, not navigation), where
   live jq 1.7.1 retries into `[$z]`'s own destructuring failure against the same value and
   refuses there ("near attempt to access element 0 of 1"): both exit 5, and only the wording
   differs. Confirmed narrow: with a realistic
   `.`-navigated source both agree (`path(foreach .a as $x ?// [$z] (null; .; $x))` on
   `{"a":1}` is `["a"]` in both) — the divergence needs a SOURCE that is genuinely not
   navigation-derived at all. Loosening `is_resolver_refusal`'s retry-suppression to special
   case this one shape would risk the already-verified `$q[0]`-inside-`if` case above
   regressing, since both are the same rule; not planned for closure on that basis.

   A `?//` chain of a plain `as` bind on an *untracked* stage no longer falls to the
   by-value catch-all either: `path(5 \| . as {a:$v} ?// $v \| .b?)` refuses on both (it
   exited 0 here before).

   **The register on an untracked stage** —
   [#3043](https://github.com/rust-works/succinctly/issues/3043),
   [#3120](https://github.com/rust-works/succinctly/issues/3120) (and
   [#3124](https://github.com/rust-works/succinctly/issues/3124) part A). Until #3120 the
   arm seeded the pattern walk's register from the stage's *own* value, which on an untracked
   stage is not jq's register at all. A `null` literal stage therefore let the walk's
   null-identity clause pass a step jq refuses against the document root — `path(null \| . as
   [$v] \| empty)` on `{"a":1}` exited 0 with nothing where jq exits 5, `del(null \| . as [$v]
   \| empty)` echoed the document, `(null \| . as [$v] \| empty) \|= 5` too, and a body whose
   own navigation is `?`-suppressed (`path(null \| .a as {arr:[$v]} \| .arr[]?)`) pruned the
   deferred refusal outright. `resolve_seq_stage` — the one frame that holds the *carried*
   register (#2046) — now dispatches an untracked `AsPattern` stage to `resolve_as_pattern`
   with it, and the first step is checked against that: every row above refuses with jq's own
   message, while the same shapes accept exactly where jq does (`path(.a \| null \| . as {b:$v}
   \| $v)` on `{"a":null}` is `["a","b"]`, the register being `null`). With the register in
   hand, two families that were refuse-only for want of it now answer: a marker that *is* the
   register at the head of the pattern (`path(. as $x \| 5 \| $x as {a:$q} ?// $z \| $q)`,
   `["a"]`, and `del` of it, `{}`), and a `?//` whose refused first alternative is provably
   jq's own verdict — the source is not even value-equal to the register — which retries as
   jq's fork does (`path(5 \| 5 as {a:$v} ?// $v \| empty)` answers nothing, exit 0, and
   `path(5 \| . as {a:$v} ?// $v \| .b?)` now refuses with jq's own `element "b" of 5`). An
   equal-valued non-`null`/`bool` source is still a guess (jq compares pointers) and keeps
   the no-retry rule: `path(.a \| {b:1} \| . as {b:$v} ?// $z \| $z)` on `{"a":{"b":1}}`
   refuses on both, jq at `PATH_END` and succinctly at the step.

   A bare-`$z` alternative reached by a retry keeps the register in hand too (review): its
   body is seeded like a stepped one, so `path(. as $x \| 5 \| $x as [$v] ?// $z \| $z)` is
   jq's `[]` and `del(... \| $z.a)` its `{}` (both refused before, `$z` having nothing to
   re-establish against).

   Where no register can be handed in, the walk refuses unconditionally rather than guess.
   Two of the residuals #3120 recorded here — a `catch` handler with no register, and a pipe
   nested under `if`/`try` with a `null`/`bool` register behind a non-matching literal — are
   closed by [#3133](https://github.com/rust-works/succinctly/issues/3133) below. What
   remains, each pinned in `scripts/jq-bind-origin-oracle-sweep.sh`:
   - an *opaque* stage (`reduce`, a `def` call, `first(..)`) drops the carried register
     (`cannot_move_register`, #1573), so after one the walk refuses without retrying:
     `del(reduce 1 as $i (null; null) \| . as [$v] ?// $v \| empty)` on `{"a":{"b":null}}`
     echoes the document in jq (the step is refused there too, and the retried `$v` body is
     `empty`) and exits 5 here. `main` echoed it by the ambient-`null` coincidence the fix
     removes.
   - a later-step refusal after a marker-certified first step does not retry `?//` (review):
     `refusal_is_exact` is decided per source (`bound != register`), and a marker that *is*
     the register is value-equal to it, so `path(.a as $x \| .a \| 5 \| $x as {b:[$q,$r]} ?//
     $w \| $w)` on `{"a":{"b":[1]}}` refuses at element `0` of `[1]` where jq retries onto
     `$w` and answers `["a"]`; the same shape on a trackable stage retries and agrees.

   **The register reaches a nested pipe and a `catch` handler** —
   [#3133](https://github.com/rust-works/succinctly/issues/3133). A pipe nested under
   `try`/`if`/`,` is resolved by `resolve_node_sink`, which has no `PathBranch` to carry the
   register in, so it used to start register-less: a `$w` marker there could not
   re-establish, its navigation raised the resolver's own refusal, and `try` caught it as jq's
   `try` catches its own path errors — but jq had no error to catch, since `$w` *is* its
   register, and the write was silently discarded: `del(. as {a:$w} \| try ($w \| .b))` on
   `{"a":{"b":1}}` echoed the document where jq writes `{"a":{}}` (and `del(.a as $x \| .a \|
   5 \| try ($x as {b:$q} \| $q))` likewise, the same loss in the source position). An
   untracked stage's `Frame` now carries the register's *value* beside its position, and a
   nested pipe seeds itself from it; the outer stage trusts a trackable step out of such a
   stage, since from an untracked input only a re-establishment against that very register
   can produce one. A `catch` handler gets the register jq restores at the `try`'s entry (its
   fork point saves and restores the path state — confirmed live, `null \| path(try (.a \|
   error(null)) catch .b)` is `["b"]`, and with a non-null register the handler's `.b`
   refuses): `path(.a as $y \| .a \| try error(1) catch $y)` is jq's `["a"]` (the
   `catch-handler-var` row the #2042 table carried), and the `null`-document destructuring
   row #3120 recorded as a residual answers again — for the right reason this time. The
   handler's output is still handed on to the stages after the `try` rather than refused on
   the spot (`[path(.a \| (try error(null) catch .) \| empty)]` is `[]`).

   The same change closes a pre-existing write-through: `. as $x` on an *untracked* stage
   binds a value the stage computed, not a node, yet carried `Origin::Snapshot`, whose value
   rule then certified a constructed copy against the register — `del(.a \| {b:{c:1}} \| . as
   $x \| $x)` on `{"a":{"b":{"c":1}}}` deleted `.a` where jq refuses, and with the register
   now reaching nested pipes `... \| $x \| .b \| .c` would have navigated through it too.
   Such a bind is `Origin::Untracked` now (`identity_bind_position`); a marker source keeps
   its own origin, and a `null`/`bool` `.` loses nothing since `jv_identical` admits those by
   value. [#3145](https://github.com/rust-works/succinctly/issues/3145) extends the same
   carrying to a fold's UPDATE/EXTRACT body, whose own route (`FoldRegister::resolve`) passed
   its register to `resolve_seq` explicitly under a frame that carried none: `del(foreach .a as
   $v (.; try ($v \| .b); .))` on `{"a":{"b":1}}` echoed the document and now writes
   `{"a":{}}`, as jq does. It does not reach a fold whose INIT is untracked *after a literal
   stage* — that fold re-seeds its register from the ambient value, so the marker is not
   recognised at all (the pre-existing `literal-then-fold-untracked-init` /
   `carried-register-passthrough` class), and with a generator source the enclosing `try` then
   swallows that refusal into a no-op write: `del(.a as $y \| .a \| 5 \| foreach range(1) as $i
   (0; .; try ($y \| .b)))` is `{"a":{}}` in jq and echoes here. Two refuse-only residuals,
   both in the sweep: `path(.a \| try error(.) catch .)` —
   `error(.)` raises the register node itself and jq answers `["a"]`, but a payload equal to
   the register by value cannot be told from a rebuilt copy (`error({"a":1,"b":2})` refuses in
   jq), so the handler stays untracked unless the payload is `null`/`bool`; and `(if true
   then $x else . end) as $y` on an untracked stage, which binds `Untracked` because the
   condition is not evaluated where jq evaluates it and binds the marker.

   **`resolve_as_pattern`'s own first-step identity test recognizes every
   `is_identity_passthrough` spelling now** —
   [#3119](https://github.com/rust-works/succinctly/issues/3119). It used to recognize only a
   bare `.` head; every other spelling (`try . catch 1`, `if true then . else . end`,
   `. // 1`) fell to the null/bool catch-all, wrongly decided the step was not intact, and
   either refused (a single pattern) or retried a `?//` chain onto the *wrong* alternative,
   which `del`/`=` then wrote through — `del((. // 1) as {a:$v} ?// $v \| $v)` on
   `{"a":"s","c":"s"}` wrote `null`, deleting the whole document, where jq deletes one key.
   Closed by `resolves_to_register`, which re-derives the bare-`TrackedVar` arm's own
   `marker.value == *reg && frame.certifies(...)` check at every level it recurses through
   (not `is_identity_passthrough`'s weaker, deferred-certification guarantee — see that
   function's own doc comment) and requires the register be truthy for an `Alternative`
   *and* `is_raise_free_identity_passthrough(left)`, closing an `If`-on-`left`-with-an-
   empty-condition hole two review rounds found live (`del(.a \| ((if empty then . else .
   end) // {p:100,q:200}) as {p:$x} \| $x)` on `{"a":{"p":1,"q":2},"c":"keep"}` wrote
   `{"a":{"q":2},"c":"keep"}` without that extra guard).

   **Two residual, refuse-only gaps remain**, both pinned in
   `test_alternative_identity_passthrough_pattern_head_is_recognized_3119`:
   - a marker frozen off the register (#3120's carried register, not `trackable`) whose
     value happens to be truthy and value-equal to the register's own real node — jq answers
     (`path(. as $x \| 5 \| ($x // 1) as {a:$q} ?// $z \| $q)` on `{"a":1}` is `["a"]`), but a
     value-only check off-register cannot tell that real coincidence from an unrelated one
     (the exact #3129 class of bug), so this stays refuse-only;
   - an `if` whose branches don't *both* statically recognize, even when its condition is a
     constant that always takes the recognized one — `path(.a \| (if true then . else 5 end)
     as {b:$v} \| $v)` on `{"a":{"b":1}}` is jq's `["a","b"]`; the static rule cannot tell
     it from a truly arbitrary condition without evaluating it, the same refuse-only cost
     `test_identity_bind_position_traps_keep_refusing_2978`'s row 9 already pays for
     `identity_bind_position`'s sibling mechanism.
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

`?//`-alternatives reach the same walk per alternative since #2979 (item 2 above).

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
a plain `String` to `ObjectKey` (the same type object construction already used), and the
matcher resolves the key expression against the pattern's own current node via
`index_one_owned`, the same generic `.[EXPR]` operator used elsewhere (`nth`,
`(.a\|tostring)[$k]`) — so a computed key can resolve to a *string* (indexing an object) or a
*number* (indexing an array, with jq's own float-truncation and negative-index wraparound,
`path(.arr as {(-1):$q}\|$q)` on an array giving `[-1]`, the raw key, matching
`path(.arr[-1])`; a float-spelled key keeps its spelling as a path component, `[1.7]`, exactly
as `path(.[1.7])` does since #1088) exactly like `.[EXPR]` does, in both value and path
position, using the same "Cannot index `<type>` with `<type>`" wording jq's own `INDEX`
bytecode gives for any key, computed or literal — confirmed live in both positions, including
nested computed keys and through `\|=`/`del()`. (PR #2873 review caught and fixed an initial
version of this that unconditionally required a string key, wrongly rejecting a numeric key
against an array.)

**The matcher is jq's backtracking matcher** since
[#2872](https://github.com/rust-works/succinctly/issues/2872): one demand-driven,
continuation-passing walk (`walk_pattern_each`, `src/jq/eval.rs`) shared by value mode (`. as
PATTERN` in both evaluators, the owned-identity pipe, `reduce`/`foreach`'s own pattern) and
path mode (`path()`/`del()`/assignment targets and the path-mode folds, where each index step
also moves the register). It follows jq's compilation (`gen_object_matcher`/`gen_array_matcher`)
exactly: object entries in source order, array elements **right to left**, a computed key's
outputs produced one at a time and only when something backtracks for them, a later entry's key
re-run once per earlier binding, and the body run at the end of every completed path through
the matcher. Consequences, every one confirmed live against jq 1.7.1 and pinned in
`tests/jq_cli_tests.rs` (`test_pattern_computed_key_*_2872`):

- a multi-output key fans out one full body run per output in **every** position — `[path(.
  as {("a","b"):$q} \| $q)]` is `[["a"],["b"]]`, `(. as {("a","c"):$q} ?// {a:$q} \| $q) \|=
  "X"` on `{"a":{"b":1},"c":2}` writes both, `reduce . as {("a","b"):$q} ?// $r (0; .+$q)` is
  `3`, `[path(foreach .[] as {("c","d"):$q} (.; .; $q))]` walks every `(element, key)` pair
  with its own register; a zero-output key matches nothing and still wins a `?//` chain;
- the walk is **lazy**: `first(. as {("a", (input\|"b")):$q} \| $q)` never consumes an
  input, `. as {("a",("K"\|stderr\|"b")):$q} \| ("B\(EXPR)"\|stderr\|empty)` writes
  `B1KB2`, a `halt`/`error` behind a body error that retried the next `?//` alternative is
  never reached (`. as {("a", halt_error(7)):$q} ?// $z \| if $q==1 then error("boom") else
  ($q // $z) end` on `{"a":1}` prints `{"a":1}` and exits 0 — the eager design pinned exit 7
  here), `[. as {("a","b"):$q, ("K"\|stderr\|"a"):$r} \| [$q,$r]]` writes `KK`, and `. as
  [{("K1"\|stderr\|"a"):$x},{("K2"\|stderr\|"a"):$y}]` writes `K2K1`;
- `null` no longer short-circuits a pattern: every key runs against it (`null \| .[k]` is
  `null`), so `null \| [. as {("a","b"):$q} \| $q]` is `[null,null]` and `null \| . as
  {(error("E")):$q} \| $q` raises `E` (the pre-#2872 short-circuit answered `null`, and its
  doc comment's claim that jq did too was false);
- the `?//` retry rules apply per completed match, partway through the stream: a body error or
  `break` after k matches retries the next alternative and keeps what was already emitted; a key
  expression's own error or `break` (raised after the matches its earlier outputs completed)
  retries the same way, `halt` and an uncatchable error never; in a fold the next alternative
  resumes from the state the steps already run left (`[label $o \| reduce . as {("a", break
  $o):$q} ?// $r (0; .+1)]` is `[2]`); the resolver's own refusals in path position still never
  retry (#2979);
- the duplicate-name rule is one pass over visit order — a bare pattern keeps the **first**
  occurrence, a `?//` alternative the **last** — for every container kind and nesting depth,
  which is what jq's `bind_matcher`/preamble-slot compilation does (`{"a":[1,2],"b":3} \| . as
  {a:[$x,$x],b:$x} \| $x` is `2`, `3` under `?//`). #1366's per-container spelling ("object
  keeps first, array keeps last, both inverted under `?//`") was this rule as seen from index
  order.

One bound of succinctly's own: a single pattern (one `?//` alternative, all nesting levels)
holds at most 4096 computed keys (`MAX_PATTERN_COMPUTED_KEYS`, a clean parse error past it,
beside `MAX_PATTERN_DEPTH`'s 256 nesting levels from #1240). The matcher runs the rest of the
walk from inside each computed key's own generator callback, so every computed key on a match
path is one native frame of the evaluation stack -- a pattern with 100k of them overflowed the
CLI's 256 MB stack, a process abort rather than an answer, which is the one kind of divergence
ADR-0018 permits. A literal key costs no frame, so a pattern's *width* is unbounded (a million
literal entries walk fine, as before #2872); jq itself accepts any number of computed keys.

Before #2872 value mode built the whole cartesian product of every key's outputs eagerly, and
path position and both folds refused any key with other than exactly one output through an
ordinary *catchable* error — which `?//` and `try` read as "this alternative did not match" and
answered wrongly, a silently dropped write included (`(. as {("a","c"):$q} ?// {a:$q} \| $q)
\|= "X"` gave `{"a":"X","c":2}`). The claim recorded here at the time, that every unsupported
site "refuses cleanly, never a wrong value", was therefore false. `scripts/jq-pattern-fanout-fuzz.py`
is the randomised differential check over the whole alphabet (generators, `empty`, `error`,
`break`, `halt_error`, `stderr` in keys; nested patterns; `?//`; both folds; every path-mode
consumer), validated against the pre-#2872 binary, which it flags.

A related, narrower gap the fix surfaced and closed along the way: `map_subexprs`
(`src/jq/walk.rs`) — the shared tree-rewrite primitive `install_def_calls`/`bind_def` and
`substitute_var_impl` both build on — cloned a pattern's own `patterns` field verbatim in its
`Reduce`/`Foreach`/`AsPattern` arms rather than descending into it, a premise ("a pattern holds
only destructuring names, never an `Expr`") that held before #2677 and silently broke once a
computed key gave a pattern entry a real `Expr` to hold. Concretely, `def f: "a"; . as {(f):$q}
\| $q` wrongly raised "undefined function: f/0" even though `f` *is* defined, and an outer
`$var` referenced inside a computed key was left unsubstituted — both fixed by a new
`map_pattern_subexprs` helper. PR #2873 review found the identical gap in two more,
separate hand-written traversals `map_subexprs` does not cover: `substitute_func_param_impl`
(a `$`-style `def` parameter referenced inside a computed key was never substituted, `def
f($x): . as {($x): $y} \| $y; f("a")` wrongly raised "undefined variable: $x") and
`substitute_var_impl`'s own shadow-guarded `Reduce`/`Foreach`/`AsPattern` arms (when the
pattern's *own* binding shadows the outer variable for its body, the computed key — which
resolves *before* that shadow takes effect — must still see the outer value, `"a" as $x \| (.
as {($x): $x} \| $x)` wrongly raised the same "undefined variable" error). Both fixed the same
way, with `map_pattern_subexprs` wired into their own `Reduce`/`Foreach`/`AsPattern` arms.
Three siblings with the analogous gap — `any_subexpr`, `node_reads_ambient`, and
`resolve::check` (`src/jq/resolve.rs`) not descending into a computed key either — are,
confirmed live, in the safe direction only (a missed static-analysis optimization, or jq's own
compile-time "undefined function" check surfacing one stage later as a runtime error instead of
at parse time) and are left for the same #2872 follow-up's walker-invariant audit.

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

Seven residual divergences remain in this area (the first five predate
[#2732](https://github.com/rust-works/succinctly/issues/2732); the second was narrowed to its
consumer half by [#2694](https://github.com/rust-works/succinctly/issues/2694), re-tracked as
[#2908](https://github.com/rust-works/succinctly/issues/2908), and that half is now closed too;
#2732 closed a
separate, sixth bug in `fold_source_ambient`'s fork-0 arm — a `null`/`bool` document did not reestablish
against an equal-valued register, `path(reduce .a as $k (.b; .))` on `null` raised where jq
answers `["b"]` — and classified the two residuals appended below):

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

- ~~**A generator the resolver reaches only through an eager arm is still collected before
  its first element is folded.**~~ **Closed by
  [#2694](https://github.com/rust-works/succinctly/issues/2694).** `drive_fold_source` used to
  pull by demand only as far as `resolve_node_sink`'s lazy arms reached, and the four arms
  that had none materialized their own generator first: `resolve_leaf`'s general case, an
  `if`/`select` condition, an `as` source, and a fold nested inside the source. The nested
  fold was already covered by #2235's own `Reduce`/`Foreach` sink arms; the other three now
  stream through `resolve_leaf_sink`, `resolve_cond_fork_stream` and
  `resolve_bind_source_sink`. The leaf arm was the one with a correctness cost rather than
  only a side-effect one: `inputs` is a generator whose unconsumed documents stay readable, so
  collecting the source drained the stream — `path(first(foreach (inputs | select(.a)) as $i
  (.; .))), [inputs]` answered `[]` then `[]` where jq answers `[]` then
  `[{"a":2},{"a":3}]`. `path(reduce (if (.[]|stderr) then 1 else 2 end) as $i (.;
  error("u")))` on `[1,2,3]` now writes `1`, as jq does, where it wrote `123`.

  **Two parts of the streaming stay collecting on purpose, and are not gaps.** A
  `Keep::First` leaf (every ordinary `path()`/`=`/`|=`/`del()` entry) keeps the collecting
  form because its halt rule returns *no* prefix — it discards the one value a limit-1 sink
  had already taken, which streaming cannot take back — and loses nothing by it, since its
  limit is 1. #2042's two `as`-source witness routes keep it because each can still *decline*
  after resolving and fall back to by-value evaluation, another decision a streamed value
  cannot be taken back from; they run only on `is_pure_navigation`'s closed grammar, which
  has no side effects and no generator to interleave with.

  **The consumer side was a different mechanism under the same original bullet, and is
  closed by [#2908](https://github.com/rust-works/succinctly/issues/2908).** `path()` itself
  resolved every path before its own consumer saw one, so a bound *outside* it could only
  truncate a finished list. That was never fold-specific — a plain comma showed it just as
  well (`[limit(1; path((.a|stderr), (.b|stderr), (.c|stderr)))]` wrote three lines for jq's
  one) — and the `?//` row diverged the other way, jq writing twice where succinctly wrote
  once. `each_path_on_owned` now interleaves resolution and walking, so each resolved branch
  is walked and emitted before the next is asked for, which is jq's own order and what carries
  the consumer's stop back into the generator. Pure navigation fan-out (`path(.[])`,
  `path(.a[])`) always agreed — nothing in it is observable per element — and keeps #2061/
  #2168's no-materialization cursor walk untouched.

  **Two shapes still collected after that work, neither introduced by it, both tracked and
  now closed as [#2925](https://github.com/rust-works/succinctly/issues/2925).** A generator
  in *index* position closed with [#2267](https://github.com/rust-works/succinctly/issues/2267):
  `E[K]`/`E[S:T]` are native `resolve_node_sink` arms now, so a bound reaches the key and
  bound generators too (`[limit(1; path(.[("a"|stderr),("b"|stderr)]))]` writes `a`, matching
  jq, where it used to write `ab`). And a document the reindex bridge will not round-trip
  identically (a NaN spelling; a bare `Float` has been bridge-identity since #2902)
  now forwards demand across the bridge the same way, instead of collecting every path
  first: `[limit(1; path((.a|stderr),(.b|stderr)))]` writes `1` whether or not a 300-digit
  literal sits elsewhere in the document. #3025 removed the old 256-character
  `NumberLiteral` limit: the bridge now retains that source text, so long literals
  no longer select the non-identity route.
- **`recurse(f)`/`recurse(f; cond)` past its native stack budget finishes one node's own `f`
  before descending.**
  `resolve_recurse_sink` (#2235) streams each visited node to a bounded consumer as soon as
  it is popped, and defers `f`/`cond` for a node until its own delivery is accepted — so
  `path(limit(1; recurse(if (.|debug) < 3 then .+1 else empty end)))` now runs `debug` zero
  times in both jq and here, where the pre-#2235 collecting version ran it for every node up
  to `RECURSE_MAX_ITEMS` regardless of the bound.

  **The *value* evaluators had no such arm at all until
  [#2693](https://github.com/rust-works/succinctly/issues/2693)**, which is where the cost
  actually was: neither `eval_each` nor `eval_each_generic` handled the parameterised
  spellings, so both fell to the collecting route, walked to `RECURSE_MAX_ITEMS`, and ran `f`
  at every one of those nodes before a bounded consumer could truncate the finished list.
  `[limit(1; recurse((.a|debug), (.b|debug)))]` on `{"a":{"x":1},"b":{"y":2}}` wrote **20000**
  `debug` lines — 10000 visited nodes, two `f` outputs each — for a five-node document, where
  jq writes none. The walk is now one shared `each_recurse_walk` behind both the collecting
  builtins and the two lazy arms, and emits a node before running `f` on it, as jq's
  `def r: ., (f | r); r;` does. That also closed a halt leak the short-circuit tests had
  pinned: `path((0 | recurse(if . < 1 then .+1 else ("x"|halt_error(3)) end)) == 1)` exited 3
  with `x` on stderr where jq raises a path error, and its mirror wrote a stray `x`; with `f`
  unevaluated the `halt_error` is never reached at all.

  **Closed within a stack budget by
  [#2918](https://github.com/rust-works/succinctly/issues/2918).** jq descends into `f`'s first
  output's whole subtree before asking `f` for its second, so a bound stops *between* `f`'s
  outputs and an unbounded walk interleaves `f`'s side effects with the descent. Both walkers
  (`each_recurse_walk` for the two value evaluators, `resolve_recurse_sink` for path mode) now
  visit each output from inside `f`'s own sink, and `cond`'s the same way, so nothing has to be
  suspended: `[limit(2; recurse(.[]?|debug))]` on `[1,[2,[3]]]` writes one `debug` line, as in
  jq, and `[recurse(.[]?|debug)]` interleaves.

  That descent is native recursion, so it is budgeted by the stack it has actually spent,
  measured as it goes: at most 512 KiB, so that it stays safe for a library caller on an
  ordinary 2 MiB thread, and charged on the ambient frame depth `MAX_EVAL_FRAMES` guards `def`
  recursion with. **What remains** is past that budget: the rest of the subtree is walked with
  the old explicit stack, where `f` runs to completion per node and the residual above returns
  for those levels only. How deep the budget reaches depends on `f`'s shape — for
  `if . < N then .+1 elif … end`, 46 levels in release and 9 in debug — so a document nested
  deeper than a few dozen levels, or a synthetic chain, sees the old order below that depth;
  `recurse_queues_past_its_native_stack_budget_2918` pins it. Values are identical in both
  orders. Where `f` takes the reindex bridge (`.[]?` does), each native level also keeps its
  node's temporary document alive while its children run, so peak memory is bounded by the
  subtree sizes along the current path rather than one node's: flat on a 10 MB `users`
  document, +11% on a 10 MB `nested` one, +30% on a 200-deep object chain (`{"k":v,"i":i}`)
  whose leaf is 10,000 100-character strings (~1 MB; 1.0 GB → 1.3 GB), time unchanged. The overhead is a roughly
  fixed number of bytes from the native window's overlapping retention, so its *percentage*
  depends on how much of that fixed cost the rest of the document dilutes: on a second shape
  — nested single-element arrays with a fixed 5 MB leaf, at depths spanning the native/queued
  boundary — it falls from +30% at 20 levels (fully native) to +18% at 40 (the boundary),
  +13% at 100, +7% at 200 (mostly queued past the budget), as the queued (unchanged) portion
  of the walk grows.
  (#2908, once listed as sharing this class, turned out not to need it: its branches were
  already produced lazily and only the terminal was collecting them.)
  Bare `..`/`recurse`/
  `recurse_down`, which predates #2235 and was not part of that migration at all, had the
  same gap one level up (no `f`/`cond` to over-fire, but the same "collects the whole
  tree/queue before a bounded consumer sees the first branch" shape) — closed by
  [#2696](https://github.com/rust-works/succinctly/issues/2696)'s own sink,
  `resolve_recursive_descent_sink`, which streams each node as soon as it is popped with no
  parameterised `f` in the way at all, so there is nothing left for it to over-fire.
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
- **`reduce`'s mid-step register position is message-only wrong ([#2732](https://github.com/rust-works/succinctly/issues/2732)).**
  `resolve_reduce` checks every UPDATE step against the fold's persistent register, never a
  per-step one seeded from a navigating SOURCE element the way `resolve_foreach` already does
  — correctly, for the *final* re-entry check (`path(reduce (.[]) as $k (.; .))` on
  `{"a":[1,2]}` is `[]` in both, jq's own path-restoring `FORK`/`BACKTRACK` at the exit
  boundary). But jq's `gen_reduce` bytecode is `DUPN, source, …` with no `SUBEXP` around it,
  so *inside* a step the register genuinely sits where SOURCE left it — the same per-step
  position `foreach` already models. Every *outcome* agrees — exit code, whether a value is
  produced at all, whether a write goes through (never, on either side, for this shape) — and
  only the error message's own wording differs, including a value it happens to quote from
  wherever each side's own model of the register landed:
  ```
  $ echo '{"a":1}' | jq -c 'path(reduce .[] as $k (.; .a))'
  jq: error: Invalid path expression near attempt to access element "a" of {"a":1}
  $ echo '{"a":1}' | succinctly jq -c 'path(reduce .[] as $k (.; .a))'
  jq: error: Invalid path expression with result 1
  ```
  Wrapping the navigating step in `try`/`catch` does not change this into a genuine output
  divergence either — confirmed live that jq's own `try` here does not prevent `path()`'s
  outer check from raising too, just with the *caught* value quoted instead:
  `path(reduce .[] as $k (.; try .a catch "x"))` on `{"a":1}` is jq's `Invalid path expression
  with result "x"`, exit 5; succinctly's `Invalid path expression with result 1`, exit 5 — the
  identical message-wording-only pattern, not a new case where one side succeeds.
  Both exit 5. Modelling the mid-step position too needs a dual provenance per step ("at the
  per-step register" *and* "still identical to the persistent one") without regressing the
  `[]` case above — recorded rather than built, since the exit code and value already match
  and only wording differs.
- **`getpath` is transparent to the path register, and succinctly now models it
  ([#2896](https://github.com/rust-works/succinctly/issues/2896),
  [#2978](https://github.com/rust-works/succinctly/issues/2978)).** The mechanism recorded
  here as *hypothesized* is now confirmed behaviourally against jq 1.7.1, with a probe that
  isolates each half. jq's `f_getpath` is `_jq_path_append(jq, a, p, jv_getpath(a, p))`, so
  (a) on an input that is not the register it returns the value and leaves the register
  untouched, where an ordinary `.k` in the same slot raises; and (b) the value it returns is a
  pointer *into* its input, so it can itself *be* the register's node, at which point tracking
  silently resumes:
  ```
  $ echo '{"a":1,"b":2}' | jq -c 'path(.a as $y | .a | {b:9} | getpath(["b"]) | $y)'
  ["a"]                                      # (a) register survived the getpath stage
  $ echo '{"a":1,"b":2}' | jq -c 'path(.a as $y | .a | {b:9} | .b       | $y)'
  jq: error: ... element "b" of {"b":9}      # ordinary navigation raises instead
  $ echo '{"a":{"b":2}}' | jq -c 'path(. as $x | .a | $x | getpath(["a"]) | .b)'
  ["a","b"]                                  # (b) result IS the register's node
  ```
  `getpath` is the only *navigating* builtin with this property — swept `.b?`, `first(.b)`,
  `last(.b)`, `limit(1;.b)`, `[.b][0]` and `tojson|fromjson|.b` in the same slot, all raise.
  The scope recorded here was also wrong twice: the divergence was never `foreach`-specific
  (the plain pipe register and `reduce`'s per-step register diverge identically) and never
  read-only (`=`/`|=`/`del()` refused a documented jq write).

  succinctly models both halves as of #2896, composed over #2042's existing absolute-position
  witness rather than by value equality, so the originally filed repro now agrees in both
  directions:
  ```
  $ echo '{"a":{"b":2}}' | succinctly jq -c 'path(foreach .[] as $k (.; getpath(["a"]); .b))'
  ["a","b"]
  $ echo '{"a":{"b":2}}' | succinctly jq     'del(foreach .[] as $k (.; getpath(["a"]); .b))'
  {"a":{}}
  ```
  #2896 left one input without a position to compose from: a variable bound by an
  identity-passthrough source (`. as $x`, and every spelling `is_identity_passthrough`
  recognises), as in the `(b)` probe above, which was marked `Origin::Snapshot` — the #844
  value-equality witness, position-less by construction. #2978 closed that: a bind made inside
  a resolver invocation while the branch is trackable now carries `Origin::SnapshotAt`, the
  same value-equality witness plus the position `.` was frozen at (`Frame::at`, which by
  #2042's invariant *is* the register's position there — including below the invocation root,
  `path(.x | . as $v | .a | $v | getpath(["a"]) | .b)` is `["x","a","b"]`). `Frame::certifies`
  admits it exactly as it admits `Snapshot`, so no #844 shape narrowed; only the positional
  readers (`getpath`'s result composition, the register-preservation proof) see the position.
  A rebind from such a marker (`$x as $y`) inherits the *marker's* position, never the frame's,
  which is what keeps `path(. as $x | .a | ($x as $y | .c | $y | getpath(["c"]) | .b))`
  refusing as jq does. The acceptance oracle is `scripts/jq-bind-origin-oracle-sweep.sh`'s
  `identity-*` rows and `scripts/jq-bind-origin-fuzz.py`, whose alphabet gained a navigation
  prefix before the first bind (so `. as $v` is drawn below the root) and a nested rebind use.

  The same review closed a hole in `is_identity_passthrough`'s own `try A catch B` arm, on
  `main` before #2978: it took `A` to be raise-free, but an `if` inside `A` has an arbitrary
  condition, so `(try (if error("e") then . else . end) catch {"b":1}) as $x` bound the
  handler's fresh `{"b":1}` as a value-certified snapshot of `.` — and
  `del(.a | (try (if error("e") then . else . end) catch {"b":1}) as $x | .k | $x | .b)` on
  `{"a":{"k":{"b":1}}}` wrote `{"a":{"k":{}}}` where jq refuses. A `try` body must now be
  raise-free (the same grammar minus `if`) for the `try` to count as a passthrough; the same
  gate keeps `SnapshotAt` from being minted for it.

  **What still refuses** after #2978, each a refusal where jq answers, never a fabricated path:
  - a `try` whose body holds an `if` whose condition happens *not* to raise —
    `path(.a | (try (if true then . else . end) catch 1) as $x | .k | $x | getpath(["k"]) | .b)`,
    `["a","k","b"]` in jq. The raise-free gate above is static and cannot tell it from the
    raising twin, so the bind is a plain value. Note that when such a `$x` is then navigated
    *inside* a `try` (`((try (if .a then . else . end) catch 1) as $v | try ($v | .b?)) |= .`),
    the plain value's refusal is caught by that `try` exactly as jq's own path errors are, and
    the write jq performs (`"b":null`) is silently skipped rather than refused — the same
    static gate, surfacing through `try` instead of as an exit 5 (found by #3133's fuzz);
  - an `if` bind source whose arms sit at *different* positions —
    `path(. as $p | .a | (if true then $p else . end) as $x | $x | getpath(["a"]) | .b)`,
    `["a","b"]` in jq. `identity_bind_position` is static (the condition is not evaluated),
    so it proves a position only when both arms prove the same one; otherwise the bind stays a
    bare `Snapshot`;
  - a navigated source headed by such a marker (`$x.a as $w`): `resolve_bind_source_witness`
    re-roots only an `Origin::At` head, so this still binds by value;
  - a bind made in *value* mode, outside any resolver invocation, whose node is an ancestor of
    a later invocation's root: `. as $x | .a | path($x | getpath(["a"]) | .b)` is `["b"]` in jq
    (the `getpath` lands on the invocation's own root). Only a bind made inside the invocation
    has a `Frame` to take a position from; a value-mode bind has no invocation at all, so its
    marker stays a bare `Snapshot`. The same family as
    [#3037](https://github.com/rust-works/succinctly/issues/3037)'s value-mode navigated
    binds. (Wrapping the whole pipe in `path(...)` makes both refuse: the inner `path()`'s
    result is then not a path expression of the outer one.)

  **Also still refusing**: `getpath` as a fold *SOURCE*
  (`path(foreach getpath(["a"]) as $x (...))`), which is a separate position from the
  UPDATE/EXTRACT one this closed and is recorded in
  `scripts/jq-path-context-oracle-sweep.sh` as the `foreach_path` known divergence (#2388).

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
  the CLI.
- **#2202 (decided, not a bug): the rule this leaves for argument-vs-input ordering on an
  undecodable input.** `in(xs)`/`IN(s)` have the identical shape as `contains`/`inside` but
  evaluate their argument against the *decoded* input (`eval_owned_multi_keep_partial`/
  `eval_each_owned` take `&OwnedValue`), so they keep the eager early return rather than
  #1800's deferral: `in(error("boom"), {"a":1})` and `IN(error("boom"), {"a":1})` both raise
  the decode failure, not `boom`. Nothing about `in(xs)`'s desugar, `. as $x | xs | has($x)`,
  structurally *forces* this — `contains`'s own #1800 fix shows the same poisoned-value shape
  (fan the argument out over the raw cursor, consult a stored `to_owned` `Result` lazily inside
  the per-candidate body) would work here too, it just isn't built. What actually decides it is
  that there is no jq oracle for either ordering — every reference document containing an
  undecodable string is rejected at JSON-parse time, before any filter runs — so with today's
  eager `. as $x` (#1902), the eager order already agrees with `in(xs)`'s own desugar for
  strictly less work (ADR-0018 step 3); deferring `in`/`IN` the way `contains` defers would need
  `eval_as` to also defer materializing `$x`, or the desugar and the builtin would newly
  disagree with each other. This is a different question from the #2103/#2168/#2692 amendment
  above: those concern whether *navigating to a value* triggers its own validation depending on
  spelling, one value, one answer either way; here two *independent* computations (the ambient
  input's decode and the argument's own error) can each fail, and the question is only which
  failure is reported first when both are present — the same "earlier step wins" precedent
  `IN(s)`'s own #910/#932 fix and `builtin_contains`'s #1800 fix already establish, not a new
  per-spelling validation split. Like #1800's asymmetry above, the asymmetry between `in`/`IN`
  and `contains` is reachable through the library API only — the CLI's `eval_generic.rs` bridge
  materializes the input a layer earlier and raises the decode failure uniformly for all three
  builtins, `contains` included.
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
was fixed too). At the time, `pick`/`omit` had no native `eval_generic.rs` arm, so the "CLI unaffected" claim
did hold for them -- the reindex bridge they fell through to already fed them an
already-validated `OwnedValue` before this file's own fix, making that fix real for the library
API but largely redundant for `sjq`/`syq`. #3026 adds a jq-mode `pick` arm in the generic
evaluator; yq-mode `pick` and `omit` still use the older route.

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

**`paths`/`leaf_paths`'s nested recursion and `getpath`'s cursor walk --
closed by #2731.** `collect_paths_generic` now threads a `cursor:
Option<&V::Cursor>` down through every recursive call (the same shape
`to_owned_at_depth` already used, #2358), so `empty_fields_tail_gap_ok`/
`empty_elements_tail_gap_ok` can run for a `{,}`/`[,]` nested at any depth
inside a `paths`/`leaf_paths` walk, not only the outermost container the
dispatch site's own pre-check covered. Its array arm also switched from a
bare-value walk to `collect_cursors_checked` (review finding, not the
original draft), so a *non-empty* array with a mid-list or trailing stray
comma nested inside the walk raises too, matching what `.a[]` already did
on the same document -- a first pass that only added the zero-element
check left this half open. `getpath_walk_cursor`'s two child-miss exits
(`find_cursor` returning `Ok(None)`, an out-of-range `get_cursor`) run the
object/array-specific check directly against the container already in
scope before answering `null`, closing the gap `.b.x`-style navigation
already closed via #2594's `Expr::Field`/`Index` arms.

Still open, unchanged by #2594/#2731: the same shape reached through
`to_owned_at_depth`'s cursor-less top-level callers (#2262 above) -- no
cursor exists to give it at the true top level by construction, and (unlike
`paths`/`getpath`) there is no dispatch site further out that could hand
one down.

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

### Malformed text a filter never reads is never diagnosed (#3035)

The incremental model's far edge: when a filter never *reaches* the
malformed token, succinctly has nothing to fail on, where jq's atomic
whole-document parse fails regardless. The keyword validation itself is
exact (#3035 -- `nullx`/`nul` decode as errors and every path that reads
them exits 5), but a filter that answers without reading the token is
allowed its honest answer:

```
$ echo '[nul]'     | jq  -c 'type'        # (parses nothing) exit 5
$ echo '[nul]'     | sjq -c 'type'        # "array"            exit 0
$ echo '{"a":[nul]}' | jq  -c 'map(type)' # (parses nothing) exit 5
$ echo '{"a":[nul]}' | sjq -c 'map(type)' # ["array"]          exit 0
$ echo '{"b":nul}' | jq  '.a'             # (parses nothing) exit 5
$ echo '{"b":nul}' | sjq '.a'             # null               exit 0
```

`type` on a container does not descend into it, so the malformed child is
never decoded; `.a` reads only the `"a"` field. The moment the filter does
read the bad token -- `.[]|type`, `map(type)` on `[nul]`, `.b`, a bare
`.` -- succinctly raises with exit 5, matching jq. Scoped out of #3035 as
the same lazy-vs-atomic parse divergence the section above records for
partial output; a whole-input pre-validate (`--validate`) still rejects
all of these.

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
consistently, not a new one. Whether a truthiness-only read should validate at all is
settled by #2692's own ADR-0018 instance above ("a filter validates only what it
materializes" — truthiness is not a read that can meet a malformed byte, so the line
belongs there and has no remainder): #2669 only removed the route-dependence, it did not
newly decide this. A route that genuinely reads a member (`.a and true`) still raises.
Pinned by `test_boolean_routes_agree_on_a_malformed_document_2669`.

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

The **null**-target corner of that rule was the last piece, closed by
[#2853](https://github.com/rust-works/succinctly/issues/2853): jq resolves a non-numeric
bound over `null` into the path verbatim (`null | path(.["x":])` is
`[{"start":"x","end":null}]`, `?` or not; `path(.[1.5:"y"])` keeps a float spelling and a
raw bound in one descriptor) and leaves it to the write to refuse. succinctly used to raise
`Array/string slice indices must be integers` at *resolution*, which matched jq for every
write it refuses but was wrong for `path()` (jq reports the descriptor) and `del()` (jq
answers `null`). `Expr::Slice`'s bound keys now carry the raw value, and the refusal happens
at the write, where jq puts it — so `.["x":] = 5` still raises, `.["x":] -= 5` raises the
*subtraction* error from inside its own filter, and `.["x":] |= empty` no-ops to `null`
because jq's `_modify` falls back to `delpaths`, which never parses the descriptor.

The path-mode resolver used to drain each bound generator eagerly before slicing, so on an
array target `[1,2] | path(.[("x",(1|debug)):])` printed a DEBUG line jq never reaches (over
a `null` target both tools print it, since the pair resolves there rather than failing).
[#2267](https://github.com/rust-works/succinctly/issues/2267) closed that: `S`, `T` and `E`
are now driven as three nested demand-driven generators, exactly as jq's
`S as $s | T as $t | E | .[$s:$t]` desugaring pulls them, so `E` runs before the next `t` is
asked for and a pair completes before the next `s` is. `.[(0,1|debug):(2,3|debug)] = [9]`
writes DEBUG `0 2 3 1 2 3` here as on jq 1.7.1, and a pair whose `E` escapes stops the `T`
generator rather than letting a later `t`'s side effect fire. yq mode keeps the eager drain
— [#2351](https://github.com/rust-works/succinctly/issues/2351)'s rule discards every value a
bound produced once it escapes, which is only decidable after the generator has finished, and
real yq cannot express any of these shapes anyway (no `path()`, no `debug`, and `.[0:(1,2)]`
is "bad expression" on v4.53.3).

`resolve_index_expr` had the identical gap for a computed key's own generator and was
closed the same way: `path((.|debug("E"))[("a"|debug("ka")),("b"|debug("kb"))])` printed
`ka, kb, E, E` where jq prints `ka, E, kb, E`, and a key whose target escaped still
evaluated the *next* key, which jq never reaches.

Both are now native `resolve_node_sink` arms (#2267), which closed the last piece of the
same rule in the other direction: a *consumer's* demand reaches those generators too.
`[first(path((.|debug("E"))[((0|debug("s0")),(1|debug("s1"))):(2|debug("t"))]))]`
evaluated both `s`s here where jq 1.7.1 evaluates only `s0`; both now stop after the
first. The collecting forms have no callers left and are gone — `resolve_node` is the one
collector, and the cross-product refusal moved with it, from an up-front refusal of a
whole product to per-branch growth in `collect_resolved`. A consumer that keeps nothing
can no longer be refused at all, which is also jq's own behaviour (it has no such guard);
one that keeps everything is still refused, and a genuine OOM still arrives as a catchable
error rather than an abort, which is the property
[ADR-0018](../../adrs/adr-0018.md)'s "would take the host process down" exception was
granted for.

The write side followed: an assignment now applies each write as its path resolves, so the
first write that fails validation stops the path generator, exactly as jq's
`reduce path(paths) as $p (.; setpath($p; $v))` abandons the reduce and never asks for a
later path. `echo '[10,20,30]' | jq -c '(.|stderr)[(0,1):(2,3)] = 99'` wrote the document
to stderr four times here and once on jq 1.7.1; both write it once now, as do
`echo null | jq -c '(.|stderr)[(-1,-2)] = 1'` (the index spelling of the same gate). The
path generator resolves against the document as it was, not the one the writes accumulate
into — jq's `reduce` evaluates its source against the outer `.` — which is observable:
`{"a":"b","b":1} | .[(.a,.a)] = 5` is `{"a":"b","b":5}`, where resolving the second `.a`
against the written document would read back `5` and raise.

`|=` and the `op=`/`//=` family followed in
[#2974](https://github.com/rust-works/succinctly/issues/2974), through the same loop and
the same gate: jq lowers them to `_modify(paths; f)` and `$v as $tmp | _modify(paths; . op
$tmp)`, the same `reduce` over `path(paths)`. So `(.|stderr)[(0,1):(2,3)] |= 99` and
`(.|stderr)[(0,1)] += "x"` fire the target once now, as in jq, and so do `try`/`?`, a
halting or breaking filter, and a nested `|=`. Unlike `=`, this is visible without any
failure: the filter now runs between paths, so `.[(0,1)|debug("p")] |= debug("f")` writes
`p f p f` (it wrote `p p f f`), `.[(input,input)] |= input` reads its inputs in jq's order,
and `(.[] | select(.a > 0)) |= error("x")` on `[{"a":1},"s"]` raises the filter's `x` from
the first path rather than `Cannot index string with string "a"` from the second. A write
that stops the generator classifies its reason for any `?//` inside the path, as jq's own
backtracking does: an error or `break` retries the next alternative, a halt never does. Two
shapes keep the eager route: a filter reading `parent` (a succinctly extension with no jq
ordering to follow, which sees the document with every target already created), and
`sort_keys(f)`'s skip-absent pre-filter (yq only).

One jq 1.7.1 quirk is deliberately not reproduced: a `?//` retry that re-enters `_modify`
after a write already succeeded corrupts jq's `reduce` accumulator and fails with `Paths must
be specified as an array`, having fired the target three times
(`(. as $x ?// $y | (.|stderr)[(0,1)]) += "x"` on `[1,2]`). That is jq's VM stack, not a
rule, and it fires on *any* retry through `_modify`, even one whose writes all succeed.
succinctly keeps the first failed write's own error instead (`number (1) and string ("x")
cannot be added`, here after firing the target twice), which keeps jq's exit code. `=` is
different in jq, and succinctly follows it: `_assign`'s `reduce` carries on with the retried
alternative, so `(. as $x ?// $y | .[if $x == null then 0 else -5 end]) = 1` on `[1]` is `[1]`
(it raised `Out of bounds negative array index` here before #2974's review). A retry that
resolves no path at all is a jq quirk too, `null` for `=`, and is not reproduced.

That interleave has a price, and it is charged only where it buys something. The
streaming route keeps two documents where the eager one keeps one, so on a 1.5 MB /
200,000-element array `.[(0,1)] = 0` costs more peak RSS than the eager `.[$k] = 0`:
74.5 MB for the streaming form against 54.4 MB for the eager one on an Apple M4 Pro, and
66.8 MB against 51.5 MB on an AMD Ryzen 9 7950X (interleaved `/usr/bin/time -l`/`-v`,
five reps, minimum of each). Both pairs are post-#3000 — before `OwnedValue` shrank from
72 bytes to 32 they were 92.5 MB against 73.4 MB and 104.7 MB against 75.3 MB on the same
two machines. A path that provably resolves to **at most one path** stays on the eager
route and pays none of it — `.[$k] = 0` is the eager number above, as are a static path
and `del()`. `|=` and `op=` pay it under exactly the same gate, which matters more for
them, because `(.[] | select(...)) |= f` is a common idiom: see the #2974 measurements
below.

The count is the whole criterion, and purity is not part of it
([#2976](https://github.com/rust-works/succinctly/issues/2976)): the eager and streaming
routes reach their path through the same resolver call, so a side effect, an `input` read
or a raise on the way to the *only* path happens identically on both. The gate's first
form also demanded that reaching that path be inert, which charged the second document to
`.[length - 1] = 0`, `.[(.a | stderr)] = 0` and `(. | debug)[0] = 0` for nothing; those
are eager now. Two shapes are refused despite plausibly qualifying — `try`, whose count
depends on no other admitted shape emitting a value and then raising, and `reduce`, whose
`?//` pattern alternatives are a second fan-out axis — because establishing their count
means reasoning about something other than their own operands.

What survives is irreducible by any gate: `.[(0,1)] = 0` genuinely needs both documents.
jq pays nothing for the same separation because its values are refcounted, and since
[#2999](https://github.com/rust-works/succinctly/issues/2999) (ADR-0024) so does
succinctly: `OwnedValue`'s containers are `Rc`-backed and copied on write, so the second
document is a refcount bump and the write copies only the containers it goes through.
[#3000](https://github.com/rust-works/succinctly/issues/3000) had first taken `OwnedValue`
from 72 bytes to 32 without any sharing, which made both documents smaller but left two of
them. What sharing leaves is the *spine*: for an array of scalars the spine is the whole
array (a scalar has no sharing granularity, and a number keeps its own boxed spelling), so
the 200,000-int repro still pays one copy of it — 70.1 MB → 60.2 MB on the M4 Pro and
66.0 MB → 54.5 MB on the 7950X, against 46.9 MB and 43.1 MB for `.[$k] = 0`, a gap of
+28% and +26% where #3000 left +31% and +29%. For an array of *containers* the spine is
the array of handles and the gap closes: `.[(0,1)] = 0` on 10 MB of three-key objects went
from 879 MB to 544 MB on the M4 Pro (−38%) and 984 MB to 596 MB on the 7950X (−39%),
against 543 MB and 518 MB for the single-path `.[0].a = 1`. Sharing the scalars' strings
too (ADR-0024's option D) was expected to close the rest; built and measured in
[#3182](https://github.com/rust-works/succinctly/issues/3182), it did not -- the refcount
header on every element cost more than the spelling copies it removed (`.[(0,1)] = 0` on 2M
ints read +15% peak RSS, and strings +29%), and the option was rejected. The gap stands.

[#3009](https://github.com/rust-works/succinctly/issues/3009) stopped the CLI rebuilding an
owned result as a `lazy::JqValue` before printing it, and a write produces an owned document and
then prints it — so both sides of the `.[$k] = 0` / `.[(0,1)] = 0` comparison above carried that
rebuild. **The M4 Pro figures are stale, and the gap they are quoted for is understated.**
Re-measured on that same machine (interleaved, minimum of 9; the pre-change numbers reproduce
the figures above to within 0.2 MB, which is what says the two harnesses agree): `.[$k] = 0`
falls 46.8 MB → 35.5 MB (−24%) and `.[(0,1)] = 0` falls 60.4 MB → 55.1 MB (−9%). The two sides
do *not* move together — the eager route sheds far more, its whole result being one owned
document handed straight to the writer — so the gap widens from +29% to **+55%**. The argument
this paragraph makes is strengthened, not weakened; only its arithmetic is out of date.

On the 7950X the same pair moves neither side (−1.0% and +0.3%), so that machine's absolutes
above stand as written and its gap stays at +29%. The difference is the allocator, not the
change: the rebuild's free/alloc size mismatch left holes in libmalloc's size classes that
glibc's did not leave. Wall clock improved on both (~6% on the M4 Pro, ~9% on the 7950X).

What #2974 costs, measured on generated `users` documents (release, `cgu1+fat`, seven
interleaved repetitions of each binary, median wall time and maximum peak RSS, outputs
identical; the baseline is the commit #2974 branched from):

| machine            | size  | filter                                            | base             | #2974            |
|--------------------|-------|---------------------------------------------------|------------------|------------------|
| Apple M4 Pro       | 1 MB  | `(.users[] \| select(.age > 30)).score \|= . + 1` | 0.03 s, 34.3 MB  | 0.03 s, 39.6 MB  |
| Apple M4 Pro       | 10 MB | `(.users[] \| select(.age > 30)).score \|= . + 1` | 0.31 s, 254.3 MB | 0.32 s, 316.0 MB |
| Apple M4 Pro       | 10 MB | `(.users[] \| select(.age > 30)).score += 1`       | 0.30 s, 253.1 MB | 0.32 s, 312.0 MB |
| Apple M4 Pro       | 10 MB | `.users[(0,1)].score \|= 0`                        | 0.37 s, 255.0 MB | 0.38 s, 311.3 MB |
| Apple M4 Pro       | 10 MB | `.users[].score \|= . + 1` (static, control)      | 0.27 s, 198.6 MB | 0.27 s, 198.6 MB |
| AMD Ryzen 9 7950X  | 1 MB  | `(.users[] \| select(.age > 30)).score \|= . + 1` | 0.05 s, 32.3 MB  | 0.05 s, 38.8 MB  |
| AMD Ryzen 9 7950X  | 10 MB | `(.users[] \| select(.age > 30)).score \|= . + 1` | 0.51 s, 237.3 MB | 0.58 s, 305.7 MB |
| AMD Ryzen 9 7950X  | 10 MB | `(.users[] \| select(.age > 30)).score += 1`       | 0.51 s, 237.3 MB | 0.57 s, 300.6 MB |
| AMD Ryzen 9 7950X  | 10 MB | `.users[(0,1)].score \|= 0`                        | 0.59 s, 254.3 MB | 0.66 s, 322.2 MB |
| AMD Ryzen 9 7950X  | 10 MB | `.users[].score \|= . + 1` (static, control)      | 0.43 s, 169.2 MB | 0.43 s, 169.1 MB |

So +22 to +29% peak RSS at 10 MB on both machines, and +3 to +7% wall time on the M4 Pro
against +12 to +14% on the 7950X — the second document's clone, not the interleave. That
is paid for fidelity under ADR-0018's decision order, where performance only breaks a tie
between two faithful options. The eager route was not faithful here: it fires side
effects jq never fires and raises a different error. The same structural sharing
(#2999) that would retire `=`'s second document retires this one.

**Still open, tracked on #2267.** jq re-resolves an assignment's path once per
right-hand-side output (`_assign(paths; $value)` binds `$value` as the outer generator),
so `echo '{}' | jq -c '(.|stderr)[("a","b")] = (1,2)'` fires the target four times where
this fires it twice. It cannot be fixed while `collect_rhs_outputs` is eager --
re-resolving per output then fires the target for outputs a downstream consumer never
pulls, which is *worse*: it was implemented, measured at 87 regressions against 69 fixes
on a 5,000-shape sweep, and backed out. `first((.|debug("E"))["a"] = (1,2))` fires `E`
twice under it where jq fires it once, and with `input` in the path it changes stdout;
`first((.|stderr)["a"] = (1,2))` is pinned as a holdout so a re-attempt fails loudly. The
streaming write above is deliberately gated to a *single* RHS output for the same reason —
one output means one output document, so none of that class is reachable from it. The
`op=`/`//=` family shares both the rule and the gap (`(.|stderr)[(0,1)] += (1,"x")` fires
twice where jq fires three times, pinned as a #2974 holdout).

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
- **Closed: a call reached through an `include`d module or `~/.jq` now names the module's
  own file, line and source echo.** [#2951](https://github.com/rust-works/succinctly/issues/2951)
  gave every unresolved call an `origin` (which run it was written in); on top of that,
  [#2991](https://github.com/rust-works/succinctly/issues/2991) re-derives the module's
  own text (a second, cold-path-only read of the file `origin` resolves to) and the
  module's own call-site table, using the same `nth`-occurrence rule and the same
  lookup the main filter's own diagnostics already use. `... is not defined at
  /path/mymod.jq, line 1:` with the module's own source line echoed, byte-for-byte
  against jq.
- **Closed: a namespaced call (`ns::f`) no longer relies on a text search at all.**
  [#3010](https://github.com/rust-works/succinctly/issues/3010): `parse_namespaced_call`
  (`src/jq/parser.rs`) previously had no `call_sites.push`, unlike the plain-call path a
  few lines above it, so every namespaced-call diagnostic (main filter or module body)
  fell back to a plain identifier-boundary text search with no awareness of `#`-comments
  or string literals — a coincidental earlier occurrence of the same spelling inside
  either would win over the real call. `parse_namespaced_call` now pushes its own
  `CallSite` under the joined `namespace::name` spelling, so the diagnostic is a direct
  table lookup like any other call, the same fix #2085's own sibling gap would need for
  plain identifiers.
- **jq's trailing padding on the echoed source line is not reproduced exactly.** jq pads with
  a `%*s` whose width follows the failing node's start column for a simple undefined name but
  points elsewhere for an arity mismatch; succinctly reproduces the column rule. It is
  trailing whitespace either way.
- **Closed: whitespace around `::` in a namespaced call (`mymod :: func`) is
  now a syntax error, matching both reference tools.**
  [#3116](https://github.com/rust-works/succinctly/issues/3116) removed the
  `skip_ws()` calls `Parser::parse_func_call_or_error` and
  `parse_namespaced_call` (`src/jq/parser.rs`) applied around the `::` token:
  the token is now checked immediately after the namespace identifier, and the
  function name must follow the `::` immediately — with jq's whitespace
  tolerance before `(` (`map (.*2)`, `f (1)`) preserved on both sides. jq
  1.7.1 and yq v4.53.3 both treat `::` as a single non-whitespace-separated
  token (confirmed live: `unexpected ':'` / `lexer: invalid input text`,
  neither ever reaching a "not defined" diagnostic). Verified byte-for-byte
  against jq: `mymod::func` and `mymod::func (1)` keep jq's exact
  `.../N is not defined at <top-level>, line 1:` with source echo, and
  `mymod :: func`, `mymod:: func`, `mymod ::func`, `mymod  ::  func` all exit
  3 with `jq: 1 compile error`; yq mode exits 1 with a parse error. The
  adjacent form's yq-mode acceptance (`module not loaded`, #1473) is a
  pre-existing extension — yq keeps jq's whole language surface including the
  module system — and is unaffected.

Every builtin the pinned jq defines is implemented since #3042 (the libm family) and #3046
(`JOIN`, `format`, `input_filename`, ...). The roster captured from the pinned oracle
(`tests/data/jq-builtin-names.txt`, regenerated by `./scripts/sync-jq-builtin-names.sh`)
still backs the pass for a spelling the parser declines to lower (an arity the dedicated
parser does not take, a `def`-shadowed name), and
`test_every_pinned_jq_builtin_is_implemented_1473` sweeps it so that the "unimplemented"
set stays empty.

`succinctly yq` runs the same pass, keeping yq's uniform `Error: …` wording and exit 1. Real
yq has no `def` at all — its lexer rejects `def f: 42; f` outright — so succinctly's `def`
support there is an extension (ADR-0018 rule 5) rather than a behaviour with a reference to
match.

**"Regardless of whether reached" above means an unreached *branch* of executed code**
(`if false then f(1;2;3) else 1 end` still rejects `f/3`), **not a `def` nobody ever calls at
all** — [#2740](https://github.com/rust-works/succinctly/issues/2740) closed that residual
gap: `resolve_func_calls_all` first builds a call graph among every `def`'s body (keyed by
each body's own heap address, since neither pass restructures the tree between building the
graph and walking it for real) and only checks a body reachable from the program's own
top-level execution, exactly mirroring jq's substitution-based compiler, which only ever
compiles a `def` at the call site referencing it. `def h: nosuchfn; 1` now compiles (`h` is
declared but never mentioned anywhere), matching jq; `def h: nosuchfn; h` still rejects it,
and a def referenced only as an unused filter *argument* (`def use(f): 1; use(h)`) still
counts as reached, confirmed live: real jq's own argument-closure compilation needs that
reference to resolve independent of whether the callee's body ever invokes the parameter.

### The other three compile-error paths — shape now shared, wording still not (#2703)

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

**Closed for *which* module is named when *more than one* unresolvable declaration is spread
across the top-level `include`s and `import`s (#2857).** jq reports the last failing declaration
in true source order, regardless of kind or of how many resolvable declarations sit between
the failures (confirmed live: `include "AAA"; include "BBB"; include "CCC"; 1` names `CCC`;
interleaving kinds — `include "CCC"; import "AAA" as a; 1` names `AAA`, and reversing the two
names `CCC` — always whichever comes last in the source, not last within its own directive
kind). The loader now can: `Program`'s `Import`/`Include` each carry a shared `decl_index`
(the parser assigns every directive one slot in the combined source order as it scans), and
both `unqualified_def_names` and `process_program` give every directive its turn to load,
keeping whichever failure has the highest `decl_index` and reporting it after the loops —
verified byte-for-byte against jq 1.7.1 across the decide-by-kind, decide-by-position, and
mixed-success shapes. This closed #2703's recorded residual; the gap below is what the same
research surfaced as still open.

**Residual gap: a module's *own* dependencies resolve with the old first-failure
short-circuit, not the source-order rule.** The same `last failing directive` comparison now
handles every top-level `include`/`import`, but a directive *inside* a module (`module_dep_defs`)
still returns on its first failure, so a module with several unresolvable `include`s reports
the first rather than jq's last, and a failing `include` inside a module can shadow a
later-declared failing `import` in the same module. The `decl_index` machinery would apply
there unchanged; it was deliberately left out of scope for #2857 (whose repros are all
top-level) to keep the change reviewable, and jq itself falls back to per-module ordering
here anyway when the failure is inside a module that a *resolvable* top-level directive
ultimately pulls in.

**Closed for a syntax error in the main filter.** Route to the same shared reporter
(`report_syntax_error` in `jq_runner.rs`, factored out of the undefined-name path's inline
construction): `jq: error: … at <top-level>, line N:`, the echoed filter line, and the
trailer. The flat `jq: compile error: parse error at position N: …` is gone.

**Closed for a syntax error inside an `include`d module.** Same reporter, with the module's
*canonical absolute* resolved path in the `at …` label (`/private/tmp/…` — canonicalizing
`-L /tmp` matches jq's own naming), the module's echoed source line, a blank line before the
trailer (jq 1.7.1 leaves one here, but not after a *top-level* syntax error — reproduced
per-path), and the trailer. The pre-#2703 `jq: module error: parse error in module
'{path-as-given}': …` is gone.

**What remains open, all three narrower than the shape gap:**

- **Message wording is succinctly's own.** A syntax error reads `jq: error: unexpected end of
  input at …` / `jq: error: unexpected character ']', expected expression at …`, not jq's
  `syntax error, unexpected end of file (Unix shell quoting issues?)` /
  `syntax error, unexpected ']' (Unix shell quoting issues?)`. jq's phrasing — which token
  name, which "expecting …" list, and the `(Unix shell quoting issues?)` suffix — is a
  function of its bison parse state, and a single `ParseError` reason cannot reproduce it:
  `1 +` and `."` are both "end of input" to our parser, but jq distinguishes them, appending
  a `QQSTRING_TEXT or …` expect-list to the latter. Reproducing it needs reading jq's C
  parser source (`parser.y`/`scanner.l`) or a much wider oracle sweep than #2703 did. One
  intended exception: the `$__loc__`-as-binding-name rejection (#3029) replicates jq's wording
  for those productions exactly — `syntax error, unexpected $__loc__, expecting IDENT or
  BINDING` for a `def` `$`-param and `… expecting BINDING or '[' or '{'` for `as`/`reduce`/
  `foreach`/destructuring/`?//`, bare `unexpected $__loc__` for the `{$__loc__}`/`{$__loc__:
  Pattern}` shorthand — because it is a single, closed production family whose bison messages
  were captured live from jq 1.7.1 rather than guessed; it still omits the
  `(Unix shell quoting issues?)` suffix, as every other error here does.
- **The echoed line's trailing padding is the column rule, not a fixed formula.** jq's own
  `locfile_locate` `%*s` width is not formula-derived (probing `1 +`, `[1,]`, `def f: ;`,
  `1 2`, `if then`, `?` gives no consistent rule); the reporter points at the error column
  instead, so where the two differ it is only in trailing whitespace.
- **jq can report *multiple* compile errors from one malformed filter** (e.g. `if 1 then`
  → 2 errors); succinctly's parser returns a single `Result<_, ParseError>` and has no
  architecture for accumulating more than one — a separate, larger gap than the wording.

All four paths now share the same `jq: error:` prefix, the same `jq: N compile error(s)`
trailer discriminator, and exit 3, so a script that greps or counts `compile error` to tell
a compile failure from a runtime one gets a consistent answer on every one of them.
Neither remaining gap corrupts output or crashes — the two conditions that would otherwise
make this an accepted ADR-0018 divergence rather than an open gap.

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
  `MAX_EVAL_FRAMES` (`src/jq/eval.rs`). Each `$`-style parameter adds a frame
  of its own: since [#3149](https://github.com/rust-works/succinctly/issues/3149)
  it binds through a real `x as $x` around the body, as jq's desugaring does, and
  that binding holds ~2.9 KB of native stack per level. The ceiling is
  therefore `MAX_EVAL_FRAMES / (b + k)` levels for a body charging `b` frames
  a level with `k` `$` parameters, so it falls with every `$` parameter the
  signature adds. A thin `def sum_to($n)` (`b` = 3) stops at 10,000 levels,
  not 13,333. `def r($p1;$p2;$p3;$p4): if $p1 == 0 then
  0 else r($p1-1;$p2;$p3;$p4) end` (`b` = 2) stops at 6,666, and the same
  shape with 16 `$` parameters at 2,222. jq answers all of these. Left
  uncharged, three parameters overflowed the stack below the guard; charged,
  the margin to the real crash floor stays 2.1-2.9x from one to eight.
  Since [#3262](https://github.com/rust-works/succinctly/issues/3262)
  ([ADR-0025](../../adrs/adr-0025.md)) a second limit sits beside the frame
  count: the evaluation thread registers its stack, and a recursion refuses once
  it has used half of it, whatever its shape. The two differ most for a *bare*
  parameter. It binds by name, so each level's argument wraps the previous
  level's (`n - 1` over the last `n - 1`), and reading it evaluates that whole
  chain natively on top of the live levels -- stack no structural count sees.
  Measured on an Apple M4 Pro (release, 256 MB), a thin `def f(n): ... f(n -
  1)` stops at ~10,000 levels, a bare `def sum_to(n)` at ~9,200, and a chain
  read inside `path()` or `|=` at ~11,600; each is at the frame count's limit
  or the stack floor, whichever comes first, and each is at least 2x short of
  where the stack really runs out. jq answers all of these. The steepest
  divergence is a chain link that is not plain arithmetic (`f(n - 1 | .)`), a
  nested `def` closing over the parameter (`def h: n - 1; ... f(h)`), or a
  helper `def` in the argument (`f(g(n))`): each refuses at ~170-220 levels,
  where jq runs 100,000 in 22-42 MB. Such a link goes through the demand-driven
  evaluator, whose native stack per link is what the self-recursive generator
  bullet below describes, and each level's continuation runs on top of the
  previous level's live chain, so the stack grows with the *square* of the
  depth: 4x the stack buys 2x the levels. No stack size closes that gap; a
  bounded-stack path for such links would
  ([#3287](https://github.com/rust-works/succinctly/issues/3287)). Before #3262
  all three aborted the process at ~240-330 levels with the guard on. Tree
  recursion that combines two calls with `+` or `as` (`fib(n - 1) + fib(n -
  2)`) runs each right-hand call inside its left sibling's output sink, so its
  stack grows with the number of *calls*, not the depth: `fib(20)` (21,891
  calls) refuses, `fib(21)` aborted the process before #3262, and jq answers
  both in 2 MB. The zero-parameter spelling (`def fib: ... (. - 1 | fib) + (.
  - 2 | fib)`) holds more per call and refuses from `18 | fib`. `[fib(n - 1), fib(n - 2)] | add` returns before continuing and
  runs `fib(24)`
  ([#3296](https://github.com/rust-works/succinctly/issues/3296)).
- **A recursively-built value can exceed `MAX_VALUE_TREE_DEPTH` (384) where jq has no such
  limit.** `def deep(m): if m == 0 then . else [[…]]deep(m-1)[[…]] end; deep(60)` builds
  1,200 levels; jq prints it, succinctly reports `nesting depth exceeds limit of 384` and
  exits 5. Before #1371 this shape could not recurse far enough to reach the ceiling at
  all. Between f2789080a (2026-08-29) and #3261, the guard's most commonly reached call
  site -- `reduce`/`foreach`'s own per-iteration reindex bridge
  (`OwnedValue::to_json_for_reindex`), which every ordinary evaluation of this shape
  actually hits -- instead panicked uncaught (exit 101, no clean diagnostic), contradicting
  this very entry; #3261 fixed the reindex bridge to pre-check depth and report a clean,
  **uncatchable** error instead (`EvalError::resource_limit`, the same #2132 machinery
  every other succinctly-only cap uses -- this constant has no jq counterpart, so a
  `try`/`catch`/`?` written against jq semantics cannot have meant "accept a truncated
  value here either"; a `try (reduce range(400) as $i (null; [.])) catch "x"` still exits 5
  with the diagnostic, not `"x"`). Two call sites still panic uncaught rather than exiting
  5: `path()`'s non-cursor-native walk (`eval.rs`'s `walk_path`/`step_into`, reached when a
  trailing `Expr::Slice` routes around the cursor-native evaluator, #2061), and
  `succinctly yq`'s own YAML-emission pipeline (`yq_runner.rs`'s `emit_yaml_value_at_depth`
  and four sibling `assert_value_tree_depth` call sites, reached by a write (`=`/`|=`/`+=`)
  whose right-hand side constructs a deep value directly rather than through the reindex
  bridge). Both hit the identical guard and are deliberately left unfixed here, tracked
  separately as #3275 (`path()`) and #3278 (yq emission) -- see
  `test_path_non_cursor_native_deep_static_chain_panics_cleanly_not_stack_overflow_2058`.
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

Recursion through a bare parameter is quadratic in time in both tools — a call-by-name
parameter is re-evaluated at each use, so reading one at depth `d` costs `O(d)`. A
`$`-style parameter is bound to a value once per call (#3149), so recursion through one is
linear. Measured interleaved on one machine,
`sum_to(8000)` is 10.8 s here against jq's 4.3 s: same complexity, ~2.5x constant.

`MAX_EVAL_FRAMES`'s ceiling is calibrated against the 256 MB (release) / 2 GB (debug)
stack the CLI reserves for evaluation (`EVAL_STACK_SIZE`, `src/bin/succinctly/main.rs`),
and the CLI registers that stack for ADR-0025's floor. A library caller invoking
`succinctly::jq::eval` directly gets neither guarantee on its own: the same recursion that
errors cleanly under the CLI can abort the process with a native stack overflow on an
ordinary (e.g. default 8 MB) thread. Wrapping the evaluation in
`succinctly::jq::with_stack_budget(bytes_available, || ...)` arms the floor for that thread,
so a recursion too deep for it refuses instead; running recursive `def`s at any real depth
still needs a thread reserved at a comparable size.

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

## `skip` with a generator count

`skip(n; f)` is a succinctly extension relative to the pinned jq 1.7.1,
which rejects `skip/2` as undefined. Its count is still evaluated as a single
value: `[skip((0,1); 10,20,30)]` raises `expected number, got null` instead
of running the body once per count. Captured on the pre-#2863 checkout;
accepting the equivalent bare-comma spelling in #2863 does not change this
existing evaluator limitation. `builtin_skip` needs count fan-out separately
from the parser change; tracked as [#2934](https://github.com/rust-works/succinctly/issues/2934).

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

**`foreach`'s UPDATE and INIT no longer diverge either — #2668 closed both.** `fold_step_each`
(`src/jq/eval.rs`) replaced `fold_step_via_accumulator_or_fork`: a sink-shaped helper delivering
each UPDATE output to a callback as it is produced, instead of collecting them into a `Vec`
first. In `try_foreach_step_alternatives`, EXTRACT (or the implicit identity push) now runs from
*inside* that callback, so a consumer's `Demand::Stop` reaches UPDATE's own generator — and any
`?//` bind inside it — before it produces anything further. `try_reduce_step_alternatives` wraps
the same helper in a collecting-last sink, since `reduce`'s own UPDATE has no per-step visible
output for a consumer to stop after (confirmed unaffected both before and after this change) —
sharing one dispatch with `foreach`'s fast path is the only reason it changed at all.

INIT's own fan-out (#534: each INIT output is an independent run over the source) still has to
run before the source is ever pulled (#2440), but no longer needs a `Vec` to do it: `foreach_forks`
itself takes a `ForeachInitDrive` closure (the same shape as its existing `ForeachSourceDrive`,
one level further out) instead of a pre-collected `init_values`/`init_control` pair, and the whole
fork loop is now `drive_init`'s own per-fork callback — `#2440`'s "zero INIT outputs ⇒ source
never pulled" falls out structurally (the source is only ever driven from *inside* that callback),
rather than needing its own `is_empty()` guard. `each_foreach`/`each_foreach_generic` and both
eager entry points build `drive_init` from `eval_each`/`eval_each_generic`, exactly mirroring how
they already build `drive_source`. Confirmed live under both wrappers:

| filter                                                                     | jq 1.7.1 and `succinctly jq` |
|----------------------------------------------------------------------------|-------------------------------|
| `[first(foreach (1) as $v (0; . + (1 as $x ?// $y \| 1)))]` (UPDATE)       | `[1,1]`                       |
| `[isempty(foreach (1) as $v (0; . + (1 as $x ?// $y \| 1)))]`              | `[false,false]`               |
| `[first(foreach (1) as $v ((1 as $x ?// $y \| 1); .+1; .))]` (INIT)        | `[2,2]`                       |
| `[isempty(foreach (1) as $v ((1 as $x ?// $y \| 1); .+1; .))]`             | `[false,false]`               |
| `first(foreach (1) as $x (0; (.+1, ("U"\|stderr)); .))` (stderr in UPDATE) | no write                      |
| `first(foreach (1) as $x ((0, ("I"\|stderr)); .+1))` (stderr in INIT)      | no write                      |

Pinned in `test_nested_short_circuit_consumer_hides_the_stop_2180` and
`test_short_circuit_side_effect_shapes_already_match_jq_820` (`tests/jq_cli_tests.rs`), and both
positions are back in `scripts/jq-alt-retry-oracle-sweep.sh`'s `W_ENTRIES`.

**~~`reduce`'s own INIT has the identical bug and is *not* fixed by #2668~~ — closed by
[#2899](https://github.com/rust-works/succinctly/issues/2899).**
`[first(reduce (1) as $v ((1 as $x ?// $y \| 1); . + 1))]` is jq's `[2,2]` and was
`succinctly jq`'s `[2]`, the same class as `foreach`'s own INIT row above (`reduce`'s own UPDATE
was never affected — its construct has no per-step visible output, so #2668's UPDATE fix already
covered it by sharing `fold_step_each`).

The diagnosis when this was filed was right and is worth keeping: unlike `foreach`, `reduce` had
no demand-forwarding dispatch arm at all, in either evaluator, so reshaping its core alone could
not have changed `first(reduce(...))`'s answer — nothing would ever have called it through a live
sink. #2899 therefore did both halves: `reduce_forks` replaces `eval_reduce_with_values` as
`foreach_forks`' structural twin, and `each_reduce`/`each_reduce_generic` are the arms that give a
consumer's stop somewhere to land, gated on `!streams_unbounded` exactly as `foreach`'s are.

What that diagnosis did *not* predict is the second divergence the reshape fixed: the collecting
version pulled SOURCE once and reused its values across every INIT fork, where jq re-drives it per
fork — `[reduce ("s"\|stderr) as $x ((0,1); .)]` writes `ss` in jq and wrote `s` here. See the
INIT-fork re-entry entry below, whose own description of `reduce` this changed.

A plain (non-`?//`) reduce source still cannot be stopped early by either tool:
`first(reduce (1,2,("X"\|stderr),4) as $x (0; .+$x))` writes `X` in jq and here, because `reduce`
emits only its final accumulator and so has to exhaust the source to produce anything at all.

**~~`path(foreach(...))`'s own INIT is now inconsistent with value-mode `foreach`~~ — closed by
[#2903](https://github.com/rust-works/succinctly/issues/2903).** `resolve_foreach`
(`src/jq/eval.rs`), the path-mode evaluator reached from `path(foreach(...))` and assignment
targets, has its own separate fold loop (`FoldRegister`/`drive_fold_source`) and was not touched
by #2668 — an explicitly anticipated risk in #2668's own triage plan, which checked it does not
call `foreach_forks` and left it alone on that basis. Before #2668 both evaluators collected INIT
eagerly and so agreed with each other; #2668 fixed only the value-mode one, so path-mode
*disagreed with value-mode* as well as with jq:
`echo '{"a":1}' | jq -c 'first(path(foreach (1) as $i ((.a, ("I"|stderr)); .; .)))'` writes nothing
and answers `["a"]`, where `succinctly jq` wrote `I` first.

#2903 resolves INIT through `resolve_node_sink`, running each fork's whole fold inside that sink
before the next INIT output is asked for. The per-fork body is unchanged — it already lived in a
closure driven by `drive_fold_source` — so only the *outer* loop moved. **`resolve_reduce` was
fixed alongside it**: the same loop in the adjacent function, diverging identically
(`first(path(reduce (1) as $i ((.a, ("I"|stderr)); .)))` wrote `I` too), which #2903's own text did
not name. Fixing one and not the other would have left the two path-mode folds inconsistent with
each other — the very complaint #2903 makes about path versus value mode. Pinned in
`path_mode_fold_resolves_init_by_demand_2903` (`tests/jq_cli_tests.rs`), with the moved rows in
`test_short_circuit_side_effect_shapes_already_match_jq_820`.

Value-mode `reduce`'s INIT (#2899 above) is untouched and still collects — it needs a native
`Expr::Reduce` dispatch arm before a stop has anywhere to land, which is a different change.

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
jointly: **0 stderr mismatches and 0 stdout supersets** over 4,083 generated streams per
seed.

Values and diagnostics come from one walk over jq's own raw bytes, and UTF-8 substitution
is applied afterwards to each value's own bytes, so invalid UTF-8 anywhere in the stream
can't move a record boundary. Before
[#3247](https://github.com/rust-works/succinctly/issues/3247), values came from a second walk
over the substituted stream. Outside a string, jq's short-tail rule could fold an invalid
lead byte *and* the RS or whitespace after it into one U+FFFD, so
`printf '\xe0\x1e"a"\n' | succinctly jq --seq -c .` printed nothing where jq prints `"a"`.

One `--seq` divergence remains, an artifact of jq's 4096-byte `fgets` *line reader* rather
than its parser, and unreachable from a newline-free chunk:

* `jq_util_input_read_more` measures each chunk with `strlen`, so a NUL byte truncates the
  input — only in a chunk holding no newline. `printf '\x1eA+\x008e'` reports at column 3
  in real jq (which stopped at the NUL) and column 6 here.

Running the sweep with `--expect-artifacts` puts this shape back -- attributable, and the
only one left. The `--seq` reader's own non-slurp end-of-stream rule, described just below,
no longer needs that flag: the corpus generates it unconditionally now that it is modeled.

### jq's stream can end silently, mid-buffer, with nothing to explain it (#2998)

Two shapes that used to be recorded here as unattributed "`fgets` line-reader artifacts"
turned out to be the *same* rule, once traced into jq's own C source rather than inferred
from output alone: real jq's non-slurp `--seq` driver
(`jq_util_input_next_input`, `src/util.c`) tracks "was the buffer I'm holding just
refilled" in a local variable, reset on every call. `jv_parser_next` returns a
message-less invalid — no value, no error — exactly once: when an RS byte arrives with
nothing pending (an empty record). If that is the *first* thing scanned from the specific
call that just performed the terminal `fgets` refill, the driver's loop condition reads it
as "no more input" and exits **without ever setting `p->eof`** — so nothing after that RS
is scanned, and there is no `at EOF` diagnostic for whatever was abandoned. Any later call
reusing the same (already-final) buffer has no fresh refill to reset that local variable,
so it keeps looping internally instead, absorbing any number of further empty records
without dropping anything:

```
$ printf '\x1e\x1etrue'     | jq --seq -c '.'   # (nothing at all, exit 0)
$ printf '\x1e1\n\x1etrue'  | jq --seq -c '.'   # 1        -- the earlier record already
                                                #             leaves the parser in the same
                                                #             state an empty one would
$ printf '\x1e1 \x1etrue'   | jq --seq -c '.'   # 1, true  -- one buffer, no newline: the
                                                #             first event is a *value*, so
                                                #             the rest parses normally
```

Real jq's own `fgets` chunks at a newline *or* after 4095 bytes, whichever comes first, so
the drop is observable by padding alone: `\x1e1\n\x1e` + 4093 spaces + `\x1etrue` keeps
`true`, and one byte more of padding drops it. `succinctly jq` now matches all of this,
modeled in [`jq_seq_reader`](../../../src/bin/succinctly/jq_seq_reader.rs)'s
`final_buffer_start`/`drops_at_first_empty_record`. It lives in the same raw-byte walk
that produces the values, which simply stops at the drop point (#3247). `-s` is unaffected (`has_more` keeps its own loop
going) -- the warning this same drop would otherwise suppress still fires there:

```
$ printf '\x1e\x1etrue' | jq --seq -s -c '.'
jq: ignoring parse error: Unfinished string at EOF at line 2, column 6
[1]
```

This is also what an earlier draft of this page recorded, without the mechanism, as "a
separate, pre-existing rule" on multi-record streams -- the trigger it named ("once jq's
reader has seen one newline, a later record must itself be newline-terminated to be
emitted") was an *effect* of this same driver rule, not an independent one:

```
$ printf '\x1e"a"\x1e"b"'     | jq --seq -c '.'   # "a" and "b" -- one buffer, no newline
$ printf '\x1e"a"\n\x1e"b"'   | jq --seq -c '.'   # "a" only    -- second RS is the first
                                                  #                event of the final buffer
$ printf '\x1e"a"\n\x1e"b"\n' | jq --seq -c '.'   # "a" and "b" -- trailing newline empties
                                                  #                the final buffer entirely
```

All three now match (`succinctly jq` reproduces every row above, including `-s`); before
#2998 `succinctly jq` also printed `"b"` on the middle row.

### `--seq -s` keeps its EOF location when the final byte completes a value (#3003)

The same per-call local decides whether a runtime error under `--seq -s` can still name a
file and line. #2947 established the rule: `<unknown>` is printed only when `read_more`
closes a stream already at `feof`, which jq is made to do by its parser returning an *error*
from the stream's final `fgets` buffer — the one outcome that `return`s early and makes
`main` call back in. What #2947's model missed is that the EOF branch is not always reached
in that call: `jv_parser_next` returns the moment `scan()` completes a top-level value, and
if that happens on the final buffer's **last byte** the buffer is fully consumed
(`has_more == 0`), the loop exits, and the slurped array is dispatched with the filename
intact. The `Unfinished JSON term at EOF` for whatever that byte opened is only detected by
the *next* `next_input` call, after the filter has already run. A number or keyword is
completed by the byte *after* it; a string or container completes on its own last byte, so
the opener after it is scanned in the same call and the position is lost as before:

```
$ printf '\x1e1{'          | jq --seq -s -c 'error("x")'   # jq: error (at <stdin>:0): x
$ printf '\x1etrue"'       | jq --seq -s -c 'error("x")'   # jq: error (at <stdin>:0): x
$ printf '\x1e"a"{'        | jq --seq -s -c 'error("x")'   # jq: error (at <unknown>): x
$ printf '\x1e1{ '         | jq --seq -s -c 'error("x")'   # jq: error (at <unknown>): x
$ printf '\x1e[0,]\x1e1{'  | jq --seq -s -c 'error("x")'   # jq: error (at <unknown>): x  (error in the final buffer)
$ printf '\x1e1}\n\x1e2{'  | jq --seq -s -c 'error("x")'   # jq: error (at <stdin>:1): x  (error in an earlier one)
```

`succinctly jq` now matches every row (before #3003 the first two and the last answered
`<unknown>`), including jq's 4095-byte `fgets` boundary — `\x1e` + spaces + `1{` keeps the
position at 4094 and 4096 bytes and loses it at 4095, where `fgets` fills its buffer exactly
and an *empty* final buffer follows — and across files, where the final buffer is the last
file's own last chunk (`\x1e1` in one file and `{` in the next keeps `second:0`). The rule is
[`jq_seq_reader::SeqStreamWalk::slurp_position_lost`](../../../src/bin/succinctly/jq_seq_reader.rs),
read off the same walk that produces the warnings; `scripts/jq-seq-oracle-sweep.py
--slurp-location` sweeps it.

The warning on these streams belongs to jq's call *after* the one that dispatched the array,
and succinctly reproduces that too
([#3201](https://github.com/rust-works/succinctly/issues/3201)). The walk holds that one
warning back, and it follows the array the way a plain JSON stream's trailing parse error
does (#2961):

- a runtime error prints first and the `ignoring parse error` line second;
- `halt` in the filter suppresses the warning entirely;
- `input`/`inputs` in the filter receive it as a runtime error instead
  (`jq: error (at <unknown>): Unfinished JSON term at EOF ...`, exit 5), and
  `try input catch .` yields the message.

Every other `--seq -s` warning is reported in the reading call, before the filter runs, in
jq as here. Without `-s`, the interleaving of warnings with evaluation is still not
reproduced; see the ordering note below.

A **malformed** BOM — a byte sequence that begins one and then contradicts it, such as
`\xef\xbb` — is *not* in that category and is matched exactly. jq consumes the bytes that did
match without counting them as columns, and then re-runs `parser_reset` at the top of every
read, which leaves it parsing the bytes before the first RS instead of discarding them and
wipes its state after every value, at every `fgets` refill (after a newline, or after a full
4095-byte chunk with no newline,
[#3200](https://github.com/rust-works/succinctly/issues/3200)), and at an empty final buffer:

```
$ printf '\xef\xbb1 2' | jq --seq -c '.'   # Potentially truncated top-level numeric value at EOF at line 1, column 3
                                          # -- not the abandoned-text template, despite no RS byte anywhere
```

`succinctly jq` reproduces this on both stderr and stdout, including the pre-RS `1` jq
reads. Both come from the same walk over the raw bytes. A walk over the UTF-8-substituted
stream can't decide the BOM: a raw `\xef\xbb` is itself invalid UTF-8 and becomes a U+FFFD
of a different width ([#3199](https://github.com/rust-works/succinctly/issues/3199)), and a
U+FFFD substituted for any invalid lead byte starts with `EF BF`, which reads as a
malformed BOM of its own ([#3195](https://github.com/rust-works/succinctly/issues/3195)).

Real-time interleaving of the warnings against stdout is also not reproduced: succinctly
materializes `--seq` input before evaluating, so all warnings precede all values (apart from
`-s`'s deferred warning above). jq's default block-buffered stdout produces the same ordering,
but `jq --unbuffered` does not. Without `-s` the same cause shows on stderr too. jq prints each
warning after evaluating the values read before it, so a runtime error on an earlier value
comes first, and `halt` suppresses the warnings after it. jq's `input`/`inputs` read each
warning as an error, and under `-n` that means every one of them, `-n -s` included: there only
the deferred warning above reaches `input` here, while jq's `input` also raises a warning from
the reading call (`printf '\x1e1 {' | jq --seq -s -n 'input'` fails with `Unfinished JSON
term at EOF`, exit 5). succinctly prints all the others up front, before any of that can
happen -- or, under `-n`, not at all.

The differential guard remains deliberately asymmetric: **0 stdout supersets** across the
randomized corpus. A fabricated value is a correctness failure even where an unrelated jq
line-reader artifact still leaves succinctly with extra output.

That is a statement about malformed-record handling, not a guarantee about `--seq` as a
whole -- see "jq's stream can end silently, mid-buffer, with nothing to explain it" above
for the multi-record, non-slurp rule that also bears on stdout's contents (#2998), now
matched rather than diverging.

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

[#2575](https://github.com/rust-works/succinctly/issues/2575) widened the rule once more, to
array *construction*: `[.[]]`, `[.[] | select(f)]` and the sibling shapes `Expr::Array`'s
generic-evaluator arm can now answer straight from a `LazySeq` (no `map` stage needed, unlike
#1687's own reordering-builtin route into the same type) instead of materializing an
`OwnedValue::Array` up front. `[.[]] | length` and `[.[] | select(.)] | length` on
`{"a":"\ud800","d":5}` now answer `2` where they used to raise — `length` counts without
converting anything, the same "count-and-discard" fast path `map`'s own `Length` arm has
always had. Recorded on the same footing, for the same three reasons #2103's own list gives:
the agreement being given up was an accident of a materializing implementation, not a
decision that `[...]` should validate its elements; it made the answer depend on spelling
(`[.[]] | length` raised where `.[] | select(.) | .. ; map(.) | length` did not); and it cost
a whole-array copy on a filter that reads nothing it did not already need to. Pinned in
`test_lazy_validation_boundary_2168`'s and
`test_select_passes_through_corruption_it_only_tests_1645_2692`'s own rows.

[#2658](https://github.com/rust-works/succinctly/issues/2658) applied the rule to the last
five spellings of #2476's class: `any(cond)`/`all(cond)`, `isvalid(f)`, `until` and `while`.
Each had no native arm in the generic evaluator and fell to the wildcard bridge, whose first
act is a `to_owned` of the whole ambient input — `O(2^N)` on an alias fan-out, and a
whole-document validation for filters that read almost none of it. Their inputs are cursors
now: `any(cond)` probes `cond` at each element's own position and stops at the first
decisive one, `isvalid` runs `f` to exhaustion without reading its outputs, and the loops
carry a document state as a cursor for as long as `update` stays in the document. On
`{"a":"\ud800","d":5}`:

| filter                                                       | jq 1.7.1 | before #2658           | now                       |
|--------------------------------------------------------------|----------|------------------------|---------------------------|
| `any(true)`, `all(false)`                                    | error    | error — **matched jq** | `true`/`false` — **diverges** |
| `isvalid(.d)`, `isvalid(.a)` (navigates, never decodes)      | error    | error — **matched jq** | `true` — **diverges**     |
| `until(true; .) \| .d`, `[while(false; .)] \| length`        | error    | error — **matched jq** | `5`/`0` — **diverges**    |
| `[5, "\ud800"] \| any(. == 5)`                               | error    | error                  | `true` (#1755's short-circuit, as the eager arm always answered) |
| `any(. == "x")`, `isvalid(.a \| length)`, `until(.a == "x"; .)` | error | error — **matched jq** | error — **matched jq** (the read decodes `.a`) |

The eager arms these mirror already followed the rule as far as they could: `any_all_f`
converts each element right before its own probe (#1755), so an element the short-circuit
never reaches was never read — but one it does reach is converted whole even when `cond`
would not look at it, and the loops' `to_owned` of the starting value read everything. What
changed is only that the bridge's copy in front of them is gone. `isvalid` is a succinctly
extension (neither jq 1.7.1 nor yq defines it), so its rows have no reference either way;
its one rule change under this issue is that a decode failure now passes through it
uncaught, as it does through `try` (#1620), instead of being digested into `false`. Pinned
in `test_any_all_cond_isvalid_loops_validate_only_what_they_read_2658` and
`test_lazy_validation_boundary_2168`'s answering column.

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
`{k: .}`, anything without a native streaming arm **that reads `.`** — still materializes
and so still validates, which is the same rule applied to a route that reads the whole
document, not an exception to it. (`label $x | .` is *not* one of those: it has a native arm
and already answered at exit 0 on the eager route, as the matrix in 2 above records. `[.]`
was in this group until #2575 gave `Expr::Array` its own native, cursor-forwarding arm —
`[.]`'s single element is now a `LazySeq` pointing at the same cursor `.` did, so it no
longer materializes or validates either; see that issue's own entry above.) The three
examples just named all read `.`, and all still materialize; what changed under #2173 is the
bridge's behaviour for a filter that does **not** — see the next entry.

The context that makes the loss survivable is the one #2168's entry states: succinctly
already diverges on this whole class of document through `.` itself, deliberately, and a
user who wants jq's rejection has `--validate` and `succinctly json validate`.

**A raw control character is the one fault in this class that is caught document-wide
(#2878), and it is not an exception to the rule above.** "A filter validates only what it
materializes" governs the *filter*; it has never described the **document splitter**, which
runs on every input ahead of any filter and already rejected an unterminated string or
container there (`[1,2,` and `"unterminated` both exit 5 under `1+1`, matching jq). #2878
added an unescaped `U+0000`-`U+001F` inside a string to that same splitter class, so
`1+1` on `["a<TAB>b"]` now exits 5 as jq does, rather than answering `2`.

It is affordable where up-front validation is not for the reason that section's cost
argument turns on: `find_json_values` already inspects every byte of every string, so the
rule rides a scan the input pays for anyway — no "second index-building pass". It is also
the *stricter*, jq-matching direction, so it recovers fidelity in this table rather than
spending more of it.

The resulting asymmetry is real and deliberate: a raw control character is caught
document-wide, while a malformed *member* still is not (`{invalid}` stays at exit 0 under
`1+1`, where jq exits 5 — the row above). The line between them is the same cost-driven one
this section already draws. `0x7F` is on the accepting side of it in both tools, since jq's
own check compares a signed char.

A rejected later value no longer takes the values before it with it: on `{"ok":1}` followed
by `["a<TAB>b"]` (or a truncated `[1,2,`), succinctly prints `{"ok":1}` and then exits 5, as jq
does, on both input routes — closed by [#2961](https://github.com/rust-works/succinctly/issues/2961),
which keeps the splitter's clean prefix instead of discarding it, and queues the parse error
behind it for `input`/`inputs` (catchable there, delivered once, `break` after). Two
differences remain, both pre-existing: the error reads `Invalid JSON text` where jq quotes its
parser (`jq: parse error: Unfinished JSON term at EOF at line 2, column 5`, with no `(at …)`
marker from the driver loop), and on the lazy per-file route a splitter error moves on to the
next file, where jq stops the whole stream — the [#355](https://github.com/rust-works/succinctly/issues/355)
continue-past-error rule below. The `input`/`inputs` route stops there, as jq does. And the
`(at …)` marker a later error names after the parse error has been read counts newlines
through the end of the line the malformed value starts on, which is jq's answer whenever that
value sits on one line (`1\n2 }\n\n\n` → line 2); for a malformed value spanning several lines
jq names wherever its parser gave up inside it, and for one cut off at end of input
`<unknown>`, where succinctly still names the end of its first line.

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
| `.`, `. \| .`, `first(.)`, `getpath([])` | `[.] as [$x] \| $x` |
| `. as $x \| $x`, `. as {a:$v} \| .` | `. as $x \| $x + {}` |
| `if . then . else . end` | `tojson`, `to_entries`, `keys`, `with_entries(.)` |
| `[.] \| .[0]`, `. as $x \| [$x] \| .[0]` (#2575) | |

Every raw row forwards a cursor; every doubled row builds an `OwnedValue`. No filter on
either side contradicts the rule, which is what makes "the golden was stale" the answer
rather than "the rule is too broad". jq 1.7.1 has no opinion to appeal to — it rejects both
documents at parse time — so this is succinctly's own rule throughout. `[.] \| .[0]` and
`. as $x \| [$x] \| .[0]` moved from doubled to raw with #2575: `Expr::Array`'s
generic-evaluator arm no longer materializes a cursor-shaped inner result, so a
single-element `[...]` forwards its element's own cursor through `.[0]` instead of decoding
it. `[.] as [$x] \| $x` stays doubled — pattern destructuring binds through a different,
still-materializing route.

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

### `isempty`/`any`/`all`/`IN`/`skip` validate only what their arguments materialize (#2968)

The same mechanism again, six constructs over. `isempty(g)`, `any(gen; cond)`,
`all(gen; cond)`, `IN(s)`, `IN(src; s)` and `skip(n; f)` had no native arm in the generic
evaluator at all: every spelling reached `eval.rs` through `bridge_to_each_owned_flow`, whose
first act is the whole-document `to_owned_with_cursor` the #2173 entry describes, so they
validated everything — and, the reason [#2968](https://github.com/rust-works/succinctly/issues/2968)
was filed, evaluated their arguments with no cursor, so a positional read inside one
(`isempty(range(0; (key|length)))`) answered its no-cursor default on every route. All six
now have native, cursor-threaded arms on both routes (`each_isempty_generic` and siblings,
collected by `eval_builtin`), with `cond` evaluated at each `gen` output's own position.

On `{"a": "bad\x", "b": 5}`, which jq 1.7.1 rejects at parse time for every filter:

| filter                    | jq 1.7.1 | succinctly before | succinctly now |
|---------------------------|----------|-------------------|----------------|
| `isempty(.b)`             | error    | error             | `false`        |
| `any(.b; . == 5)`         | error    | error             | `true`         |
| `IN(.b; 5)`               | error    | error             | `true`         |
| `[skip(0; .b)]`           | error    | error             | `[5]`          |
| `isempty(.a)`             | error    | error             | `false`        |
| `[skip(1; .a)]`           | error    | error             | `[]`           |
| `any(.a; length > 0)`     | error    | error — unchanged | error — unchanged |
| `.a \| IN("x")`           | error    | error — unchanged | error — unchanged |
| `[skip(0; .a)]`           | error    | error — unchanged | error — unchanged |

The last three rows are the rule stated positively: an argument that *decodes* the
malformed scalar still raises. `isempty(.a)` and a dropped `skip` output do not decode it —
`isempty` asks whether `g` produces anything, as `select`'s truthiness read does under
[#2692](https://github.com/rust-works/succinctly/issues/2692), and a skipped output's value
is never read — so they answer, like `.[]` over the same document. `IN(s)` compares `.`
against each candidate, so it materializes both (`.a | IN("x")` raises; `.b | IN(5)` never
raised, `.b` being sound). Same in yq mode (`--jq-extensions`), where real yq rejects the
document at read time.

Sanctioned by ADR-0018's #2103 amendment, like the entries above. Pinned by
`consumers_validate_only_what_they_read_2968` and the `IN(1; 1)`/`skip` rows in
`test_closed_terms_do_not_validate_2173` (`IN(1, 2)` is deliberately *not* a closed term
there: it reads `.`).

**A read inside `cond` on the rewrite routes (#3079).** `cond` stands at each `gen`
output's position, which the constant rewriter cannot in general spell as the stage's
own, so #2968 left `map_values(any(.; key == "c"))` and `.aa[] |= any(.; key == "c")`
answering `false` where the direct spelling answers `[false,true]`.
[#3079](https://github.com/rust-works/succinctly/issues/3079) closes that three ways: when
`gen` hands the stage's input on unchanged (`.`, or an `if`/`try` every branch of which
does — deliberately *not* a `$x` frozen elsewhere or `A // B`, whose right side stands
elsewhere; PR #3087's review caught both) the rewrite *can* express `cond`'s reads, so
every rewrite route resolves them; the owned
identity pipe (the `map_values`/`map`/`with_entries` positioned route) runs a navigating
`gen` natively, probing `cond` from each `(value, identity)` pair
(`eval_owned_identity_any_all`); and a rewrite route that can prefetch through that pipe
(interpolation, an assignment's right side) does so for the navigating shape too. What
remains is the one route with neither: a `|=` filter whose `gen` navigates and whose
`cond` reads — `.aa |= any(.[]; key == "c")` answers `{"aa":false}` where the members'
keys say `true`. `.aa |= any(.; key == "aa")` is resolved. Note `with_entries(.value |= any(.;
key == "value"))`: the entry is `{key, value}`, so `key` there is `"value"` — real yq
answers `with_entries(.value |= key)` the same way.

[#2658](https://github.com/rust-works/succinctly/issues/2658) gave `any(cond)`/`all(cond)`,
`isvalid` and `until`/`while` native arms and registered them in the same gates. `any(cond)` is
`any(.[]; cond)` with a navigating `gen`, so a read in its `cond` takes exactly #3079's three
routes above (`map_values(any(key == "c"))` runs natively over `.[]`, a scalar member still
raising `any_all_f`'s own `Cannot iterate over …`) and shares its one remaining gap (`.aa[] |=
any(key == "c")` answers `false`). `isvalid`'s `f` stands at the stage's own input, so the
rewrite resolves it everywhere. **A loop whose `cond` or `update` reads position is resolved
on the streaming route and the positioned pipe (`[.[] | until(key == "c"; .c), key]`), but
not on the owned identity pipe:** `map_values(until(key == "c"; .c))` still falls to the
no-cursor evaluator, where `key` is `null` and the loop runs to its step cap or errors. A
native owned-identity loop (`eval_owned_identity_stages` over each `(value, identity)` state,
as `eval_owned_identity_any_all` does for one probe) is the fix; it is left recorded here
because `until`/`while` with a `key`-reading body on the `map_values` route has no reference
behaviour to match (`key` is not jq's, and real yq's lexer rejects `until`/`while`) and no
reported user. Pinned in `test_position_reads_inside_any_cond_isvalid_loops_agree_2658`.

Two further behaviours moved with the route, both toward jq: the `any`/`all` probe now stops
`cond` at its first decisive output on both evaluators, so `[any(1; (true, ("C"|stderr)))]`
writes nothing (jq 1.7.1: `or` breaks out of its `first`; succinctly wrote `C`), and
`[skip(1000; .users[])]`-shaped queries no longer re-serialize and re-index the document per
call (3–15× on a 7 MB `users` document, outputs identical). What the eager route's own doc
comment feared from leaving the owned bridge — duplicate-key and number-spelling fidelity —
was re-verified and does not move: `{"a":1,"a":2} | any(.[]; . == 1)` is `false` and
`[.[] | IN(2)]` is `[true]` on both, `[1.0, 1e2] | [skip(1; .[]) | tostring]` is
`["100"]` on both.

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

### A non-string `strflocaltime`/`strftime` format raises where jq aborts (#3046)

`0 | strflocaltime(1)` and `0 | strftime(1)` kill jq 1.7.1 on an assertion
(`Assertion failed: (JVP_HAS_KIND(j, JV_KIND_STRING)), function jv_string_value`, exit 134).
succinctly raises a catchable error at exit 5 instead — the "would take the host process
down" condition. The wording is succinctly's own (`expected string, got strflocaltime
format`), since jq has none to match.

`strflocaltime` uses the local zone `localtime` already uses, so the two agree with each
other, and each matches jq for a POSIX `TZ` offset string (`EST5EDT`, `UTC-9`). They do not
yet resolve an IANA zone name or the system zone and use UTC there instead — a gap rather
than a deliberate divergence, tracked in
[#3054](https://github.com/rust-works/succinctly/issues/3054). The `strftime` specifiers the
two share, and `strftime`'s own `%z`/`%Z`, are tracked in
[#3055](https://github.com/rust-works/succinctly/issues/3055).

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
was unchanged by #2157. (Since #3138 a key computed from the loop variable
through `.field`/`.[n]`/`tostring` -- `{($r.name): $r.score}` -- takes this
by-value path too; see the #3138 paragraph below.)

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
(`test_2152_closed_expr_to_owned_single_element_array_has_no_comma_wrapper`;
[#3138](https://github.com/rust-works/succinctly/issues/3138) folded
`literal_shaped_expr_to_owned` into its superset `closed_expr_to_owned`)
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

**An UPDATE that *assigns* into the accumulator
([#3138](https://github.com/rust-works/succinctly/issues/3138))** --
`reduce .users[] as $r ({}; .[$r.name] = $r.score)`, and the same with
`|=`, `op=` or `//=` -- was a third O(n²) shape neither #2086 nor #2157
reached: the UPDATE is an assignment, not arithmetic, so every step went
through `eval_each_owned`'s reindex bridge, serializing, re-indexing,
resolving and re-materializing the whole accumulator. `owned_assign_step`
(`src/jq/eval.rs`) now answers it on the owned state the fold hands over by
value, as an in-place write into the copy-on-write container (#2999). It
takes a path of static `.name`/`.[n]` steps and `.[K]` steps whose `K` is
*closed* -- a literal, or a pipe from a spliced `$r` through
`.field`/`.[n]`/`tostring` (`closed_expr_to_owned`) -- with a closed right
side, a string key into an object or `null`, and an integer key
`0 <= k <= len` into an array. Everything else is left to the evaluator
untouched, so every diagnostic is unchanged: a negative or padding index,
a float or `null` key, a key of the wrong kind, a combine that fails, a
multi-output or `.`-reading right side. The same shapes are answered
outside a fold too: `eval_owned_reindex_free`, which evaluates any
expression against an owned input without the reindex bridge, runs
`owned_assign_step` on a copy-on-write clone of that input, so
`map(.k = 1)` over a constructed array also skips the re-index.

Measured (Apple M5 Max, release build, `json generate 2mb -p users`, 13,981
records; interleaved A/B against the merge-base `82e73c6f9`, min of 2-3
reps, output identical to the merge-base and to jq 1.7.1 on every row):

| UPDATE (`reduce .users[0:N][] as $r (INIT; ...)`) | N      | before  | after  |
|---------------------------------------------------|--------|---------|--------|
| `.[$r.name] = $r.score`                           | 2,000  | 0.91 s  | 0.05 s |
| `.[$r.name] = $r.score`                           | 4,000  | 3.52 s  | 0.06 s |
| `.[$r.name] = $r.score`                           | 8,000  | 14.03 s | 0.08 s |
| `.[$r.name] = $r.score`                           | 13,981 | 42.98 s | 0.05 s |
| `.[$r.name] \|= $r.score`                         | 13,981 | 41.67 s | 0.05 s |
| `.[$r.name] += $r.score`                          | 13,981 | 41.46 s | 0.05 s |
| `.x[$r.name] = $r.score`                          | 13,981 | 43.55 s | 0.06 s |
| `[]` INIT, `.[$r.id] = $r.score`                  | 13,981 | 13.14 s | 0.05 s |
| `. + {($r.name): $r.score}`                       | 13,981 | 33.68 s | 0.06 s |
| `.[$r.name \| tostring] = $r`                     | 4,000  | 25.23 s | 0.06 s |

The "after" column is the ~40 ms process floor at every N. The before
column grows ~4x per doubling. The same holds for `. + {($r.name): $r.score}`,
because the arithmetic by-value path above (#2157) now takes a closed right
side too, not only a literal one. Controls are unchanged: `.users[] | .name`,
`. + "x"` (#2086), `. + [$x]` (#2152), `.i += 1` (#3025, 99,999 steps),
identity and `map` all read 1.0x.

What it does not change:

- **`foreach`** and **`while`** still hand the state to a second reader
  each step (EXTRACT, or the emit), so the copy-on-write write copies the
  map: still O(n²), at a memcpy constant instead of a re-index -- the same
  structural limit #2157 recorded above. Measured: `[foreach .users[0:4000][]
  as $r ({}; .[$r.name] = $r.score; length)]` 4.89 s -> 1.38 s (3.5x).
- **yq mode** keeps the evaluator's route for every assignment but #3025's
  `.field op= <number>` (the field spelled `.f`, `.["f"]` or `(.f)`, the
  number any closed expression such as `$r.n`): yq's `=` vivifies its targets before the right side
  runs (#2481), classifies no-op writes, and redirects writes through
  aliases (#1351), none of which a direct write reproduces.
- A fold running inside an `as` binding that holds a document node (the
  #2889 embed table is active, e.g. `. as $d | reduce ...`) skips the
  owned step entirely, as before (3.44 s at N=4,000, unchanged); tracked
  in [#3241](https://github.com/rust-works/succinctly/issues/3241).

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
divergence (see "Unary minus in filter text destroyed literal preservation"
below, #2357), not something #2219 changed. The pre-#2219 empty answer
matched jq's only by coincidence of that unrelated quirk; feeding the same
magnitudes in as *data* instead of literals
(`echo '[-9223372036854775758,-9223372036854775808,-100]' | jq -c
'[range(.[0];.[1];.[2])]'`) already showed jq answering
`[-9223372036854775758]` even before this fix -- the single value #2219 now
also gave from the literal spelling.

**Update, #3044:** the #2357 divergence just above is now closed, so this
section's own repro no longer demonstrates what it did when written --
`range(-9223372036854775758; -9223372036854775808; -100)` typed literally
now answers empty in succinctly too, agreeing with jq for the same reason
jq does (the literal is a computed double before `range` ever sees it).
The `checked_add`-based overflow-safety fix this whole section documents
is untouched; it just needs the data-sourced spelling above to reach it
with these particular magnitudes now, same as it always did in real jq.
See `test_range_i64_overflow_keeps_exact_prefix_2219`
(`tests/jq_cli_tests.rs`), which was retargeted to source `from`/`to`/`step`
from data for this reason.

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

### Unary minus in filter text destroyed literal preservation — closed, the accepted-divergence premise was false (#2357, closed by #3044)

Real jq preserves a number literal's exact source spelling through to output (`jq -n
'1.0'` → `1.0`, `1e10` → `1E+10`), but a *unary minus written in the filter text* breaks
that: `-1.0` is `negate(1.0)`, a computed `double`, not a preserved literal, so jq's own
answer at extreme magnitude no longer matches the value as written. Only reachable above
`2^53` (where a `double` can no longer hold the literal exactly). It is specifically the
*filter-text* unary minus: the identical value arriving as **data** keeps its exact
spelling in both tools, unaffected by any of this:

```console
$ echo '[-9223372036854775758]' | jq  -c '.[0]'   # -9223372036854775758
$ echo '[-9223372036854775758]' | sjq -c '.[0]'   # -9223372036854775758
```

**Previously accepted as a divergence, on a premise #3044 found false.** succinctly used
to keep the exact value regardless of a leading `-` in the filter, reasoned as the
necessary consequence of the `range` divergence's own ADR-0018 rule-4(c) grant above —
specifically because "`9223372036854775758` negated via `0 - 9223372036854775758`... keeps
the exact value today," so singling out unary minus for jq's rounding would have been an
inconsistent special case. #2631/#2906 made that premise false: binary `-`/`+`/`*` have
rounded a literal past `2^53` since those fixes, so `0 - 9223372036854775758` already
disagreed with the claim by the time #3044 checked it live — unary minus had become the
one remaining holdout, not a consistent exception. Closed by matching jq exactly:

```console
$ jq  -nc -- '-9223372036854775758'      # -9223372036854776000
$ sjq -nc -- '-9223372036854775758'      # -9223372036854776000  (was -9223372036854775758)
$ jq  -nc -- '-9007199254740993'         # -9007199254740992
$ sjq -nc -- '-9007199254740993'         # -9007199254740992     (was -9007199254740993)
$ jq  -nc -- '-1.10'                     # -1.1  (agrees -- magnitude-specific, unaffected)
$ sjq -nc -- '-1.10'                     # -1.1
```

**Fix:** a literal-adjacent `-` (`-9223372036854775758`, no parens/pipe) now splits into
`-1 * <positive literal>` in jq mode, the same desugaring `parser.rs` already used for a
negative float/exponent literal (#1035) — extended from "only when the text contains
`.`/`e`/`E`" to "whenever the magnitude is outside `jq_int_within_exact_f64_range`"
(`eval.rs`, made `pub(crate)` for this reuse), so the existing `arith_mul`/
`jq_checked_int_arith` rounding (#2631/#2906) does the rest. A magnitude within that range
(the overwhelming majority of real negative literals, e.g. every `.[-1]`-style index) is
untouched — same AST shape as before, so `fold_index_key`'s existing fast path for a
literal index is unaffected. The parenthesized/piped spellings (`-(9223372036854775758)`,
`9223372036854775758 | -.`) go through `arith_negate` (`eval.rs`) instead, which gained the
matching rounding rule directly. Pinned by
`test_unary_minus_matches_jq_past_2_53_3044` (`tests/jq_cli_tests.rs`).

**Downstream effect on the `range` divergence above:** `range`'s own accepted exact-`i64`
fast path is otherwise unchanged, but a bare negative-literal argument past `2^53` no
longer reaches it directly (it arrives as a computed `Float`, same as every other operand
this section already rounds) — `range(-9223372036854775758; -9223372036854775808; -100)`
written literally now agrees with jq's own empty answer, genuinely rather than by the
coincidence the `range` section above used to describe. Reaching the fast path with these
exact magnitudes still works, unaffected, via data (`.[0]`/`--argjson`/...) — see
`test_range_i64_overflow_keeps_exact_prefix_2219` (`tests/jq_cli_tests.rs`).

### Large-integer arithmetic past `2^53` — closed for `i64` literals: jq rounds a literal to 17 decimal digits *before* the double conversion (#2906)

#2631 fixed a fast-path bug where an exact, non-overflowing `i64` `+`/`-`/`*` result past
`2^53` was kept as `OwnedValue::Int` and printed via its own exact digits, bypassing
`jq_bare_float_display`'s shortest-round-trip formatting entirely. Its fallback — `a as f64
op b as f64` — still disagreed with real jq on a residual few percent of random 18/19-digit
operands, even with both operands non-negative:

```console
$ jq  -n '869389897822472004 + 944331'    # 869389897823416300
$ sjq -n '869389897822472004 + 944331'    # 869389897823416400  (before #2906)
```

An earlier revision of this entry recorded that residue as an open gap on the theory that
jq must be doing arbitrary-precision decimal arithmetic. It is not; the earlier analysis
modelled the *operands* as exact integers, and that was the mistake. jq 1.7.1 keeps every
parsed number — program text, document input, `tonumber`, `fromjson`, `--argjson` — as an
exact `decNumber` literal and converts it to a double only when arithmetic first reads it,
in `jvp_literal_number_to_double` (`src/jv.c`): `decNumberReduce` under a
`DEC_INIT_DECIMAL64` context whose `digits` is raised to 17 (`DEC_NUBMER_DOUBLE_PRECISION`,
round-half-even), then `decNumberToString`, then a correctly-rounded `strtod`. That is a
*double* rounding — the literal's decimal value is first rounded to 17 significant decimal
digits, and only that shorter number is rounded to the nearest double — and it lands on a
different double than the exact integer's nearest one whenever the 17-digit intermediate
sits across a rounding boundary. `869389897822472004` (18 digits) becomes
`869389897822472000`, whose nearest double is `869389897822471936` (a tie, resolved to
even); the exact integer's nearest double is `869389897822472064`. Every binary operator
then runs plain `double` arithmetic on those values, `%` truncates each of them to
`intmax_t` (`dtoi`, saturating) before taking the remainder, and a *computed* number is a
plain double that is never re-rounded. Modelled that way, jq's answer was reproduced on
100% of 1200 random `+`/`-`/`*`/`/` cases, 200 input-sourced cases including negatives and
over-`i64` values, and 100 program-literal negatives (the "no model explains it" bucket
was 0).

For an `i64` the two conversions differ only when the magnitude is at least `10^17` (18 or
19 digits): below that a decimal integer has at most 17 significant digits, so the
intermediate rounding is the identity and `n as f64` was already right — everything in
`[2^53, 10^17)` behaved correctly before this fix. `jq_literal_int_to_f64`
(`src/jq/value.rs`) implements the rounding with integer arithmetic only, and jq mode uses
it wherever an `Int` is widened for `+`/`-`/`*`/`/`/`%`, in both evaluators; `%`
additionally follows `binop_mod`'s truncate-the-double model, so `869389897822472004 %
1000` is `936` (jq) rather than the exact `4`, and `9007199254740993 % 2` is `0`.

Comparison follows `jvp_number_cmp`, which has two rules (`jq_numeric_cmp`, behind
jq-mode `==` and every ordering consumer — `<`, `sort`, `unique`, `group_by`, `min`/`max`,
`bsearch`): a literal against a *computed* double widens the literal through the same
17-digit rounding, so `869389897822472004 == (869389897822472004 + 0)` is `true`; two
*literals* compare exactly as decimals (`decNumberCompare`), with no rounding on either
side, so `869389897822472004 == 869389897822471936.0` is `false` while `869389897822472004
== 869389897822472004.0` is `true`, `9007199254740993 == 9007199254740992.0` is `false`
(previously a recorded divergence), and `1.00000000000000001 == 1.0` is `false`. The
mode-blind `PartialEq` on `OwnedValue` keeps the old plain-cast widening: the yq
presentation layer relies on it (comment alignment across a `|=` matches a yq-mode `Int`
against the plain-cast double yq's own arithmetic produced), and a review round caught
that routing it through the jq rounding moved yq comments onto the wrong elements. yq mode
is otherwise untouched: real yq's `int64` arithmetic is exact (`869389897822472004 +
944331` is `869389897823416335` there, as it always was here), so its widening stays a
plain cast (`EvalSemantics::DECNUMBER_LITERALS`).

Two things #2906 makes *visible* without changing: jq's `dtoi` on exactly `2^63` is a C
cast that saturates on the arm64 oracle (`as i64` does the same), so
`9223372036854775807 % 10` is `7` on the pin -- but the cast is undefined behaviour in C,
and **the x86_64 case is not live-verified**: no x86_64 jq 1.7.1 was available to capture
this session, so whether it saturates the same way there, yields `INT64_MIN`, or differs by
compiler/libc is an open question, not a claim to build on (flag for whoever closes #2936
with x86_64 hardware in reach); and jq parses `-x % y` as `-(x % y)`
(unary minus binds looser than
`%`) where succinctly parses `(-x) % y`, a pre-existing precedence gap that only shows at
this magnitude (`-9223372036854775807 % 10` is `-7` in jq, `-8` here; the parenthesised
and data-sourced spellings agree on `-8`).

**Closed in two steps.** The math builtins (`floor`, `sqrt`, `pow`, …) widening a large
`Int` with a bare cast (`869389897822472004 | sqrt` was `…647` here against jq's `…645`)
closed as a side effect of #2936 routing the cursor-level readers through
`document_number_f64`, which widens a document number the way jq does (both the integer
and the float-literal-rounding halves, #2906/#2936's own scope). The sibling gap #2936 left
open — `floor`/`ceil`/`round`/`trunc` (and `length`, which is `fabs(jv_number_value(x))` in
jq) of a large `Int` literal printing their exact integer digits where jq prints the double
(`869389897822472004 | floor` is `869389897822472000` in jq, `869389897822471936` here --
the same double, spelled exactly; before #2936 it was the unrounded `869389897822472064`)
-- was filed as #2937 and closed by #2937 itself: `integral_f64_result` builds
`floor`/`ceil`/`round`/`trunc`'s own `Int`/`Float` result from the computed double directly
(capped at `2^53` in jq mode, the same display-optimization bound arithmetic results
already used, rather than an unconditional `Int`), and `numeric_length_owned`'s jq arm gets
the identical cap. A third gap this fix's review found — the reindex bridge re-parsing a
computed double's printed digits as an *integer* literal before a document-input builtin
(`sort`, `unique`, `min`, `max`, `group_by` on an array built in the filter), so that it
then compared exactly against a real literal instead of equal — was filed as #2938 and
closed by #2902 landing first: the bridge now hands a computed float back as a bare
`Float`, so `[869389897822472004, (869389897822472000+0), 5] | sort` is jq's
`[5,869389897822472004,869389897822472000]` here too. The `range` entry above stays on the
exact `i64` path it documents, unaffected. (The unary-minus entry once cited
alongside it here was #2357, since closed by #3044 — see that section for why it no
longer belongs in this "unaffected" list.)

### Every literal rounds to 17 significant digits before the double conversion, not just an `i64` (#2936) — divergence closed

#2906 modelled `jvp_literal_number_to_double` -- `decNumberReduce` under a DECIMAL64 context
with `digits` raised to 17, half-even, then a correctly rounded `strtod` -- for `i64` literals
only. Every fraction, exponent form and integer past `i64` still took Rust's single correctly
rounded parse, which lands on a different double whenever the 17-digit intermediate sits on
the other side of a rounding boundary: 2-6% of random 18-30-digit literals (measured against
`/usr/bin/jq` over 4000 seeded cases; the model matched 4000/4000).

```console
$ jq  -n '2.7293109604053567083 + 0'         # 2.7293109604053565
$ sjq -n '2.7293109604053567083 + 0'         # 2.729310960405357   (before #2936)
$ jq  -n '2.4703282292062327209e-324 + 0'    # 0   -- the subnormal half-way case
$ jq  -n '1.797693134862315808e308 | isinfinite'   # false -- 17 digits round down to DBL_MAX
$ jq  -n '2.7293109604053567083 == (2.729310960405357 + 0)'   # false (was true here)
```

Every entry point jq reads a literal through moves together -- program text, the document on
the cursor, materializing and `--slurp` routes, `input`/`inputs`, `--argjson`, `--jsonargs`,
`--slurpfile`, `--seq`, `tonumber`, `fromjson`, and the `|=`/`getpath`/`to_entries`/`tostream`
shapes -- because the double is fixed where the literal is materialized
(`OwnedValue::from_number_literal::<S>`/`from_number_bytes::<S>`, the parser's number token)
rather than at each of the ~50 sites that read a `NumberRepr::Float`. There is no mode-less
funnel left to default silently to the plain parse. The cursor-level readers that never build an
`OwnedValue` (`length`, `isnan`/`isinfinite`/`isnormal`, `normals`/`finites`, `strftime`,
`implode`, every libm builtin through `get_float_value`) read through one accessor,
`document_number_f64::<S>`, which is also where #2937's `Int` widening belongs.

Display is untouched: a `NumberLiteral` keeps its source text, so `2.7293109604053567083`,
`| tojson`, `| tostring` and `1.797693134862315808E+308` print as before, and two literals
still compare exactly by their digits (`2.7293109604053567083 == 2.7293109604053565` is
`false` in both). That exactness is why `jq_numeric_cmp`'s two-literal shortcut -- doubles
first, digits on a tie -- had to change with it: an `Int` literal's double now comes from
`jq_literal_int_to_f64` too, never `as f64`, or `869389897822472001 < 869389897822472004.9`
(`true` in jq, and here before and after) would have read `false` with only the `Float` side
rounded.

yq mode is unmoved: real yq parses with Go's correctly rounded `ParseFloat`, so `YqSemantics`
keeps the plain parse in every funnel, including the reindex bridge under a write and the
`-p json` DOM route (`test_yq_float_literal_stays_correctly_rounded_unaffected_by_2936`,
`tests/yq_cli_tests.rs`). No ADR: ADR-0018 rule 3 decides this, and there is no choice between
behaviours to make. Pinned against jq 1.7.1 on a 2,015-row table
(`tests/data/jq-literal-17-digit-oracle-2936.tsv`) plus the entry-point matrix in
`tests/jq_cli_tests.rs` and the two-evaluator rows in `tests/jq_evaluator_parity_tests.rs`.

### `--argjson`/`--jsonargs`/`fromjson` still reject a bare trailing decimal point with no exponent (`1.`) — accepted divergence, ADR-0018 rule 4c (#2240, #3032)

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

#2052 later replaced the whole validation gate behind these flags -- `serde_json` plus one
text-rewriting normalizer per leniency became one lenient mode on this crate's own RFC 8259
validator (`json::validate::validate_jq_lenient`) -- and this rejection moved with it,
unchanged and for the same reason: the lenient mode admits the other three spellings and
refuses a bare trailing dot explicitly, with `validate_number`'s own comment pointing back
at this row. The accept-set is pinned against jq 1.7.1 in
`test_argjson_accept_set_matches_jq_2052`/`test_argjson_reject_set_2052`
(`tests/jq_cli_tests.rs`).

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

### A plain decimal and an overflowing literal render with decNumber's notation (#3212) — divergence closed

jq 1.7.1 renders every number literal with decNumber's to-scientific-string rule: scientific
notation when the exponent is positive or the adjusted exponent (`exponent + digits - 1`) is
below -6. succinctly applied that rule only to literals *written* with an exponent
(#1207/#1226), and echoed a plain decimal verbatim. Now a plain decimal follows it too, on
printing, `tostring` and every text format alike: `0.0000001` → `1E-7`, `0.000000100` →
`1.00E-7`, `0.0000000` → `0E-7`, while `0.000001` stays `0.000001`. Separately, `tostring` of
a literal that overflows `f64` (`1e400`) keeps its literal as jq does (`"1E+400"`), where it
substituted `DBL_MAX` text. Only a *computed* infinity (`1e400 + 0`) still takes the
`DBL_MAX` text, as in jq.

The scientific path used to render at most 100,000 mantissa digits
(`MAX_RENDERED_MANTISSA_DIGITS`), so a literal with more significant digits than that came out
truncated where jq renders every one. [#3257](https://github.com/rust-works/succinctly/issues/3257)
removed that cap — real jq itself has none (oracle-verified past 500,000 digits, no ceiling
found) — so every notation path now renders every given digit unconditionally, matching jq's
own cost profile: the input already had to contain that many bytes to trigger the cost, so
it's linear in the caller's own paid-for input, not an amplification.

### A magnitude-overflowing literal is no longer rejected by `--argjson`/`--jsonargs` (#2052) — divergence closed

Until #2052 these two flags validated through `serde_json::Value`, whose "number out of
range" refused a literal too large for `f64`. That was never a jq behaviour:

```console
$ jq -nc --argjson x 1e400 '$x'
1E+400
$ succinctly jq -nc --argjson x 1e400 '$x'     # before #2052
Error: Invalid JSON for --argjson x
$ succinctly jq -nc --argjson x 1e400 '$x'     # after
1E+400
```

#1095 adopted `serde_json::Value` over the cheaper `IgnoredAny` specifically to keep that
rejection, reasoning the value would otherwise materialize as `null`. Both halves have since
stopped holding — the materializer preserves the literal spelling (the `--slurpfile` and
primary-input paths already answered `1E+400`), and the oracle accepts it — so dropping the
serde gate closed a divergence rather than opening one. `+1`, `nan`/`NaN` and
`Infinity`/`-Infinity`, which jq also accepts, stayed rejected on every input path until
[#2877](https://github.com/rust-works/succinctly/issues/2877) closed them too — the next
section.

`succinctly yq`'s own `--argjson` keeps the rejection, per ADR-0018 rule 2 -- real yq's JSON
input path refuses an overflowing literal as well, and yq mode has nowhere to put an
infinity. See the yq limitations doc for that half.

### jq's decNumber number spellings — `+1`, `nan`, `sNaN12`, `Infinity` — are read on every input path (#2877) — divergence closed, one identity residual

jq 1.7.1 builds with decNumber, and its parser hands every token that is not `t…`/`f…`/`nu…`
to `decNumberFromString` (`check_literal`, `src/jv_parse.c`), so a JSON *number* there has
decNumber's grammar: an optional `+` as well as `-`, and the special values `nan`/`sNaN`
(case-insensitive, optional digit payload) and `inf`/`infinity` (case-insensitive). A NaN is
a real number (`type` is `"number"`, `isnan` is `true`) that only *prints* as `null`; an
infinity is a real `f64` infinity clamped to `DBL_MAX` text at print time; `+X` prints
exactly as `X`. Until #2877 succinctly rejected every one of these on every path:

```console
$ printf '[nan,+1.500,-Infinity,sNaN12]' | jq -c '., map(type), (.[0]|isnan), (.[2]|isinfinite)'
[null,1.500,-1.7976931348623157e+308,null]
["number","number","number","number"]
true
true
$ printf '[nan,+1.500,-Infinity,sNaN12]' | succinctly jq -c .     # before #2877
[jq: error (at <stdin>:0): Invalid JSON text: invalid keyword 'nan' (expected null, true, or false)
$ printf '[nan,+1.500,-Infinity,sNaN12]' | succinctly jq -c '., map(type), (.[0]|isnan), (.[2]|isinfinite)'   # after
[null,1.500,-1.7976931348623157e+308,null]
["number","number","number","number"]
true
true
```

The same on the primary document (top-level and nested, raw-echo, cursor, materializing and
`--slurp` routes), `-n input`/`inputs`, `--argjson`, `--jsonargs`, `--slurpfile` and `--seq`
(whose reader already accepted the grammar and then silently dropped the record), plus
`tonumber` on `"sNaN"`/`"nan12"`, the two words Rust's own float parser lacks. The reject
side is jq's too: `nanx`, `nan1.5`, `nan(1)`, `infinity1`, `inf1`, `+-1`, `++1`, `+ 1`,
`{nan:1}` all stay `Invalid numeric literal` (exit 5) — the whole token has to validate, so a
valid prefix is never read out of an invalid word. The grammar lives once, in
`json::validate::jq_special_number`/`strip_leading_plus`; the `--seq` reader that first
implemented it (#1723) now calls the same function. No ADR: ADR-0018 rule 2 decides this, and
none of the rule-4 conditions apply. The `fromjson` path joined them in
[#3032](https://github.com/rust-works/succinctly/issues/3032): jq mode now validates the
entire string with `validate_jq_lenient` and materializes it through the JSON
cursor, preserving decimal spelling and overflowing literals. Its former hand-written
parser remains on the yq path, whose separate fidelity issue is #2018.

```console
$ jq -nc --arg s '007.500' '$s|fromjson'
7.500
$ succinctly jq -nc --arg s '007.500' '$s|fromjson'   # after #3032
7.500
$ jq -nc --arg s '1e400' '$s|fromjson'
1E+400
$ succinctly jq -nc --arg s '1e400' '$s|fromjson'     # after #3032
1E+400
```

`--preserve-input` (a succinctly extension) echoes the source spelling verbatim, as it does
for `007` and `.5`. yq mode is unmoved: `-p json` goes through the YAML parser and rejects
every spelling as real yq does, `-p json --slurp`/`--eval-all` and yq's `--argjson` refuse
the non-finite words at their materializer (that mode has nowhere to put a NaN) and admit a
leading `+` with the spelling dropped, like the `007`/`.5` they already take.

**The residual — a literal NaN compared with itself.** jq's `jv_equal` short-circuits on
pointer identity before comparing values, and a parsed number literal is an allocated `jv`,
so the *same* document NaN is equal to itself there while a computed NaN is not:

| Filter                              | Input        | jq      | succinctly |
|-------------------------------------|--------------|---------|------------|
| `.[0] == .[0]`                      | `[nan]`      | `true`  | `false`    |
| `.[0] as $x \| $x == $x`            | `[nan]`      | `true`  | `false`    |
| `indices(.[0])`                     | `[nan,NaN]`  | `[0]`   | `[]`       |
| `.[0] == .[1]`                      | `[nan,NaN]`  | `false` | `false`    |
| `nan == nan`, `nan as $x \| $x == $x` | (any)      | `false` | `false`    |
| `. == .`                            | `[nan]`      | `true`  | `false`    |

The last row shows the class predates #2877: the identity short-circuit fires on any
allocated value holding a NaN, the builtin's array included. `OwnedValue` has no notion of
value identity (two copies of a NaN are indistinguishable from one), so matching this would
need a representation change; recorded here on its merits, tracked as
[#3069](https://github.com/rust-works/succinctly/issues/3069). It is observable only when a
NaN is compared against the very same NaN; `sort`, `unique`, `group_by` and `<` already agree
with jq.

### A malformed nested number raises when read; jq rejects the document (#966, #3034, #3222) — accepted uniform divergence

A number *inside* an array or object is recovered by a greedy span (`[0-9.eE+-]`, plus
#2877's decNumber words). A span that none of jq's number spellings accepts is an error
value in the index. Every route that reads it raises, and a route that never reaches it
still answers. jq's parser rejects the whole document instead:

```console
$ printf '[1.2.3,2]' | jq -c .                 # parse error: Invalid numeric literal at line 1, column 7, exit 5
$ printf '[1.2.3,2]' | succinctly jq -c .      # Invalid JSON text: expected ',' or ']', found '.', exit 5
$ printf '[1.2.3,2]' | succinctly jq -c '.[0] | type'   # exit 5, as are length, tostring, .x, has, ...
$ printf '[1.2.3,2]' | succinctly jq -c '.[1]'  # 2, exit 0 -- jq: exit 5
$ printf '[1.2.3,2]' | succinctly jq -c 'length' # 2, exit 0 -- jq: exit 5
```

This is ADR-0018's #2103 amendment: a value is validated when, and only when, something reads
it (#2692), the rule a malformed object member already follows (#1194). The error is a decode
failure, so neither `try` nor `?` catches it, as neither can in jq. Truthiness reads nothing
(#2692), so `-e`, `select`, `if`, `and`/`or`, `not` and `//` still answer on the bad value
itself (`.[0] | not` is `false`), and a slice raises because it materializes its array, as it
does for every value the index can't read.

**Until #3222 the span read as `null`** (#966). `null` was a local fallback chosen over the `0`
the funnels made up before #966. Nobody ever compared it with jq, and it was filed here under
the amendment afterwards. That was a misreading. The amendment accepts a *uniform* divergence
because matching jq's up-front rejection isn't reachable, but raising on read is. It is
uniform, and it matches jq on every filter that reads the value, while `null` matches jq on
none. The `null` wasn't even uniform: `.[0] | type` was `"number"`, `.[0] | length` raised
`null (null) has no length`, which `try` could catch, and the printer and funnels printed
`null`. `--slurpfile` now rejects such a document too (exit 2), as jq does.

A top-level malformed number was already rejected (the top-level splitter is strict), and
`--validate` rejects the document up front (exit 3). `--preserve-input` with compact output
(`--preserve-input -c .`) copies each value's source bytes without decoding them, so it echoes
the span verbatim, as it echoes a stray trailing comma. That is the flag's deliberate
non-validating echo, not a route that reads the value.

**The reindex bridge's number tokens are in the class, not exceptions to it (#3034).**
succinctly's evaluator round-trips computed values through JSON text, and writes a NaN, an
infinity and a computed float as `9e999e999`, `8e999e999`/`-8e999e999` and `<float>e0` there.
Until #3034 the same bytes in a *document* decoded as the value they stand for, breaking the
uniformity above: `[9e999e999]` was NaN, `[8e999e999]` was `DBL_MAX`, `[1e0e0]` was `1`, and
routes disagreed (`isnan` true, `tostring` `"null"`). Only an index built over the bridge's own
text decodes a token now (`JsonIndex::build_reindex`, `JsonNumber::bridge_value`), so each
spelling reads exactly like its nearest non-token sibling (`9e999e998`, `1e0e1`) on every route
-- since #3222, by raising.

Pinned by `test_bridge_token_spellings_read_like_their_siblings_3034` and
`test_bridge_tokens_still_round_trip_computed_values_3034` (`tests/jq_cli_tests.rs`),
`test_bridge_token_spelling_in_a_real_document_is_not_a_token_3034` (`src/jq/eval.rs`), and
`bridge_tokens_decode_only_under_the_bridge_index_3034` (`src/json/light.rs`).

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
`eval_generic.rs`'s generic/CLI evaluator — share one core per construct (`reduce_forks`,
`foreach_forks`), and neither ever substitutes a synthetic `null` for the ambient document.
Both now re-drive SOURCE per INIT fork — `foreach` since #2180 WP3's review, `reduce` since
#2899 — against the *real* ambient input every time, which is exactly the half jq does not
do. (Before #2899 `reduce` differed again: it computed SOURCE's values once, upfront, and
reused them across every fork, so `[reduce ("s"|stderr) as $x ((0,1); .)]` wrote `s` where
jq writes `ss`. That half is closed; the synthetic-`null` half is what remains.)

**#2899 widened this entry's reach**, and the trade is worth stating plainly. `reduce`'s old
value-caching *accidentally* matched jq on shapes where the only visible difference was
between "re-drive against the real document" and "re-drive against jq's synthetic `null`" —
because caching re-drove against nothing at all. Driving per fork correctly is what the `?//`
fix required, and it exposes the synthetic-`null` gap on those shapes:

```console
$ echo '{"a":1}' | jq -c 'reduce ((.[]|stderr)?) as $x ((0,1); .)'
1            # succinctly writes `11`; it wrote `1` before #2899
```

jq's second fork iterates its synthetic `null`, the `?` swallows that error before `stderr`
runs, so jq writes once; succinctly's second fork iterates the real `{"a":1}` and writes
again. Stdout and exit codes are unaffected in every such shape. This is the divergence this
entry already records, reached by more spellings — not a new one, and not something `reduce`
had ever deliberately matched.

That front-end-level agreement (not independent
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
shared `reduce_forks`/`foreach_forks` core so it evaluates SOURCE per-fork against a
synthetic `null` document — which is now only the synthetic-`null` half for *both* folds, the
per-fork re-evaluation itself having landed with #2180 WP3's review for `foreach` and #2899
for `reduce`. A prior attempt at a narrower version of this fix (nulling only the bound
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

### Caller-supplied NaN literals still avoid the reindex bridge

`jq::eval_owned_with_file_index` can receive a caller-built NaN literal. The
reindex bridge cannot preserve that representation, so the owned identity
route still handles it without serialization. #3025 removed the analogous
long-literal exception: non-NaN number literals of any length now retain
their text through the bridge, including stages reached from the owned route.

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

**A long literal's spelling now matches jq on both routes.** A `NumberLiteral` longer than
the former 256-character reindex cap disqualified a document from `getpath`'s native arm,
sending the call through the reindex round trip, which re-spelled it: `getpath(["big"])`
printed jq's `1E-301` for a 303-character `0.000…1`, and `.big` printed all 303 characters,
because a document number's written form is preserved (`DocumentValue::number_literal`,
[#387](https://github.com/rust-works/succinctly/issues/387)/[#966](https://github.com/rust-works/succinctly/issues/966)).
#2168 made both print the source form. Since
[#3212](https://github.com/rust-works/succinctly/issues/3212), rendering a preserved literal
applies decNumber's notation rule, as jq does, so both routes print `1E-301`.

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

### `break $x` shadowed by a `def error:` observes arbitrary ambient pipe state, not jq's own context-independent `{"__jq":N}` sentinel (#2687, #2840)

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

**Closed by #2840.** `label`'s own error re-raise (when a caught escape isn't a matching
`Control::Break`) is *also* `gen_call("error", gen_noop())` in jq, resolved at the **label's**
lexical position — so a shadowing `def error:` also intercepts an uncaught
`error(...)`/division-by-zero/etc. escaping through a `label` block, and a non-matching
`break` of an *outer* label too. Fixed by an AST rewrite at `resolve.rs`'s `check`/
`build_call_graph` time (`in_scope(scope, "error", 0)` at the label rewrites its body to
`try BODY catch <call to error/0>` — the same runtime semantics every evaluator arm already
gives `try`/`catch`, so no evaluator arm changed), rather than the fallthrough-rewrite-in-six-
places approach originally filed: `Expr::Try`'s catch arm already binds the raised payload as
`.`, catches a `Control::Break` of any label (#562), preserves the already-emitted prefix, and
declines `Halt`/uncatchable errors, in every arm that walks `Expr::Label`.

**Break-payload divergence, extended.** The `{"__jq":N}`-vs-`null` payload gap this entry
already documents for a *directly* shadowed `break $x` applies identically to a break
intercepted through #2840's re-raise (`def error: .; label $a | (def f: break $a; label $b | f)`
sees `null` where jq sees the sentinel) — the same #562 choice, not a new divergence.

**A narrower gap found while fixing #2840 — closed by #2964 meanwhile.** With `error` in
scope at `f`'s own lexical position rather than only at the label's, #2687's shadowing
would hit the `break` directly there, before it ever became a `Control::Break` — so before
#2964 the *shadowing* def declared *after* `f` produced a Try-based re-raise that
swallowed a self-matching break only jq's compiler would reject:

```console
$ jq -nc 'def f: break $x; def error: "S"; label $x | f'
jq: error: $*label-x is not defined at <top-level>, line 1:
def f: break $x; def error: "S"; label $x | f       
jq: 1 compile error
$ succinctly jq -nc 'def f: break $x; def error: "S"; label $x | f'
jq: error: $*label-x is not defined at <top-level>, line 1:
def f: break $x; def error: "S"; label $x | f       
jq: 1 compile error
```
(The echoed lines above carry jq's caret-echo padding; both streams are byte-identical,
verified against the pinned oracle.) #2964's compile-time label-scope check is exactly the
real fix this gap was filed as calling for.

Real jq's own label scoping is lexical at compile time — `break $x` is only valid textually
inside `label $x | ...`, so `def f: break $x;` (declared *before* the label, where `$x` is
not yet in scope) fails to compile regardless of whether `f` is ever actually called from
inside the label, and regardless of any shadowing `def error:` declared afterward.

**Closed by #2964.** succinctly now applies the same compile-time label-scope check
(`resolve::check` tracks a `label_scope` of enclosing label names per expression, its
`Expr::Break` arm reports `$*label-<name> is not defined` with the same `jq: N compile
error`/exit-3 reporting as the unresolved-call and unbound-variable paths), so
`def f: break $x; label $x | f` — with or without a shadowing `def error:` — now fails to
compile byte-for-byte like jq, including the caret-echo padding pointing at the break's own
column. Four CLI tests pin this under the "compile-time label-scope check" heading.

**Formerly a residual, closed by #3085: an unreferenced `def` body shifted the cited
position.** The occurrence counter the CLI indexes its site tables with used to count only
the sites `resolve::check` visited, and `check` skips unreferenced `def` bodies (jq never
compiles them either). The site tables come from a separate textual re-parse that lists
every site, so a same-named site inside an unreferenced body, textually before the failing
one, took the failing site's slot: `def f: break $x; break $x` cited column 7 where jq
cites column 17. `check` now walks an unreferenced body to count its sites and discards
the errors it would raise, so the index and the table agree for calls and breaks:

```console
$ succinctly jq -nc 'def f: break $x; break $x'
jq: error: $*label-x is not defined at <top-level>, line 1:
def f: break $x; break $x                 
jq: 1 compile error
```

Inside a module body the count also restarts at each module-level `def`, because the
loader does not always splice the whole module: a dependency's link run keeps only the
defs something references, and each `import` of a module is a separate copy that can reach
different defs. Per-def counting keeps every copy pointing at the same physical site, which
is also what lets the CLI report a site once however many copies reach it, as jq does.
**Closed by #3107: an unbound variable in the *main filter* used the older failures-only
counter.** `UnboundVar` (`resolve.rs`) already carried the same scope-aware `occurrence`
count `#3085` gave calls and breaks, but `jq_runner.rs`'s `report_unbound_var` call site for
the main filter still indexed `var_sites` with its own separately-tracked, failures-only
counter instead of consuming `occurrence` directly (the module-body branch a few lines away
already did). `def u: $x; $x` used to cite the first `$x` (inside the unreferenced `def`)
instead of the second; now byte-for-byte against jq:

```console
$ succinctly jq -nc 'def u: $x;
$x'
jq: error: $x is not defined at <top-level>, line 2:
$x
jq: 1 compile error
```

A second, distinct shape hits the same table-vs-counter mismatch for a different reason:
`reduce`/`foreach` checks `init` before the bound pattern's own computed keys (`init` must
not see the pattern's not-yet-bound variables, #2734) even though the pattern is written
*first* in the source (`reduce EXPR as PATTERN (INIT; UPDATE)`) — confirmed this is not an
implementation quirk but matches real jq's own diagnostic order for the construct, live:

```console
$ jq -nc 'reduce (1,2) as {($x): $v} ($x; .)'
jq: error: $x is not defined at <top-level>, line 1:
reduce (1,2) as {($x): $v} ($x; .)                            
jq: error: $x is not defined at <top-level>, line 1:
reduce (1,2) as {($x): $v} ($x; .)                  
jq: 2 compile errors
```

jq's own compiler visits `init` before the pattern too (its first reported error is
`init`'s `$x`, not the pattern key's, even though the pattern is written first) — so the
resolver's visit order already matches jq's *diagnostic order*. What it cannot also match
is `collect_var_sites`/`collect_break_sites`'s table order, which is sorted by pure text
offset (the pattern key's `$x` sorts before `init`'s): a same-named `$var`/`break $x` split
across a computed pattern key and `init` gets the *right two messages in the right order*,
each pointing at the *other* one's position. Reordering the visit to match textual order
was tried and reverted: it fixes the position match at the cost of reporting the two
diagnostics in the wrong order relative to jq, which is a worse trade for a construct real
jq itself does not compile in textual order. Confirmed live with `break $x` in place of
`$x` (same swap, `reduce (1,2) as {(break $x): $v} (break $x; .)`).

### A module `include` cycle is a compile error, where jq segfaults — accepted divergence, ADR-0018 rule 4 (#2865)

[#2865](https://github.com/rust-works/succinctly/issues/2865) made a module's own
`include`/`import` directives load transitively, which makes a cycle between two modules
reachable for the first time. Real jq 1.7.1 does not diagnose one at all — it recurses
until the process dies:

```console
$ cat ca.jq
include "cb";
def a: 1;
$ cat cb.jq
include "ca";
def b: 2;

$ jq -L . -n 'include "ca"; a'; echo "exit=$?"
exit=139                       # SIGSEGV, nothing on stdout or stderr

$ succinctly jq -L . -n 'include "ca"; a'; echo "exit=$?"
jq: error: module cycle detected: ca -> cb -> ca

jq: 1 compile error
exit=3
```

A module that includes itself behaves the same way in jq (`exit=139`), and reports
`module cycle detected: selfinc -> selfinc` here.

This is the **cleanest** of ADR-0018 rule 4's carve-outs rather than a policy stretch:
the rule permits refusing the reference's behaviour where "matching would take the host
process down," and matching here means exactly a SIGSEGV. There is no reference *output*
to be faithful to — jq writes nothing to either stream — so the only open question was
which shape to leave through, and the answer is the one the other two compile-error kinds
already use (`jq: N compile error`, exit 3, per "Undefined functions and arity
mismatches" above).

Detection keys on the **resolved** file rather than the module path as written, so one
module reachable under two spellings still closes a cycle
(`alia.jq` containing `include "./alia"` reports `alia -> ./alia`); the chain in the
message keeps the spellings, since those are what the source actually says. It cannot use
the module memo cache as its guard: a module is absent from that cache for exactly as long
as its own dependencies are loading, which is precisely the window in which a cycle closes.

`test_module_cycle_is_a_compile_error_not_a_hang_2865` (`tests/jq_cli_tests.rs`) pins all
four shapes (two-module cycle, self-include, aliased spelling, `import`-side cycle).

### Seven module-scoping rules that *are* matched, and read as bugs (#2865)

Not divergences — recorded here because the next person to touch `ModuleLoader` will
otherwise read them as ones, and because the exclusions in `dep_stubs_for` have no
other explanation. All captured live against jq 1.7.1, with `inner.jq` = `def g: 42;`:

1. **A dependency outranks the module's own same-name sibling.** With the module written
   `include "inner"; def g: 7; def h: g;`, `h` answers **`42`** — not the `7` one line
   above it. The dependency is bound innermost.
2. **...while that module still exports its own `g`**, which answers `7`.
3. **The same collision at the top level goes the opposite way.** `include "inner"; def
   g: 7; g` answers `7`, because a filter's own defs bind at parse time, before the
   module block is spliced in.
4. **A def's own recursive call binds to itself, not to a same-named dependency, per
   (name, arity).** With `inner`'s `g/0` in scope, a module's
   `def g: if . == 0 then "base" else (. - 1 | g) end;` still answers `"base"`, never
   `42`. Arity-scoped: a dependency `g/1` alongside an own `g/0` leaves both reachable
   from the same body (`["own0","dep1arg"]`).
5. **A parameter beats both.** `def f(g): g; def q: f(7);` answers `7` even with a
   dependency or a sibling named `g` in scope, and `def f($g): [$g, g]` answers `[7,7]`
   — a `$`-spelled parameter binds the bare call-site namespace too.

6. **A module's own def keeps the bindings it was written under**, whatever the def
   that calls it declares. `def f: 1; def k: f; def h(f): k;` answers `1` for `h(99)` —
   `k`'s `f` is the module's, not `h`'s parameter. `def h: "first"; def g: h; def h:
   "second-" + g;` answers `"second-first"` — `g`'s `h` is the first one. `def a: length;
   def h(length): a;` answers `2` for `h(9)` on `[1,2]` — `a`'s `length` is the builtin.
7. **...including when the name is defined nowhere.** `def a: b; def h(b): a;` is
   `b/0 is not defined`, exit 3, not something `h`'s parameter can satisfy.

Rule 4 is why an exported def's body gets *no* forwarding stub for a dependency matching
its own (name, arity), and rule 5 is why it gets none for one matching any of its
parameters — both scoped to a name the body calls *directly*, since a dependency reached
only through another one is bound where that one was written (inside its own linked run,
since #2955) and is not the def's own to shadow. Rules 6 and 7 are why a module's own defs
are never nested inside each other: they are emitted as siblings in the chain they are
exported into — a top-level run, or the module's linked run — exactly as a filter's own
defs are, and jq's lexical rule relates them there. Nesting a copy of one inside another's
body puts it under scopes it was never written in, and rule 6's third row shows sealing
cannot repair that — a call to a builtin is free in every pre-bound form of the copy.
Rules 1-3 are why the stubs go around each exported def's **body** rather than the
dependency being spliced into the module's exported chain.

One consequence worth stating, since it is the reason a dependency's body is resolved
inside its own module's run and nowhere else: a def handed to another module has to keep
its **own** bindings. With `inner.jq` = `def g: 42; def k: g;` and `outer.jq` =
`include "inner"; def g: k;`, jq answers `42` — `k`'s `g` is `inner`'s. Leaving `k`'s `g`
to resolve outward into wherever `k` gets called would instead find `outer`'s own `g`,
which is `k`.

### Deeply chained modules compounded in memory — closed (#2955)

Binding a module's dependencies by **copying** their bodies into every def that reached
them compounded down a chain: each level's bodies already carried the level below, so a
chain whose defs each call `F` defs from the level below held `F^L` copies of the bottom
one. jq binds symbolically and shares its blocks. Closed by linking each dependency module
**once**, as a run of its own with a hidden alias, and reaching it through forwarding stubs
— [ADR-0023](../../adrs/adr-0023.md)'s #2955 amendment. Measured before and after (Apple
M-series, release, `/usr/bin/time -l`, the issue's own generator, output identical to jq
on every row):

| chain                            | before | after  | jq     |
|----------------------------------|--------|--------|--------|
| 6 levels x 40 defs, 1 call each  | 12 MB  | 9 MB   | 2 MB   |
| 8 levels x 6 defs, 3 calls each  | 142 MB | 34 MB  | 2 MB   |
| 12 levels x 4 defs, 2 calls each | 162 MB | 36 MB  | 2 MB   |
| 14 levels x 4 defs, 2 calls each | 681 MB | 112 MB | 2 MB   |

The 14-level row also went from 0.38 s to 0.03 s. The processed program is now linear in
chain depth — `link_size_guard_2955` (`src/bin/succinctly/jq_runner.rs`) counts its nodes
at 6, 8 and 10 levels and asserts a constant per-level delta; against the copying loader it
read 1253 / 5093 / 20453. `test_fan_out_module_chain_stays_linear_2955`
(`tests/jq_cli_tests.rs`) runs the issue's two shapes end to end.

What remains above jq's 2 MB is the **evaluator's**, not the loader's, and it needs no
module to appear: a `DefCall` node caches its bound body, so a call tree of `F^L` calls
leaves `F^L` cached copies behind for the program's lifetime. The same 56 defs written in
one file cost 58 MB, `def fib(n): if n < 2 then n else fib(n-1) + fib(n-2) end; fib(20)`
costs 234 MB against jq's 2 MB, and `fib(21)` overflows the stack. Filed as
[#3148](https://github.com/rust-works/succinctly/issues/3148).

**One shape pays for the linking:** a wide chain of which the filter uses *everything*. With
20 modules of 50 defs each including the one below, `include "s19"; s19_0` starts in 9 ms
(20 ms before), but a filter naming all 50 top-level defs links all 950 dependency defs into
the top-level chain and starts in 136 ms (32 ms before; Apple M4 Pro, idle, interleaved
medians of 25 reps). A module-heavy loop, `[range(1e5) | f]` with `f` calling one
dependency through a stub, is neutral within noise (+1.6%, against +3.3% drift on the same
defs written inline).
Each chain def is installed over the whole program below it when bound, so the chain's
length is quadratic at startup -- the same pre-existing evaluator cost a single 3000-def
`include` pays today (1 GB, 0.5 s; also #3148) -- where the copying loader had kept those
bodies nested inside the defs that used them. Only the defs the filter reaches are linked, which is what
keeps the common shapes at or below their old cost.

### A wrapped dependency sits inside the including def's scope — closed (#2962)

A module's dependencies are bound by wrapping them around the body of each def that
reaches them, and that wrap nests **inside** the def's own `Expr::FuncDef`, so the def's
own name (for self-recursion) and its parameters used to be enclosing binders for every
dependency in the block. Real jq binds a module's block in its own scope and only then
links it, so nothing of the caller is ever in scope for it. Three shapes, confirmed live
against jq 1.7.1, with what `succinctly jq` answered before #2962:

| fixtures | jq 1.7.1 | succinctly, before |
|----------|----------|--------------------|
| `inner` = `def c: 7; def g: c;`, `mid` = `include "inner"; def c: if . == 0 then g else (. - 1 \| c) end;`, then `0 \| c` | `7` | `g/0 exceeded maximum recursion depth`, exit 5 |
| `gb` = `def g: b;`, `hb` = `include "gb"; def h(b): g; def q: h(99);`, then `q` | `b/0 is not defined`, exit 3 | `99`, exit 0 |
| `inner3` = `def g: 42; def k: [g];`, `h3` = `include "inner3"; def h($g): [g, k]; def q: h(7);`, then `q` | `[7,[42]]` | `[7,[7]]` |

**Closed** first by wrapping each dependency group as a flooring run (the #2951 marker, so
a dependency body sees nothing of the def it is wrapped into, for `$variables` as well as
calls) with a clashing dependency **renamed** rather than excluded, and since #2955 by not
wrapping dependency bodies into defs at all: a dependency module is linked once, in its own
run, and a def reaches it through forwarding stubs that carry no free name to capture. The
exclusion was what produced the first and third rows: it kept the def's own binding for the
body but stranded the other dependency that called the excluded one; under linking, that
other dependency's call is answered inside its own module.
[ADR-0023](../../adrs/adr-0023.md)'s #2962 and #2955 amendments record both mechanisms.
`test_dependencies_are_bound_in_their_own_scope_2962` (`tests/jq_cli_tests.rs`) pins 30
rows, each byte for byte against jq 1.7.1 (stdout, stderr and exit code): these three, the
scope leaks the floor closes (a parameter, a sibling origin's group, a `$`-parameter), and
the cases binding must respect (a nested def or parameter of the same name, a dependency's
own dependency, a later same-name entry, and declaration order).

A module body's unbound `$variable` is now also reported at the module's own file, line
and source, as jq reports it and as #2991 already did for calls.

### A dependency's compile error was reported once per copy — closed (#3058)

A dependency reached from two defs was copied into both, and each copy's body was checked:
`jq: 2 compile errors`, the second without a line. Since #2955 the body exists once and is
checked once, so `include "mid"; a, b` with `mid` = `include "dep"; def a: g; def b: g;` and
`dep` = `def g: $nosuch;` reports jq's one error. Cross-module errors also come out in jq
1.7.1's order now — a dependency's before its includer's, the last-declared dependency's
first, a chain's deepest first — because that is the order the linked runs are wrapped in.
Both pinned in `tests/jq_cli_tests.rs` (`_3058`, `_2955`).

**Residual:** a module that is both a top-level `include` and another module's dependency
has two copies (its top-level run and its linked run), so a body error in it is reported
twice when the main filter reaches *both* — `include "dep"; include "mid"; [h, bad]` with
`dep` = `def bad: nosuch;` and `mid` = `include "dep"; def h: bad;` prints
`jq: 2 compile errors` where jq prints one. Reaching only the linked copy (`[h, k]`) is one
report, since an unreached body is never checked (#2740).

### Module-scope gaps that are genuinely open

Found while closing #2865, filed rather than recorded as divergences: a dependency named
after a builtin cannot shadow that builtin *inside* the module body, because a module's
own source is parsed with no shadow-candidate seeding
([#2950](https://github.com/rust-works/succinctly/issues/2950)). Data imports
(`import "f" as $d;`) are also still unimplemented: #2865 records the `$` on `Import::data`
and resolves the file so a typo is still jq's own `module not found`, but binding the
variable is [#2956](https://github.com/rust-works/succinctly/issues/2956).

**A module body seeing names it should not** — `~/.jq`'s defs, and sibling `include`d and
`import`ed modules' defs in a declaration-order-dependent way — **is closed**
([#2951](https://github.com/rust-works/succinctly/issues/2951)). The loader now brackets
every run of defs it wraps between two marker defs whose names begin with a NUL byte, and
`resolve.rs` treats an unmatched begin marker as a scope floor. See
[ADR-0023](../../adrs/adr-0023.md) for why the boundary is encoded that way rather than as
a field on `Expr::FuncDef`, a side table, or name mangling.

What remained of that area was diagnostic detail, not scope: a module-body compile error
named the module's own canonical file, exactly as jq does, but not jq's trailing
`, line N:` or its echo of the offending source line. Closed by
[#2991](https://github.com/rust-works/succinctly/issues/2991). Two further pre-existing
divergences in the same reporter are unmeasured and unfiled as their own oracle matrices:
jq reports module errors in *reverse* include order, and a main-*body* error suppresses
def errors entirely.

### `--slurpfile`'s malformed-JSON detail text is succinctly's own, not jq's (#3051) — accepted divergence

A bad `--slurpfile`/`--rawfile`/`--argjson` argument now exits 2 (jq's usage-error code) with
a single `jq: ...` line instead of routing through `anyhow`'s generic exit-1 `Error: .../Caused
by:` block ([#3051](https://github.com/rust-works/succinctly/issues/3051)). The wrapper
wording matches jq 1.7.1 byte-for-byte, confirmed live, for a missing file on either flag and
for a malformed `--argjson` value:

```console
$ jq  -nc 1 --slurpfile x /nonexistent    # jq: Bad JSON in --slurpfile x /nonexistent: Could not open /nonexistent: No such file or directory
$ sjq -nc 1 --slurpfile x /nonexistent    # byte-identical
$ jq  -nc 1 --rawfile x /nonexistent      # jq: Bad JSON in --rawfile x /nonexistent: Could not open /nonexistent: No such file or directory
$ sjq -nc 1 --rawfile x /nonexistent      # byte-identical
$ jq  -nc 1 --argjson x '[1,'             # jq: invalid JSON text passed to --argjson  (+ usage-hint trailer)
$ sjq -nc 1 --argjson x '[1,'             # byte-identical
```

Only `--slurpfile`'s own malformed-JSON *inner* detail (after the wrapper's second `: `)
diverges:

```console
$ jq  -nc 1 --slurpfile x t1.json   # jq: Bad JSON in --slurpfile x t1.json: Unfinished JSON term at EOF at line 2, column 5
$ sjq -nc 1 --slurpfile x t1.json   # jq: Bad JSON in --slurpfile x t1.json: Invalid JSON in stream: EOF while parsing a value at line 2 column 5
```

jq's own line/column diagnostic comes from its own hand-written JSON reader; succinctly's
`Invalid JSON in stream: <serde_json detail>` comes from `anyhow`'s context chain over
`serde_json`'s own `Display` for the same failure (`parse_json_stream`'s
`.context("Invalid JSON in stream")` — the chain is preserved via `anyhow::Error`'s alternate
`{:#}` Display, not dropped). Reproducing jq's exact wording there would mean re-deriving its
parser's own error-position/message rules for this one CLI-arg error path — out of proportion
to this issue's own "Low severity" scope, which was the exit code and the reporting channel,
not this detail text.

`--rawfile`'s "Could not open" wrapper also covers invalid-UTF-8 file content (`std::io::Error`'s
`ErrorKind::InvalidData`, since this crate reads the argument as a Rust `String`) — unverifiable
against jq, which reads raw bytes and never rejects invalid UTF-8 there at all (confirmed live:
`jq -nc 1 --rawfile x <file with invalid UTF-8>` exits 0, the content silently unused by a filter
that never reads `$x`). Not the scope of this issue either: it is a pre-existing limitation of
this crate's whole `--rawfile`/`--slurpfile` pipeline being built on `String`, not something
#3051 introduces or narrows.

`--jsonargs`, `-f`/`--from-file`, and the main input file had the same exit-1 bug (confirmed
live: jq exits 2 for a bad `--jsonargs` value, an unreadable filter file, and an unreadable
main input file) and are now fixed too. [#3096](https://github.com/rust-works/succinctly/issues/3096)
routes a bad `--jsonargs` value through the exact `--argjson` wording above (same
`invalid JSON text passed to --<flag>` line and usage-hint trailer); [#3098](https://github.com/rust-works/succinctly/issues/3098)
gives `-f` its own `jq: Could not open <path>: <detail>` shape and the main input file its
own `jq: error: Could not open file <path>: <detail>` shape. All five wordings confirmed live
against jq 1.7.1 — the main input-file one deliberately differs from `-f`'s (no `error:`
prefix or `file` noun there) and from `--slurpfile`/`--rawfile`'s wrapper above, so none of
their message shapes was reusable for it.

Two jq behaviors around an unreadable main input remain unmatched here, both pre-existing and
orthogonal to the exit-code class above: jq reports *every* file it cannot open (one `jq: error:
Could not open file ...` line per missing file) and keeps processing the files that did open,
where succinctly stops at the first failure — `sjq -c . missing valid` prints the one error and
exits 2 with no output, where `jq -c . missing valid` at the same exit code still prints the
filtered content of `valid` after its error line. Under `-s` the same gap lets `jq` slurp the
readable files into output (`[]` when none opened) while still exiting 2; succinctly emits no
output. Both are unfiled follow-ups to the exit-code change, not part of its scope.

## Bare `..`/`recurse` on a `$var` off the register: a nested-fold refuse-only residual (#3048)

`resolve_node_sink`'s bare-recursion arm (#2761) emits the identity seed of an untracked
value, then raises `invalid_path_expression_near_iterate` if the continuation discards
that seed and asks for more — matching jq's own `def r: ., (f | r); r;` unfolding of
`.[]?` at its own path register. Before #3048 this arm only fired when the value was
*plainly* untracked (`!snapshot.is_marked()`); an untracked value that still carried a
[`Snapshot`](../../../src/jq/eval.rs) mark — a `$var` bound off the register, which
`resolve_node_sink`'s `TrackedVar` arm could not certify against the current position —
fell through to `resolve_recursive_descent_sink` instead, whose plain structural descent
performs no iterate check at all. That let `del()`/`|=`/`=` succeed as silent no-ops
where jq exits 5:

```console
$ echo '{"a":[1],"c":1}' | jq 'del(.a as $x | $x | .. | select(false))'
jq: error (at <stdin>:1): Invalid path expression near attempt to iterate through [1]
$ echo '{"a":[1],"c":1}' | succinctly jq 'del(.a as $x | $x | .. | select(false))'
{"a":[1],"c":1}
```

#3048 widens the arm's guard to `!trackable` alone (dropping `!snapshot.is_marked()`):
a marked-but-untracked `$var` now refuses through the same path a plain untracked value
already did, `recurse_family_root_seed` still passing `snapshot` through unchanged so the
emitted seed keeps its mark (a bound consumer can still stop on it, and an outer fold
register can still recognise it) — only the *descent past* that seed is refused. A
`trackable` input is unaffected: it keeps the fast structural descent
`resolve_recursive_descent_sink` was already correct for. yq mode is unaffected too —
its own eager `recurse_untracked_error` guard (#843/#1591) already excludes a marked
snapshot from refusing here for a different, still-valid reason: real yq's lexer rejects
`recurse(f)` outright, so a marked `$var` reaching the walk below and correctly re-emitting
its own mark is the intended behaviour there, not this bug.

**One residual, refuse-only row remains**, unclosed by this fix and out of its scope
(confirmed live against jq 1.7.1, `main` `af43dc42b`):

```console
$ echo '{"a":1}' | jq 'path(. as $x | foreach (1) as $i (0; 5; foreach (1) as $j (0; 6; $x | .. | select(false))))'
                                                                                                                    # no output, exit 0
$ echo '{"a":1}' | succinctly jq 'path(. as $x | foreach (1) as $i (0; 5; foreach (1) as $j (0; 6; $x | .. | select(false))))'
jq: error (at <stdin>:1): Invalid path expression near attempt to iterate through {"a":1}   # exit 5
```

Two nested `foreach` folds each hold their own copy of `$x`'s register; by the time the
inner fold's extract runs, jq's *actual* current register has cycled back around to where
`$x` was bound, so jq accepts. Nothing at the point of the iterate check here can see
either fold's own register state — the same limitation `recurse(.[]?)`'s own eager guard
already has (#843) — so the fixed `..`/`recurse` refuses where jq would accept. `..` used
to get this row right *by accident*, because it deferred everything to the (never-checking)
structural descent this issue closes; `recurse(.[]?)` already refused it before this fix
and still does after. Closing this residual needs the iterate check to see the outer fold
register, which is out of scope here — tracked as a follow-up alongside #3048 rather than
opened as its own issue, since no design for surfacing fold-register state to a
`resolve_node_sink` arm exists yet.

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

## ANSI color bytes: succinctly emits one SGR per token, jq emits redundant repeats (#3110)

`-C` output wraps the same tokens in the same colors as real jq -- including, since #3110,
the `:` in the object color and the `,` in its enclosing container's color (verified live
against jq 1.7.1 with a custom `JQ_COLORS` that distinguishes object/array colors). The one
byte-level difference is that jq redundantly emits the SGR a second time immediately before
a closing delimiter (`\x1b[1;39m\x1b[1;39m}` for `}`/`]`), a quirk of its
color-of-every-token-with-conditional-reset writer; succinctly emits each SGR once. Renders
are identical.

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
