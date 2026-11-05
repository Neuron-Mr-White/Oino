# Owner fix 7 (2026-11-05): the 24-item UI round — map and evidence

Commits on `v2`: ea0c697e, 20f560bb, 4e920b9e, 6e584927, aea4b24c, 233e980c, b5e23e81.
121 files changed, +3756 / −537 across `2ca5fbe4..b5e23e81`.

## Re-verified test output (2026-11-05, this session)

```
$ cd apps/unipi && npx tsc --noEmit && echo "TSC: 0 errors"
TSC: 0 errors

$ npx vitest run
 Test Files  336 passed (336)
      Tests  3261 passed (3261)

$ cargo test -p unipi-host --lib usage_stats        # apps/unipi workspace
running 4 tests
  usage_stats::tests::parses_replies_and_header ... ok
  usage_stats::tests::unpriced_models_never_read_as_free_and_forks_dedupe ... ok
  usage_stats::tests::range_and_timezone_bound_the_days ... ok
  usage_stats::tests::reads_a_tree_and_reuses_the_cache ... ok
test result: ok. 4 passed; 0 failed

$ cargo test -p unipi-proto                          # proto usage.rs/.ts
 (1+5+2+4 tests) all ok

$ cd src-tauri && cargo test --lib zoxide            # standalone workspace
  open_local::zoxide_tests::parses_scores_best_first ... ok
test result: ok. 1 passed; 0 failed

$ npx vitest run src/lib/usage.test.ts
 Test Files  1 passed (1)   Tests  6 passed (6)
```

Headless-Chromium UI assertions (capture harness, mock host) — all pass:
fusion_chip="Fusion · Claude Opus 4.1 ◆ GLM-4.6"; fusion_selected_groups=[Selected,All models]×2;
first_row_sub="openai/gpt-5"; cron_search=1; cron_sched_icon=1; usage=[1,1,1,1];
archived_btn=1; plug=1; phone_tab_layer=1.

`cargo check -p unipi-mcp -p unipi-skill-registry` ok; both components rebuilt into `extensions/dist`.
Screenshots (capture harness, mock host, headless Chromium 1440×900 and 390×844):
`docs/dev/v2/design/shots/owner-fix-7/`.

| # | Item | Where | Evidence |
|---|---|---|---|
| 1 | Model rows show the provider/id path; Fusion picks are the right pair | `components/model/ModelList.tsx`, `FusionPair.tsx`, `suggested.ts modelPathLabel` | ChatModelPicker tests; `d-fusion-picker.png`, `d-model-search.png` (row sub `openai/gpt-5`) |
| 1b | Selected model pinned on top after search+select | `ModelList.tsx` "Selected" group, FusionPair `Selected` per column | ChatModelPicker test "ranks …"; `d-fusion-picker.png` |
| 1c | Composer chip shows the Fusion pair | `ModelChip.tsx`, `lib/model/fusion.ts activeSessionFusion` | ModelControls test; `d-fusion-chat.png` ("Fusion · Claude Opus 4.1 ◆ GLM-4.6") |
| 1d | Chat wiped after a Fusion switch / compaction | `lib/session/reducer.ts` (branchOf bridge, catchUp / reloadTail), `store.ts` | reducer.test "bridges across unannounced intermediate parents" |
| 2 | User-only / agent-only / both; compaction shown to both, history kept | `lib/timeline/eventKinds.ts eventAudience`, `EventCard.tsx`, `Compaction.tsx`, `turns.ts` | eventKinds.test, EventCard goldens |
| 3 | Long menus stay clickable | `board/AnchoredMenu.tsx` (re-measure on resize/mutation, max-height), Select flip | AnchoredMenu.test |
| 4 | Phone grouping / formatting | `phone/ChatsList.tsx`, `screens/Chat.tsx` Group menu | `p-chats.png` ("oino · 24") |
| 5 | Short phone model container | `model.css` sheet max-height, content detent | — |
| 6 | Full-screen panes with ← on phone; missing panes added | `panes/tabs/SessionTabLayer.tsx`, `chat/BrowserSide.tsx`, `PhoneChatHeader.tsx` | tabs.test, PanesMenu.test; `p-tree-pane.png` |
| 7 | Cron search + pagination | `screens/Cron.tsx filterJobs/pageOf`, `lib/paging.ts` | Cron.test (200 jobs); `d-cron.png` |
| 8 | Scheduler settings as a top icon | `Cron.tsx` gear → sheet | Cron.test; `d-cron.png` |
| 9 | Files padding, Download (chunked + phone notification), Open in explorer / detected editors | `files/FileTree.tsx`, `PhoneFiles.tsx`, `lib/filesTransfer.ts downloadWithNotify`, `chat/FileRefMenu.tsx`, `EditorArea.tsx` | FilesTab.test |
| 10 | Smaller phone New chat | `draft/draft.css` | `p-draft.png` |
| 11 | `.env` and other unknown text previews | `lib/media/previewer.ts classifyBytes` | previewer.test |
| 12 | Fun "Working…" words | `lib/status/workingLine.ts CRAB_WORDS`, `dock/WorkingLine.tsx` | BottomDock.test |
| 12b | Background (subagents running) line animates | `workingLine.css` spinning ring, shimmer | BottomDock.test |
| 13 | Usage = ZCode (lifetime strip, 52-week heatmap D/W/C, model trend, donut) | `settings/UsagePage.tsx`, `lib/usage.ts`, host `usage_stats.rs`, proto `usage.rs/.ts` | usage.test, Rust usage_stats; `d-usage.png` |
| 14 | Skills: wide manager, all skills, project context menu | `skills/SkillsManager.tsx`, skill-registry `skills.catalog/save`, `ProjectGroup.tsx` | SkillsManager.test, CapabilityPages test |
| 15 | Skills hub (redundant) removed | `lib/settings/manage.ts`, `SkillsPage.tsx` | manage.test |
| 16 | Skill editor (unusable) removed | same | CapabilityPages test |
| 17 | Dream has a real purpose | `memory/DreamPanel.tsx` (status, run/stop, runs, report, approve/reject, settings) | McpServers.test (parseDreamStatus) |
| 18 | MCP settings fixed one by one | `mcp/McpServers.tsx`, mcp `mcp.servers / mcp.server-action`, no overlay bleed (`ui/settings/command.tsx`) | CapabilityPages MCP tests |
| 19 | Settings rows without description aligned | `ui/settings/fields.tsx data-bare`, css | form snapshots |
| 20 | Sidebar property visibility (compact icons, no truncation) | `sidebar/SessionRow.tsx`, `sidebar.css` | sidebar tests |
| 21 | Archived = fixed button + wide searchable modal | `sidebar/ArchivedModal.tsx`, `shell/Sidebar.tsx` | archivedList.test; `d-archived-modal.png` |
| 22 | Quick navigate (z / zoxide) with confirm | `draft/QuickNavigate.tsx`, `src-tauri open_local.rs local_zoxide` | Rust zoxide test; `zoxide query -l -s -- oino` → `/home/oi/Projects/Personal/llm/oino` |
| 23 | Browse folder overlap (phone) | `draft/draft.css` (no flex-shrink, sheet scroll) | — |
| 24 | TUI-era slash commands out of `/`; plug button opens their surfaces | `lib/chat/commands.ts GUI_HIDDEN_COMMANDS`, `composer/PlugMenu.tsx` | commands.test; `d-draft-plug.png` |

Not yet done: a check in the installed app on this PC / on a phone (needs a new build with the
updated host and the rebuilt extensions).
