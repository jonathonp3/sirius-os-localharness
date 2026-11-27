# Providers

Local Harness uses one OpenAI-compatible client for local and remote providers.

## Local llama.cpp

Default:

```text
endpoint: http://127.0.0.1:8080
provider: local
```

The local server must expose:

```text
/v1/chat/completions
/health
/v1/models
```

## Remote profiles

Remote profiles are declared in `~/.config/local-harness/models.json` or an explicit `--registry`.

Example:

```json
{
  "default": "deepseek-chat",
  "models": {
    "deepseek-chat": {
      "model": "deepseek-chat",
      "endpoint": "https://api.deepseek.com",
      "provider": "openai",
      "api_key_env": "DEEPSEEK_API_KEY",
      "context_limit": 65536,
      "max_tokens": 4096,
      "tool_policy": "full"
    },
    "kimi-k2": {
      "model": "kimi-k2-0711-preview",
      "endpoint": "https://api.moonshot.ai/v1",
      "provider": "openai",
      "api_key_env": "MOONSHOT_API_KEY",
      "context_limit": 131072,
      "max_tokens": 4096,
      "tool_policy": "full"
    }
  }
}
```

A repository example is [examples/models.remote.json](../examples/models.remote.json).

## Provider selector

The Models panel has a provider dropdown:

```text
Local llama.cpp
DeepSeek API
Kimi API
```

It shows:

```text
ACTIVE FOR NEW CHATS
SAVED CHOICE
```

Changing the saved choice requires restarting Local Harness. Local model cards are disabled while a remote provider is active.

The saved choice is:

```text
~/.config/local-harness/provider.json
```

```json
{
  "active": "deepseek"
}
```

Valid values: `local`, `deepseek`, `kimi`.

## API keys

Keys are never stored in `provider.json`.

Preferred:

```bash
export DEEPSEEK_API_KEY=...
export MOONSHOT_API_KEY=...
```

Fallback:

```text
~/.config/local-harness/secrets.env
```

```bash
chmod 600 ~/.config/local-harness/secrets.env
```

```text
DEEPSEEK_API_KEY=...
MOONSHOT_API_KEY=...
```

The loader also accepts `export KEY=value` and, inside the Flatpak, falls back to the host `~/.config` path.

The file must not be readable by group or others.

## Context and tokens

Remote providers do not expose llama.cpp `/props` or `/tokenize`.

- context comes from `context_limit`
- exact token counting is unavailable
- the local governor uses estimates and profile limits

## Tool calling

Tool calling depends on the selected model and API. Use a model that supports function calling for agent work.

## CLI examples

```bash
harness --registry examples/models.remote.json --profile deepseek-chat run "..."
harness --registry examples/models.remote.json --profile kimi-k2 run "..."
```

## Related

- [Models](models.md)
- [Security](security.md)
- [Troubleshooting](troubleshooting.md)
