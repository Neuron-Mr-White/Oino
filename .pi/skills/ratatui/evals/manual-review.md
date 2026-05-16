# Ratatui Skill Manual Review

The Skill Creator `eval-viewer/generate_review.py` script was not found in this Pi environment, so this file is the manual review artifact for the Ratatui skill.

Use with `.pi/skills/ratatui/evals/evals.json`.

## How to review

For each eval:
1. Run the prompt with the `.pi/skills/ratatui/` skill available.
2. Save output or patch under `ratatui-eval-workspace/iteration-1/<eval-id>/with_skill/`.
3. Optionally run a baseline without reading the skill and save under `without_skill/`.
4. Check each assertion from `evals.json` as pass/fail.
5. Add qualitative notes below.

## Review table

| Eval | What to inspect | Human notes |
| --- | --- | --- |
| live-kafka-browser | Does it use inspector + streaming + table + input + document-viewer patterns? Does it specify follow mode, bounded draining, record details, search/autocomplete, and tests? | |
| sql-grid-horizontal-scroll | Does it avoid naive `Table` only? Does it include frozen columns, x/y offsets, row/cell modes, Unicode widths, copy modes, and tests? | |
| streaming-agent-chat | Does it separate transcript/composer/backend/scroll/popup state? Does it cover paste, frame coalescing, bubbles, code blocks, tool lifecycle, external editor handoff? | |
| network-telemetry-dashboard | Does it use snapshots, freeze mode, configurable columns, chart/map fallbacks, runtime info bar, breakpoints, and tests? | |
| markdown-schema-viewer | Does it use async parsing, document ids, section heights, image cache, stale event filtering, overlays, and semantic themes? | |
| fixed-grid-game | Does it separate game rules from UI, compute board geometry, map mouse to cells, layer rendering, and test overlays/cursor? | |
| ratatui-audit | Does it audit terminal lifecycle, focus/modes, resize, tables, async/render separation, theming, and tests with priorities? | |

## Pass/fail rubric

A good answer should:
- Name the reference files it used.
- Propose concrete Rust modules/types/functions, not just UX wishes.
- Include terminal safety, focus state, event/action flow, responsive layout, and tests.
- Pull in source-derived patterns from the studied apps.
- Avoid blocking work in render and avoid stale static key hints.

A weak answer usually:
- Only says “use ratatui widgets”.
- Omits terminal teardown or panic cleanup.
- Uses one giant `ui.rs` for everything.
- Ignores narrow terminals.
- Uses byte length for text/table widths.
- Does streaming/network work inside render.
- Leaves modals and focused panes competing for the same key.
