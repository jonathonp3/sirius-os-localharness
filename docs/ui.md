# UI

The web UI is served at loopback with a generated token.

## Chats and workspaces

The left sidebar is hidden by default. Open it with **Chats** or Ctrl+B.

Supports:

- search
- pin and rename
- delete
- select-all / delete-selected
- export one or many chats
- copy user prompts
- workspace grouping

## Models panel

Shows:

- provider and active model
- saved provider vs active provider
- featured models
- installed GGUF models
- Hugging Face search and download
- model tagging

Local model controls are disabled while a remote provider is active.

## MCP panel

Shows connected servers, transport, tools, and failures.

## Automation tasks

The header **Tasks** button opens a panel with Enabled, Paused, Ended, and All
tabs, search, a create form, and a Daily/Weekly review. See
[automation.md](automation.md).

## Artifacts

The artifact panel is hidden by default. When a tool result contains a workspace
image or code file, Local Harness adds a thumbnail or "Open" button to the tool
card. Clicking it opens the panel.

Supported:

- images: PNG, JPEG, WebP, GIF, SVG
- code/text: Python, shell, JavaScript/TypeScript, JSON, Markdown, HTML, CSS,
  Rust, TOML, YAML, XML, text, logs

Files are served only from the workspace, with a 25 MB limit. The panel is for
display only; it does not edit artifacts.

For assistant output, a line such as:

```text
Artifact: /absolute/path/to/export.png
```

is rendered as an inline thumbnail in the response. Clicking the thumbnail opens
the artifact panel. Code artifacts appear as an "Open" button.

## Markdown and code

- Markdown rendering
- syntax highlighting
- copy buttons for code blocks
- tool cards with copy command, copy output, and copy-as-comment

## Themes

The theme selector changes the full palette, including code blocks and tool cards.

Available:

- Xterm
- Solarized
- Golden Sand
- Ember Orange
- Crimson
- Cobalt Blue

Theme XML mapping is documented in [themes/README.md](../themes/README.md).

## Scale

The header selector scales the whole interface from 80% to 150% and remembers the choice.

## Spellcheck

The prompt uses a bundled `en_AU` Hunspell dictionary through Typo.js.

- red underlines
- suggestions in a custom context menu
- Cut / Copy / Paste / Delete / Select all
- emoji picker
- toggle remembered in `localStorage`

No network lookup is used.

## Prompt sizing

The prompt grows with content up to roughly 65% of the window height, then scrolls. The action bar stays fixed below it.

## Native window

`local-harness-window` wraps the UI in GTK4/WebKitGTK. It connects to the Flatpak host service and leaves it running when the window closes.

## Related

- [Installation](install.md)
- [Troubleshooting](troubleshooting.md)
