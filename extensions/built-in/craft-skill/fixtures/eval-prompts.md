# Craft Skill Evaluation Prompts

Use these prompts to check whether a proposed Oino skill triggers at the right time.

## Direct trigger should use the skill

```text
Create an Oino project skill that helps review database migrations before release.
```

## Nearby request should not use the skill

```text
Review this existing migration for correctness and performance.
```

## Messy request should ask or assume carefully

```text
We keep repeating the same release checklist. Make it reusable for this repo and include the right Oino path.
```

A good skill description catches the first and third prompts without catching the second unless the user explicitly asks to create or revise a skill.
