# Oino foundation — untangle inventory

## 1. Area summary

- `oino-ai` supplies model/message/stream types; `oino-agent` runs turns and tools on those types.
- `oino-models` owns provider/auth/model resolution; `oino-settings` owns layered settings and secrets.
- `oino-resources` discovers prompts, skills and keybindings; `oino-session` persists and projects session trees.
- `oino-extension-api` defines extension contracts; `oino-wasm-host` adapts WIT, services and extension state.
- `oino-tools` implements built-ins and semantic search; `oino-zvec` is its helper process; `xtask` orchestrates developer builds.

Scope: working-tree snapshot, including concurrent uncommitted edits; this is an inventory, not a defect adjudication or exhaustive semantic proof. Public SDK crates have external consumers; absence of local callers would not prove an API is removable. Confidence rates the coupling observation, not the benefit of a proposed redesign. History check: read-only `git log --format= --name-only -n 400 -- <area>` shows high churn in wasm-host hub (21 file occurrences), core bridge (16), and session manager (16); this is frequency, not proof of co-change. No tests or builds were run for this read-only inventory.

## 2. Candidate fixes ranked

1. **Typed/versioned contracts at cross-process and UI boundaries** — addresses #1–#6, #17. Generate or share zvec request/reply definitions, event discriminants/payloads and WIT package artifacts; keep packaging drift check. Files: `oino-tools/src/zvec/{mod,client}.rs`, `oino-zvec/src/main.rs`, `oino-wasm-host/{wit,src/hub.rs}`, root `wit/`, session entry and UniPi timeline/search UI, extension notification producer. Risk: large cross-crate/UI blast radius; protocol/version and UI rendering changes **NEEDS-OWNER**. WIT duplication is a deliberate crates.io packaging constraint, not simply removable.
2. **One compatibility boundary per input domain** — addresses #8–#11, #30–#35. Normalize old tool calls once at ingress, keep versioned session/settings migrations explicit, and remove downstream re-detection only after proving all direct callers normalized. Files: `oino-tools/src/{legacy,canonical,read,grep,edit,registry}.rs`, core dispatcher/direct-tool tests, `oino-session/src/{manager,migrate}`, `oino-settings/src/json.rs`, resource keybindings. Risk: changing accepted legacy inputs, stored records or default behavior **NEEDS-OWNER**; public direct-tool API and existing sessions must continue working or be versioned.
3. **Make durable-state errors explicit at storage boundaries** — addresses #36–#40, #45–#50, #55–#57. Distinguish missing state from corrupt/unreadable state and surface failed mutations, with fault-injection tests; keep intentionally disposable caches and best-effort cleanup distinct. Files: wasm-host `kv.rs`, `state.rs`, `tasks.rs`, imports and WIT if needed; settings `secrets.rs`; session `manager/{lease,index}.rs`; resource loader/walk. Risk: callers may now see failures and data recovery policy changes **NEEDS-OWNER**; WIT ABI may need a versioned change.
4. **Scope mutable services to an explicit runtime/context** — addresses #12–#14, #18, #27. Inject diagnostics, reserved-owner registry, command cache and helper pool through owning runtime instead of process-wide state, keeping embedder compatibility adapters as necessary. Files: `oino-tools/src/{write,zvec/client}.rs`, `oino-models/src/config_value.rs`, `oino-extension-api/src/manifest.rs`, their host/core callers and tests. Risk: public SDK interface and multi-runtime isolation semantics **NEEDS-OWNER**; cache lifetime changes can affect credential refresh.
5. **Represent capabilities and classifications as data owned by their feature** — addresses #7, #15, #20–#21, #28–#29. Describe patch availability/model kind/search model once and consume it in registry, runtime and UI rather than infer from IDs or duplicate tables. Files: `oino-ai/src/model.rs`, models `kind.rs` and providers, tools `apply_patch.rs`, `registry.rs`, `zvec/{mod,native}.rs`, UniPi model/search screens. Risk: model selection, exposed tools and search index compatibility are user-visible **NEEDS-OWNER**.
6. **Replace timing-dependent test coordination with explicit events** — addresses #41–#44. Use barriers/notifications for ordering and retain timeouts only as bounds; separate optional performance runs. Files: tools mutation queue and behavior tests, wasm-host timers/tasks tests, session index/perf tests. Risk: test-only except optional instrumentation; perf limits still need environment policy.
7. **Shrink misleading interfaces after caller audit** — addresses #22–#26. Remove private unused parameters and redundant internal forwarding where possible; preserve or deprecate published surface. Files: resources `system_prompt.rs`, tools `zvec/mod.rs`, extension `host_service.rs`; external SDK compatibility **NEEDS-OWNER** for public changes.

## 3. Full problem list

### Hidden links

1. **[high]** `crates/oino-tools/src/zvec/mod.rs:109` and `crates/oino-zvec/src/main.rs:10` independently encode `index_status` and its fields in JSON, so the caller and helper must change together without a shared request type.
2. **[high]** `crates/oino-tools/src/zvec/client.rs:145` and `crates/oino-zvec/src/main.rs:78` independently interpret/build the `ok`/`result`/`error` response envelope.
3. **[high]** `crates/oino-tools/src/zvec/mod.rs:48` and `crates/oino-zvec/src/main.rs:40` separately set the helper idle duration to 300 seconds, so a policy edit can desynchronize lifetime assumptions.
4. **[high]** `crates/oino-wasm-host/tests/wit_sync.rs:33` documents two actual WIT trees, `wit/oino-extension` and `crates/oino-wasm-host/wit/oino-extension`, that must match; the test catches drift only when run inside the monorepo.
5. **[high]** `crates/oino-wasm-host/src/hub.rs:38`, `extensions/unipi/background-tasks/src/lib.rs:994`, and `apps/unipi/src/components/timeline/cards/RendererCard.tsx:48` separately spell the background-task message discriminator, tying producer, host and renderer through a string.
6. **[high]** `crates/oino-session/src/entry.rs:302` and `apps/unipi/src/lib/session/turns.ts:723` independently define the cwd-change discriminator; `apps/unipi/src/lib/timeline/snapshotCache.ts:37` repeats it again.
7. **[high]** `crates/oino-models/src/kind.rs:19` and `apps/unipi/src/lib/model/availability.ts:11` independently classify model IDs; their token lists already differ (e.g. UI `music`/`lyria`), so picker and cycle can disagree.
8. **[high]** `crates/oino-tools/src/legacy.rs:36` normalizes calls while `crates/oino-tools/src/read.rs:112` still inspects the original legacy `path` to select behavior, making upstream and tool implementation jointly responsible for compatibility.
9. **[high]** `crates/oino-tools/src/legacy.rs:70` rewrites legacy grep fields and defaults, while `crates/oino-tools/src/grep.rs:351` independently detects the old field names and selects output behavior.
10. **[high]** `crates/oino-tools/src/canonical.rs:16`, `crates/oino-tools/src/legacy.rs:45`, and `crates/oino-tools/src/edit/mod.rs:51` each participate in legacy path/edit normalization, so direct and dispatcher paths must stay aligned.
11. **[medium]** `crates/oino-session/src/manager/mod.rs:295` migrates files on open while `crates/oino-session/src/manager/mod.rs:502` preserves other migrated records as an in-memory compatibility view; callers need to know which migration persists and when.
12. **[high]** `crates/oino-tools/src/write.rs:119` stores a process-global diagnostics hook installed through `set_diagnostics_hook` at line 126, so all tool instances depend on external installation order and share a hook.
13. **[high]** `crates/oino-extension-api/src/manifest.rs:27` requires the embedder to register process-global reserved owners before manifest ownership checks; an omitted or late registration changes validation.
14. **[high]** `crates/oino-models/src/config_value.rs:27` and `:350` implement a process-global command-result cache with a separate clearing API, coupling credentials/config freshness to caller-managed invalidation.
15. **[high]** `crates/oino-tools/src/zvec/mod.rs:44`, `crates/oino-tools/src/zvec/native.rs:106`, and `apps/unipi/src/screens/settings/SearchSettings.tsx:104` separately identify the supported embedding model.
16. **[high]** `crates/oino-tools/src/config.rs:51` enumerates session environment names, while `:76` builds aliases from prefix/suffix conventions; adding an exported variable requires both lists to remain synchronized.
17. **[medium]** `crates/oino-extension-api/src/manifest.rs:17` has a Rust WIT-version literal while `crates/oino-wasm-host/tests/wit_sync.rs:35` protects only the two WIT trees, not this separate reported version.

### Needless links

18. **[medium]** `crates/oino-tools/src/zvec/client.rs:66` uses one global pool and `:109` holds its single mutex across process request/response I/O, coupling unrelated project helper calls and lifecycle.
19. **[high]** `crates/oino-wasm-host/src/core_bridge.rs:339` maps typed control operations through loosely typed `HostOp`/JSON replies and defaults, so adding a control response requires synchronized adapters across API and core.
20. **[medium]** `crates/oino-tools/src/info.rs:36` maintains name enumeration and `crates/oino-tools/src/registry.rs:23` separately dispatches construction, so adding a built-in requires parallel edits to shallow inventories.
21. **[medium]** `crates/oino-tools/src/registry.rs:82` treats grep metadata specially for semantic settings while tool construction is elsewhere, coupling generic registry assembly to one optional search workflow.
22. **[medium]** `crates/oino-resources/src/system_prompt.rs:242` accepts `options` and `declared` in a generic default-section function but ignores both at line 249, leaving callers coupled to parameters the function does not use.

### Surplus / shallow boundaries

23. **[high]** `crates/oino-tools/src/zvec/mod.rs:10` is a public forwarding wrapper to `install::install` at line 15, adding a layer without independent policy; public SDK stability prevents assuming it can be removed.
24. **[high]** `crates/oino-tools/src/zvec/mod.rs:31` and `:36` only forward progress/capabilities to private modules, so the façade adds indirection rather than behavior; it may nonetheless be a useful intentional public boundary.
25. **[medium]** `crates/oino-extension-api/src/host_service.rs:41` provides a contextual `call_in` default that drops `cx` at line 48 and delegates to `call`, forcing implementations to understand two entry points even when context is irrelevant; retain if actual embedders need both.
26. **[high]** `crates/oino-resources/src/system_prompt.rs:244` passes two unused private parameters through to `insert_default_sections`, which can be simplified without a public API decision if all local callers agree.
27. **[medium]** `crates/oino-tools/src/write.rs:136` maps a poisoned global hook lock to no diagnostics, coupling unrelated tool calls to global lock health while hiding the distinction from no hook.

No dead public item is asserted: a repository-only grep cannot establish absence of users of published crates, and a full symbol-by-symbol cross-repo audit was not completed.

### Special cases

28. **[high]** `crates/oino-tools/src/apply_patch.rs:14` enables patch tooling from `gpt`, `codex` or `o`+digit name heuristics, and `registry.rs:95` ties declaration to the same heuristic rather than a model capability. **NEEDS-OWNER**.
29. **[high]** `crates/oino-tools/src/zvec/native.rs:106` accepts only the literal local embedding provider/model; model upgrades need coordinated index and UI policy. **NEEDS-OWNER**.
30. **[high]** `crates/oino-tools/src/legacy.rs:86` translates `bg_run` into a shell invocation with invented high-risk/fire-and-forget defaults, so compatibility includes behavior rather than just renaming. **NEEDS-OWNER**.
31. **[high]** `crates/oino-tools/src/legacy.rs:96` maps three old background-tool names to two new operations with special argument defaults; old transcripts/callers depend on this branch. **NEEDS-OWNER**.
32. **[high]** `crates/oino-tools/src/info.rs:79` special-cases `find` even though `all()` at line 39 omits it, preserving an execution-only legacy alias. **NEEDS-OWNER**.
33. **[high]** `crates/oino-settings/src/json.rs:50` migrates old `websockets`, `skills`, `theme`, and retry shapes in normal settings handling, tying old compatibility to all current loads/writes. **NEEDS-OWNER**.
34. **[high]** `crates/oino-session/src/manager/mod.rs:295` performs legacy file migration implicitly when opening a session, including filesystem moves in `manager/files.rs:289`. **NEEDS-OWNER**.
35. **[high]** `crates/oino-resources/src/keybindings.rs:120` migrates legacy keybinding names using a separate table, making historical identifiers part of current resource loading. **NEEDS-OWNER**.
36. **[high]** `crates/oino-wasm-host/src/kv.rs:27` treats read/JSON errors as an empty KV map and `:63` can subsequently overwrite it on set, conflating missing and damaged durable state. **NEEDS-OWNER**.
37. **[high]** `crates/oino-settings/src/secrets.rs:89` treats invalid secrets JSON as an empty map before a dirty update writes it back, potentially replacing unrelated secrets. **NEEDS-OWNER**.
38. **[high]** `crates/oino-wasm-host/src/state.rs:90` defaults failed daemon-state reads/parses to empty and ignores write failure at line 96, so daemon tracking can silently disappear. **NEEDS-OWNER**.
39. **[high]** `crates/oino-wasm-host/src/tasks.rs:489` ignores task-log write errors and `:495` discards the timeout result after a fixed two-second drain; truncated output can be presented without a failure indication. **NEEDS-OWNER**.
40. **[high]** `crates/oino-wasm-host/src/tasks.rs:551` ignores task-state serialization/write failure, making persisted task history silently best-effort. **NEEDS-OWNER**.
41. **[high]** `crates/oino-tools/src/mutation_queue.rs:99` and `:108` use sleeps to establish ordering in a concurrency test, so scheduling variability can make the intended order nondeterministic.
42. **[high]** `crates/oino-wasm-host/src/timers.rs:89` uses fixed sleeps in timer tests, tying assertions to machine scheduling rather than a controlled clock.
43. **[high]** `crates/oino-session/src/manager/index/tests.rs:437` requires a lookup to finish under 50 ms, so host load can fail a correctness test independent of behavior.
44. **[high]** `crates/oino-session/tests/perf_cards.rs:88` ignores a large performance test by default because it writes ~400 MB, leaving that check outside ordinary test runs.
45. **[high]** `crates/oino-wasm-host/src/imports/runtime.rs:284` turns denied/failed editor reads into empty text, while `:295` and `:306` discard failed editor mutations; the current WIT return shapes constrain error reporting. **NEEDS-OWNER**.
46. **[high]** `crates/oino-wasm-host/src/core_bridge.rs:350` interprets malformed idle replies as `true`, and `:385` turns invalid active-tool replies into an empty list, masking contract failures. **NEEDS-OWNER**.
47. **[high]** `crates/oino-session/src/manager/lease.rs:145` ignores creation/write failures for the fallback boot ID, so the returned identity may not be stable across processes or restarts. **NEEDS-OWNER**.
48. **[high]** `crates/oino-resources/src/walk.rs:89` silently skips unreadable ignore files and `:114` drops invalid rules; omission can change which resources are discovered. **NEEDS-OWNER** for diagnostic/behavior policy.
49. **[high]** `crates/oino-resources/src/loader.rs:512` treats an unreadable theme directory as empty despite already having a diagnostics channel, so users cannot distinguish empty from inaccessible. **NEEDS-OWNER**.
50. **[high]** `crates/oino-session/src/manager/index.rs:402` drops index-write errors and marks the cache dirty for retry; reasonable for a disposable cache, but persistence failures remain invisible.
51. **[high]** `crates/oino-tools/src/env/local.rs:313` polls Windows child completion every 10 ms because the child is behind a mutex, coupling synchronization to a platform-specific timer.
52. **[high]** `xtask/src/remote.rs:274` syncs tracked HEAD only and ignores untracked files even in its dirty check at line 276; a remote build can omit a local new file without the warning identifying it.
53. **[high]** `xtask/src/remote.rs:365` polls remote build completion every 60 seconds, trading implementation simplicity for delayed status and cancellation; not automatically a defect.
54. **[medium]** `crates/oino-models/src/config_value.rs:388` polls credential-command completion every 5 ms and collapses execution/reader failures into `None`; callers cannot distinguish missing configuration from command failure. **NEEDS-OWNER**.
55. **[high]** `crates/oino-wasm-host/src/kv.rs:68` ignores delete persistence errors, so the API can report completion even when the key remains on disk. **NEEDS-OWNER**.
56. **[high]** `crates/oino-wasm-host/src/core_bridge.rs:474` drops subscribe/unsubscribe and unregister errors because these APIs return unit, masking registration state disagreement. **NEEDS-OWNER**.
57. **[high]** `crates/oino-settings/src/autonomy.rs:222` treats invalid `autonomy.defaultLevel` as its default; this fail-default policy is security-relevant and must not change without a decision. **NEEDS-OWNER**.

## 4. Special-case count

There are **30 individually listed special-case findings (#28–#57)**, including intentional compatibility, error and scheduling choices; this is a reviewed finding count, **not** the total number of branches in the area. Mechanical grep across all area `.rs` files including tests found 121 lines containing `let _ =`, 186 with `.ok()`, 175 with `unwrap_or_default(`, 48 with `sleep(`, 36 with `timeout(`, 213 with `legacy|compat`, and one `#[ignore` line. These are overlapping text-match counts, not counts of defects, synchronization uses, distinct sites or exhaustive special cases; many represent harmless conversions, legitimate deadlines, tests, documentation, or intentional best-effort cleanup. No comprehensive enumeration of value/id branches or empty catches was performed.

## 5. Comments / tests notes

- **Comments:** a rough mechanical count finds 58 spaced-out generated-looking doc lines matching `/// [A-Z]( [a-zA-Z]){3}` across area Rust files; this is a lower-bound heuristic, not a count of all restating comments. Examples: `crates/oino-tools/src/registry.rs:21` (“Create Tool”), `crates/oino-tools/src/bash/accumulator.rs:142` (“Append”), and `crates/oino-resources/src/walk.rs:80` (“Add Ignore Rules”) repeat the following function names rather than explain contracts. Preserve useful public API and safety documentation.
- **Tests:** at least four reviewed timing/coverage concerns, #41–#44; this is a lower bound, not an area-wide count. `crates/oino-resources/tests/template_props.rs:14` is intentionally a no-panic property, not a semantic substitution oracle; it can fail on a panic but accepts incorrect non-panicking output. `crates/oino-wasm-host/tests/wit_sync.rs:37` intentionally returns without comparison in a packaged crate, so a green packaged run does not establish monorepo WIT parity. `crates/oino-tools/src/mutation_queue.rs:117` asserts an externally meaningful order but arranges it with sleeps; no claim that it cannot fail. `crates/oino-session/src/manager/index/tests.rs:450` discards warm-up results, potentially obscuring the source of later failures. No blanket claim that golden/round-trip tests merely reproduce implementation was established.

Candidate-fix cross references are illustrative clusters, not claims that one patch mechanically deletes every item: e.g. storage observability (#36–#40) overlaps adapter/interface concerns, and timing fixes cannot remove legitimate production timeouts. Further triage should verify all cross-references against the evolving working tree before implementation.
