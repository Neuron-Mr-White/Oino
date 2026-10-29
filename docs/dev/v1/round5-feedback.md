# Round 5 — owner feedback on alpha.4 (2026-10-29, verbatim, numbered)

Source: owner QA of `1.0.0-alpha.4` desktop (`45658e4`). The owner attached 7 screenshots
that are **not available** to agents; where an item references `[image N]`, the
surrounding text is the only description — locate the element yourself in the running app
(GUI driver screenshots, `docs/dev/v1/gui-driver.md`).

## Direction (overrides earlier "unipi-first" / TUI-parity decisions)

- D1. Dispose what we have so far for UNIPI. We do not go back to TUI: extensions that are
  primarily for the TUI are removed without argument (users can build their own).
- D2. Many things still look like the original unipi. Follow **ZCode** more for everything
  that is not harness behaviour (layout, UX, UI, animation, design), while making sure every
  harness feature bubbles up to the GUI. Drop the unipi mindset in the desktop unless there
  is a specific reason.
- D3. Oino primarily supports the GUI. Drop TUI parity. The only parity we keep is
  **harness parity** (pi 1.4.0 runtime behaviour + unipi harness features that affect agent
  behaviour). Everything else can go.
- D4. Learn from / take components directly from Zed (`github.com/zed-industries/zed`) and
  `github.com/zed-industries/awesome-gpui` — scout real implementations.

## Issues

1. Model picker must be like unipi desktop's model picker: a **modal**, not an overlay.
   The current one can't search, can't pick fusion, can't show the full provider/model
   name. Unipi's one also can't pick a custom lead or sidekick — fix that. Learn from
   Devin's picker too: the user can **pin** models to the top.
2. The input box is far too complex. Remove the "unipi" text from it. Turn the regular mode
   into a **switcher** so the user sees and switches modes up front (no need to type
   `/graph`, `/goal`). Remove the permission access from there (the input already has it).
3. Too many elements in the bottom of the input (project, memory count, context, model,
   thinking — repetitive or useless). Remove the input's footer entirely; keep the input's
   inner footer. Bring context **inside** the input like ZCode (without ZCode business
   logic).
4. Pressing `+` multiple times must not create multiple new sessions in the workspace
   (it currently does).
5. "Automation" in the sidebar has no icon.
6. The new-session icon at the top should open a **new chat page like ZCode** [image 1]:
   it can pick an existing project, or work outside of a project.
7. Treat Oino differently from unipi now: the **kanboard must not be a daemon**
   (`oino-kanboard serve`); bring it in-process from the start.
8. Kanboard UX still differs from unipi. Make cards **draggable**, animated like unipi on
   drag (non-related columns collapse while dragging).
9. The kanboard modal is very ugly, not formatted like unipi [image 2].
10. Kanboard on the right side panel should display like the **unipi phone version**; the
    full-page (PC) version should follow the current unipi board design as exactly as
    possible — today it has weird buttons and is hard to use.
11. In kanboard, the overlay [image 3] must show the **full text**; there is lots of space,
    no need to truncate.
12. **Centralize the host ("pc").** Instead of every page (sessions, boards, …) having its
    own host picker like unipi, have one host switcher visible on all pages (sidebar,
    footer or header). With no hosts set up it reads "Current PC".
13. In Automations, the cron icon does not exist either.
14. Settings is too compact [image 4]; follow ZCode's settings layout [image 5].
15. While the agent runs, **Esc twice** stops it. On the first Esc, tell the user to press
    Esc once more to stop.
16. Does the conversation use virtualization? It should (GPUI approach; learn from Zed's
    agent panel rendering).
17. The conversation doesn't have unipi's **markdown rendering colors** — copy them.
18. Keep the bottom-most footer, but remove the second-bottom-most one (`oino`, `0 mem…`)
    — same as issue 3.
19. Settings has Global / Project tabs; the Project tab must **label which project** the
    user is in.
20. Settings doesn't need an "Extensions" page — redundant with the sidebar.
21. The chat sometimes **bounces repeatedly**. Learn from how Zed renders the chat.
22. Does every setting do what it says? If not, remove it or implement it. E.g. disabling
    the footer setting did nothing visible. Do we really need these extensions?
23. With the search overlay open, scrolling scrolls both the kanboard and the search box.
24. There is no built-in files view / editor. It should be Zed-like with simple LSP and
    formatting. If it is easy to take the full component from Zed with a simple rewrite,
    take it.
25. Some text can't be read [image 6] (contrast — find low-contrast text across the app).
26. The unipi logo is wrong — render the real logo instead of a grey icon.
27. The refresh button at the top of the chat [image 7] is useless now — remove it.
