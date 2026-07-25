#!/usr/bin/env bash
#
# Survey YAML input shapes across upstream repositories (#326).
#
# The benchmark corpus (#301) answers "does this shape exist in real workloads?"
# for the handful of files it contains. It cannot answer "how often?" — seven
# files cannot measure a shape that occurs in a fraction of a percent of real
# input. This script answers the frequency half, by streaming public repository
# tarballs and classifying every YAML file inside them.
#
# It reports two shapes the corpus has been unable to speak to:
#   * bare-dash sequence items — `-` alone on its line, the shape whose
#     detection cost #106 measured at 16.37x @ 4 MB (Ryzen 9 7950X)
#   * anchors / aliases        — the corpus reads `anchors: 0`, and P4's
#     micro-benchmarks assumed anchor-heavy input
#
# Nothing is written to disk: each tarball is decompressed in memory and
# discarded. Only files between 200 B and 300 KB are counted, which excludes
# both trivia and generated giants (e.g. CRDs) that would swamp the denominator.
#
# Usage:
#   ./scripts/survey-yaml-shapes.sh <owner>/<repo>@<branch> ...
#   ./scripts/survey-yaml-shapes.sh --defaults     # the #326 repository set
#
# Example:
#   ./scripts/survey-yaml-shapes.sh kubernetes/kubernetes@master istio/istio@master
#
# Results are recorded in docs/benchmarks/yaml-shape-survey.md. Re-running will
# not reproduce those numbers byte-for-byte — upstream branches move — so the
# doc pins the date it was run. This is deliberately not a CI drift check.

set -euo pipefail

command -v python3 >/dev/null 2>&1 || { echo "error: python3 is required" >&2; exit 1; }

# The repository set behind docs/benchmarks/yaml-shape-survey.md. Chosen to
# over-sample the workloads #326 predicted would carry the bare-dash shape
# (Ansible, Kubernetes) so a null result there is meaningful rather than absent.
DEFAULT_REPOS=(
  # Kubernetes / manifests
  kubernetes/kubernetes@master
  kubernetes/examples@master
  kubernetes-sigs/kustomize@master
  kubernetes/ingress-nginx@main
  argoproj/argo-cd@master
  prometheus-operator/kube-prometheus@main
  helm/charts@master
  bitnami/charts@main
  istio/istio@master
  # Ansible
  ansible/ansible@devel
  ansible/ansible-examples@master
  ansible/awx@devel
  ansible/molecule@main
  ansible-collections/community.docker@main
  kubernetes-sigs/kubespray@master
  openstack/openstack-ansible@master
  geerlingguy/ansible-for-devops@master
  geerlingguy/ansible-role-docker@master
  # Compose / CI / other hand-written config
  docker/awesome-compose@master
  prometheus/prometheus@main
  open-telemetry/opentelemetry-collector@main
  elastic/beats@main
  gitlabhq/gitlabhq@master
  saltstack/salt@master
  home-assistant/core@dev
  concourse/concourse@master
)

if [[ "${1:-}" == "--defaults" ]]; then
  set -- "${DEFAULT_REPOS[@]}"
fi

if [[ $# -eq 0 ]]; then
  echo "usage: $0 <owner>/<repo>@<branch> ... | --defaults" >&2
  exit 1
fi

python3 - "$@" <<'PY'
import re
import sys
import tarfile
import urllib.request

# A `-` alone on its line. Whether it is a sequence indicator (rather than, say,
# a line inside a block scalar) is not decidable without parsing; the follow-on
# indentation check below is what makes this a usable approximation.
BARE = re.compile(r'^([ \t]*)-[ \t]*$')

# An anchor or alias only ever occupies a node position: directly after a `:` or
# after a `-` item indicator. Requiring that is what separates the real thing from
# the three things that otherwise dominate the count — prose emphasis (`*every*`),
# Helm comment delimiters (`{{/*...*/}}`), and glob/regex wildcards inside quoted
# strings (`["*bar/ns/foo"]`). It also catches merge keys (`<<: *defaults`).
ANCHOR = re.compile(r'(?:^[ \t]*-|:)[ \t]+&[A-Za-z0-9_][\w.-]*')
ALIAS = re.compile(r'(?:^[ \t]*-|:)[ \t]+\*[A-Za-z0-9_][\w.-]*')
COMMENT = re.compile(r'(?:^|[ \t])#.*$')

MIN_BYTES, MAX_BYTES = 200, 300_000


def count_anchors(text):
    """Anchors and aliases in node position, per line, comments stripped.

    Approximate by construction: it is a regex, not the parser, so an anchor
    inside a block scalar still counts. Good enough to answer "is the corpus's
    `anchors: 0` a sampling artefact or upstream reality?", which is all it is for.
    """
    anchors = aliases = 0
    for line in text.split('\n'):
        line = COMMENT.sub('', line)
        anchors += len(ANCHOR.findall(line))
        aliases += len(ALIAS.findall(line))
    return anchors, aliases


def classify(text):
    """Count bare-dash items, split by whether the item has indented content.

    `structured` is the #326 shape (`-\\n  key: value`) — an item whose content
    begins on a later line. `empty` is a `-` with nothing under it, i.e. a null
    item, which is a different shape that happens to share the indicator form.
    """
    lines = text.split('\n')
    structured = empty = 0
    for i, line in enumerate(lines):
        m = BARE.match(line)
        if not m:
            continue
        indent = len(m.group(1))
        nxt = None
        for cand in lines[i + 1:]:
            if cand.strip() == '' or cand.lstrip().startswith('#'):
                continue
            nxt = cand
            break
        if nxt is not None and (len(nxt) - len(nxt.lstrip())) > indent:
            structured += 1
        else:
            empty += 1
    return structured, empty


def survey(spec):
    owner_repo, branch = spec.rsplit('@', 1)
    url = f"https://codeload.github.com/{owner_repo}/tar.gz/refs/heads/{branch}"
    try:
        stream = urllib.request.urlopen(url, timeout=300)
        tf = tarfile.open(fileobj=stream, mode='r|gz')
    except Exception as e:  # network, 404, bad branch — report and continue
        print(f"!! {spec}: {e}", flush=True)
        return None

    files = bare_files = anchor_files = 0
    bare_items = anchors = aliases = 0
    worst = []
    try:
        for m in tf:
            if not m.isfile() or not m.name.endswith(('.yml', '.yaml')):
                continue
            if not (MIN_BYTES <= m.size <= MAX_BYTES):
                continue
            try:
                text = tf.extractfile(m).read().decode('utf-8')
            except Exception:
                continue  # binary or non-UTF-8; not YAML we can classify
            files += 1
            structured, _empty = classify(text)
            if structured:
                bare_files += 1
                bare_items += structured
                worst.append((structured, m.size, m.name))
            a, al = count_anchors(text)
            if a:
                anchor_files += 1
            anchors += a
            aliases += al
    except Exception as e:
        print(f"!! {spec} mid-scan: {e}", flush=True)

    worst.sort(reverse=True)
    print(
        f"== {spec}\n"
        f"   yaml files scanned:   {files}\n"
        f"   with bare-dash items: {bare_files} ({bare_items} items)\n"
        f"   with anchors:         {anchor_files} ({anchors} anchors, {aliases} aliases)",
        flush=True,
    )
    for n, size, name in worst[:5]:
        print(f"     {n:4d} items  {size:7d} B  {name}", flush=True)
    return files, bare_files, bare_items, anchor_files, anchors, aliases


totals = [0] * 6
scanned = 0
for spec in sys.argv[1:]:
    r = survey(spec)
    if r is None:
        continue
    scanned += 1
    totals = [t + v for t, v in zip(totals, r)]

files, bare_files, bare_items, anchor_files, anchors, aliases = totals
pct = (100.0 * bare_files / files) if files else 0.0
apct = (100.0 * anchor_files / files) if files else 0.0
print(
    f"\nTOTAL over {scanned} repo(s): {files} yaml files\n"
    f"  bare-dash sequence items: {bare_files} files ({pct:.3f}%), {bare_items} items\n"
    f"  anchors:                  {anchor_files} files ({apct:.3f}%), {anchors} anchors, {aliases} aliases",
    flush=True,
)
PY
