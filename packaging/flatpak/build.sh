#!/usr/bin/env bash
# Build/install the Flatpak locally. Needs flatpak-builder and the SDK/runtime.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
if ! command -v flatpak-builder >/dev/null; then
  echo "flatpak-builder is required (org.flatpak.Builder or the host package)" >&2
  exit 1
fi
cargo build --release --locked
cp target/release/harness packaging/flatpak/harness
trap 'rm -f packaging/flatpak/harness' EXIT
flatpak-builder --user --force-clean --install-deps-from=flathub \
  build packaging/flatpak/org.jonathonp3.LocalHarness.json
echo "installed: flatpak run org.jonathonp3.LocalHarness"
