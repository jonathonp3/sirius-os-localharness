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

`local-harness-window` opens the UI in a GTK4/WebKit2 window (the installed
desktop entry uses it). It starts `harness serve` on a free loopback port,
waits for health, and stops the server when the window closes — no Electron or
Tauri toolchain required.

```bash
./packaging/local-harness-window                # source tree
./packaging/local-harness-window --self-test    # start server, print URL, exit
```

Silverblue already ships GTK4 and WebKit2, so this is the native shell path;
Flatpak packaging is unnecessary for a localhost-only host.


## Flatpak (GitHub-hosted signed repo)

The GitHub repo builds a signed Flatpak on every push to `main`, publishes the
bundle as release `latest-build`, and deploys a signed repository to GitHub
Pages (`.github/workflows/build.yml`, key
`65269CFC0D32C2CA632C48C5F8AF682DDA82A240`).

```bash
flatpak remote-add --if-not-exists local-harness \
  https://jonathonp3.github.io/sirius-os-localharness/local-harness.flatpakrepo
flatpak install local-harness org.jonathonp3.LocalHarness
```

Or install the release bundle directly:

```bash
flatpak install --from local-harness-stable.flatpak
```

Local build (needs `flatpak-builder` + SDK):

```bash
./packaging/flatpak/build.sh
```

`--open` detects `FLATPAK_ID` and uses `flatpak-spawn --host xdg-open`, so the
UI opens in the host browser while the host stays sandboxed. Nested `bwrap` is
not permitted inside Flatpak, so plugin commands run directly inside the
Flatpak sandbox there (on the host they still use `bwrap`). Flathub remains
optional; see `packaging/flatpak/FLATHUB.md`.

## Safety

- `read_file` / `list_dir` resolve and canonicalize paths and reject anything
  outside the workspace.
- `write_file` / `bash` are disabled unless the run explicitly opts in.
- `bash` runs non-interactively with a 60 s timeout and a 20k-character output
  cap.

## Next phases

1. MCP HTTP authentication/TLS.
2. Per-plugin network opt-in and resource limits.
3. Richer project configs (tools/MCP scopes per project).
4. Upstream packaging (RPM/COPR) alongside the Flatpak.
