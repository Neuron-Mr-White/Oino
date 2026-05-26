# Oino Mode Sandbox

Oino Mode Sandbox is an optional built-in package that enables named sandbox profiles in the TUI.

## Install during development

Open `/extensions`, press `i` for a project install or `I` for a global install, and enter:

```text
builtin:mode-sandbox
```

After install, use `/extensions` project/global toggles to enable or disable the package or its command contribution. When enabled, switch profiles with `/mode <profile>`.

## Commands

```text
/mode plan
/mode work
/mode <custom-profile>
```

The older colon forms (`/mode:plan`, `/mode:work`, `/mode:<profile>`), `/mode:read`, and `/mode:create` are intentionally not supported. To create or edit profiles, change the JSON files directly or include the contributed skill with `/skill:mode-sandbox` and ask the agent to configure them.

## Semantics

Mode sandbox has two layers:

1. Oino prunes the provider-visible tool list to the current profile allow-list, so asking the model what tools it has should match the active profile.
2. Oino enforces the same allow-list at the host `before_tool_call` guard, so a hidden or stale tool call is still blocked before execution.

Default profiles are created in global scope on first install/switch under:

```text
~/.oino/sandbox-mode/plan.json
~/.oino/sandbox-mode/work.json
```

Project-specific files override global files when present:

```text
<project>/.oino/sandbox-mode/<profile>.json
```

Default `plan.json`:

```json
{
  "allowed_tools": ["read", "bash"],
  "prompt": "Sandbox mode: PLAN. Treat the workspace as read-only planning context. Use read freely and bash only for inspection. Do not edit/write files or run mutating shell commands unless the user switches to work mode or changes this profile."
}
```

Default `work.json`:

```json
{
  "allowed_tools": ["*"],
  "prompt": "Sandbox mode: WORK. Normal enabled Oino tools are available; still follow project instructions and ask before risky or destructive actions."
}
```

Custom profiles default to `read` + `bash` until a JSON file is added. `allowed_tools` accepts exact tool names or `"*"`. The profile `prompt` is injected into model context for each turn in that mode.

## Agent-assisted configuration

This package contributes the `mode-sandbox` skill. Include it with:

```text
/skill:mode-sandbox
```

Then ask the agent to create or update a global or project profile. The skill guides scope selection, profile-name validation, JSON editing, and safety checks.

**Important:** allowing `bash` means Oino permits the bash tool call. The default plan prompt tells the model to use bash only for inspection, but shell-command semantic safety is by instruction plus review, not by a shell-command parser. Remove `bash` if you want tool-enforced file-read-only behavior.

## Safety

This package declares command metadata, a skill resource, and scoped filesystem access for global and project `sandbox-mode/**` profile files. It has no model-callable tools, raw shell entitlement, network access, secrets, package-management rights, provider mutation, or persistence permissions. The actual enforcement lives in Oino core so disabling the package removes the user-facing mode command.
