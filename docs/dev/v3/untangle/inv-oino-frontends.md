# Frontend/client untangle inventory

## 1. Area summary / module map

- `oino` parses CLI/startup and selects interactive, print, RPC or ACP; `oino-bench` constructs core services to measure subagents and persistence.
- `oino-client` defines backend commands, frontend convenience methods, in-process dispatch and session following; `oino-rpc` translates those contracts to JSONL.
- `oino-acp` adapts core sessions, UI, commands and execution environments to ACP clients.
- `oino-view` holds frontend view models; `oino-tui` owns terminal state, host integration, input, rendering and effects.
- `oino-ui` owns serializable UI nodes, settings, patches and widgets; the Solid app independently consumes related protocol/UI shapes.

Scope: static inspection of the current working tree, including existing concurrent edits; no source changed or tests run. This is a prioritized, non-exhaustive inventory, not a claim that every public item, comment, or conditional was audited. Confidence refers to evidence for the coupling, not certainty that removal is desirable. History check: `git log --format= --name-only -n 400` shows formerly monolithic TUI `render.rs` (62 appearances) and `app.rs` (56), and current client `in_process.rs` (22) and `hub.rs` (20); historical paths include deleted/renamed files, and frequency alone does not establish co-change.

## 2. Candidate fixes ranked

1. **One protocol contract and generated cross-language fixtures/types.** Removes #1–#6, reduces #24–#27. Files: `oino-client/src/protocol.rs`, `oino-rpc/src/{wire,backend}.rs`, `oino-tui/src/app/mod.rs`, `apps/unipi/src/lib/session/reducer.ts` and command consumers. Risk: wire compatibility and independently deployed clients; **NEEDS-OWNER** before any wire behavior change.
2. **Make RPC event/reply ordering and failure propagation explicit.** Removes #7, #8, #21–#23, #28–#30 and reduces timing tests #47–#49. Files: `oino-client/src/{backend,follow}.rs`, `oino-rpc/src/{server,client,backend}.rs`, RPC tests. Risk: ordering, disconnect and error behavior are externally visible; **NEEDS-OWNER**.
3. **Unify settings normalization/semantics at the UI contract boundary.** Removes #9–#11 and reduces #31. Files: `oino-ui/src/settings{,/semantics}.rs`, `oino-view/src/settings_hub.rs`, TUI settings consumers, `apps/unipi/src/ui/settings/`. Risk: defaults/validation may change existing extension settings; **NEEDS-OWNER**.
4. **Separate ACP capability-backed execution from local fallback and model lifecycle.** Removes #12–#14, #32–#36. Files: `oino-acp/src/{client,env,session,factory}.rs`, ACP tests and potentially `oino-core` execution interfaces. Risk: file location, permissions, terminal status and cancellation change; **NEEDS-OWNER**.
5. **Move session/host operations behind purpose-specific client interfaces.** Removes #15–#18, reduces #37 and #38. Files: `oino-client/src/{backend,frontend,hub}.rs`, `oino-tui/src/{host,runner}.rs`, `oino/src/{startup,modes/interactive}.rs`; impact on other backend implementers. Risk: extension/frontends may rely on defaults or filesystem behavior; **NEEDS-OWNER** for behavioral changes.
6. **Make CLI and benchmark bootstrap reuse a core-owned service/tool construction boundary.** Removes #19–#20 and #39. Files: `oino/src/startup.rs`, `oino-acp/src/factory.rs`, `oino-bench/src/subagents.rs`, core service/builtin code. Risk: benchmark comparability and tool selection; owner decision if measurements change.

## 3. Full problem list

### Hidden links

1. **[high]** `crates/oino-client/src/protocol.rs:289`, `:627`, `:731`: `_oino/hello` is independently repeated in serde tagging, a names list and command-name matching; a new spelling must update all three.
2. **[high]** `crates/oino-client/src/protocol.rs:385`, `:637`, `:742`: `_oino/ui_action` repeats the same three-way maintenance obligation, with no compiler guarantee that the strings agree.
3. **[high]** `crates/oino-client/src/protocol.rs:543`, `:657`, `:762` and `apps/unipi/src/dev/mock/scenarios/queue.ts:67`: queue-edit spelling spans Rust dispatch and TS mock code without a shared definition.
4. **[high]** `crates/oino-rpc/src/wire.rs:138` and `apps/unipi/src/lib/session/reducer.ts:579`: legacy UI event shape and event name must stay aligned between the Rust producer and TS reducer.
5. **[high]** `crates/oino-rpc/src/wire.rs:205` and `apps/unipi/src/lib/session/reducer.ts:589`: v2 patch degradation and TS patch consumption evolve as one wire feature across separate implementations.
6. **[high]** `crates/oino-rpc/src/wire.rs:83` and `crates/oino-rpc/src/backend.rs:36`: the encoder drops cumulative message fields and a stateful decoder reconstructs them; changes to either event shape require coordinated updates.
7. **[high]** `crates/oino-client/src/follow.rs:3` and `crates/oino-rpc/src/server.rs:7`: reply ordering depends on synchronous event publication and draining, a cross-module timing convention rather than an explicit response barrier.
8. **[high]** `crates/oino-acp/src/session.rs:116` and `:134`: callers must register prompt waiters before sending and separately mark queued prompts, or settlement can be missed or attributed incorrectly.
9. **[high]** `crates/oino-view/src/settings_hub.rs:211` and `crates/oino-ui/src/settings.rs:478`: both parse settings descriptors from loosely typed JSON, so producer shape changes can fail silently in separate places.
10. **[high]** `crates/oino-view/src/settings_hub.rs:219` and `apps/unipi/src/ui/settings/fields.tsx:1`: TUI descriptor normalization and web field rendering are separate halves of the same extension-settings contract.
11. **[med]** `crates/oino-view/src/settings_hub.rs:638` and `crates/oino-ui/src/settings/semantics.rs:668`: settings value operations/defaults live in separate layers and require parity checks when field semantics change.
12. **[high]** `crates/oino-acp/src/client.rs:64` and `:75`: the link can be used before its session ID is bound and silently keeps the first binding, making correct call order a caller convention.
13. **[high]** `crates/oino-acp/src/session.rs:185` and `crates/oino-acp/src/env.rs:130`: associating an ACP terminal with a tool depends on the observer reconstructing identity from a spawn request.
14. **[high]** `crates/oino-acp/src/env.rs:170` and `:178`: full reads may use the client while prefix reads always use local storage, so callers implicitly depend on client and agent seeing the same files.
15. **[high]** `crates/oino-tui/src/host.rs:235` and `crates/oino/src/modes/interactive.rs:1`: session discovery and local host wiring are split between terminal integration and CLI startup, making session storage changes span layers.
16. **[high]** `crates/oino-client/src/backend.rs:33` and `crates/oino-rpc/src/server.rs:194`: bash output correlation depends on the request ID being forwarded in a special form through both layers.
17. **[high]** `crates/oino-tui/src/host.rs:59` and `:67`: density and width values are string conventions passed to render providers, so provider and frontend must agree on thresholds and spelling.
18. **[med]** `crates/oino-client/src/frontend.rs:76` and `crates/oino-client/src/protocol.rs:287`: each frontend command's typed helper and wire enum are separate places that must evolve together.
19. **[high]** `crates/oino-acp/src/factory.rs:156` and `crates/oino/src/startup.rs:7`: ACP has a separate builtin tool construction path from CLI startup, so tool registration changes require checking both.
20. **[high]** `crates/oino-bench/src/subagents.rs:272` and `extensions/unipi/subagents/extension.json:1`: benchmark setup hardcodes the subagents settings namespace/key instead of obtaining it from the feature owning that setting.

### Needless links

21. **[high]** `crates/oino-rpc/src/backend.rs:4`: a wire decoder relies on the host's `PartialAssembler` internals to reconstruct client events, coupling transport behavior to a core implementation detail.
22. **[high]** `crates/oino-client/src/backend.rs:35`: the general backend interface carries a request ID specifically for bash streaming, burdening implementations with a single workflow's correlation rule.
23. **[med]** `crates/oino-rpc/src/client.rs:294` and `crates/oino-client/src/frontend.rs:76`: RPC-specific typed command helpers coexist with generic frontend helpers, creating overlapping client surfaces for the same backend capability.
24. **[high]** `crates/oino-tui/src/app/mod.rs:1005`: app state directly parses raw response field names and defaults instead of consuming a typed reply, linking presentation to protocol representation.
25. **[high]** `crates/oino-tui/src/app/mod.rs:1048`: command-list presentation independently knows the `commands` reply field and JSON shape.
26. **[high]** `crates/oino-tui/src/app/mod.rs:1078`: model presentation independently knows the `models` reply field and JSON shape.
27. **[high]** `crates/oino-tui/src/app/mod.rs:1214`: queue presentation independently knows `steering` and `followUp` response fields.

### Surplus / shallow boundaries

28. **[med]** `crates/oino-client/src/backend.rs:35`: the default `send_with_id` discards its extra parameter and simply delegates to `send`, so implementers can appear to support correlation without implementing it.
29. **[med]** `crates/oino-client/src/backend.rs:66`: the default `set_approval_ui` is a silent no-op, making the extension point look supported even when a transport cannot honor it.
30. **[med]** `crates/oino-client/src/backend.rs:89`: the default never-resolving shutdown future hides an unsupported lifecycle operation behind the common interface.
31. **[med]** `crates/oino-view/src/settings_hub.rs:513` and `:520`: JSON-facing wrappers repeatedly turn descriptors into view models, retaining an extra untyped entry surface alongside typed descriptors.
32. **[med]** `crates/oino-acp/src/client.rs:131` and `:139`: per-method JSON-to-SDK adapters duplicate wire field construction around SDK calls; SDK non-exhaustiveness may justify some of this boundary.
33. **[med]** `crates/oino-acp/src/factory.rs:98`: a general options-producing closure is exposed to construct per-cwd services, while ACP then overrides mode/UI internally; a narrower factory input could communicate the actual variation.

No dead-public-item claim is made: absence of a local caller is insufficient, and a whole-repository symbol-by-symbol search including apps and extensions was not completed.

### Special cases (20 inventoried occurrences)

34. **[high]** `crates/oino-rpc/src/server.rs:290`: a one-second sleep releases a held `agent_start`, making a protocol ordering invariant depend on elapsed time. **NEEDS-OWNER**.
35. **[high]** `crates/oino-rpc/src/server.rs:300`: shutdown waits at most 500 ms for forwarding and ignores timeout, so trailing events may be lost. **NEEDS-OWNER**.
36. **[high]** `crates/oino-rpc/src/server.rs:219`: output write/flush errors only set `broken`, while `run` documents only input errors; callers cannot distinguish failed output. **NEEDS-OWNER**.
37. **[high]** `crates/oino-rpc/src/server.rs:380`: writer task join failure is discarded, suppressing a potential failure of the RPC output path.
38. **[high]** `crates/oino-rpc/src/backend.rs:64`: malformed assistant deltas are converted to `None`, indistinguishable from an intentionally ignored non-event.
39. **[high]** `crates/oino-rpc/src/backend.rs:79`: malformed client events are likewise silently dropped, allowing protocol drift to look like missing updates.
40. **[high]** `crates/oino-tui/src/app/mod.rs:1005`: malformed session entries become an empty list, conflating parse failure with no entries.
41. **[high]** `crates/oino-tui/src/app/mod.rs:1078`: malformed model replies become an empty list, concealing a protocol or backend error.
42. **[high]** `crates/oino-tui/src/app/mod.rs:1214`: malformed queue fields become empty queues, potentially presenting data loss as an empty state.
43. **[high]** `crates/oino-tui/src/render/mod.rs:201`: invalid UI node JSON is discarded with `.ok()`, hiding the rendering cause from the user.
44. **[high]** `crates/oino-acp/src/client.rs:77`: rebinding an already bound session ID ignores the `OnceLock::set` error, masking lifecycle misuse.
45. **[high]** `crates/oino-acp/src/env.rs:297`: terminal wait errors are converted to no exit status; the command outcome can therefore hide a client failure. **NEEDS-OWNER**.
46. **[high]** `crates/oino-acp/src/env.rs:305`: final terminal-output failures are ignored, potentially dropping last output without diagnosis.
47. **[high]** `crates/oino-tui/tests/frontend.rs:80`: a fixed 170 ms sleep coordinates a test with asynchronous work rather than waiting for a defined event.
48. **[high]** `crates/oino-rpc/tests/ui_wire.rs:71`: a fixed 50 ms sleep stands in for a UI event/handshake barrier.
49. **[high]** `crates/oino-client/tests/hub.rs:569`: a two-second sleep makes the test sensitive to scheduler speed and adds fixed latency.
50. **[high]** `crates/oino-rpc/src/wire.rs:133`: plain clients receive legacy event filtering and unsupported dialogs are cancelled, requiring parallel protocol behavior. **NEEDS-OWNER**.
51. **[high]** `crates/oino-rpc/src/server.rs:183`: malformed or absent v2 capabilities fall back to the v1 connection interpretation, obscuring bad handshakes. **NEEDS-OWNER**.
52. **[high]** `crates/oino-acp/src/env.rs:298`: ACP terminal output uses 100 ms polling rather than a notification/barrier; this may be required by ACP, so replacing it depends on client capability. **NEEDS-OWNER** if latency changes.
53. **[high]** `crates/oino-tui/src/runner.rs:298`: failure to launch the URL opener is discarded, making the action appear successful when it failed.

## 4. Special-case count

20 numbered examples (#34–#53), not an area-wide total. A broad textual scan found hundreds of syntactic hits, including legitimate `Option` defaults, infallible formatting, shutdown cleanup and fixtures; these are not all special cases or swallowed failures. No `#[ignore]` match appeared in the scoped `src`/`tests` scan; conditional runtime skips were not exhaustively audited. Polling for an external ACP protocol (#52) is recorded separately from test synchronization and may be unavoidable.

## 5. Comments / tests notes

- Comments: rough sampled estimate **dozens** of restatements, not a full count. Examples: `crates/oino-rpc/src/server.rs:33` (“Reply queue depth” above `REPLY_QUEUE`), `crates/oino-acp/src/client.rs:29` (`fs.readTextFile` above `read_text_file`), and `crates/oino-client/src/frontend.rs:24` (“Action id” above `id`). Contract, ordering, and compatibility comments should not be removed merely for brevity.
- Tests: **3 representative timing-coupled cases** are #47–#49; there are additional sleeps in `crates/oino-client/tests/hub.rs:233` and `crates/oino-rpc/tests/ui_wire.rs:109`. Timing alone does not prove a test cannot fail; no fully non-failing test is claimed without deeper inspection.
- Test design risk: `crates/oino-rpc/src/wire.rs:519` round-trips the encoder through the decoder, useful but capable of missing a matching mistake in both; retain it and add externally authored wire fixtures. `crates/oino-ui/src/patch.rs:1241` similarly round-trips its own JSON operations, which alone cannot establish interoperability.
- Validation: line references and examples were inspected statically; no builds/tests were run for this documentation-only inventory. Other agents may move lines or change code while this document is being written.
