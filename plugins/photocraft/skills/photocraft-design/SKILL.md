---
name: photocraft-design
display-name: PhotoCraft Design
description: Use for image creation, icon design, composition, editing, or visual verification. Trigger when the user says "create image", "edit image", "make an icon", "design a logo", or asks for PhotoCraft output. Reuse saved JSON specs when modifying an existing image. Compose and verify with a locked structured spec, deterministic validation, and a read-only vision critic.
---

# PhotoCraft Design

Use this skill for PhotoCraft icon or design work.

## Editing an existing image

When the user asks to modify an image, do not edit the flat PNG by hand and do
not invent a new design. Find the structured source first.

### 1. Locate the source

Given an image path such as `/path/design.png`, search for:

```text
/path/design.spec.json
/path/design.json
/path/design.pcraft.json
```

Then search the workspace for JSON specs whose `export_path` references the
image:

```text
grep -rl "design.png" <workspace> --include='*.json'
```

Use `glob`/`grep` and `read_file`. Prefer the original spec over a later copy.

### 2. Inspect before editing

- `read_file` the spec.
- `document_inspect` if a PSD/TIFF master is involved.
- Use the exported PNG or `vision_critique` only to understand the current
  appearance, never to authorize changes.

### 3. Edit minimally

Copy the spec to a new version:

```text
design.v2.spec.json
```

Change only the requested text, color, position, shape order, or parameter.
Do not rebuild geometry or change unrelated elements.

### 4. Rebuild and export

Run the edited spec with an absolute path:

```text
run_spec path=/absolute/path/design.v2.spec.json
```

Export a new lossless PNG:

```text
/absolute/path/design.v2.png
```

Keep the original PNG and spec intact.

### 5. Report

```text
Source spec: /absolute/path/design.spec.json
Edited spec: /absolute/path/design.v2.spec.json
Artifact: /absolute/path/design.v2.png
```

If no JSON spec exists, say so. Do not claim the image was edited from a PNG.
Ask for the spec, or offer to reconstruct one explicitly with the user's
approval.

## Default output

Unless the user asks for a layered master, produce:

```text
design.spec.json   editable construction source
design.png         lossless flat image for display
```

The PNG is flat; editability comes from the spec, not the PNG. PSD or layered
TIFF is exported only when explicitly requested.

## Unsupported text effects

The PhotoCraft text schema supports a fill color, not a text stroke or outline.
Do not claim that a requested text outline was rendered. Approximate only when
the user agrees, for example with duplicated offset text layers; otherwise report
the limitation.

## Image creation routing

When the user asks to "create an image", "make an icon", "design a logo", or
similar, use the PhotoCraft MCP tools. Do not claim an image was created from
text alone.

Required path:

```text
write_file path=design.spec.json (the locked spec)
→ icon_compose or run_spec
→ document_check
→ document_export (.png, lossless)
→ Artifact: /absolute/path/to/design.png
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

For `run_spec`, the spec runner validates and builds the document; do not call
`document_check` against a fresh/blank document afterwards. For `icon_compose`
or an active document being refined, call:

```text
document_check
```

Treat its score and warnings as hard evidence.

Spec constraints that prevent common failures:

- gradient stop positions must be numbers, not strings:
  `"stops": [[0, "#003366"], [0.3, "#66ccff"]]`
- include at least one actual shape layer; a gradient background alone may not
  satisfy the validator
- text `x`/`y` are top-left unless `width`, `height`, and `align: "center"` make
  the box center at `x`
- use absolute paths for `run_spec` and `document_export`
- a `run_spec` export may close the active document; rely on its returned result,
  not a later `document_check` on an empty canvas

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

PhotoCraft has no implicit format; the path selects it.

Default export:

```text
document_export path=/absolute/path/design.png
```

PNG is lossless and is always produced for viewing. Keep the structured spec as
the editable source of truth:

```text
/absolute/path/design.spec.json
```

If the user explicitly asks for a layer-preserving master, additionally export:

```text
document_export path=/absolute/path/design.psd
```

or a layered TIFF when requested. Keep the PNG preview even when exporting PSD
or TIFF. Do not use `force=true` unless the user explicitly asks to bypass
validation.

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
Spec: /absolute/path/design.spec.json
Artifact: /absolute/path/design.png
Layered file: /absolute/path/design.psd (optional)
```

Use actual exported paths, not placeholders, and do not wrap the paths in
backticks. The `Artifact:` line points at the PNG preview so Local Harness can
display it; the `Layered file:` line identifies the editable PSD master.

Then report:

```text
spec or command used
deterministic result
Spec: /absolute/path/design.spec.json
Artifact: /absolute/path/design.png
Layered file: /absolute/path/design.psd (optional)
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
