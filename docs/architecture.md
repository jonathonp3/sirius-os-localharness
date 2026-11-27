# Architecture

```text
CLI / browser / native window
            │
        HTTP + SSE
            │
        agent loop
   ┌────────┼────────┐
 model    tools    sessions
            │
   plugins / MCP / workspaces
```

## Agent loop

Each step:

1. build the system prompt and tool definitions
2. apply the context budget
3. stream a model response
4. execute tool calls
5. append results
6. repeat until a final answer or the step limit

The prompt prefix is stable so llama.cpp can reuse its prompt cache.

## Context governor

```text
budget = min(context_limit, model n_ctx) - max_tokens - safety
```

When over budget:

1. truncate large tool results
2. drop oldest complete turns
3. keep tool-call/result pairs together
4. fail clearly if the latest turn still does not fit

Large tool results get handles that can be retrieved later with `tool_result_get`.

## Sessions

Sessions are append-only JSONL files under the data directory.

```text
~/.local/share/local-harness/sessions/
```

Features:

- resume
- fork
- search
- export
- soft-delete to trash
- pin and rename
- file history

## Workspaces

A workspace pins the directory used by file and shell tools. `Home` uses the service workspace; named workspaces can point at explicit paths.

Sessions are grouped by workspace in the sidebar.

## Workflows

```bash
harness workflow examples/steps.json
```

Steps run sequentially as child agents. `{{name}}` inserts an earlier step result.

## Subagents

The root agent can delegate one self-contained task to a child with its own context and session.

```bash
harness run --max-depth 1 --max-children 4 "..."
```

Children share the single model slot; delegation isolates context, not compute.

## Tools

Core tools include:

```text
read_file, write_file, append_file, list_dir, grep, glob
file_history, session_search, tool_result_get, skill
bash, host_bash
web_search, web_fetch
```

Policy controls exposure:

```text
read-only, write, full
```

## Model client

The client speaks the OpenAI-compatible chat API:

```text
POST /v1/chat/completions
```

Local llama.cpp and remote providers differ in:

- context discovery
- token counting
- API-key headers
- server-specific request fields

Remote profiles skip llama.cpp-only probes.

## Plugins and MCP

- Plugins expose command-backed tools and skills.
- MCP exposes external stateful servers.
- Both are explicit opt-ins under the `full` policy.
- Plugin permissions and approval guard host and network access.

## Related

- [Models](models.md)
- [Plugins](plugins.md)
- [Security](security.md)
