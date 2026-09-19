#!/usr/bin/env python3
"""CI instruction-count regression guard for `succinctly jq`/`yq` (issue #1523).

#1385's two fix commits made `succinctly jq` 2-3x slower on object-heavy input,
landed on `main`, and were found only because someone happened to re-verify an
unrelated, already-deferred perf issue on a pinned bench box a day later --
nothing in CI would have caught it. Wall-clock timing in CI is the wrong
mechanism (shared runners are noisy neighbours; a threshold loose enough not
to flake is loose enough to let a real 2-3x regression through). This script
uses `valgrind --tool=cachegrind` instead: binary instruction-count
instrumentation, deterministic regardless of what else the runner is doing,
so a real 2-3x work increase shows up as a 2-3x instruction-count increase and
a 5% threshold is meaningful rather than aspirational.

Two modes (both require --arch, since instruction counts are architecture-
specific -- different ISA, different codegen -- so the baseline file holds
one set of counts per arch, not one shared set):

    scripts/perf-guard.py --binary target/release/succinctly --arch x86_64 --check
        Measure the fixed query/shape matrix below, compare each against
        that arch's entry in the committed baseline
        (tests/data/perf-guard-baseline.json), and fail (exit 1) if any
        drifts by more than --threshold percent.

    scripts/perf-guard.py --binary target/release/succinctly --arch x86_64 --update-baseline
        Re-measure and overwrite that arch's entry in the baseline file. Run
        this deliberately, with a stated reason in the commit message --
        never automatically -- or the guard silently ratchets and stops
        catching anything (the exact failure mode this issue was filed to
        prevent). `--arch` must match one of ci.yml's `perf-guard` matrix
        names (currently `x86_64`, `ARM64-Linux`) for the CI job to find it.

`--check` also accepts `--baseline-binary PATH` in place of the default
`--baseline FILE`: instead of comparing against the checked-in JSON, it
measures a second binary (also generated fresh, same as `--binary`) and
compares against that (issue #1582). ci.yml's `perf-guard` job uses this on
`pull_request` runs, building `--baseline-binary` from the PR's own
`git merge-base` -- see "Why a baseline binary, not just the file" below.

The query/shape matrix is #1523's own "minimum viable set": #1514's two
regressions were complementary and each was invisible to the other's own
signal (one showed only in a `wide`-shaped `keys_unsorted` query, the other
only in a plain `.` identity query and would have looked like an *improvement*
under `keys_unsorted` alone) -- so this covers both queries against both a
`wide` (many top-level keys, no nesting) and a `users` (typical small-record)
shape, plus an `arrays` shape (no objects, isolates whether a regression is
object-specific) and one `yq`-mode query (the shared evaluator's cost is
otherwise unverified in yq mode at all) -- #2655 adds a second yq-mode row,
see below.

#2655 added a second class the original set missed entirely: every query
above it reads only, so none exercised the write/path machinery at all --
`del()`/`=`/`path(...)` and an `as` binding route through one of *two*
walkers depending on shape (`needs_path_prepass`, `src/jq/eval.rs`): a
plain `Identity`/`Field`/`Index`/`Slice`/`Iterate` chain (nested under
`Pipe`/`Paren`/`Optional`) takes the single-path walkers (`walk_path`/
`set_path`/`set_path_steps`/`update_path`/`delete_at_path`); anything else
-- a computed key, a `Comma`, or a control-flow shape like `select`/`as`/
`..`/`if` -- needs `resolve_node`/`resolve_node_sink` first. A regression
confined to either was invisible before this issue (#2042's own +21-33%,
the incident that exposed the gap, was in `resolve_node_sink`'s own
`as`-binding/frame-witness machinery). `users_del_select` (a `select`) and
`users_del_bound_select` (an `as` binding, #2042's own shape) exercise
`resolve_node`/`resolve_node_sink`; `users_assign_scores`/`users_path_walk`
are plain field/iterate targets, so they exercise the single-path walkers
instead -- first-ever coverage for that route, not a second instance of the
`resolve_node_sink` coverage the other two rows already give. `|=` is not
yet covered by any row either way (tracked as #2905, since a row for it
needs the same `select`/`as`-shaped target the two `del_*` rows above use
to actually reach `resolve_node_sink`, not a plain field target, which
would only re-cover the single-path walkers already-covered above).
`users_yq_del_select` covers yq-mode writes, which take a different route
(`evaluate_yaml_cursor`'s DOM path in `src/bin/succinctly/yq_runner.rs`)
than any jq-mode row above and that `users_yq_keys_unsorted` (a read) never
exercised either.

Fixtures are generated fresh each run (`succinctly json generate --seed
<fixed>`) rather than checked in, so instruction counts stay meaningful
without growing the repo -- generation is deterministic per pattern/seed, so
the *content* measured is stable run to run. Two rows (`users_compact_identity`
/`users_compact_latefail`, #2608) are derived rather than generated directly:
`measure_all` runs the binary under test over the base `users`/2mb fixture
(`jq -c .`) to get a self-certified canonical compact fixture, then, for the
latefail twin, applies one raw byte edit duplicating the last object's last
member -- see `measure_all` and `inject_duplicate_last_member`.

Known scope limits, deliberate for this minimum-viable-set v1 rather than
oversights -- worth revisiting once this guard has a track record:

- One fixed size (2mb) per query, not a scaling curve across sizes. Catches
  the #1514 shape (a regression visible at a realistic size) but not one
  that only shows up at a different scale.
- The CI job builds with `cargo build --release --features cli`, which does
  not enable the `simd` feature -- so a regression confined to
  `BalancedParens`'s SIMD-accelerated rank/select index build (`src/trees/
  bp.rs`'s NEON/SSE4.1 L1/L2 index builders, gated behind `feature = "simd"`)
  or `src/bits/popcount.rs`'s explicit intrinsics is invisible here, the same
  split `bench`'s own default/simd/portable-popcount 3-way matrix exists to
  separate for `rank_select`.
- The committed `--baseline` file carries a real staleness risk: at this
  project's merge cadence, `Ir` drifts 1-3% across every query within days
  from ordinary accumulated changes alone (measured directly: baseline
  source commit vs. `main` 3 days/147 commits later, 55 of them touching
  `eval.rs`/`json`/`yaml`). This is *not* a `codegen-units=16` artifact --
  pinning `codegen-units=1` on both sides of that same comparison left the
  drift the same size or larger, so #1587's fix for a similar-looking
  wall-clock/icache issue does not apply here. It is why `--baseline-binary`
  exists: a same-run comparison cancels staleness from any cause, instead of
  trying to keep a checked-in file fresh against a fast-moving `main`. `ci.yml`
  now passes `--baseline-binary` on *every* run this job triggers for --
  `pull_request` against the PR's own `git merge-base`, and (#2117) `push`
  against `github.event.before`, the commit `main` pointed to immediately
  before that push -- so the checked-in file only still matters as a
  fallback for the rare case neither can supply a binary (a branch's first
  push, or a `before` SHA this checkout doesn't have), and as a seed for a
  human running this script locally with no comparison binary of their own.

Standard library only; no third-party dependencies.
"""

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile

# Fixed seed for every generated fixture -- the whole point of a checked-in
# instruction-count baseline is that the input content doesn't change between
# runs, only the code being measured does.
FIXTURE_SEED = 20260101

# (id, generate-pattern, size, mode, filter) -- ids are the baseline file's
# keys, so renaming one here orphans its old baseline entry (caught by
# `--check`'s own "missing baseline entry" error) rather than silently
# comparing against the wrong row. Several rows share a (pattern, size) --
# `measure_all` below generates each distinct (pattern, size) fixture once
# and reuses it, rather than regenerating an identical file per row.
QUERIES = [
    ("wide_keys_unsorted", "wide", "2mb", "jq", "keys_unsorted"),
    ("wide_escaped_keys_unsorted", "wide-escaped-keys", "2mb", "jq", "keys_unsorted"),
    ("wide_identity", "wide", "2mb", "jq", "."),
    ("users_keys_unsorted", "users", "2mb", "jq", "keys_unsorted"),
    ("users_identity", "users", "2mb", "jq", "."),
    ("arrays_identity", "arrays", "2mb", "jq", "."),
    ("users_yq_keys_unsorted", "users", "2mb", "yq", "keys_unsorted"),
    # #2666: the two sides of the `map(f) | .[]` atomicity boundary, on the
    # one fixture where `map` applies at the root (a top-level array of
    # arrays). The first is #1565's win -- a truncating consumer over a
    # `LazySeq` pulls one element and stops -- and is the row that must NOT
    # move when #2666 lands. The second is the shape #2666 makes atomic: it
    # goes from streaming to the O(n) buffer jq itself pays for the array
    # `map` builds, so it is *expected* to move. Its `QUERY_THRESHOLDS`
    # entry arrives with the fix PR, sized from that PR's own merge-base
    # delta (`--baseline-binary`, #1582) -- the one measurement that can be
    # attributed to the fix. It is deliberately absent here: an override
    # committed before the fix is a permanent loosening on a row whose only
    # job is to be watched, during a window in which nothing has moved.
    # Neither row existed before, which is how #2042's +21-33% resolver
    # regression passed at -0.0% (#2655) -- this guard cannot see a shape
    # it does not run.
    ("arrays_first_map_iterate", "arrays", "2mb", "jq", "first(map(length) | .[])"),
    ("arrays_map_iterate", "arrays", "2mb", "jq", "map(length) | .[]"),
    # #2655: path-mode rows. Every query above reads only -- none exercises
    # the write/path machinery at all, so a regression confined to it (like
    # #2042's own +21-33%, in `resolve_node_sink`'s `as`-binding machinery)
    # passes this guard at -0.0% no matter how large. Four rows on the
    # existing `users`/`2mb` fixture: the two `del_*` rows below have a
    # `select`/`as` in them, so `needs_path_prepass` routes them through
    # `resolve_node`/`resolve_node_sink` (`src/jq/eval.rs`) -- the general
    # resolver #2042 hit; `users_assign_scores`/`users_path_walk` are plain
    # field/iterate targets, so they route through the single-path walkers
    # (`walk_path`/`set_path`) instead -- a different route, not a second
    # instance of the same coverage. See the module docstring for the full
    # split, and #2905 for `|=`, not yet covered either way.
    ("users_del_select", "users", "2mb", "jq", "del(.users[] | select(.score < 100))"),
    (
        "users_del_bound_select",
        "users",
        "2mb",
        "jq",
        "del(.users[] | .score as $y | select($y < 100))",
    ),
    ("users_assign_scores", "users", "2mb", "jq", "(.users[] | .score) = 1"),
    ("users_path_walk", "users", "2mb", "jq", "[path(.users[] | .age)] | length"),
    # yq-mode writes take a different route (`evaluate_yaml_cursor` -> the
    # DOM path in `src/bin/succinctly/yq_runner.rs`) than jq-mode `del()`
    # above -- `users_yq_keys_unsorted` is this matrix's only yq row and it
    # only reads.
    ("users_yq_del_select", "users", "2mb", "yq", "del(.users[] | select(.score < 100))"),
    # #2608: compact (`-c`) rows. Every row above runs pretty-printed output
    # (no row passes `-c`/`--tab`/`--indent`), so none of them ever reaches
    # `stream_json`'s canonical-compact echo fast path at all -- it is gated
    # on `indent.is_compact()` -- the same "this guard cannot see a shape it
    # does not run" blind spot #2655's own comment above calls out for the
    # write/path machinery. `QUERY_FLAGS` below carries the `-c` these two
    # rows need; `measure_all` derives their fixtures from the base
    # `users`/2mb fixture rather than generating them directly (see its own
    # comment). `users_compact_identity` runs `-c .` over a fixture that is
    # *already* canonical compact jq output (self-certified by round-tripping
    # the base fixture through this run's own binary), so the whole document
    # is echoed verbatim instead of re-rendered -- a regression here is
    # either the echo silently ceasing to fire (same output, far more work)
    # or the scan itself growing slower. `users_compact_latefail` is its
    # twin: the same fixture with one byte-edited duplicate of the very last
    # user object's last member, so the canonical scan walks the *entire*
    # document before failing at that last object, then pays the ordinary
    # re-render on top -- the gate's own precheck cost, worst-cased. Measured
    # against the PR's own merge-base (`--baseline-binary`, #1582; 7950X
    # callgrind, a 7MB `{"data":[...]}` fixture of the same record shape as
    # the 2MB `users` fixture here): `users_compact_identity` ~-66% Ir (the
    # walk disappears entirely), `users_compact_latefail` ~+13% Ir (full
    # scan, then full re-render).
    ("users_compact_identity", "users", "2mb", "jq", "."),
    ("users_compact_latefail", "users", "2mb", "jq", "."),
]

# Per-query extra CLI flags, inserted between the mode (`jq`/`yq`) and the
# filter expression -- e.g. `succinctly jq -c . fixture.json`. A parallel dict
# rather than a sixth `QUERIES` element: every existing row above stays a
# plain 5-tuple with unchanged unpacking and unchanged behaviour (no flags),
# and `measure_all`'s command construction stays a single obvious
# `[binary, mode, *flags, filter_expr, fixture_path]` regardless of whether a
# given row uses this dict at all (#2608).
QUERY_FLAGS = {
    "users_compact_identity": ["-c"],
    "users_compact_latefail": ["-c"],
}

IR_PATTERN = re.compile(r"I\s+refs:\s+([\d,]+)")

DEFAULT_THRESHOLD = 5.0

# Per-query threshold overrides for a drift that is real, understood, and
# accepted as the deliberate cost of a correctness fix -- not a regression
# this guard should keep failing on every future run. `length`/`census`
# absorbed a similar ~10% cost for the same reason (#1677's `,`/`:`
# delimiter check) without ever needing an entry here, simply because it
# isn't one of `QUERIES` above; `wide_keys_unsorted` doesn't have that
# option; it's one of the tracked queries, so its accepted cost needs an
# explicit, narrower threshold instead of silently failing forever.
#
# Measured live on this repo's own CI runners (not a pinned bench box --
# `--baseline-binary`'s same-run merge-base comparison, #1582, is what makes
# this number trustworthy despite that): `wide_keys_unsorted` (159K short
# top-level keys, no nesting) is the one workload with nothing else to
# dilute #1677's two new per-key checks (`key_delimiter_ok`,
# `key_only_value_delimiter_ok`) against -- x86_64 measured +4.8%, already
# under `DEFAULT_THRESHOLD`; ARM64-Linux measured +8.6%, consistently,
# across a redundant-decode fix that halved `key_is_malformed`'s own cost
# without moving this number (both delimiter checks were already on their
# cheapest available scan direction -- see `following_gap_ok`'s own doc
# comment for the +16%-to-noise fix that predates this). 10% leaves headroom
# above the observed ARM64 number while still catching a *further*
# regression on top of this one.
#
# An override is a permanent loosening of a row whose only job is to be
# watched, so one that covers a *one-off* drift (an accepted improvement, or
# a cost that only shows against a merge-base predating the change) must go
# once `main` has moved past the change: with `--baseline-binary` on every
# PR and push run the checked-in file is never consulted, so the row reads
# ~0% again from then on and the override only blinds it. The guard is
# `abs(drift)`, so direction is irrelevant to whether an override is needed
# (see 9d555f3f0, which corrected exactly that misreading).
# `users_keys_unsorted` carried one such entry for #2878's document-splitter
# scan (-8.2% ARM64 / +2.2% x86_64 against its own merge-base) until #2963
# removed it.
#
# `users_del_select` / `users_del_bound_select` / `users_yq_del_select` (#2999):
# a drift that is *faster* on both architectures. Structural sharing in
# `OwnedValue` (ADR-0024, option C) turned the whole-document copy that
# `del(paths)` kept beside its result for path resolution into a refcount
# bump, and measured by this guard on its own runners against the PR's
# merge-base:
#
#                          users_del_select   users_del_bound_select   users_yq_del_select
#   ARM64-Linux                 -13.2%               -13.2%                 -6.5%
#   x86_64                      -11.3%               -11.3%                 -4.9%
#
# Every other row is within +2.3% (`users_path_walk`, the per-emitted-path
# array's refcount box) and -0.2%. The guard is `abs(drift)`, so an accepted
# improvement needs the same override a regression would; 20% / 20% / 12%
# clear the measured numbers with headroom while still catching a further
# move on top of them in either direction.
#
# **Remove these three entries once `main` has moved past #2999** (the rule
# above). Tracked by #3077.
#
# `users_identity` (#2720): faster on both architectures. The identity writer
# no longer materializes every field of an object (a 144-byte `DocumentField`
# each, decoded up front) before writing the first one; it validates the
# object with one key-only walk and streams the fields, materializing only
# for `-S` or a repeated key. Measured by this guard against the PR's own
# merge-base: `users_identity` -7.2% x86_64 / -7.1% ARM64-Linux,
# `wide_identity` -3.0% / -1.0% (under the default), every other row within
# +0.4% (the `keys_unsorted` rows, for the `,` arm #2720's review added to
# the key-only value-delimiter scan).
# 12% clears the measured number with headroom; remove once `main` has moved
# past #2720, per the rule above (alongside #3077's three).
#
# `users_assign_scores` (#3009): faster on both architectures. A write
# produces an owned document and then prints it, and printing one used to
# rebuild the whole tree as `lazy::JqValue` first -- freeing each source map
# and allocating the destination one, which since #3000 lands in a different
# allocator bin. #3009 prints the owned tree directly. Measured by this guard
# on its own runners against the PR's merge-base:
#
#                          users_assign_scores   users_del_select   users_del_bound_select
#   ARM64-Linux                  -6.4%               -5.7%                -5.5%
#   x86_64                       -6.3%               -5.6%                -5.5%
#
# `arrays_map_iterate` (the `LazySeq` route, which #3009 does not touch)
# reads -0.3%/-0.4%; every other row is within +/-0.0%.
# 12% clears the measured number with headroom, matching #2720's entry for
# a move of the same size. Remove once `main` has moved past #3009, per the
# rule above -- tracked by #3161.
#
# The two `del` rows need no entry of their own *only because* #2999's 20%
# entries above already cover them: #3009 moves them past the 5% default
# too. So #3077 must not remove those two until `main` carries #3009 as
# well, or they fail at the default -- #3161 records the coupling.
#
# `users_compact_identity` (#2608): faster on both architectures, measured
# against the PR's own merge-base (~-66% Ir, see the `QUERIES` comment
# above). Unlike the entries above, this one is not simply a one-off to
# remove the moment it's added: the drift comes from comparing a binary
# *with* the canonical-compact echo fast path against a merge-base
# *without* it, which is exactly what every `--baseline-binary` run does
# on every PR/push until `main` itself carries #2608. 75% clears the
# measured number with headroom. Remove only once `main` has moved past
# #2608, per the rule above (tracked by #3170) -- from that point on the merge-base always
# includes the echo path too and this row reads ~0% again like every other
# row.
#
# `users_compact_latefail` (#2608): slower on both architectures, measured
# the same way (~+13% Ir) -- the gate's own precheck cost, paid in full
# (a walk of the whole document) before falling back to the unchanged
# re-render. 20% clears the measured number with headroom. Remove once
# `main` has moved past #2608, per the rule above (tracked by #3170).
QUERY_THRESHOLDS = {
    "wide_keys_unsorted": 10.0,
    "users_del_select": 20.0,
    "users_del_bound_select": 20.0,
    "users_yq_del_select": 12.0,
    "users_identity": 12.0,
    "users_assign_scores": 12.0,
    "users_compact_identity": 75.0,
    "users_compact_latefail": 20.0,
}

# argparse wants a plain string for `epilog`; keeping it as a real constant
# (not a slice of `__doc__`) means reflowing the module docstring above can
# never silently truncate or misplace `--help` output.
EPILOG = (
    "The query/shape matrix is #1523's own \"minimum viable set\": #1514's two "
    "regressions were complementary and each was invisible to the other's own "
    "signal (one showed only in a `wide`-shaped `keys_unsorted` query, the "
    "other only in a plain `.` identity query and would have looked like an "
    "*improvement* under `keys_unsorted` alone) -- so this covers both queries "
    "against both a `wide` (many top-level keys, no nesting) and a `users` "
    "(typical small-record) shape, plus an `arrays` shape (no objects, "
    "isolates whether a regression is object-specific) and one `yq`-mode "
    "query (the shared evaluator's cost is otherwise unverified in yq mode "
    "at all) -- #2655 adds a second yq-mode row below. #2655 also added "
    "path-mode coverage on top of that: every row before #2655 read only, "
    "so a regression confined to del()/=/path(...)/an as binding's resolver "
    "passed at -0.0% no matter how large. users_del_select/"
    "users_del_bound_select exercise resolve_node/resolve_node_sink "
    "(needs_path_prepass's multi-path route); users_assign_scores/"
    "users_path_walk exercise the single-path walkers instead (a plain "
    "field/iterate target skips resolve_node entirely) -- first coverage "
    "for that route, not a second instance of the other two rows'. "
    "users_yq_del_select covers yq-mode writes (a different route, "
    "evaluate_yaml_cursor's DOM path) than any jq-mode row. |= is not yet "
    "covered either way -- #2905."
)


def positive_int(text):
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError(f"must be >= 1, got {value}")
    return value


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        description="CI instruction-count regression guard for succinctly jq/yq.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=EPILOG,
    )
    p.add_argument("--binary", required=True, help="path to the succinctly binary (release build)")
    baseline_source = p.add_mutually_exclusive_group()
    baseline_source.add_argument("--baseline", default="tests/data/perf-guard-baseline.json",
                    help="path to the checked-in baseline file")
    baseline_source.add_argument("--baseline-binary", default=None,
                    help="path to a second built binary (e.g. the PR's git merge-base) to "
                         "measure and compare against instead of --baseline -- cancels drift "
                         "from any cause shared by both binaries (staleness, codegen shift, "
                         "toolchain version), since both are built the same way in the same "
                         "run. Only valid with --check (issue #1582).")
    p.add_argument("--arch", required=True,
                    help="baseline key for this run, e.g. x86_64/ARM64-Linux -- instruction "
                         "counts are architecture-specific (different ISA, different codegen), "
                         "so the baseline file holds one set of counts per arch, not one shared "
                         "set")
    p.add_argument("--valgrind-bin", default="valgrind", help="path to valgrind")
    p.add_argument("--threshold", type=float, default=DEFAULT_THRESHOLD,
                    help="max allowed instruction-count drift, in percent")
    p.add_argument("--reps", type=positive_int, default=3,
                    help="measurement repetitions per query (median reported)")
    mode = p.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="compare against the baseline; exit 1 on drift")
    mode.add_argument("--update-baseline", action="store_true", help="overwrite the baseline file")
    return p.parse_args(argv)


def generate_fixture(binary, pattern, size, seed, out_path):
    cmd = [binary, "json", "generate", size, "-p", pattern, "-s", str(seed), "-o", out_path]
    result = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    if result.returncode != 0:
        sys.exit(f"'{' '.join(cmd)}' exited {result.returncode}; stderr:\n{result.stderr}")


def generate_compact_fixture(binary, source_path, out_path):
    """Canonicalizes `source_path` into compact jq output by running the
    binary under test over it (`jq -c .`), rather than assuming the
    generator's own formatting already matches that binary's writer byte for
    byte -- self-certifying, and deterministic given a fixed source file
    (#2608). This is what makes `users_compact_identity` reach
    `stream_json`'s canonical-compact echo fast path: the fast path only
    fires when the span it's about to write is *exactly* what this same
    write path would have produced anyway."""
    cmd = [binary, "jq", "-c", ".", source_path]
    with open(out_path, "wb") as out:
        result = subprocess.run(cmd, stdout=out, stderr=subprocess.PIPE)
    if result.returncode != 0:
        sys.exit(
            f"'{' '.join(cmd)}' exited {result.returncode}; stderr:\n"
            f"{result.stderr.decode(errors='replace')}"
        )


def inject_duplicate_last_member(compact_path, out_path):
    """Byte-edits `compact_path` (expected to end `...}]}`, i.e. a top-level
    `{"users":[...]}` document) into `users_compact_latefail`'s twin: the
    last user object's last member (`"score":N`, no decoding involved) is
    duplicated immediately before that object's own closing `}` --
    `{"...,"score":5}]}` -> `{...,"score":5,"score":5}]}`. Deliberately raw
    bytes, not a JSON parse -- the fixture this runs on is already known-
    canonical compact `users` output (`generate_compact_fixture`), so its
    shape is fixed and a byte search is both simpler and cannot itself
    perturb the very spelling `users_compact_identity`'s twin needs to stay
    unchanged (#2608). A duplicate key is syntactically legal JSON (jq's own
    "last value wins" semantics apply on re-render), so the result still
    round-trips -- it is only *not certifiable as canonical*, which is the
    point: `canonical_compact_jq_span_end`'s duplicate-key check rejects it
    only once it reaches this last object, after walking every one before
    it."""
    with open(compact_path, "rb") as f:
        data = f.read()
    tail = b"}]}"
    close_brace = data.rfind(tail)
    if close_brace == -1:
        sys.exit(
            f"{compact_path}: expected a '{tail.decode()}' tail (top-level "
            f"{{\"users\":[...]}} document) -- unexpected users-fixture shape"
        )
    comma = data.rfind(b",", 0, close_brace)
    if comma == -1:
        sys.exit(f"{compact_path}: no comma found before the last object's closing brace")
    last_member = data[comma + 1:close_brace]
    new_data = data[:close_brace] + b"," + last_member + data[close_brace:]
    with open(out_path, "wb") as f:
        f.write(new_data)


def run_cachegrind_once(valgrind_bin, binary, mode, flags, filter_expr, fixture_path):
    with tempfile.TemporaryDirectory() as tmp:
        log = os.path.join(tmp, "cg.log")
        cg_out = os.path.join(tmp, "cg.out")
        cmd = [
            valgrind_bin, "--tool=cachegrind", f"--log-file={log}",
            f"--cachegrind-out-file={cg_out}",
            binary, mode, *flags, filter_expr, fixture_path,
        ]
        # `--log-file` captures valgrind's own diagnostics; the *traced*
        # binary's stderr is separate and captured too (not discarded) so a
        # failure -- the traced binary erroring on a query it always used to
        # handle, for instance -- reports why, not just an opaque nonzero
        # exit. `LC_ALL=C` pins cachegrind's number formatting so a locale
        # change on the runner can't silently alter how IR_PATTERN below
        # needs to parse it.
        env = {**os.environ, "LC_ALL": "C"}
        result = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
        if result.returncode != 0:
            sys.exit(
                f"'{' '.join(cmd)}' exited {result.returncode}; stderr:\n{result.stderr}"
            )
        # A query that silently starts producing no output (a bug that
        # changes correctness, not just cost) would look exactly like a
        # legitimate speedup -- catch that before the instruction count is
        # ever trusted, rather than only checking the exit code.
        if not result.stdout.strip():
            sys.exit(
                f"'{' '.join(cmd)}' exited 0 but produced no output -- refusing to trust "
                f"its instruction count (a correctness regression can look exactly like a "
                f"speedup)"
            )
        if not os.path.exists(log):
            sys.exit(f"'{' '.join(cmd)}' exited 0 but wrote no cachegrind log at {log}")
        with open(log) as f:
            text = f.read()
        m = IR_PATTERN.search(text)
        if not m:
            sys.exit(f"could not find instruction count in cachegrind output:\n{text}")
        return int(m.group(1).replace(",", ""))


def measure_query(valgrind_bin, binary, mode, flags, filter_expr, fixture_path, reps):
    counts = [
        run_cachegrind_once(valgrind_bin, binary, mode, flags, filter_expr, fixture_path)
        for _ in range(reps)
    ]
    # `statistics.median` returns a float for an even-length input (the
    # average of the two middle values) but an int for odd-length -- round
    # to a plain int either way so the baseline file's schema doesn't flip
    # between int/float purely as a side effect of --reps's parity.
    return round(statistics.median(counts))


def measure_all(binary, valgrind_bin, reps, label="binary"):
    """Generate each distinct (pattern, size) fixture exactly once -- several
    `QUERIES` rows share one, and regenerating an identical file per row
    would be pure waste -- then measure every query against its fixture.
    Returns {query_id: median instruction count}. `label` only affects the
    printed header, distinguishing this run's output in CI logs when
    `measure_all` is called twice (current binary, then --baseline-binary)."""
    with tempfile.TemporaryDirectory() as tmp:
        fixture_paths = {}
        for _, pattern, size, _, _ in QUERIES:
            shape = (pattern, size)
            if shape not in fixture_paths:
                # `succinctly json generate` always writes JSON; the `.json`
                # extension matters for the `yq`-mode query above -- `yq`'s
                # input-format auto-detection is by file extension, not
                # content sniffing, so this is what makes it resolve to JSON
                # rather than falling through to YAML's own default.
                path = os.path.join(tmp, f"{pattern}_{size}.json")
                generate_fixture(binary, pattern, size, FIXTURE_SEED, path)
                fixture_paths[shape] = path

        # #2608: `users_compact_identity`/`users_compact_latefail` don't read
        # a generated fixture directly -- they're derived from the base
        # `users`/2mb one above, using this run's own `binary` (so a fixture
        # measured against `--baseline-binary` is canonicalized by *that*
        # binary's writer, not this run's, keeping each `measure_all` call
        # self-consistent the same way its own fixture generation already is).
        derived_fixture_paths = {}
        if ("users", "2mb") not in fixture_paths:
            # Fail loudly rather than fall back to measuring `-c .` over the
            # pretty-printed fixture: that early-fails on its first whitespace
            # byte and never reaches the echo, so the rows would keep
            # reporting a number for a shape they no longer measure (#2608
            # review) -- the same blind spot the row comment above warns
            # about.
            sys.exit(
                "the #2608 compact rows derive from the `users`/2mb fixture, which is no "
                "longer in QUERIES -- re-derive them from a shape that is, or drop them"
            )
        base = fixture_paths[("users", "2mb")]
        compact_path = os.path.join(tmp, "users_2mb_compact.json")
        generate_compact_fixture(binary, base, compact_path)
        derived_fixture_paths["users_compact_identity"] = compact_path
        latefail_path = os.path.join(tmp, "users_2mb_compact_latefail.json")
        inject_duplicate_last_member(compact_path, latefail_path)
        derived_fixture_paths["users_compact_latefail"] = latefail_path

        measured = {}
        print(f"Measuring {label} ({binary}):")
        print(f"{'query':<26} {'instructions':>16}")
        print("-" * 44)
        for query_id, pattern, size, mode, filter_expr in QUERIES:
            fixture_path = derived_fixture_paths.get(query_id, fixture_paths[(pattern, size)])
            flags = QUERY_FLAGS.get(query_id, [])
            ir = measure_query(valgrind_bin, binary, mode, flags, filter_expr, fixture_path, reps)
            measured[query_id] = ir
            print(f"{query_id:<26} {ir:>16,.0f}")
            sys.stdout.flush()
        print()
    return measured


def load_baseline_file(path):
    """The whole checked-in file: {arch: {query_id: instructions}}."""
    if not os.path.exists(path):
        return {}
    with open(path) as f:
        try:
            return json.load(f)
        except json.JSONDecodeError as e:
            sys.exit(f"{path} is not valid JSON ({e}) -- a stale conflict marker from a bad "
                     f"rebase/merge, or a truncated write?")


def save_baseline_file(path, data):
    with open(path, "w") as f:
        json.dump(data, f, indent=2, sort_keys=True)
        f.write("\n")


def main(argv=None):
    args = parse_args(argv)

    if shutil.which(args.valgrind_bin) is None:
        sys.exit(
            f"valgrind not found at {args.valgrind_bin!r} -- install it (e.g. `apt-get install "
            f"valgrind` on Linux) or pass --valgrind-bin. This guard has no valgrind on macOS "
            f"(Apple Silicon isn't supported by valgrind at all), so it only runs on the "
            f"Linux CI legs (issue #1523)."
        )
    if not os.path.exists(args.binary):
        sys.exit(f"binary not found: {args.binary}")
    if not os.access(args.binary, os.X_OK):
        sys.exit(f"binary is not executable: {args.binary} (lost its execute bit in transit?)")

    if args.baseline_binary and args.update_baseline:
        sys.exit("--baseline-binary is only valid with --check -- there's nothing to update "
                 "a checked-in baseline *from* a transient second binary.")
    if args.baseline_binary:
        if not os.path.exists(args.baseline_binary):
            sys.exit(f"baseline binary not found: {args.baseline_binary}")
        if not os.access(args.baseline_binary, os.X_OK):
            sys.exit(f"baseline binary is not executable: {args.baseline_binary} (lost its "
                     f"execute bit in transit?)")

    # Fail fast on a missing/incomplete baseline before running any of the
    # (expensive, cachegrind-instrumented) measurements below. Instruction
    # counts are architecture-specific (different ISA, different codegen),
    # so the baseline file is keyed by `--arch` -- x86_64's counts are not a
    # valid baseline for ARM64-Linux's run or vice versa. None of this
    # applies when `--baseline-binary` is given: `measure_all` below always
    # returns exactly `known_ids`, so there's no file to be stale/incomplete.
    #
    # Loaded whenever `--baseline-binary` isn't in play -- including
    # `--update-baseline` -- because `--update-baseline` merges `measured`
    # into whatever this dict already holds and then writes the whole thing
    # back out. Gating the load on `args.check` left `--update-baseline`
    # starting from `{}` and overwriting the file with only the arch just
    # measured, silently deleting every other arch's entries (#1582).
    baseline_file = {}
    baseline = {}
    known_ids = {q[0] for q in QUERIES}
    if not args.baseline_binary:
        baseline_file = load_baseline_file(args.baseline)
    if args.check and not args.baseline_binary:
        baseline = baseline_file.get(args.arch, {})
        missing = known_ids - set(baseline)
        if missing:
            sys.exit(
                f"baseline for arch {args.arch!r} is missing entries for: {sorted(missing)} "
                f"-- run with --update-baseline first (and commit the result)."
            )

    measured = measure_all(args.binary, args.valgrind_bin, args.reps, label="current binary")

    if args.update_baseline:
        baseline_file[args.arch] = measured
        save_baseline_file(args.baseline, baseline_file)
        print(f"Wrote {len(measured)} entries for arch {args.arch!r} to {args.baseline}")
        return 0

    if args.baseline_binary:
        # A same-run comparison against the PR's own merge-base: whatever
        # staleness the checked-in file would carry (real accumulated drift,
        # codegen shift, toolchain version) is shared by both binaries here
        # and cancels out, leaving only what this binary's diff changed
        # (issue #1582).
        baseline = measure_all(args.baseline_binary, args.valgrind_bin, args.reps,
                                label="baseline binary")

    stale = set(baseline) - known_ids
    if stale:
        print(f"NOTE: baseline has stale entries no longer measured: {sorted(stale)}")

    failed = []
    print(f"{'query':<26} {'baseline':>14} {'current':>14} {'drift':>8}")
    print("-" * 66)
    for query_id, _, _, _, _ in QUERIES:
        base = baseline[query_id]
        cur = measured[query_id]
        drift = (cur / base - 1) * 100 if base else float("inf")
        threshold = QUERY_THRESHOLDS.get(query_id, args.threshold)
        flag = "  <-- FAIL" if abs(drift) > threshold else ""
        print(f"{query_id:<26} {base:>14,.0f} {cur:>14,.0f} {drift:>+7.1f}%{flag}")
        if abs(drift) > threshold:
            failed.append((query_id, drift, threshold))

    print()
    if failed:
        print(f"FAILED: {len(failed)} quer{'y' if len(failed) == 1 else 'ies'} exceeded "
              f"its threshold:")
        for query_id, drift, threshold in failed:
            print(f"  {query_id}: {drift:+.1f}% (threshold {threshold}%)")
        print()
        print("A drift in either direction fails: a genuine improvement needs the same "
              "conscious baseline update as a regression, so a stale baseline can't quietly "
              "keep passing. If this is real and understood (new correctness work that "
              "genuinely costs more, or an optimization that genuinely costs less), re-run "
              "with --update-baseline and say why in the commit message. If it's unexpected, "
              "that's exactly what this guard exists to catch -- see "
              "docs/guides/benchmarking.md for how to investigate.")
        return 1

    print("OK: all queries within threshold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
