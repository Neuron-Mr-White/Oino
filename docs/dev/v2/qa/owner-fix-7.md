# Owner fix 7 (2026-11-05): the 24-item UI round — map and evidence

Commits on `v2`: ea0c697e, 20f560bb, 4e920b9e, 6e584927, aea4b24c (+ this doc).
Checks: `npx tsc --noEmit` clean; `npx vitest run` 336 files / 3261 tests pass;
`cargo test -p unipi-host --lib usage_stats` 4 pass; `cargo test --lib zoxide` (src-tauri) 1 pass;
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
