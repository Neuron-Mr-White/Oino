# oino.presets

Named work presets switched via `/preset [name]` (no arg lists presets and
shows the current one). Selection persists per project.

- `minimal` — model-visible tools restricted to `read`, `edit`, `write`,
  `bash`; adds the `oino/preset` section: "Be concise; prefer the fewest tool
  calls that solve the task."
- `standard` — removes the `oino/preset` section; all tools visible.
- `thorough` — adds a planning/verification checklist section:
  "Plan before acting: outline the approach, then verify each change before
  finishing. Prefer explicit checks over assumptions." All tools visible.

Headless: `--preset <name>` selects the preset for the run.
