# Oino Craft Skill

`builtin:craft-skill` installs an Oino-native skill for designing, writing, and validating skills.

## Install

Open `/extensions`, choose **Install package**, and enter:

```text
builtin:craft-skill
```

Then enable package `oino.craft_skill` if it is not already enabled.

## Use

Reload resources if needed, then include the skill in a request:

```text
/skill:craft-skill Create a project skill for release-note generation.
```

Or search for it from the composer:

```text
/S:craft skill
```

The skill guides Oino to choose the correct destination (`.oino/skills/<name>/SKILL.md` for project skills, `~/.oino/skills/<name>/SKILL.md` for global skills, or extension resource paths for packaged skills), write concise front matter, add practical guardrails, and record evaluation prompts.

## Validation fixture

This package includes:

- `fixtures/valid-skill/SKILL.md` — a minimal valid Oino skill shape.
- `fixtures/eval-prompts.md` — example prompts for testing whether a skill should trigger.

The repository tests also check that the built-in craft skill uses Oino terminology and does not reintroduce source-project-specific wording.

## Safety

The package contributes only a skill resource. It declares no commands, tools, shell access, filesystem access, network access, secrets, provider mutation, or package-management permission. If the skill asks Oino to write a file, the normal Oino tool and mode permission model still applies.
