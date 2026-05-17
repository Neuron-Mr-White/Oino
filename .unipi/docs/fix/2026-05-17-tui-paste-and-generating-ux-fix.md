---
title: "TUI Paste and Generating UX — Quick Fix"
type: quick-fix
date: 2026-05-17
---

# TUI Paste and Generating UX — Quick Fix

## Bug
Pasting multi-line text into the TUI composer could treat pasted Enter/newline characters like interactive Enter key presses, submitting the prompt before the paste finished. While a prompt was running, the UI also did not clearly communicate that the assistant was generating.

## Root Cause
The terminal was not enabling bracketed paste mode and the main event loop only accepted `Event::Key`, so pasted text arrived as a stream of key presses. A newline in that stream was indistinguishable from the user pressing Enter to submit. The busy state only changed input availability/status text and was too subtle in the transcript/composer.

## Fix
- Enabled bracketed paste on TUI entry and disabled it on exit.
- Handled `Event::Paste(text)` separately from key events.
- Added `ComposerState::insert_text` to insert pasted text atomically, normalizing CRLF/CR newlines to `\n` without submitting.
- Added `TuiState::handle_paste` so pasted multi-line content lands in the composer and still updates command-suggestion state.
- Updated busy wording to `Generating…` and made it visible in the transcript title, composer title, and footer while input is paused.

### Files Modified
- `crates/oino-app/src/main.rs` — enables bracketed paste and routes `Event::Paste` to the TUI state.
- `crates/oino-tui/src/composer.rs` — adds atomic pasted-text insertion with newline normalization.
- `crates/oino-tui/src/app.rs` — handles paste without submit and updates busy status wording.
- `crates/oino-tui/src/render.rs` — renders visible `Generating…` indicators in the TUI.
- `README.md` — documents multi-line paste and generating indicators.

## Verification
- `cargo test -p oino-tui pasted_newlines -- --nocapture`
- `cargo test -p oino-tui insert_text_preserves -- --nocapture`
- `cargo test -p oino-tui render_working_state_shows_generating_indicator -- --nocapture`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

## Notes
Bracketed paste makes terminals send the whole pasted payload as a single `Event::Paste`, so Enter remains submit for interactive typing while pasted newlines are preserved as composer text.
