# The Path-Context Walk: Shared Trail and Streamed Positions (#2572, #3022)

[Home](../../) > [Docs](../) > [Optimizations](./) > Path-context walk

The walk behind `key`, `parent` and `file_index` (`path_context_step_generic`,
`path_context_walk_generic` and their helpers in `src/jq/eval_generic.rs`) answers a pipe on the
generic evaluator since the eager path-context evaluator was deleted (#2559,
[ADR-0021](../adrs/adr-0021.md) decision 8). The exit measurement in
[plan/path-context-arm-reachability.md](../plan/path-context-arm-reachability.md) attributed the
walk's remaining per-position cost to its own allocations; these two changes removed them. Both
sections were moved here from that plan, unchanged, so the results live with the other
optimization records (#5847).

See also [path-resolution-clone.md](path-resolution-clone.md) (#2058), the sibling fix to
`path()`'s own walk, whose `PathTrail` the path-context trail now shares a `StepTrail` trait with.

## #2572: the walk's position becomes one shared trail

Date: 2026-09-16. The per-position cost the exit measurement attributed to the
walk's own `Vec` clones, removed.

**What changed.** `PathContextPos` carried `path: Vec<OwnedValue>` and
`ancestors: Vec<PathNode<V>>`. Each navigational step rebuilt the path into a
`PathTrail` (`PathTrail::from_slice`, one allocation and one component clone
per level), flattened the stepped trail back with `to_vec`, and cloned the
ancestors; `path_context_step_generic`'s `Expr::Pipe` arm also re-wrapped its
remaining stages in `Expr::Pipe(rest.to_vec())` per head. The position now
holds a `PathContextTrail`: an `Rc`-linked chain whose links carry a component
and the node it was taken from (the old `ancestors[i]` is link `i + 1`'s
`from`). `path_step_generic` extends through a `StepTrail` trait both trails
implement, so the one step definition stays shared with `path()` and no
bridge is left (`from_slice` is deleted). `parent(n)` walks `n` links, `path`
flattens once where it is emitted, and the pipe step recurses on a slice.
Links unlink iteratively on drop, since a walk rooted at a nested input
carries a trail as deep as the document.

**The measurement.** `scripts/ab-cli.py`, base = the merge-base `9d6607978`,
head = the branch, both `codegen-units=1` + fat LTO, 11 interleaved reps, a
deterministic corpus of `- k: {x: n}` records (block YAML and JSON twins) at
1/6/20 MB. `--control` floor on the same corpus: M4 Pro -4.1..+1.9% (yq),
-3.9..+1.7% (jq); 7950X (`taskset -c 2`) -2.6..+5.2% (yq), -1.5..+2.4% (jq).
Output identical on all 84 A/B configurations; the path-context oracle sweep
reports 0 unexpected divergences over 6300 cases. Median change at 1/6/20 MB:

| shape                               | M4 Pro yq       | M4 Pro jq       | 7950X yq        | 7950X jq        |
|-------------------------------------|-----------------|-----------------|-----------------|-----------------|
| `[.[] \| .k.x \| parent] \| length` | -24/-23/-24%    | -34/-35/-33%    | -18/-19/-20%    | -24/-25/-28%    |
| `[.[] \| .k \| select(key == "k")]` | -14/-13/-16%    | -27/-28/-29%    | -11/-11/-15%    | -18/-17/-23%    |
| `[.[] \| .missing \| parent]`       | -11/-9/-10%     | -17/-16/-18%    | -9/-9/-9%       | -14/-11/-16%    |
| `[.[] \| key] \| length`            | -15/-15/-17%    | -17/-20/-21%    | -14/-6/-20%     | -21/-26/-24%    |
| `first(.[] \| key)`                 | -19/-20/-22%    | -24/-28/-30%    | -19/-14/-22%    | -23/-21/-33%    |
| `[.[] \| .k.x] \| length` (holdout) | +0.3/-0.7/-0.5% | -0.6/+0.1/+0.4% | +0.8/+0.8/+1.1% | +1.6/+0.9/+0.7% |

The holdout does not enter the walk, so what it reads is layout: flat on ARM,
a consistent ~+1% on x86 that stays inside that box's control range. Peak RSS
at 20 MB, base -> head: M4 Pro 1051 -> 817 MB (yq) and 1109 -> 822 MB (jq) on
`[.[] | .k.x | parent]`, 1470 -> 1235 MB and 1514 -> 1242 MB on the
absent-head shape; the 7950X is essentially unchanged (645 -> 646, 663 -> 640,
1102 -> 1100, 1106 -> 1089 MB). A `[paths] | length` row was also timed and is
not reported: in yq mode `paths` is gated behind `--jq-extensions`, so it timed
the error path, and in jq mode it never enters this walk (flat on both boxes).

The `first(.[] | key)` row is a timing of succinctly's own answer, not of a
yq-compatible one: in yq mode both binaries print `0` where yq v4.53.3's
`first(f)` ignores its argument and prints the first child (#2377), so that
row is evidence about the walk only, not about fidelity.

**What is left.** The absent-head shape keeps the smallest win on both boxes;
its remaining term is the missing-key scan itself (#2470/#2482's territory,
and the YAML-only part the exit measurement named), not the walk. The largest
remaining per-position allocation is now the `heads` `Vec` each step
collects into: it is created inside calls made once per incoming position
(`path_context_walk_pipe`, `path_context_step_pipe`, the navigational arm), so
every stage costs one more buffer per element -- on `[.[] | .k.x | parent]`
more buffers than trail links. Streaming positions instead of collecting them
per step is the lever the issue estimated at 2-3 days. A walk rooted at a
nested node also pays one link per document level to seed its trail
(`PathContextTrail::from_climb`), where the flat pair collected one `Vec`.

## #3022: positions stream, and a nested seed is one shared prefix

Date: 2026-09-23. The two allocation terms #2572 left behind, removed.

**What changed.** Two mechanisms, kept separable so each could be attributed.

*Streaming.* `path_context_step_each` delivers each position to a callback
instead of collecting a stage's positions into a `heads` `Vec` first. All four
buffers the #2572 note named are gone: `path_context_walk_pipe` and
`path_context_walk_generic`'s `_` arm consume the callback; the navigational
arm of `path_context_step_generic` wraps each `(trail, node)` as it arrives
rather than beside the `out` it is filling; and `path_context_step_pipe` is
deleted in favour of the streaming `path_context_step_pipe_each` it
duplicated. `try_path_context_absent_sink` drives its head through the same
producer and evaluates `rest` per position, so it no longer retains every
final head position either.

The streamed set is exactly literal navigation -- identity, parentheses,
comma branches, `parent`, `Expr::Field`, `Expr::Index`, `Expr::Iterate`, and
nested pipes composed of those. Every component there is fixed by the query,
so none of it can be the computed component `path_context_component_escape`
rolls a step back for in yq mode; computed keys, targets and bounds keep
their collecting boundary, array construction stays atomic, and bounded
consumers keep their own output policies. Fan-out shares one primitive with
`path()`'s own walk: `path_iterate_step_generic` joins the existing
`Field`/`Index` helpers under `path_step_each_generic`, and `path_step_generic`
becomes the collecting adapter for the walkers that want a vector. The child
cursors still come from the checked collection helpers, so a malformed
container raises before any position is delivered, exactly as when the buffer
was filled first. A downstream `Control` is parked in a separate slot and
returned after the producing stage exits, so it can never be caught as that
producer's own error by `try`, `?` or `label`.

*Seeding.* `PathContextTrail` gains a `Seed { steps: Rc<Vec<..>>, visible_len }`
variant beside `Root` and `Link`. `from_climb` moves the climbed vectors into
one shared owner instead of folding them into one `Rc` link per document
level, and a hop into the prefix only shortens `visible_len` rather than
copying. New navigation still costs one link per step, and `depth`,
`last_component`, ancestor iteration and the iterative drop all cross the
prefix/link boundary. Below `PATH_CONTEXT_SEED_MIN_LEVELS` (3) the link form
is kept: a shared vector plus its `Rc` cannot save an allocation at one or
two levels, and a depth sweep put the break-even at two.

**Allocation counts.** Evaluation only -- parse, index, cursor and a warm-up
evaluation are outside the counted window (`examples/alloc_probe_3022.rs`,
release build, `alloc`/`alloc_zeroed`/`realloc` calls). The record rows use the
decimal 1 MB fixture: 56,173 records, 1,000,006 JSON bytes. The nested rows use
the leaf count and depth named in the table. These are counts, not timings.

| row (jq mode / JSON)                                               | base      | head      |
|--------------------------------------------------------------------|-----------|-----------|
| `[.[] \| .k.x \| parent] \| length`                                | 1,348,034 | 1,067,139 |
| `[.[] \| .k \| select(key == "k")] \| length`                      |   561,808 |   505,590 |
| `[.[] \| .missing \| parent] \| length`                            | 1,797,418 | 1,628,869 |
| `[.[] \| key] \| length`                                           |   112,428 |   112,398 |
| `first(.[] \| key)`                                                |    56,220 |        18 |
| `[.[] \| .k.x] \| length` (never enters the walk)                  |    56,197 |    56,197 |
| `[paths] \| length`                                                | 2,022,080 | 2,022,080 |
| `[path(..)] \| length`                                             | 3,594,946 | 3,594,946 |
| `[.. \| select(type=="string") \| (key\|tostring)] \| length`, d64 |   160,610 |    97,610 |
| ... the same at depth 32                                           |    94,290 |    63,290 |
| ... the same at depth 16                                           |    60,129 |    45,129 |
| ... depth 64, 250 leaves                                           |    40,601 |    24,851 |

The seeding rows are linear in `(depth - 1) x leaves` -- 63 per leaf at depth
64, 31 at 32, 15 at 16 -- which is the per-level link term, gone. `first(.[] |
key)` is the demand path: the iteration now stops at the first element instead
of enumerating all 56,173 of them, leaving only the checked enumeration's own
buffer. `[.[] | key]` barely moves because its remaining cost is the per-key
`String`, not the buffer. The never-entering-walk row and both shared-
navigation rows are untouched, which is the point of quoting them.

**Reproduction.** `scripts/generate-path-context-3022-fixtures.py` is the
canonical fixture definition. `mb` means decimal megabytes; JSON and YAML
twins contain the same records. Its default output is:

```text
n1000: records=1000 json_bytes=15892 yaml_bytes=15890
1mb: records=56173 json_bytes=1000006 yaml_bytes=1000004
6mb: records=321638 json_bytes=6000014 yaml_bytes=6000012
20mb: records=1055556 json_bytes=20000012 yaml_bytes=20000010
deep-d64-n1000: bytes=11276
deep-d32-n1000: bytes=11084
deep-d16-n1000: bytes=10988
deep-d64-n250: bytes=3026
```

The reported comparison is exactly base `717833c17` versus candidate
`a422fa389`. Build both with the repository's release profile
(`codegen-units = 1`, fat LTO), using external worktrees as required by the
repository instructions:

```bash
REPO_WT=/path/to/current/succinctly-worktree
BASE_WT=/path/outside/the/repository/issue-3022-base
HEAD_WT=/path/outside/the/repository/issue-3022-head
FIXTURE_DIR=/tmp/succinctly-3022-fixtures

python3 "$REPO_WT/scripts/generate-path-context-3022-fixtures.py" "$FIXTURE_DIR"
git -C "$REPO_WT" worktree add "$BASE_WT" --detach 717833c17
git -C "$REPO_WT" worktree add "$HEAD_WT" --detach a422fa389
cp "$HEAD_WT/examples/alloc_probe_3022.rs" "$BASE_WT/examples/"
cargo build --manifest-path "$BASE_WT/Cargo.toml" --release --features cli --bin succinctly
cargo build --manifest-path "$HEAD_WT/Cargo.toml" --release --features cli --bin succinctly
cargo build --manifest-path "$BASE_WT/Cargo.toml" --release --example alloc_probe_3022
cargo build --manifest-path "$HEAD_WT/Cargo.toml" --release --example alloc_probe_3022
```

The allocation table can then be regenerated verbatim with:

```bash
BASE_PROBE="$BASE_WT/target/release/examples/alloc_probe_3022"
HEAD_PROBE="$HEAD_WT/target/release/examples/alloc_probe_3022"
RECORD_FILE="$FIXTURE_DIR/records-1mb.json"
RECORD_QUERIES=(
  '[.[] | .k.x | parent] | length'
  '[.[] | .k | select(key == "k")] | length'
  '[.[] | .missing | parent] | length'
  '[.[] | key] | length'
  'first(.[] | key)'
  '[.[] | .k.x] | length'
  '[paths] | length'
  '[path(..)] | length'
)
for query in "${RECORD_QUERIES[@]}"; do
  "$BASE_PROBE" jq "$RECORD_FILE" "$query"
  "$HEAD_PROBE" jq "$RECORD_FILE" "$query"
done

DEEP_QUERY='[.. | select(type=="string") | (key|tostring)] | length'
for fixture in deep-d64-n1000 deep-d32-n1000 deep-d16-n1000 deep-d64-n250; do
  "$BASE_PROBE" jq "$FIXTURE_DIR/$fixture.json" "$DEEP_QUERY"
  "$HEAD_PROBE" jq "$FIXTURE_DIR/$fixture.json" "$DEEP_QUERY"
done
```

For wall time, run both the real A/B and the base-against-itself control on an
idle, mains-powered machine. The following invocations reproduce the 1 MB and
6 MB matrix; `ab-cli.py` alternates base/head within each of 11 repetitions and
refuses to time configurations whose output or exit status differs:

```bash
BASE_BIN="$BASE_WT/target/release/succinctly"
HEAD_BIN="$HEAD_WT/target/release/succinctly"
PROFILE='release: codegen-units=1, lto=fat'
QUERIES=(
  '[.[] | .k.x | parent] | length'
  '[.[] | .k | select(key == "k")] | length'
  '[.[] | .missing | parent]'
  '[.[] | key] | length'
  'first(.[] | key)'
  '[.[] | .k.x] | length'
  '[paths] | length'
  '[path(..)] | length'
)

python3 "$REPO_WT/scripts/ab-cli.py" \
  --before "$BASE_BIN" --after "$HEAD_BIN" \
  --before-profile "$PROFILE" --after-profile "$PROFILE" \
  --files "$FIXTURE_DIR/records-1mb.json" "$FIXTURE_DIR/records-6mb.json" \
  --tool jq --reps 11 --queries "${QUERIES[@]}"
python3 "$REPO_WT/scripts/ab-cli.py" \
  --before "$BASE_BIN" --before-profile "$PROFILE" --control \
  --files "$FIXTURE_DIR/records-1mb.json" "$FIXTURE_DIR/records-6mb.json" \
  --tool jq --reps 11 --queries "${QUERIES[@]}"

python3 "$REPO_WT/scripts/ab-cli.py" \
  --before "$BASE_BIN" --after "$HEAD_BIN" \
  --before-profile "$PROFILE" --after-profile "$PROFILE" \
  --files "$FIXTURE_DIR/records-1mb.yaml" "$FIXTURE_DIR/records-6mb.yaml" \
  --tool yq --extra-args='--jq-extensions -I0' --reps 11 \
  --queries "${QUERIES[@]}"
python3 "$REPO_WT/scripts/ab-cli.py" \
  --before "$BASE_BIN" --before-profile "$PROFILE" --control \
  --files "$FIXTURE_DIR/records-1mb.yaml" "$FIXTURE_DIR/records-6mb.yaml" \
  --tool yq --extra-args='--jq-extensions -I0' --reps 11 \
  --queries "${QUERIES[@]}"
```

The published wall-clock and RSS runs used an Apple M4 Pro and an AMD Ryzen 9
7950X (the latter pinned with `taskset -c 2`). The original OS/kernel and Rust
version strings were not retained, so a rerun must record `rustc -Vv`,
`cargo -V`, `uname -a`, and the CPU model alongside its raw output. That
omission does not affect the deterministic allocation counts above, but it
limits bit-for-bit reproduction of the historical timing environment.

Peak RSS used each binary and the 20 MB twin directly. On macOS use
`/usr/bin/time -l`; on Linux use `/usr/bin/time -v` and add `taskset -c 2`
before the binary. Repeat for base/head, the four table queries, and JSON/jq
or YAML/yq as applicable:

```bash
/usr/bin/time -l "$HEAD_BIN" yq --jq-extensions -I0 \
  '[.[] | .k.x | parent]' "$FIXTURE_DIR/records-20mb.yaml" >/dev/null
/usr/bin/time -v taskset -c 2 "$HEAD_BIN" yq --jq-extensions -I0 \
  '[.[] | .k.x | parent]' "$FIXTURE_DIR/records-20mb.yaml" >/dev/null
```

The 7950X instruction counts used Valgrind Cachegrind with cache and branch
simulation disabled; repeat this command for both binaries, both modes, and
each table query, then read the `Ir` total with `cg_annotate`:

```bash
valgrind --tool=cachegrind --cache-sim=no --branch-sim=no \
  --cachegrind-out-file=/tmp/issue-3022-head.cg \
  "$HEAD_BIN" jq -c '[.[] | .k.x | parent] | length' \
  "$FIXTURE_DIR/records-1mb.json" >/dev/null
cg_annotate /tmp/issue-3022-head.cg
```

Finally, semantic verification used jq 1.7.1 and yq v4.53.3, not whichever
versions happen to be first on `PATH`. Verify those version strings, then run
the sweep against the candidate binary; it checks the pins itself and reports
the 6,300-case classification:

```bash
jq --version
yq --version
"$HEAD_WT/scripts/jq-path-context-oracle-sweep.sh" --summary "$HEAD_BIN"
```

**Wall clock.** `scripts/ab-cli.py`, base = the merge-base `717833c17`, head =
`a422fa389`, both `codegen-units=1` + fat LTO, 11 interleaved reps, output and
exit identity gated before any timing. Corpus: the #2572 shape, deterministic
`- k: {x: n}` records as JSON and block-YAML twins at 1 and 6 MB. Median of
medians, 1 MB / 6 MB:

| shape                                       | M4 Pro jq   | M4 Pro yq   | 7950X jq    | 7950X yq    |
|---------------------------------------------|-------------|-------------|-------------|-------------|
| `[.[] \| .k.x \| parent] \| length`         | -8.7/-10.9% | -5.5/-4.0%  | -3.4/-5.1%  | -4.7/-3.9%  |
| `[.[] \| .k \| select(key == "k")]`         | -5.9/-6.6%  | -1.7/-1.6%  | -29.1/-30.5%| -13.3/-12.1%|
| `[.[] \| .missing \| parent]`               | -4.3/-6.5%  | -3.0/-4.5%  | -1.8/-4.3%  | -4.6/-2.1%  |
| `[.[] \| key] \| length`                    | -3.3/-7.1%  | -2.8/-6.1%  | -21.3/-24.6%| -17.6/-20.5%|
| `first(.[] \| key)`                         | -21.5/-30.7%| -15.9/-22.2%| -55.5/-62.2%| -41.4/-45.3%|
| `[.[] \| .k.x] \| length` (holdout)         | +0.6/+1.3%  | -0.8/+0.3%  | +0.7/-0.6%  | +2.0/+4.7%  |
| `[paths] \| length`                         | +2.8/+1.5%  | -1.8/+1.2%  | +1.2/-0.7%  | -0.8/+0.6%  |
| `[path(..)] \| length`                      | -1.8/-0.2%  | +0.7/+0.1%  | +1.0/-1.1%  | +1.5/+1.7%  |

`--control` floors on the same corpora: M4 Pro -1.4..+0.4% (jq), -1.4..+2.0%
(yq); 7950X -1.6..+1.4% (jq), -2.5..+3.0% (yq). The win grows from 1 MB to
6 MB on every improved row, which is the shape a removed per-element term has
rather than a constant factor.

**Holdout builds.** A third binary per box, built from head with a temporary
`PATH_CONTEXT_STREAM_POSITIONS = false` and `PATH_CONTEXT_SEED_MIN_LEVELS =
usize::MAX` -- the two mechanisms off, this branch's code layout kept (rule
10). The switch was removed once these numbers were recorded; only
`PATH_CONTEXT_SEED_MIN_LEVELS` remains. It reads a clean floor on the M4 Pro
(+0.06% median jq, -0.30% yq, per-row -2.3..+1.4%), so the ARM wins above are
the mechanism. On the 7950X it does not: -13.4%..+6.0% (jq), -7.2%..+2.2%
(yq). It is not a pure base twin either -- the navigational arm's buffer
removal and `path_context_step_pipe`'s deletion are unconditional, and
switching the dispatch off leaves a call layer the base does not have -- so
the x86 rows are attributed by instruction count instead.

**Instructions retired** (`valgrind --tool=cachegrind` on the 7950X, 1 MB,
deterministic, so it separates work from code layout):

| row                                         | jq base Ir    | jq d    | yq base Ir    | yq d    |
|---------------------------------------------|---------------|---------|---------------|---------|
| `[.[] \| .k.x \| parent] \| length`         | 1,057,826,129 | -8.67%  | 1,950,209,011 | -4.52%  |
| `[.[] \| .k \| select(key == "k")]`         |   686,468,674 | -10.11% | 1,663,134,182 | -3.74%  |
| `[.[] \| .missing \| parent]`               | 1,260,494,461 | -5.32%  | 2,342,511,380 | -2.79%  |
| `[.[] \| key] \| length`                    |   233,186,156 | -6.66%  |   327,094,630 | -5.02%  |
| `first(.[] \| key)`                         |   126,302,752 | -27.38% |   219,770,334 | -16.14% |
| `[.[] \| .k.x] \| length` (holdout)         |   377,523,108 | +0.00%  |   726,539,964 | +0.00%  |
| `[paths] \| length`                         | 1,327,428,028 | +0.00%  | 1,886,256,253 | +0.00%  |
| `[path(..)] \| length`                      | 2,141,756,263 | +0.00%  | 3,013,691,319 | +0.00%  |

The three bottom rows move by +2,661..+2,921 instructions -- a constant, and
about one part in 10^5 of the smallest of them. So the x86 wall-clock wobble on
those rows (+4.7% on yq `[.[] | .k.x]` at 6 MB, +1.5..+1.7% on `path(..)`) is
code layout, not work: the same reading the #3122 A/B had to settle the same
way. The holdout's own -5.47% Ir on `select(key)` is likewise real and not
layout -- it is the unconditional half of the change, which is why that row's
holdout wall clock read -13%.

**Peak RSS at 20 MB** (`/usr/bin/time`, base -> head, output identical):

| shape                               | M4 Pro yq       | 7950X yq        |
|-------------------------------------|-----------------|-----------------|
| `[.[] \| .k.x \| parent]`           | 707 -> 631 MB   | 584 -> 584 MB   |
| `[.[] \| .missing \| parent]`       | 1104 -> 1014 MB | 1034 -> 1032 MB |
| `first(.[] \| key)`                 | 399 -> 158 MB   | 278 -> 90 MB    |
| `[.[] \| .k.x]` (holdout)           | 256 -> 256 MB   | 172 -> 173 MB   |

jq mode peaks at ~5-8 MB on both boxes on all four rows, unchanged: the YAML
index is what makes the yq figures large. `first(.[] | key)` is again the
demand path -- the iteration's positions are never materialized. The 7950X
absorbing the other two rows where the M4 Pro shows them is the same
architecture split #2572 recorded.

**What is left.** The checked enumeration helpers (`collect_cursors_checked`,
`effective_fields_checked`) still build one vector of child cursors per
container before iteration starts; removing the outer buffer does not make
`.[]` allocation-free, and rewriting those helpers is deliberately out of
scope here. Computed components keep a collecting boundary because yq's
rollback needs one. The absent-head shape's remaining cost is still the
missing-key scan itself (#2470/#2482), not the walk, and the climb that seeds
a nested trail still pays `cursor_slot` per level plus a key `String` per
mapping level -- the seed change removes the link per level, not the climb.
