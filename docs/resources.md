# Oino resources: prompts, skills, and project instructions

Oino resources are reusable Markdown files that you choose explicitly. They let you keep common instructions close to the project without Oino silently reading unrelated agent files.

## Where resources live

| Resource | Path | What it does |
|---|---|---|
| Global system prompt | `~/.oino/SYSTEM.md` | Personal baseline instructions loaded before project instructions. |
| Project instructions | `<project>/.oino/AGENT.md` | Repository-specific guidance such as build commands, style rules, and constraints. |
| Prompt templates | `<project>/.oino/prompts/<name>.md` | Reusable request templates you insert with `/prompt:<name>`. |
| Project skills | `<project>/.oino/skills/<name>/SKILL.md` | Project-specific workflows you insert or run with `/skill:<name>`. |
| Global skills | `~/.oino/skills/<name>/SKILL.md` | Personal workflows available across projects. |

Oino creates the default files and folders on startup if they are missing. It does not overwrite your edits.

## Browse and use resources

In the TUI:

- `/prompts` opens project prompt templates.
- `/skills` opens project and global skills.
- `/reload` rescans `SYSTEM.md`, `AGENT.md`, prompts, and skills after you edit them.
- `/prompt:<name>` includes a prompt template in your next message.
- `/skill:<name>` includes a skill in your next message.
- `/P:<query>` and `/S:<query>` search prompts or skills from anywhere in the composer.

You can repeat resource tokens in one message, for example:

```text
/prompt:review /skill:debug check the failing test
```

Use `Ctrl-O e` to expand a `/prompt:<name>` reference in the composer before sending when you want to inspect or edit the generated text.

## Write prompt templates

Prompt templates are single Markdown files under `<project>/.oino/prompts/`. The file name becomes the prompt name, so use lowercase letters, numbers, hyphens, or underscores.

Optional front matter can make the browser easier to scan:

```markdown
---
description: Review code changes
argument-hint: [focus]
---
Review the current changes for correctness, tests, and edge cases.

Focus: $ARGUMENTS
```

Template placeholders:

- `$ARGUMENTS` or `$@` inserts all text after the prompt token.
- `$1` through `$9` insert individual words from that text.

## Write skills

A skill is a folder with a `SKILL.md` file:

```text
<project>/.oino/skills/debug/SKILL.md
```

The `name` must match the folder name:

```markdown
---
name: debug
description: Investigate a failing test or bug
---

# Debug

1. Reproduce the failure.
2. Find the smallest cause.
3. Explain the fix and validation.
```

Use project skills for repository-specific workflows and global skills for personal habits you want everywhere. If a project skill and global skill share the same name, the project skill wins for that project.

## Share resources with a team

Commit project resources when they describe the repository:

```text
.oino/AGENT.md
.oino/prompts/*.md
.oino/skills/*/SKILL.md
```

Keep personal preferences in `~/.oino/` instead. Avoid secrets in committed resources; use environment variables or Oino auth files for credentials.

## Extension resources

Installed extensions can also contribute prompts, skills, and other resources. Manage extension-provided resources from the [Extension user guide](extension-kernel/user-guide.md); build them with the [Extension developer guide](extension-kernel/developer-guide.md).

## Troubleshooting

- Resource missing: run `/reload`, then check the file path and name.
- Prompt not listed: confirm it is a Markdown file directly under `<project>/.oino/prompts/` and the file name uses allowed characters.
- Skill not listed: confirm it uses `skills/<name>/SKILL.md`, has `name` and `description` front matter, and the `name` matches the folder.
- Duplicate skill name: rename one skill. Project skills take priority over global skills.
