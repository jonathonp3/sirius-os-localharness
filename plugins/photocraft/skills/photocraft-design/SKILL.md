---
name: photocraft-design
display-name: PhotoCraft Design
description: Use for image creation, icon design, composition, or visual verification. Trigger when the user says "create image", "make an icon", "design a logo", or asks for PhotoCraft output. Compose and verify with a locked structured spec, deterministic validation, and a read-only vision critic.
---

# PhotoCraft Design

Use this skill for PhotoCraft icon or design work.

## Image creation routing

When the user asks to "create an image", "make an icon", "design a logo", or
similar, use the PhotoCraft MCP tools. Do not claim an image was created from
text alone.

Required path:

```text
icon_compose or run_spec
→ document_check
→ document_export (.psd, layered)
→ document_export (.png, preview)
→ Artifact: /absolute/path/to/export.png
```

If PhotoCraft is unavailable, report that image creation is unavailable. Do not
substitute a text description for the requested image.

## Roles

- **Qwen3-Coder** selects PhotoCraft tools and fills parameters from known facts.
- **Qwen3-VL** only observes and critiques an export. It does not choose tools or issue edits.
- **PhotoCraft validators** are authoritative for geometry, contrast, duplicates, bounds, text clipping, and export eligibility.
- **The structured spec** is the source of truth. Do not invent geometry from a prose description when a spec exists.

## Workflow

### 1. Orient

For an existing design:

```text
document_inspect
```

Use the returned layer ids and names. Do not assume them.

### 2. Build a locked spec

For a new icon or a full composition, use `run_spec` with a `sirius/icon_compose` JSON document, or `icon_compose` for a single complete composition.

The spec must contain:

- canvas width and height
- background
- ordered shape elements
- text content, font, size, position, and color
- no unspecified geometry

Do not ask a vision model to generate shapes.

### 3. Execute minimally

Use:

- `run_spec` for a JSON design spec
- `icon_compose` for a complete new icon
- `shape_add` / `text_add` only for a specific refinement

Do not rebuild the whole design to change one label or parameter.

### 4. Deterministic check

Call:

```text
document_check
```

Treat its score and warnings as hard evidence.

If it reports:

- low contrast
- empty layers
- out-of-bounds geometry
- duplicate geometry
- text overflow

fix only the named element, then check again.

### 5. Export gate

Call `document_export` without `force` for a normal export. The export must pass
the strict validator.

Save the editable master as Photoshop format by default:

```text
document_export path=/absolute/path/design.psd
```

Then export a PNG preview for the Local Harness artifact panel:

```text
document_export path=/absolute/path/design.png
```

Do not use the PNG as the master. The PSD is the editable source with layers,
text, and history preserved. Do not use `force=true` unless the user explicitly
asks to bypass validation.

### 6. Visual critique

Only after a successful export, call:

```text
vision_critique
```

Use it for observations such as:

- text cut off
- color mismatch
- duplicated visual element
- alignment problem
- OCR/readability issue

Do not let the critic authorize an edit by itself. Any resulting change must be a minimal text or parameter edit, followed by `document_check` and export again.

### 7. Report

After a successful export, include one machine-readable handoff line:

```text
Layered file: /absolute/path/design.psd
Artifact: /absolute/path/design.png
```

Use actual exported paths, not placeholders, and do not wrap the paths in
backticks. The `Artifact:` line points at the PNG preview so Local Harness can
display it; the `Layered file:` line identifies the editable PSD master.

Then report:

```text
spec or command used
deterministic result
Layered file: /absolute/path/design.psd
Artifact: /absolute/path/design.png
critic observations, clearly labelled as observations
```

Never say the design is correct only because a model said so.

## Hard rules

- No geometry redraw from a vision model.
- No `force` export without explicit permission.
- No silent redesign while changing text.
- No claims of success when `document_check` or export validation failed.
- No model self-certification.
- Keep the structured spec editable.
- If the 8B vision model and 30B coder cannot both be resident, run them sequentially; do not pretend both are active.

## Failure handling

If `document_check` fails:

1. identify the failing property
2. make the smallest parameter or text change
3. re-run `document_check`
4. only then export

If `vision_critique` disagrees with the deterministic result, report the disagreement; do not override the validator without user approval.

If export is blocked and the cause is unclear, stop and report the validator output.
