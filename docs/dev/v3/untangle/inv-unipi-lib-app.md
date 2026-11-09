# UniPi lib/UI/screens and app crates: untangle inventory

## 1. Area summary

- Solid frontend: `src/lib` owns state/transports, `src/ui` shared widgets, `src/screens` workflows; `src/settings` and `src/hosts` provide contexts.
- `packages/protocol` contains TS/Zod wire definitions; `crates/unipi-proto` separately owns Rust wire types.
- `src-tauri` adapts desktop commands and streams to host operations; `src/dev/mock` substitutes a host for tests/captures.
- `unipi-kanboard` owns board files/CLI; `unipi-cron` owns scheduling/storage, with `unipi-cron-schedule` parsing schedules.
- `unipi-browser` implements Chromium/CDP and `unipi-browser-protocol` its browser contract; features also cross into frontend components and host/extension code.

Method: signature/import skeleton, targeted reads and whole-area pattern searches, plus read-only `git log --format= --name-only -n 400`; a frequency count is not proof of co-change. The tree is concurrently changing; line numbers describe the inspected snapshot. This is a targeted inventory, not an exhaustive audit of every file. High confidence means the cited coupling/branch is present, not that removing it is automatically safe. No dead-public-item claim is made without whole-repository usage verification.

## 2. Candidate fixes ranked

1. **One authoritative wire contract and generated/contract-tested adapters**: addresses #1–#11, #19, #20, #28. Files: `packages/protocol/src/*`, `crates/unipi-proto/src/*`, `src/lib/cron.ts`, `src/lib/host/tauri.ts`, `src-tauri/src/chat.rs`, relevant host dispatch/tests. High blast radius; retain wire compatibility and runtime validation; generation technology and version policy need owner decision. NEEDS-OWNER if compatibility changes.
2. **Board service owns rules and task DTOs**: addresses #12–#14, #21, #22, #29. Files: `src/lib/board/{api,transport,dashboard}.ts`, `crates/unipi-kanboard/src/{transitions,commands,model,run}.rs`, host bridge and board tests. Medium/high blast radius; preserve offline/remote behavior and permissions. NEEDS-OWNER for changing UI gating, batching or CLI contract.
3. **Make scheduler persistence failures explicit at one boundary**: addresses #30–#37, #45. Files: `unipi-cron/src/{scheduler,store,embedded,exec,control}.rs` and tests. Medium/high risk: scheduler and user-visible run status can change; choose failure policy with owner. NEEDS-OWNER.
4. **Host identity and transport composition as explicit lifecycle**: addresses #15–#18, #23–#25, #38–#40. Files: `src/hosts/context.tsx`, `src/lib/host/{factory,localId,tauri,client}.ts`, `src/main.tsx`, affected consumers and tests. Medium/high risk: routing, reconnects and mocks; preserve existing aliases and liveness. NEEDS-OWNER if user-visible recovery changes.
5. **Browser download/permission boundary with explicit outcomes**: addresses #41–#44, #46, #47. Files: `unipi-browser/src/{profile,permissions,launch,page}.rs`, browser frontend/protocol and tests. Medium/high risk to downloads, grants and automation timing; choose failure semantics with owner. NEEDS-OWNER.
6. **Define board read/placement failure policy**: addresses #48–#51. Files: `unipi-kanboard/src/{board,commands,attachments,order}.rs` and CLI/UI tests. Medium risk to existing boards and ordering; NEEDS-OWNER for user-facing error policy.
7. **Version settings migrations explicitly**: addresses #26, #27, #52. Files: `src/settings/{schema,load,migrations}.ts` and settings tests. Medium risk to existing customized settings; preserve old blobs. NEEDS-OWNER before changing migration behavior.
8. **Test seams at observable boundaries**: addresses #53–#56; files: cited tests, test fixtures and integration harness. Moderate harness cost; keep intentionally manual live tests if infrastructure is unavailable.

## 3. Full problem list

Paths below are relative to `apps/unipi/`. Candidate fixes describe opportunities, not instructions to delete compatibility safeguards.

### Hidden links

1. **[high]** `crates/unipi-proto/src/cron.rs:45` and `packages/protocol/src/cron.ts:14` independently specify four cron limits; a limit change requires synchronized edits across languages.
2. **[high]** `crates/unipi-proto/src/cron.rs:1` and `packages/protocol/src/cron.ts:54` mirror cron job wire shapes while `crates/unipi-cron/src/model.rs:72` owns persisted jobs, making job evolution a three-way contract.
3. **[high]** `crates/unipi-proto/src/fs.rs:21` and `packages/protocol/src/fs.ts:8` independently validate filesystem paths and search inputs, so validation changes must be coordinated.
4. **[high]** `crates/unipi-proto/src/git.rs:32` and `packages/protocol/src/git.ts:9` duplicate git bounds, allowing frontend and host limits to drift.
5. **[high]** `crates/unipi-proto/src/push.rs:24` and `packages/protocol/src/push.ts:13` separately encode push wait limits and ID validation.
6. **[high]** `crates/unipi-proto/src/sharing.rs:33` and `packages/protocol/src/sharing.ts:19` repeat sharing limits and feature variants, so new features must be added twice.
7. **[high]** `crates/unipi-proto/src/settings.rs:35` and `packages/protocol/src/settings.ts:21` repeat the secret mask and host setting key list, tying settings changes to two unlinked definitions.
8. **[high]** `crates/unipi-proto/src/browser.rs:58` and `packages/protocol/src/browser.ts:33` independently maintain browser limits, tying browser request validation across languages.
9. **[high]** `crates/unipi-proto/src/board.rs:18` and `packages/protocol/src/board.ts:14` duplicate board command/slug limits and need coordinated edits.
10. **[high]** `crates/unipi-proto/src/session.rs:27` and `packages/protocol/src/session.ts:36` separately define session bounds, leaving the protocol constraint implicit across languages.
11. **[high]** `src/lib/host/tauri.ts:130` and `src-tauri/src/chat.rs:57` independently declare `SessionMsg` variants; channel evolution requires manually updating both.
12. **[high]** `src/lib/board/api.ts:224` mirrors `crates/unipi-kanboard/src/transitions.rs:1` user transition rules; backend policy and UI move affordances can diverge.
13. **[high]** `src/lib/board/api.ts:62` independently declares CLI task fields backed by `crates/unipi-kanboard/src/model.rs`; output schema changes require coordinating UI and CLI.
14. **[high]** `src/lib/board/api.ts:10` derives project/dashboard state from CLI calls while `crates/unipi-kanboard/src/commands.rs` implements the data, spreading one board behavior across layers.
15. **[high]** `src/hosts/context.tsx:74` populates the global in `src/lib/host/localId.ts:17`, which `src/lib/host/tauri.ts:64` consumes; host identity depends on initialization order and fallback lookup.
16. **[high]** `src/lib/host/factory.ts:20` stores a process-global override/cache and `:30` reads `window.__UNIPI_HOST__`, so the capture bootstrap must secretly install a factory before consumers request clients.
17. **[high]** `src/lib/host/tauri.ts:88` chooses the liveness listener based on whether local identity is already known, tying construction timing to later connectivity reporting.
18. **[medium]** `src/lib/openLocal.ts:13` treats only literal `local` as own-PC while `src/lib/host/localId.ts:26` resolves that alias to a real ID; callers must know which representation the file-opening workflow expects.
19. **[high]** `src/lib/cron.ts:68` hardcodes operation names also described in `crates/unipi-proto/src/cron.rs:18`; neither string is checked against the other by the type system.
20. **[high]** `src/lib/host/tauri.ts:73` converts dotted superagent operation names and maintains a special JSON-argument list separately from `src-tauri/src/chat.rs:737` command registrations.

### Needless links

21. **[high]** `src/lib/board/api.ts:20` exposes UI board calls through the CLI transport, so CLI argument/output evolution directly affects screen-facing APIs.
22. **[medium]** `src/lib/board/api.ts:13` implements bulk lane actions as individual moves, coupling UI behavior to per-task CLI workflow rather than a bulk operation boundary.
23. **[high]** `src/lib/host/tauri.ts:74` makes the generic `request` path know one superagent workflow's argument envelope and operation inventory.
24. **[high]** `src/lib/host/tauri.ts:145` makes the generic session-open adapter know the single special `superagent` session command alongside ordinary session opens.
25. **[medium]** `src/lib/host/factory.ts:8` imports the concrete Tauri implementation into the globally used factory, tying otherwise generic consumers to desktop composition.
26. **[medium]** `src/settings/load.ts:110` runs two unversioned, input-specific migrations before the general migration registry, so the generic loader must understand particular historical settings changes.
27. **[medium]** `src/settings/migrations.ts:39` compares an entire historical toolbar default to determine customization, tying a general migration path to one particular legacy value.

### Surplus candidates

28. **[medium]** `src/lib/cron.ts:67` through `:120` is a series of one-call wrappers around stringly `hostCall`; a typed contract-generated client could replace these without losing the useful feature seam.
29. **[medium]** `src/lib/board/api.ts:366` wraps a constant mirror in an immediately resolved promise to preserve an older asynchronous rules interface; whether that compatibility is still needed requires caller review.

### Special cases and error policy

30. **[high]** `crates/unipi-cron/src/scheduler.rs:313` defaults settings on any load error, making corrupt/unreadable settings indistinguishable from defaults. NEEDS-OWNER.
31. **[high]** `crates/unipi-cron/src/scheduler.rs:392` discards a run-save error; run state may be missing from durable history. NEEDS-OWNER.
32. **[high]** `crates/unipi-cron/src/scheduler.rs:425` independently discards another run-save failure on a separate path. NEEDS-OWNER.
33. **[high]** `crates/unipi-cron/src/scheduler.rs:502` ignores run persistence at execution transition, so durable history can lag execution. NEEDS-OWNER.
34. **[high]** `crates/unipi-cron/src/scheduler.rs:506` ignores failure to disable a job after a run; the scheduling policy's intended state may not persist. NEEDS-OWNER.
35. **[high]** `crates/unipi-cron/src/scheduler.rs:518` ignores saving the final run, potentially losing the result. NEEDS-OWNER.
36. **[high]** `crates/unipi-cron/src/store.rs:364` treats any log read failure as an empty log before writing, not just a missing file. NEEDS-OWNER.
37. **[high]** `crates/unipi-cron/src/embedded.rs:116` intentionally falls back to interval scanning if watcher creation fails and `:118` ignores watch registration failure; the degradation is invisible here.
38. **[high]** `src/lib/host/tauri.ts:47` suppresses local-host lookup errors at `:53`, then `:66` sends the unresolved alias; the source failure is obscured.
39. **[high]** `src/lib/pairPhone.ts:63` polls every 800 ms and `:73` suppresses poll errors, allowing the waiting state to persist without a visible cause. NEEDS-OWNER.
40. **[high]** `src/lib/openLocal.ts:20` caches a failed editor lookup as an empty list for the session, preventing retry and hiding the failure. NEEDS-OWNER.
41. **[high]** `crates/unipi-browser/src/profile.rs:810` ignores download directory creation errors and `:814` ignores copy errors before `:815` deletes the source and records the destination; failed fallback copies can lose downloads. NEEDS-OWNER.
42. **[high]** `crates/unipi-browser/src/permissions.rs:14` treats unreadable or malformed grants as empty grants, hiding the distinction between no permissions and storage failure. NEEDS-OWNER.
43. **[high]** `crates/unipi-browser/src/launch.rs:322` waits for DevTools readiness by polling a file every 100 ms up to 200 times; startup synchronization is timing-dependent.
44. **[high]** `crates/unipi-browser/src/page.rs:320` substitutes default layout when layout retrieval fails, so downstream page behavior can proceed with missing geometry.
45. **[high]** `crates/unipi-cron/src/scheduler.rs:684` polls active runs every 200 ms for shutdown rather than waiting on a completion signal; shutdown latency is timer-dependent.
46. **[high]** `crates/unipi-browser/src/profile.rs:108` ignores failure to create the temporary download directory, deferring its diagnosis to later browser operations.
47. **[high]** `src/lib/browser.ts:344` silently ignores `browser_stop` failure on cleanup, making a failed stop unobservable at this boundary.
48. **[high]** `crates/unipi-kanboard/src/board.rs:120` maps every archived-task read error to no dependency, conflating missing and unreadable data. NEEDS-OWNER.
49. **[high]** `crates/unipi-kanboard/src/commands.rs:154` ignores all project load errors before re-registration, although `:152` explains that retaining the old ID counter prevents overwrites. NEEDS-OWNER.
50. **[high]** `crates/unipi-kanboard/src/commands.rs:862` converts placement lookup errors to `None`, conflating an invalid target with a lane needing rebalancing.
51. **[high]** `crates/unipi-kanboard/src/attachments.rs:212` filters out failed attachment descriptions, so a partial listing does not expose which entries failed. NEEDS-OWNER.
52. **[high]** `src/settings/migrations.ts:48` upgrades only a toolbar exactly equal to one old default, an intentional compatibility branch that needs a retention policy before removal. NEEDS-OWNER.

## 4. Special-case count

23 individually recorded special-case findings (#30–#52), including two timing-based synchronization sites; these are **not** a total count for the area. A broad grep found **841 matching lines** across the requested Rust/TS source directories for `let _ =`, `.ok()`, `unwrap_or_default()`, `#[ignore]`, `tokio::time::sleep`, and selected catch forms. This is a noisy, non-deduplicated indicator: it includes tests, benign `write!` to strings, optional values, cleanup, intentional debounce, and excludes other spelling/patterns; it is not 841 defects or a complete special-case census. Two `#[ignore]` matches are recorded below. Legacy/compat migration examples are #26, #27 and #52; compatibility is not presumed obsolete.

## 5. Comments and tests notes

- **Comments:** rough estimate from this sampled inspection: at least 3 restatements, not a whole-area count. `src/lib/host/factory.ts:50` restates `hostCall`'s body, `:55` restates the app-level wrapper, and `src/settings/load.ts:108` repeats the read/migrate/validate sequence. Keep comments that document contracts or reasons; these examples alone are low priority.
- **#53 [high]** `src-tauri/src/forward.rs:670`: the real forward test is ignored by default and needs a separately configured host; default tests cannot exercise the real transport path.
- **#54 [high]** `src-tauri/src/sharing.rs:765`: the real-host sharing test is ignored by default and depends on external binary/directory setup; document or automate a safe isolated integration job.
- **#55 [medium]** `src/lib/host/tauri.test.ts:41`: assertions directly encode `openArgs`' internal command/argument mapping; useful as a wire contract but susceptible to refactor churn unless paired with a Tauri boundary test.
- **#56 [medium]** `src/lib/host/mock.test.ts:80`: the replay test compares lengths/sequences against the same recording fixture, so by itself it does not establish compatibility with an independently produced real-host message.
- Rough sampled count: 4 test notes, including 2 disabled tests; no claim that the entire suite has only four such tests, nor that these tests cannot fail. No tests/builds were run: this is inventory-only, with no source edits.
