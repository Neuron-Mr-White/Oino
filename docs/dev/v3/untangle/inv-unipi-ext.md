# UniPi extensions — change-together inventory

## 1. Area summary

- 23 extension crates: ask-user, background-tasks, browser, btw, compactor, core, cron, dream, file-summary, fusion, info-screen, kanboard, long-horizon, mcp, memory, notify, skill-registry, subagents, updater, utility, watchdog, web-api, workflow.
- Orchestration: long-horizon/kanboard/workflow coordinate goals, boards and permission/plan state; subagents/fusion/background-tasks run work; core assembles the work tray.
- Integration: browser/cron use host services; mcp, memory, web-api, notify and updater cross process, network or storage boundaries; dream coordinates memory and cron.
- Presentation: info-screen, utility, ask-user, file-summary, compactor and the app consume session entries, bus topics and panels; `extensions/sdk/unipi-kit` supplies shared Rust contracts.
- Scope: working-tree static inventory, not a complete audit of ~92k Rust source lines; confidence concerns the observed coupling, not proof of a production defect; no dead-public-item claim is made without a complete consumer analysis.

Read-only history sample (`git log --format= --name-only -n 400 -- extensions/unipi`): long-horizon/lib 29 appearances, subagents/lib 27, long-horizon/graph 13, workflow/lib and memory/lib 12 each. Counts are file appearances, not co-change proof; the large orchestration files merit first attention, not automatic splitting.

## 2. Candidate fixes, ranked

1. **Version and share cross-boundary contracts** — removes #1–#9, #17; files: unipi-kit topics/services, workflow, browser, cron, long-horizon, subagents, fusion, app status/session/chat consumers, host cron/browser models. Generate or fixture-check Rust/TS wire types and topic/entry names; keep UI-specific projection separate. Risk: wire/persisted compatibility and permissions; **NEEDS-OWNER** for migration and security semantics.
2. **Make child completion and handoff explicit** — removes #10, #11, #18–#22, #38–#41; files: subagents/child, fusion, dream, ask-user, memory, host task/agent APIs. Share a stable lifecycle service, completion notifications and bounded fallback policy rather than coupling to source layout or timing. Risk: changed cancellation, output ordering and handoff latency; **NEEDS-OWNER**.
3. **Establish durable state/error boundaries** — removes #25–#34; files: long-horizon, subagents, fusion, dream, file-summary, updater, info-screen, mcp, workflow, skill-registry. Return/log actionable persistence failures and distinguish absent state from corrupt/unreadable state; group related state writes where consistency matters. Risk: user-visible errors and recovery semantics; **NEEDS-OWNER**.
4. **Isolate scheduler and browser adapters behind small typed clients** — removes #4, #5, #12–#15, #23, #24; files: cron jobs/tools/settings/lib, dream, browser tools/result, host services. One client owns operation names, transport and scheduler file contract, while commands/tools remain adapters. Risk: changes to offline cron behavior, permission flow and host protocol; **NEEDS-OWNER**.
5. **Retire compatibility paths only after migration/consumer evidence** — removes #16, #36, #37, #43, #44; files: workflow, app status, dream, updater cache, background-tasks, skill-registry, TUI consumers and saved files. Inventory versions and migrate deliberately before removal. Risk: old sessions/configuration and multi-chat status; **NEEDS-OWNER**.
6. **Contract-oriented test fixtures and comment pruning** — addresses #42, #45, #46; files: cron tests, memory tests, browser and app contract tests, selected comments. Risk: low if coverage retained; fixture design must not hide behavioral changes.

## 3. Full problem list

Paths are repo-relative. Each numbered item is an observed change-together concern; confidence is high unless marked otherwise. Proposed remedies are not assertions that every fallback or compatibility path is wrong today.

### Hidden links

1. **Workflow per-chat topic** — `extensions/unipi/workflow/src/lib.rs:201`, `extensions/unipi/browser/src/result.rs:68`, `apps/unipi/src/lib/status/plan.ts:13`: producer, browser permission consumer and TS app independently construct the topic; changing it requires coordinated edits. [high]
2. **Plan entry wire identifier** — `extensions/unipi/workflow/src/plan.rs:12`, `apps/unipi/src/lib/status/plan.ts:14`: Rust persists an identifier the app independently matches, with no shared schema ensuring compatible changes. [high]
3. **Todo shape and validation** — `extensions/unipi/long-horizon/src/ralph.rs:197`, `apps/unipi/src/lib/status/plan.ts:47`: both encode statuses and 200-character truncation, but UI accepts lists the producer rejects, so changes to the domain require deliberate dual updates. [high]
4. **Cron disk schema** — `extensions/unipi/cron/src/jobs.rs:38` and `:91`: the extension explicitly mirrors the scheduler job and file envelope while reading/writing host-owned files, making independent model evolution unsafe. [high]
5. **Cron run response shape** — `extensions/unipi/dream/src/lib.rs:742`: dream calls `cron/show` and manually interprets `/job/next_fire`, `/job/running`, and `/runs/0`, coupling to an untyped host response. [high]
6. **Child step IDs** — `extensions/unipi/subagents/src/lib.rs:684`, `extensions/unipi/fusion/src/lib.rs:829`, `apps/unipi/src/lib/chat/model.ts:178`, `apps/unipi/src/lib/session/agentRuns.ts:50`: producers and multiple app classifiers repeat entry names; a rename must be synchronized. [high]
7. **Browser turn-shot ID** — `extensions/unipi/browser/src/lib.rs:34`, `apps/unipi/src/lib/timeline/eventKinds.ts:509`: persisted entry producer and app timeline independently spell the ID. [high]
8. **Fusion service/topic names** — `extensions/sdk/unipi-kit/src/services.rs:50`, `extensions/sdk/unipi-kit/src/topics.rs:63`, `apps/unipi/src/lib/model/fusion.ts:14`: Rust and TS separately own service and topic strings and must evolve together. [high]
9. **Tray invalidation convention** — `extensions/unipi/subagents/src/lib.rs:383`, `extensions/unipi/background-tasks/src/lib.rs:184`, `extensions/unipi/core/src/lib.rs:40`: contributors publish raw `WORK_TRAY_CHANGED` with a tab string that core interprets; neither typed contributor contract nor compiler enforces invalidation after changes. [high]
10. **Shared child runtime by source path** — `extensions/unipi/fusion/src/lib.rs:30`, `extensions/unipi/subagents/src/child.rs:22`: fusion includes subagents' internal source file rather than a separately owned API; a subagents-internal change silently changes fusion. [high]
11. **Child lifecycle and grace** — `extensions/unipi/subagents/src/lib.rs:700`, `extensions/unipi/fusion/src/lib.rs:834`: both use the same child settle grace while separately orchestrating steps and completion, so lifecycle changes must coordinate two implementations. [high]
12. **Browser op naming** — `extensions/unipi/browser/src/result.rs:94`, `extensions/unipi/browser/src/tools.rs:48`: host operation name is derived by stripping `browser_` from tool names rather than declared alongside the host contract. [high]
13. **Browser action classification** — `extensions/unipi/browser/src/tools.rs:48`, `:267`: tool registration and screenshot-trigger classification repeat tool names, so a new page action can omit the turn-tail screenshot. [high]
14. **Cron transport duplicated** — `extensions/unipi/cron/src/lib.rs:79`, `extensions/unipi/cron/src/tools.rs:17`: two `host_op` implementations independently check availability, call the same service and map errors. [high]
15. **Split cron service/config** — `extensions/unipi/cron/src/settings.rs:118`, `extensions/unipi/cron/src/tools.rs:18`, `extensions/unipi/dream/src/lib.rs:742`: settings, tools and dream separately know the scheduler service and operations. [high]
16. **Global versus per-chat status** — `extensions/unipi/workflow/src/lib.rs:205`, `apps/unipi/src/lib/status/plan.ts:22`: publisher intentionally maintains both topics and app falls back to the global one even with a session ID; consumers must know when fallback is safe. [high]
17. **Workflow permissions affect browser** — `extensions/unipi/browser/src/result.rs:62`, `:81`, `extensions/unipi/workflow/src/lib.rs:185`: the browser separately interprets the string `full` as approval, tying security semantics to a workflow payload convention. [high]

### Needless links

18. **Fusion reaches into a peer's implementation** — `extensions/unipi/fusion/src/lib.rs:32`: the path include exposes the entire child module, so independent fusion releases are coupled to subagents' file layout and private implementation. [high]
19. **Memory startup card depends on async wake timing** — `extensions/unipi/memory/src/lib.rs:1090`: card rendering polls a separate session-start task for up to one second instead of consuming its completion; cosmetic presentation and startup scheduling change together. [high]
20. **Dream wait knows internal registries** — `extensions/unipi/dream/src/lib.rs:657`: waiting for a run depends on both `agents` and `closing` maps being updated in the right order, not a run-completion interface. [high]
21. **Subagent foreground progress polls state** — `extensions/unipi/subagents/src/lib.rs:938`: progress delivery and completion detection are driven by a 400 ms polling loop, tying responsiveness to runtime scheduling. [med]
22. **Fusion foreground progress polls state** — `extensions/unipi/fusion/src/lib.rs:939`: a separate 500 ms polling loop has the same lifecycle coupling but different cadence. [med]

### Surplus

23. **Cron instance wrapper without instance state** — `extensions/unipi/cron/src/lib.rs:105`: `list_value` explicitly discards `self` and wraps `jobs::list`/`job_value`; the receiver adds coupling to `Cron` without providing behavior. [high]
24. **Duplicate scheduler adapter** — `extensions/unipi/cron/src/tools.rs:17`, `extensions/unipi/cron/src/lib.rs:79`: near-identical host wrappers are redundant separate maintenance surfaces, though one shared adapter remains useful. [high]

### Special cases

25. **Graph state writes discarded** — `extensions/unipi/long-horizon/src/lib.rs:322`: several separately persisted owner, goal and graph writes ignore failure, so an in-memory transition can appear successful without durable state. [high]
26. **Subagent records write discarded** — `extensions/unipi/subagents/src/lib.rs:378`: failed index writes are ignored even though records are later reloaded from disk. [high]
27. **Fusion preset writes discarded** — `extensions/unipi/fusion/src/lib.rs:511`, `:534`: runtime and curation saves ignore errors, so persisted configuration can differ from displayed state. [high]
28. **Dream writes discarded** — `extensions/unipi/dream/src/lib.rs:264`: directory creation, serialization fallback and write errors are hidden in the state-file helper, obscuring run artifact failures. [high]
29. **File-summary index write discarded** — `extensions/unipi/file-summary/src/lib.rs:112`: failure to persist the per-turn index is ignored before later flush/recovery uses indexes. [high]
30. **Updater cache write discarded** — `extensions/unipi/updater/src/cache.rs:65`: failed cache persistence is invisible; native serialization also defaults to empty bytes on failure. [high]
31. **Info-screen cache errors become empty** — `extensions/unipi/info-screen/src/registry.rs:275`: missing, unreadable, malformed and wrong-version snapshots collapse to an empty cache, hiding corruption as absence. [high]
32. **Watchdog restore errors become no pins** — `extensions/unipi/watchdog/src/lib.rs:84`: failed session queries and absent cap entries both yield null pins, potentially changing effective caps after restore. [high]
33. **Workflow publication failure discarded** — `extensions/unipi/workflow/src/lib.rs:202`: per-session sticky status publish ignores errors even though browser/app read it for permission or display state. [high]
34. **Skill judgment freeze write discarded** — `extensions/unipi/skill-registry/src/lib.rs:530`: failure to persist the freeze is ignored before selected skills are announced, so restart/replay may differ. [med]
35. **MCP startup errors discarded** — `extensions/unipi/mcp/src/lib.rs:498`: `start_servers` result is dropped at this call site, making startup failures dependent on other reporting paths. [med]
36. **Dream legacy migration rename ignored** — `extensions/unipi/dream/src/lib.rs:313`: legacy state is absorbed and rename is attempted without checking success, leaving migration repeatability dependent on filesystem behavior. [high]
37. **Updater dual dismissal format** — `extensions/unipi/updater/src/cache.rs:98`: old single-channel skip fields and new per-channel dismissals are both interpreted; removal requires persisted-cache migration policy. [high]
38. **Background cancellation polling** — `extensions/unipi/background-tasks/src/lib.rs:318`: cancellation waits in fixed 50 ms polls for state to change instead of awaiting completion. [high]
39. **Background output polling** — `extensions/unipi/background-tasks/src/lib.rs:390`: blocking output spins at 25 ms until finish/deadline/cancel, coupling task latency and host load to a magic interval. [high]
40. **Compaction handoff race** — `extensions/unipi/ask-user/src/lib.rs:409`, `:425`: a 1.5 s fallback can deliver the selected command while compaction is still running; `Once` prevents duplicate delivery, not ordering changes. [high]
41. **Dream waits by polling** — `extensions/unipi/dream/src/lib.rs:657`: a two-second sleep synchronizes on removal from internal collections and adds up to that much completion latency. [high]
42. **Memory real-store test skipped** — `extensions/unipi/memory/src/files.rs:538`: the only cited real-store reproduction is ignored and also silently returns if its environment variable is missing, so routine CI cannot exercise it. [high]
43. **Background legacy input defaults** — `extensions/unipi/background-tasks/src/lib.rs:342`: missing summary is labeled `(legacy call)` and missing risk defaults high, preserving an implicit old caller contract. [high]
44. **Skill settings legacy import** — `extensions/unipi/skill-registry/src/lib.rs:564`: initialization imports legacy settings, so configuration schema cleanup must account for existing installs. [high]

## 4. Special-case count

20 individually catalogued special-case findings (#25–#44); this is a verified lower bound, **not** a whole-area total. A mechanical grep across `extensions/unipi/*/src/*.rs` and nested Rust files found 401 lines containing `let _ =`, 278 containing `.ok()`, 692 containing `unwrap_or_default(`, 21 containing `sleep(`, 64 containing lowercase `legacy|compat|old host`, and one `#[ignore]`; categories overlap and include tests, benign optional values, real timeouts and comments, so do not add them or treat them as defect counts. Polling, ignored test and compatibility cases are explicitly represented above; not every match was individually reviewed.

## 5. Comments and tests notes

- Restatement examples: `extensions/unipi/cron/src/lib.rs:70` (“The extension” above `Cron`), `:58` (“The bundled skill's name” above `CRON_SKILL`), `extensions/unipi/cron/src/tools.rs:16` (host-op description repeats the helper name). Rough estimate: **dozens** across this area; not a census, and public API documentation can be useful even when terse.
- #45 **Implementation-coupled help test** — `extensions/unipi/cron/src/lib.rs:222`: `help_lists_the_subcommands` iterates the implementation's own `SUBCOMMANDS` and checks generated help contains them; it can miss a subcommand omitted from both, though it can fail on rendering regressions. [high]
- #46 **Weak ignored repro** — `extensions/unipi/memory/src/files.rs:538`: the ignored real-store smoke test asserts only a positive parsed count and allows missing environment to return successfully, so it does not verify expected search results. [high]
- Rough count: **at least 2 representative weak/internals-coupled tests**, not a quantified suite-wide estimate; no blanket claim that these tests cannot ever fail. Add independent wire fixtures and end-to-end behavior checks before changing contracts.

Verification: static reads and cross-repo literal searches, plus read-only history sampling; no builds/tests run because this is inventory-only and other agents are modifying the tree. Findings and counts reflect the working tree at inspection time; concurrently edited lines may move.
