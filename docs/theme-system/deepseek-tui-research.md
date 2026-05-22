# DeepSeek TUI Theme Research Notes

This note summarizes ideas to learn from the locally installed `deepseek-tui` crate. These are patterns to adapt into Oino-native design, not APIs or palettes to copy.

Inspected sources:

- `/home/pi/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/deepseek-tui-0.8.36/src/palette.rs`
- `/home/pi/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/deepseek-tui-0.8.36/src/deepseek_theme.rs`
- `/home/pi/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/deepseek-tui-0.8.36/src/config_ui.rs`

A quick web search for public DeepSeek TUI theming docs returned no useful results, so the local crate is the primary reference.

## Useful patterns

### 1. Small named theme IDs with aliases

DeepSeek TUI exposes stable theme identifiers such as `system`, `dark`, `light`, `grayscale`, `catppuccin-mocha`, `tokyo-night`, `dracula`, and `gruvbox-dark`, plus normalizers/aliases. This is good UX because users can type memorable names and config stays stable.

Oino adaptation:

- Provide built-in IDs like `oino-dark`, `oino-light`, `oino-mono`, and a DeepSeek-inspired-but-distinct initial theme such as `oino-aurora`.
- Accept aliases, but persist canonical IDs.
- Show display name + tagline in `/settings theme`.

### 2. `system` theme follows terminal background

DeepSeek TUI uses `COLORFGBG` to infer light/dark when `system` is selected. Missing/unparseable falls back to dark.

Oino adaptation:

- Support `system` as the default effective theme.
- Detect terminal background once at startup and optionally on reload.
- Keep explicit project/global theme IDs deterministic.

### 3. Palette mode separate from concrete colors

DeepSeek TUI has a palette mode concept (`Dark`, `Light`, `Grayscale`) and concrete `UiTheme` values containing surface, panel, elevated, composer, selection, header/footer, status, text, and border colors.

Oino adaptation:

- Separate `ThemeMode` from `ThemeSpec`.
- Let components ask for semantic roles, not raw colors.
- Use mode for fallback logic, contrast validation, and syntax-highlight adaptation.

### 4. Explicit surface hierarchy

DeepSeek TUI names `surface_bg`, `panel_bg`, `elevated_bg`, `composer_bg`, `selection_bg`, `header_bg`, and `footer_bg`.

Oino adaptation:

- Add first-class roles for app background, panel background, elevated/overlay background, composer background, selection background, status/footer background.
- This closes the largest current Oino gap: the app mostly themes foreground/borders, so themes can appear invisible.

### 5. Component tokens for heavily reused components

DeepSeek TUI has a smaller `deepseek_theme.rs` layer for section chrome, tool cells, and plan cells: border type/color, padding, title/value/label styles, status colors.

Oino adaptation:

- Keep a generic semantic layer, but provide component-specific structs for high-use widgets: panels, lists, composer, transcript messages, tool rows, markdown/code, extension surfaces.
- Avoid hard-coding Ratatui widget details everywhere; expose helper methods like `theme.panel_block(active)` and `theme.list_row(selected)` where useful.

### 6. Theme picker metadata

DeepSeek TUI themes include display names and taglines for picker rows.

Oino adaptation:

- Theme registry entries should include `id`, `display_name`, `description/tagline`, `mode`, `source`, and `preview_tokens`.
- `/settings theme` should show source and scope: built-in, extension, project file, global file.

### 7. Background override is simple and powerful

DeepSeek TUI allows a `background_color` override independent of the selected theme.

Oino adaptation:

- Allow small project-level overrides without forking an entire theme, e.g. `background`, `accent`, `selection_bg`, or any token override.
- UX should support "project inherits global theme but overrides accent/background".

## What not to copy

- Do not copy DeepSeek palette values or theme IDs wholesale.
- Do not keep a flat one-struct-only theme forever; Oino has extension surfaces, markdown-rich transcripts, tools, and settings overlays that need component roles.
- Do not make the theme UX config-only. Oino should have a TUI picker/preview.

## Oino first-theme implications

The initial Oino system should include:

1. Built-in theme registry with a small set of curated themes.
2. Semantic palette with foreground, background, surface, elevated, muted, accent, success, warning, error.
3. Component roles for app shell, panels, lists, composer, transcript, tool rows, markdown/code, status/footer, settings, and extension surfaces.
4. Project/global precedence and a `/settings theme` UX to preview and set either scope.
5. Extension theme contributions as additional registry entries, with conflict handling and user overrides.
