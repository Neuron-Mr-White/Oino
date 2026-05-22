# Oino Theme Component Classification

This is the initial component/text classification matrix for the first-class Oino theme system. It is intentionally Oino-native: extension theme contributions, built-in themes, and project overrides should all resolve into these shared roles instead of each widget inventing one-off colors.

See also: [`README.md`](README.md) for the current theme-docs index and [`text-inventory.generated.md`](text-inventory.generated.md) for the generated first-pass literal inventory.

## Inventory scope

Current render/text sources inventoried in iteration 1:

- `crates/oino-tui/src/render.rs` — top-level layout, extension panels, transcript viewport, composer, suggestions, overlays, settings/keymaps.
- `crates/oino-tui/src/transcript.rs` — chat/agentic/minimal transcript styles, tool calls/results, resource cards, bubbles.
- `crates/oino-tui/src/markdown.rs` — markdown headings, inline styles, links, quotes, lists, tables, code blocks, syntax spans.
- `crates/oino-tui/src/composer.rs` — composer content model and collapsed-resource labels.
- `crates/oino-tui/src/settings.rs` — settings/keymap interaction status strings.
- `crates/oino-tui/src/help.rs` — help rows and sections.
- `crates/oino-tui/src/command.rs` — slash command labels, aliases, suggestions.
- `crates/oino-tui/src/app.rs` — state/status/action text and overlay behavior messages.
- `crates/oino-tui/src/keymap.rs` — keymap action labels/descriptions.
- `crates/oino-tui/src/message.rs`, `resource.rs`, `theme.rs` — message/resource/theme model text.

Generated first pass: **1,574** non-test string literals.

## Theme classifications

| Classification | Components / text sources | Shared theme roles needed |
| --- | --- | --- |
| App shell | Root area, tiny-terminal fallback, app title, transcript frame, scrollbar | `app.bg`, `app.fg`, `app.border`, `app.border_focused`, `app.title`, `app.warning`, `app.error` |
| Panels / overlays | Help, send panel, sessions, prompts, skills, inspect, settings, extensions, extension settings | `panel.bg`, `panel.fg`, `panel.border`, `panel.border_focused`, `panel.title`, `panel.shadow_or_dim`, `panel.footer` |
| Lists / selection | Overlay rows, model lists, settings rows, keymap rows, extension rows, resource rows | `list.fg`, `list.muted`, `list.selected_fg`, `list.selected_bg`, `list.cursor`, `list.badge`, `list.separator` |
| Composer | Composer frame, input, placeholder, cursor, collapsed paste/resource labels | `composer.bg`, `composer.fg`, `composer.placeholder`, `composer.border`, `composer.border_focused`, `composer.cursor`, `composer.reference` |
| Command/autosuggest | Slash suggestions, model/thinking/resource suggestions, extension autosuggest badges | `suggestion.bg`, `suggestion.fg`, `suggestion.match`, `suggestion.category`, `suggestion.border`, `suggestion.selected_bg` |
| Status/footer | Footer hints, status line, working indicator, inline status, top/bottom extension status | `status.bg`, `status.fg`, `status.muted`, `status.working`, `status.success`, `status.warning`, `status.error`, `status.extension` |
| Chat transcript | Chat bubbles, agentic rows, minimal rows, user/assistant titles and borders | `message.user.*`, `message.assistant.*`, `message.system.*`, `message.error.*`, `message.border`, `message.bg` |
| Tool activity | Tool call running rows, tool result rows, collapsed summaries, exploration markers | `tool.title`, `tool.fg`, `tool.muted`, `tool.running`, `tool.success`, `tool.error`, `tool.border`, `tool.output`, `tool.bg` |
| Thinking/reasoning | Thinking sections, collapsed thinking markers, thinking level labels | `thinking.fg`, `thinking.muted`, `thinking.border`, `thinking.bg`, `thinking.live`, `thinking.collapsed` |
| Resources | Prompt/skill attachments, included files, resource cards, attachment borders | `resource.title`, `resource.fg`, `resource.muted`, `resource.border`, `resource.bg`, `resource.badge` |
| Markdown prose | Headings, paragraphs, emphasis/strong/strike, links, footnotes, HTML placeholders | `markdown.fg`, `markdown.heading`, `markdown.heading_secondary`, `markdown.link`, `markdown.link_url`, `markdown.marker`, `markdown.muted` |
| Markdown blocks | Quotes, lists, task lists, tables, horizontal rules, code block borders/line numbers | `markdown.quote`, `markdown.quote_border`, `markdown.list_marker`, `markdown.table_border`, `markdown.code_bg`, `markdown.code_border`, `markdown.code_line_number` |
| Syntax highlighting | Syntect-derived spans in fenced code | `syntax.comment`, `syntax.keyword`, `syntax.function`, `syntax.variable`, `syntax.string`, `syntax.number`, `syntax.type`, `syntax.operator`, `syntax.punctuation`, plus a named syntax theme override later |
| Settings/keymaps | Settings menu, child pages, keymap capture/preset confirm, conflict warnings | `settings.title`, `settings.fg`, `settings.muted`, `settings.active`, `settings.changed`, `settings.warning`, `settings.danger` |
| Extension management | `/extensions` Manage/Registered tabs, contribution families, conflicts, diagnostics, toggles | `extension.package`, `extension.runtime`, `extension.contribution`, `extension.enabled`, `extension.disabled`, `extension.conflict`, `extension.diagnostic`, `extension.override` |
| Extension surfaces | Sidebars, main panels, floating panels, footer top/bottom/inline, extension settings pages | `extension_surface.bg`, `extension_surface.fg`, `extension_surface.border`, `extension_surface.focused_border`, `extension_surface.title`, `extension_surface.tab_active`, `extension_surface.tab_inactive`, `extension_surface.conflict` |
| Diagnostics/errors | Error bubbles, denied permissions, validation errors, uninstall confirmations | `diagnostic.info`, `diagnostic.warning`, `diagnostic.error`, `diagnostic.success`, `diagnostic.danger_bg` |
| Badges/tokens | Mode labels, scope labels, provider/tool/resource badges, P/G toggles, conflicts | `badge.fg`, `badge.bg`, `badge.accent`, `badge.success`, `badge.warning`, `badge.error`, `badge.muted` |

## First schema direction

The first Oino theme schema uses three layers:

1. **Palette** — reusable colors (`background`, `surface`, `elevated`, `text`, `muted`, `accent`, `success`, `warning`, `error`, etc.).
2. **Semantic roles** — component-independent meanings (`fg`, `bg`, `border`, `selected_bg`, `focused_border`, `live`, `danger`).
3. **Component overrides** — optional role overrides for `composer`, `panel`, `list`, `message`, `tool`, `markdown`, `syntax`, `settings`, and `extension_surface`.

This keeps simple themes small while allowing project/extension-specific polish.

## UX direction for project-level theming

Implemented settings entry points:

```text
/settings theme
```

Implemented actions:

- `Enter` preview selected theme immediately.
- `p` set project theme.
- `g` set global theme.
- `r` reset project override to inherit global.
- `R` reset global theme to `system`.
- Footer/settings state shows effective precedence: `project override → global setting → system/default`.

Theme search and preview-sample switching remain follow-ups tracked in [Theme precedence and UX](theme-precedence-and-ux.md).

Project-level theming should not require editing extension policy manually; it should persist through the project settings file and be visible as an effective theme badge in the footer/settings UI.
