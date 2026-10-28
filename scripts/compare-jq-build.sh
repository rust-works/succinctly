#!/usr/bin/env bash
# Build a pinned jq from source into target/compare/jq-<ver>/ (#5409).
#
# Usage: scripts/compare-jq-build.sh <version>      e.g. 1.7.1
#
# The tarball's sha256 comes from comparison/jq/manifest.toml and is verified
# before anything is unpacked. Prints the installed binary's path on success.
set -euo pipefail

ver="${1:?usage: $0 <version>}"
root="$(cd "$(dirname "$0")/.." && pwd)"
manifest="$root/comparison/jq/manifest.toml"
dest="$root/target/compare/jq-$ver"

if [ -x "$dest/bin/jq" ] && [ "$("$dest/bin/jq" --version)" = "jq-$ver" ]; then
  echo "$dest/bin/jq"
  exit 0
fi

# Pull url and sha for this version out of the manifest's [[reference]] block.
read -r url sha < <(awk -v id="jq-$ver" '
  /^\[\[reference\]\]/ { hit = 0 }
  $1 == "id" && $3 == "\"" id "\"" { hit = 1 }
  hit && $1 == "source_url" { gsub(/"/, "", $3); url = $3 }
  hit && $1 == "source_sha256" { gsub(/"/, "", $3); sha = $3 }
  END { print url, sha }' "$manifest")
[ -n "${url:-}" ] && [ -n "${sha:-}" ] || { echo "jq-$ver not in $manifest" >&2; exit 1; }

sha256_of() {
  if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

work="$root/target/compare/build-jq-$ver"
rm -rf "$work"; mkdir -p "$work"
curl -fsSL -o "$work/jq.tar.gz" "$url"
got="$(sha256_of "$work/jq.tar.gz")"
[ "$got" = "$sha" ] || { echo "sha256 mismatch for $url: got $got, pinned $sha" >&2; exit 1; }

tar -xzf "$work/jq.tar.gz" -C "$work"
cd "$work/jq-$ver"
# Bundled oniguruma, so the regex engine does not depend on the host's.
./configure --prefix="$dest" --with-oniguruma=builtin --disable-docs --disable-shared \
  --enable-static --disable-maintainer-mode >"$work/configure.log" 2>&1
make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)" >"$work/make.log" 2>&1
make install >>"$work/make.log" 2>&1
echo "$dest/bin/jq"
