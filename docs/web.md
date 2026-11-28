# Web access

Local Harness exposes read-only web search and fetch tools when started with
`--allow-web`.

## Downloads

`web_download` is available only with a remote API provider, such as DeepSeek.
It is not exposed for the local llama.cpp provider.

Downloads always require approval:

```text
web download: https://example.com/file.zip
```

Approved downloads are written inside the workspace, with a 50 MB limit and a
workspace-relative path. Absolute paths, `..`, and paths outside the workspace
are rejected.

Local llama remains read-only:

```text
web_search      allowed (approval)
web_fetch       allowed (approval)
web_download    unavailable
```

Use a workspace-relative destination:

```text
web_download url=https://example.com/file.zip path=downloads/file.zip
```
