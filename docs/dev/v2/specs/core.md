# Core durability, efficiency, crash protection, package split — spec (v2)

Status: **spec, implementation-ready** (core work package, DIRECTION §3.1/§3.2/§3.3 + §1 package
split and bundle move). Inputs studied: [DIRECTION.md](../DIRECTION.md), [CHARTER](../../v1/CHARTER.md),
[architecture.md](../../v1/architecture.md), [runtime.md](../../v1/runtime.md),
[session-v3.md](../../v1/session-v3.md), [extension-api.md](../../v1/extension-api.md), the current
code (citations below), pi 1.4.0 dist (`dist/core/session-manager.js` and friends),
deepseek-harness (`docs/subsystems/persistence.md`,
`packages/session/session-checkpoint-policy`, `packages/session/session-persistence-jsonl/src/lease.ts`),
hydro-project/infinity (`crates/infinity-agent-core/src/system/local/{router,driver}.rs`), and the
sibling spec [dist.md](dist.md) (its WP-D1 depends on this spec's package-split WP).

**Not in this spec** (owner assigned elsewhere): WIT / kanboard / cron / browser host-service ABI
changes (spec-ext-abi), unipi-host process, UI. Where those phases interact with core, the
interface is named and the owning phase is referenced.

Rule carried over from DIRECTION: everything stays inside **pi session format v3**; pi parity
tests stay green; measure before and after (§4). All new persisted records use pi entry types a
stock pi 1.4.0 reader already understands (§3).

---

## 1. Current-state findings (with file:line)

### 1.1 Session persistence today

- Appends go through `files::append_line` — `OpenOptions::append`, one `write_all` +
  `flush` per line, **file opened and closed per append, no `fsync`**
  (`crates/oino-session/src/manager/files.rs:172-193`). `SessionManager::persist_line`
  (`crates/oino-session/src/manager/mod.rs:550-566`) calls it whenever the session is flushed.
- Atomic rewrites (`write_atomic`, files.rs:159-175) fsync the temp file; `write_new`
  (files.rs:141-152) does not, and neither syncs the parent directory.
- A message entry is appended only at the loop's `MessageEnd`
  (`crates/oino-core/src/session/listener.rs:138-141`, `persist_message` at :146): stream chunks
  are never persisted and one settled assistant message per model attempt is already the rule.
  The toolResult `message` entry is appended after the tool finished; the assistant `message`
  entry containing the `toolCall` blocks is appended before execution starts.
- Read tolerance for torn tails already matches pi: malformed lines are skipped
  (`crates/oino-session/src/migrate/mod.rs:94-130`; pi `dist/core/session-manager.js:313-317`),
  and pi additionally terminates a torn last line by appending `"\n"` (session-manager.js:365-367)
  — Oino does **not** do that repair append today.
- No record exists of "this tool call was dispatched" before the tool body runs, and there is no
  repair of dangling tool calls on resume. pi has neither; an assistant message ending in
  `toolUse` with no matching `toolResult` survives a crash and would break the next provider
  request (pi `continue_run` requires the last message to convert).

### 1.2 Single-writer and locking today

- There is **no lock** on session files. Two `oino` processes (TUI + CLI, or two hosts) can open
  the same file and interleave `O_APPEND` lines. In-process, the `manager: Mutex<SessionManager>`
  (`crates/oino-core/src/session/mod.rs:223`) serialises one session's writes, and
  "session writes happen only from the session task, entry appended before the event is
  published" is the v1 rule (architecture.md §7) — but nothing exists across processes.
- `oino-settings` uses `fs4` file locks for settings/auth (`fs4` in root `Cargo.toml`), so the
  crate is already a workspace dependency.

### 1.3 Extension runtime / engine today

- One `WasmRuntime` = one wasmtime `Engine`, one epoch-ticker thread, one `LocalServices` hub
  (`crates/oino-wasm-host/src/engine.rs:121-170`). It is created per `CliLoader`
  (`crates/oino/src/loader.rs:226-236`), i.e. **one per `Services::create` call**: the binary's
  startup path (`crates/oino/src/startup.rs:460`), a second one in the ACP factory
  (`crates/oino/src/modes/acp.rs:30`). A host that opens several `(cwd, agent_dir)` services
  would create one engine, ticker thread and hub each. The compiled-component cache is wasmtime's
  on-disk cache (`~/.oino/cache/wasmtime`, engine.rs:133-137); in-memory `Component`s are
  recompiled per `load` (engine.rs `load_inner`).
- One component instance per extension per `Services`, shared by every session and child of that
  cwd (extension-api.md principle 5; `WasmInstance` actor, `crates/oino-wasm-host/src/instance.rs`).
  Guests keep per-session state in `SessionMap<T>` keyed by session id.
- Child sessions never instantiate their own components: `child_extensions` filters the parent's
  `ExtensionSet` (`crates/oino-core/src/agents/child.rs:351-366`, `ExtensionPolicy::None` default).

### 1.4 Nested agents today

- `spawn_agent` (`crates/oino-core/src/agents/child.rs:372-431`) builds a full `AgentSession`
  (`build_child`, :432-502) sharing the parent's settings/models/stream, registers it on the
  `HostRouter`, spawns an event-forwarding task (:563) and keeps the child alive until
  `close(grace)` or parent dispose. An idle child retains its session, transcript projection,
  subscriptions and forward task indefinitely. Nothing hibernates.
- Children with `Persistence::File` write their own session file immediately
  (`child_manager`, child.rs) under `<parent session dir>/children/` (deviation 15), and
  `Seed::SessionFile` continues such a file in place — the cold-resume primitive already exists.
- Concurrency cap: `max_concurrent` = setting `extensionSettings.subagents.maxConcurrent`,
  default 8, hard cap 16 (child.rs:261-277). Tree accounting: `Tree::live`
  (`crates/oino-core/src/agents/mod.rs`).

### 1.5 Crash behaviour today

- A trap poisons the instance actor; the next call restarts it (max 3 restarts / 10 min), then
  the extension is disabled **for the whole process** (`crates/oino-wasm-host/src/instance.rs:42`,
  :160-190). There is no per-session disable and no client event naming it (only
  `extension_error`).
- The agent run is driven by `tokio::spawn(async move { session.drive_run(..) })`
  (`crates/oino-core/src/session/prompt.rs:565`); a panic inside aborts the task, leaves
  `run_active = true` in the run book forever (session stuck "busy"), and no event reaches
  clients. Same for `forward_child_events` and other spawned tasks.
- No safe mode, no `--exclude-extension`. Persistent per-extension disable exists as
  `disabledExtensions` settings + `_oino/set_extension_enabled`
  (`crates/oino-core/src/session/feeds.rs:293-318`), effective only for sessions created
  afterwards.

### 1.6 Bundle and discovery today

- `oino-bundle` (build.rs at `crates/oino-bundle/build.rs`) embeds `extensions/dist` into the
  `oino` binary (zstd components + docs pseudo-package `oino.docs`). `CliLoader`
  (`crates/oino/src/loader.rs`) loads bundled → `~/.oino/extensions` → trust-gated
  `.oino/extensions` → `-e`, same-id replacement in place; `OINO_BUNDLED=0` drops the bundle
  (loader.rs:27, 79-82). Discovery lives in the binary, not core — the host never reuses it.
- `docs_root` = extracted `~/.oino/bundled/oino.docs@<version>-<hash8>` (runtime.md §9.4).
- The distribution spec [dist.md](dist.md) §5 already decides the replacement: a **share dir**
  per OS the installers own, discovery moving **into `oino-core`**, `OINO_BUNDLED` retiring. This
  spec owns the core side and the crate move; dist WP-D1 owns staging.

### 1.7 Extension state audit (rule §2.1.8)

All unipi extensions keep session state in `SessionMap<T>` (in-guest memory). Durable restore
today (survey of `extensions/unipi/*/src`):

| Extension | Session state (memory) | Durable restore today | Verdict |
|---|---|---|---|
| workflow | `PlanState` | `restore()` reads session `custom` entries `unipi:plan-mode` (`workflow/src/plan.rs:11,63-81`) | **OK** |
| subagents | records + live runs (`subagents/src/lib.rs:85-98`) | records in `/ext-workspace/<wsid>/sessions/<sid>/index.json` (lib.rs:335-346, 415-421); `record::restore` marks Running → Failed "interrupted — Oino exited while it ran" (`record.rs`) | **OK** (needs one fix, §2.1.8) |
| fusion | sidekick + active model (`fusion/src/lib.rs:112-124`) | sidekick spawned with `seed_session_file` + `session_dir` (lib.rs:629-640) so the child file is found again; `active` restored from global state file | **Partial** (§2.1.8 fix) |
| long-horizon | graph machine, ralph loop, todos (`long-horizon/src/lib.rs:90-110`) | workspace files `state.json`, `graph.json`, outbox (lib.rs:202-238) + `custom` entries `long-horizon-decision` (:460); crash-recovery re-delivery at session_start (:2429-2470) | **OK** (files are workspace-scoped: same graph re-attached to a new session — intended) |
| background-tasks | `local` map (cwd/notified/stopped), seen, dismissed, pane (`background-tasks/src/lib.rs:65-72`) | host task list re-read at session_start (:1082) from the host task service (`~/.oino/state/oino.tasks/session/<sid>/tasks.json`); `local` and `dismissed` are memory-only | **Violates** (§2.1.8 fix) |
| watchdog | tracked tools, streaks, kills (`watchdog/src/lib.rs:112-130`) | observation rebuilds by watching; decisions appended as `custom` (`long-horizon-decision`, :817) | **OK** (state genuinely disposable) |
| ask-user | none beyond settings; the question lives in a dialog + tool call | cancellation text constant exists (`ask-user/src/lib.rs:53-57`); an in-flight `ask_user` tool call is closed by resume repair (§2.1.4) | **OK after repair** |
| btw | side-note queue (`btw/src/lib.rs:204-215`) | cleared on `session_start`/`session_tree` (lib.rs:601-608) | **OK** (disposable) |
| compactor | pending method/stats (`compactor/src/lib.rs:86`) | none; a pending compaction command is lost | **Violates** (§2.1.8 fix) |
| core | work-tray tabs (`core/src/lib.rs:56-58`) | tray refetched from bus | **OK** |
| dream | digest views (`dream/src/lib.rs:141`) | digests on disk (`dream/src/store.rs`) | **OK** |
| info-screen | dashboards | derived from bus/services | **OK** |
| kanboard | guard/monitor/tracker, notices, cache (`kanboard/src/lib.rs:90`) | board data in the kanboard CLI's own locked files; session state rebuilds from events | **OK** |
| mcp | panels | server configs durable; connections reconnect | **OK** |
| memory | per-run flags (`memory/src/lib.rs:195`) | palace on files (`palace.rs`) | **OK** |
| notify | alert dedup/timers (`notify/src/lib.rs:100`) | none; worst case one duplicate notification after restart | **OK** (accepted) |
| skill-registry / updater / utility / web-api | panels/caches | caches refetch | **OK** |

---

## 2. Design

### 2.1 Durability (DIRECTION §3.1)

**D-LOG — the log is the only truth.** No change needed to the principle: model history, UI
wakeup and extension state are already derived from the session file + kv/workspace files
(§1.7). This spec closes the gaps that made that claim false under crash (write durability,
open tool calls, second writers, disposable guest state).

**D-WRITER — one open append handle per session file.**
`SessionManager` gains `writer: Option<SessionWriter>` (`struct SessionWriter { file: std::fs::File, path: PathBuf }`), opened lazily at the first `persist_line`, held until
`set_session_file` / `new_session` / `reset_leaf` invalidate it. `persist_line` writes the line
(+`\n`) with one `write_all` on the held `O_APPEND` handle. New API:

```rust
impl SessionManager {
    /// Flush + fsync the session file. Also fsyncs the parent dir on first creation (POSIX).
    pub fn sync(&mut self) -> Result<(), SessionError>;
}
```

- First flush (header + buffered entries) uses the existing `write_new` path, then `sync()` and a
  one-time parent-directory `fsync` (POSIX; Windows has no directory-fsync contract — same
  exception deepseek's JSONL backend documents, `session-persistence-jsonl/src/win32.ts`).
- `files::append_line` stays for migration/listing code paths only.
- `SessionError::Io` from `sync()` is the fail-closed signal every barrier below propagates.

**D-FLUSH — three fsync barriers, fail-closed** (deepseek session-checkpoint-policy model):

1. **Before each model request.** `SessionStream::stream`
   (`crates/oino-core/src/session/hooks.rs:378`) begins with
   `session.durability_barrier(Barrier::Request)` → `SessionManager::sync()`. On error the
   stream returns a single `error` event (same shape as an authorize failure), so the failed
   request never leaves the process. This covers the first request of a run and every
   continuation (steering, retries, compaction continuations).
2. **Before each tool with side effects.** In `SessionHooks::before`
   (`crates/oino-core/src/session/hooks.rs:57-74`), after the extension gate passes and next to
   the existing `checkpoint_tool_call` call: append the dispatch record (D-DISPATCH) and
   `sync()`. On error return `HookError` — the tool body does not run; `run_tool_call` converts
   it into the tool's error result (pi's hook-failure path), so the turn fails loudly instead of
   running an unpersisted side effect.
   *Side-effect classification:* every tool except built-in `read`, `grep`, `find`, `ls` and
   extension tools whose `annotations.readOnly == true`. Read-only tools skip the record and the
   fsync (they have nothing to repair and no effect to attribute).
3. **At each step boundary.** After boundary entries are appended in
   `dispatch_turn_end_boundary` (`crates/oino-core/src/session/boundary.rs:199`) and
   `run_before_settle_boundary` (:269), and after `flush_pending_custom_messages`
   (`crates/oino-core/src/session/prompt.rs:521-528`): call `sync()`; a failure is logged as a
   diagnostic and **the next barrier (1) fails the next request** — the turn cannot continue on
   top of unpersisted state, but a failing boundary sync does not tear down an already-finished
   turn.

Stream chunks remain never persisted; one settled assistant message per attempt stays the rule
(§1.1). Held `bashExecution` messages (user bash while streaming) are still in-memory only —
documented, accepted (they are user-initiated and replayable by the user).

**D-DISPATCH — tool call recorded before it runs, in pi v3 entries.**
For every barrier (2) tool call, append one **`custom`** entry before the tool body runs:

```json
{"type":"custom","id":"<8hex>","parentId":"<current leaf>","timestamp":"<ISO>",
 "customType":"oino.tool-dispatch",
 "data":{"toolCallId":"<id>","toolName":"bash","input":{…},"readOnly":false}}
```

Why this is safe for pi readers: `custom` entries are display/state entries that **do not
participate in context** (pi `session-manager.js:164-181` projects nothing for `custom`;
`appendCustomEntry` exists for exactly this shape, session-manager.js:900-905). Unknown
`customType`s are inert to pi. Oino's own projection already ignores `custom`
(session-v3.md §4 rule 4). Extensions can read them via `session.entries()`.

**D-REPAIR — interrupted-turn repair on resume (in the resume layer, never truncation).**
Modelled on deepseek's crash recovery ("only a torn physical tail is discarded; repair is the
reader's job and appends closers"):

```rust
impl SessionManager {
    /// Repairs an interrupted turn at the leaf. Call once, under write ownership, before the
    /// session is used. Appends only; never rewrites or truncates.
    pub fn repair_interrupted_turn(&mut self) -> Result<RepairReport, SessionError>;
}
pub struct RepairReport { pub closed_tool_calls: Vec<String>, pub torn_tail: bool }
```

1. **Torn tail:** if the file's last line is non-empty and not valid JSON (partial write), append
   `"\n"` (pi's own repair, session-manager.js:365-367) so the fragment stays a permanently
   skipped line. `parse_session_text` already skips it (§1.1). Never delete bytes.
2. **Dangling tool calls:** compute the projection
   (`build_session_projection`); scan the trailing run (from the end: trailing `toolResult`
   messages, then the assistant message that issued them): every `toolCall.id` in that assistant
   message without a matching `toolResult` in the trailing run is an **open call**. For each,
   append one `message` entry (pi v3, exactly a toolResult a stock pi reader understands):

```json
{"type":"message","id":"<8hex>","parentId":"<current leaf>","timestamp":"<ISO>",
 "message":{"role":"toolResult","toolCallId":"<id>","toolName":"<name>",
   "content":[{"type":"text","text":"Tool execution was interrupted before completion; the outcome is unknown. The effect may or may not have happened. Do not retry blindly: verify the result first, and ask the user if verification is not possible."}],
   "isError":true,"timestamp":<ms>}}
```

3. Append one `custom` entry `customType:"oino.resume-repair"` with
   `data:{"closedToolCalls":[…],"tornTail":bool,"at":<ms>}` and `sync()`.
4. Repair runs **only on a write open with the lease held** (§ D-LOCK): core's
   `AgentSessionRuntime` calls it right after `SessionManager::open` when it acquired (or
   reacquired, § takeover/hibernation) write ownership. Read-only paths (listing, fork sources,
   `session-read`, export) never repair.

Rules: open calls are **never auto-retried** — there is no retry mechanism for tool calls and
none is added; the repair text forbids blind retry; ids are unique per attempt so a re-run of the
same call is a new call. `continue_run`/`prompt` now always see a balanced transcript.

**D-LOCK — single-writer lease per session file, kernel-arbitrated** (deepseek lease model,
`session-persistence-jsonl/src/lease.ts`):

- Lock file: **`<session file>.lock`** sibling. Acquire = open (create) + exclusive lock:
  POSIX `flock` via `fs4::File::try_lock_exclusive`; Windows the same `fs4` call (byte-range
  `LockFileEx`). The kernel releases both on process death — **a crashed holder never blocks a
  successor, and there is no expiry that could expropriate a stalled writer** (deepseek's
  deliberate choice; adopted). The lock file is **never unlinked on release** (POSIX locks name
  an inode); it is removed only when the session file itself is deleted (best-effort).
- In-process registry (required: `flock` is per-descriptor, the same process could lock twice):
  `static SESSION_LOCKS: OnceLock<Mutex<HashMap<PathBuf, ()>>>` inside `manager::lease`; a
  second acquire in-process fails with the same `SessionLockError::HeldBy` naming the caller's
  own process.
- Advisory metadata `<session file>.lock.json`, written after acquiring, removed on release
  (best-effort): `{"v":1,"pid":…,"bootId":"…","host":"…","client":"unipi-host 2.0.0","startedAt":…}`.
  `bootId` = `/proc/sys/kernel/random/boot_id` (Linux), else a random id persisted once under
  `<agent>/state/oino.boot-id`. `HeldBy(info).dead()` = same host+bootId and pid not alive
  (`kill(pid, 0)` / `OpenProcess` probe) — used only for diagnostics and the takeover wait loop;
  the kernel lock remains the arbiter.
- API (`oino-session`, feature `manager`):

```rust
pub struct SessionLease { /* path + held file */ }
pub struct LeaseInfo { pub pid: u32, pub boot_id: String, pub host: String, pub client: String }
pub enum SessionLockError { HeldBy(LeaseInfo), Io { path: PathBuf, source: std::io::Error } }
impl SessionLease {
    pub fn acquire(session_file: &Path, holder: &LeaseInfo) -> Result<Self, SessionLockError>;
    pub fn info(&self) -> &LeaseInfo;
    /// Deletes the takeover request file, drops the kernel lock, removes the metadata.
    pub fn release(self);
}
```

- Ownership: the lease belongs to the **process**, held by `AgentSessionRuntime` for the current
  session (new field on `RuntimeCore`; released on `dispose`, `switch_session`, `new_session`,
  `fork`, `import_jsonl`, takeover). Acquired (a) when opening a file-backed session source,
  (b) lazily at the first flush for new sessions, before the file is created. Children acquire
  for their own file in `child_manager` when `Persistence::File` (child.rs). NFS/home mounts are
  out of scope (documented).

**D-TAKEOVER — "close the other runner and reload here" (Devin-like).**

1. Acquire fails with `HeldBy(info)` → if `info.dead()` retry acquire for ≤2 s (kernel lag);
2. still held → write the takeover request `<session file>.takeover`
   (`{"v":1,"requester":LeaseInfo,"at":…}`) and poll the lock every 200 ms for ≤10 s;
3. **Holder side:** every lease runs a takeover watcher (a 500 ms poll task; `notify` is already
   a wasm-host dep but core stays poll-based for simplicity). On a request: dispatch
   `session_shutdown` with payload reason `"takeover"` (additive reason string), abort the run,
   dispose the session (which releases the lease), publish
   `OinoEvent::SessionTakenOver { by: LeaseInfo }` so the TUI/host shows *"Session moved to
   another window (by <client>)"* and returns to its session list / next session. The embedding
   mode can override the default handler via `RuntimeOptions::on_takeover` (the hub closes the
   backend and notifies its client);
4. lock released within the deadline → acquire, `open`, `repair_interrupted_turn`, continue here
   (the "reload" is exactly the resume path);
5. deadline expired → `SessionOpError::SessionLocked`, RPC/CLI text:
   `"Session <id> is open in <client> (pid N on <host>). It was asked to close and did not respond; close it there and try again."`

Cross-host ("another host" in DIRECTION) means another process on the same machine — apps on
other machines are clients of their own host and never touch the file; the protocol above
covers every local case.

**D-EXTSTATE — the extension-state rule.** *"Every extension must be rebuildable from its
session `custom` entries plus its kv store (`~/.oino/state/<id>/…`). State held only in memory is
disposable."* Enforcement is by audit + fix + a restart test, not by new host machinery (the
kv scopes and `session.append-entry` already exist):

- **background-tasks**: persist the `local` map (cwd, notified, stopped, adopted) as one
  `custom` entry `unipi:bg-tasks` written (append-entry) whenever a field changes and rewritten
  at settle; `dismissed` stays disposable. Restore at `session_start` merges host task list +
  `local`. Fix: `extensions/unipi/background-tasks/src/{lib,snapshot}.rs`.
- **fusion**: the sidekick record (child session file, model, effort) is memory-only in `State`;
  persist it as `custom` `unipi:fusion-sidekick` on spawn/close and restore at `session_start`
  so a restarted host re-attaches with `seed_session_file` instead of losing the pairing. Fix:
  `extensions/unipi/fusion/src/lib.rs`.
- **subagents**: `restore` marks Running→Failed already; change the record's error to the
  repair-compatible text `"interrupted — Oino exited while it ran (outcome unknown; see the
  child session)"` and add `resumeHint` when the child file exists so the tray offers
  cold-resume. Fix: `extensions/unipi/subagents/src/record.rs`.
- **compactor**: persist `pending_method`/`pending_command` as `custom` `unipi:compactor-pending`
  when set and clear the entry on execution; restore at `session_start`. Fix:
  `extensions/unipi/compactor/src/lib.rs`.
- **ask-user**: no state change; an in-flight `ask_user` tool call is closed by D-REPAIR with the
  unknown-outcome text (its own cancel text stays for user cancellation).
- All others: verdict OK (§1.7); each gets the restart test below instead of a fix.
- **Restart test (all unipi extensions)**: extend each `harness/run.sh` (and/or the wasm-host
  harness) with: scripted session → SIGKILL the host mid-turn → restart → assert extension state
  restored / safely reset and the repaired session usable. This is the durable regression gate
  for the rule.

### 2.2 Efficiency (DIRECTION §3.2, model: infinity router/driver)

**E-ENGINE — one shared wasmtime Engine (and compiled-component cache) per process.**
Infinity's router owns one pool and fully releases a driver's memory when idle; the analogous
core move is sharing the heavy per-process resource (the engine) instead of multiplying it:

- `oino-wasm-host` gains `EngineRegistry`:
  `static REGISTRY: OnceLock<Mutex<HashMap<EngineKey, Arc<EngineShared>>>>` where `EngineKey` =
  (cache_dir, config-hash). `WasmRuntime::new` resolves-or-creates the `EngineShared`
  (Engine + Linker + epoch ticker + `Component`/`InstancePre` caches), so every `CliLoader`,
  the ACP factory and the future multi-cwd host share **one engine, one ticker thread**.
  `WasmRuntimeOptions::engine: Option<Arc<EngineShared>>` lets tests isolate.
- In-memory compile cache on `EngineShared`: `RwLock<HashMap<Sha256, Arc<Component>>>` and a
  matching `InstancePre` cache, so N services of the same packages compile once per process.
- `LocalServices` (bus, secrets, tasks) moves to the same registry, keyed by `agent_dir`
  (process-wide task store and sticky topics become coherent across services of one host).
  `WasmRuntimeOptions::local_services` stays for test isolation.

**E-LAZY — lazy session loading: a session is a record until a message arrives.**
New `crates/oino-client/src/hub.rs` (the piece unipi-host links; infinity's "spawn driver on
demand" as a core type):

```rust
pub struct SessionHub { /* agent_dir, mode, Services cache, records, live map, idle timer */ }
impl SessionHub {
    pub async fn create(opts: HubOptions) -> Result<Arc<Self>, ClientError>;
    pub async fn open(&self, target: &str) -> Result<Arc<dyn SessionBackend>, ClientError>;  // wake
    pub async fn attach(&self, session_id: &str) -> Result<Arc<dyn SessionBackend>, ClientError>;
    pub async fn list(&self) -> Result<Vec<HubSession>, ClientError>;   // records only, no wake
    pub async fn close(&self, session_id: &str) -> Result<(), ClientError>;
}
pub struct HubSession { pub info: SessionInfo, pub state: HubSessionState } // record|live|crashed|locked
```

- Records come from `SessionManager::list` (no Services, no engine, no lease). A record becomes
  live on `open`/`attach`/any inbound message for it: acquire lease → repair →
  `AgentSessionRuntime` → `InProcessBackend`.
- `Services` cache per `(cwd, agent_dir)` (LRU, default 4 cwds; engine shared via E-ENGINE);
  a services set with zero live sessions for `services_idle` (default 900 s) is disposed
  (extension `shutdown`, instances dropped).
- Idle eviction: a live session with no run, no compaction, no subscribers, no live children and
  no arbiter wait for `session_idle` (default 900 s) is disposed (`dispose` already emits
  `session_shutdown`, which makes guests drop their `SessionMap` entry — the rule §2.1.8 keeps
  this lossless) and returns to a record; the lease is released; the next message re-opens and
  repairs. Frontends get `agents_changed`-style notification via a new
  `OinoEvent::SessionEvicted { session_id }` only if they had subscribed before (they are gone
  by then; the hub list is the source of truth).
- Settings: `extensionSettings.hub.{sessionIdleSec, servicesIdleSec, maxServices}` (defaults
  900/900/4; 0 disables eviction — used by the TUI's single-session runtime, which does not use
  the hub at all).

**E-HIBERNATE — hibernating subagents, woken by cold resume from log + kv.**
On the `ChildAgent` shell (child.rs):

- A file-persisted child (`Persistence::File`) that is idle (no run, no dialogs, no `events()`
  subscriber, no open tools) for `hibernateAfterSec`
  (`extensionSettings.subagents.hibernateAfterSec`, default 120, 0 = never) is hibernated:
  emit `session_shutdown{reason:"hibernate"}` to the child's extensions (guests drop the
  per-child `SessionMap` entry — the "drops its WASM state" half of DIRECTION §3.2.3; the shared
  instances stay for the parent), dispose the child `AgentSession` (transcript projection,
  subscriptions, forward task released — the "drops its task" half), keep the
  `ChildAgent { id, label, owner, session_file, spec }` shell with `state() == "hibernated"`.
- The `AgentSpec` (already `Clone + Serialize`, agents/spec.rs) is stored in the shell at spawn.
- Wake: any handle operation (`prompt`, `steer`, `follow_up`, `abort`, `set_thinking`,
  `last_assistant_text`, `usage`, `wait_idle`, `events`, `close`) rebuilds the session exactly
  like `spawn_agent` but with `Seed::SessionFile(child_file)` (continues the file in place —
  the cold-resume primitive that already exists, §1.4) plus repair (D-REPAIR), then serves the
  call. The child's session id (hence the agent id) is stable across hibernation.
- `AgentInfo.state` gains `"hibernated"`; `agents_changed` fires on hibernate/wake. `usage()` on
  a hibernated child returns the shell's last known stats (child usage entries are already
  appended to the parent at every `MessageEnd`, child.rs:563-607, so nothing is lost).
- Parent eviction (E-LAZY) closes children as today (`close_children`); hibernation never
  outranks the parent's lifetime. Memory-persisted children never hibernate (nothing to resume
  from); they are simply closed when the parent goes.

**E-CHILDSET — lighter children.** Structurally already true (§1.3: children share the parent's
instances; `ExtensionPolicy::None` is the default and filters the set). This spec pins it:
subagent profiles that need extension tools must list them (`Only(ids)`) — verified in
`subagents/src/profiles.rs` and fusion's sidekick spec (`deny`/allow lists, fusion lib.rs:634-643)
— and the benchmark (§4) measures RSS with the default `None` policy. No new machinery.

**E-BENCH — see §4.**

### 2.3 Crash protection (DIRECTION §3.3)

**X-TRAP — a trapping or panicking extension is disabled for that session; the app is notified.**

- `ExtensionRunner` gains `disabled: Arc<Mutex<IndexSet<ExtensionId>>>` (session-scoped);
  `active()`/`handlers()`/`tools()`/`commands()`/`renderers()`/arbiter scans skip disabled ids
  (runner.rs filtering points: §1.3 runner functions).
- Trigger: any `ExtensionError` with `trap: true` from a call made with this session's context,
  **or** a Rust-side panic caught in `ExtensionRunner::call` (wrap the future in
  `AssertUnwindSafe(..).catch_unwind()`; a panic becomes `ExtensionCallError` with the panic
  message). One trap disables the extension for this session immediately (DIRECTION wording;
  cheap and reversible — a session replacement resets it), emits `extension_error` and a new
  `OinoEvent::ExtensionDisabled { sessionId, extensionId, reason: "trap"|"panic"|"disabled" }`
  which frontends surface as a notice ("Extension X crashed and was disabled for this session").
- The instance-level circuit breaker (3 restarts / 10 min → disabled for the process,
  instance.rs:160-190) stays as the last resort; the per-session disable usually fires first and
  keeps other sessions running.

**X-PANIC — per-session panic isolation in the host.**

- `crates/oino-core/src/session/mod.rs` gains `spawn_isolated(&self, name, fut)`: a tokio task
  whose panic is caught (`catch_unwind`), logged with the session id, and reported to the
  session. `drive_run` (prompt.rs:565) and `forward_child_events` (child.rs:563) run through it,
  as do the retry/arbiter timers.
- On a caught panic in `drive_run`: reset the run book (`run_active = false`, clear cancel
  state), publish `OinoEvent::SessionCrashed { error }` and `run_state{idle}`; the lease stays;
  the session stays open and the next prompt works (state is rebuilt from the log — the point of
  §2.1). Other sessions are unaffected (they are separate tasks).
- The hub (E-LAZY) marks the session `crashed` in `list()` until re-opened. unipi-host and the
  TUI surface the event (host/UI phases).

**X-SAFE — safe mode after 2 failed host starts, with a named suspect.**

- New `crates/oino-core/src/safe_mode.rs`: state file `<agent>/state/oino.safe-mode.json`:
  `{"v":1,"consecutiveFailures":n,"lastFailures":[{"at":ms,"where":"services|session","extensionId?":"…","error":"…"}]}`
  (max 8 entries).
- `SafeMode::load(agent_dir)`, `.plan()` → `SafeModePlan { disable_extensions: bool
  (consecutiveFailures ≥ 2), suspects: Vec<(id, error)> }`; `record_failure(entry)` /
  `record_success()` (writes the reset).
- Who calls it: every embedding that creates a runtime — `crates/oino/src/startup.rs::start`
  (binary/TUI/print/RPC/ACP) and, later, unipi-host's main. Protocol: load → build
  `ServicesOptions` with `flags.no_extensions = plan.disable_extensions` (`-ne` semantics:
  **providers stay**, so the agent still runs — recommended, DIRECTION leaves the set open) and
  surface `plan` via `Services` diagnostics (TUI banner: "Safe mode: extensions are off after 2
  failed starts; suspects: unipi.x (…), unipi.y (…)"). Failure = `Services::create`/runtime
  create returned Err, or the process died before `record_success`; success = first session
  bound → `record_success()`. Extension init/load failures recorded during a failed start become
  the named suspects.
- `_oino/hello` reply gains `safeMode: bool`.

**X-EXCLUDE — `--exclude-extension <id>` (repeatable) + `--no-extensions` (existing).**

- `Args::exclude_extensions: Option<Vec<String>>` (`crates/oino/src/cli/args.rs`), wired into
  `LoaderContext { excluded: Vec<String> }`; `CliLoader`/core discovery drops any resolved
  package whose manifest id matches, with a diagnostic per dropped id (including `-e` paths).
  Process-scoped; `disabledExtensions` (settings, `_oino/set_extension_enabled`) stays the
  persistent app-level toggle. `-ne` unchanged.

### 2.4 Package split and bundle move (DIRECTION §1)

**P-LAYOUT — target layout** (pi-monorepo model: one repo, blank core, clients, distribution):

```text
crates/            # root workspace — the blank core + clients (a user builds on this alone)
  oino-ai oino-agent oino-session oino-settings oino-models oino-resources oino-tools
  oino-extension-api oino-core oino-wasm-host oino-rpc oino-client oino-view oino-tui
  oino-acp oino                       # (oino-bundle deleted, P-BUNDLE)
extensions/         # wasm workspace: sdk/, providers/, unipi/, fixtures/  (unchanged)
apps/unipi/         # distribution workspace (imported): app, host, proto
  dist/             # NEW: distribution packaging output (share-dir tree + docs tree), built by
                    #     `cargo xtask dist` (dist.md §6/§7) — the successor of oino-bundle
wit/  xtask/  scripts/  docs/  tests/
```

Crate moves that belong to **later phases** (listed so the target is complete, not executed here):
`oino-kanboard`, `oino-cron`, `oino-browser`, `oino-browser-protocol` move under the
distribution umbrella (`apps/unipi/crates/…`) together with the §3.4 host-service change
(spec-ext-abi) and the unipi-host phase — today `oino-wasm-host`'s hub links them
(hub.rs browser/cron/kanboard services), and unwiring them is ABI work, not a move. The `oino`
binary already depends on none of them.

**P-BUNDLE — `oino-bundle` leaves the `oino` binary.**

- Delete `crates/oino-bundle`; `crates/oino` drops the dependency and the `bundled()` loader
  source; `OINO_BUNDLED` retires (dist.md §5). Tests that relied on bundled fixtures switch to
  `-e extensions/dist/<id>` or a temp agent dir with installed packages.
- The embed role's successor is the **share dir** (dist.md §5): installers own
  `<dist>/extensions/<id>/…`; CLI-only installs unpack `unipi-extensions-<v>-<target>.tar.gz`
  into `~/.oino/extensions`. Nothing is compiled into any binary.

**P-DISCOVERY — discovery moves into `oino-core`** (dist.md §5 wording; today it lives in
`crates/oino/src/loader.rs`, which the host cannot link):

- New `oino-core::extensions::sources` (feature-default on, std-only): resolves the ordered
  package list — (1) distribution share dir (exe-relative heuristics per OS from dist.md §5,
  override `OINO_DIST_EXTENSIONS`), (2) `~/.oino/extensions`, (3) trust-gated
  `<cwd>/.oino/extensions`, (4) `-e` explicit dirs — with today's same-id replacement-in-place
  semantics (loader.rs `place`), plus `excluded` filtering (X-EXCLUDE).
- `ServicesOptions::loader` keeps the trait; `crates/oino/src/loader.rs` shrinks to: CLI arg →
  `sources::resolve`, `builtin:faux`/`ui-probe` (feature `faux`), and the `WasmRuntime`
  construction (shared engine, E-ENGINE). unipi-host gets identical discovery by linking core
  and passing its sidecar-adjacent share dir.

**P-DOCS — docs root without the bundle.** `docs_root` resolution becomes (in
`crates/oino/src/startup.rs::docs_root`, moving into core `sources`):
`OINO_DOCS_ROOT` > `<dist share>/docs` > legacy `~/.oino/bundled/oino.docs@*` > absent.
When absent, the system prompt's docs section is omitted (degrade gracefully instead of pointing
at a missing path). **Request to dist.md**: the share dir must also carry the docs tree
(`README.md` + `docs/*.md`, the files `oino-bundle/build.rs::write_docs` embeds today) — one
added line to dist §3 bundle contents (noted for the dist owner; nothing here blocks on it,
the fallback chain works meanwhile).

**P-WORKSPACE mechanics:** root `Cargo.toml` members unchanged (`crates/*`, `xtask`) after
deleting `oino-bundle`; `cargo xtask layering`-style checks keep `oino-core` free of wasmtime
(architecture §3 rule 2: core still only knows `oino-extension-api`; the `sources` module
returns `LoadedPackage` descriptors and calls the injected `ExtensionRuntime`).

---

## 3. pi session-format v3 compatibility and parity gates

Everything persisted here is stock v3:

| New record | Entry type | pi reader impact |
|---|---|---|
| `oino.tool-dispatch` | `custom` | none — `custom` never projects into context (pi session-manager.js:164-181); unknown `customType` inert |
| repair toolResult | `message{role:"toolResult"}` | none — exactly a pi toolResult message |
| `oino.resume-repair` | `custom` | none |
| `session.lock` / `.lock.json` / `.takeover` | sidecar files | none — pi ignores unknown siblings; session-file bytes untouched |
| torn-tail `"\n"` append | byte append | pi's own repair behaviour (session-manager.js:365-367) |

Never: rewriting or truncating existing lines, new header fields, new entry `type`s, version
bumps. `OINO_SESSION_*` env, file naming and layout unchanged.

**Parity regression gates** (every WP runs the subset it touches; all must stay green):
`cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`;
`cargo test --workspace` (includes `oino-session/tests/golden_pi.rs`,
`oino-agent/tests/{agent_loop_golden,validation_golden,agent_behaviour}.rs`,
`oino-core/tests/{session_golden,compaction_golden,runtime_flow,session_flow,agents}.rs`,
provider pi fixtures); `cargo xtask wasm` + committed `extensions/dist/checksums.json` (only when
extensions changed, WP-C9); every touched `extensions/unipi/*/harness/run.sh`;
`scripts/docs-check.sh`; new pi-oracle check (WP-C2): `scripts/pi-oracle/core/session-load.mjs`
loads a fixture session containing `oino.tool-dispatch`, the repair toolResult and
`oino.resume-repair` with **pi's own** `SessionManager` and asserts entry counts and an unchanged
`buildSessionContext` output vs the same session without the custom entries.

---

## 4. Benchmarks (DIRECTION §3.2.5) and measurement

**B-1 Subagent RSS/CPU benchmark.** New `xtask bench subagents --n {1,10,50} [--out dir]`:

- Drives a new small crate `crates/oino-bench` (root workspace, feature `faux`, `#[tokio::main]`)
  which: creates a temp agent dir; builds `Services` with the faux provider (no unipi set);
  opens a root session; spawns `n` children via `AgentSession::spawn_agent`
  (`AgentSpec { persistence: File, seed: Messages(one user msg), extensions: None, model: faux/faux-1 }`,
  the same path the `subagents` extension uses); waits idle; then prompts every child once
  (parallel faux turn).
- Samples (Linux `/proc/self/status` VmRSS/VmHWM, `/proc/self/stat` utime+stime; other OSes
  report `unsupported`): RSS after spawn, RSS after one full turn per child, ΔRSS per child,
  total CPU during the turn, plus barrier micro-cost (mean µs of `SessionManager::sync()` over
  100 appends, WP-C1).
- Writes `bench/results/subagents-<n>-<timestamp>.json` + a markdown summary table.

**Baseline before / after:** run on current `v2` HEAD **before** WP-C4/C7/C8 land (commit the
baseline JSONs under `bench/results/baseline-v2-<date>/`), then re-run after each of those WPs.
Acceptance targets: after E-ENGINE + E-HIBERNATE, per-child steady-state RSS cost at n=50 drops
measurably (expect ≥30 % vs baseline with hibernation on; the exact number is a recorded
measurement, not a gate); no WP makes RSS/CPU worse than baseline by >10 %.

**Parity gates:** §3 list.

---

## 5. Work packages

Sizing: one worker, one session each. `Dep` = hard prerequisite. Every WP ends with the §3 gates
green for the crates it touches and its acceptance row demonstrated.

| WP | Title | Files (new/changed) | Dep | Acceptance (commands + expected) |
|---|---|---|---|---|
| **WP-C1** | Session writer + fsync barriers | `crates/oino-session/src/manager/{mod,files}.rs` (writer + `sync()` + dir fsync), `crates/oino-core/src/session/{hooks,listener,boundary,prompt}.rs` (barriers 1–3), tests in both | — | `cargo test -p oino-session -p oino-core`: new tests — appends reuse one handle (fd count), `sync()` survives `kill -9` + reopen without loss at each barrier; barrier failure (chmod 0444 dir) fails the request/tool, never runs the body; read-only tools skip the fsync; existing goldens green |
| **WP-C2** | Tool-dispatch record + resume repair | `crates/oino-session/src/manager/{mod,repair}.rs` (D-DISPATCH append + `repair_interrupted_turn` + torn-tail append), `crates/oino-core/src/runtime.rs` (call after lease/open), `crates/oino-core/src/session/hooks.rs` (record in `before`), `scripts/pi-oracle/core/session-load.mjs`, fixtures | C1 | Unit: dangling toolCall repaired with unknown-outcome toolResult; balanced transcript feeds `continue_run`; no repair on read paths. Oracle: `node scripts/pi-oracle/core/session-load.mjs` → pi loads the fixture, entry count matches, context equal to the custom-entry-free twin. `cargo test -p oino-core tests/session_flow.rs` green |
| **WP-C3** | Single-writer lease + takeover | `crates/oino-session/src/manager/lease.rs` (new), `crates/oino-session/src/error.rs`, `crates/oino-core/src/runtime.rs` (acquire/release points incl. children), `crates/oino-core/src/events.rs` (`SessionTakenOver`), `crates/oino/src/modes/interactive.rs` (notice), `tests/e2e/lock.rs` (new) | C2 | `cargo test -p oino-session` lease tests (in-process double-open fails fast; kernel release on process death via a killed child process); `cargo test --test lock` e2e: two `oino` processes — second open closes the first's session and reloads here within 10 s; unresponsive holder yields the `SessionLocked` error text |
| **WP-C4** | Shared engine + component/InstancePre cache | `crates/oino-wasm-host/src/{engine,hub,core_bridge}.rs`, `crates/oino/src/{loader,startup}.rs`, `crates/oino/src/modes/acp.rs` | — | New test: two `Services` (two cwds) in one process share one `Engine` and one ticker thread (expose a counter for regression), packages compiled once (compile-call counter); `cargo test --workspace` + wasm-host harness green; cold-start time of a 2-services process not worse than 1-services baseline |
| **WP-C5** | Per-session crash protection (trap disable + panic isolation) | `crates/oino-core/src/extensions/runner.rs`, `crates/oino-core/src/session/{mod,prompt}.rs` (`spawn_isolated`), `crates/oino-core/src/agents/child.rs`, `crates/oino-core/src/events.rs` (`ExtensionDisabled`, `SessionCrashed`), `extensions/fixtures/*` (a panicking fixture) | — | Tests: trapping fixture disables itself for session A only (session B in the same process still dispatches) and emits both events; a panicking drive task publishes `SessionCrashed`, session accepts a new prompt, hub siblings unaffected (covered again in C7 test) |
| **WP-C6** | Safe mode + `--exclude-extension` | `crates/oino-core/src/safe_mode.rs` (new), `crates/oino-core/src/services.rs` (plan in/out), `crates/oino/src/{cli/args,startup,loader}.rs`, `crates/oino-tui` banner hook, docs | — | Unit: 2 recorded failures → next `Services::create` runs with extensions off and providers on, banner diagnostic names recorded suspects; success resets. `oino --exclude-extension unipi.notify …` drops exactly that package with a diagnostic; `-ne` unchanged; `oino --help` documents both |
| **WP-C7** | Lazy session hub | `crates/oino-client/src/hub.rs` (new), `crates/oino-client/src/lib.rs`, `crates/oino-core/src/runtime.rs` (dispose/lease support), `oino-settings` keys (`extensionSettings.hub.*`) | C3, C2 | Tests (faux provider): list shows records without engines/leases (assert lazy); open → lease + repair → live; idle eviction (short `sessionIdleSec`) disposes, releases lease, guest state cleared (fixture observes `session_shutdown`); re-open continues the conversation with no lost entries; crashed session (C5) listed `crashed` |
| **WP-C8** | Subagent hibernation | `crates/oino-core/src/agents/{child,feed,mod,spec}.rs`, `crates/oino-core/src/events.rs` (`AgentInfo` state), settings key `subagents.hibernateAfterSec` | C2 (repair on wake), C5 (spawn_isolated) | Tests: idle file-persisted child hibernates within the configured window (child `AgentSession` dropped, `session_shutdown{hibernate}` observed by fixture, state `"hibernated"`); `prompt` wakes it and continues the same child session id; `events()` subscribers prevent hibernation; parent dispose closes hibernated shells cleanly |
| **WP-C9** | Extension-state audit fixes + restart tests | `extensions/unipi/{background-tasks,compactor,fusion,subagents}/src/**` (fixes §2.1.8), all `extensions/unipi/*/harness/run.sh` (restart test), `extensions/dist/checksums.json` | C2 (repair semantics) | Each fixed extension: unit test restoring from the new `custom` entry/kv; harness run.sh green including the new kill-and-restart step (assert state restored / safely reset and the session usable). `cargo xtask wasm` + checksums updated |
| **WP-C10** | Package split: bundle out, discovery into core, docs root | delete `crates/oino-bundle/**`, `crates/oino/src/{loader,startup}.rs`, new `crates/oino-core/src/extensions/sources.rs`, `Cargo.toml`s, `xtask/src/main.rs` (bundle-check task), tests using `OINO_BUNDLED`, `docs/` (extensions.md, packages.md, environment-variables.md) | C6 (X-EXCLUDE wiring exists) | `oino` binary contains no embedded components (`OINO_BUNDLED` gone; `strings`/size check); with `OINO_DIST_EXTENSIONS=<staged dir>` and `~/.oino/extensions` population, load order + shadowing per dist.md §5 verified by test; docs root chain (env > share > legacy > absent-with-omitted-section) tested; full gates + `scripts/docs-check.sh` green. **Unblocks dist WP-D1** |
| **WP-C11** | Benchmarks + before/after record | `crates/oino-bench/**` (new), `xtask/src/bench.rs` (new), `.mise.toml` (`bench` task), `bench/results/**` (committed JSONs) | baseline before C4/C7/C8; final runs after them | `mise run bench -- subagents --n 50` produces `bench/results/subagents-50-*.json` on this machine; baseline + post-C4/C7/C8 numbers committed with a markdown table; barrier micro-cost recorded; no-parity-regression confirmed (§3 gates) |

Sequencing note for the graph: C1→C2→{C3, C9}; C4, C5, C6 are parallel-safe anytime after C1
(C4/C5/C6 have no durability deps); C7 needs C3; C8 needs C2+C5; C10 blocks dist WP-D1 and can
run parallel to the durability spine; C11 bookends.

---

## 6. Risks

1. **fsync latency** on slow/network disks adds ms per request/tool/step. Mitigation: barriers
   only (never per line), read-only tools exempt, micro-cost recorded in B-1; group-commit
   (deepseek's write-behind window) is a documented follow-up, not in scope.
2. **Windows lease semantics**: `fs4`'s byte-range lock blocks deleting a live `.lock` file and
   differs from deepseek's named-semaphore choice (which we reject: needs `windows-sys` unsafe in
   a crate that forbids it). Session deletion on Windows may leave a `.lock` behind while held —
   harmless (kernel-released), cleaned next delete.
3. **Takeover of a wedged runner** cannot force-close (deliberate, deepseek parity: no expiry
   that expropriates a live writer). The 10 s timeout yields a precise error naming the holder;
   force-kill stays a user action.
4. **Shared engine config drift** (cache dir, epoch settings) silently splitting the registry if
   `HostConfig` grows — keyed on a config hash, and a debug log line on split.
5. **Hibernation event gap**: subscribers of a hibernated child's `events()` stream see silence
   until wake; we prevent hibernation while subscribers exist, and `AgentInfo.state` marks it —
   frontends must render `hibernated` (UI phase informed via `agents_changed`).
6. **Idle eviction vs long-running user bash/tasks**: arbiter wait sources and open tools keep a
   session live (E-LAZY conditions); a bug here would kill a watched session — covered by a C7
   test with a running background task.
7. **Repair on every wake** could append duplicates if a wake races the same file twice —
   repair is idempotent (dangling detection finds nothing after the first repair) and runs only
   under the lease.
8. **Safe-mode false positives** (a user's broken unrelated setting fails starts twice and all
   extensions go off) — banner names the true error from `lastFailures`; success resets; manual
   `--exclude-extension`/`-ne` unaffected.
9. **Unipi-side fixes (C9) change component checksums** — `cargo xtask wasm` + committed
   checksums must be updated in the same change or CI fails.
10. **Docs-tree interface to dist.md** (P-DOCS) is a request to another spec; until granted, the
    legacy `oino.docs` extraction path keeps working and the section is omitted when absent.

---

## 7. Owner questions (recommendations chosen so work can start)

1. **Safe mode keeps providers on** (like `-ne`, D15) so the agent is still usable with
   extensions off. Recommended: yes. Alternative: everything off including providers.
2. **Trap threshold for per-session disable** = one trap (DIRECTION wording), with the
   process-wide 3-in-10-min breaker kept as backstop. Recommended: one.
3. **Hibernation default** 120 s idle, configurable via `subagents.hibernateAfterSec`, 0 = off.
   Recommended: 120 s.
4. **Session idle eviction default** 900 s in the hub (TUI single-session runtime never
   evicts). Recommended: 900 s.

None of these block the WPs; each is recorded as the default in §2 and reversible by settings or
a one-line change.
