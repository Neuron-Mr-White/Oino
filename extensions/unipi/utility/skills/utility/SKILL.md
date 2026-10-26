---
name: utility
scope: agent
description: |
  What the unipi.utility extension does for the user: the /unipi:settings
  hub, /unipi:continue (/unipi:retry), /unipi:cleanup, /unipi:doctor,
  /unipi:answer, /unipi:move, /unipi:summarize, pasted-file attachments and
  automatic session naming. Its only agent tools are the image tools.
---

# unipi.utility

## Commands (user-facing)

| Command | Purpose |
|---------|---------|
| `/unipi:settings [search]` | Configure every UniPi module (opens the settings hub, optionally filtered) |
| `/unipi:continue` / `/unipi:retry` | Take another turn without new text |
| `/unipi:answer [reply\|questions\|web]` | Answer the questions in your last reply without scrolling |
| `/unipi:cleanup` | Remove stale UniPi temp files (previews and asks first; `--dry-run`, `--yes`) |
| `/unipi:doctor` | Check folders, config, the model registry, Decision Model key, skill exposure |
| `/unipi:move [<old-path>] [--dry-run]` | Re-link sessions, trust, boards and memories after the project folder moved |
| `/unipi:summarize [focus]` | Summarize the last reply again (runs the `summarize` skill); any text is passed to the skill as the user's question |

## Notes for the agent

- There is no tool to rename the session. Naming happens automatically after
  real requests; the user can also use `/name`.
- Pasted file paths become `[Image #N]` / `[File #N]` tokens; images arrive as
  image content, files as `[File #N: <path>]`.
- Skills not listed in your catalog may still be installed. If the user names
  a skill, read its SKILL.md or ask them to run `/skill:<name>`.
- When something UniPi-related looks broken, suggest `/unipi:doctor`.
