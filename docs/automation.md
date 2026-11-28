# Automation tasks

Automation tasks run saved prompts on a daily or weekly schedule.

Open the **Tasks** button in the header.

## Panel

```text
Automation tasks          + New
Enabled  Paused  Ended  All
Search automation tasks

No active automation tasks

Automation Tasks Review
Daily   0
Weekly  0
```

The panel is hidden until opened.

## Task types

- **Prompt task** — runs a saved prompt through the model.
- **System health** — runs `local-harness-health` directly, with no model or
  tool-approval dependency, and reports `HEALTH: HEALTHY | DEGRADED | FAILED`
  followed by the snapshot.

Choose **System health** in the New form for reliable scheduled health reports.

## Create a task

**New** opens a form with:

- title
- prompt
- daily or weekly frequency
- time in local time
- weekday for weekly tasks

Creation returns the next scheduled run.

## Actions

Each task card supports:

- Pause
- Resume
- End
- Run now
- View result
- Delete

`Run now` executes immediately in a new session without waiting for the schedule.
Once a task has run, **View result** opens that session in the chat. The task card
also shows a short result preview.

## Scheduling

- Daily tasks run at the selected local time.
- Weekly tasks run on the selected weekday and local time.
- New tasks compute the next future run.
- Pausing clears the next run.
- Resuming recomputes it.
- Ending stops scheduling but keeps the task in the Ended tab.

## Execution

When a task is due, the scheduler:

1. recalculates its next run
2. acquires a slot
3. creates a session
4. runs either the saved prompt or the health snapshot
5. stores a short result preview on the task

Prompt tasks respect the configured policy and approval settings. In the
service, human approvals are unavailable, so mutating actions that require
approval are denied unless the service was started with auto-approval.

System health tasks execute the fixed snapshot command directly and do not use
model tool selection or approvals.

## Storage

```text
<data-dir>/automations.json
```

Tasks are persisted as JSON and survive restarts.

## API

```text
GET    /api/automations
POST   /api/automations
PATCH  /api/automations/{id}
DELETE /api/automations/{id}
```

Actions:

```json
{"action":"pause"}
{"action":"resume"}
{"action":"end"}
{"action":"run"}
```

## Health-check tasks

For system-status automation, use the `automation-health` skill.

Recommended prompt:

```text
Load the automation-health skill.
Load journal-check before inspecting logs.
Use plugin__log_check for unit logs.
Check service state, containers, disk, and package metadata.
Report HEALTHY, DEGRADED, or FAILED with concise evidence.
A checksum mismatch is DEGRADED, never normal.
```

The skill requires journal evidence and forbids describing the system as normal
while package metadata checksum errors are present.

## Limitations

- schedules are local-time only
- no one-off date tasks yet
- no task editing form yet; delete and recreate
- result preview is capped
