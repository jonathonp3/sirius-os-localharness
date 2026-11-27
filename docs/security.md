# Security

## Tool policy

```text
read-only   file reads and search only
write       adds workspace writes
full        adds shell, plugins, and MCP
```

`write_file`, `append_file`, `bash`, plugins, and MCP are explicit opt-ins.

## Path safety

File tools canonicalize paths and reject anything outside the workspace. Symlink escapes are rejected.

## Approvals

Destructive commands pause for approval:

- CLI: `[y/N]`
- Web: Run / Deny card; timeout denies
- `--yes` auto-approves for scripts

Matched patterns include `rm`, `dd`, `mkfs`, `sudo`, `git reset --hard`, `git push --force`, package managers, and service changes.

The classifier is conservative and can miss wrapped commands. Use `--dry-run` when unsure.

## Dry run

```bash
harness run --dry-run "..."
```

Mutating tool calls are reported but not executed. Read tools still work.

## Web tools

`--allow-web` enables:

- `web_search`
- `web_fetch`

Restrictions:

- HTTPS only
- port 443 only
- no credentials in URLs
- private and reserved addresses rejected
- redirects revalidated
- page content treated as untrusted data

Page instructions must not be followed. Cite sources.

## Plugins

Plugins declare:

```json
{
  "permissions": {
    "network": false,
    "write": "none",
    "host": false,
    "shell": true
  }
}
```

Host or network plugins require approval.

On the host, command plugins can be sandboxed with `bwrap`:

- read-only `/usr` and `/etc`
- workspace write only when allowed
- isolated network when `network: false`
- isolated PID namespace

Inside the Flatpak, the Flatpak sandbox is the outer boundary; permissions still drive approval.

Review a plugin before installing it. Prefer signed packages and pinned versions.

## MCP

MCP servers are external processes with host access. They are exposed only under the `full` policy.

MCP startup failures are logged and skipped rather than stopping the harness.

## API keys

Keys are never written to `provider.json`.

Preferred:

```bash
export DEEPSEEK_API_KEY=...
export MOONSHOT_API_KEY=...
```

Fallback:

```text
~/.config/local-harness/secrets.env
```

must be mode `0600`.

The Flatpak also checks the host `~/.config/local-harness/secrets.env` when its own config path has no key.

## Prompt injection

Web pages, plugins, MCP results, and documents are untrusted data. The system prompt instructs the model not to follow instructions found inside them.

## Related

- [Plugins](plugins.md)
- [Providers](providers.md)
- [Troubleshooting](troubleshooting.md)
