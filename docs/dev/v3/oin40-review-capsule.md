# OIN-40 — Review capsule + review modal (design, confirmed against tokens)

Confirmed by the lead on 2026-11-09 against `apps/unipi/src/style.css` (@theme tokens),
`components/superagent/superagent.css` and `components/superagent/DESIGN.md`.
No new palette, no new radius, no count animation (DESIGN.md "Status values: static").

## Capsule (Superagent header, top right)
- Placement: in `.sa-header`, pushed right (`margin-left:auto`), before the existing
  icon buttons (dashboard / + / history). The Auto-start pill stays in the pane.
- Shape: one `role="group"` pill, `border:1px solid var(--color-line)`,
  `background:var(--color-surface-2)`, `border-radius:var(--radius-pill)`, visual height 32px
  (hit target ≥44px on phone via padding/`::before`), two segment `<button>`s split by a 1px
  `var(--color-line)` divider.
- Segment "Review": label `Review` in `var(--color-text-muted)`, 13px × `--ui-font-scale`,
  then the count in the existing `.sa-badge` style (tabular-nums, 600, `--color-text`,
  `--color-surface-3`). Zero → `.sa-badge[data-zero]` (muted, 400). The Review segment is
  always shown, also at 0.
- Segment "Escalate" (blocked + needs-you with a question): shown only when count > 0.
  Count chip uses the existing attention tone, the same as `.agent-dot`:
  `color:var(--color-amber); background:rgb(240 168 24 / 14%)`. Never shell red
  (brand/primary/error).
- States: hover = shared hover (`--color-surface-3` background); focus = global
  `:focus-visible` shell outline; open = `aria-expanded="true"` + `aria-haspopup="dialog"`
  on the clicked segment.
- a11y: group `aria-label="Review queue: 34 to review, 2 to escalate"`; each button
  `aria-label="Open review (34)"` / `"Open escalations (2)"`.
- Phone (`?shell=phone`, 390px): segments collapse to icon + count (lucide
  `ClipboardCheck` for Review, `Hand` for Escalate), labels kept in aria-label; no
  horizontal overflow; the existing Users sheet button stays.
- Counts are live (the same status poll as the pane/phone badge) and exclude archived work (OIN-21).

## Review modal
- Desktop: centered dialog using the existing `.sm-overlay` / `.sm-dialog` pattern (as in
  ModelDialog), `width:min(1160px, 100vw - 48px)`, `max-height:85vh`,
  `border-radius:var(--radius-sheet)`, `background:var(--color-surface-1)`,
  `border:1px solid var(--color-line)`. Focus trap, Esc closes, focus returns to the
  clicked capsule segment.
- Phone: `SheetFrame` with `detents={['full']}`. List → detail push inside the sheet; the
  sheet header back arrow returns to the list.
- Header: title "Review", filter tabs `All · N` / `Review · N` / `Escalate · N` (defaults to the
  clicked segment), project native `<select>`, search input (`--radius-input`).
- Body (desktop): left queue list 340px, scrolls on its own; right is the task detail
  (TaskDetail in embedded mode) with a header row `‹ Prev  3 of 12  Next ›` and the actions
  (Mark done / Rework / reply for escalations). Mark done/Rework → next item in the filtered
  queue; counts drop at once. Empty filter → one muted line ("Nothing to review"), plus a
  "Queue done" line after the last item.
- Keys: `j`/`k` or ArrowDown/ArrowUp move in the list; `[` / `]` prev/next in the detail.
- Deep link: `/superagent?review=<filter>&task=<id>` (query, replace-navigation while moving
  between tasks, so one Back closes the modal and never goes through /board).
- No bottom Review accordion in the pane any more; the pane may keep a small "Open review" link.
