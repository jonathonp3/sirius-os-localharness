# Prerequisites and host setup

This page covers the moving parts around llama.cpp: the distrobox container, the Vulkan build, helper scripts, selected-model metadata, and the systemd user units.

The Flatpak is self-contained for the harness. The model server is a separate host service.

```text
systemd user
├── llama-server.service       → llama-dev distrobox → llama-server :8080
├── local-harness-flatpak.service → Flatpak harness → :8787
└── local-harness-system-profile.timer → system profile refresh
```

## 1. Host tools

On Fedora Silverblue:

```bash
sudo rpm-ostree install distrobox podman
```

If those are already present, skip the layering. `distrobox` and `podman` are also available from Flathub or a toolbox image.

Optional host package:

```bash
sudo rpm-ostree install fuse fuse-libs
```

`fuse` and `fuse-libs` are not required by llama.cpp or Local Harness itself. They are useful if you:

- build Flatpaks locally with FUSE-backed filesystems
- run AppImages that mount their squashfs image through FUSE
- use other FUSE-dependent container or image tooling

CI builds pass `--disable-rofiles-fuse`, so CI does not need a working FUSE mount. AppImages can also run with `--appimage-extract-and-run` when FUSE is unavailable.

Verified on 2026-11-27 on a Fedora 45 VM:

```text
fuse:       not installed
fuse-libs:  not installed
fuse3:      installed (host base)
cmake/g++:  installed only inside a Fedora build container
llama.cpp:  CPU build succeeded
```

The build produced a working `llama-server`; FUSE was not required.

Verify:

```bash
distrobox --version
podman --version
fusermount --version || fusermount3 --version
```

## 2. Create the llama container

The wrapper expects the name `llama-dev`:

```bash
distrobox create \
  --name llama-dev \
  --image registry.fedoraproject.org/fedora:latest
```

Enter it:

```bash
distrobox enter llama-dev
```

Inside the container:

```bash
sudo dnf install -y \
  git cmake gcc-c++ make \
  vulkan-loader-devel vulkan-tools glslc \
  openssl-devel python3
```

Package names vary by Fedora release. `vulkaninfo` and `glslc` are the important build checks. FUSE is a host-side packaging dependency, not a container build dependency:

```bash
vulkaninfo --summary
glslc --version
```

If `glslc` is not packaged separately, install the Vulkan SDK package that provides it.

## 3. Build llama.cpp with Vulkan

Clone into the path used by the wrappers:

```bash
git clone https://github.com/ggml-org/llama.cpp \
  ~/.local/share/llama.cpp

cd ~/.local/share/llama.cpp
```

Configure a clean build:

```bash
cmake -S . -B build \
  -DCMAKE_BUILD_TYPE=Release \
  -DGGML_VULKAN=ON
```

Build:

```bash
cmake --build build -j"$(nproc)"
```

Binaries:

```text
~/.local/share/llama.cpp/build/bin/llama-server
~/.local/share/llama.cpp/build/bin/llama-cli
```

The wrapper uses `LD_LIBRARY_PATH` from that `bin` directory.

### Rebuilding after an update

Use a fresh build directory if the old CMake cache points at a different mount path:

```bash
cmake -S . -B build-lfm \
  -DCMAKE_BUILD_TYPE=Release \
  -DGGML_VULKAN=ON
cmake --build build-lfm -j"$(nproc)"
```

Then:

```bash
mv build/bin build-old-bin
cp -a build-lfm/bin build/bin
systemctl --user restart llama-server
```

## 4. Install the helper wrappers

From the packaged release:

```bash
install -Dm755 local-harness-llama-start \
  ~/.local/bin/local-harness-llama-start
install -Dm755 local-harness-modelctl \
  ~/.local/bin/local-harness-modelctl
```

From this repository:

```bash
cd packaging
install -Dm755 local-harness-llama-start \
  ~/.local/bin/local-harness-llama-start
install -Dm755 local-harness-modelctl \
  ~/.local/bin/local-harness-modelctl
```

Verify:

```bash
~/.local/bin/local-harness-modelctl list
```

## 5. Models and selected state

Models are stored in:

```text
~/.local/share/llama.cpp/models
```

The current model is selected by:

```text
~/.config/local-harness/selected-model.json
```

Example:

```json
{
  "path": "/home/you/.local/share/llama.cpp/models/Qwen3.5-9B-Q4_K_M.gguf",
  "alias": "Qwen3.5-9B-Q4_K_M",
  "name": "Qwen3.5-9B-Q4_K_M"
}
```

A multimodal model adds:

```json
"mmproj": "/home/you/.local/share/llama.cpp/models/mmproj-....gguf"
```

`local-harness-modelctl` writes this file. Do not edit it while a switch is running.

## 6. llama systemd unit

Create:

```text
~/.config/systemd/user/llama-server.service
```

```ini
[Unit]
Description=llama.cpp server in Distrobox
After=default.target

[Service]
Type=simple
ExecStart=%h/.local/bin/local-harness-llama-start
ExecStop=/usr/bin/podman stop --time 10 --ignore llama-dev
Restart=on-failure
RestartSec=5
TimeoutStopSec=60
LimitNOFILE=1048576

[Install]
WantedBy=default.target
```

The `ExecStop` line is important: it stops the container explicitly so model switches do not wait for the systemd stop timeout.

Reload and start:

```bash
systemctl --user daemon-reload
systemctl --user enable --now llama-server
systemctl --user status llama-server
```

Health check:

```bash
curl -s http://127.0.0.1:8080/health
```

## 7. Harness host service

The Flatpak host service needs a token and port:

```bash
install -d ~/.config/local-harness
printf 'PORT=8787\nTOKEN=%s\n' "$(openssl rand -hex 16)" \
  > ~/.config/local-harness/flatpak.env
chmod 600 ~/.config/local-harness/flatpak.env
```

The host launcher reads that file:

```text
~/.local/bin/local-harness-serve
```

It starts the Flatpak harness and passes the workspace, policy, and token.

Unit:

```ini
[Unit]
Description=Local Harness Flatpak host
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=%h
ExecStartPre=-/usr/bin/flatpak kill org.jonathonp3.LocalHarness
ExecStart=%h/.local/bin/local-harness-serve
ExecStop=-/usr/bin/flatpak kill org.jonathonp3.LocalHarness
KillMode=control-group
Restart=on-failure
RestartSec=2
TimeoutStopSec=15

[Install]
WantedBy=default.target
```

Then:

```bash
systemctl --user daemon-reload
systemctl --user enable --now local-harness-flatpak
```

## 8. Boot at login / boot

To keep user services running without an active login:

```bash
loginctl enable-linger "$USER"
```

Check:

```bash
loginctl show-user "$USER" -p Linger
```

## 9. System profile timer

The `linux-ops` plugin provides the profile collector. Install it first:

```bash
harness plugins install-catalog linux-ops
```

The service runs:

```text
%h/.config/local-harness/plugins/linux-ops/bin/system_profile.py
```

Example timer:

```ini
[Unit]
Description=Refresh Local Harness system profile periodically

[Timer]
OnBootSec=2min
OnUnitActiveSec=15min
Persistent=true

[Install]
WantedBy=timers.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now local-harness-system-profile.timer
```

## 10. Verify the moving parts

```bash
systemctl --user is-active llama-server
systemctl --user is-active local-harness-flatpak
curl -s http://127.0.0.1:8080/health
curl -s http://127.0.0.1:8787/api/health
```

If the model is loaded:

```bash
curl -s http://127.0.0.1:8080/v1/models
```

The Models panel should show the same active model.

## Distrobox quirks

- The unit runs `distrobox enter --name llama-dev --no-tty`.
- Model switching uses `systemctl --user restart llama-server`.
- `podman stop llama-dev` is the explicit stop path.
- Rootless podman needs a writable runtime directory; do not run `distrobox enter` from a sandbox that mounts `/run/user/1000` read-only.
- If a sandbox blocks `distrobox enter`, use `podman exec` against the already-running container for builds.

## Related

- [Models](models.md)
- [Installation](install.md)
- [Troubleshooting](troubleshooting.md)
