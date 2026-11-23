#!/usr/bin/env bash
# Install Local Harness into the user's home. No root, no rpm-ostree layering.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
UNIT_DIR="${HOME}/.config/systemd/user"
APP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/scalable/apps"

install -Dm755 "$ROOT/bin/harness" "$BIN_DIR/harness"
install -Dm755 "$ROOT/local-harness-window" "$BIN_DIR/local-harness-window"
install -Dm755 "$ROOT/local-harness-modelctl" "$BIN_DIR/local-harness-modelctl"
install -Dm755 "$ROOT/local-harness-llama-start" "$BIN_DIR/local-harness-llama-start"
install -d "$APP_DIR" "$UNIT_DIR" "$ICON_DIR"
sed "s|@HOME@|${HOME}|g" "$ROOT/local-harness.desktop" > "$APP_DIR/org.jonathonp3.LocalHarness.desktop"
chmod 644 "$APP_DIR/org.jonathonp3.LocalHarness.desktop"
install -Dm644 "$ROOT/org.jonathonp3.LocalHarness.svg" "$ICON_DIR/org.jonathonp3.LocalHarness.svg"
install -Dm644 "$ROOT/local-harness.service" "$UNIT_DIR/local-harness.service"

systemctl --user daemon-reload 2>/dev/null || true
echo "installed: $BIN_DIR/harness"
echo "start now:  systemctl --user start local-harness"
echo "open native: $BIN_DIR/local-harness-window"
