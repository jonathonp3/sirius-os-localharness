# PhotoCraft

PhotoCraft is an MCP server for structured icon and design work.

When the user asks to create an image, make an icon, or design a logo, route the
request through PhotoCraft. DeepSeek can plan and call tools, but it cannot
generate the image itself.

## Why MCP

PhotoCraft is stateful and multi-tool. It owns:

- the active document
- layers and history
- shape and text operations
- deterministic checks
- export validation
- the local Qwen3-VL critic

The Local Harness MCP bundle installs its configuration and design skill, but the PhotoCraft MCP binary must already exist.

## Prerequisites

- PhotoCraft automation server or MCP binary
- `photocraft-design-mcp` on the host
- a llama.cpp build with vision support for `vision_critique`
- a Qwen3-VL model configured for PhotoCraft

## Install the bundle

```bash
harness plugins install-catalog photocraft
```

This merges:

```json
{
  "name": "photocraft",
  "command": "/usr/bin/flatpak-spawn",
  "args": ["--host", "%h/.local/bin/photocraft-design-mcp"]
}
```

into `~/.harness/mcp.json` and installs `photocraft-design`.

Restart Local Harness after installing or changing the server.

## Tools

The MCP exposes:

```text
document_inspect
document_check
document_export
vision_critique
run_spec
icon_compose
shape_add
text_add
document_open
layer_delete
ambient_occlusion
```

Exact tool availability depends on the installed PhotoCraft version.

## Role separation

```text
Qwen3-Coder
  → choose tools and parameters

PhotoCraft
  → execute structured operations

document_check
  → deterministic geometry, contrast, empty-layer checks

document_export
  → strict final validation gate

Qwen3-VL
  → post-export observations only
```

Do not let the vision model issue edits or certify correctness.

## Locked spec

For new designs, use:

```text
run_spec
```

with a `sirius/icon_compose` JSON document, or `icon_compose` for one complete composition.

The model may edit text and parameters. It must not invent geometry from an image.

For existing designs:

```text
document_inspect
```

then make the smallest `shape_add` or `text_add` change.

## Validation

After each refinement:

```text
document_check
```

Then export without `force`:

```text
document_export
```

`force=true` bypasses the strict validator. Do not use it unless the user explicitly asks.

## Vision critique

Run after export:

```text
vision_critique
```

It returns observations about:

- clipping
- color mismatch
- duplicates
- alignment
- OCR/readability

If the critic disagrees with a deterministic check, report the disagreement. Do not override the validator without approval.

## Multimodal staging

An 8B vision model and a 30B coder cannot both stay resident in 8 GB VRAM.

Run them sequentially:

```text
vision pass → release
coder/tool pass → release
deterministic validation → CPU
```

## Export format

The editable master is Photoshop `.psd` by default, preserving layers, text, and
editability. A `.png` preview is exported for the chat/artifact panel; browsers
cannot display PSD directly.

Report both:

```text
Layered file: /absolute/path/design.psd
Artifact: /absolute/path/design.png
```

## Artifact handoff

After a successful export, the skill reports:

```text
Artifact: /absolute/path/to/export.png
```

Local Harness scans tool results for workspace artifact paths and shows the
result in the artifact panel, not inline in the assistant prose. Do not wrap the
path in backticks; use one absolute path per line.

## Skill

The workflow is installed at:

```text
~/.config/local-harness/skills/photocraft-design/SKILL.md
```

Repository copy:

[skills/photocraft-design/SKILL.md](../skills/photocraft-design/SKILL.md)

## Related

- [Plugins](plugins.md)
- [Skills](skills.md)
- [Models](models.md)
