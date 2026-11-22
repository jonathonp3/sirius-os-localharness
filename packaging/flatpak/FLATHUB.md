# Flathub submission checklist

The CI workflow (`.github/workflows/flatpak.yml`) builds the bundle from this
checkout. Flathub itself requires a public source repository and a few extra
steps:

1. Push this project to a public git repository and tag a release
   (`v0.1.0`).
2. Fork `flathub/flathub`, create a branch, and copy
   `org.jonathonp3.LocalHarness.json` to the repository root.
3. Replace the local directory source with a `git` source pinned to the tag:
   ```json
   { "type": "git", "url": "https://github.com/<owner>/local-harness.git",
     "tag": "v0.1.0" }
   ```
   Flathub does not accept `type: dir` sources.
4. Add `flathub.json` with the supported arches and submit the icon.
5. Open a pull request; Flathub review checks the app id, icon, build, and
   permissions.
6. Once accepted, releases are built by Flathub from the tag.

Current permissions: network, home filesystem, Wayland/X11, IPC, and
`org.freedesktop.Flatpak` (used to open the host browser via
`flatpak-spawn --host xdg-open`). `--open` in the app detects `FLATPAK_ID`.

This submission was not performed from this environment: it requires a public
GitHub repository, a Flathub fork/PR, and review.
