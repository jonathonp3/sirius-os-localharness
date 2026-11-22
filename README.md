# local-harness

A local-first agent harness: one CLI, one agent loop, four tools, and a
llama.cpp OpenAI-compatible backend. This is the Phase-0 scaffold from
`../LOCAL-AI-HARNESS-BLUEPRINT.md`.

```text
CLI (this binary) ─┐
Browser client ────┴─ HTTP/SSE ── llama.cpp server (127.0.0.1:8080)
     │
     ├── agent loop: prompt → model → tool calls → repeat
     ├── tools: read_file, list_dir, write_file (opt-in), bash (opt-in)
     └── session log: append-only JSONL
```

## Why it is shaped this way

- **CLI first.** The binary is the host and the client. A web UI can be added as
  another client of the same loop; no GUI logic lives here.
- **One generation at a time.** The model server is configured with
  `--parallel 1`; the CLI issues one request at a time. A future `serve`
  command will own a queue rather than firing concurrent requests.
- **Thinking off by default.** Qwen3.5 on this stack generates ~10 tok/s; an
  unbounded reasoning pass is a UX failure. `--think` enables it explicitly.
- **Stable prompt prefix.** System prompt and tool schemas are assembled once
  per run and reused, so llama.cpp's prompt cache stays warm.
- **Bounded tools.** `write_file` and `bash` are off unless requested; every
  path must stay inside the workspace.

## Build

The Silverblue host has no C linker, so the normal development loop builds on
the Fedora VM and copies the binary back:

```bash
./scripts/build-vm.sh          # VM=user@host to override the target
./bin/harness --help
```

The script tars the source to `~/src/local-harness` on the VM, runs
`cargo build --release`, and copies `bin/harness` back. The VM's Rust toolchain
lives in `~/.cargo` (rustup, no root, no rpm-ostree layering).

On a machine with Rust and `cc` installed, `cargo build --release` is enough.

## Run

```bash
# model list
./harness models

# plain question (no tools enabled)
./harness run "In one sentence, what is a KV cache?"

# read-only tools
./harness run "List this directory and summarize what is here."

# writes + shell, explicitly enabled
./harness --workspace "$PWD" run --allow-write --allow-shell \
  "Create notes/todo.md with three bullet points, then show it."

# thinking mode, with a larger budget
./harness run --think --max-tokens 1024 "Plan a small Rust CLI."
```

Defaults:

| Setting | Default | Override |
|---|---|---|
| endpoint | `http://127.0.0.1:8080` | `--endpoint`, `HARNESS_ENDPOINT` |
| model | `Qwen3.5-9B-Q4_K_M` | `--model`, `HARNESS_MODEL` |
| workspace | current directory | `--workspace`, `HARNESS_WORKSPACE` |
| data dir | `$XDG_DATA_HOME/local-harness` or `~/.local/share/local-harness` | `--data-dir`, `HARNESS_DATA_DIR` |
| max steps | 8 | `--max-steps` |
| max tokens/step | 512 | `--max-tokens` |
| thinking | off | `--think` |

Session logs are JSONL files under `<data-dir>/sessions/`; `harness sessions`
lists them.


## Serve (browser client)

```bash
./bin/harness serve --port 8787
# harness serve listening on http://127.0.0.1:8787/?token=<generated>
```

Open the printed URL. The page streams tokens and tool calls over SSE and
runs one chat at a time (matching the model server's single slot). Options:
`--token <token>` (fixed token), `--allow-write`, `--allow-shell`, `--think`,
`--max-steps`, `--max-tokens`.

The server binds loopback only and accepts the token as `?token=` or
`Authorization: Bearer`. Sessions are recorded exactly as in the CLI.

The browser client renders Markdown (headings, lists, tables, links, fenced
code), highlights code with copy buttons, and shows tool calls as collapsible
cards with arguments and result previews. The renderer libraries (markdown-it,
highlight.js, DOMPurify) are vendored into the binary and served locally, so
the UI works offline.
`--concurrency N` allows N chats in flight (default: the model server's slot
count). Real parallelism needs llama.cpp started with `--parallel N`; otherwise
the backend serializes the requests.




## Model registry and presets

Profiles live in `~/.config/local-harness/models.json` (or `--registry`).
A workspace-local `.harness/models.json` is discovered by walking up from the
workspace, so each project can pin its own models; `--registry` still overrides.
The same walk discovers `.harness/plugins.json` and `.harness/mcp.json`, so a
project can carry its own tools. See [examples/models.json](examples/models.json):

```json
{"default": "local-9b", "models": {
  "local-9b": {"model": "Qwen3.5-9B-Q4_K_M", "endpoint": "http://127.0.0.1:8080",
                "context_limit": 8192, "max_tokens": 512, "temperature": 0.0},
  "local-9b-fast": {"model": "Qwen3.5-9B-Q4_K_M", "context_limit": 4096,
                    "max_tokens": 256, "temperature": 0.2}
}}
```

```bash
./bin/harness --registry examples/models.json models
./bin/harness --registry examples/models.json --profile local-9b-fast run "…"
./bin/harness --profile local-9b-fast --context-limit 2048 doctor
```

Precedence is **CLI flags → selected profile → built-in defaults**. `--profile`
defaults to the registry's `default`; `models` lists profiles (marking the
default) followed by the server's `/v1/models`. Env: `HARNESS_PROFILE`,
`HARNESS_REGISTRY`, `HARNESS_TEMPERATURE`.


## Tool policy and plugins

`--tool-policy read-only|write|full` (or the older `--allow-write` /
`--allow-shell`) controls the built-in tools. Profiles may set `tool_policy`.
Every session records its policy as
`{"type":"policy","policy":"…","allow":[…],"deny":[…]}`.

Per-tool scopes narrow that further:

```bash
./bin/harness --allow-tools read_file,list_dir ...      # only these
./bin/harness --deny-tools bash,plugin__wordcount ...   # everything else
```

Comma-separated; `prefix*` wildcards work (`mcp__github__*`). Profiles may set
`allow_tools` / `deny_tools`. Scopes are restrictive — the policy still gates
write/shell and plugin/MCP exposure.

Command-backed plugins are explicit opt-in:

```bash
./bin/harness --plugins examples/plugins.json --allow-shell run \
  "Use the plugin__wordcount tool on README.md."
```

Plugin tools are named `plugin__<name>`, receive JSON arguments, and substitute
shell-quoted `{arg}` placeholders into `command`. They run with the workspace as
cwd, a 60 s timeout, and are only exposed under the `full` policy.

Plugins are sandboxed with `bwrap` by default: read-only `/usr` and `/etc`, the
workspace read-write, **no network**, and an isolated PID namespace. Set
`"sandbox": false` on a plugin only when it genuinely needs the host.

MCP stdio servers work through the same seam:

```bash
./bin/harness --mcp examples/mcp.json --allow-shell run \
  "Use the mcp__echo__echo tool with text 'hello'."
```

`--mcp` supports both transports:

- **stdio** — `{"name": "echo", "command": "python3", "args": [...]}`
- **Streamable HTTP** — `{"name": "http-echo", "url": "http://127.0.0.1:8799/mcp"}`

Each server is initialized, its tools are listed, and exposed as
`mcp__<server>__<tool>` (full policy only). `examples/mcp_echo.py` and
`examples/mcp_http.py` are minimal test servers; `examples/mcp.json` and
`examples/mcp-http.json` are matching configs. HTTPS URLs are rejected by this
build (no TLS backend).

## Sessions: resume and fork

```bash
./bin/harness sessions
./bin/harness run --resume <id> "follow-up question"
./bin/harness run --fork <id> "explore a different branch"
```

`--resume` appends to the same JSONL log; `--fork` creates a new session
seeded with the parent's messages (the parent is untouched). Interrupted
tool-call turns at the end of a log are dropped on load.

## Resource governor

The governor enforces a prompt budget before every model call:

```text
budget = min(--context-limit, model n_ctx) - max_tokens - 256 safety
```

Defaults: `--context-limit 8192`, `--max-tokens 512`. If the prompt exceeds the
budget, it first truncates tool results, then drops the oldest complete turns
(never breaking a tool_call/tool_result pair) and emits a `context:` notice.
If even the latest turn does not fit, the run fails with a clear message.

`harness doctor` prints the model context, slots, budget, RAM/VRAM, and server
health:

```bash
./bin/harness doctor
```



## Workflows

```bash
./bin/harness workflow examples/steps.json
```

```json
{"steps": [
  {"name": "facts", "prompt": "Read Cargo.toml and list the dependencies."},
  {"name": "summary", "prompt": "Using {{facts}}, write one sentence about the stack."}
]}
```

Steps run sequentially as child agents with their own sessions; `{{name}}`
inserts an earlier step's output and `{{prev}}` inserts the previous one.
Maximum 8 steps. Each step is bounded by the usual governor and child limits.

## Subagents

The root agent can delegate one self-contained task to a child with its own
context and JSONL session:

```bash
./bin/harness run --max-depth 1 --max-children 4 \
  "Use the subagent tool to ask a child: what is 17*23?"
```

- `--max-depth 1` (default): the root may spawn children; children cannot spawn
  further. `--max-depth 2` allows one more level.
- `--max-children 4` (default): cap per agent run.
- Children run at most 4 steps and at most 512 output tokens, and share the
  single model slot — delegation isolates context, it does not add parallelism.
- Child logs start with `{"type":"subagent","parent":"<id>","task":"..."}`.


## Packaging and install

```bash
./scripts/package.sh          # builds and creates dist/local-harness-<version>.tar.gz
tar -xzf dist/local-harness-<version>.tar.gz
cd local-harness-<version>
./install.sh
```

The installer puts the binary in `~/.local/bin`, the desktop entry in
`~/.local/share/applications`, and the user unit in
`~/.config/systemd/user` — no root or rpm-ostree layering. Then:

```bash
systemctl --user start local-harness     # host on 127.0.0.1:8787
harness serve --port 8787 --open         # or start + open the browser
```

Sessions live in `~/.local/share/local-harness/sessions`.


## Native window

`local-harness-window` is a GTK4/WebKitGTK 6.0 shell around the web UI. It connects
to the `local-harness-flatpak` user service, starting it if needed, and leaves
the service running when the window closes. If the service cannot start it
falls back to launching the Flatpak host directly with an ephemeral token.

```bash
./packaging/local-harness-window --self-test   # start/verify, print URL, exit
./packaging/local-harness-window                # native window
./packaging/local-harness-window --browser      # hand off to the browser
```

The installer places a host desktop entry with the same app id as the Flatpak
(`org.jonathonp3.LocalHarness.desktop`), so the application menu launches the
native window instead of the browser. The window does not stop a
systemd-managed service; use `systemctl --user stop local-harness-flatpak`.


## Copying tool output

Tool cards have three copy actions:

- **copy command** — only the shell command (safe to paste and run);
- **copy output** — raw output for an editor or document;
- **copy # comment** — every output line prefixed with `#`, so pasting it
  into a shell cannot execute it.

Card labels are non-selectable, so selecting the card and copying picks up the
arguments/output values rather than the `ARGUMENTS`/`RESULT` chrome. Raw
`ls -l` output is not a shell command: pasting it into bash runs each line as a
command (`total: command not found`, etc.). Use **copy command** to re-run, or
**copy # comment** when pasting into a terminal.

## Themes

The header theme selector switches background/palette presets and remembers the
choice in `localStorage`:

- Dark
- Midnight
- Solarized
- **Golden Sand** — derived from `themes/solarized-light-golden-sand.xml`
  (GtkSourceView scheme): golden-sand background, dark base02 text, Solarized
  accent colors, and matching highlight.js token colors.

The selected theme also themes code blocks and tool cards, not just the page
background. `themes/README.md` documents the XML-to-UI mapping.

Answers are guided toward compact bullet lists; for large directory listings
or command output the agent summarizes and lists at most ten relevant entries
instead of reproducing the full transcript.

## Prompt sizing

The prompt box grows automatically as lines are added, up to roughly 65% of the
window height (max 700px). Past that it scrolls. This replaces the old fixed
single-line-height prompt; no manual drag handle is needed.

## Spellcheck

The prompt uses an in-app **en_AU Hunspell dictionary** via Typo.js, so it works
in both the browser and the native WebKit window without relying on the
browser's dictionary:

- red underlines for misspelled words;
- right-click opens a custom menu: spelling suggestions first, then
  Cut / Copy / Paste / Delete / Select all / Insert emoji;
- the native WebKit context menu is suppressed in the native window so the
  custom menu is visible;
- the **Spell** toggle enables/disables it and is remembered in `localStorage`.

The custom menu includes an 80-emoji picker; the old WebKit “insert unicode”
item is not included. Keyboard clipboard shortcuts still work.

The dictionary and Typo.js are bundled under `src/assets/` and served locally;
no network lookup is used. In the native window, the menu's Paste action reads
the system clipboard through a small GTK bridge; browser mode uses the
Clipboard API.

## Interface scale

The header **80%–150%** selector scales the whole UI like Telegram Desktop's
interface scale. The choice is remembered in `localStorage`. Base text, session
metadata, and tool labels were also enlarged for readability.

## Safety

- `read_file` / `list_dir` resolve and canonicalize paths and reject anything
  outside the workspace.
- `write_file` / `bash` are disabled unless the policy enables them.
- `bash` runs non-interactively with a configurable timeout (300 s default)
  and output cap (100 000 characters default).

### Guardrails

- **Dry run** — `--dry-run` reports every mutating tool call
  (`bash`, `write_file`, plugins, MCP) without executing it;
  read tools still work. Tool results are prefixed `DRY-RUN — NOT EXECUTED`, and the agent
  finishes with a “no changes were made” notice.
- **Destructive command approval** — commands matching high-risk patterns
  (`rm`, `dd`, `mkfs`, `sudo`, `git reset --hard`, `git push --force`,
  `flatpak uninstall`, `systemctl`, package managers, etc.) pause for approval:
  - CLI: a `[y/N]` prompt on stdin (EOF means deny).
  - Web: an approval card with **Run** / **Deny** buttons; no response within
    120 s denies.
  - `--yes` auto-approves (for scripts); use it deliberately.
- **Subagents off for the service** — `--max-depth 0` removes the `subagent`
  tool. The user service runs with it, so the model goes straight to `bash`.
- **Tunables** — `HARNESS_BASH_TIMEOUT` seconds (default `300`) and
  `HARNESS_MAX_OUTPUT` characters (default `100000`).
- Tool cards expose a **copy command** button for `bash` (and copy-args for
  other tools); fenced code blocks have a copy button.

The destructive classifier is conservative — it can miss wrapped commands and
can over-match; use dry-run when in doubt.

## Next phases

1. MCP HTTP authentication/TLS.
2. Per-plugin network opt-in and resource limits.
3. Richer project configs (tools/MCP scopes per project).
4. Upstream packaging (RPM/COPR) alongside the Flatpak.
