# Oino Ask User

Oino Ask User is an optional built-in model-visible tool for structured human input. It is inspired by `rpiv-ask-user-question`, but Oino owns the tool schema, validation, and Ratatui modal.

## Install

Open `/extensions`, press `i` for project install or `I` for global install, and enter:

```text
builtin:ask-user
```

## Tool

The package enables:

```text
ask_user({ questions: [...] })
```

Each call supports 1-4 questions. Each question has 2-4 options, optional short `header`, optional option `preview`, and optional `multi_select`/`multiSelect`.

The TUI modal supports:

- single-option answers
- multi-select answers
- custom answers with `c`
- chat escape with `t`
- cancellation with `Esc`

Non-interactive or unavailable UI mode returns a structured cancelled result with `error: "no_ui"`.

## Safety

The tool is sequential so it pauses other tool execution while waiting for the user. It declares no filesystem, shell, network, secret, session-persistence, or provider-mutation permissions. Preview text is rendered as plain terminal text.
