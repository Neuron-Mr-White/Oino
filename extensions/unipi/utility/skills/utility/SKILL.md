---
name: utility
scope: agent
description: |
  What the unipi.utility extension does for the user: settings
  pages in the /settings hub, /continue (/retry), /cleanup, /doctor,
  /answer, /move, /summarize, pasted-file attachments and
  automatic session naming. Its only agent tools are the image tools.
---

# unipi.utility

## Commands (user-facing)

| Command | Purpose |
|---------|---------|
| `/settings` | Core settings hub; every feature's options live there (this extension contributes the Utility and Image pages) |
| `/continue` / `/retry` | Take another turn without new text |
| `/answer [reply\|questions\|web]` | Answer the questions in your last reply without scrolling |
| `/cleanup` | Remove stale Oino temp files (previews and asks first; `--dry-run`, `--yes`) |
| `/doctor` | Check folders, config, the model registry, Decision Model key, skill exposure |
| `/move [<old-path>] [--dry-run]` | Re-link sessions, trust, boards and memories after the project folder moved |
| `/summarize [focus]` | Summarize the last reply again (runs the `summarize` skill); any text is passed to the skill as the user's question |

## Notes for the agent

- There is no tool to rename the session. Naming happens automatically after
  real requests; the user can also use `/name`.
- Pasted file paths become `[Image #N]` / `[File #N]` tokens; images arrive as
  image content, files as `[File #N: <path>]`.
- Skills not listed in your catalog may still be installed. If the user names
  a skill, read its SKILL.md or ask them to run `/skill:<name>`.
- When an Oino feature looks broken, suggest `/doctor`.
