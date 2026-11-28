---
name: automation-health
display-name: Automation Health
description: Use for scheduled or manual system-health automation. Load journal-check for logs, use plugin__log_check, and report HEALTHY, DEGRADED, or FAILED with evidence.
---

# Automation Health

Use this skill for an automation task that checks system or service health.

## Required sources

- Load `journal-check` before inspecting unit logs.
- Use `plugin__log_check` for systemd unit logs and summaries.
- Do not run bare `journalctl` against a unit.
- Use `host_bash` only for bounded state commands.
- Never infer health from old chat history.

## Checks

Inspect, where available:

```text
systemctl --user is-active llama-server local-harness-flatpak
podman ps
distrobox list
rpm-ostree status
df -h /var/home
```

For service logs use:

```text
plugin__log_check unit=<name>
```

For package metadata, run a bounded check such as:

```text
dnf makecache
```

Treat checksum mismatches, repository metadata failures, failed units, repeated
restarts, unavailable model endpoints, and full disks as health evidence.

Do not run package installs or destructive commands from an automation task.

## Verdict rules

### FAILED

Use when a critical service is down, the model endpoint is unavailable, the
filesystem is inaccessible, or package management cannot function.

### DEGRADED

Use when the system runs but any evidence is abnormal:

- package metadata checksum mismatch
- repository or mirror failure
- failed or restarting unit
- unavailable optional service
- low disk space
- unexpected container state
- journal warnings or application errors

A checksum mismatch is **DEGRADED**, never normal.

### HEALTHY

Use only when all required checks pass and there are no warnings, checksum
failures, failed units, or unexplained errors.

Do not say "functioning normally", "likely temporary", or "probably a mirror
issue" without evidence. State what failed and what was verified.

## Report

Start with the verdict:

```text
HEALTH: DEGRADED

Evidence:
- ...
- ...

Checks:
- llama-server: active
- local-harness-flatpak: active
- package metadata: checksum mismatch

Next:
- ...
```

Keep the report short. Do not dump raw journal history.

## Automation prompt

Recommended task prompt:

```text
Load the automation-health skill.
Load journal-check before inspecting logs.
Use plugin__log_check for unit logs.
Check service state, containers, disk, and package metadata.
Report HEALTHY, DEGRADED, or FAILED with concise evidence.
A checksum mismatch is DEGRADED, never normal.
```
