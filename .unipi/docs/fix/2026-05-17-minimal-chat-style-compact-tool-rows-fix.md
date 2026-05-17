---
title: "Minimal Chat Style Compact Tool Rows — Quick Fix"
type: quick-fix
date: 2026-05-17
---

# Minimal Chat Style Compact Tool Rows — Quick Fix

## Bug
The `minimal`/jcode-style transcript was still too verbose for tool results. Successful `read` results in collapsed mode rendered as two rows:

```text
  ✓ Read /path · 79 lines
    [collapsed]
```

Consecutive reads also had blank spacing between them. In the latest session log, the fourth branch message was a tool-call-only assistant message for a `read` that failed on a directory; collapsed minimal rendering did not expose the useful error inline.

## Root Cause
The new minimal renderer reused the generic transcript spacing helper, which always inserts a blank line between message blocks. It also reused generic tool-output display behavior, so collapsed tool output appeared as a second `[collapsed]` row even in the compact jcode-style view.

## Fix
- Added a minimal-specific append helper that does not insert spacing before tool/result rows or metadata rows.
- Changed minimal tool result rendering to a single compact summary row for successful tools.
- Added concise inline error summaries for failed minimal tool rows, so a failed `read` directory call now renders its useful error in the row.
- Kept agentic/chat output behavior intact, except agentic collapsed errors now show the truncated error instead of an unhelpful `[collapsed]` placeholder.

### Files Modified
- `crates/oino-tui/src/transcript.rs` — compact minimal tool rows, minimal spacing rules, inline tool error summaries, regression tests.

## Verification
- `cargo test -p oino-tui transcript -- --nocapture`
- `cargo test`

## Notes
This follows jcode's approach of avoiding inserted blank rows before `tool`/`meta` messages and keeping normal tool outputs as compact status rows.
