#!/usr/bin/env bash
# Rehearse the Linux half of the release workflow locally, on a clean copy of the repository.
#
# Mirrors the "Linux archives" job in .github/workflows/release.yml step for step:
#   1. copy the tracked files (and nothing else) to a scratch directory, like a fresh checkout
#   2. stamp the release version with packaging/stamp-version.py
#   3. run every test suite exactly as the job does
#   4. build the Linux archives with packaging/package-dist.sh --linux-only
#   5. check that the packaged CLI reports the stamped version
# The repository itself is never modified. The scratch directory is printed and kept on
# failure so it can be inspected, and removed on success.
#
# Usage: packaging/rehearse-release.sh [v1.YYYYMMDD[.N]]   (default: today's date)

set -euo pipefail
# Note for edits: never put `grep -q` at the end of a pipeline here. It exits on the first
# match, the writer then dies of a broken pipe, and pipefail turns that into a failure.

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TAG=${1:-"v1.$(date -u +%Y%m%d)"}
SCRATCH=$(mktemp -d -t link-resolver-rehearsal-XXXXXX)

echo "== Rehearsing release $TAG in $SCRATCH =="
cd "$ROOT"
git ls-files -z | tar --null -cf - --files-from=- | tar -xf - -C "$SCRATCH"
cd "$SCRATCH"

echo "== Stamp the release version =="
python3 packaging/stamp-version.py "$TAG"
crate_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)

echo "== Test (loopback servers only) =="
cargo test --locked
cargo test --locked --no-default-features
python3 -m unittest discover -s tests -v

echo "== Build the Linux archives =="
packaging/package-dist.sh --linux-only

echo "== Check the packaged binaries report $crate_version =="
for binary in dist/gui-linux-x64/link-resolver-cli dist/linux-x64/link-resolver; do
    reported=$("$binary" --version)
    [ "$reported" = "Link Resolver $crate_version" ] \
        || { echo "$binary printed '$reported'" >&2; exit 1; }
    echo "$binary: $reported"
done
tar -tzf dist/link-resolver-gui-linux-x64.tar.gz | grep -x 'link-resolver/link-resolver' >/dev/null \
    || { echo "GUI archive does not unpack to link-resolver/" >&2; exit 1; }
(cd dist && sha256sum -c SHA256SUMS --quiet) && echo "SHA256SUMS verify"

echo "== Rehearsal passed; removing $SCRATCH =="
rm -rf "$SCRATCH"
