# Local Harness

A local-first agent harness for llama.cpp and OpenAI-compatible APIs. One agent loop, a browser UI, tool calling, approval gates, sessions, skills, and versioned plugins.

```text
Browser or CLI ── HTTP/SSE ── harness ── llama.cpp / DeepSeek / Kimi
                                ├── tools and approvals
                                ├── sessions and workspaces
                                ├── skills
                                └── plugins and MCP
```

## Features

- **Local or remote models.** Run GGUF models through llama.cpp, or use DeepSeek and Kimi profiles with API keys.
- **One agent loop.** CLI, browser UI, workflows, and subagents share the same tool and session machinery.
- **Safe tools.** Read-only by default; write, shell, plugins, and MCP are explicit opt-ins.
- **Approvals.** Destructive commands and host-level plugins pause for confirmation.
- **Sessions and workspaces.** Resume, fork, search, export, pin, and organise chats.
- **Context management.** Prompt budgeting, tool-result eviction, restore handles, and long-document support.
- **Skills.** Reusable procedures installed from local files or plugin packages.
- **Plugins.** Versioned packages with permissions, SHA-256 verification, rollback, a catalog, and optional MCP bundles.
- **Model downloads.** Hugging Face GGUF downloads, progress, resume, curated entries, and paired `mmproj` support.
- **PhotoCraft.** MCP-based design tools with deterministic validation and a read-only vision critic.
- **Offline UI.** Markdown, code highlighting, spellcheck, themes, and the native window work without external assets.

## Install

Flatpak:

```bash
wget2 -q -O /tmp/sirius-os-localharness.gpg \
  https://raw.githubusercontent.com/jonathonp3/sirius-os-localharness/main/packaging/flatpak/local-harness.gpg

flatpak remote-add --system --if-not-exists \
  --gpg-import=/tmp/sirius-os-localharness.gpg \
  local-harness https://jonathonp3.github.io/sirius-os-localharness/local-harness.flatpakrepo

flatpak install --system local-harness org.jonathonp3.LocalHarness
flatpak run org.jonathonp3.LocalHarness
```

Update and restart:

```bash
flatpak update --system org.jonathonp3.LocalHarness
systemctl --user restart local-harness-flatpak
local-harness-window
```

Source builds are covered in [docs/install.md](docs/install.md).

## Models and providers

Local llama.cpp is the default. The Models panel can download GGUFs, switch models, and select DeepSeek or Kimi.

API keys are never stored in `provider.json`. Use:

```bash
export DEEPSEEK_API_KEY=...
export MOONSHOT_API_KEY=...
```

or a mode-`0600` file at `~/.config/local-harness/secrets.env`.

See:

- [docs/models.md](docs/models.md)
- [docs/providers.md](docs/providers.md)

## Plugins and skills

Install from the catalog:

```bash
harness plugins catalog
harness plugins install-catalog linux-ops
harness plugins install-catalog markdown-formatter
harness plugins install-catalog photocraft
```

Plugins are self-contained: they can ship tools, skills, and MCP configuration.

- [docs/plugins.md](docs/plugins.md)
- [docs/skills.md](docs/skills.md)
- [Plugin catalog](plugins.json)
- [Releases](https://github.com/jonathonp3/sirius-os-localharness/releases)

## PhotoCraft

PhotoCraft runs as an MCP server. The design workflow separates:

```text
Qwen3-Coder       → tool selection and parameters
PhotoCraft        → structured operations
document_check    → deterministic validation
vision_critique   → observations only
document_export   → strict final gate
```

Details, prerequisites, and the validation workflow:

[docs/photocraft.md](docs/photocraft.md)

## Documentation

| Topic | Document |
|---|---|
| Installation and builds | [docs/install.md](docs/install.md) |
| Models, downloads, mmproj | [docs/models.md](docs/models.md) |
| Local and remote providers | [docs/providers.md](docs/providers.md) |
| Plugins, catalog, releases | [docs/plugins.md](docs/plugins.md) |
| Skills | [docs/skills.md](docs/skills.md) |
| PhotoCraft MCP | [docs/photocraft.md](docs/photocraft.md) |
| Architecture and context | [docs/architecture.md](docs/architecture.md) |
| UI, chats, themes | [docs/ui.md](docs/ui.md) |
| Security and approvals | [docs/security.md](docs/security.md) |
| Troubleshooting | [docs/troubleshooting.md](docs/troubleshooting.md) |

## Status

- Local text and tool models are the primary supported path.
- LFM 3 Omni requires a current llama.cpp build; audio input is experimental.
- Remote providers depend on their APIs, model names, and tool-calling support.
- Plugins can request host or network access; review their permissions before installing.

## Development

```bash
cargo test --release --locked
cargo build --release --locked
```

CI builds the binary, runs the regression tests, builds the Flatpak, and deploys the signed repository.

See [docs/install.md](docs/install.md) for the host/VM build loop.
