# GUI vs TUI: commands, settings and hints

Owner request (2026-11-03): commands built for the terminal UI (for example `/answer`) do not
belong in the Unipi app. This page lists every slash command, setting and hint text the app can
show, says which client it is for, and records what was done.

Oino has two clients:

- **GUI**: the Unipi app (desktop and phone).
- **TUI**: the terminal UI and CLI (`crates/oino-tui`, `crates/oino`).

Nothing was deleted. Terminal-only items still work in the TUI. Each client filters by metadata:

| Where | Metadata | Who reads it |
|---|---|---|
| Settings field | `surfaces: ["tui"]` on the field (`oino_ui::settings::Field.surfaces`, `Field::only_on`; extension kit `unipi_kit::settings::Field::only_on`) | App: `appNamespaces` / `appHidden` (`apps/unipi/src/ui/settings/types.ts`) |
| Extension command | `"ui": {"commands": {"<name>": {"surfaces": ["tui"]}}}` in `extension.json`. Core copies it onto the `get_commands` row (`SlashCommandInfo.surfaces`). | App: `shownInMenu` (`lib/chat/commands.ts`) for the `/` menu and the palette's slash rows. TUI: its `/` menu skips rows whose list has no `tui`. |
| App built-in | `surfaces` on the `PI_BUILTINS` row (`lib/chat/commands.ts`) | App `/` menu |
| Widget, status, header/footer, panel ([ask-user], 2026-11-04) | `"surfaces": ["tui"]` on the `setWidget` / `setStatus` / `setHeader` / `setFooter` / `openPanel` request (SDK: `ui::Panel::only_on(&[Surface::Tui])`). Core copies it onto the registry entry (`oino_ui::slot::UiEntry.surfaces`). | App: the registry's slot hosts skip entries not meant for `gui` (`entryShownInApp`, `lib/surfaces.ts`; extension builds from before the field are matched by entry id). TUI: `UiState::apply` skips producers without `tui`. |
| Notice (one-shot effect: `notify`, …) | `"surfaces"` on the fired request | App: `applyEffect` (`ui/slots/effects.ts`) drops it. TUI: `UiState::apply` drops it. |

If `surfaces` is absent, every client shows the item. When you type a hidden command in the
app, it still runs. It is only left out of the suggestions, so typing it never sends text to the
model.

Surface key: **GUI**, **TUI**, **both**, **dead** (no use in either client).

## 1. Built-in slash commands

The app supplies these itself (`PI_BUILTINS`). The TUI's own list is `oino_view::commands::BUILTINS`.

| Command | Surface | GUI path | Action taken |
|---|---|---|---|
| `/model` | both | Model dialog | kept |
| `/thinking` | both | thinking menu | kept |
| `/compact` | both | `compact` RPC | kept |
| `/new` | both | new-chat draft | kept |
| `/resume` | both | command center › Sessions | kept |
| `/fork` | both | chat ⋯ Fork | kept |
| `/tree` | both | Tree tab | kept |
| `/name` | both | Rename dialog | kept |
| `/copy` | both | copies the last reply | kept |
| `/session` | both | Session modal › Info | kept |
| `/settings` | both | Settings | kept |
| `/scoped-models` | both | Model dialog, "Cycle set" open | kept |
| `/export` | both | chat ⋯ Export | kept |
| `/import` | both | Import session dialog | kept |
| `/clone` | both | chat ⋯ Duplicate | kept |
| `/changelog` | both | About › What's new | kept |
| `/hotkeys` | both | the Ctrl+/ shortcut list | kept |
| `/login`, `/logout` | both | Settings › Providers | kept |
| `/bug` (app only) | GUI | About › Report a bug | kept |
| `/quit` | both | quits the desktop app | kept |
| `/trust` | TUI | none (the host decides trust when it opens a folder) | **hidden in the app** (`surfaces: ["tui"]`). Typed, a notice explains it. |
| `/share` (app only) | dead | none (there is no share service) | **hidden in the app** (`surfaces: []`). Typed, it points to Export. |
| `/reload` (TUI only) | TUI | Settings › Extensions › Restart agent host | not in the app list (no reload RPC) |
| `/extensions` (TUI only) | TUI | Settings › Extensions | not in the app list |

## 2. Extension commands

These are the listed commands, by extension. Hidden runner commands (`hidden: true`) are never
listed on any client. They are `ask-user-new-session`, `fusion-preset`, `kanboard-open`, the
long-horizon runners (`goal-*`, `lh-*`, `ralph-start`), `mcp-settings`, `mcp-add`, `mcp-sync`,
`mcp-reload`, `notify-test`, `notify-set-*`, `skills-editor`, `changelog` (updater),
`update-check`, `rename-now`, `web-cache-clear`, `permission-clear-rules`, `plan-off`,
`plan-enter` and `plan-exit`. The app calls them from settings actions.

| Extension | Command | Surface | GUI path / note | Action taken |
|---|---|---|---|---|
| utility | `/answer` | **TUI** | The hint widget becomes the **Answer N questions** pill and dialog, on the desktop as well as the phone (`AnswerQuestions`, `WidgetStack`) | **hidden in the app** (manifest `ui.commands.answer.surfaces = ["tui"]`). Its `answer.method` and `answer.port` settings are hidden too. |
| utility | `/continue`, `/retry`, `/cleanup`, `/doctor`, `/summarize`, `/move` | both | the reply lands in the chat | kept |
| btw | `/btw` | both | BTW panel | kept |
| info-screen | `/info` | both | Session modal › Info | kept |
| background-tasks | `/bg`, `/bg-tasks` | both | Session modal › Running | kept |
| watchdog | `/bg-detach` | both | | kept |
| browser | `/browser` | both | Browser pane | kept |
| compactor | `/compact-vcc`, `/compact-by-llm`, `/session-recall`, `/compact-stats`, `/compact-doctor`, `/compact-help` | both | | kept |
| cron | `/cron` | both | Cron page | kept |
| dream | `/dream` | both | | kept. The "↓ → Dream" notice text was reworded. |
| fusion | `/fusion`, `/fusion-stats` | both | Model dialog | kept |
| kanboard | `/kanboard`, `/kanboard-add`, `/kanboard-do`, `/kanboard-autowork` | both | Board | kept |
| long-horizon | `/goal`, `/ralph`, `/swarm`, `/graph`, `/regular`, `/visualize-progress` | both | mode chip, status-pane graph | kept |
| mcp | `/mcp-status` | both | Settings › MCP | kept |
| memory | `/memory`, `/memory-process`, `/memory-consolidate`, `/memory-search`, `/global-memory-search`, `/memory-forget`, `/global-memory-list` | both | Memory browser, chat ⋯ Save to memory | kept |
| notify | `/notify-event` | both | Settings › Notifications › Events | kept. The description said "without the TUI" and is now neutral. |
| skill-registry | `/skills` | both | Settings › Skills | kept |
| subagents | `/subagents`, `/agents` | both | Session modal › Running, Agent profiles | kept. The description said "also ↓ from an empty input" (a terminal key) and that part was removed. |
| updater | `/readme` | both | | kept |
| workflow | `/permission`, `/plan` | both | mode chips | kept |

## 3. Settings

### Core (`oino.*` namespaces)

| Namespace | Field | Surface | Action taken |
|---|---|---|---|
| `oino.model` | all fields (model, defaultModel, thinking level, scoped models, per-model levels, budgets) | both | kept |
| `oino.compaction`, `oino.branch-summary`, `oino.retry` | all fields | both | kept |
| `oino.agent` | steeringMode, followUpMode, defaultProjectTrust, enableSkillCommands, sessionDir | both | kept |
| `oino.agent` | `quietStartup` | TUI (hides the terminal startup banner) | `surfaces: ["tui"]`, hidden in the app. The old key fallback is kept for hosts that predate `surfaces`. |
| `oino.transport`, `oino.images`, `oino.shell` | all fields | both | kept |
| `oino.tui` | `theme` | TUI (the app has its own Appearance) | `surfaces: ["tui"]` |
| `oino.tui` | `hideThinkingBlock` | TUI (the app folds thinking per row) | `surfaces: ["tui"]` |
| `oino.tui` | `doubleEscapeAction` | TUI (a terminal key) | `surfaces: ["tui"]` |
| `oino.tui` | `treeFilterMode` | TUI (the terminal tree view) | `surfaces: ["tui"]` |
| `oino.tui` | `tuiMode` | TUI | `surfaces: ["tui"]`. The whole **Terminal UI** namespace no longer shows in the app. |
| `oino.tui` | `autocompleteMaxVisible` (read by `oino-tui` render/chrome, not in the schema) | TUI | not in the app schema, so nothing to hide |
| `oino.ui` | `density` | both (it drives extension renderer density in both) | kept |
| `oino.serve` | bind, tls, allowRemote | both (host listener) | kept |

### Extension settings

| Extension | Field | Surface | Action taken |
|---|---|---|---|
| ask-user | `escape` ("Esc"), `digitAdvance` ("Number keys move on"), `helpLine` ("Not ready" line) | TUI (the app's question card never reads them) | `only_on(&[Surface::Tui])` |
| ask-user | enabled, notifyOnAsk, maxQuestions, other | both | kept |
| utility | `answer.method`, `answer.port` | TUI (`/answer` modes, ssh -L web form) | `only_on(&[Surface::Tui])` |
| utility | `answer.hint` | TUI (the hint names `/answer`; the app no longer shows it, see §5) | `only_on(&[Surface::Tui])` ([ask-user]) |
| utility | rename.*, render.style, attachments.*, debug, image generate/edit/recognize, keys | both | kept |
| info-screen | `bootMode`, `bootTimeoutMs` (the Unicrab splash) | TUI (the app drops the splash header) | `only_on(&[Surface::Tui])` |
| info-screen | groups-page, groupOrder | both (Session modal › Info) | kept |
| notify | `native.suppressWhenFocused` ("while the terminal has focus") | TUI | `only_on(&[Surface::Tui])` |
| notify | every other field (platforms, recap, silence, renotify, events, gotify, telegram, ntfy) | both | kept |
| background-tasks, compactor, cron, dream, fusion, kanboard, long-horizon, mcp, memory, skill-registry, subagents, updater, watchdog, web-api, workflow, core (jev) | every field | both | kept |

### App settings (`apps/unipi/src/settings/schema.ts`)

All of these are GUI-only by definition: appearance, input, shortcuts, terminal tab (xterm),
editor, board and notifications. The TUI never sees them, and nothing changed.

## 4. Hints, tips and notices

| Text (source) | Surface | Action taken |
|---|---|---|
| "N questions in the reply — /answer …" widget (utility `unipi-answer-hint`) | TUI | [ask-user] (owner 2026-11-04: "didn't we already remove it from the GUI?"): the widget is `surfaces: ["tui"]` and the app's **Answer N questions** pill + full-page sheet are removed. Questions in a reply are answered in the composer; `ask_user` questions are inline timeline cards (§5). |
| "Open the Subagents tab of the work tray (also ↓ from an empty input)" (subagents) | TUI key | reworded |
| "Toggle a notify event without the TUI" (notify) | TUI wording | reworded |
| "Watch it in the work tray (↓ → Dream)." (dream) | TUI key | reworded to "(Dream tab)" |
| "No data yet / press r to refresh" (info-screen page) | TUI key | reworded to "nothing recorded for this page yet" |
| "This build cannot read the clipboard — answer in the terminal." (app clipboard-image dialog) | wrong for the GUI | reworded: "cancel, then attach the image in the composer" |
| Work-tray `KeyHints` legends (bg tasks, subagents, dream: "↑↓ navigate · esc close") | TUI keys | already removed in the app (`sanitizeTrayBody`). Hints that carry an action stay as buttons. |
| `KeyHints` on the phone | TUI keys | already hidden unless they carry an action (`ui/nodes/KeyHints.tsx`) |
| Working message "Subagent running · Ctrl+B to run in background" | desktop GUI and TUI | kept. The desktop app registers extension shortcuts (`setExtensionShortcuts`), so Ctrl+B works there. The app shows the key as a quiet tail chip. |
| Extension shortcuts (ctrl+alt+c, shift+down, ctrl+b, alt+p, alt+m) | desktop GUI and TUI | kept |
| Info-screen splash hint ("/info dashboard · alt+s shortcuts · any key close") | TUI (splash) | the app already drops the splash header |
| Compactor card "ctrl+o" expand hint (renderer) | TUI | the app draws compaction with its own event card and never shows this renderer |
| "press Enter to run it" (ask-user handoff notice) | both (Enter sends in the composer too) | kept |

## 5. Widgets, panels, notices and hints from extensions ([ask-user], 2026-11-04)

Owner (2026-11-04): on opening a chat the app jumped to an "Answer 1 question" page; "not only
commands have it". gui-cleanup filtered commands and settings; this pass audits every
extension's retained UI output — `setWidget`, `setStatus`, `setHeader`/`setFooter`,
`openPanel` (`editorReplace`, `overlay`, `sidePane`), `setWorkingMessage` and `notify` — and
marks the terminal-only ones with `surfaces: ["tui"]` (§ top table).

### Widgets (`setWidget`)

| Extension | Widget key | What it shows | Surface | Action taken |
|---|---|---|---|---|
| utility | `unipi-answer-hint` | "N questions in the reply — /answer …" | TUI | `surfaces: ["tui"]`; the app's Answer pill/sheet removed |
| utility | `unipi-attachments` | the terminal editor's attachment tokens ("attached to your next message"; fed by `oino_editor` events the app never sends) | TUI | `surfaces: ["tui"]` (the app's composer has its own chips) |
| subagents | `subagents-foreground` | bordered live cards of runs watched with the tray's `f` key | TUI | `surfaces: ["tui"]` (the app shows live runs in the timeline's subagent card) |
| core | `work-tray-strip` (`belowEditor`) | "Subagents 1 running · ↓ open" | both | kept: the app turns it into its "N running · Open" button (`trayButtonText`) and drops the "↓ open" key |
| fusion | `fusion-sidekick-wake` | spinner + "sidekick waking · N tools" | both | kept |

### Panels (`openPanel`)

| Extension | Panel | Placement | Surface | Action taken |
|---|---|---|---|---|
| core | `unipi.core/tray` (work tray, `↓` / `/tray` in the terminal) | `editorReplace` | TUI | `only_on(&[Surface::Tui])`: it can no longer take the app's composer when the terminal opens it on a shared session (the app has Session › Running) |
| btw | `unipi.btw/panel` (`/btw`, keys ctrl+c / ↑↓) | `editorReplace` | TUI | `only_on(&[Surface::Tui])` (the app routes `/btw` to its Side chat tab) |
| info-screen | `/info` dashboard | `overlay` | both | kept (Session modal › Info) |
| mcp, memory, skill-registry, updater | their command panels | `overlay` | both | kept (opened by a command the user ran) |
| utility | `/answer` form | `overlay` | TUI command | kept: only `/answer` typed by hand opens it (the command is already hidden in the app) |
| kanboard, long-horizon | board / graph | `sidePane` | both | kept |

### Header, status, working message

| Extension | Output | Surface | Action taken |
|---|---|---|---|
| info-screen | `setHeader` `splash` (ASCII Unicrab, "/info dashboard · alt+s shortcuts · any key close") | TUI | `surfaces: ["tui"]` (the app also matched it by id before) |
| long-horizon (`lh-mode`, `lh-goal`, `lh-progress`), memory (`unipi-memory`), skill-registry (`skills`) | `setStatus` lines | both | kept (the app's status strip) |
| subagents | `setWorkingMessage` "Subagent running · Ctrl+B to run in background" | desktop GUI + TUI | kept on the desktop (Ctrl+B is a registered app shortcut, shown as a quiet chip); the phone drops the key hint (`SegmentFold`) |

### Notices (`notify`) and hint text

| Text (source) | Surface | Action taken |
|---|---|---|
| "Agent is busy. Press ESC to interrupt, then try again." (utility) | both | kept: Esc stops the run on the desktop app too |
| "Automatic handoff failed. The command was placed in the editor; press Enter to run it." (ask-user) | both | kept |
| "usage: /btw …", "Usage: /memory-process …", "Usage: /notify-event …" | both | kept (answers to a command the user typed) |
| "/answer needs an interactive frontend" (utility) | TUI | only reachable by typing the hidden `/answer` |
| Work-tray / subagents / dream `KeyHints` legends | TUI keys | already removed in the app (`sanitizeTrayBody`, `ui/nodes/KeyHints.tsx`) |

No notice is terminal-only today, so none carries `surfaces` yet; the filter is in place on both
clients for the next one.

### ask_user in the app

`ask_user` (unipi.ask-user) questions are a first-class card in the chat timeline at the point
they were asked (`AskUserCard`): never inside the "Worked for …" fold, never on a separate page,
nothing opens or scrolls by itself. Options are buttons (checkboxes for multi-select), with an
"Other" field, Skip and Submit; once answered the card is one "You answered: …" row in place. The
bottom dock shows only a small "question waiting · Answer ↑" chip that scrolls to the card. On
the phone the same card is inline; a full sheet opens only from its expand button. The
extension's terminal knobs (`escape`, `digitAdvance`, `helpLine`) stay TUI-only (§3).

## Verification

- `apps/unipi/src/lib/surfaces.test.ts`: settings filtering (field, page drill-in, namespace).
- `apps/unipi/src/lib/chat/commands.test.ts`: menu filtering, and typed commands still known.
- `apps/unipi/src/ui/slots/slots.test.tsx`: terminal-only widgets (and the old `/answer` hint by id) never render, desktop and phone; no Answer pill.
- `apps/unipi/src/components/timeline/cards/AskUserCard.test.tsx`, `apps/unipi/src/lib/session/turns.ask.test.ts`: the inline ask_user card and the dock chip.
- Rust ([ask-user]): `oino-core` `producers_map_to_entries` (surfaces carried), `oino-view` `retained_state_and_effects` (a gui-only widget is skipped), SDK `open_request_carries_the_v2_slot_fields` (`only_on`), utility harness (the hint is `surfaces: ["tui"]`), ask-user harness (`toolCallId` on the form).
- Rust: `oino-ui` `surfaces_filter_per_client`, `oino-extension-api` `command_surfaces` and
  `oino-core` `terminal_only_core_fields_are_marked_tui`.
- Real-app screenshots are in `docs/dev/v2/design/shots/owner-fix/gui-cleanup-*.png`.
