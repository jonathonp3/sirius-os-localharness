---
name: photocraft-design
display-name: PhotoCraft Design
description: Compose and verify PhotoCraft icons using a locked structured spec, deterministic validation, and a read-only vision critic.
---

# PhotoCraft Design

Use this skill for PhotoCraft icon or design work.

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

Call:

```text
document_export
```

without `force` for a normal export. The export must pass the strict validator.

Do not use `force=true` unless the user explicitly asks to bypass validation.

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
Artifact: /absolute/path/to/export.png
```

Use the actual exported path, not a placeholder, and do not wrap the path in
backticks. This lets Local Harness detect the image and show it in the artifact
panel.

Then report:

```text
spec or command used
deterministic result
Artifact: /absolute/path/to/export.png
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
