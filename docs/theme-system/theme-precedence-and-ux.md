# Oino Theme Precedence and UX Design

This document designs project/global theme behavior and the user-facing theme picker.

## Settings scopes

Oino already has two settings scopes:

- **Global** — `~/.oino/settings.json`
- **Project** — `<project>/.oino/settings.json`

Theme selection should use the same model as tools/extensions: project settings are an explicit project override, not a separate hidden mode.

## Theme settings shape

Add a `theme` settings object to both global and project `UserSettings`.

```json
{
  "theme": {
    "active": "oino-aurora",
    "overrides": {
      "app.bg": "#08111f",
      "accent": "#7dd3fc"
    }
  }
}
```

Rules:

- If `project.theme.active` is set, it chooses the effective theme ID.
- If `project.theme.active` is unset, inherit `global.theme.active`.
- If global is unset, default to `system`.
- Global overrides apply to the global-selected theme.
- Project overrides apply last and can be used with inherited global theme.
- Resetting the project theme removes `project.theme.active` and `project.theme.overrides` unless the user chooses to clear only one.

## Theme source registry

The picker lists resolved themes from these sources:

1. Built-in themes.
2. Global user theme files: `~/.oino/themes/**/*.json`.
3. Enabled global extension theme contributions.
4. Enabled project extension theme contributions.
5. Project theme files: `<project>/.oino/themes/**/*.json`.

Project files are registered last so they win duplicate theme IDs according to the source precedence table below.

Each theme row should display:

```text
source  scope  id  display name  mode  status
```

Example:

```text
builtin  global   system       System               system  inherited
builtin  global   oino-dark    Oino Dark            dark
file     project  team-brand   Team Brand           dark    PROJECT ACTIVE
extension project  example     Example Extension    dark
```

## Theme ID conflict precedence

When multiple sources provide the same `id`, choose the visible candidate using:

```text
project file > project extension > global file > global extension > built-in
```

Diagnostics should show all shadowed candidates in `/settings theme` and `/extensions → Registered`.

Extension contribution conflict policy still applies. If two extension themes provide the same ID, normal extension override controls choose the active contribution before theme-source precedence is applied.

## Effective theme resolution

Effective theme computation:

```text
base_id = project.active_theme
       || global.active_theme
       || "system"

base_theme = resolve(base_id) according to theme source precedence
resolved = built_in_defaults
resolved = merge(resolved, inherited themes)
resolved = merge(resolved, base_theme)
resolved = merge(resolved, global.overrides) if global.active participates
resolved = merge(resolved, project.overrides)
```

For a project-selected theme, global token overrides should not unexpectedly recolor it unless the user selects "inherit global overrides" later. The first implementation should keep this simple:

- Inherited global theme: apply global overrides, then project overrides.
- Project-selected theme: apply project-selected theme, then project overrides.

## `/settings theme` UX

Add a dedicated theme page reachable by:

```text
/settings theme
/theme
```

### Layout

```text
┌ Settings › Theme ─────────────────────────────────────────────┐
│ Effective: Oino Aurora  Scope: project override               │
│ Global: Oino Dark       Project: Oino Aurora                  │
├ Themes ───────────────────────────────┬ Preview ──────────────┤
│ › PROJECT ACTIVE  oino-aurora         │ App title             │
│   GLOBAL ACTIVE   oino-dark           │ User message          │
│   builtin         system              │ Assistant message     │
│   extension       example-theme       │ Tool running/done     │
│   project file    team-brand          │ Markdown + code       │
├────────────────────────────────────────┴──────────────────────┤
│ Enter preview • p set project • g set global • r reset project │
│ / search • o overrides • e edit/open file • Esc back           │
└───────────────────────────────────────────────────────────────┘
```

### Core actions

| Key | Action |
| --- | --- |
| `Enter` | Preview selected theme without saving. |
| `p` | Save selected theme as project theme. |
| `g` | Save selected theme as global theme. |
| `r` | Reset project theme to inherit global. |
| `R` | Reset global theme to `system`. |
| `/` | Search themes by ID/name/source/mode. Planned follow-up; not in the first picker slice. |
| `o` | Open token override editor for current scope. Planned follow-up. |
| `O` | Clear overrides for current scope. Planned follow-up. |
| `v` | Cycle preview sample: shell, transcript, markdown/code, extensions. Planned follow-up. |
| `e` | Open/edit theme file when the selected source is a file theme. Planned follow-up. |
| `Esc` | Leave preview or return to settings. |

### Preview behavior

- Preview is immediate but unsaved.
- Status line should say `Previewing <theme>; p project / g global / Esc cancel`.
- Leaving the theme page without saving reverts to the effective persisted theme.
- Saving commits the preview to the chosen scope and updates the effective badge.

## Footer/status visibility

The footer should expose theme state without taking much space:

```text
Theme: Oino Aurora(project)
```

If project inherits global:

```text
Theme: Oino Dark(global)
```

If previewing:

```text
Preview: Team Brand • p project / g global / Esc cancel
```

## Project-level workflow

Recommended workflow for a project team:

1. Create or install a theme.
2. Open `/settings theme`.
3. Select the theme.
4. Press `p` to save it to `<project>/.oino/settings.json`.
5. Commit `.oino/settings.json` and optional `.oino/themes/team.json` if the team wants shared project theming.

This makes project theming explicit and reviewable.

## Extension theme UX

Extension themes should appear in both places:

- `/extensions → Registered` — raw contribution visibility, enable/disable, conflicts.
- `/settings theme` — user-facing selection/preview.

If an extension package is disabled, its theme disappears from `/settings theme`. If the active project/global theme disappears, Oino should fall back to `system` and show a warning:

```text
Theme `example-theme` unavailable; using System. Re-enable the extension or choose a new theme.
```

## Override UX

Theme settings already persist a token override map, but a dedicated override editor is a follow-up. The intended compact editor is:

```text
Scope: project overrides
accent               #7dd3fc
app.bg               #08111f
list.selected_bg     #213a5a
```

Actions:

- `a` add token override.
- `Enter` edit value.
- `x` delete selected override.
- `O` clear all overrides for scope.
- `Tab` switch project/global override scope.

Token completion should use the canonical token registry from the schema design.

## Implemented component coverage

The first implementation resolves theme tokens into a runtime `Theme` and applies roles across:

- app, panel, composer, list, status, footer, selection, and focus states;
- transcript message, tool, thinking, resource, markdown, and syntax spans;
- settings menu rows and extension-management rows;
- suggestion panels, badges, diagnostics, extension surfaces, and command category labels.

Syntax colors are Oino role colors (`syntax.keyword`, `syntax.string`, etc.) derived from Syntect scope categories, not an externally selected Syntect theme.

## Conflict and diagnostic display

Rows should carry badges:

```text
ACTIVE
PROJECT ACTIVE
GLOBAL ACTIVE
PREVIEW
SHADOWED
INVALID
MISSING BASE
EXTENSION DISABLED
```

Selecting an invalid/shadowed row shows diagnostics in the preview pane rather than applying a broken theme.

## Implementation slices

1. Add theme registry/data model and built-in theme definitions.
2. Add project/global theme settings and effective resolution.
3. Replace current `theme_with_extension_tokens` bridge with resolved theme application.
4. Add `/settings theme` picker with preview and scope actions.
5. Load theme files and extension theme paths.
6. Expand component styles using the classification matrix.
