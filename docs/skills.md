# Skills

Skills are Markdown procedures loaded lazily by the local agent.

## Locations

```text
~/.config/local-harness/skills/<name>/SKILL.md
```

Plugin packages may also ship skills. Installing the plugin copies them into the same directory.

## Format

```markdown
---
name: example
display-name: Example
description: Use this procedure when the task matches this description.
---

# Example

1. Read the input.
2. Apply the procedure.
3. Verify the result.
```

The frontmatter `description` is what the model sees when choosing a skill.

## Usage

In chat:

```text
@example
```

or:

```text
Use the skill tool to load example.
```

The agent can also load a skill by name when the task matches its description.

## Installed skills

Current repository skills:

- `cloudflare-fetch`
- `long-document-formatter`
- `markdown-formatter` (inside its plugin)
- `photocraft-design` (inside the PhotoCraft bundle)
- `journal-check` (inside `linux-ops`)

## Long documents

Use `long-document-formatter` with:

```text
read_file offset/limit
append_file
```

The agent reads a chunk, formats it, appends it, and verifies the assembled file.

## Cloudflare

Use `cloudflare-fetch` when a public page returns:

```text
403 Forbidden
Just a moment...
```

It tries a rendered-text reader and public alternates. It does not bypass CAPTCHAs, authentication, or paywalls.

## PhotoCraft

Use `photocraft-design` for the role-separated design workflow:

```text
document_inspect
document_check
document_export
vision_critique
```

The deterministic validator is authoritative; the vision critic only observes.

## Related

- [Plugins](plugins.md)
- [PhotoCraft](photocraft.md)
- [Markdown formatter](../plugins/markdown-formatter/skills/markdown-formatter/SKILL.md)
