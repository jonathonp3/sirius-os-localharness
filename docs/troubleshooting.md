# Troubleshooting

## Multiple app instances or stale windows

Several Local Harness windows can compete for the same service and token. Close them all before diagnosing:

```bash
flatpak ps | grep org.jonathonp3.LocalHarness
flatpak kill org.jonathonp3.LocalHarness
systemctl --user stop local-harness-flatpak
```

Then start one fresh instance:

```bash
systemctl --user start local-harness-flatpak
local-harness-window
```

If `flatpak ps` shows more than one `org.jonathonp3.LocalHarness` process, kill the app before reopening. A window opened before a service restart can keep an old token and appear to send messages that never receive a response.

## The app does not respond

Check the service and provider:

```bash
systemctl --user status local-harness-flatpak
curl -s http://127.0.0.1:8787/api/health
```

If the window was open across a restart, its token may be stale. Reopen it:

```bash
local-harness-window
```

The Models panel should show:

```text
ACTIVE FOR NEW CHATS
```

A saved provider is not active until Local Harness restarts.

## DeepSeek key not detected

Check the file first:

```bash
stat -c '%a %U' ~/.config/local-harness/secrets.env
```

Use `600`.

The fallback file is checked in both:

```text
~/.config/local-harness/secrets.env
~/.var/app/org.jonathonp3.LocalHarness/config/local-harness/secrets.env
```

Set the key in one of them. The environment takes precedence.

## DeepSeek responds, Kimi does not

Kimi requires `MOONSHOT_API_KEY`. A missing key shows:

```text
Kimi API · key missing
```

Local and DeepSeek remain available.

## A web page returns 403 / Cloudflare

Use the `cloudflare-fetch` skill. Do not retry the same plain request.

The skill tries:

1. browser user agent
2. `r.jina.ai` rendered text
3. browser rendering
4. public AMP/RSS/metadata

It stops rather than inventing content.

## Model download fails

Check:

```bash
harness plugins list
ls -lh ~/.local/share/llama.cpp/models
```

For gated Hugging Face repos set `HF_TOKEN`.

A resumable download uses `.part`; cancel from the Models panel.

## LFM 3 Omni will not load

Check the llama.cpp revision:

```bash
llama-server --version
```

It must be after 2026-10-27. LFM3 support is PR #31257.

Check that `selected-model.json` contains `mmproj`.

The model log should say:

```text
loaded multimodal model
```

## `distrobox enter` fails with a read-only podman error

This can happen inside a restricted sandbox that mounts `/run/user/1000` read-only. Use the existing container directly:

```bash
XDG_RUNTIME_DIR=/run/user/1000 podman exec llama-dev ...
```

Run the build from a normal host terminal if possible.

## Prompt too large

The governor reports:

```text
prompt exceeds the local budget
```

Reduce pasted content, lower context, or start a new chat. For long files use `read_file` with `offset` and `limit` and `append_file`.

## Nested bwrap unavailable

Inside the Flatpak, nested `bwrap` may be unavailable. The Flatpak sandbox remains the outer boundary. Plugin permissions still require approval.

## CI deployment waiting

The repository uses a protected `flatpak-signing` environment. A maintainer must approve the pending deployment before the release is published.

## Related

- [Installation](install.md)
- [Models](models.md)
- [Providers](providers.md)
- [Security](security.md)
