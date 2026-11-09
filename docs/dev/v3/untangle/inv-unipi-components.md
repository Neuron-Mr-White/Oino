# UniPi components untangling inventory

## 1. Area summary

- `shell`, `phone`, `panes`, `sidebar`: navigation, layout, tab lifecycle and device surfaces.
- `conversation`, `composer`, `chat`, `timeline`, `dock`, `draft`, `model`: chat creation, controls and rendering.
- `status`, `superagent`, `board`: long-horizon progress, agent fleet/review and task management.
- `browser`, `files`, `git`, `media`, `memory`, `skills`, `mcp`, `session`: tool and settings surfaces.
- Shared component helpers are also imported by `lib`, `screens`, and settings; host/extension contracts extend outside this area.

Scope: current working tree, including concurrent uncommitted work; line numbers can move. Confidence is in the observation, not a guarantee that a proposed change is safe. This is a prioritized, evidence-backed inventory rather than a claim that every file or every special case was exhaustively audited. The last 400 commits' component-path frequency puts `Composer.tsx` (36), `timeline.css` (29), `sidebar.css` (25), `AppShell.tsx` (24), and `Conversation.tsx` (23) near the top; this is frequency, not proof of pairwise co-change. No tests were run for this inventory.

## 2. Candidate fixes, ranked

1. **Own long-horizon domain contracts and vocabulary outside the status UI.** Consolidate mode identity/labels and wire types in a domain/protocol module, retaining view-specific wording. Removes #1–#4, #12–#14; files: `components/status/{lhMode,lhProgress}.ts`, `components/superagent/{api,format}.ts`, `lib/{chat/store,status/lh,status/modeStarter,composer/chips}.ts`, consumers, extension contract/tests. Risk: medium; generated/shared Rust–TS contract requires cross-package ownership. Any changed unknown-mode or older-host behavior **NEEDS-OWNER**.
2. **Make session creation and attachment a host-owned lifecycle.** Give clients an explicit durable-ready/attach handoff instead of reimplementing replay and timed stream retention. Addresses #5–#7, #27–#32; files: `draft/{draftSession,startSession}.ts`, `superagent/api.ts`, host/session stream protocol and Rust implementation, tests. Risk: high, touches first prompt, choices and session continuity; **NEEDS-OWNER** for changed failure semantics.
3. **Extract Superagent domain API and normalize once.** Share approval/question classification, blocker construction, compatibility decoding and subscription/polling by host. Addresses #8–#10, #15, #44–#48, #55–#56; files: `superagent/{api,needs,state,railStatus,Projects,fleet}.ts[x]`, consumers and host/extension tests. Risk: medium/high; changing badge counts, old-host support or fallbacks **NEEDS-OWNER**.
4. **Give host capabilities/cache a neutral owner and explicit contract.** Move command/model caching from draft to host domain; validate cache rows and separate persistence failure policy. Removes #11, #16–#18, #33–#35; files: `draft/{hostCache,draftSession}.ts`, `composer/Composer.tsx`, `model/HostModelPicker.tsx`, `lib/model/thinking.ts`, Cron/settings consumers and tests. Risk: medium; cache migration and offline behavior **NEEDS-OWNER**.
5. **Replace cross-surface singleton/DOM conventions with owned UI controllers.** Scope review, picker and terminal lifecycle to an explicit owner and attach via refs/cleanup. Addresses #19–#22, #36–#39; files: `superagent/{reviewUi,Review,SuperagentPage,canvasStore}.ts[x]`, `model/{target,ChatModelPicker}.tsx`, `panes/tabs/{terminalClose,TerminalTab}.tsx`, pane and shell consumers. Risk: medium/high: routing, scroll restoration and terminal termination are visible; **NEEDS-OWNER** for any changed timing or confirmation policy.
6. **Consolidate resilient storage policy only where behavior agrees.** Use typed per-feature codecs atop a common storage boundary, not one global fallback. Addresses #33–#35, #40–#43; files: draft, browser, shell, command and superagent storage consumers/tests. Risk: medium; preserve keys and quota/private-mode behavior; **NEEDS-OWNER** if errors become visible.

## 3. Full problem list

Paths below are relative to `apps/unipi/src/` unless otherwise stated.

### Hidden links

1. **[high]** `components/status/lhProgress.ts:11`, `components/status/lhMode.ts:20`, `components/superagent/api.ts:7` each enumerate the mode vocabulary independently, so a new mode requires coordinated changes with no single type owner.
2. **[high]** `components/status/lhProgress.ts:60` and `components/superagent/format.ts:7` separately map the same mode names to labels, inviting inconsistent copy.
3. **[high]** `components/status/lhProgress.ts:13` and `lib/status/lh.ts:9` bridge UI-owned progress definitions into domain parsing, while the producer lives in `extensions/unipi/long-horizon/src/lib.rs:1`; wire shape and UI type can drift.
4. **[high]** `components/status/lhMode.ts:20`, `lib/status/modeStarter.ts:13`, and `lib/composer/chips.ts:49` encode overlapping mode lists for different entry points, so adding a command requires several manually synchronized edits.
5. **[high]** `components/draft/startSession.ts:3` and `:65` replay choices in a prescribed order using the same operations as chat controls, but this orchestration is not a host-level creation contract.
6. **[high]** `components/draft/startSession.ts:7` and `:89` assume a user `message_end` implies the session file is ready to open; callers depend on persistence/event ordering not expressed in the API.
7. **[high]** `components/superagent/api.ts:181` and `:193` require the creation stream to outlive routing until Conversation attaches, a lifecycle obligation hidden behind a returned string.
8. **[high]** `components/superagent/needs.ts:5` and `components/superagent/api.ts:69` encode different approval predicates: the badge counts two fields while the renderable approval requires six, so their semantics can drift.
9. **[high]** `components/superagent/Projects.tsx:15` and `:27` fabricate blocker/action pairs also represented by `components/superagent/api.ts:31` and `apps/unipi/src/dev/mock/scenarios/superagent-leads.ts:35` (repo-relative), requiring independent vocabulary updates.
10. **[high]** `components/superagent/state.ts:128` and `components/superagent/railStatus.ts:12` poll the same host status separately, so freshness, failure and lifecycle policy are split across page and rail.
11. **[high]** `components/composer/Composer.tsx:236` populates `components/draft/hostCache.ts:39`, which `components/draft/draftSession.ts:104` and `lib/model/thinking.ts:11` consume; the composer is an implicit cache producer for unrelated entry points.

### Needless links

12. **[high]** `lib/chat/store.ts:8` imports validation and protocol types from `components/status/lhProgress.ts`, making session infrastructure depend on a presentation directory.
13. **[high]** `lib/session/store.ts:25` imports generic unknown-op detection from `components/status/lhProgress.ts`, coupling session transport behavior to long-horizon presentation.
14. **[high]** `components/command/FindBar.tsx:25` and `components/memory/MemoryBrowser.tsx:23` import a general `agoText` formatter from the long-horizon progress module, tying unrelated views to its internals.
15. **[high]** `lib/notifyInit.ts:47` dynamically imports `components/superagent/api.ts`, putting host service access under a volatile component feature instead of a domain boundary.
16. **[high]** `lib/model/thinking.ts:11` imports the shared model cache from `components/draft/hostCache.ts`, reversing the expected model-domain-to-UI dependency.
17. **[high]** `screens/Cron.tsx:46` and `ui/settings/ModelPickerField.tsx:13` use a cache named and owned for drafts, although Cron and settings change independently of draft UI.
18. **[medium]** `components/model/hostTarget.ts:13` adapts host settings to `Pick<SessionHandle, 'cmd'>` and manually dispatches four session command variants; a model settings dialog is coupled to a much broader session command language.
19. **[high]** `components/superagent/reviewUi.ts:13` locates a scroller by CSS class across the pane and phone sheet, so layout class changes silently affect review scroll restoration.
20. **[high]** `components/model/target.ts:17` stores the picker target in a module singleton while `:3` requires a separately mounted shell layer; opening a picker works only if its consumer is installed.
21. **[high]** `components/superagent/canvasStore.ts:48` and `:54` use module-global return state shared by `SuperagentPage.tsx:22` and canvas views, tying navigation lifetime to a global rather than an explicit host/page owner.
22. **[high]** `lib/panes/kinds.ts:19` imports terminal-close UI from `components/panes/tabs/terminalClose.tsx`, coupling the general tab registry to one tab's confirmation internals.

### Surplus / shallow interfaces

23. **[high]** `components/status/LhSection.tsx:47` wraps `sharedClock()` without adding behavior; direct use would remove the extra interface (check external test imports before removal).
24. **[medium]** `components/model/hostTarget.ts:20` exposes a broad generic `cmd<T>` only to reject unsupported commands at `:34`; a narrow settings port would eliminate the facade rather than expand it.
25. **[medium]** `components/draft/hostCache.ts:34` and `:48` are thin typed wrappers over the same unchecked generic storage read; merging them into an owned capability-cache interface would remove pass-through surface while retaining separate data types.
26. **[medium]** `components/superagent/api.ts:91` and `:92` define identical config and reply schemas separately; reuse is possible if request and response are intentionally the same contract, otherwise retain the distinction.

### Special cases (counted items)

27. **[high; NEEDS-OWNER]** `components/draft/startSession.ts:76` swallows Fusion setup errors and continues the first prompt, potentially running a different configuration than chosen.
28. **[high; NEEDS-OWNER]** `components/draft/startSession.ts:78` and `:79` swallow model/thinking replay errors, leaving the caller without a failure signal.
29. **[high; NEEDS-OWNER]** `components/draft/startSession.ts:82` and `:83` swallow permission/plan command errors, although they are user-selected workflow choices.
30. **[high; NEEDS-OWNER]** `components/draft/startSession.ts:89` routes after an eight-second race even without the persistence event, explicitly substituting a timeout for readiness.
31. **[high; NEEDS-OWNER]** `components/draft/startSession.ts:90` keeps a stream alive for six seconds instead of synchronizing attachment; slow transitions can outlive the assumed window.
32. **[high; NEEDS-OWNER]** `components/superagent/api.ts:193` independently uses a six-second attachment grace rather than an explicit consumer acknowledgement.
33. **[high]** `components/draft/hostCache.ts:18` accepts any nonempty array as typed model/command rows and `:20` suppresses parse/storage errors; corrupt rows can escape the cache boundary.
34. **[high; NEEDS-OWNER]** `components/draft/draftStored.ts:64` treats invalid JSON, bad storage and absent draft identically, so the user cannot distinguish lost saved choices.
35. **[high; NEEDS-OWNER]** `components/draft/draftStored.ts:83` silently leaves choices in memory when persistence fails; a later restart may lose the apparent selection.
36. **[high]** `components/superagent/reviewUi.ts:21` retries up to 30 animation frames to infer sheet/layout readiness; restoration is coupled to rendering speed rather than a mount/layout signal.
37. **[high; NEEDS-OWNER]** `components/panes/tabs/TerminalTab.tsx:504` ignores detach failure and sleeps 250 ms before reconnecting, using elapsed time to sequence two streams.
38. **[high; NEEDS-OWNER]** `components/panes/tabs/terminalClose.tsx:54` silently ignores a host `term_kill` rejection, so tab closure can hide a surviving terminal.
39. **[high; NEEDS-OWNER]** `components/panes/tabs/terminalClose.tsx:139` turns both `term_list` failure and timeout into `null` at `:152`; close logic must use the same unknown state for unrelated failures.
40. **[high]** `components/browser/BrowserTab.tsx:45` silently discards malformed or inaccessible saved tab URLs, while `:56` silently loses writes; this is a separate persistence policy from draft storage.
41. **[high]** `components/shell/Workspace.tsx:45` treats invalid or inaccessible last-session storage as no remembered session, and `:52` suppresses failure to save it.
42. **[high]** `components/command/CommandCenter.tsx:89` and `:111` silently reset or fail to persist command history; failures are not distinguishable from empty history.
43. **[high]** `components/superagent/canvasStore.ts:16` and `:19` suppress storage errors, intentionally falling back to window-only state without reporting persistence failure.
44. **[high; NEEDS-OWNER]** `components/superagent/api.ts:75` translates an old worker-shaped status and `:81` derives review from needs-you; deleting or changing this alters old-host support.
45. **[high; NEEDS-OWNER]** `components/superagent/api.ts:45` defaults absent older-host registration and blockers to registered/no blockers, concealing absence of those fields from the parsed model.
46. **[high; NEEDS-OWNER]** `components/superagent/state.ts:92` ignores history failures as optional old-host behavior and `:113` suppresses all preset-load exceptions, including non-compatibility errors.
47. **[high; NEEDS-OWNER]** `components/superagent/api.ts:120`–`:122` silently maps unknown fleet mode, role and status to plain/subagent/idle; future protocol values look like valid existing values.
48. **[high; NEEDS-OWNER]** `components/superagent/railStatus.ts:37` ignores history errors and `:44` hides errors by clearing the summary, so an unavailable badge can resemble no data.
49. **[high; NEEDS-OWNER]** `components/status/lhMode.ts:60` and `:78` fall back to progress-derived mode for older bridges; this compatibility policy is separate from topic parsing and must be retained or retired deliberately.
50. **[high]** `components/panes/tabs/TreeTab.tsx:76`–`:85` treats errors and a short timeout equally as reasons to fall through from tree to flat to paged requests, embedding host-version/performance assumptions in the view.
51. **[high; NEEDS-OWNER]** `components/panes/tabs/BgTaskTab.tsx:75` uses the snapshot tail after any read exception, conflating older extension, deleted task and transport errors.
52. **[high; NEEDS-OWNER]** `components/media/Previewer.tsx:190` silently ignores image save errors, leaving a user action with no failure feedback.
53. **[high]** `components/draft/draftSession.ts:110` uses a 1.5-second grace to stop hiding initial controls, so initial presentation depends on host latency rather than readiness alone.
54. **[high]** `components/media/PdfView.tsx:9` selects the legacy PDF.js build specifically for older Chromium; removing it requires confirming supported runtimes, not just renaming imports.
55. **[high]** `components/superagent/Projects.tsx:15` synthesizes a single `not_registered` blocker when an older/incomplete host gives no blockers; the UI maintains a special case of host domain logic.
56. **[high]** `components/superagent/fleet.ts:47` only preserves graph/swarm as fleet modes and maps all other lead modes to plain; adding a mode requires deciding whether this deliberate projection still holds.

## 4. Special-case count

**30 numbered findings (#27–#56)**, counted as grouped sites, not individual `catch` clauses or an exhaustive repository total. This includes deliberate compatibility and best-effort fallbacks, not assertions that each is a defect; behavior-changing removals need owner approval. Search for `it.skip`, `test.skip`, `describe.skip`, and `.todo(` in component TS/TSX found **0 matching lines**; alternative skip spellings and runtime conditional skips were not exhaustively ruled out. UI animation/debounce timers were not counted merely for being timers.

## 5. Comments and tests notes

- Rough estimate: **at least several dozen** comments restate straightforward names/operations; not a full manual classification. Representative: `components/model/target.ts:29` (“Closes it”) directly above `closeModelPicker`; `components/status/LhSection.tsx:46` describes the clock immediately above a pass-through; `components/draft/hostCache.ts:60` says only “Tests.” above a reset hook. Many other comments explain important ordering and compatibility and should remain.
- **152** `*.test.ts`/`*.test.tsx` files in this area; no assertion-free or incapable-of-failing test was established in the sampled tests. Do not infer a count of such tests from a grep.
- Representative implementation-coupled tests: `components/status/lhMode.test.ts:4` and `:17` directly exercise helper exports and assert internal `phase`/`live` representation; `components/model/cycleModel.test.ts:2` and `:11` directly assert helper transformations. These are useful unit tests, but representation-only refactors require edits even if UI behavior is unchanged. Rough sampled count: **at least 2 test files**, not an area-wide estimate.
- Synchronization fragility: `components/draft/DraftView.test.tsx:137` waits 30 ms; `components/shell/missingSession.test.tsx:86` waits 1,200 ms. Search found **89 lines** containing timers in component test files, not 89 bad tests; replace fixed waits only where an observable completion condition exists.
- No dead public item is claimed: a repo-wide user search including extensions is required for each candidate before deletion, including dynamic/string-based consumers.
