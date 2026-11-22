# Differential bug hunting

[Home](../../) > [Docs](../) > [Guides](./) > Differential bug hunting

A **bug hunt** is a time-boxed search for unknown bugs in `succinctly jq` (or `succinctly yq`). It compares the tool
with its reference implementation, and with itself, on generated input, then turns each confirmed difference into an
issue, a fix and a pinning test. It is not the regression suite: the suite and `succinctly dev compare jq` re-check what
is already known, and a hunt looks for what nobody has written a test for yet.

This guide describes the method of the hunt run on 2026-11-21 and 2026-11-22 before a release. Its filing log lists
133 issues (#7425 to #7700); 130 of them concern `succinctly jq` once one duplicate and two issues about the library
and the other subcommands are set aside, and fixers filed a few more directly (#7575, #7576). Of the 129 jq issues the
third verification pass checked on `main` at `b7e58eb2a` (2026-11-22 09:07 UTC), 99 were fixed and 9 were recorded as
deliberate divergences. The guide is written for whoever runs the next hunt, alone or as a coordinator of several
agents. Read it once top to bottom; the later sections are reference.

- [When to run a hunt](#when-to-run-a-hunt)
- [What counts as a finding](#what-counts-as-a-finding)
- [The known-set check](#the-known-set-check)
- [Strategies](#strategies)
- [Coverage map](#coverage-map)
- [Territories and the hunter's report](#territories-and-the-hunters-report)
- [Harness rules](#harness-rules)
- [From finding to merged fix](#from-finding-to-merged-fix)
- [Coordinating several agents](#coordinating-several-agents)
- [Running the next hunt: checklist](#running-the-next-hunt-checklist)

## When to run a hunt

Run one when the board has run dry of reported bugs but you have no evidence the tool is clean, and above all before a
release. The 2026-11-21 hunt started after sweeps that had all reported zero differences: 96,000 grammar-fuzz
programs, 4,999 real JSON files under 10 filters and 20 option sets, the output formats over random arrays, the JSON
test suite and jq's golden cases. In two days it found 130 jq issues, nearly all in dimensions those sweeps did not
generate. A clean sweep says only that the shapes it generates are clean.

Run another after a large batch of fixes has merged. More than 100 PRs merged during this hunt, many of them in the same
input routes, and fixes interact: #7570 was a new bug in the fix for #7455, found by a later hunter, not by its own
tests.

Do not hunt in the last hours before a release. A finding then has no time to be fixed and verified; see
[Stopping rules](#stopping-rules).

## What counts as a finding

### The four classes

A finding is any one of:

1. **A difference from the reference** in stdout, in the text of stderr, or in the exit status, on the same command line,
   standard input, environment and working directory. "jq rejects it and we accept it" is a difference, and so is the
   reverse: a reference error is behaviour too.
2. **A crash, hang or runaway memory** in `succinctly`: a panic (exit 101), an abort (134), a native stack overflow, a
   signal, no answer in many times the reference's time, or peak memory growing with the input on a route documented
   as flat. What the reference does is irrelevant to this class.
3. **Two routes or two builds disagreeing.** The same data read from a file, a pipe, `/dev/fd/N`, split across files,
   under `-n inputs` or through the windowed route; or the same case on the `cli` build, the `cli,simd` build and a
   debug build. A disagreement is a bug even before you know which side is wrong: reduce the case and run the reference
   on the small one to say which.
4. **Wrong at scale.** A correct answer on 1 MB that becomes wrong, refused or quadratic at 2^31 or 2^32 bytes, lines
   or elements, or memory far beyond what [limitations.md](../compliance/jq/limitations.md) documents.

### The reference

`succinctly jq` follows jq 1.7.1 bug for bug ([ADR-0018](../adrs/adr-0018.md)); on macOS that is `/usr/bin/jq`
(`jq-1.7.1-apple`). The repository owner added a rule during this hunt: on every difference from 1.7.1, also run jq
1.8.2, the latest release, and record all three outputs.

- **1.8.2 agrees with 1.7.1:** 1.7.1 is the reference, as before.
- **1.8.2 differs because upstream fixed a 1.7.1 bug** (a crash, a wrong answer, an inconsistency their NEWS, commit or
  PR calls a fix): adopt 1.8.2. Keep the 1.7.1 golden and list the row as `newer_than_primary` in
  `tests/data/jq-golden-known-failures.txt`, and record the difference from the pin in limitations.md. Examples:
  negated literals (#7453), `last(empty)` (#7518), NaN with a payload (#7522, a CVE fix), `--indent 0` (#7605).
- **1.8.2 differs by a deliberate feature or language change:** 1.7.1 stays the reference. The reworded value
  previews in type errors (jqlang/jq#3478) are one.
- **1.8.2's hard limits that protect its C implementation** (the 10,000-level caps on paths, equality, comparison,
  containment and merge; `String too long` at 2^31 bytes; the 1 GiB token and 999,999,999-digit limits; the 4,095
  parameter limit; `Repeat string result too long`): not adopted, by the owner's decision. `succinctly jq` answers
  those inputs correctly and each is a recorded divergence (#7526, #7527, #7540, #7541, #7500).
- **The evidence is unclear:** say so, quote what upstream says, and leave the decision to the owner rather than guess.

Decide from upstream evidence, never from which answer looks better: jq's `NEWS.md` at tag `jq-1.8.2`, the source at
both tags, and the commit or PR that changed the behaviour (`gh api repos/jqlang/jq/...`). The 1.8.2 test files are
vendored in `comparison/jq/vendor/jq-1.8.2/`.

`succinctly yq` follows yq v4.53.3 the same way; see [For a yq hunt](#for-a-yq-hunt).

### Capture, never recall

Never state what jq does from memory: run the pinned binary and quote its output. That applies to "jq has no such
feature" as much as to any output. Some behaviour is a property of the platform, not of the version: the last digit
of a math builtin comes from the platform libm ([limitations.md](../compliance/jq/limitations.md#the-math-builtins-print-the-platform-libms-last-bit-as-jq-does-3045)),
and jq 1.7.1's casts from double to int are undefined behaviour that x86_64 and aarch64 answer differently
(`halt_error(2147483648)` exits 0 on x86_64 Linux and 255 on aarch64, #7561). Capture such a behaviour on each
platform it can differ on, and check that a Linux box's `jq` really is 1.7.1: a distribution `jq` may be 1.6.

## The known-set check

Before a candidate is written up, check it against everything already known. Every source, every time:

- [docs/compliance/jq/limitations.md](../compliance/jq/limitations.md): the deliberate divergences and the documented
  gaps. It is long (15,834 lines at `47c652241`), so search it: `git grep -n -i <keyword> -- docs/compliance/jq/limitations.md`.
  Read the lazy-validation sections in particular ("Malformed text a filter never reads is never diagnosed", #3035, and
  the #2103 and #2261 entries): a semi-index validates only what a filter reads, and much of what a malformed-input
  fuzzer reports is that rule.
- The ledger, `comparison/jq/divergences/**/*.toml` (one file per entry; 65 entries at `47c652241`), and its `reason`
  fields.
- The known-failure lists: `tests/data/jq-error-known-divergences.txt`, `tests/data/jq-golden-known-failures.txt`,
  `tests/data/json-test-suite-known-failures.txt` and `tests/data/json-test-suite-i-decisions.txt`.
- Issues, **open and closed**: `gh issue list -R rust-works/succinctly --state all --search '<keywords> in:title,body'`.
- The hunt's own record: the findings other hunters already wrote and the issues filed so far in this hunt.
- Standing facts: the semi-index validates lazily; jq 1.8-only builtins and documented extensions (`leaf_paths`,
  `isvalid`, `at_offset`, `@dsv`, `@yaml`, `@props`) are not differences; compile-error *wording* is a recorded
  divergence (#2703), so for exit 3 compare the exit status and the number of errors, not the text.

The check is the cheapest step and the one that saves the most. A duplicate costs a filer's verification, then a
fixer's reproduction and design time, then a closed-as-duplicate PR. Of the first 115 findings, 5 were rejected at
filing (a documented extension, a case that already matched 1.8.2, three variants of filed issues), and of 29
follow-ups that fixers reported, 8 were rejected as documented or already in progress. A complete check by
whoever found them would have stopped each one earlier. One duplicate issue was still filed (#7700, a duplicate of
#7699).

## Strategies

Each strategy below says what it generates, what it compares, and what it found. The
[coverage map](#coverage-map) has the volumes. A strategy that finds nothing still produces a result: record the case
count, so the next hunt does not repeat it.

| Strategy                       | Generates                                                         | Compares                      | Example issue | Yield                                                                          |
|--------------------------------|-------------------------------------------------------------------|-------------------------------|---------------|--------------------------------------------------------------------------------|
| Grammar and mutation fuzzing   | programs from a grammar, or token edits and splices of seeds      | against jq                    | #7557         | 5 parser bugs from 146k one-edit mutants; 11 issues from 4.2M two-edit mutants |
| Hostile values                 | ordinary programs over extreme values, as input and as literals   | against jq; input vs literal  | #7453         | 6 issues                                                                       |
| Route differentials            | the same data through every input route                           | route against route, and jq   | #7446         | 6 issues (first pass), 7 (`--stream` pass)                                     |
| Fault shapes and positions     | one fault per container shape, or one fault moved across offsets  | reader against reader, and jq | #7554         | 3 issues; window-seam sweep 0 of 3,242                                         |
| Build differentials            | the same cases on `cli`, `cli,simd` and debug builds              | build against build           | #7482         | 2 issues; cli vs simd never differed                                           |
| Platform differentials         | the same corpus on Linux x86_64 and macOS arm64                   | platform against platform     | #7486         | 8 issues from one x86_64 Linux pass                                            |
| Scale tests                    | inputs past 2^31 and 2^32 bytes, lines and elements               | against jq, or route vs route | #7425         | 7 issues                                                                       |
| Resource limits                | runs under `ulimit -v`, `-n`, a full disk, a native-stack floor   | against jq                    | #7477         | 4 issues (#7476, #7477, #7502, #7576)                                          |
| Upstream fixes and test suites | one probe per entry of jq's NEWS; jq's own test files             | 1.7.1 vs 1.8.2 vs succinctly  | #7519         | 21 issues from about 140 upstream entries                                      |
| Real-world programs            | programs people wrote (Rosetta Code, jq wiki, public `.jq` files) | against jq, time included     | #7460         | 8 issues from about 33,000 runs                                                |
| Complexity probes              | one program at three or more sizes                                | growth against jq's growth    | #7563         | 12 quadratic-or-worse issues across the strategies                             |
| Verification passes            | every filed reproduction                                          | against the target reference  | #7448         | 0 regressions; 3 incomplete fixes completed by follow-up PRs                   |

### Grammar and mutation fuzzing

A grammar fuzzer generates well-formed programs from a model of the language and runs them against jq.
`scripts/jq-grammar-fuzz.py` is the repo's (the `fuzz` dimension of `dev compare jq`); with
`scripts/jq-pattern-fanout-fuzz.py` and `scripts/jq-bind-origin-fuzz.py` it had found nothing in 96,000 programs before
the hunt, and 85,500 more on a debug build during it found nothing again. A grammar only generates what its author
modelled, so the yield came from mutation instead:

- **Byte insertion:** every byte 0x01-0x7f, and 23 multi-byte and control sequences (NBSP, U+2028, BOM, CRLF, ...),
  inserted at every position of 37 seed programs, passed as an argument and through `-f` (74,504 and 27,186 programs).
  It found `.@base64 "x"` refused where jq treats it as a field access (#7456).
- **One token edit** (delete, duplicate, swap, replace or insert one of 78 vocabulary items at each token of 86 seeds,
  146,423 mutants): five parser bugs, among them `def f(a,b)` accepted (#7457), `$__loc__` accepted as a label (#7458)
  and `3end` refused (#7496).
- **Two token edits and splices** (a sub-expression of one seed inserted into another), seeded with the programs of
  jq's own test files for 1.7.1 and 1.8.2 and the real-world corpus (9,128 seeds), each mutant run with `-n`, its seed's
  input and three generic inputs: 4.2 million mutants and 301,000 option-and-route mutants on an x86_64 Linux box gave
  11 issues, among them `break $f.a` refused (#7557), `.[-2][]? = 0` raising early (#7559) and `map(f) | map(g)` running
  `g` before `f` has finished (#7565).

Bucket the differences by a signature (the pair of exit statuses, the first differing token class, the normalised
error text) and investigate the largest buckets first. Run mutants with a short timeout on the reference (2 s) and a
longer one on `succinctly` (10 s), and re-run a one-sided timeout alone before believing it.

For builtins, generate per **family** (dates, number formatting, strings, collections, paths, control flow) and spend
the volume on the families with no findings yet, at hundreds of thousands of cases per family. Regex builtins are a
separate territory with their own long tail (#7689).

### Hostile values

Keep the programs simple and make the values hostile: every builtin of `tests/data/jq-builtin-names.txt` at each arity,
and every operator, over a pool of about 190 values. The pool covered number spellings at every limit (`-0`, `1e1000`,
`5e-324`, 17+ significant digits, integers around 2^31, 2^53, 2^63, 2^64), strings with NUL, lone surrogates, invalid
UTF-8 and non-BMP characters, empty, deep and duplicate-keyed containers. Feed each value three ways: as input (the
document route), as a program literal and through `--jsonargs` (the owned route). A value that behaves differently as
input and as a literal is a strong lead: that is how `-1 == -1.00000000000000000001` was found to compare the literal
exactly (#7453), and how a NUL in a key was found printed whole in an error message (#7450).

### Route differentials

`succinctly jq` reads input through several routes: the whole-file semi-index, the windowed route for large files,
lists of files and pipes, the lazy `input`/`inputs` reader, `-R` raw lines, `--seq`, `--stream`, `--slurp`, the
streaming fast path for a plain filter, and path-mode evaluation for writes. The same bytes and program must give the
same stdout, stderr (file names normalised) and exit status on every route, so a route differential needs no reference
to find a bug. This was the hunt's richest vein.

Cross four dimensions:

- **Delivery:** a named file, `< file`, a pipe, `/dev/fd/N`, `/dev/stdin`, a FIFO, `-` among named files, the same file
  twice, standard input at a non-zero offset, the same bytes split across 2 to 40 files at every kind of boundary
  (inside a token, between documents, inside a BOM).
- **Program:** about 40 that read different amounts (`.`, `.a`, `.[0]`, `first`, `length`, `-n input`, `[inputs]`,
  `first(inputs)`, `limit(1; inputs)`, `input_line_number`, `input_filename`, `halt`, `error` at document K, `tostream`).
- **Flags:** `-c -r -j -a -S -s -n -R -e --seq --stream --stream-errors --tab --indent N`, `--arg` and its family, in
  combinations of two and three.
- **Size and fault position:** sizes just below, at and above each route threshold, with any fault placed at a window
  or piece boundary and at plus or minus one byte. Find the thresholds in the source, not from memory: the window
  (`WINDOW_BYTES` in `src/bin/succinctly/jq_runner/window.rs`, 256 KiB since #7412), the size at which a route switches
  to windows (4 MiB for most routes, 64 KiB for a lone `--seq` file since #7260; limitations.md lists each), the lazy
  reader's prefix (`LAZY_PREFIX_BYTES` in `jq_runner.rs`, 256 KiB), the 32-file hand-off, the `--seq` record cap, and
  jq's own 4,095-byte read chunk, which decides what `input_line_number` and `(at FILE:N)` report.

A debug build reads a smaller window from `SUCCINCTLY_DEBUG_JQ_WINDOW_BYTES` (8192 to 1048576), so a debug hunt can
cross window boundaries with inputs of tens of KiB ([developer.md](developer.md#testing)). The release window is a
constant: re-check a debug-window finding on a release build at full size.

Also test what happens to the descriptors around a run, which a single-run differential cannot see: whether a
redirected standard input is left where jq leaves it after an early exit (`{ jq -n input; jq -n input; } < f`, #7455),
and whether a file that grows while it is read is read to its new end (#7446).

The routes pass found 6 issues, among them lines appended during a run dropped (#7446), the stdin offset after an early
exit (#7455), and about 23 builtins running a generator argument to its end before their first output (#7474). The
`--stream` pass (inputs split across 2 to 40 files, 5 to 8 MiB streams, 36 fault classes, 27 flag sets) found 7 more,
among them a BOM split across the first files (#7566) and the new #7570.

### Fault shapes and fault positions

A semi-index validates what a filter reads, so its fast paths can answer from text a slower reader would reject. Map
this systematically rather than by example: every one-fault container shape (`[1,]`, `[,1]`, `[1 2]`, `{"a":1,}`,
`{"a" 1}`, `{"a":1 "b":2}`, at depth 1 to 3, as an NDJSON line, as an element of a 5 MiB array) against every reader
that could take a short cut (`first`, `last`, `.[0]`, `nth(0)`, `any`, `all`, `has`, `length`, `keys`, `getpath`,
`to_entries`, `.a`, ...). The documented lazy rule explains most cells. What matters is an inconsistency it does not
explain: two readers of the same element that disagree, or a value taken from across the fault. About 37,000 small and
19,000 large runs found three: `[,1] | first` printing `1` where `.[0]` raises (#7554), `all(f)` and `any(f)` answering
across a gap where `all(.[]; f)` raises (#7555), and an absent-key lookup ignoring a missing separator that `has`
reports (#7556).

A **fault-position sweep** moves one malformed byte, or one window boundary, across every offset of a document. The
window-seam sweep placed 37 valid values so a 256 KiB window boundary fell at every byte inside them, and 30 faulty
tokens at 5 offsets each: 0 of 3,242 cases differed. Invalid UTF-8 at jq's 4095, 8190 and 12285 read boundaries gave 0
of 5,184. A stdin-offset fuzz (where a run that stops early leaves a redirected standard input, 1.77 million cases) is
the net for #7570, a stop on an event completed by the last byte of jq's 4,095-byte chunk: on a build without that
fix it finds 16 differences in 12,000 cases, and with it 0 in 246,011.

### Build differentials

The shipped binary is `--features cli`. Nearly all earlier fuzzing and the whole jq comparison had run on
`cli,simd`, and a debug build turns on overflow checks and `debug_assert!`s that a release build skips. Build all three
from one commit and require identical stdout, stderr and exit status:

- **Scanner boundaries:** strings whose closing quote, backslash run (`\\\"`), escape or non-ASCII byte falls at every
  offset relative to 16-, 32- and 64-byte blocks; every document length 0 to 300 (600 on x86_64); long whitespace runs.
- **Tree navigation:** documents of 3 to 500,000 nodes in chain, wide, balanced and random shapes, sized at word and
  block boundaries, under navigation programs.
- **Mutated documents** of the JSON test suite and the golden cases, checked for panics on the debug build.
- **Every SIMD level and CPU:** AVX-512 (an AMD Ryzen 9 7950X), AVX2 without AVX-512 (an Intel i7-6700), NEON (Apple
  M-series), and the levels `SUCCINCTLY_SIMD` can force on x86_64 (`scalar`, `sse2`, `sse4.2`, `avx2`;
  [environment-variables.md](../reference/environment-variables.md#succinctly_simd)). Any difference between levels is
  a bug whatever jq says.

`cli` and `cli,simd` never disagreed in this hunt: 18,474 scanner cases on Apple silicon, 551,200 block-boundary
documents and 3.84 million mutants on the 7950X, and about 880,000 cases on the AVX2 machine. The yield came from
mutation and program size, run on all three builds: a malformed document under 44 open arrays panicked in
`src/trees/bp.rs` on every build (#7482, one hit in 45,000 mutants), and a program of 100,000 `as` bindings overflowed
the native stack (#7475; the threshold lies between 45,000 and 60,000). A fixer found
that the NEON and portable index builders disagree on one malformed token (#7539), which no CLI run had shown: when a
library-level differential is cheap, run it too.

### Platform differentials

Run the same corpus on Linux x86_64 and on macOS arm64, against jq 1.7.1 on each. One pass on an x86_64 Linux box found
8 issues, 5 of which the Macs could not show:

- **libc:** time zones `posix/...`, `right/...` and `TZDIR` read differently from glibc (#7486); `now` with nanosecond
  resolution where jq has microseconds (#7484).
- **Double-to-int conversion:** `"x" * n` for n of 2^31 or more, where jq 1.7.1 on x86_64 answers `""` (#7500);
  `halt_error(n)` exit codes (#7561). `9223372036854775808 % 5` also differs on x86_64, but jq 1.8.2 fixed it and
  succinctly already answers as 1.8.2, so it was rejected, not filed.
- **Descriptor semantics:** Linux has `/dev/full` (a stderr write failure panicked, #7476) and enforces `ulimit -v`
  (#7502); Linux opens `/dev/stdin` and `/dev/fd/N` again from offset 0, where macOS shares the open file, so a test
  of descriptor offsets has two sets of expectations (#7687).
- **Locale:** a test that passed under `LC_ALL=C` failed under `LANG=en_AU.UTF-8` (#7612). Hunt under more than one
  locale.

Math digits from the platform libm are known and recorded; do not report them.

### Scale tests

Nothing in the routine tests went past 1 GB. Generate, with a fast tool rather than a Python loop, inputs past each
32-bit limit: one document over 2^32 bytes, a string over 2^31 bytes, more than 2^32 lines, a stream past 4 GiB.
Search for the counters first (`git grep -n 'u32' -- src/json src/bin/succinctly/jq_runner.rs`) and aim an input at
each. Use cheap queries (`length`, `.[N]` past the limit, `input_line_number`, `-c .` hashed on both sides rather than
diffed in memory), and compare with succinctly's other routes where jq is too slow.

This found a 4 GiB document panicking (#7425), most programs refused on one (#7537), `input_line_number` wrapping at
2^32 (#7472), parse-error positions past 2^31 (recorded as a divergence, #7461), and memory growing with the input on
routes documented as flat (#7447, #7480). Verifying these fixes needs the same sizes: the 4.3 GB rows were checked on
the x86_64 Linux box on 2026-11-22 at `47c652241`, where jq 1.7.1 took 221 s to count 2^32 newlines (AMD Ryzen 9 7950X).

### Resource limits

Run under limits jq copes with: `ulimit -n 4` (one free descriptor; #7477), `ulimit -v` below the file size (Linux
only; #7502 and #7576), a small `RLIMIT_STACK` or `RLIMIT_NPROC`, a full disk or `/dev/full` as stdout or stderr
(#7476), stdout closed or `| head -1` (SIGPIPE), stdin closed. Programs of tens of thousands of bindings or operators
test the native-stack floor (#7435, #7475, #7676). macOS does not enforce `RLIMIT_AS`, so memory-limit rows need Linux.

### Upstream fixes and the reference's own test suites

List every upstream entry between the pinned and the latest reference (jq's `NEWS.md` for 1.8.0 to 1.8.2 and the
commit log between the tags), write one probe per entry, and run it on 1.7.1, 1.8.2 and succinctly. Classify each
entry as a bug fix, a security fix, a deliberate change or not applicable. A bug fix that succinctly does not follow is
a finding. Of about 140 entries, 27 were findings (filed as 21 issues between #7518 and #7542), about 18 were already
adopted, 8 were already filed, about 60 did not apply and 21 were deliberate changes, kept in a separate
list for a future decision to move the pin.

jq's own test files are an oracle-independent corpus. The repo runs 1.7.1's (the golden cases in
`tests/data/jq-golden/` and the vendored `comparison/jq/vendor/jq-1.7.1/`); 1.8.2's are vendored but informational. Of
the vendored 1.8.2 cases, 119 differ between the two jq versions: succinctly matched 1.8.2 on 29, 1.7.1 on 68 and
neither on 22, and each of those was accounted for. jq's shell tests (`tests/shtest`) are scripts: never run a
downloaded one; read it and re-express its cases as your own commands.

### Real-world programs

Generators do not write the programs people write. A corpus of 4,951 programs (jq's test files, the jq wiki, 2,283
Rosetta Code solutions, 1,376 public `.jq` files) and 4,845 `jq '...'` commands taken from public shell scripts and CI
files, run over a small set of generic inputs, gave about 33,000 paired runs. jq's own test cases all agreed apart
from the recorded compile-error wording, but long programs found what nothing else did: equal-length slices of one
array that jq compares as equal (#7459), recursion refused after about 8,000 sequential calls (#7460), a
shebang-polyglot comment continuation (#7481), a lost compile error (#7443), and four quadratic or worse
accumulators (#7451, #7452, #7473, #7483). Treat downloads as untrusted data: keep them in their own directory and
pass programs only as arguments.

### Complexity probes

Time one program at three or more sizes, doubling each time, on both tools. A factor of about 4 per doubling where jq's
time stays flat is quadratic; 8 is cubic. The same binary pair at three sizes on any machine is evidence of an
algorithmic bug, but label it indicative: it is not a benchmark number (see
[The performance hold](#the-performance-hold)). On a loaded machine use user CPU time or instruction counts
(`valgrind --tool=cachegrind`) instead of wall time. The probe also confirms a fix: the slope should change, not just
the constant.

A cheaper variant is **time-ratio fuzzing**: keep the mutants where succinctly takes 10 or more times jq's time. That is
how a negative-index assignment in `reduce` (#7563) and a dead `1 |= 1` branch slowing `add` 15 to 80 times (#7564)
surfaced. The quadratic-or-worse issues of this hunt were #7451, #7452, #7470, #7471, #7473, #7483, #7495, #7563,
#7564, #7567, #7610 and #7611.

### Verification passes

After fixes merge, re-run every filed reproduction against a fresh build of `main`, on all three tools, and classify
each issue as FIXED (byte-identical to the target reference), RECORDED (limitations.md names it), STILL-DIFFERS or
CANNOT-RUN (needs a size or platform the pass does not have). Keep one case script per issue so each pass re-runs the
same rows, and compare each pass with the previous one so a row that was FIXED and now differs stands out.

The hunt ran four passes: on the combined tree of unmerged fixes (107 issues, 0 regressions), on `main` at `41642530`
(86 FIXED), at `b7e58eb2a` (99 FIXED, 9 RECORDED, 10 STILL-DIFFERS, 11 CANNOT-RUN, 0 regressions), and on the x86_64
Linux box at `47c652241` for the rows that need Linux or 4 GiB inputs. The second pass found three fixes incomplete on
shapes their tests did not cover (a nested one-member object, #7448; `/dev/fd/7`, #7446; `/dev/stdin`, #7568), and each
got a follow-up PR on the closed issue: two fixes and one recorded divergence.

## Coverage map

Volumes are approximate and come from the hunters' reports; "zero" means no new difference after the known-set check.
Re-fuzzing a zero row at the same volume is unlikely to pay. A row with findings may hold more: most hunters stopped on
a time box, not on exhaustion.

### Hunted on 2026-11-21 and 2026-11-22

| Area                                                         | Approximate cases                                                    | Outcome                                                                   |
|--------------------------------------------------------------|----------------------------------------------------------------------|---------------------------------------------------------------------------|
| Expression grammar fuzz (repo fuzzers)                       | 96,000 before; 85,500 on debug; 10,000 on the combined fix tree      | zero                                                                      |
| Program lexer: byte and sequence insertion                   | 101,690 programs                                                     | #7456; the rest known (non-ASCII identifiers #7433, NUL in `-f` #7434)    |
| One-token program mutation                                   | 146,423                                                              | #7456, #7457, #7458, #7496, #7497                                         |
| Two-token mutation and splices                               | 4.2 million, plus 301,000 option and route mutants                   | 11 issues (#7557 to #7565, #7572, #7574)                                  |
| Builtin by argument-variant grid                             | about a third of the builtins                                        | the split-flags family (#7560); not finished                              |
| Hostile values through every builtin and operator            | about 3,500 batched runs; 20,000 doubles; 6,000 number pairs         | #7426, #7435, #7436, #7445, #7450, #7453                                  |
| Number spellings in input                                    | 18,048 runs                                                          | #7468, #7469                                                              |
| Malformed input mutation (22 mutation kinds)                 | about 900 documents by 32 programs, file and pipe                    | #7427, #7437, #7448, #7449, #7454                                         |
| Invalid UTF-8 in input, including at jq's read boundaries    | 13,984 runs                                                          | zero                                                                      |
| One-fault container shapes by 100 readers                    | about 37,000 small and 19,000 at 5 MiB                               | #7554, #7555, #7556                                                       |
| Window-seam sweep                                            | 3,242                                                                | zero                                                                      |
| `scripts/jq-windowed-file-fuzz.py`, all modes, new seeds     | about 43,000 over all runs                                           | zero                                                                      |
| Route differential, plain and `-n` programs                  | about 10,000 (routes, metamorphs, special stdin shapes)              | #7444, #7446, #7455, #7463, #7474, #7478                                  |
| `--stream`, `--stream-errors` and input programs over splits | about 27,500, plus 1.77 million offset and 8,100 trickled-pipe cases | #7566 to #7571, #7573                                                     |
| CLI argument matrices                                        | 3,299 rows when re-run on the fixes (first-run volume not recorded)  | #7438, #7439, #7462, #7494                                                |
| Modules, `-f` files, colours, time, environment              | about 33,000                                                         | #7428 to #7434, #7440 to #7442                                            |
| Scale past 2^31 and 2^32                                     | a few dozen multi-GB runs                                            | #7425, #7447, #7461, #7470, #7472, #7480, #7537                           |
| cli vs simd vs debug on Apple silicon (NEON)                 | 18,474 scanner, 2,358 file, 850 large-stream, 720 tree, 1,100 DSV    | #7475, #7482; builds never differed                                       |
| Debug-build panic hunt (hostile corpus, mutants, sweeps)     | 14,616 hostile jobs; 45,000 mutants; 22,000 `--seq` sweep cases      | #7482                                                                     |
| cli vs simd on x86_64 with AVX-512                           | 551,200 block-boundary, 246,800 string, 3.84 million mutants         | zero                                                                      |
| cli vs simd on x86_64 with AVX2 only                         | about 880,000, plus the three test suites                            | zero                                                                      |
| DSV input against Python's `csv`                             | 282,000 on x86_64, 76,200 on AVX2                                    | zero                                                                      |
| glibc and Linux I/O against macOS                            | 9,408 time and locale, 6,000 number, 84 I/O scenarios                | #7476, #7477, #7484, #7486, #7487, #7500, #7502                           |
| jq 1.8 upstream fixes                                        | about 140 upstream entries; the vendored 1.8.2 test files            | 21 issues                                                                 |
| Real-world programs                                          | about 33,000 paired runs                                             | 8 issues                                                                  |
| Regex builtins                                               | 632 cases in the values pass; 37,067 rows in a dedicated fuzz        | #7520, #7528, #7654, #7669, #7670, #7682; 1,401 rows still differ (#7689) |

### Hunting in progress at the time of writing

Four hunters reopened the hunt on 2026-11-22 and had not reported when this page was written (2026-11-23), so their
counts are not here. Their territories, so the next hunt can read their reports first:

- the language and builtins by family-level grammar fuzzing, regex excluded, on macOS arm64;
- routes, input shapes and CLI flags, with sizes crossing each threshold and faults at boundaries, and whether the fixes
  of the first round interact, on macOS arm64;
- jq's upstream test suites (1.7.1 and 1.8.2, including the shell tests), modules and the argument family;
- x86_64 and Linux: forced SIMD levels, glibc arithmetic and conversions, Linux I/O (terminals and character devices
  named as input, SIGPIPE disposition, `RLIMIT_NPROC`, files changing while read).

### Never covered

- **`succinctly yq`.** No hunt of this kind has run on yq mode; four yq issues were filed only in passing (#7615 to
  #7618).
- **Linux aarch64** (Graviton, Neoverse) and the SVE2 path: every arm64 run was on Apple silicon.
- **Windows** and 32-bit targets.
- **jq 1.8.2 on Linux:** no 1.8.2 binary was on the Linux box, so every 1.8.2 output came from macOS arm64, and the
  4 GiB rows were never run on 1.8.2.
- **Malformed `--slurpfile`, `--argjson` and `--rawfile` data**, and `--seq` and `-R` under heavy mutation (lightly
  covered).
- **The index budget at 2^21 keys or elements and above** (sizes 2^21 and 2^21+1 did not finish), and the memory
  findings repeated at 8 GB or on pipes with several files.
- **CPU-bound real programs against jq:** several Rosetta Code programs ran slower than jq, seen only in loaded,
  uncontrolled runs, so no ratio is quoted (one cause is #7699; its measured gap on the benchmark machines is in
  [docs/compliance/jq/limitations.md](../compliance/jq/limitations.md)).
- **The library API** apart from the CLI, except the builder differential behind #7539.

## Territories and the hunter's report

Give each hunter a **territory** that no other hunter shares: a set of dimensions (builtins, routes, platform), not a
list of issues. Disjoint territories keep two hunters from minimising the same bug in parallel and filing it twice,
and they make the coverage map possible: each zero result belongs to one territory. Split along the strategies above,
and along machines where a strategy needs one (x86_64 SIMD and glibc need an x86_64 Linux box; scale needs disk and
memory). Name in each hunter's brief what neighbouring hunters own and tell it to stay out, list what is already known
and swept at zero in its area, and give it the earlier hunters' drivers to extend rather than rewrite.

The first round used 90 to 100 minute time boxes and a cap of six findings per hunter. Four hunters stopped at the cap
(one 33 minutes into its 100), leaving time and leads unused; the later rounds dropped the cap, ran 3 to 6 hours, and
kept one finding per root cause.

A hunter does not file issues, edit the repository or push. It writes one file per finding, in this shape:

```text
TITLE: jq: <what succinctly does>, where jq 1.7.1 <does what>
SEVERITY: wrong-output | crash | hang | memory | error-text | exit-status
CONFIDENCE: confirmed | probable
## Reproduce
<exact commands, runnable from any directory, building their inputs with printf or python3>
## jq 1.7.1
<stdout, stderr, exit status>
## jq 1.8.2
<stdout, stderr, exit status, and the verdict when the two versions differ>
## succinctly jq
<stdout, stderr, exit status; the commit and the builds that reproduce; the platforms>
## Scope
<variants that share the cause; what does not reproduce; the size threshold if any>
## Suspected cause
<file and function, marked as a guess>
## Not a known divergence because
<the limitations.md keywords, ledger entries and issue searches checked, with the nearest matches>
```

Its final report lists the findings (one line each), **the areas it covered with zero differences, with case counts and
the builds used**, and the leads it did not finish. Write findings as they are confirmed, not at the end, so a hunter
cut off by a time box or a dropped connection loses nothing. Re-check every candidate on a build of current `main` (the
hunters' binaries were hours old while fixes merged), and on the other platform when the finding could be
platform-dependent.

## Harness rules

Each rule comes from an incident in this hunt.

- **Write drivers in Python, and run every case as its own subprocess with a timeout that kills the whole process
  group and waits for it.** Shell loops report wrong exit statuses, and a timeout that kills only the direct child
  leaves its children running. On the overloaded machine of the next rule, one case
  (`[limit(50; combinations(4294967297))]`) was found running at 98% CPU.
- **Cap concurrent children and reap each one.** A route hunter's driver spawned processes faster than it waited for
  them and leaked pipe descriptors: an M4 Pro Mac mini went to a load average of 31 to 33 on 12 cores and the driver
  died with `OSError: [Errno 24] Too many open files`. Four concurrent children per hunter was the working limit;
  check that load and open descriptors stay flat before scaling a driver up.
- **Cap input sizes and memory.** jq builds a full DOM: it held 4.1 GB for a 4.3 GB file, and the reproductions of the
  unbounded-read bugs (#7463, #7478) took several GB each. Give each run a size limit (500 MB to 1 GB on shared remote
  machines), watch peak RSS, and delete multi-GB inputs as soon as their runs are done.
- **Never pipe a differential run through `tail` or `head`.** Write the full output to a file and search the file. A
  truncated tail once read as "only a documented extension differs" and hid a real regression (#2658).
- **Check that the side under test took the route under test.** `scripts/jq-windowed-file-fuzz.py` used `/dev/fd/N`
  and pipes as its oracle, and those became windowed routes themselves, so for some modes it compared a route with
  itself and could not fail (#7688). Assert the input size against the route's threshold, and compare with a route that
  reads the input whole or with jq.
- **Generate the matrix; do not hand-pick it.** A hand-picked matrix holds the cases its author expected. The grammar
  fuzzer's zero over 181,500 programs and the one-token mutation's five parser bugs over 146,423 mutants come from the
  same language; only the second generated near-miss syntax.
- **Cluster by cause before minimising.** Bucket by signature, check each bucket against the known set, then minimise
  one representative per bucket. One root cause is one finding: #7474 is one issue covering about 23 builtins.
- **Minimise, then re-run the minimal case alone.** Confirm the exact stdout, stderr and exit status of both tools
  outside the loop before writing it up.
- **Expect the reference to misbehave.** jq 1.7.1 reads an uninitialised byte on a bare `n` token, so a ledger row
  diverged intermittently on Linux CI (#7391): a flaky row may be the reference. jq also aborts on some inputs (module
  cycles, `%j` in `strftime`) and hangs on others; 1.8.2 has its own bugs. A crash of the reference is not a target.
- **Exclude what cannot be compared.** `now`, random values and the clock; compile-error wording; math last digits.
  Normalise only the program name and temporary paths.
- **Never conclude anything from timing on a loaded machine.** The laptop that ran the agents (an Apple M5 Max) reached
  load averages of 191 and 277 during the hunt. Use instruction counts or leave timing to the benchmark machines.

## From finding to merged fix

### Filing

A filer reproduces each finding (all three tools, on current `main`), runs the known-set check again, and files one
issue per root cause. The issue holds:

- a title in the repo's style: `jq: <what succinctly does>, where jq 1.7.1 <does what>`;
- the exact commands, self-contained (inputs built with `printf` or `python3`);
- the three outputs (succinctly, jq 1.7.1, jq 1.8.2), the versions and the commit, and which platforms reproduce;
- the scope (variants, what does not reproduce, the size threshold);
- the jq 1.8.2 verdict and its upstream evidence when the two versions differ, or a "Decision needed" paragraph;
- why it is not already known (the searches and their nearest matches).

A variant of an existing issue goes on that issue as a comment, not as a new issue.

### One worktree and branch per issue

Each fix gets its own branch, `issue-N-<slug>`, in its own worktree outside the repository, branched from a freshly
fetched `origin/main`. Before creating it, check that nobody else has one (`git branch -a --list "*issue-N-*"`,
`gh pr list --search N`). Branch from `origin/main`, not from another unmerged fix, unless the fix depends on it; then
say "must merge after" in the hand-off. Reproduce the issue on a build of the worktree before changing code, and stop
if it does not reproduce. The test goes in `tests/jq_cli/iN.rs`, pins the reference's captured output over a generated
matrix, and must fail before the fix (prove it once). The changelog entry is a fragment in `changelog.d/`.

### Deciding what the fix is

When the reference's behaviour looks wrong, decide in [ADR-0018](../adrs/adr-0018.md)'s order: first whether a condition
lets succinctly refuse the reference's behaviour, then which option matches the pinned reference more closely, and only
then performance. In practice the outcomes were three:

- **Fix to match 1.7.1.** Lines appended to a file during a run are read, as jq reads them (#7446).
- **Adopt the newer reference.** Negated number literals compute as in jq 1.8.2, which fixed them upstream, `-0`
  spelling included; 27 pins moved to 1.8.2 and the 1.7.1 rows were listed as `newer_than_primary` (#7453).
- **Record a divergence with a pinning test.** jq 1.7.1 and 1.8.2 both say `.[:3] == .[3:]` is true for
  `[1,2,3,4,5,6]`, because their equality compares buffer pointers and ignores the slice offset; succinctly keeps
  comparing contents, and limitations.md records it with a test that pins succinctly's answer (#7459). A record follows
  [STYLE-0014](../STYLE_GUIDE.md#style-0014-one-sentence-per-line-in-the-compliance-limitation-pages): one sentence per
  source line.

A decision the rules do not settle goes to the owner with a recommendation; the fixer skips that part rather than
guess.

### Gates

Run every gate after the last edit, with cargo output redirected to a file and its exit status read directly (a pipe
hides it):

- `cargo check --all-targets --features cli,simd,regex,serde` and `cargo check --no-default-features`;
- the clippy invocations of `scripts/lib/clippy-variants.sh` for the architecture, and rustdoc with `-D warnings`;
- `scripts/fmt-per-issue.sh` and `cargo fmt --check`;
- the lib tests and the full `--test jq_cli_tests`; `yq_cli_tests` as well when shared evaluator or builtin code
  changed, because a jq fix can change yq mode;
- `python3 scripts/ledger_refs.py check`, `python3 scripts/changelog.py check`, `python3 scripts/doc_lines.py check`;
- `scripts/pre-push.sh` (`--full` for the lints CI runs).

A change that adopts jq 1.8.2 also needs CI's full ledger comparison, on Linux. The local `--ledger-only` gate finds
only *stale* entries; a new divergence that is not in the ledger fails CI's `Ledger drift` job instead (#7583 and #7584
failed that way, #7613). Before handing the branch over, run CI's Compare step on a Linux box, against source builds of
both jq versions (`scripts/compare-jq-build.sh`) and a `cli,simd,regex,serde,bench-runner` release build:

```bash
./target/release/succinctly dev compare jq --quick --dimension smoke --dimension features --dimension cli \
  --dimension differential --dimension fuzz --dimension input --dimension robustness --out data/compare/ledger
```

The fuzz dimension uses the manifest's seeds (1 to 3 in CI). An entry that only other seeds find would read as stale in
CI, so ledger only what CI's seeds find.

### The performance hold

A fix that touches code ordinary queries run (a hot path) is held until it has been timed. The fixer marks the hand-off
`HOLD-PERF` and names the queries and inputs to time and why; a timing agent clears or rejects it. The method is the
[A/B Benchmarking Method](benchmarking.md#ab-benchmarking-method):

- **Three builds from one source tarball:** A (`main`), A' (the same source built a second time, the A/A control) and B
  (A with the fix merged in), each in a directory of the same path length, built with the tree as the working
  directory so `.cargo/config.toml` applies. A and A' were byte-identical, so A'/A measures run-to-run noise (about ±2%
  median).
- **Identity first:** every cell runs once per binary, and stdout hashes and exit statuses must match before any timing
  counts.
- **Interleaved:** A, A', B and jq 1.7.1 in a rotating order within each repetition; 15 repetitions at 1 and 10 MB and
  7 at 100 MB; each block under an exclusive machine lock, with the machine's idleness recorded before each block.
- **On the benchmark machines only:** an AMD Ryzen 9 7950X (x86_64 Linux) and an Apple M4 Pro Mac mini. Never the
  machine that runs the agents, and never on battery.
- **Instruction counts as a design signal:** cachegrind on the 7950X attributes a cost (`cg_annotate`, `cg_diff`) and
  predicts whether a rework will help. It does not clear a hold: wall time does.

Outcomes in this hunt: #7563 cleared (controls flat, its target exponent 2 to 1). #7567 cost 2.4 to 3.0% on ordinary
`--stream`; it was reworked to share paths only past 64 levels (+0.17% instructions) and cleared. The NUL check of
#7437 cost 1.4 to 2.8% on whole-file reads; the first rework fixed the instruction count but not the wall time (the
extra pass was memory-bandwidth bound), and the second folded the check into the existing UTF-8 validation scan and
cleared. #7465 regressed 4.7 to 13% on the 7950X only (the M4 Pro improved); attribution traced it to glibc's
`malloc_consolidate` sweeping fastbins after a coalescing free, and the rework cleared. The `first` fix of #7554 made
bare `first` O(n) like `.[0]` (15 to 105% slower on the affected rows, still faster than jq); that is a trade-off, so
it went to the owner rather than through the hold.

### Landing

- **Serialise PRs that edit the same docs page.** Most fixes add a section to limitations.md; any two in flight
  conflict. In the first integration pass 7 of 44 candidates conflicted only there. Land them one at a time, merging
  `main` into the next after each lands (rerere can replay a keep-both resolution), and place a new section beside the
  most closely related one rather than at the top of a list every branch edits.
- **Integrate before releasing a batch.** Merge every unmerged fix into one throwaway branch and run the full suite
  there. The three passes found a test interaction between #7492 and #7501, and a branch that failed against newer
  `main` (#7489 against the stack-floor change of #7635).
- **Respect CI capacity.** macOS runners were the scarce resource: a merge queue of 20 entries thrashed on 60-minute
  timeouts and nothing merged for four hours. What worked: at most 5 PRs in the merge queue, each after a clean
  `git merge-tree` against the queue tip; no new PR while jobs it needs are queued; and cancelling runs of merge groups
  that no longer exist (#7552). Only one agent (the shepherd, below) opens, enqueues and repairs PRs.

### Verification after merge

Close the loop with a [verification pass](#verification-passes) on `main`, including the rows that need Linux or large
inputs. Merged is not the same as fixed: the second pass found three fixes that missed shapes outside their tests.

### What went wrong when a step was skipped

- **Linux-sensitive tests verified on macOS only.** #7687 and #7686 passed every local gate on macOS and failed or hung
  on Linux CI: Linux opens `/dev/stdin` again where macOS shares the descriptor, and a test blocked reading a pipe that
  only Linux hands it. One of the two hid a real Linux product bug. Run descriptor and I/O tests on Linux before the PR.
- **A fix gated on an old base.** #7679 passed its gates and did not compile once merged with `main`, where newer PRs
  had added code using the old `u32` line counter. Gate a long-lived branch again after merging current `main`.
- **A guard that fails on improvement.** The perf guard fails on any drift over its threshold, including a speed-up:
  #7672 failed on -7.2% and -7.4%. It needed a per-query override and a follow-up to remove it once `main` carried the
  change (#7695).
- **A local ledger gate taken for CI's.** See [Gates](#gates).
- **A coverage-only failure.** A test passed under `cargo test` and failed only in the coverage shards, because the
  profiler's runtime sets an environment variable in the child (#7586). Read which job failed before assuming a flake.

## Coordinating several agents

### Roles

- **Coordinator:** writes the briefs, assigns territories and issues, files or delegates filing, relays owner
  decisions, keeps the running log, and decides when to stop. It does not hunt or fix.
- **Hunters:** one territory each; find, minimise and write up. Read-only on the repository.
- **Filer:** verifies hunters' findings on current `main`, de-duplicates them, files issues and records each in the
  filing log. A **triager** can run the latest-reference check for a batch of issues at once.
- **Fixers:** one issue (or a queue of small ones) each, from worktree to a gated local commit; they hand off rather
  than push.
- **Timing agents:** run the performance holds on the benchmark machines and clear or reject them.
- **Integrators and verifiers:** the integration passes and the verification passes.
- **One shepherd:** the only agent that pushes, opens PRs, enqueues, re-enqueues after an ejection, and parks a failing
  PR as "needs fixer". Exactly one runs at a time; a successor starts from the previous one's hand-off file.

Machines that other people use need an owner who grants budgets; route every request for a machine through them.

### Shared state files

Agents share state through files in a git-ignored scratch directory, one writer per file where possible, and each
line self-contained:

- **The briefs:** the hunter brief (what counts, what is known, the report format), the fixer brief (worktree, gates,
  hand-off), the latest-reference rule, and per-machine rules. Every agent prompt names the briefs to read; a rule
  change is appended to the brief and sent to running agents.
- **Findings directory:** one file per finding, named `<territory>-<nn>-<slug>.md`.
- **Filing log:** finding file, issue number, severity, size estimate, title. **Rejection log:** finding file, reason.
- **Latest-reference triage table:** issue, verdict (same, differs and 1.7.1 is a bug, differs by design, unclear), note.
- **Hand-off list ("ready"):** branch, worktree, issues, full head sha, priority, note. The note carries gates run, what
  is unverified, merge-order dependencies, `Closes #N` or `Refs #N`, and a hold marker (`HOLD-PERF`, `HOLD-GATES`,
  `HOLD-LEDGER`) only when a hold is meant: a marker word inside an ordinary note held one branch back by mistake.
- **Shepherd lists:** PRs waiting on CI (PR, issue, head sha, note); released branches (branch, PR, sha, time); parked
  PRs (PR, sha, reason); and a status table (PR, issue, state, last action, time).
- **Verification tables:** issue, state, class, target reference, note; one per pass.
- **Follow-ups list:** loose ends fixers and hunters reported, to verify and file in batches.
- **The coordinator's log:** decisions, owner rulings, agent assignments and failures, with times. Re-read after any
  context loss.

### Sharing machines

- **A lock on each shared machine** with two kinds of hold: *shared* for builds, tests and fuzzing, *exclusive* for
  timing. A timing run queues for an exclusive hold, and new shared holds wait behind it. Keep holds to 30 minutes or
  less and give each driver a time budget so it exits by itself. Do setup (copying, extracting, deleting trees) and
  post-processing inside a shared hold too: anything outside the lock disturbs a timing run.
- **Budgets are per machine, for all agents together:** jobs, RAM and disk. The grants in this hunt were 24 jobs, 24 GB
  and 30 GB on the 7950X box; 8 jobs and 16 GB on the M4 Pro (2 cores left free for its owner); 8 jobs and 10 GB on an
  M5 MacBook Air (builds and tests only); 6 jobs and 5 GB on a Linux VM on the Intel i7-6700. Test threads count as
  jobs: set `RUST_TEST_THREADS` and `cargo test -j`, and run the deep-recursion tests (2 GB stacks each) at 2 threads.
  On the coordinating laptop: `CARGO_BUILD_JOBS=4`, one cargo invocation per agent, never a build and a test run together.
- **Disk:** `CARGO_INCREMENTAL=0`, one target directory per agent, `cargo clean -p succinctly` between branches, and
  `cargo clean` once a branch's gates are done.
- **Resumable jobs.** The network dropped several times and every remote box went offline once for hours. Run each long
  job in a detached terminal, as a script that skips finished steps and logs to files on the machine; never as one long
  foreground call. When a machine is unreachable, probe it at most every 10 minutes and work locally meanwhile.
- **Liveness:** every live job touches a heartbeat file each minute and appends `started`/`finished` lines to a status
  file, so the machine owner can tell finished from crashed. A runner refuses to start a second copy of itself.
- **Cleanup:** delete sources, targets and inputs, keep small logs, append the agent's tag to a done file, and tell the
  machine owner. Never kill a process you did not start, and never `pkill` by pattern.

### Stopping rules

- **Freeze hunting when findings outpace fixes.** Ten hours in, 110 issues had been filed and 23 PRs merged; the
  coordinator froze hunting and spent the next day landing fixes. Hunting reopened once only owner decisions and
  performance work were left open.
- **Stop assigning new fixes before the release** with enough time left for one integration pass and one verification
  pass of the release commit. The plan used here: no new fixers in the last 1.5 hours of a 12-hour time box, and a
  final verification pass in the last half hour.
- **Record a long tail instead of chasing it.** The regex translation is the example: one fix (#7682) cut the differing
  rows of a 37,067-row regex fuzz from 2,567 to 1,401, but also made 11 formerly matching rows differ (4 of them toward
  jq 1.8.2; the others were recorded). The remaining four shapes were filed together as #7689 and left unassigned
  until the scope was widened, with the fuzz as the regression net.
- **Leave design questions to the owner.** A performance trade-off (#7554's O(n) `first`), an ownership-model change
  (#7483) or a cap that jq added for its own safety is a decision, not a fix.

## Running the next hunt: checklist

1. Read this guide's coverage map and the reports of the hunters listed as in progress. Choose territories from the
   [never covered](#never-covered) list and the rows with findings, not the zero rows.
2. Fetch `main` and build `cli`, `cli,simd` and a debug build from one commit, on every platform you will hunt on.
   Confirm the reference versions with `--version` on each machine.
3. Write the briefs: finding classes, the reference rule, the known-set sources, the report format, the harness rules,
   and the machine budgets. List what is already filed and swept at zero.
4. Give each hunter one territory, a time box of several hours, the earlier drivers to extend, and no findings cap.
5. Run each candidate through the known-set check, then file one issue per root cause with three outputs.
6. One worktree and branch per issue; reproduce first; test fails before the fix; all gates after the last edit;
   `HOLD-PERF` for hot paths; Linux for descriptor and I/O tests; CI's full ledger comparison for adopted behaviour.
7. One shepherd lands the hand-offs, a few at a time, docs-page conflicts in series, within CI's capacity.
8. Integrate the unmerged fixes and run the full suite before a batch lands.
9. Verify every filed reproduction on `main` (and on Linux at size) after the fixes land, and compare with the last pass.
10. Update this page's coverage map with the new areas, volumes and outcomes, and the index.

### For a yq hunt

The method carries over; the inputs change:

- The reference is yq v4.53.3 (`yq` on `PATH`; capture its output, never recall it). Real yq features look like
  extensions (`--front-matter`, `--split-exp`); check before calling anything an extension.
- The known set is [docs/compliance/yq/limitations.md](../compliance/yq/limitations.md),
  `tests/data/yq-golden-known-failures.txt`, `tests/data/yq-path-consistency-known-failures.txt`, and issues titled
  `yq:`. There is no yq ledger.
- jq-only builtins are gated behind `--jq-extensions` in yq mode: a run without the flag must reject them as yq does.
- The YAML test suite (`tests/data/yaml-test-suite-*`) is the oracle-independent corpus, and `scripts/yq-*` hold the
  existing sweeps: read their headers before writing new ones.
- Shared evaluator code means a yq finding's fix can change jq mode and the reverse: gate both suites.

## Related

- [ADR-0018](../adrs/adr-0018.md): reference fidelity and the decision order.
- [docs/compliance/jq/limitations.md](../compliance/jq/limitations.md): the known set for jq.
- [comparison/jq/README.md](../../comparison/jq/README.md): `succinctly dev compare jq`, the ledger and the probes.
- [Reproducing the jq comparison](reproducing-the-jq-comparison.md): valid comparison runs on any machine.
- [Developer Guide](developer.md#differential-sweeps-against-jq): the repo's committed differential sweeps.
- [Benchmarking Guide](benchmarking.md#ab-benchmarking-method): the A/B method behind the performance hold.
- [Release Guide](release.md): the release checklist a hunt feeds.
- [changelog.d/README.md](../../changelog.d/README.md): changelog fragments.
