# Oino Ralph Loop

Oino Ralph Loop is an optional built-in package for long-running iterative work. It is inspired by the Ralph-loop control pattern, but Oino owns the implementation and stores state in Oino project paths rather than depending on or vendoring an external package.

## Install during development

Open `/extensions`, press `i` for a project install or `I` for a global install, and enter:

```text
builtin:ralph-loop
```

After install, use `/extensions` project/global toggles to enable or disable the package or its command contribution. When enabled, `/ralph` appears in slash command suggestions and `/help` lists the Ralph controls.

## Commands

```text
/ralph help
/ralph start <name> <task>
/ralph list
/ralph status [name]
/ralph resume <name>
/ralph continue [name]
/ralph once [name]
/ralph steer <name> <urgent instruction>
/ralph pause <name>
/ralph cancel <name>
/ralph archive <name>
/ralph clean
/ralph record <name> <continue|complete|blocked|decide|done> [note-or-task-id]
```

`/ralph start` creates the task/state/log/steering/history files and immediately starts the first iteration in the TUI. When the assistant ends with `CONTINUE` or `TASK-ID:DONE`, Oino records the output and automatically queues the next iteration until the loop completes, blocks, asks for a decision, or reaches the max iteration count. `/ralph once` runs exactly one iteration without auto-continuing, and `/ralph steer` appends urgent instructions to the steering file that is included in every future iteration prompt.

`record` remains the low-level/manual command used to persist an iteration promise; for `done`, pass the task id first and any note after it, e.g. `/ralph record docs done TASK-1 updated README`.

## State model

Ralph loop state is project-scoped and lives under:

```text
<project>/.oino/ralph/
  <loop-name>.md       # task/checklist/progress document
  <loop-name>.json     # typed state machine snapshot
  <loop-name>.log.md   # append-only iteration notes
  <loop-name>.steering.md # live human steering included each iteration
  history/<loop-name>/ # assistant output captured per iteration
  archive/             # archived loop snapshots
```

The typed state tracks:

- normalized loop name
- status: `active`, `paused`, `blocked`, `awaiting_decision`, `complete`, `cancelled`, or `archived`
- current and maximum iteration counts
- items-per-iteration and reflection cadence
- task/log/steering/history paths
- creation/update/archive timestamps
- last promise tag and per-iteration progress notes

## Promise tags

At the end of each iteration the agent should emit one promise tag:

```text
<promise>CONTINUE</promise>
<promise>COMPLETE</promise>
<promise>BLOCKED:reason</promise>
<promise>DECIDE:question</promise>
<promise>TASK-ID:DONE</promise>
```

Oino parses these tags to update loop state. `CONTINUE` and `TASK-ID:DONE` record progress and leave the loop active; `COMPLETE`, `BLOCKED`, and `DECIDE` transition the loop and stop auto-continuation. A missing promise tag is treated as blocked so the loop does not run unattended with ambiguous state.

## Runtime safety

This package declares no shell, network, tool, secret, provider-mutation, or package-management permissions. The only intended writes are Oino-owned project loop files under `.oino/ralph/`.
