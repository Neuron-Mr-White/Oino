# Unipi changelog

What changed in each Unipi app release (desktop and phone), newest first.

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
