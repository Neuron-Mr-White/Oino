# Migrating from pi + unipi

Oino runs the unipi features natively (bundled `unipi.*` extensions) and reads pi's file
formats, but it never reads `~/.pi` or `~/.unipi` on its own: there is no legacy unipi
support in the code and the old `/unipi:*` command names do not exist
([commands](dev/v1/commands.md)). To bring an existing pi + `@pi-unipi/unipi` install over,
ask the agent to run the bundled **`unipi-migrate`** skill.

## Run it

The skill is **opt-in**: it is not listed in the system prompt until you enable it.

- One-off: type `/skill:unipi-migrate migrate everything from my pi/unipi install`.
- Or enable it (`/skills` → *Skill settings…* → `unipi-migrate` → **E** on, or
  `"extensionSettings": {"skills": {"states": {"unipi-migrate": {"enabled": true}}}}` in
  `~/.oino/settings.json`) and ask "migrate my unipi setup into Oino".

Close other Oino sessions first. The agent then:

1. **Discovers** (read-only) `~/.pi/agent` (settings, auth, models, sessions, skills,
   prompts, themes, keybindings, extensions, npm packages, MCP), `~/.unipi` (module configs,
   memory, kanboard, skill vault, workspaces, hints, images), your projects' `.pi/` and
   `.unipi/` dirs (found through session dirs, `trust.json` and `.unipi-workspace.json`
   markers) and MemPalace.
2. **Shows a dry-run inventory**: every item with source, target, action, conflicts and what
   will *not* be migrated and why. Secrets are shown by name only.
3. **Asks for your confirmation** (you can exclude items).
4. **Backs up** `~/.oino` to `~/.oino.pre-unipi-migrate-<time>` (and each project's `.oino/`).
5. **Migrates**: pi settings → `~/.oino/settings.json`; `auth.json` / `models.json` (providers
   Oino implements); sessions (pi v3 is Oino's format) → `~/.oino/sessions`; skills, prompts,
   themes, keybindings, `AGENTS.md`/`SYSTEM.md`, trust; every unipi module's settings →
   `extensionSettings.<namespace>` (legacy keys rewritten, secrets into
   `~/.oino/secrets.json`); memory files, skill vault and states, subagent profiles, kanboard
   boards, ralph task files and paused goals, Fusion presets, MCP servers, hints, the
   input-shortcut stash and generated images → `~/.oino/state/unipi.*`; project files →
   `<project>/.oino/`; old `/unipi:*` names in your prompts, skills, profiles, ralph files and
   `AGENTS.md`/`CLAUDE.md` → the new names. Your pi/unipi files are never changed.
6. **Verifies** (JSON, permissions, Oino start, settings read-back, models, session and
   memory counts, kanboard validation, no `/unipi:` left) and prints the rollback command.

## Not migrated

TypeScript pi packages and extensions (`packages`, `extensions`, `~/.pi/agent/npm`), OAuth
logins of providers Oino has no flow for, `models.json` providers with an API Oino does not
implement, cron, herdr, the UniPi desktop/phone app (app host, bridge, pairing, app push
endpoints), tips, caches/logs/indexes, and run state of old processes (subagent indexes,
long-horizon leases and history, dream runs). The inventory names each one it finds.

## Roll back

Restore the backup (`mv ~/.oino ~/.oino.migrated && cp -a ~/.oino.pre-unipi-migrate-<time>
~/.oino`), restore each project's `.oino` archive, and `git checkout` any project file the
rename changed. pi + unipi keep working, since their files were not touched.
