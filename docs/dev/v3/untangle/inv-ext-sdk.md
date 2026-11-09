# Extension SDK + providers inventory

## 1. Area summary

- `wit/oino-extension` defines component worlds; the published extension SDK embeds a second WIT tree.
- `oino-extension` wraps generated guest bindings; `oino-extension-macros` generates tool adapters.
- `oino-provider-kit` owns provider requests, wire codecs, streams, auth helpers and guest I/O.
- `extensions/providers` supplies catalogs, component registration, credentials and provider-specific OAuth.
- `unipi-kit` supplies suite conventions, topics, settings, services and shared extension workflows; fixtures exercise host contracts.

Scope: working-tree snapshot, including concurrent uncommitted changes; findings are opportunities, not confirmed runtime failures. History check: `git log --format= --name-only -n 400 -- <area>` shows SDK lib (12 appearances), SDK manifest (10), WIT worlds (9), UI SDK (9), and echo fixture (8); counts are per-file appearances, not co-change proof. No source edits or test runs. Paths below are repo-relative. Confidence describes the observed coupling or behavior, not the merit of deleting a compatibility feature.

## 2. Candidate fixes ranked

1. **Generate shared contracts across component, host and UI.** Candidate fix: make WIT plus a versioned JSON schema the sources for event names, UI operations, provider requests and topic payloads; removes #1–#8 and strengthens the checks described in §5. Files: `wit/`, SDK WIT copy and sync tests, `crates/oino-extension-api`, core/view/RPC UI, provider request, `unipi-kit`, `apps/unipi/src/lib/status`. Risk: large, cross-language compatibility and public SDK; **NEEDS-OWNER** for wire changes. Preserve packaging and standalone crate builds.
2. **Own provider policy with the provider adapter.** Candidate fix: move endpoint/auth/model-list policy and provider-only parser behavior behind explicit provider-owned preparation/stream hooks, not a central ID switch; addresses #9–#12, #19, #20, #25–#28 and #47–#50. Files: provider kit `providers.rs`, `guest/mod.rs`, `exchange.rs`, provider components and catalogs. Risk: request ordering, retry, wire parity and component size; **NEEDS-OWNER** for visible model/auth behavior. Provider-specific branches may be required for upstream compatibility.
3. **Make failures observable at capability boundaries.** Candidate fix: return structured errors from directory creation, bus/settings/JSON conversion and OAuth setup, retaining explicitly named best-effort APIs only where intended; removes #21–#24, #29–#38. Files: extension SDK fs/bus/events/tool, unipi-kit bus/paths/settings/tray/jev, provider guest OAuth and ChatGPT, callers. Risk: public API and fail-open behavior change; **NEEDS-OWNER** especially for permissions/judging and login.
4. **Validate catalog registration once.** Candidate fix: share a minimal fallible catalog loader and register only after validation, replacing repeated default-on-error setup; removes #14–#16 and #39. Files: all seven provider `src/lib.rs`, provider-kit catalog/guest, tests. Risk: malformed bundled catalogs become startup errors instead of empty model lists; **NEEDS-OWNER**.
5. **Consolidate contract checks and retire copies only with evidence.** Candidate fix: cross-boundary fixture tests and explicit compatibility lifecycle for event routing, WIT packaging, OAuth and legacy reasoning; addresses #17, #18 and #46, and improves the checks described in §5. Files: SDK tests, host probe tests, provider fixtures, compatibility parser and suite metadata. Risk: compatibility removal can break old transcripts/hosts; **NEEDS-OWNER** before removal. WIT mirror is intentional for publishing and must remain available in the package.

## 3. Full problem list

### Hidden links

1. **[high]** `wit/oino-extension/worlds.wit:1` and `extensions/sdk/oino-extension/src/lib.rs:81` use separate canonical and packaged WIT trees; an interface edit must be copied even though bindings compile from only the latter.
2. **[high]** `wit/oino-extension/registry.wit:6`, `crates/oino-extension-api/src/registry.rs:41`, and `extensions/sdk/unipi-kit/src/bus.rs:110` independently enumerate event identities; additions must update all representations or routing diverges.
3. **[high]** `extensions/sdk/oino-extension/src/events.rs:63` derives wire names from generated Rust `Debug` casing, while `wit/oino-extension/registry.wit:7` specifies kebab-to-snake conversion; code generation naming is an implicit protocol dependency.
4. **[high]** `extensions/sdk/oino-extension/src/ui.rs:186` and `crates/oino-core/src/ui/registry.rs:1153` independently spell `setStatus` and its fields, so producer and receiver changes are not compiler-linked.
5. **[high]** `extensions/sdk/oino-extension/src/ui.rs:210`, `crates/oino-core/src/ui/registry.rs:1154`, and `crates/oino-view/src/ui_state.rs:739` separately encode `setWidget`, spanning SDK, host and client.
6. **[high]** `extensions/sdk/unipi-kit/src/topics.rs:57` and `apps/unipi/src/lib/status/lh.ts:12` duplicate long-horizon topic names and `topics.rs:380` and `lh.ts:23` independently define the payload shape.
7. **[high]** `extensions/sdk/unipi-kit/src/topics.rs:97` and `crates/oino-core/src/topics.rs:23` separately define the settings topic prefix, so a rename silently disconnects subscribers.
8. **[high]** `wit/oino-extension/provider.wit:5` documents JSON request fields while `extensions/sdk/oino-provider-kit/src/request.rs:14` separately deserializes them; WIT type-checks only opaque JSON.
9. **[high]** `extensions/sdk/oino-provider-kit/src/guest/mod.rs:23` separately enumerates provider environment names consumed by provider policy (`providers.rs:18`); a new variable also needs host permissions in component manifests.
10. **[high]** `extensions/sdk/oino-provider-kit/src/providers.rs:16`, `extensions/providers/openai-compat/src/lib.rs:70`, and `extensions/sdk/oino-provider-kit/src/responses.rs:705` duplicate the Copilot ID despite an existing constant.
11. **[high]** `extensions/sdk/oino-provider-kit/src/providers.rs:27` and `:59` split Azure request adaptation into before- and after-payload stages; callers must preserve the sequencing convention.
12. **[high]** `extensions/sdk/oino-provider-kit/src/guest/mod.rs:183` invokes the host request hook before finalization and signing at `:186–194`; changing request assembly in one layer requires knowing this ordering.
13. **[medium]** `extensions/sdk/unipi-kit/src/suite.rs:69` duplicates extension identifiers and directory/namespace metadata already distributed in extension manifests, creating a manual suite synchronization point.
14. **[high]** `extensions/providers/anthropic/src/lib.rs:21` and `:79` couple included catalog content to registration and list-models, but validation occurs only lazily on first use.
15. **[high]** `extensions/providers/openai-compat/src/lib.rs:26` and `:96` repeat the same catalog-to-registration convention separately from Anthropic and the other provider components.
16. **[medium]** `extensions/sdk/unipi-kit/src/services.rs:22` defines `work-tray.tab` while `apps/unipi/src/dev/mock/scenarios/status.ts:91` independently matches it; mocks can drift from the actual service contract.
17. **[high]** `extensions/sdk/oino-extension/tests/wit_sync.rs:33` checks two WIT copies only in a monorepo and returns at `:37` without comparison in the packaged crate; package consumers rely on a prior repository check.
18. **[medium]** `extensions/sdk/unipi-kit/src/paths.rs:38` and `:44` assume host preopen names `/tmp` and `/cache`; availability and spelling depend on host setup outside the helper.

### Needless links

19. **[high]** `extensions/sdk/oino-provider-kit/src/providers.rs:28` puts Copilot, Cloudflare and Azure policy in one general adapter, so independent provider changes edit shared dispatch.
20. **[high]** `extensions/sdk/oino-provider-kit/src/exchange.rs:65` adds `emits_start` to every stream parser for Bedrock's one start-event timing rule.
21. **[medium]** `extensions/sdk/unipi-kit/src/jev.rs:360` combines transport, credential resolution and fail-open decision semantics, coupling infrastructure failure to the judge workflow's policy.
22. **[medium]** `extensions/sdk/unipi-kit/src/bus.rs:23` makes all callers inherit a single ignored-publish-error policy instead of choosing whether delivery is required.
23. **[high]** `extensions/sdk/oino-extension/src/tool.rs:65` couples a general details builder to silent serialization failure; callers cannot distinguish omitted details from bad details.
24. **[high]** `extensions/sdk/unipi-kit/src/tray.rs:112` combines heterogeneous response normalization, ownership tagging and silent invalid-item filtering, so consumers cannot choose strict decoding.
25. **[high]** `extensions/sdk/oino-provider-kit/src/request.rs:53` collects Azure, Vertex, Bedrock, Anthropic and Codex-only options in one general request extension, tying unrelated provider option changes together.

### Surplus / shallow interfaces

26. **[medium]** `extensions/sdk/oino-provider-kit/src/exchange.rs:73` adds a default parser hook specifically for Bedrock failure diagnostics; consider a provider-specific diagnostic strategy rather than extending every parser.
27. **[medium]** `extensions/sdk/oino-provider-kit/src/guest/mod.rs:53` exposes a wire override for dual-wire gateways; verify current consumers before keeping this generic configuration (public SDK consumers may exist outside the repo).
28. **[medium]** `extensions/sdk/oino-provider-kit/src/providers.rs:86` exposes Copilot dynamic headers in shared provider policy while immediately returning empty for other providers; moving it next to the adapter could shrink the general interface.
29. **[medium]** `extensions/sdk/unipi-kit/src/paths.rs:20` and `:26` are thin joins over SDK path helpers; their value is convention naming, so consolidate only if callers do not need that abstraction.
30. **[medium]** `extensions/sdk/oino-extension/src/fs.rs:50` wraps one generated call and maps its result; this is a shallow facade but may be warranted to hide binding types in the public SDK.

No public item is asserted dead: a repository-wide name search cannot establish absence of external SDK users, and no candidate was verified sufficiently to label it dead.

### Special cases

31. **[high]** `extensions/sdk/oino-extension/src/fs.rs:27` discards workspace directory creation errors and returns the path anyway; **NEEDS-OWNER** if propagated.
32. **[high]** `extensions/sdk/oino-extension/src/fs.rs:45` does the same for session directories, delaying or obscuring the failure; **NEEDS-OWNER**.
33. **[high]** `extensions/sdk/oino-extension/src/bus.rs:25` turns malformed sticky JSON into absence, concealing a producer/host contract error.
34. **[high]** `extensions/sdk/oino-extension/src/events.rs:51` converts failed event-result serialization to `None`, indistinguishable from no handler result.
35. **[high]** `extensions/sdk/oino-extension/src/command.rs:193` treats malformed flag JSON as a missing flag.
36. **[high]** `extensions/sdk/unipi-kit/src/settings/values.rs:240` converts any settings host error into `None`, losing the reason a layer is unavailable.
37. **[high]** `extensions/sdk/unipi-kit/src/paths.rs:108` and `:119` collapse permission, I/O and malformed JSON errors into missing data; their documented best-effort semantics may be intentional.
38. **[high]** `extensions/sdk/unipi-kit/src/jev.rs:351` and `:368` turn credential/HTTP failures into `None`, while `:513–521` also collapses timeout and malformed answers; callers explicitly fail open, so changing it is **NEEDS-OWNER**.
39. **[high]** `extensions/providers/anthropic/src/lib.rs:32` defaults a bad embedded catalog to empty; the same pattern appears in all seven provider components, hiding registration defects; **NEEDS-OWNER** if startup begins failing.
40. **[high]** `extensions/sdk/oino-provider-kit/src/guest/mod.rs:65` and `:79` suppress host environment access errors, making denied or broken access look like unset configuration.
41. **[medium]** `extensions/sdk/oino-provider-kit/src/guest/mod.rs:187` substitutes an empty request body on JSON serialization error rather than emitting a stream error; determine whether the input value can actually fail serialization.
42. **[high]** `extensions/providers/openai-responses/src/chatgpt.rs:55` ignores failure to persist the installation ID, so future calls can generate a different ID; **NEEDS-OWNER** for error policy.
43. **[high]** `extensions/sdk/oino-provider-kit/src/guest/oauth.rs:112` ignores browser-open and notification errors; a user may receive neither login URL nor instructions.
44. **[high]** `extensions/sdk/oino-provider-kit/src/guest/oauth.rs:163` and `:182` ignore callback HTTP response failures, allowing login to continue while the browser receives no confirmation.
45. **[high]** `extensions/sdk/oino-provider-kit/src/guest/oauth.rs:209` and `:214` translate canceled manual input to an empty string rather than preserving cancellation.
46. **[high]** `extensions/sdk/oino-provider-kit/src/completions.rs:349` and `:565` preserve legacy encrypted tool-call signatures alongside current reasoning details; removal risks replaying stored transcripts incorrectly and is **NEEDS-OWNER**.
47. **[high]** `extensions/sdk/oino-provider-kit/src/guest/mod.rs:240` retries Anthropic strict-schema limit failures with preferred strict disabled; it is a provider compatibility fallback with potentially different validation semantics, **NEEDS-OWNER**.
48. **[high]** `extensions/sdk/oino-provider-kit/src/guest/mod.rs:382` filters Copilot models and `:385` personalizes OpenRouter models by ID in shared listing code; provider policies must evolve with those services.
49. **[high]** `extensions/sdk/oino-provider-kit/src/providers.rs:36` and `:49` special-case Cloudflare and Azure endpoint/auth rules in shared preparation; these are likely necessary upstream differences, not automatically dead.
50. **[high]** `extensions/providers/openai-compat/src/lib.rs:70` and `:85` repeat a Copilot-only OAuth guard in login and refresh, so adding an OAuth provider requires changing both.
51. **[high]** `extensions/sdk/oino-provider-kit/src/guest/oauth.rs:244` uses timed polling for device authorization; this is protocol-driven polling rather than evidence of a synchronization workaround, and should not be removed without protocol review.
52. **[high]** `extensions/sdk/unipi-kit/src/jev.rs:517` races a model request against a timer; this is a workflow deadline rather than proven synchronization, but timed-out decisions are indistinguishable from other failures (#38).

## 4. Special-case count

22 numbered observations (#31–#52), not 22 individual occurrences: 14 error-suppression/cancellation observations (#31–#38 and #40–#45, with #37/#38 each aggregating multiple sites), one catalog-default cluster (#39), one legacy branch (#46), four provider-specific branch clusters (#47–#50), and two timed workflows (#51–#52). Specifically, suppression/cancellation items are #31–#38 and #40–#45: **14**, so the categories total 14 + 1 + 1 + 4 + 2 = **22**. Counts are an inventoried lower bound, not a repo-wide exhaustive count; test-only defaults and infallible string formatting were excluded. A textual search found no `#[ignore]` in area Rust files; conditional skips and non-Rust tests were not exhaustively audited. Polling and deadlines are included for review, not asserted defective.

## 5. Comments and tests notes

- **Comments:** roughly 10–30 restatement candidates in the sampled SDK, not a full count. Examples: `extensions/sdk/oino-extension/src/fs.rs:11` says “Every preopen” above `preopens`; `src/tool.rs:63` says “Adds details” above `with_details`; `src/command.rs:189` says “Current value of a flag” above `get_flag`. Public API documentation can still be valuable; do not bulk-delete.
- **Tests:** roughly 3–6 sampled tests/branches are implementation-coupled or conditionally ineffective, not an exhaustive count. `extensions/sdk/unipi-kit/src/suite.rs:148` repeats the exact `LOAD_ORDER` literal from `:69–89`, so intentional reordering requires updating both without checking actual extension behavior; `src/topics.rs:488` asserts a fixed count of the manually maintained topic table; `extensions/sdk/oino-extension/tests/wit_sync.rs:37` explicitly returns without asserting in packaged contexts (#17). These tests can fail in the monorepo and are not classified as universally unable to fail.
- **Coverage direction:** add producer-to-consumer JSON contract tests and package-artifact WIT checks; retain provider oracle fixtures (`extensions/sdk/oino-provider-kit/tests/pi_fixtures.rs`) as independent behavioral evidence rather than replacing them with implementation-derived expectations.
- **Limitations:** inspected signatures, targeted cross-repo references, selected branch bodies and history frequency; not every file, public symbol, TS schema, fixture, comment or test was exhaustively audited. No builds or tests run because this is an inventory-only snapshot and the tree is concurrently changing.
