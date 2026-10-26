# Deviations: core-types-session

Owner: `core-types-session`. Format per README §5: section, change, reason, impact.
Where the spec text and pi 1.4.0 disagreed, pi won when the spec's acceptance (§7.2) requires
equality with pi's `buildSessionContext`. Every item is covered by a test.

## Contract (public API) differences

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | session-v3 §4 rule 5 | `SessionContext.model` / `SessionProjection.model` are also updated by assistant messages on the path (`provider`/`model` of the physical model that answered), not only by `model_change`. `branch_selection` still reports only explicit `model_change` / `thinking_level_change`. | pi `getSessionContextSettings` does this; golden equality (§7.2) needs it. | core-runtime: virtual-model resolution uses `branch_selection` as specced. |
| 2 | session-v3 §4 rule 4 | A `compaction` without `systemMessage` projects **only** the `compactionSummary` (pi `sessionEntryToContextMessages`). The fold fallback is available through `entry_to_context_messages(entry, Some(fallback))`; projection passes `None`. Legacy migration instead **stores** the folded checkpoint in `systemMessage`, so migrated sessions keep their prompt. `append_compaction` without a draft checkpoint folds the current projection like pi `appendCompaction`. | pi parity of old pi sessions; spec intent kept for migrated Oino sessions. | none for new sessions. |
| 3 | session-v3 §4 rule 3 | Context edits are collected from the compaction-selected entries (pi), and `replacement: null` omits *any* targeted selected entry (pi `projectContextEntry`). Mismatched block kinds (images on assistants, thinking/tool calls on users) are dropped when a replacement is applied. | pi parity. `append_context_edit` still only accepts user/assistant/toolResult/custom_message targets on the active branch. | none. |
| 4 | session-v3 §6.1/§6.2 | `is_persisted()` keeps pi semantics (file-backed vs in-memory). New `is_flushed()` reports whether the file has been created. | Core ports pi code that uses `isPersisted()` to mean "not in-memory". | core-runtime: use `is_flushed()` for the "file exists yet" question. |
| 5 | session-v3 §6.2 | Every `session_dir` argument is the **sessions root**; files always go to `<root>/--<cwd>--/`. `session_dir()` returns the root, `uses_default_session_dir()` compares it with `$OINO_AGENT_DIR` or `~/.oino` + `/sessions`. Listing and `find_by_id` do not filter by header cwd (pi only does that for flat custom dirs). | §6.1 defines the per-cwd layout under the resolved dir; pi's flat custom-dir mode does not exist in Oino. | core-runtime / modes: pass the resolved root (`--session-dir` > `OINO_SESSION_DIR` > setting > `<agent>/sessions`). |
| 6 | session-v3 §6.2 | `create_branched_session(&self, leaf)` writes the new file immediately (even without a user/assistant message) and fails with `InvalidAppend` for in-memory sessions. Added `branch_into_new_session(&mut self, leaf) -> Option<PathBuf>`, which follows pi `createBranchedSession` exactly (replaces `self`, writes only once a conversation exists, works in memory). | `&self` cannot switch the manager; the caller reopens the returned path. | core-runtime `/clone`, `/fork`: either method. |
| 7 | session-v3 §6.3 | `SessionError::Cancelled` added (listing with the cancel flag). Error is `#[non_exhaustive]`. | Needed to report cancellation. | additive. |
| 8 | session-v3 §5.2 step 7 | `MigratedSession { path: Option<PathBuf>, legacy_path, warnings }`: `path` is `None` when the session had no messages and was only moved aside. | Spec says such sessions are not migrated. | additive. |
| 9 | session-v3 §5.2 steps 3, 6 | Legacy entries are written **parents first** (pre-order, siblings in file order) before timestamps are assigned. Legacy writers dumped a `BTreeMap` keyed by UUID, so "file order" was random. Labels and branch-summary `fromId` resolve through dropped ancestors (for example `leaf_move` or an unresolved `context_edit`). A `label` on a root entry is dropped with a warning. | Keeps timestamps causal and the tree order sensible. | none. |
| 10 | session-v3 §6.2 | `append_session_info(None)` leaves out `name`. `append_label_change` with an empty label writes the label entry without a `label` field (pi writes `""`); both clear the label. | Option-typed API. | none. |
| 11 | session-v3 §1.2 | `AgentMessage::as_llm(&self) -> Option<Message>` returns an owned value, since `AgentMessage` holds no `Message` to borrow. | Not representable as `Option<&Message>`. | callers clone implicitly. |
| 12 | session-v3 §1.2 | `ConstrainedSamplingConfig` gained `variants: Option<IndexMap<String,String>>` (pi `{type:"grammar", variants}`). | pi 1.4.0 has two config shapes. | additive. |
| 13 | session-v3 §2 | `SessionHeader` is serialised as `{"type":"session",…}` via `#[serde(tag)]`. `CompactionEntry.system_message` is serialised with `"role":"system"` and read with or without it. Optional `details`/`data` keep an explicit JSON `null` (`oino_ai::json::keep_null`). | Byte-level pi shapes; the round-trip test covers them. | none. |
| 14 | session-v3 §1.1 | On read, missing message `content` defaults to `""` (system) or `[]` (others), as pi's projection does for `null`. `Usage.cost` defaults to zeros when missing. | Lenient reading of older files. | none. |
| 15 | session-v3 §4 | Free functions treat `leaf: None` as pi's `null` leaf (empty path). An unknown id falls back to the last entry. Use `EntryIndex::last_id()` for pi's default leaf. | `Option` cannot express pi's `undefined` vs `null`. | none. |

## Additive API (no spec change)

- `oino_ai::transform::transform_messages` (pi `transform-messages.js`), `split_error_tool_result_images`.
- `oino_ai::transcript::{SystemState, resolve_transcript_tools, tool_state_changes, render_system_message_update, initial_system_message, collapse_system_messages, …}`.
- `oino_ai::json::{json_semantic_eq, keep_null}`, `Usage::accumulate`, `AssistantMessage::empty`, `AnyModel`.
- `oino_session::time` (ISO formatting/parsing like `Date#toISOString`), `manager::{encode_cwd, cwd_session_dir, session_file_name, resolve_path, validate_session_id, default_agent_dir, default_sessions_root, migrate_legacy_file, session_info}`, `migrate::{parse_session_text, migrate_pi_values, legacy::migrate_oino_legacy_text}`, `SessionManager::index()`.

## Workspace / process

| # | Change | Reason | Request |
|---|---|---|---|
| W1 | `crates/oino-session` (pre-v1) moved to `legacy/crates/oino-session`, package renamed `oino-session-legacy`. `crates/oino-harness` and `crates/oino-app` depend on it through `oino-session = { package = "oino-session-legacy", path = … }`. The root `members` list was edited minimally: it adds `crates/oino-ai` and swaps the old path for the legacy path. | The new v3 crate owns the path; the workspace has to keep compiling before the D1 freeze. | **integration**: the D1 freeze should `git mv` the remaining `crates/*` legacy crates and leave `crates/oino-ai` and `crates/oino-session` alone. The new glob members cover them. |
| W2 | The new crates declare `[lints]` locally (mirroring quality §2) and depend on `thiserror = "2"` directly. The root workspace still pins `thiserror = "1"` and has no pedantic lints. Appended `indexmap` and `proptest` to `[workspace.dependencies]`. | Shared-file rules allow only appends. | **integration**: switch the root to quality §2 lints and `thiserror = "2"`; then the crates move to `lints.workspace = true`. |
| W3 | The pi oracle (`scripts/pi-oracle/core-types-session/`) imports pi from the global mise install (override with `PI_CODING_AGENT_PKG`), not from `scripts/pi-oracle/node_modules`. | No pinned oracle package exists yet. | **integration**: point `PI_CODING_AGENT_PKG` at the version-locked oracle install once it exists. |

## Deferred

- `crates/oino-protocol` is **not started** (client events, RPC records, UI protocol/OUS, extension event payloads; runtime §6, modes, ui-protocol, extension-api §4). It needs its own pass.
- Criterion benches (quality §5: `open` 10k < 60 ms, `build_session_context` 10k < 15 ms) are not written yet.
