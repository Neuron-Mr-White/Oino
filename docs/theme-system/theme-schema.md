# Oino Theme Schema Design

This document defines the first Oino-native theme contract. It is a design target for the implementation that follows the component inventory and classification pass.

## Goals

- Make themes visibly affect Oino, including backgrounds, selection, focused borders, markdown/code, transcript/tool rows, settings, and extension surfaces.
- Keep simple themes easy: a small palette should be enough.
- Allow advanced themes to override component roles without forcing every theme to define every token.
- Preserve compatibility with existing extension `tokens` maps.
- Support built-in themes, theme files, extension theme contributions, global user settings, and project-level overrides.

## Theme document

Theme files should be JSON first because Oino extension manifests and user settings are already JSON.

```json
{
  "schema_version": 1,
  "id": "oino-aurora",
  "display_name": "Oino Aurora",
  "description": "Blue-green dark theme with visible project-friendly surfaces",
  "mode": "dark",
  "inherits": "oino-dark",
  "palette": {
    "bg": "#08111f",
    "surface": "#0f1b2d",
    "elevated": "#17263c",
    "text": "#e6eef8",
    "muted": "#91a4b8",
    "dim": "#5e7083",
    "accent": "#7dd3fc",
    "success": "#86efac",
    "warning": "#f6c177",
    "error": "#f38ba8",
    "selection": "#213a5a"
  },
  "tokens": {
    "app.bg": "$palette.bg",
    "panel.bg": "$palette.surface",
    "panel.border_focused": "$palette.accent",
    "list.selected_bg": "$palette.selection",
    "composer.bg": "$palette.elevated",
    "status.working": "$palette.warning",
    "extension_surface.tab_active": "$palette.accent"
  }
}
```

### Fields

| Field | Required | Notes |
| --- | --- | --- |
| `schema_version` | yes | Starts at `1`; reject future incompatible major versions with a diagnostic. |
| `id` | yes | Stable canonical theme ID. Lowercase, dot/dash/underscore safe. |
| `display_name` | yes | Human-readable name in theme picker. |
| `description` | no | One-line tagline or longer description. |
| `mode` | yes | `system`, `dark`, `light`, or `mono`. Used for fallback and contrast validation. |
| `inherits` | no | Optional base theme ID. Resolved before local tokens. |
| `palette` | no | Named reusable colors. Values can be named Ratatui colors, `#rrggbb`, indexed colors, `default`, or `reset`. |
| `tokens` | no | Flat semantic/component token map. Values are colors or `$palette.name` references. |
| `syntax` | no | Future syntax theme name or token overrides. |
| `metadata` | no | Optional author, license, homepage, preview swatches. |

## Token model

The canonical internal representation should be a flat `BTreeMap<ThemeToken, ThemeValue>`, where a token path is normalized across:

- snake case: `focused_border`
- kebab case: `focused-border`
- dotted paths: `panel.focusedBorder`
- camel case: `focusedBorder`

Existing extension tokens like `accent`, `success`, `focused_border`, and `toolTitle` remain aliases into the new role table.

## Required fallback roles

Every resolved theme must provide these minimum roles, either directly or through defaults:

```text
app.bg
app.fg
app.border
app.border_focused
app.title
panel.bg
panel.fg
panel.border
panel.border_focused
list.fg
list.muted
list.selected_fg
list.selected_bg
composer.bg
composer.fg
composer.placeholder
composer.border
composer.border_focused
status.bg
status.fg
status.muted
status.working
status.success
status.warning
status.error
message.user.fg
message.user.border
message.assistant.fg
message.assistant.border
tool.title
tool.fg
tool.muted
tool.running
tool.success
tool.error
markdown.fg
markdown.heading
markdown.link
markdown.code_border
extension_surface.bg
extension_surface.fg
extension_surface.border
extension_surface.focused_border
```

## Component role families

### App shell

```text
app.bg
app.fg
app.border
app.border_focused
app.title
app.warning
app.error
app.tiny_terminal
```

### Panel and overlay chrome

```text
panel.bg
panel.fg
panel.border
panel.border_focused
panel.title
panel.footer
panel.dim
```

### Lists and selected rows

```text
list.fg
list.muted
list.separator
list.cursor
list.selected_fg
list.selected_bg
list.badge
list.badge_bg
```

### Composer

```text
composer.bg
composer.fg
composer.placeholder
composer.cursor
composer.border
composer.border_focused
composer.reference
composer.collapsed_paste
```

### Suggestions

```text
suggestion.bg
suggestion.fg
suggestion.match
suggestion.category
suggestion.border
suggestion.selected_fg
suggestion.selected_bg
```

### Status/footer

```text
status.bg
status.fg
status.muted
status.working
status.success
status.warning
status.error
status.extension
```

### Transcript messages

```text
message.user.fg
message.user.bg
message.user.border
message.assistant.fg
message.assistant.bg
message.assistant.border
message.system.fg
message.system.bg
message.system.border
message.error.fg
message.error.bg
message.error.border
message.title
message.muted
```

### Tool activity

```text
tool.title
tool.fg
tool.muted
tool.border
tool.bg
tool.running
tool.success
tool.error
tool.output
tool.diff_added
tool.diff_removed
tool.diff_context
```

### Thinking/reasoning

```text
thinking.fg
thinking.muted
thinking.bg
thinking.border
thinking.live
thinking.collapsed
```

### Resources

```text
resource.title
resource.fg
resource.muted
resource.bg
resource.border
resource.badge
```

### Markdown/code

```text
markdown.fg
markdown.heading
markdown.heading_secondary
markdown.link
markdown.link_url
markdown.marker
markdown.muted
markdown.quote
markdown.quote_border
markdown.list_marker
markdown.table_border
markdown.code_bg
markdown.code_border
markdown.code_line_number
```

### Syntax highlighting

```text
syntax.comment
syntax.keyword
syntax.function
syntax.variable
syntax.string
syntax.number
syntax.type
syntax.operator
syntax.punctuation
```

The first implementation may keep Syntect colors as-is, but the theme model should reserve these roles.

### Settings/keymaps

```text
settings.title
settings.fg
settings.muted
settings.active
settings.changed
settings.warning
settings.danger
```

### Extension management and surfaces

```text
extension.package
extension.runtime
extension.contribution
extension.enabled
extension.disabled
extension.conflict
extension.diagnostic
extension.override
extension_surface.bg
extension_surface.fg
extension_surface.border
extension_surface.focused_border
extension_surface.title
extension_surface.tab_active
extension_surface.tab_inactive
extension_surface.conflict
```

## Alias compatibility

Existing extension token aliases should map as follows:

| Existing token | Canonical target |
| --- | --- |
| `accent` | `app.border_focused`, `panel.border_focused`, `status.extension` |
| `success` | `status.success`, `tool.success`, `extension.enabled` |
| `text`, `fg` | `app.fg`, `panel.fg`, `message.assistant.fg`, `markdown.fg` |
| `muted` | `status.muted`, `list.muted`, `message.muted`, `markdown.muted` |
| `dim` | `panel.dim`, `composer.placeholder` |
| `focused_border`, `borderAccent` | `app.border_focused`, `panel.border_focused`, `composer.border_focused` |
| `panel_border`, `border`, `borderMuted` | `panel.border`, `app.border`, `extension_surface.border` |
| `user_border`, `userMessageText` | `message.user.border` or `message.user.fg` depending on token name |
| `assistant_border`, `assistantMessageText` | `message.assistant.border` or `message.assistant.fg` depending on token name |
| `tool_border`, `toolTitle` | `tool.border` or `tool.title` depending on token name |
| `title` | `app.title`, `panel.title` |
| `warning` | `status.warning`, `settings.warning`, `diagnostic.warning` |
| `error` | `status.error`, `message.error.fg`, `diagnostic.error` |
| `footer`, `status`, `inline_status` | `status.fg` |
| `working`, `working_indicator` | `status.working`, `tool.running` |

## Validation rules

- Unknown token path: accepted with warning for extension compatibility; rejected only in strict devkit mode.
- Invalid color: ignored with warning; devkit validation fails in strict mode.
- Missing required roles: filled from inherited/default theme.
- Theme ID conflict: resolved through normal extension conflict policy and user overrides.
- Unsafe path in theme contribution: reject path traversal; theme files must live inside package/project/global theme roots.

## Built-in initial themes

The first implementation should include at least:

- `system` — follows terminal light/dark detection; defaults to `oino-dark` when unknown.
- `oino-dark` — current Oino default upgraded with explicit backgrounds and selection.
- `oino-light` — light variant for bright terminals.
- `oino-mono` — low-color grayscale theme.
- `oino-aurora` — a visible blue/green theme inspired by terminal AI UIs, including DeepSeek TUI's semantic-surface pattern, but with Oino's own palette and component roles.
