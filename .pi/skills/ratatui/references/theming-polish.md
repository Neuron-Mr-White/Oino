# Theming and polish subskill

Use for palettes, semantic styles, selected/focused states, low-color fallbacks, icons, status colors, contrast, command bars, and visual consistency.

## Source inspirations
`material` crate, mdfrier/mdfried, oatmeal, gitui, trippy, oxker, rainfrog, openapi-tui, yozefu.

## Related references

- `layouts.md`
- `document-viewers.md`
- `chat-agent.md`
- `testing-evals.md`

## Core rule: semantic styles first

Do not scatter raw colors across widgets. Define a theme struct with semantic surfaces:

```rust
struct Theme {
    fg: Color,
    bg: Color,
    muted: Color,
    focused_border: Color,
    dialog_border: Color,
    selected_focused: Style,
    selected_unfocused: Style,
    active: Style,
    disabled: Style,
    success: Style,
    warning: Style,
    error: Style,
    link: Style,
    code: Style,
}
```

Then render with `theme.selected_focused` etc. This makes light/dark/custom themes possible and keeps focus/selection consistent.

## Palette/color-system pattern (material)

Material’s CLI shows how to expose a palette clearly:

- Represent palettes as families × variants with stable short codes (`A0`, `A1`, …).
- Render a centered swatch grid by computing `sq_width = area.width / n_colors` and `sq_height = (area.height - footer) / n_variants`.
- For each swatch, set `bg` to the color and choose foreground by shade: dark text on light variants, white text on mid/dark variants.
- Put copy/search input and feedback in a bordered footer below the grid (`Type color code to copy`, current input, copied message).
- For libraries, implement conversion into `ratatui::style::Color` so app code can do `Style::new().fg(my_color.into())`.

## Contrast and selection

- Focused border should be different from selected row. A thick/colored focused border plus selected background is clearer than background alone.
- Selected but unfocused rows should still be visible, but quieter than selected+focused.
- Active/live statuses (`Live`, current clock, selected hop) can use strong color; inactive context should be muted.
- Error text/borders should not rely only on red if terminal may be low-color; pair with labels/icons (`Error`, `!`, `⚠`).

## Markdown/document theme traits

mdfrier uses a theme trait that extends a symbol mapper and provides semantic methods:

- `blockquote_color(depth)` cycles colors by nesting depth.
- `link_fg`, `link_bg`, and underline styles distinguish link description and URL wrappers.
- `code_fg/code_bg`, `emphasis_style`, `strong_emphasis_style`, `strikethrough_style` map markdown semantics to Ratatui styles.
- Table border/header and horizontal-rule styles are separate surfaces.

Use this pattern for any renderer that maps semantic content to `Line`/`Span`: keep symbol choices and color choices overridable.

## Chat bubble polish

- Align user bubbles right and assistant/system bubbles left.
- Put author name into top border when space allows.
- Use border color to signal author or error (`Error` bubbles red, assistant branded color, user default).
- Enforce a minimum terminal width; if bubble border + author + padding cannot fit, render a clear tiny-width message rather than broken borders.
- Number code blocks in chat so slash commands/copy actions can reference them (` ```rust (3) `).

## Icons and glyphs

- Icons add scanability but should not be the only meaning. Pair `💣`, `⛳`, `⚠`, `Live`, `Frozen`, etc. with labels when consequences matter.
- Use box drawing consistently: do not mix rounded/double/thick borders randomly. Reserve thick/double for focus/modals.
- For low-color terminals, prefer modifiers (`BOLD`, `DIM`, `UNDERLINED`) plus text labels.

## Config-driven themes

For real apps:
- Parse style strings/config into `Style` once at startup.
- Keep keymaps and help text generated from config, not hard-coded, so custom keybindings do not lie.
- Snapshot test default, dark/light, and one custom theme. Add cell-level assertions for focused border, selected row, error popup, and muted footer.

## Testing checks

- Contrast for all selected/focused combinations.
- Palette swatches choose readable foreground.
- Low-color fallback has labels/modifiers.
- Theme changes affect every component surface consistently.
- Command/footer/help text uses current keymap and theme.
