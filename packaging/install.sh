#!/usr/bin/env bash
# Install Local Harness into the user's home. No root, no rpm-ostree layering.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
UNIT_DIR="${HOME}/.config/systemd/user"
APP_DIR="${HOME}/.local/share/applications"

install -Dm755 "$ROOT/bin/harness" "$BIN_DIR/harness"
install -Dm755 "$ROOT/local-harness-window" "$BIN_DIR/local-harness-window"
install -d "$APP_DIR" "$UNIT_DIR"
sed "s|@HOME@|${HOME}|g" "$ROOT/local-harness.desktop" > "$APP_DIR/local-harness.desktop"
chmod 644 "$APP_DIR/local-harness.desktop"
install -Dm644 "$ROOT/local-harness.service" "$UNIT_DIR/local-harness.service"

systemctl --user daemon-reload 2>/dev/null || true
echo "installed: $BIN_DIR/harness"
echo "start now:  systemctl --user start local-harness"
echo "open UI:    $BIN_DIR/harness serve --port 8787 --open"
