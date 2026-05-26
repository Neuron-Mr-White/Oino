# Theme Development

Themes define semantic colors for the Oino TUI. Users pick them with `/theme` or from settings.

## Theme sources

Themes can come from built-ins, user/project files, or extensions. Run `/reload` after editing theme files.

## Basic shape

```json
{
  "id": "my-theme",
  "display_name": "My Theme",
  "mode": "dark",
  "palette": {
    "bg": "#101014",
    "fg": "#eeeeee",
    "accent": "#7aa2f7"
  },
  "tokens": {
    "app.bg": "$palette.bg",
    "text.primary": "$palette.fg",
    "accent": "$palette.accent",
    "composer.border_focused": "$palette.accent"
  }
}
```

## Tips

- Prefer semantic tokens over hard-coded component guesses.
- Test dark and light terminals when possible.
- Keep contrast high for selected rows and status text.
- Check `/settings` and `/extensions`; they exercise many UI components.
