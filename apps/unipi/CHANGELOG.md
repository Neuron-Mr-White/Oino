# Unipi changelog

What changed in each Unipi app release (desktop and phone), newest first.

## Unreleased

## 1.0.4 — 2026-11-09

- Superagent canvas: a collapsible right rail and a full-screen live agent canvas, a floating capsule with a one-at-a-time workbench, and a leads-only pane.
- Phone: Superagent is the centred bottom tab, the Settings gear sits in the Chats header, and the Superagent screen uses one keyboard inset and one gutter.
- A retired lead that you reopen keeps its board tool.
- Internal cleanup (untangle) of unused parameters, duplicate wrappers and stale comments, with no behaviour change.

## 1.0.3 — 2026-11-09

- Prevent duplicate user messages after chat catch-up and keep final responses outside work folds.
- Scope subagent activity to its session, and show concurrent Superagent runs and review queues.
- Improve model selection and updater screens, and normalize tool names for providers.

## 1.0.2 — 2026-11-08

- Superagent workspace onboarding, working-lead preservation, wire-call normalization, paired remote settings routing, and wake-aware Lead row activity/spinners.
- Auto-compaction fixes for single-turn context bursts (bounded `session_recall` snippets, forward/custom-message compaction anchors, single-turn boundary materialization, reset threshold suppression, and 60 s `session_before_*` Wasm hook deadline).
- Archiving a chat leaves the shown session immediately, removes the row optimistically without racing `sessions_list`, and cleanly closes file-less empty chats instead of raising `No session found matching`.
- `/reload` in the app, like the terminal has: the chat re-reads its settings, the installed
  extension packages, skills, prompt templates, context files, `models.json` and your API keys
  from disk and the `/` list follows — no app restart. Settings › General has a Reload button for
  the host-wide half (models, keys, extension packages).
- A chat whose runtime was retired (the services cap, an idle disposal, a host restart) reopens
  by itself instead of staying blank until the app is restarted: the host now ends that chat's
  stream, so the app reconnects and wakes it, and a chat that ended without an error also counts
  as failed when you click it again, come back to the window or the host returns.
- More chats stay live at once: the host sizes its services cache to the machine (about 300 open
  files and up to 768 MiB of memory per open project, half of available RAM, at most 32) instead
  of the fixed 4 it kept before. On the owner's PC that is 26 projects rather than 4, so opening a
  chat in a sixth project no longer unloads the one you were reading.
- The `maxServices` setting no longer decides: the app host's own budget overrides it.

## 1.0.1 — 2026-11-06

- Updating the desktop app no longer stops running tasks: the new version picks up the
  host that is still working, and swaps in the new host once it is idle.
- Drafts are kept per chat, chips and edited images included, and follow you to your
  other devices. The sidebar marks a chat with a draft.
- Copying from the message box keeps chips and images when you paste it back into Unipi
  (other apps get plain text).
- When a phone or another PC asks this PC to turn on scheduled runs, web apps or the
  remote browser, the request pops up right away.
- Subagent tabs are live: the latest work stays open and a spinner shows while it runs.
- A finished turn no longer keeps saying "Subagent running".
- Compaction can't hang any more: a model that stops answering fails it after 3 minutes.
- Phone: no file list under each turn, shorter image chips that open the editor when
  tapped, and tighter tool rows. On the desktop the file summary under a turn is a proper
  native row again and no longer blinks while the turn runs.
- New chats started from a project run in that project, not in a scratch folder.
- The Git panel's inline diff no longer overlaps the file list.
- Menus and pickers opened inside a dialog show above it, and picking an option no
  longer closes the dialog.
- The sidebar's Task list header stays in view; pinned chats also stay in their project;
  subagent filters start on "In progress"; idle live chats get a quiet status ring.
- MCP: Reconnect is a real button with feedback, and HTTP servers that sent a duplicate
  Content-Type header (HTTP 415) connect again.
- About › What's new shows these notes without needing the Updater extension.

## 1.0.0 — 2026-11-05

- First Unipi Oino release: the desktop and phone app for Oino.
