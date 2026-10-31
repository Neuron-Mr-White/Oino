# Migrating a previous install

Oino runs the unipi features natively (bundled `unipi.*` extensions) and reads the standard
file formats, but it never reads another agent's directories on its own. To bring an
existing install over, ask the agent to run the bundled **`unipi-migrate`** skill — it
discovers a previous stack read-only, copies settings, credentials, models, sessions,
skills, prompts and state into `~/.oino`, and never modifies the old files.

## Run it

The skill is **opt-in**: it is not listed in the system prompt until you enable it.

- One-off: type `/skill:unipi-migrate migrate everything from my previous install`.
- Or enable it (`/skills` → *Skill settings…* → `unipi-migrate` → **E** on, or
  `"extensionSettings": {"skills": {"states": {"unipi-migrate": {"enabled": true}}}}` in
  `~/.oino/settings.json`) and ask "migrate my setup into Oino".

Close other Oino sessions first. The skill walks you through the plan and confirms every
copy; see its `SKILL.md` for the full checklist. What it does not carry over is listed in
the skill's report when it finishes.
