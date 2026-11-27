# Installation

## Flatpak

```bash
wget2 -q -O /tmp/sirius-os-localharness.gpg \
  https://raw.githubusercontent.com/jonathonp3/sirius-os-localharness/main/packaging/flatpak/local-harness.gpg

flatpak remote-add --system --if-not-exists \
  --gpg-import=/tmp/sirius-os-localharness.gpg \
  local-harness https://jonathonp3.github.io/sirius-os-localharness/local-harness.flatpakrepo

flatpak install --system local-harness org.jonathonp3.LocalHarness
flatpak run org.jonathonp3.LocalHarness
```

Apply updates and restart the host service:

```bash
flatpak update --system org.jonathonp3.LocalHarness
systemctl --user restart local-harness-flatpak
local-harness-window
```

## Native window

`local-harness-window` is a GTK4/WebKitGTK shell around the same web UI. It starts `local-harness-flatpak` if needed and leaves the service running when the window closes.

```bash
local-harness-window
local-harness-window --browser
local-harness-window --self-test
```

## Host prerequisites

llama.cpp is a separate host service. See [prerequisites.md](prerequisites.md) for:

- distrobox/podman setup
- Vulkan build dependencies
- helper wrappers
- selected-model and mmproj files
- `llama-server` and `local-harness-flatpak` systemd units

## Source build

On a machine with Rust and a C linker:

```bash
cargo build --release --locked
./target/release/harness --help
```

On a Silverblue host without a linker, the normal loop builds on a Fedora VM:

```bash
./scripts/build-vm.sh
```

The script tars the source to the VM, runs `cargo build --release`, and copies `bin/harness` back. Override the target with `VM=user@host`.

## Tarball install

```bash
./scripts/package.sh
tar -xzf dist/local-harness-<version>.tar.gz
cd local-harness-<version>
./install.sh
```

The installer places the binary in `~/.local/bin`, a desktop entry in `~/.local/share/applications`, and a user unit in `~/.config/systemd/user`. No root or rpm-ostree layering is required.

## Services

```bash
systemctl --user status local-harness-flatpak
systemctl --user restart local-harness-flatpak
```

The Flatpak host runs the web service on loopback. The llama.cpp service is separate:

```bash
systemctl --user status llama-server
systemctl --user restart llama-server
```

## CI

The GitHub Actions workflow builds the binary, runs `cargo test --release --locked`, builds the Flatpak, signs the repository, and deploys GitHub Pages.

## Related

- [Models](models.md)
- [Providers](providers.md)
- [Troubleshooting](troubleshooting.md)
