# Commands

Type `/` at the start of the composer to see Oino commands. Suggestions update as you type.

## Everyday commands

```text
/help        show help and key hints
/model       choose a model
/thinking    choose the thinking level
/theme       choose a theme
/settings    open settings
/new         start a new session
/sessions    browse saved sessions
/title       rename the current session
/usage       show usage totals
/reload      rescan resources, extensions, themes, and cached model lists
```

## Auth and provider commands

```text
/auth              show readiness status
/account           show provider/runtime status
/9router setup     set up the recommended local router
/9router dashboard open the dashboard URL
/9router models    fetch and cache models
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
