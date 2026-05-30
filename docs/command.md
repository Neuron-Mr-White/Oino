# Commands

Type `/` at the start of the composer to see Oino commands. Suggestions update as you type.

## Everyday commands

```text
/help        show help and key hints
/model       choose a model; also configure model-backed features
/thinking    choose the thinking level
/theme       choose a theme
/settings    open settings
/new         start a new session
/sessions    browse saved sessions
/title       rename the current session
/usage       show usage totals
/btw         open the side plan chat panel
/btw new     open a fresh side plan chat panel
/compact     compact session with configured method (VCC or LLM)
/compact vcc     compact with deterministic VCC
/compact llm    compact with LLM summarization
/compact threshold [pct]   set/show auto-compact threshold %
/compact auto <on|off>     enable/disable auto-compaction
/compact model [inherit|<provider:model>]   set/show LLM compact model
/compact prompt [path]     set/show LLM compact prompt
/reload      rescan resources, extensions, themes, and cached model lists
```

## Model-first configuration

Use `/model` for any model-backed selection so the same cached catalog and search behavior is reused:

```text
/model <provider:model>                    set the main chat model
/model btw inherit|<provider:model>        configure the BTW side panel model

Inside the BTW panel, submit `/new` with no other text to reset that panel.
/model notify-summary off|<provider:model> configure notification summarization
```

## Auth and provider commands

```text
/auth              show readiness status
/account           show provider/runtime status
/router setup     set up the recommended local router
/router dashboard open the dashboard URL
/router models    fetch and cache models
```

## Prompts and skills

Use prompt and skill tokens anywhere in your message:

```text
/prompt:release-notes summarize these changes
/skill:debug investigate this failure
/P:<query> search prompt templates
/S:<query> search skills
```

Browse resources with:

```text
/prompts
/skills
```

## Non-interactive examples

```bash
oino "explain this repository"
oino --session <uuid> "continue the previous task"
oino /reload
```
