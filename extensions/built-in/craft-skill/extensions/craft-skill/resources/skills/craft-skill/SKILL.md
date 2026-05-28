---
name: craft-skill
description: Use when the user asks to create, improve, validate, package, or install an Oino skill; also use for requests mentioning skill design, trigger descriptions, SKILL.md files, `.oino/skills`, or extension-contributed skills.
---

# Craft an Oino Skill

Use this skill to turn a repeatable workflow into a compact, reliable Oino skill. A good skill has a precise trigger, a small set of operating rules, realistic validation prompts, and Oino-native paths.

## First clarify the skill shape

Before writing files, identify:

1. **Outcome** — what the skill helps Oino do better than the default assistant.
2. **Trigger phrases** — the user language that should cause Oino to load the skill.
3. **Scope** — whether the skill is personal, project-specific, or extension-contributed.
4. **Inputs** — what information the user must provide and what can be inferred.
5. **Boundaries** — what the skill must not do, including unsafe commands, secrets, or project-specific assumptions.

Ask a short clarifying question only when these points change the design materially. Otherwise proceed with a stated assumption.

## Choose the Oino destination

Use one of these locations:

- Project skill: `<project>/.oino/skills/<skill-name>/SKILL.md`
- Global skill: `~/.oino/skills/<skill-name>/SKILL.md`
- Optional built-in extension resource: `extensions/built-in/<package>/extensions/<extension>/resources/skills/<skill-name>/SKILL.md`
- External extension resource: `<package>/extensions/<extension>/resources/skills/<skill-name>/SKILL.md`

Skill names should be lowercase and use hyphens for words. For project/global skills, the front matter `name` must match the folder name.

## Draft `SKILL.md`

Use this structure:

```markdown
---
name: example-skill
description: Use when the user asks for the specific workflow this skill handles; include concrete trigger phrases and avoid vague wording.
---

# Example Skill

One short paragraph explaining what the skill does.

## Workflow

1. Inspect the relevant context.
2. Make the smallest safe plan.
3. Execute the workflow with Oino-native tools and paths.
4. Validate the result.
5. Report concise outcomes and next steps.

## Guardrails

- Do not expose secrets.
- Ask before destructive or ambiguous actions.
- Prefer existing project conventions over generic templates.
```

Keep the body focused. A skill is not a tutorial; it is just enough instruction to improve the next run.

## Validate the skill

Create or record at least three evaluation prompts:

1. A direct trigger that should use the skill.
2. A nearby request that should not use the skill.
3. A realistic messy request with missing context.

Review the draft against those prompts:

- Would the front matter description reliably trigger only when useful?
- Does the workflow reference Oino paths such as `.oino/skills` or extension resource paths correctly?
- Are required inputs and refusal/confirmation points clear?
- Does it avoid tool permissions or behavior that Oino should enforce elsewhere?
- Is the skill short enough to load without wasting context?

If possible, run `/reload`, open `/skills`, and include it with `/skill:<skill-name>` or `/S:<query>` to smoke-test discovery.

## Improve existing skills

When revising an existing skill:

1. Preserve useful domain-specific rules.
2. Remove stale paths, duplicated instructions, and hidden assumptions.
3. Tighten the description before expanding the body.
4. Add a short changelog note in the final response; do not add comments inside the skill unless they help future users.

## Output expectations

When finished, report:

- skill name and path;
- trigger summary;
- validation prompts used or added;
- any follow-up manual checks such as `/reload` or `/skills`.
