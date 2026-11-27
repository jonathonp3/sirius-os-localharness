# Plugins

Plugins are versioned packages that add command-backed tools, skills, and optionally MCP server configuration.

## Layout

```text
plugins/<name>/
├── plugin.json
├── bin/
│   └── tool-script
└── skills/
    └── <skill>/
        └── SKILL.md
```

`plugin.json`:

```json
{
  "name": "example",
  "version": "1.0.0",
  "revision": 1,
  "api": "1",
  "min_harness": "0.1.0",
  "description": "Example plugin",
  "permissions": {
    "network": false,
    "write": "none",
    "host": false,
    "shell": true
  },
  "skills": ["skills/example/SKILL.md"],
  "tools": [
    {
      "name": "example_tool",
      "description": "Do one thing.",
      "command": "python3 bin/tool.py '{value}'",
      "parameters": {
        "value": { "type": "string" }
      },
      "required": ["value"],
      "sandbox": false
    }
  ]
}
```

Tool names are exposed as:

```text
plugin__example_tool
```

## Permissions

- `network` — nested sandbox network access
- `write` — `none` or `workspace`
- `host` — host-level access; requires approval
- `shell` — shell execution

A host or network plugin requires approval before it runs.

## Installation

From a directory or tarball:

```bash
harness plugins install ./plugins/example
harness plugins install ./dist/example-1.0.0.tar.gz
```

From HTTPS:

```bash
harness plugins install-url https://example.com/example-1.0.0.tar.gz
```

From the catalog:

```bash
harness plugins catalog
harness plugins install-catalog example
harness plugins install-catalog example --version 1.0.0
```

The default catalog is:

[plugins.json](../plugins.json)

## Versions and rollback

```bash
harness plugins update ./plugins/example
harness plugins rollback example
```

Previous versions are preserved under:

```text
~/.config/local-harness/plugins/versions/
```

## Signatures

Optional Ed25519 signatures:

```text
plugin.json.sig
```

with:

```text
HARNESS_PLUGIN_PUBKEY=<32-byte hex public key>
```

The catalog also supports SHA-256 archive digests.

## Skills in plugins

When a plugin declares `skills`, installation copies them to:

```text
~/.config/local-harness/skills/<name>/SKILL.md
```

Bootstrap installs the bundled `linux-ops` skills the same way.

## MCP bundles

A package without tools may declare an MCP server:

```json
{
  "name": "photocraft",
  "version": "1.0.0",
  "skills": ["skills/photocraft-design/SKILL.md"],
  "mcp": {
    "name": "photocraft",
    "command": "/usr/bin/flatpak-spawn",
    "args": ["--host", "%h/.local/bin/photocraft-design-mcp"]
  }
}
```

Installation merges the server into:

```text
~/.harness/mcp.json
```

`%h` expands to the current home directory.

## Why PhotoCraft is an MCP bundle

PhotoCraft is stateful and multi-tool. MCP provides:

- an active document and layer tree
- tool discovery
- process lifecycle
- deterministic validators

A normal command plugin would duplicate that state. The MCP bundle only packages configuration and skills.

## Security

Review a plugin before installing it. Plugins can request host access, network access, or workspace writes. Prefer signed packages and pinned versions.

## Related

- [Skills](skills.md)
- [PhotoCraft](photocraft.md)
- [Security](security.md)
