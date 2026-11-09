# UniPi host daemon: untangling inventory

## 1. Area summary

- `main.rs` composes CLI, daemon startup/shutdown, scheduler, local and iroh transports.
- `protocol.rs` authenticates and routes `unipi-proto` requests to feature implementations.
- `sessions/`, `host_cmd.rs`, `superagent*.rs` bridge core runtime, chat, settings and board orchestration.
- `browser/`, `fs*`, `git*`, `term`, `lsp`, `blob`, `push`, `sharing` own local/remote capabilities.
- `cron*`, `notify`, `update`, `settings`, `data_dir`, `instance` provide background and host services.

Scope: current working tree, including concurrent edits; no source changes. Paths below are relative to repository root; `H` means `apps/unipi/crates/unipi-host`. Confidence describes the observation, not certainty that removing it is safe. `NEEDS-OWNER` flags possible user-visible or security behavior changes. This is an evidence-based inventory, not a claim that every occurrence of every pattern was individually reviewed. The last 400 commits touch `sessions/stream.rs` 35 times, `sessions/hub.rs` 32, `superagent.rs` 22, `protocol.rs` and `superagent_session.rs` 18 each; these are frequency counts, not proof of co-change.

## 2. Candidate fixes ranked

1. **Explicit host service context and lifecycle.** Inject session, dispatcher, registries and watcher services into handlers instead of installing/looking up process globals; removes #1–#8, #20–#22 and reduces #35. Files: `H/src/main.rs`, `protocol.rs`, `sessions/stream.rs`, `superagent*.rs`, `term.rs`, `lsp.rs`, watcher modules and tests. Risk: lifecycle/shutdown and multi-instance semantics; NEEDS-OWNER if behavior changes.
2. **Typed protocol and settings contracts with policy next to definitions.** Generate or contract-test TS bindings from `unipi-proto`; associate authorization and device context with operations/settings instead of independent lists; removes #9–#15, #23–#24. Files: `H/src/protocol.rs`, `settings.rs`, `host_cmd.rs`, `apps/unipi/crates/unipi-proto/src/{control,settings}.rs`, frontend host/settings clients and tests. Risk: wire compatibility and permissions; NEEDS-OWNER.
3. **Feature-owned dispatch boundaries.** Route cron, superagent and session operations through feature-level handlers rather than repeating request lists and mixing status assembly into transport; removes #12, #16–#19, #25–#27. Files: `protocol.rs`, `sessions/stream.rs`, `cron_serve.rs`, `superagent*.rs`, `host_cmd.rs`, proto and tests. Risk: dispatch and response parity; NEEDS-OWNER for any changed response.
4. **Explicit failure policy for storage and background actions.** Distinguish absent data, corrupt data and failed work; report/log errors at feature boundaries and test them; removes #36–#47. Files: `cron_serve.rs`, `term.rs`, `lsp.rs`, `notify.rs`, `update.rs`, `browser/{mod,agent_service,session}.rs`, `superagent.rs`, `extension_install.rs`, tests. Risk: errors become visible and fallback behavior changes; NEEDS-OWNER.
5. **Lifecycle signals and deterministic tests.** Replace polling where a readiness/completion signal exists and expose behavior rather than registry counters; addresses #28–#30, #48–#51, #60–#62. Files: `browser/{mod,gate,agent_service}.rs`, `lsp.rs`, `sessions/hub.rs`, `tests/{lsp,sessions_hub,term}.rs`. Risk: timing and process lifecycle; NEEDS-OWNER if deadlines or observable outcomes change.
6. **Isolate migration/compatibility and test-only surface.** Put legacy import paths behind explicit migration and narrow implementation-only public APIs; addresses #31–#34, #52–#55, #63. Files: browser profiles, notify, cron, `lsp.rs`, `term.rs`, related tests. Risk: persisted-user-data compatibility; NEEDS-OWNER before removing any migration.

## 3. Full problem list

### Hidden links

1. **[high]** `H/src/main.rs:707` installs `SessionService`, while `H/src/protocol.rs:972` and `H/src/sessions/stream.rs:600` read a process singleton; startup order and availability are implicit.
2. **[high]** `H/src/main.rs:701` installs the superagent dispatcher, while `H/src/protocol.rs:1137` retrieves it through `H/src/superagent.rs:1301`; transport behavior depends on another module's global initialization.
3. **[high]** `H/src/protocol.rs:377` accesses `PushHub::shared(&self.paths)`, whose static per-path registry is in `H/src/push.rs:297`; instance identity is a hidden path/global convention.
4. **[high]** `H/src/main.rs:918` and `H/src/term.rs:486` couple shutdown to a process-wide terminal registry not carried by the handler.
5. **[high]** `H/src/main.rs:920` and `H/src/lsp.rs:659` repeat that lifecycle coupling for language servers.
6. **[high]** `H/src/protocol.rs:620` uses the global `GitWatchers` defined at `H/src/git_watch.rs:61`; watcher ownership is not visible in `Handler`.
7. **[high]** `H/src/protocol.rs:655` uses the global board watcher at `H/src/board_cli.rs:381`; its lifetime is similarly outside the request context.
8. **[high]** `H/src/browser/mod.rs:1657` caches managers in a global keyed by path, so callers using the same path implicitly share browser state.
9. **[high]** `H/src/protocol.rs:279` duplicates remotely writable dotted setting names, while `H/src/settings.rs:124` validates settings and `apps/unipi/src/screens/settings/HostSettingsSection.tsx:149` sends one; a new key requires coordinated security, validation and UI edits.
10. **[high]** `H/src/host_cmd.rs:64` manually maps scope strings and `:74` maps layers to core enums, while `apps/unipi/crates/unipi-proto/src/settings.rs:169` describes the wire commands; enum names and parsing must remain aligned.
11. **[high]** `H/src/host_cmd.rs:235` recognizes literal `extensionSettings.permission.mode` paths independently of the core settings schema; renaming the setting also requires updating this security special case.
12. **[high]** `H/src/protocol.rs:307` enumerates cron variants to obtain device names and repeats them for dispatch at `:1145`; adding an operation requires keeping both lists synchronized.
13. **[high]** `H/src/protocol.rs:258` maintains a separate local-only request list from the `Request` definition and its handlers; new privileged operations need an extra manual authorization decision. NEEDS-OWNER: security policy.
14. **[high]** `H/src/host_cmd.rs:9` promises session-command reply shapes, but `apps/unipi/src/settings/hostCmd.ts:73` consumes a separate TS contract; Rust and frontend changes can drift.
15. **[high]** `H/src/notify.rs:302` reads `extensionSettings.notify` from the agent settings file written by another feature, with its own decoding at `:313`; configuration schema changes span extension/core/host without a typed boundary.
16. **[high]** `H/src/protocol.rs:1118` enumerates superagent requests, then `H/src/superagent.rs` handles their behavior; operation additions change transport and feature modules together.
17. **[high]** `H/src/protocol.rs:1176` groups session, memory, usage and superagent requests under the session route, with their actual branches in `H/src/sessions/stream.rs:774`; adding a feature requires edits across layers.
18. **[high]** `H/src/protocol.rs:1132` detects only status and augments its result from the session hub at `:1137`; status shape depends on transport knowing the dispatcher and hub internals.
19. **[high]** `H/src/cron_serve.rs:798` hardcodes the migration to `extensionSettings.notify.ntfy.topic`, coupling cron settings to the separate notify feature's storage layout.

### Needless links

20. **[high]** `H/src/protocol.rs:67` exposes `CronHost` in an otherwise transport-level handler, while most other services come from globals; feature lifecycle and transport composition are inconsistently coupled.
21. **[high]** `H/src/sessions/stream.rs:571` exposes a singleton service even though its constructor at `:594` receives a hub; tests and alternate hosts inherit process-wide ownership.
22. **[med]** `H/src/host_cmd.rs:32` keeps search install/build state in process-wide statics rather than a hub or search service, tying independent host instances and cancellation lifetimes together.
23. **[high]** `H/src/settings.rs:133` reaches into terminal implementation for its maximum while `H/src/protocol.rs:279` separately controls access; policy/validation depends on feature internals rather than shared setting metadata.
24. **[med]** `H/src/host_cmd.rs:242` embeds a specific permission-key convention inside generic host-command handling; generic settings commands must understand one workflow's security rule.
25. **[high]** `H/src/sessions/stream.rs:774` handles memory browsing and `:805` usage statistics within the session stream dispatcher, so independently evolving read-only features depend on session-service routing.
26. **[high]** `H/src/sessions/stream.rs:823` also handles superagent fleet/session operations, linking board orchestration to chat stream plumbing.
27. **[med]** `H/src/host_cmd.rs:91` combines search installation, index jobs, extension management and ordinary settings commands in one entry point; changes to unrelated settings workflows converge on the same dispatcher.
28. **[high]** `H/src/term.rs:584` and `:592` expose attachment and resize counters for integration tests (`H/tests/term.rs:96`, `:197`), binding the production API to testing internals.
29. **[high]** `H/src/lsp.rs:581` exposes `ServerEntry::for_test` in production code, although its observed caller is an in-module test at `:1239`.
30. **[med]** `H/src/lsp.rs:790` makes the stdout pump public although repo search found only its internal production call at `:779`; this exposes implementation mechanics unnecessarily.
31. **[high]** `H/src/notify.rs:302` supports both the full settings file and a bare test/legacy block, making production parsing carry a fixture/compatibility shape. NEEDS-OWNER before retiring legacy support.
32. **[med]** `H/src/browser/profiles.rs:256` performs legacy profile discovery in normal profile code; migration and ordinary profile access must evolve together. NEEDS-OWNER: existing profile data.

### Surplus / surface candidates

33. **[high]** `H/src/lsp.rs:581` is a public test constructor whose only found invocation is its own unit test (`:1239`); it can potentially be test-gated, not deleted as dead.
34. **[high]** `H/src/lsp.rs:790` is public but the only found call is inside the same module (`:779`); visibility can potentially be narrowed, not treated as dead.
35. **[med]** `H/src/protocol.rs:43` returns an owned device-id `String` and callers such as `:325` immediately pass borrowed views onward; review whether this conversion boundary adds ownership without value (not a claim of dead code).

Whole-repo textual searches for the specific APIs in #28–#30 included `apps`, `crates`, and `extensions`, excluding build output and dependencies; dynamic or external consumers cannot be excluded. No public item is claimed dead solely because it is public or absent from a local import list.

### Special cases

36. **[high]** `H/src/lsp.rs:250` treats unreadable or corrupt LSP config as default, obscuring why configured servers disappeared. NEEDS-OWNER.
37. **[high]** `H/src/term.rs:113` similarly defaults corrupt/unreadable terminal config and clamps the maximum, conflating missing and invalid settings. NEEDS-OWNER.
38. **[high]** `H/src/cron_serve.rs:135` hides failure loading run and job stores as an empty cache, so session ownership information silently disappears; the comment explicitly chooses availability. NEEDS-OWNER.
39. **[high]** `H/src/cron_serve.rs:245` defaults failed cron settings during test notification, potentially selecting unintended channels. NEEDS-OWNER.
40. **[high]** `H/src/cron_serve.rs:574` turns failed log reads into an empty log, indistinguishable from a successful empty log. NEEDS-OWNER.
41. **[high]** `H/src/notify.rs:320` defaults invalid notification config, obscuring parse failures and disabling configured behavior. NEEDS-OWNER.
42. **[high]** `H/src/notify.rs:645` silently treats unreadable or malformed app endpoint files as no endpoints. NEEDS-OWNER.
43. **[high]** `H/src/update.rs:495` discards the result of writing update state, so subsequent status can hide persistence failure. NEEDS-OWNER.
44. **[high]** `H/src/extension_install.rs:461` ignores rollback rename failure after installation fails, potentially leaving the prior extension displaced without reporting the rollback failure. NEEDS-OWNER.
45. **[high]** `H/src/superagent.rs:715` ignores automatic start transaction errors, making ready tasks appear not to start without a surfaced reason. NEEDS-OWNER.
46. **[high]** `H/src/browser/session.rs:901` and `:906` ignore navigation/reload CDP failures; command rejection can look like successful user action. NEEDS-OWNER.
47. **[high]** `H/src/browser/mod.rs:1994` and `H/src/main.rs:1214` discard profile migration errors before listing/managing profiles, hiding migration failure. NEEDS-OWNER.
48. **[high]** `H/src/browser/gate.rs:424` repeatedly peeks a partial HTTP head and sleeps 10 ms (`:434`) up to 200 times instead of waiting on a complete-head signal; timing affects acceptance. NEEDS-OWNER for changed deadline.
49. **[high]** `H/src/browser/mod.rs:1019` sleeps 350 ms during browser lifecycle handling before checking again, making progress depend on elapsed time. NEEDS-OWNER for lifecycle changes.
50. **[high]** `H/src/browser/agent_service.rs:225` polls for handoff with `HANDOFF_POLL`, coupling takeover latency to periodic timing. NEEDS-OWNER.
51. **[high]** `H/src/lsp.rs:873` polls child completion with 50 ms thread sleeps; lifecycle detection and resource use depend on a polling interval.
52. **[high]** `H/src/browser/profiles.rs:256` branches on a legacy profile directory and migrates it; removal can strand existing profiles. NEEDS-OWNER.
53. **[high]** `H/src/notify.rs:302` explicitly accepts a legacy/bare settings shape in addition to the current one; removal changes persisted configuration compatibility. NEEDS-OWNER.
54. **[high]** `H/src/host_cmd.rs:242` specially matches both the permission setting and its trailing-dot spelling; this unusual one-key security rule must track settings semantics. NEEDS-OWNER.
55. **[high]** `H/src/protocol.rs:279` special-cases individual remote-writable keys rather than deriving permission from setting metadata. NEEDS-OWNER: security.
56. **[high]** `H/src/memory_browse.rs:380` ignores a real-store test unless manually supplied `MEMORY_STORE`; normal test runs omit that coverage.
57. **[high]** `H/tests/fs_scope.rs:227` and `:289` are ignored tests, so their filesystem-scope cases do not run by default.
58. **[high]** `H/tests/integration.rs:634` is ignored because it needs built extension artifacts and a spawned host; default CI may miss end-to-end startup regressions.
59. **[high]** `H/tests/push.rs:493`, `H/tests/forward.rs:402`, `H/tests/sharing.rs:260`, and `H/tests/sidebar_perf.rs:52` are ignored, omitting those integration/performance cases from default execution.

## 4. Special-case count

**24 inventoried items (#36–#59)**; several aggregate related sites, so this is not a count of every occurrence. Mechanical whole-area scan of `H/src` and `H/tests` found **8 `#[ignore]` annotations**, 250 lines containing `let _ =`, 204 containing `.ok()`, 142 containing `unwrap_or_default(`, 121 containing `sleep(`, 25 containing `legacy`, and 9 containing `compat`. These are overlapping textual leads, including tests, expected absence, cleanup and comments—not 759 confirmed defects. Network timeouts and intentional throttles were not automatically classified as synchronization defects. Only the specific sites above were assessed; a complete semantic count would require reviewing all matches.

## 5. Comments / tests notes

- **[med] Comments:** roughly 5–15 obvious restatements in the sampled files, not an exhaustive count; examples `H/src/blob.rs:515` (“Send body as kind-2 frames”), `:554` (“Get it back”), and `H/src/pairing.rs:126` (“Clears any in-flight pairing attempt”). Public API docs often add useful contract information and should not be deleted indiscriminately.
- **60 [high]** `H/tests/lsp.rs:391` sleeps 300 ms before dropping the attach, coupling the test to a timing assumption instead of observing the didClose flush it describes.
- **61 [high]** `H/tests/sessions_hub.rs:531` sleeps five seconds to cross an idle window; it tests meaningful behavior but is slow and timing-sensitive rather than impossible to fail.
- **62 [high]** `H/tests/term.rs:197` asserts internal resize counters exposed at `H/src/term.rs:592`; a correct alternative implementation can break the test without breaking terminal behavior.
- **63 [med]** `H/src/local.rs:404` asserts the pipe constant against its literal and checks its prefix; it pins a compatibility contract, but alone cannot prove a client and listener interoperate.
- Rough estimate from targeted sampling: 3–10 tests show timing/internal-coupling concerns; no exhaustive assertion audit and no verified cannot-fail test. The many integration tests with behavioral assertions should not be characterized as implementation repeats.

Verification: inspected declarations, imports, concrete call sites, integration-test excerpts, cross-repo textual matches, and read-only Git history; tests/builds were not run because this is an inventory-only task. `rg` was unavailable, so `grep` was used. No source file was modified.
