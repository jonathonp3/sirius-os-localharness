---
name: long-document-formatter
display-name: Long Document Formatter
description: Format large documents or web pages in safe chunks, then assemble a complete Markdown file without losing content.
---

# Long Document Formatter

Use this skill when the source is too large to read, format, or write in one model response.

## Rules

1. Never ask the model to hold the entire document in one context window.
2. Read the source in character ranges with `read_file` using `offset` and `limit`.
3. Format one chunk at a time.
4. Append each finished chunk with `append_file`; do not rewrite the whole output for every chunk.
5. Never split inside a fenced code block, table, or indented block.
6. Keep a small overlap at chunk boundaries so headings and lists are not lost.
7. Write the document title, source URL, and TOC once, before the first content chunk.
8. Do not repeat the title or TOC for later chunks.
9. Verify the assembled file after the final chunk.

## Procedure

### 1. Measure

```text
read_file(path, offset=0, limit=200)
```

The result reports:

```text
file ...: chars 0..200 of N
```

Use `N` to plan chunks of roughly 6000–8000 characters.

### 2. Prepare the output

Write the front matter once:

- H1 title
- one-line summary
- source URL and retrieval method
- TOC

Then append a separator before the first content chunk.

### 3. Read and format chunks

For each chunk:

```text
read_file(path, offset=START, limit=7000)
```

Format the chunk according to the target skill.

If the next chunk starts inside a code block or table, move the boundary back to the start of that block.

### 4. Append

```text
append_file(path=output, content=formatted_chunk)
```

Do not append a second H1 or TOC.

### 5. Verify

- count the source chunks and output sections
- confirm the final heading and final code block are present
- search the output for `TODO`, `truncated`, and duplicate headings
- report the final byte size

## Failure modes

- If a chunk exceeds the model context, reduce it to 4000–5000 characters.
- If `append_file` is unavailable, stop and report the missing capability.
- If the source contains a Cloudflare challenge, use the `cloudflare-fetch` skill first.
- If the document cannot be reconstructed faithfully, do not claim completion.
