# YAML Input Shape Survey — Upstream Frequency

> Produced by `./scripts/survey-yaml-shapes.sh --defaults`, run 2026-07-25 against
> branch tips. Deliberately **not** CI-checked: upstream branches move, so re-running
> will not reproduce these numbers byte-for-byte. The date above is the pin.

## Why this exists

[`corpus-shape.md`](corpus-shape.md) is the representativeness lookup that gates perf
claims (#235 Phase 6): before optimising for an input shape, confirm the shape appears
there. It answers **existence** — this shape occurs in real files, at this size. It
cannot answer **frequency**: seven files cannot measure a shape that occurs in a
fraction of a percent of real input, and reading a shape's presence in a seven-file
corpus as "one real file in seven looks like this" inverts the very error the corpus
exists to prevent.

This survey answers the frequency half, by scanning whole upstream repositories.
It was built for #326, which asked what fraction of real YAML uses bare-dash block
sequence items — the shape whose detection cost #106 measured at up to 16.37x.

## Method

- 26 public repositories, branch tips as of 2026-07-25, weighted towards the
  Kubernetes and Ansible workloads #326 predicted would carry the bare-dash shape,
  so that a null result there means something.
- Every `.yml`/`.yaml` file between 200 B and 300 KB. The bounds exclude trivia and
  generated giants (CRDs, vendored lock-alikes) that would swamp the denominator.
- **Bare-dash item**: a `-` alone on its line whose next non-blank, non-comment line
  is more indented — i.e. `-\n  key: value`, an item with content of its own. A `-`
  with nothing under it is a null item, a different shape, and is excluded.
- **Anchors / aliases**: `&name` / `*name` in node position (directly after a `:` or
  a `-` indicator), comments stripped.

Both classifiers are regexes, not the parser, so they are approximations: an anchor
inside a block scalar still counts, and a bare dash inside a block scalar could too.
They are calibrated for the one question asked of them — order of magnitude — and the
node-position constraint on anchors matters, since without it prose emphasis
(`*every*`), Helm comment delimiters, and globs in quoted strings dominate the count.

## Headline

| Shape                     | Files    | % of 33,575 | Occurrences         |
|---------------------------|----------|-------------|---------------------|
| bare-dash sequence items  | 14       | **0.042%**  | 65 items            |
| anchors                   | 166      | **0.494%**  | 777 anchors         |
| aliases                   | —        | —           | 2,543 aliases       |

## Bare-dash items: the #326 hypothesis is false

#326 expected the shape to be "common in Ansible task lists and hand-written
Kubernetes manifests, particularly where the first key would make the line long".
It is neither:

| Workload family      | Repos | YAML files | Files with bare-dash | Rate       |
|----------------------|-------|------------|----------------------|------------|
| Kubernetes/manifests | 9     | 18,362     | 5                    | **0.027%** |
| Ansible              | 9     | 3,785      | 2                    | **0.053%** |
| Other config/CI      | 8     | 11,428     | 7                    | 0.061%     |

`kubernetes/kubernetes` has **0 of 6,154**. Both Ansible hits are
`test/integration/` fixtures in `ansible/ansible`; across every production playbook
scanned — `ansible-examples`, `awx`, `molecule`, `kubespray`, `openstack-ansible`,
`community.docker`, and two `geerlingguy` repos — the count is **zero**.

Every file where the shape does occur, in full — 14 files, and the list is this short
because the shape really is this rare:

| File                                                              | Items | Bytes   | Kind             |
|-------------------------------------------------------------------|-------|---------|------------------|
| `opentelemetry-collector` `confmap/testdata/merge-append-…yaml`   | 22    | 12,673  | test fixture     |
| `opentelemetry-collector` `…-featuregate-disabled.yaml`           | 6     | 3,839   | test fixture     |
| `gitlabhq` `spec/fixtures/config/redis_sentinel_password.yml`     | 6     | 1,075   | test fixture     |
| `gitlabhq` `spec/fixtures/config/redis_new_format_host.yml`       | 6     | 949     | test fixture     |
| `gitlabhq` `spec/fixtures/config/redis_cluster_format_host.yml`   | 6     | 600     | test fixture     |
| **`istio` `common/config/sass-lint.yml`**                         | **5** | 2,055   | **real config**  |
| `gitlabhq` `config/mail_room.yml`                                 | 3     | 3,518   | ERB template     |
| `beats` `filebeat/tests/files/config.yml`                         | 3     | 849     | test fixture     |
| `istio` `manifests/charts/gateways/istio-ingress/…/service.yaml`  | 2     | 1,674   | Helm template    |
| `istio` `manifests/charts/gateways/istio-egress/…/service.yaml`   | 2     | 1,670   | Helm template    |
| `ansible` `test/integration/targets/filter_core/vars/main.yml`    | 1     | 4,920   | test fixture     |
| `ansible` `test/integration/targets/vars_files/runme.yml`         | 1     | 439     | test fixture     |
| `kube-prometheus` `manifests/prometheus-prometheusRule.yaml`      | 1     | 17,233  | generated        |
| `bitnami/charts` `bitnami/seaweedfs/values.yaml`                  | 1     | 184,155 | Helm values      |

Only one is an ordinary hand-written config file that parses standalone. That file is
now vendored into the corpus as `yaml/lint/sass-lint.yml`, which is why the corpus can
report `bare-dash items: 5` at all. Its selection was a search over 46 repositories,
not a preference.

Two denser candidates were rejected, recorded so the search is not repeated: an AWS
CloudFormation template (4 items in 6,194 B) is unusable because its `!Ref`/`!GetAtt`
shorthand tags hit `YamlError::TagNotSupported`, and the 22-item
`opentelemetry-collector` file is a test fixture, which cannot carry a "real workload"
claim.

## Anchors are the opposite case — a genuine sampling gap

[`NOTICES.md`](../../tests/data/bench-corpus/NOTICES.md) long flagged that the
corpus read `anchors: 0`. The natural reading — that P4's anchor-heavy
micro-benchmarks simply over-assumed, and real input has no anchors — is wrong.
Anchors are an order of magnitude more common than bare dashes, and heavily
concentrated by ecosystem:

| Repo                          | YAML files | Files with anchors | Rate       |
|-------------------------------|------------|--------------------|------------|
| `home-assistant/core`         | 747        | 82                 | **10.98%** |
| `saltstack/salt`              | 67         | 6                  | **8.96%**  |
| `concourse/concourse`         | 166        | 7                  | 4.22%      |
| `elastic/beats`               | 1,667      | 23                 | 1.38%      |
| `gitlabhq/gitlabhq`           | 8,354      | 27                 | 0.32%      |
| Kubernetes/manifests (9 repos)| 18,362     | 5                  | 0.027%     |

So `anchors: 0` in the corpus was a **sampling artefact**, not upstream reality: the
corpus over-sampled Kubernetes and Docker Compose, the two families that genuinely do
not use anchors. In workloads that do — Home Assistant configuration, Salt states,
Concourse pipelines, GitLab CI — anchors appear in 4–11% of files, which makes them
roughly 100x more prevalent than the bare-dash shape. Of the two gaps #326 raised,
this was the one worth closing with a corpus file.

#342 closed it, from the densest ecosystem in the table above:
`yaml/home-assistant/air-quality-conditions.yaml` is `home-assistant/core`'s
`homeassistant/components/air_quality/conditions.yaml`, a shipped hand-written config
carrying 41 anchors, 86 aliases, and 6 merge keys (`<<: *name`) in 9,990 B — so the
corpus now reports `anchors: 41, aliases: 86` rather than zero. As with the bare-dash
search, the alternates are recorded so the search is not repeated: `saltstack/salt`
`pkg/common/env-cleanup-rules.yml` (12 anchors, 18 aliases, 12,796 B) parses cleanly and
would serve as a second ecosystem, but adds no idiom the Home Assistant file lacks;
every anchor-carrying file in `concourse/concourse` is a `testflight/` or `topgun/`
fixture, which cannot carry a "real workload" claim.

## What this means for #106

The bare-dash finding was a real quadratic term, and removing it was worth doing on
its own merits. What the survey changes is the claim attached to it. Before:

> large where the shape occurs, and the shape is idiomatic but unsampled

After:

> large where the shape occurs, but the shape occurs in 0.042% of real YAML files —
> absent from `kubernetes/kubernetes` entirely and from every production Ansible
> playbook scanned — and where it does occur it is overwhelmingly in test fixtures,
> generated manifests, and templates rather than hand-written config.

The 16.37x @ 4 MB figure is not wrong; it is just not a claim about the input this
parser will typically see. The `seqwrap` generator remains the right tool for
*benchmarking* the shape — it simply is not evidence of representativeness, which is
the distinction that got P5 rejected.

## Reproducing

```bash
# The 26-repo set behind this page (streams tarballs; nothing written to disk).
./scripts/survey-yaml-shapes.sh --defaults

# Or an ad-hoc set.
./scripts/survey-yaml-shapes.sh kubernetes/kubernetes@master istio/istio@master
```

## Related

| Document                                                          | Purpose                                      |
|-------------------------------------------------------------------|----------------------------------------------|
| [corpus-shape.md](corpus-shape.md)                                | Shape distributions of the corpus (existence)|
| [../../tests/data/bench-corpus/NOTICES.md](../../tests/data/bench-corpus/NOTICES.md) | Corpus provenance and licenses |
| [../guides/benchmarking.md](../guides/benchmarking.md)            | How the corpus and its reports are used      |
| [../parsing/yaml.md](../parsing/yaml.md)                          | YAML optimization phases (P4 anchors, P5)    |
