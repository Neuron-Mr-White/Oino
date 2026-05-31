---
name: ralph-loop
description: Use when a user asks Oino to run a long-running iterative development loop, continue a loop, process a few checklist items per turn, reflect on cadence, or coordinate progress with promise tags.
---

# Oino Ralph Loop Operator

Use this skill to run Oino-native Ralph loops. A Ralph loop keeps a stable task file, performs a bounded amount of work per iteration, records progress, and emits a machine-readable promise tag so Oino can decide whether to continue, pause, ask the user, or finish.

## Operating rules

1. Read the loop task file from `.oino/ralph/<loop-name>/task.md` before changing code.
2. Read `.oino/ralph/<loop-name>/steering.md` for urgent human steering before choosing work.
3. Work on only the requested number of checklist items for the current iteration.
4. Update the task file with concise progress notes and any changed checklist boxes.
5. Preserve Oino conventions: project state stays under `.oino/`, user-facing skills stay under `.oino/skills/`, and optional built-in packages stay under `extensions/built-in/`.
6. Do not assume Docker, outside tool paths, or external `ralph-loop` packages. Oino owns the loop state and controller behavior.
7. If blocked by a decision, stop and emit a `DECIDE` promise rather than guessing.

## Promise tags

End each iteration with exactly one of these tags:

- `<promise>CONTINUE</promise>` — bounded progress was made and more work remains.
- `<promise>COMPLETE</promise>` — all work for the loop is done.
- `<promise>BLOCKED:reason</promise>` — progress cannot continue without an external fix.
- `<promise>DECIDE:question</promise>` — the user must choose a direction.
- `<promise>TASK-ID:DONE</promise>` — a specific task was completed and the loop may continue.

If more work remains but no specific task ID applies, use `<promise>CONTINUE</promise>`. Oino treats a missing promise as blocked to avoid unattended ambiguous loops.

## Reflection cadence

When the loop state says the next iteration is a reflection point, briefly verify:

- the implementation still matches the original task;
- checklist priorities are still ordered correctly;
- tests or docs are keeping up with implementation;
- no external dependency was introduced accidentally.

Keep reflection short and actionable.
