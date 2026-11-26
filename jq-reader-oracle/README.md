# jq-reader-oracle

jq's own input reader, linked in-process from vendored jq 1.7.1 and jq 1.8.2 sources. It is a test oracle for the
input layer of `succinctly jq`, and comes with a differential harness over generated edge inputs (#7970, step 1 of the
input-reader migration plan).

It is a development tool and is never shipped. It is a separate package with its own lockfile, like `fuzz/` and
`bench-compare/`, so the succinctly crate has no feature, build script or C code for it. The shipped binary, the
published crate and every `--all-features` build are unchanged. It needs a C compiler and builds on Linux and macOS
only: jq's sources need POSIX and GNU C, and `build.rs` refuses an MSVC target.

```bash
cd jq-reader-oracle
cargo test                                   # the oracle against 75 x 3 captured jq outputs, both releases
cargo build --release
./target/release/jq-reader-oracle read --release 1.8.2 a.json b.json       # what `jq-1.8.2 -c . a.json b.json` prints
./target/release/jq-reader-oracle read --mode trace --print-program        # the -n trace program
./target/release/jq-reader-oracle diff --subject ../target/release/succinctly --cases 3000 --out /tmp/oracle
./target/release/jq-reader-oracle show --case 784                          # one case as a shell repro
./target/release/jq-reader-oracle check --subject ../target/release/succinctly -n f0 f1  # classify given files
```

## What is in it

- **`vendor/jq-1.7.1/`, `vendor/jq-1.8.2/`**: the subset of each release that `jq_util_input_*` and `jv_parse.c`
  need, which is about 20.7k lines each, with jq's `COPYING` (MIT, plus decNumber's ICU licence).
  - It has no interpreter, no lexer or parser of the jq language and no oniguruma, so bison, flex and autoreconf are
    never needed.
  - `vendor/README.md` lists the provenance and the two local patches.
- **`build.rs`**: compiles each release once. Every translation unit gets `-include vendor/rename-<release>.h`, so
  that every global symbol becomes `sjq171_*` or `sjq182_*` and both releases link into one binary.
  - `scripts/gen-rename.py` regenerates the headers.
  - The `every_global_symbol_is_renamed` test fails when they fall behind.
- **`src/shim.c`**: a pointer-only C interface, compiled into each build.
- **`src/reader.rs`**:
  - `UtilReader` is jq's real `jq_util_input_*` over files. It is exact by construction.
  - `PortReader` is a Rust port of its read loop over memory, which on jq 1.8.2 can resume at a parser sync point.
  - The drivers print what jq prints for `jq -c .`, a `-n` trace program and the reader model's `inputs` program.
- **`src/diff/`**: the harness.
  - `gen.rs` generates the inputs.
  - `classify.rs` classifies each difference.
  - `mod.rs` runs the cases, minimises the differences and writes the report.

## The harness

`diff` generates cases from `(seed, index)`. Each case puts a *piece* where jq's reads cut:
- **where:** its first byte lands at 4,085-4,100 or 8,180-8,200 bytes, at a file seam, or near the start;
- **families:** a NUL outside or inside a string, a BOM, invalid UTF-8, a straddling token, a fault, RS bytes, or a
  plain long line;
- **fillers:** spaces, newline-ended values, one long string, `--seq` records, or the inside of an array;
- **layouts:** one file, two, or three with an empty, missing or newline-only middle;
- **entry:** the files, standard input from a file, or a pipe;
- **flags:** `-s`, `-n` (the trace program), `--seq` and their mixes.

For each case it runs both C readers in-process and the `--subject` binary as `jq -c`. Each difference gets one class:

| Class         | Meaning                                                                                        |
|---------------|------------------------------------------------------------------------------------------------|
| `same`        | equal to both releases, which agree                                                            |
| `split=1.7.1` | the releases differ; equal to 1.7.1, the pinned release                                        |
| `split=1.8.2` | the releases differ; equal to 1.8.2 only: a deviation from the pin unless a recorded adoption  |
| `E2`          | a NUL outside a string, reported as a fault, printing no value neither release prints          |
| `E1`          | equal to a release through its first fault; best effort above the floor after it               |
| `E2-pending`  | a NUL outside a string that the subject does not report as a fault yet (#7966), no wrong value |
| `recorded`    | a divergence `docs/compliance/jq/limitations.md` records (#5804, #5308, #2103)                 |
| `BUG(agree)`  | the releases agree and the subject differs outside E1, E2 and the recorded ones                |
| `BUG(split)`  | the releases differ and the subject equals neither, outside E1, E2 and the recorded ones       |

- **The predicates** are spike E's (`preds.py`), applied to whole transcripts. E1 and E2 are the owner's 2026-11-27
  decisions (#7963, #7966, #7967). The floor of both is the same: a value neither release prints is a bug.
- **Checking the oracle:** `--jq171 PATH --jq182 PATH --verify-every K` also spawns the real binaries on every `k`-th
  case and reports where they disagree with the in-process reader. The exit status is then 3.
- **Output:** `--out DIR` writes `summary.txt` and `cases.jsonl` (every case that is not `same` or `split=1.7.1`). It
  also writes `repros/<class>-<family>-<flags>-<case>/`, with `repro.sh` and the three transcripts. Up to `--keep`
  repros are kept per signature, each minimised in at most `--minimize-evals` runs.
- **Exit status:** 0 when no case is a bug, 1 when one is, 3 on an oracle mismatch, 2 on a setup error.

### Speed

Cases per second. Every case is staged to disk and run on both releases, so with `--c-only` the rate is mostly
file writes; the spike timed the C reads alone at 18,032 cases/s.

| Machine                         | Jobs | `--c-only` | Subject spawned | Also both jq binaries (`--verify-every 1`) |
|---------------------------------|-----:|-----------:|----------------:|-------------------------------------------:|
| terminus (x86_64 Linux, shared) |    1 |      1,865 |                 |                                            |
| terminus (x86_64 Linux, shared) |    8 |      3,827 |           1,311 |                                        560 |
| Apple silicon laptop            |    1 |      1,941 |                 |                                            |
| Apple silicon laptop            |    4 |      4,214 |             717 |                                        257 |

Spawning the subject is the cost. A 3,000-case subset takes 2-5 s.

### Wiring into CI (not done here)

`diff`'s exit status is the contract of a `[[sweep]]` entry in `comparison/jq/manifest.toml`: 0 agrees, 1 diverges,
anything else is an error. A fixed-seed 3,000-case subset (`--seed 1 --cases 3000`) can join `succinctly dev compare
jq` there, with this package built first (`cargo build --release --manifest-path jq-reader-oracle/Cargo.toml`, on the
Linux and macOS legs only).

Two steps should come with that:
- a `cargo test` of this package beside the `fuzz harness compile check` job;
- `cargo audit`/`cargo deny` for its lockfile in `security.yml`, as `fuzz/` and `bench-compare/` have.

The 1.8.2 adoptions the reader keeps on purpose would then need listing, so that `split=1.8.2` rows of a recorded
adoption stop counting.
