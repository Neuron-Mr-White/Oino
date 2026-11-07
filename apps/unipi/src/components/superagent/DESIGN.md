# Superagent

Operate surface extending the incumbent Unipi shell: existing Inter typography,
warm dark surfaces, shared buttons, phone SheetFrame and the conversation renderer.
No new palette, navigation framework or duplicate approval surface.

Model: one **lead per project** (not one worker per ticket). The page is the
workspace-free router chat plus a side pane that shows only what the owner acts on.

## Pane (desktop, 360px, scrolls independently; header sticky)
- Header: one quiet pill, "Auto-start on · up to 2 leads", opens the settings
  popover (master switch, max-leads stepper 1–3, Manage presets). Refresh at right.
- **Needs you** (count): approvals only through the shared `ApprovalCard`, one at a
  time with "1 of N"; blocked tickets with their question and a reply box that relays
  to that project's lead (steer, text prefixed `Re <ticket>:`). Nothing here takes
  focus. Empty: one line. Questions show 2, then "Show N more".
- **Leads**: compact card per live lead: project, mode chip (native select, confirm
  sheet before `set_mode`), preset · model · thinking · autonomy, step + tool,
  elapsed · cost, running/queued/done bar and ticket count, Open, Stop (confirmed).
- **Projects**: own Auto switch per project (`configure_project`) and Start (ready
  tickets sheet with preset picker). "Auto-start paused" when the master switch is off.
  4 rows, then "Show N more".
- **Review**: collapsed, count badge, opens the ticket (protocol has no accept op).
- Presets: native select in Start; manage sheet lists, stars the default, edits,
  deletes (not the default) and saves.

## Phone (`?shell=phone`)
Chat first. Header Users button with a Needs-you count badge (page-level status
poll, works with the sheet closed) opens a full-height sheet with tabs
"Needs you · N" (plus Review), "Leads · N", "Projects". Targets ≥44px, no horizontal
overflow at 390. Selects are native, so the OS picker is used.

## Degrading
An older host sends `workers[]`: each becomes a lead card without mode/preset
(Stop uses its ticket), `needsYou` in_review moves to Review, Projects explains that
the host has no per-project controls. Unknown telemetry is omitted, never zero.

## Motion
- Buttons/list rows/inputs/selects: no decorative catalog entry fits dense operational controls; reuse shared hover and instant keyboard focus.
- Switch: reuse SwitchField (desktop) and the phone `Switch` (phone); their existing motion.
- Review disclosure chevron: 150ms rotate, drops under reduced motion.
- Sheets/confirm: reuse SheetFrame's established motion, focus and reduced-motion handling.
- Status values: static updates without count animations.
