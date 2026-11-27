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
