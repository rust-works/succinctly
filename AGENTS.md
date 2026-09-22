# AGENTS.md

Guidance for Codex when working in this repository. Read the linked reference only when the task needs its detail.

## Project

Succinctly is a Rust library and CLI for succinct data structures and semi-indexed JSON, YAML, and DSV queries. Semi-indexing builds a small structural index and materializes values lazily. It performs less validation than a full DOM parser; benchmark comparisons must account for that difference. Start with [docs/index.md](docs/index.md), [ARCHITECTURE.md](ARCHITECTURE.md), and [docs/architecture/semi-indexing.md](docs/architecture/semi-indexing.md).

- `src/bits/`: bit vectors, rank/select, popcount
- `src/trees/`: balanced parentheses
- `src/json/`, `src/yaml/`, `src/dsv/`: semi-indexed formats
- `src/jq/`: query language and evaluator
- `src/bin/`: CLI

The library supports `no_std` with `alloc`. Check feature gates when changing code: `cli`, `simd`, `regex`, `serde`, `bench-runner`, and the large-data test features.

## Worktree discipline

If working in a Git worktree, identify its root and use explicit paths for every Git, build, search, and edit command. Do not assume the shell's current directory is the intended checkout. After the first edit, run `git -C <worktree> status --porcelain` and confirm the edited file appears there. Before a commit, confirm `git -C <worktree> branch --show-current` is the feature branch. Create new worktrees outside the repository under `$HOME/wrk/work-trees/succinctly/<branch>/`.

## Query compatibility

`succinctly jq` follows jq and `succinctly yq` follows yq. The mode decides behavior, never the input format. Reproduce reference behavior, including inconsistencies, unless it would emit output the reference cannot read back, corrupt or discard a write, or take down the host process. Record every permitted divergence in [jq limitations](docs/compliance/jq/limitations.md) or [yq limitations](docs/compliance/yq/limitations.md). Decide behavioral forks in this order: permitted divergence, fidelity to the pinned reference, then performance and memory. Put mode-specific rules on `EvalSemantics`, not per-format traits. See [ADR-0018](docs/adrs/adr-0018.md).

Never state jq/yq behavior from memory. Capture it from the pinned binaries: `/usr/bin/jq` 1.7.1 and Homebrew `yq` v4.53.3. Check before calling something a succinctly extension. Floating-point digits from math builtins depend on platform `libm`; do not pin them in a cross-platform golden without checking both platforms.

For detailed behavior and examples, consult [CLAUDE.md](CLAUDE.md), especially its sections on yq merges, regex substitution, YAML anchors, and succinctly extensions. The detailed reference is task material, not a substitute for measuring current code and pinned binaries.

## Build and verification

```bash
cargo build
cargo build --features simd
cargo test
cargo clippy --all-targets --all-features -- -D warnings
./scripts/build.sh
```

Use focused tests during development and the relevant full checks before a PR. CI coverage uses `cli,simd,regex,serde`:

```bash
cargo llvm-cov --features cli,simd,regex,serde --workspace --summary-only --fail-under-lines 0
omni-dev coverage diff
```

Build the CLI with `cargo build --release --features cli`. The binary's `jq` and `yq` subcommands also have `sjq` and `syq` aliases. Use files under `.ai/scratch/` for manual CLI experiments; tracked examples are not scratch files.

## Documentation and skills

The knowledge wiki begins at [docs/index.md](docs/index.md). Coding conventions are in [docs/STYLE_GUIDE.md](docs/STYLE_GUIDE.md); architecture decisions are in [docs/adrs/README.md](docs/adrs/README.md). Update relevant docs when changing behavior or performance claims.

Repository skills live in `.agents/skills/`. Use the relevant `SKILL.md` for benchmark documentation, bit/SIMD optimization, JSON/YAML indexing, testing, Markdown tables, ADR reviews, release work, and commit messages. Keep skill manifests named exactly `SKILL.md`.
