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
`/skills`.

## Opt-in skills (`default-enabled: false`)

A skill whose front matter has `default-enabled: false` is **off** until you turn it on. While
it is off it is not listed in the system prompt (the model never sees it) and the skill
registry neither judges nor indexes it, but `/skill:<name>` still runs it when you type it.
Turn it on — and the model sees it from the next prompt — in either way:

- `/skills` → *Skill settings…*: the skill shows `· opt-in` and **E** off; switch **E** (or
  **M**) on and save (global or project scope; the skill proxy does not need to be on);
- or set it in `settings.json`:
  `"extensionSettings": {"skills": {"states": {"<name>": {"enabled": true}}}}`.

Bundled opt-in skills: `unipi-migrate` (`unipi.core`), the full migration from a pi + unipi
install — see [migrate-from-unipi.md](migrate-from-unipi.md).
