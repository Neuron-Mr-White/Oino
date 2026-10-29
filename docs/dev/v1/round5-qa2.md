# Owner QA pass 2 on alpha.5 (2026-10-30, verbatim, numbered)

Source: owner QA of `1.0.0-alpha.5`. The owner's screenshots are **not available** to
agents; where an item references `[image N]`, locate the element in the running app
(GUI driver screenshots / `mise run desktop` on a private Xvfb) and identify it from the
text description.

## Standing directive (very important)

- **If pi full parity is done, we do not touch `oino-core` anymore.** Recorded in the
  charter (Settled decisions). Core changes only for P0 defects or owner-approved spec
  additions. QA and feature work happens in clients, extensions and tooling.

## Issues

1. The session context menu needs an **"archived" status** (unipi has it). Without it,
   showing every chat takes too much space. After archiving exists, there must also be a
   place to **reopen archived chats**.
2. **Remove the answer extension.** It was built for the TUI and now breaks the GUI — it
   appears at the top for no reason. (unipi `utility` `/answer` surface.)
3. Model picker (single tab):
   a. The selected-model label is in caps; it should read like `Now ollamacloud/deepseek-v4.1-flash`.
   b. The tab strip is too long to fit both "single" and "fusion" text.
   c. Cancel / Apply spacing is wrong: buttons belong in the right corner with no extra
      left/right padding, and a smaller gap between them.
4. The picker modal should be **larger** (big screens) so full model names fit. In fusion,
   lead and sidekick are squeezed into two columns and long names are unreadable. Fusion
   also needs the **same search box as the single picker**, above both columns.
5. Some button is not shown in full (truncated) — find it in the picker/settings surfaces.
6. **Consistent padding and title font on every page.** Spotted: Automations page has
   larger padding than the others; Files page has less padding and a different (bold)
   title font.
7. Files page: a **selected file row** turns a color too close to the background — hard
   to see `[image 1]`.
8. The mini **context pie chart** at ~0% is invisible — its color can't be seen `[image 2]`.
9. The input does not support **drag-over of files/images** and has no **chips** like the
   unipi input. Support everything unipi's input supports. After pasting, an "image from
   clipboard" chip appears but is styled badly — indistinguishable from the background
   `[image 3]`.
10. **No emoji.** Use **lucide icons** for brain, skills, ctx and the up/down arrows, and
    on hover show plain-English tooltips. Keep the convention uniform.
11. **Image editing on attached images** (arrow, circle, pencil, crop) is missing.
12. The **git status panel at the top right shifts the whole chat left** — it should be an
    overlay, not push content.
13. Tool-call rows, expanded `[image 5]`, should render like unipi `[image 6]`: no extra
    padding, no repeating the command — just `bash` (more correct).
14. Per-message action buttons: like ZCode `[image 7]`, they should apply **only to the
    last message**, sit at the **bottom left**, not top right on every message.
15. Input attachment chip: removing the chip does **not** remove the image — it stays
    forever, and the chip above the input can't be removed. It should be a **preview**
    like unipi, not a chip.
16. **Padding between the last message and the input is too small** — "Done in 1m34s" and
    "Image #1" sit too close `[image 8]`.
17. The top-right button should be a **terminal icon like ZCode**, not the bottom-panel
    icon `[image 9]`.
18. The **graph / goal / … entrances** should display like ZCode `[image 10]`: git on top,
    our status at the bottom.
19. **Freeze bug**: after clicking between different sessions several times, letting it
    stall for a while, then clicking a session again, the app **freezes (bricks)** until
    restarted. Repro + root-cause this — likely the worst defect here.
20. `[image 11]` — a row of buttons labelled "Todo", "In progress", … : unclear what they
    do. Identify them in the app (likely board lane shortcuts), then either explain in the
    UI or remove/redesign per the unipi board design.

## Open decision (blocks planning)

The owner asks whether to keep GPUI at all, versus taking the unipi app "casing"
(SolidJS + Tauri 2, `archived/unipi/unipi-app/apps/mobile`, one codebase for desktop and
phone) and rewriting its data layer to oino's protocol (oino extensions instead of unipi
extensions). Analysis in the round-6 discussion; owner decides before round 6 starts.
