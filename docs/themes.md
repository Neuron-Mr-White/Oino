# Themes

The terminal UI ships pi's built-in `dark` and `light` themes (same colour tokens as pi's
`theme-schema.json`). Pick one with `/settings` → Theme or `--use-theme <name>` for a single
run; the choice is stored as `theme` in settings.

Theme files (`~/.oino/themes/*.json`, `<project>/.oino/themes/`, `--theme <path>`) are
discovered like pi's, but this alpha renders only the two built-in themes; custom theme files
are not applied yet. Oino Desktop has its own light/dark appearance (Settings → General).
