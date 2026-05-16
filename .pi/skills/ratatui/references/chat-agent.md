# Chat and agent terminal subskill

Use for agent shells, chat transcripts, streaming answers, tool-call logs, approvals, model pickers, composers, slash commands, mentions, markdown/code in chat, and terminal assistants.

## Source inspirations
DeepSeek-TUI (`ui.rs`, `composer_ui.rs`, `widgets/*`, `paste_burst.rs`, `footer_ui.rs`, `tool_routing.rs`), OpenAI Codex TUI (`chatwidget/*`, `bottom_pane/*`, `tui/frame_requester.rs`, `clipboard_paste.rs`), oatmeal.

## Related references

- `input-focus.md`
- `streaming-async.md`
- `document-viewers.md`
- `theming-polish.md`

## State model

Keep separate state for:
- transcript/history cells (user, assistant, tool start/end, reasoning, errors, approvals);
- composer draft (`textarea`, cursor, history search, attachments/mentions, pending pastes, bash/mode flags);
- active turn/backend status (`is_loading`, running tools, cancellation available, rate-limit/status cards);
- scroll state (at bottom, manual scroll offset, live tail overlay);
- bottom pane/modal (`slash menu`, file mention picker, approval popup, settings/model picker);
- frame requester/dirty redraw state.

Do not store rendered lines as the only source of truth. Preserve source messages so resize can rewrap/reflow.

## Layout recipe

Use a vertical app shell:

```text
header/session/status (optional)
transcript/body        flexible
pending preview/tools  optional
composer              dynamic height
footer/key hints       1-2 lines
```

- Composer height grows with content but never steals all transcript space; enforce min chat height and min composer height.
- Hide sidebars below a width threshold (DeepSeek uses a sidebar min width around 100 columns).
- Add pending-input preview above the composer for queued drafts/context attachments.
- In narrow terminals, collapse rich cards into one-line status/tool summaries.

## Composer behavior

- Use a multiline text area/composer, not raw strings, once you support history, cursor motion, paste, or mentions.
- Support newline with `Ctrl-J`, `Alt-Enter`, or `Shift-Enter` when terminal can disambiguate; plain `Enter` submits.
- Escape should be contextual. DeepSeek-style priority: close slash menu -> cancel request -> discard queued draft -> clear input -> noop.
- Up/Down can navigate history when composer has text; optionally scroll transcript when composer is empty.
- Gate input with `input_enabled` and placeholder text while backend is in a state that cannot accept edits.

## Slash commands, mentions, and bottom panes

- Treat slash menu, file mention picker, custom prompt args, approvals, and settings as bottom-pane views with their own key handling and completion result.
- Limit source results high enough for keyboard navigation; paginate/center selected row in render rather than truncating the actual candidate list.
- Return structured results (`Submit`, `Cancel`, `InsertText`, `OpenView`) instead of letting the popup mutate app internals.

## Streaming transcript

- Append/merge streaming chunks into the active assistant/tool cell.
- Schedule/coalesce frames via `FrameRequester`; do not draw on every token chunk.
- Keep a forced low-rate status animation tick for spinners/tool pulses while a turn is live.
- Preserve scroll intent: if user was at bottom, follow new chunks; if user scrolled up, do not yank them down. Provide a live-tail indicator or shortcut.
- For resize, rebuild wrapped transcript cells from source history and do one full reflow pass.

## Tool calls and approvals

- Represent tool lifecycle explicitly: pending, running, succeeded, failed, cancelled, requires approval.
- Render noisy progress as compact one-line summaries in footer/status; let users open a pager/details view for full logs.
- Approval popups must capture focus and block background shortcuts until answered.
- Long tool output should become a pager/view, not an enormous chat bubble.

## Paste handling

- Enable bracketed paste when supported.
- Normalize CRLF/CR to LF before inserting; Codex documents iTerm2 paste CR behavior.
- Detect paste bursts so accidental multi-line paste can be previewed/confirmed or batched.
- Do not interpret pasted leading `/` as a slash command unless explicitly desired.

## Rendering details

- Cache wrapped bubble/code-block lines by `(message_id, width, streaming_generation)` and invalidate on width or content change.
- Syntax-highlight fenced code blocks; include line numbers for code when useful, but keep short command snippets compact.
- Use semantic message styles: user, assistant, tool, warning, error, approval, muted metadata.
- Keep terminal notifications optional and focus-aware: Codex only emits desktop notifications when terminal is unfocused and policy allows it.

## Testing checks

- Submit vs newline key behavior.
- Escape priority states.
- Paste normalization and paste burst handling.
- Streaming chunk coalescing does not emit unbounded frames.
- Resize reflows transcript from source.
- Composer cursor style/position only appears when focused.


## Oatmeal bubble transcript pattern

Oatmeal is a compact reference for chat bubbles and code-block UX.

### Bubble geometry

- Determine maximum line length from message lines and author name, then clamp to terminal width minus border elements and outer padding.
- Use `BubbleAlignment::{Left, Right}`: user messages right-aligned, assistant/system left-aligned.
- Build each line with left border, text spans, fill spaces, right border, then add outer padding on the left or right.
- Put author name into the top border by replacing a run of `─` with the username.
- Error bubbles use error-colored borders; assistant bubbles can use brand color; normal user bubbles can stay default.

### Bubble caching

- Cache rendered bubble lines by message index.
- Invalidate all cache when width changes.
- For all but the last message, reuse cache if present. For the last message, reuse only if text length is unchanged so streaming updates re-render.
- Track cumulative code-block count through cached entries so code block numbering stays stable.

### Code blocks

- Detect fenced code blocks while building bubble lines.
- Use syntect per language; append `\n` before highlighting a code line so multiline grammar state is correct, then trim the last segment.
- Increment and display code-block numbers at fence start; keep a separate `CodeBlocks` service that can return the latest block, individual indices, ranges (`2..5`), or comma lists for slash commands/copy actions.

### App shell

- Before rendering bubbles, verify the terminal is wide enough for author + padding + borders. If not, show a clear “make me bigger” message.
- Layout: message list with scrollbar + textarea/composer. Composer height is `textarea.lines() + 3`.
- While waiting for backend, replace composer with loading widget and ignore normal text input.
- `Ctrl-C` while backend is running aborts request; otherwise first `Ctrl-C` shows a quit warning and second quits.
- Pasted text normalizes `\r` to `\n` and uses textarea paste/yank APIs.
