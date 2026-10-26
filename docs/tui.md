# Terminal UI

`oino` with a terminal on stdin and stdout starts the interactive UI (`crates/oino-tui`, a
ratatui client over the in-process session). It follows pi 1.4.0's interactive mode.

```bash
oino                                   # new session
oino "Explain src/main.rs"             # with a first message (more messages are queued)
oino @screenshot.png "What is wrong?"  # attach files / images
oino -c                                # continue the last session here
```

## Screen

- **Header** — product, version, cwd, hints from extensions (`unipi.core` hints).
- **Transcript** — markdown with syntax highlighting, collapsible thinking blocks and tool
  output (`app.tools.expand`), pi's layouts for `read`/`write`/`edit`/`bash`, extension
  renderers and cards.
- **Widgets / panels** — extension widgets above the editor, overlays, editor-replacing
  panels (e.g. `/unipi:btw`) and fullscreen panels (e.g. `/unipi:info`).
- **Editor** — multi-line input with history, kill ring, undo, paste markers, `@` path
  completion, `/` command completion.
- **Footer** — tokens ↑↓, cache R/W, cost, context %, model, thinking level, routed model and
  extension status segments (`unipi.footer`).

## Input

- `enter` sends; while the agent works, `enter` steers (delivered after the current tool
  call) and `alt+enter` queues a follow-up.
- `escape` interrupts; `escape escape` opens the tree or forks (setting
  `doubleEscapeAction`).
- `!cmd` runs a shell command and adds its output to the context; `!!cmd` runs it without
  adding it.
- `ctrl+l` model picker, `ctrl+p` next scoped model, `shift+tab` thinking level, `ctrl+d`
  quit (empty editor). All bindings: [keybindings.md](keybindings.md).

## Slash commands

Built-in: `/settings`, `/model`, `/thinking`, `/scoped-models`, `/new`, `/resume`, `/name`,
`/session`, `/tree`, `/fork`, `/clone`, `/compact`, `/import`, `/copy`, `/export`, `/reload`,
`/hotkeys`, `/changelog`, `/quit`; `/login [provider]` and `/logout [provider]`
run OAuth sign-in; `/trust` and `/extensions` only print guidance in this alpha. Extension commands (`/unipi:*`), prompt templates and `/skill:*`
appear in the same menu ([unipi.md](unipi.md)).

## Differences from pi

The UI uses the alternate screen by default (`--tui-mode regular` or setting
`tuiMode: "regular"` keeps the terminal scrollback); images render as `[image: mime]`
placeholders. Extension completions, the extension settings hub (`/settings`) and panel
buttons work. See
[dev/v1/deviations/tui-client.md](dev/v1/deviations/tui-client.md).
