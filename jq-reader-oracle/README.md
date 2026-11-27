# jq-reader-oracle

jq's own input reader, linked in-process from vendored jq 1.7.1 and jq 1.8.2 sources. It is a test oracle for the
input layer of `succinctly jq`, and comes with a differential harness over generated edge inputs (#7970, step 1 of the
input-reader migration plan).

It is a development tool and is never shipped. It is a separate package with its own lockfile, like `fuzz/` and
`bench-compare/`, so the succinctly crate has no feature, build script or C code for it. The shipped binary, the
published crate and every `--all-features` build are unchanged. It needs a C compiler and builds on Linux and macOS
only: jq's sources need POSIX and GNU C, and `build.rs` refuses a non-unix or big-endian target.

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
| `E2`          | a NUL outside a string, reported as a fault at its own line and column, every value before it  |
| `E1`          | equal to a release through its first fault; best effort above the floor after it               |
| `E2-pending`  | E2's ground, not reported at the NUL yet (#7966): no fault before it, its values kept, floor   |
| `recorded`    | a divergence `docs/compliance/jq/limitations.md` records (#5804, #5308, #2103)                 |
| `BUG(agree)`  | the releases agree and the subject differs outside E1, E2 and the recorded ones                |
| `BUG(split)`  | the releases differ and the subject equals neither, outside E1, E2 and the recorded ones       |

- **The predicates** are spike E's (`preds.py`), applied to whole transcripts and tightened to ADR-0018's #7963
  amendment, which states E1 and E2 (`classify.rs` documents each one):
  - **E1:** equal to a release through its first fault: the values before it (when the run goes on past the fault, as
    under `--seq`, the oracle counts them), the message and its position, and the exit status when the fault ends
    the run. The exit status is zero exactly when jq 1.7.1's is.
  - **E2:** the fault is at the first NUL outside a string, at its own line and column; the values before it are
    jq 1.7.1's from the bytes before the NUL (no NUL-ended number), and a fault that ends the run exits non-zero.
  - **`#5308`:** with the subject's echoed strings taken out, the output before jq's escape fault is exact, and the
    values and the exit status are those of jq on the stream with the escapes made valid.
- **The floor** of all of them is the same: a value neither release prints is a bug. A line that is not JSON is such
  a value, and numbers compare as decimals, not as `f64`. The floor's first two bullets (resume at the next line or
  record; every value lies wholly after the fault) are only approximated: the subject's values must be a
  subsequence of one release's, so a value assembled across the fault, or read from a resume point jq does not use,
  is caught only when no release prints an equal value.
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

### In CI (#7986)

- **Build and tests:** `ci.yml`'s `jq-reader-oracle` job runs `cargo test --locked` on this package, on x86_64 Linux.
- **Lockfile:** `security.yml` runs `cargo audit` and `cargo deny` (with `deny.toml` here) on its lockfile, as for
  `fuzz/` and `bench-compare/`. `ci.yml`'s nested-lockfile step checks that the lockfile matches the manifest.
- **The sweep:** `scripts/jq-reader-oracle-sweep.py SUCCINCTLY` builds this package and runs `diff` over a
  fixed subset, `--seed 3 --cases 15000`. It is the `reader-oracle` `[[sweep]]` of `comparison/jq/manifest.toml`,
  so `succinctly dev compare jq` runs it. The Ledger drift job (`compare-jq-ledger.yml`) runs it too, on every
  pull request that touches `src/**`.
- **What fails the sweep:** a `BUG(agree)` or `BUG(split)` case whose (class, family, flags) signature is not in the
  script's `RESIDUALS` list, or a listed residual that no longer shows. Each residual names its issue. A fix
  deletes its entries in the same change.
- **What doesn't:** `split=1.8.2` rows. They are #7971's to sort out.
- **Why seed 3:** its case 14434 is the lazy-prefix line count of #7948 and #7980, so reverting #7974 fails the
  sweep. Seed 1's first 40,000 cases never reach that shape.
