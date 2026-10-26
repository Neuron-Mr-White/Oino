# Skills

Skills are folders with a `SKILL.md` (front matter `name`, `description`) and any supporting
files, exactly as in pi 1.4.0. Oino lists them in the system prompt; the model reads a skill's
`SKILL.md` with the `read` tool when it is relevant. With `enableSkillCommands` they are also
available as `/skill:<name>`.

Discovery (later entries win on name conflicts, pi precedence):

| Location | Scope |
|---|---|
| bundled extension skills (`~/.oino/bundled/<id>@…/skills/`) | extensions (`unipi.ask-user`, `unipi.dream` craft-skill, `unipi.kanboard`, …) |
| `~/.oino/skills/` | global |
| `~/.agents/skills/` and project `.agents/skills/` | shared agent skills (pi layout) |
| `<project>/.oino/skills/` | project (trusted projects only) |
| `--skill <path>` | this run |

`-ns/--no-skills` disables discovery (explicit `--skill` paths still load).

The bundled `unipi.skill-registry` adds per-skill enable / must-show switches, a skill vault
and judged exposure (only skills relevant to the prompt are shown in full); configure it with
`/unipi:skills`.
