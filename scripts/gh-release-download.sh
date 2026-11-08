#!/usr/bin/env bash
#
# Download one asset of a pinned GitHub release, retrying (#6652).
#
# Usage:
#   scripts/gh-release-download.sh <owner/repo> <tag> <asset> <dest>
#
# Every workflow step that installs a pinned tool calls this instead of `curl`ing
# https://github.com/<owner>/<repo>/releases/download/<tag>/<asset>. On 2026-11-08 that URL
# answered 504 to every request for hours (about 70 ms from the edge, no redirect, so a retry
# of the same URL cannot recover: `curl --retry 10 --retry-all-errors` failed all 11
# attempts) while the API asset endpoint served the same bytes. `gh release download` goes
# through the API endpoint. `scripts/test_ci_release_downloads.py` fails a workflow that
# fetches a release asset by that URL again.
#
# Needs `gh` and `GH_TOKEN` (`GH_TOKEN: ${{ github.token }}` in the step's `env`).
# GH_RELEASE_DOWNLOAD_ATTEMPTS (default 5) and GH_RELEASE_DOWNLOAD_SLEEP (seconds, default 5)
# set the retry loop: it sleeps SLEEP * attempt after attempt number `attempt` fails, so the
# default waits 5, 10, 15 and 20 seconds before giving up.

set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 <owner/repo> <tag> <asset> <dest>" >&2
  exit 2
fi

repo="$1"
tag="$2"
asset="$3"
dest="$4"
attempts="${GH_RELEASE_DOWNLOAD_ATTEMPTS:-5}"
sleep_unit="${GH_RELEASE_DOWNLOAD_SLEEP:-5}"

for ((attempt = 1; attempt <= attempts; attempt++)); do
  if gh release download "$tag" -R "$repo" -p "$asset" -O "$dest" --clobber; then
    exit 0
  fi
  if ((attempt == attempts)); then
    echo "::error::could not download ${asset} of ${repo} ${tag} after ${attempt} attempts"
    exit 1
  fi
  echo "::warning::download of ${asset} of ${repo} ${tag} failed (attempt ${attempt} of ${attempts}); retrying"
  sleep $((attempt * sleep_unit))
done
