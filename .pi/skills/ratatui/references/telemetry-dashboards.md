# Telemetry and dashboard subskill

Use for bandwidth, Docker/system, traceroute/network, charts, maps, live tables, runtime info bars, and operations dashboards.

## Source inspirations
bandwhich (`display/components/layout.rs`, `table.rs`), oxker (`ui/mod.rs`, `gui_state.rs`, draw blocks), trippy (`frontend/render/*`, `tui_app.rs`, `columns.rs`).

## Related references

- `layouts.md`
- `tables-grids.md`
- `streaming-async.md`
- `theming-polish.md`

## Layout patterns

### Progressive pane dropping (bandwhich)

- Reserve header/footer rows first.
- For 1 child, use full body.
- For 2 children: if both height and width are below breakpoints, show one pane; if narrow, vertical split; otherwise horizontal split.
- For 3 children: if too small, show one; if short, show two horizontal panes; if narrow, stack vertically; otherwise top two quarters + bottom half.
- Let user cycle which table appears in limited slots (`table_cycle_offset`).

### Operations dashboard (oxker)

- Top header line, optional one-line filter/search bar, main content.
- Main content: containers/logs vertical stack; commands sidebar only if containers exist; lower charts/ports only if data exists.
- Calculate narrow side sections from measured content (ports width from longest port fields), not fixed percentages.
- Modal statuses (`Inspect`, `DeleteConfirm`, `Help`, `Error`) override normal layout.

### Network visualizer (trippy)

- Header always.
- Optional tabs if multiple targets; optional flows view if flow mode active.
- Body switches among splash/error/table/chart/map.
- Footer charts plus one-line runtime info bar.
- Centered help/settings overlays with `Clear` and double borders.

## State model

- Split data from GUI state. Data holds samples/containers/flows; GUI state holds selected panel, selected hop/container, toggles, scroll offsets, filters, map/chart visibility, freeze state.
- For draw, build a snapshot (`FrameData`, trace snapshot) so render does not repeatedly lock shared data.
- Support freeze/pause mode: stop ingesting/updating selected context but keep navigation/settings live.

## Tables, charts, and maps

- Telemetry tables need configurable typed columns: fixed-width columns keep known widths; variable columns divide leftover width.
- Selected context should drive charts: selected hop -> latency chart/frequency histogram; selected container -> CPU/mem/bandwidth/logs.
- Hide charts/maps when area is too small; show a compact table/status fallback.
- Ratatui `Canvas` maps need privacy controls, selected-point overlays, and a clear info panel.

## Input and mouse

- Store region maps for headers, panels, and buttons; clicks become `FocusPanel`, `SortHeader`, `ToggleHelp`, `Confirm/Cancel`.
- Avoid full mouse motion unless dragging/resizing; down/scroll events are enough for most dashboards.
- Provide keymap-configurable actions for sort, filter, logs toggle, freeze, chart/map/detail toggles, and settings/help.

## Status bars

- Show immutable runtime config (protocol, privilege, locale, target) separately from toggles (ASN, DNS/address mode, privacy, max hosts, details, freeze).
- Use filled/empty markers or muted labels so dense info remains scannable.
- Show `Live`, `Frozen`, `Filtering`, `Error`, and loading indicators near the data they refer to.

## Testing checks

- Breakpoint behavior at narrow/short sizes.
- Hidden optional panes do not receive focus.
- Region maps clear on resize.
- Configurable column order/visibility affects layout correctly.
- Freeze mode prevents new data from changing displayed snapshot.
