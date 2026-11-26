---
name: journal-check
display-name: Journal Check
description: Inspect systemd unit state, recent logs, and application errors safely.
---

# Journal Check

Use this procedure for any request to inspect logs, errors, service state, or journal output.

1. Call `log_check` with the unit and a recent window (default `24 hours ago`).
2. Read `VERDICT` first.
3. Read `SERVICE`: `ActiveState`, `NRestarts`, `ActiveEnterTimestamp`.
4. Read `APPLICATION ERROR MARKERS` and `WARNINGS/ERRORS`.
5. Report every `error`, `failed`, `exceeded`, or `panic` marker, even when the unit is active.
6. If the verdict is `DEGRADED` or `FAILED`, say "running with warnings" or "FAILED"; never say functioning, normal, or healthy.
7. Use the `REPORT` block from `log_check` verbatim as the final status answer.
8. Only after steps 1-7, summarize; never summarize the first lines of `journalctl`.
9. Do not run a bare `journalctl -u <unit>`; if raw output is required, add `--since` and `-n`.
10. Do not recommend restarting for request/context-size errors; reduce input or start a fresh chat.
