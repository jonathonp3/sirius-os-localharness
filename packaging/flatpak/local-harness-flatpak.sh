#!/bin/sh
# Flatpak entrypoint: host + browser bridge (--open uses flatpak-spawn).
exec /app/bin/harness --workspace "$HOME" serve --port 8787 --open
