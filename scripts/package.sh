#!/usr/bin/env bash
# Build a relocatable tarball: binary + installer + desktop entry + user unit.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

./scripts/build-vm.sh
VERSION="$(./bin/harness --version | awk '{print $2}')"
DIST="$ROOT/dist/local-harness-$VERSION"

rm -rf "$DIST"
mkdir -p "$DIST/bin"
cp bin/harness "$DIST/bin/harness"
cp README.md "$DIST/README.md"
cp -r examples "$DIST/examples"
cp -r packaging/flatpak "$DIST/flatpak"
cp packaging/local-harness.desktop packaging/local-harness.service packaging/local-harness-window packaging/install.sh "$DIST/"
chmod +x "$DIST/install.sh"

tar -C "$ROOT/dist" -czf "$ROOT/dist/local-harness-$VERSION.tar.gz" "local-harness-$VERSION"
echo "==> $ROOT/dist/local-harness-$VERSION.tar.gz"
