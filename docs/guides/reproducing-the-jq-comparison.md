# Reproducing the jq comparison

How to reproduce the `succinctly jq` versus jq comparison that a release quotes: the per-platform reports in
[docs/compare/](../compare/jq.md), and the figures generated from them in [docs/benchmarks/jq.md](../benchmarks/jq.md),
[CLAUDE.md](../../CLAUDE.md), the [README](../../README.md) and [choosing-jq.md](choosing-jq.md).

The comparison itself is `succinctly dev compare jq`: compatibility (builtins, syntax forms, CLI behaviour, jq's own
test files, grammar fuzzing, oracle sweeps, JSONTestSuite, robustness) and performance (interleaved, output-gated
run time and peak memory) against pinned source builds of jq 1.7.1 and 1.8.2.
[comparison/jq/README.md](../../comparison/jq/README.md) describes each dimension and the files that drive it.
This guide covers the part that README does not: getting a machine into a state where a run is valid, running every
platform at the same commit, and merging the results into the committed docs.

Two ways to get a machine:

- **Any Linux or macOS machine you control** ([below](#on-a-machine-you-control)).
  The committed x86_64 and Apple M4 Pro reports come from machines like this.
- **A fresh AWS EC2 instance, fully scripted** ([below](#on-aws-ec2-scripted)): `scripts/compare-ec2/` bakes an image
  and runs the comparison unattended, then terminates the instance.
  Use it for the Graviton4 (ARM Neoverse-V2) report, or for any EC2 instance type.

## What makes a run valid

The harness checks most of these and marks a run **INVALID** in its report (and refuses `--write-docs`) when one fails;
the rest are on you.

| Requirement                                                                   | Checked by the harness |
|-------------------------------------------------------------------------------|------------------------|
| A clean working tree at the commit being measured                             | yes                    |
| A release build, without `scalar-yaml`, `portable-popcount`, `select-stats` or `share-stats` | yes     |
| Mains power, not battery                                                      | yes                    |
| The pinned jq builds (`scripts/compare-jq-build.sh`; their sources are sha256-pinned) | yes (version)   |
| The generated inputs and the corpus files match their sha256 pins             | yes                    |
| An idle machine (see [benchmarking.md](benchmarking.md#6-verify-the-machine-is-idle--and-read-the-right-signal)) | no |
| Run as an ordinary user, not root: several CLI probes test permission errors that root never sees | no |
| Enough memory: about 16 GiB for the 1 GB tier (`--huge`), where jq holds a 1 GB document at 9-11 GB | no |
| Enough disk: about 15 GB for the build, both jq builds and the 3.2 GB of `--huge` inputs | no       |
| Every platform at the same commit                                             | no                     |

## On a machine you control

```bash
git clone https://github.com/rust-works/succinctly.git && cd succinctly
git checkout --detach <commit>

scripts/compare-jq-build.sh 1.7.1     # hermetic, sha256-verified builds into target/compare/
scripts/compare-jq-build.sh 1.8.2
scripts/sync-bench-corpus.sh          # the six corpus files too large to commit (~1 MB)

cargo build --release --locked --features cli,simd,regex,serde,bench-runner
./target/release/succinctly dev compare jq --full --huge --write-docs
```

The feature list is `[succinctly] features` in [comparison/jq/manifest.toml](../../comparison/jq/manifest.toml) plus
`bench-runner`, which the harness needs. `--full` takes about three hours on a Graviton4 and `--huge` adds about an hour;
both are single-threaded measurements, so core count barely changes that. The run writes `data/compare/<run_id>/`
(`env.json`, `results.json`, `report.md`) and, with `--write-docs`, rewrites `docs/compare/jq-<os>-<arch>.{md,json}`
plus the index and the blocks other pages quote.

The performance rows time a copy of the binary stripped with plain `strip`, as `release.yml` strips the archive's, and
not `target/release/succinctly` itself: the unstripped build starts about 0.07 ms slower on an Apple M4 Pro (#7321). The
copy is made under `target/compare/` and removed when the dimension ends; the report's *Timed binary* line says so. A
`strip` that cannot run leaves the unstripped binary timed and adds a reason to `env.json`'s `invalid_reasons`.

macOS has no `ulimit -v`, so the 1 GB tier relies on the machine having the memory; on Linux, `--cachegrind` (with
`valgrind` installed) adds instruction counts beside the timings.

## Merging several platforms

Each platform's `--write-docs` regenerates the shared pages from the platform files it can see, so run them all at one
commit and merge in this order:

1. Take only `docs/compare/jq-<os>-<arch>.md` and `.json` from each machine into one checkout of that commit.
2. Re-render the shared pages from the full set:

   ```bash
   ./target/release/succinctly dev compare jq --render-docs
   ```

   It regenerates [docs/compare/jq.md](../compare/jq.md) and the generated blocks of
   [docs/benchmarks/jq.md](../benchmarks/jq.md), [CLAUDE.md](../../CLAUDE.md), the [README](../../README.md) and
   [choosing-jq.md](choosing-jq.md).
3. Commit the platform files and the re-rendered pages together.

## On AWS EC2 (scripted)

`scripts/compare-ec2/compare-ec2.sh` runs on your machine and drives everything through the AWS CLI;
`scripts/compare-ec2/instance.sh` runs on the instance. No inbound network access is needed: the instance reports to an
S3 bucket, and you reach it, if you need to, through AWS Systems Manager Session Manager or, optionally, Tailscale.

```text
 your machine                         AWS
 ------------                         ---
 compare-ec2.sh setup  ───────────►   S3 bucket · IAM role + instance profile · security group (no inbound)
 compare-ec2.sh bake   ───────────►   stock Amazon Linux 2023 instance
                                        installs packages, Rust, Tailscale (off);
                                        builds jq 1.7.1 and 1.8.2 into /opt; syncs the corpus
                                        ──► status/log to S3 ──► powers off ──► saved as an image
 compare-ec2.sh run    ───────────►   instance from that image (terminates itself when done)
   (waits on S3)                        clones the commit; release build; dev compare jq
                                        ──► results.tar.gz + provenance to S3 ──► powers off
 data/compare/ec2/<run-id>/  ◄──────   (fetched)
```

### Prerequisites

- An AWS account and the AWS CLI v2 with credentials for it.
  The permissions the operating user needs are printed, scoped to your bucket and names, by
  `scripts/compare-ec2/compare-ec2.sh operator-policy`; attach that as a policy (or use broader rights you already have).
- `git`, and the [Session Manager plugin](https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html)
  if you want a shell on an instance.
- A region that offers the instance type. The scripts default to `c8g.2xlarge` (Graviton4, 8 vCPU, 16 GiB), the
  smallest Graviton4 that fits the 1 GB tier; any EC2 type works, and an x86_64 type bakes and runs an x86_64 image.

### Configure

Copy [scripts/compare-ec2/compare-ec2.env.example](../../scripts/compare-ec2/compare-ec2.env.example) to
`~/.config/succinctly/compare-ec2.env` (or point `COMPARE_EC2_CONFIG` elsewhere) and set at least the region and a
bucket name of your own. That file names your account's resources, so it stays outside the repository; nothing in the
repository names an account, a resource or a tailnet.

### One-time setup

```bash
scripts/compare-ec2/compare-ec2.sh setup
```

Creates, if missing (it is safe to re-run):

- the S3 bucket, with all public access blocked;
- an IAM role and instance profile (default name `succinctly-compare`) that may read and write that bucket, read the
  pipeline's own SSM parameters, and register with Session Manager (`AmazonSSMManagedInstanceCore`);
- a security group with **no inbound rules** in the VPC of the subnet it will launch into.

The instance still needs outbound internet access (GitHub, crates.io, jq's release tarballs, Amazon Linux's
repositories): the default subnets give each instance a public IPv4 address for that, which costs a few cents per run;
with `COMPARE_SUBNET` you can choose a private subnet behind a NAT gateway instead.

### Bake the image

```bash
scripts/compare-ec2/compare-ec2.sh bake [--instance-type c8g.2xlarge] [--ref main] [--rust-toolchain stable]
```

Launches the newest stock Amazon Linux 2023 image for the instance type's architecture (looked up from AWS's public
SSM parameter) and runs `instance.sh bake` on it, which:

1. installs `gcc make git valgrind perf patch tar gzip xz python3` with `dnf`;
2. installs Tailscale but leaves it disabled and without state, so no node identity is ever baked in;
3. installs a safety-net unit that powers any instance from the image off 60 minutes after boot unless a run takes over;
4. as `ec2-user`: installs Rust with `rustup` (stable by default), clones the repository at `--ref`, builds every jq
   version the manifest pins into `/opt/succinctly-compare/jq/` with `scripts/compare-jq-build.sh`, copies the synced
   corpus to `/opt/succinctly-compare/corpus/`, and warms the cargo registry cache;
5. records what it baked in `/opt/succinctly-compare/image.txt` and clears instance-specific state (`cloud-init clean`,
   `machine-id`, authorized keys), then powers off.

The driver then saves the stopped instance as an image tagged `Purpose=succinctly-compare` and terminates it.
Baking takes 5-25 minutes, depending on the instance type. Saving the image is the one step your machine does itself; if
it slept or lost the network before that, `bake --resume <bake-id>` (the id is printed at launch) finishes the bake from
wherever it stopped. Re-bake every few months, or when the manifest pins a new jq: the image is a snapshot,
and packages and Rust age in it (a run updates Rust itself, see below).

### Run

```bash
scripts/compare-ec2/compare-ec2.sh run --commit <sha-or-branch-or-tag>
```

The commit must be reachable from `COMPARE_REPO_URL`. Options: `--instance-type`, `--args` (default
`--full --huge --write-docs`; `--args --quick` checks the pipeline itself with fewer repetitions), `--max-hours` and
`--idle-minutes` (see [When the instance shuts down](#when-the-instance-shuts-down)), `--rust-toolchain` (a pinned
version such as `1.100.0` instead of the default `stable`), `--keep-on-failure` (leave a failed instance up for
troubleshooting), `--image` (a specific image instead of the newest), `--no-wait`, and the
Tailscale options below.

The run instance, launched with shutdown behaviour **terminate**, runs `instance.sh run`:

1. replaces the image's 60-minute safety net with the run's hard cap, starts the monitor (below), and stops
   `dnf-makecache.timer` so nothing periodic runs during the timed rows;
2. as `ec2-user`: updates the Rust toolchain, clones the commit and checks that `HEAD` is exactly it, copies the baked
   jq builds into `target/compare/` and the corpus into `data/bench/corpus/` (re-syncing if the commit's corpus
   manifest differs), builds with the manifest's features plus `bench-runner` (`--locked`), and runs
   `succinctly dev compare jq <args> --build-jq` (which builds any pinned jq the image lacks);
3. uploads `results.tar.gz` — the harness's output directory, `provenance.txt` (commit, arguments, instance type,
   image, kernel, OS release, the baked `image.txt`, `rustc -vV`, `cargo -V`, the compiler, memory and every installed
   package) and any `docs/compare/jq-<os>-<arch>.{md,json}` that `--write-docs` produced — then powers off, which
   terminates it.

While it runs, `status.json`, `log.txt` and `cap.txt` under `s3://<bucket>/runs/<run-id>/` show its state, its log and
its current hard cap. A monitor on the instance refreshes the log every five minutes, at the lowest CPU priority, in the
same wake-up that applies `extend` and checks for a hang. `run` waits and then downloads everything to
`data/compare/ec2/<run-id>/`; `wait <run-id>` and `fetch <run-id>` do the same for a run started with `--no-wait`,
`status` lists live instances and recent runs with their caps, and `cancel <run-id>` terminates one.

**Your machine can sleep.** The run needs nothing from the machine that launched it: the instance builds, measures,
uploads and terminates on its own, and every local command only reads S3 and the EC2 API. A `run` or `wait` that was
asleep or offline carries on polling when the machine wakes, retrying through the network coming back, and `wait
<run-id>` from any shell picks a run up again.

### When the instance shuts down

A run instance stops — and, launched with shutdown behaviour terminate, is deleted — at the first of:

1. **The run ending**, successfully or not: it powers off as soon as the results (or the failure's log) are uploaded,
   so padding below costs nothing on a run that finishes when expected.
2. **The hard cap:** a `shutdown -h` scheduled at boot, so it fires even if every script on the instance has died. By
   default it is about 2.5-3x the expected duration: 12 hours with `--huge`, 8 with `--full`, 3 otherwise (`--full` is
   about three hours on a Graviton4 and `--huge` adds about one). `--max-hours` sets another.
3. **The idle watchdog:** no CPU progress (under 15 CPU-seconds in five minutes) for `--idle-minutes` (default 60)
   means the run is hung, not slow — every timed row keeps a core busy, and the harness's own deadline for one tool run
   is 15 minutes — so it powers off instead of waiting out the cap, with status `idle-timeout`.

A run that is still going near its cap, or that you want to look into, can be given more time while it runs:

```bash
scripts/compare-ec2/compare-ec2.sh extend <run-id> 6          # hard cap: 6 hours from now
scripts/compare-ec2/compare-ec2.sh extend <run-id> 2 --hold   # ... and pause the idle watchdog until then
```

`extend` writes `control.txt` to the run's S3 prefix; the monitor applies it at its next wake-up (within five minutes),
reschedules the shutdown and acknowledges in `cap.txt`, which `extend` waits for. It moves the cap either way (a smaller
number shortens it) and needs no shell on the instance. `--hold` is for troubleshooting, when nothing on the instance is
using CPU; on the instance itself, `touch /run/succinctly-compare-hold` does the same and `sudo shutdown -h +<minutes>`
moves the cap. A failed run kept up with `--keep-on-failure` keeps its monitor, so `extend` works on it too.

Bake instances follow the same rules with a 2-hour cap and a 30-minute idle watchdog. An instance launched from the
image by hand, outside the scripts, powers off 60 minutes after boot unless something cancels that.

Merge the downloaded platform files as in [Merging several platforms](#merging-several-platforms): copy
`data/compare/ec2/<run-id>/out/docs/*` into `docs/compare/` and run `--render-docs`.

### Troubleshooting a run

- **Session Manager:** `scripts/compare-ec2/compare-ec2.sh shell <run-id>` opens a shell on a live instance (as
  `ssm-user`; `sudo -iu ec2-user` for the build user). The run's log is `/var/log/succinctly-compare.log`, the checkout
  `~ec2-user/work/src`.
- **Tailscale (optional):** pass `--tailscale-key-file <file>` (and optionally `--tailscale-hostname <name>`), where the
  file holds a Tailscale auth key you created for this run. Use an **ephemeral, single-use, pre-approved** key, so the
  node removes itself once the instance is gone and the key cannot be reused. The driver stores it as an SSM
  `SecureString` parameter under `/succinctly-compare/<run-id>/` that, besides you, only the instance role can read (it is
  never in the instance's launch script), the instance joins with Tailscale SSH enabled, and `fetch` or `cancel` deletes the
  parameter. Your tailnet policy must allow SSH to that node (for example an `ssh` rule for the key's tag or owner).
  Tailscale runs a daemon during the timed rows, so leave it off for a run whose numbers you will publish.
- A failure before the harness starts (a build error, a clone of an unpushed commit) shows in `log.txt`; with
  `--keep-on-failure` the instance stays up, subject to the hard cap and the idle watchdog — `extend <run-id> <hours>
  --hold` gives you the time to look.

### Costs and cleanup

On-demand prices vary by region; check them before a run. As an indication, a `c8g.2xlarge` cost about $0.42 an hour
on demand in November 2026, so a `--full --huge` run (about five hours with setup) is a few dollars. The shutdown rules
bound the worst cases: a hung run costs at most the idle watchdog's hour beyond the point it hung, and a run still busy
at its cap costs the cap (12 hours, about $5 at that price). The image's snapshot is billed per GB-month for the blocks
it holds (a few GB), the bucket for what it stores, and a public IPv4 address by the hour while an instance runs.
Nothing else runs between runs.

```bash
scripts/compare-ec2/compare-ec2.sh images --prune 1     # keep only the newest image per architecture
scripts/compare-ec2/compare-ec2.sh teardown --yes        # role, instance profile, security group
scripts/compare-ec2/compare-ec2.sh teardown --yes --delete-bucket   # ... and the bucket with all logs and results
```

## What a reproduction holds fixed, and what it does not

Fixed and verified: the commit (checked out and compared by sha), the jq sources (sha256 against the manifest), the
corpus files and the generated inputs (sha256 against their pins), the build's feature set and profile (checked by the
harness), and, on EC2, the instance type.

Recorded but not fixed: the Rust toolchain (stable at the time of the run unless `run --rust-toolchain` pins one), the
base operating-system image and its packages (`provenance.txt` lists them), the kernel, and the physical host behind an
EC2 instance type. Timings are therefore comparable across runs on the same kind of
machine to within the run's own noise (the report's CV column and `--control`), not bit-for-bit. Compatibility results
do not depend on any of these, except the `math`-builtin digits, which come from the platform's libm
([CLAUDE.md](../../CLAUDE.md#the-rule-the-divergence-sections-below-are-exceptions-to)).

One EC2-specific effect: an instance launched from an image fetches the image's blocks from S3 the first time each is
read. The baked jq binaries are read before any timed row (every row's first, gating run), and the build and the
generated inputs are written fresh on the instance, so this does not reach the timings.
