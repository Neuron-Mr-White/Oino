# Transcript rendering, inspect, and export

The transcript is Oino's working view of the current session. It is optimized for reading while an assistant is streaming, then reviewing or sharing the result afterward.

Adjacent guides: [TUI shell](auth-openrouter-tui.md), [commands](commands.md), [sessions](sessions.md), and [themes](theme-system/README.md).

## Choose a transcript style

Use `/settings chat-style` or a direct command:

```text
/settings chat-style chat
/settings chat-style agentic
/settings chat-style minimal
```

| Style | Best for |
|---|---|
| `chat` | A bubble-style conversation view. |
| `agentic` | Codex-like activity rows when you want tool/status flow to stand out. |
| `minimal` | Compact jcode-like rows for small terminals or dense reviews. |

All styles render assistant Markdown. Change detail level separately with:

```text
/settings collapse thinking truncate
/settings collapse tool collapse
```

- `full` keeps the full content visible.
- `truncate` shortens long thinking or tool sections.
- `collapse` hides thinking details and turns tool results into one-line summaries.

## What Markdown looks like

Assistant output renders common Markdown directly in the terminal:

- headings, emphasis, lists, task lists, block quotes, and rules
- links with visible URL fallbacks; Ctrl-click opens links when the terminal reports Ctrl-click mouse events
- fenced code blocks with line numbers and syntax coloring for common languages
- tables drawn as terminal grids, including alignment
- image placeholders such as `[image: alt] (path-or-url)`; Ctrl-click opens the image target when the terminal supports it

If an LLM wraps a table inside a `markdown` code fence, Oino unwraps that specific table so it displays as a table. Other fenced Markdown stays as code.

## Inspect the prompt

Run `/inspect` to see the provider request preview before the next message is sent. The inspect overlay includes:

- model and thinking level
- system prompt and loaded Oino resources
- current messages plus a `<next user message>` placeholder
- tool definitions and input schemas
- an approximate token count

Inspect controls are shown in the overlay footer: scroll with the listed movement keys, page with PgUp/PgDn, press `e` to export chat HTML, and close with `q` or Esc. In non-interactive mode, `oino --session <uuid> /inspect` prints the same preview text.

Review inspect output before sharing it. It does not include API keys from `~/.oino/auth.json`, but it can include any secrets that were typed into the chat, prompt templates, skills, project instructions, or tool schemas.

## Export chat HTML

From `/inspect`, press `e` to write a static HTML export under:

```text
<project>/.oino/exports/chat-<timestamp>.html
```

The export contains the current saved transcript messages, message roles, thinking details, and tool calls. It escapes message text for safe viewing and stores content as readable preformatted text; it is not a full Markdown-to-HTML rendering of every assistant message.

Not included in the export:

- unsent composer text
- send-panel queue and draft items
- open overlay state, search text, scroll position, or keymap settings
- API keys or model-list cache files

## Contributor notes

- [`oino-tui`](../crates/oino-tui) owns transcript state, chat styles, collapse modes, Markdown-to-terminal rendering, click targets, `/inspect` overlay state, and keymap-aware inspect controls.
- [`oino-harness`](../crates/oino-harness) builds the full prompt inspect snapshot from agent state, hooks, messages, tools, and system prompt.
- [`oino-app`](../crates/oino-app) writes chat HTML exports to the project export directory and handles Ctrl-click opening through platform/browser helpers.
- [`oino-resource`](../crates/oino-resource) owns the `<project>/.oino/exports` path.
