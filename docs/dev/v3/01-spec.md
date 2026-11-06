# Oino v3 implementation spec

Binding source: `00-owner-decisions.md` A–L. Reference examples are subordinate to those decisions. Existing sessions must remain readable, including their original tool calls and results; argument migration is only for re-executing legacy calls, never a rewrite of the stored transcript.

## A. Frozen system prompt

**Current:** `crates/oino-resources/src/system_prompt.rs:263` builds sections, including skills at :316; `crates/oino-core/src/session/tools.rs:397` rebuilds the prompt and :519 computes section diffs. `crates/oino-core/src/session/prompt.rs:679` permits per-turn extension changes. `crates/oino-ai/src/transcript.rs:195` collapses system messages. These paths cannot all remain authoritative in v3.

**Target:** build one Oino-specific system prompt when a session starts and persist its exact bytes. Follow `ref-droid-prompts.md` §1 structure, retuned to Oino and lowercase tools, omitting Factory-only material. Identify `<system-reminder>` as trusted harness context; distinguish it from ordinary user text. No skill catalog or mutable workspace state in this prompt. On resume and `/reload`, use the persisted bytes, not a fresh render; reload can update catalogs only by appending reminders. Provider adaptation must not fold later changes into the frozen system message. See C for the second immutable slot and change delivery.

**Migration:** pre-v3 sessions have no stored initial prompt (`crates/oino-session/src/entry.rs:189`): on first v3 resume render once and append `system_snapshot {text, version:3}`. Never rewrite prior entries. Keep provider-specific serialization separate from stored messages. Settings for existing custom prompts remain session-start inputs, not mid-session mutations.

**Tests:** assert the rendered prompt contains none of `Droid`, `Factory`, `FetchUrl`, `missions`, or `Co-authored-by`; compare provider-bound prefix bytes across changes to tools, model, MCP, skills, reload and compaction; resume a legacy fixture and confirm no history mutation.

## B. Tools and execution

**Current:** lowercase registrations and `find` are at `crates/oino-tools/src/info.rs:33` and :44; current schemas start at :171, :213, :234, :317, :360, :387 and :410. They differ from the v3 contracts below. `crates/oino-core/src/session/tools.rs:371` currently rebuilds the prompt on tool changes.

**Input schema conventions:** JSON Schema draft-07; every object below has `additionalProperties:false`. Integers are nonnegative unless explicitly constrained. Paths are absolute where required below. Fields absent from `required` are optional; omitted booleans are false unless stated. Result data and validation errors must not mutate historical records.

```json
{"name":"read","parameters":{"type":"object","properties":{"file_path":{"type":"string"},"offset":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1,"default":2400},"line_numbers":{"type":"boolean"},"image_quality":{"type":"string","enum":["default","high"],"default":"default"}},"required":["file_path"],"additionalProperties":false}}
{"name":"ls","parameters":{"type":"object","properties":{"directory_path":{"type":"string"},"ignorePatterns":{"type":"array","items":{"type":"string"}}},"additionalProperties":false}}
{"name":"glob","parameters":{"type":"object","properties":{"patterns":{"type":"array","items":{"type":"string"},"minItems":1},"excludePatterns":{"type":"array","items":{"type":"string"}},"folder":{"type":"string"}},"required":["patterns"],"additionalProperties":false}}
{"name":"grep","parameters":{"type":"object","properties":{"pattern":{"type":"string"},"path":{"type":"string"},"glob_pattern":{"type":"string"},"output_mode":{"type":"string","enum":["content","files_with_matches","count"],"default":"files_with_matches"},"case_insensitive":{"type":"boolean"},"type":{"type":"string"},"context_before":{"type":"integer","minimum":0},"context_after":{"type":"integer","minimum":0},"context":{"type":"integer","minimum":0},"line_numbers":{"type":"boolean"},"head_limit":{"type":"integer","minimum":0},"multiline":{"type":"boolean"},"fixed_string":{"type":"boolean"},"search_mode":{"type":"string","enum":["exact","semantic","hybrid"],"default":"exact"}},"required":["pattern"],"additionalProperties":false}}
{"name":"write","parameters":{"type":"object","properties":{"file_path":{"type":"string"},"content":{"type":"string"}},"required":["file_path","content"],"additionalProperties":false}}
{"name":"edit","parameters":{"type":"object","properties":{"file_path":{"type":"string"},"edits":{"type":"array","minItems":1,"items":{"type":"object","properties":{"oldText":{"type":"string"},"newText":{"type":"string"},"change_all":{"type":"boolean","default":false}},"required":["oldText","newText"],"additionalProperties":false}}},"required":["file_path","edits"],"additionalProperties":false}}
{"name":"apply_patch","parameters":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false}}
{"name":"bash / powershell","parameters":{"type":"object","properties":{"command":{"type":"string"},"summary":{"type":"string"},"riskLevel":{"type":"string","enum":["low","medium","high"]},"timeout":{"type":"number","exclusiveMinimum":0,"default":60},"fireAndForget":{"type":"boolean","default":false}},"required":["command","summary","riskLevel"],"additionalProperties":false}}
{"name":"task_output","parameters":{"type":"object","properties":{"task_id":{"type":"string"},"block":{"type":"boolean","default":true},"timeout":{"type":"integer","minimum":0,"maximum":1200000,"default":30000}},"required":["task_id"],"additionalProperties":false}}
{"name":"task_stop","parameters":{"type":"object","properties":{"task_id":{"type":"string"}},"required":["task_id"],"additionalProperties":false}}
{"name":"tool_search","parameters":{"type":"object","properties":{"query":{"type":"string"},"max_results":{"type":["integer","null"],"minimum":1}},"required":["query"],"additionalProperties":false}}
```

The `search_mode` property is present in `grep` only when semantic search was enabled at session start; otherwise omit it from the schema and use exact mode.

`read` requires an absolute path; bare long reads show a numbered declaration outline, short reads the full file; explicit ranges are 1-based. Text line numbering is `12→text`; images limited to 5 MB, PDFs to 3 MB. `ls` and `glob` default to cwd. `grep` defaults to in-process ripgrep; semantic search is addressed in I. `write` creates or overwrites atomically, preserving existing encoding, BOM and line endings, ensuring a trailing newline, rejecting a directory with `Cannot create file at "X" because a directory already exists there.`, and returning `{success,file_path}` with only new diagnostics. `edit` checks the last observed file state before all batched edits, tracks an untouched file automatically, fails on external modification with the owner-specified stale-read message, commits atomically preserving encoding/BOM/line endings, and returns a diff summary and only new diagnostics. `change_all` applies only to its edit; otherwise a match must be unique. `apply_patch` accepts Codex `*** Begin Patch` create/update/delete/move syntax and is exposed alongside write/edit only for matching GPT, Codex and o-series models.

Each shell invocation starts a new shell, defaults to 60 seconds, and returns at most 32 KiB head+tail inline with the remainder in a log. `fireAndForget` returns a task id and log path and delivers the exit result automatically, without required polling. Shell tasks and background subagents share an id namespace; `task_output` and `task_stop` address either. Preserve mid-run detach and the work tray's complete arguments and live status. Remove watchdog Decision-model judging; session caps may remain. Risk enforcement is specified in E.

**Legacy execution normalization:** preserve original stored arguments and card renderers (`extensions/unipi/background-tasks/src/lib.rs:919`). Normalize before the permission gate, retaining task-id prefixes:

| Legacy input | Normalized execution |
|---|---|
| `read`/`write`/`edit` `{path}` | `{file_path}`; resolve relative paths against session cwd |
| `ls {path,limit}` | `{directory_path:path}`; truncate results to limit |
| `find {pattern,path,limit}` | `glob {patterns:[pattern],folder:path}`; truncate results to limit |
| `grep {glob,ignoreCase,literal,limit,context}` | `{glob_pattern,case_insensitive,fixed_string,head_limit,context,output_mode:"content"}` |
| shell without summary or risk | `summary:"(legacy call)"`; absent risk is `high` |
| `bg_run {name,command,timeoutSeconds}` | shell `{command,summary:name,riskLevel:"high",timeout:timeoutSeconds,fireAndForget:true}` |
| `bg_status` / `bg_logs {taskId}` | `task_output {task_id:taskId,block:false}`; status without id lists shared registry |
| `bg_kill {taskId}` | `task_stop {task_id:taskId}` |

Legacy aliases are not advertised to new sessions. Replaying results never re-executes calls. On Windows atomic replacement uses `MoveFileExW(REPLACE_EXISTING)` with sharing-violation retry, preserves CRLF/BOM, and process timeout/stop terminates the tree using Job Objects; background log paths must be valid Windows paths.

**Tests:** schema snapshots, old transcript replay, text/image/PDF size limits, declaration outline and paging, ripgrep modes, atomic write and metadata preservation, external edit race, ambiguous edits, patch operations and model gating, Windows PowerShell parity, timeout/background completion/detach/stop and mixed task IDs.

## C. Append-only prompt context

**Current:** prompt hooks rewrite options in `crates/oino-core/src/session/prompt.rs:701`; tool changes produce section patches in `crates/oino-core/src/session/tools.rs:519`. Provider compatibility folds system messages in `crates/oino-ai/src/transcript.rs:195`, called by `extensions/sdk/oino-provider-kit/src/anthropic.rs:930` and completions. Reload destroys extension instances (`crates/oino-core/src/session/mod.rs:617`).

**Target:** slot 1 is A's immutable system bytes. Slot 2 is one persisted synthetic user message, before the first real user message, containing ordered `<system-reminder>` blocks: unified catalog and deferred names, subagents, system/cwd, date, initial ls/git status and context files. Capture once; later changes append LLM-only user-role reminders after existing history. Never mutate either slot; compaction changes only the model's tail view, preserving both slots byte-for-byte. The provider prefix comprises the ordered tool declaration array, slot 1, slot 2, then history. At session start freeze declarations in built-in registry order followed by extension names sorted lexically. Later tools are deferred and announced in a tail catalog update; `tool_search` appends loaded schemas only. A removed tool remains declared until compaction and returns `tool X is unavailable`. Check apply_patch gating at start and model switch; append a model-change reminder and append its schema if newly eligible. Slot 2 contains no model id. Existing declaration bytes and both slots remain identical; additions are suffixes, not rewrites. Provider cache invalidation on an appended declaration does not change existing prefix bytes. Adapt providers that cannot accept later system messages by serializing later updates as user-role reminders rather than merging them into slot 1. Reset cache tracking on compaction without altering slot bytes. Do not prune stale reminders in v3. Escape literal `<system-reminder>` and `</system-reminder>` tags in all untrusted user text and tool output at provider serialization; only host-created reminder envelopes retain the trusted tags. Preserve stored source bytes.

**Extension API contract:** replace hook return mutation with host-owned append. A request is `{ "type":"append_reminder", "key":"<extension-id>/<topic>", "text":"<plain text>" }`; response `{ "id":"<message-id>", "appended":true|false }`. Host validates ownership of key, wraps text once in `<system-reminder>`, persists an LLM-only entry before next model request, serializes appends in event order, without replacing earlier messages. This is an append operation, not an upsert: never replace existing message bytes. Extensions track their own announcement state in the session stream. Failure to append must be surfaced to the originating extension. Tool results may carry a reminder without altering earlier messages. WIT/host bindings and both native and WASM extension paths must implement the same semantics.

**Migration:** for old transcripts, do not insert slot 2 into old history: append its content as the first tail reminder. Legacy system-role entries and `CompactionEntry.system_message` remain visible in the UI but are excluded from provider requests. Append the v3 system snapshot once. Old reveal entries count as already announced; all other skills are unannounced. `systemPromptOptions` mutations by migrated bundled extensions are removed, not silently applied to slot 1. Keep legacy event deserialization.

**Settings/tests:** no new setting. Test first-turn order, append ordering across reload, model changes, dynamic tools, MCP, and both provider serializers; ensure no sensitive user-authored text is promoted to trusted reminder content.

## D. Skills

**Current:** `crates/oino-resources/src/system_prompt.rs:316` inserts skill sections; registry hook returns mutated options at `extensions/unipi/skill-registry/src/lib.rs:446` and :567. Its judge uses jev (`extensions/unipi/skill-registry/src/judge.rs:207`).

**Target:** never include skills in A's system prompt or C's initial slot. The existing skill Decision model selects relevant skills per turn; emit selected skill metadata/instructions as C reminders or a tool-result hint. Persist announced and loaded names in session state, dedupe repeat announcements, retain them across compaction and reload. Restore selected skills after compaction as required by F without re-running the decision engine for already retained skills. Keep long-horizon routing and session renaming unchanged. Remove Decision-model uses only from workflow and watchdog, not the shared jev transport or skill judge.

**Migration/settings/tests:** interpret existing registry reveal entries as prior announcements and preserve current skill settings. Test no skill text in the immutable prompt, per-turn selection, no duplicates, tool hint, reload and compaction retention; verify workflow/watchdog no longer invoke jev.

## E. Autonomy and permission rules

**Current:** `extensions/unipi/workflow/src/decide.rs:121` configures jev confidence and :138 sends ambiguous commands to jev. `extensions/unipi/workflow/src/bash.rs:1` classifies shell commands. The composer currently has `apps/unipi/src/components/composer/PermissionControl.tsx`.

**Risk enforcement:** Low < Medium < High. Effective risk is max(model declaration, harness floor); the existing `extensions/unipi/workflow/src/bash.rs` classifier may only raise it. Parse bash chains `&&`, `||`, `;`, pipes, newlines, command substitutions `$()`/backticks, process substitution `<()`, nested `bash|sh|zsh -c`, `eval`, `xargs`, `find -exec/-delete`, heredocs and redirects; unknown syntax is High. PowerShell must cover `;`, `|`, `&`, `iex`/`Invoke-Expression`, `-EncodedCommand`, `Start-Process`, `Out-File`, `Set-Content` and `>`; unknown syntax is High. File redirects are at least Medium, except `/dev/null`, `$null` and `NUL` as appropriate. Chains take maximum risk, and user authorization never changes classification. Builds, tests and commits are Medium; `push`, `rm -rf` and `sudo` are High. Reject absent or malformed risk except legacy normalization (B). Rules and risk apply after normalization and to aliases, codemode and background execution. Non-shell tools use rules only, no risk ceiling. Child subagents use min(parent, child) ceiling. Cron uses global default. TUI/RPC surfaces unable to display cards are unattended, never hang; ACP maps to `request_permission`.

**Reviewer decisions, supervisor accepted, owner may override:** global interactive default, legacy-ticket default and new-ticket default are Medium. An interactive “always” writes an exact normalized command allow scoped to that project. It permits that command above the ceiling only in interactive sessions of that project; it never raises the ceiling and is ignored unattended. Explicit deny, legacy deny and ask retain precedence. Other allows do not bypass the ceiling. Plan remains an independent read-only mode.

**Rules envelope:** `{"version":1,"rules":[{"id":"string","scope":{"kind":"global"}|{"kind":"project","projectId":"string"},"tool":"string","pattern":"string","effect":"deny|ask|allow","interactiveAboveCeiling":false}],"legacyDeny":[{"tool":"string","pattern":"string"}],"default":"ask|allow|deny"}`. `interactiveAboveCeiling` is true only on an “always” rule, whose pattern is an exact normalized command, not a glob. Validate version, unique ids, scope, effect and pattern before use; reject unknown versions. Resolve explicit deny → legacy deny → ask → allow → default, considering only applicable scopes. Normalize command without changing shell semantics: retain quoting, operators and whitespace inside tokens; use parsed canonical token serialization, rejecting unparseable commands for persistent allow. Rules never override Plan restrictions.

**Approval protocol:** host emits `approval_request` `{requestId,sessionId,tool,arguments,riskLevel,ceiling,reason}` before side effects; app sends `approval_response` `{requestId,decision:"allow_once"|"always"|"deny"}`. Request ids are unique, scoped to session; consume exactly once. `allow_once` authorizes only this pending call; `always` atomically saves the project exact-command interactive-only rule and authorizes this call; deny aborts. Reject stale, duplicate, wrong-session and unattended responses; cancellation or disconnect fails closed. Unattended above-ceiling calls fail with exactly `needs Autonomy High` for High (corresponding level otherwise). Never block waiting for a nonexistent UI.

**UI:** replace ask/auto/full with `Autonomy ▮▯▯`, `Autonomy ▮▮▯`, `Autonomy ▮▮▮`; click cycles Low → Medium → High → Low. Tooltips are `Autonomy: Low (read-only commands)`, `Autonomy: Medium (reversible commands)`, `Autonomy: High (allow all commands)`. Card offers allow once / always / deny. Plan stays separate. Compaction adds a visible render-cutoff divider while retaining scrollable history.

**Migration/settings/tests:** migrate legacy denies without weakening them; preserve historical workflow entries. Global `autonomy.defaultLevel` defaults `medium`, session overrides and per-ticket `autonomy` default to Medium; rules persist in versioned `permissions` envelope. Tests cover precedence, shell constructs, bypasses, child ceiling, unattended surfaces, approval correlation and PowerShell parity.

## F. Compaction

**Current:** `crates/oino-core/src/session/compaction_flow.rs:68` resolves existing settings, :145 calls the compactor, :184 invokes hooks and :233 appends a compaction entry. Existing settings are in `crates/oino-core/src/compaction/settings.rs:8`.

**Target record:** append to the same session stream under its existing single-writer lock:

```json
{"type":"compaction_state","id":"string","timestamp":"RFC3339","summaryText":"string","summaryTokens":0,"summaryKind":"llm_summary|vcc_summary","anchorMessage":{"id":"string","index":0},"removedCount":0,"retainedSkills":["string"],"retainedImages":[{"path":"string","excerpt":"string","imageIndex":0,"imageCount":1}],"loadedDeferredToolNames":["string"]}
```

Each subsequent `SessionEntry` carries `compactionSummaryId` equal to the active record id; do not change provider message bytes. Anchor at a turn boundary, never between an assistant tool call and its results. Construct the injected summary deterministically from the state so reload is byte-identical. v3 writes only `compaction_state`, not legacy `CompactionEntry`, but reads the latter. A committed state and its anchor must refer to existing messages on the same branch. Reload selects the newest valid state on that branch, forms the model view as immutable slots 1–2, injected summary, then messages after the anchor; preserve the entire raw transcript for human scrolling. If the anchor is missing, reject the state and surface a recoverable error rather than discard history. Import legacy compaction entries as legacy records; do not rewrite them. Never append a partial compaction state on failure.

The LLM summarizer uses `ref-droid-prompts.md` §5.1, §5.3, §5.4 and §5.5 verbatim, with Droid → Oino only and conditional interpolation; omit the unrecovered `{ur}` identity prefix and do not use §5.2. At a 6000-token previous-summary soft cap use the §5.4 IMPORTANT length instruction. Summarize the previous summary plus messages after its anchor, stripping the previous TODO block and appending current todos. Keep vcc/llm selection, both using the same persistence, trigger and recovery machinery; vcc writes `vcc_summary` without the LLM delta step. Clamp custom instructions to 25% of summarizer context; reserve 4k tokens, pin the recent tail and insert the omitted-message marker. Retry context failures on the 0.9 → 0.75 → 0.5 shrink ladder; resample empty/degenerate responses then escalate once to the session main model; final failure preserves history, and threshold failure is nonfatal. Preserve original data if every attempt fails.

Post-turn threshold uses actual provider input usage including cache reads; resolution is session override → model/family → global → model default → 250k, clamped to max input. Suppress repeated threshold compaction for the same session/model/limit until usage falls below the limit; failed threshold compaction is nonfatal. On in-turn overflow compact with `requireProgress` and retry only the failed model request, never completed tools; if no messages are removed, return the original failure. `/compact` accepts custom instructions; PreCompact may block only manual compaction. Reset the cache snapshot after success, then re-announce retained skills and loaded deferred tools without altering slots 1–2.

**Settings:** preserve vcc/llm toggle, existing compactor model and `compaction.enabled`; add only `compaction.tokenLimit`, `compaction.tokenLimitPerModel` and a session override. Existing `keep_recent_tokens` pins the tail for both compactors; replace `reserve_tokens` with the limit chain. `context_edit` may target only tail entries the provider has not seen; reject extension drafts targeting earlier entries (`crates/oino-core/src/session/boundary.rs:66`). **Tests:** legacy and epoch reload, branch anchor validation, crash before commit, no data loss, retained images/skills, iterative delta/TODO replacement, all trigger and failure paths, provider usage accounting, cache-prefix byte identity.

## G. Loop guards

**Current:** `crates/oino-agent/src/agent_loop.rs:128` polls steering and :213 executes tool batches; it has no v3 todo/empty-response guard at those points.

**Target:** every normal session counts completed tool calls since a list was created or updated. Suggest TodoWrite after five calls without a list; remind after 20 calls without an update; once per ending attempt, if open items remain, append the verbatim reminders from `ref-droid-prompts.md` §§4.1, 4.2 and 4.3 at their respective triggers (where §4.2 is only partially recovered, use its recovered prefix and do not invent a verbatim remainder); append the settle nag and continue. A new follow-up user message during execution gets the §4.4 steering note, not cancellation. Stop after three consecutive responses with neither text nor tool calls, and surface a notice. Stop on three consecutive identical calls, defined by tool name plus canonical JSON arguments. Resample degenerate/looping output at most three times, only before any tool in that response runs. A length-truncated empty response gets `Continue.` before retry. Reset counters at appropriate turn/list progress boundaries and ensure resampling never executes the same side effect twice.

**Migration/settings/tests:** existing sessions start counters at zero; no new settings. Simulate thresholds, progress resets, open-todo settlement, user steering, empty/length responses, repeated calls and side-effect deduplication.

## H. Codemode and deferred tools

**Current:** dynamic system/tool changes pass through `crates/oino-core/src/session/tools.rs:371`; the agent handles tool declarations at `crates/oino-agent/src/agent_loop.rs:337`. Consult the installed pi `docs/codemode.md` for current parity, including QuickJS sandbox, `tools`, discovery, storage, options and output limits.

**Target:** enable existing codemode execution and exposure modes with pi parity; every invoked tool still traverses the E permission gate. Slot 2 includes a stable `Deferred tools:` list. B's `tool_search` selects exact comma-separated names after `select:` or matches keywords against tool names; keyword limit defaults to five, exact selection ignores the limit. Add selected schemas to the live declarations and retain loaded names through compaction. On new or removed tools append a C catalog-update reminder, never mutate slot 2. Enforce unknown/unavailable names as errors without fabricating tools.

**Migration/settings/tests:** retain existing codemode configuration and old tool entries; preserve `find` replay alias. Test scripts, isolation, partial failures, permission bypass attempts, deferred exact/keyword search, MCP updates and retained tools after reload/compaction.

## I. Semantic grep

**Current:** grep registration is `crates/oino-tools/src/info.rs:360`.

**Target:** `search.semanticEnabled` defaults false and is snapshotted at session start; a mid-session change applies next session. Default exact mode uses in-process ripgrep, needs no index. When enabled, B's `grep` schema adds `search_mode: exact|semantic|hybrid` (default exact). Use the in-process zvec-grep CPU engine library in `oino-tools`, no subprocess or daemon, including MSVC CPU packaging. Semantic/hybrid uses `<project>/.zvec-grep/`; unload model after five minutes idle. Put `*` in `.zvec-grep/.gitignore`, not repository `.gitignore`. Index creation and model download (default `local/potion-code-16m-v2`) happen only after explicit consent in Settings › Search › Build index. The model cannot initiate a build. If absent, grep refuses and names that control. Project isolation and E rules apply.

**Migration/settings/tests:** existing sessions remain exact, opt-in applies next session. Test exact without index, refusal, explicit consent, per-project index, five-minute unload, CPU/MSVC.

## J. Memory and dreaming

**Current:** `extensions/unipi/dream/src/schedule.rs:41` schedules runs; UI is `apps/unipi/src/components/memory/DreamPanel.tsx`. `extensions/unipi/memory/src/usage.rs:1` tracks search, read and injection in a sidecar, with `lastUsed`/`useCount` (:47); `is_stale` (:281) can mark a never-used memory stale after tracking has existed long enough. `extensions/unipi/dream/src/lib.rs:106` uses the stale flag and excludes preferences in its prompt.

**Target:** retain mempalace, existing dream schedule, model picker, recent runs, source links and approval of skill/check proposals. A memory counts as used when returned to a session by search or injection; existing explicit reads also count. Usage must persist `last_used_at` and `use_count` in mempalace metadata, preserving and migrating existing sidecar data. For stale-usage pruning require positive per-memory usage history, at least 30 days since last use, and no explicit preference; missing history is never evidence for deletion. Preserve other owner-requested consolidation and cleanup without deleting preferences. wp-ext-reminders owns this small remaining R15 gap and its tests; it does not redo the already implemented dream UI.

**Migration/settings/tests:** preserve dream schedule and existing records; idempotently import sidecar values before metadata becomes authoritative, with no loss of sources or merged history. Existing `staleDays` may be more conservative but cannot permit the usage-pruning threshold below 30 days. Test returned search/injection, read, no-history safety, preferences, 30-day boundary and sidecar migration.

## K. Sheet select bug

**Current and target:** fixed in commit `8abc78b3`; preserve portaled Kobalte select behavior and genuine outside dismissal. No new implementation package. Existing `apps/unipi/src/components/phone/SheetSelect.test.tsx` verifies shared behavior; retain the Subagents sheet regression in acceptance checks.

## L. Superagent

**Current:** kanboard records the claimant in `apps/unipi/crates/unipi-kanboard/src/model.rs:368`; host cron is embedded in `apps/unipi/crates/unipi-host/src/cron_serve.rs:172`. `apps/unipi/src/lib/session/turns.ts:304` classifies steering without the required author distinction.

**Target:** workspace-free Superagent chat in top navigation manages all project boards, replacing `/kanboard-do` and per-session `/kanboard-autowork`. Select projects from sidebar history and zoxide; ask if ambiguous. It can create backlog tasks, move to todo, edit/order/depend, set ticket autonomy and summarize boards. No worktrees. Parallelism across projects is subject to a global cap and at most one active worker per project by default; dependencies encode same-project serialization.

The Superagent session has no cwd, filesystem tools, shell tools or autonomy chip. Its tools are cross-project kanboard, project resolver (sidebar + zoxide), `superagent.start/stop/relay`, and `ask_user`; use the frozen prompt in A. Remove `/kanboard-do` and `/kanboard-autowork`, retaining old transcript rendering.

A single dispatcher per machine holds a host-wide lock; a second host cannot dispatch. Both watcher and manual start go through this dispatcher. Under the existing kanboard project lock, check dependencies, no live project Run and global cap, preallocate a session id and persist `Run.session` before idempotent session creation; attach if already created. The worker's own claim is a no-op if its id matches. No worktrees. A live worker finishes to in_review or blocked with a question. Lowering the cap never kills active workers. The watcher can be off while manual start remains available. On restart reconcile every local Run before dispatch: a non-live session becomes blocked with `Worker was interrupted; move to todo to retry`; never requeue automatically. The kanboard reaper must not return dispatcher runs to todo. On worker exit without a terminal transition, block with its last message as question. Hold slot reservations across the launch transaction; crash before launch must not create a second session.

**Host protocol:** all ops use existing host request id/reply framing; require authenticated host client. Errors include stable code and actionable message. `superagent.status {}` returns `{enabled,concurrencyCap,workers:[{projectId,taskId,sessionId,autonomy,step,tool,startedAt,cost}],needsYou:[{projectId,taskId,status,question?}]}`. `superagent.configure {enabled:boolean,concurrencyCap:integer>=1}` returns effective config. `superagent.start {projectId,taskId}` returns `{sessionId}` or refusal. `superagent.stop {projectId,taskId}` returns `{stopped:boolean}`. `superagent.relay {projectId,taskId,text}` returns `{commentId,sessionId,delivered:boolean}`; validate project/task and current claim, persist author-tagged ticket comment, then send follow-up to `Run.session`; report partial delivery failure, never claim success. Relay is explicit; board comments do not auto-relay. Host/agent messages show author and never user Steer; actual user follow-ups do.

**Kanboard contract:** add optional task `autonomy: low|medium|high` with Medium default, exposed in create/edit/show/board APIs; preserve legacy tasks and `Run` fields. Add dispatcher-owned claim marker to Run, preventing reaper auto-release, and idempotent same-session claim. Enforce dependency satisfaction and in_review/blocked transitions under existing locks. Watcher config `superagent.enabled` and `superagent.concurrencyCap` persisted by host; default cap 1. No silent project guesses.

**UI:** center chat; side pane live task/project/autonomy/step/tool/elapsed/cost with stop/open, Needs you (in_review and blocked including question), watcher switch and cap. Test concurrent watcher/manual starts, project exclusion, crash recovery, relay, and author rendering. Windows tray host runs dispatcher; cfg(windows) tests and one real Windows host run are required before closure.

## Already implemented

- J dream backend/UI: `80d2e16e`, `8e6dbda2`, `de6d39c6`, `91e8ce60`; R15's remaining usage-history/metadata gap is assigned above.
- K sheet select: `8abc78b3`; no new package.

## Work packages

**Planning draft only:** ownership and tests below are starting boundaries, not approved execution contracts. Before dispatch, enumerate all shared integration files, verify commands and legacy mappings, and resolve the questions below. Dependencies do not authorize parallel edits of an unlisted shared file. Shared integration edits must be serialized in this order: wp-cache → wp-tools-fs → wp-exec-bg → wp-ext-reminders → wp-loop-guards → wp-compaction → wp-codemode → wp-autonomy → wp-autonomy-ui → wp-zvec → sa-backend → sa-ui. This order applies only when packages actually touch the same file; independent owned files can proceed in parallel after dependencies.

| Package | Initial owned files | Depends on | Candidate acceptance commands (not yet verified) |
|---|---|---|---|
| wp-cache | `crates/oino-ai/src/transcript.rs`, `crates/oino-core/src/session/{prompt,tools}.rs`, `crates/oino-resources/src/system_prompt.rs`, `extensions/sdk/oino-provider-kit/src/{anthropic,completions}.rs` | none | `CARGO_BUILD_JOBS=8 cargo test -p oino-ai`; `CARGO_BUILD_JOBS=8 cargo test -p oino-core` |
| wp-tools-fs | `crates/oino-tools/src/{read,write,find,grep,ls,info}.rs`, `crates/oino-tools/src/edit/` | wp-cache | `CARGO_BUILD_JOBS=8 cargo test -p oino-tools` |
| wp-exec-bg | `crates/oino-tools/src/bash/`, `extensions/unipi/background-tasks/`, `extensions/unipi/watchdog/` | wp-tools-fs | `CARGO_BUILD_JOBS=8 cargo test -p oino-tools`; `cargo xtask wasm` |
| wp-ext-reminders | `extensions/unipi/{skill-registry,long-horizon,subagents,fusion,browser}/` | wp-cache | `cargo xtask wasm`; `CARGO_BUILD_JOBS=8 cargo test -p oino-core` |
| wp-loop-guards | `crates/oino-agent/src/agent_loop.rs` | wp-cache | `CARGO_BUILD_JOBS=8 cargo test -p oino-agent` |
| wp-compaction | `crates/oino-core/src/compaction/`, `crates/oino-core/src/session/compaction_flow.rs`, `extensions/unipi/compactor/` | wp-cache, wp-ext-reminders | `CARGO_BUILD_JOBS=8 cargo test -p oino-core`; `cargo xtask wasm` |
| wp-codemode | `crates/oino-tools/src/js.rs` | wp-cache, wp-tools-fs, wp-ext-reminders | `CARGO_BUILD_JOBS=8 cargo test -p oino-tools` |
| wp-autonomy | `extensions/unipi/workflow/` | wp-exec-bg, wp-codemode | `cargo xtask wasm`; `CARGO_BUILD_JOBS=8 cargo test -p oino-core` |
| wp-autonomy-ui | `apps/unipi/src/components/composer/PermissionControl.tsx`, `apps/unipi/src/components/phone/Sheet.tsx`, `apps/unipi/src/components/board/BoardSheet.tsx` | wp-autonomy | `cd apps/unipi && npx tsc --noEmit && npx vitest run src/components/composer/PermissionControl.test.tsx src/components/phone/SheetSelect.test.tsx` |
| wp-zvec | new zvec integration under `crates/oino-tools/src/` | wp-tools-fs | `CARGO_BUILD_JOBS=8 cargo test -p oino-tools` |
| sa-backend | `apps/unipi/crates/unipi-kanboard/`, new dispatcher under `apps/unipi/crates/unipi-host/src/`, `extensions/unipi/kanboard/` | wp-autonomy | `CARGO_BUILD_JOBS=8 cargo test -p unipi-kanboard`; `CARGO_BUILD_JOBS=8 cargo test -p unipi-host` |
| sa-ui | `apps/unipi/src/components/shell/`, `apps/unipi/src/lib/session/turns.ts`, `apps/unipi/src/components/memory/DreamPanel.tsx`, `extensions/unipi/{dream,memory}/` | sa-backend, wp-autonomy-ui | `cd apps/unipi && npx tsc --noEmit && npx vitest run src/components/memory/DreamPanel.test.tsx` |

### Open owner questions

1. When an above-ceiling interactive call is approved with “always”, should that choice raise the session/ticket autonomy ceiling, or authorize a scoped exception without raising it? E1 says the ceiling is enforced while E3 offers always; this affects both UI wording and unattended safety.
2. What is the default autonomy for legacy tasks without a per-ticket value, and for newly created tickets when none is specified? A safe migration must not silently grant High.
