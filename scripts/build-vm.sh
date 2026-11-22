#!/usr/bin/env bash
# Build the Rust binary on the Fedora VM (which has a C linker) and copy it back.
# The Silverblue host has no gcc, so this is the normal development loop.
set -euo pipefail

VM="${VM:-jonathon@192.168.100.48}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> syncing to $VM"
tar -czf - \
  --exclude=./target --exclude=./bin --exclude=./.harness-data --exclude=./demo \
  . | ssh -F /dev/null -o BatchMode=yes "$VM" \
  'mkdir -p ~/src/local-harness && tar -C ~/src/local-harness -xzf -'

echo "==> cargo build --release"
ssh -F /dev/null -o BatchMode=yes "$VM" \
  'cd ~/src/local-harness && ~/.cargo/bin/cargo build --release'

echo "==> copying binary back"
mkdir -p "$ROOT/bin"
scp -F /dev/null -o BatchMode=yes \
  "$VM:~/src/local-harness/target/release/harness" "$ROOT/bin/harness"
scp -F /dev/null -o BatchMode=yes \
  "$VM:~/src/local-harness/Cargo.lock" "$ROOT/Cargo.lock"
chmod +x "$ROOT/bin/harness"
echo "==> built $ROOT/bin/harness"
