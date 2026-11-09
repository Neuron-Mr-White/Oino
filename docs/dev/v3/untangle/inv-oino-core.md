# Oino core engine — untangle inventory

Scope: current working tree, `crates/oino-core` including tests; cross-repo references sampled in `crates`, `apps/unipi/src`, and `extensions`. This is an inventory, not a claim that every compatibility path is removable. Line numbers reflect the concurrently edited tree. Confidence describes the evidence for the coupling, not certainty that the proposed fix is worthwhile.

## 1. Area summary (five-line module map)

- `services.rs` composes trust, settings, resources, models, extension loading and host routing; `runtime.rs` owns session creation, replacement and leases.
- `session/` implements the agent facade, prompt/turn lifecycle, hooks, tools, permissions, frontend feeds, checkpoints, compaction and child wiring.
- `agents/` owns child specifications, lifecycle/hibernation and tree feeds; `builtin.rs` binds built-in tools back to session context.
- `extensions/` owns host contracts, dispatch, discovery, deadlines and settings; `provider.rs` translates component provider streams.
- `events.rs`, `ui/`, `hub.rs`, `topics.rs`, and `compaction/` hold client events, UI projection, fanout, topics and summary mechanics.

History signal: `git log --format= --name-only -n 400 -- crates/oino-core` reports `session/mod.rs` in 28 commits, `session/host.rs` in 22, `services.rs` in 18, and `agents/child.rs` / `extensions/settings.rs` in 15 each. These are per-file touch counts, not proof of pairwise co-change; this inventory does not infer co-change from them.

## 2. Candidate fixes, ranked

1. **Make protocol and persisted identifiers owned contracts**: shared Rust definitions plus generated or contract-tested TS/guest representations, retaining wire spelling. Removes #1–#9 and reduces #10. Files: `session/{permission,tools,host,hooks,compaction_flow,frontend}.rs`, `provider.rs`, `ui/{bridge,registry}.rs`, client protocol, app session files and extension SDK/guests. Risk: high cross-workspace blast radius; wire/persistence changes **NEEDS-OWNER**; generation versus contract tests is a design choice.
2. **Give session construction and attachment one lifecycle owner**: construct a fully bound session/tool context/router through a single internal builder; expose narrow operations rather than mutable manager internals. Removes #11–#14, #19–#22 and #28. Files: `services.rs`, `runtime.rs`, `session/{mod,feeds}.rs`, `agents/{mod,child}.rs`, `builtin.rs`, affected clients/tests. Risk: high lifecycle and initialization-order regression; preserve extension callback order and existing public API or seek owner approval.
3. **Encapsulate checkpoint storage with explicit outcomes**: own snapshot/index/blob lifecycle together and report persistence versus in-memory fallback. Removes #23, #29–#33. Files: `session/checkpoints.rs`, checkpoint callers/frontend, tests. Risk: durability and error-display behavior **NEEDS-OWNER**; existing best-effort semantics may be intentional.
4. **Make wait/settle and child lifecycle event-driven where signals exist**: one owner for wake, forwarded events and wait-source refresh; retain bounded timeouts for external processes. Removes or narrows #34–#39 and test risks #57–#59. Files: `runtime.rs`, `session/{boundary,feeds}.rs`, `agents/child.rs`, extension API/runner and lifecycle tests. Risk: high concurrency and cross-process lease semantics; timing/user-visible waiting changes **NEEDS-OWNER**. File-based takeover may still require polling.
5. **Separate generic dispatch from product-specific policies and compatibility adapters**: move superagent, MCP, model/tool aliases and historical formats to owned adapters/policy tables, not generic dispatch. Removes #15–#18, #40–#46 and reduces #47–#49. Files: `session/{tools,host,hooks,prompt,permission}.rs`, `provider.rs`, `extensions/{settings,sources}.rs`, relevant extension and tests. Risk: high provider/tool, migration and permission behavior **NEEDS-OWNER**; keep persisted transcripts readable.
6. **Make fallible boundary decoding observable**: distinguish absent, malformed, timed out and rejected at extension/session edges, then apply an explicit policy. Addresses #24–#27 and #50–#56. Files: `extensions/{runner,settings}.rs`, `session/{frontend,hooks,host,permission}.rs`, `safe_mode.rs`, tests. Risk: changing default/fail-open behavior **NEEDS-OWNER**; logging alone can preserve semantics.

## 3. Full problem list

Each entry gives the smallest useful evidence and why it changes with another piece. `NEEDS-OWNER` marks a fix that could change externally visible behavior, not an assertion that the current implementation is incorrect.

### Hidden links

1. **[high]** `session/permission.rs:91,104` and `apps/unipi/src/lib/session/autonomy.ts:19`: persisted `oino:autonomy` and its `level` payload are independently spelled/decoded across Rust and TS; format changes require coordinated edits.
2. **[high]** `session/permission.rs:132,178` and `apps/unipi/src/lib/session/turns.ts:165`: `oino:unattended` is separately written, read and filtered, without a shared persisted-entry contract.
3. **[high]** `session/permission.rs:316`, `session/prompt.rs:943`, `session/cwd.rs:46`: the same `oino:superagent` marker independently controls authorization, prompting and working directory; a marker change silently alters several behaviors.
4. **[high]** `session/tools.rs:421` and `session/host.rs:296`: `oino.tool-declarations` ties persisted declarations to host updates through duplicated strings.
5. **[high]** `session/tools.rs:995` and `provider.rs:204`: the four `superagent.*` to underscore aliases are duplicated in tool selection and provider transcript normalization; both need the same mapping.
6. **[high]** `session/frontend.rs:756`, `ui/bridge.rs:477`, `ui/registry.rs:1176`: `setShortcuts` must be emitted, categorized and projected using matching ad-hoc method names.
7. **[high]** `ui/bridge.rs:476,486`, `ui/registry.rs:1207`, `crates/oino-wasm-host/src/core_bridge.rs:928`: `openPanel` and its panel key are independently encoded on host and UI sides; renames or field changes are not type-checked across the boundary.
8. **[high]** `session/hooks.rs:276,280,328` and `extensions/runner.rs:480`: hook event type strings and JSON payload fields must match registrations and runner dispatch without a typed event contract.
9. **[high]** `session/compaction_flow.rs:263`, `compaction/hook_payload.rs:56` and `extensions/unipi/compactor/`: the `session_before_compact` hook and its bounded payload span separate producer, payload and guest feature; protocol evolution requires coordination.
10. **[med]** `services.rs:107` and `crates/oino-settings/src/paths.rs:32`: the offline environment key is spelled locally despite an existing paths constant; CLI help also repeats accepted values (`crates/oino/src/cli/help.rs:75`).
11. **[high]** `builtin.rs:28` and `session/feeds.rs:317`: a tool context must be bound after the session exists and must separately install its detach registry; construction order is not expressed by the context type.
12. **[high]** `services.rs:657,676,687`: router models, settings, context and extensions are progressively installed through `OnceLock`s around guest initialization; callbacks rely on which phase has completed.
13. **[high]** `session/mod.rs:504,682,687`: stream, agent router and runtime are backfilled after constructing the session; consumers must know which binding operations occurred.
14. **[med]** `session/mod.rs:531` and `runtime.rs:703`: session-start emission, resource discovery, renderer setup and wait-source refresh are ordered initialization side effects spread across runtime and session.

### Needless links

15. **[high]** `session/tools.rs:1026`: generic tool classification recognizes MCP by extension ID/suffix; changing MCP packaging conventions requires editing core selection logic.
16. **[high]** `session/host.rs:314`: generic custom-message routing singles out `unipi.subagents` and `subagent-completion`, tying host internals to one extension's naming and delivery workflow.
17. **[high]** `session/tools.rs:469,481,494`: generic tool catalog construction knows the specific `codemode` and `tool_search` workflow, so catalog and workflow policy evolve together.
18. **[high]** `session/tools.rs:487,1036,1073`: `apply_patch` visibility/handling depends on model-name heuristics in general tool wiring; provider naming and generic catalog policy are coupled.
19. **[high]** `session/mod.rs:737,742`: public `with_manager`/`with_settings` expose mutable concrete managers through arbitrary closures, letting clients depend on storage and settings internals rather than session operations.
20. **[high]** `runtime.rs:786,794,800`: the runtime exposes `Services`, extension settings and the entire session, making the composition root's internal ownership structure part of client API.
21. **[med]** `agents/mod.rs:40,73` and `session/mod.rs:316`: child bookkeeping reaches `session::Inner` and `HostRouter` directly; child lifecycle changes are tied to parent session representation.
22. **[med]** `builtin.rs:10,16`: built-in tool context holds a weak `session::Inner` rather than a narrow tool-service interface, tying a general tools adapter to session layout.
23. **[high]** `session/checkpoints.rs:120,143,194`: the same session module coordinates index serialization, filesystem layout, blob storage and eviction, coupling user checkpoint behavior to storage format.
24. **[high]** `extensions/runner.rs:692,730,756`: general hook dispatch parses individual reply payloads into agent messages and silently discards invalid shapes; protocol validation and dispatch are mixed.
25. **[med]** `extensions/settings.rs:260`: the settings registry probes an untyped JSON schema by deserializing a concrete namespace; schema evolution and runtime fallback behavior are tied.
26. **[med]** `session/frontend.rs:462,499`: frontend completion orchestration owns service time budgets and extension reply decoding, coupling UI responsiveness to provider-specific call mechanics.
27. **[high]** `session/hooks.rs:264,294` and `provider.rs:67`: legacy tool-call normalization occurs both at hooks and provider transcript boundaries, so compatibility rules must remain aligned with both workflows.

### Surplus / shallow indirection

28. **[med]** `builtin.rs:28` and `session/feeds.rs:317`: two public/semipublic one-time bind operations effectively pass the same session attachment through separate layers; folding them into lifecycle construction removes an ordering surface (also #11).
29. **[med]** `session/checkpoints.rs:143`: `persist` returns `()` and its caller cannot tell successful persistence from a no-op/failure; its apparently simple interface hides storage distinctions required by callers that promise durable rewind (also #30–#33).

No public item is claimed dead: repo-wide textual search alone cannot rule out generated, feature-gated, external or dynamic users. Nor is an extension point claimed never to vary without inspecting all embedders. These are investigation boundaries, not evidence of absence.

### Special cases

30. **[high; NEEDS-OWNER]** `session/checkpoints.rs:127`: unreadable or malformed checkpoint index becomes no index, indistinguishable from first use; users can lose visibility of recoverable rewind data.
31. **[high; NEEDS-OWNER]** `session/checkpoints.rs:148,151`: directory creation and index rename errors are discarded, so `record` can report success without a durable index.
32. **[high; NEEDS-OWNER]** `session/checkpoints.rs:173`: every metadata error is treated as an absent file, conflating nonexistence with permission/I/O failure in a rewind snapshot.
33. **[high; NEEDS-OWNER]** `session/checkpoints.rs:194`: blob-write failure falls back to memory while index persistence continues; persistence/restart guarantees differ without a surfaced outcome.
34. **[high; NEEDS-OWNER]** `runtime.rs:262,290`: takeover synchronization polls the lock on a timer after dead-holder detection/request; changing retry windows changes takeover behavior.
35. **[high; NEEDS-OWNER]** `runtime.rs:492`: the holder polls for takeover requests, coupling handoff latency to the watcher interval and external file convention.
36. **[high; NEEDS-OWNER]** `session/boundary.rs:514`: wait-source state is refreshed by 2/5-second polling, so background/waiting state can lag its actual producer.
37. **[high; NEEDS-OWNER]** `agents/child.rs:1088,1110`: child hibernation and lifecycle conditions are checked using sleeps/polling, linking correctness and responsiveness to intervals.
38. **[high]** `session/feeds.rs:401`: shell-process status is polled every 500 ms while forwarding feed state, adding timing coupling to process completion.
39. **[high]** `extensions/deadline.rs:30`: dialog closure is detected by a 200 ms recheck loop; deadline resumption depends on polling rather than a closure signal.
40. **[high; NEEDS-OWNER]** `session/host.rs:314`: one extension/message pair has a dedicated delivery branch; changing this requires coordinating core and the specific extension.
41. **[high; NEEDS-OWNER]** `session/tools.rs:1026`: exact/suffix `mcp` extension-name checks are a one-feature special case in core.
42. **[high; NEEDS-OWNER]** `session/tools.rs:487,1036`: `apply_patch` eligibility is inferred from selected model-name fragments, not capability metadata; changing names can change available tools.
43. **[high; NEEDS-OWNER]** `provider.rs:77`: `openai-completions` changes transcript normalization through a specific API string in shared provider dispatch.
44. **[high; NEEDS-OWNER]** `provider.rs:204` and `session/tools.rs:995`: legacy superagent dotted aliases are maintained as special cases in two paths; older transcripts and tool lookup depend on them.
45. **[high; NEEDS-OWNER]** `extensions/settings.rs:976`: legacy dotted settings keys are expanded with nested-value precedence; removing this changes old configuration interpretation.
46. **[high; NEEDS-OWNER]** `extensions/sources.rs:459,463`: docs resolution retains a legacy bundled-directory fallback, a compatibility branch whose removal affects old installations.
47. **[high; NEEDS-OWNER]** `session/prompt.rs:1053,1107`: legacy conversations take a different session-start snapshot path; migration behavior is embedded in prompt assembly.
48. **[high; NEEDS-OWNER]** `session/permission.rs:255,270`: legacy deny entries are parsed and merged into modern autonomy policy, tying historical data format to current authorization.
49. **[high; NEEDS-OWNER]** `session/hooks.rs:264`: legacy tool normalization runs before extension hooks, so retiring it changes what extensions see.
50. **[high; NEEDS-OWNER]** `safe_mode.rs:148–150`: read and parse errors default safe-mode state, conflating broken state with no previous failures.
51. **[high; NEEDS-OWNER]** `extensions/runner.rs:998–1001`: arbiter wait-source timeout becomes no reason without reporting, potentially making a waiting extension appear idle.
52. **[high; NEEDS-OWNER]** `extensions/runner.rs:1024–1028`: timed-out or malformed arbiter proposals disappear as no proposal, hiding extension failure from callers.
53. **[high; NEEDS-OWNER]** `session/frontend.rs:462–464`: completion timeout returns an empty completion list, indistinguishable to the caller from a genuine empty result.
54. **[high; NEEDS-OWNER]** `session/frontend.rs:499–503`: extension completion timeout and malformed JSON both become no groups; callers cannot diagnose either cause.
55. **[high; NEEDS-OWNER]** `session/host.rs:879`: spawned prompt errors are explicitly discarded, so the originating workflow cannot observe failure.
56. **[high; NEEDS-OWNER]** `session/permission.rs:90,132`: persistence errors for autonomy and unattended markers are discarded, so in-memory intent and restored state can diverge.

## 4. Special-case count

**27 distinct inventory items (#30–#56)**: 4 checkpoint/error cases, 6 timing/polling cases, 10 ID/model/compat cases, and 7 further swallowed-error cases. This is a reviewed-item count, not a count of every textual `if`, `.ok()`, timeout, or occurrence across the crate; related occurrences are grouped. No `#[ignore]` matches were found under `crates/oino-core` in the textual scan; conditional runtime skips were not exhaustively audited. Some timeouts and best-effort failures are intentional boundaries, not automatically defects.

## 5. Comments and tests notes

- **Comments, rough count:** a lexical scan found 53 `///` lines starting with `Gets`, `Returns`, `Sets`, `Creates`, or `The … state/value/constant`; this is an upper bound/proxy, not 53 confirmed redundant comments. Representative restatements: `extensions/host.rs:161` (“Sets the session name”), `extensions/host.rs:170` (“Returns the active tools”), `session/nested.rs:22` (“The nested call limits constant”). Public API documentation can be useful even when short; do not mass-delete.
- **Tests, timing coupling:** 22 textual sleep occurrences in `crates/oino-core/tests`, not necessarily 22 bad tests. **#57 [high]** `tests/agents.rs:169` sleeps 100 ms before asserting forwarded child usage; scheduling delays can make it flaky. **#58 [high]** `tests/hibernation.rs:250` waits four real seconds to assert no hibernation, tying runtime and confidence to timing windows. **#59 [med]** `tests/session_flow.rs:208,222` coordinates threads by fixed sleeps; inspect the whole scenario before replacing with explicit readiness.
- **Tests, implementation coupling:** **#60 [med]** `session/queue.rs:270` directly tests private text-to-message matching, so representation changes can require test rewrites even when public queue behavior remains correct; keep behavioral coverage through `tests/gui_feeds.rs:264`. **#61 [med]** `extensions/deadline.rs:45–76` exercises a private implementation's polling/rebudget logic; retain public deadline behavior assertions when changing internals. These can fail and are not claimed tautological.
- No reliably non-failing test was established in the sampled code; a full assertion-by-assertion audit was not performed. Estimated directly internal-focused examples in this note: 2, not a crate-wide total.

Verification: source and selected cross-repo consumers were read, textual scans and read-only history were used; no build/tests run because this is inventory-only. Findings are opportunities and require design/behavior review, not an instruction to delete compatibility paths.
