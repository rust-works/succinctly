# Benchmark corpus — provenance & licenses

Every file in the real-workload benchmark corpus (#301) is sourced from a
permissively-licensed public repository, pinned to a commit SHA, and content-hashed
in [`manifest.json`](manifest.json). Files marked **vendored** are committed under
[`seed/`](seed/) (tiny, offline, CI-verifiable); the rest are fetched on demand by
[`scripts/sync-bench-corpus.sh`](../../../scripts/sync-bench-corpus.sh) into the
git-ignored `data/bench/corpus/`.

This corpus is used **only** as benchmark input to measure parser performance and
input-shape statistics; no file is redistributed as a product artifact. Each file
retains its upstream license below.

## Vendored (committed under `seed/`)

| File | Source | License |
|------|--------|---------|
| `yaml/compose/wordpress.yaml`         | [docker/awesome-compose](https://github.com/docker/awesome-compose) `wordpress-mysql/compose.yaml`     | CC0-1.0    |
| `yaml/compose/nginx-flask-mysql.yaml` | [docker/awesome-compose](https://github.com/docker/awesome-compose) `nginx-flask-mysql/compose.yaml`   | CC0-1.0    |
| `yaml/actions/codeql-analysis.yml`    | [prometheus/prometheus](https://github.com/prometheus/prometheus) `.github/workflows/codeql-analysis.yml` | Apache-2.0 |
| `yaml/actions/stale.yml`              | [prometheus/prometheus](https://github.com/prometheus/prometheus) `.github/workflows/stale.yml`        | Apache-2.0 |
| `yaml/k8s/nginx-deployment.yml`       | [kubernetes/examples](https://github.com/kubernetes/examples) `nginx-platform-app/deployment.yml`      | Apache-2.0 |
| `yaml/lint/sass-lint.yml`             | [istio/istio](https://github.com/istio/istio) `common/config/sass-lint.yml` (bare-dash sequence items: `-` alone on its line) | Apache-2.0 |
| `json/charts/bullet-data.json`        | [plotly/datasets](https://github.com/plotly/datasets) `BulletData.json`                                | MIT        |
| `dsv/world-gdp/world-gdp-with-codes.csv` | [plotly/datasets](https://github.com/plotly/datasets) `2014_world_gdp_with_codes.csv` (genuine quoting: `"Bahamas, The"`) | MIT |

## Fetched on demand (not committed)

| File | Tier | Source | License |
|------|------|--------|---------|
| `yaml/actions/prometheus-ci.yml`     | 10kb  | [prometheus/prometheus](https://github.com/prometheus/prometheus) `.github/workflows/ci.yml` | Apache-2.0 |
| `json/geojson/us-election.geojson`   | 100kb | [plotly/datasets](https://github.com/plotly/datasets) `election.geojson`                      | MIT        |
| `dsv/gapminder/gapminder-five-year.csv` | 100kb | [plotly/datasets](https://github.com/plotly/datasets) `gapminderDataFiveYear.csv`         | MIT        |

## What the corpus can and cannot tell you

The corpus establishes **existence** — that a shape occurs in real files, and what
it looks like at what size. It cannot establish **frequency**: seven files can never
proportionally represent a shape that appears in a fraction of a percent of real
input. `yaml/lint/sass-lint.yml` is the worked example. It was added for #326 so the
bare-dash sequence item (`-` alone on its line) is sampled at all, but a survey of
26 upstream repositories (33,575 YAML files) found that shape in 0.042% of them.
Reading its presence here as "roughly one YAML file in six uses bare dashes" inverts
the error the corpus exists to prevent. Frequency questions belong to
[`docs/benchmarks/yaml-shape-survey.md`](../../../docs/benchmarks/yaml-shape-survey.md).

## Extending the corpus

The 1MB/10MB ladder tiers and anchor/alias-heavy YAML are follow-up curation. The
anchor gap is the more urgent of the two: the corpus still shows `anchors: 0`, and the
survey establishes that this is a **sampling artefact**, not upstream reality — the
corpus over-samples Kubernetes and Compose, the two families that genuinely avoid
anchors, while ecosystems that use them (Home Assistant, Salt, Concourse, GitLab CI)
carry anchors in 4–11% of files, roughly 100x the bare-dash rate. To add one: add a
`manifest.json` entry (`vendored: false`) with a
commit-pinned `source_url`, `license`, `bytes`, and `sha256`, then run
`./scripts/sync-bench-corpus.sh` to fetch and verify it. Prefer public-domain or
permissive (CC0 / MIT / Apache-2.0 / BSD) sources and always record provenance.

Test-parse any candidate before committing it (`succinctly yq -o json '.' <file>`).
`ingest_yaml` turns a parse failure into an error that aborts the whole report, and
plausible-looking sources do fail: the densest bare-dash file found during the #326
search was an AWS CloudFormation template, which is unusable here because its
`!Ref`/`!GetAtt` shorthand tags hit `YamlError::TagNotSupported`.
