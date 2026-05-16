# Input, focus, and keybindings subskill

Use for focus routing, modes, keymaps, command bars, search, text input, paste, autocomplete, shortcuts, mouse routing, and modal priority.

## Source inspirations
Codex bottom panes/chat composer, DeepSeek composer/slash menus, binsider `Command`, openapi-tui footer command mode, rainfrog Vim editor, eilmeldung key sequences, fzf-make picker, yozefu search/footer/root.

## Related references

- `architecture.md`
- `layouts.md`
- `streaming-async.md`
- `testing-evals.md`

## Routing priority

Route input in this order:

1. Fatal/terminal-level shortcuts if any (`Ctrl-C` immediate quit, suspend).
2. Top modal/overlay/view (`help`, `settings`, approval, confirmation, picker).
3. Active text editor/input mode.
4. Focused component/pane.
5. Global navigation shortcuts.

Use an enum, not booleans, for exclusive modes:

```rust
enum InputMode { Normal, Insert, Command }
enum Focus { Menu, Editor, Data, History, Favorites, Popup }
```

Use a set only when modes can overlap, like oxker `HashSet<Status>` (`Help`, `Filter`, `SearchLogs`, `Inspect`, `DeleteConfirm`, `Exec`).

## Command conversion

For simple/medium apps, convert keys to commands before mutation:

- arrows and `hjkl` for movement;
- `Tab`/`BackTab` for pane focus;
- `/` for search, `:` for command line;
- `[`/`]`, `g/G`, `Home/End` for top/bottom or tab variants;
- `Esc` for close/cancel before quit;
- mouse wheel maps to same scroll commands as keyboard.

Special embedded widgets may translate keys: binsider maps user keys into heh hex viewer control keys while preserving original events for heh search/jump windows.

## Focus and view stacks

- Store `focus_order` per view and `focus_history` so closing a popup returns to the previous meaningful pane.
- `Esc` should pop the current view. At root, consider notifying “Press Ctrl-C to exit” rather than quitting if accidental quit is costly (yozefu pattern).
- When toggling full-screen or hiding panes, validate current focus; if hidden, move to first visible focus target.

## Text input and command bars

- Use `tui-input` for single-line filter/search/command/footer.
- Use `tui-textarea` for multiline editors/composers/SQL.
- Set cursor manually only when the input is focused.
- Render parse errors inline by styling the unparsed suffix/remaining input, like yozefu search.
- Command footer pattern (openapi-tui): `FocusFooter(cmd, seed)` enters command mode, seeds `/` or `:`, stores history, renders prompt + visual-scrolled input, and returns `FooterResult(cmd,args)`.

## Autocomplete and history

- Keep history separate from input value.
- Filter possibilities by current prefix; Up/Down cycles history/completions; Right/Tab accepts a muted preview suffix.
- Debounce expensive validation with cancellation tokens; do not parse on every keystroke synchronously.

## Paste and multi-key sequences

- Enable bracketed paste for serious text input.
- Normalize pasted CR to LF.
- For paste bursts, temporarily collect pasted chunks and process after a tick/delay to avoid treating each chunk as a separate command.
- Multi-key commands need a pending buffer with timeout. Yozefu uses a small buffer for `gg`/`GG`; eilmeldung-style keymaps can show prefix help until timeout.

## Dynamic shortcuts/footer

Each component should expose `shortcuts()`. Root merges focused component shortcuts with global ones (`Help`, `Next panel`, `Quit`, `Close` if stacked) and sends them to a footer. This prevents stale key hints after focus/mode changes.

## Mouse routing

- Store per-frame rect maps for click targets (panels, headers, buttons) and clear on resize.
- Convert clicks/scrolls to semantic actions (`FocusPanel`, `SortByHeader`, `ConfirmDelete`, `ScrollLogs`) rather than directly mutating render state.
- Avoid full mouse-motion capture unless drag resizing needs it; oxker captures down/scroll only for performance.

## Testing checks

- Mode priority: modal blocks background shortcuts.
- Focus cycling skips hidden/unavailable panels.
- Footer shortcuts change with focus/mode/follow state.
- Paste does not execute slash commands accidentally.
- Autocomplete accept/cancel and debounced validation.


## Data-grid input modes (csvlens)

For table-heavy tools, an input buffer can be more precise than generic `InputMode::Insert`:

- Default mode handles navigation and direct actions.
- Buffering modes include find `/`, row filter `&`, column filter `*`, option `-`, freeze columns `f`, and goto line when digits are typed.
- `Esc` cancels buffer and restores default mode; `Enter` confirms.
- Normalize Shift modifiers across platforms before matching keys; on Windows punctuation can arrive with `SHIFT`, while Unix may not.
- Treat digits in default mode as the start of a goto-line buffer rather than as commands.
