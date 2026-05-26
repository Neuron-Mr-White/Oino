---
name: mode-sandbox
description: Use when the user asks to configure Oino /mode sandbox profiles, create or edit mode profile JSON, change allowed tools, adjust plan/work/custom mode behavior, or migrate from removed /mode:create or /mode:<profile> commands.
---

# Configure Oino Mode Sandbox

Use this skill to help users configure `builtin:mode-sandbox` profiles safely. The user-facing command is `/mode <profile>` only; do not suggest `/mode:<profile>`, `/mode:read`, or `/mode:create`.

## Workflow

1. Confirm `builtin:mode-sandbox` is installed/enabled if `/mode <profile>` is unavailable.
2. Decide scope:
   - project override: `<project>/.oino/sandbox-mode/<profile>.json`
   - global default: `~/.oino/sandbox-mode/<profile>.json`
   Ask when scope is ambiguous; prefer project scope for project-specific policies.
3. Validate the profile name: lowercase ASCII letters/digits plus `-` or `_`, max 64 chars. Use `plan`, `work`, or a custom name; avoid removed/reserved names `read` and `create`.
4. Read the existing project profile first, then global profile, before changing behavior. Project files override global files.
5. Write a small JSON object:

```json
{
  "allowed_tools": ["read", "bash"],
  "prompt": "Sandbox mode: REVIEW. Explain the expected behavior and restrictions for this profile."
}
```

6. Validate JSON after editing and summarize how to activate it with `/mode <profile>`.

## Profile semantics

- `allowed_tools` contains exact tool names or `"*"` for all normally enabled tools.
- The prompt is injected into model context each turn while that profile is active.
- Oino both hides non-allowed tools from the provider-visible list and blocks stale/disallowed tool calls before execution.
- Default `plan` allows `read` and `bash` with inspection-only instructions.
- Default `work` allows `"*"` while still following normal Oino policy.
- Custom profiles default to `read` + `bash` if no JSON file exists.

## Guardrails

- Allowing `bash` permits the bash tool call; Oino does not parse shell commands for semantic read-only safety. Remove `bash` for stricter profiles.
- Do not add destructive tools or `"*"` without explicit user intent.
- If the current mode blocks edits, ask the user to switch to `/mode work` or make the file change themselves.
- Do not store secrets in profile prompts.
